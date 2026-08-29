<script>
  // The entity page: everything the graph knows about one node, on the
  // phone. The web twin of the TUI's /entity — and, like it, a review
  // surface in its own right: opening an entity is one of review-on-use's
  // verdict triggers, so unreviewed (◌) facts carry Confirm/Refute here,
  // wired to the same owner-shaped verdict route the review page uses.
  // A denial (✗) renders dimmed and settled — a recorded no, not a weak
  // yes. Both labels arrive from the server (`tier`, `polarity`) and are
  // never derived in page script.
  let { initial = null } = $props();

  let query = $state(initial ? decodeURIComponent(initial) : '');
  let entity = $state(null); // the kg_entity envelope
  let busy = $state(false);
  let error = $state(null);
  let reasons = $state({}); // fact uid → typed refute reason
  let notes = $state({}); // fact uid → verdict error
  let said = $state(null); // one-line confirmation of the last verdict

  async function lookup(name) {
    const n = (name ?? query).trim();
    if (!n) return;
    busy = true;
    said = null;
    try {
      const res = await fetch(`/api/entity?${new URLSearchParams({ name: n })}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      entity = await res.json();
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }
  if (query) lookup(query);

  // One verdict, in place: the same route as the review page, because two
  // verdict paths would be two things to keep honest. On success the page
  // re-reads the entity — the fact's tier changed server-side, and this
  // page renders the store, never its own recollection of a click.
  async function factVerdict(f, confirm) {
    busy = true;
    try {
      const res = await fetch('/api/queue/shadow/verdict', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ uid: f.uid, confirm, reason: reasons[f.uid]?.trim() || null }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      said = confirm ? 'confirmed — now reviewed' : 'refuted — retracted as never true';
      const rest = { ...notes };
      delete rest[f.uid];
      notes = rest;
      await lookup(entity?.node?.name ?? query);
    } catch (e) {
      notes = { ...notes, [f.uid]: String(e?.message ?? e) };
    } finally {
      busy = false;
    }
  }

  const unreviewed = (f) => f.tier !== 'reviewed';
  const day = (ts) => (ts ?? '').slice(0, 10);
</script>

<div class="pane">
  <form
    class="lookuprow"
    onsubmit={(e) => {
      e.preventDefault();
      lookup();
    }}
  >
    <input
      class="field"
      placeholder="entity — name, alias, or email"
      bind:value={query}
      autocapitalize="off"
    />
    <button class="minibtn" disabled={busy || !query.trim()}>Open</button>
  </form>

  {#if error}<div class="warnline">{error}</div>{/if}
  {#if said}<div class="saidline">{said}</div>{/if}

  {#if entity?.found === false}
    <div class="empty">no entity matches “{entity.query}”</div>
  {:else if entity?.ambiguous?.length}
    <div class="footnote">several entities answer to this name — pick one:</div>
    {#each entity.ambiguous as c}
      <button class="card row" onclick={() => lookup(c.id)}>
        <div class="rowtop">
          <span class="pname">{c.name}</span>
          <span class="chip">{c.type}</span>
        </div>
        <div class="rowsub">
          <span>{c.interaction_count} interactions · last seen {day(c.last_seen) || '—'}</span>
        </div>
      </button>
    {/each}
  {:else if entity?.node}
    {@const n = entity.node}
    <div class="card head">
      <div class="rowtop">
        <span class="ename">{n.name}</span>
        <span class="chip">{n.node_type ?? n.type}</span>
      </div>
      {#if entity.interaction}
        <div class="rowsub">
          <span>
            {entity.interaction.interaction_count} interactions · last seen
            {day(entity.interaction.last_seen_at) || '—'} via
            {entity.interaction.last_channel ?? '—'}
          </span>
        </div>
      {/if}
    </div>

    {#if entity.facts?.length}
      <div class="sect">facts</div>
      {#each entity.facts as f (f.uid)}
        <div class="card fact" class:denied={f.polarity === 'negative'}>
          <div class="factline">
            {#if unreviewed(f)}<span class="unrev" title="unreviewed">◌</span>{/if}
            {#if f.polarity === 'negative'}<span class="neg">✗</span>{/if}
            <span class="statement">{f.statement}</span>
          </div>
          <div class="meta">
            <span>{f.predicate}</span>
            {#if f.valid_from}<span>· as of {day(f.valid_from)}</span>{/if}
            <span>· {f.extractor ?? '?'}</span>
            {#if unreviewed(f)}<span class="unrevword">· unreviewed</span>{/if}
          </div>
          {#if unreviewed(f)}
            <input
              class="field small"
              placeholder="refute reason — feeds rejection memory (optional)"
              bind:value={reasons[f.uid]}
            />
            <div class="btnrow">
              <button class="minibtn" disabled={busy} onclick={() => factVerdict(f, false)}
                >Refute</button
              >
              <button class="minibtn primary" disabled={busy} onclick={() => factVerdict(f, true)}
                >Confirm</button
              >
            </div>
            {#if notes[f.uid]}<div class="warnline">{notes[f.uid]}</div>{/if}
          {/if}
        </div>
      {/each}
    {/if}

    {#if entity.episodes?.length}
      <div class="sect">recent evidence</div>
      {#each entity.episodes as ep (ep.uid)}
        <div class="card ep">
          <div class="meta"><span>{day(ep.occurred_at)}</span><span>· {ep.source}</span></div>
          <div class="preview">{ep.preview}</div>
        </div>
      {/each}
    {/if}
  {:else if busy}
    <div class="empty">reading the graph…</div>
  {:else}
    <div class="empty">
      Name an entity to see everything the graph knows — its facts (unreviewed ones marked ◌,
      decidable right here), recorded denials, and the evidence behind them.
    </div>
  {/if}
</div>

<style>
  .pane { display: flex; flex-direction: column; gap: 10px; padding: 16px 16px 8px; overflow-y: auto; flex: 1; }
  .lookuprow { display: flex; gap: 8px; }
  .field { flex: 1; min-height: 44px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; padding: 0 12px; }
  .field.small { min-height: 38px; font-size: 12px; }
  .minibtn { min-height: 44px; padding: 0 16px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); color: var(--text); font-size: 13px; cursor: pointer; }
  .minibtn.primary { background: var(--accent-400); color: var(--void); border: none; font-weight: 500; }
  .minibtn:disabled { opacity: 0.5; }
  .card { background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); padding: 12px 14px; display: flex; flex-direction: column; gap: 7px; }
  .row { text-align: left; cursor: pointer; color: var(--text); font: inherit; }
  .rowtop { display: flex; align-items: center; gap: 8px; }
  .rowsub { font-size: 11px; color: var(--text-muted); }
  .head { border-color: var(--accent-700); }
  .ename { font-size: 16px; font-weight: 500; }
  .pname { font-family: var(--mono); font-size: 13px; color: var(--accent-400); }
  .chip { font-family: var(--mono); font-size: 10px; color: var(--text-muted); background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); padding: 3px 8px; margin-left: auto; }
  .sect { font-family: var(--mono); font-size: 11px; color: var(--text-muted); margin-top: 4px; }
  .factline { display: flex; gap: 7px; align-items: baseline; }
  .statement { font-size: 14px; line-height: 1.5; }
  .fact.denied .statement { color: var(--text-muted); }
  .unrev { color: var(--accent-400); font-weight: 600; }
  .neg { color: var(--hazard); }
  .unrevword { color: var(--accent-400); }
  .meta { display: flex; gap: 6px; flex-wrap: wrap; font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .btnrow { display: flex; gap: 8px; }
  .btnrow .minibtn { flex: 1; }
  .preview { font-size: 12px; color: var(--text-muted); line-height: 1.5; overflow: hidden; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 3; line-clamp: 3; }
  .warnline { font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .saidline { font-family: var(--mono); font-size: 11px; color: var(--accent-400); }
  .empty { color: var(--text-muted); font-size: 14px; padding: 20px 0; text-align: center; line-height: 1.6; }
</style>
