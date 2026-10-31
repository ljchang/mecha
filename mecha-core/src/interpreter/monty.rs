//! The monty backend: programs run in `monty-pool` workers, each worker a
//! pinned `monty` binary ([`super::worker`]) inside the sandbox.
//!
//! **The pool is a third process class** beside llama-server seats and
//! sandboxed MCP servers (CODE-MODE-DESIGN.md §7), and it gets the whole of
//! `mcp::McpClient::connect`'s treatment:
//! - **The environment is cleared.** `monty-pool` spawns its workers with
//!   `env_clear`, and the launcher clears it again on top — bwrap
//!   `--clearenv`, or `env -i` ahead of `sandbox-exec` — so no provider key
//!   reaches a worker by either route.
//! - **The worker is confined, with no network and no filesystem beyond its
//!   own binary.** On Linux bwrap unshares every namespace it can, binds the
//!   pinned binary read-only at `/monty` and nothing else — no `/usr`, no
//!   `/proc`, no `/dev`, a read-only empty root. The Linux worker is a static
//!   musl build, which is what makes that possible. On a Mac `sandbox-exec`
//!   runs it under its own Seatbelt profile
//!   ([`crate::sandbox::seatbelt::worker_profile`]): deny by default, exec of
//!   the binary, the root's listing and one named sysctl — no other read, no
//!   write, no network, no service.
//! - **No mounts, and every `OsCall` and `NameLookup` is refused.** Monty's
//!   files, environment, clock-driven sleeps and unknown imports all arrive
//!   as `OsCall`s; this backend answers each `NotHandled`, so the program
//!   gets monty's own `PermissionError`/`RuntimeError`/`ModuleNotFoundError`.
//!   A `NameLookup` (an undefined name) is answered `Undefined`. Registered
//!   functions are globals, so no lookup is ever needed to reach one.
//! - **`preflight` runs a program through the real confinement**, and a
//!   sandbox this backend cannot use refuses at construction: the silently
//!   degrading guard is the one shape this module must never take. Under
//!   Seatbelt it also asks the kernel what the running worker's profile
//!   denies (`sandbox::seatbelt::live`), so a launcher that ran the worker
//!   bare fails the start rather than passing it.
//!
//! **Which confinement is the platform's, not a setting.** On Linux it is
//! `[sandbox] kind = "bwrap"`, which also carries `memory_mb` and `cpus` to
//! the worker; docker would leave a killed worker's container running (the
//! pool kills one pid, `docker run`'s client), landlock cannot close the
//! network and applies its rules in a `pre_exec` the pool does not expose,
//! and `none` is no second wall, so each refuses by name. On a Mac it is
//! Seatbelt whatever `[sandbox]` names: `[sandbox]` is `shell`'s table and
//! cannot name Seatbelt (`SEATBELT_IS_DOCUMENTS_ONLY`), the profile is fixed
//! in code, and no value loosens it or runs the worker bare.
//!
//! **A Mac has no memory wall outside the worker.** There are no cgroups, so
//! `[sandbox] memory_mb` never reaches it, and the bound is monty's own
//! allocator ceiling (`Backstops::max_memory`) alone (CODE-MODE-DESIGN.md
//! §5).

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use monty_pool::{
    on_print_sync, Checkout, Pool, PoolConfig, PoolError, ReplConfig, ResumeValue, TurnEvent,
};
use monty_types::{
    CallArgs, ExcType, MontyException, MontyObject, NameLookupResult, NamedValues, ObjectRef,
    OsPolicy, ResourceLimits, SleepMode,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::worker::VerifiedWorker;
use super::{
    registrable, Backstop, Backstops, Builtin, End, ExternalCall, Finished, HostRaise, Interpreter,
    Program, Raised, Step, Value,
};
use crate::sandbox::{Backend, Sandbox};

/// `sandbox-exec`, and `env`, which clears what the launcher's `/bin/sh`
/// would export ahead of it. System paths, so no `PATH` is searched.
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
const ENV: &str = "/usr/bin/env";

/// How a worker is confined; see the module docs for why the other backends
/// refuse.
#[derive(Debug, Clone)]
pub enum Confinement {
    /// `launcher` is what starts bwrap — bwrap itself, or `systemd-run`
    /// carrying the sandbox's memory and CPU limits — with every program in
    /// it resolved to an absolute path, because the worker's environment is
    /// empty and has no `PATH` to search.
    Bwrap { launcher: Vec<String> },
    /// macOS: `sandbox_exec` runs the worker under
    /// [`crate::sandbox::seatbelt::worker_profile`], behind `env -i`.
    Seatbelt { sandbox_exec: PathBuf },
}

impl Confinement {
    /// Whether the launcher must start [`WATCHDOG`] beside the worker:
    /// Seatbelt has no `--die-with-parent`, so nothing else ends a worker
    /// whose mecha was `SIGKILL`ed (`CODE-MODE-DESIGN.md` §7).
    fn watchdog(&self) -> bool {
        matches!(self, Confinement::Seatbelt { .. })
    }

    /// The confinement this platform gives a worker, or a refusal naming why
    /// it gives none. On Linux that is the configured sandbox, and `none`
    /// refuses too: the sandbox is the second wall (§7), because round 1 of
    /// Hack Monty was an escape. On a Mac it is [`Self::seatbelt`] whatever
    /// `sandbox` names — `[sandbox]` is `shell`'s, and its one value that
    /// could name Seatbelt is a load error.
    pub fn for_sandbox(sandbox: &Sandbox) -> Result<Self> {
        if cfg!(target_os = "macos") {
            return Self::seatbelt();
        }
        match sandbox.backend() {
            Backend::Bwrap => {
                if !cfg!(target_os = "linux") {
                    bail!(
                        "bwrap is Linux-only; code mode has no worker sandbox on this platform yet"
                    );
                }
                let launcher = sandbox
                    .bwrap_launcher()
                    .into_iter()
                    .map(|a| match a.as_str() {
                        "bwrap" | "systemd-run" => on_path(&a)
                            .map(|p| p.display().to_string())
                            .with_context(|| format!("`{a}` is not on PATH")),
                        _ => Ok(a),
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Confinement::Bwrap { launcher })
            }
            Backend::None => bail!(
                "code mode runs its interpreter only inside a sandbox, and [sandbox] kind is \
                 \"none\"; set kind = \"bwrap\""
            ),
            Backend::Docker => bail!(
                "code mode cannot confine its interpreter with docker yet: the pool kills a \
                 worker by its pid, which under docker is the client, not the container; \
                 set [sandbox] kind = \"bwrap\""
            ),
            Backend::Landlock => bail!(
                "code mode cannot confine its interpreter with landlock: it cannot close the \
                 network, and its rules apply in a pre_exec the worker pool does not expose; \
                 set [sandbox] kind = \"bwrap\""
            ),
            Backend::Seatbelt => bail!(
                "the seatbelt sandbox is macOS's own; here code mode's interpreter is confined \
                 by [sandbox] kind = \"bwrap\""
            ),
        }
    }

    /// macOS's confinement for a worker: `sandbox-exec` with the worker's
    /// own profile. Refuses anywhere else, and where `sandbox-exec` is gone.
    pub fn seatbelt() -> Result<Self> {
        if !cfg!(target_os = "macos") {
            bail!("the seatbelt sandbox is macOS's own; it does not run here");
        }
        let sandbox_exec = PathBuf::from(SANDBOX_EXEC);
        if !sandbox_exec.is_file() {
            bail!("{SANDBOX_EXEC} is missing, so code mode has no worker sandbox here");
        }
        Ok(Confinement::Seatbelt { sandbox_exec })
    }

    /// Seatbelt through an explicit `sandbox-exec` — for a test that needs
    /// one which fails, or one that ignores its profile. Not a construction
    /// path, as [`Self::bwrap_at`] is not.
    #[doc(hidden)]
    pub fn seatbelt_at(program: impl Into<PathBuf>) -> Self {
        Confinement::Seatbelt {
            sandbox_exec: program.into(),
        }
    }

    /// bwrap at an explicit path — for a test that needs one which fails.
    /// Not a construction path: it skips [`Self::for_sandbox`]'s backend and
    /// platform gate, so every caller outside the tests goes through that.
    #[doc(hidden)]
    pub fn bwrap_at(program: impl Into<PathBuf>) -> Self {
        Confinement::Bwrap {
            launcher: vec![program.into().display().to_string()],
        }
    }

    /// The full argv that starts one worker, up to the worker's own
    /// arguments (`monty-pool` appends `subprocess`).
    fn argv(&self, worker: &Path) -> Result<Vec<String>> {
        let launcher = match self {
            Confinement::Bwrap { launcher } => launcher,
            // `env -i` execs `sandbox-exec`, which applies the profile and
            // execs the worker: one process from the launcher's shell to the
            // program, so the pid the pool holds is the confined worker.
            // There is no `--die-with-parent` here; the launcher's
            // [`WATCHDOG`] stands in for it.
            Confinement::Seatbelt { sandbox_exec } => {
                return Ok(vec![
                    ENV.into(),
                    "-i".into(),
                    sandbox_exec.display().to_string(),
                    "-p".into(),
                    crate::sandbox::seatbelt::worker_profile(worker)?,
                    worker.display().to_string(),
                ])
            }
        };
        let mut a = launcher.clone();
        a.extend(
            [
                "--die-with-parent",
                "--new-session",
                "--unshare-all",
                // Named again: `--disable-userns` requires it spelled out.
                "--unshare-user",
                "--disable-userns",
                "--clearenv",
                "--ro-bind",
            ]
            .map(String::from),
        );
        a.push(worker.display().to_string());
        a.extend(
            [
                "/monty",
                "--remount-ro",
                "/",
                "--chdir",
                "/",
                "--",
                "/monty",
            ]
            .map(String::from),
        );
        Ok(a)
    }

    /// The running worker's confinement, read from the kernel rather than
    /// believed from the argv. bwrap's is proven by the worker starting at
    /// all inside an empty root; Seatbelt's by asking what the profile on
    /// the live process denies.
    fn check_live(&self, pid: u32) -> Result<()> {
        match self {
            Confinement::Bwrap { .. } => Ok(()),
            Confinement::Seatbelt { .. } => seatbelt_holds(pid),
        }
    }
}

/// The preflight's reading of a Seatbelt worker: under a profile, and one
/// that denies `/etc`, the home, a write to `/dev/null`, a shell and the
/// network, and grants its one read — the control. Each is a question to the kernel, and an answer
/// it will not give fails the start like a wrong one.
#[cfg(target_os = "macos")]
fn seatbelt_holds(pid: u32) -> Result<()> {
    use crate::sandbox::seatbelt::live;
    if live::is_sandboxed(pid) != Some(true) {
        bail!("process {pid} is not under a Seatbelt profile");
    }
    // The kernel matches resolved paths: `/etc` is `/private/etc`.
    let canon = |p: PathBuf| p.canonicalize().unwrap_or(p);
    // Each operation is one the kernel answers both ways, measured against
    // an unsandboxed process on macOS 26: `process-exec` has to be asked
    // as `process-exec*` (the bare name is an error), and a path that does
    // not exist reads as denied even unsandboxed, so every path asked about
    // exists — a write is asked of `/dev/null`.
    let mut probes: Vec<(&str, PathBuf)> = vec![
        ("file-read-data", canon(PathBuf::from("/etc/passwd"))),
        ("file-write-data", PathBuf::from("/dev/null")),
        ("process-exec*", PathBuf::from("/bin/sh")),
    ];
    // Only a home that resolves: an unresolved one would read as denied for
    // free, and the probes above stand without it (review of #769).
    if let Some(home) = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .and_then(|h| PathBuf::from(h).canonicalize().ok())
    {
        probes.push(("file-read-data", home));
    }
    for (op, path) in &probes {
        if live::denies_path(pid, op, path) != Some(true) {
            bail!(
                "the worker's profile does not deny {op} on {}",
                path.display()
            );
        }
    }
    if live::denies(pid, "network-outbound") != Some(true) {
        bail!("the worker's profile does not deny the network");
    }
    // The control: the one read the profile grants must read as allowed on
    // the same process, or a check that answered "denied" to everything
    // would have passed every probe above (review of #769).
    if live::denies_path(pid, "file-read-data", Path::new("/")) != Some(false) {
        bail!("the kernel does not read the worker's own profile back: its one granted read is not allowed");
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn seatbelt_holds(_pid: u32) -> Result<()> {
    bail!("the seatbelt sandbox is macOS's own; it does not run here")
}

/// `name` on mecha's own `PATH` — read once here, at construction, because
/// the worker has none.
fn on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(name))
            .find(|c| c.is_file())
    })
}

