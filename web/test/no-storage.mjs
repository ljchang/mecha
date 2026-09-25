// The page stores nothing on the device (INCOGNITO-DESIGN §4.4).
//
// `npm test` in web/. An incognito chat's promise ends at the server unless
// the page keeps nothing either: no storage API anywhere in `web/src`, so a
// later convenience — a remembered draft, a cached transcript — cannot
// quietly write a conversation to the browser's disk. A preference that
// genuinely needs one belongs outside a chat's reach, and this test is where
// that argument has to be made.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, '..', 'src');
const STORAGE = /\b(localStorage|sessionStorage|indexedDB|caches\s*\.\s*open|document\s*\.\s*cookie|serviceWorker)\b/;

function* files(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, entry.name);
    if (entry.isDirectory()) yield* files(p);
    else if (/\.(js|mjs|svelte|ts)$/.test(entry.name)) yield p;
  }
}

let checked = 0;
const hits = [];
for (const file of files(root)) {
  checked++;
  fs.readFileSync(file, 'utf8')
    .split('\n')
    .forEach((line, i) => {
      if (STORAGE.test(line)) hits.push(`${path.relative(root, file)}:${i + 1}: ${line.trim()}`);
    });
}

// The negative is only worth something if the scan looked at the page.
if (checked < 10) {
  console.error(`FAIL  scanned only ${checked} files under ${root}`);
  process.exit(1);
}
if (hits.length) {
  for (const h of hits) console.error(`FAIL  storage API in web/src: ${h}`);
  process.exit(1);
}
console.log(`  ok    no storage API in ${checked} files under web/src`);
console.log('\n1 passed, 0 failed');
