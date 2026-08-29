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
  {:else if view === 'graph'}
    <Graph initial={route.sub} />
  {:else if view === 'settings'}
    <Settings />
  {:else}
    <Home {navigate} />
  {/if}
  <Nav {view} {navigate} />
</div>

<style>
  .screen {
    max-width: 560px;
    margin: 0 auto;
    height: 100dvh;
    display: flex;
    flex-direction: column;
  }
</style>
