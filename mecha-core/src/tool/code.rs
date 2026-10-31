//! The `code` tool (`docs/CODE-MODE-DESIGN.md`, slice 3b): a short Python
//! program run in the interpreter ([`crate::interpreter`]), whose calls to
//! tools come back through the run's own gate, and whose results stay in the
//! host as opaque handles.
//!
//! The invariants, each a section of the design:
//! - **Admission is per run** (§2). A tool is a program function only if it
//!   is in the registry of the run the tool was built for, its effect class
//!   is `Read` or `ReadThatRecords`, its egress is `None`, it is not
//!   `destructive` and it is `read_only()` ([`Admission`]). Everything else
//!   is not registered in the program at all. At every program's start the
//!   dispatch serving it recomputes the set over *its* registry and refuses a
//!   `code` built over another one — the parent's, cloned into a child, or
//!   one built before the last registration.
//! - **Every host call takes the gate** (§7), `Agent::gate`, in the mode that
//!   refuses rather than asks: a call that would put a question to a person
//!   (a path outside the jail, a `prompt` rule) raises `ToolError:` in the
//!   program and prompts nobody. Taint is armed from the raw result before
//!   the program resumes, and each host call pushes a trace entry tagged
//!   `in_program`.
//! - **Results are handles** (§3): the bytes stay here, and the program holds
//!   a stub (`<handle 3: 48,213 chars from mail_get_thread>`), which is also
//!   what `print` shows. Text reaches the program only through `peek`,
//!   capped and counted. Patterns run here, with Rust's linear-time `regex`.
//! - **Host messages carry no bytes** (§3, §4): every raise names a handle,
//!   an offset, a tool or a limit, under one of `ToolError:`, `HandleError:`
//!   and `LimitError:`. A tool's error body is a handle like any result.
//! - **Host limits raise, catchably** (§5), each naming itself
//!   ([`HostLimits`], R4's defaults owed in [`defaults`]); the backstops are
//!   the interpreter's own and end the program uncatchably.
//! - **The result is rendered once** (§7): what the program printed, then
//!   its ending, as this call's own `tool_result`, which the dispatch caps,
//!   spills and wraps like any other. It is `external` when any host call
//!   returned external content.
//!
//! `code` declares the union of its admitted reads' capabilities with egress
//! `None` (§6), which is what `dispatch`'s precomputed turn taint reads
//! before a program runs.
//!
//! Not in this slice: `llm_query`/`llm_map` (3c), the `program` record, its
//! blobs and the grounding walk's second source (3d), and registration (3d).

use super::effect::{self, Effect, EffectClass};
use super::{Capabilities, Egress, Registry, Tool, ToolCtx, ToolOutput};
use crate::interpreter::{
    registrable, End, ExternalCall, Finished, HostRaise, Interpreter, Step, Tag, Value,
};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::{mpsc, oneshot};

/// The tool's registered name.
pub const NAME: &str = "code";

/// §5's host limits, at the defaults it proposes. **R4 owes these numbers**
/// (`CODE-MODE-DESIGN.md` §11, waiting on M1's reading of this box): they
/// live here, in one place, until it is ruled, and
/// [`crate::interpreter::COUNTED_HOST_LIMITS`] sizes the suspension backstop
/// from the same counts (a test holds the two together).
pub mod defaults {
    /// Tool reads per program.
    pub const TOOL_READS: u32 = 256;
    /// Handle operations per program, `budget()` included.
    pub const HANDLE_OPERATIONS: u32 = 2048;
    /// Bytes the program's handles may hold: every tool read's raw result,
    /// and every new string a `json_get` leaf decodes.
    pub const HOST_BYTES: usize = 64 * 1024 * 1024;
    /// The same, for every program in the process at once.
    pub const PROCESS_HOST_BYTES: usize = 256 * 1024 * 1024;
    /// Handles one program may hold.
    pub const LIVE_HANDLES: usize = 4096;
    /// Text `peek` may hand the program, in bytes (one count with 3c's
    /// `str[n]` results, `peeked_bytes`).
    pub const PEEK_BYTES: usize = 16 * 1024;
    /// `peek`'s `n` when the program names none.
    pub const PEEK_DEFAULT: usize = 2000;
    /// The least `n` `chunks` accepts, so one counted operation cannot mint
    /// a handle per character.
    pub const CHUNK_FLOOR: usize = 256;
    /// Not a §5 row: the most `(start, end)` pairs one `find` returns. A
    /// pattern matching every character of a 64 MB handle would otherwise
    /// build a value of billions of pairs in the host before any limit saw
    /// it, so a sweep that wants more narrows the pattern or slices first.
    pub const FIND_MATCHES: usize = 4096;
}

/// The per-program host limits (§5), each raising a catchable `LimitError:`
/// that names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostLimits {
    pub tool_reads: u32,
    pub handle_operations: u32,
    pub host_bytes: usize,
    pub live_handles: usize,
    pub peek_bytes: usize,
}

impl Default for HostLimits {
    fn default() -> Self {
        HostLimits {
            tool_reads: defaults::TOOL_READS,
            handle_operations: defaults::HANDLE_OPERATIONS,
            host_bytes: defaults::HOST_BYTES,
            live_handles: defaults::LIVE_HANDLES,
            peek_bytes: defaults::PEEK_BYTES,
        }
    }
}

