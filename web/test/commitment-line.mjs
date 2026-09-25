// The Today page's commitment line, dated and undated.
//
// `npm test` in web/. Plain node, like the rest of this rig. An undated
// commitment (1f-2, ruling (b)) states no deadline: the line must say so in
// words, and must not render "Invalid Date" or a 1970 date — the page used
// `new Date(item.commitment.due_at)` unconditionally before.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { commitmentLine } from '../src/lib/commitment.js';

const iso = (at) => at;

assert.equal(commitmentLine(null), '');
assert.equal(commitmentLine(undefined), '');
assert.equal(
  commitmentLine({ party: 'the reading group', due_at: '2026-10-15T17:00:00Z' }, iso),
  ' · due 2026-10-15T17:00:00Z · the reading group',
);
const undated = commitmentLine({ party: 'the reading group', source: 'task:agenda' });
assert.equal(undated, ' · no deadline stated · the reading group');
assert.doesNotMatch(undated, /Invalid Date|1970/);

// The page reads the line through the helper, not its own `new Date` of a
// field that may be absent.
const here = path.dirname(fileURLToPath(import.meta.url));
const page = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Today.svelte'), 'utf8');
assert.match(page, /commitmentLine\(item\.commitment\)/);
assert.doesNotMatch(page, /new Date\(item\.commitment\.due_at\)/);

console.log('commitment-line: ok');
