// The mail queue both mail pages drive: MailDesk.svelte at a desk and
// Mail.svelte on a phone. They differ in layout and input — keys and a
// cursor, taps and swipes — and in nothing about what an action does, when
// it commits, or what the list shows afterwards. That half lives here once,
// so the two readers cannot drift apart on it (mail-desk.js holds the pure
// rules underneath; this holds the state and the timing).
//
// What it owns:
//
// - **The rows.** The store's queue (`/api/mail`) and, on demand, the plain
//   inbox (`/api/mail/inbox`, one row per thread).
// - **Undo as a delay.** Every action goes through `hold`, which hides its
//   rows at once and keeps them in `PendingActions` for HOLD_MS, where
//   `undo` cancels them outright. Past that they commit in the background.
// - **What stays hidden after a commit.** `done` remembers the store state a
//   row had when it was acted on; a fresh load drops the key once the store
//   reports anything else, so a parked or drafted thread reappears in the
//   lane it moved to, and a detached draft that never staged comes back
//   after DONE_GRACE_MS instead of vanishing.
// - **Reading ahead.** `readAround` fetches the thread in front of the
//   reader and the next few, two at a time, dropping whatever was queued for
//   threads already passed.
//
// Everything a page renders is plain reactive state (`rows`, `hidden`,
// `failed`, `reads`, …), and `attach()` — called once while the page is
// being created — sets up the timers that keep it current.
import { onDestroy } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';
import { apiFetch as fetch } from './api.js';
import { laneOf, keyOf, PendingActions, DRAFTING, splitSender } from './mail-desk.js';

export { DRAFTING };

export const HOLD_MS = 5000;
// Drafting verbs (`DRAFTING`, mail-desk.js) answer at once and run detached,
// so the commit bound does not bound them: each is a whole agent run on the
// local model. One gesture may start at most this many.
export const MAX_DRAFTS = 5;
// Full thread text kept for re-reading; the oldest are dropped past this.
const MAX_READS = 60;
const DONE_GRACE_MS = 3 * 60 * 1000;

export const VERB_PAST = {
  archive: 'Archived',
  dismiss: 'Dismissed',
  task: 'Task created',
  spam: 'Marked spam',
  reply: 'Reply drafting',
  schedule: 'Invite drafting',
  forward: 'Forward drafting',
  'needs-info': 'Parked',
};
export const VERB_LABEL = {
  reply: 'Draft reply',
  archive: 'Archive',
  task: 'Make task',
  schedule: 'Schedule',
  spam: 'Spam',
  forward: 'Forward',
};

export class MailQueue {
  rows = $state(null);
  inbox = $state(null);
  inboxNote = $state(null);
  error = $state(null);
  /** key -> { state, at, inbox }: committed this session (see the header). */
  done = new SvelteMap();
  /** key -> why the server refused the last action on it. */
  failed = new SvelteMap();
  /** key -> { status: 'loading' | 'ok' | 'error', text } */
  reads = new SvelteMap();

  // `PendingActions` is plain JS; `#tick` is how its changes reach the
  // derivations below.
  #tick = $state(0);
  #readQueue = [];
  #reading = 0;

  /**
   * `say(text, undoable)` shows a notice; `keepKey()` names the thread the
   * page is showing, which the read cache must never evict.
   */
  constructor({ say, keepKey = () => null }) {
    this.say = say;
    this.keepKey = keepKey;
    this.pending = new PendingActions({
      delayMs: HOLD_MS,
      commit: (item) => this.#commitOne(item),
      onChange: () => this.#tick++,
      onFail: (item, err) => {
        this.failed.set(keyOf(item.row), String(err?.message ?? err));
        this.say(`${item.verb} failed for “${item.row.subject || item.row.summary}” — it is back in the list`);
      },
    });
  }

