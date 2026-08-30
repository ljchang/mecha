# Making the learning loop autonomous and self-measuring — research

*2026-08-29. The question: mecha has 43 reflections and zero rules. flowmail
had neither a human gate nor that problem. What does flowmail's loop actually
do, what does mecha's do, and what has to change so that learning runs without
the owner and can be shown to be working.*

Companion documents: `LEARNING-AUTONOMY-DESIGN.md` holds the **decision** to
ungate (2026-08-19, Luke) and the per-domain argument; this document is the
prior art behind it, the build-status finding that motivates re-opening it, and
the one thing that decision does not cover — how anyone would know the loop is
improving anything. `MEMORY-RESEARCH.md` is the evidence for curation over
accumulation. `SELF-IMPROVEMENT-RESEARCH.md` and `candidate.rs` are the harness
loop this proposes to converge on.

---

## 0. The finding that reframes the question

**The decision to remove the human gate was already made on 2026-08-19 and has
not been built.** `LEARNING-AUTONOMY-DESIGN.md` §0 opens: "Luke's decision:
learning in mecha is ungated in every domain. A rule goes live when it is
derived, not when someone approves it."

Ten days later `scripts/ruminate.sh` still runs:

```
"$MECHA" learn -p "$PROVIDER" --holdout 0.25 --propose
```

and `commands/learn.rs`'s module docs still say "Unattended learning — the
nightly timer — should always propose." Parts of the design did ship — the
`PASS_DOMAINS`/`routed_domains()` split, `normalized_rule_key` retirement
inheritance, `finalize_rules` carry-forward — but the gate itself did not.

The observable consequence, measured 2026-08-29:

| | |
|---|---|
| reflections on disk | 43 (29 clean, 12 untrusted, 2 harness-voice) |
| rules live | **0** |
| proposals pending | 4, holding 16 rules, oldest 6 days |
| reflections claimed by pending proposals | 27 of 43 |
| `LeapRun` records ever | 1, dated 2026-08-04 |

This is the settled-and-unbuilt shape. Nothing below re-opens the ungating
decision; it takes it as given and asks what else has to be true.

### The queue is self-stalling, which is why it looks stuck rather than slow

Three failures compound, and each is individually reasonable:

1. **Proposals claim their reflections.** 27 of 43 are marked as belonging to a
   pending proposal and are not reconsidered, so fresh reflections keep falling
   below `LEARN_MIN_REFLECTIONS` (3). Last night: `behavior: 2 unprocessed
   reflection(s), below --min 3; skipping`.
2. **Every proposal is a full rewrite computed against `rules_before: []`.**
   `proposals::accept` correctly refuses to apply a proposal whose baseline
   moved, so accepting any one of the four makes the other three unacceptable.
   The queue renders as four decisions and can drain exactly one.
3. **Nothing notices.** `mecha doctor` reports nothing about learning. Its
   starved-learner check measures *origin exclusion*, not review latency, so
   six days of stalled review is indistinguishable from a healthy loop.

(3) is the project's own "nothing went wrong and nothing happened are opposite
findings" rule, failing in the direction it warns about.

---

## 1. What flowmail actually does

Two loops, not one. `dev_docs/CORRECTION_SYSTEM.md` documents only the first.

### 1a. Compaction — accumulate, then abstract on volume

```
correction recorded (with sender, subject, snippet, card)
   ↓  every 30 uncompacted corrections
LLM reads the batch + existing learned rules
   ↓
full replacement of the learned rule set, live immediately
```

Triggered by **volume, not a clock** (`COMPACTION_THRESHOLD = 30`), fired
inline from `correct_classification` as a spawned task. The triage prompt is
three tiers: user rules, then learned rules, then the **last 10 corrections
verbatim with their full context**. That last tier matters — it is
`MEMORY-RESEARCH.md`'s "verbatim beats extraction" finding, implemented.

### 1b. Rumination — a scored search over candidate rules

