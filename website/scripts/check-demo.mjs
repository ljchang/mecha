// The drift guard on the docs demo.
//
// The demo answers `/api` from fixtures. When somebody adds a page to the web
// surface — or moves an endpoint — the demo does not break loudly: it returns
// a 501 that the component renders as its own error state, and a docs reader
// sees a page that looks like a broken feature rather than a broken demo. That
// failure is invisible to `docusaurus build`, which is why it needs a check.
//
// What it does: reads every `/api/…` path the components actually reach (the
// `fetch` calls and the one `EventSource`), asks the demo's real ROUTES table
// whether each one is answered, and fails if any is not. It asks the exported
// object rather than grepping the source, because a guard that re-implements
// what it is guarding drifts from it.
//
// It also checks the *shipped* bundle carries none of the fixtures, when one
// has been built. Rollup should drop the dynamic import behind a false
// `import.meta.env` constant — "should" being the word this project makes a
// test out of.

import {readFileSync, readdirSync, existsSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const web = resolve(here, '../../web');
const lib = join(web, 'src/lib');

const {ROUTES} = await import(join(web, 'src/demo/index.js'));

// `fetch('/api/x')`, `fetch(\`/api/chat/${key}\`)`, `new EventSource(...)`.
// An interpolation stands in for one path segment, which is exactly what the
// routes match with `[^/]+`.
const CALL = /(?:fetch|new EventSource)\(\s*[`'"]([^`'"]*)/g;

const paths = new Set();
const files = [join(web, 'src/App.svelte'), ...readdirSync(lib).map((f) => join(lib, f))];
for (const file of files) {
  if (!file.endsWith('.svelte')) continue;
  const source = readFileSync(file, 'utf8');
  for (const [, raw] of source.matchAll(CALL)) {
    if (!raw.startsWith('/api/')) continue;
    // Drop the query string and substitute interpolations with a placeholder
    // segment, so `/api/chat/${key}/send` is checked as a real path shape.
    const path = raw.split('?')[0].replace(/\$\{[^}]*\}/g, 'X');
    paths.add(path);
  }
}

const answered = (path) => ROUTES.some(([, pattern]) => pattern.test(path));
const missing = [...paths].filter((p) => !answered(p)).sort();

let failed = false;

if (missing.length) {
  failed = true;
  console.error('check-demo: the web app reaches endpoints the docs demo does not answer:\n');
  for (const path of missing) console.error(`  ${path}`);
  console.error(
    '\nAdd each to ROUTES in web/src/demo/index.js — with a fixture if a page reads it, ' +
      'or in the 501 group if it is a mutation the demo should decline.',
  );
} else {
  console.log(`check-demo: all ${paths.size} endpoints the app reaches are answered by fixtures`);
}

// The shipped bundle must not carry the demo. Only checkable when `web/dist`
// exists; a missing build is not a failure, because most runs of this script
// happen in a docs-only checkout.
const dist = join(web, 'dist/assets');
if (existsSync(dist)) {
  const leaked = readdirSync(dist)
    .filter((f) => f.endsWith('.js'))
    .filter((f) => readFileSync(join(dist, f), 'utf8').includes('Ostrander'));
  if (leaked.length) {
    failed = true;
    console.error(
      `check-demo: fixture data is in the SHIPPED bundle (${leaked.join(', ')}). ` +
        'The demo import is meant to be dropped when VITE_MECHA_DEMO is false.',
    );
  } else {
    console.log('check-demo: the shipped bundle carries no fixture data');
  }
}

process.exit(failed ? 1 : 0);
