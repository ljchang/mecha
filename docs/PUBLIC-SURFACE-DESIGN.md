# The public surface — design

Decisions, not evidence. `docs/PUBLIC-SURFACE-RESEARCH.md` is the survey and
the argument; this is what we are actually building, settled 2026-08-05 after
the user reviewed it. Where a decision contradicts the research doc, this file
wins and the research doc keeps the reasoning.

**Still unbuilt.** This is the shape to build, written so someone can start.

---

## 1. Decisions taken

| Question | Decision | Consequence |
|---|---|---|
| Own server, or a platform? | **Own server on a VPS.** One Rust binary, SQLite in WAL, its own ACME. No CDN in front to begin with. | Total header control, nobody else terminating TLS, testable locally because it is just a program. A box to patch forever. |
| Where does the code live? | **`mecha-factory`, its own repository.** Rust throughout; Python only as a sandboxed marimo subprocess. | §2.1. It is a deployed server with its own release cycle, and the credential-isolation property becomes checkable by looking at what is deployed. Free on crates.io, PyPI and GitHub as of 2026-08-05. |
| How does mecha reach it? | **MCP for `publish`; CLI for `drain`.** | §2.2. Outbox routing is by tool *name* in the dispatch path, so staging works with zero mecha-core changes — and the common case (nothing new) must cost zero tokens, which rules out an agent tool. |
| How does mecha talk to it? | **API key, not OAuth.** Two scoped keys, minted by mecha, stored hashed on the server. | Mirrors the two forced-command SSH keys. OAuth buys delegation between parties, and there is only one party. |
| Which direction do packets go? | **Push–pull, unchanged.** mecha publishes and drains; the server never initiates. | The home machine's attack surface is unchanged by shipping this. |
| Versioning | **Immutable versions + a moving alias.** Every publish is a new version; the share URL points at the alias. | The property the user liked in Claude artifacts, and the only way "republish" is safe. |
| Read-back | **Bundles are built in the run's workspace and mirrored to `~/.mecha/bundles/`.** | An agent can read what it published, with no hole in the path jail. |
| Templates | **Yes, and they are the extension point** — `report`, `notebook`, `booking`, plus request-type starters. | Adding a kind of output or a kind of ask is writing a directory, not writing code. |
| marimo | **First-class. Three render modes, and only the third needs its own origin.** | Section 7. `marimo-book` already implements the modes and settles the asset question. |
| Where notebooks live | **Ours by default; molab as a second target** (`target = factory \| molab \| both`). | §7.7. molab is for notebooks that could go in a public GitHub repo; it has no publish API, no expiry, no revocation and no documented versioning. |
| Frontend | **Plain HTML + vanilla JS. No framework.** Server-rendered from MiniJinja, progressively enhanced. | §5.1. Confirmed by the user after reviewing the alternative. The form works with JavaScript disabled; JS only sharpens it. |
| Injection defense | **Four layers, in mecha, not on the server. The classifier is never a gate.** | Section 8. The server filters *shape*; only mecha can filter *meaning*, and a model on the public box would put a provider key on the box we assumed lost. |
| Scheduling | **Book-me first, then group availability *seeded by mine*.** | Section 9. The seeding makes the group case strictly easier than when2meet. |
| WebMCP | **Design for it, ship it later, behind a flag.** | Section 10. The manifest already emits everything it needs. |
| Compliance | **Deferred.** | Keep `retain_days` in the manifest so it stays cheap to turn on. Revisit before the form is linked from a public page. |

---

## 2. The pieces

```
        home (spark-8c43)                    the public box
  ┌──────────────────────────┐        ┌──────────────────────────────┐
  │ mecha                    │        │ mecha-factory (one binary)   │
  │  ├ tools: publish ───────┼──POST─▶│  /v1/bundles     ← publish   │
  │  │        (outbox-routed)│        │  /v1/types       ← schemas   │
  │  ├ trigger: drain ◀──────┼──GET───│  /v1/queue       ← drain     │
  │  └ triage run (tainted,  │        │                              │
  │     read-only, staged)   │        │  SQLite (WAL)                │
  │                          │        │  static bytes on disk        │
  │ ~/.mecha/                │        │                              │
  │  ├ frontdoor/types/      │        │  serves three origins:       │
  │  ├ bundles/<id>/<ver>/   │        │   gate / artifacts / compute │
  │  └ factory/*.key         │        └──────────────────────────────┘
  └──────────────────────────┘                     ▲
                                                   │ HTTPS
                                              the world
```

The publish path at home has one more boundary inside it (§2.3):

```
  markdown / data ──────────────────────┐
                                        │
  notebook.py ──▶ marimo (subprocess) ──┤
                  sandboxed: no network,│
                  no key, executes the  │
                  notebook              │
                                        ▼
                              factory-publish (Rust)
                              renders data→HTML in process,
                              runs the vendoring gate,
                              holds the key, POSTs
```

Only the marimo path is sandboxed, and the reason is not the language: it is
the only renderer that **executes code**. Everything else is data→HTML with
nothing to run.

### 2.1 Its own repository, and three deployables

An earlier draft of this document put the server in mecha's Cargo workspace and
argued that a second repository was two CI configs for one program. **That was
wrong, and reading `marimo-book` is what showed why: this is not one program.**

| Deployable | Language | Runs on | Holds |
|---|---|---|---|
| **`mecha-factory`** (bin `factory`) | Rust, one static binary | the VPS | request queue, published bytes, a TLS cert |
| **`mecha-factory-publish`** (bin `factory-publish`) | Rust | home | the publish API key; renders everything that is data→HTML |
| the marimo renderer | Python **subprocess** | home, sandboxed, **no network, no key** | executes notebook code |

**Correction, and it matters.** An earlier draft of this section said "the
render pipeline *is* marimo and marimo-book, both Python" and made that the
first argument for a separate repository. That was wrong. **Python is required
for exactly one class of input — marimo — and everything else renders in
Rust.** See the renderer table in §5.

So the publisher is Rust and shells out to marimo for the marimo templates,
which is the same move this project already makes for sandbox backends and MCP
servers: a subprocess with a narrow contract, not a second implementation
language for the whole component. It may not even need a bespoke Python
package — `marimo export html-wasm` and `marimo-book build` are already CLIs,
so the contract can be "invoke this tool, get a directory".

**The repository split survives, on the two arguments that are actually
load-bearing**, and it is worth being explicit that the language argument is no
longer one of them:

