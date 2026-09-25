//! Closing or reopening a board task, as a recorded lifecycle event.
//!
//! `docs/APPRAISAL-WIRING-DESIGN.md` S8, ruled by the owner on 2026-09-24:
//! "closing a task via a chat, tui, slack, or web should be recorded and have a
//! hook just like pre/post turn and session start/end." Before this, the
//! closure appraisal was printed on the stderr of whichever process ran
//! `mecha tasks set` and was gone: a closure could not be read back, joined to
//! a later reopen, or observed by the owner's own tooling.
//!
//! **The store is append-only, one JSON object per line**, in
//! `~/.mecha/closures/closures.jsonl`. There is no store-specific environment
//! override for its location (`MECHA_CLOSURES_DIR` was removed on review of
//! #293): the `mecha tasks set` that writes it can be a descendant of a
//! model's `shell`, and a location the command text could redirect is a
//! record the command text could hide while the real board moves. The root
//! is `work::mecha_home()`, which honours `MECHA_HOME`, and on #293 that was
//! residue: a command that set it wrote the record into another home and,
//! since the run markers came from the same home, emptied the ancestry check
//! too (found on review of #293). #294 closes it on the read side — the
//! shell registry and the markers are read under the owner's real home as
//! well, so a run's command that redirects its home is refused before
//! anything is written ([`ShellReading::Redirected`]). Tests reach a
//! scratch store through `MECHA_HOME` or [`ClosureStore::open`]. The kinds
//! of line:
//!
//! - a [`Transition`] — written **before** the board row moves. It names the
//!   task, the move (`from` → `to`), who made it and on which surface, the
//!   sessions that worked the task, and for a reopen the closure it undoes.
//! - an [`Entry::Aborted`] line naming a transition whose board write then
//!   failed. Readers drop the transition it names. Write-ahead, not
//!   write-behind: a closure whose record cannot be written does not happen
//!   (the silently-degrading guard), and a record whose closure did not happen
//!   is withdrawn rather than left standing.
//! - an [`Entry::Uncertain`] line naming a transition whose board write
//!   ended with an *unknown* outcome — the transport failed, or the answer
//!   did not parse — so the board may or may not have moved. The transition
//!   stands (unknown is never withdrawn as if it were a known failure), and
//!   the next status change that reads the row settles it: an
//!   [`Entry::Confirmed`] line if the board shows the move, an `aborted`
//!   line if it does not ([`ClosureStore::unresolved_uncertain`]).
//! - an [`Entry::Readout`] line carrying what the closure appraisal said, so a
//!   surface that is not a terminal — the web board — can show it.
//!
//! **Every closed enum here is a wire format.** An unknown surface, actor or
//! line kind from a newer build loads as `unknown` rather than failing the
//! row; a torn line is skipped with a warning rather than hiding the rest.
//!
//! **Who may close.** A closure is the owner's verdict on a delegated run, so
//! a run with nobody present must not be able to make one — not through the
//! graph tool (`closure_guard::ClosedStatusGuard` already refuses a model's
//! status write) and not through `shell: mecha tasks set` either, which is the
//! residue `closure_guard.rs` names. Three signals decide it, see [`decide`]:
//! the nearest ancestor of this process that the `shell` tool registered,
//! with the posture of the run that spawned it
//! ([`crate::shell_registry`] — written by the harness, out of reach of the
//! command text); whether any ancestor is a live delegated or scheduled run
//! (its run marker's pid); and [`POSTURE_ENV`], now advisory: a process that
//! claims a run no registration confirms is refused. All fail closed.
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The environment variable the harness stamps on every command its `shell`
/// tool runs, naming the posture of the run that ran it.
pub const POSTURE_ENV: &str = "MECHA_RUN_POSTURE";

/// The board statuses that close a task.
pub const CLOSED_STATUSES: [&str; 2] = ["done", "dropped"];

/// Whether a board status is a closed one.
/// Every status the board accepts — mecha-graph's `gtd::TASK_STATUSES`,
/// mirrored because this crate cannot depend on the graph's. Used where a
/// status is echoed into a suggested command line (`closure_guard`), so a
/// model-supplied value that is not one of these never is.
pub const TASK_STATUSES: [&str; 6] = ["next", "inbox", "scheduled", "waiting", "done", "dropped"];

/// Whether `status` is one of [`TASK_STATUSES`].
pub fn is_known_status(status: &str) -> bool {
    TASK_STATUSES.contains(&status)
}

pub fn is_closed_status(status: &str) -> bool {
    CLOSED_STATUSES.contains(&status)
}

/// Which way a status change moves a task across the open/closed line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Move {
    /// Open → `done` or `dropped`.
    Close,
    /// `done` or `dropped` → an open status.
    Reopen,
    /// A newer build's move this one cannot read.
    #[serde(other)]
    Unknown,
}

/// The move a status change makes, if it crosses the line at all. `from` is
/// the status the row had going in; an unknown prior status reads as open,
/// which is what `tasks set` has always assumed (`is_fresh_closure`).
pub fn classify(from: Option<&str>, to: &str) -> Option<Move> {
    let was_closed = from.is_some_and(is_closed_status);
    match (was_closed, is_closed_status(to)) {
        (false, true) => Some(Move::Close),
        (true, false) => Some(Move::Reopen),
        _ => None,
    }
}

