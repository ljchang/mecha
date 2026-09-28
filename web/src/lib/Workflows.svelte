<script>
  import { apiFetch as fetch } from './api.js';
  import { commitmentLine } from './commitment.js';
  import { workflowGroups } from './home-view.js';
  // The workflows `workflow today` reports — follow-through the assistant is
  // carrying — as a view of the task board (`#tasks/workflows`). It lived on
  // the home page until 2026-09-28, where a list of every open decision was
  // what made home unusable; follow-through is work on tasks, so it sits
  // with them, and home shows one line that opens it.
  //
  // Only workflows: the loose drafts and questions `workflow today` also
  // reports have their own places (the outbox, and this board's waiting
  // view). Two duties from ARCHITECTURE.md §Assistant workflows hold here:
  // overdue work shows during a snooze (the server files it `urgent`, and
  // the urgent group renders whatever the reminder state), and a finished
  // or cancelled workflow keeps its web recovery action — Reopen, below.
  let { navigate = () => {} } = $props();
  let today = $state(null);
  let error = $state('');
  let busy = $state('');

  async function load() {
    try {
      const r = await fetch('/api/today');
      if (!r.ok) throw new Error(await r.text());
      today = await r.json();
      error = '';
    } catch (e) {
      error = String(e.message ?? e);
    }
  }
  // Every thirty seconds while someone is looking, as home does.
  $effect(() => {
    load();
    const seen = () => document.visibilityState === 'visible';
    const timer = setInterval(() => seen() && load(), 30_000);
    const onShow = () => seen() && load();
    document.addEventListener('visibilitychange', onShow);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', onShow);
    };
  });

  async function act(id, action, body = {}) {
    busy = id;
    try {
      const r = await fetch(`/api/workflows/${encodeURIComponent(id)}/${action}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!r.ok) throw new Error(await r.text());
      await load();
    } catch (e) {
      error = String(e.message ?? e);
    } finally {
      busy = '';
    }
  }

  const groups = $derived(workflowGroups(today));
  const closed = $derived(today?.closed ?? []);
  const when = (at) => new Date(at).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
</script>

{#if error}<p class="err" role="alert">{error}</p>{/if}
{#if !today && !error}<div class="empty">Reading your workflows…</div>{/if}
{#if today}
  {#each groups as g (g.key)}
    <section aria-label={g.label}>
    <h2>{g.label} <span>{g.items.length}</span></h2>
    {#each g.items as item (item.id)}
      <article class:urgent={g.key === 'urgent'}>
        <h3>{item.title}</h3>
        <p>{item.state.replaceAll('_', ' ')}{commitmentLine(item.commitment)}</p>
        {#each item.waiting_for ?? [] as reason}<p>{reason}</p>{/each}
        {#if item.notice}<p>{item.notice.reason}</p>{/if}
        {#if item.snoozed_until}<p>Reminders deferred until {when(item.snoozed_until)}</p>{/if}
        {#each item.verification ?? [] as check}<p>{check.passed ? '✓' : 'Needs work:'} {check.evidence}</p>{/each}
        <div class="actions">
          {#if item.outbox?.length}<button onclick={() => navigate('review/outbox')}>Review drafts</button>{/if}
          <button disabled={busy === item.id} onclick={() => act(item.id, 'verify')}>Check completion</button>
          {#if g.key === 'ready'}<button disabled={busy === item.id} onclick={() => act(item.id, 'close')}>Finish workflow</button>{/if}
          <button disabled={busy === item.id} onclick={() => act(item.id, 'snooze', { until: new Date(Date.now() + 86400000).toISOString() })}>Remind me tomorrow</button>
          {#if item.notice}<button disabled={busy === item.id} onclick={() => act(item.id, 'ack')}>Dismiss reminder</button>{/if}
        </div>
      </article>
    {/each}
    </section>
  {:else}
    <div class="empty">No workflow is open.</div>
  {/each}
  {#if closed.length}
    <details class="finished">
      <summary>Finished workflows <span>{closed.length}</span></summary>
      {#each closed as item (item.id)}
        <article>
          <h3>{item.title}</h3>
          <p>{item.state === 'cancelled' ? 'Cancelled' : 'Finished'} {when(item.closed_at)}</p>
          <div class="actions">
            <button disabled={busy === item.id} onclick={() => act(item.id, 'reopen')}>Reopen workflow</button>
          </div>
        </article>
      {/each}
    </details>
  {/if}
{/if}

<style>
  h2 { font-size: 13px; font-weight: 600; color: var(--text-muted); margin: 18px 0 8px; }
  section:first-of-type h2 { margin-top: 4px; }
  h2 span, summary span { font-family: var(--mono); font-weight: 400; margin-left: 6px; }
  h3 { font-size: 15px; margin: 0; font-weight: 500; line-height: 1.4; overflow-wrap: anywhere; }
  article { background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); padding: 14px; margin-bottom: 8px; }
  article.urgent { border-left: 3px solid var(--hazard); }
  p { margin: 4px 0 0; font-size: 13px; color: var(--text-muted); line-height: 1.5; overflow-wrap: anywhere; }
  .actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 10px; }
  button { font: inherit; font-size: 13px; color: var(--text); background: transparent; border: 1px solid var(--accent-900); border-radius: var(--radius-chip); min-height: 40px; padding: 8px 12px; cursor: pointer; }
  button:hover:not(:disabled) { border-color: var(--accent-700); }
  button:disabled { opacity: 0.5; cursor: wait; }
  .finished { margin-top: 18px; }
  summary { cursor: pointer; min-height: 44px; display: flex; align-items: center; font-size: 14px; color: var(--text-muted); }
  .empty { padding: 40px 0; text-align: center; font-size: 14px; color: var(--text-muted); }
  .err { color: var(--hazard); font-size: 13px; overflow-wrap: anywhere; }
</style>