- **It is a deployed server.** Its release cycle is "push a binary to a box you
  patch forever"; mecha's is `cargo install`. Coupling those means a docs typo
  in mecha is a reason to think about the VPS.

  On Rust rather than Fastify, and stated at its real width rather than wider:
  the user's `pbs_knowledge` backend is Fastify with `@fastify/helmet`,
  `@fastify/swagger` and Ajv, and it is the right choice for what it is — a
  large application behind SAML with Firestore, Redis, Graph and Excel export.
  `mecha-factory` is a static file server plus **one unauthenticated write
  endpoint on a box we have agreed to assume is lost**, which is precisely
  where one static binary pays and a `node_modules` tree costs. That is the
  whole argument; it does not generalise to "Rust is better here in general",
  and the four-verb interface keeps the choice swappable.
- **The credential-isolation property becomes checkable by looking.** "The
  public box holds none of mecha's code and none of its credentials" is a
  claim you verify from what is deployed rather than from build discipline.
- Plus, weakly: it is genuinely usable by any MCP-speaking agent, and its
  integration tests want to stand up a server with real TLS and run marimo —
  heavier CI than `cargo test --workspace` should carry.

**The consequence for `mecha-manifest`:** both sides are Rust and both need it
(mecha validates drained records, the factory validates at the edge), so it
becomes **its own published crate** — free on crates.io as of today — rather
than a path dependency. Version discipline on the manifest is a feature, not
friction: it is a versioned schema format, and being forced to bump it
deliberately is the point.

**The name.** A factory is where machines are built and shipped from — and it
is deliberately *not* the machine. That is exactly the relationship this
repository has to mecha: same family, separate deployment, and forbidden from
depending on `mecha-core`. It also reads correctly in both directions, which
was the test every candidate name had to pass: orders come in, product goes
out. The mild caveat, recorded so nobody is surprised: "factory" leans
outbound, and half this repository is inbound typed requests. Orders-in
carries it, but it is the one place the metaphor has to stretch.

Three reasons the repository boundary is the right one:

- **Two languages.** The render pipeline *is* marimo and marimo-book, both
  Python. A Rust wrapper would shell out to Python to do all the work. Putting
  a Python package with pytest, ruff and a marimo dependency inside a repo
  whose CI is `cargo test --workspace && cargo clippy` grows a Python matrix
  onto the agent harness for something unrelated to it.
- **Different release cadences.** The server is deployed to a machine; mecha is
  a tool you `cargo install`. Coupling their versions means a docs typo in
  mecha is a reason to think about the VPS.
- **The property we keep claiming becomes trivially checkable.** "The public
  box has none of mecha's code and none of its credentials" is a sentence you
  verify by looking at what is deployed, and a separate repository makes it
  obvious rather than a matter of build discipline.

The one hard invariant survives regardless: **nothing in this repository may
depend on `mecha-core`.** The shared contract is the manifest, and it is
**data** — TOML plus a generated JSON Schema — not a shared type. Each side
parses it in its own language. That is a feature: it forces the contract to be
inspectable rather than a struct two crates happen to agree on, and it is what
lets validation happen independently at the edge and at home.

### 2.2 MCP for `publish`, CLI for `drain`

The mecha-mail shape applies, with one deliberate exception.

**`publish` and friends are MCP tools**, served by `mecha-factory-publish`, for three
concrete reasons rather than for consistency:

- **Outbox routing is by tool name in the dispatch path.** `agent.rs` asks
  `cx.outbox.routes(name)` before execution and stages if it matches — with no
  knowledge of where the tool came from. So naming `publish` in
  `[outbox] tools` gives us "an agent drafts a page, a human releases it" with
  **zero changes to mecha-core**. That is the same argument that made an email
  tool outbox-coverable without the outbox knowing what email is.
- **`[[mcp]] capabilities` overrides already exist** and only ever widen, so
  config can force `untrusted_input` on anything that reads back from the
  surface.
- **The agent loop stays ignorant**, which is the project's founding
  invariant. A native tool would put the surface's URL scheme inside
  mecha-core.

And the reusability that motivated all this: any MCP-speaking agent — Claude
Code, anything else — can publish to the same surface without knowing mecha
exists.

**`drain` is a CLI, not a tool.** The common case is "nothing new", and it must
cost zero tokens; a trigger runs `factory drain` on a schedule and only spawns
an agent run when the queue was non-empty. Making it a tool would put a model
in the polling loop, which is the same mistake as putting one in the request
path, one hop later.

### 2.3 Render and publish are split, because rendering executes the notebook

Not an accident of packaging — a trust boundary, and the thing most likely to
be got wrong by merging them.

`MarimoIslandGenerator.build()` **executes the notebook** to capture initial
state. So the renderer runs arbitrary Python. If the same process also held the
publish API key and had network access, a compromised dependency anywhere in a
notebook's import graph would have both.

So: **the marimo subprocess executes notebook code in the sandbox with no
network and no key, and emits a directory. `mecha-factory-publish` takes a
directory, runs the vendoring check, and POSTs it — holding the key, executing
nothing.** Same
instinct the sandbox already encodes for `shell`: narrow what the dangerous
thing can reach, rather than trusting it.

The vendoring gate (§7.2) belongs in `mecha-factory-publish`, on the far side of that
boundary, so the check runs on bytes rather than on the process that produced
them.

---

## 3. The typed state machine

The thing the user liked, made concrete. Every inbound request is one row with
one state, and **every transition is either deterministic or staged for a
human — never a model acting alone.**

```
                    ┌──────────── deterministic, at the origin ─────────────┐
                    │                                                       │
  submitted ──▶ verified ──▶ queued ──▶ drained ──▶ triaged ──▶ awaiting_me
      │             │           │                       │            │
      │ (no click)  │ cap/season/notice fails           │            │ I act
      ▼             ▼                                   ▼            ▼
   expired      declined ◀──────────────────────── needs_info ──▶ answered
                    │                                                │
                    └──────────────────▶ closed ◀────────────────────┘
```

| Transition | Who | Notes |
|---|---|---|
| `submitted → verified` | the origin | magic link clicked; single-use, short expiry |
| `submitted → expired` | the origin | never verified. **Never drained, never costs a token.** |
| `verified → declined` | the origin | capacity, season, or minimum-notice rule failed. Templated, immediate, dignified. |
| `verified → queued` | the origin | the only path into the queue |
| `queued → drained` | mecha | schema-validated at home before it enters anything |
| `drained → triaged` | mecha | quarantined extraction (§8), then a triage run |
| `triaged → awaiting_me` | mecha | a staged draft in the outbox, grouped by type |
| `awaiting_me → answered` | **me** | outbox release |
| `triaged → needs_info` | **me** | reply carries a link back into the same typed flow |
| any `→ closed` | the ledger | with a reason. Silence is the failure mode we are here to fix. |

Two rules that make this worth having:

- **A request has one row, and the row is the truth.** Not a thread, not a
  conversation, not an inbox search. `mecha frontdoor list --state awaiting_me`
  is the queue.
- **Expiry is a state, not an absence.** Anything that sits too long in
  `awaiting_me` gets an honest auto-response and moves to `closed`. That is the
  one feature an inbox structurally cannot have.

