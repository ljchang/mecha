<script>
  // The chat's model chip, and the owner's picker behind it
  // (REMOTE-SURFACE-DESIGN §14, step 5). One model serves every surface, so a
  // pick here is a pick for voice, Slack and background work too — the menu
  // says so. Each action is a `mecha model` verb run by `mecha serve` (D4);
  // what is loaded, loading, or waiting is the router's own report, read
  // through `model list --json`, never a guess from a timer.
  import { apiFetch as fetch } from './api.js';
  import { reader, routerOf, unavailable, rows, phase, busy, pollEvery, canHurry, chipLabel, waitingLine, outcomeNote } from './model-chip.js';

  /// `model` is what this chat's agent is bound to — the label until the
  /// router has been read. An incognito chat gets the same picker: every model
  /// it lists is on this machine by construction (`model list` reads only the
  /// routers `follow_loaded` entries name, and `router::follows_here` admits
  /// those only as `kind = "local"` on a loopback address), and incognito's
  /// local-only guarantee is its own provider and per-turn gate, never this
  /// chip (owner's ruling, 2026-09-28, amending INCOGNITO-DESIGN §6.1).
  let { model = '', incognito = false } = $props();

  let data = $state(null);
  let readError = $state(null);
  let open = $state(false);
  let acting = $state(false);
  let actError = $state(null);
  // The server's record of the last switch as it stood just before this
  // page's tap: an outcome that is still that one is not this tap's to report.
  let asked = $state(null);
  let wrapEl = $state(null);

  const router = $derived(routerOf(data));
  const ph = $derived(phase(router));
  const list = $derived(rows(router, incognito));
  const note = $derived(busy(ph) ? null : outcomeNote(data, asked));
  const down = $derived(data ? unavailable(data) : readError);

  // `refresh` for polls, which share a read on the wire; `fresh` for the
  // tap's baseline, which must be a read that starts after it.
  const { refresh, fresh } = reader(async () => {
    try {
      const res = await fetch('/api/model');
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      data = await res.json();
      readError = null;
    } catch (e) {
      readError = `the model list could not be read: ${e?.message ?? e}`;
    }
  });

  // Read once for the label; again whenever the menu is open or something is
  // moving, and not otherwise — every read is a child process on the server.
  // `model` is read so a turn that moved this chat's agent re-reads the router.
  $effect(() => {
    void model;
    refresh();
  });
  // A number, so a read that changes nothing but the data does not restart
  // the timer.
  const every = $derived(pollEvery(ph, open));
  $effect(() => {
    if (!every) return;
    const t = setInterval(refresh, every);
    return () => clearInterval(t);
  });

  async function post(path, body) {
    acting = true;
    actError = null;
    try {
      const res = await fetch(path, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body ?? {}),
      });
      if (!res.ok) {
        actError = (await res.text()).trim() || `HTTP ${res.status}`;
        // Said here already; the recorded outcome would say it twice.
        asked = null;
      }
    } catch (e) {
      actError = `${e?.message ?? e}`;
    } finally {
      acting = false;
      refresh();
    }
  }

  async function pick(row) {
    if (row.current || row.disabled || acting || ph.kind === 'switching') return;
    acting = true;
    // Fresh, so a switch that ended since the last read is not taken for
    // this one's outcome.
    await fresh();
    asked = { before: data?.last_switch?.at ?? null };
    post('/api/model/use', { name: row.name });
  }

  function switchNow() {
    if (!canHurry(ph)) return;
    const row = list.find((r) => r.id === ph.to);
    post('/api/model/use', { name: row?.name ?? ph.to, now: true });
  }

  // `cancel-switch` withdraws every pending switch on every router — one and
  // the same here, where the chip speaks for the one router there is.
  function cancelSwitch() {
    post('/api/model/cancel');
  }

  function toggle() {
    open = !open;
    actError = null;
    if (open) refresh();
  }

  function outside(e) {
    if (open && wrapEl && !wrapEl.contains(e.target)) open = false;
  }
  function keydown(e) {
    if (open && e.key === 'Escape') open = false;
  }
</script>

<svelte:window onpointerdown={outside} onkeydown={keydown} />

