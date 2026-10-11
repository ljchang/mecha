<script>
  // One Vega-Lite panel (`docs/LIVE-DASHBOARD-DESIGN.md` §4.1–§4.2).
  //
  // - `actions: false`, always: the default menu's "Open in Vega Editor"
  //   opens another site and posts it the spec, rows included.
  // - `ast: true`: expressions run in Vega's interpreter, never through the
  //   `Function` constructor the page's CSP forbids.
  // - Rows go in before compile, through the view's `datasets` (`viewFor`).
  // - The tooltip is ours and writes text nodes only: a dataset may be a
  //   stranger's text, and vega-tooltip's default handler assigns HTML.
  // - Vega is imported on first draw, so the app's main bundle never carries it.
  import { viewFor, chartRefusal, tooltipLines } from './hud.js';
  import { vegaConfig } from './theme.js';

  let { chart, rowsByName, mode = 'dark' } = $props();

  let el = $state();
  let width = $state(0);
  // Only crossing the threshold redraws, not every pixel of a resize.
  const narrow = $derived(width > 0 && width < 480);
  let failed = $state(null);
  const refusal = $derived(chartRefusal(chart));

  let tip = null;
  function tooltip(_handler, event, _item, value) {
    const lines = tooltipLines(value);
    if (lines.length === 0 || !event) {
      tip?.remove();
      tip = null;
      return;
    }
    if (!tip) {
      tip = document.createElement('div');
      tip.className = 'hud-tip';
      document.body.appendChild(tip);
    }
    tip.replaceChildren(
      ...lines.map((l) => {
        const d = document.createElement('div');
        d.textContent = l;
        return d;
      }),
    );
    tip.style.setProperty('left', `${Math.round(event.clientX + 12)}px`);
    tip.style.setProperty('top', `${Math.round(event.clientY + 12)}px`);
  }

  let shown = null;
  $effect(() => {
    if (!el || refusal) return;
    const view = viewFor(chart, rowsByName);
    const config = vegaConfig(mode, narrow);
    let cancelled = false;
    // Each draw gets its own target, hidden until it is drawn: datasets land
    // one at a time, so a second draw can start before the first finishes,
    // and two embeds into one element would leave whichever finished last —
    // possibly the older rows (review of #646). A superseded draw is thrown
    // away; the newest one replaces the chart.
    const target = document.createElement('div');
    target.className = 'vega-target pending';
    el.appendChild(target);
    (async () => {
      try {
        const { default: embed } = await import('vega-embed');
        if (cancelled) {
          target.remove();
          return;
        }
        const result = await embed(target, view, {
          mode: 'vega-lite',
          actions: false,
          ast: true,
          renderer: 'svg',
          defaultStyle: false,
          config,
          tooltip,
        });
        if (cancelled) {
          result.finalize();
          target.remove();
          return;
        }
        shown?.finalize();
        for (const old of [...el.children]) if (old !== target) old.remove();
        target.classList.remove('pending');
        shown = result;
        failed = null;
      } catch (e) {
        target.remove();
        if (!cancelled) failed = String(e?.message ?? e);
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  $effect(() => () => {
    shown?.finalize();
    tip?.remove();
  });
</script>

{#if refusal}
  <p class="note bad">Not drawn: {refusal}</p>
{:else}
  {#if failed}<p class="note bad">This chart could not be drawn: {failed}</p>{/if}
  <div class="chart" class:hidden={failed} bind:this={el} bind:clientWidth={width}></div>
{/if}

<style>
  .chart { width: 100%; min-height: 120px; position: relative; }
  /* In the flow's width, so a `container` chart measures right, but unseen
     until drawn. */
  .chart :global(.vega-target.pending) { position: absolute; left: 0; right: 0; top: 0; visibility: hidden; }
  .chart.hidden { display: none; }
  .note { margin: 0; font-size: 13px; color: var(--hud-muted); }
  .bad { color: var(--hud-bad); }
  :global(.hud-tip) {
    position: fixed;
    z-index: 50;
    pointer-events: none;
    max-width: 320px;
    padding: 6px 8px;
    border-radius: 6px;
    font: 12px/1.45 Inter, system-ui, sans-serif;
    background: #0c0d16;
    color: #e9e9ed;
    border: 1px solid #34364a;
    overflow-wrap: anywhere;
  }
</style>
