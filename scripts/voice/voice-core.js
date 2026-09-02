/* voice-core.js — the embeddable heart of mecha's voice mode.
 *
 * Framework-agnostic on purpose, and it stays that way for a reason that
 * outlived its original one. It was extracted so a standalone page and the
 * tailnet app's in-chat voice mode could not drift; the page was retired on
 * 2026-08-25 (one door is clearer than two, and sharing a module never
 * stopped the two *shells* diverging - the voice controls had to be built
 * twice), so `Chat.svelte` is the only consumer today. The module keeps no
 * framework dependency anyway: this is the layer that must be portable if
 * voice is ever embedded anywhere else, and coupling it to Svelte would be
 * a decision made once and regretted at the next surface.
 * docs/VOICE-RESEARCH.md D7 governs the sounds; the RTVI event names come
 * from pipecat 1.7.
 *
 * Contract:
 *   const session = createVoiceSession({
 *     offerUrl,            // default "/api/offer"; serve proxies it to the
 *                          // loopback worker, so the offer rides the owner
 *                          // guard and no cross-origin fetch exists to fail
 *     sessionKey,          // optional: a conversation the host front-end
 *                          // owns, for the call to speak into rather than
 *                          // opening one of its own (D3). It rides the
 *                          // offer because that is the only message sent
 *                          // before the bot exists, and the bot is what
 *                          // has to know — the data channel opens too late
 *     onState,             // (name, label) — idle|connecting|listening|thinking|speaking
 *     onTranscript,        // ({who: "user"|"bot", text, interim})
 *     onLevel,             // (0..1) real mic level, for state rings
 *     onLink,              // (live: bool)
 *     onBotTurnEnd,        // () — the open bot utterance is complete
 *     onVoiceConfig,       // ({voices, voice, speed, range, refused})
 *   });
 *   await session.connect();   // user gesture required (audio unlock)
 *   session.end();             // graceful; abrupt loss fires the same chime
 *   session.setMicEnabled(on); // mute control: pauses the outbound track
 *                              // without ending the call; state is readable
 *                              // as session.micEnabled
 *   session.connected          // bool
 *   session.voiceConfig(patch) // {} reads, {voice,speed} sets; the reply
 *                              // always arrives via onVoiceConfig, so the
 *                              // UI renders the server's answer and never
 *                              // its own optimistic guess
 *
 * Sounds are synthesized, never fetched: the end chime's most important
 * trigger is the network dying, and a sound that must be downloaded
 * cannot play then.
 */
/* Remembered voice and speed.
 *
 * Per browser, never on the server: `LocalTTS` is deliberately
 * per-connection so one listener's choice cannot reach another's, and a
 * server-side default would take that property away to solve a problem
 * that is really "this browser forgot". Storage can throw outright in a
 * private window, so every access is guarded and a failure degrades to
 * the behaviour that existed before this: ask the server, take its answer.
 *
 * What is stored is the server's *reply*, never the request. That is the
 * same rule the UI already follows for rendering, and it buys the
 * self-healing case for free: a remembered voice whose file has since been
 * deleted is refused, the server answers with what it is actually speaking
 * as, and that is what gets written back. A stale preference therefore
 * survives exactly one connection. */
const PREFS_KEY = "mecha-voice-prefs";
/* Chat.svelte grew its own copy of this machinery under a *different*
 * spelling while claiming to share this one — a voice picked in a call was
 * saved where nothing else looked. That copy is gone (the pickers live on
 * the settings page now); this read honours what it wrote, once, so a
 * preference from before the merge is not silently lost. */
const LEGACY_PREFS_KEY = "mecha.voice.prefs";

function readStored() {
  try {
    const raw = localStorage.getItem(PREFS_KEY) ?? localStorage.getItem(LEGACY_PREFS_KEY);
    if (!raw) return {};
    const p = JSON.parse(raw), out = {};
    if (typeof p.voice === "string") out.voice = p.voice;
    if (typeof p.speed === "number") out.speed = p.speed;
    if (Array.isArray(p.voices) && p.voices.every(v => typeof v === "string")) out.voices = p.voices;
    if (typeof p.range?.min === "number" && typeof p.range?.max === "number") out.range = { min: p.range.min, max: p.range.max };
    return out;
  } catch { return {}; }
}