/// The bytes every program in the process holds in handles, against one
/// ceiling (§5): two `code` calls in one turn, a parent's beside a child's,
/// and `mecha batch` all run at once, and per-program limits alone would
/// not bound N × 64 MB. Never shown to a program: it moves with other
/// programs, and reading it would be a cross-program observable.
#[derive(Debug)]
pub struct BytePool {
    max: usize,
    used: AtomicUsize,
}

impl BytePool {
    pub fn new(max: usize) -> Arc<Self> {
        Arc::new(BytePool {
            max,
            used: AtomicUsize::new(0),
        })
    }

    /// The one pool every `code` tool in this process draws from unless it
    /// was given its own.
    pub fn process() -> Arc<Self> {
        static POOL: OnceLock<Arc<BytePool>> = OnceLock::new();
        Arc::clone(POOL.get_or_init(|| BytePool::new(defaults::PROCESS_HOST_BYTES)))
    }

    fn take(&self, n: usize) -> bool {
        let mut used = self.used.load(Ordering::Acquire);
        loop {
            let Some(total) = used.checked_add(n).filter(|t| *t <= self.max) else {
                return false;
            };
            match self
                .used
                .compare_exchange_weak(used, total, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return true,
                Err(now) => used = now,
            }
        }
    }

    fn give(&self, n: usize) {
        self.used.fetch_sub(n, Ordering::AcqRel);
    }

    /// Bytes held right now, for tests and the doctor; never a program's.
    pub fn in_use(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
}

/// The host functions every program has (§3), beside its admitted tools.
/// A tool whose name collides with one of these is not admitted.
pub const HOST_FUNCTIONS: [&str; 10] = [
    "budget", "chunks", "count", "find", "json_get", "lines", "peek", "size", "slice", "split",
];

/// Which tools a program may call, by name, with what each declares (§2):
/// computed over one registry, so two runs' sets can be compared.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Admission {
    tools: BTreeMap<String, (Capabilities, EffectClass)>,
}

impl Admission {
    /// The admitted set of `registry`, with `routed` the operator's outbox
    /// routing for the run (`[outbox] tools`): a routed read resolves
    /// `Staged` and is not admitted, so nothing in a program can stage.
    pub fn over(registry: &Registry, routed: impl Fn(&str) -> bool) -> Self {
        let tools = registry
            .iter()
            .filter_map(|t| {
                let class = effect::class_of(t.as_ref(), routed(t.name()));
                Self::admits(t.as_ref(), class)
                    .then(|| (t.name().to_string(), (t.capabilities(), class)))
            })
            .collect();
        Admission { tools }
    }

    /// §2's rule for one tool whose effect class is `class`: a plain read
    /// (`Read`, `ReadThatRecords`), egress `None`, not `destructive`, and
    /// `read_only()`. The `destructive` term is what keeps `request_access`
    /// out; the class is what keeps `ask_user`, `message_send` and
    /// `image_generate` out, all three of which are `read_only()`. Two terms
    /// are mechanical: a tool that itself runs programs is never one — asked
    /// by `hosts_programs()` *and* by name, so a wrapper that forgot to
    /// forward the method still cannot admit `code` into its own program
    /// (review of #768) — and a name a program cannot hold as a global
    /// (`interpreter::registrable`) or that a host function already takes
    /// is left out rather than renamed.
    pub fn admits(tool: &dyn Tool, class: EffectClass) -> bool {
        let caps = tool.capabilities();
        matches!(class, EffectClass::Read | EffectClass::ReadThatRecords)
            && caps.egress == Egress::None
            && !caps.destructive
            && tool.read_only()
            && !tool.hosts_programs()
            && tool.name() != NAME
            && registrable(tool.name())
            && !HOST_FUNCTIONS.contains(&tool.name())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(String::as_str)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// What `code` declares (§6): the union of its admitted reads, and egress
    /// `None`, because no sender is admitted.
    pub fn capabilities(&self) -> Capabilities {
        let mut caps = self
            .tools
            .values()
            .fold(Capabilities::default(), |acc, (c, _)| acc.union(*c));
        caps.egress = Egress::None;
        caps.destructive = false;
        caps
    }

    /// `ReadThatRecords` if any admitted read records, else `Read`.
    fn effect(&self) -> Effect {
        if self
            .tools
            .values()
            .any(|(_, class)| *class == EffectClass::ReadThatRecords)
        {
            Effect::ReadThatRecords
        } else {
            Effect::Read
        }
    }
}

/// The channel a running program's host calls take back to the dispatch
/// running its `code` call (§7): stamped on that call's [`ToolCtx`] by
/// `Agent::dispatch` and on no other. Only the loop can make one.
#[derive(Clone)]
pub struct HostCalls {
    tx: mpsc::UnboundedSender<HostRequest>,
    call_id: String,
    result_cap: usize,
}

impl std::fmt::Debug for HostCalls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostCalls")
            .field("call_id", &self.call_id)
            .field("result_cap", &self.result_cap)
            .finish_non_exhaustive()
    }
}

