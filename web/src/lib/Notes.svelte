<script>
  import Dictate from './Dictate.svelte';
  // Notes: the owner's own words into the graph, as evidence. A capture is
  // `mecha kg note` — an episode the nightly extractor mines, with anything
  // it derives waiting in the review queue, never entering belief directly.
  // The recent list is `kg_notes` (the notebook view the store gained for
  // exactly this page); the search box is `kg find`.
  let draft = $state('');
  let recent = $state(null); // kg_notes, newest first

  async function loadRecent() {
    try {
      const res = await fetch('/api/notes');
      if (res.ok) recent = (await res.json()).notes ?? [];
    } catch {
      // the list is a convenience; the capture is the point
    }
  }
  loadRecent();
  let busy = $state(false);
  let error = $state(null);
  let query = $state('');
  let results = $state(null);
  let searching = $state(false);

  // A note opens where it sits: tap the row to read all of it, tap Edit to
  // rewrite it. Collapsed rows are clamped to three lines, which is what
  // makes "expand" mean something — an unclamped list of long notes is a
  // wall of text with no notion of an item in it.
  //
  // The row is keyed by `uid` for display and written by `source_id`,
  // deliberately not the same field: `uid` names the episode row, and the
  // only key that can WRITE to it is (source, source_id), the graph's
  // idempotence key. Re-upserting under it updates the note in place; a
  // fresh id would leave the old wording sitting beside the new one, both
  // reading as true.
  let open_ = $state(null); // uid of the expanded row
  let editing = $state(null); // uid being rewritten
  let editText = $state('');
  let saving = $state(false);

  function toggle(note) {
    if (editing === note.uid) return; // an open editor is not a toggle target
    open_ = open_ === note.uid ? null : note.uid;
  }

  function startEdit(note) {
    editing = note.uid;
    editText = note.body;
    error = null;
  }

  function cancelEdit() {
    editing = null;
    editText = '';
  }

  async function saveEdit(note) {
    const text = editText.trim();
    if (!text || text === note.body) {
      cancelEdit();
      return;
    }
    saving = true;
    try {
      const res = await fetch('/api/notes/edit', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ source_id: note.source_id, text }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      cancelEdit();
      error = null;
      // Reload rather than patching the row in place: the graph decides what
      // the note now says (an edit whose text hashes the same is a no-op
      // there), and a row painted from what was typed would be this page's
      // guess about a store it does not own.
      loadRecent();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      saving = false;
    }
  }

  async function capture() {
    const text = draft.trim();
    if (!text) return;
    busy = true;
    try {
      const res = await fetch('/api/notes', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ text }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      draft = '';
      error = null;
      loadRecent();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function find() {
    const q = query.trim();
    if (!q) {
      results = null;
      return;
    }
    searching = true;
    try {
      const res = await fetch(`/api/find?q=${encodeURIComponent(q)}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      results = Array.isArray(data) ? data : (data.items ?? []);
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      searching = false;
    }
  }
</script>

{#snippet hazardGlyph(size = 12)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

<div class="page">
  <header>
    <span class="title">Notes</span>
    <span class="chip">graph · kg note</span>
  </header>

  <div class="scroll">
    {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}

    <div class="capture card">
      <textarea
        rows="3"
        placeholder="Capture a note — entities named in it are linked on landing"
        bind:value={draft}
      ></textarea>
      <div class="capturerow">
        <Dictate onText={(text, err) => { if (text) draft = draft ? `${draft} ${text}` : text; if (err) error = err; }} />
        <button class="btn primary grow" disabled={busy || !draft.trim()} onclick={capture}>
          {busy ? 'staging…' : 'Capture'}
        </button>
      </div>
    </div>

    <div class="kicker">Recent</div>
    {#if recent === null}
      <div class="empty">reading the notebook…</div>
    {:else}
      {#each recent as note}
        <div class="card noterow">
          {#if editing === note.uid}
            <textarea class="editbox" rows="6" bind:value={editText}></textarea>
            <div class="editrow">
              <button class="btn slim" disabled={saving} onclick={cancelEdit}>Cancel</button>
              <button
                class="btn primary grow"
                disabled={saving || !editText.trim()}
                onclick={() => saveEdit(note)}
              >{saving ? 'saving…' : 'Save'}</button>
            </div>
            <div class="editfoot">
              Rewrites the note in place, keeping when it happened. Anything the graph already
              derived from the old wording stays in your review queue — an edit is not a retraction.
            </div>
          {:else}
            <button class="notehead" onclick={() => toggle(note)}>
              <div class="notetext" class:clamp={open_ !== note.uid}>{note.body}</div>
              <div class="notemeta">{(note.occurred_at ?? '').slice(0, 16).replace('T', ' ')}</div>
            </button>
            {#if open_ === note.uid}
              <div class="editrow">
                <button class="btn slim grow" onclick={() => startEdit(note)}>Edit</button>
              </div>
            {/if}
          {/if}
        </div>
      {:else}
        <div class="empty">Nothing captured yet.</div>
      {/each}
    {/if}

    <div class="kicker">Search the graph</div>
    <div class="searchrow">
      <input
        class="field"
        placeholder="a person, a project, a fact…"
        bind:value={query}
        onkeydown={(e) => e.key === 'Enter' && find()}
      />
      <button class="btn slim" disabled={searching} onclick={find}>{searching ? '…' : 'Find'}</button>
    </div>
    {#if results}
      {#each results as r}
        <div class="card noterow">
          <div class="notetext">{r.statement ?? r.name ?? r.text ?? JSON.stringify(r).slice(0, 140)}</div>
          {#if r.subject || r.kind}
            <div class="notemeta">{r.kind ?? ''}{r.subject ? ` · ${r.subject}` : ''}</div>
          {/if}
        </div>
      {:else}
        <div class="empty">Nothing matched.</div>
      {/each}
    {/if}

    <div class="footnote">
      A note is evidence — what the graph derives from it waits in your review queue.
    </div>
  </div>
</div>

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  header { display: flex; align-items: center; justify-content: space-between; padding: 22px 56px 12px 20px; }
  .title { font-weight: 500; font-size: 17px; letter-spacing: -0.02em; }
  .scroll { flex: 1; overflow-y: auto; padding: 2px 20px 20px; display: flex; flex-direction: column; gap: 10px; }
  .capture { padding: 12px; display: flex; flex-direction: column; gap: 10px; }
  .capturerow { display: flex; gap: 8px; }
  .grow { flex: 1; }
  textarea { background: var(--surface); border: none; border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; line-height: 1.5; padding: 12px 14px; resize: vertical; }
  textarea:focus { outline: 1px solid var(--accent-500); }
  .btn { min-height: 46px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; }
  .btn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .btn.slim { min-width: 72px; }
  .btn:disabled { opacity: 0.5; }
  .noterow { padding: 12px 14px; display: flex; flex-direction: column; gap: 6px; }
  /* The whole row is the target — a tap that only lands on the text is a
     tap most thumbs miss. */
  .notehead { background: none; border: none; padding: 0; margin: 0; color: inherit; font: inherit; text-align: left; display: flex; flex-direction: column; gap: 6px; cursor: pointer; width: 100%; }
  .notetext { font-size: 14px; line-height: 1.5; white-space: pre-wrap; }
  .notetext.clamp { display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .editbox { background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; line-height: 1.5; padding: 12px 14px; resize: vertical; }
  .editbox:focus { outline: 1px solid var(--accent-500); }
  .editrow { display: flex; gap: 8px; }
  .editfoot { font-size: 11px; color: var(--text-muted); line-height: 1.45; }
  .notemeta { font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .searchrow { display: flex; gap: 8px; }
  .field { flex: 1; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 15px; padding: 12px 14px; min-height: 44px; box-sizing: border-box; }
  .warnline { display: flex; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .empty { color: var(--text-muted); font-size: 13px; padding: 8px 0; }
  .footnote { font-size: 11px; color: var(--text-muted); text-align: center; padding-top: 6px; }
  .kicker { margin-top: 8px; }
</style>
