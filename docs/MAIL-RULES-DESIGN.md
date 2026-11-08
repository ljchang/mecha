# Mail rules — design

*2026-11-02. Not built. Written for review before any of it is, so anything
here is still cheap to change. §0 holds the owner's rulings; §13 what is still
open. `MAIL-UX-DESIGN.md` is the surface this extends and is assumed read —
its §8 said "auto-send of anything" and "an autonomy tier" were deliberately
not being built, and R1–R3 below are the owner reversing that, inside the
bounds this document sets.*

*Implementation note, 2026-11-04: P0 prerequisites, P1a's store, and P1b's
header engine with its mail-binary entrypoint are in the tree. P1c-1 adds the
owner CLI, retrospective header testing, corrections and pre-timer fixes.
P1c-2 (2026-11-06) adds opt-in unattended execution, doctor and backlog.
P2–P7 remain; see §11. The original design below is retained,
with review corrections marked where they change it.*

**One word, two meanings, kept apart here:** *the digest* (§7) is the
morning summary of what rules did; a rule's *digest* in §3 and §6.2 is a hash
of its files, which an approval is bound to. `do = "digest"` is the action
that puts a message in the former.

**The one-paragraph version.** A mail rule is *when* (a matcher over incoming
mail), *what* (a list of actions) and a *mode* (`off`, `shadow`, `propose`,
`auto`). It is the event-driven twin of a trigger: a trigger is a cron slot
pointing at a prompt; a rule is a match pointing at actions. Most rules need
no model at all — "trash GitHub notifications" is a match on the sender — and
the ones that do reuse the quarantined classifier that already reads every
thread. Every action a rule takes, and every one it *would have* taken, is a
row in an append-only ledger, which is what the web page shows. A rule is
installed in `shadow`, never runs unattended until the owner says so per rule,
and loses that permission the moment its files change.

---

## 0. The owner's rulings (2026-11-02)

| | Ruling |
|---|---|
| **R1** | **Automatic replies are allowed — reversing `MAIL-UX-DESIGN.md` §8 for this case.** A rule may reply on its own only after the owner approves it, per rule, and **nothing ever starts in that state by default**. |
| **R2** | **A model-drafted reply goes to the outbox by default, and may graduate to automatic.** Graduation is the owner's act, per rule. |
| **R3** | **A rule may forward, or send a message carrying the details, to a specific person** — the receipt-to-finance case. |
| **R4** | **The format is the design's call.** The owner suggested skills and said they were fine with whatever made most sense. §2 decides: rules are their own store, and a skill is what a rule *names* when it needs a procedure for drafting. |
| **R5** | **A rule's reply is ordinary mail from the owner, not a marked auto-reply.** No `Auto-Submitted` header and nothing that makes Gmail or Outlook present it as automatic (§5.4). |
| **R6** | **Going automatic is the owner enabling it, not a track record.** No history threshold gates graduation; the owner turns it on per rule (§6.3). |
| **R7** | **Any edit to a rule's files lapses `auto`**, the owner's own included and for every action group, until the owner re-approves (§6.2). |
| **R8** | **An automatic forward carries no "sent by a rule" line.** It reads as the owner's own; the control moves to what may trigger one (§5.2). |
| **R9** | **The digest arrives in the morning, with the briefing** (§7). |
| **R10 (2026-11-03)** | **The engine lives in `mecha-core` behind a small provider-neutral interface; `mecha-mail rules apply` runs it.** Mail providers and OAuth tokens stay in the mail binary. `mecha` keeps `mecha-mail` as a dev-dependency only; P1c's `mecha mail rule …` verbs manage the store and launch `mecha-mail rules apply` as a child. |

One correction made while ruling R4, recorded so it is not re-assumed: **the
harness cannot write a skill.** `ARCHITECTURE.md` §Skills makes that absence
the whole safety argument — a skill body is trusted instructions, and nothing
derived from a session may become one. What the owner wanted from "the harness
can create them as needed" is delivered by §9 instead: a conversation *drafts*
a rule, and only the owner installs it, the way dashboards already work.

---

## 1. What already exists, and what is new

Almost every part is built. The new thing is the join.

| Need | Already exists | New |
|---|---|---|
| Recognise a kind of mail | `mail_triage.rs`: bucket, urgency, tags, `request_type`, `prefilter` (envelope-only, no model) | a `rule` field on the verdict (§4.2) |
| Archive, read, spam, trash | `mail_triage` (`TriageAction`), mailbox-only quadrant | restore verbs for undo (§11, P0) |
| Reply, forward, stage | `mecha mail reply` / `forward` / `compose`, the outbox | template and typed-field sends (§5) |
| Tasks | `mecha mail task`, deadline carried from the verdict | — |
| A schedule, a ledger, a run lock | triggers (`trigger.rs`) | the same store rules for rules |
| A daily read | the `morning` trigger, `mecha mail list --aged` | `mecha mail digest` (§7) |
| A web surface | `MailDesk.svelte`, `Mail.svelte`, `Outbox.svelte` | `#mail/rules` (§8) |
| Owner-acts-on-drafts history | outbox stamps (`owners_unchanged_release`, `owners_edit`, rejects) | shown per rule on the ledger (§8), informational only |

---

## 2. Why rules are not skills (R4)

A skill is the *instruct* shape: a procedure a model loads mid-run when it
judges one relevant (`ARCHITECTURE.md` §Skills). A mail rule is a different
object on four counts, and each would be a bug if forced into `SKILL.md`:

- **Most rules have no model in them.** Trash GitHub notifications, archive a
  mailing list, put receipts in the digest: a header match and a mailbox
  verb. As skills, each would need a model run per email to *decide* to apply
  a procedure whose outcome is fixed — slower on the local model, and
  non-deterministic where a string comparison is exact.
- **A rule is fired by the harness, not chosen by a model.** Skills are
  selected by the model reading level-1 descriptions; a rule that a model
  could decline to apply is not a rule.
- **A rule carries permission state** — mode, approval, a digest of what was
  approved (§6). Skills deliberately carry none: loading one arms nothing and
  permits nothing.
- **A rule's history is the product.** The ledger (§8) is what the owner
  monitors. Skills have no per-use record beyond the run's trace.

So: **`~/.mecha/mail-rules/` is its own store, and a rule may name a skill**
(`draft_with = "advising-prereqs"`) as the procedure for the one action that
needs prose. The skill stays exactly what it is today — user-authored,
progressive disclosure, `tools:` narrows — and gains a second caller. That
split is the triggers precedent again: triggers name their skills, and empty
means none.

---

## 3. The store

```
~/.mecha/mail-rules/
  rules/<name>/rule.toml       the matcher and the actions — owner-written
  rules/<name>/reply.md        a fixed reply, when the rule has one
  rules/<name>/<other>.md      a covering note, a typed-send template
  approvals.jsonl              every mode change: rule, action group, who, when,
                               from_mode, to_mode, digest
  ledger.jsonl                 one row per rule × message × action (§8)
  proposals/<id>/              drafts from a conversation, never live (§9)
  aliases.toml                 the owner's other addresses (§5.4), owner-written
  limits.toml                  the global daily cap (§5.4); a rule's own cap is
                               in its rule.toml
  deferred.jsonl               threads whose archive waits for classify (§4.1);
                               the one file here that is rewritten, not
                               appended — ids leave once their actions settle
  .lock
```

The trigger store's rules, unchanged: **global only, never layered config** (a
rule is an unattended actor over private data with a way to send, and a cloned
repository must not bring one); one TOML per rule so `$EDITOR` and `git diff`
work; temp-sibling-and-rename for every rewrite; an advisory flock for
read-modify-write; the ledger append-only.

**Every file a rule names resolves inside its own directory.** `template`,
`note` and §5.3's template are paths in `rule.toml`, which a proposal writes
whole (§9), and the store sits in `~/.mecha/`, out of `ToolCtx::resolve`'s
reach. So the loader canonicalises each one and proves containment in
`rules/<name>/`, the way `skill(name, file:)` proves a bundled file is inside
its own skill's directory (`ARCHITECTURE.md` §Skills); a path that escapes,
or a symlink that leads out, fails the rule's load rather than reading a file
elsewhere in the home.

**The mode is not in `rule.toml`.** It lives in `approvals.jsonl`, keyed to a
digest of the inputs §6.2 covers. A mode written into the file would let an
edit — or an installed proposal — grant itself `auto`.

**The store keeps provenance separately from `rule.toml`, keyed by `(rule,
field, digest)`.** For provenance, `rule` identifies the installed proposal
instance, not just its reusable name; a later re-proposal gets a fresh
identity. `field` identifies the exact key, including its owning action, or
the named file, and `digest` identifies the shown bytes. Identical text in
another rule, another field or a later proposal must not inherit an earlier
acknowledgement. An owner acknowledgement records `Owner` without rewriting
the rule only for text whose origin is `ModelClean`, excluding `to` (§9).
Every `ModelUntrusted` field and every proposed `to` requires an actual
owner edit; recognising an address does not rule out a lookalike domain,
even from a clean conversation. Approval binds to the full inputs in §6.2,
while measurement uses only §4.2's match digest, so adopting action text
does not discard matching history unless an edit changes the origin gate.
Acknowledgement changes neither that gate nor the match digest.

