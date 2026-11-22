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

use anyhow::{Context, Result};
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

/// Where a mecha home keeps its delegated-task run markers — said once, for
/// `mecha tasks`, the closure guard's walk over every guard home, and the
/// situation brief's reader.
pub fn task_dir_under(home: &Path) -> PathBuf {
    home.join("taskruns")
}

/// [`RunMarkers::live_names`]'s answer: the runs in flight, and how many
/// markers could not be read — each of which may be one.
#[derive(Debug, Default)]
pub struct LiveNames {
    pub names: Vec<String>,
    pub unreadable: usize,
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
        self.write_marker(name, &marker)
    }

    /// Name the transcript a run that already marked itself is writing — for
    /// a run that has to announce itself before its session exists (a
    /// trigger marks itself, then waits out a model switch, then creates its
    /// transcript).
    ///
    /// **Rewrites the marker only, never the cancel or steer files.**
    /// [`mark_running_for`](Self::mark_running_for) clears both because a run
    /// *starts* uncancelled; this one has already started, and a stop asked
    /// for while it waited for its model must still reach it.
    ///
    /// Refuses a marker that is not this process's, and one that is not
    /// there: naming a session on another run's marker would tell every
    /// reader that run owns a transcript it has never seen.
    pub fn name_session(&self, name: &str, session: &str) -> Result<()> {
        let path = self.marker_path(name);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading the run marker {}", path.display()))?;
        let mut marker: RunMarker = serde_json::from_str(&text)
            .with_context(|| format!("parsing the run marker {}", path.display()))?;
        if marker.pid != std::process::id() {
            anyhow::bail!(
                "the run marker for `{name}` belongs to process {}, not this one",
                marker.pid
            );
        }
        marker.session = Some(session.to_string());
        self.write_marker(name, &marker)
    }

    fn write_marker(&self, name: &str, marker: &RunMarker) -> Result<()> {
        let path = self.marker_path(name);
        let tmp = path.with_extension("running.tmp");
        std::fs::write(&tmp, serde_json::to_string(marker)?)?;
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

    /// [`running`](Self::running) for a reader that is not the run's owner:
    /// a dead marker is skipped, not cleared.
    pub fn peek(&self, name: &str) -> Option<RunMarker> {
        let text = std::fs::read_to_string(self.marker_path(name)).ok()?;
        let marker: RunMarker = serde_json::from_str(&text).ok()?;
        crate::process_alive(marker.pid).then_some(marker)
    }

    /// [`peek`](Self::peek) for a reader that must refuse when it cannot
    /// tell: no marker is `Ok(None)`, as is one whose process is gone, but a
    /// marker that is there and cannot be read or parsed is an error — it
    /// may be a live run, and `None` would say there is none.
    pub fn peek_checked(&self, name: &str) -> Result<Option<RunMarker>> {
        let path = self.marker_path(name);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(anyhow::Error::new(e)
                    .context(format!("reading the run marker {}", path.display())))
            }
        };
        let marker: RunMarker = serde_json::from_str(&text)
            .with_context(|| format!("parsing the run marker {}", path.display()))?;
        Ok(crate::process_alive(marker.pid).then_some(marker))
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

    /// The pids of every run in flight in this directory — what
    /// `closure::run_ancestor` checks a process's ancestry against. Read-only:
    /// a dead marker is skipped rather than cleared, because the caller is a
    /// check, not the run's owner. A directory that does not exist is no
    /// runs (a fresh install); one that exists and cannot be read is an
    /// error, because the closure guard's rule 1 reads this and an empty set
    /// there switches it off (review of #294 — the posture variable that used
    /// to be checked beside it is advisory now).
    pub fn live_pids(&self) -> Result<Vec<u32>> {
        let dir = match std::fs::read_dir(&self.dir) {
            Ok(dir) => dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(anyhow::Error::new(e)
                    .context(format!("reading run markers in {}", self.dir.display())))
            }
        };
        Ok(dir
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "running"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|t| serde_json::from_str::<RunMarker>(&t).ok())
            .map(|m| m.pid)
            .filter(|pid| crate::process_alive(*pid))
            .collect())
    }

    /// The names of every run in flight in this directory, sorted — the
    /// situation brief's "runs in flight" reader. Read-only like
    /// [`live_pids`](Self::live_pids) and fallible for the same reason: a
    /// directory that does not exist is no runs, one that cannot be read is
    /// an error, never an empty list. A marker that cannot be read or parsed
    /// is counted in `unreadable`, never dropped: it may be a live run
    /// (found on review, on `Permits::read_live`'s shape).
    pub fn live_names(&self) -> Result<LiveNames> {
        let dir = match std::fs::read_dir(&self.dir) {
            Ok(dir) => dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LiveNames::default()),
            Err(e) => {
                return Err(anyhow::Error::new(e)
                    .context(format!("reading run markers in {}", self.dir.display())))
            }
        };
        let mut out = LiveNames::default();
        for entry in dir.flatten() {
            let path = entry.path();
            if !path.extension().is_some_and(|x| x == "running") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string);
            let marker = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<RunMarker>(&t).ok());
            match (name, marker) {
                (Some(name), Some(m)) if crate::process_alive(m.pid) => out.names.push(name),
                (Some(_), Some(_)) => {}
                _ => out.unreadable += 1,
            }
        }
        out.names.sort();
        Ok(out)
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

    /// No directory is no runs; a directory that cannot be read is an error,
    /// never an empty set — the closure guard's rule 1 reads this (review of
    /// #294).
    #[test]
    fn an_unreadable_marker_directory_is_an_error_not_no_runs() {
        let base = std::env::temp_dir().join(format!("mecha-markers-{}", uuid::Uuid::new_v4()));
        assert!(RunMarkers::new(base.join("absent"))
            .live_pids()
            .unwrap()
            .is_empty());
        // A file where the directory should be fails `read_dir` without
        // depending on permission bits the test user might override.
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("file"), "x").unwrap();
        assert!(RunMarkers::new(base.join("file")).live_pids().is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

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

    /// A run that marks itself before its transcript exists names it later,
    /// and the naming keeps what the first mark set: its start, its slot,
    /// and a stop already asked for — `mark_running_for` would clear that,
    /// and a stop asked for while the run waited for its model would be
    /// lost. Another process's marker is never renamed.
    #[test]
    fn naming_a_session_keeps_the_start_the_slot_and_a_pending_stop() {
        let m = RunMarkers::new(scratch("namesession"));
        let slot = Utc::now() - chrono::Duration::minutes(3);
        m.mark_running("t", Some(slot)).unwrap();
        let first = m.running("t").unwrap();
        assert!(first.session.is_none());
        assert!(m.request_cancel("t").unwrap());

        m.name_session("t", "20261120T090000-named").unwrap();
        let named = m.running("t").unwrap();
        assert_eq!(named.session.as_deref(), Some("20261120T090000-named"));
        assert_eq!(named.started_at, first.started_at);
        assert_eq!(named.slot, Some(slot));
        assert!(m.cancel_requested("t"), "naming the session dropped a stop");
        assert_eq!(
            m.live_writer_of("20261120T090000-named").as_deref(),
            Some("t")
        );

        assert!(m.name_session("absent", "s").is_err(), "no marker to name");
        std::fs::write(
            m.dir().join("other.running"),
            serde_json::json!({"pid": 1, "started_at": Utc::now().to_rfc3339()}).to_string(),
        )
        .unwrap();
        assert!(m.name_session("other", "s").is_err(), "another process's");
    }

    /// The checked read refuses to say "nothing running" about a marker it
    /// could not read: absent and dead are `None`, garbage is an error.
    #[test]
    fn a_checked_peek_says_unreadable_rather_than_none() {
        let m = RunMarkers::new(scratch("checkedpeek"));
        assert!(m.peek_checked("a").unwrap().is_none(), "no directory");
        m.mark_running_for("a", None, Some("s-1")).unwrap();
        assert_eq!(
            m.peek_checked("a").unwrap().unwrap().session.as_deref(),
            Some("s-1")
        );
        std::fs::write(
            m.dir().join("dead.running"),
            serde_json::json!({"pid": 0, "started_at": Utc::now().to_rfc3339()}).to_string(),
        )
        .unwrap();
        assert!(m.peek_checked("dead").unwrap().is_none());
        assert!(
            m.dir().join("dead.running").exists(),
            "a reader does not sweep"
        );
        std::fs::write(m.dir().join("torn.running"), "{\"pid\": ").unwrap();
        assert!(m.peek_checked("torn").is_err());
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
