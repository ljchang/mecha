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
    (async () => {
      try {
        const { default: embed } = await import('vega-embed');
        if (cancelled) return;
        const result = await embed(el, view, {
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
          return;
        }
        shown?.finalize();
        shown = result;
        failed = null;
      } catch (e) {
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
  .chart { width: 100%; min-height: 120px; }
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
