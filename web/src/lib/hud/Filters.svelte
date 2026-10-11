<script>
  // The dashboard's filters, drawn by us (§2.3): one select per filter over the
  // distinct values of its column. The choice narrows rows in the host, so
  // every panel bound to the dataset follows without the chart naming it.
  let { filters = [], optionsFor, chosen = $bindable({}) } = $props();

  function pick(id, value) {
    chosen = { ...chosen, [id]: value === '' ? null : value };
  }
</script>

{#if filters.length > 0}
  <div class="filters">
    {#each filters as f (f.id)}
      <label>
        <span>{f.label}</span>
        <select value={chosen[f.id] ?? ''} onchange={(e) => pick(f.id, e.currentTarget.value)}>
          <option value="">All</option>
          {#each optionsFor(f) as v}<option value={v}>{v}</option>{/each}
        </select>
      </label>
    {/each}
  </div>
{/if}

<style>
  .filters { display: flex; flex-wrap: wrap; gap: 10px 16px; }
  label { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--hud-muted); }
  select {
    font: inherit;
    color: var(--hud-text);
    background: var(--hud-panel);
    border: 1px solid var(--hud-line);
    border-radius: 6px;
    padding: 4px 8px;
    min-height: 32px;
  }
</style>
