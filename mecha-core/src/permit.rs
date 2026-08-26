//! How many background runs may hold the model at once — as files in a
//! directory.
//!
//! **A latency control, and the measurement says so.** The scarce resource is
//! a scheduling seat on llama-server, not memory: `-c` is divided across
//! slots and committed at startup, so an idle conversation costs no extra
//! VRAM. Against `-np 4` on 2026-08-26, six concurrent conversations reached
//! **1.67×** the throughput of one while each turn took **3.6×** as long, and
//! four reached 1.58× at 2.5×. Throughput saturates at the seat count, so a
//! fifth concurrent conversation is close to pure loss — no more work done,
//! everybody waiting longer.
//!
//! **It is deliberately not about the prefix cache.** The obvious argument —
//! bound conversations so they stop evicting each other's prefix — was
//! measured and refuted in the same run: six conversations on four slots
//! re-prefilled 31 tokens per turn after the first, never a transcript.
//! `-cram` already handles that, and anything validating this module on
//! prefix reuse will find no effect, because there is none. Judge it on
//! per-turn latency.
//!
//! **Files, because the contenders are separate processes.** A delegation is
//! a chat session inside `mecha serve` or a detached `mecha tasks work`
//! child, and they share nothing but the filesystem — so an in-process
//! semaphore (`batch.rs`'s shape, and what the design originally called for)
//! would bound each process separately and none of them together. This is
//! [`crate::runmarker`]'s mechanism asked a different question: *may I
//! start*, rather than *am I running*. Its four rules carry over unchanged,
//! and the load-bearing one is the pid range check in
//! [`crate::process_alive`] — `kill(-1, 0)` succeeds, so a naive liveness
//! test would report every dead holder as alive and leak the pool shut.
//!
//! **Reserve, never preempt.** The owner must not queue behind delegations,
//! and the way to guarantee that is to leave a seat empty rather than to kill
//! something occupying one: a request in flight cannot be preempted anyway,
//! and cancelling a run to make room throws away a partial turn to save
//! latency the reserve already saved. So interactive work — a chat turn, a
//! voice call, a Slack thread — **never takes a permit at all**. It is not
//! admitted; it is simply not counted, which is the same thing done without a
//! mechanism that could fail closed against the person the system is for.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Background runs allowed at once, against `-np 4`.
///
/// One seat short of the server's, so the owner's turn never queues. Measured
/// rather than chosen: see the module doc and the measurement record.
pub const DEFAULT_BACKGROUND_PERMITS: usize = 3;

/// A held seat: who has it, since when, and what for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permit {
    pub pid: u32,
    pub taken_at: DateTime<Utc>,
    /// What the holder is doing, for a human reading `mecha doctor` or a
    /// refusal. Never matched on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub what: Option<String>,
}

/// The pool, as a directory.
pub struct Permits {
    dir: PathBuf,
    capacity: usize,
}

