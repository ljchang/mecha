// Behaviour checks for the chat's catch-up after a run it did not watch.
//
// `npm test` in web/. Plain node, same rig as `chat-drop.mjs`: the functions
// are read OUT of the component so this exercises the text that ships, and
// `fetch` is a stub whose answers the test releases one at a time — the
// order of arrival is the whole subject.
//
// **Why it needs a test.** The phone lost its chat on this side of the wire:
// a read taken mid-run replaced the transcript, and nothing re-read when the
// run ended. The re-read at `done` replaces the transcript again, so what it
// carries over decides what the owner still sees — drop a card and it is
// gone, carry a delivered message and it is drawn twice — and two reads on
// the wire at once decide which of them the page ends up showing.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Chat.svelte'), 'utf8');

function readOut(marker, end = '\n  }\n') {
  const start = src.indexOf(marker);
  if (start < 0) throw new Error(`Chat.svelte no longer defines ${marker.trim()}`);
  return src.slice(start, src.indexOf(end, start) + end.length);
}

const lifted = `${readOut('  const pageOnly = ', ';\n')}
  ${readOut('  function catchUp(sessionKey) {')}
  ${readOut('  async function load(')}
  ${readOut('  async function refreshTodo() {')}`;

// One page: the component's state as plain variables, and the reads it
// makes held until the test answers them.
function page() {
  const wire = [];
  const fetch = (url, opts) =>
    new Promise((resolve, reject) => wire.push({ url, opts, resolve, reject }));
  const p = new Function(
    'fetch',
    `let entries = [], running = false, partialRun = false, liveFrom = 0, doneSeq = 0;
     let task, incognito, todo, taint, model, mode, usage, error = null;
     let gone = false, key = 'k', viewSignal = null, loadGen = 0, todoGen = 0;
     let queue = [], queueSeq = 0;
     function closeIncognito(why) { gone = why; entries = []; }
     function scrollDown() {}
     ${lifted}
     return {
       load, catchUp, refreshTodo,
       get entries() { return entries; },
       get todo() { return todo; },
       push(e) { entries.push(e); },
       get partialRun() { return partialRun; },
       get gone() { return gone; },
       done() { doneSeq += 1; },
     };`
  )(fetch);
  p.wire = wire;
  return p;
}

const reply = (status, data = {}) => ({
  status,
  ok: status >= 200 && status < 300,
  json: async () => data,
  text: async () => 'nope',
});
const transcript = (texts, held = false) => ({
  entries: texts.map((text) => ({ kind: 'user', text })),
  running: held,
  held_by_run: held,
});
const texts = (p) => p.entries.map((e) => e.text ?? e.kind);
const settle = () => new Promise((r) => setTimeout(r, 0));

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
    console.log(`  FAIL  ${what}\n        got  ${a}\n        want ${b}`);
  }
}

// ---- what a re-read carries ----
{
  const pageOnly = new Function(`${readOut('  const pageOnly = ', ';\n')} return pageOnly;`)();
  is(pageOnly({ kind: 'draft', id: 'd1' }), true, 'a draft card is carried');
  is(pageOnly({ kind: 'notice', text: 'arrived too late — send again' }), true, 'a notice is carried');
  is(
    pageOnly({ kind: 'user', text: 'and blue', queued: true, delivery: 'discarded' }),
    true,
    'words the run never took are carried, beside the notice about them'
  );
  is(
    pageOnly({ kind: 'user', text: 'and blue', queued: true, delivery: 'queued' }),
    true,
    'a queued message not yet settled is carried'
  );
  is(
    pageOnly({ kind: 'user', text: 'and blue', queued: true, delivery: 'delivered' }),
    false,
    'a delivered message is in the transcript, so it is not carried twice'
  );
  for (const kind of ['user', 'assistant', 'tool', 'question']) {
    is(pageOnly({ kind }), false, `a ${kind} entry comes back from the read, not the carry`);
  }
}

