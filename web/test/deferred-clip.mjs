// The still-out state and the clip under a row whose tool defers
// (docs/BACKGROUND-JOBS-DESIGN.md §8).
//
// `npm test` in web/. The page turns a tool's first line into a URL it
// fetches, so the clip matcher is held as strictly as the picture's: only a
// row the server marked `deferred`, only a finished call, only a plain
// chat-relative `.mp4` path. And a deferred row is still out only while its
// first line says `being made: `, keyed on the mark, never the words.
import { readFileSync } from 'node:fs';
import { clipOf, stillOut } from '../src/lib/picture.js';

let passed = 0;
let failed = 0;
function is(actual, expected, what) {
  if (actual === expected) {
    passed++;
    console.log(`  ok    ${what}`);
  } else {
    failed++;
    console.log(`  FAIL  ${what}\n        expected ${JSON.stringify(expected)}\n        got      ${JSON.stringify(actual)}`);
  }
}

const row = (preview, extra = {}) => ({ kind: 'tool', name: 'render', is_error: false, deferred: true, preview, ...extra });
const making = 'being made: `render` is running.';

is(stillOut(row(making)), true, 'a deferred row being made is still out');
is(stillOut(row(making, { deferred: undefined })), false, 'the same words from a tool that does not defer are not');
is(stillOut(row('video: clips/a.mp4')), false, 'a deferred row whose result landed is not still out');
is(stillOut(row(making, { is_error: true })), false, 'a failed one is not still out');
is(stillOut({ kind: 'tool', name: 'image_generate', is_error: false, preview: 'being made: images/a.png' }), true, 'a picture is still out by name, as before');

is(clipOf(row('video: clips/a.mp4')), 'clips/a.mp4', 'a finished clip is played from its path');
is(clipOf(row('video: web/chat-1/videos/v1-0001.mp4\ntook 4 min')), 'web/chat-1/videos/v1-0001.mp4', 'only the first line is read');
is(clipOf(row('video: clips/a.mp4', { deferred: undefined })), null, 'not on a row the server did not mark');
is(clipOf(row('video: clips/a.mp4', { is_error: true })), null, 'not on a failed call');
is(clipOf(row('video: clips/a.mp4', { pending: true })), null, 'not while the call is open');
is(clipOf(row('video: ../secret.mp4')), null, 'never a parent segment');
is(clipOf(row('video: /etc/a.mp4')), null, 'never an absolute path');
is(clipOf(row('video: .hidden/a.mp4')), null, 'never a dot segment');
is(clipOf(row('video: clips/a.mov')), null, 'only mp4');
is(clipOf(row('here is video: clips/a.mp4')), null, 'only a line that is the fact itself');
is(clipOf(row(making)), null, 'nothing while it is being made');
// The envelope as `jobs::settle` writes it, from the case the Rust test
// checks against `settle` itself — never retyped here.
const cases = JSON.parse(readFileSync(new URL('./deferred-clip-cases.json', import.meta.url), 'utf8'));
is(clipOf(row(cases.wrapped)), cases.clip, 'the settled, wrapped result plays as the live one did');
is(clipOf(row(cases.result)), cases.clip, 'and the live, unwrapped one');
is(
  clipOf(row(cases.wrapped.replace('---\n', ''))),
  null,
  'only the envelope the loop writes, line for line'
);

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