/// The surface a closure was made on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Surface {
    Cli,
    Tui,
    Web,
    Slack,
    /// A model ran `mecha tasks set` from a chat, behind the approver.
    Chat,
    GraphTui,
    #[serde(other)]
    Unknown,
}

impl Surface {
    /// The surfaces a caller may name with `--surface`. `chat` is not one:
    /// it is derived from the run posture, so a model cannot claim to be a
    /// person's button.
    pub fn from_flag(s: &str) -> Option<Self> {
        match s {
            "cli" => Some(Self::Cli),
            "tui" => Some(Self::Tui),
            "web" => Some(Self::Web),
            "slack" => Some(Self::Slack),
            "graph-tui" => Some(Self::GraphTui),
            _ => None,
        }
    }
}

/// Who made the move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Actor {
    /// The owner, on a surface where their own hand makes the call.
    Owner,
    /// A model in a conversation the owner is present in ran it, behind the
    /// approver — the owner's approval, not the owner's keystroke.
    OwnerApproved,
    #[serde(other)]
    Unknown,
}

/// The posture of a run, as the harness stamps it on the commands the run's
/// `shell` tool executes. Only an interactive run — one with a person in the
/// conversation — may close a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunPosture {
    /// A person is in the conversation and the approver asks them.
    Interactive,
    /// Nobody is at the conversation: a trigger, a front-door or mail run,
    /// a web chat running with approvals off.
    Unattended,
    /// A delegated board task (`tasks work`, a question resume, a web chat
    /// about a task) — the run whose closure would be its own verdict.
    Delegated,
}

impl RunPosture {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Unattended => "unattended",
            Self::Delegated => "delegated",
        }
    }

    /// The value a shell command sees for a run whose posture nobody
    /// stamped. Reads as a refusal, never as permission.
    pub const UNKNOWN: &'static str = "unknown";
}

/// What this process's environment says about the run that started it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostureReading {
    /// Not started by a mecha `shell` call at all — a person's terminal, or
    /// a surface's own child process.
    NotInRun,
    InRun(RunPosture),
    /// Stamped, but not a value this build knows — including `unknown`.
    Unreadable(String),
}

/// Read [`POSTURE_ENV`] from a value (pure, for tests) …
pub fn read_posture(value: Option<&str>) -> PostureReading {
    match value {
        None => PostureReading::NotInRun,
        Some("interactive") => PostureReading::InRun(RunPosture::Interactive),
        Some("unattended") => PostureReading::InRun(RunPosture::Unattended),
        Some("delegated") => PostureReading::InRun(RunPosture::Delegated),
        Some(other) => PostureReading::Unreadable(other.to_string()),
    }
}

/// … and from this process's environment.
pub fn posture_from_env() -> PostureReading {
    read_posture(std::env::var(POSTURE_ENV).ok().as_deref())
}

/// What the `shell` registry says about this process: the nearest ancestor
/// the harness registered, and the posture it recorded for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellReading {
    /// No registered shell among this process and its ancestors, in a
    /// registry that exists and could be read (or does not exist yet).
    NotRegistered,
    /// The registry, or a registration on this process's chain, could not
    /// be read. Refused: unknown is never clean (review of #294).
    Unreadable(String),
    /// A registered shell; `posture` is `Err` with the recorded word when
    /// this build cannot read it (including `unknown`).
    Registered {
        pid: u32,
        posture: std::result::Result<RunPosture, String>,
    },
    /// A registered shell found only in the owner's real registry while this
    /// process reads a different `MECHA_HOME`: a command inside a run that
    /// pointed its own reads somewhere else. Refused whatever the posture —
    /// a run's command does not get to choose where its closure is recorded
    /// (review of #293).
    Redirected { pid: u32, root: PathBuf },
}

impl ShellReading {
    /// Read the registry at its default location. A registry that does not
    /// exist yet holds no registration; one that exists and cannot be read,
    /// or a registration on this process's chain that cannot be parsed, is
    /// [`ShellReading::Unreadable`].
    ///
    /// The registries read are [`crate::shell_registry::guard_roots`]: the
    /// one under `MECHA_HOME` and, when that differs, the owner's real one.
    pub fn from_registry() -> Self {
        match crate::shell_registry::guard_roots() {
            Err(why) => ShellReading::Unreadable(why),
            Ok(roots) => Self::from_roots(&roots),
        }
    }

    /// Read the registries at `roots`, the first being this process's own
    /// (`MECHA_HOME`'s). A registration found first in any later root is
    /// [`ShellReading::Redirected`].
    pub fn from_roots(roots: &[PathBuf]) -> Self {
        use crate::shell_registry::{nearest_registered_ancestor_among, Lookup, ShellRegistry};
        let mut open = Vec::new();
        let mut at = Vec::new();
        for (i, root) in roots.iter().enumerate() {
            match ShellRegistry::open_existing(root.clone()) {
                Err(why) => return ShellReading::Unreadable(why),
                Ok(None) => {}
                Ok(Some(registry)) => {
                    open.push(registry);
                    at.push(i);
                }
            }
        }
        match nearest_registered_ancestor_among(&open) {
            (Lookup::Absent, _) => ShellReading::NotRegistered,
            (Lookup::Unreadable(why), _) => ShellReading::Unreadable(why),
            (Lookup::Registered(e), Some(i)) if at[i] > 0 => ShellReading::Redirected {
                pid: e.pid,
                root: roots[at[i]].clone(),
            },
            (Lookup::Registered(e), _) => ShellReading::Registered {
                pid: e.pid,
                posture: e.posture(),
            },
        }
    }
}

