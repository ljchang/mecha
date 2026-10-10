// An edit made on the call screen is a call turn (owner's ruling, 2026-10-04).
//
// `npm test` in web/. Same rig as `call-mic.mjs`: the functions are read OUT
// of Personas.svelte, so this exercises the text that ships.
//
// What must hold: opening the edit from the call pauses the call's mic;
// sending it goes into the call as a typed line (`caller.say`) and never
// through the chat's own send; a call with no live line says so and sends
// nothing; and closing the modal, sent or not, gives the mic back.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));

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

const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Personas.svelte'), 'utf8');
const readOut = (marker) => {
  const start = src.indexOf(marker);
  if (start < 0) throw new Error(`Personas.svelte no longer defines ${marker.trim()}`);
  return src.slice(start, src.indexOf('\n  }\n', start) + 4);
};
const fns = ['  function editImage(path) {', '  function editInCall(path) {', '  function closeEdit() {', '  async function sendEdit({ text, mask, regions = [] }) {']
  .map(readOut)
  .join('\n');

function rig({ live = true } = {}) {
  return new Function(
    'live',
    `'use strict';
     const log = [];
     let key = 'k1', token = null, input = 'typed in the chat box', imageEdit = null;
     const caller = {
       holdMic: (on) => log.push(on ? 'hold' : 'release'),
       say: (text) => (log.push('say:' + text), live),
     };
     const pictureUrl = (p) => '/files/' + p;
     const uploadUrl = () => '/upload';
     const maskName = (p) => p + '.mask.png';
     const composeEditMessage = (p, mask, text) => (text ? 'edit ' + p + ': ' + text : null);
     const composeRegionsMessage = (p, idx, rs) => 'regions ' + p + ': ' + rs.map((r) => r.colour).join(',');
     const fetch = async () => ({ ok: true, json: async () => ({ path: 'inbox/mask.png' }) });
     const send = async (opts) => log.push('chat-send:' + input + (opts?.edit ? ' [edit ' + JSON.stringify(opts.edit) + ']' : ''));
     ${fns}
     return { editImage, editInCall, closeEdit, sendEdit, log, state: () => ({ imageEdit, input }) };`,
  )(live);
}

{
  const r = rig();
  r.editInCall('images/a.png');
  is(r.log, ['hold'], 'opening an edit from the call pauses the mic');
  is(r.state().imageEdit.initial, '', "the chat box's text does not ride into a call edit");
  await r.sendEdit({ text: 'make it night', mask: null });
  is(r.log, ['hold', 'say:edit images/a.png: make it night', 'release'], 'sent into the call as a typed line, then the mic is given back');
  is(r.state().imageEdit, null, 'the modal closes once the call has it');
  is(r.state().input, 'typed in the chat box', "the chat's own box and send are untouched");
}

{
  // Coloured regions from the chat (not a call): the index is uploaded and
  // each region's words go as fields beside it (M2).
  const r = rig();
  r.editImage('images/a.png');
  await r.sendEdit({
    text: '',
    mask: new Blob(['x']),
    regions: [
      { colour: 'magenta', words: 'make it red' },
      { colour: 'cyan', words: 'remove the cup' },
    ],
  });
  const sent = r.log.find((l) => l.startsWith('chat-send:')) ?? '';
  is(
    sent.includes('"regions":[{"colour":"magenta","words":"make it red"},{"colour":"cyan","words":"remove the cup"}]'),
    true,
    'regions go to the chat send as fields',
  );
  is(sent.includes('"mask":"inbox/mask.png"'), true, 'with the uploaded index as the mask');
}

{
  const r = rig({ live: false });
  r.editInCall('images/a.png');
  await r.sendEdit({ text: 'make it night', mask: null });
  const e = r.state().imageEdit;
  is([e?.busy, /not connected/.test(e?.error ?? '')], [false, true], 'no live line: said, and the modal stays to retry or close');
  is(r.log.includes('release'), false, 'and the mic stays paused while the modal is still open');
  r.closeEdit();
  is(r.log.at(-1), 'release', 'closing the modal gives the mic back');
}

{
  const r = rig();
  r.editImage('images/b.png');
  await r.sendEdit({ text: 'add a hat', mask: null });
  is(
    r.log,
    ['chat-send:edit images/b.png: add a hat [edit {"picture":"images/b.png","words":"add a hat"}]'],
    "an edit from the chat goes through the chat's send, with the edit as fields (IMAGE-DESIGN.md §5.3), the mic untouched",
  );
}

