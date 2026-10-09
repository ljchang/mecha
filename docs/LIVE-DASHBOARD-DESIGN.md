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
| R2 | **Vega-Lite is the chart grammar** — **confirmed by step 0** on 2026-10-09 (§8.1: 15 of 22 charts right and 5 partly, against ECharts' 0 and 2). Whether charts render through Vega or our own components is open beneath it (§4.2). |
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
 ~/.mecha/dashboards/boards/<id>/
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
                          └─ bundle page ─ the same renderer polls /b/<id>/data/<name>.json
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
layered config, never in a project's `mecha.toml`. Installed dashboards sit one
level down, in `boards/<id>/`, so an id can never collide with the store's
fixed entries (`sources.toml`, `themes/`, `host.sqlite`).

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
| `data.url`, `data.values` | refused | a fetch the spec chooses; inline values bypass the loader's reviewed schema. (Mind the name: the *dashboard* spec's top-level `datasets` is a list of declared names and is required (§2.1); a *chart's* `datasets` is Vega-Lite's inline data map and is refused — the renderer alone fills that map, §4.2.) |
| `href` channel, `image` mark, `url` fields | refused | a navigation or fetch to a data-derived address |
| `usermeta`, `config` | refused | config comes from the theme, not the spec |
| `transform`: `filter`, `calculate`, `aggregate`, `joinaggregate`, `fold`, `window`, `bin`, `timeUnit`, `stack`, `flatten`, `pivot`, `density`, `regression`, `loess`, `quantile`, `impute`, `extent`, `sample` | allowed, **each with its own option keys** — an unknown sibling beside a known operation is refused, because Vega-Lite tells transforms apart by which key is present | pure data transforms, no destination; expressions run in Vega's interpreter (§4.2), bounded length |
| `params` with `select` (`point`, `interval`) | allowed | this is where in-chart reactivity comes from — brushing, cross-filtering |
| `params` with a `name` and a `value` (a variable) | allowed | the slot the renderer sets from a dashboard filter, so a chart can read the filter's state; the host writes the value, never the reader |
| `params` with `bind` | refused — input elements, and in v1 `bind: "scales"` too | inputs belong to the dashboard's `filters`, rendered by us; scale-bound pan and zoom is left out of v1 for simplicity, while an interval selection's `translate`/`zoom` events (§2.2's expression note) stay |
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

**Expressions live in named places only** — a `filter` or `calculate`
transform, a parameter's `expr`, a condition's `test`, and the event-stream
filters inside a parameter's `select` (`on`, `clear`, `translate`, `zoom`),
which are screened as style — each bounded in length, all evaluated by the same
interpreter, whose language cannot fetch or navigate. Vega-Lite also accepts `{"expr": ...}` for almost any presentational
property, and some take a bare expression under a key ending in `Expr`
(`axis.labelExpr`); both pass the screens, so inside a style object an `expr`
key and any key ending in `Expr` are refused. Data-dependent styling goes through an encoding's `condition`, and the
rest comes from the theme. Without this, R1's "no model-authored script" would
erode through the one region no allowlist walks.

**Two spellings are refused as syntax rather than chased one at a time.** No
spec string may contain a character reference (`&#106;`, `&amp;`) — CommonMark
decodes them in a link destination, so one can spell any letter of a scheme,
and JSON carries every character directly. And a text panel may not contain
link syntax at all — inline, image (`![alt](dest)`), autolink or reference
definition — which is also
what keeps a *relative* destination (`/outbox/approve/…`, on the origin that
holds that button) from becoming one, since no scheme test can see it. Raw
HTML is the fourth way to write a link, and the one that carries a relative
destination or a handler past the others (`<a href="/x">`, `<img onerror=…>`),
so a text panel may not contain an HTML tag start at all; and no spec string
may contain CSS `url(`, a fetch whether relative or not.
(Proposed in #621: the walker in `mecha-core/src/dashboard/vegalite.rs`, the
string and link-syntax rules in `spec.rs`.) The renderer re-checks on load — the server check is the control, the
browser one a convenience, the same split as the form evaluator in `PUBLIC-SURFACE-DESIGN.md` §5.1.

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
content = "owner"            # owner | third-party — who wrote the values

[source.archive]
kind = "postgres"
url_env = "ARCHIVE_PG_URL"   # the *name* of an environment variable, never the value
content = "owner"            # ignored for a remote kind: always external
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
| v1 | `sqlite` | in process, `rusqlite` (already a mecha-core dependency), opened read-only — and **confined to its one file**: a read-only open bounds writes, not reach, and `ATTACH DATABASE '<any path>'` is a read-only statement that would let a model-drafted query read any SQLite file on the host, a path from tool input that never met `ToolCtx::resolve`. `VACUUM INTO '<path>'` is the same hazard in the write direction: measured on 2026-10-09 against the *system* SQLite (3.45.1, through Python's `sqlite3`) — not the engine mecha links, which is whatever `libsqlite3-sys` bundles, so step 2 re-runs the same probe against that — it **writes a file at the query's chosen path through a `mode=ro` connection**. It runs internally as an `ATTACH`, so the controls below refuse it (both the limit and the authorizer did, and no file was written) — named here so nobody relies on that by accident. So the connection sets `SQLITE_LIMIT_ATTACHED` to 0 and installs an **allowlist** authorizer — `SELECT`, `READ`, `FUNCTION` and `RECURSIVE` (so `WITH RECURSIVE` works) only, everything else denied, which needs no knowledge of how a given statement is implemented — rather than a list of refusals (`ATTACH`, `DETACH`, `PRAGMA` are all outside it). **Extension loading is not an action code**: `load_extension()` arrives as `SQLITE_FUNCTION`, which the allowlist admits, so it is closed by three things that are the real control and are named as such — the connection's `SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION` stays off (SQLite's default; nothing calls `enable_load_extension`), rusqlite's `load_extension` feature is not enabled, and the authorizer's `FUNCTION` arm refuses `load_extension` by name besides, and runs exactly one prepared statement per loader (never `execute_batch`, whose tail would run a second). The limit and the authorizer are **deliberately redundant** — remove neither as dead. Both APIs sit behind rusqlite cargo features this workspace does not enable (`limits`, `hooks`; read in rusqlite 0.37's `lib.rs`), so step 2 adds them to the dependency; a missing method is not a reason to fall back to checking the query's text |
| v1 | `mecha` | one of a **closed set of read-only readouts, enumerated in code** (`learning-report`, `sessions health`, …) — never an argv. The mecha CLI also releases outbox drafts and accepts harness candidates, so a free-form command run unattended would be a model-drafted cron slot with side effects; a readout name that is not in the enum is a parse error, and adding one is a code change a review sees. A readout is often an object, not a table, so each enum variant carries its own fixed projection to rows, in code beside it |
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
edit carries their text; as a remote source it is external regardless (below).

**One axis, named for what it answers.** Every source here is private data —
that is why `dashboard_preview` declares `private_data` always — so the field
is not "private or untrusted", which would put two axes in one enum and let a
source that is both (a table of mail subjects) pick one. It is `content`: who
wrote the values, `owner` or `third-party`.

**The source's kind decides `external`; `content` only tightens it.** Any
source that reaches a network — Postgres, Sheets, anything remote — yields
datasets that are `.from_outside()` whatever its `content` says, because that
is what `external` means everywhere else in the tree, and a line in
owner-written TOML cannot earn back what crossing a network costs (the same
reason `Blind` is earned in code, never granted in TOML). For a local source,
`content` decides, fail-closed: unset, or `content = "third-party"`, marks its
datasets as carrying text other people wrote. Either way it matters in §5.3 and
is never a reason to refuse.

### 3.2 The loader file

```toml
# ~/.mecha/dashboards/boards/<id>/loaders/visits_by_day.toml
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

The renderer is ours, so it obeys the house rules a model would not:

- **No `{@html}`** anywhere.
- **Text panels render no links.** Markdown goes through a renderer with raw
  HTML off and links off — a link's text renders as text and its destination
  is dropped — so a scheme the server screen missed (an entity-encoded colon
  the renderer decodes, say) has nowhere to land. This is the second layer
  behind the spec's own refusal of link syntax and character references
  (§2.2; proposed in #621, `spec.rs`), not a replacement for it.
- **CSS extracted to a file** (§5.2 of the public-surface design), so the
  strictest `style-src` holds — **and no inline `style=` attribute either.**
  The `interactive` class blocks those too, and this repo's own Svelte writes
  them; a blocked attribute degrades a layout silently. Dynamic values (a
  bar's width, a colour from the theme) are set through the CSSOM —
  `element.style.setProperty`, CSS custom properties — which the CSP does not
  govern [I, believed; step 3 confirms], never as a `style=` in markup or a
  `setAttribute("style", …)`. Gate: the renderer's source is scanned for both
  spellings, and the step-5 browser probe loads **the renderer**, not only
  the vendored libraries step 0 measured.
- **A rate over nothing is `null` and renders as a dash** —
  `LearningCharts.svelte`'s rule, kept.
- **Dataset values reach the DOM only as text.** A dataset may be
  third-party text (§3.1), so every place a value is drawn — table cells,
  KPI figures, chart labels, tooltips — sets text, never HTML. Axis and legend
  labels are SVG text nodes already; the one HTML path in the chart chain is
  `vega-tooltip`'s default handler, which assigns an HTML string. Whether it
  escapes is **unverified**, so tooltips use our own handler that writes text
  nodes, and the question does not need answering.

### 4.2 Charts

Vega-Lite leaves render through `vega-embed` with the **`vega-interpreter`
plug-in** (`ast: true`, `expr: expressionInterpreter`). Without it Vega compiles
expressions with the `Function` constructor, which both CSPs here forbid. Pinned
and vendored; never from a CDN. **Settled by step 0:** `vega-embed` 7.3.0
bundles the interpreter, and `ast: true` alone turns it on; across all 22
renders under the factory's exact `interactive` CSP there were **zero**
`script-src` violations, sort cases included, and the negative control —
the same render without `ast` — hit `script-src eval` and threw.

**Data goes in before compile, through the top-level `datasets` map.**
Inserting rows with `view.data()` after `vegaEmbed` broke facet layout in step 0
(tiny single panels); supplying them as `datasets` before compile drew both
correctly. That map is **host-only**: the renderer fills it from the loaders'
datasets, and a model-written `datasets` stays refused (§2.2).

**The two CSPs differ on `style-src`, and Vega's chain uses exactly that
directive.** The tailnet allows `'unsafe-inline'` styles; the factory's
`interactive` class is `style-src 'self'` only. `vega-embed` (its actions
menu) and `vega-tooltip` inject `<style>` elements at runtime, which a
build-time CSS extraction cannot reach. Under the tailnet that passes
silently; under `interactive` the styles are blocked and charts render
unthemed. Step 0 measured it: the one violation in all 22 renders was
`style-src-elem` from `vega-embed` injecting a `<style>` — **even with
`defaultStyle: false`**. It is cosmetic (the actions menu), so the renderer
ships that CSS itself in the extracted file, and the violation is the one
§6.1's gate names and accounts for. Our own tooltip handler (§4.1) has no
style of its own to inject.

**Owning the chart components is a later swap, not a v1 choice.** The spec is
the durable part. If Vega's weight (hundreds of KB) or its tooltip surface
proves a problem, common shapes — bar, line, area, point — can be drawn by our
own SVG components, the way `LearningCharts.svelte` already draws two, with
no published spec changing. The subset in §2.2 is chosen so that swap stays
possible.

### 4.3 Polling and freshness

- Each dataset is fetched with `If-None-Match`; a `304` costs nothing.
- The interval comes from the loader's schedule (five-field cron, so a
  minute at the shortest), floored at **30 seconds** — no page polls faster
  than that whatever its schedule says. The floor binds nothing today, since
  cron cannot express less than a minute; it is there for a future
  sub-minute schedule kind. And
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
`#dashboards`. Read-only; writes stay in the CLI. **`{id}` and `{name}` are
checked as identifiers in the handler, before either touches a path** — `{id}`
through `Installed::load(boards, id)`, which refuses a non-identifier before
the join (#621), and `{name}` against the loaded spec's declared datasets, so
a URL segment never reaches the filesystem on the strength of a check that was
made about a *file*.

### 5.2 Why the control surface's origin is acceptable here

`PUBLIC-SURFACE-RESEARCH.md` names the hazard: agent-authored markup on the
origin that holds the outbox's approve button. **It does not arise at rung 1
because nothing agent-authored executes.** The renderer is ours; the spec is
data validated against a subset with no destinations; text panels are escaped
and carry no links or HTML; **dataset values**, which may be third-party,
reach the DOM only as text, tooltips included (§4.1); Vega runs under the
interpreter with no `Function`. The existing CSP
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
the ordinary file tools, and checks its work with one new tool. The draft
directory `dashboard_preview` is given is a model-supplied path, the only one
in this design, so it goes through `ToolCtx::resolve` like every other: a
preview reads a draft inside the workspace jail or reads nothing.

- **`dashboard_preview`** — validates the spec and loaders (§2.2, §3.2), runs
  each **local** loader once against its registered source (a remote source
  shows its last scheduled dataset — see Egress below), renders the dashboard
  headless, and returns the **screenshot** plus each dataset's row count and
  shape. Errors are named field by field. This is the visual loop: write,
  preview, look, fix. The screenshot travels as `ToolOutput::image`, the
  existing vehicle `image_view` uses, and inherits its cost: a transcript
  resends every image for the rest of the conversation, so five previews are
  five resident screenshots. Preview is the loop's end, not its middle —
  §8.1's 89 s per chart is the other reason.

Capabilities, declared honestly — and statically, because
`Tool::capabilities(&self)` takes no input and the loop reads it *before* the
batch runs, when nothing yet knows which sources a spec will name:

- `private_data`: always — it is a picture of private data.
- `untrusted_input`: **always**. The capability says what the tool *can*
  return, and any source may carry third-party content, even a v1 SQLite file.
  Whether a given result actually carries third-party values is
  `.from_outside()` on that result, set when a source it read **reaches a
  network — whatever its `content`** — or is a local source whose `content` is
  `third-party` or unset (§3.1: kind decides, `content` only tightens). This flag is the
  whole control, not a refinement: the conversation's untrusted taint arms
  only on `untrusted_input && external`, so a remote row returned without it
  would reach the privileged conversation clean.
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
draft into `~/.mecha/dashboards/boards/<id>/` and enables its loaders; in the web UI it
is one accept on the proposals pane, as a fourth store beside the three it
already reviews. A model-drafted loader is a
model-proposed cron slot, and no lane promotes itself — the same reason
triggers are not model-installable.

**For a loader over a remote source, install is also the review.** A
scheduled refresh sends its query to that source's host, and the query was
drafted by a model — so the install surface shows each remote loader's query
and source in full, the way the outbox shows a publish, and the owner reads it
before it ever runs. Install also offers the owner **one run of each remote
loader, at their command** — no model in it, the owner watching — so a wrong
`[[column]]` surfaces as a shape refusal while the owner is looking, not at the
first scheduled refresh after review. Declining is allowed; the first refresh
then fails closed the same way, shown as stale with its reason. A `sqlite`
loader needs no more than install — a read-only
open of a file, whose data never leaves the machine — and a `mecha` loader's
readout is a closed enum (§3.1), so neither has an effect to review.

The proposals pane is laid out for three stores ("short enough to sit
three-across on a phone"); a fourth means revisiting that layout, not only
adding a store.

---

## 6. Rung 3 — the factory

### 6.1 The bundle

A `dashboard` template in `mecha-factory-publish`, class `interactive`: the
vendored renderer bundle, the spec, and the theme rendered to CSS. **No dataset
is inside a version.** A version is content-addressed, immutable and
addressable forever (`PUBLIC-SURFACE-DESIGN.md` §6); a snapshot baked into one
would make a data change a republish, or a push would mutate a version that was
promised never to change, and every version would keep its own snapshot for
good — three readings, each breaking a rule stated here. So datasets live
beside the versioned tree, under the bundle id (§6.2), outside the digest, and
the page fetches them on load. First paint is the renderer's empty state for
the moment that fetch takes.

The cost of taking datasets out of versions, named rather than discovered: an
older version, still addressable at `/b/{id}/v/{n}/`, reads the *current*
dataset — so after a shape-changing republish (§6.3), an old version's panels
bound to a changed column fail in place (§4.3's per-panel error) rather than
showing stale numbers. That is accepted: the alias moves to the new version,
an old version is a record of a layout rather than of numbers, and the failure
is visible, never silent.

**The control on code in the bundle is the CSP, enforced by the browser.** The
box serves `interactive` bundles without `'unsafe-eval'`, so any runtime code
construction — `new Function`, a bare `Function(`, `eval(`, `.constructor(` —
throws when it runs, whatever library carries it. The publish gate is
therefore **functional, under the real policy**: a browser load of the real
bundle under the real `interactive` CSP (`factory-publish serve --class
interactive`, then a headless load) in which **the charts render correctly and
every violation observed is accounted for** — named, with the degradation it
causes, or none. Not a zero count: §7.1 of the public-surface design measured
one violation on a working bundle (a library's `Function("")` feature probe
taking its slower path) and wrote down that "a violation appeared" and "the
page is broken" are not the same thing. A violation that changes the render —
a blocked runtime `<style>`, a chart that needed the code it tried to build —
fails the gate; a probe that falls back cleanly is recorded and passes.

**The gate, as step 0 measured it, has three parts:**

1. **Our code is scanned, and the scan is a gate.** The renderer and the
   glue we write may contain none of the family — `new Function`, a bare
   `Function(`, `eval(`, `.constructor(`.
2. **Vendored libraries are pinned by sha256**, not scanned: a static scan of
   them is a report. Step 0's scan matched `vega.min.js` 7 times — four real
   `Function`-constructor sites (d3-dsv's row parser, and the expression,
   field-accessor and comparator codegens; the one `new Function` is also
   counted as a bare `Function(`) plus a method named `eval` and a
   typed-array `new x.constructor(n)` — and `echarts.min.js` 5 times, one
   real site (a GeoJSON fallback, matched twice, as `new Function` and as a
   bare `Function(`) and three `.constructor(` false positives; `vega-lite`, `vega-embed` and
   `vega-interpreter` had none. A grep gate would refuse every chart library on
   code that never runs.
3. **The runtime render decides**, under the real `interactive` CSP, by the
   one rule stated above: the charts render correctly, and every violation is
   accounted for. For `script-src` that means: a violation that changes what a
   chart shows fails the publish — a library that needs code it cannot build
   is a library this page cannot use, and the answer is a build without the
   codegen or §4.2's own SVG components, never a looser policy; a feature
   probe that is blocked and falls back cleanly (§7.1's `Function("")`) is
   named at review and passes, because nothing is lost. Step 0 measured
   **zero** `script-src` violations for Vega with `ast: true`, so today's
   bundle needs no such name. Its one violation (`vega-embed`'s injected
   `<style>`) is named in §4.2.

A new library digest is a change a review sees, which is what keeps (2) from
being an exemption that grows.

### 6.2 The dataset channel

The generalisation of `put_slots`:

| | |
|---|---|
| Push | `PUT /v1/bundles/{id}/datasets/{name}` with a new `Data` key scope; body = rows + `generated_at` + generation + loader digest + **`release`** — the publish version the push was made under, which is what makes "scoped to a release" implementable |
| Names | `{name}` is a dataset name, `[a-z][a-z0-9_]{0,63}`, and the box refuses anything else before it touches a path — the same rule the spec and loaders enforce at home (proposed in #621), restated here because this is the table a factory-side implementer reads |
| Replace | wholesale, ordered by generation. An **equal** generation whose digest and payload match what is stored returns success and changes nothing — the retry after a timeout, by `PUBLIC-SURFACE-DESIGN.md` §4's idempotency rule. A **lower** generation, or an equal one with different bytes, is refused — the out-of-order or forked push. **Generations are scoped to a release**: the box keys the channel by (bundle, `release`), a new publish (§6.3) starts it over, and a push naming a release that is no longer current is refused as **stale** — its own refusal, so home drops it rather than retrying; that is the retried push that arrives across a layout-only republish. **The publish seeds the channel**: releasing a dashboard (or republishing it) pushes each dataset's current snapshot under the new release as generation 1, so the page never opens on the empty state waiting for the next cron tick; "starts over" means a new (bundle, `release`) key with its own counter, and the previous release's stored bytes are dropped. The owner can also reset the channel explicitly (`factory-publish dataset reset <id>`) — the recovery for a home that lost its ledger, which would otherwise be refused forever while the page silently stopped moving |
| Read | `/b/{id}/data/{name}.json` — under the bundle **id**, beside the versioned tree (`/b/{id}/v/{n}/`), never inside a version and never in its digest (§6.1). The template writes that base into the page, since a version's relative `./data/` would point inside it. The grant that admits the page must admit this path too (§10, item 2); `ETag` |
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
| Untrusted content | any remote source, always; a local source whose `content` is third-party or unset | `.from_outside()` on preview results — decided by kind, tightened by `content` (§3.1); flagged at review |
| A way out | a remote loader's query (rung 1) | the model never runs one: preview shows the last scheduled refresh (§5.3); install shows the query and the owner reads it first |
| Effects | a `mecha` loader (rung 1) | a closed enum of read-only readouts in code, never an argv (§3.1) |
| A way out | the dataset push (rung 3) | the loader released once (R4); digest-pinned refresh; shape checked at home and on the box; no model in the refresh path |
| Code execution | the renderer | ours; no `{@html}`; Vega's interpreter; the CSP without `'unsafe-eval'`, and a functional browser probe at publish (§6.1); the spec has no destinations |
| The box lost | the factory | holds snapshots and public keys only — no database credential, no route home (R3) |

---

## 8. Build order

R12 orders this: the host dashboard (§11) goes end to end — tailnet, then
factory — before anything widens the source list or the authoring tools.
Its spec is written by hand for the prototype; the model-authoring loop comes
after the factory path is proven.

| Step | What | Where | Done when |
|---|---|---|---|
| 0 ✓ | **Measure the grammar, and the bundle.** — *done 2026-10-09, §8.1* — ~20 dashboard requests on the served model, Vega-Lite vs ECharts option JSON: valid / renders / looks right (judged from the screenshot). And the bundle-level questions §4.2 and §6.1 send here: build the real vendored bundles, scan them for runtime code construction, load them under the real `interactive` policy, and check that `vega-embed` passes `ast`/`expr` through | a scratch harness, results in this doc | a number per grammar; the CSP-violation count per bundle; the passthrough answer; **R2 confirmed or reversed** |
| 1 ⧗ | Spec types, the subset walker, loader TOML, shape check | `mecha-core/src/dashboard/` | unit tests refuse each forbidden field by name — proposed in #621 |
| 2 | The host sampler (§11) and the SQLite loader; `mecha dashboard {list, validate, refresh, install}`; the timers | core + cli | host samples accumulate; a dataset refreshes on schedule; a drifted query is refused |
| 3 | The renderer, both builds (web app and standalone) | `web/src/lib/dashboard/` | renders the host spec in light and dark; filters link panels |
| 4 | Serve routes and `#dashboards`; the proposals pane's fourth store and its layout (§5.3) | `serve/`, `web/` | **rung 1: the host dashboard live on the tailnet**, installable from the phone |
| 5 | `dashboard` template, the three-part gate (§6.1), dataset channel, `Data` scope, per-tenant cap, digest-pinned push, outbox preview; **a `TRIFECTA.md` channel row** for the dataset push. It is not the first standing egress grant — `mecha-slots.timer` (§3.3) already pushes unreviewed on a schedule, and has no row either, so the row covers both. What is new is that this one's **payload shape was drafted by a model**: one review authorises every future refresh of a query a model wrote | `mecha-factory-publish`, `mecha-factory`, `serve/`, `docs/` | **rung 3: the host dashboard, private, updating on the factory** — rendering correctly under the real `interactive` policy with every CSP violation accounted for (§6.1) |
| 6 | `dashboard_preview` and the visual loop | core tool + headless render | the model fixes its own broken chart from the screenshot; **the headless render loads the bundle under the `interactive` CSP from a loopback origin that serves only that bundle and its datasets, in a browser with no other network** — `Egress::None` (§5.3) rests on this render reaching nothing, so it is not left to §2.2's string screens alone |
| 7 | DuckDB runner (Postgres, Parquet, CSV); the one-variable environment allowlist (§3.1); install shows remote queries in full (§5.3) | core, `fetch.rs`, `sandbox.rs` (the allowlist entry is decided by the source in `sources.toml`, never a `Config` field — §1) | a Postgres loader runs confined and sees exactly one inherited variable; `dashboard_preview` never reaches it |
| 8 | Sheets source over `sheets_read` | core + mecha-docs | a picked sheet refreshes a dataset; `dashboard_preview` never reaches it |
| 9 | User docs | `website/docs/features/` | — |

Steps 1–4 are the tailnet prototype and need nothing from the factory
repository. **Rung 2 (publish with data frozen in) is skipped**: R12 moves
straight to the factory's live channel, and a frozen publish is step 5 with
the dataset channel left out, so it needs no step of its own. ✓ is done; ⧗ is built and in review (step 1, #621). Step 1 was built while step 0 ran: the spec's types, panels,
loaders and shape check are grammar-neutral, and only the Vega-Lite walker
assumed R2 — which step 0 then confirmed. Step 5 is deliberately next, not last: the factory is where this
design's untested assumptions live — the grant under a polling page (§6.4),
the CSP probe against a real Vega build, the box-side shape check — and
finding one wrong after steps 6–8 would mean redoing them.

### 8.1 Step 0: what was measured (2026-10-09)

Twenty-two dashboard chart requests over four small fictional datasets, sent
once each in each grammar to the router's loaded model,
`qwen3.6-35b-a3b-uncensored` — not `qwen3.8-27b`, which was not loaded and
was not swapped in — one at a time, each gated on no live voice call (none
was skipped). Rendered in headless Chromium under the factory's exact
`interactive` CSP (`script-src 'self'`, `style-src 'self'`, no
`unsafe-eval`), with vega 6.4.0, vega-lite 6.5.0, vega-embed 7.3.0,
vega-interpreter 2.3.2 and echarts 6.1.0. "Shows what was asked" was judged by
looking at each screenshot.

| | Vega-Lite | ECharts |
|---|---|---|
| Parses as JSON, no fences or prose | 22/22 | 22/22 |
| Renders under the CSP without throwing | 22/22 | 11/22 |
| Every referenced field exists | 22/22 | 19/22 |
| Shows what was asked (yes / partly / no) | **15 / 5 / 2** | **0 / 2 / 20** |
| Median latency per chart | 89 s (2,346 tokens) | 156 s (4,285 tokens) |

Latency is inflated — another session's image job shared the GPU throughout —
but the order of magnitude is the planning fact: authoring a five-chart
dashboard is minutes of model time, which is why the visual loop (§5.3)
previews and does not regenerate.

**ECharts' failure is structural**: its option grammar has no aggregation, so
the model invented an `aggregate` transform or reached for ecStat, and renders
that succeeded drew raw rows or nothing.

**Vega-Lite's commonest failure is the walker's best argument.** Five of its
seven imperfect charts used another grammar's syntax — Vega's
`{"type": "aggregate", ...}` transforms, Vega-Lite v4's removed `selection`
key — which Vega-Lite **ignores silently**, drawing an empty or unfiltered
chart with no error. §2.2's refusal of unknown keys turns every one of those
into a named refusal the model can fix. **The subset walker is the main
correctness check, not only a security one**; it is what turns "renders wrong
and says nothing" into "refused, here is why".

The rest of step 0's findings are in place: §4.2 (the interpreter passthrough,
data before compile, the injected `<style>`) and §6.1 (the gate's three
parts). Raw outputs, screenshots and the harness were kept in the session's
scratch space, not the repository.

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
   sign-in — and the grant's scope, which must cover `/b/{id}/data/` as well
   as the version the page loaded from (§6.2). A factory-side decision; it does
   not block rung 1.

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
#[repr(u8)]
enum Category {
    Other = 0, ChatModel = 1, Embeddings = 2, Ocr = 3, Voice = 4, ImageGen = 5, Mecha = 6,
}
```

**The discriminants are a wire format.** Ninety days of rollups store a
category as its number (the row has no `String` field, by design), so
reordering the variants would silently relabel history. Each variant carries
an explicit number that is never reused or reassigned, a new category takes
the next free number, and a stored number this build does not know reads back
as `None` — the repo's rule for a closed enum written to an append-only store.

- **The mapping lives in code**: mecha's own systemd units — the router and
  the on-demand servers `llama_units.rs` ships, the voice services, the image
  server, `mecha serve` and the timers — each map to one category. An owner
  file may map further units, **into the same closed set only**.
- **Everything unmapped is `Other`**, and it is summed, never itemised. A new
  service shows up as a bigger `Other`, which is the right failure.
- **Measured from cgroup counters, not by inspecting processes**: systemd's
  per-unit `MemoryCurrent`, `CPUUsageNSec` and `TasksCurrent` already exist
  for every unit (read here, 2026-10-09). GPU memory per process comes from
  `nvidia-smi --query-compute-apps=pid,used_memory` (as run in §11.2); the
  pid is used in memory to find its
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
| GPU utilisation, temperature, power draw | `nvidia-smi --query-gpu=utilization.gpu,temperature.gpu,power.draw` | `power.limit` is `[N/A]` on the GB10 (`GOAL-SYSTEM-DESIGN.md` §4.2) |
| GPU memory per category | `nvidia-smi --query-compute-apps=pid,used_memory` → cgroup → category | read on this machine 2026-10-09: the query field `used_memory` is accepted and its CSV column prints as `used_gpu_memory [MiB]`; it **does** report per process (one process at 6081 MiB) even though `memory.used` is `[N/A]` |
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

**The labels live in the store, written from code.** A loader reads the rows
by SQL, so a stored category never passes back through the enum — and a
mapping from number to name written in a loader's query would be a
model-drafted `CASE WHEN`, the silent relabel the fixed discriminants exist to
prevent. So the sampler also maintains a `categories (id, label)` table,
rewritten from the enum on every run; loaders join it, and a number with no
row there is drawn as "unknown", never as a neighbour's name.

### 11.4 The one thing categories do not hide

A category's activity over time is still a record of **when** that kind of
work happened — a voice series is, in effect, a log of when calls took place.
On the tailnet that is the owner reading their own machine. Published, it is
not process detail, but it is a pattern of life. Recommended: this dashboard
is private or invited-only on the factory, never public, and its factory
datasets are coarsened to fifteen-minute buckets. The owner's call (§10, item 1).
