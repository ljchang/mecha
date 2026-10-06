// A reply made fit to be heard (the owner's ask, 2026-10-01: a play button
// on each reply, the text written as usual and tidied for speech). Pure, so
// node can test it (web/test/speech.mjs); `reply-player.svelte.js` plays it.
//
// What a listener should not hear: Markdown's marks, a URL spelled out, a
// code block read character by character, a persona's bracketed citation.
// What they should: the words, a link's label, "from the field guide".
//
// A call speaks by the same rule, applied as the reply streams
// (`mecha-cli/src/voice/speech.rs`); `test/speakable-cases.json` holds the
// two to it, read by both suites — change a rule here and that file, and the
// Rust test says whether the call still agrees.

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

// A whole line that only points at a picture the chat already draws —
// "[Image: images/20261006-….png]", which a persona sometimes types after
// making one, echoing the tool's own "image: <path>" line. Nothing to read
// and nothing to say: the picture card is the picture (2026-10-06). Narrow
// on purpose — a bracketed description in the persona's own words stays.
// The path is `picture.js`'s `image_generate` form — what the tool writes
// and the card draws — so no line is hidden that no card stands in for
// (review of #578).
const PICTURE_REF = /^[ \t]*\[\s*image\s*:\s*images\/[A-Za-z0-9._-]+\.png\]\s*$/i;

/** `text` without its picture-reference lines (`PICTURE_REF`), for display.
 * Never inside a fenced code block: a block shows what was written, whole
 * (review of #578). */
export function withoutPictureRefs(text) {
  const s = String(text ?? '');
  if (!s.includes('[')) return s;
  // The renderer's own fence rule (`mail-markdown.js` `parseBlocks`): a
  // fence opens on ``` or ~~~ indented at most three spaces, and only the
  // same marker closes it (review of #578).
  let fence = null;
  return s
    .split('\n')
    .filter((line) => {
      const m = /^ {0,3}(```|~~~)/.exec(line);
      if (m && (fence === null || m[1] === fence)) {
        fence = fence === null ? m[1] : null;
        return true;
      }
      return fence !== null || !PICTURE_REF.test(line);
    })
    .join('\n');
}

/** `text` (a reply's Markdown) as plain sentences to speak. */
export function speakable(text) {
  let s = withoutPictureRefs(text);
  // Code blocks: never read out.
  s = s.replace(/```[\s\S]*?(```|$)/g, '\nThere is a code block here.\n');
  // Citations, before links (both are bracketed).
  s = s.replace(CITE, (_, file) => `(from ${fileWords(file)})`);
  // Images: their alt text, if any. Links: their words.
  s = s.replace(/!\[([^\]]*)\]\([^)]*\)/g, (_, alt) => (alt ? `a picture: ${alt}` : 'a picture'));
  s = s.replace(/\[([^\]]+)\]\([^)]*\)/g, '$1');
  // A bare URL is "a link", not its spelling.
  // Less the punctuation that ends it: the sentence's own stop is not the
  // address's ("Read https://a.io/x. It's good." is two sentences).
  s = s.replace(/\bhttps?:\/\/\S*[^\s.,!?;:'")\]”’]/g, 'a link');
  // Inline code keeps its words; the marks go.
  s = s.replace(/`([^`\n]+)`/g, '$1');
  const lines = s.split('\n').map((line) =>
    line
      .replace(/^\s{0,3}#{1,6}\s+/, '') // headings
      .replace(/^\s{0,3}>\s?/, '') // quotes
      .replace(/^\s*(?:[-*+]|\d+[.)])\s+/, '') // list markers
      .replace(/^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/, '') // rules
      .replace(/(\*\*|__)(.+?)\1/g, '$2') // bold
      .replace(/\*(?=\S)(.+?)(?<=\S)\*/g, '$1') // italics
      // Underscores only around words: one inside a word (snake_case) is
      // the word's, as CommonMark has it.
      .replace(/(?<![\p{L}\p{N}])_(?=\S)(.+?)(?<=\S)_(?![\p{L}\p{N}])/gu, '$1')
      .replace(/~~(.+?)~~/g, '$1')
      .replace(/\|/g, ' ') // table pipes
      .trim(),
  );
  // A line that ends without punctuation (a list item, a heading) still
  // ends a sentence when heard.
  return lines
    .filter((l) => l && !/^[-:\s]+$/.test(l))
    .map((l) => (/[.!?:;)。！？]$/.test(l) ? l : `${l}.`))
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
 * The pieces the player asks for, in order: whole sentences grouped up to
 * `max` characters. Each piece streams, heard from its first chunk, so a
 * piece's length no longer decides how soon it is heard — the first is not
 * cut short to start sooner, which left a one-word opener ("Oh.") to an
 * engine that invents speech around too little text, and a long second
 * piece made in silence behind it. The cap keeps a piece inside what the
 * engine renders whole (a long passage can stop a sentence early). Every
 * piece of a reply is spoken with the one direction serve settles for it
 * (Listen's director pass), so grouping changes nothing about how it sounds.
 */
export function speechPieces(text, max = 250) {
  const out = [];
  let cur = '';
  for (const s of speechSentences(text, max)) {
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
