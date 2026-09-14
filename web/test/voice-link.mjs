import assert from 'node:assert/strict';
import { freshLink, linkVerdict, Pauses, serverPauseExpired, INBOUND_STALL_MS, REPORT_STALL_MS, SERVER_PAUSE_EXPIRY_MS } from '../../scripts/voice/voice-core.js';

// A working link: packets rise every tick, reports every second. No verdict.
let st = freshLink();
let t = 1_000_000;
for (let i = 0; i < 30; i++) {
  const v = linkVerdict(st, { packetsIn: 50 * i, reportAt: t - (t % 1000) }, t);
  assert.equal(v.stalled, null, `healthy tick ${i} produced a verdict`);
  st = v.next; t += 100;
}

// The 2026-09-12 shape: the far end goes quiet. Packets stop rising; the
// verdict arrives once, at the inbound window, and not again while it lasts.
const frozen = st.packetsIn;
let verdicts = [];
for (let i = 0; i < 40; i++) {
  const v = linkVerdict(st, { packetsIn: frozen, reportAt: st.reportAt }, t);
  if (v.stalled !== null) verdicts.push([v.stalled, v.reason, t - st.packetsAt]);
  st = v.next; t += 100;
}
assert.equal(verdicts.length, 1, 'a stall must be declared exactly once');
assert.equal(verdicts[0][0], true);
assert.equal(verdicts[0][1], 'inbound');
assert.ok(verdicts[0][2] >= INBOUND_STALL_MS && verdicts[0][2] < INBOUND_STALL_MS + 200, `declared at ${verdicts[0][2]}ms`);

// Packets resume: declared back exactly once, on the first rising tick.
verdicts = [];
for (let i = 1; i <= 10; i++) {
  const v = linkVerdict(st, { packetsIn: frozen + 50 * i, reportAt: t - (t % 1000) }, t);
  if (v.stalled !== null) verdicts.push(v.stalled);
  st = v.next; t += 100;
}
assert.deepEqual(verdicts, [false]);

// The other witness: inbound fine (the bot's silence keeps arriving) but the
// far end stops reporting that it hears us. Declared on the report window.
verdicts = [];
const lastReport = st.reportAt;
for (let i = 1; i <= 60; i++) {
  const v = linkVerdict(st, { packetsIn: st.packetsIn + 50, reportAt: lastReport }, t);
  if (v.stalled !== null) verdicts.push([v.stalled, v.reason, t - st.reportSeenAt]);
  st = v.next; t += 100;
}
assert.equal(verdicts.length, 1);
assert.equal(verdicts[0][1], 'report');
assert.ok(verdicts[0][2] >= REPORT_STALL_MS, `report stall declared early at ${verdicts[0][2]}ms`);

// Unknown is not a stall: a browser reporting neither counter never pauses.
st = freshLink();
for (let i = 0; i < 100; i++) {
  const v = linkVerdict(st, { packetsIn: null, reportAt: null }, t);
  assert.equal(v.stalled, null, 'a stall was declared from no evidence');
  st = v.next; t += 100;
}
console.log('voice link verdict: ok');

// The pause protocol between page and worker. Review of #226: the page
// echoed the worker's own pause back as a client `paused`, the worker held
// on the page's behalf, and neither side could let go.

// Worker-first: the worker announces; the page must not tell it anything.
let p = new Pauses();
let r = p.add('server');
assert.deepEqual([r.first, r.announce], [true, false], 'a worker pause was echoed back to the worker');
r = p.remove('server');
assert.deepEqual([r.last, r.announce], [true, false]);

// Page-first (the iOS screen-lock shape): the page pauses on the mic edge,
// tells the worker, the worker holds and announces, the page hears its own
// pause come back. On unmute the page must tell the worker `ok` even though
// the worker's echo is still in the set - that echo clears only once the
// worker, released, announces so.
p = new Pauses();
r = p.add('mic');
assert.deepEqual([r.first, r.announce], [true, true]);
r = p.add('server');
assert.deepEqual([r.first, r.announce], [false, false], 'the echo was treated as a new pause');
r = p.remove('mic');
assert.deepEqual([r.last, r.announce], [false, true], 'the worker was not told the page had let go');
assert.equal(p.first, 'server');
r = p.remove('server');
assert.deepEqual([r.last, r.announce], [true, false]);
assert.equal(p.any, false);

// Two page reasons: the worker hears each one begin and end by name, so
// it can hold them as a set and expire each by its own witness - a link
// pause followed by a mute must leave the worker holding for the mic once
// the link's hold has expired.
p = new Pauses();
assert.equal(p.add('link').announce, true);
assert.equal(p.add('mic').announce, true, 'the second local reason was not announced');
assert.equal(p.add('mic').announce, false, 'a repeat was announced');
assert.equal(p.remove('link').announce, true);
assert.equal(p.remove('mic').announce, true);
assert.equal(p.remove('mic').announce, false);
console.log('voice pause protocol: ok');

