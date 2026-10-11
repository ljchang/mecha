// The dashboard renderer's pure half (src/lib/hud/hud.js) and the source
// gates `docs/LIVE-DASHBOARD-DESIGN.md` §4.1 names. `npm test` in web/.
//
// Pinned here: a filter narrows rows in the host, so a chart obeys it without
// naming it; a figure over nothing is null (a dash), and a count over nothing
// is 0; the browser re-check refuses what would make a chart fetch, and lets
// a style's `values` through; rows reach Vega before compile, through
// `datasets`. Then the gates: no `{@html}`, no `innerHTML`, no inline style.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  records,
  filterOptions,
  applyFilters,
  kpi,
  formatFigure,
  formatCell,
  spanOf,
  datasetsNamed,
  chartRefusal,
  viewFor,
  pollEvery,
  POLL_FLOOR_MS,
  tooltipLines,
} from '../src/lib/hud/hud.js';
import { vegaConfig, scheme } from '../src/lib/hud/theme.js';

const visits = {
  columns: [
    { name: 'day', type: 'date' },
    { name: 'site', type: 'string' },
    { name: 'visits', type: 'integer' },
  ],
  rows: [
    ['2031-04-14', 'North', 10],
    ['2031-04-14', 'South', 4],
    ['2031-04-15', 'North', 6],
    ['2031-04-15', null, 2],
  ],
};

// Rows to records, by column name; a missing cell is null, never undefined.
const rows = records(visits);
assert.deepEqual(rows[0], { day: '2031-04-14', site: 'North', visits: 10 });
assert.equal(records({ columns: [{ name: 'a' }, { name: 'b' }], rows: [[1]] })[0].b, null);
assert.deepEqual(records(undefined), [], 'no dataset is no rows');

// A filter offers each value once, in order, and never null.
assert.deepEqual(filterOptions(rows, 'site'), ['North', 'South']);

// The host narrows rows; a filter on another dataset, or one set to all, is no
// filter here.
const filters = [
  { id: 'site', label: 'Site', dataset: 'visits', field: 'site' },
  { id: 'other', label: 'Other', dataset: 'elsewhere', field: 'site' },
];
assert.equal(applyFilters('visits', rows, filters, { site: 'North' }).length, 2);
assert.equal(applyFilters('visits', rows, filters, { site: null }).length, 4);
assert.equal(applyFilters('visits', rows, filters, { other: 'South' }).length, 4);
assert.equal(applyFilters('visits', rows, filters, {}).length, 4);
// A row with no value for the column never matches a chosen value.
assert.ok(applyFilters('visits', rows, filters, { site: 'North' }).every((r) => r.site === 'North'));
// Numbers compare as the select holds them, as strings.
assert.equal(
  applyFilters('n', [{ k: 1 }, { k: 2 }], [{ id: 'k', dataset: 'n', field: 'k' }], { k: '2' }).length,
  1,
);

// KPIs: null over nothing, a real 0 for a count of nothing.
assert.equal(kpi(rows, { op: 'sum', field: 'visits' }), 22);
assert.equal(kpi(rows, { op: 'mean', field: 'visits' }), 5.5);
assert.equal(kpi(rows, { op: 'min', field: 'visits' }), 2);
assert.equal(kpi(rows, { op: 'max', field: 'visits' }), 10);
assert.equal(kpi(rows, { op: 'count' }), 4);
assert.equal(kpi(rows, { op: 'last', field: 'visits' }), 2);
assert.equal(kpi([], { op: 'count' }), 0, 'a count of nothing is zero');
for (const op of ['sum', 'mean', 'min', 'max', 'last']) {
  assert.equal(kpi([], { op, field: 'visits' }), null, `${op} over nothing is null`);
}
assert.equal(kpi([{ v: 'n/a' }], { op: 'sum', field: 'v' }), null, 'no number to sum is null');
assert.equal(kpi(rows, { op: 'median', field: 'visits' }), null, 'an op this build does not know');

// A dash for null, never a zero; a number trimmed to sense.
assert.equal(formatFigure(null), '—');
assert.equal(formatFigure(NaN), '—');
assert.equal(formatFigure(0), '0');
assert.equal(formatCell(null), '—');
assert.equal(formatCell('<b>x</b>'), '<b>x</b>', 'a cell is text, drawn as text');

// Span is clamped into the four-column grid.
assert.equal(spanOf({ span: 2 }), 2);
assert.equal(spanOf({}), 1);
assert.equal(spanOf({ span: 9 }), 4);
assert.equal(spanOf({ span: 'x' }), 1);

// The datasets a chart names, through layer and concat.
assert.deepEqual(
  datasetsNamed({ layer: [{ data: { name: 'a' }, mark: 'line' }, { data: { name: 'b' }, mark: 'point' }] }).sort(),
  ['a', 'b'],
);
assert.deepEqual(datasetsNamed({ hconcat: [{ data: { name: 'c' } }] }), ['c']);

