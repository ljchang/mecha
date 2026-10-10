// Regenerate in the assistant's chat (IMAGE-DESIGN.md §5.4; owner,
// 2026-10-10): the picture showing goes out as `regenerate` beside the line
// the owner sees, and the harness draws it before the model replies.
//
// `npm test` in web/. Same rig as `edit-send.mjs`: `send` and `regenerate`
// are read OUT of Chat.svelte, so this exercises the text that ships.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Chat.svelte'), 'utf8');
const readOut = (marker) => {
  const start = src.indexOf(marker);
  if (start < 0) throw new Error(`Chat.svelte no longer defines ${marker.trim()}`);
  return src.slice(start, src.indexOf('\n  }\n', start) + 4);
};
const fns = [
  '  async function send({ text: fixed = null, regenerate = null } = {}) {',
  '  async function regenerate(root, picture) {',
].map(readOut).join('\n');

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

// A page with a half-typed draft and a file in the composer.
function page({ running = false, status = 200 } = {}) {
  return new Function(
    'running',
    'status',
    `'use strict';
     const bodies = [], notes = [];
     let key = 'chat-ab', draft = 'half a thought', attachments = ['inbox/notes.pdf'];
     let chosenVersion = new Map([['images/a.png', 'images/a.png']]);
     let regenerated = null;
     const withAttachments = (typed, files) => [typed, ...files].join(' ');
     const composeRegenerateMessage = (p) => 'Regenerate ' + p;
     const closeIncognito = () => {};
     const receiveInput = () => {};
     const pushEntry = (e) => notes.push(e.text);
     const fetch = async (url, opts) => {
       bodies.push(JSON.parse(opts.body));
       return {
         status,
         ok: status === 200,
         json: async () => ({ started: true }),
         text: async () => 'a reply is running; Regenerate waits for it to finish',
       };
     };
     ${fns}
     return {
       regen: () => regenerate('images/a.png', 'images/b.png'),
       typed: (t) => { draft = t; return send(); },
       state: () => ({ bodies, notes, draft, attachments, chosen: [...chosenVersion.keys()], regenerated }),
     };`,
  )(running, status);
}

{
  const p = page();
  await p.regen();
  const s = p.state();
  is(s.bodies.length, 1, 'Regenerate sends one turn');
  const { request_id, ...body } = s.bodies[0];
  is(body, { text: 'Regenerate images/b.png', attachments: [], regenerate: 'images/b.png' }, 'of the version showing, as `regenerate`, with no files');
  is([s.draft, s.attachments], ['half a thought', ['inbox/notes.pdf']], "the owner's draft and files stay in the composer");
  is(s.chosen, [], 'the card goes back to the newest, so the new version is what lands');
  is(s.regenerated, 'chat-ab', 'and the finished turn is read again for its version_of');
}

{
  const p = page({ running: true });
  await p.regen();
  is(p.state().bodies.length, 0, 'nothing is sent while a reply runs');
}

{
  // The server refuses a Regenerate into a running turn (a steer is text only).
  const p = page({ status: 409 });
  await p.regen();
  const s = p.state();
  is(s.notes.length, 1, 'a refusal is said');
  is([s.chosen, s.regenerated], [['images/a.png'], null], 'and the card stays where it was, owing no re-read');
}

{
  const p = page();
  await p.typed('hello');
  const body = p.state().bodies[0];
  is('regenerate' in body, false, 'a typed turn sends no regenerate field');
  is(p.state().regenerated, null, 'and owes no re-read');
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