/// A private directory holding the launcher script and the workers' stderr,
/// removed with the interpreter. `monty-pool` takes one binary path and
/// appends `subprocess`, so the confinement has to be an executable; it is
/// written here, mode 0700, by mecha alone.
#[derive(Debug)]
struct Launcher {
    dir: PathBuf,
    script: PathBuf,
    log: PathBuf,
}

impl Launcher {
    /// `watchdog` puts [`WATCHDOG`] ahead of the `exec`.
    fn write(argv: &[String], watchdog: bool) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("mecha-monty-{}", uuid::Uuid::new_v4()));
        let mut builder = std::fs::DirBuilder::new();
        // Created 0700, never chmodded after: no window where it is open.
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
            .create(&dir)
            .with_context(|| format!("creating {}", dir.display()))?;
        let script = dir.join("worker");
        let log = dir.join("worker.log");
        let quoted_log = sh_quote(&log.display().to_string());
        let mut body = String::from("#!/bin/sh\n");
        if watchdog {
            body.push_str(&WATCHDOG.replace("{log}", &quoted_log));
        }
        body.push_str("exec");
        for a in argv {
            body.push(' ');
            body.push_str(&sh_quote(a));
        }
        body.push_str(" \"$@\" 2>>");
        body.push_str(&quoted_log);
        body.push('\n');
        std::fs::write(&script, body)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Launcher { dir, script, log })
    }

    /// The last lines a worker wrote to stderr, for a refusal's message.
    /// Reads only the file's last 4 KiB, however long a worker has run.
    fn log_tail(&self) -> String {
        use std::io::{Read, Seek, SeekFrom};
        let mut text = String::new();
        if let Ok(mut f) = std::fs::File::open(&self.log) {
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            let _ = f.seek(SeekFrom::Start(len.saturating_sub(4096)));
            let mut bytes = Vec::new();
            let _ = f.read_to_end(&mut bytes);
            text = String::from_utf8_lossy(&bytes).into_owned();
        }
        let lines: Vec<&str> = text.lines().rev().take(5).collect();
        lines.into_iter().rev().collect::<Vec<_>>().join("\n")
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What ends a Seatbelt worker whose mecha died without dropping the pool.
///
/// bwrap's `--die-with-parent` has no macOS counterpart, and the launch
/// chain is all `exec`s, so the pool's kill on drop is the only thing that
/// ends a worker, and a `SIGKILL`ed mecha never runs it. A worker computing
/// at that moment never reads its stdin's EOF, and the wall clock died with
/// the host, so it would spin until someone killed it.
///
/// The launcher runs this before it `exec`s the chain, so `$$` is the
/// worker's pid from then on, and `$PPID` is mecha. A loop polls both with
/// `kill -0` (a builtin, so nothing forks but `sleep`) once a second, and
/// ends when the worker does. When mecha is gone first, it kills the worker,
/// but only if that pid still started when the worker did. `exec` keeps a
/// process's start time, so a pid the kernel has since reused is never
/// killed. A start time that cannot be read fails the launch: the worker
/// never starts, and preflight refuses code mode, rather than running it
/// unguarded. The loop forks twice to leave the worker's process tree, so
/// it neither outlives the worker inside it nor stops with it, and it holds
/// none of the worker's pipes. It runs outside the profile: it is this
/// fixed text, with no input from a program. `PATH` is empty here, so every
/// command it forks is named absolutely. `{log}` is the quoted stderr log.
const WATCHDOG: &str = r#"w=$$ p=$PPID
t=$(/bin/ps -o lstart= -p "$w" 2>>{log})
if [ -z "$t" ]; then echo "watchdog: the worker's start time is unreadable" >>{log}; exit 1; fi
( (
while kill -0 "$w" 2>/dev/null; do
if ! kill -0 "$p" 2>/dev/null; then
[ "$(/bin/ps -o lstart= -p "$w" 2>/dev/null)" = "$t" ] && kill -9 "$w"
exit 0
fi
/bin/sleep 1
done
) </dev/null >/dev/null 2>&1 & ) </dev/null >/dev/null 2>&1
"#;

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// How many programs one worker process runs before the pool retires it:
/// one. The pool serves every conversation in the process, so a worker
/// that ran a second program could run it for a different conversation,
/// with a different taint, owner-facing posture or trust context, inside
/// the process that had held the first one's state; a program that escaped
/// the interpreter once (Hack Monty round 1) would then be waiting for the
/// next one. Pinning workers to a conversation would keep reuse, but one
/// process per program is the stronger boundary and costs about 4 ms of
/// cold start per program (`CODE-MODE-DESIGN.md` §10, M1), against passes
/// and tool reads measured in tenths of a second (review of #765).
pub const PROGRAMS_PER_WORKER: u32 = 1;

/// Programs in `monty-pool` workers.
///
/// Field order is load-bearing: fields drop in declaration order, so the
/// pool (and its workers) goes before the launcher's directory is removed.
pub struct Monty {
    pool: Pool,
    repl: ReplConfig,
    backstops: Backstops,
    confinement: Confinement,
    launcher: Launcher,
}

impl Monty {
    /// A pool of up to `max_workers` confined workers. Nothing is spawned
    /// until the first program (or [`Interpreter::preflight`]) needs one.
    pub async fn new(
        worker: &VerifiedWorker,
        confinement: &Confinement,
        backstops: Backstops,
        max_workers: usize,
    ) -> Result<Self> {
        // Hashed again here, so "verified" means "at this pool's start",
        // not whenever a `VerifiedWorker` was last made.
        let worker = super::worker::verify(worker.path())?;
        let launcher = Launcher::write(&confinement.argv(worker.path())?, confinement.watchdog())?;
        let mut cfg = PoolConfig::subprocess(&launcher.script);
        cfg.min_processes = 0;
        cfg.max_processes = max_workers.max(1);
        // Unset by default, and the one deadline the pool enforces from
        // outside the worker: per protocol turn, so the program's own wall
        // clock (which spans turns) is this module's, below.
        cfg.request_timeout = Some(backstops.wall_clock);
        cfg.checkout_timeout = Some(Duration::from_secs(60));
        // One program per worker (`PROGRAMS_PER_WORKER`): a worker is never
        // handed back to serve another.
        cfg.max_checkouts_per_worker = Some(PROGRAMS_PER_WORKER);
        let pool = Pool::new(cfg)
            .await
            .map_err(|e| anyhow::anyhow!("starting the monty pool: {e}"))?;
        let limits = ResourceLimits::default()
            .max_feed_duration(backstops.wall_clock)
            .max_memory(backstops.max_memory)
            .max_recursion_depth(backstops.max_recursion_depth)
            // One above the backstop: this module counts suspensions itself
            // and kills the worker at the first one past the limit, so a
            // program cannot raise the pool's message and be believed. The
            // pool's own budget stays behind it, one call later.
            .max_suspensions(
                usize::try_from(backstops.max_suspensions.saturating_add(1)).unwrap_or(usize::MAX),
            );
        let repl = ReplConfig {
            limits: Some(limits),
            os_policy: OsPolicy {
                // A sleep goes to the host, which refuses it like any OsCall.
                sleep: SleepMode::CallHost,
                ..OsPolicy::default()
            },
            ..ReplConfig::default()
        };
        Ok(Monty {
            pool,
            repl,
            backstops,
            confinement: confinement.clone(),
            launcher,
        })
    }
}

#[async_trait]
impl Interpreter for Monty {
    fn name(&self) -> &'static str {
        "monty"
    }

    /// A program that stops at a host call, so the live worker's
    /// confinement is read while it runs, and then finishes.
    async fn preflight(&self) -> Result<()> {
        let ran = async {
            let (mut p, step) = self
                .start("21 * probe()", &["probe".into()], CancellationToken::new())
                .await?;
            if !matches!(&step, Step::Call(c) if c.name == "probe") {
                bail!("a preflight program reached {step:?} instead of its call");
            }
            let pid = p.worker_pid().context("a preflight worker with no pid")?;
            // A refusal drops `p`, which kills the worker.
            self.confinement.check_live(pid)?;
            match p.resume(Value::from(2)).await? {
                Step::Done(Finished {
                    end: End::Complete { value: Some(v), .. },
                    ..
                }) if v == 42 => Ok(()),
                other => bail!("a preflight program ended {other:?}"),
            }
        };
        match tokio::time::timeout(Duration::from_secs(30), ran).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => bail!(
                "the monty worker does not run inside the sandbox: {e:#}{}; refusing to run \
                 programs unconfined",
                match self.launcher.log_tail() {
                    t if t.is_empty() => String::new(),
                    t => format!(" (worker stderr: {t})"),
                }
            ),
            Err(_) => bail!("the monty worker did not answer within 30 s inside the sandbox"),
        }
    }

    async fn start(
        &self,
        source: &str,
        externals: &[String],
        cancel: CancellationToken,
    ) -> Result<(Box<dyn Program>, Step)> {
        for name in externals {
            if !registrable(name) {
                bail!("`{name}` cannot be registered as an external function");
            }
        }
        let started = Instant::now();
        let deadline = started + self.backstops.wall_clock;
        // A Stop while every worker is busy ends the program here, rather
        // than waiting out `checkout_timeout` (review of #765).
        let checkout = tokio::select! {
            c = self.pool.checkout(&self.repl) => Some(c.map_err(|e| anyhow::anyhow!("checking out a monty worker: {e}"))?),
            _ = cancel.cancelled() => None,
            _ = tokio::time::sleep_until(deadline) => bail!("no monty worker within the program's wall clock"),
        };
        let cancelled_waiting = checkout.is_none();
        let externals: BTreeSet<String> = externals.iter().cloned().collect();
        let inputs: Vec<(String, MontyObject)> = externals
            .iter()
            .map(|n| (n.clone(), MontyObject::function(n.clone(), None)))
            .collect();
        let mut program = MontyProgram::new(checkout, externals, self.backstops, started, cancel);
        let step = if cancelled_waiting {
            program.end(End::Cancelled, false).await
        } else {
            program
                .run(Req::Feed(source.to_string(), NamedValues::from(inputs)))
                .await?
        };
        Ok((Box::new(program), step))
    }
}

