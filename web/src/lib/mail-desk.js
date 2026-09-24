// The desktop mail triage's logic, kept out of MailDesk.svelte so node can
// test it without a component rig (web/test/mail-desk.mjs).
//
// Three ideas carry the desk, and each lives here rather than in markup:
//
// - **Lanes are the store's states, not a second classifier.** A row's lane is
//   read off `state` and `bucket` the way `Record::needs_me` and the TUI read
//   them. It is a re-derivation in JS, so it can drift; an unknown state
//   lands in "Needs you" rather than nowhere, so drift shows.
// - **The classifier's proposal is the default action.** `acceptVerb` maps a
//   proposal to the verb Enter runs, and says so when there is nothing to
//   accept rather than inventing one.
// - **Undo is a delay, not an inverse.** The CLI has no unarchive or
//   undismiss, so the page holds each action for a few seconds before it
//   reaches the server; `z` inside that window cancels it and nothing ever
//   happened. Past the window the action is real, and the page says so
//   instead of pretending it can take it back.

/** The lanes, in display order. `key` is the digit that jumps to it. */
export const LANES = [
  { id: 'respond', label: 'Needs you', key: '1' },
  { id: 'notify', label: 'FYI', key: '2' },
  { id: 'parked', label: 'Parked', key: '3' },
  { id: 'drafted', label: 'In the outbox', key: '4' },
];

/**
 * Which lane a store row belongs to, or null when it is finished (`acted`,
 * `dismissed`) or the classifier said ignore. `failed` is the owner's
 * problem by definition, so it sits with the threads that need them — and so
 * does any state this file does not know: a state added to `mail_triage.rs`
 * later must show up as a puzzling row, never vanish from the backlog.
 */
export function laneOf(row) {
  switch (row.state) {
    case 'failed':
      return 'respond';
    case 'parked':
      return 'parked';
    case 'drafted':
      return 'drafted';
    case 'classified':
      // `ignore` is the classifier saying so; an unknown bucket is drift,
      // and lands with the owner for the same reason an unknown state does.
      if (row.bucket === 'ignore') return null;
      if (row.bucket === 'notify') return 'notify';
      return 'respond';
    case 'acted':
    case 'dismissed':
      return null;
    default:
      return 'respond';
  }
}

const URGENCY_RANK = { now: 0, today: 1, week: 2 };

/** Urgency first, then newest first — the order a person works in. */
export function sortRows(rows) {
  return [...rows].sort((a, b) => {
    const ua = URGENCY_RANK[a.urgency] ?? 3;
    const ub = URGENCY_RANK[b.urgency] ?? 3;
    if (ua !== ub) return ua - ub;
    return (b.date ?? '').localeCompare(a.date ?? '');
  });
}

/**
 * The verb Enter runs for a row: its proposal, mapped onto the verbs
 * `/api/mail/act` accepts. `none` (and an unclassified row) has nothing to
 * accept, and `forward` needs a recipient the proposal does not carry, so
 * both return null and the page asks instead of guessing.
 */
export function acceptVerb(row) {
  switch (row.proposed) {
    case 'reply':
    case 'archive':
    case 'task':
    case 'schedule':
    case 'spam':
      return row.proposed;
    default:
      return null;
  }
}

/** A row's stable identity: thread ids are only unique within an account. */
export const keyOf = (row) => `${row.account}\u0000${row.thread_id}`;

/** Display name for a sender: the name when there is one, else the address. */
export const senderOf = (row) => (row.from_name ?? '').trim() || row.from || 'unknown sender';

/**
 * Sweep groups: the open rows whose proposal is `verb`, grouped by sender
 * address and largest group first. The address is the key and the name is
 * display, because the name is attacker-chosen and two senders can share one.
 */
export function sweepGroups(rows, verb) {
  const groups = new Map();
  for (const row of rows) {
    if (row.proposed !== verb) continue;
    const k = row.from || '(no address)';
    if (!groups.has(k)) groups.set(k, { key: k, name: senderOf(row), rows: [] });
    groups.get(k).rows.push(row);
  }
  return [...groups.values()].sort(
    (a, b) => b.rows.length - a.rows.length || a.name.localeCompare(b.name),
  );
}

/**
 * Verbs whose every thread is a whole agent run: they answer at once and run
 * detached, so the commit bound does not bound them.
 */
export const DRAFTING = new Set(['reply', 'schedule', 'forward']);

/**
 * The sweep groups that are ticked. `marks` holds the groups the owner has
 * toggled; `seen` the groups that were on screen when the sweep opened (or
 * its verb changed). What a mark means depends on the verb:
 *
 * - archive and task sweeps are the bulk the sweep exists for, so a group
 *   that was on screen is in unless marked out. One that appeared later (a
 *   new sender, on the minute's reload) is out unless marked in: the owner
 *   never read it, and an archive has no inverse once its hold ends.
 * - drafting sweeps start empty and a group is in only if marked in. Each
 *   ticked thread is an agent run the owner must have chosen.
 *
 * With no `seen`, every group counts as having been on screen.
 */
export function tickedGroups(groups, verb, marks, seen = null) {
  const optIn = DRAFTING.has(verb);
  return groups.filter((g) => {
    if (optIn) return marks.has(g.key);
    const shown = !seen || seen.has(g.key);
    return shown !== marks.has(g.key);
  });
}

/**
 * `mail recent` gives a sender as `Name <address>`; the store's rows keep the
 * two apart. Split one into `{ name, address }` so both read the same.
 */
export function splitSender(from) {
  const m = /^\s*"?([^"<]*?)"?\s*<([^>]+)>\s*$/.exec(from ?? '');
  return m ? { name: m[1].trim(), address: m[2].trim() } : { name: '', address: (from ?? '').trim() };
}

