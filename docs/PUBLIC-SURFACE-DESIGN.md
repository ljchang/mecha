# The public surface — design

Decisions, not evidence. `docs/PUBLIC-SURFACE-RESEARCH.md` is the survey and
the argument; this is what we are actually building, settled 2026-08-05 after
the user reviewed it. Where a decision contradicts the research doc, this file
wins and the research doc keeps the reasoning.

**Still unbuilt.** This is the shape to build, written so someone can start.

---

## 0. What `mecha-factory` is for

**Two purposes, matching the two directions of one boundary:**

- **Publish what mecha makes.** Reports, dashboards, a morning briefing,
  marimo notebooks — as durable, versioned, permissioned URLs that can be read
  on a phone, sent to a collaborator, or read back by a later agent run.

  Today there is no answer at all, and the shipped example shows the shape of
  the gap: the morning-briefing trigger ends with
  `notify = "mkdir -p ~/.mecha/briefings && cat > .../$(date +%F).md"` — a
  shell one-liner dumping markdown into a directory it creates on the way past,
  **outside every path jail**, so no agent can read it back and there is
  nothing to send anyone. That is not a subsystem and should not become one;
  it is the absence of one. **A briefing is just a `report`-class bundle**, and
  making it an ordinary artifact — versioned, addressable, readable — is
  exactly what step 2 of the build order is for. It is the right first test
  precisely because it is small, daily, and already produces real content.
- **Build interfaces back into mecha.** A form is the default rendering, not
  the point. The point is that the outside world gets a *typed way in* —
  meetings, letters, applications, invitations — with schemas, state and
  deadlines instead of unstructured prose.

  This is what §2's "one manifest, six surfaces" is *for*, and it is worth
  saying as a purpose rather than as a mechanism: one request type emits the
  HTML form, the WebMCP tool, the MCP tool and the A2A skill, so **a human
  with a browser, an agent with a browser, an agent with MCP, and an agent
  doing discovery all arrive at the same typed object**. Adding a modality is
  a renderer, not a parallel system.

  And the corollary the mail evidence forced: **a form cannot be the only
  door.** Requests arrive by email, by a colleague forwarding one, and by a
  phone call someone typed up afterwards. So there must be a path that turns
  an inbound message into a typed request — mecha replies with the form link,
  or fills the request on the sender's behalf and asks them to confirm. A
  typed system whose only entrance is a URL will be routed around by everyone
  who does not know the URL exists.

The unifying claim, which is why these are one system rather than two: **a
typed, versioned, schema-described object crossing the boundary between the
user and the world, staged for human review in both directions.**

### How this relates to mecha's goals, since it is easy to conflate

Email responsiveness is a **goal of mecha**, not of `mecha-factory`. This
component *contributes* to it and does not deliver it:

- The inbound half reduces unstructured mail **at the source** — a typed
  request that meets a deterministic decline rule never becomes an email
  thread at all.
- A booking page collapses scheduling threads, but only if the link is *in the
  reply* rather than in a signature. Composing and sending that reply is
  mecha's job; the page is this component's.

So the largest near-term wins for responsiveness — suppressing machine mail,
digesting broadcast, drafting the remainder, batching the review — are **mecha
work and live outside this document**. `mecha-factory` should not be built
first if answered mail is the pressing problem. It should be built because the
artifacts have nowhere to live and the requests have no shape.

### Sections here that describe mecha, not the factory

The conversation that produced this document drifted across the boundary, and
these sections are mecha-side machinery that `mecha-factory` depends on or
forces. They are kept here because the reasoning is entangled, and flagged so
nobody looks for them in the wrong repository:

| Section | Actually lives in |
|---|---|
| §2.2b outbox review affordances for a publish | mecha — forced by this |
| §3.1 the autonomy ratchet, the question queue | mecha |
| §3.2, §3.3 tasks, deadlines, the `/tasks` modal | mecha + pkg |
| §8 layers 1–3 of the quarantine | mecha |
| §9 `calendar_freebusy`, the availability engine | mecha-mail + mecha |

Worth splitting into its own document once any of it is built.

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
| Frontend | **No framework where there is no reactivity; Svelte where there is.** Intake forms and the booking page are server-rendered HTML; `interactive` bundles and the admin UI are Svelte. | §5.1, §5.2. The line follows the content class, not taste. Svelte's build runs at home and adds nothing to the box. |
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

### 2.2a The tool surface, concretely

"`publish` and friends" was carrying too much. The surface is five tools, and
the capability declarations follow `mecha-mail`'s precedent exactly rather than
being invented here.

**This table is canonical.** §2.2c gives the reasoning behind four of the rows;
where the two disagree, this one is right.

| Tool | Does | `readOnlyHint` | `openWorldHint` | Capabilities | Outbox |
|---|---|---|---|---|---|
| `bundle_render` | template + source → a directory, locally | no | no | `private_data` | no |
| `bundle_publish` | upload a **rendered** bundle, get a version | no | **yes** | `external_send` | **routed** |
| `bundle_alias` | point a share URL at a version | no | **yes** | `external_send` | **routed** |
| `bundle_unpublish` | alias to nothing + private; destroys nothing | no | **yes** | `external_send` | **routed** |
| `bundle_fetch` | copy a published bundle from the **local mirror** into the workspace | **yes** | no | `private_data` | no |
| `bundle_list` | what is published, and at which version | **yes** | no | `untrusted_input` | no |
| `bundle_status` | one bundle: versions, alias, visibility | **yes** | no | `untrusted_input` | no |
| `type_push` | upload a request-type manifest + schema | no | **yes** | `external_send` | **routed** |

