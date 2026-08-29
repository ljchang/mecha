<script>
  import SettingsCharter from './SettingsCharter.svelte';
  import SettingsLearning from './SettingsLearning.svelte';
  import SettingsVoice from './SettingsVoice.svelte';

  // Settings is an index of features, not one long scroll: each row opens a
  // pane at `#settings/<pane>`, so the hash names where you are, reload
  // lands back there, and browser-back is the way out. The panes are
  // separate components for the same reason Review's are — the charter's
  // two-tap save and the voice pane's live microphone have no business
  // sharing a scope.
  let { initial = null, navigate, backTo } = $props();
  const PANES = ['charter', 'learning', 'voice'];
  // Derived, never copied into state: App re-renders this with a new
  // `initial` on back/forward, and a `$state` snapshot would ignore it.
  const pane = $derived(PANES.includes(initial) ? initial : null);

  const TITLE = { charter: 'Charter', learning: 'Learning', voice: 'Voice' };

  // Each pane reads its own data when opened. These are only the one-line
  // summaries the index rows show, re-read on every return so an edit made
  // inside a pane is not still showing its old count out here.
  let charter = $state(null);
  let charterErr = $state(null);
  let rules = $state(null);
  let rulesErr = $state(null);
  let voice = $state(null);

  async function loadSummary() {
    try {
      const res = await fetch('/api/settings/charter');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      charter = await res.json();
      charterErr = charter.error ?? null;
    } catch (e) {
      charterErr = String(e?.message ?? e);
    }
    try {
      const res = await fetch('/api/settings/rules');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      rules = await res.json();
      rulesErr = null;
    } catch (e) {
      rules = null;
      rulesErr = String(e?.message ?? e);
    }
    try {
      const res = await fetch('/api/settings/voice');
      // A failed re-read is unknown, shown as a dash — never as "down", and
      // never as the previous answer left standing as though it were current.
      voice = res.ok ? await res.json() : null;
    } catch {
      voice = null;
    }
  }

  // Only `pane` is read synchronously, so this re-runs on a return to the
  // index and not on its own writes.
  $effect(() => {
    if (pane === null) loadSummary();
  });

  // A summary is `{ text, bad }`. Unknown renders as a dash and never as a
  // zero: "nothing went wrong" and "nothing was read" are opposite findings,
  // and an index row is exactly where that distinction gets lost.
  const charterLine = $derived.by(() => {
    if (charterErr) return { text: `could not be read: ${charterErr}`, bad: true };
    if (charter === null) return { text: '—' };
    if (charter.parse_error)
      return { text: 'does not load — every run is starting with none', bad: true };
    const n = charter.lines?.length ?? 0;
    if (!n) return { text: 'not set — nothing rides in any prompt' };
    return {
      text: `${n} ${n === 1 ? 'priority' : 'priorities'}${charter.over_budget ? ' · over budget' : ''}`,
      bad: !!charter.over_budget,
    };
  });

  const learningLine = $derived.by(() => {
    if (rulesErr) return { text: `could not be read: ${rulesErr}`, bad: true };
    if (rules === null) return { text: '—' };
    const active = rules.filter((r) => r.active).length;
    const retired = rules.filter((r) => r.retired).length;
    if (!active && !retired) return { text: 'no rules yet' };
    return { text: `${active} active${retired ? ` · ${retired} retired` : ''}` };
  });

  const voiceLine = $derived.by(() => {
    if (voice === null) return { text: '—' };
    if (voice.offer_target === null) return { text: 'not wired on this serve' };
    const worker = voice.worker_reachable ? 'worker up' : 'worker unreachable';
    if (voice.cloned === null || voice.cloned === undefined)
      return { text: `${worker} · cloning not configured`, bad: !voice.worker_reachable };
    if (voice.cloned_error) return { text: `${worker} · voices unreadable`, bad: true };
    const n = voice.cloned.length;
    return {
      text: `${worker} · ${n} cloned voice${n === 1 ? '' : 's'}`,
      bad: !voice.worker_reachable,
    };
  });

  const ICON = {
    // Ranked lines, shortening: the charter's order is its rank.
    charter: 'M5 6h14M5 11h10M5 16h6',
    // A bulb: what the loop worked out and now carries in every prompt.
    learning: 'M9.5 18h5M10.5 21h3M12 3a6 6 0 00-3.5 10.9V16h7v-2.1A6 6 0 0012 3z',
    // A waveform.
    voice: 'M4 10v4M8 6.5v11M12 9v6M16 4.5v15M20 10v4',
  };

  const rows = $derived([
    { pane: 'charter', name: 'Charter', line: charterLine },
    { pane: 'learning', name: 'Learning', line: learningLine },
    { pane: 'voice', name: 'Voice', line: voiceLine },
  ]);
