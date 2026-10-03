// A reply made fit to be heard (the owner's ask, 2026-10-01: a play button
// on each reply, the text written as usual and tidied for speech). Pure, so
// node can test it (web/test/speech.mjs); `reply-player.svelte.js` plays it.
//
// What a listener should not hear: Markdown's marks, a URL spelled out, a
// code block read character by character, a persona's bracketed citation.
// What they should: the words, a link's label, "from the field guide".

// A persona's citation, as the page parses it (`citeSegments`): `[file, p. N:
// "quote"]` or `[file: "quote"]`. Spoken as where it came from.
const CITE = /\[([^\]\n]+?)(?:,\s*p\.\s*\d+)?:\s*"[^"\n]*"\]/g;

function fileWords(file) {
  return file
    .replace(/^@[^/]*\//, '')
    .replace(/\.[a-z0-9]{1,5}$/i, '')
    .replace(/[-_]+/g, ' ')
    .trim();
}

/** `text` (a reply's Markdown) as plain sentences to speak. */
export function speakable(text) {
  let s = String(text ?? '');
  // Code blocks: never read out.
  s = s.replace(/```[\s\S]*?(```|$)/g, '\nThere is a code block here.\n');
  // Citations, before links (both are bracketed).
  s = s.replace(CITE, (_, file) => `(from ${fileWords(file)})`);
  // Images: their alt text, if any. Links: their words.
  s = s.replace(/!\[([^\]]*)\]\([^)]*\)/g, (_, alt) => (alt ? `a picture: ${alt}` : 'a picture'));
  s = s.replace(/\[([^\]]+)\]\([^)]*\)/g, '$1');
  // A bare URL is "a link", not its spelling.
  s = s.replace(/\bhttps?:\/\/\S+/g, 'a link');
  // Inline code keeps its words; the marks go.
  s = s.replace(/`([^`\n]+)`/g, '$1');
  const lines = s.split('\n').map((line) =>
    line
      .replace(/^\s{0,3}#{1,6}\s+/, '') // headings
      .replace(/^\s{0,3}>\s?/, '') // quotes
      .replace(/^\s*(?:[-*+]|\d+[.)])\s+/, '') // list markers
      .replace(/^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/, '') // rules
      .replace(/(\*\*|__)(.+?)\1/g, '$2') // bold
      .replace(/(\*|_)(?=\S)(.+?)(?<=\S)\1/g, '$2') // italics
      .replace(/~~(.+?)~~/g, '$1')
      .replace(/\|/g, ' ') // table pipes
      .trim(),
  );
  // A line that ends without punctuation (a list item, a heading) still
  // ends a sentence when heard.
  return lines
    .filter((l) => l && !/^[-:\s]+$/.test(l))
    .map((l) => (/[.!?:;)]$/.test(l) ? l : `${l}.`))
    .join(' ')
    .replace(/\s+/g, ' ')
    .trim();
}

/**
 * The sentences of `text`, each at most `max` characters, in order
 * (`speechPieces` groups them for the player). A sentence ends at `.`,
 * `!` or `?` before a space or the end — so "3.5" and "e.g." inside a word
 * stay whole — with any closing quote or bracket kept on it. A sentence
 * longer than `max` is cut at a word. A piece with no letter or digit in it
 * (a trailing "...") is dropped: the speech engine invents words for one.
 */
export function speechSentences(text, max = 400) {
  const s = String(text ?? '').trim();
  const ends = /[.!?]+["'”’)\]]*(?=\s|$)/g;
  const sentences = [];
  let start = 0;
  for (let m = ends.exec(s); m; m = ends.exec(s)) {
    const end = m.index + m[0].length;
    sentences.push(s.slice(start, end).trim());
    start = end;
  }
  if (start < s.length) sentences.push(s.slice(start).trim());
  const out = [];
  for (let rest of sentences) {
    while (rest.length > max) {
      const at = rest.lastIndexOf(' ', max);
      const cut = at > 0 ? at : max;
      out.push(rest.slice(0, cut).trim());
      rest = rest.slice(cut).trim();
    }
    out.push(rest);
  }
  return out.filter((p) => /[\p{L}\p{N}]/u.test(p));
}

/**
 * The pieces the player asks for, in order: the first sentence alone, so
 * speech starts as soon as one sentence is spoken, then the rest grouped up
 * to `max` characters, so each piece is made while the one before plays and
 * the voice does not stop between sentences. Every piece of a reply is
 * spoken with the one direction serve settles for it (Listen's director
 * pass), so grouping changes nothing about how it sounds.
 */
export function speechPieces(text, max = 400) {
  const [first, ...rest] = speechSentences(text, max);
  if (first === undefined) return [];
  const out = [first];
  let cur = '';
  for (const s of rest) {
    if (cur && cur.length + 1 + s.length > max) {
      out.push(cur);
      cur = '';
    }
    cur = cur ? `${cur} ${s}` : s;
  }
  if (cur) out.push(cur);
  return out;
}

/**
 * A reply's key for Listen: the same text, the same key, so a second tap on
 * a reply is spoken with the direction the first was given (serve records
 * it under the key). Two 32-bit FNV-1a hashes over the text — a join key within
 * one chat, not a secret.
 */
export function replyKey(text) {
  const t = String(text ?? '');
  let a = 0x811c9dc5;
  let b = 0x01000193 ^ 0x5bd1e995;
  for (let i = 0; i < t.length; i += 1) {
    const c = t.charCodeAt(i);
    a = Math.imul(a ^ c, 0x01000193) >>> 0;
    b = Math.imul(b ^ c, 0x01000193) >>> 0;
  }
  return 'r' + a.toString(16).padStart(8, '0') + b.toString(16).padStart(8, '0');
}

/**
 * What the director is told about the moment a reply was said in: the
 * owner's words it answers (`asked`) and what the speaker said before it
 * (`lastReply`) — read from the page's entries, nearest first, as a call's
 * scene is built from the conversation. `words` is how the page shows the
 * owner's text — a persona chat's strips its goal preamble (`ownWords`) —
 * so the director reads what the owner said, not the harness's framing.
 */
export function replyContext(entries, i, words = (t) => t) {
  let asked = null;
  let lastReply = null;
  for (let j = i - 1; j >= 0; j -= 1) {
    const e = entries[j];
    if (asked === null) {
      const said = e?.kind === 'user' ? words(e.text ?? '') : '';
      if (said?.trim()) asked = said;
    } else if (e?.kind === 'assistant' && e.text?.trim()) {
      lastReply = e.text;
      break;
    }
  }
  return { asked, lastReply };
}
