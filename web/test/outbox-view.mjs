// Behaviour checks for the outbox page's logic (src/lib/outbox-view.js).
//
// **What is worth pinning.** The event editor's round trip is where being
// wrong is silent: an edited time that lands an hour off because the offset
// was today's rather than the event's date's, an all-day end one day short,
// a key the form does not show dropped on save, an empty attendee field sent
// as `[""]`. Each of those looks fine in the form and is wrong on the
// calendar.
import { kindOf, stampIn, wallIn, eventFields, eventArgs, inclusiveEnd, whenLabel, attendeesOf, editsAsEvent } from '../src/lib/outbox-view.js';

let pass = 0;
let fail = 0;
const t = (name, cond) => {
  if (cond) {
    pass += 1;
    console.log('  ok   ', name);
  } else {
    fail += 1;
    console.log('  FAIL ', name);
  }
};

// ---- kinds ----
t('a reply is mail', kindOf('mail__mail_reply') === 'mail');
t('a new mail is mail', kindOf('mail__mail_send') === 'mail');
t('an event is an event', kindOf('mail__calendar_create_event') === 'event');
t('a doc edit is a doc', kindOf('docs__docs_replace') === 'doc');
t('a meeting poll is a poll', kindOf('factory__poll_meeting_create') === 'poll');
t('anything else is other', kindOf('web__fetch') === 'other');
t('create and update edit as events', editsAsEvent('mail__calendar_create_event') && editsAsEvent('mail__calendar_update_event'));
t('a delete does not', !editsAsEvent('mail__calendar_delete_event'));

// ---- offsets follow the date, not today ----
const NY = 'America/New_York';
t('September in New York is EDT', stampIn(NY, '2026-09-30', '15:30') === '2026-09-30T15:30:00-04:00');
t('December in New York is EST', stampIn(NY, '2026-12-02', '15:30') === '2026-12-02T15:30:00-05:00');
t('the morning after fall-back is EST', stampIn(NY, '2026-11-01', '09:00') === '2026-11-01T09:00:00-05:00');
t('UTC is +00:00', stampIn('UTC', '2026-09-30', '08:05') === '2026-09-30T08:05:00+00:00');
t('east of UTC is positive', stampIn('Asia/Kolkata', '2026-09-30', '10:00') === '2026-09-30T10:00:00+05:30');
{
  const w = wallIn(NY, '2026-09-30T19:30:00Z');
  t('a UTC stamp reads as New York wall time', w.date === '2026-09-30' && w.time === '15:30');
}

// ---- the round trip ----
const staged = {
  title: 'PBS Faculty Meeting',
  start_time: '2026-09-30T15:30:00-04:00',
  end_time: '2026-09-30T17:00:00-04:00',
  account: 'dartmouth',
  attendees: ['organiser@example.edu'],
  calendar_id: 'primary',
  location: 'Moore Hall, Library Room 402',
  timezone: NY,
  send_updates: 'none',
};
{
  const f = eventFields(staged);
  t('fields read the wall time in the event zone', f.date === '2026-09-30' && f.start === '15:30' && f.end === '17:00');
  t('attendees read as a list', f.attendees === 'organiser@example.edu');
  const { args } = eventArgs(staged, f);
  t('an untouched form round-trips the times', args.start_time === staged.start_time && args.end_time === staged.end_time);
  t('a key the form does not show survives', args.send_updates === 'none');
  t('primary is the default, not a sent value', !('calendar_id' in args));

  const moved = eventArgs(staged, { ...f, date: '2026-12-02', endDate: '2026-12-02' }).args;
  t('moving into winter takes winter\'s offset', moved.start_time === '2026-12-02T15:30:00-05:00');

  const noone = eventArgs(staged, { ...f, attendees: '  ' }).args;
  t('clearing attendees removes the key', !('attendees' in noone));
  const two = eventArgs(staged, { ...f, attendees: 'a@example.edu, b@example.edu' }).args;
  t('two attendees are two', two.attendees.length === 2);

  const other = eventArgs(staged, { ...f, calendar_id: 'lab@group.calendar.google.com', account: 'personal' }).args;
  t('a chosen calendar is sent', other.calendar_id === 'lab@group.calendar.google.com' && other.account === 'personal');

  t('an end before the start is refused', !!eventArgs(staged, { ...f, end: '14:00' }).error);
  t('an empty title is refused', !!eventArgs(staged, { ...f, title: ' ' }).error);
  t('a cleared location is dropped, not blank', !('location' in eventArgs(staged, { ...f, location: '' }).args));
}

// ---- all-day ----
{
  const allDay = { title: 'Retreat', start_time: '2026-10-05', end_time: '2026-10-07', all_day: true };
  const f = inclusiveEnd(eventFields(allDay));
  t('an all-day event reads its last day inclusively', f.allDay && f.date === '2026-10-05' && f.endDate === '2026-10-06');
  const { args } = eventArgs(allDay, f);
  t('and writes the exclusive end back', args.start_time === '2026-10-05' && args.end_time === '2026-10-07');
  const one = eventArgs(allDay, { ...f, endDate: '2026-10-05' }).args;
  t('a one-day event ends the next day', one.end_time === '2026-10-06');
}

// ---- display ----
t('whenLabel names the day and both times', /Wed, Sep 30 · 3:30\sPM – 5:00\sPM\sEDT/.test(whenLabel(staged, NY)));
t('whenLabel says all day', whenLabel({ start_time: '2026-10-05', all_day: true }).endsWith('all day'));
t('attendees accept a comma string', attendeesOf({ attendees: 'a@x.edu, b@x.edu' }).length === 2);
t('attendees accept objects', attendeesOf({ attendees: [{ email: 'a@x.edu' }] })[0] === 'a@x.edu');

console.log(`\n${pass} passed, ${fail} failed`);
if (fail) process.exit(1);
