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
 *     onState,             // (name, label) — idle|connecting|listening|thinking|speaking|paused
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

/* The link verdict, from two counters the browser keeps.

   `packetsIn` is `inbound-rtp.packetsReceived` for the bot's audio track.
   The worker's output track sends silence whenever it is not speaking
   (`RawAudioTrack(auto_silence=True)`), so on a working link this rises
   every tick, and a count that has not moved for `INBOUND_STALL_MS` is a
   link that has stopped delivering - the same two seconds the worker's own
   transport waits before it logs "No audio frame received".

   `reportAt` is `remote-inbound-rtp.timestamp`: when this browser last
   received an RTCP receiver report about the audio it is *sending*. The
   far end sends one about every second; a stamp that has not advanced for
   `REPORT_STALL_MS` means the worker has stopped saying it hears us, which
   is the stall that matters to a person mid-sentence. Longer than the
   inbound window because the report interval is the far end's to choose.

   Missing counters are unknown, not a stall: a browser that reports neither
   gets no verdict rather than a pause tone on every call. `stalled` is
   `true` on the tick the link is declared paused, `false` on the tick it is
   declared back, and `null` in between - so the caller sounds each
   transition once. */
export const INBOUND_STALL_MS = 2000;
export const REPORT_STALL_MS = 4000;

export function freshLink() {
  return { packetsIn: null, packetsAt: null, reportAt: null, reportSeenAt: null, stalled: false };
}

export function linkVerdict(prev, sample, now) {
  const next = { ...prev };
  if (typeof sample.packetsIn === "number") {
    if (next.packetsIn === null || sample.packetsIn > next.packetsIn) {
      next.packetsIn = sample.packetsIn; next.packetsAt = now;
    }
  }
  if (typeof sample.reportAt === "number") {
    if (next.reportAt === null || sample.reportAt > next.reportAt) {
      next.reportAt = sample.reportAt; next.reportSeenAt = now;
    }
  }
  const inboundStalled = next.packetsAt !== null && now - next.packetsAt >= INBOUND_STALL_MS;
  const reportStalled = next.reportSeenAt !== null && now - next.reportSeenAt >= REPORT_STALL_MS;
  const known = next.packetsAt !== null || next.reportSeenAt !== null;
  if (!known) return { next, stalled: null, reason: null };
  const stalled = inboundStalled || reportStalled;
  if (stalled === prev.stalled) return { next, stalled: null, reason: null };
  next.stalled = stalled;
  return { next, stalled, reason: inboundStalled ? "inbound" : reportStalled ? "report" : null };
}

/* Why a call is paused, by source, and which transitions are the worker's
   business. Two kinds of reason: the page's own (`link` from its statistics,
   `mic` from the track's mute edge) and `server`, the worker's announcement
   of a pause it is already holding for. The worker is told when the page's
   *own* reasons begin and when they end - never about `server`, which is
   the worker talking; echoing that back made the worker hold on the page's
   behalf until the page said otherwise, which the page would only do once
   the worker had let go (review of #226 - a call that paused once stayed
   paused for its life). Sounds and the label follow the whole set: a pause
   is one pause whoever saw it first, and it is over when nobody holds it. */
export class Pauses {
  constructor() { this.reasons = new Set(); }
  static isLocal(reason) { return reason !== "server"; }
  get size() { return this.reasons.size; }
  get any() { return this.reasons.size > 0; }
  get localCount() { return [...this.reasons].filter(Pauses.isLocal).length; }
  get first() { const [r] = this.reasons; return r ?? null; }
  /* add: `first` - the set was empty (sound it, label it);
     `announce` - this is one of the page's own reasons (tell the worker,
     naming it). Every local reason is announced, not only the first: the
     worker holds them as a set and expires each by its own witness, and
     telling it only the first let a `link` hold's expiry lift a `mic` hold
     the page still owned (eighth review of #226). */
  add(reason) {
    if (this.reasons.has(reason)) return { first: false, announce: false, added: false };
    const first = this.reasons.size === 0;
    this.reasons.add(reason);
    return { first, announce: Pauses.isLocal(reason), added: true };
  }
  /* remove: `last` - the set is now empty (resume, sound it);
     `announce` - one of the page's own reasons ended (tell the worker `ok`, naming it). */
  remove(reason) {
    if (!this.reasons.delete(reason)) return { last: false, announce: false, removed: false };
    return { last: this.reasons.size === 0, announce: Pauses.isLocal(reason), removed: true };
  }
  clear() { this.reasons.clear(); }
}

