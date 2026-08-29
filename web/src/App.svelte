<script>
  import Home from './lib/Home.svelte';
  import Chat from './lib/Chat.svelte';
  import Nav from './lib/Nav.svelte';
  import Review from './lib/Review.svelte';
  import Mail from './lib/Mail.svelte';
  import Tasks from './lib/Tasks.svelte';
  import Notes from './lib/Notes.svelte';
  import Entity from './lib/Entity.svelte';
  import Settings from './lib/Settings.svelte';

  // Hash routing keeps back/forward and reload honest with zero machinery.
  // A hash may carry a sub-view after a slash (#review/frontdoor), which the
  // view's component interprets; the router only splits it.
  const views = ['home', 'chat', 'mail', 'review', 'tasks', 'notes', 'graph', 'settings'];
  const fromHash = () => {
    const [h, s] = location.hash.slice(1).split('/');
    return views.includes(h) ? { view: h, sub: s ?? null } : { view: 'home', sub: null };
  };
  let route = $state(fromHash());
  const view = $derived(route.view);

  function navigate(to) {
    const [v] = to.split('/');
    route = { view: v, sub: to.split('/')[1] ?? null };
    location.hash = to === 'home' ? '' : to;
  }

  $effect(() => {
    const onHash = () => {
      route = fromHash();
    };
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
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
  {:else if view === 'notes'}
    <Notes />
  {:else if view === 'graph'}
    <Entity initial={route.sub} />
  {:else if view === 'settings'}
    <Settings initial={route.sub} {navigate} />
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
    onclick={() => navigate('settings')}
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
