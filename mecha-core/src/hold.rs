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
    /// chat", "trigger morning-brief", "mecha tasks"). Never matched on.
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
///
/// **An identity, not a path.** The path is a function of the router alone,
/// so after `cancel-switch` another switch's file can land at the same name;
/// a withdrawn switcher that tested only "does the file exist" read the new
/// one as its own, kept waiting, and loaded its model beside the new switch —
/// and either one's drop deleted the other's marker (review of #350). The
/// pid and start time say whose file it is.
pub struct Switching {
    path: PathBuf,
    pid: u32,
    started_at: DateTime<Utc>,
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(&self.cancel);
    }
}

/// Which switch a "switch now" was asked of — by identity, for `Switching`'s
/// reason: the marker's name is a function of the router alone, so one left
/// by a withdrawn switch must not hurry the next.
#[derive(Serialize, Deserialize)]
struct NowFor {
    pid: u32,
    started_at: DateTime<Utc>,
}

impl Switching {
    /// Is *this* switch still the pending one? `false` once `mecha model
    /// cancel-switch` withdrew it — including when another switch has since
    /// taken the same path, which is not this one. The switch then stops
    /// rather than load a model nobody is waiting for any more.
    pub fn still_pending(&self) -> bool {
        read::<Switch>(&self.path)
            .is_some_and(|s| s.pid == self.pid && s.started_at == self.started_at)
    }

    /// Has the owner asked *this* waiting switch to go now
    /// ([`Holds::request_now`])? The chip's "switch now" reaches a switch
    /// another process is already making — `mecha model use` started without
    /// `--now`, from the page or a terminal.
    pub fn now_requested(&self) -> bool {
        read::<NowFor>(&self.path.with_extension("now"))
            .is_some_and(|n| n.pid == self.pid && n.started_at == self.started_at)
    }
}

impl Drop for Switching {
    /// Removes the file only while it is still this switch's.
    fn drop(&mut self) {
        if self.now_requested() {
            let _ = std::fs::remove_file(self.path.with_extension("now"));
        }
        if self.still_pending() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Held {
    /// Has a "switch now" asked this run to stop? For tests: runs are reached
    /// through [`on_cancel`](Self::on_cancel).
    #[cfg(test)]
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
                // **Unreadable, and its holder alive: held — on every
                // router, since which one is unknown.** The same damage
                // `pending` resolves toward waiting; deleted, a newer
                // binary's hold format (or a short write on a full disk) let
                // the switch go ahead under a live run (review of #350).
                // The pid is in the name; `--now`'s grace is the way past.
                None if pid_of(&path).is_some_and(crate::process_alive) => out.push(Hold {
                    pid: pid_of(&path).unwrap_or_default(),
                    base_url: base.clone(),
                    what: "(a hold this build cannot read)".into(),
                    taken_at: DateTime::<Utc>::MIN_UTC,
                }),
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
            // Its time is the file's, so repeated reads are one switch to a
            // waiter that says so once, not a new one every poll.
            None if path.exists() => Some(Switch {
                pid: 0,
                base_url: crate::provider::router::base(base_url),
                from: None,
                to: "(unreadable switch file)".into(),
                started_at: std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .map(DateTime::<Utc>::from)
                    .unwrap_or_default(),
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
        self.begin_switch_try(base_url, from, to, true)
    }

    fn begin_switch_try(
        &self,
        base_url: &str,
        from: Option<&str>,
        to: &str,
        retry: bool,
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
            Ok(()) => Ok(Ok(Switching {
                path,
                pid: switch.pid,
                started_at: switch.started_at,
            })),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                match self.pending(&switch.base_url) {
                    Some(existing) => Ok(Err(existing)),
                    // Swept between the check and the link: one retry, and
                    // only one.
                    None if retry => self.begin_switch_try(&switch.base_url, from, to, false),
                    None => anyhow::bail!(
                        "a switch file on {} keeps appearing and going; try again",
                        switch.base_url
                    ),
                }
            }
            Err(e) => Err(e).with_context(|| format!("writing {}", path.display())),
        }
    }

    /// Withdraw the switch pending on `base_url`, readable or not, and say what
    /// it was. The way out of a switch file nothing will remove: a switcher
    /// whose pid was reused, or a file nobody can parse — both of which read
    /// as pending, so every run on the router would wait for ever.
    pub fn withdraw_switch(&self, base_url: &str) -> Option<Switch> {
        let path = self.switch_path(base_url);
        if !path.exists() {
            return None;
        }
        let was = read::<Switch>(&path).unwrap_or(Switch {
            pid: 0,
            base_url: crate::provider::router::base(base_url),
            from: None,
            to: "(unreadable switch file)".into(),
            started_at: Utc::now(),
        });
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("now"));
        Some(was)
    }

