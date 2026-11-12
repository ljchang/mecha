---
title: Memory
sidebar_position: 1
description: Existing memory stores and opt-in encrypted episodic retention for ordinary runs and CLI conversations.
---

# Memory

The default runtime uses **three memory stores**, with distinct trust models:

| Memory | Holds | Trust model | Write path |
|---|---|---|---|
| **The knowledge graph** | The world: people, projects, events, facts | **Untrusted on read** — third-party text by construction | Agent writes are staged candidates; a person promotes them |
| **The [learning store](/docs/features/learning)** | Behaviour: rules mined from your corrections | Provenance-gated — only clean-origin reflections become rules (a tainted session still yields clean ones from your own typed words; triage rules carry a narrow exemption) | `reflect` → consolidation from a `session_end` hook (`scripts/learn-live.sh`), self-applied behind a measurement gate (probation when nothing can be graded); a nightly sweep catches the rest; `--propose` stages for review instead |
| **The [session transcript](/docs/features/memory/sessions-and-replay)** | What was actually said and done, verbatim | The record itself; `recall` re-surfaces it taint-neutrally | Append-only, rewrites recorded with what they replaced |

The separations are load-bearing. A learned rule rides in the cached prefix of
every future run whose situation matches its scope — a longer-lived injection path than anything else in
the system — so learning is gated on provenance and never fed by machine
policy. The graph is fed by mail, messages, and pages other people wrote, so
reading it must arm the trifecta interlock. The transcript is your own
record, so re-reading it can be free. Collapse any two of these and one
trust model has to lie.

## Experimental episodic memory for ordinary runs

A build with `memory-prototype` can attach encrypted episodic storage to a fresh
`mecha run` through `--episodic-memory`. It retains attributed observations and
reuses `goal_context` for source-checked recall across sessions. It does not
replace mecha-graph, the learning store, or ordinary session JSONL, and does not
automatically schedule concept learning or rumination.

### The home store and the `memory` switch

A store can live in two places. The examples below name one with a manifest
(`--request FILE`, `--episodic-memory FILE`), which is how a disposable or
experimental store is made. The other is **the home store**, the one mecha
keeps for itself:

```bash
mecha memory-session init
```

With no `--request`, `memory-session` works on the store under the mecha home:
`memory/store/agent.memory-prototype.sqlite`, `memory/key/agent.key` and
`memory/authority/`, each in a directory only you can read. Those three paths
are fixed. No configuration file can move them, so a cloned repository cannot
say where your memory or its key is. `init` refuses a home that already holds
any part of a store. **Keep a copy of the key somewhere else**: the database
is encrypted with it, and a store whose key is lost cannot be opened.

`[features] memory = true` (`mecha features enable memory`) says you want the
home store. `mecha features` then reads the row from those files: **on** when
all three are there, **unready** with `mecha memory-session init` when none
is, and unready with a repair instruction when only some are or they cannot be
read. In a build without `memory-prototype` the row reads as not in this
build.

**The switch does not attach memory to a session yet.** A run or chat still
attaches a store only through `--episodic-memory FILE`, under the profile
described below. Attaching the home store to ordinary sessions is the next
change, and this page will say so when it lands.

**Retention is separate from success.** A tool returning successfully or an
assistant claiming completion does not establish that a task succeeded. The
optional JSON `memory` report identifies retained, pending or unavailable capture,
with `semantic_assessment: "not_evaluated"`. The task's ordinary answer and outcome
remain separate.

A one-shot run is remembered with whatever tools, servers, hooks, grants and
inbound messages it is configured with. It needs a confirmed goal, a fresh
session, and `goal_context` among its tools; mecha installs the recall tool
under that name. The recorder stores the owner's request, what the model
wrote, each tool call and each result, with where each came from. What it has
no passage for is left out and never refuses the run: an image, a late result
from a deferred tool, a message delivered from another agent, a notice from
the harness, the rewrite a compaction makes. A run that was cancelled or ran
out of turns is remembered as far as it got.

A grant is the one configuration memory checks. A grant that contains the
store, its key, its authority directory or the session's transcript is
refused, at the start and again whenever the run recalls, because the jail
is what keeps those out of a tool's reach.

