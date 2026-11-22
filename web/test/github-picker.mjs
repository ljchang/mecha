// Behaviour checks for Settings → GitHub's pure half (src/lib/github-picker.js).
//
// **What is worth pinning.** The readings where being wrong is silent: a
// watched fork hidden behind the forks filter, a checkbox that writes a
// redundant line (or none where one is needed), an edit offered while the
// file cannot be read, and an unknown selection or store drawn as "none".
import {
  visibleRepos,
  repoChange,
  ownerChange,
  noneStep,
  ownerLine,
  indexLine,
  filterChange,
  packageChange,
  ownerCount,
  pending,
  packageLine,
  hasPackages,
  storeLine,
  fileNotice,
} from '../src/lib/github-picker.js';

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
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const NOW = Date.parse('2031-04-20T12:00:00Z');
const repo = (name, extra = {}) => ({
  full_name: `example-lab/${name}`,
  name,
  fork: false,
  archived: false,
  private: false,
  pushed_at: '2031-04-01T00:00:00Z',
  listed: true,
  collected: false,
  included: false,
  by_owner: false,
  line: null,
  packages: [],
  ...extra,
});
const repos = [
  repo('tool-a', { included: true, by_owner: true, collected: true }),
  repo('fork-of-upstream', { fork: true }),
  repo('old-site', { archived: true, pushed_at: '2028-01-01T00:00:00Z' }),
  repo('kept-fork', { fork: true, included: true, line: true }),
  repo('no-date', { pushed_at: null }),
];
const names = (rs) => rs.map((r) => r.name);

console.log('visibleRepos');
t('forks and archived are hidden by default (G4)', same(names(visibleRepos(repos, { now: NOW })), ['tool-a', 'kept-fork', 'no-date']));
t('a selected fork is never hidden by the filter', names(visibleRepos(repos, { now: NOW })).includes('kept-fork'));
t(
  'the toggles show them',
  same(names(visibleRepos(repos, { forks: true, archived: true, now: NOW })), names(repos)),
);
t(
  '"pushed in the last year" drops the old and the undated, never a selected one',
  same(names(visibleRepos(repos, { archived: true, recent: true, now: NOW })), ['tool-a', 'kept-fork']),
);
t('search is by name, case-insensitively', same(names(visibleRepos(repos, { query: 'TOOL', now: NOW })), ['tool-a']));
t(
  'a watched fork about to be dropped stays in sight, with its tag',
  names(visibleRepos([repo('was-watched', { fork: true, collected: true, included: false })], { now: NOW })).includes(
    'was-watched',
  ),
);
t('search narrows selected ones too', same(names(visibleRepos(repos, { query: 'old', archived: true, now: NOW })), ['old-site']));

console.log('repoChange');
t(
  'ticking a repository the owner line leaves out writes its own line',
  same(repoChange(repos[1], true), { op: 'repo', name: 'example-lab/fork-of-upstream', include: true }),
);
t(
  "unticking one the owner line takes writes an exclusion",
  same(repoChange(repos[0], false), { op: 'repo', name: 'example-lab/tool-a', include: false }),
);
t(
  'agreeing with the owner line removes the exception rather than restating it',
  same(repoChange(repo('x', { by_owner: true, line: false }), true), { op: 'repo', name: 'example-lab/x', include: null }),
);
t('no edit while the owner line is unknown (the file cannot be read)', repoChange(repo('x', { by_owner: null }), true) === null);

console.log('owner and package changes');
t('select-all', same(ownerChange('example-lab', true), { op: 'select_all', login: 'example-lab' }));
t(
  'unticking all goes back to listed, keeping the hand-made lines — never select-none',
  same(ownerChange('example-lab', false), { op: 'select_listed', login: 'example-lab' }),
);

console.log('select none, confirmed in the page');
const picked = {
  login: 'example-lab',
  repos: [repo('tool-a', { line: true, included: true }), repo('tool-b', { line: false })],
  unseen: [{ name: 'example-lab/gone', include: true }],
};
const first = noneStep(picked);
t('a hand-made inclusion asks first, and sends nothing', !!first.confirm && first.change === undefined);
t(
  'the question names what goes',
  /example-lab\/tool-a/.test(first.confirm) && /example-lab\/gone/.test(first.confirm) && /2 repositories/.test(first.confirm),
);
t(
  'confirmed, it sends select-none',
  same(noneStep(picked, true), { change: { op: 'select_none', login: 'example-lab' } }),
);
t(
  'with no inclusion to lose, it sends at once',
  same(noneStep({ login: 'example-lab', repos: [repo('tool-b', { line: false })], unseen: [] }), {
    change: { op: 'select_none', login: 'example-lab' },
  }),
);
t(
  'an owner line that takes all is not an inclusion to confirm',
  noneStep({ login: 'l', repos: [repo('x', { included: true, by_owner: true, line: null })], unseen: [] }).change?.op ===
    'select_none',
);

