---
title: Sessions and replay
sidebar_position: 4
description: Append-only JSONL transcripts, what they record beyond the messages, and replay as a standing regression check.
---

# Sessions and replay

Every run writes an append-only JSONL transcript. That file is the record: what
was asked, what the model did, what the tools returned, what had entered the
conversation, and what the run was configured with. Everything downstream —
resuming, `reflect`, `distill`, and `replay` — reads it rather than a second
copy that could disagree with it.

```bash
mecha sessions list
mecha sessions show 20260805T091500
mecha sessions path 20260805T091500
mecha sessions stats --days 30      # what runs cost
mecha sessions health --days 30     # how runs went
```

Transcripts live in `~/.mecha/sessions` (override with `MECHA_SESSION_DIR`), in
a directory created owner-only. `--no-session` opts out.

## The file

One JSONL file per session: a header line, then one line per record. Append-only,
so a crashed run still leaves a readable transcript. Ids are
`20260805T091500-3f2a1b7c` — sortable by name, and still unique when two runs
start in the same second.

Nine record kinds:

```json
{"record":"meta","id":"20260805T091500-3f2a1b7c","created_at":"...","provider":"anthropic","model":"claude-opus-5","workspace":"/home/you/project","title":"summarize what changed"}
{"record":"config", ...}
{"record":"message","role":"user","content":[{"type":"text","text":"..."}]}
{"record":"taint","private":true,"untrusted":false}
{"record":"rewrite","messages":[...]}
{"record":"goal_anchor","goal":"epic:7"}
{"record":"title","title":"summarize what changed"}
{"record":"summary","usage":{...},"turns":4}
{"record":"outcome","turns":4,"stop_cause":"end_turn","tool_calls":11,"tool_errors":1,"tool_denied":0,"ended_on_failed_call":false,"compactions":0, ...}
```

The `rewrite` record is how an append-only file expresses an in-place edit:
compaction, eviction, and thinning all rewrite earlier messages, and slicing
"what the run added" off a rewritten list would record a lie — the stale
head kept, the rebuilt one lost. The record carries the whole current list,
and loading replaces what was accumulated so far. The states a rewrite
*replaced* are recorded too: the loop keeps each pre-rewrite message list on
the conversation, and the end-of-run recording walks them before the final
state — so a run long enough to compact itself still gets its whole head
into the file.

A `goal_anchor` record follows each recorded run and names the goal that run
was anchored to, so a resumed conversation keeps its goal. A `title` record
renames the conversation as it grows; the last one wins over the header's
title.

`load` skips unparseable lines rather than failing — a truncated final line is
the normal result of a killed process. A file whose first record is not a
header is not a session mecha wrote, and is skipped.

`mecha sessions list` reads only each file's first line, so listing stays fast
however large the store grows.

### recall: the record is searchable

Sessions recorded by `chat`, the TUI, and resumed runs register a `recall`
tool: a case-insensitive search over the *union* of everything the
transcript ever recorded — including the messages a compaction rewrite
replaced. When a summary drops the one detail the run later needs, the
model looks it up instead of re-running tools or re-living the stretch.

Two properties make it safe to hand to the model. It is **taint-neutral by
construction**: everything it can return entered this conversation once,
and that arrival is what armed the interlock — taint never un-arms, so
re-surfacing recorded content changes nothing the interlock knows. And the
**transcript path is fixed at registration**, never taken from model input,
so no other conversation's content is reachable. It is deliberately absent
from Slack (one shared registry serves every thread; a per-run insert would
point one thread's recall at another's transcript) and from fresh one-shots
and triggers, whose per-run record is empty until the run ends.

### The taint record

Taint is recorded because **it cannot be recovered by reading the transcript
back**. Taint keys off *provenance* — whether a result actually came from
outside the machine — and the transcript stores only content. Without the
record, resuming a session that had read a hostile page would hand the model
that page again with the interlock disarmed.

Every front end appends a taint checkpoint after the messages of the run it
describes. On load, checkpoints are **merged rather than replaced**: taint only
ever grows, a later clean checkpoint cannot disarm an earlier armed one, and a
transcript written by an older build simply has none.

`Session::taint_timeline` positions those checkpoints against the messages. The
checkpoint covering a message is the first one written *after* it — and by then
the taint of everything earlier in that run has merged in. That ordering is what
makes it safe to gate on: it can **over-taint a message, never under-taint
one**. A message with no checkpoint after it returns `None`, which the caller
must treat as *unknown*, and unknown provenance is never clean. This is what
`mecha learn` uses to exclude non-clean reflections structurally — see
[Learning](/docs/features/learning).

### The outcome record

