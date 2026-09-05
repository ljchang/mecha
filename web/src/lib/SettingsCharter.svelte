<script>
  import { tick } from 'svelte';
  import {
    readingStands,
    rows,
    sensorProblems,
    sensorsWouldDrop,
    serialize as toToml,
    slugify,
    splitHeader,
  } from './charter-toml.js';

  // The charter pane. The lines are edited in place — tap one to open it,
  // drag its grip to re-rank — and nothing reaches disk until an explicit
  // two-tap save, because this document rides in every run's prompt.
  //
  // **Dragging is not a convenience here, it is the only rank control there
  // can be.** `CharterLine` denies unknown fields and there is deliberately
  // no `priority`/`rank` key (GOAL-SYSTEM-DESIGN §11: a second statement of
  // priority disagrees with the first the moment either is edited), so
  // position in the file *is* the ranking, and moving a line is — in the
  // design's own words — "the only editing gesture that cannot produce a
  // tie".
  //
  // The invariant this does not touch: the owner authors every line. No
  // model composes, suggests or edits one, at any privilege level. A form is
  // the owner typing, which is why it is allowed to exist at all.
  let charter = $state(null);
  let charterError = $state(null);

  // The structured draft. `uid` is a render key only — never written; the
  // `id` is what a GoalRef::Charter points at.
  let uidSeq = 0;
  let lines = $state([]);
  let original = $state('[]');
  // Everything above the first `[[line]]`, kept byte-for-byte across a save:
  // the file's header comments (and, for a first charter, the whole template)
  // are the owner's writing too.
  let header = $state('');
  // Why the structured editor is unavailable, when it is. Never edit blind:
  // regenerating tables out of a document we could not fully account for
  // would silently drop whatever we failed to understand.
  let blocked = $state(null);

  let editing = $state(null); // uid of the open line
  let deleteArmed = $state(null); // uid a first delete tap armed
  let confirming = $state(false);
  let saveError = $state(null);
  let savedNote = $state(null);
  let busy = $state(false);

  // The raw TOML editor, kept as the escape hatch: null when closed.
  let draft = $state(null);

  async function load() {
    try {
      const res = await fetch('/api/settings/charter');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      charter = await res.json();
      charterError = charter.error ?? null;
      hydrate(charter);
    } catch (e) {
      charterError = String(e?.message ?? e);
    }
  }
  load();

  function hydrate(c) {
    // A document that does not parse has no trustworthy `lines` — offering an
    // empty list here would let a save quietly replace a file we could not
    // read. The TOML editor is the only honest surface for that.
    if (c.parse_error) {
      blocked = `The charter on disk does not load, so its lines cannot be edited as a list: ${c.parse_error}`;
      header = '';
      lines = [];
      original = '[]';
      return;
    }
    const raw = (c.raw ?? '').trim() ? c.raw : (c.template ?? '');
    const split = splitHeader(raw);
    header = split.header;
    blocked = split.blocked;
    // `sensor` is carried through a re-rank exactly as it was read, and the
    // form under an open line may change it — the owner typing, which is
    // the author rule's whole condition (see `serialize`, `addSensor`).
    // `reading` rides beside it for display only — see `rows`.
    lines = rows(c.lines, () => ++uidSeq);
    original = snapshot();
  }

  /// Split the document at its first `[[line]]`. A `#` that opens a *line* is
  /// a comment; one inside a string value never can be, since the value is on
  /// the right of an `=`.
  // We have no trustworthy document: the GET failed, or answered with its own
  // error, or reported a parse failure over bytes it never managed to read.
  // Nothing that can write may render — not the list (whose empty state
  // invites authoring a "first" charter over a real one) and not the TOML
  // editor (which would seed from an empty buffer).
  const unreadable = $derived(
    charterError !== null ||
      charter === null ||
      // `charter_state` reads the file with `unwrap_or_default()`, so an I/O
      // failure arrives as `parse_error` with `raw: ""` and no template.
      (charter.parse_error != null && !charter.raw)
  );

  const serialize = () => toToml(header, lines);

  const snapshot = () =>
    JSON.stringify(lines.map((l) => [l.id, l.text, l.sensor?.kind ?? null, l.sensor?.setpoint ?? null]));

  // Reads `lines`, writes neither of the values it sets, so it cannot
  // re-trigger itself. `save()` arms `confirming` without touching `lines`,
  // so arming survives until the document actually changes.
  $effect(() => {
    snapshot();
    confirming = false;
    savedNote = null;
  });

  // The same rule for the raw editor: arming describes the document that was
  // on screen, and editing the text makes it a different one.
  $effect(() => {
    draft;
    confirming = false;
  });
  const dirty = $derived(snapshot() !== original);

  // The id is a pointer, not a label: `GoalRef::Charter` carries an id and no
  // rank. So it is derived once, when a line is created and still has none,
  // and never re-derived from edited text — re-slugging on every keystroke
  // would break every recorded reference to that priority.
  function fillId(line) {
    if (line.id.trim() || !line.text.trim()) return;
    let base = slugify(line.text) || 'priority';
    let candidate = base;
    let n = 2;
    while (lines.some((l) => l !== line && l.id.trim() === candidate)) candidate = `${base}-${n++}`;
    line.id = candidate;
  }

  // Client-side checks are for immediate feedback only; the server validates
  // with the same reader every run loads through and refuses on its own.
  const problems = $derived.by(() => {
    const out = [];
    const seen = new Map();
    for (const [i, l] of lines.entries()) {
      if (!l.text.trim()) out.push(`Line ${i + 1} has no text.`);
      const id = l.id.trim();
      if (!id) out.push(`Line ${i + 1} has no id.`);
      else if (seen.has(id)) out.push(`Two lines share the id “${id}”.`);
      else seen.set(id, i);
    }
    out.push(...sensorProblems(lines));
    return out;
  });

  // The closed set the server offers, with each unit's hint. Absent on an
  // older server, in which case the form stands down and says so — the
  // page never carries its own copy of the list, so a kind the binary does
  // not know cannot be offered here.
  const sensorKinds = $derived(charter?.sensor_kinds ?? null);
  const kindInfo = (kind) => sensorKinds?.find((k) => k.kind === kind) ?? null;

  // A sensor is the owner's, typed here exactly as it would be typed in the
  // TOML: the page fills in neither the kind nor the setpoint, and the hint
  // beside the field is the parser's own sentence for the unit, not a value.
  function addSensor(line) {
    line.sensor = { kind: '', setpoint: '' };
  }
  function removeSensor(line) {
    line.sensor = null;
    line.reading = null;
    line.read_for = null;
  }

  const budget = $derived(charter?.budget ?? 2000);

  // Derive the id when the row *closes*, whatever closed it. Hanging this off
  // the textarea's `blur` alone is a bet on the browser: `dragStart` removes
  // the textarea by setting `editing = null`, and whether `blur` fires for a
  // focused node that is removed is not consistent across engines. With a
  // mouse the grip takes focus first and it works either way; a touch drag
  // moves no focus at all, and this surface is a phone first. The `onblur`
  // stays for tab-away; `fillId` only fills an empty id, so both firing is
  // harmless.
  let lastEditing = null;
  $effect(() => {
    const open = editing;
    if (lastEditing !== null && lastEditing !== open) {
      const line = lines.find((l) => l.uid === lastEditing);
      if (line) fillId(line);
    }
    lastEditing = open;
  });

  function addLine() {
    const line = { uid: ++uidSeq, id: '', text: '', sensor: null, reading: null, read_for: null };
    lines = [...lines, line];
    editing = line.uid;
    savedNote = null;
  }

  function removeLine(uid) {
    if (deleteArmed !== uid) {
      deleteArmed = uid;
      return;
    }
    deleteArmed = null;
    lines = lines.filter((l) => l.uid !== uid);
    if (editing === uid) editing = null;
  }

  function revert() {
    confirming = false;
    saveError = null;
    editing = null;
    deleteArmed = null;
    hydrate(charter);
  }

  async function save(raw) {
    if (!confirming) {
      confirming = true;
      return;
    }
    confirming = false;
    busy = true;
    try {
      const res = await fetch('/api/settings/charter', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ raw }),
      });
      if (!res.ok) {
        // 422 carries the parse error — nothing typed is discarded, which is
        // the whole point of refusing server-side.
        saveError = (await res.text()).trim();
        return;
      }
      charter = await res.json();
      charterError = charter.error ?? null;
      draft = null;
      saveError = null;
      editing = null;
      hydrate(charter);
      // `hydrate` reassigns `lines`, which dirties the effect that clears
      // `savedNote` — it flushes after this block, so setting the note first
      // would have it wiped before it ever rendered.
      await tick();
      savedNote =
        'saved — rides in the prompt of new sessions; this page cannot rebuild ones already running';
    } catch (e) {
      saveError = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  // ── Re-ranking by drag ─────────────────────────────────────────────────
  // Pointer events, not HTML5 drag-and-drop: this surface is a phone first,
  // and dragstart/dragover never fire for touch.
  let listEl;
  let dragUid = $state(null);
  let dragDy = $state(0);
  let dragEl = null;
  let grip = null;
  let pointerY = 0;
  // Where in the row the drag started, measured from its centre. Without it
  // the row snaps its centre to the pointer and jumps half its height.
  let grabOffset = 0;
  let raf = null;

  $effect(() => () => {
    if (raf !== null) cancelAnimationFrame(raf);
  });

  function dragStart(e, uid) {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    editing = null; // a row changing height mid-drag makes the maths lie
    deleteArmed = null;
    dragUid = uid;
    dragDy = 0;
    dragEl = e.currentTarget.closest('.line');
    const box = dragEl.getBoundingClientRect();
    grabOffset = e.clientY - (box.top + box.height / 2);
    // Capture on the grip, which is where the move/up handlers are: a
    // captured event is retargeted to the capture element, and an event
    // dispatched at `.line` would never reach a descendant.
    grip = e.currentTarget;
    grip.setPointerCapture(e.pointerId);
    pointerY = e.clientY;
    e.preventDefault();
    raf = requestAnimationFrame(edgeScroll);
  }

  function dragMove(e) {
    if (dragUid === null) return;
    pointerY = e.clientY;
    reposition();
  }

  function dragEnd(e) {
    if (dragUid === null) return;
    try {
      grip?.releasePointerCapture(e.pointerId);
    } catch {}
    cancelAnimationFrame(raf);
    raf = null;
    dragUid = null;
    dragDy = 0;
    dragEl = null;
    grip = null;
  }

  /// Follow the pointer, and swap past a neighbour once the dragged row's
  /// centre crosses that neighbour's centre.
  ///
  /// The offset is recomputed from the element's *live* rect every call
  /// (rect centre minus the transform already applied = where it rests), so
  /// a swap — which moves the node and changes where it rests — corrects
  /// itself on the next pass instead of needing the layout to be tracked.
  async function reposition(depth = 0) {
    if (dragUid === null || !dragEl || depth > 24) return;
    const rect = dragEl.getBoundingClientRect();
    const rest = rect.top + rect.height / 2 - dragDy;
    dragDy = pointerY - grabOffset - rest;
    const centre = rest + dragDy;

    const i = lines.findIndex((l) => l.uid === dragUid);
    const rows = [...listEl.querySelectorAll('.line')];
    let to = null;
    if (i > 0) {
      const r = rows[i - 1].getBoundingClientRect();
      if (centre < r.top + r.height / 2) to = i - 1;
    }
    if (to === null && i < lines.length - 1) {
      const r = rows[i + 1].getBoundingClientRect();
      if (centre > r.top + r.height / 2) to = i + 1;
    }
    if (to === null) return;
    const next = [...lines];
    next.splice(to, 0, next.splice(i, 1)[0]);
    lines = next;
    await tick();
    await reposition(depth + 1); // a fast drag can cross several rows at once
  }

  /// A charter longer than the screen has to be draggable past its own edge.
  function edgeScroll() {
    if (dragUid === null) return;
    const scroller = dragEl?.closest('main');
    if (scroller) {
      const r = scroller.getBoundingClientRect();
      const EDGE = 64;
      const before = scroller.scrollTop;
      if (pointerY < r.top + EDGE) scroller.scrollTop -= 10;
      else if (pointerY > r.bottom - EDGE) scroller.scrollTop += 10;
      if (scroller.scrollTop !== before) reposition();
    }
    raf = requestAnimationFrame(edgeScroll);
  }

  /// Dragging is the gesture; the keyboard still needs a way through, and it
  /// costs no visible control.
  function gripKey(e, uid) {
    const i = lines.findIndex((l) => l.uid === uid);
    const to = e.key === 'ArrowUp' ? i - 1 : e.key === 'ArrowDown' ? i + 1 : null;
    if (to === null || to < 0 || to >= lines.length) return;
    e.preventDefault();
    const next = [...lines];
    next.splice(to, 0, next.splice(i, 1)[0]);
    lines = next;
    tick().then(() => listEl?.querySelectorAll('.grip')[to]?.focus());
  }
</script>

<p class="hint">
  Standing priorities every run carries, ranked highest first — order is rank: when two conflict,
  the higher line wins outright. Tap a line to edit it, drag its grip to re-rank.
</p>

{#if charterError}
  <div class="card notice">{charterError}</div>
{/if}

{#if blocked}
  <div class="card notice">{blocked}</div>
{/if}

{#if draft !== null}
  <!-- The escape hatch: the whole document, exactly as it sits on disk. -->
  <textarea class="editor" bind:value={draft} spellcheck="false" rows="18"></textarea>
  {#if saveError}<div class="card notice">not saved: {saveError}</div>{/if}
  <div class="row-actions">
    <button class="btn primary" class:confirm={confirming} disabled={busy} onclick={() => save(draft)}>
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
{:else if charter === null && !charterError}
  <div class="card"><div class="sub">loading…</div></div>
{:else if unreadable}
  <!-- The notice above says what failed. Nothing here writes: an empty list
       whose save replaces the file is the failure mode this branch exists to
       refuse. -->
  <div class="card">
    <div class="sub">
      The charter could not be read, so it cannot be edited here — what is on disk is
      untouched.
    </div>
  </div>
  <div class="row-actions">
    <button class="btn" onclick={load}>Try again</button>
  </div>
{:else}
  {#if !blocked}
  <div class="lines" bind:this={listEl}>
    {#each lines as line, i (line.uid)}
      <div
        class="line"
        class:dragging={dragUid === line.uid}
        class:open={editing === line.uid}
        style:transform={dragUid === line.uid ? `translateY(${dragDy}px)` : null}
      >
        <div class="linetop">
          <button
            class="grip"
            aria-label={`re-rank ${line.id || 'this line'} — drag, or arrow keys`}
            title="drag to re-rank"
            onpointerdown={(e) => dragStart(e, line.uid)}
            onpointermove={dragMove}
            onpointerup={dragEnd}
            onpointercancel={dragEnd}
            onkeydown={(e) => gripKey(e, line.uid)}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" aria-hidden="true">
              <circle cx="9" cy="6" r="1.5" /><circle cx="15" cy="6" r="1.5" />
              <circle cx="9" cy="12" r="1.5" /><circle cx="15" cy="12" r="1.5" />
              <circle cx="9" cy="18" r="1.5" /><circle cx="15" cy="18" r="1.5" />
            </svg>
          </button>
          <span class="rank">{i + 1}</span>
          {#if editing === line.uid}
            <input
              class="idfield"
              bind:value={line.id}
              spellcheck="false"
              autocapitalize="off"
              placeholder="id"
              aria-label="id — what a goal reference points at"
            />
          {:else}
            <button class="idbtn" onclick={() => (editing = line.uid)}>{line.id || 'no id yet'}</button>
          {/if}
        </div>

        {#if editing === line.uid}
          <textarea
            class="textfield"
            bind:value={line.text}
            onblur={() => fillId(line)}
            rows="4"
            placeholder="What this priority actually asks for."
            aria-label="the priority, in your own words"
          ></textarea>
          {#if line.sensor}
            <!-- The sensor form. Author rule, not verb rule: the owner types
                 the kind and the setpoint, and the page proposes neither —
                 the select opens on "choose a kind", the setpoint field is
                 empty, and the hint under it is the parser's own unit
                 sentence. The server validates with the same reader every
                 run loads through, and a refused save keeps the draft. -->
            <div class="sensorform">
              {#if sensorKinds}
                <select class="idfield" bind:value={line.sensor.kind} aria-label="what the sensor watches">
                  <option value="">choose a kind</option>
                  {#each sensorKinds as k (k.kind)}
                    <option value={k.kind}>{k.kind} — {k.describe}</option>
                  {/each}
                </select>
                <input
                  class="idfield"
                  bind:value={line.sensor.setpoint}
                  spellcheck="false"
                  autocapitalize="off"
                  placeholder="setpoint"
                  aria-label="the setpoint, in the kind's unit"
                />
                <div class="sub hint">
                  {#if kindInfo(line.sensor.kind)}
                    setpoint: {kindInfo(line.sensor.kind).hint} — what the line means by "short" or "few"
                  {:else}
                    pick what the line watches; the setpoint's unit follows from it
                  {/if}
                </div>
                <!-- Containment 5's first guard, where the value is typed:
                     the reading that still stands for this exact sensor. It
                     goes quiet the moment the kind or setpoint changes
                     (`readingStands`), and comes back with the save. -->
                {#if readingStands(line)}
                  <div class="sub hint">
                    reading now: <span class:over={line.reading.state === 'observed' && line.reading.over}>{line.reading.summary}</span>
                  </div>
                {:else if line.reading}
                  <div class="sub hint">reading: not yet, for this setpoint — save to read it</div>
                {/if}
                <button class="btn small" onclick={() => removeSensor(line)}>Remove sensor</button>
              {:else}
                <!-- An older server serves no kinds: the form declines to
                     compose, so it must not delete blind either. The sensor
                     is shown as it is, and the TOML editor is the way to
                     change it. -->
                <div class="sub hint">
                  sensor · {line.sensor.kind || 'no kind'} · setpoint {line.sensor.setpoint || '—'} — this
                  server offers no sensor kinds here; edit the sensor as TOML
                </div>
              {/if}
            </div>
          {/if}
          <div class="row-actions">
            <button class="btn small" onclick={() => (editing = null)}>Done</button>
            {#if !line.sensor && sensorKinds}
              <button class="btn small" onclick={() => addSensor(line)}>+ Add sensor</button>
            {/if}
            <button
              class="btn small danger"
              class:armed={deleteArmed === line.uid}
              onclick={() => removeLine(line.uid)}
            >
              {deleteArmed === line.uid ? 'sure?' : 'Delete'}
            </button>
          </div>
        {:else}
          <button class="textbtn" onclick={() => (editing = line.uid)}>
            {line.text || 'Empty — tap to write it.'}
          </button>
        {/if}
        {#if line.sensor && editing !== line.uid}
          <!-- The owner's own setpoint, in their spelling, kept across a
               save by `serialize`; tap the line to change it. The current
               reading beside it is §11.1 containment 5's first guard: a
               setpoint in the wrong unit shows as always past it, here,
               where the owner is editing. -->
          <div class="sensor" title="an observable mecha reads from its own stores; runs that touch what it watches are attributed to this line — the reading never enters a prompt">
            sensor · {line.sensor.kind || 'no kind yet'} · setpoint {line.sensor.setpoint || '—'}
            {#if readingStands(line)}
              · <span class:over={line.reading.state === 'observed' && line.reading.over}>reading {line.reading.summary}</span>
            {:else if line.reading}
              · reading: not yet, for this setpoint — save to read it
            {/if}
          </div>
        {/if}
      </div>
    {/each}
  </div>

  {#if !lines.length}
    <div class="card">
      <div class="sub">
        No charter yet — nothing rides in any prompt. Add the first priority; the file's comments
        are kept as they are.
      </div>
    </div>
  {/if}

  <div class="row-actions">
    <button class="btn" onclick={addLine}>+ Add priority</button>
  </div>

  {#each problems as p}
    <div class="sub notice">{p}</div>
  {/each}

    {#if saveError}<div class="card notice">not saved: {saveError}</div>{/if}
  {/if}

  <!-- Outside `!blocked`: a raw-TOML save whose result carries comments among
       its lines lands fine, and the owner still has to be told it landed. -->
  {#if savedNote}<div class="card ok-note">{savedNote}</div>{/if}

  <!-- Outside the `!blocked` branch on purpose: a charter that does not parse
       is exactly the one that needs an editor, and the notice above promises
       it opens as TOML. -->
  <div class="footer">
    {#if charter?.char_count != null}
      <!-- The server's number, not the file's length. `char_count` is
           `prompt_block(..).chars().count()` — what actually rides in the
           cached prefix — so it counts the rendered header and per-line
           formatting and *not* the file's comments, which never reach a
           prompt. Measuring `serialize().length` against the same budget
           compared two different quantities. -->
      <span class="count" class:over={charter.over_budget}>
        {charter.char_count.toLocaleString()} / {budget.toLocaleString()} characters{dirty
          ? ' (on disk)'
          : ''}
      </span>
    {:else}
      <span class="count">&nbsp;</span>
    {/if}
    <!-- The handoff serialises the list, and `serialize` writes no table
         for a sensor without a kind — so a half-filled sensor would be
         dropped from the draft at the moment the notice naming it
         disappears, and the raw editor's save has no problems gate. The
         same gate as the list save, then: fix the line first (found on
         review). -->
    <button
      class="btn"
      class:ghost={!blocked}
      disabled={!blocked && dirty && sensorsWouldDrop(lines).length > 0}
      onclick={() => {
        // Carry unsaved list edits across rather than silently reverting to
        // what is on disk.
        draft = !blocked && dirty ? serialize() : charter?.raw || charter?.template || '';
        confirming = false;
        saveError = null;
      }}>Edit as TOML</button
    >
    {#if !blocked && dirty && sensorsWouldDrop(lines).length > 0}
      <!-- Said here, not in a title: a disabled button swallows its title in
           every engine. Only a sensor `serialize` would drop gates the
           hatch — a kindless one; an empty id, text or setpoint and two
           lines of one kind serialise faithfully and the server refuses
           them with the draft kept, so those keep the hatch open. -->
      <span class="sub hint">
        line{sensorsWouldDrop(lines).length === 1 ? '' : 's'} {sensorsWouldDrop(lines).join(', ')}: give the sensor a kind or remove it — the TOML draft would drop it
      </span>
    {/if}
  </div>

  {#if charter?.over_budget && !dirty}
    <div class="card notice">
      Over the {budget} budget. It still rides in full, but costs more of the cached prefix than
      argued.
    </div>
  {/if}

  {#if dirty && !blocked}
    <div class="row-actions sticky">
      <button
        class="btn primary"
        class:confirm={confirming}
        disabled={busy || problems.length > 0}
        onclick={() => save(serialize())}
      >
        {confirming ? 'This rides in every run’s prompt — confirm save' : 'Save'}
      </button>
      <button class="btn" onclick={revert}>Cancel</button>
    </div>
  {/if}
{/if}

<style>
  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .card {
    padding: 10px 12px;
  }
  .notice {
    color: var(--hazard);
    font-size: 12.5px;
    white-space: pre-wrap;
  }
  .ok-note {
    color: var(--accent-300);
    font-size: 12.5px;
  }
  .lines {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .line {
    background: var(--surface);
    border: 1px solid transparent;
    border-radius: var(--radius);
    padding: 8px 10px 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .line.open {
    border-color: var(--accent-700);
  }
  .line.dragging {
    position: relative;
    /* Above the shell's gear (3): the line being dragged is the thing under
       the finger, and it reaches the top of the list. */
    z-index: 4;
    border-color: var(--accent-400);
    box-shadow: 0 8px 20px rgba(0, 0, 0, 0.45);
  }
  .linetop {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .grip {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    min-height: 32px;
    margin: -4px 0 -4px -4px;
    padding: 0;
    background: none;
    border: none;
    color: var(--accent-700);
    cursor: grab;
    /* Or the browser scrolls the page instead of moving the line. */
    touch-action: none;
  }
  .grip:hover,
  .grip:focus-visible {
    color: var(--accent-400);
  }
  .line.dragging .grip {
    cursor: grabbing;
  }
  .rank {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--accent-500);
    min-width: 12px;
  }
  .idbtn,
  .textbtn {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    text-align: left;
    cursor: text;
    color: var(--text);
  }
  .sensor {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--muted, #8a8f98);
    margin-top: 4px;
  }
  .sensor .over {
    color: var(--warn, #e0a458);
  }
  .sensorform {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 6px 8px;
    margin: 6px 0 4px 36px;
    align-items: center;
  }
  .sensorform .hint,
  .sensorform .btn {
    grid-column: 1 / -1;
    justify-self: start;
  }
  .sensorform .hint {
    font-size: 11.5px;
  }
  .sensorform .over {
    color: var(--warn, #e0a458);
  }
  .idbtn {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--accent-400);
  }
  .textbtn {
    font-size: 13.5px;
    line-height: 1.45;
    padding-left: 36px;
  }
  .idfield,
  .textfield {
    width: 100%;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius-chip);
    font: inherit;
    padding: 6px 8px;
    min-width: 0;
  }
  .idfield {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--accent-400);
    min-height: 32px;
  }
  .textfield {
    font-size: 13.5px;
    line-height: 1.45;
    resize: vertical;
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
    align-items: center;
  }
  .row-actions.sticky {
    position: sticky;
    bottom: 0;
    padding: 10px 0;
    background: linear-gradient(to top, var(--void) 62%, transparent);
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
  .btn.primary.confirm {
    background: var(--accent-900);
  }
  .btn.danger,
  .btn.danger.armed {
    border-color: var(--hazard);
    color: var(--hazard);
  }
  .btn.ghost {
    border-color: transparent;
    background: none;
    color: var(--text-muted);
    padding: 7px 4px;
    min-height: 32px;
  }
  .footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .count {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-muted);
  }
  .count.over {
    color: var(--hazard);
  }
  .sub {
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.4;
  }
  .sub.notice {
    color: var(--hazard);
  }
</style>
