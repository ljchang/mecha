# Rollouts: continuing a branch past the recording

2026-10-18. The engine that lets a branch keep going after it departs from
what was recorded, then scores how it went. It runs on two schedules:
- **offline** over recorded sessions, which is replay that can follow a new
  path;
- **online** at a live decision point, which is preplay.

**Status: designed, nothing built.** §11 lists the rulings the build waits
on.

## 0. Why, and what this builds on

**The owner's framing (2026-10-18).** A branch should "continue the talks
under the new alternative branch and score how well it went". Preplay is the
same engine, "just running online in parallel instead of replayed after the
session is over". And the world should be live, "as every important thing
always needs to be approved".

**What this answers.** No harness change rumination has proposed has ever
been accepted (`HARNESS-EVOLUTION-DESIGN.md` §5.0). §1 shows the structural
reason: every instrument mecha has stops where a better path would start.

**Built on, not restated:**
- **`APPRAISAL-INVENTORY-RESEARCH.md` §10 "Online feasibility"** already
  sketched in-run branching:
  - read-only tools run live, sends stage into a scratch outbox, and writes
    need a scratch copy of the jail;
  - `Egress::Chosen` is not branchable;
  - K = 2;
  - the conversation takes the **union** of every branch's taint;
  - a losing branch still teaches.

  This document is that sketch made buildable, and extended offline.
- **`APPRAISAL-WIRING-DESIGN.md` N3** parked lookahead and speculative
  execution "until a decision has a structural validator". This document
  keeps that condition: online rollouts run only at decisions §7.1 names a
  structural validator for, and *imagined* lookahead stays parked (§10).
- **`BRANCHING-DESIGN.md`.** Only the in-memory mechanism shipped:
  `counterfactual::branch_at` plus `replay_run::drive_branch`.
  `Record::Branch` and chains are unbuilt, and nothing here needs them.
- **`EXPERIMENT-DESIGN.md` §16, the principal.** The gold principal is
  built and the model-driven one is not. Multi-turn rollouts depend on it
  (§4.3).
- **`HARNESS-EVOLUTION-DESIGN.md`.** P2 (the null arm) becomes the control
  arm (§5.1). P8 and P11 point here.

The research behind this document is summarised in §9, with links.

---

## 1. What exists, verified against `880ebbc59`

| What | Where | What it means here |
|---|---|---|
| A branch is one `run_in` over the recording's tail | `replay_run::drive_branch`; `counterfactual::truncate_after_run` cuts before the next owner turn | Nothing continues past the current run |
| Tool calls after the branch are answered from the recording, and the first unanswerable call stops the arm | `probe::PROBE_MODE = OnDivergence::Stop`; pointwise likewise | A new path ends at its first new call |
| Validators reward *reproducing* the recording | `counterfactual` (steer: "tracks the recording"), `pointwise` (`StructuralSteer`, `ReleasedDraft`, …) | Right for learning's question ("would it do what the owner steered toward?"); structurally unable to approve a better, different path. `pointwise.rs`: "past the first divergence there is no world left to grade it in" |
| Arms are capped at `HORIZON_TURNS` = 4, and a capped arm reads as `CutShort` | `pointwise::HORIZON_TURNS`, `probe::within_horizon` | The cap is confounded with a real ceiling |
| **An argument change is answered from the recording** | `ReplayTool::decide`: an unbatched call is paired by name alone, and a batch falls back to name when the arguments differ | A branch that searches for something *different* is handed the old search's results |
| `Conversation::resumed` drops the goal anchor and harness notes | `agent.rs` | A branch runs without the recorded goal |
| Sessions with step checks are refused | `replay::ensure_replayable` | Exactly the sessions with the best scoring evidence are excluded |
| `OnDivergence::Live` exists, but only in `mecha replay` | `commands/replay.rs` | All-or-nothing at the first divergence, against the real world: see §1.1 |
| No "changes the world" axis | `Tool::read_only()` means "skip the approver", not "has no effects" | Eight tools that write claim it. In §3.2's terms: six `StoreWrite` tools, `image_generate` (`Render`) and `http_fetch` (`Irreversible`) |
| Reads that write | Graph reads append `query_log` and `retrieval_touch` (`INCOGNITO-DESIGN.md` §3.3; the unrecorded-read mode in §5.2 is unbuilt); token refresh rewrites token files | A rollout's reads are not free |
| Store roots outside `RunContext` | questions, hud, personas, imagelib, the forecast ledger and the shell registry resolve from `mecha_home()` | Two redirects exist, at different points. The outbox is per run (`RunContext::with_outbox`; `OutboxStore::open_like`, which Slack already uses per thread). `message_send` binds its route when the registry is built (`MessageSendTool::new(route)`), so a rollout's registry can bind a scratch `MailboxRoute`. The rest are per process only |
| No copy-on-write workspace | Branches use the recorded workspace; `mismatch::drive` stages an empty temp dir | Branch writes would land in the real workspace |
| Uneven seats and voice-call checks | Ruminate's replay arms and `validate` take no seat (`permit::Permits`); only `replay_persona::wait_for_owner` checks for a live call | A night of rollouts could stutter a call |