Leaving something out has one cost. When the run's recorded taint says
untrusted content entered and the recorder met something it could not place,
everything the model wrote in that run is stored as outside content, so
recalling it later arms the interlock. A run whose recorded taint is clean
keeps its passages local.

One run is one capture of at most 256 passages and 1 MiB. A longer run keeps
the request, the model's last text and as many calls from the start as fit; the
transcript still holds every line. A chat still uses the restricted profile
below: the tools `fs_read`, `fs_write`, `fs_list` and `goal_context`, no MCP,
code tool, grants, hooks, fallbacks, subagents or outbox routes, and held or
disabled inbound messages.
Assistant reasoning stays in the original transcript, covered by its source
digest, but is never imported as memory evidence. Reasoning is never silently
disabled; `--no-thinking` remains a provider-dependent explicit control.

Keep the manifest, database, key and private authority outside every task
workspace. The database filename must end in `.memory-prototype.sqlite`.
Initialization requires new paths and refuses overwrites. For a
disposable example, start from the source checkout:

```bash
cargo build -p mecha-cli --features memory-prototype
MECHA_MEMORY_BIN="$PWD/target/debug/mecha"
MECHA_MEMORY_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/mecha-memory.XXXXXX")"
MECHA_MEMORY_ROOT="$(cd "$MECHA_MEMORY_ROOT" && pwd -P)"
mkdir -m 700 "$MECHA_MEMORY_ROOT/db" "$MECHA_MEMORY_ROOT/keys" \
  "$MECHA_MEMORY_ROOT/work-a" "$MECHA_MEMORY_ROOT/work-b"
python3 - "$MECHA_MEMORY_ROOT" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
request = root / "memory.json"
request.write_text(json.dumps({
    "protocol": "ordinary-session-memory-v1",
    "db": str(root / "db" / "episodes.memory-prototype.sqlite"),
    "key_file": str(root / "keys" / "episodes.key"),
    "authority": str(root / "authority"),
    "domain": "local-demo",
    "account": "agent-demo",
    "scopes": ["agent-experience"]
}))
request.chmod(0o600)
PY
"$MECHA_MEMORY_BIN" memory-session --request "$MECHA_MEMORY_ROOT/memory.json" init
cd "$MECHA_MEMORY_ROOT/work-a"
"$MECHA_MEMORY_BIN" --no-learned-rules --no-past-appraisals \
  --no-success-examples \
  --workspace "$MECHA_MEMORY_ROOT/work-a" \
  run --episodic-memory "$MECHA_MEMORY_ROOT/memory.json" \
  --goal project:memory-demo "Remember that this demo uses a blue indicator."
```

Use a configured fixed provider, or explicitly pin the provider/model if the
configuration follows a router. Change into `work-b`, repeat the invocation with
`--workspace "$MECHA_MEMORY_ROOT/work-b"` and a new goal, and ask it to query
`goal_context` with
`memory.query: "blue indicator"`. Retrieval remains evidence, not an instruction
or a verified belief. Legacy goal-context lessons and appraisals cannot be
silently certified as v2 provenance; the example explicitly disables those
older consumers. Ordinary invocations without this opt-in are unchanged.

`memory-session --request FILE retry RUN_ID` retries only admission of a durable
capture; it never repeats the action or calls a model. `forget RUN_ID` withdraws
the episode and purges its dependent memory from the selected database. Both
commands work without the old task directory. Deleting or editing the original
JSONL withholds its memory, but is not a substitute for explicitly purging stored
copies. Keep the selected key and authority for recovery. Failed partial
initialization preserves created files for explicit reconciliation.

**What a recall does to the interlock.** Each stored passage records where
it came from: your own words, a local tool result, or something read from
outside mecha. Recalling memory always counts as private data. It counts as
untrusted content only when the recall includes something from outside,
something whose origin was never recorded (every passage stored before this
was recorded, and imported transcripts), or something a model derived, such
as a concept. What the model wrote after outside content entered its
conversation is stored as outside too. So a session that recalls only your
own notes and local results can still make an outbound call, and one that
recalls a fetched page cannot. Records returned with `"external": false`
are the ones stored as local.

