//! Which Slack events have already been handled, across a restart.
//!
//! Acks happen before the handler runs and Slack's redelivery across a
//! dropped socket is undocumented (`mecha_slack::socket`), so the connector
//! dedupes on `event_id`. Held only in memory, that memory ended with the
//! process: an event redelivered to the restarted connector was a first
//! sighting again, and a message already answered started a second run.
//!
//! `~/.mecha/slack/seen-events` is one id per line, appended as each is first
//! seen. It names no user, channel or text — an event id is Slack's own
//! opaque label.
//!
//! **A dedupe, never a gate**: a file that cannot be read or written is a
//! warning and the ring carries on in memory, which is exactly what there was
//! before. Refusing events because their ids could not be recorded would turn
//! a disk fault into a connector that answers nobody.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;

/// How many event ids are remembered. Slack's redelivery semantics across a
/// dropped socket are undocumented, so handlers are idempotent and this is
/// what makes them so.
pub const SEEN_EVENTS: usize = 512;

/// The file is rewritten down to the ring once it holds this many lines, so
/// an append-only file does not grow for the life of the install.
const COMPACT_AT: usize = SEEN_EVENTS * 4;

pub struct SeenEvents {
    ring: VecDeque<String>,
    path: Option<PathBuf>,
    /// Lines in the file, counted rather than re-read on every append.
    lines: usize,
}

impl SeenEvents {
    /// The default store, beside the rest of the Slack state. A home that
    /// cannot be resolved yields a ring that only remembers in memory.
    pub fn open_default() -> Self {
        match mecha_core::work::mecha_home() {
            Ok(home) => Self::open(home.join("slack").join("seen-events")),
            Err(e) => {
                tracing::warn!("no mecha home — seen Slack events will not survive a restart: {e}");
                Self::in_memory()
            }
        }
    }

    /// A ring with no file behind it.
    pub fn in_memory() -> Self {
        Self {
            ring: VecDeque::new(),
            path: None,
            lines: 0,
        }
    }

    /// Load the newest [`SEEN_EVENTS`] ids from `path`. A missing file is an
    /// empty ring; an unreadable one is a warning and an empty ring, with the
    /// path kept so later sightings are still recorded.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut ring = VecDeque::new();
        let mut lines = 0;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                for id in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                    lines += 1;
                    ring.push_back(id.to_string());
                    if ring.len() > SEEN_EVENTS {
                        ring.pop_front();
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(
                "could not read {} — events seen before this start are not remembered: {e}",
                path.display()
            ),
        }
        let mut seen = Self {
            ring,
            path: Some(path),
            lines,
        };
        if seen.lines >= COMPACT_AT {
            seen.compact();
        }
        seen
    }

    /// `true` the first time an id is offered, `false` after — including
    /// after a restart. An empty id cannot be deduplicated and always passes.
    pub fn first_time(&mut self, event_id: &str) -> bool {
        if event_id.is_empty() {
            return true;
        }
        if self.ring.iter().any(|s| s == event_id) {
            return false;
        }
        self.ring.push_back(event_id.to_string());
        if self.ring.len() > SEEN_EVENTS {
            self.ring.pop_front();
        }
        self.record(event_id);
        true
    }

    fn record(&mut self, event_id: &str) {
        let Some(path) = self.path.clone() else {
            return;
        };
        if !one_row(event_id) {
            return;
        }
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                mecha_slack::store::create_private_dir(dir)?;
            }
            let fresh = !path.exists();
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            if fresh {
                mecha_slack::store::set_owner_only(&path)?;
            }
            writeln!(file, "{event_id}")
        };
        match write() {
            Ok(()) => {
                self.lines += 1;
                if self.lines >= COMPACT_AT {
                    self.compact();
                }
            }
            Err(e) => tracing::warn!("could not record a seen Slack event: {e}"),
        }
    }

    /// Rewrite the file as the ring: a sibling written whole, then renamed
    /// over it, so a crash leaves the old file or the new one and never half.
    fn compact(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        let tmp = path.with_extension("tmp");
        let write = || -> std::io::Result<usize> {
            let mut body = String::new();
            let mut rows = 0;
            for id in self.ring.iter().filter(|id| one_row(id)) {
                body.push_str(id);
                body.push('\n');
                rows += 1;
            }
            std::fs::write(&tmp, body)?;
            mecha_slack::store::set_owner_only(&tmp)?;
            std::fs::rename(&tmp, path)?;
            Ok(rows)
        };
        match write() {
            Ok(rows) => self.lines = rows,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                tracing::warn!("could not compact {}: {e}", path.display());
            }
        }
    }
}

