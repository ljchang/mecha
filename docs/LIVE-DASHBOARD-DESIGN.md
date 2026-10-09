# Live dashboards — design

**2026-10-09.** What is being designed: a dashboard the model writes as a
**spec**, drawn by a **Svelte renderer** we write once, over **datasets** that
**loaders** refresh on a schedule with no model in the loop — served first on
the tailnet by `mecha serve`, then published to the factory as a live
publication whose data moves while its code does not.

**Not built.** The evidence and the argument are
[`LIVE-DASHBOARD-RESEARCH.md`](LIVE-DASHBOARD-RESEARCH.md); where this file and
that one disagree, this one wins. It extends
[`PUBLIC-SURFACE-DESIGN.md`](PUBLIC-SURFACE-DESIGN.md) — §5's planned
`dashboard/` template, §5.2's Svelte rule, §14.3's publication/instrument split
— and does not restate it.

---

## 0. Rulings

Taken by the owner on 2026-10-09, in the session that produced the research.
Not to be re-asked.

| # | Ruling |
|---|---|
| R1 | **The model writes a spec; a hand-written Svelte renderer draws it.** No model-authored script in a dashboard. |
| R2 | **Vega-Lite is the working chart grammar**, subject to the measurement in §8 step 0. Whether charts render through Vega or our own components is open beneath it (§4.2). |
| R3 | **Source data lives at home or in a cloud database mecha reaches — never on the factory.** The factory holds **snapshots**: the latest dataset each published page shows. |
| R4 | **A loader is reviewed once.** Refreshes then run the reviewed loader with no model and no further review. |
| R5 | **Periodic polling**, the way drains and poll sweeps already run. No push channel in v1. |
| R6 | **Private by default**, with invites to specific people, or published. |
| R7 | **The tailnet first.** Dashboards run on `mecha serve` before anything is published to the factory. |
| R8 | Sources are open: SQLite, DuckDB, Postgres, Firebase/PocketBase and others are all in bounds (§3.1 orders them). |
| R9 | **DuckDB runs as a pinned binary**, not the crate (§3.4). |
| R10 | **Google Sheets is a source**, through mecha-docs' existing `sheets_read` (§3.1). |
| R11 | **Serving the web UI from the factory is deferred** — not designed here, not to be re-pitched as part of this arc. |
| R12 | **The tailnet is a prototype, not a destination.** Move to the factory as soon as rung 1 works, because the factory's constraints — CSP classes, grants, the vendor gate, pushes — are different and must be met early (§8). |
| R13 | **The first dashboard is the host's load** — RAM, storage, GPU, processes — **with no detail about any process beyond a generic category** such as voice, imagegen or OCR (§11). |

---

## 1. The shape

```
 ~/.mecha/dashboards/<id>/
   dashboard.json ── the spec (model-written, validated)        ┐
   loaders/<name>.toml ── a source, a query, a schema, a cron   │ reviewed together
                                                                ┘
        │ mecha dashboard refresh --due  (timer; no model)
        ▼
   data/<name>.json ── latest dataset + generated_at + digest
        │
        ├─ rung 1 ─▶ mecha serve  GET /api/dashboards/<id>/data/<name>  (tailnet)
        │                 └─ #dashboards/<id> ─ Svelte renderer polls it
        │
        └─ rung 3 ─▶ factory  PUT /v1/bundles/<id>/datasets/<name>      (push)
                          └─ bundle page ─ the same renderer polls ./data/<name>.json
```

Three objects, and the line between them is the whole design:

- **The spec** says what to draw. Data, not code (§2).
- **The loader** says where numbers come from. It is the only part that
  touches a source, and it runs without a model (§3).
- **The renderer** is code, ours, written once, vendored into every bundle
  (§4).

A **dataset** is a loader's output: one table, latest only. Datasets are what
moves; the spec and the renderer are what was reviewed.

Dashboards live in `~/.mecha/dashboards/`, beside triggers and skills and for
their reason: a loader is a cron slot that reads private data, and **a cloned
repository must not be able to bring one into a trusted session**. Never in
layered config, never in a project's `mecha.toml`.