SQLCipher encrypts the database. Source JSONL, private recovery payloads, task
artifacts and retained copies are separate filesystem protection surfaces.
Current qualification uses authored data; it does not establish general learning
benefit, private-data readiness or corpus-scale performance.

### Conversations with changing goals

The separate experimental chat profile uses the same initialized manifest and
encrypted store. From an isolated workspace, keep the restricted flags and tool
selection above, and replace the `run` command with:

```bash
chat --dynamic-goals --episodic-memory "$MECHA_MEMORY_ROOT/memory.json"
```

This requires an interactive terminal and session recording. Owner messages can
change direction between turns; the owner-authored charter is refreshed for each
decision. Each retained turn records which context versions and original source
passages it used. A request is context evidence, not an automatically confirmed
success criterion. Retention remains `not_evaluated`.

To resume, use the same workspace, manifest, restricted flags and tool selection,
adding `--resume SESSION_ID` to that chat command. A different authority, competing
writer, incomplete turn or unsupported unflagged resume refuses before inference.
The CLI prints a separate `episodic chat memory` JSON report after each turn;
save its `run_id` for the existing host-only `retry` and `forget` commands.
Admission retry never repeats actions. Forgetting remains possible when the
original transcript is missing, although that transcript can no longer supply
recall evidence.

Later conversation turns do not invalidate earlier observations: each turn seals
its own immutable transcript prefix. Changing an earlier prefix invalidates the
evidence that depends on it. Recalled material remains a dependency rather than
being imported again as independent corroboration. Historical replay uses the
recorded owner and charter context, not today's charter.

This first chat profile is bounded. It refuses `/clear`, grant changes,
compaction, context-overflow recovery and unfinished or noncompleted capture.
A failed or oversized capture can follow an executed action; its report is
unavailable and further memory-enabled turns stop. Only complete seals can be
retried; an ambiguous unsealed turn requires a fresh chat. Native transcripts
are limited to 32 MiB, and the inherited passage and context limits
can be reached sooner. In both one-shot and chat capture, tool arguments above
1,024 serialized JSON bytes retain their full-input digest, native source pointer
and action receipt with an explicit oversized status; the memory record omits
the argument content. This does not make omitted text factual support or prove
the action achieved its goal. Full arguments remain in the native transcript.
A tool result or other passage longer than 4,096 bytes is stored as its first
4,096 bytes, marked as an excerpt with the full length.
TUI memory attachment and automatic learning remain separate work. The distinct
authenticated web profile is described below. SQLCipher does not encrypt the native transcript, terminal history
or private recovery files.

### Durable todos and planning in CLI memory chat

The separate task-chat profile stores the owner's task and the model's versioned
plan in Mecha's encrypted database. Restart restores the database plan, including
check declarations for unfinished items dropped from the visible list. The owner
task's title, intent and lifecycle remain separate from model-authored progress.
Marking every step complete does not close the task or verify its success.

First enable planning explicitly on the initialized store:

```bash
mecha memory-session --request /absolute/path/memory.json enable-planning
```

This upgrades ordinary schema v8 through task v9 to plan v10. Each step is atomic;
retry continues from the surviving version. Opening or starting a chat never
performs the migration implicitly. A restored historical database cannot become
live task authority through this command.

Create a private owner-authored JSON file outside the workspace, with exactly
`{"title":"Prepare the demo","intent":"Organize and check the demo work."}`.
Use the same isolated workspace, manifest and restricted flags as the ordinary
chat profile. Select both `goal_context` and `todo`; optionally include
`fs_read`, `fs_write` and `fs_list`. Add:

```bash
chat --dynamic-goals --episodic-memory /absolute/path/memory.json \
  --memory-task /absolute/path/task.json
```

The task file establishes the owner container; tell the agent what to do in chat.
It is not silently inserted as owner speech or charter content. Resume with the
same manifest, workspace and exact tool selection:

```bash
chat --dynamic-goals --episodic-memory /absolute/path/memory.json \
  --resume SESSION_ID --memory-task-resume
```

