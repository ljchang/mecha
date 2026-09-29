<script>
  let { view = 'home', navigate } = $props();

  const items = [
    ['home', 'M4 11l8-7 8 7M6 10v10h12V10', true],
    ['chat', 'M4 5h16v11H9l-5 4z', true],
    ['mail', 'M3 7l9 6 9-6M3 5h18v14H3z', true],
    ['graph', 'M5 7a2 2 0 104 0 2 2 0 10-4 0M15 17a2 2 0 104 0 2 2 0 10-4 0M15 5.5a2 2 0 104 0 2 2 0 10-4 0M8.7 8.2l5.4 7.4M9 6.7l4-0.5', true],
    ['review', 'M12 3l9 5-9 5-9-5zM3 13l9 5 9-5', true],
    ['tasks', 'M4 6h2M4 12h2M4 18h2M9 6h11M9 12h11M9 18h11', true],
    ['personas', 'M12 12a4 4 0 100-8 4 4 0 000 8zM4.5 20c.8-3.5 3.8-5.5 7.5-5.5s6.7 2 7.5 5.5', true],
    ['library', 'M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4M15.5 9.5h.01', true],
  ];
  // Settings deliberately takes no slot here: it is chrome, and the shell
  // owns one gear in the same corner of every view (#118) — which is what
  // makes it reachable from anywhere without being a seventh place to be.
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
    /* Shared, not fixed: eight places at a fixed 52px need 432px and pushed
       the last tab off a 375px phone when Personas became the eighth. Each
       now takes an equal share — about 45px there, still a thumb's width. */
    flex: 1 1 0;
    min-width: 0;
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
    white-space: nowrap;
  }
  .nav-item.active {
    color: var(--accent-400);
  }
  .nav-item.disabled {
    opacity: 0.45;
    cursor: default;
  }

  /* The same places, turned ninety degrees. A bottom bar is a thumb
     affordance; on a desktop the thumb is a cursor and the bottom of a
     1400px window is the furthest point from where the eye already is —
     so the bar becomes a left rail. `order: -1` does the moving: the shell
     turns `.screen` into a row at the same breakpoint and this element is
     still written last in the markup, which is where it belongs for a
     screen reader and for a phone. */
  @media (min-width: 900px) {
    nav {
      order: -1;
      flex-direction: column;
      justify-content: flex-start;
      gap: 2px;
      width: var(--rail);
      flex-shrink: 0;
      overflow-y: auto;
      border-top: none;
      border-right: 1px solid var(--accent-900);
      background: var(--void);
      /* Room at the foot for the shell's gear, which docks to the rail on
         this breakpoint rather than floating over the content. */
      padding: 14px 8px 76px;
    }
    .nav-item {
      /* A column: sharing the height would stretch each to fill the rail. */
      flex: none;
      width: 100%;
      padding: 10px 0;
      border-radius: var(--radius);
    }
    .nav-item:not(.disabled):hover {
      color: var(--text);
      background: var(--bg);
    }
    .nav-item.active {
      background: var(--accent-900);
    }
  }
</style>
