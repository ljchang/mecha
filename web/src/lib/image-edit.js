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
  const safe = stem.replace(/[^A-Za-z0-9_-]+/g, '-').slice(0, 60) || 'picture';
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
