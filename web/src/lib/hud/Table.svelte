<script>
  // A table panel: the named columns of a dataset's rows, every cell as text.
  // A dataset may be a stranger's text (§3.1), so nothing here is markup. A
  // column may instead be a typed GitHub reference, drawn by `IssueLink` —
  // an anchor only on a tailnet board (`links`) and only for a row the serve
  // route vouched for (`linkable`), its text everywhere else.
  import { formatCell, columnKind, columnHeader } from './hud.js';
  import IssueLink from './IssueLink.svelte';

  /** How many rows a table draws before it says how many more there are. */
  const SHOWN = 200;

  let { columns = [], rows = [], linkable = null, links = false } = $props();
  const shown = $derived(rows.slice(0, SHOWN));
  const kinds = $derived(columns.map(columnKind));
</script>

<div class="wrap">
  <table>
    <thead>
      <tr>{#each columns as c}<th>{columnHeader(c)}</th>{/each}</tr>
    </thead>
    <tbody>
      {#each shown as r}
        <tr>
          {#each kinds as k}
            {#if k.kind === 'github'}
              <td class="ref"><IssueLink reference={k.ref} row={r} {linkable} {links} /></td>
            {:else if k.kind === 'field'}
              <td class:num={typeof r[k.field] === 'number'}>{formatCell(r[k.field])}</td>
            {:else}
              <td>{formatCell(null)}</td>
            {/if}
          {/each}
        </tr>
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
  td.ref { white-space: nowrap; font-variant-numeric: tabular-nums; }
  .more { margin: 6px 0 0; font-size: 12px; color: var(--hud-muted); }
</style>
