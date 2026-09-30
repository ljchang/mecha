// Behaviour checks for the feature readings (src/lib/features.js).
//
// **What is worth pinning.** The readings where being wrong is silent: a
// page that hides tabs before `/api/features` has answered, or when it
// failed; an `unready` feature the owner switched on, hidden; a queue with
// strangers' requests waiting, hidden because its switch is off; and a
// banner that names the child when the parent is what is broken.
import { OPENS_ANYWAY, opensAnyway, opens, index, isShown, queueCardShown, banner, hiddenLine, summary, tree, detail, VIEW_FEATURE } from '../src/lib/features.js';

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

// Rows as `/api/features` sends them: the state flattened into the row.
const row = (id, state, extra = {}) => ({
  id,
  label: id[0].toUpperCase() + id.slice(1),
  part_of: null,
  requires: [],
  state,
  shown: state !== 'off' && state !== 'blocked',
  next: null,
  pending: false,
  ...extra,
});
const body = {
  features: [
    row('web', 'on', { detail: 'serving' }),
    row('mail', 'off', { reason: 'not enabled in [features]', next: 'mecha features enable mail' }),
    row('graph', 'on'),
    row('tasks', 'on', { part_of: 'graph' }),
    row('image', 'unready', { reason: 'no [image] table', fix: 'add an [image] table', next: 'add an [image] table' }),
    row('library', 'on', { part_of: 'image' }),
    row('personas', 'unknown', { reason: 'the store could not be read' }),
    row('voice', 'off', { next: 'mecha features enable voice', pending: true }),
    row('dictate', 'blocked', { part_of: 'voice', requires: ['web'], on: 'voice', next: 'mecha features enable voice' }),
    row('frontdoor', 'off', { next: 'mecha features enable frontdoor' }),
  ],
  unknown_switches: [],
};
const rows = index(body);

console.log('what is shown');
t('nothing is hidden before the answer', isShown(undefined, 'mail') && isShown(null, 'mail'));
t('core views are always shown', isShown(rows, null) && isShown(rows, VIEW_FEATURE.home));
t('an off feature is hidden', !isShown(rows, 'mail'));
t('a blocked part is hidden', !isShown(rows, 'dictate'));
t('an unready feature the owner switched on is shown', isShown(rows, 'image'));
t('an unknown feature is shown', isShown(rows, 'personas'));
t('an id this serve does not know is shown', isShown(rows, 'teleport'));
t('a malformed body is unanswered, not empty', index({}) === null && index(null) === null);

console.log('queue cards');
t('an off queue with nothing waiting is hidden', !queueCardShown(rows, 'front-door requests', 0));
t('an off queue with requests waiting is shown', queueCardShown(rows, 'front-door requests', 3));
t('an off queue that could not be read is shown', queueCardShown(rows, 'front-door requests', null));
t('a core queue is shown at zero', queueCardShown(rows, 'outbox drafts', 0));

console.log('routes that open anyway');
t('a run\'s question opens on the board\'s page with the board off', opensAnyway('tasks', 'waiting'));
t('so do the workflows', opensAnyway('tasks', 'workflows'));
t('and a kept image-candidates card\'s landing', opensAnyway('library', 'candidates'));
t('the board itself does not', !opensAnyway('tasks', null) && !opensAnyway('tasks', 'actionable'));
t('a hidden view offers only what opens', opens(rows, 'tasks', 'waiting') && !opens(new Map([['tasks', row('tasks', 'blocked', { on: 'graph' })]]), 'tasks', 'done'));
t('a shown view offers everything', opens(rows, 'tasks', 'done') && opens(rows, 'graph', null));
t('an unanswered read offers everything', opens(null, 'library', 'styles'));
t('every entry is view/sub', OPENS_ANYWAY.every((r) => /^[a-z]+\/[a-z]+$/.test(r)));

console.log('banners');
const lib = banner(rows, 'library');
t('a part of an unready parent carries the parent\'s banner', lib?.label === 'Image' && lib?.word === 'unready');
t('the banner carries the command', lib?.next === 'add an [image] table');
t('an unknown feature carries its own banner', banner(rows, 'personas')?.word === 'unknown');
t('an on feature standing on on features has none', banner(rows, 'tasks') === null);
t('core has none', banner(rows, null) === null && banner(null, 'mail') === null);

console.log('a direct link to a hidden view');
const h = hiddenLine(rows, 'mail');
t('names the feature and that it is off', h?.text === 'Mail is off');
t('with the command that turns it on', h?.next === 'mecha features enable mail');
t('a blocked view names what it needs', hiddenLine(rows, 'dictate')?.text === 'Dictate needs Voice');

console.log('settings');
const s = summary(body);
t('the summary says what is wrong first', s.text.startsWith('1 not ready · 1 unreadable'));
t('and counts blocked as off', s.text.includes('4 off'));
t('and names a switch waiting on a restart', s.text.includes('1 waiting on a restart'));
t('and is bad when anything is wrong', s.bad === true);
t('an unanswered summary is null, not zero', summary(undefined) === null && summary(null) === null);
const tr = tree(body);
t('parts sit one level under their parent', tr.find((r) => r.id === 'tasks').depth === 1 && tr.find((r) => r.id === 'graph').depth === 0);
t('a blocked row says what it needs by label', detail(rows.get('dictate'), rows) === 'needs Voice');

console.log(`\n${pass} passed, ${fail} failed`);
if (fail) process.exit(1);