#[derive(Default)]
struct Printed {
    text: String,
    dropped: u64,
}

/// What the next protocol turn sends.
enum Req {
    Feed(String, NamedValues),
    Resume(ResumeValue),
    Undefined,
}

/// How a protocol turn came back.
enum Turned {
    Event(TurnEvent),
    Pool(PoolError),
    Deadline,
    Cancelled,
}

struct MontyProgram {
    /// `None` once the program is over: finished back to the pool, or
    /// dropped, which kills the worker.
    checkout: Option<Checkout>,
    externals: BTreeSet<String>,
    backstops: Backstops,
    printed: Arc<Mutex<Printed>>,
    started: Instant,
    deadline: Instant,
    cancel: CancellationToken,
    /// Every suspension the pool handed this program, refused or answered:
    /// the suspension backstop's count, kept by the host.
    suspensions: u64,
    /// A call is waiting on the host.
    pending: bool,
}

impl MontyProgram {
    fn new(
        checkout: Option<Checkout>,
        externals: BTreeSet<String>,
        backstops: Backstops,
        started: Instant,
        cancel: CancellationToken,
    ) -> Self {
        MontyProgram {
            checkout,
            externals,
            backstops,
            printed: Arc::new(Mutex::new(Printed::default())),
            started,
            deadline: started + backstops.wall_clock,
            cancel,
            suspensions: 0,
            pending: false,
        }
    }

