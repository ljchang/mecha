// The still-out state and the clip under a row whose tool defers
// (docs/BACKGROUND-JOBS-DESIGN.md §8).
//
// `npm test` in web/. The page turns a tool's first line into a URL it
// fetches, so the clip matcher is held as strictly as the picture's: only a
// row the server marked `deferred`, only a finished call, only a plain
// chat-relative `.mp4` path. And a deferred row is still out only while its
// first line says `being made: `, keyed on the mark, never the words.
import { readFileSync } from 'node:fs';
import { clipOf, stillOut, makingText, slotText, repeatedClips, jobStarts } from '../src/lib/picture.js';
import { applyEvent } from '../src/lib/persona.js';
import { historyLines } from '../src/lib/call-lines.js';

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

// A persona's chat (§9): the live fold keeps the server's mark, so a clip
// still out has its state and Stop, and the clip plays once it lands.
{
  let state = { entries: [] };
  state = applyEvent(state, { type: 'tool', id: 'v1', name: 'video_generate' });
  state = applyEvent(state, { type: 'tool_result', id: 'v1', is_error: false, preview: making, deferred: true });
  is(stillOut(state.entries[0]), true, 'a persona row the server marked is still out');
  state = applyEvent(state, { type: 'tool_result', id: 'v1', is_error: false, preview: 'video: videos/v1.mp4', deferred: true });
  is(clipOf(state.entries[0]), 'videos/v1.mp4', 'and its clip plays once it lands');
  let plain = { entries: [] };
  plain = applyEvent(plain, { type: 'tool', id: 's1', name: 'shell' });
  plain = applyEvent(plain, { type: 'tool_result', id: 's1', is_error: false, preview: making });
  is(stillOut(plain.entries[0]), false, 'an unmarked row with the same words is not');
}

is(makingText({ name: 'image_generate' }), 'drawing a picture…', 'a picture keeps its words');
is(makingText({ name: 'video_generate' }), 'making a clip…', 'the video tool says it is making a clip');
is(makingText({ name: 'mail__draft' }), 'working on it…', 'anything else is not named');

{
  const lines = historyLines([
    row(making, { id: 'v1', name: 'video_generate' }),
    row('video: videos/v2.mp4', { id: 'v2', name: 'video_generate' }),
  ]);
  is(lines[0]?.text, 'making a clip…', 'the call screen says a clip is being made, not a picture');
  is(lines[1]?.text, 'a clip, in the chat', 'and where a finished clip is');
  const twice = historyLines([
    row('video: videos/v2.mp4', { id: 'v2', name: 'video_generate' }),
    row('video: videos/v2.mp4', { id: 'v3', name: 'video_generate' }),
  ]);
  is(twice.length, 1, 'one clip is noted once');
}

{
  // The call screen's one slot, whose one Stop stops everything out.
  const picture = { kind: 'tool', name: 'image_generate', is_error: false, preview: 'being made: images/a.png' };
  const clip = row(making, { name: 'video_generate' });
  is(slotText([picture]), 'drawing a picture…', 'the slot keeps a picture\'s words');
  is(slotText([clip]), 'making a clip…', 'and calls a clip alone no picture');
  is(slotText([clip, picture]), 'drawing a picture, and making a clip…', 'a picture beside a clip names both');
  const other = row(making, { name: 'mail__draft' });
  is(slotText([other, picture]), 'drawing a picture, and working on something else…', 'and beside other work says so');
  is(slotText([clip, other]), 'making a clip, and working on something else…', 'a clip beside other work names both');
  is(slotText([other, clip]), 'making a clip, and working on something else…', 'in either order');
  is(slotText([other, clip, picture]), 'drawing a picture, and making a clip, and working on something else…', 'and all three are named');
  is(slotText([row('video: videos/v1.mp4'), picture]), 'drawing a picture…', 'a finished clip is not out');
}

{
  const again = repeatedClips([
    row('video: videos/v1.mp4', { id: 'v1' }),
    row(making, { id: 'v2' }),
    row('video: videos/v1.mp4', { id: 'v3' }),
    row('video: videos/v4.mp4', { id: 'v4' }),
  ]);
  is([...again].join(','), '2', 'a clip a row above already plays is not played again');
  is(repeatedClips([row('video: videos/v1.mp4', { deferred: undefined }), row('video: videos/v1.mp4')]).size, 0, 'an unmarked row is no clip, so it repeats nothing');
}

// The clock on a still-out row (owner, 2026-10-15: the assistant's chat has
// one too): a running job's start from the server's duration, kept across
// queue events so it never steps back, and dropped once it stops running.
{
  const line = [
    { call_id: 'clip', running: true, elapsed_ms: 90_000 },
    { call_id: 'pic', running: false },
  ];
  const first = jobStarts(new Map(), line, 100_000);
  is(first.get('clip'), 10_000, 'a running job starts elapsed_ms before now');
  is(first.has('pic'), false, 'a waiting job has no clock');
  const later = jobStarts(first, [{ call_id: 'clip', running: true, elapsed_ms: 95_123 }], 105_000);
  is(later.get('clip'), 10_000, 'a known start is kept, not re-read each event');
  is(jobStarts(later, [{ call_id: 'pic', running: true, elapsed_ms: 0 }], 106_000).has('clip'), false, 'a job no longer running loses its clock');
  is(jobStarts(new Map(), [{ call_id: 'x', running: true }], 1).has('x'), false, 'no duration, no clock');
  is(jobStarts(null, null).size, 0, 'nothing out, nothing timed');
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
