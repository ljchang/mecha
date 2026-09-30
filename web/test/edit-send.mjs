// The edit modal's send: the mask goes up and is named in the message, and is
// never put in `attachments` — those ride on the turn as pixels, and a mask
// is for `image_generate`, not for the model to look at.
//
// `npm test` in web/. Same rig as `incognito-switch.mjs`: `sendEdit` is read
// OUT of the component, so this exercises the text that ships (review of #429).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { composeEditMessage, maskName } from '../src/lib/image-edit.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Chat.svelte'), 'utf8');
const marker = '  async function sendEdit({ text, mask }) {';
const start = src.indexOf(marker);
if (start < 0) throw new Error('Chat.svelte no longer defines sendEdit');
const sendEditSrc = src.slice(start, src.indexOf('\n  }\n', start) + '\n  }\n'.length);

let passed = 0;
let failed = 0;
function is(actual, expected, what) {
  const a = JSON.stringify(actual);
  const b = JSON.stringify(expected);
  if (a === b) {
    passed++;
    console.log(`  ok    ${what}`);
  } else {
    failed++;
    console.log(`  FAIL  ${what}\n        expected ${b}\n        got      ${a}`);
  }
}

// A page with an open modal, an attachment the owner added to the composer,
// and a server that files the mask under `inbox/`.
async function page({ mask, status = 200 }) {
  const run = new Function(
    'composeEditMessage',
    'maskName',
    'text',
    'mask',
    'status',
    `'use strict';
     let key = 'chat-ab';
     let draft = '';
     let attachments = ['inbox/notes.pdf'];
     let editing = { path: 'images/pic.png', busy: false, error: null };
     const uploads = [];
     const sent = [];
     const fetch = async (url, opts) => {
       uploads.push({ url, body: opts.body });
       return { status, ok: status === 200, json: async () => ({ path: 'inbox/mask-pic-7.png' }), text: async () => 'no room' };
     };
     const closeIncognito = () => {};
     const send = async () => { sent.push({ draft, attachments: [...attachments] }); };
     ${sendEditSrc}
     return sendEdit({ text, mask }).then(() => ({ uploads, sent, editing }));`,
  );
  return run(composeEditMessage, maskName, 'make the dress green', mask, status);
}

{
  const r = await page({ mask: 'MASK-BLOB' });
  is(r.uploads.length, 1, 'a painted edit uploads its mask once');
  is(r.uploads[0].body, 'MASK-BLOB', 'the mask itself');
  is(r.uploads[0].url.startsWith('/api/chat/chat-ab/upload?name=mask-pic-'), true, 'into this chat, named for its picture');
  is(r.sent.length, 1, 'then sends once');
  is(r.sent[0].draft, 'Edit images/pic.png with mask inbox/mask-pic-7.png: make the dress green', 'naming the mask in the message');
  is(r.sent[0].attachments, ['inbox/notes.pdf'], 'and never adding it to the attachments');
  is(r.editing, null, 'and the modal closes');
}
{
  const r = await page({ mask: null });
  is(r.uploads.length, 0, 'nothing painted uploads nothing');
  is(r.sent[0].draft, 'Edit images/pic.png: make the dress green', 'and sends a plain edit');
}
{
  const r = await page({ mask: 'MASK-BLOB', status: 409 });
  is(r.sent.length, 0, 'a failed upload sends nothing');
  is([r.editing?.busy, (r.editing?.error ?? '').includes('Nothing was sent')], [false, true], 'and says so in the modal, which can close');
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
