<script>
  // The settings page: the charter (view + a validated edit), the learned
  // rules (a read), and the voice stack's health (a read). The one write on
  // this whole page is the charter save, and the server refuses any save
  // the runs' own reader would not load — see serve/settings.rs for the
  // boundary and for what is deliberately not editable from a browser.
  let charter = $state(null);
  let charterError = $state(null);
  let rules = $state(null);
  let rulesError = $state(null);
  let voice = $state(null);

  // The editor: null when closed, else the text being edited. `confirming`
  // is the two-tap save — the charter rides in every run's prompt, so one
  // stray tap must not rewrite it.
  let draft = $state(null);
  let confirming = $state(false);
  let saveError = $state(null);
  let savedNote = $state(null);

  async function load() {
    try {
      const res = await fetch('/api/settings/charter');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      charter = await res.json();
      charterError = charter.error ?? null;
    } catch (e) {
      charterError = String(e?.message ?? e);
    }
    try {
      const res = await fetch('/api/settings/rules');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      rules = await res.json();
      rulesError = null;
    } catch (e) {
      rulesError = String(e?.message ?? e);
    }
    try {
      const res = await fetch('/api/settings/voice');
      if (res.ok) voice = await res.json();
    } catch {
      voice = null; // unknown, shown as a dash — never as "down"
    }
  }
  load();

  function openEditor() {
    draft = charter?.raw ?? '';
    confirming = false;
    saveError = null;
    savedNote = null;
  }

  async function save() {
    if (!confirming) {
      confirming = true;
      return;
    }
    confirming = false;
    try {
      const res = await fetch('/api/settings/charter', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ raw: draft }),
      });
      if (!res.ok) {
        // 422 carries the parse error — the draft stays open so nothing
        // typed is lost, which is the whole point of refusing server-side.
        saveError = (await res.text()).trim();
        return;
      }
      charter = await res.json();
      charterError = charter.error ?? null;
      draft = null;
      saveError = null;
      savedNote =
        'saved — rides in the prompt of new sessions; this page cannot rebuild ones already running';
    } catch (e) {
      saveError = String(e?.message ?? e);
    }
  }

  const activeRules = $derived((rules ?? []).filter((r) => r.active));
  const retiredRules = $derived((rules ?? []).filter((r) => r.retired));
  const dash = (v) => (v === null || v === undefined ? '—' : v);
</script>

<header>
  <div class="kicker">Settings</div>
</header>

