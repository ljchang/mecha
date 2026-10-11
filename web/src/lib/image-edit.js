// The edit modal's logic, apart from the canvas so it can be tested in node.
//
// The owner paints (or boxes) the part of a picture to change; the page turns
// that into a black-and-white mask at the picture's own size and uploads it
// into the chat's jail. The mask is named in the message and never attached:
// attachments ride on the turn as pixels (REMOTE-SURFACE-DESIGN D6), and a
// mask is for `image_generate`, not for the model to look at — attaching one
// would spend context and mark the chat as holding a picture for nothing
// (IMAGE-REGION-EDIT-RESEARCH.md §5).

/** A point on the displayed picture, in the picture's own pixels, clamped. */
export function toNatural(clientX, clientY, rect, natural) {
  const sx = natural.width / rect.width;
  const sy = natural.height / rect.height;
  const x = Math.min(Math.max((clientX - rect.left) * sx, 0), natural.width);
  const y = Math.min(Math.max((clientY - rect.top) * sy, 0), natural.height);
  return [x, y];
}

/** The box two corners span, whichever way it was dragged. */
export function boxFrom([ax, ay], [bx, by]) {
  return { x: Math.min(ax, bx), y: Math.min(ay, by), w: Math.abs(bx - ax), h: Math.abs(by - ay) };
}

/** A brush a few percent of the picture wide: big enough to paint a person
 *  in a few strokes on a phone, small enough to leave the rest alone. */
export function defaultBrush(naturalWidth) {
  return Math.min(Math.max(Math.round(naturalWidth * 0.04), 8), 160);
}

/** Whether anything is painted, from the operations drawn so far. An erase
 *  alone paints nothing; a box too small to see is not a region. The server
 *  refuses an empty mask either way — this only decides the button. */
export function hasPaint(ops) {
  return ops.some((op) =>
    op.kind === 'box' ? op.w >= 2 && op.h >= 2 : op.kind === 'stroke' && !op.erase,
  );
}

/** A mask's upload name, tied to the picture it was painted over. The server
 *  tames it again (`files::tame_filename`); this keeps it readable. */
export function maskName(picturePath, now = Date.now()) {
  const stem = (picturePath.split('/').pop() ?? 'picture').replace(/\.[^.]*$/, '');
  const safe =
    stem
      .replace(/[^A-Za-z0-9_-]+/g, '-')
      .replace(/-+/g, '-')
      .replace(/^-|-$/g, '')
      .slice(0, 60) || 'picture';
  return `mask-${safe}-${now}.png`;
}

/** The message a modal send puts in the chat: the picture, the mask when
 *  there is one, and the owner's words. `null` when there are no words —
 *  a region with nothing to do in it is not a request. */
export function composeEditMessage(picturePath, maskPath, instruction) {
  const words = (instruction ?? '').trim();
  if (!words) return null;
  return maskPath
    ? `Edit ${picturePath} with mask ${maskPath}: ${words}`
    : `Edit ${picturePath}: ${words}`;
}

// Regenerate (IMAGE-DESIGN.md §5.4): the message the owner sees for the
// picture drawn again as it is. The turn's `edit` carries `redraw` and the
// picture alone, never a mask or regions, which the server refuses beside it.
export function composeRegenerateMessage(picturePath) {
  return `Regenerate ${picturePath}`;
}

// Regions (IMAGE-REGION-EDIT-RESEARCH.md §7, M2): the colours a region may
// be painted in, by name and exact value, mirroring `picture::REGION_COLOURS`
// on the server. The page picks four, those least present in the picture,
// since Qwen asks for a colour "that appears nowhere else in the frame".
export const REGION_COLOURS = [
  ['magenta', [255, 0, 255]],
  ['cyan', [0, 255, 255]],
  ['blue', [0, 64, 255]],
  ['green', [0, 200, 0]],
  ['orange', [255, 128, 0]],
  ['yellow', [255, 230, 0]],
  ['red', [230, 0, 0]],
  ['purple', [128, 0, 255]],
];