This is the half the correction doc does not mention, in `ai/rumination.rs` and
`commands/ai.rs`. Per correction:

1. Generate **N candidate reflexions** (`num_candidates`, default 3).
2. Compute a **baseline** once: classify a sampled test set
   (`test_set_size`, default 4) with no candidate injected, scoring each
   against ground truth.
3. For each candidate, inject it into the system prompt
   (`augment_context_with_candidate`) and re-classify:
   - `+trigger_weight` (default 2) if it fixes the email that was corrected
   - `+1` for each test email it turns wrong→right
   - `−1` for each it turns right→wrong
4. **Promote every candidate scoring above `promotion_threshold` (default 0).**
   No human. The run and every candidate's score are recorded whether promoted
   or not.

Every constant lives in a `learning_settings` table — tunable at runtime.

### What flowmail does *not* have

No validation ledger, no per-rule attribution, no retirement, no holdout, no
work guardrail, and rules carry a model-authored `confidence` float. Its
"Verification" section is build-time testing — unit, integration, manual E2E —
not a runtime check that the rule set is still good. Once promoted, a flowmail
rule is only removed by the next full replacement.

---

## 2. The comparison that sets the design

| | flowmail | mecha today |
|---|---|---|
| Trigger | volume (30 corrections) | clock (nightly) + volume floor (3) |
| Candidates | **N generated, scored, best promoted** | 1 rewrite, pass/fail |
| Evidence | replay against **labelled ground truth** | replay judged by a model |
| Baseline | computed once, real A/B | current rule block, real A/B |
| Accept | score > 0, automatic | **human** |
| Holdout | none | `--holdout 0.25`, feeds `validate` but **not the gate** |
| Retirement | none | attributed regressions ≥ 3, bisected |
| Ledger | run + per-candidate scores | `ValidationRecord` → `rule_tallies` |

Read as one sentence: **flowmail is a search with cheap ground truth and no
memory of what failed; mecha is a filter with expensive evidence, a good
memory, and a person in the way.**

Neither is the target. The target takes flowmail's *shape* (generate several,
score, promote automatically) and keeps mecha's *ledger* (attribute, retire,
never re-derive).

### mecha already has the better gate, and it guards the smaller risk

`candidate.rs` grades harness changes and auto-applies config wins. Set its
gate beside the learning gate in `commands/learn.rs`:

| | `candidate::judge` (harness) | the learn gate |
|---|---|---|
| Holdout | deterministic FNV split, `MIN_HOLDOUT_PAIRS = 4` | not consulted |
| Sample floor | `MIN_SELECTION_PAIRS = 8` | none — `measured == 0` still yields `pending` |
| Work guardrail | `WORK_FLOOR = 0.75` | none |
| Verdict | `Accept` / `Propose(why)` / `Reject(why)` | `regressed > 0` → reject, else `pending` |
| Autonomy | auto-applies `Config` and `Prose` | always a human |

The weaker gate guards the change with the **wider blast radius** — a
`behavior` rule rides in every run's cached prefix of an agent with tools, a
network and a way to send, which is `LEARNING-AUTONOMY-DESIGN.md` §3's stated
cost. A rule change is closest to `ChangeClass::Prose`, which that gate already
auto-accepts.

**Recommendation: one gate, two callers.** Reuse `candidate::judge` rather than
growing a second disposition model. It brings the holdout, the sample floor and
the work guardrail for free, and it makes "why did this land" answerable in the
same words for both loops.

### The crux: all four pending proposals measured nothing

Every one ends `no trace-gradeable reflections in this batch; review by
reading`. Sixteen rules, zero evidence. Two causes:

- **Fixed but unexercised.** `ask_user` is registered only by a front-end that
  owns a human, so the CLI replay registry bailed on every interactive session
  — "246 of 408 sessions" per `a_surface_only_tool_fills_a_gap_under_stop_and_never_otherwise`.
  Closed 2026-08-27 by the `surface_only` stand-in (`57a122e`, `110e969`,
  `c7ab860`); confirmed present in the installed binary. No proposal has been
  generated since, so it has never run in anger.
