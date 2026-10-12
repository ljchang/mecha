<script>
  // `#hud` and `#hud/<id>`: installed dashboards, read-only
  // (`docs/LIVE-DASHBOARD-DESIGN.md` §5.1). The list is the boards
  // `mecha hud install` put in place; a board is its checked spec drawn by
  // `hud/Dashboard.svelte`, fetched on this route only — Vega is hundreds of
  // KB, and the rest of a phone-first app should not carry it.
  //
  // `#hud/proposed:<id>` is a board a conversation proposed (`hud_propose`,
  // §5.3, R18), drawn with the rows it had when proposed, beside its queries
  // and who proposed it: the reviewable object is the board itself. Install
  // sends back the digest this page was given, so a proposal revised since
  // is refused rather than installed unseen. A colon never occurs in a board
  // id, so the two routes cannot collide.
  //
  // Refreshing and validating stay in the CLI.
  // Aliased as `fetch`, as every component does: `check-demo` finds the
  // endpoints a page reaches by its `fetch(` calls.
  import { apiFetch as fetch } from './api.js';

  let { initial = null, navigate } = $props();

  let boards = $state(null);
  let proposals = $state([]);
  let listError = $state(null);
  const PROPOSED = 'proposed:';
  const proposedId = $derived(initial?.startsWith(PROPOSED) ? initial.slice(PROPOSED.length) : null);
  let proposal = $state.raw(null);
  let proposalError = $state(null);
  let acting = $state(false);
  let actionError = $state(null);
  // Raw: a spec is data handed to the renderer, never edited here, and a
  // deep proxy of it is what a chart's copy would have to see through.
  let board = $state.raw(null);
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
      const res = await fetch('/api/hud');
      if (!res.ok) throw new Error((await text(res)) || `HTTP ${res.status}`);
      const got = await res.json();
      boards = got.boards ?? [];
      proposals = got.proposals ?? [];
      listError = null;
    } catch (e) {
      listError = String(e?.message ?? e);
    }
  }

  // Which board load is current: an answer for a board the page has since
  // left must not be drawn against the new board's data route.
  let loading = 0;
  async function loadBoard(id) {
    const mine = ++loading;
    board = null;
    boardError = null;
    try {
      const res = await fetch(`/api/hud/${encodeURIComponent(id)}`);
      if (!res.ok) throw new Error((await text(res)) || `HTTP ${res.status}`);
      const got = await res.json();
      if (mine === loading) board = got;
    } catch (e) {
      if (mine === loading) boardError = String(e?.message ?? e);
    }
  }

  async function loadProposal(id) {
    const mine = ++loading;
    proposal = null;
    proposalError = null;
    actionError = null;
    try {
      const res = await fetch(`/api/hud-proposals/${encodeURIComponent(id)}`);
      if (!res.ok) throw new Error((await text(res)) || `HTTP ${res.status}`);
      const got = await res.json();
      if (mine === loading) proposal = got;
    } catch (e) {
      if (mine === loading) proposalError = String(e?.message ?? e);
    }
  }

  /** A proposal's datasets: the rows it had when proposed, fixed. */
  function proposalSource(id) {
    return {
      async dataset(name, etag) {
        if (etag) return null;
        const res = await fetch(`/api/hud-proposals/${encodeURIComponent(id)}/data/${encodeURIComponent(name)}`, {
          cache: 'no-cache',
        });
        if (!res.ok) throw new Error((await text(res)) || `HTTP ${res.status}`);
        return { dataset: await res.json(), etag: res.headers.get('ETag') ?? 'proposed' };
      },
    };
  }

  /** Install (or replace) exactly what this page was shown, or set it aside. */
  async function act(verb) {
    if (!proposal || acting) return;
    acting = true;
    actionError = null;
    const id = proposal.id;
    try {
      const res = await fetch(`/api/hud-proposals/${encodeURIComponent(id)}/${verb}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: verb === 'accept' ? JSON.stringify({ shown: proposal.digest }) : '{}',
      });
      if (!res.ok) {
        const why = (await text(res)) || `HTTP ${res.status}`;
        // Revised since this page loaded it: show the new one, and say so.
        if (res.status === 409) await loadProposal(id);
        actionError = why;
        return;
      }
      await loadList();
      navigate(verb === 'accept' ? `hud/${id}` : 'hud');
    } catch (e) {
      actionError = String(e?.message ?? e);
    } finally {
      acting = false;
    }
  }

  /** Datasets over the API, revalidated by ETag. */
  function apiSource(id) {
    return {
      async dataset(name, etag) {
        const res = await fetch(`/api/hud/${encodeURIComponent(id)}/data/${encodeURIComponent(name)}`, {
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
    if (proposedId) loadProposal(proposedId);
    else if (initial) loadBoard(initial);
  });

  const status = $derived(boards?.find((b) => b.id === initial) ?? null);
  // Each loader the server called stale, by dataset name, for the badge.
  const stale = $derived(
    Object.fromEntries(
      (status?.loaders ?? []).filter((l) => l.stale).map((l) => [l.name, 'older than two of its refresh periods']),
    ),
  );
  const source = $derived(proposedId ? proposalSource(proposedId) : initial ? apiSource(initial) : null);

  function when(iso) {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime()) || d.getFullYear() < 2000) return 'at an unknown time';
    return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  }

  function staleCount(b) {
    return (b.loaders ?? []).filter((l) => l.stale).length;
  }

  /** Datasets whose file could not be read: a finding, not a healthy board. */
  function unreadable(b) {
    return (b.loaders ?? []).filter((l) => l.dataset_error).length;
  }

  /** The newest `generated_at` of a board's datasets, as the page shows it. */
  function updated(b) {
    const times = (b.loaders ?? []).map((l) => l.generated_at).filter(Boolean).sort();
    if (times.length === 0) return 'no data yet';
    const d = new Date(times[times.length - 1]);
    return `updated ${d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}`;
  }
</script>

{#if proposedId}
  <div class="page">
    <button class="back" onclick={() => navigate('hud')}>← Dashboards</button>
    {#if proposalError}
      <p class="bad">{proposalError}</p>
    {:else if !proposal}
      <p class="muted">Loading…</p>
    {:else}
      <section class="review">
        <p class="what">
          <strong>Proposed</strong> by a conversation {when(proposal.proposed_at)}{#if proposal.replaces}, to <strong>replace</strong> the installed “{proposal.id}”{/if}.
          Drawn below with the rows it had then.
        </p>
        {#if proposal.origin !== 'model_clean'}
          <p class="warn small">Drafted in a conversation that had read text from outside — read the queries before installing.</p>
        {/if}
        <details>
          <summary>{proposal.loaders.length} {proposal.loaders.length === 1 ? 'query' : 'queries'}, run on a schedule once installed</summary>
          {#each proposal.loaders as l (l.name)}
            <p class="qname">{l.name} <span class="muted-inline">from {l.source}</span></p>
            <pre>{l.query.trim()}</pre>
          {/each}
        </details>
        <div class="actions">
          <button class="primary" disabled={acting} onclick={() => act('accept')}>{proposal.replaces ? 'Replace' : 'Install'}</button>
          <button disabled={acting} onclick={() => act('reject')}>Reject</button>
        </div>
        {#if actionError}<p class="bad small">{actionError}</p>{/if}
      </section>
      {#key proposal.digest}
        {#await import('./hud/Dashboard.svelte') then m}
          <m.default spec={proposal.spec} {source} stale={{}} mode="dark" />
        {:catch e}
          <p class="bad">The dashboard renderer did not load: {e?.message ?? e}</p>
        {/await}
      {/key}
    {/if}
  </div>
{:else if initial}
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
    <p class="intro">Ask your assistant to draft one, or install with <code>mecha hud install</code>; each dataset refreshes on its own schedule, with no model involved.</p>
    {#if proposals.length}
      <h2>Proposed</h2>
      <ul class="proposed">
        {#each proposals as p (p.id)}
          <li>
            <button onclick={() => navigate(`hud/${PROPOSED}${p.id}`)} disabled={p.refusals?.length > 0}>
              <span class="title">{p.title ?? p.id}</span>
              <span class="meta">{p.replaces ? `replaces “${p.id}”` : 'new'} · {when(p.proposed_at)}</span>
            </button>
            {#if p.refusals?.length}<p class="bad small">{p.refusals.join('; ')}</p>{/if}
          </li>
        {/each}
      </ul>
      <h2>Installed</h2>
    {/if}
    {#if listError}
      <p class="bad">The dashboards could not be read: {listError}</p>
    {:else if boards === null}
      <p class="muted">Loading…</p>
    {:else if boards.length === 0}
      <p class="muted">No dashboard is installed yet. Ask your assistant for one — it proposes, and you install from here.</p>
    {:else}
      <ul>
        {#each boards as b (b.id)}
          <li>
            <button onclick={() => navigate(`hud/${b.id}`)} disabled={b.refusals?.length > 0}>
              <span class="title">{b.title ?? b.id}</span>
              <span class="meta">
                {b.loaders?.length ?? 0} datasets · {updated(b)}{#if staleCount(b)} · <span class="warn">{staleCount(b)} stale</span>{/if}{#if unreadable(b)} · <span class="warn">{unreadable(b)} unreadable</span>{/if}
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
  h1 { margin: 0 0 6px; font-size: 20px; font-weight: 600; }
  .intro { margin: 0 0 14px; font-size: 13px; color: var(--text-muted); }
  .intro code { font-family: var(--mono); font-size: 12px; }
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
  h2 { margin: 18px 0 8px; font-size: 13px; font-weight: 600; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .proposed li button { border-color: var(--accent, #6b8afd); }
  .review {
    margin: 12px var(--gutter, 16px) 0;
    padding: 12px 14px;
    background: var(--surface);
    border: 1px solid var(--accent, #6b8afd);
    border-radius: var(--radius, 8px);
    font-size: 14px;
  }
  .review .what { margin: 0 0 6px; }
  .review details { margin: 8px 0; font-size: 13px; }
  .review summary { cursor: pointer; color: var(--text-muted); }
  .review pre {
    margin: 4px 0 10px;
    padding: 8px;
    overflow-x: auto;
    font-family: var(--mono);
    font-size: 12px;
    background: var(--bg, #15161e);
    border-radius: 6px;
    white-space: pre;
  }
  .qname { margin: 8px 0 0; font-family: var(--mono); font-size: 12px; }
  .muted-inline { color: var(--text-muted); font-family: inherit; }
  .actions { display: flex; gap: 8px; margin-top: 8px; }
  .actions button {
    font: inherit;
    padding: 8px 16px;
    border-radius: var(--radius, 8px);
    border: 1px solid #2c2e3c;
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
  }
  .actions button.primary { background: var(--accent, #6b8afd); border-color: transparent; color: #fff; }
  .actions button:disabled { opacity: 0.6; cursor: default; }
  .review .warn { color: var(--hazard); }
  .review .bad { padding: 0; }
</style>
