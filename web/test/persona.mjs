// The Personas tab's pure logic, imported from the shipped module.
import assert from 'node:assert/strict';
import {
  isPersonaKey, withUnlock, listUrl, personaUrl, chatUrl, relationshipLabel, emptyRun, applyEvent, ENDPOINTS, settle,
  taintLabel, safetyLine, doseLine, personaName, authoringUrl, OWNER_FILES,
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
assert.equal(ENDPOINTS.length, 12);
assert.ok(ENDPOINTS.includes('/api/personas/authoring') && ENDPOINTS.includes('/api/personas/X/files'));
assert.equal(authoringUrl('t'), '/api/personas/authoring?unlock=t');
assert.equal(personaUrl('mara', '/files', null), '/api/personas/mara/files');
// A name as the store holds it, or null for the obvious refusals.
assert.equal(personaName('  Mara '), 'mara');
assert.equal(personaName('devils_advocate'), 'devils_advocate');
for (const bad of ['', 'files', '-x', 'a b', 'a/b', 'x'.repeat(65)]) assert.equal(personaName(bad), null, bad);
assert.deepEqual(OWNER_FILES.map(([f]) => f), ['identity', 'motivation', 'settings']);

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

// A re-read after the run keeps what only the page held: the server's
// entries, then the notice and the undelivered steer — not dropped.
{
  let r = emptyRun([{ kind: 'user', text: 'Hi' }]);
  r = applyEvent(r, { type: 'queued', text: 'late', request_id: 'r-9' });
  r = applyEvent(r, { type: 'notice', text: 'switching models' });
  r = applyEvent(r, { type: 'queued_discarded', request_ids: ['r-9'] });
  const server = [{ kind: 'user', text: 'Hi' }, { kind: 'assistant', text: 'Hello.' }];
  assert.deepEqual(
    settle(server, r).map((e) => [e.kind, e.text]),
    [['user', 'Hi'], ['assistant', 'Hello.'], ['user', 'late'], ['notice', 'switching models']],
  );
}

// A crisis pause is drawn as its own card and survives a re-read; the run
// ends, and what it touched is kept for the chip.
{
  let r = emptyRun([{ kind: 'user', text: 'hi' }]);
  r = applyEvent(r, { type: 'user', text: 'something hard' });
  r = applyEvent(r, { type: 'crisis', text: 'This is mecha, not the character… 988' });
  r = applyEvent(r, { type: 'done', ok: true, stop: 'CrisisPause', taint_private: true, taint_untrusted: false });
  assert.equal(r.running, false);
  assert.deepEqual(r.taint, { private: true, untrusted: false });
  assert.deepEqual(settle([{ kind: 'user', text: 'hi' }], r).map((e) => e.kind), ['user', 'crisis']);
}
assert.equal(taintLabel({ private: true, untrusted: true }), 'private + untrusted');
assert.equal(taintLabel({ private: false, untrusted: false }), '');
assert.equal(taintLabel(null), '');
assert.equal(safetyLine({ crisis: 'on', disclosure: true, reanchor: true, dose: true }), 'crisis detection on');
assert.equal(safetyLine({ crisis: 'degraded', disclosure: true, reanchor: true, dose: true }), 'crisis detection: keywords only (the model check could not answer)');
// A state this page does not know reads as the cautious one, never as "on".
assert.ok(safetyLine({ crisis: 'judged-v2' }).includes('keywords only'));
assert.equal(safetyLine({ crisis: 'off', disclosure: false, reanchor: true, dose: false }), 'crisis detection off · off: disclosure, dose');
assert.equal(safetyLine({ crisis: 'on', disclosure: true, reanchor: true, dose: true, farewell: 'off' }), 'crisis detection on · off: farewell');
// Two crisis cards get two ids, and a re-read keeps each card's own.
{
  let r = emptyRun();
  r = applyEvent(r, { type: 'crisis', text: 'a' });
  r = applyEvent(r, { type: 'crisis', text: 'b' });
  const ids = r.entries.map((e) => e.id);
  assert.equal(new Set(ids).size, 2);
  assert.deepEqual(settle([{ kind: 'user', text: 'x' }], r).filter((e) => e.kind === 'crisis').map((e) => e.id), ids);
}
assert.equal(doseLine({ turns_today: 3, turns_7d: 12, late_night_7d: 2 }), '3 today · 12 this week · 2 late at night');
assert.equal(doseLine({ turns_today: 0, turns_7d: 0, late_night_7d: 0 }), '0 today · 0 this week');
assert.equal(doseLine(null), '');
assert.equal(doseLine({ unread: 'Permission denied' }), 'usage meters unreadable');
assert.equal(doseLine({ turns_today: 1, turns_7d: 2, late_night_7d: 0, skipped: 3 }), '1 today · 2 this week · 3 unreadable records not counted');

// A failed turn says so rather than ending silently.
s = applyEvent(emptyRun(), { type: 'done', ok: false, error: 'model unavailable' });
assert.deepEqual(s.entries, [{ kind: 'notice', text: 'model unavailable' }]);

// An event this page does not know changes nothing.
const before = emptyRun([{ kind: 'user', text: 'x' }]);
assert.equal(applyEvent(before, { type: 'affect', label: 'calm' }), before);

console.log('persona: ok');