Two capability calls worth the sentence. `bundle_render` and `bundle_fetch`
are **`private_data` but not `untrusted_input`**: both touch bytes that are
ours — a source in the workspace, a mirror we wrote — so they carry the same
label `fs_read` does and do not arm the untrusted leg. The two `bundle_*`
reads that hit the *origin* do arm it, because that box is assumed lost.

Three things that fall out of the precedent and are worth stating so nobody
re-derives them:

- **`bundle_alias` is routed too.** Moving an alias changes what every existing
  share link resolves to — that is a publication, not a bookkeeping change, and
  it is the one people forget. A staged alias move shows both versions.
- **The reads are `untrusted_input` but not `openWorldHint`.** The query goes
  only to our own origin, which already holds the bytes — the same reasoning
  that puts `mail_search` on one side of that line and `http_fetch` on the
  other. But the origin is a box we have agreed to assume is lost, so
  everything it returns arrives `.from_outside()`. A returned URL is data to
  show, never a thing to fetch.
- **There is no `bundle_delete`.** Versions are immutable and the alias is the
  only moving part; unpublishing is `bundle_alias` to nothing plus a visibility
  flag. A delete verb would be the one operation that could destroy the record.

**The `[[mcp]]` block**, which needs one deliberate exception:

```toml
[[mcp]]
name = "factory"
command = "mecha-factory-publish"
sandbox = true
network = true          # it must POST; the global no-network default cannot hold
env_passthrough = []
env = { MECHA_FACTORY_URL = "...", MECHA_FACTORY_KEY_FILE = "~/.mecha/factory/publish.key" }
```

`network = true` on exactly one server is why per-server override exists — the
alternative would be giving `shell` the network so one server can reach its own
API. The key is passed as a **path, not a value**, so it never appears in an
environment dump or a crash log.

**And the confinement subtlety that is easy to miss:** mecha sees one MCP
server and confines *it*. The render subprocess lives **inside**
`mecha-factory-publish`, so mecha's sandbox cannot see it — which means the
"no network, no key while executing the notebook" claim of §2.3 is
`mecha-factory`'s job to enforce and to preflight, not mecha's. The rule
transfers with it: **a configured sandbox that does not work stops the run**,
because silently falling back to unconfined execution is worse than no sandbox.
If that is not implemented on the factory side, the confinement claim is
decoration.

### 2.2c Six corrections to the surface above

Brainstormed after writing §2.2a down. Five of these are things that table got
wrong or left out; the first is the one that improves the whole shape.

**1. `bundle_render` is a separate tool, and the trust boundary turns out to be
the workflow boundary.** *(Now row one of the table in §2.2a.)* §2.3 splits rendering from publishing because
rendering executes notebook code. The *workflow* wants the same split for an
unrelated reason: **rendering is cheap and publishing is expensive**, because
publishing costs a human review. Without a render tool, every iteration —
render, look, fix a chart, render again — is a staged outbox item somebody has
to reject. So:

| Tool | Cost | Routed | Takes |
|---|---|---|---|
| `bundle_render` | cheap, local, no network | no | a template + a source path |
| `bundle_publish` | one human review | **yes** | an already-rendered directory |

The agent renders, reads the output back with `fs_read`, fixes it, and publishes
once. That the security split and the ergonomic split land on the same line is
a good sign the line is real. It also simplifies `bundle_publish`, which no
longer needs to know what a template is.

**2. The URL is knowable at staging time, and the tool should return it.** A
staged call returns "drafted, not sent", which would normally mean the agent
cannot tell the user where the report will live. But versions are
content-addressed and the share URL is `/b/<id>/` — **both computable at home,
before the POST**. So the staged response carries the real final URL. Without
this, every published artifact needs a second conversation to find out where it
went.

**3. `target = "molab"` cannot mean what §7.7 implies, and that is my error.**
molab has no publish API — that was the finding that disqualified it as a
platform. So `target` cannot be a parameter that routes a publish. It describes
the bundle's *intended homes*: `factory` is the only automatable one, and
`molab` emits a launch-button URL plus a manual instruction for the human. Any
tool schema offering `molab` as a publish destination is offering something the
tool cannot do.

**4. `bundle_unpublish` should exist, for discoverability rather than
capability.** *(Now in the table in §2.2a.)* §2.2a says there is no delete and that taking something down is
`bundle_alias` to nothing plus a visibility flag. Correct, and useless: a model
asked to "take that down" will look for a takedown tool, not deduce it from
alias semantics. Name the composite. It still destroys nothing — versions stay
immutable and the ledger keeps the row — but it is findable, and a tool surface
a model cannot navigate is a tool surface that gets worked around.

**5. The vendoring gate must name the URLs it found.** "Vendoring failed" is
unactionable; `Ok(ToolOutput { is_error: true })` exists so the model can
recover, and recovery here means knowing *which* external reference survived.
The error lists every one, with the file and line. Same reasoning as every
other expected failure in this project: reserve `Err` for what the model cannot
route around.

**6. Cross-run read-back — resolved, two cheap pieces.** §6 mirrors bundles to
`~/.mecha/bundles/`, which no jailed run can read, and the workspace copy
belongs to the run that made it. So *this* run can read what it published and a
*later* run cannot — the question that started this entire line of work,
answered for one hop and not two.

