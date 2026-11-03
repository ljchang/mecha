# Mail rules — design

*2026-11-02. Not built. Written for review before any of it is, so anything
here is still cheap to change. §0 holds the owner's rulings; §13 what is still
open. `MAIL-UX-DESIGN.md` is the surface this extends and is assumed read —
its §8 said "auto-send of anything" and "an autonomy tier" were deliberately
not being built, and R1–R3 below are the owner reversing that, inside the
bounds this document sets.*

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
                               from, to, digest
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
digest of the rule's directory (§6). A mode written into the file would let an
edit — or an installed proposal — grant itself `auto`.

**The effective mode is the latest row for the rule and action group, and the
digest only ever lowers it.** **Every row names one action group** (§5's
table), and resolution reads only that group's rows, latest wins. A
whole-rule change — R7's lapse, `pause-all` — is written as **one row per
group**, each carrying its own value, never as a single row for the rule:
groups routinely sit at different rungs (*archive automatically, propose the
reply*), and one value for all of them would either raise a lower group or
drop an untouched one further than asked. A later row raises a group again
only by the owner's approval. Owner approvals, owner step-downs and §6.4's automatic brakes are
all rows in the same file, and the most recent one wins; if its digest no
longer matches the rule's directory, the mode reads as `propose` (or lower, if
the row says lower). Never "the current digest matches *some* `auto`
approval": that reading would let the next run undo a brake, which changes no
file, and would re-arm a rule whose edit was reverted to an earlier approved
state, when the lapse was the point. That is why a detected digest change is
itself written as a row (§6.2), not only inferred at read time. Permission
moves up only by a new owner row.

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

`to` is the only place an outbound address appears in a rule, and it is
owner-written. No action takes a recipient from the mail or from a model
except `reply`, whose recipient is the thread's sender under the guards of §5.4.

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

They run **before classification**, and a rule whose actions are all
mailbox-only can mark the thread handled so the classifier never spends a
call on it — **but only when no description rule is enabled at all, and no
matching header rule has an outbound action, a `digest` or a `task`.** The
last two need a verdict too: §7 renders a flagged thread with the
classifier's one-line and an automatic task with the verdict's deadline, and
a skipped thread has neither. Any enabled description rule
needs the classifier to see the thread, whatever its actions — a
`digest`- or `task`-only description rule is silenced by a skip exactly as a
reply rule is, since §4.3 applies record actions from every match. A
skipped classification is a description rule never evaluated, so a
`priority = 90` header rule that archives a list must not silence a
`priority = 5` description rule that would have replied — **and neither may
a `priority = 5` archive silence a `priority = 90` reply**, because §4.3 gives
outbound actions to the first rule *that has any*, so a higher-priority
mailbox-only rule does not take the thread's outbound slot. So the condition is
not about rank. **A header rule's inbox-removing actions (`archive`,
`trash`, `spam`) wait for the classify pass whenever either holds:**

- any description rule is enabled, for any action, not only outbound ones; or
- any matching header rule — this one included, since one rule holding both
  `forward` and `archive` is §5.2's expected shape — has an outbound action,
  a `digest` or a `task` (the last two because they need the verdict, §7).

**Enabled means any mode but `off`, `shadow` included**: a shadow rule's
would-be actions have to see the thread too, or an archive would silently
truncate the would-have rows the owner is reading to decide whether to
approve it, and shadow would under-measure in exactly the direction that
looks safe.

