//! The interpreter behind code mode (`docs/CODE-MODE-DESIGN.md` §7): a
//! program runs until it calls an external function the host registered,
//! the host answers with a value or by raising, and the program runs on —
//! or it ends, with what it printed and how it ended.
//!
//! **This is a trait so the backend is swappable.** `CODE-MODE-RESEARCH.md`
//! asked for that from day one: monty is young, was escaped once, and the
//! runner-up (a WASM guest) must stay reachable. Nothing here names monty;
//! [`monty`] (cargo feature `monty`) is the one implementation.
//!
//! **What the trait promises, whichever backend is behind it:**
//! - A program reaches the host only through the names it was started with.
//!   A call to any other name never surfaces as a [`Step::Call`] — the
//!   backend refuses it inside the program.
//! - No filesystem, environment, network, clock-driven sleep or import
//!   reaches the host: those requests are refused inside the program.
//! - The host can raise only a built-in exception type with one message
//!   ([`HostRaise`]). Monty has no other kind (CODE-MODE-DESIGN.md §1), so
//!   the host's failure classes are a closed set of message tags ([`Tag`]),
//!   and the text after a tag must carry no bytes from a tool (§3).
//! - **Backstops end a program from outside and cannot be caught**
//!   ([`Backstop`]). The program's own exceptions, and anything the host
//!   raised that it did not catch, end it as [`End::Raised`]. The two are
//!   kept apart in the type, so a caller cannot render a limit as the
//!   program's own failure, or the reverse.
//! - A cancellation token stops a program at its next suspension, and a
//!   program that does not reach one within [`Backstops::cancel_grace`] is
//!   killed. Either way it ends [`End::Cancelled`] with what it printed.
//!
//! `dump`/`load` are reserved for the sink slice (§12) and refuse here.

use anyhow::Result;
use async_trait::async_trait;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "monty")]
pub mod monty;
pub mod worker;

/// Whether this binary has an interpreter at all: the cargo feature `monty`,
/// off by default while `monty-pool` needs a newer Rust than the workspace
/// promises. A build without it still knows code mode, and says by name
/// that it cannot run it (`feature::unbuilt`).
pub const BUILT: bool = cfg!(feature = "monty");

/// Workers one run's interpreter may hold at once: two `code` calls in one
/// message, and a long-lived surface's concurrent chats, each hold one per
/// program (one program per worker); a fifth program waits for a free one,
/// as a Stop can end that wait.
pub const RUN_WORKERS: usize = 4;

/// The interpreter a run's `code` serves its programs with: the pinned
/// worker installed under `mecha_home`, confined by the platform
/// ([`monty::Confinement::for_sandbox`] over `sandbox`), and proven by
/// [`Interpreter::preflight`] before it is handed out. **An `Err` is for the
/// caller to stop the start with**: code mode switched on with a worker or a
/// sandbox that cannot start must never become a run without `code`.
///
/// One per built agent, shared by every run it serves (`Agent::enable_code`):
/// a worker is never reused across programs, so a pool per process would
/// share nothing but its launcher, and an agent rebuilt for a config change
/// takes the new `[sandbox]` with it. The host bytes every program holds are
/// still one process-wide pool (`code::BytePool::process`, §5).
///
/// On a Mac, `[sandbox] memory_mb` and `cpus` never reach the worker: a
/// Seatbelt profile has no cgroup to carry them, and monty's own allocator
/// ceiling is the only memory bound there (`ARCHITECTURE.md` §Code mode's
/// interpreter). That is warned once per process, here, rather than
/// refused: `[sandbox]` is `shell`'s table, where the limits do hold, and
/// refusing code mode for a setting it never claimed would be the wrong
/// table's answer (review of #769).
pub async fn for_runs(
    sandbox: &crate::sandbox::SandboxConfig,
    mecha_home: &std::path::Path,
) -> Result<std::sync::Arc<dyn Interpreter>> {
    if let Some(gap) = limits_not_applied(sandbox, cfg!(target_os = "macos")) {
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| tracing::warn!("{gap}"));
    }
    start(sandbox, mecha_home).await
}

