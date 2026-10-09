# Live dashboards — research

> **Decided.** The owner ruled on this document the same day; the decisions
> are [`LIVE-DASHBOARD-DESIGN.md`](LIVE-DASHBOARD-DESIGN.md) §0. Three
> recommendations below were overtaken there: polling is the default with no
> push channel planned (LD4); the database stays at home or in the cloud
> with the factory holding snapshots only (Part 8, ruled R3); and **the static
> eval gate (LD8, and Part 5's "fail the publish if the bundle contains `new
> Function`") was reversed** — a grep matches every chart library on code that
> never runs, so the gate is a functional browser probe under the real CSP
> (design §6.1). LD8 and Part 5 are struck in place, because a cheap-looking
> hardening is the one a reader acts on; for everything else the body is the
> research as it stood, and the design's §0 is the status.

**2026-10-09.** One question: *how should mecha author a page that is attached
to data — a dashboard that stays current as the data changes — and publish it
on the factory, fed by pushed data, a database on the box, or a database
somewhere else?*

Researched in three passes on this date: Anthropic's artifact and design
tooling (including the artifact runtime contract served to a Claude Code
session, read first-hand); OpenAI's equivalents and the "prompt → live data
app" field; and what mecha and the factory already have. Claims carry their
strength: **[D]** a primary source says it; **[C]** the artifact runtime
contract 0.2.75 type definitions served to this session say it — authoritative
for that version but not a public page, so cite as observed; **[I]** inference;
**[NF]** looked for and not found. Product behaviour here is perishable — two of
the surfaces below (Claude's `db`, OpenAI's Sites) shipped within the last four
months — so re-check a product before building against its behaviour. The
architectural conclusions move slower.

This document **extends** `PUBLIC-SURFACE-DESIGN.md` rather than reopening it.
§5 there already plans a `dashboard/` template (class `interactive`, "charts,
no network, no eval"), §5.2 already rules that interactive bundles are Svelte,
and §14.3 already splits *publications* from *instruments*. What is new here is
a third thing neither covers: a page whose code is finished but whose data is
not.

---

## The one-sentence answer

**The field has converged on "declarative spec, host-owned renderer,
server-owned data" — not "the model writes an app" — and that shape is also
the one that fits a 27B local model, the factory's no-eval CSP, and the
trifecta at once; the new piece mecha needs is a *dataset channel* on the box
fed by *loaders reviewed once and run without a model*, so that a refresh is
deterministic egress rather than a fresh decision.**

---

## Part 1 — What Claude actually ships

### The `db` capability is real, reactive, and Firestore-shaped [C]

Artifacts made from 2026-09-16 on are a new generation [D, support 9487310];
the 2025 `window.storage` (personal/shared, 20 MB per artifact, text only) is
the legacy one [D]. The new one declares **runtime capabilities** at publish —
`capabilities: {db: {}, user: {}, ...}` — and the page reaches each through
`await claude.use(name)`, which resolves `null` when the view cannot run it.
The contract is explicit that absence is a design case, not an error.

- **Shape.** One JSON document store per artifact; slash paths alternate
  collection/document (`tasks/t1`); `get / set / update / delete`,
  `where / orderBy / limit(≤1000)`, and `acquire()` for leases. The store is
  created on first write, survives republishes, and dies with the artifact.
- **Reactivity.** `onSnapshot` "rides a realtime stream when available and
  falls back to periodic refresh (about 30 s foreground) — same callbacks
  either way." Other viewers' writes arrive live; your own appear immediately
  with `hasPendingWrites`.
- **Access.** Level-based on the share menu: `view < interact < admin < owner`.
  Up to 64 per-path rules set minimum read/write levels; `{self}` names each
  viewer's own subtree, private even from the owner. A refused read looks like
  a missing document and a refused write is `invalid_argument` — there is
  deliberately no `permission-denied` code.
- **Limits.** 256 KiB and 32 levels per document; 64 subscriptions per view; a
  per-viewer call rate; count and size caps whose numbers the contract does
  not state [C; values NF].
- **The agent writes from outside.** A separate `ArtifactData` tool reads and
  writes rows, every write pinned with `if_version` so a concurrent viewer
  edit is refused rather than overwritten. **A data change is never a
  republish** — the page is code, the rows are data, and they version
  independently.

### Live data from connectors: `mcp.watchTool` [C]

The `mcp` capability calls **the viewer's** claude.ai connectors with the
viewer's credentials; the page never sees a token. The manifest names each
server and each tool at publish (an empty tool list is refused, never "all").
Two arms, and the split is the useful idea: **displaying** data is
`watchTool(server, tool, input, handler)` — replays a cache, refreshes when
stale, polls only on an explicit `refetchInterval`; **acting** is `callTool`,
once, never retried unless the error says `retryable`, because a timed-out
write is an ambiguous outcome. Freshness is shown from a shell-attested
`cache.storedAt`, never `Date.now()`.

### The rest of the roster [C]

`sample` (the page asks Claude; spends the *viewer's* usage, consent on first
call), `artifact` (the page republishes itself — every open view reloads to
the winner, a concurrent save gets `conflict`), `room` (ephemeral presence and
broadcast, nothing persists), `user`, `assets`, `files`, `downloads`,
`comments`, and built-in `permissions`. Billing: "usage counts against each
person's own plan limits rather than yours" [D, claude.com/blog].

### Serving and CSP [D]

Pages run on a sandboxed `*.claudeusercontent.com` origin; scripts only from
five CDNs, fonts only from Google Fonts, `fetch` only to the page's own
origin. Everything that reaches outside goes through a capability the shell
brokers — **the page has no network of its own**. That is the property worth
copying, and the factory already has it (`connect-src 'self'`, Part 6).

### The "artifact MCP" and the "designer MCP" [D / NF]

- **No public Anthropic "Artifacts MCP server" exists** [NF]. Publishing from
  Claude Code is a first-party `Artifact` tool tied to a claude.ai login
  [D, code.claude.com/docs/en/artifacts]; third-party lookalikes exist.
- **Claude Design** launched 2026-04-17 as an Anthropic Labs preview [D] and
  has since folded into artifacts as the Design type; the standalone
  claude.ai/design closes 2026-12-14 [D, support 14604416].
- **`DesignSync`** is the tool behind `/design-sync`: it syncs a *local
  component library* into a claude.ai design-system project [D, support
  14604397; C, tool schema]. It is not a design generator. Its shape is the
  part worth noting: list → **`finalize_plan`** (the exact paths to write,
  approved by the user, returning a plan id) → writes that must fall inside
  the plan. A design system there is a guide, tokens (palette, type scale,
  spacing) and components with preview cards [D].
- **Figma** has an official remote MCP server [D]; it is the nearest thing to
  a "designer MCP" and is a design-file reader, not a dashboard builder.

## Part 2 — What OpenAI ships

- **The Apps SDK is now MCP Apps.** UI is a `ui://` resource in a sandboxed
  iframe over JSON-RPC `postMessage`; `window.openai` survives as
  compatibility aliases over the standard [D, developers.openai.com]. OpenAI's
  own persistence advice is the line to keep: "Business data is the source of
  truth. Do not store it only in the UI" — business data on the server,
  ephemeral state in the component, durable cross-session state "in storage
  you control" [D].
- **Sites** (announced 2026-06-02, connected data at DevDay 2026-09-29) is the
  direct analogue of this feature: prompt → hosted web app with a relational
  store (D1, 10 GB per Site) and object storage, a **two-stage publish** —
  "Save a version" (a reviewable candidate tied to a commit) then "Deploy" —
  owner-only by default, and a page that reads **each viewer's own** connected
  apps, with writes behind visitor consent [D, learn.chatgpt.com/docs/sites].
- **Codex** itself has no artifact or preview surface: the CLI has no visual
  preview, and Sites is managed from ChatGPT, not Codex [D].
- **Canvas** renders HTML/React with network access off by default in
  Enterprise [D, secondhand — the help page refused the fetcher].

## Part 3 — MCP Apps: the open standard both sides adopted

Proposed 2025-11-21 by Anthropic, OpenAI and MCP-UI; **Stable 2026-01-26**;
ext-apps SDK v2.0.3 (2026-09-25) [D]. What it is:

- A tool links a UI with `_meta.ui.resourceUri`; the resource is
  `text/html;profile=mcp-app`, declaring its own CSP domains and permissions.
- A web host **must** render through a sandbox proxy on a different origin;
  the default CSP has `connect-src 'none'` [D].
- Data reaches the view as `tool-result` notifications (`structuredContent`
  for the UI, `content` for the model), and the view calls tools back through
  the host, which may gate each call. Tools can be marked app-only
  (`visibility: ["app"]`), hidden from the model's tool list [D].
- **No persistence and no push.** "Interactive updates" means the view re-calls
  tools [D]. Claude's `db`, `room` and `sample` are proprietary layers on top.

[I] For mecha this is a *later* target, not v1: it is how a mecha dashboard
could render inside Claude or ChatGPT, and the same renderer could ship as a
`ui://` resource. It does not answer the hosting question, because it assumes
a chat host is present.

## Part 4 — The field, and the direction it moved

| Product | Where data lives | Freshness | Notes |
|---|---|---|---|
| v0 / Lovable / Bolt / Replit | Managed Postgres/Supabase; the agent gets DDL [D] | App-defined | Generated code against a managed store |
| **Evidence** | One warehouse, queried server-side [D] | Live query (formerly static rebuild) | **Moved from Svelte components to Markdoc tags: "Core expresses config not code, JS is not executed"** [D] |
| Observable Framework | Data loaders write snapshots at build [D] | Rebuild only; secrets stay build-side [D] | Its cloud deploy was discontinued 2025-10-15 [D] |
| Streamlit / Gradio | Server process | Timed server reruns [D] | A long-running Python server per app |
| Grafana | Its datasources | Polling panels | Schema too large for a model to validate unaided [D, community] |

And the generative-UI layer: **json-render** (Vercel Labs, Apache-2.0) — a
Zod-typed component catalog, the model emits a JSON tree, anything outside the
catalog cannot render, and it ships **a Svelte 5 renderer** [D]. **A2UI**
(Google) is the same idea with a data model bound by JSON paths and
`updateDataModel` over SSE, but lists no Svelte renderer and no chart
component [NF]. CopilotKit names the ladder plainly: static components,
declarative specs, then open-ended iframes that "trade consistency and safety
for flexibility" [D].

**The pattern [I]:** products optimising for *speed to a demo* generate code
against a managed database; the one optimising for *agent-authored analytics
that someone must trust* (Evidence) moved the other way, from code to config.
mecha is the second kind.

## Part 5 — What our model can actually author

The router today serves, among others, `qwen3.8-27b` and the `qwen3.6-35b-a3b`
family (asked of `GET /models` on 2026-10-09, not asserted).

- **General web dev.** Arena WebDev (updated 2026-10-08): qwen3.8-27b **1593**,
  best open-weight entries ~1620–1640, top proprietary 1813 [D, via fetcher].
  That arena is largely a **React + Tailwind** benchmark [D, Willison; I].
- **Svelte 5.** SvelteBench (nine small runes-era components, pass@1): local
  qwen3.6:27b q4 **93.3%**, qwen3.6:35b-a3b 88.9%, gpt-oss:20b 24.4% [D]. Runes
  are not a blocker for this model family *on components* (these are
  qwen3.6 figures, a prior generation to `qwen3.8-27b`; step 0 later measured
  the model the router had loaded, `qwen3.6-35b-a3b-uncensored` — design §8.1). Svelte ships an
  official MCP server with docs and static analysis [D].
- **Frameworks vs HTML.** DesignBench: models perform "substantially lower …
  in framework-based development compared to vanilla HTML/CSS" [D]. Web-Bench
  (2025 models) has React usually ahead and Svelte highly variable [D].
- **Chart specs.** Evidence on current open models emitting Vega-Lite or
  ECharts JSON is thin and old [NF for current models]; small models fail
  Vega-Lite zero-shot [D, CycleChart].
- **No head-to-head React-vs-Svelte measurement for current models exists**
  [NF].

[I] The question is not React or Svelte. It is **free code or a spec**, and the
answer for a 27B model is a spec validated by a schema — because a schema
turns a silent wrong render into a loud error the agent can retry on, which
matters more at this size than raw accuracy. Svelte stays the right choice for
the *renderer*, which we write once, by hand, and the model never touches.

### A CSP fact that decides the chart library [D]

The factory's `interactive` class is `script-src 'self'` with no eval. **Vega
compiles expressions with the `Function` constructor by default** and needs
the `vega-interpreter` plug-in (`ast: true`, `expr: vega.expressionInterpreter`)
to run under such a policy, about 10% slower [D, vega.github.io/vega/usage/interpreter].
Whether `vega-embed` passes those options through needs checking at build
[NF]. Observable Plot and ECharts are believed eval-free [I, unverified].
~~Either way the cheap enforcement is the one the vendor gate already uses:
fail the publish if the bundle contains `new Function` or `eval(`.~~
*Reversed by design §6.1: a static scan matched 8 times in Vega and 5 in
ECharts on code that never ran; the gate is a functional browser probe under
the real CSP, and the scan is a report.*

## Part 6 — What mecha and the factory already have

Mapped 2026-10-09 against `main` at `e777602a1` and `mecha-factory` at
`e0e99a8`. Cited by symbol.

- **Publish is an MCP tool on the `factory` server** (`factory-publish mcp`),
  routed through `[outbox] tools`; `[outbox] publish_tools` gives
  `OutboxKind::Publish`. mecha-core knows nothing about the factory.
  `bundle_render` is local and unreviewed — the intended loop is render,
  inspect, fix, publish once.
- **Content classes and CSP** live in `mecha-manifest` (`ContentClass`,
  `csp.rs`): `static` (`script-src 'none'`), `interactive` (`'self'`, no eval,
  no wasm, no inline), `compute` (wasm, COOP/COEP, own origin). All three are
  `connect-src 'self'`. `vendor.rs` fails a publish on any surviving external
  reference.
- **Only the `report` template exists.** The `dashboard/` template is planned
  (§5) and unbuilt; marimo html-wasm notebooks are today's only
  data-in-the-browser path.
- **A pushed-data precedent already exists.** `PUT /v1/instruments/{id}/slots`
  (`put_slots`, a dedicated `Scope::Slots` key) replaces a booking page's
  availability wholesale with a `generated_at`, and the page reads
  `/s/{handle}/{id}/slots.json` live. That is a dataset channel in miniature,
  and the thing to generalise.
- **Review gap.** The outbox shows a publish as its bundle directory; the web
  `Outbox.svelte` blocks editing for `kind === 'publish'` but **renders no
  preview**, and `mecha serve`'s `security_headers` set
  `frame-ancestors 'none'`, which rules out framing one today.
- **No database tool is exposed to the agent** — no SQL, SQLite or DuckDB tool
  in `mecha-core/src/tool/` or over MCP.
- **The model sees images** (`image_view`) and a headless Chromium renders
  here; together they are a visual render-inspect-fix loop nobody has wired to
  `bundle_render` yet.

## Part 7 — A third kind of artifact

§14.3 has two kinds: a **publication** (immutable versions, a moving alias,
finished when published) and an **instrument** (a schema, an inbox, a lease, a
handler). A dashboard attached to data is neither:

| Kind | Code | Data | Who writes data | Needs |
|---|---|---|---|---|
| Publication | immutable version | baked in | nobody | nothing |
| **Live publication** | **immutable version** | **mutable dataset** | **home, by push** | **a dataset channel, a loader** |
| Instrument | immutable version | an inbox | strangers | a schema, a lease, a handler |

The split Claude's contract draws — page versions and rows move independently,
and a data change is never a republish — is exactly this middle row. It keeps
the useful property of a publication (the *code* a viewer runs is a reviewed,
content-addressed version) while letting the numbers move.

## Part 8 — The trifecta reading, which decides the architecture

A dashboard of anything worth seeing is **private data**, and pushing it to the
factory is **egress**. The first publish already goes through the outbox. The
question the existing design does not answer: **what about the hundredth
refresh?** Reviewing every push makes an hourly dashboard unusable; reviewing
none makes the push an unreviewed send.

[I] The resolution is the one §14.6 already reached for handlers, applied to
data: **review the loader, not each refresh, and keep the model out of the
refresh.**

- A **loader** is declarative: a named source, a read-only query, and a
  declared output schema (columns, types, a row cap). It is reviewed once, at
  publish, alongside a rendered sample of its output — the reviewable object
  is the page with real numbers in it.
- A refresh **runs the reviewed loader and nothing else** — a trigger with no
  model in it. An injection in the source data can change *values*; it cannot
  change *which* query runs or *where* the result goes, because neither is
  decided at refresh time. That is what makes it safe to skip review.
- The push is **schema-checked against the reviewed shape**: an extra column,
  a type change, or a row count over the cap refuses the push. A loader whose
  output has drifted is a different loader and needs review again — the
  silently-degrading-guard rule, applied to data.
- **Changing a loader is a publish**, through the outbox, never an edit in
  place.

And three placements that follow from "the box never dials home" and "assume
the box is lost":

- **Home databases and cloud databases are read at home.** The loader runs
  where the credential already lives and pushes the result. The box never
  holds a database credential — Observable Framework's rule ("secrets stay on
  the build side") arrived at independently [D].
- **A database *on the box*** is the dataset store itself (SQLite, WAL, beside
  the factory's own), plus whatever the box already owns — instrument
  submissions, poll tallies. A dashboard over those needs no push at all.
- **No viewer-parameterised SQL on the box.** Filters a viewer applies are
  computed client-side over the pushed dataset, or select among named
  precomputed datasets. An endpoint that runs a query built from a URL is a
  new attack surface on hardware we have agreed to assume is lost.

## Part 9 — "A frontend designer in mecha"

[I] Read against everything above, the designer is not a code generator. It
is four parts:

1. **A dashboard spec** the model writes: layout, KPI tiles, tables, and chart
   panels whose leaves are chart specs bound to named datasets. JSON with a
   JSON Schema, validated in Rust at render and again at publish.
2. **A renderer** we write once in Svelte 5 — compiled, CSS extracted,
   vendored into each bundle, eval-free and checked to be — reading the spec
   and the datasets. Templates are data, not code (§5), and a spec is data.
3. **A theme** the owner owns: tokens (palette, type scale, spacing) in a file
   the renderer reads, the model choosing token *names*, never hex. This is
   Claude Design's design system and `DesignSync`'s token sync, minus the
   cloud.
4. **A visual loop**: `bundle_render` → headless screenshot → `image_view` →
   fix the spec → render again, before anything reaches the outbox. And the
   outbox gains the preview it lacks — the rendered page in a sandboxed frame
   on its own origin, so the review is of the thing itself.

The escape hatch — the model writing a Svelte component — stays available as
a later tier for what the catalog cannot express, under the same vendor gate
and eval check. It is a tier, not the default.

---

## Recommendations

### Shape

- **LD1. A third artifact kind, the live publication** — immutable code
  version plus mutable datasets — added to `PUBLIC-SURFACE-DESIGN.md` §14.3's
  table rather than a parallel model.
- **LD2. A dashboard spec plus a fixed Svelte renderer** for the `dashboard/`
  template. The model never authors script for a published bundle in v1.
- **LD3. Generalise `put_slots` into a dataset channel**:
  `PUT /v1/bundles/{id}/datasets/{name}` under a new key scope, wholesale
  replace with a `generated_at` and a generation number, a per-tenant byte cap
  (already owed, §14.9.3); the page reads
  `GET /b/{id}/data/{name}.json` on its own origin (`connect-src 'self'`
  already allows it), with an ETag. *(The design keeps this path and pins it
  absolutely: the page loads from `/b/{id}/v/{n}/`, where a relative
  `./data/` would resolve inside the immutable version — design §6.2.)*
- **LD4. Freshness by polling first, SSE second.** A dashboard refreshed every
  few minutes needs a conditional GET and nothing else. SSE (one stream per
  page, carrying invalidations, never the data) earns its place only for
  sub-minute data, and needs HTTP/2 to dodge the six-connection limit [D, MDN].
- **LD5. Show freshness from the dataset's own `generated_at`**, never the
  viewer's clock — Claude's `cache.storedAt` rule. A dataset older than its
  loader's schedule says so on the page; stale is a state, not an absence.

### Security

- **LD6. Loaders are reviewed once and run without a model**; a push that
  does not match the reviewed schema is refused; changing a loader is a
  publish. This is the load-bearing decision and is the owner's to make (Q1).
- **LD7. No database credential on the box, ever.** Cloud and home sources are
  read at home.
- ~~**LD8. Extend the publish gate**: fail on `new Function` / `eval(` in a
  bundle's scripts, beside the existing external-reference check.~~
  *Reversed — design §6.1.*
- **LD9. A preview in the outbox**, framed from a separate sandbox origin —
  `mecha serve`'s `frame-ancestors 'none'` stays as is for its own pages.

### The ladder

| Rung | What | New machinery |
|---|---|---|
| 0 | Spec + renderer + the visual loop, local only | the renderer, a spec schema, screenshot → `image_view` |
| 1 | Dashboards on `mecha serve` over the tailnet, reading home sources directly | a read-only SQL source tool; a sandboxed route — **no egress decision at all** |
| 2 | Publish to the factory with data frozen in (a publication) | the `dashboard/` template; nothing on the box |
| 3 | The live publication: datasets, loaders, the refresh trigger | LD3, LD6; a key scope |
| 4 | Later: SSE; MCP Apps packaging; a Svelte-component tier; viewer writes (an instrument) | each separately argued |

Rung 1 is worth noticing: most dashboards an owner wants of their own data
never need to leave the tailnet, and on it the whole trifecta question about
pushes does not arise.

## Deliberately not recommended

- **Model-authored JavaScript in a published bundle, in v1.** The evidence
  says it is the harder target for the model, the CSP makes it fragile, and it
  makes every refresh-capable page a review of code instead of a picture.
- **Copying Claude's `sample` onto the factory.** A public page that spends
  model tokens per viewer is the faucet §14.4 closed.
- **Viewer-writable shared state in v1.** That is an instrument, with leases
  and handlers; a dashboard does not need it.
- **A2UI** (no Svelte renderer, no charts), **Thesys C1** (hosted, cannot use a
  local model), **Grafana JSON** (too large to author unaided), **PocketBase**
  (a second server to own beside the factory's SQLite).
- **DuckDB-WASM for private data** — it cannot send auth headers [D]. Fine for
  public datasets, and a compute-class question then, not a dashboard one.

## Open questions for the owner

1. **Q1 — Is "review the loader, run refreshes unreviewed" acceptable?**
   Every other rung depends on it. The alternative is a per-push outbox item,
   which caps freshness at the owner's attention.
2. **Q2 — Which dashboard first?** The renderer's catalog should be built from
   a real subject, not a speculative one; the answer decides which source
   types rung 1 needs.
3. **Q3 — Cloud databases:** home-side loaders only (recommended), or a
   read-only replica credential on the box as an opt-in with its cost written
   down in `TRIFECTA.md`?
4. **Q4 — Chart grammar:** Vega-Lite (the best-known spec, needs the
   interpreter) or ECharts/Plot options (lighter, believed eval-free,
   less-known to models)? Resolvable by a small measurement on the served
   model, rather than by argument.

## Sources

**Anthropic.** support.claude.com/en/articles/9487310 (artifacts, storage);
/9547008 (sharing); /14604416 (Claude Design); /14604397 (design systems,
`/design-sync`); code.claude.com/docs/en/artifacts;
claude.com/blog/claude-powered-artifacts;
anthropic.com/news/claude-design-anthropic-labs (2026-04-17);
claude.com/blog/interactive-tools-in-claude (2026-01-26);
github.com/anthropics/skills `frontend-design` (2026-09-03). Artifact runtime
contract 0.2.75 type definitions (`db.d.ts`, `mcp.d.ts`, `claude.d.ts`) and the
`DesignSync` / `ArtifactData` tool schemas, as served to a Claude Code session
on 2026-10-09 — observed, not published.

**MCP Apps.** blog.modelcontextprotocol.io/posts/2025-11-21-mcp-apps;
github.com/modelcontextprotocol/ext-apps (`specification/2026-01-26/apps.mdx`,
v2.0.0 2026-09-08); modelcontextprotocol.io/seps/1865.

**OpenAI.** learn.chatgpt.com/docs/sites.md;
learn.chatgpt.com/docs/whats-new/devday-2026.md;
developers.openai.com/plugins/build/chatgpt-ui.md;
help.openai.com/en/articles/9930697 (secondhand).

**Field.** docs.evidence.dev (migration guide, self-host);
observablehq.com/documentation/data-apps/security; github.com/vercel-labs/json-render;
a2ui.org (v0.9.1); docs.copilotkit.ai/learn/generative-ui; v0.app/docs/databases;
docs.lovable.dev/integrations/supabase; support.bolt.new/cloud/database;
docs.replit.com SQL database; docs.streamlit.io `st.fragment`; gradio.app Timer.

**Models.** arena.ai/leaderboard/webdev (2026-10-08);
khromov.github.io/svelte-bench; arxiv.org/abs/2505.07473 (Web-Bench);
arxiv.org/html/2506.06251v3 (DesignBench); arxiv.org/abs/2507.04952
(ArtifactsBench); arxiv.org/pdf/2512.19173 (CycleChart);
huggingface.co/Qwen/Qwen3.6-27B; svelte.dev/docs/ai.

**Plumbing.** vega.github.io/vega/usage/interpreter;
developer.mozilla.org Server-sent events; fly.io/blog/litestream-v050-is-here;
duckdb.org/docs/current/clients/wasm/known_issues; pocketbase.io/docs/api-realtime.