Otherwise the inbox-removing actions run on the timer. Either way, `read`,
`unread` and `tag` run on the timer. **`digest` and `task` never
run on the timer**: they read the verdict, so they run at the end of the
classify pass with everything else that does, and a thread carrying one is
always classified. The header case
matters as much as the description one: a `priority = 10` rule that archives
a list and a `priority = 50` rule that forwards the same sender's mail both
match at minute three, the forward waits for `classify` (§4.4), and an
immediate archive would remove the thread before the forward is decided.
They have to: `classify` enumerates threads from `mail_recent`, which is
inbox-only (`in:inbox` on Gmail, `recent` on Outlook), so a thread archived at
minute three would never be classified and the reply would never be
considered, with nothing on the ledger to say why. **Deferring alone is not
enough, though**: `classify` reads `mail_recent`'s newest-N window, threads
leave that window on a busy day, and a deferred thread that left it would
lose its archive and every rule's evaluation in silence. So a deferral is
**recorded**: the header pass writes the thread's id to a deferred queue in
the rule store, and `classify` reads that queue beside `mail_recent` — the
same need `due_outside_window` answers for the triage store's retries (work
owed on threads the window no longer returns), here met with a queue because
there is no earlier record to walk. A queued thread is evaluated and its
deferred actions run whether or not it is still in the window. **The queue
exists so `classify` re-evaluates the thread, and an entry leaves once it
has**: each deferred action is then executed (`done`), offered to the owner
(`offered` — a `propose`-mode action now waiting in the mail desk, which
carries it from there and counts in `backlog.rs`), staged, skipped, or
failed. A thread that cannot be read is a `failed: thread unreadable` row,
never a silent drop. An entry older than seven days settles as `expired`
with a notice, so the queue cannot grow without bound behind a broken
classifier — and `expired` is not a failure: it does not count toward
§6.4's consecutive-failure brake, which owner inattention must never be able
to trip. Whether any
description rule is enabled is known statically from the enabled set, and
which other header rules match a given thread is known in the same timer
pass, before anything executes. P1 builds this, though no description rule exists until P4,
so the bug cannot land dormant. A GitHub
notification should cost a string comparison, not a model call, whenever
"the first rule by priority owns the thread" allows it.

### 4.2 Description matchers — the classifier, one more field

**A rule is a description rule if and only if it declares `describe`.** Its
header keys, if any, are then an *additional precondition* checked in the
same classify pass, against the same newest inbound message, **before** the
classifier's list is built: only rules whose header preconditions pass for
this message are offered to it, so a rule that could not apply never takes
the single `rule` slot from one that could. Never a separate header rule — and **none of its actions run on the `--headers`
timer**. This is the common shape, not an edge: §5.2 requires a sender key
for any automatic forward, send or drafted reply, so §3's `receipts-to-finance`
example (`from_domain` + `describe`) is exactly it. Read as a header rule, its
`digest` would fire for every message from the vendor, whether or not it was
a receipt. For §5.5, such a rule counts as a description rule,
so its `trash` and `spam` propose: a pinned sender set narrows who can
trigger it but not what the description generalises to, and an owner who
wants an automatic trash on a sender alone writes a header rule for it.

`describe` is a sentence the owner writes. The classifier — already one
isolated, tool-less call per thread — is given the enabled rules' descriptions
as numbered lists, **in `priority` then name order** so the frame's bytes are
stable from thread to thread (the cached-prefix rule), and returns two more
fields:

```
rule: <one of the active rules' names> | none
rule_quote: words from the message that show it
shadow_rules: [ { name: <a shadow rule's name>, quote: <words that show it> } ]
```

**Shadow rules have their own list and their own field**, so they never
compete for `rule` — and **that field is a list**, so they do not compete
with each other either. Every rule installs in `shadow`, so several at once
is the normal state, and a single slot would leave the rule the classifier
did not name with no would-have row at all: its match rate would read low,
in the direction that looks safe, on the number the owner approves against.
Shadow rules act on nothing, so a list needs no overlap semantics, only rows.
Each named shadow rule's quote is grounded like `rule_quote`. Were they in one list, a rule freshly installed in
`shadow` could be named instead of a rule the owner approved for `auto`, and
the approved rule would do nothing and leave no row — against §6.1's promise
that `shadow` acts on nothing. A shadow rule's would-have row therefore says
what it would do *beside* the active rules, not in place of one; how two
rules interact once both are active is something the owner sees after
promotion, in `propose`, before either goes back to `auto`.

