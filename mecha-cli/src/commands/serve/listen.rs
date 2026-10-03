//! Listen's director pass (`docs/VOICE-BREEZE-DESIGN.md` §3.1): one delivery
//! direction for a whole reply read aloud (owner, 2026-10-03: "just do it
//! for the entire turn … let's start simple"). Calls are untouched: their
//! sentences are each directed (`voice::direct`).
//!
//! The page asks `/api/speak` a piece at a time, the next while the last
//! plays, naming the reply by a key of its own and sending its text with
//! each piece. Four rules carry the design:
//!
//! - **Once per reply, before the first piece.** The direction is settled
//!   when the first piece is asked for and every piece is spoken with it, so
//!   speech never waits on the director after it starts — a per-sentence
//!   direction would, whenever a short sentence leaves too little playback
//!   to hide the next call behind.
//! - **The same director, under the same rules.** The frame and the call are
//!   a call's (`voice::ask_director_on`): the loaded model, a switch skipped
//!   rather than waited on, never a single-slot server, and any failure
//!   speaks the reply undirected. Only the cue differs (`Cue::Reply`).
//! - **A direction already given is given again.** An earlier tap's, kept
//!   under the reply's key, or — for a reply spoken in a call — the call's
//!   first line (`saved`). Read back rather than asked for, so a reply heard
//!   twice sounds the same and costs no model call.
//! - **Recorded as a call's are, except in an incognito chat**: one
//!   `Record::SpokenDirection` per reply under `listen:<key>`
//!   (`voice_direction::LISTEN_TURN`). An incognito chat has no transcript
//!   by type, and nothing here writes a journal line about one. The reply's
//!   state lives on its chat (`WebSession::listen`), so it goes when the
//!   chat does.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mecha_core::session::{Record, Session};
use mecha_core::voice_direction::{self as vd, Cue, Scene, SpokenDirection};

/// The longest key a page may name a reply by.
const MAX_KEY: usize = 32;

/// How much of a reply, the owner's words and the speaker's last reply is
/// kept from a tap; the prompt bounds each shorter still.
const MAX_CONTEXT: usize = 2_000;

/// The fewest words a reply needs before a call's direction is taken for
/// it: "Okay." was said in every call.
const MIN_CALL_WORDS: usize = 4;

/// How long serve waits for the worker to say whether its engine takes
/// directions.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// What a tap says about the reply its piece is in (`SpeakBody::listen`).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ListenCue {
    /// The page's key for the reply: the same text, the same key.
    pub reply: String,
    /// The piece's position in the reply, from 0. A 0 starts the reply
    /// again, as a second tap does.
    pub index: u32,
    /// The whole reply as it is spoken: what the director reads.
    #[serde(default)]
    pub whole: Option<String>,
    /// The owner's words the reply answers.
    #[serde(default)]
    pub asked: Option<String>,
    /// What the speaker said before this reply.
    #[serde(default)]
    pub last_reply: Option<String>,
}

impl ListenCue {
    /// A key a record can carry: short, and nothing but letters, digits,
    /// `-` and `_`.
    pub fn valid(&self) -> bool {
        !self.reply.is_empty()
            && self.reply.len() <= MAX_KEY
            && self
                .reply
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }
}

fn bounded(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(|t| t.chars().take(MAX_CONTEXT).collect::<String>())
        .filter(|t| !t.trim().is_empty())
}

/// One reply being read aloud in a chat. Held on the chat's session
/// (`WebSession::listen`, `PersonaSession::listen`); a new reply, or the
/// same one from its first piece, replaces it.
pub(crate) struct Reply {
    key: String,
    whole: String,
    scene: Scene,
    /// What the transcript already holds for this reply (`saved`).
    saved: Option<String>,
    /// The direction every piece is spoken with, settled once — `None`
    /// inside for a reply spoken undirected.
    direction: tokio::sync::OnceCell<Option<String>>,
}

impl Reply {
    pub(crate) fn new(cue: &ListenCue, character: Option<String>, saved: Option<String>) -> Self {
        Reply {
            key: cue.reply.clone(),
            whole: bounded(&cue.whole).unwrap_or_default(),
            scene: Scene {
                character,
                last_reply: bounded(&cue.last_reply),
                utterance: bounded(&cue.asked).unwrap_or_default(),
                last_direction: None,
            },
            saved,
            direction: tokio::sync::OnceCell::new(),
        }
    }

