<script>
  // The learning pane: what mecha has been taught, at the two stages the
  // owner can act on. A *reflection* is one lesson mined from one of the
  // owner's own interventions; a *rule* is what several consolidate into,
  // and it rides in every future prompt's cached prefix.
  //
  // **Editing is offered at the first stage on purpose.** A rule is a
  // consolidation, so objecting once one exists costs the good lessons
  // merged into it; at the reflection it costs nothing and says exactly what
  // was wrong. It is also a *provenance promotion* rather than a text
  // change — a lesson the owner typed is the owner's, so the gate stops
  // excluding it — which is why an unchanged save is refused rather than
  // accepted, and why the editor keeps the original beside the draft.
  //
  // The third stage, a rule proposal, is decided in the queue: accepting one
  // applies a whole rewritten set, which is not a decision to hand a thumb
  // on a phone.
  //
  // Every mutation is a `mecha reflections …` / `mecha rules …` child
  // process (see serve/settings.rs), so this pane cannot do anything to the
  // store that the command line cannot, and the promotion, the withholding
  // and the git commit behind each write stay in one implementation.
  let pane = $state('reflections');
  let rules = $state(null);
  let rulesError = $state(null);
  let reflections = $state(null);
  let reflectionsError = $state(null);

  // The open lesson editor: { id, text, original }. The original rides along
  // because an unchanged save must not be offered — `edit_reflexion` refuses
  // it outright, and that is the guarantee; this is manners.
  let lessonDraft = $state(null);
  // The first tap of a two-tap refusal: { verb, id }. Drop and retire are
  // both flags rather than deletions, but both change what every future run
  // carries, so neither happens on one tap.
  let armed = $state(null);
  let armedReason = $state('');
  let busy = $state(false);
  let note = $state(null);
  let error = $state(null);
  // One reflection in full: { id, record }. What was happening and what was
  // said are the evidence a refusal rests on, and the listing carries
  // neither.
  let detail = $state(null);

  async function loadRules() {
    try {
      const res = await fetch('/api/settings/rules');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      rules = await res.json();
      rulesError = null;
    } catch (e) {
      rulesError = String(e?.message ?? e);
    }
  }

  // `--all` server-side: `reflections list` hides dropped rows by default,
  // and a page that hid them would make restore unreachable — dropping is
  // what removes a row from the default listing.
  async function loadReflections() {
    try {
      const res = await fetch('/api/settings/reflections');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      reflections = await res.json();
      reflectionsError = null;
    } catch (e) {
      reflectionsError = String(e?.message ?? e);
    }
  }

  loadRules();
  loadReflections();

  function setPane(next) {
    pane = next;
    // A different list, and the next tap may be a refusal — nothing armed,
    // half-edited or expanded should survive the move.
    lessonDraft = null;
    armed = null;
    detail = null;
    note = null;
    error = null;
  }

  // One verb, and its own report of what it did. The note is the child's
  // stdout when it said something — `mecha reflections edit` prints the
  // provenance move it just performed, which is the one thing about an edit
  // that is not obvious from the result.
  async function act(path, body, fallback) {
    busy = true;
    error = null;
    note = null;
    try {
      const res = await fetch(path, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!res.ok) {
        // The CLI's refusal is the API's, arriving as its own last line.
        error = (await res.text()).trim();
        return false;
      }
      const out = await res.json();
      note = (out?.output || '').trim() || fallback;
      return true;
    } catch (e) {
      error = String(e?.message ?? e);
      return false;
    } finally {
      busy = false;
    }
  }

  function openLesson(r) {
    armed = null;
    detail = null;
    note = null;
    error = null;
    lessonDraft = { id: r.id, text: r.title, original: r.title };
  }

  const lessonChanged = $derived(
    !!lessonDraft?.text.trim() && lessonDraft.text.trim() !== lessonDraft.original.trim()
  );

  async function saveLesson() {
    if (!lessonChanged) return;
    const { id, text } = lessonDraft;
    if (await act('/api/settings/reflections/edit', { id, text }, 'edited')) {
      lessonDraft = null;
      await loadReflections();
    }
  }

  // Arm, then act. The reason rides with the second tap: it is recorded on
  // the record, and for a retirement the learner is shown it, so the same
  // lesson does not come back under new wording.
  function arm(verb, id) {
    lessonDraft = null;
    note = null;
    error = null;
    armed = { verb, id };
    armedReason = '';
  }

  async function confirmArmed() {
    if (!armed) return;
    const { verb, id } = armed;
    const reason = armedReason.trim() || null;
    const path =
      verb === 'drop' ? '/api/settings/reflections/drop' : '/api/settings/rules/retire';
    armed = null;
    // Spelled out rather than derived: `${verb}ped` reads "retireped". Both
    // children print on success today so the fallback never fires, which is
    // exactly the kind of unreachable string that surfaces the day one of
    // them goes quiet.
    const fallback = verb === 'drop' ? 'dropped' : 'retired';
    if (await act(path, { id, reason }, fallback)) {
      await (verb === 'drop' ? loadReflections() : loadRules());
    }
  }

  async function restoreReflection(id) {
    armed = null;
    if (await act('/api/settings/reflections/restore', { id }, 'restored')) {
      await loadReflections();
    }
  }

  async function restoreRule(id) {
    armed = null;
    if (await act('/api/settings/rules/restore', { id }, 'restored')) {
      await loadRules();
    }
  }

  async function readDetail(id) {
    if (detail?.id === id) {
      detail = null;
      return;
    }
    detail = null;
    error = null;
    try {
      const res = await fetch(`/api/settings/reflections/show?id=${encodeURIComponent(id)}`);
      if (!res.ok) {
        error = (await res.text()).trim();
        return;
      }
      detail = { id, record: await res.json() };
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  // What the ledger says about one rule. Absent is not zero: a rule no probe
  // has ever reached is not a rule that passed, and rendering both as
  // "0 regressions" would read as a clean bill of health for a rule nothing
  // has ever measured.
  function tally(r) {
    if (r.user) return 'yours — never tallied, never retired';
    if (r.observations === null || r.observations === undefined || r.observations === 0) {
      return 'never validated — no probe has reached it';
    }
    return r.attributed_regressions
      ? `${r.observations} probe(s), ${r.attributed_regressions} attributed regression(s)`
      : `${r.observations} probe(s), none attributed`;
  }

  // A learned rule with an id is the only thing retire/restore can resolve:
  // a user rule is not on trial, and a rule minted before ids existed has
  // nothing to name it with. Offering the button anyway would send an empty
  // needle, which prefix-matches every rule in the store.
  const actionable = (r) => !r.user && !!r.id;

  const day = (s) => (s ? String(s).slice(0, 10) : '');
</script>

<p class="hint">
  What mecha has been taught, at the two stages you can act on. A <em>reflection</em> is one
  lesson mined from one of your interventions; a <em>rule</em> is what several of them
  consolidate into, and it rides in every run's prompt. Disagree at the lesson where you can —
  objecting to a rule costs the other lessons merged into it. Rule proposals are decided in the
  queue.
</p>

<div class="tabs">
  <button class="tab" class:on={pane === 'reflections'} onclick={() => setPane('reflections')}>
    reflections{#if reflections}<span class="n">{reflections.length}</span>{/if}
  </button>
  <button class="tab" class:on={pane === 'rules'} onclick={() => setPane('rules')}>
    rules{#if rules}<span class="n">{rules.length}</span>{/if}
  </button>
</div>

{#if error}
  <div class="card notice">{error}</div>
{/if}
{#if note}
  <div class="card ok-note">{note}</div>
{/if}

{#if pane === 'reflections'}
  {#if reflectionsError}
    <div class="card notice">could not read the reflections: {reflectionsError}</div>
  {:else if reflections === null}
    <div class="card"><div class="sub">loading…</div></div>
  {:else if reflections.length === 0}
    <div class="card">
      <div class="sub">
        No reflections yet — <code>mecha reflect</code> mines them from interventions.
      </div>
    </div>
  {:else}
    {#each reflections as r (r.id)}
      <div class="rule" class:spent={r.dropped}>
        <div class="rule-head">
          <span class="chip domain">{r.domain}</span>
          <span class="chip">{r.trigger}</span>
          {#if r.edited}<span class="chip mine">yours</span>{/if}
          {#if r.dropped}<span class="chip gone">dropped</span>{/if}
          <span class="tally">{day(r.created_at)}</span>
        </div>

        {#if lessonDraft?.id === r.id}
          <textarea class="editor" bind:value={lessonDraft.text} rows="4"></textarea>
          <div class="sub">
            A lesson you write is yours: the provenance gate stops excluding it, so it can become
            a rule — and what was happening is withheld on the way through, because that is the
            field any third-party text was in. Saving it unchanged is refused, since the promotion
            rests on the words being yours.
          </div>
          <div class="row-actions">
            <button class="btn small primary" disabled={busy || !lessonChanged} onclick={saveLesson}>
              {busy ? 'saving…' : 'Save lesson'}
            </button>
            <button class="btn small" onclick={() => (lessonDraft = null)}>Cancel</button>
          </div>
        {:else}
          <div class="rule-text">{r.title}</div>
          {#if r.blocked}
            <div class="sub blocked">{r.blocked}</div>
          {/if}

          {#if detail?.id === r.id}
            <div class="detail">
              <div class="dlabel">while</div>
              <div class="dtext">{detail.record.context}</div>
              <div class="dlabel">the intervention</div>
              <div class="dtext">{detail.record.intervention}</div>
              <!-- The gate's verdict, never the stored field: for a record
                   written before the harness-voice check existed the two
                   disagree, and this sits under a row already reporting the
                   computed answer. -->
              <div class="sub">
                provenance {detail.record.provenance} · evidence {detail.record.evidence} · session
                {detail.record.session_id}
              </div>
            </div>
          {/if}

          {#if armed?.verb === 'drop' && armed.id === r.id}
            <input
              class="reason"
              placeholder="why — recorded for the next reader (optional)"
              bind:value={armedReason}
              maxlength="200"
            />
          {/if}

          <div class="row-actions">
            <button class="btn small" onclick={() => openLesson(r)}>Edit lesson</button>
            {#if r.dropped}
              <button class="btn small" disabled={busy} onclick={() => restoreReflection(r.id)}>
                Restore
              </button>
            {:else if armed?.verb === 'drop' && armed.id === r.id}
              <button class="btn small danger" disabled={busy} onclick={confirmArmed}>
                Confirm drop
              </button>
              <button class="btn small" onclick={() => (armed = null)}>Cancel</button>
            {:else}
              <button class="btn small" onclick={() => arm('drop', r.id)}>Drop</button>
            {/if}
            <button class="btn small" onclick={() => readDetail(r.id)}>
              {detail?.id === r.id ? 'Hide' : 'Read'}
            </button>
          </div>
        {/if}
      </div>
    {/each}
    <div class="sub retired-count">
      A drop is a flag, never a deletion — the record stays as evidence that this lesson was
      considered and refused, so the same one cannot come back next pass unjudged.
    </div>
  {/if}
{:else if rulesError}
  <div class="card notice">could not read the rules: {rulesError}</div>
{:else if rules === null}
  <div class="card"><div class="sub">loading…</div></div>
{:else if rules.length === 0}
  <div class="card"><div class="sub">No rules yet — <code>mecha learn</code> creates them.</div></div>
{:else}
  <!-- `active` is `enabled && not retired`, so a rule hand-disabled in the
       learned-rules TOML is not retired and still rides in no prompt. On a
       pane whose job is "what a run actually carries", that reads as spent
       too. -->
  <!-- Keyed on domain+text where there is no id — every user rule, and any
       learned rule minted before ids existed. `rules list --json` iterates
       the domains, so the same text in two of them would otherwise be two
       rows with one key, which Svelte 5 raises on. -->
  {#each rules as r (r.id ?? `${r.domain}:${r.title}`)}
    <div class="rule" class:spent={r.retired || !r.active}>
      <div class="rule-head">
        <span class="chip domain">{r.domain}</span>
        {#if r.user}<span class="chip mine">yours</span>{/if}
        {#if r.retired}<span class="chip gone">retired</span>{/if}
        {#if !r.retired && !r.active}<span class="chip gone">disabled</span>{/if}
        <span class="tally">{tally(r)}</span>
      </div>
      <div class="rule-text">{r.title}</div>
      {#if r.retired && r.retired_reason}
        <div class="sub blocked">retired — {r.retired_reason}</div>
      {:else if !r.active && !r.user}
        <!-- Learned rules only: a user rule can carry `enabled = false` in
             its own file too, and it has no Retire button to honour the
             advice with. The chip above says "disabled" either way. -->
        <div class="sub blocked">
          disabled by hand in the rules file — it rides in no prompt, and retiring is the
          reversible way to say so
        </div>
      {/if}

      {#if armed?.verb === 'retire' && armed.id === r.id}
        <input
          class="reason"
          placeholder="why — shown to the learner so it does not come back reworded"
          bind:value={armedReason}
          maxlength="200"
        />
      {/if}

      {#if actionable(r)}
        <div class="row-actions">
          {#if r.retired}
            <button class="btn small" disabled={busy} onclick={() => restoreRule(r.id)}>
              Restore
            </button>
          {:else if armed?.verb === 'retire' && armed.id === r.id}
            <button class="btn small danger" disabled={busy} onclick={confirmArmed}>
              Confirm retire
            </button>
            <button class="btn small" onclick={() => (armed = null)}>Cancel</button>
          {:else}
            <button class="btn small" onclick={() => arm('retire', r.id)}>Retire</button>
          {/if}
        </div>
      {/if}
    </div>
  {/each}
  <div class="sub retired-count">
    Retiring is a flag, never a deletion: the rule stays in the file as evidence and the learner
    is told it was measured harmful, so restore can undo what erasure could not.
  </div>
{/if}

<style>
  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .tabs {
    display: flex;
    gap: 6px;
  }
  .tab {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-muted);
    font: inherit;
    font-family: var(--mono);
    font-size: 11.5px;
    padding: 4px 2px 5px;
    margin-right: 10px;
    min-height: 32px;
    cursor: pointer;
  }
  .tab.on {
    color: var(--accent-300);
    border-bottom-color: var(--accent-400);
  }
  .tab .n {
    margin-left: 6px;
    color: var(--text-muted);
  }
  .card {
    padding: 10px 12px;
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
    white-space: pre-wrap;
  }
  .rule {
    background: var(--surface);
    border-radius: var(--radius);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  /* Dropped and retired records stay on the page — a refusal has to be
     visible to be undone — but read as past. */
  .rule.spent .rule-text {
    color: var(--text-muted);
    text-decoration: line-through;
    text-decoration-color: var(--accent-700);
  }
  .rule-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .chip.domain {
    color: var(--accent-400);
  }
  /* The owner's own mark — an edited lesson, a user rule — is the thing most
     worth telling apart at a glance: what mecha decided, and what you did
     about it. */
  .chip.mine {
    color: var(--accent-300);
    border-color: var(--accent-400);
  }
  .chip.gone {
    color: var(--hazard);
    border-color: var(--hazard);
  }
  .tally {
    margin-left: auto;
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .rule-text {
    font-size: 13px;
    line-height: 1.4;
    white-space: pre-wrap;
  }
  /* Why this one is excluded, retired or refused — the sentence a decision
     rests on, so never the same colour as the record itself. */
  .sub.blocked {
    color: var(--accent-400);
  }
  .detail {
    border-top: 1px solid var(--accent-900);
    padding-top: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .dlabel {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--text-muted);
  }
  .dtext {
    font-size: 12.5px;
    line-height: 1.45;
    white-space: pre-wrap;
    padding-bottom: 4px;
  }
  .editor {
    width: 100%;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius);
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.5;
    padding: 10px;
    resize: vertical;
  }
  .reason {
    width: 100%;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    font: inherit;
    font-size: 13px;
    padding: 6px 8px;
    min-width: 0;
  }
  .row-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .btn {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    padding: 7px 14px;
    font: inherit;
    font-size: 13px;
    min-height: 40px;
    cursor: pointer;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .btn.small {
    min-height: 32px;
    padding: 4px 10px;
    font-size: 12px;
  }
  .btn.primary {
    border-color: var(--accent-400);
    color: var(--accent-300);
  }
  .btn.danger {
    border-color: var(--hazard);
    color: var(--hazard);
  }
  .retired-count {
    font-size: 12px;
  }
  code {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--accent-300);
  }
  .sub {
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.4;
  }
</style>