**Stable per-trigger workspaces**, which need no new code at all: `workspace`
is already a field on `Trigger`, written at `add` time and persisted in the
TOML. Nobody has set it. Setting it makes yesterday's report an ordinary file
in today's run, and it simultaneously fixes a live hazard — see the note below,
which is the more urgent half of this finding.

**`bundle_fetch` for everything else** — across triggers, from a chat session,
and version-addressed ("diff this against what I published Monday"). Two
properties make it safe rather than a hole in the jail:

- **It copies from the local mirror into the current workspace, and the model
  names a bundle id, never a path.** The tool resolves id → mirror path
  internally, so `ToolCtx::resolve` is never handed a path that could escape.
  Same pattern as *the model names an account, never a provider*.
- **The mirror, not the origin.** If it fetched from the server, an agent
  reading *its own last report* would get it back `.from_outside()` — the box
  is assumed lost — arming the untrusted taint leg every time anything looked
  at its own work. The local mirror is trusted, so it does not.

> **Live hazard, and not really about artifacts at all.** A trigger's
> `workspace` is `Option<PathBuf>`; when unset, `setup::prepare_tools` falls
> back to `std::env::current_dir()`, and the daemon's unit sets
> `WorkingDirectory=%h`. So **a trigger with filesystem tools and no explicit
> workspace is path-jailed to `$HOME`, which contains `~/.mecha/`** — mail
> OAuth tokens, every session transcript, the learning store. The shipped
> `morning.toml` is safe only by accident of its tool allowlist, not by the
> jail.
>
> It is broader than triggers: running `mecha chat` from your home directory
> has the identical shape. The interlock is still a backstop — reading a token
> arms `private_data`, and exfiltration needs an `external_send` the interlock
> refuses — but a jail rooted where the secrets live is close to no jail, which
> is the silently-degrading-sandbox pattern this project keeps naming.
>
> Two fixes, and the first delivers stable read-back for free: **default a
> trigger's workspace to `~/.mecha/workspaces/<name>/`**, created at `add`
> time, durable across runs and containing nothing sensitive. And **refuse in
> `setup` any workspace that contains the mecha home**, which catches the
> interactive case and every future variant rather than this one instance.

And one smaller thing, recorded rather than solved: **staging takes no lock**,
deliberately, so a retried tool call stages twice. Content-addressing makes the
duplicate detectable — identical bytes are the same version — so review should
collapse duplicates rather than the agent trying not to create them.

### 2.2b A staged publish is not a staged email, and the outbox assumes it is

A real gap, found by writing the surface down rather than by using it.

`mecha outbox` was built for messages: `show` prints the args, `edit` opens
`$EDITOR` on them, and `diff(args_before, args)` of sent-with-edits items is
mined into **`writing`-domain reflections**. Every one of those assumptions
breaks on a publish:

- **`show`** would print `{bundle_path, id, target, visibility}`. That is not
  reviewable. The reviewable object is *the rendered page*, and the reviewer
  needs to open it.
- **`edit`** on those args means changing a filesystem path or a visibility
  flag. It does not mean editing the draft. Editing the content means editing
  the source and re-rendering.
- **The writing miner would learn from path edits.** This is the
  "Blocked by a hook:" mistake in a new costume — the miner keys on the shape
  of an edit, and feeding it a changed directory path teaches voice rules from
  noise, into the cached prefix of every future run.

So, three decisions:

- **`show` on a publish prints the local preview path** from the workspace
  mirror (§6) and the bundle's diff against the currently aliased version —
  "what would change for a reader" — rather than the args. Review means opening
  the page.
- **`edit` on a publish is refused**, with a message naming the real action:
  edit the source and re-render, which stages a new item. Simplest, honest, and
  it avoids inventing a re-render-from-the-outbox path nobody asked for.
- **The writing-reflection miner filters on item kind.** Publishes are excluded
  from `mined_outbox.jsonl` entirely. There is a test for the string
  `"Blocked by a hook:"` for exactly this reason; this wants the same.

The general lesson, worth keeping: **the outbox generalised to a new sink
without changing, which was the design goal — but its *review* affordances did
not.** Staging is sink-agnostic. Reviewing is not.

> **Built 2026-08-06**, all three, ahead of anything that stages a publish —
> deliberately, because the miner is the one that does damage retroactively.
> `OutboxKind` (`message` | `publish`) rides on the item, defaulted on load;
> the kind comes from `[outbox] publish_tools` in config, never from the tool,
> since the loop must not learn what a publish is and an MCP server cannot be
> trusted to say. `show` leads with the bundle directory and the file to open,
> `edit` is refused with the real action named, and the miner filter lives on
> `OutboxItem::mineable_as_writing` so it is a property of the type with a test
> on it. Naming the publish tools in `[outbox] publish_tools` is therefore part
> of wiring the factory MCP server, not an afterthought.

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

### 3.1 The autonomy ratchet

The stated goal of the whole project is that this **gets easier over time** —
that mecha gets what it needs from the user and handles the rest. That is a
design requirement, not an aspiration, and it decomposes into two mechanisms
plus a set of floors.