### 1.1 A defect found on the way, independent of this design

> **Fix in flight:** PR #713 (branch `fix/live-replay-keeps-guards`), phase
> 0. Its shape is the owner's ruling of 2026-10-18, below.

`mecha replay --on-divergence live` builds its run with
`RunContext::new(tool_ctx, approver)`: **no outbox route, no hooks, no
approval rules.** After a divergence, a tool `[outbox] tools` normally
stages goes to the approver as an ordinary call instead.
- On a machine whose `permission_mode` is `allow`, the call executes: mail is
  sent, a calendar event is created.
- On this machine `permission_mode` is unset (the default), so the call is
  blocked. But tools that write while claiming `read_only()` (§3.2) skip the
  approver and execute either way.

This is the silently-degrading-guard shape, and it is fixed ahead of
everything else (phase 0, §8).

**The fix: live until the first consequential call** (owner's ruling,
2026-10-18).
- After the divergence, a live replay runs only **pure reads that nothing
  configured guards**. It stops in front of the first call that is routed,
  rule-gated, hook-watched or a write, and reports that call and its
  arguments. Nothing is executed or staged.
- The owner's reason: by the time a branch reaches a consequential action,
  whether it is promising is already visible. The call is the result.
- **The guards are asked as a gate inside the replay
  (`replay_run::LiveGate`), not attached to the run.** Attached, they act
  ahead of the tool. A recorded call they intercept *before* the divergence
  then never reaches the recording's cursor, and the replay dies one call
  later and runs the rest live. That was #713's first shape, and its review
  caught it.
- A live replay built without a gate executes nothing past its divergence.
- This is the first slice of §3's dry world (live reads, halt at everything
  else). Phases 1–2 add the scratch outbox and workspace on top of it.

---

## 2. The engine in one paragraph

A **rollout** is five things:
1. a **branch prefix**: a recorded prefix, or a live conversation's current
   state. "Seed" is kept for the sampler's random seed throughout;
2. a **world** that answers every call the recording cannot, without
   consequences (§3);
3. a **driver** that keeps the agent going past the branch, across turns
   once a principal exists (§4);
4. a **horizon** that bounds it;
5. a **scorer** that grades the outcome from structure and artifacts,
   never from a model's opinion of itself (§5).

A **comparison** of a harness *change* is always at least three rollouts
from one branch prefix, in one world, at one time:
- a **control** under the current harness;
- a **second control** under the current harness with a different sampling
  seed;
- one **arm per candidate**.

Online preplay (§7) is a different question: it chooses between *actions*
under one harness, not between harnesses. Its arms are two samples of the
current harness, and their disagreement is the thing it exploits, not
noise to subtract. The gain test (they agree, so nothing is rolled out)
does the control's job there, and §7.1's structural validator decides.

The two controls give the noise floor. The **offline** schedule (§6) runs
comparisons over recorded points at night. The **online** schedule (§7) runs
them at a live decision point and acts on the winner. Same code, different
clock.

---

## 3. The world: live reads, sunk writes

### 3.1 The recorded world comes first, exactly

- Before the branch, the branch prefix is the recording, as `branch_at` builds it
  today.
- After the branch, a call is answered from the recording only when **its
  name and its arguments both match** (`replay::same_arguments`) and the
  rollout has not yet departed. This is stricter than `ReplayTool::decide`
  is today (§1).
- Anything else goes to the dry world. The first such call marks the
  rollout as departed, and from then on every call goes to the dry world.
  Mixing recorded and present answers after a departure would stitch two
  worlds together.

