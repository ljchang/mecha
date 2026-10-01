// The new-persona form's lists follow the lock.
//
// `npm test` in web/. Same rig as `incognito-switch.mjs`: the functions are
// read OUT of the component, so this exercises the text that ships.
//
// **Why.** The form read `/api/personas/authoring` once, when it opened, and
// `unlock()` / `relock()` refreshed only the persona grid — so unlocking with
// the form open never offered a locked character as a portrait, and
// relocking left one chosen (#425). The defect was the wiring, not a pure
// function, so the wiring is what this drives.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { authoringUrl, keptCharacter, personaName } from '../src/lib/persona.js';
import { idleSpan } from '../src/lib/autolock.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Personas.svelte'), 'utf8');

function readOut(marker) {
  const start = src.indexOf(marker);
  if (start < 0) throw new Error(`Personas.svelte no longer defines ${marker.trim()}`);
  const end = '\n  }\n';
  return src.slice(start, src.indexOf(end, start) + end.length);
}

const fns = [
  readOut('  async function startMaking() {'),
  readOut('  async function unlock() {'),
  readOut('  async function relock() {'),
  readOut('  async function rereadAuthoring() {'),
  readOut('  async function addNew() {'),
  readOut('  function armIdle(secs) {'),
  readOut('  function dropToken() {'),
  readOut('  function revoke(t) {'),
].join('\n');

// The autolock's watcher, recorded rather than run: what span it was armed
// with, and whether it was stopped.
const watched = [];
function watchIdle({ idleMs }) {
  const w = { idleMs, stopped: false };
  watched.push(w);
  return () => (w.stopped = true);
}

// The server's rule (`PersonaChat::authoring`): locked characters only to an
// unlocked page. `stella` is locked.
const LISTS = (unlocked) => ({
  relationships: [], groups: [],
  characters: unlocked ? ['john', 'maya', 'stella'] : ['john', 'maya'],
});

// A page with the form open, its lists read under `unlocked`, and a fetch
// that answers as the server would. `hold` delays the authoring answer so a
// test can close the form while it is in flight.
function page({ unlocked, character, hold, holdIf = () => true, gate = null, formOpen = true, failIf = null }) {
  const fetch = async (url, init) => {
    if (url === '/api/library/unlock') return { ok: true, json: async () => ({ token: 't1', idle_secs: 900 }) };
    if (url === '/api/library/relock') return { ok: true, json: async () => ({}) };
    if (url === '/api/personas/relationships') {
      const g = gate?.(url);
      if (g) await g;
      return { ok: true, json: async () => ({}) };
    }
    if (url.startsWith('/api/personas/authoring')) {
      if (failIf?.(url)) return { ok: false, text: async () => 'server down' };
      if (hold && holdIf(url)) await hold;
      const g = gate?.(url);
      if (g) await g;
      const lists = LISTS(url.includes('unlock='));
      return { ok: true, json: async () => lists };
    }
    throw new Error(`unexpected fetch ${url} ${init?.method ?? 'GET'}`);
  };
  return new Function(
    'fetch', 'authoringUrl', 'keptCharacter', 'personaName', 'watchIdle', 'idleSpan', 'start',
    `'use strict';
     let token = start.unlocked ? 't0' : null;
     let authoring = start.lists;
     let making = start.formOpen
       ? { name: '', display: '', relationships: [], character: start.character, groups: [], locked: false }
       : null;
     let chosen = null, sheet = false, busy = false, password = '', error = '';
     let authoringGen = 0;
     let adding = null;
     let stopIdle = null;
     // The framing sheet, which a relock closes (review of #491).
     let framing = { frame: null };
     const TEMPLATE = (name) => '# ' + name;
     const back = () => {};
     // The grid's read takes a turn, as a real round trip does, so a list
     // cleared only after it would be seen on screen first.
     let loads = 0;
     const load = async () => { loads++; await new Promise((r) => setImmediate(r)); };
     ${fns}
     return {
       startMaking, unlock, relock, addNew,
       add: (a) => { adding = a; },
       close: () => { making = null; },
       get: () => ({ token, authoring, making, error, loads, framing }),
     };`,
  )(fetch, authoringUrl, keptCharacter, personaName, watchIdle, idleSpan, {
    unlocked, character, formOpen, lists: formOpen ? LISTS(unlocked) : null,
  });
}

// Unlocking with the form open offers the locked character.
{
  const p = page({ unlocked: false, character: 'maya' });
  await p.unlock();
  const { authoring, making, error } = p.get();
  assert.equal(error, '');
  assert.deepEqual(authoring.characters, ['john', 'maya', 'stella']);
  assert.equal(making.character, 'maya', 'an unlock keeps what was chosen');
}

// An unlock arms the autolock with the server's span, and a relock — the
// autolock's own or a tap — stops it, so a dead token is never relocked twice.
{
  watched.length = 0;
  const p = page({ unlocked: false, character: 'maya' });
  await p.unlock();
  assert.equal(watched.length, 1);
  assert.equal(watched[0].idleMs, 900_000);
  assert.equal(watched[0].stopped, false);
  await p.relock();
  assert.equal(watched[0].stopped, true);
  assert.equal(p.get().token, null);
  // A framing sheet open over the persona goes with the lock (review of #491).
  assert.equal(p.get().framing, null);
}

