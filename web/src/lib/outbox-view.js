// The outbox page's logic, kept out of Outbox.svelte so node can test it
// (web/test/outbox-view.mjs).
//
// Two jobs. **What kind of draft is this** — a mail, a calendar event, a doc
// edit — read from the tool's registry name, so the page can show each as
// the thing it is: an email as an email, an event as a time and a place.
// And **the event editor's round trip**: arguments to form fields and back,
// without losing a key the form does not know about and without moving the
// event by an hour when the date it lands on is on the other side of a DST
// change.

/** The registry name's last segment: `mail__mail_reply` → `mail_reply`. */
export const toolSuffix = (tool) => (tool ?? '').split('__').pop();

/** `mail` | `event` | `doc` | `poll` | `other`. */
export function kindOf(tool) {
  const t = toolSuffix(tool);
  if (t === 'mail_reply' || t === 'mail_send' || t === 'mail_forward') return 'mail';
  if (t.startsWith('calendar_')) return 'event';
  if (t.startsWith('docs_') || t.startsWith('sheets_') || t.startsWith('slides_')) return 'doc';
  if (t.startsWith('poll_')) return 'poll';
  return 'other';
}

export const KINDS = [
  { id: 'mail', label: 'Mail' },
  { id: 'event', label: 'Calendar' },
  { id: 'doc', label: 'Docs' },
  { id: 'poll', label: 'Polls' },
  { id: 'other', label: 'Other' },
];

/** Whether the event editor can edit this tool's arguments. */
export const editsAsEvent = (tool) => {
  const t = toolSuffix(tool);
  return t === 'calendar_create_event' || t === 'calendar_update_event';
};

/** The browser's IANA zone, else UTC. */
export function localZone() {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}

/** Whether `zone` is a zone Intl knows. The tool's `timezone` is free text. */
export function isZone(zone) {
  if (!zone) return false;
  try {
    new Intl.DateTimeFormat('en-US', { timeZone: zone });
    return true;
  } catch {
    return false;
  }
}

/** Minutes east of UTC that `zone` is at the instant `ms`. */
function offsetMinutesAt(zone, ms) {
  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone: zone,
    hourCycle: 'h23',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  }).formatToParts(new Date(ms));
  const get = (t) => Number(parts.find((p) => p.type === t)?.value);
  const wall = Date.UTC(get('year'), get('month') - 1, get('day'), get('hour') % 24, get('minute'), get('second'));
  return Math.round((wall - Math.floor(ms / 1000) * 1000) / 60000);
}

const pad = (n) => String(Math.abs(n)).padStart(2, '0');
const fmtOffset = (mins) => `${mins < 0 ? '-' : '+'}${pad(Math.trunc(mins / 60))}:${pad(mins % 60)}`;

/**
 * The RFC 3339 stamp for a wall-clock `date` (YYYY-MM-DD) and `time` (HH:MM)
 * in `zone` — the offset that zone has *on that date*, not today's. Two
 * passes, because the offset depends on the instant and the instant on the
 * offset; the second pass settles it everywhere but inside a DST gap, where
 * the wall time does not exist and the later offset is as good as any.
 */
export function stampIn(zone, date, time) {
  const [y, mo, d] = date.split('-').map(Number);
  const [h, mi] = time.split(':').map(Number);
  const wall = Date.UTC(y, mo - 1, d, h, mi);
  let off = offsetMinutesAt(zone, wall);
  off = offsetMinutesAt(zone, wall - off * 60000);
  return `${date}T${pad(h)}:${pad(mi)}:00${fmtOffset(off)}`;
}

/** `{date, time}` of an RFC 3339 stamp, as the wall clock reads in `zone`. */
export function wallIn(zone, stamp) {
  const ms = Date.parse(stamp ?? '');
  if (Number.isNaN(ms)) return null;
  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone: zone,
    hourCycle: 'h23',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).formatToParts(new Date(ms));
  const get = (t) => parts.find((p) => p.type === t)?.value;
  return { date: `${get('year')}-${get('month')}-${get('day')}`, time: `${get('hour') === '24' ? '00' : get('hour')}:${get('minute')}` };
}

const DATE_ONLY = /^\d{4}-\d{2}-\d{2}$/;

/** The zone an event is edited and shown in: its own when it names a real one. */
export const eventZone = (args) => (isZone(args?.timezone) ? args.timezone : localZone());

/** Attendees, whatever shape the model gave them in. */
export function attendeesOf(args) {
  const a = args?.attendees;
  if (Array.isArray(a)) return a.map((x) => String(typeof x === 'object' && x ? x.email ?? '' : x).trim()).filter(Boolean);
  if (typeof a === 'string') return a.split(/[,;\s]+/).map((x) => x.trim()).filter(Boolean);
  return [];
}

