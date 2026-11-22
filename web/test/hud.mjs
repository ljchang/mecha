// The dashboard renderer's pure half (src/lib/hud/hud.js) and the source
// gates `docs/LIVE-DASHBOARD-DESIGN.md` §4.1 names. `npm test` in web/.
//
// Pinned here: a filter narrows rows in the host, so a chart obeys it without
// naming it; a figure over nothing is null (a dash), and a count over nothing
// is 0; the browser re-check refuses what would make a chart fetch, and lets
// a style's `values` through; rows reach Vega before compile, through
// `datasets`. The typed GitHub reference (`GITHUB-DASHBOARD-DESIGN.md` §4)
// builds its one address only from vouched, well-formed values, on a tailnet
// board only — read off a real server render of the table. Then the gates: no
// `{@html}`, no `innerHTML`, no inline style, and one link sink, by name.
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
  validOwner,
  validRepoName,
  issueNumber,
  columnKind,
  columnHeader,
  refText,
  githubHref,
} from '../src/lib/hud/hud.js';
import { vegaConfig, scheme } from '../src/lib/hud/theme.js';
import { dataBase, filesSource } from '../src/lib/hud/source.js';

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
// A spec held in Svelte state arrives as a proxy, which `structuredClone`
// refuses; a Proxy stands in for it here.
const proxied = new Proxy(bar, {});
assert.throws(() => structuredClone(proxied));
assert.equal(viewFor(proxied, { visits: rows }).datasets.visits.length, 4, 'a proxied spec still draws');

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
// Text marks read on the page: the scheme's text colour, never Vega's black.
assert.equal(vegaConfig('dark').text.color, '#e9e9ed');
assert.notEqual(vegaConfig('light').text.color, vegaConfig('dark').text.color);
assert.equal(vegaConfig('dark', true).legend.columns, 3);
assert.equal(vegaConfig('dark').legend.columns, undefined);
assert.notDeepEqual(vegaConfig('light').range.category, vegaConfig('dark').range.category);

// Where a published page's rows are (§6.2): beside the versions, never in
// one; under the grant that opened the page; or beside a local preview.
assert.equal(dataBase('/b/hud-lab/v/3/'), '/b/hud-lab/data/');
assert.equal(dataBase('/b/hud-lab/v/3/index.html'), '/b/hud-lab/data/');
assert.equal(dataBase('/g/0a1b2c/'), '/g/0a1b2c/data/');
assert.equal(dataBase('/g/0a1b2c/index.html'), '/g/0a1b2c/data/');
assert.equal(dataBase('/preview/index.html'), './data/');
assert.equal(dataBase('/b/hud-lab/'), './data/', 'the share URL redirects to a version first');

assert.equal(dataBase('/g/0A1B2C/x'), '/g/0A1B2C/data/', 'either case of hex');

// R16: under a grant, a read that fails after one that worked is the grant
// ending — the page is told to sign in again and keeps what it has. A first
// read that fails, or a public page's, is an ordinary error.
{
  const answers = [];
  globalThis.fetch = async () => answers.shift();
  const ok = { status: 200, ok: true, json: async () => ({ rows: [] }), headers: { get: () => '"t"' } };
  const gone = { status: 404, ok: false };

  const granted = filesSource('/g/ab/data/', { granted: true });
  answers.push(gone);
  await assert.rejects(granted.dataset('x', null), (e) => !e.signIn, 'a grant never read is no sign-in');
  answers.push(ok, gone);
  assert.equal((await granted.dataset('x', null)).etag, '"t"');
  await assert.rejects(granted.dataset('x', '"t"'), (e) => e.signIn === true && /sign in again/.test(e.message));
  answers.push(gone);
  await assert.rejects(granted.dataset('y', null), (e) => !e.signIn, 'another dataset never seen is not the grant');

  const open = filesSource('/b/hud-lab/data/');
  answers.push(ok, gone);
  await open.dataset('x', null);
  await assert.rejects(open.dataset('x', null), (e) => !e.signIn, 'a public page has no grant to renew');

  answers.push({ status: 304, ok: false });
  assert.equal(await open.dataset('x', '"t"'), null, 'unchanged is null');
}

// ---- the typed reference (`docs/GITHUB-DASHBOARD-DESIGN.md` §4) ----