  hidden = $derived.by(() => {
    this.#tick;
    const keys = this.pending.hiddenKeys();
    for (const k of this.done.keys()) keys.add(k);
    return keys;
  });
  heldCount = $derived((this.#tick, this.pending.pendingCount));
  sendingCount = $derived((this.#tick, this.pending.committingCount));
  /** Queue rows in a lane and not hidden. */
  openRows = $derived((this.rows ?? []).filter((r) => laneOf(r) && !this.hidden.has(keyOf(r))));
  laneCounts = $derived.by(() => {
    const c = {};
    for (const r of this.openRows) c[laneOf(r)] = (c[laneOf(r)] ?? 0) + 1;
    return c;
  });
  /** Plain-inbox rows not hidden. */
  inboxRows = $derived((this.inbox ?? []).filter((r) => !this.hidden.has(keyOf(r))));

  /** The rows of one lane, unsorted ('inbox' is the plain inbox). */
  laneRows(lane) {
    return lane === 'inbox' ? this.inboxRows : this.openRows.filter((r) => laneOf(r) === lane);
  }

  load = async () => {
    try {
      const res = await fetch('/api/mail');
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${(await res.text()).trim()}`);
      const fresh = await res.json();
      const byKey = new Map(fresh.map((r) => [keyOf(r), r]));
      const now = Date.now();
      for (const [k, d] of this.done) {
        // The queue load says nothing about plain-inbox threads, which the
        // store may never have seen — `loadInbox` owns whether they are gone.
        // The grace window is still enforced here, because `loadInbox` only
        // runs while that lane is open, and an entry nobody prunes hides its
        // row in every lane for good (a thread parked from the inbox moves
        // to Parked, and `mail recent` keeps listing it).
        if (d.inbox) {
          if (now - d.at > DONE_GRACE_MS) this.done.delete(k);
          continue;
        }
        const r = byKey.get(k);
        if (!r || r.state !== d.state || now - d.at > DONE_GRACE_MS) this.done.delete(k);
      }
      this.rows = fresh;
      this.error = null;
    } catch (e) {
      this.error = String(e?.message ?? e);
    }
  };

  loadInbox = async () => {
    try {
      const res = await fetch('/api/mail/inbox');
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      // Plain inbox rows carry no verdict, so they have no proposal to accept;
      // every other verb works on them, since a verb only needs the thread.
      const list = Array.isArray(data) ? data : [];
      this.inboxNote = Array.isArray(data) ? null : (data.note ?? null);
      // `mail recent` lists messages, not threads, and every verb acts on a
      // thread: keep one row per thread, the newest message, so a thread
      // with two messages in the inbox is one row and one key.
      const seen = new Set();
      const threads = [];
      for (const m of [...list].sort((a, b) => (b.date ?? '').localeCompare(a.date ?? ''))) {
        const k = `${m.account}\u0000${m.thread_id}`;
        if (seen.has(k)) continue;
        seen.add(k);
        threads.push(m);
      }
      // A fresh inbox is the truth only for what it no longer lists: a thread
      // that is gone has been acted on and needs no `done` entry. One still
      // listed may just predate an action that landed while this (slow) fetch
      // was outstanding, so it stays hidden until DONE_GRACE_MS says otherwise.
      const now = Date.now();
      for (const [k, d] of this.done) {
        if (!d.inbox) continue;
        if (!seen.has(k) || now - d.at > DONE_GRACE_MS) this.done.delete(k);
      }
      this.inbox = threads.map((m) => ({
        fromInbox: true,
        thread_id: m.thread_id,
        account: m.account,
        from: splitSender(m.from).address,
        from_name: splitSender(m.from).name,
        subject: m.subject ?? '',
        summary: m.snippet ?? '',
        date: m.date ?? '',
        urgency: '',
        tags: [],
        proposed: null,
        unread: m.unread,
      }));
      this.error = null;
    } catch (e) {
      this.error = String(e?.message ?? e);
    }
  };

  async #commitOne({ row, verb, extra }) {
    const res = await fetch('/api/mail/act', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ verb, thread: row.thread_id, account: row.account, ...extra }),
      // A flush on pagehide must outlive the page.
      keepalive: true,
    });
    const text = await res.text();
    if (!res.ok) throw new Error(text.trim() || `HTTP ${res.status}`);
    this.failed.delete(keyOf(row));
    this.done.set(keyOf(row), { state: row.state, at: Date.now(), inbox: !!row.fromInbox });
  }

  /**
   * Hold one gesture's actions as one undoable entry; false when refused.
   * `undoHint` is how the page says to undo ("z to undo", "tap Undo").
   */
  hold(items, label, undoHint) {
    if (!items.length) return false;
    const drafts = items.filter((it) => DRAFTING.has(it.verb)).length;
    if (drafts > MAX_DRAFTS) {
      this.say(`That would start ${drafts} drafting runs at once — choose ${MAX_DRAFTS} or fewer`);
      return false;
    }
    // A retry clears the old failure, or the row would stay on screen while
    // its new action is held.
    for (const it of items) this.failed.delete(keyOf(it.row));
    this.pending.add(items, label);
    this.say(undoHint ? `${label} · ${undoHint}` : label, true);
    return true;
  }

  undo() {
    const entry = this.pending.undo();
    if (!entry) {
      this.say(this.pending.committingCount ? 'Already sent — that one cannot be taken back' : 'Nothing to undo');
      return null;
    }
    this.say(`Undone · ${entry.label.split(' · ')[0]}`);
    return entry;
  }

  // ---- reading ----

  #want(row) {
    if (!row) return;
    // A failed read is retried the next time the reader asks: the read is a
    // CLI child reaching the provider, and one OAuth refresh or MCP startup
    // hiccup must not pin "could not read" on a thread for the session.
    const prev = this.reads.get(keyOf(row));
    if (prev && prev.status !== 'error') return;
    this.reads.set(keyOf(row), { status: 'loading', text: '' });
    this.#readQueue.push(row);
    this.#pumpReads();
  }

  #pumpReads() {
    // Two at a time: each is a CLI child reaching the provider, and the one
    // in front of the reader must not wait behind a long read-ahead queue.
    while (this.#reading < 2 && this.#readQueue.length) {
      const row = this.#readQueue.shift();
      this.#reading++;
      const q = new URLSearchParams({ thread: row.thread_id, account: row.account });
      fetch(`/api/mail/read?${q}`)
        .then(async (res) => {
          const text = await res.text();
          this.reads.set(keyOf(row), res.ok ? { status: 'ok', text } : { status: 'error', text: text.trim() });
          // Insertion order is age: drop the oldest finished reads past the
          // cap — never the thread on screen, which would sit on "reading…".
          const keep = this.keepKey();
          for (const [k, v] of this.reads) {
            if (this.reads.size <= MAX_READS) break;
            if (v.status !== 'loading' && k !== keep) this.reads.delete(k);
          }
        })
        .catch((e) => this.reads.set(keyOf(row), { status: 'error', text: String(e?.message ?? e) }))
        .finally(() => {
          this.#reading--;
          this.#pumpReads();
        });
    }
  }

  /**
   * Read `here` first and then `ahead`. Read-ahead is a fixed cost: whatever
   * was queued for threads already passed is dropped (the two in flight
   * finish), so moving fast through a long lane never leaves hundreds of
   * reads behind it. Call it from an effect wrapped in `untrack`, since it
   * writes the cache it reads.
   */
  readAround(here, ahead = []) {
    for (const r of this.#readQueue) this.reads.delete(keyOf(r));
    this.#readQueue = [];
    this.#want(here);
    ahead.forEach((r) => this.#want(r));
  }

  // ---- lifetime ----

  /**
   * Set up the timers. Call once while the page is being created (it uses
   * `$effect` and `onDestroy`). `inboxOpen()` says whether the plain inbox is
   * on screen, so the minute's refresh reloads it too.
   */
  attach({ inboxOpen = () => false } = {}) {
    this.load();

    // New mail is classified through the day, and drafts land minutes after
    // they start: keep the list current without a manual refresh.
    $effect(() => {
      const every = setInterval(() => {
        if (document.hidden) return;
        this.load();
        if (inboxOpen()) this.loadInbox();
      }, 60 * 1000);
      return () => clearInterval(every);
    });

    // Reload once everything held has gone out, so the list shows what the
    // store now says. On the busy → idle edge only: `load` itself prunes
    // `done`, so a condition on `done` here would re-arm after every load.
    let reloadTimer;
    let wasBusy = false;
    $effect(() => {
      this.#tick;
      const busy = this.pending.pendingCount + this.pending.committingCount > 0;
      if (wasBusy && !busy) {
        clearTimeout(reloadTimer);
        reloadTimer = setTimeout(this.load, 1500);
      }
      wasBusy = busy;
    });

    // Leaving commits anything still held (dropping it would be worse), and
    // a fresh mount starts with no memory of it: come straight back and a row
    // whose CLI child has not finished shows again for a moment.
    const flushNow = () => this.pending.flush({ now: true });
    $effect(() => {
      window.addEventListener('pagehide', flushNow);
      return () => window.removeEventListener('pagehide', flushNow);
    });
    onDestroy(() => {
      clearTimeout(reloadTimer);
      this.pending.flush();
    });
  }
}
