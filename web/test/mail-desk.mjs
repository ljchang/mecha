// Behaviour checks for the desktop mail triage's logic (src/lib/mail-desk.js).
//
// `npm test` in web/. Plain node, like the other files here; mail-desk.js is a
// plain module, so it is imported rather than sliced out of a component.
//
// **What is worth pinning.** The lanes must read the store the way the TUI
// does, or the desk shows a different backlog than every other surface. Enter
// must never invent an action. And the undo window is the part where being
// wrong is silent: an action that commits early cannot be taken back, and one
// that never commits looks done and is not.
import {
  laneOf,
  sortRows,
  acceptVerb,
  keyOf,
  sweepGroups,
  ageOf,
  PendingActions,
  tickedGroups,
  splitSender,
} from '../src/lib/mail-desk.js';

let pass = 0;
let fail = 0;
const t = (name, cond) => {
  if (cond) {
    pass += 1;
    console.log('  ok   ', name);
  } else {
    fail += 1;
    console.log('  FAIL ', name);
  }
};

// ---- lanes ----
t('a classified respond thread needs you', laneOf({ state: 'classified', bucket: 'respond' }) === 'respond');
t('a classified notify thread is FYI', laneOf({ state: 'classified', bucket: 'notify' }) === 'notify');
t('an ignored thread is in no lane', laneOf({ state: 'classified', bucket: 'ignore' }) === null);
t('a failed classification needs you', laneOf({ state: 'failed' }) === 'respond');
t('parked and drafted have their own lanes', laneOf({ state: 'parked' }) === 'parked' && laneOf({ state: 'drafted' }) === 'drafted');
t('acted and dismissed are finished', laneOf({ state: 'acted' }) === null && laneOf({ state: 'dismissed' }) === null);
t('a state this page does not know shows up rather than vanishing', laneOf({ state: 'snoozed' }) === 'respond');
t('and so does a bucket it does not know', laneOf({ state: 'classified', bucket: 'later' }) === 'respond');

// ---- order ----
const sorted = sortRows([
  { id: 'a', urgency: 'week', date: '2026-09-20T00:00:00Z' },
  { id: 'b', urgency: '', date: '2026-09-23T00:00:00Z' },
  { id: 'c', urgency: 'today', date: '2026-09-01T00:00:00Z' },
  { id: 'd', urgency: 'week', date: '2026-09-22T00:00:00Z' },
]).map((r) => r.id).join('');
t('urgency first, then newest first', sorted === 'cdab');

// ---- Enter ----
t('Enter accepts a proposed archive', acceptVerb({ proposed: 'archive' }) === 'archive');
t('Enter accepts a proposed reply', acceptVerb({ proposed: 'reply' }) === 'reply');
t('Enter has nothing to accept on none', acceptVerb({ proposed: 'none' }) === null);
t('nor on a forward, which needs a recipient', acceptVerb({ proposed: 'forward' }) === null);
t('nor on an unclassified row', acceptVerb({}) === null);

// ---- identity ----
t('a thread id is scoped by its account', keyOf({ account: 'a', thread_id: 'x' }) !== keyOf({ account: 'b', thread_id: 'x' }));

// ---- sweep ----
const groups = sweepGroups(
  [
    { from: 'news@list.edu', from_name: 'Listserv', proposed: 'archive' },
    { from: 'news@list.edu', from_name: 'Listserv', proposed: 'archive' },
    { from: 'cal@x.com', from_name: '', proposed: 'archive' },
    { from: 'news@list.edu', from_name: 'Listserv', proposed: 'reply' },
    { from: 'spoof@evil.com', from_name: 'Listserv', proposed: 'archive' },
  ],
  'archive',
);
t('groups only the chosen proposal', groups.reduce((n, g) => n + g.rows.length, 0) === 4);
t('largest group first', groups[0].key === 'news@list.edu' && groups[0].rows.length === 2);
t('groups by address, not the attacker-chosen name', groups.filter((g) => g.name === 'Listserv').length === 2);
t('a nameless sender shows its address', groups.find((g) => g.key === 'cal@x.com').name === 'cal@x.com');

// ---- which sweep groups are ticked ----
{
  const g = (key) => ({ key, rows: [] });
  const marks = new Set(['b']);
  t('an archive sweep ticks every group but the marked', tickedGroups([g('a'), g('b')], 'archive', marks).map((x) => x.key).join() === 'a');
  t('a drafting sweep ticks only the marked', tickedGroups([g('a'), g('b')], 'reply', marks).map((x) => x.key).join() === 'b');
  // The regression: a group arriving after the tab opened (the minute's
  // reload, a new sender) must not be ticked for a drafting verb.
  t('a group that appears later is not ticked for a draft', tickedGroups([g('a'), g('b'), g('new')], 'reply', marks).every((x) => x.key !== 'new'));
  t('but is for an archive', tickedGroups([g('a'), g('new')], 'archive', new Set()).some((x) => x.key === 'new'));
}

// ---- senders ----
t('a Name <address> sender splits', JSON.stringify(splitSender('Tomas Lindqvist <editor@jac.example.org>')) === JSON.stringify({ name: 'Tomas Lindqvist', address: 'editor@jac.example.org' }));
t('a quoted name splits too', splitSender('"Barnett, Hollis" <hb@x.edu>').name === 'Barnett, Hollis');
t('a bare address is only an address', JSON.stringify(splitSender('it@x.edu')) === JSON.stringify({ name: '', address: 'it@x.edu' }));

