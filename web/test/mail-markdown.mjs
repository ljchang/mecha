// Behaviour checks for the mail body parser (src/lib/mail-markdown.js).
//
// **What is worth pinning.** The safety rules, first: a link that is not
// http, https or mailto must come out as text, a redirector must be unwrapped
// to where it goes, and an image must never become something a browser
// fetches. Then the shapes real mail arrives in — the `\` hard breaks, the
// escaped asterisks and the safelinks-wrapped `[url](wrapper)` pairs that made
// the reader look like a diff.
import { parseInline, parseBlocks, safeHref, unwrapUrl, shortUrl, linksOf } from '../src/lib/mail-markdown.js';

let pass = 0;
let fail = 0;
const t = (name, cond) => {
  if (cond) {
    pass += 1;
    console.log('  ok   ', name);
  } else {
    fail += 1;
    console.log('  FAIL ', name);
  }
};
const textOf = (nodes) => nodes.map((n) => (n.t === 'text' ? n.v : n.t === 'br' ? '\n' : n.c ? textOf(n.c) : n.v ?? '')).join('');

// ---- links are safe or they are text ----
t('javascript: is not a link', safeHref('javascript:alert(1)') === null);
t('data: is not a link', safeHref('data:text/html,<b>x</b>') === null);
t('a relative path is not a link', safeHref('/api/outbox/1/approve') === null);
t('https is a link', safeHref('https://example.org/a') === 'https://example.org/a');
t('mailto is a link', safeHref('mailto:a@example.org') === 'mailto:a@example.org');
{
  const nodes = parseInline('[click me](javascript:alert(1))');
  t('an unsafe link keeps its words as text', nodes.length === 1 && nodes[0].t === 'text' && nodes[0].v === 'click me');
}

// ---- redirectors are unwrapped ----
const wrapped = 'https://nam12.safelinks.protection.outlook.com/?url=https%3A%2F%2Fwww.nature.com%2Farticles%2Fs41586-025-08993-1&data=05%7C02&reserved=0';
t('safelinks unwrap to the destination', unwrapUrl(wrapped) === 'https://www.nature.com/articles/s41586-025-08993-1');
t('google redirector unwraps', unwrapUrl('https://www.google.com/url?q=https://example.org/x&sa=D') === 'https://example.org/x');
t('a wrapper around javascript: stays the wrapper', unwrapUrl('https://nam12.safelinks.protection.outlook.com/?url=javascript%3Aalert(1)').startsWith('https://nam12'));
t('a short URL is host and path', shortUrl('https://www.nature.com/articles/s41586') === 'nature.com/articles/s41586');
t('a query is elided', shortUrl('https://dartmouth.zoom.us/j/95332509984?pwd=abc') === 'dartmouth.zoom.us/j/95332509984?…');

{
  // The shape in the B4 announcement: the visible text is a URL and the
  // destination is its safelinks wrapper.
  const src = `Here's a link to the paper:  [https://www.nature.com/articles/s41586-025-08993-1](${wrapped})`;
  const nodes = parseInline(src);
  const link = nodes.find((n) => n.t === 'link');
  t('a [url](wrapper) pair is one link', !!link && nodes.filter((n) => n.t === 'link').length === 1);
  t('…pointing at the destination', link?.href === 'https://www.nature.com/articles/s41586-025-08993-1');
  t('…and the wrapper never shows', !textOf(nodes).includes('safelinks'));
}

{
  const nodes = parseInline('[https://good.example/login](https://evil.example/login)');
  t('a URL as link text shows the real destination', textOf(nodes) === 'evil.example/login');
  t('…and is not a link inside a link', nodes[0].c.every((n) => n.t !== 'link'));
}

{
  // Found on review: scheme-less address text over a different destination.
  const nodes = parseInline('[mail.dartmouth.edu/login](https://evil.example/login)');
  t('host-shaped link text shows the real destination', textOf(nodes) === 'evil.example/login');
  const mail = parseInline('[dean@dartmouth.edu](mailto:attacker@evil.example)');
  t('an address as link text shows the real recipient', textOf(mail) === 'attacker@evil.example');
  t('ordinary words stay words', textOf(parseInline('[the programme](https://uct.example.org/p)')) === 'the programme');
}

// ---- images are never fetched ----
{
  const nodes = parseInline('[![](https://st2.zoom.us/static/logo.png)](https://zoom.us/)');
  t('a linked image is a link', nodes.length === 1 && nodes[0].t === 'link');
  t('…whose content is an img placeholder, no src', nodes[0].c[0].t === 'img' && !('src' in nodes[0].c[0]) && !('href' in nodes[0].c[0]));
}

