<script>
  let { view = 'home', navigate } = $props();

  const items = [
    ['home', 'M4 11l8-7 8 7M6 10v10h12V10', true],
    ['chat', 'M4 5h16v11H9l-5 4z', true],
    ['mail', 'M3 7l9 6 9-6M3 5h18v14H3z', true],
    ['notes', 'M4 4h16v16H4zM8 9h8M8 13h5', true],
    ['graph', 'M5 7a2 2 0 104 0 2 2 0 10-4 0M15 17a2 2 0 104 0 2 2 0 10-4 0M15 5.5a2 2 0 104 0 2 2 0 10-4 0M8.7 8.2l5.4 7.4M9 6.7l4-0.5', true],
    ['review', 'M12 3l9 5-9 5-9-5zM3 13l9 5 9-5', true],
    ['tasks', 'M4 6h2M4 12h2M4 18h2M9 6h11M9 12h11M9 18h11', true],
  ];
</script>

<nav>
  {#each items as [label, d, enabled]}
    <button
      class="nav-item"
      class:active={view === label}
      class:disabled={!enabled}
      onclick={() => enabled && navigate(label)}
      title={enabled ? label : 'coming in a later phase'}
    >
      <svg viewBox="0 0 24 24" width="21" height="21" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
        <path {d} />
      </svg>
      <span>{label}</span>
    </button>
  {/each}
</nav>

<style>
  nav {
    display: flex;
    align-items: stretch;
    justify-content: space-around;
    border-top: 1px solid var(--accent-900);
    background: var(--bg);
    padding: 8px 8px calc(14px + env(safe-area-inset-bottom));
  }
  .nav-item {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    min-width: 52px;
    min-height: 44px;
    color: var(--text-muted);
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    font: inherit;
  }
  .nav-item span {
    font-family: var(--mono);
    font-size: 9px;
  }
  .nav-item.active {
    color: var(--accent-400);
  }
  .nav-item.disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
