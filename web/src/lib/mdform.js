// A form over a Markdown file: the page half of `mecha-core/src/mdform.rs`.
// The server splits the file into a title, a note, text and `## ` sections,
// and writes it back from them in one layout; this module edits that shape
// and never writes Markdown itself.
import { repairComments } from './tomlform.js';

let nextId = 1;

// The draft: the doc with an editor-local id on each section, so a moved
// section keeps its box. A section the file must keep (`fixed`) that is
// missing is put back at the top, empty — a change the owner sees before
// saving, not a refusal after.
export function draftOf(doc, fixed = []) {
  const sections = (doc?.sections ?? []).map((s) => ({ id: nextId++, heading: s.heading ?? '', note: s.note ?? '', body: s.body ?? '' }));
  for (const want of [...fixed].reverse()) {
    if (!sections.some((s) => s.heading.trim() === want)) sections.unshift({ id: nextId++, heading: want, note: '', body: '' });
  }
  return { title: doc?.title ?? '', note: doc?.note ?? '', body: doc?.body ?? '', sections };
}

// What the server is sent: the doc without ids, each text trimmed, and a
// phone's "smart" dashes put back into any comment typed in the text.
export function docOf(draft) {
  return {
    title: draft.title.trim(),
    note: draft.note.trim(),
    body: repairComments(draft.body).trim(),
    sections: draft.sections.map((s) => ({ heading: s.heading.trim(), note: s.note.trim(), body: repairComments(s.body).trim() })),
  };
}

export function isDirty(original, draft, fixed = []) {
  return JSON.stringify(docOf(draftOf(original, fixed))) !== JSON.stringify(docOf(draft));
}

export const isFixed = (fixed, s) => fixed.includes(s.heading.trim());

export function move(sections, i, by) {
  const j = i + by;
  if (j < 0 || j >= sections.length) return sections;
  const out = [...sections];
  [out[i], out[j]] = [out[j], out[i]];
  return out;
}

export function addSection(sections) {
  return [...sections, { id: nextId++, heading: '', note: '', body: '' }];
}

// Said before a save, as the server would refuse it after: a note is a
// comment, and these would end or nest it.
export function noteProblem(note) {
  return /-->|<!--/.test(note ?? '') ? 'A note cannot hold --> or <!--.' : null;
}

// Every problem the server would refuse, so the Save button can say why not.
// A title or heading is one line of prompt text; a comment marker in one
// would open or close a comment. The server refuses both (`mdform::one_line`);
// saying so here keeps Save from sending what will come back a 400.
export const MAX_HEADING = 120;
function headingProblem(what, text) {
  if (/<!--|-->/.test(text ?? '')) return `${what} cannot hold <!-- or -->.`;
  if ([...(text ?? '')].length > MAX_HEADING) return `${what} is at most ${MAX_HEADING} characters.`;
  return null;
}

export function problems(draft, fixed = []) {
  const out = [];
  if (headingProblem('The title', draft.title)) out.push(headingProblem('The title', draft.title));
  if (noteProblem(draft.note)) out.push(`Title note: ${noteProblem(draft.note)}`);
  const seen = new Set();
  for (const s of draft.sections) {
    const h = s.heading.trim();
    if (!h) out.push('A section needs a heading.');
    if (h && seen.has(h)) out.push(`Two sections are called “${h}”.`);
    if (headingProblem(`“${h}”`, h)) out.push(headingProblem(`“${h}”`, h));
    seen.add(h);
    if (noteProblem(s.note)) out.push(`${h || 'A section'}: ${noteProblem(s.note)}`);
  }
  for (const want of fixed) if (!seen.has(want)) out.push(`“${want}” must stay.`);
  return out;
}
