# Code mode, reshaped by RLM — design

**Status: 2026-10-30, designed, nothing built. Ruled 2026-10-30 except R4, which waits on M1 (§11). M0 measured (§10).**

This supersedes the *shape* of `AUDIT-RESEARCH.md` §3.12 ("programmatic
tool calling, on monty"). That section's contract, its first hazard (taint
per host call) and its tests stand and are not restated. Its second hazard
(approving a capability set for a program's calls) does not arise in the
read-only slice, and §8 supersedes its "Landed when" item on static
capability extraction. `CODE-MODE-RESEARCH.md` remains the
interpreter comparison. The evidence for the changes is in
`RLM-RESEARCH.md`, and §1 below says what moved since the plan was written.

---

## 0. The design in one paragraph

A `code` tool runs a short Python program in monty, out of process through
`monty-pool`. The program can call only tools whose effect class is a
plain read (§2), and every call goes through the same gate a model-issued
call takes. What a read returns stays in the host as an **opaque handle**;
the program slices, searches and counts it through host functions, and
the bytes reach the model's context only if the program asks to see them
(§3). A host function, `llm_query`, hands one or more handles to a
`QuarantinedPass` whose answer is decoded against a **shape** — a boolean,
a number, one of a closed set, or a small list or record of those (§4). A
**budget** caps sub-calls (§5). Nothing in the program can send, write,
stage or render, so nothing needs to pause, and no taint semantics change.
The security gain is taken at a boundary that already exists:
**the typed code subagent** (§6) is a subagent profile whose child runs
programs and whose answer to its parent is checked by `trusted_output`'s
existing `AnswerShape` rule.

---

## 1. What changed since §3.12

- **monty reached 1.0** (2026-09-25); v1.4.0 is current (2026-10-26).
  `monty-pool` now ships in monty's own workspace and is "the recommended way to
  run Monty from Rust" for untrusted code. Its workers are CLI
  subprocesses, and a crash becomes `PoolError::Crashed` with the worker
  replaced. `asyncio.gather` runs host calls concurrently, which is what a
  batched sub-call needs. Snapshots moved to CBOR and are **not
  authenticated**, so anything mecha stores must carry a MAC. No security
  advisories are published; Hack Monty round 3 has no published result.
  The only post-1.0 escapes fixed are in monty-fs mounts, which a pool
  with no mounts never reaches.
- **The subset covers RLM programs**: closures, `for`/`while`,
  comprehensions, `try`/`except`, f-strings, `match`, simple classes,
  `re` (fancy-regex; no `subn`, no `VERBOSE`), `json`, `collections`,
  `itertools`, `datetime`, slicing and string methods. Absent: `enum`,
  `string`, `io`, `operator`.
- **Exceptions are a closed set, each carrying one string.** A program
  cannot define an exception class (classes cannot inherit), and a host
  can raise only a built-in `ExcType` with one message: `MontyException`
  is a type, a message, a traceback, and structured data for the unicode
  and JSON errors alone (monty's `docs/limitations/exceptions.md`, not this repo's, and
  `monty-types`' `ExcType`/`ExcData` at tag 1.4.0). So a host failure has
  no class of its own and carries no value but its message, which §4
  builds on and §3 constrains.
- **`AUDIT-RESEARCH.md` §3.2 shipped.** `QuarantinedPass::response_schema`
  exists. The shipped default is `structured_output = "disabled"` until an
  endpoint is verified; the owner's install sets `"llama_json"` on every
  local provider.
- **Its §3.7 did not, but the part this design needs exists.** `agent.rs`
  was 9,400 lines at the audit and is 20,039 now. `run_tools` is a thin
  wrapper over `Agent::dispatch`, and `Agent::dispatch_one` is already the
  one path a harness-made call takes through every gate (§7).
- **RLM evidence** (`RLM-RESEARCH.md`): the handle carries much of the
  gain (Qwen3-Coder CodeQA 20.0 → 66.0 with no sub-calls), depth above 1
  hurt the open model on three of four benchmarks (it helped on
  BrowseComp+, 44.7 → 68.0), and an unbudgeted open model made thousands
  of sub-calls.
- **The typed-extraction literature** says a typed answer from text an
  attacker wrote is *bounded*, not clean. FIDES (arXiv 2505.23643) orders
  labels by capacity, `bool ⊑ enum ⊑ string`, and keeps a typed result
  untrusted; endorsing it "effectively endorses untrusted values". Agent
  Data Injection (arXiv 2607.05120) makes the extractor return the wrong
  value. "CaMeLs Can Use Computers Too" (arXiv 2601.09923) names *branch
  steering*: a flipped boolean takes a valid but malicious branch of the
  approved plan. This is why §4 changes no labels and §6 puts the trust
  decision where the owner wrote the shape.

---

## 2. The first slice is read-only

**A program may call a tool only if it is in the registry the run was
given, its effect class is `Read` or `ReadThatRecords`, its egress is
`None`, it is not `destructive`, and it is `read_only()`.** Everything else is not registered in the program at all: not
refused at call time, absent. The registry term matters as much as the
other two. A subagent's registry is an allowlist, a policy can narrow a
run's tools, and `mecha eval` throws levers off. A global admitted set
would let `code` reach a tool the run was never given, which would make it
a bypass of all three. The `destructive` term is what excludes
`request_access`, which is `read_only()` but reaches a person to widen the
jail; that exclusion is deliberate. The `read_only()` term is not for safety; §7 says
what it is for. And one admission is per *call*, not per tool: a host call
for which the tool's `needs_a_person` answers (a path outside the jail
with no grant, through `ToolCtx::ask_for_path`) is refused in the program
and raises. It never reaches a person (§7).

The effect class is `ROLLOUT-DESIGN.md` §3.2's closed per-tool axis,
declared in code for built-in and first-party tools and earned from the
operator for third-party MCP tools. It is designed and not built. This
design is its second consumer, which is the argument for building it first
and once.

**`read_only()` cannot stand in for it.** Measured over `mecha tools --json`
on 2026-10-30: 47 of 83 tools are `read_only` with egress `None` and not
destructive, and that set includes `message_send`, `image_generate`,
`video_generate`, `persona_propose`, `hud_propose`, `todo` and `skill`.
`read_only()` means "skips the approval gate", not "has no effect". The
sharpest case is `ask_user`. It is `read_only()`, declares
`Capabilities::default()`, sits in every run's registry, and blocks on a
person *inside `call`*. Neither of the two routes §7 refuses would stop
it; only its effect class (`StoreWrite` in `ROLLOUT-DESIGN.md` §3.2)
keeps it out of a program.

What the slice gives up, and why that is the right trade:

- **No sinks**, so §3.12's hardest machinery is not needed yet: approving a
  capability set, pausing at an outbox gate, and a `dump()` carrying
  locals and taint across a release. Sends stay ordinary tool calls in the
  loop, where the interlock already works.
- **No `Blind` tools.** `web_search` is a `Read` by effect, but its query
  leaves the machine, and a query a program composes from private reads is
  exactly what the leak guard refuses. Research over the web waits for a
  later slice.
- **Outbox routing takes a read out of the set.** `ROLLOUT-DESIGN.md`
  §3.2 classifies first by "routed by the operator's outbox → `Staged`",
  so a read the owner routes through `[outbox] tools` is not `Read` and
  is not admitted. That ordering is what makes "nothing in the program
  can stage" true. Without it, a routed `mail_search` would stage
  mid-program and hand the program a draft notice instead of a handle.
- **Admitted today** (by egress and effect as `ROLLOUT-DESIGN.md` §3.2
  assigns them; names bare here, registered with their server prefix, such
  as `mail__mail_search`, in §6): `fs_read`, `fs_list`, `document_read`, `mail_search`,
  `mail_recent`, `mail_get_thread`, `calendar_list_events`,
  `calendar_freebusy`, `docs_read`, `sheets_read`, the graph reads, and
  `recall`. That is the set the uses in §9 need.

---

## 3. Handles

A tool called from a program returns a `Handle`, not a `str`. The bytes
stay in the host, owned by the program's run, along with the tool's name,
the call's `ToolOutput::external` and the tool's capabilities.

| Host function | Returns | Note |
|---|---|---|
| `size(h)` | int | |
| `slice(h, start, end)` | Handle | |
| `lines(h)`, `split(h, pattern)`, `chunks(h, n, overlap=0)` | list of Handle | `n` is each chunk's size in characters, not a count of chunks |
| `find(h, pattern)` | list of (start, end) | offsets, never text |
| `count(h, pattern)` | int | |
| `json_get(h, path)` | Handle for a string, object or array leaf; a number, bool or null as itself | structured tool output without decoding it into the program. A string leaf is text and stays a handle, or it would bypass `peek`'s cap and count |
| `peek(h, n=2000)` | str | **the only way a handle's text enters the program**; a `str[n]` sub-call result (§4) is the other way prose does, and both draw on one cap and one count |

`print(h)` renders a stub such as
`<handle 3: 48,213 chars from mail_get_thread>`, so the default spelling of
"look at it" costs a line, not the payload.

- **Taint is unchanged.** It arms on the host call, from the result's
  `external` and the tool's capabilities, before the program resumes —
  §3.12's first hazard, as specified. A handle therefore saves *context*;
  it does not lower a label. What it adds is that third-party prose reaches
  the model's planning only when the model chooses `peek`, which CaMeL
  calls control-flow integrity, and which mecha records rather than
  credits (§10, the `peeked` count).
- **Numbers do enter without `peek`.** `size`, `count`, `find` offsets and
  `json_get`'s number, bool or null leaves are values computed over
  attacker text. They are attacker-influenced, as a typed sub-call answer
  is, and bounded the same way: a number carries no instruction. They
  change no label, and they are not counted with `peek`, because the cap
  exists to bound prose.
- **But `find` and `count` take the program's pattern, so they are an
  oracle.** A program can learn text a character at a time by asking
  whether a pattern matches, which a number bounded by an attacker's
  choice is not. It spends the 2,048 handle-operation budget, not the
  peek cap, and `peeked_bytes` cannot see it. This is not a taint hole:
  the read armed the run, and the slice has no sinks. It does mean "prose
  reaches planning only through `peek`" is the default a program follows,
  not a property the host enforces, and §10 reads `peeked_bytes` as that,
  never as proof.
- **`peek` exists** because RLM roots look at a prefix to learn a format,
  and a program that cannot see its data guesses. It is capped, counted,
  and its output takes the ordinary output budget.
- **Patterns run host-side** with Rust's `regex` (linear time), not
  monty's backtracking engine, so a pattern cannot cost unbounded time on
  adversarial text.
- **A host-raised error is a third route for prose, so the host's
  messages carry no bytes.** A program reads a failure as `str(e)`, and an
  uncaught one reaches the model as a traceback. A message quoting a
  handle's text, a tool's error body or a failed pass's raw output would
  put prose into the program past `peek`'s cap and outside
  `peeked_bytes`, and into the model's context unasked. So every message
  the host writes names the handle's number, offsets, the tool and the
  limit, and for a failed pass what failed to parse (wrong type, a missing
  field, a label outside the set), never the text. A tool's error body is
  bytes like any result: the message names a handle that holds it, and a
  program that wants the reason `peek`s it, counted. Monty's own errors
  need no rule: a program holds a `str` only from `peek`, a `str[n]`
  result or its own literals, so an error quoting a value re-quotes
  counted text. An uncaught error's traceback takes the ordinary output
  budget, and a cut keeps the raised line and says how many frames it
  dropped, because that line is the model's only feedback (§5).

---

## 4. `llm_query` and `llm_map`

```python
llm_query(instruction: str, source: Handle | list[Handle], shape) -> value
llm_map(instruction: str, sources: list[Handle], shape) -> list[value]
```

- **One `QuarantinedPass` per call.** A fixed harness system frame, then
  the program's instruction, then the source bytes delimited, then the
  schema. No tools and one message, so **depth 1 is a property of the
  type**, not a setting. Recursion is impossible, not merely discouraged.
- **The shape vocabulary**, compiled to JSON Schema for
  `response_schema`, then checked again in Rust as a whole answer, the way
  `AnswerShape::matches` checks a child's (decoder and parser both):

  | Shape | Spelled | Bound |
  |---|---|---|
  | boolean | `bool` | |
  | number | `int`, `float`, `int[0,100]` | ranges where given |
  | one of | `["urgent", "fyi", "none"]` | ≤ 32 labels, each ≤ 64 chars |
  | list | `[shape]` | ≤ 64 items |
  | record | `{"due": "str[32]", "urgent": bool}` | ≤ 8 fields, no nesting past one level |
  | short text | `str[n]` | n ≤ 2,000; with lists and records, the result must also fit §5's 16 KB prose cap, so `[{"x": "str[2000]"}]` raises on the cap and the program narrows `n`. The table bounds shapes; it is not a menu of shapes that fit |

- **A `str[n]` result is prose entering the program**, so it is counted
  with `peek` against one cap (§5); otherwise it would be a second,
  uncounted route around §3.
- **`str[n]` and record fields of it never cross a trust boundary.**
  `AnswerShape` deliberately has no bounded-string shape, because
  "ignore previous instructions" fits in very few characters: a length
  cap vouches for nothing. Here it is a program-internal shape that
  changes no label. It must not become a `trusted_output` shape (§6).
- **`structured_output` must be on.** A provider with it `Disabled` makes
  `llm_query` raise (`QueryError:`, below), never fall back to parsing prose
  by hope: the silently-degrading-guard rule.
- **Results carry the union of their sources' labels.** A shape bounds how
  much an attacker can say; it does not change who said it. Inside a
  program nothing narrows, which is why this slice needs no security
  ruling.
- **A failure raises.** A pass that fails the schema twice raises,
  catchable in the program. It never returns a sentinel string: DSPy's
  `"[ERROR] ..."` string is the kind of value a program aggregates as
  data. Monty has no class to raise it as (§1), so every host failure is
  a `RuntimeError` whose message opens with a fixed tag. The tag is the
  only thing a program can tell *host* causes apart by; an untagged
  exception is monty's own (a `TypeError` in the program's code, say), and
  a backstop (§5) is not an exception at all. The tags are `QueryError:` for a failed pass,
  `ToolError:` for a tool that returned `is_error` or a host call refused
  (§7, followed by the loop's own refusal prefix, so a policy block still
  never reads as a person's denial), `HandleError:` for a §3 handle
  operation that cannot apply (`json_get` on bytes that are not JSON, a
  path that does not exist, a slice out of range), and `LimitError:` for
  a §5 limit. The text after the tag follows §3's rule: no bytes.
- **`llm_map` raises at the first failed item, and the passes before it
  are spent.** The error names the item's index and how many items
  completed, as numbers. Items, not passes: a schema retry is another
  pass (§5). An exception cannot carry the completed values
  (§1), and returning `None` in a failed item's place would be the
  sentinel again: a filter that keeps the `"urgent"` answers drops it
  silently, and the count comes out low with nothing raised. A program
  that wants partial results loops `llm_query` under `try`/`except` and
  keeps its own list of failures. Under R7 that costs nothing, because
  `llm_map` runs one pass at a time anyway. It does lose one guarantee:
  `llm_map` refuses up front when the sweep is longer than the remaining
  budget, while a hand-rolled loop spends passes until the cap and then
  raises mid-sweep. The prompt's example checks the remaining budget
  first. The root's prompt shows that
  loop, and an eval case checks the model uses it when one bad source
  should not void a sweep.
- **Same model only.** Sub-calls go to the model the run already holds. On
  the router, naming another preset swaps the resident model and evicts
  every slot.
- **On a remote provider, a handle removes a cap that exists today.** Now,
  `output_budget_bytes` and the spill rule bound how much of an oversized
  result ever enters a request. A handle chunked into `llm_map` sends all
  of it to whatever model the run holds. The provider connection is not a
  tool, so the interlock never covered it. That is harmless on the local
  router, which is what this slice is for. On a remote provider it is
  more of the owner's data leaving the box than the same task sends
  today. Ruled R8 (no): `llm_query` refuses on any
  provider that is not local, by the same rule as `structured_output`.
  "Local" is the predicate that already exists: the provider's `kind` is
  `local` and `provider::router::is_loopback` holds for its base URL. A
  base URL that does not parse is not local.

  **The seam is the request, not the loop.** The analogy to
  `structured_output` holds for the enforcement, not the plumbing:
  `structured_output` is a `Provider` trait method, and locality today
  lives only in `ProviderConfig`. And deciding it once when `code` is
  constructed is wrong under `Failover`, whose fallback may be remote.
  So it follows `response_schema`'s precedent end to end:
  - each pass's `CompletionRequest` carries `local_only: true`;
  - `Provider` gains `local()`, computed from the predicate above at
    provider construction, where the config is in hand;
  - a provider that is not local refuses a `local_only` request, by name;
  - `Failover` skips a non-local fallback for that request, exactly as it
    already skips a fallback without structured-output support;
  - `Halting`, which `provider::build` wraps around every provider,
    forwards `local()` to its inner provider. Its own comment says a new
    trait method must be forwarded there, after the `Failover::vision`
    incident. Either default body would be wrong: `false` silently kills
    the feature on the owner's own router, and `true` decides R8 by
    omission. So `local()` has **no default body**, and the compiler
    finds every implementation, including both wrappers. In-process
    test doubles (`ScriptedProvider`, the replay doubles, `provider/mod.rs`'s
    `Failing` and `Recording`) answer `true`, because they send nothing
    off the box, so `llm_query` is testable without a router. The refusal
    is tested with one double that answers `false`.

  The loop never learns which provider answered, which is the invariant
  `CLAUDE.md` names.
- **`max_tokens` sits above the reasoning budget** for every pass, as the
  local clients already require. A tight allowance for a three-label enum
  with thinking on is the HTTP-200-with-empty-`content` failure, and a
  pass refuses that by name, as the clients do, rather than reading it as
  an answer.
- **Thinking is off by default** (R6, after M0). A pass asks for
  `enable_thinking: false`. A program may ask for thinking per call. **The
  floor above wins either way: every pass gets `max_tokens` of 8,192,
  above the server's 4,096 reasoning budget.** Thinking off is a request
  for lower latency, not a licence to drop below the budget. The request
  cannot prove it was honoured: `enable_thinking` rides in
  `chat_template_kwargs`, which `LLAMA-SERVER.md` records as accepted and
  ignored without `--jinja`. The router writes `jinja = true` today, but a
  preset that lost it would leave thinking on. A 128-token pass would then
  return HTTP 200 with empty `content`, and every default `llm_query`
  would raise. M0 measured thinking at ~40× the latency of a pass
  without it, and it was less consistent on an ambiguous message (§10).

`llm_map` is written with `asyncio.gather`, so the program does not
change when concurrency does, but **its concurrency bound is one by
default**: the host runs one pass at a time. The bound is the host's own, because nothing else bounds it:
- `permit.rs` never applies to interactive work ("a chat turn, a voice
  call, a Slack thread — never takes a permit at all").
- `DEFAULT_BACKGROUND_PERMITS = 3` is one seat short of the server's so
  that the owner's turn never queues. Those three slots are the owner's
  reserve, not spare capacity. A background run holding a permit that
  fanned out to three concurrent sub-calls would fill all four slots and
  put the owner behind a decode, which is what that module exists to
  prevent.
- The slot count is not fixed: `router::background_seats` sizes to the
  resident model, never fewer than one.

**R7 is ruled: one at a time, for every run.** An interactive run's
sub-calls take no permit, as interactive work never does. A background
run's sub-calls run inside the one permit the run already holds. Nothing
in this slice adds seat accounting for sub-call passes.

*If R7 is ever raised* (design kept for that ruling, not built): any
concurrency above one would be bounded across runs, not per run. An
interactive run would still stay at one, or the owner would queue behind
their own fan-out. A background run's pass beyond its first would take a
seat from the permit directory the background runs share, and wait when
none is free, so the total of running background passes would never
exceed `router::background_seats`. At the pool's designed occupancy every
seat is already held by a run, so even then a second pass would usually
have nothing to take. On a model with two or more slots that would keep
the owner's reserve whole. On a one-slot model it cannot: `seats_for` is
never below one, and `permit.rs` concedes the owner can wait behind one
background decode there.

---

## 5. The budget

Per program. **Two kinds of limit, and only the first is catchable:**
- **Host limits** are counted by mecha at a host call. Crossing one
  raises a `LimitError:`-tagged `RuntimeError` naming the limit, which the
  program can catch (§4). These are sub-calls, sub-call input tokens,
  thinking-on passes, tool reads, handle operations, host bytes (per
  program and per process), live handles, and `peek` plus `str[n]` bytes.
- **Backstops** are the wall clock (`monty-pool`'s `request_timeout`), and
  monty's own memory, allocation and recursion limits with
  `max_suspensions`. They end the program from outside, uncatchably,
  because a backstop the program could catch would not be one. The `code`
  call still returns a `tool_result`: whatever the program printed, then
  a harness line naming the backstop that fired.

| Limit | Default (ruling R4) | Enforced by |
|---|---|---|
| sub-calls | 32 | the host, **per pass**: `llm_map` over N handles is N, and a schema retry is another. An `llm_map` longer than the remaining budget raises before its first pass |
| sub-call input tokens | 120k | the host, from `Usage`. Sized so that it binds before the clock even under contention (below) |
| tool reads | 256 | the host, counted at the gate |
| handle operations (§3) | 2,048 | the host; monty `max_suspensions` set to the sum as a backstop |
| wall clock | 600 s | `monty-pool` `request_timeout`, which is **unset by default** and must be set |
| host bytes held by handles | 64 MB | the host, summed over every tool read's raw result; the read that would cross it raises |
| live handles | 4,096 | the host. `split`, `lines` and `chunks` refuse a result that would cross it, and `chunks` refuses `n` below 256, so one counted operation cannot mint a handle per byte |
| host bytes, all programs in the process | 256 MB | one process-wide pool every program draws from. `dispatch` runs a turn's calls under `join_all`, so two `code` calls in one message, a parent's program beside an `analyst` child's, and `mecha batch` are all concurrent, and per-program limits alone would not bound N × 64 MB. A read that would cross either limit raises |
| memory, allocations, recursion | monty's limits | the pool worker, which bounds the interpreter's heap. It does not own the handles' bytes, which is why the two rows above exist |
| `peek` bytes plus `str[n]` result bytes | 16 KB together | the host, one count (`peeked_bytes`) |

**The limit a well-behaved program should hit first is the sub-call
cap, then the token cap, and the clock never.** The sub-call cap is the
expensive resource and the one the model's prompt names. The clock is the
backstop for a hang, not a budget, so it is sized against the *contended*
worst case. `provider/mod.rs`'s `STALL_TIMEOUT` comment gives both
halves: about 120 s for a 170k-token prefill alone, and a ~2.85× slowdown
under contention on this hardware. So the whole 120k-token budget is
about 85 s of prefill alone and about 240 s contended. That is why the
clock is 600 s, the number `STALL_TIMEOUT` reached by the same arithmetic,
and not 300. R4 chooses between raising the clock and lowering the token
cap, after M0 and M1 measure a pass on this box under contention as well
as alone.

**That ordering holds only with thinking off, the R6 default.** M0
measured a thinking-on pass at a median of ~14 s, before prefill. 32
thinking-on sub-calls are ~450 s of decode alone, and ×2.85 under
contention is past the 600 s clock. So a pass that asks for thinking
draws on the budget at a higher rate. R4 sets a separate, smaller cap on
thinking-on passes (8 is a proposal), so the clock still never binds
first.

A long document chunked into handles can reach 256 host calls before 32
sub-calls, because `find`, `slice` and `chunks` are host calls. So the
handle operations of §3 count toward their own, larger limit (2,048),
and 256 applies to tool reads only. Each limit's error says which limit it
was, because that error is the model's only feedback.

Each run's total also draws on its existing `RunContext` budget.

**Stop cancels; it does not wait out the budget.** The run's
`ToolCtx::cancel` token reaches the pool worker and every in-flight
`llm_query` and `llm_map` pass. The program stops at its next host call,
and a worker that does not answer within a grace period is killed (a
dropped `Checkout` already kills its worker). A cancelled program still
returns a `tool_result` carrying what it printed so far and a line saying
it was stopped, because the turn owes a result for every `tool_use` id. The
budget exists because the failure is known: Qwen3-Coder without a warning
made thousands of sub-calls. The root's prompt for this tool is written
for Qwen and tested in eval, never ported from the paper's GPT-5 prompt.

---

## 6. The typed code subagent

The security gain is taken at the subagent boundary, where the shape is
already the owner's:

```toml
[[subagent]]
name = "analyst"
description = "Answers one question over mail, documents or the graph by writing a program. Returns only the declared shape."
tools = ["code", "mail__mail_search", "mail__mail_get_thread", "document_read", "docs__docs_read", "graph__kg_search", "graph__kg_timeline"]
max_turns = 12
trusted_output = true
answer_shape = ["yes", "no", "unclear"]
```

The names are a default install's (`{server}__{tool}`, `prefix_tools`
defaulting true). A box whose graph server sets `prefix_tools = false`
registers `kg_search` bare and spells it so. `build_subagent` fails the
launch on a name that resolves to nothing, rather than running without it.

**`recall` is not in the profile, and cannot be.** It searches *this
conversation's* history and is registered after `prepare`, so it is not
in the pool `build_subagent` reads, and naming it fails the launch. If it
were in the pool, `Arc::clone` would point the child at the parent's
transcript, which `tool/recall.rs` forbids by name. It is the same clone
hazard as `code`'s. A program in the parent's own run may read `recall`
(§2, §9); a child may not.

**A profile that names `code` requires the feature.** `code` is a
`Feature`, off by default, and a feature that is off registers no tool
(`Feature::System`'s `system_read` is the precedent in `setup.rs`). So with
the feature off, this profile fails at launch, which is the right
fail-closed behaviour. But "asks for tool `code`, which is not available"
reads like a typo. `build_subagent` names the cause when the missing tool
belongs to a feature that is off ("needs the `code` feature: `mecha
features enable code`"), distinct from a misspelled name. `code` is the
first optional tool a documented profile names, so this is the first
time that distinction matters.

- **Inside the child**, everything above applies: it may peek, it arms
  honestly, and it has no tool that sends, so arming costs it nothing.
- **What `code` declares.** Its `Capabilities` are the union over its
  admitted set (§2) as built for this run, with egress `None` because no
  sender is admitted. `Subagent::new` derives the parent-facing legs
  statically from the child's registry, so a profile of
  `tools = ["code", …]` hands the parent a tool whose `private_data` and
  `untrusted_input` are those of the reads the program can reach. A `code`
  that declared `Capabilities::default()` would reopen the laundering hole
  that derivation's own comment says was closed. Strictly, §2's registry
  term already puts the reads in the child's registry, where the
  derivation looks, so the declaration does its work in `dispatch`'s
  precomputed `turn_taint` (§7). That holds only if the child's `code` was
  built over the child's registry. `build_subagent` hands a child
  `Arc::clone` of the parent's tool, so a cloned `code` would carry the
  *parent's* admitted set and declaration. **`build_subagent` therefore
  constructs a fresh `code` over the child's registry, after every other
  tool is inserted**, the way it already special-cases `kg_task_update`
  (`ClosedStatusGuard::wrap`). §8's allowlist test is what fails if it is
  cloned.
- **At the boundary**, today's rule applies unchanged: an answer that
  parses as a whole as the declared shape returns to the parent not
  `external`, and private data stays private. The parent's interlock, the
  outbox and the leak guard keep doing their jobs on whatever the parent
  does next.
- **The residual, named.** The attacker who wrote a thread still chooses
  *which* allowed answer returns (selection, branch steering). The shape
  bounds that to a choice among the owner's own words. It does not bound
  it to zero, and FIDES's warning applies: this endorses an untrusted
  value. `trusted_output` already made that trade for profiles. A
  `classify` profile in the owner's own `~/.mecha/config.toml`, not in
  this repo, already uses it.

**Two extensions are rulings, not defaults:**

- **R2, a shape the parent chooses per call.** A parent that is unarmed
  when it writes the shape is, by the interlock's own definition, writing
  owner-influenced labels. The recommendation: allow per-call shapes of
  bool, number and one-of (≤ 16 labels) from an unarmed parent; an armed
  parent's shape narrows nothing, since it is already armed.
- **R3, composite shapes at the boundary.** Lists and records multiply the
  bits an attacker selects; a list of 64 booleans is 64 bits.
  Recommendation: not across the boundary in this slice. Composites come
  back `external`, and the parent arms honestly. They are still useful,
  because the context saving stands.

**Pointers (spans, ids) never cross the boundary as trusted.** An id the
attacker selects from their own text can be an address; that is a
`Chosen` destination by another name.

---

## 7. Where it plugs in

- **The gate exists; what is missing is a caller from inside a tool.**
  `Agent::dispatch_one` is documented as the path every call takes, so it
  meets the same gates in the same order: the phase gate, the
  restriction, the trifecta interlock against the taint and any job still
  out, the `pre_tool` hooks, outbox staging, the approval rules and the
  approver. It arms the conversation's taint from the result and mints a
  non-model call id. Three things are new:
  - **A host call is made from inside a running tool, so it cannot call
    `dispatch_one` itself.** `dispatch_one` takes `&mut Conversation`,
    and by the time a tool executes the loop has passed `&mut
    convo.taint` down into `dispatch`. It also mints a `HARNESS_CALL` id
    and re-runs `arm_at_start`, neither of which a mid-turn host call
    wants. So the gate half is extracted (below) as a function over what
    `dispatch` holds. `dispatch` serves a channel on `ToolCtx` for the
    duration of the `code` call and runs each host call through that same
    function. One gate, not a copy of one; §8's first hazard tests assert
    that host calls arrive there.
  - **No approver prompt mid-program.** The loop approves sequentially,
    then executes concurrently, and a program runs inside the concurrent
    phase. A host call that reached a person would put a prompt behind
    tools already executing, under a 600 s clock. Two routes lead there,
    and `read_only()` closes only one of them. (A third, a tool that asks
    inside its own `call`, like `ask_user`, is closed by the effect class
    in §2 rather than here.) It skips the rule-free
    approver branch. But the branch above it fires on `needs_a_person`,
    which `fs_read`, `fs_list` and `document_read` (three of §2's set)
    implement through `ToolCtx::ask_for_path` for a path outside the jail.
    So the host-call path refuses both: a call `needs_a_person` would
    escalate, and a call a `prompt` policy rule matches, each raise in the
    program and never prompt. Existing grants still resolve as they do
    for a model call.
  - **The turn's taint forecast stays honest.** `dispatch` precomputes
    `turn_taint` from declared capabilities before anything executes, to
    close "read a secret and send it in the same turn". A host call's
    taint arriving mid-turn does not defeat that, because `code` declares
    the union of its reads (§6). This, and not the subagent derivation, is
    where the declaration is load-bearing.

  - **The gate is reused; the result path is not.** `dispatch_one`
    renders a result for the context: capped at `output_budget_bytes`,
    spilled past it with `CAP_MARKER`, and wrapped by
    the untrusted-content wrapper that `[security] mark_untrusted_output`
    switches on. A handle over that rendering would be a view
    of truncated bytes plus wrapper tags, and `find` offsets would point
    into the wrapper. The checks and the rendering both live in
    `dispatch`, which `dispatch_one` and `run_tools` wrap. So it is
    `dispatch`'s per-call body that splits into its gate (every check, then
    taint armed from the raw result) and its rendering, and a host call
    takes the gate and the raw `ToolOutput`. The cap, the spill and
    the wrapper apply to what re-enters the context: printed output,
    `peek`, and `str[n]` results. They apply once, to the `code` call's
    whole `tool_result`, as to any tool's: one cap, one spill, and one
    untrusted wrapper when the result is `external` (§7, below). Markers
    are never interleaved inside a program's stdout.

  Moving `dispatch` and its wrappers into `dispatch.rs` is optional tidying, a pure
  move checked by a diffed `cargo test -- --list`. It is not a
  prerequisite.
- **The parent's `code` is built last too.** `setup.rs` registers the
  front-end's own tools (`ask_user`, `recall`, the TUI's) *after*
  `prepare` returns. That comment records the same trap once already, for
  learned-rule scoping. A `code` built inside `prepare` would not reach
  `recall`, which §2 admits and §9 leads with. It would also declare a
  capability union over an incomplete registry, which is exactly the
  under-declaration `turn_taint` depends on not happening. So the
  parent's `code` is constructed after the last registration, in the
  named place in `setup.rs`, and §8 tests that a parent program reaches
  `recall`.
- **The interpreter behind a trait** (`start`, `resume`; `dump`/`load`
  reserved for the sink slice), as `CODE-MODE-RESEARCH.md` asked.
  - monty 1.4 through `monty-pool` is the implementation.
  - The pool is a third process class beside llama-server seats and
    sandboxed MCP servers. It gets the whole of `connect`'s treatment,
    not half: the environment cleared, **and** the configured `Sandbox`
    around the worker, with no network and no filesystem beyond its own
    binary. The interpreter's refusal of `OsCall` and unknown
    `NameLookup`, with no mounts, is the first wall. The sandbox is the
    second, because round 1 of Hack Monty was an escape. `preflight`
    fails the run if a worker will not start inside it.
  - Littrs (0.6.2, Apache-2.0) is a fallback for this slice only. It has
    no pause API but needs none here, and its limits are weaker (no memory
    or time limit outside its WASM mode). wasmtime with a Python guest
    remains the runner-up for the sink slice.
- **Output.** Printed output plus the final expression, under
  `output_budget_bytes` and the spill rule, exactly as a tool result.
- **Recording a program's reads, so grounding can find them.** A host
  call is not a `tool_use`/`tool_result` pair, and a handle keeps its
  bytes out of the transcript. Without a record, everything that walks
  `grounding::calls` loses its referent for what a program read:
  - gossip's citation check (`gossip.rs`);
  - the outbox's "what is this draft answering" join (`outbox_source.rs`);
  - `attribution.rs`'s pack;
  - appraisal's evidence (`appraisal_store.rs`);
  - persona citations (`persona/cite.rs`);
  - and, outside the walk, the diagnostician's carry-over refusal, which
    reads tool results directly.

  A draft staged after an `analyst` run would join to nothing. That fails
  closed, but silently. So:
  - The `code` tool's output carries its host calls: tool, input,
    `external`, and a pointer to the result bytes, in issue order. The
    loop writes them to the session file as a `program` record beside the
    `tool_result`, the same way a compaction writes `rewrite`. The model's
    context never holds them.
  - **The bytes are not in the JSONL.** A 200-thread sweep would put
    megabytes into a transcript that every reader loads whole. Capping
    them there would break `replay_run`, which must answer a replayed host
    call with the bytes the original got. So each result is stored once,
    content-addressed, in a blob store the record points into. The record holds
    its hash and length. A missing or mismatched blob makes that call
    unreplayable and ungrounded, by name, never silently empty.
  - **Not in the spill directory: it is inside the jail.**
    `ToolCtx::inside_jail` admits the session's spill directory as "the
    one sanctioned exception", tested by
    `the_jail_admits_the_spill_directory_and_nothing_else_new`, because a
    cap marker tells the model to go and read the rest there. Blobs there
    would let `fs_list` and `fs_read`, both admitted in §2 and both
    callable by the model directly, read every handle's raw bytes,
    uncapped and uncounted. That would bypass §3's `peek` rule and §5's
    cap completely.
  - **A root per session, under the mecha home.** Spill is also the wrong
    lifetime: its root is keyed by the workspace's hash, and
    `forget::purge_workspace` keeps it whenever another conversation shares
    the workspace, which is the common case. So program blobs live in
    `~/.mecha/programs/<session-id>/`, content-addressed within that
    session only.
  - **Why no jail reaches it, stated exactly.** `ensure_outside_mecha_home`
    refuses a workspace that *contains* the mecha home. It allows one
    *inside* it, and `~/.mecha/work/<producer>/` is the default. So that
    rule alone does not keep blobs out of reach: a workspace at or under
    `~/.mecha/programs/` would hold them inside its jail. The guarantee
    takes one named edit: `work::ensure_outside_mecha_home` also refuses a
    workspace at or under the programs root, exactly as it refuses one
    over the home. Every workspace it accepts then has the programs root
    outside it, a sibling of `work/`. No grant may name a path under the
    mecha home (`grant.rs`), so a grant cannot reopen it. There is
    no cross-session sharing to refcount, and `forget` removes the
    session's directory unconditionally, as a named edit to `forget.rs`
    with a test that a deleted session leaves no blob behind.
  - **Retention, for the sessions nobody deletes.** One program may hold
    64 MB (§5), stored whole for replay, so the store needs a bound on
    growth, not only a deletion rule. Program blobs follow `work.rs`'s
    retention model: a session's blob directory is reaped after the
    retention period of the work directory its run used, and by an
    owner-set size ceiling on the whole store, oldest session first. A
    reaped session's programs become unreplayable and ungrounded by
    name, as a missing blob already is. `doctor` reports the store's size
    against its ceiling, and any directory whose session no longer
    exists, so an orphan is a finding rather than silent disk.
  - **Which list carries them, exactly.** `grounding::calls` takes
    `&[Message]` and borrows each `Call` out of it. Several of its readers
    hold a live conversation, where the only message list in scope *is*
    the model's context. `Session::messages_ever` admits only records that
    yield messages (`Record::Message`, `Record::Rewrite`,
    `Record::Extend`). So the records cannot ride in the message list
    without entering the context.
    They live on `Conversation` beside the messages, as taint does: a
    `program_calls` list, persisted as `program` records and loaded by
    `messages_ever`'s sibling.
  - **That is a change to `grounding.rs`, not a record format.** The walk
    gains a second source (`&[ProgramCall]`), and `Call` gains an owned,
    nested variant under the `code` call that made it, so a handle's
    source and a span inside it dereference. The five readers of the
    walk change to pass it: `attribution.rs`, `gossip.rs`,
    `outbox_source.rs`, `appraisal_store.rs` and `persona/cite.rs`. A
    sixth edit is outside it: the diagnostician's carry-over check in
    `mecha-cli` reads `Block::ToolResult` out of the conversation directly.
    §8's join test is what proves the walk.
  - **Across a compaction cut**, `program_calls` is carried as taint and
    tool state are, never summarised, or a draft staged after the cut
    joins to nothing. §8's join test runs once with a compaction between
    the program and the draft.
  - **The `code` result's own `external`** is true when any host call in
    the program returned external content. That is the same rule that
    armed the taint, so `Dispatched::external` and the provenance map
    agree with it.
  - **Through a subagent there is no child session to hold the record:**
    a child agent writes no session file today. So the subagent tool's
    output carries its child's `program` records up, and the parent's
    session writes them nested under the subagent call. They go in the
    file, never the parent's context, and they serve replay and the
    owner's review.
  - **The walk stops at a `trusted_output` child's shape.** "Never the
    parent's context" is true of the message list but not of what reaches
    a prompt. `gossip.rs` takes `call.result` into a packet a model reads,
    `persona/cite.rs` checks claims against it, and `attribution.rs` and
    `appraisal_store.rs` build their evidence from it. Today a
    `trusted_output` child's reads reach none of them, because a child
    writes no session. That is §6's whole gain: only the declared shape
    crosses. Nested records must not undo it. So `grounding::calls`
    yields a `trusted_output` child's nested calls to no reader by
    default; for those readers, the subagent call's result is the shaped
    answer and nothing under it. Whether the outbox's owner-facing
    "what this draft answers" view may open them is ruling R9. That view
    is read by a person, but any model pass over a draft that reads the
    same join (a forecast, a title) would inherit the prose.
  - `replay_run` answers a replayed program's host calls from the same
    record. Without it, a replay of any session that ran a program
    diverges at the first host call.
- **It is a feature and a lever, not just a tool.** `code` follows
  `ARCHITECTURE.md` §Features → *Adding a feature*: a `Feature` variant,
  off by default, whose `own_state` asks the predicate its registration
  asks. It needs a pinned `monty-pool` worker binary as its sidecar.
  `Feature::Ocr` is the precedent for a feature with a sidecar and a pin;
  `Feature::System` is the precedent for a one-tool feature. It is also a `harness::Lever`
  (`Lever::CodeTool`) that `mecha eval` throws off, with
  `Lever::CompactTool` as the precedent for a tool registered per request.
  A scorecard must not grade a model with programs on unless it says so.
- **Host calls enter the trace.** `ToolCallTrace` is pushed from
  `dispatch`'s per-call body at every refusal and execution, and it feeds
  `RunStats::tool_calls`, `tool_errors`, `tool_denied` and everything
  `runlog` reads. A host call takes the gate half of that body, so it
  pushes a trace entry the same way, tagged `in_program`. Otherwise tool
  use would silently stop being counted the moment a run used a program,
  and `sessions health` would stop being comparable across the release
  that ships `code`, which `POST-TRAINING-RESEARCH.md` §7 step 0 relies
  on. One `code` call can therefore add up to 256 to `tool_calls`.
  `sessions health` splits by the tag, so a program-heavy week reads as
  that and not as a jump in tool use.
- **Counters.** New `RunStats` counters: `programs`, `program_errors`,
  `subcalls`, `subcall_tokens`, `peeked_bytes`, `budget_stops`. Each is an
  `Option<u32>`, as `context_overflows` is, because a row written before
  the sensor existed is unknown, not zero.

---

## 8. Landed when

1. **The hazard tests fail on the naive build and pass:**
   - A tool outside the admitted set cannot be reached by name, by
     `getattr`, or through `NameLookup` — the soundness test §3.12 named.
   - A subagent whose profile grants `code` but not `mail_search` cannot
     reach `mail_search` from a program, though its effect class would
     admit it. A global admitted set passes the test above and fails this
     one.
   - An active skill restriction (`Tool::narrows_surface_to`) that leaves
     out `mail_search` keeps it out of the program's function list, and a
     call to it raises. The restriction is not in the registry: the gate
     checks it per call, and a `code` that built its function list from
     the registry alone would widen it. The same holds for a policy that
     forbids a tool, the third narrowing §2 names.
   - A turn that calls `code` (whose program reads mail) and a `Chosen`
     sender in the same assistant message has the sender refused by the
     precomputed `turn_taint`, because `code` declared the union of its
     reads. With `Capabilities::default()` on `code`, this test fails.
   - A program's `fs_read` of a path outside the jail with no grant
     raises in the program, and no prompt reaches a person.
   - Neither a program nor the model can reach the blob store through
     `fs_read` or `fs_list`. A `fs_list` of the spill directory after a
     program ran shows no blob.
   - A workspace at `~/.mecha/programs/x` is refused at setup. A run in
     the default `~/.mecha/work/<producer>/` cannot `fs_read` a blob by a
     `../` path. Without the refusal, the first assertion fails, so the
     test is not vacuous.
   - `chunks(h, 1)` refuses, and a program that reads past 64 MB of raw
     results raises at the read that would cross it.
   - A Stop during a program returns its `tool_result` within the grace
     period, and no sub-call outlives it.
   - A draft staged after a program read a thread joins to that thread
     through `grounding::calls`.
   - A transcript carrying a `program` record loads in a binary that
     predates it: the record is skipped as unknown, never failing the
     file. It is a wire format, read in `Session::messages_ever`'s loop.
   - After an `analyst` child read a thread, a parent-side `gossip` packet
     and a `persona/cite` pass see the shaped answer and no nested call.
     With the walk yielding child records, this fails.
   - A program in the parent's own run reaches `recall`, which is
     registered after `prepare`, and `code`'s declared capabilities
     include every tool registered before it was built.
   - Each host call pushes a `ToolCallTrace` tagged `in_program`. A
     `pre_tool` hook that denies a host call raises in the program *and*
     increments `tool_denied`.
   - With the feature off, a profile naming `code` fails at launch with
     the message that names the feature, not the typo message.
   - A marker string planted in a source whose pass fails its schema
     twice, in a tool's error body, and in a non-JSON handle given to
     `json_get` appears in none of the three raised messages, and
     `peeked_bytes` is unchanged. Each message opens with its §4 tag. A
     host that quotes the failed output fails the first.
   - `llm_map` over three handles whose second pass fails raises
     `QueryError:` naming index 1 and one completed item.
   - Taint is armed before `resume` on a host call whose result is never
     printed, and the `code` result comes back `external` when any host
     call returned external content, so `Dispatched::external` and the
     provenance map agree.
   - A program that peeks two external handles returns one `tool_result`
     with exactly one untrusted wrapper around the whole result, capped
     once.
   - A handle over a result larger than `output_budget_bytes` holds every
     byte, with no `CAP_MARKER` and no untrusted wrapper, and a `find`
     offset past the budget indexes the source correctly.
   - `llm_query` refuses when `structured_output` is disabled.
   - Each host limit (§5) raises a catchable `LimitError:`-tagged error
     naming the limit. Each backstop ends the program uncatchably, and the
     `tool_result` names the backstop after the program's printed output.
2. **The effect class from `ROLLOUT-DESIGN.md` §3.2** is built and is the
   admission rule; a test asserts `message_send` and `image_generate` are
   absent from a program.
3. **`structured_output` is probed** on the installed build (M0).
4. **The experiment in §10 has run** and its predictions are recorded,
   whichever way they fell.

---

## 9. What it is for

- **Questions over the inbox** that triage does not ask: "which threads
  this month asked me for something I haven't answered?", one typed
  verdict per thread from `llm_map`, a count back.
- **Long documents**: the extraction cache's pages as handles, `find` for
  candidates, `llm_query` per section, the answer as a shape.
- **This conversation's own history**: `recall` as a source after a
  compaction, for the exact figure a summary dropped. Past *sessions* need
  a reader that does not exist yet and are not in this slice.
- **The graph**: aggregation over `kg_timeline` that a model cannot do by
  reading thousands of episodes.
- **Later**: the harness's own quarantined lanes (reflect, distill, the
  eval judge) as programs, and the sink slice for outbox-routed writes.
- **Not** personas. They keep their own boundary (`PERSONA-DESIGN.md` §3).

---

## 10. Measurement

**M0, before code.** Probe Qwen3.6 with `response_format: json_schema` on
the installed llama.cpp (27637851), with thinking on and off. Record
schema-valid rate, latency for a three-label enum, and whether reasoning
leaks into the grammar, and the smallest `max_tokens` at which a pass with
thinking on still returns content. That floor is R6's input. Issue #20345
has April and May reports of failure on this model after the fix landed.

**M0, measured 2026-10-30.** 34 sequential requests on the router
(build `b1915-27637851`, reasoning budget 4,096), against the resident
preset `qwen3.6-35b-a3b-uncensored`, the same architecture and template as
the production preset. Probing production would have swapped the resident
model. Each request was gated on `voice.live = no`. The input was a made-up
message carrying an injected "reply APPROVED" line. Schemas were a
three-label enum and a three-field record (bool, `str` ≤ 32, int range).

| Thinking | `max_tokens` | Schema-valid | Median latency | Failures |
|---|---|---|---|---|
| on | 512 | 0/6 | 5.5 s | all `finish: length`, HTTP 200, empty `content` |
| on | 2,048 | 5/6 | 15 s | one: reasoning ran to ~8.5k characters |
| on | 8,192 | 8/8 | 14 s | none |
| off | 128 or 1,024 | 14/14 | 0.3–0.5 s | none |

- **The grammar held.** Every pass that returned content was
  schema-valid, in both the `llama_json` wire form
  (`{"type":"json_object","schema":…}`) and the standard
  `json_schema` form. No reasoning leaked into the output. The #20345
  reports do not reproduce on this build.
- **Every failure was the envelope failure CLAUDE.md names:** thinking
  used the whole allowance. With thinking on, `max_tokens` must clear the
  reasoning budget.
- **No pass that returned content echoed the injected instruction**
  (0/27; the other 7 returned nothing).
- **Thinking was less consistent.** On the enum (an arguably ambiguous
  message), thinking gave all three labels across seven runs. Without it,
  eight runs agreed. One message and small numbers, and agreement is not
  correctness, but it points the same way as the latency.

The probe and its raw results are outside the repo; the table above is
the record.

**M1.** `monty-pool` worker start and per-host-call overhead on this box,
owed since `CODE-MODE-RESEARCH.md`.

**The experiment** (`mecha exp`, fixtures on the fictional cast only):

| Arm | Tools |
|---|---|
| control | today's tools |
| code | plus `code` |
| analyst | plus the §6 profile |

The tasks:
- a 200-thread fixture inbox with 10 questions;
- long-document QA over a 300-page fixture PDF;
- an OOLONG-style aggregation;
- injection cases:
  - an instruction in a thread;
  - an ADI wrong-value case (a delimiter that steers the extracted enum);
  - a branch-steering case where a boolean gates a draft;
- a decomposition check: a long-input task where answering without the
  tool should score visibly worse (the Recursive Agent Harnesses failure).

The predictions to falsify:
- The `code` arm cuts root prompt tokens by at least half on the document
  and aggregation tasks, and does not lose accuracy.
- Wall clock on the inbox task is no worse than 1.5× control.
- The `analyst` arm leaves the parent unarmed on at least 80% of inbox
  questions.
- Zero composed sends to an attacker address in any arm.
- The branch-steering case **succeeds** in the `analyst` arm at some rate.
  It is the residual §6 names, and the experiment should show it rather
  than assume it away.

---

## 11. Rulings

Ruled by the owner on 2026-10-30, taking the recommendation on each ("go
with your recs"), except R4, which waits on M1.

- **R1. Yes.** The first slice is read-only (§2), and the effect-class
  table is built first and shared with rollouts.
- **R2. Yes, narrowly.** An unarmed parent may choose a per-call shape of
  bool, number, or one-of with at most 16 labels (§6). An armed parent's
  shape narrows nothing.
- **R3. Yes.** Composite shapes come back `external` (§6).
- **R4. Owed.** The budget defaults (§5) are set after M0 (done) and M1
  measure a pass and a worker on this box.
- **R5. Yes.** `peek` exists, capped together with `str[n]` results, and
  counted (§3). The cap's number belongs to R4's budget table (§5), with
  16 KB as its proposed default.
- **R6. Thinking off by default**, on per call (§4, from M0). **Every
  pass gets `max_tokens` of 8,192, thinking or not**, because the
  thinking-off request can be silently ignored (§4).
- **R7. One at a time.** No `llm_map` concurrency above one for now (§4).
- **R8. No.** `llm_query` refuses on any provider that is not local (§4).
- **R9. Yes, conditionally.** The outbox's owner-facing join may open a
  `trusted_output` child's nested program records, but only while no model
  pass reads that join (§7). The build lists every reader of the join and
  asserts none is a model pass.
- **And the `code` call itself is `read_only()`.** A program runs
  without a prompt. The reason is not that its tools would all skip the
  approver: a read-only tool still meets a `prompt` or `forbid` policy
  rule. The reason is that every host call still goes through the full
  gate, policy included (§7). A host call that would prompt raises
  instead, and one that is forbidden is refused. Approving the program as
  a whole would add a prompt in front of checks that already run per
  call. This was
  raised by review pass 17 and decided with the rest. `read_only()`
  controls three things, not one: the approver's rule-free branch,
  `PermissionMode::ReadOnly` (which allows a `read_only()` tool and blocks
  any other), and the planning phase gate (`Phase::allows`, applied in
  `Registry::specs_for` and per call). So a read-only `code` is offered
  and runnable while planning, and in a read-only session. Both are right
  for this slice. **The flag flips when the sink slice admits a sink**,
  and §12 says so.

---

## 12. Deliberately not in this slice

When code lands, the invariants §2–§7 assert move to `ARCHITECTURE.md`, in
a §Code mode section, and this document stops being cited for them. It
is a design, and its list of named edits rots against the tree faster
than an architecture section kept beside the code.

- **Sinks of any kind**, pausing at the outbox, and persisted program
  state. Admitting a sink also flips `code` off `read_only()`, which takes
  it out of `Phase::Plan` and `PermissionMode::ReadOnly` (§11). That
  flip is the mechanism, not just a consequence: a host call skips the
  approver's rule-free branch where `PermissionMode::ReadOnly` is
  checked, so only the `code` call's own flag keeps a read-only session
  from running a program with a sink. The sink slice is §3.12's original
  shape, with snapshots MAC'd, taint carried beside them, and a version
  mismatch failing closed.
- **`Blind` tools.**
- **Value-level labels inside the program** (CaMeL's capability tags).
  monty does not track data flow, and filtering it by hand is the approach
  `CODE-MODE-RESEARCH.md` rules out.
- **Depth above 1.**
- **Training** the model on its programs. `POST-TRAINING-RESEARCH.md` §7
  names it as a later target.

---

## 13. Adoption risk: will the model use it?

**Raised by the owner on 2026-10-30, and ruled: build the feature, test it,
then refine.** The concern is that the local model will never call `code`,
or will call it and write programs that do not work. Nothing below gates
the build. It is what testing should look at first.

**The evidence for the concern.** Measured over the non-test sessions
on 2026-10-30, 767 of them:

| Tool | Calls | Sessions |
|---|---|---|
| `recall`, `compact`, `skill` | 0 | 0 |
| the `research` subagent | 48 | 31 |
| `todo` | 84 | 23 |
| `shell` running Python | 44 of 319 shell calls | |

The three tools never called were not registered in every session, so
their zero is measured against an unknown denominator (how many
sessions offered them), so it is not a refusal rate.
The pattern still reads plainly:
- Tools that serve the harness have never been chosen.
- A tool named for a task is chosen routinely.
- The model already writes Python when it has a shell.

The literature agrees:
- `RLM-RESEARCH.md`: Qwen3-Coder needed its prompt changed, and Qwen3-8B
  needed post-training.
- Prime Agent reports friction allocating subagents.
- Recursive Agent Harnesses saw the parent skip spawning.

**What to do about it, in order of cost:**

1. **Name entry points for the task, not the mechanism.** Profiles such
   as `ask_inbox` and `ask_document` (§6) whose descriptions say what
   they answer, the shape `research` already proves this model uses. A
   bare `code` reads like `compact` and `recall`, which it has never
   called.
2. **Offer it at the moment of need.** When a read would spill past
   `output_budget_bytes`, the result can carry a handle and one line
   saying `code` can query it. The line rides in a tool result, so the
   cached prefix is untouched.
3. **Entry points the owner writes**, where no model choice is involved:
   a trigger, a skill, or a slash command that calls the analyst directly.
4. **M2, once the slice runs:** on fixture tasks that need aggregation,
   measure *choice* (how often `code` is picked when offered) and
   *competence* (whether its programs run and answer correctly). The
   §10 experiment's decomposition check is the same question at
   experiment scale.
5. **Post-training, only if competence is what fails.** That would be
   the first real target `POST-TRAINING-RESEARCH.md` has found; its step 0
   found none in the current corpus. The precedent is RLM-Qwen3-8B,
   about 1,000 filtered trajectories from a stronger model writing
   programs in the same harness.
