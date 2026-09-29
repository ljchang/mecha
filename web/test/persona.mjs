// The Personas tab's pure logic, imported from the shipped module.
import assert from 'node:assert/strict';
import {
  isPersonaKey, withUnlock, listUrl, personaUrl, chatUrl, relationshipLabel, emptyRun, applyEvent, ENDPOINTS,
} from '../src/lib/persona.js';

// Only a key the server could have minted is a persona chat's.
assert.ok(isPersonaKey('p-0123456789ab'));
for (const bad of ['main', 'chat-abc', 'p-', 'p-0123456789AB', 'p-0123456789abc', '../p-0123456789ab', null]) {
  assert.ok(!isPersonaKey(bad), String(bad));
}

// Every URL is the persona door's; an assistant key is refused outright, so
// this page can never read or write an assistant chat.
assert.equal(chatUrl('p-0123456789ab'), '/api/persona-chat/p-0123456789ab');
assert.equal(chatUrl('p-0123456789ab', '/events', 't k'), '/api/persona-chat/p-0123456789ab/events?unlock=t%20k');
assert.throws(() => chatUrl('main', '/send'));
assert.throws(() => chatUrl('chat-abcdef'));
assert.equal(listUrl(null), '/api/personas');
assert.equal(listUrl('abc'), '/api/personas?unlock=abc');
assert.equal(personaUrl('devils_advocate', '/chats', null), '/api/personas/devils_advocate/chats');
assert.equal(withUnlock('/x?a=1', 'z'), '/x?a=1&unlock=z');

// A builder reaches only what ENDPOINTS lists — the list check-demo holds
// the docs demo to — so a new endpoint cannot slip past that guard.
assert.throws(() => personaUrl('mara', '/delete', null));
assert.throws(() => chatUrl('p-0123456789ab', '/mode'));
assert.ok(ENDPOINTS.includes('/api/persona-chat/X/events'));
assert.equal(ENDPOINTS.length, 7);

assert.equal(relationshipLabel({ relationship: ['colleague', 'devils_advocate'] }), 'colleague · devils advocate');
assert.equal(relationshipLabel({}), '');

// A run, folded event by event: the owner's turn, a tool, streamed text,
// then the end — the streamed text becomes the answer.
let s = emptyRun([{ kind: 'user', text: 'earlier' }]);
s = applyEvent(s, { type: 'user', text: 'Hello, Mara', spoken: false });
assert.equal(s.running, true);
s = applyEvent(s, { type: 'tool', id: 't1', name: 'image_view' });
s = applyEvent(s, { type: 'tool_result', id: 't1', name: 'image_view', is_error: false, preview: '' });
s = applyEvent(s, { type: 'delta', text: 'Hello ' });
s = applyEvent(s, { type: 'delta', text: 'back.' });
assert.equal(s.streaming, 'Hello back.');
s = applyEvent(s, { type: 'done', ok: true, stop: 'EndTurn' });
assert.equal(s.running, false);
assert.equal(s.streaming, null);
assert.deepEqual(
  s.entries.map((e) => [e.kind, e.text ?? e.name, e.is_error ?? null]),
  [
    ['user', 'earlier', null],
    ['user', 'Hello, Mara', null],
    ['tool', 'image_view', false],
    ['assistant', 'Hello back.', null],
  ],
);

// A steer resolves: delivered into the run, or discarded as too late —
// never left reading "queued" (review of #415).
s = emptyRun();
s = applyEvent(s, { type: 'queued', text: 'also this', request_id: 'r-1' });
s = applyEvent(s, { type: 'queued', text: 'and this', request_id: 'r-2' });
s = applyEvent(s, { type: 'queued_delivered', request_id: 'r-1' });
s = applyEvent(s, { type: 'queued_discarded', request_ids: ['r-2'] });
assert.deepEqual(s.entries.map((e) => e.delivery), ['delivered', 'discarded']);

// A failed turn says so rather than ending silently.
s = applyEvent(emptyRun(), { type: 'done', ok: false, error: 'model unavailable' });
assert.deepEqual(s.entries, [{ kind: 'notice', text: 'model unavailable' }]);

// An event this page does not know changes nothing.
const before = emptyRun([{ kind: 'user', text: 'x' }]);
assert.equal(applyEvent(before, { type: 'affect', label: 'calm' }), before);

console.log('persona: ok');
