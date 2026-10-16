// The edit modal's send: the mask goes up and is named in the message, and is
// never put in `attachments` — those ride on the turn as pixels, and a mask
// is for `image_generate`, not for the model to look at.
//
// `npm test` in web/. Same rig as `incognito-switch.mjs`: `sendEdit` is read
// OUT of the component, so this exercises the text that ships (review of #429).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { composeEditMessage, composeRegionsMessage, maskName } from '../src/lib/image-edit.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Chat.svelte'), 'utf8');
const marker = '  async function sendEdit({ text, mask, regions = [] }) {';
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
async function page({ mask, status = 200, regions = [], running = false, starts = true }) {
  const run = new Function(
    'composeEditMessage',
    'composeRegionsMessage',
    'maskName',
    'text',
    'mask',
    'status',
    'regions',
    'running',
    'starts',
    `'use strict';
     let key = 'chat-ab';
     let draft = 'a red scarf';
     let attachments = ['inbox/notes.pdf'];
     let editing = { path: 'images/pic.png', busy: false, error: null };
     const uploads = [];
     const sent = [];
     const fetch = async (url, opts) => {
       uploads.push({ url, body: opts.body });
       return { status, ok: status === 200, json: async () => ({ path: 'inbox/mask-pic-7.png' }), text: async () => 'no room' };
     };
     const closeIncognito = () => {};
     const send = async (opts = {}) => {
       sent.push({ draft, attachments: [...attachments], opts });
       return starts;
     };
     ${sendEditSrc}
     return sendEdit({ text, mask, regions }).then(() => ({ uploads, sent, editing, draft }));`,
  );
  return run(composeEditMessage, composeRegionsMessage, maskName, 'make the dress green', mask, status, regions, running, starts);
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

// Coloured regions (owner, 2026-10-16: as the persona chat's): the index goes
// up like a mask, the message names it with each colour's words, and the
// regions ride beside it as fields for the harness to draw.
{
  const regions = [
    { colour: 'magenta', rgb: [255, 0, 255], words: 'a red scarf' },
    { colour: 'cyan', rgb: [0, 255, 255], words: 'snow on the roof' },
  ];
  const r = await page({ mask: 'INDEX-BLOB', regions });
  is(r.uploads.length, 1, 'painted regions upload their index once');
  is(r.sent.length, 1, 'then send once');
  is(
    r.sent[0].opts,
    {
      text: 'Edit images/pic.png in regions inbox/mask-pic-7.png: magenta: a red scarf; cyan: snow on the roof',
      regions: {
        picture: 'images/pic.png',
        mask: 'inbox/mask-pic-7.png',
        regions: [
          { colour: 'magenta', words: 'a red scarf' },
          { colour: 'cyan', words: 'snow on the roof' },
        ],
      },
    },
    'the message names the index and each colour, and the regions go as fields, colour and words only',
  );
  is([r.editing, r.draft], [null, ''], 'and once the turn starts, the modal closes and the draft is spent');
  // A run that began after the check: the server refuses, and the paint stays.
  const raced = await page({ mask: 'INDEX-BLOB', regions, starts: false });
  is(raced.editing?.busy, false, 'a refused send leaves the modal open');
  is((raced.editing?.error ?? '').includes('still here'), true, 'saying the regions are kept');
  is(raced.draft, 'a red scarf', 'and the draft untouched');
  const busy = await page({ mask: 'INDEX-BLOB', regions, running: true });
  is([busy.uploads.length, busy.sent.length], [0, 0], 'while a reply runs, nothing goes up or out');
  is((busy.editing?.error ?? '').includes('reply is running'), true, 'and the modal keeps the work and says why');
  const plain = await page({ mask: null });
  is(plain.sent[0].opts, {}, 'nothing painted sends no regions');
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