// The browser's re-check: what would make a chart fetch, navigate or carry
// its own rows is not drawn.
const bar = { data: { name: 'visits' }, mark: 'bar', encoding: { x: { field: 'day' } } };
assert.equal(chartRefusal(bar), null);
assert.match(chartRefusal({ ...bar, data: { url: 'x.csv' } }), /url/);
assert.match(chartRefusal({ ...bar, data: { values: [{ a: 1 }] } }), /values/);
assert.match(chartRefusal({ ...bar, datasets: { visits: [] } }), /datasets/);
assert.match(chartRefusal({ ...bar, encoding: { href: { field: 'u' } } }), /href/);
assert.match(chartRefusal({ ...bar, mark: 'image' }), /image/);
assert.match(chartRefusal({ ...bar, mark: { type: 'image' } }), /image/);
assert.match(chartRefusal({ ...bar, config: {} }), /config/);
assert.match(chartRefusal({ ...bar, title: 'https://example.org/x' }), /address/);
assert.match(chartRefusal({ layer: [{ ...bar, data: { url: 'y' } }] }), /url/, 'at any depth');
assert.equal(
  chartRefusal({ ...bar, encoding: { y: { field: 'v', axis: { values: [0, 50, 100] } } } }),
  null,
  "an axis's values are tick positions, not data",
);
assert.equal(
  chartRefusal({ $schema: 'https://vega.github.io/schema/vega-lite/v6.json', ...bar }),
  null,
  "Vega-Lite's own marker is not fetched",
);
assert.notEqual(chartRefusal(null), null);
assert.notEqual(chartRefusal([bar]), null);

// Rows go in before compile, only the ones the chart names; the spec's own
// object is not touched; a single view fills its panel's width.
const view = viewFor(bar, { visits: rows, unrelated: [{ x: 1 }] });
assert.deepEqual(Object.keys(view.datasets), ['visits']);
assert.equal(view.datasets.visits.length, 4);
assert.equal(view.width, 'container');
assert.equal(bar.datasets, undefined, 'the spec is not mutated');
assert.deepEqual(viewFor(bar, {}).datasets.visits, [], 'a dataset not loaded yet is empty');
assert.equal(viewFor({ ...bar, width: 300 }, {}).width, 300, 'a chart that sized itself keeps it');
assert.equal(viewFor({ hconcat: [bar] }, {}).width, undefined, 'a composed view keeps its own sizing');

// Polling never faster than the floor.
assert.equal(pollEvery(5_000), POLL_FLOOR_MS);
assert.equal(pollEvery('nonsense'), POLL_FLOOR_MS);
assert.equal(pollEvery(120_000), 120_000);

// A tooltip is lines of text.
assert.deepEqual(tooltipLines({ site: 'North', visits: null }), ['site: North', 'visits: —']);
assert.deepEqual(tooltipLines(null), []);

// The theme: light or dark, and a narrow chart's legend wraps.
assert.equal(scheme('light'), 'light');
assert.equal(scheme('anything'), 'dark');
assert.equal(vegaConfig('dark').background, null);
assert.equal(vegaConfig('dark', true).legend.columns, 3);
assert.equal(vegaConfig('dark').legend.columns, undefined);
assert.notDeepEqual(vegaConfig('light').range.category, vegaConfig('dark').range.category);

// The gates (§4.1): the renderer's source carries no `{@html}`, no
// `innerHTML`, and no inline style — a dynamic value goes through the CSSOM.
const here = path.dirname(fileURLToPath(import.meta.url));
const dir = path.join(here, '../src/lib/hud');
const files = fs.readdirSync(dir).filter((f) => /\.(svelte|js)$/.test(f));
assert.ok(files.length >= 7, `the renderer's files: ${files}`);
for (const f of [...files.map((x) => path.join(dir, x)), path.join(here, '../src/hud-standalone.js')]) {
  const src = fs.readFileSync(f, 'utf8');
  const code = src.replace(/\/\/.*$/gm, '').replace(/\/\*[\s\S]*?\*\//g, '');
  const name = path.basename(f);
  assert.ok(!/\{@html/.test(code), `${name}: {@html}`);
  assert.ok(!/innerHTML|outerHTML|insertAdjacentHTML/.test(code), `${name}: an HTML string sink`);
  assert.ok(!/\sstyle\s*=/.test(code), `${name}: an inline style attribute`);
  assert.ok(!/style:[a-z-]+\s*=/.test(code), `${name}: a style: directive`);
  assert.ok(!/setAttribute\(\s*['"]style/.test(code), `${name}: setAttribute("style")`);
  assert.ok(!/\bhref\s*=/.test(code), `${name}: a link`);
}

console.log('hud: ok');
