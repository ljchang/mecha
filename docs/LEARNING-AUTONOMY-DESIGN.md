# Learning without a human gate — design

*2026-08-19. Luke's decision: **learning in mecha is ungated in every domain**.
A rule goes live when it is derived, not when someone approves it. This
document is what has to be true for that to be safe, what changes per domain,
and what is deliberately not being built.*

Companion documents: `MEMORY-RESEARCH.md` is the evidence behind the current
learning system, `MAIL-UX-DESIGN.md` §5 is the triage correction loop this
enables, and `dev_docs/CORRECTION_SYSTEM.md` in the **flowmail** repository is
the prior art for the triage half — a Reflexion/LEAP-shaped correction memory
that this borrows from directly and improves on in one place.

---

## 0. The decision, and the argument that has to replace the gate

Today a learned rule reaches a prompt only after `mecha learn --propose`
stages it and a person accepts. That gate goes.

**A gate is a substitute for evidence.** It exists because a rule's effect was
unmeasurable, so a human had to guess whether it would help. Where the effect
*is* measurable, the gate is strictly worse than the measurement: approving a
rule is a prediction, and the ledger is an observation. Removing it there is
not a relaxation, it is an upgrade.

Where the effect is *not* measurable, removing it is a real loss and this
document says so plainly rather than pretending otherwise. See §3.

So the whole safety burden moves from a **pre-filter on rules** to the
**ledger**: measure, attribute, retire, and never re-derive. Everything below
is about making that load-bearing.

### What does not change

- **Rules are still evidence, never deletions.** `retired_at` /
  `retired_reason` stay; a retired rule remains in the file and the learner is
  told it was tried and measured harmful.
- **Provenance still classifies.** `Origin` still records whether third-party
  content was in context. What changes is §4: for `triage` it stops being an
  exclusion and becomes a label.
- **Domains are still opt-in and per-domain budgeted.** `RUN_DOMAINS` stays
  `["behavior", "writing"]`; `triage` rides in the classifier's own frame and
  nowhere else, so nothing here can leak one domain's rules into another's
  prompt. `MAX_ACTIVE_RULES_PER_DOMAIN` and `RULES_CHAR_BUDGET` are unchanged.
- **User rules are not on trial.** They ride in every arm and a regression they
  cause alone attributes to nothing.

---

## 1. Cadence follows evidence, not the clock

The domains differ in **how fast ground truth arrives**, and that — not how
much any of them is trusted — is what sets the measurement interval.

| Domain | The evidence | Arrives | Cadence |
|---|---|---|---|
| `triage` | did the user reply, archive, correct | ~10 threads/day | **Nightly** |
| `writing` | `diff(args_before, args)` on sent-with-edits outbox items | a few a week | **Every ~20 corrections** |
| `behavior` | replay probes — no natural outcome | never on its own | **Nightly, changed rules only** |

- **Triage measures itself for free.** The day-one cliff (`MAIL-CORPUS-RESEARCH.md`
  §3) means an outcome is known within a day, so a nightly pass sits on a real
  sample. It belongs in `ruminate.sh` beside reflect/distill/learn/validate.
- **Writing has ground truth too** — an edited draft is the user correcting the
  voice — but it arrives on review, not on a schedule. Measuring nightly would
  mostly re-measure an unchanged corpus and spend inference reproducing
  yesterday's number. Volume-triggered instead, which is flowmail's shape.
- **Behavior has no arriving evidence at all.** Its probes must be *run*, and
  each costs a request, so `--unprocessed-only` bounds the nightly cost by how
  many rules actually changed.

## 2. Retirement thresholds differ, and get *stricter* where evidence is weaker

Three attributed regressions is currently uniform. Uniformity was defensible
when a human stood in front of every rule. It is not now.

