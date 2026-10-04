// The play button's player (the owner's ask, 2026-10-01): one reply at a
// time, spoken a piece at a time through `/api/speak`, each piece streamed —
// played from its first chunk while the rest is still being made, the way a
// call is heard (docs/VOICE-BREEZE-DESIGN.md §3.1). The next piece is asked
// for as soon as the last has arrived, not when it has been heard: the
// engine speaks faster than real time, so the voice stays ahead of the
// listener and the pieces play as one stream. Tapping another reply's
// button, or this one's again, stops what is playing.
//
// One AudioContext, resumed inside the tap: a phone plays only audio a tap
// started, and a piece fetched after an await is no longer that tap's — so
// the context is woken in the tap, and every piece plays through it.
import { micOpen } from '../../../scripts/voice/voice-core.js';
import { apiFetch as fetch } from './api.js';
import { listenSession, pcmSamples, Schedule, Stall } from './pcm.js';
import { replyKey, speakable, speechPieces } from './speech.js';

// Which reply is playing, by the id its ChatProse gave it; and why one could
// not, shown on that reply's button.
export const player = $state({ id: null, state: 'idle', error: null });

// How far the voice may run ahead of the listener before the next piece
// waits: enough that a slow stretch (a chat sharing the GPU) is never heard,
// little enough that a stop wastes little synthesis.
const MAX_AHEAD = 20;
// The shortest run of samples given its own buffer while the listener is
// not waiting: a network chunk can be a few milliseconds, and a node per
// chunk is work for nothing.
const MIN_RUN = 0.1;
// The rate the voice worker streams at (`PCM_RATE`), and the context's own:
// a run at the context's rate is played as it is, where one at another rate
// is resampled node by node with no state carried across a join (review of
// #555; Chromium measured clean at 44.1 kHz either way, other engines not).
const RATE = 24000;

let ctx = null;
// Whether this player set the phone's audio session, and so must hand it
// back: one it found set otherwise is left alone both ways.
let tookSession = false;
let gen = 0;
let abort = null;
let wake = null;
const sources = new Set();

function context() {
  if (!ctx) {
    const Ctx = globalThis.AudioContext ?? globalThis.webkitAudioContext;
    try {
      ctx = new Ctx({ sampleRate: RATE });
    } catch {
      // An engine that takes no rate plays at its own, resampling each run.
      ctx = new Ctx();
    }
  }
  return ctx;
}

/** Inside the tap: wake the context and play a sample of silence, which is
 * what an older iPhone needs before a context sounds at all; and ask for
 * playback rather than ambient audio, so the ringer switch does not mute a
 * reply the owner asked to hear — unless a call holds the microphone, which
 * playback would silence (`listenSession`). */
function unlock(c) {
  c.resume?.().catch(() => {});
  const s = c.createBufferSource();
  s.buffer = c.createBuffer(1, 1, 22050);
  s.connect(c.destination);
  s.start(0);
  try {
    const session = navigator.audioSession;
    const want = session ? listenSession(session.type, micOpen()) : null;
    if (want) {
      session.type = want;
      tookSession = true;
    }
  } catch {}
}

/** Hand the phone's audio session back, if this player took it and nothing
 * (a call starting meanwhile) has changed it since. */
function relinquish() {
  try {
    if (tookSession && navigator.audioSession?.type === 'playback') navigator.audioSession.type = 'auto';
  } catch {}
  tookSession = false;
}

/** Stop whatever is playing. */
export function stopPlaying() {
  gen += 1;
  abort?.abort();
  abort = null;
  for (const s of sources) {
    try {
      s.stop();
    } catch {}
  }
  sources.clear();
  wake?.();
  wake = null;
  relinquish();
  player.id = null;
  player.state = 'idle';
}

/** Wait `ms`, or less if the reply is stopped. */
function nap(ms) {
  return new Promise((resolve) => {
    const t = setTimeout(resolve, ms);
    wake = () => {
      clearTimeout(t);
      resolve();
    };
  });
}

/**
 * Speak `text` (a reply's Markdown) as `id`'s — or stop it, if it is the one
 * playing. `chat` is the chat the reply is in: a persona chat is spoken in
 * the persona's voice and rate, which serve decides; `voice` and `speed` are
 * the owner's own choice for the assistant's replies. `asked` and
 * `lastReply` are the moment the reply was said in (`replyContext`), which
 * serve's director reads, with the reply itself, to settle the one
 * direction every piece is spoken with (Listen's director pass).
 * Called from the tap itself. A reply whose read failed is retried by the same tap, not stopped:
 * there is nothing playing to stop (review of #502).
 */
