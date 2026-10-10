// Regenerate's versions on the persona chat's picture card (IMAGE-DESIGN.md
// §5.4): a card whose `version_of` names a picture drawn higher up shows on
// that picture's card, ‹ k/n ›, and the version showing is the one Edit and
// Regenerate act on.
//
// `npm test` in web/. Plain node, as `generated-image.mjs`: the grouping lives
// in `picture.js`, so this exercises the text that ships.
import { pictureVersions, shownVersion } from '../src/lib/picture.js';

let passed = 0;
let failed = 0;
function is(actual, expected, what) {
  const a = JSON.stringify(actual);
  const b = JSON.stringify(expected);
  if (a === b) {
    passed++;
    console.log(`  ok    ${what}`);
  } else {
    failed++;
    console.log(`  FAIL  ${what}\n        expected ${b}\n        got      ${a}`);
  }
}

const drawn = (path, version_of) => ({
  kind: 'tool',
  name: 'image_generate',
  is_error: false,
  preview: `image: ${path}`,
  ...(version_of ? { version_of } : {}),
});
const said = (text) => ({ kind: 'assistant', text });
const groups = (v) => Object.fromEntries(v.groups);
const folded = (v) => Object.fromEntries(v.folded);

{
  const v = pictureVersions([drawn('images/a.png'), said('one'), drawn('images/b.png'), said('two')]);
  is(groups(v), { 'images/a.png': ['images/a.png'], 'images/b.png': ['images/b.png'] }, 'with no Regenerate every picture is its own card');
  is(folded(v), {}, 'and nothing folds');
}

{
  // a, then a drawn again (b), then the version showing drawn again (c).
  const v = pictureVersions([
    drawn('images/a.png'),
    said('one'),
    drawn('images/b.png', 'images/a.png'),
    said('two'),
    drawn('images/c.png', 'images/b.png'),
  ]);
  is(groups(v), { 'images/a.png': ['images/a.png', 'images/b.png', 'images/c.png'] }, "a Regenerate of a version joins the first picture's card");
  is(folded(v), { 2: { root: 'images/a.png', k: 2 }, 4: { root: 'images/a.png', k: 3 } }, 'each version folds, counted from 1');
}

{
  // The picture it was drawn from is not in this transcript (compacted away).
  const v = pictureVersions([drawn('images/b.png', 'images/gone.png')]);
  is(groups(v), { 'images/b.png': ['images/b.png'] }, 'a version of a picture not shown stands as its own card, never vanishes');
  is(folded(v), {}, 'and does not fold');
}

{
  // A refused Regenerate: an error card, with nothing to show or build on.
  const refused = { ...drawn('images/b.png', 'images/a.png'), is_error: true, preview: 'Nothing was drawn.' };
  const v = pictureVersions([drawn('images/a.png'), refused]);
  is(groups(v), { 'images/a.png': ['images/a.png'] }, 'a refused Regenerate adds no version');
}

{
  // The same picture again (an image_view of it): the repeat is not a version.
  const viewed = { kind: 'tool', name: 'image_view', is_error: false, preview: 'image: images/a.png' };
  const v = pictureVersions([drawn('images/a.png'), viewed]);
  is(folded(v), {}, 'a picture shown twice is a repeat, not a version');
}

{
  const list = ['images/a.png', 'images/b.png', 'images/c.png'];
  is(shownVersion(list, undefined), 'images/c.png', 'the newest shows until the owner steps');
  is(shownVersion(list, 'images/a.png'), 'images/a.png', 'the version stepped to shows');
  is(shownVersion(list, 'images/x.png'), 'images/c.png', 'a choice no longer among them falls to the newest');
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
