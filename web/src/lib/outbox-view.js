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

/**
 * Whether a draft is shown as an event card and edited as event fields: a
 * create only. The other calendar calls are `event` for the list's filter and
 * icon, but an update carries only the fields it changes and a delete only an
 * id — a card would render them as a new event with no date ("add to
 * calendar", on a cancellation), and the editor would write a full start and
 * end into an update that had none, turning a location fix into a reschedule
 * that notifies everyone. Those two keep the generic view, every field shown.
 */
export const editsAsEvent = (tool) => toolSuffix(tool) === 'calendar_create_event';

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
 * The editor's fields → arguments, **writing only what the owner changed**.
 * Every field left as it was keeps its original bytes: the staged arguments
 * carry pinned schema defaults (`calendar_id: "primary"`, from
 * `with_schema_defaults`) and the model's own stamp spellings, and an
 * untouched Save that re-rendered them would make `args != args_before` —
 * an "edit" the writing miner would learn from, and the end of the goal
 * system's only sent-unchanged signal (found on review). Keys the form does
 * not show ride through untouched; a cleared optional field is removed
 * rather than sent blank.
 *
 * Returns `{ args }` or `{ error }` — an end before its start is refused here
 * rather than by the provider after the owner pressed Approve.
 */
export function eventArgs(original, f) {
  const base = inclusiveEnd(eventFields(original));
  const out = { ...original };
  const title = f.title.trim();
  if (!title) return { error: 'The event needs a title' };
  if (!DATE_ONLY.test(f.date)) return { error: 'Pick a date' };
  const endDate = DATE_ONLY.test(f.endDate) ? f.endDate : f.date;
  if (title !== base.title) out.title = title;

  const whenChanged = f.allDay !== base.allDay || f.date !== base.date || endDate !== base.endDate
    || (!f.allDay && (f.start !== base.start || f.end !== base.end || f.zone !== base.zone));
  if (f.allDay) {
    if (endDate < f.date) return { error: 'The event ends before it starts' };
    if (whenChanged) {
      out.all_day = true;
      out.start_time = f.date;
      // All-day ends are exclusive in both providers; a one-day event ends
      // the next day. The form shows the last day, inclusive.
      const next = new Date(`${endDate}T00:00:00Z`);
      next.setUTCDate(next.getUTCDate() + 1);
      out.end_time = next.toISOString().slice(0, 10);
    }
  } else {
    if (!/^\d{2}:\d{2}$/.test(f.start) || !/^\d{2}:\d{2}$/.test(f.end)) return { error: 'Pick a start and an end time' };
    const start = stampIn(f.zone, f.date, f.start);
    const end = stampIn(f.zone, endDate, f.end);
    if (Date.parse(end) <= Date.parse(start)) return { error: 'The event ends before it starts' };
    if (whenChanged) {
      if (f.allDay !== base.allDay || 'all_day' in original) out.all_day = false;
      out.start_time = start;
      out.end_time = end;
      // The stamps carry their offset; the zone is named only where it was
      // named before or has just changed.
      if ('timezone' in original || f.zone !== base.zone) out.timezone = f.zone;
    }
  }
  const text = (k, now, was) => {
    if (now === was) return;
    if (now) out[k] = now;
    else delete out[k];
  };
  text('location', f.location.trim(), base.location.trim());
  text('description', f.description.trim(), base.description.trim());
  const people = f.attendees.split(/[,;\s]+/).map((x) => x.trim()).filter(Boolean);
  if (people.join(',') !== attendeesOf(original).join(',')) {
    if (people.length) out.attendees = people;
    else delete out.attendees;
  }
  text('account', f.account.trim(), base.account.trim());
  if (f.calendar_id !== base.calendar_id) {
    if (f.calendar_id) out.calendar_id = f.calendar_id;
    else delete out.calendar_id;
  }
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
    const day = (iso) => new Date(`${iso}T12:00:00Z`).toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric', timeZone: 'UTC' });
    // The end is exclusive: the last day is the one before it. A span of
    // more than one day must say so — the card is the reviewable object.
    const e = (args?.end_time ?? '').slice(0, 10);
    if (DATE_ONLY.test(e)) {
      const last = new Date(`${e}T00:00:00Z`);
      last.setUTCDate(last.getUTCDate() - 1);
      const lastIso = last.toISOString().slice(0, 10);
      if (lastIso > s.slice(0, 10)) return `${day(s.slice(0, 10))} – ${day(lastIso)} · all day`;
    }
    return `${day(s.slice(0, 10))} · all day`;
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

/** The arguments the event card renders itself; any other is listed beside it. */
export const EVENT_CARD_KEYS = [
  'title', 'start_time', 'end_time', 'timezone', 'all_day', 'account',
  'location', 'attendees', 'description', 'calendar_id',
];

/** The accounts `calendar_list` could not read (rows carrying an `error`). */
export const unreadableAccounts = (calendars) =>
  Array.isArray(calendars) ? calendars.filter((a) => a?.error).map((a) => a.account) : [];

/**
 * The picker's warning for unreadable accounts, or null when there are none.
 * Missing, not empty: an unreadable account has calendars nobody could list.
 */
export function unreadableNote(names) {
  if (!names.length) return null;
  const list = names.length === 1 ? names[0] : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
  return names.length === 1
    ? `${list} could not be read — its calendars are missing from this list, not empty.`
    : `${list} could not be read — their calendars are missing from this list, not empty.`;
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

/**
 * The conversation a reply answers — `{ account, subject }` — from the thread
 * read the drafting run made, or null.
 *
 * A reply's arguments are an opaque thread id and a body, so the pane used
 * to title it "Reply" and never say which conversation, or which mailbox it
 * leaves from — the account is looked up from the thread at send time now,
 * so it is often not an argument at all. Only the **first** message's header
 * is read: mecha-mail writes it (`--- [account] From: …`) at the very start
 * of the text, while every later header follows a stranger's body, where a
 * line shaped like one can be typed by anybody.
 */
export function threadOf(sources) {
  const read = (sources ?? []).find((s) => toolSuffix(s.tool) === 'mail_get_thread');
  const lines = (read?.text ?? '').split('\n', 4);
  const head = /^--- \[([^\]]+)\] From: /.exec(lines[0] ?? '');
  if (!head) return null;
  const subject = lines.slice(1, 3).find((l) => l.startsWith('Subject: '));
  return { account: head[1], subject: subject ? subject.slice('Subject: '.length).trim() : '' };
}

/**
 * Arguments that are routing, not content: shown under "exact arguments",
 * never as a row of their own. A 150-character thread id is nothing a
 * reviewer can check by eye, and `reply_all: false` is the default saying
 * nothing — `reply_all: true` is shown, as a chip, because it widens who
 * gets the mail.
 */
export const ROUTING_KEYS = ['thread_id', 'message_id', 'reply_all'];

const MSG_HEAD = /^--- \[([^\]]+)\] From: (.*) <([^<>]*)> · (\S+)$/;

