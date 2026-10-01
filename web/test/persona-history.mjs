// A persona's earlier chats: a failed read says so, a lapsed unlock is
// noticed, and a slow answer never lands under another persona.
//
// `npm test` in web/. The same rig as `persona-lock-lists.mjs`: the function
// is read OUT of the component, so this drives the text that ships.
//
// **Why.** `loadHistory` turned every failure into `[]`. An unlock lives in
// `mecha serve`'s memory, so after a restart the page kept a dead token, the
// list answered 404, and a persona with chats read as one with none — the
// "past chats don't load" report of 2026-10-01.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { personaUrl } from '../src/lib/persona.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Personas.svelte'), 'utf8');
const marker = '  async function loadHistory() {';
const start = src.indexOf(marker);
if (start < 0) throw new Error('Personas.svelte no longer defines loadHistory');
const fn = src.slice(start, src.indexOf('\n  }\n', start) + 4);

const CHATS = [{ id: 'a', created: '2026-09-28T16:12:00Z' }];

function page({ token = null, answer }) {
  const calls = { load: 0 };
  const fetch = async (url) => answer(url);
  return new Function(
    'fetch', 'personaUrl', 'calls', 'start',
    `'use strict';
     let chosen = { name: 'mara' };
     let token = start.token;
     let history = [], historyLoading = false, historyNote = '';
     let historyGen = 0;
     const loadSources = () => {};
     // The list's re-read: a lapsed token is dropped, and a persona it hid
     // closes.
     const load = async () => { calls.load++; token = null; chosen = null; };
     ${fn}
     return {
       loadHistory,
       choose: (name) => { chosen = { name }; },
       get: () => ({ history, historyLoading, historyNote, token, chosen }),
     };`,
  )(fetch, personaUrl, calls, { token });
}

const ok = (body) => ({ ok: true, status: 200, json: async () => body, text: async () => '' });
const notFound = { ok: false, status: 404, json: async () => ({}), text: async () => 'no such persona\n' };

// Read, and nothing said.
{
  const p = page({ answer: () => ok({ chats: CHATS }) });
  await p.loadHistory();
  assert.deepEqual(p.get().history, CHATS);
  assert.equal(p.get().historyNote, '');
  assert.equal(p.get().historyLoading, false);
}

// A 404 while holding a token re-reads the list, which learns the unlock
// lapsed — never an empty list standing in for one.
{
  const p = page({ token: 'dead', answer: () => notFound });
  const before = p.get().history;
  await p.loadHistory();
  assert.equal(p.get().token, null, 'the list re-read dropped the dead token');
  assert.equal(p.get().chosen, null, 'and closed the persona it hid');
  assert.equal(p.get().history, before);
}

// A failure on a visible persona is said, and what was listed stays.
{
  let fail = false;
  const p = page({ answer: () => (fail ? { ok: false, status: 500, text: async () => 'disk on fire' } : ok({ chats: CHATS })) });
  await p.loadHistory();
  fail = true;
  await p.loadHistory();
  assert.deepEqual(p.get().history, CHATS, 'a failed re-read keeps the list');
  assert.match(p.get().historyNote, /could not be read: disk on fire/);
}

// A network error is said too.
{
  const p = page({ answer: () => { throw new Error('offline'); } });
  await p.loadHistory();
  assert.match(p.get().historyNote, /offline/);
  assert.equal(p.get().historyLoading, false);
}

// A slow answer for the last persona never lands under the next one.
{
  let release;
  const slow = new Promise((r) => (release = r));
  const p = page({
    answer: async (url) => {
      if (url.includes('/mara/')) {
        await slow;
        return ok({ chats: CHATS });
      }
      return ok({ chats: [] });
    },
  });
  const first = p.loadHistory();
  p.choose('rook');
  await p.loadHistory();
  release();
  await first;
  assert.deepEqual(p.get().history, [], "mara's chats are not rook's");
}

console.log('persona-history: ok');
