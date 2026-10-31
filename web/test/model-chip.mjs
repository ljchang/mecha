// Behaviour checks for the model chip's logic (src/lib/model-chip.js).
//
// **What is worth pinning.** The readings where being wrong is silent: a
// router whose list cannot be read shown as "no model loaded" (the default
// would then stand, and evict the owner's pick), a model offered that runs
// would not follow, a switch's wait shown as a load, and an outcome from some
// earlier switch reported as this tap's.
import { reader, canHurry, routerOf, unavailable, rows, phase, busy, pollEvery, chipLabel, waitingLine, outcomeNote, gib, available, planLine, installing, installLine, pollWhileInstalling, catalogTrouble } from '../src/lib/model-chip.js';

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
{
  const withFallbacks = router([m('cloudy-x', 'unloaded', { incognito_refusal: 'the provider `cloudy` has fallbacks (anthropic)' }), m('local-y', 'unloaded')]);
  const plain = rows(withFallbacks);
  const incog = rows(withFallbacks, true);
  t('an ordinary chat offers a model with fallbacks', !plain[0].disabled);
  t('an incognito chat refuses it, saying why', incog[0].disabled && incog[0].why.includes('fallbacks'));
  t('and still offers a local one', !incog[1].disabled);
}
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
const stuck = phase(router([m('qwen-a', 'loaded')], {
  pending_switch: { to: '(unreadable switch file)', from: null, started_at: '2026-09-27T20:00:00Z', waiting_on: ['web chat'], readable: false },
}));
t('an unreadable switch file is a stuck switch', stuck.kind === 'switching' && stuck.stuck);
t('a stuck switch is not named as a target', chipLabel(stuck, 'x') === 'switch stuck' && stuck.to === null);
t('a stuck switch points at cancel', waitingLine(stuck).includes('cancel withdraws it'));
t('a readable switch is not stuck', pw.stuck === false);
t('an older list without the flag is not stuck', phase(router([m('a', 'loaded')], { pending_switch: { to: 'b', waiting_on: [] } })).stuck === false);
const past = phase(router([m('qwen-a', 'loaded'), m('gemma-b', 'unloaded')], {
  pending_switch: { to: 'gemma-b', from: 'qwen-a', started_at: '2026-09-27T20:00:00Z', waiting_on: [], readable: true, past_the_wait: true },
}));
t('a waiting switch can be hurried', canHurry(pw));
t('a switch past its wait cannot', past.past && !canHurry(past));
t('past its wait it says it is switching, not waiting', waitingLine(past) === 'switching to gemma-b…');
{
  const busyNoRuns = phase(router([m('qwen-a', 'loaded'), m('gemma-b', 'unloaded')], {
    pending_switch: { to: 'gemma-b', from: 'qwen-a', started_at: '2026-09-28T03:21:40Z', waiting_on: [], readable: true, past_the_wait: false },
  }));
  t('no runs but not past its wait is a busy model, and says so', waitingLine(busyNoRuns).startsWith('waiting for the loaded model to finish a request'));
  t('and it can still be hurried', canHurry(busyNoRuns));
}
t('a stuck switch cannot be hurried', !canHurry(stuck));
t('an older list without the flag is still waiting', phase(router([m('a', 'loaded')], { pending_switch: { to: 'b', waiting_on: ['x'] } })).past === false);
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

// ---- the tap's baseline is a read that starts after the tap ----
{
  // A fake server whose record changes between reads: each read resolves
  // when told to, and reports the record as it stood when the read *began*.
  let record = 'before';
  const started = [];
  const pending = [];
  const r = reader(() => {
    const seen = record;
    started.push(seen);
    return new Promise((resolve) => pending.push(() => resolve(seen)));
  });
  const poll = r.refresh(); // a poll on the wire…
  const shared = r.refresh();
  t('a second poll shares the read on the wire', started.length === 1 && poll === shared);
  record = 'after'; // …when an earlier switch ends
  let baseline = null;
  const tap = r.fresh().then((v) => (baseline = v));
  pending.shift()(); // the old read lands
  await poll;
  await Promise.resolve();
  await Promise.resolve();
  t('fresh does not settle on the read that began before it', baseline === null && started.length === 2);
  pending.shift()();
  await tap;
  t('fresh answers with a read that began after it', baseline === 'after');
}

// ---- the catalog ----
{
  const offer = (id, extra = {}) => ({
    id,
    provider: id,
    label: `${id} label`,
    note: `${id} note`,
    installed: false,
    unfit: null,
    download_bytes: 2 * 1073741824,
    memory_mb: 20 * 1024,
    ...extra,
  });
  const served = router([m('qwen-a', 'loaded'), m('gemma-b', 'unloaded')]);
  const cat = { offers: [offer('gemma-b'), offer('big-c', { unfit: 'needs the 64 GB memory tier or larger' }), offer('new-d')], refusal: null, installing: null };
  const data = { routers: [served], catalog: cat };
  const av = available(data, routerOf(data));
  t('a model the router serves is not offered again', !av.some((r) => r.id === 'gemma-b'));
  t('a model not offered here stays listed, refused with why', av.find((r) => r.id === 'big-c')?.disabled && av.find((r) => r.id === 'big-c').why.includes('64 GB'));
  t('one that fits is offered', av.find((r) => r.id === 'new-d') && !av.find((r) => r.id === 'new-d').disabled);
  t('a router mecha cannot install onto refuses every row', available({ routers: [served], catalog: { ...cat, refusal: 'mecha did not install the chat router here' } }, served).every((r) => r.disabled));
  t('no catalog in the answer offers nothing', available({ routers: [served] }, served).length === 0);
  t('a router not read yet offers nothing — what is served is unknown', available({ routers: [{ base_url: 'x', reachable: false }], catalog: cat }, null).length === 0);
  t('no followed router at all (a fresh install) still offers the catalog', available({ routers: [], catalog: cat }, null).length === 3);
  const plan = planLine(av.find((r) => r.id === 'new-d'));
  t('the plan says the download, the memory and the table it writes', plan.includes('2.0 GiB to download') && plan.includes('~20.0 GiB in memory') && plan.includes('[providers.new-d]'));
  t('gib is one decimal', gib(1610612736) === '1.5 GiB');
  t('no install is none', installing(data) === null && installLine(null) === null && pollWhileInstalling(null, true) === null);
  const p = { id: 'new-d', phase: 'fetching', done_bytes: 1073741824, total_bytes: 2 * 1073741824, pid: 1, started_at: 'x' };
  t('an install under way is read from the catalog', installing({ catalog: { ...cat, installing: p } })?.id === 'new-d');
  t('a download says how far it is', installLine(p) === 'downloading new-d — 1.0 GiB of 2.0 GiB');
  t('a restart is said as such', installLine({ ...p, phase: 'restarting' }).includes('restarting the router'));
  t('an unreadable catalog is said, not hidden', catalogTrouble({ catalog: { offers: [], refusal: 'mecha\'s catalog could not be read: x', installing: null } })?.includes('could not be read'));
  t('a router-wide refusal is said beside the offers too', catalogTrouble({ catalog: { ...cat, refusal: 'mecha did not install the chat router here' } })?.includes('did not install'));
  t('a catalog that can be installed from has no trouble line', catalogTrouble({ catalog: cat }) === null);
  t('an install reads fast with the menu open, slowly with it closed', pollWhileInstalling(p, true) === 2000 && pollWhileInstalling(p, false) === 15000);
}

console.log(`\n${pass} passed, ${fail} failed`);
if (fail) process.exit(1);
