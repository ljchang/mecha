// The charter document, as bytes.
//
// Extracted from `SettingsCharter.svelte` so it can be tested: these are the
// functions that decide what reaches `~/.mecha/charter.toml`, and inside a
// component the only way to exercise them was to drive a browser. Two
// languages describe this file — these functions write it, `Charter::parse`
// decides whether it loads — so `website/scripts/check-charter-toml.mjs`
// pins the agreement against the sample `charter.rs` reads back.

/// Always a single-line basic string: unambiguous, and it matches how the
/// file is already written. A control character TOML forbids is refused by
/// the server's parse, which is the check that counts.
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

/// A `#` outside a string opens a comment — including one trailing a value,
/// which a whole-line regex misses and the regenerating serialiser would then
/// drop silently. Tracks basic and literal strings, respecting escapes. A `#`
/// inside a multi-line string reads as a comment here and blocks the list
/// editor: wrong, but wrong in the direction that keeps the writing.
export function hasComment(row) {
  let basic = false;
  let literal = false;
  for (let i = 0; i < row.length; i++) {
    const c = row[i];
    if (basic) {
      if (c === '\\') i++;
      else if (c === '"') basic = false;
    } else if (literal) {
      if (c === "'") literal = false;
    } else if (c === '"') basic = true;
    else if (c === "'") literal = true;
    else if (c === '#') return true;
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
  const commented = rows.slice(first).some(hasComment);
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
