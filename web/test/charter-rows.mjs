// The settings page's rows carry what the server put beside each line.
//
// `npm test` in web/. Plain node, like `queue-logic.mjs` and for the same
// reason. `rows` is the one place a server field becomes an editor row, and
// the shape it must keep is two-sided: `sensor` narrowed to exactly the keys
// `serialize` writes back, and `reading` carried for display — the field the
// first cut dropped, leaving the browser the one charter surface with no
// reading while the payload had one.
import assert from 'node:assert/strict';
import { rows, serialize } from '../src/lib/charter-toml.js';

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
assert.deepEqual(out[0], { uid: 1, id: 'plain', text: 'No sensor.', sensor: null, reading: null });
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

console.log('charter-rows: ok');