/// An id is one line of the file. Slack's are alphanumeric; one that is not
/// would be read back as two ids, so it is remembered in memory only — by
/// both writers, the append and the compaction.
fn one_row(event_id: &str) -> bool {
    !event_id.contains(['\n', '\r'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mecha-slack-seen-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("seen-events")
    }

    #[test]
    fn an_event_seen_before_a_restart_is_not_a_first_sighting_after_it() {
        let path = scratch("restart");
        let mut before = SeenEvents::open(&path);
        assert!(before.first_time("Ev1"));
        assert!(!before.first_time("Ev1"));
        drop(before);

        let mut after = SeenEvents::open(&path);
        assert!(!after.first_time("Ev1"), "the restart forgot the event");
        assert!(after.first_time("Ev2"));
    }

    #[test]
    fn an_in_memory_ring_forgets_across_a_restart() {
        // The negative of the test above: without the file, the same
        // sequence does repeat — so that test is measuring the file.
        let mut before = SeenEvents::in_memory();
        assert!(before.first_time("Ev1"));
        let mut after = SeenEvents::in_memory();
        assert!(after.first_time("Ev1"));
    }

    #[test]
    fn an_empty_id_always_passes_and_is_never_written() {
        let path = scratch("empty");
        let mut seen = SeenEvents::open(&path);
        assert!(seen.first_time(""));
        assert!(seen.first_time(""));
        assert!(!path.exists());
    }

    #[test]
    fn the_ring_keeps_only_the_newest_ids_across_a_restart() {
        let path = scratch("ring");
        let mut seen = SeenEvents::open(&path);
        for i in 0..SEEN_EVENTS + 1 {
            assert!(seen.first_time(&format!("Ev{i}")));
        }
        drop(seen);
        let mut after = SeenEvents::open(&path);
        assert!(after.first_time("Ev0"), "the oldest id fell off the ring");
        assert!(!after.first_time(&format!("Ev{SEEN_EVENTS}")));
    }

    #[test]
    fn the_file_is_compacted_to_the_ring_rather_than_growing_forever() {
        let path = scratch("compact");
        let mut seen = SeenEvents::open(&path);
        for i in 0..COMPACT_AT {
            assert!(seen.first_time(&format!("Ev{i}")));
        }
        let lines = std::fs::read_to_string(&path).unwrap().lines().count();
        assert_eq!(lines, SEEN_EVENTS);
        // Compaction kept the newest and still appends after it.
        assert!(!seen.first_time(&format!("Ev{}", COMPACT_AT - 1)));
        assert!(seen.first_time("EvNext"));
        drop(seen);
        let mut after = SeenEvents::open(&path);
        assert!(!after.first_time("EvNext"));
    }

    #[test]
    fn an_id_with_a_newline_is_deduplicated_in_memory_and_forges_no_row() {
        let path = scratch("newline");
        let mut seen = SeenEvents::open(&path);
        assert!(seen.first_time("Ev1\nEv2"));
        assert!(!seen.first_time("Ev1\nEv2"));
        drop(seen);
        let mut after = SeenEvents::open(&path);
        assert!(
            after.first_time("Ev2"),
            "a forged row was read back as seen"
        );
    }

    #[test]
    fn compaction_writes_no_forged_row_either() {
        // The ring holds the id whole; the file must hold none of it, on the
        // rewrite as on the append.
        let path = scratch("newline-compact");
        let mut seen = SeenEvents::open(&path);
        for i in 0..COMPACT_AT - 2 {
            assert!(seen.first_time(&format!("Ev{i}")));
        }
        assert!(seen.first_time("EvA\nEvB"));
        assert!(seen.first_time("EvX"));
        assert!(seen.first_time("EvY"), "this append trips the compaction");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), SEEN_EVENTS - 1, "the rewrite ran");
        drop(seen);
        let mut after = SeenEvents::open(&path);
        assert!(
            after.first_time("EvB"),
            "a forged row was read back as seen"
        );
        assert!(!after.first_time("EvY"));
    }

    #[test]
    fn an_unwritable_path_degrades_to_memory_and_still_deduplicates() {
        // The parent is a file, so the store can never be created.
        let dir = scratch("unwritable");
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        std::fs::write(&dir, "").unwrap();
        let mut seen = SeenEvents::open(dir.join("seen-events"));
        assert!(seen.first_time("Ev1"));
        assert!(!seen.first_time("Ev1"));
    }
}
