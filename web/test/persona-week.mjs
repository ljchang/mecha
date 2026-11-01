// The persona week view's pure logic, imported from the shipped module.
import assert from 'node:assert/strict';
import { personaUrl, ENDPOINTS, weekUrl, weekDays, foldedHours, weekUnavailable, cellBody, WEEK_CELLS, ACTIVITY_GROUP, shiftWeek, stretchLine, dayClock, WEEK_ABSENT } from '../src/lib/persona.js';
import { PANE_FEATURE } from '../src/lib/features.js';

// The endpoint is one the builder accepts, and so one the docs demo checks.
assert.equal(personaUrl('mara', '/week', 'tok'), '/api/personas/mara/week?unlock=tok');
assert.ok(ENDPOINTS.includes('/api/personas/X/week'));
assert.equal(weekUrl('mara', null, null), '/api/personas/mara/week');
assert.equal(weekUrl('mara', '2026-11-02', 'tok'), '/api/personas/mara/week?unlock=tok&date=2026-11-02');
assert.equal(weekUrl('mara', '2026-11-02', null), '/api/personas/mara/week?date=2026-11-02');

// A personas sub is a persona's name, so the Week tab takes no pane row:
// one would hand a persona named `week` the world's banner.
assert.equal(PANE_FEATURE['personas/week'], undefined);

// The cell pick goes to its own endpoint, with the routine as shown.
assert.ok(ENDPOINTS.includes('/api/personas/X/routine/cell'));
assert.equal(WEEK_CELLS.length, 6);
assert.deepEqual(cellBody('retired', { routine: { digest: 'abc' } }, '2026-11-02', 'tok'), { cell: 'retired', base: 'abc', date: '2026-11-02', unlock: 'tok' });
assert.deepEqual(cellBody('student', { routine: { digest: '' } }, null), { cell: 'student', base: '', date: undefined, unlock: undefined });
assert.throws(() => cellBody('astronaut', {}, null), /not a kind of week/);
// Every activity has a group; Rust holds the key set to Activity::ALL.
assert.equal(Object.keys(ACTIVITY_GROUP).length, 26);

// Each reason she has no days has its own words.
for (const why of ['off', 'no_routine', 'no_cell']) assert.ok(WEEK_ABSENT[why]);

// A day's stretches sit on a 4am-to-4am bar; past midnight is the next morning.
assert.equal(dayClock(25 * 60 + 5), '01:05');
const data = {
  week: {
    monday: '2026-11-02',
    days: [
      {
        date: '2026-11-02',
        when: 'today',
        sources: ['sampler', 'mystery'],
        day: {
          stretches: [
            { activity: 'sleep', start_minute: 240, end_minute: 600, company: null, anchor: null },
            { activity: 'exercise', start_minute: 600, end_minute: 1680, company: ['friend', 'colleague'], anchor: 'swim' },
          ],
        },
      },
      { date: '2026-10-26', when: 'folded', sources: [], day: null },
    ],
    folded: { days: 2, minutes: [{ activity: 'sleep', minutes: 960 }, { activity: 'personal_care', minutes: 1920 }], unreadable: 0 },
  },
};
const days = weekDays(data);
assert.equal(days.length, 2);
const [sleep, swim] = days[0].bars;
assert.equal(sleep.left, 0);
assert.equal(sleep.width, 25);
assert.equal(sleep.group, 'rest');
assert.equal(sleep.unclean, false);
assert.equal(swim.left + swim.width, 100, 'the day ends at the next 4am');
assert.equal(swim.unclean, true, 'an unknown source is never clean');
assert.equal(swim.line, '10:00–04:00+1 exercise (swim), with friend, colleague');
assert.equal(days[1].folded, true);
assert.equal(days[1].undrawn, false);
assert.equal(days[1].bars.length, 0);
assert.equal(days[0].undrawn, false);
// A day her routine could not draw is not a folded one, and the week says why.
const undrawn = weekDays({ week: { days: [{ date: '2026-11-04', when: 'today', sources: [], day: null }], unavailable: 'routine.toml does not load' } });
assert.equal(undrawn[0].folded, false);
assert.equal(undrawn[0].undrawn, true);
assert.equal(weekUnavailable({ week: { unavailable: 'routine.toml does not load' } }), 'routine.toml does not load');
assert.equal(weekUnavailable(data), null);
assert.equal(stretchLine({ activity: 'tv', start_minute: 1200, end_minute: 1260, company: ['alone'], anchor: 'tv' }), '20:00–21:00 tv, alone');
// After midnight, no marker: the stretch is wholly the next morning's.
assert.equal(stretchLine({ activity: 'sleep', start_minute: 1500, end_minute: 1680, company: null, anchor: null }), '01:00–04:00 sleep');
// An activity this bundle does not know is unknown, never a known group.
const odd = weekDays({ week: { days: [{ date: '2026-11-02', when: 'past', sources: ['sampler'], day: { stretches: [{ activity: 'juggling', start_minute: 240, end_minute: 1680, company: null, anchor: null }] } }] } });
assert.equal(odd[0].bars[0].group, 'unknown');

// Folded minutes, most first.
const f = foldedHours(data);
assert.deepEqual(f.rows, [{ activity: 'personal care', hours: '32h00' }, { activity: 'sleep', hours: '16h00' }]);
assert.equal(f.unreadable, null);
assert.equal(foldedHours({ week: { days: [], folded: null } }), null);

// A week away by whole days, across a month and a clock change.
assert.equal(shiftWeek('2026-10-26', 1), '2026-11-02');
assert.equal(shiftWeek('2026-11-02', -1), '2026-10-26');
assert.equal(shiftWeek('2026-12-28', 1), '2027-01-04');

console.log('persona-week: ok');
