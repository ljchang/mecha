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
learn = 5
validate = 5
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
`learn --holdout 0.25 --auto`, `harness ruminate`, the nightly's own order
and flags, so what a lifetime measures is the loop that ships (validate
measures before learn consumes) — one after another and never beside a
task, and writes
each to the lifetime's **stage ledger** (`stages/<lifetime>.jsonl`) with its
exit status and where its output went. The ledger is what says a stage ran;
the manifest says only what was scheduled. Resume reads both: a finished task
is not rerun, and a stage the ledger lacks after a finished position runs
before the next task starts.

**Stage levers** are a second closed set, beside the per-run levers, and a
lifetime's arm may name them off in `stages_off`: `reflect`, `learn`,
`validate`, `ruminate`, and `sensors_in_brief` — the last is not a stage but
the switch (`[agent] sensors_in_brief`) that hands the homeostat's and guilt's
readings to the diagnostician's brief, which is those sensors' only reader.
A `single` manifest refuses them. Stage levers off are part of a row's
condition hash; an arm with every stage on hashes as its single-trial twin.

`status` shows each lifetime's sequence by position (`✓ ✗ ! ~ ·`) with its
stage counts; `judge` pairs positions across arms as it pairs tasks, since the
sequence is shared. Read the trajectory, not the mean: a loop that learns has
a slope.

One limit worth knowing before spending a night on it: every reflection
trigger today is an owner's act — a steer, a denial, a follow-up, an edited
draft. A lifetime run without anyone answering produces none, so `reflect` and
`learn` are exercised but mine nothing; `harness ruminate`, which reads run
counters, is the one stage with an effect until the principal (the owner's
simulator) lands.
