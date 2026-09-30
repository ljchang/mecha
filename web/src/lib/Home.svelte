<script>
  import { apiFetch as fetch } from './api.js';
  import { needsYou, fyi, actionable, workflowGroups, healthLine, queueCount } from './home-view.js';
  import { features } from './features.svelte.js';
  import { isShown, queueCardShown, QUEUE_FEATURE } from './features.js';
  // Home is counts and doors. One card per place, a number, a tap into the
  // tab that has the rest — and nothing that tab already shows. The cards
  // never move: where Mail is today is where it is tomorrow, which is what
  // lets a thumb find it without reading.
  //
  // Four reads, each on its own, so one slow store cannot blank the page.
  // `undefined` is "not read yet", `null` is "could not read": a null is a
  // dash, never a zero — "nothing waiting" and "could not look" are
  // opposite findings.
  let { navigate = () => {} } = $props();
  let summary = $state(undefined);
  let mail = $state(undefined);
  let tasks = $state(undefined);
  let today = $state(undefined);
  let error = $state(null);
  let healthOpen = $state(false);

  async function getJson(path) {
    const res = await fetch(path);
    if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
    return res.json();
  }

  async function load() {
    const [s, m, t, w] = await Promise.allSettled([
      getJson('/api/summary'),
      getJson('/api/mail'),
      getJson('/api/tasks'),
      getJson('/api/today'),
    ]);
    summary = s.status === 'fulfilled' ? s.value : null;
    error = s.status === 'fulfilled' ? null : String(s.reason?.message ?? s.reason);
    mail = m.status === 'fulfilled' && Array.isArray(m.value) ? m.value : null;
    tasks = t.status === 'fulfilled' ? (t.value?.items ?? null) : null;
    today = w.status === 'fulfilled' ? w.value : null;
    if (!summary?.doctor?.length) healthOpen = false;
  }

  // Every thirty seconds while someone is looking; a phone in a pocket
  // does not need its queues re-read, and coming back reads at once.
  load();
  $effect(() => {
    const seen = () => document.visibilityState === 'visible';
    const timer = setInterval(() => seen() && load(), 30_000);
    const onShow = () => seen() && load();
    document.addEventListener('visibilitychange', onShow);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', onShow);
    };
  });

  const dash = (v) => (v === null || v === undefined ? '—' : v.toLocaleString('en-US'));

  const needs = $derived(needsYou(mail));
  const open = $derived(actionable(tasks));
  const overdue = $derived(Array.isArray(open) ? open.filter((t) => t.overdue).length : 0);
  // The same function the workflows view renders, so the line and the page
  // it opens cannot count differently.
  const workflows = $derived(today ? workflowGroups(today).reduce((n, g) => n + g.items.length, 0) : today);
  const health = $derived(healthLine(summary?.doctor));

  // The four places that are yours. `to` is where a tap lands; the question
  // card lands on the board's waiting view, which is where a paused run's
  // question is answered.
  // A backlog card whose count is unknown shows the backlog's own reason.
  const fromQueue = (label, to, sub) => {
    const c = queueCount(summary, cardQueue[label]);
    return { label, to, n: c?.n, sub, why: c?.why };
  };
  // Mail and Tasks follow their features (FEATURES-DESIGN.md §5); the
  // outbox and questions are core stores and always here.
  const mine = $derived(
    [
      isShown(features.rows, 'mail') && { label: 'Mail', to: 'mail', n: Array.isArray(needs) ? needs.length : needs, sub: fyi(mail) ? `need you · ${fyi(mail)} FYI` : 'need you' },
      fromQueue('Outbox', 'review/outbox', 'drafts to review'),
      fromQueue('Questions', 'tasks/waiting', 'a run waits on your answer'),
      isShown(features.rows, 'tasks') && { label: 'Tasks', to: 'tasks', n: Array.isArray(open) ? open.length : open, sub: overdue ? `to do · ${overdue} overdue` : 'to do next', hot: overdue > 0 },
    ].filter(Boolean),
  );

  // The two queues with a large card of their own, by wire name, and not
  // repeated as small cards below. A map rather than literals at each use
  // because the Rust guard reads it: a renamed queue fails `cargo test` here
  // too, instead of leaving the big card reading "could not be read" for a
  // queue that reads fine *and* the same queue back as a small card.
  const queueCards = {
    'outbox drafts': 'Outbox',
    'blocked questions': 'Questions',
  };
  const cardQueue = Object.fromEntries(Object.entries(queueCards).map(([q, label]) => [label, q]));
  // A queue whose feature is off leaves the page only when nothing waits in
  // it: the front door's drain fills its store whatever the switch says,
  // and a count of what waits never degrades (FEATURES-DESIGN.md §5). One
  // kept for that reason says it is off, and what turns it on.
  const machine = $derived(
    (summary?.queues ?? [])
      .filter((q) => !(q.queue in queueCards))
      .filter((q) => queueCardShown(features.rows, q.queue, q.depth ?? null)),
  );
  const offFix = (queue) => {
    const f = QUEUE_FEATURE[queue];
    if (!f || isShown(features.rows, f)) return null;
    return features.rows?.get(f)?.next ?? `mecha features enable ${f}`;
  };

  // Every name `collect_queues()` can push, in its order. A queue missing
  // from here renders under its raw wire name, which is how `blocked
  // questions` shipped titled "blocked questions" and going nowhere for as
  // long as it took someone to tap it: this map is a hardcoded reader of a
  // list produced in Rust, so the drift is silent in both directions.
  // `every_queue_the_backlog_reports_is_named_and_reachable_from_the_web_home`
  // (mecha-cli/src/commands/review.rs) fails `cargo test` instead — it checks
  // both maps against this file *and* against the router's own view and pane
  // lists, because a wrong destination is the silent half: the card keeps its
  // chevron and quietly lands on home.
  const queueLabels = {
    'outbox drafts': 'Outbox',
    'blocked questions': 'Questions',
    'front-door requests': 'Front door',
    'graph candidates': 'Graph queue',
    'graph shadow': 'Shadow verdicts',
    'graph entities': 'Graph entities',
    'rule proposals': 'Rule proposals',
    'harness changes': 'Harness',
    'image candidates': 'Image library',
  };
  // A card that has a surface on this phone navigates to it; the ones that
  // are CLI-only stay flat rather than pretending — and a flat card prints
  // the command that *does* open it, because a card you can tap and a card
  // you cannot have to be told apart before the tap, not after it.
  const queueTargets = {
    'outbox drafts': 'review/outbox',
    'blocked questions': 'tasks/waiting',
    'front-door requests': 'review/frontdoor',
    'graph candidates': 'review/graph',
    'graph shadow': 'review/graph',
    // The three proposal stores share one pane and one component; the pane
    // name is the store, so a card lands on the store it names.
    'harness changes': 'review/harness',
    'rule proposals': 'review/rules',
    'graph entities': 'review/entities',
    'image candidates': 'library/candidates',
  };
