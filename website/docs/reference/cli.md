---
title: CLI
sidebar_position: 2
description: Every mecha subcommand, its flags, and a worked example of each.
---

# CLI

```
mecha [GLOBAL OPTIONS] <COMMAND> [ARGS]
```

`mecha --help` lists the commands; `mecha <command> --help` prints the full flag set
for one. `mecha --version` prints the build.

## Global options

These are declared once and accepted by every subcommand, including the ones that
never build an agent (where they are simply ignored).

| Flag | Description |
|---|---|
| `-p`, `--provider <PROVIDER>` | Provider to use, by config key. Defaults to the config's `default_provider`. |
| `-m`, `--model <MODEL>` | Model id, overriding the provider's default. |
| `-e`, `--effort <EFFORT>` | Reasoning depth: `low`, `medium`, `high`, `xhigh`, `max`. |
| `-s`, `--system <SYSTEM>` | System prompt. Use `@path` to read it from a file. |
| `-w`, `--workspace <WORKSPACE>` | Directory the agent may read and write. Defaults to the working directory. |
| `-y`, `--yes` | Approve every tool call without asking. Required for unattended runs that need to write or execute. |
| `--read-only` | Refuse anything that is not read-only. Conflicts with `--yes`. |
| `--max-turns <N>` | Stop after this many model turns. |
| `--max-output-tokens <N>` | Stop once the run has generated this many output tokens. |
| `--max-cost <USD>` | Stop once the run has cost this much. Needs prices configured on the provider. |
| `--tool <NAME>` | Only expose these tools. Repeatable; names are matched exactly. |
| `--no-mcp` | Skip MCP servers entirely. |
| `--no-mcp-server <NAME>` | Skip these MCP servers by name. Repeatable; for turning one off while the rest stay. |
| `--no-thinking` | Turn off reasoning. Cheaper and faster, but noticeably worse on multi-step work. |
| `--no-skills` | Don't load skills from `~/.mecha/skills` — no `skill` tool, and nothing about them in the system prompt. |
| `--skill <NAME>` | Only carry these skills. Repeatable, and narrows what `[skills]` already selected — it cannot enable one config withheld. |
| `--no-learned-rules` | Do not inject learned rules from `~/.mecha/learning` into the system prompt. |
| `--no-hooks` | Do not run configured `[[hook]]` commands. Config is still validated. |
| `--no-outbox` | Do not route any tools through the outbox; configured `[outbox]` tools execute directly. |
| `--no-messages` | No inter-agent messaging: no `message_send` tool, and nothing from the mailbox is delivered into this run. |
| `--no-fallback` | Never fall back to another provider. A transient failure that survives its retries fails the run. |
| `--compact-at <N>` | Summarise older turns once the prompt passes this many tokens. |
| `-v`, `--verbose` | Print tool calls, results, and token usage as they happen. |
| `-h`, `--help` | Print help. |
| `-V`, `--version` | Print the version. |

`MECHA_LOG` controls internal tracing, which goes to **stderr** and is independent of
`--verbose`:

```bash
MECHA_LOG=debug mecha run "what changed today?"
MECHA_LOG=mecha_core::mcp=trace mecha tools     # one module only
```

The default filter is `warn`.

## `run`

Run one task and print the answer.

```
mecha run [OPTIONS] [PROMPT]
```

| Flag | Description |
|---|---|
| `[PROMPT]` | The task. Omit it, or pass `-`, to read from stdin. |
| `--json` | Emit a single JSON object instead of prose. Implies `--quiet`. |
| `--quiet` | Print only the answer — no tool narration. |
| `--no-stream` | Wait for the whole answer instead of streaming it. |
| `--resume <ID>` | Continue a saved session by id or unique prefix. |
| `--no-session` | Do not write a transcript. |

Exit codes: `0` success, `1` error, `2` the model refused, `3` it produced no
answer at all. Exhaustion is deliberately not a failure code — a run stopped by
a turn, token or cost ceiling that still answered exits `0`, because the work it
left behind is graded on its own terms. `--json`'s `stop_cause` names the
ceiling for callers that care which one it was.

Approval prompts are only offered when stdin is a terminal and `--json` was not
passed; otherwise the configured `permission_mode` decides.

```bash
mecha run "summarize what changed in this repo today"

# Unattended, bounded, machine-readable.
mecha run --json --yes --max-cost 0.25 \
  -w /srv/reports "regenerate the weekly summary in reports/weekly.md"

# Piped in, continuing an earlier session.
git log --oneline -20 | mecha run - --resume 20260805T091500
```

## `chat`

Interactive session in the terminal, with history and slash commands.

```
mecha chat [OPTIONS]
```

| Flag | Description |
|---|---|
| `--resume <ID>` | Continue a saved session by id or unique prefix. |
| `--no-session` | Do not write a transcript. |

Slash commands: `/tools`, `/model`, `/usage`, `/clear`, `/session`, `/help`,
`/exit` (also `/quit`, `/q`). `/clear` starts a new conversation, dropping its taint
along with its messages. Ctrl-D exits.

```bash
mecha chat -p local -m qwen3-14b
```

## `tui`

Full-screen session. The input line stays live while the agent works, so a message
typed mid-run steers it instead of waiting for it.

```
mecha tui [OPTIONS]
```

| Flag | Description |
|---|---|
| `--resume <ID>` | Continue a saved session by id or unique prefix. |
| `--no-session` | Do not write a transcript. |

Slash commands:

