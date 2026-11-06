# Driven runs: a stronger agent drives mecha on mecha's own work — design

**Status: 2026-11-06, designed and ruled (§9, R1–R5). Nothing here is
built.**

What is being designed: a loop in which an outside agent (Claude Code, at
the owner's desk) gives mecha real work, mostly changes to mecha itself,
grades the artifact, and **attributes each failure to the harness or to the
model** by rerunning it under other models. Harness defects become PRs with a
stated prediction. Model gaps become training data. New failure shapes
become eval cases.

This builds on four documents and restates none of them:

- `EXPERIMENT-DESIGN.md`: the instrument. §16 is the principal simulator,
  and this design is its model-driven second version. §21.1 is the task
  source contract (`list` / `setup` / `grade`). §9 lists the confounds of
  this machine.
- `SELF-IMPROVEMENT-RESEARCH.md`: §10 (best-of-N over a fixed corpus is a
  multiple-comparisons trap) and §14.7 (fitness is the owner's own usage,
  and a public benchmark is a transfer check only).
- `CODE-MODE-DESIGN.md` §13: M2–M6, and the conclusion that on the battery
  the remaining gap is the local model (Qwen 31/40, Claude 20/20 in the same
  harness).
- `POST-TRAINING-RESEARCH.md` §4 and §7: training data comes first, and it
  has no source yet beyond fixtures.

---

## 0. Why now

Harness rumination has never had a candidate accepted
(`HARNESS-EVOLUTION-DESIGN.md` §5.0). The reason is a starved sensor, not a
broken proposer: the corpus's only dense failure signal is the owner
stepping in. The code-mode arc showed the alternative. M2 through M6 were a
driver giving mecha tasks with frozen graders, and each pass found a
harness defect that the corpus had never shown: `open()`, walks, cut notices
read as "0 failures", `key=d.get`. That arc ended because its battery
saturated, not because the method did. The battery was single-turn
analysis. Last week's real sessions made no `code` call, and long multi-turn
repository work has not been measured at all.

## 1. The loop

```
 task bank (pinned commit, prompt, hidden grade; frozen by sha before any run)
        │
        ▼
 driver ──opens──▶ mecha run, kind = driven, jailed to a worktree at the pin
        │                       │
        │◀── transcript + diff ─┘
        ▼
 grade the ARTIFACT (hidden tests), never the run's own report
        │ fail
        ▼
 attribute (§4) ─┬─ harness defect     → PR + falsifiable prediction → mecha exp → owner merges
                 ├─ harness affordance → same, scoped to the models that need it
                 ├─ model gap          → (failed, corrected) trajectory pair → SFT bank
                 └─ new shape          → eval case
```

There are two modes, and they never trade places:

- **Exploration.** The driver works interactively, one task at a time, and
  reads every transcript. It produces the failure ledger and hypotheses.
  **It never produces a rate.** A number from exploration is a count of
  what was seen, and the driver chose what to look at.
- **Measurement.** `mecha exp` over the task source (§3), with arms that vary
  the model or the harness change under test, on tasks the change was not
  derived from. This is the only mode whose numbers go in a PR description
  or a design doc's results section.

## 2. The session kind

**D1. A thirteenth `SessionKind`, `Driven`, wire form `"driven"`.** It marks
a session whose principal is another agent rather than the owner. It is
settable through `MECHA_SESSION_KIND=driven`, which makes three marks a
process may put on a session. `SessionKind::test_override`'s comment says
"these two and no more", and D1 changes that rule deliberately:

- `Test` is a development run, and nothing about it should be studied.
- `Experiment` is a trial inside a trial home, read by `mecha exp`.
- `Driven` runs in the **real** home, against real repositories, under a
  real config. It is worth studying, but it is not the owner's usage.

Readers follow the `Experiment` shape. Every corpus reader hides driven
sessions by default (`hidden_driven` beside `hidden_tests` and the
experiment twin) and admits them on an explicit flag. Fitness (§14.7) never
counts them. Two readers want them on: this design's ledger, and appraisal
calibration (§6).

**D2. Driven sessions feed learning in a pool of their own.** The owner's
ruling (R2, 2026-11-06) rejected excluding them. This is the one place the
learning loop can be tuned against ground truth, and excluding it would
throw that away. Three parts:

- **The driver's interventions are labelled as the driver's, never as the
  owner's.** A driver's steer or denial is a new `Origin::Driven`, not
  `Clean`. It is not `Derived` either, because those are mecha's own words
  and this is another agent's. That is the same split the three refusal
  strings keep: no approver can label a machine's verdict as a human
  correction. `Origin` is written to an append-only store, which makes it a
  wire format, so an older build reads the new variant as untrusted.
  Untrusted taint still excludes as before, and driven runs over the repo,
  with no network, are normally clean.
- **Rules mined from driven sessions are scoped to driven runs.** Each one
  carries `surface = driven` (`situation.rs`), so it renders into a driven
  run's rules block and never into the owner's chat, Slack or triggers.
  This is where `Driven` departs from `Test` on purpose. The test override
  marks the record and never the match. The driven mark reaches the match
  too, because the pool's separation depends on it.
- **The grade is the gate `Derived` was waiting for.** `Origin::Derived`'s
  own documentation calls it "one gate away from being usable the day
  something can grade it". A driven run's hidden test is that grade. A
  driven-pool rule is validated by `mecha exp` with the `learned_rules`
  lever on against off, on held-out driven tasks with hidden tests. That
  measures an outcome, where `counterfactual.rs` can only ask whether a
  trajectory changed. So the whole self-updating machinery (reflect, learn,
  validate, retirement) gets exercised at a volume the owner's sessions
  cannot supply, and every verdict is grounded in a passing test.

**Promotion crosses the owner.** A driven-pool rule enters the owner's pool
only by the owner's act, with its measured lift attached, and the rule text
is shown as written. It loses its `surface = driven` scope in that one step,
and nothing does it automatically. The existing self-accept gate for learned
rules does not extend to promotion, because that gate measures on the
owner's own sessions and a driven task is not one. Whether promotion can
later earn a gate of its own is a question for once the pool has a track
record (§8).

## 3. The task bank

**D3. The first source is mecha's own merged history.** It follows
SWE-bench's construction. For a merged PR with tests:

- the pin is the PR's first parent;
- the prompt is written from the PR's description and linked issue, without
  the diff;
- the grade is the PR's own new or changed tests applied over the run's
  diff, plus the touched crate's existing suite for regressions.

The ground truth already exists, it is the owner's real distribution of
work, and nothing in it is private: the repository is public, and
`check-private.py` already gates it. That last point makes it the cleanest
training data available (§5).

Task selection is stratified and seeded (`sample.rs`), by size (lines
changed) and by subsystem. The driver does not hand-pick. Whatever the
driver reads during exploration moves to a **seen** set and never enters a
measurement holdout.

The second source is **the open leftovers**: the minors that review passes
left, as `HANDOFF.md` and the PR threads list them. These have no ground
truth, so the driver writes the hidden test and freezes it by sha before the
first run. The grader-v2 practice applies: a grade revised after reading
answers is reported beside the frozen one, never instead of it.

The third and fourth sources are **tasks the owner and the driver write
together** and **tasks the driver writes alone**, such as a feature the
harness lacks or a bug reproduced from a transcript. The freeze rule applies
the same way. Every task records its author (`history`, `leftover`,
`co-written`, `driver`), and results are reported per author. A task the
driver wrote alone has its prompt and its grade from the same hand, so it
can show that the harness improved but never that it improved *for the
owner's work*. Only `history` and `co-written` carry that claim.

All sources are served through §21.1's contract as
`eval/fixtures/mecha_history.py`: `setup` makes a worktree at the pin, and
`grade` applies the hidden tests and runs them.

Terminal-Bench and the other public suites come **after** this loop has
tuned the harness, as transfer checks (§14.7). Their place in this design is
to be the number that tells whether the tuning overfitted to mecha's own
repository.

## 4. Attribution: harness or model

The defining step. A failure is rerun under a **panel** with the same
harness, task and jail:

| role | model (router preset or provider) |
|---|---|
| default | `qwen3.6-35b-a3b-uncensored` |
| local peer | `qwen3.8-27b` |
| local peer, other family | `gemma-4-26b-a4b` |
| ceiling | `claude-opus-5` (Anthropic) |

**The default is the model the owner actually runs**, not the router's
startup preset. Fitness is the owner's usage (§14.7), so the harness is
tuned on the model that usage runs. It is also usually the resident model,
so exploration does not evict it. The peers are base models, because the
panel's job is to tell harness from model, and a base peer says that most
plainly. One consequence: a harness affordance (D4) that the default needs
and `qwen3.8-27b` does not could be an effect of the uncensoring rather than
of the family. Before such a result is called general, a measurement arm
checks it against `qwen3.6-35b-a3b`, the default's own base. The abliterated
variants are left out: their GGUFs drop the MTP head, so they also run at a
different speed.

**D4. Three harness-side verdicts, not one.** "Harness or model" is not
binary. The R12 `open()` shim was a harness change made because of a model
habit:

- **Harness defect**: the panel fails in the same way, the ceiling included.
  The fix is for everyone.
- **Harness affordance**: the ceiling passes and the local models fail in the
  same way, on something the harness can meet them on (a shim, a cut notice,
  a description line). The fix is justified by the default model and
  measured on it.
- **Model gap**: the ceiling passes and the local models fail differently, or
  on reading and judgement (M4–M6's docstring decoy). No harness change.
  The pair goes to the SFT bank.
- One local model failing while its peers pass is **model-specific**, and is
  fixed only if that model is the default.

**D5. Fork at the turn.** For a failed transcript, `replay_run` re-serves the
recorded tool results up to turn *k*, then hands the rest to the ceiling.
The earliest *k* from which the ceiling recovers names the turn that lost the
run. That is a turn-level label for the SFT pair, and a pointer to what the
harness showed the model there. This works only for turns whose tool results
are recorded: `replay_run` does not yet answer a program's host calls from
the record (`CODE-MODE-DESIGN.md` §7, owed), so a fork inside a `code` call
waits for that work.

**The router's confounds.** These are additions to `EXPERIMENT-DESIGN.md` §9:

- Switching a preset evicts the resident model. Every switch makes the owner's
  next chat wait for a load, and it goes through `hold.rs`. Exploration runs
  on the default, which is normally resident, and the peers run only for
  attribution, batched by model. Each batch ends by restoring the owner's
  pick with `mecha model use`, because a router restart comes back on the
  startup preset rather than on that pick. A batch is announced to live peer
  sessions before it starts, since the resident model is machine state.
- Presets differ in `context_window`, so compaction counts differ for
  reasons that have nothing to do with the model's skill. Each trial records
  its compaction count and its served model, the latter asked through
  `served_model` rather than asserted.
- The ceiling runs at a different speed and cost. Wall-clock time and tokens
  are compared within a model, never across.

## 5. What the loop produces

1. **Harness PRs.** Each one opens with its prediction, in `candidate.rs`'s
   shape: which tasks move, in which direction, under which models. The
   prediction is measured by `mecha exp` on the holdout before review is
   asked for. Merging remains the owner's call. The driver is a lane, and a
   lane does not promote itself.
2. **The SFT bank.** Each entry pairs a failed local trajectory with the
   ceiling's passing one on the same task, with the fork turn from D5. These
   are rendered later with the router's template (`POST-TRAINING-RESEARCH.md`
   §7 step 1). A pair is kept only when the ceiling's run passed the hidden
   grade, because a ceiling run that reads well but fails the tests teaches
   the wrong thing.
3. **Eval cases.** These are failure shapes that are not about one task
   (a compaction that dropped the plan, a retry loop that `boredom` did not
   name) and that `eval/cases.jsonl` can hold deterministically.
4. **The ledger.** One row per failure: the task, the panel's verdicts, the
   D4 class, the fork turn, and what became of it. It lives in the
   experiment store beside the trials it came from, not in a document.

## 6. The principal, and the seam for appraisal

In a multi-turn driven run, the driver is §16's model-driven principal. It
steers, answers `questions`, and denies calls, always through the owner's own
verbs, so the stores record exactly what they would record from a person.
§16's first rule holds: **a verdict comes from gold wherever gold exists**.
A task is closed `done` by its hidden grade, never by the driver's opinion.

The seam that matters for the v2 appraisal loop: every driven run carries an
**external grade** joined to its session id. Today appraisal infers how a
run went from the record, with no ground truth to calibrate against. Driven
runs provide that ground truth at volume, so the question "does the
appraisal's valence agree with whether the tests passed?" becomes answerable.
This design records the join and builds nothing on top of it. What v2 reads
from it is v2's design.

## 7. Guardrails

- **The jail stays the jail.** The run is jailed to its worktree. A worktree
  never contains the mecha home, so `setup`'s refusal does not fire. Shell
  runs under bwrap, with no network. `cargo` therefore runs `--offline`
  against a registry fetched once outside the run, with read access granted
  through `grant.rs` (§8, open).
- **No push, ever.** A driven run has no git remote credentials, and the
  outbox gets no route for it. Its output is a diff in a worktree.
- **No persona data and no mail.** The task bank is code. Fixture-based
  assistant tasks are a later source and use fixtures only. A driver reading
  the owner's real mail into a run would build a trifecta session for no
  reason.
- **Peers.** Announce a model batch, check `.git/worktrees/*/MERGE_HEAD`
  before touching a shared tree, and give every worktree its own path.

## 8. Open, and named

- **Build cost.** A cold `cargo build` per trial worktree is minutes, and the
  disk cost is large. The likely answer is a shared `CARGO_TARGET_DIR` per
  pin with trials run sequentially within a pin, but the cargo caps in
  OPERATIONS need checking first.
- **Cargo inside the jail**: whether an offline registry plus one read grant
  is enough, or whether the toolchain under `~/.rustup` needs one too.
- **Prompt leakage from PR descriptions.** A description often names the fix.
  Prompts are written from the issue or the "why" paragraph, and the rule for
  what to strip is fixed before the first run.
- **The ceiling's cost.** About 10 tasks × k=3 × one ceiling rerun per
  failure is small. The full bank is not, so measurement arms run the ceiling
  on a seeded subset.

- **Promotion's own gate.** When, if ever, a driven-pool rule could enter
  the owner's pool behind a measurement rather than the owner's act. Not
  before the pool has validated rules whose lift held on a later holdout.

## 9. Rulings — the owner's, 2026-11-06

- **R1. Yes.** D1: a third environment-settable kind, `driven`, hidden by
  default from every corpus reader.
- **R2. Learning reads driven sessions, in a pool of their own.** This
  replaces the draft's exclusion. The owner's reason: driven work is exactly
  what would make the self-updating system useful, and excluding it would
  waste that. D2 is the shape.
- **R3. All four sources.** Merged-PR history, the leftovers, tasks written
  with the owner, and the driver's own. Each records its author (§3).
- **R4. Yes.** The panel in §4, including Anthropic spend for the ceiling.
  The default is the owner's daily model, `qwen3.6-35b-a3b-uncensored`; the
  peers are base models.
- **R5. Yes.** The pilot: build D1, then about 10 history tasks in
  exploration with the full panel, with the ledger as the deliverable. No
  harness PR from the pilot until the D4 split is in. D2's pool follows D1,
  before the first validation run.

## 10. Deliberately out of scope

- **An unattended loop.** No trigger or timer starts driven runs. The driver
  is at the owner's desk, and the owner merges.
- **RL**, and any training run. This design fills the bank; it does not
  spend it.
- **Skills or charter lines** from driven runs. The standing rulings apply
  (`SKILLS-RESEARCH.md` §9, and the charter's author rule). Learned rules
  are in scope, in their own pool (D2).
- **Public benchmarks as fitness.** They are transfer checks, later (§3).
