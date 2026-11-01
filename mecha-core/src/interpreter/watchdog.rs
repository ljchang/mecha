//! What ends a Seatbelt worker whose mecha died without dropping the pool
//! (`CODE-MODE-DESIGN.md` §7, R10).
//!
//! bwrap's `--die-with-parent` has no macOS counterpart, and the Seatbelt
//! launch chain is all `exec`s, so the pool's kill on drop is the only thing
//! that ends a worker, and a `SIGKILL`ed mecha never runs it. A worker
//! computing at that moment never reads its stdin's EOF, and the wall clock
//! died with the host, so it would spin until someone killed it. An idle
//! worker is not the risk: it sits reading its stdin and exits on the EOF.
//! So a program arms one of these when it checks a worker out, before the
//! worker runs a line.
//!
//! **It is mecha's child, in a process group of its own.** The first shape
//! of this guard was a loop the launcher double-forked: reparented to
//! launchd at every worker start while it stayed in mecha's process group.
//! On CI that group is also the runner's, and with the tests that `SIGSTOP`
//! a worker it lost the runner itself in 5 of 18 macOS runs, against 0 of 16
//! with those tests skipped (#772). Here nothing in mecha's group is ever
//! reparented while mecha lives, the loop's group holds only the loop and
//! its `sleep`, and nothing ever stops either. tokio reaps the loop when it
//! ends, because a dropped [`Watchdog`] is never killed: it has to outlive
//! mecha, which is its whole job.
//!
//! A loop abandoned before its handshake (refused, timed out, or dropped
//! when a Stop or the wall clock won the race in `Monty::start`) is killed
//! with its whole group despite `kill_on_drop(false)`, which only ever
//! applies to an armed one (review of #780).
//!
//! The loop is [`LOOP`], fixed text with no input from a program. It runs
//! outside the profile, as mecha does. `PATH` is empty under `env_clear`,
//! so every command it forks is named absolutely.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::AsyncBufReadExt;

/// What the loop reads start times with.
pub(crate) const PS: &str = "/bin/ps";

/// How long the loop has to read both start times and say so.
const HANDSHAKE: Duration = Duration::from_secs(10);

/// The loop, given the worker's pid as `$1` and mecha's as `$2`. Each pid
/// is recorded with its start time (`ps -o lstart`), because a pid alone
/// names a process only until the kernel reuses it, and `exec` keeps a
/// process's start time. It says `ok` once it has both, and then, once a
/// second:
///
/// - It ends when the worker does (`kill -0`, a builtin).
/// - Mecha is alive while `kill -0` reaches its pid **and** that pid still
///   has mecha's start time, so a reused pid cannot pass for a live mecha.
///   A reused pid owned by another user fails `kill -0` (`EPERM`), which
///   reads as gone, the right answer.
/// - Once mecha is gone, it kills the worker, but only if that pid still
///   has the worker's start time, so a reused pid is never killed. A worker
///   pid that reads another start time means the worker is gone, and the
///   loop ends.
///
/// **An unreadable `ps` is never an answer** (review of #772). An empty
/// read means a failed fork as often as a missing process, under the
/// process limit or memory pressure, which is exactly when a worker sits at
/// `max_memory`. Concluding from it would kill a healthy worker, or retire
/// the loop and leave the worker unguarded. So an empty read of mecha's
/// start time counts as alive, an empty read of the worker's counts as
/// unknown, and the loop asks again a second later. Before the handshake,
/// an unreadable start time ends the loop without `ok`, and the program is
/// refused. `{ps}` is [`PS`], or a test's stand-in.
const LOOP: &str = r#"w=$1 p=$2
t=$({ps} -o lstart= -p "$w" 2>/dev/null)
tp=$({ps} -o lstart= -p "$p" 2>/dev/null)
if [ -z "$t" ] || [ -z "$tp" ]; then echo "a start time is unreadable"; exit 1; fi
echo ok
exec >/dev/null
while kill -0 "$w" 2>/dev/null; do
if kill -0 "$p" 2>/dev/null; then
c=$({ps} -o lstart= -p "$p" 2>/dev/null)
if [ -z "$c" ] || [ "$c" = "$tp" ]; then /bin/sleep 1; continue; fi
fi
c=$({ps} -o lstart= -p "$w" 2>/dev/null)
if [ "$c" = "$t" ]; then kill -9 "$w"; exit 0; fi
if [ -n "$c" ]; then exit 0; fi
/bin/sleep 1
done
"#;

