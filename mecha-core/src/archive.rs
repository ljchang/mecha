//! Archived sessions: out of the conversation list, still on the record.
//!
//! **Archiving is filing, not forgetting** (owner's ruling, 2026-09-28). The
//! transcript is untouched and every reader that walks the store — the
//! corpus, reflect, distill, appraisal — keeps reading it; the only surface
//! that consults the mark is the one that lists conversations to pick up.
//! Forgetting is [`crate::forget`]'s, and is a different act entirely.
//!
//! The mark is a file per session in `<sessions>/.archived/`, holding when it
//! was archived. A file in a directory rather than a field in the transcript
//! or a shared index, on the `runmarker`/`permit` pattern: archiving and
//! restoring are a create and a remove, so two processes never race a
//! read-modify-write, and the transcript — an append-only record of what was
//! said — never grows a line about how the owner filed it. The directory is
//! dotted and inside the store, so it moves with `MECHA_SESSION_DIR` and is
//! invisible to [`crate::session::Session::list`], which reads `.jsonl` only.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const DIR: &str = ".archived";

fn dir(sessions: &Path) -> PathBuf {
    sessions.join(DIR)
}

/// A session id is the file name here, so it must be one: the shape
/// [`crate::session::Session::new_id`] mints, and nothing that could walk out
/// of the directory.
fn marker(sessions: &Path, id: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "invalid session id {id:?}"
    );
    Ok(dir(sessions).join(id))
}

/// Archive `id`. Idempotent: archiving an archived session keeps the first
/// date, because that is when it left the list.
pub fn archive(sessions: &Path, id: &str, now: DateTime<Utc>) -> Result<()> {
    let path = marker(sessions, id)?;
    anyhow::ensure!(
        sessions.join(format!("{id}.jsonl")).is_file(),
        "no session {id}"
    );
    if path.exists() {
        return Ok(());
    }
    crate::create_private_dir(&dir(sessions))
        .with_context(|| format!("creating {}", dir(sessions).display()))?;
    std::fs::write(&path, now.to_rfc3339()).with_context(|| format!("writing {}", path.display()))
}

/// Put `id` back in the list. `false` when it was not archived.
pub fn unarchive(sessions: &Path, id: &str) -> Result<bool> {
    let path = marker(sessions, id)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// Every archived session and when it was archived. A marker whose date
/// cannot be read is still archived — the file is the mark, the date is a
/// detail — and reads as `None`.
pub fn archived(sessions: &Path) -> Result<HashMap<String, Option<DateTime<Utc>>>> {
    let mut out = HashMap::new();
    let read = match std::fs::read_dir(dir(sessions)) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e).context("reading the archive"),
    };
    for entry in read {
        let entry = entry?;
        let Some(id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let at = std::fs::read_to_string(entry.path())
            .ok()
            .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
            .map(|d| d.with_timezone(&Utc));
        out.insert(id, at);
    }
    Ok(out)
}

pub fn is_archived(sessions: &Path, id: &str) -> bool {
    marker(sessions, id).is_ok_and(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(name: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("mecha-archive-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("20260928T120000-aaaa.jsonl"), "{}\n").unwrap();
        Scratch(dir)
    }

    #[test]
    fn archiving_marks_without_touching_the_transcript() {
        let d = store("marks");
        let id = "20260928T120000-aaaa";
        let before = std::fs::read(d.path().join(format!("{id}.jsonl"))).unwrap();
        archive(d.path(), id, Utc::now()).unwrap();
        assert!(is_archived(d.path(), id));
        assert_eq!(
            std::fs::read(d.path().join(format!("{id}.jsonl"))).unwrap(),
            before
        );
        // Invisible to the store's own listing, which reads `.jsonl` only.
        let listed: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
            .collect();
        assert_eq!(listed.len(), 1);
        assert!(unarchive(d.path(), id).unwrap());
        assert!(!is_archived(d.path(), id));
        assert!(!unarchive(d.path(), id).unwrap());
    }

    #[test]
    fn archiving_twice_keeps_the_first_date() {
        let d = store("first-date");
        let id = "20260928T120000-aaaa";
        let first = DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        archive(d.path(), id, first).unwrap();
        archive(d.path(), id, Utc::now()).unwrap();
        assert_eq!(archived(d.path()).unwrap()[id], Some(first));
    }

    #[test]
    fn an_id_is_a_file_name_and_nothing_else() {
        let d = store("ids");
        assert!(archive(d.path(), "../escape", Utc::now()).is_err());
        assert!(archive(d.path(), "20260928T120000-none", Utc::now()).is_err());
        assert!(!is_archived(d.path(), "../../etc/passwd"));
    }
}
