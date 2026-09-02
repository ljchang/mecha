# Code mode: which interpreter should run the model's programs?

Research pass, 2026-09-02, prompted by `AUDIT-RESEARCH.md` §3.12 landing on
monty and the owner saying he is "not wedded to monty". The question: when a
local open-weight model writes a short program that calls the harness's tools
as functions, and only the program's return value re-enters its context, what
runs the program?

This file is about the *interpreter behind a narrow `code` tool*. What it is
not about — confining `shell`, running `git`/`cargo`, the Landlock verdict —
is settled in `SANDBOX-RESEARCH.md` and not restated here.

**Marks**: ✅ verified against a primary source this pass · 📰 vendor's own
figure, not reproduced · ❓ inferred, not established.

---

## The one-sentence answer

**Stay on monty, but run it the way Pydantic now tells Rust embedders to —
through `monty-pool`'s subprocess workers, not in-process — and build the
tool so the interpreter is swappable, because monty is pre-1.0, was escaped
once in under 48 hours, and the runner-up (a WASM guest under wasmtime) wins
the moment a second escape lands or V1 slips past the build.**

Nothing else surveyed has all three of: a deny-by-construction capability
bridge, Python (what the local model writes best, by a wide margin), and
*pausing at a host call as the API itself* — which an outbox-routed send
needs. Every alternative gives up at least one.

---

## What the reference designs agree on

Four independent implementations converged on the same model-facing shape,
and the disagreements are only about language.

- **Anthropic, programmatic tool calling** ✅ ([docs](https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling),
  read 2026-09-02). Python. Allowed tools are "exposed to Claude's code as
  async Python functions"; each "takes a single dict of arguments and returns
  a string: the text of the `tool_result` you send back", and the model
  parses it (`rows = json.loads(await query_database({...}))`). At a tool
  call "code execution pauses and the API returns a `tool_use` block" tagged
  with a `caller`; the client answers, execution resumes; a pending call
  times out after ~4 min. Only `stdout` re-enters the context. Measured:
  +11% on BrowseComp/DeepSearchQA with 24% fewer input tokens 📰. And the
  line to copy into any design doc: `allowed_callers` "is not a hard
  API-level block… Do not rely on `allowed_callers` as a security boundary."
