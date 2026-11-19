import assert from 'node:assert/strict';
import { answeredChat, chatHref, resumesInChat } from '../src/lib/question-resume.js';

// Which questions say they will open as a chat.
assert.equal(resumesInChat({ resume: 'chat' }), true);
assert.equal(resumesInChat({ resume: 'background' }), false);
assert.equal(resumesInChat({}), false, 'an older server sends no field: the old path');
assert.equal(resumesInChat(null), false);

// The chat an answer continued — only from a chat resume's reply.
assert.equal(answeredChat({ resumed: 'chat', key: 'r-0a1b2c3d' }), 'r-0a1b2c3d');
assert.equal(
  answeredChat({ ok: true, detached: true, note: 'resuming' }),
  null,
  'the detached background resume has no chat to open',
);
assert.equal(answeredChat({ resumed: 'chat' }), null, 'no key, no link made up');
assert.equal(answeredChat({ resumed: 'chat', key: '  ' }), null);
assert.equal(answeredChat(null), null);

assert.equal(chatHref('r-0a1b2c3d'), '#chat/r-0a1b2c3d');
assert.equal(chatHref('a b'), '#chat/a%20b');
console.log('question resume: ok');