/* When a pause the *worker* announced may be cleared without its `ok`.
   The worker pauses on an uplink stall - our audio not reaching it - and
   the only uplink witness this page has is the far end's receiver report
   (`reportSeenAt`). Downlink packets arriving prove nothing about it, so a
   healthy inbound count alone must not clear the pause (fifth review of
   #226): every witness has to be fresh, the report included, for
   `SERVER_PAUSE_EXPIRY_MS` after the announcement. A browser that never
   populates `remote-inbound-rtp` therefore never expires a worker pause -
   it waits for the `ok` - which is the fail-closed side of an unknown. */
export const SERVER_PAUSE_EXPIRY_MS = 5000;

export function serverPauseExpired(link, serverPausedAt, now) {
  if (!serverPausedAt || now - serverPausedAt <= SERVER_PAUSE_EXPIRY_MS) return false;
  if (link.stalled) return false;
  if (link.packetsAt === null || now - link.packetsAt >= INBOUND_STALL_MS) return false;
  if (link.reportSeenAt === null || now - link.reportSeenAt >= REPORT_STALL_MS) return false;
  return true;
}

/* ---- the reliable uplink (docs/VOICE-LINK-DESIGN.md) --------------------
   Speech is captured and buffered here, on the phone, and delivered over the
   data channel, which retransmits; RTP does not. An unstable link therefore
   delays the owner's words instead of losing them. Input is never paused;
   output waits. The pieces below are pure so `web/test/voice-link.mjs` can
   drive them without a browser.

   What is buffered is the *encoded* frame the RTP sender was about to send
   (see `voice-uplink-transform.js`), so the audio is byte-identical to the
   RTP path's, post-AEC, and five minutes of it is about a megabyte. */

/* How much of an outage is kept, in audio (§4.2). Past this the oldest is
   dropped — and recorded, so the worker can say what was lost. */
export const UPLINK_RING_MS = 300_000;
/* One batch on the wire. Small enough that a `link` message queued behind
   it is not late; large enough that the JSON is not the cost. */
export const UPLINK_BATCH_MS = 100;
/* Bytes the SCTP queue may hold before the page keeps the rest in the ring,
   where it controls the drop policy (§2.2). */
export const UPLINK_SCTP_HIGH_BYTES = 32_768;
/* The cue keys on how far behind the assistant is, never on packets (§3):
   nothing under this, one soft cue past it, "caught up" at drain. */
export const BEHIND_TONE_MS = 3000;
export const CAUGHT_UP_MS = 500;
/* What a millisecond of speech costs on the channel: ~4 kB/s of Opus, a
   third more as base64, plus the JSON around each frame. Used to turn
   `bufferedAmount` — bytes the SCTP queue still holds — back into time. */
export const UPLINK_WIRE_BYTES_PER_MS = 6;
/* How far behind the worker really is: what the ring holds *and* what has
   been handed to the channel but not delivered. The first cut sent the ring
   alone, and with the pump keeping ~32 kB queued the last batches of a drain
   reported near zero while seconds of the sentence were still in flight —
   so the worker lifted its hold mid-sentence (review of #231). */
export function wireBacklogMs(pendingMs, bufferedAmount) {
  return pendingMs + (bufferedAmount || 0) / UPLINK_WIRE_BYTES_PER_MS;
}
/* Opus at 48 kHz; the transform reports RTP timestamps at the codec clock. */
const RTP_CLOCK_HZ = 48000;
const RTP_TS_WRAP = 2 ** 32;

/* Encoded frames, each with the duration the *next* timestamp proves and a
   position on the page's media clock — milliseconds of captured audio since
   this session object was created. The clock runs across reconnects: a new
   RTP stream restarts its timestamps, and the first frame of it is simply
   placed at the clock's current value, so what was said during the old call
   sits before what is said in the new one, in order. */
