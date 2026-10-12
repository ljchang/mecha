# Drafting a dashboard

A dashboard is a spec (what the page shows) and one loader per dataset (a
read-only SQL query against a registered source, the columns it returns, and
how often it refreshes). You never write files for it: you pass the spec and
the loaders to the tools.

1. `hud_sources` — the sources, their tables and a few rows of each, the
   boards already installed, and this guide. With `source` and `query` it runs
   one read-only query so you can look at real values.
2. `hud_preview` with `id`, `spec` and `loaders` — checks everything, runs
   every query once, and returns each dataset's row count and first rows, or
   every error by name. Fix and preview again until it is clean. With `id`
   alone it returns that board's spec and loaders, to revise — a proposal
   still waiting for the owner first, else the installed board.
3. `hud_propose` with the same three — stages the board for the owner, who
   sees it drawn with real data and installs it with one tap. Proposing an
   installed board's id proposes a replacement. Nothing is installed until the
   owner says so.

## The spec

```json
{
  "version": 1,
  "title": "Short title",
  "datasets": ["speed_24h"],
  "panels": [
    {"type": "kpi", "title": "Fastest today (tokens/s)", "dataset": "speed_24h",
     "value": {"op": "max", "field": "tokens_per_s"}},
    {"type": "chart", "title": "Speed over the last day", "span": 2,
     "vegalite": {
       "data": {"name": "speed_24h"},
       "mark": {"type": "point", "filled": true},
       "encoding": {
         "x": {"field": "at", "type": "temporal", "title": null},
         "y": {"field": "tokens_per_s", "type": "quantitative",
               "title": "Tokens per second"}
       }
     }},
    {"type": "text", "markdown": "Speeds appear only for minutes that had requests."}
  ]
}
```

- `datasets` lists every dataset; each needs a loader of the same name. Names
  match `[a-z][a-z0-9_]*` — lowercase, so `mem_gib`, never `mem_GiB`. Column
  names follow the same rule.
- `filters` (optional): `[{"id": "x", "label": "X", "dataset": "…", "field":
  "col"}]` — a dropdown over one column's distinct values; every panel bound
  to that dataset follows it.
- Panels sit on a 4-column grid; `span` is 1–4 (default 1). On a phone every
  panel is full width.
  - `kpi`: `"value": {"op": "sum|mean|min|max|count|last", "field": "col"}`.
    `count` needs no field; `last` is the value in the dataset's last row.
  - `chart`: `"vegalite"` is a Vega-Lite view (rules below). A chart panel
    has no `dataset` key — the view names its data itself.
  - `table`: `"dataset"` and `"columns": ["a", "b"]`. Prefer a chart where the
    point is a comparison; a table of long text is hard to read on a phone.
  - `text`: `"markdown"`, plain prose — no links, no HTML.
- No colours or fonts in the spec: the theme supplies them.

## Charts: the Vega-Lite subset

- Data only as `"data": {"name": "<a dataset>"}` — never `url`, `values` or a
  chart-level `datasets`.
- Allowed: `mark` (`arc` for pie and donut, `area`, `bar`, `circle`, `line`,
  `point`, `rect`, `rule`, `text`, `tick`, …), `encoding` (x, y, color,
  theta, radius, size, opacity, text, tooltip, order, detail, …),
  `transform` (filter, calculate, aggregate, fold, window, bin, timeUnit,
  stack, …), `layer`, `hconcat`/`vconcat`, `facet`, `params` with `select`.
- Refused: `config`, `usermeta`, `href`, the `image` mark, `params` with
  `bind`, any key ending in `Expr`, and any URL anywhere.
- Field types: `quantitative`, `temporal`, `nominal`, `ordinal`.
- Give every quantitative axis a `title` with its unit ("Memory (GiB)");
  the default is the raw column name.
- Values far apart in size (hundreds against thousands) get separate charts,
  not one axis.
- Sparse readings — minutes that had an event — are points, not lines: a
  line joins across the hours with none.
- A pie or donut: an `arc` mark with `theta` on the value, `"stack": true`,
  `color` on the label and `order` on a sort column; `innerRadius` makes it
  a donut. Legends: `"legend": {"orient": "bottom", "columns": 2}` reads on
  a phone.

## Loaders

Passed as TOML text, one per dataset:

```toml
source = "system"           # a registered source, by name
schedule = "*/5 * * * *"    # five-field cron
max_rows = 2000             # refused if the query returns more
query = """
SELECT at, round(gen_tps, 1) AS tokens_per_s
FROM model_minute
WHERE kind = 1 AND at >= datetime('now', '-24 hours') AND gen_tps IS NOT NULL
ORDER BY at
"""

[[column]]
name = "at"
type = "timestamp"
[[column]]
name = "tokens_per_s"
type = "float"
```

- One read-only `SELECT` (a `WITH` is fine). The result's columns must be
  exactly the declared `[[column]]`s, in order, with those types: `string`,
  `integer`, `float`, `boolean`, `date` (`YYYY-MM-DD`), `timestamp` (RFC 3339,
  or `YYYY-MM-DD HH:MM:SS` in UTC as SQLite's `datetime()` writes it).
- `timezone = "America/New_York"` (optional) is the zone the schedule is read
  in; UTC otherwise.
- Refresh a "now" dataset every minute and a day-long history every few.

## Reading the data honestly

- NULL means unknown or not measured, never zero. Leave NULL rows out of a
  chart rather than drawing them as zero.
- "The latest value" of something measured only sometimes is the last
  non-NULL one, with when it was measured — not the newest row, which is
  usually NULL.
- A "now" dataset should return no rows when its source has stopped
  updating (`HAVING max(at) >= datetime('now', '-5 minutes')`), so the page
  shows nothing rather than an old reading as current.
- Look at real rows with `hud_sources` before guessing a format or a unit.