| Command | Description |
|---|---|
| `/help` | The list. |
| `/tools` | Tools this agent can call. |
| `/triggers` | Scheduled prompts: see, edit, run, cancel. |
| `/outbox` | Staged sends and publishes: show, edit, send, reject. |
| `/frontdoor` | Inbound requests: extract, triage, close. |
| `/polls` | Open polls, their tallies, and the lecture controls. |
| `/review [now\|later\|auto]` | What happens to drafts a run stages. |
| `/model [id]` | Show or switch the model. |
| `/provider [name]` | Show or switch the provider. |
| `/mode [ask\|allow\|read-only]` | Show or switch the permission mode. |
| `/mcp [on\|off]` | List MCP servers, or turn them all off and on. |
| `/mcp <server> [on\|off]` | Turn one server off and on. |
| `/usage` | Tokens used this session. |
| `/clear` | Start a new conversation, dropping its taint. |
| `/session` | Where the transcript is being written. |
| `/todo` | Show or hide the live task pane. |
| `/exit` | Quit. |

The status line shows context use as a fraction of the window when
`[providers.X] context_window` is configured. Steering is a property of this
front-end: it needs one owner of stdin, which a readline REPL cannot be while a run
is streaming.

```bash
mecha tui -w ~/code/my-project
```

## `batch`

Run the same agent over a JSONL file of prompts, with bounded concurrency.

```
mecha batch [OPTIONS] <INPUT>
```

| Flag | Description |
|---|---|
| `<INPUT>` | JSONL input. Each line is `{"id": "...", "prompt": "...", "meta": {...}}`, or a bare string used as both id and prompt. `-` reads stdin. |
| `-o`, `--out <OUT>` | Where to write results, one JSON object per line. Defaults to stdout. |
| `-c`, `--concurrency <N>` | How many items to run at once. Default `4`. |
| `--limit <N>` | Stop after this many items. Useful for a smoke test over a big file. |

Results stream to the output file as they finish, keyed by `id`, so a killed run
still leaves everything completed so far on disk. Each item gets a fresh
conversation, and therefore fresh taint.

```bash
# items.jsonl
# {"id": "q1", "prompt": "who did I meet with last week?", "meta": {"gold": "..."}}

mecha batch items.jsonl --concurrency 8 --out results.jsonl --yes
mecha batch items.jsonl --limit 3 -c 1 -v      # smoke test first
```

## `eval`

Score a model on a case set, grading the tool-call trace first and the text second.

```
mecha eval [OPTIONS] [CASES]
```

| Flag | Description |
|---|---|
| `[CASES]` | JSONL case file. Default `eval/cases.jsonl`. |
| `--fixture <PATH>` | Workspace the agent reads during the run. Defaults to a `workspace` directory beside the case file. |
| `-o`, `--out <OUT>` | Write the full scorecard and per-case detail here as JSON. |
| `-c`, `--concurrency <N>` | How many cases to run at once. Default `4`. |
| `-k`, `--runs <K>` | Run every case K times and report pass^k beside pass@k. Default `1`. |
| `--tag <TAG>` | Run only cases carrying this tag. Repeatable. |
| `--failures` | Show every failed check, not just the count. |
| `--judge-model <MODEL>` | Model that grades `expect.judge` rubrics. Defaults to the model under test. |
| `--judge-provider <PROVIDER>` | Provider entry the judge model runs on. Defaults to the one under test. |
| `--keep-workspaces` | Keep the staged workspaces of sandboxed cases instead of deleting them. |
| `--mcp` | Connect MCP servers during the eval. Off by default for reproducibility. |
| `--mcp-file <PATH>` | Connect exactly the servers named in this TOML file, instead of the machine's config. |
| `--no-ask-user` | Withhold `ask_user`, which is otherwise part of the tool surface. |
| `--ab-rules` | Run the set twice — rules-free, then with this machine's learned rules — and report the per-case flips. |
| `--ab-config <KEY=VALUE>` | Run the set twice, differing only in this override, and judge the difference against a holdout. Repeatable. |
| `--holdout-in <N>` | One case in N is held out of selection, for `--ab-config`. Default `3`. |
| `--compare <FILES>...` | Compare previously written scorecards side by side instead of running. |

`mecha eval` exits non-zero when anything fails, so it works as a regression gate.
It forces MCP off, hooks off, learned rules off, the outbox off and fallback off, so
a scorecard grades the model it names rather than this machine's local setup.

`--runs k` matters more than it looks: reliability decays much faster than mean
success, and the gap between pass^k and pass@k is the model's unreliability. A
pinned seed at `--concurrency 1` replays token-for-token, making the k samples one
sample counted k times; the harness warns when it detects that.

`--mcp-file` resolves relative paths in a server's `command`/`args` against the
file's own directory, and a server that fails to connect is fatal here.

`--ab-config` overrides a **closed set** of run options — `compact_at_tokens`,
`max_turns`, `max_output_tokens`, `effort` — so both arms are built by one code
path. Unknown keys are refused, and every override is parsed before the first arm
runs. Neither arm is filed as an ordinary scorecard, and it always exits 0: a
delta is a finding, not a gate.

```bash
mecha eval -p local -m qwen3-moe -o results/qwen.json
mecha eval -p anthropic          -o results/opus5.json
mecha eval --compare results/*.json

mecha eval --tag chaining --failures            # one slice, with reasons
mecha eval -k 5 -o results/qwen-k5.json         # pass^5 beside pass@5
mecha eval eval/graph-cases.jsonl --mcp-file eval/mcp.toml
mecha eval --ab-config max_turns=40             # measure a proposed change
```

## `tools`

List the tools an agent would see. Runs without any provider configured, which makes
it a good MCP-server smoke test.

```
mecha tools [OPTIONS]
```

| Flag | Description |
|---|---|
| `--schema` | Print the full JSON schema for each tool, exactly as the model sees it. |
| `--json` | Emit JSON instead of a table. |

The output names the active sandbox backend, and `--json` includes each tool's
capabilities. Subagent profiles are shown with the tools they were granted, with a
warning when a profile holds all three legs of the trifecta.

```bash
mecha tools
mecha tools --json | jq '.[] | select(.capabilities.external_send)'
mecha tools --schema --no-mcp
```

## `skills`

List the skills an agent would carry — the procedures you have written in
`~/.mecha/skills/`, and which of them this run would load. Builds no provider
and connects to nothing.

```
mecha skills [--show] [--json]
```

