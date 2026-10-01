// Download for a generated picture (owner request, 2026-10-01), imported from
// the shipped module and driven with a fake fetch and fake document: no
// browser, no server.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { pictureName, downloadPicture } from '../src/lib/picture.js';

assert.equal(pictureName('images/20261001-1612-a1b2.png'), '20261001-1612-a1b2.png');
assert.equal(pictureName('portrait.webp'), 'portrait.webp');
assert.equal(pictureName('images/'), 'images');
assert.equal(pictureName(''), 'picture.png');
assert.equal(pictureName(null), 'picture.png');

function page() {
  const made = [];
  const body = { kids: [], appendChild(el) { this.kids.push(el); } };
  const doc = {
    body,
    createElement(tag) {
      const el = { tag, clicked: 0, click() { this.clicked++; }, remove() { body.kids = body.kids.filter((k) => k !== this); } };
      made.push(el);
      return el;
    },
  };
  const urls = { made: [], revoked: [] };
  const url = {
    createObjectURL(b) { urls.made.push(b); return 'blob:picture-1'; },
    revokeObjectURL(u) { urls.revoked.push(u); },
  };
  return { doc, made, url, urls };
}

const PNG = new Blob([new Uint8Array([137, 80, 78, 71])], { type: 'image/png' });

// Read through the chat's own route, saved from a blob under its own name:
// the browser is never pointed at the URL, so it lands in no history.
{
  const { doc, made, url, urls } = page();
  const asked = [];
  const get = async (u) => { asked.push(u); return { ok: true, blob: async () => PNG }; };
  const why = await downloadPicture(get, '/api/chat/main/file?path=images%2Fa.png', 'images/a.png', doc, url);
  assert.equal(why, '');
  assert.deepEqual(asked, ['/api/chat/main/file?path=images%2Fa.png']);
  assert.equal(made[0].download, 'a.png');
  assert.equal(made[0].href, 'blob:picture-1', 'never the file route itself');
  assert.equal(made[0].clicked, 1);
  assert.equal(urls.made[0], PNG);
  await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(urls.revoked, ['blob:picture-1']);
}

// A refusal (a lapsed unlock, a picture since removed) or a network failure
// is said, and nothing is saved.
{
  const { doc, made, url } = page();
  const missing = async () => ({ ok: false, status: 404, text: async () => 'no such file\n' });
  assert.equal(await downloadPicture(missing, '/api/x', 'images/a.png', doc, url), 'no such file');
  const silent = async () => ({ ok: false, status: 500, text: async () => '' });
  assert.equal(await downloadPicture(silent, '/api/x', 'images/a.png', doc, url), 'HTTP 500');
  const offline = async () => { throw new Error('offline'); };
  assert.equal(await downloadPicture(offline, '/api/x', 'images/a.png', doc, url), 'offline');
  assert.equal(made.length, 0);
}

// Both chats offer it beside Edit, through this module.
{
  const here = path.dirname(fileURLToPath(import.meta.url));
  for (const file of ['Chat.svelte', 'Personas.svelte']) {
    const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', file), 'utf8');
    // Download follows Edit, whatever sits between them.
    const edit = src.indexOf('>Edit</button>');
    const download = src.indexOf('onclick={() => savePicture(picture)}>Download</button>');
    assert.ok(edit > 0 && download > edit, file);
    assert.match(src, /downloadPicture\(fetch, /, file);
    // `fetch` there is the same-origin, /api/-checked wrapper, not the global.
    assert.match(src, /import \{ apiFetch as fetch \} from '\.\/api\.js';/, file);
  }
}

console.log('picture-download: ok');
