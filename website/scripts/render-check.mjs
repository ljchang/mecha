// Load every page of the embedded demo in a real browser and fail if one of
// them breaks.
//
// **Why this exists.** `check-demo.mjs` proves every endpoint has a fixture and
// `docusaurus build` proves the prose compiles, and a page can still be broken
// with both of them green — because neither executes the app's JavaScript.
// That is not hypothetical: `web/src/lib/Tasks.svelte` shipped in v0.1.16
// calling `stalled(t)`, a function nothing defines, where the server stamps a
// `stalled` *field*. `stateOf` runs on every card, so a board with any task at
// all rendered its header and no tasks, with a `ReferenceError` on the console.
// Nothing in the Rust suite covers it — the defect is entirely in the Svelte —
// and it was found by loading the page and looking at it. This is that, as a
// step that runs every time.
//
// **What counts as broken**, in three kinds, because they fail differently:
//
//   - An uncaught error or a console error. The `stalled` class.
//   - A page that drew almost nothing. A component that throws during render
//     leaves the shell and the nav behind, which looks like a page rather than
//     a crash — so a byte floor is what separates them.
//   - A page showing one of the app's own failure affordances. A fixture with
//     the wrong shape does not throw; it renders "could not reach the box", or
//     an empty pane, which reads to a docs reader as a feature that does
//     nothing.
//
// It also drives one scripted turn, because the docs claim a reader can watch a
// run happen and a static load does not check that claim.
//
// **Skipping is loud.** With no browser installed this warns and exits 0, so a
// docs-only checkout still builds — but `MECHA_DOCS_REQUIRE_BROWSER=1` turns
// that into a failure, and CI sets it. Same rule as the Rust integration tests
// and `MECHA_TEST_REQUIRE_BACKENDS=1`: in CI a silently skipped check reads
// exactly like a passing one.

import {createServer} from 'node:http';
import {readFile, stat} from 'node:fs/promises';
import {readFileSync, existsSync} from 'node:fs';
import {dirname, extname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const demo = resolve(here, '../static/demo');
const app = resolve(here, '../../web/src/App.svelte');
const REQUIRED = process.env.MECHA_DOCS_REQUIRE_BROWSER === '1';

function bail(message) {
  if (REQUIRED) {
    console.error(`render-check: ${message}`);
    process.exit(1);
  }
  console.warn(`render-check: ${message} — skipping (set MECHA_DOCS_REQUIRE_BROWSER=1 to fail)`);
  process.exit(0);
}

if (!existsSync(join(demo, 'index.html'))) {
  bail('static/demo is not built; run `npm run build-demo`');
}

let chromium;
try {
  ({chromium} = await import('playwright'));
} catch {
  bail('playwright is not installed');
}

// The routes come from the app itself, so a page added to `App.svelte` is
// checked without anyone remembering to add it here. The review sub-panes are
// listed separately because they are hash suffixes the view interprets rather
// than entries in that array, and the docs link straight into them.
const source = readFileSync(app, 'utf8');
const declared = /const views = \[([^\]]*)\]/.exec(source);
if (!declared) {
  bail('could not read the view list out of App.svelte');
}
const ROUTES = [
  ...[...declared[1].matchAll(/'([^']+)'/g)].map((m) => (m[1] === 'home' ? '' : m[1])),
  'review/graph',
  'review/frontdoor',
  // Settings' panes are hash suffixes too, and they hold nearly all of the
  // settings code — a gate that visits only `#settings` checks three rows and
  // a chevron.
  'settings/charter',
  'settings/learning',
  'settings/voice',
];

// Enough that a shell-plus-nav crash cannot clear it. The nav alone is about
// thirty characters; the thinnest real page (the front door, holding one
// request) came in at 153.
const FLOOR = 120;

// The app's own ways of saying it could not read something.
const DISTRESS = [
  'Could not reach the box',
  'demo: no fixture',
  'The live demo is not built',
  'could not look',
];

