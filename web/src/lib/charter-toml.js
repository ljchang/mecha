// The charter's rows, as the settings page edits them.
//
// The page never writes TOML. It sends its rows — `toRows` — and the server
// sets them in place in `~/.mecha/charter.toml` (`charter::form` over
// `tomlform`), keeping the owner's comments, among the lines as well as above
// them, and each untouched value exactly as written. What is left here is the
// page's half: turning the server's lines into editor rows, naming problems
// before a save, and whether a reading still describes its sensor. (The
// JavaScript serialiser that used to live here regenerated every table, so a
// comment among the lines could not survive a save, and a second script had
// to keep it agreeing with the Rust reader.)

/// What a list save sends: each row as the charter form takes it — id and
/// text trimmed, a sensor as its kind and setpoint (the owner's spelling,
/// trimmed) or `null`. Nothing else: the reading, the render key and what it
/// was read for are the page's, and the server refuses any other field.
export function toRows(lines) {
  return lines.map((l) => ({
    id: String(l.id ?? '').trim(),
    text: String(l.text ?? '').trim(),
    sensor:
      l.sensor && String(l.sensor.kind ?? '').trim()
        ? { kind: String(l.sensor.kind).trim(), setpoint: String(l.sensor.setpoint ?? '').trim() }
        : null,
  }));
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
/// keys because `toRows` sends them back; `reading` is carried for display
/// and is never sent (the first cut rebuilt the row without it, and the
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

/// What a half-filled sensor would cost silently: `toRows` sends no sensor
/// for one without a kind, so a form the owner opened and left empty
/// would vanish on save with nothing said, and a kind with no setpoint would
/// reach the server only to be refused after the two-tap save. Said here
/// instead, beside the line, before the save is armed.
export function sensorProblems(lines) {
  const out = [];
  const seen = new Map();
  for (const [i, l] of lines.entries()) {
    if (!l.sensor) continue;
    const kind = String(l.sensor.kind ?? '').trim();
    const setpoint = String(l.sensor.setpoint ?? '').trim();
    if (!kind && !setpoint) out.push(`Line ${i + 1}'s sensor needs a kind and a setpoint, or remove it.`);
    else if (!kind) out.push(`Line ${i + 1}'s sensor has no kind.`);
    else if (!setpoint) out.push(`Line ${i + 1}'s sensor has no setpoint.`);
    // The parser refuses two lines of one kind (only the higher-ranked
    // would ever be attributed anything); said here, before the save.
    if (kind) {
      if (seen.has(kind)) out.push(`Lines ${seen.get(kind) + 1} and ${i + 1} both carry a ${kind} sensor — keep one.`);
      else seen.set(kind, i);
    }
  }
  return out;
}
