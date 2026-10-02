//! How much a persona's reply repeats one it already gave in the same chat,
//! read back from the chat's transcript.
//!
//! **Why it exists.** On 2026-10-02 a persona chat sent the same reply
//! several times over, and a replay found why (a pinned seed and its own
//! preserved thinking, both since taken out of persona chats). One session
//! is a small sample to tune on, so the fix was the defaults and this is
//! the measurement: every persona, every model, typed and spoken, read by
//! `mecha persona show` — so the next change to how personas sample or
//! prompt is judged on what chats did, not on one replay.
//!
//! **Offline, on purpose.** Nothing acts on the number while a chat runs,
//! and the session file already records every reply, compacted ones
//! included. So nothing is computed per turn and nothing new is stored: a
//! reading is taken from the transcripts when asked, and a better metric
//! later recomputes the whole history rather than living with what an
//! earlier one wrote down.
//!
//! **The number.** Of a reply's runs of [`RUN`] words, the share that also
//! appear in the single earlier reply it overlaps most. Word runs, not
//! characters or tokens: a lowercased word sequence is what reads as "the
//! same thing again" and is stable across tokenizers. 1.0 is a copy. In the
//! session that prompted this the copies scored 0.71–1.00; replayed with
//! the fix, no reply passed 0.42.
//!
//! **Nothing to compare is not zero.** A first reply, or one too short to
//! hold a single run, has no echo — `None`, and it is not counted.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::message::{Message, Role};
use crate::session::{Record, Session};

/// Words per run. Four is short enough that a reworded reply shares few and
/// long enough that "I've got you" alone is not a run.
pub const RUN: usize = 4;

/// At or above this echo a reply counts as repeating an earlier one: past
/// half its runs, it is the earlier reply with edits rather than a new one.
pub const REPEATED: f64 = 0.5;

/// The window `mecha persona show` reads: chats active in the last this
/// many days.
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

/// The text of an assistant message with something to say.
fn reply_text(m: &Message) -> Option<String> {
    let text = m.text();
    (m.role == Role::Assistant && !text.trim().is_empty()).then_some(text)
}