export async function playReply(
  id,
  text,
  { chat = null, unlock: grant = null, voice = null, speed = null, asked = null, lastReply = null } = {},
) {
  if (player.id === id && player.state !== 'error') return stopPlaying();
  stopPlaying();
  const mine = gen;
  const said = speakable(text);
  const pieces = speechPieces(said);
  const reply = replyKey(said);
  // Nothing to say touches nothing: a phone's session set to playback here
  // would have no stop to hand it back (review of #555).
  if (!pieces.length) return;
  // Inside the tap, before any await: this is what lets the pieces play.
  const c = context();
  unlock(c);
  player.id = id;
  player.state = 'loading';
  player.error = null;
  const controller = new AbortController();
  abort = controller;
  const clock = new Schedule();
  // A context that will not run raises nothing; its clock standing still
  // with speech queued is the one sign, and it is said rather than shown as
  // "Stop" over silence forever (review of #555).
  const stall = new Stall();
  const check = () => {
    const visible = typeof document === 'undefined' || document.visibilityState === 'visible';
    if (stall.frozen(c.currentTime, performance.now(), clock.ahead(c.currentTime) > 0, visible)) {
      throw new Error('the audio would not play — tap Listen to try again');
    }
  };

  const play = (samples, rate) => {
    if (!samples.length || mine !== gen) return;
    const buffer = c.createBuffer(1, samples.length, rate);
    buffer.copyToChannel(samples, 0);
    const source = c.createBufferSource();
    source.buffer = buffer;
    source.connect(c.destination);
    source.onended = () => sources.delete(source);
    sources.add(source);
    source.start(clock.place(c.currentTime, buffer.duration));
    player.state = 'playing';
  };

  const piece = async (t, index) => {
    const res = await fetch('/api/speak', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      signal: controller.signal,
      body: JSON.stringify({
        text: t,
        chat,
        unlock: grant ?? undefined,
        voice: voice ?? undefined,
        speed: speed ?? undefined,
        stream: true,
        listen: {
          reply,
          index,
          whole: said.slice(0, 2000),
          asked: asked ?? undefined,
          last_reply: lastReply ?? undefined,
        },
      }),
    });
    if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
    if (!(res.headers.get('content-type') ?? '').startsWith('audio/pcm')) {
      // A worker older than streaming: the piece is one WAV, heard once it
      // has all arrived, through the same clock.
      const buffer = await c.decodeAudioData(await res.arrayBuffer());
      if (mine !== gen) return;
      play(buffer.getChannelData(0), buffer.sampleRate);
      return;
    }
    const rate = Number(res.headers.get('x-sample-rate'));
    if (!Number.isFinite(rate) || rate <= 0) throw new Error('the voice streamed at no rate');
    const reader = res.body.getReader();
    const min = Math.ceil(rate * MIN_RUN);
    let carry = null;
    let held = [];
    let heldLen = 0;
    const flush = () => {
      if (!heldLen) return;
      const run = new Float32Array(heldLen);
      let at = 0;
      for (const h of held) {
        run.set(h, at);
        at += h.length;
      }
      held = [];
      heldLen = 0;
      play(run, rate);
    };
    for (;;) {
      const { done, value } = await reader.read();
      if (mine !== gen) return;
      if (done) break;
      const out = pcmSamples(value, carry);
      carry = out.carry;
      held.push(out.samples);
      heldLen += out.samples.length;
      // A run goes once it is long enough to be worth a buffer — or at once
      // when the listener would otherwise be waiting on it.
      if (heldLen >= min || clock.ahead(c.currentTime) < MIN_RUN) flush();
      check();
    }
    flush();
  };

  try {
    for (let i = 0; i < pieces.length; i += 1) {
      while (clock.ahead(c.currentTime) > MAX_AHEAD) {
        await nap(500);
        if (mine !== gen) return;
        check();
      }
      await piece(pieces[i], i);
      if (mine !== gen) return;
    }
    // Everything is queued; the reply ends when the last of it is heard.
    while (clock.ahead(c.currentTime) > 0) {
      await nap(Math.min(1000, clock.ahead(c.currentTime) * 1000 + 50));
      if (mine !== gen) return;
      check();
    }
    stopPlaying();
  } catch (e) {
    if (mine !== gen) return;
    stopPlaying();
    player.id = id;
    player.state = 'error';
    player.error = String(e?.message ?? e);
  }
}
