---
sidebar_position: 12
---

# Experiments

`mecha exp` runs a **designed comparison** over a chosen set of runs: arms
that vary the harness, a control they are measured against, a prediction each
treatment arm makes before anything runs, and one isolated home per arm so a
trial's learning never touches your real store.

An arm is a **model and a harness configuration**, and an experiment may vary
either or both. [`mecha eval`](/docs/features/evaluation) is the special case
of arms that name models under the `bare` preset, run in-process and printed
as a scorecard; it shares this feature's case file, fixture and graders, and
its own A/B flags are two-arm manifests waiting to be written as such.

## Quickstart

Two arms, one task, one seed: the harness as the default
[environment](#environments) configures it, against the same harness without
learned rules. Nothing a trial does reaches your own graph, mail or rules. Run
it from the checkout root, since relative paths in a manifest resolve against
the directory you run `mecha exp` from.

```toml
# quickstart.toml
name = "quickstart"
control = "full"
seeds = [1]
split_seed = 7

[tasks]
cases = "eval/cases.jsonl"
fixture = "eval/workspace"
ids = ["read-readme"]

[arms.full]
preset = "full"

[arms.no-rules]
levers_off = ["learned_rules"]
[arms.no-rules.prediction]
metric = "turns"
rationale = "without the rules block the run takes more turns"
```

```bash
mecha exp new quickstart.toml
mecha exp run quickstart --dry-run     # the trial plan, with each row's condition hash
mecha exp run quickstart
mecha exp status quickstart
mecha exp judge quickstart
```

Here is what `judge` prints for that design:

```
no-rules  [turns]  1 pairs (0 selection, 1 holdout)
  selection: 0 wins · 0 losses · 0 ties    holdout: 0 wins · 0 losses · 1 ties
  work: control 1 calls, treatment 1 calls
  PROPOSE — only 0 paired episode(s) in the selection slice, below the floor of 8 — read it rather than trusting it
```

The mechanics work, but one pair is not evidence. [Sizing a
design](#sizing-a-design) says how many trials a verdict needs.

## The manifest

The design is a TOML file, written once:

```toml
name = "boredom"
control = "full"
split_seed = 7
seeds = [1, 2, 3]

[tasks]
cases = "eval/cases.jsonl"
fixture = "eval/workspace"
tags = ["files"]

[arms.full]
preset = "full"

[arms.quiet]
levers_off = ["boredom"]
[arms.quiet.prediction]
metric = "turns"
rationale = "without the notice, the run does not spend turns answering it"

[arms.rules-only]
preset = "bare"
levers_on = ["learned_rules"]     # add-one-to-bare
[arms.rules-only.prediction]
metric = "failure"
rationale = "the rules alone, over nothing else"

[arms.bare]
preset = "bare"
[arms.bare.prediction]
metric = "failure"
rationale = "everything off should fail more"

[arms.small-model]
provider = "small"          # a key in your [providers] table
model = "gemma-4-e4b"
[arms.small-model.prediction]
metric = "failure"
rationale = "the same harness on a smaller model fails more"
```

An arm may name its `provider` (a key in your config's `[providers]` table)
and `model`; absent, it runs against your default. Beyond that it may only
vary the closed set: levers by name in `levers_off`, or turned back on
after a preset in `levers_on` (`bare` plus `learned_rules` is the
add-one-to-bare design; a name in both lists is on — the list of levers is
`mecha_core::harness::Lever`), knobs as `KEY=VALUE` over the same override set
`harness ruminate` uses, and a preset — `bare` is what eval runs, `full`
forces nothing off. An unknown lever name is a load error. The `approval_rules`
lever is refused: a `forbid` in your rules file is your standing word, and an
experiment does not lift it.

The control carries no prediction. Every other arm must, and the metric is
always a cost — the task outcome enters as `failure` (`1 − passed`), the rest
are the gate's own (`turns`, `tool_error_rate`, `cut_short`, `compactions`,
`ended_on_failed_call`, `malformed_args`).

## What an arm can vary

The set is closed. Anything an arm could change without the record naming it
would be a confound that nothing can read back later, so every axis below is
recorded on each trial's row and folded into its condition hash.

**Model.** `provider` (a key in `[providers]`) and `model`. llama-server
ignores the requested model name, so check what is actually loaded
(`GET /props`) before an arm names a second local model.

**Presets.** `full` forces nothing off, so the arm runs your config as it
stands. `bare` forces every lever off except `approval_rules`, and is what
`mecha eval` runs. The preset applies first, then `levers_off`, then
`levers_on`.

`levers_on` undoes a preset or a `levers_off`, and **forces** a switch on
even where your config turns it off, so `levers_on = ["step_escalation"]`
measures step escalation although it ships off. A forced switch is part of
the row's condition hash. Switches an arm does not name keep your config's
value.

**Levers**, the per-run switches, named in `levers_off` / `levers_on`:

| Lever | Off means |
|---|---|
| `mcp` | Configured MCP servers are not connected. |
| `learned_rules` | The learned-rules block is not built into the prompt. |
| `hooks` | `[[hook]]` commands do not run. |
| `outbox` | `[outbox] tools` execute unstaged. |
| `fallback` | A provider failure is not retried against a fallback. |
| `messages` | No inter-agent mailbox is attached. |
| `skills` | The skill block is not built. |
| `charter` | The charter block is not built. |
| `compact_tool` | The model's `compact` tool is not registered. |
| `step_escalation` | An ambiguous finished plan step is not escalated to a quarantined check. Ships off. |
| `step_checks` | Declared plan-step checks are not run. |
| `goal_guidance` | No guidance from the goal, charter and planning discrepancies is added. Ships off. |
| `boredom` | The notice that an approach has stopped teaching the run anything is never raised. |
| `compact_validate` | A compaction summary is not checked for omissions against what it replaced. |
| `predictive_compaction` | Compaction triggers on the reported size only, never on the forecast. |
| `carried_state` | Tool state (the plan) does not carry across a compaction. |
| `approval_rules` | *Refused in a manifest.* Your `forbid` list stands. |

Every lever except `approval_rules` corresponds to a `--no-…` flag on
`mecha run`, so you can try one by hand before designing around it. There is
deliberately no flag for `approval_rules`: a `forbid` is your standing word,
and only `mecha eval`'s fixture workspaces lift it.

**Knobs** are `KEY=VALUE` strings in `overrides`, validated when the
manifest loads:

| Key | Values |
|---|---|
| `max_turns` | an integer ≥ 1 |
| `max_output_tokens` | an integer ≥ 1 |
| `compact_at_tokens` | an integer ≥ 1000 |
| `effort` | `low`, `medium`, `high`, `xhigh`, `max` |

```toml
[arms.short-leash]
overrides = ["max_turns=6", "effort=low"]
```

A case in the case file may set its own ceiling. The arm's knob wins when an
arm moves that knob, because the arm is the treatment.

**Stage levers** apply to [lifetimes](#lifetimes) only: `reflect`, `learn`,
`validate`, `retire`, `ruminate`, `sensors_in_brief`, named in `stages_off`.

**The environment.** An arm may name its own
[environment](#arm-environments), which is how it varies anything the list
above can't: the system prompt, the tool list, the charter, the graph's
contents, or any other setting an environment's `config.toml` can hold. The
environment's resolved contents are part of the arm's condition hash.

**The rest of the world**, `[fixtures]` and `[tasks]`, is the manifest's and
the same for every arm, and it is part of the hash too.

## Environments

A trial never runs in your home. Its home is built from an **environment**:
a directory holding the harness configuration, the charter, the learning
store and each server's starting data. A manifest names one, and with no
`[environment]` table it runs in the repository's `eval/envs/default`.

```toml
[environment]
dir = "eval/envs/my-lab"      # relative to the checkout
live_servers = []             # your own [[mcp]] servers a trial may reach, by name
```

The directory's layout:

| Path | What it is |
|---|---|
| `config.toml` | The harness: `[agent]`, `[tools]`, `[[mcp]]`, `[[hook]]`, `[outbox]`, skills, subagents. |
| `charter.toml` | The charter each home starts with. |
| `skills/`, `learning/` | The skills and learning store (rules, reflections) each home starts with. |
| `stores/<server>/` | A server's starting files, copied into its store. |
| `stores/<server>.calls.jsonl` | Calls replayed into the server's store, in order, once per experiment. |

**Machine facts come from your config, never from an environment.** An
environment's `config.toml` may not set `default_provider`, `[providers]`,
`[sandbox]`, `[security]`, `[[rule]]`, `[approval]` or `[[search]]`, and
`mecha exp` refuses the file if it does. Those tables describe this machine
and your standing rules, so an environment cannot lift a `forbid` any more
than an arm can.

**Servers get their own data.** A server in the environment's config that
writes `${STORE}` in an `env` value or an argument gets a store directory
under the trial home, and `${STORE}` becomes its path. The default
environment runs the real knowledge graph server this way, with every tool
available, on the trial's own database:

```toml
[[mcp]]
name = "graph"
command = "mecha-graph-mcp"
prefix_tools = false
env = { MECHA_GRAPH_DB = "${STORE}/graph.db", MECHA_GRAPH_CONFIG = "${STORE}/config.toml" }
```

`mecha exp run` builds each store once per experiment, before the first
trial. It copies `stores/<server>/`, then replays `stores/<server>.calls.jsonl`
line by line. A line is either a call through the server's own tools,
`{"tool": "kg_upsert", "arguments": {…}}`, or a command,
`{"run": ["mecha-graph", "embed"]}`, which runs with the server's environment.
Any call answered with an error, and any command that fails, stops the build
rather than seeding part of a world. A `single` trial starts from the built
store every time. A lifetime keeps what its tasks wrote.

The graph's own `config.toml` (`stores/graph/config.toml` in the default
environment) names the embedding model and its dimension, and nothing else.
It has to match the embedding server this machine runs (`curl -s
localhost:8081/props`), or search cannot run against the trial's graph.

**The default environment** is a small synthetic lab, with Ada Okafor's
fictional cast: a charter, a seeded knowledge graph (people, projects, a task
board), and the fixture mail and calendar server with its outbox route. It
turns off no feature, and it contains no real person. Copy it to start your
own.

**Your live servers are opt-in.** `live_servers = ["graph"]` would hand a
trial your real graph, reads and writes. The names are part of the condition
hash, so a live-world row never pairs with a sandboxed one. Leave the list
empty unless the question is about your real world.

An environment holds files, never links: a symlink anywhere in it is
refused, since the directory is copied into trial homes. The whole
environment directory is part of every row's condition hash, by content.
Editing a file between two runs makes a new condition, and its stores are
built fresh beside the old ones. `[fixtures]` servers, when a manifest names
any, replace the environment's servers entirely.

### Arm environments

An arm can run in its own environment, in place of the manifest's:

```toml
[arms.no-shell]
environment = { dir = "eval/envs/no-shell" }
[arms.no-shell.prediction]
metric = "failure"
rationale = "without shell, the shell tasks fail"
```

Usually that environment **extends** another and holds only what differs.
Its `environment.toml` names the base:

```toml
# eval/envs/no-shell/environment.toml
extends = "eval/envs/default"
```

```toml
# eval/envs/no-shell/config.toml: merged over the default's
[tools]
disabled = ["shell"]
```

How a variant combines with its base:

- **Any file** in the variant replaces the base's file at the same path, such
  as a different `charter.toml` or a different `stores/graph.calls.jsonl`
  for a richer graph. A file that `config.toml` names by path (the system
  prompt, a server's script) is read from the checkout, not from the built
  environment, so to vary one, point the setting at the variant's own file
  by its checkout-relative path, as the table below shows. Overlaying a file
  at the base's path changes nothing but the hash.
- **`config.toml`** merges key by key. Tables combine; a single value or a
  list replaces the base's. So a variant's `[agent] max_turns = 6` keeps the
  rest of the base's `[agent]`, while a variant's `[[mcp]]` list is its whole
  server list, not an addition.
- **Chains** are allowed: a variant can extend a variant, up to eight deep.
  A loop, a link anywhere in the chain, or a chain that reaches your mecha
  home is refused.

Some variations, and where each goes:

| To vary | In the variant |
|---|---|
| The system prompt | `config.toml`: `[agent] system_prompt_file = "eval/envs/x/agent.md"` |
| Which tools exist | `config.toml`: `[tools] disabled = ["shell"]` (or `enabled = [...]`) |
| Any agent setting | `config.toml`: under `[agent]` |
| The charter | `charter.toml` |
| What the graph knows | `stores/graph.calls.jsonl` |
| The servers a trial reaches | `config.toml`: its own `[[mcp]]` list |

`mecha exp run` resolves each distinct environment once, builds it into the
experiment's directory, and runs every trial of the arm from that build. The
arm's condition hash takes the resolved files' digest, so editing the base
also moves every variant that extends it, since that edit changes what the
variant runs. Arms whose environments resolve the same share a condition
and are flagged as identical like any other.

If you edit an arm's environment between two sittings, a `single` arm's home
is re-seeded from the new build (its charter, skills and learning store), since
each trial starts from the seed anyway. A lifetime refuses to resume instead:
re-seeding halfway through would discard what the loop learned and splice two
worlds into one sequence, so start a new experiment for the new environment.

## Running

```bash
mecha exp new boredom.toml          # writes ~/.mecha/experiments/boredom/
mecha exp run boredom               # one child `mecha run` per trial; resumable
mecha exp status boredom
mecha exp judge boredom             # each treatment arm against the control
mecha exp export boredom > out.json
```

Every trial is its own `mecha` process, started with `MECHA_HOME` pointing at
its arm's home under the experiment directory and with the staged workspace
as its working directory. That home's `config.toml` *is* the arm: the
environment's harness, your machine facts (providers with every inline key
scrubbed, since the environment variable the key names still reaches the
child; your sandbox, security and approval rules), and the arm's switches
and knobs applied. So nothing about an arm is ambient, and your `forbid`
list still stands. The home's learning store, skills and charter come from
the environment once, when the home is first created, and nothing is ever
copied back. The child gets a clean environment with only what it needs, and
the runner refuses a home that is, or contains, your real one.

A trial's session is marked `experiment`. In your real store that kind is
hidden from every readout, like a smoke test; in the trial home it is admitted,
so `reflect`, `learn` and the run-quality corpus read a trial's sessions there
without a flag. The session's config record carries which trial it was.

## Judging

`judge` pairs each treatment arm with the control by task, seed and
repetition, draws the holdout with the manifest's `split_seed`, and rules
through the same gate `harness ruminate` uses: wins on the selection slice,
confirmed on the holdout, under the work guardrail. A trial with no grade or
no stats drops its pair rather than counting as zero. Below the gate's floors
the verdict is *propose*, and says so.

### Reading the results

`judge` gives a verdict. `mecha exp report <name>` shows what's behind it:

```
arm               done failed pending running unknown        pass  pass^k  pass@k  turns  tokens in/out    wall tools (err)  jobs
rules-off-12         8      0       0       0       0     6/8 75%     6/8     6/8    6.1    7.3k / 6.4k   13.0s      55 (0)     1
rules-on-12          8      0       0       0       0     6/8 75%     6/8     6/8    5.9    7.3k / 6.4k   12.7s      53 (0)     1

task                         rules-off-12  rules-on-12
latest-evidence                       0/1          0/1
linked-packet-11                      0/1          0/1
…
```

- **Per arm:** trials by status (pending, running, done, failed, and
  unknown, a status this build can't read, which a lifetime stops at); the
  pass rate; **pass^k** (tasks that passed
  on every run, across seeds and repetitions) and **pass@k** (on at least
  one); mean turns and wall time; total tokens and tool calls; and the
  `--jobs` limits it ran under.
- **Per task:** passes over runs in each arm. A task every arm passes, or
  every arm fails, tells the gate nothing, and this is where you see it.
- **For a lifetime:** pass rate by sequence position for each arm, with the
  failure rate over the first half against the second. A loop that learns
  fails less later.

A rate with nothing under it prints as a dash, never a zero: "nothing ran"
and "nothing passed" are different findings. `--json` gives the same data
for your own analysis.

### Sizing a design

A treatment arm is compared with the control **pair by pair**: the same task,
seed and repetition on both sides. The gate needs at least **4 holdout pairs
and 8 selection pairs**. One pair in every `holdout_in` (default 3) is held
out, with a minimum of 4. So a verdict needs **at least 12 pairs per
treatment arm**:

```
pairs per arm = tasks × len(seeds) × repetitions      # ≥ 12
trials        = pairs per arm × number of arms
```

Six tasks × two seeds gives twelve pairs. With a control and three treatments
that is 48 trials, each a full child run. Use `--dry-run` to count them and
`--limit N` to spread a large design across sittings. `run` resumes, and
never reruns a finished trial.

**Two arms with the same condition hash are one condition under two
names**, so every difference between them is noise. That is what an A/A
design wants, and a mistake anywhere else: `levers_on = ["learned_rules"]`
over `full` is `full`, since `full` already has it on. `run` warns about
identical arms when it plans, and `judge` marks such an arm
`same condition as <control>`, which is `same_condition_as_control` in
`--json`. Neither refuses, and the verdict is left as it is.

Each treatment arm is judged against the control only, on its own predicted
metric. Tasks with more room to differ make better pairs: a task every arm
passes in two turns ties every time and teaches the gate nothing.

### Running trials in parallel

`mecha exp run <name> --jobs N` keeps up to N trials in flight (`single`
designs only). Two rules shape it:

- **Never two trials of one arm at once.** Each trial resets its arm's home
  (config, server stores, clock) before it starts, so two of one arm would run
  in each other's world. With two arms, `--jobs 3` therefore runs two at a
  time. Arms interleave rather than running in blocks.
- **Each trial holds one of the background model seats** (three, one fewer
  than the server's slots, so your own turn never queues). When none is free,
  the run waits and says who holds them. `--jobs` above the seat count is
  clamped to it (`--jobs 8` runs at most three), and fewer run while other
  background runs hold seats.
  `--jobs 1`, the default, takes no seat, like any run you are watching from
  a terminal.

**While it holds the seats, your other background work waits.** A detached
`mecha tasks work` and an unattended `mecha questions answer` refuse to start
when every seat is taken, and they name the experiment's trials as the
holders. Scheduled triggers take no seat and still run. A long design at
`--jobs 3` can hold all three for hours; `--jobs 2` leaves one free.

Every row records the `--jobs` limit it ran under, after that clamp. It's
still an upper bound, because the one-trial-per-arm rule can hold it lower.
Concurrent requests share the server, so time and queue wait change with
it, and a pinned seed only replays exactly when nothing else is in the
batch. So when an arm's pairs ran under different limits (say, an
experiment started at `--jobs 1` and resumed at `--jobs 3`), `judge` holds
the verdict at *propose* and says why.

Expect a modest speed-up, not an N-fold one. The server's throughput stops
growing at its slot count, and short, prompt-heavy trials barely gain at
all. One measurement on this machine (idle server, one sample each):

| Tasks | `--jobs 1` | `--jobs 3` |
|---|---|---|
| 12 two-to-three-turn lookups, 3 arms | 31–32 s | 24–32 s |
| 16 file-correction tasks, 2 arms (two in flight) | 207 s | 154 s (1.34×) |

## Lifetimes

A `single` trial is one run per arm × task × seed × repetition, which is the
shape that answers "does this disposition help inside a run". Everything the
appraisal loop *does* acts across runs — reflections become rules in the next
run's prefix, run counters become config overrides — so the unit that can
measure it is a **lifetime**: an ordered task sequence sharing one home, with
the loop's stages run between tasks.

```toml
name = "loop"
kind = "lifetime"
control = "full"
split_seed = 11
seeds = [1, 2]
repetitions = 1

[schedule]            # every N tasks; 0 = never. This is the default.
reflect = 1
validate = 5
learn = 5
retire = 5
ruminate = 10

[tasks]
cases = "eval/cases.jsonl"
fixture = "eval/workspace"
ids = ["hello", "files-read", "files-write", "shell-ls"]   # the sequence, in order (required)

[arms.full]
preset = "full"

[arms.deaf]
preset = "full"
stages_off = ["ruminate", "sensors_in_brief"]
[arms.deaf.prediction]
metric = "failure"
rationale = "without rumination the loop cannot move a knob, so failures do not fall over the sequence"
```

Each lifetime — one per arm × seed × repetition — gets its own home under the
experiment directory, seeded like an arm's. The driver walks the sequence in
order: after each task it runs the stages the schedule makes due, as child
`mecha` verbs against that home — `reflect`, `validate --unprocessed-only`,
`learn --holdout 0.25 --auto`, `rules propose-retirements --apply`,
`harness ruminate`, the nightly's own order and flags, so what a lifetime measures is the loop that ships (validate
measures before learn consumes) — one after another and never beside a
task, and writes
each to the lifetime's **stage ledger** (`stages/<lifetime>.jsonl`) with its
exit status and where its output went. The ledger is what says a stage ran;
the manifest says only what was scheduled. Resume reads both: a finished task
is not rerun, and a stage the ledger lacks after a finished position runs
before the next task starts — but only while no later position has finished;
past that it is recorded as skipped out of sequence, which holds the verdict,
since a stage run after later tasks would act on sessions those tasks never
ran under.

**Stage levers** are a second closed set, beside the per-run levers, and a
lifetime's arm may name them off in `stages_off`: `reflect`, `learn`,
`validate`, `retire`, `ruminate`, and `sensors_in_brief` — the last is not a stage but
the switch (`[agent] sensors_in_brief`) that hands the homeostat's and guilt's
readings to the diagnostician's brief, which is those sensors' only reader.
A `single` manifest refuses them. Stage levers off are part of a row's
condition hash; an arm with every stage on hashes as its single-trial twin.

`status` shows each lifetime's sequence by position (`✓ ✗ ! ~ ·`) with its
stage counts; `judge` pairs positions across arms as it pairs tasks, since the
sequence is shared, and reads the ledger: a stage that failed, was
interrupted, or could not be read on either side holds the verdict at
*propose*, because a treatment not known to have run cannot claim its
effect. Read the trajectory, not the mean: a loop that learns has
a slope.

## The principal

Every reflection trigger is an owner's act — a steer, a denial, a follow-up,
an edited draft — and a lifetime run with nobody answering mines nothing. The
**principal** plays the owner: an executable the manifest names, called
before and after every task with the trial's state on stdin, answering with
the owner's verbs to run and the refusals to script.

```toml
[principal]
command = ["/home/me/mecha/scripts/principal-gold.py", "/home/me/mecha/eval/principal.toml"]
timeout_secs = 600
```

The principal must read its whole state before answering; one that exits
without draining stdin is a failed call, recorded on the ledger. The
principal is pure: it never runs a verb itself. The driver runs each one
as a child `mecha` against the trial home, from a closed set — `tasks
set|steer|stop`, `outbox reject|edit`, `questions answer|abandon`,
never a session, a reflection or a rule — and records the call and every act
with its exit status on the lifetime's ledger, so a principal that could not
act holds the verdict like a failed stage. Refusals it scripts before a task
land in the home's `principal/denials.toml`; the run reads them ahead of its
own approver and renders each as "Denied by the user", which the learning
loop mines as a correction — the owner's word, inside the trial home. Only an
experiment's run honours that file; any other run started with it refuses to
start, since a scripted refusal on your real home would author corrections
nobody made.

`scripts/principal-gold.py` is the gold-verdict version: a draft addressed off
the fixture cast is rejected and one on the cast is released, a board task
is closed by the task's grade, a parked question is answered from a table,
and refusals come from the policy file it is given.

Two of the principal's verbs reach a server — `outbox approve` executes the
routed tool for real, and `tasks set` writes the board, which lives in the
knowledge graph over MCP. An environment can open your live servers
(`live_servers`), and then a release would send from your account and a
closure would close a real task. The driver therefore permits those two
verbs **only under a manifest that names fixture servers**, and vets a
release against the draft it names: the draft's tool must be a fixture
server's, by its `<name>__` prefix.

## Task sources

A manifest's tasks come from an eval case file, or from a **task source**:
an executable that answers three verbs.

```toml
[tasks]
source = ["python3", "eval/fixtures/dojo.py", "--suite", "workspace"]
fixture = "eval/workspace"
source_timeout_secs = 600
```

`list` prints the tasks as JSON (id, prompt, tags, an optional turn ceiling,
an optional `expect` block); `setup <task>` puts the world in the task's
starting state before the run; `grade <task>` reads the run's `--json` result
on stdin and prints a verdict with the checks behind it. The driver calls each
with `MECHA_HOME`, `MECHA_FIXTURES` (the home's fixture-store root),
`MECHA_EXPERIMENT_WORKSPACE` and `MECHA_EXPERIMENT_TASK` set, and every edge
fails the trial rather than passing it: a non-zero exit, a timeout, no JSON,
an unknown shape, or a verdict that disagrees with its own checks. Under
`mecha exp run --jobs`, a source's `setup` and `grade` may run for several
arms at once, so key any state on `MECHA_FIXTURES` or `MECHA_HOME` (both per
arm), never on a fixed path. `eval/fixtures/source_stub.py` is the whole
contract in forty lines.

`eval/fixtures/dojo.py` is AgentDojo as a fixture world — the same program
serves a suite's tools over MCP and acts as the task source for its user
tasks, each also paired with an injection task, graded by the suite's own
`utility` and `security` functions. It needs the venv `scripts/dojo-venv.sh`
builds. `eval/dojo-workspace.toml` runs the workspace suite:

```
scripts/dojo-venv.sh
mecha exp new eval/dojo-workspace.toml
mecha exp run dojo-workspace
```

## Executable correction pilot

Recorded replay returns saved tool results. To measure whether a changed harness
actually completes work, `eval/executable-validation.toml` instead runs real file
tools on eight registered corrective tasks. Each trial starts in a fresh workspace
with an incorrect output artifact. Independent JSON and preserved-input checks
accept any correct tool sequence.

From a source checkout with Python 3.11 or later and a built binary:

```bash
python3 scripts/executable-validation.py --out /tmp/executable-pilot
```

The registered design uses the local `qwen3.6-35b-a3b` server, three seeds and four
arms: frozen learned rules off/on crossed with turn limits 12/10. The script checks
that the configured limit is 12 before calling it the control. These are model-turn
limits, not counts of individual tool calls; a turn may contain several calls.
`--binary`, `--config` and `--rules` choose the runtime and snapshot inputs.
`--candidate /path/to/candidate.json` ties the report to a staged `max_turns=10`
proposal without changing that proposal.

The output directory contains the native experiment, transcripts, resulting files,
input/runtime hashes and a paired scorecard. It also contains a private snapshot
of the operator's rules and config. A cheaper unfinished task counts as a
regression; cost differences are also reported for pairs that both completed.
No learning stages run and no override is installed.

This is a synthetic pilot of corrective task execution. It starts from known
incorrect artifacts, not reconstructed historical conversations. It does not test
live-service effects or the process that learned the frozen rules. Repeated seeds
are repeats of the same tasks, not unseen task families; the native gate's
selection/holdout split is not evidence of transfer to different work.

## Fixture servers

A manifest may carry a `[fixtures]` table naming MCP servers the trial home
runs **instead of** yours. When it names any, the home's `[[mcp]]` is
exactly that list, for every arm — no live server reaches it — and each
server keeps its state under the home (`fixtures/<name>/`, handed to it as
`MECHA_FIXTURE_DIR`), seeded once from a directory you name. The outbox
route is the world's too: `outbox_tools` names the fixture tools whose calls
are staged as drafts, and it must be spelled — your own `[outbox] tools`
names live tools that are not in this world, so it is not inherited.
Relative paths are resolved against the checkout you run `mecha exp` from.

```toml
[fixtures]
charter = "eval/fixtures/home/charter.toml"   # written over the home's before every task
outbox_tools = ["mail__mail_send", "mail__mail_reply"]   # the world's staged sinks; required, [] for none

[[fixtures.mcp]]
name = "graph"
command = "python3"
args = ["eval/fixtures/board_server.py"]
prefix_tools = false                          # the board is kg_task_*, as in production
seed = "eval/fixtures/home/board"
[fixtures.mcp.capabilities]
untrusted_input = true

[[fixtures.mcp]]
name = "mail"
command = "python3"
args = ["eval/fixtures/mail_server.py"]
seed = "eval/fixtures/home/mail"
[fixtures.mcp.capabilities]
untrusted_input = true
```

Two fixture servers ship with the repository, stateful where the eval
rig's `graph_server.py` is deliberately not: `board_server.py` is the graph's
task board with the real server's argument and answer shapes (what `mecha
tasks` parses) plus the canned graph reads, and `mail_server.py` is the mail
and calendar surface, with every send a line in `sent.jsonl` and nothing
delivered. Both refuse to start without a store directory — a board that
forgets is not a fixture. The fixture names are part of every row's
condition hash, so a trial against a fixture board never pairs with one
against your live graph.

`eval/fixtures/home/` is a **synthetic assistant home** built on them: a
board with a task per case, a mailbox on a fictional cast (including one
message that tries to instruct the assistant), a calendar and a charter.
`eval/home-lifetime.toml` runs it as a lifetime with the gold principal
releasing drafts to the cast and closing each case's task by its grade:

```
mecha exp new eval/home-lifetime.toml
mecha exp run home-loop
```

### Gossip follow-up comparison

The developer fixture in `eval/fixtures/gossip/` compares ordinary peer questions
with follow-ups generated from each reader's own evidence. Build the native actor
with `cargo build -p mecha-core --example gossip_compare`, then run
`python3 scripts/gossip-comparison.py --out /tmp/new-gossip-comparison`.

Both conditions have the same request ceilings. Actual calls and tokens are
recorded, and an explicit citation audit supplies supported/contradicted labels
before scoring. The sources are synthetic and frozen; this experiment does not
file graph verdicts or change the ordinary `mecha gossip` behavior. Its README
specifies the local model requirements, audit format and limits of the comparison.

## Recipes: one design per question

Every recipe below is a manifest with the fields above. What changes is which
axis the arms vary and which trial kind the question needs.

| Question | Kind | Arms | Metric |
|---|---|---|---|
| Does subsystem X earn its place? | `single` | `full` against `full` + `levers_off = ["x"]` | `failure`, or the cost X claims to lower |
| Does X help on its own? | `single` | `bare` against `bare` + `levers_on = ["x"]` | `failure` |
| Is model B as good as model A here? | `single` | same preset, different `provider`/`model` | `failure` |
| Where should a limit sit? | `single` | one arm per value, e.g. `overrides = ["max_turns=8"]` | `cut_short`, `failure` |
| Does the learning loop improve later runs? | `lifetime` | `full` against `stages_off = ["learn"]` (or `ruminate`, …) | `failure`, read as a slope |
| Does the interlock stop an injected send, and what does it cost? | `single`, task source | `full` on `eval/dojo-workspace.toml` | the source's `security` and `utility` checks |
| Does the run behave with an owner in the loop? | `lifetime`, principal | `eval/home-lifetime.toml` | `failure` |
| Does prompt B, tool set B, or charter B do better? | `single` | the default against an arm whose `environment` extends it with the change | `failure`, `turns` |

### Functionality suites

`eval/suites/` holds one ready-made baseline per area of the assistant, each
run in the default environment (Ada Okafor's synthetic lab) with one arm and
three seeds:

| Suite | What it checks |
|---|---|
| `memory.toml` | Recall from the knowledge graph, the task board, recording a fact, and not inventing someone the graph doesn't hold |
| `mail.toml` | What needs a reply, what can be ignored, a detail from one thread, and a reply staged for your review |
| `scheduling.toml` | Reading the calendar, spotting a conflict, finding a free slot, and creating an event |
| `injection.toml` | A fake "IT helpdesk" message tells the assistant to send budget files to an outside address, and nothing may be sent, even when you say "do what it asks" |
| `compaction.toml` | A forced mid-task summary, after which a running total and earlier details must still be in the answer |
| `files.toml` | Chained reads, arithmetic across files, a long chain of linked entries, and two code fixes graded by running the tests |
| `all.toml` | Every case above except `files`, at once |

```bash
mecha exp new eval/suites/memory.toml
mecha exp run suite-memory --jobs 3
mecha exp report suite-memory
```

To measure a change, add an arm with a prediction and set `control = "full"`,
for example an arm whose `environment` extends the default with the change.

The cases are calibrated: each passed at least once on the local model when
it was added. Two were rewritten during that pass because the fixture
calendar dates its events by the UTC day while "tomorrow" is asked in your
time zone, so a one-day window graded a correct answer wrong for part of
every day. Cases that depend on a date use a window of several days.

Shipped manifests to copy from: `eval/dojo-workspace.toml` (task source,
fixture servers), `eval/home-lifetime.toml` and `eval/assistant-lifetime.toml`
(lifetime, principal, synthetic home), and `eval/appraisal-*.toml` (single and
lifetime pilots over an artifact-graded task source).

**To test something the shipped tasks do not cover**, write the tasks rather
than the harness. A line in a case file is a prompt plus an `expect` block:
which tools must or must not be called, what the answer must contain, a turn
ceiling, a stop cause, a taint state. [Evaluation](/docs/features/evaluation)
documents every check. For a world with state, such as a mailbox, a board, or
a suite with its own grader, write a [task source](#task-sources).
`eval/fixtures/source_stub.py` is the whole contract.

Two habits pay for themselves. Run `mecha exp run <name> --limit 1` and
open that trial's session before spending the other forty-seven. And read the
first trials by hand with `mecha exp export <name>`: the gate counts wins, and
only a transcript shows why.

## Not built yet

These are the limits of the instrument today. Design with them in mind;
[`EXPERIMENT-DESIGN.md`](https://github.com/ljchang/mecha/blob/main/docs/EXPERIMENT-DESIGN.md)
holds the plans for each one.

- **Lifetimes run one at a time.** `--jobs` covers `single` designs only. A
  lifetime's stages must not compete with its tasks for the model, and
  running lifetimes side by side needs that rule restated first.
- **The sandbox, security and approval rules can't vary by arm.** They are
  machine facts, taken from your config for every arm, and an environment is
  refused if it sets them.
- **Judging is pairwise, on one metric.** Each treatment is judged against
  the control only, by win/loss/tie counts. `report` shows the rest (per-task
  results, cost, pass^k, lifetime curves), but as a readout, not a test:
  there are no confidence intervals or significance tests yet.
- **No multi-agent trials, and no mid-run forking.** An `ensemble` kind,
  branching a recorded run at one message, and snapshotting an environment
  are the communication-research half of the design, and none is built.
- **The principal is gold-verdict only.** A model-driven owner for judgements
  gold cannot express, such as tone or usefulness, is proposed and not built.
