// The image library page's pure logic, imported from the shipped module.
import assert from 'node:assert/strict';
import {
  PANES, paneOf, entriesFor, counts, originLabel, listUrl, validName, tameName,
  TEXT_MAX, fitWithin, formProblem, formBody,
} from '../src/lib/library.js';

const entries = [
  { kind: 'character', name: 'maya', status: 'approved' },
  { kind: 'character', name: 'sam', status: 'candidate' },
  { kind: 'style', name: 'noir', status: 'approved' },
  { kind: 'style', name: 'pastel', status: 'candidate' },
];

// A hash sub that is not a pane lands on characters, never on nothing.
assert.equal(paneOf('candidates'), 'candidates');
assert.equal(paneOf('styles'), 'styles');
assert.equal(paneOf(''), 'characters');
assert.equal(paneOf('nonsense'), 'characters');
assert.deepEqual(PANES, ['characters', 'styles', 'candidates']);

// Candidates are their own pane whatever their kind, and never among the
// approved: a card there would read as usable.
assert.deepEqual(entriesFor('characters', entries).map((e) => e.name), ['maya']);
assert.deepEqual(entriesFor('styles', entries).map((e) => e.name), ['noir']);
assert.deepEqual(entriesFor('candidates', entries).map((e) => e.name), ['sam', 'pastel']);
assert.deepEqual(entriesFor('characters', null), []);
assert.deepEqual(counts(entries), { characters: 1, styles: 1, candidates: 2 });

// An origin the page does not know reads as the cautious one.
assert.match(originLabel('model_untrusted'), /read it before approving/);
assert.equal(originLabel('owner'), 'yours');
assert.match(originLabel('something new'), /read it before approving/);

// The token rides only while there is one, encoded.
assert.equal(listUrl(null), '/api/library');
assert.equal(listUrl('ab/c'), '/api/library?unlock=ab%2Fc');

// Names as the store takes them; typing is tamed, never invented.
assert.ok(validName('maya-2'));
for (const bad of ['', 'Maya', '-x', 'a b', 'x'.repeat(65)]) assert.ok(!validName(bad), bad);
assert.equal(tameName('  Maya Lee '), 'maya-lee');
assert.equal(tameName('Théo!'), 'tho');
assert.equal(tameName(''), '');

// A portrait is only ever scaled down, keeping its shape.
assert.deepEqual(fitWithin(4000, 3000, 1536), { w: 1536, h: 1152 });
assert.deepEqual(fitWithin(3000, 4000, 1536), { w: 1152, h: 1536 });
assert.deepEqual(fitWithin(800, 600, 1536), { w: 800, h: 600 });

// The add form: a name, text within the store's cap, and a portrait for a
// character — a style has none.
const add = { mode: 'add', kind: 'character', name: 'theo', text: 'a lanky man with red hair', portrait: 'AAAA' };
assert.equal(formProblem(add), null);
assert.match(formProblem({ ...add, name: 'Theo' }), /lowercase/);
assert.match(formProblem({ ...add, text: '  ' }), /description/);
assert.match(formProblem({ ...add, text: 'x'.repeat(TEXT_MAX.character + 1) }), /at most 400/);
assert.match(formProblem({ ...add, portrait: null }), /portrait/);
assert.equal(formProblem({ ...add, kind: 'style', portrait: null }), null);
assert.deepEqual(formBody({ ...add, locked: true }), {
  kind: 'character', name: 'theo', text: 'a lanky man with red hair', locked: true, portrait: 'AAAA',
});
assert.equal(formBody({ ...add, kind: 'style' }).portrait, undefined);

// The edit form sends only what changed, and the token for a locked entry.
const entry = { kind: 'character', name: 'maya', text: 'a woman in her mid-30s' };
const edit = { mode: 'edit', kind: 'character', text: 'a woman in her mid-30s ', portrait: null };
assert.match(formProblem(edit, entry), /nothing changed/);
assert.deepEqual(formBody({ ...edit, portrait: 'BBBB' }, entry), { kind: 'character', name: 'maya', portrait: 'BBBB' });
assert.deepEqual(formBody({ ...edit, text: 'a woman of 40' }, entry, 'tok'), {
  kind: 'character', name: 'maya', text: 'a woman of 40', unlock: 'tok',
});

console.log('library.mjs: ok');
