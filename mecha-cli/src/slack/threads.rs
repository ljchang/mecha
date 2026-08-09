//! What state a Slack thread is in, and the store that remembers it.
//!
//! Written before any run exists, deliberately: a remote-control surface whose
//! states have no documented meaning cannot distinguish *waiting* from
//! *wedged*, which is the failure `docs/SLACK-RESEARCH.md` §9 found in a
//! shipped product's API — `waiting_for_user` and `idle` enumerated and neither
//! defined. So **every state here names both what it means and what resolves
//! it**, in code rather than in a comment, and a test walks the enum to prove
//! nobody added a state with no way out.
//!
//! The record is the truth and memory is not. A run lives in the connector's
//! process; if that process dies the run is gone, and a thread left displaying
//! "working…" forever is exactly the confusion above. [`ThreadStore::sweep`]
//! is what turns that into an honest state on startup.
//!
//! **One writer, and it is currently a convention rather than an enforcement.**
//! The connector's event loop is the only thing that writes here, so the store
//! takes no per-record lock — writes are temp-sibling-and-rename, so a reader
//! (`mecha slack status`) sees either the old record or the new one and never
//! half of either. What is *owed* is a connector-wide lock: two `mecha slack
//! connect` processes would both hold the socket, both answer, and both write.
//! Nothing stops that today except there being one operator, which is exactly
//! the shape this project distrusts elsewhere — see the trigger store's flock,
//! which exists for the same reason.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Where a thread is. Absence of a record is the ninth state — *unbound*, a
/// thread that exists in Slack and that mecha has never been asked about —
/// which is represented by `None` rather than by a variant, because a record
/// saying "I have no record" is a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadState {
    Idle,
    Running,
    AwaitingInput,
    Cancelled,
    Staged,
    Done,
    Failed,
    Orphaned,
}

impl ThreadState {
    /// Every variant, so tests can walk them and nothing can be added without
    /// answering the two questions below.
    pub const ALL: [ThreadState; 8] = [
        ThreadState::Idle,
        ThreadState::Running,
        ThreadState::AwaitingInput,
        ThreadState::Cancelled,
        ThreadState::Staged,
        ThreadState::Done,
        ThreadState::Failed,
        ThreadState::Orphaned,
    ];

    /// The name a human types and the name on disk — one function, so the CLI
    /// filter and the serialised form cannot disagree. Deriving it from
    /// `Debug` looks equivalent and is not: `AwaitingInput` lowercases to
    /// `awaitinginput`, which is not what `serde(rename_all)` writes and not
    /// what the help text promises.
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadState::Idle => "idle",
            ThreadState::Running => "running",
            ThreadState::AwaitingInput => "awaiting_input",
            ThreadState::Cancelled => "cancelled",
            ThreadState::Staged => "staged",
            ThreadState::Done => "done",
            ThreadState::Failed => "failed",
            ThreadState::Orphaned => "orphaned",
        }
    }

    /// What being in this state means.
    pub fn describe(self) -> &'static str {
        match self {
            ThreadState::Idle => "bound to a session and a workspace; nothing running",
            ThreadState::Running => "a run is in flight",
            ThreadState::AwaitingInput => "the run is blocked on an approval or a question",
            ThreadState::Cancelled => "stopped at a safe point; the partial turn was kept",
            ThreadState::Staged => "finished, and it left drafts in the outbox",
            ThreadState::Done => "finished, nothing pending",
            ThreadState::Failed => "the run errored",
            ThreadState::Orphaned => "the connector restarted while this run was in flight",
        }
    }

    /// What gets it out of this state. The point of the type: a state with no
    /// answer here is a thread a human cannot rescue.
    pub fn resolved_by(self) -> &'static str {
        match self {
            ThreadState::Idle => "an owner message starts a run",
            ThreadState::Running => "the run ends, or the owner presses Stop",
            ThreadState::AwaitingInput => {
                "the owner answers, or the timeout fires and the call is refused"
            }
            ThreadState::Cancelled => "an owner message starts a new run on the same conversation",
            ThreadState::Staged => "release or reject the drafts, from here or any outbox surface",
            ThreadState::Done => "an owner message starts a run",
            ThreadState::Failed => {
                "an owner message starts a run; the error was posted, not just logged"
            }
            ThreadState::Orphaned => {
                "the restart sweep announces it in the thread and resets to idle"
            }
        }
    }

    /// A run is in flight.
    pub fn is_active(self) -> bool {
        matches!(self, ThreadState::Running | ThreadState::AwaitingInput)
    }

    /// An owner message here begins work rather than steering it.
    pub fn accepts_new_prompt(self) -> bool {
        matches!(
            self,
            ThreadState::Idle
                | ThreadState::Cancelled
                | ThreadState::Staged
                | ThreadState::Done
                | ThreadState::Failed
        )
    }
}