/// The echo of each completed run's reply in one session file, in order.
///
/// Walks the records as they were written: a run's messages (appended, or
/// a `Rewrite` holding the whole history when it compacted), then its
/// `Outcome`. The run's reply is the last assistant text before that
/// outcome. It is scored only when the outcome says the run completed — a
/// stopped run keeps a partial reply, which would read low, and a
/// judge-stopped one was never shown; an outcome with no known cause is not
/// a completed one. Every reply, scored or not, joins what later replies
/// are compared with, since the owner saw it.
pub fn session_echoes(path: &Path) -> Result<Vec<f64>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut said: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut pending: Option<String> = None;
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str(line) {
            Ok(Record::Message(m)) => {
                if let Some(reply) = reply_text(&m) {
                    pending = Some(reply);
                }
            }
            // The whole history after a compaction (or a rollback). Only its
            // last message can be this run's reply; everything before it
            // was said in earlier runs and is already in `said`.
            Ok(Record::Rewrite { messages }) => {
                if let Some(reply) = messages.last().and_then(reply_text) {
                    pending = Some(reply);
                }
            }
            Ok(Record::Outcome(stats)) => {
                let Some(reply) = pending.take() else {
                    continue;
                };
                if stats.stop_cause == Some(crate::agent::StopCause::Completed) {
                    if let Some(e) = echo(&reply, said.iter().map(String::as_str)) {
                        out.push(e);
                    }
                }
                if seen.insert(reply.clone()) {
                    said.push(reply);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// What one persona's chats say, over a window.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Echoes {
    /// Chats read.
    pub chats: usize,
    /// Replies scored.
    pub replies: usize,
    /// Of them, at or above [`REPEATED`].
    pub repeated: usize,
    /// The highest echo; `None` when nothing was scored.
    pub max: Option<f64>,
    /// Session files that could not be read — counted, never silently an
    /// empty chat.
    pub unreadable: usize,
}

/// The echoes of every chat in `sessions` (a persona's `sessions/`) last
/// written at or after `since`. No folder is no chats.
pub fn echoes(sessions: &Path, since: std::time::SystemTime) -> Result<Echoes> {
    let (listed, unreadable) = Session::list_counting(sessions)?;
    let mut out = Echoes {
        unreadable,
        ..Echoes::default()
    };
    for (_, path) in listed {
        let active = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t >= since);
        if !active {
            continue;
        }
        let Ok(scores) = session_echoes(&path) else {
            out.unreadable += 1;
            continue;
        };
        out.chats += 1;
        for e in scores {
            out.replies += 1;
            if e >= REPEATED {
                out.repeated += 1;
            }
            out.max = Some(out.max.map_or(e, |m: f64| m.max(e)));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Conversation, StopCause};
    use crate::message::Block;
    use crate::session::{RunStats, SessionMeta};

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

    const SAME: &str = "The kelp line runs north of the second buoy today";
    const NEW: &str = "Nobody told the gulls so they shouted at the harbour";

    fn reply(t: &str) -> Message {
        Message::assistant(vec![Block::text(t)])
    }

    /// A run's outcome as the session records it.
    fn outcome(s: &Session, stop: StopCause) {
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(stop),
            ..RunStats::default()
        }))
        .unwrap();
    }

    fn scratch() -> (std::path::PathBuf, Session) {
        let dir = std::env::temp_dir().join(format!("mecha-echo-{}", uuid::Uuid::new_v4()));
        let meta = SessionMeta {
            id: "20260101T000000-echo".into(),
            created_at: chrono::Utc::now(),
            provider: "scripted".into(),
            model: "test-model".into(),
            workspace: dir.clone(),
            title: None,
            kind: None,
        };
        let s = Session::create(&dir, meta).unwrap();
        (dir, s)
    }

    /// One run as a persona chat records it: the turn and its reply, then
    /// the outcome.
    fn run(s: &Session, convo: &mut Conversation, said: &str, answer: &str, stop: StopCause) {
        let before = convo.messages.clone();
        convo.push(Message::user(said));
        convo.push(reply(answer));
        s.record_run(&before, convo).unwrap();
        outcome(s, stop);
    }

    #[test]
    fn a_transcript_scores_each_completed_reply_against_every_one_before_it() {
        let (dir, s) = scratch();
        let mut convo = Conversation::default();
        run(&s, &mut convo, "hi", SAME, StopCause::Completed);
        run(&s, &mut convo, "and?", NEW, StopCause::Completed);
        // A compaction summarises the first two away; the copy of the first
        // is still a copy.
        let before = convo.messages.clone();
        convo.messages = vec![Message::user("[summary]"), Message::user("mm"), reply(SAME)];
        s.record_run(&before, &convo).unwrap();
        outcome(&s, StopCause::Completed);
        // A stopped run's partial reply is not scored.
        run(&s, &mut convo, "stop", SAME, StopCause::Stopped);

        let scores = session_echoes(&s.path).unwrap();
        assert_eq!(scores.len(), 2, "first: nothing to compare; last: stopped");
        assert!(scores[0] < 0.2, "{scores:?}");
        assert_eq!(scores[1], 1.0, "the summarised-away reply still counts");
        let file = std::fs::read_to_string(&s.path).unwrap();
        assert!(file.contains("\"record\":\"rewrite\""), "it compacted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_window_reads_active_chats_and_counts_what_it_cannot_read() {
        let (dir, s) = scratch();
        let mut convo = Conversation::default();
        run(&s, &mut convo, "hi", SAME, StopCause::Completed);
        run(&s, &mut convo, "mm", SAME, StopCause::Completed);
        let long_ago = std::time::SystemTime::UNIX_EPOCH;
        let e = echoes(&dir, long_ago).unwrap();
        assert_eq!(
            (e.chats, e.replies, e.repeated, e.max, e.unreadable),
            (1, 1, 1, Some(1.0), 0)
        );
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        assert_eq!(echoes(&dir, later).unwrap().chats, 0, "outside the window");
        assert_eq!(
            echoes(&dir.join("nowhere"), long_ago).unwrap(),
            Echoes::default()
        );
        std::fs::write(dir.join("20260101T000001-torn.jsonl"), "not json\n").unwrap();
        assert_eq!(echoes(&dir, long_ago).unwrap().unreadable, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