Resume cannot replace the task draft or select a different TaskRef. The CLI shows
the stored plan before accepting another turn. Plans are bounded to 32 items,
512 UTF-8 bytes per content/expect/check field, 256 preserved declarations and
32 KiB per snapshot. Exceeding a bound refuses; it never evicts old checks.

This first profile supports local file/context tools and durable `todo` only.
Graph MCP, shell, code/program tools, delegation, compaction and background jobs
are excluded. Declared checks are preserved but **Skipped**, never executed or
reported as passing. Existing approval and taint gates still apply; a durable
todo is a private-data write. Structured todo calls/results remain in typed plan
revisions and native audit; generic episodic recall does not expose that state.
Independent ordinary observations continue to be retained and recalled.

A database commit and a complete native audit are distinct. If final recording
fails, use `memory-session --request FILE retry RUN_ID`. Recovery may verify and
sync already-complete authenticated bytes, reconcile the original receipts and
finish retention. It never reruns a model/tool or invents missing results. Missing,
partial or conflicting source evidence remains pending and blocks continuation,
including a turn with no todo writes. Startup prints its session ID; an interrupted
task attachment has the separate storage-only repair command:

```bash
mecha memory-session --request /absolute/path/memory.json \
  retry-task-start SESSION_ID --task /absolute/path/task.json
```

That repair requires the original exact draft and startup evidence. It neither
adopts an arbitrary task nor truncates a damaged transcript. Task/plan storage does
not migrate the existing graph task board, mail task capture or web task routes;
those consumers still need explicit adapters. SQLCipher does not encrypt native
JSONL, and forgetting a task in the database does not promise to erase independent
prose or copies in the session transcript.

### Read-only owner-graph access in CLI memory chat

Add `--memory-graph-server NAME` to explicitly select one enabled owner-global
mecha-graph MCP entry. Omit `--no-mcp` in this mode. Retain the other restricted
flags and select `goal_context` plus the exact local names of the graph readers
needed for this conversation. The supported remote tools are `kg_search`,
`kg_entity` and `kg_task_list`; graph writes remain unavailable.

For an existing owner-configured server named `graph` with tool prefixes enabled:

```bash
"$MECHA_MEMORY_BIN" chat --dynamic-goals \
  --episodic-memory "$MECHA_MEMORY_ROOT/memory.json" \
  --memory-graph-server graph \
  --no-code-tool --no-grants --no-messages --no-hooks --no-fallback \
  --no-outbox --no-learned-rules --no-past-appraisals --no-success-examples \
  --tool goal_context --tool fs_read --tool graph__kg_search
```

Use the same workspace and options for `--resume SESSION_ID`. The attachment
binds the selected launch configuration and actual tool definitions; changes
refuse before inference rather than silently widening the conversation. This
profile does not start unrelated configured MCP servers. It preserves the
existing sandbox policy; selected Docker launches are not supported because
executable resolution inside the image cannot be bound by this adapter.
Direct bwrap launches use the same frozen, filtered environment for startup
checks and execution. This selected profile refuses bwrap scoped resource limits
and dynamic-loader environment controls; it never silently ignores them.

Retained evidence is the exact bounded text delivered by the query at that time,
with its source pointer, action receipt and external-content provenance. It is
not a synchronized copy of the owner graph, a verified fact, or a promise to
propagate graph-side deletion. Read-only domain queries may still write graph
retrieval telemetry. Unsupported/nontext responses become external failures;
results above the existing capture limit withhold retention and stop the chat.
Host retry/forget do not need a running graph server. SQLCipher's protection
boundary remains the database, not the separate JSONL or recovery files.

Historical `replay --on-divergence stop|error` compares against recorded answers;
it does not reconnect the attachment or check today's source authority. `live`
divergence refuses memory-marked recordings. Use authenticated chat resume for
continued execution. Authored tests qualify this integration; a private-data
trial and evidence of decision benefit remain separate work.

### Authenticated web conversations

The experimental build can expose an explicit memory chat option in the existing
owner-authenticated web interface. Initialize the encrypted store as above, build
the web assets, and start the server with a separate fixed memory profile:

