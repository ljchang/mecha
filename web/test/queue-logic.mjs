// Behaviour checks for the review queue's cache filtering.
//
// `npm test` in web/. Plain node, no framework and no dependency — the pane
// had no JS rig and adding one for a single pure function would be a larger
// commitment than the function is worth.
//
// **Why this exists at all.** Most of what Queue.svelte does is assertable
// from Rust by reading the source (`serve/review.rs` does exactly that, and
// those tests catch a call reappearing where it must not). `withoutJudged` is
// the one piece that is real logic rather than a shape: which groups survive a
// verdict filed somewhere else, which are dropped, and which fields stop being
// true when one shrinks. A source-string assertion cannot tell whether any of
// that is correct, and successive rounds of review found staleness bugs in
// exactly this area — the cache is the part of the pane where being wrong is
// silent.
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

// Leader #100 with six members, plus a PAIR (#200 + one member) — the
// commonest group size, and the one where a single verdict leaves a leader
// standing alone.
//
// `sample` is deliberately populated but never trusted as an id-to-statement
// map: whether its order matches `members` is the graph's serialisation
// detail, and the function under test is written not to depend on it.
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
  considered: 7013,
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

// A judged LEADER drops the group rather than promoting an heir. The only
// id-to-statement mapping available here is `sample`, whose alignment with
// `members` is another repository's serialisation detail — and a wrong
// mapping would show one candidate's words over Accept-all and file the
// verdict on another. `reconcileGroup` promotes instead, because it holds the
// real statements by id.
judgedIds.clear();
judgedIds.add(100);
r = withoutJudged(listing());
t('a judged leader drops its group rather than promoting an heir', !r.rows.some((g) => g.leader_id === 100 || g.leader_id === 101));
t('the untouched sibling group survives it', r.rows.length === 1 && r.rows[0].leader_id === 200);

// The leader statement is what a verdict is filed under, so a surviving group
// keeps the graph's own, untouched.
judgedIds.clear();
judgedIds.add(104);
r = withoutJudged(listing());
t('a surviving group keeps the leader statement the graph sent', r.rows[0].leader_statement === 'S100');
// `sample` cannot be trimmed without the mapping this function refuses to
// assume, so it is emptied rather than guessed at.
t('a shrunken group shows no invented member lines', r.rows[0].sample.length === 0);
// The denominator was measured over the queue as it was at fetch time.
t('an edited listing drops its frozen total', r.considered === null);

// A pair, both ways round. Judging the member leaves the leader alone;
// judging the leader would leave the member alone. Neither is a group, and a
// card offering "Reject all 1" covers one candidate the item list already
// shows. `reconcileGroup` applies the same rule on the live path.
judgedIds.clear();
judgedIds.add(201);
r = withoutJudged(listing());
t('a leader with nobody behind it is not a group', !r.rows.some((g) => g.leader_id === 200));

judgedIds.clear();
judgedIds.add(200);
r = withoutJudged(listing());
t('a pair whose leader was judged leaves no group of one', !r.rows.some((g) => g.leader_id === 201));

judgedIds.clear();
judgedIds.add(104);
const original = listing();
const before = JSON.stringify(original);
withoutJudged(original);
t('the cached entry handed in is never mutated', JSON.stringify(original) === before);

console.log(`\n${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
