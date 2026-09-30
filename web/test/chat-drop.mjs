// Behaviour checks for dropping files onto the chat.
//
// `npm test` in web/. The rules live in `attach.js`, which both chats
// import, so this exercises the text that ships in each.
//
// **Why it needs a test.** Two things a browser will not tell you by
// failing loudly. A drag of selected text or a link carries no files, and
// claiming it would swallow every in-page drag behind an overlay. And a
// dropped folder arrives as a `File` like any other — it has to be told
// apart by its entry, or it goes up as an empty upload the server refuses
// with nothing on screen saying why.
import { carriesFiles, droppedFiles, withAttachments } from '../src/lib/attach.js';

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
    console.log(`  FAIL  ${what}\n        expected ${b}`);
    console.log(`        got      ${a}`);
  }
}

const file = (name) => ({ name });
const item = (name, { dir = false, kind = 'file', entry = true } = {}) => ({
  kind,
  getAsFile: () => (kind === 'file' ? file(name) : null),
  ...(entry ? { webkitGetAsEntry: () => ({ isDirectory: dir }) } : {}),
});
const names = ({ files, folders }) => ({ files: files.map((f) => f.name), folders });

console.log('carriesFiles');
is(carriesFiles({ types: ['Files'] }), true, 'a file drag from the desktop is claimed');
// Chrome and Firefox hand `types` over as a frozen array, old Safari as a
// DOMStringList — iterable, but with no `.includes`.
is(carriesFiles({ types: new Set(['text/uri-list', 'Files']) }), true, 'any iterable of types is read');
is(carriesFiles({ types: ['text/plain', 'text/html'] }), false, 'selected text is not claimed');
is(carriesFiles({ types: ['text/uri-list'] }), false, 'a dragged link or image is not claimed');
is(carriesFiles(null), false, 'an event with no dataTransfer is not claimed');

console.log('droppedFiles');
is(
  names(droppedFiles({ items: [item('a.pdf'), item('b.png')] })),
  { files: ['a.pdf', 'b.png'], folders: [] },
  'every dropped file goes up, in order'
);
is(
  names(droppedFiles({ items: [item('notes.md'), item('project', { dir: true })] })),
  { files: ['notes.md'], folders: ['project'] },
  'a folder is split out by its entry, and its siblings still go up'
);
is(
  names(droppedFiles({ items: [item('x', { kind: 'string' }), item('c.txt')] })),
  { files: ['c.txt'], folders: [] },
  'a string item riding along with the files is ignored'
);
is(
  names(droppedFiles({ items: [item('d.csv', { entry: false })] })),
  { files: ['d.csv'], folders: [] },
  'a browser with no entry API still uploads the file'
);
is(
  names(droppedFiles({ items: [], files: [file('e.txt')] })),
  { files: ['e.txt'], folders: [] },
  'with no items list, the plain files list is used'
);
is(names(droppedFiles(null)), { files: [], folders: [] }, 'nothing dropped is nothing uploaded');

// The message a turn with attachments sends: words first, then each path.
is(withAttachments('look', []), 'look', 'no attachments leave the words alone');
is(
  withAttachments('look', ['inbox/a.png', 'inbox/b.pdf']),
  'look\n\nAttached file at inbox/a.png\nAttached file at inbox/b.pdf',
  'each attachment is named on its own line after the words',
);
is(withAttachments('', ['inbox/a.png']), 'Attached file at inbox/a.png', 'a file alone is a message');

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