/// Something that happened to a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// An allowlisted owner posted in the thread.
    OwnerSpoke,
    /// The run asked for an approval or called `ask_user`.
    AskedForInput,
    /// The question was answered — or timed out, which is also an answer.
    InputSettled,
    /// The run ended cleanly. `staged` is whether it left drafts behind.
    Finished { staged: bool },
    /// The run ended badly.
    Errored,
    /// The owner pressed Stop.
    StopPressed,
    /// The connector came up and found this thread mid-flight.
    ConnectorRestarted,
    /// An orphan was reported in the thread.
    OrphanAnnounced,
}

/// The transition table, pure and total.
///
/// `None` means the event does not apply in that state. It is never a panic and
/// never a silent no-op that pretends otherwise: the caller logs it, because an
/// event arriving in a state that cannot take it is information about a bug.
pub fn next(state: ThreadState, event: Event) -> Option<ThreadState> {
    use Event::*;
    use ThreadState::*;
    match (state, event) {
        // A message either begins work or steers work already going. Steering
        // leaves the state alone on purpose — the run did not restart.
        (s, OwnerSpoke) if s.accepts_new_prompt() => Some(Running),
        (Running, OwnerSpoke) => Some(Running),
        (AwaitingInput, OwnerSpoke) => Some(AwaitingInput),
        // An orphan has to be reported before it can be reused, or the
        // announcement never happens.
        (Orphaned, OwnerSpoke) => None,

        (Running, AskedForInput) => Some(AwaitingInput),
        (AwaitingInput, InputSettled) => Some(Running),

        (s, Finished { staged }) if s.is_active() => Some(if staged { Staged } else { Done }),
        (s, Errored) if s.is_active() => Some(Failed),
        (s, StopPressed) if s.is_active() => Some(Cancelled),

        (s, ConnectorRestarted) if s.is_active() => Some(Orphaned),
        // A restart is not news for a thread that was not running.
        (s, ConnectorRestarted) => Some(s),

        (Orphaned, OrphanAnnounced) => Some(Idle),

        _ => None,
    }
}

/// The run that is in flight, and the process holding it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMarker {
    pub pid: u32,
    pub started_at: DateTime<Utc>,
}

impl RunMarker {
    pub fn here() -> Self {
        Self {
            pid: std::process::id(),
            started_at: Utc::now(),
        }
    }

    /// Whether the process holding this run still exists.
    ///
    /// Uses the shared check rather than a local `kill`, because the range test
    /// inside it is the whole correctness: `kill(-1, 0)` succeeds and would
    /// report every dead run as alive.
    pub fn is_live(&self) -> bool {
        mecha_core::process_alive(self.pid)
    }
}