Outbound bundles get a much smaller machine: `built → published → aliased`,
plus `superseded` when a newer version takes the alias. Nothing is ever
deleted; the alias moves.

---

## 4. The protocol

Four verbs over HTTPS, JSON in and out, bearer token. Deliberately boring.

```
PUT    /v1/types/{id}          upload a request-type manifest + JSON Schema
POST   /v1/bundles             publish a bundle    → {id, version, urls}
POST   /v1/bundles/{id}/alias  point the share URL at a version
GET    /v1/queue?since={seq}   drain              → [typed records]
POST   /v1/queue/ack           delete what we took (by seq)
GET    /v1/health              is it up, what version, how many queued
```

### Auth

Two keys, two scopes, mirroring the two `rrsync` forced commands:

| Key | Scope | Lives |
|---|---|---|
| `mk_pub_…` | `PUT /v1/types`, `POST /v1/bundles*` | `~/.mecha/factory/publish.key`, mode 0600 |
| `mk_drn_…` | `GET /v1/queue`, `POST /v1/queue/ack` | `~/.mecha/factory/drain.key`, mode 0600 |

Minted by `factory key create --scope publish`, printed once, stored on
the server as an Argon2id hash. **The server holds no key that reaches home**,
which stays the single property to verify by inspection. Rotation is: mint,
install, revoke — the server keeps both live until the old one is revoked.

An API key rather than OAuth because OAuth exists to let a third party act on a
user's behalf with scoped, revocable delegation, and there is no third party
here. If a second consumer ever appears (a phone app releasing outbox drafts),
that is the moment to revisit — and it would want OAuth for the *human*, not
for mecha.

### The property that makes `drain` safe

> **The server can only return objects that validate against a schema mecha
> itself uploaded**, and mecha re-validates on arrival before anything enters a
> conversation.

That is not belt-and-braces, it is the whole containment story for a
compromised public box. A hostile server cannot invent a field, cannot change a
request's type, cannot exceed a length cap, and cannot make the record claim a
verification that did not happen (the verification stamp is signed by the
origin, but the *decision* to trust it is mecha's, and a forged stamp only gets
you to `queued` — which is where every stranger already is). What it can do is
put hostile prose in a field that was already declared `untrusted = true`,
which is exactly the case §8 is built for.

Everything drained arrives `.from_outside()`. No exceptions, including the
fields that look structural — a compromised origin controls all of them.

### Idempotency

`POST /v1/bundles` takes an `Idempotency-Key`; a retry after a timeout returns
the original version rather than minting a second. The retry rule mecha already
lives by — *a retry must never duplicate work* — applies to publishing a report
exactly as it applies to a provider call.

---

## 5. Templates

A template is a directory plus a `template.toml`, rendered with MiniJinja. Two
families, because there are two directions.

```
templates/
  report/        outbound, class=static|interactive   prose + computed figures
  notebook/      outbound, class=compute              a whole marimo notebook
  dashboard/     outbound, class=interactive          charts, no network, no eval
  booking/       inbound + outbound                   availability page + claim
  request/       inbound                              the generic typed form
```

**Which renderer each template needs**, since this is where the language
question actually lives:

| Template | Renderer | Where | Executes code? |
|---|---|---|---|
| `report` (markdown) | pulldown-cmark + MiniJinja | Rust, in process | no |
| `dashboard` | MiniJinja + a data file | Rust, in process | no |
| `booking` | MiniJinja + availability JSON | Rust, in process | no |
| `request` (the form) | generated from the manifest | Rust, in process | no |
| `notebook` | `marimo export html-wasm` | Python subprocess | **yes** |
| `report` with live cells | `marimo-book build` | Python subprocess | **yes** |

Four of six render in Rust with nothing to execute. **The sandbox is required
exactly where the renderer runs code we did not write** — the marimo rows,
because both `marimo export` and `MarimoIslandGenerator.build()` execute the
notebook to capture its outputs. That is the real boundary, and it happens to
fall on a language line rather than being caused by one.

The corollary worth holding onto: **a report, a dashboard, a booking page and
a form need no Python at all.** If marimo were never wired up, everything
except the notebook templates would still work.

`report` spans two classes on purpose: it is `static` until a discrete widget
appears, and `interactive` once precompute ships a lookup table and a shim
(§7.5). The publisher decides the class from what was actually emitted, not
from what the template declared — declaring `static` and emitting a `<script>`
must fail the publish, not silently upgrade it.

Each declares what it needs and what it emits:

```toml
[template]
id = "notebook"
class = "compute"                # decides the CSP and therefore the origin (§7)
inputs = ["notebook_path", "title"]
build = "marimo export html-wasm {notebook} -o {out} --mode run"
vendor = true                    # rewrite every external reference, or fail
```

### 5.1 How the form is actually generated

Plain HTML and vanilla JavaScript, no framework. The decision is the user's,
made after reviewing the alternative, and it is the simpler option rather than
a compromise — an earlier draft of this document argued for it too strongly and
then over-corrected; the honest position is that a framework would have been
*fine*, and going without costs little here because there are five forms rather
than an application.

The generation has two layers, and the first does most of the work:

- **HTML5 constraint attributes, emitted from the manifest.** `required`,
  `type="email"`, `type="date"`, `min`, `max`, `step`, `maxlength`, `pattern`,
  and `<select>` for every enum. The browser validates natively, announces
  errors to a screen reader natively, and **needs no JavaScript at all**. For
  the majority of every form, this is the entire client-side story.
- **A small declarative-condition evaluator** for the rest: show/hide, and
  cross-field rules that HTML5 cannot express. A few hundred lines of vanilla
  JS reading the same manifest, with the server re-evaluating every rule on
  submit because a client-side check is a convenience and never a control.

Multi-step is server-side — one page per step, a POST between them — so it
works with JavaScript off and survives a closed tab. Drafts key off the
capability token rather than `localStorage`, for the same reason.

**What we are taking from `pbs_knowledge`, and what we are not.** The user's
departmental site already runs a schema-driven form system in production —
`FormRenderer`, `WizardEntityForm`, an admin `FormConfigEditor`, and thirteen
live configs (honors thesis, dissertation, annual review, mentoring forms,
award nomination, fellowship application). Since we are not using Svelte, the
*components* are not reusable. **The config format is**, and it is the more
valuable half: a shape someone has lived with across thirteen real forms beats
one invented in a design document, which is the same argument as mining the
mail rather than guessing at request types.

Three things to lift directly:

- **`WizardStep`** — `fields`, `requiredFields`, `hiddenFields`,
  `conditionalFields`, `stepType`. This is the manifest's step model, already
  proven.
