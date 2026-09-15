<script>
  import { apiFetch as fetch } from './api.js';
  // The front door on the phone — strangers' requests, worked from the
  // couch. The list is typed fields plus the extraction's own summary; the
  // raw prose opens only on tap, marked as third-party text with the
  // gutter. Every action drives a `mecha frontdoor …` verb: extract is the
  // quarantined pass, triage is a whole agent run whose drafts land in the
  // outbox, close requires a reason because silence is the failure mode
  // this component exists to fix.
  let rows = $state(null);
  let reading = $state(null); // { row, text }
  let error = $state(null);
  let busy = $state(false);
  let toast = $state(null);
  let asking = $state(null); // { verb, label, placeholder, required }
  let askText = $state('');

  async function load() {
    try {
      const res = await fetch('/api/frontdoor');
      if (!res.ok) throw new Error((await res.text()).trim());
      rows = (await res.json()).requests;
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }
  load();

  async function open(row) {
    reading = { row, text: null };
    try {
      const res = await fetch(`/api/frontdoor/read?seq=${row.seq}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      reading = { row, text: await res.text() };
    } catch (e) {
      error = String(e?.message ?? e);
      reading = null;
    }
  }

  function back() {
    reading = null;
    asking = null;
    load();
  }

  async function act(verb, row, extra = {}) {
    busy = true;
    try {
      const res = await fetch('/api/frontdoor/act', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ verb, seq: row.seq, ...extra }),
      });
      const text = await res.text();
      if (!res.ok) throw new Error(text.trim());
      const out = JSON.parse(text);
      toast = out.detached ? out.note : `${verb} ✓`;
      setTimeout(() => (toast = null), 4000);
      error = null;
      return true;
    } catch (e) {
      error = String(e?.message ?? e);
      return false;
    } finally {
      busy = false;
    }
  }

  function prompt(verb, label, placeholder, required = false) {
    asking = { verb, label, placeholder, required };
    askText = '';
  }

  async function submitPrompt() {
    if (asking.required && !askText.trim()) return;
    const extra = askText.trim() ? { text: askText.trim() } : {};
    if (await act(asking.verb, reading.row, extra)) back();
  }

  const stateChip = (s) =>
    ({ drained: 'new', extraction_failed: 'extraction failed', booked: 'on your calendar' })[s] ??
    s.replaceAll('_', ' ');

  // The meeting in the *viewer's* zone. This page is only ever read by its
  // owner on a device that knows where it is, so the browser is the right
  // clock — the terminal renderer uses `[agent] timezone` instead, because a
  // server has no way to know.
  const span = (b) => {
    const start = new Date(b.start);
    const end = new Date(b.end);
    if (isNaN(start) || isNaN(end)) return `${b.start} – ${b.end}`;
    const day = start.toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' });
    const t = (d) => d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
    return `${day} · ${t(start)} – ${t(end)}`;
  };

  const isPast = (b) => {
    const end = new Date(b.end);
    return !isNaN(end) && end < new Date();
  };

  // Settled bookings are not review work — they are a confirmed meeting the
  // machinery already answered. Kept on the page (the record is the archive)
  // but out of the queue, because a queue that lists finished work stops
  // reading as a queue.
  // `state === 'booked'`, not "is this a booking": a booking somebody later
  // closed by hand with a reason is not filed under "nothing owed". The
  // terminal renderer gates the same sentence on the same state.
  // `&& r.booking` is not reachable through the code paths that exist — only
  // `settle_bookings` writes `booked`, and it requires a parseable booking —
  // but a hand-edited or future-written record would take the whole page down
  // on the dereference below rather than degrade. It costs nothing.
  const settled = (r) => r.state === 'booked' && r.booking;
  const queue = $derived((rows ?? []).filter((r) => !settled(r)));
  const booked = $derived(
    (rows ?? [])
      .filter(settled)
      .sort((a, b) => new Date(b.booking.start) - new Date(a.booking.start)),
  );
  let showBooked = $state(false);
</script>

{#snippet hazardGlyph(size = 13)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

<div class="page">
  {#if !reading}
    <div class="scroll">
      {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}
      {#if rows === null && !error}
        <div class="empty">reading the requests…</div>
      {:else}
        {#each queue as r}
          <button class="card rowbtn" onclick={() => open(r)}>
            <div class="rowtop">
              <span class="chip">{r.type_id}</span>
              <span class="chip" class:hazard={r.state === 'extraction_failed'}>{stateChip(r.state)}</span>
              {#if !r.valid}<span class="chip hazard">invalid</span>{/if}
              <span class="when">#{r.seq} · {(r.created_at ?? '').slice(0, 10)}</span>
            </div>
            {#if r.booking}
              <!-- A booking leads with the meeting. Reading the slot out of a
                   column of `_`-prefixed machinery is how a confirmed meeting
                   came to look like an unanswered question. -->
              <div class="topic" class:past={isPast(r.booking)}>{span(r.booking)}</div>
              <div class="meta">
                {#if r.reply_to}<span>{r.reply_to}</span>{/if}
                {#if r.booking.duration_minutes}<span>{r.booking.duration_minutes} min</span>{/if}
                {#if isPast(r.booking)}<span>already happened</span>{/if}
              </div>
            {:else}
              {#if r.topic}<div class="topic">{r.topic}</div>{/if}
              {#if r.reading}<div class="readingline">{r.reading}</div>{/if}
              {#if r.urgency_claimed}<div class="claim">claims: {r.urgency_claimed}</div>{/if}
            {/if}
            {#if r.extraction_error}<div class="claim hazardtext">{r.extraction_error}</div>{/if}
          </button>
        {:else}
          <div class="empty">Nobody is waiting at the door.</div>
        {/each}

        {#if booked.length}
          <button class="foldrow" onclick={() => (showBooked = !showBooked)}>
            <span>{booked.length} confirmed booking{booked.length === 1 ? '' : 's'}</span>
            <span class="foldnote">on your calendar · nothing owed</span>
            <span class="chev" class:open={showBooked}>›</span>
          </button>
          {#if showBooked}
            {#each booked as r}
              <button class="card rowbtn muted" onclick={() => open(r)}>
                <div class="rowtop">
                  <span class="chip">{r.type_id}</span>
                  <span class="chip">{stateChip(r.state)}</span>
                  <span class="when">#{r.seq}</span>
                </div>
                <div class="topic" class:past={isPast(r.booking)}>{span(r.booking)}</div>
                <div class="meta">
                  {#if r.reply_to}<span>{r.reply_to}</span>{/if}
                  {#if r.booking.duration_minutes}<span>{r.booking.duration_minutes} min</span>{/if}
                </div>
              </button>
            {/each}
          {/if}
        {/if}
      {/if}
    </div>
  {:else}
    <div class="scroll">
      <div class="deckhead">
        <button class="backbtn" onclick={back} aria-label="back">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6l-6 6 6 6" /></svg>
        </button>
        <span class="dtitle">#{reading.row.seq} · {reading.row.type_id}</span>
      </div>
      {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}
      {#if reading.text === null}
        <div class="empty">reading the request…</div>
      {:else}
        <!-- A stranger's words: the per-line gutter, exactly as mail bodies
             and outbox sources are marked. Reading it here is the safe
             context; no run ever sees these bytes. -->
        <div class="quoted"><span class="gutter"></span><div class="qtext">{reading.text}</div></div>
      {/if}
    </div>
    {#if reading.text !== null}
      <div class="actionbar">
        <!-- A settled booking gets no draft action at all. It briefly had a
             "Reply anyway…" button, which could not work: `frontdoor triage`
             selects on `state == EXTRACTED` and a settled booking is never
             extracted, so the detached child printed `nothing to triage` and
             the page reported success for a draft that was never coming.
             Proposing another time needs the decline path, which does not
             exist yet — a dead button is worse than an absent one. -->
        {#if settled(reading.row)}
          <!-- nothing to draft; Close… below is the real action -->
        {:else if ['drained', 'extraction_failed'].includes(reading.row.state)}
          <button class="abtn primary" disabled={busy} onclick={async () => { if (await act('extract', reading.row)) back(); }}>Extract</button>
        {:else}
          <button class="abtn primary" disabled={busy} onclick={async () => { if (await act('triage', reading.row)) back(); }}>Draft a reply…</button>
        {/if}
        {#if !settled(reading.row)}
          <button class="abtn" disabled={busy} onclick={() => prompt('needs-info', 'What is missing before this can proceed?', 'which dates they need')}>Park…</button>
        {/if}
        <button class="abtn" disabled={busy} onclick={() => prompt('close', 'Why? The reason is the record.', 'out of scope — not taking new students', true)}>Close…</button>
      </div>
      {#if settled(reading.row)}
        <div class="barnote">Confirmed at the gate and on your calendar — the invite went from your own mailbox. Nothing here is waiting on you.</div>
      {:else}
        <div class="barnote">Drafted replies land in the outbox for review — nothing sends from here.</div>
      {/if}
    {/if}

    {#if asking}
      <div class="sheet">
        <div class="sheet-grip"></div>
        <div class="sheet-text">{asking.label}</div>
        <textarea class="editbox" rows="3" bind:value={askText} placeholder={asking.placeholder}></textarea>
        <div class="btnrow">
          <button class="abtn" onclick={() => (asking = null)}>Back</button>
          <button class="abtn primary" disabled={busy || (asking.required && !askText.trim())} onclick={submitPrompt}>Go</button>
        </div>
      </div>
    {/if}
  {/if}

  {#if toast}<div class="toast">{toast}</div>{/if}
</div>

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  .scroll { flex: 1; overflow-y: auto; padding: 14px var(--gutter); display: flex; flex-direction: column; gap: 10px; }
  .scroll > * { flex-shrink: 0; }
  .rowbtn { text-align: left; padding: 14px; display: flex; flex-direction: column; gap: 6px; cursor: pointer; color: var(--text); font: inherit; overflow: hidden; }
  .rowtop { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .chip.hazard, .hazardtext { color: var(--hazard); }
  .when { font-family: var(--mono); font-size: 10px; color: var(--accent-700); margin-left: auto; }
  .topic { font-size: 14px; font-weight: 500; line-height: 1.35; overflow-wrap: anywhere; }
  .readingline { font-size: 12px; line-height: 1.45; color: var(--text-muted); overflow: hidden; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; }
  .claim { font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .meta { display: flex; flex-wrap: wrap; gap: 4px 10px; font-family: var(--mono); font-size: 10px; color: var(--text-muted); overflow-wrap: anywhere; }
  .topic.past { color: var(--text-muted); }
  .rowbtn.muted { opacity: 0.72; }
  .foldrow { display: flex; align-items: center; gap: 8px; min-height: 44px; padding: 0 4px; background: none; border: none; border-top: 1px solid var(--accent-900); color: var(--text-muted); font: inherit; font-size: 12px; cursor: pointer; text-align: left; margin-top: 4px; }
  .foldnote { font-family: var(--mono); font-size: 10px; color: var(--accent-700); }
  .chev { margin-left: auto; transition: transform 120ms ease; }
  .chev.open { transform: rotate(90deg); }
  .empty { color: var(--text-muted); font-size: 14px; padding: 24px 0; text-align: center; }
  .warnline { display: flex; align-items: flex-start; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .deckhead { display: flex; align-items: center; gap: 8px; }
  .backbtn { background: none; border: none; color: var(--text-muted); min-width: 44px; min-height: 44px; margin: -10px 0 -10px -12px; cursor: pointer; display: flex; align-items: center; justify-content: center; }
  .dtitle { font-family: var(--mono); font-size: 13px; color: var(--accent-400); }
  .quoted { display: flex; gap: 10px; }
  .gutter { width: 2px; background: var(--hazard); flex-shrink: 0; }
  .qtext { font-size: 14px; line-height: 1.55; color: var(--text); white-space: pre-wrap; overflow-wrap: anywhere; }
  .actionbar { display: flex; gap: 8px; overflow-x: auto; padding: 10px 20px 4px; border-top: 1px solid var(--accent-900); background: var(--bg); }
  .abtn { flex-shrink: 0; min-height: 44px; padding: 0 16px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; white-space: nowrap; }
  .abtn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .abtn:disabled { opacity: 0.5; }
  .barnote { font-size: 10px; color: var(--text-muted); text-align: center; padding: 4px 20px 8px; background: var(--bg); }
  .btnrow { display: flex; gap: 10px; }
  .btnrow .abtn { flex: 1; }
  .editbox { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; line-height: 1.5; padding: 12px 14px; resize: vertical; box-sizing: border-box; }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px var(--gutter) 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; }
  .toast { position: absolute; bottom: 18px; left: 50%; transform: translateX(-50%); background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 10px 16px; font-size: 13px; white-space: nowrap; max-width: 90%; overflow: hidden; text-overflow: ellipsis; }
</style>