/**
 * Verbs the CLI will run on a thread the triage store has never seen — the
 * plain inbox's rows, mostly. `archive` and `spam` resolve a thread leniently
 * (`mail.rs`, `triage`: "the store is a lookup table here, not a
 * precondition"); `dismiss`, `task`, `needs-info` and the drafting verbs use
 * the strict resolver and refuse one.
 */
export const LENIENT = new Set(['archive', 'spam']);

/**
 * The verb a batch key stands for when a modifier is still held, or null.
 *
 * A selection is built with a modifier — ⇧ for a range, ⌘/Ctrl to toggle —
 * and the hand is often still on it when the action key goes down. ⇧E, ⇧D and
 * ⇧T are bound to nothing else, so they always mean e, d and t. ⌘/Ctrl-E and
 * -D mean archive and dismiss only while more than one thread is selected:
 * elsewhere they stay the browser's (find-selection, bookmark). ⌘T is not
 * offered — browsers keep it for a new tab, and a page never sees it.
 */
export function batchKeyVerb(key, { shift = false, meta = false, ctrl = false, alt = false } = {}, selectedCount = 0) {
  if (alt || typeof key !== 'string' || key.length !== 1) return null;
  const k = key.toLowerCase();
  if (meta || ctrl) {
    if (shift || selectedCount < 2) return null;
    return { e: 'archive', d: 'dismiss' }[k] ?? null;
  }
  if (shift && key !== k) return { e: 'archive', d: 'dismiss', t: 'task' }[k] ?? null;
  return null;
}

/** Whether `verb` can act on `row`, given the keys the store holds. */
export const verbWorksOn = (verb, row, storeKeys) => LENIENT.has(verb) || storeKeys.has(keyOf(row));

/** "3m", "2h", "5d", "6w" — compact enough for a list column. */
export function ageOf(date, now = Date.now()) {
  const t = Date.parse(date ?? '');
  if (Number.isNaN(t)) return '';
  const mins = Math.max(0, Math.round((now - t) / 60000));
  if (mins < 60) return `${mins}m`;
  const hours = Math.round(mins / 60);
  if (hours < 48) return `${hours}h`;
  const days = Math.round(hours / 24);
  if (days < 14) return `${days}d`;
  return `${Math.round(days / 7)}w`;
}

/**
 * Actions held for `delayMs` before they are committed, so the newest can be
 * taken back. One entry is one keystroke — a single archive or a whole sweep —
 * so one `z` undoes exactly what one key did.
 *
 * `commit(item)` sends one `{ row, verb, extra }` and resolves or throws.
 * `onChange()` fires whenever the pending set or a failure changes, and
 * `onFail(item, error)` reports an item the server refused, so the page can
 * put the row back with the reason.
 *
 * Batches commit with bounded concurrency: every verb is a CLI child that
 * starts the mail provider, and a sweep of a hundred must not start a hundred.
 */
export class PendingActions {
  constructor({ commit, onChange = () => {}, onFail = () => {}, delayMs = 5000, concurrency = 3, timers = globalThis }) {
    this.commitOne = commit;
    this.onChange = onChange;
    this.onFail = onFail;
    this.delayMs = delayMs;
    this.concurrency = concurrency;
    this.timers = timers;
    this.entries = []; // { id, items, timer, label }
    this.flight = new Set(); // items whose commit is outstanding
    this.queue = []; // items waiting for a commit slot
    this.nextId = 1;
  }

  /** Keys of every row with an action held or in flight. */
  hiddenKeys() {
    const keys = new Set();
    for (const e of this.entries) for (const it of e.items) keys.add(keyOf(it.row));
    for (const it of this.queue) keys.add(keyOf(it.row));
    // In flight too: a commit is a CLI child that takes seconds, and a row
    // that reappeared meanwhile could be acted on twice.
    for (const it of this.flight) keys.add(keyOf(it.row));
    return keys;
  }

  get pendingCount() {
    return this.entries.reduce((n, e) => n + e.items.length, 0);
  }

  get committingCount() {
    return this.queue.length + this.flight.size;
  }

  /** Hold `items` as one undoable entry; returns its id. */
  add(items, label) {
    const id = this.nextId++;
    const timer = this.timers.setTimeout(() => this.release(id), this.delayMs);
    this.entries.push({ id, items, timer, label });
    this.onChange();
    return id;
  }

  /** Take back the newest held entry. Returns it, or null when none is held. */
  undo() {
    const entry = this.entries.pop();
    if (!entry) return null;
    this.timers.clearTimeout(entry.timer);
    this.onChange();
    return entry;
  }

  /**
   * Commit every held entry now — leaving the view must not drop them.
   * `now` starts everything queued at once, because on `pagehide` there is no
   * later turn in which a queued item would get its slot. It lifts the bound
   * for this one drain only: `pagehide` also fires entering the back/forward
   * cache, and a restored page must not keep an unbounded sweep.
   */
  flush({ now = false } = {}) {
    for (const e of [...this.entries]) {
      this.timers.clearTimeout(e.timer);
      this.release(e.id, false);
    }
    this.pump(now ? Infinity : this.concurrency);
    this.onChange();
  }

  release(id, start = true) {
    const i = this.entries.findIndex((e) => e.id === id);
    if (i < 0) return;
    const [entry] = this.entries.splice(i, 1);
    this.queue.push(...entry.items);
    if (!start) return;
    this.pump();
    this.onChange();
  }

  pump(limit = this.concurrency) {
    while (this.flight.size < limit && this.queue.length) {
      const item = this.queue.shift();
      this.flight.add(item);
      Promise.resolve()
        .then(() => this.commitOne(item))
        .catch((err) => this.onFail(item, err))
        .finally(() => {
          this.flight.delete(item);
          this.pump();
          this.onChange();
        });
    }
  }
}
