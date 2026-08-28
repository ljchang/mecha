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
  let pane = $state('queue'); // queue | inbox
  let inbox = $state(null); // mail_recent rows, fetched on first open
  let inboxNote = $state(null);
  let composing = $state(false);
  let cTo = $state('');
  let cSubject = $state('');
  let cBody = $state('');
  let cAccount = $state('');
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

  async function loadInbox() {
    try {
      const res = await fetch('/api/mail/inbox');
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      if (Array.isArray(data)) {
        inbox = data;
        inboxNote = null;
      } else {
        inbox = [];
        inboxNote = data.note ?? null;
      }
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  function openInbox() {
    pane = 'inbox';
    if (inbox === null) loadInbox();
  }

  async function stageCompose() {
    if (!cTo.trim() || !cSubject.trim() || !cBody.trim()) return;
    busy = true;
    try {
      const res = await fetch('/api/mail/compose', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          to: cTo.trim(),
          subject: cSubject.trim(),
          body: cBody,
          account: cAccount.trim() || null,
        }),
      });
      const text = await res.text();
      if (!res.ok) throw new Error(text.trim());
      composing = false;
      cTo = cSubject = cBody = cAccount = '';
      toast = 'staged — review it in the Outbox before it sends';
      setTimeout(() => (toast = null), 5000);
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
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

  // Presentation-only parse of `mecha mail show`'s text: the leading
  // `key:   value` block, then messages split on `--- ` separators. The
  // CLI's text stays the one renderer of a thread — this shapes it for a
  // phone and falls back to the raw text verbatim on any drift, so a
  // format change degrades to yesterday's display, never to a wrong one.
  function parseThread(text) {
    const lines = text.split('\n');
    let i = 0;
    const header = [];
    while (i < lines.length && lines[i].trim() !== '') {
      const m = lines[i].match(/^([a-z]+):\s+(.*)$/);
      if (!m) return null;
      header.push([m[1], m[2]]);
      i++;
    }
    const chunks = lines
      .slice(i)
      .join('\n')
      .split(/\n(?=--- )/)
      .map((c) => c.trim())
      .filter(Boolean);
    const messages = [];
    for (const c of chunks) {
      if (!c.startsWith('--- ')) continue;
      const ls = c.split('\n');
      const meta = ls[0].replace(/^---\s*/, '');
      let j = 1;
      let subject = null;
      while (j < ls.length && ls[j].trim() !== '') {
        // The message-id line is addressed to the model, not the reader.
        if (ls[j].startsWith('Subject: ')) subject = ls[j].slice(9);
        j++;
      }
      const body = ls.slice(j).join('\n').trim();
      messages.push({ meta, subject, body });
    }
    return messages.length ? { header, messages } : null;
  }

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
      <div class="tabs">
        <button class="tab" class:on={pane === 'queue'} onclick={() => (pane = 'queue')}>Queue</button>
        <button class="tab" class:on={pane === 'inbox'} onclick={openInbox}>Inbox</button>
      </div>
    </header>
    <div class="scroll">
      {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}
      {#if pane === 'queue'}
        {#if rows === null && !error}
          <div class="empty">reading the queue…</div>
        {:else}
          <div class="paneinfo">{needs.length} need you · {parked.length} parked</div>
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
      {:else}
        {#if inbox === null && !error}
          <div class="empty">reading the inbox — every account, newest first…</div>
        {:else}
          <div class="pull"><button class="ghost" onclick={loadInbox}>refresh</button></div>
          {#each inbox ?? [] as m}
            <button
              class="card rowbtn"
              onclick={() => open({ thread_id: m.thread_id, account: m.account, summary: m.subject })}
            >
              <div class="rowtop">
                {#if m.unread}<span class="unread"></span>{/if}
                <span class="from">{m.from}</span>
                <span class="acct">{m.account}</span>
              </div>
              <div class="summary">{m.subject}</div>
              {#if m.snippet}<div class="snippet">{m.snippet}</div>{/if}
              <div class="rowfoot"><span class="tag">{(m.date ?? '').slice(0, 16).replace('T', ' ')}</span></div>
            </button>
          {:else}
            <div class="empty">{inboxNote ?? 'The inbox is empty.'}</div>
          {/each}
        {/if}
      {/if}
    </div>
    <button class="fab" onclick={() => (composing = true)} title="write a new email — staged for review">
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="var(--void)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9M16.5 3.5a2.1 2.1 0 013 3L7 19l-4 1 1-4z" /></svg>
    </button>
    {#if composing}
      <div class="cscrim" onclick={() => (composing = false)} aria-hidden="true"></div>
      <div class="sheet">
        <div class="sheet-grip"></div>
        <div class="sheet-text">New email — stages for review, never sends from here</div>
        <input class="editline" bind:value={cTo} placeholder="to: who@example.edu" />
        <input class="editline" bind:value={cSubject} placeholder="subject" />
        <input class="editline" bind:value={cAccount} placeholder="from account (blank = default)" />
        <textarea class="editbox" rows="5" bind:value={cBody} placeholder="the email, in your words (markdown ok)"></textarea>
        <div class="btnrow">
          <button class="btn" onclick={() => (composing = false)}>Back</button>
          <button class="btn primary" disabled={busy || !cTo.trim() || !cSubject.trim() || !cBody.trim()} onclick={stageCompose}>Stage it</button>
        </div>
      </div>
    {/if}
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
        {@const parsed = parseThread(reading.text)}
        {#if parsed}
          <div class="card headers">
            {#each parsed.header as [k, v]}
              <div class="hrow"><span class="hkey">{k}</span><span class="hval">{v}</span></div>
            {/each}
          </div>
          {#each parsed.messages as msg}
            <div class="msg">
              <div class="msg-meta">{msg.meta}</div>
              {#if msg.subject}<div class="msg-subject">{msg.subject}</div>{/if}
              <!-- Third-party text: the gutter marks every line, the outbox
                   source-read rule — a heading scrolls off, a per-line
                   marker cannot. Plain text on purpose: a rendered link in
                   a stranger's mail is a tap onto a stranger's URL. -->
              <div class="quoted"><span class="gutter"></span><div class="mailbody">{msg.body}</div></div>
            </div>
          {/each}
        {:else}
          <div class="quoted"><span class="gutter"></span><div class="qtext">{reading.text}</div></div>
        {/if}
      {/if}
    </div>
    {#if reading.text !== null}
      <div class="actionbar">
        <button class="abtn primary" disabled={busy} onclick={() => prompt('reply', 'Steering for the draft (optional)', 'decline politely; ask for the deadline')}>Draft reply…</button>
        <button class="abtn" disabled={busy} onclick={() => quick('archive', reading.row)}>Archive</button>
        <button class="abtn" disabled={busy} onclick={() => quick('task', reading.row)}>→ Task</button>
        <button class="abtn" disabled={busy} onclick={() => prompt('needs-info', 'What are you waiting for?', 'their dates, before I can book')}>Park…</button>
        <button class="abtn" disabled={busy} onclick={() => prompt('schedule', 'Steering for the calendar draft (optional)', 'propose Thursday afternoon')}>Calendar…</button>
        <button class="abtn" disabled={busy} onclick={() => prompt('forward', 'Forward to (comma-separated) + covering note', 'FYI — this is the one I mentioned', true)}>Forward…</button>
        <button class="abtn" disabled={busy} onclick={() => quick('dismiss', reading.row)}>Dismiss</button>
        <button class="abtn" disabled={busy} onclick={() => (confirmSpam = reading.row)}>Spam…</button>
      </div>
      <div class="barnote">Drafts land in the outbox for review — nothing sends from here.</div>
    {/if}

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
  .headers { padding: 12px 14px; display: flex; flex-direction: column; gap: 7px; }
  .hrow { display: flex; gap: 10px; font-size: 13px; }
  .hkey { font-family: var(--mono); font-size: 11px; color: var(--accent-700); min-width: 68px; padding-top: 1px; flex-shrink: 0; }
  .hval { overflow-wrap: anywhere; line-height: 1.45; }
  .msg { display: flex; flex-direction: column; gap: 8px; }
  .msg-meta { font-family: var(--mono); font-size: 11px; color: var(--text-muted); overflow-wrap: anywhere; }
  .msg-subject { font-size: 15px; font-weight: 500; line-height: 1.4; overflow-wrap: anywhere; }
  .mailbody { font-size: 15px; line-height: 1.6; color: var(--text); white-space: pre-wrap; overflow-wrap: anywhere; }
  .actionbar { display: flex; gap: 8px; overflow-x: auto; padding: 10px 20px 4px; border-top: 1px solid var(--accent-900); background: var(--bg); -webkit-overflow-scrolling: touch; }
  .actionbar::-webkit-scrollbar { display: none; }
  .abtn { flex-shrink: 0; min-height: 44px; padding: 0 16px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; white-space: nowrap; }
  .abtn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .abtn:disabled { opacity: 0.5; }
  .barnote { font-size: 10px; color: var(--text-muted); text-align: center; padding: 4px 20px calc(6px + env(safe-area-inset-bottom)); background: var(--bg); }
  .btnrow { display: flex; gap: 10px; }
  .btn { min-height: 48px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; }
  .btnrow .btn { flex: 1; }
  .btn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .ghost { background: none; border: none; color: var(--text-muted); font-size: 13px; min-height: 44px; cursor: pointer; }
  .footnote { font-size: 11px; color: var(--text-muted); text-align: center; }
  .editbox { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; line-height: 1.5; padding: 12px 14px; resize: vertical; box-sizing: border-box; }
  .editline { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--mono); font-size: 13px; padding: 12px 14px; box-sizing: border-box; }
  /* Above the scrim (5) and the fab (4), matching Tasks/Frontdoor — at
     z auto the scrim paints over the sheet and swallows every tap. */
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px 20px 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; }
  .sheet-sub { font-size: 13px; color: var(--text-muted); overflow-wrap: anywhere; }
  .tabs { display: flex; gap: 8px; }
  .tab { font-family: var(--mono); font-size: 12px; color: var(--text-muted); background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); padding: 8px 13px; min-height: 38px; cursor: pointer; }
  .tab.on { color: var(--text); background: var(--accent-900); border-color: var(--accent-700); }
  .paneinfo { font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  .unread { width: 7px; height: 7px; border-radius: 50%; background: var(--accent-400); flex-shrink: 0; }
  .pull { display: flex; justify-content: flex-end; margin: -6px 0; }
  .fab { position: absolute; right: 18px; bottom: 18px; width: 54px; height: 54px; border-radius: 50%; background: var(--accent-400); border: none; cursor: pointer; display: flex; align-items: center; justify-content: center; box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4); z-index: 4; }
  .cscrim { position: absolute; inset: 0; background: rgba(0, 0, 0, 0.45); z-index: 5; }
  .toast { position: absolute; bottom: 18px; left: 50%; transform: translateX(-50%); background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 10px 16px; font-size: 13px; white-space: nowrap; max-width: 90%; overflow: hidden; text-overflow: ellipsis; }
</style>
