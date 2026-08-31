// Behaviour checks for the review queue's cache filtering.
//
// `npm test` in web/. Plain node, no framework and no dependency — the pane
// had no JS rig and adding one for a single pure function would be a larger
// commitment than the function is worth.
//
// **Why this exists at all.** Most of what Queue.svelte does is assertable
// from Rust by reading the source (`serve/review.rs` does exactly that, and
// those tests catch a call reappearing where it must not). `withoutJudged` is
// the one piece that is real logic rather than a shape: it promotes leaders,
// drops groups it cannot name a face for, and keeps `sample` aligned with the
// members that survive. A source-string assertion cannot tell whether any of
// that is correct, and two rounds of review found staleness bugs in exactly
// this area — the cache is the part of the pane where being wrong is silent.
//
// The function is read OUT of the component rather than copied here, so this
// exercises the text that ships. A copy would be a second reader of a rule,
// which is the failure mode the pane's own comments keep warning about.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Queue.svelte'), 'utf8');

const start = src.indexOf('  function withoutJudged(listing) {');
if (start < 0) throw new Error('Queue.svelte no longer defines withoutJudged');
const fnSrc = src.slice(start, src.indexOf('\n  }\n', start) + 4);

const judgedIds = new Set();
const withoutJudged = new Function('judgedIds', `${fnSrc}; return withoutJudged;`)(judgedIds);

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

// Leader #100 with six members. `sample` holds the statements of the first
// three members, in members order — the graph's own construction in
// `assemble_global_groups`, and the only id-to-statement mapping the page has.
const group = () => ({
  leader_id: 100,
  leader_statement: 'S100',
  members: [
    [101, 0.95],
    [102, 0.94],
    [103, 0.93],
    [104, 0.92],
    [105, 0.91],
    [106, 0.9],
  ],
  sample: ['S101', 'S102', 'S103'],
  classes: { 'bee . plays': 4, 'mail . plays': 3 },
});
const listing = () => ({
  key: 'global:0.87',
  rows: [
    group(),
    { ...group(), leader_id: 200, leader_statement: 'S200', members: [[201, 0.9]], sample: ['S201'] },
  ],
});

// Untouched listings must cost nothing — this runs on every cache read.
t(
  'a listing with nothing judged comes back by identity',
  (() => {
    const l = listing();
    return withoutJudged(l) === l;
  })()
);

judgedIds.clear();
judgedIds.add(104);
let r = withoutJudged(listing());
t('a judged member is removed', r.rows[0].members.map((m) => m[0]).join() === '101,102,103,105,106');
t('the kicker count follows it down', r.rows[0].members.length + 1 === 6);
// The chips render under that kicker; two numbers disagreeing on one card is
// the thing being prevented.
t('the shrunken group drops its class chips', r.rows[0].classes === null);
t('a sibling group nobody touched keeps its chips', r.rows[1].classes !== null);

judgedIds.clear();
judgedIds.add(100);
r = withoutJudged(listing());
t('a judged leader is replaced', r.rows[0].leader_id === 101);
t('the new face is a real member statement', r.rows[0].leader_statement === 'S101');
t('the promoted leader is no longer also a member', !r.rows[0].members.some((m) => m[0] === 101));

// Past `sample` there is no statement to promote with, and a group's face must
// never be something this page invented.
judgedIds.clear();
[100, 101, 102, 103].forEach((i) => judgedIds.add(i));
r = withoutJudged(listing());
t('a group with no nameable face is dropped, not faked', r.rows.length === 1 && r.rows[0].leader_id === 200);

// A pair is the commonest group size, and judging its one member leaves a
// leader alone. `reconcileGroup` and this function must agree about that, or
// a card renders "1 near-repeats" over Reject all 1 — and tapping it sends an
// empty cascade, which comes back with no `cascade:` line to read.
judgedIds.clear();
judgedIds.add(201);
r = withoutJudged(listing());
t('a leader with nobody behind it is not a group', !r.rows.some((g) => g.leader_id === 200));

// The same shape reached the other way: the leader of a pair is judged, so
// the lone survivor would be promoted into a group of one.
judgedIds.clear();
judgedIds.add(200);
r = withoutJudged(listing());
t('a pair whose leader was judged leaves no group of one', !r.rows.some((g) => g.leader_id === 201));

judgedIds.clear();
judgedIds.add(102);
r = withoutJudged(listing());
t('sample stays aligned with the surviving members', r.rows[0].sample.join() === 'S101,S103');

judgedIds.clear();
judgedIds.add(104);
const original = listing();
const before = JSON.stringify(original);
withoutJudged(original);
t('the cached entry handed in is never mutated', JSON.stringify(original) === before);

console.log(`\n${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