/// What a program asks of the dispatch running it.
pub(crate) enum HostRequest {
    /// At the program's start: the names of `admission` the run may reach
    /// right now, or a refusal when the dispatch's own registry admits a
    /// different set.
    Reach {
        admission: Admission,
        reply: oneshot::Sender<std::result::Result<Vec<String>, String>>,
    },
    /// One host call, through the gate.
    Call {
        call_id: String,
        name: String,
        input: Value,
        reply: oneshot::Sender<HostAnswer>,
    },
}

/// What the gate made of one host call.
pub(crate) enum HostAnswer {
    /// Settled at the gate, in its own words (`Blocked by policy:`, a hook's
    /// refusal, an unknown or withheld name).
    Refused(String),
    /// Staged in the outbox. An admitted tool is never routed, so this is a
    /// refusal in the program too.
    Staged(String),
    /// Ran: the raw output, uncapped and unwrapped, with the conversation's
    /// taint already armed from it.
    Ran(ToolOutput),
}

impl HostCalls {
    /// The channel for the call `call_id`, whose result is rendered at
    /// `result_cap` bytes: its share of the turn's output budget.
    pub(crate) fn new(
        tx: mpsc::UnboundedSender<HostRequest>,
        call_id: &str,
        result_cap: usize,
    ) -> Self {
        HostCalls {
            tx,
            call_id: call_id.to_string(),
            result_cap,
        }
    }

    /// This call's share of the turn's output budget, which the dispatch
    /// caps its result at.
    pub fn result_cap(&self) -> usize {
        self.result_cap
    }

    async fn reach(&self, admission: &Admission) -> std::result::Result<Vec<String>, String> {
        let (reply, answer) = oneshot::channel();
        let gone = || "the run that started this program has ended".to_string();
        self.tx
            .send(HostRequest::Reach {
                admission: admission.clone(),
                reply,
            })
            .map_err(|_| gone())?;
        answer.await.map_err(|_| gone())?
    }

    async fn call(&self, name: &str, input: Value) -> HostAnswer {
        let (reply, answer) = oneshot::channel();
        let gone = || HostAnswer::Refused("the run that started this program has ended".into());
        if self
            .tx
            .send(HostRequest::Call {
                call_id: self.call_id.clone(),
                name: name.to_string(),
                input,
                reply,
            })
            .is_err()
        {
            return gone();
        }
        answer.await.unwrap_or_else(|_| gone())
    }
}

/// The `code` tool. Built over one run's registry ([`CodeTool::over`]), and
/// deliberately not `Clone`: a subagent's `code` is built afresh over the
/// child's registry, never a clone of the parent's (§6), and a clone that
/// reached a child anyway is refused at its first program.
pub struct CodeTool {
    admission: Admission,
    interpreter: Arc<dyn Interpreter>,
    limits: HostLimits,
    pool: Arc<BytePool>,
    description: String,
}

impl CodeTool {
    /// `code` over `registry`, the registry of the run it will serve, as it
    /// stands when this is called. **Call it after the last registration**:
    /// for the parent that is after `setup.rs` registers the front end's own
    /// tools (`ask_user`, `recall`, the TUI's) past `prepare`, so `recall`
    /// is admitted and the declared union is over the whole set. For a
    /// subagent it is the child's registry, after every other tool is in.
    /// Either mistake — too early, or the wrong registry — is refused at the
    /// program's start, when the serving dispatch's own registry admits a
    /// different set.
    ///
    /// `routed` is the run's outbox routing; `interpreter` must already have
    /// passed its `preflight` (`ARCHITECTURE.md` §Code mode's interpreter:
    /// nothing in the trait enforces it).
    pub fn over(
        registry: &Registry,
        routed: impl Fn(&str) -> bool,
        interpreter: Arc<dyn Interpreter>,
    ) -> Self {
        let admission = Admission::over(registry, routed);
        let description = describe(&admission);
        CodeTool {
            admission,
            interpreter,
            limits: HostLimits::default(),
            pool: BytePool::process(),
            description,
        }
    }