**Ask about policy, not about items.** The elicitation that makes this work is
not "approve this draft" repeated forty times; it is one question whose answer
governs a class. The mail evidence is unusually clear here: **a handful of
booleans resolve most decisions across the four highest-volume request types**
— a travel constraint, a standing recurring commitment, a
conflict-of-interest check, and a capacity-and-funding flag. Asked once, a
single capacity answer disposes of a year of lab-join requests. (The specific
values are the user's private policy and live with the evidence, not here.) That ratio — one answer, many decisions — is the
thing to optimise, and it is why the typed request matters: a policy answer
can act on a typed field and cannot act on free prose.

This needs somewhere to put a question. **`ask_user` is absent from unattended
runs by construction** — it is only ever registered by a front-end that owns a
human — so a trigger cannot ask anything today; it can only stage.

**It should not be a third queue, and it should not be called an inbox.**

*Not an inbox*, for two reasons. In a system whose primary job is triaging
mail, `inbox` is the most overloaded word available — `mecha inbox` would be
ambiguous with the user's actual inbox, in exactly the context where the
ambiguity costs the most. And it imports the connotation the project exists to
fight: an unbounded pile you dread opening. This queue's *defining* property is
that it is **capped**. The symmetry with `outbox` is also weaker than it looks:
an outbox item is complete and needs approval; a question is incomplete and
needs an answer. Different verbs.

*Not a third queue*, because the right home already exists. **`mecha
proposals`** is the learning system's gate — mecha proposes a rule, the user
accepts or declines, and future behaviour changes. A policy question is the
same shape and the same review cadence, and §3.1 already says autonomy
graduation should go "through a gate the user accepts, exactly as learned rules
are". If that is true it should go through the *actual* gate rather than a
parallel one.

So the surface is **two queues, and they split on a clean line**:

| Queue | Holds | Verb |
|---|---|---|
| **`mecha outbox`** | things that would leave the machine | approve / edit / reject |
| **`mecha proposals`** | things that would change how mecha behaves | accept / decline |

`proposals` gains kinds alongside the existing `rule` and `retirement`:
**`policy`** (a question whose answer governs a class) and **`autonomy`**
(graduate a category to auto-send). Two surfaces is learnable as a morning
routine; three is one too many.

The hard rule stands wherever it lives: **a question must name the class it
unblocks and how many pending items it would resolve.** A question that unblocks
one item is a draft, not a question.

**Autonomy is earned by measurement, per category, and revoked by evidence.**
This is the project's existing philosophy applied to a new subject: *acceptance
is not tenure*. A category that has been approved without edit N times running
is a candidate for auto-send — **proposed through a gate the user accepts**,
exactly as learned rules are, never a config flag someone sets optimistically.
One edit or one rejection drops it back to reviewed immediately. The
validation-ledger machinery already does the measuring half of this for
learned rules; the shape transfers without inventing anything.

**Floors that never graduate**, because "handles the rest" is the highest-trust
operation in the system:

- A first message to a **new correspondent** stays reviewed. Autonomy is per
  category *and* conditioned on an existing relationship.
- Anything drafted with the **trifecta armed** stays reviewed. The outbox
  already records the taint snapshot on every item, so this is a check rather
  than new machinery.
- Anything containing a **commitment** — a date, a promise, a letter, a
  recommendation, an agreement to serve — never graduates. Declines and
  acknowledgments can; obligations cannot.
- A category whose recent **edit rate** is non-zero is not a candidate,
  regardless of streak length.

**What the user sees each morning**, and the shape of it is how you tell
whether this is working:

```
questions   n, each unblocking a named class      ← rare, high value
drafts      m, batched by type                    ← should shrink over time
handled     k, with a ledger row each             ← should grow over time
```

If `drafts` is not falling and `handled` is not rising over months, the ratchet
is not ratcheting and the system is a nicer inbox rather than a smaller one.
**The metric is the fraction of messages that never needed the user** — never
drafts produced, which is a vanity number that rises as the classifier gets
worse.

**And the invariant that keeps the whole thing honest: cap the review queue.**
If more than a set number of drafts are pending on any morning, that is a bug
in the classification, not a demand on the user. Overload is what produced the
silence in the first place; a design that relocates the pile into a prettier
queue has solved nothing.

### 3.2 Tasks and deadlines, and why they are the load-bearing part

Nothing durable tracks these today. `~/.mecha/` holds briefings, learning,
mail, outbox, sessions and triggers; there is no task store, and the `todo`
tool is an in-run scratchpad that dies with the run.

**The reason this matters more than it looks: a deadline is what makes silence
detectable.** Today an unanswered message is *invisible* — there is no artifact
anywhere that says "you owe this person a reply and it has been eleven days."
Nothing can surface it, escalate it, or close it out, because there is no
object to hang a state on. Give the obligation a due date and silence stops
being an absence and becomes a **state** — which can be shown in the morning,
escalated when it ages, and eventually auto-declined with dignity rather than
left to rot. That is the mechanism by which this whole system attacks the
actual problem, and it is why tasks are not a side feature.

**Three sources, and the second is the one nobody has:**

- **Inbound requests.** The manifest's `sla_days` generates the due date, so
  every typed request carries a deadline without anyone typing one. Already
  modelled by §3; this just names the derivation.
- **Commitments the user made.** "I'll send you the draft next week" in a sent
  message is a task, and nothing records it. Extractable from Sent Items and —
  more reliably — from **released outbox items**, where mecha already knows
  exactly what went out because it staged it. This is the distinctive
  capability here and it is squarely on the stated goal: a dropped promise is
  the same failure as an unanswered message, one turn later.
- **Direct capture.** The user says so.

**Stored in pkg, which is the right home and was nearly rejected for a bad
reason.** An earlier draft of this section said tasks must not live in the
knowledge graph because mecha reads pkg back through the `untrusted_input`
override and "escalation logic must not run on untrusted data". That
overstates it. Every send in this design is outbox-routed, so the blast radius
of a poisoned task is **a draft in a review queue**, not an action — which is
the same containment the whole system already rests on. And pkg is not an open
sewer: its extractor turns content into *candidates that wait in the user's
review queue*, so entries are already human-gated on the way in.

What pkg buys is the thing a flat store cannot: **tasks are inherently
relational.** A letter deadline belongs to a student; a grant report belongs to
a grant, a program officer and a budget line; a review belongs to a journal and
possibly a conflicted author. That is a graph, and modelling it as rows in a
JSON directory throws away exactly the structure that makes "what breaks if
this slips" answerable. This is not wired into mecha today — `kg_upsert` is
used by `distill` for episodes and nothing reads tasks back — so it is new
work, but it is new work in the right place.

Three consequences to accept deliberately rather than discover:

- **Tasks arrive `.from_outside()` and arm the untrusted taint leg**, because
  the capability override is per-server and only ever widens — correctly, and
  it should not gain a per-record exception. In practice this changes little:
  any run that reads tasks is also reading mail, so the trifecta was already
  armed and every send was already staged.
- **Carry an `Origin` on every task**, exactly as reflections do:
  `user` (typed by the user, in the TUI or the CLI), `derived` (extracted from
  a message), `external` (created by anything else). **Only `user`-origin tasks
  drive escalation unattended**; a `derived` task must be confirmed once before
  it can chase anybody. That is the provenance discipline the learning store
  already uses, applied to a new subject, and it is the real answer to the
  concern the earlier draft was reaching for.
- **pkg is a separate process**, so tasks are unavailable when it is down and
  every read is an MCP round trip. If the TUI's list feels slow, the fix is a
  short-lived local cache with pkg as the record — never a second source of
  truth, for the same reason `/triggers` reads its detail view back from the
  session transcript.

### 3.3 `/tasks` in the TUI

The user wants to see open tasks and deadlines, add them, and complete or
remove them, without leaving the terminal.

**It follows `/triggers` exactly: the modal drives the CLI, not the store.**
Every action shells out to `mecha task ...` as a child process. The reasoning
transfers and is doubly true here — `/triggers` does it because firing a
trigger can run for twenty minutes and would freeze the event loop, and tasks
do it because the store is behind an MCP round trip to another process. Going
through the CLI also means one implementation, and no way for the TUI to do
something the command line cannot.

So `mecha task` is the primitive and the modal is a view over it:
`add`, `list`, `done`, `rm`, `defer`, with `list` sorted by deadline and
showing what breaks if it slips. The panel is the `due` line of the morning
surface (§3.1), rendered live.

Two details worth fixing now because they are cheap and annoying to retrofit:

- **A derived task shows its source**, and the modal can open it. This is what
  makes the list trustworthy: a task the user does not recognise is one
  keystroke from the message that produced it.
- **Deleting a derived task records a negative example** rather than silently
  forgetting, so extraction gets better instead of repeating the same mistake
  every week.

**Recurrence reuses `cron.rs`.** Annual reports, term-bound obligations, a
review load that resets. The parser is already hand-rolled precisely because
every crate spoke Quartz's dialect, and it is already correct across both DST
directions in a named IANA zone. Inventing a second recurrence model here would
be re-answering a question this project already paid for.

**The daily surface gains a line, and it goes first:**

```
due          sorted by deadline, each with what breaks if it slips
questions    policy questions, each unblocking a named class
drafts       batched by type, to approve
handled      one ledger row each
```

This is also what makes one of the mining findings load-bearing rather than
incidental: **the hard deadline, and what breaks if it slips, is missing from
almost every inbound request.** It is a required field precisely because it
feeds this, and "what breaks" is what lets the morning list sort by
consequence instead of by date.

**One caution, because it decides whether this gets used.** A task list you do
not trust is worse than no task list — one hallucinated commitment and the user
stops reading it. So extraction is conservative by default, every derived task
**links to the message it came from**, and a derived task the user deletes is
recorded as a negative example rather than silently forgotten. Grade the
artifact, not the model's confidence in it.

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

Plain HTML and vanilla JavaScript for the *forms*. Not a blanket rule — §5.2
draws the line — but for intake it is the simpler option rather than a
compromise: there are five forms here, not an application, and a form has no
reactive state to manage.

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

### 5.2 Where reactivity earns a framework

The rule, and it is a property of the work rather than a preference:

> **Reactivity is a property of the content class.** Where the page is a form
> — a bounded set of fields, submitted once — there is no reactive state to
> manage and a framework buys nothing. Where the page is a *view over data the
> reader manipulates*, there is, and Svelte is the right tool.

| Surface | Reactive? | Stack |
|---|---|---|
| Intake forms (letter, speaking, lab, question) | no — linear entry, conditional show/hide | HTML5 constraints + a small evaluator |
| Booking page | no — slots are radio buttons; zone rendering is one `Intl` call | same |
| Group availability | no — 5–15 slots × yes/no/if-needed | same |
| `static` bundles (reports, posts) | no — nothing executes | none |
| **`interactive` bundles** (dashboards, linked charts, filters) | **yes** | **Svelte** |
| **Admin UI** (release an outbox draft from a phone) | **yes** | **Svelte**, when it is built |
| `compute` bundles (marimo) | yes, by Pyodide | not ours |

Two borderline cases, named so nobody re-argues them by accident. A booking
page grows a month-grid calendar with keyboard navigation → revisit. A group
poll grows a when2meet-style drag-select grid → revisit. Neither is in v1, and
both are genuinely reactive if they arrive.

**Why Svelte specifically, on the three axes that matter here:**

- **Performant.** It compiles to imperative DOM updates with a runtime measured
  in a few kilobytes. That matters more than usual because every byte in a
  bundle must be *vendored* — there is no CDN to amortise a framework across
  pages.
- **Reliable, in the specific sense this project needs: a bundle published
  today must still render in three years.** A compiled, self-contained bundle
  with no runtime dependency to stay compatible with is more durable than one
  that ships a framework expecting its own ecosystem. Compile-away is an
  artifact-permanence property, not just a size one.
- **Secure.** Svelte escapes by default and `{@html}` is the single opt-out —
  forbid it in our templates and lint for it. The compiled output contains no
  `eval` or `new Function`, so `script-src 'self'` holds with no
  `unsafe-inline`. One real gotcha: scoped styles emit `<style>` tags unless
  CSS is extracted at build, which would force `style-src 'unsafe-inline'`.
  **Extract CSS to a file**; it is a build flag and it keeps the strictest
  style policy intact.

And the reassuring part: **adding Svelte adds Node to the home-side build, not
to the VPS.** It is the same shape as marimo — a build-time subprocess whose
output is a directory of bytes that the vendoring gate checks like any other.
Nothing on the box changes, and the "assume the public box is lost" inventory
is untouched.

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

### 6.1 Two directories, and the scratch one solves three problems

Generated output needs somewhere to go that is **not** the published store, and
today it goes wherever a `notify` shell command improvises. The fix is a
designated, cleanable directory — and it turns out to be the same directory
that answers two other open items, which is a good sign it is the right shape.

```
~/.mecha/work/<producer>/       generated · mutable · disposable · cleanable
~/.mecha/bundles/<id>/<ver>/    published · immutable · versioned · never deleted
```

`<producer>` is a trigger's name, or `chat`, or a session id. **This is also
the run's workspace**, which is what makes it worth more than tidiness:

- **It fixes the jail default.** A trigger with no explicit workspace currently
  jails to `$HOME`, which contains the mail tokens and the learning store
  (`HANDOFF.md`, Triggers). Defaulting it to `~/.mecha/work/<name>/` roots the
  jail somewhere containing nothing sensitive.
- **It fixes cross-run read-back within a producer.** Yesterday's briefing is an
  ordinary file in today's run, because the directory is stable and named.
  `bundle_fetch` (§2.2c) still handles the cross-producer and version-addressed
  cases.
- **It gives `notify` something better to be.** The morning trigger's
  `mkdir -p ~/.mecha/briefings && cat > …` exists only because there was no
  designated place. With one, the trigger writes into its workspace like any
  other run, and publishing is `bundle_publish` rather than a shell redirect.

**Retention has to be a policy, not an intention.** The lesson of this entire
project is that anything without one becomes a pile nobody opens. So: keep the
last *N* per producer, a `mecha work clean` that says what it removed, and one
hard rule — **never delete anything a published bundle's source references**,
because "regenerate last week's report" must not silently lose its input.

> **Built 2026-08-06** (mecha-side, `mecha-core/src/work.rs` + `mecha work`),
> with *N* = 10 as `[work] keep`. Two notes for the publisher, which is the
> other half of the hard rule:
>
> - **The protection contract is one field of data**, not a shared type, for the
>   same reason the manifest is: a mirrored version directory
>   (`~/.mecha/bundles/<id>/<ver>/`) may carry a `bundle.json` with a
>   `"sources": ["<absolute path>", …]` array naming what it was rendered from,
>   and `clean` reads exactly that. Everything else in the file is
>   `mecha-factory-publish`'s business. **If the publisher does not write
>   `sources`, the rule protects nothing** — the mechanism is in place and its
>   input is the publisher's job to supply, which is worth knowing before
>   assuming a source is safe.
> - `clean` counts **entries**, not files, because a rendered bundle is a
>   directory. It reports protected survivors by name rather than skipping them
>   silently: an entry that outlives its own retention window reads as a bug in
>   the sweep unless the reason is on screen.

**A note on the name.** `artifacts/` is the obvious word and the wrong one
here: this document uses *artifact* throughout to mean a **published** thing,
and a directory of unpublished scratch called `artifacts/` sitting next to
`bundles/` would invert that every time someone read it. `work/` matches
*workspace*, which is what it actually is. Worth deciding deliberately rather
than inheriting.

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

> **Verified in a browser 2026-08-06**, which is what §12 step 4 asks for, and
> the result is that this policy stands — with one addition and one correction
> to how it was reasoned about.
>
> Served under exactly the policy above, a real `marimo export html-wasm`
> produced **six violations and did not boot**: four blocked inline scripts, a
> blocked `data:` font, and a blocked `eval`. Then:
>
> - **`font-src 'self' data:`** — marimo embeds a woff2 inline. A font that is
>   already in the page is not a fetch, so this grants no reach it did not have.
> - **The four inline scripts are extracted into files**, not hashed. Both work;
>   hashes would make the policy **per bundle**, and a CSP that varies per
>   artifact is one nobody can state, test, or check by looking. Extraction
>   keeps `script-src 'self'` exact for every compute bundle.
> - **`unsafe-eval` is *not* needed, and the rule below survives.** The blocked
>   `eval` is zod's memoized feature probe —
>   `try { return Function(""), true } catch { return false }` — which detects
>   that dynamic evaluation is unavailable and takes a slower path. The browser
>   reports a violation; the code degrades gracefully. Worth recording precisely,
>   because "a CSP violation appeared" and "the page is broken" look identical in
>   a console and are not the same thing.
>
> With those two changes the page reports **one** violation (that probe), **zero
> off-origin loads**, and fails at exactly one remaining place: Pyodide, which
> `connect-src 'self'` correctly refuses to let it fetch. The harness is
> `factory-publish serve` plus `scripts/csp-probe.py` in `mecha-factory`.

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

> **Measured 2026-08-06, and the table above is about the *islands* path.** A
> real `marimo export html-wasm` (marimo 0.23.16, a four-cell notebook) was
> exported and scanned. What it actually produces:
>
> - **710 files, 27 MB**, with marimo's own frontend assets copied in locally.
>   `index.html` is **clean** — zero external references at the markup level.
> - **Pyodide is not vendored.** There are *no* local Pyodide files at all. Two
>   minified workers (`worker-*.js`, `save-worker-*.js`) hardcode
>   ``indexURL: `https://cdn.jsdelivr.net/pyodide/${version}/full/` `` **and**
>   `lockFileURL: https://wasm.marimo.app/pyodide-lock.json?…`. So under
>   `script-src 'self'` / `connect-src 'self'` **the notebook does not boot.**
>   marimo's own help text calls the export "completely self-contained"; with
>   respect to Pyodide it is not.
> - **There is no configuration hook.** `packageBaseUrl` exists as a parameter
>   and would win over the CDN fallback — but marimo sets it to the CDN literal
>   itself, so it is unreachable from outside. Vendoring is a string
>   substitution in two minified files, plus shipping the pinned Pyodide dist
>   and lock file. Filed as work with a known shape rather than a risk.
> - Also CDN: `mathjax-full@3.2.2` and `lucide-static@0.452.0` icons.
> - **The export is `marimo/_static/` copied wholesale** — all eleven files,
>   byte-identical — with `index.html` rewritten and `.nojekyll` added. So a
>   published notebook carries marimo's `CLAUDE.md`: a 367-line prompt telling
>   an agent how to *edit* marimo notebooks, alongside a page exported
>   `--mode run` that nobody can edit. Nothing references it — it is the one
>   file in the export with no inbound references at all. Drop it, and drop
>   `.nojekyll` with it (a GitHub Pages marker, meaningless here).
>
>   Worth stating the real reason rather than the tidiness one: it is
>   **instruction-shaped text served from an origin we hand to
>   correspondents**. marimo's file is plainly benign; the bad default is a
>   published artifact containing imperative AI instructions we did not write
>   and did not read, on the one surface whose whole posture is *assume the
>   public box is lost*.
>
>   **But the prune has to follow references transitively, not scan the entry
>   point.** Measured: `logo.png` is unreferenced by `index.html` and
>   referenced by two assets; `android-chrome-192/512.png` are unreferenced by
>   `index.html` and named in `manifest.json`. A naive "delete what the page
>   does not mention" breaks both. So the `notebook` template prunes to a
>   **declared list** rather than to whatever the exporter happened to leave —
>   a published bundle contains what we meant to publish, which is the same
>   argument the vendoring gate makes one layer down.
>
> - `manifest.json` declares `"short_name": "Marimo"`, `"name": "A Marimo
>   App"`. Installed to a home screen a published notebook would be called *A
>   Marimo App*; it wants rewriting to the bundle's own title at publish time.
>
> - **Pinning the asset tree hides the Pyodide problem from the gate**, since a
>   pinned tree is not walked. The `notebook` template therefore needs its own
>   check that the runtime is actually vendored — the general gate cannot be
>   the thing that catches it, and a notebook that passes the gate and cannot
>   boot is precisely the failure the gate exists to prevent.

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

The verification recipe was the starting point, and running it on a real bundle
is what showed why it cannot be the gate:

```bash
grep -rIoE 'https?://[^"'\''` )]+' "$bundle" | sort -u    # must be empty
```

> **Measured 2026-08-06.** On the export above that recipe yields **541 hits,
> 224 distinct URLs**, of which **234 are XML namespace identifiers**
> (`http://www.w3.org/2000/svg` and friends — declarations, never fetched),
> most of the rest are documentation and attribution links, and roughly **30**
> are genuinely fetchable at runtime (jsdelivr, `basemaps.cartocdn.com`,
> `fonts.openmaptiles.org`, `mapbox.com`, buried in charting libraries). A
> check that reports 541 things nobody will read is not a check.
>
> **So the gate has two modes**, and the split is between kinds of object
> rather than kinds of URL:
>
> - **Files we emit** — strict, zero tolerance, every finding named by file,
>   line, URL and reason. A link (`<a href>`) is never a finding; anything the
>   page *fetches* on load is.
> - **A vendored third-party tree** — the unit of review is the **tree**. It is
>   declared with a digest, reviewed once at the version pinned, and not walked
>   line by line; **the CSP is the runtime enforcement**, which is what §7.1 was
>   always for. `connect-src 'self'` means a map-tile fetch inside a charting
>   library simply fails.
>
> Fail-closed in both directions: an *undeclared* subtree is scanned strictly,
> so nothing becomes vendored by being forgotten, and a declared tree whose
> digest no longer matches is a finding rather than a pass — otherwise
> "reviewed once" quietly means "reviewed once, then never again". And because
> a pin is a claim rather than a conclusion, `check` prints what was pinned
> beside the verdict instead of absorbing it: the marimo tree above passes the
> gate while still being unable to boot under the CSP, and a report that read
> simply "self-contained" would be the wrong thing to believe.
>
> Built and verified against the real 710-file export.

