<script>
  import { readVoicePrefs, writeVoicePrefs } from '../../../scripts/voice/voice-core.js';
  // The settings page: the charter (view + a validated edit), the learned
  // rules (a read), and the voice stack's health (a read). The one write on
  // this whole page is the charter save, and the server refuses any save
  // the runs' own reader would not load — see serve/settings.rs for the
  // boundary and for what is deliberately not editable from a browser.
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
    try {
      const res = await fetch('/api/settings/rules');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      rules = await res.json();
      rulesError = null;
    } catch (e) {
      rulesError = String(e?.message ?? e);
    }
    try {
      const res = await fetch('/api/settings/voice');
      if (res.ok) voice = await res.json();
    } catch {
      voice = null; // unknown, shown as a dash — never as "down"
    }
  }
  load();

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

  const activeRules = $derived((rules ?? []).filter((r) => r.active));
  const retiredRules = $derived((rules ?? []).filter((r) => r.retired));
  const dash = (v) => (v === null || v === undefined ? '—' : v);
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
    <div class="kicker">Learned rules</div>
    <div class="hint">
      What rides in prompts from the learning loop. A read: retiring goes through its own staged
      review, never a tap here.
    </div>
    {#if rulesError}
      <div class="card notice">could not read the rules: {rulesError}</div>
    {:else if rules === null}
      <div class="card"><div class="sub">loading…</div></div>
    {:else if rules.length === 0}
      <div class="card"><div class="sub">No rules yet — `mecha learn` creates them.</div></div>
    {:else}
      <div class="rules">
        {#each activeRules as r}
          <div class="rule">
            <div class="rule-head">
              <span class="chip domain">{r.domain}</span>
              {#if r.user}<span class="chip">yours</span>{/if}
              <span class="tally"
                >{dash(r.observations)} obs · {dash(r.attributed_regressions)} regressions</span
              >
            </div>
            <div class="rule-text">{r.title}</div>
          </div>
        {/each}
        {#if retiredRules.length}
          <div class="sub retired-count">
            {retiredRules.length} retired rule{retiredRules.length === 1 ? '' : 's'} kept as
            evidence (mecha rules restore un-retires)
          </div>
        {/if}
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
  .rule-head {
    display: flex;
    align-items: center;
    gap: 8px;
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
  .tally {
    margin-left: auto;
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--text-muted);
  }
  .rule-text {
    font-size: 13px;
    line-height: 1.4;
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
