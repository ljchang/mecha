// Behaviour checks for the model chip's logic (src/lib/model-chip.js).
//
// **What is worth pinning.** The readings where being wrong is silent: a
// router whose list cannot be read shown as "no model loaded" (the default
// would then stand, and evict the owner's pick), a model offered that runs
// would not follow, a switch's wait shown as a load, and an outcome from some
// earlier switch reported as this tap's.
import { routerOf, unavailable, rows, phase, busy, pollEvery, chipLabel, waitingLine, outcomeNote } from '../src/lib/model-chip.js';

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

const m = (id, status, extra = {}) => ({
  id,
  status,
  providers: [id.split('-')[0]],
  sampling_mismatches: [],
  would_not_follow: null,
  ...extra,
});
const router = (models, extra = {}) => ({
  base_url: 'http://127.0.0.1:8080',
  reachable: true,
  readable: true,
  resident: models.find((x) => ['loaded', 'loading', 'sleeping'].includes(x.status))?.id ?? null,
  models,
  unserved: [],
  pending_switch: null,
  ...extra,
});

// ---- which router, and when there is none ----
t('no routers is said, not an empty menu', unavailable({ routers: [] })?.includes('no provider follows'));
t('a router down is said as down', unavailable({ routers: [{ base_url: 'http://x', reachable: false }] })?.includes('not answering'));
t('a reachable router is available', unavailable({ routers: [router([m('a', 'loaded')])] }) === null);
t('the first reachable router is the one', routerOf({ routers: [{ base_url: 'x', reachable: false }, router([m('a', 'loaded')])] })?.reachable === true);

// ---- rows ----
const r1 = router([m('qwen-a', 'loaded'), m('gemma-b', 'unloaded'), m('odd-c', 'unloaded', { would_not_follow: 'no entry names it' })]);
const rs = rows(r1);
t('the loaded model is current', rs.find((x) => x.id === 'qwen-a').current);
t('an unloaded model is not', !rs.find((x) => x.id === 'gemma-b').current);
t('a single provider entry is the name handed to `use`', rs.find((x) => x.id === 'gemma-b').name === 'gemma');
t('a model several entries name is handed by id', rows(router([m('x', 'unloaded', { providers: ['p', 'q'] })]))[0].name === 'x');
t('a model runs would not follow is refused, with why', rs.find((x) => x.id === 'odd-c').disabled && rs.find((x) => x.id === 'odd-c').why === 'no entry names it');
t('an R4 mismatch is refused, with why', rows(router([m('x', 'unloaded', { sampling_mismatches: ['temp 0.7 vs 0.6'] })]))[0].why === 'temp 0.7 vs 0.6');
t('an unreadable list marks nothing current', !rows(router([m('a', 'loaded')], { readable: false }))[0].current);

// ---- phase ----
t('at rest is idle on the resident', phase(r1).kind === 'idle' && phase(r1).resident === 'qwen-a');
t('an unreadable list is unknown, never "nothing loaded"', phase(router([m('a', 'weird')], { readable: false, resident: null })).kind === 'unknown');
t('no router is unknown', phase(null).kind === 'unknown');
const waiting = router([m('qwen-a', 'loaded'), m('gemma-b', 'unloaded')], {
  pending_switch: { to: 'gemma-b', from: 'qwen-a', started_at: '2026-09-27T20:00:00Z', waiting_on: ['web chat', 'trigger morning'] },
});
const pw = phase(waiting);
t('a pending switch is switching', pw.kind === 'switching' && pw.to === 'gemma-b');
t('it names what it waits for', waitingLine(pw) === 'waiting for: web chat, trigger morning');
t('while it waits it is not loading', !pw.loading);
const loadingNow = router([m('qwen-a', 'unloaded'), m('gemma-b', 'loading')], {
  pending_switch: { to: 'gemma-b', from: 'qwen-a', started_at: '2026-09-27T20:00:00Z', waiting_on: [] },
});
t('the target loading under a pending switch is the load', phase(loadingNow).loading && waitingLine(phase(loadingNow)) === 'loading gemma-b…');
t('a load nobody here asked for is loading', phase(router([m('a', 'loading')])).kind === 'loading');
t('switching and loading poll', busy(pw) && busy(phase(router([m('a', 'loading')]))));
t('rest does not poll', !busy(phase(r1)) && !busy(phase(null)));

// ---- label ----
t('the label at rest is the resident', chipLabel(phase(r1), 'old') === 'qwen-a');
t('switching shows where it is going', chipLabel(pw, 'old') === '→ gemma-b');
t('before the router is read, the chat\'s own model', chipLabel(phase(null), 'bound-model') === 'bound-model');
t('nothing at all is an ellipsis', chipLabel(phase(null), '') === '…');

// ---- polling ----
t('an open menu reads fast', pollEvery(phase(r1), true) === 2000);
t('a closed chip at rest does not read', pollEvery(phase(r1), false) === null);
t('a closed chip over a waiting switch reads slowly', pollEvery(pw, false) === 15000);
t('a closed chip over a load reads fast', pollEvery(phase(loadingNow), false) === 2000);

// ---- outcome ----
// Server timestamps only: the baseline is the record as it stood before the
// tap, so no browser clock is ever compared with the server's.
const failed = { last_switch: { to: 'gemma-b', ok: false, message: 'gemma-b did not load; qwen-a is loaded again', at: '2026-09-27T20:01:00Z' } };
t('a failure after the tap is reported', outcomeNote(failed, { before: null })?.tone === 'bad');
t('a failure after an earlier record is reported', outcomeNote(failed, { before: '2026-09-27T19:00:00Z' })?.tone === 'bad');
t('the record from before the tap is someone else\'s', outcomeNote(failed, { before: '2026-09-27T20:01:00Z' }) === null);
t('nothing tapped reports nothing', outcomeNote(failed, null) === null);
t('a clean success needs no line', outcomeNote({ last_switch: { to: 'x', ok: true, message: '', at: '2026-09-27T20:01:00Z' } }, { before: null }) === null);
t('a success with a warning says it', outcomeNote({ last_switch: { to: 'x', ok: true, message: 'runs will not follow', at: '2026-09-27T20:01:00Z' } }, { before: null })?.tone === 'warn');

console.log(`\n${pass} passed, ${fail} failed`);
if (fail) process.exit(1);
