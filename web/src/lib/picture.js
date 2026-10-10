// The picture under a tool row, in either chat — the assistant's
// (`Chat.svelte`) and a persona's (`Personas.svelte`). One matcher for both:
// the match is what turns a tool's text into a URL the page fetches, and two
// copies of a strict pattern drift into one loose one.
//
// Two other places match a picture path, for a different job — a reply line
// that only points at a picture, hidden from display and speech:
// `speech.js` `PICTURE_REF` and `mecha-cli/src/voice/speech.rs`
// `is_picture_ref`, held to each other by `test/speakable-cases.json`. A
// change to what a picture path is belongs in all three.
//
// The picture is the file `image_generate` saved, or the one
// `image_view` put in front of the model — which the owner must be able to
// see too, since the model's answer is about it. Read off the first line of
// the tool's own result and matched strictly, so no other text in a preview
// is ever taken for a path to fetch: `image_generate` only ever writes
// `images/<name>.png`; `image_view` reports a workspace-relative path of
// plain segments, none starting with a dot. A refusal or failure has no
// picture.

import { saveBlob } from './reply-export.js';
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

// Every picture the tool rows drew, in order, each once.
export function picturesIn(entries) {
  const out = [];
  for (const entry of entries) {
    const picture = entry.kind === 'tool' ? pictureOf(entry) : null;
    if (picture && !out.includes(picture)) out.push(picture);
  }
  return out;
}

// The pictures drawn since a call began, oldest first: those not among
// `before`, the set taken when the call was placed. Keyed on the picture,
// never an entry index — a finished turn reloads the transcript from the
// session file, which rebuilds the entries and moves every index.
export function picturesSince(entries, before) {
  return picturesIn(entries).filter((picture) => !before.has(picture));
}

// Where a turn asked for pictures and got none: the index of the turn's last
// entry, for a plain line under it. Read off the tool rows' own results, so
// a reply that says "here you go" over nothing cannot stand as the last word
// (2026-09-30: seven refused calls, then "Here you go"). A turn runs from one
// message the owner sent to the next; a steer folded into the run is not a
// new turn — `queued` while it is live, `steered` as the transcript reads it
// back, where it comes after the turn's tool rows (review of #444). A turn still running, or with a call still out, is not
// judged yet.
// A picture still being drawn past the turn that asked for it
// (`docs/BACKGROUND-JOBS-DESIGN.md` §2.1): its result came back at once as
// `being made: <path>`, `is_error: false`, and the finished one replaces it
// when the job ends. Neither drawn nor failed — still out.
export function stillOut(entry) {
  return (
    entry?.kind === 'tool' &&
    entry.name === 'image_generate' &&
    entry.is_error === false &&
    String(entry.preview ?? '').startsWith('being made: ')
  );
}

// The pictures still out that wait their turn, as indices into `entries`.
// The server's line (`queue`, `jobs::QueueItem`) names the one drawing, and
// is the truth whenever the page has one: a reorder permutes the line while
// the transcript keeps the order the pictures were asked for, so "the oldest
// still out is drawing" stops holding the moment the owner moves one (review
// of #607). With no line yet, the queue's own order is the guess: first in,
// first out, so every one after the oldest waits.
export function waitingPictures(entries, queue = []) {
  const out = new Set();
  const drawing = (queue ?? []).find((q) => q.running)?.call_id;
  if (queue?.length) {
    (entries ?? []).forEach((e, i) => {
      if (stillOut(e) && e.id !== drawing) out.add(i);
    });
    return out;
  }
  let first = true;
  (entries ?? []).forEach((e, i) => {
    if (!stillOut(e)) return;
    if (first) first = false;
    else out.add(i);
  });
  return out;
}

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
    // A picture being made is still out, never drawn: the counter's own
    // check, since `pictureOf`'s pattern already refuses its first line.
    if (stillOut(entry)) open += 1;
    else if (entry.is_error === false) drawn += 1;
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

// The queue panel's reorder (`PictureQueue.svelte`), pure so node tests it.
// `ids` is the waiting line; the moved job is named by its id, never an
// index, which a `queue` event mid-drag would shift (review of #607).

// `id` to position `to` in the new line, or null when either is gone.
export function moveTo(ids, id, to) {
  const out = [...ids];
  const at = out.indexOf(id);
  if (at < 0 || to < 0 || to >= out.length) return null;
  out.splice(at, 1);
  out.splice(to, 0, id);
  return out;
}

// `id` dropped at `slot`, the gap above row `slot` as the line stood when the
// drag began (`ids.length` is below the last row) — where the marker is drawn.
// Moving down, taking the row out shifts the gaps below it up by one, so the
// landing is one less: otherwise a row dropped above `c` lands below it
// (review of #607). Null when nothing would move.
export function dropAt(ids, id, slot) {
  const at = ids.indexOf(id);
  if (at < 0 || slot < 0 || slot > ids.length) return null;
  const to = slot > at ? slot - 1 : slot;
  if (to === at) return null;
  return moveTo(ids, id, to);
}

// Regenerate's versions (IMAGE-DESIGN.md §5.4). A card whose `version_of`
// names a picture drawn higher up is that picture drawn again: it shows on
// the first card of the chain, ‹ k/n ›, not as a card of its own. A
// Regenerate of a version names the version, so the chain is followed back
// to its first picture. A `version_of` naming a picture this transcript
// does not show (compacted away) leaves the card standing alone, since a
// version must never vanish into a card nobody can see.
//
// `groups`: each first picture → its versions in order, itself first.
// `folded`: the entry indices whose picture shows on an earlier card, each
// → `{ root, k }`, `k` counting from 1 as the card does.
export function pictureVersions(entries) {
  const rootOf = new Map();
  const groups = new Map();
  const folded = new Map();
  entries.forEach((entry, i) => {
    const picture = entry.kind === 'tool' ? pictureOf(entry) : null;
    if (!picture || rootOf.has(picture)) return;
    const root = entry.version_of ? rootOf.get(entry.version_of) : undefined;
    if (root === undefined) {
      rootOf.set(picture, picture);
      groups.set(picture, [picture]);
      return;
    }
    rootOf.set(picture, root);
    const list = groups.get(root);
    list.push(picture);
    folded.set(i, { root, k: list.length });
  });
  return { groups, folded };
}

// The version a card shows: the one the owner stepped to, while it is still
// among them, else the newest, so a Regenerate's result is what lands.
export function shownVersion(list, chosen) {
  return chosen && list.includes(chosen) ? chosen : list[list.length - 1];
}