- **`AcknowledgmentConfig`** — a required checkbox with a label, a description
  and an `infoLink`. That *is* the FERPA-consent-as-a-field design from §9,
  already solved: consent is a boolean a human sets, with a link to what they
  are consenting to.
- **`WizardDraft`** — save and resume a partial submission. Worth having from
  the start; it is what lets a reply carry a link back into the same typed flow
  rather than asking a question in prose.

And one rule to impose that their config does not need: **the declarative
subset only.** `pbs_knowledge` allows a step's `showWhen`, `validate`,
`skipWhen` and `canProceed` to be either a `DeclarativeCondition` or an
arbitrary TypeScript closure. A closure cannot cross to a Rust server, and the
server must evaluate exactly the rules the browser did — so the manifest takes
`DeclarativeCondition` (`field`, `operator`, `value`) and forbids the function
form. Their type is already a union of the two; we keep one arm of it.

Three rules for templates, each of which is a bug if undone:

- **The template declares the content class; the publisher enforces the
  policy.** A template cannot ask for a laxer CSP than its class allows, and a
  `compute` template cannot be published to the artifact origin. The check is
  at publish time, in `mecha-factory`, not in the template.
- **`vendor = true` means the publish *fails* on a surviving external
  reference.** Not warns. This is the one enforcement the whole artifact
  security model rests on, and a warning is how it silently stops holding.
- **Templates are data, not code**, and they live in
  `~/.mecha/frontdoor/templates/` — never in a project's `mecha.toml`, for the
  same reason request types don't. A cloned repository must not be able to
  ship a template that publishes to your domain.

Request-type starters ship as manifests, not code: `meeting`, `letter`,
`speaking`, `lab-application`, `question`. Copy one, edit the fields, and the
form, both validators, the tool declarations and the triage frame regenerate.

---

## 6. Bundles: versioning, immutability, read-back

The four properties the user liked about Claude artifacts, and how each is
obtained:

| Property | How |
|---|---|
| Unique URL | `https://<artifacts-origin>/b/<id>/` → the current alias |
| Requires auth to view | capability URL checked on the **gate** origin, which issues a short-lived single-use URL on the artifact origin |
| Can be made public | one flag on the bundle row; the gate stops requiring the capability |
| Permanent | versions are immutable and never deleted; the alias moves |
| Versioned | `/b/<id>/v/<n>/` addresses a specific version forever |
| Readable by the agent | mirrored to `~/.mecha/bundles/<id>/<version>/` and built in the run's workspace |

A version is content-addressed: the publish computes a digest over the bundle,
and re-publishing identical bytes returns the existing version rather than
minting a new one. That makes "did anything actually change?" a comparison
rather than a guess, and it makes the nightly briefing that produced the same
page twice cost one row.

**The transcript is still the record.** A bundle is a rendering of a run, and
when they disagree the transcript wins — the rule `/triggers` already follows.
Versioning does not change that; it just means the rendering has a history too.

---

## 7. Serving marimo safely

The hardest part, and the one the user named as a must-have. Three problems,
in the order they bite.

### 7.1 The CSP problem

Pyodide cannot run under the artifact CSP: WebAssembly instantiation is blocked
by any policy with `script-src`/`default-src` unless it grants
`wasm-unsafe-eval`. Granting that on the artifact origin would weaken every
static report to accommodate notebooks — the silently-degrading-sandbox shape.

**The answer is a third origin**, and its policy is:

```
Content-Security-Policy:
  default-src 'self';
  script-src 'self' 'wasm-unsafe-eval';
  style-src 'self' 'unsafe-inline';
  img-src 'self' data: blob:;
  connect-src 'self';
  worker-src 'self' blob:;
  frame-ancestors 'none';
  base-uri 'none';
  form-action 'none'
X-Content-Type-Options: nosniff
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
Cross-Origin-Resource-Policy: same-origin
```

Notes on the choices, because each cost something to decide:

- **`wasm-unsafe-eval`, never `unsafe-eval`.** The narrow directive permits
  `WebAssembly.compile`/`instantiate` and still forbids `eval` and
  `new Function`. Chrome 97+, Firefox 102+.
- **COOP/COEP are on**, which unlocks `SharedArrayBuffer` for Pyodide's
  threading. They are normally painful because they break third-party embeds —
  here everything is same-origin and vendored, so they cost nothing. This is a
  case where the strict thing is also the free thing.
- **`connect-src 'self'`** is what stops a notebook phoning home. A notebook
  that wants remote data must have the data baked in at publish time, which is
  the right constraint for something meant to be reproducible anyway.
- **`frame-ancestors 'none'`** so a notebook cannot be framed by the gate.

### 7.2 The vendoring problem, now settled

The research doc left this open because marimo's documentation is silent and
its issue tracker contradicts itself. **`marimo-book` answers it**, because the
user already built and shipped a static marimo publisher and its source records
what actually happens.

What is external in a marimo-book build, enumerated from the source rather than
guessed:

| Reference | Where from | Vendorable |
|---|---|---|
| marimo islands frontend bundle | `MarimoIslandGenerator.render_head()` → jsdelivr, by default | yes — rewrite the emitted `<script>`/`<link>` |
| Pyodide | loaded by that bundle on first paint | yes — pinned copy, rewrite the loader path |
| MathJax | `cdn.jsdelivr.net/npm/mathjax@3/…`, hard-coded | yes — one file |
| Plotly | `cdn.jsdelivr.net/npm/plotly.js-dist-min@2.35.2/…`, lazy | yes — one file, already version-pinned |
| Gravatar (blog avatars) | `gravatar.com` | drop it; a default avatar is a local asset |
| third-party wheels | micropip/Pyodide at runtime | yes — declare and vendor at publish time |

So it is **four hosts and six references, all version-pinned** — a bounded
afternoon, not a research project. The rule stands unchanged and is now known
to be achievable: **the publisher rewrites external references to vendored
copies and fails the publish if any survive.** Pin the Pyodide version per
bundle so a notebook published today still runs when Pyodide moves.

marimo-book never needed this because it deploys to GitHub Pages, which cannot
set response headers and therefore has no CSP to satisfy. We can set headers,
which is the whole reason our artifacts are safe to open — so the vendoring
pass is **new work we add on top of marimo-book**, not something to borrow from
it. That asymmetry is worth stating plainly so nobody wonders why the existing
tool doesn't already do it.

The verification recipe stays, as a publish-time gate rather than a one-off:

