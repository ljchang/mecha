<script>
  // The review tab: two queues, one place. Outbox keeps its own confirm and
  // taint machinery; the graph pane is the sample deck. Each pane is its own
  // component so neither's rules leak into the other.
  import Outbox from './Outbox.svelte';
  import Queue from './Queue.svelte';

  let pane = $state('outbox');
</script>

<div class="review">
  <div class="tabs">
    <button class="tab" class:active={pane === 'outbox'} onclick={() => (pane = 'outbox')}>Outbox</button>
    <button class="tab" class:active={pane === 'graph'} onclick={() => (pane = 'graph')}>Graph queue</button>
  </div>
  {#if pane === 'outbox'}
    <Outbox />
  {:else}
    <div class="scrollwrap"><Queue /></div>
  {/if}
</div>

<style>
  .review {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .tabs {
    display: flex;
    gap: 8px;
    padding: 22px 20px 0;
  }
  .tab {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-muted);
    background: var(--bg);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius-chip);
    padding: 9px 14px;
    min-height: 40px;
    cursor: pointer;
  }
  .tab.active {
    color: var(--text);
    background: var(--accent-900);
    border-color: var(--accent-700);
  }
  .scrollwrap {
    flex: 1;
    overflow-y: auto;
    padding: 14px 20px;
  }
</style>
