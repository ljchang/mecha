<script>
  // What a persona remembers, and the owner's hand on it (PERSONA-DESIGN
  // §9.8): approve what is waiting, correct a fact, pin, share a fact about
  // you, forget. Every act goes to the owner-guarded route and comes back as
  // the page as it now stands; no tool reaches it, so no model writes here.
  import { apiFetch as fetch } from './api.js';
  import { personaUrl, memorySections, memoryText, memoryActBody, MEMORY_KINDS } from './persona.js';

  let { name, token = null, onclose } = $props();

  let data = $state(null);
  let error = $state('');
  let busy = $state(false);
  // One record at a time is being edited or confirmed: its uid and what.
  let editing = $state(null); // { uid, text }
  let confirming = $state(null); // uid about to be forgotten
  let sharing = $state(null); // uid whose audience is being chosen

  const sections = $derived(memorySections(data));

  // One row mid-act at a time: starting one closes any other.
  function start(what, uid, text = '') {
    editing = what === 'edit' ? { uid, text } : null;
    confirming = what === 'forget' ? uid : null;
    sharing = what === 'share' ? uid : null;
  }

  async function load() {
    error = '';
    try {
      const res = await fetch(personaUrl(name, '/memory', token));
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  async function act(action, id, extra = {}) {
    busy = true;
    error = '';
    try {
      const res = await fetch(personaUrl(name, '/memory', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(memoryActBody(action, id, extra, token)),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      editing = null;
      confirming = null;
      sharing = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    // Again when the persona or the unlock changes under the page.
    void name;
    void token;
    load();
  });
</script>

{#snippet row(r)}
  <li class="mrow" class:pinned={r.pinned}>
    {#if editing?.uid === r.uid}
      <textarea class="medit" rows="2" bind:value={editing.text} aria-label="Corrected wording"></textarea>
      <div class="mbtns">
        <button class="linkbtn" disabled={busy} onclick={() => (editing = null)}>Cancel</button>
        <button class="linkbtn strong" disabled={busy || !editing.text.trim() || editing.text.trim() === r.text}
          onclick={() => act('correct', r.uid, { text: editing.text })}>Save</button>
      </div>
    {:else}
      <div class="mline">
        <span class="mday">{r.day}</span>
        <span class="mtext">{memoryText(r)}</span>
      </div>
      {#if r.section === 'episodes' && r.open_threads?.length}
        <div class="mnote">Left open: {r.open_threads.join('; ')}</div>
      {/if}
      {#if r.origin === 'model_untrusted'}
        <div class="mnote">From a chat that read something from outside.</div>
      {/if}
      {#if r.shared?.length}
        <div class="mshared">
          {#each r.shared as s (s.uid)}
            <span class="mchip">shared with {s.group ?? 'everyone'}
              <button class="x" disabled={busy} aria-label="Stop sharing" title="Stop sharing" onclick={() => act('unshare', s.uid)}>×</button>
            </span>
          {/each}
        </div>
      {/if}
      {#if confirming === r.uid}
        <div class="mbtns">
          <span class="mwarn">Forget this for good? It cannot be brought back.</span>
          <button class="linkbtn" disabled={busy} onclick={() => (confirming = null)}>Keep</button>
          <button class="linkbtn danger" disabled={busy} onclick={() => act('forget', r.uid)}>Forget</button>
        </div>
      {:else if sharing === r.uid}
        <div class="mbtns">
          <span class="mnote">Share with</span>
          <button class="linkbtn" disabled={busy} onclick={() => act('share', r.uid)}>everyone</button>
          {#each data?.groups ?? [] as g}
            <button class="linkbtn" disabled={busy} onclick={() => act('share', r.uid, { group: g })}>{g}</button>
          {/each}
          <button class="linkbtn" disabled={busy} onclick={() => (sharing = null)}>Cancel</button>
        </div>
      {:else}
        <div class="mbtns">
          {#if r.status === 'candidate'}
            <button class="linkbtn strong" disabled={busy} onclick={() => act('approve', r.uid)}>Keep it</button>
          {/if}
          {#if r.section !== 'episodes' && r.status !== 'candidate'}
            <button class="linkbtn" disabled={busy} onclick={() => start('edit', r.uid, r.text)}>Edit</button>
          {/if}
          {#if r.status !== 'candidate'}
            <button class="linkbtn" disabled={busy} onclick={() => act(r.pinned ? 'unpin' : 'pin', r.uid)}>{r.pinned ? 'Unpin' : 'Pin'}</button>
          {/if}
          {#if (r.section === 'user' || r.section === 'inferred') && r.status === 'active'}
            <button class="linkbtn" disabled={busy} onclick={() => start('share', r.uid)}>Share</button>
          {/if}
          <button class="linkbtn" disabled={busy} onclick={() => start('forget', r.uid)}>Forget</button>
        </div>
      {/if}
    {/if}
  </li>
{/snippet}

<div class="memory">
  {#if error}<div class="warnline">{error}</div>{/if}
  {#if data?.shared_problem}
    <div class="warnline">What is shared could not be read ({data.shared_problem}), so share marks are missing here.</div>
  {/if}
  {#if data?.shared_unreadable}
    <div class="warnline">{data.shared_unreadable} shared {data.shared_unreadable === 1 ? 'copy' : 'copies'} this version cannot read — what is shared may be more than shown here. <code>mecha persona memory shared</code> lists them.</div>
  {/if}
  {#if !data && !error}
    <div class="mnote">loading…</div>
  {:else if data && sections.empty}
    <p class="mnote">Nothing remembered yet. Memories are written overnight from your chats.</p>
  {:else if data}
    {#if sections.waiting.length}
      <section>
        <h3>Waiting for you</h3>
        <p class="mnote">From chats that read something from outside. None of these reaches a chat until you keep it.</p>
        <ul>{#each sections.waiting as r (r.uid)}{@render row(r)}{/each}</ul>
      </section>
    {/if}
    {#each MEMORY_KINDS as [kind, label]}
      {#if sections.kept[kind].length}
        <section>
          <h3>{label}</h3>
          <ul>{#each sections.kept[kind] as r (r.uid)}{@render row(r)}{/each}</ul>
        </section>
      {/if}
    {/each}
  {/if}
  <div class="barnote">
    Edit keeps the old wording out of every chat; Forget deletes for good, shared copies too.
  </div>
  <button class="abtn" onclick={() => onclose?.()}>Close</button>
</div>

<style>
  .memory { display: flex; flex-direction: column; gap: 14px; }
  section { display: flex; flex-direction: column; gap: 6px; }
  h3 { margin: 0; font-size: 13px; font-weight: 500; color: var(--text-muted); font-family: var(--mono); text-transform: lowercase; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .mrow { padding: 10px 12px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); display: flex; flex-direction: column; gap: 6px; }
  .mrow.pinned { border-color: var(--accent-700); }
  .mline { display: flex; gap: 10px; align-items: baseline; }
  .mday { flex-shrink: 0; font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  .mtext { font-size: 14px; line-height: 1.45; color: var(--text); overflow-wrap: anywhere; }
  .mnote { margin: 0; font-size: 12px; color: var(--text-muted); line-height: 1.45; }
  .mwarn { font-size: 12px; color: var(--hazard); }
  .mbtns { display: flex; flex-wrap: wrap; align-items: center; gap: 14px; }
  .mshared { display: flex; flex-wrap: wrap; gap: 6px; }
  .mchip { display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px; border: 1px solid var(--accent-900); border-radius: var(--radius-chip); font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  .mchip .x { background: none; border: none; color: var(--text-muted); cursor: pointer; font-size: 14px; padding: 0 2px; min-height: 24px; }
  .medit { width: 100%; box-sizing: border-box; padding: 8px; background: var(--bg); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font: inherit; font-size: 14px; resize: vertical; }
  /* A field under 16px makes iOS zoom the page, and it stays zoomed. */
  @media (pointer: coarse) { .medit { font-size: 16px; } }
  .linkbtn { background: none; border: none; padding: 4px 0; min-height: 32px; color: var(--accent-400); font-size: 12px; cursor: pointer; }
  .linkbtn:disabled { color: var(--text-muted); cursor: default; }
  .linkbtn.strong { font-weight: 600; }
  .linkbtn.danger { color: var(--hazard); font-weight: 600; }
  .warnline { font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .barnote { font-size: 11px; color: var(--text-muted); line-height: 1.5; }
  .abtn { min-height: 44px; padding: 0 16px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; }
</style>
