---
title: Compaction
sidebar_position: 3
description: Making a long conversation fit — eviction first, a legal cut, a validated summary, and what must survive all of it.
---

# Compaction

Every turn sends the whole history, so a session that runs long enough stops
being able to send anything at all. Compaction replaces the middle of the
transcript with a summary and keeps the ends: the task at the top, so the agent
still knows what it was asked, and the most recent turns verbatim, because that
is where the work is.

It is **on wherever the provider declares a context window**: the threshold
derives from that window (two thirds of it) unless `compact_at_tokens` sets one
directly, and only a provider with no declared window runs uncompacted.
Compaction is lossy, which is why it is validated and recorded rather than
left off.

```toml
[agent]
compact_at_tokens = 22000     # explicit
compact_keep_recent = 6       # turns kept verbatim
compact_validate = true       # on by default
loop_guard = true             # on by default
predictive_compaction = true  # on by default: also fire on the forecast
carried_state = true          # on by default: carry the plan verbatim
```

```bash
mecha run "..." --compact-at 22000
```

## Knowing when to compact

The threshold is checked between turns against two readings, and either one
fires it. The first is what the provider **reported** for the last request —
not an estimate over the message list, so it counts cached tokens too and
tracks the real prompt.

That reading is a turn out of date: by the time the check runs, the assistant
turn and a batch of tool results nobody has priced are already in the
transcript. So the second is a **forecast** of the next request — the last
reported size plus the bytes added since, converted at the token-per-byte rate
measured between the last two reports and clamped into the range a real
tokenizer can occupy. It is arithmetic on two measurements: no tuned parameter
and no model call. The check is `reported || predicted`, never the forecast
alone, so it can only make compaction fire *earlier* than the reported size
would, never later.

The forecast is on by default. `predictive_compaction = false` in `[agent]`, or
`--no-predictive-compaction` on a run, leaves only the reported size — the
threshold itself stays. It needs a first response to anchor on, so it starts
empty on every run, which in `mecha chat` and the TUI means every user turn.

You can set it directly with `compact_at_tokens`, or let it derive: unset, the
threshold is 0.66 of the provider's `context_window`.

`[providers.X] context_window` is how many tokens the model's context holds —
for llama-server, **`-c / -np`**, confirmed by `n_ctx_slot` at startup.
Ordinary model responses report usage, not capacity; `mecha setup` can probe
the local server and save the setting. Setting it turns
compaction from something you must remember to configure into something that
works, and the failure it prevents is total rather than gradual — one turn over
the window and the server refuses the request outright.

Two thirds, not nine tenths, because the check still happens *between* turns.
The request it clears must also fit the model's reply, the forecast is an
estimate at a measured rate rather than a count, and with the forecast turned
off, whatever a burst of parallel tool results added since the last report is
not seen at all. Leaving a third of the window free is what makes that safe.

