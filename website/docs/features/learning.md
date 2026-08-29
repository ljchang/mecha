---
title: Learning
sidebar_position: 15
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
| `validations.jsonl` | Every probe outcome, keyed to the rule set measured |
| `runs.jsonl` | One audit record per consolidation pass |
| `mined.jsonl`, `mined_outbox.jsonl`, `distilled.jsonl` | Idempotence ledgers |
| `proposals/<id>.json` | Rule changes waiting for a human |

## `mecha reflect` — interventions become reflections

Extraction from transcripts is pure code. Three kinds of intervention are
recognised:

- **Steer** — user text riding in the same message as tool results.
  Unambiguous: the user reached in mid-run to redirect.
- **Denial** — a tool result reading `Denied by the user: …`. A recorded
  rejected intent. (A hook denial reads `Blocked by a hook:` instead, and is
  deliberately not mined — machine policy is not a user correction.)
- **Follow-up turn** — a later user turn *may* be a correction or just the next
  task. Extraction only flags the candidate; the reflector decides, and is told
  to skip freely.

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

`mecha reflect` also mines the [outbox](/docs/features/outbox): an item that was
sent with edits yields a `writing`-domain reflection from `diff(staged, sent)`.

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
[web settings page](/docs/features/web#settings-and-what-a-browser-may-write) —
the same two panes, the same verbs as child processes, so no surface can do
something to the store that the command line cannot.

## `mecha learn` — reflections become rules

Consolidation rewrites `rules/<domain>.learned.toml` whole: absorb the new
reflections, merge overlapping rules, resolve contradictions, drop rules too
narrow to fire again. Rewriting rather than appending is what keeps learning
from growing the system prompt without bound.

```bash
mecha learn                      # apply immediately
mecha learn --min 5              # need this many unprocessed reflections (default 3)
mecha learn --holdout 0.25       # leave every k-th out, for validate to probe
mecha learn --propose            # stage as a proposal instead of applying
mecha learn --dry-run
```

Rules ride in the system prompt under a `## Learned rules` heading, user rules
first, then enabled learned ones, inside the cached prefix — so they change only
at consolidation time and cost nothing per turn. `--no-learned-rules` opts out
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

### Unattended learning never applies its own output

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

Direct `mecha learn` at a terminal still applies immediately, with git history
as undo. The gate exists for the runs nobody watches.

## Provenance gating: why this is stricter than the interlock

Every reflection carries an `Origin`:

| Origin | Meaning |
|---|---|
| `clean` | No third-party content had entered the conversation when the intervention happened |
| `untrusted` | Third-party content was in context |
| `derived` | Not an interactive session: a subagent, eval case or batch item |

`classify_origin` is deterministic code over the transcript's **recorded** taint
(`Session::taint_timeline`) — never inferred from the text, because prose
claiming to be from the user does not make it user content. `Reflexion::learnable()`
returns true only for `Clean`, and `mecha learn` filters on it *before any prompt
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
not the enum's first variant). `derived` exists because a subagent's steer is
mecha correcting itself, not the user correcting mecha — learning from it is a
feedback loop, not a lesson.

There is deliberately **no knob** that loosens this. A switch that lets
third-party text into every future prompt is the silently-degrading-sandbox
shape. Excluded reflections stay in `reflections.jsonl` as readable evidence;
they are simply never candidates.

## The triage domain

`triage` is the mail classifier's own rule set, fed by
[`mecha mail reflect`](/docs/features/mail#corrections-become-rules) turning your
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
[measured daily](/docs/features/mail#measuring-it-score-and-eval).

**The residual is stated rather than hidden.** The check keys on `RUN_DOMAINS`
membership, which is a *proxy* for the consumer: it catches someone routing
triage into ordinary runs, and it does not catch a future tool-having caller
that reads triage rules directly. Expressing the real property needs "this
domain has exactly one load site", which Rust cannot say cheaply and a registry
would cost more than it protects. So it is written where the next person meets
it — a sentence to argue with rather than an assumption to discover.

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
mecha validate --trigger steer,denial          # default is all three
mecha validate --judge-provider gemma26 --judge-model ...
mecha validate --no-attribute                  # skip bisection
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

Follow-up probes re-ask the corrective turn and are judge-graded, which is
non-deterministic; treat a single flip as a prompt to read the two answers.

Each row is keyed by `rules_hash` — a stable FNV-1a hash of the rendered block,
written out longhand because the std hasher is deliberately unstable across Rust
releases and a ledger key that drifts with the toolchain would silently split
every tally. The row also records `rule_ids` (weak observations for everything
riding along), the outcome (`improved` / `regressed` / `unchanged_pass` /
`unchanged_fail` / `inconclusive`), and the model, since tallies are only
comparable within one.

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

## `mecha rules` — tallies, retirement, restore

```bash
mecha rules                                   # every rule with its ledger tallies
mecha rules retire <id> --reason "..."        # by id or unique prefix
mecha rules restore <id>
mecha rules propose-retirements --min-attributed 3
```

`list` folds `validations.jsonl` into per-rule tallies and prints each rule's
state, id, creation date, and what has been measured:

```
## behavior
  2 user rule(s) — immutable, never tallied
  [active] Ask before rewriting a file you have not read this run.
      id r-20260805-a3f10c2b · created 2026-08-05T09:14:22Z · 11 probe(s):
      3 improved, 1 regressed, 0 attributed to this rule; last 2026-08-05T03:31:07Z
```

`propose-retirements` is a **deterministic ledger scan with no model anywhere**.
Once a rule accumulates `--min-attributed` (default 3) attributed regressions, it
stages `enabled = false` plus `retired_at` / `retired_reason` through the same
proposal gate as any other rule change, with the tallies as the evidence text. A
pending proposal already retiring those exact rules is not re-staged, so a
nightly run cannot spam the queue while a human has not looked yet.

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

Matching is by a `normalized_rule_key` that folds case, punctuation, spacing and
`-ise`/`-ize`, so the variants a learner actually produces between runs are
caught rather than only byte-identical text. It is scoped tightly on purpose:
checked **only against retired rules**, **only for retirement**, with identity
carry-forward still on exact text — so two genuinely distinct rules cannot be
merged by a normalisation accident, which has its own test. No stemming, no
stopword removal, no synonym table. The asymmetry sets how aggressive this may
be: a false match silently retires a *good* rule.

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

Only *measured harm* argues for retirement, and a human accepts the argument.

## `mecha eval --ab-rules` — the coarse complement

`mecha eval` forces learned rules off, because a scorecard shaped by your local
rules grades the machine rather than the model. `--ab-rules` is the deliberate
opt-in that default reserves space for: the case set runs rules-free and then
rules-on, and the per-case flips are reported **as their own artifact** — never
as a comparable scorecard. It is the literature's "task outcomes are free
per-memory quality labels", bought with machinery that already existed. See
[Evaluation](/docs/features/evaluation).

## Running the cycle nightly

`scripts/ruminate.sh` chains the whole thing behind a systemd user timer:

```
reflect → distill → validate --unprocessed-only → learn --holdout 0.25 --propose
        → rules propose-retirements → proposals
```

The ordering is the one deliberate choice. **`validate` runs before `learn`**,
because `learn` marks reflections processed and measuring afterwards would grade
the rules on their own training data. Tonight's fresh reflections are unseen by
the current rules by construction; `--holdout` keeps a slice unseen by the next
generation too, and the holdout is deterministic (every k-th by id) because a
measurement set that changes between runs measures nothing.

Every stage is idempotent and defers on failure. If the model server is not
answering, the script exits 0 and the whole night is skipped — a skipped night
is not a failed night, and tomorrow catches up.

The cycle can also drive itself from a [hook](/docs/features/hooks), detached so
the hook timeout never kills a model call in flight:

```toml
[[hook]]
event = "session_end"
command = "nohup mecha reflect -p local >/dev/null 2>&1 &"
```

The evidence behind all of this is `docs/MEMORY-RESEARCH.md` in the repository.
