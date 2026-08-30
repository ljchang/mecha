<script>
  // Hold nothing, tap twice: a mic button that records, encodes 16 kHz mono
  // WAV in the page, and hands the clip to /api/dictate — the local
  // Parakeet transducer, which cannot obey speech, only transcribe it. The
  // audio never leaves the box; that is the whole argument against the
  // browser speech APIs, which ship the clip to a third party.
  //
  // WAV is encoded here because Parakeet reads WAV and MediaRecorder emits
  // opus: capture PCM off an AudioContext, downsample to 16 kHz, write the
  // 44-byte header. ~2 MB a minute — a long thought still fits the route's
  // limit.
  let { onText = () => {} } = $props();
  let state_ = $state('idle'); // idle | recording | transcribing
  let stream = null;
  let ctx = null;
  let node = null;
  let chunks = [];
  let sourceRate = 48000;

  async function start() {
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      ctx = new (window.AudioContext || window.webkitAudioContext)();
      sourceRate = ctx.sampleRate;
      const source = ctx.createMediaStreamSource(stream);
      node = ctx.createScriptProcessor(4096, 1, 1);
      chunks = [];
      node.onaudioprocess = (e) => chunks.push(new Float32Array(e.inputBuffer.getChannelData(0)));
      source.connect(node);
      node.connect(ctx.destination);
      state_ = 'recording';
    } catch (e) {
      onText(null, `microphone: ${e?.message ?? e}`);
      cleanup();
    }
  }

  function cleanup() {
    node?.disconnect();
    ctx?.close();
    stream?.getTracks().forEach((t) => t.stop());
    node = ctx = stream = null;
    state_ = 'idle';
  }

  function encodeWav() {
    const total = chunks.reduce((n, c) => n + c.length, 0);
    const pcm = new Float32Array(total);
    let off = 0;
    for (const c of chunks) {
      pcm.set(c, off);
      off += c.length;
    }
    // Downsample to 16 kHz by stride — fine for speech into a transducer.
    const ratio = sourceRate / 16000;
    const n = Math.floor(pcm.length / ratio);
    const out = new Int16Array(n);
    for (let i = 0; i < n; i++) {
      const s = Math.max(-1, Math.min(1, pcm[Math.floor(i * ratio)]));
      out[i] = s < 0 ? s * 0x8000 : s * 0x7fff;
    }
    const buf = new ArrayBuffer(44 + out.length * 2);
    const v = new DataView(buf);
    const str = (o, t) => [...t].forEach((ch, i) => v.setUint8(o + i, ch.charCodeAt(0)));
    str(0, 'RIFF');
    v.setUint32(4, 36 + out.length * 2, true);
    str(8, 'WAVE');
    str(12, 'fmt ');
    v.setUint32(16, 16, true);
    v.setUint16(20, 1, true);
    v.setUint16(22, 1, true);
    v.setUint32(24, 16000, true);
    v.setUint32(28, 32000, true);
    v.setUint16(32, 2, true);
    v.setUint16(34, 16, true);
    str(36, 'data');
    v.setUint32(40, out.length * 2, true);
    new Int16Array(buf, 44).set(out);
    return buf;
  }

  async function stop() {
    state_ = 'transcribing';
    const wav = encodeWav();
    cleanup();
    state_ = 'transcribing';
    try {
      const res = await fetch('/api/dictate', {
        method: 'POST',
        headers: { 'content-type': 'audio/wav' },
        body: wav,
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      const text = (data.text ?? '').trim();
      onText(text || null, text ? null : 'heard nothing');
    } catch (e) {
      onText(null, String(e?.message ?? e));
    } finally {
      state_ = 'idle';
    }
  }

  function toggle() {
    if (state_ === 'idle') start();
    else if (state_ === 'recording') stop();
  }

  $effect(() => () => cleanup());
</script>

<!-- type="button", load-bearing: a button defaults to type="submit", so
     inside a form (the graph tab's find row) implicit submission — the
     user hitting Enter in the text field — "clicks" the first submit
     button it finds, which was this mic. Enter searched; the mic listened. -->
<button
  type="button"
  class="dictate"
  class:rec={state_ === 'recording'}
  class:busy={state_ === 'transcribing'}
  disabled={state_ === 'transcribing'}
  onclick={toggle}
  title={state_ === 'recording' ? 'tap to finish — transcribed locally' : 'dictate (local STT, nothing leaves the box)'}
>
  {#if state_ === 'transcribing'}
    <span class="dots">…</span>
  {:else}
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
      <rect x="9" y="3" width="6" height="11" rx="3" />
      <path d="M5 11a7 7 0 0014 0M12 18v3" />
    </svg>
  {/if}
</button>

<style>
  .dictate {
    min-width: 44px;
    min-height: 44px;
    background: var(--bg);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius);
    color: var(--text-muted);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .dictate.rec {
    color: var(--hazard);
    border-color: var(--hazard);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .dictate.busy {
    color: var(--accent-400);
  }
  @keyframes pulse {
    50% {
      background: color-mix(in srgb, var(--hazard) 12%, var(--bg));
    }
  }
  .dots {
    font-size: 16px;
    letter-spacing: 2px;
  }
</style>
