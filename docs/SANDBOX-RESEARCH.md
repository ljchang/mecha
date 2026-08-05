# Sandboxing: is anything better than a container per command?

Research pass, 2026-08-04. The question was whether something faster and
lighter than Docker exists for confining `shell`, and specifically whether the
"langchain rust-based Python" sandbox is useful here.

Context that shapes the answer: on this machine **bwrap does not work** —
Ubuntu 23.10+ added `kernel.apparmor_restrict_unprivileged_userns=1` and ships
no AppArmor profile for it — so the fallback is a throwaway Docker container
per command. That works and is correctly fail-closed, but it is slow to start.

---

## The recommendation

**Landlock + seccomp, measured at 1.28 ms against 192 ms for the current
`docker run`** — roughly 150× faster — while still running `git`, `gcc`,
`cargo` and `node`. It needs **no root, no KVM, and no unprivileged user
namespaces**, which is precisely why it works on this box where bwrap does not.

Two things to carry over from Codex's implementation rather than reinvent:

- Its **fail-closed check on `RulesetStatus::NotEnforced`** — Landlock degrades
  silently on kernels that lack it, and a sandbox that quietly does nothing is
  the failure mode mecha already refuses elsewhere (`Sandbox::preflight`).
- Its **seccomp denylist**, which closes two holes a naive prototype missed:
  `io_uring` (a general syscall-bypass channel) and `AF_INET` socket creation.

Honest caveat: **Codex is retreating from this approach while Cursor stands on
it.** That disagreement is unresolved and worth understanding before
committing. Landlock is also filesystem-and-network only — it is not a
namespace, so process and PID isolation would still come from elsewhere.

---

## WASM and "langchain rust-based Python": fast, and disqualified

The startup numbers are genuinely excellent — better than expected:

| Runtime | Cold start | Memory | Artifact |
|---|---|---|---|
| MicroPython wasm | **16 ms** | 8 MB | 0.45 MB |
| componentize-py (AOT) | **0.03 s** | 38 MB | 18.4 MB |
| CPython WASI (AOT `.cwasm`) | **0.04 s** | 31 MB | 26 MB |
| Pyodide | ~1.0 s (76 ms with snapshot) | ~112 MB | 6.2 MB brotli |

The AOT figures beat Docker by ~5,000× and are competitive with Landlock.
**And it does not matter**, because the capability probe is fatal for this
workload. Measured on CPython WASI under wasmtime 47:

- **No `dlopen`.** WASI preview1 has none; C extensions must be statically
  linked at build time. There is no runtime loading, ever.
- **No subprocess, no `fork`, no threads, no sockets.** `os.system` does not
  merely fail — *the function does not exist*.
- `ssl`, `ctypes`, `mmap`, `fcntl`, `termios` are all `ModuleNotFoundError`.
- componentize-py is stricter still: the component contains **exactly the
  module graph the build-time analysis saw**, so
  `importlib.import_module('gzip')` raises unless `gzip` was imported at build
  time. Any dynamic-plugin or arbitrary-command architecture is structurally
  impossible.

**This is durable rather than transitional.** PEP 816, verbatim: *"WASI 0.2
support has been skipped due to lack of time… it was deemed better to go
straight to WASI 0.3."* The support table pins WASI **0.1** for Python 3.11
through **3.15** — no sockets and no threads for the foreseeable future.

So the headline is "you cannot run an existing ELF binary," but the sharper
version is that **even the purpose-built WASM Python toolchain cannot spawn a
process**. Running `git`, `gcc` or `cargo` under WASM is not a porting problem;
it is a missing primitive.

Also corrected along the way: **Pyodide will not load under wasmtime at all** —
`legacy_exceptions feature required for try instruction`. Its import section
has **534 of 548 imports satisfied by JavaScript glue**; the 14
`wasi_snapshot_preview1` imports make it *look* partly WASI and it is not.
Committing to Pyodide means committing to a JS host permanently.

### A measured escape, worth recording in the security notes

**Pyodide running under Node has a live host-shell escape.** Measured:
`os.system("touch /tmp/PROOF_OF_SHELL")` returned 0 and **created a real file
on the host filesystem**, not in the wasm MEMFS. The cause is in the shipped
`pyodide.asm.mjs`:

```js
function __emscripten_system(command){ if(ENVIRONMENT_IS_NODE){
  var cp=require("node:child_process");
  var ret=cp.spawnSync(cmdstr,[],{shell:true,stdio:"inherit"}); ... }
```

In a browser it returns `-ENOSYS` and is inert. Under Node it is not a sandbox
at all. The Pyodide FAQ says *"attempts to use `threading`,
`multiprocessing`, or `subprocess` will raise a `RuntimeError`"* — which is
true and **does not cover this path**, because `os.system` goes through libc
`system()` rather than the `subprocess` module.

langchain-sandbox runs Pyodide under **Deno** rather than Node, so this exact
code path differs. But the lesson generalises and is the reason to record it:
**in a Pyodide sandbox the JS host decides what escapes, and the documentation
was wrong about its own boundary.** That moves the verdict from "wrong tool"
to "wrong tool with a known escape class."

---

## Where this leaves mecha

1. **Keep Docker as the confinement backend for `shell`.** It is the only
   surveyed option that both runs arbitrary binaries and is already working
   and verified here.
2. **Landlock + seccomp is the upgrade path worth prototyping** — 150× faster
   start, no root, and it works where bwrap is blocked by AppArmor. It would
   also give this machine a working confined-`shell` story that does not
   depend on Docker being up.
3. **A WASM Python sandbox cannot replace `shell`** — but it *could* back a
   separate, narrower code-execution tool. That is the interesting connection
   to the context research: **programmatic tool calling**, where the model
   writes code that calls tools and intermediate results never enter the
   context, is the largest unexplored token lever, and it only needs Python
   plus tool bindings — no subprocess, no compilers. The capability limits that
   disqualify WASM for `shell` are mostly irrelevant for that use.
