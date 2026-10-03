//! The voice director: how one spoken sentence should sound, asked of the
//! model already loaded, and recorded beside the conversation it shaped.
//!
//! A speech engine that takes a free-text voice direction beside the words
//! (Breeze TTS 2's `instructions`) can be told *how* to say a sentence. The
//! owner's ruling (2026-10-03, `docs/VOICE-BREEZE-DESIGN.md`) is that every
//! spoken sentence gets one, written by a separate one-shot call — the
//! director — rather than by the chat model inside its reply. The reply's
//! prompt is untouched; the director reads the sentence, the moment in the
//! conversation and the speaker, and answers with one line.
//!
//! **What a direction can reach is the shape of the safety argument.** The
//! sentence it reads may quote third-party text, so the pass is a
//! [`QuarantinedPass`]: no tools, no history, one user turn. What leaves is
//! one line, bounded by [`tidy`], and it goes to exactly one place: the
//! speech engine's `instructions`, which changes how a sentence sounds and
//! never what is said or sent.
//!
//! **Every direction is recorded** ([`SpokenDirection`], appended to the
//! session's transcript) so the corpus can be studied later — except in an
//! incognito chat, where the director still runs and nothing is written
//! (owner ruling, 2026-10-03). That rule lives with the caller, which is the
//! side that knows which chat a call is in.

use crate::message::StopReason;
use crate::provider::Provider;
use crate::quarantine::QuarantinedPass;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// How long a sentence after the first may wait for its direction, from
/// when the worker asks.
///
/// **Set above the measured cost.** On the loaded `qwen3.6-35b-a3b-uncensored`
/// (2026-10-03: the real [`SYSTEM`], a scene, the growing "already directed"
/// list, thinking off, 3 rounds × 6 sentences), a 12–25-word direction took
/// a median 1.61 s, p90 1.77 s, max 1.83 s; a 6–12-word one, a median
/// 1.11 s. The owner wants detailed directions, so the length stays and the
/// deadline sits above it — an earlier 1.5 s would have timed out most
/// sentences.
///
/// **The trade-off.** A sentence is asked about when the worker reaches it,
/// which is while the sentence before it plays, so the slack a direction
/// hides in is that sentence's *remaining* playback — about (1 − RTF) × its
/// length, at Breeze's real-time factor. A short sentence before a long
/// direction leaves a gap: the next sentence waits up to this long before it
/// starts. The follow-up that removes it is prefetch: the facade directs
/// each sentence as its text streams out of the model, before the worker
/// asks, so the answer is usually waiting.
pub const SENTENCE_DEADLINE: Duration = Duration::from_millis(2500);

/// How long the first sentence may wait, counted from when the worker asks.
/// Its direction was started from the owner's words when the turn began
/// (the *opening*), so most of the call has already run by then; this is
/// the first sound of the reply, and nothing plays while it waits.
pub const FIRST_SENTENCE_DEADLINE: Duration = Duration::from_millis(2500);

/// The opening call's own bound, from the start of the turn. Longer than
/// either deadline above because nothing waits on it until the first
/// sentence is asked for, and that ask carries its own deadline.
pub const OPENING_DEADLINE: Duration = Duration::from_secs(8);

/// The longest direction sent. Matches the Breeze adapter's own cap
/// (`scripts/voice/breeze_server.py` `INSTRUCTIONS_MAX`), so neither side
/// truncates what the other passed.
pub const MAX_CHARS: usize = 300;

/// How much of a persona's identity the director is shown: enough for a
/// voice, not so much that every sentence re-reads a character sheet.
const MAX_CHARACTER_CHARS: usize = 1_200;

/// How much of the last reply and the owner's words the director is shown.
const MAX_CONTEXT_CHARS: usize = 600;

/// The most of a whole reply a Listen tap's director reads ([`Cue::Reply`]):
/// enough to hear where a long reply goes, and still one short prompt.
const MAX_REPLY_CHARS: usize = 1_500;

