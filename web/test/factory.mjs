// The Factory view's pure helpers.
import assert from 'node:assert/strict';
import { byDay, nextWindow } from '../src/lib/factory.js';

// Two days of slots in New York, one start offering both lengths.
const slots = [
  { start: '2026-11-03T18:00:00Z', end: '2026-11-03T18:30:00Z', duration_minutes: 30 },
  { start: '2026-11-03T18:00:00Z', end: '2026-11-03T19:00:00Z', duration_minutes: 60 },
  { start: '2026-11-03T21:30:00Z', end: '2026-11-03T22:00:00Z', duration_minutes: 30 },
  { start: '2026-11-04T14:00:00Z', end: '2026-11-04T14:30:00Z', duration_minutes: 30 },
  { start: '2026-12-20T14:00:00Z', end: '2026-12-20T14:30:00Z', duration_minutes: 30 },
];
const now = Date.parse('2026-11-02T12:00:00Z');
const days = byDay(slots, 'America/New_York', { now, locale: 'en-US' });
assert.equal(days.length, 2, 'only the first two weeks');
assert.equal(days[0].starts.length, 2);
assert.deepEqual(days[0].starts[0].ds, [30, 60], 'one start, every length at it');
assert.match(days[0].starts[0].t, /^1:00\s?PM$/, 'rendered in the page zone, not UTC');
assert.match(days[1].label, /Wed/);
// The zone decides the day: 02:00Z on the 4th is the 3rd in New York.
const late = byDay([{ start: '2026-11-04T02:00:00Z', duration_minutes: 30 }], 'America/New_York', { now, locale: 'en-US' });
assert.match(late[0].label, /Nov 3/);
assert.deepEqual(byDay(null, 'America/New_York'), []);
assert.deepEqual(byDay(slots, ''), []);

// The "+" on a day.
assert.deepEqual(nextWindow([]), { start: '13:00', end: '17:00' });
assert.deepEqual(nextWindow([{ start: '09:00', end: '12:00' }]), { start: '12:00', end: '13:00' });
assert.deepEqual(
  nextWindow([{ start: '13:00', end: '17:30' }, { start: '09:00', end: '10:00' }]),
  { start: '17:30', end: '18:30' },
  'after the latest window, whatever order they were added in',
);
// Late in the day: after the last window, never inside it, and nothing
// once the day is open to its end.
const lateWindow = nextWindow([{ start: '20:00', end: '23:30' }]);
assert.deepEqual(lateWindow, { start: '23:30', end: '23:59' });
assert.ok(lateWindow.start < lateWindow.end, 'never ends before it starts');
assert.equal(nextWindow([{ start: '20:00', end: '23:59' }]), null);
assert.deepEqual(nextWindow([{ start: '21:00', end: '22:15' }]), { start: '22:15', end: '23:15' });

console.log('factory: ok');

import { nameFrom } from '../src/lib/factory.js';
assert.equal(nameFrom('Your email'), 'your_email');
assert.equal(nameFrom("Where you're from?"), 'where_you_re_from');
assert.equal(nameFrom('2nd choice'), 'f_2nd_choice');
assert.equal(nameFrom(''), 'field');
assert.equal(nameFrom('Topic', ['topic']), 'topic_2');
assert.equal(nameFrom('Topic', ['topic', 'topic_2']), 'topic_3');
assert.equal(nameFrom('Café notes'), 'cafe_notes');
console.log('factory names: ok');
// The page's own words for a window that runs backwards.
import { backwards, clock12, todayIn } from '../src/lib/factory.js';
assert.equal(clock12('21:30'), '9:30 PM');
assert.equal(clock12('00:15'), '12:15 AM');
assert.equal(clock12('12:00'), '12:00 PM');
assert.equal(
  backwards({ windows: [{ day: 'tue', start: '21:30', end: '12:00' }], overrides: [] }),
  'Tuesday starts at 9:30 PM, after it ends at 12:00 PM — check AM and PM.',
);
assert.equal(backwards({ windows: [{ day: 'wed', start: '09:00', end: '12:00' }], overrides: [] }), null);
assert.match(backwards({ windows: [], overrides: [{ date: '2026-12-24', windows: [['15:00', '10:00']] }] }), /2026-12-24 starts at 3:00 PM/);
// Today in the page's zone, not UTC: 03:00Z on the 3rd is still the 2nd in New York.
assert.equal(todayIn('America/New_York', new Date('2026-11-03T03:00:00Z')), '2026-11-02');
assert.equal(todayIn('UTC', new Date('2026-11-03T03:00:00Z')), '2026-11-03');
console.log('factory polish: ok');