// ---- age ----
const now = Date.parse('2026-09-23T12:00:00Z');
t('minutes', ageOf('2026-09-23T11:55:00Z', now) === '5m');
t('hours', ageOf('2026-09-23T09:00:00Z', now) === '3h');
t('days', ageOf('2026-09-20T12:00:00Z', now) === '3d');
t('an unreadable date is blank, not NaN', ageOf('soon', now) === '');

// ---- the undo window ----
// A fake clock: timers fire only when the test says so.
function fakeTimers() {
  let id = 0;
  const due = new Map();
  return {
    setTimeout: (fn) => (due.set(++id, fn), id),
    clearTimeout: (i) => due.delete(i),
    fireAll: () => {
      const fns = [...due.values()];
      due.clear();
      fns.forEach((f) => f());
    },
    pending: () => due.size,
  };
}
const settle = () => new Promise((r) => setTimeout(r, 0));
const row = (id) => ({ account: 'acct', thread_id: id });

{
  const timers = fakeTimers();
  const sent = [];
  const p = new PendingActions({ commit: async (it) => sent.push(it.row.thread_id), timers });
  p.add([{ row: row('1'), verb: 'archive' }], 'one');
  t('a held action hides its row', p.hiddenKeys().has(keyOf(row('1'))));
  t('and has not been sent', sent.length === 0);
  const undone = p.undo();
  t('undo returns what it took back', undone?.label === 'one');
  t('an undone action is never sent', (timers.fireAll(), await settle(), sent.length === 0));
  t('and its row is no longer hidden', !p.hiddenKeys().has(keyOf(row('1'))));
}

{
  const timers = fakeTimers();
  const sent = [];
  const p = new PendingActions({ commit: async (it) => sent.push(it.row.thread_id), timers });
  p.add([{ row: row('1'), verb: 'archive' }], 'first');
  p.add([{ row: row('2'), verb: 'archive' }, { row: row('3'), verb: 'archive' }], 'batch');
  const undone = p.undo();
  t('undo takes back the newest keystroke whole', undone.label === 'batch' && undone.items.length === 2);
  timers.fireAll();
  await settle();
  t('the older one still commits when its window ends', sent.join() === '1');
  t('nothing is left to undo once it has gone', p.undo() === null);
}

{
  const timers = fakeTimers();
  let inFlight = 0;
  let peak = 0;
  const release = [];
  const p = new PendingActions({
    concurrency: 3,
    timers,
    commit: () => {
      inFlight++;
      peak = Math.max(peak, inFlight);
      return new Promise((r) => release.push(() => (inFlight--, r())));
    },
  });
  p.add(Array.from({ length: 10 }, (_, i) => ({ row: row(String(i)), verb: 'archive' })), 'sweep');
  timers.fireAll();
  await settle();
  t('a sweep commits at most three at a time', peak === 3 && p.committingCount === 10);
  t('a row stays hidden while its commit is in flight', ['0', '1', '2'].every((id) => p.hiddenKeys().has(keyOf(row(id)))));
  while (release.length) {
    release.shift()();
    await settle();
    await settle();
  }
  t('and every item eventually commits', p.committingCount === 0 && peak === 3);
}

{
  const timers = fakeTimers();
  const sent = [];
  const p = new PendingActions({ commit: async (it) => sent.push(it.row.thread_id), timers });
  p.add([{ row: row('1'), verb: 'archive' }], 'a');
  p.add([{ row: row('2'), verb: 'archive' }], 'b');
  p.flush();
  await settle();
  t('flush commits every held action without waiting', sent.sort().join() === '1,2' && timers.pending() === 0);
}

{
  // pagehide: everything starts at once, but only for that drain — a page
  // restored from the back/forward cache keeps its bound.
  const timers = fakeTimers();
  let inFlight = 0;
  let peak = 0;
  const release = [];
  const p = new PendingActions({
    concurrency: 3,
    timers,
    commit: () => {
      inFlight++;
      peak = Math.max(peak, inFlight);
      return new Promise((r) => release.push(() => (inFlight--, r())));
    },
  });
  p.add(Array.from({ length: 6 }, (_, i) => ({ row: row(`a${i}`), verb: 'archive' })), 'before');
  p.flush({ now: true });
  await settle();
  t('flush on pagehide starts everything at once', peak === 6);
  while (release.length) {
    release.shift()();
    await settle();
    await settle();
  }
  peak = 0;
  p.add(Array.from({ length: 6 }, (_, i) => ({ row: row(`b${i}`), verb: 'archive' })), 'after');
  timers.fireAll();
  await settle();
  t('and the bound is back for the next batch', peak === 3);
  while (release.length) {
    release.shift()();
    await settle();
    await settle();
  }
}

{
  const timers = fakeTimers();
  const failures = [];
  const p = new PendingActions({
    timers,
    commit: async () => {
      throw new Error('provider said no');
    },
    onFail: (item, err) => failures.push(`${item.row.thread_id}: ${err.message}`),
  });
  p.add([{ row: row('9'), verb: 'archive' }], 'x');
  timers.fireAll();
  await settle();
  await settle();
  t('a refused commit is reported with its reason', failures.join() === '9: provider said no');
  t('and its row is no longer hidden as if it went', !p.hiddenKeys().has(keyOf(row('9'))));
}

console.log(`\n${pass} passed, ${fail} failed`);
process.exit(fail === 0 ? 0 : 1);
