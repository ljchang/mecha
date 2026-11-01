<script>
  // Her week (PERSONA-LIFE-DESIGN.md §15): each day of the week her routine
  // and the time-use table drew, 4am to 4am. Read only: what is drawn is
  // kept, and a day ahead is shown as it would be drawn now. A persona with
  // her life switched off says so here rather than failing.
  import { untrack } from 'svelte';
  import { apiFetch as fetch } from './api.js';
  import { personaUrl, weekUrl, weekDays, foldedHours, weekUnavailable, shiftWeek, cellBody, draftBody, anchorLine, WEEK_ABSENT, WEEK_GROUPS, WEEK_CELLS } from './persona.js';

  let { name, token = null, onclose } = $props();

  let data = $state(null);
  let error = $state('');
  // Any day of the week shown; null is this week.
  let date = $state(null);
  // The stretch tapped, as `${date}/${index}`, whose line shows under its day.
  let picked = $state(null);
  // A cell pick on its way to routine.toml.
  let saving = $state(false);
  // A draft being asked of her model, or acted on.
  let drafting = $state(false);
  let acting = $state(false);
  // The owner's text while editing a draft, null when not editing.
  let draftText = $state(null);
  // What the last draft found, when it found nothing to stage.
  let draftNote = $state('');

  const days = $derived(weekDays(data));
  const folded = $derived(foldedHours(data));
  const unavailable = $derived(weekUnavailable(data));
  const monday = $derived(data?.week?.monday ?? null);
  const draft = $derived(data?.routine?.draft ?? null);
  const cellLabel = (id) => WEEK_CELLS.find(([c]) => c === id)?.[1] ?? id;
  // Where the bar's ticks sit: 8am, noon, 6pm and midnight on a 4am day.
  const TICKS = [
    [((8 * 60 - 240) / 1440) * 100, '8'],
    [((12 * 60 - 240) / 1440) * 100, '12'],
    [((18 * 60 - 240) / 1440) * 100, '18'],
    [((24 * 60 - 240) / 1440) * 100, '0'],
  ];

  async function load() {
    error = '';
    try {
      // Asked with the page's header: this read stores the days it draws, so
      // the server wants proof it came from here (`request_verified`).
      const res = await fetch(weekUrl(name, date, token), { headers: { 'X-Mecha-Request': '1' } });
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      picked = null;
    } catch (e) {
      // A failed step keeps no stale week under the error, or the nav
      // would step on from a week no longer shown.
      data = null;
      error = String(e?.message ?? e);
    }
  }

  // The owner picks her kind of week: written into routine.toml over the
  // file as shown, and the week answered back as it now stands. A refused
  // pick puts the picker back on the cell her routine names, never the one
  // refused; a stale one (409, an edit made elsewhere) reads the week
  // again, so the next pick is made over the file as it now stands.
  async function pickCell(select, cell) {
    // Not while a draft runs or lands: whichever answer came last would
    // show, and a pick's answer was read before the draft was staged.
    if (!cell || cell === data?.routine?.cell || drafting || acting) {
      select.value = data?.routine?.cell ?? '';
      return;
    }
    saving = true;
    error = '';
    try {
      const res = await fetch(personaUrl(name, '/routine/cell', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(cellBody(cell, data, date, token)),
      });
      if (res.status === 409) {
        const why = (await res.text()).trim();
        await load();
        // A reload that failed keeps its own words: the week is gone, and
        // saying it is shown would say the opposite of the page.
        if (data) error = `${why} It is shown as it stands now.`;
        return;
      }
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      picked = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      saving = false;
      select.value = data?.routine?.cell ?? '';
    }
  }

  // Her model drafts her week from what the owner wrote of her (§4.1): a
  // model run of tens of seconds, so the button says so while it runs. Zero
  // anchors is the model finding no fixed point, said rather than failed.
  async function draftWeek() {
    drafting = true;
    error = '';
    draftNote = '';
    try {
      const res = await fetch(personaUrl(name, '/routine/draft', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(draftBody(data, date, token)),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      picked = null;
      if (data.drafted === 0) draftNote = 'Her model found no fixed point in her week to draft from what you wrote of her.';
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      drafting = false;
    }
  }

  // The owner's doors to a waiting draft: approve it as shown, save an edit
  // over it as shown, or discard it. A 409 (changed since it was shown)
  // reads the week again, as a stale cell pick does.
  async function actOnDraft(method, suffix, text = undefined) {
    acting = true;
    error = '';
    try {
      const res = await fetch(personaUrl(name, suffix, null), {
        method,
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(draftBody(data, date, token, text)),
      });
      if (res.status === 409) {
        const why = (await res.text()).trim();
        await load();
        draftText = null;
        if (data) error = `${why} It is shown as it stands now.`;
        return;
      }
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      draftText = null;
      picked = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      acting = false;
    }
  }

  function step(weeks) {
    if (!monday) return;
    date = shiftWeek(monday, weeks);
    load();
  }

  $effect(() => {
    // Back to this week when the persona or the unlock changes under the
    // page. Untracked, or the `date` that `load` reads would make every
    // step through the weeks land back on this one.
    void name;
    void token;
    untrack(() => {
      date = null;
      draftText = null;
      draftNote = '';
      load();
    });
  });
</script>

<div class="week">
  {#if error}<div class="warnline">{error}</div>{/if}
  {#if !data && error}
    <!-- A failed step keeps no week, so the nav goes with it: these are
         the way on from here (review of #797). -->
    <div class="wnav">
      <button class="linkbtn" onclick={load}>Try again</button>
      <button class="linkbtn" onclick={() => { date = null; load(); }}>This week</button>
    </div>
  {/if}
  {#if !data && !error}
    <div class="wnote">loading…</div>
  {:else if data?.absent}
    <p class="wnote">{WEEK_ABSENT[data.absent] ?? data.why}</p>
  {/if}
  {#if data && data.absent !== 'off' && data.routine}
    <label class="wcell">
      <span>Kind of week</span>
      <select disabled={saving || drafting || acting} value={data.routine.cell ?? ''} onchange={(e) => pickCell(e.currentTarget, e.currentTarget.value)}>
        {#if !data.routine.cell}<option value="" disabled>Pick one…</option>{/if}
        {#each WEEK_CELLS as [id, label]}<option value={id}>{label}</option>{/each}
      </select>
    </label>
    <!-- Her routine's anchors, drafted by her model and shown as they would
         land before anything reaches her days (§4.1). -->
    <div class="wdraft">
      {#if draft}
        <div class="wdhead">A drafted week waits on you{#if draft.edited} · edited by you{/if}</div>
        {#if draftText !== null}
          <textarea class="wdtext" rows="14" spellcheck="false" bind:value={draftText} disabled={acting}></textarea>
          <div class="wdacts">
            <button class="abtn" disabled={acting} onclick={() => actOnDraft('PUT', '/routine/draft', draftText)}>Save draft</button>
            <button class="linkbtn" disabled={acting} onclick={() => (draftText = null)}>Cancel</button>
          </div>
        {:else}
          {#if draft.anchors?.length}
            <ul class="wdlist">{#each draft.anchors as a}<li>{anchorLine(a)}</li>{/each}</ul>
          {:else if draft.anchors}
            <div class="warnline">It holds no anchors, so approving it would empty her week. Edit it or discard it.</div>
          {:else}
            <div class="warnline">It does not load, so it cannot be approved yet: {draft.error}</div>
          {/if}
          {#if draft.kept_cell}<p class="wnote">Approving keeps her kind of week: {cellLabel(draft.kept_cell)}.</p>{/if}
          {#if data.routine.digest}<p class="wnote">Approving replaces the anchors her routine has now; the old routine is kept beside it.</p>{/if}
          <div class="wdacts">
            <button class="abtn" disabled={acting || saving || !draft.anchors?.length} onclick={() => actOnDraft('POST', '/routine/draft/approve')}>Approve</button>
            <button class="linkbtn" disabled={acting} onclick={() => (draftText = draft.text)}>Edit</button>
            <button class="linkbtn" disabled={acting} onclick={() => actOnDraft('DELETE', '/routine/draft')}>Discard</button>
          </div>
        {/if}
      {:else}
        <button class="abtn" disabled={drafting || saving || !!data.routine.undraftable} onclick={draftWeek}>
          {drafting ? 'Drafting her week…' : 'Draft her week from what you wrote'}
        </button>
        {#if data.routine.undraftable}<p class="wnote">{data.routine.undraftable}</p>{/if}
        {#if draftNote}<p class="wnote">{draftNote}</p>{/if}
      {/if}
    </div>
  {/if}
  {#if data?.week}
    <div class="wnav">
      <button class="linkbtn" onclick={() => step(-1)}>‹ Earlier</button>
      <span class="wtitle">Week of {monday}</span>
      <button class="linkbtn" onclick={() => step(1)}>Later ›</button>
    </div>
    <div class="wrows">
      {#each days as d (d.date)}
        <div class="wrow" class:today={d.when === 'today'}>
          <div class="wday">
            <span>{d.weekday} {d.dayOfMonth}</span>
            {#if d.when === 'today'}<span class="wtag">today</span>{:else if d.when === 'ahead'}<span class="wtag dim">ahead</span>{/if}
          </div>
          {#if d.folded}
            <!-- The store counts folded days a week, never which, so a day
                 here may be folded or may never have been drawn. -->
            <div class="wbar folded"><span>not kept</span></div>
          {:else if d.undrawn}
            <div class="wbar folded"><span>not drawn</span></div>
          {:else}
            <div class="wbar" class:ahead={d.when === 'ahead'}>
              {#each TICKS as [at]}<span class="tick" style:left="{at}%"></span>{/each}
              {#each d.bars as b, i}
                <button
                  class="seg {b.group}"
                  class:unclean={b.unclean}
                  class:sel={picked === `${d.date}/${i}`}
                  style:left="{b.left}%"
                  style:width="{b.width}%"
                  title={b.line}
                  aria-label={b.line}
                  onclick={() => (picked = picked === `${d.date}/${i}` ? null : `${d.date}/${i}`)}
                ></button>
              {/each}
            </div>
          {/if}
          {#each d.bars as b, i}
            {#if picked === `${d.date}/${i}`}
              <div class="wline">{b.line}{#if b.unclean} · from a source this version does not know{/if}</div>
            {/if}
          {/each}
        </div>
      {/each}
      <div class="wrow">
        <div class="wday"></div>
        <div class="wscale">
          <span style:left="0%">4</span>
          {#each TICKS as [at, label]}<span style:left="{at}%">{label}</span>{/each}
        </div>
      </div>
    </div>
    {#if unavailable}
      <div class="warnline">The days not drawn wait on her routine: {unavailable}</div>
    {/if}
    <div class="wlegend">
      {#each WEEK_GROUPS as [group, label]}<span class="lchip"><span class="swatch {group}"></span>{label}</span>{/each}
    </div>
    {#if folded}
      <div class="wnote">
        {folded.days} {folded.days === 1 ? 'day' : 'days'} folded: {folded.rows.map((r) => `${r.activity} ${r.hours}`).join(', ')}{#if folded.unreadable}, and {folded.unreadable} this version cannot name{/if}.
      </div>
    {/if}
    <div class="barnote">
      Drawn from her routine and American time-use diaries, the first time each day is asked for, then kept. A day ahead is how it would be drawn now; it can change until it comes.
    </div>
  {/if}
  <button class="abtn" onclick={() => onclose?.()}>Close</button>
</div>

<style>
  .week { display: flex; flex-direction: column; gap: 14px; }
  .wnav { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .wcell { display: flex; align-items: center; gap: 10px; font-size: 12px; color: var(--text-muted); }
  .wcell select { min-height: 36px; padding: 0 8px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font: inherit; font-size: 13px; }
  /* A field under 16px makes iOS zoom the page, and it stays zoomed. */
  @media (pointer: coarse) { .wcell select { font-size: 16px; } }
  .wtitle { font-family: var(--mono); font-size: 12px; color: var(--text-muted); }
  .wrows { display: flex; flex-direction: column; gap: 8px; }
  .wrow { display: grid; grid-template-columns: 64px 1fr; column-gap: 10px; row-gap: 4px; align-items: center; }
  .wday { display: flex; flex-direction: column; font-family: var(--mono); font-size: 12px; color: var(--text); line-height: 1.3; }
  .wrow.today .wday { color: var(--accent-400); }
  .wtag { font-size: 10px; color: var(--accent-400); }
  .wtag.dim { color: var(--text-muted); }
  .wbar { position: relative; height: 28px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); overflow: hidden; }
  .wbar.ahead { opacity: 0.6; }
  .wbar.folded { display: flex; align-items: center; padding: 0 8px; font-size: 11px; color: var(--text-muted); }
  .tick { position: absolute; top: 0; bottom: 0; width: 1px; background: var(--accent-900); pointer-events: none; }
  .seg { position: absolute; top: 2px; bottom: 2px; border: none; padding: 0; border-radius: 3px; cursor: pointer; min-width: 2px; box-shadow: inset -1px 0 0 var(--surface); }
  .seg.sel { outline: 2px solid var(--text); outline-offset: -2px; z-index: 1; }
  .seg.unclean { background-image: repeating-linear-gradient(45deg, transparent 0 4px, var(--hazard) 4px 6px); }
  .wline { grid-column: 2; font-size: 12px; color: var(--text); line-height: 1.4; }
  .wscale { position: relative; height: 14px; font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .wscale span { position: absolute; transform: translateX(-50%); }
  .wscale span:first-child { transform: none; }
  .wlegend { display: flex; flex-wrap: wrap; gap: 10px; }
  .lchip { display: inline-flex; align-items: center; gap: 5px; font-size: 11px; color: var(--text-muted); }
  .swatch { width: 10px; height: 10px; border-radius: 2px; }
  /* One hue a group, mid lightness so each reads on a light and a dark
     surface alike. */
  .rest { background: #7c8db5; }
  .work { background: #c9894a; }
  .home { background: #6fa37a; }
  .move { background: #4f9fb0; }
  .social { background: #b8739d; }
  .leisure { background: #a99a52; }
  .unknown { background: var(--text-muted); }
  .wnote { margin: 0; font-size: 12px; color: var(--text-muted); line-height: 1.45; }
  .linkbtn { background: none; border: none; padding: 4px 0; min-height: 32px; color: var(--accent-400); font-size: 12px; cursor: pointer; }
  .warnline { font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .barnote { font-size: 11px; color: var(--text-muted); line-height: 1.5; }
  .wdraft { display: flex; flex-direction: column; gap: 8px; padding: 10px 12px; border: 1px solid var(--accent-900); border-radius: var(--radius); }
  .wdhead { font-size: 13px; color: var(--text); }
  .wdlist { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--text); line-height: 1.4; }
  .wdtext { width: 100%; box-sizing: border-box; padding: 8px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-family: var(--mono); font-size: 12px; line-height: 1.45; resize: vertical; }
  @media (pointer: coarse) { .wdtext { font-size: 16px; } }
  .wdacts { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; }
  .abtn:disabled, .linkbtn:disabled { opacity: 0.5; cursor: default; }
  .abtn { min-height: 44px; padding: 0 16px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; }
</style>