    pub fn with_limits(mut self, limits: HostLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Draw on `pool` rather than the process's: for a test.
    pub fn with_pool(mut self, pool: Arc<BytePool>) -> Self {
        self.pool = pool;
        self
    }

    pub fn admission(&self) -> &Admission {
        &self.admission
    }

    async fn run(&self, program: &str, host: &HostCalls, ctx: &ToolCtx) -> ToolOutput {
        let reach = match host.reach(&self.admission).await {
            Ok(r) => r,
            Err(why) => return ToolOutput::refusal(why),
        };
        let mut externals: Vec<String> = HOST_FUNCTIONS.iter().map(|s| s.to_string()).collect();
        externals.extend(reach.iter().cloned());
        let cancel = ctx.cancel.clone().unwrap_or_default();
        let mut state = Run::new(self.limits, Arc::clone(&self.pool)).reaching(reach);
        let (mut program, mut step) = match self
            .interpreter
            .start(program, &externals, cancel.clone())
            .await
        {
            Ok(started) => started,
            Err(e) => {
                return ToolOutput::err(format!("the interpreter could not run the program: {e:#}"))
            }
        };
        let finished = loop {
            let call = match step {
                Step::Done(f) => break f,
                Step::Call(call) => call,
            };
            let answer = if HOST_FUNCTIONS.contains(&call.name.as_str()) {
                state.op(&call)
            } else {
                state.read(host, &call, &cancel).await
            };
            let next = match answer {
                Ok(v) => program.resume(v).await,
                Err(raise) => program.raise(raise).await,
            };
            step = match next {
                Ok(s) => s,
                Err(e) => {
                    let mut out =
                        ToolOutput::err(format!("the interpreter failed mid-program: {e:#}"));
                    out.external = state.external;
                    return out;
                }
            };
        };
        let (content, is_error) = render(finished, host.result_cap);
        ToolOutput {
            content,
            is_error,
            external: state.external,
            ..ToolOutput::ok("")
        }
    }
}

/// The tool's description: what a program can do, and the tools it may call
/// as built. Fixed at construction, so it never changes a cached prefix
/// between turns.
fn describe(admission: &Admission) -> String {
    let mut d = String::from(
        "Run a short Python program (monty, a subset of Python 3) over tool results \
         without reading them into your context. A tool called from the program returns a \
         handle, not text: print(h) shows only a stub. Work on handles with size(h), \
         slice(h, start, end), lines(h), split(h, pattern), chunks(h, n, overlap=0) (n \
         characters each, at least 256), find(h, pattern) (a list of [start, end] character \
         offsets), count(h, pattern) and json_get(h, path) (a string, object or array leaf \
         stays a handle; a number, bool or null comes back as itself). peek(h, n=2000) is \
         the only way to see a handle's text, and is capped per program; budget() says what \
         is left of each limit. Patterns are Rust regular expressions. Call a tool with \
         keyword arguments, as in fs_read(path=\"notes.txt\"). A failure raises \
         RuntimeError whose message starts ToolError:, HandleError: or LimitError:. The \
         result is what the program printed, then its final expression.",
    );
    if admission.is_empty() {
        d.push_str(" No tool may be called from a program in this run.");
    } else {
        d.push_str(" Tools a program may call: ");
        d.push_str(&admission.names().collect::<Vec<_>>().join(", "));
        d.push('.');
    }
    d
}

#[async_trait]
impl Tool for CodeTool {
    fn name(&self) -> &str {
        NAME
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "program": {
                    "type": "string",
                    "description": "The Python source to run."
                }
            },
            "required": ["program"]
        })
    }

    /// A program runs without a prompt, because every host call meets the
    /// whole gate, policy included, and refuses where it would ask (§11).
    /// The flag flips when a sink is admitted (§12).
    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Capabilities {
        self.admission.capabilities()
    }

    fn effect(&self) -> Option<Effect> {
        Some(self.admission.effect())
    }

    fn hosts_programs(&self) -> bool {
        true
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(program) = input.get("program").and_then(Value::as_str) else {
            return Ok(ToolOutput::err(
                "`program` is required: the Python source to run",
            ));
        };
        let Some(host) = ctx.host_calls.as_ref() else {
            return Ok(ToolOutput::refusal(
                "`code` runs a program only inside a run's dispatch, where each of its calls \
                 is gated; this call came by another path, so nothing ran",
            ));
        };
        Ok(self.run(program, host, ctx).await)
    }
}

/// A host failure's raise: a `RuntimeError` opening with its tag, carrying
/// no bytes from any handle or tool (§3).
fn raise(tag: Tag, message: impl AsRef<str>) -> HostRaise {
    HostRaise::tagged(tag, message.as_ref())
}

/// One buffer of bytes a handle points into: a tool's raw result, or a
/// string a `json_get` decoded.
struct Buffer {
    text: Box<str>,
    ascii: bool,
    tool: String,
}

/// A view of a buffer: byte offsets into it, and its length in characters.
#[derive(Clone)]
struct Handle {
    buf: Arc<Buffer>,
    start: usize,
    end: usize,
    chars: usize,
}

impl Handle {
    fn text(&self) -> &str {
        &self.buf.text[self.start..self.end]
    }

    /// Byte offsets, relative to this handle, of the character offsets
    /// `chars` (ascending, each at most `self.chars`).
    fn bytes_at(&self, chars: &[usize]) -> Vec<usize> {
        if self.buf.ascii {
            return chars.to_vec();
        }
        let text = self.text();
        let mut it = text.char_indices();
        let (mut ci, mut bi) = (0usize, 0usize);
        chars
            .iter()
            .map(|&c| {
                while ci < c {
                    match it.next() {
                        Some((b, ch)) => {
                            bi = b + ch.len_utf8();
                            ci += 1;
                        }
                        None => break,
                    }
                }
                bi
            })
            .collect()
    }

    /// Character offsets of the byte offsets `bytes` (ascending, each on a
    /// character boundary of this handle's text).
    fn chars_at(&self, bytes: &[usize]) -> Vec<usize> {
        if self.buf.ascii {
            return bytes.to_vec();
        }
        let text = self.text();
        let (mut lb, mut lc) = (0usize, 0usize);
        bytes
            .iter()
            .map(|&b| {
                lc += text[lb..b].chars().count();
                lb = b;
                lc
            })
            .collect()
    }

    /// The sub-view `[from, to)` in bytes relative to this handle.
    fn sub(&self, from: usize, to: usize) -> Handle {
        let text = &self.text()[from..to];
        Handle {
            buf: Arc::clone(&self.buf),
            start: self.start + from,
            end: self.start + to,
            chars: if self.buf.ascii {
                to - from
            } else {
                text.chars().count()
            },
        }
    }
}