/// The Mac gap [`for_runs`] warns about, as a sentence, or `None` where
/// there is none: `[sandbox] memory_mb` or `cpus` set on a platform whose
/// worker confinement cannot apply them.
pub fn limits_not_applied(sandbox: &crate::sandbox::SandboxConfig, macos: bool) -> Option<String> {
    let mut set = Vec::new();
    if let Some(mb) = sandbox.memory_mb {
        set.push(format!("memory_mb = {mb}"));
    }
    if let Some(cpus) = sandbox.cpus {
        set.push(format!("cpus = {cpus}"));
    }
    (macos && !set.is_empty()).then(|| {
        format!(
            "code mode: [sandbox] {} does not reach the interpreter's worker on a Mac —              Seatbelt has no cgroup to apply it, so monty's own memory ceiling is the only              bound on a program there, and nothing limits its CPU (`shell` still gets the              limits under its own sandbox)",
            set.join(", ")
        )
    })
}

#[cfg(feature = "monty")]
async fn start(
    sandbox: &crate::sandbox::SandboxConfig,
    mecha_home: &std::path::Path,
) -> Result<std::sync::Arc<dyn Interpreter>> {
    use anyhow::Context as _;
    let confinement =
        monty::Confinement::for_sandbox(&crate::sandbox::Sandbox::new(sandbox.clone()))
            .context("code mode's interpreter has no sandbox to run in")?;
    let worker = worker::installed(mecha_home)
        .context("code mode's worker — `mecha features enable code` installs it")?;
    let interpreter =
        monty::Monty::new(&worker, &confinement, Backstops::default(), RUN_WORKERS).await?;
    interpreter
        .preflight()
        .await
        .context("code mode's interpreter failed its preflight")?;
    Ok(std::sync::Arc::new(interpreter))
}

#[cfg(not(feature = "monty"))]
async fn start(
    _sandbox: &crate::sandbox::SandboxConfig,
    _mecha_home: &std::path::Path,
) -> Result<std::sync::Arc<dyn Interpreter>> {
    anyhow::bail!(
        "this mecha was built without code mode's interpreter (the cargo feature `monty`)"
    )
}

/// A value crossing between the program and the host. JSON's shapes,
/// because tool arguments and results already are; a program value with no
/// JSON form (a set, a bytes, a class instance) does not cross as an
/// argument at all — the backend raises `TypeError` in the program instead.
pub use serde_json::Value;

/// A call the program made to a registered external function.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalCall {
    pub name: String,
    pub args: Vec<Value>,
    pub kwargs: Vec<(String, Value)>,
}

/// Where a program is: waiting on the host, or over.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Answer with [`Program::resume`] or [`Program::raise`].
    Call(ExternalCall),
    Done(Finished),
}

/// A program that is over, and everything it printed on the way.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    /// Printed output, both streams in order, capped at
    /// [`Backstops::max_printed_bytes`]; bytes past the cap are counted in
    /// `printed_dropped`, never silently lost.
    pub printed: String,
    pub printed_dropped: u64,
    pub end: End,
}

/// How a program ended.
#[derive(Debug, Clone, PartialEq)]
pub enum End {
    /// It ran to the end. `value` is the final expression's value when it
    /// has a JSON form; `repr` is its Python `repr()` either way.
    Complete { value: Option<Value>, repr: String },
    /// An exception nothing in the program caught.
    Raised(Raised),
    /// A backstop ended it, uncatchably.
    Backstop(Backstop),
    /// The run's cancellation token fired.
    Cancelled,
}

/// An uncaught exception: its built-in type's name, its message, and the
/// traceback as Python would print it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raised {
    pub exc_type: String,
    pub message: Option<String>,
    pub traceback: String,
}

/// The limits that end a program from outside (CODE-MODE-DESIGN.md §5).
/// Each names itself, because the line naming it is the model's only
/// feedback; each carries the limit that was crossed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backstop {
    /// The program's wall clock, counted from its start and including the
    /// time the host spent answering its calls.
    WallClock(Duration),
    /// The interpreter's heap, in bytes.
    Memory(usize),
    /// The number of suspensions (calls, refused requests) one program may make.
    Suspensions(u64),
    /// The worker died under the program — killed by the sandbox (a cgroup
    /// memory limit from `[sandbox] memory_mb` is the usual cause) or
    /// crashed. A program can bring this about, so it is an ending, not a
    /// backend failure; `signal` is the one that killed it, when known.
    WorkerKilled { signal: Option<i32> },
}