const OWNERS_ACT: &str = "closing or reopening a task is the owner's act — close it from the \
     board, the TUI, Slack or a terminal";

/// Who made a move, on which surface — or why it may not be made.
///
/// **The posture comes from the harness, not the environment**
/// (`docs/APPRAISAL-WIRING-DESIGN.md` 1b-2, the owner's option A on the
/// review of #293). The `shell` tool registers every command it spawns
/// ([`crate::shell_registry`]), so `shell` — the nearest registered ancestor
/// — says what run this command belongs to in a way the command text cannot
/// forge. [`POSTURE_ENV`] (`env`) was that signal until the review found a
/// command could set it itself (`MECHA_RUN_POSTURE=interactive mecha tasks
/// set …`) and be recorded `owner-approved`; it is now advisory. In order:
///
/// 1. `ancestor_run`, a live delegated or scheduled run's marker pid above
///    this process ([`run_ancestor`]), refuses — kept from #293.
/// 2. A registered shell with the interactive posture is the owner's
///    approval: `owner-approved`, surface `chat`, whatever `--surface` says.
/// 3. A registered shell with any other posture, or one this build cannot
///    read, refuses.
/// 4. No registered shell and no [`POSTURE_ENV`]: not in a run at all — the
///    owner's own terminal or a surface's child — `owner`, on the flagged
///    surface.
///
///    Checked before 4 and 5: a registry, or a registration on this
///    process's chain, that cannot be read is refused whatever the variable
///    says — an I/O fault must not read as the owner's terminal (review of
///    #294: unknown is never clean).
/// 5. No registered shell but [`POSTURE_ENV`] set: a process claiming a run
///    no registration confirms. Refused: the legitimate way to carry the
///    variable is to be a registered shell's descendant, so its presence
///    without one is either a registration the harness could not write
///    (which the `shell` tool refuses to run without) or a forgery.
///
/// An MCP server and what it spawns *with its environment* meet rule 5: the
/// harness stamps every server `unknown` (`mcp::McpClient::build_command`)
/// and never registers one, because a server outlives any one run's posture
/// (review of #293). That is the one place the variable is still
/// load-bearing: a server is spawned by the agent process, not by a
/// registered shell, so a child it starts with a cleared environment reads
/// `(NotRegistered, NotInRun)` — rule 4, the owner — with no detaching
/// needed (review of #294). A server is third-party code the owner chose to
/// run; confining it (`sandbox = true`, with the mecha home unmounted) is
/// what takes the store out of its reach.
///
/// **What this does not close, named.** A command that detaches from its
/// shell, so it is reparented away from the registered pid, *and* clears
/// the variable reads as rule 4. The registry's location has no environment
/// override of its own (review of #294), and `MECHA_HOME` no longer hides a
/// registration either: the registry and the run markers are read under the
/// owner's real home too — from the password database, never `HOME` — and a
/// registration found only there is [`ShellReading::Redirected`], refused
/// (review of #293). Off Linux the ancestry cannot be walked, so a
/// registered shell anywhere refuses (`shell_registry`'s module doc). A
/// nested front-end is named there too. An unconfined
/// `shell` can do these, and can edit `~/.mecha` directly besides; the
/// answer to all of them is the sandbox —
/// bwrap and docker run the command in a pid namespace with no `~/.mecha`
/// mounted, and landlock does not grant the owner's home (`mecha doctor`
/// reports a `[sandbox]` that mounts the mecha home; `mecha tools` shows
/// an unconfined `shell`, the default).
pub fn decide(
    env: &PostureReading,
    shell: &ShellReading,
    ancestor_run: Option<u32>,
    flagged: Option<Surface>,
) -> std::result::Result<(Actor, Surface), String> {
    if let Some(pid) = ancestor_run {
        return Err(format!(
            "this command is running inside a delegated or scheduled run (process {pid}); \
             {OWNERS_ACT}"
        ));
    }
    match (shell, env) {
        (
            ShellReading::Registered {
                posture: Ok(RunPosture::Interactive),
                ..
            },
            _,
        ) => Ok((Actor::OwnerApproved, Surface::Chat)),
        (
            ShellReading::Registered {
                pid,
                posture: Ok(p),
            },
            _,
        ) => Err(format!(
            "this command was run by a {} run's shell (process {pid}), with nobody in the \
             conversation; {OWNERS_ACT}",
            p.as_str()
        )),
        (
            ShellReading::Registered {
                pid,
                posture: Err(word),
            },
            _,
        ) => Err(format!(
            "this command was run by a shell (process {pid}) whose run posture is {word:?}, \
             which is not one this build can read; refusing to close or reopen a task on its \
             behalf — {OWNERS_ACT}"
        )),
        (ShellReading::Redirected { pid, root }, _) => Err(format!(
            "this command was run by a shell (process {pid}) registered in {}, but it reads \
             a different MECHA_HOME — a command inside a run cannot redirect where its \
             closure is recorded; {OWNERS_ACT}",
            root.display()
        )),
        (ShellReading::Unreadable(why), _) => Err(format!(
            "the harness's shell registry could not be read ({why}), so this command's run \
             posture is unknown; refusing to close or reopen a task — {OWNERS_ACT}"
        )),
        (ShellReading::NotRegistered, PostureReading::NotInRun) => {
            Ok((Actor::Owner, flagged.unwrap_or(Surface::Cli)))
        }
        (ShellReading::NotRegistered, _) => Err(format!(
            "this command carries {POSTURE_ENV} but no registered mecha shell is among its \
             ancestors — a run's posture is read from the harness's registry, never from the \
             variable alone; {OWNERS_ACT}"
        )),
    }
}

