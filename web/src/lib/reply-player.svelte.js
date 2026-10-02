// The play button's player (the owner's ask, 2026-10-01): one reply at a
// time, spoken a piece at a time through `/api/speak` — the next piece asked
// for while this one plays, so speech starts after the first sentence rather
// than the whole reply. Tapping another reply's button, or this one's again,
// stops what is playing.
//
// One <audio> element, unlocked inside the tap: a phone plays only audio a
// tap started, and a piece fetched after an await is no longer that tap's —
// so the element is played (silence) in the tap and every piece reuses it.
import { apiFetch as fetch } from './api.js';
import { speakable, speechChunks } from './speech.js';

// Which reply is playing, by the id its ChatProse gave it; and why one could
// not, shown on that reply's button.
export const player = $state({ id: null, state: 'idle', error: null });

// 44 bytes of silent WAV, to unlock the element inside the tap.
const SILENCE = 'data:audio/wav;base64,UklGRiQAAABXQVZFZm10IBAAAAABAAEAQB8AAIA+AAACABAAZGF0YQAAAAA=';

let el = null;
let gen = 0;
let release = null;
const urls = new Set();

function element() {
  if (!el) el = new Audio();
  return el;
}

function forget() {
  for (const u of urls) URL.revokeObjectURL(u);
  urls.clear();
}

/** Stop whatever is playing. */
export function stopPlaying() {
  gen += 1;
  el?.pause();
  release?.();
  release = null;
  forget();
  player.id = null;
  player.state = 'idle';
}

/**
 * Speak `text` (a reply's Markdown) as `id`'s — or stop it, if it is the one
 * playing. `chat` is the chat the reply is in: a persona chat is spoken in
 * the persona's voice and rate, which serve decides; `voice` and `speed` are
 * the owner's own choice for the assistant's replies. Called from the tap
 * itself. A reply whose read failed is retried by the same tap, not stopped:
 * there is nothing playing to stop (review of #502).
 */
export async function playReply(id, text, { chat = null, unlock = null, voice = null, speed = null } = {}) {
  if (player.id === id && player.state !== 'error') return stopPlaying();
  stopPlaying();
  const mine = gen;
  const audio = element();
  // Inside the tap, before any await: this is what lets the pieces play.
  audio.src = SILENCE;
  audio.play().catch(() => {});
  const pieces = speechChunks(speakable(text));
  if (!pieces.length) return;
  player.id = id;
  player.state = 'loading';
  player.error = null;
  const piece = async (t) => {
    const res = await fetch('/api/speak', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        text: t,
        chat,
        unlock: unlock ?? undefined,
        voice: voice ?? undefined,
        speed: speed ?? undefined,
      }),
    });
    if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
    const url = URL.createObjectURL(await res.blob());
    urls.add(url);
    return url;
  };
  try {
    let next = piece(pieces[0]);
    for (let i = 0; i < pieces.length; i += 1) {
      const url = await next;
      if (mine !== gen) return;
      next = i + 1 < pieces.length ? piece(pieces[i + 1]) : null;
      // A failure ahead is met when its turn comes, not as an unhandled one.
      next?.catch(() => {});
      audio.src = url;
      player.state = 'playing';
      await new Promise((resolve, reject) => {
        release = resolve;
        audio.onended = resolve;
        audio.onerror = () => reject(new Error('this piece could not be played'));
        audio.play().catch(reject);
      });
      URL.revokeObjectURL(url);
      urls.delete(url);
      if (mine !== gen) return;
    }
    stopPlaying();
  } catch (e) {
    if (mine !== gen) return;
    forget();
    player.state = 'error';
    player.error = String(e?.message ?? e);
  }
}
