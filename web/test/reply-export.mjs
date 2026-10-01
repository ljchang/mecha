// Copy and download for a chat reply, imported from the shipped module and
// driven with fake navigator / document / URL objects: no browser.
import assert from 'node:assert/strict';
import { replyFilename, copyText, downloadText } from '../src/lib/reply-export.js';

// Who said it and when, in the page's clock; nothing a file system chokes on.
const at = new Date(2026, 9, 1, 16, 12);
assert.equal(replyFilename('mecha', at), 'mecha-2026-10-01-1612.md');
assert.equal(replyFilename('Devils_Advocate', at), 'devils-advocate-2026-10-01-1612.md');
assert.equal(replyFilename('Zoë / “the” owner?', at), 'zoe-the-owner-2026-10-01-1612.md');
assert.equal(replyFilename('', at), 'reply-2026-10-01-1612.md');
assert.equal(replyFilename(null, at), 'reply-2026-10-01-1612.md');
assert.match(replyFilename('x'.repeat(200), at), /^x{40}-2026/);
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

console.log('reply-export: ok');