- **Structural.** `Trigger::Edit` has no replayable intervention point and
  never will; `Trigger::Followup` is judge-graded only. The gate's allowlist is
  steers and denials.
- **The pre-point lottery** (found and closed 2026-08-29, after the two
  above). Even with the registry fixed, the probe *regenerated* the whole
  prefix and required the model to reproduce it call-for-call before the
  intervention: on a sampled model the odds decay with the point's depth, and
  the first full pass lost 11 of 12 steer probes to `inconclusive: diverged
  at call #1` against points at #10–#28. Closed by branching
  (`counterfactual::branch_at` + `replay_run::drive_branch`): the recorded
  prefix is resubmitted verbatim, steering text stripped, and the model
  generates only the continuation — pre-point divergence is now structurally
  impossible, and the forced prefix reads from the server's KV cache instead
  of being re-decided. Known residual sensitivity: a steer *pass* still
  requires tracking the steered continuation call-for-call, so long
  post-steer windows bias toward `Fail`/`Fail` — but both arms carry the same
  bias, and the comparison between them is what the ledger keys on.

This is the decision that actually forks the build, and §5 puts it plainly.

---

## 3. The basal ganglia framing is load-bearing, not decorative

Reading the store as a cortico-striatal system is not an analogy for the
documentation — it predicts which parts are missing.

| Basal ganglia | mecha | Status |
|---|---|---|
| Parallel segregated loops (motor, oculomotor, prefrontal, limbic) sharing one circuit motif | `domain` — same reflect/learn/validate/retire machinery, separate rule sets and prompts | **built**, and the reason this generalises beyond mail |
| Actor (striatal policy) | the rule block in the prompt | built |
| Critic (value estimate) | the validation ledger, `rule_tallies` | built for `behavior`, absent elsewhere |
| Phasic dopamine / reward prediction error | `GoalError` — signed error per `Channel`, tagged with `Agency` | built, **largely unread** |
| Tonic dopamine / setpoints | `homeostat.rs` | built, ships with no consumer |
| Go pathway (D1, promote) | `mecha learn` | gated shut |
| NoGo pathway (D2, suppress) | retirement at 3 attributed regressions | built, has never fired |
| Eligibility trace / credit assignment | `attributed_rule_id` via bisection in `mecha validate` | built |

Three things fall out of the table.

**The loop, not the rule, is the unit of scoping.** The user's instinct that
mecha is "more general-purpose than flowmail" is exactly the segregated-loops
property: `PASS_DOMAINS` vs `RUN_DOMAINS` already distinguishes a domain read
by one named pass from one riding in every run. That is the mechanism for
adding loops without cross-talk, and it is the thing flowmail cannot do — its
learning is triage-shaped throughout.

**Go and NoGo must be symmetric or the loop drifts.** Today promotion is
human-gated and retirement has never fired. Ungating promotion without
exercising retirement replaces a stuck system with a ratchet. Retirement is the
load-bearing half after the gate goes, which is why
`LEARNING-AUTONOMY-DESIGN.md` §2 makes thresholds *stricter* where evidence is
weaker — the right instinct, still unbuilt (`min_attributed` is uniform).

**`behavior` is a dumping ground.** 41 of 43 reflections land in it, because
`Trigger::domain()` maps everything except `Edit` there. One loop is carrying
mail steers, coding corrections and chat redirections in a single rule set that
rides in every prompt. That is one striatal loop wired to every cortical area —
the pathology the segregation exists to prevent.

The constraint on splitting it: **the cached prefix is sacred.** Per-run domain
selection would change early bytes and re-pay the prefix every run. Per
*surface* selection does not — a mail pass, a coding run and a chat session
each get a stable domain list, so each keeps its own stable prefix. That is
`PASS_DOMAINS` generalised, and it is the only shape of this that is affordable.

---

## 4. Nothing measures whether the system is improving

