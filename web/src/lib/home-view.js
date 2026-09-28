// The home page's logic, kept out of Home.svelte so node can test it
// (web/test/home-view.mjs).
//
// Home is counts and doors: one card per place, a number, a tap into the
// tab that has the rest. Rows belong to their tabs — a home that listed the
// head of every tab was tried on 2026-09-28 and read as a second copy of
// each, busier than any of them.
//
// **A store that could not be read is not an empty one.** Every count here
// is `null` when its read failed and `undefined` before it answers, so a
// card can show a dash, and never a zero it did not measure.

import { laneOf, sortRows } from './mail-desk.js';

/** The threads in mail's "Needs you" lane, in the order the desk shows them. */
export function needsYou(rows) {
  if (!Array.isArray(rows)) return rows;
  return sortRows(rows.filter((r) => laneOf(r) === 'respond'));
}

/** Mail's FYI lane: classified, and the classifier said notify. */
export const fyi = (rows) => (Array.isArray(rows) ? rows.filter((r) => laneOf(r) === 'notify').length : 0);

/**
 * The board's default view, `actionable`: do next, or newly captured. One
 * predicate for two readers — `Tasks.svelte` filters its default view with
 * it and home's Tasks card counts with it — because the card lands on that
 * view, so it must count what that view shows. Counting every open task
 * (scheduled and waiting too) put a 9 on the card and three rows under the
 * tap, and a second copy of the predicate is how the two drift again.
 */
export const isActionable = (t) => t.status === 'next' || t.status === 'inbox';

/** The tasks the Tasks card's tap lands on. */
export function actionable(items) {
  if (!Array.isArray(items)) return items;
  return items.filter(isActionable);
}

/**
 * A backlog queue as a card's count: `{ n, why }`. `undefined` while the
 * backlog is still being asked. When the count is unknown, `why` carries the
 * reason — `collect_queues()` reports an unreadable store as a null depth
 * with the error in `detail`, and a card that keeps the dash but drops the
 * detail has thrown away the only account of what went wrong.
 */
export function queueCount(summary, name) {
  if (summary === undefined) return undefined;
  if (summary === null) return { n: null, why: null };
  const row = (summary.queues ?? []).find((q) => q.queue === name);
  if (!row) return { n: null, why: null };
  const n = row.depth ?? null;
  return { n, why: n === null ? (row.detail ?? null) : null };
}

/** `workflow today`'s sections, in the order a person works through them. */
export const SECTIONS = [
  ['urgent', 'Needs attention'],
  ['decisions', 'Decisions for you'],
  ['ready', 'Ready to finish'],
  ['waiting', 'In progress and waiting'],
];

/**
 * The workflows themselves, grouped by section. Loose drafts and questions
 * (`workflow today` reports those too, without `workflow: true`) are left
 * to the outbox and the board's waiting view, which already show them.
 */
export function workflowGroups(today) {
  const mine = (today?.items ?? []).filter((i) => i.workflow);
  return SECTIONS.map(([key, label]) => ({ key, label, items: mine.filter((i) => i.section === key) })).filter(
    (g) => g.items.length,
  );
}

/** "2 broken, 1 needs a look" — or null when doctor could not be read. */
export function healthLine(doctor) {
  if (!Array.isArray(doctor)) return null;
  if (!doctor.length) return 'Nothing silently wrong';
  const broken = doctor.filter((f) => f.severity === 'broken').length;
  const rest = doctor.length - broken;
  const bits = [];
  if (broken) bits.push(`${broken} broken`);
  if (rest) bits.push(`${rest} ${rest === 1 ? 'needs' : 'need'} a look`);
  return bits.join(', ');
}