    /// Whether a tap continues this reply rather than starting one.
    pub(crate) fn continues(&self, cue: &ListenCue) -> bool {
        cue.index != 0 && self.key == cue.reply
    }
}

/// What a chat lends a tap: the reply, and where its direction is recorded
/// — `None` for an incognito chat (`Recording::kept`).
pub(crate) struct Seat {
    pub(crate) reply: Arc<Reply>,
    pub(crate) transcript: Option<Arc<Session>>,
}

/// The direction a transcript already holds for the reply keyed `key` whose
/// text is `whole`: an earlier tap's, else that of the call turn which
/// spoke it — the one turn holding at least half the reply's sentences,
/// its first line. Only lines the director wrote count.
pub(crate) fn saved(records: &[SpokenDirection], key: &str, whole: &str) -> Option<String> {
    let lines = || {
        records
            .iter()
            .filter(|d| d.outcome == vd::Outcome::Ok)
            .filter_map(|d| d.direction.as_ref().map(|line| (d, line)))
    };
    let own = format!("{}{key}", vd::LISTEN_TURN);
    if let Some((_, line)) = lines().rfind(|(d, _)| d.turn == own) {
        return Some(line.clone());
    }
    if normalise(whole).split(' ').count() < MIN_CALL_WORDS {
        return None;
    }
    let wanted: HashSet<String> = sentences(whole)
        .map(normalise)
        .filter(|s| !s.is_empty())
        .collect();
    let mut turns: HashMap<&str, CallTurn> = HashMap::new();
    for d in records
        .iter()
        .filter(|d| !d.turn.starts_with(vd::LISTEN_TURN))
    {
        let t = turns.entry(d.turn.as_str()).or_default();
        if wanted.contains(&normalise(&d.sentence)) {
            t.spoke += 1;
        }
    }
    for (d, line) in lines() {
        if let Some(t) = turns.get_mut(d.turn.as_str()) {
            if t.first.is_none_or(|(at, _)| d.index < at) {
                t.first = Some((d.index, line));
            }
        }
    }
    let best = turns.values().map(|t| t.spoke).max()?;
    if best == 0 || best * 2 < wanted.len() {
        return None;
    }
    let mut top = turns.values().filter(|t| t.spoke == best);
    let (Some(chosen), None) = (top.next(), top.next()) else {
        return None;
    };
    chosen.first.map(|(_, line)| line.clone())
}

/// One call turn, as `saved` weighs it.
#[derive(Default)]
struct CallTurn<'a> {
    /// How many of the reply's sentences it spoke.
    spoke: usize,
    /// Its earliest directed line, with that sentence's index.
    first: Option<(u32, &'a String)>,
}

/// The direction a transcript holds for a reply (`saved`), read off the
/// runtime's threads. Unreadable is nothing saved: the director is asked, as
/// for a reply never heard.
pub(crate) async fn recall(transcript: &Session, cue: &ListenCue) -> Option<String> {
    let path = transcript.path.clone();
    let key = cue.reply.clone();
    let whole = bounded(&cue.whole).unwrap_or_default();
    tokio::task::spawn_blocking(move || match Session::spoken_directions(&path) {
        Ok(records) => saved(&records, &key, &whole),
        Err(e) => {
            tracing::debug!("listen: the transcript's directions were not read: {e:#}");
            None
        }
    })
    .await
    .ok()
    .flatten()
}

/// A reply's sentences, cut where the page cuts them: after a run of `.`,
/// `!` or `?` that a space or the end follows.
fn sentences(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text.trim();
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let bytes = rest.as_bytes();
        let mut end = rest.len();
        let mut i = 0;
        while i < bytes.len() {
            if matches!(bytes[i], b'.' | b'!' | b'?') {
                let mut j = i;
                while j < bytes.len() && matches!(bytes[j], b'.' | b'!' | b'?') {
                    j += 1;
                }
                if j == bytes.len() || bytes[j].is_ascii_whitespace() {
                    end = j;
                    break;
                }
                i = j;
            } else {
                i += 1;
            }
        }
        let (sentence, tail) = rest.split_at(end);
        rest = tail.trim_start();
        Some(sentence)
    })
}

