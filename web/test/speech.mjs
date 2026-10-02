// A reply tidied for speech, and cut into pieces the player can ask for one
// at a time (`speech.js`; the owner's ask, 2026-10-01).
import assert from 'node:assert/strict';
import { speakable, speechChunks } from '../src/lib/speech.js';

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

// Pieces: whole sentences, in order, none over the cap, nothing lost.
const long = 'One short. ' + 'A much longer sentence that goes on for a while. '.repeat(12) + 'Last.';
const pieces = speechChunks(long, 120);
assert.ok(pieces.length > 1);
assert.ok(pieces.every((p) => p.length <= 120), JSON.stringify(pieces));
assert.equal(pieces.join(' ').replace(/\s+/g, ' '), long.replace(/\s+/g, ' ').trim());
// A sentence over the cap is cut at a word, never mid-word.
const run = 'word '.repeat(60).trim() + '.';
const cut = speechChunks(run, 50);
assert.ok(cut.every((p) => p.length <= 50 && p.split(' ').every((w) => w === 'word' || w === 'word.')), JSON.stringify(cut));
assert.deepEqual(speechChunks(''), []);

console.log('speech: ok');