const TYPES = {
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.css': 'text/css',
  '.svg': 'image/svg+xml',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.json': 'application/json',
};

const server = createServer(async (request, response) => {
  const path = decodeURIComponent(new URL(request.url, 'http://x').pathname);
  const file = join(demo, path === '/' ? 'index.html' : path);
  // Containment, on the same rule every model-supplied path in this project
  // follows: resolve, then prove the result is still inside.
  if (!resolve(file).startsWith(demo)) {
    response.writeHead(403).end();
    return;
  }
  try {
    await stat(file);
    response.writeHead(200, {'content-type': TYPES[extname(file)] ?? 'application/octet-stream'});
    response.end(await readFile(file));
  } catch {
    response.writeHead(404).end();
  }
});

await new Promise((ok) => server.listen(0, '127.0.0.1', ok));
const base = `http://127.0.0.1:${server.address().port}/index.html`;

let browser;
try {
  browser = await chromium.launch();
} catch (error) {
  server.close();
  bail(`could not launch chromium (${String(error.message).split('\n')[0]}) — \`npx playwright install chromium\``);
}

const failures = [];
let checked = 0;

for (const route of ROUTES) {
  const context = await browser.newContext({viewport: {width: 420, height: 900}});
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(m.text());
  });

  await page.goto(base + (route ? `#${route}` : ''), {waitUntil: 'networkidle'});
  await page.waitForTimeout(700);
  const body = await page.innerText('body');
  const name = route || 'home';

  if (errors.length) failures.push(`${name}: ${errors[0].split('\n')[0]}`);
  if (body.length < FLOOR) failures.push(`${name}: drew only ${body.length} characters`);
  for (const needle of DISTRESS) {
    if (body.includes(needle)) failures.push(`${name}: shows "${needle}"`);
  }

  checked += 1;
  await context.close();
}

// The interactive claim: the docs say a reader can send a message and watch a
// turn run. A page that loads but whose scripted run never arrives would pass
// every check above.
{
  const context = await browser.newContext({viewport: {width: 420, height: 900}});
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto(`${base}#chat`, {waitUntil: 'networkidle'});
  await page.locator('textarea').last().fill('does the scripted run arrive?');
  await page.keyboard.press('Enter');
  await page.waitForTimeout(5000);
  const body = await page.innerText('body');
  for (const [what, ok] of [
    ['the typed turn', body.includes('does the scripted run arrive?')],
    ['a tool call', body.includes('recall')],
    ['the staging notice', body.includes('staged, not executed')],
    ['the draft card', body.includes('Re: Review request')],
  ]) {
    if (!ok) failures.push(`chat run: ${what} never appeared`);
  }
  if (errors.length) failures.push(`chat run: ${errors[0].split('\n')[0]}`);
  checked += 1;
  await context.close();
}

