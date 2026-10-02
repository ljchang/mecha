<script>
  // A call with a persona (PERSONA-DESIGN.md §11): the assistant's call
  // surface, pared to what a call needs, speaking into this persona chat.
  //
  // What differs from the assistant's, and why:
  // - The voice is the persona's, bound by serve at the offer — never the
  //   listener's remembered one, which is neither sent nor overwritten
  //   (`rememberVoice: false`).
  // - The offer carries the unlock token for a locked persona; serve checks
  //   and strips it before the worker hears the call exists.
  // - When a call ends its length goes to the dose meter (§12.3): voice
  //   raises attachment, so the meters count call minutes, not only turns.
  //
  // Mounted while a chat is open and idle until `start()`, which the call
  // button invokes inside its own tap: the audio unlock needs the gesture.
  import { onDestroy } from 'svelte';
  import { apiFetch as fetch } from './api.js';
  import { createVoiceSession } from '../../../scripts/voice/voice-core.js';
  import { chatUrl, hangUpReport } from './persona.js';

  let { chatKey, token = null, display, face = null, onended = () => {} } = $props();

  let open = $state(false);
  let session = null;
  let callKey = null;
  let callToken = null;
  // The binding serve made for this connection (the offer's answer names
  // it): the hang-up releases exactly this one, never another tab's call.
  let callId = null;
  let callState = $state({ name: 'idle', label: '' });
  let level = $state(0);
  let linked = $state(false);
  let muted = $state(false);
  let entries = $state([]);
  let pane = $state(null);
  // When the line first carried the call: what the meter counts from, so a
  // call that never connected is no minutes at all.
  let since = null;

  function onTranscript({ who, text, interim }) {
    const last = entries.at(-1);
    if (last && last.who === who && last.interim) {
      last.text = text;
      last.interim = interim;
    } else {
      entries.push({ who, text, interim });
    }
    requestAnimationFrame(() => pane && (pane.scrollTop = pane.scrollHeight));
  }

  export function start({ keep = false } = {}) {
    if (!keep) entries = [];
    // A line typed into another persona's call must not wait in this one's.
    if (!keep) typed = '';
    // Read at the tap, never bound: a chat switched mid-call must not have
    // the words being spoken redirected into it.
    callKey = chatKey;
    callToken = token;
    callId = null;
    open = true;
    muted = false;
    callState = { name: 'connecting', label: 'connecting' };
    session = createVoiceSession({
      offerUrl: '/api/offer',
      offerHeaders: { 'X-Mecha-Request': '1' },
      sessionKey: callKey,
      offerExtra: callToken ? { unlock: callToken } : null,
      rememberVoice: false,
      onAnswer: (answer) => (callId = Number.isInteger(answer?.call) ? answer.call : null),
      onState: (name, label) => {
        callState = { name, label };
        if (since == null && (name === 'listening' || name === 'speaking')) since = Date.now();
      },
      onTranscript,
      onLevel: (l) => (level = l),
      onLink: (live) => {
        linked = live;
        if (!live && open) {
          count();
          callState = { name: 'idle', label: 'line dropped — tap to call again' };
        }
      },
      onBotTurnEnd: () => {},
    });
    session.connect().catch((e) => {
      callState = { name: 'idle', label: `could not connect: ${e?.message ?? e} — tap to try again` };
    });
  }

  // The call's length to the meter, and its binding let go — once per
  // connection, and for a call the worker took that never got past
  // connecting too (zero seconds), or serve would hold its binding and
  // unlock for the life of the process (review of #483). `keepalive` so a
  // closing tab still sends it.
  function count() {
    const report = callKey && hangUpReport({ since, callId, now: Date.now() });
    if (!report) return;
    since = null;
    fetch(chatUrl(callKey, '/call'), {
      method: 'POST',
      keepalive: true,
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ ...report, unlock: callToken ?? undefined }),
    }).catch(() => {});
    callId = null;
  }

  function stopSession() {
    try {
      session?.end();
    } catch {
      // Already gone is the usual reason to be here.
    }
    session = null;
  }

  export function end() {
    if (!open) return;
    count();
    stopSession();
    open = false;
    level = 0;
    linked = false;
    onended();
  }

  function redial() {
    if (callState.name !== 'idle') return;
    stopSession();
    start({ keep: true });
  }

  // Typing into the call (the owner's ask, 2026-10-01): a typed line is a
  // turn the persona answers aloud. The mic is paused while the box has
  // focus — keys and a room are not words — and given back as it was; the
  // mute button stays the owner's.
  let typed = $state('');
  let typing = $state(false);
  function typingStart() {
    typing = true;
    if (session && !muted) session.setMicEnabled(false);
  }
  function typingEnd() {
    typing = false;
    if (session && !muted) session.setMicEnabled(true);
  }
  function sendTyped() {
    if (session?.sendText(typed)) typed = '';
  }

  function toggleMute() {
    if (!session) return;
    muted = !muted;
    session.setMicEnabled(!muted);
  }

  // A different chat, or none: the call was this one's and ends with it.
  $effect(() => {
    if (open && chatKey !== callKey) end();
  });
  onDestroy(end);
  // A tab closing, reloading or navigating away runs no `onDestroy`: the
  // hang-up goes out on `pagehide`, which is what `keepalive` is for
  // (review of #483; `mail-queue.svelte.js` is the precedent).
  $effect(() => {
    window.addEventListener('pagehide', end);
    return () => window.removeEventListener('pagehide', end);
  });
</script>