---

## 2. The spec

### 2.1 Format

`dashboard.json`, JSON because its chart leaves are Vega-Lite and a second
syntax around them buys nothing. Illustrative, not final:

```json
{
  "version": 1,
  "title": "Lab week",
  "theme": "default",
  "datasets": ["visits_by_day", "instruments"],
  "filters": [
    { "id": "site", "label": "Site", "dataset": "visits_by_day", "field": "site" }
  ],
  "panels": [
    { "type": "kpi", "title": "Visits this week", "dataset": "visits_by_day",
      "value": { "op": "sum", "field": "visits" } },
    { "type": "chart", "title": "Visits per day", "span": 2,
      "vegalite": {
        "data": { "name": "visits_by_day" },
        "mark": "bar",
        "encoding": {
          "x": { "field": "day", "type": "temporal" },
          "y": { "field": "visits", "type": "quantitative" }
        }
      } },
    { "type": "table", "title": "Instrument hours", "dataset": "instruments",
      "columns": ["instrument", "hours"] },
    { "type": "text", "markdown": "Pilot sessions are excluded." }
  ]
}
```

Four panel types in v1: `kpi`, `chart`, `table`, `text`. A layout is a
responsive grid; `span` is the only layout knob. **Colours, type and spacing
are not in the spec** — the spec names a `theme`, the owner owns the theme
(§4.4), and a model choosing hex values is how every generated dashboard ends
up looking the same.

### 2.2 The Vega-Lite subset — no destinations

The rule is the egress rule restated for a spec: **Blind is earned by a schema
with no destination** (`EGRESS-DESIGN.md`). A chart leaf may not name anywhere
data could come from or go to:

| Vega-Lite feature | v1 | Why |
|---|---|---|
| `data: {name}` | **only form allowed** | binds to a dataset the dashboard declared |
| `data.url`, `data.values` | refused | a fetch the spec chooses; inline values bypass the loader's reviewed schema |
| `href` channel, `image` mark, `url` fields | refused | a navigation or fetch to a data-derived address |
| `usermeta`, `config` | refused | config comes from the theme, not the spec |
| `transform` (`filter`, `calculate`, `aggregate`, `fold`, `window`, `bin`, `timeUnit`) | allowed | expressions run in Vega's interpreter (§4.2), bounded length |
| `params` with `select` (`point`, `interval`) | allowed | this is where in-chart reactivity comes from — brushing, cross-filtering |
| `params` with `bind` to input elements | refused | inputs belong to the dashboard's `filters`, rendered by us |
| `layer`, `concat`, `facet`, `repeat` | allowed | composition, no new surface |