/// `n` with thousands separators, as a stub prints it.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(d);
    }
    out
}

fn stub(id: u64, h: &Handle) -> String {
    format!(
        "<handle {id}: {} chars from {}>",
        grouped(h.chars),
        h.buf.tool
    )
}

/// One program's handles and counts.
struct Run {
    limits: HostLimits,
    pool: Arc<BytePool>,
    handles: BTreeMap<u64, Handle>,
    next: u64,
    reads: u32,
    ops: u32,
    /// Bytes this program's buffers hold, all drawn from `pool` too.
    bytes: usize,
    peeked: usize,
    /// Any host call returned external content.
    external: bool,
    /// The tools this program may call, as its start resolved them: checked
    /// again here on every read, so §2 does not rest on the interpreter
    /// alone refusing an unregistered name (review of #768). `ask_user` and
    /// `message_send` would clear the gate if one reached it.
    reach: std::collections::BTreeSet<String>,
}

impl Drop for Run {
    fn drop(&mut self) {
        self.pool.give(self.bytes);
    }
}

/// A call's arguments, by position or by keyword.
struct Args<'a> {
    call: &'a ExternalCall,
    f: &'static str,
}

impl<'a> Args<'a> {
    fn get(&self, i: usize, name: &str) -> Option<&'a Value> {
        self.call.args.get(i).or_else(|| {
            self.call
                .kwargs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v)
        })
    }

    fn int(&self, i: usize, name: &str, default: Option<i64>) -> Result<i64, HostRaise> {
        match self.get(i, name) {
            None | Some(Value::Null) => default.ok_or_else(|| {
                raise(
                    Tag::HandleError,
                    format!("`{}`: `{name}` is required", self.f),
                )
            }),
            Some(v) => v.as_i64().ok_or_else(|| {
                raise(
                    Tag::HandleError,
                    format!("`{}`: `{name}` must be an integer", self.f),
                )
            }),
        }
    }

    fn pattern(&self, i: usize) -> Result<regex::Regex, HostRaise> {
        let Some(p) = self.get(i, "pattern").and_then(Value::as_str) else {
            return Err(raise(
                Tag::HandleError,
                format!("`{}`: `pattern` must be a string", self.f),
            ));
        };
        regex::RegexBuilder::new(p)
            .size_limit(1 << 20)
            .dfa_size_limit(1 << 22)
            .build()
            .map_err(|_| {
                raise(
                    Tag::HandleError,
                    format!(
                        "`{}`: the pattern does not compile as a Rust regular expression (or \
                         compiles too large)",
                        self.f
                    ),
                )
            })
    }
}

impl Run {
    fn new(limits: HostLimits, pool: Arc<BytePool>) -> Self {
        Run {
            limits,
            pool,
            handles: BTreeMap::new(),
            next: 1,
            reads: 0,
            ops: 0,
            bytes: 0,
            peeked: 0,
            external: false,
            reach: Default::default(),
        }
    }

    /// The tools this program may call; none until this is said.
    fn reaching(mut self, names: impl IntoIterator<Item = String>) -> Self {
        self.reach = names.into_iter().collect();
        self
    }

    /// The handle a program passed, by its stub. A string that is not the
    /// exact stub of a handle this program holds is refused without being
    /// quoted.
    fn handle(&self, a: &Args<'_>) -> Result<(u64, Handle), HostRaise> {
        let not = || {
            raise(
                Tag::HandleError,
                format!(
                    "`{}`: its first argument is not a handle this program holds",
                    a.f
                ),
            )
        };
        let s = a.get(0, "h").and_then(Value::as_str).ok_or_else(not)?;
        let id = s
            .strip_prefix("<handle ")
            .and_then(|r| r.split_once(':'))
            .and_then(|(n, _)| n.parse::<u64>().ok())
            .ok_or_else(not)?;
        let h = self
            .handles
            .get(&id)
            .filter(|h| stub(id, h) == s)
            .ok_or_else(not)?;
        Ok((id, h.clone()))
    }

    /// Hold `n` more bytes, against this program's limit and the process's.
    fn charge(&mut self, n: usize) -> Result<(), HostRaise> {
        if self.bytes.saturating_add(n) > self.limits.host_bytes {
            return Err(raise(
                Tag::LimitError,
                format!(
                    "host bytes: a result of {n} bytes would take this program past its {} \
                     ({} held)",
                    self.limits.host_bytes, self.bytes
                ),
            ));
        }
        if !self.pool.take(n) {
            return Err(raise(
                Tag::LimitError,
                format!(
                    "host bytes, all programs: a result of {n} bytes would take this process \
                     past its {}",
                    self.pool.max
                ),
            ));
        }
        self.bytes += n;
        Ok(())
    }

    fn buffer(&mut self, text: String, tool: &str) -> Result<Handle, HostRaise> {
        self.charge(text.len())?;
        let ascii = text.is_ascii();
        let chars = if ascii {
            text.len()
        } else {
            text.chars().count()
        };
        let end = text.len();
        Ok(Handle {
            buf: Arc::new(Buffer {
                text: text.into_boxed_str(),
                ascii,
                tool: tool.to_string(),
            }),
            start: 0,
            end,
            chars,
        })
    }

