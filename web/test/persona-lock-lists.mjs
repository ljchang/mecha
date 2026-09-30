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
import { authoringUrl, keptCharacter } from '../src/lib/persona.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Personas.svelte'), 'utf8');

function readOut(marker) {
  const start = src.indexOf(marker);
  if (start < 0) throw new Error(`Personas.svelte no longer defines ${marker.trim()}`);
  const end = '\n  }\n';
  return src.slice(start, src.indexOf(end, start) + end.length);
}

const fns = [
  readOut('  async function unlock() {'),
  readOut('  async function relock() {'),
  readOut('  async function rereadAuthoring() {'),
].join('\n');

// The server's rule (`PersonaChat::authoring`): locked characters only to an
// unlocked page. `stella` is locked.
const LISTS = (unlocked) => ({
  relationships: [], groups: [],
  characters: unlocked ? ['john', 'maya', 'stella'] : ['john', 'maya'],
});

// A page with the form open, its lists read under `unlocked`, and a fetch
// that answers as the server would. `hold` delays the authoring answer so a
// test can close the form while it is in flight.
function page({ unlocked, character, hold, holdIf = () => true }) {
  const fetch = async (url, init) => {
    if (url === '/api/library/unlock') return { ok: true, json: async () => ({ token: 't1' }) };
    if (url === '/api/library/relock') return { ok: true, json: async () => ({}) };
    if (url.startsWith('/api/personas/authoring')) {
      if (hold && holdIf(url)) await hold;
      const lists = LISTS(url.includes('unlock='));
      return { ok: true, json: async () => lists };
    }
    throw new Error(`unexpected fetch ${url} ${init?.method ?? 'GET'}`);
  };
  return new Function(
    'fetch', 'authoringUrl', 'keptCharacter', 'start',
    `'use strict';
     let token = start.unlocked ? 't0' : null;
     let authoring = start.lists;
     let making = { name: '', display: '', relationships: [], character: start.character, groups: [], locked: false };
     let chosen = null, sheet = false, busy = false, password = '', error = '';
     let authoringGen = 0;
     const back = () => {};
     const load = async () => {};
     ${fns}
     return {
       unlock, relock,
       close: () => { making = null; },
       get: () => ({ token, authoring, making, error }),
     };`,
  )(fetch, authoringUrl, keptCharacter, { unlocked, character, lists: LISTS(unlocked) });
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

console.log('persona-lock-lists: ok');
