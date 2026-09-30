<script>
  import { features, loadFeatures } from './features.svelte.js';
  import { tree, detail } from './features.js';

  // Every optional part of mecha, off ones included, each with the command
  // that turns it on. Hidden from the nav is never undiscoverable: this is
  // where a feature that is off is still findable (FEATURES-DESIGN.md §4.2
  // item 3). The page shows each command and runs none of them — enabling
  // most features means installing a binary, authorising an account or
  // starting a server, which is a terminal's job on this machine (§8).
  //
  // Re-read on open, so a switch flipped in a terminal a moment ago is
  // what this lists.
  loadFeatures();

  const rows = $derived(tree(features.body));
  const unknown = $derived(features.body?.unknown_switches ?? []);
  const pending = $derived(rows.some((r) => r.pending));
</script>

<p class="hint">
  What this install has on. A feature that is off is gone from the rest of the app; the command beside it turns it on,
  run in a terminal on this machine.
</p>

{#if features.error}
  <div class="card notice">
    Could not read the features — nothing is hidden until they can be: {features.error}
  </div>
{:else if features.body === undefined}
  <p class="hint">reading…</p>
{/if}

{#if pending}
  <div class="card notice">
    Some switches changed since <code>mecha serve</code> started. The pages follow at once; the chat's tools follow when it
    restarts.
  </div>
{/if}

{#each unknown as key}
  <div class="card notice">
    <code>[features] {key}</code> is not a feature this build knows — a newer build's, or a typo. It is ignored.
  </div>
{/each}

<div class="list">
  {#each rows as r (r.id)}
    <div class="feature" class:part={r.depth > 0} style:--depth={r.depth}>
      <div class="top">
        <span class="name">{r.label}</span>
        <span class="state {r.state}">{r.state}</span>
      </div>
      <div class="meta">
        <code class="id">{r.id}</code>
        {#if detail(r, features.rows)}<span class="detail">{detail(r, features.rows)}</span>{/if}
      </div>
      {#if r.in_use}
        <div class="flag">set up on this machine, but not switched on</div>
      {/if}
      {#if r.pending}
        <div class="flag">changed since <code>mecha serve</code> started — restart it for the chat's tools</div>
      {/if}
      {#if r.next}
        <code class="next">{r.next}</code>
      {/if}
    </div>
  {/each}
</div>

<style>
  .hint {
    margin: 0 0 14px;
    color: var(--text-muted);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .notice {
    padding: 10px 12px;
    margin-bottom: 12px;
    font-size: 12.5px;
    line-height: 1.45;
    border-left: 2px solid var(--hazard);
  }
  code {
    font-family: var(--mono);
    font-size: 11px;
  }
  .list {
    display: flex;
    flex-direction: column;
  }
  .feature {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 12px 0 12px calc(var(--depth) * 16px);
    border-top: 1px solid var(--accent-900);
  }
  .feature:first-child {
    border-top: none;
  }
  .top {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
  }
  .name {
    font-size: 14px;
  }
  .part .name {
    font-size: 13px;
  }
  .state {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--text-muted);
    flex-shrink: 0;
  }
  .state.on {
    color: var(--accent-400);
  }
  .state.unready,
  .state.unknown {
    color: var(--hazard);
  }
  .meta {
    display: flex;
    gap: 8px;
    align-items: baseline;
    min-width: 0;
    font-size: 12px;
    color: var(--text-muted);
  }
  .id {
    flex-shrink: 0;
  }
  .detail {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .flag {
    font-size: 12px;
    color: var(--hazard);
  }
  .next {
    align-self: flex-start;
    max-width: 100%;
    overflow-wrap: anywhere;
    padding: 5px 8px;
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--accent-400);
    user-select: all;
  }
</style>