### 7.3 The `data:` URL problem — which turns out not to be ours

> **Corrected 2026-08-06 by measurement.** Everything below is true, and it is
> a property of **`MarimoIslandGenerator`**, which runs under
> `ScriptRuntimeContext` with `virtual_files_supported=False`. A real
> `marimo export html-wasm` was scanned for `data:` URLs in script positions
> and contains **none**. marimo-book hits this because it embeds islands into
> MkDocs pages; we publish standalone bundles and take the export path, so the
> anywidget shim and the data-URL rewrite are **work we do not have to do**.
> Kept in full because the reasoning is what tells us the two paths differ, and
> because an islands path would resurrect it exactly.

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

**Scoped to `mecha-factory`.** The mecha-side prerequisites are noted where a
step depends on one, but their own ordering belongs in `HANDOFF.md`.

**Where this sits overall:** if answered mail is the pressing problem, the
mecha-side suppression and drafting work (§0) comes first — none of it needs
this component. Build the factory because artifacts have nowhere to live.

1. **`mecha-manifest`**: the request-type and bundle types, the JSON Schema
   generator, the HTML form generator, both validators. Pure, unit-tested,
   renders to a file you can open. Seeded from `pbs_knowledge`'s `FormConfig`
   (§5.1) and from the mail evidence, which is now gathered. *Two days, and it
   is the architecture.*