**A shadow rule with an outbound action is measured by both passes**, so its
would-have rows are the match rate `auto` will actually have: a `shadow_rules`
candidate with an outbound action gets the confirmation pass too, exactly as
a `rule` candidate does, and a disagreement is recorded as `would have:
propose`. Measuring shadow with the main pass alone would over-state the
rate the owner approves against. It costs one call per shadow send
candidate, which is a small fraction of mail and only while a rule is being
evaluated.
The same discipline as `request_type` and `deadline_quote`:

- **Recognition, never invention.** The value is checked against the live
  list; an unknown name is `none`. Constrained decoding gets the enum.
- **The quote is grounded** — the words must be in the message, or the match
  is dropped, recorded with why (the `ground_deadline` pattern). The quote is
  what the ledger shows a human as *matched because*; it is prose, the
  sender's, and never reaches a privileged run.
- **The descriptions are the owner's words** and enter the classifier's frame
  as instructions, exactly as the tag vocabulary does. A rule installed from a
  proposal (§9) carries its `Origin` into that frame's provenance.
- **Escalation.** The classifier reads a snippet first. A match on a rule
  whose actions include any send forces the full-body escalation before the
  verdict is final, so nothing is sent on the strength of a snippet.

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

So the main pass's `rule` field is **a candidate, never a decision, for any
rule with an outbound action or a `task`** — the Board group too, because a
task reaches a task run's prompt (§7) and an unattended run with tools is the
consumer `learnable()`'s comment names as well as a send is. The harness-written
name closes what a task *says*; the confirmation pass closes *which threads
start one*. Such a candidate gets a second quarantined
call, the **confirmation pass**: no tools, no history, and a frame made only
of the owner's rule description and a fixed instruction compiled into the
binary — **no learned rules and no few-shot examples**. It answers one
question about one rule, `matches: yes | no` with its own grounded quote. A
send proceeds only when both passes agree; disagreement goes to `propose` with
both answers on the ledger row.

That keeps the exemption's premise true rather than re-arguing it: untrusted
triage rules still reach only a pass whose output cannot send on its own. When
this is built, `learnable()`'s comment gains one sentence naming the
confirmation pass as the reason the premise still holds, so the next reader
inherits a true argument. The cost is one extra call per *send or task
candidate*, not per thread — a small fraction of mail. Rules whose actions
are only in the **Mailbox and Private record groups** (§5's table: `archive`,
`read`, `unread`, `trash`, `spam`, `tag`, `digest` — not `task`) keep
matching in the main pass alone, learned rules and all: their worst case is a
reversible archive shown in the digest (§5.5, §7).

**Cost.** Each description adds its tokens to every classification. The
working assumption is a cap of 12 enabled description rules, revisited once
shadow mode (§6) has measured precision with a real list. A per-rule yes/no
pass would be more accurate and costs one call per rule per thread; it is the
fallback if the single field measures badly, not the start.

### 4.3 Order and overlap

Rules evaluate in `priority` order, header rules before description rules. A
thread may match several header rules, but **at most one description rule**:
§4.2's `rule` field is single-valued, so when two descriptions both fit, the
classifier's choice decides between them and `priority` does not. Priority
orders header rules among themselves and header rules against the one
description match. Two consequences follow and are accepted: the owner who
wants two description rules' actions on one kind of mail writes one rule
with both, and a description rule the classifier did not name is never
reported — the ledger cannot show it was considered, so there is no
`superseded` row for it. If the per-rule yes/no fallback of §4.2 is ever
taken, multiple description matches become possible and this paragraph is
rewritten with it. Mailbox actions and `digest`/`task` from every match
apply. **Outbound actions come from at most one rule per thread** — the
first by priority *that has any outbound action*, so a higher-priority
mailbox-only rule never takes the slot — because two rules answering the same email is never what
anyone meant, and the ledger records the other rules' outbound actions as
`skipped: superseded by <rule>`. Within that one rule every outbound action
runs: the receipts case is a `forward` of the invoice *and* a typed `send`,
both declared by the same rule.

### 4.4 When it runs

