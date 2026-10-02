// The mic in a call follows mute and typing together, on both call panes.
//
// `npm test` in web/. Same rig as `incognito-switch.mjs`: the functions are
// read OUT of the components, so this exercises the text that ships.
//
// **Why.** "mic paused while you type" is a privacy claim. It once rested
// on the browser moving focus off the box when mute was tapped (review of
// #499, pass 5), and then on nothing at all after a reconnect, whose fresh
// session starts with the mic live (passes 6 and 7, the persona pane after
// the assistant's). The track is derived in one place, `applyMic`, from
// both flags, and every path that changes either flag or makes a new
// session goes through it.
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

// Each pane's names for the same three things.
const panes = [
  { file: 'Chat.svelte', session: 'vSession', muted: 'vMuted', typing: 'vTyping', start: '  function startVoice(' },
  { file: 'PersonaCall.svelte', session: 'session', muted: 'muted', typing: 'typing', start: '  export function start(' },
];

for (const pane of panes) {
  const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', pane.file), 'utf8');
  const readOut = (marker) => {
    const start = src.indexOf(marker);
    if (start < 0) throw new Error(`${pane.file} no longer defines ${marker.trim()}`);
    return src.slice(start, src.indexOf('\n  }\n', start) + 4);
  };
  const fns = ['  function applyMic() {', '  function toggleMute() {', '  function typingStart() {', '  function typingEnd() {']
    .map(readOut)
    .join('\n');
  const call = new Function(
    `'use strict';
     const mic = [];
     let ${pane.session} = { setMicEnabled: (on) => mic.push(on) };
     let ${pane.muted} = false, ${pane.typing} = false;
     ${fns}
     return { toggleMute, typingStart, typingEnd, applyMic, live: () => mic.at(-1) };`,
  )();
  const steps = (...fns) => fns.map((f) => (f(), call.live()));

  is(steps(call.typingStart, call.typingEnd), [false, true], `${pane.file}: typing pauses the mic and gives it back`);
  // Mute tapped while the box keeps focus: unmuting must not open the mic
  // under a hint that says it is paused.
  is(
    steps(call.typingStart, call.toggleMute, call.toggleMute, call.typingEnd),
    [false, false, false, true],
    `${pane.file}: mute and unmute while typing leave the mic paused`,
  );
  is(steps(call.toggleMute, call.typingStart, call.typingEnd, call.toggleMute), [false, false, false, true], `${pane.file}: typing while muted leaves it muted`);

  // A new session's mic starts live: the connect chain re-applies both flags
  // once the track exists.
  const start = readOut(pane.start);
  is(/\.connect\(\)\s*\.then\(applyMic\)\s*\.catch\(/.test(start), true, `${pane.file}: a (re)connected session gets the mic the page shows`);
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