```bash
"$MECHA_MEMORY_BIN" serve --dynamic-goals \
  --episodic-memory "$MECHA_MEMORY_ROOT/memory.json" \
  --memory-provider CONFIGURED_PROVIDER --memory-model CONFIGURED_MODEL \
  --memory-tool goal_context --memory-tool fs_read \
  --assets web/dist
```

Replace the provider/model placeholders with your configured names. Keep the
existing web owner authentication; this adds no anonymous or loopback bypass.
The memory-prefixed options apply only to memory chats. Ordinary chats retain
their existing behavior and model following. Each memory conversation owns its
runtime, workspace and memory reader, while sharing the selected encrypted store.

Choose **Memory +** to start a new conversation with this profile. It does not
convert an existing conversation. Typed input is preserved exactly, with a
2,048-byte limit per message. The model and tool selection remain fixed; uploads,
voice, permission changes, queued input and steering are unavailable in this
initial profile. The configured permission policy still applies to selected tools;
selecting a write tool does not grant approval by itself.

The banner distinguishes running, retained and blocked states. Retention does
not establish task success: evaluation remains `not_evaluated`. A refused or
uncertain send keeps the draft and requires explicit resume before another send.
After a server restart, use the memory-labeled saved session to resume. The server
reauthenticates its source chain and next turn; reconnect never creates an ordinary
replacement. A resumed connection can be ready without displaying an earlier
retention receipt.

Stop preserves the partial native record and blocks continuation. An incomplete,
unsealed turn requires a new conversation. Complete sealed captures can use the
existing host-only retry procedure without repeating actions; the blocked web
runtime releases its lease so repair does not require stopping the server. Then
explicitly resume. Archive/delete controls are unavailable for memory sessions
until they can withdraw derived evidence consistently; host-only forgetting
remains separate from deleting the native transcript.

This first web profile supports local file tools and `goal_context`. Graph MCP
memory remains a CLI opt-in; the separate owner-graph dashboard is unchanged.
Todo/planning continuity, compaction, TUI memory and automatic chat learning are
still separate work. Encryption covers the memory database, not the source JSONL
or private recovery files. Authored integration checks are not evidence of model
benefit or private-data deployment readiness.

### Explicit concept learning

The same experimental build provides host-only `memory-session sources` and
`memory-session learn`. These operate on the home store, or with `--request` on
the store a manifest names; they do not schedule background learning or change the agent's tool list.
Their current source-selection contract accepts one-shot captures; selecting the
new per-turn chat captures for concept learning is a separate integration step.

Use `sources --workspace WORKSPACE --run-id RUN_ID` to inspect authenticated
original passages from a retained run. Select two to eight originals spanning
independent episodes, then supply an owner-created learning input:

```json
{
  "protocol": "ordinary-session-learning-v1",
  "operation": {
    "kind": "induce",
    "sources": [
      {"run_id": "FIRST_RUN_ID", "original_index": 2},
      {"run_id": "SECOND_RUN_ID", "original_index": 2}
    ]
  }
}
```

Replace the run IDs and zero-based indices with values from the inventories.
Keep the input and memory manifest outside the selected task workspace. Invoke:

```bash
"$MECHA_MEMORY_BIN" memory-session --request "$MECHA_MEMORY_ROOT/memory.json" \
  learn --workspace "$MECHA_MEMORY_ROOT/work" \
  --input "$MECHA_MEMORY_ROOT/learning.json" \
  --endpoint http://127.0.0.1:8080 --model RESIDENT_MODEL \
  --holds-dir "$MECHA_MEMORY_ROOT/holds" --seed 42
```

This permits one local model call and refuses model loading or switching. A
retained concept remains a provisional hypothesis with exact source quotations.
It is not an approved instruction or an independently verified fact.

For revision, use `operation.kind: "ruminate"`, an `expected` object/revision
reference from the retained concept, and `counterexamples`, each containing a
`source` selector and an exact, unambiguous `quote`. Core resolves the original
field and byte offsets. Sources are checked before and after inference;
withdrawal withholds retention even when the model reply is valid. A recovered
storage handle cannot acquire learning authority.