2. **The bundle store and a plain markdown `report`**, published to
   `tailscale serve <dir>`. No public box, no inbound, no origin decisions, no
   Python. Proves publish, versioning, aliasing and workspace read-back.
3. **The vendoring pass and the publish-time external-reference gate**
   (§7.2, §7.3), then `report` on marimo-book's static + precompute pipeline
   (§7.5). Before the notebook path on purpose: it covers the common case,
   needs no third origin, and the vendoring it forces is the prerequisite for
   WASM anyway.
4. **The `notebook` template** on `marimo export html-wasm`, and the `compute`
   origin's headers — verified locally under a real CSP before there is a VPS
   to configure. The step most likely to surprise us. *Scoped by measurement
   2026-08-06 (§7.2, §7.3): the anywidget/data-URL work is **not needed** on
   this path, and the real work is vendoring Pyodide — two string substitutions
   in minified workers, plus the pinned dist and lock file, plus MathJax and
   the icon set. `export html-wasm` over islands, because only it reads PEP
   723.*
5. **`bundle_fetch` and stable trigger workspaces** (§2.2c), which is what
   makes "read back what a previous run published" true across runs rather
   than within one. *The mecha-side workspace fix shipped 2026-08-06, so the
   trigger-workspace half of this step is already done — what remains is
   `bundle_fetch` for the cross-producer and version-addressed cases.*
