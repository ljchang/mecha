// Behaviour checks for the chat's inline picture under an `image_generate` or
// `image_view` row.
//
// `npm test` in web/. Plain node, same rig as `tool-digest.mjs`. The matcher
// lives in `picture.js`, which both chats import, so this exercises the text
// that ships in each.
//
// **Why it needs a test.** The page turns text into a URL it fetches. The
// text is the tool's own first line, but a preview is whatever came back, so
// the match has to be strict: only `image_generate` (`images/<plain name>.png`)
// or `image_view` (a workspace-relative path of plain segments, none starting
// with a dot, with an image extension), only a finished call — never a `..`,
// a leading `/`, or anything a different tool happened to print.
import { pictureOf as generatedImage, repeatedPictures, picturesSince } from '../src/lib/picture.js';

let passed = 0;
let failed = 0;

function is(actual, expected, what) {
  if (actual === expected) {
    passed++;
    console.log(`  ok    ${what}`);
  } else {
    failed++;
    console.log(`  FAIL  ${what}\n        expected ${JSON.stringify(expected)}`);
    console.log(`        got      ${JSON.stringify(actual)}`);
  }
}

const done = (preview, extra = {}) => ({
  kind: 'tool',
  name: 'image_generate',
  pending: false,
  is_error: false,
  preview,
  ...extra,
});
const ok = 'image: images/20260925-153000-7.png\nGenerated a 1024×1024 image in 43 s';

is(generatedImage(done(ok)), 'images/20260925-153000-7.png', 'a finished call shows its file');
is(generatedImage(done(ok, { pending: true })), null, 'a running call shows nothing yet');
is(generatedImage(done(ok, { is_error: true })), null, 'a failed call has no picture');
is(generatedImage(done(ok, { blocked: true })), null, 'nor does a refused one');
is(generatedImage(done(ok, { name: 'shell' })), null, 'another tool printing the same line is ignored');
is(generatedImage(done(undefined)), null, 'a result with no preview has no picture');
is(
  generatedImage(done('Generated…\nimage: images/a.png')),
  null,
  'only the first line counts'
);
for (const bad of [
  'image: images/../secrets.png',
  'image: images/sub/x.png',
  'image: /etc/passwd.png',
  'image: images/x.png.html',
  'image: images/x.png extra',
]) {
  is(generatedImage(done(bad)), null, `refuses ${bad}`);
}

// `image_view`: what the model looked at, wherever in the workspace it is.
const viewed = (preview, extra = {}) => done(preview, { name: 'image_view', ...extra });
for (const [line, path] of [
  ['image: images/20260925-153000-7.png', 'images/20260925-153000-7.png'],
  ['image: inbox/Screenshot 2026-09-27 at 10.02.jpg', 'inbox/Screenshot 2026-09-27 at 10.02.jpg'],
  ['image: uploads/a.WEBP', 'uploads/a.WEBP'],
]) {
  is(generatedImage(viewed(line)), path, `a look shows ${path}`);
}
is(generatedImage(viewed('image: images/a.png', { is_error: true })), null, 'a failed look has none');
for (const bad of [
  'image: ../secrets.png',
  'image: images/../../x.png',
  'image: .hidden/x.png',
  'image: /etc/passwd.png',
  'image: notes.txt',
  'image: images/x.png.html',
]) {
  is(generatedImage(viewed(bad)), null, `a look refuses ${bad}`);
}
is(generatedImage(done('x', { name: 'constructor' })), null, 'a prototype key is no tool');

// A picture drawn once is not drawn inline again: the `image_view` of the file
// `image_generate` just saved keeps its row, with the picture behind the tap.
const indices = (entries) => JSON.stringify([...repeatedPictures(entries)]);
is(indices([done(ok), viewed(ok)]), '[1]', 'a look at the picture just made is a repeat');
is(indices([viewed(ok), viewed(ok), done(ok)]), '[1,2]', 'the first to show it keeps it inline');
is(
  indices([done(ok), viewed('image: images/other.png')]),
  '[]',
  'a look at a different picture is not'
);
is(
  indices([done(ok, { is_error: true }), viewed(ok)]),
  '[]',
  'a failed call drew nothing, so the look after it is the first'
);
is(
  indices([{ kind: 'user', text: ok }, { kind: 'assistant', text: ok }, viewed(ok)]),
  '[]',
  'only tool rows draw pictures'
);

// A call shows the pictures made since it was placed (the call screen covers
// the chat, so a picture drawn only in the chat is out of sight until the
// owner hangs up). Keyed on the picture, so a transcript reloaded mid-call —
// every entry rebuilt, every index moved — shows the same ones.
const a = 'image: images/a.png';
const b = 'image: images/b.png';
const c = 'image: images/c.png';
const since = (entries, before) => JSON.stringify(picturesSince(entries, new Set(before)));
is(since([done(a), done(b), done(c)], ['images/a.png']), '["images/b.png","images/c.png"]', 'the pictures since the call began, oldest first');
is(since([done(a)], ['images/a.png']), '[]', 'none yet');
is(
  since([{ kind: 'notice', text: 'x' }, done(a), done(b)], ['images/a.png']),
  since([done(a), done(b)], ['images/a.png']),
  'a reload that moves every index shows the same pictures'
);
is(since([done(b), viewed(b)], []), '["images/b.png"]', 'a look at the picture just made is one picture');
is(since([done(b, { is_error: true }), done(b, { pending: true })], []), '[]', 'a failed or running call shows nothing');

// The call screen never opens a picture in a new tab or window: on a phone
// that sends the call page to the background, its microphone stream dies and
// the call drops (three of seven drops on 2026-10-04). It opens in place,
// with the chat's Download and Edit.
{
  const fs = await import('node:fs');
  const call = fs.readFileSync(new URL('../src/lib/PersonaCall.svelte', import.meta.url), 'utf8');
  const markup = call.slice(call.indexOf('</script>'));
  is(/target=["']_blank|window\.open\(/.test(markup), false, 'the call screen opens nothing outside its page');
  is(/onclick=\{download\}/.test(markup) && /onclick=\{edit\}/.test(markup), true, 'the in-call viewer offers Download and Edit');
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
