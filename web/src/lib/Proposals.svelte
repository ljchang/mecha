<script>
  // The proposal stores on the phone: harness candidates, rule proposals and
  // the graph's entity proposals. One pane over three stores, because they
  // answer the same shape and take the same verbs — the same argument the
  // TUI's `/queues` review level is built on, and the reason `mecha harness
  // list` used to be printed on a card nobody could open.
  //
  // Read-then-decide: the buttons live in the detail view and nowhere else.
  // A harness candidate carries a prediction, a rationale and the evidence
  // the diagnostician saw, and accepting a change to your own config off a
  // one-line title is the failure this queue exists to prevent. The server
  // refuses an unread decision too — a greyed button is only a suggestion.
  let { store = 'harness', onstore = () => {} } = $props();

  // Short enough to sit three-across on a phone. The store's own word for
  // itself ("harness candidates") comes from the API and heads the list.
  const shortName = { harness: 'Harness', rules: 'Rules', entities: 'Entities' };

  let stores = $state(null);
  let listing = $state(null); // { label, rows }
  let reading = $state(null); // { row, text }
  let error = $state(null);
  let busy = $state(false);
  let toast = $state(null);
  // Which confirmation is open: null, 'reject', or 'accept'. Accept needs
  // one too on the entities store — see `noUndo`.
  let sheet = $state(null);
  let reason = $state('');

  const dash = (v) => (v === null || v === undefined ? '—' : v);

  async function loadStores() {
    try {
      const res = await fetch('/api/proposals');
      if (!res.ok) throw new Error((await res.text()).trim());
      stores = await res.json();
    } catch (e) {
      stores = null; // a dash on every chip, never a row of zeroes
      error = String(e?.message ?? e);
    }
  }

  async function loadList() {
    listing = null;
    try {
      const res = await fetch(`/api/proposals/${store}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      listing = await res.json();
      error = null;
    } catch (e) {
      // The graph's binary may simply not be installed here; the server says
      // so by name with the variable that fixes it. Keep that sentence.
      error = String(e?.message ?? e);
    }
  }

  loadStores();
  // Re-runs whenever the chip selection changes; `store` is a prop so a deep
  // link (#review/harness) lands on the right store with no extra plumbing.
  $effect(() => {
    store;
    reading = null;
    loadList();
  });

  function pick(next) {
    if (next === store) return;
    error = null;
    onstore(next); // the route owns the selection, so Back works
  }

  async function open(row) {
    reading = { row, text: null };
    try {
      const res = await fetch(`/api/proposals/${store}/${encodeURIComponent(row.id)}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      reading = { row, text: (await res.json()).text };
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
      reading = null;
    }
  }

  function back() {
    reading = null;
    sheet = null;
    reason = '';
    loadStores();
    loadList();
  }

  async function decide(accepting) {
    busy = true;
    try {
      const verb = accepting ? 'accept' : 'reject';
      const res = await fetch(
        `/api/proposals/${store}/${encodeURIComponent(reading.row.id)}/${verb}`,
        {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          // `read` is the page saying it rendered `show`, which it only ever
          // does from inside this view.
          body: JSON.stringify({ reason: reason.trim(), read: reading.text !== null }),
        },
      );
      const text = await res.text();
      if (!res.ok) throw new Error(text.trim());
      // The child's own first line: an accept can *apply* something — an
      // override-layer entry, a merge — and what it did is the child's to say.
      const out = JSON.parse(text);
      toast = out.said || `${verb}ed`;
      setTimeout(() => (toast = null), 5000);
      error = null;
      back();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  // Entity proposals quote text that came out of somebody's mail or Slack, so
  // their evidence gets the third-party gutter the front door and mail bodies
  // use. Harness candidates and rule proposals are the harness's own account
  // of its own runs — marking those would say something untrue about where
  // the words came from.
  const thirdParty = $derived(store === 'entities');
  // `mecha-graph proposals reject` takes no `--reason`, so for the entities
  // store a typed reason goes nowhere — the server does not forward it and
  // the graph records none. Asking for one anyway, and blocking the button
  // until it is typed, collects a sentence into a bin and tells the owner it
  // is "the record". Ask where it is kept; say so where it is not.
  const reasonKept = $derived(store !== 'entities');
  // Accepting an entity proposal runs `mecha-graph proposals accept`, and for
  // a merge candidate that applies the merge — there is no unmerge. So the
  // phone's least reversible action must not also be its cheapest gesture:
  // before this, Accept was one un-confirmed tap while Reject — the
  // recoverable one — got the whole sheet, which is the asymmetry backwards.
  // The graph tab's own merge form, on the same verb, already says "no undo".
  const noUndo = $derived(store === 'entities');
  const current = $derived((stores ?? []).find((s) => s.store === store) ?? null);
  // A depth of null is "could not look", which is not a disagreement with
  // anything — only two real numbers that differ are worth a line.
  const shownOfTotal = $derived.by(() => {
    const shown = listing?.rows?.length;
    const total = current?.depth;
    if (shown === undefined || total === null || total === undefined) return null;
    return shown < total ? { shown, total } : null;
  });
</script>

{#snippet hazardGlyph(size = 13)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

<div class="page">
  {#if !reading}
    <div class="chips">
      {#each stores ?? [{ store: 'harness' }, { store: 'rules' }, { store: 'entities' }] as s}
        <button class="chipbtn" class:active={s.store === store} onclick={() => pick(s.store)}>
          {shortName[s.store] ?? s.store}
          <span class="chipcount">{dash(s.depth)}</span>
        </button>
      {/each}
    </div>
    <div class="scroll">
      {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}
      {#if current?.oldest}
        <div class="waitline">oldest has waited {current.oldest}</div>
      {/if}
      {#if shownOfTotal}
        <!-- Two numbers that disagree, said out loud. The listing verbs are
             asked for far more than any real backlog, so this should never
             appear — and if it does, the honest reading is that the store
             outgrew the surface, not that the chip is wrong. -->
        <div class="waitline">showing {shownOfTotal.shown} of {shownOfTotal.total}</div>
      {/if}
      {#if listing === null && !error}
        <div class="empty">reading the store…</div>
      {:else}
        {#each listing?.rows ?? [] as r}
          <button class="card rowbtn" onclick={() => open(r)}>
            <div class="rowtop">
              {#if r.kind}<span class="chip">{r.kind}</span>{/if}
              <span class="when">{r.id}</span>
            </div>
            <div class="topic">{r.title || r.id}</div>
            {#if r.detail}<div class="readingline">{r.detail}</div>{/if}
          </button>
        {:else}
          {#if !error}
            <div class="empty">Nothing staged in {listing?.label ?? 'this store'}.</div>
          {/if}
        {/each}
      {/if}
      {#if current?.opens}
        <!-- The phone must never be the only way to reach a queue. -->
        <div class="opens">{current.opens}</div>
      {/if}
    </div>
  {:else}
    <div class="scroll">
      <div class="deckhead">
        <button class="backbtn" onclick={back} aria-label="back">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6l-6 6 6 6" /></svg>
        </button>
        <span class="dtitle">{reading.row.id}</span>
      </div>
      {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}
      {#if reading.text === null}
        <div class="empty">reading the candidate…</div>
      {:else if thirdParty}
        <div class="quoted"><span class="gutter"></span><div class="qtext">{reading.text}</div></div>
      {:else}
        <div class="qtext">{reading.text}</div>
      {/if}
    </div>
    {#if reading.text !== null && !sheet}
      <div class="actionbar">
        <button
          class="abtn primary"
          disabled={busy}
          onclick={() => (noUndo ? (sheet = 'accept') : decide(true))}
        >
          {noUndo ? 'Accept…' : 'Accept'}
        </button>
        <button class="abtn" disabled={busy} onclick={() => { sheet = 'reject'; reason = ''; }}>
          Reject…
        </button>
      </div>
      <div class="barnote">
        {#if store === 'harness'}
          A config change inside the closed override set applies on accept — reversibly, through
          a layer your own config beats. Anything else is only marked.
        {:else if noUndo}
          Accepting applies the proposal to the graph. A merge cannot be undone.
        {:else}
          Decided here exactly as the command line decides it.
        {/if}
      </div>
    {/if}

    {#if sheet === 'accept'}
      <div class="sheet">
        <div class="sheet-grip"></div>
        <div class="sheet-text">Accept this proposal?</div>
        <div class="sheetnote">
          {@render hazardGlyph()}
          <span>
            This applies it to the graph now. If it is a merge, the two entities become one and
            there is no unmerge.
          </span>
        </div>
        <div class="btnrow">
          <button class="abtn" onclick={() => (sheet = null)}>Back</button>
          <button class="abtn primary" disabled={busy} onclick={() => decide(true)}>Accept</button>
        </div>
      </div>
    {/if}

    {#if sheet === 'reject'}
      <div class="sheet">
        <div class="sheet-grip"></div>
        <div class="sheet-text">
          {reasonKept
            ? 'Why? The reason is the record.'
            : 'Reject this proposal? The graph keeps no reason, so anything typed here is not stored.'}
        </div>
        {#if reasonKept}
          <textarea
            class="editbox"
            rows="3"
            bind:value={reason}
            placeholder="the prediction does not follow from the evidence"
          ></textarea>
        {/if}
        <div class="btnrow">
          <button class="abtn" onclick={() => (sheet = null)}>Back</button>
          <button
            class="abtn primary"
            disabled={busy || (reasonKept && !reason.trim())}
            onclick={() => decide(false)}
          >
            Reject
          </button>
        </div>
      </div>
    {/if}
  {/if}

  {#if toast}<div class="toast">{toast}</div>{/if}
</div>

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  .chips { display: flex; gap: 8px; padding: 12px 20px 0; overflow-x: auto; }
  .chipbtn { flex-shrink: 0; display: flex; align-items: center; gap: 7px; min-height: 40px; padding: 0 13px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); color: var(--text-muted); font-family: var(--mono); font-size: 12px; cursor: pointer; }
  .chipbtn.active { color: var(--text); background: var(--accent-900); border-color: var(--accent-700); }
  .chipcount { color: var(--accent-400); font-size: 13px; }
  .scroll { flex: 1; overflow-y: auto; padding: 14px 20px; display: flex; flex-direction: column; gap: 10px; }
  .scroll > * { flex-shrink: 0; }
  .rowbtn { text-align: left; padding: 14px; display: flex; flex-direction: column; gap: 6px; cursor: pointer; color: var(--text); font: inherit; overflow: hidden; }
  .rowtop { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .when { font-family: var(--mono); font-size: 10px; color: var(--accent-700); margin-left: auto; overflow: hidden; text-overflow: ellipsis; }
  .topic { font-family: var(--mono); font-size: 14px; line-height: 1.35; overflow-wrap: anywhere; }
  .readingline { font-size: 12px; line-height: 1.45; color: var(--text-muted); overflow: hidden; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; }
  .waitline, .opens { font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .opens { margin-top: auto; padding-top: 10px; border-top: 1px solid var(--accent-900); overflow-wrap: anywhere; }
  .empty { color: var(--text-muted); font-size: 14px; padding: 24px 0; text-align: center; }
  .warnline { display: flex; align-items: flex-start; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .deckhead { display: flex; align-items: center; gap: 8px; }
  .backbtn { background: none; border: none; color: var(--text-muted); min-width: 44px; min-height: 44px; margin: -10px 0 -10px -12px; cursor: pointer; display: flex; align-items: center; justify-content: center; }
  .dtitle { font-family: var(--mono); font-size: 13px; color: var(--accent-400); overflow-wrap: anywhere; }
  .quoted { display: flex; gap: 10px; }
  .gutter { width: 2px; background: var(--hazard); flex-shrink: 0; }
  .qtext { font-family: var(--mono); font-size: 12px; line-height: 1.6; color: var(--text); white-space: pre-wrap; overflow-wrap: anywhere; }
  .actionbar { display: flex; gap: 8px; overflow-x: auto; padding: 10px 20px 4px; border-top: 1px solid var(--accent-900); background: var(--bg); }
  .abtn { flex-shrink: 0; min-height: 44px; padding: 0 16px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; white-space: nowrap; }
  .abtn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .abtn:disabled { opacity: 0.5; }
  .barnote { font-size: 10px; color: var(--text-muted); text-align: center; padding: 4px 20px 8px; background: var(--bg); line-height: 1.5; }
  .btnrow { display: flex; gap: 10px; }
  .btnrow .abtn { flex: 1; }
  .editbox { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; line-height: 1.5; padding: 12px 14px; resize: vertical; box-sizing: border-box; }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px 20px 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; }
  .sheetnote { display: flex; align-items: flex-start; gap: 8px; font-size: 12px; line-height: 1.45; color: var(--hazard); }
  .toast { position: absolute; bottom: 18px; left: 50%; transform: translateX(-50%); background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 10px 16px; font-size: 13px; white-space: nowrap; max-width: 90%; overflow: hidden; text-overflow: ellipsis; }
</style>