`summary` answers *what did this run cost*; `outcome` answers *did it work*.
They are two records rather than more fields on one because the audience is
different — cost is for a person reading `sessions show`, and the outcome is for
a machine reading a thousand sessions at once.

It carries the stop cause, whether a budget was reached, tool calls attempted
against errors, denials and stagings, malformed arguments, blocked sends,
compactions taken, the end-of-run taint, and whether the run stopped of its own
accord with its last call failed. Written by **every** front-end: before it
existed, an interactive run was measurably less observable than a trigger, whose
ledger already recorded most of this.

Two counters that must not be added together: `tool_errors` is the environment
refusing, and `tool_denied` is a human or a policy refusing — which is the
harness working. Everything downstream keys on that split.

`mecha sessions health` reads these back across the store, and the loop built on
top of them is [Run quality](/docs/features/learning/run-quality).

```bash
mecha sessions health --days 30
```

### The config record

A `config` record says what the run was configured with, so it can be replayed:
the mecha version, provider and model, workspace, the resolved system prompt
text, the tool list in registry order (and a fingerprint of the tool
definitions), effort, thinking, the temperature and seed actually sent,
`max_tokens`, every budget and ceiling, the compaction settings, the permission
mode, the trifecta policy, the sandbox, the active harness levers, and the
workspace and surface used to match learned rules.

The rule behind that list: **anything that shapes the request or constrains the
run is a confound if it is not recorded.** A replay that did not know whether
compaction was on, which permission mode denied a call, or which sandbox
narrowed `shell` would compare two incomparable runs and report a model
regression.

The system prompt is stored in full rather than hashed, so a replay can rebuild
the request; it is no more sensitive than the transcript beside it. A new
config record is written each time a process attaches to the session, because a
resumed session may run under different flags. The sampler is recorded only as
far as it was pinned — no temperature or seed means the server chose, and the
run is not exactly repeatable.

### The summary record

`Record::Summary { usage, turns }` is written when a run finishes, so
`sessions show` and `sessions stats` can report cost without replaying the
transcript. `usage_totals` sums every summary in a file; a transcript that
predates the record or died before writing one totals zero — an honest
under-count, never a guess.

`mecha sessions stats` rolls that up by provider and model, priced at *today's*
configured rates. The transcript records tokens, not prices, so historical runs
are re-priced rather than remembered — the table says so. A provider with no
configured prices shows `—` rather than `$0.00`; a local model with no prices
really does cost nothing, and only rows with a price claim a dollar figure. A
torn transcript still contributes what it recorded.

## Replay

```bash
mecha replay 20260805T091500
mecha replay 20260805T091500 --on-divergence=error --json
mecha replay 20260805T091500 -p anthropic          # same work, another model
```

Replay re-drives a recorded session with model calls and tool results taken
from the recording. In the default `stop` mode, replayed tool calls do not
execute their underlying tools. Model requests still cost tokens, and setup
can connect configured MCP servers.

The result is a controlled comparison over recorded evidence, with limits:
replay reapplies output limits and untrusted-content warnings, so the bytes
shown to the model can differ from the original transcript. Modern recordings
preserve per-call provenance. Legacy results with unknown provenance count as
external, including old harness refusals; this can add a warning or a second
warning envelope. In `live` mode it can also block a send the original allowed.
The CLI reports this, and JSON includes `legacy_provenance_calls` and
`provenance_note`. Compare arms under the same replay policy before attributing
a difference to the model.

Recordings that dispatched harness plan checks cannot yet be replayed or used
for trace-based counterfactual probes. Their check observations are part of the
decision context, and replay cannot reconstruct them yet. These comparisons
return an explicit unsupported result; independent artifact-task grading remains
available.

### How the run is rebuilt

From the session's `RunConfig`, not from today's flags: system prompt, tool
list, effort, thinking, budgets, compaction settings. If a session has several
config records, replay uses the first and prints a note. A session with no
config record cannot be replayed.

In `stop` and `error` modes, saved tool schemas and descriptions take precedence
when the surface store still holds the recording's `tools_hash`. They can also
stand in for tools no longer available. If neither setup nor a recorded or
supported display-only surface can supply a tool, replay refuses.

`live` mode uses today's tool definitions and requires executable tools, because
it can actually call them after divergence. Recorded descriptions never grant
capabilities or permissions to a live tool.

Provider and model default to the recorded ones and can be overridden. Replaying
one model's session on another is how you compare them on real work — and when
`-p` names a different provider, the model defaults to *that* provider's own,
because sending the recorded name would name a model the other server does not
serve.

### Extraction

