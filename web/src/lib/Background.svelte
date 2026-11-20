<script>
  import { apiFetch as fetch } from './api.js';
  import { features } from './features.svelte.js';
  import { isShown } from './features.js';
  import {
    KIND_WORDS,
    RUN_STATES,
    JOB_STATES,
    assistantRows,
    jobLine,
    jobState,
    openChatHref,
    sectionOf,
    taintBadge,
    when,
  } from './background.js';
  // Tasks → Background (`#tasks/background`): every unattended run in one
  // place (ASSISTANT-FOLLOW-THROUGH-DESIGN.md §Tasks → Background, B1).
  //
  // A view, not a verb. It lists and links; answering is Tasks → Waiting's,
  // stopping a delegated run is its board card's, dismissing a follow-up is
  // its workflow card's — one door per act. Triggers, follow-ups and the
  // machine's jobs come from `/api/background`; delegated task runs from the
  // board the page already holds (`items`), so this adds no graph read.
  let { items = null, boardError = null, questions = [], navigate = () => {} } = $props();
  let body = $state(null);
  let error = $state('');
  let now = $state(Date.now());

  async function load() {
    try {
      const r = await fetch('/api/background');
      if (!r.ok) throw new Error((await r.text()).trim() || `HTTP ${r.status}`);
      body = await r.json();
      error = '';
    } catch (e) {
      error = String(e?.message ?? e);
    }
    now = Date.now();
  }
  // Polled only while this view is mounted and the page is visible — a
  // list that reloads forever is a phone that never sleeps.
  $effect(() => {
    load();
    const seen = () => document.visibilityState === 'visible';
    const timer = setInterval(() => seen() && load(), 15_000);
    const onShow = () => seen() && load();
    document.addEventListener('visibilitychange', onShow);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', onShow);
    };
  });

  const runs = $derived(assistantRows(body, items, questions ?? [], now));
  const triggers = $derived(body ? sectionOf(body, 'triggers') : null);
  const followUps = $derived(body ? sectionOf(body, 'follow_ups') : null);
  const jobs = $derived(body ? sectionOf(body, 'jobs') : null);
  const boardOn = $derived(isShown(features.rows, 'tasks'));

  const stamp = (r) => {
    if (r.state === 'running') return r.started_at ? `started ${when(r.started_at, now)}` : 'in flight';
    const end = r.finished_at ? `ended ${when(r.finished_at, now)}` : null;
    const start = r.started_at && r.started_at !== r.finished_at ? `started ${when(r.started_at, now)}` : null;
    return [start, end].filter(Boolean).join(' · ') || 'no time recorded';
  };
</script>