| Flag | Description |
|---|---|
| `--show` | Print each skill's full body, exactly as the model would receive it. |
| `--json` | Emit JSON instead of a table. |

A skill config withholds is listed with a `-` rather than omitted, so "why is
this not firing" is answerable here instead of by intersecting two config files
by hand. Exits non-zero when a `SKILL.md` failed to parse, so it works as a
check in a script; a store that is merely empty is healthy.

```bash
mecha skills
mecha skills --show
mecha skills --json | jq '.skills[] | select(.carried)'
```

See [Skills](/docs/features/skills).

## `sessions`

Inspect saved transcripts. Requires a subcommand.

```
mecha sessions <list|show|path|stats|health> [OPTIONS]
```

| Subcommand | Flag | Description |
|---|---|---|
| `list` | `-n`, `--limit <N>` | How many to show. Default `20`. |
| `show` | `<ID>` | Session id or unique prefix. |
| `show` | `--json` | Emit the raw JSONL records instead of formatted text. |
| `path` | `<ID>` | Print the path to a session file. |
| `stats` | `--days <N>` | Only sessions started in the last N days. |
| `stats` | `--json` | Emit JSON instead of a table. |
| `health` | `--days <N>` | Only sessions started in the last N days. |
| `health` | `-n`, `--limit <N>` | Stop after this many sessions, newest first. |
| `health` | `--json` | Emit JSON instead of a table. |

`stats` totals token usage — and cost, where prices are configured — grouped by
provider and model. Transcripts live in `~/.mecha/sessions` unless
`MECHA_SESSION_DIR` says otherwise.

`health` is the other question: not what runs cost but **how they went** — stop
causes, tool calls against errors and denials, runs that finished over a failed
call, compactions taken. Rates split by model, because a blend across two
describes neither, and a rate with no denominator prints `—` rather than `0%`.
Transcripts written before the outcome record carry none, so the corpus fills as
you use it. See [Run quality](/docs/features/run-quality).

```bash
mecha sessions list -n 50
mecha sessions show 20260805T091500 --json | jq -r 'select(.role == "user") | .content'
mecha sessions stats --days 30
mecha sessions health --days 30
mecha sessions health --json | jq '.by_model'
cat "$(mecha sessions path 20260805T091500)"
```

## `replay`

Re-run a recorded session against its recorded tool results and report where the
model diverged.

```
mecha replay [OPTIONS] <SESSION>
```

| Flag | Description |
|---|---|
| `<SESSION>` | Session id, unique prefix, or a path to a transcript file. |
| `--on-divergence <stop\|error\|live>` | What to do when the replay departs from the recording. Default `stop`. |
| `--json` | Emit the report as JSON instead of prose. |

`stop` ends the run at the divergence, because after one every later recorded result
answers a question nobody asked. `error` does the same and exits non-zero on *any*
divergence, argument spellings included — use it in CI. `live` abandons the recording
and keeps going against the real tools.

```bash
mecha replay 20260805T091500
mecha replay 20260805T091500 --on-divergence error --json > replay.json   # CI gate
mecha replay ~/.mecha/sessions/20260805T091500-3f2a1b7c.jsonl -m qwen3-14b -p local
```

## `outbox`

Review, edit, release, or reject staged outbound actions. `list` is the default
subcommand.

```
mecha outbox [list|show|edit|review|send|reject] [ARGS]
```

| Subcommand | Flag | Description |
|---|---|---|
| `list` | | List staged items, grouped by kind. |
| `list` | `--kind <KIND>` | Only `message` or only `publish`. |
| `list` | `--via <VIA>` | Only items staged by a tool whose name contains this. |
| `show` | `<ID>` | The exact arguments a release would execute, its provenance, and the edit diff if there is one. |
| `edit` | `<ID>` | Open the item's arguments in `$EDITOR`. What you save is what `send` executes. |
| `review` | `[IDS]...` | Walk items one at a time, deciding each. Ids, or unique prefixes; several is fine. |
| `review` | `--all` | Every pending item, subject to the filters. |
| `review` | `--kind <KIND>` | Only `message` or only `publish`. |
| `send` | `<ID>` | Execute the item's tool call, for real, and mark it sent. |
| `send` | `-y`, `--yes` | Skip the confirmation shown for items drafted in a tainted conversation. |
| `reject` | `<ID>` | Refuse an item. It stays on file as the record of the refusal. |
| `reject` | `--reason <REASON>` | Why — recorded on the item for the next reader. |

`edit` rewrites the arguments only; the original draft is kept, and the pair is what
`mecha reflect` mines into `writing`-domain reflections. `send` holds the store's
lock across execution so two sends cannot double-fire.

An item's **kind** decides how it is reviewed, not how it was staged. A
`publish` shows the rendered page rather than the arguments, and refuses
`edit` — see [Publishing](/docs/features/publishing).

```bash
mecha outbox
mecha outbox show 3f2a
mecha outbox edit 3f2a && mecha outbox send 3f2a
mecha outbox review --all --kind message
mecha outbox reject 3f2a --reason "wrong recipient"
```

## `msg`

Messages between this machine's own agents — a chat session, a trigger, a
one-shot `run` — addressed by **producer** name rather than by session, so an
overnight trigger can write to `chat` without knowing which chat will read it.
Delivery happens at the recipient's next turn boundary. Requires a subcommand.

```
mecha msg <send|list|show|dismiss|agents> [ARGS]
```

| Subcommand | Flag | Description |
|---|---|---|
| `send` | `<TO> <BODY>` | Leave a message for a producer: `chat`, a trigger's name, `run`. |
| `send` | `--from <NAME>` | Sender recorded on the message. Default `user`. |
| `send` | `--reply-to <ID>` | Id of the message this answers. |
| `list` | | Messages, pending first, across every mailbox. |
| `list` | `--to <NAME>` | Only this recipient's mailbox. |
| `list` | `--all` | Include delivered messages, not just pending. |
| `show` | `<ID>` | One message in full. Id or unique prefix. |
| `dismiss` | `[IDS]...` | Set pending messages aside unread. Ids, or unique prefixes. |
| `dismiss` | `--all` | Every pending message instead. |
| `dismiss` | `--to <NAME>` | With `--all`: only this recipient's mailbox. |
| `agents` | | Which agents are live right now, per the session markers. |

