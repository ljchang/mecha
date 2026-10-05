// What a call screen draws, in both chats (the owner's ask, 2026-10-05: the
// call shows the whole conversation, in persona chats and the assistant's
// alike). Pure, so `web/test/call-lines.mjs` imports it.

import { pictureOf } from './picture.js';
import { speakable } from './speech.js';
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
// other line as it is), the persona's replies as plain text (`speakable`, so no
// Markdown marks), and a picture as a line saying so — once, as the chat
// draws it once (`image_view` of a picture just made is the same picture).
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
      const text = speakable(e.text).trim();
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
// the line must not blink out in between. `heardAt` is how many of the
// owner's lines the transcript held when the line finished. Only the owner's:
// the persona's words come from the transcript and the reply streaming in.
export function pendingSpeech(callEntries, lines) {
  const said = (lines ?? []).filter((l) => l.who === 'user').length;
  return (callEntries ?? []).filter(
    (e) => e.who === 'user' && (e.interim || e.heardAt == null || said <= e.heardAt),
  );
}