/// The parent of `pid`, from `/proc/<pid>/stat`. `None` off Linux, for pid 1,
/// and when the file cannot be read or parsed.
pub fn parent_of(pid: u32) -> Option<u32> {
    parent_of_checked(pid).ok().flatten()
}

/// [`parent_of`], keeping "reached the root" (`Ok(None)`) apart from "could
/// not read" (`Err`) — a walk that must not read an unreadable link as the
/// end of the chain (review of #294).
pub fn parent_of_checked(pid: u32) -> std::result::Result<Option<u32>, String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|e| format!("process {pid}'s parent could not be read ({e})"))?;
    // The command name is in parentheses and may itself contain spaces or
    // parentheses, so the fields after it are read from the last `)`.
    let ppid: Option<u32> = stat
        .rfind(')')
        .and_then(|at| stat[at + 1..].split_whitespace().nth(1))
        .and_then(|p| p.parse().ok());
    match ppid {
        Some(ppid) => Ok((ppid > 1).then_some(ppid)),
        None => Err(format!("process {pid}'s parent could not be parsed")),
    }
}

/// The first ancestor of this process whose pid is in `run_pids`, if any.
/// Bounded, because `/proc` is a live tree and a cycle is not impossible to
/// observe mid-reparent.
pub fn run_ancestor(run_pids: &std::collections::HashSet<u32>) -> Option<u32> {
    let mut pid = std::process::id();
    for _ in 0..64 {
        pid = parent_of(pid)?;
        if run_pids.contains(&pid) {
            return Some(pid);
        }
    }
    None
}

/// One task's move across the open/closed line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub id: String,
    pub task: String,
    /// The status going in; `None` when the board did not say.
    #[serde(default)]
    pub from: Option<String>,
    pub to: String,
    #[serde(rename = "move")]
    pub kind: Move,
    pub actor: Actor,
    pub surface: Surface,
    /// The sessions that worked the task, as the board links them.
    #[serde(default)]
    pub sessions: Vec<String>,
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// For a reopen: the id of the closure it undoes, when one is recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undoes: Option<String>,
}

impl Transition {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task: &str,
        from: Option<&str>,
        to: &str,
        kind: Move,
        actor: Actor,
        surface: Surface,
        sessions: Vec<String>,
        reason: Option<String>,
    ) -> Self {
        Transition {
            id: format!("close-{}", uuid::Uuid::new_v4()),
            task: task.to_string(),
            from: from.map(str::to_string),
            to: to.to_string(),
            kind,
            actor,
            surface,
            sessions,
            at: Utc::now(),
            reason,
            undoes: None,
        }
    }
}

/// One line of the closure store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    Transition(Transition),
    /// The board write for transition `of` failed; the transition did not
    /// happen.
    Aborted {
        of: String,
        at: DateTime<Utc>,
        error: String,
    },
    /// The board write for transition `of` ended with an unknown outcome
    /// (transport failure, unparseable answer): it may have landed. The
    /// transition stands until a later read settles it.
    Uncertain {
        of: String,
        at: DateTime<Utc>,
        error: String,
    },
    /// A later read of the board showed the uncertain transition `of` had
    /// landed.
    Confirmed {
        of: String,
        at: DateTime<Utc>,
    },
    /// What the closure appraisal said about transition `of`.
    Readout {
        of: String,
        task: String,
        at: DateTime<Utc>,
        /// The task's appraisal in one line, when it had one — `None` for a
        /// task with no session or no recorded outcome.
        #[serde(default)]
        readout: Option<String>,
        /// Whether a follow-up task was staged.
        #[serde(default)]
        follow_up_staged: bool,
        /// The project's reading, when this closure closed a project.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<String>,
    },
    /// A line kind from a newer build.
    #[serde(other)]
    Unknown,
}

/// The append-only closure store.
pub struct ClosureStore {
    root: PathBuf,
}

/// Holds the store's writer lock for as long as it lives.
pub struct ClosureLock {
    _file: std::fs::File,
}

impl ClosureStore {
    /// `~/.mecha/closures` — no environment override (module doc).
    pub fn default_root() -> Result<PathBuf> {
        Ok(crate::work::mecha_home()?.join("closures"))
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        crate::create_private_dir(&root).with_context(|| format!("creating {}", root.display()))?;
        Ok(ClosureStore { root })
    }

