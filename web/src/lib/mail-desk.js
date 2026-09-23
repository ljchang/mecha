// The desktop mail triage's logic, kept out of MailDesk.svelte so node can
// test it without a component rig (web/test/mail-desk.mjs).
//
// Three ideas carry the desk, and each lives here rather than in markup:
//
// - **Lanes are the store's states, not a second classifier.** A row's lane is
//   read off `state` and `bucket` exactly as `Record::needs_me` and the TUI
//   read them; the page never decides on its own what needs the owner.
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
 * problem by definition, so it sits with the threads that need them.
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
      if (row.bucket === 'respond') return 'respond';
      if (row.bucket === 'notify') return 'notify';
      return null;
    default:
      return null;
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