```bash
grep -rIoE 'https?://[^"'\''` )]+' "$bundle" | sort -u    # must be empty
```

### 7.3 The `data:` URL problem, which is new and specific

marimo-book's source records a trap that would have cost us a day. Under
marimo's `ScriptRuntimeContext`, `virtual_files_supported=False`, so every
anywidget's ES module is emitted as a `data:text/javascript;base64,…` URL — and
marimo's own islands runtime then **refuses to load them** ("Refusing to load
anywidget module from untrusted URL"; only `@file/…` is trusted). marimo-book
works around this with its own shim that `import()`s the data URL directly.

That workaround collides with our CSP: dynamic `import()` of a `data:` URL is
governed by `script-src`, and `data:` is not permitted by default. Two options,
and the first is clearly right:

- **Rewrite the data URLs into vendored files at publish time**, turning them
  into ordinary `'self'` resources. This is the same pass §7.2 already needs,
  extended by one rule, and it keeps `data:` out of `script-src` entirely.
- Allow `data:` in `script-src` on the compute origin only. Cheaper, and it
  re-opens a script-execution channel on the one origin that already has
  `wasm-unsafe-eval`. Don't.

### 7.4 Two notebook paths, because they answer different questions

The other thing marimo-book's source settles: **islands and `export html-wasm`
are not interchangeable, and the difference is dependencies.**

The islands runtime has two package paths — `loadPackagesFromImports`, which
AST-scans cell source for Pyodide-bundled packages, and a `micropip.install`
call over a list **hard-coded into the JS bundle**. Nothing reads PEP 723. So a
pure-Python package on PyPI that isn't in Pyodide's distribution (`nltools` is
the case that motivated the finding) silently fails to import, with no hook on
the host page to fix it. `marimo export html-wasm` *does* read PEP 723.

Hence two templates rather than one:

| Template | Path | When | Class |
|---|---|---|---|
| `notebook` | `marimo export html-wasm --mode run` | share a whole notebook; arbitrary deps | `compute` |
| `report` | marimo-book's static / precompute pipeline | prose with computed figures | `static` or `interactive` |

The second is the discovery worth the most here, and §7.5 is about it.

### 7.5 Most "interactive" notebooks do not need WASM at all

marimo-book has a **kernel-free reactivity path**, and it maps exactly onto the
content classes this design already had — which is a good sign that the classes
are real and not invented.

- **`static`** — outputs baked at build time. No script, no runtime, no Python
  in the browser. The strictest CSP applies unchanged.
- **`static` + precompute** — a discrete widget (`mo.ui.slider` with explicit
  steps, `dropdown`, `radio`, `checkbox`, `switch`) is detected by AST scan, the
  notebook is re-exported once **per value** at build time, and the outputs ship
  as a JSON lookup table that a small shim swaps on interaction. The reader gets
  a working slider. **There is no Pyodide, no `wasm-unsafe-eval`, no COOP/COEP,
  and no third origin** — this is the `interactive` class, and it can live
  beside the reports.
- **`wasm`** — islands or the export path; real Python in the browser;
  `compute` class, own origin, all of §7.1.

The rule that falls out is about *defaults*, not about capability:

> **Publish at the lowest class that answers the question — and `compute` is a
> first-class answer, not a fallback.** A figure driven by one slider with
> twelve steps is `interactive`: a build-time loop, no Pyodide, no third
> origin. A notebook where the reader edits an array, runs a fit, or explores
> something we did not anticipate is `compute`, and that case is a
> requirement rather than an escape hatch — it is the reason §7.1 exists and
> the reason the third origin gets built at all.

Both ship. Precompute is not a way to avoid solving WASM; it is a way to stop
*every* page paying WASM's cost. The build order does the WASM path at step 7,
before there is a VPS to configure, precisely so that "notebooks work under a
real CSP" is proven early rather than assumed.

Where precompute stops, stated honestly so nobody is surprised into a bad
build: it is a Cartesian product over discrete value sets, so *k* widgets with
*n* values each is *n^k* re-exports. Fine for one slider with twelve steps;
hopeless for four widgets, a continuous slider, a text input, or anything whose
value set is not statically extractable. The publisher should compute that
product up front, refuse above a configured ceiling, and say plainly which
widget to make discrete — or that this page wants `compute`. **A page that
falls off precompute's edge should land on WASM, not on a static screenshot.**

### 7.6 The isolation problem

Notebooks are code. Under `wasm-unsafe-eval` one notebook on a shared origin
can reach another's storage. Two answers, and the cheap one is right for now:

- **Now:** all notebooks on one `compute` origin. Everything on it was written
  by you or your agent, so notebook-versus-notebook isolation is not the threat
  model — the threat model is notebook-versus-*booking page*, and a separate
  registrable domain already solves that.
- **Later, if we ever host a notebook someone else wrote:** one subdomain per
  bundle (`<id>.compute.example.com`), wildcard TLS via a DNS-01 challenge —
  which is the only way to get a wildcard cert, and which also keeps the
  per-bundle names out of certificate transparency logs. Public Suffix List
  submission only if it becomes genuinely multi-tenant; do not block on it.

`--mode run`, never `--mode edit`, for anything published.

### 7.7 molab: a target, not the platform

Worth taking seriously, because it would delete the hardest section of this
document. **WASM is an artifact; molab is a session** — that framing is the
whole answer, and they are complementary rather than alternatives.

**What molab gives.** Free cloud-hosted marimo notebooks: 4 CPUs and 32 GB by
default, an optional RTX Pro 6000 Blackwell, real server-side Python so
arbitrary dependencies just work (no Pyodide, no wheel vendoring, no
`wasm-unsafe-eval`), GitHub mirroring, share-as-app and share-as-slides,
iframe embedding, and `marimo pair` for driving a live notebook from a coding
agent. For heavy or GPU work it is straightforwardly better than anything we
would build.

**What it costs, checked against this design's own requirements:**

| Requirement | molab |
|---|---|
| Private by default | **No.** "Public but undiscoverable", like a secret Gist |
| Expiry, revocation, per-recipient URLs | **None** — the three W3C TAG gaps P14 exists to close |
| Viewer cannot execute or copy | **No.** Anyone with the link can run it and fork it |
| Agent can publish unattended | **No documented API or CLI** |
| Versioned | **Not documented** — the property singled out as important |
| Permanent | 12-hour sessions, 90-minute idle shutdown; storage "limited" |
| Stated data terms | **Not documented** |

Two of those are decisive on their own. **No publish API means the `publish`
verb has no implementation**, which breaks the loop that started this entire
thread — a scheduled run produces something and gets a URL. And **"public but
undiscoverable" is not private**: for a notebook over unpublished data it is a
capability URL that never expires, cannot be revoked, and is identical for
every recipient.

**What it actually removes is smaller than it looks.** Only the `compute`
class. The gate origin, the artifact origin, inbound requests, verification,
the quarantine, the state machine and booking are all untouched — and even the
vendoring pass survives, because MathJax and Plotly are needed by the `static`
and `interactive` classes regardless of how notebooks are hosted.

**One property in its favour that is easy to miss, and it is a real one.**
Embedding a molab notebook by iframe means our CSP relaxes from
`script-src 'wasm-unsafe-eval'` — script execution on *our* origin — to
`frame-src https://molab.marimo.io`, execution on *theirs*. A cross-origin
iframe is a stronger isolation boundary than same-origin WASM. The price is
that the notebook must be public and molab sees every viewer, so this is right
for a methods demo and wrong for anything else.

