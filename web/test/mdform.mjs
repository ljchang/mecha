// The Markdown form's page half: the shape it edits, never Markdown itself.
import assert from 'node:assert/strict';
import { draftOf, docOf, isDirty, isFixed, move, addSection, problems } from '../src/lib/mdform.js';

const doc = {
  title: 'Mara', note: 'who she is', body: '',
  sections: [{ heading: 'Core', note: '', body: 'Dry.' }, { heading: 'Background', note: 'kelp', body: 'Reef.' }],
};

// A draft of what was read is no change; ids never reach the server.
let d = draftOf(doc, ['Core']);
assert.ok(!isDirty(doc, d, ['Core']));
assert.deepEqual(docOf(d), doc);

// Moving, adding and editing are changes.
d.sections = move(d.sections, 1, -1);
assert.deepEqual(d.sections.map((s) => s.heading), ['Background', 'Core']);
assert.ok(isDirty(doc, d, ['Core']));
assert.equal(move(d.sections, 0, -1), d.sections);
d = draftOf(doc, ['Core']);
d.sections = addSection(d.sections);
assert.deepEqual(problems(d, ['Core']), ['A section needs a heading.']);
d.sections[2].heading = 'Background';
assert.deepEqual(problems(d, ['Core']), ['Two sections are called “Background”.']);

// A file missing Core gets it back, empty, at the top — visible, not refused.
const bare = draftOf({ title: 'Ada', sections: [{ heading: 'Voice', body: 'x' }] }, ['Core']);
assert.deepEqual(bare.sections.map((s) => s.heading), ['Core', 'Voice']);
assert.ok(isFixed(['Core'], bare.sections[0]));

// A note that would end its comment is named before the save.
d = draftOf(doc, ['Core']);
d.sections[1].note = 'ends --> here';
assert.equal(problems(d, ['Core']).length, 1);

// Phone dashes in a comment are put back on the way out.
d = draftOf(doc, ['Core']);
d.sections[0].body = 'Dry. <!— aside —>';
assert.equal(docOf(d).sections[0].body, 'Dry. <!-- aside -->');

console.log('mdform: ok');
