<script>
  import Home from './lib/Home.svelte';
  import Chat from './lib/Chat.svelte';
  import Nav from './lib/Nav.svelte';
  import Review from './lib/Review.svelte';
  import Mail from './lib/Mail.svelte';
  import Tasks from './lib/Tasks.svelte';
  import Graph from './lib/Graph.svelte';
  import Settings from './lib/Settings.svelte';

  // Hash routing keeps back/forward and reload honest with zero machinery.
  // A hash may carry a sub-view after a slash (#review/frontdoor), which the
  // view's component interprets; the router only splits it.
  const views = ['home', 'chat', 'mail', 'review', 'tasks', 'graph', 'settings'];
  const fromHash = () => {
    let [h, s] = location.hash.slice(1).split('/');
    // The notes tab folded into graph (NOTES-GRAPH-DESIGN.md D1); the old
    // hash keeps working so bookmarks and habits do.
    if (h === 'notes') h = 'graph';
    return views.includes(h) ? { view: h, sub: s ?? null } : { view: 'home', sub: null };
  };
  let route = $state(fromHash());
  const view = $derived(route.view);

  // Entries are pushed with a depth stamped on them, which is what lets a
  // back gesture know whether there is anywhere to go back *to*. Rewriting
  // the current entry instead (the obvious fix) leaves two entries with the
  // same fragment, and the first Back then moves between them without
  // changing the URL — no event, no re-render, a dead button press.
  function navigate(to, { replace = false } = {}) {
    const [v] = to.split('/');
    route = { view: v, sub: to.split('/')[1] ?? null };
    const url = `${location.pathname}${location.search}${to === 'home' ? '' : `#${to}`}`;
    const depth = history.state?.mechaDepth ?? 0;
    // Navigating to where you already are must not add an entry. Assigning
    // `location.hash` used to dedup this for free; `pushState` does not, and
    // the nav bar fires on the active tab while the gear falls through to
    // here on the settings index — so without this, one tap on either buys a
    // dead Back press, the very thing this function's depth exists to avoid.
    const here = `${location.pathname}${location.search}${location.hash}`;
    if (replace || url === here) history.replaceState({ mechaDepth: depth }, '', url);
    else history.pushState({ mechaDepth: depth + 1 }, '', url);
  }

  /// Undo a navigation. Inside the app that is a real `history.back()`, so the
  /// entry is popped rather than duplicated; on a cold deep link there is
  /// nothing behind us, so rewrite the entry instead of stranding the owner
  /// on a Back that leaves the site.
  function backTo(to) {
    if ((history.state?.mechaDepth ?? 0) > 0) history.back();
    else navigate(to, { replace: true });
  }

  $effect(() => {
    const onNav = () => {
      route = fromHash();
    };
    window.addEventListener('popstate', onNav);
    window.addEventListener('hashchange', onNav);
    return () => {
      window.removeEventListener('popstate', onNav);
      window.removeEventListener('hashchange', onNav);
    };
  });
</script>

<div class="screen">
  {#if view === 'chat'}
    <Chat resume={route.sub} />
  {:else if view === 'mail'}
    <Mail />
  {:else if view === 'review'}
    <Review initial={route.sub} />
  {:else if view === 'tasks'}
    <Tasks />
  {:else if view === 'graph'}
    <Graph initial={route.sub} />
  {:else if view === 'settings'}
    <Settings initial={route.sub} {navigate} {backTo} />
  {:else}
    <Home {navigate} />
  {/if}

  <!-- Settings is chrome, not a seventh place to be, so it does not take a
       slot in the nav — but it has to be reachable from wherever you are.
       One gear, owned by the shell, in the same corner on every view; each
       view's header leaves the corner clear. It sits *below* the app's
       scrims and sheets (z-index 4-6) and drawers (40+) on purpose: a
       button that floated over an open drawer would be a bug you only meet
       on a phone. From inside a settings pane it returns to the index. -->
  <button
    class="gear"
    class:active={view === 'settings'}
    title="settings"
    aria-label="settings"
    onclick={() => (view === 'settings' && route.sub ? backTo('settings') : navigate('settings'))}
  >
    <svg viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.01a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h.01a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.01a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  </button>

  <Nav {view} {navigate} />
</div>

<style>
  .screen {
    max-width: 560px;
    margin: 0 auto;
    height: 100dvh;
    display: flex;
    flex-direction: column;
    /* The frame the gear is positioned against — without this it would
       resolve to the viewport and drift off the centred column. */
    position: relative;
  }
  .gear {
    position: absolute;
    top: 17px;
    right: 14px;
    z-index: 3;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    padding: 0;
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
  }
  .gear:hover {
    color: var(--accent-400);
  }
  .gear.active {
    color: var(--accent-400);
  }
</style>