impl std::fmt::Display for Backstop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Backstop::WallClock(d) => write!(
                f,
                "stopped by the wall-clock backstop: the program ran past {} s",
                d.as_secs()
            ),
            Backstop::Memory(b) => write!(
                f,
                "stopped by the memory backstop: the program's heap passed {b} bytes"
            ),
            Backstop::Suspensions(n) => write!(
                f,
                "stopped by the suspension backstop: the program suspended more than {n} times (host calls and refused requests)"
            ),
            Backstop::WorkerKilled { signal } => match signal {
                Some(s) => write!(
                    f,
                    "stopped: the interpreter was killed by signal {s} (the sandbox's memory limit, or something outside the program)"
                ),
                None => f.write_str("stopped: the interpreter died under the program"),
            },
        }
    }
}

/// The backstop settings one interpreter applies to every program it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backstops {
    /// §5's wall clock: 600 s, sized against the contended worst case of the
    /// sub-call token budget, not against a well-behaved program.
    pub wall_clock: Duration,
    /// The interpreter's heap. The handles' bytes live in the host, not
    /// here (§5), so this bounds the program's own data.
    pub max_memory: usize,
    /// Call-stack depth. Monty raises a *catchable* `RecursionError` here,
    /// as CPython does, so this one bounds the stack rather than ending the
    /// program; an uncaught one ends it as [`End::Raised`].
    pub max_recursion_depth: usize,
    /// Suspensions per program. §5 sets it to the sum of the counted host
    /// limits, so it binds only if the host's own counting fails.
    pub max_suspensions: u64,
    /// How long a cancelled program has to reach its next suspension before
    /// its worker is killed.
    pub cancel_grace: Duration,
    /// Printed output kept in the host per program; the rest is counted.
    /// A host-memory bound, not a backstop: the program keeps running.
    pub max_printed_bytes: usize,
}

/// §5's counted host limits, as R4 ruled them: 32 sub-calls, 256 tool
/// reads and 2,048 handle operations. Thinking passes are a subset of
/// sub-calls and add nothing; an `llm_query` or `llm_map` call is one
/// suspension and spends a handle operation, so the sum still bounds it.
pub const COUNTED_HOST_LIMITS: u64 = 32 + 256 + 2048;

impl Default for Backstops {
    fn default() -> Self {
        Backstops {
            wall_clock: Duration::from_secs(600),
            max_memory: 256 * 1024 * 1024,
            // monty's own default (`DEFAULT_MAX_RECURSION_DEPTH`).
            max_recursion_depth: 1000,
            max_suspensions: COUNTED_HOST_LIMITS,
            cancel_grace: Duration::from_secs(2),
            max_printed_bytes: 1024 * 1024,
        }
    }
}

/// The built-in exception types the host may raise into a program. Closed
/// on purpose: a program cannot define exception classes, so these and a
/// message are all a host failure can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    RuntimeError,
    ValueError,
    TypeError,
    KeyError,
    IndexError,
    PermissionError,
}

/// The fixed message prefixes the code tool's host failures open with
/// (CODE-MODE-DESIGN.md §4). The tag is the only thing a program can tell
/// host causes apart by; an untagged exception is the interpreter's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    QueryError,
    ToolError,
    HandleError,
    LimitError,
}

impl Tag {
    pub fn prefix(self) -> &'static str {
        match self {
            Tag::QueryError => "QueryError:",
            Tag::ToolError => "ToolError:",
            Tag::HandleError => "HandleError:",
            Tag::LimitError => "LimitError:",
        }
    }
}

/// What the host raises into a program in answer to a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRaise {
    pub exc: Builtin,
    pub message: String,
}

impl HostRaise {
    /// A `RuntimeError` opening with `tag` — every host failure's form (§4).
    pub fn tagged(tag: Tag, message: &str) -> Self {
        HostRaise {
            exc: Builtin::RuntimeError,
            message: format!("{} {message}", tag.prefix()),
        }
    }
}

