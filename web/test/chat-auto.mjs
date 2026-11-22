// The chat header's auto chip (F7): what it says, when it is offered, and
// that only a server `true` reads as on.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { autoChip, autoOffered, autoFrom } from '../src/lib/chat-auto.js';

const off = autoChip(false);
assert.equal(off.label, 'auto off');
assert.equal(off.pressed, 'false');
const on = autoChip(true);
assert.equal(on.label, 'auto on');
assert.equal(on.pressed, 'true');
for (const c of [on, off]) {
  assert.match(c.title, /^Follow-ups only/, 'the chip says what it covers');
  assert.match(c.title, /drafts?/i, 'and what it does not');
}

const rows = (shown) => new Map([['followups', { id: 'followups', shown }]]);
assert.equal(autoOffered({ memoryIntent: false, incognito: false, rows: rows(true) }), true);
assert.equal(autoOffered({ memoryIntent: false, incognito: false, rows: rows(false) }), false, 'feature off hides it');
assert.equal(autoOffered({ memoryIntent: false, incognito: true, rows: rows(true) }), false, 'incognito proposes nothing');
assert.equal(autoOffered({ memoryIntent: true, incognito: false, rows: rows(true) }), false, 'a memory chat has no chip');
// Not answered is shown, as every feature-gated control is.
assert.equal(autoOffered({ memoryIntent: false, incognito: false, rows: null }), true);

// Off unless the server said true: an absent field (an older serve), a
// string, anything else is off — the chip never claims a standing
// acceptance the server did not report.
assert.equal(autoFrom(true), true);
for (const v of [false, undefined, null, 'true', 1]) assert.equal(autoFrom(v), false, String(v));

// The component reads the server's word on both paths, and posts to the
// owner's endpoint — the one writer of the flag.
const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Chat.svelte'), 'utf8');
assert.match(src, /auto = autoFrom\(data\.auto\)/, 'the transcript settles it');
assert.match(src, /case 'auto':\s*\n[\s\S]*?auto = autoFrom\(ev\.on\)/, 'the event settles it');
assert.match(src, /\/api\/chat\/\$\{key\}\/auto/, 'the chip posts to the owner endpoint');
console.log('chat auto: ok');