    /// One protocol turn, bounded by the program's wall clock and its
    /// cancellation token. On cancel the turn gets the grace period to reach
    /// a boundary (so what it printed by then arrives); either way the
    /// caller then drops the checkout, which kills the worker.
    async fn turn(&mut self, req: Req) -> Turned {
        let Self {
            checkout,
            printed,
            deadline,
            cancel,
            backstops,
            ..
        } = self;
        let Some(co) = checkout.as_mut() else {
            return Turned::Pool(PoolError::Finished);
        };
        let sink = Arc::clone(printed);
        let cap = backstops.max_printed_bytes;
        let mut on_print = on_print_sync(move |_stream, text| {
            let mut p = sink.lock().unwrap_or_else(|e| e.into_inner());
            let room = cap.saturating_sub(p.text.len());
            if text.len() <= room {
                p.text.push_str(text);
            } else {
                let mut cut = room;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                p.text.push_str(&text[..cut]);
                p.dropped += (text.len() - cut) as u64;
            }
        });
        let fut = async {
            match req {
                Req::Feed(src, inputs) => co.feed(src, inputs, vec![], true, &mut on_print).await,
                Req::Resume(v) => co.resume(v, &mut on_print).await,
                Req::Undefined => {
                    co.resume_name_lookup(NameLookupResult::Undefined, &mut on_print)
                        .await
                }
            }
        };
        tokio::pin!(fut);
        tokio::select! {
            biased;
            r = &mut fut => match r {
                Ok(e) => Turned::Event(e),
                Err(e) => Turned::Pool(e),
            },
            _ = cancel.cancelled() => {
                let _ = tokio::time::timeout(backstops.cancel_grace, &mut fut).await;
                Turned::Cancelled
            }
            _ = tokio::time::sleep_until(*deadline) => Turned::Deadline,
        }
    }

