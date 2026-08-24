<script>
  // The graph queue on the phone, at the TUI /queues modal's three depths:
  // proposers → one proposer's classes (with the evidence-tier filter) →
  // either a random sample deck or the class's similarity groups.
  //
  // The sampling rules are the CLI's: the seed is drawn server-side and
  // printed here, a verdict never resamples (the card is dropped locally;
  // these twelve stay one sample), and a new draw is an explicit button. An
  // unjudged class shows a dash, never 0% — "untouched" and "rejected" are
  // opposite findings. Tiers arrive stamped by the server
  // (`tui::queues::Tier::of`, the single definition) and are never
  // re-derived here, where the thresholds would drift.
  //
  // Groups are where one verdict fans out furthest: a group's face is a
  // real member statement, never a paraphrase, and a group verdict is ONE
  // human verdict — the leader is yours, the members follow as a labeled
  // machine cascade the autonomy ladder never counts. A class group never
  // crosses a class; the front screen's "similar across everything" is the
  // invited crossing — stricter floor, every class named on the card.
  let proposers = $state(null);
  let classes = $state(null); // { proposer, rows }
  let tierFilter = $state(null); // null = all
  let groups = $state(null); // { proposer, predicate, threshold, rows }
  let deck = $state(null); // { proposer, predicate, seed, items, judged }
  let error = $state(null);
  let busy = $state(false);

  const TIERS = ['unjudged', 'thin', 'some', 'solid'];

  async function load() {
    try {
      const res = await fetch('/api/queue');
      if (!res.ok) throw new Error((await res.text()).trim());
      proposers = await res.json();
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }
  load();

  async function openClasses(proposer) {
    try {
      const q = new URLSearchParams({ proposer });
      const res = await fetch(`/api/queue/classes?${q}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      classes = { proposer, rows: await res.json() };
      tierFilter = null;
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  async function openGroups(proposer, predicate) {
    groups = { proposer, predicate, threshold: null, rows: null };
    try {
      const q = new URLSearchParams({ proposer, predicate });
      const res = await fetch(`/api/queue/groups?${q}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      groups = { proposer, predicate, threshold: data.threshold, rows: data.groups };
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
      groups = null;
    }
  }

  // The top layer: near-repeats across the WHOLE queue, wherever they sit.
  // Embedding every pending statement takes minutes, and an honest wait
  // message beats a spinner that looks hung.
  async function openGlobal(threshold = null) {
    groups = { all: true, threshold: null, rows: null, considered: null };
    try {
      const q = new URLSearchParams({ all: 'true' });
      // Only a real number becomes a param — an event object handed by a
      // bare onclick={openGlobal} must fall through to the server default.
      if (typeof threshold === 'number' && isFinite(threshold)) q.set('threshold', threshold.toFixed(2));
      const res = await fetch(`/api/queue/groups?${q}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      groups = { all: true, threshold: data.threshold, rows: data.groups, considered: data.considered };
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
      groups = null;
    }
  }

  async function draw(proposer, predicate = null, seed = null) {
    busy = true;
    try {
      const res = await fetch('/api/queue/sample', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ proposer, predicate, seed }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      deck = { proposer, predicate, seed: data.seed, items: data.items, judged: 0, total: data.items.length };
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function verdict(accept) {
    const item = deck.items[0];
    busy = true;
    try {
      const res = await fetch('/api/queue/verdict', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ id: item.id, accept }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      deck.items.shift();
      deck.judged += 1;
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  // One tap, one human verdict: the leader id is the owner's, the member
  // ids ride as the cascade — always the ids the page showed, never a
  // re-derived similarity.
  async function groupVerdict(g, accept) {
    busy = true;
    try {
      const res = await fetch('/api/queue/verdict', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ id: g.leader_id, accept, cascade: g.members.map((m) => m[0]), across: !!groups.all }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      groups.rows = groups.rows.filter((r) => r !== g);
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  function skip() {
    deck.items.push(deck.items.shift());
  }

  const rate = (p) => {
    const judged = p.accepted_hist + p.rejected_hist;
    return judged > 0 ? `${Math.round((p.accepted_hist / judged) * 100)}%` : '—';
  };
  const shownClasses = $derived(
    classes ? classes.rows.filter((c) => !tierFilter || c.tier === tierFilter) : []
  );
</script>

{#snippet hazardGlyph(size = 13)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

{#snippet backTo(action, label)}
  <button class="backbtn" onclick={action} aria-label={label}>
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6l-6 6 6 6" /></svg>
  </button>
{/snippet}

<div class="pane">
  {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}

  {#if deck}
    <div class="deckhead">
      {@render backTo(() => { deck = null; }, 'back')}
      <span class="pname">{deck.proposer}{deck.predicate ? ` · ${deck.predicate}` : ''}</span>
      <span class="seed">sample of {deck.total} · seed {deck.seed}</span>
    </div>
    <div class="progress">
      <div class="bar"><div class="fill" style:width="{(deck.judged / deck.total) * 100}%"></div></div>
      <span class="seed">{deck.judged} / {deck.total}</span>
    </div>

    {#if deck.items.length === 0}
      <div class="empty">
        Sample done — {deck.judged} verdicts on one draw.
        <button class="btn" onclick={() => draw(deck.proposer, deck.predicate)}>New draw</button>
      </div>
    {:else}
      {@const item = deck.items[0]}
      <div class="card candidate">
        <div class="kicker">proposed belief</div>
        <div class="statement">
          {item.payload?.statement ?? `${item.payload?.subject} — ${item.payload?.predicate} — ${item.payload?.object}`}
        </div>
        <div class="meta">
          <span>confidence {item.confidence?.toFixed(2) ?? '—'}</span>
          <span>·</span>
          <span>{item.proposed_by}</span>
          <span>·</span>
          <span>{(item.created_at ?? '').slice(0, 10)}</span>
        </div>
      </div>
      <div class="btnrow">
        <button class="btn" disabled={busy} onclick={() => verdict(false)}>Reject</button>
        <button class="btn primary" disabled={busy} onclick={() => verdict(true)}>Accept</button>
      </div>
      <div class="deckfoot">
        <button class="ghost" onclick={skip}>Skip for now</button>
        <button class="ghost" disabled={busy} onclick={() => draw(deck.proposer, deck.predicate)}>New draw</button>
      </div>
      <div class="footnote">These verdicts describe one sample — the seed is printed above.</div>
    {/if}
  {:else if groups}
    <div class="deckhead">
      {@render backTo(() => { groups = null; }, 'back')}
      <span class="pname">{groups.all ? 'across all classes' : groups.predicate}</span>
      {#if groups.threshold != null}
        <span class="seed">cosine ≥ {groups.threshold.toFixed(2)}</span>
        {#if groups.all}
          <!-- Step from the threshold the envelope says RAN, never from a
               constant of this page's own — the drifted-literal trap. -->
          <button class="stepbtn" title="looser — bigger groups, read more carefully" onclick={() => openGlobal(groups.threshold - 0.03)}>−</button>
          <button class="stepbtn" title="stricter — only near-identical" onclick={() => openGlobal(groups.threshold + 0.03)}>+</button>
        {/if}
      {/if}
    </div>
    {#if groups.rows === null}
      <div class="empty">
        {groups.all
          ? 'embedding the whole queue — this takes a couple of minutes, stay put'
          : 'grouping by similarity…'}
      </div>
    {:else if groups.rows.length === 0}
      <div class="empty">Nothing repeats above the threshold — review item by item.</div>
    {:else}
      {#if groups.all}
        <div class="footnote">
          {groups.rows.length} groups covering
          {groups.rows.reduce((n, g) => n + g.members.length + 1, 0)} of {groups.considered} pending ·
          singletons stay in their class listings. One tap is one human verdict — the shown
          statement is yours, the rest follow as a labeled machine cascade, and each group names
          every class it touches.
        </div>
      {:else}
        <div class="footnote">
          A group verdict is one human verdict: the shown statement is yours, the rest follow as a
          labeled machine cascade. A class group never crosses a class — the “everything” layer is
          on the front screen.
        </div>
      {/if}
      {#each groups.rows as g}
        <div class="card candidate">
          <div class="kicker">{g.members.length + 1} near-repeats · leader #{g.leader_id}</div>
          <div class="statement">{g.leader_statement}</div>
          {#if groups.all && g.classes}
            <div class="spans">
              {#each Object.entries(g.classes) as [c, n]}
                <span class="spanchip">{c} ×{n}</span>
              {/each}
            </div>
          {/if}
          {#each g.sample as s}
            <div class="member">≈ {s}</div>
          {/each}
          <div class="btnrow">
            <button class="btn" disabled={busy} onclick={() => groupVerdict(g, false)}>Reject all {g.members.length + 1}</button>
            <button class="btn primary" disabled={busy} onclick={() => groupVerdict(g, true)}>Accept all {g.members.length + 1}</button>
          </div>
        </div>
      {/each}
    {/if}
  {:else if classes}
    <div class="deckhead">
      {@render backTo(() => { classes = null; load(); }, 'back to proposers')}
      <span class="pname">{classes.proposer}</span>
      <span class="seed">{shownClasses.length} of {classes.rows.length} classes</span>
    </div>
    <div class="tierchips">
      <button class="tchip" class:on={tierFilter === null} onclick={() => (tierFilter = null)}>all</button>
      {#each TIERS as t}
        <button class="tchip" class:on={tierFilter === t} onclick={() => (tierFilter = t)}>{t}</button>
      {/each}
    </div>
    {#each shownClasses as c}
      <div class="card row">
        <div class="rowtop">
          <span class="pname">{c.predicate}</span>
          <span class="chip">{c.tier}</span>
          <span class="pcount">{c.pending.toLocaleString('en-US')}</span>
        </div>
        {#if c.samples?.length}<div class="sample">{c.samples[0]}</div>{/if}
        <div class="rowsub">
          <span>your accept rate {rate(c)} over {c.accepted_hist + c.rejected_hist} verdicts</span>
        </div>
        <div class="rowbtns">
          <button class="minibtn" disabled={busy} onclick={() => draw(classes.proposer, c.predicate)}>Sample 12</button>
          <button class="minibtn" disabled={busy} onclick={() => openGroups(classes.proposer, c.predicate)}>Similar groups</button>
        </div>
      </div>
    {:else}
      <div class="empty">No classes in this tier.</div>
    {/each}
  {:else if proposers === null && !error}
    <div class="empty">reading the queue…</div>
  {:else if proposers}
    <button class="card row global" onclick={() => openGlobal()} disabled={busy}>
      <div class="rowtop">
        <span class="pname">similar across everything</span>
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="var(--accent-400)" stroke-width="1.8" stroke-linecap="round"><path d="M13 3L4 14h6l-1 7 9-11h-6z" /></svg>
      </div>
      <div class="rowsub">
        <span>near-repeats grouped over the whole queue, wherever they sit — the fast way through {proposers.reduce((n, p) => n + p.pending, 0).toLocaleString('en-US')} pending</span>
      </div>
    </button>
    {#each proposers as p}
      <button class="card row" onclick={() => openClasses(p.proposer)} disabled={busy}>
        <div class="rowtop">
          <span class="pname">{p.proposer}</span>
          <span class="chip">{p.tier}</span>
          <span class="pcount">{p.pending.toLocaleString('en-US')}</span>
        </div>
        <div class="rowsub">
          <span>your accept rate {rate(p)} over {p.accepted_hist + p.rejected_hist} verdicts</span>
          <span class="dim">{p.classes} class{p.classes === 1 ? '' : 'es'}</span>
        </div>
      </button>
    {:else}
      <div class="empty">The queue is empty.</div>
    {/each}
  {/if}
</div>

<style>
  .pane { display: flex; flex-direction: column; gap: 10px; }
  .warnline { display: flex; align-items: flex-start; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .row { text-align: left; padding: 14px; display: flex; flex-direction: column; gap: 7px; cursor: pointer; color: var(--text); font: inherit; }
  .rowtop { display: flex; align-items: center; gap: 8px; }
  .pname { font-family: var(--mono); font-size: 13px; color: var(--accent-400); overflow-wrap: anywhere; }
  .pcount { font-size: 16px; font-weight: 500; margin-left: auto; }
  .rowsub { display: flex; justify-content: space-between; font-size: 11px; color: var(--text-muted); }
  .dim { color: var(--accent-700); }
  .sample { font-size: 12px; color: var(--text-muted); line-height: 1.45; overflow: hidden; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; }
  .rowbtns { display: flex; gap: 8px; }
  .minibtn { flex: 1; min-height: 40px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); color: var(--text); font-size: 12px; cursor: pointer; }
  .minibtn:disabled { opacity: 0.5; }
  .tierchips { display: flex; gap: 6px; flex-wrap: wrap; }
  .tchip { font-family: var(--mono); font-size: 11px; color: var(--text-muted); background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); padding: 7px 11px; min-height: 34px; cursor: pointer; }
  .tchip.on { color: var(--text); background: var(--accent-900); border-color: var(--accent-700); }
  .empty { color: var(--text-muted); font-size: 14px; padding: 20px 0; text-align: center; display: flex; flex-direction: column; gap: 12px; align-items: center; }
  .deckhead { display: flex; align-items: center; gap: 10px; }
  .backbtn { background: none; border: none; color: var(--text-muted); min-width: 44px; min-height: 44px; margin: -10px 0 -10px -12px; cursor: pointer; display: flex; align-items: center; justify-content: center; }
  .seed { font-family: var(--mono); font-size: 11px; color: var(--text-muted); margin-left: auto; }
  .progress { display: flex; align-items: center; gap: 10px; }
  .bar { flex: 1; height: 3px; background: var(--accent-900); border-radius: 2px; overflow: hidden; }
  .fill { height: 3px; background: var(--accent-400); }
  .candidate { padding: 18px; display: flex; flex-direction: column; gap: 12px; background: var(--surface); border-color: var(--accent-700); }
  .statement { font-size: 15px; line-height: 1.5; }
  .member { font-size: 12px; line-height: 1.45; color: var(--text-muted); }
  .global { border-color: var(--accent-700); }
  .spans { display: flex; gap: 6px; flex-wrap: wrap; }
  .spanchip { font-family: var(--mono); font-size: 10px; color: var(--accent-400); background: var(--accent-900); border-radius: var(--radius-chip); padding: 4px 8px; }
  .stepbtn { font-family: var(--mono); font-size: 15px; color: var(--text); background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); min-width: 34px; min-height: 34px; cursor: pointer; }
  .meta { display: flex; gap: 8px; font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .btnrow { display: flex; gap: 10px; }
  .btn { flex: 1; min-height: 52px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 15px; cursor: pointer; }
  .btn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .btn:disabled { opacity: 0.5; }
  .deckfoot { display: flex; justify-content: space-between; }
  .ghost { background: none; border: none; color: var(--text-muted); font-size: 13px; min-height: 44px; cursor: pointer; }
  .footnote { font-size: 11px; color: var(--text-muted); text-align: center; }
</style>