/**
 * The thread a drafting run read, as messages — `{ account, messages: [{
 * name, address, date, subject, replyId, body }] }`, oldest first — or null
 * when the text is not one `mail_get_thread` wrote.
 *
 * Presentation only, like `mail-thread.js`'s parse for the Mail tab, and
 * stricter than it: a message starts only at a line that is a whole header
 * mecha-mail writes — after a blank line, naming the thread's own account,
 * followed by its `Calendar date:` (or, in older reads, `Subject:`) line — so a signature's `---`, or an
 * "-----Original Message-----" block quoted in a body, never splits one. A
 * body that forges all of it can still split; the verbatim text stays one
 * click away, and nothing here decides where a reply goes.
 */
export function threadMessages(text) {
  const lines = (text ?? '').split('\n');
  const first = MSG_HEAD.exec(lines[0] ?? '');
  if (!first) return null;
  const account = first[1];
  const starts = [];
  lines.forEach((l, i) => {
    const m = MSG_HEAD.exec(l);
    // `Calendar date:` since mecha-mail started writing one; `Subject:`
    // straight after the header in the reads drafts staged before that hold.
    const nextLine = lines[i + 1] ?? '';
    const ours = nextLine.startsWith('Calendar date: ') || nextLine.startsWith('Subject: ');
    if (m && m[1] === account && (i === 0 || lines[i - 1] === '') && ours) starts.push(i);
  });
  const messages = starts.map((s, k) => {
    const [, , name, address, date] = MSG_HEAD.exec(lines[s]);
    const end = k + 1 < starts.length ? starts[k + 1] : lines.length;
    let i = s + 1;
    let subject = '';
    let replyId = null;
    for (; i < end && lines[i] !== ''; i++) {
      if (lines[i].startsWith('Subject: ')) subject = lines[i].slice('Subject: '.length).trim();
      else if (lines[i].startsWith('Message id (for mail_reply): ')) replyId = lines[i].slice('Message id (for mail_reply): '.length).trim();
    }
    return { name: name.trim(), address, date, subject, replyId, body: lines.slice(i, end).join('\n').trim() };
  });
  return { account, messages };
}