    fn room_for(&self, n: usize, f: &str) -> Result<(), HostRaise> {
        if self.handles.len().saturating_add(n) > self.limits.live_handles {
            return Err(raise(
                Tag::LimitError,
                format!(
                    "live handles: `{f}` would make {n} more, past this program's {} ({} held)",
                    self.limits.live_handles,
                    self.handles.len()
                ),
            ));
        }
        Ok(())
    }

    fn mint(&mut self, h: Handle) -> String {
        let id = self.next;
        self.next += 1;
        let s = stub(id, &h);
        self.handles.insert(id, h);
        s
    }

    fn mint_all(&mut self, hs: Vec<Handle>, f: &str) -> Result<Value, HostRaise> {
        self.room_for(hs.len(), f)?;
        Ok(Value::Array(
            hs.into_iter()
                .map(|h| Value::String(self.mint(h)))
                .collect(),
        ))
    }

    /// One tool read: counted, through the gate, and its result a handle.
    async fn read(
        &mut self,
        host: &HostCalls,
        call: &ExternalCall,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Value, HostRaise> {
        if !self.reach.contains(&call.name) {
            return Err(raise(
                Tag::ToolError,
                format!(
                    "`{}` is not a tool this program may call, so nothing ran",
                    call.name
                ),
            ));
        }
        if self.reads >= self.limits.tool_reads {
            return Err(raise(
                Tag::LimitError,
                format!(
                    "tool reads: this program has made all {} it may",
                    self.limits.tool_reads
                ),
            ));
        }
        self.reads += 1;
        let name = call.name.as_str();
        let input = tool_input(call)?;
        // A Stop reaches a program waiting on a host call too (§5): the
        // wait ends, the raise lands, and the interpreter, seeing the token,
        // ends the program `Cancelled` at that suspension.
        let answer = tokio::select! {
            a = host.call(name, input) => a,
            () = cancel.cancelled() => HostAnswer::Refused(
                "stopped: the run was cancelled while this call ran".into(),
            ),
        };
        match answer {
            HostAnswer::Refused(why) => Err(raise(Tag::ToolError, why)),
            // Unreachable while admission and the gate read one routing: a
            // routed read is `Staged` by class and never admitted. Said
            // plainly if it ever happens, with the gate's own notice (the
            // harness's words, naming the draft).
            HostAnswer::Staged(notice) => Err(raise(
                Tag::ToolError,
                format!(
                    "`{name}` is routed through the outbox, so the gate staged it rather than \
                     running it, and a program cannot stage. The gate said: {notice}"
                ),
            )),
            HostAnswer::Ran(out) => {
                self.external |= out.external;
                self.room_for(1, name)?;
                let failed = out.is_error;
                let h = self.buffer(out.content, name)?;
                let s = self.mint(h);
                if failed {
                    Err(raise(
                        Tag::ToolError,
                        format!("`{name}` returned an error; its text is {s}"),
                    ))
                } else {
                    Ok(Value::String(s))
                }
            }
        }
    }

    /// One handle operation (§3): counted, `budget()` included.
    fn op(&mut self, call: &ExternalCall) -> Result<Value, HostRaise> {
        let f: &'static str = HOST_FUNCTIONS
            .iter()
            .find(|n| **n == call.name)
            .copied()
            .unwrap_or("?");
        if self.ops >= self.limits.handle_operations {
            return Err(raise(
                Tag::LimitError,
                format!(
                    "handle operations: this program has spent all {}",
                    self.limits.handle_operations
                ),
            ));
        }
        self.ops += 1;
        let a = Args { call, f };
        match f {
            "budget" => Ok(self.budget()),
            "size" => Ok(Value::from(self.handle(&a)?.1.chars)),
            "slice" => self.slice(&a),
            "lines" => self.lines(&a),
            "split" => self.split(&a),
            "chunks" => self.chunks(&a),
            "find" => self.find(&a),
            "count" => {
                let (_, h) = self.handle(&a)?;
                Ok(Value::from(a.pattern(1)?.find_iter(h.text()).count()))
            }
            "json_get" => self.json_get(&a),
            "peek" => self.peek(&a),
            _ => Err(raise(Tag::HandleError, "no such host function")),
        }
    }

    fn budget(&self) -> Value {
        let l = &self.limits;
        json!({
            "tool_reads": l.tool_reads.saturating_sub(self.reads),
            "handle_operations": l.handle_operations.saturating_sub(self.ops),
            "host_bytes": l.host_bytes.saturating_sub(self.bytes),
            "live_handles": l.live_handles.saturating_sub(self.handles.len()),
            "peek_bytes": l.peek_bytes.saturating_sub(self.peeked),
        })
    }

    fn slice(&mut self, a: &Args<'_>) -> Result<Value, HostRaise> {
        let (id, h) = self.handle(a)?;
        let n = h.chars as i64;
        let (start, end) = (a.int(1, "start", Some(0))?, a.int(2, "end", Some(n))?);
        let norm = |i: i64| if i < 0 { i + n } else { i };
        let (s, e) = (norm(start), norm(end));
        if s < 0 || e > n || s > e {
            return Err(raise(
                Tag::HandleError,
                format!("`slice`: {start}..{end} is outside handle {id}, which has {n} characters"),
            ));
        }
        self.room_for(1, "slice")?;
        let b = h.bytes_at(&[s as usize, e as usize]);
        Ok(Value::String(self.mint(h.sub(b[0], b[1]))))
    }

    fn lines(&mut self, a: &Args<'_>) -> Result<Value, HostRaise> {
        let (_, h) = self.handle(a)?;
        let text = h.text();
        let mut pieces = text.matches('\n').count() + 1;
        if text.ends_with('\n') || text.is_empty() {
            pieces -= 1;
        }
        self.room_for(pieces, "lines")?;
        let mut out = Vec::with_capacity(pieces);
        let mut from = 0;
        for (i, _) in text.match_indices('\n') {
            let to = if text[..i].ends_with('\r') { i - 1 } else { i };
            out.push(h.sub(from, to));
            from = i + 1;
        }
        if from < text.len() {
            out.push(h.sub(from, text.len()));
        }
        self.mint_all(out, "lines")
    }

    fn split(&mut self, a: &Args<'_>) -> Result<Value, HostRaise> {
        let (_, h) = self.handle(a)?;
        let re = a.pattern(1)?;
        let text = h.text();
        self.room_for(re.find_iter(text).count() + 1, "split")?;
        let mut out = Vec::new();
        let mut from = 0;
        for m in re.find_iter(text) {
            out.push(h.sub(from, m.start()));
            from = m.end();
        }
        out.push(h.sub(from, text.len()));
        self.mint_all(out, "split")
    }

    fn chunks(&mut self, a: &Args<'_>) -> Result<Value, HostRaise> {
        let (_, h) = self.handle(a)?;
        let n = a.int(1, "n", None)?;
        let overlap = a.int(2, "overlap", Some(0))?;
        if n < defaults::CHUNK_FLOOR as i64 {
            return Err(raise(
                Tag::LimitError,
                format!(
                    "chunk size: `chunks` takes n of at least {} characters, and was given {n}",
                    defaults::CHUNK_FLOOR
                ),
            ));
        }
        if overlap < 0 || overlap >= n {
            return Err(raise(
                Tag::HandleError,
                format!("`chunks`: `overlap` must be at least 0 and below n ({n})"),
            ));
        }
        let (n, step) = (n as usize, (n - overlap) as usize);
        let mut spans = Vec::new();
        let mut start = 0;
        // Counted as it goes and stopped one past the limit, so a refusal
        // never allocates the whole list first.
        while start < h.chars {
            let end = (start + n).min(h.chars);
            spans.push((start, end));
            if end == h.chars {
                break;
            }
            start += step;
            if spans.len() > self.limits.live_handles {
                break;
            }
        }
        self.room_for(spans.len(), "chunks")?;
        let flat: Vec<usize> = {
            let mut v: Vec<usize> = spans.iter().flat_map(|(s, e)| [*s, *e]).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let bytes = h.bytes_at(&flat);
        let at = |c: usize| bytes[flat.binary_search(&c).unwrap_or_default()];
        let out = spans.iter().map(|(s, e)| h.sub(at(*s), at(*e))).collect();
        self.mint_all(out, "chunks")
    }

    fn find(&mut self, a: &Args<'_>) -> Result<Value, HostRaise> {
        let (_, h) = self.handle(a)?;
        let re = a.pattern(1)?;
        let mut flat = Vec::new();
        for (k, m) in re.find_iter(h.text()).enumerate() {
            if k == defaults::FIND_MATCHES {
                return Err(raise(
                    Tag::LimitError,
                    format!(
                        "find matches: more than {} for one `find`; narrow the pattern, or \
                         slice the handle first",
                        defaults::FIND_MATCHES
                    ),
                ));
            }
            flat.push(m.start());
            flat.push(m.end());
        }
        let chars = h.chars_at(&flat);
        Ok(Value::Array(
            chars.chunks(2).map(|p| json!([p[0], p[1]])).collect(),
        ))
    }

    fn json_get(&mut self, a: &Args<'_>) -> Result<Value, HostRaise> {
        let (id, h) = self.handle(a)?;
        let steps: Vec<Value> = match a.get(1, "path") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::String(s)) if s.is_empty() => Vec::new(),
            Some(Value::String(s)) => s.split('.').map(Value::from).collect(),
            Some(Value::Array(v)) => v.clone(),
            Some(Value::Number(n)) => vec![Value::Number(n.clone())],
            Some(_) => {
                return Err(raise(
                    Tag::HandleError,
                    "`json_get`: `path` must be a string of dot-separated steps or a list",
                ))
            }
        };
        let root: Value = serde_json::from_str(h.text()).map_err(|e| {
            raise(
                Tag::HandleError,
                format!(
                    "`json_get`: handle {id} is not JSON (it fails at line {}, column {})",
                    e.line(),
                    e.column()
                ),
            )
        })?;
        let mut at = &root;
        for (k, step) in steps.iter().enumerate() {
            let missing = || {
                raise(
                    Tag::HandleError,
                    format!("`json_get`: handle {id} has nothing at step {k} of the path"),
                )
            };
            at = match (at, step) {
                (Value::Object(m), Value::String(key)) => m.get(key).ok_or_else(missing)?,
                (Value::Object(m), Value::Number(n)) => {
                    m.get(&n.to_string()).ok_or_else(missing)?
                }
                (Value::Array(v), step) => {
                    let i = match step {
                        Value::Number(n) => n.as_i64(),
                        Value::String(s) => s.parse::<i64>().ok(),
                        _ => None,
                    }
                    .ok_or_else(missing)?;
                    let i = if i < 0 { i + v.len() as i64 } else { i };
                    usize::try_from(i)
                        .ok()
                        .and_then(|i| v.get(i))
                        .ok_or_else(missing)?
                }
                _ => return Err(missing()),
            };
        }
        let leaf = match at {
            Value::String(s) => s.clone(),
            Value::Object(_) | Value::Array(_) => at.to_string(),
            scalar => return Ok(scalar.clone()),
        };
        self.room_for(1, "json_get")?;
        let tool = h.buf.tool.clone();
        let leaf = self.buffer(leaf, &tool)?;
        Ok(Value::String(self.mint(leaf)))
    }

