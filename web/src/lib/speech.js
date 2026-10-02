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
 * Sentence-sized pieces of `text`, each at most `max` characters, in order:
 * the player asks for one while the last plays, so speech starts after the
 * first sentence rather than the whole reply. A sentence longer than `max`
 * is cut at a word.
 */
export function speechChunks(text, max = 400) {
  const sentences = String(text ?? '').match(/[^.!?]+(?:[.!?]+|$)\s*/g) ?? [];
  const out = [];
  let cur = '';
  const push = () => {
    if (cur.trim()) out.push(cur.trim());
    cur = '';
  };
  for (let s of sentences) {
    while (s.length > max) {
      const cut = s.lastIndexOf(' ', max) > 0 ? s.lastIndexOf(' ', max) : max;
      push();
      out.push(s.slice(0, cut).trim());
      s = s.slice(cut);
    }
    if (cur.length + s.length > max) push();
    cur += s;
  }
  push();
  return out.filter(Boolean);
}