/**
 * The message a reply answers: the one `message_id` names, else the newest —
 * `mail_reply`'s own rule. Null when `message_id` names none the run read.
 */
export function answeredMessage(thread, args) {
  const msgs = thread?.messages ?? [];
  if (!msgs.length) return null;
  if (args?.message_id) return msgs.find((m) => m.replyId === args.message_id) ?? null;
  return msgs[msgs.length - 1];
}

/** "Tue, Sep 15, 12:56 PM", in the viewer's zone. */
export function msgWhen(iso) {
  const ms = Date.parse(iso ?? '');
  if (Number.isNaN(ms)) return iso ?? '';
  return new Date(ms).toLocaleString('en-US', { weekday: 'short', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
}

/**
 * Who a mail draft is to and what it is about, for a list row — `{ who,
 * subject }` — from a detail (`/api/outbox/{id}`), or null for anything
 * else. A staged reply carries no recipient and no subject (both follow from
 * its thread), so its row used to read "REPLY" over the first line of
 * prose, and a queue of those is a queue nobody can triage. `who` for a
 * reply is the sender of the message it answers — who the reply is
 * addressed back to by default.
 */
export function rowSummary(detail) {
  if (!detail) return null;
  const tool = toolSuffix(detail.tool);
  const args = detail.args ?? {};
  if (tool === 'mail_send') return { who: args.to ?? '', subject: args.subject ?? detail.headline ?? '' };
  if (tool !== 'mail_reply') return null;
  const read = (detail.sources ?? []).find((s) => toolSuffix(s.tool) === 'mail_get_thread');
  const thread = read ? threadMessages(read.text) : null;
  const answered = answeredMessage(thread, args);
  const first = thread?.messages[0]?.subject ?? '';
  const subject = detail.headline || (first ? (/^re:/i.test(first) ? first : `Re: ${first}`) : '');
  return { who: answered ? answered.name || answered.address : '', subject };
}

/**
 * A `docs_replace` as the edit it is — `{ find, replace, matchCase, url }` —
 * or null. `url` opens the document, and only for an id shaped like a Drive
 * id, so a model-written value cannot become a link anywhere but Google Docs.
 */
export function docEdit(tool, args) {
  if (toolSuffix(tool) !== 'docs_replace' || typeof args?.find !== 'string') return null;
  const id = typeof args.file_id === 'string' && /^[A-Za-z0-9_-]{20,}$/.test(args.file_id) ? args.file_id : null;
  return {
    find: args.find,
    replace: typeof args.replace === 'string' ? args.replace : '',
    matchCase: args.match_case === true,
    url: id ? `https://docs.google.com/document/d/${id}/edit` : null,
  };
}

/** The keys a doc-edit card renders itself. */
export const DOC_EDIT_KEYS = ['find', 'replace', 'match_case', 'file_id'];

/**
 * One-press reasons for a reject. A reason is still recorded — the learning
 * miner reads it — but most drafts in a clogged queue are refused for one
 * of these, and typing it was the step that left them sitting there.
 */
export const REJECT_REASONS = ['Already handled', 'No longer needed', "Not right — I'll write it myself"];
