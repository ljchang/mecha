// Behaviour checks for the chat's catch-up after a run it did not watch.
//
// `npm test` in web/. Plain node, same rig as `chat-drop.mjs`: the predicate
// is read OUT of the component so this exercises the text that ships.
//
// **Why it needs a test.** The phone lost its chat on this side of the wire:
// a read taken mid-run replaced the transcript, and nothing re-read when the
// run ended. The re-read at `done` replaces the transcript again, so what it
// carries over decides what the owner still sees — drop a card and it is
// gone, carry a delivered message and it is drawn twice.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Chat.svelte'), 'utf8');

function readOut(marker, end) {
  const start = src.indexOf(marker);
  if (start < 0) throw new Error(`Chat.svelte no longer defines ${marker.trim()}`);
  return src.slice(start, src.indexOf(end, start) + end.length);
}

const pageOnly = new Function(
  `${readOut('  const pageOnly = ', ';\n')}
   return pageOnly;`
)();

let passed = 0;
let failed = 0;

function is(actual, expected, what) {
  if (actual === expected) {
    passed++;
    console.log(`  ok    ${what}`);
  } else {
    failed++;
    console.log(`  FAIL  ${what}: got ${actual}, want ${expected}`);
  }
}

is(pageOnly({ kind: 'draft', id: 'd1' }), true, 'a draft card survives the re-read');
is(
  pageOnly({ kind: 'notice', text: '1 queued message(s) arrived too late for this run — send again' }),
  true,
  'a notice survives the re-read'
);
is(
  pageOnly({ kind: 'user', text: 'and the blue one', queued: true, delivery: 'discarded' }),
  true,
  'words the run never took survive, beside the notice about them'
);
is(
  pageOnly({ kind: 'user', text: 'and the blue one', queued: true, delivery: 'queued' }),
  true,
  'a queued message not yet settled survives'
);
is(
  pageOnly({ kind: 'user', text: 'and the blue one', queued: true, delivery: 'delivered' }),
  false,
  'a delivered message is in the transcript, so it is not carried twice'
);
is(pageOnly({ kind: 'user', text: 'make it blue' }), false, 'a sent message comes back from the transcript');
is(pageOnly({ kind: 'assistant', text: 'done' }), false, 'a reply comes back from the transcript');
is(pageOnly({ kind: 'tool', name: 'image_generate' }), false, 'a tool call comes back from the transcript');
is(pageOnly({ kind: 'question', qid: 'q1' }), false, 'a question card comes back with the read');

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