The agents are wired up by `[messages] enabled`, which is off by default, but
this surface is not gated on it: the store is yours, and "what did the overnight
run tell me" must not depend on a feature flag.

`dismiss` rather than `rm` is the shape that matters — a full mailbox refuses
new sends, so a backlog nobody is coming to claim needs setting aside, and the
message stays on file either way.

A send from a terminal is stamped clean, because a person typing is the one
sender whose words are trusted input. A send whose stdin is *not* a terminal — a
pipe, a script, or an agent's `shell` reaching for `mecha msg send` to route
around the harness — is stamped private and untrusted, so the receiver's
interlock sees it exactly as `message_send` would have presented it.

```bash
mecha msg send chat "the briefing is in ~/.mecha/work/briefing"
mecha msg list --all
mecha msg show 9c1e
mecha msg dismiss --all --to chat
mecha msg agents
```

## `work`

What runs have generated, and removing what is past. Every producer — a
trigger, a chat, a session — writes into its own directory, which is also the
path jail its runs get. `list` is the default subcommand.

```
mecha work [list|path|clean] [ARGS]
```

| Subcommand | Flag | Description |
|---|---|---|
| `list` | | What each producer has generated, with entry counts, size, and the newest entry. |
| `path` | `<PRODUCER>` | Print one producer's directory, creating it if absent. For `cd $(mecha work path x)`. |
| `clean` | `--keep <N>` | How many entries survive per producer. Defaults to `[work] keep` (10). |
| `clean` | `--producer <NAME>` | Only this producer. |
| `clean` | `--dry-run` | Say what would go, and remove nothing. |

`clean` never removes anything a published bundle names as a source, and says
which entries survived for that reason. The producer directory itself is never
removed.

```bash
mecha work
mecha work path briefing
mecha work clean --dry-run
mecha work clean --producer briefing --keep 3
```

See [The work directory](/docs/features/work).

## `mail`

The inbox as a queue you work. `list` is the default subcommand.

```
mecha mail [list|show|classify|reply|forward|schedule|archive|spam|task|needs-info|correct|dismiss|reflect|score|eval] [ARGS]
```

Threads are named by an eight-character handle — the **last** eight characters of
the id — and any unique suffix is accepted. A suffix rather than a prefix because
Outlook conversation ids share a 57-character common prefix. Ambiguity is an
error, never a guess.

| Subcommand | Flag | Description |
|---|---|---|
| `list` | `--all` | include threads already acted on, and the ones classified `ignore` |
| `list` | `--aged` | day two: `respond` threads old enough to have been answered and still untouched |
| `list` | `--aged-hours <N>` | how old that means. Default `30` — a working day, so an evening email is not nagged about at breakfast |
| `list` | `--surface` | record that these were surfaced, so they are not surfaced again. Deliberately separate from reading the list |
| `list` | `--json` | machine output: the typed fields only |
| `show` | `<THREAD>` | read one thread — the prose, for a human |
| `classify` | `--account <NAME>` | one mailbox. Omit to sweep every configured account |
| `classify` | `--limit <N>` | recent threads to consider per account. Default `25` |
| `classify` | `--force` | re-classify threads already in the store |
| `classify` | `--dry-run` | say what would be classified, and spend nothing |
| `reply` | `--note <TEXT>` | extra steering — "decline politely", "ask for the deadline first" |
| `forward` | `--to <ADDRS>` | comma-separated recipients |
| `task` | `--name` / `--due` / `--context` / `--project` | the task, its deadline, its GTD context (`@email`), and a parent project that **must already exist** on the graph |
| `needs-info` | `--missing <TEXT>` | what you are waiting for, in your own words |
| `correct` | `--bucket` / `--urgency` / `--proposed` / `--request-type` / `--deadline` | field-level; `none` clears a field |
| `reflect` | `--dry-run` | turn corrections into `triage`-domain reflections |
| `score` | `--min-age-hours <N>` | exclude threads younger than this. Default `48` |
| `eval` | `--sample` / `--seed` / `--prefilter-only` / `--out <PATH>` | grade the classifier against a corpus whose outcome is known |

Every subcommand takes `--account <NAME>`.

`reply`, `forward` and `schedule` **stage into the [outbox](/docs/features/outbox)
and never send**. `archive` and `spam` reach nobody outside your own mailbox and
so are not staged. Separate verbs rather than one `--action` argument, because a
free-form label would put `spam` inside a verb that reads as harmless.

`eval` writes nothing to the triage store: grading year-old mail is not triaging
it, and a scorecard that mutated the queue it measures would be unrepeatable.
`score` reads the corpus written by `mecha-mail corpus`, not the MCP tools — a
measurement keyed on a display format breaks silently the day the format
changes.

```bash
mecha mail classify --account dartmouth
mecha mail list
mecha mail list --aged --surface                  # what the morning briefing runs
mecha mail show 3f2a1b7c
mecha mail reply 3f2a1b7c --note "decline politely"
mecha mail correct 3f2a1b7c --bucket respond --urgency today
mecha mail task 3f2a1b7c --due +3d
mecha mail reflect --dry-run
mecha-mail corpus --since 2026-07-01 --account dartmouth && mecha mail score
```