/// The director's frame. Static, so it is the cached prefix of every call.
///
/// **Delivery, never identity.** Breeze's direction mode is defined as
/// steering "tone, emotion, pace, and delivery" while keeping the reference
/// speaker's identity; a pitch *level* belongs to its voice-design mode,
/// which makes a new voice. Measured on the engine (2026-10-03, one logged
/// turn's directions over 8 sentences × 3): directions naming "low/high/
/// medium pitch" moved the median pitch 5.5 semitones between adjacent
/// lines, up to 14.5 — the owner heard "a different person" — against 2.2
/// with only those words removed and texture kept. Texture and pitch
/// *movement* within a line are what the vendor's own direction benchmark
/// grades, and stay.
///
/// **The texture list is balanced, and that is load-bearing.** The real
/// director, asked over three logged replies (24 sentences a draw):
///
/// - a list of only breathy/husky/trembling textures was copied as a menu
///   ("husky" in 21 of 24 directions, for the assistant's scene too), and one
///   draw collapsed into the same ten adjectives on 19 lines;
/// - no list at all wandered into gestures, gaze and vocal events ("a sharp
///   intake of air", "a low, dangerous purr"), a 2.2-semitone mean move and
///   one of 17.6;
/// - this frame — plain textures beside the expressive ones, "or none", the
///   voice only — named no pitch level in 24, moved 1.6 semitones on
///   average and 5.3 at most, against 2.8 and 18.0 for the frame before it.
pub const SYSTEM: &str = "\
You direct a voice actor who is about to speak one sentence of a reply \
aloud, always in their own voice. Given who is speaking, the moment in the \
conversation and the sentence, answer with one line of delivery direction, \
12 to 25 words: the emotion, the energy, the pace, the emphasis and the \
texture of the voice when the moment calls for one (clear, bright, soft, \
warm, breathy, hushed, husky, tight, trembling, or none). Pitch may move \
within the line - rising, falling, lifting at the end - but never name a \
pitch level, register, age or accent, not even inside a texture: no high, \
low, deep or mid. Direct only the voice: never gestures, gaze, actions or \
the scene. Stay close to how the previous line was delivered and change by \
degrees; change sharply only when the moment turns. Never repeat, quote or \
rewrite the words, and never add new ones. No preamble, no quotes, no \
markdown.";

/// Put in front of every direction sent, by code rather than asked of the
/// director: the steadiest form measured (2026-10-03, hand-written lines), a
/// 1.5-semitone mean pitch move between lines against 2.2 for the same
/// directions without it. Measured as "In her own…"; "your" so that no
/// speaker's pronouns are assumed, and it addresses the voice actor the
/// frame does. Never shown back to the director, so it cannot learn to copy
/// it.
pub const ANCHOR: &str = "In your own natural voice: ";

/// The longest line the director's answer is cut to, leaving room for
/// [`ANCHOR`] inside [`MAX_CHARS`].
const LINE_MAX: usize = MAX_CHARS - ANCHOR.len();

/// What is sent as `instructions` for a direction the director wrote.
pub fn sent(line: &str) -> String {
    format!("{ANCHOR}{line}")
}

/// The director's own line inside a recorded `direction`, as [`sent`] was
/// given it — what goes back into a prompt's "already directed" list, which
/// never shows the director the anchor. A line recorded without one is
/// returned whole.
pub fn unsent(direction: &str) -> &str {
    direction.strip_prefix(ANCHOR).unwrap_or(direction)
}

/// What a Listen tap's directions are recorded under in
/// [`SpokenDirection::turn`]: `listen:` and the page's key for the reply, so
/// a second tap on the same reply finds them and a study tells them from a
/// call's, whose turn is the facade's completion id.
pub const LISTEN_TURN: &str = "listen:";

/// What the director knows about the turn it is directing. Built once per
/// turn by the caller; the per-sentence part is passed to [`prompt`].
#[derive(Debug, Clone, Default)]
pub struct Scene {
    /// Who is speaking: a persona's identity, or `None` for the assistant.
    pub character: Option<String>,
    /// What the speaker said last, before this turn.
    pub last_reply: Option<String>,
    /// What the owner just said: the words this turn answers.
    pub utterance: String,
    /// How the speaker's last directed line was delivered, carried from the
    /// reply before, so a new reply moves on from it rather than starting
    /// cold.
    pub last_direction: Option<String>,
}