/// One thread, as remembered between events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadRecord {
    pub key: String,
    pub channel_id: String,
    pub thread_ts: String,
    pub state: ThreadState,
    /// The mecha session this thread's conversation is recorded in. Opaque
    /// here; the connector resolves it.
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    /// `ask` (the default), `allow`, or `read-only`. Set by a button, never
    /// inferred from prompt text.
    pub mode: String,
    /// The newest message this thread has already handled, for catch-up after
    /// a disconnect.
    #[serde(default)]
    pub last_seen_ts: Option<String>,
    #[serde(default)]
    pub run: Option<RunMarker>,
    /// The streamed answer, and the small message carrying Stop / Mode /
    /// Outbox.
    #[serde(default)]
    pub stream_ts: Option<String>,
    #[serde(default)]
    pub controls_ts: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl ThreadRecord {
    fn new(channel_id: &str, thread_ts: &str, mode: &str) -> Self {
        Self {
            key: key_for(channel_id, thread_ts),
            channel_id: channel_id.to_string(),
            thread_ts: thread_ts.to_string(),
            state: ThreadState::Idle,
            session_id: None,
            workspace: None,
            mode: mode.to_string(),
            last_seen_ts: None,
            run: None,
            stream_ts: None,
            controls_ts: None,
            updated_at: Utc::now(),
        }
    }
}

/// A thread's identity, and its filename.
///
/// Sanitised to alphanumerics rather than trusted: the components come from
/// Slack and are well-formed today, but a key that reaches the filesystem is a
/// path, and "it has always been well-formed" is not a containment argument.
pub fn key_for(channel_id: &str, thread_ts: &str) -> String {
    fn safe(s: &str) -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    }
    format!("{}-{}", safe(channel_id), safe(thread_ts))
}

pub struct ThreadStore {
    root: PathBuf,
}