/// A held permit, released when dropped.
///
/// **Released on drop rather than by a call**, because every early return in
/// a run is a place a release would be forgotten — and a leaked permit is
/// invisible until the pool is full, at which point the symptom is that
/// nothing starts and nothing says why. The `Drop` still cannot run on a
/// SIGKILL, which is what the pid check is for: the next caller reclaims it.
pub struct Held {
    path: PathBuf,
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Permits {
    pub fn new(dir: impl Into<PathBuf>, capacity: usize) -> Self {
        Permits {
            dir: dir.into(),
            capacity,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Live holders, sweeping any whose process is gone.
    ///
    /// The sweep is what makes a crash cost one stale file rather than a
    /// permanently smaller pool — `runmarker`'s rule, and the reason this is
    /// a read that also writes.
    pub fn live(&self) -> Vec<Permit> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut held = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("permit") {
                continue;
            }
            let parsed = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<Permit>(&t).ok());
            match parsed {
                Some(p) if crate::process_alive(p.pid) => held.push(p),
                // Gone, or unreadable. Both are swept: a permit file nothing
                // can parse is holding a seat for nobody, which is worse than
                // losing the record of who had it.
                _ => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        held
    }

    /// Take a seat, or say who has them.
    ///
    /// **Never blocks.** A caller that waits is a caller holding a slot in
    /// some *other* queue — a web request, a Slack ack with three seconds to
    /// answer in — so this reports the refusal and lets the caller decide,
    /// which for a delegation means telling the owner it is queued rather
    /// than freezing the tap they just used.
    ///
    /// The race is real and deliberately unguarded: two callers can both see
    /// a free seat and both take it. The cost is one extra concurrent run
    /// against a soft latency target, and the alternative is a lock held
    /// across process boundaries for the length of an agent run — which is
    /// the thing `runmarker` refuses to do with the trigger flock, for the
    /// same reason. Over-admitting by one occasionally is cheaper than a
    /// stuck pool.
    pub fn take(&self, what: &str) -> Result<Result<Held, Vec<Permit>>> {
        let held = self.live();
        if held.len() >= self.capacity {
            return Ok(Err(held));
        }
        crate::create_private_dir(&self.dir)?;
        let permit = Permit {
            pid: std::process::id(),
            taken_at: Utc::now(),
            what: (!what.is_empty()).then(|| what.to_string()),
        };
        let path = self.dir.join(format!("{}.permit", std::process::id()));
        std::fs::write(&path, serde_json::to_string_pretty(&permit)?)?;
        Ok(Ok(Held { path }))
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mecha-permit-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_permit_is_held_until_dropped() {
        let p = Permits::new(scratch("basic"), 2);
        assert!(p.live().is_empty());
        let one = p.take("task-a").unwrap().expect("a free seat");
        assert_eq!(p.live().len(), 1);
        drop(one);
        assert!(p.live().is_empty(), "released on drop, not by a call");
    }

    /// The refusal names who is holding, because "queued" with no reason is
    /// the shape a person cannot act on.
    #[test]
    fn a_full_pool_refuses_and_says_who_has_it() {
        let p = Permits::new(scratch("full"), 1);
        let _one = p.take("task-a").unwrap().expect("a free seat");
        match p.take("task-b").unwrap() {
            Ok(_) => panic!("capacity ignored"),
            Err(held) => {
                assert_eq!(held.len(), 1);
                assert_eq!(held[0].what.as_deref(), Some("task-a"));
            }
        }
    }

    /// A holder killed outright cannot run its own `Drop`, so the pool would
    /// shrink by one for the life of the machine. This is the check that
    /// rests on the pid range guard: `kill(-1, 0)` succeeds, and a naive
    /// liveness test would call every dead holder alive.
    #[test]
    fn a_permit_whose_process_is_gone_is_reclaimed() {
        let p = Permits::new(scratch("dead"), 1);
        crate::create_private_dir(p.dir()).unwrap();
        std::fs::write(
            p.dir().join("0.permit"),
            serde_json::json!({"pid": 0, "taken_at": Utc::now().to_rfc3339()}).to_string(),
        )
        .unwrap();
        assert!(p.live().is_empty(), "a dead pid is not a holder");
        assert!(
            p.take("task-a").unwrap().is_ok(),
            "and its seat is available again"
        );
    }

    /// A file nothing can parse holds a seat for nobody, which is worse than
    /// losing the record of who had it.
    #[test]
    fn an_unreadable_permit_is_swept_rather_than_counted() {
        let p = Permits::new(scratch("junk"), 1);
        crate::create_private_dir(p.dir()).unwrap();
        std::fs::write(p.dir().join("x.permit"), "{not json").unwrap();
        assert!(p.live().is_empty());
        assert!(p.take("task-a").unwrap().is_ok());
    }

    /// Anything that is not a permit is not a permit — one stray `.DS_Store`
    /// must not read as a held seat, which is `ThreadStore`'s lesson one
    /// store over.
    #[test]
    fn a_stray_file_is_not_a_holder() {
        let p = Permits::new(scratch("stray"), 1);
        crate::create_private_dir(p.dir()).unwrap();
        std::fs::write(p.dir().join(".DS_Store"), "junk").unwrap();
        assert!(p.live().is_empty());
        assert!(p.take("task-a").unwrap().is_ok());
    }
}
