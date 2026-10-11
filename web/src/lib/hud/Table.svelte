<script>
  // A table panel: the named columns of a dataset's rows, every cell as text.
  // A dataset may be a stranger's text (§3.1), so nothing here is markup.
  import { formatCell } from './hud.js';

  /** How many rows a table draws before it says how many more there are. */
  const SHOWN = 200;

  let { columns = [], rows = [] } = $props();
  const shown = $derived(rows.slice(0, SHOWN));
</script>

<div class="wrap">
  <table>
    <thead>
      <tr>{#each columns as c}<th>{c}</th>{/each}</tr>
    </thead>
    <tbody>
      {#each shown as r}
        <tr>{#each columns as c}<td class:num={typeof r[c] === 'number'}>{formatCell(r[c])}</td>{/each}</tr>
      {/each}
    </tbody>
  </table>
  {#if rows.length > SHOWN}<p class="more">{rows.length - SHOWN} more rows not shown</p>{/if}
</div>

<style>
  .wrap { overflow-x: auto; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th { text-align: left; font-weight: 500; color: var(--hud-muted); padding: 4px 8px; border-bottom: 1px solid var(--hud-line); white-space: nowrap; }
  td { padding: 4px 8px; border-bottom: 1px solid var(--hud-line-soft); color: var(--hud-text); overflow-wrap: anywhere; }
  td.num { text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .more { margin: 6px 0 0; font-size: 12px; color: var(--hud-muted); }
</style>
