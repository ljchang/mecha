// Every view keeps the reading measure one way: a full-bleed scroller that
// pads its content in with `--gutter` (app.css, `--measure`).
//
// `npm test` in web/. Plain node, like the other files here.
//
// **What is pinned.** No rule that pads with `var(--gutter)` (or
// `--gutter-gear`) may also set a `max-width`. On a wide window the gutter is
// `(100% - --measure) / 2` of the *whole* width; under a cap with border-box
// sizing that padding comes out of the cap, so the content is
// `cap - (W - measure)` and shrinks as the window grows. #mail/rules did
// exactly this with a 920px cap and rendered as a sliver on a large monitor.
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

let failed = 0;
const t = (name, ok) => {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}`);
  if (!ok) failed++;
};

const root = new URL('../src/', import.meta.url).pathname;
const files = [];
const walk = (dir) => {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p);
    else if (e.name.endsWith('.svelte')) files.push(p);
  }
};
walk(root);

const gutter = /padding[^;]*var\(--gutter(-gear)?\)/;
const offenders = [];
for (const f of files) {
  const src = readFileSync(f, 'utf8');
  for (const m of src.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)) {
    const css = m[1].replace(/\/\*[\s\S]*?\*\//g, '');
    // A declaration block is the text between a `{` and the next `}`;
    // nesting (a @media around rules) only adds empty-ish segments.
    for (const block of css.split('}')) {
      const decls = block.slice(block.lastIndexOf('{') + 1);
      if (gutter.test(decls) && /(^|[;\s])max-width\s*:/.test(decls)) {
        const sel = block.slice(0, block.lastIndexOf('{')).trim().split('\n').pop().trim();
        offenders.push(`${f.slice(root.length)}: ${sel}`);
      }
    }
  }
}

t(`found the components (${files.length})`, files.length > 10);
t('no gutter-padded rule also caps its width', offenders.length === 0);
for (const o of offenders) console.log(`     ${o}`);

if (failed) {
  console.log(`\n${failed} failed`);
  process.exit(1);
}
