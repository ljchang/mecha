# Driven runs: a stronger agent drives mecha on mecha's own work — design

**Status: 2026-11-06, designed and ruled (§9, R1–R6 and R8; R7 and R9 owed). D1 is
built in #848, not yet merged; nothing else is built.**

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

**D2. Driven sessions feed learning in a store of their own.** The owner's
ruling (R2, 2026-11-06) rejected excluding them. This is the one place the
learning loop can be tuned against ground truth, and excluding it would
throw that away. R6 (2026-11-06, after review) set the separation: **a
separate store, not a scope inside the owner's.**

Why a scope fails. The first draft kept driven rules in the owner's store,
scoped `surface = driven`. Review of #847 showed that every way that scope
can be lost *widens* the rule onto the owner's sessions:

- `Situation::scope` strips a `MARK_KINDS` surface, and a scope with no
  surface matches every run.
- A mined lesson is stamped with `GlobalOpts::surface`, never the
  environment mark, so a driven `mecha run` stamps `run`.
- `mecha exp` marks its children `experiment`, so a validation trial could
  never present `driven` anyway.
- `Situation::region` collapses a surface that disagrees across a batch to
  none.

That is the half-life argument `EXPERIMENT-DESIGN.md` D12 called
non-negotiable for trial homes: a rule learned inside a trial that landed in
the owner's store would ride every real run's cached prefix from then on.

The shape:

- **Its own directory.** Driven reflections and rules live in a learning
  store under its own path (`~/.mecha/learning-driven/` or similar; the
  build names it). In the real home, a driven run loads the owner's rules
  *and* the driven store's; that kind-gated load is the default for a
  real-home run only. An owner run never opens the driven store. Every
  failure goes dark: if the kind is not detected, a driven rule simply does
  not apply, and nothing reaches the owner's prefix. `Driven` stays in
  `Situation::MARK_KINDS`, because nothing needs it as a scope key.
- **The readers that notice a sick store must see it.** `doctor` walks
  `<home>/learning` by name, and `backlog` iterates a literal list of
  stores that includes `learning`. A driven store under its own path gets
  neither, and then a wedged driven store looks exactly like the design's
  chosen dark failure, while the ledger's attributions depend on driven
  rules actually applying. So `doctor` reads the driven store in its one
  pass. The driven store joins `backlog` only for what waits on the owner:
  promotion candidates, once measured.
- **Mining is its own pass.** `reflect` keeps hiding driven sessions from
  the owner's store (D1, #848), its outbox pass included: a draft a driven run staged is passed over even after the owner acts on it. A separate pass reads only driven
  sessions and writes only the driven store. The two passes share code and
  never share a destination.
- **The driver's interventions are labelled as the driver's, never as the
  owner's.** A driver's steer or denial gets a new `Origin::Driven`, never
  `Clean`. It is not `Derived` either, because those are mecha's own words
  and this is another agent's. That is the split the three refusal strings
  keep. **Taint is classified first, and `Driven` is stamped only over it.**
  `Origin` is one field and `classify_origin` answers `Clean` or
  `Untrusted`, so a stamp that *replaced* the classification would make a
  reflection mined after a `.from_outside()` result learnable, which is the
  one case `Untrusted` exists to exclude. The rule: an intervention whose
  covering taint is untrusted stays `Untrusted`, driver or not, and
  `Driven` replaces only `Clean`. The store boundary protects owner runs.
  This rule protects every later driven run's prefix, and through it the
  ledger's attributions. Three things the build owes, none true of today's
  code:
  - `Origin` has no `#[serde(other)]` arm, and its field default fires only
    when the key is absent, so today an older build reading
    `"origin":"driven"` *drops the reflection* (one corrupt line, one lost
    reflection). The variant needs a lenient read first, parked as
    `Untrusted` (the shape `surface_unread` and `de_lenient_kind` use).
  - **The destination enters the type.** `Reflexion::learnable(&self)`
    sees only the record, and "learnable only into the driven store" is a
    fact about the destination, which nothing on a `Reflexion` carries. A
    bare `Driven => true` arm would be true everywhere: three of its readers
    have no store context at all (`lesson_source::backfill_targets`,
    `doctor`, `probe`), and the `learn` gate is store-blind. So
    `learnable()` stays closed to `Driven`, returning false as it does for
    `Derived`, and a new `learnable_into(&self, store: StoreKind)` is the
    only place `Driven` is admitted, for `StoreKind::Driven` alone. The
    driven store's own consolidation path calls it, and the compiler finds
    every call site. This is the one place the design loosens a
    structurally fail-closed gate. The triage arm is not a precedent: its
    own documentation calls `RUN_DOMAINS` membership "a *proxy* for the
    consumer rather than the consumer itself" and names the residual it
    does not catch.
  - Three tests: an owner-store `learn` refuses a driven reflection; a
    store-agnostic reader (`learnable()`) never reports a driven reflection
    as learnable, which is the direction the triage residual shows actually
    rots; and a driven intervention under untrusted taint classifies
    `Untrusted` and is learnable into neither store.
