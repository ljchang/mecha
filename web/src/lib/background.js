// Tasks → Background: every unattended run in one place
// (docs/ASSISTANT-FOLLOW-THROUGH-DESIGN.md §Tasks → Background, ruling B1).
//
// A view, not a verb: it lists and links. Answering stays in Tasks → Waiting,
// stopping a delegated run on its board card, dismissing on the workflow card.
//
// Three of the four kinds come from `GET /api/background` (scheduled
// triggers, woken follow-ups, and the machine's `mecha-*` jobs); the fourth,
// a delegated task's run, is read off the board the page already holds
// (`/api/tasks`' `run` summary), so the view adds no second reader of the
// graph store.
//
// Unknown is said, never zero: a section the server could not read is a line
// saying so, a state this build cannot name is `unknown` and never drawn as
// done, and a run whose taint was not recorded says so rather than looking
// clean.

import { chatHref } from './question-resume.js';

/// How far back the view looks — the server's window, applied to the board's
/// rows too.
export const WINDOW_MS = 7 * 24 * 60 * 60 * 1000;

/// The assistant-run states, in words. A closed set: anything else is
/// `unknown`.
export const RUN_STATES = {
  running: { word: 'running now', cls: 'agent' },
  waiting: { word: 'waiting on you', cls: 'needs' },
  done: { word: 'done', cls: 'ready' },
  failed: { word: 'failed', cls: 'broke' },
  unknown: { word: 'state unknown', cls: 'broke' },
};

export const JOB_STATES = {
  running: { word: 'running', cls: 'agent' },
  ok: { word: 'ok', cls: 'ready' },
  failed: { word: 'failed', cls: 'broke' },
  unknown: { word: 'unknown', cls: 'broke' },
};

export const KIND_WORDS = { trigger: 'scheduled', follow_up: 'follow-up', task: 'task' };

/// A state as the view draws it: a known one, or `unknown` — never `done`
/// for something the server did not say was done.
export const runState = (s) => (Object.hasOwn(RUN_STATES, s) ? s : 'unknown');
export const jobState = (s) => (Object.hasOwn(JOB_STATES, s) ? s : 'unknown');

/// One section of the endpoint's body: `{rows}`, `{error}`, or
/// `{unavailable}`. A missing or malformed section is an error, because "the
/// server said nothing" is not "there is nothing".
export function sectionOf(body, key) {
  const s = body?.[key];
  if (Array.isArray(s?.rows)) {
    return { rows: s.rows, unreadable: s.unreadable ?? 0, truncated: !!s.truncated };
  }
  if (typeof s?.unavailable === 'string') return { unavailable: s.unavailable };
  if (typeof s?.error === 'string') return { error: s.error };
  return { error: 'the server gave no answer for this section' };
}

/// The taint badge a row carries: `outside` when third-party content was in
/// the run's conversation, `unknown` when nothing was recorded, `null` when
/// it was recorded clean.
export function taintBadge(row) {
  if (row?.taint == null) return 'unknown';
  return row.taint.untrusted ? 'outside' : null;
}

// The board's own vocabulary, said once: `Tasks.svelte` imports these for
// its cards rather than keeping a copy, so the card and this view cannot
// come to disagree about what a task's run is (pinned in test/background.mjs).

/// Who `waiting_on` names when the assistant has a task.
export const AGENT = 'mecha';
/// The statuses a task is disposed of in.
export const CLOSED_STATUSES = ['done', 'dropped'];
/// The stop causes a person, a question, or the process produced — never a
/// failure: `stopped` is a person, `parked` a question to the owner,
/// `shutdown` the process, and `interrupted` the older record that did not
/// say which.
export const QUIET_STOPS = ['interrupted', 'parked', 'stopped', 'shutdown'];