/** The most regions one edit carries (`picture::MAX_REGIONS`). */
export const MAX_REGIONS = 4;

/** The four region colours least present in a picture's pixels (RGBA, as
 *  `getImageData` gives them), in palette order among equals. */
export function pickColours(data) {
  const near = REGION_COLOURS.map(() => 0);
  for (let i = 0; i < data.length; i += 4) {
    REGION_COLOURS.forEach(([, [r, g, b]], k) => {
      const d = Math.abs(data[i] - r) + Math.abs(data[i + 1] - g) + Math.abs(data[i + 2] - b);
      if (d < 120) near[k] += 1;
    });
  }
  return REGION_COLOURS.map((c, k) => [c, near[k], k])
    .sort((a, b) => a[1] - b[1] || a[2] - b[2])
    .slice(0, MAX_REGIONS)
    .map(([c]) => ({ colour: c[0], rgb: c[1] }));
}

/** A region index's pixels (RGBA): black everywhere, and each region's own
 *  colour exactly where its layer's alpha is over half. No smoothing, so the
 *  server can read every pixel as one colour or none. `layers` are each
 *  region's RGBA data at the picture's size, in region order. The modal
 *  keeps regions apart (paint takes a pixel from the others), so a later
 *  region winning is only a tie-break on anti-aliased rims. */
export function indexPixels(width, height, layers, rgbs) {
  const out = new Uint8ClampedArray(width * height * 4);
  for (let i = 3; i < out.length; i += 4) out[i] = 255;
  layers.forEach((layer, k) => {
    const [r, g, b] = rgbs[k];
    for (let i = 0; i < out.length; i += 4) {
      if (layer[i + 3] > 127) {
        out[i] = r;
        out[i + 1] = g;
        out[i + 2] = b;
      }
    }
  });
  return out;
}

/** The message a regions edit puts in the chat: the picture, the index, and
 *  each region's colour and words. `null` when a painted region has none. */
export function composeRegionsMessage(picturePath, indexPath, regions) {
  if (!regions.length || regions.some((r) => !(r.words ?? '').trim())) return null;
  const each = regions.map((r) => `${r.colour}: ${r.words.trim()}`).join('; ');
  return `Edit ${picturePath} in regions ${indexPath}: ${each}`;
}

// Whether the modal holds work: paint, or words changed from the chat's draft.
// In regions mode the first region opens holding that draft, which is not
// work; single mode has no region words to read (review of #623).
export function editDirty({ multi, painted, words, initial, regionWords }) {
  if (painted || words.trim() !== initial.trim()) return true;
  return multi && regionWords.some((w, k) => (w ?? '').trim() !== (k === 0 ? initial.trim() : ''));
}

// The words of the first region to hold paint, at the moment it does: what
// the owner typed in the whole-picture box, unless the region's own words
// were already changed from what it opened with (the chat's draft for region
// one, nothing for an added one). Without this the typed sentence vanished
// with no sign (review of #623).
export function firstRegionWords(regionWords, typed, opened) {
  return (regionWords ?? '').trim() === opened.trim() ? typed : regionWords;
}

// The operations left when region `k` is removed: its paint goes, but every
// erase stays, since the eraser clears all regions whichever was selected.
// Dropping an erase made with `k` selected brought back paint it had rubbed
// out of another region (review of #623). Later regions shift down one.
export function opsWithout(ops, k) {
  return ops
    .filter((op) => op.erase || op.region !== k)
    .map((op) => {
      if (op.region === k) return { ...op, region: Math.max(0, k - 1) };
      return op.region > k ? { ...op, region: op.region - 1 } : op;
    });
}

/** The longest side the edit modal's canvases work at. */
export const WORK_EDGE = 2048;

/** The size the modal paints at for a picture of `width`×`height`: its own
 *  size up to `WORK_EDGE` on the long side, else scaled down to it, keeping
 *  the shape the server checks a mask against (review of #623). */
export function workSize(width, height) {
  const scale = Math.min(1, WORK_EDGE / Math.max(width, height));
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
  };
}
