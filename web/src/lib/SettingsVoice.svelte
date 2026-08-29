<script>
  import { readVoicePrefs, writeVoicePrefs } from '../../../scripts/voice/voice-core.js';

  // The voice pane: how calls sound, and the references the local cloner
  // speaks from. Three unrelated things share this pane on purpose — the
  // worker's health explains why a picker is empty, so hiding it one screen
  // away would turn "no voices yet" into a mystery.
  let voice = $state(null);

  // The voice preference and the last call's voices/range, from voice-core's
  // own store — the same bytes a connecting call reads, so a choice made
  // here is the choice the next call opens with. The list and range are a
  // cache of the worker's last answer: a picker with no live call cannot
  // ask, and rendering the remembered answer with a dated note beats either
  // a hardcoded list or no picker at all.
  let vprefs = $state(readVoicePrefs());
  let vSavedNote = $state(null);

  async function load() {
    try {
      const res = await fetch('/api/settings/voice');
      if (res.ok) voice = await res.json();
    } catch {
      voice = null; // unknown, shown as a dash — never as "down"
    }
  }
  load();

  function saveVoicePref(patch) {
    writeVoicePrefs(patch);
    vprefs = readVoicePrefs();
    vSavedNote = 'saved — applies from the next call';
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
  // The passage is long, and on the default view it is the biggest block on
  // the page for a thing most visits do not do. It shows while recording
  // (that is when it is read) and on request before.
  let showPassage = $state(false);
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

  // Three states, not two. A route that did not answer leaves `voice` null,
  // which is *unknown* — asserting "not configured" there states a fact about
  // the owner's config that nothing checked. `Settings.svelte`'s `voiceLine`
  // makes the same distinction; this used to, via an `{:else if voice}`.
  const cloneState = $derived(
    voice === null ? 'unknown' : voice.cloned === null || voice.cloned === undefined ? 'off' : 'on'
  );
</script>

<p class="hint">
  How calls sound. A choice here is what the next call opens with — a call already running
  keeps the voice it started in.
</p>

<!-- The worker first: it is what explains an empty picker below. -->
{#if voice === null}
  <div class="status"><span class="label">worker</span><span class="val">—</span></div>
{:else if voice.offer_target === null}
  <div class="status"><span class="label">worker</span><span class="val">not wired on this serve</span></div>
{:else}
  <div class="status">
    <span class="label">worker</span>
    <span class="val" style:color={voice.worker_reachable ? 'var(--accent-400)' : 'var(--hazard)'}>
      {voice.worker_reachable ? 'up' : 'unreachable'}
    </span>
    <span class="target">{voice.offer_target}</span>
  </div>
{/if}

<section>
  <div class="kicker">Playback</div>
  {#if vprefs.voices?.length}
    <div class="card fields">
      <label class="vfield">
        <span class="flabel">voice</span>
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
        <span class="flabel">rate</span>
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
</section>

<section>
  <div class="kicker">Voices on this box</div>
  {#if cloneState === 'unknown'}
    <div class="card"><div class="sub">—</div></div>
  {:else if cloneState === 'off'}
    <div class="card">
      <div class="sub">
        Voice cloning is not configured — set <code>[web] voices_dir</code> to the host
        directory the TTS container mounts as /voices, and restart serve.
      </div>
    </div>
  {:else}
    {#if voice.cloned_error}
      <div class="card notice">
        the voices directory could not be listed — recordings may not land: {voice.cloned_error}
      </div>
    {/if}

    {#if voice.cloned.length}
      <div class="card cloned">
        {#each voice.cloned as c}
          <div class="cloned-row">
            <span class="cname">{c.name}</span>
            <span class="sub">
              {c.seconds ? `${c.seconds.toFixed(0)}s` : ''}
              {c.created ? ` · ${new Date(c.created * 1000).toLocaleDateString()}` : ''}
            </span>
            <button
              class="btn tiny"
              class:armed={deleteArmed === c.name}
              onclick={() => deleteClone(c.name)}
            >
              {deleteArmed === c.name ? 'sure?' : 'delete'}
            </button>
          </div>
        {/each}
      </div>
    {:else}
      <div class="card"><div class="sub">No cloned voices yet.</div></div>
    {/if}

    <div class="card recorder">
      {#if recState === 'idle'}
        <div class="sub">
          Record someone reading a short passage — it opens with them agreeing, and the whole
          clip stays on this box. 15–60 seconds is plenty.
        </div>
        {#if showPassage}
          <blockquote class="passage">{CLONE_PASSAGE}</blockquote>
        {/if}
        <div class="row-actions">
          <button class="btn primary" onclick={startClone}>Record a voice</button>
          <button class="btn ghost" onclick={() => (showPassage = !showPassage)}>
            {showPassage ? 'Hide passage' : 'Read the passage first'}
          </button>
        </div>
      {:else if recState === 'recording'}
        <blockquote class="passage">{CLONE_PASSAGE}</blockquote>
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
    </div>
  {/if}
</section>

<style>
  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: 12.5px;
    line-height: 1.45;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .status {
    display: flex;
    align-items: baseline;
    gap: 10px;
    font-size: 12.5px;
  }
  .status .label {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-muted);
  }
  .status .val {
    font-family: var(--mono);
  }
  .status .target {
    margin-left: auto;
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card {
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .notice {
    color: var(--hazard);
    font-size: 12.5px;
    white-space: pre-wrap;
    font-family: var(--mono);
  }
  .vfield {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .flabel {
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
    min-height: 36px;
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
  .sub {
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.4;
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
  .row-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
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
  .btn.primary {
    border-color: var(--accent-400);
    color: var(--accent-300);
  }
  .btn.ghost {
    border-color: transparent;
    background: none;
    color: var(--text-muted);
    padding: 7px 4px;
  }
  .btn.recording {
    border-color: var(--hazard);
    color: var(--hazard);
  }
  .btn.tiny {
    padding: 2px 8px;
    font-size: 11px;
    min-height: 28px;
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
    min-height: 40px;
    padding: 6px 8px;
    min-width: 0;
  }
  audio {
    width: 100%;
    height: 36px;
  }
  .cloned {
    gap: 8px;
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
  code {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--accent-300);
  }
</style>
