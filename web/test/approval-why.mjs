// The approval card shows why a person is being asked.
//
// `npm test` in web/. `WebApprover::ask` sends an escalation's reason — the
// interlock's `ask`, a `request_access` ("for the rest of this
// conversation", "will be refused"), a `prompt` rule — as the card's
// `question`, and the card used to drop it: the one sentence that says what a
// yes means reached the TUI and never the web (PR #711). A template cannot be
// read out and run the way `denied-chip.mjs` runs a function, so this reads
// the approval branch of the markup and checks it renders the field.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Chat.svelte'), 'utf8');

const open = "{:else if entry.kind === 'question' && entry.qkind === 'approval'}";
const start = src.indexOf(open);
if (start < 0) throw new Error('Chat.svelte no longer has an approval-card branch');
const next = src.indexOf('{:else', start + open.length);
const branch = src.slice(start, next);

let failed = 0;
function check(name, ok) {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}`);
  if (!ok) failed += 1;
}

check('the approval card renders the question it was sent', branch.includes('{entry.question}'));
check(
  'only when there is one — a plain approval carries none',
  /\{#if entry\.question\}[\s\S]*\{entry\.question\}/.test(branch)
);
// Present first, or a missing field (-1) reads as "before" everything.
const why = branch.indexOf('{entry.question}');
check(
  'above the call, so the reason is read before the arguments',
  why >= 0 && why < branch.indexOf('{entry.args}')
);

if (failed) {
  console.error(`${failed} approval-card check(s) failed`);
  process.exit(1);
}
