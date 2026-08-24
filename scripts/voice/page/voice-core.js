/* voice-core.js — the embeddable heart of mecha's voice mode.
 *
 * Framework-agnostic on purpose: the standalone page (index.html) and the
 * tailnet app's in-chat voice mode (Chat.svelte, the remote-surface arc)
 * must be the same machinery with different shells, or the two will drift
 * apart in exactly the ways that matter (chime timing, barge-in, the
 * end-sound-on-dead-network rule). docs/VOICE-RESEARCH.md D7 governs the
 * sounds; the RTVI event names come from pipecat 1.7.
 *
 * Contract:
 *   const session = createVoiceSession({
 *     offerUrl,            // default "/api/offer"; the app passes the
 *                          // worker origin's absolute URL until the
 *                          // process unification gives it a local proxy
 *     onState,             // (name, label) — idle|connecting|listening|thinking|speaking
 *     onTranscript,        // ({who: "user"|"bot", text, interim})
 *     onLevel,             // (0..1) real mic level, for state rings
 *     onLink,              // (live: bool)
 *     onBotTurnEnd,        // () — the open bot utterance is complete
 *   });
 *   await session.connect();   // user gesture required (audio unlock)
 *   session.end();             // graceful; abrupt loss fires the same chime
 *   session.setMicEnabled(on); // mute control: pauses the outbound track
 *                              // without ending the call; state is readable
 *                              // as session.micEnabled
 *   session.connected          // bool
 *
 * Sounds are synthesized, never fetched: the end chime's most important
 * trigger is the network dying, and a sound that must be downloaded
 * cannot play then.
 */
export function createVoiceSession(opts = {}) {
  const cfg = {
    offerUrl: "/api/offer",
    onState: () => {},
    onTranscript: () => {},
    onLevel: () => {},
    onLink: () => {},
    onBotTurnEnd: () => {},
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

  let pc = null, micStream = null, meterTrack = null, levelRAF = 0, ended = false;

  function setState(name, label) { cfg.onState(name, label); }

  function onRtvi(msg) {
    switch (msg.type) {
      case "user-started-speaking":
        thinkingSound(false); setState("listening", "listening"); break;
      case "user-stopped-speaking":
        setState("connecting", "…"); break;
      case "user-transcription":
        cfg.onTranscript({ who: "user", text: msg.data.text, interim: !msg.data.final }); break;
      case "bot-llm-started": // request in flight, no first token: D7's trigger
        thinkingSound(true); setState("thinking", "thinking"); break;
      case "bot-tts-started":
      case "bot-started-speaking":
        thinkingSound(false); setState("speaking", "speaking"); break;
      case "bot-transcription":
        cfg.onTranscript({ who: "bot", text: msg.data.text, interim: false }); break;
      case "bot-stopped-speaking":
        cfg.onBotTurnEnd(); setState("listening", "listening"); break;
      case "error":
        cfg.onTranscript({ who: "bot", text: "something went wrong: " + (msg.data?.message || "unknown error"), interim: false });
        break;
    }
  }

  async function connect() {
    ended = false;
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
    // The meter taps a CLONE of the mic track, never the one WebRTC
    // sends: WebKit silently disables echo cancellation on a getUserMedia
    // track once WebAudio attaches to it, and a phone that hears its own
    // speaker becomes a bot talking to itself (observed in production,
    // 2026-08-24: transcripts attributed to the owner that were the TTS).
    meterTrack = micStream.getAudioTracks()[0].clone();
    const meterStream = new MediaStream([meterTrack]);
    const src = AC.createMediaStreamSource(meterStream);
    const analyser = AC.createAnalyser(); analyser.fftSize = 512;
    src.connect(analyser);
    const data = new Uint8Array(analyser.frequencyBinCount);
    const step = () => {
      analyser.getByteTimeDomainData(data);
      let peak = 0;
      for (const v of data) peak = Math.max(peak, Math.abs(v - 128) / 128);
      cfg.onLevel(Math.min(1, peak * 2.2));
      levelRAF = requestAnimationFrame(step);
    };

    pc = new RTCPeerConnection();
    micStream.getTracks().forEach(t => pc.addTrack(t, micStream));
    pc.addTransceiver("audio", { direction: "recvonly" });
    const speaker = new Audio(); speaker.autoplay = true;
    pc.ontrack = (e) => { speaker.srcObject = e.streams[0]; };

    const dc = pc.createDataChannel("rtvi");
    dc.onopen = () => dc.send(JSON.stringify({ label: "rtvi-ai", type: "client-ready", id: crypto.randomUUID() }));
    dc.onmessage = (e) => { try { onRtvi(JSON.parse(e.data)); } catch { /* not rtvi */ } };

    /* The end chime is wired to the connection-state machine, not a server
       message: a server cannot announce a drop over the connection that
       dropped. Abrupt loss and graceful end sound the same because to the
       listener they are the same fact. */
    pc.onconnectionstatechange = () => {
      if (!pc) return;
      if (pc.connectionState === "connected") {
        cfg.onLink(true);
        chimeStart();
        setState("listening", "listening");
        levelRAF = requestAnimationFrame(step);
      } else if (["disconnected", "failed", "closed"].includes(pc.connectionState)) {
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
    const resp = await fetch(cfg.offerUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ sdp: pc.localDescription.sdp, type: pc.localDescription.type }),
    }).catch(() => null);
    if (!resp || !resp.ok) { end("could not reach mecha — tap to retry"); return; }
    await pc.setRemoteDescription(await resp.json());
  }

  function end(label) {
    if (ended) return;
    ended = true;
    thinkingSound(false);
    chimeEnd();
    cancelAnimationFrame(levelRAF);
    cfg.onLevel(0);
    if (micStream) micStream.getTracks().forEach(t => t.stop());
    if (meterTrack) meterTrack.stop();
    if (pc) { try { pc.close(); } catch { /* already gone */ } }
    pc = null;
    cfg.onLink(false);
    setState("idle", label || "call ended — tap to reconnect");
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
  };
}
