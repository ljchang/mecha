//! Session transcripts.
//!
//! One JSONL file per run: a header line describing the session, then one line
//! per message. Append-only, so a crashed run still leaves a readable
//! transcript, and `mecha sessions resume` can pick it back up.

use crate::agent::{Conversation, Taint};
use crate::message::{Message, Usage};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum Record {
    Meta(SessionMeta),
    Message(Message),
    /// Written when a run finishes, so `sessions show` can report cost without
    /// replaying the whole transcript.
    Summary { usage: Usage, turns: u32 },
    /// What had entered the conversation by this point.
    ///
    /// Recorded because it cannot be recovered by reading the transcript back:
    /// taint keys off *provenance* — whether a result actually came from
    /// outside — and the transcript stores only the content. Without this,
    /// resuming a session that had read a hostile page would hand the model
    /// that page again with the interlock disarmed.
    Taint(Taint),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub provider: String,
    pub model: String,
    pub workspace: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

pub struct Session {
    pub meta: SessionMeta,
    pub path: PathBuf,
}

impl Session {
    /// Where transcripts live: `~/.mecha/sessions`, or `$MECHA_SESSION_DIR`.
    pub fn default_dir() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("MECHA_SESSION_DIR") {
            return Ok(PathBuf::from(dir));
        }
        let home = dirs::home_dir().context("cannot determine home directory")?;
        Ok(home.join(".mecha").join("sessions"))
    }

    pub fn create(dir: &Path, meta: SessionMeta) -> Result<Self> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating session directory {}", dir.display()))?;
        let path = dir.join(format!("{}.jsonl", meta.id));
        let session = Session { meta: meta.clone(), path };
        session.append(&Record::Meta(meta))?;
        Ok(session)
    }

    pub fn new_id() -> String {
        // Sortable by name, and still unique when two runs start in the same
        // second.
        format!(
            "{}-{}",
            Utc::now().format("%Y%m%dT%H%M%S"),
            &uuid::Uuid::new_v4().to_string()[..8]
        )
    }

    pub fn append(&self, record: &Record) -> Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("opening {}", self.path.display()))?;
        writeln!(file, "{}", serde_json::to_string(record)?)?;
        Ok(())
    }

    pub fn append_messages(&self, messages: &[Message]) -> Result<()> {
        for m in messages {
            self.append(&Record::Message(m.clone()))?;
        }
        Ok(())
    }

    /// Read a transcript back, taint included.
    ///
    /// Unparseable lines are skipped rather than failing the load — a truncated
    /// final line is the normal result of a killed process.
    pub fn load(path: &Path) -> Result<(SessionMeta, Conversation)> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;

        let mut meta = None;
        let mut messages = Vec::new();
        let mut taint = Taint::default();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Record>(line) {
                Ok(Record::Meta(m)) => meta = Some(m),
                Ok(Record::Message(m)) => messages.push(m),
                // Merged rather than replaced: taint only ever grows, and a
                // transcript written by an older build has none at all.
                Ok(Record::Taint(t)) => taint.merge(t),
                Ok(Record::Summary { .. }) => {}
                Err(e) => tracing::warn!(error = %e, "skipping malformed transcript line"),
            }
        }

        let meta = meta.with_context(|| format!("{} has no session header", path.display()))?;
        Ok((meta, Conversation::resumed(messages, taint)))
    }

    /// Sessions in `dir`, newest first.
    pub fn list(dir: &Path) -> Result<Vec<(SessionMeta, PathBuf)>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            // A transcript with no header is unusable; skip it quietly.
            if let Ok((meta, _)) = Session::load(&path) {
                out.push((meta, path));
            }
        }
        out.sort_by_key(|(meta, _)| std::cmp::Reverse(meta.created_at));
        Ok(out)
    }

    /// Find a session by full id or unique prefix.
    pub fn find(dir: &Path, id_prefix: &str) -> Result<PathBuf> {
        let matches: Vec<_> = Session::list(dir)?
            .into_iter()
            .filter(|(m, _)| m.id.starts_with(id_prefix))
            .collect();
        match matches.len() {
            0 => anyhow::bail!("no session matching {id_prefix:?}"),
            1 => Ok(matches.into_iter().next().unwrap().1),
            n => anyhow::bail!("{id_prefix:?} matches {n} sessions; use a longer prefix"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Block;

    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mecha-session-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn meta_with_id(id: &str) -> SessionMeta {
        SessionMeta {
            id: id.to_string(),
            created_at: Utc::now(),
            provider: "scripted".into(),
            model: "test-model".into(),
            workspace: PathBuf::from("/tmp"),
            title: None,
        }
    }

    #[test]
    fn a_transcript_round_trips_its_messages_and_its_taint() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-round")).unwrap();
        session
            .append_messages(&[
                Message::user("summarise this page"),
                Message::assistant(vec![Block::text("done")]),
            ])
            .unwrap();
        session
            .append(&Record::Taint(Taint { private: true, untrusted: true }))
            .unwrap();

        let (meta, convo) = Session::load(&session.path).unwrap();

        assert_eq!(meta.model, "test-model");
        assert_eq!(convo.messages.len(), 2);
        assert_eq!(convo.messages[0].text(), "summarise this page");
        assert_eq!(convo.messages[1].text(), "done");
        // The whole point of recording it: provenance cannot be recovered by
        // re-reading the content, so a resumed conversation that had read a
        // hostile page must come back with the interlock still armed.
        assert!(convo.taint.trifecta_armed());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn taint_records_merge_so_a_later_clean_one_cannot_disarm_the_interlock() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-merge")).unwrap();

        // The order a real run writes them in: one leg arrives, then the other,
        // and the loop may checkpoint again with nothing new to say.
        session
            .append(&Record::Taint(Taint { untrusted: true, private: false }))
            .unwrap();
        session
            .append(&Record::Taint(Taint { private: true, untrusted: false }))
            .unwrap();
        session.append(&Record::Taint(Taint::default())).unwrap();

        let (_, convo) = Session::load(&session.path).unwrap();

        // Replacing rather than merging would leave this clean, and resuming
        // would hand the model the attacker's page with the guard switched off.
        assert!(convo.taint.private, "an earlier private leg was dropped");
        assert!(convo.taint.untrusted, "an earlier untrusted leg was dropped");
        assert!(convo.taint.trifecta_armed());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_transcript_written_before_taint_was_recorded_loads_clean() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-old")).unwrap();
        session.append_messages(&[Message::user("hello")]).unwrap();

        let (_, convo) = Session::load(&session.path).unwrap();

        assert_eq!(convo.messages.len(), 1);
        assert!(!convo.taint.private);
        assert!(!convo.taint.untrusted);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_truncated_final_line_does_not_lose_the_rest_of_the_transcript() {
        use std::io::Write;
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-killed")).unwrap();
        session.append_messages(&[Message::user("first")]).unwrap();
        session
            .append(&Record::Taint(Taint { private: true, untrusted: false }))
            .unwrap();

        // What a killed process leaves behind: a half-written final record.
        let mut file = std::fs::OpenOptions::new().append(true).open(&session.path).unwrap();
        write!(file, "{{\"record\":\"message\",\"role\":\"assis").unwrap();
        drop(file);

        let (_, convo) = Session::load(&session.path).unwrap();

        assert_eq!(convo.messages.len(), 1);
        assert_eq!(convo.messages[0].text(), "first");
        assert!(convo.taint.private, "a torn last line lost the taint before it");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_transcript_with_no_header_is_refused() {
        let dir = tmpdir();
        let path = dir.join("headerless.jsonl");
        let line = serde_json::to_string(&Record::Message(Message::user("orphan"))).unwrap();
        std::fs::write(&path, format!("{line}\n")).unwrap();

        let err = Session::load(&path).unwrap_err().to_string();
        assert!(err.contains("no session header"), "unexpected error: {err}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_ambiguous_id_prefix_is_an_error_rather_than_a_guess() {
        let dir = tmpdir();
        Session::create(&dir, meta_with_id("20260101T000000-aaaaaaaa")).unwrap();
        Session::create(&dir, meta_with_id("20260101T000000-bbbbbbbb")).unwrap();

        let err = Session::find(&dir, "20260101").unwrap_err().to_string();
        assert!(err.contains("matches 2 sessions"), "unexpected error: {err}");

        // A full id still resolves, and resuming the wrong transcript is the
        // failure being guarded against.
        let path = Session::find(&dir, "20260101T000000-aaaaaaaa").unwrap();
        assert!(path.ends_with("20260101T000000-aaaaaaaa.jsonl"));

        assert!(Session::find(&dir, "nothing-like-this").is_err());

        std::fs::remove_dir_all(&dir).ok();
    }
}
