// The image library page's pure logic, imported from the shipped module.
import assert from 'node:assert/strict';
import {
  PANES, paneOf, entriesFor, counts, pendingLine, refApprovable, originLabel, listUrl, thumbUrl, validName, tameName, voicesUrl, voiceLine,
  sampleUrl, cloneUrl, chatVoices, VOICE_ENDPOINTS,
  TEXT_MAX, fitWithin, formProblem, formBody, previewOf,
  REF_KINDS, REF_LIMIT, refNote, refAddable, refProblem, refBody, refRemoveBody, LABEL_MAX,
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
assert.deepEqual(PANES, ['characters', 'styles', 'locations', 'voices', 'candidates']);
assert.equal(paneOf('voices'), 'voices');
assert.deepEqual(entriesFor('voices', entries), []);
// The voice library's URL carries the unlock only when there is one.
assert.equal(voicesUrl(null), '/api/library/voices');
assert.equal(voicesUrl('a b'), '/api/library/voices?unlock=a%20b');
// So do a sample's, and a clip saved locked says so.
assert.equal(sampleUrl('ada', null), '/api/library/voices/sample?name=ada');
assert.equal(sampleUrl('ada', 't 1'), '/api/library/voices/sample?name=ada&unlock=t%201');
assert.equal(cloneUrl('ada'), '/api/settings/voice/clone?name=ada');
assert.equal(cloneUrl('ada', true), '/api/settings/voice/clone?name=ada&locked=true');
// The chat's picker offers what the library served it, less a name on
// neither the server nor this box; one it could not be asked about stays,
// and so does a clone the server has not read yet.
assert.deepEqual(
  chatVoices({ voices: [
    { name: 'ada', listed: true }, { name: 'gone', listed: false }, { name: 'fresh', listed: null },
    { name: 'justmade', listed: false, cloned: { seconds: 20 } },
  ] }),
  ['ada', 'fresh', 'justmade'],
);
assert.deepEqual(chatVoices(null), []);
// Every URL the builders return is on the list check-demo reads.
for (const url of [voicesUrl(null), voicesUrl('t'), sampleUrl('ada', 't'), cloneUrl('ada', true)]) {
  assert.ok(VOICE_ENDPOINTS.includes(url.split('?')[0]), url);
}
// A voice's line: what it is, who speaks in it, and "not yet" only when the
// server's list was read and lacks it.
assert.equal(
  voiceLine({ name: 'ada', listed: true, cloned: { seconds: 31.6 }, used_by: ['Mara'] }),
  'cloned here · 32s reference · Mara speaks in it',
);
assert.equal(voiceLine({ name: 'default', listed: true, cloned: null, used_by: [] }), "the voice server's own");
assert.equal(voiceLine({ name: 'solo', listed: false, cloned: {}, used_by: ['Mara', 'Ada'] }), 'cloned here · not on the voice server yet · Mara, Ada speak in it');
assert.equal(voiceLine({ name: 'x', listed: null, cloned: {}, used_by: [] }), 'cloned here');
// A clone's row says when "who speaks in it" may be short — and only a
// clone's, since that is the row with Delete.
assert.equal(
  voiceLine({ name: 'ada', listed: true, cloned: { seconds: 20 }, used_by: [] }, 'locked'),
  'cloned here · 20s reference · unlock to see every persona that speaks in it',
);
assert.equal(
  voiceLine({ name: 'ada', listed: true, cloned: {}, used_by: [] }, 'unreadable'),
  'cloned here · who speaks in it could not be fully read',
);
assert.equal(voiceLine({ name: 'default', listed: true, cloned: null, used_by: [] }, 'locked'), "the voice server's own");
// The clone folder unread: a listed voice is not claimed as the server's own.
assert.equal(voiceLine({ name: 'ada', listed: true, cloned: null, used_by: [] }, null, true), '');
// Nor "nor this box" for a voice the server lacks: this box was not read.
assert.equal(voiceLine({ name: 'adaa', listed: false, cloned: null, used_by: ['Mara'] }, null, true), 'Mara speaks in it');
// The server not asked: nothing claimed about where the voice comes from.
assert.equal(voiceLine({ name: 'adaa', listed: null, cloned: null, used_by: ['Mara'] }), 'Mara speaks in it');
assert.equal(voiceLine({ name: 'adaa', listed: null, cloned: null, used_by: [] }), '');
// Named by a persona and nowhere else: said, so a typo shows before a call.
assert.equal(
  voiceLine({ name: 'adaa', listed: false, cloned: null, used_by: ['Mara'] }),
  'on neither the voice server nor this box — a call in it is refused · Mara speaks in it',
);

// Candidates are their own pane whatever their kind, and never among the
// approved: a card there would read as usable.
assert.deepEqual(entriesFor('characters', entries).map((e) => e.name), ['maya']);
assert.deepEqual(entriesFor('styles', entries).map((e) => e.name), ['noir']);
assert.deepEqual(entriesFor('candidates', entries).map((e) => e.name), ['sam', 'pastel']);
assert.deepEqual(entriesFor('characters', null), []);
assert.deepEqual(counts(entries), { characters: 1, styles: 1, locations: 0, candidates: 2 });

// A location has its own pane and count, and a kind with no pane is
// counted nowhere, never as a character (review of #740).
const withPlaces = [
  ...entries,
  { kind: 'location', name: 'boathouse', status: 'approved' },
  { kind: 'outfit', name: 'later', status: 'approved' },
];
assert.deepEqual(entriesFor('locations', withPlaces).map((e) => e.name), ['boathouse']);
assert.deepEqual(entriesFor('characters', withPlaces).map((e) => e.name), ['maya']);
assert.deepEqual(counts(withPlaces), { characters: 1, styles: 1, locations: 1, candidates: 2 });
assert.equal(paneOf('locations'), 'locations');

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

// References: the store's limits, approved characters only.
assert.deepEqual(REF_LIMIT, { face: 1, full_body: 1, detail: 4 });
for (const k of REF_KINDS) assert.ok(refNote(k), k);
const wren = { kind: 'character', name: 'wren', status: 'approved', references: [{ kind: 'full_body', label: null }] };
assert.equal(refAddable(wren, 'full_body'), false, 'one full body only');
assert.equal(refAddable(wren, 'face'), true);
assert.equal(refAddable({ ...wren, status: 'candidate' }, 'face'), false, 'a candidate takes none');
assert.equal(refAddable({ kind: 'style', name: 'noir', status: 'approved' }, 'detail'), false);
const four = { ...wren, references: ['a', 'b', 'c', 'd'].map((label) => ({ kind: 'detail', label })) };
assert.equal(refAddable(four, 'detail'), false, 'four details at most');
// A detail needs a label, and a label is unique within its kind.
assert.equal(refProblem({ kind: 'full_body', label: '', picture: null }), 'a picture');
assert.equal(refProblem({ kind: 'full_body', label: '', picture: 'AA' }), null);
assert.match(refProblem({ kind: 'detail', label: ' ', picture: 'AA' }), /label/);
assert.match(refProblem({ kind: 'detail', label: 'x'.repeat(LABEL_MAX + 1), picture: 'AA' }), /at most/);
assert.match(refProblem({ kind: 'detail', label: 'a', picture: 'AA' }, four), /does not already have/);
assert.equal(refProblem({ kind: 'detail', label: 'scar', picture: 'AA' }, four), null);
// Bodies: trimmed label only when given, the token only when held.
assert.deepEqual(refBody({ kind: 'full_body', label: ' ', picture: 'AA' }, wren), { name: 'wren', as: 'full_body', picture: 'AA' });
assert.deepEqual(refBody({ kind: 'detail', label: ' tattoo ', picture: 'AA' }, wren, 't'), {
  name: 'wren', as: 'detail', picture: 'AA', label: 'tattoo', unlock: 't',
});
assert.deepEqual(refRemoveBody(wren, { kind: 'full_body', label: null }), { name: 'wren', as: 'full_body' });
assert.deepEqual(refRemoveBody(wren, { kind: 'detail', label: 'scar' }, 't'), { name: 'wren', as: 'detail', label: 'scar', unlock: 't' });

// Staged reference changes count as waiting, and read as what they do.
assert.equal(counts(entries, [{ id: 'a' }, { id: 'b' }]).candidates, 4);
assert.equal(counts(entries).candidates, 2);
assert.equal(pendingLine({ action: 'add', name: 'wren', kind: 'full_body' }), "Add wren's full body");
assert.equal(pendingLine({ action: 'remove', name: 'wren', kind: 'detail', label: 'scar' }), "Remove wren's detail · scar");
// Approve only what this build can apply.
assert.equal(refApprovable({ action: 'add' }), true);
assert.equal(refApprovable({ action: 'remove' }), true);
assert.equal(refApprovable({ action: 'unknown' }), false);
assert.equal(refApprovable({}), false);
// A picked picture's preview is a data: URL, which serve's CSP allows
// (img-src 'self' data:); a blob: URL rendered broken until saved.
assert.equal(previewOf('AAAA'), 'data:image/jpeg;base64,AAAA');


// A thumbnail is the picture route with `size=thumb-v1`, beside any unlock; any
// other picture (a preview, nothing at all) is left alone.
assert.equal(thumbUrl('/api/library/portrait/sha256-ab.png'), '/api/library/portrait/sha256-ab.png?size=thumb-v1');
assert.equal(
  thumbUrl('/api/library/portrait/sha256-ab.png?unlock=t1'),
  '/api/library/portrait/sha256-ab.png?unlock=t1&size=thumb-v1',
);
assert.equal(thumbUrl('data:image/png;base64,AAAA'), 'data:image/png;base64,AAAA');
assert.equal(thumbUrl(null), null);

console.log('library.mjs: ok');
