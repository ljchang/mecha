// A streamed piece's bytes into samples, and the clock that places them
// (`pcm.js`): the parts of the play button's player that are not the browser.
import assert from 'node:assert/strict';
import { listenSession, pcmSamples, Schedule } from '../src/lib/pcm.js';

// Clock arithmetic is in floating point seconds.
const near = (a, b, what) => assert.ok(Math.abs(a - b) < 1e-9, `${what}: ${a} vs ${b}`);

// Little-endian signed 16-bit: 0, 1/2, -1, and the largest positive.
const bytes = new Uint8Array([0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0xff, 0x7f]);
const whole = pcmSamples(bytes);
assert.deepEqual(Array.from(whole.samples), [0, 0.5, -1, 32767 / 32768]);
assert.equal(whole.carry, null);

// A chunk that ends mid-sample keeps its odd byte for the next: the same
// samples whichever way the bytes were cut.
for (let cut = 0; cut <= bytes.length; cut += 1) {
  const a = pcmSamples(bytes.subarray(0, cut));
  const b = pcmSamples(bytes.subarray(cut), a.carry);
  assert.deepEqual([...a.samples, ...b.samples], Array.from(whole.samples), `cut at ${cut}`);
  assert.equal(b.carry, null, `cut at ${cut}`);
}
// A lone byte is all carry, no sample.
const lone = pcmSamples(new Uint8Array([0x01]));
assert.equal(lone.samples.length, 0);
assert.deepEqual(Array.from(lone.carry), [0x01]);
// A view into a larger buffer reads its own bytes, not the buffer's start.
const big = new Uint8Array([9, 9, 0x00, 0x40, 9]);
assert.deepEqual(Array.from(pcmSamples(big.subarray(2, 4)).samples), [0.5]);

// The clock: runs butt together while the voice is ahead of the listener...
const s = new Schedule(0.15);
near(s.place(10, 2), 10.15, 'first'); // nothing queued: a short lead
near(s.place(10.5, 1), 12.15, 'queued'); // queued: right after the last
near(s.ahead(11), 2.15, 'ahead');
// ...and when it falls behind, the next run starts a lead from now, never
// in the past and never on top of what is playing.
near(s.place(20, 1), 20.15, 'behind');
assert.equal(s.ahead(30), 0);
// A run due within the clock's slop of now is placed fresh, not at a start
// that has effectively passed.
const t = new Schedule(0.1);
t.place(0, 1); // ends at 1.1
near(t.place(1.095, 1), 1.195, 'slop');

// The phone's audio session: playback, so the ringer switch does not mute
// a reply — but never over a call's live microphone, which iOS's playback
// category would silence, and never over a type something else set.
assert.equal(listenSession('auto', false), 'playback');
assert.equal(listenSession('auto', true), null);
assert.equal(listenSession('play-and-record', false), null);
assert.equal(listenSession('playback', false), null);

console.log('pcm: ok');