    fn peek(&mut self, a: &Args<'_>) -> Result<Value, HostRaise> {
        let (_, h) = self.handle(a)?;
        let n = a.int(1, "n", Some(defaults::PEEK_DEFAULT as i64))?;
        if n < 0 {
            return Err(raise(Tag::HandleError, "`peek`: `n` must be at least 0"));
        }
        let take = (n as usize).min(h.chars);
        let end = h.bytes_at(&[take])[0];
        let text = &h.text()[..end];
        if self.peeked.saturating_add(text.len()) > self.limits.peek_bytes {
            return Err(raise(
                Tag::LimitError,
                format!(
                    "peek bytes: {} more would take this program past its {} ({} used)",
                    text.len(),
                    self.limits.peek_bytes,
                    self.peeked
                ),
            ));
        }
        self.peeked += text.len();
        Ok(Value::String(text.to_string()))
    }
}

/// A tool call's input from a program's arguments: keywords, or one dict,
/// or both (keywords win).
fn tool_input(call: &ExternalCall) -> Result<Value, HostRaise> {
    let mut input = match call.args.as_slice() {
        [] => serde_json::Map::new(),
        [Value::Object(m)] => m.clone(),
        _ => {
            return Err(raise(
                Tag::ToolError,
                format!(
                    "`{}` takes keyword arguments (or one dict), not positional values",
                    call.name
                ),
            ))
        }
    };
    for (k, v) in &call.kwargs {
        input.insert(k.clone(), v.clone());
    }
    Ok(Value::Object(input))
}

