<script>
  import Home from './lib/Home.svelte';
  import Chat from './lib/Chat.svelte';
  import Nav from './lib/Nav.svelte';
  import Outbox from './lib/Outbox.svelte';

  // Hash routing keeps back/forward and reload honest with zero machinery.
  const views = ['home', 'chat', 'review'];
  const fromHash = () => {
    const h = location.hash.slice(1);
    return views.includes(h) ? h : 'home';
  };
  let view = $state(fromHash());

  function navigate(to) {
    view = to;
    location.hash = to === 'home' ? '' : to;
  }

  $effect(() => {
    const onHash = () => {
      view = fromHash();
    };
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
  });
</script>

<div class="screen">
  {#if view === 'chat'}
    <Chat />
  {:else if view === 'review'}
    <Outbox />
  {:else}
    <Home />
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
