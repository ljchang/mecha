//! The persona safety layer's model-free half (`docs/PERSONA-DESIGN.md` §12):
//! the crisis sensor's keyword tier, the plain-voice message it hands over
//! to, the re-anchor text, and the two content-free records — the crisis
//! counter and the dose meter.
//!
//! **It lives in the harness, not the persona** (§12): persona prompts do not
//! hold, and a model's own safety behaviour degrades over long conversations.
//! Nothing here is a line in a persona's prompt except the re-anchor, which
//! is the owner's own Core, handed back.
//!
//! **A check that cannot run is never silent.** The keyword tier needs no
//! model, so it always runs; the model tiers (a classifier and a judge, step
//! 2c-2) are not built yet, and every surface says so — `CrisisState::Degraded`
//! is "keywords only", never "passed".

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::sections;

/// Which detector made a crisis record: the tier and its revision. Bumped
/// whenever the phrase list changes, so a count can be read against the
/// detector that produced it (§12.2's counter carries it for that reason).
pub const DETECTOR: &str = "keywords-1";

/// First-person phrases that name wanting to die or to hurt oneself. A
/// *conservative* list: a keyword tier cannot read context, so it is kept to
/// phrases whose ordinary meaning is the one that matters, and the model
/// tiers (2c-2) are what will read the rest. Matched on normalised text
/// (lowercase, apostrophes straightened, whitespace collapsed) as whole
/// phrases bounded by non-letters.
const PHRASES: &[&str] = &[
    "kill myself",
    "killing myself",
    "end my life",
    "ending my life",
    "take my own life",
    "taking my own life",
    "want to die",
    "wanna die",
    "wish i was dead",
    "wish i were dead",
    "better off dead",
    "better off without me",
    "no reason to live",
    "nothing to live for",
    "don't want to be alive",
    "dont want to be alive",
    "don't want to live",
    "dont want to live",
    "don't want to be here anymore",
    "suicide",
    "suicidal",
    "hurt myself",
    "hurting myself",
    "harm myself",
    "self-harm",
    "self harm",
    "cut myself",
    "cutting myself",
    "overdose",
];

fn normalise(text: &str) -> String {
    let lowered = text.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'");
    lowered.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether the owner's text trips the keyword tier: a listed phrase, not
/// inside a longer word ("suicide" in "suicidesquad" does not count, and
/// neither does "overdose" in "overdosed" — past tense about someone else is
/// left to the model tiers).
pub fn keyword_hit(text: &str) -> bool {
    let text = normalise(text);
    let bytes = text.as_bytes();
    let boundary = |i: Option<&u8>| i.is_none_or(|b| !b.is_ascii_alphabetic());
    PHRASES.iter().any(|phrase| {
        text.match_indices(phrase).any(|(at, m)| {
            boundary(at.checked_sub(1).and_then(|i| bytes.get(i)))
                && boundary(bytes.get(at + m.len()))
        })
    })
}

/// What the plain voice says when the sensor fires — not in character, not
/// the persona's words, and never sent anywhere. Safe-messaging shape: it
/// names the feeling without alarm, says help exists now, and gives ways to
/// reach it (US numbers; the owner is in the US).
pub const SAFE_MESSAGE: &str = "\
This is mecha, not the character you were talking with. It sounds like you \
might be going through something really hard right now. You don't have to \
face it alone.

If you're thinking about suicide or hurting yourself, you can call or text \
988 (the Suicide & Crisis Lifeline) any time, or text HOME to 741741 to \
reach the Crisis Text Line. If you're in immediate danger, call 911.

When you're ready, you can close this and carry on the conversation.";

/// How long after a pause a further hit in the same chat does not pause
/// again: without it, talking through something hard would stop the persona
/// at every message. The resources stay on the page either way.
pub const CRISIS_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// What the crisis sensor can do in a chat right now — said on every
/// surface, so "switched off" and "couldn't check" never pass for each
/// other (§12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrisisState {
    /// The owner switched it off for this persona.
    Off,
    /// Only the keyword tier runs: the model tiers are unbuilt (2c-2), or
    /// cannot reach a model.
    Degraded,
}

pub fn crisis_state(crisis_on: bool) -> CrisisState {
    if crisis_on {
        CrisisState::Degraded
    } else {
        CrisisState::Off
    }
}

/// One line of `~/.mecha/personas/safety.jsonl`: a crisis hit, content-free
/// — when, which tier, which surface, which detector, and whether the persona
/// paused. No persona, no chat, no words: this is all a host's report needs
/// and all this stores (§12.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrisisRecord {
    pub at: String,
    pub check: String,
    pub tier: String,
    pub surface: String,
    pub detector: String,
    pub paused: bool,
}