`replay::extract` reduces a transcript to a `Trajectory`: the user's turns, every
tool call paired with its recorded result, and the final assistant text.

The distinction doing the work: **a user message carrying `tool_result` blocks
is the harness feeding results back, not the user saying something.** Treating
those as turns would replay a conversation with twice the turns and none of the
same structure. Results are matched to calls by id rather than position, because
calls are issued in parallel and nothing promises the results come back in
order.

Text sitting *alongside* tool results is mid-run steering, and it sets
`trajectory.steered`. Steering rides in the same user message as the results it
accompanies (there is no legal slot between a `tool_use` and its result), which
makes it indistinguishable from a turn once flattened, and re-submitting it as
one would change the shape of the conversation being replayed. It is flagged
rather than silently dropped, and `mecha replay` prints a note:

```
note: the recording was steered mid-run; steering cannot be re-injected, so the
comparison is approximate
```

### Divergence

A replay can depart from its recording in four ways:

| Divergence | Meaning |
|---|---|
| tool | the model called a different tool entirely |
| arguments | the right tool, with different arguments |
| extra | the replay kept going after the recording ran out |
| missing | the replay stopped early |

The comparison preserves order **between assistant turns**. Within one recorded
parallel batch, calls may arrive in a different order. Matching prefers the same
tool and arguments, then the same tool name; each result keeps its own provenance.
Legacy calls without batch markers remain positional.

Argument differences are reported separately and do not stop replay. The same
file can have different path spellings, but changed arguments can also mean a
different action. Replay returns the matched recorded result and leaves that
judgment to the reviewer; an argument mismatch is not proof of equivalence.

`--on-divergence` decides what happens at a structural divergence:

| Mode | Behaviour |
|---|---|
| `stop` (default) | end the run there — after a divergence, every later recorded result answers a question nobody asked |
| `error` | the same, and exit non-zero on *any* divergence, argument spellings included |
| `live` | abandon the recording and continue against the real tools |

Underlying tool calls do not execute in `stop` or `error` mode. `live`
falls back to the configured permission mode: real tools run after the
divergence and deserve exactly the scrutiny they always get.

### A replayed episode comes back gradeable

The report carries the replayed episode's outcome counters — the same
`RunStats` a live run records — alongside the calls and the final text. Without
them a replay was gradeable only by a divergence diff, which answers "did it do
something else" and not "did it go better".

That is what lets a replayed corpus be one arm of the
[candidate gate](/docs/features/learning/run-quality#the-gate): each episode names itself, produces
a cost, and is paired against the same episode in the other arm. Note the limit
this arm has by construction — replay holds the tool results fixed, so it cannot
see a change in *what the model said*. A prose change needs the
`eval --ab-config` arm instead.

### What replay is not

**A probe's verdict is not stored.** `mecha sessions appraise --probe` replays
from each steer and derives its verdict on demand; the transcript stays the
record, and rerunning the probe recomputes it.

**Replay against a non-greedy provider is pass@k-shaped, not
exact-match-shaped.** A local server's sampler is outside this process's
knowledge, and the same case measures 5/5 rather than deterministically. One
divergent replay is a sample, not a regression.

A replay is also **never less armed than the recording**. Each recorded tool
result carries its provenance (`tool_provenance` on the message), and replay
passes every call's `external` marking through unchanged; a result whose
provenance is unknown — a recording made before the field existed — counts as
external. Replay can therefore over-taint a legacy recording, never
under-taint one. Refusals the interlock produced at record time were recorded
as results, so they replay verbatim regardless.

## The standing regression check

`scripts/replay-regression.sh` replays a set of pinned sessions against the
current build and fails on any divergence.

```bash
scripts/replay-regression.sh              # replay every pinned session
scripts/replay-regression.sh <id> [...]   # replay just these
```

Pins live in `~/.mecha/regression-sessions.txt`, one session id per line —
**machine-local on purpose**, because transcripts are personal data and do not
belong in the repository.

Adding a pin means recording a session that uses only built-in tools, verifying
it replays clean once, and appending its id:

```bash
mecha run -p local --no-mcp --no-learned-rules \
  --tool fs_read --tool fs_list -w eval/workspace "<task>"
```

Built-ins only, because an MCP surface makes a pin break whenever a server is
rewired — which is drift, not regression.

The script refuses to run unless llama-server is on one slot (`-np 1`). Seeded
replay is only repeatable sequentially against a single slot; continuous
batching makes concurrent requests perturb each other's numerics, seed or no
seed. Refusing beats reporting fake divergence.

A pin that diverges means the harness — prompt assembly, tool dispatch, request
shape — or the model changed. Read the JSON before deciding which.
