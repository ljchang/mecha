<script>
  import { apiFetch as fetch } from './api.js';
  import MailBody from './MailBody.svelte';
  import {
    kindOf, KINDS, editsAsEvent, eventFields, eventArgs, inclusiveEnd, whenLabel, eventZone,
    attendeesOf, MAIL_HEADERS, ago, localZone, EVENT_CARD_KEYS, unreadableAccounts, unreadableNote,
    threadOf, ROUTING_KEYS, toolSuffix, threadMessages, answeredMessage, msgWhen,
    rowSummary, docEdit, DOC_EDIT_KEYS, REJECT_REASONS, tooSoon,
    replySubject, liveThread, sinceDrafted, readOf,
  } from './outbox-view.js';

  // The outbox: every draft waiting on the owner, and the one place any of
  // them goes out. The page renders the whole reviewable object — taint
  // warning, headers, prose, everything-else, and the quoted source the draft
  // answers — because approving without reading is the failure this queue
  // exists to prevent. Every action drives a `mecha outbox …` verb on the
  // box; the owner pressing Send on the open draft is what earns `--yes`.
  //
  // Each draft is shown as the thing it is. A mail reads as a mail (headers,
  // then the letter, rendered); an event reads as a time, a place, a calendar
  // and who gets invited, and edits as fields — a start time or a calendar is
  // not prose, and JSON in a terminal was the only way to change one. At a
  // desk the list and the open draft sit side by side and the keys move
  // through them, as in Mail; on a phone it is a list, then a draft.
  //
  // What "reviewed" means: the taint notice, every link's destination on the
  // page, the source reads, the exact arguments one click away, the reason on
  // a reject. Send is one press. An armed draft used to take a second, on a
  // sheet of raw JSON — but every draft the assistant writes is armed (it
  // read your mail to write it), so the second press was on every draft and
  // taught clicking through; the one thing the JSON showed that the page did
  // not, a link's real destination, the page now shows beside the link.

  let pending = $state([]);
  // Every pending draft's detail, fetched once and kept: the rows need a
  // reply's thread to say who and what it answers (its arguments carry
  // neither), and a draft already read opens instantly. A draft's detail
  // only changes when something here acts on it, which drops its entry.
  let cache = $state({});
  let resolved = $state(0);
  let loaded = $state(false);
  let detail = $state(null);
  let error = $state(null); // an action's failure: stays until the next action
  let notice = $state(null); // what the last action did, briefly
  let noticeTimer = null;
  let openedAt = 0; // when the open draft was shown: `a` is refused on one not yet seen
  let listError = $state(null); // the list poll's own, cleared by the next poll
  let busy = $state(false);
  let filter = $state('all');
  let selectedId = $state(null);

  // What the draft pane is doing.
  let mode = $state('read'); // read | prose | event | reject
  let editDraft = $state('');
  let ev = $state(null); // the event editor's fields
  let evError = $state(null);
  let rejectReason = $state('');
  let showSources = $state(false); // the other reads, or every read when no thread parsed
  let showThread = $state(false); // every message of the thread, not only the one answered
  let showRaw = $state(false); // the thread exactly as the drafting run read it
  let showArgs = $state(false);
  let deliveryEvidence = $state('');
  let calendars = $state(null); // null | 'loading' | 'error' | [{account, calendars}]
  let reasonEl = $state(null);
  let proseEl = $state(null);
  let listEl = $state(null);

  const WIDE = '(min-width: 1000px)';
  let wide = $state(typeof matchMedia === 'function' && matchMedia(WIDE).matches);
  $effect(() => {
    if (typeof matchMedia !== 'function') return;
    const mq = matchMedia(WIDE);
    const on = () => (wide = mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  });

  const counts = $derived.by(() => {
    const c = { all: pending.length };
    for (const p of pending) c[kindOf(p.tool)] = (c[kindOf(p.tool)] ?? 0) + 1;
    return c;
  });
  const visible = $derived(filter === 'all' ? pending : pending.filter((p) => kindOf(p.tool) === filter));
  const kind = $derived(detail ? kindOf(detail.tool) : null);
  // The event card is for a create only; see `editsAsEvent`.
  const asEvent = $derived(!!detail && editsAsEvent(detail.tool));
  const args = $derived(detail?.args ?? {});
  const mailHeaders = $derived((detail?.headers ?? []).filter(([k]) => MAIL_HEADERS.includes(k)));
  // Routing arguments (a thread id, `reply_all: false`) are not rows: they
  // live under "exact arguments", and a reply-all is a chip on the letter.
  const shown = ([k]) => !ROUTING_KEYS.includes(k);
  const restHeaders = $derived((detail?.headers ?? []).filter(([k]) => !MAIL_HEADERS.includes(k)).filter(shown));
  const replyAll = $derived(args.reply_all === true);
  // The store prefixes an uncertain attempt's reason with what the heading
  // already says; the pane shows the heading and the reason once each.
  const failWhy = $derived((detail?.error ?? '').replace(/^Delivery outcome unknown[^.;]*[.;]\s*((inspect the destination|reconcile) before retrying\.\s*)?/i, '').trim());
  const thread = $derived(kind === 'mail' ? threadOf(detail?.sources) : null);
  // The thread the run read, as messages: the one a reply answers is shown
  // as mail — who, when, what they said — and the rest one click away.
  const readThread = $derived(readOf(detail));
  const answered = $derived(readThread ? answeredMessage(readThread, args) : null);
  const otherSources = $derived((detail?.sources ?? []).filter((x) => x !== readThread?.source));
  // A reply's sender is the thread's account, looked up at send time; say so
  // when the draft does not name one itself.
  const fromThread = $derived(thread && !mailHeaders.some(([k]) => k === 'account') ? thread.account : null);
  const title = $derived(
    !detail ? '' : asEvent ? (args.title ?? detail.headline) : detail.headline || replySubject(thread?.subject),
  );
  const invited = $derived(attendeesOf(args));
  const doc = $derived(detail ? docEdit(detail.tool, detail.args) : null);
  const unshown = $derived(
    asEvent
      ? [...(detail?.headers ?? []), ...(detail?.other ?? [])].filter(([k]) => !EVENT_CARD_KEYS.includes(k))
      : (detail?.other ?? []).filter(shown).filter(([k]) => !(doc && DOC_EDIT_KEYS.includes(k))),
  );

  async function loadList() {
    try {
      const res = await fetch('/api/outbox');
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${(await res.text()).trim()}`);
      const data = await res.json();
      pending = data.pending;
      resolved = data.resolved;
      loaded = true;
      listError = null;
      const ids = new Set(pending.map((p) => p.id));
      for (const id of Object.keys(cache)) if (!ids.has(id)) delete cache[id];
      prefetch(pending.map((p) => p.id).filter((id) => !cache[id]));
      // At a desk the pane is never empty while there is something to read.
      if (wide && !selectedId && visible.length) open(visible[0].id);
    } catch (e) {
      listError = String(e?.message ?? e);
    }
  }

  async function fetchDetail(id) {
    const res = await fetch(`/api/outbox/${id}`);
    if (!res.ok) throw new Error((await res.text()).trim());
    const d = await res.json();
    cache[id] = d;
    return d;
  }
  /** Three at a time, so a long queue does not start with a burst. */
  async function prefetch(ids) {
    const queue = [...ids];
    const worker = async () => {
      while (queue.length) {
        const id = queue.shift();
        try { await fetchDetail(id); } catch { /* the row keeps its plain label */ }
      }
    };
    await Promise.all([worker(), worker(), worker()]);
  }

  /** The cached detail at once when there is one; the fetched one always. */
  async function open(id, { keepError = false, fresh = false } = {}) {
    selectedId = id;
    const hit = fresh ? null : cache[id];
    if (hit) show(hit, keepError);
    try {
      const d = await fetchDetail(id);
      if (selectedId !== id) return; // a later click won
      if (!hit) show(d, keepError);
      else if (mode === 'read') detail = d;
    } catch (e) {
      const why = String(e?.message ?? e);
      if (!hit) error = why;
      else if (selectedId === id) {
        // The copy on screen came from the prefetch; if it cannot be reread
        // it may have been sent or rejected elsewhere since. Say so rather
        // than keep showing it as current (found on review).
        delete cache[id];
        error = `This draft could not be reread — it may have been sent or rejected elsewhere. ${why}`;
        loadList();
      }
    }
  }
  // The thread as it is now, for an unpinned reply: `mail_reply` answers the
  // newest message *when it is sent*, and a draft can sit for days. One
  // read-only `mecha mail show` per draft opened, reused for two minutes.
  let live = $state({}); // id → { status: 'loading'|'ok'|'error', thread, at }
  const LIVE_FRESH_MS = 120_000;
  async function checkLive(d) {
    if (toolSuffix(d.tool) !== 'mail_reply' || d.args?.message_id || !d.args?.thread_id) return;
    const account = d.args.account ?? threadOf(d.sources)?.account;
    if (!account) return;
    const had = live[d.id];
    if (had && had.status !== 'error' && Date.now() - had.at < LIVE_FRESH_MS) return;
    live[d.id] = { status: 'loading', at: Date.now() };
    try {
      const q = new URLSearchParams({ thread: d.args.thread_id, account });
      const res = await fetch(`/api/mail/read?${q}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      live[d.id] = { status: 'ok', thread: liveThread(await res.text()), at: Date.now() };
    } catch {
      live[d.id] = { status: 'error', at: Date.now() };
    }
  }
  // Only for a draft that stays selected: `show` runs on every j/k, held
  // keys included, and each reread is a `mecha mail show` subprocess and a
  // provider round-trip (review of #275).
  const LIVE_SETTLE_MS = 300;
  let settleTimer = null;
  function settleThenCheck(d) {
    clearTimeout(settleTimer);
    settleTimer = setTimeout(() => {
      if (selectedId === d.id) checkLive(d);
    }, LIVE_SETTLE_MS);
  }
  const liveNow = $derived(detail ? live[detail.id] : null);
  const since = $derived(liveNow?.status === 'ok' ? sinceDrafted(readThread, liveNow.thread) : null);

  function show(d, keepError) {
    detail = d;
    openedAt = Date.now();
    mode = 'read';
    evError = null;
    // The message a draft answers is always shown; the rest of the thread
    // and the verbatim read wait for a click.
    showThread = false;
    showRaw = false;
    // With no thread to show as mail, the reads are what there is to see.
    // Only a mail draft draws its thread as messages; an event or a doc edit
    // written from a mail has no other rendering of it (found on review).
    showSources = !readOf(d);
    settleThenCheck(d);
    showArgs = false;
    rejectReason = '';
    if (!keepError) error = null;
  }

  function back() {
    detail = null;
    selectedId = null;
    mode = 'read';
    loadList();
  }

  /** After a decision: the next draft at a desk, the list on a phone. */
  async function next() {
    const i = visible.findIndex((p) => p.id === selectedId);
    const after = visible[i + 1] ?? visible[i - 1] ?? null;
    detail = null;
    selectedId = after && wide ? after.id : null;
    mode = 'read';
    await loadList();
    if (wide && after && pending.some((p) => p.id === after.id)) open(after.id);
    else if (wide) {
      selectedId = null;
      if (visible.length) open(visible[0].id);
    }
  }

  function say(text) {
    notice = text;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => (notice = null), 6000);
  }

  /** The verb's stdout on success, null on failure (the reason in `error`). */
  async function act(path, body) {
    busy = true;
    error = null;
    delete cache[detail.id]; // whatever happens, the stored draft may have changed
    try {
      const res = await fetch(`/api/outbox/${detail.id}/${path}`, {
        method: 'POST',
        headers: body ? { 'content-type': 'application/json' } : {},
        body: body ? JSON.stringify(body) : undefined,
      });
      const text = await res.text();
      if (!res.ok) throw new Error(text.trim());
      return text;
    } catch (e) {
      error = String(e?.message ?? e);
      return null;
    } finally {
      busy = false;
    }
  }

  // "It was sent" closes the item; "it wasn't" puts the draft back in
  // front of you, ready to send again — so it stays open rather than moving on.
  async function reconcile(outcome) {
    const id = detail.id;
    if ((await act('reconcile', { outcome, evidence: deliveryEvidence.trim() })) === null) return;
    deliveryEvidence = '';
    if (outcome === 'delivered') {
      say('Recorded as sent.');
      next();
    } else {
      say('Recorded as not sent — the draft can go again.');
      open(id, { fresh: true });
    }
  }
  // Never on a draft that appeared under your finger or pointer: after a
  // send the next draft opens in the same place, instantly from the cache,
  // and a second press or click would send it unread.
  async function approve() {
    if (!detail || busy || detail.delivery_uncertain) return;
    if (tooSoon(openedAt)) {
      say('This draft just opened — press again to send it.');
      return;
    }
    const id = detail.id;
    const out = await act('approve');
    if (out !== null) {
      say(sentLine(out) ?? 'Sent.');
      next();
    } else {
      // Reread it: a failed send changes the draft (its error, and whether
      // delivery is now uncertain), and the page must show that draft, not
      // the one it had before the press.
      await loadList();
      if (selectedId === id) open(id, { keepError: true, fresh: true });
    }
  }
  /** The tool's own one-line answer out of the verb's stdout, if it gave one. */
  const sentLine = (out) =>
    out.split('\n').slice(1).map((l) => l.trim()).find((l) => /^(sent|replied|created|added)\b/i.test(l)) ?? null;
  // The reasons are one press each (1, 2, 3 at a desk); the box is for
  // anything else, so it is not focused until you click into it.
  function startReject() {
    mode = 'reject';
  }
  async function reject(reason = rejectReason) {
    if (!reason.trim() || busy) return;
    if ((await act('reject', { reason: reason.trim() })) !== null) {
      rejectReason = '';
      say('Rejected.');
      next();
    }
  }
  function startEdit() {
    if (!detail || detail.kind === 'publish') return;
    if (editsAsEvent(detail.tool)) {
      ev = inclusiveEnd(eventFields(detail.args));
      evError = null;
      mode = 'event';
      if (calendars === null || calendars === 'error') loadCalendars();
    } else if (detail.body != null) {
      editDraft = detail.body ?? '';
      mode = 'prose';
      queueMicrotask(() => proseEl?.focus());
    }
  }
  function proseKey(e) {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      saveProse(true);
    }
  }
  // `andSend`: the edit is saved, then the saved draft goes — one press for
  // the commonest review there is, "fix a word and send it".
  async function saveProse(andSend = false) {
    const id = detail.id;
    if ((await act('edit', { body: editDraft })) === null) return;
    if (andSend) return approve();
    open(id, { fresh: true });
  }
  async function saveEvent(andSend = false) {
    const out = eventArgs(detail.args, ev);
    if (out.error) {
      evError = out.error;
      return;
    }
    const id = detail.id;
    if ((await act('edit', { args: out.args })) === null) return;
    if (andSend) return approve();
    open(id, { fresh: true });
  }

  async function loadCalendars() {
    calendars = 'loading';
    try {
      const res = await fetch('/api/mail/calendars');
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      calendars = Array.isArray(data) ? data : [];
    } catch {
      calendars = 'error';
    }
  }
  const writable = (c) => c.can_edit ?? ['owner', 'writer'].includes(c.access_role);
  const calendarName = $derived.by(() => {
    if (!Array.isArray(calendars)) return null;
    const acct = calendars.find((a) => a.account === (args.account ?? ''));
    const id = args.calendar_id ?? 'primary';
    const c = acct?.calendars?.find((c) => c.id === id || (id === 'primary' && c.is_primary));
    return c?.name ?? null;
  });
  // The pane names the calendar an event lands on; ask once, lazily.
  $effect(() => {
    if (asEvent && calendars === null) loadCalendars();
  });

  const calKey = (account, id) => `${account}\u0000${id}`;
  function pickCalendar(value) {
    const [account, id] = value.split('\u0000');
    ev.account = account;
    ev.calendar_id = id;
  }
  const evCalKey = $derived.by(() => {
    if (!ev) return '';
    if (!Array.isArray(calendars)) return calKey(ev.account, ev.calendar_id);
    const acct = calendars.find((a) => a.account === ev.account);
    const primary = acct?.calendars?.find((c) => c.is_primary);
    const id = ev.calendar_id === 'primary' && primary ? primary.id : ev.calendar_id;
    return calKey(ev.account, id);
  });
  const evCalListed = $derived(
    Array.isArray(calendars) && calendars.some((a) => (a.calendars ?? []).some((c) => calKey(a.account, c.id) === evCalKey)),
  );

  const approveLabel = $derived(
    !detail ? 'Approve'
      : kind === 'mail' ? 'Send'
      : asEvent ? (invited.length ? 'Add and send invites' : 'Add to calendar')
      : kind === 'doc' ? 'Apply the edit'
      : 'Approve',
  );
  const canEdit = $derived(!!detail && detail.kind !== 'publish' && (editsAsEvent(detail.tool) || detail.body != null));

  // ---- keys, at a desk ----
  function move(d) {
    if (!visible.length) return;
    const i = visible.findIndex((p) => p.id === selectedId);
    const j = Math.max(0, Math.min(visible.length - 1, i < 0 ? 0 : i + d));
    open(visible[j].id);
  }
  function onKey(e) {
    if (!wide || e.metaKey || e.ctrlKey || e.altKey) return;
    // A held key repeats, and none of these keys can be undone: a held `a`
    // would send every draft in the list (found on review).
    if (e.repeat && ['a', 'x', 'e'].includes(e.key)) {
      e.preventDefault();
      return;
    }
    const t = e.target;
    const typing = t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable);
    if (e.key === 'Escape') {
      if (mode !== 'read') {
        mode = 'read';
        e.preventDefault();
      }
      t?.blur?.();
      return;
    }
    if (typing) return;
    if (mode === 'reject' && /^[1-9]$/.test(e.key) && REJECT_REASONS[Number(e.key) - 1]) {
      e.preventDefault();
      reject(REJECT_REASONS[Number(e.key) - 1]);
      return;
    }
    if ((e.key === 'Enter' || e.key === ' ') && t?.closest?.('button, a, select, label')) return;
    switch (e.key) {
      case 'j': case 'ArrowDown': if (mode === 'read') move(1); else return; break;
      case 'k': case 'ArrowUp': if (mode === 'read') move(-1); else return; break;
      case 'a': if (mode === 'read') approve(); else return; break;
      case 'e': if (mode === 'read' && canEdit) startEdit(); else return; break;
      case 'x': if (mode === 'read' && detail) startReject(); else return; break;
      default: return;
    }
    e.preventDefault();
  }
  $effect(() => {
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
  $effect(() => {
    selectedId;
    listEl?.querySelector('.row.on')?.scrollIntoView({ block: 'nearest' });
  });

  loadList();
  const timer = setInterval(() => {
    if (!document.hidden) loadList();
  }, 30_000);
  $effect(() => () => clearInterval(timer));

  const monthOf = (a) => {
    const s = a?.start_time ?? '';
    if (/^\d{4}-\d{2}-\d{2}$/.test(s)) return new Date(`${s}T12:00:00Z`).toLocaleDateString('en-US', { month: 'short', timeZone: 'UTC' });
    const ms = Date.parse(s);
    return Number.isNaN(ms) ? '' : new Date(ms).toLocaleDateString('en-US', { month: 'short', timeZone: eventZone(a) });
  };
  const dayOf = (a) => {
    const s = a?.start_time ?? '';
    if (/^\d{4}-\d{2}-\d{2}$/.test(s)) return String(Number(s.slice(8, 10)));
    const ms = Date.parse(s);
    return Number.isNaN(ms) ? '' : new Date(ms).toLocaleDateString('en-US', { day: 'numeric', timeZone: eventZone(a) });
  };
  // In the event's own zone, as the pane shows it, so the two agree.
  const rowWhen = (p) => (p.start_time ? whenLabel({ start_time: p.start_time, timezone: p.timezone, all_day: p.all_day }) : '');
</script>

{#snippet hazardGlyph(size = 13)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

{#snippet message(m, target)}
  <!-- Third-party text: the left rule marks it, as the gutter did. -->
  <article class="msg" class:target>
    <header class="msghead">
      <span class="who">{m.name || m.address}</span>
      {#if m.name}<span class="addr">{m.address}</span>{/if}
      <span class="grow"></span>
      <span class="when">{msgWhen(m.date)}</span>
    </header>
    <MailBody text={m.body} />
  </article>
{/snippet}

{#snippet kindGlyph(k, size = 16)}
  <svg viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    {#if k === 'mail'}<rect x="3" y="5" width="18" height="14" rx="2" /><path d="M3 7l9 6 9-6" />
    {:else if k === 'event'}<rect x="3" y="5" width="18" height="16" rx="2" /><path d="M3 10h18M8 3v4M16 3v4" />
    {:else if k === 'doc'}<path d="M14 3H7a2 2 0 00-2 2v14a2 2 0 002 2h10a2 2 0 002-2V8z" /><path d="M14 3v5h5M9 13h6M9 17h6" />
    {:else if k === 'poll'}<path d="M5 20V10M12 20V4M19 20v-7" />
    {:else}<circle cx="12" cy="12" r="8" /><path d="M12 8v4l3 2" />{/if}
  </svg>
{/snippet}

<div class="ob" class:wide>
  {#if wide || !detail}
    <aside class="listpane">
      <header class="listhead">
        <span class="title">Outbox</span>
        <span class="count">{pending.length} waiting</span>
        <span class="grow"></span>
        <span class="muted">{resolved} done</span>
      </header>
      {#if notice}<div class="notice" role="status">{notice}</div>{/if}
      {#if pending.length}
        <div class="filters" aria-label="Kind">
          <button class="fchip" class:on={filter === 'all'} onclick={() => (filter = 'all')}>All <span>{counts.all}</span></button>
          {#each KINDS as k}
            {#if counts[k.id]}
              <button class="fchip" class:on={filter === k.id} onclick={() => (filter = k.id)}>{k.label} <span>{counts[k.id]}</span></button>
            {/if}
          {/each}
        </div>
      {/if}
      <div class="rows" bind:this={listEl}>
        {#if listError || (error && !detail)}<div class="warnline pad">{@render hazardGlyph()}<span>{listError ?? error}</span></div>{/if}
        {#each visible as item (item.id)}
          {@const sum = rowSummary(cache[item.id])}
          {@const moved = live[item.id]?.status === 'ok' ? sinceDrafted(readOf(cache[item.id]), live[item.id].thread) : null}
          <button class="row" class:on={item.id === selectedId} onclick={() => open(item.id)}>
            <span class="kicon k-{kindOf(item.tool)}">{@render kindGlyph(kindOf(item.tool))}</span>
            <span class="rbody">
              <span class="rtop">
                {#if moved?.grew}
                  <!-- Reread when the draft was opened: the reply now goes to
                       whoever wrote last, not to who the draft answered. -->
                  <span class="rwho">{moved.newest.name || moved.newest.address}</span>
                {:else if sum?.who}
                  <span class="rwho">{sum.who}</span>
                {:else}
                  <span class="rlabel">{item.label}</span>
                  {#if item.account}<span class="acct">{item.account}</span>{/if}
                {/if}
                <span class="grow"></span>
                {#if moved?.grew}<span class="stuck">{moved.grew} new</span>{/if}
                {#if cache[item.id]?.delivery_uncertain}<span class="stuck">check sent</span>{:else if cache[item.id]?.error}<span class="stuck">failed</span>{/if}
                <span class="when">{ago(item.created_at)}</span>
              </span>
              {#if item.headline || sum?.subject}<span class="rhead">{item.headline || sum.subject}</span>{/if}
              {#if rowWhen(item)}<span class="rsnip strong">{rowWhen(item)}</span>
              {:else if docEdit(item.tool, cache[item.id]?.args)}
                {@const d = docEdit(item.tool, cache[item.id]?.args)}
                <span class="rsnip">replace “{d.find}” → “{d.replace}”</span>
              {:else if item.snippet}<span class="rsnip">{item.snippet}</span>{/if}
              {#if item.edited}<span class="edited">edited by you</span>{/if}
            </span>
          </button>
        {:else}
          <div class="empty">
            {#if !loaded}reading the outbox…{:else if filter !== 'all'}Nothing of this kind is waiting.{:else}Nothing waiting on you.{/if}
          </div>
        {/each}
      </div>
      {#if wide}
        <footer class="keys"><span><kbd>j</kbd><kbd>k</kbd> move</span><span><kbd>a</kbd> send</span><span><kbd>e</kbd> edit</span><span><kbd>x</kbd> reject</span><span><kbd>esc</kbd> cancel</span></footer>
      {/if}
    </aside>
  {/if}

  {#if detail}
    <section class="pane" aria-label="Draft">
      <div class="scroll">
        <div class="dtop">
          {#if !wide}
            <button class="backbtn" onclick={back} aria-label="back">
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6l-6 6 6 6" /></svg>
            </button>
          {/if}
          <span class="kpill k-{kind}">{@render kindGlyph(kind, 13)}{detail.label}</span>
          <span class="grow"></span>
          <span class="muted mono">staged {ago(detail.created_at)}{detail.edited ? ' · edited by you' : ''}</span>
        </div>
        {#if title}<h1>{title}</h1>{/if}

        {#if detail.taint.armed}
          <!-- One line: nearly every draft carries it, and a banner on every
               draft is one nobody reads. What it asks is specific. -->
          <div class="taint">
            {@render hazardGlyph(13)}
            <span><strong>Written after reading outside content.</strong> Check the recipients, links and times are yours.</span>
          </div>
        {/if}

        <!-- The bar carries an action's error; where there is no bar, here. -->
        {#if error && (detail.delivery_uncertain || mode === 'event' || mode === 'prose')}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}

        {#if detail.error}
          <!-- A failed send stays pending: the draft is good, the delivery was
               not. Saying which is the difference between an item somebody
               can fix and an item that sits in the queue being clicked at. -->
          <div class="failline">
            {@render hazardGlyph()}
            <div>
              <div class="failhead">{detail.delivery_uncertain ? 'mecha could not confirm this was sent' : 'The last attempt did not send'}</div>
              {#if failWhy}<div class="failwhy">{failWhy}</div>{/if}
            </div>
          </div>
        {/if}

        {#if detail.delivery_uncertain}
          <!-- Sending again before anyone checks is how a mail goes out twice,
               so the draft cannot be sent from here until you say what the
               destination shows. Neither answer sends anything. -->
          <div class="card pad col">
            <p class="note">Look in {fromThread ?? args.account ?? 'the account'}'s Sent folder{kind === 'event' ? ' or calendar' : ''}, then say what you found.</p>
            <label class="field">What you checked
              <input bind:value={deliveryEvidence} placeholder="e.g. not in Sent — or the message id you found" />
            </label>
            <div class="btnrow start">
              <button class="btn" disabled={busy || !deliveryEvidence.trim()} onclick={() => reconcile('not-delivered')}>It wasn't sent</button>
              <button class="btn" disabled={busy || !deliveryEvidence.trim()} onclick={() => reconcile('delivered')}>It was sent</button>
            </div>
          </div>
        {/if}

        {#if asEvent && mode === 'event' && ev}
          <!-- The event, as fields. What is saved is what Approve sends. -->
          <form class="card pad col" onsubmit={(e) => { e.preventDefault(); saveEvent(); }}>
            <label class="field">Title<input bind:value={ev.title} /></label>
            <label class="check"><input type="checkbox" bind:checked={ev.allDay} /> All day</label>
            <div class="frow">
              <label class="field">{ev.allDay ? 'First day' : 'Date'}<input type="date" bind:value={ev.date} onchange={() => { if (!ev.endDate || ev.endDate < ev.date) ev.endDate = ev.date; }} /></label>
              {#if !ev.allDay}
                <label class="field">Starts<input type="time" bind:value={ev.start} /></label>
                <label class="field">Ends<input type="time" bind:value={ev.end} /></label>
              {/if}
              <label class="field">{ev.allDay ? 'Last day' : 'End date'}<input type="date" bind:value={ev.endDate} /></label>
            </div>
            {#if !ev.allDay}<div class="hint">Times are in {ev.zone}.</div>{/if}
            <div class="field">
              <span>Calendar</span>
              {#if Array.isArray(calendars) && calendars.length}
                <select value={evCalKey} onchange={(e) => pickCalendar(e.currentTarget.value)} aria-label="Calendar">
                  {#if !evCalListed}
                    <option value={evCalKey}>{ev.account || 'default account'} · {ev.calendar_id}</option>
                  {/if}
                  {#each calendars as a}
                    {#if a.error}
                      <!-- An account the provider would not list is shown as
                           unreadable, never as an account with no calendars. -->
                      <optgroup label={`${a.account} — could not be read`}>
                        <option disabled value="">{a.error}</option>
                      </optgroup>
                    {:else}
                      <optgroup label={a.account}>
                        {#each (a.calendars ?? []).filter(writable) as c}
                          <option value={calKey(a.account, c.id)}>{c.name}{c.is_primary ? ' (primary)' : ''}</option>
                        {/each}
                      </optgroup>
                    {/if}
                  {/each}
                </select>
                {#if unreadableNote(unreadableAccounts(calendars))}
                  <span class="hint warntext">{unreadableNote(unreadableAccounts(calendars))}</span>
                {/if}
              {:else}
                <div class="frow">
                  <input bind:value={ev.account} placeholder="account" aria-label="Account" />
                  <input bind:value={ev.calendar_id} placeholder="primary" aria-label="Calendar id" />
                </div>
                <span class="hint">{calendars === 'loading' ? 'reading your calendars…' : calendars === 'error' ? 'Could not list your calendars — type the account and calendar id.' : Array.isArray(calendars) ? 'No calendars were listed for any account — type the account and calendar id.' : ''}</span>
              {/if}
            </div>
            <label class="field">Location<input bind:value={ev.location} /></label>
            <label class="field">Invite — each address gets an invitation; leave it empty to keep the event on your calendar only
              <input bind:value={ev.attendees} placeholder="nobody" />
            </label>
            <label class="field">Notes<textarea rows="4" bind:value={ev.description}></textarea></label>
            {#if evError}<div class="warnline">{@render hazardGlyph()}<span>{evError}</span></div>{/if}
            <div class="btnrow">
              <button class="btn" type="button" onclick={() => (mode = 'read')}>Cancel{#if wide}<kbd>esc</kbd>{/if}</button>
              <button class="btn" type="submit" disabled={busy}>Save</button>
              <button class="btn primary" type="button" disabled={busy} onclick={() => saveEvent(true)}>{`Save & ${approveLabel.toLowerCase()}`}</button>
            </div>
          </form>
        {:else if asEvent}
          <div class="card event">
            <div class="cal"><span class="cm">{monthOf(args)}</span><span class="cd">{dayOf(args)}</span></div>
            <div class="evbody">
              <div class="evwhen">{whenLabel(args)}</div>
              {#if !args.all_day && eventZone(args) !== localZone()}
                <div class="muted">{whenLabel(args, localZone())} where you are</div>
              {/if}
              {#if args.location}<div class="evline"><span class="lk">where</span><span class="v">{args.location}</span></div>{/if}
              <div class="evline"><span class="lk">calendar</span><span class="v">{args.account ?? 'default account'} · {calendarName ?? (args.calendar_id && args.calendar_id !== 'primary' ? args.calendar_id : 'primary')}</span></div>
              <div class="evline">
                <span class="lk">invites</span>
                {#if invited.length}
                  <span class="v">{invited.join(', ')}</span>
                  <span class="chip warnchip">{invited.length === 1 ? '1 person gets' : `${invited.length} people get`} an invitation</span>
                {:else}
                  <span class="v muted">nobody — on your calendar only</span>
                {/if}
              </div>
            </div>
          </div>
          {#if args.description}
            <div class="card pad"><div class="kicker">notes</div><MailBody text={args.description} compact /></div>
          {/if}
        {:else if kind === 'mail'}
          {#if readThread}
            <section class="answering" aria-label="What this answers">
              <!-- Only a verified split may say who a reply goes back to: a
                   body can forge a header, and a staged reply names no one
                   else. Unverified, it is what the run read, newest last. -->
              <div class="kicker">{toolSuffix(detail.tool) !== 'mail_reply' ? 'Written from' : readThread.verified || since?.grew ? 'Replying to' : 'The thread it read'}</div>
              {#if toolSuffix(detail.tool) === 'mail_reply' && !args.message_id}
                <!-- The draft answers the newest message *the run read*; the
                     reply goes to the newest one when it is sent, and a draft
                     can wait days (review of #272). The thread is reread when
                     the draft opens — a verified live read, so the page can
                     say who. A reply pinned by message_id has a fixed target. -->
                {#if since?.grew}
                  <div class="newmail" role="status">
                    {@render hazardGlyph()}
                    <span><strong>{since.grew} new {since.grew === 1 ? 'message' : 'messages'} since this was drafted.</strong>
                      The draft was written before {since.grew === 1 ? 'it' : 'them'}. Sending now replies to {since.newest.name || since.newest.address}.</span>
                  </div>
                  {#if since.added}
                    {#each since.added as m}{@render message(m, m === since.newest)}{/each}
                  {:else}
                    {@render message(since.newest, true)}
                  {/if}
                  <div class="kicker">what the draft was written to</div>
                {:else if since}
                  <div class="hint ok">✓ No new messages since this was drafted.</div>
                {:else if liveNow?.status === 'loading'}
                  <div class="hint">Checking the thread for new messages…</div>
                {:else if readThread.verified}
                  <!-- A reread that failed, or came back unverifiable, is not a
                       reread that found nothing: say which (review of #275). -->
                  <div class="hint">The newest message when this was drafted. If anyone has written since, the reply goes to them instead{liveNow?.status === 'error' ? " — the thread couldn't be reread just now" : liveNow?.status === 'ok' ? " — the thread was reread, but mecha couldn't confirm what is new in it" : ''}.</div>
                {/if}
              {/if}
              {#if !readThread.verified}
                <!-- An unproven split is not drawn as messages: a parsed header
                     in bold is a sender the page vouches for, and a forged one
                     would be styled exactly like a real one (review of #272).
                     The read is shown as it was read, headers as plain text. -->
                <div class="hint warntext">
                  {readThread.clipped
                    ? "The run's read of this thread was cut short, so newer messages may be missing."
                    : "mecha can't confirm where each message in this read starts, so it's shown exactly as read."}
                  The reply goes to the newest message in the real thread.
                </div>
                <div class="quoted"><span class="gutter"></span><div class="qtext"><MailBody text={readThread.source.text} compact /></div></div>
              {/if}
              {#if readThread.verified && showThread}
                {#each readThread.messages as m}{@render message(m, m === answered)}{/each}
              {:else if readThread.verified && answered}
                {@render message(answered, false)}
              {:else if readThread.verified}
                <div class="muted">The message this replies to is not in the thread the run read — show the thread to see what it did read.</div>
              {/if}
              <div class="answerlinks">
                {#if readThread.verified && readThread.messages.length > 1}
                  <button class="linkish" onclick={() => (showThread = !showThread)}>{showThread ? 'only the message answered' : `whole thread · ${readThread.messages.length} messages`}</button>
                {/if}
                {#if readThread.verified}<button class="linkish" onclick={() => (showRaw = !showRaw)}>{showRaw ? 'hide' : 'exactly what the drafting run read'}</button>{/if}
              </div>
              {#if showRaw && readThread.verified}<pre class="argdump rawread">{readThread.source.text}</pre>{/if}
            </section>
          {/if}
          <div class="card letter">
            {#if mailHeaders.length || fromThread || replyAll}
              <div class="lhead">
                {#if fromThread}
                  <div class="hrow"><span class="hkey">from</span><span class="hval">{fromThread} <span class="muted">— the account this thread is in</span></span></div>
                {/if}
                {#each mailHeaders as [key, value]}
                  <div class="hrow"><span class="hkey">{key}</span><span class="hval">{value}</span></div>
                {/each}
                {#if replyAll}<div class="hrow"><span class="hkey"></span><span class="chip warnchip">reply all — everyone on the thread gets it</span></div>{/if}
              </div>
            {/if}
            {#if mode === 'prose'}
              <textarea class="prosebox" bind:this={proseEl} bind:value={editDraft} rows="14" onkeydown={proseKey}></textarea>
              <div class="btnrow pad">
                <span class="hint grow">Markdown — **bold**, [link](https://…), lists.{#if wide} <kbd>⌘/ctrl ↵</kbd> saves and sends.{/if}</span>
                <button class="btn" onclick={() => (mode = 'read')}>Cancel{#if wide}<kbd>esc</kbd>{/if}</button>
                <button class="btn" disabled={busy} onclick={() => saveProse()}>Save</button>
                <button class="btn primary" disabled={busy} onclick={() => saveProse(true)}>{busy ? 'sending…' : `Save & ${approveLabel.toLowerCase()}`}</button>
              </div>
            {:else if detail.body}
              <div class="lbody"><MailBody text={detail.body} revealLinks /></div>
            {/if}
          </div>
          {#if restHeaders.length}
            <div class="card pad hgrid">
              {#each restHeaders as [key, value]}<div class="hrow"><span class="hkey">{key}</span><span class="hval">{value}</span></div>{/each}
            </div>
          {/if}
        {:else}
          {#if doc}
            <div class="card pad col">
              <div class="kicker">every “{doc.find}” in the document{doc.matchCase ? ', matching case' : ''}, becomes</div>
              <div class="diff"><del>{doc.find}</del><span class="arrow">→</span><ins>{doc.replace || '(nothing — deleted)'}</ins></div>
              {#if doc.url}<a class="openlink" href={doc.url} target="_blank" rel="noopener noreferrer">open the document ↗</a>{/if}
            </div>
          {/if}
          {#if detail.headers?.length}
            <div class="card pad hgrid">
              {#each detail.headers as [key, value]}<div class="hrow"><span class="hkey">{key}</span><span class="hval">{value}</span></div>{/each}
            </div>
          {/if}
          {#if mode === 'prose'}
            <textarea class="prosebox card" bind:this={proseEl} bind:value={editDraft} rows="14" onkeydown={proseKey}></textarea>
            <div class="btnrow">
              <button class="btn" onclick={() => (mode = 'read')}>Cancel{#if wide}<kbd>esc</kbd>{/if}</button>
              <button class="btn" disabled={busy} onclick={() => saveProse()}>Save</button>
              <button class="btn primary" disabled={busy} onclick={() => saveProse(true)}>{busy ? 'sending…' : `Save & ${approveLabel.toLowerCase()}`}</button>
            </div>
          {:else if detail.body}
            <div class="card pad"><MailBody text={detail.body} revealLinks /></div>
          {/if}
        {/if}

        <!-- Every argument is shown somewhere (`detail_json`'s nothing-is-
             dropped property): on an event card, whatever the card itself
             does not render — a `recurrence`, a `send_updates` — lands here. -->
        {#if unshown.length}
          <div class="card pad hgrid">
            {#each unshown as [key, value]}<div class="hrow"><span class="hkey">{key}</span><span class="hval">{value}</span></div>{/each}
          </div>
        {/if}

        {#if otherSources.length}
          <div class="sources">
            <button class="disclose" onclick={() => (showSources = !showSources)} aria-expanded={showSources}>
              <span class="chev" class:open={showSources}>▸</span>
              {readThread ? 'Other things it read' : 'What this answers'}
              <span class="muted mono">{otherSources.length === 1 ? (readThread ? '1 read' : 'the read it made') : `${otherSources.length} reads`} · their words, not your draft</span>
            </button>
            {#if showSources}
              {#each otherSources as source}
                <div class="source">
                  {#if otherSources.length > 1 || readThread}<div class="source-head" title={source.heading}>{toolSuffix(source.tool).replace(/_/g, ' ')}</div>{/if}
                  <!-- Third-party text: the gutter marks every line. -->
                  <div class="quoted"><span class="gutter"></span><div class="qtext"><MailBody text={source.text} compact /></div></div>
                </div>
              {/each}
            {/if}
          </div>
        {/if}

        {#if detail.serves}
          <!-- What the drafting run's plan served when it staged this: a
               pointer from the run, the line's text from the owner's own
               charter. Approving the draft is what confirms it. -->
          <div class="serves">serves <span class="mono">{detail.serves.ref}</span>{detail.serves.text ? ` — ${detail.serves.text}` : ''}</div>
        {/if}
        <div class="provenance">
          <button class="linkish" onclick={() => (showArgs = !showArgs)}>{showArgs ? 'hide' : 'show'} the exact arguments</button>
          <span>· {detail.tool}{detail.session_id ? ` · session ${detail.session_id}` : ''}</span>
        </div>
        {#if showArgs}<pre class="argdump">{JSON.stringify(detail.args, null, 2)}</pre>{/if}
      </div>

      {#if !detail.delivery_uncertain && mode !== 'event' && mode !== 'prose'}
        <div class="bar">
          {#if error}
            <!-- At the button that was pressed: a failure reported at the top
                 of a scrolled pane looked like the button doing nothing. -->
            <div class="barerr" role="alert">{@render hazardGlyph()}<span>{error}</span></div>
          {/if}
          {#if mode === 'reject'}
            <div class="rejectbox">
              <div class="reasons">
                {#each REJECT_REASONS as reason, i}
                  <button class="btn reason" disabled={busy} onclick={() => reject(reason)}>{reason}{#if wide}<kbd>{i + 1}</kbd>{/if}</button>
                {/each}
              </div>
              <form class="rejectrow" onsubmit={(e) => { e.preventDefault(); reject(); }}>
                <input bind:this={reasonEl} bind:value={rejectReason} placeholder="Or say why" aria-label="Reason" />
                <button class="btn" type="button" onclick={() => (mode = 'read')}>Cancel{#if wide}<kbd>esc</kbd>{/if}</button>
                <button class="btn danger" type="submit" disabled={busy || !rejectReason.trim()}>Reject</button>
              </form>
            </div>
          {:else if mode === 'read'}
            <button class="btn primary big" disabled={busy} onclick={approve}>
              {busy ? 'sending…' : approveLabel}{#if wide}<kbd>a</kbd>{/if}
            </button>
            <button class="btn" disabled={busy || !canEdit} onclick={startEdit}>Edit{#if wide}<kbd>e</kbd>{/if}</button>
            <button class="btn" disabled={busy} onclick={startReject}>Reject…{#if wide}<kbd>x</kbd>{/if}</button>
          {/if}
        </div>
      {/if}
    </section>
  {:else if wide}
    <section class="pane empty-pane">
      <div class="empty">{pending.length ? 'Pick a draft on the left.' : 'Nothing waiting on you.'}</div>
    </section>
  {/if}
</div>

<style>
  .ob { flex: 1; display: flex; min-height: 0; position: relative; }
  .grow { flex: 1; min-width: 0; }
  .muted { color: var(--text-muted); font-size: 12px; }
  .mono { font-family: var(--mono); font-size: 11px; }
  .pad { padding: 14px 16px; }
  .col { display: flex; flex-direction: column; gap: 12px; }
  kbd { font-family: var(--mono); font-size: 10px; padding: 0 4px; border: 1px solid #3a3a4a; border-radius: 4px; color: var(--accent-400); margin-left: 8px; }
  button { font: inherit; color: inherit; cursor: pointer; }

  /* ---- the list ---- */
  .listpane { flex: 1; min-width: 0; display: flex; flex-direction: column; min-height: 0; }
  .wide .listpane { flex: 0 0 400px; border-right: 1px solid #2a2a38; }
  .listhead { display: flex; align-items: baseline; gap: 10px; padding: 14px var(--gutter) 10px; }
  .wide .listhead { padding: 16px 18px 10px; }
  .title { font-weight: 600; font-size: 17px; letter-spacing: -0.02em; }
  .count { font-family: var(--mono); font-size: 12px; color: var(--accent-400); }
  .filters { display: flex; gap: 6px; flex-wrap: wrap; padding: 0 var(--gutter) 10px; border-bottom: 1px solid #2a2a38; }
  .wide .filters { padding: 0 18px 12px; }
  .fchip { font-size: 12px; padding: 4px 10px; border-radius: 999px; border: 1px solid #2a2a38; background: none; color: var(--text-muted); }
  .fchip span { font-family: var(--mono); font-size: 11px; margin-left: 4px; opacity: 0.8; }
  .fchip.on { background: var(--accent-900); border-color: var(--accent-700); color: var(--text); }
  .rows { flex: 1; overflow-y: auto; min-height: 0; }
  .row { width: 100%; display: flex; gap: 12px; align-items: flex-start; padding: 12px var(--gutter); border: 0; border-bottom: 1px solid #1f2130; background: none; text-align: left; box-sizing: border-box; }
  .wide .row { padding: 12px 18px; }
  .row:hover { background: #181a27; }
  .row.on { background: var(--surface); box-shadow: inset 3px 0 0 var(--accent-500); }
  .row:focus-visible { outline: 1px solid var(--accent-500); outline-offset: -1px; }
  .kicon { width: 30px; height: 30px; flex-shrink: 0; border-radius: 8px; display: grid; place-items: center; background: #1d1f2c; color: var(--text-muted); }
  .k-mail { color: var(--accent-400); }
  .k-event { color: #7fc4b8; }
  .k-doc { color: #e0a458; }
  .k-poll { color: #c792ea; }
  .rbody { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .rtop { display: flex; align-items: center; gap: 8px; }
  .rlabel { font-size: 11px; font-weight: 600; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  .acct { font-family: var(--mono); font-size: 10px; color: var(--accent-700); }
  .tflag { display: inline-flex; }
  .when { font-family: var(--mono); font-size: 11px; color: var(--text-muted); white-space: nowrap; }
  .rhead { font-size: 14px; font-weight: 500; line-height: 1.35; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rsnip { font-size: 12px; line-height: 1.45; color: var(--text-muted); overflow: hidden; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow-wrap: anywhere; }
  .rsnip.strong { color: #7fc4b8; }
  .edited { font-family: var(--mono); font-size: 10px; color: var(--accent-400); }
  .keys { display: flex; gap: 14px; flex-wrap: wrap; padding: 9px 18px; border-top: 1px solid #2a2a38; font-size: 11px; color: var(--text-muted); }
  .keys kbd { margin: 0 2px 0 0; }
  .empty { color: var(--text-muted); font-size: 14px; padding: 40px 20px; text-align: center; }

  /* ---- the draft ---- */
  .pane { flex: 1; min-width: 0; display: flex; flex-direction: column; min-height: 0; }
  .empty-pane { justify-content: center; }
  .scroll { flex: 1; overflow-y: auto; min-height: 0; padding: 14px var(--gutter) 20px; display: flex; flex-direction: column; gap: 12px; }
  .wide .scroll { padding: 22px max(28px, calc((100% - 820px) / 2)) 28px; }
  .scroll > * { flex-shrink: 0; }
  .dtop { display: flex; align-items: center; gap: 10px; }
  .backbtn { background: none; border: none; color: var(--text-muted); min-width: 40px; min-height: 40px; margin: -8px 0 -8px -12px; display: flex; align-items: center; justify-content: center; }
  .kpill { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; font-weight: 600; padding: 3px 10px 3px 8px; border-radius: 999px; background: #1d1f2c; }
  h1 { margin: 0; font-size: 21px; font-weight: 600; line-height: 1.3; letter-spacing: -0.01em; overflow-wrap: anywhere; }
  .card { background: var(--bg); border: 1px solid #2a2a38; border-radius: var(--radius); }
  .kicker { font-family: var(--mono); font-size: 10px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--text-muted); margin-bottom: 6px; }
  .taint { display: flex; gap: 8px; align-items: baseline; font-size: 12px; line-height: 1.45; color: var(--text-muted); }
  .taint strong { color: var(--hazard); font-weight: 600; }
  .warnline { display: flex; align-items: flex-start; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .failline { display: flex; gap: 10px; align-items: flex-start; padding: 10px 12px; border: 1px solid var(--hazard); border-radius: var(--radius); }
  .failhead { font-size: 13px; color: var(--hazard); }
  .failwhy { font-size: 13px; line-height: 1.45; overflow-wrap: anywhere; }
  .note { margin: 0; font-size: 13px; color: var(--text-muted); }

  .letter { overflow: hidden; }
  .lhead { padding: 12px 16px; border-bottom: 1px solid #2a2a38; display: flex; flex-direction: column; gap: 6px; background: #161826; }
  .lbody { padding: 18px 20px 22px; }
  .hgrid { display: flex; flex-direction: column; gap: 6px; }
  .hrow { display: flex; gap: 12px; font-size: 13px; }
  .hkey { font-family: var(--mono); font-size: 11px; color: var(--text-muted); min-width: 64px; padding-top: 1px; }
  .hval { overflow-wrap: anywhere; min-width: 0; }
  .prosebox { display: block; width: 100%; box-sizing: border-box; border: 0; background: var(--void); color: var(--text); font-family: var(--sans); font-size: 14px; line-height: 1.6; padding: 16px 20px; resize: vertical; outline: none; }
  .prosebox.card { border: 1px solid var(--accent-700); }

  .event { display: flex; gap: 18px; padding: 18px; align-items: flex-start; }
  .cal { width: 64px; flex-shrink: 0; border-radius: 10px; overflow: hidden; border: 1px solid #2a2a38; text-align: center; background: var(--void); }
  .cm { display: block; font-family: var(--mono); font-size: 11px; text-transform: uppercase; letter-spacing: 0.08em; padding: 4px 0; background: #7fc4b8; color: var(--void); font-weight: 600; }
  .cd { display: block; font-size: 26px; font-weight: 600; padding: 6px 0 8px; }
  .evbody { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
  .evwhen { font-size: 16px; font-weight: 600; }
  .evline { display: flex; gap: 10px; align-items: baseline; flex-wrap: wrap; font-size: 13px; }
  .lk { font-family: var(--mono); font-size: 11px; color: var(--text-muted); min-width: 64px; }
  .v { overflow-wrap: anywhere; min-width: 0; }
  .chip { font-family: var(--mono); font-size: 10px; padding: 1px 7px; border-radius: 4px; }
  .warnchip { background: #3a2e1a; color: var(--hazard); }

  .frow { display: flex; gap: 10px; flex-wrap: wrap; }
  .frow > .field, .frow > input { flex: 1; min-width: 130px; }
  .field { display: flex; flex-direction: column; gap: 5px; font-size: 12px; color: var(--text-muted); }
  .field input, .field select, .field textarea, .frow input, .rejectrow input { box-sizing: border-box; padding: 8px 10px; border: 1px solid #2a2a38; border-radius: var(--radius-chip); background: var(--void); color: var(--text); font: inherit; font-size: 14px; color-scheme: dark; }
  .field input:focus, .field select:focus, .field textarea:focus, .rejectrow input:focus, .frow input:focus { outline: none; border-color: var(--accent-500); }
  .field textarea { resize: vertical; line-height: 1.5; }
  .check { display: flex; gap: 8px; align-items: center; font-size: 13px; color: var(--text); }
  .check input { accent-color: var(--accent-500); }
  .hint { font-size: 12px; color: var(--text-muted); }
  .hint.warntext { color: var(--hazard); }

  .sources { display: flex; flex-direction: column; gap: 10px; }
  .disclose { display: flex; align-items: center; gap: 8px; background: none; border: 0; padding: 4px 0; font-size: 13px; color: var(--text-muted); text-align: left; }
  .disclose:hover { color: var(--text); }
  .chev { display: inline-block; transition: transform 0.12s; font-size: 10px; }
  .chev.open { transform: rotate(90deg); }
  .source-head { font-family: var(--mono); font-size: 11px; color: var(--accent-700); margin-bottom: 6px; }
  .quoted { display: flex; gap: 12px; }
  .gutter { width: 2px; background: var(--hazard); flex-shrink: 0; border-radius: 1px; }
  .qtext { min-width: 0; flex: 1; max-height: 340px; overflow-y: auto; }
  .answering { display: flex; flex-direction: column; gap: 10px; }
  .answering .kicker { margin-bottom: 0; }
  .msg { border-left: 2px solid rgba(224, 164, 88, 0.55); padding: 2px 0 2px 14px; display: flex; flex-direction: column; gap: 8px; max-height: 360px; overflow-y: auto; }
  .msg.target { border-left-color: var(--hazard); }
  .msghead { display: flex; align-items: baseline; gap: 8px; flex-wrap: wrap; }
  .who { font-size: 14px; font-weight: 600; color: var(--text); }
  .addr { font-family: var(--mono); font-size: 11px; color: var(--text-muted); overflow-wrap: anywhere; }
  .answerlinks { display: flex; gap: 14px; flex-wrap: wrap; }
  .answerlinks .linkish { font-size: 11px; }
  .rawread { white-space: pre-wrap; overflow-wrap: anywhere; }
  .serves { font-size: 12px; color: var(--text-muted); line-height: 1.45; }
  .provenance { display: flex; gap: 6px; flex-wrap: wrap; font-family: var(--mono); font-size: 10px; color: var(--accent-700); }
  .linkish { background: none; border: 0; padding: 0; color: var(--text-muted); font-family: var(--mono); font-size: 10px; text-decoration: underline; text-underline-offset: 2px; }
  .argdump { background: var(--void); border: 1px solid #2a2a38; border-radius: var(--radius); padding: 12px; font-family: var(--mono); font-size: 11px; line-height: 1.5; overflow: auto; max-height: 260px; margin: 0; color: var(--text); }

  /* ---- the decision bar ---- */
  .bar { flex-shrink: 0; display: flex; gap: 8px; flex-wrap: wrap; align-items: center; padding: 12px var(--gutter) calc(12px + env(safe-area-inset-bottom, 0px)); border-top: 1px solid #2a2a38; background: var(--bg); }
  .wide .bar { padding: 12px max(28px, calc((100% - 820px) / 2)); }
  .btn { min-height: 40px; padding: 0 14px; display: inline-flex; align-items: center; justify-content: center; background: var(--surface); border: 1px solid #2a2a38; border-radius: var(--radius-chip); font-size: 14px; }
  .btn:hover:not(:disabled) { border-color: var(--accent-700); }
  .btn:disabled { opacity: 0.45; cursor: default; }
  .btn.primary { background: var(--accent-500); border-color: var(--accent-500); color: var(--void); font-weight: 600; }
  .btn.primary:hover:not(:disabled) { background: var(--accent-400); }
  .btn.primary kbd { color: var(--void); border-color: rgba(18, 20, 31, 0.3); }
  .btn.big { flex: 1; min-width: 200px; }
  .btn.danger { background: #d4526e; border-color: #d4526e; color: var(--void); font-weight: 600; }
  .btnrow { display: flex; gap: 8px; align-items: center; justify-content: flex-end; }
  .barerr { width: 100%; display: flex; gap: 8px; align-items: flex-start; font-size: 13px; line-height: 1.45; color: var(--hazard); overflow-wrap: anywhere; }
  .btnrow.start { justify-content: flex-start; }
  .newmail { display: flex; gap: 8px; align-items: flex-start; padding: 9px 12px; border: 1px solid var(--hazard); border-radius: var(--radius); font-size: 13px; line-height: 1.45; color: var(--text-muted); }
  .newmail strong { color: var(--hazard); font-weight: 600; }
  .hint.ok { color: #7fc4b8; }
  .notice { margin: 0 18px 10px; padding: 7px 10px; border-radius: var(--radius-chip); background: rgba(127, 196, 184, 0.1); color: #7fc4b8; font-size: 12px; overflow-wrap: anywhere; }
  .rejectbox { width: 100%; display: flex; flex-direction: column; gap: 8px; }
  .reasons { display: flex; gap: 8px; flex-wrap: wrap; }
  .reason { flex: 1; min-width: 150px; }
  .rejectrow { width: 100%; display: flex; gap: 8px; flex-wrap: wrap; }
  .rwho { font-size: 14px; font-weight: 600; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .stuck { font-family: var(--mono); font-size: 10px; padding: 1px 6px; border-radius: 4px; background: #3a2e1a; color: var(--hazard); white-space: nowrap; }
  .diff { display: flex; flex-wrap: wrap; align-items: baseline; gap: 10px; font-size: 15px; }
  .diff del { color: #d4526e; text-decoration: line-through; overflow-wrap: anywhere; }
  .diff ins { color: #7fc4b8; text-decoration: none; overflow-wrap: anywhere; }
  .arrow { color: var(--text-muted); }
  .openlink { font-size: 12px; color: var(--accent-400); }
  .rejectrow input { flex: 1; min-width: 220px; }
  .pad.btnrow { padding: 10px 16px 14px; }

  @media (max-width: 999px) {
    .btn { min-height: 46px; }
    .bar .btn:not(.big) { flex: 1; }
  }
</style>