Neither design document answers this, and it is the user's sharpest question.
What exists today is entirely **local**: `rule_tallies` scores one rule,
`candidate::Judgement` scores one candidate. There is no number for "is mecha
getting better," and therefore no way to notice that ungating made things worse.

The substrate is already on disk and unread:

- **487 session transcripts**, each with a `RunStats` outcome record —
  `stop_cause`, `tool_calls`, `tool_errors`, `tool_denied`, `blocked_sends`,
  `compactions`, turns and usage.
- **`Channel::Intervention`** — a human steered, denied or corrected. If
  learning works, this falls.
- **`Channel::Edit`** — whose own doc comment in `appraisal.rs` reads: an
  outbox draft edited before it went, "or **sent unchanged**, which is the one
  channel in this system that says something went well and was recorded for the
  whole life of the outbox with nothing reading it."

That last one is the finding. The single positive signal mecha records has
never been consumed. **Sent-unchanged rate is the `writing` domain's accuracy
metric**, already collected, free.

### Proposed: `mecha learning report`

Four series, all derived from records that already exist, none requiring a
model call:

1. **Intervention rate** — interventions per run, per domain, over a trailing
   window. The headline. Falling means the loop works.
2. **Sent-unchanged rate** — outbox items released without edit, over time.
   `writing`'s ground truth.
3. **Rule tenure survival** — how long rules live before retirement, and how
   many are never validated. A loop minting rules that are retired promptly is
   churning, not learning; a loop whose rules are never validated is guessing.
4. **Gate disposition mix** — accept / propose / reject per night. Sudden
   all-accept is the oscillation hazard of `LEARNING-AUTONOMY-DESIGN.md` §5
   showing up as a number.

**The honest caveat, stated once.** These are observational and uncontrolled.
The corpus is one owner's real work, so the mix of tasks moves under the
metric; a falling intervention rate could mean better rules or an easier week.
It is a monitor for catching regression, not proof of improvement. The
controlled claim comes from `mecha eval --ab-rules`, which already runs a case
set rules-free and rules-on, and that — not this report — is what should be
cited as evidence that rules help. The report's job is to notice when something
breaks between evals.

---

## 5. What to build, and the decisions still open

**Status, 2026-08-29 evening.** Rungs 1, 2 and 4 shipped, and the live/nightly
split below was taken further than this plan proposed: mining was *already*
live (a `session_end` hook), so what moved was consolidation. Rules now go
live when derived, per session; the nightly keeps the expensive half. Rung 3
(`learn --auto`, one gate for both loops) and rungs 6–7 remain.

**Status, 2026-08-30.** Precondition 1 below is discharged — after cutover,
which is the wrong order and this line owns it. `scripts/retirement-drill.sh`
now runs the seeded end-to-end proof this section asked for (bad rule →
regressed probe → bisection → `propose-retirements` convicting at the
probation leash of 2), and its **first run found that the leash could never
fire**: probation released on bare ledger coverage, which conviction evidence
always supplies, so every probationary rule answered to the ordinary
threshold of 3. Fixed the same day
(`release_probation_when_measured_clean`); the drill is the standing check.
The prediction this section made — "a NoGo pathway that has never fired is
an untested backstop" — was exact.

What landed: `proposals supersede` (releases reflections **unconsumed** —
`reject` marks them processed, which would have burned 27 real corrections);
`rules propose-retirements --apply` (per-rule removal with no queue — `git
revert` over the whole store had been the only path, and no rule had ever been
removed); the validation ledger rendered into the learner's prompt so a
full-replacement rewrite drops what measured badly rather than guessing;
`mecha learning-report` plus `/api/settings/learning-report` and a trend pane
in the web learning settings. First consolidation since 2026-08-04:
28 reflections → **12 live rules**.

**Recommended order.** Each rung is independently useful and reversible.

1. **Unstall the queue.** Supersede rather than accumulate: a new proposal for
   a domain resolves its predecessors as `superseded`, releasing their claimed
   reflections. Add a doctor check for review latency and for proposals whose
   `rules_before` no longer matches. Cheap, and it works whether or not the
   gate goes.
