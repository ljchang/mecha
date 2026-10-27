// Settings → LoRAs, the pure half (web/test/loras.mjs): the owner's LoRA
// registry as `/api/loras/{image|video}` serves it, and the form an entry is
// added or edited through (LORA-DESIGN.md §6). The server's parser is the
// judge of an entry; nothing here restates its rules beyond what a form
// needs to start from.

/** The two sections, each under its own feature and route. */
export const SECTIONS = [
  { family: 'qwen-image', route: 'image', feature: 'image', title: 'Pictures' },
  { family: 'h3', route: 'video', feature: 'video', title: 'Clips' },
];

/**
 * Every endpoint the pane reaches, as path shapes: the URLs are built here
 * rather than named after `fetch(`, so the docs demo's drift guard
 * (website/scripts/check-demo.mjs) reads this list instead.
 */
export const LORA_ENDPOINTS = SECTIONS.flatMap((s) => [`/api/loras/${s.route}`, `/api/loras/${s.route}/X`]);

/** A section's list URL, with the unlock when there is one. */
export function listUrl(route, token) {
  return token ? `/api/loras/${route}?unlock=${encodeURIComponent(token)}` : `/api/loras/${route}`;
}

/** An entry's route, for an edit or a remove. */
export function entryUrl(route, name, token) {
  const base = `/api/loras/${route}/${encodeURIComponent(name)}`;
  return token ? `${base}?unlock=${encodeURIComponent(token)}` : base;
}

/**
 * What an entry's row says about where it stands. A checked picture entry
 * says it is not applied yet: the picture graph has no LoRA node until the
 * build's step 3. A checked clip entry reaches clips once mecha restarts,
 * since the clip tool reads the registry when it starts.
 */
export function standing(entry, family) {
  if (entry.status !== 'checked') return { text: entry.reason ?? 'left out', bad: true };
  if (family === 'qwen-image') return { text: 'checked · not applied to pictures yet', bad: false };
  return { text: entry.always ? 'checked · on every clip' : 'checked', bad: false };
}

/** The files an entry names, as a row lists them. */
export function filesOf(entry) {
  if (entry.file) return [entry.file];
  if (entry.frames && entry.frames === entry.references) return [entry.frames];
  return [entry.frames && `frames: ${entry.frames}`, entry.references && `references: ${entry.references}`].filter(Boolean);
}

/** A row's second line: its files, strength and trigger. */
export function rowDetail(entry) {
  const bits = [...filesOf(entry), `strength ${entry.strength}`];
  if (entry.trigger) bits.push(`trigger "${entry.trigger}"`);
  return bits.join(' · ');
}

/** The toggle for a server's files of the other model, or unreadable ones. */
export function others(unused, family) {
  const n = unused.filter((u) => u.family !== family).length;
  return `${n} other ${n === 1 ? 'file' : 'files'} on this server`;
}

/** A name to start from, out of a file's stem: what `name` accepts. */
export function suggestName(file) {
  const stem = (file.split('/').pop() ?? file).replace(/\.safetensors$/, '');
  return stem
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 48);
}

/** A blank form, or one for a server file no entry uses yet. */
export function formFor(family, file = '') {
  return {
    editing: null,
    name: file ? suggestName(file) : '',
    about: '',
    file: family === 'qwen-image' ? file : '',
    frames: family === 'h3' ? file : '',
    references: family === 'h3' ? file : '',
    strength: '',
    trigger: '',
    always: false,
  };
}

/** The form for editing an entry as it stands. */
export function formFrom(entry) {
  return {
    editing: entry.name,
    name: entry.name,
    about: entry.about ?? '',
    file: entry.file ?? '',
    frames: entry.frames ?? '',
    references: entry.references ?? '',
    strength: entry.strength == null || entry.strength === 1 ? '' : String(entry.strength),
    trigger: entry.trigger ?? '',
    always: !!entry.always,
  };
}

/**
 * The body's `entry` for a form: a picture LoRA's one file, or a clip
 * LoRA's file per mode (one file for both when they match). An empty
 * strength is the default; one that is not a number is sent as it is, for
 * the server to refuse with its reason.
 */
export function entryOf(form, family) {
  const trim = (s) => (s ?? '').trim();
  const out = { name: trim(form.name), about: trim(form.about) };
  if (family === 'qwen-image') {
    out.file = trim(form.file);
  } else {
    const fr = trim(form.frames);
    const re = trim(form.references);
    if (fr && fr === re) out.file = fr;
    else {
      if (fr) out.frames = fr;
      if (re) out.references = re;
    }
    if (form.always) out.always = true;
  }
  const s = trim(String(form.strength ?? ''));
  if (s) out.strength = Number(s);
  if (trim(form.trigger)) out.trigger = trim(form.trigger);
  return out;
}

/** What an Add list row says about a file's family. */
export function unusedLine(u, family) {
  if (u.family === family) return { text: u.note ?? '', ok: true };
  if (u.family) return { text: `a ${u.family === 'h3' ? 'clip' : 'picture'} LoRA, not this model's`, ok: false };
  return { text: u.note ?? 'its model could not be read', ok: false };
}
