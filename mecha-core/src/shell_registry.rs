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
//! So the posture now comes from here: a file the harness writes **before**
//! the command can run a single instruction, keyed by the pid of the direct
//! child it spawned (`bash`, `bwrap` or `docker`), removed when that child is
//! gone. `mecha tasks set` walks its own ancestry and takes the posture of
//! the nearest registered shell ([`nearest_registered_ancestor`]). The
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
//! A start time that cannot be read on either side is taken as a match: that
//! direction refuses a closure, never permits one.
//!
//! **What it does not see, named.** A command that detaches from its shell
//! (so it is reparented away from the registered pid) escapes the ancestry
//! walk; an unconfined shell can edit `~/.mecha` directly; and `MECHA_HOME`
//! itself is the one environment input left. Redirecting it moves the config,
//! the closure store and this registry together — but not the board: the
//! graph server resolves its database from its own environment
//! (`XDG_DATA_HOME`, `MECHA_GRAPH_DB`, or its default), not from
//! `MECHA_HOME`. So a command that writes a config naming the real graph
//! server under a fresh `MECHA_HOME` lands the move on the real board, reads
//! as rule 4, and records it in the fake store — which needs the owner's
//! config contents or the server's path, both within an unconfined shell's
//! reach and neither within a confined one's. The answer to all three is the
//! sandbox — under bwrap and docker the command runs in a pid
//! namespace without `~/.mecha` mounted, and landlock does not grant it the
//! owner's home at all (`mecha doctor` reports a `shell` that runs unconfined).
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

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        crate::create_private_dir(&root).with_context(|| format!("creating {}", root.display()))?;
        Ok(ShellRegistry { root })
    }

    /// Open at the default location only if it exists — a reader must not
    /// create the registry it is about to consult.
    pub fn open_existing_default() -> Option<Self> {
        let root = Self::default_root().ok()?;
        root.is_dir().then_some(ShellRegistry { root })
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
    pub fn lookup(&self, pid: u32) -> Option<Entry> {
        let text = std::fs::read_to_string(self.path_of(pid)).ok()?;
        let entry: Entry = serde_json::from_str(&text).ok()?;
        if entry.pid != pid || !crate::process_alive(pid) {
            return None;
        }
        match (entry.proc_start, proc_start(pid)) {
            (Some(recorded), Some(now)) if recorded != now => None,
            _ => Some(entry),
        }
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

/// The nearest ancestor of this process that is a registered shell, and its
/// entry. Bounded, like `closure::run_ancestor`.
pub fn nearest_registered_ancestor(registry: &ShellRegistry) -> Option<Entry> {
    let mut pid = std::process::id();
    for _ in 0..64 {
        pid = crate::closure::parent_of(pid)?;
        if let Some(entry) = registry.lookup(pid) {
            return Some(entry);
        }
    }
    None
}

#[cfg(test)]
mod tests {
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
        assert!(nearest_registered_ancestor(&reg).is_none());
        let _held = reg
            .register(grandparent, Some(RunPosture::Interactive), None)
            .unwrap();
        let found = nearest_registered_ancestor(&reg).expect("grandparent is registered");
        assert_eq!(found.pid, grandparent);
        assert_eq!(found.posture(), Ok(RunPosture::Interactive));
        let _ = std::fs::remove_dir_all(reg.root());
    }
}