// ---- the phone's case: a read mid-run, then the re-read at done ----
{
  const p = page();
  const first = p.load('k');
  p.wire.shift().resolve(reply(200, transcript(['earlier', 'make it blue'], true)));
  is(await first, true, 'a mid-run read replaces the transcript');
  is(texts(p), ['earlier', 'make it blue'], 'and it is the history, not nothing');
  is(p.partialRun, true, 'and the page knows it missed part of the run');

  p.push({ kind: 'draft', id: 'd1' });
  const again = p.catchUp('k');
  // Pushed while the re-read is on the wire: taken at replacement, so kept.
  p.push({ kind: 'notice', text: 'late notice' });
  p.wire.shift().resolve(reply(200, transcript(['earlier', 'make it blue', 'image: images/a.png'])));
  is(await again, true, 'the re-read at done replaces it again');
  is(
    texts(p),
    ['earlier', 'make it blue', 'image: images/a.png', 'draft', 'late notice'],
    'the whole run, then the cards — including one pushed during the read'
  );
  is(p.partialRun, false, 'and nothing is left to catch up');

  const third = p.catchUp('k');
  p.wire.shift().resolve(reply(200, transcript(['earlier', 'make it blue', 'image: images/a.png'])));
  await third;
  is(texts(p).filter((t) => t === 'draft').length, 1, 'carried cards stay page-only for the next re-read, once');
}

// ---- a re-read that fails or finds the chat gone carries nothing ----
{
  const p = page();
  const first = p.load('k');
  p.wire.shift().resolve(reply(200, transcript(['earlier'])));
  await first;
  p.push({ kind: 'draft', id: 'd1' });

  const broken = p.catchUp('k');
  p.wire.shift().resolve(reply(500));
  is(await broken, false, 'a failed read says it replaced nothing');
  is(texts(p), ['earlier', 'draft'], 'and the draft is not drawn twice');

  const ended = p.catchUp('k');
  p.wire.shift().resolve(reply(410));
  is(await ended, false, 'a read that finds the chat gone replaces nothing');
  is([p.gone, p.entries.length], ['closed', 0], 'and the ended chat stays empty in memory');
}

// ---- two reads on the wire: the older answer never wins ----
{
  const p = page();
  const older = p.load('k');
  const newer = p.catchUp('k');
  const [a, b] = p.wire.splice(0);
  b.resolve(reply(200, transcript(['earlier', 'reply'])));
  a.resolve(reply(200, transcript(['earlier'], true)));
  is([await older, await newer], [false, true], 'the read that started last is the one kept');
  is([texts(p), p.partialRun], [['earlier', 'reply'], false], 'the run it saw end stays ended');
}

// ---- a done that overtakes the read ----
{
  const p = page();
  const read = p.load('k');
  p.done();
  p.wire.shift().resolve(reply(200, transcript(['earlier'], true)));
  await read;
  await settle();
  is(p.wire.length, 1, 'a read that saw a run the page already heard end reads again');
  p.wire.shift().resolve(reply(200, transcript(['earlier', 'reply'])));
  await settle();
  is([texts(p), p.partialRun], [['earlier', 'reply'], false], 'and lands on the finished run');
}

// ---- the plan: a slow transcript read never puts back an older plan ----
{
  const p = page();
  const slow = p.load('k'); // mid-run: the read that carries the history
  const fresh = p.refreshTodo(); // the run revised its plan meanwhile
  const [t, plan] = p.wire.splice(0);
  plan.resolve(reply(200, { todo: [{ content: 'revised step' }] }));
  await fresh;
  t.resolve(reply(200, { ...transcript(['earlier'], true), todo: [{ content: 'old step' }] }));
  await slow;
  is(p.todo.map((s) => s.content), ['revised step'], 'the plan read that started last is the plan shown');
  is(texts(p), ['earlier'], 'and the slow read still delivers its history');
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