| Domain | Threshold | Why |
|---|---|---|
| `triage` | **2** | High-volume ground truth; a regression is meaningful sooner. A rule that buries a thread the user answered is a countable, serious error rather than a style opinion |
| `writing` | 3 | Real outcome, low volume — the count is what supplies confidence |
| `behavior` | **3, and revisit upward** | Probe-graded, and judges disagree with themselves across runs. It is now the only brake on the weakest evidence in the system |

The asymmetry is the point: **the less observable a domain is, the harder it
should be for a rule to keep its seat**, because nothing else is watching.

## 3. The honest cost of ungating `behavior`

Stated once, in full, because a design document that only lists benefits is
advocacy.

`behavior` rules ride in every run's cached prefix of an agent that **has
tools, network access and the ability to send**. Their validation is a replay
probe against a recorded transcript — indirect evidence about a trajectory,
not an outcome — and the judge that grades some of them is non-deterministic.

So after this change, the rules with the widest blast radius go live on the
weakest evidence in the system with nothing in front of them. The mitigations
are the stricter threshold above, the never-re-derive marker, and visibility
(§5). That is a real reduction in safety margin, accepted deliberately, and it
is the item to revisit first if anything goes wrong.

## 4. The `triage` domain, and why it may read the mail

The correction loop needs context or it learns nothing. A pair like
`{bucket: ignore → respond}` cannot generalise: it says an answer was wrong and
nothing about which *kind* of mail to treat differently. flowmail's
`CORRECTION_SYSTEM.md` names exactly this as the defect it was built to fix —
"the LLM can't learn *why* a correction was made (which sender? what kind of
email?)" — and the same mistake was nearly repeated here.

So a `triage` reflection sees the sender, the subject, the snippet, the
classifier's prediction and the user's correction.

**Why that is acceptable here and would not be for `behavior`:**

- The consumer is a **tool-less, history-less pass emitting a fixed schema**.
  A poisoned triage rule can bias a label. It cannot exfiltrate, cannot send,
  cannot reach the network. A poisoned `behavior` rule reaches an agent that
  can do all three. Same words, different blast radius — and the security
  argument lives entirely in what the consumer can do.
- **Generalisation is itself the filter.** Rules are compacted from a batch and
  must be supported by several corrections; a one-off produces nothing. A
  single hostile email cannot mint a rule, because it would have to produce a
  *pattern* of corrections the user actually made. This targets the threat
  rather than the ingredient, which refusing context does not.
- **The outcome is measured.** A rule that starts burying answered mail
  regresses false-`ignore` and is retired. No other domain can say that.

Two constraints that stay:

1. **The rule text is the reflector's generalisation, never quoted prose.**
   "Conference registration receipts are never urgent" is a rule; a rule
   carrying verbatim email text is a carrier for whatever that text says. A
   constraint on the frame, not on the input.
2. **The reflection records that it saw third-party content** — visible
   provenance, not exclusion. Evidence, not silence.

## 5. The oscillation hazard

Auto-accept plus auto-retire can loop: a rule is derived, retired on a noisy
measurement, and re-derived the next night from the same corrections that
produced it the first time. Under a human gate somebody would have noticed. Now
nothing would, except the inference bill.

`retired_at` plus the learner being shown retired rules as *measured harmful,
never re-derive* is the mechanism that prevents it, and it was written when a
human was also in the loop. **It needs a test that a retired rule is never
re-proposed from the same evidence**, and that test is a prerequisite for
shipping this rather than a follow-up.

## 6. Deliberately not being built

- **No autonomy tiers or per-rule graduation.** The decision is ungated, not
  gradually-gated.
- **No model-rated confidence anywhere.** flowmail's rules carry a `confidence`
  float; mecha does not build policy on a model's opinion of itself, and now
  does not have to, because the ledger supplies the real thing.
- **No auto-acceptance of anything outside the learning store.** This changes
  which rules reach a prompt. It does not touch the outbox, the trifecta
  interlock, or the approver — sending is still reviewed, and nothing here
  gives a run a capability it did not have.
- **No cross-domain rules.** A rule belongs to one domain and rides in that
  domain's prompt only.
