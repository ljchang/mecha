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

