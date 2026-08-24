<script>
  // Phase 1: a read-only rendering of what the stores hold, straight off
  // `mecha review queues --json` and `mecha doctor --json` — the page adds
  // presentation and nothing else. A depth the server could not read arrives
  // as null and renders as a dash: "nothing waiting" and "could not look"
  // are opposite findings.
  let summary = $state(null);
  let error = $state(null);

  async function load() {
    try {
      const res = await fetch('/api/summary');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      summary = await res.json();
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  load();
  const timer = setInterval(load, 30_000);
  $effect(() => () => clearInterval(timer));

  const dash = (v) => (v === null || v === undefined ? '—' : v.toLocaleString('en-US'));

  // The queue names are the CLI's contract (tui/queues.rs leans on the same
  // strings); the labels here are display only.
  const queueLabels = {
    'outbox drafts': 'Outbox',
    'front-door requests': 'Front door',
    'graph candidates': 'Graph queue',
    'rule proposals': 'Rule proposals',
    'harness changes': 'Harness',
  };
</script>

{#snippet hazardGlyph(size = 13)}
  <svg
    viewBox="0 0 24 24"
    width={size}
    height={size}
    style="flex-shrink: 0"
    fill="none"
    stroke="var(--hazard)"
    stroke-width="1.8"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path d="M12 4l9 16H3z" />
    <path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

<div class="screen">
  <header>
    <div class="wordmark">
      <svg viewBox="0 0 63 54" width="26" height="22" fill="var(--accent-400)" role="img" aria-label="mecha">
        <path d="M0 0h24l7.5 8.5L39 0h24v16H0z" />
        <path d="M0 20h14v15H0zM49 20h14v15H49zM0 39h14v15H0zM49 39h14v15H49z" />
        <path d="M14 39v15h13.24zM49 39v15H35.76z" />
        <path d="M21 24h21v7H21z" />
      </svg>
      <span>mecha</span>
    </div>
    <div class="tailnet chip">
      <span class="dot" style:background={error ? 'var(--accent-700)' : 'var(--accent-400)'}></span>
      {summary?.owner ?? '…'}
    </div>
  </header>

  <main>
    {#if error}
      <div class="card notice">
        {@render hazardGlyph(16)}
        <span>Could not reach the box: {error}</span>
      </div>
    {/if}
    {#each summary?.errors ?? [] as e}
      <div class="card notice">
        {@render hazardGlyph(16)}
        <span>{e}</span>
      </div>
    {/each}

    <section>
      <div class="kicker">Waiting on you</div>
      <div class="grid">
        {#each summary?.queues ?? [] as q}
          <div class="card stat">
            <div class="row">
              <span class="label">{queueLabels[q.queue] ?? q.queue}</span>
              <span class="count">{dash(q.depth)}</span>
            </div>
            <div class="sub" title={q.opens}>{q.detail}</div>
          </div>
        {:else}
          {#if !error}
            <div class="card stat"><div class="sub">loading…</div></div>
          {/if}
        {/each}
      </div>
    </section>

    {#if summary?.doctor?.length}
      <section>
        <div class="kicker">Doctor</div>
        <div class="findings">
          {#each summary.doctor as finding}
            <div class="finding">
              {@render hazardGlyph()}
              <span>
                <span class="component">{finding.component}</span>
                {finding.summary}
              </span>
            </div>
          {/each}
        </div>
      </section>
    {:else if summary && summary.doctor !== null}
      <section>
        <div class="kicker">Doctor</div>
        <div class="finding ok">Nothing silently wrong.</div>
      </section>
    {/if}
  </main>

  <nav>
    <div class="nav-item active">
      <svg viewBox="0 0 24 24" width="21" height="21" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
        <path d="M4 11l8-7 8 7" /><path d="M6 10v10h12V10" />
      </svg>
      <span>home</span>
    </div>
    {#each [['chat', 'M4 5h16v11H9l-5 4z'], ['mail', 'M3 7l9 6 9-6M3 5h18v14H3z'], ['notes', 'M4 4h16v16H4zM8 9h8M8 13h5'], ['review', 'M12 3l9 5-9 5-9-5zM3 13l9 5 9-5'], ['tasks', 'M4 6h2M4 12h2M4 18h2M9 6h11M9 12h11M9 18h11']] as [label, d]}
      <div class="nav-item disabled" title="Phase 2+">
        <svg viewBox="0 0 24 24" width="21" height="21" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
          <path {d} />
        </svg>
        <span>{label}</span>
      </div>
    {/each}
  </nav>
</div>

<style>
  .screen {
    max-width: 560px;
    margin: 0 auto;
    min-height: 100dvh;
    display: flex;
    flex-direction: column;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 22px 20px 14px;
  }
  .wordmark {
    display: flex;
    align-items: center;
    gap: 10px;
    font-weight: 500;
    font-size: 19px;
    letter-spacing: -0.02em;
  }
  .tailnet {
    display: flex;
    align-items: center;
    gap: 7px;
    background: var(--bg);
    padding: 6px 12px;
    font-size: 11px;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }
  main {
    flex: 1;
    padding: 0 20px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }
  .stat {
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }
  .label {
    font-size: 13px;
    color: var(--text-muted);
  }
  .count {
    font-size: 22px;
    font-weight: 500;
    color: var(--accent-400);
  }
  .sub {
    font-size: 11px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .notice {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    font-size: 13px;
    color: var(--text-muted);
  }
  .findings {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .finding {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: 12px;
    color: var(--hazard);
    line-height: 1.45;
  }
  .finding .component {
    font-family: var(--mono);
    color: var(--text-muted);
    margin-right: 4px;
  }
  .finding.ok {
    color: var(--text-muted);
  }
  nav {
    display: flex;
    align-items: stretch;
    justify-content: space-around;
    border-top: 1px solid var(--accent-900);
    background: var(--bg);
    padding: 8px 8px calc(14px + env(safe-area-inset-bottom));
    margin-top: 18px;
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
  }
</style>