// Relocking takes it back out, and drops it if it was the chosen portrait.
{
  const p = page({ unlocked: true, character: 'stella' });
  await p.relock();
  const { authoring, making } = p.get();
  assert.deepEqual(authoring.characters, ['john', 'maya']);
  assert.equal(making.character, '', 'a hidden portrait is not sent');
}

// The form closed while the lists were in flight: nothing throws, nothing
// is written back into a form that no longer exists.
{
  let release;
  const hold = new Promise((r) => (release = r));
  const p = page({ unlocked: false, character: '', hold });
  const pending = p.unlock();
  await new Promise((r) => setImmediate(r));
  p.close();
  release();
  await pending;
  const { making, error, authoring } = p.get();
  assert.equal(making, null);
  assert.equal(error, '');
  assert.deepEqual(authoring.characters, ['john', 'maya']);
}

// Unlock, then relock before the unlocked lists arrive: the slow unlocked
// answer lands last and must not put `stella` back on a locked page (review
// of #425 — the lock button is live while a read is in flight).
{
  let release;
  const hold = new Promise((r) => (release = r));
  const p = page({ unlocked: false, character: '', hold, holdIf: (url) => url.includes('unlock=') });
  const unlocking = p.unlock();
  await new Promise((r) => setImmediate(r));
  await p.relock();
  release();
  await unlocking;
  const { token, authoring, making, error } = p.get();
  assert.equal(error, '');
  assert.equal(token, null);
  assert.deepEqual(authoring.characters, ['john', 'maya'], 'a stale unlocked read landed over the relock');
  assert.equal(making.character, '');
}

// Relock while the form is still opening under an unlock: the form opens
// with the locked lists, and the unlocked answer is never shown — not even
// for the round trip before the re-read lands.
{
  let releaseUnlocked, releaseLocked;
  const unlockedHeld = new Promise((r) => (releaseUnlocked = r));
  const lockedHeld = new Promise((r) => (releaseLocked = r));
  const gate = (url) => (url.includes('unlock=') ? unlockedHeld : lockedHeld);
  const p = page({ unlocked: true, character: '', gate, formOpen: false });
  const opening = p.startMaking();
  await new Promise((r) => setImmediate(r));
  const relocking = p.relock();
  releaseUnlocked();
  for (let i = 0; i < 5; i++) await new Promise((r) => setImmediate(r));
  const mid = p.get();
  assert.ok(mid.making, 'the form opened');
  assert.ok(!mid.authoring?.characters?.includes('stella'), 'the unlocked list was shown on a relocked page');
  releaseLocked();
  await Promise.all([opening, relocking]);
  const { authoring, error } = p.get();
  assert.equal(error, '');
  assert.deepEqual(authoring.characters, ['john', 'maya']);
}

// Relock while a new relationship is being added, with a locked portrait
// chosen: the add's re-read must not overtake the relock's and skip the
// portrait check, or the form sends a character the page now hides.
{
  let releasePost, releaseLists;
  const posted = new Promise((r) => (releasePost = r));
  const listed = new Promise((r) => (releaseLists = r));
  const gate = (url) => (url === '/api/personas/relationships' ? posted : listed);
  const p = page({ unlocked: true, character: 'stella', gate });
  p.add({ kind: 'relationship', name: 'Mentor', text: '' });
  const adding = p.addNew();
  await new Promise((r) => setImmediate(r));
  const relocking = p.relock();
  releasePost();
  for (let i = 0; i < 5; i++) await new Promise((r) => setImmediate(r));
  releaseLists();
  await Promise.all([adding, relocking]);
  const { authoring, making, error } = p.get();
  assert.equal(error, '');
  assert.deepEqual(authoring.characters, ['john', 'maya']);
  assert.equal(making.character, '', 'a locked portrait survived the relock');
  assert.deepEqual(making.relationships, ['mentor']);
}

// Relock: the unlocked list leaves the screen at once, not when the locked
// answer lands — and if that answer never comes, a locked pick is dropped.
{
  let release;
  const held = new Promise((r) => (release = r));
  const p = page({ unlocked: true, character: 'maya', gate: (url) => (url.includes('unlock=') ? null : held) });
  const relocking = p.relock();
  assert.deepEqual(p.get().authoring.characters, [], 'the unlocked list outlived the relock press');
  for (let i = 0; i < 3; i++) await new Promise((r) => setImmediate(r));
  assert.deepEqual(p.get().authoring.characters, [], 'the unlocked list stayed up while the relock read');
  assert.equal(p.get().making.character, 'maya', 'the choice waits for the answer');
  release();
  await relocking;
  assert.equal(p.get().making.character, 'maya', 'an unlocked pick survives a relock');
}
{
  const p = page({ unlocked: true, character: 'stella', failIf: (url) => !url.includes('unlock=') });
  await p.relock();
  const { authoring, making, error } = p.get();
  assert.equal(error, 'server down');
  assert.deepEqual(authoring.characters, []);
  assert.equal(making.character, '', 'a locked pick outlived a failed relock read');
}

console.log('persona-lock-lists: ok');