// GitHub's grammar, as `github.rs` holds it: nothing that walks or escapes.
assert.ok(validOwner('example-lab') && validOwner('a_b') && validOwner('A1'));
for (const bad of ['', '.', '..', 'a/b', 'a%2e', 'a b', 'a.b', 'x'.repeat(40), null, 7]) {
  assert.ok(!validOwner(bad), `owner ${JSON.stringify(bad)}`);
}
assert.ok(validRepoName('tool-a') && validRepoName('.github') && validRepoName('a..b') && validRepoName('x_y.z'));
for (const bad of ['', '.', '..', '%2e', '%2e%2e', 'a/b', '../x', 'a?b', 'a#b', 'a b', 'a\\b', 'x'.repeat(101), null, 7]) {
  assert.ok(!validRepoName(bad), `repo ${JSON.stringify(bad)}`);
}
assert.equal(issueNumber(12), 12);
for (const bad of [0, -1, 1.5, '12', NaN, Infinity, null, undefined, 2 ** 60]) {
  assert.equal(issueNumber(bad), null, `number ${String(bad)}`);
}

// A column is a field, the one reference shape, or nothing the page guesses at.
const ghRef = { ref: 'github', owner: 'owner', repo: 'name', number: 'number' };
assert.deepEqual(columnKind('title'), { kind: 'field', field: 'title' });
assert.equal(columnKind(ghRef).kind, 'github');
assert.equal(columnKind({ ...ghRef, ref: 'gitlab' }).kind, 'unknown', 'ref is a closed set');
assert.equal(columnKind({ ...ghRef, host: 'example.org' }).kind, 'unknown', 'no key beside the four');
assert.equal(columnKind({ ...ghRef, path: '/x' }).kind, 'unknown');
assert.equal(columnKind({ ref: 'github', owner: 'owner', repo: 'name' }).kind, 'unknown');
assert.equal(columnHeader(ghRef), 'GitHub');
assert.equal(columnHeader('title'), 'title');

// The address is the one template, built only from vouched, well-formed values.
const watchedRow = { title: 'A bug', owner: 'example-lab', name: 'tool-a', number: 7 };
const vouched = [{ ...ghRef, allowed: [['example-lab', 'tool-a', 7]] }];
assert.equal(githubHref(ghRef, watchedRow, vouched, true), 'https://github.com/example-lab/tool-a/issues/7');
assert.equal(refText(ghRef, watchedRow), '#7');
// Tailnet only: a published page or a proposal never sets `links`.
assert.equal(githubHref(ghRef, watchedRow, vouched, false), null, 'links off');
assert.equal(githubHref(ghRef, watchedRow, vouched, 'yes'), null, 'links is exactly true');
// No list, no link: the serve route did not vouch (a factory dataset has none).
assert.equal(githubHref(ghRef, watchedRow, undefined, true), null, 'no linkable list');
assert.equal(githubHref(ghRef, watchedRow, [], true), null);
assert.equal(
  githubHref(ghRef, { ...watchedRow, name: 'bait' }, vouched, true),
  null,
  'a repository the serve route did not vouch for',
);
assert.equal(
  githubHref(ghRef, watchedRow, [{ ...ghRef, repo: 'other', allowed: [['example-lab', 'tool-a', 7]] }], true),
  null,
  'a list for another reference vouches for nothing here',
);
// Even vouched, a value outside the grammar is text: the browser re-checks.
for (const [field, value] of [
  ['name', '..'],
  ['name', '.'],
  ['name', '%2e%2e'],
  ['name', 'tool-a/../../x'],
  ['owner', '..'],
  ['owner', 'example-lab/x'],
  ['number', 0],
  ['number', -3],
  ['number', '7'],
  ['number', 'seven'],
  ['number', 7.5],
]) {
  const row = { ...watchedRow, [field]: value };
  const list = [{ ...ghRef, allowed: [[row.owner, row.name, row.number]] }];
  assert.equal(githubHref(ghRef, row, list, true), null, `${field} = ${JSON.stringify(value)}`);
}
assert.equal(refText(ghRef, { ...watchedRow, number: 'seven' }), 'seven', 'a bad number is its own text');
assert.equal(refText(ghRef, { ...watchedRow, number: null }), '—');