    /// Withdraw every switch file in the directory, whatever router it names —
    /// the way out of one on a router the config no longer follows, which
    /// [`withdraw_switch`](Self::withdraw_switch) keyed by router cannot reach.
    pub fn withdraw_all_switches(&self) -> Vec<Switch> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // A "switch now" marker whose switcher was killed before its drop
            // could remove it: harmless (it names a switch that is gone), and
            // swept here so it does not sit in the directory for ever.
            if name.starts_with("switch-") && name.ends_with(".now") {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            if !(name.starts_with("switch-") && name.ends_with(".json")) {
                continue;
            }
            out.push(
                read::<Switch>(&path).unwrap_or(Switch {
                    pid: 0,
                    base_url: name
                        .trim_start_matches("switch-")
                        .trim_end_matches(".json")
                        .into(),
                    from: None,
                    to: "(unreadable switch file)".into(),
                    started_at: Utc::now(),
                }),
            );
            let _ = std::fs::remove_file(&path);
        }
        out
    }

    /// Ask the switch waiting on `base_url` to load `to` to stop waiting — it
    /// then does what `--now` does (asks the runs to stop, gives them its
    /// grace) — and say which switch was asked. `None` when nothing is
    /// pending, when what is pending is a switch to another model (the one
    /// asked about was replaced — never hurried in its name), or when the
    /// pending file cannot be read: no switcher is there to hear it, and
    /// `cancel-switch` is that one's way out.
    pub fn request_now(&self, base_url: &str, to: &str) -> Result<Option<Switch>> {
        let Some(switch) = self.pending(base_url).filter(|s| s.pid != 0 && s.to == to) else {
            return Ok(None);
        };
        let path = self.switch_path(base_url).with_extension("now");
        let tmp = path.with_extension("now.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_string(&NowFor {
                pid: switch.pid,
                started_at: switch.started_at,
            })?,
        )?;
        std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))?;
        Ok(Some(switch))
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
            // An unreadable hold is asked too, by the pid in its name: it is
            // one `live` counts as holding every router.
            let (pid, ours) = match read::<Hold>(&path) {
                Some(h) => (Some(h.pid), h.base_url == base),
                None => (pid_of(&path), true),
            };
            if ours
                && pid.is_some_and(crate::process_alive)
                && std::fs::write(path.with_extension("cancel"), b"switch now\n").is_ok()
            {
                n += 1;
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

/// The pid a hold's file name carries (`{pid}-{uuid}.hold`) — what is left to
/// go on when the file itself cannot be read.
fn pid_of(path: &Path) -> Option<u32> {
    path.file_stem()?.to_str()?.split('-').next()?.parse().ok()
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

    /// The way out of a switch nothing will remove: withdrawn, readable or
    /// not, runs take holds again — and a live switcher can tell.
    #[test]
    fn a_stuck_switch_can_be_withdrawn() {
        let h = holds("withdraw");
        let switching = h.begin_switch(ROUTER, Some("a"), "b").unwrap().unwrap();
        assert!(switching.still_pending());
        assert_eq!(h.withdraw_switch(ROUTER).map(|s| s.to), Some("b".into()));
        assert!(
            !switching.still_pending(),
            "the switcher must see it was withdrawn"
        );
        assert!(h.try_hold(ROUTER, "next").unwrap().is_ok());

        // The recovery the command documents: withdraw, then switch again.
        // The withdrawn switcher must not read the new file as its own, nor
        // remove it when it drops.
        let next = h.begin_switch(ROUTER, Some("a"), "c").unwrap().unwrap();
        assert!(
            !switching.still_pending(),
            "a withdrawn switch took the next one's file"
        );
        drop(switching);
        assert!(
            next.still_pending(),
            "a withdrawn switch's drop removed the next one"
        );
        drop(next);

        std::fs::write(h.switch_path(ROUTER), b"not json").unwrap();
        assert!(h.pending(ROUTER).is_some(), "unreadable reads as pending");
        assert!(h.withdraw_switch(ROUTER).is_some());
        assert!(h.pending(ROUTER).is_none());
        assert_eq!(h.withdraw_switch(ROUTER).map(|s| s.to), None);
    }

    /// "Switch now" reaches the switch that is waiting, and only that one: a
    /// marker left by a withdrawn switch does not hurry the next, a switch to
    /// another model is never hurried in the asked one's name, and nothing
    /// pending is nothing to ask.
    #[test]
    fn switch_now_reaches_the_waiting_switch_and_no_later_one() {
        let h = holds("now");
        let marker = h.switch_path(ROUTER).with_extension("now");
        assert!(
            h.request_now(ROUTER, "b").unwrap().is_none(),
            "nothing pending"
        );
        let first = h.begin_switch(ROUTER, Some("a"), "b").unwrap().unwrap();
        assert!(!first.now_requested());
        assert!(
            h.request_now(ROUTER, "c").unwrap().is_none(),
            "a switch to b was hurried in c's name"
        );
        assert!(!first.now_requested());
        assert_eq!(
            h.request_now(ROUTER, "b").unwrap().map(|s| s.to),
            Some("b".into())
        );
        assert!(first.now_requested());

        // Withdrawn — which takes the marker — and then a marker put back as
        // a killed switcher would leave it, under a new switch.
        let stale = std::fs::read(&marker).unwrap();
        h.withdraw_switch(ROUTER).unwrap();
        assert!(!marker.exists(), "withdrawing left the marker");
        std::fs::write(&marker, &stale).unwrap();
        let next = h.begin_switch(ROUTER, Some("a"), "c").unwrap().unwrap();
        assert!(
            !next.now_requested(),
            "a withdrawn switch's marker hurried the next"
        );
        drop(next);

        // `cancel-switch`'s sweep takes a stale marker with nothing pending —
        // while its switcher (`first`, withdrawn) has not dropped, as one
        // killed never will.
        assert!(marker.exists());
        h.withdraw_all_switches();
        assert!(!marker.exists(), "cancel-switch left a stale marker");
        drop(first);

        // A finished switch takes its marker with it.
        let s = h.begin_switch(ROUTER, Some("a"), "d").unwrap().unwrap();
        h.request_now(ROUTER, "d").unwrap().unwrap();
        drop(s);
        assert!(!marker.exists());

        // An unreadable switch file has no switcher to hear it.
        std::fs::write(h.switch_path(ROUTER), b"not json").unwrap();
        assert!(h
            .request_now(ROUTER, "(unreadable switch file)")
            .unwrap()
            .is_none());
    }

    /// A hold this build cannot read is held while its pid lives — on every
    /// router, since which is unknown — and `--now` still reaches it; once
    /// its pid is gone it is swept like any other.
    #[test]
    fn an_unreadable_hold_is_held_while_its_process_lives() {
        let h = holds("unreadable");
        std::fs::create_dir_all(&h.dir).unwrap();
        let alive = h.dir.join(format!("{}-x.hold", std::process::id()));
        std::fs::write(&alive, b"{ a newer format }").unwrap();
        let live = h.live(ROUTER);
        assert_eq!(
            live.len(),
            1,
            "an unreadable hold of a live process let a switch go"
        );
        assert_eq!(
            h.live("http://127.0.0.1:9090").len(),
            1,
            "held on every router"
        );
        assert_eq!(h.cancel_holders(ROUTER), 1, "--now could not reach it");
        assert!(alive.with_extension("cancel").exists());

        let dead = h.dir.join(format!("{}-y.hold", u32::MAX - 1));
        std::fs::write(&dead, b"garbage").unwrap();
        h.live(ROUTER);
        assert!(!dead.exists(), "a dead holder's unreadable file is swept");
    }

    /// `cancel-switch` reaches every switch file there is, whichever router it
    /// names and whether or not it can be read.
    #[test]
    fn every_switch_file_can_be_withdrawn() {
        let h = holds("withdraw-all");
        let _a = h.begin_switch(ROUTER, Some("a"), "b").unwrap().unwrap();
        let _b = h
            .begin_switch("http://127.0.0.1:9090", None, "c")
            .unwrap()
            .unwrap();
        std::fs::write(h.dir.join("switch-elsewhere.json"), b"damaged").unwrap();
        let mut to: Vec<String> = h
            .withdraw_all_switches()
            .into_iter()
            .map(|s| s.to)
            .collect();
        to.sort();
        assert_eq!(to, ["(unreadable switch file)", "b", "c"]);
        assert!(h.pending(ROUTER).is_none());
        assert!(h.pending("http://127.0.0.1:9090").is_none());
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
