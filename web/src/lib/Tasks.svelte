<script>
  // The GTD board, over `mecha tasks …` — which reaches the graph's own
  // store through its MCP surface. Nothing here confirms: every status is
  // one tap from where it was, and the tool surface has no delete.
  let data = $state(null);
  let error = $state(null);
  let filter = $state('actionable');
  let selected = $state(null);
  let adding = $state(false);
  let addName = $state('');
  let addDue = $state('');
  let addContext = $state('');
  let busy = $state(false);

  async function load() {
    try {
      const res = await fetch('/api/tasks');
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }
  load();

  const ACTIONABLE = ['next', 'inbox'];
  const filters = [
    ['actionable', (t) => ACTIONABLE.includes(t.status)],
    ['scheduled', (t) => t.status === 'scheduled'],
    ['waiting', (t) => t.status === 'waiting'],
    ['done', (t) => t.status === 'done' || t.status === 'dropped'],
  ];
  const tasks = $derived.by(() => {
    const pred = filters.find(([name]) => name === filter)?.[1] ?? (() => true);
    return (data?.items ?? []).filter(pred);
  });
  const count = (name) => {
    const pred = filters.find(([n]) => n === name)?.[1];
    return (data?.items ?? []).filter(pred).length;
  };

  async function setStatus(task, status) {
    busy = true;
    try {
      const res = await fetch('/api/tasks/set', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ task, status }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      selected = null;
      await load();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function add() {
    if (!addName.trim()) return;
    busy = true;
    try {
      const res = await fetch('/api/tasks/add', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          name: addName.trim(),
          due: addDue.trim() || null,
          context: addContext.trim() || null,
        }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      adding = false;
      addName = addDue = addContext = '';
      await load();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  const dueLabel = (t) => {
    if (!t.due_at) return null;
    const date = t.due_at.slice(0, 10);
    const today = new Date().toISOString().slice(0, 10);
    if (t.overdue) return { text: 'overdue', hazard: true };
    if (date === today) return { text: 'due today', hazard: true };
    return { text: `due ${date.slice(5)}`, hazard: false };
  };

  const STATUSES = ['next', 'inbox', 'scheduled', 'waiting', 'done', 'dropped'];
</script>

{#snippet hazardGlyph(size = 12)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

<div class="page">
  <header>
    <span class="title">Tasks</span>
    <span class="chip">graph board</span>
  </header>

  <div class="filters">
    {#each filters as [name]}
      <button class="chipbtn" class:active={filter === name} onclick={() => (filter = name)}>
        {name}{count(name) ? ` ${count(name)}` : ''}
      </button>
    {/each}
  </div>

  <div class="scroll">
    {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}
    {#if data === null && !error}
      <div class="empty">reaching the graph…</div>
    {/if}
    {#each tasks as t}
      <button class="card row" onclick={() => (selected = selected === t.id ? null : t.id)}>
        <div class="name">{t.name}</div>
        <div class="meta">
          <span class="chip">{t.status}</span>
          {#if dueLabel(t)}
            {@const due = dueLabel(t)}
            <span class="due" class:hazard={due.hazard}>
              {#if due.hazard}{@render hazardGlyph(11)}{/if}
              {due.text}
            </span>
          {/if}
          {#if t.context}<span class="chip">{t.context}</span>{/if}
          {#if t.project}<span class="chip dim">{t.project}</span>{/if}
        </div>
        {#if selected === t.id}
          <div class="statusrow">
            {#each STATUSES.filter((s) => s !== t.status) as status}
              <button
                class="statusbtn"
                disabled={busy}
                onclick={(e) => {
                  e.stopPropagation();
                  setStatus(t.id, status);
                }}
              >{status}</button>
            {/each}
          </div>
        {/if}
      </button>
    {:else}
      {#if data}<div class="empty">Nothing here.</div>{/if}
    {/each}
    <div class="footnote">Every change is one tap and reversible — nothing here confirms.</div>
  </div>

  <button class="fab" onclick={() => (adding = true)} title="capture a task">
    <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="var(--void)" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14" /></svg>
  </button>

  {#if adding}
    <div class="scrim" onclick={() => (adding = false)} aria-hidden="true"></div>
    <div class="sheet">
      <div class="grip"></div>
      <div class="sheettitle">Capture — lands in inbox</div>
      <input class="field" placeholder="The task, phrased as an action" bind:value={addName} />
      <div class="fieldrow">
        <input class="field" placeholder="due: today, +3d, 2026-09-05" bind:value={addDue} />
        <input class="field" placeholder="@context" bind:value={addContext} />
      </div>
      <div class="btnrow">
        <button class="btn" onclick={() => (adding = false)}>Cancel</button>
        <button class="btn primary" disabled={busy || !addName.trim()} onclick={add}>Capture</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  header { display: flex; align-items: center; justify-content: space-between; padding: 22px 20px 12px; }
  .title { font-weight: 500; font-size: 17px; letter-spacing: -0.02em; }
  .filters { display: flex; gap: 6px; padding: 0 20px 12px; overflow-x: auto; }
  .chipbtn { font-family: var(--mono); font-size: 12px; color: var(--text-muted); background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); padding: 8px 12px; min-height: 40px; cursor: pointer; white-space: nowrap; }
  .chipbtn.active { color: var(--text); background: var(--accent-900); border-color: var(--accent-700); }
  .scroll { flex: 1; overflow-y: auto; padding: 2px 20px 90px; display: flex; flex-direction: column; gap: 10px; }
  .row { text-align: left; padding: 14px; display: flex; flex-direction: column; gap: 8px; cursor: pointer; color: var(--text); font: inherit; }
  .name { font-size: 14px; font-weight: 500; line-height: 1.4; }
  .meta { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .due { display: flex; align-items: center; gap: 5px; font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .due.hazard { color: var(--hazard); }
  .dim { color: var(--accent-700); }
  .statusrow { display: flex; gap: 6px; flex-wrap: wrap; border-top: 1px solid var(--accent-900); padding-top: 10px; }
  .statusbtn { font-family: var(--mono); font-size: 11px; color: var(--text); background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 9px 12px; min-height: 40px; cursor: pointer; }
  .warnline { display: flex; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .empty { color: var(--text-muted); font-size: 14px; padding: 20px 0; text-align: center; }
  .footnote { font-size: 11px; color: var(--text-muted); text-align: center; padding-top: 6px; }
  .fab { position: absolute; right: 20px; bottom: 20px; width: 56px; height: 56px; border-radius: 14px; background: var(--accent-400); border: none; display: flex; align-items: center; justify-content: center; cursor: pointer; }
  .scrim { position: absolute; inset: 0; z-index: 5; }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px 20px 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheettitle { font-size: 15px; font-weight: 500; }
  .field { background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; padding: 12px 14px; min-height: 44px; box-sizing: border-box; width: 100%; }
  .fieldrow { display: flex; gap: 8px; }
  .btnrow { display: flex; gap: 10px; }
  .btn { flex: 1; min-height: 48px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; }
  .btn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .btn:disabled { opacity: 0.5; }
</style>
