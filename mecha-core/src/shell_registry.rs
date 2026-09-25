//! Every command the `shell` tool spawns, registered by the harness with the
//! posture of the run that spawned it.
//!
//! `docs/APPRAISAL-WIRING-DESIGN.md` 1b-2, the owner's option A on the review
//! of #293. `closure::decide` used to take a run's posture from
//! [`crate::closure::POSTURE_ENV`], an environment variable the command
//! string itself can set: `MECHA_RUN_POSTURE=interactive mecha tasks set …`
//! in a delegated run's `bash -lc` overrode the stamp, and where the run held
//! no task or trigger marker nothing else stopped it — the move was allowed
//! and recorded `owner-approved`, a forged verdict in the store whose purpose
//! is to say who closed the task.
//!
//! So the posture now comes from here: a file the harness writes
//! **immediately after** spawning the command — keyed by the pid of the
//! direct child it spawned (`bash`, `bwrap` or `docker`), removed when that
//! child is gone. `mecha tasks set` checks its own pid and then walks its
//! ancestry, taking the posture of the nearest registered shell
//! ([`nearest_registered_ancestor`]); its own pid first, because `bash -lc
//! '<one simple command>'` execs that command in place, so the pid the tool
//! registered is often the reader itself (found on review of #294). The
//! directory is fixed — `MECHA_HOME/runs/shells`, with no override of its
//! own — so no variable the command text sets can point the reader at a
//! registry it wrote itself (found on review of #294: a `MECHA_SHELLS_DIR`
//! override the reader honoured did exactly that).
//!
//! **The shell child, not the hosting process.** `serve` hosts both the
//! owner's web board and a delegated web chat's model; registering `serve`
//! would refuse the owner's own board tap while that chat ran. Registering
//! the command the tool spawned covers exactly the model's commands.
//!
//! **A pid is not an identity on its own.** A registration left by a crash
//! outlives its process, and the pid can be reused by something unrelated —
//! even an ancestor of the owner's terminal. Each entry therefore carries the
//! process's start time (`/proc/<pid>/stat` field 22), and a reader accepts an
//! entry only when the live process with that pid started at the same tick.
//! Where `/proc` exists a start time that cannot be read on either side is
//! [`Lookup::Unreadable`], refused: taking it as a match would *permit* a
//! closure whenever the unverified entry says `interactive` (review of #294
//! found this paragraph claiming the opposite).
//!
//! **Unknown is never clean.** A registry directory that exists but cannot
//! be read, or an entry for a live ancestor that cannot be parsed, is
//! [`Lookup::Unreadable`], and `closure::decide` refuses it — only an absent
//! directory or an absent entry reads as "no registered shell".
//!
//! **What it does not see, named.** The entry is written immediately after
//! `spawn` returns, so there is a window of microseconds in which the command
//! is running and not yet registered; a command that clears the posture
//! variable and reads the registry inside that window reads as rule 4 (in
//! practice `mecha tasks set` spends far longer starting its graph client
//! before it consults the registry, but that is timing, not a guarantee). A
//! command that detaches from its shell (so it is reparented away from the
//! registered pid) escapes the ancestry walk; and an unconfined shell can
//! edit `~/.mecha` directly. The answer to both is the sandbox — under bwrap
//! and docker the command runs in a pid
//! namespace without `~/.mecha` mounted, and landlock does not grant it the
//! owner's home at all (`mecha tools` shows a `shell` that runs unconfined).
//!
//! **`MECHA_HOME` does not hide a registration.** Registration writes under
//! `MECHA_HOME` *and* the owner's real home ([`write_roots`]), and the
//! reader ([`guard_roots`]) reads both, the real one taken from the password
//! database for the current uid — never from `HOME` or `MECHA_HOME`, which
//! the command sets. Writing both is what holds for a harness that itself
//! runs under a `MECHA_HOME` (a trial arm): with the write only under the
//! harness's home, a command that redirected again read two empty
//! registries (review of #294). The same entry in both corroborates; an
//! entry found only in the real one, or a disagreeing one, is a redirect.
//! A registration found only there means the command redirected its own
//! home, and `closure::decide` refuses it whatever the posture; the run
//! markers are read the same way. Before this, `MECHA_HOME=/tmp/x mecha
//! tasks set …` read an empty registry and landed on rule 4 — and the graph
//! server resolves the board from its own environment, not `MECHA_HOME`, so
//! the move could land on the real board and be recorded in the fake store
//! (review of #293).
//!
//! **Off Linux** there is no `/proc`: the walk sees only this process's own
//! pid and no start time can be compared. A reader there that finds no
//! registration for itself while any live one exists refuses
//! ([`Lookup::Unreadable`]) rather than reading as the owner's terminal — the
//! price is that the owner's own terminal cannot close a task on macOS while
//! a run's shell is live.
//!
//! **A nested front end is not a person.** A registered shell that runs
//! `mecha chat`, `mecha run`, `mecha tui` or `mecha serve` starts a new front
//! end whose own `shell` tool registers its children with that front end's
//! posture — so each asks this registry first. `chat`, `run` and `tui` stamp
//! `interactive` only with a terminal on stdin *and* no registered shell
//! above them (`setup::front_end_interactive`), so a piped chat or a tui fed
//! a pty (`script -qc 'mecha tui'`) is unattended; `serve` reads the same
//! answer once at startup and latches it (`serve::chat::started_by_a_run`),
//! so a `serve` a run started — and authenticated to with a login it chose —
//! never stamps `interactive` (review of #294). A front end that detaches
//! first escapes the walk, as any command does; the sandbox is again the
//! answer (the confined command has no `~/.mecha` to run from).
use crate::closure::RunPosture;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One registered command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub pid: u32,
    /// The run posture as the harness knew it, or `unknown` for a run whose
    /// front-end never stamped one. A string, because a closed enum written
    /// to disk is a wire format: a word this build cannot read refuses.
    pub posture: String,
    /// The tool call that spawned it — a pointer, for the record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    pub started_at: DateTime<Utc>,
    /// Clock ticks since boot at which the process started, when readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proc_start: Option<u64>,
}

