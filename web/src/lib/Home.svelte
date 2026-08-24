<script>
  // The read-only dashboard: `mecha review queues --json` and
  // `mecha doctor --json`, rendered. A null depth is a dash — "nothing
  // waiting" and "could not look" are opposite findings.
  let { navigate = () => {} } = $props();
  let summary = $state(null);
  let mailNeeds = $state(null);
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
    try {
      const res = await fetch('/api/mail');
      if (res.ok) mailNeeds = (await res.json()).filter((r) => r.needs_me).length;
    } catch {
      mailNeeds = null; // a dash, never a zero
    }
  }

  load();
  const timer = setInterval(load, 30_000);
  $effect(() => () => clearInterval(timer));

  const dash = (v) => (v === null || v === undefined ? '—' : v.toLocaleString('en-US'));

  const queueLabels = {
    'outbox drafts': 'Outbox',
    'front-door requests': 'Front door',
    'graph candidates': 'Graph queue',
    'rule proposals': 'Rule proposals',
    'harness changes': 'Harness',
  };
  // A card that has a surface on this phone navigates to it; the ones that
  // are CLI-only stay flat rather than pretending.
  const queueTargets = {
    'outbox drafts': 'review/outbox',
    'front-door requests': 'review/frontdoor',
    'graph candidates': 'review/graph',
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
      <button class="card stat tappable" onclick={() => navigate('mail')}>
        <div class="row">
          <span class="label">Mail</span>
          <span class="count">{dash(mailNeeds)}</span>
        </div>
        <div class="sub">threads that need you</div>
      </button>
      {#each summary?.queues ?? [] as q}
        {#if queueTargets[q.queue]}
          <button class="card stat tappable" onclick={() => navigate(queueTargets[q.queue])}>
            <div class="row">
              <span class="label">{queueLabels[q.queue] ?? q.queue}</span>
              <span class="count">{dash(q.depth)}</span>
            </div>
            <div class="sub" title={q.opens}>{q.detail}</div>
          </button>
        {:else}
          <div class="card stat">
            <div class="row">
              <span class="label">{queueLabels[q.queue] ?? q.queue}</span>
              <span class="count">{dash(q.depth)}</span>
            </div>
            <div class="sub" title={q.opens}>{q.detail}</div>
          </div>
        {/if}
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

<style>
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
    overflow-y: auto;
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
  .tappable {
    text-align: left;
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  .tappable:active {
    background: var(--accent-900);
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
</style>