Other things read `context_window` too: the TUI status line becomes a fuel
gauge (`context 29.3k/32.8k (89%)`, yellow at 75%, red at 90%) instead of a
number with nothing to compare to, the per-turn tool-output budget is sized
from it, and whether the [`compact` tool](#the-model-can-ask-for-it) is offered
depends on there being a threshold at all. Overflow recovery does not need it —
it keys on the server's refusal — but with no window and no
`compact_at_tokens`, that refusal is the only thing that ever compacts. If you
change the server's `-c`, change `context_window` to match — a stale value is
worse than none, because the derived threshold trusts it.

An [eval case](/docs/features/experiments/evaluation) can set its own
`compact_at_tokens`, so a case that means to exercise compaction does not ask
every other case to compact too.

## The model can ask for it

Wherever a run has a threshold, the model also gets a `compact` tool. It takes
no arguments and rewrites nothing itself: it sets a flag, and before the next
turn the loop runs exactly what a threshold crossing runs — the free passes
below, then the summary. The summary always runs on a request, because the
request is by design made while the transcript is still under the threshold,
and re-asking "is it over?" there would answer no after the model was told
it would be summarised.

The description tells the model to call it at a boundary — after finishing a
step of its plan, before starting the next — because that is the one thing the
model knows and the harness does not: how much of its own plan is left. *How*
to compact stays the harness's: the cut point is chosen by the same pure code
either way.

It can only ever **add** a compaction; the threshold still fires on its own, so
nothing the model reads can make a run compact later. It is not gated on
approval — compaction is lossy but not destructive, and the states a rewrite
replaces are [still recorded](#what-a-rewrite-replaced-is-still-recorded).

`--no-compact-tool` withholds it (the run still compacts at its threshold), as
does leaving `compact` out of `[tools]` — in `disabled`, or an `enabled` list
or `--tool` allowlist that does not name it. `mecha eval` forces it off with the
rest of its levers, `predictive_compaction` and `carried_state` among them, so
a scorecard does not depend on this machine's config.

## The order of operations

When the threshold is crossed, the loop does the cheap and lossless things
first and only pays for a summary if they were not enough.

### 1. Evict superseded results

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
- **It counts toward "freed enough" by what it actually freed, no more.** The
  loop re-measures after the passes, so a collapse that removed repetition but
  little bulk simply leaves the transcript over the threshold and the summary
  runs. It is always enough to write a `rewrite` record, because the
  transcript really did change.

Distinct from [the loop guard](#a-compaction-arms-the-loop-guard), which stops a
run that has already gone wrong and only after a compaction. This runs before
there is anything to stop.

### 3. Thin old results, keep the calls

Older tool results — everything before the last `compact_keep_recent` turns —
are cut to their first 240 characters; the calls themselves are kept whole.
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

**Then the loop asks again, in the same turn.** The passes rewrote the list the
reported size measured, so that number is retired and the forecast answers
against the transcript as it now is. If the three passes between them brought
it back under the threshold, no summary is paid for — a summary is lossy where
thinning is merely lossy about the middle of a file. With
`predictive_compaction` off there is no fresh reading, and unknown falls toward
the summary: it runs unless a new report says otherwise. A model's `compact`
request skips the re-ask, for the reason [above](#the-model-can-ask-for-it).

### 4. Summarise the middle

Only if the transcript is still too big.

## The cut has to be legal, not convenient

A `tool_result` whose `tool_use` is gone is a **400**, and that is the whole run.

Tool results arrive in the user message immediately after the assistant turn
that asked for them, so the only safe place to resume is at an **assistant
message**. Cutting there drops each `tool_use` together with the results
answering it.

When there is no legal cut — normal for a short conversation — nothing is
compacted, and that means "not yet" rather than "something is wrong". The
original task is always kept, and a cut that would drop fewer than four
messages is not made, because below that the summary is likely longer than
what it replaces.

The summary is appended to the *original task message* rather than
inserting a message of its own — two user messages in a row are rejected by some
providers, and the task and the summary of what happened to it belong together:

```
[Earlier turns were compacted to fit the context window. What happened in them:]
…
```

And the rebuilt transcript is checked for orphaned tool results **before it is
installed**, not after. A guard that fires once the damage is done is not a guard. The caller treats an
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

Any tool can hand state to the compaction to be kept **verbatim**. It is
placed after the summary, because it is the one part
of the rebuilt head known to be current rather than paraphrased, and last is
where a model reads most carefully:

```
[Live state, carried past the compaction and current as of now — it supersedes
anything about it in the summaries above:]

## todo
1. [x] read the transcripts
2. [ ] write the report
```

That header is a sentinel, not a convention. The next compaction finds the
previous carried block by it and **replaces** it, so exactly one copy survives a second
compaction: there is only ever one *current* state, and keeping the old copy
would be keeping a wrong one. The summary is replaced the same way. The
summariser is shown the whole stretch it is compacting, the earlier summary
included, and is told to fold that summary into the new one — so a long
session carries one summary, not a growing stack of them, and the validator
checks the new summary against a rendering that still holds the old one.

The loop learns that some tools have state, never which one. `carried_state =
false` (or `--no-carried-state`) is an experiment's lever: the summary still
installs, the plan just does not ride across it. See
[Tools and MCP](/docs/features/tools).

## The summariser gets prose, not a replay

The stretch being compacted is rendered as labelled prose, not sent as
messages. Sending the real messages means sending `tool_result`s on a request that
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

The rendering labels each line with who said it, and harness text that rides
in a user message — a peer's delivered message, a boredom notice, a plan-step
nudge — is labelled `[harness]`, not as the owner. Labelled by role alone, the
summary would say the owner had said them, and it lands in the task message
where it outlives every turn it describes; `mecha distill` reuses the same
rendering.

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

The threshold cannot always prevent an overflow: a turn's parallel tool
results land all at once, so the size last reported can sit well under the
limit while the *next* request is well over. The forecast narrows that gap but
cannot close it — it has no anchor on a run's first request, and it is off in
`mecha eval`.

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
[Evaluation](/docs/features/experiments/evaluation).

## Taint survives compaction

Summarising away the *text* of a hostile page does not un-read it, and the
model's context is still downstream of it.

Taint belongs to the conversation, not to its messages, so rewriting the
messages cannot drop it.

The same is true in the other direction for the session record: the taint
checkpoint written after a run reflects everything that entered the
conversation, compacted or not. See
[Sessions and replay](/docs/features/memory/sessions-and-replay) and
[Security](/docs/features/security).

## What a rewrite replaced is still recorded

Compaction, eviction and thinning rewrite the conversation in place, but the
session file still gets everything: what each rewrite replaced is recorded
alongside the rewritten state. A run long enough to compact itself keeps its
whole history on disk, where the `recall` tool can search it — see
[Sessions and replay](/docs/features/memory/sessions-and-replay).

## The cache lens

Prompt caching only pays if every request is a prefix of the next, and a
regression there is invisible — requests succeed and every turn quietly
re-pays for the whole history. So each run watches its own cache reuse and
logs a warning when a large part of the previous prompt had to be paid for
again with nothing — tools, system prompt, or transcript — having changed. A provider that has never reported a cache figure
is never accused. The model and the loop never see these verdicts.

## When a compaction fails

A failed summary is not a reason to abandon the run — the oversized request
might still succeed, and if it does not, the provider's own error is clearer.
But the loop **stops trying**: each attempt is a request of its own, and
retrying a failure every turn would cost more than the compaction was going to
save.

A compaction interrupted mid-summary leaves the transcript alone. A half-written
summary is worse than an oversized conversation, and the run is ending anyway.
