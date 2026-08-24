<script>
  // Notes: the owner's own words into the graph, as evidence. A capture is
  // `mecha kg note` — an episode the nightly extractor mines, with anything
  // it derives waiting in the review queue, never entering belief directly.
  // The search box is `kg find`; recency listing waits for a CLI verb that
  // can answer it (an absence, not an oversight — one implementation rule).
  let draft = $state('');
  let captured = $state([]); // this sitting's captures, newest first
  let busy = $state(false);
  let error = $state(null);
  let query = $state('');
  let results = $state(null);
  let searching = $state(false);

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
      captured.unshift({ text, at: new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) });
      draft = '';
      error = null;
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
      <button class="btn primary" disabled={busy || !draft.trim()} onclick={capture}>
        {busy ? 'staging…' : 'Capture'}
      </button>
    </div>

    {#each captured as note}
      <div class="card noterow">
        <div class="notetext">{note.text}</div>
        <div class="notemeta">captured {note.at} · staged to the graph as evidence</div>
      </div>
    {/each}

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
  header { display: flex; align-items: center; justify-content: space-between; padding: 22px 20px 12px; }
  .title { font-weight: 500; font-size: 17px; letter-spacing: -0.02em; }
  .scroll { flex: 1; overflow-y: auto; padding: 2px 20px 20px; display: flex; flex-direction: column; gap: 10px; }
  .capture { padding: 12px; display: flex; flex-direction: column; gap: 10px; }
  textarea { background: var(--surface); border: none; border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; line-height: 1.5; padding: 12px 14px; resize: vertical; }
  textarea:focus { outline: 1px solid var(--accent-500); }
  .btn { min-height: 46px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; }
  .btn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .btn.slim { min-width: 72px; }
  .btn:disabled { opacity: 0.5; }
  .noterow { padding: 12px 14px; display: flex; flex-direction: column; gap: 6px; }
  .notetext { font-size: 14px; line-height: 1.5; }
  .notemeta { font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .searchrow { display: flex; gap: 8px; }
  .field { flex: 1; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 15px; padding: 12px 14px; min-height: 44px; box-sizing: border-box; }
  .warnline { display: flex; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .empty { color: var(--text-muted); font-size: 13px; padding: 8px 0; }
  .footnote { font-size: 11px; color: var(--text-muted); text-align: center; padding-top: 6px; }
  .kicker { margin-top: 8px; }
</style>