// A worker-announced pause is an uplink stall; only fresh uplink evidence
// (the far end's receiver report) may expire it without the worker's `ok`.
const T = 5_000_000;
const healthy = { packetsIn: 900, packetsAt: T - 100, reportAt: T - 500, reportSeenAt: T - 500, stalled: false };
assert.equal(serverPauseExpired(healthy, T - SERVER_PAUSE_EXPIRY_MS - 1, T), true);
assert.equal(serverPauseExpired(healthy, T - SERVER_PAUSE_EXPIRY_MS + 100, T), false, 'expired early');
assert.equal(serverPauseExpired(healthy, 0, T), false, 'no announcement, nothing to expire');
assert.equal(serverPauseExpired({ ...healthy, stalled: true }, T - 9000, T), false, 'expired while the page itself sees a stall');
assert.equal(serverPauseExpired({ ...healthy, reportSeenAt: null }, T - 9000, T), false, 'downlink alone cleared an uplink pause');
assert.equal(serverPauseExpired({ ...healthy, reportSeenAt: T - REPORT_STALL_MS }, T - 9000, T), false, 'a stale report counted as fresh');
assert.equal(serverPauseExpired({ ...healthy, packetsAt: T - INBOUND_STALL_MS }, T - 9000, T), false, 'stale inbound counted as fresh');
console.log('server pause expiry: ok');


// ---- the reliable uplink (docs/VOICE-LINK-DESIGN.md) -----------------------
import { UplinkRing, behindVerdict, BEHIND_TONE_MS, CAUGHT_UP_MS } from '../../scripts/voice/voice-core.js';

{
  // 20 ms Opus frames at the 48 kHz RTP clock: each frame's duration is the
  // gap to the next timestamp, so the first frame is held until the second.
  const ring = new UplinkRing(1000);
  ring.push(1000, new Uint8Array([1]).buffer);
  assert.equal(ring.frames.length, 0, 'a frame is held until its duration is known');
  for (let i = 1; i <= 6; i++) ring.push(1000 + 960 * i, new Uint8Array([i + 1]).buffer);
  assert.equal(ring.frames.length, 6);
  assert.equal(ring.pendingMs, 120);
  assert.equal(ring.nowMs, 120);
  const b = ring.takeBatch(100);
  assert.equal(b.frames.length, 5, 'a 100 ms batch is five 20 ms frames');
  assert.equal(b.ms, 0);
  assert.equal(typeof b.wallMs, 'number', 'a batch carries the wall clock its first frame was captured at');
  assert.equal(b.backlogMs, 20, 'what is left is the backlog');
  assert.equal(b.seq, 0);
  assert.equal(ring.takeBatch(100).frames.length, 1);
  assert.equal(ring.takeBatch(100), null);

  // A reconnect restarts the RTP clock; the media clock continues.
  ring.restart();
  assert.equal(ring.frames.length, 1, 'restart closes the held frame at 20 ms');
  ring.push(5, new Uint8Array([9]).buffer);
  ring.push(5 + 960, new Uint8Array([10]).buffer);
  assert.equal(ring.frames[ring.frames.length - 1].ms, 140, 'the new stream is placed after the old');

  // Past the cap the oldest goes, and the loss is recorded, not silent.
  const small = new UplinkRing(100);
  for (let i = 0; i <= 10; i++) small.push(960 * i, new Uint8Array([i]).buffer); // 10 closed frames = 200 ms
  assert.equal(small.pendingMs, 100);
  const dropped = small.takeDropped();
  assert.deepEqual(dropped, [{ fromMs: 0, toMs: 100 }], JSON.stringify(dropped));
  assert.deepEqual(small.takeDropped(), [], 'dropped spans are read once');
  console.log('uplink ring: ok');
}

{
  // The cue keys on how far behind, once per episode, never on packets.
  let st = { sounded: false }, cues = [];
  for (const ms of [0, 800, 2900, 3100, 4000, 9000, 1200, 500, 300, 0, 3500, 200]) {
    const v = behindVerdict(st, ms); st = v.next; if (v.cue) cues.push([ms, v.cue]);
  }
  assert.deepEqual(cues, [[3100, 'behind'], [500, 'caught'], [3500, 'behind'], [200, 'caught']], JSON.stringify(cues));
  assert.ok(BEHIND_TONE_MS > CAUGHT_UP_MS);
  console.log('behind cue: ok');
}

{
  // What the worker is told is the ring *and* the queue (review of #231):
  // with 32 kB queued at ~6 B/ms, an empty ring is still ~5 s behind.
  const { wireBacklogMs, UPLINK_WIRE_BYTES_PER_MS } = await import('../../scripts/voice/voice-core.js');
  assert.equal(wireBacklogMs(0, 0), 0);
  assert.equal(wireBacklogMs(1500, 0), 1500);
  assert.equal(wireBacklogMs(0, 32768), 32768 / UPLINK_WIRE_BYTES_PER_MS);
  assert.ok(wireBacklogMs(0, 32768) > 5000, 'a full SCTP queue is seconds, not nothing');

  // A batch the channel refused goes back, dropped spans and sequence with it.
  const ring = new UplinkRing(100_000);
  for (let i = 0; i <= 10; i++) ring.push(960 * i, new Uint8Array([i]).buffer);
  const small = new UplinkRing(60);
  for (let i = 0; i <= 5; i++) small.push(960 * i, new Uint8Array([i]).buffer); // 100 ms in, 40 dropped
  const dropped = small.takeDropped();
  assert.equal(dropped.length, 1);
  const b = small.takeBatch(100);
  assert.equal(small.frames.length, 0);
  small.unsend(b, dropped);
  assert.equal(small.frames.length, b.frames.length, 'frames back');
  assert.equal(small.pendingMs, 60, 'pending restored');
  assert.deepEqual(small.takeDropped(), dropped, 'the dropped record survives a failed send');
  assert.equal(small.takeBatch(100).seq, b.seq, 'the retry reuses the sequence number');
  console.log('wire backlog + unsend: ok');
}