    /// Drive the program until it calls a registered function or ends.
    async fn run(&mut self, mut req: Req) -> Result<Step> {
        loop {
            if self.cancel.is_cancelled() {
                return Ok(self.end(End::Cancelled, false).await);
            }
            if Instant::now() >= self.deadline {
                let b = Backstop::WallClock(self.backstops.wall_clock);
                return Ok(self.end(End::Backstop(b), false).await);
            }
            let event = match self.turn(req).await {
                Turned::Event(e) => e,
                Turned::Pool(e) => return self.failed(e).await,
                Turned::Deadline => {
                    let b = Backstop::WallClock(self.backstops.wall_clock);
                    return Ok(self.end(End::Backstop(b), false).await);
                }
                Turned::Cancelled => return Ok(self.end(End::Cancelled, false).await),
            };
            // The suspension backstop, counted here rather than believed from
            // a message: the first suspension past the limit kills the worker.
            if !matches!(
                event,
                TurnEvent::Complete(_) | TurnEvent::ResolveFutures { .. }
            ) {
                self.suspensions += 1;
                if self.suspensions > self.backstops.max_suspensions {
                    let b = Backstop::Suspensions(self.backstops.max_suspensions);
                    return Ok(self.end(End::Backstop(b), false).await);
                }
            }
            req = match event {
                TurnEvent::FunctionCall {
                    function_name,
                    args,
                    object_id,
                    ..
                } => {
                    // Registered functions are plain globals; a call routed to
                    // a host object, or under any other name, has no handler.
                    if object_id.is_some() || !self.externals.contains(&function_name) {
                        Req::Resume(ResumeValue::Error(MontyException::new(
                            ExcType::NameError,
                            Some(format!("name '{function_name}' is not defined")),
                        )))
                    } else {
                        match call_args(&args) {
                            Ok((args, kwargs)) => {
                                self.pending = true;
                                return Ok(Step::Call(ExternalCall {
                                    name: function_name,
                                    args,
                                    kwargs,
                                }));
                            }
                            Err(why) => Req::Resume(ResumeValue::Error(MontyException::new(
                                ExcType::TypeError,
                                Some(format!("{function_name}(): {why}")),
                            ))),
                        }
                    }
                }
                // Files, the environment, sleeps, imports: no handler.
                TurnEvent::OsCall { .. } => Req::Resume(ResumeValue::NotHandled),
                TurnEvent::NameLookup { .. } => Req::Undefined,
                // Only an answer of `Future` creates one, and this host never
                // gives that answer.
                TurnEvent::ResolveFutures { .. } => {
                    self.checkout = None;
                    bail!("the monty worker asked to resolve futures this host never created");
                }
                TurnEvent::Complete(obj) => {
                    let value = to_json(obj.as_ref(), 0).ok();
                    let repr = obj.py_repr();
                    return Ok(self.end(End::Complete { value, repr }, true).await);
                }
            };
        }
    }

    /// A pool error, sorted: a backstop, the program's own uncaught
    /// exception, or the backend failing (`Err`).
    async fn failed(&mut self, e: PoolError) -> Result<Step> {
        match e {
            PoolError::Runtime(exc) => {
                let worker_gone = self.checkout.as_ref().is_none_or(|c| c.pid().is_none());
                if let Some(b) = self.backstop(&exc, worker_gone) {
                    return Ok(self.end(End::Backstop(b), false).await);
                }
                let raised = Raised {
                    exc_type: exc.exc_type().to_string(),
                    message: exc.message().map(str::to_string),
                    traceback: exc.to_string(),
                };
                Ok(self.end(End::Raised(raised), !worker_gone).await)
            }
            // The pool killed the worker at `request_timeout`.
            PoolError::Timeout { .. } => {
                let b = Backstop::WallClock(self.backstops.wall_clock);
                Ok(self.end(End::Backstop(b), false).await)
            }
            // The worker vanished mid-program: the sandbox's cgroup limit
            // (`[sandbox] memory_mb` below `max_memory`) SIGKILLs it before
            // monty's own check fires, or the program crashed it. Either way
            // the program brought it about, so it ends the program rather
            // than reading as the backend failing. A worker that announced
            // its own fatal error (version skew, say) is the backend's.
            PoolError::Crashed {
                status,
                cause: monty_pool::CrashCause::Vanished { .. },
            } => {
                #[cfg(unix)]
                let signal = status.and_then(|s| std::os::unix::process::ExitStatusExt::signal(&s));
                #[cfg(not(unix))]
                let signal: Option<i32> = {
                    let _ = status;
                    None
                };
                Ok(self
                    .end(End::Backstop(Backstop::WorkerKilled { signal }), false)
                    .await)
            }
            other => {
                self.checkout = None;
                bail!("the monty worker failed: {other}")
            }
        }
    }

