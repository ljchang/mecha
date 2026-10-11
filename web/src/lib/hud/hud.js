// The dashboard renderer's pure half (`docs/LIVE-DASHBOARD-DESIGN.md` §2, §4).
//
// Everything here is a function of its arguments, so node can pin it
// (web/test/hud.mjs); the Svelte components only draw what it returns.
//
// - **A dashboard filter is applied by the host, to rows** (§2.3): a chart
//   needs no reference to a filter to obey it, so a model that left one out
//   cannot make a filter silently do nothing.
// - **A figure over nothing is null** and renders as a dash — "nothing
//   happened" and "nothing went wrong" are opposite findings. A count over
//   no rows is a real 0.
// - **The server's check is the control; this one is a convenience** (§2.2).
//   It refuses the few shapes that would make a chart fetch, so a spec that
//   reached the page some other way still draws nothing it names.

/** The grid is four columns wide; a panel's `span` is clamped into it. */
export const GRID = 4;

/** Fastest a page polls, whatever a loader's schedule says (§4.3). */
export const POLL_FLOOR_MS = 30_000;

export function spanOf(panel) {
  const s = Number(panel?.span ?? 1);
  return Number.isInteger(s) ? Math.min(Math.max(s, 1), GRID) : 1;
}

/** A stored dataset (`{columns, rows}`) as the records Vega and the table read. */
export function records(dataset) {
  if (!dataset || !Array.isArray(dataset.columns) || !Array.isArray(dataset.rows)) return [];
  const names = dataset.columns.map((c) => c.name);
  return dataset.rows.map((row) => {
    const r = {};
    names.forEach((n, i) => {
      r[n] = row[i] ?? null;
    });
    return r;
  });
}

/** The distinct values a filter offers, in first-seen order, nulls left out. */
export function filterOptions(rows, field) {
  const seen = new Set();
  const out = [];
  for (const r of rows) {
    const v = r[field];
    if (v === null || v === undefined) continue;
    const key = String(v);
    if (!seen.has(key)) {
      seen.add(key);
      out.push(key);
    }
  }
  return out;
}

/**
 * Narrow one dataset's records by every filter bound to it. `chosen` maps a
 * filter id to the selected value (as a string), or null for all. Values are
 * compared as strings, which is how a `<select>` holds them.
 */
export function applyFilters(name, rows, filters, chosen) {
  const mine = (filters ?? []).filter((f) => f.dataset === name && chosen?.[f.id] != null);
  if (mine.length === 0) return rows;
  return rows.filter((r) => mine.every((f) => r[f.field] != null && String(r[f.field]) === chosen[f.id]));
}

function numbers(rows, field) {
  const out = [];
  for (const r of rows) {
    const v = r[field];
    if (typeof v === 'number' && Number.isFinite(v)) out.push(v);
  }
  return out;
}

/** A KPI's figure: null when there is nothing to compute it over. */
export function kpi(rows, value) {
  const op = value?.op;
  if (op === 'count') return rows.length;
  if (rows.length === 0) return null;
  const field = value?.field;
  if (op === 'last') {
    const v = rows[rows.length - 1][field];
    return v === undefined ? null : v;
  }
  const xs = numbers(rows, field);
  if (xs.length === 0) return null;
  switch (op) {
    case 'sum':
      return xs.reduce((a, b) => a + b, 0);
    case 'mean':
      return xs.reduce((a, b) => a + b, 0) / xs.length;
    case 'min':
      return Math.min(...xs);
    case 'max':
      return Math.max(...xs);
    default:
      return null;
  }
}

/** A figure as the page shows it: a dash for null, a number trimmed to sense. */
export function formatFigure(v) {
  if (v === null || v === undefined) return '—';
  if (typeof v !== 'number') return String(v);
  if (!Number.isFinite(v)) return '—';
  const abs = Math.abs(v);
  const digits = Number.isInteger(v) ? 0 : abs >= 100 ? 0 : abs >= 10 ? 1 : 2;
  return v.toLocaleString(undefined, { maximumFractionDigits: digits, minimumFractionDigits: 0 });
}

