---
title: Learning
sidebar_position: 1
description: How mecha mines your corrections into rules, gates them on provenance, and keeps measuring whether they still earn their place in the prompt.
---

# Learning

mecha learns how you want work done from the moments you stepped in. The signal
is already in the transcripts — a mid-run **steer**, a **denied** tool call, a
corrective **follow-up** turn are all recorded, so nothing new had to be
captured to start.

The cycle is four commands:

```bash
mecha reflect     # mine transcripts for interventions → one lesson each
mecha validate    # measure whether the current rules change an answer
mecha learn       # consolidate reflections into a rule set
mecha rules       # what each rule has measured, and what should retire
```

Everything lives in files under `~/.mecha/learning/`, which is a git
repository. `git log` is the learning history; `git revert` is the undo.

| Path | What it is |
|---|---|
| `reflections.jsonl` | Append-only evidence, each pointing at its transcript |
| `rules/<domain>.user.toml` | Yours. Never written by code, only read |
| `rules/<domain>.learned.toml` | Consolidation's output — edit or delete freely |
| `validations.jsonl` | Every attempted probe outcome, keyed to the rule set measured |
| `validation-attempts.jsonl` | Input identities, reasons and arm receipts used to schedule retries |
| `runs.jsonl` | One audit record per consolidation pass |
| `mined.jsonl`, `mined_outbox.jsonl`, `distilled.jsonl` | Idempotence ledgers |
| `proposals/<id>.json` | Rule changes waiting for a human |

## `mecha reflect` — interventions become reflections

Extraction from transcripts is pure code. Four kinds of intervention are
recognised:

- **Steer** — user text riding in the same message as tool results.
  Unambiguous: the user reached in mid-run to redirect.
- **Denial** — a tool result reading `Denied by the user: …`. A recorded
  rejected intent. (A hook denial reads `Blocked by a hook:` instead, and is
  deliberately not mined — machine policy is not a user correction.)
- **Follow-up turn** — a later user turn *may* be a correction or just the next
  task. Extraction only flags the candidate; the reflector decides, and is told
  to skip freely.
- **Mismatch** — a verified failure of an owner-bound task criterion or a
  declared check, read from the run's own harness metadata rather than from
  any tool's prose. The first trigger that needs no person to fire; capped at
  one per step and three per run.

Each candidate goes to one model call, which returns the reusable lesson behind
it. The result is appended to `reflections.jsonl` with the session id that
proves it.

```bash
mecha reflect --dry-run          # what would be mined, no model call, no writes
mecha reflect --limit 5
mecha reflect --sessions-dir /path/to/transcripts
```

A session whose reflections fail — a provider being down, usually — is left
**unmined** for a later run to retry rather than marked and silently lost. Every
writing pass takes the store's writer lock *before* reading what has been mined,
so two concurrent closes cannot mine the same session twice.

`mecha reflect` also mines the [outbox](/docs/features/security/outbox): an item that was
sent with edits yields a `writing`-domain reflection from `diff(staged, sent)`,
and a message draft you rejected **with a reason** (`mecha outbox reject <id>
--reason "…"`) yields a `behavior`-domain reflection (trigger `reject`) from
your words. When the draft was written while third-party content was in the
conversation, the reflector sees only your reason and the tool name, never the
draft.

### Reading the lessons before they are consolidated

```bash
mecha reflections                       # newest first
mecha reflections show 20260828T0915    # what happened, what was said, the lesson
mecha reflections edit 20260828T0915    # rewrite it in your own words
mecha reflections drop 20260828T0915 --reason "specific to one thread"
```

The store had no reader for a long time, and that is the wrong end of the
pipeline to be blind at. A rule is a *consolidation* of several lessons, so by
the time a proposal is reviewable the thing you wanted to disagree with has
already been merged with four others and rewritten. The lesson is where a
disagreement is cheap and precise.

`edit` is a **provenance promotion** rather than a text change: a lesson you
typed yourself skips the model that would otherwise have laundered third-party
bytes into it, which is the way an excluded reflection gets rescued rather than
merely lamented. `drop` is a flag and never a deletion, on the same rule retired
rules follow — a store that forgets its refusals offers the same lesson again
next pass with nothing to say it was already judged; `restore` undoes it.

