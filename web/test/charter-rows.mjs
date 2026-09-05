// The settings page's rows carry what the server put beside each line.
//
// `npm test` in web/. Plain node, like `queue-logic.mjs` and for the same
// reason. `rows` is the one place a server field becomes an editor row, and
// the shape it must keep is two-sided: `sensor` narrowed to exactly the keys
// `serialize` writes back, and `reading` carried for display — the field the
// first cut dropped, leaving the browser the one charter surface with no
// reading while the payload had one.
import assert from 'node:assert/strict';
import { readingStands, rows, sensorProblems, serialize } from '../src/lib/charter-toml.js';

let uid = 0;
const next = () => ++uid;

const served = [
  { id: 'plain', text: 'No sensor.' },
  {
    id: 'waits',
    text: 'Keep it short.',
    sensor: { kind: 'outbox_age', setpoint: '24h', stray: 'must not survive' },
    reading: { state: 'observed', over: true, summary: '3d, past the 24h setpoint' },
  },
];

const out = rows(served, next);
assert.equal(out.length, 2);
assert.deepEqual(out[0], { uid: 1, id: 'plain', text: 'No sensor.', sensor: null, reading: null, read_for: null });
assert.deepEqual(out[1].sensor, { kind: 'outbox_age', setpoint: '24h' });
assert.equal(out[1].reading.summary, '3d, past the 24h setpoint');
assert.equal(out[1].reading.over, true);

// A row without a reading (an older server, or a line whose store was not
// read) is null, never undefined — the template branches on it.
assert.equal(rows([{ id: 'x', text: 'y', sensor: { kind: 'outbox_waiting', setpoint: '3' } }], next)[0].reading, null);
assert.deepEqual(rows(undefined, next), []);

// The reading never reaches the file: serialising the rows writes the
// sensor's two keys and nothing of the reading.
const toml = serialize('', out);
assert.match(toml, /kind = "outbox_age"/);
assert.match(toml, /setpoint = "24h"/);
assert.doesNotMatch(toml, /reading|observed|past the/);


// A sensor the owner typed in the form is written exactly as one read from
// the file — the same two keys — and a half-filled one is named before the
// save rather than dropped silently by `serialize`.
const typed = [{ id: 'q', text: 'Answer fast.', sensor: { kind: 'question_latency', setpoint: '12h' }, reading: null }];
assert.match(serialize('', typed), /\[line\.sensor\]\nkind = "question_latency"\nsetpoint = "12h"/);
assert.deepEqual(sensorProblems(typed), []);
assert.deepEqual(sensorProblems([{ id: 'a', text: 't', sensor: { kind: '', setpoint: '' } }]), [
  "Line 1's sensor needs a kind and a setpoint, or remove it.",
]);
assert.deepEqual(sensorProblems([{ id: 'a', text: 't', sensor: { kind: 'outbox_age', setpoint: ' ' } }]), [
  "Line 1's sensor has no setpoint.",
]);
assert.deepEqual(sensorProblems([{ id: 'a', text: 't', sensor: { kind: '', setpoint: '3' } }]), ["Line 1's sensor has no kind."]);
assert.deepEqual(sensorProblems([{ id: 'a', text: 't', sensor: null }, { id: 'b', text: 'u' }]), []);
// And an empty sensor writes no table: the problem above is the only thing
// standing between the owner and a silent drop.
assert.doesNotMatch(serialize('', [{ id: 'a', text: 't', sensor: { kind: '', setpoint: '' } }]), /line\.sensor/);


// The reading stands beside the sensor it was computed against and stands
// down the moment the owner changes the kind or the setpoint in place — a
// reading that says "within the 24h setpoint" beside a setpoint of 5 would be
// the guard reassuring about the old value.
const withReading = rows([{ id: 'w', text: 't', sensor: { kind: 'outbox_age', setpoint: '24h' }, reading: { state: 'observed', over: false, summary: '3h, within the 24h setpoint' } }], next)[0];
assert.deepEqual(withReading.read_for, { kind: 'outbox_age', setpoint: '24h' });
assert.equal(readingStands(withReading), true);
withReading.sensor.setpoint = '1h';
assert.equal(readingStands(withReading), false);
withReading.sensor.setpoint = ' 24h ';
assert.equal(readingStands(withReading), true, 'the owner\'s spelling, trimmed');
withReading.sensor.kind = 'outbox_waiting';
assert.equal(readingStands(withReading), false);
assert.equal(readingStands(rows([{ id: 'x', text: 'y', sensor: { kind: 'outbox_age', setpoint: '24h' } }], next)[0]), false, 'no reading served');
assert.equal(readingStands({ sensor: null, reading: null, read_for: null }), false);


// Two lines of one kind: the parser refuses the document naming both, and
// the form says so before the save is armed.
assert.deepEqual(
  sensorProblems([
    { id: 'a', text: 't', sensor: { kind: 'outbox_age', setpoint: '24h' } },
    { id: 'b', text: 'u', sensor: { kind: 'question_latency', setpoint: '12h' } },
    { id: 'c', text: 'v', sensor: { kind: 'outbox_age', setpoint: '48h' } },
  ]),
  ['Lines 1 and 3 both carry a outbox_age sensor — keep one.']
);

console.log('charter-rows: ok');