### 3.2 A closed effect class per tool

A new axis, separate from `read_only()`.

It has a second consumer: `CODE-MODE-DESIGN.md` §2 admits a tool into a
program by this class, so the table is built once, for both.

- **Built-in tools and first-party servers declare it in mecha's code.**
  That covers `mecha-mail`, `mecha-docs` and the graph's pinned revision.
- **For a third-party MCP tool it is earned from the operator, never
  claimed by the server.**
  - Inside a rollout, the `RolloutApprover` decides from this table alone.
    A server's own `readOnlyHint` would therefore be the only thing between
    an unattended run and a real write.
  - So an MCP tool is *undeclared*, and halts, until the operator's
    `[[mcp]]` entry names its class. This is the rule `Egress::Blind`
    already follows: earned in code, never granted by the party it
    protects against.
- **A server's annotations may only move a tool toward caution.**
  `destructiveHint` makes it `Irreversible`, and `readOnlyHint` never
  promotes it. An operator's class may likewise be overridden only toward
  caution.

| Effect | What the dry world does | Examples today |
|---|---|---|
| `Read` | Executes for real. **A rollout's path jail *is* its scratch workspace (`Workspace` below)**, so `fs_*` reads see the branch's own writes, never the pre-branch tree. "For real" means the world outside the workspace: mail, calendar, documents, search | `fs_read`, `fs_list`, `document_read`, `mail_search`, `mail_get_thread`, `calendar_list_events`, `docs_read`, `web_search` (`Blind`) |
| `ReadThatRecords` | Executes in an **unrecorded** mode, which is built before rollouts touch the graph (ruling **W1**): a rollout never nudges recency and never halts at a graph read | graph reads (`query_log`, `retrieval_touch`) |
| `Staged` | Stages into a **scratch outbox** opened per rollout (`OutboxStore::open_like`). The model gets the same "Drafted, not sent" answer it gets live. The draft is kept for scoring and discarded with the rollout. A scratch outbox that cannot be opened halts the rollout: a staging failure never executes | `mail_send`, `mail_reply`, `calendar_*_event`, `docs_append`/`replace`, `sheets_write`, `factory__*` publishes |
| `Workspace` | Executes against a **scratch copy** of the workspace taken at branch time. Shell runs sandboxed against the copy. The copy lives where run workspaces already live, under `~/.mecha/work/<producer>/` (`work.rs`). The rule it must obey is `work::ensure_outside_mecha_home`'s: a jail may never *contain* the mecha home. A root under it is the normal case | `fs_write`, `fs_edit`, confined `shell` |
| `StoreWrite` | Not executed: the rollout **halts** at the call, and the halt is recorded (ruling **W2**). No synthetic "recorded, not delivered" answer. `message_send` is the exception, delivered through a scratch `MailboxRoute` into a store nobody reads | `message_send`, `ask_user` (headless), `hud_propose`, `persona_propose`, `image_library_propose`/`_reference` |
| `Render` | Not executed: the rollout halts at the call (a render costs the GPU, and a fake picture misleads the continuation) | `image_generate`, video |
| `Irreversible` | The rollout **halts before the call**, and the halt is recorded with the tool's name. A halt is information, not a failure | `calendar_hold`, `mail_triage`, `docs_create`, `docs_trash`, graph writes, unconfined `shell`, any `Egress::Chosen` call (`http_fetch`) |
| *(undeclared)* | Halts. Unknown is never safe | MCP tools without annotations |

**One class per tool, decided in this order, first match wins.** The table
must be a partition, because it is the `RolloutApprover`'s only authority.
Its rows overlap as written: `mail_send` is routed *and* declares
`openWorldHint` (so `Chosen`), and `calendar_delete_event` is routed *and*
`destructiveHint`.
1. **Routed by the operator's outbox: `Staged`.** A routed call never
   executes in any world. That is also why the loop skips the interlock for
   it today.
2. **Undeclared: its own row, and it halts.**
3. **Any `Chosen` egress: `Irreversible`.**
4. **Jailed to the workspace: `Workspace`.** `fs_write`, `fs_edit` and
   confined `shell` all declare `destructive`, and confinement narrows only
   their egress. So this row must come **before** the destructive one, or
   the scratch-workspace half of the design is unreachable.