// The table drawn for real — Svelte's server render of `Table.svelte` and
// `IssueLink.svelte` as they ship — so "no anchor" is read off the markup,
// not inferred from a function.
const here = path.dirname(fileURLToPath(import.meta.url));
{
  const { compile } = await import('svelte/compiler');
  const { render } = await import('svelte/server');
  const lib = path.join(here, '../src/lib/hud');
  const asModule = (code) => `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`;
  const serverModule = (file, replace) => {
    let code = compile(fs.readFileSync(path.join(lib, file), 'utf8'), { filename: file, generate: 'server' }).js.code;
    code = code.replace(/from (["'])(svelte(?:\/[^"']+)?)\1/g, (_, q, s) => `from ${q}${import.meta.resolve(s)}${q}`);
    for (const [from, to] of replace) code = code.split(from).join(to);
    return asModule(code);
  };
  const hudUrl = new URL('../src/lib/hud/hud.js', import.meta.url).href;
  const issueLink = serverModule('IssueLink.svelte', [["'./hud.js'", `'${hudUrl}'`]]);
  const { default: Table } = await import(
    serverModule('Table.svelte', [
      ["'./hud.js'", `'${hudUrl}'`],
      ["'./IssueLink.svelte'", `'${issueLink}'`],
    ])
  );
  const tableRows = [
    watchedRow,
    { title: 'Stranger', owner: 'stranger-org', name: 'bait', number: 1 },
    { title: 'Walks', owner: 'example-lab', name: '..', number: 2 },
    { title: 'Zero', owner: 'example-lab', name: 'tool-a', number: 0 },
  ];
  // Vouched for every row but the stranger's (which the serve route leaves
  // out), so only the renderer's own checks stand between `..` or `0` and an
  // anchor.
  const everything = [
    { ...ghRef, allowed: tableRows.filter((r) => r.owner !== 'stranger-org').map((r) => [r.owner, r.name, r.number]) },
  ];
  const draw = (props) => render(Table, { props: { columns: ['title', ghRef], rows: tableRows, ...props } }).body;
  const anchors = (html) => html.match(/<a\b[^>]*>/g) ?? [];

  const tailnet = anchors(draw({ linkable: everything, links: true }));
  assert.equal(tailnet.length, 1, `only the watched row links: ${tailnet}`);
  assert.match(tailnet[0], /href="https:\/\/github\.com\/example-lab\/tool-a\/issues\/7"/);
  assert.match(tailnet[0], /rel="noopener noreferrer"/);
  assert.match(tailnet[0], /referrerpolicy="no-referrer"/);
  const html = draw({ linkable: everything, links: true });
  const text = html.replace(/<[^>]*>/g, '');
  for (const t of ['A bug#7', 'Stranger#1', 'Walks#2', 'Zero0']) assert.ok(text.includes(t), `${t} drawn as text: ${text}`);
  assert.match(html, /<th[^>]*>GitHub<\/th>/);

  // A published board: the standalone page sets no `links`, and its datasets
  // (pushed by `hud::publish`, never through the serve route) carry no list.
  assert.deepEqual(anchors(draw({})), [], 'a published board renders no anchor');
  assert.deepEqual(anchors(draw({ linkable: everything })), [], 'links off, even with a list');
  assert.deepEqual(anchors(draw({ links: true })), [], 'links on, with no list');
  // The standalone page mounts the dashboard with no `links` at all, and the
  // dashboard's default is off.
  const standalone = fs.readFileSync(path.join(here, '../src/hud-standalone.js'), 'utf8');
  const mountCall = standalone.slice(standalone.indexOf('mount(Dashboard'));
  assert.ok(mountCall.startsWith('mount(Dashboard'), 'the standalone page mounts the dashboard');
  assert.ok(!/\blinks\b/.test(mountCall.slice(0, mountCall.indexOf(');'))), 'the standalone page turns links on');
  const dashboard = fs.readFileSync(path.join(lib, 'Dashboard.svelte'), 'utf8');
  assert.match(dashboard, /\blinks = false\b/, "the dashboard's links default is off");
}

// The gates (§4.1): the renderer's source carries no `{@html}`, no
// `innerHTML`, and no inline style — a dynamic value goes through the CSSOM.
const dir = path.join(here, '../src/lib/hud');
const files = fs.readdirSync(dir).filter((f) => /\.(svelte|js)$/.test(f));
assert.ok(files.length >= 7, `the renderer's files: ${files}`);
const scanned = [
  ...files.map((x) => path.join(dir, x)),
  path.join(here, '../src/hud-standalone.js'),
  // The route component draws board titles and refusal strings.
  path.join(here, '../src/lib/Hud.svelte'),
];
// Every way the renderer's source could send a click or a load somewhere.
// Markup and DOM writes, not mentions: `hud.js` names `href` as a refused
// Vega-Lite key, which is a string, not a sink.
const SINKS = {
  'an anchor': /<a[\s>/]/g,
  'an area': /<area[\s>/]/g,
  'an href attribute': /\bhref\s*=/g,
  'an href shorthand': /\{\s*href\s*\}/g,
  'a spread onto an element': /<[a-z][^>]*\{\s*\.\.\./g,
  'setAttribute("href")': /setAttribute\(\s*['"](?:xlink:)?href/g,
  'an href write': /\.href\s*=[^=]/g,
  'window.open': /\bwindow\s*\.\s*open\s*\(/g,
  'a location write': /\blocation\s*(?:\.\s*(?:assign|replace)\s*\(|(?:\.\s*href\s*)?=[^=])/g,
};
const sinksIn = (code) =>
  Object.entries(SINKS).flatMap(([what, re]) => [...code.matchAll(re)].map(() => what));
// The scan is not vacuous: each sink it names, it finds.
for (const [code, want] of [
  ['<a href="/x">x</a>', ['an anchor', 'an href attribute']],
  ['<a {href}>x</a>', ['an anchor', 'an href shorthand']],
  ['<area href=x>', ['an area', 'an href attribute']],
  ['<div {...props}></div>', ['a spread onto an element']],
  ["el.setAttribute('href', u)", ['setAttribute("href")']],
  ['a.href = u', ['an href attribute', 'an href write']],
  ['window.open(u)', ['window.open']],
  ['location.assign(u)', ['a location write']],
  ['location = u', ['a location write']],
  ['location.href = u', ['a location write', 'an href attribute', 'an href write']],
  ["const NEVER = new Set(['url', 'href'])", []],
  ['if (location.pathname === x) {}', []],
]) {
  assert.deepEqual(sinksIn(code).sort(), [...want].sort(), `the scan reads ${code}`);
}
// The one allowed sink (`GITHUB-DASHBOARD-DESIGN.md` §4), by name: one
// anchor whose address is `githubHref`'s, in `IssueLink.svelte`. A second
// anywhere — another anchor, another address — fails here.
const ALLOWED_SINK = 'href={githubHref(reference, row, linkable, links)}';
const found = [];
for (const f of scanned) {
  const src = fs.readFileSync(f, 'utf8');
  // Whole-line comments and block comments only: a `//` inside a string
  // (`'https://x'`) must not blank the code after it on the same line.
  const code = src.replace(/^\s*\/\/.*$/gm, '').replace(/\/\*[\s\S]*?\*\//g, '');
  const name = path.basename(f);
  assert.ok(!/\{@html/.test(code), `${name}: {@html}`);
  assert.ok(!/innerHTML|outerHTML|insertAdjacentHTML/.test(code), `${name}: an HTML string sink`);
  assert.ok(!/\sstyle\s*=/.test(code), `${name}: an inline style attribute`);
  assert.ok(!/style:[a-z-]+\s*=/.test(code), `${name}: a style: directive`);
  assert.ok(!/setAttribute\(\s*['"]style/.test(code), `${name}: setAttribute("style")`);
  for (const what of sinksIn(code)) found.push(`${name}: ${what}`);
  if (name === 'IssueLink.svelte') {
    assert.equal(code.split(ALLOWED_SINK).length - 1, 1, `${name}: the one link is githubHref's`);
    assert.equal((code.match(/\bhref\s*=/g) ?? []).length, 1, `${name}: one href`);
  }
  // One template builds the one address, and it is `githubHref`'s.
  const github = (code.match(/https:\/\/github\.com\//g) ?? []).length;
  assert.equal(github, name === 'hud.js' ? 1 : 0, `${name}: a GitHub address outside githubHref`);
}
assert.deepEqual(
  found.sort(),
  ['IssueLink.svelte: an anchor', 'IssueLink.svelte: an href attribute'],
  'the typed reference is the one link sink',
);

console.log('hud: ok');
