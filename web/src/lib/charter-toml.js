// The charter document, as bytes.
//
// Extracted from `SettingsCharter.svelte` so it can be tested: these are the
// functions that decide what reaches `~/.mecha/charter.toml`, and inside a
// component the only way to exercise them was to drive a browser. Two
// languages describe this file — these functions write it, `Charter::parse`
// decides whether it loads — so `website/scripts/check-charter-toml.mjs`
// pins the agreement against the sample `charter.rs` reads back.

/// Always a single-line basic string: unambiguous, and it matches how the
/// file is already written.
///
/// Escapes backslash, quote, newline, carriage return and tab — not the other
/// control characters TOML forbids in a basic string (U+0000-U+0008,
/// U+000B-U+000C, U+000E-U+001F, U+007F). That is deliberate rather than
/// missed: one of those reaching here would be refused by the server's
/// `Charter::parse` with a 422 that keeps the draft open, so the failure is
/// closed and nothing typed is lost. Escaping them here would let a
/// character no owner can type into the file instead.
export const esc = (s) =>
  '"' +
  String(s)
    .replace(/\\/g, '\\\\')
    .replace(/"/g, '\\"')
    .replace(/\n/g, '\\n')
    .replace(/\r/g, '\\r')
    .replace(/\t/g, '\\t') +
  '"';

/// `header` is kept byte-for-byte; only the `[[line]]` tables are generated.
export function serialize(header, lines) {
  const out = [];
  if (header.trim()) out.push(header, '');
  for (const l of lines) {
    out.push('[[line]]', `id = ${esc(l.id.trim())}`, `text = ${esc(l.text.trim())}`, '');
  }
  return out.join('\n').replace(/\n+$/, '\n');
}

/// Does a comment open anywhere in this text?
///
/// Takes a whole text rather than a row, because a multi-line string spans
/// rows and a row read in isolation cannot tell an opening delimiter from a
/// closing one: `""" # note` looks like a `#` *inside* a string when it is
/// really the comment after that string ends. `splitHeader` therefore hands
/// the entire tail below the first `[[line]]` over in one go — applying this
/// per row was the bug.
///
/// A comment below that point cannot survive a save, since the tables are
/// regenerated, so finding one is what makes the list editor stand down.
export function hasComment(text) {
  const s = String(text);
  let i = 0;
  const skipTo = (close, escapes) => {
    while (i < s.length && !s.startsWith(close, i)) {
      if (escapes && s[i] === '\\') i++;
      i++;
    }
    i += close.length;
  };
  while (i < s.length) {
    if (s[i] === '#') return true;
    if (s.startsWith('"""', i)) {
      i += 3;
      skipTo('"""', true);
    } else if (s.startsWith("'''", i)) {
      i += 3;
      skipTo("'''", false);
    } else if (s[i] === '"' || s[i] === "'") {
      // A single-line string ends at a newline as well as at its own quote:
      // an unterminated one must not swallow the rest of the document and
      // hide every comment below it.
      const q = s[i];
      i++;
      while (i < s.length && s[i] !== q && s[i] !== '\n') {
        if (q === '"' && s[i] === '\\') i++;
        i++;
      }
      i++;
    } else {
      i++;
    }
  }
  return false;
}

/// Split the document at its first `[[line]]`. Everything above it is the
/// owner's own writing and survives a save untouched; a comment below it
/// cannot, so the list editor refuses the document rather than rewriting it.
export function splitHeader(src) {
  const rows = (src ?? '').split('\n');
  const first = rows.findIndex((r) => /^\s*\[\[/.test(r));
  if (first === -1) return { header: rows.join('\n').replace(/\s+$/, ''), blocked: null };
  const commented = hasComment(rows.slice(first).join('\n'));
  return {
    header: rows.slice(0, first).join('\n').replace(/\s+$/, ''),
    blocked: commented
      ? 'This charter has comments in among its lines. Editing it as a list would rewrite the tables and drop them, so it opens as TOML instead.'
      : null,
  };
}

/// A starting point for a new line's id, derived from text the owner typed.
/// Derived once on creation and never re-derived: `GoalRef::Charter` carries
/// an id and no rank, so re-slugging would break recorded references.
export const slugify = (t) =>
  String(t)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .split('-')
    .filter(Boolean)
    .slice(0, 5)
    .join('-')
    .slice(0, 40);
