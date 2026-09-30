// A form over a TOML file: the page half of `mecha-core/src/tomlform.rs`.
//
// The page never writes TOML. It holds a draft of the field values the
// server read, and sends back only the fields that differ, as
// `{path: value}`; the server sets each one in place, keeping the owner's
// comments, and refuses a path or value the form did not offer. So this
// module needs to know nothing about TOML — only which values changed.

// A value as the draft holds it: text is never null (an empty box), a list
// is never null (no chips).
export function draftValue(field, v) {
  if (field.kind === 'text') return v ?? '';
  if (field.kind === 'chips') return Array.isArray(v) ? [...v] : [];
  if (field.kind === 'toggle') return Boolean(v);
  return v ?? null;
}

export function draftOf(form, values) {
  const out = {};
  for (const s of form.sections) for (const f of s.fields) out[f.path] = draftValue(f, values?.[f.path]);
  return out;
}

function same(a, b) {
  if (Array.isArray(a) || Array.isArray(b)) {
    return Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((x, i) => x === b[i]);
  }
  return a === b;
}

// The changes to send: each field whose draft differs from what was read.
// An optional text left empty is sent as null (the key removed), and text is
// compared trimmed, so a stray space is not a change.
export function changesOf(form, values, draft) {
  const out = {};
  for (const s of form.sections) {
    for (const f of s.fields) {
      const was = draftValue(f, values?.[f.path]);
      let now = draft[f.path];
      if (f.kind === 'text') {
        now = String(now ?? '').trim();
        if (now === String(was).trim()) continue;
        out[f.path] = now === '' && f.optional ? null : now;
        continue;
      }
      if (!same(was, now)) out[f.path] = now;
    }
  }
  return out;
}

// A closed choice with few, short labels reads best as a segmented control;
// anything longer, or one that may be left unset, as a select.
export function segmented(field) {
  return field.kind === 'choice' && !field.none && field.options.length <= 3
    && field.options.every((o) => o.label.length <= 18);
}

// The chips to draw: every option, then any value the file holds that is not
// an option (a name typed free, or one the viewer cannot see listed) — shown
// so it can be removed, never silently dropped.
export function chipsFor(field, value) {
  const known = new Set(field.options.map((o) => o.value));
  return [
    ...field.options,
    ...(value ?? []).filter((v) => !known.has(v)).map((v) => ({ value: v, label: v, extra: true })),
  ];
}

export function toggleChip(value, name) {
  const list = value ?? [];
  return list.includes(name) ? list.filter((v) => v !== name) : [...list, name];
}

// A name typed into a free chip field: trimmed, and added once.
export function addChip(value, typed) {
  const name = String(typed ?? '').trim();
  const list = value ?? [];
  if (!name || list.includes(name)) return list;
  return [...list, name];
}

// iOS "smart punctuation" turns a typed `--` into a dash, so a comment
// typed on a phone arrives as `<!—` … `—>` and is no longer a comment — the
// note would be sent to every chat. Only those exact spellings are put
// back; a dash anywhere else is the owner's.
export function repairComments(text) {
  return String(text).replace(/<!\s?[—–]/g, '<!--').replace(/[—–]>/g, '-->');
}