- Header rules' **mailbox actions and `tag`**: `mecha mail rule apply
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
below), `done`, `sent`, `staged`, `offered` and `held` close it. `would have`, `skipped` and
`expired` do not — nothing ran, and a thread readable again later may be
evaluated again. `failed` closes it for outbound actions, which are never
retried (a failure after a provider accepted a send cannot be told from one
before), and for `task`, where a retry after a create that succeeded but went
unrecorded would put a second task on the board; it leaves the key open for
the Mailbox and Private record groups, which are idempotent and safe to retry
after a transient error.

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
`rejected`, `undone`. None of them reopens the key. Under "latest row wins",
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
between. A step down to `shadow` (a brake or `pause-all`) does end the
stretch, but mail arriving while paused is not lost: the rule is in
`shadow`, so it writes would-have rows for it, and resuming offers *apply to
these N matches from while I was paused*. **A message found ineligible gets
a row** — `skipped: before the rule was active` — so nothing is passed over
without a trace. Acting on past matches is an explicit owner action on the
rule page — *apply to these N earlier matches* — and runs in `propose`
whatever the rule's mode, so the backlog is reviewed, not sent.

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
before this key is relied on — and the once-per-thread check below is a lookup
on `(rule, account, thread id)` over the outbound rows, a key of its own
beside the per-message one, and it names its closing outcomes as the
per-message key does: **`pending`, `sent`, `staged`, `held` and an outbound
`failed` close the thread** — a send that reached the provider, one queued
and still owed, or one whose fate cannot be told — and **`would have`,
`skipped` and `expired` leave it open**. Otherwise a rule promoted out of
`shadow` would find its own would-have rows and never send on the threads it
was approved against, and a rule that lost the outbound slot to another on
one message could never take it on the next. Outbound actions are additionally limited to **one
occasion per rule per thread**: all of a rule's outbound actions fire together
on the first message that triggers them — the receipts rule's `forward` *and*
typed `send` both go (§4.3) — and a later message in a thread the rule has
already sent from is `skipped: thread already handled`, never a second forward
under a new message id. P0 gives the thread read the newest inbound message's provider id
and headers for this. The ledger row is written
`pending` first and settled after, so a crash between the two is visible as a
pending row, never silently retried as a send.

---

## 5. Actions

Grouped by what can go wrong, because that decides what may run unattended.

| Group | `do =` | Reaches | May run in `auto`? |
|---|---|---|---|
| Mailbox | `archive`, `read`, `unread`, `trash`, `spam` | the owner's mailbox | yes — with the §5.5 limits on `trash`/`spam` |
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
aligned with its `From`**, the same check §5.4 puts on replies, and anything
that fails goes to `propose`. That stops the forged-sender invoice — and
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
   `money` and `number` (parsed, then re-rendered by the harness — never the
   extracted string), `date` (parsed to `YYYY-MM-DD` and re-rendered), and
   `email` (a single address that parses, else the field fails). A type added
   later states its rule too.
2. **Every value is grounded**: its source words must appear in the message
   (the `deadline_quote` rule). An ungrounded field fails the send, which
   falls back to `propose` with the reason on the ledger row.
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
| Sender-constrained matcher for `auto` | drafted only (§5.2) | yes (§5.2) |
| `From` only, never `Reply-To` or reply-all | yes | n/a — the recipient is the owner's `to` |
| Once per sender per rule per 7 days | yes | **no** — every new receipt thread from a listed vendor is forwarded; §4.4's once-per-thread limit still applies, so a vendor reply into an existing order thread is `skipped: thread already handled`, visibly |
| Per-rule and global daily caps | yes | yes |
| Not into a conversation the owner has joined | yes | no |
| Never on a message `From` one of the owner's own addresses | yes | yes |
| Never on one `From` a `to` address; RFC 3834 on for senders at a `to` domain | n/a | yes |
| `block_sends_after_private` (§6.3) | yes | yes |

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
harness, all are recorded on the ledger row when they fire (`skipped: <guard>`),
and none is configurable off:

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
  submitted one with it is a second claim and proposes, including when
  Graph's truncation has dropped EOP's instance and the sender's is topmost. P0's real-header check covers Gmail's ordering as
  well as EOP's shape, with the same fallback: if it is not reliable, that
  provider's sends stay in `propose`. `Authentication-Results` gets its
  own field on `Email` rather than riding the existing loop.
  One consequence worth saying in the UI: mail that reaches an account by
  forwarding from another address routinely fails alignment, so a rule on
  such an account may never leave `propose` for its sends. That is the guard
  working, not a fault, and the rule page says which guard is holding it.
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
So **`trash` and `spam` run in `auto` only from header matchers**; on a
description match they propose. And every automatic mailbox action lands in
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
(`mecha mail rule mode <name> <mode>`) or the web page — and never a tool.
This is `/review now|later|auto`'s rule: a policy that decides what leaves the
machine must not be decidable by anything sharing a context window with
third-party text.

