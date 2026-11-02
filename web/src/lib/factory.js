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
 * The window "+" adds to a day: an hour after the day's last window ends,
 * or a working afternoon on a day with none. Never one that ends before it
 * starts — a window that late gets the rest of the day instead.
 */
export function nextWindow(existing) {
  const last = [...existing].sort((a, b) => a.end.localeCompare(b.end)).at(-1);
  if (!last) return { start: '13:00', end: '17:00' };
  const [h, m] = last.end.split(':').map(Number);
  if (!(h < 23)) return { start: '23:00', end: '23:45' };
  const start = last.end;
  const end = h + 1 >= 24 ? '23:45' : `${String(h + 1).padStart(2, '0')}:${String(m).padStart(2, '0')}`;
  return { start, end };
}
