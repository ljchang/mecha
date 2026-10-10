// The edit modal's logic: coordinates, boxes, what counts as painted, and the
// message it sends — the mask named, never attached.
import assert from 'node:assert/strict';
import {
  toNatural,
  boxFrom,
  defaultBrush,
  hasPaint,
  maskName,
  composeEditMessage,
} from '../src/lib/image-edit.js';

// A picture shown at half size: a click maps to its own pixels, clamped.
const rect = { left: 10, top: 20, width: 672, height: 384 };
const natural = { width: 1344, height: 768 };
assert.deepEqual(toNatural(10, 20, rect, natural), [0, 0]);
assert.deepEqual(toNatural(682, 404, rect, natural), [1344, 768]);
assert.deepEqual(toNatural(346, 212, rect, natural), [672, 384]);
assert.deepEqual(toNatural(-50, 999, rect, natural), [0, 768], 'clamped to the picture');

// A box dragged up and to the left is the same box.
assert.deepEqual(boxFrom([300, 200], [100, 50]), { x: 100, y: 50, w: 200, h: 150 });

assert.equal(defaultBrush(1344), 54);
assert.equal(defaultBrush(100), 8);
assert.equal(defaultBrush(8000), 160);

assert.equal(hasPaint([]), false);
assert.equal(hasPaint([{ kind: 'stroke', erase: true, points: [[1, 1]] }]), false, 'erasing alone');
assert.equal(hasPaint([{ kind: 'box', x: 0, y: 0, w: 1, h: 40 }]), false, 'a sliver is no region');
assert.equal(hasPaint([{ kind: 'box', x: 0, y: 0, w: 40, h: 40 }]), true);
assert.equal(hasPaint([{ kind: 'stroke', erase: false, points: [[1, 1]] }]), true);

assert.match(maskName('images/20260929-163113-4012932085.png', 7), /^mask-20260929-163113-4012932085-7\.png$/);
assert.equal(maskName('inbox/My photo (1).JPG', 7), 'mask-My-photo-1-7.png');
assert.equal(maskName('inbox/((( ))).png', 7), 'mask-picture-7.png', 'nothing usable in the name');

assert.equal(
  composeEditMessage('images/a.png', 'inbox/mask-a-7.png', '  make the dress green  '),
  'Edit images/a.png with mask inbox/mask-a-7.png: make the dress green',
);
assert.equal(composeEditMessage('images/a.png', null, 'make it dusk'), 'Edit images/a.png: make it dusk');
assert.equal(composeEditMessage('images/a.png', 'inbox/m.png', '   '), null, 'no words, no request');

// Regions: the least-present colours are picked, the index is exact, and a
// region with no words makes no message.
{
  const { pickColours, indexPixels, composeRegionsMessage, REGION_COLOURS } = await import(
    '../src/lib/image-edit.js'
  );
  // A picture that is all magenta: magenta is never offered.
  const px = new Uint8ClampedArray(16 * 4);
  for (let i = 0; i < px.length; i += 4) px.set([255, 0, 255, 255], i);
  const picked = pickColours(px);
  assert.equal(picked.length, 4);
  assert.ok(!picked.some((c) => c.colour === 'magenta'));
  assert.ok(picked.every((c) => REGION_COLOURS.some(([n]) => n === c.colour)));
  // Two 2x1 layers: the first paints pixel 0, the second pixel 1; an
  // anti-aliased rim at alpha 100 stays black.
  const a = new Uint8ClampedArray([0, 0, 0, 255, 0, 0, 0, 0]);
  const b = new Uint8ClampedArray([0, 0, 0, 100, 0, 0, 0, 255]);
  const out = indexPixels(2, 1, [a, b], [[255, 0, 255], [0, 255, 255]]);
  assert.deepEqual([...out], [255, 0, 255, 255, 0, 255, 255, 255]);
  assert.equal(
    composeRegionsMessage('images/a.png', 'inbox/r.png', [
      { colour: 'magenta', words: 'make it red' },
      { colour: 'cyan', words: ' remove the cup ' },
    ]),
    'Edit images/a.png in regions inbox/r.png: magenta: make it red; cyan: remove the cup',
  );
  assert.equal(composeRegionsMessage('images/a.png', 'inbox/r.png', [{ colour: 'cyan', words: ' ' }]), null);
}

