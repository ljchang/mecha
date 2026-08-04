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
