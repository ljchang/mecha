<script>
  // The review tab: two queues, one place. Outbox keeps its own confirm and
  // taint machinery; the graph pane is the sample deck. Each pane is its own
  // component so neither's rules leak into the other.
  import Outbox from './Outbox.svelte';
  import Queue from './Queue.svelte';
  import Frontdoor from './Frontdoor.svelte';
  import Proposals from './Proposals.svelte';

  let { initial = null, navigate = () => {} } = $props();
  // The three proposal stores are panes in their own right rather than one
  // `proposals` pane with hidden state: each is a card on the home page, and
  // a card has to land on the store it names. They share one component and
  // one tab — six tabs on a phone is a scroller nobody reads.
  // One literal, read by `every_queue_the_backlog_reports_is_named_and_\
  // reachable_from_the_web_home` in Rust — which parses it as a string array
  // and failed loudly on a `...spread`, which is the guard working. The
  // proposal stores are what is left after the panes that own their own
  // component, so the two lists cannot drift.
  const panes = ['outbox', 'graph', 'frontdoor', 'harness', 'rules', 'entities'];
  const ownPanes = ['outbox', 'graph', 'frontdoor'];
  const proposalStores = panes.filter((p) => !ownPanes.includes(p));
  // Derived, never copied into state — `Settings.svelte` records this trap
  // verbatim: App does not remount this component on a hash change, it only
  // passes a new `initial`, so a `$state` snapshot ignores back/forward. A
  // first pass here synced the snapshot from an effect, which cured the dead
  // Back press but left the other half: the tabs assigned `pane` without
  // navigating, so the URL could name one pane while another was on screen.
  // One direction of truth instead — the route names the pane, every tab and
  // chip navigates, and the two cannot disagree because there is only one.
  const pane = $derived(panes.includes(initial) ? initial : 'outbox');
  const inProposals = $derived(proposalStores.includes(pane));
</script>

<div class="review">
  <div class="tabs">
    <button class="tab" class:active={pane === 'outbox'} onclick={() => navigate('review/outbox')}>Outbox</button>
    <button class="tab" class:active={pane === 'graph'} onclick={() => navigate('review/graph')}>Graph queue</button>
    <button class="tab" class:active={pane === 'frontdoor'} onclick={() => navigate('review/frontdoor')}>Front door</button>
    <button class="tab" class:active={inProposals} onclick={() => navigate('review/harness')}>Proposals</button>
  </div>
  {#if pane === 'outbox'}
    <Outbox />
  {:else if pane === 'frontdoor'}
    <Frontdoor />
  {:else if inProposals}
    <!-- The store selection is the route itself, so Back out of `entities`
         returns to `harness` instead of leaving the tab. -->
    <Proposals store={pane} onstore={(s) => navigate(`review/${s}`)} />
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
    padding: 14px var(--gutter-gear) 0 var(--gutter);
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
    padding: 14px var(--gutter);
  }
</style>
