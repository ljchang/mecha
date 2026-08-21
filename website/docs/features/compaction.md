---
title: Compaction
sidebar_position: 18
description: Making a long conversation fit — eviction first, a legal cut, a validated summary, and what must survive all of it.
---

# Compaction

Every turn sends the whole history, so a session that runs long enough stops
being able to send anything at all. Compaction replaces the middle of the
transcript with a summary and keeps the ends: the task at the top, so the agent
still knows what it was asked, and the most recent turns verbatim, because that
is where the work is.

It is **off by default**. Compaction is lossy, and paraphrasing someone's
conversation because it got long is their decision to make.

```toml
[agent]
compact_at_tokens = 22000     # explicit
compact_keep_recent = 6       # turns kept verbatim
compact_validate = true       # on by default
loop_guard = true             # on by default
```

```bash
mecha run "..." --compact-at 22000
```

## Knowing when to compact

The threshold is measured against what the provider **reported** for the last
turn, not against an estimate over the message list — so it counts cached
tokens too, and tracks the real prompt.

You can set it directly, or let it derive:

```rust
pub const COMPACT_FRACTION: f64 = 0.66;

pub fn compact_at(&self, context_window: Option<u64>) -> Option<u64> {
    self.compact_at_tokens.or_else(|| {
        context_window.map(|w| (w as f64 * Self::COMPACT_FRACTION) as u64)
    })
}
```

`[providers.X] context_window` is how many tokens the model's context holds —
for a local server, the `-c` it was started with. **Nothing can discover it**: a
provider reports what a prompt *cost*, never what is left. Setting it turns
compaction from something you must remember to configure into something that
works, and the failure it prevents is total rather than gradual — one turn over
the window and the server refuses the request outright.

Two thirds, not nine tenths, because the check happens *between* turns against
what the last one reported. The next request still has to fit the model's reply
and whatever a burst of parallel tool results adds. Leaving a third of the
window free is what makes a reactive check safe.

Two other things read `context_window`: the TUI status line becomes a fuel
gauge (`context 29.3k/32.8k (89%)`, yellow at 75%, red at 90%) instead of a
number with nothing to compare to, and overflow recovery has something to aim
at. If you change the server's `-c`, change `context_window` to match — a stale
value is worse than none, because the derived threshold trusts it.

A per-run override exists (`RunContext::with_compact_at`) for the same reason
the budget and the path jail are per-run: one agent serves many runs, and an
eval case that means to exercise compaction cannot ask every other case to
compact too.

## The order of operations

When the threshold is crossed, the loop does the cheap and lossless things
first and only pays for a summary if they were not enough.

### 1. Evict superseded results

```rust
let evicted = crate::compact::evict_superseded_results(messages);
```

This runs first at both compaction sites — the threshold check and overflow
recovery — because it is **the only pass that removes damage rather than
trading tokens for fidelity**.

A superseded read is semantically *related* to the current state of the work
and *wrong* about it. That is measurably worse than irrelevant bulk:
related-but-wrong distractors cost 25–68% where unrelated content is near-free.
A transcript holding two versions of the same file is exactly that shape, and
deleting the old one is lossless — the newest result still says everything the
transcript knows to be true.

What counts as the same target:

- **A string `path` argument, across tools.** An `fs_write` supersedes an
  earlier `fs_read` of the file it just changed, which is the case the
  distractor research names directly. The target is deliberately *not* prefixed
  with the tool name: the newest operation on a path speaks for the path,
  whichever tool performed it. A *ranged* read is different — `offset` and
  `limit` join the key, or reading lines 100–110 would evict a full read of the
  same file, and successive range reads (exactly what the spill marker tells the
  model to do) would evict each other while holding different content.
- **Otherwise, the tool name plus its exact arguments.** The model asked the
  same question twice and the newer answer speaks for both.

**Errors neither supersede nor get evicted.** A failed call says nothing about
the target's state, and "what failed" is what keeps it from being retried.

The evicted result is replaced by a marker that names the recovery, rather than
one that only says "gone":

```
[stale: a later fs_write call covered the same target, so this older result no
longer reflects it. The newest result is authoritative; call fs_read again if
this content is needed.]
```

The marker also lets a second pass tell it has already been here.

### 2. Collapse repeated failures

```rust
let collapsed = crate::compact::collapse_repeated_failures(messages);
```