/* What a connection sends as its opening patch: the preference alone. The
 * cached voices/range are for a picker with no live call to ask, never
 * something to send back at a worker that owns both. */
function readPrefs() {
  const { voice, speed } = readStored();
  const out = {};
  if (voice !== undefined) out.voice = voice;
  if (speed !== undefined) out.speed = speed;
  return out;
}

/* Merge, never overwrite blind: a worker whose TTS list was unreachable
 * this call answers voices:null, and wiping the cached list over that
 * would cost the settings page its picker until the next healthy call. */
function writePrefs(d) {
  try {
    const out = readStored();
    if (typeof d?.voice === "string") out.voice = d.voice;
    if (typeof d?.speed === "number") out.speed = d.speed;
    if (Array.isArray(d?.voices) && d.voices.every(v => typeof v === "string")) out.voices = d.voices;
    if (typeof d?.range?.min === "number" && typeof d?.range?.max === "number") out.range = { min: d.range.min, max: d.range.max };
    localStorage.setItem(PREFS_KEY, JSON.stringify(out));
  } catch { /* private window, or storage disabled - not worth a failure */ }
}

/* The settings page's half: read everything remembered (preference plus the
 * last call's voices/range), and save a picked voice/speed without touching
 * the cache. The next call's opening patch carries the change - voice-core
 * itself sends readPrefs() the moment the data channel opens. */
export function readVoicePrefs() {
  return readStored();
}
export function writeVoicePrefs(patch) {
  writePrefs(patch ?? {});
}

