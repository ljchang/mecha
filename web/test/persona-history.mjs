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
import { personaUrl, listUrl } from '../src/lib/persona.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Personas.svelte'), 'utf8');
function readOut(marker) {
  const start = src.indexOf(marker);
  if (start < 0) throw new Error(`Personas.svelte no longer defines ${marker.trim()}`);
  return src.slice(start, src.indexOf('\n  }\n', start) + 4);
}
// The shipped `loadHistory`, and the shipped `load` and `dropToken` it
// leans on when a read fails — so a lapsed unlock is caught by the page's
// own code, not by a stub that does what the test hopes. `personas` is
// `$derived(data?.personas ?? [])` in the component; spelled out here.
const fns = [
  readOut('  async function loadHistory() {'),
  readOut('  async function load() {'),
  readOut('  function dropToken() {'),
].join('\n').replaceAll('personas.find(', '(data?.personas ?? []).find(');

const CHATS = [{ id: 'a', created: '2026-09-28T16:12:00Z' }];

// `listed` is what the persona list holds for this request's token, and
// `unlocked` whether the server still honours it.
function page({ token = null, answer, listed = () => [{ name: 'mara' }], unlocked = () => !!token }) {
  const calls = { toList: 0, stopped: 0 };
  const fetch = async (url) => {
    if (url.startsWith('/api/personas?') || url === '/api/personas') {
      const t = url.includes('unlock=');
      return ok({ personas: listed(t), unlocked: t && unlocked() });
    }
    return answer(url);
  };
  return new Function(
    'fetch', 'personaUrl', 'listUrl', 'calls', 'start',
    `'use strict';
     let chosen = { name: 'mara', locked: !!start.token };
     let token = start.token;
     let data = null, error = '';
     let history = [], historyLoading = false, historyNote = '';
     let historyGen = 0;
     let usedInitial = true;
     const initial = null;
     let stopIdle = start.token ? () => calls.stopped++ : null;
     const loadSources = () => {};
     const startMaking = async () => {};
     const toList = () => { calls.toList++; chosen = null; };
     ${fns}
     return {
       loadHistory,
       choose: (name) => { chosen = { name }; },
       get: () => ({ history, historyLoading, historyNote, token, chosen, error }),
       calls,
     };`,
  )(fetch, personaUrl, listUrl, calls, { token });
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
// lapsed (a restart forgets every token): the token is dropped, its
// autolock stopped, and the persona it hid closed whole — never an empty
// list standing in for its chats.
{
  const p = page({
    token: 'dead',
    unlocked: () => false,
    listed: () => [],
    answer: () => notFound,
  });
  await p.loadHistory();
  const { token, chosen, historyNote } = p.get();
  assert.equal(token, null, 'the dead token is dropped');
  assert.equal(chosen, null, 'and the persona it hid is closed');
  assert.equal(p.calls.toList, 1, 'through toList, stream and all');
  assert.equal(p.calls.stopped, 1, 'its autolock stops with it');
  assert.equal(historyNote, '', 'a closed persona has no list to explain');
}

// A 404 for a persona still listed (a live token) is said, not hidden.
{
  const p = page({ token: 'live', unlocked: () => true, listed: () => [{ name: 'mara' }], answer: () => notFound });
  await p.loadHistory();
  assert.equal(p.get().token, 'live');
  assert.match(p.get().historyNote, /could not be read: no such persona/);
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

// A newer read can start while the last one's body is still arriving: the
// generation is checked after the body too (review of #469, pass 3).
{
  let release;
  const slowBody = new Promise((r) => (release = r));
  const p = page({
    answer: async (url) =>
      url.includes('/mara/')
        ? { ok: true, status: 200, json: async () => { await slowBody; return { chats: CHATS }; }, text: async () => '' }
        : ok({ chats: [] }),
  });
  const first = p.loadHistory();
  await new Promise((r) => setImmediate(r));
  p.choose('rook');
  await p.loadHistory();
  release();
  await first;
  assert.deepEqual(p.get().history, [], "mara's chats, slow in the body, are not rook's");
}

// Any failure while holding a token asks the list, not only a 404: a 500
// from a server that just restarted is as likely a lapse as anything.
{
  const p = page({
    token: 'dead',
    unlocked: () => false,
    listed: () => [],
    answer: () => ({ ok: false, status: 500, text: async () => 'restarting' }),
  });
  await p.loadHistory();
  assert.equal(p.get().token, null);
  assert.equal(p.calls.toList, 1);
}

console.log('persona-history: ok');
