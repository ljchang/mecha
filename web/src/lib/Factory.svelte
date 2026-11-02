<script>
  import { apiFetch as fetch } from './api.js';
  import { byDay, nextWindow } from './factory.js';
  // The Factory view: the instruments on the public surface, from home.
  // A booking page's hours are edited here and nowhere a model can reach —
  // the save is one handler in `serve/factory.rs`, with no CLI verb and no
  // tool behind it, because what strangers may book of the owner's week is
  // what a stranger's prose would most like to change. Every number on this
  // page is the factory's own answer: the preview is `slots preview` over
  // the busy time the last push saw, the check is `type check`.
  let { initial = null, navigate = () => {} } = $props();

  let list = $state(null);
  let listError = $state(null);
  let item = $state(null); // the GET /api/factory/instrument body
  let itemError = $state(null);
  let draft = $state(null); // the hours being edited
  // The answer and the draft it answers, kept together: a preview of the
  // previous draft is not a preview of this one, and the confirm sheet
  // quotes it. `current` below is the only reader.
  let draftPreview = $state(null);
  let draftPreviewFor = $state(null);
  let draftError = $state(null);
  let saving = $state(false);
  let confirming = $state(false);
  let toast = $state(null);
  let toastHazard = $state(false);

  // Derived from the route, never copied into state — the trap
  // `Review.svelte` records: App passes a new `initial` on a hash change
  // without remounting.
  const id = $derived(initial);

  async function loadList() {
    try {
      const res = await fetch('/api/factory');
      if (!res.ok) throw new Error((await res.text()).trim());
      list = (await res.json()).instruments;
      listError = null;
    } catch (e) {
      listError = String(e?.message ?? e);
    }
  }
  loadList();

  function adopt(body) {
    item = body;
    draft = body.hours ? structuredClone($state.snapshot(body.hours)) : null;
    draftPreview = null;
    draftPreviewFor = null;
    draftError = null;
  }

  async function loadItem(which) {
    item = null;
    itemError = null;
    draft = null;
    try {
      const res = await fetch(`/api/factory/instrument?id=${encodeURIComponent(which)}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      const body = await res.json();
      if (which === id) adopt(body);
    } catch (e) {
      itemError = String(e?.message ?? e);
    }
  }
  // Leaving an instrument for the list drops its edit: App keeps this
  // component mounted across the hash change, so nothing else would, and a
  // Save bar under the list would offer to save a page that is not shown.
  $effect(() => {
    if (id) {
      loadItem(id);
    } else {
      item = null;
      draft = null;
      confirming = false;
    }
  });

  const draftKey = $derived(draft ? JSON.stringify(draft) : null);
  const dirty = $derived(item?.hours && draft ? draftKey !== JSON.stringify(item.hours) : false);
  // The preview of *this* draft, or nothing yet.
  const current = $derived(draftPreviewFor !== null && draftPreviewFor === draftKey ? draftPreview : null);

  // The preview follows the draft, debounced: a person dragging a time
  // should see what it offers without pressing anything.
  let previewTimer = null;
  // The request in flight, cancelled when the draft moves on: the server
  // kills a read child whose client walked away (`Child::Read`), and only
  // an aborted request walks away. Without this, dragging a time for ten
  // seconds left twenty `type check` + `slots preview` pairs running to
  // completion for answers the page then threw away.
  let previewAbort = null;
  $effect(() => {
    const snapshot = draftKey;
    if (!snapshot || !dirty) {
      draftPreview = null;
      draftPreviewFor = null;
      draftError = null;
      return;
    }
    // A moved draft has no answer until its own arrives.
    draftError = null;
    clearTimeout(previewTimer);
    previewTimer = setTimeout(() => runPreview(snapshot), 500);
    return () => {
      clearTimeout(previewTimer);
      previewAbort?.abort();
      previewAbort = null;
    };
  });

  // An emptied number box binds as null; say so in the page's words rather
  // than showing the server's refusal to parse the body.
  const NUMBERS = { buffer_minutes: 'the gap', min_notice_hours: 'the notice', horizon_days: 'how far ahead' };
  function missingNumber(h) {
    return Object.entries(NUMBERS).find(([k]) => !Number.isFinite(h?.[k]))?.[1] ?? null;
  }

  async function runPreview(snapshot) {
    const missing = missingNumber(JSON.parse(snapshot));
    if (missing) {
      draftError = `Fill in ${missing} before this can be previewed.`;
      return;
    }
    try {
      previewAbort?.abort();
      const abort = new AbortController();
      previewAbort = abort;
      const res = await fetch('/api/factory/preview', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ id, base: item.digest, hours: JSON.parse(snapshot) }),
        signal: abort.signal,
      });
      const text = await res.text();
      // A stale answer for a draft that has moved on is dropped; the
      // newer request's own answer is the one `current` waits for.
      if (snapshot !== draftKey) return;
      if (!res.ok) {
        draftPreview = null;
        draftPreviewFor = null;
        draftError = text.trim();
      } else {
        draftPreview = JSON.parse(text).preview;
        draftPreviewFor = snapshot;
        draftError = null;
      }
    } catch (e) {
      // An abort is the draft moving on, not a failure to report.
      if (e?.name === 'AbortError') return;
      if (snapshot === draftKey) draftError = String(e?.message ?? e);
    }
  }

  async function save() {
    saving = true;
    try {
      const res = await fetch('/api/factory/hours', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ id, base: item.digest, hours: $state.snapshot(draft) }),
      });
      const text = await res.text();
      if (!res.ok) throw new Error(text.trim());
      const out = JSON.parse(text);
      confirming = false;
      // The save stands either way; a failed re-read only means the page
      // asks again rather than showing what came back.
      if (out.instrument) adopt(out.instrument);
      else await loadItem(id);
      if (out.pushed) {
        say('Saved and pushed — the booking page offers the new hours now.');
      } else {
        // The save stands; only the immediate push failed.
        say(`Saved. The push failed (${out.push_error}); the slot timer retries every two minutes.`, true);
      }
    } catch (e) {
      confirming = false;
      say(String(e?.message ?? e), true);
    } finally {
      saving = false;
    }
  }

  function say(text, hazard = false) {
    toast = text;
    toastHazard = hazard;
    setTimeout(() => (toast = null), hazard ? 9000 : 5000);
  }

  function revert() {
    draft = structuredClone($state.snapshot(item.hours));
  }

  // ── editing ──────────────────────────────────────────────────────────
  const DAY_NAMES = { mon: 'Mon', tue: 'Tue', wed: 'Wed', thu: 'Thu', fri: 'Fri', sat: 'Sat', sun: 'Sun' };
  const LENGTHS = [15, 30, 45, 60, 90];
  const lengths = $derived([...new Set([...LENGTHS, ...(item?.hours?.durations ?? [])])].sort((a, b) => a - b));

  const windowsOf = (day) => (draft?.windows ?? []).map((w, i) => [w, i]).filter(([w]) => w.day === day);
  function addWindow(day) {
    const next = nextWindow(windowsOf(day).map(([w]) => w));
    if (next) draft.windows.push({ day, ...next });
  }
  const canAdd = (day) => nextWindow(windowsOf(day).map(([w]) => w)) !== null;
  function toggleLength(n) {
    const has = draft.durations.includes(n);
    draft.durations = has ? draft.durations.filter((d) => d !== n) : [...draft.durations, n].sort((a, b) => a - b);
  }

  let newDate = $state('');
  function addOverride() {
    if (!newDate || draft.overrides.some((o) => o.date === newDate)) return;
    draft.overrides = [...draft.overrides, { date: newDate, windows: [] }].sort((a, b) => a.date.localeCompare(b.date));
    newDate = '';
  }
  const today = new Date().toISOString().slice(0, 10);


  // ── reading the answers ──────────────────────────────────────────────
  const ago = (stamp) => {
    const ms = Date.now() - new Date(stamp).getTime();
    if (isNaN(ms)) return 'at an unknown time';
    const min = Math.round(ms / 60000);
    if (min < 1) return 'just now';
    if (min < 90) return `${min} min ago`;
    const h = Math.round(min / 60);
    return h < 48 ? `${h} h ago` : `${Math.round(h / 24)} days ago`;
  };
  // The timer runs every two minutes (`mecha-slots.timer`); ten minutes is
  // five missed ticks, which is a timer in trouble rather than a late one.
  // A missing stamp is "not pushed yet", never the epoch: `new Date(null)`
  // is 1970, which would read as decades late and blame a healthy timer.
  const pushed = (stamp) => stamp != null && !isNaN(new Date(stamp).getTime());
  const pushIsLate = (stamp) => pushed(stamp) && Date.now() - new Date(stamp).getTime() > 10 * 60000;

  const meeting = (b) => {
    const start = new Date(b.start);
    if (isNaN(start)) return b.start;
    return start.toLocaleString([], { weekday: 'short', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit', timeZoneName: 'short' });
  };

  const shown = $derived(dirty ? current : (item?.preview ?? null));
  const shownDays = $derived(byDay(shown?.slots, shown?.timezone));
  // A start that offers every length needs no label; one that offers only
  // some (the hour before a window closes fits 30 but not 60) says which.
  const offered = $derived((dirty ? draft : item?.hours)?.durations ?? []);
</script>

{#snippet hazardGlyph(size = 13)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

<div class="page">
  <div class="scroll">
    {#if !id}
      <div class="head"><span class="title">Factory</span></div>
      <p class="lede">Your instruments on the public surface — the booking page and the forms strangers fill in. Requests they send are reviewed in <button class="link" onclick={() => navigate('review/frontdoor')}>Review → Front door</button>.</p>
      {#if listError}<div class="warnline">{@render hazardGlyph()}<span>{listError}</span></div>{/if}
      {#if list === null && !listError}
        <div class="empty">reading the manifests…</div>
      {:else if list}
        {#each list as inst (inst.path)}
          <button class="card rowbtn" disabled={!!inst.problem} onclick={() => navigate(`factory/${inst.id}`)}>
            <div class="rowtop">
              <span class="chip">{inst.kind === 'booking' ? 'booking page' : inst.kind === 'request' ? 'form' : inst.kind}</span>
              <span class="when">{inst.id}</span>
            </div>
            <div class="topic">{inst.title || inst.id}</div>
            {#if inst.problem}<div class="claim hazardtext">cannot be read: {inst.problem}</div>{/if}
          </button>
        {:else}
          <div class="empty">No manifests in ~/.mecha/factory/types yet.</div>
        {/each}
      {/if}
    {:else}
      <div class="deckhead">
        <button class="backbtn" onclick={() => navigate('factory')} aria-label="back">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6l-6 6 6 6" /></svg>
        </button>
        <span class="dtitle">{item?.title || id}</span>
      </div>
      {#if itemError}<div class="warnline">{@render hazardGlyph()}<span>{itemError}</span></div>{/if}
      {#if !item && !itemError}
        <div class="empty">reading {id}…</div>
      {:else if item}
        <!-- Status: what the page is serving now, by the factory's answer. -->
        <div class="card status">
          {#if item.kind === 'booking'}
            {#if item.preview}
              <div class="statline">
                <span class="big">{item.preview.slots.length}</span>
                <span>open slots over the next {item.preview.horizon_days} days</span>
              </div>
              <div class="meta" class:hazardtext={pushIsLate(item.preview.last_pushed_at)}>
                <span>{pushed(item.preview.last_pushed_at) ? `last pushed ${ago(item.preview.last_pushed_at)}` : 'not pushed yet'}</span>
                {#if pushIsLate(item.preview.last_pushed_at)}<span>— the slot timer (every two minutes) may be failing</span>{/if}
                {#if item.preview.held_slots}<span>{item.preview.held_slots} slot{item.preview.held_slots === 1 ? '' : 's'} held by open polls</span>{/if}
              </div>
            {:else if item.preview_error}
              <div class="warnline">{@render hazardGlyph()}<span>{item.preview_error}</span></div>
            {/if}
          {/if}
          <div class="meta">
            {#if item.requests_open == null}
              <span class="hazardtext">request count unknown — the front-door store could not be read</span>
            {:else}
              <span>{item.requests_open} request{item.requests_open === 1 ? '' : 's'} waiting</span>
              {#if item.requests_skipped}
                <span class="hazardtext">and {item.requests_skipped} request file{item.requests_skipped === 1 ? '' : 's'} could not be read — the count and the meetings below may be short</span>
              {/if}
            {/if}
            <button class="link" onclick={() => navigate('review/frontdoor')}>open the front door ›</button>
          </div>
        </div>

        {#if item.bookings?.length}
          <div class="section">Coming up</div>
          {#each item.bookings as b (b.seq)}
            <div class="card brow">
              <div class="topic">{meeting(b)}</div>
              <div class="meta">
                {#if b.reply_to}<span>{b.reply_to}</span>{/if}
                {#if b.duration_minutes}<span>{b.duration_minutes} min</span>{/if}
                <span class:hazardtext={!b.on_calendar}>{b.on_calendar ? 'on your calendar' : 'not on your calendar yet'}</span>
              </div>
            </div>
          {/each}
          <div class="note">To cancel one, delete it from your calendar for now — cancelling from here needs a host-cancel path the factory does not have yet.</div>
        {:else if item.bookings_error}
          <div class="warnline">{@render hazardGlyph()}<span>{item.bookings_error}</span></div>
        {/if}

        {#if item.kind !== 'booking'}
          <div class="note">This is a form. Editing its fields from here is not built yet — edit <code>{item.path}</code> and run <code>factory-publish type push</code>.</div>
        {:else if item.hours_error}
          <div class="warnline">{@render hazardGlyph()}<span>{item.hours_error}</span></div>
        {:else if draft}
          <div class="section">Hours <span class="sub">in {draft.timezone}</span></div>
          <div class="card week">
            {#each item.days as day (day)}
              <div class="dayrow">
                <span class="dayname">{DAY_NAMES[day]}</span>
                <div class="spans">
                  {#each windowsOf(day) as [w, i] (i)}
                    <span class="span">
                      <input type="time" step="900" bind:value={draft.windows[i].start} aria-label="{DAY_NAMES[day]} start" />
                      <span class="dash">–</span>
                      <input type="time" step="900" bind:value={draft.windows[i].end} aria-label="{DAY_NAMES[day]} end" />
                      <button class="x" aria-label="remove" onclick={() => draft.windows.splice(i, 1)}>×</button>
                    </span>
                  {:else}
                    <span class="closed">closed</span>
                  {/each}
                  {#if canAdd(day)}<button class="add" aria-label="add hours on {DAY_NAMES[day]}" onclick={() => addWindow(day)}>+</button>{/if}
                </div>
              </div>
            {/each}
          </div>

          <div class="section">Meetings</div>
          <div class="card fields">
            <div class="field">
              <span class="flabel">Lengths offered</span>
              <div class="pills">
                {#each lengths as n (n)}
                  <button class="pill" class:on={draft.durations.includes(n)} onclick={() => toggleLength(n)}>{n} min</button>
                {/each}
              </div>
            </div>
            <label class="field">
              <span class="flabel">Gap around other meetings</span>
              <span class="num"><input type="number" min="0" max="240" step="5" bind:value={draft.buffer_minutes} /> min</span>
            </label>
            <label class="field">
              <span class="flabel">Minimum notice</span>
              <span class="num"><input type="number" min="0" max="720" bind:value={draft.min_notice_hours} /> hours</span>
            </label>
            <label class="field">
              <span class="flabel">How far ahead</span>
              <span class="num"><input type="number" min="1" max={item.pipeline_days} bind:value={draft.horizon_days} /> days (up to {item.pipeline_days})</span>
            </label>
            <label class="field">
              <span class="flabel">Most per day</span>
              <span class="num">
                <input
                  type="number"
                  min="1"
                  max="20"
                  placeholder="no cap"
                  value={draft.per_day_cap ?? ''}
                  oninput={(e) => (draft.per_day_cap = e.currentTarget.value === '' ? null : Number(e.currentTarget.value))}
                />
              </span>
            </label>
            <div class="field">
              <span class="flabel">Time zone</span>
              <!-- Read-only: the booking page itself reads this field to
                   label its week, so changing it is a type push — a
                   publish — and the server refuses it as an hours save. -->
              <span class="num"><code>{draft.timezone}</code></span>
            </div>
            <div class="note">The page labels its times in this zone. Changing it is a <code>factory-publish type push</code> of the manifest, not an hours save.</div>
          </div>

          <div class="section">Days off and exceptions</div>
          <div class="card fields">
            <!-- Keyed by position, not date: a hand-written manifest may
                 repeat a date (the save refuses it), and a duplicate key
                 would throw before the owner could see and fix it. -->
            {#each draft.overrides as o, oi (oi)}
              <div class="dayrow">
                <span class="dayname wide">{o.date}</span>
                <div class="spans">
                  {#each o.windows as pair, pi (pi)}
                    <span class="span">
                      <input type="time" step="900" bind:value={draft.overrides[oi].windows[pi][0]} aria-label="{o.date} start" />
                      <span class="dash">–</span>
                      <input type="time" step="900" bind:value={draft.overrides[oi].windows[pi][1]} aria-label="{o.date} end" />
                      <button class="x" aria-label="remove hours" onclick={() => draft.overrides[oi].windows.splice(pi, 1)}>×</button>
                    </span>
                  {:else}
                    <span class="closed">closed all day</span>
                  {/each}
                  <button class="add" title="open some hours this day" onclick={() => draft.overrides[oi].windows.push(['09:00', '12:00'])}>+</button>
                  <button class="x" aria-label="remove exception" title="back to the weekly hours" onclick={() => draft.overrides.splice(oi, 1)}>remove</button>
                </div>
              </div>
            {:else}
              <div class="note">None — every date follows the weekly hours.</div>
            {/each}
            <div class="addrow">
              <input type="date" min={today} bind:value={newDate} aria-label="date" />
              <button class="abtn" disabled={!newDate} onclick={addOverride}>Add a date</button>
            </div>
          </div>

          <!-- What the page would offer: the saved hours, or the draft's. -->
          <div class="section">
            {dirty ? 'With these changes' : 'What the push offers'}
            <span class="sub">next two weeks, {shown?.timezone ?? draft.timezone}</span>
          </div>
          {#if dirty && draftError}
            <div class="warnline">{@render hazardGlyph()}<span>{draftError}</span></div>
          {:else if dirty && !current}
            <div class="empty">computing…</div>
          {:else if shown}
            <div class="note">Times already booked are removed by the page itself, so a confirmed meeting's slot can show here and not there.</div>
            {#if dirty}
              <div class="meta"><span>{current.slots.length} slots over {current.horizon_days} days (now {item.preview?.slots.length ?? '?'})</span></div>
            {/if}
            <div class="card preview">
              {#each shownDays as d (d.label)}
                <div class="pday">
                  <span class="pdate">{d.label}</span>
                  <span class="pstarts">{#each d.starts as s}<span class="pstart">{s.t}{#if s.ds.length !== offered.length}<sub>{s.ds.join('/')}</sub>{/if}</span>{/each}</span>
                </div>
              {:else}
                <div class="note">Nothing bookable in the next two weeks.</div>
              {/each}
            </div>
          {/if}
        {/if}
      {/if}
    {/if}
  </div>

  {#if id && dirty && draft}
    <div class="actionbar">
      <button class="abtn" disabled={saving} onclick={revert}>Revert</button>
      <button class="abtn primary" disabled={saving || !current || !!draftError} onclick={() => (confirming = true)}>Save hours…</button>
    </div>
  {/if}

  <!-- `current`, never the last answer: the number confirmed is the number
       of the hours about to be written. An edit while the sheet is open
       closes it until the new answer lands. -->
  {#if id && confirming && current}
    <div class="sheet">
      <div class="sheet-grip"></div>
      <div class="sheet-text">Strangers will be able to book {current.slots.length} slot{current.slots.length === 1 ? '' : 's'} over the next {current.horizon_days} days (now {item.preview?.slots.length ?? 'unknown'}).</div>
      <div class="note">Saving writes <code>{item.path}</code> and pushes the slots to the booking page now. Meetings already booked are not touched.</div>
      <div class="btnrow">
        <button class="abtn" disabled={saving} onclick={() => (confirming = false)}>Back</button>
        <button class="abtn primary" disabled={saving} onclick={save}>{saving ? 'Saving…' : 'Save and push'}</button>
      </div>
    </div>
  {/if}

  {#if toast}<div class="toast" class:hazardtoast={toastHazard}>{toast}</div>{/if}
</div>

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  .scroll { flex: 1; overflow-y: auto; padding: 14px var(--gutter) 24px; display: flex; flex-direction: column; gap: 10px; }
  .scroll > * { flex-shrink: 0; }
  .head { display: flex; align-items: center; min-height: 32px; padding-right: var(--gutter-gear); }
  .title { font-family: var(--mono); font-size: 13px; color: var(--accent-400); }
  .lede { font-size: 13px; line-height: 1.5; color: var(--text-muted); margin: 0; }
  .link { background: none; border: none; padding: 0; color: var(--accent-400); font: inherit; cursor: pointer; text-decoration: underline; text-underline-offset: 2px; }
  .rowbtn { text-align: left; padding: 14px; display: flex; flex-direction: column; gap: 6px; cursor: pointer; color: var(--text); font: inherit; overflow: hidden; }
  .rowbtn:disabled { cursor: default; opacity: 0.8; }
  .rowtop { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .when { font-family: var(--mono); font-size: 10px; color: var(--accent-700); margin-left: auto; }
  .topic { font-size: 14px; font-weight: 500; line-height: 1.35; overflow-wrap: anywhere; }
  .claim { font-family: var(--mono); font-size: 10px; }
  .hazardtext { color: var(--hazard); }
  .meta { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 10px; font-family: var(--mono); font-size: 10px; color: var(--text-muted); overflow-wrap: anywhere; }
  .empty { color: var(--text-muted); font-size: 14px; padding: 24px 0; text-align: center; }
  .note { font-size: 11px; line-height: 1.45; color: var(--text-muted); }
  .warnline { display: flex; align-items: flex-start; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; overflow-wrap: anywhere; }
  .deckhead { display: flex; align-items: center; gap: 8px; padding-right: var(--gutter-gear); }
  .backbtn { background: none; border: none; color: var(--text-muted); min-width: 44px; min-height: 44px; margin: -10px 0 -10px -12px; cursor: pointer; display: flex; align-items: center; justify-content: center; }
  .dtitle { font-family: var(--mono); font-size: 13px; color: var(--accent-400); }
  .section { font-family: var(--mono); font-size: 11px; letter-spacing: 0.04em; text-transform: uppercase; color: var(--accent-500); margin-top: 8px; }
  .section .sub { text-transform: none; letter-spacing: 0; color: var(--text-muted); margin-left: 6px; }
  .status { padding: 14px; display: flex; flex-direction: column; gap: 8px; }
  .statline { display: flex; align-items: baseline; gap: 8px; font-size: 13px; }
  .big { font-family: var(--mono); font-size: 22px; color: var(--accent-400); }
  .brow { padding: 12px 14px; display: flex; flex-direction: column; gap: 4px; }
  .week, .fields { padding: 6px 14px; display: flex; flex-direction: column; }
  .dayrow { display: flex; align-items: center; gap: 10px; min-height: 48px; border-bottom: 1px solid var(--accent-900); }
  .dayrow:last-child { border-bottom: none; }
  .dayname { font-family: var(--mono); font-size: 12px; width: 34px; flex-shrink: 0; color: var(--text); }
  .dayname.wide { width: 86px; }
  .spans { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; flex: 1; min-width: 0; padding: 6px 0; }
  .span { display: inline-flex; align-items: center; gap: 2px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); padding: 2px 2px 2px 6px; }
  /* Wide enough for a 12-hour clock's AM/PM: at 5.6em a 1 PM start read
     as "01:00", which is the wrong hour on the one page about hours. */
  .span input { background: none; border: none; color: var(--text); font-family: var(--mono); font-size: 12px; padding: 4px 0; width: 8.2em; color-scheme: dark; }
  .dash { color: var(--text-muted); }
  .closed { font-size: 12px; color: var(--text-muted); font-style: italic; }
  .x, .add { background: none; border: none; color: var(--text-muted); font: inherit; font-size: 13px; min-width: 32px; min-height: 32px; cursor: pointer; }
  .add { color: var(--accent-400); font-size: 18px; }
  .field { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-height: 48px; border-bottom: 1px solid var(--accent-900); flex-wrap: wrap; }
  .field:last-child { border-bottom: none; }
  .flabel { font-size: 13px; }
  .num { font-size: 12px; color: var(--text-muted); display: inline-flex; align-items: center; gap: 6px; }
  .num input, .addrow input { background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-family: var(--mono); font-size: 13px; padding: 8px 10px; color-scheme: dark; }
  .num input { width: 5em; }
  .pills { display: flex; flex-wrap: wrap; gap: 6px; }
  .pill { min-height: 36px; padding: 0 12px; border-radius: var(--radius-chip); border: 1px solid var(--accent-900); background: none; color: var(--text-muted); font-family: var(--mono); font-size: 12px; cursor: pointer; }
  .pill.on { background: var(--accent-900); color: var(--text); border-color: var(--accent-500); }
  .addrow { display: flex; gap: 8px; align-items: center; padding: 10px 0; flex-wrap: wrap; }
  .preview { padding: 6px 14px; display: flex; flex-direction: column; }
  .pday { display: flex; gap: 10px; padding: 8px 0; border-bottom: 1px solid var(--accent-900); align-items: baseline; }
  .pday:last-child { border-bottom: none; }
  .pdate { font-family: var(--mono); font-size: 11px; width: 86px; flex-shrink: 0; color: var(--text); }
  .pstarts { display: flex; flex-wrap: wrap; gap: 4px 10px; font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  .pstart sub { font-size: 8px; margin-left: 1px; color: var(--accent-700); }
  .actionbar { display: flex; gap: 8px; justify-content: flex-end; padding: 10px var(--gutter); border-top: 1px solid var(--accent-900); background: var(--bg); }
  .abtn { flex-shrink: 0; min-height: 44px; padding: 0 16px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; white-space: nowrap; }
  .abtn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .abtn:disabled { opacity: 0.5; }
  .btnrow { display: flex; gap: 10px; }
  .btnrow .abtn { flex: 1; }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px var(--gutter) 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; line-height: 1.4; }
  .toast { position: absolute; bottom: 18px; left: 50%; transform: translateX(-50%); background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 10px 16px; font-size: 13px; max-width: 90%; z-index: 7; text-align: center; }
  .toast.hazardtoast { border-color: var(--hazard); color: var(--hazard); }
  code { font-family: var(--mono); font-size: 0.92em; overflow-wrap: anywhere; }
</style>
