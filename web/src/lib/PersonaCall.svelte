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
  // - The pictures the persona makes during the call are shown here. This
  //   screen covers the chat, where they are also drawn, so without this
  //   they stayed out of sight until the owner hung up (2026-10-03).
  //   Tapped, one opens full screen *inside* this screen, with the chat's
  //   Download and Edit — never a new tab: on a phone a tab sends the call
  //   page to the background, its microphone stream dies and the call
  //   drops, which is what three of the seven drops on 2026-10-04 were.
  //
  // Mounted while a chat is open and idle until `start()`, which the call
  // button invokes inside its own tap: the audio unlock needs the gesture.
  import { onDestroy } from 'svelte';
  import { apiFetch as fetch } from './api.js';
  import { createVoiceSession } from '../../../scripts/voice/voice-core.js';
  import { chatUrl, hangUpReport } from './persona.js';

  let {
    chatKey,
    token = null,
    display,
    face = null,
    onended = () => {},
    // Made since the call was placed, oldest first (`picture.js`
    // `picturesSince`), with the chat's own URL for each.
    pictures = [],
    pictureUrl = (path) => path,
    // The chat's own Download (`path` → why it failed, or null) and Edit
    // (`path` → the chat opens its edit modal over this screen, and sends
    // the edit as a line typed into the call: `say`). Null hides the button.
    ondownload = null,
    onedit = null,
    // The conversation before the call (`persona.js` `historyLines`), drawn
    // above the call's own lines: a call continues the chat, and the owner
    // reads back through it here as in the chat (the owner's ask,
    // 2026-10-05).
    history = [],
  } = $props();

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
  // Opened at the bottom: the call starts where it always has, with the
  // conversation before it a scroll up.
  $effect(() => {
    if (open && pane && history.length) requestAnimationFrame(() => pane && (pane.scrollTop = pane.scrollHeight));
  });
  // The picture shown large: the newest, unless the owner tapped an earlier
  // one — and a new picture takes the stage again when it arrives.
  let picked = $state(null);
  let seen = 0;
  $effect(() => {
    if (pictures.length !== seen) {
      seen = pictures.length;
      picked = null;
    }
  });
  const shown = $derived(picked && pictures.includes(picked) ? picked : (pictures.at(-1) ?? null));
  // The picture open full screen, inside the call; and why its download
  // failed, when it did.
  let viewing = $state(null);
  let viewNote = $state(null);
  function view(path) {
    viewing = path;
    viewNote = null;
  }
  // Focus goes into the viewer when it opens and back to the picture when
  // it closes, so a keyboard owner is never left on what it covers, or on
  // nothing.
  let shotButton = $state(null);
  let backButton = $state(null);
  let wasViewing = false;
  $effect(() => {
    if (viewing && !wasViewing) backButton?.focus();
    else if (!viewing && wasViewing) shotButton?.focus();
    wasViewing = !!viewing;
  });
  async function download() {
    const path = viewing;
    const why = await ondownload?.(path);
    if (viewing === path) viewNote = why || null;
  }
  function edit() {
    const path = viewing;
    viewing = null;
    onedit?.(path);
  }
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
    if (!keep) {
      typed = '';
      typing = false;
      viewing = null;
    }
    // No hold outlives the call that took it. Unreachable today — the edit
    // modal's scrim covers the call button, and leaving the chat closes the
    // modal — but a fresh call never starts with its mic paused behind a
    // modal nobody can see (review of #552).
    away = false;
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
          // Never a picture over a dropped line: the call's own state, and
          // its redial, must be what the owner sees (review of #552).
          viewing = null;
          callState = { name: 'idle', label: 'line dropped — tap to call again' };
        }
      },
      onBotTurnEnd: () => {},
    });
    // A redial keeps `typing`, and a fresh session's mic starts live: the
    // track is set from mute and typing once it exists (review of #499,
    // pass 7 - the same as the assistant's call).
    session
      .connect()
      .then(applyMic)
      .catch((e) => {
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
    applyMic();
  }
  function typingEnd() {
    typing = false;
    applyMic();
  }
  // Both at once, wherever either changes: the hint is a privacy claim and
  // must not rest on focus leaving the box when mute is tapped (review of
  // #499, pass 5).
  function applyMic() {
    session?.setMicEnabled(!muted && !typing && !away);
  }
  // Typing somewhere else for the call — the chat's edit modal, open over
  // this screen: the mic pauses as it does for the call's own box.
  let away = $state(false);
  export function holdMic(on) {
    away = on;
    applyMic();
  }
  // A line typed into the call from outside it (an edit): sent as the box
  // sends one, and false when there is no live line to carry it.
  export function say(text) {
    return !!(linked && session?.sendText(text));
  }
  function sendTyped() {
    if (session?.sendText(typed)) typed = '';
  }

  function toggleMute() {
    if (!session) return;
    muted = !muted;
    applyMic();
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

<svelte:window onkeydown={(e) => viewing && e.key === 'Escape' && (viewing = null)} />

{#if open}
  <div class="call" role="dialog" aria-label={`call with ${display}`}>
    <div class="call-top" inert={!!viewing}>
      <span class="chip">on a call with {display} — same chat</span>
    </div>
    <div class="call-stage" inert={!!viewing}>
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
      {#if shown}
        <div class="shot">
          <button class="shotbtn" bind:this={shotButton} onclick={() => view(shown)} aria-label="look at the picture full screen">
            <img src={pictureUrl(shown)} alt="made during the call" />
          </button>
        </div>
        {#if pictures.length > 1}
          <div class="thumbs" aria-label="pictures from this call">
            {#each pictures as picture (picture)}
              <button class="thumb" class:on={picture === shown} onclick={() => (picked = picture)} aria-label="show this picture">
                <img src={pictureUrl(picture)} alt="" loading="lazy" />
              </button>
            {/each}
          </div>
        {/if}
      {/if}
    </div>
    <div class="call-pane" bind:this={pane} inert={!!viewing}>
      {#if history.length}
        {#each history as line}
          <div class={['past', line.who === 'user' ? 'said' : 'heard']} class:pictured={line.picture}>{line.text}</div>
        {/each}
        <div class="call-start" role="separator" aria-label="Call">Call</div>
      {/if}
      {#each entries as entry}
        <div class={entry.who === 'user' ? 'said' : 'heard'} class:interim={entry.interim}>{entry.text}</div>
      {/each}
    </div>
    <form class="typerow" inert={!!viewing} onsubmit={(e) => { e.preventDefault(); sendTyped(); }}>
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
    {#if (typing || away) && !muted}<div class="typehint">mic paused while you type</div>{/if}
    <div class="call-controls" inert={!!viewing}>
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
    {#if viewing}
      <!-- Full screen inside the call, so the call keeps its page. -->
      <!-- Modal for focus as well as for the eye: what it covers is inert, so
           Tab cannot reach the call's typing box (which pauses the mic under
           a hint the viewer hides) (review of #552). -->
      <div class="viewer" role="dialog" aria-modal="true" aria-label="picture from the call">
        <img src={pictureUrl(viewing)} alt="made during the call" />
        {#if viewNote}<div class="viewnote">could not download: {viewNote}</div>{/if}
        <div class="viewbar">
          <!-- An edit is a call turn: offered only while there is a line to
               carry it, as the call's own typing box is (review of #552). -->
          {#if onedit}<button class="viewbtn" onclick={edit} disabled={!linked}>Edit</button>{/if}
          {#if ondownload}<button class="viewbtn" onclick={download}>Download</button>{/if}
          <button class="viewbtn" bind:this={backButton} onclick={() => (viewing = null)}>Back to the call</button>
          <!-- The hang-up stays one tap away while a picture is open. -->
          <button class="viewbtn viewend" onclick={end}>End call</button>
        </div>
      </div>
    {/if}
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
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 18px;
  }
  /* The newest picture takes what the stage has left, scaled to fit whole:
     on a phone it sits under the face, never pushing the controls away. */
  .shot {
    flex: 1 1 0;
    min-height: 96px;
    width: 100%;
    padding: 0 16px;
    box-sizing: border-box;
    display: flex;
    justify-content: center;
    /* Not stretched: the picture keeps its own shape, and its border with it. */
    align-items: center;
  }
  /* A real box, not `display: contents`: browsers do not agree on unboxing a
     button, and an unboxed one leaves the accessibility tree (review of #552). */
  .shotbtn {
    display: flex;
    /* Stretched, so the picture's max-height resolves against the slot as it
       did under the old link: a tall portrait stays inside the stage
       (measured headless, review of #552). */
    align-self: stretch;
    /* Centred on every engine: Chromium's button default is flex-start,
       which would lift a short picture to the top (review of #552). */
    align-items: center;
    justify-content: center;
    min-width: 0;
    min-height: 0;
    max-width: 100%;
    max-height: 100%;
    padding: 0;
    border: 0;
    background: none;
    cursor: zoom-in;
  }
  .viewer {
    position: absolute;
    inset: 0;
    z-index: 2;
    background: var(--void);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 16px;
    box-sizing: border-box;
  }
  .viewer img {
    flex: 1 1 0;
    min-height: 0;
    max-width: 100%;
    object-fit: contain;
    border-radius: var(--radius);
  }
  .viewnote {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--hazard);
  }
  .viewbar {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }
  .viewbtn {
    min-height: 44px;
    padding: 0 16px;
    border-radius: 999px;
    border: 1px solid var(--accent-900);
    background: transparent;
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  .viewbtn:disabled {
    opacity: 0.4;
  }
  .viewend {
    border-color: var(--hazard);
    color: var(--hazard);
  }
  .shot img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border-radius: var(--radius);
    border: 1px solid var(--accent-900);
  }
  .thumbs {
    display: flex;
    gap: 6px;
    max-width: 100%;
    overflow-x: auto;
    padding: 0 16px;
    box-sizing: border-box;
  }
  .thumb {
    flex: 0 0 auto;
    width: 44px;
    height: 44px;
    padding: 0;
    border: 1px solid var(--accent-900);
    border-radius: 6px;
    background: none;
    overflow: hidden;
    cursor: pointer;
  }
  .thumb.on {
    border-color: var(--accent-400);
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
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
  /* Before the call: the same bubbles, quieter, and a picture as a line. */
  .past {
    opacity: 0.62;
  }
  .pictured {
    font-style: italic;
  }
  .call-start {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-muted);
    opacity: 0.8;
  }
  .call-start::before,
  .call-start::after {
    content: '';
    flex: 1;
    border-top: 1px solid var(--accent-900);
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