// The chat's own send puts `edit` on the wire only for the panel's turn, so
// the harness draws the change there and nowhere else (`persona::edit`).
{
  const sendSrc = readOut('  async function send({ edit = null, text: fixed = null } = {}) {');
  const run = new Function(
    `'use strict';
     const bodies = [];
     let input = '', attachments = [], key = 'k1', token = null, error = null;
     let regenerated = null;
     const chosen = { display: 'Mara' };
     const withAttachments = (typed) => typed;
     const chatUrl = (k, p) => '/api/persona-chat/' + k + p;
     const notice = () => {};
     const fetch = async (url, opts) => {
       bodies.push(JSON.parse(opts.body));
       return { ok: true, json: async () => ({ started: true }) };
     };
     ${sendSrc}
     return async (typed, opts) => { input = typed; regenerated = null; await send(opts); return { body: bodies.at(-1), regenerated }; };`,
  )();
  const edit = { picture: 'images/a.png', words: 'add a hat' };
  const panel = await run('Edit images/a.png: add a hat', { edit });
  is(panel.body.edit, edit, "the panel's turn carries its fields");
  // Any panel edit can come back a redraw, with a version_of only the
  // transcript carries (review of #634).
  is(panel.regenerated, 'k1', 'and the finished turn is read again for its version_of');
  const typed = await run('hello');
  is('edit' in typed.body, false, 'a typed turn sends no edit field');
  is(typed.regenerated, null, 'and owes no re-read');
}

// Regenerate (IMAGE-DESIGN.md §5.4): the picture showing goes out with
// `redraw` and nothing else, never a mask or regions (the server refuses the
// pair); the owner's draft and files stay in the composer; and nothing goes
// while a run is live, where a message would steer it as bare text.
{
  const sendSrc = readOut('  async function send({ edit = null, text: fixed = null } = {}) {');
  const regenSrc = readOut('  async function regenerate(root, picture) {');
  const make = (running) =>
    new Function(
      'running',
      `'use strict';
       const bodies = [];
       let input = 'half a thought', attachments = ['inbox/notes.pdf'], key = 'k1', token = null, error = null;
       let chosenVersion = new Map([['images/a.png', 'images/a.png']]);
       let regenerated = null;
       const run = { running };
       const chosen = { display: 'Mara' };
       const withAttachments = (typed, files) => [typed, ...files].join(' ');
       const chatUrl = (k, p) => '/api/persona-chat/' + k + p;
       const notice = () => {};
       const composeRegenerateMessage = (p) => 'Regenerate ' + p;
       const fetch = async (url, opts) => {
         bodies.push(JSON.parse(opts.body));
         return { ok: true, json: async () => ({ started: true }) };
       };
       ${sendSrc}
       ${regenSrc}
       return async () => {
         await regenerate('images/a.png', 'images/b.png');
         return { bodies, input, attachments, chosen: [...chosenVersion.keys()], regenerated };
       };`,
    )(running);
  const out = await make(false)();
  is(out.bodies.length, 1, 'Regenerate sends one turn');
  is(out.bodies[0].edit, { picture: 'images/b.png', redraw: true }, 'of the version showing, with redraw and no mask or regions');
  is(out.bodies[0].text, 'Regenerate images/b.png', 'and the line the owner sees, with no files riding along');
  is(out.bodies[0].attachments, [], 'the composer\'s files are not attached to it');
  is([out.input, out.attachments], ['half a thought', ['inbox/notes.pdf']], "the owner's draft and files stay in the composer");
  is(out.chosen, [], 'the card goes back to the newest, so the new version is what lands');
  is(out.regenerated, 'k1', 'and the finished turn is read again for its version_of');
  const busy = await make(true)();
  is(busy.bodies.length, 0, 'nothing is sent while a run is live');
}

// Regenerate on the call screen (owner, 2026-10-10: a spoken call turn):
// registered through the chat's own door first, then the server's line said
// into the call, never the chat's send; a call with no live line says so.
{
  const src2 = readOut('  async function regenerateInCall(path) {');
  const make = (live) =>
    new Function(
      'live',
      `'use strict';
       const log = [];
       let key = 'k1', token = null, regenerated = null;
       const caller = { say: (text) => (log.push('say:' + text), live) };
       const chatUrl = (k, p) => '/api/persona-chat/' + k + p;
       const notice = (text) => log.push('notice:' + text);
       const send = async () => log.push('chat-send');
       const fetch = async (url, opts) => {
         log.push('post:' + url + ' ' + opts.body);
         return { ok: true, json: async () => ({ line: 'Regenerate images/b.png' }) };
       };
       ${src2}
       return async () => { await regenerateInCall('images/b.png'); return { log, regenerated }; };`,
    )(live);
  const out = await make(true)();
  is(
    out.log,
    ['post:/api/persona-chat/k1/call-regenerate {"picture":"images/b.png"}', 'say:Regenerate images/b.png'],
    "the redraw is registered, then the server's line goes into the call, not the chat's send",
  );
  is(out.regenerated, 'k1', 'and the finished turn is read again for its version_of');
  const dead = await make(false)();
  is(dead.log.at(-1), 'notice:The call is not connected, so the picture was not drawn again.', 'a call with no live line says so');
  is(dead.regenerated, null, 'and owes no re-read');
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
