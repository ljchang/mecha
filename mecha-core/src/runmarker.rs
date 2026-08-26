//! "Is a run in flight, and please stop it" — as two files in a directory.
//!
//! Lifted out of `trigger.rs` when `mecha tasks work` needed the same thing,
//! because the mechanism is four subtle rules and two copies is two places for
//! one of them to rot:
//!
//! - **A marker, not the flock.** The obvious way to ask "is it running?" is to
//!   try to claim the lock and see — but that acquires and drops it, so a UI
//!   polling the question would occasionally hold the lock at the instant a
//!   scheduler tried to fire and cause a spurious overlap skip. Watching must
//!   never perturb what is watched.
//! - **A marker whose process is gone is a crashed run, not a running one.**
//!   It is cleaned up and reported absent, so a hard kill cannot leave
//!   something looking permanently busy in every surface that asks. That rests
//!   entirely on the pid range check in [`crate::process_alive`]: `kill(-1, 0)`
//!   succeeds and would report every dead run as alive.
//! - **Cancel is a file, never a signal.** The run may be inside a caller's own
//!   process — a trigger firing in the daemon — where SIGTERM would take the
//!   whole scheduler down. The runner polls for the file and cancels its own
//!   token, which stops at the next safe point with the partial answer intact:
//!   the same path as Ctrl-C and the timeout, rather than a kill that discards
//!   the very thing cancellation exists to preserve.
//! - **Clearing removes both files.** A cancel that arrives as a run is ending
//!   must not be left lying around to kill the *next* one.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Who is running something right now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMarker {
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    /// The scheduled slot this run is accounting for, when it has one.
    /// Absent for anything a person started by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<DateTime<Utc>>,
}

/// A directory of run markers, keyed by whatever the caller calls its runs.
pub struct RunMarkers {
    dir: PathBuf,
}

impl RunMarkers {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        RunMarkers { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn marker_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.running"))
    }

    fn cancel_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.cancel"))
    }

    /// Announce that a run has started, for anything that wants to *display*
    /// whether one is in flight.
    pub fn mark_running(&self, name: &str, slot: Option<DateTime<Utc>>) -> Result<()> {
        crate::create_private_dir(&self.dir)?;
        // **A run starts uncancelled, whatever was left lying around.**
        // `clear` removes both files, but a cancel written in the window
        // between `request_cancel`'s liveness check and the previous run's
        // `clear` survives it — as does one left by a SIGKILL or a reboot.
        // `cancel_requested` is a bare existence check, so the next run would
        // stop itself two seconds in and report a near-empty partial that
        // looks exactly like a model giving up.
        let _ = std::fs::remove_file(self.cancel_path(name));
        let marker = RunMarker {
            pid: std::process::id(),
            started_at: Utc::now(),
            slot,
        };
        let path = self.marker_path(name);
        let tmp = path.with_extension("running.tmp");
        std::fs::write(&tmp, serde_json::to_string(&marker)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Clear the marker and any unclaimed cancel request.
    pub fn clear(&self, name: &str) {
        let _ = std::fs::remove_file(self.marker_path(name));
        let _ = std::fs::remove_file(self.cancel_path(name));
    }

    /// The run in flight, if there is one.
    pub fn running(&self, name: &str) -> Option<RunMarker> {
        let text = std::fs::read_to_string(self.marker_path(name)).ok()?;
        let marker: RunMarker = serde_json::from_str(&text).ok()?;
        if crate::process_alive(marker.pid) {
            Some(marker)
        } else {
            self.clear(name);
            None
        }
    }

    /// Ask the run in flight to stop. `false` when there is nothing to stop,
    /// so a caller can say so rather than pretending it did something.
    pub fn request_cancel(&self, name: &str) -> Result<bool> {
        if self.running(name).is_none() {
            return Ok(false);
        }
        crate::create_private_dir(&self.dir)?;
        std::fs::write(self.cancel_path(name), Utc::now().to_rfc3339())?;
        Ok(true)
    }

    /// Has a cancel been requested for the run in flight?
    pub fn cancel_requested(&self, name: &str) -> bool {
        self.cancel_path(name).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mecha-runmarker-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_marker_reports_its_own_run_and_clears() {
        let m = RunMarkers::new(scratch("basic"));
        assert!(m.running("a").is_none());
        m.mark_running("a", None).unwrap();
        assert_eq!(m.running("a").unwrap().pid, std::process::id());
        m.clear("a");
        assert!(m.running("a").is_none());
    }

    /// A hard kill must not leave something looking busy forever. This is the
    /// check that rests on the pid range guard: `kill(-1, 0)` succeeds, so a
    /// naive `process_alive` would report every dead run as alive.
    #[test]
    fn a_marker_whose_process_is_gone_reads_as_not_running() {
        let m = RunMarkers::new(scratch("dead"));
        crate::create_private_dir(m.dir()).unwrap();
        // pid 0 is never a live process this could be, and is exactly the
        // value the range check exists to refuse.
        std::fs::write(
            m.dir().join("a.running"),
            serde_json::json!({"pid": 0, "started_at": Utc::now().to_rfc3339()}).to_string(),
        )
        .unwrap();
        assert!(m.running("a").is_none(), "a dead pid is not a running run");
        assert!(
            !m.dir().join("a.running").exists(),
            "and the stale marker is swept on the way past"
        );
    }

    /// A cancel that outlived the run it was meant for must not reach the
    /// next one. Fails on the old `mark_running`, which wrote the marker and
    /// left whatever cancel was already there.
    #[test]
    fn a_stale_cancel_does_not_reach_the_next_run() {
        let m = RunMarkers::new(scratch("stalecancel"));
        crate::create_private_dir(m.dir()).unwrap();
        // The shape a kill or a lost race leaves behind: a cancel with no
        // marker beside it.
        std::fs::write(m.dir().join("a.cancel"), "whenever").unwrap();
        assert!(m.cancel_requested("a"));

        m.mark_running("a", None).unwrap();
        assert!(
            !m.cancel_requested("a"),
            "the new run must not inherit the old run's stop"
        );
    }

    #[test]
    fn cancelling_nothing_says_so_rather_than_pretending() {
        let m = RunMarkers::new(scratch("nothing"));
        assert!(!m.request_cancel("a").unwrap());
        assert!(!m.cancel_requested("a"));
    }

    /// A cancel arriving as a run ends must not be left lying around to kill
    /// the next one.
    #[test]
    fn clearing_removes_an_unclaimed_cancel_too() {
        let m = RunMarkers::new(scratch("stale"));
        m.mark_running("a", None).unwrap();
        assert!(m.request_cancel("a").unwrap());
        assert!(m.cancel_requested("a"));
        m.clear("a");
        assert!(!m.cancel_requested("a"), "the next run starts uncancelled");
    }
}
