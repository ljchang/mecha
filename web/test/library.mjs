// The image library page's pure logic, imported from the shipped module.
import assert from 'node:assert/strict';
import { PANES, paneOf, entriesFor, counts, originLabel, listUrl, validName, tameName } from '../src/lib/library.js';

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

console.log('library.mjs: ok');