- **The grade is the gate `Derived` was waiting for.** `Origin::Derived`'s
  own documentation calls it "one gate away from being usable the day
  something can grade it". A driven run's hidden test is that grade. A
  driven-store rule is validated by `mecha exp` on held-out driven tasks
  with hidden tests, and the contrast is **the owner's rules against the
  owner's rules plus the driven store's**. Comparing the `learned_rules`
  lever on against off would not do: `Lever::LearnedRules` switches the
  whole block, so on-against-off measures owner plus driven against
  nothing, and credits the owner's rules to the driven rule under test.
  **The mechanism is a composed seed, not a second store.** A trial is
  marked `experiment`, so the real home's kind-gated load would never open
  the driven store, and the treatment arm would quietly equal the control.
  Instead, the runner composes each arm's `learning/` before the trial:
  the control's holds the owner's rules, and the treatment's holds the
  owner's rules plus the driven rule or rules under test. `trial_env`
  already seeds one `learning/`, so no loader learns a new path. **A
  positive check guards against a vacuous null.** The treatment arm's run
  records must show the rule under test carried in the prefix (the run
  record's rules-carried set). A trial whose prefix lacks it is a failed
  trial, never a measured zero, because a store that failed to load reads
  exactly like an absence of lift. This measures an outcome, where
  `counterfactual.rs` can only ask whether a trajectory changed.

**Promotion crosses the owner.** A driven-store rule enters the owner's
store only by the owner's act: a copy, with its measured lift attached and
the rule text shown as written. Nothing does it automatically. The existing
self-accept gate for learned rules does not extend to promotion, because
that gate measures on the owner's own sessions and a driven task is not one.
Whether promotion can later earn a gate of its own is a question for once
the store has a track record (§8).

**Revisit when the v2 memory system rolls out (owner, 2026-11-06).** The
owner expects the v2 appraisal and memory loop, more tightly integrated with
memory and conceptual and semantic knowledge, to change how learned
knowledge is stored and scoped. This separate-store shape was chosen
against *today's* learning store and its scope semantics. When v2 lands,
re-read this D2 against v2's store before building on it further:
whether v2's representation can hold the driven/owner boundary
structurally (a boundary that fails dark, never wide), and whether
promotion becomes a v2 operation. Until then, D2 builds as written.

## 3. The task bank

**D3. The first source is mecha's own merged history.** It follows
SWE-bench's construction. For a merged PR with tests:

- the pin is the PR's first parent;
- the prompt is written from the PR's description and linked issue, without
  the diff;
- the grade is the PR's own new or changed tests applied over the run's
  diff, plus the touched crate's existing suite for regressions.

The ground truth already exists, and it is the owner's real distribution of
work. **Being public is not a privacy guarantee**, and neither is
`check-private.py`. The 2026-08-07 and 2026-08-25 rewrites both found
private data in this public repository, the second time real people used
as fixtures, one in a published docs page. And `check-private.py` reads a
staged diff, a commit range or a commit message, scoped to persona chat and
call text, so it never saw a PR body or an issue. The prompt's source is
exactly that text, it lives on GitHub where no `filter-repo` reaches, and a
prompt has the longest reach in this design: it goes to the ceiling's
provider on every rerun (§4), and it stays in the SFT bank (§5).

**D3b. A prompt is written, never copied, and is scrubbed before it is
admitted.** The driver writes each prompt fresh, from what the PR changed
and why, and quotes no PR or issue text. Each prompt is then checked before
the task is admitted: `check-private.py`'s matcher run over the prompt text
itself, and a read against CLAUDE.md's conventions rule that fixtures and
examples are made up, with no persona's name and no real person in them. A
prompt that matches is rewritten, never admitted with an
exception. The pins' trees were already scrubbed by the history rewrites;
the prompt is the one new text. Whether prompts also need the owner's eye
before a ceiling run is R9, owed.

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

**D3a. A task is admitted only when its grade discriminates.** Before a
task enters the bank, its hidden tests are run *k* times (k=5 by default)
on the pin, where they must fail every time, and *k* times on the merge,
where they must pass every time. The failure on the pin must also be the
one the test asserts, not a build error or an unrelated panic. A grade that
passes on the pin cannot tell a fix from no fix. A grade that is flaky on
the merge would charge its noise to the model. A race test needs *k*
because it fails only sometimes. The first prototype, #525 (one write per
record, a concurrent-writer test), failed 5/5 on the pin with "a merged or
torn line" and passed 5/5 on the merge.

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
checks it against `qwen3.6-35b-a3b`, the default's own base. The one
abliterated build in the catalog, `qwen3.8-27b-abliterated`, is left out:
it is a third flavour of the `qwen3.8-27b` family, and it tells the panel
nothing that the base does not.

