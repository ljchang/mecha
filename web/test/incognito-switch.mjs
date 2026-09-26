// Leaving an incognito chat takes nothing typed there with you.
//
// `npm test` in web/. Same rig as `denied-chip.mjs`: the function is read OUT
// of the component, so this exercises the text that ships.
//
// **Why.** The composer (`draft`) and the pending attachments are one piece
// of page state shared by every chat. End clears them (`forget`), but a
// switch from the drawer did not, so words typed into an incognito chat
// arrived in the next chat's composer — a recorded one, where pressing send
// writes them into a transcript (review of #326). An ordinary chat's unsent
// text still travels, as it always has; that half is pinned too, so the fix
// cannot quietly become a change to every chat.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Chat.svelte'), 'utf8');

function readOut(marker) {
  const start = src.indexOf(marker);
  if (start < 0) throw new Error(`Chat.svelte no longer defines ${marker.trim()}`);
  const end = '\n  }\n';
  return src.slice(start, src.indexOf(end, start) + end.length);
}

const switchToSrc = readOut('  function switchTo(k) {');

// A page whose state starts as given, with `switchTo` closed over it.
function page(start) {
  return new Function(
    'start',
    `'use strict';
     let { key, draft, attachments, incognito, gone } = start;
     let todo = ['[~] plan the thing'];
     let entries = ['x'], streaming = 'y', usage = 1, taint = 1;
     let affect = 1, valence = 1, sawAffectThisRun = true;
     const receivedInputs = new Set(), inputDelivery = new Map();
     ${switchToSrc}
     return { switchTo, now: () => ({ key, draft, attachments, incognito, gone, todo }) };`,
  )(start);
}

let passed = 0;
let failed = 0;
function is(actual, expected, what) {
  const a = JSON.stringify(actual);
  const b = JSON.stringify(expected);
  if (a === b) {
    passed++;
    console.log(`  ok    ${what}`);
  } else {
    failed++;
    console.log(`  FAIL  ${what}\n        expected ${b}\n        got      ${a}`);
  }
}

{
  const p = page({ key: 'incognito-ab', draft: 'KUMQUAT', attachments: ['inbox/x.txt'], incognito: true, gone: null });
  p.switchTo('main');
  const s = p.now();
  is([s.key, s.draft, s.attachments, s.incognito], ['main', '', [], false], 'leaving incognito clears the composer');
  is(s.todo, [], "and the incognito chat's plan");
}
{
  const p = page({ key: 'main', draft: 'half a thought', attachments: ['inbox/a.pdf'], incognito: false, gone: null });
  p.switchTo('chat-x1');
  const s = p.now();
  is([s.key, s.draft, s.attachments], ['chat-x1', 'half a thought', ['inbox/a.pdf']], 'an ordinary chat still carries its unsent text');
}
{
  const p = page({ key: 'incognito-ab', draft: 'KUMQUAT', attachments: [], incognito: true, gone: 'ended' });
  p.switchTo('incognito-ab');
  is(p.now().draft, 'KUMQUAT', 'switching to the same chat is a no-op');
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
