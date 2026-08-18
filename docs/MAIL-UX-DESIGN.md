# Managing mail from mecha — design

*2026-08-18. What phases 1–3 settled, and what phases 4–6 will be.
`docs/MAIL-UX-RESEARCH.md` is the survey this argues from; where the two
disagree, this one is later and wins. Written for review before 4–6 are built,
so anything here is still cheap to change.*

---

## 0. What is already true

Shipped and running, so these are facts rather than proposals:

| | |
|---|---|
| `mail_triage` | archive / read / unread / spam / trash, closed enum, thread-level, both providers |
| `~/.mecha/mail-triage/` | one typed verdict per thread; ids and envelope metadata, never bodies |
| The classifier | no tools, no history, no system prompt, no shared cache prefix; one isolated call per thread |
| `mecha mail` | `classify` / `list` / `show` / `dismiss` |
| Escalation | snippet first; full body when the verdict is `respond` or names a request kind (~25% of threads) |
| The nightly | `mecha-mail-classify.timer`, 05:30 UTC, Dartmouth only |

Measured on 51 real threads: 30 `ignore`, 9 `notify`, 12 `respond`. Five
request kinds recognised.

### Six decisions that are settled, and should not be re-opened without cause

1. **The privileged run sees the extraction, never the prose.**
   `Record::for_privileged_run` has no argument that returns the subject, the
   sender's display name, the classifier's reasoning, or `one_line`.
2. **Tags are mecha's own**, not Gmail labels or Graph categories. They cost no
   scope and work identically per provider. The price — invisible in every
   other mail client — is accepted.
3. **`mail_triage` is `destructiveHint` alone**: never in `[outbox] tools`,
   never `readOnlyHint`.
4. **Recognising a request kind is not routing it.** `REQUEST_TYPES` is what
   the classifier can name; `ROUTABLE_TYPES` is the subset with a manifest.
5. **The store is an index, not a mailbox copy.**
6. **Google stays in Testing**; CASA revisited after the main features land.

---

## 1. Phase 4 — front-door routing

### The claim

An email asking for a letter is a `letter` request that arrived through the
wrong door, untyped. `mecha-manifest/types/` already describes five such
requests, and `~/.mecha/requests/` already has the machinery to carry one to an
answer: a quarantined extractor, `needs-info`, `triage` drafting into the
outbox, `reconcile` closing the loop, and `close` requiring a reason.

Phase 4 is the promotion, and almost nothing else.

### What a promoted record looks like

`frontdoor.rs`'s `extractor_prompt` **never consults the manifest** — it works
from `record.prose()` and emits a fixed schema. So a mail-promoted record needs
no manifest to be extracted; the manifest matters for knowing what is *missing*.

Which gives the shape: **a mail-arriving request is a form request with every
field blank.**

```jsonc
{
  "seq":        <next>,
  "type_id":    "letter",            // from verdict.request_type, routable only
  "state":      "drained",
  "valid":      true,
  "values":     { "requester_email": "…" },   // only what the envelope proves
  "free_text":  ["body"],            // the whole message is prose
  "reply_to":   "<sender address>",  // an address, used as an address
  "origin":     { "kind": "mail", "account": "…", "thread_id": "…" }
}
```

Three things to note:

- **`values` is nearly empty, and that is the point.** `needs-info` stops being
  a fallback and becomes the primary path: the reply asks for exactly the
  fields the form would have collected, ideally with the form's URL. The most
  useful thing mecha can do with "can you write me a letter?" is ask the six
  questions that make a good one possible.
- **`reply_to` comes from the envelope**, not from the prose. The front door's
  own note applies: this is an address, not evidence about who anybody is.
- **`origin` is new** and is the join back to the thread, so `reconcile` can
  mark the mail record `acted` when the drafted reply is sent. The front door
  has never heard of mail and will not now; this is one field it preserves.

### The promotion is a keystroke, never automatic

`mecha mail route <thread_id>`, and `f` in the modal.

Auto-promotion would let a classifier decide what enters the privileged request
queue, and that queue's entire value is that a human put things there. The
classifier already misroutes — a lab application came back as `meeting` before
the prompt was fixed — and the cost of a wrong promotion is a stranger getting
a reply about the wrong thing.

**Unroutable kinds cannot be promoted at all.** `review`, `grant-support` and
`data-request` are recognised but have no manifest, so `route` refuses them by
name and says why. They stay in the queue as evidence about what actually
arrives, which is what will decide whether those manifests get written.

### What phase 4 does not do

No manifest authoring — that is `mecha-factory`'s repository and a separate
decision, deliberately informed by what this store accumulates rather than
guessed at first.

---

## 2. Phase 5 — `/mail`

A sixth modal on the `/outbox` pattern: store read for display, every mutation
a `mecha mail …` child process, slow work spawned detached and watched by
polling the store rather than the child.