**The decision: `target` becomes a bundle field**, `factory` (default) |
`molab` | `both`, with a rule that is easy to apply and hard to get wrong:

> **molab is for notebooks you would put in a public GitHub repository
> anyway.** If it cannot go in a public repo, it cannot go on molab.

So: unpublished data, anything an agent publishes unattended, anything needing
expiry or revocation, anything that must still resolve in three years →
`factory`. Teaching material, methods demos, reproductions of published work,
anything that wants a GPU → `molab`, and often `both`.

**`both` is the shape marimo-book already demonstrates**, and it is probably
the common case: publish the read-only rendering ourselves — permanent,
versioned, access-controlled, no third party — and put a *launch button* on it
pointing at molab for the reader who wants a live kernel. `launch_buttons.py`
already generates exactly that URL. The artifact stays ours; molab is the
escape hatch, not the record.

This does not change the WASM decision. A WASM bundle works forever, offline,
with no cold start, no session limit and no third party — which is what makes
it an *artifact*. molab is a live kernel with real compute behind it, which is
what makes it a *session*. We want both, for different notebooks, and the
build order is unchanged: the vendoring pass and the `compute` origin still
come before there is a VPS to configure, because they are what make a notebook
something you can send someone.

### 7.8 What to borrow from marimo-book, and what not to

`ljchang/marimo-book` (MIT, alpha, in production behind dartbrains.org) is a
static-site generator for marimo notebooks built on Material for MkDocs, with
zensical migration planned. It is the same author, the same problem, one layer
up — and it has already paid for a set of lessons this design would otherwise
pay for again.

**Borrow directly:**

- **The three render modes** (`static` / `wasm` / `cached`, per-entry override
  in `book.yml`). This is our content class, already implemented, already with
  a config surface someone has lived with. Take the semantics and the naming.
- **The precompute pipeline** (§7.5). It is the single largest reduction in how
  often we need the `compute` origin at all.
- **The content-hashed incremental build cache** (`rendered_store.py`): only
  changed chapters re-render. This is our content-addressed bundle version by
  another name, and a working implementation of it.
- **`checks.py`** as the skeleton for the publish-time gate. It already walks
  the built tree looking for problems; "no external reference survives" is one
  more check in a place that exists.
- **The anywidget shim and the `data:` URL rewrite** (§7.3) — the workaround is
  the borrowable part; our version writes files instead of importing data URLs.
- **The traps, which are the cheapest thing to inherit**: bound the render with
  a timeout so a hung notebook cannot stall a build; strip non-deterministic
  stderr before diffing outputs, or cache keys thrash; hoist the notebook's
  first `# H1` or the theme injects a second one; stage a copy of the notebook
  with PEP 723 injected rather than editing the original.

**Do not borrow:**

- **The CDN posture.** marimo-book targets GitHub Pages, where headers are
  impossible and a CDN is the only sane choice. We set headers, so we vendor.
  Same author, same tool, opposite constraint — and it is the reason §7.2 is
  work rather than configuration.
- **Material for MkDocs as the artifact shell.** Right for a 30-chapter book,
  heavy for a two-page report, and it drags a Python build chain into the
  publish path. The `report` template wants the *pipeline* — cells to HTML,
  precompute, anywidget mounts — not the theme.

**The integration shape**, then: `mecha-factory` never runs Python, and
neither does `mecha-factory-publish` for four of the six templates. The
`notebook` and `report` templates shell out to marimo / marimo-book **at
publish time, on the home machine**, inside the run's workspace, and what
crosses to the public box is a vendored, checked, immutable directory of bytes.
That keeps the public box a static file server plus one `write` endpoint, which
is the whole reason it is auditable.

---

## 8. The quarantine layer

Four layers, and the ordering is the design: each one below is only a backstop
for the one above.

### Where the split falls: the server filters shape, mecha filters meaning

The separate hosting server does solve a great deal, and it is worth being
precise about which half, because the boundary is also the reason the
quarantine lives at home.

**The server filters *shape*, for free, deterministically, with no model:**
which request type this is, that every field validates, that lengths and enums
and dates are in range, that the capacity and season and minimum-notice rules
pass, that the slot claim is atomic, that the sender verified an address, that
the rate limit holds. That is most of the defense in this design, it is
auditable by reading a few hundred lines, and it happens before a byte reaches
the house.

**The server cannot filter *meaning*.** A prompt injection is well-formed
UTF-8 inside a valid `text` field of correct length. No amount of structural
validation distinguishes "I'm applying to your lab because I loved your 2024
paper" from the same sentence with an instruction appended. Deciding what may
reach a privileged context is a judgement that can only be made where the
privileged context is.

And the decisive reason not to move it outward: **a model on the public box is
a provider key on the box we have agreed to assume is lost.** It would also
re-open the token faucet — an unauthenticated form that spends money per
submission — which is the failure the whole posture exists to avoid. Keeping
the extractor at home means it runs *after* verification has already gated the
spend, so only real requests ever cost anything.

So the layering is: the server is layer 0's enforcement point, and layers 1–3
below all run in mecha, on drained records that arrive `.from_outside()`.

### Layer 0 — structure (the actual defense)

The request type is chosen by the form. Fields are enums, dates, booleans and
length-capped text. Capacity, season and minimum notice are arithmetic. Nothing
a stranger types can change what kind of thing their request is or what happens
to it. This is the Action-Selector pattern, and it is doing ~all of the work.

### Layer 1 — a quarantined extraction pass

The part worth building, and it is CaMeL's dual-LLM shape at a size where it is
cheap:

```
  raw free text ──▶ extractor (no tools, no history, structured output only)
                        │
                        ▼
                    typed fields  ──▶ triage run (tools, context, drafts a reply)
                        │
  raw free text ────────┴──▶ shown to ME, never to the privileged pass
```

The extractor is a tool-less call with one job: turn 2000 characters of prose
into the schema's remaining typed fields — `topic`, `urgency_claimed`,
`dates_mentioned`, `institution` — plus a verbatim quote budget. It cannot call
tools, cannot see the conversation, and cannot emit anything that is not
schema-valid. **The privileged run that has calendar and mail access sees the
extraction, not the prose.** If the extraction is insufficient and the run
needs the original, that is a decision I make when reading the staged draft.

Two details that matter:

- The extractor's schema puts free-form reasoning **first** and the typed
  fields after, because constrained decoding degrades reasoning when the answer
  field precedes the thinking.
- An extraction failure is not a silent pass-through. It marks the record
  `extraction_failed` and routes to me with the prose unread by anything
  privileged.