console.log('the owner line');
t('unknown when the file cannot be used', /unknown/.test(ownerLine({ line: null }, 'invalid')));
t('no line is "not named"', /not named/.test(ownerLine({ line: null }, 'read')));
t('all', /every repository/.test(ownerLine({ line: { include: 'all', forks: false, archived: false } }, 'read')));
t('listed', ownerLine({ line: { include: 'listed', forks: false, archived: false } }, 'read') === 'only those ticked below');
t(
  'filters',
  same(filterChange('example-lab', true, false), { op: 'filters', login: 'example-lab', forks: true, archived: false }),
);
t(
  'a name, trimmed',
  same(packageChange('example-lab/tool-a', 'pypi', ' tool-a-py '), {
    repo: 'example-lab/tool-a',
    registry: 'pypi',
    to: { name: 'tool-a-py' },
  }),
);
t('none', same(packageChange('r/x', 'npm', 'none').to, 'none'));
t('back to detected', same(packageChange('r/x', 'npm', 'detected').to, 'detected'));

console.log('words');
t('a count over every repository, not only those shown', ownerCount({ repos }) === '2 of 5 selected');
t(
  'an unknown selection is said, never counted as none',
  ownerCount({ repos: [repo('x', { included: null })] }) === '1 repository · selection unknown',
);
t('pending when the next collect differs from the last', pending(repos[3]) && !pending(repos[0]));
t('not pending when unknown', !pending(repo('x', { included: null })));
t('detected says its manifest', packageLine({ state: 'detected', name: 'tool-a', manifest: 'pyproject.toml' }).text === 'tool-a · from pyproject.toml');
t('the owner\'s line is marked', packageLine({ state: 'owner', name: 'x' }).owner && packageLine({ state: 'owner_none' }).owner);
t('not scanned is not none found', packageLine({ state: 'not_scanned' }).text !== packageLine({ state: 'none_found' }).text);
t(
  'hasPackages ignores "none found" and "not scanned"',
  !hasPackages({ packages: [{ state: 'none_found' }, { state: 'not_scanned' }] }) &&
    hasPackages({ packages: [{ state: 'owner_none' }] }),
);

console.log('the store and the files');
t('no store is said', /No collection has run/.test(storeLine({ state: 'absent' }).text));
t('an unreadable store is said, and marked', storeLine({ state: 'unreadable', error: 'file is not a database' }).bad);
t('no finished run is said', /no finished collection/.test(storeLine({ state: 'read', last_collect: null }).text));
t(
  'a listing older than the last run is said',
  storeLine({
    state: 'read',
    listed_at: '2031-04-18T10:40:00.000Z',
    last_collect: { started: '2031-04-20T10:40:00.000Z', finished: '2031-04-20T10:44:00.000Z' },
  }).bad,
);
t(
  'a current listing is not',
  !storeLine({
    state: 'read',
    listed_at: '2031-04-20T10:40:00.000Z',
    last_collect: { started: '2031-04-20T10:40:00.000Z', finished: '2031-04-20T10:44:00.000Z' },
  }).bad,
);
const neverListed = {
  state: 'read',
  listed_at: null,
  last_collect: { started: '2031-04-20T10:40:00.000Z', finished: '2031-04-20T10:40:01.000Z' },
};
t(
  'a finished collect that never listed is said, and marked',
  storeLine(neverListed).bad && /never listed/.test(storeLine(neverListed).text),
);

console.log('the Settings index row');
t('before a read, what the pane is', indexLine(null).text === 'which repositories the GitHub board watches');
t(
  'never listed is not "0 of 0 watched"',
  indexLine({ selection: { state: 'absent' }, store: neverListed, owners: [] }).bad &&
    /never listed/.test(indexLine({ selection: { state: 'absent' }, store: neverListed, owners: [] }).text),
);
t(
  'an empty list is not a count',
  indexLine({
    selection: { state: 'read' },
    store: { ...neverListed, listed_at: '2031-04-20T10:40:00.000Z' },
    owners: [{ repos: [] }],
  }).text === 'nothing listed yet',
);
t(
  'a list is counted',
  indexLine({
    selection: { state: 'read' },
    store: { ...neverListed, listed_at: '2031-04-20T10:40:00.000Z' },
    owners: [{ repos }],
  }).text === '2 of 5 repositories watched',
);
t('an unusable file is marked', indexLine({ selection: { state: 'invalid' }, store: neverListed, owners: [] }).bad);
t('an unreadable store is marked', indexLine({ selection: { state: 'read' }, store: { state: 'unreadable' }, owners: [] }).bad);

t('an unusable file is said', /cannot be used/.test(fileNotice({ state: 'invalid', error: 'x' }, 'repos.toml')));
t('a readable or absent file says nothing', fileNotice({ state: 'read' }, 'x') === null && fileNotice({ state: 'absent' }, 'x') === null);

console.log(`\n${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
