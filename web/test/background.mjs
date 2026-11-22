import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  AGENT,
  CLOSED_STATUSES,
  QUIET_STOPS,
  assistantRows,
  delegatedRow,
  delegatedRows,
  jobLine,
  jobState,
  openChatHref,
  runState,
  sectionOf,
  sortKey,
  taintBadge,
  when,
} from '../src/lib/background.js';

const now = Date.parse('2026-11-20T12:00:00Z');
const hoursAgo = (h) => new Date(now - h * 3_600_000).toISOString();

// States: a known one, or unknown — never done for what the server did not say.
for (const s of ['running', 'waiting', 'done', 'failed', 'unknown']) assert.equal(runState(s), s);
assert.equal(runState('re-queued-by-a-later-build'), 'unknown');
assert.equal(runState(undefined), 'unknown');
assert.equal(runState('toString'), 'unknown', 'an inherited name is not a state');
assert.equal(jobState('ok'), 'ok');
assert.equal(jobState('activating'), 'unknown');

// Sections: rows, error, unavailable — and a missing one is an error, never empty.
assert.deepEqual(sectionOf({ triggers: { rows: [] } }, 'triggers'), { rows: [], unreadable: 0, truncated: false });
assert.deepEqual(sectionOf({ triggers: { error: 'disk' } }, 'triggers'), { error: 'disk' });
assert.deepEqual(sectionOf({ jobs: { unavailable: 'no systemctl' } }, 'jobs'), { unavailable: 'no systemctl' });
assert.ok(sectionOf({}, 'jobs').error, 'no answer is not no jobs');
assert.ok(sectionOf(null, 'triggers').error);
assert.equal(sectionOf({ triggers: { rows: [], unreadable: 2, truncated: true } }, 'triggers').unreadable, 2);

// Taint: outside, unknown when unrecorded, nothing when recorded clean.
assert.equal(taintBadge({ taint: { private: true, untrusted: true } }), 'outside');
assert.equal(taintBadge({ taint: { private: true, untrusted: false } }), null);
assert.equal(taintBadge({ taint: null }), 'unknown', 'not recorded is not clean');
assert.equal(taintBadge({}), 'unknown');

// Delegated task runs, off the board item.
const task = (over) => ({ id: 'task-1', name: 'check the nightly build', status: 'next', session: '20261120T090000-aaaa', ...over });
const ran = (over) => ({ recorded: true, transcript: true, stop_cause: 'completed', cut_short: false, ended_on_failed_call: false, ended_at: hoursAgo(2), taint: { private: false, untrusted: false }, ...over });
assert.equal(delegatedRow(task({ session: undefined })), null, 'never delegated is not a run');
assert.equal(delegatedRow(task({ status: 'waiting', waiting_on: 'mecha', run: ran() })).state, 'running');
const waiting = delegatedRow(task({ run: ran() }), [{ id: 'q-1', task: 'task-1' }]);
assert.equal(waiting.state, 'waiting');
assert.equal(waiting.question, 'q-1');
assert.equal(delegatedRow(task({ status: 'waiting', waiting_on: 'mecha', stalled: true })).state, 'unknown', 'a dead run is not running');
assert.equal(delegatedRow(task({})).state, 'unknown', 'no summary is unknown');
assert.equal(delegatedRow(task({ run: { recorded: false, transcript: true } })).state, 'unknown', 'no outcome is unknown, never done');
assert.equal(delegatedRow(task({ run: ran() })).state, 'done');
assert.equal(delegatedRow(task({ run: ran({ stop_cause: 'stopped', cut_short: false }) })).state, 'done', 'a person stopping it is not a failure');
assert.equal(delegatedRow(task({ run: ran({ cut_short: true, stop_cause: 'timeout' }) })).state, 'failed');
assert.equal(delegatedRow(task({ run: ran({ ended_on_failed_call: true }) })).state, 'failed');
assert.equal(delegatedRow(task({ run: ran({ taint: undefined }) })).taint, null, 'an older server sends no taint: unknown');

