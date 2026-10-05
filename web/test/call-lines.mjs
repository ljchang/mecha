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
// and never the persona's words, which come from the transcript.
{
  const lines = [{ who: 'user', text: 'Is the bakery open?' }, { who: 'persona', text: 'Until six.' }];
  const call = [
    { who: 'user', text: 'Is the bakery open?', interim: false, heardAt: 0 },
    { who: 'persona', text: 'Until six.', interim: false, heardAt: null },
    { who: 'user', text: 'And tomor', interim: true, heardAt: null },
  ];
  // The first line is in the transcript (one owner line now, heard at zero):
  // only the line still being heard is drawn.
  assert.deepEqual(pendingSpeech(call, lines).map((e) => e.text), ['And tomor']);
  // Finished, and the transcript has not taken it yet: still drawn.
  call[2] = { who: 'user', text: 'And tomorrow?', interim: false, heardAt: 1 };
  assert.deepEqual(pendingSpeech(call, lines).map((e) => e.text), ['And tomorrow?']);
  // The transcript takes it: gone from below, drawn once, above.
  const taken = [...lines, { who: 'user', text: 'And tomorrow?' }];
  assert.deepEqual(pendingSpeech(call, taken), []);
  assert.deepEqual(pendingSpeech(undefined, undefined), []);
}

console.log('call-lines: ok');