/** Arguments → the editor's fields. */
export function eventFields(args) {
  const zone = eventZone(args);
  const allDay = !!args?.all_day || DATE_ONLY.test(args?.start_time ?? '');
  const start = allDay ? { date: (args?.start_time ?? '').slice(0, 10), time: '' } : wallIn(zone, args?.start_time) ?? { date: '', time: '' };
  const end = allDay ? { date: (args?.end_time ?? '').slice(0, 10), time: '' } : wallIn(zone, args?.end_time) ?? { date: '', time: '' };
  return {
    title: args?.title ?? '',
    allDay,
    date: start.date,
    start: start.time,
    endDate: end.date,
    end: end.time,
    location: args?.location ?? '',
    description: args?.description ?? '',
    attendees: attendeesOf(args).join(', '),
    account: args?.account ?? '',
    calendar_id: args?.calendar_id ?? 'primary',
    zone,
  };
}

/**
 * The editor's fields → arguments. Keys the form does not show ride through
 * untouched; empty optional fields are dropped rather than sent blank; an
 * empty attendee list is no `attendees` at all.
 *
 * Returns `{ args }` or `{ error }` — an end before its start is refused here
 * rather than by the provider after the owner pressed Approve.
 */
export function eventArgs(original, f) {
  const out = { ...original };
  const title = f.title.trim();
  if (!title) return { error: 'The event needs a title' };
  if (!DATE_ONLY.test(f.date)) return { error: 'Pick a date' };
  const endDate = DATE_ONLY.test(f.endDate) ? f.endDate : f.date;
  out.title = title;
  if (f.allDay) {
    if (endDate < f.date) return { error: 'The event ends before it starts' };
    out.all_day = true;
    out.start_time = f.date;
    // All-day ends are exclusive in both providers; a one-day event ends the
    // next day. The form shows the last day, inclusive.
    const next = new Date(`${endDate}T00:00:00Z`);
    next.setUTCDate(next.getUTCDate() + 1);
    out.end_time = next.toISOString().slice(0, 10);
  } else {
    if (!/^\d{2}:\d{2}$/.test(f.start) || !/^\d{2}:\d{2}$/.test(f.end)) return { error: 'Pick a start and an end time' };
    const start = stampIn(f.zone, f.date, f.start);
    const end = stampIn(f.zone, endDate, f.end);
    if (Date.parse(end) <= Date.parse(start)) return { error: 'The event ends before it starts' };
    out.all_day = false;
    out.start_time = start;
    out.end_time = end;
    out.timezone = f.zone;
  }
  const set = (k, v) => {
    if (v) out[k] = v;
    else delete out[k];
  };
  set('location', f.location.trim());
  set('description', f.description.trim());
  const people = f.attendees.split(/[,;\s]+/).map((x) => x.trim()).filter(Boolean);
  if (people.length) out.attendees = people;
  else delete out.attendees;
  set('account', f.account.trim());
  set('calendar_id', f.calendar_id && f.calendar_id !== 'primary' ? f.calendar_id : '');
  return { args: out };
}

/** The editor's all-day end is inclusive: undo the exclusive end on the way in. */
export function inclusiveEnd(fields) {
  if (!fields.allDay || !DATE_ONLY.test(fields.endDate)) return fields;
  const d = new Date(`${fields.endDate}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() - 1);
  const last = d.toISOString().slice(0, 10);
  return { ...fields, endDate: last < fields.date ? fields.date : last };
}

/** "Wed, Sep 30 · 3:30 – 5:00 PM EDT", in `zone`. */
export function whenLabel(args, zone = eventZone(args)) {
  const s = args?.start_time;
  if (!s) return '';
  if (args?.all_day || DATE_ONLY.test(s)) {
    const d = new Date(`${s.slice(0, 10)}T12:00:00Z`);
    return `${d.toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric', timeZone: 'UTC' })} · all day`;
  }
  const a = Date.parse(s);
  const b = Date.parse(args?.end_time ?? '');
  if (Number.isNaN(a)) return s;
  const day = new Date(a).toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric', timeZone: zone });
  const t = (ms, withZone) => new Date(ms).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit', timeZone: zone, ...(withZone ? { timeZoneName: 'short' } : {}) });
  if (Number.isNaN(b)) return `${day} · ${t(a, true)}`;
  const sameDay = new Date(b).toLocaleDateString('en-US', { timeZone: zone }) === new Date(a).toLocaleDateString('en-US', { timeZone: zone });
  return sameDay ? `${day} · ${t(a, false)} – ${t(b, true)}` : `${day} ${t(a, false)} → ${new Date(b).toLocaleDateString('en-US', { month: 'short', day: 'numeric', timeZone: zone })} ${t(b, true)}`;
}

/** The mail headers worth a row each, in reading order. */
export const MAIL_HEADERS = ['to', 'cc', 'bcc', 'subject', 'account'];

/** "3m ago", "2h ago", "5d ago". */
export function ago(iso, now = Date.now()) {
  const ms = now - Date.parse(iso ?? '');
  if (Number.isNaN(ms)) return '';
  const m = Math.max(0, Math.floor(ms / 60000));
  if (m < 1) return 'just now';
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}