### Layer 2 — detection, as a label and never a gate

Run a classifier (Prompt Guard 2-class, local, cheap) over every `untrusted`
field, and **write the score onto the record**. Show it in `mecha outbox show`
and in the queue listing.

It does not gate anything, and that is a decision rather than laziness. The
headline numbers are good — 99.8% AUC, 97.5% detection at 1% FPR — and the
deployment literature is consistently worse: substantial false negatives on
novel phrasings in independent evaluations, false positives on legitimate text
that merely *discusses* the attack class, and a body of work on evading these
guardrails specifically. A gate built on that would (a) reject a real student
who wrote something odd and (b) still pass the attack that mattered. As a
**label on a record a human is already reading**, it is free value with no
failure mode.

The corollary: **classification failure is fail-open, structural controls are
fail-closed.** If the classifier is down, the record is annotated `unknown` and
proceeds. If the *schema* validation fails, the record is rejected. Getting
that backwards is how a security layer becomes an availability problem.

### Layer 3 — the machinery that already exists

Everything drained is `.from_outside()`, so taint arms. The triage run is
read-only with the outbox routed. The interlock refuses `external_send` once
private and untrusted are both present. A reply is a staged draft I release.
Nothing here is new; the point is that layers 0–2 exist so that this last one
is not the only thing standing.

### What the model is never allowed to do with stranger text

A short list, worth having as tests:

- Resolve a URL, fetch an attachment, or send to an address that appeared in a
  free-text field.
- Decide a request's type, its priority tier, or whether consent exists.
- Move a request out of `awaiting_me`.
- Cause anything to be published.

---

## 9. Scheduling

The user wants both halves, in this order.

### 9.1 Book me

The one-sided page. `availability(windows, busy[], holds[], bookings[], now)`
is pure and deterministic; the page is a published bundle regenerated on a
trigger; the claim is an `O_EXCL` create at the origin (SQLite transaction in
`mecha-factory`, `holds/<slot>.hold` in the local store). A booking is a typed
request whose state machine has one extra deterministic step — the claim —
before `queued`.

`calendar_freebusy` on mecha-mail's unified surface is the prerequisite and is
useful on its own the day it lands: Google `freeBusy.query`, Graph
`getSchedule` (20 mailboxes, 62 days, chunked), fan-out across accounts,
intervals only, never `findMeetingTimes`.

### 9.2 Group availability, conditional on mine

The user's framing is better than when2meet's and makes the problem *smaller*,
which is worth stating plainly:

> **Participants never see a blank grid. They see the slots I can actually
> do.**

So the flow is:

1. I name the participants, a duration and a window.
2. The engine computes *my* candidate slots — real availability, buffers,
   minimum notice, per-day caps. For anyone whose free/busy is readable
   (`calendar_freebusy`), their busy time is subtracted too, with no poll.
3. The remaining candidates — typically five to fifteen, not a 7×24 grid — are
   published as a bundle behind a per-recipient capability URL.
4. Each participant marks yes / no / if-needed. That is the entire interaction:
   one screen, no account, no registration.
5. The trigger nudges whoever has not answered, closes when everyone has or the
   deadline passes, ranks by cost rather than by count, and stages the
   invitation.

Three properties fall out of the seeding that when2meet does not have:

- **The option set is bounded and always feasible for me**, so there is no
  "everyone agreed on a time I can't do".
- **The poll is small**, which is the difference between a colleague answering
  in ten seconds and not answering at all.
- **Tier 1 and tier 3 are the same code path.** If everyone's calendar is
  readable, step 2 leaves one obvious answer and steps 3–5 are skipped. If
  nobody's is, the poll is the whole thing. The mixed case — some readable,
  some not — is the common one and needs no special handling.

Ranking reports **feasibility, cost and fairness separately**, per CalBench: an
agent that always finds a time while quietly costing someone their afternoon
scores 100% on the only number most systems report. Surface the top three with
reasons — "Tuesday 2pm, but Tal moves something" is the sentence that makes
group scheduling work.

Not building: autonomous email negotiation, agent-to-agent handshakes, a
general calendar UI.

---

## 10. WebMCP — what it is, and the call

**What it is.** A W3C draft (Community Group Report, 2026-02-10) that lets a
page register tools with the browser: `navigator.modelContext.provideContext()`
with a name, a description and JSON Schemas for input and output. An agentic
browser — Chrome shipped it behind a flag in 146 Canary and enabled it for real
traffic by 149 — discovers those tools and calls them instead of driving the
DOM. `navigator.modelContext` is `[SecureContext]`, so HTTPS only, and the
browser asks the user for consent per site+agent pair.

**Why it is interesting here.** Our form already *is* a JSON Schema. A visitor
arriving with an agentic browser could have their agent fill and submit the
form from a conversation on their side of the boundary, on their tokens, and
hand us a schema-valid object. The conversational UX everyone wants, with no
model of ours on the request path.

**The risks, and why they mostly don't apply to us.** The reported hazards are
real: a third-party script can overwrite a registered tool and proxy every
call, tool descriptions can lie to the model, and a WebMCP tool is an
intent-level API handed to something reading untrusted content — the lethal
trifecta wearing a browser. Every one of those is about a page **with a session
and privileged actions**. Ours has neither: no login, no cookies, no
authenticated state, and the single "action" is submitting a form the server
independently validates. A public, unauthenticated, schema-validated intake
form is close to the safest possible WebMCP deployment. The third-party-script
risk is nil because we ship no third-party scripts.

**The call: design for it, do not ship it first.** Concretely — the manifest
emits the WebMCP registration alongside the form, the generator is written, and
it stays behind `webmcp = false` until one request type has been live for a
while. Reasons: it helps only the fraction of visitors on an agentic browser;
the spec is young enough to have open issues about tool-registration semantics;
and it can be turned on later with a config flag and no schema change. Nothing
about it is on the critical path, and nothing about deferring it costs
anything.

---

## 11. Deliberately not in v1

- **Compliance work** — deferred at the user's direction. Keep `retain_days`
  and the consent field in the manifest so turning it on later is a policy
  change rather than a migration. Revisit before the form is linked publicly or
  before the first non-US requester.
- **The research-question type.** It carries the *Moffatt* exposure, its
  usefulness is entirely a function of retrieval quality, and a static FAQ page
  may satisfy it. If it ships, it retrieves pointers from an approved corpus
  and never composes prose in real time.
- **Institutional identity** (SSO, ORCID). The enum value exists;
  nothing implements it.
- **MCP resources.** mecha's client speaks `tools/list` and nothing else, so
  artifacts-as-resources cannot be consumed by us or anyone. Worth fixing on
  its own schedule; the read-back path is the workspace mirror.
- **Per-notebook subdomains.** One `compute` origin until we host a notebook we
  did not write.