/// A loop watching one worker for mecha's death. Dropping it leaves the
/// loop running until the worker ends, and tokio reaps it then.
pub(crate) struct Watchdog {
    /// Held, never killed: tokio reaps the loop when it ends.
    #[cfg_attr(not(test), allow(dead_code))]
    child: tokio::process::Child,
}

impl Watchdog {
    /// Starts the loop over `worker` on behalf of `mecha` (this process, in
    /// production), and returns once it has read both start times. A loop
    /// that cannot is refused, so nothing runs unguarded.
    pub(crate) async fn arm(worker: u32, mecha: u32, ps: &str) -> Result<Self> {
        Self::arm_within(worker, mecha, ps, HANDSHAKE).await
    }

    /// [`Self::arm`], with the handshake bounded by `within`.
    async fn arm_within(worker: u32, mecha: u32, ps: &str, within: Duration) -> Result<Self> {
        let mut child = tokio::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(LOOP.replace("{ps}", &sh_quote(ps)))
            .arg("mecha-watchdog")
            .arg(worker.to_string())
            .arg(mecha.to_string())
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .kill_on_drop(false)
            .spawn()
            .context("starting the monty worker's watchdog")?;
        // Until the handshake succeeds, the loop's whole group goes with this
        // call however it ends: refused, timed out, or dropped because a Stop
        // or the wall clock won the race in `Monty::start`. A `ps` hung long
        // enough to cause any of those would otherwise outlive the loop that
        // forked it, and the loop, blocked on it, would never reach the write
        // that ends it (review of #780). Declared after `child`, so it drops
        // first on every path: the group is signalled while its leader is
        // still this process's unreaped child, and so cannot be anyone
        // else's group by then.
        let mut pending = Unarmed(child.id());
        let stdout = child
            .stdout
            .take()
            .context("the watchdog's stdout was not piped")?;
        let mut line = String::new();
        let said = tokio::time::timeout(
            within,
            tokio::io::BufReader::new(stdout).read_line(&mut line),
        )
        .await;
        if matches!(said, Ok(Ok(_))) && line.trim() == "ok" {
            pending.0 = None;
            return Ok(Watchdog { child });
        }
        drop(pending);
        let _ = child.wait().await;
        let why = match said {
            Err(_) => format!("no answer within {within:.1?}"),
            Ok(Err(e)) => e.to_string(),
            Ok(Ok(_)) if line.trim().is_empty() => "it ended without a word".into(),
            Ok(Ok(_)) => line.trim().to_string(),
        };
        bail!("the monty worker's watchdog did not start ({why}); refusing to run it unguarded")
    }

    /// The loop's pid, while it runs.
    #[cfg(test)]
    pub(crate) fn pid(&self) -> Option<u32> {
        self.child.id()
    }
}

/// A loop not yet armed: its group is killed when this drops, unless the
/// handshake took the pid out first.
struct Unarmed(Option<u32>);

