<script>
  // The chat view: a rendering of the conversation the server owns, plus a
  // live SSE feed of the run in flight. Sending during a run steers it —
  // the server folds the text into the tool-results turn.
  //
  // Voice rides the voice arc's module (scripts/voice/page/voice-core.js) —
  // imported by relative path so this wrapper cannot drift from the
  // standalone page. Until process unification, a call is its own facade
  // conversation, and the overlay says so rather than pretending otherwise.
  import { createVoiceSession } from '../../../scripts/voice/page/voice-core.js';

  let key = $state('main');
  let rail = $state([]);
  let entries = $state([]);
  let streaming = $state('');
  let running = $state(false);
  let taint = $state(null);
  let usage = $state(null);
  let model = $state('');
  let draft = $state('');
  let error = $state(null);
  let transcriptEl = $state(null);

  // Interim voice-out: the browser's own synthesis reads replies aloud when
  // toggled. Deliberately a stopgap — the real voice mode (Pipecat, the
  // chosen launch voice, barge-in) replaces this when the speech servers
  // land; until then it is the fail-to-a-lesser-mode shape, and marked so.
  let speak = $state(false);

  function speakText(text) {
    if (!speak || !('speechSynthesis' in window)) return;
    const utterance = new SpeechSynthesisUtterance(text);
    utterance.rate = 1.05;
    speechSynthesis.speak(utterance);
  }

  function toggleSpeak() {
    speak = !speak;
    if (!speak) speechSynthesis?.cancel();
  }

  function pushEntry(entry) {
    flushStreaming();
    entries.push(entry);
    scrollDown();
  }

  function flushStreaming() {
    if (streaming.trim()) {
      entries.push({ kind: 'assistant', text: streaming });
      speakText(streaming);
    }
    streaming = '';
  }

  function scrollDown() {
    queueMicrotask(() => {
      transcriptEl?.scrollTo({ top: transcriptEl.scrollHeight });
    });
  }

  async function load() {
    try {
      const res = await fetch(`/api/chat/${key}`);
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${(await res.text()).trim()}`);
      const data = await res.json();
      entries = data.entries.map((e) =>
        e.kind === 'tool' ? { ...e, pending: false } : e
      );
      running = data.running;
      taint = data.taint;
      model = data.model;
      if (data.usage?.prompt_tokens) {
        usage = { prompt: data.usage.prompt_tokens, window: data.usage.context_window };
      }
      error = null;
      scrollDown();
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  function subscribe() {
    // tailscale serve injects the identity header on this request too —
    // EventSource cannot set headers, and never needs to here.
    const source = new EventSource(`/api/chat/${key}/events`);
    source.onmessage = (raw) => {
      const ev = JSON.parse(raw.data);
      switch (ev.type) {
        case 'delta':
          streaming += ev.text;
          scrollDown();
          break;
        case 'queued':
          pushEntry({ kind: 'user', text: ev.text, queued: true });
          break;
        case 'tool':
          pushEntry({ kind: 'tool', name: ev.name, pending: true });
          break;
        case 'tool_result': {
          const open = entries.findLast((e) => e.kind === 'tool' && e.pending);
          if (open) {
            open.pending = false;
            open.is_error = ev.is_error;
          }
          break;
        }
        case 'denied':
          pushEntry({ kind: 'tool', name: ev.name, blocked: true, pending: false });
          break;
        case 'usage':
          usage = { prompt: ev.prompt_tokens, window: ev.context_window };
          break;
        case 'notice':
          pushEntry({ kind: 'notice', text: ev.text });
          break;
        case 'done':
          flushStreaming();
          running = false;
          taint = { private: ev.taint_private, untrusted: ev.taint_untrusted };
          if (!ev.ok && ev.error) pushEntry({ kind: 'notice', text: ev.error });
          entries = entries.map((e) =>
            e.kind === 'tool' && e.pending ? { ...e, pending: false } : e
          );
          break;
      }
    };
    return source;
  }

  async function loadRail() {
    try {
      const res = await fetch('/api/sessions');
      if (res.ok) rail = (await res.json()).sessions;
    } catch {
      // the rail is a convenience; the transcript is the truth
    }
  }

  function switchTo(k) {
    if (k === key) return;
    key = k;
    entries = [];
    streaming = '';
    usage = null;
    taint = null;
  }

  function newSession() {
    const name = prompt('Session name (lowercase, dashes):');
    if (!name) return;
    const k = name.trim().toLowerCase().replace(/[^a-z0-9_-]/g, '-').slice(0, 32);
    if (k) switchTo(k);
  }

  // Re-subscribe whenever the key changes; the server owns every
  // conversation, so switching is just pointing the rendering elsewhere.
  $effect(() => {
    const source = subscribe();
    load();
    loadRail();
    return () => source.close();
  });
  const railTimer = setInterval(loadRail, 20_000);
  $effect(() => () => clearInterval(railTimer));

  // ---- voice call (overlay over this view) ----
  let voiceOpen = $state(false);
  let vState = $state({ name: 'idle', label: 'connecting' });
  let vEntries = $state([]);
  let vLinked = $state(false);
  let vLevel = $state(0);
  let vSession = null;
  let voicePane = $state(null);

  function vScroll() {
    queueMicrotask(() => voicePane?.scrollTo({ top: voicePane.scrollHeight }));
  }

  function onTranscript({ who, text, interim }) {
    const last = vEntries.at(-1);
    if (last && last.who === who && last.interim) {
      last.text = text;
      last.interim = interim;
    } else {
      vEntries.push({ who, text, interim });
    }
    vScroll();
  }

  function startVoice() {
    // connect() inside the tap handler — the audio unlock needs the gesture.
    vEntries = [];
    vState = { name: 'connecting', label: 'connecting' };
    vSession = createVoiceSession({
      // Same-origin: serve proxies to the loopback runner, so the offer
      // rides the owner guard and no cross-origin fetch exists to fail.
      offerUrl: '/api/offer',
      onState: (name, label) => (vState = { name, label }),
      onTranscript,
      onLevel: (level) => (vLevel = level),
      onLink: (live) => {
        vLinked = live;
        if (!live && voiceOpen) {
          vState = { name: 'idle', label: 'line dropped' };
        }
      },
      onBotTurnEnd: () => {},
    });
    voiceOpen = true;
    vSession.connect().catch((e) => {
      vState = { name: 'idle', label: `could not connect: ${e?.message ?? e}` };
    });
  }

  let vMuted = $state(false);
  function toggleMute() {
    if (!vSession) return;
    vMuted = !vMuted;
    vSession.setMicEnabled(!vMuted);
  }

  function endVoice() {
    try {
      vSession?.end();
    } finally {
      vSession = null;
      voiceOpen = false;
      vLevel = 0;
    }
  }

  $effect(() => () => vSession?.end());

  async function send() {
    const text = draft.trim();
    if (!text) return;
    draft = '';
    try {
      const res = await fetch(`/api/chat/${key}/send`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ text }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      if (data.started) {
        pushEntry({ kind: 'user', text });
        running = true;
      } else if (data.steered) {
        pushEntry({ kind: 'user', text, queued: true });
      }
    } catch (e) {
      pushEntry({ kind: 'notice', text: `send failed: ${e?.message ?? e}` });
    }
  }

  async function cancel() {
    try {
      await fetch(`/api/chat/${key}/cancel`, { method: 'POST' });
    } catch {
      // The done event reports the real outcome either way.
    }
  }

  const pct = $derived(
    usage?.window ? Math.min(100, Math.round((usage.prompt / usage.window) * 100)) : null
  );
  const fmt = (n) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
</script>

<div class="chat">
  <header>
    <span class="title">Chat</span>
    <div class="meta">
      {#if taint?.untrusted || taint?.private}
        <span
          class="chip taint"
          title="what this conversation has touched decides what it may still do"
        >
          {taint.private ? 'private' : ''}{taint.private && taint.untrusted ? ' + ' : ''}{taint.untrusted ? 'untrusted' : ''}
        </span>
      {/if}
      <span class="chip">{model || '…'}</span>
      <button
        class="speakbtn"
        class:on={speak}
        onclick={toggleSpeak}
        title="read replies aloud (interim browser voice)"
      >
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4 10v4h4l5 4V6L8 10z" />
          {#if speak}<path d="M16 9a4 4 0 010 6M18.5 6.5a8 8 0 010 11" />{/if}
        </svg>
      </button>
    </div>
  </header>

  <div class="rail">
    {#each rail.length ? rail : [{ key: 'main', running: false }] as s}
      <button class="railchip" class:active={s.key === key} onclick={() => switchTo(s.key)}>
        <span class="raildot" class:on={s.running}></span>
        {s.key}
        {#if s.taint?.untrusted}<span class="railtaint">▲</span>{/if}
      </button>
    {/each}
    <button class="railchip plus" onclick={newSession} title="new session">
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M12 5v14M5 12h14" /></svg>
    </button>
  </div>

  <div class="transcript" bind:this={transcriptEl}>
    {#if error}
      <div class="notice">{error}</div>
    {/if}
    {#each entries as entry}
      {#if entry.kind === 'user'}
        <div class="bubble" class:queued={entry.queued}>
          {entry.text}
          {#if entry.queued}<span class="queued-tag">steered</span>{/if}
        </div>
      {:else if entry.kind === 'assistant'}
        <div class="answer">{entry.text}</div>
      {:else if entry.kind === 'tool'}
        <div class="tool" class:err={entry.is_error} class:blocked={entry.blocked}>
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6" /></svg>
          <span>{entry.name}</span>
          {#if entry.pending}<span class="tool-state">running…</span>
          {:else if entry.blocked}<span class="tool-state">blocked (read-only)</span>
          {:else if entry.is_error}<span class="tool-state">failed</span>{/if}
        </div>
      {:else if entry.kind === 'notice'}
        <div class="notice">{entry.text}</div>
      {/if}
    {/each}
    {#if streaming}
      <div class="answer">{streaming}</div>
    {/if}
    {#if running && !streaming}
      <div class="thinking">
        <span class="dot"></span><span class="dot d2"></span><span class="dot d3"></span>
      </div>
    {/if}
  </div>

  <footer>
    {#if usage}
      <div class="gauge-row">
        <div class="gauge">
          <div
            class="fill"
            style:width="{pct ?? 0}%"
            style:background={pct !== null && pct >= 75 ? 'var(--hazard)' : 'var(--accent-400)'}
          ></div>
        </div>
        <span class="gauge-label">
          context {fmt(usage.prompt)}{usage.window ? ` / ${fmt(usage.window)}` : ''}
        </span>
      </div>
    {/if}
    <div class="input-row">
      <textarea
        rows="1"
        placeholder={running ? 'Steer the run…' : 'Ask mecha…'}
        bind:value={draft}
        onkeydown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            send();
          }
        }}
      ></textarea>
      <button
        class="round voice"
        onclick={startVoice}
        title="start a voice call (its own session until unification)"
      >
        <svg viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="var(--accent-400)" stroke-width="1.8" stroke-linecap="round"><path d="M4 10v4M8 7v10M12 4v16M16 7v10M20 10v4" /></svg>
      </button>
      {#if running}
        <button class="round stop" onclick={cancel} title="stop at the next safe point">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round"><rect x="7" y="7" width="10" height="10" rx="1.5" /></svg>
        </button>
      {/if}
      <button class="round send" onclick={send} title="send">
        <svg viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="var(--void)" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V5M6 11l6-6 6 6" /></svg>
      </button>
    </div>
  </footer>

  {#if voiceOpen}
    <div class="voice-overlay">
      <div class="voice-top">
        <span class="chip">grant-review is not this call — a voice call is its own session for now</span>
      </div>
      <div class="voice-stage">
        <svg viewBox="0 0 63 54" width="112" height="96" role="img" aria-label="mecha {vState.label}">
          <g fill="var(--accent-700)">
            <path d="M0 0h24l7.5 8.5L39 0h24v16H0z" />
            <path d="M0 20h14v15H0zM49 20h14v15H49zM0 39h14v15H0zM49 39h14v15H49z" />
            <path d="M14 39v15h13.24zM49 39v15H35.76z" />
          </g>
          <path
            d="M21 24h21v7H21z"
            class="slot {vState.name}"
            style:opacity={vState.name === 'listening' ? 0.7 + vLevel * 0.3 : 1}
          />
        </svg>
        <div class="voice-state">
          <span class="vdot" class:live={vLinked}></span>
          <span>{vState.label}</span>
        </div>
      </div>
      <div class="voice-pane" bind:this={voicePane}>
        {#each vEntries as entry}
          <div class="vline">
            <span class="vwho">{entry.who === 'user' ? 'you' : 'mecha'}</span>
            <span class="vtext" class:interim={entry.interim}>{entry.text}</span>
          </div>
        {/each}
      </div>
      <div class="voice-controls">
        <button class="mutebtn" class:muted={vMuted} onclick={toggleMute} title={vMuted ? 'unmute' : 'mute'}>
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="3" width="6" height="11" rx="3" />
            <path d="M5 11a7 7 0 0014 0M12 18v3" />
            {#if vMuted}<path d="M4 4l16 16" />{/if}
          </svg>
        </button>
        <button class="endcall" onclick={endVoice} title="end the call">
          <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="var(--hazard)" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M6 6l12 12M18 6L6 18" /></svg>
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .chat {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 22px 20px 6px;
  }
  .title {
    font-weight: 500;
    font-size: 17px;
    letter-spacing: -0.02em;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .chip.taint {
    color: var(--hazard);
  }
  .speakbtn {
    background: none;
    border: none;
    color: var(--text-muted);
    min-width: 44px;
    min-height: 44px;
    margin: -12px -12px -12px 0;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }
  .speakbtn.on {
    color: var(--accent-400);
  }
  .rail {
    display: flex;
    gap: 8px;
    padding: 10px 20px 12px;
    overflow-x: auto;
    border-bottom: 1px solid var(--accent-900);
  }
  .railchip {
    display: flex;
    align-items: center;
    gap: 6px;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-muted);
    background: var(--bg);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius-chip);
    padding: 8px 12px;
    min-height: 40px;
    cursor: pointer;
    white-space: nowrap;
  }
  .railchip.active {
    color: var(--text);
    background: var(--accent-900);
    border-color: var(--accent-700);
  }
  .railchip.plus {
    min-width: 44px;
    justify-content: center;
  }
  .raildot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--accent-900);
  }
  .raildot.on {
    background: var(--accent-400);
  }
  .railtaint {
    color: var(--hazard);
    font-size: 9px;
  }
  .transcript {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .bubble {
    align-self: flex-end;
    max-width: 82%;
    background: var(--surface);
    border-radius: var(--radius);
    padding: 11px 14px;
    font-size: 14px;
    line-height: 1.45;
    white-space: pre-wrap;
  }
  .bubble.queued {
    border: 1px solid var(--accent-700);
    background: var(--bg);
  }
  .queued-tag {
    display: block;
    margin-top: 4px;
    font-family: var(--mono);
    font-size: 9px;
    color: var(--text-muted);
  }
  .answer {
    max-width: 92%;
    font-size: 14px;
    line-height: 1.5;
    white-space: pre-wrap;
  }
  .tool {
    display: flex;
    align-items: center;
    gap: 7px;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-muted);
  }
  .tool svg {
    color: var(--accent-700);
  }
  .tool-state {
    font-size: 11px;
    color: var(--accent-700);
  }
  .tool.err .tool-state,
  .tool.blocked .tool-state {
    color: var(--hazard);
  }
  .notice {
    font-size: 12px;
    color: var(--hazard);
    display: flex;
    gap: 8px;
  }
  .thinking {
    display: flex;
    gap: 5px;
    padding: 4px 2px;
  }
  .dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--accent-400);
    animation: pulse 1.2s infinite;
  }
  .d2 {
    animation-delay: 0.2s;
    background: var(--accent-500);
  }
  .d3 {
    animation-delay: 0.4s;
    background: var(--accent-700);
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 0.35;
    }
    50% {
      opacity: 1;
    }
  }
  footer {
    border-top: 1px solid var(--accent-900);
    background: var(--bg);
    padding: 10px 14px 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .gauge-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 4px;
  }
  .gauge {
    flex: 1;
    height: 3px;
    background: var(--accent-900);
    border-radius: 2px;
    overflow: hidden;
  }
  .fill {
    height: 3px;
  }
  .gauge-label {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--text-muted);
  }
  .input-row {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }
  textarea {
    flex: 1;
    min-height: 44px;
    max-height: 130px;
    background: var(--surface);
    border: none;
    border-radius: var(--radius);
    padding: 12px 14px;
    color: var(--text);
    font-family: var(--sans);
    font-size: 16px;
    resize: none;
  }
  textarea:focus {
    outline: 1px solid var(--accent-500);
  }
  .round {
    width: 44px;
    height: 44px;
    border-radius: var(--radius);
    border: none;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }
  .send {
    background: var(--accent-400);
  }
  .stop {
    background: var(--surface);
    color: var(--hazard);
  }
  .voice {
    background: var(--surface);
    border: 1px solid var(--accent-700);
  }
  .voice-overlay {
    position: absolute;
    inset: 0;
    background: var(--void);
    display: flex;
    flex-direction: column;
    z-index: 5;
  }
  .voice-top {
    display: flex;
    justify-content: center;
    padding: 22px 20px 0;
  }
  .voice-stage {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 18px;
  }
  .slot {
    fill: var(--accent-700);
  }
  .slot.listening {
    fill: var(--accent-400);
  }
  .slot.thinking {
    fill: var(--accent-500);
    animation: slotpulse 1.1s infinite;
  }
  .slot.speaking {
    fill: var(--accent-300);
  }
  @keyframes slotpulse {
    0%,
    100% {
      opacity: 0.45;
    }
    50% {
      opacity: 1;
    }
  }
  .voice-state {
    display: flex;
    align-items: center;
    gap: 8px;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-muted);
  }
  .vdot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--accent-900);
  }
  .vdot.live {
    background: var(--accent-400);
  }
  .voice-pane {
    max-height: 34%;
    overflow-y: auto;
    margin: 0 20px;
    background: var(--bg);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius);
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .vline {
    display: flex;
    gap: 10px;
  }
  .vwho {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--accent-700);
    min-width: 40px;
    padding-top: 2px;
    flex-shrink: 0;
  }
  .vtext {
    font-size: 13px;
    line-height: 1.5;
  }
  .vtext.interim {
    color: var(--text-muted);
  }
  .voice-controls {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 24px;
    padding: 24px 0 34px;
  }
  .mutebtn {
    width: 56px;
    height: 56px;
    border-radius: 14px;
    background: var(--surface);
    border: 1px solid var(--accent-900);
    color: var(--text);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }
  .mutebtn.muted {
    color: var(--hazard);
    border-color: var(--accent-700);
  }
  .endcall {
    width: 68px;
    height: 68px;
    border-radius: 16px;
    background: var(--surface);
    border: 1px solid var(--accent-700);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }
</style>
