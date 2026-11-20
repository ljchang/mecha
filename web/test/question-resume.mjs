import assert from 'node:assert/strict';
import { answeredChat, chatHref, resumesInChat } from '../src/lib/question-resume.js';

// Which questions say they will open as a chat.
assert.equal(resumesInChat({ resume: 'chat' }), true);
assert.equal(resumesInChat({ resume: 'background' }), false);
assert.equal(resumesInChat({}), false, 'an older server sends no field: the old path');
assert.equal(resumesInChat(null), false);

// The chat an answer continued — only from a chat resume's reply.
assert.equal(
  answeredChat({ resumed: 'chat', key: 'r-0a1b2c3d', session: '20261119T090000-abcd' }),
  '20261119T090000-abcd',
  'the session id, which #chat/<id> resumes; the key would 404 there',
);
assert.equal(answeredChat({ resumed: 'chat', key: 'r-0a1b2c3d' }), null, 'a key alone is no link');
assert.equal(
  answeredChat({ ok: true, detached: true, note: 'resuming' }),
  null,
  'the detached background resume has no chat to open',
);
assert.equal(answeredChat({ resumed: 'chat' }), null, 'no session, no link made up');
assert.equal(answeredChat({ resumed: 'chat', session: '  ' }), null);
assert.equal(answeredChat(null), null);

assert.equal(chatHref('20261119T090000-abcd'), '#chat/20261119T090000-abcd');
assert.equal(chatHref('a b'), '#chat/a%20b');
console.log('question resume: ok');
