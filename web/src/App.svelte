<script>
  import Home from './lib/Home.svelte';
  import Chat from './lib/Chat.svelte';
  import Nav from './lib/Nav.svelte';

  // Hash routing keeps back/forward and reload honest with zero machinery.
  let view = $state(location.hash === '#chat' ? 'chat' : 'home');

  function navigate(to) {
    view = to;
    location.hash = to === 'home' ? '' : to;
  }

  $effect(() => {
    const onHash = () => {
      view = location.hash === '#chat' ? 'chat' : 'home';
    };
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
  });
</script>

<div class="screen">
  {#if view === 'chat'}
    <Chat />
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