console.log('image-edit ok');

// An untouched modal closes, in either mode, whatever draft the chat held:
// the first region opening with the draft is not work (review of #623).
import { editDirty, firstRegionWords } from '../src/lib/image-edit.js';
const draft = 'make it dusk';
assert.equal(editDirty({ multi: false, painted: false, words: draft, initial: draft, regionWords: [''] }), false);
assert.equal(editDirty({ multi: true, painted: false, words: draft, initial: draft, regionWords: [draft] }), false);
assert.equal(editDirty({ multi: true, painted: false, words: draft, initial: draft, regionWords: [draft, 'a hat'] }), true);
assert.equal(editDirty({ multi: true, painted: false, words: draft, initial: draft, regionWords: ['a hat'] }), true);
assert.equal(editDirty({ multi: false, painted: false, words: 'other', initial: draft, regionWords: [''] }), true);
assert.equal(editDirty({ multi: false, painted: true, words: draft, initial: draft, regionWords: [''] }), true);

// What was typed before the first stroke carries into the first region,
// unless that region's words were already changed.
assert.equal(firstRegionWords(draft, 'make the scarf yellow', draft), 'make the scarf yellow');
assert.equal(firstRegionWords('', 'make the scarf yellow', ''), 'make the scarf yellow');
assert.equal(firstRegionWords('a red hat', 'make the scarf yellow', draft), 'a red hat');
// A region added before the first stroke opens empty, so it takes the typed
// words even with a draft in the chat.
assert.equal(firstRegionWords('', 'make the scarf yellow', ''), 'make the scarf yellow');
assert.equal(firstRegionWords('a red hat', 'make the scarf yellow', ''), 'a red hat');

// The page's colour table is the server's, value for value: the server
// matches the index by exact pixel, so a one-step drift would refuse every
// edit in that colour as an empty region (review of #623). Read from the
// Rust source, as call-edit.mjs reads Personas.svelte.
import fs from 'node:fs';
import { REGION_COLOURS, MAX_REGIONS } from '../src/lib/image-edit.js';
const rust = fs.readFileSync(new URL('../../mecha-core/src/picture.rs', import.meta.url), 'utf8');
const table = rust.match(/pub const REGION_COLOURS[^=]*=\s*\[([\s\S]*?)\];/)[1];
const server = [...table.matchAll(/\("(\w+)",\s*\[(\d+),\s*(\d+),\s*(\d+)\]\)/g)].map((m) => [
  m[1],
  [Number(m[2]), Number(m[3]), Number(m[4])],
]);
assert.equal(server.length, 8, 'the server table was read');
assert.deepEqual(REGION_COLOURS, server);
assert.equal(MAX_REGIONS, Number(rust.match(/pub const MAX_REGIONS: usize = (\d+);/)[1]));

// Removing a region keeps every erase, whichever region was selected when
// it was made, and shifts later regions down (review of #623).
import { opsWithout } from '../src/lib/image-edit.js';
const paint1 = { kind: 'stroke', region: 0, erase: false };
const paint2 = { kind: 'stroke', region: 1, erase: false };
const eraseWith2 = { kind: 'stroke', region: 1, erase: true };
const paint3 = { kind: 'stroke', region: 2, erase: false };
assert.deepEqual(opsWithout([paint1, paint2, eraseWith2, paint3], 1), [
  paint1,
  { ...eraseWith2, region: 0 },
  { ...paint3, region: 1 },
]);
assert.deepEqual(opsWithout([paint1, eraseWith2], 0), [{ ...eraseWith2, region: 0 }]);

