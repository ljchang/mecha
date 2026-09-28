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

const CLOSED = new Set(['done', 'dropped']);
const STATUS_RANK = { next: 0, inbox: 1, waiting: 2 };

/**
 * Open tasks, most pressing first: overdue, then by due date, then undated
 * by status. The board is read with `--closed`, so finished tasks are
 * filtered here.
 */
export function openTasks(items) {
  if (!Array.isArray(items)) return items;
  return items
    .filter((t) => !CLOSED.has(t.status) && !t.completed_at)
    .sort((a, b) => {
      if (!!a.overdue !== !!b.overdue) return a.overdue ? -1 : 1;
      if (a.due_at && b.due_at && a.due_at !== b.due_at) return a.due_at.localeCompare(b.due_at);
      if (!!a.due_at !== !!b.due_at) return a.due_at ? -1 : 1;
      return (STATUS_RANK[a.status] ?? 3) - (STATUS_RANK[b.status] ?? 3);
    });
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
