import assert from 'node:assert/strict';
import { apiFetch } from '../src/lib/api.js';
const calls = [];
globalThis.fetch = async (...args) => { calls.push(args); return { ok: true }; };
for (const method of ['POST', 'post', 'PUT', 'PATCH', 'DELETE']) {
  const body = new Blob(['file bytes']);
  await apiFetch('/api/chat/main/upload?name=x', { method, body, headers: { 'content-type': 'text/plain' } });
  const [path, options] = calls.at(-1);
  assert.equal(path, '/api/chat/main/upload?name=x');
  assert.equal(options.headers.get('x-mecha-request'), '1');
  assert.equal(options.headers.get('content-type'), 'text/plain');
  assert.equal(options.body, body);
  assert.equal(options.mode, 'same-origin');
}
await apiFetch('/api/outbox/x/approve', { method: 'POST' });
assert.equal(calls.at(-1)[1].headers.get('x-mecha-request'), '1');
await apiFetch('/api/ping');
assert.equal(calls.at(-1)[1].headers.has('x-mecha-request'), false);
assert.throws(() => apiFetch('https://elsewhere.example/api/send', { method: 'POST' }));
assert.throws(() => apiFetch('//elsewhere.example/api/send', { method: 'POST' }));
console.log('api request protection: ok');