**D4. Three harness-side verdicts, not one, and a residual.** "Harness or model" is not
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
- **Unattributed**: every panel outcome the four above do not cover. Two
  are expected. The ceiling fails while a local peer passes, which is the
  inverted result: it says the task or the grade is suspect, not the harness
  and not the model. Or the whole panel fails, each in a different way. The
  set is closed, and an outcome that fits none of the four is recorded as
  this, never forced into the nearest one, because forcing is how a harness
  PR gets written for a bad grader. No PR and no SFT pair come from an
  unattributed row. The task is re-read first, and a grade found wrong is
  fixed under the freeze rule (§3), with the old grade kept beside it.

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
   D4 class, the fork turn, and what became of it. **There are two ledgers,
   kept apart.** Measurement rows live in the experiment store beside the
   trials they came from. Exploration rows (the pilot's) live in a ledger of
   their own, every row marked `exploration`, because §1's caveat (an
   exploration count is what the driver chose to look at, never a rate) is
   exactly what gets lost when a row is filed among trials.

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

**D1b. A driven run drops the surfaces the owner's config attaches.** A
driven run is in the real home under the real config, and the global layer
is where the private surfaces live: `[[mcp]]` (the knowledge graph's `kg_*`
tools, mail and calendar), `[[hook]]`, `[[grant]]`, which is
global-file-only because it is the jail's way out, and `[messages]`, the
owner's mailbox between live sessions, where taint travels with a message
in both directions. Project layers are
narrowing-only, so the worktree's own `mecha.toml` is not the exposure; the
owner's own file is. An outside agent, driving prompts written from PR and
issue text, should not hold the owner's graph or run the owner's hooks.

So the driven mark forces those four off **structurally**, from the same
`harness::Lever` definitions `mecha eval` uses (`RunConfig::levers_off`),
rather than trusting each run to be configured right:

- **Off**: MCP servers, hooks, the owner's grants, and the mailbox
  (`Lever::Messages`). A driven run must not post into the owner's live
  sessions: a peer message is an influence channel, which is why a peer
  cannot grant escalation, and here the peer would be the driver. A model
  batch is announced to peers (below) by the driver, from outside the run,
  never by the run.
- **Kept**: builtins (under the interlock as always), the owner's learned
  rules, skills and charter. The harness is measured as the owner runs it,
  minus the private surfaces.