The JSON report distinguishes `retained`, `revised_provisional`, `unchanged`,
`abstained`, and `failed`, and preserves available reply/usage information.
Failure prints its report before a nonzero exit. Nothing automatically retries
the model call. Ordinary capture's `not_evaluated` status remains unchanged.
The learning report conservatively marks zero token counters unknown because
the shared provider representation cannot distinguish an omitted counter from
an explicit zero; wire-audited evaluations can retain that distinction.

## The knowledge graph: mecha-graph

The world-memory is its own project —
[**mecha-graph**](https://github.com/ljchang/mecha-graph), a personal
knowledge graph served over MCP to any client, published on crates.io and
usable without mecha. Your mail, calendar, notes, and messages import as
**episodes** (append-only evidence); linkers wire them to **entities**; and
**facts** — bi-temporal claims, each carrying its evidence — are what
retrieval builds context packs from. It has its own
[section of this site](/docs/features/memory/graph) — architecture, integrations,
the self-improvement design, and its changelog, synced from the repository
at every build; this page covers the mecha side of the seam.

### Wiring

```toml
[[mcp]]
name = "graph"
command = "mecha-graph-mcp"      # `mecha features enable graph` builds it and writes this
# The kg_* tools carry their own namespace, so skip the graph__ prefix.
prefix_tools = false

# The embedder the store is built for: mecha's on-demand embeddings server.
# Without these the server assumes a 768-wide store and refuses harrier's
# 1024-wide vectors. The three move together.
[mcp.env]
MECHA_GRAPH_EMBED_URL = "http://127.0.0.1:8081"
MECHA_GRAPH_EMBED_MODEL = "harrier-oss-v1-0.6b"
MECHA_GRAPH_EMBED_DIMS = "1024"

# The graph holds other people's words — mail bodies, invite titles,
# extracted claims — so reading it must count as untrusted input. No MCP
# annotation can declare that, so config forces it, and the override can
# only ever widen.
[mcp.capabilities]
untrusted_input = true
```

The tools arrive as bare `kg_search`, `kg_entity`, `kg_timeline`,
`kg_related`, `kg_upsert`, and the verification family (`kg_verify`,
`kg_pending`, `kg_verdict`, tasks).

### Why reading memory arms the interlock

A personal graph fed by mail holds third-party text by construction, and
text from outside can carry instructions. Marking the server
`untrusted_input` is what makes the [trifecta
interlock](/docs/features/security) refuse an outbound call once memory and
private data are both in the conversation — retrieving a memory and then
reaching for `http_fetch` is exactly the exfiltration shape the interlock
exists to stop. The consequence is real and intended; relax it with
`[security] trifecta = "ask"` if you would rather be prompted than stopped.

### How memories get in

Nothing an agent writes becomes a belief without review. Two paths feed the
graph from mecha's side:

- **[Distillation](/docs/features/memory/distillation)** — `mecha distill`
  summarises each closed session into an episode and stages it through
  `kg_upsert`. The graph's extractor turns episodes into *candidates* that
  wait in your review queue; a tainted session still distills (losing the
  record of a real afternoon because a web page was open would gut the
  memory), with the taint recorded on the episode for review to see.
- **Corrections** — when you tell the assistant the graph is wrong, the
  distiller ships the correction and the graph acts: supersede the wrong
  belief, stage the replacement, demote whatever produced the error. The
  one exception to "tainted sessions still distill": corrections from
  untrusted transcripts are withheld entirely, because a fetched page must
  not be able to evict a true belief with nobody in the loop.

Reads fan the other way — `distill`, `vet`, and `corroborate` default to
`--server graph`, and the graph's own review tooling (`mecha-graph review`,
`accept`, `reject`, `precheck`) is where staged candidates become beliefs.

## What memory is *not*

The graph never enters a prompt as trusted text, rules never come from
machine refusals, and the transcript search can only reach the conversation
it records. Each "never" has a test or a structural guard behind it — the
details live in [Security](/docs/features/security),
[Learning](/docs/features/learning), and
[Sessions and replay](/docs/features/memory/sessions-and-replay).
