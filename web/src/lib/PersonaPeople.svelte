<script>
  // Her people (PERSONA-LIFE-DESIGN.md §6, §15): concentric Dunbar rings with
  // her at the centre, portraits from library links, warm or tense lines,
  // ended ties greyed on the outer band, and a card for each person with
  // the timeline of what happened between them. The owner writes it here or
  // with `mecha persona people`; every change answers with the network as
  // it now stands. A persona with her people switched off says so.
  import { untrack } from 'svelte';
  import { apiFetch as fetch } from './api.js';
  import { personaUrl, personUrl, peopleUrl, acceptedLine, ringLayout, initials, eventLine, personBody, PEOPLE_TIES, PEOPLE_EVENTS, PEOPLE_LAYERS, PEOPLE_ABSENT, RINGS, RING_ORDER } from './persona.js';

  let { name, display = '', token = null, onclose } = $props();

  let data = $state(null);
  let error = $state('');
  let busy = $state(false);
  // The person whose card is open, by id.
  let picked = $state(null);
  let adding = $state(false);
  let editing = $state(false);
  let removing = $state(false);
  const blankPerson = () => ({ name: '', aliases: '', role: '', description: '', character: '', tie: '', label: '', layer: '', since: '' });
  const blankEvent = () => ({ kind: 'met', on: '', context: '', with: '', tie: '', label: '', layer: '' });
  let newPerson = $state(blankPerson());
  let newEvent = $state(blankEvent());
  // The time slider (§15): the day being looked back at, null for now.
  let asOf = $state(null);
  let asOfInput = $state('');
  // A proposal from identity.md: her model reading it, which of the waiting
  // people are ticked (numbered from 1), and what the last accept did.
  let proposing = $state(false);
  let ticked = $state([]);
  let proposalNote = $state('');
  let edit = $state(blankPerson());

  const people = $derived(data?.people ?? []);
  const nodes = $derived(ringLayout(people));
  const at = $derived(new Map(nodes.map((n) => [n.id, n])));
  const byId = $derived(new Map(people.map((p) => [p.id, p])));
  const person = $derived(picked ? byId.get(picked) ?? null : null);
  const ties = $derived(data?.ties ?? []);
  // The list beside the rings: everyone, by ring, innermost first.
  const listed = $derived(
    RING_ORDER
      .map((ring) => [ring, nodes.filter((n) => n.ring === ring)])
      .filter(([, ns]) => ns.length),
  );
  const ringName = (ring) => (ring === 'none' ? 'Not placed' : ring === 'ended' ? 'No longer in their life' : `Closest ${ring}`);
  const tieWord = (edge) => edge?.label ?? PEOPLE_TIES.find(([k]) => k === edge?.tie)?.[1] ?? null;
  // A timeline in the order things happened: dated first by date, undated
  // by when they were recorded.
  const timeline = (events) =>
    [...(events ?? [])].sort((a, b) => (a.when?.on ?? '9999').localeCompare(b.when?.on ?? '9999') || a.recorded_at.localeCompare(b.recorded_at));
  const tiesOf = (id) =>
    ties
      .filter((t) => t.a === id || t.b === id)
      .map((t) => ({ ...t, other: byId.get(t.a === id ? t.b : t.a) }))
      .filter((t) => t.other);

  async function load() {
    error = '';
    try {
      const res = await fetch(peopleUrl(name, asOf, token));
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      tickAll();
      if (picked && !data.people?.some((p) => p.id === picked)) picked = null;
    } catch (e) {
      data = null;
      error = String(e?.message ?? e);
    }
  }

  // One door, one answer: the network as it now stands, or why not.
  async function send(method, url, fields) {
    busy = true;
    error = '';
    try {
      const res = await fetch(url, {
        method,
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(personBody(fields, token)),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      return true;
    } catch (e) {
      error = String(e?.message ?? e);
      return false;
    } finally {
      busy = false;
    }
  }

  // Looking back is read only: the forms hide, and the network shown is
  // folded from the events by that day.
  const past = $derived(Boolean(data?.as_of));
  function tickAll() {
    ticked = (data?.proposal?.people ?? []).map((_, i) => i + 1);
  }
  function toggle(n) {
    ticked = ticked.includes(n) ? ticked.filter((t) => t !== n) : [...ticked, n].sort((a, b) => a - b);
  }

  function lookBack() {
    if (!asOfInput) return;
    asOf = asOfInput;
    picked = null;
    load();
  }

  function backToNow() {
    asOf = null;
    asOfInput = '';
    load();
  }

  // Her model reads identity.md for the people it names: a model run of a
  // minute or two, so the button says so while it runs.
  async function propose() {
    proposing = true;
    proposalNote = '';
    const ok = await send('POST', personaUrl(name, '/people/proposal', null), {});
    proposing = false;
    if (ok) {
      tickAll();
      if (data.proposed === 0) proposalNote = 'Their model found no one named in identity.md.';
    }
  }

  async function acceptProposal() {
    const all = (data?.proposal?.people ?? []).length;
    const only = ticked.length === all ? undefined : ticked;
    if (await send('POST', personaUrl(name, '/people/proposal/accept', null), { only })) {
      proposalNote = acceptedLine(data.accepted);
      tickAll();
    }
  }

  async function dismissProposal() {
    if (await send('DELETE', personaUrl(name, '/people/proposal', null), {})) proposalNote = '';
  }

  const words = (s) => (s.trim() ? s.trim() : undefined);
  const aliasList = (s) => s.split(',').map((a) => a.trim()).filter(Boolean);

  async function addPerson() {
    const p = newPerson;
    const ok = await send('POST', personaUrl(name, '/people', null), {
      name: p.name.trim(),
      aliases: aliasList(p.aliases),
      role: words(p.role),
      description: words(p.description),
      character: words(p.character),
      tie: words(p.tie),
      label: words(p.label),
      layer: p.layer ? Number(p.layer) : undefined,
      since: words(p.since),
    });
    if (ok) {
      newPerson = blankPerson();
      adding = false;
    }
  }

  function pick(id) {
    picked = picked === id ? null : id;
    editing = false;
    removing = false;
    newEvent = blankEvent();
  }

  function startEdit() {
    const p = person;
    edit = {
      ...blankPerson(),
      name: p.name,
      aliases: (p.aliases ?? []).join(', '),
      role: p.role ?? '',
      description: p.description ?? '',
      // A portrait this page was not shown (locked) is left out of the
      // edit, so saving keeps it: hidden, never cut.
      character: p.character ?? '',
    };
    editing = true;
  }

  async function saveEdit() {
    const p = person;
    const e = edit;
    const fields = {
      name: e.name.trim() !== p.name ? e.name.trim() : undefined,
      aliases: aliasList(e.aliases),
      role: e.role.trim(),
      description: e.description.trim(),
      // Sent only when the owner changed it, so a hidden one stays.
      character: e.character !== (p.character ?? '') ? e.character : undefined,
    };
    if (await send('PUT', personUrl(name, picked), fields)) editing = false;
  }

  async function record() {
    const e = newEvent;
    const ok = await send('POST', personUrl(name, picked, ''), {
      kind: e.kind,
      on: words(e.on),
      context: words(e.context),
      with: words(e.with),
      tie: words(e.tie),
      label: words(e.label),
      layer: e.layer ? Number(e.layer) : undefined,
    });
    if (ok) newEvent = blankEvent();
  }

  async function retract(event) {
    await send('DELETE', personUrl(name, picked, event), {});
  }

  async function remove() {
    if (await send('DELETE', personUrl(name, picked), {})) {
      picked = null;
      removing = false;
    }
  }

  $effect(() => {
    // Back to no card when the persona or the unlock changes under the page.
    void name;
    void token;
    untrack(() => {
      picked = null;
      adding = false;
      asOf = null;
      asOfInput = '';
      proposalNote = '';
      load();
    });
  });
</script>

<div class="people">
  {#if error}<div class="warnline">{error}</div>{/if}
  {#if !data && !error}
    <div class="pnote">loading…</div>
  {:else if data?.absent}
    <p class="pnote">{PEOPLE_ABSENT[data.absent] ?? data.why}</p>
  {:else if data}
    {#each data.problems ?? [] as problem}<div class="warnline">{display || name} {problem}.</div>{/each}
    <!-- The time slider: her network as it was at the end of a day. -->
    <div class="pwhen">
      <label class="pwhenl"><span>As it was on</span>
        <input type="date" bind:value={asOfInput} disabled={busy} />
      </label>
      <button class="linkbtn" disabled={busy || !asOfInput} onclick={lookBack}>Look back</button>
      {#if past}<button class="linkbtn" disabled={busy} onclick={backToNow}>Back to now</button>{/if}
    </div>
    {#if past}
      <p class="pnote pastnote">Their people as they were at the end of {data.as_of}, from what is recorded as having happened by then. Read only.</p>
    {/if}

    {#if !past}
      <!-- People proposed from identity.md (PL31): her model reads it, the
           owner ticks who to add, and nothing rewrites identity.md. -->
      {#if data.proposal}
        <div class="pcard">
          <div class="psec">Proposed from identity.md</div>
          {#if data.proposal.stale}
            <div class="warnline">identity.md has changed since these were read from it; propose again to accept any.</div>
          {/if}
          <ul class="pprop">
            {#each data.proposal.people as q, i}
              <li>
                <label>
                  <input type="checkbox" checked={ticked.includes(i + 1)} disabled={busy || data.proposal.stale} onchange={() => toggle(i + 1)} />
                  <span class="pname2">{q.name}</span>
                  <span class="psub">{[q.role, q.label && q.label !== q.role ? q.label : null, q.layer ? `closest ${q.layer}` : null].filter(Boolean).join(' · ')}</span>
                  {#if q.description}<span class="pdesc2">{q.description}</span>{/if}
                </label>
              </li>
            {/each}
          </ul>
          <div class="pacts">
            <button class="abtn" disabled={busy || data.proposal.stale || !ticked.length} onclick={acceptProposal}>
              Add {ticked.length === data.proposal.people.length ? 'them all' : `${ticked.length} ticked`}
            </button>
            <button class="linkbtn" disabled={busy} onclick={dismissProposal}>Dismiss</button>
          </div>
        </div>
      {:else}
        <div class="pacts">
          <button class="linkbtn" disabled={busy || proposing} onclick={propose}>
            {proposing ? 'Reading identity.md…' : 'Propose their people from identity.md'}
          </button>
        </div>
      {/if}
      {#if proposalNote}<p class="pnote">{proposalNote}</p>{/if}
    {/if}
    {#if !past && data.overlap?.length}
      <div class="pnote">
        identity.md still names people who now live in their network. Edit these lines on the Who they are tab so each lives in one place:
        <ul class="overlap">{#each data.overlap as o}<li><span class="lno">{o.line}</span> {o.text}</li>{/each}</ul>
      </div>
    {/if}

    <svg class="rings" viewBox="-100 -100 200 200" role="img" aria-label="Their people in rings by closeness, the persona at the centre">
      {#each PEOPLE_LAYERS as l}<circle class="ring" r={RINGS[l]} />{/each}
      <circle class="ring dashed" r={RINGS.none} />
      <circle class="band" r={RINGS.ended} />
      {#each ties as t}
        {#if at.get(t.a) && at.get(t.b)}
          <line class="tie" class:tense={t.edge.valence === 'tense'} class:ended={t.edge.status === 'ended'}
            x1={at.get(t.a).x} y1={at.get(t.a).y} x2={at.get(t.b).x} y2={at.get(t.b).y} />
        {/if}
      {/each}
      {#each nodes as n (n.id)}
        <line class="spoke {n.valence ?? ''}" class:paused={n.status === 'paused'} class:ended={n.status === 'ended'} x1="0" y1="0" x2={n.x} y2={n.y} />
      {/each}
      <circle class="self" r="10" />
      <text class="selfini" y="3.2">{initials(display || name)}</text>
      {#each nodes as n (n.id)}
        <g class="node" class:sel={picked === n.id} class:ended={n.status === 'ended'} transform="translate({n.x} {n.y})"
          role="button" tabindex="0" aria-label={n.name}
          onclick={() => pick(n.id)} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), pick(n.id))}>
          <clipPath id="pclip-{n.id}"><circle r="7" /></clipPath>
          {#if n.portrait}
            <image href={n.portrait} x="-7" y="-7" width="14" height="14" clip-path="url(#pclip-{n.id})" preserveAspectRatio="xMidYMid slice" />
          {:else}
            <circle class="nodebg" r="7" />
            <text class="ini" y="2.4">{initials(n.name)}</text>
          {/if}
          <circle class="rim" class:unvouched={n.unvouched} r="7.6" />
          <text class="nlabel" y="12.5">{n.name}</text>
        </g>
      {/each}
    </svg>
    <div class="plegend">
      <span class="lchip"><span class="sw warm"></span>warm</span>
      <span class="lchip"><span class="sw tense"></span>tense</span>
      <span class="lchip"><span class="sw dim"></span>no word yet, or ended</span>
    </div>

    {#if !people.length}
      <p class="pnote">No one yet. Add the people in their life below, or propose them from identity.md: <code>mecha persona people propose {name}</code>.</p>
    {/if}

    <!-- The same people as a list: a tap target on any screen, and the way in
         for a screen reader. -->
    {#each listed as [ring, ns]}
      <div class="pring">
        <div class="pringname">{ringName(ring)}</div>
        <div class="pchips">
          {#each ns as n (n.id)}<button class="pchip" class:active={picked === n.id} onclick={() => pick(n.id)}>{n.name}</button>{/each}
        </div>
      </div>
    {/each}

    {#if person}
      <div class="pcard">
        <div class="phead">
          {#if person.portrait}<img class="pthumb" src={person.portrait} alt="" />{/if}
          <div>
            <div class="pname">{person.name}</div>
            <div class="psub">
              {[person.role, tieWord(person.edge), person.edge?.layer ? `closest ${person.edge.layer}` : null].filter(Boolean).join(' · ') || 'no role or tie yet'}
            </div>
            <div class="psub">
              {[person.edge?.started?.said ? `since ${person.edge.started.said}` : null,
                person.edge?.status === 'ended' ? `ended${person.edge?.ended?.said ? ` ${person.edge.ended.said}` : ''}` : person.edge?.status === 'paused' ? 'out of touch' : null,
                person.edge?.valence].filter(Boolean).join(' · ')}
            </div>
          </div>
        </div>
        {#if person.description}<p class="pdesc">{person.description}</p>{/if}
        {#if person.aliases?.length}<p class="psub">Also called {person.aliases.join(', ')}</p>{/if}
        {#if person.edge?.unreadable || person.edge?.unclean}
          <div class="warnline">
            {#if person.edge.unreadable}{person.edge.unreadable} event{person.edge.unreadable === 1 ? '' : 's'} this version cannot read.{/if}
            {#if person.edge.unclean}Some of it came from a source that is not clean.{/if}
            This tie is kept out of their chats until it reads whole.
          </div>
        {/if}

        <div class="psec">What happened</div>
        <ul class="ptime">
          {#each timeline(person.events) as e (e.id)}
            <li>{eventLine(e)}{#if e.source === 'owner' && !past}<button class="linkbtn take" disabled={busy} onclick={() => retract(e.id)}>take back</button>{/if}</li>
          {/each}
        </ul>
        {#each tiesOf(person.id) as t}
          <div class="psec">With {t.other.name}: {[tieWord(t.edge), t.edge.valence, t.edge.status === 'ended' ? 'ended' : null].filter(Boolean).join(' · ') || 'a tie'}</div>
          <ul class="ptime">
            {#each timeline(t.events) as e (e.id)}
              <li>{eventLine(e)}{#if e.source === 'owner' && !past}<button class="linkbtn take" disabled={busy} onclick={() => retract(e.id)}>take back</button>{/if}</li>
            {/each}
          </ul>
        {/each}

        {#if !past}
        <div class="psec">Record something</div>
        <div class="pform">
          <select bind:value={newEvent.kind} disabled={busy} aria-label="What happened">
            {#each PEOPLE_EVENTS as [id, label]}<option value={id}>{label}</option>{/each}
          </select>
          <input bind:value={newEvent.on} placeholder="when: 2019, summer 2019, 2019-06" disabled={busy} aria-label="When" />
          <input bind:value={newEvent.context} placeholder="what it was about (a line)" disabled={busy} aria-label="What it was about" />
          <select bind:value={newEvent.with} disabled={busy} aria-label="Between them and">
            <option value="">between {display || name} and {person.name}</option>
            {#each people.filter((p) => p.id !== person.id) as p (p.id)}<option value={p.id}>between {person.name} and {p.name}</option>{/each}
          </select>
          <select bind:value={newEvent.tie} disabled={busy} aria-label="The tie, if it changed">
            <option value="">tie unchanged</option>
            {#each PEOPLE_TIES as [id, label]}<option value={id}>{label}</option>{/each}
          </select>
          <select bind:value={newEvent.layer} disabled={busy} aria-label="How close, if it changed">
            <option value="">closeness unchanged</option>
            {#each PEOPLE_LAYERS as l}<option value={l}>closest {l}</option>{/each}
          </select>
          <button class="abtn" disabled={busy} onclick={record}>Record</button>
        </div>

        <div class="pacts">
          {#if !editing}<button class="linkbtn" disabled={busy} onclick={startEdit}>Edit who they are</button>{/if}
          {#if !removing}
            <button class="linkbtn" disabled={busy} onclick={() => (removing = true)}>Remove from their network…</button>
          {:else}
            <span class="pnote">Everything recorded with {person.name} goes too.</span>
            <button class="abtn danger" disabled={busy} onclick={remove}>Remove {person.name}</button>
            <button class="linkbtn" disabled={busy} onclick={() => (removing = false)}>Keep</button>
          {/if}
        </div>
        {#if editing}
          <div class="pform">
            <input bind:value={edit.name} placeholder="name" disabled={busy} aria-label="Name" />
            <input bind:value={edit.aliases} placeholder="also called (comma separated)" disabled={busy} aria-label="Also called" />
            <input bind:value={edit.role} placeholder="role in their life: older sister" disabled={busy} aria-label="Role" />
            <input bind:value={edit.description} placeholder="a line about them" disabled={busy} aria-label="A line about them" />
            <select bind:value={edit.character} disabled={busy} aria-label="Portrait">
              <option value="">no portrait</option>
              {#each data.characters ?? [] as c}<option value={c}>{c}</option>{/each}
              {#if edit.character && !(data.characters ?? []).includes(edit.character)}<option value={edit.character}>{edit.character} (not approved)</option>{/if}
            </select>
            <button class="abtn" disabled={busy || !edit.name.trim()} onclick={saveEdit}>Save</button>
            <button class="linkbtn" disabled={busy} onclick={() => (editing = false)}>Cancel</button>
          </div>
        {/if}
        {/if}
      </div>
    {/if}

    {#if past}
      <!-- Nothing is added while looking back. -->
    {:else if !adding}
      <button class="linkbtn" onclick={() => (adding = true)}>+ Add someone to their life</button>
    {:else}
      <div class="pcard">
        <div class="psec">Someone in their life</div>
        <div class="pform">
          <input bind:value={newPerson.name} placeholder="name" disabled={busy} aria-label="Name" />
          <input bind:value={newPerson.role} placeholder="role in their life: older sister" disabled={busy} aria-label="Role" />
          <input bind:value={newPerson.description} placeholder="a line about them" disabled={busy} aria-label="A line about them" />
          <input bind:value={newPerson.aliases} placeholder="also called (comma separated)" disabled={busy} aria-label="Also called" />
          <select bind:value={newPerson.tie} disabled={busy} aria-label="Tie">
            <option value="">tie</option>
            {#each PEOPLE_TIES as [id, label]}<option value={id}>{label}</option>{/each}
          </select>
          <input bind:value={newPerson.label} placeholder="the tie in their words: sister" disabled={busy} aria-label="The tie in their words" />
          <select bind:value={newPerson.layer} disabled={busy} aria-label="How close">
            <option value="">how close</option>
            {#each PEOPLE_LAYERS as l}<option value={l}>closest {l}</option>{/each}
          </select>
          <input bind:value={newPerson.since} placeholder="since: 2019, summer 2019" disabled={busy} aria-label="Since" />
          <select bind:value={newPerson.character} disabled={busy} aria-label="Portrait">
            <option value="">no portrait</option>
            {#each data.characters ?? [] as c}<option value={c}>{c}</option>{/each}
          </select>
          <button class="abtn" disabled={busy || !newPerson.name.trim()} onclick={addPerson}>Add</button>
          <button class="linkbtn" disabled={busy} onclick={() => { adding = false; newPerson = blankPerson(); }}>Cancel</button>
        </div>
      </div>
    {/if}
  {/if}
  <button class="abtn" onclick={() => onclose?.()}>Close</button>
</div>

<style>
  .people { display: flex; flex-direction: column; gap: 12px; }
  .pnote { margin: 0; font-size: 12px; color: var(--text-muted); line-height: 1.45; }
  .pnote code { font-family: var(--mono); font-size: 11px; }
  .warnline { font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .overlap { margin: 4px 0 0; padding-left: 18px; }
  .overlap li { margin: 2px 0; }
  .lno { font-family: var(--mono); color: var(--text-muted); margin-right: 4px; }
  .rings { width: 100%; max-width: 520px; align-self: center; touch-action: manipulation; }
  .ring { fill: none; stroke: var(--accent-900); stroke-width: 0.4; }
  .ring.dashed { stroke-dasharray: 1.5 1.5; }
  .band { fill: none; stroke: var(--accent-900); stroke-width: 6; opacity: 0.35; }
  .spoke { stroke: var(--text-muted); stroke-width: 0.6; opacity: 0.45; }
  .spoke.warm { stroke: #6fa37a; opacity: 0.9; }
  .spoke.tense { stroke: #c9894a; opacity: 0.9; stroke-dasharray: 2 1; }
  .spoke.paused { stroke-dasharray: 0.8 1.6; }
  .spoke.ended { opacity: 0.2; }
  .tie { stroke: var(--text-muted); stroke-width: 0.45; opacity: 0.5; }
  .tie.tense { stroke: #c9894a; opacity: 0.85; }
  .tie.ended { opacity: 0.2; }
  .self { fill: var(--accent-400); }
  .selfini { font-size: 7px; text-anchor: middle; fill: var(--surface); font-weight: 600; pointer-events: none; }
  .node { cursor: pointer; outline: none; }
  .node.ended { opacity: 0.45; }
  .nodebg { fill: var(--surface); stroke: var(--accent-900); stroke-width: 0.5; }
  .ini { font-size: 5.5px; text-anchor: middle; fill: var(--text); pointer-events: none; }
  .rim { fill: none; stroke: transparent; stroke-width: 0.9; }
  .rim.unvouched { stroke: var(--hazard); stroke-dasharray: 1 1; }
  .node.sel .rim, .node:focus-visible .rim { stroke: var(--text); }
  .nlabel { font-size: 4.6px; text-anchor: middle; fill: var(--text); pointer-events: none; }
  .plegend { display: flex; flex-wrap: wrap; gap: 10px; justify-content: center; }
  .lchip { display: inline-flex; align-items: center; gap: 5px; font-size: 11px; color: var(--text-muted); }
  .sw { width: 14px; height: 3px; border-radius: 2px; }
  .sw.warm { background: #6fa37a; }
  .sw.tense { background: #c9894a; }
  .sw.dim { background: var(--text-muted); opacity: 0.5; }
  .pring { display: flex; flex-direction: column; gap: 4px; }
  .pringname { font-size: 11px; color: var(--text-muted); }
  .pchips { display: flex; flex-wrap: wrap; gap: 6px; }
  .pchip { min-height: 36px; padding: 0 12px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: 18px; color: var(--text); font-size: 13px; cursor: pointer; }
  .pchip.active { border-color: var(--accent-400); color: var(--accent-400); }
  .pcard { display: flex; flex-direction: column; gap: 8px; padding: 10px 12px; border: 1px solid var(--accent-900); border-radius: var(--radius); }
  .phead { display: flex; gap: 10px; align-items: center; }
  .pthumb { width: 48px; height: 48px; border-radius: 50%; object-fit: cover; }
  .pname { font-size: 15px; color: var(--text); }
  .psub { font-size: 12px; color: var(--text-muted); line-height: 1.4; margin: 0; }
  .pdesc { margin: 0; font-size: 13px; color: var(--text); line-height: 1.45; }
  .psec { font-size: 12px; color: var(--text); margin-top: 4px; }
  .ptime { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 3px; font-size: 12px; color: var(--text); line-height: 1.4; }
  .pform { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .pform input, .pform select { min-height: 36px; padding: 0 8px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font: inherit; font-size: 13px; flex: 1 1 160px; min-width: 0; }
  /* A field under 16px makes iOS zoom the page, and it stays zoomed. */
  @media (pointer: coarse) { .pform input, .pform select { font-size: 16px; } }
  .pacts { display: flex; flex-wrap: wrap; gap: 12px; align-items: center; }
  .linkbtn { background: none; border: none; padding: 4px 0; min-height: 32px; color: var(--accent-400); font-size: 12px; cursor: pointer; text-align: left; }
  .linkbtn:disabled, .abtn:disabled { opacity: 0.5; cursor: default; }
  .abtn { min-height: 44px; padding: 0 16px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; }
  /* Its own margin: Svelte trims the space at the block's edge. */
  .take { margin-left: 8px; min-height: 0; padding: 0; }
  .pwhen { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  .pwhenl { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--text-muted); }
  .pwhenl input { min-height: 36px; padding: 0 8px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font: inherit; font-size: 13px; }
  @media (pointer: coarse) { .pwhenl input { font-size: 16px; } }
  .pastnote { color: var(--accent-400); }
  .pprop { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 8px; }
  .pprop label { display: grid; grid-template-columns: auto 1fr; column-gap: 8px; row-gap: 2px; align-items: baseline; cursor: pointer; }
  .pprop input { width: 18px; height: 18px; }
  .pprop .psub, .pprop .pdesc2 { grid-column: 2; }
  .pname2 { font-size: 14px; color: var(--text); }
  .pdesc2 { font-size: 12px; color: var(--text); line-height: 1.4; }
  .abtn.danger { border-color: var(--hazard); color: var(--hazard); }
</style>
