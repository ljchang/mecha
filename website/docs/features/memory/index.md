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

**Retention is separate from success.** A tool returning successfully or an
assistant claiming completion does not establish that a task succeeded. The
optional JSON `memory` report identifies retained, pending or unavailable capture,
with `semantic_assessment: "not_evaluated"`. The task's ordinary answer and outcome
remain separate.

This first profile supports fresh, bounded, text-only one-shot sessions with
explicitly selected `fs_read`, `fs_write`, `fs_list` and `goal_context` tools.
It requires a confirmed goal, no MCP, code tools or grants, and refuses project
configuration instead of silently discarding its restrictions. Hooks, provider
fallbacks and unapproved inbound-message profiles are refused. Existing approval
rules still apply. Configured subagents and active outbox routes intersecting the
selected tools are refused. One-shot resume, images, delegated/program traces and
compaction rewrites are not supported for retention. An unsupported recording
can leave a completed action intact while its memory report is unavailable.
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
"$MECHA_MEMORY_BIN" --no-mcp --no-code-tool --no-grants --no-messages \
  --no-hooks --no-fallback --no-learned-rules --no-past-appraisals \
  --no-success-examples --no-thinking \
  --tool fs_read --tool fs_write --tool fs_list --tool goal_context \
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
are limited to 4 MiB, and the inherited passage and context limits
can be reached sooner. In both one-shot and chat capture, tool arguments above
1,024 serialized JSON bytes retain their full-input digest, native source pointer
and action receipt with an explicit oversized status; the memory record omits
the argument content. This does not make omitted text factual support or prove
the action achieved its goal. Full arguments remain in the native transcript.
Tool results and other original passages still have a 4,096-byte limit.
Web/TUI memory attachment and automatic learning remain
separate work. SQLCipher does not encrypt the native transcript, terminal history
or private recovery files.

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

### Explicit concept learning

The same experimental build provides host-only `memory-session sources` and
`memory-session learn`. These operate on an explicitly selected ordinary memory
store; they do not schedule background learning or change the agent's tool list.
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
