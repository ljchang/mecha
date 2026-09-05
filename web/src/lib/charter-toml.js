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

/// The header is preserved — every comment and its text — and only the
/// `[[line]]` tables are generated. Not quite byte-for-byte: `splitHeader`
/// trims trailing whitespace and exactly one blank line is re-emitted before
/// the tables, so a header ending in several blank lines comes back with one.
/// The promise that matters is that nothing the owner wrote is lost.
///
/// A line's `sensor` (`{kind, setpoint}`, as the server serves it) is written
/// back as a `[line.sensor]` sub-table with the owner's own setpoint
/// spelling, always as a string — `setpoint = 3` on disk comes back as
/// `setpoint = "3"`, which the reader types identically. The editor never
/// composes a sensor; this exists so a re-rank or a text edit does not
/// silently delete one (GOAL-SYSTEM-DESIGN §11.1: the parser, this
/// serialiser and the template move together).
export function serialize(header, lines) {
  const out = [];
  if (header.trim()) out.push(header, '');
  for (const l of lines) {
    out.push('[[line]]', `id = ${esc(l.id.trim())}`, `text = ${esc(l.text.trim())}`);
    if (l.sensor && l.sensor.kind) {
      out.push(
        '[line.sensor]',
        `kind = ${esc(String(l.sensor.kind).trim())}`,
        `setpoint = ${esc(String(l.sensor.setpoint ?? '').trim())}`
      );
    }
    out.push('');
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
/// Tables are found by `/^\s*\[\[/`, so a charter written as an inline array
/// (`line = [{ id = "a", text = "b" }]`) lands wholly in the "header" and a
/// save would emit a duplicate `line` key. That fails closed — `Charter::parse`
/// refuses it with a 422 and the draft stays open — and nothing writes that
/// shape, so it is left alone rather than handled.
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

/// The editor's rows from the server's `lines` — one place, so a field the
/// server adds beside `sensor` is carried or dropped on purpose rather than
/// by which literal someone last edited. `sensor` is copied down to its two
/// keys because `serialize` writes it back; `reading` is carried for display
/// and is never serialised (the first cut rebuilt the row without it, and the
/// settings page was the one surface of three showing no reading — found on
/// review). `nextUid` hands each row its editor-local key.
export function rows(lines, nextUid) {
  return (lines ?? []).map((l) => ({
    uid: nextUid(),
    id: l.id,
    text: l.text,
    sensor: l.sensor ? { kind: l.sensor.kind, setpoint: l.sensor.setpoint } : null,
    reading: l.reading ?? null,
    // The sensor the reading was computed against, kept apart from the
    // editable `sensor` so an in-place edit cannot leave a reading beside
    // a setpoint it never saw (`readingStands`).
    read_for: l.sensor && l.reading ? { kind: l.sensor.kind, setpoint: l.sensor.setpoint } : null,
  }));
}

/// Does the row's reading still describe the row's sensor? The server
/// computes `reading.summary` and `reading.over` against the *saved* kind and
/// setpoint; once the owner changes either in the form, the reading is about
/// a sensor that no longer exists on the row, and showing it would let
/// containment 5's guard — the reading beside the value being typed —
/// reassure about the old value (found on review). Same kind and the same
/// setpoint spelling, or the reading stands down until the next save.
export function readingStands(line) {
  if (!line?.reading || !line.sensor || !line.read_for) return false;
  return (
    String(line.sensor.kind ?? '').trim() === String(line.read_for.kind ?? '').trim() &&
    String(line.sensor.setpoint ?? '').trim() === String(line.read_for.setpoint ?? '').trim()
  );
}

/// What a half-filled sensor would cost silently: `serialize` writes no table
/// for a sensor without a kind, so a form the owner opened and left empty
/// would vanish on save with nothing said, and a kind with no setpoint would
/// reach the server only to be refused after the two-tap save. Said here
/// instead, beside the line, before the save is armed.
export function sensorProblems(lines) {
  const out = [];
  for (const [i, l] of lines.entries()) {
    if (!l.sensor) continue;
    const kind = String(l.sensor.kind ?? '').trim();
    const setpoint = String(l.sensor.setpoint ?? '').trim();
    if (!kind && !setpoint) out.push(`Line ${i + 1}'s sensor needs a kind and a setpoint, or remove it.`);
    else if (!kind) out.push(`Line ${i + 1}'s sensor has no kind.`);
    else if (!setpoint) out.push(`Line ${i + 1}'s sensor has no setpoint.`);
  }
  return out;
}
