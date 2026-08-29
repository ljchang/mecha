<script>
  // The learning pane: what the reflection loop has put into every future
  // prompt's cached prefix. Deliberately a read — a learned rule outlives
  // any one run, so retiring one goes through its own staged review rather
  // than a tap here.
  let rules = $state(null);
  let rulesError = $state(null);

  async function load() {
    try {
      const res = await fetch('/api/settings/rules');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      rules = await res.json();
      rulesError = null;
    } catch (e) {
      rulesError = String(e?.message ?? e);
    }
  }
  load();

  const activeRules = $derived((rules ?? []).filter((r) => r.active));
  const retiredRules = $derived((rules ?? []).filter((r) => r.retired));
  // A rate over no observations is undefined, not zero — the corpus makes
  // that distinction everywhere and a dash is how it reads here.
  const dash = (v) => (v === null || v === undefined ? '—' : v);
</script>

<p class="hint">
  What rides in prompts from the learning loop. A read: retiring goes through its own staged
  review, never a tap here.
</p>

{#if rulesError}
  <div class="card notice">could not read the rules: {rulesError}</div>
{:else if rules === null}
  <div class="card"><div class="sub">loading…</div></div>
{:else if rules.length === 0}
  <div class="card"><div class="sub">No rules yet — <code>mecha learn</code> creates them.</div></div>
{:else}
  {#each activeRules as r}
    <div class="rule">
      <div class="rule-head">
        <span class="chip domain">{r.domain}</span>
        {#if r.user}<span class="chip">yours</span>{/if}
        <span class="tally">
          {dash(r.observations)} obs · {dash(r.attributed_regressions)} regressions
        </span>
      </div>
      <div class="rule-text">{r.title}</div>
    </div>
  {/each}
  {#if retiredRules.length}
    <div class="sub retired-count">
      {retiredRules.length} retired rule{retiredRules.length === 1 ? '' : 's'} kept as evidence
      (<code>mecha rules restore</code> un-retires)
    </div>
  {/if}
{/if}

<style>
  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .card {
    padding: 10px 12px;
  }
  .notice {
    color: var(--hazard);
    font-size: 12.5px;
    white-space: pre-wrap;
    font-family: var(--mono);
  }
  .rule {
    background: var(--surface);
    border-radius: var(--radius);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .rule-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .chip.domain {
    color: var(--accent-400);
  }
  .tally {
    margin-left: auto;
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .rule-text {
    font-size: 13px;
    line-height: 1.4;
  }
  .retired-count {
    font-size: 12px;
  }
  code {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--accent-300);
  }
  .sub {
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.4;
  }
</style>
