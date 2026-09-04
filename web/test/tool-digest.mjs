// Behaviour checks for the tool chip's one-line digest.
//
// `npm test` in web/. Plain node, no framework — the same rig and the same
// reason as `queue-logic.mjs`: this is the one piece of the chip that is real
// logic rather than a shape, and a source-string assertion cannot tell whether
// it picks the right argument.
//
// **Why it needs a test at all.** `DraftView::of` fills `other` from a
// `serde_json::Map`, which is a `BTreeMap` (no `preserve_order` in the tree),
// so `other` is sorted by key. Every case below where the digest is right and
// the sort order is wrong — `fs_read`, `web_search`, `fs_edit` — was a chip
// reading `50` where it should read the filename, found on review of the
// change that introduced it. The failure is silent by construction: a label
// that is confidently wrong looks exactly like one that is right.
//
// The function is read OUT of the component rather than copied here, so this
// exercises the text that ships.
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

function readConst(name) {
  const marker = `  const ${name} = `;
  const start = src.indexOf(marker);
  if (start < 0) throw new Error(`Chat.svelte no longer defines ${name}`);
  return src.slice(start, src.indexOf('\n', start) + 1);
}

const toolDigest = new Function(
  `${readConst('DIGEST_FIELDS')}
   ${readConst('QUANTITY')}
   ${readOut('  function oneLine(text) {')}
   ${readOut('  function toolDigest(draft) {')}
   return toolDigest;`
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

// `other` is written sorted, as the wire delivers it — writing these in
// schema order would test a world this page does not live in.
const draft = (headers, other, body = null) => ({ headers, body, other });

// The real schemas, each with its arguments in BTreeMap order.
is(
  toolDigest(draft([], [['limit', '50'], ['offset', '1'], ['path', 'agent.rs']])),
  'agent.rs',
  'fs_read names the file, not the limit that sorts before it'
);
is(
  toolDigest(draft([], [['limit', '8'], ['query', 'rust flexbox min-height']])),
  'rust flexbox min-height',
  'web_search names the query, not the limit'
);
is(
  toolDigest(draft([], [['command', 'cargo test'], ['cwd', '/tmp']])),
  'cargo test',
  'shell names the command'
);
is(
  toolDigest(draft([], [['new', 'replacement text'], ['old', 'original text'], ['path', 'agent.rs']])),
  'agent.rs',
  'fs_edit names the file, not the replacement text'
);
// `content` is a BODY_FIELDS key, so the server lifts it out of `other`.
is(
  toolDigest(draft([], [['path', 'policy.toml']], 'days = 3')),
  'policy.toml',
  'fs_write names the path it writes'
);

// Addressing wins outright: DraftView already put `headers` in reading order.
is(
  toolDigest(draft([['to', 'tomas@example.org'], ['subject', 'Re: review']], [['account', 'personal']])),
  'tomas@example.org',
  'a mail call leads with who it is addressed to'
);

// A tool nobody anticipated still gets a label rather than a quantity.
is(
  toolDigest(draft([], [['depth', '2'], ['entity', 'Ostrander']])),
  'Ostrander',
  'an unknown call skips the bare number and names the thing'
);
is(
  toolDigest(draft([], [['a', '1'], ['b', '2']])),
  '1',
  'all quantities falls back to the first argument rather than to nothing'
);
is(toolDigest(draft([], [])), '', 'a call with no arguments has no digest');
is(toolDigest(null), '', 'a call with no shape at all has no digest');

// Long values are truncated on one line: the chip is one row, and a
// newline in a command would otherwise break the row it sits in.
const long = toolDigest(draft([], [['command', 'x'.repeat(200)]]));
is(long.length, 73, 'a long value is cut to 72 characters plus an ellipsis');
is(long.endsWith('…'), true, 'and the cut is marked');
is(
  toolDigest(draft([], [['command', 'one\n  two']])),
  'one two',
  'a multi-line value is flattened to one line'
);

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
