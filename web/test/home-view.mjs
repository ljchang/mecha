// The home page's pure half, and the doors it opens.
//
// `npm test` in web/. Home is counts, so the rules worth pinning are about
// what a count means: a failed read stays null (a dash, never a zero), the
// mail count is the desk's "Needs you" lane, and the task count leaves
// finished work out. Then the doors: a `tasks/<view>` target that names no
// view lands on the default one silently, and the Rust guard over home's
// queue map does not check Tasks' views, so this does.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { needsYou, fyi, actionable, ACTIONABLE, workflowGroups, healthLine } from '../src/lib/home-view.js';

// Unread passes through: null stays null, undefined stays undefined.
assert.equal(needsYou(null), null);
assert.equal(needsYou(undefined), undefined);
assert.equal(actionable(null), null);
assert.equal(fyi(null), 0);

// Mail's "Needs you" lane, in the desk's order — the count on home is the
// count the owner lands on.
const rows = [
  { thread_id: 'a', account: 'w', state: 'classified', bucket: 'respond', urgency: 'week', date: '2026-09-01' },
  { thread_id: 'b', account: 'w', state: 'classified', bucket: 'notify', urgency: 'now', date: '2026-09-02' },
  { thread_id: 'c', account: 'w', state: 'classified', bucket: 'respond', urgency: 'now', date: '2026-08-01' },
  { thread_id: 'd', account: 'w', state: 'acted', bucket: 'respond', urgency: 'now', date: '2026-09-03' },
  { thread_id: 'e', account: 'w', state: 'failed', date: '2026-09-04' },
];
assert.deepEqual(needsYou(rows).map((r) => r.thread_id), ['c', 'a', 'e']);
assert.equal(fyi(rows), 1);

// The Tasks card counts what its tap lands on — the board's actionable view
// (next or inbox) — not every open task: scheduled and waiting are other
// views, and finished work (the board is read with --closed) is none.
const tasks = [
  { id: 'w', status: 'waiting' },
  { id: 'n', status: 'next' },
  { id: 'late', status: 'next', due_at: '2026-09-20', overdue: true },
  { id: 'new', status: 'inbox' },
  { id: 'later', status: 'scheduled', due_at: '2026-10-01' },
  { id: 'done', status: 'done', overdue: true },
  { id: 'gone', status: 'dropped' },
  { id: 'finished', status: 'next', completed_at: '2026-09-01' },
];
assert.deepEqual(actionable(tasks).map((t) => t.id), ['n', 'late', 'new']);

// The workflows view shows workflows only — every section, urgent included
// (overdue work must show during a snooze) — and leaves loose drafts and
// questions to the outbox and the waiting view.
const today = {
  items: [
    { id: 'd1', title: 'mail__mail_reply {…', section: 'urgent', state: 'delivery uncertain', outbox: ['d1'] },
    { id: 'w0', title: 'Send the grades', section: 'urgent', state: 'idle', workflow: true, snoozed_until: '2026-09-29T00:00:00Z' },
    { id: 'w2', title: 'Chase the letters', section: 'waiting', state: 'awaiting_owner', workflow: true },
    { id: 'w3', title: 'Ship the syllabus', section: 'ready', state: 'idle', workflow: true },
    { id: 'q1', title: 'Which paper?', section: 'decisions', state: 'answer needed', questions: ['q1'] },
  ],
  closed: [],
};
assert.deepEqual(
  workflowGroups(today).map((g) => [g.key, g.items.map((i) => i.id)]),
  [['urgent', ['w0']], ['ready', ['w3']], ['waiting', ['w2']]],
);
assert.deepEqual(workflowGroups(null), []);

assert.equal(healthLine(null), null);
assert.equal(healthLine(undefined), null);
assert.equal(healthLine([]), 'Nothing silently wrong');
assert.equal(healthLine([{ severity: 'broken' }, { severity: 'broken' }, { severity: 'attention' }]), '2 broken, 1 needs a look');
assert.equal(healthLine([{ severity: 'attention' }, { severity: 'attention' }]), '2 need a look');

// Every `tasks/<view>` home links to is a view Tasks has.
const here = path.dirname(fileURLToPath(import.meta.url));
const read = (f) => fs.readFileSync(path.join(here, '..', 'src', 'lib', f), 'utf8');
const board = read('Tasks.svelte');
const boardActionable = JSON.parse(board.match(/const ACTIONABLE = (\[[^\]]*\])/)[1].replaceAll("'", '"'));
assert.deepEqual(ACTIONABLE, boardActionable, "home's Tasks count and the board's default view must select the same statuses");
assert.match(board, /\['actionable', \(t\) => ACTIONABLE\.includes\(t\.status\)/, 'the default view is still ACTIONABLE');
const views = [...board.matchAll(/^\s*\['(\w+)', \(t\) =>/gm)].map((m) => m[1]);
views.push(board.match(/const WORKFLOWS = '(\w+)'/)[1]);
assert.ok(views.includes('waiting') && views.includes('workflows'), `Tasks views parsed as ${views}`);
const targets = [...read('Home.svelte').matchAll(/'tasks\/(\w+)'/g)].map((m) => m[1]);
assert.ok(targets.length >= 2, 'home links into at least two board views');
for (const t of targets) assert.ok(views.includes(t), `home links to tasks/${t}, which Tasks does not have (${views})`);

// Home stays counts: no row lists of another tab's items.
const home = read('Home.svelte');
for (const f of ['/api/outbox', '/api/questions', '/api/sessions']) {
  assert.ok(!home.includes(f), `Home.svelte reads ${f} — home shows counts, its tab shows the rows`);
}

console.log('home-view: ok');
