//! How much a persona's reply repeats one it already gave in the same chat
//! — one number per reply, never the words.
//!
//! **Why it exists.** On 2026-10-02 a persona chat sent the same reply
//! several times over, and a replay found why (a pinned seed and its own
//! preserved thinking, both since taken out of persona chats). One session
//! is a small sample to tune on, so the fix was the defaults and this was
//! the measurement: every persona, every model, typed and spoken, read
//! back with `mecha persona show` — so the next change to how personas
//! sample or prompt is judged on what chats did, not on one replay.
//!
//! **The number.** Of the reply's runs of [`RUN`] words, the share that
//! also appear in the single earlier reply it overlaps most. Word runs, not
//! characters or tokens: a lowercased word sequence is what reads as "the
//! same thing again" and is stable across tokenizers. 1.0 is a copy. In
//! the session that prompted this the copies scored 0.71–1.00; replayed
//! with the fix, no reply passed 0.42.
//!
//! **Nothing to compare is not zero.** A first reply, or one too short to
//! hold a single run, has no echo — `None`, and no record is written.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Words per run. Four is short enough that a reworded reply shares few and
/// long enough that "I've got you" alone is not a run.
pub const RUN: usize = 4;

/// At or above this echo a reply counts as repeating an earlier one: past
/// half its runs, it is the earlier reply with edits rather than a new one.
pub const REPEATED: f64 = 0.5;

/// The window `mecha persona show` reads.
pub const SHOWN_DAYS: i64 = 7;

fn runs(text: &str) -> HashSet<Vec<String>> {
    let words: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    words.windows(RUN).map(<[String]>::to_vec).collect()
}

/// `reply`'s echo of the replies before it, or `None` when there is nothing
/// to compare: no earlier reply with a run in it, or a reply with none.
pub fn echo<'a>(reply: &str, earlier: impl IntoIterator<Item = &'a str>) -> Option<f64> {
    let mine = runs(reply);
    if mine.is_empty() {
        return None;
    }
    earlier
        .into_iter()
        .map(runs)
        .filter(|theirs| !theirs.is_empty())
        .map(|theirs| mine.intersection(&theirs).count() as f64 / mine.len() as f64)
        .fold(None, |best: Option<f64>, e| {
            Some(best.map_or(e, |b| b.max(e)))
        })
}

/// One reply's echo. Its own file, as `calls.jsonl` is beside
/// `dose.jsonl`: a line here must never read as one more turn to the dose
/// meters of an older binary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EchoRecord {
    pub at: chrono::DateTime<chrono::Utc>,
    pub persona: String,
    pub chat: String,
    pub echo: f64,
}

/// Record a reply's echo. `dir` is the persona store.
pub fn record_echo(dir: &Path, persona: &str, chat: &str, echo: f64) -> Result<()> {
    let record = EchoRecord {
        at: chrono::Utc::now(),
        persona: persona.into(),
        chat: chat.into(),
        echo: echo.clamp(0.0, 1.0),
    };
    super::safety::append_line(&dir.join("echo.jsonl"), &serde_json::to_string(&record)?)
}

/// What the echo records say about one persona since a moment.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Echoes {
    /// Replies measured.
    pub replies: usize,
    /// Of them, at or above [`REPEATED`].
    pub repeated: usize,
    /// The highest echo; `None` when nothing was measured.
    pub max: Option<f64>,
    /// Lines that did not parse, or held no usable number — counted, never
    /// silently dropped. An unparseable line names no persona, so every
    /// persona's reading counts it.
    pub skipped: usize,
}

/// The echo records for `persona` since `since`. No file is no replies;
/// an unreadable one is an error, never an empty reading.
pub fn echoes(dir: &Path, persona: &str, since: chrono::DateTime<chrono::Utc>) -> Result<Echoes> {
    let path = dir.join("echo.jsonl");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Echoes::default()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut out = Echoes::default();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(r) = serde_json::from_str::<EchoRecord>(line) else {
            out.skipped += 1;
            continue;
        };
        if r.persona != persona || r.at < since {
            continue;
        }
        if !r.echo.is_finite() {
            out.skipped += 1;
            continue;
        }
        out.replies += 1;
        if r.echo >= REPEATED {
            out.repeated += 1;
        }
        out.max = Some(out.max.map_or(r.echo, |m: f64| m.max(r.echo)));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_is_one_and_a_new_reply_is_near_zero() {
        let earlier = "The tide went out early and the boats sat crooked on the mud all morning";
        assert_eq!(echo(earlier, [earlier]), Some(1.0));
        let fresh = "Nobody told the gulls, so they shouted at the empty harbour until noon";
        assert_eq!(echo(fresh, [earlier]), Some(0.0));
    }

    #[test]
    fn the_closest_earlier_reply_is_the_one_measured() {
        let a = "We walked the long way round past the old mill and the bakery";
        let b = "Something else entirely about weather and the price of coffee lately";
        let reply = "We walked the long way round past the old mill and the church";
        let e = echo(reply, [b, a]).unwrap();
        assert!(e > 0.8 && e < 1.0, "{e}");
    }

    #[test]
    fn case_and_punctuation_do_not_hide_a_copy() {
        let earlier = "Good. I'd keep going, slow and steady, just like that.";
        let again = "good i'd KEEP going — slow and steady; just like that";
        assert_eq!(echo(again, [earlier]), Some(1.0));
    }

    #[test]
    fn nothing_to_compare_is_none_never_zero() {
        assert_eq!(echo("a reply with enough words in it", []), None);
        assert_eq!(
            echo("too short", ["an earlier reply long enough to count"]),
            None
        );
        assert_eq!(echo("a reply with enough words in it", ["hm", "ok"]), None);
    }

    #[test]
    fn the_reader_counts_one_persona_in_its_window_and_skips_what_it_cannot_read() {
        let dir = std::env::temp_dir().join(format!("mecha-echo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let since = chrono::Utc::now() - chrono::Duration::days(SHOWN_DAYS);
        assert_eq!(echoes(&dir, "mara", since).unwrap(), Echoes::default());

        record_echo(&dir, "mara", "c1", 0.02).unwrap();
        record_echo(&dir, "mara", "c1", 0.91).unwrap();
        record_echo(&dir, "ada", "c2", 1.0).unwrap();
        let old = EchoRecord {
            at: since - chrono::Duration::hours(1),
            persona: "mara".into(),
            chat: "c0".into(),
            echo: 1.0,
        };
        let mut text = std::fs::read_to_string(dir.join("echo.jsonl")).unwrap();
        text.push_str(&serde_json::to_string(&old).unwrap());
        text.push_str("\nnot json\n");
        std::fs::write(dir.join("echo.jsonl"), text).unwrap();

        let e = echoes(&dir, "mara", since).unwrap();
        assert_eq!(
            (e.replies, e.repeated, e.max, e.skipped),
            (2, 1, Some(0.91), 1)
        );

        std::fs::remove_file(dir.join("echo.jsonl")).unwrap();
        std::fs::create_dir_all(dir.join("echo.jsonl")).unwrap();
        assert!(
            echoes(&dir, "mara", since).is_err(),
            "unreadable is a finding"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