export class UplinkRing {
  constructor(capMs = UPLINK_RING_MS) {
    this.capMs = capMs;
    this.frames = [];        // {ms, durMs, data}
    this.pendingMs = 0;      // captured, not yet taken
    this.nowMs = 0;          // the media clock: end of the last frame
    this.lastTs = null;      // last RTP timestamp seen on the current stream
    this.pendingFrame = null; // a frame whose duration is unknown until the next arrives
    this.dropped = [];       // [{fromMs, toMs}] since last `takeDropped`
    this.seq = 0;
  }
  /* A new RTP stream (a reconnect): the next timestamp anchors at `nowMs`. */
  restart() { this.lastTs = null; this._flushPending(20); }
  /* An encoded frame from the tap. Its duration is the gap to the next
     frame's timestamp, so each frame is held until the one after it. */
  push(ts, data) {
    if (this.lastTs !== null) {
      const delta = (ts - this.lastTs + RTP_TS_WRAP) % RTP_TS_WRAP;
      // A sane Opus frame is 2.5–120 ms; anything else is a restarted or
      // corrupt clock, and the held frame is closed at the nominal 20 ms.
      const durMs = delta >= 120 && delta <= 5760 ? delta / (RTP_CLOCK_HZ / 1000) : 20;
      this._flushPending(durMs);
    }
    this.lastTs = ts;
    // The wall clock at capture, on the frame itself: the media clock does
    // not run while nothing is captured, so it cannot place speech from
    // before an outage — this can (review of #231).
    this.pendingFrame = { data, wallMs: Date.now() };
  }
  _flushPending(durMs) {
    const f = this.pendingFrame;
    if (!f) return;
    this.pendingFrame = null;
    this.frames.push({ ms: this.nowMs, durMs, data: f.data, wallMs: f.wallMs });
    this.nowMs += durMs;
    this.pendingMs += durMs;
    while (this.pendingMs > this.capMs && this.frames.length) {
      const old = this.frames.shift();
      this.pendingMs -= old.durMs;
      const last = this.dropped[this.dropped.length - 1];
      if (last && Math.abs(last.toMs - old.ms) < 1) last.toMs = old.ms + old.durMs;
      else this.dropped.push({ fromMs: old.ms, toMs: old.ms + old.durMs });
    }
  }
  /* Up to `maxMs` of the oldest frames, as one batch, or null if empty. */
  takeBatch(maxMs = UPLINK_BATCH_MS) {
    if (!this.frames.length) return null;
    const out = [];
    let ms = 0;
    while (this.frames.length && (out.length === 0 || ms + this.frames[0].durMs <= maxMs)) {
      const f = this.frames.shift();
      out.push(f); ms += f.durMs; this.pendingMs -= f.durMs;
    }
    return { seq: this.seq++, ms: out[0].ms, wallMs: out[0].wallMs, frames: out, backlogMs: this.pendingMs };
  }
  takeDropped() { const d = this.dropped; this.dropped = []; return d; }
  /* A batch the channel would not take goes back where it was, its dropped
     spans with it: the ring's whole job is to not forget, and forgetting on
     a failed send would be the one place it did (review of #231). */
  unsend(batch, dropped = []) {
    this.frames.unshift(...batch.frames);
    this.pendingMs += batch.frames.reduce((a, f) => a + f.durMs, 0);
    if (dropped.length) this.dropped = [...dropped, ...this.dropped];
    this.seq = batch.seq;
  }
}

/* The cue policy over a behind-count, kept pure. `prev` is what was last
   decided; returns the next state and which sound, if any, to make. A
   "behind" cue is made once per episode, when the count first passes the
   threshold; "caught" is made once, when a sounded episode drains. */
export function behindVerdict(prev, behindMs) {
  const next = { sounded: prev?.sounded ?? false };
  if (!next.sounded && behindMs >= BEHIND_TONE_MS) { next.sounded = true; return { next, cue: "behind" }; }
  if (next.sounded && behindMs <= CAUGHT_UP_MS) { next.sounded = false; return { next, cue: "caught" }; }
  return { next, cue: null };
}