**"Never a tool" is not the whole boundary**, because a model with `shell`
needs no tool to type `mecha mail rule mode <name> auto`. So **raising** a
mode refuses any process that descends from a registered run — the shell
registry and run markers `closure.rs` already reads to decide who may close a
task (`ShellReading`, `POSTURE_ENV` as advisory) — and records the raise with
`Actor::Owner` only. This deliberately does **not** inherit the outbox's
ruling that a release from an unknown actor is stamped `unknown` but not
refused: one release sends one reviewed draft, while a raise is a standing
permission to send unattended, and a run with the owner present is still a
context window with mail in it. Lowering is allowed from anywhere, a run
included, because stepping down never needs approval (§6.4).

**That gate is advisory, and the structural answer is the sandbox**, exactly
as in `closure.rs`. `closure::decide`'s own documentation names the way past
it — a command detached from the run with its environment cleared — and an
unconfined `shell` does not need the CLI at all: it can append a row to
`approvals.jsonl` directly. That matters more here than for task closure,
because §3 moved the mode out of `rule.toml` precisely so nothing could
self-grant `auto`. With `shell` confined (§Security model, the sandbox), the
store is outside anything a run can write and the gate is real. Without it,
the residual is stated rather than hidden: `mecha doctor` reports any rule
holding an outbound group in `auto` while `shell` runs unconfined, as
attention, with the sandbox as the remedy.

A rule may be `auto` for some actions and not others: `mode` can be set per
action group (§5's rows), and every mode row names one group (§3) — there
is no rule-wide mode for a group to default to. "Archive these
automatically, but propose the reply" is the common case.

### 6.2 Approval binds to what was shown (R1)

Entering `auto` for any send group shows the owner, on one screen: the
matcher, the exact template or covering note, the recipient rule, the guards,
and the shadow-mode rows so far. The approval written to `approvals.jsonl`
carries **a digest of the rule's directory** (and of the named skill's, for
§6.3, and of `aliases.toml` and `limits.toml` for any rule with an outbound
action, since §5.4's loop guard and caps rest on them — removing an alias or
raising the global cap must lapse the approvals that counted on them). That
includes tightening edits, and one global-cap edit lapses every outbound
rule at once; the screen that edits `limits.toml` and `aliases.toml` says so
before saving, with the count of approvals it will lapse. The
caps live in the rule store, never on `Config`, so they are not layered
config and not the two-edit `Config`/`ConfigLayer` trap. On every run the digest is recomputed; **if anything changed, the rule
drops to `propose` or stays lower, group by group** — one lapse row per
action group, each recording the lower of *that group's* current effective
mode and `propose`, so a lapse never raises anything: a group in `shadow`
stays in `shadow`, a group under §6.4's failure brake stays braked, and a
group in `auto` drops to `propose`. They are recorded as **lapse rows appended to
`approvals.jsonl`**, as well as a ledger row (`approval lapsed: files
changed`) for the page, and a notice. The approvals row is what makes it
stick: with only a ledger row, reverting the edit would make the latest
approval's digest match again and re-arm `auto` with no owner act, which §3
rules out. With the lapse as the latest row, a revert is inert. Re-approving is one action that shows the diff.