</script>

{#if pane === null}
  <header>
    <div class="kicker">Settings</div>
  </header>
  <main>
    <div class="index">
      {#each rows as r}
        <button class="row" onclick={() => navigate(`settings/${r.pane}`)}>
          <svg
            class="glyph"
            viewBox="0 0 24 24"
            width="19"
            height="19"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
            stroke-linejoin="round"><path d={ICON[r.pane]} /></svg
          >
          <span class="rowtext">
            <span class="name">{r.name}</span>
            <span class="status" class:bad={r.line.bad}>{r.line.text}</span>
          </span>
          <svg
            class="chev"
            viewBox="0 0 24 24"
            width="16"
            height="16"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linecap="round"
            stroke-linejoin="round"><path d="M9 6l6 6-6 6" /></svg
          >
        </button>
      {/each}
    </div>
  </main>
{:else}
  <header class="detail">
    <button class="backbtn" onclick={() => backTo('settings')} aria-label="back to settings">
      <svg
        viewBox="0 0 24 24"
        width="18"
        height="18"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"><path d="M15 6l-6 6 6 6" /></svg
      >
    </button>
    <span class="title">{TITLE[pane]}</span>
  </header>
  <main>
    {#if pane === 'charter'}
      <SettingsCharter />
    {:else if pane === 'learning'}
      <SettingsLearning />
    {:else}
      <SettingsVoice />
    {/if}
  </main>
{/if}

<style>
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    /* Room for the shell's gear, which floats in this corner on every view. */
    padding: 22px 56px 12px 20px;
  }
  /* Both header states sit on the same 24px row. */
  header .kicker {
    line-height: 24px;
  }
  /* A 44px tap target that occupies a 24px row, the same trick Chat's
     menu button uses so a touch-sized control does not inflate the header. */
  .backbtn {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 44px;
    min-height: 44px;
    margin: -10px 4px -10px -12px;
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0;
  }
  .backbtn:hover {
    color: var(--accent-400);
  }
  .title {
    font-size: 15px;
    font-weight: 500;
  }
  main {
    flex: 1;
    overflow-y: auto;
    padding: 0 20px 24px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  /* The index: one grouped list, hairline-separated. A row is a title, what
     is actually in there, and a chevron — nothing on this screen is a
     control, so nothing on it should look like one. */
  .index {
    display: flex;
    flex-direction: column;
    background: var(--bg);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    min-height: 62px;
    padding: 12px 14px;
    background: none;
    border: none;
    border-top: 1px solid var(--accent-900);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .row:first-child {
    border-top: none;
  }
  .row:hover {
    background: var(--surface);
  }
  .glyph {
    flex: none;
    color: var(--accent-400);
  }
  .rowtext {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    flex: 1;
  }
  .name {
    font-size: 14px;
  }
  .status {
    font-size: 12px;
    line-height: 1.35;
    color: var(--text-muted);
  }
  .status.bad {
    color: var(--hazard);
  }
  .chev {
    flex: none;
    color: var(--accent-700);
  }
</style>