See [Mail and calendar](/docs/features/mail#triage-the-queue-over-the-mailbox).

## `tasks`

The GTD board in the knowledge graph. `list` is the default subcommand.

```
mecha tasks [list|add|set] [ARGS]
```

Reached through the MCP tool surface (`kg_task_list` / `kg_task_create` /
`kg_task_update`), the same way the model reaches it — so this is one reader of
one store rather than a second copy of it, and a configuration with no graph
server says so instead of showing an empty board.

| Subcommand | Flag | Description |
|---|---|---|
| `list` | `--closed` | also show done and dropped — the history |
| `list` | `--json` | machine output: the tool's own answer, which is what the `/tasks` modal reads |
| `add` | `<NAME…>` | the task, phrased as an action. Trailing words are joined, so it needs no quoting |
| `add` | `--due <WHEN>` | `YYYY-MM-DD`, `today`, `tomorrow`, or `+Nd` |
| `add` | `--project <NAME>` | parent project — **must already name a node on the graph**; an unknown name is an error, not an implicit node |
| `add` | `--context <TAG>` | GTD context, e.g. `@email`, `@lab` |
| `set` | `<ID>` | the task's node id, from `tasks list` |
| `set` | `--status <S>` | `next`, `inbox`, `scheduled`, `waiting`, `done`, `dropped` |
| `set` | `--due` / `--defer` / `--context` | omit to leave untouched; pass `""` to clear |

A capture lands in `inbox` — captured, not yet committed to. `done` and
`dropped` stamp a completion time and are **reversible**: any other status
reopens the task. Nothing here deletes, and there is no delete verb, because
the board is the record.

The omit-versus-empty distinction on `set` is the tool's and is passed through
rather than reinterpreted — a driver that read "unset" as "clear" would wipe a
due date every time somebody changed a status.

```bash
mecha tasks
mecha tasks add --due +3d --context @lab -- Re-run the eval set on the new prefix
mecha tasks set task-1a2b3c4d --status next
mecha tasks set task-1a2b3c4d --due ""          # clear it
mecha tasks list --closed
```

The `/tasks` modal in `mecha tui` drives exactly these verbs, and
`mecha-graph tui` screen 6 is the same board with the same status letters.

## `frontdoor`

Requests that arrived through the public surface, and the quarantine they pass
through before any run with tools is told about them. `list` is the default
subcommand.

```
mecha frontdoor [list|show|extract|next|triage|needs-info|close] [ARGS]
```

| Subcommand | Flag | Description |
|---|---|---|
| `list` | | What has arrived, and what state each request is in. |
| `list` | `--state <STATE>` | Only this state: `drained`, `extracted`, `extraction_failed`, … |
| `show` | `<SEQ>` | One request in full, **including the prose a stranger wrote**. |
| `extract` | | Run the quarantined extraction over everything not yet extracted. |
| `extract` | `--seq <SEQ>` | Just this one. |
| `extract` | `--force` | Re-extract records that already have an extraction. |
| `next` | `--limit <N>` | What a triage run may be told, as JSON — extractions only, never prose. Default `5`. |
| `triage` | | Draft a reply to each extracted request, into the outbox. |
| `triage` | `--seq <SEQ>` | Just this one. |
| `triage` | `--limit <N>` | At most this many. Default `5`. |
| `needs-info` | `<SEQ>` | Park a request until the requester answers something. |
| `needs-info` | `--note <TEXT>` | What is missing. |
| `close` | `<SEQ>` | End a request. |
| `close` | `--reason <REASON>` | Why. **Required** — `any → closed` is the one transition that must never be silent. |

The verbs split along the quarantine. `list` and `show` are **for you**: `show`
is the one place the original text is printed, and a terminal is where that is
safe, because you cannot be prompt-injected into sending your own calendar
somewhere. `extract` is the quarantined pass — a tool-less model call per
record, turning prose into typed fields. `next` is what a triage trigger pipes
into a prompt, and it is structurally incapable of including the words a
stranger typed.

`triage` is the privileged half: a full agent with mail and calendar, told only
what `next` would print, drafting into the outbox. It refuses to run without
`[outbox] tools` naming the send, rather than running unrouted — a stranger's
inbox is not where you want to discover the route was unset. Each request gets
a fresh conversation, so one request's flagged prose cannot arm the interlock
for the request behind it.

`needs-info` and `close` are how a request stops growing the queue. A rejected
draft returns its request to `extracted` rather than to `closed`: "not this
reply" is not "not this request".

Draining is deliberately not here — `mecha-factory-publish drain` holds the key
and speaks the protocol, and the common case, nothing new, must cost zero tokens
and no model at all.

```bash
mecha frontdoor
mecha frontdoor list --state extraction_failed
mecha frontdoor show 42
mecha frontdoor extract
mecha frontdoor next --limit 3
mecha frontdoor triage --limit 3
mecha frontdoor needs-info 42 --note "no date given"
mecha frontdoor close 42 --reason "answered by the sent draft"
```

See [The front door](/docs/features/frontdoor).

## `slack`

Driving mecha from Slack: the credential, the binding, and who may drive.
`status` is the default subcommand.

```
mecha slack [status|auth|link|threads|connect|sweep|notify|send|remote|unlink] [ARGS]
```

| Subcommand | Flag | Description |
|---|---|---|
| `status` | | What is bound, and whether the credential still works. |
| `auth` | | Store the bot and app-level tokens, after proving them against Slack. |
| `link` | `--timeout <MINUTES>` | Give up on an unclaimed code after this long. Default `10`. |
| `link` | `--force` | Bind even though this install is already bound to another workspace. |
| `threads` | `--state <STATE>` | What state each thread is in: `idle`, `running`, `awaiting_input`, `cancelled`, `staged`, `done`, `failed`, `orphaned`. |
| `connect` | | Run the connector: hold the Slack socket open and drive runs from threads. |
| `sweep` | | Mark threads whose run did not survive a restart, so none is left showing "working…" forever. |
| `notify` | `--title <TEXT>` | Read stdin and send it to the owner as a DM. |
| `send` | `<PATH>`, `--comment <TEXT>` | Upload a file to the owner's DM — a chart, a log, a screenshot. |
| `remote` | `--sweep` | Named threads a TUI session is mirrored into. `--sweep` cools any whose session has gone. |
| `unlink` | | Forget the binding. The tokens stay, so `link` can be run again. |

`auth` reads the tokens from `MECHA_SLACK_BOT_TOKEN` (`xoxb-`) and
`MECHA_SLACK_APP_TOKEN` (`xapp-`) rather than from flags, because a flag lands
in shell history and in `ps` output, and a Slack bot token reaches the whole
workspace. It proves both against Slack before storing either.

`link` prints a one-time code **here** and binds whoever types it into Slack.
Typing a code printed on this machine proves shell access to the machine the
agent runs on; an email address proves only what the workspace claims about it.

`connect` is what the systemd unit runs (`scripts/mecha-slack.service`); it does
a `sweep` on startup. `notify` is what a trigger's `notify` calls — that command
already runs with the run's answer on stdin, so
`--notify 'mecha slack notify --title briefing'` puts the morning briefing on a
phone with no new trigger concept at all.

`send` is how something a headless box made gets looked at. Over SSH there is
no viewer, and scp in the other direction is a second connection nobody wants
to set up to look at a PNG — so the file goes to the one place already
reachable from a phone. The destination is not an argument: it is the owner's
DM, from the binding, and there is deliberately no flag that moves it.
`[slack] max_upload_mb` caps it (25 MB by default), in both directions.

The TUI has the same thing as `/send <path>`, with one difference: there the
path goes through the run's path jail, because a session has one and there is
no reason for it to have a second rule. Here the path is taken as typed —
this verb runs in your own shell, which is already the boundary.

Nothing in `[slack]` config grants access. Who may drive lives in
`~/.mecha/slack/binding.json`, a store rather than config.

```bash
export MECHA_SLACK_BOT_TOKEN=xoxb-…
export MECHA_SLACK_APP_TOKEN=xapp-…
mecha slack auth
mecha slack link                  # then type the printed code at the app in Slack
mecha slack status
mecha slack threads --state awaiting_input
echo "deploy finished" | mecha slack notify --title deploy
mecha slack send results/accuracy.png --comment 'the run finished'
mecha slack remote            # what this machine is mirroring
mecha slack remote --sweep    # cool attachments whose session died
```

See [Slack](/docs/features/slack).

## `trigger`

Prompts that run on a schedule. `list` is the default subcommand.

```
mecha trigger [list|add|show|edit|rm|enable|disable|next|run|tick|daemon|cancel|runs] [ARGS]
```

| Subcommand | Flag | Description |
|---|---|---|
| `list` | | Triggers, when each next fires, and how the last run went. |
| `add` | `<NAME>` | Lowercase letters, digits, `-` and `_`. It is the filename. |
| `add` | `--schedule <CRON>` | Five-field cron, or `@daily`/`@hourly`/`@weekly`. Required. |
| `add` | `--prompt <PROMPT>` | What to ask. `@path` reads it from a file. Required. |
| `add` | `--description <TEXT>` | One line, shown under the trigger in `list`. |
| `add` | `--timezone <IANA>` | Defaults to `[agent] timezone`, and is written into the file either way. |
| `add` | `--timeout <DUR>` | Wall-clock ceiling on one run. `20m` by default. |
| `add` | `--catch-up <SPEC>` | `always` (default), `never`, or a duration like `2h`. |
| `add` | `--notify <CMD>` | Command run with the answer on stdin. |
| `add` | `--disabled` | Create it switched off. |
| `add` | `--force` | Overwrite an existing trigger of the same name. |
| `show` | `<NAME>` | The trigger's settings and its recent runs. |
| `show` | `--last` | Print the last run's answer, read back from its session transcript. |
| `edit` | `<NAME>` | Open the trigger's file in `$EDITOR`. |
| `rm` | `<NAME>` | Delete a trigger. Its ledger rows stay as the record. |
| `enable` | `<NAME>` | Let a trigger fire again. |
| `disable` | `<NAME>` | Stop it firing without deleting it or losing its history. |
| `next` | `[NAME]`, `-n`, `--count <N>` | Upcoming fire times, without running anything. Default `5`. |
| `run` | `<NAME>` | Run one trigger now, whatever its schedule says. |
| `tick` | `--dry-run` | Say what would fire, and fire nothing. |
| `daemon` | | Tick once a minute until stopped. |
| `cancel` | `<NAME>` | Stop the run in flight, if there is one. |
| `runs` | `[NAME]`, `-n`, `--count <N>` | The run ledger, newest first. Default `20`. |

The schedule is five fields — `minute hour day-of-month month day-of-week`. Seconds
are **not** a field: `0 7 * * *` is 7am.

`tick` is the primitive and `daemon` is a loop over it, so a crontab line or a
systemd timer reaches the same answer: being due is a function of the ledger and the
clock. Missed slots collapse — a machine off for a week owes one run of each trigger,
not a week's worth — and `--catch-up` decides whether a stale slot still runs, with
skips written to the ledger.

`trigger run` is recorded with no slot, so a test run at noon does not cancel
tomorrow's 07:00. `trigger cancel` stops the run at its next safe point, keeping the
partial answer, and works even when the run is inside the daemon's process.

Triggers are read-only unless the file says otherwise; `--yes` at `add` time is what
writes `allow`. Outbox staging still works under read-only, because staging executes
nothing. Definitions live in `~/.mecha/triggers/<name>.toml` and a trigger run reads
`~/.mecha/config.toml` only, never a `mecha.toml` from the directory it starts in.

```bash
mecha trigger add briefing \
  --schedule '0 7 * * 1-5' \
  --prompt "Summarise anything in my inbox that needs an answer today, and what's on my calendar." \
  --catch-up 3h --notify 'notify-send "mecha briefing"'

mecha trigger next                 # when everything fires next
mecha trigger tick --dry-run       # what would fire, and why
mecha trigger run briefing         # fire now, without consuming the scheduled slot
mecha trigger show briefing --last # the answer it produced
mecha trigger daemon               # or point a systemd timer at `mecha trigger tick`
```

## `reflect`

Mine recorded sessions for user interventions — a mid-run steer, a denied tool call,
a corrective follow-up — and turn each into one reflection.

```
mecha reflect [OPTIONS]
```

| Flag | Description |
|---|---|
| `--sessions-dir <DIR>` | Directory of session transcripts. Defaults to the standard location. |
| `--dry-run` | List what would be mined without calling a model or writing anything. |
| `--limit <N>` | Mine at most this many sessions this run. |

Reflections are appended to `reflections.jsonl` in the learning store, each carrying
the session id that proves it and an `Origin` classified from the transcript's
recorded taint. A session whose reflections fail is left unmined for a later run to
retry rather than marked and silently lost.

```bash
mecha reflect --dry-run
mecha reflect -p local --limit 20
```

## `learn`

Absorb unprocessed reflections into the consolidated learned rule set.

```
mecha learn [OPTIONS]
```

| Flag | Description |
|---|---|
| `--min <N>` | Only run when a domain has at least this many unprocessed reflections. Default `3`. |
| `--holdout <F>` | Hold out this fraction of unprocessed reflections from the pass. Default `0`. |
| `--propose` | Stage the result as a proposal instead of writing the live rules. |
| `--dry-run` | Show what would run without calling a model or writing anything. |

`learn` rewrites `rules/<domain>.learned.toml` within a fixed character budget;
`rules/<domain>.user.toml` is yours and is never written by code. The store is a git
repo, so `git log` is the learning history and `git revert` is the undo. Non-clean
reflections are excluded structurally, before any prompt is built.

`--holdout` is deterministic (every k-th by id), because a measurement set that
changes between runs measures nothing. `--propose` gates the candidate by
counterfactual replay first and stages what survives for `mecha proposals`.

```bash
mecha learn --dry-run
mecha learn --holdout 0.25        # leave a measurement set for validate
mecha learn --propose -p local    # the nightly, unattended form
```

## `validate`

Probe whether the learned rules change the answers at the recorded moments the user
stepped in, and append every outcome to the validation ledger.

```
mecha validate [OPTIONS]
```

| Flag | Description |
|---|---|
| `--unprocessed-only` | Only validate reflections not yet consumed by a learn pass — the held-out set. |
| `--trigger <LIST>` | Probe only these triggers (comma-separated: `steer`, `denial`, `followup`). Default is all three. |
| `--judge-model <MODEL>` | Judge model id. |
| `--judge-provider <PROVIDER>` | Provider entry the judge runs on. Defaults to the model under test. |
| `--no-attribute` | Skip the bisection that attributes a regression to one rule. Regressions are still recorded, just unattributed. |

Steer and denial probes are counterfactual replays: the recorded prefix is driven
again, with and without the rules, and the verdict is structural. Followup probes are
judge-graded, so n=1 means little — read the answers before believing a flip. Run
`validate` *before* `learn`, or the rules are graded on their own training data.

```bash
mecha validate --unprocessed-only
mecha validate --trigger steer,denial --judge-provider anthropic
```

## `rules`

Rule tenure: ledger tallies per rule, retirement, and staging retirements for rules
the validation ledger keeps convicting. `list` is the default subcommand.

```
mecha rules [list|retire|restore|propose-retirements] [ARGS]
```

| Subcommand | Flag | Description |
|---|---|---|
| `list` | | Every rule with its ledger tallies and staleness. |
| `retire` | `<ID>` | Retire a rule by id or unique prefix. |
| `retire` | `--reason <REASON>` | Recorded on the rule and shown to the learner so the lesson does not come back reworded. |
| `restore` | `<ID>` | Un-retire a rule by id or unique prefix. |
| `propose-retirements` | `--min-attributed <N>` | Attributed regressions required before a rule is proposed for retirement. Default `3`. |

Retirement is a flag, never a deletion: the rule stays in the file as evidence and
`rules restore` undoes it. `propose-retirements` is a deterministic ledger scan with
no model anywhere; what it stages goes through the same proposal gate as any other
rule change.

```bash
mecha rules
mecha rules retire 7c1e --reason "measured harmful on the audit probes"
mecha rules propose-retirements --min-attributed 3
```

## `proposals`

Review, accept, or reject rule changes staged by `mecha learn --propose` or
`mecha rules propose-retirements`. `list` is the default subcommand.

```
mecha proposals [list|show|accept|reject] [ARGS]
```

| Subcommand | Flag | Description |
|---|---|---|
| `list` | | List proposals. |
| `show` | `<ID>` | The rules diff and the gate's evidence. |
| `accept` | `<ID>` | Apply a pending proposal to the live rules. |
| `accept` | `--force` | Apply even though the live rules changed since the proposal was measured. |
| `reject` | `<ID>` | Refuse a pending proposal, consuming its reflections. |
| `reject` | `--reason <REASON>` | Why — recorded on the proposal for the next reader. |

`accept` checks that the live rules still match what the candidate was measured
against; a diff on screen that is not the change being applied needs `--force` to say
so. `reject` retires the reflections so a human's no is not re-argued nightly.
Proposals can only ever touch `rules/*.learned.toml`.

```bash
mecha proposals
mecha proposals show 9a4
mecha proposals accept 9a4
mecha proposals reject 9a4 --reason "the second rule contradicts the first"
```

## `distill`

Summarise closed sessions into episodes staged to a knowledge-graph MCP server.

```
mecha distill [OPTIONS]
```

| Flag | Description |
|---|---|
| `--sessions-dir <DIR>` | Directory of session transcripts. Defaults to the standard location. |
| `--server <SERVER>` | The `[[mcp]]` server holding the knowledge graph. Default `graph`. |
| `--dry-run` | List what would be distilled without calling a model or writing. |
| `--limit <N>` | Distill at most this many sessions this run. |

Episodes are pushed through the server's `kg_upsert` as evidence, not belief: the
graph's own extractor turns them into candidates that wait in the user's review
queue. A tainted session still distills — losing the record of a real afternoon
because a web page was open would gut the memory — and the taint snapshot is recorded
on the episode's metadata instead. Idempotent at both ends.

```bash
mecha distill --dry-run
mecha distill -p local --limit 10 --server graph
```

## `doctor`

Read every store — no network, no model, no tokens — and report what is
silently wrong: dead mail logins, stuck outbox drafts, stalled frontdoor
requests, triggers whose slots stopped advancing, failed `mecha-*` units, graph
nightlies that stopped writing their log, and the population signals in the run
corpus.

```
mecha doctor [--json]
```

| Flag | Description |
|---|---|
| `--json` | Machine output: the findings as JSON. Never prompts, even on a TTY. |

On a terminal, each finding offers its remedy through an existing command —
one at a time, EOF is no. Piped or `--json`, it only reports. Exit 0 is
healthy; 1 means findings. Findings propose; a human disposes — there is
deliberately no `--yes`.

Beyond the incident checks it reads **populations**, per model, over the last
200 sessions and only above a floor of 20 runs: a model finishing a fifth of its
runs over a failed call, failing a quarter of its tool calls, or having a quarter
of its runs cut short by a ceiling. Thresholds are deliberately high — a doctor
that cries wolf stops being read — and cancellations are excluded, because a
person pressing Ctrl-C is the system working. Two trigger checks sit beside them:
one quietly failing a third of its tool calls, and one whose most recent run
succeeded having made none at all. See
[Run quality](/docs/features/run-quality).

## `diagnose`

Read the run corpus and propose one change to try — the stage between `doctor`
saying something is wrong and `eval --ab-config` saying whether a fix helped.

```
mecha diagnose [OPTIONS]
```

| Flag | Description |
|---|---|
| `--model <MODEL>` | Which model's runs to diagnose. Defaults to whichever has the most. |
| `--days <N>` | Only sessions started in the last N days. |
| `-n`, `--limit <N>` | Stop after this many sessions, newest first. Default `200`. |
| `--dry-run` | Print the brief the diagnostician would be handed, and stop. |

**It proposes; it does not measure and does not apply.** It prints a typed block
— class, change, predicted metric, rationale — and then the exact
`mecha eval --ab-config …` line that would falsify it, shell-quoted because the
change is model-authored and the line exists to be pasted.

The brief is built from counters and doctor's own findings; there is no field for
a transcript excerpt, so the corpus cannot be an injection surface. The run is
read-only with learned rules and the outbox off. A proposal that reproduces eight
consecutive words from anything the diagnostician read is **refused** — a
conclusion drawn from a source is a proposal, a sentence lifted from one is the
source's. Declining to propose is a legitimate answer, and a block that could not
be measured parses as nothing.

```bash
mecha diagnose --dry-run                    # see the evidence, pay nothing
mecha diagnose --model qwen3-moe --days 14
```

## `vet`

Judge queued knowledge-graph claims against the evidence they were extracted
from, and file the verdicts beside them.

```
mecha vet [OPTIONS]
```

| Flag | Description |
|---|---|
| `--proposer <P>` | Proposer of the class to work, e.g. `llm`. Default `llm`. |
| `--predicate <P>` | Predicate of the class to work, e.g. `has`. Default `has`. |
| `--limit <N>` | Candidates to judge, oldest first. Default 10. |
| `--record` | File the verdicts beside their candidates (mechanism `verification`). |
| `--server <S>` | The `[[mcp]]` server holding the graph. Default `graph`. |
| `--out <PATH>` | Write the judgements to a file as well. |

A verdict is an opinion filed beside a candidate that stays pending — the
graph's own review remains the door. The per-class verdict history is what
its autonomy ladder promotes on.

## `corroborate`

Judge whether queued generalisations hold beyond their one source.

```
mecha corroborate [OPTIONS]
```

| Flag | Description |
|---|---|
| `--proposer <P>` / `--predicate <P>` / `--limit <N>` / `--server <S>` | As in `vet`. |
| `--since <DATE>` | Evidence on/after this date. |
| `--min-coverage <N>` | Minimum episodes a source needs before it counts. |

## `gossip`

Two readers with different sources ask each other about one entity —
disagreement between them is the finding, not a failure to converge.

```
mecha gossip --entity <ENTITY> [OPTIONS]
```

| Flag | Description |
|---|---|
| `--entity <E>` | The person or project to gossip about — a name, alias, or id. |
| `--rounds <N>` | Rounds of question-and-answer. Bounded on purpose: a preserved disagreement is a finding. |
| `--since <DATE>` | Evidence on/after this date — both readers get the same window, so a difference must be the *sources* disagreeing, not the world having moved. |
| `--min-coverage <N>` | Minimum episodes a source needs before it can be a vantage. Default 3. |
| `--verify <N>` | Claims to audit after the exchange; 0 skips the audit. |
| `--adjudicate <N>` | Pending claims *about this entity* to adjudicate after the exchange; 0 skips it. |
| `--server <S>` | The `[[mcp]]` server holding the graph. Default `graph`. |

After building context on the entity, the run judges that entity's pending
claims — the one output that makes the review backlog smaller rather than
larger. Each reader's own searches are marked as instrumentation, so a probe
cannot manufacture its own demand signal.

## `config`

Show or create configuration. Requires a subcommand.

```
mecha config <show|path|init> [OPTIONS]
```

| Subcommand | Flag | Description |
|---|---|---|
| `show` | | Print the merged configuration as TOML. |
| `path` | | Print the files that are being read, and whether they exist. |
| `init` | `--project` | Write `./mecha.toml` instead of `~/.mecha/config.toml`. |
| `init` | `--force` | Overwrite an existing file. |

```bash
mecha config init                 # ~/.mecha/config.toml
mecha config init --project       # ./mecha.toml
mecha config path
mecha config show | grep -A4 '\[sandbox\]'
```

See the [configuration reference](/docs/reference/configuration) for every key.
