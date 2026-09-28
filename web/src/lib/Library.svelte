<script>
  import { apiFetch as fetch } from './api.js';
  import { PANES, paneOf, entriesFor, counts, originLabel, listUrl } from './library.js';
  // The image library: the characters and styles `image_generate` compiles a
  // scene against (docs/IMAGE-COMPILER-DESIGN.md §7).
  //
  // Read-then-decide, as on the proposals page: approve and reject live in the
  // detail view, beside the text they approve, and approval sends back the
  // digest of the text shown — the server approves only that text, so a
  // candidate proposed while the model was reading outside content is
  // approved as read, never as whatever it says by the time the tap lands.
  //
  // Locked entries are hidden by the server, not by this page. Unlocking
  // trades the lock password for a token held in a variable here — no
  // cookie, no storage — so a reload locks again, and it lapses on its own
  // after half an hour idle. Generation never looks at the lock.
  let { initial = '', navigate = () => {} } = $props();

  const pane = $derived(paneOf(initial));
  const label = { characters: 'Characters', styles: 'Styles', candidates: 'Waiting' };

  let data = $state(null);
  let error = $state(null);
  let token = $state(null);
  let open = $state(null); // the entry in the detail view
  let sheet = $state(null); // 'unlock' | 'remove' | 'reject' | null
  let password = $state('');
  let busy = $state(false);
  let toast = $state(null);

  const shown = $derived(entriesFor(pane, data?.entries));
  const tally = $derived(counts(data?.entries));

  async function load() {
    try {
      const res = await fetch(listUrl(token));
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      // A token the server no longer honours has lapsed: drop it, so the
      // toggle says what is true.
      if (token && !data.unlocked) token = null;
      if (open) open = data.entries.find((e) => e.kind === open.kind && e.name === open.name) ?? null;
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }
  load();

  function say(text) {
    toast = text;
    setTimeout(() => (toast === text ? (toast = null) : null), 3500);
  }

  async function unlock() {
    busy = true;
    try {
      const res = await fetch('/api/library/unlock', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ password }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      token = (await res.json()).token;
      sheet = null;
      await load();
    } catch (e) {
      say(String(e?.message ?? e));
    } finally {
      password = '';
      busy = false;
    }
  }

  async function relock() {
    const t = token;
    token = null;
    open = open?.locked ? null : open;
    await load();
    if (t) {
      fetch('/api/library/relock', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ token: t }),
      }).catch(() => {});
    }
  }

  async function act(entry, action) {
    busy = true;
    try {
      // The token rides along: the server acts on a locked entry only while
      // unlocked, as it shows one. `shown` is the server's own signature of
      // the text on screen — approval is of exactly that.
      const body = { unlock: token ?? undefined, ...(action === 'approve' ? { shown: entry.shown } : {}) };
      const res = await fetch(`/api/library/${entry.kind}/${entry.name}/${action}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      say(
        {
          approve: `Approved ${entry.name}`,
          reject: `Rejected ${entry.name}`,
          remove: `Removed ${entry.name}`,
          lock: `Locked ${entry.name}`,
          unlock: `Unlocked ${entry.name}`,
        }[action],
      );
      sheet = null;
      if (action === 'reject' || action === 'remove' || (action === 'lock' && !token)) open = null;
      await load();
    } catch (e) {
      say(String(e?.message ?? e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="page">
  <header class="head">
    <div class="chips">
      {#each PANES as p}
        <button class="chipbtn" class:active={pane === p} onclick={() => { open = null; navigate(`library/${p}`); }}>
          {label[p]}<span class="chipcount">{data ? tally[p] : '—'}</span>
        </button>
      {/each}
    </div>
    <!-- Compact on purpose: on a phone a labelled pill pushed the first
         pane chip off the row. The state is in its colour and its label. -->
    <button
      class="lockbtn"
      class:on={!!token}
      title={token ? 'showing locked entries — tap to hide them' : 'show locked entries'}
      aria-label={token ? 'hide locked entries' : 'show locked entries'}
      aria-pressed={!!token}
      onclick={() => (token ? relock() : (sheet = 'unlock'))}
    >
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
        {#if token}<path d="M7 11V7a5 5 0 019.9-1M5 11h14v10H5z" />{:else}<path d="M7 11V7a5 5 0 0110 0v4M5 11h14v10H5z" />{/if}
      </svg>
    </button>
  </header>

  <div class="scroll">
    {#if error}
      <div class="warnline">{error}</div>
    {/if}
    {#if open}
      <div class="deckhead">
        <button class="backbtn" aria-label="back" onclick={() => (open = null)}>
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M15 5l-7 7 7 7" /></svg>
        </button>
        <span class="dtitle">{open.kind} · {open.name} · v{open.version}</span>
      </div>
      {#if open.portrait}
        <img class="portrait" src={open.portrait} alt={open.name} />
      {/if}
      <div class="meta">
        {open.status === 'candidate' ? 'waiting for you' : 'approved'}{open.locked ? ' · locked' : ''} · {originLabel(open.origin)}
      </div>
      <div class="quoted">
        <div class="gutter" class:calm={open.origin === 'owner'}></div>
        <div class="qtext">{open.text}</div>
      </div>
      {#if open.status === 'candidate'}
        <div class="btnrow">
          <button class="abtn" disabled={busy} onclick={() => (sheet = 'reject')}>Reject</button>
          <button class="abtn primary" disabled={busy} onclick={() => act(open, 'approve')}>Approve this text</button>
        </div>
      {:else}
        <div class="btnrow">
          <!-- Locking with no password set would hide the entry with no way
               to show it again from here, so the button asks for one first. -->
          <button class="abtn" disabled={busy} onclick={() => (open.locked || data?.has_password ? act(open, open.locked ? 'unlock' : 'lock') : (sheet = 'unlock'))}>
            {open.locked ? 'Unlock' : 'Lock'}
          </button>
          <button class="abtn" disabled={busy} onclick={() => (sheet = 'remove')}>Remove</button>
        </div>
        <div class="barnote">
          Lock hides it while browsing; chats can still use it. In a chat, name
          <code>{open.name}</code> and say what they are wearing and doing.
        </div>
      {/if}
    {:else if data && shown.length === 0}
      <div class="empty">
        {#if pane === 'candidates'}Nothing is waiting.
        {:else if pane === 'styles'}No styles yet — <code>mecha imagelib add-style</code>.
        {:else}No characters yet. Tap <b>Save to library</b> under a picture in chat.{/if}
      </div>
    {:else if pane === 'styles'}
      {#each shown as e (e.name)}
        <button class="rowbtn card" onclick={() => (open = e)}>
          <span class="rowtop"><span class="topic">{e.name}</span>{#if e.locked}<span class="badge">locked</span>{/if}<span class="when">v{e.version}</span></span>
          <span class="readingline">{e.text}</span>
        </button>
      {/each}
    {:else}
      <div class="grid">
        {#each shown as e (e.kind + e.name)}
          <button class="tile" onclick={() => (open = e)}>
            {#if e.portrait}<img src={e.portrait} alt={e.name} loading="lazy" />{:else}<span class="noimg">{e.kind}</span>{/if}
            <span class="tname">{e.name}{#if e.locked}<span class="lockmark"> · locked</span>{/if}</span>
          </button>
        {/each}
      </div>
    {/if}
    {#if data?.hidden_locked && !token}
      <div class="hidden">{data.hidden_locked} locked {data.hidden_locked === 1 ? 'entry' : 'entries'} hidden</div>
    {/if}
    {#if data?.unreadable}
      <div class="warnline">{data.unreadable} {data.unreadable === 1 ? 'entry' : 'entries'} could not be read — <code>mecha imagelib list</code></div>
    {/if}
  </div>

  {#if sheet}
    <div class="scrim" onclick={() => (sheet = null)} role="presentation"></div>
    <div class="sheet">
      <div class="sheet-grip"></div>
      {#if sheet === 'unlock'}
        {#if data?.has_password}
          <div class="sheet-text">Show locked entries</div>
          <form onsubmit={(e) => { e.preventDefault(); unlock(); }}>
            <!-- svelte-ignore a11y_autofocus -->
            <input class="editbox" type="password" autocomplete="off" placeholder="lock password" bind:value={password} autofocus />
            <div class="btnrow">
              <button type="button" class="abtn" onclick={() => (sheet = null)}>Cancel</button>
              <button class="abtn primary" disabled={busy || !password}>Unlock</button>
            </div>
          </form>
          <div class="barnote">Until you reload or leave it idle for half an hour.</div>
        {:else}
          <div class="sheet-text">No lock password yet</div>
          <div class="barnote">Set one in a terminal: <code>mecha imagelib set-lock-password</code></div>
          <button class="abtn" onclick={() => (sheet = null)}>OK</button>
        {/if}
      {:else if sheet === 'remove'}
        <div class="sheet-text">Remove {open?.name}?</div>
        <div class="barnote">It is moved aside, not deleted: pictures already made from it keep their record.</div>
        <div class="btnrow">
          <button class="abtn" onclick={() => (sheet = null)}>Keep</button>
          <button class="abtn primary" disabled={busy} onclick={() => act(open, 'remove')}>Remove</button>
        </div>
      {:else if sheet === 'reject'}
        <div class="sheet-text">Reject {open?.name}?</div>
        <div class="barnote">The proposal and its portrait are deleted.</div>
        <div class="btnrow">
          <button class="abtn" onclick={() => (sheet = null)}>Keep</button>
          <button class="abtn primary" disabled={busy} onclick={() => act(open, 'reject')}>Reject</button>
        </div>
      {/if}
    </div>
  {/if}
  {#if toast}<div class="toast">{toast}</div>{/if}
</div>

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  .head { display: flex; align-items: center; gap: 8px; padding: 12px var(--gutter) 0; padding-right: 56px; }
  .chips { display: flex; gap: 8px; overflow-x: auto; flex: 1; }
  .chipbtn { flex-shrink: 0; display: flex; align-items: center; gap: 7px; min-height: 40px; padding: 0 13px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); color: var(--text-muted); font-family: var(--mono); font-size: 12px; cursor: pointer; }
  .chipbtn.active { color: var(--text); background: var(--accent-900); border-color: var(--accent-700); }
  .chipcount { color: var(--accent-400); font-size: 13px; }
  .lockmark { color: var(--hazard); }
  .lockbtn { flex-shrink: 0; display: flex; align-items: center; justify-content: center; min-height: 40px; min-width: 40px; padding: 0; background: none; border: 1px solid var(--accent-900); border-radius: var(--radius-chip); color: var(--text-muted); font-family: var(--mono); font-size: 11px; cursor: pointer; }
  .lockbtn.on { color: var(--hazard); border-color: var(--hazard); }
  .scroll { flex: 1; overflow-y: auto; padding: 14px var(--gutter); display: flex; flex-direction: column; gap: 10px; }
  .scroll > * { flex-shrink: 0; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 10px; }
  .tile { display: flex; flex-direction: column; gap: 6px; padding: 0; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); overflow: hidden; cursor: pointer; color: var(--text); }
  .tile img { width: 100%; aspect-ratio: 1; object-fit: cover; display: block; }
  .noimg { aspect-ratio: 1; display: flex; align-items: center; justify-content: center; color: var(--text-muted); font-family: var(--mono); font-size: 11px; }
  .tname { font-family: var(--mono); font-size: 12px; padding: 0 10px 10px; text-align: left; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .card { background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); }
  .rowbtn { text-align: left; padding: 14px; display: flex; flex-direction: column; gap: 6px; cursor: pointer; color: var(--text); font: inherit; overflow: hidden; }
  .rowtop { display: flex; align-items: center; gap: 8px; }
  .topic { font-family: var(--mono); font-size: 14px; }
  .badge { font-family: var(--mono); font-size: 10px; color: var(--hazard); border: 1px solid var(--hazard); border-radius: var(--radius-chip); padding: 1px 6px; }
  .when { font-family: var(--mono); font-size: 10px; color: var(--accent-700); margin-left: auto; }
  .readingline { font-size: 12px; line-height: 1.45; color: var(--text-muted); overflow: hidden; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; }
  .deckhead { display: flex; align-items: center; gap: 8px; }
  .backbtn { background: none; border: none; color: var(--text-muted); min-width: 44px; min-height: 44px; margin: -10px 0 -10px -12px; cursor: pointer; display: flex; align-items: center; justify-content: center; }
  .dtitle { font-family: var(--mono); font-size: 13px; color: var(--accent-400); overflow-wrap: anywhere; }
  .portrait { width: 100%; max-width: 420px; border-radius: var(--radius); align-self: center; }
  .meta { font-family: var(--mono); font-size: 11px; color: var(--text-muted); line-height: 1.5; }
  .quoted { display: flex; gap: 10px; }
  .gutter { width: 2px; background: var(--hazard); flex-shrink: 0; }
  .gutter.calm { background: var(--accent-700); }
  .qtext { font-family: var(--mono); font-size: 12px; line-height: 1.6; color: var(--text); white-space: pre-wrap; overflow-wrap: anywhere; }
  .btnrow { display: flex; gap: 10px; }
  .btnrow .abtn { flex: 1; }
  .abtn { flex-shrink: 0; min-height: 44px; padding: 0 16px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; white-space: nowrap; }
  .abtn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .abtn:disabled { opacity: 0.5; }
  .barnote { font-size: 11px; color: var(--text-muted); line-height: 1.5; }
  .empty { color: var(--text-muted); font-size: 14px; padding: 24px 0; text-align: center; line-height: 1.6; }
  .hidden { font-family: var(--mono); font-size: 11px; color: var(--text-muted); text-align: center; padding: 8px 0; }
  .warnline { font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .editbox { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; padding: 12px 14px; box-sizing: border-box; margin-bottom: 12px; }
  .scrim { position: absolute; inset: 0; background: rgba(0, 0, 0, 0.45); z-index: 5; }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px var(--gutter) 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; }
  .toast { position: absolute; bottom: 18px; left: 50%; transform: translateX(-50%); background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 10px 16px; font-size: 13px; white-space: nowrap; max-width: 90%; overflow: hidden; text-overflow: ellipsis; z-index: 7; }
  code { font-family: var(--mono); font-size: 11px; }
</style>
