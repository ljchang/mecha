// Copy and download for a chat reply, imported from the shipped module and
// driven with fake navigator / document / URL objects: no browser.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { replyFilename, copyText, downloadText } from '../src/lib/reply-export.js';

// Who said it and when, in the page's clock; nothing a file system chokes on.
const at = new Date(2026, 9, 1, 16, 12);
assert.equal(replyFilename('mecha', at), 'mecha-2026-10-01-1612.md');
assert.equal(replyFilename('Devils_Advocate', at), 'devils-advocate-2026-10-01-1612.md');
assert.equal(replyFilename('Zoë / “the” owner?', at), 'zoe-the-owner-2026-10-01-1612.md');
assert.equal(replyFilename('', at), 'reply-2026-10-01-1612.md');
assert.equal(replyFilename(null, at), 'reply-2026-10-01-1612.md');
assert.match(replyFilename('x'.repeat(200), at), /^x{40}-2026/);
// A cut that lands on a hyphen leaves none behind.
assert.equal(replyFilename(`${'a'.repeat(39)} bb`, at), `${'a'.repeat(39)}-2026-10-01-1612.md`);
assert.equal(replyFilename('Mara Okonkwo', at), 'mara-okonkwo-2026-10-01-1612.md');
assert.match(replyFilename('mara', new Date('nonsense')), /^mara-\d{4}-\d{2}-\d{2}-\d{4}\.md$/);

// A fake page: a body that keeps what is appended, and elements that record
// what was done to them.
function page({ exec = true } = {}) {
  const made = [];
  const body = {
    kids: [],
    appendChild(el) { this.kids.push(el); el.parent = this; },
  };
  const doc = {
    body,
    createElement(tag) {
      const el = {
        tag, style: {}, attrs: {}, clicked: 0, selected: false,
        setAttribute(k, v) { this.attrs[k] = v; },
        select() { this.selected = true; },
        click() { this.clicked++; },
        remove() { body.kids = body.kids.filter((k) => k !== this); this.removed = true; },
      };
      made.push(el);
      return el;
    },
    execCommand(cmd) {
      if (cmd !== 'copy') return false;
      const area = body.kids.find((k) => k.tag === 'textarea');
      doc.copied = area?.selected ? area.value : null;
      return exec;
    },
  };
  return { doc, made };
}

const REPLY = '# Plan\n\n- **one** [p. 2: "a quote"]\n\n```\ncode\n```\n';

// The clipboard API, when the page has it: the reply as written, verbatim.
{
  let wrote = null;
  const nav = { clipboard: { writeText: async (t) => { wrote = t; } } };
  assert.equal(await copyText(REPLY, nav, page().doc), true);
  assert.equal(wrote, REPLY);
}

// Refused (an insecure page, a denied permission): the selection route, and
// the textarea it used is gone afterwards.
{
  const nav = { clipboard: { writeText: async () => { throw new Error('NotAllowedError'); } } };
  const { doc, made } = page();
  assert.equal(await copyText(REPLY, nav, doc), true);
  assert.equal(doc.copied, REPLY);
  assert.ok(made[0].removed && doc.body.kids.length === 0);
}

// No clipboard and a browser that will not copy: said, never a silent yes.
{
  const { doc } = page({ exec: false });
  assert.equal(await copyText(REPLY, {}, doc), false);
  assert.equal(await copyText(REPLY, undefined, undefined), false);
}

// A download is a blob made here, clicked once, then let go — the reply
// never goes back to the server.
{
  const { doc, made } = page();
  const urls = { made: [], revoked: [] };
  const url = {
    createObjectURL(b) { urls.made.push(b); return 'blob:reply-1'; },
    revokeObjectURL(u) { urls.revoked.push(u); },
  };
  downloadText('mara-2026-10-01-1612.md', REPLY, doc, url);
  const a = made[0];
  assert.equal(a.tag, 'a');
  assert.equal(a.href, 'blob:reply-1');
  assert.equal(a.download, 'mara-2026-10-01-1612.md');
  assert.equal(a.clicked, 1);
  assert.ok(a.removed);
  assert.equal(urls.made[0].type, 'text/markdown;charset=utf-8');
  assert.equal(await urls.made[0].text(), REPLY);
  await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(urls.revoked, ['blob:reply-1']);
}

// The call sites, read out of the components that ship: a finished reply
// has its actions, a streaming one has none, and an incognito chat keeps
// Copy but never Download — a file on the device outlives the room
// (INCOGNITO-DESIGN R2; review of #484).
{
  const here = path.dirname(fileURLToPath(import.meta.url));
  const read = (f) => fs.readFileSync(path.join(here, '..', 'src', 'lib', f), 'utf8');
  const calls = (src) => [...src.matchAll(/<ChatProse\b[^>]*\/>/g)].map((m) => m[0]);
  const chat = calls(read('Chat.svelte'));
  const personas = calls(read('Personas.svelte'));
  // Every site is counted, so a rename cannot make a check vacuous: one
  // finished and one streaming reply on each page.
  const finished = (sites) => sites.filter((c) => c.includes('text={entry.text}'));
  const streaming = (sites) => sites.filter((c) => /text=\{(streaming|run\.streaming)\}/.test(c));
  for (const [page, sites] of [['Chat', chat], ['Personas', personas]]) {
    assert.equal(finished(sites).length, 1, `${page}: ${sites.join('\n')}`);
    assert.equal(streaming(sites).length, 1, `${page}: ${sites.join('\n')}`);
    assert.match(finished(sites)[0], /actions=/, page);
    assert.ok(!/actions|download/.test(streaming(sites)[0]), `${page}: a streaming reply has no actions`);
  }
  assert.match(finished(chat)[0], /actions="mecha"/);
  // Download only where the chat is not incognito: tied to the flag, not a
  // bare `download`.
  assert.match(finished(chat)[0], /download=\{[^}]*incognito/);
  // ChatProse itself defaults Download off, so a new surface fails closed.
  assert.match(read('ChatProse.svelte'), /download = false \} = \$props\(\)/);
}

console.log('reply-export: ok');