- **Cloudflare, Code Mode** ✅ ([blog](https://blog.cloudflare.com/code-mode/),
  2025-09-26). TypeScript, on training-data grounds: models have seen far
  more TypeScript than contrived tool-call formats. Schema → typed API with
  JSDoc; results via `console.log`; the isolate has no network, only RPC
  bindings to the connected servers.
- **MCP client best practices** ✅ ([2026-07-28 spec](https://modelcontextprotocol.io/docs/2026-07-28/develop/clients/client-best-practices)),
  now a normative pattern. The sentence that answers the approval question
  below: "Approving the script does not grant blanket approval for every tool
  call it makes at runtime; hosts may grant categorical approval… but the
  broker must still evaluate each call against that grant." Also: results
  from one server are untrusted input to another, and `isError: true`
  should become a thrown exception so model code can `try/except`.
- **openclaw** ✅ ([docs](https://docs.openclaw.ai/tools/code-mode)). JS/TS in
  a QuickJS-WASI worker; every enabled tool is "an async global function";
  one `maxOutputBytes` budget across all output; and suspension is real —
  `exec` returns `waiting`, "QuickJS-WASI snapshot/restore is the resume
  mechanism".
- **smolagents** ✅ ([secure execution](https://huggingface.co/docs/smolagents/main/en/tutorials/secure_code_execution)).
  Python, citing CodeAct (ICML 2024 ✅: code actions "up to 20% higher
  success rate" than JSON across 17 LLMs, including fine-tuned Llama 2 and
  Mistral). Its AST-walking `LocalPythonExecutor` is "best-effort mitigations
  only and… not a security boundary"; production means E2B, Docker or Modal.
  That is the honest position for any sandbox built by *filtering* a full
  interpreter, and it is the approach this file rules out.

So: **one function per tool, arguments as a dict/kwargs, string result the
model parses, printed/returned output back, pause at the host call.** The
contract is not in question; the interpreter is.

---

## The candidates

### monty (baseline) — Python subset, Rust, deny-by-construction

[pydantic/monty](https://github.com/pydantic/monty), v0.0.21 (2026-08-09) ✅,
README still says "not ready for prime time" ✅.

- **Bridge** ✅: "filesystem, env variables and network access are all
  implemented via external function calls" — `open`, `__import__`, `eval`,
  `exec` do not exist; `os`/`sys` are stubs. Nothing is unlinked; it was
  never there.
- **Pause is the API** ✅ ([docs.rs](https://docs.rs/monty/latest/monty/)):
  `MontyRun::start` returns `RunProgress::FunctionCall`; the host calls
  `.resume(value)` or `.resume_pending(future)` and gets `ResolveFutures`
  back when the program needs the result. A session serialises with
  `dump()` behind a `DUMP_VERSION` check and restores with `Dump::load()`.
  Pydantic: "a Monty snapshot is single-digit kilobytes" 📰.
- **Limits** ✅: `max_duration`, `max_memory`, `max_allocations`,
  `max_recursion_depth`; Willison (2026-05-22) checked them and they "all
  appear to work as advertised". The gaps, from Pydantic's own
  [resource_limits.md](https://github.com/pydantic/monty/blob/main/limitations/resource_limits.md) ✅:
  time is polled every 256 instructions and "host function calls pause the
  clock entirely"; memory is measured by the worker's *process-global*
  allocator, so a per-session budget is approximate.
- **Static analysis** ✅/❓: ships `ty`, type-checks against stubs before
  running (4.8 ms with, 4.5 µs without 📰). With no `eval`/`exec`/`import`,
  the host functions a program can reach are its free names ∩ registered
  names — a sound analysis *if* `getattr` (supported) cannot reach a host
  function by string. Not established; it is the first test to write.
- **Startup** 📰: 4.5 µs; measured by Pydantic under CodSpeed, script in the
  repo, **not independently reproduced**. Pydantic's own table puts
  starlark-rust at 1.7 ms and Docker at 195 ms (within noise of the 192 ms
  `SANDBOX-RESEARCH.md` measured here).
- **Security record** ✅: Hack Monty round 1 ($5k, May 2026) was escaped in
  under 48 hours — a use-after-free from a `list.sort` key function and a
  missing GC root, chained into a heap read/write primitive
  ([postmortem](https://pydantic.dev/articles/hack-monty-postmortem),
  [write-up](http://verialabs.com/blog/pwning-pydantic-monty/)); fixed in
  v0.0.16 with a re-audit of every `unsafe` block. Round 2 ($10k, ended
  2026-05-30): "no one escaped the sandbox". Round 3 ($20k) is open, "the
  last round before Monty V1".
- **The change since the addendum** ✅: the README now says "for running
  untrusted code from Rust, we recommend the `monty-pool` crate rather than
  the in-process API" — subprocess workers, so an adversarial crash "kills
  only the worker", with a watchdog for hard timeouts and a hard memory
  limit. The WASM build is the opposite: "a sandbox crash is a host crash
  there." The pool's per-call overhead is unmeasured ❓.
- **Language** ✅: v0.0.19 added classes (no inheritance, `super()` or
  `match`); stdlib is `asyncio, base64, binascii, collections, dataclasses,
  datetime, functools, itertools, json, math, os, pathlib, re, sys, typing,
  unicodedata`. Enough for orchestration; still no numpy.

### RustPython — disqualified on criterion 1

v0.5.0 (2026-03-31) ✅. Full CPython-compatible stdlib including a working
`os`, and the sandboxing RFC
([#4210](https://github.com/RustPython/RustPython/issues/4210), opened
2022-10) is still open: "execution can loop forever, memory allocation is
not bounded, and imports cannot be disabled" ✅. It is the smolagents
position without the AST filter. Out.

### starlark-rust — hermetic by specification, no pause

[facebook/starlark-rust](https://github.com/facebook/starlark-rust) 0.14.2
(2026-06-05), ~870k downloads/month, Buck2 depends on it ✅. Starlark is
"deterministic and hermetic" by spec: no I/O, no clock, no `import`, no
`while`, no unbounded recursion. `Evaluator` offers `set_max_callstack_size`,
`set_max_heap_size` (best-effort), `set_max_tick_count` (deterministic) and
`set_check_cancelled` ✅ — a complete limits story. Sync only; `Evaluator`
is `!Send` ✅; a host call must return before evaluation continues, so an
outbox pause means parking a thread or re-running. Reach analysis is trivial
and sound. The model problem is real: a 2026-08-24
[field report](https://imti.co/starlark-agents/) on starlark-go agents had
to pre-validate what models keep writing — "import statements, try/except
blocks, f-strings" — because it *is* Python syntax and the model writes
Python. Right for a build system; wrong for a 7B model without a retry loop.

### Rhai — excellent sandbox, wrong language

1.26.0 (2026-08-25), 1.5M downloads/month, 627 dependents ✅. Sandboxed by
default; `set_max_operations`, `on_progress` called per operation for
wall-clock cut-off, call-depth and size limits ✅. No async: the book's
["Blocking/Async Function Calls"](https://rhai.rs/book/patterns/blocking.html)
pattern is a channel and a blocked engine ✅. Syntax is "JavaScript+Rust-like"
and appears in no code corpus a local model has learned from. Out on
criterion 4 alone.

### mlua + Luau — the mature sandbox, in C, in Lua

mlua 0.12.1 (2026-08-29) ✅ binds Luau 0.736 (2026-08-28, weekly releases)
✅. `Lua::new_with(StdLib, …)` omits `io`/`os`/`package`/`debug`;
`Lua::sandbox(true)` freezes globals and library tables; `set_memory_limit`,
`set_interrupt` (`VmState::Yield`/`Continue`), `set_hook` every N
instructions; `create_async_function` runs a Rust future when the caller is a
coroutine — so **a host call can suspend the Lua thread and resume later**
✅, though as a live VM, not bytes on disk. Luau's
[security policy](https://github.com/luau-lang/luau/blob/master/SECURITY.md)
✅: "a safe sandbox that scripts cannot escape from, short of vulnerabilities
in custom C functions", a HackerOne bounty, and the honest exclusion — "Luau
does not provide termination guarantees" (the interrupt is the host's job).
Costs: a C VM in-process, so an interpreter bug is a native memory bug in
the agent; and Lua is low-resource — StarCoder2 groups it with D, Julia and
Perl ✅, CodeLlama-70B scores 41.7 pass@1 on MultiPL-E Lua ✅ against Python
in the 50s–60s for the same class. The strongest candidate *if* Lua were
acceptable; for a Qwen-class model that must get the first draft right, it
is not.

### piccolo — the right VM design, not yet a product

0.3.3, released 2024-06-16 ✅, "still very experimental… expect frequent
pre-1.0 API breakage", un-paused "after four years" ✅. Stackless: `Executor::step`
takes fuel, and Rust callbacks return `Sequence`s that can yield — the cleanest
pause/resume design in the survey. But `io`/`os`/`string`/`table` are
"sparsely implemented", there are no error messages or stack traces, and no
release in over two years. Watch; do not build on.

### rquickjs / QuickJS-ng — the JavaScript option

rquickjs 0.12.2 (2026-07-27) binds QuickJS-ng ✅ (0.16.2, 2026-08-20 ✅).
`set_memory_limit` (a no-op under the Rust allocator features ✅),
`set_interrupt_handler`, `set_max_stack_size`; `AsyncRuntime` maps Rust
futures to Promises both ways ✅, so a host tool call is just an unresolved
Promise — a natural in-process pause, not a serialisable one. JS is the
second-best language for the model (Qwen2.5-Coder-32B-Instruct MultiPL-E ✅:
Python 92.7, TypeScript 86.8, JavaScript 85.7; the 7B: 87.8 / 81.8 / 83.2).
The cost is a C engine with a live CVE stream: CVE-2024-13903 (stack
overflow), CVE-2025-46687 (heap overflow in `JS_ReadString`),
CVE-2025-69653/69654 (GC assertion DoS) ✅. openclaw's answer to that was to
put QuickJS *inside* WASM — which is the runner-up below, with JS as guest.

### Boa — pure Rust JS, not yet for critical work

boa_engine 0.22.0 (2026-08-28), Test262 94% ✅. `RuntimeLimits` (loop
iterations, recursion, stack) and `instructions_remaining` exist; no memory
limit; `Context` is `!Send` ✅; the project's own guidance is to wait
"before using this for critical workloads" ✅. A good future answer to "JS
without a C engine in-process"; not this year's.

### Deno / V8 isolates — the second runtime, measured

`deno_core` 0.411.0 (2026-08-27), 369 breaking releases, built on `rusty_v8`
✅; no permission layer at that level — the embedder writes the ops. Deno
the CLI is the *inverse* of deny-by-construction: ambient capabilities you
deny with flags, and the record shows the cost — ten advisories between
2026-05-27 and 2026-06-17 alone, including `fetch()` and WebSocket "sandbox
bypass via missing DNS resolution check" ✅, and the 2024 Secfault write-up's
ToCToU race on the permission check ✅. Cloudflare's Code Mode is the
cleanest V8 design (no network, bindings only), and Check Point still found
five bugs in workerd's native glue in 2026, two Critical, one a zlib
use-after-free giving native code execution — "isolating the execution
engine is insufficient" ✅. This is `SANDBOX-RESEARCH.md`'s Pyodide finding
in a different host: the JS runtime decides what escapes.

### wasmtime + a WASM guest — the runner-up

wasmtime: `consume_fuel` (deterministic), `epoch_interruption` (wall-clock,
"up to 2–3×" faster), `ResourceLimiter` for memory, `func_wrap_async` host
functions ✅. **No snapshot of a running instance**: Wizer pre-initialises,
Asyncify instruments for unwinding; neither dumps a paused program ✅. Ten
advisories 2026-04 to 2026-08, three escape/memory-safety class — but the
High ones are WASI filesystem bugs (trailing-slash symlink escape,
`path_open(TRUNCATE)` bypass) ✅, and a code tool links **no WASI at all**:
the imports *are* the bridge. Guests, from `SANDBOX-RESEARCH.md`'s
measurements: MicroPython 16 ms, CPython WASI AOT ~40 ms, or QuickJS-wasm for
JS. A guest-interpreter bug is confined by wasm; a wasmtime bug is native.
Cost: two interpreters per call, JSON through linear memory, and 16–40 ms
per start against monty's microseconds.

---

## Comparison

Criteria in the brief's order of weight. ● meets · ◐ partial · ○ fails.

| | monty | RustPython | starlark-rs | Rhai | mlua/Luau | piccolo | rquickjs | Boa | Deno/V8 | wasmtime+guest |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 Bridge is the only way out | ● by construction | ○ full `os` | ● by spec | ● | ● after `sandbox` | ● | ● | ● | ◐ embedder-built ops | ● imports only |
| 2 No second runtime; bug cost | ● Rust; pool isolates crash | ● Rust | ● Rust | ● Rust | ◐ C VM in-process | ● Rust | ◐ C engine, CVEs | ● Rust | ○ V8 + glue (Check Point) | ◐ Rust JIT; guest confined |
| 3 Startup / per-call | ● 4.5 µs 📰 | ◐ | ● 1.7 ms 📰 | ● | ● sub-ms ❓ | ● | ● sub-ms ❓ | ◐ | ◐ ms | ○ 16–40 ms ✅ |
| 4 Local model writes it | ● Python | ● Python | ◐ Python minus features | ○ | ◐ Lua low-resource | ◐ | ◐ JS | ◐ JS | ◐ JS/TS | ● (Python guest) |
| 5 Static reach analysis | ● free names + `ty` ❓ | ○ | ● | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ per guest |
| 6 Pause at host call / resume | ● API + `dump()` | ○ | ○ sync | ○ sync | ◐ coroutine, live only | ● fuel/yield | ◐ Promise, live only | ◐ | ◐ | ○ no snapshot |
| 7 Limits with cancel | ● (host time uncounted) | ○ | ● | ● | ● | ● | ● (mem n/a w/ Rust alloc) | ◐ no mem | ● | ● |
| 8 Maturity | ○ pre-1.0, escaped once | ◐ | ● Buck2 | ● | ● Roblox, bounty | ○ dormant | ● | ◐ | ● | ● |
| 9 Rust integration, async host | ● `resume_pending` | ◐ | ◐ `!Send`, sync | ◐ sync | ● `async` feature | ◐ | ● futures | ◐ `!Send` | ◐ heavy dep | ● `func_wrap_async` |

---

## Ranked recommendation

1. **monty via `monty-pool`.** The only entry ● on the four heaviest
   criteria *and* on pause/resume, which turns an outbox-routed send from
   "fail and re-run" into "park and resume". Its ○ is maturity, and the
   mitigation is structural: register only `Registry` dispatchers, run the
   worker out of process, and keep every host function inside the trust
   boundary (path jail, approver, interlock), so an escape buys only the
   tools the program was already allowed to call.
2. **Runner-up: wasmtime with a Python guest** (MicroPython for size, CPython
   WASI for fidelity). Wins if, before the tool ships, Hack Monty is escaped
   again or V1 does not land. It trades microsecond starts and snapshots for
   a boundary with a fuzzing pedigree and a bigger Python; the pause is an
   async host import that awaits.
3. **If the language had to be JS**, rquickjs on QuickJS-ng — but inside
   wasm (openclaw's construction), never as a C engine in the agent process.

**Not chosen, one line each**: RustPython — no bridge; starlark — no pause,
and the model writes Python at it; Rhai — no corpus; Luau — Lua; piccolo —
no release since 2024; Boa — its own advice; Deno/V8 — the second runtime
`SANDBOX-RESEARCH.md` already measured escaping.

---

## The model-facing contract

Follow Anthropic and openclaw where they agree; take the simpler option where
they differ.

- **One function per registered tool**, not a generic `call(name, args)`.
  Every reference design does this, it is what the model has seen, and it
  makes criterion 5 cheap: the tools a program can reach are its free names,
  where a generic `call` hides the name in a string. Names from `Tool::name`;
  parameters and a `ty` stub from `Tool::input_schema`, so a wrong argument
  is a type error before the first host call.
- **Synchronous in the program, asynchronous in the host.** The program
  writes `rows = json.loads(query_database(sql=...))`; monty pauses on the
  call regardless, the harness awaits `Tool::call` and then `.resume`s. A 7B
  model writing top-level `await` correctly is a bet not worth taking in v1;
  `resume_pending`/`ResolveFutures` keeps `asyncio.gather` reachable later.
- **Results are the tool's text**, as `ToolOutput` carries them; the model
  parses. `is_error: true` raises a `ToolError` the program can catch (the
  MCP recommendation); an uncaught error ends the program and its traceback
  is the result, so the model can self-correct.
- **What re-enters the context**: `print` output plus the final expression,
  under the existing `output_budget_bytes` and spill rule — it is a tool
  result and gets a tool result's cut.
- **A `code` tool, not a mode**: one `Tool` impl taking `{ program }`, whose
  declared `Capabilities` are the union of the tools the program reaches, so
  the interlock's view of the turn is honest before anything executes.

---

## The two hazards, restated against monty

**Taint must update per host call, inside the program.** Every
`RunProgress::FunctionCall` goes through the *same* gate a model-issued call
takes in `run_tools` — the interlock against `Taint` as of *that call*,
`pre_tool` hooks, `Approver::approve`, `OutboxRoute::routes` — and on return
the result's `ToolOutput::external` and the tool's `Capabilities` arm the
taint *before* `.resume()`. A value held in a variable and never printed
armed it just the same. This is the same-turn batching hole
(`ARCHITECTURE.md` §Security model) one level down. The test that fails on
the naive implementation: a program calling a `private_data` tool and then
an `external_send` tool is refused at the second call with
`Decision::Blocked`, and the refusal reaches the program as the raised
error, not the model as a tool result.

**Approval of a capability set, once — and still enforced per call.**
Before running, compute free names ∩ registered tool names and show *that
list* as the thing being approved; thirty calls to three tools is one
prompt. Then keep the grant categorical and per-run, as the MCP text says,
and evaluate each `FunctionCall` against it — a tool outside the set is
`Blocked`, and any `external_send` still goes to `Approver::escalate`,
because a human reading a list of names is not a human reading a send. The
analysis is sound only if monty's `getattr` cannot reach a host function by
string — unverified; the test is a program that tries.

New since the addendum: **a paused program is state that outlives the
turn.** A `dump()` taken at an outbox gate carries locals — including
private data already read — and must carry the `Taint` beside it, as the
session file does. `DUMP_VERSION` means a mecha upgrade invalidates parked
programs; that must fail closed (staged send dropped, with a note), never
silently re-run from the start.

---

## What this means for mecha

- `AUDIT-RESEARCH.md` §3.12's shape stands and still depends on §3.7's
  dispatch split: the gate in `run_tools` must be callable from a
  `FunctionCall` handler with the same `RunContext`, `ToolCtx::taint` and
  approver. Build the gate first; the interpreter plugs into it.
- Put the interpreter behind a small trait (`start`, `resume`, `dump`,
  `load`) from day one — the cheap part of keeping the runner-up reachable,
  and what lets `mecha eval` grade one program against two backends.
- `monty-pool` is a *third* process class beside llama-server seats and
  sandboxed MCP servers: clear its environment the way `mcp.rs`'s `connect`
  does, and `preflight` it so a worker that will not start fails the run.
- Every startup figure here is somebody's own. Before one decides a design,
  run monty's `scripts/startup_performance.py` and a wasmtime MicroPython
  start on this box and write both numbers into this file.

---

## Not researched

- **Independent reproduction of any startup number.** Every microsecond and
  millisecond above is the vendor's; the only figure measured here remains
  Docker's 192 ms.
- **Whether `getattr` in monty can reach a registered external function by
  name.** Decides the soundness of the free-name analysis; a one-line test.
- **`monty-pool`'s per-call overhead** and what an IPC round trip per host
  call costs against the in-process microseconds.
- **How a Qwen/DeepSeek-class model actually performs writing against a
  one-function-per-tool Python stub** — first-draft success rate, retries.
  CodeAct measured Llama 2/Mistral in 2024; no measurement exists for the
  model this box runs, and it is an eval case, not a search.
- **pctx** (two locked-down Deno sandboxes, Rust) and **Codex's in-process
  V8 code mode** — noted as V8 designs, not evaluated further.
- **littrs** (chonkie-inc): a second Rust Python sandbox with a `#[tool]`
  macro, instruction caps, optional WASM isolation, no `async`, no snapshot.
  Younger than monty with less scrutiny; the fallback if monty falls over.
- **Wizer/Asyncify for pause-and-dump under wasmtime**, beyond confirming
  neither is a running-instance snapshot.
