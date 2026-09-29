<script>
  import { tick, untrack } from 'svelte';
  import { apiFetch as fetch } from './api.js';
  import {
    listUrl, personaUrl, chatUrl, relationshipLabel, emptyRun, applyEvent, settle,
    taintLabel, safetyLine, doseLine,
  } from './persona.js';
  // The Personas tab (PERSONA-DESIGN.md §8; the owner's ruling of
  // 2026-09-29: a tab of its own, not a mode of the assistant's chat).
  //
  // Everything here goes through the persona door (`persona_chat.rs`) —
  // `persona.js` refuses to build any other URL — so nothing on this page can
  // read or write one of the assistant's conversations.
  //
  // The lock is the image library's: the same password, the same token, held
  // only in memory (the no-storage test holds the whole bundle to that).

  let { initial = null } = $props();

  let data = $state(null);
  let error = $state('');
  let token = $state(null);
  let password = $state('');
  let sheet = $state(false);
  let busy = $state(false);

  // The persona being looked at, its earlier chats, and the open chat.
  let chosen = $state(null);
  let history = $state([]);
  let key = $state(null);
  let run = $state(emptyRun());
  // The open chat's safety switches, as its transcript reports them (§12).
  let safety = $state(null);
  // Crisis cards the owner has closed, by id; a closed one leaves a link to
  // its resources rather than vanishing.
  let dismissed = $state(new Set());
  // The resources card opened from the chat's standing link.
  let showResources = $state(false);
  // Attached while a run was streaming: that run's end is re-read, because
  // what streamed before the stream opened is only on the server.
  let partial = false;
  let refused = $state([]);
  let goal = $state('');
  let input = $state('');
  let source = null;
  let usedInitial = false;
  let scroller = $state(null);

  const personas = $derived(data?.personas ?? []);

  async function load() {
    try {
      const res = await fetch(listUrl(token));
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      // A token the server no longer honours has lapsed.
      if (token && !data.unlocked) token = null;
      if (chosen) chosen = personas.find((p) => p.name === chosen.name) ?? null;
      // A deep link (`#personas/mara`) opens that persona, earlier chats and
      // all — once: a later reload (a lock toggle) must not re-enter it.
      if (!chosen && initial && !usedInitial) {
        usedInitial = true;
        chosen = personas.find((p) => p.name === initial) ?? null;
        if (chosen) await loadHistory();
      }
      error = '';
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  // Once, on mount. `load` reads `token` and `chosen` before its first
  // await, so called bare here it made them this effect's dependencies: every
  // lock toggle re-ran it, and the teardown closed the open chat's stream
  // while `key` stayed set — a chat frozen without a word (review of #415).
  $effect(() => {
    untrack(() => load());
    return () => close();
  });

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
      sheet = false;
      await load();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      password = '';
      busy = false;
    }
  }

  async function relock() {
    const t = token;
    token = null;
    // A locked persona's chat closes with the lock: the lock hides (§8.3).
    if (chosen?.locked) back();
    await load();
    if (t) {
      fetch('/api/library/relock', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ token: t }),
      }).catch(() => {});
    }
  }

  function close() {
    source?.close();
    source = null;
  }

  function back() {
    close();
    chosen = null;
    key = null;
    run = emptyRun();
    history = [];
    refused = [];
  }

  async function choose(p) {
    close();
    chosen = p;
    key = null;
    run = emptyRun();
    refused = [];
    goal = '';
    await loadHistory();
  }

  async function loadHistory() {
    if (!chosen) return;
    try {
      const res = await fetch(personaUrl(chosen.name, '/chats', token));
      history = res.ok ? (await res.json()).chats : [];
    } catch {
      history = [];
    }
  }

  async function scrollDown() {
    await tick();
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  // Read the transcript as the server holds it now, keeping what only the
  // page holds (`settle`). Reads are numbered so a slow one never lands over
  // a newer one.
  let readGen = 0;
  async function reread(k) {
    const gen = ++readGen;
    const res = await fetch(chatUrl(k, '', token));
    if (!res.ok) throw new Error((await res.text()).trim());
    const t = await res.json();
    if (key !== k || gen !== readGen) return;
    run = { ...emptyRun(settle(t.entries, run), t.taint ?? null), running: !!t.running };
    safety = t.safety ?? null;
    scrollDown();
    return t;
  }

  // The stream opens *before* the read, and a finished run is read again:
  // a page attached mid-run cannot rebuild the part of the answer that
  // streamed before it subscribed, and without the re-read that turn would
  // read as a complete but truncated reply (review of #415; `Chat.svelte`
  // argues the same hazard).
  async function attach(k) {
    close();
    key = k;
    run = emptyRun();
    // Or the previous chat's disclosure line and resources show for a round
    // trip (review of #418).
    safety = null;
    dismissed = new Set();
    showResources = false;
    partial = false;
    const s = new EventSource(chatUrl(k, '/events', token));
    source = s;
    s.onmessage = (m) => {
      let ev;
      try {
        ev = JSON.parse(m.data);
      } catch {
        return; /* a malformed event is dropped, never drawn */
      }
      run = applyEvent(run, ev);
      scrollDown();
      if (ev.type === 'done') {
        // Only a run that finished is re-read: a failed one was rolled back
        // on the server, and the page's own record of it — the message and
        // why it failed — is the one worth keeping on screen.
        // And only when this page joined it midway: otherwise the stream
        // carried the whole turn (review of #415).
        if (ev.ok && partial) {
          partial = false;
          reread(k).catch((e) => (error = String(e?.message ?? e)));
        }
        loadHistory();
      }
    };
    // A stream the server ended (a relock, a restart) is said, not frozen.
    s.onerror = () => {
      if (s.readyState === 2 && source === s) {
        error = 'this chat stopped updating — open it again';
      }
    };
    try {
      const t = await reread(k);
      partial = !!t?.running;
    } catch (e) {
      close();
      key = null;
      error = String(e?.message ?? e);
    }
  }

  async function start() {
    busy = true;
    error = '';
    try {
      const res = await fetch(personaUrl(chosen.name, '/chats', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ unlock: token ?? undefined, goal: goal.trim() || undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const opened = await res.json();
      refused = opened.refused ?? [];
      goal = '';
      await attach(opened.key);
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function resume(id) {
    busy = true;
    error = '';
    try {
      const res = await fetch(personaUrl(chosen.name, '/resume', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ id, unlock: token ?? undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const opened = await res.json();
      refused = opened.refused ?? [];
      await attach(opened.key);
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function send() {
    const text = input.trim();
    if (!text || !key) return;
    input = '';
    try {
      const res = await fetch(chatUrl(key, '/send'), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ text, unlock: token ?? undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
    } catch (e) {
      input = text;
      error = String(e?.message ?? e);
    }
  }

  async function stop() {
    if (!key) return;
    await fetch(chatUrl(key, '/cancel'), {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ unlock: token ?? undefined }),
    }).catch(() => {});
  }

  function onKey(e) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  }

  const clock = (iso) => {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
  };

  const when = (iso) => {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  };
</script>

<div class="page">
  <header class="head">
    {#if chosen}
      <button class="backbtn" aria-label="all personas" onclick={back}>
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 5l-7 7 7 7" /></svg>
      </button>
      <div class="who">
        <span class="dtitle">{chosen.display}</span>
        <!-- Disclosure is the harness's, not the persona's (§12.1). -->
        <span class="meta"><span class="ai">AI</span>{#if relationshipLabel(chosen)} · {relationshipLabel(chosen)}{/if} · v{chosen.version}</span>
      </div>
      {#if key && taintLabel(run.taint)}
        <!-- What this conversation has touched. With the leak guard lifted
             for personas (D11), this is what is left to say it. -->
        <span class="chip taint" title="what this conversation has touched">{taintLabel(run.taint)}</span>
      {/if}
    {:else}
      <div class="dtitle grow">Personas</div>
    {/if}
    <button
      class="lockbtn"
      class:on={!!token}
      title={token ? 'showing locked personas — tap to hide them' : 'show locked personas'}
      aria-label={token ? 'hide locked personas' : 'show locked personas'}
      aria-pressed={!!token}
      onclick={() => (token ? relock() : data?.has_password ? (sheet = true) : unlock())}
    >
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
        {#if token}<path d="M7 11V7a5 5 0 019.9-1M5 11h14v10H5z" />{:else}<path d="M7 11V7a5 5 0 0110 0v4M5 11h14v10H5z" />{/if}
      </svg>
    </button>
  </header>

  {#if error}
    <div class="warnline pad">{error}</div>
  {/if}

  {#if !chosen}
    <div class="scroll">
      {#if data && personas.length === 0 && !data.hidden_locked}
        <div class="empty">
          No personas yet. Make one in a terminal with
          <code>mecha persona new &lt;name&gt; --relationship colleague</code>,
          then write who they are with <code>mecha persona edit &lt;name&gt;</code>.
        </div>
      {/if}
      <div class="grid">
        {#each personas as p (p.name)}
          <button class="tile" onclick={() => choose(p)}>
            {#if p.portrait}
              <img src={p.portrait} alt={p.display} />
            {:else}
              <div class="noimg">{p.display.slice(0, 1).toUpperCase()}</div>
            {/if}
            <span class="tname">{p.display}{#if p.locked} 🔒{/if}</span>
            <span class="trel">{relationshipLabel(p) || 'no relationship'}</span>
            {#if !p.approved}
              <span class="badge">not approved</span>
            {:else if p.problems.length}
              <span class="badge">{p.problems.length} to fix</span>
            {/if}
          </button>
        {/each}
        {#if data?.hidden_locked}
          <div class="tile hiddentile" aria-label="locked personas hidden">
            <div class="noimg">🔒 {data.hidden_locked}</div>
            <span class="tname">hidden</span>
          </div>
        {/if}
      </div>
    </div>
  {:else}
    <div class="scroll" bind:this={scroller}>
      {#if !key}
        {#if !chosen.approved}
          <div class="warnline">
            {chosen.display} is not approved — <code>mecha persona approve {chosen.name}</code> after reading them.
          </div>
        {/if}
        {#each chosen.problems as problem}
          <div class="warnline">{problem}</div>
        {/each}
        <div class="barnote">
          {safetyLine(chosen.safety)}{#if doseLine(chosen.dose)}<br />{doseLine(chosen.dose)}{/if}
        </div>
        <div class="startbox">
          <input
            class="editbox"
            placeholder="What you want from this chat (optional)"
            maxlength="2000"
            bind:value={goal}
          />
          <button class="abtn primary" disabled={busy || !chosen.approved} onclick={start}>New chat</button>
        </div>
        {#if history.length}
          <div class="earlier">earlier</div>
          {#each history as h (h.id)}
            <button class="card rowbtn" disabled={busy} onclick={() => resume(h.id)}>
              <span class="rowtop"><span class="topic">{when(h.created)}</span><span class="when">{clock(h.created)}</span></span>
            </button>
          {/each}
        {/if}
      {:else}
        {#if safety?.disclosure}
          <!-- The harness says it, not the persona (§12.1). -->
          <div class="disclosure">{chosen.display} is an AI playing a character you wrote.</div>
        {/if}
        {#if refused.length}
          <div class="barnote">
            Not given here: {refused.map((r) => `${r.tool} (${r.why})`).join('; ')}.
          </div>
        {/if}
        {#each run.entries as entry, i (i)}
          {#if entry.kind === 'user'}
            <div class="bubble" class:queued={entry.queued}>
              {entry.text}{#if entry.queued}<span class="queued-tag">{entry.delivery === 'discarded' ? 'not delivered — send again' : entry.delivery === 'delivered' ? 'steered' : 'queued'}</span>{/if}
            </div>
          {:else if entry.kind === 'assistant'}
            <div class="answer">{entry.text}</div>
          {:else if entry.kind === 'tool'}
            <div class="tool" class:err={entry.is_error}>{entry.name}{entry.is_error ? ' — failed' : ''}</div>
          {:else if entry.kind === 'notice'}
            <div class="notice">{entry.text}</div>
          {:else if entry.kind === 'crisis'}
            {#if dismissed.has(entry.id)}
              <button class="linkbtn" onclick={() => { dismissed.delete(entry.id); dismissed = new Set(dismissed); }}>
                support resources
              </button>
            {:else}
              <!-- The plain voice, not the persona (§12.2): the persona paused
                   on this message and did not answer it. -->
              <div class="crisis" role="alert">
                <div class="crisistext">{entry.text}</div>
                <button class="abtn" onclick={() => (dismissed = new Set([...dismissed, entry.id]))}>Close</button>
              </div>
            {/if}
          {/if}
        {/each}
        {#if run.streaming}
          <div class="answer">{run.streaming}</div>
        {/if}
        {#if safety?.resources}
          <!-- One tap away in every chat the sensor watches, whatever a
               reload did to the card (review of #418). -->
          {#if showResources}
            <div class="crisis" role="note">
              <div class="crisistext">{safety.resources}</div>
              <button class="abtn" onclick={() => (showResources = false)}>Close</button>
            </div>
          {:else}
            <button class="linkbtn quiet" onclick={() => (showResources = true)}>support resources</button>
          {/if}
        {/if}
      {/if}
    </div>
    {#if key}
      <div class="composer">
        <textarea
          class="editbox"
          rows="2"
          placeholder={`Say something to ${chosen.display}`}
          bind:value={input}
          onkeydown={onKey}
        ></textarea>
        <!-- Send stays during a run: it steers, and a phone has no Enter to
             spare for that. -->
        <button class="abtn primary" disabled={!input.trim()} onclick={send}>{run.running ? 'Steer' : 'Send'}</button>
        {#if run.running}
          <button class="abtn" onclick={stop}>Stop</button>
        {/if}
        <button class="abtn" onclick={() => { close(); key = null; run = emptyRun(); loadHistory(); }}>Done</button>
      </div>
    {/if}
  {/if}

  {#if sheet}
    <button class="scrim" aria-label="close" onclick={() => (sheet = false)}></button>
    <div class="sheet">
      <div class="sheet-grip"></div>
      <div class="sheet-text">Show locked personas</div>
      <input class="editbox" type="password" placeholder="library lock password" bind:value={password}
        onkeydown={(e) => e.key === 'Enter' && unlock()} />
      <button class="abtn primary" disabled={busy || !password} onclick={unlock}>Unlock</button>
      <div class="barnote">The image library's password. Locking hides; it is not encryption.</div>
    </div>
  {/if}
</div>

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  .head { display: flex; align-items: center; gap: 8px; padding: 12px var(--gutter) 0; padding-right: 56px; min-height: 44px; }
  .grow { flex: 1; }
  .who { flex: 1; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .dtitle { font-family: var(--mono); font-size: 13px; color: var(--accent-400); overflow-wrap: anywhere; }
  .meta { font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  /* Muted, not amber: amber is the taint chip's (Chat.svelte's rule), and
     "this is an AI" is not a security posture. */
  .ai { color: var(--text-muted); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 0 5px; margin-right: 2px; }
  .chip.taint { flex-shrink: 0; font-family: var(--mono); font-size: 10px; color: var(--hazard); border: 1px solid var(--hazard); border-radius: var(--radius-chip); padding: 2px 6px; }
  .disclosure { font-size: 12px; color: var(--text-muted); border: 1px solid var(--accent-900); border-radius: var(--radius); padding: 8px 12px; }
  .crisis { display: flex; flex-direction: column; gap: 10px; background: var(--surface); border: 1px solid var(--accent-500); border-radius: var(--radius); padding: 14px; }
  .crisistext { font-size: 14px; line-height: 1.55; white-space: pre-wrap; color: var(--text); }
  .crisis .abtn { align-self: flex-start; }
  .linkbtn.quiet { color: var(--text-muted); font-size: 11px; }
  .linkbtn { align-self: flex-start; background: none; border: none; padding: 0; color: var(--accent-400); font-size: 12px; text-decoration: underline; cursor: pointer; }
  .lockbtn { flex-shrink: 0; display: flex; align-items: center; justify-content: center; min-height: 40px; min-width: 40px; padding: 0; background: none; border: 1px solid var(--accent-900); border-radius: var(--radius-chip); color: var(--text-muted); cursor: pointer; }
  .lockbtn.on { color: var(--hazard); border-color: var(--hazard); }
  .backbtn { background: none; border: none; color: var(--text-muted); min-width: 44px; min-height: 44px; margin: -10px 0 -10px -12px; cursor: pointer; display: flex; align-items: center; justify-content: center; }
  .scroll { flex: 1; overflow-y: auto; padding: 14px var(--gutter); display: flex; flex-direction: column; gap: 10px; }
  .scroll > * { flex-shrink: 0; }
  .pad { padding: 8px var(--gutter) 0; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 10px; }
  .tile { display: flex; flex-direction: column; gap: 4px; padding: 0 0 10px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); overflow: hidden; cursor: pointer; color: var(--text); text-align: left; font: inherit; }
  .tile img { width: 100%; aspect-ratio: 1; object-fit: cover; display: block; }
  .noimg { aspect-ratio: 1; display: flex; align-items: center; justify-content: center; color: var(--accent-400); font-family: var(--mono); font-size: 28px; }
  .hiddentile { cursor: default; border-style: dashed; }
  .hiddentile .noimg { font-size: 14px; color: var(--text-muted); }
  .tname { font-family: var(--mono); font-size: 12px; padding: 0 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .trel { font-size: 11px; color: var(--text-muted); padding: 0 10px; }
  .badge { align-self: flex-start; margin: 2px 10px 0; font-family: var(--mono); font-size: 10px; color: var(--hazard); border: 1px solid var(--hazard); border-radius: var(--radius-chip); padding: 1px 6px; }
  .card { background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); }
  .rowbtn { text-align: left; padding: 12px 14px; cursor: pointer; color: var(--text); font: inherit; }
  .rowtop { display: flex; align-items: center; gap: 8px; }
  .topic { font-family: var(--mono); font-size: 13px; }
  .when { font-family: var(--mono); font-size: 10px; color: var(--accent-700); margin-left: auto; }
  .earlier { font-family: var(--mono); font-size: 11px; color: var(--text-muted); margin-top: 8px; }
  .startbox { display: flex; gap: 10px; }
  .startbox .editbox { flex: 1; margin-bottom: 0; }
  .editbox { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; padding: 12px 14px; box-sizing: border-box; }
  .abtn { flex-shrink: 0; min-height: 44px; padding: 0 16px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; white-space: nowrap; }
  .abtn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .abtn:disabled { opacity: 0.5; }
  .bubble { align-self: flex-end; max-width: 82%; background: var(--surface); border-radius: var(--radius); padding: 11px 14px; font-size: 14px; line-height: 1.45; white-space: pre-wrap; }
  .bubble.queued { border: 1px solid var(--accent-700); background: var(--bg); }
  .queued-tag { display: block; margin-top: 4px; font-family: var(--mono); font-size: 9px; color: var(--text-muted); }
  .answer { max-width: 92%; font-size: 14px; line-height: 1.5; white-space: pre-wrap; }
  .tool { font-family: var(--mono); font-size: 12px; color: var(--text-muted); }
  .tool.err { color: var(--hazard); }
  .notice { font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  .composer { display: flex; gap: 8px; align-items: flex-end; padding: 10px var(--gutter) 14px; border-top: 1px solid var(--accent-900); }
  .composer .editbox { flex: 1; resize: none; font-size: 14px; }
  .barnote { font-size: 11px; color: var(--text-muted); line-height: 1.5; }
  .empty { color: var(--text-muted); font-size: 14px; padding: 24px 0; text-align: center; line-height: 1.6; }
  .warnline { font-size: 12px; color: var(--hazard); line-height: 1.45; }
  code { font-family: var(--mono); font-size: 11px; }
  .scrim { position: absolute; inset: 0; background: rgba(0, 0, 0, 0.45); z-index: 5; border: none; }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px var(--gutter) 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; }
</style>
