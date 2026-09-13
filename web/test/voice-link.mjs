import assert from 'node:assert/strict';
import { freshLink, linkVerdict, Pauses, INBOUND_STALL_MS, REPORT_STALL_MS } from '../../scripts/voice/voice-core.js';

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

// Two page reasons: the worker hears one `paused` and one `ok`.
p = new Pauses();
assert.equal(p.add('link').announce, true);
assert.equal(p.add('mic').announce, false);
assert.equal(p.remove('link').announce, false);
assert.equal(p.remove('mic').announce, true);
console.log('voice pause protocol: ok');