impl Drop for Unarmed {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            // SAFETY: a plain signal to the group `arm_within` created, whose
            // leader is still this process's unreaped child.
            unsafe { libc::killpg(pid as libc::pid_t, libc::SIGKILL) };
        }
    }
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::super::test_process as process;
    use super::*;

    /// This test's own children: a worker and a stand-in mecha, killed if an
    /// assert fails first. A child is never reaped until it is waited on, so
    /// its pid cannot be reused under the signal.
    struct Kids(Vec<std::process::Child>);

    impl Drop for Kids {
        fn drop(&mut self) {
            for c in &mut self.0 {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    fn sleeper() -> std::process::Child {
        std::process::Command::new("/bin/sleep")
            .arg("600")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Whether `pid` is gone (or a zombie) within `limit`.
    async fn gone_within(pid: u32, limit: Duration) -> bool {
        let t = std::time::Instant::now();
        while !process::is_gone(pid) && t.elapsed() < limit {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        process::is_gone(pid)
    }

    /// What a process that should be gone is doing: its state, and on
    /// Linux its `/proc` stat line (state, parent, group) and wait channel.
    fn linger(pid: u32) -> String {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let wchan = std::fs::read_to_string(format!("/proc/{pid}/wchan")).unwrap_or_default();
        format!(
            "{:?}, pgid {}, stat {:?}, wchan {wchan:?}",
            process::state(pid),
            pgid(pid),
            stat.trim()
        )
    }

    fn pgid(pid: u32) -> i32 {
        // SAFETY: a plain query of a process this test started.
        unsafe { libc::getpgid(pid as libc::pid_t) }
    }

    /// The loop ends a worker whose mecha was `SIGKILL`ed, and leaves one
    /// whose mecha lives. It sits in a process group of its own and never in
    /// its starter's, which is what keeps it from taking that group's other
    /// members down with it (#772's runner).
    #[tokio::test]
    async fn the_watchdog_ends_a_worker_whose_mecha_was_killed() {
        let mut kids = Kids(vec![sleeper(), sleeper()]);
        let (worker, mecha) = (kids.0[0].id(), kids.0[1].id());
        let dog = Watchdog::arm(worker, mecha, PS).await.unwrap();
        let loop_pid = dog.pid().unwrap();
        assert_eq!(
            pgid(loop_pid),
            loop_pid as i32,
            "the loop leads its own group"
        );
        // SAFETY: a plain query of this process.
        assert_ne!(pgid(loop_pid), unsafe { libc::getpgrp() });
        // Past two polls: a worker whose mecha lives is left alone.
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(
            !process::is_gone(worker),
            "the worker ended while its mecha lived"
        );
        // Reaped, as a real mecha's parent would: a zombie still answers
        // `kill -0` and `ps`.
        let _ = kids.0[1].kill();
        let _ = kids.0[1].wait();
        assert!(
            gone_within(worker, Duration::from_secs(10)).await,
            "the worker outlived its killed mecha"
        );
        assert!(
            gone_within(loop_pid, Duration::from_secs(3)).await,
            "the loop outlived its worker"
        );
    }

    /// A `ps` that fails mid-program is no answer: the loop neither kills a
    /// worker whose mecha lives nor retires, and once `ps` answers again it
    /// ends the worker of a mecha that died meanwhile. An earlier loop read
    /// the empty answer as "mecha is gone", and with the worker's read
    /// failing too it exited, so the worker outlived its mecha (review of
    /// #772).
    #[tokio::test]
    async fn the_watchdog_takes_no_answer_from_a_ps_that_fails() {
        let dir = std::env::temp_dir().join(format!("mecha-ps-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let (calls, failing, ps) = (dir.join("calls"), dir.join("failing"), dir.join("ps"));
        std::fs::write(
            &ps,
            format!(
                "#!/bin/sh\necho >>{}\n[ -e {} ] && exit 1\nexec /bin/ps \"$@\"\n",
                sh_quote(&calls.display().to_string()),
                sh_quote(&failing.display().to_string()),
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ps, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let reads = || {
            std::fs::read_to_string(&calls)
                .map(|c| c.lines().count())
                .unwrap_or(0)
        };
        let mut kids = Kids(vec![sleeper(), sleeper()]);
        let (worker, mecha) = (kids.0[0].id(), kids.0[1].id());
        let _dog = Watchdog::arm(worker, mecha, &ps.display().to_string())
            .await
            .unwrap();
        // The handshake's two reads answered. The stand-in counts a read as
        // it starts, so arm at the loop's first: arming at the second could
        // land mid-read (review of #772).
        let t = std::time::Instant::now();
        while reads() < 3 && t.elapsed() < Duration::from_secs(10) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        std::fs::write(&failing, "").unwrap();
        let before = reads();
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(
            !process::is_gone(worker),
            "a failed read of a live mecha killed its worker"
        );
        assert!(
            reads() > before,
            "the loop stopped asking after a failed read"
        );
        // Mecha dies while every read fails: the worker's start time is
        // unknown, so it is not killed yet, and the loop keeps asking.
        let _ = kids.0[1].kill();
        let _ = kids.0[1].wait();
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(!process::is_gone(worker));
        let before = reads();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(reads() > before, "the loop retired on an unreadable answer");
        // `ps` answers again, and the orphan is ended.
        std::fs::remove_file(&failing).unwrap();
        assert!(
            gone_within(worker, Duration::from_secs(10)).await,
            "the worker outlived its killed mecha once ps answered again"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A dropped [`Watchdog`] is never killed: the program that held it is
    /// gone, but the worker may not be, and the loop has to outlive mecha
    /// itself. With `kill_on_drop(true)` the loop dies here and the worker of
    /// a killed mecha lives on (review of #772).
    #[tokio::test]
    async fn a_dropped_watchdog_keeps_guarding() {
        let mut kids = Kids(vec![sleeper(), sleeper()]);
        let (worker, mecha) = (kids.0[0].id(), kids.0[1].id());
        let dog = Watchdog::arm(worker, mecha, PS).await.unwrap();
        let loop_pid = dog.pid().unwrap();
        drop(dog);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(
            !process::is_gone(loop_pid),
            "dropping the watchdog ended its loop"
        );
        let _ = kids.0[1].kill();
        let _ = kids.0[1].wait();
        assert!(
            gone_within(worker, Duration::from_secs(10)).await,
            "a dropped watchdog stopped guarding"
        );
    }

    /// A `ps` that hangs times the handshake out, and the refusal takes the
    /// whole group with it: signalling only the shell, as `start_kill` does,
    /// left the hung `ps` running (review of #780).
    #[tokio::test]
    async fn a_failed_handshake_takes_its_hung_ps_with_it() {
        let dir = std::env::temp_dir().join(format!("mecha-ps-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let (pids, ps) = (dir.join("pids"), dir.join("ps"));
        std::fs::write(
            &ps,
            format!(
                "#!/bin/sh\necho $$ >>{}\nexec /bin/sleep 300\n",
                sh_quote(&pids.display().to_string())
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ps, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let kids = Kids(vec![sleeper(), sleeper()]);
        let (worker, mecha) = (kids.0[0].id(), kids.0[1].id());
        let e = Watchdog::arm_within(
            worker,
            mecha,
            &ps.display().to_string(),
            Duration::from_secs(1),
        )
        .await
        .err()
        .expect("a hung ps armed a watchdog");
        assert!(format!("{e:#}").contains("no answer"), "{e:#}");
        let hung: Vec<u32> = std::fs::read_to_string(&pids)
            .unwrap()
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect();
        assert_eq!(hung.len(), 1, "the handshake's first read hung: {hung:?}");
        // What this test claims is that the group kill reached the hung `ps`:
        // gone, or a `SIGKILL` pending. How soon the kernel finishes it off is
        // the scheduler's, and on loaded runners it read as alive to the last
        // poll and as gone a moment later (#785, #783, #788's CI), so the poll
        // keeps the last state it saw for the message.
        let t = std::time::Instant::now();
        let mut seen = String::new();
        while !process::is_killed(hung[0]) && t.elapsed() < Duration::from_secs(10) {
            seen = linger(hung[0]);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let killed = process::is_killed(hung[0]);
        if !killed {
            // SAFETY: a plain signal to the stand-in this test started.
            unsafe { libc::kill(hung[0] as libc::pid_t, libc::SIGKILL) };
        }
        assert!(
            killed,
            "the hung ps outlived the refused watchdog, last seen as {seen}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A loop that cannot read the start times is refused, and says why:
    /// one that cannot tell a reused pid from its own must not guard by
    /// guessing, and the worker it would have guarded is left as it was.
    #[tokio::test]
    async fn a_watchdog_that_cannot_read_start_times_is_refused() {
        let kids = Kids(vec![sleeper(), sleeper()]);
        let (worker, mecha) = (kids.0[0].id(), kids.0[1].id());
        let e = Watchdog::arm(worker, mecha, "/nonexistent/ps")
            .await
            .err()
            .expect("an unguarded worker");
        assert!(format!("{e:#}").contains("unreadable"), "{e:#}");
        assert!(!process::is_gone(worker));
    }
}
