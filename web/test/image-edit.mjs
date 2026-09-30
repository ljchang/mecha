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

console.log('image-edit ok');