/** A table cell: text always, a dash for null. */
export function formatCell(v) {
  if (v === null || v === undefined) return '—';
  if (typeof v === 'number') return formatFigure(v);
  return String(v);
}

/** Every dataset a Vega-Lite view names, at any depth (layer, concat, facet). */
export function datasetsNamed(view) {
  const out = new Set();
  const walk = (node) => {
    if (Array.isArray(node)) return node.forEach(walk);
    if (!node || typeof node !== 'object') return;
    if (node.data && typeof node.data === 'object' && typeof node.data.name === 'string') {
      out.add(node.data.name);
    }
    for (const [k, v] of Object.entries(node)) {
      if (k !== 'data') walk(v);
    }
  };
  walk(view);
  return [...out];
}

// Keys that would make a chart fetch, navigate or carry its own data. The
// server refuses far more (`vegalite.rs`); these are the ones a page must
// never draw whatever reached it. `values` is refused only as inline data:
// `axis.values` is a list of tick positions.
const NEVER = new Set(['url', 'href', 'datasets', 'usermeta', 'config', 'signal', 'encode']);

/**
 * Why a chart must not be drawn, or null. The browser's re-check of §2.2: a
 * convenience behind the server's walker, never instead of it.
 */
export function chartRefusal(view) {
  if (!view || typeof view !== 'object' || Array.isArray(view)) return 'the chart is not a Vega-Lite view';
  let found = null;
  const walk = (node, path) => {
    if (found) return;
    if (Array.isArray(node)) return node.forEach((n, i) => walk(n, `${path}/${i}`));
    if (!node || typeof node !== 'object') {
      if (typeof node === 'string' && /^[a-z][a-z0-9+.-]*:\/\//i.test(node) && path !== '/$schema') {
        found = `${path || '/'}: an address is not allowed in a chart`;
      }
      return;
    }
    for (const [k, v] of Object.entries(node)) {
      if (NEVER.has(k) || (k === 'values' && path.endsWith('/data'))) {
        found = `${path}/${k}: not allowed in a chart`;
        return;
      }
      if (k === 'mark' && (v === 'image' || v?.type === 'image')) {
        found = `${path}/mark: the image mark is not allowed`;
        return;
      }
      walk(v, `${path}/${k}`);
    }
  };
  walk(view, '');
  return found;
}

/**
 * The view handed to vega-embed: the spec's chart with the host's rows put in
 * before compile through the top-level `datasets` map (§4.2) — inserting them
 * afterwards broke facet layout in step 0. A single view or a layer fills the
 * panel's width; a composed one keeps its own sizing.
 */
export function viewFor(chart, rowsByName, height = 220) {
  const view = structuredClone(chart);
  const datasets = {};
  for (const name of datasetsNamed(chart)) datasets[name] = rowsByName[name] ?? [];
  view.datasets = datasets;
  const single = 'mark' in view || 'layer' in view;
  if (single) {
    if (view.width === undefined) view.width = 'container';
    if (view.height === undefined) view.height = height;
    if (view.autosize === undefined) view.autosize = { type: 'fit-x', contains: 'padding' };
  }
  return view;
}

/** How often to poll: the given period, never faster than the floor. */
export function pollEvery(ms) {
  const n = Number(ms);
  return Number.isFinite(n) && n > POLL_FLOOR_MS ? n : POLL_FLOOR_MS;
}

/**
 * "as of 14:05" for a dataset's `generated_at`. The time the data was made,
 * never an age from the viewer's clock, which may disagree with the box's.
 */
export function asOf(generatedAt, timeZone) {
  const d = new Date(generatedAt);
  if (Number.isNaN(d.getTime())) return null;
  const opts = { hour: '2-digit', minute: '2-digit', ...(timeZone ? { timeZone } : {}) };
  return `as of ${d.toLocaleTimeString(undefined, opts)}`;
}

/** A tooltip's lines as text, from whatever vega hands the handler. */
export function tooltipLines(value) {
  if (value === null || value === undefined) return [];
  if (typeof value !== 'object') return [String(value)];
  return Object.entries(value).map(([k, v]) => `${k}: ${v === null || v === undefined ? '—' : String(v)}`);
}