    /// Whether an exception is a backstop rather than the program's own.
    /// A program can raise a built-in with any message, so a message alone
    /// is checked against what the host can see for itself: the clock for a
    /// time limit, a dead worker for the allocator's kill. The suspension
    /// budget never arrives here: [`Self::run`] counts it and kills first.
    /// The interpreter's own heap check is the one read from its message
    /// (which carries the configured limit), so a program that raises that
    /// message verbatim is misreported as the memory backstop — a
    /// misleading line, not a bypass, since it ends the program either way.
    fn backstop(&self, exc: &MontyException, worker_gone: bool) -> Option<Backstop> {
        let msg = exc.message().unwrap_or_default();
        let b = &self.backstops;
        match exc.exc_type() {
            ExcType::MemoryError
                if worker_gone
                    || (msg.starts_with("memory limit exceeded:")
                        && msg.ends_with(&format!("> {} bytes", b.max_memory))) =>
            {
                Some(Backstop::Memory(b.max_memory))
            }
            ExcType::TimeoutError
                if msg.contains("time limit exceeded")
                    && self.started.elapsed() >= b.wall_clock =>
            {
                Some(Backstop::WallClock(b.wall_clock))
            }
            _ => None,
        }
    }

    /// End the program: hand a clean worker back to the pool, or drop the
    /// checkout, which kills it — after a backstop the heap has no
    /// correctness guarantee, and a cancelled turn's state is unknowable.
    async fn end(&mut self, end: End, reusable: bool) -> Step {
        self.pending = false;
        if let Some(co) = self.checkout.take() {
            // The hand-back is a protocol turn too, so it is bounded: a
            // worker that does not reset within the grace period is dropped
            // with the timed-out future, which kills it (review of #765).
            if reusable {
                let _ = tokio::time::timeout(self.backstops.cancel_grace, co.finish()).await;
            }
        }
        let p = self.printed.lock().unwrap_or_else(|e| e.into_inner());
        Step::Done(Finished {
            printed: p.text.clone(),
            printed_dropped: p.dropped,
            end,
        })
    }

    fn take_pending(&mut self) -> Result<()> {
        if self.checkout.is_none() {
            bail!("this program is over");
        }
        if !std::mem::take(&mut self.pending) {
            bail!("this program is not waiting on a call");
        }
        Ok(())
    }
}

#[async_trait]
impl Program for MontyProgram {
    async fn resume(&mut self, value: Value) -> Result<Step> {
        // Converted before the pending call is taken, so a host value that
        // cannot cross leaves the call still answerable.
        let obj = from_json(&value, 0)?;
        self.take_pending()?;
        self.run(Req::Resume(ResumeValue::Return(obj))).await
    }

    async fn raise(&mut self, raise: HostRaise) -> Result<Step> {
        self.take_pending()?;
        let exc = match raise.exc {
            Builtin::RuntimeError => ExcType::RuntimeError,
            Builtin::ValueError => ExcType::ValueError,
            Builtin::TypeError => ExcType::TypeError,
            Builtin::KeyError => ExcType::KeyError,
            Builtin::IndexError => ExcType::IndexError,
            Builtin::PermissionError => ExcType::PermissionError,
        };
        let exc = MontyException::new(exc, Some(raise.message));
        self.run(Req::Resume(ResumeValue::Error(exc))).await
    }

    /// The pool's child: the sandbox's launcher, whose process group (it
    /// starts a new session) holds the worker.
    fn worker_pid(&self) -> Option<u32> {
        self.checkout.as_ref().and_then(Checkout::pid)
    }
}

/// Nesting deeper than this does not cross the boundary either way.
const MAX_DEPTH: usize = 64;

/// A call's positional and keyword arguments, as JSON.
type Args = (Vec<Value>, Vec<(String, Value)>);

fn call_args(args: &CallArgs) -> std::result::Result<Args, String> {
    let positional = args
        .args()
        .enumerate()
        .map(|(i, a)| to_json(a, 0).map_err(|t| format!("argument {} is {t}", i + 1)))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let keyword = args
        .kwargs()
        .map(|(k, v)| {
            let k = k
                .as_str()
                .ok_or("a keyword that is not a string")?
                .to_string();
            let v = to_json(v, 0).map_err(|t| format!("argument `{k}` is {t}"))?;
            Ok((k, v))
        })
        .collect::<std::result::Result<Vec<_>, String>>()?;
    Ok((positional, keyword))
}

/// A program value as JSON, or a description of why it has none ("a set").
fn to_json(o: ObjectRef<'_>, depth: usize) -> std::result::Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err(format!("nested deeper than {MAX_DEPTH}"));
    }
    let t = o.type_name();
    let unsupported = || format!("a {t}, which has no JSON form");
    match t {
        "NoneType" => Ok(Value::Null),
        "bool" => o.as_bool().map(Value::Bool).ok_or_else(unsupported),
        "int" => o
            .as_int()
            .map(Value::from)
            .ok_or_else(|| "an int too large for 64 bits".into()),
        "float" => o
            .as_float()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| "a float that is not finite".into()),
        "str" => o
            .as_str()
            .map(|s| Value::String(s.to_string()))
            .ok_or_else(unsupported),
        "list" | "tuple" => o
            .items()
            .ok_or_else(unsupported)?
            .into_iter()
            .map(|i| to_json(i, depth + 1))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(Value::Array),
        "dict" => {
            let mut m = serde_json::Map::new();
            for (k, v) in o.pairs().ok_or_else(unsupported)? {
                let k = k.as_str().ok_or("a dict with a key that is not a string")?;
                m.insert(k.to_string(), to_json(v, depth + 1)?);
            }
            Ok(Value::Object(m))
        }
        _ => Err(unsupported()),
    }
}

