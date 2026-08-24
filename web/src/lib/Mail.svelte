<script>
  // The mail queue on the phone — the TUI /mail modal's shape: the list is a
  // store read, the reader is `mecha mail show`'s exact text (one renderer
  // of a thread), and every action drives a `mecha mail …` verb through
  // /api/mail/act. A reply never sends from here: the drafting verbs stage
  // into the outbox, which is the one approval surface.
  //
  // Spam is the one action that confirms — it trains the provider's filter,
  // so it is the only triage verb whose effect leaves the mailbox. Archive
  // and the rest are one tap: reversible, private, and a confirmation on a
  // reversible private change teaches people to tap without reading.
  let rows = $state(null);
  let showParked = $state(false);
  let reading = $state(null); // { row, text } — the open thread
  let error = $state(null);
  let busy = $state(false);
  let toast = $state(null);
  let confirmSpam = $state(null); // row awaiting the spam confirm sheet
  let asking = $state(null); // { verb, label, placeholder, wantTo } — text prompt sheet
  let askText = $state('');
  let askTo = $state('');

  async function load() {
    try {
      const res = await fetch('/api/mail');
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${(await res.text()).trim()}`);
      rows = await res.json();
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  async function open(row) {
    reading = { row, text: null };
    try {
      const q = new URLSearchParams({ thread: row.thread_id, account: row.account });
      const res = await fetch(`/api/mail/read?${q}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      reading = { row, text: await res.text() };
    } catch (e) {
      error = String(e?.message ?? e);
      reading = null;
    }
  }

  function back() {
    reading = null;
    confirmSpam = null;
    asking = null;
    load();
  }

  async function act(verb, row, extra = {}) {
    busy = true;
    try {
      const res = await fetch('/api/mail/act', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ verb, thread: row.thread_id, account: row.account, ...extra }),
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

  async function quick(verb, row) {
    if (await act(verb, row)) back();
  }

  function prompt(verb, label, placeholder, wantTo = false) {
    asking = { verb, label, placeholder, wantTo };
    askText = '';
    askTo = '';
  }

  async function submitPrompt() {
    const row = reading.row;
    const extra = {};
    if (asking.wantTo) {
      if (!askTo.trim()) return;
      extra.to = askTo.trim();
    }
    if (askText.trim()) extra.text = askText.trim();
    if (asking.verb === 'needs-info' && !askText.trim()) return;
    if (await act(asking.verb, row, extra)) back();
  }

  load();

  const urgencyRank = { now: 'now', today: 'today', week: 'week' };
  const needs = $derived((rows ?? []).filter((r) => r.needs_me));
  const parked = $derived((rows ?? []).filter((r) => !r.needs_me));
</script>

{#snippet hazardGlyph(size = 13)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

{#snippet mailRow(r)}
  <button class="card rowbtn" onclick={() => open(r)}>
    <div class="rowtop">
      {#if r.urgency && urgencyRank[r.urgency]}<span class="chip urgency-{r.urgency}">{r.urgency}</span>{/if}
      {#if r.state === 'failed'}<span class="chip failed">classification failed</span>{/if}
      {#if r.state === 'parked'}<span class="chip">parked</span>{/if}
      {#if r.deadline}<span class="deadline">due {r.deadline}</span>{/if}
      <span class="acct">{r.account}</span>
    </div>
    <div class="summary">{r.summary}</div>
    <div class="rowfoot">
      <span class="from">{r.from}</span>
      {#each r.tags as t}<span class="tag">#{t}</span>{/each}
    </div>
  </button>
{/snippet}

<div class="page">
  {#if !reading}
    <header>
      <span class="title">Mail</span>
      <span class="chip">{needs.length} need you · {parked.length} parked</span>
    </header>
    <div class="scroll">
      {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}
      {#if rows === null && !error}
        <div class="empty">reading the queue…</div>
      {:else}
        {#each needs as r}{@render mailRow(r)}{:else}
          <div class="empty">Nothing needs you.</div>
        {/each}
        {#if parked.length}
          <button class="ghost" onclick={() => (showParked = !showParked)}>
            {showParked ? 'hide' : 'show'} {parked.length} handled / parked
          </button>
          {#if showParked}
            {#each parked as r}{@render mailRow(r)}{/each}
          {/if}
        {/if}
      {/if}
    </div>
  {:else}
    <header>
      <button class="backbtn" onclick={back} aria-label="back">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6l-6 6 6 6" /></svg>
      </button>
      <span class="title">{reading.row.summary}</span>
    </header>
    <div class="scroll">
      {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}
      {#if reading.text === null}
        <div class="empty">reading thread…</div>
      {:else}
        <!-- Third-party text: the gutter marks every line, the outbox
             source-read rule — a heading scrolls off, a per-line marker
             cannot. -->
        <div class="quoted"><span class="gutter"></span><div class="qtext">{reading.text}</div></div>
      {/if}

      <div class="btngrid">
        <button class="btn" disabled={busy} onclick={() => quick('archive', reading.row)}>Archive</button>
        <button class="btn" disabled={busy} onclick={() => (confirmSpam = reading.row)}>Spam…</button>
        <button class="btn" disabled={busy} onclick={() => quick('task', reading.row)}>→ Task</button>
        <button class="btn" disabled={busy} onclick={() => quick('dismiss', reading.row)}>Dismiss</button>
        <button class="btn" disabled={busy} onclick={() => prompt('needs-info', 'What are you waiting for?', 'their dates, before I can book')}>Park…</button>
        <button class="btn" disabled={busy} onclick={() => prompt('schedule', 'Steering for the calendar draft (optional)', 'propose Thursday afternoon')}>Calendar…</button>
        <button class="btn" disabled={busy} onclick={() => prompt('forward', 'Forward to (comma-separated) + covering note', 'FYI — this is the one I mentioned', true)}>Forward…</button>
        <button class="btn primary" disabled={busy} onclick={() => prompt('reply', 'Steering for the draft (optional)', 'decline politely; ask for the deadline')}>Draft reply…</button>
      </div>
      <div class="footnote">Drafts land in the outbox for review — nothing sends from here.</div>
    </div>

    {#if confirmSpam}
      <div class="sheet">
        <div class="sheet-grip"></div>
        <div class="warnline">{@render hazardGlyph()}<span>Spam trains the provider's filter — the one triage action with an effect outside your mailbox.</span></div>
        <div class="sheet-sub">{confirmSpam.from} · {confirmSpam.summary}</div>
        <div class="btnrow">
          <button class="btn" onclick={() => (confirmSpam = null)}>Back</button>
          <button class="btn primary" disabled={busy} onclick={async () => { const r = confirmSpam; confirmSpam = null; await quick('spam', r); }}>Mark spam</button>
        </div>
      </div>
    {/if}

    {#if asking}
      <div class="sheet">
        <div class="sheet-grip"></div>
        <div class="sheet-text">{asking.label}</div>
        {#if asking.wantTo}
          <input class="editline" bind:value={askTo} placeholder="who@example.edu, other@example.com" />
        {/if}
        <textarea class="editbox" rows="3" bind:value={askText} placeholder={asking.placeholder}></textarea>
        <div class="btnrow">
          <button class="btn" onclick={() => (asking = null)}>Back</button>
          <button
            class="btn primary"
            disabled={busy || (asking.wantTo && !askTo.trim()) || (asking.verb === 'needs-info' && !askText.trim())}
            onclick={submitPrompt}
          >{asking.verb === 'needs-info' ? 'Park it' : 'Go'}</button>
        </div>
      </div>
    {/if}
  {/if}

  {#if toast}<div class="toast">{toast}</div>{/if}
</div>

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  header { display: flex; align-items: center; gap: 10px; padding: 14px 20px 12px; border-bottom: 1px solid var(--accent-900); }
  header .title { font-weight: 500; font-size: 17px; letter-spacing: -0.02em; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .backbtn { background: none; border: none; color: var(--text-muted); min-width: 44px; min-height: 44px; margin: -12px 0 -12px -12px; cursor: pointer; display: flex; align-items: center; justify-content: center; }
  .scroll { flex: 1; overflow-y: auto; padding: 14px 20px; display: flex; flex-direction: column; gap: 10px; }
  /* A flex item with overflow!=visible has automatic min-size ZERO, so a
     long list's cards silently shrink to fit the viewport and clip their
     own text — 24 rows rendered as 30px slivers on a real phone. Never
     let the scroll container's children shrink; scrolling is its job. */
  .scroll > * { flex-shrink: 0; }
  .rowbtn { text-align: left; padding: 14px; display: flex; flex-direction: column; gap: 6px; cursor: pointer; color: var(--text); font: inherit; overflow: hidden; }
  .rowtop { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .urgency-now { color: var(--hazard); border-color: var(--hazard); }
  .urgency-today { color: var(--accent-400); }
  .failed { color: var(--hazard); }
  .deadline { font-family: var(--mono); font-size: 10px; color: var(--hazard); }
  .acct { font-family: var(--mono); font-size: 10px; color: var(--accent-700); margin-left: auto; }
  .summary { font-size: 14px; font-weight: 500; line-height: 1.35; overflow-wrap: anywhere; }
  .rowfoot { display: flex; align-items: center; gap: 8px; min-width: 0; flex-wrap: wrap; }
  .from { font-size: 12px; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 60%; }
  .tag { font-family: var(--mono); font-size: 10px; color: var(--accent-700); }
  .empty { color: var(--text-muted); font-size: 14px; padding: 24px 0; text-align: center; }
  .warnline { display: flex; align-items: flex-start; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .quoted { display: flex; gap: 10px; }
  .gutter { width: 2px; background: var(--hazard); flex-shrink: 0; }
  .qtext { font-size: 13px; line-height: 1.55; color: var(--text-muted); white-space: pre-wrap; overflow-wrap: anywhere; }
  .btngrid { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  .btnrow { display: flex; gap: 10px; }
  .btn { min-height: 48px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; }
  .btnrow .btn { flex: 1; }
  .btn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .ghost { background: none; border: none; color: var(--text-muted); font-size: 13px; min-height: 44px; cursor: pointer; }
  .footnote { font-size: 11px; color: var(--text-muted); text-align: center; }
  .editbox { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; line-height: 1.5; padding: 12px 14px; resize: vertical; box-sizing: border-box; }
  .editline { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--mono); font-size: 13px; padding: 12px 14px; box-sizing: border-box; }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px 20px 28px; display: flex; flex-direction: column; gap: 12px; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; }
  .sheet-sub { font-size: 13px; color: var(--text-muted); overflow-wrap: anywhere; }
  .toast { position: absolute; bottom: 18px; left: 50%; transform: translateX(-50%); background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 10px 16px; font-size: 13px; white-space: nowrap; max-width: 90%; overflow: hidden; text-overflow: ellipsis; }
</style>