2. **Add the tracking layer (§4) before ungating, not after.** A loop that
   starts accepting its own output with no baseline series has nothing to
   compare against when it goes wrong. This is the item with the highest ratio
   of value to risk, and it is the one thing neither design document covers.
3. **Unify the gate on `candidate::judge`.** Brings the holdout, the sample
   floor and the work guardrail to learning; makes disposition language uniform.
4. **Make retirement fire.** The stricter per-domain thresholds of
   `LEARNING-AUTONOMY-DESIGN.md` §2, plus the probation marker and its own
   threshold from D1 — and a seeded end-to-end proof that a bad rule is
   actually convicted and retired. **Precondition for step 5**, not a
   follow-up: under D1 and D3 this is the only thing in front of a bad rule.
5. **Ungate all three domains** per `LEARNING-AUTONOMY-DESIGN.md` §0, cadence
   per its §1.
6. **Adopt flowmail's N-candidate search** — generate several rule sets, score,
   promote the best. This is the largest quality win available and the most
   expensive: N× inference per pass. Worth doing after the ledger can show
   whether it paid.
7. **Split `behavior` by surface** (§3), once the report can show whether the
   split helped.

### Decisions (Luke, 2026-08-29)

**D1 — the zero-measurement case: accept on probation.** A rule the gate could
not measure goes live carrying a probation marker; retirement applies a
stricter threshold to it and the ledger corrects it. This is the
basal-ganglia answer — act, and let prediction error correct — chosen over
holding (which reproduces today's stall, since unmeasurable batches are the
common case) and over refusing (which would permanently give up the `writing`
and `followup` half of the corpus, as edits have no replayable intervention
point and never will).

**D3 — all three domains cut over together**, matching §0's decision as
literally written. `triage` is not used as a proving ground first.

**What D1 and D3 together imply, stated once.** Both rulings move load onto the
same component. Probation means unmeasured rules live until the ledger convicts
them; cutting all three domains at once means `behavior` — the widest blast
radius and the weakest evidence in the system — carries probationary rules from
the first night. **Retirement is now the only thing standing between a bad rule
and every future run's cached prefix, and it has never fired once.** So two
items stop being recommendations and become preconditions:

1. **Retirement must be exercised before cutover**, not merely implemented —
   proven to fire end to end on a seeded regression, with the stricter
   per-domain thresholds of `LEARNING-AUTONOMY-DESIGN.md` §2 in place. A NoGo
   pathway that has never fired is an untested backstop, and under D1 it is the
   only one.
2. **The tracking layer (§4) ships first**, so the intervention-rate and
   rule-tenure series have a pre-cutover baseline. Without it there is no
   before-picture to compare against, and "did ungating make things worse" is
   unanswerable rather than merely unanswered.

Neither is a hedge against the decision; they are what makes it safe to take.

### Still open

**D2 — ground truth for `behavior`.** flowmail's loop works because corrections
are free labels. `behavior` has none, which is why its probes are judge-graded
and why `LEARNING-AUTONOMY-DESIGN.md` §3 calls it the weakest evidence in the
system. Either accept that (and lean on stricter retirement), or invest in a
held-out labelled corpus of behavior cases — expensive, and the only thing that
would make `behavior` as measurable as `triage`.

### Deliberately not proposed

- **No model-rated confidence.** flowmail's rules carry one; mecha does not
  build policy on a model's opinion of itself, and the ledger supplies the real
  thing. Unchanged from `LEARNING-AUTONOMY-DESIGN.md` §6.
- **No relaxation of the provenance gate.** `Origin` still excludes untrusted
  reflections from `RUN_DOMAINS`. Autonomy is about who *approves* a rule, not
  about what may become one — two different gates, and only one is under
  discussion.
- **Nothing outside the learning store.** The outbox, the interlock and the
  approver are untouched. No run gains a capability it did not have.
