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
`harness ruminate` uses, and a preset — `bare` is what eval runs, `full` is
every lever on. An unknown lever name is a load error. The `approval_rules`
lever is refused: a `forbid` in your rules file is your standing word, and an
experiment does not lift it.

The control carries no prediction. Every other arm must, and the metric is
always a cost — the task outcome enters as `failure` (`1 − passed`), the rest
are the gate's own (`turns`, `tool_error_rate`, `cut_short`, `compactions`,
`ended_on_failed_call`, `malformed_args`).

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
as its working directory. That home's `config.toml` *is* the arm: your whole
config with every inline provider key scrubbed (the environment variable the
key names still reaches the child), your sandbox, security and approval rules
intact, and the arm's switches and knobs applied — so nothing about an arm is
ambient, and your `forbid` list still stands. Your learning store, skills and
charter are copied into the arm's home once, when it is first created, so a
`full` arm runs the harness as your machine has it; nothing is ever copied
back. The child gets a clean environment with only what it needs, the runner
refuses a home that is, or contains, your real one.

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
knowledge graph over MCP. A `full` arm carries your live servers into the
trial home, so without more a release would send from your account and a
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
starting state before the run; `grade <task>` reads the run's `--json`
result on stdin and prints a verdict with the checks behind it. The driver
calls each with `MECHA_HOME`, `MECHA_FIXTURES` (the home's fixture-store
root), `MECHA_EXPERIMENT_WORKSPACE` and `MECHA_EXPERIMENT_TASK` set, and
every edge fails the trial rather than passing it: a non-zero exit, a
timeout, no JSON, an unknown shape, or a verdict that disagrees with its own
checks. `eval/fixtures/source_stub.py` is the whole contract in forty lines.

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
