//! Which runs are using a router's model, and whether a switch is waiting for
//! them — as files in a directory (REMOTE-SURFACE-DESIGN §14, D13).
//!
//! **The owner's rulings (2026-09-27):** a switch waits until no *run* holds
//! the model, so a run is answered by one model from start to finish; a run
//! that starts while a switch waits waits for the switch; and there is no time
//! limit — "switch now" is the way out. The router alone cannot give that: it
//! evicts only an idle model, which protects one request, and between a run's
//! requests the model is idle.
//!
//! **Files, because the contenders are separate processes** — `permit.rs`'s
//! reason and shape: a web chat inside `mecha serve`, a trigger fire, a
//! detached `tasks work`, and `mecha model use` share nothing but the
//! filesystem. A file whose process is gone is swept, never waited for:
//! [`crate::process_alive`]'s pid check is what makes a crash cost one stale
//! file rather than a switch that waits forever.
//!
//! **The handshake.** A run writes its hold, *then* looks for a pending
//! switch; a switch writes its file, *then* looks for holds. Each writes
//! before it reads, so at least one side sees the other — there is no moment
//! in which a run starts on the old model after the switch looked and found
//! nothing. And the run is the side that yields: seeing a switch, it drops its
//! hold and waits for the switch to clear.
//!
//! **Keyed by router, not by model.** A switch waits for every run on its
//! router, whatever model each resolved — a run holds before it resolves, so
//! at the moment it takes the hold it may not know yet.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A run holding a router's model: who, what, since when.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hold {
    pub pid: u32,
    /// The router, normalised with [`crate::provider::router::base`].
    pub base_url: String,
    /// What the run is, for a person reading what a switch waits on ("web
    /// chat", "trigger morning-brief", "mecha tasks work"). Never matched on.
    #[serde(default)]
    pub what: String,
    pub taken_at: DateTime<Utc>,
}

/// A switch waiting for, or in the middle of, replacing a router's model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Switch {
    pub pid: u32,
    pub base_url: String,
    #[serde(default)]
    pub from: Option<String>,
    pub to: String,
    pub started_at: DateTime<Utc>,
}

/// The directory.
pub struct Holds {
    dir: PathBuf,
}

/// A held run. Released on drop — `permit::Held`'s reason: every early return
/// is a place a release call would be forgotten, and a leaked hold is a switch
/// that waits on nobody. Its cancel file, if a "switch now" wrote one, goes
/// with it, so it cannot cancel whatever takes the next hold.
pub struct Held {
    path: PathBuf,
    cancel: PathBuf,
}

/// A pending switch, withdrawn on drop — so a switch that fails, is refused
/// or is cancelled with Ctrl-C never leaves every surface waiting.
pub struct Switching {
    path: PathBuf,
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(&self.cancel);
    }
}

impl Drop for Switching {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Held {
    /// Has a "switch now" asked this run to stop?
    pub fn cancel_requested(&self) -> bool {
        self.cancel.exists()
    }

    /// Call `stop` once if a "switch now" asks this run to stop. Polled — the
    /// cadence of `interrupt::run_interruptible_watching` — and ended when the
    /// hold is dropped, so a finished run's watcher does not outlive it.
    pub fn on_cancel(&self, stop: impl FnOnce() + Send + 'static) {
        let (path, cancel) = (self.path.clone(), self.cancel.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if cancel.exists() {
                    stop();
                    return;
                }
                if !path.exists() {
                    return;
                }
            }
        });
    }
}