fn append_line(path: &Path, line: &str) -> Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    writeln!(f, "{line}").with_context(|| format!("writing {}", path.display()))
}

/// Count a crisis hit. `dir` is the persona store.
pub fn record_crisis(dir: &Path, surface: &str, paused: bool) -> Result<()> {
    let record = CrisisRecord {
        at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        check: "crisis".into(),
        tier: "keyword".into(),
        surface: surface.into(),
        detector: DETECTOR.into(),
        paused,
    };
    append_line(&dir.join("safety.jsonl"), &serde_json::to_string(&record)?)
}

/// One owner turn with a persona, for the dose meters (§12.3). The time and
/// the persona, and whether the chat's crisis sensor was degraded when it was
/// taken — §12's "the dose record says so for that chat". Never the words.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoseRecord {
    pub at: chrono::DateTime<chrono::Utc>,
    pub persona: String,
    pub chat: String,
    pub crisis: CrisisStateWire,
}

/// [`CrisisState`] as the dose record keeps it: a closed enum on disk, so an
/// unknown value reads as degraded — the cautious reading — rather than
/// failing the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CrisisStateWire {
    Off,
    #[default]
    #[serde(other)]
    Degraded,
}

impl From<CrisisState> for CrisisStateWire {
    fn from(s: CrisisState) -> Self {
        match s {
            CrisisState::Off => CrisisStateWire::Off,
            CrisisState::Degraded => CrisisStateWire::Degraded,
        }
    }
}

/// Record a turn. `dir` is the persona store.
pub fn record_dose(dir: &Path, persona: &str, chat: &str, crisis: CrisisState) -> Result<()> {
    let record = DoseRecord {
        at: chrono::Utc::now(),
        persona: persona.into(),
        chat: chat.into(),
        crisis: crisis.into(),
    };
    append_line(&dir.join("dose.jsonl"), &serde_json::to_string(&record)?)
}

/// What the dose meters say about one persona, as of `now`, in the owner's
/// zone. Late night is 23:00–05:00 local.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct Dose {
    pub turns_today: u32,
    pub turns_7d: u32,
    pub late_night_7d: u32,
}

pub fn is_late_night(hour: u32) -> bool {
    !(5..23).contains(&hour)
}

/// Read the meters for `persona` (or every persona, with `None`). An
/// unreadable line is skipped rather than failing the read; a missing file
/// is no turns.
pub fn dose(
    dir: &Path,
    persona: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
    tz: chrono_tz::Tz,
) -> Dose {
    let all = doses(dir, now, tz);
    match persona {
        Some(p) => all.get(p).copied().unwrap_or_default(),
        None => all.values().fold(Dose::default(), |a, d| Dose {
            turns_today: a.turns_today + d.turns_today,
            turns_7d: a.turns_7d + d.turns_7d,
            late_night_7d: a.late_night_7d + d.late_night_7d,
        }),
    }
}

/// Every persona's meters in one walk of the file — what a list of
/// personas reads, rather than one parse per row (review of #418).
pub fn doses(
    dir: &Path,
    now: chrono::DateTime<chrono::Utc>,
    tz: chrono_tz::Tz,
) -> std::collections::HashMap<String, Dose> {
    let mut out: std::collections::HashMap<String, Dose> = Default::default();
    let Ok(text) = std::fs::read_to_string(dir.join("dose.jsonl")) else {
        return out;
    };
    let today = now.with_timezone(&tz).date_naive();
    let week_ago = now - chrono::Duration::days(7);
    for r in text
        .lines()
        .filter_map(|l| serde_json::from_str::<DoseRecord>(l).ok())
    {
        let out = out.entry(r.persona.clone()).or_default();
        let local = r.at.with_timezone(&tz);
        if local.date_naive() == today {
            out.turns_today += 1;
        }
        if r.at > week_ago && r.at <= now {
            out.turns_7d += 1;
            use chrono::Timelike;
            if is_late_night(local.hour()) {
                out.late_night_7d += 1;
            }
        }
    }
    out
}

/// How the re-anchor begins — registered in `agent::is_harness_voice`, so no
/// reader takes it for the owner's words and no page draws it in their
/// bubble.
pub const REANCHOR_STEM: &str = "(A reminder from the harness of who you are";

/// How many owner turns between re-anchors (§12.5).
pub const REANCHOR_EVERY: u32 = 8;

