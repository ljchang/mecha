<script>
  import { onDestroy } from 'svelte';
  import { apiFetch as fetch } from './api.js';
  import './form.css';
  import { idleSpan, watchIdle } from './autolock.js';
  import { features } from './features.svelte.js';
  import { isShown } from './features.js';
  import { SECTIONS, listUrl, entryUrl, standing, rowDetail, formFor, formFrom, entryOf, unusedLine, others } from './loras.js';

  // The owner's LoRA registry (docs/LORA-DESIGN.md §6): one section per
  // model, each shown while its feature is on and read through its own
  // route. The whole pane sits behind the library lock (L3): the server
  // sends no name, about or trigger without a live unlock, so this page
  // never holds them locked.
  const sections = $derived(SECTIONS.filter((s) => isShown(features.rows, s.feature)));

  let token = $state(null);
  let hasPassword = $state(null);
  let password = $state('');
  let busy = $state(false);
  let error = $state(null);
  // Per family: the route's answer, or { error }.
  let data = $state({});
  // The open form: { family, route, ...fields }, or null.
  let form = $state(null);
  let formError = $state(null);
  // The old file is the registry, and saving would stop it being read: the
  // server's words, shown before the save is sent again with consent.
  let confirmNew = $state(null);
  // A remove waiting for its second tap: `${route}/${name}`.
  let removing = $state(null);
  // Which sections show their server's files for other models too.
  let showOthers = $state({});

  async function readLock() {
    try {
      const res = await fetch('/api/settings/lock');
      hasPassword = res.ok ? !!(await res.json()).has_password : null;
    } catch {
      hasPassword = null;
    }
  }
  readLock();

  async function load() {
    const next = {};
    await Promise.all(
      sections.map(async (s) => {
        try {
          const res = await fetch(listUrl(s.route, token));
          if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
          next[s.family] = await res.json();
        } catch (e) {
          next[s.family] = { error: String(e?.message ?? e) };
        }
      }),
    );
    // A token the server no longer honours has lapsed: drop it.
    if (token && Object.values(next).some((d) => d.locked)) dropToken();
    data = next;
    // A read that went through clears what an earlier action left standing.
    if (Object.values(next).every((d) => !d.error)) error = null;
  }

  async function unlock() {
    busy = true;
    error = null;
    try {
      const res = await fetch('/api/library/unlock', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ password }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const granted = await res.json();
      token = granted.token;
      stopIdle?.();
      stopIdle = watchIdle({ idleMs: idleSpan(granted.idle_secs) * 1000, onIdle: relock, onReturn: load });
      await load();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      password = '';
      busy = false;
    }
  }

  let stopIdle = null;
  function dropToken() {
    token = null;
    stopIdle?.();
    stopIdle = null;
    form = null;
    data = {};
  }

  function revoke(t) {
    fetch('/api/library/relock', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ token: t }),
    }).catch(() => {});
  }

  function relock() {
    const t = token;
    dropToken();
    if (t) revoke(t);
  }

  // Leaving the pane ends the unlock, as leaving the Library tab does.
  onDestroy(() => {
    stopIdle?.();
    if (token) revoke(token);
  });

  function openAdd(s, file = '') {
    form = { family: s.family, route: s.route, ...formFor(s.family, file) };
    formError = null;
    confirmNew = null;
  }

  function openEdit(s, entry) {
    form = { family: s.family, route: s.route, ...formFrom(entry) };
    formError = null;
    confirmNew = null;
  }

  async function save(createNew = false) {
    busy = true;
    formError = null;
    try {
      const editing = form.editing;
      const res = await fetch(editing ? entryUrl(form.route, editing) : `/api/loras/${form.route}`, {
        method: editing ? 'PUT' : 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ unlock: token, entry: entryOf(form, form.family), create_new: createNew }),
      });
      if (res.status === 409) {
        // Only an add that would create the new file can go ahead on
        // consent; the other refusal is said as an error.
        const why = await res.json().catch(() => ({}));
        if (why.consent) confirmNew = why.message;
        else formError = why.message ?? 'refused';
        return;
      }
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      form = null;
      confirmNew = null;
      await load();
    } catch (e) {
      formError = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function remove(s, entry) {
    const key = `${s.route}/${entry.name}`;
    if (removing !== key) {
      removing = key;
      return;
    }
    removing = null;
    busy = true;
    try {
      const res = await fetch(entryUrl(s.route, entry.name, token), { method: 'DELETE' });
      if (res.status === 409) throw new Error((await res.json().catch(() => ({}))).message ?? 'refused');
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      if (form?.editing === entry.name && form?.route === s.route) form = null;
      await load();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }
</script>

<p class="hint">
  Your own LoRAs, from <code>~/.mecha/loras.toml</code>. mecha reads each file's header to check which model it was
  made for, and leaves out any entry whose file is for another model, cannot be read, or is not on the server.
</p>

{#if error}<div class="card notice">{error}</div>{/if}

{#if !token}
  <div class="card fields">
    <p class="sub">LoRAs are behind the library lock, as locked characters are.</p>
    <form
      class="unlock"
      onsubmit={(e) => {
        e.preventDefault();
        unlock();
      }}
    >
      {#if hasPassword !== false}
        <input class="tf-text" type="password" placeholder="password" autocomplete="current-password" bind:value={password} />
      {/if}
      <button class="tf-btn primary" disabled={busy}>Unlock</button>
    </form>
  </div>
{:else}
  <div class="bar">
    <button class="tf-btn" onclick={relock}>Lock</button>
  </div>
  {#each sections as s (s.family)}
    {@const d = data[s.family]}
    <section class="tf-sec">
      <header>
        <h3>{s.title}</h3>
        <p class="tf-help">
          {s.family === 'qwen-image'
            ? 'Qwen-Image LoRAs. Checked and kept here; pictures do not use them yet.'
            : 'MiniMax H3 LoRAs, which the model may pick per clip. A change reaches clips when mecha next restarts.'}
        </p>
      </header>
      {#if !d}
        <div class="tf-row stack"><span class="tf-help">reading…</span></div>
      {:else if d.error}
        <div class="tf-row stack"><span class="bad">{d.error}</span></div>
      {:else}
        {#if d.unreadable}
          <div class="tf-row stack"><span class="bad">The registry does not read ({d.unreadable}); fix it by hand.</span></div>
        {/if}
        {#if d.source === 'legacy'}
          <div class="tf-row stack">
            <span class="tf-help">
              Read from the old clip file <code>{d.path}</code>, which mecha does not edit: change its entries there. Adding
              one here creates <code>{d.new_path}</code>, and the old file then stops being read, so move its entries
              there first.
            </span>
          </div>
        {/if}
        {#if d.ignored}
          <div class="tf-row stack">
            <span class="tf-help">The old clip file <code>{d.ignored}</code> is ignored now.</span>
          </div>
        {/if}
        {#if !d.folders}
          <div class="tf-row stack">
            <span class="bad">
              <code>{d.dir_key}</code> is not set, so no file here can be checked: set it to the folders its server loads
              LoRAs from.
            </span>
          </div>
        {/if}
        {#if d.skipped?.length}
          <div class="tf-row stack">
            <span class="bad">{d.skipped.length} {d.skipped.length === 1 ? 'entry does' : 'entries do'} not read:</span>
            {#each d.skipped as line}<span class="tf-help mono">{line}</span>{/each}
          </div>
        {/if}

        {#each d.entries as e (e.name)}
          {@const st = standing(e, s.family)}
          <div class="tf-row">
            <div class="tf-label">
              <span class="tf-name">{e.name}{#if e.always}<span class="tag">always</span>{/if}</span>
              <span class="tf-help">{e.about}</span>
              <span class="tf-help mono">{rowDetail(e)}</span>
              <span class="tf-help" class:bad={st.bad}>{st.text}</span>
            </div>
            {#if d.source !== 'legacy'}
            <div class="acts">
              <button class="tf-btn" disabled={busy} onclick={() => openEdit(s, e)}>Edit</button>
              <button class="tf-btn" class:warn={removing === `${s.route}/${e.name}`} disabled={busy} onclick={() => remove(s, e)}>
                {removing === `${s.route}/${e.name}` ? 'Remove?' : 'Remove'}
              </button>
            </div>
            {/if}
          </div>
        {:else}
          <div class="tf-row stack"><span class="tf-help">No {s.title.toLowerCase()} LoRAs yet.</span></div>
        {/each}

        {#if form && form.family === s.family}
          <div class="tf-row stack editor">
            <span class="tf-name">{form.editing ? `Edit ${form.editing}` : 'New LoRA'}</span>
            <label class="f"><span>name</span><input class="tf-text" bind:value={form.name} placeholder="letters, digits, - or _" /></label>
            <label class="f"><span>about</span><input class="tf-text" bind:value={form.about} placeholder="one line: what it does" /></label>
            {#if s.family === 'qwen-image'}
              <label class="f"><span>file</span><input class="tf-text mono" bind:value={form.file} /></label>
            {:else}
              <label class="f"><span>frames file</span><input class="tf-text mono" bind:value={form.frames} placeholder="for clips from words or a pinned frame" /></label>
              <label class="f"><span>references file</span><input class="tf-text mono" bind:value={form.references} placeholder="for clips from reference pictures" /></label>
            {/if}
            <label class="f"><span>strength</span><input class="tf-text" inputmode="decimal" bind:value={form.strength} placeholder="1" /></label>
            <label class="f"><span>trigger</span><input class="tf-text" bind:value={form.trigger} placeholder="words added to the prompt, optional" /></label>
            {#if s.family === 'h3'}
              <label class="check"><input type="checkbox" bind:checked={form.always} /> on every clip, never picked by the model</label>
            {/if}
            {#if formError}<span class="bad">{formError}</span>{/if}
            {#if confirmNew}
              <span class="bad">{confirmNew}</span>
              <div class="acts">
                <button class="tf-btn primary" disabled={busy} onclick={() => save(true)}>Save anyway</button>
                <button class="tf-btn" disabled={busy} onclick={() => (confirmNew = null)}>Not yet</button>
              </div>
            {:else}
              <div class="acts">
                <button class="tf-btn primary" disabled={busy} onclick={() => save(false)}>Save</button>
                <button class="tf-btn" disabled={busy} onclick={() => (form = null)}>Cancel</button>
              </div>
            {/if}
          </div>
        {/if}

        <!-- Add: the server's own files that no entry uses yet. -->
        <div class="tf-row stack">
          <span class="tf-name">Add</span>
          {#if d.unused_error}<span class="tf-help bad">{d.unused_error}</span>{/if}
          {#each d.unused.filter((u) => u.family === s.family || showOthers[s.family]) as u (u.file)}
            {@const line = unusedLine(u, s.family)}
            <div class="unused">
              <span class="tf-help mono">{u.file}</span>
              {#if line.text}<span class="tf-help" class:bad={!line.ok}>{line.text}</span>{/if}
              {#if line.ok}
                <button class="tf-btn" disabled={busy} onclick={() => openAdd(s, u.file)}>Add</button>
              {/if}
            </div>
          {/each}
          {#if d.unused.some((u) => u.family !== s.family)}
            <button class="link" onclick={() => (showOthers = { ...showOthers, [s.family]: !showOthers[s.family] })}>
              {showOthers[s.family] ? 'Hide' : 'Show'} {others(d.unused, s.family)}
            </button>
          {/if}
          <button class="link" disabled={busy} onclick={() => openAdd(s)}>Add by file name</button>
        </div>
      {/if}
    </section>
  {/each}
{/if}

<style>
  .hint {
    margin: 0 0 14px;
    color: var(--text-muted);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .notice {
    padding: 10px 12px;
    margin-bottom: 12px;
    font-size: 12.5px;
    border-left: 2px solid var(--hazard);
  }
  .fields {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
  }
  .sub {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  .unlock {
    display: flex;
    gap: 8px;
  }
  .bar {
    display: flex;
    justify-content: flex-end;
  }
  /* Settings' main is a flex column with a fixed height: without this a
     section shrinks to fit and its overflow cuts an open form off. */
  .tf-sec,
  .fields,
  .bar,
  .hint,
  .notice {
    flex: none;
  }
  .tf-sec {
    margin-bottom: 14px;
  }
  .acts {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .editor {
    gap: 8px;
    background: var(--surface);
  }
  .f {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .unused {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 10px;
  }
  .unused .mono {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .mono {
    font-family: var(--mono);
    font-size: 11.5px;
    overflow-wrap: anywhere;
  }
  .tag {
    font-family: var(--mono);
    font-size: 10px;
    padding: 1px 6px;
    border: 1px solid var(--accent-700);
    border-radius: 8px;
    color: var(--accent-300);
  }
  .bad {
    color: var(--hazard);
    font-size: 12px;
  }
  .warn {
    border-color: var(--hazard);
    color: var(--hazard);
  }
  .link {
    align-self: flex-start;
    background: none;
    border: none;
    padding: 4px 0;
    color: var(--accent-400);
    font-size: 12.5px;
    cursor: pointer;
  }
  code {
    font-family: var(--mono);
    font-size: 11.5px;
    overflow-wrap: anywhere;
  }
</style>