// ---- emphasis, escapes, breaks ----
{
  const nodes = parseInline('**Abstract:**  Episodic memories');
  t('strong parses', nodes[0].t === 'strong' && textOf(nodes[0].c) === 'Abstract:');
  t('…and the rest is text', textOf(nodes) === 'Abstract:  Episodic memories');
}
t('escaped asterisks are literal', textOf(parseInline('\\*Please send agenda items')) === '*Please send agenda items');
t('snake_case is not emphasis', textOf(parseInline('see mail_get_thread and mail_reply')) === 'see mail_get_thread and mail_reply');
t('an unmatched star is text', textOf(parseInline('5 * 3 = 15')) === '5 * 3 = 15');
t('em parses', parseInline('*Sent on behalf of Lily*')[0].t === 'em');
t('a bare URL is a link', parseInline('see https://example.org/x.').some((n) => n.t === 'link' && n.href === 'https://example.org/x'));
t('an angle autolink is a link', parseInline('<https://example.org>')[0]?.t === 'link');

// ---- blocks ----
{
  const body = '\\\n\n*Sent on behalf of Lily*\n\nDear all,\n\nLine one\\\nLine two\n\n- first\n- second\n\n> quoted\n> more\n\n---\n\nSee you there!';
  const blocks = parseBlocks(body);
  t('a lone backslash line vanishes', blocks[0].type === 'p' && blocks[0].inline[0].t === 'em');
  const para = blocks.find((b) => b.type === 'p' && textOf(b.inline).startsWith('Line one'));
  t('a trailing backslash is a break, not a character', !!para && textOf(para.inline) === 'Line one\nLine two');
  t('a bullet list parses', blocks.some((b) => b.type === 'ul' && b.items.length === 2));
  t('a quote parses', blocks.some((b) => b.type === 'quote' && b.blocks.length === 1));
  t('a rule parses', blocks.some((b) => b.type === 'hr'));
}
{
  const blocks = parseBlocks('**PBS Faculty Meeting**\n\n**Date:**  Wednesday, 09/30/26\n\n1. one\n2. two');
  t('an ordered list parses', blocks.some((b) => b.type === 'ol' && b.items.length === 2 && b.start === 1));
  t('linksOf finds nothing where there are none', linksOf(blocks).length === 0);
}
{
  const wrapped = 'Would you be willing to review the attached manuscript, "Spacing effects in\napplied retrieval practice", for the Journal of Applied Cognition? It runs to\nabout 9,000 words.';
  const [p] = parseBlocks(wrapped);
  t('a hard-wrapped paragraph reads as one', !p.inline.some((n) => n.t === 'br') && textOf(p.inline).includes('Spacing effects in applied'));
  const [sig] = parseBlocks('With thanks,\nTomas Lindqvist\nAssociate Editor');
  t('a signature keeps its lines', sig.inline.filter((n) => n.t === 'br').length === 2);
}
{
  // Found on review: an unclosed `[` scanned to the end of the body from
  // every `[`, quadratic in a stranger's text.
  const hostile = '['.repeat(60000) + '*a'.repeat(20000);
  const t0 = Date.now();
  parseBlocks(hostile);
  t('a hostile body parses in well under a second', Date.now() - t0 < 1000);
}
{
  // Found on review: a bare URL trailed by a long `)` run was trimmed one
  // character per full re-scan.
  const t0 = Date.now();
  const nodes = parseInline('see https://a.example/x' + ')'.repeat(60000));
  t('a URL with a hostile paren tail parses fast', Date.now() - t0 < 1000);
  t('…and the tail is not part of the link', nodes.some((n) => n.t === 'link' && n.href === 'https://a.example/x'));
  t('a URL keeps the paren it opened', parseInline('(see https://en.example.org/wiki/Foo_(bar))').some((n) => n.t === 'link' && n.href === 'https://en.example.org/wiki/Foo_(bar)'));
}
{
  const blocks = parseBlocks('Please bring the following items to the retreat on Saturday morning:\n- a laptop\n- a charger');
  t('a list right under a long line is still a list', blocks.length === 2 && blocks[1].type === 'ul' && blocks[1].items.length === 2);
}
t('empty text is no blocks', parseBlocks('').length === 0 && parseBlocks(null).length === 0);
{
  // Pathological nesting must not recurse without bound.
  const deep = '['.repeat(200) + 'x' + '](https://a.example)'.repeat(200);
  let ok = true;
  try { parseInline(deep); } catch { ok = false; }
  t('deep nesting terminates', ok);
}

console.log(`\n${pass} passed, ${fail} failed`);
if (fail) process.exit(1);