export function createVoiceSession(opts = {}) {
  const cfg = {
    offerUrl: "/api/offer",
    offerHeaders: {},
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
  /* Pause and resume: the link went quiet, the link came back. Two soft notes
     down, two soft notes up - a step, not the end chime's fall, because the
     call is not over and the sound must not say it is. Quieter than the
     chimes: they mark an event the listener asked for; these interrupt one.
     Requested 2026-09-12 after a drive on which the page had no way to say
     that the far end had stopped hearing anything (docs/VOICE-RESEARCH.md
     D7, amended). */
  const pauseTone = () => { const t = AC.currentTime; softTone(494, t, .3, .05); softTone(370, t + .18, .45, .045); };
  const resumeTone = () => { const t = AC.currentTime; softTone(370, t, .3, .05); softTone(494, t + .18, .45, .045); };
  /* Behind and caught up (docs/VOICE-LINK-DESIGN.md §3): one low note, not
     the pause step — the call is not paused and the listener need not stop;
     the rising pair when the backlog has drained. */
  const behindTone = () => { const t = AC.currentTime; softTone(330, t, .4, .045); };
  const caughtUpTone = () => { const t = AC.currentTime; softTone(370, t, .25, .04); softTone(494, t + .15, .4, .04); };
  // Thinking is a soft two-note pulse, not a tick: a slow attack removes the
  // percussive edge (the old triangle tick read as a metronome), the lowpass
  // keeps it warm, and the pair alternates rising/falling so a long wait
  // breathes instead of repeating one sound at you.
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
  /* The uplink ring outlives a call on purpose (§4.2): what was said while
     the link was down is delivered at the head of the next connection. */
  const ring = new UplinkRing();
  let uplinkWorker = null, uplinkMode = "rtp", pumpTimer = 0, pumping = false, lastFrameAt = 0;
  let behind = { sounded: false }, behindShownS = 0;
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
  /* The link monitor rides the same poll. `getStats` already carries the
     two facts a pause needs: whether the far end's audio is still arriving
     (`inbound-rtp.packetsReceived` - the worker sends silence between
     replies, so a still count is a stalled link, not a quiet bot) and when
     the far end last reported hearing *us* (`remote-inbound-rtp.timestamp`,
     the RTCP receiver report). `linkVerdict` is the decision, kept pure so
     it can be tested without a browser. */
  let link = freshLink();
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
      const sample = { packetsIn: null, reportAt: null };
      try {
        (await pc.getStats()).forEach(r => {
          if (r.type === "media-source" && r.kind === "audio" && typeof r.audioLevel === "number") level = r.audioLevel;
          if (r.type === "inbound-rtp" && r.kind === "audio" && typeof r.packetsReceived === "number") sample.packetsIn = r.packetsReceived;
          if (r.type === "remote-inbound-rtp" && r.kind === "audio" && typeof r.timestamp === "number") sample.reportAt = r.timestamp;
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
      // Only once the call is up: before `connected` nothing has arrived
      // yet and a still count is the connection being made, not lost.
      if (linked) {
        // A monotonic clock: the windows below are durations, and the one
        // environment this runs in guarantees wall-clock steps (NTP on a
        // phone crossing cells). `reportAt` is compared only for advancing.
        const now = performance.now();
        const v = linkVerdict(link, sample, now);
        link = v.next;
        if (v.stalled === true) pause("link");
        else if (v.stalled === false) resume("link");
        if (pausedBy.reasons.has("server") && serverPauseExpired(link, serverPausedAt, now)) resume("server");
        watchUplinkFrames(now);
        noteBehind();
      }
    }, LEVEL_POLL_MS);
  }
  /* Why the call is paused, by source. A pause is sounded when the first
     reason arrives and the resume when the last one leaves, so a stall the
     page saw and the worker also announced is one pause, not two - and a
     mic iOS muted on screen lock stays paused through a link that is fine.
     `link` is this page's statistics, `server` the worker's own watch over
     its audio (an RTVI `link` message), `mic` the track's mute edge. */
  const pausedBy = new Pauses();
  /* The label changes at once; the sound waits. The worker holds the turn
     from three-quarters of a second without audio, and says so, and a
     cellular link blips for that long routinely - a tone for every blip
     would be the thinking pulse's mistake again, a sound telling you
     something you did not need to know. A pause that lasts this long is
     one the listener has already noticed; the tone confirms it, and a
     resume is only sounded for a pause that was. */
  const PAUSE_TONE_DELAY_MS = 700;
  let pauseToneTimer = null, pauseSounded = false;
  const PAUSE_LABELS = {
    link: "connection paused — waiting for the network",
    server: "mecha stopped hearing you — waiting for the network",
    mic: "microphone paused — is the screen locked?",
  };
  /* The worker's announcement is cleared by its `ok`, which travels over
     a channel that may be the thing that stalled. One other witness can
     clear it: this page's own statistics reading healthy - uplink and
     down - for `SERVER_PAUSE_EXPIRY_MS` after the announcement
     (`serverPauseExpired`): the worker's settle is 0.6 s, so an `ok` five
     seconds overdue on a link demonstrably carrying packets both ways is
     a lost message, not a held turn. Without a way out, `setState`
     swallowing every transition while held would freeze a working call at
     "paused" (third review of #226). Deliberately *not* the worker's own
     events: a transcript arrives during a hold - it is the fragment the
     stall cut, held rather than acted on - and a reply can still be
     playing out, so either would un-pause the page a second after it
     paused, over a link it had just been told carries nothing (fourth
     review). */
  let serverPausedAt = 0;
  function pause(reason) {
    if (ended) return;
    /* Under the buffered uplink a stalled link is not a pause: the phone
       keeps hearing, the ring keeps growing, and what the owner is told is
       how far behind the assistant is (§3), not to stop talking. The worker
       is still told, so it holds the turn; the sound and the label are
       not made. `mic` — capture itself stopped — stays a real pause (§4.1). */
    if (uplinkMode === "channel" && (reason === "link" || reason === "server")) {
      if (reason === "link") sendLink("paused", reason);
      return;
    }
    const { first, announce } = pausedBy.add(reason);
    if (reason === "server") serverPausedAt = performance.now();
    if (announce) sendLink("paused", reason);
    if (!first) return;
    thinkingSound(false);
    pauseSounded = false;
    pauseToneTimer = setTimeout(() => { pauseToneTimer = null; pauseSounded = true; pauseTone(); }, PAUSE_TONE_DELAY_MS);
    cfg.onState("paused", PAUSE_LABELS[reason] || "paused");
  }
  function resume(reason) {
    if (uplinkMode === "channel" && (reason === "link" || reason === "server")) {
      if (reason === "link") sendLink("ok", reason);
      return;
    }
    const { last, announce, removed } = pausedBy.remove(reason);
    if (!removed) return;
    if (announce) sendLink("ok", reason);
    if (!last) {
      // Still paused for another reason; relabel to the one that remains.
      cfg.onState("paused", PAUSE_LABELS[pausedBy.first] || "paused");
      return;
    }
    if (ended) return;
    clearTimeout(pauseToneTimer); pauseToneTimer = null;
    if (pauseSounded) resumeTone();
    pauseSounded = false;
    // Back to what the call was doing, not to "listening": a pause during
    // a twenty-second search must come back as the wait it interrupted,
    // pulse and all, or the page is silent and lying about it - D7's own
    // failure mode (third review of #226).
    if (lastState.name === "thinking") thinkingSound(true);
    cfg.onState(lastState.name, lastState.label);
  }
  /* Tell the worker, so it holds the turn (a gap the page saw is a gap in
     the owner's sentence) and so the journal carries the phone's view.
     Fire-and-forget over the data channel; a channel that is itself stalled
     drops it, and the worker's own audio watch covers that case. */
  function sendLink(state, reason) {
    if (!dc || dc.readyState !== "open") return;
    try {
      dc.send(JSON.stringify({
        label: "rtvi-ai", type: "client-message", id: crypto.randomUUID(),
        data: { t: "link", d: { state, reason } },
      }));
    } catch { /* closing */ }
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

  /* The last state the call was actually in, kept up to date through a
     pause so the resume restores the present, not the moment the pause
     began. */
  let lastState = { name: "listening", label: "listening" };
  function setState(name, label) {
    if (name !== "idle" && name !== "paused") lastState = { name, label };
    // A paused call stays labelled paused: the worker's speaking edges and
    // the user's own can still arrive over a channel that is half working,
    // and "listening" over a link the page knows is not carrying anything
    // is the state this feature exists to stop showing.
    if (pausedBy.any && name !== "idle") return;
    cfg.onState(name, name === "idle" ? label : withBehind(label));
  }
  /* The one fact a listener needs about the link (§3): how far behind the
     assistant is, from this page's own numbers, no round trip. */
  function withBehind(label) {
    return behindShownS >= 1 ? `${label} · ${behindShownS} s behind` : label;
  }
  function behindMs() {
    return wireBacklogMs(ring.pendingMs, dc && dc.readyState === "open" ? dc.bufferedAmount : 0);
  }
  function noteBehind() {
    if (ended || uplinkMode !== "channel") return;
    const ms = behindMs();
    const v = behindVerdict(behind, ms);
    behind = v.next;
    if (v.cue === "behind") behindTone();
    else if (v.cue === "caught") caughtUpTone();
    const shown = ms >= 1000 ? Math.round(ms / 1000) : 0;
    if (shown !== behindShownS) {
      behindShownS = shown;
      if (!pausedBy.any) cfg.onState(lastState.name, withBehind(lastState.label));
    }
  }

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
        if (!pausedBy.any) thinkingSound(true);
        setState("thinking", "thinking"); break;
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
        // The worker's own watch over its microphone audio: it saw the gap
        // (or the page reported one) and is holding the turn. Sounded here
        // only if this page has not already sounded it.
        else if (msg.data?.t === "link") {
          if (msg.data.state === "paused") pause("server"); else resume("server");
        }
        // The worker's side of the watch: RTP was arriving and no batch
        // was, so it is reading RTP now. Follow it — the pauses come back.
        else if (msg.data?.t === "uplink" && msg.data.state === "rtp") uplinkFailed(msg.data.why || "the worker fell back", true);
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
    pausedBy.clear(); link = freshLink();
    // Added per connect and removed per end, so a session reconnected
    // through the same object keeps the promise above about re-requesting
    // the lock; `end()` removed it and `connect()` never put it back.
    document.removeEventListener("visibilitychange", onVisible);
    document.addEventListener("visibilitychange", onVisible);
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
    /* The microphone can be taken away without the call ending. iOS mutes
       the track when the screen locks or another app takes the audio
       session (a navigation prompt, a phone call) and unmutes it after; the
       packets keep flowing, carrying silence, and nothing downstream can
       tell that from the owner having stopped talking. Both calls on
       2026-09-12 went quiet ~60 s after the last touch of the screen. The
       edge is the only witness, so it is what pauses - and `ended` is the
       track gone for good, which is a reconnect, not a wait. */
    micStream.getAudioTracks().forEach(t => {
      t.onmute = () => pause("mic");
      t.onunmute = () => resume("mic");
      t.onended = () => end("microphone lost — tap to reconnect");
    });
    pc = new RTCPeerConnection();
    // After `pc` exists: the post-await guard in `holdScreen` reads it.
    holdScreen();
    micStream.getTracks().forEach(t => pc.addTrack(t, micStream));
    pc.addTransceiver("audio", { direction: "recvonly" });
    uplinkMode = (await attachUplinkTap()) ? "channel" : "rtp";
    if (!pc) return; // ended while the worker was loading
    ring.restart();
    const speaker = new Audio(); speaker.autoplay = true;
    pc.ontrack = (e) => { speaker.srcObject = e.streams[0]; };

    dc = pc.createDataChannel("rtvi");
    dc.onopen = () => {
      dc.send(JSON.stringify({ label: "rtvi-ai", type: "client-ready", id: crypto.randomUUID() }));
      if (uplinkMode === "channel") {
        // The clock the worker converts media time with, sent once per
        // connection; then whatever the ring already holds — the previous
        // call's tail, if the link died — goes first, in order (§4.2).
        sendClientMessage("audio-start", { tz_offset_min: -new Date().getTimezoneOffset() });
        pump();
      }
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
          lastFrameAt = performance.now();
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
    // Declared in the offer so the worker picks its audio source before a
    // single RTP frame is read — a switch mid-call would double the first
    // words. A page without the tap says nothing and gets the RTP path.
    if (uplinkMode === "channel") offerBody.request_data = { ...(offerBody.request_data || {}), uplink: "channel" };
    const resp = await fetch(cfg.offerUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json", ...cfg.offerHeaders },
      body: JSON.stringify(offerBody),
    }).catch(() => null);
    if (!resp || !resp.ok) { end("could not reach mecha — tap to retry"); return; }
    await pc.setRemoteDescription(await resp.json());
  }

  /* Voice/speed changes are fire-and-forget over the data channel: the
     server answers with the full resulting state, which is what the UI
     renders. A local echo would let the control drift from what is
     actually being spoken the first time a value is refused. */
  function sendClientMessage(t, d) {
    if (!dc || dc.readyState !== "open") return false;
    try {
      dc.send(JSON.stringify({ label: "rtvi-ai", type: "client-message", id: crypto.randomUUID(), data: { t, d } }));
      return true;
    } catch { return false; }
  }
  /* Attach the encoded-frame tap to the audio sender (§2.1). Standard
     `RTCRtpScriptTransform` first — a worker, from this origin, because of
     the CSP — then Chrome's older main-thread streams. Returns whether the
     uplink can be buffered at all; a browser with neither keeps the RTP
     path and the #226 behaviour, and says so in the journal via the offer. */
  /* How long the worker script gets to say it loaded (its first message),
     and how long after `connected` the first frame gets to arrive, before
     the tap is judged dead and the call falls back to RTP. */
  const UPLINK_READY_MS = 1500, UPLINK_FIRST_FRAME_MS = 5000;
  async function attachUplinkTap() {
    const sender = pc.getSenders().find(s => s.track && s.track.kind === "audio");
    if (!sender) return false;
    const onFrame = (m) => { if (m && m.data) { lastFrameAt = performance.now(); ring.push(m.ts, m.data); pump(); } };
    try {
      if (typeof RTCRtpScriptTransform === "function") {
        const w = new Worker("/voice-uplink-transform.js");
        // Proof of load, not construction: a 404 or a parse error on the
        // script arrives asynchronously, and a tap that attached but never
        // delivers is a call nobody hears (review of #231). The script's
        // first message says it ran; without it, RTP as before.
        const ready = await new Promise(res => {
          const t = setTimeout(() => res(false), UPLINK_READY_MS);
          w.onmessage = (e) => { if (e.data && e.data.ready) { clearTimeout(t); res(true); } };
          w.onerror = () => { clearTimeout(t); res(false); };
        });
        if (!ready || !pc) { try { w.terminate(); } catch { /* gone */ } return false; }
        uplinkWorker = w;
        w.onmessage = (e) => onFrame(e.data);
        w.onerror = () => uplinkFailed("worker error");
        sender.transform = new RTCRtpScriptTransform(w, {});
        return true;
      }
      if (typeof sender.createEncodedStreams === "function") {
        const { readable, writable } = sender.createEncodedStreams();
        readable.pipeThrough(new TransformStream({
          transform(frame, ctl) {
            try { onFrame({ ts: frame.timestamp, data: frame.data.slice(0) }); } catch { /* still sent */ }
            ctl.enqueue(frame);
          },
        })).pipeTo(writable);
        return true;
      }
    } catch { /* fall through to RTP */ }
    return false;
  }
  /* The tap is not delivering: fall back to RTP for the rest of this call,
     and say so. The worker's own watchdog (`UPLINK_DEAF_SECS`) starts
     reading RTP on its side; the two need no message between them. The
     `link`/`server` pauses come back with the mode. */
  function uplinkFailed(why, fromWorker = false) {
    if (uplinkMode !== "channel") return;
    uplinkMode = "rtp";
    clearTimeout(pumpTimer); pumpTimer = 0;
    if (uplinkWorker) { try { uplinkWorker.terminate(); } catch { /* gone */ } uplinkWorker = null; }
    behindShownS = 0; behind = { sounded: false };
    // Each end tells the other; the worker's reader stays parked otherwise.
    if (!fromWorker) sendClientMessage("uplink", { state: "rtp", why });
    cfg.onTranscript({ who: "bot", text: `voice: the buffered microphone path failed (${why}) — using the direct path for this call`, interim: false });
    if (!pausedBy.any) cfg.onState(lastState.name, lastState.label);
  }
  /* Standing, not one-shot (review of #231): a tap that dies mid-call is
     as deaf as one that never started. Frames stopping while the mic track
     is live and unmuted is the witness — a link stall does not stop them,
     capture is local. Read on the meter tick. */
  function micLive() {
    return !!micStream && micStream.getAudioTracks().some(t => t.readyState === "live" && t.enabled && !t.muted);
  }
  function watchUplinkFrames(now) {
    if (uplinkMode !== "channel" || !linked || !micLive()) return;
    if (now - lastFrameAt > UPLINK_FIRST_FRAME_MS) uplinkFailed("no audio frames from the tap");
  }
  /* Drain the ring onto the data channel in batches, pacing on the SCTP
     queue (§2.2): what waits, waits in the ring. Each batch carries how far
     behind the ring still is, which is the worker's "caught up" witness. */
  function pump() {
    if (pumping || ended || uplinkMode !== "channel") return;
    if (!dc || dc.readyState !== "open") return;
    pumping = true;
    try {
      while (ring.frames.length && dc.bufferedAmount < UPLINK_SCTP_HIGH_BYTES) {
        const batch = ring.takeBatch();
        if (!batch) break;
        const dropped = ring.takeDropped();
        const frames = batch.frames.map(f => [f.durMs, b64(f.data)]);
        // The worker's caught-up witness: the ring *and* the queue (§2.4).
        const d = { seq: batch.seq, ms: batch.ms, wall_ms: batch.wallMs, backlog_ms: Math.round(wireBacklogMs(batch.backlogMs, dc.bufferedAmount)), frames };
        if (dropped.length) d.dropped = dropped.map(x => ({ from_ms: Math.round(x.fromMs), to_ms: Math.round(x.toMs) }));
        if (!sendClientMessage("audio", d)) { ring.unsend(batch, dropped); break; }
      }
    } finally { pumping = false; }
    // Left over because the queue was full: come back when it drains.
    if (ring.frames.length && !pumpTimer) pumpTimer = setTimeout(() => { pumpTimer = 0; pump(); }, 50);
    noteBehind();
  }
  function b64(buf) {
    const bytes = new Uint8Array(buf);
    let s = "";
    for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    return btoa(s);
  }
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

  /* Keep the screen on for the length of the call. The lock is what stands
     between a driver and the mute above: with it, the phone does not lock,
     the track is not muted, and the pause it would have caused never
     happens. Best effort - a browser without the API, or a page that is
     not visible when asked, simply gets no lock - and re-requested whenever
     the page comes back into view, because the browser releases it on every
     hide. */
  let wakeLock = null;
  async function holdScreen() {
    if (ended || !navigator.wakeLock || document.visibilityState !== "visible") return;
    let lock = null;
    try { lock = await navigator.wakeLock.request("screen"); } catch { lock = null; }
    // Re-checked after the await, as `startMeter` does: a request in
    // flight when `end()` ran resolves afterwards, and a lock stored then
    // outlives the call - the screen stays on in the car until the page
    // hides (sixth review of #226).
    if (ended || !pc) { if (lock) lock.release().catch(() => {}); return; }
    if (wakeLock && wakeLock !== lock) wakeLock.release().catch(() => {});
    wakeLock = lock;
  }
  function releaseScreen() {
    const l = wakeLock; wakeLock = null;
    if (l) l.release().catch(() => {});
  }
  const onVisible = () => { if (document.visibilityState === "visible" && !ended && pc) holdScreen(); };

  function end(label) {
    if (ended) return;
    ended = true;
    clearTimeout(dropTimer); dropTimer = null;
    document.removeEventListener("visibilitychange", onVisible);
    releaseScreen();
    pausedBy.clear();
    clearTimeout(pauseToneTimer); pauseToneTimer = null;
    thinkingSound(false);
    chimeEnd();
    stopMeter();
    cfg.onLevel(0);
    if (micStream) micStream.getTracks().forEach(t => t.stop());
    // The ring is deliberately kept: it is what the next connection
    // delivers first (§4.2). The tap and the pump are per call.
    clearTimeout(pumpTimer); pumpTimer = 0;
    if (uplinkWorker) { try { uplinkWorker.terminate(); } catch { /* gone */ } uplinkWorker = null; }
    behindShownS = 0; behind = { sounded: false };
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
    /* "channel" when speech is buffered and delivered reliably, "rtp" when
       the browser could not tap the sender and the old path is in use. */
    get uplinkMode() { return uplinkMode; },
    voiceConfig,
  };
}