```
 ┌ mail ───────────────────────────── 22 need you · 3 drafted · 2 parked ─┐
 │ ● today  dartmouth  #admin       JOCN review — accept or decline       │
 │ ● week   dartmouth  #lab-app     PhD applicant asks about openings     │
 │   week   dartmouth  #rec-letter  Endorsement letter, due Sep 1         │
 │   none   dartmouth  #expense     Amazon receipt, $412                  │
 │ ✎ drafted dartmouth #lab-app     reply staged → /outbox                │
 └ r reply · a archive · s spam · e schedule · t task · f route · ! wrong ┘
```

| key | action | lands as |
|---|---|---|
| `r` | reply | detached agent run → drafts into **`/outbox`** |
| `a` | archive | `mail_triage`, immediate |
| `s` | spam | `mail_triage`, immediate, confirms |
| `e` | schedule | `calendar_create_event`, staged |
| `t` | task | `kg_task_create`, immediate |
| `f` | route | promote to the front door — routable kinds only |
| `g` | tag | edit tags on the record, no model |
| `n` | needs-info | park it |
| `!` | wrong | a correction (phase 6) |
| `enter` | detail | prose, verdict, reasoning |

Three rules carried from `/outbox`:

- **`r`, `e` and `f` are agent runs**, not keystrokes — they build a tool
  surface and can take minutes, so they spawn detached and are watched.
  `a`, `s`, `t`, `g` are single calls and run synchronously.
- **The result of a reply lands in `/outbox`, not here.** There is exactly one
  approval surface and this is not it. `/mail` decides *whether* something
  needs an answer; `/outbox` decides whether *this* answer goes.
- **`s` confirms.** Spam trains the provider's filter; it is the one triage
  action with an effect outside the mailbox.

---

## 3. Phase 6 — the correction loop

`!` marks a verdict wrong. Three things happen, and keeping them apart is the
design:

1. **The record is corrected in place** — a typed before/after pair on the
   store. Free, deterministic, and immediately useful to the list.
2. **The pair joins a few-shot pool** the classifier's prompt draws from.
   Cheap, fast-acting, small blast radius: it steers a tool-less pass that
   emits a fixed schema.
3. **A `triage`-domain reflection** goes to the learning store, on the ordinary
   path — provenance gating, the proposal gate, the validation ledger.

### Why the few-shot pool is not a learned rule

A learned rule rides in every future run's cached prefix, which is why
`learning.rs` gates provenance so hard. A few-shot example injected into a
tool-less classifier that returns a fixed schema is a far smaller thing. Fusing
them would mean either over-gating the cheap mechanism into uselessness or
under-gating the expensive one, and the second is how third-party text reaches
every future prompt.

`triage` is therefore **not** in `RUN_DOMAINS` — its rules ride in the
classifier's own frame and nowhere else. That separation is why domain
selection was built before this phase existed.

### What a correction records

`{ thread_id, account, field, was, now, at }` — field-level, because "wrong" is
not one thing. A misread bucket, a missed deadline and a wrong `request_type`
are different errors with different fixes, and a correction store that flattens
them teaches the classifier noise.

---

## 4. Open questions, for review

1. **Should `r` (reply) hand the drafting run the thread, or the verdict?**
   The thread is what an answer is written from, and taking the taint honestly
   is the design elsewhere. But it means every reply run is trifecta-armed and
   its draft comes out red in `/outbox`. Leaning: hand it the thread, accept
   the red, because the alternative is drafting a reply from a summary.
2. **Does `t` (task) attach the thread id to the task?** pkg's
   `kg_task_create` takes `name`, `due`, `context`, `project`. A pointer back
   to the mail would make "why is this on my board" answerable, but there is no
   field for it — it would have to live in the name or need a pkg change.
3. **Should the nightly promote nothing, or park routable kinds for review?**
   Currently it classifies only. A middle option is to mark routable threads so
   the modal can show a "3 ready to route" badge.
4. **`meeting` as a request kind** — still unresolved. It is structurally the
   greediest label (almost any request can be discussed in a meeting) and the
   booking flow may already cover it better.
5. **Retention.** Nothing prunes `~/.mecha/mail-triage/`. A year of nightlies
   is perhaps 15k small files. `mecha work clean` has a policy shape worth
   copying, but an archived verdict is also the eval fixture and the few-shot
   pool, so deleting has a cost the work directory does not have.

---

## 5. What is deliberately not being built

- **Auto-send of anything**, at any confidence, including the finance forward.
- **An autonomy tier.** Inbox Zero's per-rule graduation is the good version
  and it is v2 at the earliest; it should follow `/review now|later|auto`'s
  rule — set by explicit command, never inferred.
- **A second approval surface.** The outbox is it.
- **`mail_snooze`.** Neither provider has one; it would be a label plus a
  trigger, and a snooze that silently means "labelled and forgotten" is the
  silently-degrading-sandbox shape.
- **A mail cache or local index.**
- **Manifests invented from email traffic** without a human writing them.
