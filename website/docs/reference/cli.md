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
| `--no-learned-rules` | Do not inject learned rules from `~/.mecha/learning` into the system prompt. |
| `--no-hooks` | Do not run configured `[[hook]]` commands. Config is still validated. |
| `--no-outbox` | Do not route any tools through the outbox; configured `[outbox]` tools execute directly. |
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

Exit codes: `0` success, `1` error, `2` the model refused, `3` it ran out of turns.

Approval prompts are only offered when stdin is a terminal and `--json` was not
passed; otherwise the configured `permission_mode` decides.

```bash
mecha run "summarize what changed in this repo today"

# Unattended, bounded, machine-readable.
mecha run --json --yes --max-cost 0.25 \
  -w /srv/reports "regenerate the weekly summary in reports/weekly.md"

# Piped in, continuing an earlier session.
git log --oneline -20 | mecha run - --resume 01hq9
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
mecha tui -w ~/Github/mecha
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

```bash
mecha eval -p local -m qwen3-moe -o results/qwen.json
mecha eval -p anthropic          -o results/opus5.json
mecha eval --compare results/*.json

mecha eval --tag chaining --failures            # one slice, with reasons
mecha eval -k 5 -o results/qwen-k5.json         # pass^5 beside pass@5
mecha eval eval/pkg-cases.jsonl --mcp-file eval/mcp.toml
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

## `sessions`

Inspect saved transcripts. Requires a subcommand.

```
mecha sessions <list|show|path|stats> [OPTIONS]
```

| Subcommand | Flag | Description |
|---|---|---|
| `list` | `-n`, `--limit <N>` | How many to show. Default `20`. |
| `show` | `<ID>` | Session id or unique prefix. |
| `show` | `--json` | Emit the raw JSONL records instead of formatted text. |
| `path` | `<ID>` | Print the path to a session file. |
| `stats` | `--days <N>` | Only sessions started in the last N days. |
| `stats` | `--json` | Emit JSON instead of a table. |

`stats` totals token usage — and cost, where prices are configured — grouped by
provider and model. Transcripts live in `~/.mecha/sessions` unless
`MECHA_SESSION_DIR` says otherwise.

```bash
mecha sessions list -n 50
mecha sessions show 01hq9 --json | jq -r 'select(.role == "user") | .content'
mecha sessions stats --days 30
cat "$(mecha sessions path 01hq9)"
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
mecha replay 01hq9
mecha replay 01hq9 --on-divergence error --json > replay.json   # CI gate
mecha replay ~/.mecha/sessions/01hq9....jsonl -m qwen3-14b -p local
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

## `frontdoor`

Requests that arrived through the public surface, and the quarantine they pass
through before any run with tools is told about them. `list` is the default
subcommand.

```
mecha frontdoor [list|show|extract|next] [ARGS]
```

| Subcommand | Flag | Description |
|---|---|---|
| `list` | | What has arrived, and what state each request is in. |
| `list` | `--state <STATE>` | Only this state: `drained`, `extracted`, `extraction_failed`, … |
| `show` | `<SEQ>` | One request in full, **including the prose a stranger wrote**. |
| `extract` | | Run the quarantined extraction over everything not yet extracted. |
| `extract` | `--seq <SEQ>` | Just this one. |
| `extract` | `--force` | Re-extract records that already have an extraction. |
| `next` | `--limit <N>` | What a triage run may be told, as JSON — extractions only, never prose. Default 5. |

`show` is the one place the original text is printed, and a terminal is where
that is safe. `next` is what a triage trigger pipes into a prompt; it is
structurally incapable of including the words a stranger typed.

Draining is deliberately not here — `mecha-factory-publish drain` holds the key
and speaks the protocol.

```bash
mecha frontdoor
mecha frontdoor list --state extraction_failed
mecha frontdoor show 42
mecha frontdoor extract
mecha frontdoor next --limit 3
```

See [The front door](/docs/features/frontdoor).

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
| `--server <SERVER>` | The `[[mcp]]` server holding the knowledge graph. Default `pkg`. |
| `--dry-run` | List what would be distilled without calling a model or writing. |
| `--limit <N>` | Distill at most this many sessions this run. |

Episodes are pushed through the server's `kg_upsert` as evidence, not belief: the
graph's own extractor turns them into candidates that wait in the user's review
queue. A tainted session still distills — losing the record of a real afternoon
because a web page was open would gut the memory — and the taint snapshot is recorded
on the episode's metadata instead. Idempotent at both ends.

```bash
mecha distill --dry-run
mecha distill -p local --limit 10 --server pkg
```

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
