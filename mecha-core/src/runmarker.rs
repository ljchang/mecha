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
    /// The transcript this run is writing, when it has one.
    ///
    /// **So another process can ask "does a live run own this session?"
    /// without asking the run.** A `Conversation` — messages and taint — lives
    /// in the memory of the process holding it and the session JSONL has one
    /// writer, so anything that would pick a transcript up (`mecha chat
    /// --resume`, `/api/resume`) has to be able to find out that somebody
    /// already has it. The in-process check those surfaces already do cannot
    /// see a detached child, and the board can only say a run is in flight,
    /// not which file it is appending to.
    ///
    /// Defaulted on load, like every other field written to a store that
    /// outlives a release: a marker from a run that started before this field
    /// existed reads as "no session named", which is what it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
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

    fn steer_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.steer"))
    }

    /// Announce that a run has started, for anything that wants to *display*
    /// whether one is in flight.
    pub fn mark_running(&self, name: &str, slot: Option<DateTime<Utc>>) -> Result<()> {
        self.mark_running_for(name, slot, None)
    }

    /// The same, naming the transcript this run is writing — see
    /// [`RunMarker::session`] for why anything else would have to ask the run.
    pub fn mark_running_for(
        &self,
        name: &str,
        slot: Option<DateTime<Utc>>,
        session: Option<&str>,
    ) -> Result<()> {
        crate::create_private_dir(&self.dir)?;
        // **A run starts uncancelled, whatever was left lying around.**
        // `clear` removes both files, but a cancel written in the window
        // between `request_cancel`'s liveness check and the previous run's
        // `clear` survives it — as does one left by a SIGKILL or a reboot.
        // `cancel_requested` is a bare existence check, so the next run would
        // stop itself two seconds in and report a near-empty partial that
        // looks exactly like a model giving up.
        let _ = std::fs::remove_file(self.cancel_path(name));
        // **And uninstructed, for the same reason.** A steer queued in the
        // window before the previous run's `clear`, or left by a kill, would
        // otherwise be drained into the *next* run's first turn — an
        // instruction about work that is already over, arriving as though the
        // owner had just typed it.
        let _ = std::fs::remove_file(self.steer_path(name));
        let marker = RunMarker {
            pid: std::process::id(),
            started_at: Utc::now(),
            slot,
            session: session.map(str::to_string),
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
        let _ = std::fs::remove_file(self.steer_path(name));
    }

    /// Which live run, if any, is writing this transcript.
    ///
    /// **The cross-process half of "one conversation, one writer".** Every
    /// surface that picks a session back up already refuses to mint a twin of
    /// one *this* process holds; none of them could see a detached child, so
    /// resuming a delegation mid-flight would have given one JSONL two
    /// writers — the child appending its turns and the reader appending the
    /// owner's. Dead markers are swept by [`Self::running`] on the way past,
    /// so a crashed run does not lock its transcript out forever.
    ///
    /// Returns the run's name (a task id, here), because a caller that has to
    /// refuse should be able to say what it is refusing for.
    pub fn live_writer_of(&self, session: &str) -> Option<String> {
        let names: Vec<String> = std::fs::read_dir(&self.dir)
            .ok()?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".running"))
                    .map(str::to_string)
            })
            .collect();
        names.into_iter().find(|name| {
            self.running(name)
                .and_then(|m| m.session)
                .is_some_and(|s| s == session)
        })
    }

    /// Queue an instruction for the run in flight, to be folded into the
    /// message carrying its next tool results.
    ///
    /// **Appended, never overwritten.** Two instructions typed a second apart
    /// are two things the owner meant; a file that held only the newest would
    /// drop the first silently, which is the failure a queue exists to
    /// prevent. One JSON string per line, so a newline in the text cannot
    /// split one instruction into two.
    ///
    /// `false` when nothing is running, exactly as [`Self::request_cancel`]
    /// reports it — a steer written for a run that will never read it is not
    /// a queued instruction, it is a file waiting to ambush the next run.
    pub fn queue_steer(&self, name: &str, text: &str) -> Result<bool> {
        if self.running(name).is_none() {
            return Ok(false);
        }
        crate::create_private_dir(&self.dir)?;
        let mut line = serde_json::to_string(text)?;
        line.push('\n');
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.steer_path(name))?;
        f.write_all(line.as_bytes())?;
        Ok(true)
    }

    /// Take everything queued, leaving nothing behind.
    ///
    /// **Drained rather than read**, because this module has already learned
    /// what a file left lying around does: a steer that survived its own
    /// delivery would be re-folded into every later turn, so one sentence
    /// would arrive again and again for the rest of the run.
    ///
    /// A line that will not parse is skipped rather than failing the drain —
    /// the alternative is one malformed byte silencing every instruction
    /// behind it, and the caller is a poller with nowhere to report to.
    pub fn take_steer(&self, name: &str) -> Vec<String> {
        let path = self.steer_path(name);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Vec::new();
        };
        let _ = std::fs::remove_file(&path);
        text.lines()
            .filter_map(|l| serde_json::from_str::<String>(l).ok())
            .collect()
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

    /// The cross-process half of "one conversation, one writer": a reader in
    /// another process can find out that a live run owns a transcript, which
    /// is the only thing standing between `resume` and two writers on one
    /// JSONL. A dead marker must not lock a transcript out forever, so the
    /// sweep in `running` is load-bearing here too.
    #[test]
    fn a_live_marker_names_the_transcript_it_is_writing() {
        let m = RunMarkers::new(scratch("writer"));
        m.mark_running_for("task-1", None, Some("20260826T1200-abc"))
            .unwrap();
        assert_eq!(
            m.live_writer_of("20260826T1200-abc").as_deref(),
            Some("task-1"),
            "the owner is findable by the file it is writing"
        );
        assert!(
            m.live_writer_of("20260826T1200-other").is_none(),
            "and only that file"
        );
        m.clear("task-1");
        assert!(
            m.live_writer_of("20260826T1200-abc").is_none(),
            "a finished run releases its transcript"
        );
    }

    /// A marker written before the field existed reads as naming no session,
    /// which is what it was — the store outlives the release, so a missing
    /// field must load rather than fail the record.
    #[test]
    fn a_marker_without_a_session_still_loads() {
        let m = RunMarkers::new(scratch("oldmarker"));
        crate::create_private_dir(m.dir()).unwrap();
        std::fs::write(
            m.dir().join("t.running"),
            serde_json::json!({"pid": std::process::id(), "started_at": Utc::now().to_rfc3339()})
                .to_string(),
        )
        .unwrap();
        assert!(m.running("t").is_some(), "it is still a running run");
        assert!(m.live_writer_of("anything").is_none());
    }

    /// Two instructions typed a second apart are two things the owner meant.
    /// Overwriting would drop the first silently, which is the one failure a
    /// queue exists to prevent.
    #[test]
    fn steers_queue_up_and_drain_exactly_once() {
        let m = RunMarkers::new(scratch("steer"));
        m.mark_running("t", None).unwrap();
        assert!(m.queue_steer("t", "check the dates first").unwrap());
        assert!(m.queue_steer("t", "and use\nthe short form").unwrap());
        assert_eq!(
            m.take_steer("t"),
            vec!["check the dates first", "and use\nthe short form"],
            "both, in order, and a newline does not split one into two"
        );
        assert!(
            m.take_steer("t").is_empty(),
            "drained, or one sentence arrives on every later turn for the rest                  of the run"
        );
    }

    /// A steer for a run that is not there is not a queued instruction — it
    /// is a file waiting to ambush the next run, which is exactly what the
    /// stale-cancel test above was written for.
    #[test]
    fn a_steer_needs_a_run_to_steer_and_never_outlives_one() {
        let m = RunMarkers::new(scratch("steerstale"));
        assert!(
            !m.queue_steer("t", "too late").unwrap(),
            "nothing running, so nothing queued — and the caller is told"
        );
        m.mark_running("t", None).unwrap();
        assert!(m.take_steer("t").is_empty(), "and nothing was written");

        // The shape a kill leaves: a steer with no run to consume it.
        m.queue_steer("t", "from the run that died").unwrap();
        m.mark_running("t", None).unwrap();
        assert!(
            m.take_steer("t").is_empty(),
            "a new run starts uninstructed, like it starts uncancelled"
        );
    }

    /// **The board can outlive its run, and this is the witness.** `tasks
    /// work` restores a task's status on every exit path it controls; a
    /// `SIGKILL` controls none of them, so the graph goes on saying the agent
    /// holds the task forever and every surface reading only the board
    /// renders a dead run as one in flight. The marker answers it locally and
    /// within seconds, which is the whole reason a surface should ask.
    #[test]
    fn a_task_the_board_says_is_held_can_have_no_run_behind_it() {
        let m = RunMarkers::new(scratch("stalled"));
        m.mark_running_for("task-1", None, Some("s-1")).unwrap();
        assert!(
            m.running("task-1").is_some(),
            "while the run lives, the board's claim is corroborated"
        );

        // What a kill leaves: the marker's process is gone and nothing
        // restored the board. Written as a dead pid rather than by killing
        // something, for the same reason the test above is.
        crate::create_private_dir(m.dir()).unwrap();
        std::fs::write(
            m.dir().join("task-1.running"),
            serde_json::json!({"pid": 0, "started_at": Utc::now().to_rfc3339(), "session": "s-1"})
                .to_string(),
        )
        .unwrap();
        assert!(
            m.running("task-1").is_none(),
            "the claim is now uncorroborated, and a reader can say so"
        );
        assert!(
            m.live_writer_of("s-1").is_none(),
            "and the transcript it was writing is free — a killed run must not \
             lock its own conversation out of being resumed"
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
