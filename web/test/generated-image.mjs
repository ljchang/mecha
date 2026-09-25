// Behaviour checks for the chat's inline picture under an `image_generate` row.
//
// `npm test` in web/. Plain node, same rig as `tool-digest.mjs`, and the
// function is read OUT of the component so this exercises the text that ships.
//
// **Why it needs a test.** The page turns text into a URL it fetches. The
// text is the tool's own first line, but a preview is whatever came back, so
// the match has to be strict: only `image_generate`, only a finished call,
// only `images/<plain name>.png` — never a path with a slash beyond that, a
// `..`, or anything a different tool happened to print.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Chat.svelte'), 'utf8');

function readOut(marker, end = '\n  }\n') {
  const start = src.indexOf(marker);
  if (start < 0) throw new Error(`Chat.svelte no longer defines ${marker.trim()}`);
  return src.slice(start, src.indexOf(end, start) + end.length);
}

const generatedImage = new Function(
  `${readOut('  function generatedImage(entry) {')}
   return generatedImage;`
)();

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

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
