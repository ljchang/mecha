// The image library page's pure logic, kept out of the component so the
// tests exercise the shipped code (web/test/library.mjs).

export const PANES = ['characters', 'styles', 'voices', 'candidates'];

// Which pane a hash sub names; anything else is the characters pane.
export function paneOf(sub) {
  return PANES.includes(sub) ? sub : 'characters';
}

// The entries a pane shows. Candidates are their own pane whatever their
// kind: they wait on a decision, and a card among the approved would read as
// already usable.
export function entriesFor(pane, entries) {
  const list = entries ?? [];
  if (pane === 'candidates') return list.filter((e) => e.status === 'candidate');
  // Voices are not library entries: their pane reads its own list.
  if (pane === 'voices') return [];
  const kind = pane === 'styles' ? 'style' : 'character';
  return list.filter((e) => e.status === 'approved' && e.kind === kind);
}

export function counts(entries) {
  const out = { characters: 0, styles: 0, candidates: 0 };
  for (const e of entries ?? []) {
    if (e.status === 'candidate') out.candidates += 1;
    else if (e.kind === 'style') out.styles += 1;
    else out.characters += 1;
  }
  return out;
}

// Who wrote an entry's text, in the owner's words. The untrusted case is the
// one that matters: approving it makes a model's text ride into every prompt
// that names the character.
export const ORIGIN = {
  owner: 'yours',
  model_clean: 'proposed by the model',
  model_untrusted: 'proposed by the model while reading outside content — read it before approving',
};

export function originLabel(origin) {
  return ORIGIN[origin] ?? ORIGIN.model_untrusted;
}

// The list's URL: the unlock token rides along only while there is one.
// A token lives in the page's memory and nowhere else (no-storage.mjs).
export function listUrl(token) {
  return token ? `/api/library?unlock=${encodeURIComponent(token)}` : '/api/library';
}

// The voice library (Library → Voices), with the unlock when there is one:
// "used by" names a locked persona only for an unlock.
export function voicesUrl(token) {
  return token ? `/api/library/voices?unlock=${encodeURIComponent(token)}` : '/api/library/voices';
}

// A voice's line under its name: a clone or the server's own, how long its
// reference is, who speaks in it, and whether the server can speak it yet.
// `listed` is null when the server's list could not be read — unknown, not
// "no", so it says nothing then.
export function voiceLine(v, partial = null, clonesUnread = false) {
  const parts = [];
  if (v.cloned) {
    const secs = v.cloned.seconds ? ` · ${Math.round(v.cloned.seconds)}s reference` : '';
    parts.push(`cloned here${secs}`);
    if (v.listed === false) parts.push('not on the voice server yet');
  } else if (v.listed === false && !clonesUnread) {
    // Named by a persona, and neither the server's nor a clone here: a typo,
    // or a voice removed — a call to it is refused.
    parts.push('on neither the voice server nor this box — a call in it is refused');
  } else if (v.listed === true && !clonesUnread) {
    parts.push("the voice server's own");
  }
  // The clone folder could not be read: no row can say it is not a clone,
  // as an unasked server says nothing about being listed (review of #490).
  // listed null and no clone: the server could not be asked, so this says
  // nothing about where the voice comes from (review of #490).
  if (v.used_by?.length) parts.push(`${v.used_by.join(', ')} speak${v.used_by.length === 1 ? 's' : ''} in it`);
  // A clone can be deleted, so on its row "nobody listed" must not read as
  // "nobody": say why the list may be short (review of #490). Generic on
  // purpose — never whether a locked persona exists.
  if (v.cloned && partial === 'locked') parts.push('unlock to see every persona that speaks in it');
  if (v.cloned && partial === 'unreadable') parts.push('who speaks in it could not be fully read');
  return parts.join(' · ');
}

// What the save dialog calls a picture's library name: its file stem is no
// use, so it starts empty unless the owner types one. Names are what the
// store accepts: lowercase letters, digits and hyphens.
export function validName(name) {
  return /^[a-z0-9][a-z0-9-]{0,63}$/.test(name ?? '');
}

// A typed name, tamed toward what the store accepts — lowercased, spaces to
// hyphens — without inventing one.
export function tameName(typed) {
  return (typed ?? '')
    .toLowerCase()
    .trim()
    .replace(/\s+/g, '-')
    .replace(/[^a-z0-9-]/g, '');
}

// The store's caps on an entry's text (imagelib.rs MAX_DESCRIPTION,
// MAX_STYLE_TEXT): the form says so before the server has to.
export const TEXT_MAX = { character: 400, style: 1000 };

// A portrait's longest edge as the page sends it. Generation sends every
// portrait at 512² (research E10), so a phone's 4000-pixel photo buys
// nothing but upload time; re-encoding also drops the photo's metadata,
// location included, before it leaves the device.
export const PORTRAIT_EDGE = 1536;

// Scale (w, h) down to fit `max` on its longest edge; never up.
export function fitWithin(w, h, max) {
  const scale = Math.min(1, max / Math.max(w, h));
  return { w: Math.max(1, Math.round(w * scale)), h: Math.max(1, Math.round(h * scale)) };
}

// Why the add/edit form cannot be saved yet, or null when it can. `form` is
// { mode: 'add' | 'edit', kind, name, text, portrait } with `portrait` the
// prepared base64 (or null), and `entry` the entry being edited.
export function formProblem(form, entry = null) {
  const text = (form.text ?? '').trim();
  if (form.mode === 'add' && !validName(form.name)) return 'a name: lowercase letters, digits and hyphens';
  if (!text) return form.kind === 'style' ? 'the style’s text' : 'a description';
  if (text.length > TEXT_MAX[form.kind]) return `at most ${TEXT_MAX[form.kind]} characters`;
  if (form.mode === 'add' && form.kind === 'character' && !form.portrait) return 'a portrait';
  if (form.mode === 'edit' && entry && text === entry.text.trim() && !form.portrait) return 'nothing changed yet';
  return null;
}

// The JSON body the form posts: to /api/library/add, everything; to
// /api/library/edit, only what changed — an unchanged description is not
// sent, so replacing a portrait never rewrites the text's version history.
export function formBody(form, entry = null, token = null) {
  if (form.mode === 'add') {
    return {
      kind: form.kind,
      name: form.name,
      text: form.text.trim(),
      locked: !!form.locked,
      ...(form.kind === 'character' ? { portrait: form.portrait } : {}),
    };
  }
  const text = form.text.trim();
  return {
    kind: entry.kind,
    name: entry.name,
    ...(text !== entry.text.trim() ? { text } : {}),
    ...(form.portrait ? { portrait: form.portrait } : {}),
    ...(token ? { unlock: token } : {}),
  };
}