That includes the owner's own edits, and every action group, mailbox-only
rules too (R7). It is the simpler rule, the
`hud_propose` precedent (an accept names the digest the owner was shown), and
the cost is one click after an edit.

**What the digest covers is everything that decides a send, and it says so
where it does not cover a match.** For a header rule (one with no
`describe`, §4.2), `rules/<name>/` is the whole matcher. For a description rule *with an outbound action or a `task`*, the
decision is the confirmation pass of §4.2, whose inputs are the rule's
description and an instruction compiled into the binary — so the digest also
carries the version of that instruction **and of the §5.3 extraction pass's**,
the other compiled frame a send depends on, and a build that changes either
lapses every approval that rests on it. What the digest does **not** cover is the main pass's learned
triage rules, which change by design every time the owner corrects a verdict
(§10). For send rules that is harmless, because a learned rule can only
nominate a candidate the confirmation pass then judges without it. For a
description rule whose actions are only in the Mailbox and Private record
groups (not Board, which gets the confirmation pass, §4.2) it means the
matcher can widen after approval: accepted, because those actions are reversible,
appear in the digest, and exclude `trash` and `spam` (§5.5), and stated here
so nobody reads "approval binds to what was shown" as covering it.

The feature itself (`mail_rules` in the features registry) is **off by
default**, so on a new install nothing in this document runs at all.

### 6.3 Graduating a drafted reply (R2)

`draft_with = "<skill>"` runs a drafting agent with the skill loaded, and its
draft stages in the outbox — `propose`, always, until graduated. **Graduation
is the owner enabling it, and nothing else** (R6): no count of past drafts, no
acceptance rate and no waiting period gates the switch, and nothing ever flips
it on the owner's behalf. The approval screen of §6.2 is the whole ceremony.

The one precondition is structural, not historical. It binds **`auto`
only**: in `propose`, a drafting run may use whatever tools its skill
declares, as `mecha mail reply` does today, because the owner reads every
draft in the outbox before it goes and that review is the control. The leak
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

A rule steps itself down, with a notice, when:

- **Any automatic send is marked wrong** by the owner → `propose`. Zero
  tolerance for sends.
- **The owner undoes, or marks as a wrong match, more than 20% of its
  automatic mailbox actions** over at least 10 → `propose`. A wrong match the
  owner does not undo still counts: the archive may be harmless while the
  matcher is wrong.
- **Three consecutive action failures** → `shadow`. *Action* failures only —
  the rule's own action refused or erroring. A failure of the environment
  (`failed: thread unreadable`, a provider unreachable, a token needing
  re-auth) does not count, on `expired`'s reasoning: an outage would
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
  say `held: global cap`, and the owner gets one notice. Mailbox and record
  actions are unaffected.

`mecha mail rule pause-all` (and a button) drops every rule to `shadow` at
once — one row per group, each recording the lower of its current mode and
`shadow`, so a group the owner turned `off` stays off. Stepping down never needs approval; stepping up always does.

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
taken from the message**: the rule's own `description` (the owner's words),
the deadline if the verdict carried one, and the thread pointer in
`context`, so what reaches a task run is typed rather than quoted — the
reviewable-object rule applied one surface further. A proposed `task` offered
in the digest keeps today's naming, because the owner's tap is the review.
The approval screen still recommends `propose`, since a board filled by
whoever can send you mail is a board you stop trusting.

---

## 8. The web surface — `#mail/rules`

Inside Mail rather than Settings: it is watched daily, and the rows link into
the mail desk's reader. Every mutation is a `mecha mail rule …` CLI child, the
`/triggers` and outbox pattern, so the page can do nothing the CLI cannot.

**The list.** One row per rule: name, mode (per group, as chips), fires in the
last 7 days, last fired, pending in the outbox, failures, the owner's undo and
edit rate, and a pause toggle. A rule that stepped itself down shows why.

**A rule.**

- The definition, editable in a form over `rule.toml` (`TomlForm` exists) and
  the template files shown as rendered text — what the recipient will read.