/// The re-anchor: the persona's `## Core`, handed back in the message stream
/// near the newest turn — never the system prompt, so the cached prefix is
/// untouched. `None` when there is no Core to hand back.
pub fn reanchor_text(identity: &str) -> Option<String> {
    let core = sections(identity)
        .into_iter()
        .find(|s| s.heading.eq_ignore_ascii_case("core"))?
        .body;
    let core = core.trim();
    (!core.is_empty()).then(|| format!("{REANCHOR_STEM}, in the owner's words: {core})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyword_tier_catches_first_person_phrases() {
        for hit in [
            "I want to die",
            "honestly i just wanna die",
            "I've been thinking about suicide",
            "sometimes I feel suicidal",
            "I don’t want to be alive anymore",
            "everyone would be better off without me",
            "I cut myself last night",
            "I keep thinking about how to end my life.",
            "Self-harm is back",
        ] {
            assert!(keyword_hit(hit), "missed: {hit}");
        }
    }

    #[test]
    fn the_keyword_tier_leaves_ordinary_words_alone() {
        for miss in [
            "this is a killer app",
            "I'm dying to see the results",
            "the plot kills me",
            "suicidesquad was a bad film",
            "we analysed the overdosed samples",
            "my phone battery wants to die",
            "",
        ] {
            // "wants to die" is not a listed phrase; "want to die" needs its
            // own word boundary, so it does not match inside "wants to die".
            assert!(!keyword_hit(miss), "false hit: {miss}");
        }
    }

    #[test]
    fn the_safe_message_gives_the_resources_and_is_not_in_character() {
        for needed in ["988", "741741", "911", "not the character"] {
            assert!(SAFE_MESSAGE.contains(needed), "{needed}");
        }
    }

    #[test]
    fn the_crisis_record_carries_no_content() {
        let dir = std::env::temp_dir().join(format!("mecha-safety-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        record_crisis(&dir, "web", true).unwrap();
        let text = std::fs::read_to_string(dir.join("safety.jsonl")).unwrap();
        let value: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            vec!["at", "check", "detector", "paused", "surface", "tier"]
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_dose_meters_count_today_the_week_and_late_nights() {
        use chrono::TimeZone;
        let dir = std::env::temp_dir().join(format!("mecha-dose-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tz: chrono_tz::Tz = "America/New_York".parse().unwrap();
        let at = |d: u32, h: u32| tz.with_ymd_and_hms(2026, 9, d, h, 0, 0).unwrap().to_utc();
        let lines: Vec<String> = [
            (29, 10, "mara"),
            (29, 23, "mara"),
            (28, 2, "mara"),
            (20, 12, "mara"), // over a week ago
            (29, 11, "ada"),
        ]
        .iter()
        .map(|(d, h, p)| {
            serde_json::to_string(&DoseRecord {
                at: at(*d, *h),
                persona: (*p).into(),
                chat: "c".into(),
                crisis: CrisisStateWire::Degraded,
            })
            .unwrap()
        })
        .collect();
        std::fs::write(dir.join("dose.jsonl"), lines.join("\n") + "\nnot json\n").unwrap();
        let now = at(29, 23) + chrono::Duration::minutes(30);
        assert_eq!(
            dose(&dir, Some("mara"), now, tz),
            Dose {
                turns_today: 2,
                turns_7d: 3,
                late_night_7d: 2
            }
        );
        assert_eq!(dose(&dir, None, now, tz).turns_today, 3);
        assert_eq!(dose(&dir, Some("nobody"), now, tz), Dose::default());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_unknown_crisis_state_on_disk_reads_as_degraded() {
        let r: DoseRecord = serde_json::from_str(
            r#"{"at":"2026-09-29T10:00:00Z","persona":"m","chat":"c","crisis":"judged"}"#,
        )
        .unwrap();
        assert_eq!(r.crisis, CrisisStateWire::Degraded);
    }

    #[test]
    fn the_reanchor_is_the_core_and_nothing_else() {
        let identity =
            "# Mara\n<!-- note -->\n## Core\nDry and precise.\n<!-- was: x -->\n## Background\nKelp.\n";
        let text = reanchor_text(identity).unwrap();
        assert!(text.contains("Dry and precise."), "{text}");
        assert!(!text.contains("Kelp") && !text.contains("was:"), "{text}");
        assert!(
            crate::agent::is_harness_voice(&text),
            "the re-anchor must read as the harness's"
        );
        assert!(reanchor_text("## Core\n\n## Other\nx").is_none());
        assert!(reanchor_text("no sections").is_none());
    }
}