**Validation is an allowlist walk in Rust**, not a JSON Schema check: Vega-Lite's
published schema is enormous and permissive, and the question here is not "is
this valid Vega-Lite" but "does this use only what we allow". The allowlist
covers the *structure* — a view's keys, the data reference, mark types,
encoding channels, a field definition's keys, transform operations, a
parameter's keys — and unknown keys there are refused, so a Vega-Lite release
adding a destination-bearing field to any of them cannot slip through. The
*style* objects beneath (`axis`, `legend`, `scale`, a mark's properties) run to
hundreds of presentational keys; allowlisting them would be a copy of the
schema that drifts, so they are **screened** instead — any key naming a link,
URL, source or loader is refused at any depth. And every string in the whole
spec, chart or not, is refused if it is an address. Between the two, a style
object can change how a chart looks and cannot make it fetch or navigate.
(Proposed in #621, `mecha-core/src/dashboard/vegalite.rs`.) The renderer re-checks on load — the server check is the control, the
browser one a convenience, the same split as §5.1's form evaluator.

Every refusal names the field and the rule, because the reader of the error is
a 27B model that will retry; "invalid spec" teaches it nothing.

### 2.3 Reactivity

Two kinds, both in v1:

- **Data reactivity** — a dataset changes, the panels bound to it redraw.
  Polling (R5, §4.3).
- **View reactivity** — the reader filters, brushes, or hovers, and linked
  panels follow. The spec's `filters` render as our own controls bound to a
  Svelte store; each chart receives the store as Vega signals, and each table
  and KPI derives its rows from it. Vega-Lite `select` params give in-chart
  brushing. Nothing round-trips to a server.

---

## 3. Loaders

### 3.1 Sources, in build order

A loader names a **source** the owner registered, never a connection string:

```toml
# ~/.mecha/dashboards/sources.toml — owner-written; the model can only name these
[source.lab]
kind = "sqlite"
path = "~/data/lab.sqlite"     # illustrative
class = "private"            # private | untrusted — what its values may carry

[source.archive]
kind = "postgres"
url_env = "ARCHIVE_PG_URL"   # the *name* of an environment variable, never the value
class = "private"
```

A credential is named the way mecha already names one — an environment
variable, as `api_key_env` does for providers — and never written into a
dashboard file. That collides on purpose with the sandbox's rule: a confined
subprocess starts from a **cleared environment** with an allowlist, because
`Command::envs()` inherits. So the DuckDB runner (§3.4) receives exactly the
one variable its source names, re-exported under a fixed name the runner
reads, and nothing else — a hole in the allowlist that is one variable wide,
decided here rather than discovered at step 7.

| Order | Kind | How it runs |
|---|---|---|
| v1 | `sqlite` | in process, `rusqlite` (already a mecha-core dependency), opened read-only |
| v1 | `mecha` | mecha's own stores through the commands that already read them (`--json`), e.g. the run corpus |
| v2 | `duckdb` — and through it Postgres, MySQL, CSV, Parquet, JSON, S3 | a pinned DuckDB binary as a confined subprocess (§3.4) |
| v2 | `sheets` (R10) | mecha-docs' `sheets_read` with a fixed `file_id` and range; the first row is the header; the table lands in DuckDB and the query runs over it |
| later | `mcp` (any fixed read-only tool call), Firestore, PocketBase | the same adapter shape as `sheets` |

Firebase and PocketBase are application backends — store, auth and realtime
together. For a read-only dashboard they are sources like any other, and no
part of this design needs one as its own store.

**A Sheets source is registered by picking the sheet.** mecha-docs holds only
the `drive.file` scope, deliberately — it reads files it created or that the
owner picked through its picker, and widening the scope is "a different
project" by that module's own account. So registering a sheet is the picker,
once, which is the owner's act this design wants anyway. A sheet other people
edit carries their text: it defaults to `class = "untrusted"` unless the owner
says otherwise.

**`class` is fail-closed.** A source with no class, or `class = "untrusted"`,
marks its datasets as carrying third-party values (a table of mail subjects
is third-party text). That matters in §5.3; it is never a reason to refuse.

### 3.2 The loader file

```toml
# ~/.mecha/dashboards/<id>/loaders/visits_by_day.toml
source = "lab"
schedule = "*/15 * * * *"         # cron.rs, five fields
timezone = "America/New_York"     # IANA, never an offset
max_rows = 5000
query = """
SELECT date(started_at) AS day, site, count(*) AS visits
FROM visits WHERE started_at > date('now', '-30 days')
GROUP BY 1, 2
"""

[[column]]
name = "day"
type = "date"
[[column]]
name = "site"
type = "string"
[[column]]
name = "visits"
type = "integer"
```

The declared columns are the loader's **shape**, and the shape is what review
approves (§5). A query is a string; the reviewer reads it.

### 3.3 Refresh

`mecha dashboard refresh --due` runs every loader whose cron slot has passed,
on a user systemd timer — the `mecha-slots.timer` precedent, which already
pushes booking availability every two minutes with no model in it. Due-ness is
computed backwards from a ledger, as for triggers, so a machine asleep for a
week wakes owing one refresh per loader, not hundreds.

A refresh:

1. Runs the query with a timeout and `max_rows + 1` as the fetch limit.
2. **Checks the result against the declared shape** — the same column names,
   types that coerce, a row count within `max_rows`. A mismatch **refuses the
   dataset** and records why; the previous dataset stays and the page shows it
   as stale (§4.3). A query that has started returning a different shape is a
   different loader.
3. Writes `data/<name>.json` atomically: rows, `generated_at`, the loader's
   digest, and a monotonic generation.
4. If the dashboard is published (rung 3), pushes it (§6.2).

`generated_at` is the moment the query ran, not the moment it was pushed or
fetched — the freshness every surface shows comes from it.

### 3.4 Confinement

v1 SQLite loaders open the file read-only in process; there is nothing to
confine beyond the open flags. The DuckDB runner (v2) is a subprocess through
`sandbox.rs` with the source file bound read-only and **no network unless the
source kind needs it** (Postgres does). DuckDB downloads extensions at runtime
by default; the runner disables autoload and ships the pinned extensions with
the binary, fetched like the llama.cpp engine (`fetch.rs`, sha256-pinned).

---

## 4. The renderer

### 4.1 One source, two builds

`web/src/lib/dashboard/` — Svelte 5 components (`Dashboard`, `Kpi`, `Chart`,
`Table`, `Text`, `Filters`) used twice:

- **inside the web app** at `#dashboards/<id>` (rung 1), behind a dynamic
  import on that route — Vega is hundreds of KB, and the rest of a phone-first
  app should not carry it;
- **as a standalone bundle** — a second Vite entry building a self-contained
  `dashboard.js` + `dashboard.css`, which the factory's `dashboard` template
  vendors into each publish (rung 3).

The renderer is ours, so it obeys the house rules a model would not: no
`{@html}` anywhere, `text` panels through a markdown renderer with raw HTML off, CSS extracted to a file (§5.2 of the public-surface design), and a
rate over nothing is `null` and renders as a dash — `LearningCharts.svelte`'s
rule, kept.

### 4.2 Charts

Vega-Lite leaves render through `vega-embed` with the **`vega-interpreter`
plug-in** (`ast: true`, `expr: expressionInterpreter`). Without it Vega compiles
expressions with the `Function` constructor, which both CSPs here forbid. Pinned
and vendored; never from a CDN. **Unverified:** that `vega-embed` passes `ast`
and `expr` through to the view — Vega's own page documents the plug-in, not the
embed wrapper; step 0 and the browser probe below settle it.

**The two CSPs differ on `style-src`, and Vega's chain uses exactly that
directive.** The tailnet allows `'unsafe-inline'` styles; the factory's
`interactive` class is `style-src 'self'` only. `vega-embed` (its actions
menu) and `vega-tooltip` inject `<style>` elements at runtime, which a
build-time CSS extraction cannot reach. Under the tailnet that passes
silently; under `interactive` the styles are blocked and charts render
unthemed. The likely remedy is turning those injections off (`vega-embed`'s
`defaultStyle: false`, our own tooltip styles in the extracted CSS) —
**unverified**, and verified the only way that counts: the real bundle under
the real policy (§6.1).

**Owning the chart components is a later swap, not a v1 choice.** The spec is
the durable part. If Vega's weight (hundreds of KB) or its tooltip surface
proves a problem, common shapes — bar, line, area, point — can be drawn by our
own SVG components, the way `LearningCharts.svelte` already draws three, with
no published spec changing. The subset in §2.2 is chosen so that swap stays
possible.

### 4.3 Polling and freshness

- Each dataset is fetched with `If-None-Match`; a `304` costs nothing.
- The interval comes from the loader's schedule, floored at 30 s, and
  **polling stops while the tab is hidden** — a phone left open on a dashboard
  must not poll all night.
- **Freshness is shown from `generated_at`, never the viewer's clock.** A
  dataset older than two of its loader's periods carries a stale badge naming
  its age. A refused refresh (§3.3 step 2) shows as stale with the reason, on
  the owner's surfaces only.
- An empty dataset says so. A chart drawn from zero rows is a chart of
  nothing, and "nothing happened" and "nothing went wrong" are opposite
  findings.
- **A panel that fails renders its error in place**; one bad chart never
  blanks the dashboard.

### 4.4 Theme

`~/.mecha/dashboards/themes/<name>.toml`: a palette (named categorical,
sequential and diverging ramps), a type scale, spacing — rendered both as CSS
custom properties and as a Vega `config`, with light and dark. Owner-written.
The model picks a theme name; it never writes a colour.

---

## 5. Rung 1 — the tailnet

### 5.1 Routes

On `mecha serve`, under the existing owner check:

| Route | Returns |
|---|---|
| `GET /api/dashboards` | id, title, datasets with their `generated_at` and stale state |
| `GET /api/dashboards/{id}` | the validated spec and the theme |
| `GET /api/dashboards/{id}/data/{name}` | the dataset, with an `ETag` |

The page is `#dashboards/<id>` in the existing hash router, with a list at
`#dashboards`. Read-only; writes stay in the CLI.

### 5.2 Why the control surface's origin is acceptable here

`PUBLIC-SURFACE-RESEARCH.md` names the hazard: agent-authored markup on the
origin that holds the outbox's approve button. **It does not arise at rung 1
because nothing agent-authored executes.** The renderer is ours; the spec is
data validated against a subset with no destinations; text is escaped; Vega
runs under the interpreter with no `Function`. The existing CSP
(`security_headers`: `script-src 'self'`, `connect-src 'self'`) needs no
change.

Which is also why **rung 1 proves less about the factory than R12 wants**: the
tailnet's policy is looser on `style-src` (§4.2), it has no short-lived grant
under a polling page (§6.4), no vendor gate, and no box-side shape check. Each
of those is first exercised at step 5, which is why step 5 comes next.

That reasoning is conditional, and the condition is written down so nobody
loses it: **the day a dashboard may carry model-written script, it moves to a
separate origin.** Not before, and not "with care" on this one.

### 5.3 Authoring, and what the model sees

The model drafts in its workspace — `dashboard.json`, `loaders/*.toml` — with
the ordinary file tools, and checks its work with one new tool:

- **`dashboard_preview`** — validates the spec and loaders (§2.2, §3.2), runs
  each loader once against its registered source, renders the dashboard
  headless, and returns the **screenshot** plus each dataset's row count and
  shape. Errors are named field by field. This is the visual loop: write,
  preview, look, fix.

Capabilities, declared honestly — and statically, because
`Tool::capabilities(&self)` takes no input and the loop reads it *before* the
batch runs, when nothing yet knows which sources a spec will name:

- `private_data`: always — it is a picture of private data.
- `untrusted_input`: **always**. The capability says what the tool *can*
  return, and any source may be classed untrusted, even a v1 SQLite file.
  Whether a given result actually carries third-party values is
  `.from_outside()` on that result, set when a source it read is classed
  `untrusted` or unclassed — the same split as every network tool.
- Egress: **`None`, and it stays `None`** — because the tool never runs a
  query against a remote source. It runs local loaders (`sqlite`, `mecha`)
  itself; for a remote one (Postgres, Sheets, anything over a network) it
  shows the dataset the last *scheduled* refresh produced, or says there is
  none yet.

  The alternative was a class, and it is the wrong one. A remote preview would
  send a model-authored query (an unbounded payload) to a host the model
  selects by naming one of several registered sources — the `http_fetch`
  shape, so `Chosen`, not `Blind`: a host in owner-written TOML is
  indirection, not a schema with no destination. Declared `Chosen`, an armed
  conversation could never preview a remote source; kept local, the question
  does not arise, and "no model in the refresh path" (§9) is the one rule
  covering every remote read.

**Installing is the owner's act.** `mecha dashboard install <dir>` copies a
draft into `~/.mecha/dashboards/<id>/` and enables its loaders; in the web UI it
is one accept on the proposals pane, as a fourth store beside the three it
already reviews. A model-drafted loader is a
model-proposed cron slot, and no lane promotes itself — the same reason
triggers are not model-installable.

**For a loader over a remote source, install is also the review.** A
scheduled refresh sends its query to that source's host, and the query was
drafted by a model — so the install surface shows each remote loader's query
and source in full, the way the outbox shows a publish, and the owner reads it
before it ever runs. A local loader needs no more than install: the data never
leaves the machine and the only reader is the owner.

---

## 6. Rung 3 — the factory

### 6.1 The bundle

A `dashboard` template in `mecha-factory-publish`, class `interactive`:
the vendored renderer bundle, the spec, the theme rendered to CSS, and each
dataset's **current** snapshot so the first paint needs no fetch. The vendor
gate already fails on any external reference; **add a second check — a bundle
whose script contains `new Function` or `eval(` fails the publish** — rather
than trusting any library's documentation about itself.

**There is no carve-out in this gate, for any library.** A vendored Vega is
likely to contain `new Function` from its expression *codegen* even when every
view uses the interpreter — the code path ships whether or not it is called. If
the real bundle trips the gate, the answer is a build without the codegen, or
§4.2's swap to our own SVG components — never "no eval except in Vega", which
is a gate that has started degrading. Step 0 builds and greps the real bundle,
so this is known before step 3, not at step 5.

A grep cannot see a runtime `appendChild(style)`, so the gate is necessary and
not sufficient. The instrument for the rest is the one §7.1 of the
public-surface design already used: **serve the real bundle under the real
`interactive` policy in a browser and count CSP violations** (`factory-publish
serve --class interactive`, then a headless load). Zero is step 5's bar.

### 6.2 The dataset channel

The generalisation of `put_slots`:

| | |
|---|---|
| Push | `PUT /v1/bundles/{id}/datasets/{name}` with a new `Data` key scope; body = rows + `generated_at` + generation + loader digest |
| Replace | wholesale, ordered by generation. An **equal** generation whose digest and payload match what is stored returns success and changes nothing — the retry after a timeout, by `PUBLIC-SURFACE-DESIGN.md` §4's idempotency rule. A **lower** generation, or an equal one with different bytes, is refused — the out-of-order or forked push |
| Read | under the bundle's own path, `./data/{name}.json`, so the page's relative fetch passes through the **same grant** as the page; `ETag` |
| Kept | **latest only** (R3); deleting the bundle deletes its datasets |
| Capped | a per-tenant byte budget — owed anyway (`PUBLIC-SURFACE-DESIGN.md` §14.9.3) and now urgent, since a dataset is the one thing a held key rewrites forever |

The box checks shape too, against the column list the publish declared: a push
whose columns differ from the published manifest is refused. Home's check is
the control; the box's is the backstop for a home that has lost its mind.

### 6.3 Review: the loader once (R4)

Publishing a dashboard goes through the outbox as any publish does, and the
reviewable object is **the rendered page with real numbers in it, beside each
loader's query and declared shape**. Releasing it approves both. From then:

- a refresh pushes only if the loader's digest equals the digest released;
- **changing a loader, a query, or a shape is a new publish**, back through the
  outbox;
- a loader over an `untrusted` source is allowed, and the review shows that it
  is — its values are third-party text that will be published under the
  owner's name without a second look.

This is §14.6's signed-handler rule applied to data: what runs unattended is
exactly what a human released, and the machinery can refuse but not invent.

### 6.4 Who can see it (R6)

No new mechanism. A bundle's `Visibility::Private` is the default; the owner
invites people through the existing `share` grant (email-link sign-in,
per-owner daily budget); publishing is the `Public` flag. A private page's
grant is short-lived and re-checked on every fetch, so **a poll can outlive
its grant**: the page must treat a refused data fetch as "sign in again", not
as an empty dataset.

### 6.5 Out of the outbox's way

The outbox web view renders no preview for a publish today. The dashboard
publish needs one, so this arc builds it: the rendered bundle in a sandboxed
frame served from a **separate preview origin**, never by relaxing `mecha
serve`'s `frame-ancestors 'none'`.

---

## 7. The security reading, in one table

| Leg | Where it enters | What holds it |
|---|---|---|
| Private data | loaders | only owner-registered sources; read-only opens; the model names a source, never a credential |
| Untrusted content | a source classed untrusted; unknown counts as untrusted | `.from_outside()` on preview results; flagged at review |
| A way out | a remote loader's query (rung 1) | the model never runs one: preview shows the last scheduled refresh (§5.3); install shows the query and the owner reads it first |
| A way out | the dataset push (rung 3) | the loader released once (R4); digest-pinned refresh; shape checked at home and on the box; no model in the refresh path |
| Code execution | the renderer | ours; no `{@html}`; Vega's interpreter; eval check at publish; the spec has no destinations |
| The box lost | the factory | holds snapshots and public keys only — no database credential, no route home (R3) |

---

## 8. Build order

R12 orders this: the host dashboard (§11) goes end to end — tailnet, then
factory — before anything widens the source list or the authoring tools.
Its spec is written by hand for the prototype; the model-authoring loop comes
after the factory path is proven.

| Step | What | Where | Done when |
|---|---|---|---|
| 0 | **Measure the grammar.** ~20 dashboard requests on the served model, Vega-Lite vs ECharts option JSON: valid / renders / looks right (judged from the screenshot). Runs beside steps 1–2; it needs nothing from them | a scratch harness, results in this doc | a number per grammar; R2 confirmed or reversed |
| 1 | Spec types, the subset walker, loader TOML, shape check | `mecha-core/src/dashboard/` | unit tests refuse each forbidden field by name — proposed in #621 |
| 2 | The host sampler (§11) and the SQLite loader; `mecha dashboard {list, validate, refresh, install}`; the timers | core + cli | host samples accumulate; a dataset refreshes on schedule; a drifted query is refused |
| 3 | The renderer, both builds (web app and standalone) | `web/src/lib/dashboard/` | renders the host spec in light and dark; filters link panels |
| 4 | Serve routes and `#dashboards` | `serve/` | **rung 1: the host dashboard live on the tailnet** |
| 5 | `dashboard` template, eval check, dataset channel, `Data` scope, per-tenant cap, digest-pinned push, outbox preview | `mecha-factory-publish`, `mecha-factory`, `serve/` | **rung 3: the host dashboard, private, updating on the factory** — and zero CSP violations with the real bundle under the real `interactive` policy (§6.1) |
| 6 | `dashboard_preview` and the visual loop | core tool + headless render | the model fixes its own broken chart from the screenshot |
| 7 | DuckDB runner (Postgres, Parquet, CSV); the one-variable environment allowlist (§3.1); install shows remote queries in full (§5.3) | core, `fetch.rs`, `sandbox.rs`, `config.rs` | a Postgres loader runs confined and sees exactly one inherited variable; `dashboard_preview` never reaches it |
| 8 | Sheets source over `sheets_read` | core + mecha-docs | a picked sheet refreshes a dataset; `dashboard_preview` never reaches it |
| 9 | User docs | `website/docs/features/` | — |

Steps 1–4 are the tailnet prototype and need nothing from the factory
repository. Step 5 is deliberately next, not last: the factory is where this
design's untested assumptions live — the grant under a polling page (§6.4),
the eval check against a real Vega build, the box-side shape check — and
finding one wrong after steps 6–8 would mean redoing them.

## 9. Deliberately not in scope

- Model-authored script, Svelte components included. If it ever comes, it comes
  with its own origin (§5.2).
- A push channel (SSE, WebSocket). Polling is ruled (R5); the cost of adding
  SSE later is one endpoint and a renderer option.
- Viewer writes — annotations, inputs saved to the box. That is an instrument
  (§14.3) with a lease and a handler.
- Queries parameterised by the viewer, on any surface. Filters work over the
  dataset in the browser.
- Dataset history on the box. Latest only (R3).
- A model in the refresh path, for any reason — including "summarise this
  week's numbers" as a panel. A summary is a publish of model prose and is
  reviewed as one.
- MCP Apps packaging. The renderer could ship as a `ui://` resource later; no
  chat host is a target here.
- **The web UI served from the factory** (R11). Deferred by the owner. The
  research arm's answer stands for whoever reopens it: the full UI needs a
  route home, which the trust-direction rule forbids; read-only views pushed
  from home do not, and this design is one.

## 10. Open

1. **Whether the host dashboard may ever be public** (§11.4). Recommended:
   private or invited only.
2. **Grant lifetime against polling** (§6.4) — whether the factory's private
   grant gains a refresh path for a long-open page, or the page simply asks for
   sign-in. A factory-side decision; it does not block rungs 1–2.

---

## 11. The first dashboard: the host's load (R13)

What the machine running mecha is doing — memory, storage, GPU, CPU, and how
many processes — broken down by **what kind of work** it is doing, and
nothing finer.

### 11.1 Privacy by construction: the sampler is the boundary

The owner's condition is absolute — no detail about any process beyond a
generic category — so it is not enforced by the loader, the spec, or review.
**It is enforced by what is ever written down.** A sampler aggregates at the
moment of sampling into a row whose type holds only numbers and closed enums;
a process name, command line, pid, path, model name or unit name never reaches
a store, so no query, loader or publish downstream can leak one. The same
shape as `situation.rs`: a record built from closed sets only.

```rust
enum Category { ChatModel, Embeddings, Ocr, Voice, ImageGen, Mecha, Other }
```

- **The mapping lives in code**: mecha's own systemd units — the router and
  the on-demand servers `llama_units.rs` ships, the voice services, the image
  server, `mecha serve` and the timers — each map to one category. An owner
  file may map further units, **into the same closed set only**.
- **Everything unmapped is `Other`**, and it is summed, never itemised. A new
  service shows up as a bigger `Other`, which is the right failure.
- **Measured from cgroup counters, not by inspecting processes**: systemd's
  per-unit `MemoryCurrent`, `CPUUsageNSec` and `TasksCurrent` already exist
  for every unit (read here, 2026-10-09). GPU memory per process comes from
  `nvidia-smi --query-compute-apps`; the pid is used in memory to find its
  cgroup, then dropped — it is never written.
- **Storage by role, not by path**: the owner labels mounts in the same
  closed-set way (`system`, `models`, `data`); unlabelled mounts sum into
  `other`. A mount path is a detail.
- **A test pins it**: the stored row type has no `String` field, and an
  unmapped unit's sample contains its category and nothing else. Making a
  name leak requires changing a type, which a review sees.

### 11.2 What is sampled

Every minute, one row per category plus one system row:

| Series | Source | Note |
|---|---|---|
| Memory used / available, swap | `/proc/meminfo` | |
| Memory per category | cgroup `MemoryCurrent` | summed over the category's units |
| CPU % per category, load average | cgroup `CPUUsageNSec` deltas; `/proc/loadavg` | |
| Tasks per category, total | cgroup `TasksCurrent`; `/proc` count | a count, never a list |
| GPU utilisation, temperature, power | `nvidia-smi --query-gpu` | |
| GPU memory per category | `nvidia-smi --query-compute-apps` → cgroup → category | |
| Storage used / total per role | `statvfs` | |

**The GB10 has no separate GPU memory**: `nvidia-smi` reports
`memory.used [N/A]`, because GPU and CPU share one pool. So "GPU memory" is
never a total here, only a per-category share of the system figure, and the
dashboard must not draw an empty GPU-memory gauge as if the GPU had none. A
`[N/A]` is `None`, not zero.

### 11.3 Storage and retention

`~/.mecha/dashboards/host.sqlite`: one-minute rows kept seven days, a
fifteen-minute rollup kept ninety. The sampler runs on its own user timer and
writes nothing else. The dashboard's loaders are ordinary SQLite loaders over
it.

### 11.4 The one thing categories do not hide

A category's activity over time is still a record of **when** that kind of
work happened — a voice series is, in effect, a log of when calls took place.
On the tailnet that is the owner reading their own machine. Published, it is
not process detail, but it is a pattern of life. Recommended: this dashboard
is private or invited-only on the factory, never public, and its factory
datasets are coarsened to fifteen-minute buckets. The owner's call (§10.1).