Nothing in these verbs calls a model or touches the network, and every write
takes the store lock, so they are safe against a store the nightly is also
using. The `/learning` modal in the TUI drives exactly them, alongside
`mecha rules list --json`, and so does the
[web settings page](/docs/features/interfaces/web#settings-and-what-a-browser-may-write) —
the same two panes, the same verbs as child processes, so no surface can do
something to the store that the command line cannot.

### Whose mistake was it?

Not every correction is a lesson about how mecha behaves. If you say "no,
Dana moved to Lakeside Institute" and the knowledge graph mecha read still
listed her old employer, mecha did the right thing with wrong data. A rule
telling it to behave differently would teach it nothing true. So each
correction is placed by **what the run had actually read** before you stepped
in:

| What the run had read | Class | What happens |
|---|---|---|
| the wrong value | **data error** | the source is repaired, never a rule |
| the right value, and not the wrong one | **behaviour error** | a lesson `mecha learn` may mine |
| neither | **gap**, nobody's fault | recorded as something to look up, never a rule |

The reflector only copies the two values, word for word, out of your
correction. The class is decided by looking them up in the tool results the
run received, never by a model. A value that is not really in your words, or
not in anything the run said or read, makes the correction **unknown**. So
does a reflection recorded before this existed. Unknown corrections are
counted and never mined. A correction about *how* the work was done ("don't
run that", "shorter, please") names no fact, and is a behaviour lesson as
before.

`mecha learn` prints how many reflections it held back under each class, and
`mecha reflections` says why for each one. Editing a lesson into your own
words admits it whatever its class, the same way an edit rescues a lesson
held back for provenance. This is the agent's half of the knowledge graph's
error contract. The graph's half, superseding the wrong fact, runs when the
session is [distilled](/docs/features/memory/distillation).

## `mecha learn` — reflections become rules

Consolidation groups reflections by domain and situation, then rewrites the
rules for that region while preserving rules outside it. It absorbs new
lessons, merges overlapping rules, and resolves contradictions. The stored
`rules/<domain>.learned.toml` remains bounded across all regions.

```bash
mecha learn                      # apply immediately
mecha learn --min 5              # need this many unprocessed reflections (default 3)
mecha learn --holdout 0.25       # leave every k-th out, for validate to probe
mecha learn --auto               # measure, then apply or refuse
mecha learn --propose            # measure, then stage for owner review
mecha learn --dry-run
```

Rules ride in the system prompt under a `## Learned rules` heading, user rules
first, then matching learned ones, inside the cached prefix. The selected block
is stable during a run and can benefit from prompt caching. `--no-learned-rules` opts out
anywhere, and `mecha eval` forces it off so a scorecard measures the model
rather than your accumulated rules.

The always-loaded block has two ceilings. `RULES_CHAR_BUDGET` (2600 characters)
is the size half; `MAX_ACTIVE_RULES_PER_DOMAIN` (25 active learned rules per
domain) is the count half, and it is a check that does not depend on the model
listening to the frame instruction that says the same thing — the frame is
handed the same constant, so the two cannot drift apart.

It is also *per domain*, and a run carries only the domains it asks for
(`RUN_DOMAINS`: `behavior` and `writing`). A domain is opt-in, so a new one
joins no prompt until something names it; a domain holding active rules that
nothing carries is reported at startup, because rules that cannot fire look
exactly like rules being obeyed.

**"Routed" has two meanings, and the warning needs both.** `RUN_DOMAINS` is what
an *agent run* carries in its prompt. `PASS_DOMAINS` is what a named, tool-less
pass loads — today just [`triage`](#the-triage-domain), which the mail classifier
loads and which is deliberately absent from `RUN_DOMAINS`. Measured against
`RUN_DOMAINS` alone, `triage` would have tripped the unrouted warning on every
single `mecha` invocation from its first learned rule, with a sentence that is
false: those rules do fire, from the classifier's own pass. The cost that matters
is not the noise — a permanent false positive is where a *real* unrouted domain
hides, so the check would have stopped doing the one job it exists for. The
warning is measured against the union, and the two lists stay disjoint with a
test saying so.

A candidate set
that ends over the cap may land only by *shrinking* an already-over set toward
it — growth past the cap is refused, which is what forces the next pass to merge
or retire before it may add. User rules are not counted: they are the user's own
budget to spend.

### Where a rule loads

Rules can be scoped to a tool set, an exact workspace, a surface such as
web, TUI, or Slack, and a goal such as `trigger:morning` or `task:<id>`. The
harness derives these keys from the recorded run;
the learner does not choose them. A matching run must satisfy every named key.
Rules without scope keys remain standing rules.

The goal key is the goal the run was handed from a store you own: the task
for `mecha tasks work`, the trigger for a scheduled run, and the reference you
gave `mecha run --goal`. A lesson learned in one trigger's runs therefore loads
in that trigger's next run and not in another's. Web chat, the front door and
the conversational front ends match with no goal, so a goal-scoped rule does
not load there. A rule learned with no goal loads under every goal.

A lesson supported in another region can widen its scope. Measured harm in one
region can narrow it instead of retiring it everywhere. `mecha rules` shows
scope and tallies; `LOADS NOWHERE` identifies a scope no recorded run presented,
and `--json` exposes `loads_nowhere`. An unrecognized surface or goal matches
nothing.

A rule scoped to a task keeps that scope after the task is done or dropped, and
a rule scoped to a trigger keeps it after the trigger is removed or disabled. It
widens only when the same lesson is learned toward another goal. Until then it
loads nowhere, and `mecha rules` says so: a count at the top, and
`LOADS NOWHERE` with the reason beside each such rule (`goal_closed` in
`--json`). `mecha learn` repeats the count on every pass. If the board cannot be
read, or no longer carries the task, both say that whether those rules are dark
is unknown. The terminal commands read the board; the TUI and the web settings
page do not wait on it, so there a task goal reads as unknown.

### Choose how changes go live

Bare `mecha learn` applies immediately, with the learning store's git history as
undo. The supplied automation uses **`mecha learn --auto`**:

- A candidate that regresses any graded probe is refused.
- A candidate with graded probes and no regression applies.
- A candidate with nothing gradeable applies **on probation**, explicitly
  recorded as unmeasured and eligible for earlier retirement.

Every automatic decision leaves an audit record. The measurement gate does not
relax provenance checks or allow changes to user-authored rules.

For a review queue, use `--propose` instead of `--auto`:

`mecha learn --propose` measures the candidate rule set by counterfactual replay
against the currently deployed rules, rejects any candidate that regresses a
probe before a human ever sees it, and stages what survives as a proposal.

```bash
mecha proposals                  # list
mecha proposals show <id>        # the rules diff beside the gate's evidence
mecha proposals accept <id>      # apply, with the lineage a direct learn leaves
mecha proposals reject <id> --reason "too narrow"
```

Accepting checks that the live rules still match what the candidate was measured
against; a diff on screen that is not the change being applied needs `--force`
to say so. Rejecting retires the reflections, so a human's "no" is not re-argued
nightly. Proposals can only ever touch `rules/*.learned.toml` — the security
layer is not proposable-against, structurally.

`--auto` and `--propose` are mutually exclusive. Both measure; only
`--propose` waits for the owner to accept a surviving candidate.

## Provenance gating: why this is stricter than the interlock

Every reflection carries an `Origin`:

| Origin | Meaning |
|---|---|
| `clean` | No third-party content had entered the conversation when the intervention happened — or it had, and the reflector was shown only the user's own words (below) |
| `untrusted` | Evidence that could include third-party content: a reflection recorded before provenance existed, or a mismatch from a tainted conversation |
| `derived` | Not the user correcting mecha: mecha's own words landing in the user role (the empty-turn and final-answer nudges, the boredom notice) |

`classify_origin` is deterministic code over the transcript's **recorded** taint
(`Session::taint_timeline`) — never inferred from the text, because prose
claiming to be from the user does not make it user content. `Reflexion::learnable()`
returns true for `Clean` — and for an `Untrusted` triage reflection, under
[the triage exemption](#the-provenance-exemption-and-what-it-rests-on) — and
false for anything you dropped. `mecha learn` filters on it *before any prompt
is built*, printing what it dropped:

```
2 reflection(s) excluded by origin — evidence from untrusted or
non-interactive sessions stays in the archive, never in rules
```

**Why this is stricter than the [trifecta interlock](/docs/features/security).**
The interlock guards exfiltration *inside* one conversation: taint accumulates,
and a send is refused once private and untrusted are both present. A learned
rule is the opposite shape. It outlives the conversation that produced it and
rides in every future run's system prompt, inside the cached prefix, where
nothing will ever check it again. That is a far longer half-life injection path
than anything the interlock covers, and it is the path the memory-security
literature identifies as the one that matters — defenses have to target the
*storage decision*, not input anomalies.

It is **fail-closed throughout**. A reflection whose position cannot be
established, one from a torn transcript, and one recorded before the field
existed all classify `Untrusted` (`origin_unknown()` returns `Origin::Untrusted`,
not the enum's first variant). `derived` exists because mecha correcting
itself is not the user correcting mecha — learning from it is a feedback loop,
not a lesson. It is a label rather than an exclusion: the reflection is kept
and visible, it just cannot be graded, because replaying without a
self-authored intervention shows only the model recovering.

There is deliberately **no knob** that loosens this. A switch that lets
third-party text into every future prompt is the silently-degrading-sandbox
shape. Excluded reflections stay in `reflections.jsonl` as readable evidence;
they are simply never candidates.

### Tainted sessions still teach: the user-turns path

Working sessions — anything that touched mail, docs or the web — are exactly
where corrections happen, and they are all untrusted, so the gate once
excluded nearly every real lesson. The fix moves the evidence rather than
loosening the gate. When the taint covering an intervention is not provably
clean, the reflector is shown only **the user's own typed words plus the tool
names** from the registry's closed set; every assistant-authored excerpt is
withheld and never read. Third-party bytes never reach the model that writes
the reflection, so the reflection is `clean` by construction and learnable.
Its record says so: `evidence: "user_turns"` rather than `"full"`. (Mismatches
are the exception — a generated observation is never the owner's words, so a
tainted one stays `untrusted`.)

```bash
mecha reflect --remine-untrusted       # recover lessons from older tainted sessions
mecha reflect --backfill-situations    # give pre-situation reflections one, no model call
```

A reflection also records which goals it bears on: the goals of the plan steps
whose checks failed, or otherwise the goal the plan or question named at the
moment of the intervention, never one named later. Where no plan or question
named one, it takes the goal the run was anchored to — the task a delegated
run works, the trigger that started a scheduled one, or a goal you confirmed —
as that run recorded it, never an anchor set later in the conversation.

`--remine-untrusted` re-mines the sessions whose reflections the gate
excluded, through the user-turns path; it is idempotent and never re-mines a
clean-covered intervention. `--backfill-situations` recomputes a situation
for reflections mined before that field existed, from their transcripts.

## The triage domain

`triage` is the mail classifier's own rule set, fed by
[`mecha mail reflect`](/docs/features/tools/mail#corrections-become-rules) turning your
corrections into lessons. It is the first **pass-scoped** domain: its rules ride
in the classifier's frame and in no agent run's prompt.

Its frame differs from the other two in the way the domain does. It asks for
rules about *kinds of mail* rather than about conduct — a general instruction is
noise to a classifier exactly as a classifier's rules would be noise to a
general run — and it forbids carrying a sentence from a message into a rule
verbatim, because a rule that quotes an email is that email speaking to every
future classification.

### The provenance exemption, and what it rests on

This is the subtlest thing on the page. [Provenance gating](#provenance-gating-why-this-is-stricter-than-the-interlock)
demands `Origin::Clean` and says in its own comment that there is deliberately
no knob, because a switch letting untrusted content into every future prompt is
the silently-degrading-sandbox shape.

**A triage lesson necessarily saw mail.** Under that gate the domain is not
unsafe — it is impossible, because a correction with no context cannot
generalise. (That defect has a name in the prior art: flowmail's correction
system documents it directly, and this repository nearly repeated it.)

The resolution is to notice what the gate's premise actually is. It guards rules
that ride in *every future run's* prefix, in front of an agent with tools, a
network and a way to send. Triage rules ride only in a tool-less, history-less
pass emitting a fixed schema, which cannot exfiltrate, send, or reach the
network. So the exemption is keyed on **the consumer**, not on a setting — and it
**goes false the moment that stops being true**: adding `triage` to
`RUN_DOMAINS` disables it with nobody needing to remember, and a test says so.

Three things bound the residual risk: generalisation across many corrections
means one hostile message cannot mint a rule, the frame forbids quoting a
message verbatim, and the outcome is
[measured daily](/docs/features/tools/mail#measuring-it-score-and-eval).

**One residual is accepted rather than enforced.** The check guards how
triage rules are routed today; a future feature that gave a tool-having run
direct access to triage rules would have to argue the exemption again rather
than inherit it.

## `mecha validate` — acceptance is not tenure

A rule that clears the proposal gate rides in every future prompt's cached
prefix, so it keeps earning that seat or loses it. That requires two things a
rule did not originally have: an identity, and a record of what it measured.

Rules carry `id`, `sources` and `created_at`, minted by `finalize_rules` and
carried across consolidations by text match — a rule whose text survives a
rewrite is the same rule restated, and keeps its id. Every field defaults, so
rule files written before identity existed load unchanged.

```toml
[[rules]]
id = "r-20260805-a3f10c2b"
text = "Ask before rewriting a file you have not read this run."
enabled = true
confidence = 0.8
based_on_count = 3
sources = ["refl-...", "refl-..."]
created_at = "2026-08-05T09:14:22Z"
```

`mecha validate` drives each reflection's intervention as a probe, in two arms —
rules-free and rules-on — and appends the outcome to `validations.jsonl`:

```bash
mecha validate
mecha validate --unprocessed-only              # the holdout learn left
mecha validate --trigger steer,denial          # default is all four
mecha validate --judge-provider gemma26 --judge-model ...
mecha validate --no-attribute                  # skip bisection
mecha validate --repeat                        # deliberately remeasure unchanged inputs
```

**Steer and denial probes are counterfactual replays, graded structurally.** The
recorded prefix is driven again — recorded tool results, no steering text — and
the verdict is a fact about the trace:

- A steer **passes** iff the replay tracks the recording *through* the steer
  point: the model does the steered thing without being steered. Divergence
  before that point means the run went off the rails before the question was
  posed — `Inconclusive`, not evidence.
- A denial **passes** iff the replay reaches the decision point and never makes
  the denied call (same tool, *same arguments*) again. Same tool with different
  arguments is not a failure — "not that directory" denies an argument, not a
  capability.

Follow-up probes keep the corrective turn and replay its recorded tools and
results without executing them. Only complete, readable answers with matching
tool arguments reach the judge. Missing calls, changed arguments and truncated
answers are inconclusive. These judgments do not drive rule bisection; treat a
single flip as a prompt to read the answers in `validation-attempts.jsonl`.

Mismatch probes need an owner-supplied artifact fixture — the task, its
starting files and the expected artifacts, fixed before any run and never
inferred from a model's check command. They grade artifact success from that
pinned state; they do not reconstruct a mid-step filesystem.

Validation defers previously measured inputs when the rules, recording, rubric
and measurement settings are unchanged. Provider/judge failures retry, and
regressions remain eligible for the existing retirement confirmation process.
`--repeat` requests another measurement explicitly. Deferred inputs are filtered
before `--cover` chooses extra probes. The report separates both-pass from
both-fail outcomes; neither is evidence of an improvement.

Each row records the exact rule set measured, the rules riding along, the
outcome (`improved` / `regressed` / `unchanged_pass` / `unchanged_fail` /
`inconclusive`), and the model — tallies are only comparable within one model
and one rule set, so the ledger never mixes generations.

### Bisection: naming the rule that flips it

When a trace-graded probe regresses — rules-free passed, the full block failed —
validate bisects the active learned rules against the **same recorded prefix**,
halving the set until one rule flips the verdict. Three properties make the
answer trustworthy rather than a guess:

- **User rules ride in every arm.** They are not on trial, and an arm without
  them would measure a deployment that cannot exist.
- **A regression the user's own rules cause alone attributes to nothing.** The
  first test is the rules-free-of-learned arm; if that already fails, no learned
  rule can be charged, and a final single-rule test would blame whichever rule
  happened to ride beside them.
- **An inconclusive or failed arm aborts the attribution.** So does a regression
  that needs rules from both halves together. `None` is an honest answer,
  because retirement argues from this number.

Judge-graded followups are never bisected: a followup regression is a prompt to
read two answers, not evidence that convicts one rule.

## `mecha sessions compare` — policies at the moments you decided

Replaying a whole session cannot grade a policy that changes what the agent
does: after the first different step there is nothing left to compare it to.
So this compares policies at single moments where you already gave a verdict:

| moment | found in | your verdict | an arm passes when |
|---|---|---|---|
| a steer | your words beside tool results | where you steered the run | it goes there without being steered |
| a denial | a call you refused | the refusal | it never makes the refused call again |
| a draft you rewrote and sent | the outbox | the text you sent | it drafts exactly your text |
| a draft you rejected | the outbox | nothing sent | it ends without drafting |
| a failed check | the harness's step feedback | your pinned artifact, when you bound one | the repeated task meets it |
| a surprise | a forecast the run's own count missed | none | — |

Each point is replayed from its recorded history under up to three policies —
the prompt the run carried, the rules deployed today, and no rules — for at
most four turns, and one comparison per point is written to the comparison
store. Drafts are compared exactly, after collapsing whitespace: a draft that
rewords yours is not a pass, and one that rewords the draft you rejected is not
a pass either, so no rewording can win. A moment with no structural test — a
check the agent declared for itself, or a surprise — is stored as
inconclusive, with nothing replayed; no model is ever asked to judge.

```bash
mecha sessions compare                 # up to 8 points, today's seed
mecha sessions compare --points 20 --seed 20250 --json
```

Points are shuffled with a printed seed, then ordered by the
[replay priority](/docs/features/appraisal#6-picking-which-past-run-to-replay-tonight)
of the session they come from. Points from sessions you corrected, that
surprised the appraisal, or whose situation keeps recurring are compared
first, and the seed decides among equals, so a pass can be redrawn. The
points that measure a proposed harness change are never ordered this way.
They stay a uniform draw, because they are what confirms the change. A point
already compared under the same rules and model is not compared again.
Only clean sessions are drawn, as for learning; each point holds one of the
background model seats while its arms run, and the pass refuses a provider
that is not on this machine.

When a point is decided, the policy that lost is not thrown away: the next
`mecha distill` adds it to the session's appraisal as a [counterfactual
reflection](/docs/features/appraisal/reference#what-a-losing-arm-taught),
pointing at the comparison.

## `mecha learn --compare-sources` — lessons by source

Two things write lessons about a session: the reflector, about each moment you
corrected the run, and the session's [text appraisal](/docs/features/appraisal).
Before the reflector's job is folded into the appraisal, the appraisal's
lessons have to do at least as well on the same moments. This pass measures
that, and learns nothing from it.

At each steer or denial the reflector reflected on, the recorded history is
replayed three times — as `mecha validate` replays it — with no rules, with
only the reflector's lesson, and with only the appraisal's lessons, each in
the same "learned rules" frame. An arm passes when it does what you asked for
without being told: it goes where you steered, or never makes the call you
refused.

```bash
mecha learn --compare-sources                      # up to 8 moments, today's seed
mecha learn --compare-sources --interventions 20 --seed 20250 --json
```

The report is per region — the tools, surface, workspace and goal the moment
happened in:

| per source | what it is |
|---|---|
| rate | passes over the moments where every arm reached a verdict; `—` over none |
| pass · fail | beneath the rate |
| improved · regressed | against the no-rules arm at the same moment |
| inconclusive | moments where this arm could not be graded |

and, for the region: moments compared, not yet measured, unavailable this
pass, and excluded, by reason. A moment is compared only when **both** sides
are clean — the reflection passes the same provenance gate `mecha learn`
applies and is still the reflector's own words (not one you dropped or
rewrote), and the appraisal comes from a session with no third-party content.
A session clean for one side and not the other is excluded and counted.
Followups are excluded too: only a model could grade them, and a model never
decides here.

Each verdict is stored as a comparison, so `mecha sessions appraise` prints the
same report from the stores without replaying anything. A moment whose lessons
are already measured as they stand is not replayed again. Each moment holds one
background model seat while its arms run, and the pass refuses a provider that
is not on this machine.

## `mecha rules` — tallies, retirement, restore

```bash
mecha rules                                   # every rule with its ledger tallies
mecha rules retire <id> --reason "..."        # by id or unique prefix
mecha rules restore <id>
mecha rules propose-retirements --min-attributed 3
mecha rules propose-retirements --apply       # apply the measured verdict now
```

`retire` and `restore` also append your verdict, against the rule's id, to
`curation.jsonl` in the learning store — the one record that says *you* retired
it (the retirement scan writes the same field) and that survives a restore.
It is a verdict on the rule, never on any run; `mecha sessions appraise`
counts it apart from the runs.

`list` folds `validations.jsonl` into per-rule tallies and prints each rule's
state, id, creation date, and what has been measured:

```
## behavior
  2 user rule(s) — immutable, never tallied
  [active] Ask before rewriting a file you have not read this run.
      id r-20260805-a3f10c2b · created 2026-08-05T09:14:22Z · 11 probe(s):
      3 improved, 1 regressed, 0 attributed to this rule; last 2026-08-05T03:31:07Z
```

`propose-retirements` scans the validation ledger without a model call.
The ordinary threshold is `--min-attributed` (default 3); probationary rules
use 2. A verdict can narrow a rule to supported regions or retire it. By
default it stages the change for review; `--apply`, used by the nightly script,
applies it directly and resolves superseded proposals. Probation ends only
when graded evidence clears its recorded convictions, not merely because a
probe ran.

**Retirement is a flag, never a deletion.** `Rule::active()` is
`enabled && retired_at.is_none()`, so the stronger claim wins even if `enabled`
was left true by a hand edit. The retired rule stays in the file, and the learner
is shown it in a section headed:

```
## Retired rules (IMMUTABLE, measured harmful — never restate or re-derive these)
```

which a deleted line could not say. `finalize_rules` carries retired rules
through every consolidation untouched, so a rewrite can neither resurrect nor
erase what retirement recorded. `mecha rules restore` clears both fields.

### Re-derivation, and the brake that stops it

The prompt section above is the *soft* half — it depends on the model listening.
The hard half is that `finalize_rules` carries `retired_at` forward onto any
rewritten rule matching a retired one, so a re-derived retirement comes back
**already retired and never renders**. Enforcement that does not depend on the
model, the same principle as the count cap.

Matching ignores case, punctuation, spacing and `-ise`/`-ize` spelling, so the
small wording drift a learner produces between runs is still caught. It goes no
further — no stemming, no synonyms — because a false match would silently retire
a *good* rule, which is worse than missing a restatement.

**A genuine paraphrase is still not caught, and that is accepted.** Closing it
would need either a judge or model-attributed sources, and a model deciding
whether a rule may live is exactly the model-rated policy this project refuses
everywhere else. (A `sources` set-intersection looks like the answer and is not:
a consolidation assigns the same batch sources to every new rule, so the
intersection would match everything from an overlapping batch.) The residual is
bounded instead — a re-derived rule that is actually harmful regresses the same
probes that retired it the first time, and one measurement cycle of harm is the
price of not having a model adjudicate tenure.

## What is deliberately absent

Four things the memory literature says not to build, listed so the backlog does
not reacquire them:

- **No decay and no TTLs.** Age is a review signal, not an argument.
- **No usage-based eviction.** Its canonical failure is the rarely-retrieved
  entry that must never expire — the literature's example is a penicillin
  allergy; here it might be "never force-push to main". A rule that fires once a
  year is not a rule that is wrong.
- **No policy built on model-rated confidence.** The `confidence` field exists
  because the learner emits one; nothing is allowed to grow a policy on top of
  it. LLM-rated importance is the mechanism everyone copied from Generative
  Agents and nobody validated.
- **No LLM-adjudicated destructive delete.**

Measured harm drives automatic retirement or narrowing. The owner can also
retire or restore a rule explicitly; age and usage alone do not remove one.

## `mecha eval --ab-rules` — the coarse complement

`mecha eval` forces learned rules off, because a scorecard shaped by your local
rules grades the machine rather than the model. `--ab-rules` is the deliberate
opt-in that default reserves space for: the case set runs rules-free and then
rules-on, and the per-case flips are reported **as their own artifact** — never
as a comparable scorecard. It is the literature's "task outcomes are free
per-memory quality labels", bought with machinery that already existed. See
[Evaluation](/docs/features/experiments/evaluation).

## Running the cycle nightly

`scripts/learn-live.sh`, when installed as a `session_end` hook, mines a small
batch and runs `learn --holdout 0.25 --auto` after sessions close. It moves to
its own work directory before reading config, so the closing project's config
does not govern an unattended learning pass.

`scripts/ruminate.sh` provides the nightly measurement and catch-up sweep:

```text
reflect → distill → validate --unprocessed-only --cover 1
        → learn --holdout 0.25 --auto → rules propose-retirements --apply
        → work clean → harness ruminate
```

`validate` runs before `learn` consumes fresh reflections. Both learning paths
hold out a deterministic slice so later validation has evidence the learner
did not consume. `--cover 1` also requests coverage for rule/region pairs not
yet graded. The script's judge defaults to the local provider; this is not an
independent judge when it is also the model under test.

The script defers the night if the model health check fails. Its scripts and
systemd units are supplied in `scripts/`; installing the CLI alone does not
install a hook or timer. Use `mecha learning-report` to inspect correction
trends, rule health, and consolidation history without a model call.

The evidence behind all of this is `docs/MEMORY-RESEARCH.md` in the repository.