impl Holds {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Holds { dir: dir.into() }
    }

    /// `~/.mecha/holds`.
    pub fn open_default() -> Result<Self> {
        Ok(Self::new(dir_under(&crate::work::mecha_home()?)))
    }

    /// The live holds on `base_url`, sweeping any whose process is gone or
    /// whose file cannot be read — a hold nothing can parse is holding the
    /// router for nobody, and a switch must not wait on it.
    pub fn live(&self, base_url: &str) -> Vec<Hold> {
        let base = crate::provider::router::base(base_url);
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("hold") {
                continue;
            }
            match read::<Hold>(&path) {
                Some(h) if crate::process_alive(h.pid) => {
                    if h.base_url == base {
                        out.push(h);
                    }
                }
                _ => {
                    let _ = std::fs::remove_file(&path);
                    let _ = std::fs::remove_file(path.with_extension("cancel"));
                }
            }
        }
        out.sort_by_key(|h| h.taken_at);
        out
    }

    /// The switch pending on `base_url`, if any. One whose switcher is gone
    /// is swept: a `mecha model use` killed mid-wait must not leave every
    /// surface waiting for a switch nobody is making.
    pub fn pending(&self, base_url: &str) -> Option<Switch> {
        let path = self.switch_path(base_url);
        match read::<Switch>(&path) {
            Some(s) if crate::process_alive(s.pid) => Some(s),
            Some(_) => {
                let _ = std::fs::remove_file(&path);
                None
            }
            // Unreadable while it exists: a switch is being written (the
            // link below makes a torn read impossible, so this is a file
            // someone damaged). Pending, rather than swept — erring toward
            // waiting is recoverable, erring toward running is not.
            None if path.exists() => Some(Switch {
                pid: 0,
                base_url: crate::provider::router::base(base_url),
                from: None,
                to: "(unreadable switch file)".into(),
                started_at: Utc::now(),
            }),
            None => None,
        }
    }

    /// A run's side of the handshake: write the hold, then look for a pending
    /// switch. `Err` carries the switch, with the hold already dropped.
    pub fn try_hold(
        &self,
        base_url: &str,
        what: &str,
    ) -> Result<std::result::Result<Held, Switch>> {
        let base = crate::provider::router::base(base_url);
        crate::create_private_dir(&self.dir)?;
        let hold = Hold {
            pid: std::process::id(),
            base_url: base.clone(),
            what: what.to_string(),
            taken_at: Utc::now(),
        };
        let path = self.dir.join(format!(
            "{}-{}.hold",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        write_whole(&path, &hold)?;
        let held = Held {
            cancel: path.with_extension("cancel"),
            path,
        };
        match self.pending(&base) {
            None => Ok(Ok(held)),
            Some(switch) => {
                drop(held);
                Ok(Err(switch))
            }
        }
    }

    /// A run's hold, waiting out any pending switch first. `on_wait` is told
    /// each switch the run waits for, once, so a surface can say "switching
    /// to X…". No time limit, by ruling: the switch is the owner's, and it
    /// ends — completed, failed or cancelled — or its switcher dies and is
    /// swept.
    pub async fn hold_when_clear(
        &self,
        base_url: &str,
        what: &str,
        mut on_wait: impl FnMut(&Switch),
    ) -> Result<Held> {
        let mut told: Option<(u32, DateTime<Utc>)> = None;
        loop {
            match self.try_hold(base_url, what)? {
                Ok(held) => return Ok(held),
                Err(switch) => {
                    if told != Some((switch.pid, switch.started_at)) {
                        on_wait(&switch);
                        told = Some((switch.pid, switch.started_at));
                    }
                    while self.pending(base_url).is_some() {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        }
    }

    /// A switch's side of the handshake: write the pending switch,
    /// exclusively. `Err` is the switch already pending — a second one is
    /// refused rather than queued, so the owner sees the first.
    pub fn begin_switch(
        &self,
        base_url: &str,
        from: Option<&str>,
        to: &str,
    ) -> Result<std::result::Result<Switching, Switch>> {
        let base = crate::provider::router::base(base_url);
        crate::create_private_dir(&self.dir)?;
        if let Some(existing) = self.pending(&base) {
            return Ok(Err(existing));
        }
        let path = self.switch_path(&base);
        let switch = Switch {
            pid: std::process::id(),
            base_url: base,
            from: from.map(str::to_string),
            to: to.to_string(),
            started_at: Utc::now(),
        };
        // Written whole to a temp sibling, then linked into place: the link
        // fails if the name exists, which makes "create if absent" atomic,
        // and a reader never sees a half-written file.
        let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
        std::fs::write(&tmp, serde_json::to_string_pretty(&switch)?)?;
        let linked = std::fs::hard_link(&tmp, &path);
        let _ = std::fs::remove_file(&tmp);
        match linked {
            Ok(()) => Ok(Ok(Switching { path })),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                match self.pending(&switch.base_url) {
                    Some(existing) => Ok(Err(existing)),
                    // Swept between the check and the link: one retry.
                    None => self.begin_switch(&switch.base_url, from, to),
                }
            }
            Err(e) => Err(e).with_context(|| format!("writing {}", path.display())),
        }
    }

    /// "Switch now": ask every run holding `base_url` to stop at its next safe
    /// point. Returns how many were asked.
    pub fn cancel_holders(&self, base_url: &str) -> usize {
        let base = crate::provider::router::base(base_url);
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let mut n = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("hold") {
                continue;
            }
            if let Some(h) = read::<Hold>(&path) {
                if h.base_url == base
                    && crate::process_alive(h.pid)
                    && std::fs::write(path.with_extension("cancel"), b"switch now\n").is_ok()
                {
                    n += 1;
                }
            }
        }
        n
    }

    fn switch_path(&self, base_url: &str) -> PathBuf {
        let slug: String = crate::provider::router::base(base_url)
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        self.dir.join(format!("switch-{slug}.json"))
    }
}

/// Where a mecha home keeps its holds.
pub fn dir_under(home: &Path) -> PathBuf {
    home.join("holds")
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

/// Temp sibling and rename, so no reader sees a half-written hold (`live`
/// sweeps what it cannot parse, and would delete a hold under its holder).
fn write_whole(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension("hold.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTER: &str = "http://127.0.0.1:8080";

    fn holds(name: &str) -> Holds {
        let dir = std::env::temp_dir().join(format!(
            "mecha-holds-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        Holds::new(dir)
    }

    /// The run side yields: with a switch pending, a hold is not taken, and
    /// nothing is left behind for the switch to wait on.
    #[test]
    fn a_run_does_not_hold_while_a_switch_is_pending() {
        let h = holds("yield");
        let _switching = h.begin_switch(ROUTER, Some("a"), "b").unwrap().unwrap();
        let Err(switch) = h.try_hold(ROUTER, "web chat").unwrap() else {
            panic!("a run took a hold under a pending switch")
        };
        assert_eq!(switch.to, "b");
        assert!(
            h.live(ROUTER).is_empty(),
            "the yielded hold was left behind"
        );
    }

    /// The switch side sees a run that holds: this is what it waits on.
    #[test]
    fn a_switch_sees_the_runs_that_hold() {
        let h = holds("sees");
        let held = h.try_hold(ROUTER, "trigger morning").unwrap().unwrap();
        let _switching = h.begin_switch(ROUTER, Some("a"), "b").unwrap().unwrap();
        let live = h.live("http://127.0.0.1:8080/v1");
        assert_eq!(live.len(), 1, "keyed by router, whatever its spelling");
        assert_eq!(live[0].what, "trigger morning");
        drop(held);
        assert!(h.live(ROUTER).is_empty(), "a dropped hold is gone");
    }

    /// One switch at a time: the second is refused and told the first.
    #[test]
    fn a_second_switch_is_refused_naming_the_first() {
        let h = holds("second");
        let first = h.begin_switch(ROUTER, Some("a"), "b").unwrap().unwrap();
        let Err(pending) = h.begin_switch(ROUTER, Some("a"), "c").unwrap() else {
            panic!("two switches pending at once")
        };
        assert_eq!(pending.to, "b");
        drop(first);
        assert!(h.pending(ROUTER).is_none(), "a dropped switch is withdrawn");
        assert!(h.begin_switch(ROUTER, Some("a"), "c").unwrap().is_ok());
    }

    /// A crashed holder or switcher is swept, never waited for.
    #[test]
    fn the_dead_are_swept_not_waited_for() {
        let h = holds("dead");
        std::fs::create_dir_all(&h.dir).unwrap();
        // A pid no process has: `process_alive` rejects it.
        let dead = u32::MAX - 1;
        let hold = Hold {
            pid: dead,
            base_url: ROUTER.into(),
            what: "crashed".into(),
            taken_at: Utc::now(),
        };
        std::fs::write(
            h.dir.join("1-x.hold"),
            serde_json::to_string(&hold).unwrap(),
        )
        .unwrap();
        let switch = Switch {
            pid: dead,
            base_url: ROUTER.into(),
            from: None,
            to: "b".into(),
            started_at: Utc::now(),
        };
        std::fs::write(
            h.switch_path(ROUTER),
            serde_json::to_string(&switch).unwrap(),
        )
        .unwrap();
        assert!(h.live(ROUTER).is_empty());
        assert!(h.pending(ROUTER).is_none());
        assert!(h.try_hold(ROUTER, "next").unwrap().is_ok());
    }

    /// "Switch now" reaches the holder, and only holders of that router.
    #[test]
    fn switch_now_asks_each_holder_on_that_router_to_stop() {
        let h = holds("now");
        let mine = h.try_hold(ROUTER, "web chat").unwrap().unwrap();
        let other = h
            .try_hold("http://127.0.0.1:9090", "elsewhere")
            .unwrap()
            .unwrap();
        assert_eq!(h.cancel_holders(ROUTER), 1);
        assert!(mine.cancel_requested());
        assert!(!other.cancel_requested());
        drop(mine);
        let next = h.try_hold(ROUTER, "next").unwrap().unwrap();
        assert!(
            !next.cancel_requested(),
            "a cancel must not outlive its hold"
        );
    }

    /// A run waiting for a switch starts once it clears, and is told once.
    #[tokio::test]
    async fn a_waiting_run_starts_when_the_switch_clears() {
        let h = std::sync::Arc::new(holds("clears"));
        let switching = h.begin_switch(ROUTER, Some("a"), "b").unwrap().unwrap();
        let told = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let waiter = {
            let (h, told) = (std::sync::Arc::clone(&h), std::sync::Arc::clone(&told));
            tokio::spawn(async move {
                h.hold_when_clear(ROUTER, "web chat", |_| {
                    told.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                })
                .await
                .unwrap()
            })
        };
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(
            !waiter.is_finished(),
            "a run started under a pending switch"
        );
        drop(switching);
        let held = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("the run never started after the switch cleared")
            .unwrap();
        assert_eq!(told.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(h.live(ROUTER).len(), 1);
        drop(held);
    }
}
