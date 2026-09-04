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
vary the closed set: levers by name (the list is
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

Only `single` trials run today — one run per arm × task × seed × repetition.
A `lifetime` manifest (an ordered task sequence sharing one home, with the
learning loop's stages scheduled between tasks) loads and is refused by name
until its driver lands.
