// The Factory view's pure parts, testable without a browser
// (test/factory.mjs).

/**
 * Slots grouped by local day for the preview: `[{label, starts: [{t, ds}]}]`,
 * the first `days` days only, rendered in `tz` — the page's zone, because
 * that is the week the owner is describing. `ds` is every length offered at
 * that start, ascending.
 */
export function byDay(slots, tz, { days = 14, now = Date.now(), locale = [] } = {}) {
  if (!Array.isArray(slots) || !tz) return [];
  const day = new Intl.DateTimeFormat(locale, { timeZone: tz, weekday: 'short', month: 'short', day: 'numeric' });
  const time = new Intl.DateTimeFormat(locale, { timeZone: tz, hour: 'numeric', minute: '2-digit' });
  const until = now + days * 86400000;
  const groups = new Map();
  for (const s of slots) {
    const at = new Date(s.start);
    if (isNaN(at)) continue;
    // Slots arrive sorted by start, so the first past the edge ends it.
    if (at.getTime() > until) break;
    const key = day.format(at);
    if (!groups.has(key)) groups.set(key, new Map());
    const starts = groups.get(key);
    const t = time.format(at);
    starts.set(t, [...(starts.get(t) ?? []), s.duration_minutes]);
  }
  return [...groups].map(([label, starts]) => ({
    label,
    starts: [...starts].map(([t, ds]) => ({ t, ds: [...ds].sort((a, b) => a - b) })),
  }));
}

/**
 * The window "+" adds to a day: from the end of the day's last window, an
 * hour long or to the end of the day, whichever is sooner; a working
 * afternoon on a day with none. `null` once the day is open until 23:59 —
 * there is nothing after it to add, and a window starting inside the last
 * one is two windows saying one thing.
 */
export function nextWindow(existing) {
  const last = [...existing].sort((a, b) => a.end.localeCompare(b.end)).at(-1);
  if (!last) return { start: '13:00', end: '17:00' };
  if (last.end >= '23:59') return null;
  const [h, m] = last.end.split(':').map(Number);
  const end = h >= 23 ? '23:59' : `${String(h + 1).padStart(2, '0')}:${String(m).padStart(2, '0')}`;
  return { start: last.end, end };
}

/**
 * A field or option name from its label: lowercase, digits and `_`,
 * starting with a letter — the factory's rule for a field name — and not
 * one of `taken`. A label with nothing usable becomes `field`.
 */
export function nameFrom(label, taken = []) {
  let base = String(label ?? '')
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .slice(0, 40);
  if (!/^[a-z]/.test(base)) base = base ? `f_${base}` : 'field';
  const used = new Set(taken);
  if (!used.has(base)) return base;
  for (let i = 2; ; i++) if (!used.has(`${base}_${i}`)) return `${base}_${i}`;
}

const DAY_WORDS = { mon: 'Monday', tue: 'Tuesday', wed: 'Wednesday', thu: 'Thursday', fri: 'Friday', sat: 'Saturday', sun: 'Sunday' };

/** "21:30" → "9:30 PM": the clock the time inputs show, not the file's. */
export function clock12(hhmm) {
  const [h, m] = String(hhmm).split(':').map(Number);
  if (!Number.isFinite(h) || !Number.isFinite(m)) return String(hhmm);
  const suffix = h < 12 ? 'AM' : 'PM';
  const h12 = h % 12 === 0 ? 12 : h % 12;
  return `${h12}:${String(m).padStart(2, '0')} ${suffix}`;
}

/**
 * The first window that ends before it starts, said in the page's own
 * words — the inputs show a 12-hour clock, so the likely mistake is an AM
 * typed as PM, and the sentence says so. `null` when every window runs
 * forwards. Checked on the page so the owner reads this, not the server's
 * 24-hour refusal.
 */
export function backwards(hours) {
  for (const w of hours?.windows ?? []) {
    if (w.start >= w.end) {
      return `${DAY_WORDS[w.day] ?? w.day} starts at ${clock12(w.start)}, after it ends at ${clock12(w.end)} — check AM and PM.`;
    }
  }
  for (const o of hours?.overrides ?? []) {
    for (const [s, e] of o.windows ?? []) {
      if (s >= e) return `${o.date} starts at ${clock12(s)}, after it ends at ${clock12(e)} — check AM and PM.`;
    }
  }
  return null;
}

/**
 * Today's date in `tz` as YYYY-MM-DD — the zone the page's dates are in.
 * `toISOString()` is UTC, which west of UTC turns into tomorrow every
 * evening and refused today as a date off.
 */
export function todayIn(tz, now = new Date()) {
  try {
    return new Intl.DateTimeFormat('en-CA', { timeZone: tz, year: 'numeric', month: '2-digit', day: '2-digit' }).format(now);
  } catch {
    return now.toISOString().slice(0, 10);
  }
}
