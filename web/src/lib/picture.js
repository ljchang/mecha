// The picture under a tool row, in either chat — the assistant's
// (`Chat.svelte`) and a persona's (`Personas.svelte`). One matcher for both:
// the match is what turns a tool's text into a URL the page fetches, and two
// copies of a strict pattern drift into one loose one.
//
// The picture is the file `image_generate` saved, or the one
// `image_view` put in front of the model — which the owner must be able to
// see too, since the model's answer is about it. Read off the first line of
// the tool's own result and matched strictly, so no other text in a preview
// is ever taken for a path to fetch: `image_generate` only ever writes
// `images/<name>.png`; `image_view` reports a workspace-relative path of
// plain segments, none starting with a dot. A refusal or failure has no
// picture.
const PICTURE = {
  image_generate: /^image: (images\/[A-Za-z0-9._-]+\.png)$/,
  image_view: /^image: ((?:[A-Za-z0-9_-][A-Za-z0-9._ -]*\/)*[A-Za-z0-9_-][A-Za-z0-9._ -]*\.(?:png|jpe?g|gif|webp))$/i,
};
export function pictureOf(entry) {
  const pattern = Object.hasOwn(PICTURE, entry.name) ? PICTURE[entry.name] : null;
  if (!pattern || entry.pending || entry.is_error || entry.blocked) {
    return null;
  }
  const first = (entry.preview ?? '').split('\n', 1)[0];
  const m = pattern.exec(first);
  return m ? m[1] : null;
}

// The rows whose picture is already drawn higher up — an `image_view` of the
// file `image_generate` just saved, most often. A repeat keeps its row and
// shows the picture inside the disclosure instead of drawing it inline a
// second time (a persona chat, which has no disclosure, simply draws it
// once). Indices into `entries`, first occurrence wins.
export function repeatedPictures(entries) {
  const seen = new Set();
  const again = new Set();
  entries.forEach((entry, i) => {
    const picture = entry.kind === 'tool' ? pictureOf(entry) : null;
    if (!picture) return;
    if (seen.has(picture)) again.add(i);
    else seen.add(picture);
  });
  return again;
}
