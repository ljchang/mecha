// What a call screen draws, in both chats (`call-lines.js`).
import assert from 'node:assert/strict';
import { historyLines, pendingSpeech } from '../src/lib/call-lines.js';

// The conversation as a call screen shows it (the owner's ask, 2026-10-05):
// what was said and drawn, in order, with the owner's words as their bubble
// shows them and replies as plain text.
{
  const before = [
    { kind: 'user', text: '(What I want from this conversation: rest)\n\nDid the ferry sail on time?' },
    { kind: 'tool', name: 'memory_search', preview: '3 results' },
    { kind: 'assistant', text: 'It **did** — it left at [half past nine](https://example.com).' },
    { kind: 'tool', name: 'image_generate', preview: 'image: images/20261005-1.png' },
    { kind: 'tool', name: 'image_generate', is_error: true, preview: 'Cancelled — nothing was saved.' },
    { kind: 'notice', text: 'the model changed' },
    { kind: 'assistant', text: '   ' },
    { kind: 'user', text: 'Good to know, thanks.' },
    { kind: 'user', text: 'Pack the blue umbrella.', queued: true, delivery: 'discarded' },
    { kind: 'user', text: 'And the red one.', queued: true, delivery: 'delivered' },
  ];
  assert.deepEqual(historyLines(before), [
    { who: 'user', text: 'Did the ferry sail on time?' },
    { who: 'persona', text: 'It did — it left at half past nine.' },
    { who: 'persona', picture: true, text: 'a picture' },
    { who: 'user', text: 'Good to know, thanks.' },
    // Never received: not part of the conversation. Steered in: it was.
    { who: 'user', text: 'And the red one.' },
  ]);
  // A picture made and then viewed is one picture, as the chat draws it once.
  const twice = [
    { kind: 'tool', name: 'image_generate', preview: 'image: images/a.png' },
    { kind: 'tool', name: 'image_view', preview: 'image: images/a.png' },
  ];
  assert.equal(historyLines(twice).length, 1);
  // Nothing before the call: nothing drawn above it.
  assert.deepEqual(historyLines([]), []);
  assert.deepEqual(historyLines(undefined), []);
}

// The owner's speech below the transcript: still being heard, or finished
// and not yet in the transcript — never twice once the transcript has it,
// never the persona's words (which come from the transcript), and taken by
// text, so a re-read that folds two lines into one draws nothing twice.
{
  const lines = [{ who: 'user', text: 'Is the bakery open?' }, { who: 'persona', text: 'Until six.' }];
  const call = [
    { who: 'user', text: 'is the bakery open', interim: false },
    { who: 'persona', text: 'Until six.', interim: false },
    { who: 'user', text: 'And tomor', interim: true },
  ];
  // The first line is in the transcript (case and marks aside): only the
  // line still being heard is drawn.
  assert.deepEqual(pendingSpeech(call, lines).map((e) => e.text), ['And tomor']);
  // Finished, and the transcript has not taken it yet: still drawn.
  call[2] = { who: 'user', text: 'And tomorrow?', interim: false };
  assert.deepEqual(pendingSpeech(call, lines).map((e) => e.text), ['And tomorrow?']);
  // The transcript takes it: gone from below.
  const taken = [...lines, { who: 'user', text: 'And tomorrow?' }];
  assert.deepEqual(pendingSpeech(call, taken), []);
  // A re-read that folded the two owner lines into one: both are in it, so
  // neither is drawn again — where a count of owner lines would have shrunk.
  const folded = [{ who: 'user', text: 'Is the bakery open?\n\nAnd tomorrow?' }, { who: 'persona', text: 'Until six.' }];
  assert.deepEqual(pendingSpeech(call, folded), []);
  // A word inside another word is not the line ("art" is not in "start").
  assert.deepEqual(pendingSpeech([{ who: 'user', text: 'Art', interim: false }], [{ who: 'user', text: 'Start now' }]).map((e) => e.text), ['Art']);
  // A finished line with nothing in it is never drawn.
  assert.deepEqual(pendingSpeech([{ who: 'user', text: ' … ', interim: false }], []), []);
  assert.deepEqual(pendingSpeech(undefined, undefined), []);
  // The call's own notices always show — the pane is their only surface —
  // and the persona's speech never does: it is in the transcript.
  const withNotice = [
    { who: 'bot', text: 'Until six.', interim: false },
    { who: 'notice', text: 'voice: the microphone path stopped - tap to reconnect', interim: false },
  ];
  assert.deepEqual(pendingSpeech(withNotice, lines).map((e) => e.who), ['notice']);
  // Nothing the owner says or types retires a notice: speech is no evidence
  // a dead mic or lost audio came back. (A reconnect clears them, page side.)
  const later = [...withNotice, { who: 'user', text: 'Is the bakery open?', interim: false },
    { who: 'user', text: 'Hello?', interim: false }];
  assert.deepEqual(pendingSpeech(later, lines).map((e) => e.who), ['notice', 'user']);
}

console.log('call-lines: ok');
