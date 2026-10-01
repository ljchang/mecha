// Behaviour checks for the feature readings (src/lib/features.js).
//
// **What is worth pinning.** The readings where being wrong is silent: a
// page that hides tabs before `/api/features` has answered, or when it
// failed; an `unready` feature the owner switched on, hidden; a queue with
// strangers' requests waiting, hidden because its switch is off; and a
// banner that names the child when the parent is what is broken.
import { OPENS_ANYWAY, opensAnyway, opens, bounceFor, featureOf, refuses, index, isShown, queueCardShown, banner, hiddenLine, summary, tree, detail, VIEW_FEATURE } from '../src/lib/features.js';

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
t('but not a queue card\'s landing, which goes flat when off', !opensAnyway('library', 'candidates'));
t('the board itself does not', !opensAnyway('tasks', null) && !opensAnyway('tasks', 'actionable'));
t('a hidden view offers only what opens', opens(rows, 'tasks', 'waiting') && !opens(new Map([['tasks', row('tasks', 'blocked', { on: 'graph' })]]), 'tasks', 'done'));
t('a shown view offers everything', opens(rows, 'tasks', 'done') && opens(rows, 'graph', null));
t('an unanswered read offers everything', opens(null, 'library', 'styles'));
t('a feature\'s pane in a core view is the pane\'s', featureOf('review', 'graph') === 'graph' && featureOf('review', 'frontdoor') === 'frontdoor');
t('a core pane of a core view is nobody\'s', featureOf('review', 'outbox') === null);
t('a feature\'s view is the view\'s', featureOf('library', 'candidates') === 'library' && featureOf('tasks', null) === 'tasks');
t('every entry is view/sub', OPENS_ANYWAY.every((r) => /^[a-z]+\/[a-z]+$/.test(r)));

console.log('where a link bounces');
{
  const noImages = new Map([
    ['library', row('library', 'off', { gated: true })],
    ['voice', row('voice', 'on', { gated: true })],
  ]);
  const noVoice = new Map([
    ['library', row('library', 'on', { gated: true })],
    ['voice', row('voice', 'off', { gated: true })],
  ]);
  // Library → Voices opens without the image library, as the settings pane
  // it replaced did (review of #490) — and not without voice itself.
  t('voices open with image generation off', bounceFor(noImages, 'library', 'voices') === null);
  t('the image panes do not', bounceFor(noImages, 'library', 'characters') === 'library');
  t('voices go home with voice off', bounceFor(noVoice, 'library', 'voices') === 'voice');
  t('a pane that refuses beats one that opens anyway', bounceFor(new Map([...noImages, ['voice', row('voice', 'off', { gated: true })]]), 'library', 'voices') === 'voice');
  t('a core view never bounces', bounceFor(noImages, 'chat', null) === null);
  t('not answered is shown', bounceFor(null, 'library', 'characters') === null);
  // The chip agrees with the bounce: offered with images off, never with
  // voice off (review of #490).
  t('the voices chip shows with images off', opens(noImages, 'library', 'voices') === true);
  t('and not with voice off', opens(noVoice, 'library', 'voices') === false);
  t('a core pane opening anyway still opens', opens(new Map([['tasks', row('tasks', 'off', { gated: true })]]), 'tasks', 'waiting') === true);
}

console.log('what refuses');
const gatedRows = new Map([
  ['graph', row('graph', 'off', { gated: true })],
  ['frontdoor', row('frontdoor', 'off', { gated: false })],
  ['mail', row('mail', 'on', { gated: true })],
]);
t('an off feature whose guard has landed refuses', refuses(gatedRows, 'graph'));
t('an off feature not yet guarded does not — its door stays', !refuses(gatedRows, 'frontdoor'));
t('an on feature does not', !refuses(gatedRows, 'mail'));
t('nothing refuses before the answer', !refuses(null, 'graph') && !refuses(undefined, 'graph'));

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