/// What an ended program returns as its `tool_result`, and whether it is an
/// error: its printed output, then its ending (§5: "whatever the program
/// printed, then a harness line naming the backstop that fired").
///
/// `cap` is this call's share of the turn's output budget, which the
/// dispatch caps the result at; the ending must survive that cut, because it
/// is the model's only feedback. So a long traceback keeps its raised line
/// and says how many frames it dropped, and a result past `cap` opens with a
/// line naming how it ended, ahead of the printed output the cut may take.
fn render(f: Finished, cap: usize) -> (String, bool) {
    let (ending, is_error, summary) = match f.end {
        End::Complete { value, repr } => {
            let shown = match value {
                Some(Value::Null) => String::new(),
                _ => repr,
            };
            (shown, false, "normally".to_string())
        }
        End::Raised(r) => {
            let tb = trim_traceback(&r.traceback, cap / 2);
            let last = tb.lines().last().unwrap_or_default().to_string();
            (tb, true, format!("by raising: {last}"))
        }
        End::Backstop(b) => (format!("[{b}]"), true, format!("[{b}]")),
        End::Cancelled => {
            let line = "[stopped: the run was cancelled before the program finished]".to_string();
            (line.clone(), true, line)
        }
    };
    let mut out = f.printed;
    if f.printed_dropped > 0 {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!(
            "[{} more bytes of printed output were past the interpreter's cap, and dropped]",
            f.printed_dropped
        ));
    }
    if !ending.is_empty() {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&ending);
    }
    if out.is_empty() {
        out = "(the program printed nothing and its last line has no value)".to_string();
    }
    if out.len() > cap {
        out = format!(
            "[this program's result is {} bytes, past this call's share of the output budget \
             ({cap}); it ended {summary}]\n{out}",
            out.len()
        );
    }
    (out, is_error)
}

/// A traceback no longer than about `room` bytes: the first line, a count
/// of the frames dropped, and the last frame with the raised line.
fn trim_traceback(tb: &str, room: usize) -> String {
    if tb.len() <= room {
        return tb.to_string();
    }
    let lines: Vec<&str> = tb.lines().collect();
    if lines.len() <= 4 {
        return tb.to_string();
    }
    let head = usize::from(lines[0].starts_with("Traceback"));
    let tail = 3.min(lines.len() - head);
    let dropped = &lines[head..lines.len() - tail];
    let frames = dropped
        .iter()
        .filter(|l| l.trim_start().starts_with("File "))
        .count();
    let mut out = String::new();
    if head == 1 {
        out.push_str(lines[0]);
        out.push('\n');
    }
    out.push_str(&format!(
        "  [{frames} frames ({} lines) dropped to fit the output budget]\n",
        dropped.len()
    ));
    out.push_str(&lines[lines.len() - tail..].join("\n"));
    out
}

#[cfg(test)]
#[path = "code_tests.rs"]
mod tests;
