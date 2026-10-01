<script>
  import { onDestroy, untrack } from 'svelte';
  import { apiFetch as fetch } from './api.js';
  import { features } from './features.svelte.js';
  import { opens } from './features.js';
  import {
    PANES, paneOf, entriesFor, counts, originLabel, listUrl, tameName,
    TEXT_MAX, PORTRAIT_EDGE, fitWithin, formProblem, formBody,
  } from './library.js';
  import { watchIdle, idleSpan } from './autolock.js';
  import VoicesPane from './VoicesPane.svelte';
  // The image library: the characters and styles `image_generate` compiles a
  // scene against (docs/IMAGE-COMPILER-DESIGN.md §7).
  //
  // Read-then-decide, as on the proposals page: approve and reject live in the
  // detail view, beside the text they approve, and approval sends back the
  // digest of the text shown — the server approves only that text, so a
  // candidate proposed while the model was reading outside content is
  // approved as read, never as whatever it says by the time the tap lands.
  //
  // Locked entries are hidden by the server, not by this page. Showing them
  // trades the lock password — or nothing, when none is set, which makes the
  // lock a plain toggle (the owner's ruling) — for a token held in a variable
  // here: no cookie, no storage, so a reload hides them again. It lapses
  // after the owner's autolock (Settings → Lock, 15 minutes unless set) with
  // no one touching the page — the page locks itself (`autolock.js`), and the
  // server's token lapses on the same span for a page that was closed.
  // Generation never looks at the lock.
  //
  // Adding and editing are the owner's own acts, each the same `mecha
  // imagelib` command the terminal runs. A portrait is scaled down and
  // re-encoded here before it is sent (PORTRAIT_EDGE), which also leaves the
  // photo's metadata behind. Edit is offered on approved entries only: a
  // candidate is approved or rejected as the model wrote it.
  let { initial = '', navigate = () => {} } = $props();

  const pane = $derived(paneOf(initial));
  const label = { characters: 'Characters', styles: 'Styles', voices: 'Voices', candidates: 'Waiting' };
  // How many voices the Voices pane last read: its own list, not an entry
  // count; a dash until that pane has been opened.
  let voiceCount = $state(null);

  let data = $state(null);
  let error = $state(null);
  let token = $state(null);
  let open = $state(null); // the entry in the detail view
  let sheet = $state(null); // 'unlock' | 'remove' | 'reject' | null
  let password = $state('');
  let busy = $state(false);
  let toast = $state(null);
  // The add/edit form: { mode, kind, name, text, locked, portrait, preview }.
  let form = $state(null);
  let preparing = $state(false);

  const shown = $derived(entriesFor(pane, data?.entries));

  // A pane change — a chip, the back button, a typed hash — leaves whatever
  // was open in the pane before: a half-filled form must not follow the owner
  // into a pane it does not belong to.
  let lastPane = untrack(() => pane);
  $effect(() => {
    const now = pane;
    untrack(() => {
      if (now === lastPane) return;
      lastPane = now;
      open = null;
      closeForm();
    });
  });
  const tally = $derived(counts(data?.entries));

  async function load() {
    try {
      const res = await fetch(listUrl(token));
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      // A token the server no longer honours has lapsed: drop it, so the
      // toggle says what is true.
      if (token && !data.unlocked) dropToken();
      if (open) open = data.entries.find((e) => e.kind === open.kind && e.name === open.name) ?? null;
      // An edit form lives only as long as the entry it edits.
      if (!open && form?.mode === 'edit') closeForm();
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
      const granted = await res.json();
      token = granted.token;
      armIdle(granted.idle_secs);
      sheet = null;
      await load();
    } catch (e) {
      say(String(e?.message ?? e));
    } finally {
      password = '';
      busy = false;
    }
  }

  // The autolock, as the Personas tab keeps it: the owner's span with no one
  // touching the page locks it, and a return to the page asks whether the
  // token outlived a restart.
  let stopIdle = null;
  function armIdle(secs) {
    stopIdle?.();
    stopIdle = watchIdle({ idleMs: idleSpan(secs) * 1000, onIdle: relock, onReturn: load });
  }

  function dropToken() {
    token = null;
    stopIdle?.();
    stopIdle = null;
  }

  function revoke(t) {
    fetch('/api/library/relock', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ token: t }),
    }).catch(() => {});
  }

  // Leaving the tab ends the unlock: the page forgets its token, so the
  // server should too.
  onDestroy(() => {
    stopIdle?.();
    if (token) revoke(token);
  });

  async function relock() {
    const t = token;
    dropToken();
    if (open?.locked) {
      open = null;
      if (form?.mode === 'edit') closeForm();
    }
    await load();
    if (t) revoke(t);
  }

  function startAdd(kind) {
    open = null;
    form = { mode: 'add', kind, name: '', text: '', locked: false, portrait: null, preview: null };
  }

  function startEdit(entry) {
    form = { mode: 'edit', kind: entry.kind, name: entry.name, text: entry.text, portrait: null, preview: null };
  }

  // A picked file, scaled to fit PORTRAIT_EDGE and re-encoded as JPEG, as
  // base64 for the JSON body. createImageBitmap applies the photo's
  // orientation, so a phone portrait stays upright.
  async function pick(file) {
    if (!file || !form) return;
    // The form this picture is for: a cancel, or a new form, while it
    // decodes must not receive it.
    const target = form;
    preparing = true;
    try {
      const bitmap = await createImageBitmap(file, { imageOrientation: 'from-image' });
      const { w, h } = fitWithin(bitmap.width, bitmap.height, PORTRAIT_EDGE);
      const canvas = document.createElement('canvas');
      canvas.width = w;
      canvas.height = h;
      canvas.getContext('2d').drawImage(bitmap, 0, 0, w, h);
      bitmap.close?.();
      const blob = await new Promise((resolve) => canvas.toBlob(resolve, 'image/jpeg', 0.92));
      if (!blob) throw new Error('this picture could not be read');
      const bytes = new Uint8Array(await blob.arrayBuffer());
      let binary = '';
      for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      // Cancelled, the pane changed, or another form opened while this decoded.
      if (form !== target) return;
      if (form.preview) URL.revokeObjectURL(form.preview);
      form.portrait = btoa(binary);
      form.preview = URL.createObjectURL(blob);
    } catch (e) {
      say(String(e?.message ?? e));
    } finally {
      preparing = false;
    }
  }

  function closeForm() {
    if (form?.preview) URL.revokeObjectURL(form.preview);
    form = null;
  }

  async function submitForm() {
    const entry = form.mode === 'edit' ? open : null;
    // The form is meaningless without the entry it edits.
    if (form.mode === 'edit' && !entry) return closeForm();
    if (formProblem(form, entry)) return;
    busy = true;
    try {
      const res = await fetch(form.mode === 'add' ? '/api/library/add' : '/api/library/edit', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(formBody(form, entry, token)),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const { kind, name, mode } = form;
      say(mode === 'add' ? `Added ${name}` : `Saved ${name}`);
      closeForm();
      await load();
      // Land on what was just made, when this view may show it.
      open = data?.entries.find((e) => e.kind === kind && e.name === name) ?? null;
    } catch (e) {
      say(String(e?.message ?? e));
    } finally {
      busy = false;
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
      <!-- Only the panes that open: with the library off, its candidates
           pane opens for what waits in it (OPENS_ANYWAY), and no other. -->
      {#each PANES.filter((p) => opens(features.rows, 'library', p)) as p}
        <button class="chipbtn" class:active={pane === p} onclick={() => { open = null; closeForm(); navigate(`library/${p}`); }}>
          {label[p]}<span class="chipcount">{p === 'voices' ? (voiceCount ?? '—') : data ? tally[p] : '—'}</span>
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
      onclick={() => (token ? relock() : data?.has_password ? (sheet = 'unlock') : unlock())}
    >
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
        {#if token}<path d="M7 11V7a5 5 0 019.9-1M5 11h14v10H5z" />{:else}<path d="M7 11V7a5 5 0 0110 0v4M5 11h14v10H5z" />{/if}
      </svg>
    </button>
  </header>

  <div class="scroll">
    {#if pane === 'voices'}
      <VoicesPane {token} oncount={(n) => (voiceCount = n)} />
    {:else}
    {#if error}
      <div class="warnline">{error}</div>
    {/if}
    {#if form}
      {@const problem = formProblem(form, form.mode === 'edit' ? open : null)}
      <div class="deckhead">
        <button class="backbtn" aria-label="back" onclick={closeForm}>
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M15 5l-7 7 7 7" /></svg>
        </button>
        <span class="dtitle">{form.mode === 'add' ? `new ${form.kind}` : `${form.kind} · ${form.name} · edit`}</span>
      </div>
      <form class="libform" onsubmit={(e) => { e.preventDefault(); submitForm(); }}>
        {#if form.kind === 'character'}
          <label class="drop" class:filled={!!(form.preview || (form.mode === 'edit' && open?.portrait))}>
            {#if form.preview}
              <img src={form.preview} alt="new portrait" />
            {:else if form.mode === 'edit' && open?.portrait}
              <img src={open.portrait} alt={open.name} />
            {:else}
              <span class="dropnote">{preparing ? 'reading…' : 'choose a portrait'}</span>
            {/if}
            <input type="file" accept="image/png,image/jpeg,image/webp" onchange={(e) => pick(e.currentTarget.files?.[0])} />
          </label>
          <div class="barnote">
            {form.mode === 'edit' ? 'Tap the picture to replace it. ' : ''}One person, facing the camera, face and shoulders
            clearly visible — it is sent as that person's reference in every picture that names them.
          </div>
        {/if}
        {#if form.mode === 'add'}
          <input
            class="editbox"
            placeholder={form.kind === 'style' ? 'name, e.g. watercolour' : 'name, e.g. maya'}
            autocomplete="off"
            value={form.name}
            oninput={(e) => (form.name = tameName(e.currentTarget.value))}
          />
        {/if}
        <textarea
          class="editbox"
          rows={form.kind === 'style' ? 5 : 3}
          maxlength={TEXT_MAX[form.kind]}
          placeholder={form.kind === 'style'
            ? 'how pictures in this style look — medium, palette, light, texture'
            : 'a short description: age, build and height, hair, anything distinctive — not clothing'}
          bind:value={form.text}
        ></textarea>
        <div class="counter">{form.text.trim().length} / {TEXT_MAX[form.kind]}</div>
        {#if form.mode === 'add'}
          <label class="lockline">
            <input type="checkbox" bind:checked={form.locked} />
            lock (hide while browsing)
          </label>
        {/if}
        <div class="btnrow">
          <button type="button" class="abtn" onclick={closeForm}>Cancel</button>
          <button class="abtn primary" disabled={busy || preparing || !!problem}>
            {form.mode === 'add' ? 'Add' : 'Save'}
          </button>
        </div>
        {#if problem && problem !== 'nothing changed yet'}<div class="barnote">Still needs {problem}.</div>{/if}
        {#if form.mode === 'edit'}
          <div class="barnote">Saving makes a new version; pictures already made keep the one they used.</div>
        {/if}
      </form>
    {:else if open}
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
          <button class="abtn" disabled={busy} onclick={() => startEdit(open)}>Edit</button>
          <button class="abtn" disabled={busy} onclick={() => act(open, open.locked ? 'unlock' : 'lock')}>
            {open.locked ? 'Unlock' : 'Lock'}
          </button>
          <button class="abtn" disabled={busy} onclick={() => (sheet = 'remove')}>Remove</button>
        </div>
        <div class="barnote">
          Lock hides it while browsing; chats can still use it. In a chat, name
          <code>{open.name}</code> and say what they are wearing and doing.
        </div>
      {/if}
    {:else if data && shown.length === 0 && pane === 'candidates'}
      <div class="empty">Nothing is waiting.</div>
    {:else if pane === 'styles'}
      <button class="rowbtn card addrow" onclick={() => startAdd('style')}>
        <span class="topic">+ Add style</span>
        <span class="readingline">A name and a few lines on how the pictures should look.</span>
      </button>
      {#each shown as e (e.name)}
        <button class="rowbtn card" onclick={() => (open = e)}>
          <span class="rowtop"><span class="topic">{e.name}</span>{#if e.locked}<span class="badge">locked</span>{/if}<span class="when">v{e.version}</span></span>
          <span class="readingline">{e.text}</span>
        </button>
      {/each}
    {:else}
      <div class="grid">
        {#if pane === 'characters'}
          <button class="tile addtile" onclick={() => startAdd('character')}>
            <span class="addmark">+</span>
            <span class="tname">Add character</span>
          </button>
        {/if}
        {#each shown as e (e.kind + e.name)}
          <button class="tile" onclick={() => (open = e)}>
            {#if e.portrait}<img src={e.portrait} alt={e.name} loading="lazy" />{:else}<span class="noimg">{e.kind}</span>{/if}
            <span class="tname">{e.name}{#if e.locked}<span class="lockmark"> · locked</span>{/if}</span>
          </button>
        {/each}
      </div>
    {/if}
    {#if !form && !open && data && shown.length === 0 && pane === 'characters'}
      <div class="empty">No characters yet. Add one from a portrait, or tap <b>Save to library</b> under a picture in chat.</div>
    {/if}
    {#if data?.unreadable}
      <div class="warnline">{data.unreadable} {data.unreadable === 1 ? 'entry' : 'entries'} could not be read — <code>mecha imagelib list</code></div>
    {/if}
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
  .warnline { font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .editbox { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; padding: 12px 14px; box-sizing: border-box; margin-bottom: 12px; }
  .scrim { position: absolute; inset: 0; background: rgba(0, 0, 0, 0.45); z-index: 5; }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px var(--gutter) 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; }
  .toast { position: absolute; bottom: 18px; left: 50%; transform: translateX(-50%); background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 10px 16px; font-size: 13px; white-space: nowrap; max-width: 90%; overflow: hidden; text-overflow: ellipsis; z-index: 7; }
  code { font-family: var(--mono); font-size: 11px; }
  .addtile { border-style: dashed; align-items: stretch; }
  .addmark { aspect-ratio: 1; display: flex; align-items: center; justify-content: center; color: var(--accent-400); font-size: 40px; font-weight: 300; }
  .addrow { border-style: dashed; }
  .libform { display: flex; flex-direction: column; gap: 10px; max-width: 480px; }
  .libform .editbox { margin-bottom: 0; }
  .libform textarea.editbox { resize: vertical; font-size: 14px; line-height: 1.5; }
  .drop { position: relative; display: flex; align-items: center; justify-content: center; width: 100%; max-width: 320px; aspect-ratio: 1; border: 1px dashed var(--accent-700); border-radius: var(--radius); overflow: hidden; cursor: pointer; background: var(--surface); }
  .drop.filled { border-style: solid; }
  .drop img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .drop input { position: absolute; inset: 0; opacity: 0; cursor: pointer; }
  .dropnote { font-family: var(--mono); font-size: 12px; color: var(--text-muted); }
  .counter { font-family: var(--mono); font-size: 10px; color: var(--text-muted); text-align: right; margin-top: -6px; }
  .lockline { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--text-muted); }
</style>
