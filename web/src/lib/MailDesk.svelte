<script>
  import { onDestroy, untrack } from 'svelte';
  import { SvelteMap, SvelteSet } from 'svelte/reactivity';
  import { apiFetch as fetch } from './api.js';
  import { parseThread } from './mail-thread.js';
  import {
    LANES,
    laneOf,
    sortRows,
    acceptVerb,
    keyOf,
    senderOf,
    sweepGroups,
    ageOf,
    PendingActions,
  } from './mail-desk.js';

  // Mail triage at a desk: lanes, a dense list and the open thread side by
  // side, driven from the keyboard. The phone keeps Mail.svelte; App.svelte
  // picks between them at the rail's 900px breakpoint.
  //
  // Same contract as the phone: the list is a store read, the reader is
  // `mecha mail show`'s text, every action is a `mecha mail …` verb through
  // /api/mail/act, and drafting verbs stage into the outbox — nothing sends
  // from here. Spam still confirms, for the phone's reason: it is the one
  // verb whose effect leaves the mailbox.
  //
  // What is new is timing. Each verb is a CLI child that starts the mail
  // provider, which is what made the phone page feel slow when every action
  // waited on it. Here an action takes the row off the list at once and is
  // held for a few seconds (`PendingActions`), so `z` can take it back, then
  // commits in the background. And the next threads are read ahead, so
  // moving the cursor opens a thread that is already loaded.
  //
  // Leaving the view commits anything still held (dropping it would be
  // worse), and a fresh mount starts with no memory of it: come straight
  // back and a row whose CLI child has not finished yet shows again for a
  // moment. The phone page has the same window.

  const HOLD_MS = 5000;
  // Drafting verbs answer at once and run detached (`spawn_detached`), so the
  // commit bound does not bound them: each is a whole agent run on the local
  // model. One keystroke may start at most this many.
  const MAX_DRAFTS = 5;
  const DRAFTING = new Set(['reply', 'schedule', 'forward']);
  // Full thread text kept for re-reading; the oldest are dropped past this.
  const MAX_READS = 60;
  const VERB_PAST = {
    archive: 'Archived',
    dismiss: 'Dismissed',
    task: 'Task created',
    spam: 'Marked spam',
    reply: 'Reply drafting',
    schedule: 'Invite drafting',
    forward: 'Forward drafting',
    'needs-info': 'Parked',
  };
  const VERB_LABEL = {
    reply: 'Draft reply',
    archive: 'Archive',
    task: 'Make task',
    schedule: 'Schedule',
    spam: 'Spam',
    forward: 'Forward',
  };
  const INBOX = { id: 'inbox', label: 'Plain inbox', key: '5' };

  let rows = $state(null);
  let inbox = $state(null);
  let inboxNote = $state(null);
  let error = $state(null);
  let lane = $state('respond');
  let cursor = $state(0);
  let search = $state('');
  let searchEl = $state(null);
  let listEl = $state(null);
  let mode = $state('list'); // list | sweep
  let help = $state(false);
  let asking = $state(null); // { verb, label, placeholder, wantTo, required, targets }
  let askText = $state('');
  let askTo = $state('');
  let askEl = $state(null);
  let confirmSpam = $state(null); // the row awaiting a second `!`
  let composing = $state(false);
  let cTo = $state('');
  let cSubject = $state('');
  let cBody = $state('');
  let toast = $state(null); // { text, undo }
  let toastTimer;
  let gPrefix = 0; // timestamp of a bare `g`, for `g s`

  const selected = new SvelteSet();
  // Committed this session: key -> { state, at }, the store state the row had
  // when it was acted on. The row stays hidden only while a fresh load still
  // reports that old state; once the store has moved it (archived away, or
  // into Parked / In the outbox), the new row is the truth and shows where it
  // now belongs. A drafting run is detached, so "ok" only means it started:
  // if the store still shows the old state after DONE_GRACE_MS, the row comes
  // back rather than vanishing with nothing on screen to say why.
  const done = new SvelteMap();
  const DONE_GRACE_MS = 3 * 60 * 1000;
  const failed = new SvelteMap(); // key -> why the server refused
  const reads = new SvelteMap(); // key -> { status: 'loading' | 'ok' | 'error', text }

  // Sweep state.
  let sweepVerb = $state('archive');
  let sweepCursor = $state(0);
  const sweepOff = new SvelteSet();
  const sweepOpen = new SvelteSet();

  // `PendingActions` is plain JS; `pendingTick` is how its changes reach the
  // page's derivations.
  let pendingTick = $state(0);
  const pending = new PendingActions({
    delayMs: HOLD_MS,
    commit: commitOne,
    onChange: () => pendingTick++,
    onFail: (item, err) => {
      failed.set(keyOf(item.row), String(err?.message ?? err));
      say(`${item.verb} failed for “${item.row.subject || item.row.summary}” — it is back in the list`);
    },
  });

  load();

  async function load() {
    try {
      const res = await fetch('/api/mail');
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${(await res.text()).trim()}`);
      const fresh = await res.json();
      const byKey = new Map(fresh.map((r) => [keyOf(r), r]));
      const now = Date.now();
      for (const [k, d] of done) {
        // The queue load says nothing about plain-inbox threads, which the
        // store may never have seen; `loadInbox` owns those.
        if (d.inbox) continue;
        const r = byKey.get(k);
        if (!r || r.state !== d.state || now - d.at > DONE_GRACE_MS) done.delete(k);
      }
      rows = fresh;
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  // New mail is classified in the background, and drafts land minutes after
  // they start: keep the list current without a manual refresh.
  $effect(() => {
    const every = setInterval(() => {
      if (document.hidden) return;
      load();
      if (lane === 'inbox') loadInbox();
    }, 60 * 1000);
    return () => clearInterval(every);
  });

  async function loadInbox() {
    try {
      const res = await fetch('/api/mail/inbox');
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      // Plain inbox rows carry no verdict, so they have no proposal to accept;
      // every other verb works on them, since a verb only needs the thread.
      const list = Array.isArray(data) ? data : [];
      inboxNote = Array.isArray(data) ? null : (data.note ?? null);
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
      for (const [k, d] of done) {
        if (!d.inbox) continue;
        if (!seen.has(k) || now - d.at > DONE_GRACE_MS) done.delete(k);
      }
      inbox = threads.map((m) => ({
        fromInbox: true,
        thread_id: m.thread_id,
        account: m.account,
        from: m.from,
        from_name: '',
        subject: m.subject ?? '',
        summary: m.snippet ?? '',
        date: m.date ?? '',
        urgency: '',
        tags: [],
        proposed: null,
        unread: m.unread,
      }));
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  async function commitOne({ row, verb, extra }) {
    const res = await fetch('/api/mail/act', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ verb, thread: row.thread_id, account: row.account, ...extra }),
      // A flush on pagehide must outlive the page.
      keepalive: true,
    });
    const text = await res.text();
    if (!res.ok) throw new Error(text.trim() || `HTTP ${res.status}`);
    failed.delete(keyOf(row));
    done.set(keyOf(row), { state: row.state, at: Date.now(), inbox: !!row.fromInbox });
  }

  // Reload once everything held has gone out, so the list shows what the
  // store now says (and any mail classified meanwhile) without a manual step.
  // On the busy → idle edge only: `load` itself prunes `done`, so a
  // condition on `done` here would re-arm this after every load.
  let reloadTimer;
  let wasBusy = false;
  $effect(() => {
    pendingTick;
    const busy = pending.pendingCount + pending.committingCount > 0;
    if (wasBusy && !busy) {
      clearTimeout(reloadTimer);
      reloadTimer = setTimeout(load, 1500);
    }
    wasBusy = busy;
  });

  const flushNow = () => pending.flush({ now: true });
  $effect(() => {
    window.addEventListener('pagehide', flushNow);
    return () => window.removeEventListener('pagehide', flushNow);
  });
  onDestroy(() => {
    clearTimeout(reloadTimer);
    clearTimeout(toastTimer);
    pending.flush();
  });

  function say(text, undo = false) {
    toast = { text, undo };
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), undo ? HOLD_MS : 5000);
  }

  // ---- what is on screen ----

  const hidden = $derived.by(() => {
    pendingTick;
    const keys = pending.hiddenKeys();
    for (const k of done.keys()) keys.add(k);
    return keys;
  });

  const heldCount = $derived((pendingTick, pending.pendingCount));
  const sendingCount = $derived((pendingTick, pending.committingCount));

  const openRows = $derived((rows ?? []).filter((r) => laneOf(r) && !hidden.has(keyOf(r))));

  const laneCounts = $derived.by(() => {
    const c = {};
    for (const r of openRows) c[laneOf(r)] = (c[laneOf(r)] ?? 0) + 1;
    return c;
  });

  const matches = (r) => {
    const q = search.trim().toLowerCase();
    if (!q) return true;
    return [r.subject, r.summary, r.from, r.from_name, ...(r.tags ?? [])]
      .some((f) => (f ?? '').toLowerCase().includes(q));
  };

  const laneRows = $derived(
    lane === 'inbox' ? (inbox ?? []).filter((r) => !hidden.has(keyOf(r))) : openRows.filter((r) => laneOf(r) === lane),
  );
  const visible = $derived((lane === 'inbox' ? laneRows : sortRows(laneRows)).filter(matches));

  const at = $derived(Math.max(0, Math.min(cursor, visible.length - 1)));
  const cur = $derived(visible[at] ?? null);
  // The selection, not what the filter currently shows: typing into `/` after
  // selecting must not quietly narrow what "applies to all" applies to.
  const targets = $derived(selected.size ? laneRows.filter((r) => selected.has(keyOf(r))) : cur ? [cur] : []);
  const curRead = $derived(cur ? reads.get(keyOf(cur)) : null);
  const parsed = $derived(curRead?.status === 'ok' ? parseThread(curRead.text) : null);

  // ---- read-ahead ----

  let readQueue = [];
  let reading = 0;
  function want(row) {
    if (!row) return;
    // A failed read is retried the next time the cursor asks: the read is a
    // CLI child reaching the provider, and one OAuth refresh or MCP startup
    // hiccup must not pin "could not read" on a thread for the session.
    const prev = reads.get(keyOf(row));
    if (prev && prev.status !== 'error') return;
    reads.set(keyOf(row), { status: 'loading', text: '' });
    readQueue.push(row);
    pumpReads();
  }
  function pumpReads() {
    // Two at a time: each is a CLI child reaching the provider, and the one
    // under the cursor must not wait behind a long read-ahead queue.
    while (reading < 2 && readQueue.length) {
      const row = readQueue.shift();
      reading++;
      const q = new URLSearchParams({ thread: row.thread_id, account: row.account });
      fetch(`/api/mail/read?${q}`)
        .then(async (res) => {
          const text = await res.text();
          reads.set(keyOf(row), res.ok ? { status: 'ok', text } : { status: 'error', text: text.trim() });
          // Insertion order is age: drop the oldest finished reads past the cap.
          // Never the thread under the cursor, which would sit on "reading…".
          const keep = cur ? keyOf(cur) : null;
          for (const [k, v] of reads) {
            if (reads.size <= MAX_READS) break;
            if (v.status !== 'loading' && k !== keep) reads.delete(k);
          }
        })
        .catch((e) => reads.set(keyOf(row), { status: 'error', text: String(e?.message ?? e) }))
        .finally(() => {
          reading--;
          pumpReads();
        });
    }
  }
  $effect(() => {
    if (mode !== 'list' || !cur) return;
    const here = cur;
    const ahead = [visible[at + 1], visible[at + 2]];
    // Depends on the cursor only; the cache writes below must not re-run it.
    untrack(() => {
      // Read-ahead is a fixed cost: whatever was queued for rows the cursor
      // has already passed is dropped (the two in flight finish), so holding
      // `j` down a long lane never leaves hundreds of reads behind it.
      for (const r of readQueue) reads.delete(keyOf(r));
      readQueue = [];
      want(here);
      ahead.forEach(want);
    });
  });

  // Keep the cursor row in view as it moves.
  $effect(() => {
    at;
    visible.length;
    const el = listEl?.querySelector('[data-cursor="true"]');
    el?.scrollIntoView({ block: 'nearest' });
  });

  // ---- acting ----

  /** Hold one keystroke's actions; false when it was refused. */
  function hold(items, label) {
    if (!items.length) return false;
    const drafts = items.filter((it) => DRAFTING.has(it.verb)).length;
    if (drafts > MAX_DRAFTS) {
      say(`That would start ${drafts} drafting runs at once — tick ${MAX_DRAFTS} or fewer`);
      return false;
    }
    // A retry clears the old failure, or the row would stay on screen while
    // its new action is held.
    for (const it of items) failed.delete(keyOf(it.row));
    pending.add(items, label);
    selected.clear();
    say(`${label} · z to undo`, true);
    return true;
  }

  function run(verb, extra = {}, list = targets) {
    if (!list.length) return;
    const items = list.map((row) => ({ row, verb, extra }));
    const label = list.length === 1
      ? `${VERB_PAST[verb]} · ${list[0].subject || list[0].summary}`
      : `${VERB_PAST[verb]} ${list.length} threads`;
    hold(items, label);
  }

  function accept() {
    const list = targets;
    const items = [];
    let skipped = 0;
    for (const row of list) {
      const verb = acceptVerb(row);
      if (verb && verb !== 'spam') items.push({ row, verb, extra: {} });
      else skipped++;
    }
    // Spam is never accepted in passing: it confirms, one thread at a time.
    if (list.length === 1 && acceptVerb(list[0]) === 'spam') return askSpam();
    if (!items.length) {
      say(list.length === 1 ? 'No suggestion to accept — pick an action' : 'None of these has a suggestion to accept');
      return;
    }
    const label = items.length === 1
      ? `${VERB_PAST[items[0].verb]} · ${items[0].row.subject || items[0].row.summary}`
      : `Accepted ${items.length} suggestions`;
    hold(items, skipped ? `${label} (${skipped} without one left in place)` : label);
  }

  function undo() {
    const entry = pending.undo();
    if (!entry) {
      say(pending.committingCount ? 'Already sent — that one cannot be taken back' : 'Nothing to undo');
      return;
    }
    say(`Undone · ${entry.label.split(' · ')[0]}`);
  }

  function ask(verb, label, placeholder, { wantTo = false, required = false } = {}) {
    if (!targets.length) return;
    if (targets.length > 1 && (wantTo || verb === 'needs-info')) {
      say('That one works on a single thread');
      return;
    }
    // The threads are fixed now, as `askSpam` fixes its row: a click or the
    // background reload must not retarget a reply that is being written.
    asking = { verb, label, placeholder, wantTo, required, rows: [...targets] };
    askText = '';
    askTo = '';
    queueMicrotask(() => askEl?.focus());
  }

  function submitAsk() {
    const a = asking;
    if (!a) return;
    if (a.wantTo && !askTo.trim()) return;
    if (a.required && !askText.trim()) return;
    const extra = {};
    if (askText.trim()) extra.text = askText.trim();
    if (a.wantTo) extra.to = askTo.trim();
    asking = null;
    run(a.verb, extra, a.rows);
  }

  function askSpam() {
    if (targets.length !== 1) {
      say('Spam is marked one thread at a time');
      return;
    }
    confirmSpam = targets[0];
  }

  function move(d, extend = false) {
    if (!visible.length) return;
    if (extend && cur) selected.add(keyOf(cur));
    cursor = Math.max(0, Math.min(visible.length - 1, at + d));
    if (extend && visible[cursor]) selected.add(keyOf(visible[cursor]));
  }

  function pickLane(id) {
    lane = id;
    cursor = 0;
    selected.clear();
    if (id === 'inbox' && inbox === null) loadInbox();
  }

  async function stageCompose() {
    if (!cTo.trim() || !cSubject.trim() || !cBody.trim()) return;
    try {
      const res = await fetch('/api/mail/compose', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ to: cTo.trim(), subject: cSubject.trim(), body: cBody, account: null }),
      });
      const text = await res.text();
      if (!res.ok) throw new Error(text.trim());
      composing = false;
      cTo = cSubject = cBody = '';
      say('Staged — review it in the Outbox before it sends');
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  // ---- sweep ----

  const SWEEP_VERBS = ['archive', 'reply', 'task', 'schedule'];
  const sweepRows = $derived(openRows.filter((r) => laneOf(r) === 'respond' || laneOf(r) === 'notify'));
  const groups = $derived(sweepGroups(sweepRows, sweepVerb));
  const sweepCounts = $derived(
    Object.fromEntries(SWEEP_VERBS.map((v) => [v, sweepRows.filter((r) => r.proposed === v).length])),
  );
  const checkedRows = $derived(groups.filter((g) => !sweepOff.has(g.key)).flatMap((g) => g.rows));
  const sweepAt = $derived(Math.max(0, Math.min(sweepCursor, groups.length - 1)));

  function openSweep() {
    mode = 'sweep';
    sweepOpen.clear();
    setSweepVerb(sweepVerb);
  }

  // Archive and task sweeps start with every group ticked: that is the bulk
  // the sweep exists for. Drafting sweeps start with none, because each
  // ticked thread is an agent run and `hold` refuses more than MAX_DRAFTS.
  function setSweepVerb(v) {
    sweepVerb = v;
    sweepCursor = 0;
    sweepOff.clear();
    if (DRAFTING.has(v)) for (const g of sweepGroups(sweepRows, v)) sweepOff.add(g.key);
  }

  function applySweep() {
    const n = checkedRows.length;
    if (!n) return;
    const verb = sweepVerb;
    if (hold(checkedRows.map((row) => ({ row, verb, extra: {} })), `${VERB_PAST[verb]} ${n} threads from the sweep`)) {
      mode = 'list';
    }
  }

  function toggleGroup(g) {
    if (sweepOff.has(g.key)) sweepOff.delete(g.key);
    else sweepOff.add(g.key);
  }

  // ---- the keyboard ----

  function sweepKey(e) {
    const g = groups[sweepAt];
    switch (e.key) {
      case 'Escape': mode = 'list'; break;
      case 'ArrowDown': case 'j': sweepCursor = Math.min(groups.length - 1, sweepAt + 1); break;
      case 'ArrowUp': case 'k': sweepCursor = Math.max(0, sweepAt - 1); break;
      case ' ': if (g) toggleGroup(g); break;
      case 'ArrowRight': case 'l': if (g) sweepOpen.add(g.key); break;
      case 'ArrowLeft': case 'h': if (g) sweepOpen.delete(g.key); break;
      case '1': case '2': case '3': case '4':
        setSweepVerb(SWEEP_VERBS[Number(e.key) - 1]);
        break;
      case 'Enter': if (e.shiftKey) applySweep(); else return; break;
      case 'z': undo(); break;
      default: return;
    }
    e.preventDefault();
  }

  function onKey(e) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) {
      if (e.key === 'Escape') {
        if (asking) asking = null;
        else if (t === searchEl && search) search = '';
        t.blur();
        e.preventDefault();
      }
      return;
    }
    if (composing) {
      if (e.key === 'Escape') composing = false;
      return;
    }
    // The reply/park bar is open but lost focus (a click in the reader):
    // keys must not act on the thread underneath it.
    if (asking) {
      if (e.key === 'Escape') {
        asking = null;
        e.preventDefault();
      } else if (e.key === 'Enter') {
        askEl?.focus();
        e.preventDefault();
      }
      return;
    }
    if (help) {
      if (e.key === 'Escape' || e.key === '?') help = false;
      e.preventDefault();
      return;
    }
    if (confirmSpam) {
      if (e.key === '!' || e.key === 'y') {
        const row = confirmSpam;
        confirmSpam = null;
        hold([{ row, verb: 'spam', extra: {} }], `Marked spam · ${row.subject || row.summary}`);
      } else if (e.key === 'Escape' || e.key === 'n') {
        confirmSpam = null;
      }
      e.preventDefault();
      return;
    }
    if (mode === 'sweep') return sweepKey(e);

    if (Date.now() - gPrefix < 1000) {
      gPrefix = 0;
      if (e.key === 's') { openSweep(); e.preventDefault(); }
      else if (e.key === 'i') { pickLane('inbox'); e.preventDefault(); }
      return;
    }

    const lanes = [...LANES, INBOX];
    const byKey = lanes.find((l) => l.key === e.key);
    switch (e.key) {
      case 'ArrowDown': case 'j': move(1, e.shiftKey); break;
      case 'ArrowUp': case 'k': move(-1, e.shiftKey); break;
      case 'J': move(1, true); break;
      case 'K': move(-1, true); break;
      case 'Enter': accept(); break;
      case 'e': run('archive'); break;
      case 'd': run('dismiss'); break;
      case 't': run('task'); break;
      case 'r': ask('reply', 'Steer the reply (optional) — Enter to draft', 'decline politely; ask for the deadline'); break;
      case 's': ask('schedule', 'Steer the invite (optional) — Enter to draft', 'propose Thursday afternoon'); break;
      case 'f': ask('forward', 'Forward to, and a covering line', 'FYI — the one I mentioned', { wantTo: true }); break;
      case 'p': ask('needs-info', 'What are you waiting for?', 'their dates, before I can book', { required: true }); break;
      case '!': askSpam(); break;
      case 'x': if (cur) { const k = keyOf(cur); selected.has(k) ? selected.delete(k) : selected.add(k); } break;
      case 'z': undo(); break;
      case '/': searchEl?.focus(); break;
      case '?': help = true; break;
      case 'c': composing = true; break;
      case 'g': gPrefix = Date.now(); break;
      case ' ': break; // the list scrolls itself with the cursor; not the page
      case 'Escape': selected.clear(); search = ''; break;
      default:
        if (byKey) { pickLane(byKey.id); break; }
        return;
    }
    e.preventDefault();
  }

  $effect(() => {
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  const urgencyClass = (u) => (u === 'now' || u === 'today' || u === 'week' ? `u-${u}` : '');
  const sweepVerbLabel = { archive: 'Archive', reply: 'Draft replies', task: 'Make tasks', schedule: 'Draft invites' };
</script>

<div class="desk">
  <header class="top">
    <span class="brand">mail</span>
    <label class="search">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="11" cy="11" r="7" /><path d="M20 20l-3.5-3.5" /></svg>
      <input bind:this={searchEl} bind:value={search} placeholder="Filter this lane — sender, subject, tag" aria-label="Filter" />
      <kbd>/</kbd>
    </label>
    <span class="grow"></span>
    {#if heldCount || sendingCount}
      <span class="sending">
        {#if heldCount}{heldCount} held{/if}{#if heldCount && sendingCount} · {/if}{#if sendingCount}{sendingCount} sending{/if}
      </span>
    {/if}
    <button class="ghost" onclick={() => (composing = true)}>Compose <kbd>c</kbd></button>
    <button class="ghost" onclick={() => (lane === 'inbox' ? loadInbox() : load())}>Refresh</button>
  </header>

  {#if error}<div class="warn">{error}</div>{/if}

  {#if mode === 'list'}
    <div class="panes">
      <nav class="lanes" aria-label="Lanes">
        <div class="kicker">Lanes</div>
        {#each LANES as l}
          <button class="lane" class:on={lane === l.id} onclick={() => pickLane(l.id)}>
            <kbd>{l.key}</kbd><span class="grow">{l.label}</span><span class="n">{laneCounts[l.id] ?? 0}</span>
          </button>
        {/each}
        <button class="lane" class:on={lane === 'inbox'} onclick={() => pickLane('inbox')}>
          <kbd>{INBOX.key}</kbd><span class="grow">{INBOX.label}</span><span class="n">{inbox ? inbox.filter((r) => !hidden.has(keyOf(r))).length : ''}</span>
        </button>
        <span class="grow"></span>
        <button class="sweepcard" onclick={openSweep}>
          <strong>Sweep {sweepCounts.archive} suggested archives</strong>
          <span>Clear by sender group · <kbd>g</kbd> <kbd>s</kbd></span>
        </button>
      </nav>

      <section class="list" aria-label="Threads">
        <div class="listhead">
          <strong>{lane === 'inbox' ? INBOX.label : LANES.find((l) => l.id === lane)?.label}</strong>
          <span class="n">{visible.length}{search ? ' matching' : ''}</span>
          <span class="grow"></span>
          {#if selected.size}<span class="selnote">{selected.size} selected · actions apply to all · Esc clears</span>{/if}
        </div>
        <div class="rows" bind:this={listEl}>
          {#if rows === null && lane !== 'inbox'}
            <div class="empty">reading the queue…</div>
          {:else if lane === 'inbox' && inbox === null}
            <div class="empty">reading the inbox…</div>
          {:else}
            {#each visible as r, i (keyOf(r))}
              {@const k = keyOf(r)}
              <button
                class="row"
                class:cur={i === at}
                class:sel={selected.has(k)}
                data-cursor={i === at}
                onclick={() => (cursor = i)}
              >
                <span class="urg {urgencyClass(r.urgency)}"></span>
                <span class="rowbody">
                  <span class="line1">
                    <span class="from">{senderOf(r)}</span>
                    {#if failed.has(k)}<span class="chip bad" title={failed.get(k)}>failed</span>{/if}
                    {#if r.state === 'failed'}<span class="chip bad">not classified</span>{/if}
                    {#if r.deadline}<span class="chip due">due {r.deadline}</span>{/if}
                    <span class="grow"></span>
                    <span class="age">{ageOf(r.date)}</span>
                  </span>
                  <span class="subject">{r.subject || r.summary}</span>
                  <span class="line3">
                    <span class="summary">{r.subject ? r.summary : ''}</span>
                    {#if r.proposed && r.proposed !== 'none'}
                      <span class="prop p-{r.proposed}">{i === at ? '⏎ ' : ''}{(VERB_LABEL[r.proposed] ?? r.proposed).toLowerCase()}</span>
                    {/if}
                  </span>
                </span>
              </button>
            {:else}
              <div class="empty">
                {#if search}Nothing here matches “{search}”.{:else if lane === 'inbox'}{inboxNote ?? 'The inbox is empty.'}{:else}Lane clear.{/if}
              </div>
            {/each}
          {/if}
        </div>
      </section>

      <section class="reader" aria-label="Thread">
        {#if cur}
          <div class="readscroll">
            <h1>{cur.subject || cur.summary}</h1>
            <div class="meta">
              <span class="who">{senderOf(cur)}</span>
              {#if cur.from_name && cur.from}<span>&lt;{cur.from}&gt;</span>{/if}
              <span>· {cur.account}</span>
              {#if cur.date}<span>· {ageOf(cur.date)} ago</span>{/if}
              {#if cur.deadline}<span class="chip due">due {cur.deadline}</span>{/if}
              {#each cur.tags ?? [] as tag}<span class="chip">#{tag}</span>{/each}
            </div>
            {#if failed.has(keyOf(cur))}
              <div class="warn">The last action on this thread failed: {failed.get(keyOf(cur))}</div>
            {/if}
            {#if acceptVerb(cur)}
              <div class="suggest">
                <div class="grow">
                  <div class="kicker">Suggested</div>
                  <div><strong>{VERB_LABEL[acceptVerb(cur)]}</strong>{#if cur.subject && cur.summary}<span>{' — '}{cur.summary}</span>{/if}</div>
                </div>
                <button class="primary" onclick={accept}>Accept <kbd>⏎</kbd></button>
              </div>
            {/if}
            <!-- Third-party text: rendered as text nodes only, never markup. -->
            {#if !curRead || curRead.status === 'loading'}
              <div class="empty">reading the thread…</div>
            {:else if curRead.status === 'error'}
              <div class="warn">Could not read this thread: {curRead.text}</div>
            {:else if parsed}
              {#each parsed.messages as m}
                <article>
                  <div class="msgmeta">{m.meta}</div>
                  {#if m.subject && m.subject !== cur.subject}<div class="msgsubj">{m.subject}</div>{/if}
                  <p>{m.body}</p>
                </article>
              {/each}
            {:else}
              <pre class="raw">{curRead.text}</pre>
            {/if}
          </div>

          {#if asking}
            <form class="askbar" onsubmit={(e) => { e.preventDefault(); submitAsk(); }}>
              <div class="asklabel">{asking.label} · {asking.rows.length > 1 ? `${asking.rows.length} threads` : (asking.rows[0].subject || asking.rows[0].summary)}</div>
              {#if asking.wantTo}
                <input bind:this={askEl} bind:value={askTo} placeholder="name@example.edu, …" aria-label="Forward to" />
                <input bind:value={askText} placeholder={asking.placeholder} aria-label="Covering note" />
              {:else}
                <input bind:this={askEl} bind:value={askText} placeholder={asking.placeholder} aria-label={asking.label} />
              {/if}
              <button class="primary" type="submit">Go <kbd>⏎</kbd></button>
              <button class="ghost" type="button" onclick={() => (asking = null)}>Cancel <kbd>esc</kbd></button>
            </form>
          {:else if confirmSpam}
            <div class="askbar">
              <div class="asklabel">Mark “{confirmSpam.subject || confirmSpam.summary}” as spam? This trains the provider's filter.</div>
              <button class="primary" onclick={() => { const row = confirmSpam; confirmSpam = null; hold([{ row, verb: 'spam', extra: {} }], `Marked spam · ${row.subject || row.summary}`); }}>Mark spam <kbd>!</kbd></button>
              <button class="ghost" onclick={() => (confirmSpam = null)}>Cancel <kbd>esc</kbd></button>
            </div>
          {:else}
            <div class="actions">
              <button onclick={accept} disabled={!acceptVerb(cur) && targets.length < 2}><kbd>⏎</kbd>{acceptVerb(cur) ? VERB_LABEL[acceptVerb(cur)] : 'Accept'}</button>
              <button onclick={() => run('archive')}><kbd>e</kbd>Archive</button>
              <button onclick={() => ask('reply', 'Steer the reply (optional) — Enter to draft', 'decline politely; ask for the deadline')}><kbd>r</kbd>Reply</button>
              <button onclick={() => run('task')}><kbd>t</kbd>Task</button>
              <button onclick={() => ask('schedule', 'Steer the invite (optional) — Enter to draft', 'propose Thursday afternoon')}><kbd>s</kbd>Schedule</button>
              <button onclick={() => ask('forward', 'Forward to, and a covering line', 'FYI — the one I mentioned', { wantTo: true })}><kbd>f</kbd>Forward</button>
              <button onclick={() => ask('needs-info', 'What are you waiting for?', 'their dates, before I can book', { required: true })}><kbd>p</kbd>Park</button>
              <button onclick={() => run('dismiss')}><kbd>d</kbd>Dismiss</button>
              <button onclick={askSpam}><kbd>!</kbd>Spam</button>
            </div>
          {/if}
        {:else}
          <div class="empty">Nothing selected.</div>
        {/if}
      </section>
    </div>
  {:else}
    <div class="sweep">
      <div class="sweephead">
        <button class="ghost" onclick={() => (mode = 'list')}><kbd>esc</kbd> back to triage</button>
        <span class="grow"></span>
        {#each SWEEP_VERBS as v, i}
          <button class="tab" class:on={sweepVerb === v} onclick={() => setSweepVerb(v)}>
            <kbd>{i + 1}</kbd>{sweepVerbLabel[v]} <span class="n">{sweepCounts[v]}</span>
          </button>
        {/each}
      </div>
      <div class="sweepbody">
        <div class="sweeptitle">
          <div class="grow">
            <h1>{sweepVerbLabel[sweepVerb]} {sweepCounts[sweepVerb]} thread{sweepCounts[sweepVerb] === 1 ? '' : 's'}?</h1>
            <p>The classifier suggested this for each one. Grouped by sender. Untick anything that deserves a look; the rest go in one keystroke, and one <kbd>z</kbd> brings the whole batch back within {HOLD_MS / 1000} seconds.</p>
          </div>
          <button class="primary" disabled={!checkedRows.length} onclick={applySweep}>{sweepVerbLabel[sweepVerb]} {checkedRows.length} checked <kbd>⇧⏎</kbd></button>
        </div>
        {#each groups as g, i (g.key)}
          <section class="group" class:cur={i === sweepAt}>
            <div class="grouphead">
              <label class="grow">
                <input type="checkbox" checked={!sweepOff.has(g.key)} onchange={() => toggleGroup(g)} />
                <strong>{g.name}</strong>
                {#if g.name !== g.key}<span class="addr">{g.key}</span>{/if}
              </label>
              <button class="ghost small" onclick={() => (sweepOpen.has(g.key) ? sweepOpen.delete(g.key) : sweepOpen.add(g.key))}>
                {g.rows.length} {sweepOpen.has(g.key) ? '▾' : '▸'}
              </button>
            </div>
            {#each sweepOpen.has(g.key) ? g.rows : g.rows.slice(0, 2) as r (keyOf(r))}
              <div class="grouprow"><span class="grow">{r.subject || r.summary}</span><span class="age">{ageOf(r.date)}</span></div>
            {/each}
            {#if !sweepOpen.has(g.key) && g.rows.length > 2}<div class="more">+ {g.rows.length - 2} more</div>{/if}
          </section>
        {:else}
          <div class="empty">Nothing to sweep for this action.</div>
        {/each}
      </div>
    </div>
  {/if}

  <footer class="bar">
    {#if mode === 'list'}
      <span><kbd>↑↓</kbd><kbd>j</kbd><kbd>k</kbd> move</span>
      <span><kbd>⏎</kbd> accept</span>
      <span><kbd>x</kbd> select · <kbd>⇧↓</kbd> extend</span>
      <span><kbd>z</kbd> undo</span>
      <button class="linkish" onclick={() => (help = true)}><kbd>?</kbd> all keys</button>
    {:else}
      <span><kbd>↑↓</kbd> group</span>
      <span><kbd>space</kbd> tick</span>
      <span><kbd>→</kbd> open</span>
      <span><kbd>⇧⏎</kbd> apply</span>
    {/if}
    <span class="grow"></span>
    {#if toast}
      <span class="toast">{toast.text}{#if toast.undo}<button class="linkish" onclick={undo}>undo <kbd>z</kbd></button>{/if}</span>
    {/if}
  </footer>

  {#if help}
    <div class="scrim" onclick={() => (help = false)} aria-hidden="true"></div>
    <div class="help" role="dialog" aria-label="Keyboard shortcuts">
      <div class="helphead"><h2>Keyboard shortcuts</h2><span class="grow"></span><button class="ghost" onclick={() => (help = false)}><kbd>esc</kbd> close</button></div>
      <div class="helpgrid">
        <div><div class="kicker">Move</div>
          <p><kbd>↑↓</kbd> <kbd>j</kbd> <kbd>k</kbd> previous / next</p>
          <p><kbd>1</kbd>–<kbd>5</kbd> lanes</p>
          <p><kbd>/</kbd> filter · <kbd>g</kbd> <kbd>s</kbd> sweep · <kbd>g</kbd> <kbd>i</kbd> plain inbox</p></div>
        <div><div class="kicker">Decide</div>
          <p><kbd>⏎</kbd> accept the suggestion</p>
          <p><kbd>e</kbd> archive · <kbd>d</kbd> dismiss</p>
          <p><kbd>p</kbd> park until someone replies</p></div>
        <div><div class="kicker">Draft (to the outbox)</div>
          <p><kbd>r</kbd> reply · <kbd>s</kbd> schedule</p>
          <p><kbd>f</kbd> forward · <kbd>t</kbd> task</p>
          <p><kbd>c</kbd> compose new</p></div>
        <div><div class="kicker">Batch and recover</div>
          <p><kbd>x</kbd> select · <kbd>⇧↓</kbd> <kbd>J</kbd> <kbd>K</kbd> extend</p>
          <p><kbd>z</kbd> undo (within {HOLD_MS / 1000}s)</p>
          <p><kbd>!</kbd> spam — confirms first</p></div>
      </div>
      <p class="helpnote">Actions wait {HOLD_MS / 1000} seconds before they reach the server, so <kbd>z</kbd> can take one back. Replies, forwards and invites stage in the outbox; nothing sends from here.</p>
    </div>
  {/if}

  {#if composing}
    <div class="scrim" onclick={() => (composing = false)} aria-hidden="true"></div>
    <form class="compose" onsubmit={(e) => { e.preventDefault(); stageCompose(); }}>
      <h2>New email</h2>
      <label>To<input bind:value={cTo} placeholder="name@example.edu" /></label>
      <label>Subject<input bind:value={cSubject} /></label>
      <label>Body<textarea bind:value={cBody} rows="10"></textarea></label>
      <div class="composebar">
        <span class="grow">Staged in the outbox for review — nothing sends from here.</span>
        <button class="ghost" type="button" onclick={() => (composing = false)}>Cancel</button>
        <button class="primary" type="submit" disabled={!cTo.trim() || !cSubject.trim() || !cBody.trim()}>Stage it</button>
      </div>
    </form>
  {/if}
</div>

<style>
  .desk {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--void);
    color: var(--text);
    font-family: var(--sans);
    position: relative;
  }
  .grow { flex: 1; min-width: 0; }
  kbd {
    font-family: var(--mono);
    font-size: 11px;
    padding: 0 5px;
    min-width: 10px;
    text-align: center;
    border: 1px solid #3a3a4a;
    border-radius: 4px;
    color: var(--accent-400);
    background: none;
  }
  button { font: inherit; color: inherit; cursor: pointer; }
  button:disabled { cursor: default; opacity: 0.5; }
  .ghost { background: none; border: 1px solid #2a2a38; border-radius: var(--radius-chip); padding: 5px 10px; font-size: 13px; display: inline-flex; gap: 6px; align-items: center; }
  .ghost:hover { border-color: var(--accent-700); }
  .ghost.small { padding: 2px 8px; font-family: var(--mono); font-size: 12px; color: var(--text-muted); }
  .primary { background: var(--accent-500); color: var(--void); border: 0; border-radius: var(--radius-chip); padding: 7px 14px; font-weight: 600; font-size: 13px; display: inline-flex; gap: 8px; align-items: center; white-space: nowrap; }
  .primary kbd { color: var(--void); border-color: rgba(18, 20, 31, 0.3); }
  .primary:hover:not(:disabled) { background: var(--accent-400); }
  .linkish { background: none; border: 0; padding: 0; color: var(--text-muted); display: inline-flex; gap: 6px; align-items: center; }
  .kicker { font-family: var(--mono); font-size: 11px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--accent-500); }
  .n { font-family: var(--mono); font-size: 12px; color: var(--text-muted); }
  .empty { padding: 40px 24px; text-align: center; color: var(--text-muted); font-size: 14px; }
  .warn { margin: 8px 16px; padding: 8px 12px; border: 1px solid var(--hazard); border-radius: var(--radius-chip); color: var(--hazard); font-size: 13px; }
  .chip { font-family: var(--mono); font-size: 10px; padding: 1px 6px; border: 1px solid #2a2a38; border-radius: 4px; color: var(--text-muted); white-space: nowrap; }
  .chip.due { border-color: transparent; background: #3a2e1a; color: var(--hazard); }
  .chip.bad { border-color: #d4526e; color: #d4526e; }

  .top { height: 52px; flex-shrink: 0; box-sizing: border-box; padding: 0 20px; display: flex; align-items: center; gap: 14px; border-bottom: 1px solid #2a2a38; background: var(--bg); }
  .brand { font-family: var(--mono); font-size: 13px; font-weight: 500; letter-spacing: 0.04em; color: var(--accent-400); }
  .search { display: flex; align-items: center; gap: 8px; flex: 1; max-width: 520px; height: 32px; box-sizing: border-box; padding: 0 10px; border: 1px solid #2a2a38; border-radius: var(--radius); background: var(--void); color: var(--text-muted); }
  .search input { flex: 1; background: none; border: 0; outline: none; color: var(--text); font: inherit; font-size: 13px; }
  .search:focus-within { border-color: var(--accent-500); }
  .sending { font-size: 12px; color: var(--text-muted); font-family: var(--mono); }

  .panes { flex: 1; min-height: 0; display: flex; }
  .lanes { width: 224px; flex-shrink: 0; box-sizing: border-box; padding: 14px 10px; display: flex; flex-direction: column; gap: 2px; border-right: 1px solid #2a2a38; background: var(--bg); }
  .lanes .kicker { padding: 4px 10px 8px; color: var(--text-muted); }
  .lane { display: flex; align-items: center; gap: 10px; height: 36px; padding: 0 10px; border: 0; border-radius: var(--radius-chip); background: none; font-size: 14px; color: #c9c9d3; text-align: left; }
  .lane kbd { border: 0; color: var(--text-muted); padding: 0; }
  .lane:hover { background: #1d1f2c; }
  .lane.on { background: var(--surface); color: var(--text); font-weight: 600; }
  .sweepcard { display: flex; flex-direction: column; gap: 4px; padding: 12px; border: 1px solid #3a3558; border-radius: var(--radius); background: var(--accent-900); text-align: left; }
  .sweepcard strong { font-size: 13px; color: var(--accent-300); }
  .sweepcard span { font-size: 12px; color: var(--text-muted); }

  .list { width: 520px; flex-shrink: 0; display: flex; flex-direction: column; border-right: 1px solid #2a2a38; min-height: 0; }
  .listhead { height: 44px; flex-shrink: 0; box-sizing: border-box; padding: 0 16px; display: flex; align-items: center; gap: 10px; border-bottom: 1px solid #2a2a38; font-size: 14px; }
  .selnote { font-size: 12px; color: var(--accent-400); }
  .rows { flex: 1; min-height: 0; overflow-y: auto; }
  .row { width: 100%; display: flex; gap: 12px; align-items: stretch; box-sizing: border-box; padding: 11px 16px 11px 0; border: 0; border-bottom: 1px solid #1f2130; background: none; text-align: left; }
  .row:hover { background: #181a27; }
  .row.cur { background: var(--surface); box-shadow: inset 0 0 0 1px var(--accent-500); }
  .row.sel { background: #221f38; }
  .row.sel.cur { background: #2a2644; }
  .urg { width: 3px; flex-shrink: 0; border-radius: 0 2px 2px 0; }
  .u-now { background: #d4526e; }
  .u-today { background: var(--hazard); }
  .u-week { background: var(--accent-500); }
  .rowbody { display: flex; flex-direction: column; gap: 3px; flex: 1; min-width: 0; }
  .line1, .line3 { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .from { font-size: 13px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .age { font-family: var(--mono); font-size: 11px; color: var(--text-muted); white-space: nowrap; }
  .subject { font-size: 13px; color: #c9c9d3; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .summary { flex: 1; min-width: 0; font-size: 12px; color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .prop { flex-shrink: 0; font-family: var(--mono); font-size: 11px; padding: 1px 7px; border-radius: 4px; border: 1px solid #2a2a38; color: var(--text-muted); }
  .p-reply, .p-forward { color: var(--accent-400); }
  .p-task { color: #e0a458; }
  .p-schedule { color: #7fc4b8; }
  .p-spam { color: #d4526e; }
  .row.cur .prop { border-color: currentColor; }

  .reader { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg); }
  .readscroll { flex: 1; min-height: 0; overflow-y: auto; box-sizing: border-box; padding: 22px 32px; display: flex; flex-direction: column; gap: 16px; }
  .reader h1 { margin: 0; font-size: 20px; font-weight: 600; line-height: 1.3; }
  .meta { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; font-size: 12px; color: var(--text-muted); }
  .meta .who { color: #c9c9d3; }
  .suggest { display: flex; align-items: center; gap: 14px; padding: 12px 16px; border: 1px solid #3a3558; border-radius: var(--radius); background: var(--accent-900); font-size: 14px; }
  .suggest strong { color: var(--accent-300); font-weight: 600; }
  article { display: flex; flex-direction: column; gap: 6px; padding-bottom: 16px; border-bottom: 1px solid #2a2a38; }
  .msgmeta { font-size: 12px; color: var(--text-muted); }
  .msgsubj { font-size: 13px; font-weight: 600; }
  article p { margin: 0; font-size: 14px; line-height: 1.6; color: #d6d6de; white-space: pre-wrap; overflow-wrap: anywhere; }
  .raw { margin: 0; font-family: var(--mono); font-size: 12px; white-space: pre-wrap; overflow-wrap: anywhere; color: #d6d6de; }
  .actions, .askbar { flex-shrink: 0; box-sizing: border-box; padding: 12px 20px; display: flex; flex-wrap: wrap; gap: 6px; align-items: center; border-top: 1px solid #2a2a38; background: var(--bg); }
  .actions button { display: inline-flex; align-items: center; gap: 8px; height: 32px; padding: 0 10px; border: 1px solid #2a2a38; border-radius: var(--radius-chip); background: var(--surface); font-size: 13px; }
  .actions button:hover:not(:disabled) { border-color: var(--accent-700); }
  .asklabel { width: 100%; font-size: 13px; color: var(--accent-300); }
  .askbar input { flex: 1; min-width: 200px; height: 34px; box-sizing: border-box; padding: 0 10px; border: 1px solid #2a2a38; border-radius: var(--radius-chip); background: var(--void); color: var(--text); font: inherit; font-size: 13px; }
  .askbar input:focus { outline: none; border-color: var(--accent-500); }

  .bar { height: 36px; flex-shrink: 0; box-sizing: border-box; padding: 0 20px; display: flex; align-items: center; gap: 18px; border-top: 1px solid #2a2a38; font-size: 12px; color: var(--text-muted); }
  .bar kbd { border: 0; padding: 0 2px; }
  .toast { display: inline-flex; align-items: center; gap: 12px; height: 26px; padding: 0 12px; border-radius: var(--radius-chip); background: var(--surface); color: var(--text); max-width: 560px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }

  .sweep { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .sweephead { height: 52px; flex-shrink: 0; box-sizing: border-box; padding: 0 20px; display: flex; align-items: center; gap: 6px; border-bottom: 1px solid #2a2a38; background: var(--bg); }
  .tab { display: inline-flex; gap: 8px; align-items: center; height: 32px; padding: 0 12px; border: 1px solid #2a2a38; border-radius: var(--radius-chip); background: none; font-size: 13px; }
  .tab kbd { border: 0; padding: 0; color: var(--text-muted); }
  .tab.on { border-color: var(--accent-500); background: var(--surface); }
  .sweepbody { flex: 1; min-height: 0; overflow-y: auto; box-sizing: border-box; padding: 28px max(24px, calc((100% - 920px) / 2)) 40px; display: flex; flex-direction: column; gap: 14px; }
  .sweeptitle { display: flex; align-items: flex-end; gap: 16px; }
  .sweeptitle h1 { margin: 0 0 6px; font-size: 24px; font-weight: 600; }
  .sweeptitle p { margin: 0; font-size: 14px; color: var(--text-muted); }
  .group { border: 1px solid #2a2a38; border-radius: var(--radius); background: var(--bg); overflow: hidden; }
  .group.cur { border-color: var(--accent-500); }
  .grouphead { display: flex; align-items: center; gap: 12px; padding: 10px 14px; border-bottom: 1px solid #2a2a38; }
  .grouphead label { display: flex; align-items: center; gap: 12px; cursor: pointer; font-size: 14px; }
  .grouphead input { width: 16px; height: 16px; accent-color: var(--accent-500); }
  .addr { font-family: var(--mono); font-size: 11px; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .grouprow { display: flex; gap: 12px; padding: 7px 14px 7px 42px; border-bottom: 1px solid #1f2130; font-size: 13px; color: #c9c9d3; }
  .more { padding: 7px 14px 9px 42px; font-size: 12px; color: var(--text-muted); }

  .scrim { position: absolute; inset: 0; background: rgba(8, 9, 14, 0.6); z-index: 10; }
  .help, .compose { position: absolute; z-index: 11; left: 50%; top: 50%; transform: translate(-50%, -50%); width: min(760px, calc(100% - 48px)); box-sizing: border-box; padding: 26px 30px; border: 1px solid #2a2a38; border-radius: 12px; background: var(--bg); display: flex; flex-direction: column; gap: 16px; }
  .helphead { display: flex; align-items: center; }
  .help h2, .compose h2 { margin: 0; font-size: 18px; font-weight: 600; }
  .helpgrid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 20px 36px; }
  .helpgrid p { margin: 6px 0 0; font-size: 13px; color: #c9c9d3; }
  .helpnote { margin: 0; font-size: 12px; line-height: 1.5; color: var(--text-muted); }
  .compose label { display: flex; flex-direction: column; gap: 6px; font-size: 12px; color: var(--text-muted); }
  .compose input, .compose textarea { box-sizing: border-box; padding: 8px 10px; border: 1px solid #2a2a38; border-radius: var(--radius-chip); background: var(--void); color: var(--text); font: inherit; font-size: 14px; }
  .compose textarea { resize: vertical; }
  .composebar { display: flex; align-items: center; gap: 10px; font-size: 12px; color: var(--text-muted); }
</style>