// Chat creation is a POST, before either transcript GET or EventSource.
// Delay the opening response, leave the view, then simulate a server restart:
// this measures cancellation and reconnects in the actual compiled component.
{
  const context = await browser.newContext({viewport: {width: 420, height: 900}});
  const page = await context.newPage();
  try {
    await page.goto(`${base}#home`, {waitUntil: 'networkidle'});
    await page.evaluate(() => {
      const originalFetch = globalThis.fetch;
      const OriginalSource = globalThis.EventSource;
      const probe = window.chatProbe = {opens: 0, reads: 0, streams: [], violations: [], opened: false};
      globalThis.fetch = async (url, options) => {
        if (/^\/api\/chat\/[^/]+$/.test(url)) {
          if (options?.method === 'POST') {
            probe.opens++;
            if (new Headers(options.headers).get('x-mecha-request') !== '1') probe.violations.push('unguarded open');
            if (probe.opens === 1) await new Promise(resolve => { probe.release = resolve; });
            probe.opened = true;
          } else {
            probe.reads++;
            if (!probe.opened) probe.violations.push('read before open');
          }
        }
        return originalFetch(url, options);
      };
      globalThis.EventSource = class extends OriginalSource {
        constructor(url) {
          super(url);
          if (!probe.opened) probe.violations.push('stream before open');
          probe.streams.push(this);
        }
      };
      location.hash = 'chat';
    });
    await page.waitForFunction(() => window.chatProbe.release, null, {timeout: 5000});
    await page.evaluate(() => { location.hash = 'home'; });
    await page.waitForTimeout(100);
    await page.evaluate(() => window.chatProbe.release());
    await page.waitForTimeout(100);
    const abandoned = await page.evaluate(() => window.chatProbe.reads + window.chatProbe.streams.length);
    if (abandoned !== 0) failures.push('chat connection: an abandoned open loaded or subscribed');
    await page.evaluate(() => { location.hash = 'chat'; });
    await page.waitForFunction(() => window.chatProbe.reads >= 1 && window.chatProbe.streams.length >= 1, null, {timeout: 5000});
    await page.evaluate(() => {
      window.chatProbe.opened = false; // the restarted server lost its session map
      window.chatProbe.streams.at(-1).onerror();
    });
    await page.waitForFunction(() => window.chatProbe.opens >= 3 && window.chatProbe.reads >= 2 && window.chatProbe.streams.length >= 2, null, {timeout: 5000});
    const violations = await page.evaluate(() => window.chatProbe.violations);
    if (violations.length) failures.push(`chat connection: ${violations.join(', ')}`);
  } catch (e) {
    failures.push(`chat connection: ${String(e.message).split('\n')[0]}`);
  }
  checked++;
  await context.close();
}

// Permanent opening refusals stop; transient failures back off. Exercise
// the compiled component with real timers so the retry policy is observable.
{
  const context = await browser.newContext({viewport: {width: 420, height: 900}});
  const page = await context.newPage();
  try {
    await page.goto(`${base}#home`, {waitUntil: 'networkidle'});
    await page.evaluate(() => {
      const originalFetch = globalThis.fetch;
      const probe = window.retryProbe = {mode: 'forbidden', attempts: []};
      globalThis.fetch = async (url, options) => {
        if (/^\/api\/chat\/[^/]+$/.test(url) && options?.method === 'POST') {
          probe.attempts.push(Date.now());
          if (probe.mode === 'forbidden') return new Response('not the owner', {status: 403});
          if (probe.attempts.length <= 2) return new Response('unavailable', {status: 503});
        }
        return originalFetch(url, options);
      };
      location.hash = 'chat';
    });
    await page.waitForFunction(() => window.retryProbe.attempts.length > 0);
    await page.waitForTimeout(1800);
    if (await page.evaluate(() => window.retryProbe.attempts.length !== 1)) {
      failures.push('chat retry: repeated a permanent opening refusal');
    }
    await page.evaluate(() => { location.hash = 'home'; });
    await page.waitForTimeout(100);
    await page.evaluate(() => {
      window.retryProbe.mode = 'transient';
      window.retryProbe.attempts = [];
      location.hash = 'chat';
    });
    await page.waitForFunction(() => window.retryProbe.attempts.length >= 3, null, {timeout: 8000});
    const times = await page.evaluate(() => window.retryProbe.attempts);
    if (times[1] - times[0] < 1400 || times[2] - times[1] < 2900) {
      failures.push('chat retry: transient opening failures did not back off');
    }
  } catch (e) {
    failures.push(`chat retry: ${String(e.message).split('\n')[0]}`);
  }
  checked++;
  await context.close();
}

