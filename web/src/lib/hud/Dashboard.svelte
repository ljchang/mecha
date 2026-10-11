<script>
  // A dashboard: a validated spec drawn over its datasets
  // (`docs/LIVE-DASHBOARD-DESIGN.md` §2, §4). One component for both builds —
  // the app's `#hud/<id>` and the standalone `hud.js` — so where the data
  // comes from is a `source`, not a URL baked in here:
  //
  //   source.dataset(name, etag) → { dataset, etag } | null (unchanged)
  //
  // - Datasets are polled no faster than `POLL_FLOOR_MS`, and not at all while
  //   the tab is hidden: a phone left open on a dashboard must not poll all
  //   night (§4.3).
  // - Filters narrow rows here, in the host, so every panel bound to a
  //   dataset follows them (§2.3).
  // - A panel that fails says so in place; one bad chart never blanks the
  //   page. An empty dataset says so; a chart of nothing is not drawn.
  // - Freshness is the data's own `generated_at`, never the viewer's clock.
  import { records, applyFilters, filterOptions, kpi, formatFigure, spanOf, datasetsNamed, pollEvery, asOf } from './hud.js';
  import { untrack } from 'svelte';
  import { scheme as schemeOf } from './theme.js';
  import Chart from './Chart.svelte';
  import Table from './Table.svelte';
  import Text from './Text.svelte';
  import Filters from './Filters.svelte';

  let { spec, source, mode = 'auto', pollMs = 60_000, stale = {}, timeZone = undefined } = $props();

  // name → { dataset?, etag?, error? }. Replaced, never mutated: rows are
  // large and need no deep reactivity.
  let data = $state.raw({});
  let chosen = $state({});

  let prefersLight = $state(false);
  $effect(() => {
    if (typeof matchMedia !== 'function') return;
    const mq = matchMedia('(prefers-color-scheme: light)');
    prefersLight = mq.matches;
    const on = (e) => (prefersLight = e.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  });
  const scheme = $derived(schemeOf(mode === 'auto' ? (prefersLight ? 'light' : 'dark') : mode));

  const all = $derived(Object.fromEntries((spec?.datasets ?? []).map((n) => [n, records(data[n]?.dataset)])));
  const rowsByName = $derived(
    Object.fromEntries(Object.entries(all).map(([n, rows]) => [n, applyFilters(n, rows, spec?.filters, chosen)])),
  );

  async function refreshOne(name) {
    const prev = data[name];
    try {
      const got = await source.dataset(name, prev?.etag ?? null);
      if (got) data = { ...data, [name]: { dataset: got.dataset, etag: got.etag ?? null, error: null } };
      else if (prev?.error) data = { ...data, [name]: { ...prev, error: null } };
    } catch (e) {
      data = { ...data, [name]: { ...(prev ?? {}), error: String(e?.message ?? e) } };
    }
  }

  function refresh() {
    if (document.hidden) return;
    for (const name of spec?.datasets ?? []) refreshOne(name);
  }

  $effect(() => {
    if (!spec || !source) return;
    // Untracked: `refresh` reads `data`, and an effect that tracked it would
    // fetch again every time a dataset landed.
    untrack(refresh);
    const timer = setInterval(refresh, pollEvery(pollMs));
    const onVisible = () => {
      if (!document.hidden) refresh();
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', onVisible);
    };
  });

  /** The datasets a panel draws from. */
  function namesOf(panel) {
    if (panel.type === 'chart') return datasetsNamed(panel.vegalite);
    return panel.dataset ? [panel.dataset] : [];
  }

  /** What a panel can show yet: ready, loading, failed, or empty. */
  function stateOf(panel) {
    const names = namesOf(panel);
    const failed = names.find((n) => data[n]?.error && !data[n]?.dataset);
    if (failed) return { kind: 'failed', why: `${failed}: ${data[failed].error}` };
    if (names.some((n) => !data[n]?.dataset)) return { kind: 'loading' };
    if (names.length > 0 && names.every((n) => (rowsByName[n] ?? []).length === 0)) {
      const filtered = names.some((n) => (all[n] ?? []).length > 0);
      return { kind: 'empty', why: filtered ? 'No rows match the filters.' : 'No rows yet.' };
    }
    return { kind: 'ready' };
  }

  /**
   * Over every dataset a panel draws from: the oldest `generated_at` (a chart
   * is only as fresh as its stalest input), any stale reason, and whether a
   * read failed while older rows stay drawn.
   */
  function freshness(panel) {
    const names = namesOf(panel);
    const times = names
      .map((n) => data[n]?.dataset?.generated_at)
      .filter(Boolean)
      .sort();
    return {
      at: times.length ? asOf(times[0], timeZone) : null,
      stale: names.map((n) => stale[n]).find(Boolean) ?? null,
      lagging: names.some((n) => Boolean(data[n]?.error && data[n]?.dataset)),
    };
  }
</script>

<div class="hud {scheme}">
  <header>
    <h1>{spec?.title ?? ''}</h1>
    <Filters filters={spec?.filters ?? []} optionsFor={(f) => filterOptions(all[f.dataset] ?? [], f.field)} bind:chosen />
  </header>

  <div class="grid">
    {#each spec?.panels ?? [] as panel, i (i)}
      {@const st = stateOf(panel)}
      {@const fresh = freshness(panel)}
      <section class="panel span-{spanOf(panel)}" class:text-panel={panel.type === 'text'}>
        {#if panel.title}<h2>{panel.title}</h2>{/if}
        {#if panel.type === 'text'}
          <Text markdown={panel.markdown} />
        {:else if st.kind === 'failed'}
          <p class="note bad">Could not load {st.why}</p>
        {:else if st.kind === 'loading'}
          <p class="note">Loading…</p>
        {:else if panel.type === 'kpi'}
          <!-- Before the empty state: a count over no rows is a real 0, and
               any other figure over nothing is already a dash. -->
          <p class="figure">{formatFigure(kpi(rowsByName[panel.dataset] ?? [], panel.value))}</p>
        {:else if st.kind === 'empty'}
          <p class="note">{st.why}</p>
        {:else if panel.type === 'chart'}
          <Chart chart={panel.vegalite} {rowsByName} mode={scheme} />
        {:else if panel.type === 'table'}
          <Table columns={panel.columns} rows={rowsByName[panel.dataset] ?? []} />
        {:else}
          <p class="note bad">A panel of a kind this page does not draw.</p>
        {/if}
        {#if panel.type !== 'text' && (fresh.at || fresh.stale || fresh.lagging)}
          <footer>
            {#if fresh.at}<span>{fresh.at}</span>{/if}
            {#if fresh.stale}<span class="badge">stale: {fresh.stale}</span>{/if}
            {#if fresh.lagging}<span class="badge">not refreshed: the last read failed</span>{/if}
          </footer>
        {/if}
      </section>
    {/each}
  </div>
</div>

<style>
  .hud {
    --hud-bg: #161826;
    --hud-panel: #1d1f2c;
    --hud-line: #2c2e3c;
    --hud-line-soft: #232533;
    --hud-text: #e9e9ed;
    --hud-muted: #9a9aa8;
    --hud-bad: #e88a9a;
    --hud-warn: #e8a24a;
    background: var(--hud-bg);
    color: var(--hud-text);
    font-family: Inter, system-ui, sans-serif;
    padding: 16px;
    min-height: 100%;
    box-sizing: border-box;
  }
  .hud.light {
    --hud-bg: #f6f6f9;
    --hud-panel: #ffffff;
    --hud-line: #dcdde5;
    --hud-line-soft: #ececf1;
    --hud-text: #1d1f2b;
    --hud-muted: #5d6070;
    --hud-bad: #b3364d;
    --hud-warn: #9a5b00;
  }
  header { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 14px; }
  h1 { margin: 0; font-size: 20px; font-weight: 600; }
  .grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; }
  .panel {
    background: var(--hud-panel);
    border: 1px solid var(--hud-line);
    border-radius: 10px;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .text-panel { background: transparent; border-style: dashed; }
  .span-1 { grid-column: span 1; }
  .span-2 { grid-column: span 2; }
  .span-3 { grid-column: span 3; }
  .span-4 { grid-column: span 4; }
  h2 { margin: 0; font-size: 13px; font-weight: 500; color: var(--hud-muted); }
  .figure { margin: 0; font-size: 30px; font-weight: 600; font-variant-numeric: tabular-nums; }
  .note { margin: 0; font-size: 13px; color: var(--hud-muted); }
  .bad { color: var(--hud-bad); }
  footer { display: flex; flex-wrap: wrap; gap: 8px; margin-top: auto; font-size: 11px; color: var(--hud-muted); }
  .badge { color: var(--hud-warn); }
  @media (max-width: 900px) {
    .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .span-3, .span-4 { grid-column: span 2; }
  }
  @media (max-width: 560px) {
    .hud { padding: 12px; }
    .grid { grid-template-columns: minmax(0, 1fr); }
    .span-2, .span-3, .span-4 { grid-column: span 1; }
  }
</style>
