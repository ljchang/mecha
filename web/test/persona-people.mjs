// The People tab's pure logic, imported from the shipped module.
import assert from 'node:assert/strict';
import { ENDPOINTS, personaUrl, personUrl, peopleUrl, acceptedLine, ringLayout, RINGS, initials, eventLine, personBody, PEOPLE_TIES, PEOPLE_EVENTS, PEOPLE_LAYERS } from '../src/lib/persona.js';

const ID = 'a'.repeat(32);
const EV = 'b'.repeat(32);

// Every door is an endpoint the builders accept, and so one the demo checks.
assert.equal(personaUrl('mara', '/people', 'tok'), '/api/personas/mara/people?unlock=tok');
assert.equal(personUrl('mara', ID), `/api/personas/mara/people/${ID}`);
assert.equal(personUrl('mara', ID, ''), `/api/personas/mara/people/${ID}/events`);
assert.equal(personUrl('mara', ID, EV, 'tok'), `/api/personas/mara/people/${ID}/events/${EV}?unlock=tok`);
for (const path of ['/api/personas/X/people', '/api/personas/X/people/X', '/api/personas/X/people/X/events', '/api/personas/X/people/X/events/X']) {
  assert.ok(ENDPOINTS.includes(path), path);
}
// An id the store would not mint is refused before it becomes a path.
for (const bad of ['../x', 'A'.repeat(32), 'a'.repeat(31), '', null]) {
  assert.throws(() => personUrl('mara', bad), /not a person/);
}
assert.throws(() => personUrl('mara', ID, '../../x'), /not an event/);

// Rings: each layer on its radius, the unplaced on theirs, the ended on the
// outer band, and the same network laid out the same way every time.
const person = (id, name, edge) => ({ id: id.repeat(32).slice(0, 32), name, edge });
const people = [
  person('1', 'Ines', { layer: 5, valence: 'warm', status: 'active' }),
  person('2', 'Bo', { layer: 5, valence: 'tense', status: 'active' }),
  person('3', 'Cal', { layer: null, status: 'active' }),
  person('4', 'Sam', { layer: 50, status: 'ended' }),
  person('5', 'Dee', { layer: 150, status: 'paused', unreadable: 1 }),
];
const laid = ringLayout(people);
const at = (name) => laid.find((n) => n.name === name);
const radius = (n) => Math.round(Math.hypot(n.x, n.y));
assert.equal(radius(at('Ines')), RINGS[5]);
assert.equal(radius(at('Cal')), RINGS.none);
assert.equal(radius(at('Sam')), RINGS.ended, 'an ended tie is on the outer band whatever its layer');
assert.equal(radius(at('Dee')), RINGS[150]);
assert.equal(at('Dee').unvouched, true);
assert.equal(at('Bo').valence, 'tense');
// Two on one ring sit apart, Bo first by name, at the top of its ring.
assert.notDeepEqual([at('Bo').x, at('Bo').y], [at('Ines').x, at('Ines').y]);
assert.deepEqual(ringLayout([...people].reverse()), laid, 'order in is not order out');
assert.deepEqual(ringLayout(null), []);

assert.equal(initials('Ines'), 'IN');
assert.equal(initials('Dr. Ada Okafor'), 'DO');
assert.equal(initials(''), '?');

// A timeline line: when as said, what in the page's words, tie and context.
assert.equal(
  eventLine({ event: 'formed', tie: 'kin', label: 'sister', layer: 5, when: { said: '2019' }, context: 'at the wedding' }),
  '2019 · became close, sister, closest 5 · at the wedding',
);
assert.equal(eventLine({ event: 'conflict', when: { said: null } }), 'undated · fell out');
assert.equal(eventLine({ event: 'eloped', when: {} }), 'undated · an event this version cannot read');

// A body carries what was given and the unlock, and nothing left undefined.
assert.deepEqual(personBody({ name: 'Ines', role: undefined, layer: 5 }, 'tok'), { name: 'Ines', layer: 5, unlock: 'tok' });
assert.deepEqual(personBody({ role: '' }), { role: '' }, 'an empty value clears, so it is sent');

assert.equal(PEOPLE_TIES.length, 9);
assert.equal(PEOPLE_EVENTS.length, 8);
assert.deepEqual(PEOPLE_LAYERS, [5, 15, 50, 150]);
// The time slider's read, and the proposal doors.
assert.equal(peopleUrl('mara'), '/api/personas/mara/people');
assert.equal(peopleUrl('mara', '2025-01-31', 'tok'), '/api/personas/mara/people?unlock=tok&as_of=2025-01-31');
assert.throws(() => peopleUrl('mara', 'last spring'), /not a date/);
assert.ok(ENDPOINTS.includes('/api/personas/X/people/proposal'));
assert.ok(ENDPOINTS.includes('/api/personas/X/people/proposal/accept'));
assert.equal(acceptedLine({ added: ['Theo'], already: ['Ines'], waiting: 1 }), 'Added Theo. Ines was already in their network. 1 more still waits below.');
assert.equal(acceptedLine({ added: [], already: [], waiting: 0 }), 'Nothing to add.');
console.log('persona-people: ok');

// The demo looks back as `Network::as_of` does: someone with no event at
// all has been there from the beginning, and someone whose events all come
// later is not there yet (review of #804).
{
  const fx = await import('../src/demo/fixtures.js');
  const before = fx.personaPeople.people.length;
  const extra = { ...fx.personaPeople.people[0], id: 'c'.repeat(32), name: 'Quill', events: [] };
  fx.personaPeople.people.push(extra);
  try {
    const early = fx.personaPeopleAsOf('1900-01-01');
    assert.deepEqual(early.people.map((p) => p.name), ['Quill']);
    assert.equal(early.people[0].edge, extra.edge, 'no events: the edge as it stands');
    assert.equal(fx.personaPeopleAsOf('2999-01-01').people.length, before + 1);
  } finally {
    fx.personaPeople.people.pop();
  }
}