- **The ledger**: one row per action — when, which message (sender and
  subject, opening in the reader), matched because (header key, or the
  grounded `rule_quote`), the action, the mode it ran in, and the outcome:
  `pending` (written before an action executes, §4.4), `done`, `would have`
  (shadow), `offered` (a proposed mailbox action waiting in the mail desk),
  `staged`, `held: rule cap` and `held: global cap` (§6.4), `sent`,
  `released unchanged`,
  `edited`, `rejected`, `undone`, `expired` (§4.1), `failed: <why>`,
  `skipped: <guard>`.
  `skipped` rows are shown, not hidden — a guard visibly firing is how the
  owner learns to trust the guards.
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
opposite ones. In `approvals.jsonl` an unknown mode reads as **`shadow`**: it
never acts, so a newer build's rung cannot raise anything in an older one.
In `ledger.jsonl` an unknown outcome **closes the key**, as an unknown
`failed` does for a send: an outcome nobody can read is never taken as
permission to run the action again. Both are shown as `unknown` on the
page.

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
  write: the rule's name by `persona::validate_name` (lowercase, digits, `-`,
  `_`, a leading alphanumeric, length-capped), and every file it names as a
  **stem** passing the same check plus an **extension matched exactly against
  a closed list** (`md`). Since the stem can hold no `.` and the extension is
  compared whole, a name has exactly one dot and no separator, so neither
  `..` nor a path can be spelled — `reply.md` passes, `../reply.md` and
  `reply.md/x` cannot. Containment in `proposals/<id>/` is still proved
  before the file is created, as the second of two checks rather than the
  only one. The
  proposal id is minted by the store, never taken from the model.
- The proposal is checked by the same loader an installed rule uses, so a
  proposal that previews clean installs clean.
- The owner sees the rendered rule — the template as the recipient would read
  it — and installs it. **Installation is always into `shadow`**, whatever the
  proposal says, and an armed proposal is labelled as one. The accept names
  the digest the owner was shown.
- **The recorded `Origin` gates what can send.** The confirmation pass
  (§4.2) is sound only because nothing a stranger wrote can have shaped its
  frame — the owner's words, or a model's written in a conversation that had
  read nothing untrusted — and a proposal
  drafted after reading mail installs a description whose origin is
  `Origin::of_proposal`'s untrusted answer. So **no outbound action group —
  and not the Board group either, whose `task` name reaches a task run's
  prompt (§7) — may enter `auto` while any part of the rule is
  model-proposed text from an armed conversation: every key of `rule.toml`
  and every file it names.** The gate is the whole rule rather than a list of
  fields, because a list stops being complete without anyone noticing: in
  review it grew from the description to templates, the sender keys (§5.2's
  control on *who can make a send leave*, which a header rule has no
  description to hold back), `fields` (which shape the extraction frame, a
  send decision), `note` (words a forward sends) and `draft_with` (which
  chooses the skill whose `shareable` files are given whole), and a key
  added later would have been the next omission. Every one of those, and
  every future key, is covered by construction. And `to` is held
  to a stricter rule than words: **an outbound group may enter `auto` only
  when its `to` is `Owner` — typed or edited by the owner — never a proposed
  address left as proposed, even from a clean conversation.** `to` is the one
  field that bounds where every automatic forward and send lands, §3 calls it
  owner-written, and a proposal writes the whole `rule.toml`; the approval
  screen shows it first and opens it for editing. The rescue is `Reflexion::provenance`'s existing move: the
  installed record keeps a digest of each proposed text, and once the owner
  has changed one, it is the owner's words. The rule store records each
  text's origin in `imagelib::Origin`'s terms, the enum `of_proposal`
  answers in: `Owner` for anything the owner wrote or changed, `ModelClean`
  or `ModelUntrusted` for proposed text left as proposed. The send gate
  admits `Owner` and `ModelClean`, so in practice it holds exactly the
  `ModelUntrusted` texts. The approval
  screen says which text is holding the rule back and opens it for editing.
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