export function createVoiceSession(opts = {}) {
  const cfg = {
    offerUrl: "/api/offer",
    sessionKey: null,
    onState: () => {},
    onTranscript: () => {},
    onLevel: () => {},
    onLink: () => {},
    onBotTurnEnd: () => {},
    onVoiceConfig: () => {},
    ...opts,
  };

  const AC = new (window.AudioContext || window.webkitAudioContext)();
  function tone(freq, t0, dur, gain = 0.08, type = "sine") {
    const o = AC.createOscillator(), g = AC.createGain();
    o.type = type; o.frequency.value = freq;
    g.gain.setValueAtTime(0, t0);
    g.gain.linearRampToValueAtTime(gain, t0 + 0.02);
    g.gain.exponentialRampToValueAtTime(0.0001, t0 + dur);
    o.connect(g).connect(AC.destination);
    o.start(t0); o.stop(t0 + dur + 0.05);
  }
  const chimeStart = () => { const t = AC.currentTime; tone(659, t, .18); tone(880, t + .12, .28); };
  const chimeEnd = () => { const t = AC.currentTime; tone(440, t, .22); tone(294, t + .16, .45); };
  // Thinking is a soft two-note pulse, not a tick: a slow attack removes the
  // percussive edge (the old triangle tick read as a metronome), the lowpass
  // keeps it warm, and the pair alternates rising/falling so a long wait
  // breathes instead of repeating one sound at you.
  function softTone(freq, t0, dur, gain) {
    const o = AC.createOscillator(), g = AC.createGain(), f = AC.createBiquadFilter();
    o.type = "sine"; o.frequency.value = freq;
    f.type = "lowpass"; f.frequency.value = 1100;
    g.gain.setValueAtTime(0, t0);
    g.gain.linearRampToValueAtTime(gain, t0 + 0.12);
    g.gain.exponentialRampToValueAtTime(0.0001, t0 + dur);
    o.connect(f).connect(g).connect(AC.destination);
    o.start(t0); o.stop(t0 + dur + 0.05);
  }
  let thinkTimer = null;
  function thinkingSound(on) {
    if (on && !thinkTimer) {
      let up = true;
      const pulse = () => {
        const t = AC.currentTime;
        const [a, b] = up ? [392, 494] : [494, 392];
        up = !up;
        softTone(a, t, 0.55, 0.022);
        softTone(b, t + 0.22, 0.65, 0.017);
      };
      pulse(); thinkTimer = setInterval(pulse, 1800);
    } else if (!on && thinkTimer) { clearInterval(thinkTimer); thinkTimer = null; }
  }

  let pc = null, dc = null, micStream = null, levelTimer = 0, ended = false;
  /* Is the bot audible right now? Held so a VAD edge caused by our own
     speaker cannot be rendered as the owner talking - see `onRtvi`. */
  let botSpeaking = false;

  /* The mic level, read from **WebRTC's own sender stats** rather than from
     a WebAudio tap on the microphone.

     This used to analyse a CLONE of the mic track, to dodge a known WebKit
     trap: echo cancellation is silently disabled on a getUserMedia track
     once WebAudio attaches to it, and a phone that hears its own speaker
     becomes a bot talking to itself (observed in production 2026-08-24 -
     transcripts attributed to the owner that were the TTS). But a clone is
     not a different microphone. It shares the source, so on the browsers
     where that trap is real the clone can disarm the canceller for the
     track actually being sent, and the defence reads as one without being
     one - the worst kind, because the failure it leaves behind is quiet
     echo rather than a broken meter.

     `media-source.audioLevel` needs no tap at all: the browser is already
     measuring the track it is encoding, *after* its own processing, so the
     ring shows what the far end will hear. Nothing on this page touches the
     mic through WebAudio any more, which is a property that can be checked
     by reading the file rather than a threshold that has to be tuned.

     The cost, stated: ~10 Hz instead of a frame rate, and a browser that
     reports no `audioLevel` gets a still ring. A flat ring is a cosmetic
     loss; a disabled echo canceller is the bug this whole change is about,
     so the trade is not close. */
  const LEVEL_POLL_MS = 100;
  let levelBusy = false;
  function startMeter() {
    if (levelTimer) return;
    levelTimer = setInterval(async () => {
      // `getStats` is a promise, and setInterval does not wait for one: on a
      // loaded phone a slow read would otherwise stack ticks behind it and
      // deliver them in a burst, which is a ring that stutters rather than
      // breathes. A skipped tick is the right answer - the next one is 100ms
      // away and carries a fresher number than the one being skipped.
      if (levelBusy || !pc) return;
      levelBusy = true;
      let level = null;
      try {
        (await pc.getStats()).forEach(r => {
          if (r.type === "media-source" && r.kind === "audio" && typeof r.audioLevel === "number") level = r.audioLevel;
        });
      } catch { /* a closing connection; the next tick is the recovery */ }
      finally { levelBusy = false; }
      // Re-checked *after* the await, not only before it: a tick already in
      // flight when `end()` runs resolves afterwards, and would light the
      // ring back up a moment after the teardown zeroed it - leaving it lit
      // for the whole of the idle state that follows.
      if (!pc) return;
      // A display curve, not a measurement: audioLevel is linear amplitude,
      // where ordinary speech sits low enough that a linear ring barely
      // moves. The square root spends the ring's travel where the voice is.
      if (level !== null) cfg.onLevel(Math.min(1, Math.sqrt(level) * 2));
    }, LEVEL_POLL_MS);
  }
  function stopMeter() {
    clearInterval(levelTimer); levelTimer = 0;
  }
  /* `linked` is "we have been connected once", which is what separates a
     first connect (chime, start the meter) from a recovery (neither, or the
     call chimes and stacks a second animation loop every time wifi coughs).
     `dropTimer` is the open grace window over a transient drop, and
     `endLabel` is a reason the server announced for a teardown it is about
     to perform - the close that follows carries none. */
  let linked = false, dropTimer = null, endLabel = null;
  /* How long a `disconnected` may last before the call is declared over.
     Long enough for a wifi/cellular handoff or a route change - the events
     that produce it on a phone - and short enough that a dead line does not
     sit there pretending to be live. Usually academic: a browser gives up on
     its own when ICE consent expires (~30s) and reports `failed`, which ends
     the call through the terminal arm below without waiting for this. */
  const DROP_GRACE_MS = 15000;

  function setState(name, label) { cfg.onState(name, label); }

  function onRtvi(msg) {
    switch (msg.type) {
      /* Both user-speaking edges are ignored while the bot is audible.
         They come from the VAD, which on a laptop without headphones fires
         on the speaker as readily as on the room - and "listening" written
         under a reply that is still being spoken is the harness saying it
         heard you when what it heard was itself. It is not feedback that is
         being withheld: a turn starts on a *transcription* here, never on
         the VAD (worker.py), so a barge-in does not take effect at this
         edge either way, and the state that follows a real one is the same
         state it would have shown. */
      case "user-started-speaking":
        if (botSpeaking) break;
        thinkingSound(false); setState("listening", "listening"); break;
      case "user-stopped-speaking":
        if (botSpeaking) break;
        setState("connecting", "…"); break;
      case "user-transcription":
        cfg.onTranscript({ who: "user", text: msg.data.text, interim: !msg.data.final }); break;
      case "bot-llm-started": // request in flight, no first token: D7's trigger
        // Also the watchdog on the flag above: a request in flight is by
        // definition not a reply being played, so a `bot-stopped-speaking`
        // that never arrived cannot leave the user's own edges suppressed
        // for the rest of the call.
        botSpeaking = false;
        thinkingSound(true); setState("thinking", "thinking"); break;
      case "bot-tts-started":
      case "bot-started-speaking":
        botSpeaking = true;
        thinkingSound(false); setState("speaking", "speaking"); break;
      case "bot-transcription":
        cfg.onTranscript({ who: "bot", text: msg.data.text, interim: false }); break;
      case "bot-stopped-speaking":
        botSpeaking = false;
        cfg.onBotTurnEnd(); setState("listening", "listening"); break;
      case "server-message":
        // Custom server→client payloads share one RTVI type, so they are
        // demultiplexed on `t` here rather than upstream.
        if (msg.data?.t === "voice-config") {
          // Written before the UI renders it, and written from the
          // server's state rather than from whatever was asked for.
          writePrefs(msg.data);
          cfg.onVoiceConfig(msg.data);
        }
        // The worker announces a teardown it is about to perform. Held for
        // `end()` rather than acted on: the close arrives a moment later by
        // itself, and what was missing was never the ending - it was any
        // account of why. An unrecognised reason still ends the call, with
        // the server's own word in it.
        else if (msg.data?.t === "call-ending") endLabel = endingLabel(msg.data);
        break;
      case "error":
        cfg.onTranscript({ who: "bot", text: "something went wrong: " + (msg.data?.message || "unknown error"), interim: false });
        break;
    }
  }

  async function connect() {
    // Every per-call flag resets together: a session object that is
    // reconnected must not inherit the previous call's grace window or the
    // reason the previous one ended.
    ended = false; linked = false; endLabel = null; botSpeaking = false;
    clearTimeout(dropTimer); dropTimer = null;
    await AC.resume();
    setState("connecting", "connecting…");
    try {
      micStream = await navigator.mediaDevices.getUserMedia({
        // Explicit, not default: every echo the browser cancels is a
        // segment the STT model never has to be talked out of answering.
        audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
      });
    } catch {
      setState("idle", "microphone refused — tap to retry");
      return;
    }
    pc = new RTCPeerConnection();
    micStream.getTracks().forEach(t => pc.addTrack(t, micStream));
    pc.addTransceiver("audio", { direction: "recvonly" });
    const speaker = new Audio(); speaker.autoplay = true;
    pc.ontrack = (e) => { speaker.srcObject = e.streams[0]; };

    dc = pc.createDataChannel("rtvi");
    dc.onopen = () => {
      dc.send(JSON.stringify({ label: "rtvi-ai", type: "client-ready", id: crypto.randomUUID() }));
      // Ask immediately: the picker must be populated from the server's
      // list, so the first thing a fresh connection does is find out what
      // this worker can actually speak as. With something remembered the
      // same message carries it, so the preference is applied before the
      // first word rather than after one spoken in the wrong voice - and
      // with nothing remembered this is the empty read it always was.
      voiceConfig(readPrefs());
    };
    dc.onmessage = (e) => { try { onRtvi(JSON.parse(e.data)); } catch { /* not rtvi */ } };

    /* The end chime is wired to the connection-state machine, not a server
       message: a server cannot announce a drop over the connection that
       dropped. Abrupt loss and graceful end sound the same because to the
       listener they are the same fact.

       But `disconnected` is not loss, and ending on it is what made calls
       hang up by themselves. It is the browser reporting that packets have
       stopped arriving *for now*; ICE keeps checking and the state returns
       to `connected` on its own when they resume, which on a phone at the
       edge of a room is the normal course of events rather than a failure.
       The worker's own log shows the same hiccup from the other side -
       `socket.send() raised exception` seconds before a call died. So only
       `failed` and `closed` are terminal here; `disconnected` opens a grace
       window and ends the call only if it never comes back.

       Deliberately no ICE restart to shorten that window: pipecat's
       reconnect path (`restart_pc`) fires its own `disconnected` event
       server-side, and that event is what this worker cancels the pipeline
       on - so "reconnecting" would destroy the bot being reconnected to.
       Waiting costs nothing and keeps the conversation. */
    pc.onconnectionstatechange = () => {
      if (!pc) return;
      const state = pc.connectionState;
      if (state === "connected") {
        clearTimeout(dropTimer); dropTimer = null;
        cfg.onLink(true);
        setState("listening", "listening");
        // A recovery is not a new call. Chiming again would announce an
        // arrival that already happened, and a second meter would leave two
        // polling loops running for the rest of the session (`startMeter` is
        // idempotent for the same reason, belt and braces).
        if (!linked) {
          linked = true;
          chimeStart();
          startMeter();
        }
      } else if (state === "disconnected") {
        if (ended || dropTimer) return;
        // The thinking pulse means "a request is in flight"; over a line
        // that is not carrying anything it is a sound telling you something
        // untrue, so it stops here and the next state event restarts it.
        thinkingSound(false);
        setState("connecting", "reconnecting…");
        dropTimer = setTimeout(() => {
          dropTimer = null;
          end("connection lost — tap to reconnect");
        }, DROP_GRACE_MS);
      } else if (state === "failed" || state === "closed") {
        end();
      }
    };

    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    await new Promise(res => {
      if (pc.iceGatheringState === "complete") return res();
      pc.onicegatheringstatechange = () => pc.iceGatheringState === "complete" && res();
      setTimeout(res, 2000);
    });
    /* `request_data` is pipecat's own passthrough: the runner hands it to
       the bot as `runner_args.body`, so naming a session needs no patched
       framework and no second endpoint. Omitted entirely when there is no
       session to name, so a caller that does not use D3 sends exactly the
       bytes it always did. */
    const offerBody = { sdp: pc.localDescription.sdp, type: pc.localDescription.type };
    if (cfg.sessionKey) offerBody.request_data = { session: cfg.sessionKey };
    const resp = await fetch(cfg.offerUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(offerBody),
    }).catch(() => null);
    if (!resp || !resp.ok) { end("could not reach mecha — tap to retry"); return; }
    await pc.setRemoteDescription(await resp.json());
  }

  /* Voice/speed changes are fire-and-forget over the data channel: the
     server answers with the full resulting state, which is what the UI
     renders. A local echo would let the control drift from what is
     actually being spoken the first time a value is refused. */
  function voiceConfig(patch = {}) {
    if (!dc || dc.readyState !== "open") return false;
    dc.send(JSON.stringify({
      label: "rtvi-ai", type: "client-message", id: crypto.randomUUID(),
      data: { t: "voice-config", d: patch },
    }));
    return true;
  }

  /* A teardown the server announced, worded for a person. The reason is
     the server's word and unknown ones pass through: a label naming a cause
     nobody here anticipated still beats "call ended" with no cause at all. */
  function endingLabel(d) {
    if (d?.reason === "idle") {
      const mins = Math.max(1, Math.round((d.after_secs ?? 0) / 60));
      return `call ended — nothing said for ${mins} minutes`;
    }
    return d?.reason ? `call ended (${d.reason}) — tap to reconnect` : null;
  }

  function end(label) {
    if (ended) return;
    ended = true;
    clearTimeout(dropTimer); dropTimer = null;
    thinkingSound(false);
    chimeEnd();
    stopMeter();
    cfg.onLevel(0);
    if (micStream) micStream.getTracks().forEach(t => t.stop());
    if (pc) { try { pc.close(); } catch { /* already gone */ } }
    pc = null; dc = null;
    cfg.onLink(false);
    setState("idle", label || endLabel || "call ended — tap to reconnect");
  }

  return {
    connect,
    end: () => end(),
    /* Mute pauses the sender track rather than stopping it: stopping
       releases the device (the mic indicator goes dark and resume needs a
       new permission dance on some browsers), while a disabled track sends
       silence - which the worker's energy gate drops before the STT model
       ever hears it, so a muted room costs zero tokens. */
    setMicEnabled(on) {
      if (micStream) micStream.getAudioTracks().forEach(t => { t.enabled = !!on; });
    },
    get micEnabled() {
      return !!micStream && micStream.getAudioTracks().some(t => t.enabled);
    },
    get connected() { return !!pc; },
    voiceConfig,
  };
}
