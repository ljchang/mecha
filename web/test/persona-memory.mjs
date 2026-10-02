// The persona memory page's pure logic, imported from the shipped module.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { personaUrl, ENDPOINTS, memorySections, memoryText, memoryActBody, MEMORY_KINDS } from '../src/lib/persona.js';

// The endpoint is one the builder accepts, and so one the docs demo checks.
assert.equal(personaUrl('mara', '/memory', 'tok'), '/api/personas/mara/memory?unlock=tok');
assert.ok(ENDPOINTS.includes('/api/personas/X/memory'));

// What waits on the owner comes first, from every kind; the rest by kind.
const data = {
  episodes: [{ uid: 'e1', summary: 'Talked kelp.', status: 'active' }],
  facts: {
    persona: [{ uid: 'p1', text: 'Grew up on the coast.', status: 'active' }],
    user: [
      { uid: 'u1', text: 'Teaches on Thursdays.', status: 'active' },
      { uid: 'u2', text: 'Read about kelp.', status: 'candidate' },
    ],
    inferred: [{ uid: 'i1', text: 'Likes early starts.', status: 'candidate' }],
  },
};
const s = memorySections(data);
assert.deepEqual(s.waiting.map((r) => r.uid), ['u2', 'i1']);
assert.deepEqual(s.kept.user.map((r) => r.uid), ['u1']);
assert.equal(s.kept.inferred.length, 0);
assert.equal(s.waiting[0].kind, 'user', 'a waiting record keeps its kind');
assert.equal(s.empty, false);
assert.equal(memoryText(s.kept.episodes[0]), 'Talked kelp.');
assert.equal(memoryText(s.kept.persona[0]), 'Grew up on the coast.');
assert.deepEqual(MEMORY_KINDS.map(([k]) => k), ['episodes', 'persona', 'user', 'inferred']);

// Nothing, or an older server's missing lists: an empty page, never a throw.
assert.equal(memorySections(null).empty, true);
assert.equal(memorySections({ episodes: [] }).empty, true);

// Only an act the server takes can be sent; the token rides in the body.
assert.deepEqual(memoryActBody('correct', 'u1', { text: 'Tuesdays.' }, 'tok'),
  { action: 'correct', id: 'u1', text: 'Tuesdays.', unlock: 'tok' });
assert.equal(memoryActBody('pin', 'u1').unlock, undefined);
assert.throws(() => memoryActBody('delete_everything', 'u1'));

// The Memories tab is not an owner file: kept out of the list whose every
// entry the editor saves as a file.
const page = fs.readFileSync(new URL('../src/lib/Personas.svelte', import.meta.url), 'utf8');
assert.ok(page.includes('<PersonaMemory '), 'the editor shows the memory page');
const persona = fs.readFileSync(new URL('../src/lib/persona.js', import.meta.url), 'utf8');
const owner = persona.slice(persona.indexOf('export const OWNER_FILES'), persona.indexOf('];', persona.indexOf('export const OWNER_FILES')));
assert.ok(!/memor/i.test(owner), 'Memories is not an owner file');

console.log('persona-memory: ok');