**P1 — the store, header rules, mailbox actions.** `rule.toml`, approvals,
the ledger, `off`/`shadow`/`propose`/`auto` for the Mailbox, Private record
and Board groups — `task` on header rules, with its harness-written name
(§7); the confirmation pass that description-matched tasks also need lands
with description rules in P4 — (including a `tag` action, which writes mecha's own tag onto the
triage record and has no provider verb, since tags are never Gmail labels or
Graph categories), the five-minute timer, `mecha mail rule add | list | show | mode |
test | ledger | undo | pause-all` — P1 is where rules first act in `auto`, so
the panic button ships with them, and so do §6.4's brakes over P1's actions
(the undo-rate brake and the consecutive-failure brake) — the feature
registry entry (off by
default) — through
`ARCHITECTURE.md` §Features → *Adding a feature*, whose dozen places include
the web pane's `PANE_FEATURE` entry P3 will need (#654 missed exactly that) —
and
`mecha doctor` reporting an unreadable store or a lapsed approval. The store
also registers with `backlog.rs` (and `CLAUDE.md`'s module-map line for it,
"across five stores", gains one), so proposals awaiting install and lapsed
approvals count in `mecha review` and `/queues` — what is *waiting*, beside
the doctor's what is *wrong*; rule sends staged in the outbox already count
there through `OutboxStore`. **Covers
GitHub notifications and junk with no model involved.** Until P2 ships the
digest, the trace §5.5 promises is the ledger and `undo` — pulled rather than
pushed — which is acceptable for P1's actions because all of them are
recoverable and none reaches anyone.

**P2 — the digest.** `mecha mail digest` and `digest ack`. Then the briefing
swap, which is **an edit on the owner's machine, not in this repository**:
the `morning` trigger lives only under `~/.mecha/`. It is the `update`
skill's territory and carries a trap `HANDOFF.md` already records for
`--aged`: the installed binary must have `mail digest` before the `notify`
line names it, or the briefing pastes a usage error into itself. Install
first, then swap, then confirm by running the `notify` line, not by reading
it.

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
[IANA Auto-Submitted keywords](https://iana.org/assignments/auto-submitted-keywords),
[Graph message resource (custom headers)](https://learn.microsoft.com/graph/api/resources/message).

---

## 13. Open questions, for review

One is the owner's:

0. **Automatic forwards, sends and drafted replies need a sender-constrained
   matcher (§5.2)** — a `from` list, or a `from_domain` that is not a public
   mailbox provider or the owner's own institution. The design's default,
   found in review: without it, any
   stranger with a domain and a plausible invoice reaches finance under the
   owner's name, or steers a drafted reply's quotes from the `shareable`
   files. Template replies are exempt. The alternative is to accept that
   residual knowingly and allow `describe`-only matchers in `auto`. The
   default stands unless the owner rules otherwise.

The rest are answered by measurement or by a decision made elsewhere:

1. **Description rule cap.** 12 is a guess at what the classifier holds
   without precision dropping. Shadow mode measures it; the cap moves then.
2. **Retention.** The ledger grows forever, as `~/.mecha/mail-triage/` does
   (`MAIL-UX-DESIGN.md` §7 Q6). Same answer, whenever that one is given.

---

## 14. What is deliberately not being built

- **A rule that takes a recipient from the mail or from a model**, other than
  `reply`'s authenticated sender. A proposed `to` may be installed and used in
  `propose`, but reaches `auto` only once the owner has typed or edited it
  (§9).
- **Auto-promotion, or a promotion gate.** No rule moves up the ladder
  without the owner, no statistic moves it, and no statistic stands between
  the owner and the switch either (R6).
- **Model-written rules that go live.** §9: proposals install in `shadow`.
- **Rules in project config.** Global store only.
- **Provider-side filters.** A rule is mecha's, like tags: no Gmail filter or
  Outlook rule is created, so the two providers behave the same and a rule's
  ledger is complete.
- **Reply-all, `Reply-To`, or replies to unauthenticated senders**, at any
  mode.
- **A second approval surface.** Staged sends from rules go to the outbox,
  labelled with the rule that staged them.