6. **`mecha-factory`**: the four verbs, two scoped keys, SQLite, the three
   origins, the CSPs. **The first step that creates a box to patch forever.**
7. **Verification, the templated acknowledgment, and the state machine end to
   end**, with one request type. *Depends on the mecha-side quarantine layers
   and on batch review in the outbox.*
8. **The booking page**, and then **group availability seeded by the user's
   own**. *Depends on `calendar_freebusy` and the availability engine.*

Steps 1–5 need no VPS, no domains and no origin decisions, and they deliver the
publishing half — which is the half with no alternative today. Step 6 is the
commitment.

**Done:** mining twelve months of mail for the real request types
(2026-08-05). The evidence lives outside this repository; its conclusions are
folded into §5.1 and into the mecha-side sections listed in §0.

## 13. Open decisions

Things this document deliberately does not settle, and which need an answer
before the step that depends on them.

1. **Which domains.** Three registrable names are needed (gate, artifacts,
   compute) and none are chosen. **Blocks build step 6, nothing earlier.**
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
3. ~~**Is the scratch directory `work/` or something else** (§6.1), and what is
   *N* in "keep the last N per producer"?~~ **Settled 2026-08-06 and built:**
   `work/`, on §6.1's argument, and *N* = 10 — enough to hold a week and a half
   of a daily producer, so both "what did yesterday say" and "what changed since
   Monday" survive. Tunable as `[work] keep`, and still wanting a week of real
   output to confirm; what mattered was that the sweep exists at all.
4. **Does `mecha-factory` render the booking page, or does it serve a published
   bundle?** Serving a bundle keeps the server dumber; rendering lets
   availability be fresher than the last publish. Probably: serve a bundle,
   with the *slot list* as a small JSON the server can refresh independently.
5. **How fresh is availability allowed to be?** A published page showing
   yesterday's slots is what youcanbookme does when its sync lags, and it
   degrades gracefully. Fifteen minutes is probably right; it is a trigger
   interval, not an architecture.
6. **Does the `question` type ship at all** (§11).
7. **Where the extraction pass runs.** Local model on `:8080` is free and
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