impl Entry {
    /// The posture this entry records, or the word it recorded when this
    /// build cannot read it.
    pub fn posture(&self) -> std::result::Result<RunPosture, String> {
        match self.posture.as_str() {
            "interactive" => Ok(RunPosture::Interactive),
            "unattended" => Ok(RunPosture::Unattended),
            "delegated" => Ok(RunPosture::Delegated),
            other => Err(other.to_string()),
        }
    }
}

/// The registry directory.
pub struct ShellRegistry {
    root: PathBuf,
}

/// A live registration; removing it when dropped. Held by the `shell` tool
/// for as long as the child it names can still be running.
pub struct Registration {
    path: PathBuf,
    pid: u32,
    proc_start: Option<u64>,
}

impl Drop for Registration {
    /// Removes the entry only while it still names *this* registration — the
    /// same pid and the same process start. A pid reused by a newer
    /// registered shell between this child being reaped and the drop must
    /// keep its entry, or its command would read as unregistered (review of
    /// #294). An entry that cannot be read is left: `lookup` skips it.
    fn drop(&mut self) {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return;
        };
        let Ok(entry) = serde_json::from_str::<Entry>(&text) else {
            return;
        };
        if entry.pid == self.pid && entry.proc_start == self.proc_start {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl ShellRegistry {
    /// `MECHA_HOME/runs/shells`, with **no environment override**: the
    /// `mecha tasks set` that reads it is a descendant of the model's shell
    /// and inherits whatever the command string exports, so a reader that
    /// honoured a variable would be a reader the command text can redirect —
    /// the forgery this module exists to stop (found on review of #294).
    /// Tests that need a separate registry set `MECHA_HOME`, as
    /// `mecha-cli/tests/closure_event.rs` does.
    pub fn default_root() -> Result<PathBuf> {
        // mecha-core's own unit tests run the `shell` tool in-process, and
        // each spawn registers here: they must never write to the owner's
        // real `~/.mecha`. A per-process temp root, chosen at compile time
        // rather than by an environment variable — tests run on parallel
        // threads, and a process-global env write would race them.
        #[cfg(test)]
        let root = std::env::temp_dir().join(format!("mecha-shells-test-{}", std::process::id()));
        #[cfg(not(test))]
        let root = crate::work::mecha_home()?.join("runs").join("shells");
        Ok(root)
    }

    /// Open (creating) the registry, sweeping stale entries as it does —
    /// the `shell` tool opens it once per spawn, which is what keeps the
    /// directory from accumulating after a hard kill.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        crate::create_private_dir(&root).with_context(|| format!("creating {}", root.display()))?;
        let registry = ShellRegistry { root };
        registry.sweep();
        Ok(registry)
    }

    /// Open at the default location only if it exists — a reader must not
    /// create the registry it is about to consult. `Ok(None)` only when the
    /// directory is absent; a path that exists but is not a readable
    /// directory, or a home that cannot be resolved, is an `Err`: unknown is
    /// never clean (review of #294).
    pub fn open_existing_default() -> std::result::Result<Option<Self>, String> {
        let root = Self::default_root().map_err(|e| format!("{e:#}"))?;
        Self::open_existing(root)
    }

    /// [`Self::open_existing_default`] at an explicit root.
    pub fn open_existing(root: PathBuf) -> std::result::Result<Option<Self>, String> {
        match std::fs::metadata(&root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", root.display())),
            Ok(m) if !m.is_dir() => Err(format!("{} is not a directory", root.display())),
            Ok(_) => match std::fs::read_dir(&root) {
                Ok(_) => Ok(Some(ShellRegistry { root })),
                Err(e) => Err(format!("{}: {e}", root.display())),
            },
        }
    }

    /// Remove entries whose process is gone or whose pid now belongs to a
    /// different process, and stray temporary files from an interrupted
    /// write — so a harness killed with SIGKILL does not leave files behind
    /// forever. Bounded and best-effort: it never fails the caller, and an
    /// entry for a live process it cannot parse is left for `lookup` to
    /// report as unreadable.
    pub fn sweep(&self) {
        const MAX_PER_SWEEP: usize = 256;
        let Ok(dir) = std::fs::read_dir(&self.root) else {
            return;
        };
        for entry in dir.flatten().take(MAX_PER_SWEEP) {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // `<pid>.json`, or `.<pid>.<uuid>.tmp` from a write that never
            // renamed into place.
            let pid: Option<u32> = if let Some(stem) = name.strip_suffix(".json") {
                stem.parse().ok()
            } else if name.starts_with('.') && name.ends_with(".tmp") {
                name[1..].split('.').next().and_then(|p| p.parse().ok())
            } else {
                None
            };
            let Some(pid) = pid else { continue };
            let stale = if !crate::process_alive(pid) {
                true
            } else if name.ends_with(".json") {
                // Alive: stale only if the entry names a different process
                // start than the one now holding the pid.
                std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|t| serde_json::from_str::<Entry>(&t).ok())
                    .is_some_and(
                        |e| matches!((e.proc_start, proc_start(pid)), (Some(a), Some(b)) if a != b),
                    )
            } else {
                false
            };
            if stale {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path_of(&self, pid: u32) -> PathBuf {
        self.root.join(format!("{pid}.json"))
    }

    /// Register a spawned child. Written to a temporary sibling and renamed,
    /// so a reader never sees half an entry.
    pub fn register(
        &self,
        pid: u32,
        posture: Option<RunPosture>,
        call_id: Option<&str>,
    ) -> Result<Registration> {
        let entry = Entry {
            pid,
            posture: posture
                .map(|p| p.as_str())
                .unwrap_or(RunPosture::UNKNOWN)
                .to_string(),
            call_id: call_id.map(str::to_string),
            started_at: Utc::now(),
            proc_start: proc_start(pid),
        };
        let path = self.path_of(pid);
        let tmp = self
            .root
            .join(format!(".{pid}.{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, serde_json::to_vec(&entry)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))?;
        Ok(Registration {
            path,
            pid,
            proc_start: entry.proc_start,
        })
    }

    /// The live entry for `pid`, if there is one: the file parses, the
    /// process is alive, and it is the same process that was registered.
    /// Every reading that is not a clean yes collapses to `None` here; the
    /// ancestry walk uses [`Self::lookup_checked`], which keeps "absent"
    /// and "could not be read" apart.
    pub fn lookup(&self, pid: u32) -> Option<Entry> {
        match self.lookup_checked(pid) {
            Lookup::Registered(entry) => Some(entry),
            Lookup::Absent | Lookup::Unreadable(_) => None,
        }
    }

    /// [`Self::lookup`], keeping absent and unreadable apart. A file that is
    /// absent, or names a process that is dead or whose pid has since been
    /// reused, is [`Lookup::Absent`] — determinably not this process's
    /// registration. A file that exists for a live pid and cannot be read or
    /// parsed, or whose recorded pid disagrees with its name, is
    /// [`Lookup::Unreadable`]: unknown is never clean (review of #294).
    pub fn lookup_checked(&self, pid: u32) -> Lookup {
        let path = self.path_of(pid);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Lookup::Absent,
            Err(e) if !crate::process_alive(pid) => {
                let _ = e;
                return Lookup::Absent;
            }
            Err(e) => return Lookup::Unreadable(format!("{}: {e}", path.display())),
        };
        if !crate::process_alive(pid) {
            return Lookup::Absent;
        }
        let entry: Entry = match serde_json::from_str(&text) {
            Ok(entry) => entry,
            Err(e) => return Lookup::Unreadable(format!("{}: {e}", path.display())),
        };
        if entry.pid != pid {
            return Lookup::Unreadable(format!(
                "{} names pid {}, not {pid}",
                path.display(),
                entry.pid
            ));
        }
        match (entry.proc_start, proc_start(pid)) {
            (Some(recorded), Some(now)) if recorded != now => Lookup::Absent,
            (Some(_), Some(_)) => Lookup::Registered(entry),
            // Where `/proc` exists, `register` always records a start time
            // (the child is at worst a zombie when it is read), so a missing
            // one on either side is an entry this reader cannot verify —
            // and a `Registered { interactive }` it could not verify would
            // *permit* a closure. Unknown is never clean (review of #294:
            // the earlier doc said this direction only ever refused).
            _ if ancestry_walkable() => Lookup::Unreadable(format!(
                "{}: the process start of pid {pid} could not be verified",
                path.display()
            )),
            // Off Linux there is no start time to compare at all; see
            // `nearest_registered_ancestor_among` for what that platform
            // can and cannot check.
            _ => Lookup::Registered(entry),
        }
    }

    /// Whether any entry names a live process — what the off-Linux walk uses
    /// to decide that it cannot tell (see `nearest_registered_ancestor_among`).
    fn holds_live_entries(&self) -> std::result::Result<bool, String> {
        let dir =
            std::fs::read_dir(&self.root).map_err(|e| format!("{}: {e}", self.root.display()))?;
        Ok(dir.flatten().any(|e| {
            e.path()
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".json"))
                .and_then(|stem| stem.parse::<u32>().ok())
                .is_some_and(crate::process_alive)
        }))
    }
}

/// Whether this platform lets a process walk its ancestry and read process
/// start times — `/proc`, i.e. Linux.
pub fn ancestry_walkable() -> bool {
    std::path::Path::new("/proc/self/stat").exists()
}

/// Where a command's shell registrations are read from: the registry under
/// [`crate::work::mecha_home`], and the one under the owner's real home
/// ([`crate::work::owner_mecha_home`]) when that differs. The second is what
/// makes `MECHA_HOME=/tmp/x mecha tasks set …` useless as a hiding place: a
/// command inside a real run still finds its registration there (review of
/// #293). Registration writes under the same roots ([`write_roots`]), so a
/// harness that itself runs under a `MECHA_HOME` — a trial arm — is found by
/// a command that redirects again (review of #294). Under `cfg(test)`, the
/// hermetic per-process root alone.
pub fn guard_roots() -> std::result::Result<Vec<PathBuf>, String> {
    #[cfg(test)]
    {
        ShellRegistry::default_root()
            .map(|r| vec![r])
            .map_err(|e| format!("{e:#}"))
    }
    #[cfg(not(test))]
    {
        crate::work::guard_homes()
            .map(|homes| {
                homes
                    .into_iter()
                    .map(|h| h.join("runs").join("shells"))
                    .collect()
            })
            .map_err(|e| format!("{e:#}"))
    }
}

/// Where the `shell` tool registers a command: every [`guard_roots`] root,
/// so the write is symmetric with the read — a registration only under the
/// harness's own home let a trial arm's command hide by setting `MECHA_HOME`
/// again (review of #294). When the guard roots cannot be named (an
/// override with no password-database entry to check it against), the
/// harness's own root alone: every reader in that environment refuses on the
/// same failure, so writing fewer places opens nothing — and `shell` keeps
/// working there.
pub fn write_roots() -> Result<Vec<PathBuf>> {
    match guard_roots() {
        Ok(roots) => Ok(roots),
        Err(_) => Ok(vec![ShellRegistry::default_root()?]),
    }
}

/// Clock ticks since boot at which `pid` started (`/proc/<pid>/stat` field
/// 22). `None` off Linux or when unreadable.
pub fn proc_start(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after the parenthesised command name; field 22 overall is the
    // 20th after it (fields 3.. start at index 0).
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// What the registry says about one pid.
#[derive(Debug, Clone, PartialEq)]
pub enum Lookup {
    /// No registration for this process (none written, or one left by a
    /// process that is gone or whose pid has been reused).
    Absent,
    /// A live registration.
    Registered(Entry),
    /// A registration exists for this live pid and could not be read.
    Unreadable(String),
}

/// [`nearest_registered_ancestor`] over several registries at once, nearest
/// pid first and every registry at each pid; returns which registry answered.
///
/// **Off Linux** there is no `/proc`, so the ancestry cannot be walked past
/// this process and a start time cannot be verified. There the walk checks
/// this process's own pid (the exec-in-place case) and otherwise, if any
/// registry holds a live entry, answers [`Lookup::Unreadable`] — a shell may
/// be registered above this process and nothing here can say whether it is,
/// so the closure is refused rather than read as the owner's terminal (review
/// of #294: a compound command that dropped the variable used to land on
/// rule 4). With nothing registered anywhere, it is [`Lookup::Absent`].
pub fn nearest_registered_ancestor_among(registries: &[ShellRegistry]) -> (Lookup, Option<usize>) {
    let me = std::process::id();
    if !ancestry_walkable() {
        for (i, registry) in registries.iter().enumerate() {
            match registry.lookup_checked(me) {
                Lookup::Absent => {}
                found => return (found, Some(i)),
            }
        }
        for registry in registries {
            match registry.holds_live_entries() {
                Ok(false) => {}
                Ok(true) => {
                    return (
                        Lookup::Unreadable(
                            "shells are registered, and this platform cannot walk a \
                             process's ancestry to say whether one of them is this \
                             command's"
                                .into(),
                        ),
                        None,
                    )
                }
                Err(why) => return (Lookup::Unreadable(why), None),
            }
        }
        return (Lookup::Absent, None);
    }
    walk_from(registries, me, 64)
}

/// The walk behind [`nearest_registered_ancestor_among`] on a platform that
/// can walk: from `start`, at most `depth` processes.
///
/// **The whole chain is read, not only the nearest registration** (review of
/// #294). A command can set `MECHA_HOME` to a directory it owns and write an
/// `interactive` entry there for its own pid — `proc_start` is readable from
/// `/proc/$$/stat` — which, read nearest-first, shadowed the real `delegated`
/// entry above it. So every pid up to the root is consulted in every
/// registry, and the walk answers with the first of: an unreadable entry; a
/// registration in any registry but this process's own (`Some(i)` with
/// `i > 0`, which the caller reads as a redirect); a registration whose
/// posture is not `interactive`. Only a chain whose every registration is
/// interactive and in this process's own registry answers with its nearest.
fn walk_from(registries: &[ShellRegistry], start: u32, depth: usize) -> (Lookup, Option<usize>) {
    // No registry exists at all: nothing can be registered above anyone.
    if registries.is_empty() {
        return (Lookup::Absent, None);
    }
    let mut pid = start;
    let mut nearest: Option<(Entry, usize)> = None;
    for _ in 0..depth {
        // Every registry at this pid. The harness registers a command under
        // every guard home (`write_roots`), so the same entry in a later
        // registry corroborates the one in this process's own; an entry
        // only in a later one, or one that disagrees, is a redirect.
        let mut own: Option<Entry> = None;
        for (i, registry) in registries.iter().enumerate() {
            match registry.lookup_checked(pid) {
                Lookup::Absent => {}
                Lookup::Unreadable(why) => return (Lookup::Unreadable(why), Some(i)),
                Lookup::Registered(e) if i == 0 => own = Some(e),
                Lookup::Registered(e) => {
                    let agrees = own.as_ref().is_some_and(|o| o.posture == e.posture);
                    if !agrees {
                        return (Lookup::Registered(e), Some(i));
                    }
                }
            }
        }
        if let Some(e) = own {
            if e.posture() != Ok(RunPosture::Interactive) {
                return (Lookup::Registered(e), Some(0));
            }
            nearest.get_or_insert((e, 0));
        }
        pid = match crate::closure::parent_of_checked(pid) {
            Ok(Some(parent)) => parent,
            Ok(None) => {
                return match nearest {
                    Some((e, i)) => (Lookup::Registered(e), Some(i)),
                    None => (Lookup::Absent, None),
                }
            }
            // An unreadable link is not the root: the rest of the chain is
            // unknown, like a walk that runs out of steps (review of #294).
            Err(why) => return (Lookup::Unreadable(why), None),
        };
    }
    // The bound ran out before the root did: the rest of the chain is
    // unknown, not absent, and absent would read as the owner's terminal
    // (review of #294).
    (
        Lookup::Unreadable(
            "this command's ancestry is deeper than the registry walk reaches, so a \
             registered shell above it cannot be ruled out"
                .into(),
        ),
        None,
    )
}

/// The nearest registered shell among this process **and** its ancestors,
/// this process first — `bash -lc '<one simple command>'` execs the command
/// in place, so the pid the `shell` tool registered is then the reader's own
/// (found on review of #294: starting at the parent refused an interactive
/// run's bare `mecha tasks set …`). Bounded, like `closure::run_ancestor`.
/// Reads the whole chain (see `walk_from`): an interactive registration
/// answers only when nothing above it is unreadable, non-interactive, or in
/// another registry.
pub fn nearest_registered_ancestor(registry: &ShellRegistry) -> Lookup {
    nearest_registered_ancestor_among(std::slice::from_ref(registry)).0
}

#[cfg(test)]
mod tests {

    /// A walk that runs out of steps before the chain ends has not found
    /// "no registered shell"; it has not looked (review of #294). Depth 0 is
    /// the bound reached at once, on any live process.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_walk_that_runs_out_before_the_root_is_unknown_not_absent() {
        let registry = ShellRegistry::open(ShellRegistry::default_root().unwrap()).unwrap();
        let (found, _) = walk_from(std::slice::from_ref(&registry), std::process::id(), 0);
        assert!(matches!(found, Lookup::Unreadable(_)), "{found:?}");
    }

    use super::*;

    fn registry() -> ShellRegistry {
        let dir = std::env::temp_dir().join(format!("mecha-shells-{}", uuid::Uuid::new_v4()));
        ShellRegistry::open(dir).unwrap()
    }

    #[test]
    fn a_registration_is_found_while_held_and_gone_when_dropped() {
        let reg = registry();
        let me = std::process::id();
        let held = reg
            .register(me, Some(RunPosture::Delegated), Some("call-1"))
            .unwrap();
        let entry = reg.lookup(me).expect("registered and alive");
        assert_eq!(entry.posture(), Ok(RunPosture::Delegated));
        assert_eq!(entry.call_id.as_deref(), Some("call-1"));
        drop(held);
        assert!(reg.lookup(me).is_none());
        let _ = std::fs::remove_dir_all(reg.root());
    }

    /// A registration dropped after its pid was reused by a newer
    /// registered shell must not delete the newer entry (review of #294).
    #[test]
    fn dropping_a_registration_leaves_a_newer_entry_for_its_pid_alone() {
        let reg = registry();
        let me = std::process::id();
        let old = reg.register(me, Some(RunPosture::Delegated), None).unwrap();
        // Simulate the pid's reuse: a newer registration, from a process
        // that started at a different tick, now owns the file.
        let newer = Entry {
            pid: me,
            posture: "interactive".into(),
            call_id: Some("newer".into()),
            started_at: Utc::now(),
            proc_start: old.proc_start.map(|t| t + 1).or(Some(1)),
        };
        std::fs::write(reg.path_of(me), serde_json::to_vec(&newer).unwrap()).unwrap();
        drop(old);
        let kept: Entry =
            serde_json::from_str(&std::fs::read_to_string(reg.path_of(me)).unwrap()).unwrap();
        assert_eq!(kept.call_id.as_deref(), Some("newer"));
        let _ = std::fs::remove_dir_all(reg.root());
    }

    #[test]
    fn an_unstamped_run_registers_as_unknown_which_reads_as_unreadable() {
        let reg = registry();
        let _held = reg.register(std::process::id(), None, None).unwrap();
        let entry = reg.lookup(std::process::id()).unwrap();
        assert_eq!(entry.posture(), Err("unknown".to_string()));
        let _ = std::fs::remove_dir_all(reg.root());
    }

    #[test]
    fn a_dead_pid_and_a_reused_pid_are_both_skipped() {
        let reg = registry();
        // A pid nothing is running as (pid_max is far below this).
        let dead = 4_000_000u32;
        std::fs::write(
            reg.path_of(dead),
            serde_json::to_vec(&Entry {
                pid: dead,
                posture: "delegated".into(),
                call_id: None,
                started_at: Utc::now(),
                proc_start: None,
            })
            .unwrap(),
        )
        .unwrap();
        assert!(reg.lookup(dead).is_none(), "dead pid");

        // This process, but recorded with a start time it does not have:
        // the pid was reused by something else.
        let me = std::process::id();
        if let Some(start) = proc_start(me) {
            std::fs::write(
                reg.path_of(me),
                serde_json::to_vec(&Entry {
                    pid: me,
                    posture: "delegated".into(),
                    call_id: None,
                    started_at: Utc::now(),
                    proc_start: Some(start + 1),
                })
                .unwrap(),
            )
            .unwrap();
            assert!(reg.lookup(me).is_none(), "reused pid");
        }
        let _ = std::fs::remove_dir_all(reg.root());
    }

    /// A live entry whose start time was never recorded cannot be told from
    /// a reused pid. Where `/proc` exists that is unreadable — refused — not
    /// a match: the old reader took it as one, and an `interactive` entry
    /// then *permitted* a closure (review of #294).
    #[test]
    fn an_unverifiable_start_time_is_unreadable_where_proc_exists() {
        let reg = registry();
        let me = std::process::id();
        std::fs::write(
            reg.path_of(me),
            serde_json::to_vec(&Entry {
                pid: me,
                posture: "interactive".into(),
                call_id: None,
                started_at: Utc::now(),
                proc_start: None,
            })
            .unwrap(),
        )
        .unwrap();
        let found = reg.lookup_checked(me);
        let _ = std::fs::remove_dir_all(reg.root());
        if ancestry_walkable() {
            assert!(matches!(found, Lookup::Unreadable(_)), "{found:?}");
        } else {
            assert!(matches!(found, Lookup::Registered(_)), "{found:?}");
        }
    }

    /// The walk crosses unregistered processes to find a registered shell
    /// two levels up — this process's grandparent (the test harness's
    /// parent), the shape of `bash → mecha tasks set` under a registered
    /// `bwrap` or an intermediate `sh`.
    #[test]
    fn the_nearest_registered_ancestor_is_found_two_levels_up() {
        let reg = registry();
        let Some(grandparent) =
            crate::closure::parent_of(std::process::id()).and_then(crate::closure::parent_of)
        else {
            return; // adopted by init or off Linux: nothing to walk
        };
        assert_eq!(nearest_registered_ancestor(&reg), Lookup::Absent);
        let _held = reg
            .register(grandparent, Some(RunPosture::Interactive), None)
            .unwrap();
        let Lookup::Registered(found) = nearest_registered_ancestor(&reg) else {
            panic!("grandparent is registered");
        };
        assert_eq!(found.pid, grandparent);
        assert_eq!(found.posture(), Ok(RunPosture::Interactive));
        let _ = std::fs::remove_dir_all(reg.root());
    }

    /// `bash -lc '<one simple command>'` execs in place, so the registered
    /// pid is the reader's own. On the previous head the walk began at the
    /// parent and missed it (review of #294).
    #[test]
    fn a_registration_for_this_very_process_is_found() {
        let reg = registry();
        let _held = reg
            .register(std::process::id(), Some(RunPosture::Interactive), None)
            .unwrap();
        let Lookup::Registered(found) = nearest_registered_ancestor(&reg) else {
            panic!("this process is registered");
        };
        assert_eq!(found.pid, std::process::id());
        let _ = std::fs::remove_dir_all(reg.root());
    }

    /// An entry for a live ancestor that will not parse is unreadable, not
    /// absent, and the walk stops there rather than skipping past it.
    #[test]
    fn an_unparseable_entry_for_a_live_process_is_unreadable_not_absent() {
        let reg = registry();
        std::fs::write(reg.path_of(std::process::id()), b"{ not json").unwrap();
        assert!(matches!(
            reg.lookup_checked(std::process::id()),
            Lookup::Unreadable(_)
        ));
        assert!(matches!(
            nearest_registered_ancestor(&reg),
            Lookup::Unreadable(_)
        ));
        let _ = std::fs::remove_dir_all(reg.root());
    }

    /// Absent is `Ok(None)`; a path that exists but is not a directory is an
    /// error, never "no registry".
    #[test]
    fn an_absent_registry_is_none_and_a_broken_one_is_an_error() {
        let base = std::env::temp_dir().join(format!("mecha-shells-open-{}", uuid::Uuid::new_v4()));
        assert_eq!(
            ShellRegistry::open_existing(base.clone()).map(|r| r.is_some()),
            Ok(false)
        );
        std::fs::write(&base, b"a file, not a directory").unwrap();
        assert!(ShellRegistry::open_existing(base.clone()).is_err());
        let _ = std::fs::remove_file(&base);
    }

    /// Opening sweeps entries for dead processes and stray temporary files,
    /// and leaves a live registration alone.
    #[test]
    fn opening_sweeps_dead_entries_and_stray_temporaries() {
        let reg = registry();
        let dead = 4_000_000u32;
        let dead_entry = reg.path_of(dead);
        std::fs::write(
            &dead_entry,
            serde_json::to_vec(&Entry {
                pid: dead,
                posture: "delegated".into(),
                call_id: None,
                started_at: Utc::now(),
                proc_start: None,
            })
            .unwrap(),
        )
        .unwrap();
        let tmp = reg.root().join(format!(".{dead}.abc.tmp"));
        std::fs::write(&tmp, b"half").unwrap();
        let _live = reg
            .register(std::process::id(), Some(RunPosture::Delegated), None)
            .unwrap();
        let reopened = ShellRegistry::open(reg.root().to_path_buf()).unwrap();
        assert!(!dead_entry.exists(), "dead entry swept");
        assert!(!tmp.exists(), "stray temporary swept");
        assert!(
            reopened.lookup(std::process::id()).is_some(),
            "live entry kept"
        );
        let _ = std::fs::remove_dir_all(reg.root());
    }
}
