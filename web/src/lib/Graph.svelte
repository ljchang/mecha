<script>
  import { tick } from 'svelte';
  import Dictate from './Dictate.svelte';
  // The graph tab: one surface over one store (NOTES-GRAPH-DESIGN.md).
  // The old notes and graph tabs were two disjoint halves of this — capture
  // with no navigation on one, a single-entity lookup with no search or
  // write on the other — over the same SQLite file. This merges them along
  // the intent seam instead of the widget seam:
  //
  //   find    — one field, `kg search` (hybrid BM25+vector, fused
  //             graph-side); a hit that names an entity opens it.
  //   entity  — the `kg_entity` envelope with the halves the old page
  //             dropped (aliases, source coverage), plus the neighborhood
  //             (`kg related`, the one graph rendering the evidence
  //             supports: 1–2 hops around the node being read, never a
  //             global view) and history (`kg timeline`, superseded facts
  //             beside what replaced them). Unreviewed (◌) facts keep
  //             Confirm/Refute — opening an entity is review-on-use.
  //   capture — the owner's own words into the graph, zero decisions; the
  //             confirmation line is the CLI's own ("noted (episode N, M
  //             entities linked)"), because capture that visibly feeds the
  //             graph is what makes the habit stick.
  //   recent  — the notebook, newest first, edit in place.
  //
  // ⌘K / Ctrl-K focuses the find field. Scoped to this tab (D3) by
  // construction: the listener unmounts with the component.
  let { initial = null } = $props();

  // ---- find ----
  let query = $state(initial ? decodeURIComponent(initial) : '');
  let results = $state(null); // kg_search items
  let hitEntities = $state([]); // entity names the search surfaced
  let searching = $state(false);
  let findField = $state(null); // the input element, for ⌘K
  let error = $state(null);

  // ---- entity ----
  let entity = $state(null); // the kg_entity envelope
  let related = $state(null); // kg_related items, fetched beside the entity
  let timeline = $state(null); // kg_timeline, fetched when history opens
  let historyOpen = $state(false);
  let busy = $state(false);
  let reasons = $state({}); // fact uid → typed refute reason
  let notes_ = $state({}); // fact uid → verdict error
  let said = $state(null); // one-line confirmation of the last verdict

  // ---- fact authoring ----
  // The owner states a fact; it lands live, never in the review queue —
  // relayed through `mecha kg assert`, whose refusal (an unresolvable
  // subject, say) comes back as the error text here. A connection is a
  // fact whose object is a node, so this one form is also "connect these".
  let addingFact = $state(false);
  let factPredicate = $state('');
  let factObject = $state('');
  let factLiteral = $state(false); // object is a literal value, not a node
  let factBusy = $state(false);
  let openFact = $state(null); // uid of the expanded reviewed-fact row

  // ---- capture ----
  let draft = $state('');
  let draftBox = $state(null); // the textarea element, for autogrow reset
  let capturing = $state(false);
  let landed = $state(null); // the CLI's own confirmation line

  // ---- the notebook (peek + sheet) ----
  // Notes only, for now: recently touched entities belong interleaved here
  // (frecency — the store already counts access), but the graph has no
  // recent-entities read yet; that is recorded residue, not a decision.
  // Likewise each row should carry the entities the note linked — the
  // rendering below is ready for a `note.entities` array, but `kg_notes`
  // does not return one yet (uid, source_id, body, occurred_at only), so
  // until that envelope grows the chips simply never appear.
  let recent = $state(null);
  let sheetOpen = $state(false);
  let sort = $state('newest'); // the envelope arrives newest-first
  let filter = $state('');
  let open_ = $state(null); // uid of the expanded row
  let editing = $state(null); // uid being rewritten
  let editText = $state('');
  let saving = $state(false);

  // The full notebook, not a taste of it: 200 is the graph side's own cap
  // on `kg_notes` (and the serve route mirrors it), so this asks for
  // everything it will give. A page holding exactly FETCH notes cannot tell
  // "all of them" from "the first 200 of more", so every count rendered from
  // this list says so with a `+`.
  const FETCH = 200;
  const atCap = $derived(recent?.length === FETCH);
  async function loadRecent() {
    try {
      const res = await fetch(`/api/notes?limit=${FETCH}`);
      if (res.ok) recent = (await res.json()).notes ?? [];
    } catch {
      // the list is a convenience; the capture is the point
    }
  }
  loadRecent();

  // Sorting and filtering are presentation over the fetched envelope — the
  // store's order is the truth, this is just how it is read.
  let shown = $derived.by(() => {
    let list = recent ?? [];
    const needle = filter.trim().toLowerCase();
    if (needle) list = list.filter((n) => (n.body ?? '').toLowerCase().includes(needle));
    return sort === 'oldest' ? [...list].reverse() : list;
  });

  async function find(q) {
    const text = (q ?? query).trim();
    if (!text) {
      results = null;
      hitEntities = [];
      return;
    }
    searching = true;
    try {
      const res = await fetch(`/api/find?q=${encodeURIComponent(text)}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      results = Array.isArray(data) ? data : (data.items ?? []);
      hitEntities = Array.isArray(data?.entities) ? data.entities : [];
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      searching = false;
    }
  }

  async function lookup(name) {
    const n = (name ?? query).trim();
    if (!n) return;
    busy = true;
    said = null;
    historyOpen = false;
    timeline = null;
    related = null;
    try {
      const res = await fetch(`/api/entity?${new URLSearchParams({ name: n })}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      entity = await res.json();
      error = null;
      results = null;
      hitEntities = [];
      if (entity?.found === false) {
        // A missed name is not a dead end: the search answers what the
        // lookup could not (design §2.2 — fallback, never `found: false`
        // as the last word).
        find(n);
      } else if (entity?.node?.name) {
        loadRelated(entity.node.name);
      }
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }
  if (query) lookup(query);

  async function loadRelated(name) {
    try {
      const res = await fetch(`/api/related?name=${encodeURIComponent(name)}`);
      if (res.ok) related = (await res.json()).items ?? [];
    } catch {
      // the neighborhood is a decoration on the page, not the page
    }
  }

  async function toggleHistory() {
    historyOpen = !historyOpen;
    if (!historyOpen || timeline || !entity?.node?.name) return;
    try {
      const res = await fetch(`/api/timeline?name=${encodeURIComponent(entity.node.name)}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      timeline = await res.json();
    } catch (e) {
      error = String(e?.message ?? e);
      historyOpen = false;
    }
  }

  function closeEntity() {
    entity = null;
    related = null;
    timeline = null;
    historyOpen = false;
    said = null;
  }

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
      const rest = { ...notes_ };
      delete rest[f.uid];
      notes_ = rest;
      await lookup(entity?.node?.name ?? query);
    } catch (e) {
      notes_ = { ...notes_, [f.uid]: String(e?.message ?? e) };
    } finally {
      busy = false;
    }
  }

  async function addFact() {
    const predicate = factPredicate.trim();
    const object = factObject.trim();
    if (!predicate || !object || !entity?.node?.id) return;
    factBusy = true;
    try {
      const res = await fetch('/api/facts', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          // The id, not the name: the page is looking at one resolved node,
          // and an ambiguous name must not re-roll which one this lands on.
          subject: entity.node.id,
          predicate,
          object: factLiteral ? null : object,
          value: factLiteral ? object : null,
        }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      said = (await res.json())?.output ?? 'asserted';
      addingFact = false;
      factPredicate = '';
      factObject = '';
      factLiteral = false;
      error = null;
      await lookup(entity?.node?.name ?? query);
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      factBusy = false;
    }
  }

  async function retractFact(uid) {
    factBusy = true;
    try {
      const res = await fetch('/api/facts/retract', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ uid }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      said = (await res.json())?.output ?? 'retracted';
      openFact = null;
      error = null;
      await lookup(entity?.node?.name ?? query);
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      factBusy = false;
    }
  }

  async function capture() {
    const text = draft.trim();
    if (!text) return;
    capturing = true;
    try {
      const res = await fetch('/api/notes', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ text }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      // The CLI's own line — "noted (episode N, M entities linked)" — not a
      // paraphrase of it. Capture visibly feeding the graph is the loop.
      landed = (await res.json())?.output ?? 'noted';
      draft = '';
      tick().then(grow); // measure after the DOM has the emptied value
      error = null;
      loadRecent();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      capturing = false;
    }
  }

  function toggle(note) {
    if (editing === note.uid) return; // an open editor is not a toggle target
    open_ = open_ === note.uid ? null : note.uid;
  }

  function startEdit(note) {
    editing = note.uid;
    editText = note.body;
    error = null;
  }

  function cancelEdit() {
    editing = null;
    editText = '';
  }

  async function saveEdit(note) {
    const text = editText.trim();
    if (!text || text === note.body) {
      cancelEdit();
      return;
    }
    saving = true;
    try {
      const res = await fetch('/api/notes/edit', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ source_id: note.source_id, text }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      cancelEdit();
      error = null;
      // Reload rather than patching the row in place: the graph decides what
      // the note now says (an edit whose text hashes the same is a no-op
      // there), and a row painted from what was typed would be this page's
      // guess about a store it does not own.
      loadRecent();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      saving = false;
    }
  }

  // The textarea grows with the note, up to a cap — a capture field should
  // feel like a line that stretches, not a form that waits. A function of
  // the element, not the event: `draft` is also assigned programmatically
  // (Dictate, and the reset after capture), and those paths never fire
  // `oninput`.
  function grow() {
    if (!draftBox) return;
    draftBox.style.height = 'auto';
    draftBox.style.height = `${Math.min(draftBox.scrollHeight, 160)}px`;
  }

  $effect(() => {
    const onKey = (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        findField?.focus();
      } else if (e.key === 'Escape' && sheetOpen) {
        // Nearest thing first: with a note editor open, Escape reads as
        // "cancel this edit", not "close the notebook".
        if (editing) cancelEdit();
        else sheetOpen = false;
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  const unreviewed = (f) => f.tier !== 'reviewed';
  const day = (ts) => (ts ?? '').slice(0, 10);
  const stamp = (ts) => (ts ?? '').slice(0, 16).replace('T', ' ');
</script>

{#snippet hazardGlyph(size = 12)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

{#snippet noteRow(note)}
  <div class="card noterow">
    {#if editing === note.uid}
      <textarea class="editbox" rows="6" bind:value={editText}></textarea>
      <div class="editrow">
        <button class="btn slim" disabled={saving} onclick={cancelEdit}>Cancel</button>
        <button
          class="btn primary grow"
          disabled={saving || !editText.trim()}
          onclick={() => saveEdit(note)}
        >{saving ? 'saving…' : 'Save'}</button>
      </div>
      <div class="editfoot">
        Rewrites the note in place, keeping when it happened. Anything the graph already
        derived from the old wording stays in your review queue — an edit is not a retraction.
      </div>
    {:else}
      <button class="notehead" onclick={() => toggle(note)}>
        <div class="notetext" class:clamp={open_ !== note.uid}>{note.body}</div>
        <div class="notemeta">{stamp(note.occurred_at)}</div>
      </button>
      {#if note.entities?.length}
        <div class="chiprow">
          {#each note.entities as name}
            <button class="entchip" onclick={() => { sheetOpen = false; lookup(name); }}>{name}</button>
          {/each}
        </div>
      {/if}
      {#if open_ === note.uid}
        <div class="editrow">
          <button class="btn slim grow" onclick={() => startEdit(note)}>Edit</button>
        </div>
      {/if}
    {/if}
  </div>
{/snippet}

<div class="page">
  <header>
    <span class="title">Graph</span>
    <span class="chip">notes · entities · facts</span>
  </header>

  <div class="scroll">
    {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}

    <form
      class="findrow"
      onsubmit={(e) => {
        e.preventDefault();
        lookup();
      }}
    >
      <input
        class="field"
        placeholder="find — an entity opens, anything else searches (⌘K)"
        bind:value={query}
        bind:this={findField}
        autocapitalize="off"
      />
      <Dictate onText={(text, err) => { if (text) { query = query ? `${query} ${text}` : text; lookup(); } if (err) error = err; }} />
      <button class="minibtn" disabled={busy || searching || !query.trim()}>Open</button>
    </form>

    {#if results !== null}
      {#if hitEntities.length}
        <div class="chiprow">
          {#each hitEntities as name}
            <button class="entchip" onclick={() => lookup(name)}>{name}</button>
          {/each}
        </div>
      {/if}
      {#each results as r}
        <div class="card noterow">
          <div class="notetext">{r.statement ?? r.name ?? r.text ?? JSON.stringify(r).slice(0, 140)}</div>
          <div class="notemeta">{r.kind ?? ''}{r.occurred_at ? ` · ${day(r.occurred_at)}` : ''}</div>
        </div>
      {:else}
        <div class="empty">Nothing matched{hitEntities.length ? ' — but these entities did' : ''}.</div>
      {/each}
    {/if}

    {#if entity?.found === false}
      <div class="footnote">no entity matches “{entity.query}” — the search above answers instead</div>
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
      {#if said}<div class="saidline">{said}</div>{/if}
      <div class="card head">
        <div class="rowtop">
          <button class="backbtn" onclick={closeEntity} title="back to capture">←</button>
          <span class="ename">{n.name}</span>
          <span class="chip">{n.node_type ?? n.type}</span>
        </div>
        {#if n.aliases?.length}
          <div class="rowsub">aka {n.aliases.join(' · ')}</div>
        {/if}
        {#if entity.interaction}
          <div class="rowsub">
            <span>
              {entity.interaction.interaction_count} interactions · last seen
              {day(entity.interaction.last_seen_at) || '—'} via
              {entity.interaction.last_channel ?? '—'}
            </span>
          </div>
        {/if}
        {#if entity.sources?.length}
          <!-- Which sources actually cover this entity, and over what span —
               returned by the store since the beginning, dropped by the old
               page. A source with no coverage is why two answers about the
               same person can differ. -->
          <div class="rowsub srcline">
            {#each entity.sources as s}
              <span>{s.source} ×{s.episodes}</span>
            {/each}
          </div>
        {/if}
      </div>

      {#if related?.length}
        <div class="sect">connected</div>
        <div class="chiprow">
          {#each related as r}
            <button class="entchip" onclick={() => lookup(r.name)} title={r.via?.predicate ?? ''}>
              {r.name}{r.via?.predicate ? ` · ${r.via.predicate}` : ''}
            </button>
          {/each}
        </div>
      {/if}

      <div class="sect">facts</div>
        {#each entity.facts ?? [] as f (f.uid)}
          <div class="card fact" class:denied={f.polarity === 'negative'}>
            {#if !unreviewed(f)}
              <!-- Tap to expand, then Retract: the destructive verb sits
                   behind two taps, and what is retracted is the uid of the
                   row that was tapped — never a text match. -->
              <button class="factbtn" onclick={() => (openFact = openFact === f.uid ? null : f.uid)}>
                <div class="factline">
                  {#if f.polarity === 'negative'}<span class="neg">✗</span>{/if}
                  <span class="statement">{f.statement}</span>
                </div>
              </button>
            {:else}
              <div class="factline">
                <span class="unrev" title="unreviewed">◌</span>
                {#if f.polarity === 'negative'}<span class="neg">✗</span>{/if}
                <span class="statement">{f.statement}</span>
              </div>
            {/if}
            <div class="meta">
              <span>{f.predicate}</span>
              {#if f.valid_from}<span>· as of {day(f.valid_from)}</span>{/if}
              <span>· {f.extractor ?? '?'}</span>
              {#if unreviewed(f)}<span class="unrevword">· unreviewed</span>{/if}
            </div>
            {#if !unreviewed(f) && openFact === f.uid}
              <div class="btnrow">
                <button class="minibtn" disabled={factBusy} onclick={() => retractFact(f.uid)}>
                  Retract
                </button>
              </div>
              <div class="editfoot">
                Ends or corrects this belief as of now — it moves to history, it is not erased.
                To restate it differently, retract and add the fact again.
              </div>
            {/if}
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
              {#if notes_[f.uid]}<div class="warnline">{notes_[f.uid]}</div>{/if}
            {/if}
          </div>
        {/each}

        {#if addingFact}
          <div class="card">
            <div class="factform">
              <span class="subjname">{n.name}</span>
              <input
                class="field small"
                placeholder="predicate — works_at, met_via, advises…"
                bind:value={factPredicate}
                autocapitalize="off"
              />
              <input
                class="field small"
                placeholder={factLiteral ? 'value — a date, a title, a number…' : 'object — an entity, by name'}
                bind:value={factObject}
              />
              <button
                class="entchip"
                onclick={() => (factLiteral = !factLiteral)}
                title="whether the object is another entity or a literal value"
              >{factLiteral ? 'a value' : 'an entity'}</button>
            </div>
            <div class="btnrow">
              <button class="minibtn" disabled={factBusy} onclick={() => (addingFact = false)}>Cancel</button>
              <button
                class="minibtn primary"
                disabled={factBusy || !factPredicate.trim() || !factObject.trim()}
                onclick={addFact}
              >{factBusy ? 'stating…' : 'State it'}</button>
            </div>
            <div class="editfoot">
              Lands live — your word, not a proposal. An object that names an entity becomes a
              connection; a predicate with no existing match is minted as new.
            </div>
          </div>
        {:else}
          <button class="sectbtn" onclick={() => (addingFact = true)}>+ add a fact or connection</button>
        {/if}

      <button class="sectbtn" onclick={toggleHistory}>
        {historyOpen ? 'history —' : 'history +'}
      </button>
      {#if historyOpen && timeline}
        {#each timeline.facts ?? [] as f (f.uid)}
          <div class="card fact" class:denied={f.superseded}>
            <div class="factline">
              {#if f.superseded}<span class="neg">✗</span>{/if}
              <span class="statement">{f.statement}</span>
            </div>
            <div class="meta">
              <span>{day(f.valid_from)}</span>
              {#if f.superseded}<span>· until {day(f.valid_to) || day(f.invalidated_at)}</span>{/if}
            </div>
          </div>
        {:else}
          <div class="empty">no superseded facts — the history is the present</div>
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
    {/if}

    {#if !entity?.node}
      <!-- The composer, not a form: the field is the star and the send
           affordance earns its color only once there is something to send.
           Enter stays a newline — a note is prose — so capture is the
           button or ⌘⏎. -->
      <div class="composer">
        <textarea
          rows="1"
          placeholder="Capture a note…"
          bind:value={draft}
          bind:this={draftBox}
          oninput={grow}
          onkeydown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
              e.preventDefault();
              capture();
            }
          }}
        ></textarea>
        <Dictate onText={(text, err) => { if (text) { draft = draft ? `${draft} ${text}` : text; tick().then(grow); } if (err) error = err; }} />
        <button
          class="round send"
          class:armed={!!draft.trim()}
          disabled={capturing || !draft.trim()}
          onclick={capture}
          title="capture — entities named in the note are linked on landing (⌘⏎)"
          aria-label="capture the note"
        >
          <svg viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V5M6 11l6-6 6 6" /></svg>
        </button>
      </div>
      {#if landed}<div class="saidline">{landed}</div>{/if}

      <!-- The notebook's closed lid: the newest notes showing through, the
           whole thing one target that opens the sheet. -->
      <button class="peek card" onclick={() => (sheetOpen = true)} aria-label="open the notebook">
        <div class="handle" aria-hidden="true"></div>
        <div class="peekhead">
          <span class="sect">notebook</span>
          <span class="peekcount">{recent === null ? '…' : atCap ? `${FETCH}+` : recent.length}</span>
        </div>
        {#if recent === null}
          <div class="empty">reading the notebook…</div>
        {:else}
          {#each recent.slice(0, 2) as note (note.uid)}
            <div class="peekrow">
              <div class="notetext clamp">{note.body}</div>
              <div class="notemeta">{stamp(note.occurred_at)}</div>
            </div>
          {:else}
            <div class="empty">Nothing captured yet — the first note starts it.</div>
          {/each}
        {/if}
      </button>
    {/if}
  </div>

  {#if sheetOpen}
    <div class="scrim" onclick={() => (sheetOpen = false)} aria-hidden="true"></div>
    <aside class="sheet">
      <button class="sheetlip" onclick={() => (sheetOpen = false)} aria-label="close the notebook">
        <div class="handle" aria-hidden="true"></div>
      </button>
      <div class="sheethead">
        <span class="sheettitle">Notebook</span>
        <span class="peekcount">{filter.trim() ? `${shown.length} of ${atCap ? `${FETCH}+` : (recent?.length ?? 0)}` : atCap ? `${FETCH}+` : shown.length}</span>
        <div class="sortrow">
          <button class="sortchip" class:on={sort === 'newest'} onclick={() => (sort = 'newest')}>newest</button>
          <button class="sortchip" class:on={sort === 'oldest'} onclick={() => (sort = 'oldest')}>oldest</button>
        </div>
      </div>
      <input class="field small" placeholder="filter notes…" bind:value={filter} />
      <div class="sheetscroll">
        {#each shown as note (note.uid)}
          {@render noteRow(note)}
        {:else}
          <div class="empty">{filter.trim() ? 'No note contains that.' : 'Nothing captured yet.'}</div>
        {/each}
        <div class="footnote">
          A note is evidence — what the graph derives from it waits in your review queue.
        </div>
      </div>
    </aside>
  {/if}
</div>

<style>
  /* position: relative so the notebook scrim and sheet can sit in the
     app's sheet band (z 4-6, absolute within the page — Tasks.svelte's
     idiom) rather than the drawer band; the bottom nav stays reachable. */
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  /* The right gutter leaves the corner clear for the shell's gear, the
     agreement every view's header keeps (#118). */
  header { display: flex; align-items: center; justify-content: space-between; padding: 22px 56px 12px 20px; }
  .title { font-weight: 500; font-size: 17px; letter-spacing: -0.02em; }
  .scroll { flex: 1; overflow-y: auto; padding: 2px 20px 20px; display: flex; flex-direction: column; gap: 10px; }
  .findrow { display: flex; gap: 8px; }
  .field { flex: 1; min-height: 44px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; padding: 0 12px; min-width: 0; }
  .field.small { min-height: 38px; font-size: 12px; background: var(--bg); }
  .field:focus { outline: 1px solid var(--accent-500); }
  .minibtn { min-height: 44px; padding: 0 16px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); color: var(--text); font-size: 13px; cursor: pointer; }
  .minibtn.primary { background: var(--accent-400); color: var(--void); border: none; font-weight: 500; }
  .minibtn:disabled { opacity: 0.5; }
  .chiprow { display: flex; flex-wrap: wrap; gap: 6px; }
  .entchip { background: var(--bg); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); color: var(--accent-400); font-family: var(--mono); font-size: 11px; padding: 6px 10px; cursor: pointer; }
  .card { background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); padding: 12px 14px; display: flex; flex-direction: column; gap: 7px; }
  .row { text-align: left; cursor: pointer; color: var(--text); font: inherit; }
  .rowtop { display: flex; align-items: center; gap: 8px; }
  .rowsub { font-size: 11px; color: var(--text-muted); }
  .srcline { display: flex; gap: 10px; flex-wrap: wrap; font-family: var(--mono); font-size: 10px; }
  .head { border-color: var(--accent-700); }
  .backbtn { background: none; border: none; color: var(--text-muted); font-size: 16px; cursor: pointer; padding: 0 4px 0 0; }
  .ename { font-size: 16px; font-weight: 500; }
  .pname { font-family: var(--mono); font-size: 13px; color: var(--accent-400); }
  .chip { font-family: var(--mono); font-size: 10px; color: var(--text-muted); background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); padding: 3px 8px; margin-left: auto; }
  .sect { font-family: var(--mono); font-size: 11px; color: var(--text-muted); margin-top: 4px; }
  .sectbtn { font-family: var(--mono); font-size: 11px; color: var(--text-muted); background: none; border: none; text-align: left; cursor: pointer; padding: 4px 0 0; }
  .factbtn { background: none; border: none; padding: 0; margin: 0; color: inherit; font: inherit; text-align: left; cursor: pointer; width: 100%; }
  .factform { display: flex; flex-direction: column; gap: 8px; }
  .factform .field.small { min-height: 42px; }
  .subjname { font-size: 14px; font-weight: 500; }
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
  .composer { display: flex; align-items: flex-end; gap: 8px; }
  .composer textarea { flex: 1; min-height: 44px; max-height: 160px; resize: none; overflow-y: auto; min-width: 0; }
  .round { width: 44px; height: 44px; border-radius: var(--radius); border: 1px solid var(--accent-900); background: var(--bg); color: var(--text-muted); display: flex; align-items: center; justify-content: center; cursor: pointer; flex-shrink: 0; }
  /* Armed by content: the send affordance stays quiet until there is a note
     to send, then takes the accent — attention follows the words in. */
  .round.send.armed { background: var(--accent-400); color: var(--void); border-color: var(--accent-400); }
  .round:disabled { opacity: 0.5; cursor: default; }
  .grow { flex: 1; }
  textarea { background: var(--surface); border: none; border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; line-height: 1.5; padding: 12px 14px; resize: vertical; }
  textarea:focus { outline: 1px solid var(--accent-500); }
  .handle { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-700); margin: 0 auto; }
  .peek { text-align: left; color: inherit; font: inherit; cursor: pointer; gap: 9px; margin-top: 6px; }
  .peekhead { display: flex; align-items: baseline; gap: 8px; }
  .peekcount { font-family: var(--mono); font-size: 11px; color: var(--accent-400); }
  .peekrow { display: flex; flex-direction: column; gap: 5px; border-top: 1px solid var(--accent-900); padding-top: 9px; }
  .scrim { position: absolute; inset: 0; background: rgba(0, 0, 0, 0.55); z-index: 5; }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; margin-inline: auto; max-width: 680px; height: auto; max-height: calc(100% - 12px); background: var(--bg); border: 1px solid var(--accent-700); border-bottom: none; border-radius: 14px 14px 0 0; z-index: 6; display: flex; flex-direction: column; gap: 10px; padding: 6px 14px 0; animation: sheet-in 0.2s ease-out; }
  @keyframes sheet-in { from { transform: translateY(100%); } to { transform: translateY(0); } }
  @media (prefers-reduced-motion: reduce) { .sheet { animation: none; } }
  .sheetlip { background: none; border: none; padding: 8px 0 2px; cursor: pointer; width: 100%; flex-shrink: 0; }
  .sheethead { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
  .sheettitle { font-weight: 500; font-size: 16px; letter-spacing: -0.02em; }
  .sortrow { display: flex; gap: 6px; margin-left: auto; }
  .sortchip { font-family: var(--mono); font-size: 11px; color: var(--text-muted); background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); padding: 6px 10px; cursor: pointer; }
  .sortchip.on { color: var(--accent-400); border-color: var(--accent-700); }
  /* `.field` is flex: 1 for row layouts; in the sheet's column it must not
     grow into the empty space. */
  .sheet .field.small { flex: 0 0 auto; }
  .sheetscroll { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 10px; padding: 2px 0 calc(14px + env(safe-area-inset-bottom)); }
  .sheetscroll > * { flex-shrink: 0; }
  .btn { min-height: 46px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; }
  .btn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .btn.slim { min-width: 72px; }
  .btn:disabled { opacity: 0.5; }
  .noterow { padding: 12px 14px; }
  /* The whole row is the target — a tap that only lands on the text is a
     tap most thumbs miss. */
  .notehead { background: none; border: none; padding: 0; margin: 0; color: inherit; font: inherit; text-align: left; display: flex; flex-direction: column; gap: 6px; cursor: pointer; width: 100%; }
  .notetext { font-size: 14px; line-height: 1.5; white-space: pre-wrap; }
  .notetext.clamp { display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .editbox { background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; line-height: 1.5; padding: 12px 14px; resize: vertical; }
  .editbox:focus { outline: 1px solid var(--accent-500); }
  .editrow { display: flex; gap: 8px; }
  .editfoot { font-size: 11px; color: var(--text-muted); line-height: 1.45; }
  .notemeta { font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .warnline { display: flex; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .saidline { font-family: var(--mono); font-size: 11px; color: var(--accent-400); }
  .empty { color: var(--text-muted); font-size: 13px; padding: 8px 0; }
  .footnote { font-size: 11px; color: var(--text-muted); text-align: center; padding-top: 6px; }
</style>