</script>

{#snippet hazardGlyph(size = 14)}
  <svg viewBox="0 0 24 24" width={size} height={size} class="glyph" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
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
  <div class="tailnet chip" title="signed in over the tailnet">
    <span class="dot" class:off={error}></span>
    {summary?.owner ?? '…'}
  </div>
</header>

<main>
  {#if error}
    <div class="card notice">{@render hazardGlyph(16)}<span>Could not reach the box: {error}</span></div>
  {/if}
  {#each summary?.errors ?? [] as e}
    <div class="card notice">{@render hazardGlyph(16)}<span>{e}</span></div>
  {/each}

  <section aria-labelledby="mine-h">
    <h2 class="kicker" id="mine-h">Waiting on you</h2>
    <div class="mine">
      {#each mine as c (c.label)}
        <button class="card big" class:zero={c.n === 0} onclick={() => navigate(c.to)}>
          <span class="label">{c.label}<span class="chev" aria-hidden="true">›</span></span>
          <span class="count" class:hot={c.hot}>{c.n === undefined ? '' : dash(c.n)}</span>
          <span class="sub">{c.n === 0 ? 'clear' : c.n === null ? (c.why ?? 'could not be read') : c.sub}</span>
        </button>
      {/each}
    </div>
  </section>

  <section aria-labelledby="queues-h">
    <h2 class="kicker" id="queues-h">Review queues</h2>
    <div class="machine">
      {#each machine as q (q.queue)}
        {@const to = queueTargets[q.queue]}
        {@const fix = offFix(q.queue)}
        {#if fix}
          <!-- Kept because something waits, but its feature is off: the pane
               it would open answers `feature_off`, so the card is flat and
               says what turns it on rather than offering a dead tap. -->
          <div class="card small flat">
            <span class="row"><span class="label">{queueLabels[q.queue] ?? q.queue}</span><span class="count">{dash(q.depth)}</span></span>
            <span class="sub" title={q.detail}>{q.detail}</span>
            <span class="sub off">switched off — <code>{fix}</code></span>
          </div>
        {:else if to}
          <button class="card small" class:zero={q.depth === 0} onclick={() => navigate(to)}>
            <span class="row"><span class="label">{queueLabels[q.queue] ?? q.queue}</span><span class="count">{dash(q.depth)}</span></span>
            <span class="sub" title={q.detail}>{q.detail}</span>
          </button>
        {:else}
          <div class="card small flat">
            <span class="row"><span class="label">{queueLabels[q.queue] ?? q.queue}</span><span class="count">{dash(q.depth)}</span></span>
            <span class="sub" title={q.detail}>{q.detail}</span>
            <code class="opens">{q.opens ?? '—'}</code>
          </div>
        {/if}
      {:else}
        {#if summary === undefined}<div class="card small"><span class="sub">reading the queues…</span></div>{/if}
      {/each}
    </div>
  </section>

  <section class="lines">
    <button class="card line" onclick={() => navigate('tasks/workflows')}>
      <span class="label">Follow-through</span>
      <span class="sub">
        {#if workflows === undefined}reading…{:else if workflows === null}could not be read{:else if workflows === 0}no workflow open{:else}{workflows} workflow{workflows === 1 ? '' : 's'} the assistant is carrying{/if}
      </span>
      <span class="chev" aria-hidden="true">›</span>
    </button>
    {#if summary?.doctor?.length}
      <button class="card line warn" onclick={() => (healthOpen = !healthOpen)} aria-expanded={healthOpen}>
        <span class="label">Health</span>
        <span class="sub">{health}</span>
        <span class="chev" aria-hidden="true">{healthOpen ? '▾' : '▸'}</span>
      </button>
    {:else}
      <div class="card line">
        <span class="label">Health</span>
        <span class="sub">{health ?? (summary === undefined ? 'asking the doctor…' : 'the doctor could not be read')}</span>
      </div>
    {/if}
    {#if healthOpen && summary?.doctor?.length}
      <div class="findings">
        {#each summary.doctor as f, i (i)}
          <div class="finding">
            {#if f.severity === 'broken'}{@render hazardGlyph(13)}{:else}<span class="pip"></span>{/if}
            <span><span class="component">{f.component}</span> {f.summary}</span>
          </div>
        {/each}
      </div>
    {/if}
  </section>
</main>

<style>
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 22px var(--gutter-gear) 14px var(--gutter);
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
    background: var(--accent-400);
  }
  .dot.off {
    background: var(--accent-700);
  }
  main {
    flex: 1;
    overflow-y: auto;
    padding: 6px var(--gutter) 28px;
    display: flex;
    flex-direction: column;
    gap: 26px;
  }
  /* The command is the point of the line, so it wraps rather than being
     ellipsised with the rest of a card's sub-lines. */
  .small .sub.off {
    color: var(--hazard);
    white-space: normal;
    overflow-wrap: anywhere;
  }
  .small .sub.off code {
    font-family: var(--mono);
    font-size: 10px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h2.kicker {
    margin: 0;
    font-weight: 400;
  }
  button {
    font: inherit;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  button.card:hover {
    border-color: var(--accent-700);
  }
  button.card:active {
    background: var(--accent-900);
  }
  button:focus-visible {
    outline: 2px solid var(--accent-500);
    outline-offset: 2px;
  }
  .glyph {
    flex-shrink: 0;
  }
  .notice {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    font-size: 13px;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  /* ---- yours: four large cards, the number is the card ---- */
  .mine {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }
  .big {
    display: flex;
    flex-direction: column;
    padding: 14px 16px 16px;
    min-height: 128px;
  }
  .label {
    font-size: 14px;
    color: var(--text-muted);
  }
  /* The accent means "this opens something" — so a card with nowhere to go
     gives it up. */
  .chev {
    margin-left: 6px;
    color: var(--accent-400);
  }
  .big .count {
    margin-top: auto;
    font-size: 40px;
    line-height: 1.05;
    font-weight: 500;
    letter-spacing: -0.03em;
    color: var(--accent-400);
    font-variant-numeric: tabular-nums;
  }
  .count.hot {
    color: var(--accent-300);
  }
  .sub {
    font-size: 12px;
    line-height: 1.4;
    color: var(--text-muted);
  }
  .big .sub {
    margin-top: 4px;
  }
  /* A zero is a quiet number: the eye goes to what has something in it. */
  .zero .count {
    color: var(--text-muted);
    opacity: 0.45;
  }

  /* ---- the machine's queues: smaller, one line of detail each ---- */
  .machine {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }
  .small {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 10px 12px;
    min-width: 0;
  }
  .small .row {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
  }
  .small .label {
    font-size: 13px;
  }
  .small .count {
    font-size: 17px;
    font-weight: 500;
    color: var(--accent-400);
    font-variant-numeric: tabular-nums;
  }
  .small .sub {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .flat .count {
    color: var(--text-muted);
  }
  /* A CLI-only queue prints the command that opens it — wrapped, never
     clipped: a command with its tail cut off is one you cannot run. */
  .opens {
    margin-top: auto;
    padding-top: 7px;
    border-top: 1px solid var(--accent-900);
    font-family: var(--mono);
    font-size: 10px;
    line-height: 1.5;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  /* ---- follow-through and health: one line each ---- */
  .lines {
    gap: 8px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    min-height: 48px;
  }
  .line .label {
    color: var(--text);
    flex-shrink: 0;
  }
  .line .sub {
    flex: 1;
    min-width: 0;
    font-size: 13px;
  }
  .line.warn .sub {
    color: var(--hazard);
  }
  .findings {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 4px 16px 0;
  }
  .finding {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }
  .finding .glyph {
    margin-top: 2px;
  }
  .pip {
    width: 6px;
    height: 6px;
    margin: 5px 4px 0 3px;
    flex-shrink: 0;
    border-radius: 50%;
    border: 1.5px solid var(--hazard);
  }
  .component {
    font-family: var(--mono);
    color: var(--hazard);
    margin-right: 2px;
  }

  /* The wide window reads across: four of yours in a row, the queues three
     to a row, follow-through and health side by side. */
  @media (min-width: 900px) {
    main {
      padding-top: 20px;
      gap: 32px;
    }
    .mine {
      grid-template-columns: repeat(4, minmax(0, 1fr));
      gap: 14px;
    }
    .big {
      min-height: 150px;
      padding: 16px 18px 18px;
    }
    .big .count {
      font-size: 48px;
    }
    .machine {
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 10px;
    }
    .lines {
      display: grid;
      grid-template-columns: 1fr 1fr;
    }
    .findings {
      grid-column: 1 / -1;
    }
  }
</style>