<main>
  <section>
    <div class="kicker">Charter</div>
    <div class="hint">
      Standing priorities every run carries, ranked highest first — order is rank: when two
      conflict, the higher line wins outright.
    </div>

    {#if charterError}
      <div class="card notice">{charterError}</div>
    {/if}

    {#if charter?.parse_error && draft === null}
      <div class="card notice">
        The charter on disk does not load — every run is starting with none: {charter.parse_error}
      </div>
    {/if}

    {#if draft !== null}
      <textarea class="editor" bind:value={draft} spellcheck="false" rows="16"></textarea>
      {#if saveError}
        <div class="card notice">not saved: {saveError}</div>
      {/if}
      <div class="row-actions">
        <button class="btn primary" class:confirm={confirming} onclick={save}>
          {confirming ? 'This rides in every run’s prompt — confirm save' : 'Save'}
        </button>
        <button
          class="btn"
          onclick={() => {
            draft = null;
            confirming = false;
            saveError = null;
          }}>Cancel</button
        >
      </div>
    {:else}
      {#if charter?.lines?.length}
        <ol class="charter">
          {#each charter.lines as line}
            <li>
              <span class="id">{line.id}</span>
              <span class="text">{line.text}</span>
            </li>
          {/each}
        </ol>
        {#if charter.over_budget}
          <div class="card notice">
            {charter.char_count} characters — over the {charter.budget} budget. It still rides in
            full, but costs more of the cached prefix than argued.
          </div>
        {/if}
      {:else if charter && !charter.parse_error}
        <div class="card">
          <div class="sub">
            No charter yet — nothing rides in any prompt. Edit to write one; the format is
            explained by example once you start.
          </div>
        </div>
      {/if}
      {#if savedNote}
        <div class="card ok-note">{savedNote}</div>
      {/if}
      {#if charter}
        <div class="row-actions">
          <button class="btn" onclick={openEditor}>Edit</button>
        </div>
      {/if}
    {/if}
  </section>

  <section>
    <div class="kicker">Learned rules</div>
    <div class="hint">
      What rides in prompts from the learning loop. A read: retiring goes through its own staged
      review, never a tap here.
    </div>
    {#if rulesError}
      <div class="card notice">could not read the rules: {rulesError}</div>
    {:else if rules === null}
      <div class="card"><div class="sub">loading…</div></div>
    {:else if rules.length === 0}
      <div class="card"><div class="sub">No rules yet — `mecha learn` creates them.</div></div>
    {:else}
      <div class="rules">
        {#each activeRules as r}
          <div class="rule">
            <div class="rule-head">
              <span class="chip domain">{r.domain}</span>
              {#if r.user}<span class="chip">yours</span>{/if}
              <span class="tally"
                >{dash(r.observations)} obs · {dash(r.attributed_regressions)} regressions</span
              >
            </div>
            <div class="rule-text">{r.title}</div>
          </div>
        {/each}
        {#if retiredRules.length}
          <div class="sub retired-count">
            {retiredRules.length} retired rule{retiredRules.length === 1 ? '' : 's'} kept as
            evidence (mecha rules restore un-retires)
          </div>
        {/if}
      </div>
    {/if}
  </section>

  <section>
    <div class="kicker">Voice</div>
    {#if voice === null}
      <div class="card"><div class="sub">—</div></div>
    {:else if voice.offer_target === null}
      <div class="card"><div class="sub">Voice is not wired on this serve.</div></div>
    {:else}
      <div class="card">
        <div class="row">
          <span class="label">worker</span>
          <span class="count" style:color={voice.worker_reachable ? 'var(--accent-400)' : 'var(--hazard)'}>
            {voice.worker_reachable ? 'up' : 'unreachable'}
          </span>
        </div>
        <div class="sub">{voice.offer_target} — configured where it runs, shown here</div>
      </div>
    {/if}
  </section>
</main>

<style>
  header {
    padding: 18px 16px 6px;
  }
  main {
    flex: 1;
    overflow-y: auto;
    padding: 0 16px 24px;
    display: flex;
    flex-direction: column;
    gap: 22px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .hint {
    color: var(--text-muted);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .notice {
    color: var(--hazard);
    font-size: 12.5px;
    white-space: pre-wrap;
    font-family: var(--mono);
  }
  .ok-note {
    color: var(--accent-300);
    font-size: 12.5px;
  }
  .charter {
    margin: 0;
    padding: 0 0 0 22px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .charter li::marker {
    color: var(--accent-500);
    font-family: var(--mono);
    font-size: 12px;
  }
  .charter .id {
    display: block;
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--accent-400);
  }
  .charter .text {
    font-size: 13.5px;
    line-height: 1.45;
  }
  .editor {
    width: 100%;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius);
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.5;
    padding: 10px;
    resize: vertical;
  }
  .row-actions {
    display: flex;
    gap: 8px;
  }
  .btn {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    padding: 7px 14px;
    font: inherit;
    font-size: 13px;
    cursor: pointer;
  }
  .btn.primary {
    border-color: var(--accent-400);
    color: var(--accent-300);
  }
  .btn.primary.confirm {
    background: var(--accent-900);
  }
  .rules {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .rule {
    background: var(--surface);
    border-radius: var(--radius);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .rule-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .chip {
    font-family: var(--mono);
    font-size: 10px;
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    padding: 1px 6px;
    color: var(--text-muted);
  }
  .chip.domain {
    color: var(--accent-400);
  }
  .tally {
    margin-left: auto;
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--text-muted);
  }
  .rule-text {
    font-size: 13px;
    line-height: 1.4;
  }
  .retired-count {
    color: var(--text-muted);
    font-size: 12px;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  .label {
    font-size: 13px;
  }
  .count {
    font-family: var(--mono);
    font-size: 13px;
  }
  .sub {
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.4;
  }
</style>
