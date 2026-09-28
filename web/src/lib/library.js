// The image library page's pure logic, kept out of the component so the
// tests exercise the shipped code (web/test/library.mjs).

export const PANES = ['characters', 'styles', 'candidates'];

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