Eviction's error exemption is right for one failure and inverts for eight. A
model is measurably likelier to fail a step when the context holds its own
earlier errors — self-conditioning, which does not go away with model size — and
a repeated failure is the same-target near-miss the distractor literature prices
at 25–68%, not the free kind of bulk.

Nothing in the harness touched these before: eviction skips errors by
construction and thinning only truncates long results, so a sixty-character
failure message was untouchable by both.

The diagnosis the error exemption protects is carried by the **newest** failure
alone, so that one survives verbatim and the older identical ones become
markers:

```
[repeat: this call failed again later with the same error, which is kept in
full below. Repeating it unchanged has not worked.]
```

The marker names what happened *and what it means* — one that only says
"collapsed" invites the model to try once more to see for itself.

Four rules:

- **The key is target *and* exact error text.** "No such file" then "permission
  denied" on one path are two facts, and collapsing either loses a diagnosis.
  Collapsing too little costs tokens; collapsing too much destroys information,
  so narrow is the fail-safe direction.
- **Nothing is removed.** Dropping a `tool_result` block is a 400. The content
  is replaced, the block stays.
- **Refusals are never collapsed.** A denied call carries `is_error: true` like
  any failure, so keying on that flag alone would fold a *human's* refusals
  together. Results beginning `Denied by the user:`, `Blocked by policy:` or
  `Blocked by a hook:` are skipped — those are the strings the
  [learning miner](/docs/features/learning) reads a correction out of, and compaction
  rewrites the transcript in place, so folding three refusals into one marker
  destroys the evidence rather than merely undercounting it.
- **It does not count toward "freed enough, defer the summary".** It removes
  repetition rather than bulk, so treating it as freed space would spend a turn
  arriving back at the same threshold. It *is* enough to write a `rewrite`
  record, because the transcript really did change.

