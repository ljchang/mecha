<script>
  import { apiFetch as fetch } from './api.js';
  import MailBody from './MailBody.svelte';
  import {
    kindOf, KINDS, editsAsEvent, eventFields, eventArgs, inclusiveEnd, whenLabel, eventZone,
    attendeesOf, MAIL_HEADERS, ago, localZone, EVENT_CARD_KEYS,
  } from './outbox-view.js';

  // The outbox: every draft waiting on the owner, and the one place any of
  // them goes out. The page renders the whole reviewable object — taint
  // warning, headers, prose, everything-else, and the quoted source the draft
  // answers — because approving without reading is the failure this queue
  // exists to prevent. Every action drives a `mecha outbox …` verb on the
  // box; the confirm step is what earns `--yes`.
  //
  // Each draft is shown as the thing it is. A mail reads as a mail (headers,
  // then the letter, rendered); an event reads as a time, a place, a calendar
  // and who gets invited, and edits as fields — a start time or a calendar is
  // not prose, and JSON in a terminal was the only way to change one. At a
  // desk the list and the open draft sit side by side and the keys move
  // through them, as in Mail; on a phone it is a list, then a draft.
  //
  // What "reviewed" means did not move: the taint notice, the exact arguments
  // on an armed draft's confirm, the source reads, the reason on a reject.

  let pending = $state([]);
  let resolved = $state(0);
  let loaded = $state(false);
  let detail = $state(null);
  let error = $state(null); // an action's failure: stays until the next action
  let listError = $state(null); // the list poll's own, cleared by the next poll
  let busy = $state(false);
  let filter = $state('all');
  let selectedId = $state(null);

  // What the draft pane is doing.
  let mode = $state('read'); // read | prose | event | confirm | reject
  let editDraft = $state('');
  let ev = $state(null); // the event editor's fields
  let evError = $state(null);
  let rejectReason = $state('');
  let showSources = $state(false);
  let showArgs = $state(false);
  let deliveryEvidence = $state('');
  let deliveryOutcome = $state('delivered');
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
  const restHeaders = $derived((detail?.headers ?? []).filter(([k]) => !MAIL_HEADERS.includes(k)));
  const invited = $derived(attendeesOf(args));
  const unshown = $derived(
    asEvent
      ? [...(detail?.headers ?? []), ...(detail?.other ?? [])].filter(([k]) => !EVENT_CARD_KEYS.includes(k))
      : (detail?.other ?? []),
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
      // At a desk the pane is never empty while there is something to read.
      if (wide && !selectedId && visible.length) open(visible[0].id);
    } catch (e) {
      listError = String(e?.message ?? e);
    }
  }

  async function open(id) {
    selectedId = id;
    try {
      const res = await fetch(`/api/outbox/${id}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      const d = await res.json();
      if (selectedId !== id) return; // a later click won
      detail = d;
      mode = 'read';
      evError = null;
      // Open, as the page has always shown them: what a draft answers is part
      // of reading it. The toggle is for a long thread already read.
      showSources = true;
      showArgs = false;
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
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

  async function act(path, body) {
    busy = true;
    try {
      const res = await fetch(`/api/outbox/${detail.id}/${path}`, {
        method: 'POST',
        headers: body ? { 'content-type': 'application/json' } : {},
        body: body ? JSON.stringify(body) : undefined,
      });
      const text = await res.text();
      if (!res.ok) throw new Error(text.trim());
      return true;
    } catch (e) {
      error = String(e?.message ?? e);
      return false;
    } finally {
      busy = false;
    }
  }

  async function reconcile() {
    if (await act('reconcile', { outcome: deliveryOutcome, evidence: deliveryEvidence.trim() })) {
      deliveryEvidence = '';
      next();
    }
  }
  // The confirm step earns its place on an armed draft, where it shows the
  // exact arguments — more than the pane does. On a clean one it would show
  // strictly less than what is already open, which is a confirmation that
  // teaches people to click through, and what that trains away is the armed
  // one. So: one step here, two when the trifecta was armed.
  function approveClicked() {
    if (!detail || busy || detail.delivery_uncertain) return;
    if (detail.taint.armed) mode = 'confirm';
    else approve();
  }
  async function approve() {
    if (await act('approve')) next();
  }
  function startReject() {
    mode = 'reject';
    queueMicrotask(() => reasonEl?.focus());
  }
  async function reject() {
    if (!rejectReason.trim()) return;
    if (await act('reject', { reason: rejectReason.trim() })) {
      rejectReason = '';
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
  async function saveProse() {
    if (await act('edit', { body: editDraft })) open(detail.id);
  }
  async function saveEvent() {
    const out = eventArgs(detail.args, ev);
    if (out.error) {
      evError = out.error;
      return;
    }
    if (await act('edit', { args: out.args })) open(detail.id);
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
      : kind === 'mail' ? 'Approve and send'
      : asEvent ? (invited.length ? 'Approve — add and invite' : 'Approve — add to calendar')
      : kind === 'doc' ? 'Approve the edit'
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
    // A held key repeats. `a` flips an armed draft into its confirm step, and
    // the next repeat would press "Send it" — the confirm has to be a second,
    // deliberate press, and none of these keys can be undone (found on review).
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
    if ((e.key === 'Enter' || e.key === ' ') && t?.closest?.('button, a, select, label')) return;
    switch (e.key) {
      case 'j': case 'ArrowDown': if (mode === 'read') move(1); else return; break;
      case 'k': case 'ArrowUp': if (mode === 'read') move(-1); else return; break;
      case 'a': if (mode === 'read') approveClicked(); else if (mode === 'confirm') approve(); else return; break;
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
          <button class="row" class:on={item.id === selectedId} onclick={() => open(item.id)}>
            <span class="kicon k-{kindOf(item.tool)}">{@render kindGlyph(kindOf(item.tool))}</span>
            <span class="rbody">
              <span class="rtop">
                <span class="rlabel">{item.label}</span>
                {#if item.account}<span class="acct">{item.account}</span>{/if}
                <span class="grow"></span>
                {#if item.tainted}<span class="tflag" title="Drafted with untrusted content in context">{@render hazardGlyph(12)}</span>{/if}
                <span class="when">{ago(item.created_at)}</span>
              </span>
              {#if item.headline}<span class="rhead">{item.headline}</span>{/if}
              {#if rowWhen(item)}<span class="rsnip strong">{rowWhen(item)}</span>
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
        <footer class="keys"><span><kbd>j</kbd><kbd>k</kbd> move</span><span><kbd>a</kbd> approve</span><span><kbd>e</kbd> edit</span><span><kbd>x</kbd> reject</span><span><kbd>esc</kbd> cancel</span></footer>
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
        <h1>{asEvent ? (args.title ?? detail.headline) : detail.headline || detail.label}</h1>

        {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}

        {#if detail.taint.armed}
          <div class="taint">
            {@render hazardGlyph(15)}
            <div><strong>Drafted with untrusted content in context.</strong> If anything here was not yours — a recipient, a link, a time — an attacker may have put it there. Read all of it.</div>
          </div>
        {/if}

        {#if detail.error}
          <!-- A failed send stays pending: the draft is good, the delivery was
               not. Saying which is the difference between an item somebody
               can fix and an item that sits in the queue being clicked at. -->
          <div class="failline">
            {@render hazardGlyph()}
            <div>
              <div class="failhead">{detail.delivery_uncertain ? 'Delivery outcome is unknown' : 'The last attempt needs attention'}</div>
              <div class="failwhy">{detail.error}</div>
            </div>
          </div>
        {/if}

        {#if detail.delivery_uncertain}
          <div class="card pad col">
            <p class="note">Check the destination's sent history before deciding. Recording the outcome does not send anything.</p>
            <label class="field">What did you find?
              <select bind:value={deliveryOutcome}>
                <option value="delivered">Confirmed delivered</option>
                <option value="not-delivered">Confirmed not delivered</option>
              </select>
            </label>
            <label class="field">Evidence
              <textarea rows="3" bind:value={deliveryEvidence} placeholder="Message or event ID, or the destination check that established it was not delivered"></textarea>
            </label>
            <div><button class="btn primary" disabled={busy || !deliveryEvidence.trim()} onclick={reconcile}>Record outcome</button></div>
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
                    <optgroup label={a.account}>
                      {#each (a.calendars ?? []).filter(writable) as c}
                        <option value={calKey(a.account, c.id)}>{c.name}{c.is_primary ? ' (primary)' : ''}</option>
                      {/each}
                    </optgroup>
                  {/each}
                </select>
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
              <button class="btn primary" type="submit" disabled={busy}>Save changes</button>
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
          <div class="card letter">
            {#if mailHeaders.length}
              <div class="lhead">
                {#each mailHeaders as [key, value]}
                  <div class="hrow"><span class="hkey">{key}</span><span class="hval">{value}</span></div>
                {/each}
              </div>
            {/if}
            {#if mode === 'prose'}
              <textarea class="prosebox" bind:this={proseEl} bind:value={editDraft} rows="14"></textarea>
              <div class="btnrow pad">
                <span class="hint grow">Markdown — **bold**, [link](https://…), lists. Converted when it sends.</span>
                <button class="btn" onclick={() => (mode = 'read')}>Cancel{#if wide}<kbd>esc</kbd>{/if}</button>
                <button class="btn primary" disabled={busy} onclick={saveProse}>Save edit</button>
              </div>
            {:else if detail.body}
              <div class="lbody"><MailBody text={detail.body} /></div>
            {/if}
          </div>
          {#if restHeaders.length}
            <div class="card pad hgrid">
              {#each restHeaders as [key, value]}<div class="hrow"><span class="hkey">{key}</span><span class="hval">{value}</span></div>{/each}
            </div>
          {/if}
        {:else}
          {#if detail.headers?.length}
            <div class="card pad hgrid">
              {#each detail.headers as [key, value]}<div class="hrow"><span class="hkey">{key}</span><span class="hval">{value}</span></div>{/each}
            </div>
          {/if}
          {#if mode === 'prose'}
            <textarea class="prosebox card" bind:this={proseEl} bind:value={editDraft} rows="14"></textarea>
            <div class="btnrow">
              <button class="btn" onclick={() => (mode = 'read')}>Cancel{#if wide}<kbd>esc</kbd>{/if}</button>
              <button class="btn primary" disabled={busy} onclick={saveProse}>Save edit</button>
            </div>
          {:else if detail.body}
            <div class="card pad"><MailBody text={detail.body} /></div>
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

        {#if detail.sources?.length}
          <div class="sources">
            <button class="disclose" onclick={() => (showSources = !showSources)} aria-expanded={showSources}>
              <span class="chev" class:open={showSources}>▸</span>
              What this answers
              <span class="muted mono">{detail.sources.length === 1 ? 'the thread it read' : `${detail.sources.length} reads`}</span>
            </button>
            {#if showSources}
              {#each detail.sources as source}
                <div class="source">
                  <div class="source-head">{source.heading ?? `${source.tool} · ${source.keys.join(', ')}`}</div>
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
        {#if showArgs && mode !== 'confirm'}<pre class="argdump">{JSON.stringify(detail.args, null, 2)}</pre>{/if}
      </div>

      {#if !detail.delivery_uncertain && mode !== 'event' && mode !== 'prose'}
        <div class="bar">
          {#if mode === 'confirm'}
            <div class="confirm">
              <div class="warnline">{@render hazardGlyph()}<span>This draft was written while the trifecta was armed. These are the exact arguments that will be sent:</span></div>
              <pre class="argdump">{JSON.stringify(detail.args, null, 2)}</pre>
              <div class="btnrow">
                <button class="btn" onclick={() => (mode = 'read')}>Back{#if wide}<kbd>esc</kbd>{/if}</button>
                <button class="btn primary" disabled={busy} onclick={approve}>{busy ? 'sending…' : 'Send it'}{#if wide}<kbd>a</kbd>{/if}</button>
              </div>
            </div>
          {:else if mode === 'reject'}
            <form class="rejectrow" onsubmit={(e) => { e.preventDefault(); reject(); }}>
              <input bind:this={reasonEl} bind:value={rejectReason} placeholder="Why? Recorded on the item — e.g. wrong calendar, not needed" aria-label="Reason" />
              <button class="btn" type="button" onclick={() => (mode = 'read')}>Cancel</button>
              <button class="btn danger" type="submit" disabled={busy || !rejectReason.trim()}>Reject</button>
            </form>
          {:else if mode === 'read'}
            <button class="btn primary big" disabled={busy} onclick={approveClicked}>
              {busy ? 'sending…' : detail.taint.armed ? `${approveLabel} · confirms first` : approveLabel}{#if wide}<kbd>a</kbd>{/if}
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
  .taint { display: flex; gap: 10px; align-items: flex-start; padding: 10px 12px; border-radius: var(--radius); background: rgba(224, 164, 88, 0.08); border: 1px solid rgba(224, 164, 88, 0.3); font-size: 13px; line-height: 1.45; color: var(--text-muted); }
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

  .sources { display: flex; flex-direction: column; gap: 10px; }
  .disclose { display: flex; align-items: center; gap: 8px; background: none; border: 0; padding: 4px 0; font-size: 13px; color: var(--text-muted); text-align: left; }
  .disclose:hover { color: var(--text); }
  .chev { display: inline-block; transition: transform 0.12s; font-size: 10px; }
  .chev.open { transform: rotate(90deg); }
  .source-head { font-family: var(--mono); font-size: 11px; color: var(--accent-700); margin-bottom: 6px; }
  .quoted { display: flex; gap: 12px; }
  .gutter { width: 2px; background: var(--hazard); flex-shrink: 0; border-radius: 1px; }
  .qtext { min-width: 0; flex: 1; }
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
  .confirm { width: 100%; display: flex; flex-direction: column; gap: 10px; }
  .rejectrow { width: 100%; display: flex; gap: 8px; flex-wrap: wrap; }
  .rejectrow input { flex: 1; min-width: 220px; }
  .pad.btnrow { padding: 10px 16px 14px; }

  @media (max-width: 999px) {
    .btn { min-height: 46px; }
    .bar .btn:not(.big) { flex: 1; }
  }
</style>
