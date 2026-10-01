// The autolock's page clock, imported from the shipped module, driven by a
// fake clock and a fake document: no timers, no browser.
import assert from 'node:assert/strict';
import { lapsed, watchIdle, autolockLine, ACTIVITY } from '../src/lib/autolock.js';

const MIN = 60_000;

assert.equal(lapsed(0, 15 * MIN - 1, 15 * MIN), false);
assert.equal(lapsed(0, 15 * MIN, 15 * MIN), true);
// No span is no autolock, never "lapsed at once".
assert.equal(lapsed(0, 10 * MIN, 0), false);

function world() {
  let t = 0;
  const target = new EventTarget();
  const doc = new EventTarget();
  doc.visibilityState = 'visible';
  return {
    target,
    doc,
    now: () => t,
    advance: (ms) => (t += ms),
    show() {
      doc.visibilityState = 'visible';
      doc.dispatchEvent(new Event('visibilitychange'));
    },
    hide() {
      doc.visibilityState = 'hidden';
      doc.dispatchEvent(new Event('visibilitychange'));
    },
  };
}

// A phone put down for longer than the span locks the moment it is shown
// again — its timers were frozen, so only the timestamp can tell — and the
// return hook (the token re-check) never runs for a lapsed page.
{
  const w = world();
  let idle = 0;
  let returned = 0;
  watchIdle({ idleMs: 15 * MIN, onIdle: () => idle++, onReturn: () => returned++, target: w.target, doc: w.doc, now: w.now, every: 0 });
  w.hide();
  w.advance(16 * MIN);
  w.show();
  assert.equal(idle, 1);
  assert.equal(returned, 0);
  // Once: it stopped watching when it fired.
  w.advance(16 * MIN);
  w.show();
  assert.equal(idle, 1);
}

// Use inside the span keeps it open; a short absence asks the server.
{
  const w = world();
  let idle = 0;
  let returned = 0;
  watchIdle({ idleMs: 15 * MIN, onIdle: () => idle++, onReturn: () => returned++, target: w.target, doc: w.doc, now: w.now, every: 0 });
  for (const ev of ACTIVITY) {
    w.advance(10 * MIN);
    w.target.dispatchEvent(new Event(ev));
  }
  w.hide();
  w.advance(5 * MIN);
  w.show();
  assert.equal(idle, 0, 'every kind of touch counts');
  assert.equal(returned, 1);
}

// A stopped watcher hears nothing.
{
  const w = world();
  let idle = 0;
  const stop = watchIdle({ idleMs: MIN, onIdle: () => idle++, target: w.target, doc: w.doc, now: w.now, every: 0 });
  stop();
  w.advance(2 * MIN);
  w.show();
  assert.equal(idle, 0);
}

assert.equal(autolockLine(1), '1 minute');
assert.equal(autolockLine(15), '15 minutes');
assert.equal(autolockLine(60), '1 hour');
assert.equal(autolockLine(120), '2 hours');
assert.equal(autolockLine(null), '');

console.log('autolock: ok');