{#if open}
  <div class="call" role="dialog" aria-label={`call with ${display}`}>
    <div class="call-top">
      <span class="chip">on a call with {display} — same chat</span>
    </div>
    <div class="call-stage">
      <button
        class="face {callState.name}"
        class:tappable={callState.name === 'idle'}
        disabled={callState.name !== 'idle'}
        onclick={redial}
        aria-label={callState.name === 'idle' ? 'call again' : `${display} ${callState.label}`}
      >
        {#if face}{@render face()}{/if}
      </button>
      <div class="call-state">
        <span class="vdot" class:live={linked}></span>
        <span>{callState.label}</span>
      </div>
      <div class="meter" class:paused={callState.name === 'paused'} title="your microphone, live">
        {#each Array(14) as _, i}
          <span class="tick" class:lit={level * 14 > i} style:height="{14 + (i % 2 ? 6 : 0)}px"></span>
        {/each}
      </div>
    </div>
    <div class="call-pane" bind:this={pane}>
      {#each entries as entry}
        <div class={entry.who === 'user' ? 'said' : 'heard'} class:interim={entry.interim}>{entry.text}</div>
      {/each}
    </div>
    <form class="typerow" onsubmit={(e) => { e.preventDefault(); sendTyped(); }}>
      <input
        class="typebox"
        placeholder={`Type to ${display}`}
        aria-label={`Type to ${display}`}
        bind:value={typed}
        onfocus={typingStart}
        onblur={typingEnd}
        disabled={!linked}
        maxlength="4000"
      />
      <button type="submit" class="typesend" aria-label="send" disabled={!linked || !typed.trim()}>
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 19V5M6 11l6-6 6 6" /></svg>
      </button>
    </form>
    {#if typing && !muted}<div class="typehint">mic paused while you type</div>{/if}
    <div class="call-controls">
      <button class="mutebtn" class:muted onclick={toggleMute} title={muted ? 'unmute' : 'mute'} aria-label={muted ? 'unmute' : 'mute'}>
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
          <rect x="9" y="3" width="6" height="11" rx="3" />
          <path d="M5 11a7 7 0 0014 0M12 18v3" />
          {#if muted}<path d="M4 4l16 16" />{/if}
        </svg>
      </button>
      <button class="endcall" onclick={end} title="end the call" aria-label="end the call">
        <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="var(--hazard)" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M6 6l12 12M18 6L6 18" /></svg>
      </button>
    </div>
  </div>
{/if}

<style>
  .call {
    position: absolute;
    inset: 0;
    background: var(--void);
    display: flex;
    flex-direction: column;
    z-index: 6;
  }
  .call-top {
    display: flex;
    justify-content: center;
    padding: 22px 16px 0;
  }
  .chip {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-muted);
    border: 1px solid var(--accent-900);
    border-radius: 999px;
    padding: 4px 10px;
    text-align: center;
  }
  .call-stage {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 18px;
  }
  /* The persona's face is the call's state: its ring says listening,
     thinking or speaking, as the assistant's logo slot does. */
  .face {
    padding: 0;
    border: 2px solid var(--accent-900);
    border-radius: 50%;
    background: none;
    line-height: 0;
    transition: border-color 120ms linear;
  }
  .face.listening {
    border-color: var(--accent-400);
  }
  .face.thinking {
    border-color: var(--accent-500);
    animation: ring 1.1s infinite;
  }
  .face.speaking {
    border-color: var(--accent-300);
  }
  .face.tappable {
    cursor: pointer;
  }
  @keyframes ring {
    0%,
    100% {
      opacity: 0.55;
    }
    50% {
      opacity: 1;
    }
  }
  .call-state {
    display: flex;
    align-items: center;
    gap: 8px;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-muted);
  }
  .meter {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 24px;
  }
  .meter.paused {
    opacity: 0.3;
  }
  .tick {
    width: 3px;
    border-radius: 1px;
    background: var(--accent-900);
    transition: background 60ms linear;
  }
  .tick.lit {
    background: var(--accent-400);
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
  .call-pane {
    max-height: 34%;
    overflow-y: auto;
    margin: 0 16px;
    background: var(--bg);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius);
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .call-pane:empty {
    display: none;
  }
  .said {
    align-self: flex-end;
    max-width: 84%;
    background: var(--surface);
    border-radius: var(--radius);
    padding: 9px 12px;
    font-size: 13px;
    line-height: 1.5;
  }
  .heard {
    align-self: flex-start;
    max-width: 92%;
    font-size: 13px;
    line-height: 1.5;
  }
  .interim {
    color: var(--text-muted);
  }
  .typerow {
    display: flex;
    gap: 8px;
    margin: 14px 16px 0;
  }
  .typebox {
    flex: 1;
    min-width: 0;
    min-height: 44px;
    padding: 0 14px;
    border-radius: 22px;
    border: 1px solid var(--accent-900);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 16px;
  }
  .typebox:focus {
    outline: none;
    border-color: var(--accent-500);
  }
  .typesend {
    flex-shrink: 0;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--accent-400);
    color: var(--void);
    border: none;
    cursor: pointer;
  }
  .typesend:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .typehint {
    margin: 6px 16px 0;
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-muted);
  }
  .call-controls {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 24px;
    padding: 16px 0 34px;
  }
  .mutebtn,
  .endcall {
    width: 56px;
    height: 56px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--surface);
    border: 1px solid var(--accent-900);
    color: var(--text);
    cursor: pointer;
  }
  .mutebtn.muted {
    border-color: var(--accent-400);
  }
  @media (prefers-reduced-motion: reduce) {
    .face.thinking {
      animation: none;
      border-color: var(--accent-300);
    }
  }
</style>