- **A phone UI for releasing outbox drafts.** The best argument for a home-side
  server, and a separate project.

---

## 12. Build order

Each step is useful alone, and the first four need no public box at all.

1. **`calendar_freebusy`** in mecha-mail. *Half a day.* Useful immediately.
2. **`mecha-manifest`**: the request-type and bundle types, the JSON Schema
   generator, the HTML form generator, the validators. Pure, unit-tested,
   renders to a file you can open. *Two days, and it is the architecture.*
3. **The availability engine.** Pure, unit-tested, DST in both directions.
   *Two days.*
4. **Mine twelve months of mail for the real request types.** Read-only,
   no new code, ends with evidence instead of a guess. *An afternoon* — and it
   should happen before step 2 freezes any field list.
5. **The bundle store, and a plain markdown `report`**, published to
   `tailscale serve <dir>`. No public box, no inbound, no origin decisions,
   no Python. Proves publish, versioning, aliasing and workspace read-back.
6. **The vendoring pass and the publish-time external-reference gate**
   (§7.2, §7.3), then `report` on marimo-book's static + precompute pipeline
   (§7.5). Before the notebook path on purpose: it covers the common case,
   needs no third origin, and the vendoring it forces is the prerequisite for
   WASM anyway.
7. **The `notebook` template** on `marimo export html-wasm`, and the `compute`
   origin's headers — verified locally under a real CSP before there is a VPS
   to configure. The step most likely to surprise us.
8. **Batch review in the outbox**, grouped by type.
9. **`mecha-factory`**: the four verbs, two scoped keys, SQLite, the three
   origins, the CSPs. The first step that creates a box to patch forever.
10. **Verification, the templated acknowledgment, and the state machine end to
    end**, with one request type. The quarantine layers land here.
11. **Booking**, then **group availability**.

Steps 1–8 are reversible. Step 9 is the commitment.

---

## 13. Open decisions

Things this document deliberately does not settle, and which need an answer
before the step that depends on them.

1. **Which domains.** Three registrable names are needed (gate, artifacts,
   compute) and none are chosen. Blocks step 8, nothing earlier.
2. **Which VPS, and who patches it.** The failure mode of forgetting is not
   "the site is down", it is "the site is someone else's". Unattended-upgrades
   plus a `GET /v1/health` check from a trigger is the minimum, and the health
   check should be a trigger that stages a warning rather than something you
   remember to run.

   Settled alongside it: **no CDN in front, to start.** A proxy that terminates
   TLS sees the plaintext of every request and response, which is the one thing
   "control our own infrastructure" is actually about. The binary does its own
   ACME (TLS-ALPN-01 for three fixed hostnames — DNS-01 only becomes necessary
   if per-bundle wildcard subdomains ever happen), sets its own headers so the
   CSP lives with the code and is unit-testable, and rate-limits in process
   because there is exactly one write endpoint. The cost is honest: no DDoS
   absorption. For a personal booking page that is an annoyance, not a crisis,
   and putting a CDN in front later changes nothing about the origin — which
   is the point of keeping it a plain program.
3. **Does `mecha-factory` render the booking page, or does it serve a published
   bundle?** Serving a bundle keeps the server dumber; rendering lets
   availability be fresher than the last publish. Probably: serve a bundle,
   with the *slot list* as a small JSON the server can refresh independently.
4. **How fresh is availability allowed to be?** A published page showing
   yesterday's slots is what youcanbookme does when its sync lags, and it
   degrades gracefully. Fifteen minutes is probably right; it is a trigger
   interval, not an architecture.
5. **Does the `question` type ship at all** (§11).
6. **Where the extraction pass runs.** Local model on `:8080` is free and
   private and is exactly the kind of small structured task it is good at;
   Anthropic is better at it and costs money per stranger. Probably local, with
   the failure path routing to me.

---

## Sources for the new material

- [WebMCP tool security — Chrome for Developers](https://developer.chrome.com/docs/ai/webmcp/secure-tools)
- [`provideContext` allows overwriting previously registered tools — webmcp#101](https://github.com/webmachinelearning/webmcp/issues/101)
- [The WebMCP Tools You Expose To Agents Can Be Used To Hijack Them](https://www.searchenginejournal.com/the-webmcp-tools-you-expose-to-agents-can-be-used-to-hijack-them/579204/)
- [WebMCP just landed in Chrome 146 — Bug0](https://bug0.com/blog/webmcp-chrome-146-guide)
- [The State of WebMCP, July 2026 — Spronta](https://www.spronta.com/blog/state-of-webmcp-july-2026/)
- [marimo — Self-host WebAssembly notebooks](https://docs.marimo.io/guides/publishing/self_host_wasm/)
- [marimo static export system — DeepWiki](https://deepwiki.com/marimo-team/marimo/6.3-export-system)
- [HTML WASM stand-alone page with CDN assets — marimo#3667](https://github.com/marimo-team/marimo/issues/3667)
- [WASM exports produce error when serving locally — marimo#3492](https://github.com/marimo-team/marimo/issues/3492)
- [Wasm-html on iOS: files missing when offline — marimo#5206](https://github.com/marimo-team/marimo/issues/5206)
- [Llama Prompt Guard 2 model card — PurpleLlama](https://github.com/meta-llama/PurpleLlama/blob/main/Llama-Prompt-Guard-2/86M/MODEL_CARD.md)
- [Bypassing LLM Guardrails: evasion attacks against prompt injection and jailbreak detection (arXiv:2504.11168)](https://arxiv.org/pdf/2504.11168)
- [When Benchmarks Lie: evaluating malicious prompt classifiers under distribution shift (arXiv:2602.14161)](https://arxiv.org/pdf/2602.14161)
- [Prompt injection classifier limits for AI agents — CodeIntegrity](https://www.codeintegrity.ai/blog/prompt-injection-limits)
- [Creating and managing ChatGPT Sites — OpenAI Help Center](https://help.openai.com/en/articles/20001339-creating-and-managing-chatgpt-sites)
- [ChatGPT Sites Terms — OpenAI](https://openai.com/policies/chatgpt-sites-terms/)
- [Wildcard TLS via DNS-01](https://www.alexcoorp.fr/en/patterns/tls-dns-01/)
- [`ljchang/marimo-book` — source; the render modes, precompute pipeline, islands/PEP 723 finding, anywidget `data:` URL workaround, and the enumerated CDN references all come from reading it](https://github.com/ljchang/marimo-book)
- [marimo-book documentation](https://marimobook.org/)
- [dartbrains — marimo-book in production](https://github.com/ljchang/dartbrains)
- [Run in the cloud with molab — marimo docs](https://docs.marimo.io/guides/molab/)
- [molab, now with GPUs — marimo blog](https://marimo.io/blog/reintroducing-molab)
