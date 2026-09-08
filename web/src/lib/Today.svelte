<script>
  import { apiFetch as fetch } from './api.js';
  import { onMount } from 'svelte';
  let { navigate = () => {} } = $props();
  let data = $state(null);
  let error = $state('');
  let busy = $state('');
  const sections = [['urgent', 'Needs attention'], ['decisions', 'Decisions for you'], ['ready', 'Ready to finish'], ['waiting', 'In progress and waiting']];
  async function load() {
    try {
      const r = await fetch('/api/today');
      if (!r.ok) throw new Error(await r.text());
      data = await r.json(); error = '';
    } catch (e) { data = null; error = String(e.message ?? e); }
  }
  onMount(() => { load(); const timer = setInterval(load, 30000); return () => clearInterval(timer); });
  async function act(id, action, body = {}) {
    busy = id;
    try {
      const r = await fetch(`/api/workflows/${encodeURIComponent(id)}/${action}`, {
        method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body),
      });
      if (!r.ok) throw new Error(await r.text());
      await load();
    } catch (e) { error = String(e.message ?? e); }
    finally { busy = ''; }
  }
</script>
<section class="today" aria-label="Today">
  <h1>Today</h1>
  {#if error}<p role="alert">{error}</p>{/if}
  {#if !data && !error}<p>Loading your priorities…</p>{/if}
  {#if data}
    {#each sections as [key, label]}
      {@const rows = data.items.filter(i => i.section === key)}
      <section aria-label={label}>
        <h2>{label} <span>{rows.length}</span></h2>
        {#each rows as item (item.id)}
          <article class:urgent={key === 'urgent'}>
            <h3>{item.title}</h3>
            <p>{item.state.replaceAll('_', ' ')}{#if item.commitment} · due {new Date(item.commitment.due_at).toLocaleString()} · {item.commitment.party}{/if}</p>
            {#each item.waiting_for ?? [] as reason}<p>{reason}</p>{/each}
            {#if item.notice}<p>{item.notice.reason}</p>{/if}
            {#if item.snoozed_until}<p>Reminders deferred until {new Date(item.snoozed_until).toLocaleString()}</p>{/if}
            {#each item.verification ?? [] as check}<p>{check.passed ? '✓' : 'Needs work:'} {check.evidence}</p>{/each}
            <div class="actions">
              {#if item.outbox?.length}<button onclick={() => navigate('review/outbox')}>Review drafts</button>{/if}
              {#if item.task_id || item.questions?.length}<button onclick={() => navigate('tasks')}>Open tasks and questions</button>{/if}
              {#if item.workflow}
                <button disabled={busy === item.id} onclick={() => act(item.id, 'verify')}>Check completion</button>
                {#if key === 'ready'}<button disabled={busy === item.id} onclick={() => act(item.id, 'close')}>Finish workflow</button>{/if}
                <button disabled={busy === item.id} onclick={() => act(item.id, 'snooze', { until: new Date(Date.now() + 86400000).toISOString() })}>Remind me tomorrow</button>
                {#if item.notice}<button disabled={busy === item.id} onclick={() => act(item.id, 'ack')}>Dismiss reminder</button>{/if}
              {/if}
            </div>
          </article>
        {:else}<p class="empty">Nothing here.</p>{/each}
      </section>
    {/each}
  {/if}
</section>
<style>
  h1 { font-size: 24px; margin: 0 0 16px; }
  h2 { font-size: 15px; margin: 20px 0 10px; } h2 span { color: var(--text-muted); margin-left: 8px; }
  h3 { font-size: 15px; margin: 0; font-weight: 500; }
  article { border: 1px solid var(--accent-900); border-radius: 10px; padding: 14px; margin-bottom: 8px; }
  article.urgent { border-left: 3px solid var(--hazard); }
  p { font-size: 13px; color: var(--text-muted); line-height: 1.5; overflow-wrap: anywhere; }
  .empty { margin: 0; } .actions { display: flex; flex-wrap: wrap; gap: 8px; }
  button { color: var(--text); background: transparent; border: 1px solid var(--accent-900); border-radius: 6px; min-height: 44px; padding: 8px 12px; cursor: pointer; }
  button:disabled { opacity: .5; cursor: wait; }
  [role=alert] { color: var(--hazard); }
</style>
