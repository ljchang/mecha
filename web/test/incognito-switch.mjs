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
     let editing = { path: 'images/a.png', initial: 'KUMQUAT' };
     let pictureNote = { path: 'images/a.png', why: 'no such file' };
     let goneNote = 'incognito is unavailable: no local model';
     let entries = ['x'], streaming = 'y', usage = 1, taint = 1;
     let affect = 1, valence = 1, sawAffectThisRun = true;
     let partialRun = true, liveFrom = 3;
     const receivedInputs = new Set(), inputDelivery = new Map();
     const dropped = [];
     const dropRing = (k) => dropped.push(k);
     const INCOGNITO_PREFIX = 'incognito-';
     let vEntries = [{ who: 'user', text: 'KUMQUAT' }];
     let hungUp = 0;
     const endVoice = () => hungUp++;
     ${switchToSrc}
     return { switchTo, dropped, call: () => ({ hungUp, vEntries }), now: () => ({ key, draft, attachments, incognito, gone, todo, goneNote, partialRun, liveFrom, editing, pictureNote }) };`,
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
  is(s.editing, null, 'and an open edit modal, with its draft and paths');
  is(s.pictureNote, null, "and a download's note naming one of its pictures");
  is([s.gone, s.goneNote], [null, null], 'and the gone screen with its note');
  is(p.dropped, ['incognito-ab'], "and the audio its call buffered, by the chat's own key");
  is(p.call(), { hungUp: 1, vEntries: [] }, 'and a call still speaking into it, with its words');
}
{
  // Into an incognito chat with a recorded call live: the call ends rather
  // than going on under a page that says nothing is kept (review of #376).
  const p = page({ key: 'main', draft: '', attachments: [], incognito: false, gone: null });
  p.switchTo('incognito-cd');
  is(p.call().hungUp, 1, 'entering an incognito chat ends a call from a recorded one');
  is(p.dropped, [], "and leaves the recorded chat's ring for its next call");
}
{
  const p = page({ key: 'main', draft: 'half a thought', attachments: ['inbox/a.pdf'], incognito: false, gone: null });
  p.switchTo('chat-x1');
  const s = p.now();
  is([s.key, s.draft, s.attachments], ['chat-x1', 'half a thought', ['inbox/a.pdf']], 'an ordinary chat still carries its unsent text');
  // The new chat's catch-up state comes from its own first read, and
  // nothing in between acts on state describing a run in another chat. Left
  // set, the rail's belt (which reads `partialRun` on a timer, against the
  // new key) would spend one redundant transcript read on the chat you are
  // in.
  is([s.partialRun, s.liveFrom], [false, 0], "and what the catch-up knew of the last chat's run is gone");
  is(p.dropped, [], 'and keeps its call audio for a reconnect');
  is(p.call().hungUp, 0, 'and its call, which never crossed the incognito line');
  // But the edit modal is never portable: it holds a path in the chat it was
  // opened in, and sent from the next one it would name that picture in the
  // wrong transcript (review of #429).
  is(s.editing, null, 'while an open edit modal closes on any switch');
}
{
  const p = page({ key: 'incognito-ab', draft: 'KUMQUAT', attachments: [], incognito: true, gone: 'ended' });
  p.switchTo('incognito-ab');
  is(p.now().draft, 'KUMQUAT', 'switching to the same chat is a no-op');
}

// An incognito chat that ends takes its voice call with it: the call is
// hung up, and what the overlay showed and the ring buffered are gone
// (`forget`, which End and a server-side close both reach).
{
  const forgetSrc = readOut('  function forget() {');
  const s = new Function(
    `'use strict';
     let key = 'incognito-ab';
     let entries = ['x'], streaming = 'y', draft = 'z', attachments = ['a'], todo = ['t'];
     let editing = { path: 'images/a.png' };
     let pictureNote = { path: 'images/a.png', why: 'no such file' };
     let usage = 1, taint = 1, affect = 1, valence = 1;
     let vEntries = [{ who: 'user', text: 'KUMQUAT' }];
     let ended = 0;
     const dropped = [];
     const endVoice = () => ended++;
     const dropRing = (k) => dropped.push(k);
     ${forgetSrc}
     forget();
     return { ended, vEntries, dropped, entries, editing, pictureNote };`,
  )();
  is(s.ended, 1, 'ending an incognito chat hangs up its call');
  is(s.vEntries, [], "and clears the call's words from the overlay");
  is(s.dropped, ['incognito-ab'], 'and drops the audio it buffered');
  is(s.editing, null, 'and closes an open edit modal');
  is(s.pictureNote, null, "and forgets a download's note, which names one of its pictures");
}

// The overlay's promise describes the call, not the page: a call is bound to
// the chat it was opened in, and the page's `incognito` moves with every
// switch (review of #376).
{
  const top = src.slice(src.indexOf('<div class="voice-top">'), src.indexOf('<div class="voice-stage">'));
  is(top.includes('{#if vIncognito}') && !/\{#if incognito\}/.test(top), true, "the call overlay reads the call's own kind");
}

// A call's incognito-ness is read off the key it sends, not only the page's
// flag: after a switch the flag is false until the transcript read returns,
// and a tap in that window must still require the vouch (review of #376).
{
  const start = readOut('  function startVoice({ keep = false } = {}) {');
  is(
    start.includes('requireUnlogged: incognito || key.startsWith(INCOGNITO_PREFIX)') &&
      start.includes('vIncognito = incognito || key.startsWith(INCOGNITO_PREFIX)'),
    true,
    'a call into an incognito key requires the vouch before the page knows',
  );
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