<div class="bg">
  {#if error}<div class="err">background runs: {error}</div>{/if}
  {#if body === null && !error}<div class="empty">reading the background runs…</div>{/if}

  {#if body}
    <!-- Each store's own failure, in its own words: one unreadable store
         never blanks the others, and an unread store is never "nothing ran". -->
    {#each [['scheduled triggers', triggers], ['woken follow-ups', followUps]] as [label, s]}
      {#if s?.error}<div class="err">{label} could not be read: {s.error}</div>{/if}
      {#if s?.unreadable}<div class="note">{label}: {s.unreadable} record{s.unreadable === 1 ? '' : 's'} could not be read, and may be runs not shown here.</div>{/if}
      {#if s?.truncated}<div class="note">{label}: only the newest are shown — `mecha trigger show &lt;name&gt;` has the rest.</div>{/if}
    {/each}
  {/if}
  {#if !boardOn}
    <div class="note">the task board is off, so delegated task runs are not listed here.</div>
  {:else if items === null && boardError}
    <div class="err">delegated task runs could not be read: {boardError}</div>
  {/if}

  {#if body && !runs.length}
    <!-- "Nothing ran" and "I could not look" are opposite findings: with a
         section unreadable, say only what the readable ones show. -->
    <div class="empty">{triggers?.error || followUps?.error ? 'no runs from the stores that could be read' : 'no assistant runs in the last seven days'}</div>
  {/if}
  {#each runs as r (`${r.kind}:${r.task ?? r.workflow ?? r.name}:${r.started_at ?? r.finished_at ?? ''}`)}
    {@const st = RUN_STATES[r.state]}
    {@const badge = taintBadge(r)}
    {@const href = openChatHref(r)}
    <article class:needs={r.state === 'waiting'}>
      <div class="head">
        <span class="chip">{KIND_WORDS[r.kind] ?? r.kind}</span>
        <span class="chip {st.cls}">{st.word}</span>
        {#if badge === 'outside'}
          <!-- The question card's warning, as a badge: third-party content
               was in this run's conversation. -->
          <span class="chip broke" title="third-party content was in this run's conversation">outside content</span>
        {:else if badge === 'unknown'}
          <span class="chip" title="this run's taint was not recorded — not the same as clean">taint unknown</span>
        {/if}
      </div>
      <h3>{r.name}</h3>
      <p class="when">{stamp(r)}</p>
      {#if r.summary}<p>{r.summary}</p>{/if}
      {#if r.detail}<p class:hazard={r.state === 'failed' || r.state === 'unknown'}>{r.detail}</p>{/if}
      <div class="actions">
        {#if href}<button onclick={() => (location.hash = href)}>Open chat</button>{/if}
        {#if r.state === 'waiting'}
          <button class="ask" onclick={() => navigate('tasks/waiting')}>Answer in Waiting</button>
        {/if}
      </div>
    </article>
  {/each}

  <h2>Scheduled jobs</h2>
  {#if jobs?.unavailable}
    <div class="note">{jobs.unavailable}</div>
  {:else if jobs?.error}
    <div class="err">the jobs could not be read: {jobs.error}</div>
  {:else if jobs?.rows}
    {#if !jobs.rows.length}<div class="note">no mecha-* timers are installed on this machine.</div>{/if}
    {#each jobs.rows as j (j.timer)}
      <div class="job" class:hazard={['failed', 'unknown'].includes(jobState(j.state))}>
        <span class="dot {JOB_STATES[jobState(j.state)].cls}"></span>{jobLine(j, now)}
      </div>
    {/each}
  {/if}
</div>

<style>
  .bg { display: flex; flex-direction: column; gap: 8px; }
  article { background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); padding: 13px 14px; display: flex; flex-direction: column; gap: 6px; }
  article.needs { border-color: var(--accent-400); }
  .head { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; }
  h2 { font-size: 13px; font-weight: 600; color: var(--text-muted); margin: 18px 0 2px; }
  h3 { font-size: 14px; margin: 0; font-weight: 500; line-height: 1.4; overflow-wrap: anywhere; }
  p { margin: 0; font-size: 12.5px; color: var(--text-muted); line-height: 1.5; overflow-wrap: anywhere; }
  p.when { font-family: var(--mono); font-size: 10.5px; }
  .hazard { color: var(--hazard); }
  .actions { display: flex; flex-wrap: wrap; gap: 8px; }
  .actions:empty { display: none; }
  button { font-family: var(--mono); font-size: 11px; color: var(--text); background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); min-height: 40px; padding: 8px 12px; cursor: pointer; }
  button.ask { color: var(--accent-400); border-color: var(--accent-400); }
  .agent { color: var(--accent-400); border-color: var(--accent-700); }
  .needs.chip { color: var(--accent-400); border-color: var(--accent-400); }
  .broke { color: var(--hazard); border-color: var(--hazard); }
  .ready { color: var(--text); border-color: var(--accent-700); }
  .job { font-family: var(--mono); font-size: 11px; color: var(--text-muted); line-height: 1.5; display: flex; gap: 8px; align-items: baseline; overflow-wrap: anywhere; }
  .job.hazard { color: var(--hazard); }
  .dot { flex-shrink: 0; width: 7px; height: 7px; border-radius: 50%; background: var(--accent-900); transform: translateY(-1px); }
  .dot.agent { background: var(--accent-400); }
  .dot.ready { background: var(--accent-700); }
  .dot.broke { background: var(--hazard); }
  .note { font-family: var(--mono); font-size: 11px; color: var(--text-muted); line-height: 1.45; overflow-wrap: anywhere; }
  .err { color: var(--hazard); font-size: 12.5px; line-height: 1.45; overflow-wrap: anywhere; }
  .empty { color: var(--text-muted); font-size: 14px; padding: 20px 0; text-align: center; }
</style>
