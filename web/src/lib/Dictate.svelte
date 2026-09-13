<script>
  import { apiFetch as fetch } from './api.js';
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
  // The live level while recording, 0..1, drawn as bars inside the button.
  // Feedback that the audio graph is actually running: on 2026-09-12 the
  // button pulsed "recording" for clips that reached the server with no
  // samples in them (2 of 8), because nothing here checked that the
  // context had started - a phone suspends a fresh AudioContext until it is
  // resumed, and the pulse was driven by the permission grant, not by
  // audio. Bars that do not move are the missing signal.
  let level = $state(0);
  let peak = 0;
  let stream = null;
  let ctx = null;
  let node = null;
  let chunks = [];
  let sourceRate = 48000;

  // The same shape as the call's earcons (voice-core.js): synthesized,
  // never fetched, two soft notes up to say "listening", one down to say
  // "got it". Asked for after a drive on which there was no way to hear
  // whether a tap had taken - the eyes were on the road.
  function tone(freq, t0, dur, gain = 0.05) {
    const o = ctx.createOscillator(), g = ctx.createGain();
    o.type = 'sine'; o.frequency.value = freq;
    g.gain.setValueAtTime(0, t0);
    g.gain.linearRampToValueAtTime(gain, t0 + 0.02);
    g.gain.exponentialRampToValueAtTime(0.0001, t0 + dur);
    o.connect(g).connect(ctx.destination);
    o.start(t0); o.stop(t0 + dur + 0.05);
  }
  const listeningTone = () => { const t = ctx.currentTime; tone(659, t, 0.15); tone(880, t + 0.11, 0.22); };
  const gotItTone = () => { const t = ctx.currentTime; tone(659, t, 0.18); };

  async function start() {
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      ctx = new (window.AudioContext || window.webkitAudioContext)();
      // Explicit, and checked: a context created after the permission
      // prompt has resolved is outside the tap's activation window on some
      // phones and starts suspended, delivering no buffers at all. Resume
      // is the fix; refusing to record while it is not running is what
      // keeps the pulse honest.
      await ctx.resume();
      if (ctx.state !== 'running') throw new Error('the audio graph did not start — tap again');
      sourceRate = ctx.sampleRate;
      const source = ctx.createMediaStreamSource(stream);
      node = ctx.createScriptProcessor(4096, 1, 1);
      chunks = [];
      peak = 0;
      node.onaudioprocess = (e) => {
        const buf = new Float32Array(e.inputBuffer.getChannelData(0));
        chunks.push(buf);
        let sum = 0;
        for (let i = 0; i < buf.length; i++) sum += buf[i] * buf[i];
        const rms = Math.sqrt(sum / buf.length);
        peak = Math.max(peak, rms);
        // The call meter's curve: linear amplitude sits low for speech.
        level = Math.min(1, Math.sqrt(rms) * 2);
      };
      source.connect(node);
      node.connect(ctx.destination);
      state_ = 'recording';
      listeningTone();
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
    level = 0;
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

  // Shorter than this never reaches the server: the worker's own segment
  // gate (`MIN_SEGMENT_SECONDS`) drops the same, and a clip of no samples
  // at all crashed it. Said here as "heard nothing", which is what happened.
  const MIN_SECONDS = 0.3;

  async function stop() {
    state_ = 'transcribing';
    const total = chunks.reduce((n, c) => n + c.length, 0);
    const seconds = total / sourceRate;
    const captured = peak;
    // The refusals come first, and silently: "got it" before "heard
    // nothing" tells the ear the opposite of what happened, on exactly
    // the path the guard exists for (third review of #226).
    if (seconds < MIN_SECONDS) {
      cleanup();
      onText(null, total === 0
        ? 'heard nothing — the microphone delivered no audio (is the page allowed to use it?)'
        : 'heard nothing — tap, speak, then tap again');
      return;
    }
    if (captured < 0.001) {
      // Samples arrived and every one was zero: a muted track, which is
      // what a phone hands over when the screen is locked or another app
      // holds the microphone. Not worth a round trip to find out.
      cleanup();
      onText(null, 'heard only silence — is the microphone muted?');
      return;
    }
    gotItTone();
    const wav = encodeWav();
    // Let the tone finish before the context closes under it.
    await new Promise((r) => setTimeout(r, 220));
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
  {:else if state_ === 'recording'}
    <span class="bars" aria-hidden="true">
      {#each [0.15, 0.45, 0.8, 0.45, 0.15] as h}
        <span class="bar" style:height="{6 + (level * 14) * h}px"></span>
      {/each}
    </span>
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
  .bars {
    display: flex;
    align-items: center;
    gap: 3px;
    height: 20px;
  }
  .bar {
    width: 3px;
    border-radius: 1px;
    background: var(--hazard);
    transition: height 60ms linear;
  }
</style>