**The effective mode is the latest row for the rule and action group, and
the digest and §5.4's mode-level holds only ever lower it.** The holds —
`block_sends_after_private`, an unknown own address, a matcher with no
sender constraint, and a provider whose authentication headers cannot be
read reliably — are computed, not rows, so a group can read `auto` off its
latest row and still be held in `propose` by one of them. **Every row names
one action group** (§5's table), and resolution reads only that group's
rows, latest wins. A whole-rule change — R7's lapse, `pause-all` — is
written as **one row per group**, except that an already-off group is
excluded from lapse detection and notices (§6.2), and a non-owner leaves its
row untouched on a step-down (§6.1). Each row carries its own value, never a
single value for the rule: groups routinely sit at different rungs (*archive
automatically, propose the reply*), and one value for all of them would
either raise a lower group or drop an untouched one further than asked. A
later row raises a group again only by the owner's approval. Owner
approvals, permitted step-downs and §6.4's automatic brakes are all rows in
the same file, and the most recent one wins. An `off` row bypasses lapse
detection entirely (§6.2); for other modes, if its digest no longer matches
the inputs §6.2 covers, the mode reads as `propose` (or lower, if the row
says lower). Never "the current digest matches *some* `auto` approval": that
reading would let the next run undo a brake, which changes no file, and
would re-arm a rule whose edit was reverted to an earlier approved state,
when the lapse was the point. That is why a detected digest change is itself
written as a row (§6.2), not only inferred at read time. Permission moves up
only by a new owner row. **A group with no rows at all reads as `shadow`** —
a hand-created rule directory, a renamed one, or one whose rows were lost —
the same direction as an unknown mode (§8). That default does not bypass
§4.2's list-entry admission check: the loader counts a hand-created rule's
shadow entry, refuses admission beyond the cap, and exposes `held: list cap`
on omitted rules and as `mecha doctor` attention. A hold leaves the recorded
modes intact; it stops evaluation, not the record of what permission the
owner set.

### A rule, written out

```toml
# ~/.mecha/mail-rules/rules/github-notifications/rule.toml
description = "GitHub notification mail"
priority = 10                       # lower runs first (§4.3)

[match]
from_domain = ["github.com"]        # keys AND together; a list ORs within itself

[[action]]
do = "trash"
```

```toml
# rules/prereq-question/rule.toml
description = "Student asking whether a course can be taken before its prerequisite"
priority = 50

[match]
account = ["campus"]
describe = """A student asking whether they may take a course before
completing its listed prerequisite, or asking for a prerequisite waiver."""

[[action]]
do = "reply"
template = "reply.md"               # the owner's words, sent verbatim (§5.1)

[[action]]
do = "archive"
```

```toml
# rules/receipts-to-finance/rule.toml
description = "Purchase receipts I need reimbursed"

[match]
from_domain = ["store.example.com", "supplies.example.org"]  # needed for auto (§5.2)
describe = "A receipt or invoice for a purchase made for the lab."

[[action]]
do = "send"
to = ["finance@example.edu"]        # fixed by the owner, never by a model
template = "receipt.md"             # renders the typed fields below (§5.3)
fields = { vendor = "text", amount = "money", date = "date", order = "text" }

[[action]]
do = "digest"
```

An action's `to` is the only place an outbound address appears in a rule. It
must be `Owner` before that group enters `auto`: written or edited by the
owner (§9), because it bounds where every automatic forward and send lands.
A proposed `to` requires an actual edit even when `ModelClean`; installation
and acknowledgement never adopt an address. A proposed address may be
installed and used in `propose`. No action takes a recipient from the mail
except `reply`, whose recipient is the thread's sender under the guards of
§5.4.

---

## 4. Matching

### 4.1 Header matchers — no model

`from`, `from_domain`, `to` (which of the owner's addresses it arrived at),
`account`, `list_id`, `subject_contains`, `has_attachment`, and
`header.<Name>` (contains) for anything else. Case-insensitive, exact
semantics, documented per key.

They read **the envelope, never the body** — `prefilter`'s rule, for
`prefilter`'s reason: the body is where an injection lives, and a matcher that
parsed it would be a second place prose gets interpreted outside the
classifier's quarantine.

**They depart from `prefilter`'s other property, on purpose.** `prefilter`
may only ever say *ignore*, never *this needs a reply*, because the cases a
machine rule would have to get right to say that are the ones needing
judgement. A header rule may forward, send or reply — and §5.2 pushes those
actions *toward* header matchers. The difference is authorship: `prefilter`
is a heuristic the harness applies to everyone's mail, while a header rule is
the owner naming, by hand, the senders it is for. The judgement `prefilter`
refuses to automate is the one the owner made when writing the rule.

They run **before classification**, and a rule whose actions in non-off
groups are only `archive`, `trash`, `read`, `unread` or `tag` (the Mailbox
group plus `tag`; never `spam`, `digest` or `task`) can mark the thread
handled so the classifier never spends a call on it — **but only when that
rule's Mailbox group is in `auto`** (so a `spam`-only rule never skips, its
Report group never reaching `auto`; a `shadow` or `propose` rule has not
decided the thread's fate, so it must not decide whether the thread is
classified), **no description rule is enabled at all, and no matching header
rule has an outbound action, a `digest` or a `task` in a non-off group.** An
off action neither prevents the skip nor creates a deferral, because it owes
no work. The last two non-off actions need a verdict too: §7 renders a
flagged thread with the classifier's one-line and an automatic task with the
verdict's deadline, and a skipped thread has neither. Any enabled
description rule needs the classifier to see the thread, whatever its
actions — a `digest`- or `task`-only description rule is silenced by a skip
exactly as a reply rule is, since §4.3 applies record actions from every
match. A skipped classification is a description rule never evaluated, so a
`priority = 90` header rule that archives a list must not silence a
`priority = 5` description rule that would have replied — **and neither may
a `priority = 5` archive silence a `priority = 90` reply**, because §4.3
gives outbound actions to the first matched rule *with an active outbound
group*, so a higher-priority mailbox-only rule does not take the thread's
outbound slot. So the condition is not about rank. **A header rule's
inbox-removing actions (`archive`, `trash`, `spam`) wait for the classify
pass whenever either holds:**

- any description rule is enabled, for any action, not only outbound ones;
  or
- any matching header rule — this one included, since one rule holding both
  `forward` and `archive` is §5.2's expected shape — has an outbound action,
  a `digest` or a `task` **in a non-off group** (the last two because they
  need the verdict, §7).

**A rule is enabled if any of its groups is not `off`, `shadow` included.**
A shadow group's would-be actions have to see the thread too, or an archive
would silently truncate the would-have rows the owner is reading to decide
whether to approve it, and shadow would under-measure in exactly the
direction that looks safe.

Otherwise the inbox-removing actions run on the timer. Either way, `read`,
`unread` and `tag` run on the timer. **`digest` and `task` never run on the
timer**: they read the verdict, so they run at the end of the classify pass
with everything else that does, and a thread carrying one in a non-off group
is always classified. The header case matters as much as the description
one: a `priority = 10` rule that archives a list and a `priority = 50` rule
that forwards the same sender's mail both match at minute three, the forward
waits for `classify` (§4.4), and an immediate archive would remove the
thread before the forward is decided. They have to: `classify` enumerates
threads from `mail_recent`, which is inbox-only (`in:inbox` on Gmail,
`recent` on Outlook), so a thread archived at minute three would never be
classified and the reply would never be considered, with nothing on the
ledger to say why. **Deferring alone is not enough, though**: `classify`
reads `mail_recent`'s newest-N window, threads leave that window on a busy
day, and a deferred thread that left it would lose its archive and every
rule's evaluation in silence. So a deferral is **recorded** — and not only
an archive's: **any thread with an action in a non-off group owed to
`classify`** (a deferred inbox-removing action, an outbound action, a
`digest`, a `task`) gets an entry, because a header rule's forward that
waits for `classify` is lost just as silently if the thread ages out of the
window overnight. The header pass writes the thread's id to a deferred queue
in the rule store, and `classify` reads that queue beside `mail_recent` —
the same need `due_outside_window` answers for the triage store's retries
(work owed on threads the window no longer returns), here met with a queue
because there is no earlier record to walk. A queued thread is evaluated and
its deferred actions run whether or not it is still in the window. **The
queue exists so `classify` re-evaluates the thread, and an entry leaves once
it has**: each deferred action is then settled with the outcome its own
group uses — `done` for a mailbox, record or Board action (a task created is
a Board `done`), `sent` / `staged` / `held` for an outbound one (a
`propose`-mode send settles `staged`, never `offered`), so §4.4's per-thread
key sees an outbound settlement and a second message in the thread never
forwards again — or offered to the owner (`offered` — a `propose`-mode
mailbox, record or Board action now waiting in the mail desk or the digest,
which carries it from there and counts in `backlog.rs`), staged, skipped, or
failed. **P1b review correction (2026-11-04): an unreadable thread is
never dispatched.** It produces a notice, a non-closing `held: thread unreadable`
report row for each deferred action, and a non-zero apply exit. No ledger
settlement is written: no action key, once-per-thread occasion, cap capacity
or brake count is consumed. The deferred entry stays queued and retries until
expiry. An entry older than seven days settles as `expired`
with a notice, so the queue cannot grow without bound behind a broken
classifier — and `expired` is not a failure: it does not count toward §6.4's
consecutive-failure brake, which owner inattention must never be able to
trip. **P1b review correction (2026-11-03): expiry closes that occasion's
keys**, including the once-per-thread key for Board/outbound work. Historical
verdicts or repeated sightings must not renew its queue timestamp or authorize
it later. The notice offers manual review; any owner reset is a separate,
explicit operation, outside the automatic apply path. Whether any
description rule is enabled is known statically from the enabled set, and
which other header rules match a given thread is known in the same timer
pass, before anything executes. P1 builds this, though no description rule
exists until P4, so the bug cannot land dormant. A GitHub notification
should cost a string comparison, not a model call, whenever "the first rule
by priority owns the thread" allows it.

### 4.2 Description matchers — the classifier, one more field

**A rule is a description rule if and only if it declares
`match.describe`.** Its header keys, if any, are then an *additional
precondition* checked in the same classify pass, against the same newest
inbound message, **before** the classifier answers: only rules whose header
preconditions pass for this message are *eligible*, so a rule that could not
apply never takes the single `rule` slot from one that could. Eligibility
does not change the frame's prefix: the full active and shadow lists
admitted under §4.2's entry cap stay in the stable part, and the per-message
eligible names for each list arrive after the message as a short suffix,
with `rule` and `shadow_rules` constrained to the eligible names in their
respective lists. Never a separate header rule — and **none of its actions
run on the `--headers` timer**. This is the common shape, not an edge: §5.2
requires a sender key for any automatic forward, send or drafted reply, so
§3's `receipts-to-finance` example with `from_domain` and `describe` is
exactly it. Read as a header rule, its `digest` would fire for every message
from the vendor, whether or not it was a receipt. For §5.5, such a rule
counts as a description rule, so its `trash` and `spam` propose: a pinned
sender set narrows who can trigger it but not what the description
generalises to, and an owner who wants an automatic trash on a sender alone
writes a header rule for it.

**P1b review correction (2026-11-04): raw-header preconditions have an
account-level standing hold.** The engine asks the mailbox capability whether
its reader can prove header absence. **Owner ruling (2026-11-06): narrow this
hold; for a header rule whose actions are all non-outbound, a positive match
is real evidence even on a partial header set, while Unknown means absent for
this message, never a known negative.** A description rule (one with
`match.describe`) keeps the account hold whatever its actions — an
implementation choice in P3-1, not part of the ruling: its list admission and
§4.2's entry cap are decided before any message is read, so a per-message
exclusion would let it bypass the cap; P4 decides it with description rules.
On a provider that cannot prove absence (currently Graph), non-outbound
header rules using `list_id` / `header.<Name>` act normally on Match, recording
`headers_partial: true`; Unknown contributes no row, key, cap, deferral,
matched-set membership, outbound priority, description-list admission or
handled-marker suppression. Retrospective tests follow the same rule.
An outbound or description raw-header rule remains excluded for that account
even on a positive match: send guards need absence evidence that truncation cannot
supply. For that standing hold, emit one notice per rule/account/pass, with no
per-message rows, claims, keys or cap consumption. The held rule contributes
nothing to deferral, description-list admission, outbound priority or handled
marker suppression: other rules behave as if it were absent. Existing pending
and deferred snapshots for the held rule remain untouched, including expiry.
Gmail's full-header reader keeps its existing matching and per-message
completeness checks. This applies §5.4's standing-condition shape to matcher
preconditions; it does not promote missing headers to known negatives.

`match.describe` is a sentence the owner writes, distinct from the top-level
`description` label. The classifier — already one isolated, tool-less call
per thread — is given the admitted enabled rules' `match.describe` texts as
**two distinct numbered lists, active and shadow, which the model sees
separately**, each **in `priority` then name order** so the frame's bytes
are stable from thread to thread (the cached-prefix rule) — the *whole
admitted enabled set*, never a per-message subset, since a stable order over
a varying set is not a stable prefix; which rules are eligible for this
message comes after it (§4.2's header-precondition paragraph).

**List membership follows the individual groups' effective modes (§3),
subject to the entry cap below.** A rule appears once in the active list if
any group is `propose` or `auto`, and once in the shadow list if any group
is `shadow`; it appears in both when both conditions hold. Only the grounded
`rule` result can authorise evaluation leading to active actions. The two
results are reconciled for measurement below, so an omission from one list's
result cannot erase a match reported by the other. `off` groups use neither
result. The classifier returns:

```
rule: <one of the active rules' names> | none
rule_quote: words from the message that show it
shadow_rules: [ { name: <a shadow rule's name>, quote: <words that show it> } ]
```

**Only the grounded `rule` result authorises active actions; reconcile it
with `shadow_rules` for measurement only.** Otherwise a shadow-only match
could trigger active actions the active result never selected. A rule named
in either grounded result counts as matching evidence for all its non-off
groups, with one match per rule and thread. Its shadow groups write
would-have rows even if only `rule` named it; use the grounded shadow quote
when present, otherwise `rule_quote`. If both name it, each shadow action
still gets only one would-have row. Active actions require the grounded
`rule` selection, then their own confirmation and guards. Thus
Mailbox=`auto` and Fixed words=`shadow` records the latter's would-have
match when either result names the rule, but only `rule` can cause the
mailbox action. Without this union, the classifier could recognise the
thread in the active result and leave the shadow rate falsely low by
omitting the same name from its other answer.

**Shadow groups have their own list and field plus that reconciliation**, so
they never compete for `rule` — and **that field is a list**, so they do not
compete with each other either. Every rule installs in `shadow`, so several
at once is the normal state, and a single slot would leave the rule the
classifier did not name with no would-have row at all: its match rate would
read low, in the direction that looks safe, on the number the owner approves
against. Shadow groups act on nothing, so their list needs no overlap
semantics, only rows; an active sibling still acts only on the active
result. Each named shadow rule's quote is grounded like `rule_quote`. Were
they in one list, a rule freshly installed in `shadow` could be named
instead of a rule the owner approved for `auto`, and the approved rule would
do nothing and leave no row — against §6.1's promise that `shadow` acts on
nothing. A shadow rule's would-have row therefore says what it would do
*beside* the active rules, not in place of one; how two rules interact once
both are active is something the owner sees after promotion, in `propose`,
before either goes back to `auto`.

**Every description rule's shadow rate is labelled an upper bound, for every
group; a header rule's shadow rate gets that label only for outbound and
Board groups.** All description shadow groups — Mailbox, Private record and
Report included — avoid the main pass's single `rule` slot, so promotion can
lose matches to another rule. Description outbound and Board groups also
avoid the confirmation pass's single active choice. Shadow outbound groups
of either rule kind never compete for §4.3's outbound slot or record
`skipped: superseded`, even when header matching is exact. Board groups
still face §4.4's once-per-thread limit and §6.4's caps when active. A
**ceiling** and an **upper bound** both mean the rate can overstate
unattended activity; a ceiling additionally marks measurement with §9's gate
closed, so a gate-failing shadow rate is both.

**A description rule's shadow outbound or Board group whose text passes §9's
gate is measured by both passes.** A candidate in the reconciled match set
with a gate-passing shadow outbound or Board group gets the shadow
confirmation pass, whether it was named in `shadow_rules`, `rule` or both,
and a disagreement is recorded as `would have: propose` for those groups
only. Measuring description-rule shadow with the main pass alone would
over-state even that bound. For shadow the confirmation pass answers with
**a verdict per candidate**, a list over the reconciled shadow candidates,
not one choice among them: shadow groups have no outbound slot to share
(§4.3), and a single choice would make two matching shadow rules compete and
under-state the one not chosen. It costs one call per thread for the
gate-passing shadow candidates together (gate-failing ones are not
confirmed, below), which is a small fraction of mail and only while a rule
is being evaluated. The same discipline as `request_type` and
`deadline_quote`:

- **Recognition, never invention.** The value is checked against the live
  list; an unknown name is `none`. Constrained decoding gets the enum.
- **The quote is grounded** — the words must be in the message, or the match
  is dropped, recorded with why (the `ground_deadline` pattern). The quote
  is what the ledger shows a human as *matched because*; it is prose, the
  sender's, and never reaches a privileged run.
- **The descriptions are the owner's words** and enter the classifier's
  frame as instructions, exactly as the tag vocabulary does. A rule
  installed from a proposal (§9) carries its `Origin` into that frame's
  provenance.
- **Escalation.** The classifier reads a snippet first. A match in either
  list on a rule with a send (`reply`, `forward` or `send`) in a non-off
  group forces full-body escalation before the verdicts are final. Shadow
  sends need the same evidence they would use when active, but an `off`
  group never triggers escalation, so disabling a send does not keep paying
  for full-body reads or split other rules' rates. Raising it from `off` to
  `propose` or `auto` starts a new activation stretch (§4.4); a raise to
  `shadow` only starts measurement. In either case, `verdict_input` keeps
  any earlier snippet-only evidence apart from new full-body verdicts.
  Escalation is **per thread, across rules**: one rule's send can cause
  every description verdict for that thread to read the full body. Each
  ledger row **always writes `verdict_input`**: `snippet` or `full_body`
  from a description rule's final main-pass matching verdict, and
  `not_applicable` for a header rule or a row with no matching verdict.
  **Every P1 row writes `not_applicable` from the start**; a header match
  uses no classifier, even when its action later reads the triage verdict.
  This is a closed enum in an append-only store: omitting it in P1 would
  strand those rows in `unknown` forever instead of the header bucket. For
  description rules the value records what the verdict actually read, never
  inferred from the rule's actions. A missing or unreadable value still
  reads as `unknown`, kept apart rather than guessed from current rules. §8
  separates rates by this field because different evidence can produce
  different matches even under the same match digest.

#### A send is confirmed by a pass with no learned rules in it

The classifier's frame is not only the owner's words. It also carries the
learned `triage`-domain rules (the `pass_rules_block_for(&[TRIAGE_DOMAIN])`
load in `mail.rs`) and the few-shot pool — and those are made *from mail*.
`Reflexion::learnable` admits `Origin::Untrusted` for `triage` on one stated
premise: the classifier "can neither send nor reach the network", and its doc
comment says that a consumer of triage rules with a way to send voids the
exemption. A `rule` field that drives `reply`, `forward` or `send` is that
consumer, and §10 closes the loop: a *wrong match* correction becomes a triage
reflection, so mail the owner received could produce a learned line that
steers which rule sends. `rule_quote` grounding does not bound it — whoever
wrote the message chose the words it must quote.

So the main pass's result is **a candidate, never a decision, for an
outbound or Board group** — the Board group too, because a task reaches a
task run's prompt (§7) and an unattended run with tools is the consumer
`learnable()`'s comment names as well as a send is. The harness-written name
closes what a task *says*; the confirmation pass closes *which threads start
one*. Confirmation eligibility is per group: an outbound or Board group must
pass its §9 gate, with active candidates drawn from the grounded `rule`
selection and shadow candidates from the reconciled match set. A shadow
match alone never authorises active actions. A Mailbox, Private record or
Report sibling keeps the main pass's result, regardless of that
confirmation.

Such a candidate gets a second quarantined call, the **confirmation pass**:
no tools, no history, and a frame made only of the eligible rules'
`match.describe` texts and a fixed instruction compiled into the binary —
**no learned rules and no few-shot examples**. The active call is given
**every eligible rule with at least one confirmation-eligible active
group**, once per rule, not only the main pass's pick, and answers with its
own choice among them (or `none`) and its own grounded quote. An automatic
send or task proceeds only when both passes name the same rule; disagreement
goes to `propose` for those groups, with both answers on their ledger rows.
Judging only the winner would leave the choice *between* two automatic send
rules whose descriptions both honestly fit — which owner-fixed recipient an
attacker's message reaches — to the pass that carries the learned rules. A
rule whose text is still model-proposed from an armed conversation may sit
in `propose`, but its description never enters the frame that confirms
another rule's automatic send: the frame must be shaped by the owner's words
or a clean proposal, never unchanged prose shaped by a stranger.
Acknowledgement changes no bytes, so an untrusted `match.describe` still
requires an actual owner edit (§9); changing its origin label alone cannot
satisfy that premise.

Reconciled candidates with confirmation-eligible shadow groups are confirmed
in their own call, once per rule, with **a verdict per candidate** (above),
never one choice among them. Only descriptions eligible through those shadow
groups enter this frame. A mixed rule may appear in both confirmation calls,
but each call's result governs only its eligible groups on that side; an
active group's confirmation cannot stand in for a shadow group's
measurement, or the reverse.

**A description rule's shadow outbound or Board group whose text fails §9's
gate is not confirmed at all**: it is measured by the main pass alone, and
the page labels its would-have rate a **ceiling**, as well as an **upper
bound**. A confirmation rate for it would be a number nothing can act on,
because §9 forbids that group from entering `auto`. A sibling's confirmation
does not change its ceiling. An owner edit starts confirmed measurement only
when it changes the relevant group's gate status from failing to passing;
changing a file by itself says nothing about that transition.
Acknowledgement permitted by §9 cannot change the gate status.

**A group's would-have rates are computed only over rows written under its
current *match digest*, separately for each `verdict_input`**: the digest
covers the matcher keys (including `match.describe`), `priority` (with
§4.3's default resolved), the group's §9 origin-gate status where that gate
applies, and — for a **description** rule's groups that the confirmation
pass actually decides (an outbound action or a `task`, and only while the
text passes the gate) — the compiled confirmation version. `priority`
changes both the classifier lists' frame bytes and §4.3's outbound
precedence, so changing it starts new measurement. The top-level
`description` label is not a match input; its edit can still change the
origin gate, whose status is covered. The whole approval digest of §6.2 is
deliberately wider. **No per-rule full-body boolean remains in the match
digest**: escalation can come from another rule, so it is the row's actual
`verdict_input` that separates snippet and full-body evidence. A
confirmation-version change resets only those confirmed groups; a header
rule, a gate-failing rule's ceiling and a mixed rule's mailbox-only groups
each keep their week of measurement. The match digest is computed per group,
as the rates are. Rows from an earlier match digest are shown apart on the
rule page (§8), never mixed into the numbers the owner approves against,
under a heading that says why: *measured under an earlier build*, *before
your matcher or priority edit*, or *before the origin gate passed*. Within
each digest, `snippet`, `full_body`, `not_applicable` and `unknown` rows
stay in separate labelled groups with separate rates, never a pooled
headline rate. A gate transition therefore keeps the earlier ceiling visible
without counting it as confirmed evidence.

**Action text is not in the match digest**: an action's `to`, `note`,
`fields` declarations and rendering templates, and reply or forward
templates do not change a match. Adding, removing or changing the mode of a
send in a non-off group may change which threads escalate, for this rule and
others; the resulting rows stay apart by `verdict_input` rather than
resetting a per-rule full-body boolean. Text edits still lapse approval
(§6.2), but preserve matching history unless they change the applicable §9
gate status; an acknowledgement permitted by §9 changes only the provenance
record, never the gate status or match digest, so it preserves measurement.
The gate is still evaluated over every key and named file, so editing the
last untrusted text changes the status even when that text is outside the
matcher; acknowledgement never releases `ModelUntrusted` text or adopts `to`
at any origin. The §5.3 extraction version is **not** in the match digest
either: shadow runs no extraction, so the version cannot have touched a
would-have row. A typed-send rule's would-have rate therefore does not
subtract the sends that ungrounded fields would push to `propose` (§5.3 step
2), and the page labels it an upper bound on what `auto` will send
unattended. An `aliases.toml` or `limits.toml` edit lapses approvals (§6.2)
but resets no measurement: neither can change a match or the gate. Nor does
an edit to the `draft_with` skill's directory: the skill is owner-authored,
cannot change a match, and §9's gate is over the origin of the rule's own
text, not the skill's.

The match digest's components are always **written, empty when absent** —
P1's rows carry no compiled confirmation version, because it arrives in P4,
and an omitted component that later read as a default would silently merge
P1 rows into a P4 rate. Each description in the set is covered by its own
rule's approval digest (§6.2), so an edit to one rule lapses that rule; its
effect on another approved rule can only be to win a thread the other would
have taken, and the winner then sends only within its own approval — a
lapsed rule proposes.

That keeps the exemption's premise true rather than re-arguing it: untrusted
triage rules still reach only a pass whose output cannot send on its own.
When this is built, `learnable()`'s comment gains one sentence naming the
confirmation pass as the reason the premise still holds, so the next reader
inherits a true argument. The cost is one extra call per *thread that has a
send or task candidate* — the pass is batched, choosing among every eligible
candidate at once — plus one more when gate-passing shadow candidates are
being measured; not one per candidate, and not one per thread of mail, so a
small fraction of mail. The **Mailbox, Private record and Report groups**
(§5's table: `archive`, `read`, `unread`, `trash`, `tag`, `digest`, and
`spam` — not `task`) keep matching in the main pass alone, learned rules and
all, even in a rule with outbound or Board siblings: their worst case is a
reversible archive shown in the digest (§5.5, §7), since `spam`, the one
irreversible verb among them, never runs in `auto` and is always the owner's
confirmation.

**Cost.** The working assumption is a cap of **12 list entries across the
active and shadow lists together**, not 12 distinct description rules. A
rule in both lists occupies two entries in the stable frame and contributes
its tokens twice, so the worst case is two entries per rule. Shadow mode
(§6) measures precision with a real list before the cap moves.

**Every act that adds a list entry goes through the same admission check:**
installation, promotion, enable, an edit that adds membership, and a
hand-created or no-rows rule discovered by the loader (§3). Except for
safety lowerings below, an operation that would exceed the cap is refused
before its files or mode rows are written; the loader refuses excess entries
rather than admitting them just because their files already exist. The
refusal names the 12-entry cap, the resulting count, and rules/groups whose
demotion would free the needed entries; only a positive `Actor::Owner` may
choose an `off` target (§6.1). A safety lowering must still stop actions: if
a brake creates another list entry, the permission lowers and admission uses
the over-cap hold below, never retaining `auto` to make the count fit.

**If the store is over cap anyway, hold rules by cost to lose: shadow-only
first, then `propose`, then `auto`, each in `priority` then name order.**
Hand edits or a safety lowering can exceed the cap, but stopping measurement
costs less than stopping owner-approved actions. Classify a rule by its
highest effective group mode: any `auto` group puts it in the last class,
otherwise any `propose` group puts it in the middle, and shadow-only rules
come first. Keep a mixed rule's two entries together; holding it removes
both, so its shadow measurement cannot silently disappear while its active
half runs. Hold rules in that order until the remaining entries fit the cap,
then offer the retained entries in each list's usual `priority` then name
order. Every omitted rule shows **`held: list cap`** on its page and as
`mecha doctor` attention, with the cap, entry count and remedy. None of its
groups is evaluated or acts while held; the frame and constrained enums
contain only admitted entries. This is an evaluation hold, not an action
outcome or a mode row, so it closes no ledger key and changes no stored
permission. The page marks measurement unavailable while held rather than
reporting no matches. No shadow entry is ever dropped silently: the owner
must see when the measurement they are evaluating has stopped.

A per-rule yes/no pass would be more accurate and costs one call per rule
per thread; it is the fallback if the single field measures badly, not the
start.

### 4.3 Order and overlap

**`priority` defaults to `100`** when a rule omits it (lower runs first, so
an unprioritised rule yields to any that states one), and ties break by rule
name, so both the outbound slot and the classifier frame's order (§4.2) are
fixed for every rule. **`priority` alone decides precedence** — which rule
takes a thread's outbound slot. "Header rules before description rules" is
about *when* they are evaluated (header keys on the timer, descriptions at
classify), never about which wins: a `priority = 5` description rule
outranks a `priority = 50` header rule for the outbound slot. A thread may
match several header rules, but **at most one active description rule is
selected for actions** (`propose` or `auto`; shadow has its own list, §4.2):
§4.2's `rule` field is single-valued, so when two descriptions both fit, the
classifier's choice decides between them and `priority` does not. Priority
orders header rules among themselves and header rules against the one
description match. Two consequences follow and are accepted: the owner who
wants two description rules' active actions on one kind of mail writes one
rule with both, and a description rule not named by `rule` gets no
active-group match row — the ledger cannot show it was considered, so there
is no `superseded` row for those groups. Its shadow groups may still have
would-have rows from the reconciled match set (§4.2). If the per-rule yes/no
fallback of §4.2 is ever taken, multiple description matches become possible
and this paragraph is rewritten with it. Mailbox actions and `digest`/`task`
from every match follow their own groups' modes and results (§4.2). **Only
active outbound groups can take the single outbound slot**, and they come
from at most one rule per thread — the first matched rule by priority *with
an outbound group in `propose` or `auto`*. A higher-priority mailbox-only
rule, or one whose outbound groups are all `shadow` or `off`, never takes
the slot, because two rules answering the same email is never what anyone
meant. A rule appearing in both lists is considered once for that slot
through its active result; the reconciled match set writes its shadow
would-have rows once per action, so a shadow-only match neither takes the
slot nor double-counts. The ledger records other matched rules' active
outbound actions as `skipped: superseded by <rule>`; shadow outbound groups
never write that outcome, which is why even a header rule's shadow outbound
rate is an upper bound (§4.2). Within the winning rule every active outbound
action runs according to its group's mode: the receipts case is a `forward`
of the invoice *and* a typed `send`, when both groups are active. Shadow
siblings only record would-have rows, and off siblings do nothing.

### 4.4 When it runs

- Header rules' **mailbox actions and `tag`**: `mecha-mail rules apply
  --headers`, on its own five-minute timer — `read`, `unread` and `tag`
  always, and the inbox-removing ones only when §4.1 does not defer them.
  `digest` and `task` run after `classify`, since they read the verdict. No model, so no `model-idle.sh` gate.
- **Every outbound action, from any rule**, is decided in one place: the
  end of the `classify` pass, after every rule has been evaluated for the
  thread. That is the only way §4.3's "first by priority" can hold — a
  header rule's forward on the five-minute timer would otherwise have sent
  before a higher-priority description rule was even read. It matters on the
  common path, since §5.2 pushes outbound rules toward header matchers. The
  lag below therefore covers forwards and sends as well as replies.
- Description rules: at the end of each `mecha mail classify` pass (05:30 UTC
  nightly, every 20 minutes through the working day). An automatic reply can
  therefore lag its email by up to a classification interval, and overnight
  mail waits for the morning pass. That is acceptable for this kind of mail,
  and says so in the UI rather than promising "instant".

**Idempotent by key.** Every action is keyed `(rule, account,
provider message id, action identity)` in the ledger before it executes. The
action's identity is its `do` verb plus a digest of its own `[[action]]`
table, **never its position**: an offset re-maps when an action is inserted
or reordered, so an action that already ran would find no row under its new
index and run again — a receipt sent to finance twice. Keyed by identity, an
untouched action keeps its row across any reordering, and an edited one is
honestly a new action that has not run. Two structurally identical actions in
one rule would share an identity, so the loader refuses a rule that declares
the same action twice. The message half is the
provider's own id (`Email::provider_id`, what `get_message` takes), **made
immutable on Outlook by asking for it**: Graph's default `id` changes when a
message moves folders, and every Outlook `archive`, `trash` and `spam` is a
folder move (`move_message`), as is undo. A key read after the rule's own
archive would find no row and run the action again. So the ledger keys an
Outlook message by its **immutable** id — what Graph returns under
`Prefer: IdType="ImmutableId"`, fixed across moves within the mailbox — and
the rule engine sends the preference on **every Graph call it makes** —
reads, the mailbox action itself, the reader's fetch of a ledger row, and
undo — because an immutable id handed to a call made without the preference
is rejected as malformed. Whether the preference could go on every Graph
request instead depends on whether it reissues `conversationId`, which the
triage store keys on (`(account, thread_id)`); if it does not, the global
scope is harmless and simpler. P0 answers both questions on the owner's
mailbox: that an immutable id is **accepted** by a move and a restore (not
only that its value is unchanged after one), and whether `conversationId`
changes under the preference — and picks the scope from the answer. Gmail's message id is already stable
across label changes. It is
**never the RFC 5322 `Message-ID`**, which is sender-written and optional
here: a reused one would mark another message's actions as already done, and
every message without one would collide on a single key. The `Message-ID`
stays on the row as a display field only; a re-classification, a
`--force` sweep or a crashed run never sends twice.

**Which outcomes close the key** is part of the key's definition, because
several §8 outcomes are rows where nothing executed. `pending` (**while
unsettled** — once its settlement row exists, that row's outcome decides,
below), `done`, `sent`, `staged`, `offered`, `held` and `expired` close it.
Expiry closes an abandoned occasion even though nothing ran, so enumeration
cannot revive it. `would have` and `skipped` do not close it. `failed` closes it for outbound actions, which are never
retried (a failure after a provider accepted a send cannot be told from one
before), and for `task`, where a retry after a create that succeeded but went
unrecorded would put a second task on the board; it leaves the key open for
the Mailbox and Private record groups, which are idempotent and safe to retry
after a transient error. These failure closures concern dispatched actions;
a thread read failure never reaches dispatch and remains queued under §4.1's
P1b review correction (2026-11-04).

**A `pending` row does not stay pending forever.** It exists to survive a
crash, and a crash is the one way to strand one. A `pending` row older than
the run timeout settles by group: for the Mailbox and Private record groups
to `failed: interrupted`, which reopens the key, since those are safe to
retry; for outbound actions and `task` to `held: fate unknown`, which keeps the key closed,
since a send that may have reached the provider is never retried, and puts
it in front of the owner. `mecha doctor` reports any stranded or settled
`pending` row as attention, so a thread silently left unarchived is a
finding rather than an in-flight row nobody reads.

**A key is closed by the presence of a closing row, not by the latest row** —
deliberately unlike §3's mode resolution. The ledger is append-only, and four
outcomes land *after* a closing row: `released unchanged`, `edited`,
`rejected`, `undone`. None of them reopens the key, and each is written
with `closes: true` (§8), so no writer can encode "nothing was sent" on a
`rejected` row as permission to stage again. Under "latest row wins",
an owner's `rejected` would let the next classify pass re-stage the send just
refused, and an owner's `undone` would let the rule re-archive the thread the
owner restored to the inbox — the engine fighting its owner, with the
undo-rate brake measuring the fight. The per-thread key follows the same
rule.

**A rule acts only on messages that arrive after it became active.**
Promotion out of `shadow` does not replay the week of would-have rows, and a
`--force` sweep after promotion does not fire a backlog: a message is
eligible for an action group only if it was received after that group's
**activation** — the latest row that raised it from `shadow` or `off` to
`propose` or above. Rows that move a group *within* `propose` and `auto` —
an R7 lapse, a brake to `propose`, a re-approval — do not move the cutoff,
so lapsing and re-approving a rule never orphans mail that arrived in
between. A step down to `shadow` or `off` (a brake, `pause-all`, or the
owner turning it off) does end the stretch. A group in `shadow` writes
would-have rows for mail arriving while paused, and only resuming from
`shadow` offers *apply to these N matches from while I was paused*. A group
in `off` writes nothing for mail arriving while it is off — no would-have
row and no `skipped` row — so resuming from `off` offers no backlog. Only a
positive `Actor::Owner` reading permits writing `off` (§6.1): an owner
turning a rule `off` chooses no record; every other actor is clamped at
`shadow`, and `pause-all` targets `shadow`, so a pause keeps a record. **A
message found ineligible by a group in `propose` or `auto` gets a row** —
`skipped: before the rule was active` — so an active group passes nothing
over without a trace. (A `shadow` group has no activation and writes `would
have` rows instead; an `off` group writes nothing.) Acting on past matches
is an explicit owner action on the rule page — *apply to these N earlier
matches* — and runs in `propose` whatever the rule's mode, so the backlog is
reviewed, not sent.

**Matching is per thread; guards and keys are per message, and outbound
actions are once per thread.** The classifier reads a thread, but a thread
grows. So a rule evaluates a thread against **its newest inbound message**:
that is the message the header matchers read, the one §5.4's guards check
(its `From`, its `Authentication-Results`), and the one whose provider id
keys mailbox actions. Every ledger row therefore carries the **thread id**
as well as the message id — Gmail's `threadId`, Outlook's `conversationId`,
both stable across moves, with one caveat on Outlook: the rule engine reads
`conversationId` under the `ImmutableId` preference, and whether that
preference reissues it is the open P0 question above, which P0 settles
before this key is relied on — and the once-per-thread check below is a
lookup on `(rule, account, thread id)` over the outbound rows, a key of its
own beside the per-message one, and it names its closing outcomes as the
per-message key does: **`pending`, `sent`, `staged`, `held`, a Board `done`
(a task created), `offered` (a proposed `task` waiting in the digest) and an
outbound or Board `failed`, and `expired` close the thread** — a send that reached the
provider, one queued and still owed, or one whose fate cannot be told — and
**`would have` and `skipped` leave it open**. Expiry closes the abandoned
occasion so a stored verdict cannot resurrect it. Otherwise a rule
promoted out of `shadow` would find its own would-have rows and never send
on the threads it was approved against, and a rule that lost the outbound
slot to another on one message could never take it on the next. Outbound
actions are additionally limited to **one occasion per rule per thread** —
and the Board group's `task` with them, so an active thread produces one
task per rule rather than one per message: a rule's active outbound actions
and tasks fire together, under their own groups' modes and §4.3's outbound
slot, on the first message that triggers them; shadow groups only record
would-have rows and off groups do nothing — the receipts rule's `forward`
*and* typed `send` both go (§4.3) — and a later message in a thread the rule
has already sent from is `skipped: thread already handled`, never a second
forward under a new message id. P0 gives the thread read the newest inbound
message's provider id and headers for this. The ledger row is written
`pending` first and settled after, so a crash between the two is visible as
a pending row, never silently retried as a send.

---

## 5. Actions

Grouped by what can go wrong, because that decides what may run unattended.

| Group | `do =` | Reaches | May run in `auto`? |
|---|---|---|---|
| Mailbox | `archive`, `read`, `unread`, `trash` | the owner's mailbox | yes — with the §5.5 limit on `trash` |
| Report | `spam` | **the provider's filter** — the one mailbox verb with an effect outside the mailbox (`TriageAction::Spam`'s own words) | **never** — it never rises above `propose` (in `auto` it proposes, as the mail desk's `!` confirms), since no restore verb un-trains a filter |
| Private record | `tag`, `digest` | the owner (the triage store) | yes |
| Board | `task` | the owner's board — **and from there a task run's prompt** | yes, with a harness-written name (§7) |
| Fixed words | `reply` with `template` | the sender | after owner approval (R1) |
| Verbatim forward | `forward` with `to` | an owner-fixed recipient | after owner approval (R3) |
| Typed fields | `send` with `to`, `template`, `fields` | an owner-fixed recipient | after owner approval (R3) |
| Drafted prose | `reply` / `send` with `draft_with` | sender or owner-fixed recipient | outbox; may graduate (R2, §6.3) |

### 5.1 `reply` with a template — the vacation-responder shape

The owner's words, sent as written. Slots are a **closed set filled by the
harness**, never by a model: `{sender_first_name}` (from the `From` display
name, length-bounded, stripped of anything but letters, spaces, hyphens and
apostrophes), `{subject}` (the thread's subject as `Re:`, length-bounded, with
CR, LF, NUL and other control characters removed before it reaches
`gmail::checked_header`, which would refuse it anyway), `{today}` (the
harness's own date). Every slot states what it strips; a slot added later
states it too, or it is not a closed-set slot.

Why this is the case R1 can safely allow: **the payload carries no private
data.** The template is fixed; the slots come from the message being answered
and go back to the person who sent it. What an attacker learns from triggering
it is one bit — that a rule matched, and that the mailbox is live — which is
what every vacation responder has always disclosed. The recipient is
attacker-chosen (anyone can send mail), so the controls are about *being a
reflector*, not about leaking: §5.4.

### 5.2 `forward` — verbatim, to an owner-fixed recipient

The thread forwarded as-is, with the owner's covering note
(`note = "forward.md"`), to the addresses in `to`. No model writes a word.
Nothing private beyond the forwarded message leaves, and it goes only where
the owner said.

**An automatic forward reads as the owner's own (R8)** — no line saying it
was sent by a rule, on R5's reasoning for replies. The risk R3 opens is
integrity rather than leakage: someone mails the owner a fake invoice that
matches the rule, and it reaches finance carrying the owner's name. With no
notice on the message, the control moves to what is allowed to trigger one:
**an automatic forward or send needs the triggering message to pass DMARC
aligned with its `From`**, the same check §5.4 puts on replies, and a message
that fails stages in the outbox with the DMARC guard as its row's reason (a
per-message demotion, §5.4). That stops the forged-sender invoice — and
only that. **DMARC is authentication, not authorisation**: anyone who owns a
domain passes it for their own mail, so on its own it admits any stranger with
a plausible invoice, and the §4.2 confirmation pass would agree, because the
message honestly *is* an invoice.

So the second condition is structural: **a rule may run `forward` or `send`
in `auto` only if its matcher constrains the sender** — a `from` or
`from_domain` key. An `account` key does not count: which account mail
arrived at says nothing about who sent it. A rule whose matcher is
`describe` alone may still *propose* these sends. R3's "to a specific person"
becomes "from a known set, to a specific person".

**A domain is only a known set if few people can send from it.** A
`from_domain` naming a public mailbox provider (`gmail.com`, `outlook.com`,
`yahoo.com`, `icloud.com`, `proton.me` and the like — a closed list compiled
into the binary) or one of the owner's own accounts' domains (a university's,
where every student and colleague has an address) is a population, not a
known set, and does not count; such a rule needs a `from` key naming the
addresses. A vendor's or a company's own domain counts, and the owner who
lists it accepts that anyone who can send from that domain is in the set. The
residual the owner accepts by enabling such a rule is then the narrow one: a
listed sender's real account sending something false, with the ledger and
the digest showing every forward that went.

**A drafted reply (`draft_with`, §6.3) needs the same key to run in `auto`.**
Only a *template* `reply` is exempt, and on §5.1's argument alone: its payload
is the owner's fixed words, so an attacker-chosen recipient costs one bit. A
drafted reply is model prose over the thread and every `shareable` file, and
injected text in the triggering message steers which parts of those files it
quotes back. §6.3's closed context bounds *what could* leave; the sender
condition bounds *who can make it leave*, and the action with the widest
steerable payload should not be the one without it.

### 5.3 `send` with typed fields — extract, ground, render

The receipt-to-finance case done properly: a message *with all the
information* in a fixed shape, without letting a model write prose to a third
party.

1. A quarantined extraction pass — no tools, no history, and **the
   confirmation pass's frame shape, not the classifier's**: the owner's field
   declarations plus an instruction compiled into the binary, with no learned
   rules and no few-shot pool (§4.2). The extraction is a send decision —
   which `amount`, which `email` is the payload — so it gets the frame a send
   decision gets, and §6.2's digest already carries the compiled
   instruction's version. It fills the rule's declared `fields` from the
   message. Types are a
   closed set, and each says what it strips, as §5.1's slots do: `text` (one
   line, at most 200 characters, control characters and line breaks removed),
   `money` (amount and its currency code, both parsed, re-rendered together —
   a bare number is never sent to finance) and `number` (parsed, then
   re-rendered by the harness — never the
   extracted string), `date` (parsed to `YYYY-MM-DD` and re-rendered), and
   `email` (a single address that parses, else the field fails). A type added
   later states its rule too.
2. **Every value is grounded**: its source words must appear in the message
   (the `deadline_quote` rule). An ungrounded field fails the unattended
   send: it stages in the outbox, with the ungrounded field as the row's
   reason — a per-message demotion (§5.4), not a hold on the rule.
3. The harness renders the owner's template with the values. **The subject
   is part of the template** — `receipt.md` starts with a `Subject:` line the
   owner writes, which may use the same field slots, and reaches the provider
   through `gmail::checked_header` like any other header. A template with no
   subject line fails to load rather than sending one the harness made up.
   The model's
   output is data in typed slots, never sentences — the reviewable-object and
   typed-extraction rules from `CLAUDE.md`'s recurring shapes, applied to a
   send.

Leakage: none beyond the triggering message, to a recipient the owner fixed.
Integrity: as §5.2, with the same DMARC condition for `auto`.

### 5.4 Guards on automatic sends

Which guard binds which action group, so no section has to infer it:

| Guard | `reply` (template or drafted) | `forward`, `send` |
|---|---|---|
| RFC 3834 (`Auto-Submitted`, `Precedence`, lists, `noreply`) | yes | **no** — a vendor receipt is routinely `Auto-Submitted: auto-generated` from a `noreply@`, and §5.3 exists to forward exactly that |
| DMARC aligned with `From`, topmost instance | yes | yes (§5.2) |
| Sender-constrained matcher for `auto` — absent holds the affected groups in `propose`, with no per-message row | drafted only (§5.2) | yes (§5.2) |
| `From` only, never `Reply-To` or reply-all | yes | n/a — the recipient is the owner's `to` |
| Once per sender per rule per 7 days | yes | **no** — every new receipt thread from a listed vendor is forwarded; §4.4's once-per-thread limit still applies, so a vendor reply into an existing order thread is `skipped: thread already handled`, visibly |
| Per-rule and global daily caps (they also cap the Board group's automatic `task`s, §6.4) | yes | yes |
| Not into a conversation the owner has joined | yes | no |
| Never on a message `From` one of the owner's own addresses — an unknown own address holds outbound groups in `propose`, with no per-message row | yes | yes |
| Never on one `From` a `to` address; RFC 3834 on for senders at a `to` domain | n/a | yes |
| `block_sends_after_private` (§6.3) — a mode-level hold, not a per-message guard: it keeps every outbound group in `propose` and writes no `skipped` row | yes | yes |

Loop prevention for `forward` and `send` needs two guards of its own,
because the copy can come back two ways. Their
recipient is fixed by the owner, so nothing they send goes back to the
sender — but the forwarded copy can come back to the owner (the owner is on
the recipient alias, or the recipient copies them in), and R5 and R8 make it
ordinary mail **from the owner** with no marker to key on. A rule whose
`from_domain` names the owner's own institution would then fire on its own
output, each copy a new provider id the ledger has not seen, until the daily
cap stopped it. So, both failing closed:

- **Never act on a message whose `From` is one of the owner's own
  addresses** — the reply column's guard, applied here too. **The set has a
  stated source**, because this is the guard the loop argument rests on and it
  fails quietly when an address is missing: each account's own address as
  mecha-mail already resolves it for reply addressing (`account.address`,
  which `gmail_reply_fields` uses to keep the owner off a reply), plus an
  owner-written `aliases` list in `~/.mecha/mail-rules/` for send-as
  addresses and aliases the providers do not report. An account whose own
  address is unknown holds its rules' outbound actions in `propose`, said on
  the rule page — unknown is never an empty set. This is the guard
  that stops the loop: the copy that comes back is a different message with a
  different id, so no id-based check could recognise it, but its `From` is
  always the owner's. (An "own output" check by sent id was considered and
  dropped: Graph's send and reply return no id, and the returning copy would
  not carry it anyway.)
- **Never act on a message whose `From` is one of this rule's own `to`
  addresses.** That covers the other way back: the recipient copies the owner
  in, and the returning message is from finance, not from the owner. A rule
  whose `from_domain` happens to include the recipient's domain would
  otherwise fire on finance's acknowledgement, every day, held only by the
  daily cap. **And the RFC 3834 checks come back on for any sender at a `to`
  domain**: an acknowledgement usually arrives from a ticketing or
  `no-reply@` mailbox at the recipient's domain, not from the address in
  `to`, so the address-exact guard alone misses the loop it names. Turning
  `Auto-Submitted`, `Precedence`, list headers and `noreply` back on for
  those senders only keeps §5.3's reason for the exemption — vendor domains
  and recipient domains differ — while closing the ticketing loop.

The rest of this section is about replies. A reply goes to an address the
sender controls, so the guards keep mecha from
becoming a spam reflector or joining a mail loop. All are checked in the
harness, every per-message guard that fires is recorded on the ledger row,
and none is configurable off. They come in three shapes:

- **A refusal** — the send is not made at all, and the row is `skipped:
  <guard>` (RFC 3834, the loop guards, the per-sender limit, a conversation
  the owner has joined).
- **A demotion** — the send is not made *unattended*, but staged in the
  outbox for the owner, and the row is `staged` with the guard as its reason
  (a DMARC or authentication failure, a header read that cannot be shown
  whole for this message, or §5.3's ungrounded field).
- **A mode-level hold** — `block_sends_after_private` (§6.3), an unknown own
  address, a matcher with no sender constraint (§5.2), or a provider whose
  authentication headers cannot be read reliably at all (the Outlook
  fallback below), which keeps the affected outbound groups in `propose` and
  writes no per-message row; the rule page names which condition holds them.
  These are standing conditions of the rule, the account or the provider, so
  repeating them for every message adds no evidence. A single message whose
  headers cannot be shown whole is not one of these: it is the demotion
  above.

The guards:

- **RFC 3834.** No reply to a message whose `Auto-Submitted` has any value
  but `no`. And no reply, whatever `Auto-Submitted` says, to a message
  carrying `Precedence: bulk | list | junk`, a `List-Id` or a
  `List-Unsubscribe`, to a `prefilter` hit, to a
  `noreply`/`mailer-daemon`/`postmaster` sender, or to any of the owner's own
  addresses.
- **Authenticated sender.** `Authentication-Results` must show DMARC `pass`
  aligned with the `From` domain. A forged `From` is how an auto-responder
  sends unsolicited mail to a victim; a message that fails goes to `propose`.
  **Only one instance counts.** `Authentication-Results` is a trace header:
  every hop prepends its own, and a sender can write as many as they like
  into what they submit. The guard reads **the topmost instance, and only if
  its `authserv-id` is the account's own provider** (pinned per provider, not
  matched loosely); any other instance is ignored, and **absent or
  unparseable means `propose`, never `pass`**. **Outlook needs its own shape,
  decided before P4 rather than discovered there**: Microsoft 365 commonly
  writes its instance with no `authserv-id` at all, starting at `spf=`. Read
  literally, the rule above would hold every Outlook send in `propose`
  forever, which is not the guard working but the feature not existing on
  that account. So on Outlook the topmost instance is accepted without an
  `authserv-id` when it carries Exchange Online Protection's own `compauth=`
  verdict, which EOP writes and a submitted header has no reason to; and P0
  **verifies that against real headers from the owner's mailbox before
  anything relies on it**. If the topmost instance turns out not to be
  reliably EOP's, the design's answer is that Outlook sends stay in `propose`,
  stated on the rule page, until there is something better to key on. Graph's
  `internetMessageHeaders` is a truncated collection, so "absent" is a real
  case on Outlook, and each provider gets a test that a missing header
  proposes. **The trap is on both readers, and worse on Gmail.**
  `parse_gmail_message` (`gmail.rs`) collects headers in an assign-every-time
  loop, so its `List-Unsubscribe` is *last* wins — harmless for a bulk
  signal, and exactly wrong here: Gmail returns headers in message order, so
  adding `Authentication-Results` to that loop selects the bottom-most
  instance, the one a sender can write. On Gmail the reader takes the
  **first** instance; on Graph, `graph_mail.rs`'s `.find(…)` is first-match
  (`find_map`) and needs the provider check added. And because neither
  provider documents header order — the same caution `get_thread` writes
  down for Gmail's messages array ("observed, not documented") applies to
  the header array, which is documented no better — the guard does not rest
  on it:
  **if more than one instance claims the account's own provider, the message
  goes to `propose`.** A genuine provider instance is unique, and a forgery
  added to a submitted message makes two, so this fails closed whatever order
  the headers arrive in. What counts as a *claim* is defined per provider: on
  Gmail, an instance whose `authserv-id` is the pinned provider value; **on
  Outlook, any instance carrying a `compauth=` token**, whatever its
  `authserv-id` — since EOP's own instance is recognised by `compauth=`, a
  submitted one with it is a second claim and proposes. **Counting cannot
  cover truncation, though**: if Graph's truncated collection has dropped
  EOP's instance, a forged `compauth=` instance is the only claim, and one
  claim reads as genuine. So on Outlook the guard also requires that the
  header collection was read **whole** — P0 establishes how a truncated
  `internetMessageHeaders` can be told from a complete one (or reads the
  headers through the MIME source, which is not truncated), and a read that
  cannot be shown whole proposes. If neither is possible, Outlook sends stay
  in `propose`, as stated below — and that provider-wide fallback is a
  **mode-level hold** like the other standing conditions, holding the
  account's outbound groups with no per-message row, while an ordinary
  DMARC failure on one message stays a per-message demotion. P0's real-header check covers Gmail's ordering as
  well as EOP's shape, with the same fallback: if it is not reliable, that
  provider's sends stay in `propose`. `Authentication-Results` gets its
  own field on `Email` rather than riding the existing loop.
  One consequence worth saying in the UI: mail that reaches an account by
  forwarding from another address routinely fails alignment. That is an
  ordinary per-message failure, so it is a **demotion**, not a hold: the
  group stays at whatever mode the owner set, and each such send stages in
  the outbox with the DMARC guard as its row's reason. On an account that
  receives mostly forwarded mail, nearly every send will stage — the guard
  working, not a fault — and the rule page shows the share of staged rows by
  reason, so the owner can see why `auto` is sending little.
- **`From`, never `Reply-To`, and never reply-all.** `Reply-To` is
  sender-chosen and is the other reflection route. An automatic reply answers
  one authenticated person.
- **Once per sender per rule per 7 days** (RFC 3834 asks responders to limit
  repeats; seven days is RFC 5230's default for Sieve vacation responders),
  and a daily
  cap per rule (default 20) and across all rules (default 50). Hitting
  either cap stages the rest of the day's outbound actions in the outbox
  rather than sending them — nothing is lost, and the owner releases or
  rejects them (§6.4).
- **Not into a conversation the owner has joined.** If the owner has sent a
  message in the thread, a human is already handling it.
- **Sent as ordinary mail from the owner (R5).** No `Auto-Submitted` header
  and nothing that tells the recipient's client it was generated; only the
  threading headers an ordinary reply carries, through the existing
  `mail_reply` path on both providers. The cost is that other responders do
  not recognise the reply as automatic, so loop prevention rests on mecha's
  side alone: a responder that answers back is caught by the `Auto-Submitted`
  and `Precedence` checks above, and any responder that does not mark itself
  is stopped by the per-sender limit — one reply per sender per rule per
  seven days cannot become a loop.

### 5.5 Mailbox actions on a description match

A header matcher catches only mail carrying headers the owner chose, so its
false positives are mail the owner asked about by name. A description
generalises, so it also catches other people's mail that merely resembles the
description — and a sender can write a message to resemble it. For `archive`
that costs the owner nothing they cannot see — it is in the digest and the
ledger, and it is reversible. For `trash` and `spam` it is a way to bury mail.
So **`trash` runs in `auto` only from header matchers**; on a description
match it never rises above `propose`. (`spam` never runs in `auto` at all — §5's table.) And every automatic mailbox action lands in
the next digest (§7), so nothing leaves the inbox without a trace.

---

## 6. Modes, approval, graduation, brakes

### 6.1 The ladder

| Mode | Matches | Records | Acts |
|---|---|---|---|
| `off` | no | — | — |
| `shadow` | yes | *would have* rows | nothing |
| `propose` | yes | rows | mailbox and record actions wait in the mail desk; sends stage in the outbox |
| `auto` | yes | rows | executes, per §5's table |

**Every rule installs in `shadow`.** That is how "is it working?" gets
answered before it can do anything: a week of *would-have* rows against real
mail. Moving up a rung is the owner's act and only the owner's — the CLI
(`mecha mail rule mode <name> <group> <mode>`) or the web page — and never a
tool. **A raise always names its group**: a rule-wide raise would lift every
group to one value, which §3 rules out. With the group omitted, `mode` may
only lower, and then each group takes the lower of its current mode and the
target, as `pause-all` does.
This is `/review now|later|auto`'s rule: a policy that decides what leaves the
machine must not be decidable by anything sharing a context window with
third-party text.

**"Never a tool" is not the whole boundary**, because a model with `shell`
needs no tool to type `mecha mail rule mode <name> <group> auto`. So
**raising** a mode requires a positive `Actor::Owner` reading from the shell
registry and run markers `closure.rs` already reads to decide who may close
a task (`ShellReading`, `POSTURE_ENV` as advisory). A registered run,
`Actor::Unknown` or an unreadable actor is refused; the raise is recorded
with `Actor::Owner` only. This deliberately does **not** inherit the
outbox's ruling that a release from an unknown actor is stamped `unknown`
but not refused: one release sends one reviewed draft, while a raise is a
standing permission to send unattended, and a run with the owner present is
still a context window with mail in it.

**Writing `off` requires a positive `Actor::Owner` reading.** Every other
reading — a registered run, `Actor::Unknown`, or an unreadable actor —
clamps an `off` request to `shadow`, not a refusal, and the CLI says that it
clamped the requested mode. Anyone may stop actions without approval (§6.4),
but only an identified owner may stop the would-have record that explains
the pause (§4.4). The lowering uses the same actor reading as the raise gate
and records who made it, including `Unknown`, so the page does not attribute
an unidentified pause to the owner. A group already `off` stays there
without a new row on any non-owner step-down, automatic lapses and brakes
included; clamping the target must not raise it or write an owner-only mode
on another actor's behalf.

**P1c-1 review correction (2026-11-06): both actor gates are advisory without
the sandbox.** The shared `closure::attribute_explained` reading used by outbox,
tasks and this CLI returns `Owner` for `ShellReading::NotRegistered` plus
`PostureReading::NotInRun` with no recorded run ancestor. A detached descendant
of an unconfined shell with its environment cleared can therefore read as
`Owner`; absence of a shell registration is not an `Unknown` reading. This is
the same residual as an unconfined shell appending to `approvals.jsonl` directly.
The sandbox is the remedy: confined shell cannot write the store (§Security
model). **P1c-2 review correction (2026-11-06):** `mecha doctor` reports
**any group in `auto` while shell runs unconfined**, with attention severity
for each group and the sandbox as the remedy. This includes Mailbox and Board:
tasks reach a task run's prompt, and the self-grant residual applies to every
group. Outbound auto is held in P1, so an outbound-only check would never fire.
The attribution implementation is unchanged.
The orchestrating session keeps serve's loopback CLI-child route within this
existing residual (including a chat live in serve); doctor now also flags
every automatic group when registered shell has `Sandbox::can_reach_network`,
naming the loopback route and a sandbox with network off alongside any write exposure.

**P1c-2 adversarial review correction (2026-11-06):** a configured sandbox
with a writable mount covering the rules store is also exposed. Compare both
the protected store and mount through resolved aliases and ancestors; a backend
name alone is not confinement. The general confinement check protects the
rules store alongside the actor registry and other owner stores.

A rule may be `auto` for some actions and not others: `mode` can be set per
action group (§5's rows), and every mode row names one group (§3) — there is
no rule-wide mode for a group to default to. "Archive these automatically,
but propose the reply" is the common case. For description rules, any
`propose`/`auto` group puts the rule in the active list and any `shadow`
group puts it in the shadow list (§4.2), independently, subject to entry
admission. The reconciled match set feeds shadow measurement, while only the
grounded active `rule` result can lead to active actions; changing one
group's mode cannot activate another. Only active outbound groups can take
§4.3's outbound slot. §4.2 checks the cap on every entry addition, not just
promotion; if the loader finds an over-cap store, omitted rules are visibly
`held: list cap`, never silently missing from measurement.

**P1c-1 correction:** an active group whose activation timestamp is unknown
is held as a standing condition with one notice per group/pass; it neither
enqueues nor dispatches work, and its existing deferred snapshots remain held.

### 6.2 Approval binds to what was shown (R1)

Entering `auto` for any send group shows the owner, on one screen: the
matcher, the exact template or covering note, the recipient rule, the
guards, and the shadow-mode rows so far **under the current match digest,
with separate rates for each `verdict_input`** (§4.2). Earlier digests and
snippet versus full-body measurements stay apart, as on the rule page (§8),
because a rule's own files cannot say which evidence another rule made the
classifier read. The recipient `to` is shown first, with its origin and
editing, never acknowledgement. **Acknowledge this text** is offered for
other text only when `ModelClean` (§9), including `match.describe`, `note`
and templates. Every proposed `to` and every `ModelUntrusted` field requires
an actual edit: one click neither changes a stranger-shaped description nor
rules out a lookalike recipient domain, whatever the conversation's taint.
Acknowledgement is an `Actor::Owner` act under §6.1's raise gate, bound to
the shown `(rule, field, digest)` (§3), and records `Owner` in the store's
provenance record without rewriting `rule.toml`; a run cannot attribute its
own proposed text to the owner. Acknowledgement changes neither gate status
nor match digest. After an edit, the screen recomputes both before approval:
a gate transition shows the earlier ceiling apart, while a recipient, note
or template edit that leaves the gate unchanged keeps the measured rate.
Narrowing the match digest preserves the history of sibling groups whose
gate status stays unchanged when adopting `to` changes the outbound group's
gate. Even if acknowledgement of the address were permitted, that gate
transition would still reset the outbound group's measurement.

The approval written to `approvals.jsonl` carries **a digest of the rule's
directory** (and of the named skill's, for §6.3, and of `aliases.toml` and
`limits.toml` for any rule with an outbound action or a `task` (§6.4 caps
tasks too), since §5.4's loop guard and caps rest on them — removing an
alias or raising the global cap must lapse the approvals that counted on
them). That includes tightening edits, and one global-cap edit lapses the
non-off groups of every outbound rule at once; the screen that edits
`limits.toml` and `aliases.toml` says so before saving, with the count of
approvals it will lapse, excluding already-off groups. The caps live in the
rule store, never on `Config`, so they are not layered config and not the
two-edit `Config`/`ConfigLayer` trap. On every run, **already-off groups are
excluded from lapse detection and notification entirely**: no comparison
against their old digest, no replacement mode row, no lapse ledger row and
no notice for them. `off` is an owner-only state that records nothing, so a
lapse notice has nothing to protect; comparing its untouched row would
report the same lapse on every run. A later owner raise binds to the current
inputs. For the other groups the digest is recomputed; **if anything
changed, each drops to `propose` or stays lower** — one lapse row per
non-off action group. Each lapse row records the lower of *that group's*
current effective mode and `propose`, so a lapse never raises anything: a
group in `shadow` stays in `shadow`, a group under §6.4's failure brake
stays braked, and a group in `auto` drops to `propose`. They are recorded as
**lapse rows appended to `approvals.jsonl`**, as well as a ledger row
(`approval lapsed: files changed`) for the page, and a notice. The approvals
row is what makes it stick: with only a ledger row, reverting the edit would
make the latest approval's digest match again and re-arm `auto` with no
owner act, which §3 rules out. With the lapse as the latest row, a revert is
inert. Re-approving is one action that shows the diff.

That includes the owner's own edits and every non-off action group,
mailbox-only rules too (R7). It is the simpler rule, the `hud_propose`
precedent (an accept names the digest the owner was shown), and the cost is
one click after an edit.

**The approval digest covers everything that decides a send; the match
digest covers only §4.2's matcher keys (including `match.describe`),
`priority`, applicable origin-gate status and confirmation version.** The
top-level `description` label is not a match input. Editing a recipient,
note or template still lapses approval, but cannot reset matching history
unless it changes that status. Actual snippet or full-body evidence is
recorded per row as `verdict_input`, outside the digest, and its rates stay
separate (§4.2): another rule's send can cause escalation without changing
this rule's files.

**The approval digest says so where it does not cover a match.** For a
header rule (one with no `describe`, §4.2), `rules/<name>/` is the whole
matcher. For a description rule *with an outbound action or a `task`*, the
decision is the confirmation pass of §4.2, whose inputs are the rule's
`match.describe` and an instruction compiled into the binary — so the digest
also carries the version of that instruction **and of the §5.3 extraction
pass's**, the other compiled frame a send depends on, and a build that
changes either lapses every approval that rests on it. What the digest does
**not** cover is the main pass's learned triage rules, which change by
design every time the owner corrects a verdict (§10). For send rules that is
harmless, because a learned rule can only nominate a candidate: the
confirmation pass then chooses among every eligible send and task rule
without it, so a learned line can neither make a send happen nor pick which
of two rules sends. For a description rule's Mailbox and Private record
groups, even with outbound or Board siblings (which get the confirmation
pass, §4.2), it means the matcher can widen after approval: accepted,
because those actions are reversible, appear in the digest, and exclude
`trash` and `spam` (§5.5), and stated here so nobody reads "approval binds
to what was shown" as covering it.

The feature itself (`mail_rules` in the features registry) is **off by
default**, so on a new install nothing in this document runs at all.

### 6.3 Graduating a drafted reply (R2)

`draft_with = "<skill>"` names a skill **by name, never by path**: the value
must pass the skill store's own name check, it resolves only through
`SkillStore::get` over the global `~/.mecha/skills/` (the store a run loads
from, with its own containment), and a name that resolves to nothing fails
the rule's load rather than drafting without a procedure. §6.2's digest
covers the directory that lookup returns. It runs a drafting agent with the
skill loaded, and its
draft stages in the outbox — `propose`, always, until graduated. **Graduation
is the owner enabling it, and nothing else** (R6): no count of past drafts, no
acceptance rate and no waiting period gates the switch, and nothing ever flips
it on the owner's behalf. The approval screen of §6.2 is the whole ceremony.

The one precondition is structural, not historical. It binds **`auto`
only**: in `propose`, a drafting run may *read* with whatever tools its
skill declares, because the owner reads every draft in the outbox before it
goes and that review is the control for the draft. But the review comes
after the run, and unlike `mecha mail reply` — which drafts on a thread the
owner chose — a rule's run starts on arriving mail, unattended, steerable by
whatever the thread says. So **every drafting run, in any mode, runs
read-only**: `permission_mode = "read-only"`, as a trigger does by default,
with the skill's `tools:` as its allowlist intersected with tools that write
and destroy nothing **and reach nothing outside**: no egress of any class,
`Blind` included, so `web_search` and `web_open` are removed even though
read-only would keep them — a run started by a stranger's mail and steerable
by it must not be able to put that mail's contents into a query. Its one
effect is the staged draft: its draft is the send, and the outbox holds it. The leak
argument below is therefore the argument for `auto`, and it is true of a rule
from the moment it graduates, not before.

- **A closed drafting context, for `auto`.** The drafting run's tools are only: the
  triggering thread (given as input, not fetched) and the skill's bundled
  files, which the owner marks `shareable` in the skill's frontmatter. No
  `mail_search`, no graph, no workspace, no web. A rule whose skill declares
  anything wider cannot be set to `auto`; the CLI and page say which tool
  blocks it. With no file tool, the run cannot follow a skill's level-2 and
  level-3 pointers the way an ordinary run does, so the body and every
  `shareable` file are **given whole** in the frame — `shareable` therefore
  means "may appear in any reply this rule sends", not "may be opened if
  relevant", and the owner marks files with that in mind.

That is the security argument, and it is a different control from the
interlock — not a way around it. Even as a tool this send would be
`Egress::Blind` (the model names no recipient), which is the class the
interlock deliberately does not own (`EGRESS-DESIGN.md` §3–§4; §6 states
the residual risk).

The control that does own `Blind` is `block_sends_after_private`, which
refuses both classes but runs in the loop's dispatch, where a harness-side
send never passes. **So the rule engine reads it itself — from the global
config only** (`Config::load_global`, the trigger runner's rule), never
through the layered load: a `mecha.toml` in whatever directory the rule
engine runs from could otherwise lower a guard the owner raised globally,
which is the cloned-repository threat §3 keeps the store out of layered
config for. With
`block_sends_after_private = true`, **every** outbound action group stays in
`propose` — template replies included — and the rule page says that guard is
holding it. Its doc comment says "block *every* outbound call" and "turn this
on when private data must not leave at all", and an owner who set it on those
words should not find their mailbox answering strangers. A template reply
leaks one bit at most (§5.1), which is not worth narrowing a flag's stated
meaning for. The guard is off by default.

**`[outbox] tools` is the other loop-side control a rule send skips, and here
the rule's approval wins.** The route says a model's call to a listed send
tool is staged, not executed. On this deployment every mail send tool is
routed, so if the route held rule sends too, R1 and R2 would be impossible
in practice. This overrides a stated principle and says so: the queue is
"the one place outbound mail waits regardless of who wrote it" (`mecha mail
compose`'s own words), which is why an owner's hand-typed `compose` or
`forward` stages today. The argument is R1 and R2 alone: the owner reversed
"no auto-send of anything" for this feature, per rule, and a rule's `auto`
approval is the later and more specific statement — made per rule, against
shown text and recipients, under §6.1's raise gate. So the route does not
hold a rule send in `auto`, the outbox's own section in `ARCHITECTURE.md`
gains a sentence naming this exception when P4 lands, and the
approval screen says so in a line: *this rule sends without the outbox; your
outbox routing still applies to everything else.* `block_sends_after_private`
is different in kind — it is the owner saying nothing private leaves at all,
which no per-rule approval should silently outvote.

What bounds the leak is
the **context**: the only private data the draft can contain is the thread
itself (going back to someone already on it) and files the owner declared fit
to share. An injection can still make the draft *say* something wrong —
promise a letter, agree to a meeting — and that integrity risk is what the
owner accepts at graduation, with §6.4's brake behind it.

Graduated drafts go through §5.4's guards like any automatic reply.

### 6.4 Brakes

A rule steps itself down, or holds its sends, with a notice, when one of the
conditions below fires. The step-down brakes name the groups they lower, and
each lowers a group to the lower of its current mode and the brake's target,
like a lapse, leaving an already-off group's row untouched (§6.1). Only the
owner may write `off`; the two cap bullets change no mode and only stage.
§4.2's list-entry cap never blocks a safety lowering: excess entries are
held visibly rather than leaving a failed group in `auto`. A brake on one
rule can therefore hold another rule out of evaluation under §4.2's cost to
lose order. That rule shows `held: list cap` on its page and as `mecha
doctor` attention, never a change to its mode: the hold is capacity, not a
brake against that rule.

- **Any automatic send is marked wrong** by the owner → **that send's group**
  to `propose`. Zero tolerance for sends.
- **More than 20% of its automatic mailbox actions undone** over at least 10
  → **the Mailbox group** to `propose`.
- **More than 20% marked a wrong match**, undone or not, over at least 10 →
  **every group of the rule** to `propose`. A wrong match indicts the matcher,
  which every group shares; the archive may be harmless while the matcher is
  wrong, and the same matcher is choosing the rule's sends.
- **Three consecutive action failures** in one group → **that group** to
  `shadow`. *Action* failures only —
  the rule's own action refused or erroring. A failure of the environment
  (`held: thread unreadable`, a provider unreachable, a token needing
  re-auth) does not count, nor does `failed: interrupted` (§4.4), since a
  crash is the harness's failure and not the rule's, on `expired`'s reasoning: an outage would
  otherwise demote every matching rule to `shadow` and end its activation
  stretch (§4.4), so recovering from someone else's downtime would cost an
  owner raise per group. Environment failures are `mecha doctor`'s to
  report, where the remedy is.
- **A rule's daily cap is hit** (§5.4) → that rule's outbound actions stage
  in the outbox for the rest of the local day, with a notice and ledger rows
  saying `held: rule cap`; its mode is unchanged. Twenty matching messages in a day is the likeliest brake to
  fire, and quietly not answering them (which a step-down to `shadow` would
  do) is a worse failure than a queue the owner can release.
- **The cap across all rules is hit** → no rule changes mode, because the
  rule that happened to be fifty-first is not the cause. Every outbound action
  for the rest of the local day stages in the outbox instead, the ledger rows
  say `held: global cap`, and the owner gets one notice. Mailbox and
  Private record actions are unaffected.
- **The caps count the Board group too.** An automatic `task` reaches a task
  run's prompt (§7), so it sits on the send side of the risk line and is
  capped like a send: a rule's daily cap and the global cap count its tasks
  with its outbound actions, and past either cap further tasks are offered in
  the digest (`held`) rather than created.

`mecha mail rule pause-all` (and a button) drops every rule to `shadow` at
once — one row per group, each recording the lower of its current mode and
`shadow`, except that a non-owner leaves an already-off group's row
untouched (§6.1), so a group the owner turned `off` stays off. The target is
`shadow`, not `off`, so paused groups keep would-have rows for recovery
(§4.4). A run may pause without approval down to that floor (§6.1); stepping
up always needs approval.

---

## 7. The digest

`mecha mail digest [--since <last>]` — deterministic, reading the triage store
and the rule ledger, no model:

1. **Handled for you** — per rule, what it did since the last digest (counts,
   expandable to rows), so nothing an automatic rule removed is invisible.
2. **For your information** — `notify`-bucket threads, **plus every thread a
   rule's `digest` action flagged**, whatever its bucket: sender, the
   classifier's one-line, tags, deadline, and for a flagged thread the rule
   that flagged it. This is where "archive it, but show me tomorrow" shows
   the message itself rather than only a count under *Handled for you*.
3. **Possible tasks** — threads whose verdict proposed `task` or carries a
   deadline, each with a *Make task* action (`mecha mail task`, deadline
   carried).
4. **Still waiting** — `mecha mail list --aged`, which the morning briefing
   appends today; the digest takes that section over.

It is a reader of the stores, like `--aged`, so the CLI, a trigger and the web
page show the same thing. **And it keeps `mecha mail list`'s split between
looking and recording, and records only what was delivered.** `mecha mail
digest` only reads; it prints the digest and an id naming exactly what it
covered. `mecha mail digest ack <id>` records it — the day-two threads as
surfaced (what `list --aged --surface` writes today) and the "handled for
you" rows as reported, so the next digest starts after them. The record is a
separate step because a trigger's `notify` is an observer whose failure is
logged and never fails the run: a digest that marked itself reported before
its publish failed would drop that day's automatic actions from every later
digest, and the digest is the notice for unattended actions (§5.2, §5.5).
So `ack` is chained after the publish succeeds, and a failed publish leaves
the next morning's digest covering both days. The ledger stays the complete
record either way. A digest that wrote on every read could not be used to
look. **It arrives each morning with the briefing (R9)**: the `morning`
trigger's `notify` step renders the digest where it runs `mecha mail list
--aged --surface` today, publishes it on the same page by the same path, and
acks it — no second schedule or channel. The `one_line` text is fine here: the
digest is a surface for a human, never input to a privileged run
(`Record::for_privileged_run` stays as it is).

`digest` as a rule action means "make sure I see this in the next digest",
and nothing else: it moves no mail, which is why it is a record action. It
runs after `classify`, never on the timer, because the digest shows the
verdict's one-line (§4.1). "Archive it, but show me tomorrow" — the newsletter
you want to skim but not in your inbox — is two actions, `archive` and
`digest`, and the archive follows §4.1's deferral like any other. `task` as an action
creates the task directly, and **what it reaches is not only the board**: a
task's name goes verbatim into `tasks::work_prompt`, the frame of a run that
has tools and may be unattended. `mecha mail task` names a task from the
classifier's `one_line` or the subject — prose a sender influences — which is
fine when the owner presses `t` on a thread they read, and is not fine
unattended. So **an automatic `task`'s name is written by the harness, never
taken from the message**: the rule's own `description` (the owner's words)
followed by harness-filled slots from a closed set — the sender's address
(one address that parses, else omitted), the date the message arrived, and
the deadline if the verdict carried one — with the thread pointer in
`context`. So each task names its own email rather than every task from a
rule sharing one name, and what reaches a task run is still typed rather
than quoted — the
reviewable-object rule applied one surface further. A proposed `task` offered
in the digest keeps today's naming, because the owner's tap is the review.
The approval screen still recommends `propose`, since a board filled by
whoever can send you mail is a board you stop trusting.

---

## 8. The web surface — `#mail/rules`

Inside Mail rather than Settings: it is watched daily, and the rows link into
the mail desk's reader. Every mutation is a `mecha mail rule …` CLI child, the
`/triggers` and outbox pattern, so the page can do nothing the CLI cannot.

**The list.** One row per rule: name, mode (per group, as chips), fires in
the last 7 days, last fired, pending in the outbox, failures, the owner's
undo and edit rate, and a pause toggle. A rule that stepped itself down
shows why. A rule omitted by §4.2's entry cap shows `held: list cap`, its
measurement unavailable, the cap and the entry count, with the same
attention in `mecha doctor`; a missing evaluation must never look like a
zero match rate.

**A rule.**

- The definition, editable in a form over `rule.toml` (`TomlForm` exists)
  and the template files shown as rendered text — what the recipient will
  read.
- **The ledger**: one row per action — when, which message (sender and
  subject, opening in the reader), matched because (header key, or the
  grounded `rule_quote` for active actions or the reconciled quote for
  shadow rows (§4.2)), the action, the mode it ran in, **the match digest in
  force for the row's group when it was written** (§4.2: the matcher keys,
  including `match.describe`, `priority`, applicable origin-gate status and,
  for a description rule's confirmed groups, the compiled confirmation
  version), **the always-written `verdict_input`** (`snippet` or `full_body`
  for a description rule's final matching verdict, `not_applicable` for
  header rules or rows with no matching verdict, and `unknown` when a value
  cannot be read), and the outcome. Every P1 row writes `not_applicable`
  from the start (§4.2), because an omitted field would otherwise read as
  `unknown` and never join the header bucket; the reader still treats
  missing or unreadable values as `unknown`, never as `not_applicable`. The
  top-level `description` label and a per-rule full-body boolean are not
  match-digest inputs. The page computes rates under the current match
  digest only, separately for each `verdict_input`, and shows earlier
  digests apart under a dated heading naming the change. Snippet and
  full-body rows never share a rate, even under the same digest;
  `not_applicable` and `unknown` are separate too, with no pooled headline
  rate, because escalation is per thread and can come from another rule.
  **Every description rule's shadow rate is labelled an upper bound, in
  every group; a header rule's shadow rate gets that label only for outbound
  and Board groups.** All description shadow groups bypass the main pass's
  single choice; their outbound and Board groups also bypass the
  confirmation choice. Shadow outbound groups of either kind bypass §4.3's
  slot, and Board groups still face the per-thread limit and caps. A
  gate-failing shadow rate is additionally labelled a **ceiling** (§4.2);
  both labels mean potential overstatement, with the ceiling naming the
  closed gate. Typed sends also face §5.3's grounding check. Recipient text,
  notes and templates remain outside the match digest, so their edits
  preserve matching history unless the applicable gate status changes.
  Acknowledgement permitted by §9 excludes `to` and changes neither gate
  status nor match digest, so it always preserves matching history. Adding,
  removing or changing the mode of a send in a non-off group may change
  escalation across rules; each new row records the evidence its verdict
  actually read. Shadow rows use the reconciled union of grounded matches,
  with one would-have row per action even if both results name the rule;
  active actions still require the active `rule` selection (§4.2). Earlier
  ceiling rows stay visible after an origin-gate transition (§4.2).
  Outcomes: `pending` (written before an action executes, §4.4), `done`,
  `would have` (shadow), `offered` (a proposed mailbox, record or Board
  action waiting in the mail desk or the digest), `staged`, `held: rule cap`
  and `held: global cap` (§6.4), `held: fate unknown` (a send interrupted
  mid-flight, §4.4), `sent`, `released unchanged`, `edited`, `rejected`,
  `undo_pending` (a durable correction intent before restore, P1c-1 §11),
  `undone`, `expired` (§4.1), `failed: <why>`, `skipped: <guard>`. Undo intents
  and settlements carry `correction_of` naming the original completed row;
  the settlement also names the intent in `settlement`. One more
  row is not an action outcome and touches no action's key: `approval
  lapsed: files changed` (§6.2), a rule-level row for the page, reporting
  only non-off groups; an off group's old digest triggers no lapse row or
  notice. It is listed here so the unknown-outcome rule cannot mistake it
  for one. `skipped` rows are shown, not hidden — a guard visibly firing is
  how the owner learns to trust the guards.
- **`held: list cap`** when §4.2 omits the rule, with the cap, entry count
  and demotions that would make room. It is an evaluation hold shown by
  `mecha doctor` too, not the ledger's closing `held` action outcome,
  because no action ran; measurements are unavailable while held.
- **The share of staged rows by reason** (§5.4), so the owner can see which
  per-message guards explain why a group in `auto` is sending little.
- **Undo** on mailbox rows (archive → back to inbox, trash → restore), and
  **Wrong match** on any row, which records a correction (§10) and counts
  toward the brake.
- **Test against recent mail.** Header rules: a live `mail_search` over 30
  days, instant. Description rules: re-run over the last N classified
  threads, with the cost shown before it starts, run detached and watched.
- **The approval screen** of §6.2, which is also how a drafted reply
  graduates (§6.3).

**From the mail desk**: *Make a rule from this thread* — a new rule prefilled
from the thread's headers (sender, domain, list id), no model, opened in
`shadow`.

The ledger is append-only JSONL, and its reader follows `Proposed`'s rule:
**an outcome or action this build does not know degrades to a default rather
than failing the row.** A closed enum written to an append-only store is a
wire format — and **each store states its default's direction**, as
`imagelib::Status` and `imagelib::Origin` do, because the two stores need
opposite ones. In `approvals.jsonl` an unknown mode reads as **`shadow`**:
it never acts, so a newer build's rung cannot raise anything in an older
one. In `ledger.jsonl` every row also carries a **`closes` boolean**,
written by the build that writes the row according to §4.4, and **it may
only tighten**. A reader first runs the settlement lookup (a `pending` row
is superseded by its settlement row, below). Then **a key is closed if any
remaining row closes it by any of three signals**: the row says `closes:
true`; the reader's own §4.4 table closes the row's outcome; or the outcome
is one the reader cannot read. A union, so `closes: false` can only ever
mean "this build adds no closure" and never overrides the reader's own table
— a `sent` row marked `closes: false` still closes the key in a reader that
knows `sent`. And an omitted field on a `would have` row closes nothing,
because neither the field nor the reader's table nor unreadability says to.
A newer build's `closes: false` on an outcome an older reader does not know
is therefore not permission to run the action again — for an outbound row
that would be a duplicate send, which is irreversible, while the opposite
error (an older reader treating a thread as handled) is visible as `unknown`
on the page and in `mecha doctor`. That inert-in-an-older-build cost is
accepted, as Outlook sends staying in `propose` is elsewhere. **A `pending`
row's `closes: true` holds only until its settlement row exists** — a
settlement row names the id of the `pending` row it settles, so a second
`pending` under the same key after a reopen is never mistaken for the first
one's settled state: then the settlement row's own `closes` decides, as §4.4
has it, so a Mailbox `failed: interrupted` reopens the key while an outbound
`held: fate unknown` keeps it closed — and a settlement row an older reader
cannot read closes the key. One boolean serves both of §4.4's keys: over the
outbound and Board rows the per-thread lookup reads, the two closing sets
agree on every outcome that can appear first in a thread (`released
unchanged`, `edited` and `rejected` only ever follow a `staged` row that has
already closed it; `undone` is Mailbox-only, which the per-thread lookup
never reads). Unknown modes and outcomes are shown as `unknown` on the page.

---

## 9. A conversation may draft a rule; only the owner installs one

The owner's wish behind R4 — "the harness can create them as needed" — is
real and worth serving, and the safe shape exists already in
`hud_propose` and `persona_propose`:

- `mail_rule_propose` is a tool a chat can call with a rule's TOML and
  template text as its *input*. It writes to `proposals/<id>/`, never to
  `rules/`, records its `Origin` from `ToolCtx::recorded_taint`, and returns
  no data. **The write is checked before any byte lands**, not only by the
  loader afterwards: the store is outside `ToolCtx::resolve`'s reach, so a
  `template = "../../skills/…"` would otherwise create a file where §0 says
  the harness may never write. Every model-supplied name is validated at the
  write: the rule's name by `persona::validate_name` (lowercase, digits,
  `-`, `_`, a leading alphanumeric, length-capped), and every file it names
  as a **stem** passing the same check plus an **extension matched exactly
  against a closed list** (`md`). Since the stem can hold no `.` and the
  extension is compared whole, a name has exactly one dot and no separator,
  so neither `..` nor a path can be spelled — `reply.md` passes,
  `../reply.md` and `reply.md/x` cannot. Containment in `proposals/<id>/` is
  still proved before the file is created, as the second of two checks
  rather than the only one. The proposal id is minted by the store, never
  taken from the model.
- The proposal is checked by the same loader an installed rule uses, so a
  proposal that previews clean has a valid definition. Installation also
  checks §4.2's current list-entry capacity, which may change after preview.
- The owner sees the rendered rule — the template as the recipient would
  read it — and installs it. **Installation is always into `shadow`**,
  whatever the proposal says, and an armed proposal is labelled as one. The
  accept names the digest the owner was shown. An install that would add an
  entry beyond §4.2's cap is refused with the cap and remedy; installing in
  `shadow` is no exception, because its description still occupies the
  frame.
- **The recorded `Origin` gates what can send.** The confirmation frame
  (§4.2) must be shaped by the owner's words or a clean proposal, never
  unchanged prose shaped by a stranger. Acknowledgement changes no bytes, so
  it cannot release an untrusted description into that frame. A proposal
  drafted after reading mail installs a description whose origin is
  `Origin::of_proposal`'s untrusted answer. So **no outbound action group —
  and not the Board group either, whose `task` name reaches a task run's
  prompt (§7) — may enter `auto` while any part of the rule remains
  model-proposed text from an armed conversation: every key of `rule.toml`
  and every file it names.** The gate is the whole rule rather than a list
  of fields, because a list stops being complete without anyone noticing: in
  review it grew from the description to templates, the sender keys (§5.2's
  control on *who can make a send leave*, which a header rule has no
  description to hold back), `fields` (which shape the extraction frame, a
  send decision), `note` (words a forward sends) and `draft_with` (which
  chooses the skill whose `shareable` files are given whole), and a key
  added later would have been the next omission. Every one of those, and
  every future key, is covered by construction.

  And `to` is held to a stricter rule than words: **an outbound group may
  enter `auto` only when its `to` is `Owner` — written or edited by the
  owner, never an address left as proposed, even from a clean
  conversation.** `to` bounds where every automatic forward and send lands.
  Neither installation nor acknowledgement adopts it: recognising an address
  does not defeat a lookalike domain whatever the conversation's taint, and
  a forward sends the whole thread there. The approval screen shows `to`
  first with its origin and offers editing, never acknowledgement.

  **Acknowledgement adopts text only when its origin is `ModelClean`, and
  never adopts `to`.** It remains available for `match.describe`, `note`,
  templates and other non-recipient text. Every `ModelUntrusted` field
  requires an actual owner edit to become `Owner`, following
  `Reflexion::provenance`: an edit makes the bytes partly the owner's, while
  acknowledgement makes none of them so. The store records `Owner` for text
  the owner wrote, edited or acknowledged within that scope, and otherwise
  keeps `ModelClean` or `ModelUntrusted`, in `imagelib::Origin`'s terms.

  **The provenance record and acknowledgement bind to `(rule, field,
  digest)`**, as defined in §3: this installed proposal's identity, the
  exact key (including its owning action) or named file, and the shown
  bytes. Another rule or field with identical bytes cannot reuse that
  acknowledgement, and a later re-proposal gets a fresh provenance identity
  even under the same rule name. Acknowledgement records `Owner` without
  rewriting `rule.toml`; changed bytes cannot inherit it either. Only
  `Actor::Owner` may acknowledge, with the same registered-run refusal as
  §6.1's raise gate, advisory without the sandbox, because a run must not
  attribute its own proposed words to the owner. It does not itself raise a
  mode or change the gate status.

  The gate admits `Owner` and `ModelClean` text **except for `to`, which
  requires `Owner`**. In practice it holds the `ModelUntrusted` texts and
  every address left as proposed; acknowledgement releases none of them. The
  approval screen names each text holding the group back and offers editing.
  Other `ModelClean` text may be acknowledged, but that changes neither gate
  status nor match digest. The applicable gate status enters §4.2's match
  digest directly: an edit that changes it from failing to passing starts
  confirmed measurement for the groups the confirmation pass decides,
  keeping the earlier ceiling visible apart. Editing a recipient, note or
  template without changing the gate keeps matching history, even though a
  file edit still lapses approval (§6.2). The narrowed digest is what
  preserves sibling groups' history when their gate status stays unchanged
  after adopting `to`. Even if acknowledging an address were permitted, the
  outbound gate transition would still reset that group's measurement.
  Shadow bounds when a proposed rule acts; this bounds what it may say.

So "write me a rule that answers prerequisite questions with the usual
reply" works from a chat, ends as a rule in shadow mode, and the first
automatic send is still the owner's decision made against a week of real
matches.

---

## 10. Corrections

**Wrong match** on a **description** rule's ledger row writes a field-level
correction (`field = "rule"`, was, now) to the triage record, in the shape
`MAIL-UX-DESIGN.md` §6 already defines, so it joins the classifier's few-shot
pool. On a **header** rule's row it does not: a header match is a string
comparison the classifier never made, teaching it otherwise is the noise
`MAIL-UX-DESIGN.md` §6 warns about, and with several header matches allowed
per thread `(thread_id, field = "rule")` would not identify one correction.
A wrong header match means the matcher is wrong — an edit to `rule.toml` —
so it records the ledger outcome, counts toward §6.4's brake, and the page
offers the edit. That makes rule
precision improve the way bucket precision does, with nothing new. **Missed
match** — from the mail desk, *this should have matched rule X* — is the same
correction in the other direction.

---

## 11. Build plan

Each phase is useful alone and testable without the next.

**P0 — mecha-mail prerequisites.** Expose `Auto-Submitted`, `Precedence`,
`List-Id`, `Authentication-Results`, `Reply-To` and the owner's
participation in a thread on the thread read — `Authentication-Results` as
**every instance, in received order, each with its `authserv-id`**, because
§5.4's guard needs the count as well as the top one; absent stated rather than
defaulted. A test per provider in **both** directions — a missing, foreign or
duplicated provider instance proposes (on Outlook, a submitted `compauth=`
instance beside EOP's), and a single genuine provider instance passes —
because a test of
the negative alone passes on a reader that proposes unconditionally (§5.4); the
immutable-id preference on the rule engine's Graph calls, verified to be
accepted by a move and a restore on the owner's mailbox, with the
`conversationId` question that decides its scope (§4.4); each account's own address available to
the rule engine, with absent reported as unknown (§5.4); and the provider's own message id on the
thread read for the ledger key (§4.4). Restore verbs for undo (back to
inbox, out of trash) on both providers. Nothing changes on the send side:
rule replies go through the existing `mail_reply` path as ordinary mail (R5).

**P1 — the store, header rules, mailbox actions.** R10 splits delivery into
P1a (the merged store), P1b (the core engine and `mecha-mail rules apply
[--headers | --after-classify] [--account NAME] [--dry-run]`), and P1c (the
`mecha mail rule …` store-management verbs and child launcher, timer units,
classify hook, feature registry, doctor and backlog). The engine's `Mailbox`
and `TaskSink` interfaces contain no provider clients or credentials. The mail
binary implements them for Gmail and Outlook and connects to the globally
configured `[[mcp]]` entry named `graph` for tasks. An absent or unavailable
task sink holds the task without consuming its action key, once-per-thread
occasion or cap capacity; the next pass retries once the sink is available.
This does not reopen cap holds or tasks with uncertain dispatch fate. `mecha`
gains no runtime dependency on `mecha-mail`.
**P1c-1 implemented (2026-11-06).** `mecha mail rule add | list | show |
mode | test | ledger | undo | pause-all` is the owner's CLI. Store operations
use core directly; provider operations launch only the `mecha-mail` sibling in
the canonical directory of the current executable, using argv and preserving
output and exit status. PATH is never searched; a missing sibling names the
`cargo install --path mecha-mail` remedy.
The CLI shares the outbox's shell/posture/run-ancestry actor reading. Only a
positive owner can raise a group; non-owner `off` requests clamp to `shadow`,
and pause-all preserves already-off groups. Auto approval shows the exact
rule-directory inputs, relevant skill/global inputs and their digest, and needs
`--yes` or interactive `yes`; the store rejects any intervening edit. List,
show and ledger offer read-only JSON, with recorded/effective group modes,
load errors, lapse/brake/list-cap and provider holds, and last activity.

`mecha mail rule test NAME` launches **`mecha-mail rules test NAME`**, a
separate command with no live switch or timer options. It enumerates the last
**30 days across all mailbox folders**, including archived mail, and reports
header matches for individual inbound messages with fresh `in_inbox` evidence
and an explicit label when a message is no longer in the inbox. It answers
what the matcher would match, independently of activation, modes and prior
ledger closures; it does not claim those actions would execute. Unknown
membership is labelled unknown. Description rules refuse with the P4 requirement.
No tokens are refreshed and no store, annotation, claim or task is written.
Live `rules apply` retains its seven-day inbox window and always considers every
rule for removal and handled safeguards. Repeatable `--rule NAME` is accepted
only with `--dry-run`, enforced by argument parsing and the core engine; dry-run
selection never sweeps or expires another rule's snapshots.
Both listing paths share continuation validation and a 100-page ceiling;
incomplete enumeration is a counted failure, never a successful truncation.

`undo` launches `mecha-mail rules undo ROW-ID`. It restores only the immutable
provider message recorded by a completed Mailbox action: Gmail unarchive/
untrash, Graph move back to Inbox, and inverse read/unread verbs. The latter
reverse the verb; historical rows contain no prior read-state snapshot. Private
record actions have no restore implementation here; spam, sends and tasks also
refuse. An append-only `undo_pending` intent links to the completed row before
provider I/O, closes its undo key and counts toward the undo-rate brake immediately.
Successful restore appends an `undone` settlement linked to that intent; a failed
restore appends `failed` and ceases to count toward the undo rate. A persisted brake
is not automatically raised again. If settlement writing fails after provider
success, retrying `undo` on the original row resumes the same intent and re-drives
the idempotent restore; already-restored messages are nothing-to-do successes.
The retry never creates a second intent, concurrent attempts are refused by the
engine lease, and any settled undo (successful or failed) refuses a fresh undo.
Ordinary action recovery never settles undo intents. Older readers degrade
`undo_pending` to `unknown`, keeping the closed-enum wire rule. The engine lease
also serializes restores against apply.

Pre-timer fixes also reject empty header OR lists, pass the engine's clock into
mailbox windows, report live expiry rows just like dry runs, hold unknown
activation, report every non-zero failure counter, warn about broken triage
overlays once per list pass, and share the conservative 254-byte mailbox grammar.

**P1c-2 implemented (2026-11-06).** `mail_rules` has its own global switch,
off by default, and requires `mail`. `mecha-mail rules apply` checks both before
opening any store or provider; a disabled timer or classify hook says so once
per invocation. Active owner verbs require the feature. `mecha mail rule list`,
`show`, `ledger`, `pause-all` and lowering modes deliberately remain available
with `mail` or `mail_rules` off: inspection and revocation must not depend on the
switch. The hand-installed
`mecha-mail-rules.service` / `.timer` runs headers every five minutes, with
`~/.cargo/bin` on PATH and no model-idle gate. Packaged unit copies are held
byte-equal to `scripts/`; the classify timers have no launchd twin to mirror.
Typed engine-lease contention is a successful skip, while listing, read,
dispatched-action and handled-marker failure counts cause a non-zero exit.
Held passes are not failures.

After durable verdict writes, classify uses the same canonical-sibling launcher
for `rules apply --after-classify`, preserving the account selection. This also
runs on an empty non-dry classify pass so deferred work can finish. Hook output
and attempted/failed counts go to stderr; child failure, a missing binary,
unreadable global config or timeout cannot alter classify's own stdout or exit
status. Dry runs never launch it. A failed verdict write does not launch it.

**P1c-2 adversarial review: direct entrypoints and service deadlines.** The
mail binary checks the global `mail` and `mail_rules` switches before undo or
retrospective test opens stores, tokens or providers. Test is read-only but
still contacts a mailbox, so it requires the switches just like the owner CLI.
Unreadable configuration refuses every rules subcommand. Disabled unattended
apply remains a successful skip; explicit test/undo refuse.

**P1c-2 review correction (2026-11-06):** the classify hook gets a fixed
ten-minute total allowance for deferred work and seven days of verdicts. The
running deadline reserves ten seconds for SIGTERM grace and two seconds for
reaping after SIGKILL if needed, including when output pipes are held open.
A deadline is reported and counted as a hook failure even if SIGTERM makes the
child exit successfully; classify keeps its own status. This also applies on a
terminal and does not mistake a watchdog interval for a service start deadline.
Both classify services reserve ten minutes beyond their original sweep budgets:
daytime is now 40 minutes, nightly 55 minutes.
Script and packaged service copies are byte-equal. **Deploy note for this PR:**
re-copy `mecha-mail-classify-day.service` and `mecha-mail-classify.service` from
`scripts/` to `~/.config/systemd/user/`, along with the new mail-rules units,
then run `systemctl --user daemon-reload`. Existing classify timers need no
change. Updating binaries alone does not update these installed timeouts.

Doctor reads stores without writes and reports unreadable rules/history,
lapsed approvals (computed or persisted), unknown ledger outcomes, corrupt
approvals and the resulting inability to write `off`, missing stored addresses
(which hold the whole install), standing provider/list-cap holds, stranded
pending actions and interruption settlements. Global-config checks report every
automatic group under unconfined shell and missing, inactive or failing timer
units while enabled. An unavailable systemd query (no binary or user bus) is
skipped, matching doctor's other unit checks; returned missing/failed unit
states remain findings. No provider or model is called.

The sixth backlog store counts lapsed group approvals and remaining proposal
directories under `proposals/`, with proposal age taken from `rule.toml`'s file
modification time until P7 supplies proposal metadata and lifecycle. It creates
nothing. An unreadable or deleted definition falls back to its approval history,
so persisted lapses, other rules and proposal counts remain visible; unreadable
shared history remains unknown. `mecha review` and `/queues` show the count.
P3 has feature-owned list/show/ledger API routes, a `mail/rules` PANE_FEATURE
entry and a `mail rules` QUEUE_FEATURE entry; VIEW_FEATURE keeps the Mail view
under `mail`. No rules tab or page is added early. Model slots, a new sidecar,
automatic service installer and a new settings table are not needed: this uses
the existing mail binary and mail timers' manual install procedure.
No P2–P7 execution behavior is added.

**Carried P1c-1 review fixes (2026-11-06).** Mail views use the shared mail-store
directory resolver. Emergency pause skips junk entries with a named notice,
while add/list retain strict validation. The store's permanent ceiling is
checked before displaying the automatic approval screen and rechecked when
writing permission. Undo with unreadable rule files durably clamps historical
groups before provider I/O; replacing the old files cannot revive auto. Empty
text lists and ledgers explicitly report no entries; JSON arrays remain empty
arrays, and showing a missing rule remains an explicit error.

The following describes P1 as a whole:

 `rule.toml`, approvals,
the ledger, `off`/`shadow`/`propose`/`auto` for the Mailbox, Private record,
Report (`spam`, which never rises above `propose`) and Board groups — `task` on header rules, with its harness-written name
(§7); the confirmation pass that description-matched tasks also need lands
with description rules in P4 — (including a `tag` action, which writes mecha's own tag onto the
triage record and has no provider verb, since tags are never Gmail labels or
Graph categories), the five-minute timer and the hook at the end of
`classify` that runs `digest`, `task` and every deferred action (§4.1, §4.4),
`mecha mail rule add | list | show | mode |
test | ledger | undo | pause-all` — P1 is where rules first act in `auto`, so
the panic button ships with them, and so do §6.4's brakes over P1's actions
(the undo-rate brake, the wrong-match brake — a header rule's wrong match
counts toward it, §10 — and the consecutive-failure brake) — the feature
registry entry (off by
default) — through
`ARCHITECTURE.md` §Features → *Adding a feature*, whose dozen places include
the web pane's `PANE_FEATURE` entry P3 will need (#654 missed exactly that) —
and
`mecha doctor` reporting an unreadable store, a lapsed approval, and ledger
rows with an outcome this build cannot read (§8), corrupt approvals, absent
account addresses, standing provider holds, stranded/recovered pending actions,
and the enabled timer's health. **P1c-2 review correction (2026-11-06):** attention
for **any group in auto while shell runs unconfined**, including Mailbox, Board
and outbound groups; the sandbox is the remedy (§6.1). The store
also registers with `backlog.rs` (and `CLAUDE.md`'s module-map line for it,
"across five stores", and `backlog.rs`'s own module doc, "Five stores
collect work for a person", each gain one), so proposals awaiting install and lapsed
approvals count in `mecha review` and `/queues` — what is *waiting*, beside
the doctor's what is *wrong*; rule sends staged in the outbox already count
there through `OutboxStore`. **Covers
GitHub notifications and junk with no model involved.** Until P2 ships the
digest, the trace §5.5 promises is the ledger and `undo` — pulled rather than
pushed — which is acceptable for P1's actions because all of them are
recoverable and none reaches anyone — which is true because `spam`, the one
verb that would reach the provider's filter and has no restore, never runs
in `auto` (§5). The exception is `task`, which reaches a task run's prompt,
so **P1 ships the caps over the Board group with it** (§5.4, §6.4): a header
rule's automatic tasks are capped per rule and globally from the first day
they can run, and named by the harness (§7).

**P1 fixes the append-only wire format from its first row.** Every ledger
row writes `verdict_input: not_applicable` (§4.2); every match-digest
component is written, empty when absent, including P1's absent confirmation
version. Every ledger row also writes `closes`, and each settlement row
names the id of the `pending` row it settles (§8). Later readers must be
able to separate old measurements and settle the right attempt without
rewriting history, so these fields cannot wait for the classifier or sends.
**P1 also ships both actor gates (§6.1):** raises require a positive
`Actor::Owner` and otherwise are refused; writing `off` requires the same
positive reading and otherwise clamps the target to `shadow`, with the CLI
saying so and already-off groups left untouched. Rules already act in P1, so
a run must neither raise its own permission nor erase the would-have record
by switching it off.

**P1b implementation choices (2026-11-03).** With neither pass flag,
`rules apply` runs `--headers`; the two flags are mutually exclusive. Header
address, domain and account keys are exact ignoring ASCII case; domains do
not include subdomains implicitly. `list_id` compares the identifier inside
required angle brackets (with an optional phrase and trailing comments), and
`subject_contains` and `header.<Name>` search semantic text case-insensitively,
OR within each list and AND across keys. Raw-header comparisons unfold
continuations and decode RFC 2047 B/Q words in UTF-8, US-ASCII and Latin-1;
unsupported charsets or malformed words are unknown, not negative matches.
Graph's already decoded subject is unfolded but never decoded a second time.
Stored/displayed values remain raw. Malformed or duplicate List-Id fields also
remain unknown, even with a complete header collection. An absent field is a
nonmatch only when its absence is known; truncated or unknown header collections and omitted attachment flags yield unknown matches.
Completeness is per field: Graph's dedicated subject and To/Cc/Bcc collections
can prove known negatives even when `internetMessageHeaders` is incomplete.
A potentially matching digest/task/outbound rule defers removal and prevents a
classification skip. Unknown-fact holds are visible in the apply report without
claiming action keys, and queued work remains queued until the facts resolve.
No body is matched. Gmail sender extraction uses mailbox grammar, including
quoted display names, comments and address lists; ambiguous or malformed From
fields remain unknown and hold actions. Newest-inbound selection uses provider
direction evidence (Gmail SENT/DRAFT; Graph sent/draft folder facts or isDraft
with an owner-address check), all account addresses and `aliases.toml`.
Unknown direction holds; a sent alias reply cannot replace an older inbound.
MCP transport failures preserve their local type through ToolOutput. Graph
process exit or broken pipe holds task fate and reserves its occasion/cap without
tripping the Board failure brake; a tool refusal still counts.

**P1b review corrections (2026-11-03).** A mailbox mutation addresses exactly the
authorized provider message ID, never the thread ID. Both Gmail and Outlook
support this; Outlook keeps immutable IDs on every request. Arrival eligibility
uses only Gmail `internalDate` or Outlook `receivedDateTime`; a sender-written
Date is display data. Unknown arrival holds eligibility. A provider that only
supports thread mutations must hold unless every affected message independently
passes matching, activation and deferral. Each dispatch rechecks the fresh
ResolvedMode activation; reactivation during a sibling's I/O settles an
ineligible pending attempt as skipped.

A separate engine lease serializes apply runs across accounts, preserving
atomic cap reservations while the store's existing short lock protects
per-action claims. Stranded pending attempts older than fifteen minutes are
swept; an individual effect times out after two minutes, with a task held
as fate unknown. A fresh owner approval acknowledges earlier brake events;
new adverse evidence can brake it again. The cap day uses the global agent timezone (UTC when absent); dispatched
attempts of uncertain fate consume capacity conservatively. Sibling reservations
allocate capacity cumulatively; semantically duplicate actions, including tables
that merely spell out defaults, are rejected at load. The injected clock is read
for each reservation and dispatch; cap checks and attempt timestamps use the
same dispatch reading. Settled attempts are charged to that local day; unsettled
attempts retain capacity conservatively across midnight. Typed provider failure
kinds survive partial mutation results: only entirely environmental failures are
exempt from the failure brake. Gmail's structured 403 reasons
`rateLimitExceeded`, `userRateLimitExceeded` and `dailyLimitExceeded` are
environment failures; other, mixed or malformed 403 reason sets remain action
failures. A dry run simulates
settlements and brakes but writes no rule or triage state and starts no graph
server; provider reads use cached credentials and refuse when refreshing
would be required.

The P1b headers path retains its newest-500 inbox bound and limits provider
arrivals to the last seven days (unknown arrivals remain visible for holds).
After-classify enumerates outstanding deferred entries for the account plus
verdicts recorded within the same seven-day interval; missing, malformed or
future verdict timestamps do not establish recency. Expired occasions close
in the ledger and are refused by both claims and enqueue, independently of
candidate enumeration, including dry-run planning and legacy expiry rows.

**Known minor, deliberately unfixed (review four):** raw downstream diagnostic
text still flows into ledger `reason` and apply-report notices. The owner will
track this on the PR; this change does not sanitize or otherwise alter it.

An after-classify candidate needs a usable verdict whose envelope date covers
the newest inbound message and whose classification timestamp is no earlier
than that message. Task names use the provider's UTC arrival date; daily
caps use the owner timezone described above. A missing/stale verdict keeps owed work queued. Rules'
private tags, digest flags and message-specific handled markers are stored in
`mail-triage/rule-records/`, overlaid on triage reads, so classification neither
loses them nor mistakes a pre-classification tag for a verdict. P1c consumes
the handled marker and deferred queue in its classify hook. P1 has only
owner-authored live rule files; the proposal provenance reader must replace
that assumption when P7 adds installation from conversations.

**P2 — the digest.** `mecha mail digest` and `digest ack`. Then the briefing
swap, which is **an edit on the owner's machine, not in this repository**:
the `morning` trigger lives only under `~/.mecha/`. It is the `update`
skill's territory and carries a trap `HANDOFF.md` already records for
`--aged`: the installed binary must have `mail digest` before the `notify`
line names it, or the briefing pastes a usage error into itself. Install
first, then swap, then confirm by running the `notify` line, not by reading
it.

**P3-1 implemented (2026-11-06).** The backend for the P3 page now has
stdin installation, atomic `edit`, digest-bound JSON `mode`, per-rule `pause`,
header `wrong-match`, retrospective JSON testing, enriched list/show/ledger
JSON, and eight feature-owned POST routes. Every mutation is a CLI child;
no rule-store writes occur in serve. Svelte and the mail-desk button remain
P3-2. The JSON field and route contract is in ARCHITECTURE §Mail rules.

Implementation choices where the plan did not specify details:

- Keep P1c's separate `mecha-mail rules test NAME`, adding `--json` as an
  explicit spelling of its existing JSON output and adding per-account
  status/counts/holds/rows. It still searches 30 days across all folders,
  without token refresh or writes. A held or incomplete account has a null
  match count, with partial rows retained. The serve budget is 600 seconds,
  selected conservatively for pagination plus per-message reads (30-second
  HTTP deadlines); no live mailbox timing measurement was made.
- Add accepts `--stdin --name NAME`; serve passes `--stdin --name=NAME`, which
  binds the value whatever it begins with. Rule names `add`, `ledger`,
  `pause-all`, `undo` and `wrong-match` are refused at install, since those
  fixed route segments would shadow a rule's page. Edit
  accepts `--stdin` or `--file PATH` (`--file -` also reads stdin). Drafts
  stay on a bounded 1 MiB stdin pipe; serve creates no draft file. Editing
  changes only rule.toml and preserves bundled files. The full candidate is
  validated before durable changes. Lapses are persisted before publication:
  a failed rename may leave permission lowered, never recoverably automatic.
  An unpersisted lapse of the old files is recorded first, so restoring approved
  bytes after an external edit or invalid TOML cannot revive auto. If both
  old and replacement snapshots lapse approval, history retains both
  transitions while the edit report deduplicates affected groups.
- The mode digest is the complete existing ApprovalDigest JSON object,
  checked under the store lock. A stale supplied digest refuses even a
  lowering; a lowering without one still works. Serve requires a digest
  for any raise via the CLI's `--require-digest` guard, including shadow
  from off and propose from shadow. Auto still requires `--yes` or an
  interactive confirmation. Refusals carry typed JSON codes.
- Wrong-match writes `wrong_match` with `closes:true`, copying the original
  action key and naming `correction_of`; it never settles a pending attempt.
  This intentionally closes even a previously non-closing shadow key: the
  owner marked that matcher wrong, and neither this build nor an older
  reader (which sees unknown) may re-run that action. It records no triage
  correction and performs no undo. Existing undo remains available, and a
  duplicate wrong-match is a no-op. The engine lease serializes it with
  apply/undo; retries also retry a failed brake write.
- Retain the existing brake units: wrong matches deduplicate per thread,
  undo per automatic mailbox action. A row both undone and marked wrong
  contributes once to each distinct brake, never twice to either. Page
  combined corrections deduplicate action keys; owner rates require a
  recorded owner actor. New undo and wrong-match rows record attribution;
  historical missing attribution remains unknown. The shared CLI actor
  reader moved into core for the provider binary to use the identical
  posture/registry/run-marker reading; no authentication or request-causality
  scheme was added. Safety brakes remain conservative for non-owner
  corrections, which may lower but never raise permission.
- Rates are partitioned by group, current match digest and verdict_input;
  they never pool, and empty denominators are null. Fires count completed
  actions in seven days; failures count failed ledger rows. The ledger
  already holds sender and subject; missing text is null and the page does
  not fetch mail. Show includes rule.toml text even when parsing is broken,
  provided the contained file remains readable. Send/description fields
  remain extensible; their execution and approval screens remain P4+.
- Under the orchestrating session's choice within the existing closure
  residual, serve-child attribution is unchanged. Ordinary live chats do
  not mark serve itself as a run ancestor; a model reaching loopback with
  the expected headers can cause an Owner-attributed CLI child. Doctor now
  emits one finding per automatic rule/group for registered shell, stating
  every applicable write and network route and its remedy, including
  backends whose `can_reach_network()` stays true with network disabled.
- The redundant mail-rule dispatch after the early return is now an explicit
  unreachable exhaustiveness arm. Other carried minors remain untouched.

**P3-2 implemented (2026-11-06).** `#mail/rules` is one responsive page
(`web/src/lib/MailRules.svelte`) that App renders in place of either Mail
build, so the phone and the desk show the same page; its display logic is
`mail-rules.js`, tested as plain node. It calls only P3-1's routes and
`/api/mail/read`. Choices where this document was silent, made in the
implementation and not owner rulings:

- The open rule is page state, not a third hash segment: App's router
  carries one sub-view (`#mail/rules`), and changing routing for every view
  was out of proportion. Back from a rule returns to the list in the page.
- The definition is a plain TOML editor with the store's parse error shown,
  not `TomlForm`: rule.toml has arrays of tables and a header map, which
  the flat field schema does not express. A new rule has a guided form (one
  matcher key, one action, name, description) that writes the TOML until
  the owner types in it; the owner edits before installing.
- *Make a rule from this thread* prefills the sender and the sender's
  domain (matching on the domain), names the rule after the domain, and
  suggests `archive`; the subject is the description only when no domain can
  be read. It carries no list id: the mail views do not
  hold `List-Id`, and §8 adds no provider fetch, so the form asks for it.
- A mode change counts as a raise against the group's **effective** mode,
  as the store measures it, so re-approving a braked or lapsed group (set
  auto, running shadow) opens the approval screen. The screen shows every
  file in the review inputs with the shadow-row count under the current
  match digest, and sends that review's digest; a `stale_digest` refusal
  reloads the files and says so rather than retrying.
- Rates render a dash for a null rate and omit an empty `verdict_input`
  bucket rather than showing zero. Rows under an earlier match digest are
  grouped per group under a heading dated by their newest row; the
  rule-level `approval lapsed` row stays with the current rows and names no
  message. Outcomes, modes and groups this build cannot name read `unknown`.
- A ledger row opens its thread read-only in a sheet over the page
  (`mecha mail show` through `/api/mail/read`), since an archived thread is
  no longer in the triage queue the mail readers walk.
- The test result shows each account's status; a held or failed account
  shows no match count, and partial rows stay visible beside a failure.
- Not on the page yet, all deferred to the phase that produces them rather
  than shown empty: *pending in the outbox* and *the share of staged rows by
  reason* (§5.4) arrive with sends in P4, and `held: list cap` with its cap
  and entry count arrives with description rules in P4 — P1–P3 rules are
  header rules, which the cap never counts, so no rule can be held by it
  yet. Description-rule testing (re-run over classified threads, with its
  cost shown) is P4 too; the Test button surfaces the store's refusal.
- The approval screen's counts are the store's statistics under the current
  match digest, per `verdict_input`, never a count of the ledger window it
  loaded (the newest 200 rows, said so when full). Its route list is
  exported as `RULES_ENDPOINTS` for the docs demo's `check-demo.mjs`, which
  cannot see calls made through the page's fetch wrappers; the demo answers
  reads from fictional fixtures and declines every change.
- Three of the page's slots are written from slow requests: a rule's load,
  a thread read, and a test, which may run for minutes. Each answer lands
  only where it was asked for. A late load for the rule just left never
  fills the open rule's editor, which Save would write. A dismissed sheet
  stays closed, and a test result shows only on the rule it tested, since
  a raise is decided on that count. A reload after Pause, Undo or a mode
  change keeps an unsaved definition edit.
- What a Save and Pause all report is worded in `mail-rules.js`
  (`editNotice`, `pauseAllNotice`), where the node checks pin it. A lapse
  is reported even when the bytes did not move, because an outside edit
  since approval lapses on Save. Pause all names each entry it could not
  lower. A recorded wrong match takes the owner to the definition. These
  live in the component, where the node checks cannot reach. They were
  driven in headless Chromium with delayed responses, and nothing pins them
  in CI. The same holds for the choice that makes re-approving a braked group
  a raise: `setMode` judges against `effective_mode`. The server's
  `--require-digest` is the gate behind that choice. If it regresses, the
  owner sees a `digest_required` refusal, never an unreviewed raise.

**P4a implemented (2026-11-07): description rules over recoverable actions.**
P4 is delivered in two parts, by the orchestrating session's choice rather
than an owner ruling: P4a evaluates `match.describe` in the classify pass for
the Mailbox, Private record and Report groups; P4b adds the confirmation pass,
template `reply`, verbatim `forward` and the send guards and brakes. Until P4b,
a description rule's Board and outbound groups are held at `shadow` ("wait for
the confirmation pass"): they write would-have rows from the main pass alone,
under a match digest with no confirmation version, so P4b's rows stand apart
from them automatically (§4.2's digest rule).

**Owner ruling (2026-11-07), bulk mail.** A thread the pre-filter disposes of
(an unsubscribe header) is read by the classifier only when at least one
admitted description rule's header preconditions fit its envelope, and then
for the rule answer alone: its stored triage verdict stays the pre-filter's
`ignore`. Without this, description rules would be blind to about half of
incoming mail.

Implementation choices where this document was silent, none of them an owner
ruling:

- **The sweep asks only when a rule could answer.** The classifier gets the
  frame only for a message with at least one eligible rule; otherwise its
  prompt is byte-identical to the one without rules. A frame whose every
  answer is fixed at `none` costs tokens and decides nothing. When asked, the
  frame is the whole admitted set in both lists, in priority then name order,
  with the eligible names after the message, as §4.2 specifies.
- **Header preconditions are judged on the sweep's envelope.** The sweep holds
  the inbox listing: sender, account, subject and attachment flag. Eligibility
  uses the engine's own `header_match` over those facts, and Unknown is not
  eligible. A description rule whose precondition uses `to`, `list_id` or a
  raw header is **held** visibly (`held: a description rule's header
  conditions may use only …`) rather than evaluated on a guess: §4.2 forbids a
  rule that cannot apply from taking the single `rule` slot. Lifting the hold
  means carrying those headers into the sweep. The engine re-checks every
  precondition against the full message after classify, and that check is the
  authority for what runs.
- **The record says which matcher each answer is for.** The triage record's
  new `rules` field lists every rule asked about, with its list and its
  matcher digest at the time, plus the grounded hits and the refusals. A rule
  edited since is not answered by the record, so it writes no row until its
  thread is classified again; a rule not asked about is unknown, never "no
  match".
- **Rule-forced escalation changes only the rule answer.** A match on a rule
  with a send in a non-off group reads the full body (§4.2). The stored triage
  verdict still escalates only on its own rule (`needs_body`), so the
  escalation measurement in `Record::escalated` stays its own.
- **Grounding.** Quotes must be a literal span of the subject and text the
  model was shown, between 8 and 300 characters. A name not eligible for the
  message is refused. Refusals are recorded with their reason. The quote
  becomes the ledger row's `matched_because.quote` and is shown on the page;
  it is the sender's prose. P2's digest must not hand it to the briefing's
  model run.
- **Trash on a description rule** caps that rule's whole Mailbox group at
  `propose` (§5.5), since modes are per group. To archive automatically, put
  trash in its own rule.
- **Admission.** Install, an edit and a raise refuse to take the description
  lists over 12 entries, naming the count and the rules to lower. A store
  already over the cap refuses only what would add to it. Lowerings are never
  checked.
- **`match.describe`** must be non-empty and at most 500 characters. The frame
  renders it on one line.
- **Wrong match on a description row** (§10) records the ledger outcome,
  counts toward the wrong-match brake, and writes `rule: <name> → none` to the
  thread's triage record, idempotently, so it joins the correction examples.
  A row whose evidence kind no longer matches its rule (a header row on a rule
  since edited into a description rule) is refused: edit the rule instead.
- Description-rule testing against past mail, with its cost shown, is a
  separate PR (P4a-2). Until then the Test button reports the store's refusal.

**P4b-1 implemented (2026-11-07): the confirmation pass.** `mail_triage::confirm`
is the second quarantined call of §4.2: a compiled instruction and the
candidates' `match.describe` texts, no learned rules and no correction examples.
The classify sweep runs it after the main pass's final reading, over the same
text: the **choose** shape when the main pass's grounded `rule` names a rule
with a Board or outbound group in `propose`/`auto` (offered every eligible such
rule, in frame order), and the **each** shape, one verdict per candidate, for
the reconciled match set's rules with such a group in `shadow`. The result
rides on the triage record's `rules.confirmation` with
`CONFIRMATION_VERSION`, a digest of the compiled strings. The engine runs an
automatic description task only when the active confirmation, under this
build, chose the same rule; otherwise the task is offered with the reason
`unconfirmed` (the pass chose otherwise, or gave no answer), and the row's
`matched_because.confirmation` names what the pass chose: a rule, `none`, or
`not confirmed` when it decided nothing. A shadow task's would-have row
records `would have proposed: the confirmation pass disagreed` or `not
confirmed`. The version enters the match
digest of exactly the groups the pass decides, and the approval digest of a
description rule with a task or send, so a changed instruction separates the
measurement and lapses those approvals. A description rule's `task` therefore
leaves `shadow` in this phase; its sends stay there until P4b-2.
`Reflexion::learnable`'s comment gains the sentence naming the confirmation
pass as why the triage exemption's premise still holds.

Refusals (`rules.refused`, and the confirmation's own) are recorded with
`RuleRefusal::label` text but not yet shown on the page; the description-rule
test (P4a-2) is where they surface, beside the cost it reports.

Implementation choices, not owner rulings: a failed confirmation call, or an
answer whose quote was refused, demotes exactly as a disagreement does and is
never retried into agreement, but is recorded on both shapes as `not
confirmed` rather than as a judgement, so an outage is not counted in the
measurement promotion is decided on; a
confirmation from another build's frame confirms nothing; the shadow
confirmation of a rule with no gate-failing text (every rule until P7) is the
only shadow case.

**P3 — `#mail/rules`.** The list, the rule page, the ledger with undo and
wrong-match, header-rule testing, *make a rule from this thread*.

**P4 — description rules and fixed sends.** The classifier's `rule` field
with grounding, the confirmation pass for send candidates and the sentence it
adds to `learnable()`'s comment (§4.2), template `reply`, verbatim `forward`,
§5.4's guards and the cap brakes of §6.4, the approval screen, the outbox
exception's sentence in `ARCHITECTURE.md` (§6.3), and `mecha doctor`'s
attention for a rule holding an outbound group in `auto` while `shell` runs
unconfined (§6.1). In `propose` until the owner approves a rule.

**P5 — typed-field sends.** `fields`, the grounded extraction, the rendered
template, and a shared header check for the template's subject on both
providers: `gmail::checked_header` is private to `gmail.rs` today and has no
Graph counterpart, so P5 exports it or adds the equivalent.

**P6 — drafted replies.** `draft_with`, the closed drafting context, the
`shareable` frontmatter key, graduation through the approval screen.

**P7 — `mail_rule_propose`.** Registered where `persona_propose` is
registered, with one difference. Its effect-class entry
(`mecha-cli/tests/effect_class.rs`, `store_write`) is test-enforced: an
unlisted builtin panics there. The incognito allowlist (`serve/incognito.rs`)
is not — `withheld()` is fail-closed, so nothing fails if a tool is never
listed — and `mail_rule_propose` is **deliberately left off it**: unlike
`persona_propose`, which stages *locked* (`ToolCtx::stage_locked`) so an
incognito room's text stays hidden until unlocked, a rule proposal lands in
`proposals/`, which `backlog.rs` surfaces in `/queues`, outside the room. A
rule is not something to draft in a room that leaves no trace.

When P1 lands, `ARCHITECTURE.md` gets a *Mail rules* section for the
invariants. `TRIFECTA.md` gets the rule-driven send as a channel when P4
opens it, with §5's argument, and P5 and P6 extend that entry as their
arguments become true.

---

## 12. What other mail tools do, briefly

None of the products surveyed fits this kind of work (the owner's experience,
and the reason this is built here rather than adopted). What they confirm:

- **Gmail** filters can send a fixed template, and **Outlook** rules can
  reply with one. Fixed text is the only form of automatic reply that is
  widely deployed, which matches §5.1 being the first tier R1 opens.
- **Superhuman** has auto labels, auto archive and auto drafts; drafts are
  left as drafts and never sent on their own. Its own reported figure is that
  60% of auto drafts are sent unedited. The ledger shows the same kind of
  number per rule, for the owner's own mail, as information rather than a
  gate (R6).
- **Shortwave** defines filters by prompt, closest to §4.2.
- **Inbox Zero** has natural-language rules, a test tab and a history page,
  and its docs advise testing on recent mail and reading the history before
  trusting outbound actions — which is the advice, without the mechanism
  (shadow mode, approval bound to a digest, brakes) that would enforce it.

Nothing surveyed has a would-have mode, a per-rule switch from drafting to
sending that the owner throws, RFC 3834 guards the user can see firing, or a separation
between "a model matched this" and "a model wrote this".

Sources: [Inbox Zero security and data](https://docs.getinboxzero.com/essentials/security-and-data),
[Inbox Zero getting started](https://docs.getinboxzero.com/essentials/getting-started),
[Superhuman Mail for Gmail](https://help.superhuman.com/hc/en-us/articles/46183279736461-Superhuman-Mail-for-Gmail),
[Superhuman AI review 2026](https://fast.io/resources/superhuman-ai-review-2026/),
[Shortwave vs Superhuman](https://www.jotform.com/ai/agents/shortwave-vs-superhuman/),
[RFC 3834](https://www.rfc-editor.org/rfc/rfc3834),
[IANA Auto-Submitted keywords](https://www.iana.org/assignments/auto-submitted-keywords/),
[Graph message resource (custom headers)](https://learn.microsoft.com/graph/api/resources/message).

---

## 13. Open questions, for review

One is the owner's:

- **Q0. Automatic forwards, sends and drafted replies need a sender-constrained
   matcher (§5.2)** — a `from` list, or a `from_domain` that is not a public
   mailbox provider or the owner's own institution. The design's default,
   found in review: without it, any
   stranger with a domain and a plausible invoice reaches finance under the
   owner's name, or steers a drafted reply's quotes from the `shareable`
   files. Template replies are exempt. The alternative is to accept that
   residual knowingly and allow `describe`-only matchers in `auto`. The
   default stands unless the owner rules otherwise.

The rest are answered by measurement or by a decision made elsewhere:

1. **Description list-entry cap.** 12 total entries across the active and
   shadow lists is a guess at what the classifier holds without precision
   dropping; a rule in both counts twice (§4.2). Shadow mode measures it;
   the cap moves then.
2. **Mode-dependent frame measurement.** Promoting one group of rule A can
   add or remove a list entry and change the stable frame bytes in which
   every other description rule is matched. Their match digests do not
   change, so their rates pool rows from before and after that frame change.
   Measure whether that pooling changes the interpretation of the rates;
   this is an open measurement question, not a new digest or reset
   mechanism.
3. **Retention.** The ledger grows forever, as `~/.mecha/mail-triage/` does
   (`MAIL-UX-DESIGN.md` §7 Q6). Same answer, whenever that one is given.

---

## 14. What is deliberately not being built

- **A rule that takes a recipient from the mail or from a model**, other
  than `reply`'s authenticated sender. A proposed `to` may be installed and
  used in `propose`, but reaches `auto` only once the owner has written or
  edited it (§9). Even a `ModelClean` `to` needs an actual edit, because
  recognition alone does not defeat a lookalike domain; installation and
  acknowledgement never adopt the address.
- **Auto-promotion, or a history-based promotion gate.** No rule moves up
  the ladder without the owner, no statistic moves it, and no history
  threshold stands between the owner and the switch either (R6). §4.2's
  list-entry cap is a frame-capacity limit checked on every entry addition,
  with explicit refusal or a visible loader hold when over cap, without
  requiring a track record.
- **Model-written rules that go live.** §9: proposals install in `shadow`.
- **Rules in project config.** Global store only.
- **Provider-side filters.** A rule is mecha's, like tags: no Gmail filter
  or Outlook rule is created, so the two providers behave the same and a
  rule's ledger is complete.
- **Reply-all or `Reply-To`, at any mode, and *automatic* replies to
  unauthenticated senders.** A reply to a sender who fails §5.4's
  authentication check is never sent unattended; it is staged in the outbox,
  where the owner may still send it.
- **A second approval surface.** Staged sends from rules go to the outbox,
  labelled with the rule that staged them.
