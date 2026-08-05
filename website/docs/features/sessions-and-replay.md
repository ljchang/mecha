---
title: Sessions and replay
sidebar_position: 12
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
mecha sessions stats --days 30
```

Transcripts live in `~/.mecha/sessions` (override with `MECHA_SESSION_DIR`), in
a directory created owner-only. `--no-session` opts out.

## The file

One JSONL file per session: a header line, then one line per record. Append-only,
so a crashed run still leaves a readable transcript. Ids are
`20260805T091500-3f2a1b7c` — sortable by name, and still unique when two runs
start in the same second.

Five record kinds:

```json
{"record":"meta","id":"20260805T091500-3f2a1b7c","created_at":"...","provider":"anthropic","model":"claude-opus-5","workspace":"/home/you/project","title":"summarize what changed"}
{"record":"config", ...}
{"record":"message","role":"user","content":[{"type":"text","text":"..."}]}
{"record":"taint","private":true,"untrusted":false}
{"record":"summary","usage":{...},"turns":4}
```

`load` skips unparseable lines rather than failing — a truncated final line is
the normal result of a killed process. A file whose first record is not a
header is not a session mecha wrote, and is skipped.

Listing goes through `peek_meta`, which reads only the first line. That keeps
`mecha sessions list` at O(number of sessions) rather than O(total transcript
bytes); with reflect-on-close recording every interaction, a full parse re-read
the whole store to print one line per file.

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

### The config record

```rust
pub struct RunConfig {
    pub mecha_version: String,
    pub provider: String,
    pub model: String,
    pub workspace: PathBuf,
    pub system_prompt: Option<String>,   // the resolved text, not a path
    pub tools: Vec<String>,              // in registry order
    pub effort: Option<Effort>,
    pub temperature: Option<f64>,        // what was actually sent
    pub seed: Option<u64>,
    pub thinking: bool,
    pub cache_prompt: bool,
    pub max_tokens: u32,
    pub max_turns: u32,
    pub max_output_tokens: Option<u64>,
    pub max_cost_usd: Option<f64>,
    pub compact_at_tokens: Option<u64>,
    pub compact_keep_recent: usize,
    pub permission_mode: PermissionMode,
    pub trifecta: TrifectaPolicy,
    pub sandbox: String,
    pub sandbox_network: bool,
}
```

The rule behind that field list: **anything that shapes the request or
constrains the run is a confound if it is not recorded.** Not theoretical —
compaction on versus off measured 1/5 against 5/5 on the same task, so a replay
that did not know whether compaction was enabled would compare two incomparable
runs and report a model regression. A denied call redirects the whole
trajectory, so replaying a read-only session under `--yes` compares nothing.
And `shell` declares *narrower* capabilities when confined, with the interlock
believing them, so the same prompt can be refused under one sandbox and allowed
under another.

The system prompt is stored in full rather than hashed. A hash tells you only
*that* something differed; the text lets a replay rebuild the request. It is no
more sensitive than the transcript beside it.

It is a **record per attach**, not a header field. A session resumed under
different flags would make a header written at creation a lie about every turn
after the first; within one process the configuration cannot change, so one
record per attach is exactly the granularity that can differ.

The sampler is recorded only as far as it is pinned. `None` means the server
chose, and the run is not repeatable.

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

Replay re-drives a recorded session and reports what changed. The **recorded
tool results are replayed verbatim** and the only live component is the model.

That is the whole design. Replaying against live tools re-reads a filesystem
and a web that have both moved, so a difference tells you nothing about the
harness. Answering from the recording isolates the variable: same turns, same
tool results, and the only thing left that can differ is what the model chose
to do with them. It costs one model call per turn and has no side effects,
which turns every real session into a free regression case — recorded from real
work rather than hand-written.

### How the run is rebuilt

From the session's `RunConfig`, not from today's flags: system prompt, tool
list, effort, thinking, budgets, compaction settings. A replay under different
conditions answers a different question. A session with no config record cannot
be replayed at all, and says so.

The tool *surface* comes from today's setup — built-ins, MCP servers, subagents
— because the recorded registry may name any of them. Each recorded tool is
wrapped rather than replaced, so the spec the model sees is the live one: a
changed description **is** part of what a replay measures. If a recorded tool no
longer exists, the replay refuses rather than offering a smaller surface than
the model saw.

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

```rust
pub enum Divergence {
    Tool      { index, expected, actual },      // a different tool entirely
    Arguments { index, tool, expected, actual },// right tool, different arguments
    Extra     { index, actual },                // the replay kept going
    Missing   { index, expected },              // the replay stopped early
}
```

The comparison is **positional**, because the order tools are called in *is* the
trajectory. Argument differences are reported separately and are the only ones
`is_structural()` calls cosmetic: a model that reads the same file by a
different path spelling has not regressed, and grading it as though it had makes
replay useless inside a week. Argument differences also do not stop the run —
returning the recorded result for materially different arguments is the price of
not pretending to know which differences matter.

`--on-divergence` decides what happens at a structural divergence:

| Mode | Behaviour |
|---|---|
| `stop` (default) | end the run there — after a divergence, every later recorded result answers a question nobody asked |
| `error` | the same, and exit non-zero on *any* divergence, argument spellings included |
| `live` | abandon the recording and continue against the real tools |

Nothing executes in `stop` or `error` mode, so nothing needs approving. `live`
falls back to the configured permission mode: real tools run after the
divergence and deserve exactly the scrutiny they always get.

### What replay is not

**Replay against a non-greedy provider is pass@k-shaped, not
exact-match-shaped.** A local server's sampler is outside this process's
knowledge, and the same case measures 5/5 rather than deterministically. One
divergent replay is a sample, not a regression.

A replayed result is also **not provenance**. The transcript does not record
which results actually came from outside, so replayed outputs carry no
`external` marking and a replay's taint may be *less* armed than the
recording's was. Refusals the interlock produced at record time were recorded
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
