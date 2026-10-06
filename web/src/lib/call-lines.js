// What a call screen draws, in both chats (the owner's ask, 2026-10-05: the
// call shows the whole conversation, in persona chats and the assistant's
// alike). Pure, so `web/test/call-lines.mjs` imports it.

import { pictureOf } from './picture.js';
import { ownWords } from './persona.js';

// A line of the owner's that the chat received: not one still queued or
// discarded (its bubble says "not delivered"). Steered lines count.
function said(e) {
  return e.kind === 'user' && !(e.queued && e.delivery !== 'delivered');
}

// The conversation as the call screen shows it: the chat's own transcript,
// which a call's turns join live — one source, so nothing has to mark where
// the call began, and nothing a re-read rebuilds can draw a line twice. Only
// what was said and drawn: the owner's words as their bubble shows them
// (`ownWords`, which drops a persona chat's goal preamble and leaves any
// other line as it is), the persona's replies as written — the screen draws
// them with the chat's own renderer, so a call and the chat format a reply
// the same way (the owner's ask, 2026-10-06; what the voice says is tidied
// on the speech path, never here) — and a picture as a line saying so —
// once, as the chat draws it once (`image_view` of a picture just made is
// the same picture).
// Tool rows, notices, empty replies and lines the persona never received are
// the chat's detail, not the conversation.
export function historyLines(entries) {
  const lines = [];
  const pictures = new Set();
  for (const e of entries ?? []) {
    if (e.kind === 'user') {
      if (!said(e)) continue;
      const text = ownWords(e.text).trim();
      if (text) lines.push({ who: 'user', text });
    } else if (e.kind === 'assistant') {
      const text = (e.text ?? '').trim();
      if (text) lines.push({ who: 'persona', text });
    } else if (e.kind === 'tool') {
      const picture = pictureOf(e);
      if (picture && !pictures.has(picture)) {
        pictures.add(picture);
        lines.push({ who: 'persona', picture: true, text: 'a picture' });
      }
    }
  }
  return lines;
}

// The owner's speech the call screen draws below the transcript: what is
// still being heard (interim), and a finished line until the transcript has
// taken it — the turn reaches the chat a moment after the speech ends, and
// the line must not blink out in between. "Taken" is by text: a finished line
// is in the transcript once one of its last few owner lines contains it.
// Containment, because the server may fold two plain lines into one message
// on a re-read; never a count, which that fold changes (review of #570). A
// repeated short line ("yes") may count as taken a moment early: a blink, and
// one that corrects itself, where a stale count drew a line twice for good.
// The call's own notices (`who: "notice"` — a dead mic, audio that cannot
// reach the owner, a dropped typed line, an error) show for the rest of the
// call, as they did before this screen drew the transcript: the pane is their
// only surface (a silent one over a dead mic is the state #534 was fixed
// for), and nothing the owner says is evidence the condition has passed. A
// reconnect — the action they ask for — clears them on the page side.
// Never the persona's speech, which comes from the transcript and the reply
// streaming in.
export function pendingSpeech(callEntries, lines) {
  const norm = (t) => (t ?? '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  const shown = (callEntries ?? []).filter((e) => e.who === 'user' || e.who === 'notice');
  const finished = shown.filter((e) => e.who === 'user' && !e.interim).length;
  const recent = (lines ?? [])
    .filter((l) => l.who === 'user')
    .slice(-(finished + 2))
    .map((l) => ` ${norm(l.text)} `);
  return shown.filter((e) => {
    if (e.who === 'notice' || e.interim) return true;
    const t = norm(e.text);
    return t !== '' && !recent.some((r) => r.includes(` ${t} `));
  });
}