impl ThreadStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        mecha_slack::store::create_private_dir(&root)
            .with_context(|| format!("creating {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.json"))
    }

    pub fn get(&self, key: &str) -> Result<Option<ThreadRecord>> {
        mecha_slack::store::read_json(&self.path(key))
            .with_context(|| format!("reading thread {key}"))
    }

    pub fn put(&self, record: &ThreadRecord) -> Result<()> {
        mecha_slack::store::write_private_json(&self.path(&record.key), record)
            .with_context(|| format!("writing thread {}", record.key))
    }

    pub fn all(&self) -> Result<Vec<ThreadRecord>> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e).context("listing threads"),
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            // A record that will not parse is reported, never skipped: silently
            // ignoring it would make a corrupt thread look like an absent one.
            match mecha_slack::store::read_json::<ThreadRecord>(&path) {
                Ok(Some(r)) => out.push(r),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("unreadable thread record {}: {e}", path.display());
                }
            }
        }
        // Newest first: the thread someone is looking at is the one that just
        // moved.
        out.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        Ok(out)
    }

    /// The record for a thread, creating an idle one if this is the first time.
    pub fn ensure(
        &self,
        channel_id: &str,
        thread_ts: &str,
        default_mode: &str,
    ) -> Result<ThreadRecord> {
        let key = key_for(channel_id, thread_ts);
        if let Some(existing) = self.get(&key)? {
            return Ok(existing);
        }
        let record = ThreadRecord::new(channel_id, thread_ts, default_mode);
        self.put(&record)?;
        Ok(record)
    }

    /// Apply an event, persisting the result.
    ///
    /// Returns `Ok(None)` when the event does not apply — the caller logs it
    /// rather than treating it as success, because an event arriving in a state
    /// that cannot take it is information about a bug.
    pub fn apply(&self, key: &str, event: Event) -> Result<Option<ThreadRecord>> {
        let Some(mut record) = self.get(key)? else {
            return Ok(None);
        };
        let Some(state) = next(record.state, event) else {
            return Ok(None);
        };
        record.state = state;
        record.updated_at = Utc::now();
        if !state.is_active() {
            // The run is over however it ended; the marker must not outlive it,
            // or a dead pid becomes the thing a later sweep reasons about.
            record.run = None;
        }
        self.put(&record)?;
        Ok(Some(record))
    }

    /// On startup: find threads that were mid-flight and whose run no longer
    /// exists, and mark them orphaned.
    ///
    /// Returns them so the caller can **say so in the thread**. Resetting
    /// quietly would leave a person watching a stream that will never finish,
    /// which is the whole failure this state exists to prevent.
    pub fn sweep(&self) -> Result<Vec<ThreadRecord>> {
        let mut orphaned = Vec::new();
        for record in self.all()? {
            if !record.state.is_active() {
                continue;
            }
            // A live pid means a run really is in flight — another connector, or
            // this one mid-restart. Leave it alone.
            if record.run.as_ref().is_some_and(RunMarker::is_live) {
                continue;
            }
            if let Some(updated) = self.apply(&record.key, Event::ConnectorRestarted)? {
                orphaned.push(updated);
            }
        }
        Ok(orphaned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mecha-slack-threads-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_typed_name_and_the_stored_name_are_the_same_name() {
        // Deriving one from `Debug` would give `awaitinginput` on disk-ish
        // while the CLI promised `awaiting_input`, and the filter would then
        // silently match nothing.
        for state in ThreadState::ALL {
            let stored = serde_json::to_string(&state).unwrap();
            assert_eq!(
                stored.trim_matches('"'),
                state.as_str(),
                "{state:?} disagrees with itself"
            );
        }
    }

    #[test]
    fn every_state_says_what_it_means_and_how_to_leave_it() {
        // The test that exists because of someone else's shipped API: a state
        // enumerated and undefined makes "waiting" and "wedged" the same thing
        // to a caller.
        for state in ThreadState::ALL {
            assert!(!state.describe().is_empty(), "{state:?} has no meaning");
            assert!(
                !state.resolved_by().is_empty(),
                "{state:?} has no way out — a thread in it cannot be rescued"
            );
        }
    }

    #[test]
    fn every_state_is_reachable_and_leaveable() {
        // Reachable: some (state, event) lands on it. Leaveable: some event
        // moves it somewhere else. A state failing either is a trap.
        let events = [
            Event::OwnerSpoke,
            Event::AskedForInput,
            Event::InputSettled,
            Event::Finished { staged: true },
            Event::Finished { staged: false },
            Event::Errored,
            Event::StopPressed,
            Event::ConnectorRestarted,
            Event::OrphanAnnounced,
        ];
        for target in ThreadState::ALL {
            let reachable = ThreadState::ALL.iter().any(|&from| {
                from != target && events.iter().any(|&e| next(from, e) == Some(target))
            });
            assert!(
                reachable || target == ThreadState::Idle,
                "{target:?} unreachable"
            );

            let leaveable = events
                .iter()
                .any(|&e| matches!(next(target, e), Some(s) if s != target));
            assert!(leaveable, "{target:?} cannot be left");
        }
    }

    #[test]
    fn a_message_starts_a_run_when_idle_and_steers_one_when_running() {
        assert_eq!(
            next(ThreadState::Idle, Event::OwnerSpoke),
            Some(ThreadState::Running)
        );
        assert_eq!(
            next(ThreadState::Running, Event::OwnerSpoke),
            Some(ThreadState::Running),
            "steering does not restart the run"
        );
        assert_eq!(
            next(ThreadState::AwaitingInput, Event::OwnerSpoke),
            Some(ThreadState::AwaitingInput),
            "a message while a question is pending does not answer it"
        );
    }

    #[test]
    fn an_orphan_must_be_announced_before_the_thread_is_reused() {
        // Otherwise the announcement never happens and the person is left
        // watching a stream that will never finish.
        assert_eq!(next(ThreadState::Orphaned, Event::OwnerSpoke), None);
        assert_eq!(
            next(ThreadState::Orphaned, Event::OrphanAnnounced),
            Some(ThreadState::Idle)
        );
    }

    #[test]
    fn finishing_with_drafts_is_a_different_state_from_finishing_without() {
        assert_eq!(
            next(ThreadState::Running, Event::Finished { staged: true }),
            Some(ThreadState::Staged)
        );
        assert_eq!(
            next(ThreadState::Running, Event::Finished { staged: false }),
            Some(ThreadState::Done)
        );
    }

    #[test]
    fn events_that_do_not_apply_are_refused_rather_than_ignored() {
        assert_eq!(next(ThreadState::Idle, Event::AskedForInput), None);
        assert_eq!(next(ThreadState::Done, Event::StopPressed), None);
        assert_eq!(next(ThreadState::Idle, Event::InputSettled), None);
    }

    #[test]
    fn a_restart_is_only_news_for_a_thread_that_was_running() {
        assert_eq!(
            next(ThreadState::Running, Event::ConnectorRestarted),
            Some(ThreadState::Orphaned)
        );
        assert_eq!(
            next(ThreadState::AwaitingInput, Event::ConnectorRestarted),
            Some(ThreadState::Orphaned)
        );
        assert_eq!(
            next(ThreadState::Done, Event::ConnectorRestarted),
            Some(ThreadState::Done)
        );
    }

    #[test]
    fn a_key_cannot_escape_the_directory() {
        let key = key_for("../../etc", "passwd/../..");
        assert!(!key.contains('/'), "{key}");
        assert!(!key.contains(".."), "{key}");
    }

    #[test]
    fn the_store_round_trips_and_ensure_is_idempotent() {
        let dir = scratch("roundtrip");
        let store = ThreadStore::open(&dir).unwrap();

        let first = store.ensure("D1", "1000.1", "ask").unwrap();
        assert_eq!(first.state, ThreadState::Idle);
        assert_eq!(first.mode, "ask");

        let again = store.ensure("D1", "1000.1", "allow").unwrap();
        assert_eq!(
            again.mode, "ask",
            "ensure must not rewrite a thread's mode from a default"
        );
        assert_eq!(store.all().unwrap().len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn applying_an_event_persists_it_and_clears_a_finished_runs_marker() {
        let dir = scratch("apply");
        let store = ThreadStore::open(&dir).unwrap();
        let mut record = store.ensure("D1", "1000.1", "ask").unwrap();
        record.run = Some(RunMarker::here());
        record.state = ThreadState::Running;
        store.put(&record).unwrap();

        let done = store
            .apply(&record.key, Event::Finished { staged: false })
            .unwrap()
            .expect("the event applies");
        assert_eq!(done.state, ThreadState::Done);
        assert!(
            done.run.is_none(),
            "a marker outliving its run is what a later sweep would reason about"
        );

        // Re-read, because the assertion is about what was written.
        let reread = store.get(&record.key).unwrap().unwrap();
        assert_eq!(reread.state, ThreadState::Done);

        assert!(
            store
                .apply(&record.key, Event::StopPressed)
                .unwrap()
                .is_none(),
            "an event that does not apply changes nothing"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_sweep_orphans_a_dead_run_and_leaves_a_live_one_alone() {
        let dir = scratch("sweep");
        let store = ThreadStore::open(&dir).unwrap();

        // A thread whose process is gone.
        let mut dead = store.ensure("D1", "1.0", "ask").unwrap();
        dead.state = ThreadState::Running;
        dead.run = Some(RunMarker {
            pid: i32::MAX as u32, // real-looking, far above any pid_max
            started_at: Utc::now(),
        });
        store.put(&dead).unwrap();

        // A thread whose process is this one, which is very much alive.
        let mut live = store.ensure("D2", "2.0", "ask").unwrap();
        live.state = ThreadState::Running;
        live.run = Some(RunMarker::here());
        store.put(&live).unwrap();

        // And one that was not running at all.
        let mut idle = store.ensure("D3", "3.0", "ask").unwrap();
        idle.state = ThreadState::Done;
        store.put(&idle).unwrap();

        let orphaned = store.sweep().unwrap();
        assert_eq!(orphaned.len(), 1, "only the dead one");
        assert_eq!(orphaned[0].key, dead.key);
        assert_eq!(orphaned[0].state, ThreadState::Orphaned);

        assert_eq!(
            store.get(&live.key).unwrap().unwrap().state,
            ThreadState::Running,
            "a live run must not be swept out from under itself"
        );
        assert_eq!(
            store.get(&idle.key).unwrap().unwrap().state,
            ThreadState::Done
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_thread_with_no_marker_at_all_is_orphaned_too() {
        // The crash that happened between starting a run and recording it.
        let dir = scratch("nomarker");
        let store = ThreadStore::open(&dir).unwrap();
        let mut record = store.ensure("D1", "1.0", "ask").unwrap();
        record.state = ThreadState::Running;
        record.run = None;
        store.put(&record).unwrap();

        let orphaned = store.sweep().unwrap();
        assert_eq!(orphaned.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