/// Letters and digits, lowercased, one space between words: a call's
/// sentence and a tap's match although the page tidied the reply's marks
/// and the worker did not.
fn normalise(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Whether the worker beside `target` speaks with directions: its engine
/// honours `instructions` (`GET /mecha/directs`). A worker that predates the
/// route, or does not answer, is a no — the reply is spoken as Listen always
/// spoke it, and no model is asked for a line nobody would hear.
pub(crate) async fn worker_directs(target: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(target).and_then(|t| t.join("/mecha/directs")) else {
        return false;
    };
    let Ok(resp) = reqwest::Client::new()
        .get(url)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
    else {
        return false;
    };
    if !resp.status().is_success() {
        return false;
    }
    resp.json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v["directs"].as_bool())
        .unwrap_or(false)
}

/// The `instructions` the seat's reply is spoken with, or `None` to speak it
/// undirected: settled by the first piece to ask, and the same for every
/// piece after.
pub(crate) async fn direct(
    follower: &crate::follow::Follower,
    stopping: &tokio_util::sync::CancellationToken,
    seat: &Seat,
    voice: Option<&str>,
) -> Option<String> {
    seat.reply
        .direction
        .get_or_init(|| settle(follower, stopping, seat, voice))
        .await
        .clone()
}

/// The reply's one direction: saved, or asked for and recorded when the
/// chat keeps a transcript. A saved line is not recorded again.
async fn settle(
    follower: &crate::follow::Follower,
    stopping: &tokio_util::sync::CancellationToken,
    seat: &Seat,
    voice: Option<&str>,
) -> Option<String> {
    let reply = &seat.reply;
    if let Some(line) = &reply.saved {
        return Some(line.clone());
    }
    let asked_at = Instant::now();
    let norm = normalise(&reply.whole);
    let (answer, digest) = if norm.is_empty() || is_harness(&norm) {
        (crate::voice::DirectorAsked::Skipped("harness"), None)
    } else {
        let user = vd::prompt(&reply.scene, &[], Cue::Reply(&reply.whole));
        let deadline = vd::FIRST_SENTENCE_DEADLINE;
        let answer = crate::voice::ask_director_on(follower, stopping, &user, deadline).await;
        (answer, Some(vd::digest(&user)))
    };
    let (outcome, line, model, reason) = answer.outcome();
    let direction = line.map(|l| vd::sent(&l));
    // An incognito chat has no transcript, and nothing below it is reached.
    let transcript = seat.transcript.as_ref()?;
    let latency_ms = asked_at.elapsed().as_millis() as u64;
    let record = SpokenDirection {
        ts: Some(chrono::Utc::now()),
        turn: format!("{}{}", vd::LISTEN_TURN, reply.key),
        context: None,
        index: 0,
        sentence: reply.whole.clone(),
        opening: false,
        direction: direction.clone(),
        model,
        latency_ms,
        outcome,
        reason,
        voice: voice.map(str::to_string),
        prompt_digest: digest,
    };
    if let Err(e) = transcript.append(&Record::SpokenDirection(record)) {
        tracing::debug!("a listen direction was not recorded: {e:#}");
    }
    tracing::debug!(?outcome, latency_ms, "listen direction");
    direction
}

