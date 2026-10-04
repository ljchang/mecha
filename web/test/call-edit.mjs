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
const fns = ['  function editImage(path) {', '  function editInCall(path) {', '  function closeEdit() {', '  async function sendEdit({ text, mask }) {']
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
     const fetch = async () => ({ ok: true, json: async () => ({ path: 'inbox/mask.png' }) });
     const send = async () => log.push('chat-send:' + input);
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
  is(r.log, ['chat-send:edit images/b.png: add a hat'], "an edit from the chat goes through the chat's send, the mic untouched");
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
