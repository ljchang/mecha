<script>
  // Recording or uploading a voice for the voice library (Library → Voices):
  // a reference clip the local TTS clones from, written to `[web] voices_dir`
  // through `/api/settings/voice/clone`. Moved here from the settings pane
  // with the library (owner, 2026-10-01); what it records is unchanged.
  import { apiFetch as fetch } from './api.js';
  import { readVoicePrefs, writeVoicePrefs } from '../../../scripts/voice/voice-core.js';

  let { onsaved = () => {} } = $props();

  // The playback picker's list is the last call's answer; a voice that now
  // exists on disk belongs in it without waiting for one. The worker itself
  // revalidates on a miss, so offering it is honest.
  function remember(name) {
    const prefs = readVoicePrefs();
    if (prefs.voices && !prefs.voices.includes(name)) {
      writeVoicePrefs({ voices: [...prefs.voices, name].sort() });
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
      remember(cloneName);
      const name = cloneName;
      cloneName = '';
      discardClone();
      onsaved(name);
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

  // A clip recorded elsewhere: a WAV is sent as it is, and the server reads
  // its header as it reads a recording's. Other formats need converting
  // first, which this page does not do.
  let fileInput = $state(null);
  function picked(e) {
    const file = e.currentTarget.files?.[0];
    e.currentTarget.value = '';
    if (!file) return;
    if (!/\.wav$/i.test(file.name) && file.type !== 'audio/wav' && file.type !== 'audio/x-wav') {
      cloneError = 'a WAV file, please — convert other formats first';
      return;
    }
    discardClone();
    recWav = file;
    recUrl = URL.createObjectURL(file);
    if (!cloneName) cloneName = file.name.replace(/\.wav$/i, '').toLowerCase().replace(/[^a-z0-9_-]+/g, '-').slice(0, 40);
    recState = 'recorded';
  }
</script>

<div class="card recorder">
  {#if recState === 'idle'}
    <div class="sub">
      Record someone reading a short passage — it opens with them agreeing, and the whole clip
      stays on this box. 15–60 seconds is plenty. Or upload a WAV recorded elsewhere.
    </div>
    {#if showPassage}
      <blockquote class="passage">{CLONE_PASSAGE}</blockquote>
    {/if}
    <div class="row-actions">
      <button class="btn primary" onclick={startClone}>Record a voice</button>
      <input type="file" accept=".wav,audio/wav" hidden bind:this={fileInput} onchange={picked} />
      <button class="btn" onclick={() => fileInput?.click()}>Upload a WAV</button>
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
      <input class="vname" placeholder="name, e.g. ada" bind:value={cloneName} maxlength="40" />
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

<style>
  .card {
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    background: var(--bg);
    border: 1px solid var(--accent-900);
    border-radius: 10px;
  }
  .sub {
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.4;
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
    background: var(--void);
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
  .vname {
    flex: 1;
    background: var(--void);
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
</style>
