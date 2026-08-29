<script>
  import { readVoicePrefs, writeVoicePrefs } from '../../../scripts/voice/voice-core.js';
  // The settings page: the charter (view + a validated edit), the learning
  // store (reflections and rules — read, edit, refuse), and the voice
  // stack's health (a read). The writes are the owner's own documents: the
  // charter every run carries, and the lessons and rules mined from the
  // owner's own corrections. See serve/settings.rs for the boundary and for
  // what is deliberately not editable from a browser.
  let charter = $state(null);
  let charterError = $state(null);
  let rules = $state(null);
  let rulesError = $state(null);
  let voice = $state(null);

  // The voice preference and the last call's voices/range, from voice-core's
  // own store — the same bytes a connecting call reads, so a choice made
  // here is the choice the next call opens with. The list and range are a
  // cache of the worker's last answer: a picker with no live call cannot
  // ask, and rendering the remembered answer with a dated note beats either
  // a hardcoded list or no picker at all.
  let vprefs = $state(readVoicePrefs());
  let vSavedNote = $state(null);

  function saveVoicePref(patch) {
    writeVoicePrefs(patch);
    vprefs = readVoicePrefs();
    vSavedNote = 'saved — applies from the next call';
  }

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
    await Promise.all([loadRules(), loadReflections()]);
    try {
      const res = await fetch('/api/settings/voice');
      if (res.ok) voice = await res.json();
    } catch {
      voice = null; // unknown, shown as a dash — never as "down"
    }
  }
  load();

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

  function openEditor() {
    // A first charter must not start from an empty buffer — the TUI writes
    // the comments-only template for the same reason, and the GET serves
    // those exact bytes so the two surfaces cannot drift. The template
    // carries the format and §11's "never disappoint" authoring trap.
    draft = charter?.raw || charter?.template || '';
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

  // ── The learning store ─────────────────────────────────────────────────
  // Two of the three stages a lesson passes through, in that order: the
  // reflection (one per intervention, before anything merges them) and the
  // rule (what a run actually carries, in every prompt's cached prefix).
  // Editing is offered at the *first* stage because that is where
  // disagreeing is cheap and precise — a rule is a consolidation of several
  // lessons, so objecting once one exists costs the good ones too. The third
  // stage, a rule proposal, is decided in the queue: accepting one applies a
  // whole rewritten set, which is not a decision to hand a thumb on a phone.
  //
  // Every action is one POST to a `mecha …` verb (serve/settings.rs runs the
  // child), so this page cannot do anything to the store that the command
  // line cannot — and the promotion an edit performs, the withholding that
  // makes it sound, and the git commit behind each write all stay in one
  // implementation.
  let pane = $state('reflections');
  let reflections = $state(null);
  let reflectionsError = $state(null);
  // The open lesson editor: { id, text }.
  let lessonDraft = $state(null);
  // The first tap of a two-tap refusal: { verb, id }. Drop and retire are
  // both flags rather than deletions, but both change what every future run
  // carries, so neither happens on one tap.
  let armed = $state(null);
  let armedReason = $state('');
  let learningBusy = $state(false);
  let learningNote = $state(null);
  let learningError = $state(null);
  // One reflection in full: { id, record }. What was happening and what was
  // said are the evidence a refusal rests on, and the listing carries
  // neither.
  let detail = $state(null);

  function setPane(next) {
    pane = next;
    // A different list, and the next tap may be a refusal — nothing armed,
    // half-edited or expanded should survive the move.
    lessonDraft = null;
    armed = null;
    detail = null;
    learningNote = null;
    learningError = null;
  }

  /// One verb, and its own report of what it did.
  ///
  /// The note is the child's stdout when it said something — `mecha
  /// reflections edit` prints the provenance move it just performed, which
  /// is the one thing about an edit that is not obvious from the result —
  /// and the fallback only where the verb's whole answer is "it worked".
  async function learningAct(path, body, fallback) {
    learningBusy = true;
    learningError = null;
    learningNote = null;
    try {
      const res = await fetch(path, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!res.ok) {
        // The CLI's refusal is the API's, arriving as its own last line.
        learningError = (await res.text()).trim();
        return false;
      }
      const out = await res.json();
      learningNote = (out?.output || '').trim() || fallback;
      return true;
    } catch (e) {
      learningError = String(e?.message ?? e);
      return false;
    } finally {
      learningBusy = false;
    }
  }

  // The editor opens on the existing lesson, so the owner can amend a
  // sentence rather than retype it — and `original` is kept beside the draft
  // because an unchanged save must not be offered. An edit is a *provenance
  // promotion*: saving the model's own words back would mark the record as
  // the owner's and let it into the rules, which is the one thing the
  // promotion's argument does not cover. `edit_reflexion` refuses it outright
  // (that is the guarantee); this only keeps the page from offering a button
  // whose whole effect would be a refusal.
  function openLesson(r) {
    armed = null;
    detail = null;
    learningNote = null;
    learningError = null;
    lessonDraft = { id: r.id, text: r.title, original: r.title };
  }

  const lessonChanged = $derived(
    !!lessonDraft?.text.trim() && lessonDraft.text.trim() !== lessonDraft.original.trim()
  );

  async function saveLesson() {
    if (!lessonChanged) return;
    const { id, text } = lessonDraft;
    if (await learningAct('/api/settings/reflections/edit', { id, text }, 'edited')) {
      lessonDraft = null;
      await loadReflections();
    }
  }

  /// Arm, then act. The reason rides with the second tap: it is recorded on
  /// the record, and for a retirement the learner is shown it, so the same
  /// lesson does not come back under new wording.
  function arm(verb, id) {
    lessonDraft = null;
    learningNote = null;
    learningError = null;
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
    if (await learningAct(path, { id, reason }, `${verb}ped`)) {
      await (verb === 'drop' ? loadReflections() : loadRules());
    }
  }

  async function restoreReflection(id) {
    armed = null;
    if (await learningAct('/api/settings/reflections/restore', { id }, 'restored')) {
      await loadReflections();
    }
  }

  async function restoreRule(id) {
    armed = null;
    if (await learningAct('/api/settings/rules/restore', { id }, 'restored')) {
      await loadRules();
    }
  }

  async function readDetail(id) {
    if (detail?.id === id) {
      detail = null;
      return;
    }
    detail = null;
    learningError = null;
    try {
      const res = await fetch(`/api/settings/reflections/show?id=${encodeURIComponent(id)}`);
      if (!res.ok) {
        learningError = (await res.text()).trim();
        return;
      }
      detail = { id, record: await res.json() };
    } catch (e) {
      learningError = String(e?.message ?? e);
    }
  }

  /// What the ledger says about one rule, in the TUI's own words.
  ///
  /// Absent is not zero: a rule no probe has ever reached is not a rule that
  /// passed, and rendering both as `0 regressions` would read as a clean
  /// bill of health for a rule nothing has ever measured.
  function tally(r) {
    if (r.user) return 'yours — never tallied, never retired';
    if (r.observations === null || r.observations === undefined || r.observations === 0) {
      return 'never validated — no probe has reached it';
    }
    return r.attributed_regressions
      ? `${r.observations} probe(s), ${r.attributed_regressions} attributed regression(s)`
      : `${r.observations} probe(s), none attributed`;
  }

  /// A learned rule with an id is the only thing `retire`/`restore` can
  /// resolve: a user rule is not on trial, and a rule minted before ids
  /// existed has nothing to name it with. Offering the button anyway would
  /// send an empty needle, which prefix-matches every rule in the store.
  const actionable = (r) => !r.user && !!r.id;

  const day = (s) => (s ? String(s).slice(0, 10) : '');

  // ── Voice cloning ──────────────────────────────────────────────────────
  // The TTS is a zero-shot cloner: a "voice" is a reference WAV in the
  // voices directory, so cloning is recording a clip and naming it. The
  // reading passage opens with spoken consent on purpose — the recording
  // that creates a synthetic copy of somebody's voice should itself carry
  // them agreeing to it, and the passage after it is there to cover pitch
  // movement, questions and pauses in under a minute. Everything stays on
  // this box: the clip is written to the local voices directory the local
  // TTS reads, and nothing else.
  const CLONE_PASSAGE =
    'I am recording my voice so that this assistant can speak as me, and I agree to ' +
    'that. My voice stays on this machine. Now, something with a bit of movement in ' +
    'it: the quick brown fox jumps over the lazy dog, while bright vixens leap and ' +
    'dozy fowl quack. Would I say a question sounds different from a statement? It ' +
    'does — it rises. And a pause, held for a moment, tells you as much as a word. ' +
    'That should be plenty; thank you for lending me your voice.';

  let recState = $state('idle'); // idle | recording | recorded
  let recSeconds = $state(0);
  let recUrl = $state(null);
  let cloneName = $state('');
  let cloneError = $state(null);
  let cloneBusy = $state(false);
  let deleteArmed = $state(null); // name of the voice a first tap armed
  let recStream = null;
  let recCtx = null;
  let recNode = null;
  let recChunks = [];
  let recRate = 48000;
  let recTimer = null;
  let recWav = null;

  async function startClone() {
    cloneError = null;
    try {
      recStream = await navigator.mediaDevices.getUserMedia({ audio: true });
      recCtx = new (window.AudioContext || window.webkitAudioContext)();
      recRate = recCtx.sampleRate;
      const source = recCtx.createMediaStreamSource(recStream);
      recNode = recCtx.createScriptProcessor(4096, 1, 1);
      recChunks = [];
      recNode.onaudioprocess = (e) =>
        recChunks.push(new Float32Array(e.inputBuffer.getChannelData(0)));
      source.connect(recNode);
      recNode.connect(recCtx.destination);
      recSeconds = 0;
      recTimer = setInterval(() => (recSeconds += 1), 1000);
      recState = 'recording';
    } catch (e) {
      cloneError = `microphone: ${e?.message ?? e}`;
      stopCapture();
    }
  }

  function stopCapture() {
    clearInterval(recTimer);
    recTimer = null;
    recNode?.disconnect();
    recCtx?.close();
    recStream?.getTracks().forEach((t) => t.stop());
    recNode = recCtx = recStream = null;
  }

  function stopClone() {
    stopCapture();
    // Full source rate, no downsampling — the dictation path shrinks to
    // 16 kHz for a transducer; a cloning reference is the one clip where
    // fidelity is the point.
    const total = recChunks.reduce((n, c) => n + c.length, 0);
    const pcm = new Float32Array(total);
    let off = 0;
    for (const c of recChunks) {
      pcm.set(c, off);
      off += c.length;
    }
    const out = new Int16Array(pcm.length);
    for (let i = 0; i < pcm.length; i++) {
      const v = Math.max(-1, Math.min(1, pcm[i]));
      out[i] = v < 0 ? v * 0x8000 : v * 0x7fff;
    }
    const buf = new ArrayBuffer(44 + out.length * 2);
    const dv = new DataView(buf);
    const str = (o, t) => [...t].forEach((ch, i) => dv.setUint8(o + i, ch.charCodeAt(0)));
    str(0, 'RIFF');
    dv.setUint32(4, 36 + out.length * 2, true);
    str(8, 'WAVE');
    str(12, 'fmt ');
    dv.setUint32(16, 16, true);
    dv.setUint16(20, 1, true);
    dv.setUint16(22, 1, true);
    dv.setUint32(24, recRate, true);
    dv.setUint32(28, recRate * 2, true);
    dv.setUint16(32, 2, true);
    dv.setUint16(34, 16, true);
    str(36, 'data');
    dv.setUint32(40, out.length * 2, true);
    new Int16Array(buf, 44).set(out);
    recWav = new Blob([buf], { type: 'audio/wav' });
    if (recUrl) URL.revokeObjectURL(recUrl);
    recUrl = URL.createObjectURL(recWav);
    recChunks = [];
    recState = 'recorded';
  }

  function discardClone() {
    stopCapture();
    if (recUrl) URL.revokeObjectURL(recUrl);
    recUrl = null;
    recWav = null;
    recState = 'idle';
    cloneError = null;
  }

  async function saveClone() {
    if (!recWav || !cloneName) return;
    cloneBusy = true;
    cloneError = null;
    try {
      const res = await fetch(
        `/api/settings/voice/clone?name=${encodeURIComponent(cloneName)}`,
        { method: 'POST', headers: { 'Content-Type': 'audio/wav' }, body: recWav }
      );
      if (!res.ok) {
        cloneError = (await res.text()).trim();
        return;
      }
      voice = await res.json();
      // The picker's list is the last call's answer; a voice that now
      // exists on disk belongs in it without waiting for one. The worker
      // itself revalidates on a miss, so offering it is honest.
      if (vprefs.voices && !vprefs.voices.includes(cloneName)) {
        writeVoicePrefs({ voices: [...vprefs.voices, cloneName].sort() });
        vprefs = readVoicePrefs();
      }
      const name = cloneName;
      cloneName = '';
      discardClone();
      vSavedNote = `cloned — pick “${name}” above to use it`;
    } catch (e) {
      cloneError = String(e?.message ?? e);
    } finally {
      cloneBusy = false;
    }
  }

  // Browser-back is the ordinary way off this view, and a recording left
  // running would otherwise hold the microphone (and its indicator light)
  // for the life of the page.
  $effect(() => () => {
    stopCapture();
    if (recUrl) URL.revokeObjectURL(recUrl);
  });

  async function deleteClone(name) {
    if (deleteArmed !== name) {
      deleteArmed = name;
      return;
    }
    deleteArmed = null;
    try {
      const res = await fetch('/api/settings/voice/clone/delete', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name }),
      });
      if (!res.ok) {
        cloneError = (await res.text()).trim();
        return;
      }
      voice = await res.json();
      if (vprefs.voices?.includes(name)) {
        writeVoicePrefs({ voices: vprefs.voices.filter((v) => v !== name) });
        vprefs = readVoicePrefs();
      }
    } catch (e) {
      cloneError = String(e?.message ?? e);
    }
  }

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
      {:else if charter && !charter.parse_error && !charter.error}
        <div class="card">
          <div class="sub">
            No charter yet — nothing rides in any prompt. Edit opens the format explained by
            example, ready to fill in.
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
    <div class="kicker">Learning</div>
    <div class="hint">
      What mecha has been taught, at the two stages you can act on. A <em>reflection</em> is one
      lesson mined from one of your interventions; a <em>rule</em> is what several of them
      consolidate into, and it rides in every run's prompt. Disagree at the lesson where you can —
      objecting to a rule costs the other lessons merged into it. Rule proposals are decided in the
      queue.
    </div>

    <div class="tabs">
      <button class="tab" class:on={pane === 'reflections'} onclick={() => setPane('reflections')}>
        reflections{#if reflections}<span class="n">{reflections.length}</span>{/if}
      </button>
      <button class="tab" class:on={pane === 'rules'} onclick={() => setPane('rules')}>
        rules{#if rules}<span class="n">{rules.length}</span>{/if}
      </button>
    </div>

    {#if learningError}
      <div class="card notice">{learningError}</div>
    {/if}
    {#if learningNote}
      <div class="card ok-note">{learningNote}</div>
    {/if}

    {#if pane === 'reflections'}
      {#if reflectionsError}
        <div class="card notice">could not read the reflections: {reflectionsError}</div>
      {:else if reflections === null}
        <div class="card"><div class="sub">loading…</div></div>
      {:else if reflections.length === 0}
        <div class="card">
          <div class="sub">No reflections yet — `mecha reflect` mines them from interventions.</div>
        </div>
      {:else}
        <div class="rules">
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
                  A lesson you write is yours: the provenance gate stops excluding it, so it can
                  become a rule — and what was happening is withheld on the way through, because
                  that is the field any third-party text was in.
                </div>
                <div class="row-actions">
                  <button
                    class="btn primary"
                    disabled={learningBusy || !lessonChanged}
                    onclick={saveLesson}>{learningBusy ? 'saving…' : 'Save lesson'}</button
                  >
                  <button class="btn" onclick={() => (lessonDraft = null)}>Cancel</button>
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
                    <!-- The gate's verdict, never the stored field: for a
                         record written before the harness-voice check existed
                         the two disagree, and this block sits directly under
                         a row that already reports the computed answer. -->
                    <div class="sub">
                      provenance {detail.record.provenance} · evidence {detail.record.evidence} ·
                      session {detail.record.session_id}
                    </div>
                  </div>
                {/if}

                {#if armed?.verb === 'drop' && armed.id === r.id}
                  <input
                    class="vname"
                    placeholder="why — recorded for the next reader (optional)"
                    bind:value={armedReason}
                    maxlength="200"
                  />
                {/if}

                <div class="row-actions">
                  <button class="btn tiny wide" onclick={() => openLesson(r)}>Edit lesson</button>
                  {#if r.dropped}
                    <button
                      class="btn tiny wide"
                      disabled={learningBusy}
                      onclick={() => restoreReflection(r.id)}>Restore</button
                    >
                  {:else if armed?.verb === 'drop' && armed.id === r.id}
                    <button
                      class="btn tiny wide armed"
                      disabled={learningBusy}
                      onclick={confirmArmed}>Confirm drop</button
                    >
                    <button class="btn tiny wide" onclick={() => (armed = null)}>Cancel</button>
                  {:else}
                    <button class="btn tiny wide" onclick={() => arm('drop', r.id)}>Drop</button>
                  {/if}
                  <button class="btn tiny wide" onclick={() => readDetail(r.id)}>
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
        </div>
      {/if}
    {:else if rulesError}
      <div class="card notice">could not read the rules: {rulesError}</div>
    {:else if rules === null}
      <div class="card"><div class="sub">loading…</div></div>
    {:else if rules.length === 0}
      <div class="card"><div class="sub">No rules yet — `mecha learn` creates them.</div></div>
    {:else}
      <div class="rules">
        <!-- `active` is `enabled && not retired`, so a rule hand-disabled in
             the learned-rules TOML is not retired and still rides in no
             prompt. On a pane whose job is "what a run actually carries",
             that has to read as spent too. -->
        {#each rules as r (r.id ?? r.title)}
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
            {:else if !r.active}
              <div class="sub blocked">
                disabled by hand in the rules file — it rides in no prompt, and retiring is the
                reversible way to say so
              </div>
            {/if}

            {#if armed?.verb === 'retire' && armed.id === r.id}
              <input
                class="vname"
                placeholder="why — shown to the learner so it does not come back reworded"
                bind:value={armedReason}
                maxlength="200"
              />
            {/if}

            {#if actionable(r)}
              <div class="row-actions">
                {#if r.retired}
                  <button
                    class="btn tiny wide"
                    disabled={learningBusy}
                    onclick={() => restoreRule(r.id)}>Restore</button
                  >
                {:else if armed?.verb === 'retire' && armed.id === r.id}
                  <button class="btn tiny wide armed" disabled={learningBusy} onclick={confirmArmed}
                    >Confirm retire</button
                  >
                  <button class="btn tiny wide" onclick={() => (armed = null)}>Cancel</button>
                {:else}
                  <button class="btn tiny wide" onclick={() => arm('retire', r.id)}>Retire</button>
                {/if}
              </div>
            {/if}
          </div>
        {/each}
        <div class="sub retired-count">
          Retiring is a flag, never a deletion: the rule stays in the file as evidence and the
          learner is told it was measured harmful, so restore can undo what erasure could not.
        </div>
      </div>
    {/if}
  </section>

  <section>
    <div class="kicker">Voice</div>
    <div class="hint">
      How calls sound. A choice here is what the next call opens with — a call already running
      keeps the voice it started in.
    </div>
    {#if vprefs.voices?.length}
      <div class="card vrow">
        <label class="vfield">
          <span class="label">voice</span>
          <select
            class="vpick"
            value={vprefs.voice ?? ''}
            onchange={(e) => saveVoicePref({ voice: e.currentTarget.value })}
          >
            {#if !vprefs.voice}<option value="">worker default</option>{/if}
            {#each vprefs.voices as v}
              <option value={v}>{v}</option>
            {/each}
          </select>
        </label>
        <label class="vfield">
          <span class="label">rate</span>
          <!-- The bounds are the worker's own last answer, never a literal
               here: it owns what it can speak at. -->
          <input
            type="range"
            min={vprefs.range?.min ?? 0.5}
            max={vprefs.range?.max ?? 2}
            step="0.05"
            value={vprefs.speed ?? 1}
            onchange={(e) => saveVoicePref({ speed: Number(e.currentTarget.value) })}
          />
          <span class="vval">{Number(vprefs.speed ?? 1).toFixed(2)}×</span>
        </label>
        {#if vSavedNote}<div class="sub ok">{vSavedNote}</div>{/if}
      </div>
    {:else}
      <div class="card">
        <div class="sub">
          No voice list remembered yet — it arrives from the worker on the first call, and the
          pickers appear here after that.
        </div>
      </div>
    {/if}
    {#if voice?.cloned !== null && voice?.cloned !== undefined}
      <div class="card vrow">
        <div class="label">Clone a voice</div>
        <div class="sub">
          Record someone reading the passage below — it opens with them agreeing, and the whole
          clip stays on this box. 15–60 seconds is plenty.
        </div>
        <blockquote class="passage">{CLONE_PASSAGE}</blockquote>
        {#if recState === 'idle'}
          <div class="row-actions">
            <button class="btn" onclick={startClone}>Record</button>
          </div>
        {:else if recState === 'recording'}
          <div class="row-actions">
            <button class="btn recording" onclick={stopClone}>Stop — {recSeconds}s</button>
            <button class="btn" onclick={discardClone}>Discard</button>
          </div>
        {:else}
          <audio controls src={recUrl}></audio>
          <div class="row-actions">
            <input
              class="vname"
              placeholder="name, e.g. luke"
              bind:value={cloneName}
              maxlength="40"
            />
            <button class="btn primary" disabled={!cloneName || cloneBusy} onclick={saveClone}>
              {cloneBusy ? 'saving…' : 'Save voice'}
            </button>
            <button class="btn" onclick={discardClone}>Discard</button>
          </div>
        {/if}
        {#if cloneError}
          <div class="sub notice">{cloneError}</div>
        {/if}
        {#if voice.cloned_error}
          <div class="sub notice">
            the voices directory could not be listed — recordings may not land: {voice.cloned_error}
          </div>
        {/if}
        {#if voice.cloned.length}
          <div class="cloned">
            {#each voice.cloned as c}
              <div class="cloned-row">
                <span class="cname">{c.name}</span>
                <span class="sub">
                  {c.seconds ? `${c.seconds.toFixed(0)}s` : ''}
                  {c.created ? ` · ${new Date(c.created * 1000).toLocaleDateString()}` : ''}
                </span>
                <button class="btn tiny" class:armed={deleteArmed === c.name} onclick={() => deleteClone(c.name)}>
                  {deleteArmed === c.name ? 'sure?' : 'delete'}
                </button>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {:else if voice}
      <div class="card">
        <div class="sub">
          Voice cloning is not configured — set <code>[web] voices_dir</code> to the host
          directory the TTS container mounts as /voices, and restart serve.
        </div>
      </div>
    {/if}
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
  /* Dropped and retired records stay on the page — a refusal that must be
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
  }
  .rule-text {
    font-size: 13px;
    line-height: 1.4;
    white-space: pre-wrap;
  }
  /* Why this one is excluded, retired or refused — the sentence a decision
     rests on, so it is never the same colour as the record itself. */
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
  .retired-count {
    color: var(--text-muted);
    font-size: 12px;
  }
  .vrow {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .vfield {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .vfield .label {
    width: 44px;
    color: var(--text-muted);
    font-family: var(--mono);
    font-size: 11px;
  }
  .vpick {
    flex: 1;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    font: inherit;
    font-size: 13px;
    padding: 5px 8px;
  }
  .vfield input[type='range'] {
    flex: 1;
    accent-color: var(--accent-400);
  }
  .vval {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--text-muted);
    width: 44px;
    text-align: right;
  }
  .sub.ok {
    color: var(--accent-300);
  }
  .sub.notice {
    color: var(--hazard);
    font-family: var(--mono);
    white-space: pre-wrap;
  }
  .passage {
    margin: 0;
    padding: 8px 10px;
    border-left: 2px solid var(--accent-700);
    color: var(--text);
    font-size: 13px;
    line-height: 1.5;
    background: var(--bg);
    border-radius: 0 var(--radius-chip) var(--radius-chip) 0;
  }
  .btn.recording {
    border-color: var(--hazard);
    color: var(--hazard);
  }
  .btn.tiny {
    padding: 2px 8px;
    font-size: 11px;
    margin-left: auto;
  }
  .btn.tiny.armed {
    border-color: var(--hazard);
    color: var(--hazard);
  }
  /* `.btn.tiny` pushes itself right — it was written for the one delete at
     the end of a voice row. In a row of verbs they sit together. */
  .btn.tiny.wide {
    margin-left: 0;
    padding: 4px 10px;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .vname {
    flex: 1;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    font: inherit;
    font-size: 13px;
    padding: 6px 8px;
    min-width: 0;
  }
  audio {
    width: 100%;
    height: 36px;
  }
  .cloned {
    display: flex;
    flex-direction: column;
    gap: 6px;
    border-top: 1px solid var(--accent-900);
    padding-top: 8px;
  }
  .cloned-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .cname {
    font-family: var(--mono);
    font-size: 12.5px;
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