Distinct from [the loop guard](#a-compaction-arms-the-loop-guard), which stops a
run that has already gone wrong and only after a compaction. This runs before
there is anything to stop.

### 3. Thin old results, keep the calls

```rust
let thinned = crate::compact::thin_old_results(
    messages,
    self.cfg.compact_keep_recent.max(1) * 2,
    crate::compact::THINNED_RESULT_CHARS,   // 240
);
```

A call and its result differ enormously in both size and value:

```text
tool_use    {"path": "entry-9e1b.md"}          ~15 tokens  ← the position
tool_result "# Audit entry 11\namount: 43…"    ~80 tokens  ← the bulk
```

Position lives in the calls, which are tiny. Tokens live in the results, which
are not. Replacing the middle of a transcript wholesale throws away both, which
is why a summarised traversal loses its place — the agent can no longer see
which entries it already visited. Thinning keeps that sequence *structurally*,
so it does not depend on a summariser noticing it mattered. It costs no
request, and an already-thinned result is left alone so repeated passes do not
eat the head a chunk at a time.

**If eviction or thinning freed anything, the summary is deferred a turn** —
collapsing does not count, for the reason above. The next reported prompt size
says whether that was enough, and a summary is lossy where thinning is merely
lossy about the middle of a file.

### 4. Summarise the middle

Only if the transcript is still too big.

## The cut has to be legal, not convenient

A `tool_result` whose `tool_use` is gone is a **400**, and that is the whole run.

Tool results arrive in the user message immediately after the assistant turn
that asked for them, so the only safe place to resume is at an **assistant
message**. Cutting there drops each `tool_use` together with the results
answering it.

```rust
fn is_safe_cut(messages: &[Message], i: usize) -> bool {
    messages.get(i).is_some_and(|m| m.role == Role::Assistant)
}
```

`cut_point` searches forward from the target for the first legal index, and
returns `None` when there is none — normal for a short conversation, and it
means "do not compact" rather than "something is wrong". Index 0 is the
original task and is kept regardless. `worth_compacting` refuses a cut that
drops fewer than four messages, below which the summary is likely longer than
what it replaces.

`compact.rs` is deliberately pure and provider-free, and unit-tested for
exactly this. Getting the boundary wrong produces a 400 from a real API twenty
turns into a real session, which is the worst possible place to discover it.

`rebuild` appends the summary to the *original task message* rather than
inserting a message of its own — two user messages in a row are rejected by some
providers, and the task and the summary of what happened to it belong together:

```
[Earlier turns were compacted to fit the context window. What happened in them:]
…
```

And the rebuilt transcript is checked **before it is installed**, not after:

```rust
let orphans = crate::compact::orphaned_tool_results(&rebuilt);
anyhow::ensure!(
    orphans.is_empty(),
    "refusing to compact: it would have orphaned {} tool result(s)",
    orphans.len()
);
```

A guard that fires once the damage is done is not a guard. The caller treats an
error here as "carry on uncompacted", which is survivable; carrying on with a
transcript the API will reject is not.

## A tool's own state is carried, not summarised

The measured failure mode is that a summariser preserves *what is true* and
drops *how far you got*. Some of "how far you got" does not live in the messages
at all — it lives in a tool. For that state a summary is the wrong mechanism
twice over: it is lossy, and the tool already holds the exact current answer.

The `todo` list was the case that proved it. It reached the model only through
the echo in the last `todo` result, which is a message, and therefore precisely
what a compaction summarises away — so the plan evaporated in the one situation
where a long run needs it most.

`Tool::carried_state` lets any tool hand state to the compaction to be kept
**verbatim**. `rebuild` places it after the summary, because it is the one part
of the rebuilt head known to be current rather than paraphrased, and last is
where a model reads most carefully:

```
[Live state, carried past the compaction and current as of now — it supersedes
anything about it in the summaries above:]

## todo
1. [x] read the transcripts
2. [ ] write the report
```

That header is a sentinel, not a convention. `rebuild` finds the previous
carried block by it and **replaces** it, so exactly one copy survives a second
compaction — summaries accumulate on purpose, each describing a different
stretch, but there is only ever one *current* state and keeping the old copy
would be keeping a wrong one.

The loop learns that some tools have state, never which one. See
[Tools and MCP](/docs/features/tools-and-mcp).

## The summariser gets prose, not a replay

```rust
let rendered = crate::compact::render_for_summary(&messages[..cut], 2_000);
```

Sending the real messages means sending `tool_result`s on a request that
declares no tools, and llama-server answers that with an empty completion.
Found by running it, not by reading the spec. Prose has no such failure mode on
any provider, and it also removes any chance of the summariser deciding to call
something.

The summariser gets a **different system prompt** from the agent's own, which
would tell it to use tools and invite it to resume the task instead of
describing it:

```
You compress a transcript. You do not act on it, use tools, or answer the task
it describes. You return prose and nothing else.
```

The instruction is written for the agent that will read the result, not for a
human. It asks for the specific values, paths, names and numbers — those cannot
be recovered once the text replaces the transcript — what was tried and failed,
what remained, and, explicitly, **where in a sequence the work had got to**:

> Being told a fact is not the same as knowing your place in the work, and
> losing your place is how a traversal silently restarts or stops early.

It also asks the summariser to say when a fact came from content a third party
could have written: the distinction survives compaction even when the text does
not.

The summariser has **its own token budget** (8192), not the agent's. Tying them
was measured to kill runs: at `[agent] max_tokens = 4096` the summariser hit its
limit mid-summary, the truncation guard correctly refused it, the run gave up
compacting and died of context pressure — 2/5 on the same case in both arms of a
validation run.

## Summaries are validated before they install

Two layers, in order.

**Truncation is refused deterministically.** A summary that came back with
`stop_reason: max_tokens` lost its ending, which is where "what remained to be
done" lives. Free to check, unlike anything a validator can say.

**Then a second, tool-less call reads the summary beside the transcript it
replaces** and lists what is missing. It is asked only about *omission*, because
that is how summaries actually fail — measured here, the summariser preserved a
stated fact 3/3 while losing the traversal position 4/5, and measured elsewhere
around 90% of compaction failures are omissions. Asking a checker to critique
style invites rewrites; asking what is missing invites a list, which is what the
retry needs.

A finding triggers **one** regeneration, with the omissions named:

```
A check of your previous summary against the transcript found it omitted the
following. The rewritten summary must include them:
- the audit total established in entry 7
- which entries had already been visited
```

Naming them is the whole intervention. The producer cannot see its own gaps; a
bare "try again" would sample the same blind spot. A failed retry keeps the
first summary — validated-with-known-gaps beats empty or truncated.

The verdict parser treats a whole line saying "none" as a pass, and substrings
do not count: "none of the paths survive" is a finding, not a pass.

**This is not a completion gate.** An unusable verdict, a failed validator call,
or an interrupted one all install the summary with a warning, because a run
that needs to compact to survive must still compact. It is also not an LLM judge
scoring quality — it is a grounded comparison of two texts both present in the
request. Costs one extra request per compaction, two when a regeneration is
needed. `compact_validate = false` turns it off.

## Overflow recovery

The reactive threshold cannot always prevent an overflow: a turn's parallel tool
results land all at once, so the size checked between turns can sit well under
the limit while the *next* request is well over.

`is_context_overflow` recognises the refusal across backends by message text —
no backend gives it a usable code — and the loop compacts and retries **the same
turn, once**. All three cheap passes run first, exactly as at the threshold:
they cost no request, so there is never a reason to skip them. A false positive costs one summary; a false negative loses the
whole run, which is what used to happen.

Recovery differs from the between-turns pass in one way: it thins with
`keep_recent = 0`. The request does not fit, so *something* must shrink, and in
the common shape — a short conversation holding one enormous tool result — the
oversized result **is** the recent tail. Protecting it there protects the run to
death. Measured, not hypothetical: a capped 48 KB `seq` output still overflowed
a 32k window, and the tail-protecting recovery retried the same request into the
same 400. A thinned result can be re-fetched; a dead run cannot.

A second overflow means compaction did not free enough, and the provider's own
error is clearer than looping on it.

## A compaction arms the loop guard

```rust
Ok(Some(spent)) => {
    usage.add(&spent);
    compactions += 1;
    loop_guard.arm();
}
```

An identical tool call with an identical result, repeated within a window of
three calls after any compaction, stops the run with `StopCause::Loop`.

Distinct from `MaxTurns` on purpose: "hit the turn limit" reads as the task
being too big, when a stuck run is a different problem with a different fix.

The guard is **dormant until a compaction arms it**. Repeated calls in ordinary
work are the model's business, and a general repeated-call detector would need a
measurement to justify watching all of it. This one exists to escape a specific
failure — the run re-living what a summary dropped — at the largest prompts it
will ever send.

Two details that keep it honest. It is keyed on **call *and* result**, so
polling (same arguments, changing result) never trips it. And it observes a
**turn** rather than a call: a model emitting the same call twice in one
parallel batch is being wasteful, not stuck, and killing that run would grade
waste as a loop. The loop this catches is across turns.

Gradeable via `expect.stop_cause: "loop"` in the eval rig. No shipped case
asserts it, because a case cannot reliably make a model loop, and a case that
asserts an outcome it may never exercise is worse than no case. See
[Evaluation](/docs/features/evaluation).

## Taint survives compaction

Summarising away the *text* of a hostile page does not un-read it, and the
model's context is still downstream of it.

Taint lives on `Conversation`, alongside the messages. The compaction code
operates on `&mut Vec<Message>` and never sees the `Conversation` at all — the
type is doing the work, not a rule someone has to remember. There is a test.

The same is true in the other direction for the session record: the taint
checkpoint written after a run reflects everything that entered the
conversation, compacted or not. See
[Sessions and replay](/docs/features/sessions-and-replay) and
[Security](/docs/features/security).

## What a rewrite replaced is still recorded

Compaction (and eviction, and thinning) rewrite the message list in place,
and the front-end records a run only when it finishes — so for a long time a
run that compacted *itself* lost its own head: the rewrite record carried
only what survived. The states a rewrite replaces now ride on the
`Conversation` (`rewritten`, cleared at run start), and the session
recording walks them before the final state. A run long enough to compact
itself still gets its whole history into the file, where the `recall` tool
can search it — see [Sessions and replay](/docs/features/sessions-and-replay).

## The cache lens

Prompt caching is a prefix match, and everything protecting the prefix is an
invariant somewhere else: the registry's ordering, the append-only
transcript, the fixed system prompt. A regression in any of them presents as
nothing at all — requests succeed, answers arrive, and every turn quietly
re-pays for the whole history. The bill is the only symptom.

So each run carries a pure observer: it fingerprints every request as
actually sent, compares it with the one before, and names the reason when
cache reuse legitimately breaks (tool surface changed, transcript rewritten
by compaction). The one remaining shape — a large re-payment with nothing
changed — is a warning in the logs. Two honesty rules keep the warnings
believable: a provider that has never reported a cache figure is never
accused (zeros are silence, not a miss), and small re-payments stay below
the alarm. Verdicts go to tracing only; the model and the loop never see
them.

## When a compaction fails

A failed summary is not a reason to abandon the run — the oversized request
might still succeed, and if it does not, the provider's own error is clearer.
But the loop **stops trying**: each attempt is a request of its own, and
retrying a failure every turn would cost more than the compaction was going to
save.

A compaction interrupted mid-summary leaves the transcript alone. A half-written
summary is worse than an oversized conversation, and the run is ending anyway.