/// What a call is directing: the reply's opening before its words exist, or
/// one sentence of it — or, for Listen, the whole reply at once.
#[derive(Debug, Clone, Copy)]
pub enum Cue<'a> {
    Opening,
    Sentence(&'a str),
    /// A whole reply read aloud with Listen, directed once: one line for
    /// every sentence of it (owner, 2026-10-03: "just do it for the entire
    /// turn"), so speech never waits on the director after it starts.
    Reply(&'a str),
}

/// The user turn for one call, stable to volatile: the scene first, then
/// the sentences already directed this turn, then the cue. Each call in a
/// turn is the previous call's text with one more directed line and a new
/// cue, so a server that caches by prefix re-reads only the tail.
pub fn prompt(scene: &Scene, directed: &[(String, String)], cue: Cue<'_>) -> String {
    let mut p = String::new();
    match &scene.character {
        Some(c) => {
            p.push_str("Speaker: ");
            p.push_str(&bounded(c, MAX_CHARACTER_CHARS));
        }
        None => p.push_str("Speaker: a warm, capable personal assistant."),
    }
    p.push('\n');
    if let Some(r) = &scene.last_reply {
        p.push_str("What the speaker said last: ");
        p.push_str(&bounded(r, MAX_CONTEXT_CHARS));
        p.push('\n');
    }
    if let Some(d) = &scene.last_direction {
        p.push_str("How the speaker's last line was delivered: ");
        p.push_str(&bounded(d, MAX_CONTEXT_CHARS));
        p.push('\n');
    }
    p.push_str("They just said: ");
    p.push_str(&bounded(&scene.utterance, MAX_CONTEXT_CHARS));
    p.push('\n');
    if !directed.is_empty() {
        p.push_str("\nAlready directed in this reply:\n");
        for (sentence, direction) in directed {
            p.push_str("- \"");
            p.push_str(&bounded(sentence, MAX_CONTEXT_CHARS));
            p.push_str("\" -> ");
            p.push_str(direction);
            p.push('\n');
        }
    }
    p.push('\n');
    match cue {
        Cue::Opening => p.push_str(
            "Now: the opening of the reply. Its words are not written yet; direct how it begins.",
        ),
        Cue::Sentence(s) => {
            p.push_str("Now: \"");
            p.push_str(&bounded(s, MAX_CONTEXT_CHARS));
            p.push('"');
        }
        Cue::Reply(r) => {
            p.push_str(
                "Now: the whole reply, spoken in one steady delivery; direct all of it with one line: \"",
            );
            p.push_str(&bounded(r, MAX_REPLY_CHARS));
            p.push('"');
        }
    }
    p
}

/// A short digest of exactly what was asked — the frame and the user turn —
/// so a study can group directions by prompt without the record carrying
/// the prompt twice.
pub fn digest(user: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(SYSTEM.as_bytes());
    h.update(b"\n");
    h.update(user.as_bytes());
    h.finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The quarantined pass a direction is asked through: the director's frame
/// cached, and no reasoning — a thinking model would spend seconds per
/// sentence on a line of stage direction. `max_tokens` stays at the local
/// ceiling (CLAUDE.md: a cap below `--reasoning-budget` is an empty 200 if a
/// template ignores the request not to think); the caller's deadline is
/// what bounds the time.
pub fn pass(model: &str) -> QuarantinedPass {
    QuarantinedPass::new(model, crate::provider::LOCAL_MAX_TOKENS)
        .system(SYSTEM)
        .cache_prompt(true)
        .no_thinking()
}

/// What one call came to, before the caller's deadline is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directed {
    Line(String),
    /// The model answered with nothing usable.
    Empty,
    /// A refusal, which arrives as an ordinary 200.
    Refused,
}

/// Ask for one direction. The caller bounds the time and decides what is
/// recorded; this only asks and tidies.
pub async fn direct(provider: &dyn Provider, model: &str, user: &str) -> Result<Directed> {
    let response = provider.complete(&pass(model).ask(user), None).await?;
    // The envelope before the content: a refusal is a 200 with prose in it.
    if response.stop_reason == StopReason::Refusal {
        return Ok(Directed::Refused);
    }
    Ok(match tidy(&response.message.text()) {
        Some(line) => Directed::Line(line),
        None => Directed::Empty,
    })
}

/// Bound a model's answer into something a speech engine can take: one
/// line, no control or format characters, short enough that [`sent`] stays
/// within [`MAX_CHARS`], cut on a word where one is in reach. `None` when
/// nothing is left.
pub fn tidy(raw: &str) -> Option<String> {
    let mut cleaned = String::new();
    let mut space = false;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            space = !cleaned.is_empty();
            continue;
        }
        if ch.is_control() || crate::title::is_format_char(ch) {
            continue;
        }
        if space {
            cleaned.push(' ');
            space = false;
        }
        cleaned.push(ch);
    }
    let cleaned = cleaned
        .trim_start_matches("Direction:")
        .trim_start_matches("direction:")
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '*' || c == '#')
        .trim();
    if cleaned.is_empty() {
        return None;
    }
    if cleaned.chars().count() <= LINE_MAX {
        return Some(cleaned.to_string());
    }
    let cut: String = cleaned.chars().take(LINE_MAX).collect();
    let at = cut.rfind(' ').filter(|&i| i > LINE_MAX / 2);
    Some(at.map_or(cut.clone(), |i| cut[..i].to_string()))
}

