// The page stores nothing on the device (INCOGNITO-DESIGN §4.4).
//
// `npm test` in web/. An incognito chat's promise ends at the server unless
// the page keeps nothing either: no storage API in anything the page ships,
// so a later convenience — a remembered draft, a cached transcript — cannot
// quietly write a conversation to the browser's disk.
//
// **What "the page" is.** Everything under `web/src`, *and* every module the
// bundle pulls in from outside it — found by walking the relative imports
// from `web/src/main.js`, because `Chat.svelte` imports
// `scripts/voice/voice-core.js`, and a directory walk of `web/src` alone
// passed green over the one file that does call `localStorage` (review of
// #326).
//
// **The one allowance, argued here rather than hidden.** `voice-core.js`
// keeps the owner's voice preferences (a voice name, a speed, a cached voice
// list) under its own keys — device settings, never conversation text, and
// voice is not offered in an incognito chat. A storage call is allowed there
// only on a line that names one of those keys, so a remembered draft added
// to the same module still fails.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const web = path.join(here, '..');
const src = path.join(web, 'src');
const repo = path.join(web, '..');
const STORAGE = /\b(localStorage|sessionStorage|indexedDB|caches\s*\.\s*open|document\s*\.\s*cookie|serviceWorker)\b/;
const ALLOWED = {
  [path.join(repo, 'scripts', 'voice', 'voice-core.js')]: /\b(LEGACY_)?PREFS_KEY\b/,
};

const files = new Set();
function* walk(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, entry.name);
    if (entry.isDirectory()) yield* walk(p);
    else if (/\.(js|mjs|svelte|ts)$/.test(entry.name)) yield p;
  }
}
for (const f of walk(src)) files.add(f);

// Relative imports, static and dynamic, followed wherever they lead.
const IMPORT = /(?:\bfrom\s*|\bimport\s*\(\s*|\bimport\s+)['"](\.{1,2}\/[^'"]+)['"]/g;
const queue = [path.join(src, 'main.js')];
const seen = new Set();
while (queue.length) {
  const file = queue.pop();
  if (seen.has(file)) continue;
  seen.add(file);
  files.add(file);
  for (const [, spec] of fs.readFileSync(file, 'utf8').matchAll(IMPORT)) {
    const target = path.resolve(path.dirname(file), spec);
    const resolved = [target, `${target}.js`, path.join(target, 'index.js')].find(
      (p) => fs.existsSync(p) && fs.statSync(p).isFile(),
    );
    if (resolved && /\.(js|mjs|svelte|ts)$/.test(resolved)) queue.push(resolved);
  }
}

const outside = [...files].filter((f) => !f.startsWith(src + path.sep));
const hits = [];
for (const file of files) {
  const allow = ALLOWED[file];
  fs.readFileSync(file, 'utf8')
    .split('\n')
    .forEach((line, i) => {
      if (STORAGE.test(line) && !(allow && allow.test(line))) {
        hits.push(`${path.relative(repo, file)}:${i + 1}: ${line.trim()}`);
      }
    });
}

// The negative is only worth something if the scan looked at the page — and
// at the module outside `web/src` that the review found it was missing.
const voice = path.join(repo, 'scripts', 'voice', 'voice-core.js');
if (files.size < 10 || !files.has(voice)) {
  console.error(`FAIL  the scan did not reach the page: ${files.size} files, voice-core.js ${files.has(voice) ? 'seen' : 'missed'}`);
  process.exit(1);
}
if (hits.length) {
  for (const h of hits) console.error(`FAIL  storage API in the page: ${h}`);
  process.exit(1);
}
console.log(`  ok    no storage API in ${files.size} files (${outside.length} outside web/src, reached by import)`);
console.log('\n1 passed, 0 failed');