// The window: a finished run older than seven days is out; running and
// waiting are always in; a run with no time — unknown, or a done or failed
// one from a record older than `ended_at` — only while the task is open.
// Dropping an open task's undated run left the view saying "no assistant
// runs" about a task the board still shows.
const rows = delegatedRows(
  [
    task({ id: 'old', run: ran({ ended_at: hoursAgo(24 * 8) }) }),
    task({ id: 'new', run: ran() }),
    task({ id: 'live', status: 'waiting', waiting_on: 'mecha' }),
    task({ id: 'lost-open' }),
    task({ id: 'lost-closed', status: 'done' }),
    task({ id: 'untimed', run: ran({ ended_at: null }) }),
    task({ id: 'untimed-failed', run: ran({ ended_at: undefined, cut_short: true, stop_cause: 'timeout' }) }),
    task({ id: 'untimed-closed', status: 'dropped', run: ran({ ended_at: null }) }),
  ],
  [],
  now,
);
assert.deepEqual(rows.map((r) => r.task).sort(), ['live', 'lost-open', 'new', 'untimed', 'untimed-failed']);
const untimed = rows.find((r) => r.task === 'untimed');
assert.equal(untimed.state, 'done');
assert.equal(untimed.finished_at, null, 'no time is made up for it');
assert.equal(rows.find((r) => r.task === 'untimed-failed').state, 'failed');
assert.equal(sortKey(untimed), -Infinity, 'and it sorts last');
assert.deepEqual(
  assistantRows({}, [task({ id: 'untimed', run: ran({ ended_at: null }) })], [], now).map((r) => r.task),
  ['untimed'],
  'the only run the view knows of is still shown',
);

// One copy of the board's words: Tasks.svelte imports them from here rather
// than keeping its own (the home-view precedent for `isActionable`).
assert.equal(AGENT, 'mecha');
assert.deepEqual(CLOSED_STATUSES, ['done', 'dropped']);
assert.deepEqual(QUIET_STOPS, ['interrupted', 'parked', 'stopped', 'shutdown']);
const board = fs.readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), '../src/lib/Tasks.svelte'), 'utf8');
assert.match(
  board,
  /import \{ AGENT, CLOSED_STATUSES, QUIET_STOPS \} from '\.\/background\.js'/,
  'Tasks imports the shared words',
);
for (const name of ['AGENT', 'CLOSED_STATUSES', 'QUIET_STOPS']) {
  assert.doesNotMatch(board, new RegExp(`const ${name}\\s*=`), `Tasks keeps its own ${name}`);
}
assert.doesNotMatch(board, /\['interrupted', 'parked'/, 'Tasks spells the quiet stops out again');

// Merged, newest first; a run in flight on top, untimed history last.
const body = {
  triggers: { rows: [
    { kind: 'trigger', name: 'morning-brief', state: 'done', started_at: hoursAgo(5), finished_at: hoursAgo(5), session: 's-t1', taint: null },
    { kind: 'trigger', name: 'nightly', state: 'running', started_at: hoursAgo(0.2), finished_at: null, session: null, taint: null },
    { kind: 'trigger', name: 'later-build', state: 'brand-new', started_at: hoursAgo(1), finished_at: hoursAgo(1), session: 's-t2', taint: null },
  ] },
  follow_ups: { error: 'workflow store unreadable' },
  jobs: { unavailable: 'no systemctl' },
};
const merged = assistantRows(body, [task({ run: ran({ ended_at: hoursAgo(3) }) })], [], now);
assert.deepEqual(merged.map((r) => r.name), ['nightly', 'later-build', 'check the nightly build', 'morning-brief']);
assert.equal(merged[1].state, 'unknown', 'a state this build cannot name is drawn unknown');
assert.equal(sortKey({ state: 'done' }), -Infinity);
assert.equal(sortKey({ state: 'waiting' }), Infinity);

// Times, relative.
assert.equal(when(null, now), 'never');
assert.equal(when('garbage', now), 'unknown');
assert.equal(when(hoursAgo(0.5), now), '30m ago');
assert.equal(when(hoursAgo(5), now), '5h ago');
assert.equal(when(hoursAgo(24 * 3), now), '3d ago');
assert.equal(when(new Date(now + 2 * 3_600_000).toISOString(), now), 'in 2h');

// Jobs, one line each.
assert.equal(
  jobLine({ timer: 'mecha-mail-rules.timer', state: 'ok', last_run: hoursAgo(1), next_run: new Date(now + 600_000).toISOString() }, now),
  'mail-rules · ok · last 1h ago · next in 10m',
);
assert.equal(
  jobLine({ timer: 'mecha-ruminate.timer', state: 'failed', last_run: hoursAgo(9), next_run: null, detail: 'last run: exit-code' }, now),
  'ruminate · failed · last 9h ago · next never · last run: exit-code',
);
assert.match(jobLine({ timer: 'mecha-x.timer', state: 'weird' }, now), /· unknown ·/);

// Open chat: by session id, and nothing made up when there is none.
assert.equal(openChatHref({ session: '20261120T090000-aaaa' }), '#chat/20261120T090000-aaaa');
assert.equal(openChatHref({ session: null }), null);
assert.equal(openChatHref({ session: '  ' }), null);
console.log('background: ok');
