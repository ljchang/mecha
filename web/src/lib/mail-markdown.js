// Mail bodies as structure, never as markup.
//
// `mecha mail show` gives a thread's messages as the provider's HTML turned
// into markdown — `**bold**`, `[text](url)`, a lone `\` for a hard break —
// and the reader used to print that source verbatim, so every message read
// like a diff. This parses it into a small tree of blocks and inline spans,
// which MailBody.svelte renders with text nodes and a closed set of
// elements. There is no `{@html}` anywhere on the path: the body is a
// stranger's text, and a parser that emitted HTML would be an HTML sanitiser
// with extra steps.
//
// Three rules carry the safety, and each is enforced here, where node can
// test it (web/test/mail-markdown.mjs), rather than in markup:
//
// - **A link is http, https or mailto, or it is text.** Anything else —
//   `javascript:`, `data:`, a relative path — renders as the words it had.
// - **A wrapped link shows where it goes.** Outlook's safelinks and Google's
//   redirector are unwrapped to the destination, which is what the reader
//   needs to judge a link — the wrapper's host is the same for a phish and a
//   paper. The link still points at the destination, not the wrapper.
// - **No image is fetched.** A remote image in mail is a read receipt; the
//   reader shows `image` where it was, and a linked image stays a link.
//
// Anything the parser does not recognise is text, so a body in some other
// dialect degrades to readable prose rather than to a wrong rendering.

const SAFE_SCHEME = /^(https?:|mailto:)/i;

/** Where a link really goes: unwrap the redirectors mail clients insert. */
export function unwrapUrl(href) {
  let url;
  try {
    url = new URL(href);
  } catch {
    return href;
  }
  const host = url.hostname.toLowerCase();
  let inner = null;
  if (host.endsWith('safelinks.protection.outlook.com')) inner = url.searchParams.get('url');
  else if ((host === 'www.google.com' || host === 'google.com') && url.pathname === '/url') {
    inner = url.searchParams.get('q') ?? url.searchParams.get('url');
  }
  if (inner && SAFE_SCHEME.test(inner)) return unwrapUrl(inner);
  return href;
}

/** A link target the reader may follow, unwrapped; null when it is not one. */
export function safeHref(raw) {
  const href = (raw ?? '').trim().replace(/^<|>$/g, '');
  if (!SAFE_SCHEME.test(href)) return null;
  return unwrapUrl(href);
}

