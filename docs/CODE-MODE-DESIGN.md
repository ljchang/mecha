# Code mode, reshaped by RLM — design

**Status: 2026-10-30, designed, nothing built. Rulings owed (§11).**

This supersedes the *shape* of `AUDIT-RESEARCH.md` §3.12 ("programmatic
tool calling, on monty"). That section's contract, its two hazards and its
tests stand and are not restated. `CODE-MODE-RESEARCH.md` remains the
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
  hurt the open model, and an unbudgeted open model made thousands of
  sub-calls.
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
`read_only()` means "skips the approval gate", not "has no effect".

What the slice gives up, and why that is the right trade:

- **No sinks**, so §3.12's hardest machinery is not needed yet: approving a
  capability set, pausing at an outbox gate, and a `dump()` carrying
  locals and taint across a release. Sends stay ordinary tool calls in the
  loop, where the interlock already works.
- **No `Blind` tools.** `web_search` is a `Read` by effect, but its query
  leaves the machine, and a query a program composes from private reads is
  exactly what the leak guard refuses. Research over the web waits for a
  later slice.
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
| `lines(h)`, `split(h, pattern)`, `chunks(h, n, overlap=0)` | list of Handle | |
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
  | short text | `str[n]` | n ≤ 2,000 |

- **A `str[n]` result is prose entering the program**, so it is counted
  with `peek` against one cap (§5); otherwise it would be a second,
  uncounted route around §3.
- **`str[n]` and record fields of it never cross a trust boundary.**
  `AnswerShape` deliberately has no bounded-string shape, because
  "ignore previous instructions" fits in very few characters: a length
  cap vouches for nothing. Here it is a program-internal shape that
  changes no label. It must not become a `trusted_output` shape (§6).
- **`structured_output` must be on.** A provider with it `Disabled` makes
  `llm_query` refuse with a typed error, never fall back to parsing prose
  by hope: the silently-degrading-guard rule.
- **Results carry the union of their sources' labels.** A shape bounds how
  much an attacker can say; it does not change who said it. Inside a
  program nothing narrows, which is why this slice needs no security
  ruling.
- **A failure raises.** A pass that fails the schema twice raises
  `QueryError`, catchable in the program. It never returns a sentinel
  string: DSPy's `"[ERROR] ..."` string is the kind of value a program
  aggregates as data.
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
  today. Until the owner rules otherwise, `llm_query` refuses on any
  provider that is not local, by the same rule as `structured_output`.
- **`max_tokens` sits above the reasoning budget** for every pass, as the
  local clients already require. A tight allowance for a three-label enum
  with thinking on is the HTTP-200-with-empty-`content` failure, and a
  pass refuses that by name, as the clients do, rather than reading it as
  an answer.
- **Thinking**: the default is the server's. The ruling waits on M0 (§10),
  because llama.cpp issue #20345 has reports of grammar plus thinking
  failing on Qwen3.6 after the March fix.

`llm_map` runs its passes under `asyncio.gather`, **one at a time by
default**. The bound is the host's own, because nothing else bounds it:
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

Any concurrency above one is ruling R7, and it is bounded across runs,
not per run. **An interactive run's sub-calls stay at one**: they take no
permit, as interactive work never does, and so they are never parallel
either. Otherwise the owner would queue behind their own fan-out, or the
fan-out would be unbounded. For background runs, `DEFAULT_BACKGROUND_PERMITS`
counts three, and each could fan out. So a background run's pass beyond
its first takes a seat from the same permit directory the background runs
share, and waits when none is free. The total of running background passes then never
exceeds `router::background_seats`. In practice that means a background
`llm_map` runs one at a time in the steady state: at the pool's designed
occupancy every seat is already held by a run, so a second pass has
nothing to take. R7 decides whether that is the policy or a floor. On a
model with two or more slots that
keeps the owner's reserve whole. On a one-slot model it cannot:
`seats_for` is never below one, and `permit.rs` concedes the owner can
wait behind one background decode there.

---

## 5. The budget

Per program, enforced by the host, every limit a typed error the program
can catch:

| Limit | Default (ruling R4) | Enforced by |
|---|---|---|
| sub-calls | 32 | the host, **per pass**: `llm_map` over N handles is N, and a schema retry is another. An `llm_map` longer than the remaining budget raises before its first pass |
| sub-call input tokens | 400k | the host, from `Usage` |
| tool reads | 256 | the host, counted at the gate |
| handle operations (§3) | 2,048 | the host; monty `max_suspensions` set to the sum as a backstop |
| wall clock | 300 s | `monty-pool` `request_timeout`, which is **unset by default** and must be set |
| memory, allocations, recursion | monty's limits | the pool worker |
| `peek` bytes plus `str[n]` result bytes | 16 KB together | the host, one count (`peeked_bytes`) |

**The limit a well-behaved program should hit first is the sub-call
cap.** It is the expensive resource and the one the model's prompt names.
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
tools = ["code", "mail__mail_search", "mail__mail_get_thread", "document_read", "docs__docs_read", "graph__kg_search", "graph__kg_timeline", "recall"]
max_turns = 12
trusted_output = true
answer_shape = ["yes", "no", "unclear"]
```

The names are a default install's (`{server}__{tool}`, `prefix_tools`
defaulting true). A box whose graph server sets `prefix_tools = false`
registers `kg_search` bare and spells it so. `build_subagent` fails the
launch on a name that resolves to nothing, rather than running without it.

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
    tools already executing, under a 300 s clock. Two routes lead there,
    and `read_only()` closes only one of them. It skips the rule-free
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
    into the wrapper. So `dispatch_one` splits into its gate (every check,
    then taint armed from the raw result) and its rendering, and a host
    call takes the gate and the raw `ToolOutput`. The cap, the spill and
    the wrapper apply to what re-enters the context: printed output,
    `peek`, and `str[n]` results. They apply once, to the `code` call's
    whole `tool_result`, as to any tool's: one cap, one spill, and one
    untrusted wrapper when the result is `external` (§7, below). Markers
    are never interleaved inside a program's stdout.

  Moving `dispatch_one` into `dispatch.rs` is optional tidying, a pure
  move checked by a diffed `cargo test -- --list`. It is not a
  prerequisite.
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
    content-addressed, in the session's spill directory (the place spill
    already puts oversized results, outside the jail). The record holds
    its hash and length. A missing or mismatched blob makes that call
    unreplayable and ungrounded, by name, never silently empty.
  - **A root per session, not per workspace.** The spill root is keyed by
    the workspace's hash, and `forget::purge_workspace` keeps it whenever
    another conversation shares the workspace, which is the common case.
    Uncapped mail and document bytes there would survive `sessions delete`
    and break its "every trace of it" promise on the largest new
    artifact. So program blobs live under `programs/<session-id>/` inside
    the spill root, content-addressed within that session only. There is
    no cross-session sharing to refcount, and `forget` removes the
    session's directory unconditionally, as a named edit to `forget.rs`
    with a test that a deleted session leaves no blob behind.
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
    file, never the parent's context. A draft the parent stages after an
    `analyst` answer then joins to the threads the child read, not just to
    the shaped answer.
  - `replay_run` answers a replayed program's host calls from the same
    record. Without it, a replay of any session that ran a program
    diverges at the first host call.
- **It is a feature and a lever, not just a tool.** `code` follows
  `ARCHITECTURE.md` §Features → *Adding a feature*: a `Feature` variant,
  off by default, whose `own_state` asks the predicate its registration
  asks. It needs a pinned `monty-pool` worker binary as its sidecar, with
  `Feature::Ocr` and `Feature::System` as the precedent for a one-tool
  feature with a sidecar and a pin. It is also a `harness::Lever`
  (`Lever::CodeTool`) that `mecha eval` throws off, with
  `Lever::CompactTool` as the precedent for a tool registered per request.
  A scorecard must not grade a model with programs on unless it says so.
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
   - A turn that calls `code` (whose program reads mail) and a `Chosen`
     sender in the same assistant message has the sender refused by the
     precomputed `turn_taint`, because `code` declared the union of its
     reads. With `Capabilities::default()` on `code`, this test fails.
   - A program's `fs_read` of a path outside the jail with no grant
     raises in the program, and no prompt reaches a person.
   - A Stop during a program returns its `tool_result` within the grace
     period, and no sub-call outlives it.
   - A draft staged after a program read a thread joins to that thread
     through `grounding::calls`.
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
   - Each budget limit stops a program with its typed error.
   - A `pre_tool` hook that denies a host call raises inside the program.
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

## 11. Rulings owed

- **R1.** The first slice is read-only (§2), and the effect-class table is
  built first and shared with rollouts.
- **R2.** Per-call shapes chosen by an unarmed parent (§6).
- **R3.** Composite shapes come back `external` (§6).
- **R4.** The budget defaults (§5).
- **R5.** `peek` exists, capped and counted (§3).
- **R6.** Sub-call thinking, after M0 (§4).
- **R7.** Any `llm_map` concurrency above one, and how much of
  `router::background_seats` it may draw (§4).
- **R8.** Whether `llm_query` may run on a remote provider, which sends
  past today's output cap (§4).

---

## 12. Deliberately not in this slice

- **Sinks of any kind**, pausing at the outbox, and persisted program
  state. The sink slice is §3.12's original shape, with snapshots MAC'd,
  taint carried beside them, and a version mismatch failing closed.
- **`Blind` tools.**
- **Value-level labels inside the program** (CaMeL's capability tags).
  monty does not track data flow, and filtering it by hand is the approach
  `CODE-MODE-RESEARCH.md` rules out.
- **Depth above 1.**
- **Training** the model on its programs. `POST-TRAINING-RESEARCH.md` §7
  names it as a later target.
