import { saveBlob } from './reply-export.js';
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

// Where a turn asked for pictures and got none: the index of the turn's last
// entry, for a plain line under it. Read off the tool rows' own results, so
// a reply that says "here you go" over nothing cannot stand as the last word
// (2026-09-30: seven refused calls, then "Here you go"). A turn runs from one
// message the owner sent to the next; a steer folded into the run is not a
// new turn — `queued` while it is live, `steered` as the transcript reads it
// back, where it comes after the turn's tool rows (review of #444). A turn still running, or with a call still out, is not
// judged yet.
export function turnsWithoutPicture(entries, running = false) {
  const out = new Set();
  let asked = 0;
  let drawn = 0;
  let open = 0;
  // Under the turn's last reply or tool row: a page-only notice or crisis
  // card that `settle` carries after the server's entries is not the reply
  // (review of #444).
  let last = -1;
  const close = () => {
    if (last >= 0 && asked > 0 && drawn === 0 && open === 0) out.add(last);
    asked = drawn = open = 0;
    last = -1;
  };
  entries.forEach((entry, i) => {
    if (entry.kind === 'user' && !entry.queued && !entry.steered) close();
    if (entry.kind === 'assistant' || entry.kind === 'tool') last = i;
    if (entry.kind !== 'tool' || entry.name !== 'image_generate') return;
    asked += 1;
    if (entry.is_error === false) drawn += 1;
    else if (entry.is_error !== true) open += 1;
  });
  if (!running) close();
  return out;
}

// A generated picture's file name: the last part of its workspace path.
export function pictureName(path) {
  const base = String(path ?? '').split('/').filter(Boolean).pop() ?? '';
  return base || 'picture.png';
}

// Save a generated picture (owner request, 2026-10-01): read through the
// chat's own file route — `get`, the page's same-origin fetch — and saved
// from a blob, never by pointing the browser at the URL, so no picture
// address lands in the history a locked or incognito chat keeps out of it.
// Returns '' when saved, or why not.
export async function downloadPicture(get, url, path, doc = globalThis.document, urls = globalThis.URL) {
  try {
    const res = await get(url);
    if (!res.ok) return (await res.text()).trim() || `HTTP ${res.status}`;
    saveBlob(pictureName(path), await res.blob(), doc, urls);
    return '';
  } catch (e) {
    return String(e?.message ?? e);
  }
}