fn from_json(v: &Value, depth: usize) -> Result<MontyObject> {
    if depth > MAX_DEPTH {
        bail!("a host value nested deeper than {MAX_DEPTH}");
    }
    Ok(match v {
        Value::Null => MontyObject::none(),
        Value::Bool(b) => MontyObject::bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => MontyObject::int(i),
            None => MontyObject::float(n.as_f64().context("a number with no float form")?),
        },
        Value::String(s) => MontyObject::string(s.clone()),
        Value::Array(a) => MontyObject::list(
            a.iter()
                .map(|x| from_json(x, depth + 1))
                .collect::<Result<Vec<_>>>()?,
        ),
        Value::Object(m) => MontyObject::dict(
            m.iter()
                .map(|(k, x)| Ok((MontyObject::string(k.clone()), from_json(x, depth + 1)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
    })
}

/// The integration tests' process helpers (`/proc`, libproc), shared so the
/// unit tests below ask the kernel the same way.
#[cfg(test)]
#[path = "../../tests/support/process.rs"]
mod test_process;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::SandboxConfig;

    use super::test_process as process;

    fn sandbox(kind: Backend) -> Sandbox {
        Sandbox::new(SandboxConfig {
            kind,
            ..Default::default()
        })
    }

    /// The pinned worker for the tests that drive this module's internals:
    /// `MECHA_MONTY_WORKER`, or fetched with `MECHA_TEST_FETCH_MONTY=1` (as
    /// CI does). A skip otherwise, and a failure under
    /// `MECHA_TEST_REQUIRE_BACKENDS=1`.
    async fn worker_for_tests() -> Option<VerifiedWorker> {
        let found = if let Some(p) = std::env::var_os("MECHA_MONTY_WORKER") {
            super::super::worker::verify(Path::new(&p)).ok()
        } else if std::env::var_os("MECHA_TEST_FETCH_MONTY").is_some() {
            let home = std::env::temp_dir().join("mecha-test-monty-home");
            super::super::worker::ensure(&home, &mut |_| {}).await.ok()
        } else {
            None
        };
        // bwrap on Linux, Seatbelt on a Mac: the platform's confinement.
        let confined = Confinement::for_sandbox(&sandbox(Backend::Bwrap)).is_ok();
        if found.is_none() || !confined {
            assert!(
                std::env::var_os("MECHA_TEST_REQUIRE_BACKENDS").is_none(),
                "the pinned monty worker or its sandbox is unavailable, and MECHA_TEST_REQUIRE_BACKENDS is set"
            );
            eprintln!("SKIPPED: the pinned monty worker or its sandbox is unavailable");
            return None;
        }
        found
    }

    /// The hand-back is a protocol turn too. A worker stopped after it sent
    /// `Complete` and before it answered the pool's `Reset` must not hold the
    /// caller for `request_timeout` (600 s): the hand-back is bounded by the
    /// grace period, and the worker is killed with the dropped future
    /// (review of #765). Driven through this module's internals, because the
    /// window between `Complete` and the hand-back is too narrow to hit from
    /// outside.
    #[tokio::test]
    async fn a_worker_stopped_before_its_hand_back_does_not_hold_the_caller() {
        let Some(w) = worker_for_tests().await else {
            return;
        };
        let backstops = Backstops {
            cancel_grace: Duration::from_secs(1),
            ..Backstops::default()
        };
        let conf = Confinement::for_sandbox(&sandbox(Backend::Bwrap)).unwrap();
        let interp = Monty::new(&w, &conf, backstops, 1).await.unwrap();
        let co = interp.pool.checkout(&interp.repl).await.unwrap();
        let launcher = co.pid().unwrap();
        let mut p = MontyProgram::new(
            Some(co),
            BTreeSet::new(),
            backstops,
            Instant::now(),
            CancellationToken::new(),
        );
        let feed = Req::Feed("1".into(), NamedValues::from(Vec::new()));
        assert!(matches!(
            p.turn(feed).await,
            Turned::Event(TurnEvent::Complete(_))
        ));
        let tree = process::tree(launcher);
        for pid in &tree {
            // SAFETY: plain signal syscalls on processes this test's pool spawned.
            unsafe { libc::kill(*pid as i32, libc::SIGSTOP) };
        }
        // A stop is pending until the kernel delivers it; wait until every
        // process reads stopped, so the hand-back below cannot be answered.
        let stopped = |pid: &u32| process::state(*pid) == Some(process::State::Stopped);
        for _ in 0..100 {
            if tree.iter().all(stopped) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            tree.iter().all(stopped),
            "the worker never stopped: {tree:?}"
        );
        let t = Instant::now();
        let step = tokio::time::timeout(
            Duration::from_secs(30),
            p.end(
                End::Complete {
                    value: None,
                    repr: "1".into(),
                },
                true,
            ),
        )
        .await
        .expect("a stopped worker held the hand-back past the grace period");
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        assert!(matches!(step, Step::Done(_)));
        // Killed, not left stopped and checked out. Under bwrap the inner
        // process goes by its parent-death signal, a beat after the outer.
        for _ in 0..50 {
            if tree.iter().all(|p| process::is_gone(*p)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        for pid in tree {
            assert!(
                process::is_gone(pid),
                "process {pid} outlived its hand-back: {:?}",
                process::state(pid)
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn every_backend_but_bwrap_refuses_to_confine_a_worker() {
        for kind in [
            Backend::None,
            Backend::Docker,
            Backend::Landlock,
            Backend::Seatbelt,
        ] {
            assert!(
                Confinement::for_sandbox(&sandbox(kind)).is_err(),
                "{kind:?}"
            );
        }
    }

    /// On a Mac the worker's confinement is Seatbelt whatever `[sandbox]`
    /// names: `shell`'s table never chooses it, and no value runs it bare.
    #[cfg(target_os = "macos")]
    #[test]
    fn on_a_mac_every_sandbox_kind_gives_the_worker_seatbelt() {
        for kind in [
            Backend::None,
            Backend::Bwrap,
            Backend::Docker,
            Backend::Landlock,
            Backend::Seatbelt,
        ] {
            assert!(
                matches!(
                    Confinement::for_sandbox(&sandbox(kind)),
                    Ok(Confinement::Seatbelt { .. })
                ),
                "{kind:?}"
            );
        }
    }

    /// Off a Mac, Seatbelt refuses by name rather than standing in for bwrap.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn seatbelt_refuses_off_a_mac() {
        let err = Confinement::seatbelt().unwrap_err().to_string();
        assert!(err.contains("macOS"), "{err}");
    }

    /// The Seatbelt argv clears the environment ahead of `sandbox-exec`, and
    /// hands it the worker's own profile and the worker, nothing more.
    #[test]
    fn the_seatbelt_worker_starts_behind_env_i_under_its_own_profile() {
        let argv = Confinement::seatbelt_at("/usr/bin/sandbox-exec")
            .argv(Path::new("/opt/m/monty"))
            .unwrap();
        let profile = crate::sandbox::seatbelt::worker_profile(Path::new("/opt/m/monty")).unwrap();
        assert_eq!(
            argv,
            vec![
                "/usr/bin/env".to_string(),
                "-i".into(),
                "/usr/bin/sandbox-exec".into(),
                "-p".into(),
                profile,
                "/opt/m/monty".into(),
            ]
        );
        assert!(Confinement::seatbelt_at("/x")
            .argv(Path::new("relative"))
            .is_err());
    }

    /// The worker's argv binds its binary and nothing else, unshares the
    /// network with everything, and clears the environment.
    #[test]
    fn the_worker_sees_its_binary_and_nothing_else() {
        let argv = Confinement::bwrap_at("/usr/bin/bwrap")
            .argv(Path::new("/opt/m/monty"))
            .unwrap();
        let binds: Vec<_> = argv
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                a.contains("bind")
                    || a.as_str() == "--proc"
                    || a.as_str() == "--dev"
                    || a.as_str() == "--tmpfs"
            })
            .map(|(i, a)| (a.as_str(), argv[i + 1].as_str()))
            .collect();
        assert_eq!(binds, vec![("--ro-bind", "/opt/m/monty")]);
        for flag in [
            "--unshare-all",
            "--clearenv",
            "--die-with-parent",
            "--disable-userns",
        ] {
            assert!(
                argv.iter().any(|a| a == flag),
                "{flag} missing from {argv:?}"
            );
        }
        assert!(!argv.iter().any(|a| a == "--share-net"));
        assert_eq!(argv.last().map(String::as_str), Some("/monty"));
    }

    #[test]
    fn the_launcher_quotes_every_argument() {
        let l = Launcher::write(&["/x/b'w rap".into(), "--ro-bind".into()], false).unwrap();
        let body = std::fs::read_to_string(&l.script).unwrap();
        assert!(
            body.starts_with("#!/bin/sh\nexec '/x/b'\\''w rap' '--ro-bind' \"$@\" 2>>'"),
            "{body}"
        );
        let dir = l.dir.clone();
        drop(l);
        assert!(
            !dir.exists(),
            "the private directory outlived the interpreter"
        );
    }

    /// A worker whose mecha was `SIGKILL`ed is ended by the launcher's
    /// watchdog, and one whose mecha lives is left alone. The script is plain
    /// `sh`, so this runs on Linux too, though only Seatbelt starts it. A
    /// stand-in mecha (a shell) starts the launcher, whose worker is
    /// `/bin/sleep`. Without the watchdog the sleep outlives the kill by ten
    /// minutes.
    #[test]
    fn the_watchdog_ends_a_worker_whose_mecha_was_killed() {
        use std::io::BufRead;
        use test_process as process;
        let l = Launcher::write(&["/bin/sleep".into()], true).unwrap();
        let mut mecha = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                "\"$0\" 600 </dev/null >/dev/null 2>&1 & echo $!; exec /bin/sleep 600",
            ])
            .arg(&l.script)
            .env_clear()
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        std::io::BufReader::new(mecha.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let worker: u32 = line.trim().parse().unwrap();
        struct Reap(u32);
        impl Drop for Reap {
            fn drop(&mut self) {
                // SAFETY: a plain signal to the process this test started.
                unsafe { libc::kill(self.0 as i32, libc::SIGKILL) };
            }
        }
        let _reap = Reap(worker);
        // Past two polls: the watchdog leaves a worker whose mecha lives, and
        // the launcher did not refuse.
        std::thread::sleep(Duration::from_millis(2500));
        assert!(
            !process::is_gone(worker),
            "the worker ended while its mecha lived: {}",
            l.log_tail()
        );
        mecha.kill().unwrap();
        // Reaped, as a real mecha's parent would: `kill -0` succeeds on a
        // zombie.
        mecha.wait().unwrap();
        let t = std::time::Instant::now();
        while !process::is_gone(worker) && t.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            process::is_gone(worker),
            "the worker outlived its killed mecha by {:?}",
            t.elapsed()
        );
    }

    #[test]
    fn values_cross_as_json_and_nothing_else_does() {
        let v = serde_json::json!({"a": [1, 2.5, "x", null, true], "b": {"c": -3}});
        let o = from_json(&v, 0).unwrap();
        assert_eq!(to_json(o.as_ref(), 0).unwrap(), v);
        let set = MontyObject::set([MontyObject::int(1)]);
        assert!(to_json(set.as_ref(), 0).unwrap_err().contains("a set"));
        let bytes = MontyObject::bytes(b"x".to_vec());
        assert!(to_json(bytes.as_ref(), 0).is_err());
    }
}