- **The charter stays on, and that is a stated cost (R8, the owner's).**
  Whatever rides in the system prompt leaves the machine on every ceiling
  rerun (§4's `claude-opus-5`), the charter included. §4 requires the same
  harness on every arm, so no block is dropped for the ceiling alone, which
  would confound D4. The owner ruled to keep the charter on every arm, so
  the harness is measured as the owner runs it, and accepted that it reaches
  the ceiling's provider, along with the rest of the system prompt and any
  transcript a D5 fork replays (§9, R8).
- **Kept on purpose: the outbox.** With the outbox on, a routed call stages
  rather than executes. Turning it off would make a routed call run
  directly, and staging is the safer of the two. A driven draft is set
  aside from the forecast (D1).

D1b lands before the first driven run, not with D1: the mark alone gives a
run nothing new. A run marked driven is refused at start if any forced-off
surface is still attached, because a guard that cannot apply stops the run
and never falls through. **A build trap for its tests:**
`SessionKind::test_override()` returns `None` under `cfg(test)` unless
`MECHA_KIND_PROBE_EXPECT` is set, and `ignore_kind_env_for_tests` latches
it off for a whole test binary. A test pinning "the driven mark forces these
off" must opt back in the way the kind probe does (a child process), or it
passes vacuously.

- **The jail stays the jail.** The run is jailed to its worktree. A worktree
  never contains the mecha home, so `setup`'s refusal does not fire. Shell
  runs under bwrap, with no network. `cargo` therefore runs `--offline`
  inside the jail, with **no grant**. `Lever::Grants` off means a run reaches
  no grant (standing or the conversation's) and can add none, and
  `grant::Source` has no run-scoped variant. A grant the driver named would
  also make a peer the author of a jail exception, the rule D1b invokes to
  drop the mailbox. So task setup, which is the harness and not the model or
  the driver, vendors the dependencies into the worktree before the run
  (`cargo vendor` plus a source replacement in the worktree's
  `.cargo/config.toml`), and the run builds offline from inside its own
  jail. Whether the toolchain under `~/.rustup` is readable under the shell
  sandbox is open (§8). If it is not, the answer is an owner ruling (R7),
  never a driver-named grant.
- **No push, ever.** A driven run has no git remote credentials. Its output
  is a diff in a worktree.
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
- **Cargo inside the jail**: the dependencies are vendored into the worktree
  by setup (§7). Open is whether the shell sandbox can read the toolchain
  under `~/.rustup`, measured by running `cargo build --offline` in a
  confined shell. If it cannot, R7 is owed.
- **Old trees and the 2026-11-04 history rewrite.** Commits from 2026-09-09
  to 2026-10-04 no longer build `mecha-core`'s tests. The rewrite's
  `--replace-text` changed `include_str!` paths in `mismatch.rs` to the
  post-10-04 fixture names, but replace-text renames no files, so those
  trees still hold the fixtures under the old path. Setup copies the two
  files from `origin/main:mecha-core/tests/fixtures/mismatch/` into the
  pin's tree and records the copy on the task. The check for any other
  case like it is an `include_str!` or `include_bytes!` to a missing file.
- **Grafting tests that live in the file they test.** Rust tests here sit
  in `mod tests` beside the code, so the hidden grade is the PR's *added*
  items inside a test module, copied into the run's copy of that file. A
  test that calls a new API needs that API named in the prompt, or a
  different but correct design fails the grade. That is the interface rule
  SWE-bench-style banks use, and the prompt writer applies it per task.
- **Prompt leakage from PR descriptions.** A description often names the fix.
  Prompts are written from the issue or the "why" paragraph, and the rule for
  what to strip is fixed before the first run.
- **The ceiling's cost.** About 10 tasks × k=3 × one ceiling rerun per
  failure is small. The full bank is not, so measurement arms run the ceiling
  on a seeded subset.

- **Promotion's own gate.** When, if ever, a driven-store rule could enter
  the owner's store behind a measurement rather than the owner's act. Not
  before the driven store has validated rules whose lift held on a later
  holdout.
- **v2 memory.** D2's store boundary is chosen against today's learning
  store, and the owner expects v2 to change it (D2, last paragraph).

## 9. Rulings — the owner's, 2026-11-06

- **R1. Yes.** D1: a third environment-settable kind, `driven`, hidden by
  default from every corpus reader.
- **R2. Learning reads driven sessions, in a store of their own.** This
  replaces the draft's exclusion. The owner's reason: driven work is exactly
  what would make the self-updating system useful, and excluding it would
  waste that. D2 is the shape.
- **R3. All four sources.** Merged-PR history, the leftovers, tasks written
  with the owner, and the driver's own. Each records its author (§3).
- **R4. Yes.** The panel in §4, including Anthropic spend for the ceiling.
  The default is the owner's daily model, `qwen3.6-35b-a3b-uncensored`; the
  peers are base models.
- **R5. Yes.** The pilot: build D1, then D1b (§7), then about 10 history tasks in
  exploration with the full panel, with the ledger as the deliverable. No
  harness PR from the pilot until the D4 split is in. D2's store follows D1,
  before the first validation run.
- **R6. A separate store, not a scope** (after review of #847). Driven
  rules live in their own learning store, which owner runs never open, so
  every failure leaves a driven rule dark rather than applying it to the
  owner's sessions. To be revisited when the v2 memory system rolls out (D2).
- **R7, owed.** If the shell sandbox cannot read the Rust toolchain, how a
  driven run builds: the owner's call, never a driver-named grant (§7).
- **R8. The charter stays on** on every arm of a driven run (owner,
  2026-11-06), including the ceiling's, where it leaves the machine. The
  recommendation had been off; the owner kept it, so driven runs measure the
  harness as the owner runs it (D1b). What reaches the ceiling's provider is
  wider than the charter: the whole system prompt (the owner's learned
  rules and the skills block too), and, for a D5 fork, the recorded
  transcript up to turn *k*, whose tool results are mostly public repo
  contents.
- **R9, owed.** Whether a written, scrubbed prompt (D3b) also needs the
  owner's eye before its first ceiling run.

## 10. Deliberately out of scope

- **An unattended loop.** No trigger or timer starts driven runs. The driver
  is at the owner's desk, and the owner merges.
- **RL**, and any training run. This design fills the bank; it does not
  spend it.
- **Skills or charter lines** from driven runs. The standing rulings apply
  (`SKILLS-RESEARCH.md` §9, and the charter's author rule). Learned rules
  are in scope, in their own store (D2).
- **Public benchmarks as fitness.** They are transfer checks, later (§3).