// Two independent pages receive the same broadcast. Equal text from two
// requests must remain two messages, whichever arrives first: POST or SSE.
{
  const contexts = await Promise.all([browser.newContext(), browser.newContext()]);
  const pages = await Promise.all(contexts.map(context => context.newPage()));
  let order = 'event-first';
  let steered = false;
  let reject = false;
  const deferred = [];
  const broadcast = async (event) => Promise.all(pages.map(page => page.evaluate(ev => {
    window.syncProbe.source.onmessage({data: JSON.stringify(ev)});
  }, event)));
  try {
    for (const page of pages) {
      await page.exposeBinding('sendInput', async (_source, body) => {
        if (reject) return {error: 'server is shutting down'};
        const event = {type: steered ? 'queued' : 'user', text: body.text, request_id: body.request_id, spoken: false};
        if (order === 'event-first') {
          await broadcast(event);
          if (!steered) await broadcast({type: 'done', ok: true, stop: 'Completed'});
        } else deferred.push(event);
        return steered ? {steered: true} : {started: true};
      });
      await page.goto(`${base}#home`, {waitUntil: 'networkidle'});
      await page.evaluate(() => {
        const originalFetch = globalThis.fetch;
        const OriginalSource = globalThis.EventSource;
        window.syncProbe = {};
        globalThis.EventSource = class extends OriginalSource {
          constructor(url) { super(url); window.syncProbe.source = this; }
        };
        globalThis.fetch = async (url, options) => {
          if (/^\/api\/chat\/[^/]+\/send$/.test(url)) {
            const result = await window.sendInput(JSON.parse(options.body));
            return new Response(JSON.stringify(result), {status: result.error ? 503 : 200});
          }
          return originalFetch(url, options);
        };
        location.hash = 'chat';
      });
      await page.waitForFunction(() => window.syncProbe.source);
      await page.waitForTimeout(100);
    }
    const submit = async (page, text) => {
      await page.locator('textarea').last().fill(text);
      await page.keyboard.press('Enter');
    };
    const count = (page, text) => page.locator('.bubble').filter({hasText: text}).count();
    await submit(pages[0], 'cross-device identical input');
    await pages[0].waitForTimeout(150);
    for (const page of pages) {
      if (await count(page, 'cross-device identical input') !== 1) throw new Error('event-first input was missing or duplicated');
      if (await page.locator('textarea').last().getAttribute('placeholder') !== 'Ask mecha…') throw new Error('late POST response restarted a completed turn');
    }
    order = 'response-first';
    await submit(pages[1], 'cross-device identical input');
    await pages[1].waitForTimeout(150);
    for (const ev of deferred.splice(0)) await broadcast(ev);
    for (const page of pages) {
      if (await count(page, 'cross-device identical input') !== 2) throw new Error('equal inputs from different devices were merged or duplicated');
    }
    steered = true;
    order = 'event-first';
    await submit(pages[0], 'cross-device steering');
    await pages[0].waitForTimeout(150);
    for (const page of pages) {
      if (await count(page, 'cross-device steering') !== 1) throw new Error('steering was missing or duplicated');
    }
    reject = true;
    await submit(pages[0], 'rejected input');
    await pages[0].waitForTimeout(150);
    for (const page of pages) {
      if (await count(page, 'rejected input')) throw new Error('a rejected send appeared as accepted');
    }
  } catch (e) {
    failures.push(`chat synchronization: ${String(e.message).split('\n')[0]}`);
  }
  checked++;
  await Promise.all(contexts.map(context => context.close()));
}

await browser.close();
server.close();

// An empty denominator is a finding, not a pass. A regex that stopped matching
// `App.svelte` would otherwise report success having checked nothing.
if (checked === 0) {
  console.error('render-check: checked no pages at all — the route list came back empty');
  process.exit(1);
}

if (failures.length) {
  console.error(`render-check: ${failures.length} problem(s) across ${checked} checks:\n`);
  for (const failure of failures) console.error(`  ${failure}`);
  console.error('\nThe demo embeds the real app, so this is usually a bug in web/src — not in the fixtures.');
  process.exit(1);
}

console.log(`render-check: ${checked} checks, every page drew and none errored`);
