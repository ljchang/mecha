// The two-language pin on `~/.mecha/charter.toml`.
//
// The web settings page writes that file and `mecha_core::charter` reads it,
// and neither side proves the agreement alone. `charter.rs`'s
// `the_web_editors_serialisation_is_what_this_reader_loads` shows the reader
// accepts a sample; on its own that sample is a hand-copied expectation that
// stays green through any regression in `esc` or `serialize`. This script
// closes the loop from the other end: it reads that same literal out of
// `charter.rs` and asserts the serialiser actually emits it, byte for byte.
//
// So an edit to either half fails the other, which is the whole point — the
// expensive bugs in this project came from beliefs about the far side of a
// boundary.

import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { esc, hasComment, serialize, slugify, splitHeader } from '../../web/src/lib/charter-toml.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const fail = (msg) => {
  console.error(`check-charter-toml: ${msg}`);
  process.exit(1);
};
let checks = 0;
const eq = (got, want, what) => {
  checks++;
  if (got !== want) fail(`${what}\n  got:  ${JSON.stringify(got)}\n  want: ${JSON.stringify(want)}`);
};

// --- the shared fixture ---------------------------------------------------
const rs = readFileSync(join(root, 'mecha-core/src/charter.rs'), 'utf8');
const marked = /\/\/ web-editor-sample:begin[\s\S]*?r#"([\s\S]*?)"#;[\s\S]*?\/\/ web-editor-sample:end/.exec(rs);
if (!marked) {
  fail(
    'could not find the web-editor-sample markers in mecha-core/src/charter.rs.\n' +
      '  That literal is this check\'s expectation — if it moved, move this reader with it.'
  );
}
// A Rust raw string carries no escapes: `\"` in the file is a backslash and a
// quote, which is exactly what the TOML wants.
const want = marked[1];

const produced = serialize('# What mecha is for, most important first.\n#\n# Order is rank.', [
  { id: 'say-no-early', text: 'A refusal on Monday is a kindness.' },
  { id: 'quote-and-break', text: 'She said "no" early.\nAnd meant it.' },
]);
eq(produced, want, 'the serialiser no longer emits the document charter.rs reads back');

// --- escaping -------------------------------------------------------------
eq(esc('plain'), '"plain"', 'a plain string');
eq(esc('a "b" c'), '"a \\"b\\" c"', 'a quote must be escaped');
eq(esc('a\\b'), '"a\\\\b"', 'a backslash must be escaped');
eq(esc('one\ntwo'), '"one\\ntwo"', 'a newline must become an escape, never a raw break');
eq(esc('a\tb\rc'), '"a\\tb\\rc"', 'tab and carriage return');

// --- comments -------------------------------------------------------------
const yes = (row, what) => { checks++; if (!hasComment(row)) fail(`should read as a comment: ${what}`); };
const no = (row, what) => { checks++; if (hasComment(row)) fail(`should NOT read as a comment: ${what}`); };
yes('# a whole-line comment', 'whole line');
yes('   # indented', 'indented');
yes('text = "t"  # trailing note', 'trailing a value — the case a whole-line regex misses');
no('text = "use #hashtags freely"', 'a # inside a basic string');
no("text = 'a #literal string'", 'a # inside a literal string');
no('text = "escaped \\" then #not-a-comment"', 'a # after an escaped quote, still inside the string');
no('id = "a"', 'no comment at all');

// --- the header is the owner's writing ------------------------------------
const split = splitHeader('# keep me\n\n[[line]]\nid = "a"\ntext = "t"\n');
eq(split.header, '# keep me', 'everything above the first [[line]] is kept');
eq(split.blocked, null, 'a clean document is editable as a list');
checks++;
if (!splitHeader('# h\n[[line]]\nid = "a"\ntext = "t" # note\n').blocked) {
  fail('a comment among the lines must refuse the list editor, not be rewritten away');
}
eq(splitHeader('# comments only, no tables').header, '# comments only, no tables', 'a template-only document is all header');

// --- ids ------------------------------------------------------------------
eq(slugify('Say no early'), 'say-no-early', 'a slug from typed text');
eq(slugify('  Punctuation!! and — dashes  '), 'punctuation-and-dashes', 'punctuation collapses');
eq(slugify('one two three four five six seven'), 'one-two-three-four-five', 'capped at five words');

console.log(`check-charter-toml: ${checks} checks, the serialiser and mecha-core agree on charter.toml`);