fn bounded(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut cut: String = flat.chars().take(max).collect();
    cut.push('…');
    cut
}

/// How a direction request ended. A closed set written to an append-only
/// store, so a value this build does not know loads as `Unknown` rather than
/// failing the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// A direction was written and sent.
    Ok,
    /// The deadline passed first; the sentence went out undirected.
    Timeout,
    /// The call failed, or the model refused.
    Error,
    /// The model answered with nothing usable.
    Empty,
    /// Not directed on purpose: harness speech, a model switch in flight,
    /// no turn to direct. `reason` says which.
    Skipped,
    #[default]
    #[serde(other)]
    Unknown,
}

/// One sentence's direction as it happened: the session record
/// (`"record": "spoken_direction"`) the corpus studies.
///
/// **Joined by `turn` and `sentence`, never by where it sits in the file.**
/// A direction is requested while its sentence is about to play, which is
/// after the reply's text was streamed and often after the run's own records
/// were written, so the line lands wherever the clock put it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SpokenDirection {
    pub ts: Option<DateTime<Utc>>,
    /// The reply this sentence belongs to: the facade's completion id in a
    /// call, or [`LISTEN_TURN`] and the page's reply key for a Listen tap.
    pub turn: String,
    /// The worker's id for the same answer (pipecat's TTS context), when it
    /// sent one: a second key for joining against the worker's own journal.
    pub context: Option<String>,
    /// The sentence's position in the reply, from 0.
    pub index: u32,
    /// The sentence as the speech engine received it — for a Listen tap,
    /// the whole reply its one direction covers.
    pub sentence: String,
    /// Directed from the owner's words before the sentence existed: the
    /// first sentence's direction, started when the turn began.
    pub opening: bool,
    /// What was sent as `instructions`, when anything was.
    pub direction: Option<String>,
    /// The model that wrote it, as it answered.
    pub model: Option<String>,
    /// From the worker's request to the answer.
    pub latency_ms: u64,
    pub outcome: Outcome,
    /// Why, when it was skipped or failed: `harness`, `switching`,
    /// `no_turn`, or the error's first line.
    pub reason: Option<String>,
    pub voice: Option<String>,
    /// [`digest`] of the prompt this direction answered.
    pub prompt_digest: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Scene {
        Scene {
            character: Some("Maya, a wry friend who teases gently.".into()),
            last_reply: Some("Good luck with the grant!".into()),
            utterance: "I finally finished it, but I'm wiped out.".into(),
            last_direction: None,
        }
    }

    #[test]
    fn consecutive_calls_in_a_turn_share_everything_but_the_tail() {
        let s = scene();
        let opening = prompt(&s, &[], Cue::Opening);
        let first = vec![(
            "Oh, you did it!".to_string(),
            "bright, delighted".to_string(),
        )];
        let second = prompt(&s, &first, Cue::Sentence("Go rest."));
        let head = opening.rsplit_once("\n\nNow:").unwrap().0;
        assert!(
            second.starts_with(head),
            "the scene must be the same bytes on every call of a turn"
        );
        assert!(second.ends_with("Now: \"Go rest.\""));
        assert!(second.contains("- \"Oh, you did it!\" -> bright, delighted"));
    }

    #[test]
    fn the_pass_declines_reasoning_and_carries_no_tools() {
        let req = pass("m").ask("q");
        assert_eq!(req.think, Some(false));
        assert!(req.tools.is_empty());
        assert_eq!(req.system.as_deref(), Some(SYSTEM));
        assert!(
            req.max_tokens >= 4096,
            "a cap below the reasoning budget is an empty 200"
        );
    }

    #[test]
    fn tidy_makes_one_bounded_line_without_control_bytes() {
        assert_eq!(
            tidy("Direction: \"Warm,\n  unhurried\u{7}, smiling.\"").as_deref(),
            Some("Warm, unhurried, smiling.")
        );
        assert_eq!(tidy("  \n\t "), None);
        assert_eq!(tidy("\u{202e}"), None);
        let long = "word ".repeat(200);
        let t = tidy(&long).unwrap();
        assert!(
            sent(&t).chars().count() <= MAX_CHARS,
            "the anchor pushed it past the engine's cap"
        );
        assert!(!t.ends_with(' '));
    }

    #[test]
    fn a_carried_direction_is_part_of_the_scene_every_call_shares() {
        let mut s = scene();
        s.last_direction = Some("Soft, a little tired, slowing.".into());
        let opening = prompt(&s, &[], Cue::Opening);
        let later = prompt(&s, &[], Cue::Sentence("Go rest."));
        let head = opening.rsplit_once("\n\nNow:").unwrap().0;
        assert!(head.contains(
            "How the speaker's last line was delivered: Soft, a little tired, slowing.\n"
        ));
        assert!(later.starts_with(head));
        assert!(!prompt(&scene(), &[], Cue::Opening).contains("last line was delivered"));
        // Listen's cue: the whole reply, directed with one line.
        let whole = prompt(&scene(), &[], Cue::Reply("It went well. Go rest."));
        assert!(
            whole.ends_with("one line: \"It went well. Go rest.\""),
            "{whole}"
        );
    }

    #[test]
    fn the_director_is_asked_for_delivery_never_a_pitch_level() {
        // The words that made a different person of the same voice (§SYSTEM).
        assert!(!SYSTEM.contains("and the pitch"));
        assert!(SYSTEM.contains("never name a pitch level"));
        assert!(SYSTEM.contains("texture of the voice"));
        assert!(SYSTEM.contains("change by degrees"));
        assert!(SYSTEM.contains("Direct only the voice"));
        // A one-sided list is copied as a menu (§SYSTEM): plain textures and
        // "none" stand beside the expressive ones.
        assert!(SYSTEM.contains("clear, bright, soft") && SYSTEM.contains("or none"));
        assert_eq!(sent("Warm."), "In your own natural voice: Warm.");
    }

    #[test]
    fn an_unknown_outcome_loads_as_unknown_and_the_record_round_trips() {
        let d = SpokenDirection {
            ts: Some(Utc::now()),
            turn: "chatcmpl-1".into(),
            context: Some("ctx".into()),
            index: 2,
            sentence: "Go rest.".into(),
            opening: false,
            direction: Some("soft, firm".into()),
            model: Some("m".into()),
            latency_ms: 812,
            outcome: Outcome::Ok,
            reason: None,
            voice: Some("house".into()),
            prompt_digest: Some(digest("x")),
        };
        let back: SpokenDirection =
            serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back, d);
        let later: SpokenDirection =
            serde_json::from_str(r#"{"turn":"t","outcome":"interrupted_later"}"#).unwrap();
        assert_eq!(later.outcome, Outcome::Unknown);
    }

    /// A provider that answers with fixed text, or refuses.
    struct Says(&'static str, StopReason);

    #[async_trait::async_trait]
    impl Provider for Says {
        fn id(&self) -> &str {
            "says"
        }
        fn default_model(&self) -> &str {
            "m"
        }
        async fn complete(
            &self,
            req: &crate::message::CompletionRequest,
            _sink: Option<&crate::provider::StreamSink>,
        ) -> Result<crate::message::CompletionResponse> {
            assert_eq!(req.think, Some(false));
            Ok(crate::message::CompletionResponse {
                message: crate::message::Message::assistant(vec![crate::message::Block::text(
                    self.0,
                )]),
                stop_reason: self.1,
                usage: Default::default(),
                refusal: None,
                model: "m".into(),
                malformed_tool_args: 0,
            })
        }
    }

    #[tokio::test]
    async fn a_refusal_is_not_a_direction_and_blank_is_empty() {
        let ok = direct(&Says("Gentle, slow.", StopReason::EndTurn), "m", "q").await;
        assert_eq!(ok.unwrap(), Directed::Line("Gentle, slow.".into()));
        let refused = direct(&Says("I can't help.", StopReason::Refusal), "m", "q").await;
        assert_eq!(refused.unwrap(), Directed::Refused);
        let blank = direct(&Says("   ", StopReason::EndTurn), "m", "q").await;
        assert_eq!(blank.unwrap(), Directed::Empty);
    }
}
