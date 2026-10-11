<script>
  // `#hud` and `#hud/<id>`: installed dashboards, read-only
  // (`docs/LIVE-DASHBOARD-DESIGN.md` §5.1). The list is the boards
  // `mecha hud install` put in place; a board is its checked spec drawn by
  // `hud/Dashboard.svelte`, fetched on this route only — Vega is hundreds of
  // KB, and the rest of a phone-first app should not carry it.
  //
  // Installing, refreshing and validating stay in the CLI.
  import { apiFetch } from './api.js';

  let { initial = null, navigate } = $props();

  let boards = $state(null);
  let listError = $state(null);
  let board = $state(null);
  let boardError = $state(null);

  async function text(res) {
    try {
      return (await res.text()).trim();
    } catch {
      return '';
    }
  }

  async function loadList() {
    try {
      const res = await apiFetch('/api/hud');
      if (!res.ok) throw new Error((await text(res)) || `HTTP ${res.status}`);
      boards = (await res.json()).boards ?? [];
      listError = null;
    } catch (e) {
      listError = String(e?.message ?? e);
    }
  }

  async function loadBoard(id) {
    board = null;
    boardError = null;
    try {
      const res = await apiFetch(`/api/hud/${encodeURIComponent(id)}`);
      if (!res.ok) throw new Error((await text(res)) || `HTTP ${res.status}`);
      board = await res.json();
    } catch (e) {
      boardError = String(e?.message ?? e);
    }
  }

  /** Datasets over the API, revalidated by ETag. */
  function apiSource(id) {
    return {
      async dataset(name, etag) {
        const res = await apiFetch(`/api/hud/${encodeURIComponent(id)}/data/${encodeURIComponent(name)}`, {
          cache: 'no-cache',
          headers: etag ? { 'If-None-Match': etag } : {},
        });
        if (res.status === 304) return null;
        if (!res.ok) throw new Error((await text(res)) || `HTTP ${res.status}`);
        return { dataset: await res.json(), etag: res.headers.get('ETag') };
      },
    };
  }

  $effect(() => {
    loadList();
    if (initial) loadBoard(initial);
  });

  const status = $derived(boards?.find((b) => b.id === initial) ?? null);
  // Each loader the server called stale, by dataset name, for the badge.
  const stale = $derived(
    Object.fromEntries(
      (status?.loaders ?? []).filter((l) => l.stale).map((l) => [l.name, 'older than two of its refresh periods']),
    ),
  );
  const source = $derived(initial ? apiSource(initial) : null);

  function staleCount(b) {
    return (b.loaders ?? []).filter((l) => l.stale).length;
  }
</script>

{#if initial}
  <div class="page">
    <button class="back" onclick={() => navigate('hud')}>← Dashboards</button>
    {#if boardError}
      <p class="bad">{boardError}</p>
    {:else if !board}
      <p class="muted">Loading…</p>
    {:else}
      {#await import('./hud/Dashboard.svelte') then m}
        <m.default spec={board.spec} {source} {stale} mode="dark" />
      {:catch e}
        <p class="bad">The dashboard renderer did not load: {e?.message ?? e}</p>
      {/await}
    {/if}
  </div>
{:else}
  <main class="list">
    <h1>Dashboards</h1>
    {#if listError}
      <p class="bad">The dashboards could not be read: {listError}</p>
    {:else if boards === null}
      <p class="muted">Loading…</p>
    {:else if boards.length === 0}
      <p class="muted">No dashboard is installed. <code>mecha hud install &lt;dir&gt;</code> installs one.</p>
    {:else}
      <ul>
        {#each boards as b (b.id)}
          <li>
            <button onclick={() => navigate(`hud/${b.id}`)} disabled={b.refusals?.length > 0}>
              <span class="title">{b.title ?? b.id}</span>
              <span class="meta">
                {b.loaders?.length ?? 0} datasets{#if staleCount(b)} · <span class="warn">{staleCount(b)} stale</span>{/if}
              </span>
            </button>
            {#if b.refusals?.length}<p class="bad small">{b.refusals.join('; ')}</p>{/if}
          </li>
        {/each}
      </ul>
    {/if}
  </main>
{/if}

<style>
  .page { min-height: 100%; }
  .back {
    margin: 12px 16px 0;
    font: inherit;
    font-size: 13px;
    color: var(--text-muted);
    background: none;
    border: 0;
    padding: 4px 0;
    cursor: pointer;
  }
  .list { padding: 20px var(--gutter, 20px); max-width: var(--measure, 880px); }
  h1 { margin: 0 0 14px; font-size: 20px; font-weight: 600; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  li button {
    width: 100%;
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 12px;
    padding: 12px 14px;
    font: inherit;
    text-align: left;
    color: var(--text);
    background: var(--surface);
    border: 1px solid #2c2e3c;
    border-radius: var(--radius, 8px);
    cursor: pointer;
  }
  li button:disabled { cursor: default; opacity: 0.6; }
  .title { font-weight: 500; }
  .meta { font-size: 12px; color: var(--text-muted); }
  .warn { color: var(--hazard); }
  .muted { color: var(--text-muted); padding: 0 16px; }
  .list .muted { padding: 0; }
  .bad { color: #e88a9a; padding: 0 16px; }
  .list .bad { padding: 0; }
  .small { font-size: 12px; margin: 4px 0 0; }
</style>