<span class="wrap" bind:this={wrapEl}>
  <button
    class="chip pick"
    class:moving={busy(ph)}
    class:bad={!!note && !open}
    onclick={toggle}
    aria-haspopup="menu"
    aria-expanded={open}
    title={note && !open ? note.text : 'the model answering every surface — tap to switch'}
  >
    {chipLabel(ph, model)}
    <svg viewBox="0 0 10 6" width="8" height="5" aria-hidden="true"><path d="M1 1l4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg>
  </button>
  {#if open}
    <div class="menu" role="menu" aria-label="switch model">
      {#if down}
        <p class="line bad">{down}</p>
      {:else if !router}
        <p class="line">reading…</p>
      {:else}
        {#if !router.readable}
          <p class="line bad">the router's list is one this build cannot fully read — which model is loaded is unknown</p>
        {/if}
        {#each list as row (row.id)}
          <button
            class="row"
            class:current={row.current}
            role="menuitemradio"
            aria-checked={row.current}
            disabled={row.disabled || acting || ph.kind === 'switching'}
            title={row.why ?? (row.name !== row.id ? `[providers.${row.name}]` : undefined)}
            onclick={() => pick(row)}
          >
            <span class="dot" aria-hidden="true">{row.current ? '●' : ''}</span>
            <span class="id">{row.id}</span>
            {#if ph.kind === 'switching' && ph.to === row.id}
              <span class="word moving">{ph.loading ? 'loading' : 'next'}</span>
            {:else if row.disabled}
              <span class="word bad">{row.why?.startsWith('an incognito chat') ? 'not for incognito' : 'runs would not follow'}</span>
            {:else if row.word}
              <span class="word">{row.word}</span>
            {/if}
          </button>
        {/each}
        {#if ph.kind === 'switching'}
          <div class="pending">
            <p class="line">{waitingLine(ph)}</p>
            {#if ph.stuck}
              <!-- No switcher to hurry: cancel is the only way out. -->
              <div class="acts">
                <button class="act quiet" onclick={cancelSwitch} disabled={acting} title="withdraw the unreadable switch; the loaded model stays">cancel</button>
              </div>
            {:else if canHurry(ph)}
              <div class="acts">
                <!-- R2's way out of D13's wait: the runs it names stop at
                     their next safe point, and a reply in progress ends
                     early. -->
                <button class="act" onclick={switchNow} disabled={acting} title="stop the runs it waits for, then switch">switch now</button>
                <button class="act quiet" onclick={cancelSwitch} disabled={acting} title="withdraw the switch; the loaded model stays">cancel</button>
              </div>
            {/if}
          </div>
        {/if}
        <p class="line hint">One model answers every surface — chat, voice, Slack and background work. A switch waits for runs in progress.</p>
        {#if incognito}
          <p class="line">Every model here runs on this machine, so this chat stays local whichever you pick.</p>
        {/if}
      {/if}
      {#if actError}<p class="line bad">{actError}</p>{/if}
      {#if note}<p class="line" class:bad={note.tone === 'bad'} class:warn={note.tone === 'warn'}>{note.text}</p>{/if}
    </div>
  {/if}
  <!-- A failure the owner did not stay to watch still says so once the chip
       is at rest — R1 loaded the old model back, or the load timed out: the
       chip's outline and its title above, and here for a screen reader. -->
  {#if note && !open}<span class="sr-only" role="status">{note.text}</span>{/if}
</span>

<style>
  .wrap {
    position: relative;
    display: inline-flex;
  }
  .pick {
    cursor: pointer;
    background: var(--bg);
    min-height: 28px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 62vw;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pick:hover,
  .pick[aria-expanded='true'] {
    color: var(--text);
    border-color: var(--accent-700);
  }
  .pick.moving {
    color: var(--accent-300);
    border-color: var(--accent-500);
  }
  .pick.bad {
    color: var(--hazard);
    border-color: var(--hazard);
  }
  .pick svg {
    flex-shrink: 0;
    opacity: 0.7;
  }
  .menu {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 40;
    width: 320px;
    max-width: calc(100vw - 32px);
    box-sizing: border-box;
    padding: 6px;
    background: var(--surface);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.45);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 38px;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--radius-chip);
    background: none;
    color: var(--text);
    font: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }
  .row:hover:not(:disabled) {
    background: var(--accent-900);
  }
  .row:disabled {
    cursor: default;
    color: var(--text-muted);
  }
  .row.current {
    color: var(--accent-100);
  }
  .dot {
    width: 10px;
    flex-shrink: 0;
    font-size: 9px;
    color: var(--accent-400);
  }
  .id {
    flex: 1;
    min-width: 0;
    font-family: var(--mono);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .word {
    flex-shrink: 0;
    font-family: var(--mono);
    font-size: 10px;
    color: var(--text-muted);
  }
  .word.moving {
    color: var(--accent-300);
  }
  .word.bad,
  .line.bad {
    color: var(--hazard);
  }
  .line.warn {
    color: var(--hazard);
  }
  .line {
    margin: 0;
    padding: 6px 8px;
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-muted);
    white-space: pre-wrap;
  }
  .line.hint {
    border-top: 1px solid var(--accent-900);
    margin-top: 4px;
    padding-top: 8px;
    font-size: 11px;
  }
  .pending {
    border-top: 1px solid var(--accent-900);
    margin-top: 4px;
    padding-top: 2px;
  }
  .acts {
    display: flex;
    gap: 8px;
    padding: 2px 8px 6px;
  }
  .act {
    min-height: 32px;
    padding: 0 12px;
    border: 1px solid var(--hazard);
    border-radius: var(--radius-chip);
    background: var(--bg);
    color: var(--hazard);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .act.quiet {
    border-color: var(--accent-900);
    color: var(--text-muted);
  }
  .act:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
</style>
