// A reply tidied for speech, and cut into pieces the player can ask for one
// at a time (`speech.js`; the owner's ask, 2026-10-01).
import assert from 'node:assert/strict';
import { ownWords } from '../src/lib/persona.js';
import { replyContext, replyKey, speakable, speechPieces, speechSentences } from '../src/lib/speech.js';

// Marks go, words stay.
assert.equal(speakable('## Plan\n\n- **first**, check the *traps*\n- then `count` them'), 'Plan. first, check the traps. then count them.');
// Code is never read out.
assert.equal(speakable('Run this:\n```bash\nrm -rf /tmp/x\n```\nThen wait.'), 'Run this: There is a code block here. Then wait.');
// A link is its words; a bare URL is "a link".
assert.equal(speakable('See [the guide](https://example.com/g) or https://example.com/x.'), 'See the guide or a link.');
// A persona's citation is where it came from.
assert.equal(
  speakable('Urchins graze kelp [urchin-barrens.pdf, p. 4: "grazing fronts move"] in winter.'),
  'Urchins graze kelp (from urchin barrens) in winter.',
);
assert.equal(speakable('As noted [@kelp/field_guide.md: "low tide"].'), 'As noted (from field guide).');
// Nothing to say is nothing.
assert.equal(speakable(''), '');
assert.equal(speakable('---'), '');

// Pieces: one sentence each, in order, none over the cap, nothing lost.
const long = 'One short. ' + 'A much longer sentence that goes on for a while. '.repeat(12) + 'Last.';
const pieces = speechSentences(long, 120);
assert.equal(pieces.length, 14);
assert.equal(pieces[0], 'One short.');
assert.ok(pieces.every((p) => p.length <= 120), JSON.stringify(pieces));
assert.equal(pieces.join(' '), long.replace(/\s+/g, ' ').trim());
// A sentence over the cap is cut at a word, never mid-word.
const run = 'word '.repeat(60).trim() + '.';
const cut = speechSentences(run, 50);
assert.ok(cut.every((p) => p.length <= 50 && p.split(' ').every((w) => w === 'word' || w === 'word.')), JSON.stringify(cut));
assert.deepEqual(speechSentences(''), []);
// A number, an initialism and a closing quote stay with their sentence.
assert.deepEqual(speechSentences('It cost 3.5 million, i.e.a lot. She said "go!" Then left.'), [
  'It cost 3.5 million, i.e.a lot.',
  'She said "go!"',
  'Then left.',
]);
// Ellipses end a sentence without leaving a lone dot to speak.
assert.deepEqual(speechSentences('Well... maybe. Fine...'), ['Well...', 'maybe.', 'Fine...']);
assert.deepEqual(speechSentences('Hm. ... !'), ['Hm.']);

// The player's pieces: whole sentences grouped up to the cap from the very
// first — a short opener rides with what follows rather than going to the
// engine alone — nothing lost.
const grouped = speechPieces(long, 120);
assert.ok(grouped[0].startsWith('One short. A much longer'), grouped[0]);
assert.ok(grouped.length > 2 && grouped.length < pieces.length, JSON.stringify(grouped));
assert.ok(grouped.every((p) => p.length <= 120), JSON.stringify(grouped));
assert.equal(grouped.join(' '), long.replace(/\s+/g, ' ').trim());
assert.deepEqual(speechPieces('Just one.'), ['Just one.']);
assert.deepEqual(speechPieces(''), []);

// A reply's key: stable, plain, and different for different text.
assert.equal(replyKey('The dig went well.'), replyKey('The dig went well.'));
assert.notEqual(replyKey('The dig went well.'), replyKey('The dig went well!'));
assert.match(replyKey(''), /^r[0-9a-f]{16}$/);

// The moment a reply was said in: the owner's words before it, and the
// speaker's reply before those.
const entries = [
  { kind: 'user', text: 'Hi' },
  { kind: 'assistant', text: 'Hello!' },
  { kind: 'user', text: 'How did the dig go?' },
  { kind: 'tool', text: 'kg_search' },
  { kind: 'assistant', text: 'Well.' },
];
assert.deepEqual(replyContext(entries, 4), { asked: 'How did the dig go?', lastReply: 'Hello!' });
assert.deepEqual(replyContext(entries, 1), { asked: 'Hi', lastReply: null });
assert.deepEqual(replyContext(entries, 0), { asked: null, lastReply: null });
// The owner's words as the page shows them: a persona chat's preamble goes.
const framed = [{ kind: 'user', text: '(What I want from this conversation: rest)\n\nHow did the dig go?' }];
assert.equal(replyContext([...framed, { kind: 'assistant', text: 'Well.' }], 1, ownWords).asked, 'How did the dig go?');

console.log('speech: ok');
