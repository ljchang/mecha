// What the play button's player does with a streamed piece, apart from the
// browser: raw PCM bytes into samples, and where on the audio clock each run
// of samples goes (`reply-player.svelte.js`; docs/VOICE-BREEZE-DESIGN.md
// §3.1). Pure, so it is tested without a browser (`test/pcm.mjs`).

/**
 * Signed 16-bit little-endian mono `bytes` as samples in [-1, 1), with the
 * byte left over when a chunk ends mid-sample: a network chunk is not
 * sample-aligned, and a dropped odd byte would shift every sample after it
 * into noise. `carry` is the last call's leftover, or null.
 */
export function pcmSamples(bytes, carry = null) {
  let data = bytes;
  if (carry && carry.length) {
    data = new Uint8Array(carry.length + bytes.length);
    data.set(carry, 0);
    data.set(bytes, carry.length);
  }
  const n = data.length >> 1;
  const samples = new Float32Array(n);
  const view = new DataView(data.buffer, data.byteOffset, n * 2);
  for (let i = 0; i < n; i += 1) samples[i] = view.getInt16(i * 2, true) / 32768;
  const rest = data.length & 1 ? data.slice(data.length - 1) : null;
  return { samples, carry: rest };
}

/**
 * The audio clock's plan for one reply: each run of samples starts where
 * the last ends, so the pieces of a reply play as one stream. When the
 * voice falls behind the listener — nothing queued, or the next run late —
 * the run starts `lead` seconds from now instead: a short pause, never one
 * run on top of another and never a start in the past, which a browser
 * plays at once and so out of place.
 */
export class Schedule {
  constructor(lead = 0.15) {
    this.lead = lead;
    this.end = 0;
  }

  /** When a run of `seconds` should start, given the clock reads `now`. */
  place(now, seconds) {
    const at = this.end > now + 0.01 ? this.end : now + this.lead;
    this.end = at + seconds;
    return at;
  }

  /** How much is queued and not yet heard at `now`, in seconds. */
  ahead(now) {
    return Math.max(0, this.end - now);
  }
}