/// A delegated task's run, as a row — or null when the task never had one.
///
/// Read off the board item and the open questions the page already holds:
/// `waiting_on` naming the agent is a run in flight, an open question on the
/// task is waiting on you, a `stalled` task (the board names the agent and no
/// run marker stands) is unknown, and the transcript's outcome record says
/// how a finished one went. No outcome recorded is unknown, never done.
export function delegatedRow(t, questions = []) {
  if (!t?.session) return null;
  const run = t.run ?? null;
  const q = questions.find((x) => x.task === t.id);
  let state;
  let detail = null;
  if (q) state = 'waiting';
  else if (t.stalled) {
    state = 'unknown';
    detail = 'the board says mecha has it, and no run is alive';
  } else if (t.status === 'waiting' && t.waiting_on === AGENT) state = 'running';
  else if (!run || !run.recorded) {
    state = 'unknown';
    detail = run?.transcript === false ? 'its conversation is gone' : 'no outcome recorded';
  } else if (QUIET_STOPS.includes(run.stop_cause)) {
    state = 'done';
    detail = `stopped: ${run.stop_cause}`;
  } else if (run.cut_short || run.ended_on_failed_call) {
    state = 'failed';
    detail = run.ended_on_failed_call ? 'stopped on a failed call' : `cut short: ${run.stop_cause ?? 'no cause recorded'}`;
  } else state = 'done';
  return {
    kind: 'task',
    name: t.name ?? t.id,
    state,
    started_at: null,
    finished_at: state === 'running' || state === 'waiting' ? null : (run?.ended_at ?? null),
    session: t.session,
    question: q?.id ?? null,
    taint: run?.taint ?? null,
    summary: null,
    detail,
    task: t.id,
    closed: CLOSED_STATUSES.includes(t.status),
  };
}

/// Which delegated rows belong in a seven-day view: anything running or
/// waiting; a finished one that ended inside the window; and one with no
/// end time — unknown, or a done or failed run from a record older than its
/// `ended_at` — only while its task is still open (a closed task's old run
/// is history the board card already keeps quiet). An undated run of an
/// open task dropped here would leave the view saying "no assistant runs"
/// about a task the board still shows; it is drawn "no time recorded" and
/// sorts last (`sortKey`).
function inWindow(row, now) {
  if (row.state === 'running' || row.state === 'waiting') return true;
  const at = Date.parse(row.finished_at ?? '');
  if (Number.isFinite(at)) return at >= now - WINDOW_MS;
  return !row.closed;
}

export function delegatedRows(items, questions = [], now = Date.now()) {
  return (items ?? [])
    .map((t) => delegatedRow(t, questions ?? []))
    .filter((r) => r && inWindow(r, now));
}

/// Newest first by when the run ended (or started, while it has not). A run
/// in flight or waiting with no time at all sorts to the top — it is about
/// now — and any other row with no time sorts last rather than pretending to
/// a date.
export function sortKey(row) {
  const at = Date.parse(row.finished_at ?? row.started_at ?? '');
  if (Number.isFinite(at)) return row.state === 'running' ? Infinity : at;
  return row.state === 'running' || row.state === 'waiting' ? Infinity : -Infinity;
}

/// Every assistant run the view shows: the endpoint's trigger and follow-up
/// rows (where their sections answered) and the board's delegated ones.
export function assistantRows(body, items, questions = [], now = Date.now()) {
  const fromServer = ['triggers', 'follow_ups'].flatMap((k) => sectionOf(body, k).rows ?? []);
  const rows = [
    ...fromServer.map((r) => ({ ...r, state: runState(r.state) })),
    ...delegatedRows(items, questions, now),
  ];
  return rows.sort((a, b) => sortKey(b) - sortKey(a));
}

/// A time as a short relative phrase: `5m ago`, `in 2h`, `3d ago`, or
/// `never` for none. Relative, so the phone and the server need not agree on
/// a timezone to be read right.
export function when(iso, now = Date.now()) {
  if (iso == null) return 'never';
  const at = Date.parse(iso);
  if (!Number.isFinite(at)) return 'unknown';
  const d = at - now;
  const past = d < 0;
  const m = Math.round(Math.abs(d) / 60_000);
  if (m < 1) return past ? 'just now' : 'in under a minute';
  let span;
  if (m < 60) span = `${m}m`;
  else if (m < 48 * 60) span = `${Math.round(m / 60)}h`;
  else span = `${Math.round(m / (24 * 60))}d`;
  return past ? `${span} ago` : `in ${span}`;
}

/// A job's short name: the timer without its `mecha-` prefix and `.timer`.
export const jobName = (j) => String(j?.timer ?? '').replace(/^mecha-/, '').replace(/\.timer$/, '');

/// One job as one line: name · state · last · next.
export function jobLine(j, now = Date.now()) {
  const st = JOB_STATES[jobState(j?.state)].word;
  const bits = [jobName(j), st, `last ${when(j?.last_run, now)}`, `next ${when(j?.next_run, now)}`];
  if (j?.detail) bits.push(j.detail);
  return bits.join(' · ');
}

/// Where *Open chat* goes for a row: the chat route by session id (never a
/// chat key — question-resume.js says why), or null when the run names none.
export function openChatHref(row) {
  const s = typeof row?.session === 'string' ? row.session.trim() : '';
  return s ? chatHref(s) : null;
}