/** A long URL, shortened for display: host and the start of the path. */
export function shortUrl(href, max = 60) {
  if (/^mailto:/i.test(href)) return href.slice(7);
  let url;
  try {
    url = new URL(href);
  } catch {
    return href;
  }
  const host = url.hostname.replace(/^www\./, '');
  const rest = (url.pathname === '/' ? '' : url.pathname) + (url.search ? '?…' : '');
  const text = host + rest;
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

const PUNCT = /[!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~]/;

// How far a link's text and destination may run. Beyond these the brackets
// are text: a scan from every `[` to the end of a stranger's body is
// quadratic, and a body of `[` characters would freeze the reading tab
// (found on review). A safelinks-wrapped destination runs to ~600 characters.
const MAX_TEXT = 1000;
const MAX_DEST = 4000;

/** Find the `)` closing a link destination that opens at `start`. */
function closeParen(s, start) {
  let depth = 0;
  const stop = Math.min(s.length, start + MAX_DEST);
  for (let i = start; i < stop; i++) {
    const c = s[i];
    if (c === '\\') { i++; continue; }
    if (c === '(') depth++;
    else if (c === ')') {
      if (depth === 0) return i;
      depth--;
    } else if (c === '\n') return -1;
  }
  return -1;
}

/** Find the `]` closing link text that opens at `start`, allowing nesting. */
function closeBracket(s, start) {
  let depth = 0;
  const stop = Math.min(s.length, start + MAX_TEXT);
  for (let i = start; i < stop; i++) {
    const c = s[i];
    if (c === '\\') { i++; continue; }
    if (c === '[') depth++;
    else if (c === ']') {
      if (depth === 0) return i;
      depth--;
    }
  }
  return -1;
}

/** `url "title"` → url. */
const destOf = (inside) => inside.trim().replace(/\s+("[^"]*"|'[^']*')$/, '').trim();

/** A bare URL's end: trailing punctuation belongs to the sentence. */
function trimUrl(u) {
  let out = u;
  while (/[.,;:!?'"*_]$/.test(out)) out = out.slice(0, -1);
  // A closing paren the URL did not open is the sentence's.
  while (out.endsWith(')') && (out.match(/\(/g) ?? []).length < (out.match(/\)/g) ?? []).length) out = out.slice(0, -1);
  return out;
}

const MAX_DEPTH = 6;

// Link text that reads as an address — with a scheme, `www.`, a bare
// `host.tld/path`, or an email address. Such text is replaced by the real
// destination, because a reader takes it as the destination: the words
// `mail.dartmouth.edu/login` over `https://evil.example/login` is the phish
// the unwrapping rule exists against, and a phone has no hover to catch it.
const URLISH = /^\s*<?(?:[a-z][a-z0-9+.-]*:\/\/\S+|www\.\S+|[a-z0-9-]+(?:\.[a-z0-9-]+)*\.[a-z]{2,}(?:[/:?#]\S*)?)>?\s*$/i;
const EMAILISH = /^\s*(?:mailto:)?[^\s@]+@[^\s@]+\.[a-z]{2,}\s*$/i;

/**
 * Inline spans: `{t:'text', v}`, `{t:'strong'|'em', c}`, `{t:'code', v}`,
 * `{t:'link', href, c}`, `{t:'img', alt}`, `{t:'br'}`.
 */
export function parseInline(s, depth = 0, inLink = false) {
  const out = [];
  let buf = '';
  const flush = () => {
    if (buf) out.push({ t: 'text', v: buf });
    buf = '';
  };
  const push = (node) => {
    flush();
    out.push(node);
  };
  if (depth > MAX_DEPTH) return [{ t: 'text', v: s }];

  let i = 0;
  while (i < s.length) {
    const c = s[i];
    const rest = s.slice(i);

    if (c === '\\' && i + 1 < s.length && PUNCT.test(s[i + 1])) {
      buf += s[i + 1];
      i += 2;
      continue;
    }
    if (c === '\n') {
      push({ t: 'br' });
      i++;
      continue;
    }
    if (c === '`') {
      const end = s.indexOf('`', i + 1);
      if (end > i + 1) {
        push({ t: 'code', v: s.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    // ![alt](src) — never fetched.
    if (c === '!' && s[i + 1] === '[') {
      const close = closeBracket(s, i + 2);
      if (close > 0 && s[close + 1] === '(') {
        const end = closeParen(s, close + 2);
        if (end > 0) {
          push({ t: 'img', alt: s.slice(i + 2, close).trim() });
          i = end + 1;
          continue;
        }
      }
    }
    // [text](dest)
    if (c === '[' && !inLink) {
      const close = closeBracket(s, i + 1);
      if (close > 0 && s[close + 1] === '(') {
        const end = closeParen(s, close + 2);
        if (end > 0) {
          const words = s.slice(i + 1, close);
          const inner = parseInline(words, depth + 1, true);
          const href = safeHref(destOf(s.slice(close + 2, end)));
          // Text that is itself a URL shows where the link really goes: the
          // words and the destination can disagree, and the destination is
          // the one that matters.
          const urlish = URLISH.test(words) || EMAILISH.test(words);
          if (href) push({ t: 'link', href, c: inner.length && !urlish ? inner : [{ t: 'text', v: shortUrl(href) }] });
          else {
            flush();
            out.push(...inner);
          }
          i = end + 1;
          continue;
        }
      }
    }
    // <https://…>
    if (c === '<' && !inLink) {
      const m = /^<((?:https?|mailto):[^>\s]+)>/i.exec(rest);
      if (m) {
        const href = safeHref(m[1]);
        push({ t: 'link', href, c: [{ t: 'text', v: shortUrl(href) }] });
        i += m[0].length;
        continue;
      }
    }
    // A bare URL, at a word boundary.
    if (!inLink && (c === 'h' || c === 'H') && (i === 0 || /[\s(]/.test(s[i - 1]))) {
      const m = /^https?:\/\/[^\s<>]+/i.exec(rest);
      if (m) {
        const raw = trimUrl(m[0]);
        const href = safeHref(raw);
        if (href) {
          push({ t: 'link', href, c: [{ t: 'text', v: shortUrl(href) }] });
          i += raw.length;
          continue;
        }
      }
    }
    // **strong** / __strong__, then *em* / _em_.
    if (c === '*' || c === '_') {
      const dbl = s[i + 1] === c;
      const mark = dbl ? c + c : c;
      const open = i + mark.length;
      // Intraword `_` is a snake_case name, not emphasis.
      const wordBefore = i > 0 && /\w/.test(s[i - 1]);
      const opensOk = open < s.length && !/\s/.test(s[open]) && !(c === '_' && wordBefore);
      if (opensOk) {
        let end = s.indexOf(mark, open + 1);
        // Skip a closer that is really the start of a longer run (`***`).
        while (end > 0 && !dbl && s[end + 1] === c) end = s.indexOf(mark, end + 2);
        // A closer past MAX_TEXT does not close: emphasis is a phrase.
        if (end > open + MAX_TEXT) end = -1;
        const closesOk = end > open && !/\s/.test(s[end - 1]) && s.slice(open, end).indexOf('\n\n') < 0
          && !(c === '_' && /\w/.test(s[end + mark.length] ?? ''));
        if (closesOk) {
          push({ t: dbl ? 'strong' : 'em', c: parseInline(s.slice(open, end), depth + 1, inLink) });
          i = end + mark.length;
          continue;
        }
      }
      // An unmatched run of markers is text, all of it.
      let j = i;
      while (s[j] === c) j++;
      buf += s.slice(i, j);
      i = j;
      continue;
    }
    buf += c;
    i++;
  }
  flush();
  return out;
}

const HR = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const BULLET = /^\s{0,3}[-*+•]\s+(.*)$/;
const ORDERED = /^\s{0,3}(\d{1,3})[.)]\s+(.*)$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const FENCE = /^\s{0,3}(```|~~~)/;

/** Below this, a line ended where its writer ended it; at or above, a mail client wrapped it. */
const WRAP_AT = 60;

/**
 * A paragraph's lines, joined. A trailing `\` or two spaces is a hard break,
 * a lone `\` line is nothing, and a newline after a short line is kept — a
 * signature, an address, a list typed without markers. A newline after a
 * *long* line is a plain-text client's hard wrap at 72-odd columns, and is a
 * space: kept, it breaks sentences mid-phrase on any screen narrower than
 * the sender's.
 */
function paraText(lines) {
  const kept = lines.filter((l) => l.trim() !== '\\');
  let out = '';
  kept.forEach((l, i) => {
    const hard = /\\$/.test(l) || / {2,}$/.test(l);
    const text = l.replace(/\\$/, '').replace(/\s+$/, '');
    out += text;
    if (i < kept.length - 1) out += !hard && text.length >= WRAP_AT ? ' ' : '\n';
  });
  return out;
}

/**
 * Blocks: `{type:'p'|'h', level?, inline}`, `{type:'ul'|'ol', start?, items: inline[]}`,
 * `{type:'quote', blocks}`, `{type:'hr'}`, `{type:'code', text}`.
 */
export function parseBlocks(text, depth = 0) {
  const lines = (text ?? '').replace(/\r\n?/g, '\n').split('\n');
  const blocks = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === '' || line.trim() === '\\') {
      i++;
      continue;
    }
    if (FENCE.test(line)) {
      const fence = FENCE.exec(line)[1];
      const body = [];
      i++;
      while (i < lines.length && !lines[i].trimStart().startsWith(fence)) body.push(lines[i++]);
      i++;
      blocks.push({ type: 'code', text: body.join('\n') });
      continue;
    }
    if (HR.test(line)) {
      blocks.push({ type: 'hr' });
      i++;
      continue;
    }
    const h = HEADING.exec(line);
    if (h) {
      blocks.push({ type: 'h', level: h[1].length, inline: parseInline(h[2]) });
      i++;
      continue;
    }
    if (QUOTE.test(line) && depth < MAX_DEPTH) {
      const inner = [];
      while (i < lines.length && QUOTE.test(lines[i])) inner.push(QUOTE.exec(lines[i++])[1]);
      blocks.push({ type: 'quote', blocks: parseBlocks(inner.join('\n'), depth + 1) });
      continue;
    }
    const listKind = BULLET.test(line) ? 'ul' : ORDERED.test(line) ? 'ol' : null;
    if (listKind) {
      const re = listKind === 'ul' ? BULLET : ORDERED;
      const start = listKind === 'ol' ? Number(ORDERED.exec(line)[1]) : undefined;
      const items = [];
      while (i < lines.length) {
        const m = re.exec(lines[i]);
        if (m) {
          items.push([listKind === 'ul' ? m[1] : m[2]]);
          i++;
        } else if (lines[i].trim() !== '' && /^\s{2,}\S/.test(lines[i]) && items.length) {
          // A continuation line, indented under its item.
          items[items.length - 1].push(lines[i].trim());
          i++;
        } else break;
      }
      blocks.push({ type: listKind, start, items: items.map((ls) => parseInline(paraText(ls))) });
      continue;
    }
    // A paragraph runs to the next blank line or the next block opener.
    const para = [];
    while (
      i < lines.length &&
      lines[i].trim() !== '' &&
      !(para.length && (HR.test(lines[i]) || HEADING.test(lines[i]) || QUOTE.test(lines[i]) || FENCE.test(lines[i])))
    ) {
      para.push(lines[i++]);
    }
    const joined = paraText(para);
    if (joined.trim()) blocks.push({ type: 'p', inline: parseInline(joined) });
  }
  return blocks;
}

/** Every link in a tree, for tests and for a "links in this message" list. */
export function linksOf(blocks) {
  const out = [];
  const walkInline = (nodes) => {
    for (const n of nodes ?? []) {
      if (n.t === 'link') out.push(n.href);
      if (n.c) walkInline(n.c);
    }
  };
  for (const b of blocks) {
    if (b.inline) walkInline(b.inline);
    if (b.items) b.items.forEach(walkInline);
    if (b.blocks) out.push(...linksOf(b.blocks));
  }
  return out;
}