5. **Any other `destructive` tool: `Irreversible`.**
6. `Render`.
7. `StoreWrite`, by name. This includes `image_library_reference`, which
   declares `destructive` but writes only to the library store.
8. `ReadThatRecords`, by name (the graph's reads).
9. `Read`.

Eight classes, with `Irreversible` reached from two rules.

**Approval inside a rollout.** The dry world **narrows the run's
configured guards; it never replaces them.** These are the losses §1.1
exists to fix.
- A rollout carries the operator's approval rules (`policy::ExecPolicy`)
  and `pre_tool` hooks exactly as `setup` builds them for a real run, in the
  same order.
- A `forbid`, or a hook's denial, halts the rollout. This is `trial_env`'s
  rule: an environment cannot lift a `forbid` any more than an arm can.
- Only after those does the `RolloutApprover` apply this table, in place of
  the absent human.
- The table can only refuse more. It never permits what a rule or a hook
  refused.
- It never blocks and continues: a "blocked" result teaches the
  continuation something false about the world, so a call that cannot run
  halts the rollout. `consult` and `escalate` both halt.

### 3.3 Taint, locality, pacing

- **Taint.** A rollout starts from the branch prefix's taint, conservatively, as
  `drive_branch` already does (unknown provenance counts as external). The
  interlock runs unchanged inside it.
  - Online, the live conversation afterwards carries the **union** of every
    rollout's taint, because the chooser saw them all
    (`APPRAISAL-INVENTORY-RESEARCH.md` §10).
  - **A rollout is never evidence for learning or for the run-quality
    corpus, and that is enforced by structure, not by label.**
    `Origin::Derived` alone would not do: it is a label that a later
    grader could promote.
    - An offline rollout writes **no session file**, as probe arms do
      today. Its only record is the comparison (§5.3), which no miner
      reads.
    - Online, the losing branches are never appended to the live session.
      The appraisal note about a loser is templated from its structural
      fields ("the alternative failed its check"), never from its text.
    - The winner's continuation *is* the live run, and is recorded as
      that run.
    - The live run's `RunStats` absorbs the winner only, never the losers'
      costs. Those stay in the comparison record, **and** the run records
      its preplay spend as its own counter (rollouts, tokens, seat-seconds).
      Otherwise a preplay-on run would look exactly as cheap as a
      preplay-off one in every corpus readout, including the counters that
      propose the next candidate.
    - **A kind tag alone would not hold.** `runlog::Scan::admits` drops
      only `Some(Test)` and `Some(Experiment)`. An unknown variant reads as
      `None`, and `None` is *admitted*, so an older build would count a new
      `SessionKind::Rollout` in the corpus.
    - Any future driver that does record a rollout session therefore does
      what experiments do: it writes into a **separate home** (an isolated
      `MECHA_HOME` with its `HOME_MARKER`) that no corpus readout of the
      owner's home ever walks. It tags the kind as well, as a second line.
      The 46-of-143 incident is the reason.
- **Locality.** Rollouts drive only a loopback provider.
  `pointwise::on_this_machine` (R29) moves from the per-pass check into the
  engine, so no caller can forget it.
- **Pacing.**
  - A rollout takes a background seat (`permit::Permits`) like every other
    background model job.
  - It waits on a live voice call the way `replay_persona::wait_for_owner`
    does.
  - It never runs while an interactive turn is waiting for the seat.

### 3.4 Drift, offline only

A recorded point from last week runs against this week's mailbox. The design
neutralises drift instead of pretending it away:
- **Rollouts are compared with each other, never with the recording.** Every
  arm of a comparison runs in the same present world at the same time, so
  drift is common to all arms and cancels in the difference.
- **Drift is measured by a shadow read.** §3.1 answers an identical re-read
  from the recording, so the drift check has to be a deliberate extra call.
  - At most once per rollout, at the first re-read of a `Read`-class call
    with identical arguments before departure, the dry world also issues
    that call live.
  - It compares the live answer with the recorded one, and sets `drifted`
    when they differ.
  - The model is still handed the recorded answer, so the shadow read never
    changes the continuation.
  - One real read per rollout is the price, counted like any other.
- **Point age is capped** (ruling **W4**): an offline point may be at most
  14 days old to start, revisited once `drifted` rates are measured.

Learning's existing counterfactual gate is the one place that keeps
comparing with the recording, because its question is about the recording.
It is unchanged.

---

## 4. The driver

### 4.1 Branch prefix

- `branch_at`'s prefix, with **the goal anchor, harness notes and path
  grants restored**: a new `Conversation::resumed_from` that carries what
  `resumed` drops today.
  - Grants matter as much as the anchor. A branch from a run that held a
    grant would otherwise depart on a jail refusal the recording never saw,
    which is instrument noise charged to the candidate. The repair already
    exists: `session::Transcript` re-admits grants with `grant::readmit`.
  - **A restored grant is read-only in a rollout.** A grant points outside
    the scratch workspace by construction: `fs_write`/`fs_edit` resolve a
    granted path with `Access::Write`, and confined `shell` mounts it. So a
    write grant restored as-is would let an unattended branch write the
    owner's real tree. Write grants are restored as read grants. That
    downgrade is the protection: a write to a granted path then comes back
    as the jail's ordinary refusal (`resolve_for` with `Access::Write`), an
    error result the model sees. It is not a halt, because the per-tool
    effect table cannot tell a granted path from a workspace path.
  - The comparison records a write refused this way as `grant_refused`, so
    it reads as instrument noise and not as the candidate's failure.
  - Phase 2's exit test holds a write grant in its branch prefix, attempts
    the write, and checks two things: the real tree is byte-identical, and
    the call's result is the jail's refusal.
- The branch point is any assistant boundary, the constraint
  `BRANCHING-DESIGN.md` set.

### 4.2 Within the run

- The agent runs until it ends its turn, the world halts it (§3.2), or the
  horizon is reached.
- **The horizon** is a tool-call budget plus a turn cap. A rollout that
  reaches it stops with a new `StopCause::Horizon`, distinct from
  `MaxTurns`, so `Metric::CutShort` is no longer confounded with the cap.
- **Step checks.** `ensure_replayable` keeps refusing them for replay, and
  rollouts accept them: a check whose effect class is `Read` or `Workspace`
  runs in the dry world like any other call.

### 4.3 Across turns

- **Before departure**, the recorded next owner turn is still a true answer
  to the conversation, and the rollout may continue into it.
- **After departure**, nobody has said what the owner would say next.
  - **v1 ends the rollout at the turn boundary.**
  - Later, a principal answers. The gold principal applies where a fixture
    defines the answer (`EXPERIMENT-DESIGN.md` §16, rule 1). A model-played
    owner is not proposed here, because a scorer cannot tell a good
    continuation from one that flattered the simulated owner.

---

## 5. The scorer

### 5.1 Paired, with the noise measured every time

- Every comparison carries **two control rollouts**: the current harness,
  twice, with different seeds. Their disagreement is the noise floor *at
  that point*.
- A candidate's difference counts only beyond it.
- This is `HARNESS-EVOLUTION-DESIGN.md` P2's null arm, made part of every
  measurement instead of a separate study. It is required because the
  literature measured control forks diverging on more than 90% of forks
  under some serving setups (§9).

### 5.2 Signals, in the order they decide

**No model's opinion of its own work decides anything.** That is
N3's condition and `CLAUDE.md`'s hearsay rule.

1. **Hard outcomes, structural:**
   - ended on a failed call;
   - malformed arguments;
   - repeated a refused call (the `StructuralDenial` test);
   - repeated a known-futile call (`compact::target_of`, boredom's key);
   - tripped the interlock;
   - halted at `Irreversible`, recorded as information: neither win nor
     loss.
2. **Artifact checks, where gold exists:**
   - owner-bound criteria (`mismatch` `ArtifactGold`);
   - fixture postconditions (`fixture_check::FixtureCheck`);
   - declared step checks, which are one-sided (R11): a failure
     disqualifies, and a pass proves nothing.
3. **Drafts in the scratch outbox:**
   - deterministic checks: the recipient is on the thread, dates resolve,
     the reply goes to the right thread;
   - `forecast::base_rate` **ranks only and never decides alone**. The
     forecast stays sealed: neither the acting model nor the owner sees it at
     decision time. `sessions appraise` reads the ledger back afterwards,
     through `forecast::summarize`, as it does today.
4. **Cost, as a guard:** `RunStats` costs (turns, tokens, compactions), the
   existing work floor and the regression ceiling (`candidate.rs`).
5. **Valence:** `appraisal::live_readout`, negatives only. It has no
   positive channel without the owner.

### 5.3 The record

- `ComparisonStore` gains a continued outcome per arm:
  - the signal vector above;
  - the halt reason;
  - the horizon reached;
  - `drifted`;
  - the world mode;
  - a branch label.
- Closed sets only, no text: the comparison store's existing rule.
- Drafts from the scratch outbox are scored, then deleted with the rollout.
  Whether one may ever be shown to the owner is ruling **W5**.
- **Nothing here enters the run-quality corpus** (§3.3). A night of
  rollouts would otherwise land in the very counters that propose the next
  candidate.

---

## 6. Offline: replay that can follow a new path

### 6.1 Uses

- **Harness candidates.** Whole-run replay stays the screen for cost-only
  changes. Any candidate whose effect is behavioural is measured by
  comparisons at sampled points. R1 is ruled yes, so this replaces "nothing
  measurable" with a measurement. Fixture-world arms (`trial_env`) remain
  for tasks the owner's stores cannot host.
- **Learning, optionally later.** A steer probe can add a continuation
  score to its reproduction test: did the run go well after the model did
  the steered thing?
- **`HARNESS-EVOLUTION-DESIGN.md` P3's held-out probes.**

### 6.2 Which points: gain × need

Mattar & Daw's rule for which memories to replay maps onto this directly
(§9).
- **Gain.** Would the candidate change the action here at all? Before
  paying for continuations, sample one next action under each arm. If they
  agree, gain is zero: record a tie and continue nothing.
- **Need.** How often, and with what consequence, this kind of moment
  recurs:
  - the `Situation` class's frequency in the corpus;
  - weighted by what is at stake, with a send or a draft above a read.
- **Point sources:**
  - owner interventions (high gain by construction);
  - verdict-free futile-repeat points (`HARNESS-EVOLUTION-DESIGN.md` P8);
  - failed checks;
  - the moments before a staged draft.

### 6.3 Budget and the matched-compute check

- **Seat cost** is K arms (three or more) × horizon per point, on one
  background seat at about 45 tok/s. The nightly point count is set from
  that, not the other way round.
- **Coverage depends on the operator's outbox.** `Staged` exists only for
  tools `[outbox] tools` names (§3.2, ordering rule 1). On a machine that
  routes little, a send or a calendar change falls through to
  `Irreversible` and halts the branch at that call. That is the safe
  direction, but it shortens branches. The nightly readout should report
  how many rollouts halted at an unrouted write, so thin routing shows up
  as a number and not as mysteriously short rollouts.
- **A candidate must beat what more sampling buys.** Its gain counts only
  if it exceeds what a third control sample buys at the same seat time. The
  field's expected null is that matched-budget sampling erases harness
  "gains" (§9).

---

## 7. Online: preplay

### 7.1 Only where a structural validator exists (N3)

| Decision point | Its validator |
|---|---|
| Before staging a send or draft | §5.2's deterministic draft checks; forecast rank as tiebreak only |
| After a failed call, before the retry | The futile-repeat test, and whether the next attempt also fails |
| After a declared or owner-bound check fails | The check itself |

Free-form answers, opinions and conversation are not preplayed: nothing
structural can tell which continuation was better.

### 7.2 Shape

1. **Gain first.** Sample two next actions. If they agree, do nothing.
2. **Roll out each** for a horizon of at most two steps, in the dry world.
   The literature's best simulators degrade past one step, and real
   execution is preferred over imagined execution (§9).
3. **Choose by §5.2, with a halted arm ranked explicitly.** A halt is
   information, but it is not a score: nothing was measured about what came
   after it. The order is:
   - an arm whose structural validator passed;
   - then a halted arm;
   - then an arm whose validator failed.

   If no arm scored at all, preplay changes nothing: the live run takes its
   own next step, as if preplay had not run. A natural implementation that
   ranked a halted arm by its missing score would prefer exactly the
   continuation nothing was measured about.
4. **The live run commits only the winner's first action, then replans.**
   - The live run re-issues the winner's first call, with the same tool and
     arguments, through the normal path. A send stages into the owner's real
     outbox there, exactly as if no preplay had happened.
   - The winner's first call therefore runs twice, once in the dry world
     and once live. That is deliberate: a second real read is the price of
     never stitching two worlds together. Counted honestly, a preplay
     with K arms costs K real `Read`-class calls per step plus the live
     re-issue. That is nothing for a loopback model, and a real cost for a
     metered `[[search]]` backend. Preplay therefore counts a metered
     backend's calls against its budget, and stops rolling out when the
     budget is spent.
   - Nothing from the scratch world is adopted. Every rollout's scratch
     draft, the winner's included, is scored and deleted with it (§5.3).
     The live record is the live run's own, never a stitch of two worlds.
   - "Execute the first predicted step, then replan" is also the
     literature's horizon advice (§9).
   - Cost: K rollouts plus the live run, which was going to run anyway.
   - The loser is summarised into the run's appraisal ("the alternative …
     failed its check").
5. **Taint is the union** (§3.3).

**Preplay never sends.** It can stage a better draft. The owner still
crosses every send, as today.

### 7.3 Where it runs

- **v1: background and delegated runs only** (triggers, `tasks work`), and
  only when a seat is free.
- **Never in an interactive chat in v1.** Every rollout is extra generation
  while the owner waits (ruling **W3**).
- **It is off by default until an experiment shows it beats K-sample
  selection at equal seat-seconds.** The instrument for that is
  `EXPERIMENT-DESIGN.md`'s.

---

## 8. Build phases

Each phase ships and is measured before the next starts.

| Phase | What | Exit test |
|---|---|---|
| **0** | Fix §1.1: live until the first consequential call (`LiveGate`) | A routed call past the divergence stops the replay and executes nothing. **So do a rule-gated call and a hook-watched call**: the gate is tested as built from the run's configured route, rules *and* hooks, so one that dropped any of them fails. A recording full of routed calls that the model reproduces replays to its end. A replay built without a gate executes nothing past its divergence |
| **1** | The effect axis (§3.2): a closed enum, declared in code for built-in and first-party tools, earned from the operator for third-party MCP tools, annotations caution-only, undeclared halts. Exact-argument recorded matching for rollouts | Every registered tool has a class. A test registers a third-party tool that claims `readOnlyHint` with no operator class, and sees a halt |
| **2** | The dry world: scratch outbox, scratch workspace copy, store-write answers, halts, `RolloutApprover` behind the configured rules and hooks, R29 in the engine, seat and voice gating | Integration test, in the confinement tests' style: a rollout that tries every effect class leaves the real outbox, stores and workspace byte-identical. **And** a configured `forbid` rule and a denying `pre_tool` hook each halt a rollout at a call the effect table would have allowed. The byte-identical test cannot catch a dropped rule, because a scratch write leaves the real tree untouched either way |
| **3** | `drive_rollout`, single-turn: restored anchor, `StopCause::Horizon`, two controls, the `ComparisonStore` schema | **Measure the control noise floor on the live corpus first.** If the controls disagree more than candidates plausibly could, stop and fix sampling (seeded, pinned serving) before building on it |
| **4** | Offline wiring: harness candidates measured by comparisons; gain-gated points; verdict-free points | A behavioural candidate produces a measurement where today it produces "nothing measurable" |
| **5** | The learning gate gains a continuation score (optional) | Measured against today's reproduction-only gate on the same probes |
| **6** | Multi-turn through the gold principal, fixtures only | — |
| **7** | Online preplay on background runs | The matched-compute experiment (§7.3) |

---

## 9. Evidence

📄 preprint, 🔧 code, 📰 secondary. Read for this design on 2026-10-18.

- **Lookahead works in resettable worlds and degrades with depth.**
  - Tree search raised VisualWebArena from 24.5% to 37.0% at about 20× the
    model calls, and it resets the world by replaying the action prefix
    (📄 [2407.01476](https://arxiv.org/abs/2407.01476)).
  - WebDreamer's imagined lookahead works best at horizon 1, and depth 2–3
    is worse, from hallucinated futures (📄
    [2411.06559](https://arxiv.org/abs/2411.06559)).
  - GPT-4 predicts *environment-driven* state changes, the mail and
    calendar class, correctly only 24–50% of the time (📄
    [2406.06485](https://arxiv.org/abs/2406.06485)).

  These are the reasons for executed rather than imagined rollouts, and for
  short horizons.
- **Replaying against the present world is the hazard.**
  - The Replay Gap found 74–77% of model-swapped forks diverging at the
    first action after the fork, against 6–35% of same-model controls, and
    over 90% control divergence under FP8 serving (📄 abstract,
    [2608.08239](https://arxiv.org/abs/2608.08239)).
  - Cut-point replay (Chronicle, 📄
    [2609.20625](https://arxiv.org/abs/2609.20625)) and
    rebuild-from-log-then-regenerate (SymTrace, 📄
    [2608.25920](https://arxiv.org/abs/2608.25920)) are this design's §3.1.
- **Write interception.** ToolEmu's emulator rules (validate inputs, stay
  faithful to the spec, keep state consistent) and GoEx's undo and
  confinement are the prior art for §3.2 (📄
  [2309.15817](https://arxiv.org/abs/2309.15817),
  [2404.06921](https://arxiv.org/abs/2404.06921)). Speculative Actions
  limits speculation to idempotent, reversible or sandboxed effects (📄
  [2510.04371](https://arxiv.org/abs/2510.04371)).
- **Selection without labels fails in a known way.**
  - An in-band judge accepted 44% real regressions and rejected 38% of real
    improvements. The gap disappeared when success was checkable from the
    artifact (📄 [2607.25152](https://arxiv.org/abs/2607.25152)).
  - Verifier false positives cap resampling gains (📄
    [2411.17501](https://arxiv.org/abs/2411.17501)).
- **Matched compute.** Harness evolution scored below parallel sampling at
  the same budget, and +0.0 on held-out tasks (📄
  [2607.12227](https://arxiv.org/abs/2607.12227)). LATS cost about 55×
  more than a simple baseline that scored higher (📄
  [2407.01502](https://arxiv.org/abs/2407.01502)).
- **Neuroscience.**
  - Preplay: hippocampal sequences before a novel track predict its later
    place-cell order (Dragoi & Tonegawa, *Nature* 2011). Before each run,
    sequences lead from the animal's position to a remembered goal (Pfeiffer
    & Foster, *Nature* 2013).
  - Mattar & Daw (*Nat Neurosci* 2018) prioritise each replay by **gain ×
    need**, which yields forward replay before a run and reverse replay
    after reward. §6.2 takes the rule directly.
  - Its stated limit is that it does not produce preplay of unvisited
    paths. That is the gap §7's executed rollouts fill for an agent with a
    safe world.

---

## 10. Deliberately out of scope

- **Imagined lookahead**, where a model predicts what a tool would return,
  for mail, calendar or the graph. These are environment-driven changes,
  where simulators are right 24–50% of the time. N3 stays parked for
  imagined futures.
- **Tree search over the owner's world.** It costs 10–20× the model calls,
  and the seats cannot pay for it.
- **A judge model deciding a comparison.** It may annotate a record for
  reading, and never decide.
- **Executing an `Irreversible` or `Chosen` call in any rollout, for any
  reason.**
- **Learning from rollout transcripts.** They are excluded by structure
  (no session record, losers never appended; §3.3), not merely labelled.
- **Preplay in interactive chats in v1.**

---

## 11. Rulings

### 11.1 Ruled, 2026-10-19 (the owner took the recommendations)

- **W1, graph reads: build the unrecorded-read mode first**
  (`INCOGNITO-DESIGN.md` §5.2), before rollouts touch the graph. A graph
  read writes `query_log` and `retrieval_touch`; rollouts must neither nudge
  those recency signals nor halt at graph reads, which would lose most
  sessions, since the assistant goes to the graph first.
- **W2, store writes that claim to be read-only: halt.** The rollout stops
  at the call, which is honest and makes rollouts shorter; there is no
  synthetic "recorded, not delivered" answer for the model to mistake for a
  delivery. `message_send` needs no ruling: a rollout registry binds it to a
  scratch `MailboxRoute`, so it really is delivered, into a store nobody
  reads.
- **W4, point age: 14 days** to start, revisited once `drifted` rates are
  measured.

### 11.2 Owed

- **W3, interactive preplay.** Is preplay in a live chat ever wanted, at
  the latency it costs? v1 says no.
- **W5, showing the loser.** May a rollout's draft ever be shown to the
  owner, for example "I considered also replying to X"? v1 says no:
  rollout drafts are scored and deleted.
