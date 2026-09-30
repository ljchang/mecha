// The TOML form's page half: which values changed, and nothing about TOML.
import assert from 'node:assert/strict';
import {
  draftOf, changesOf, segmented, chipsFor, toggleChip, addChip, repairComments,
} from '../src/lib/tomlform.js';

const form = {
  sections: [
    {
      title: 'Who',
      fields: [
        { path: 'display', label: 'Name', kind: 'text', max: 80, optional: true },
        { path: 'groups', label: 'Groups', kind: 'chips', options: [{ value: 'kelp', label: 'kelp' }], free: false },
        { path: 'character', label: 'Portrait', kind: 'choice', none: 'No portrait', options: [{ value: 'mara', label: 'mara' }] },
      ],
    },
    {
      title: 'Safety',
      fields: [
        { path: 'safety.crisis', label: 'Crisis', kind: 'toggle' },
        { path: 'files.answers', label: 'Answers', kind: 'choice', options: [{ value: 'open', label: 'Open' }, { value: 'files', label: 'Files only' }] },
      ],
    },
  ],
};
const values = { display: 'Mara', groups: ['kelp'], character: null, 'safety.crisis': true, 'files.answers': 'open' };

// Nothing touched is nothing sent — a draft of what was read is no change.
const draft = draftOf(form, values);
assert.deepEqual(changesOf(form, values, draft), {});
assert.equal(draftOf(form, {}).display, '');
assert.deepEqual(draftOf(form, {}).groups, []);

// Only what differs, and an emptied optional text removes the key.
const edited = { ...draft, 'safety.crisis': false, display: '  ', groups: ['kelp'], character: 'mara' };
assert.deepEqual(changesOf(form, values, edited), { 'safety.crisis': false, display: null, character: 'mara' });
// A stray space is not a change.
assert.deepEqual(changesOf(form, values, { ...draft, display: 'Mara ' }), {});

// Two short options: segmented. A choice that may be unset: a select.
assert.ok(segmented(form.sections[1].fields[1]));
assert.ok(!segmented(form.sections[0].fields[2]));

// A value the options do not list is still drawn, so it can be removed.
assert.deepEqual(chipsFor(form.sections[0].fields[1], ['kelp', 'reef']).map((c) => [c.value, !!c.extra]), [['kelp', false], ['reef', true]]);
assert.deepEqual(toggleChip(['kelp'], 'kelp'), []);
assert.deepEqual(toggleChip([], 'kelp'), ['kelp']);
assert.deepEqual(addChip(['a'], '  b '), ['a', 'b']);
assert.deepEqual(addChip(['a'], 'a'), ['a']);
assert.deepEqual(addChip(['a'], '   '), ['a']);

// A comment typed on a phone, dashes "smartened": put back. Other dashes stay.
assert.equal(repairComments('<!— a note —>'), '<!-- a note -->');
assert.equal(repairComments('<!–note–>'), '<!--note-->');
assert.equal(repairComments('Dry — and kind.'), 'Dry — and kind.');

console.log('tomlform: ok');