    /// Open at the default location only if it already exists — a read path
    /// must not create the store it is about to report on.
    pub fn open_existing_default() -> Option<Self> {
        let root = Self::default_root().ok()?;
        root.is_dir().then_some(ClosureStore { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn ledger(&self) -> PathBuf {
        self.root.join("closures.jsonl")
    }

    fn lock(&self) -> Result<ClosureLock> {
        use std::os::unix::io::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(".lock"))?;
        // SAFETY: flock on an fd we own, held open by the returned guard.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error()).context("locking the closure store");
        }
        Ok(ClosureLock { _file: file })
    }

    /// Append one line, under the lock, synced — a caller that goes on to
    /// move the board has been told the record is on disk.
    pub fn append(&self, entry: &Entry) -> Result<()> {
        use std::io::Write;
        let _lock = self.lock()?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.ledger())
            .with_context(|| format!("opening {}", self.ledger().display()))?;
        let mut line = serde_json::to_string(entry)?;
        line.push('\n');
        file.write_all(line.as_bytes())
            .with_context(|| format!("writing {}", self.ledger().display()))?;
        file.sync_data()
            .with_context(|| format!("syncing {}", self.ledger().display()))?;
        Ok(())
    }

    /// Every line, oldest first. A torn line is skipped with a warning.
    pub fn entries(&self) -> Result<Vec<Entry>> {
        let path = self.ledger();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        Ok(text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| match serde_json::from_str::<Entry>(l) {
                Ok(e) => Some(e),
                Err(e) => {
                    tracing::warn!("skipping unreadable closure row: {e}");
                    None
                }
            })
            .collect())
    }

    /// The transitions that happened, oldest first — every transition line
    /// not withdrawn by a later `aborted` line.
    pub fn transitions(&self) -> Result<Vec<Transition>> {
        Ok(self.transitions_from(&self.entries()?))
    }

    /// [`Self::transitions`] over entries already read — one pass for a
    /// caller that also needs the other lines. An `uncertain` transition is
    /// included: it may have happened, and only a later `aborted` line
    /// withdraws it.
    pub fn transitions_from(&self, entries: &[Entry]) -> Vec<Transition> {
        let aborted: std::collections::HashSet<&str> = entries
            .iter()
            .filter_map(|e| match e {
                Entry::Aborted { of, .. } => Some(of.as_str()),
                _ => None,
            })
            .collect();
        entries
            .iter()
            .filter_map(|e| match e {
                Entry::Transition(t) if !aborted.contains(t.id.as_str()) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    /// The latest transition of `task` whose board write ended with an
    /// unknown outcome and that no later line has settled — what the next
    /// status change on the task must confirm or withdraw before it records
    /// anything of its own. Only the latest transition of the task is
    /// considered: an older uncertain one has been superseded by a later
    /// recorded move.
    pub fn unresolved_uncertain(&self, task: &str) -> Result<Option<Transition>> {
        let entries = self.entries()?;
        let Some(t) = self
            .transitions_from(&entries)
            .into_iter()
            .rev()
            .find(|t| t.task == task)
        else {
            return Ok(None);
        };
        let mut uncertain = false;
        for e in &entries {
            match e {
                Entry::Uncertain { of, .. } if *of == t.id => uncertain = true,
                Entry::Confirmed { of, .. } if *of == t.id => uncertain = false,
                _ => {}
            }
        }
        Ok(uncertain.then_some(t))
    }

    /// The latest closure of `task` that happened, if any — what a reopen
    /// undoes.
    pub fn latest_closure(&self, task: &str) -> Result<Option<Transition>> {
        Ok(self
            .transitions()?
            .into_iter()
            .rev()
            .find(|t| t.task == task && t.kind == Move::Close))
    }

    /// The latest transition of `task` and the readout written for it, if
    /// any — what a surface that did not see the appraiser's stderr shows.
    pub fn latest_with_readout(&self, task: &str) -> Result<Option<(Transition, Option<Entry>)>> {
        let entries = self.entries()?;
        let Some(t) = self
            .transitions_from(&entries)
            .into_iter()
            .rev()
            .find(|t| t.task == task)
        else {
            return Ok(None);
        };
        let readout = entries
            .into_iter()
            .rev()
            .find(|e| matches!(e, Entry::Readout { of, .. } if *of == t.id));
        Ok(Some((t, readout)))
    }
}

/// One line for a surface, built from whichever parts a readout carries —
/// the task's own appraisal, whether a follow-up was staged, the project's
/// reading — and `None` only when all three are empty. The project's line
/// stands on its own: closing the last open task of a project that was never
/// delegated has no task readout but does have a project reading (found on
/// review of #293, where it was written and shown nowhere).
pub fn readout_line(entry: Option<&Entry>) -> Option<String> {
    let Some(Entry::Readout {
        readout,
        follow_up_staged,
        project,
        ..
    }) = entry
    else {
        return None;
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(r) = readout {
        parts.push(r.clone());
    }
    if *follow_up_staged {
        parts.push("a follow-up was staged".to_string());
    }
    if let Some(p) = project {
        parts.push(p.clone());
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_readout_line_is_built_from_whichever_parts_exist() {
        let r = |readout: Option<&str>, staged: bool, project: Option<&str>| Entry::Readout {
            of: "c".into(),
            task: "t".into(),
            at: Utc::now(),
            readout: readout.map(str::to_string),
            follow_up_staged: staged,
            project: project.map(str::to_string),
        };
        assert_eq!(readout_line(None), None);
        assert_eq!(readout_line(Some(&r(None, false, None))), None);
        assert_eq!(
            readout_line(Some(&r(None, false, Some("project P closed: pride")))),
            Some("project P closed: pride".to_string())
        );
        assert_eq!(
            readout_line(Some(&r(
                Some("distress −0.5"),
                true,
                Some("project P closed")
            ))),
            Some("distress −0.5 · a follow-up was staged · project P closed".to_string())
        );
    }

    #[test]
    fn an_uncertain_transition_stands_until_a_later_read_settles_it() {
        let s = store();
        let a = close("t1");
        s.append(&Entry::Transition(a.clone())).unwrap();
        s.append(&Entry::Uncertain {
            of: a.id.clone(),
            at: Utc::now(),
            error: "connection reset".into(),
        })
        .unwrap();
        // Counted as recorded: it may have happened.
        assert_eq!(s.transitions().unwrap(), vec![a.clone()]);
        assert_eq!(s.latest_closure("t1").unwrap(), Some(a.clone()));
        assert_eq!(s.unresolved_uncertain("t1").unwrap(), Some(a.clone()));
        s.append(&Entry::Confirmed {
            of: a.id.clone(),
            at: Utc::now(),
        })
        .unwrap();
        assert_eq!(s.unresolved_uncertain("t1").unwrap(), None);
        assert_eq!(s.transitions().unwrap(), vec![a.clone()]);

        // The other settlement: a later read shows it did not land.
        let b = close("t2");
        s.append(&Entry::Transition(b.clone())).unwrap();
        s.append(&Entry::Uncertain {
            of: b.id.clone(),
            at: Utc::now(),
            error: "not JSON".into(),
        })
        .unwrap();
        s.append(&Entry::Aborted {
            of: b.id.clone(),
            at: Utc::now(),
            error: "a later read did not show it".into(),
        })
        .unwrap();
        assert_eq!(s.unresolved_uncertain("t2").unwrap(), None);
        assert_eq!(s.transitions().unwrap(), vec![a]);
    }

    fn store() -> ClosureStore {
        let dir = std::env::temp_dir().join(format!("closures-{}", uuid::Uuid::new_v4()));
        ClosureStore::open(dir).unwrap()
    }

    fn close(task: &str) -> Transition {
        Transition::new(
            task,
            Some("next"),
            "done",
            Move::Close,
            Actor::Owner,
            Surface::Cli,
            vec!["s-1".into()],
            None,
        )
    }

    #[test]
    fn a_move_is_a_crossing_of_the_open_closed_line_and_nothing_else() {
        assert_eq!(classify(Some("next"), "done"), Some(Move::Close));
        assert_eq!(classify(None, "dropped"), Some(Move::Close));
        assert_eq!(classify(Some("done"), "next"), Some(Move::Reopen));
        assert_eq!(classify(Some("dropped"), "inbox"), Some(Move::Reopen));
        assert_eq!(classify(Some("done"), "dropped"), None);
        assert_eq!(classify(Some("inbox"), "next"), None);
    }

    #[test]
    fn a_transition_round_trips_and_an_aborted_one_is_withdrawn() {
        let s = store();
        let a = close("t1");
        let b = close("t2");
        s.append(&Entry::Transition(a.clone())).unwrap();
        s.append(&Entry::Transition(b.clone())).unwrap();
        s.append(&Entry::Aborted {
            of: b.id.clone(),
            at: Utc::now(),
            error: "board down".into(),
        })
        .unwrap();
        let ts = s.transitions().unwrap();
        assert_eq!(ts, vec![a]);
    }

    #[test]
    fn a_reopen_finds_the_closure_it_undoes() {
        let s = store();
        let first = close("t1");
        s.append(&Entry::Transition(first.clone())).unwrap();
        let mut reopen = Transition::new(
            "t1",
            Some("done"),
            "next",
            Move::Reopen,
            Actor::Owner,
            Surface::Tui,
            vec![],
            None,
        );
        reopen.undoes = s.latest_closure("t1").unwrap().map(|t| t.id);
        assert_eq!(reopen.undoes.as_deref(), Some(first.id.as_str()));
        s.append(&Entry::Transition(reopen.clone())).unwrap();
        let (latest, _) = s.latest_with_readout("t1").unwrap().unwrap();
        assert_eq!(latest.kind, Move::Reopen);
        assert_eq!(latest.undoes, Some(first.id));
    }

    #[test]
    fn the_readout_is_read_back_for_the_latest_transition() {
        let s = store();
        let t = close("t1");
        s.append(&Entry::Transition(t.clone())).unwrap();
        s.append(&Entry::Readout {
            of: t.id.clone(),
            task: "t1".into(),
            at: Utc::now(),
            readout: Some("Distress · −0.5".into()),
            follow_up_staged: true,
            project: None,
        })
        .unwrap();
        let (_, readout) = s.latest_with_readout("t1").unwrap().unwrap();
        match readout {
            Some(Entry::Readout {
                readout,
                follow_up_staged,
                ..
            }) => {
                assert_eq!(readout.as_deref(), Some("Distress · −0.5"));
                assert!(follow_up_staged);
            }
            other => panic!("{other:?}"),
        }
    }

    /// A newer build's surface, actor, move or line kind loads as `unknown`
    /// rather than failing the row, and a torn line hides nothing else.
    #[test]
    fn unknown_variants_and_torn_lines_load_leniently() {
        let s = store();
        let good = close("t1");
        s.append(&Entry::Transition(good.clone())).unwrap();
        let mut raw = std::fs::read_to_string(s.ledger()).unwrap();
        raw.push_str(
            r#"{"kind":"transition","id":"close-x","task":"t2","to":"done","move":"teleport","actor":"robot","surface":"hologram","at":"2026-09-24T00:00:00Z"}
{"kind":"some_future_line","of":"close-x"}
{"kind":"transition","id":"close-torn"
"#,
        );
        std::fs::write(s.ledger(), raw).unwrap();
        let entries = s.entries().unwrap();
        assert_eq!(entries.len(), 3, "the torn line is skipped, the rest load");
        let ts = s.transitions().unwrap();
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[1].surface, Surface::Unknown);
        assert_eq!(ts[1].actor, Actor::Unknown);
        assert_eq!(ts[1].kind, Move::Unknown);
        assert!(matches!(entries[2], Entry::Unknown));
    }

    #[test]
    fn only_a_registered_interactive_shell_or_no_run_may_close() {
        use PostureReading as P;
        use ShellReading as S;
        let shell = |p: std::result::Result<RunPosture, String>| S::Registered {
            pid: 4242,
            posture: p,
        };
        // Rule 4: no registered shell, no variable — the owner's terminal or
        // a surface's own child, on the flagged surface.
        assert_eq!(
            decide(&P::NotInRun, &S::NotRegistered, None, Some(Surface::Web)),
            Ok((Actor::Owner, Surface::Web))
        );
        assert_eq!(
            decide(&P::NotInRun, &S::NotRegistered, None, None),
            Ok((Actor::Owner, Surface::Cli))
        );
        // Rule 2: a registered interactive shell is the owner's approval,
        // whatever the flag or the variable claims.
        assert_eq!(
            decide(
                &P::NotInRun,
                &shell(Ok(RunPosture::Interactive)),
                None,
                Some(Surface::Web)
            ),
            Ok((Actor::OwnerApproved, Surface::Chat))
        );
        // Rule 3: any other registered posture refuses — including when the
        // command set the variable to `interactive` itself (#293's review).
        for p in [RunPosture::Unattended, RunPosture::Delegated] {
            assert!(decide(
                &P::InRun(RunPosture::Interactive),
                &shell(Ok(p)),
                None,
                None
            )
            .is_err());
            assert!(decide(&P::NotInRun, &shell(Ok(p)), None, None).is_err());
        }
        assert!(decide(&P::NotInRun, &shell(Err("unknown".into())), None, None).is_err());
        // An unreadable registry refuses even with no variable —
        // it must not read as the owner's terminal (review of #294).
        assert!(decide(
            &P::NotInRun,
            &S::Unreadable("EACCES".into()),
            None,
            Some(Surface::Cli)
        )
        .is_err());
        // Rule 5: the variable with no registration behind it refuses, in
        // every value, `interactive` first.
        for env in [
            P::InRun(RunPosture::Interactive),
            P::InRun(RunPosture::Delegated),
            P::Unreadable("unknown".into()),
        ] {
            assert!(
                decide(&env, &S::NotRegistered, None, None).is_err(),
                "{env:?}"
            );
        }
        // Rule 1: a live run marker above this process wins over everything.
        assert!(decide(
            &P::NotInRun,
            &shell(Ok(RunPosture::Interactive)),
            Some(4243),
            None
        )
        .is_err());
        assert!(decide(&P::NotInRun, &S::NotRegistered, Some(4243), None).is_err());
    }

    /// `MECHA_HOME=/tmp/fresh mecha tasks set …` under a registered
    /// delegated shell (review of #293). The first root is the redirected
    /// home, empty; the second the owner's real one, holding the shell. Read
    /// from the first alone — all the old reader did — the command is the
    /// owner at their terminal; read from both, it is refused, and so is a
    /// redirected *interactive* shell: a run's command does not choose where
    /// its closure is recorded.
    #[test]
    fn a_command_that_redirects_mecha_home_is_still_refused() {
        use crate::shell_registry::ShellRegistry;
        let base = std::env::temp_dir().join(format!("mecha-redirect-{}", uuid::Uuid::new_v4()));
        let (fresh, owner) = (
            base.join("fresh/runs/shells"),
            base.join("owner/runs/shells"),
        );
        let registry = ShellRegistry::open(owner.clone()).unwrap();
        for posture in [RunPosture::Delegated, RunPosture::Interactive] {
            let _held = registry
                .register(std::process::id(), Some(posture), None)
                .unwrap();
            let old = ShellReading::from_roots(std::slice::from_ref(&fresh));
            assert_eq!(old, ShellReading::NotRegistered);
            assert_eq!(
                decide(&PostureReading::NotInRun, &old, None, None),
                Ok((Actor::Owner, Surface::Cli)),
                "the hole this closes"
            );
            let now = ShellReading::from_roots(&[fresh.clone(), owner.clone()]);
            assert_eq!(
                now,
                ShellReading::Redirected {
                    pid: std::process::id(),
                    root: owner.clone()
                }
            );
            assert!(decide(&PostureReading::NotInRun, &now, None, None).is_err());
            // Not redirected: the same registration in this process's own
            // root reads as before.
            let own = ShellReading::from_roots(&[owner.clone(), fresh.clone()]);
            assert!(matches!(own, ShellReading::Registered { .. }), "{own:?}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The forge direction (review of #294): a command that points
    /// `MECHA_HOME` at a directory it owns and writes an `interactive` entry
    /// there for its own pid must not shadow the real `delegated` one. Read
    /// nearest-first, the forged entry won and the close was owner-approved.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_registration_forged_under_a_redirected_home_does_not_shadow_the_real_one() {
        use crate::shell_registry::ShellRegistry;
        let base = std::env::temp_dir().join(format!("mecha-forge-{}", uuid::Uuid::new_v4()));
        let (fresh, owner) = (
            base.join("fresh/runs/shells"),
            base.join("owner/runs/shells"),
        );
        let real = ShellRegistry::open(owner.clone()).unwrap();
        let forged = ShellRegistry::open(fresh.clone()).unwrap();
        let _delegated = real
            .register(std::process::id(), Some(RunPosture::Delegated), None)
            .unwrap();
        let _claimed = forged
            .register(std::process::id(), Some(RunPosture::Interactive), None)
            .unwrap();
        let reading = ShellReading::from_roots(&[fresh.clone(), owner.clone()]);
        assert!(
            decide(&PostureReading::NotInRun, &reading, None, None).is_err(),
            "{reading:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A harness that itself runs under a `MECHA_HOME` — a trial arm —
    /// registers under that home *and* the owner's (`write_roots`), so its
    /// own commands read the entry corroborated, and a command that
    /// redirects `MECHA_HOME` again still finds it (review of #294: the
    /// write went only to the harness's home, so the redirect found two
    /// empty registries and landed on rule 4).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_trial_arms_registration_is_found_by_a_command_that_redirects_again() {
        use crate::shell_registry::ShellRegistry;
        let base = std::env::temp_dir().join(format!("mecha-trial-{}", uuid::Uuid::new_v4()));
        let (trial, owner, elsewhere) = (
            base.join("trial/runs/shells"),
            base.join("owner/runs/shells"),
            base.join("elsewhere/runs/shells"),
        );
        let _held: Vec<_> = [&trial, &owner]
            .into_iter()
            .map(|root| {
                ShellRegistry::open(root.clone())
                    .unwrap()
                    .register(std::process::id(), Some(RunPosture::Interactive), None)
                    .unwrap()
            })
            .collect();
        // The trial's own command: the same entry in both registries
        // corroborates rather than reading as a redirect.
        let own = ShellReading::from_roots(&[trial.clone(), owner.clone()]);
        assert!(
            matches!(
                own,
                ShellReading::Registered {
                    posture: Ok(RunPosture::Interactive),
                    ..
                }
            ),
            "{own:?}"
        );
        // A command that points `MECHA_HOME` somewhere else still finds it.
        let redirected = ShellReading::from_roots(&[elsewhere, owner.clone()]);
        assert!(
            matches!(redirected, ShellReading::Redirected { .. }),
            "{redirected:?}"
        );
        assert!(decide(&PostureReading::NotInRun, &redirected, None, None).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A parent link that cannot be read is not the root (review of #294).
    #[cfg(target_os = "linux")]
    #[test]
    fn an_unreadable_parent_is_an_error_not_the_end_of_the_chain() {
        assert!(parent_of_checked(u32::MAX).is_err());
        assert_eq!(parent_of_checked(1), Ok(None));
        assert!(parent_of_checked(std::process::id()).unwrap().is_some());
    }

    /// A non-interactive registration anywhere above refuses, however near
    /// an interactive one sits — in one registry too (review of #294).
    #[cfg(target_os = "linux")]
    #[test]
    fn an_interactive_entry_below_a_delegated_one_does_not_approve() {
        use crate::shell_registry::ShellRegistry;
        let base = std::env::temp_dir().join(format!("mecha-stacked-{}", uuid::Uuid::new_v4()));
        let root = base.join("runs/shells");
        let registry = ShellRegistry::open(root.clone()).unwrap();
        let parent = parent_of(std::process::id()).expect("a test has a parent");
        let _above = registry
            .register(parent, Some(RunPosture::Delegated), None)
            .unwrap();
        let _here = registry
            .register(std::process::id(), Some(RunPosture::Interactive), None)
            .unwrap();
        let reading = ShellReading::from_roots(std::slice::from_ref(&root));
        assert!(
            decide(&PostureReading::NotInRun, &reading, None, None).is_err(),
            "{reading:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_posture_variable_reads_only_its_three_words() {
        assert_eq!(read_posture(None), PostureReading::NotInRun);
        assert_eq!(
            read_posture(Some("delegated")),
            PostureReading::InRun(RunPosture::Delegated)
        );
        assert_eq!(
            read_posture(Some(RunPosture::UNKNOWN)),
            PostureReading::Unreadable("unknown".into())
        );
        assert_eq!(
            read_posture(Some("Interactive")),
            PostureReading::Unreadable("Interactive".into())
        );
    }

    #[test]
    fn this_process_has_an_ancestor_and_it_is_found_when_named() {
        let parent = parent_of(std::process::id());
        let Some(parent) = parent else {
            return; // off Linux, or adopted by init: nothing to assert
        };
        let pids: std::collections::HashSet<u32> = [parent].into();
        assert_eq!(run_ancestor(&pids), Some(parent));
        assert_eq!(run_ancestor(&std::collections::HashSet::new()), None);
    }
}
