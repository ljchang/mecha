# Mail rules — design

*2026-11-02. Not built. Written for review before any of it is, so anything
here is still cheap to change. §0 holds the owner's rulings; §13 what is still
open. `MAIL-UX-DESIGN.md` is the surface this extends and is assumed read —
its §8 said "auto-send of anything" and "an autonomy tier" were deliberately
not being built, and R1–R3 below are the owner reversing that, inside the
bounds this document sets.*

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
  approvals.jsonl              every mode change: who, when, from, to, digest
  ledger.jsonl                 one row per rule × message × action (§8)
  proposals/<id>/              drafts from a conversation, never live (§9)
  .lock
```

The trigger store's rules, unchanged: **global only, never layered config** (a
rule is an unattended actor over private data with a way to send, and a cloned
repository must not bring one); one TOML per rule so `$EDITOR` and `git diff`
work; temp-sibling-and-rename for every rewrite; an advisory flock for
read-modify-write; the ledger append-only.

**The mode is not in `rule.toml`.** It lives in `approvals.jsonl`, keyed to a
digest of the rule's directory (§6). A mode written into the file would let an
edit — or an installed proposal — grant itself `auto`.

**The effective mode is the latest row for the rule, and the digest only ever
lowers it.** Owner approvals, owner step-downs and §6.4's automatic brakes are
all rows in the same file, and the most recent one wins; if its digest no
longer matches the rule's directory, the mode reads as `propose` (or lower, if
the row says lower). Never "the current digest matches *some* `auto`
approval": that reading would let the next run undo a brake, which changes no
file, and would re-arm a rule whose edit was reverted to an earlier approved
state, when the lapse was the point. Permission moves up only by a new owner
row.

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

They run **before classification**, and a rule whose actions are all
mailbox-only can mark the thread handled so the classifier never spends a
call on it. A GitHub notification should cost a string comparison, not a
model call.

### 4.2 Description matchers — the classifier, one more field

`describe` is a sentence the owner writes. The classifier — already one
isolated, tool-less call per thread — is given the enabled rules' descriptions
as a numbered list and returns one more field:

```
rule: <one of the listed rule names> | none
rule_quote: words from the message that show it
```

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
rule with an outbound action.** Such a candidate gets a second quarantined
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
inherits a true argument. The cost is one extra call per *send candidate*, not
per thread — a small fraction of mail. Rules with only mailbox and record
actions keep matching in the main pass alone, learned rules and all: their
worst case is a reversible archive that the digest shows (§5.5, §7).

**Cost.** Each description adds its tokens to every classification. The
working assumption is a cap of 12 enabled description rules, revisited once
shadow mode (§6) has measured precision with a real list. A per-rule yes/no
pass would be more accurate and costs one call per rule per thread; it is the
fallback if the single field measures badly, not the start.

### 4.3 Order and overlap

Rules evaluate in `priority` order, header rules before description rules. A
thread may match several. Mailbox actions and `digest`/`task` from every
match apply. **Outbound actions come from at most one rule per thread** — the
first by priority — because two rules answering the same email is never what
anyone meant, and the ledger records the other rules' outbound actions as
`skipped: superseded by <rule>`. Within that one rule every outbound action
runs: the receipts case is a `forward` of the invoice *and* a typed `send`,
both declared by the same rule.

### 4.4 When it runs

- Header rules' **mailbox and record actions**: `mecha mail rule apply
  --headers`, on its own five-minute timer. No model, so no `model-idle.sh`
  gate.
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
honestly a new action that has not run. The message half is the
provider's own immutable id (`Email::provider_id`, what `get_message` takes),
**never the RFC 5322 `Message-ID`**, which is sender-written and optional
here: a reused one would mark another message's actions as already done, and
every message without one would collide on a single key. The `Message-ID`
stays on the row as a display field only; a re-classification, a
`--force` sweep or a crashed run never sends twice. The ledger row is written
`pending` first and settled after, so a crash between the two is visible as a
pending row, never silently retried as a send.

---

## 5. Actions

Grouped by what can go wrong, because that decides what may run unattended.

| Group | `do =` | Reaches | May run in `auto`? |
|---|---|---|---|
| Mailbox | `archive`, `read`, `unread`, `tag`, `trash`, `spam` | the owner's mailbox | yes — with the §5.5 limits on `trash`/`spam` |
| Private record | `digest`, `task` | the owner | yes |
| Fixed words | `reply` with `template` | the sender | after owner approval (R1) |
| Verbatim forward | `forward` with `to` | an owner-fixed recipient | after owner approval (R3) |
| Typed fields | `send` with `to`, `template`, `fields` | an owner-fixed recipient | after owner approval (R3) |
| Drafted prose | `reply` / `send` with `draft_with` | sender or owner-fixed recipient | outbox; may graduate (R2, §6.3) |

### 5.1 `reply` with a template — the vacation-responder shape

The owner's words, sent as written. Slots are a **closed set filled by the
harness**, never by a model: `{sender_first_name}` (from the `From` display
name, length-bounded, stripped of anything but letters, spaces, hyphens and
apostrophes), `{subject}` (as `Re:`), `{today}`.

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
`from_domain` key (an `account` key counts only where the owner marks that
account as reached by known correspondents alone). A rule whose matcher is
`describe` alone may still *propose* these sends. R3's "to a specific person"
becomes "from a known set, to a specific person". The residual the owner
accepts by enabling such a rule is then the narrow one: a listed sender's
real account sending something false, with the ledger and the digest showing
every forward that went.

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

1. A quarantined extraction pass — no tools, no history, the classifier's
   frame — fills the rule's declared `fields` from the message. Types are a
   closed set: `text` (one line, bounded), `money`, `date`, `email`, `number`.
2. **Every value is grounded**: its source words must appear in the message
   (the `deadline_quote` rule). An ungrounded field fails the send, which
   falls back to `propose` with the reason on the ledger row.
3. The harness renders the owner's template with the values. The model's
   output is data in typed slots, never sentences — the reviewable-object and
   typed-extraction rules from `CLAUDE.md`'s recurring shapes, applied to a
   send.

Leakage: none beyond the triggering message, to a recipient the owner fixed.
Integrity: as §5.2, with the same DMARC condition for `auto`.

### 5.4 Guards on every automatic reply

A reply goes to an address the sender controls, so the guards keep mecha from
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
  unparseable means `propose`, never `pass`**. Graph's
  `internetMessageHeaders` is a truncated collection, so "absent" is a real
  case on Outlook, and each provider gets a test that a missing header
  proposes. The `List-Unsubscribe` reader in `graph_mail.rs` is first-match,
  which is right there (any instance is a bulk signal, so sloppy is safe) and
  the wrong shape to copy here.
  One consequence worth saying in the UI: mail that reaches an account by
  forwarding from another address routinely fails alignment, so a rule on
  such an account may never leave `propose` for its sends. That is the guard
  working, not a fault, and the rule page says which guard is holding it.
- **`From`, never `Reply-To`, and never reply-all.** `Reply-To` is
  sender-chosen and is the other reflection route. An automatic reply answers
  one authenticated person.
- **Once per sender per rule per 7 days** (RFC 3834's guidance), and a daily
  cap per rule (default 20) and across all rules (default 50). Hitting a cap
  pauses the rule (§6.4) rather than queueing.
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

A rule may be `auto` for some actions and not others: `mode` can be set per
action group (§5's rows), defaulting to the rule's. "Archive these
automatically, but propose the reply" is the common case.

### 6.2 Approval binds to what was shown (R1)

Entering `auto` for any send group shows the owner, on one screen: the
matcher, the exact template or covering note, the recipient rule, the guards,
and the shadow-mode rows so far. The approval written to `approvals.jsonl`
carries **a digest of the rule's directory** (and of the named skill's, for
§6.3). On every run the digest is recomputed; **if anything changed, the rule
drops to `propose`** with a ledger row (`approval lapsed: files changed`) and a
notice. Re-approving is one action that shows the diff.

That includes the owner's own edits, and every action group, mailbox-only
rules too (R7). It is the simpler rule, the
`hud_propose` precedent (an accept names the digest the owner was shown), and
the cost is one click after an edit.

**What the digest covers is everything that decides a send, and it says so
where it does not cover a match.** For a header rule, `rules/<name>/` is the
whole matcher. For a description rule *with an outbound action*, the send
decision is the confirmation pass of §4.2, whose inputs are the rule's
description and an instruction compiled into the binary — so the digest also
carries that instruction's version, and a build that changes it lapses every
such approval. What the digest does **not** cover is the main pass's learned
triage rules, which change by design every time the owner corrects a verdict
(§10). For send rules that is harmless, because a learned rule can only
nominate a candidate the confirmation pass then judges without it. For a
description rule with only mailbox and record actions it means the matcher
can widen after approval: accepted, because those actions are reversible,
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

The one precondition is structural, not historical:

- **A closed drafting context.** The drafting run's tools are only: the
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
interlock. A graduated reply is sent by the harness, not by a tool the model
calls, so the interlock never sees a send to refuse. What bounds the leak is
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
- **The owner undoes more than 20% of its automatic mailbox actions** over at
  least 10 → `propose`.
- **Three consecutive action failures** → `shadow`.
- **A rule's daily cap is hit** (§5.4) → that rule to `shadow` until the
  owner looks.
- **The cap across all rules is hit** → no rule changes mode, because the
  rule that happened to be fifty-first is not the cause. Every outbound action
  for the rest of the local day stages in the outbox instead, the ledger rows
  say `held: global cap`, and the owner gets one notice. Mailbox and record
  actions are unaffected.

`mecha mail rule pause-all` (and a button) drops every rule to `shadow` at
once. Stepping down never needs approval; stepping up always does.

---

## 7. The digest

`mecha mail digest [--since <last>]` — deterministic, reading the triage store
and the rule ledger, no model:

1. **Handled for you** — per rule, what it did since the last digest (counts,
   expandable to rows), so nothing an automatic rule removed is invisible.
2. **For your information** — `notify`-bucket threads: sender, the
   classifier's one-line, tags, deadline.
3. **Possible tasks** — threads whose verdict proposed `task` or carries a
   deadline, each with a *Make task* action (`mecha mail task`, deadline
   carried).
4. **Still waiting** — `mecha mail list --aged`, which the morning briefing
   appends today; the digest takes that section over.

It is a reader of the stores, like `--aged`, so the CLI, a trigger and the web
page show the same thing. **And it keeps `mecha mail list`'s split between
looking and recording**: `mecha mail digest` only reads, and `mecha mail
digest --surface` additionally records what it showed — the day-two threads
as surfaced (exactly what `list --aged --surface` writes today) and the
"handled for you" rows as reported, so the next digest starts after them. A
digest that wrote on every read could not be used to look. **It arrives each
morning with the briefing (R9)**: the `morning` trigger's `notify` step
replaces `mecha mail list --aged --surface` with `mecha mail digest
--surface`, so it is published on the same page by the same path, and there
is no second schedule or channel. The `one_line` text is fine here: the
digest is a surface for a human, never input to a privileged run
(`Record::for_privileged_run` stays as it is).

`digest` as a rule action means "archive it, and make sure I see it tonight" —
the newsletter you want to skim but not in your inbox. `task` as an action
creates the task directly; it defaults to `propose`, so the digest offers it
with one tap, because a task board filled by whoever can send you mail is a
board you stop trusting.

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
  `done`, `would have` (shadow), `staged`, `sent`, `released unchanged`,
  `edited`, `rejected`, `undone`, `failed: <why>`, `skipped: <guard>`.
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
wire format.

---

## 9. A conversation may draft a rule; only the owner installs one

The owner's wish behind R4 — "the harness can create them as needed" — is
real and worth serving, and the safe shape exists already in
`hud_propose` and `persona_propose`:

- `mail_rule_propose` is a tool a chat can call with a rule's TOML and
  template text as its *input*. It writes to `proposals/<id>/`, never to
  `rules/`, records its `Origin` from `ToolCtx::recorded_taint`, and returns
  no data.
- The proposal is checked by the same loader an installed rule uses, so a
  proposal that previews clean installs clean.
- The owner sees the rendered rule — the template as the recipient would read
  it — and installs it. **Installation is always into `shadow`**, whatever the
  proposal says, and an armed proposal is labelled as one. The accept names
  the digest the owner was shown.
- **The recorded `Origin` gates what can send.** The confirmation pass
  (§4.2) is sound only because its frame is the owner's words, and a proposal
  drafted after reading mail installs a description whose origin is
  `Origin::of_proposal`'s untrusted answer. So **no outbound action group may
  enter `auto` while the rule's description or any template it sends is not
  `Clean`.** The rescue is `Reflexion::provenance`'s existing move: the
  installed record keeps a digest of each proposed text, and once the owner
  has changed one, it is the owner's words and reads `Clean`. The approval
  screen says which text is holding the rule back and opens it for editing.
  Shadow bounds when a proposed rule acts; this bounds what it may say.

So "write me a rule that answers prerequisite questions with the usual
reply" works from a chat, ends as a rule in shadow mode, and the first
automatic send is still the owner's decision made against a week of real
matches.

---

## 10. Corrections

**Wrong match** on a ledger row writes a field-level correction (`field =
"rule"`, was, now) to the triage record, in the shape `MAIL-UX-DESIGN.md` §6
already defines, so it joins the classifier's few-shot pool. That makes rule
precision improve the way bucket precision does, with nothing new. **Missed
match** — from the mail desk, *this should have matched rule X* — is the same
correction in the other direction.

---

## 11. Build plan

Each phase is useful alone and testable without the next.

**P0 — mecha-mail prerequisites.** Expose `Auto-Submitted`, `Precedence`,
`List-Id`, `Authentication-Results`, `Reply-To` and the owner's
participation in a thread on the thread read — `Authentication-Results` as the
topmost instance with its `authserv-id`, absent stated rather than defaulted,
with a test per provider (§5.4); and the provider's own message id on the
thread read for the ledger key (§4.4). Restore verbs for undo (back to
inbox, out of trash) on both providers. Nothing changes on the send side:
rule replies go through the existing `mail_reply` path as ordinary mail (R5).

**P1 — the store, header rules, mailbox actions.** `rule.toml`, approvals,
the ledger, `off`/`shadow`/`propose`/`auto` for the mailbox and record
groups, the five-minute timer, `mecha mail rule add | list | show | mode |
test | ledger | undo`, the feature registry entry (off by default) — through
`ARCHITECTURE.md` §Features → *Adding a feature*, whose dozen places include
the web pane's `PANE_FEATURE` entry P3 will need (#654 missed exactly that) —
and
`mecha doctor` reporting an unreadable store or a lapsed approval. **Covers
GitHub notifications and junk with no model involved.**

**P2 — the digest.** `mecha mail digest` and its `--surface` write, replacing
`list --aged --surface` in the morning briefing.

**P3 — `#mail/rules`.** The list, the rule page, the ledger with undo and
wrong-match, header-rule testing, *make a rule from this thread*.

**P4 — description rules and fixed sends.** The classifier's `rule` field
with grounding, the confirmation pass for send candidates and the sentence it
adds to `learnable()`'s comment (§4.2), template `reply`, verbatim `forward`,
§5.4's guards, the approval screen. In `propose` until the owner approves a
rule.

**P5 — typed-field sends.** `fields`, the grounded extraction, the rendered
template.

**P6 — drafted replies.** `draft_with`, the closed drafting context, the
`shareable` frontmatter key, graduation through the approval screen.

**P7 — `mail_rule_propose`.**

When P1 lands, `ARCHITECTURE.md` gets a *Mail rules* section for the
invariants and `TRIFECTA.md` gets the rule-driven send as a channel, with
§5 and §6.3's arguments.

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
   matcher (§5.2).** The design's default, found in review: without it, any
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
  `reply`'s authenticated sender.
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