/// A backend that runs programs.
#[async_trait]
pub trait Interpreter: Send + Sync {
    /// The backend's name, for messages and the session record.
    fn name(&self) -> &'static str;

    /// Prove a program runs end to end under this backend's confinement.
    /// A backend whose confinement cannot hold refuses here, at start, and
    /// never degrades to running unconfined.
    async fn preflight(&self) -> Result<()>;

    /// Start `source` with `externals` as the only host functions it can
    /// call, and run it to its first call or its end. `cancel` stops it.
    /// `Err` is for the backend failing (a worker that will not start, a
    /// crash, a protocol violation) — never for anything the program did.
    async fn start(
        &self,
        source: &str,
        externals: &[String],
        cancel: CancellationToken,
    ) -> Result<(Box<dyn Program>, Step)>;

    /// Reserved: restore a program [`Program::dump`] saved (the sink slice).
    async fn load(&self, _snapshot: &[u8]) -> Result<(Box<dyn Program>, Step)> {
        anyhow::bail!(
            "restoring a program is reserved for the sink slice (CODE-MODE-DESIGN.md §12)"
        )
    }
}

/// One running program, suspended at an external call.
#[async_trait]
pub trait Program: Send {
    /// Answer the pending call with a value.
    async fn resume(&mut self, value: Value) -> Result<Step>;

    /// Answer the pending call by raising in the program, where the call is.
    async fn raise(&mut self, raise: HostRaise) -> Result<Step>;

    /// The OS process the program runs in, when a backend has one —
    /// diagnostics and the confinement tests, never a control channel.
    fn worker_pid(&self) -> Option<u32> {
        None
    }

    /// Reserved: this program's state as bytes (the sink slice). A snapshot
    /// is unauthenticated, so storing one will need a MAC (§1).
    async fn dump(&mut self) -> Result<Vec<u8>> {
        anyhow::bail!("saving a program is reserved for the sink slice (CODE-MODE-DESIGN.md §12)")
    }
}

/// Whether `name` may be registered as an external function: a plain
/// identifier, never a dunder (which would reach the interpreter's own
/// machinery) — checked here so every backend refuses the same names.
pub fn registrable(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        && !name.starts_with("__")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_failures_open_with_their_tag_and_raise_a_runtime_error() {
        let r = HostRaise::tagged(Tag::LimitError, "sub-calls: 32 of 32 spent");
        assert_eq!(r.exc, Builtin::RuntimeError);
        assert_eq!(r.message, "LimitError: sub-calls: 32 of 32 spent");
        for t in [
            Tag::QueryError,
            Tag::ToolError,
            Tag::HandleError,
            Tag::LimitError,
        ] {
            assert!(HostRaise::tagged(t, "x").message.starts_with(t.prefix()));
        }
    }

    #[test]
    fn only_plain_identifiers_register() {
        for ok in ["mail_search", "kg_search", "_private", "f2"] {
            assert!(registrable(ok), "{ok}");
        }
        for bad in [
            "",
            "2f",
            "a-b",
            "a.b",
            "__import__",
            "__builtins__",
            "é",
            "a b",
        ] {
            assert!(!registrable(bad), "{bad}");
        }
    }

    #[test]
    fn each_backstop_names_itself_and_its_limit() {
        let lines = [
            Backstop::WallClock(Duration::from_secs(600)).to_string(),
            Backstop::Memory(1024).to_string(),
            Backstop::Suspensions(2336).to_string(),
        ];
        assert!(lines[0].contains("wall-clock") && lines[0].contains("600 s"));
        assert!(lines[1].contains("memory") && lines[1].contains("1024"));
        assert!(lines[2].contains("suspension") && lines[2].contains("2336"));
        let killed = Backstop::WorkerKilled { signal: Some(9) }.to_string();
        assert!(killed.contains("killed by signal 9"), "{killed}");
        let lost = Backstop::WorkerKilled { signal: None }.to_string();
        assert!(lost.contains("died under the program"), "{lost}");
    }

    #[test]
    fn the_default_backstops_are_the_designs() {
        let b = Backstops::default();
        assert_eq!(b.wall_clock, Duration::from_secs(600));
        assert_eq!(b.max_suspensions, 2336);
    }
}
