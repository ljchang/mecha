// Which optional parts of mecha are on, as the page reads them — the pure
// half, kept out of the components so node can test it
// (web/test/features.mjs). `features.svelte.js` holds the one fetch.
//
// **The rule is the server's.** Each row of `/api/features` carries `shown`
// (Off and Blocked hide; Unready and Unknown never do — hiding something the
// owner switched on reads as "not configured") and `next` (the command that
// brings it on). Nothing here restates which states hide: it reads `shown`,
// so the page and `mecha features` cannot disagree (FEATURES-DESIGN.md §4.2).
//
// **Not answered is shown.** Before the answer arrives, when the route
// fails, and for an id this serve does not know, everything is shown: a page
// that hid tabs off a read it could not make would be the silently-degrading
// guard, and step 2 changes no route, so a shown tab still works.

/**
 * The view each nav place belongs to. A view not named is core — home,
 * chat, review and settings are there whatever is switched on.
 */
export const VIEW_FEATURE = {
  mail: 'mail',
  graph: 'graph',
  tasks: 'tasks',
  personas: 'personas',
  library: 'library',
};

/**
 * Panes of a core view that belong to a feature: a link to one whose feature
 * is hidden lands on Home, as a hidden view's does — its routes answer
 * `feature_off` — and the view does not offer it.
 */
export const PANE_FEATURE = {
  'review/graph': 'graph',
  'review/entities': 'graph',
  'review/frontdoor': 'frontdoor',
};

/** The feature `view/sub` belongs to: its pane's, else its view's, else none. */
export function featureOf(view, sub) {
  return PANE_FEATURE[`${view}/${sub}`] ?? VIEW_FEATURE[view] ?? null;
}

/**
 * Home's queue cards, by the backlog's wire name. The outbox and questions
 * are core stores and appear nowhere here.
 */
export const QUEUE_FEATURE = {
  'front-door requests': 'frontdoor',
  'graph candidates': 'graph',
  'graph shadow': 'graph',
  'graph entities': 'graph',
  'image candidates': 'library',
};

/**
 * Routes that open whatever their view's feature says. A core pane that
 * happens to live in a feature's view — a delegated run's question and the
 * workflows, both on the board's page but read from core stores — and the
 * landing of a queue card kept because something waits in it. Redirecting
 * these would keep the card and refuse the tap: the Questions card would
 * land on Home with its runs still waiting (found on review of #449). A
 * queue card kept for what waits in an off feature is flat instead (Home),
 * because its pane's routes answer `feature_off`.
 * `every_place_home_lands_opens_whatever_its_view_s_switch_says` holds
 * Home's destinations to this list, and a view these admit offers only the
 * sub-views that open (`opens`), or its own controls would bounce the same
 * way one level in.
 */
export const OPENS_ANYWAY = ['tasks/waiting', 'tasks/workflows'];

/** Whether `view/sub` opens even with its view's feature hidden. */
export function opensAnyway(view, sub) {
  return sub != null && OPENS_ANYWAY.includes(`${view}/${sub}`);
}

/**
 * Whether `view/sub` would open rather than bounce to Home: its view's
 * feature is shown, or the route opens anyway. What a view's own controls
 * ask before offering a way to another of its sub-views.
 */
export function opens(rows, view, sub) {
  return isShown(rows, VIEW_FEATURE[view]) || opensAnyway(view, sub);
}

/** `/api/features`'s body as a map from id to row, or null when unanswered. */
export function index(body) {
  const rows = body?.features;
  if (!Array.isArray(rows)) return null;
  return new Map(rows.map((r) => [r.id, r]));
}

/** Whether `id` is shown. `id` null is core, always shown. */
export function isShown(rows, id) {
  if (!id || !rows) return true;
  const row = rows.get(id);
  return row ? row.shown !== false : true;
}

/**
 * A queue card is shown when its feature is — or when it has something
 * waiting, whatever the switch says: the front door's drain keeps filling
 * its store with the bool off, and a count of what waits never degrades
 * (FEATURES-DESIGN.md §5). An unreadable count (`null`) is not a zero.
 */
export function queueCardShown(rows, queue, depth) {
  return isShown(rows, QUEUE_FEATURE[queue]) || depth !== 0;
}

/** Features in the order they are listed, and each one's ancestry. */
function chain(rows, id) {
  const out = [];
  const seen = new Set();
  const walk = (f) => {
    if (!f || seen.has(f)) return;
    seen.add(f);
    const row = rows.get(f);
    if (!row) return;
    walk(row.part_of);
    for (const r of row.requires ?? []) walk(r);
    out.push(row);
  };
  walk(id);
  return out;
}

/**
 * The banner a view carries: the first feature it stands on — its parents
 * and requirements, then itself — that is switched on and not working yet
 * (`unready`) or could not be read (`unknown`). `null` when there is
 * nothing to say. `{ label, word, reason, next }`.
 */
export function banner(rows, id) {
  if (!id || !rows) return null;
  for (const row of chain(rows, id)) {
    if (row.state === 'unready' || row.state === 'unknown') {
      return { label: row.label, word: row.state, reason: row.reason ?? null, next: row.next ?? null };
    }
  }
  return null;
}

/** The one line a direct link to a hidden view lands on Home with. */
export function hiddenLine(rows, id) {
  const row = rows?.get(id);
  if (!row) return null;
  const what = row.state === 'blocked' ? `needs ${rows.get(row.on)?.label ?? row.on}` : 'is off';
  return { text: `${row.label} ${what}`, next: row.next ?? null };
}

/** Settings' index row: "12 on · 3 off", with anything wrong said first. */
export function summary(body) {
  const rows = body?.features;
  if (!Array.isArray(rows)) return null;
  const count = (w) => rows.filter((r) => r.state === w).length;
  const on = count('on');
  const off = count('off') + count('blocked');
  const bits = [];
  const unready = count('unready');
  const unknown = count('unknown');
  if (unready) bits.push(`${unready} not ready`);
  if (unknown) bits.push(`${unknown} unreadable`);
  bits.push(`${on} on`);
  if (off) bits.push(`${off} off`);
  const pending = rows.filter((r) => r.pending).length;
  if (pending) bits.push(`${pending} waiting on a restart`);
  return { text: bits.join(' · '), bad: unready + unknown > 0 };
}

/**
 * Settings → Features: every feature, parts under their parent, indented
 * by depth. Rows arrive parent-first (`Feature::ALL`), so a part's parent
 * has always been placed before it.
 */
export function tree(body) {
  const rows = body?.features;
  if (!Array.isArray(rows)) return [];
  const depth = new Map();
  return rows.map((r) => {
    const d = r.part_of ? (depth.get(r.part_of) ?? 0) + 1 : 0;
    depth.set(r.id, d);
    return { ...r, depth: d };
  });
}

/** What a row says beside its state word. */
export function detail(row, rows) {
  switch (row.state) {
    case 'on':
      return row.detail ?? '';
    case 'blocked':
      return `needs ${rows?.get(row.on)?.label ?? row.on}`;
    default:
      return row.reason ?? '';
  }
}
