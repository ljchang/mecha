//! The monty backend: programs run in `monty-pool` workers, each worker a
//! pinned `monty` binary ([`super::worker`]) inside the sandbox.
//!
//! **The pool is a third process class** beside llama-server seats and
//! sandboxed MCP servers (CODE-MODE-DESIGN.md §7), and it gets the whole of
//! `mcp::McpClient::connect`'s treatment:
//! - **The environment is cleared.** `monty-pool` spawns its workers with
//!   `env_clear`, and the launcher passes bwrap `--clearenv` on top, so no
//!   provider key reaches a worker by either route.
//! - **The worker is confined, with no network and no filesystem beyond its
//!   own binary.** bwrap unshares every namespace it can, binds the pinned
//!   binary read-only at `/monty` and nothing else — no `/usr`, no `/proc`,
//!   no `/dev`, a read-only empty root. The Linux worker is a static musl
//!   build, which is what makes that possible.
//! - **No mounts, and every `OsCall` and `NameLookup` is refused.** Monty's
//!   files, environment, clock-driven sleeps and unknown imports all arrive
//!   as `OsCall`s; this backend answers each `NotHandled`, so the program
//!   gets monty's own `PermissionError`/`RuntimeError`/`ModuleNotFoundError`.
//!   A `NameLookup` (an undefined name) is answered `Undefined`. Registered
//!   functions are globals, so no lookup is ever needed to reach one.
//! - **`preflight` runs a program through the real confinement**, and a
//!   sandbox this backend cannot use refuses at construction: the silently
//!   degrading guard is the one shape this module must never take.
//!
//! Only bwrap confines a worker today. Docker would leave a killed worker's
//! container running (the pool kills one pid, `docker run`'s client),
//! landlock cannot close the network and applies its rules in a `pre_exec`
//! the pool does not expose, and Seatbelt is documents-only. Each refuses by
//! name rather than running a worker unconfined.

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

/// How a worker is confined. One variant today; see the module docs for why
/// the other backends refuse.
#[derive(Debug, Clone)]
pub enum Confinement {
    /// `launcher` is what starts bwrap — bwrap itself, or `systemd-run`
    /// carrying the sandbox's memory and CPU limits — with every program in
    /// it resolved to an absolute path, because the worker's environment is
    /// empty and has no `PATH` to search.
    Bwrap { launcher: Vec<String> },
}

impl Confinement {
    /// The confinement the configured sandbox gives a worker, or a refusal
    /// naming why it gives none. `none` refuses too: the sandbox is the
    /// second wall (§7), because round 1 of Hack Monty was an escape.
    pub fn for_sandbox(sandbox: &Sandbox) -> Result<Self> {
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
            Backend::Seatbelt => bail!("{}", crate::sandbox::SEATBELT_IS_DOCUMENTS_ONLY),
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
    fn argv(&self, worker: &Path) -> Vec<String> {
        let Confinement::Bwrap { launcher } = self;
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
        a
    }
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
    fn write(argv: &[String]) -> Result<Self> {
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
        let mut body = String::from("#!/bin/sh\nexec");
        for a in argv {
            body.push(' ');
            body.push_str(&sh_quote(a));
        }
        body.push_str(" \"$@\" 2>>");
        body.push_str(&sh_quote(&log.display().to_string()));
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

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Programs in `monty-pool` workers.
///
/// Field order is load-bearing: fields drop in declaration order, so the
/// pool (and its workers) goes before the launcher's directory is removed.
pub struct Monty {
    pool: Pool,
    repl: ReplConfig,
    backstops: Backstops,
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
        let launcher = Launcher::write(&confinement.argv(worker.path()))?;
        let mut cfg = PoolConfig::subprocess(&launcher.script);
        cfg.min_processes = 0;
        cfg.max_processes = max_workers.max(1);
        // Unset by default, and the one deadline the pool enforces from
        // outside the worker: per protocol turn, so the program's own wall
        // clock (which spans turns) is this module's, below.
        cfg.request_timeout = Some(backstops.wall_clock);
        cfg.checkout_timeout = Some(Duration::from_secs(60));
        // Bound any slow leak in a long-lived child.
        cfg.max_checkouts_per_worker = Some(100);
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
            launcher,
        })
    }
}

#[async_trait]
impl Interpreter for Monty {
    fn name(&self) -> &'static str {
        "monty"
    }

    async fn preflight(&self) -> Result<()> {
        let ran = async {
            let (_p, step) = self.start("21 * 2", &[], CancellationToken::new()).await?;
            match step {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::SandboxConfig;

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
        let bwrap = Confinement::for_sandbox(&sandbox(Backend::Bwrap)).is_ok();
        if found.is_none() || !bwrap {
            assert!(
                std::env::var_os("MECHA_TEST_REQUIRE_BACKENDS").is_none(),
                "the pinned monty worker or bwrap is unavailable, and MECHA_TEST_REQUIRE_BACKENDS is set"
            );
            eprintln!("SKIPPED: the pinned monty worker or bwrap is unavailable");
            return None;
        }
        found
    }

    /// Every process under `pid`, itself included.
    fn process_tree(pid: u32) -> Vec<u32> {
        let mut out = vec![pid];
        let mut i = 0;
        while i < out.len() {
            let tasks = std::fs::read_dir(format!("/proc/{}/task", out[i]));
            for t in tasks.into_iter().flatten().flatten() {
                let kids = std::fs::read_to_string(t.path().join("children")).unwrap_or_default();
                out.extend(
                    kids.split_whitespace()
                        .filter_map(|k| k.parse::<u32>().ok()),
                );
            }
            i += 1;
        }
        out
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
        let tree = process_tree(launcher);
        for pid in &tree {
            // SAFETY: plain signal syscalls on processes this test's pool spawned.
            unsafe { libc::kill(*pid as i32, libc::SIGSTOP) };
        }
        // A stop is pending until the kernel delivers it; wait until every
        // process reads stopped, so the hand-back below cannot be answered.
        let stopped = |pid: &u32| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|s| s.rsplit_once(") ").is_some_and(|(_, r)| r.starts_with('T')))
        };
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
        // Killed, not left stopped and checked out. The sandbox's inner
        // process goes by its parent-death signal, a beat after the outer.
        let gone = |pid: &u32| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            stat.is_empty() || stat.contains(") Z ")
        };
        for _ in 0..50 {
            if tree.iter().all(gone) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        for pid in tree {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            assert!(
                stat.is_empty() || stat.contains(") Z "),
                "process {pid} outlived its hand-back: {stat}"
            );
        }
    }

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

    /// The worker's argv binds its binary and nothing else, unshares the
    /// network with everything, and clears the environment.
    #[test]
    fn the_worker_sees_its_binary_and_nothing_else() {
        let argv = Confinement::bwrap_at("/usr/bin/bwrap").argv(Path::new("/opt/m/monty"));
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
        let l = Launcher::write(&["/x/b'w rap".into(), "--ro-bind".into()]).unwrap();
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