4. If that is ever built, **do not use Pyodide-under-Node**, and treat the JS
   host as part of the trust boundary regardless of runtime.

---

## Not measured

**container2wasm** and **WebVM/CheerpX** — x86 emulation inside WASM, the only
route to running an existing ELF under WASM. The performance penalty is
unmeasured here; an order of magnitude or worse is the expectation, but that
is a guess and is labelled as one.

---

# Addendum, 2026-08-05: monty

[pydantic/monty](https://github.com/pydantic/monty) — a minimal Python
interpreter written in Rust, built for running LLM-generated code. Evaluated
against the two questions this file separates, and the answers are different.

**It does not replace `shell`, for the reason already settled above.** It is a
Python interpreter, so it cannot spawn `cargo`, `git` or `gcc`. Monty is
stricter than the WASM options here rather than looser: no filesystem, no
network, no environment, **no subprocess**, by construction. Same
disqualification, arrived at by design instead of by missing primitive.

**It is the best available answer to the other half — the narrow
code-execution tool behind programmatic tool calling**, and it beats every
WASM option measured above on the axis that disqualified them.

| | Startup | Host runtime |
|---|---|---|
| `docker run` (current) | 192 ms | — |
| Landlock + seccomp (recommended above) | 1.28 ms | — |
| MicroPython wasm | 16 ms | wasmtime |
| CPython WASI (AOT) | ~40 ms | wasmtime |
| Pyodide | ~1 s | **JS** |
| **monty** | **0.004 ms** (4.5 µs untyped; 4.8 ms type-checked) | **none — it is a Rust crate** |

(Pydantic's own figures, against their own Docker baseline of 195 ms, which is
within noise of the 192 ms measured here. Not independently reproduced.)

Three properties make it the right shape, in descending order of how much they
matter:

- **The only bridge to the host is functions you pass in.** Zero access by
  default — no filesystem, network, env, or syscalls — and capabilities are
  opted into as external functions the embedder registers. That is precisely
  the rule `PRIOR-ART-RESEARCH.md` wrote down for code mode and could only
  state as discipline: *every call the bridge makes must route back through the
  registry, or code mode is a hole straight through interlock, hooks and
  approver*. With monty it is not discipline, it is the architecture — register
  only `Registry` dispatchers and there is nothing else to reach.
- **No host runtime to escape into.** The sharpest finding above was a
  *measured* Pyodide-under-Node host shell escape, where the documented
  boundary was wrong about itself. Monty is a bytecode VM in Rust (Ruff's
  parser, its own bytecode) with no JS host and no wasmtime embed, so that
  entire escape class does not exist. It also compiles to WASM if a browser
  ever matters, which is how Simon Willison exercised it.
- **Snapshot and resume, in single-digit kilobytes.** State serialises and
  execution resumes later. Nothing in the harness survey uses this, and it
  lands on something mecha already has: a program that hits an outbox-routed
  call could be *paused at the gate* and resumed after release, instead of
  failing and being re-run.

Resource limits (memory, allocations, stack depth, execution time, with
cancellation) map onto the existing budget concepts. ~4.5 MB package, ~5 MB
resident, Rust/Python/JS bindings.

### What it would take, and the two things to get right

A `code` tool taking a Python program, with one external function per
registered tool — or a single `call(name, args)` — dispatching through the
**same path a model-issued call takes**: interlock → hook → approver → outbox
routing. If that path is shared, nothing about the security model changes and
the tool inherits all of it.

Two design points that are not free:

- **Taint must update *within* the program, not at its start.** This is the
  batching hole again, in a new place. That bug gated every call in a turn
  against the taint as of the turn's *start*, so "read private data and send
  it" in one assistant turn passed both gates; the fix was to gate on what the
  turn *will* arm. A program that calls a private-data tool and then a sink is
  the identical shape, and the interlock must see the second call with the
  first call's taint already applied. Also note taint has to arm on the *call*,
  not on what reaches the model — a value that stays in a Python variable and
  is never printed still armed it. Both want tests that fail on the naive
  implementation.
- **Approval does not obviously scale.** A program making thirty tool calls
  cannot prompt thirty times, and "approve the program" means approving
  something the human has to read as code. Monty compiles to its own bytecode
  and does type checking, so extracting the set of external functions a program
  can reach *before* running it looks feasible — approve a capability set once,
  then enforce it per call. That is a real open question and I have not
  established the static analysis is sound.

### Cautions

- **Experimental**, and says so: "not ready for prime time", first released
  February 2026.
- **No classes, no context managers, no match statements, no third-party
  packages**, and a partial stdlib. For tool-orchestration code that is mostly
  irrelevant — loops, comprehensions, f-strings, `json`, `re`, `datetime`,
  async are all present. For *data analysis* it is fatal: no numpy, no pandas.
  So monty does not become a general "run a script" tool.
- The supported-module list already differs between the README and Pydantic's
  own article (`os` in one, `pathlib` in the other). Check it at integration
  time rather than trusting either.
- **An interpreter escape lands in the agent process**, because monty runs
  in-process. That is hermes's point — *nothing inside the agent process
  constitutes containment* — and the mitigation is that the blast radius is
  whatever external functions were registered, which the embedder chooses. It
  is an argument for the host functions keeping the path jail and the approver,
  not for trusting the interpreter alone.

**Verdict:** the strongest candidate found for the token-offloading lever, and
it changes the ordering in that item — programmatic tool calling is no longer
blocked on building a code sandbox, because a Rust-native one now exists.
`shell`'s confinement story is unaffected: Landlock + seccomp remains the
recommendation there.