/// Words the harness says in a persona's place — the crisis pause's fixed
/// message, which a call does not direct either.
fn is_harness(reply: &str) -> bool {
    normalise(mecha_core::persona::safety::SAFE_MESSAGE).contains(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(turn: &str, index: u32, sentence: &str, direction: &str) -> SpokenDirection {
        SpokenDirection {
            turn: turn.into(),
            index,
            sentence: sentence.into(),
            direction: Some(vd::sent(direction)),
            outcome: vd::Outcome::Ok,
            ..Default::default()
        }
    }

    fn cue(reply: &str, index: u32) -> ListenCue {
        ListenCue {
            reply: reply.into(),
            index,
            whole: Some("The dig went well.".into()),
            asked: Some("How did the dig go?".into()),
            last_reply: None,
        }
    }

    #[test]
    fn a_key_is_short_and_plain() {
        assert!(cue("r-1a2b_3c", 0).valid());
        assert!(!cue("", 0).valid());
        assert!(!cue("has space", 0).valid());
        assert!(!cue("listen:nested", 0).valid());
        assert!(!cue(&"k".repeat(MAX_KEY + 1), 0).valid());
    }

    #[test]
    fn a_tap_continues_its_reply_and_a_first_piece_starts_again() {
        let r = Reply::new(&cue("abc", 0), None, None);
        assert!(r.continues(&cue("abc", 3)));
        assert!(!r.continues(&cue("abc", 0)), "a second tap starts over");
        assert!(!r.continues(&cue("xyz", 3)), "another reply");
    }

    #[test]
    fn sentences_are_cut_where_the_page_cuts_them() {
        let got: Vec<_> = sentences("It cost 3.5 million. Wow!! Fine... ok").collect();
        assert_eq!(got, ["It cost 3.5 million.", "Wow!!", "Fine...", "ok"]);
        assert_eq!(sentences("  ").count(), 0);
        assert_eq!(
            normalise("It went **really** well — thanks!"),
            normalise("it went really well, thanks.")
        );
    }

    #[test]
    fn an_earlier_tap_is_saved_by_key_and_never_another_replys() {
        let records = [
            line("listen:xyz", 0, "The dig went well.", "flat"),
            line("listen:abc", 0, "The dig went well.", "warm"),
        ];
        assert_eq!(
            saved(&records, "abc", "The dig went well."),
            Some(vd::sent("warm"))
        );
        assert_eq!(saved(&records, "new", "The dig went well."), None);
    }

    #[test]
    fn a_call_lends_its_first_line_to_the_reply_it_spoke() {
        let whole = "The grant went in this morning. Now go and rest.";
        let records = [
            line("chatcmpl-1", 0, "Okay.", "plain"),
            line("chatcmpl-1", 1, "Now go and rest.", "stern"),
            line(
                "chatcmpl-2",
                1,
                "The grant went in this morning.",
                "relieved",
            ),
            line("chatcmpl-2", 0, "Oh, you did it!", "bright"),
            line("chatcmpl-2", 2, "Now go and rest.", "soft"),
        ];
        // Turn 2 spoke both sentences; its earliest line is index 0's.
        assert_eq!(saved(&records, "abc", whole), Some(vd::sent("bright")));
        // Two turns tied: neither.
        let tied = [
            line("chatcmpl-1", 0, "The grant went in this morning.", "a"),
            line("chatcmpl-2", 0, "The grant went in this morning.", "b"),
        ];
        assert_eq!(saved(&tied, "abc", whole), None);
        // A short reply is never tied to a call.
        assert_eq!(saved(&records, "abc", "Okay."), None);
        // Less than half the reply was spoken there: not this reply.
        let other = "The grant went in this morning. Then lunch. Then a walk. Then bed.";
        assert_eq!(saved(&records, "abc", other), None);
    }

    #[test]
    fn only_a_line_the_director_wrote_is_saved() {
        let mut timed_out = line("listen:abc", 0, "The dig went well.", "warm");
        timed_out.outcome = vd::Outcome::Timeout;
        timed_out.direction = None;
        assert_eq!(saved(&[timed_out], "abc", "The dig went well."), None);
    }

    /// An incognito chat's direction leaves no record and no journal line
    /// naming it, its reply or its direction — the bar `voice::direct`
    /// holds a call to.
    #[test]
    fn an_incognito_direction_is_neither_recorded_nor_traced() {
        let src = include_str!("listen.rs");
        let body = src.split("\n#[cfg(test)]").next().unwrap();
        let handler = body
            .split("async fn settle(")
            .nth(1)
            .expect("the handler")
            .split("\nfn is_harness")
            .next()
            .unwrap();
        let gate = handler
            .find("seat.transcript.as_ref()?")
            .expect("the incognito gate");
        let append = handler.find(".append(").expect("the record");
        let first_trace = handler.find("tracing::").expect("a journal line");
        assert!(gate < append && gate < first_trace, "the gate comes first");
        for (at, _) in handler.match_indices("tracing::") {
            let statement = handler[at..].split(';').next().unwrap();
            for text in ["whole", "direction)", "?direction", "voice", "scene", "key"] {
                assert!(!statement.contains(text), "{statement} names {text}");
            }
        }
    }
}
