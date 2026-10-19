// Declining an ask is a button beside Answer, not a caption under it.
//
// `npm test` in web/. The decline control used to be muted text below the
// input, read as a hint, and the owner typed "Decline" into the box instead —
// which reaches the model as an answer, never as the structured
// `{ decline: true }` (PR #722). Like `approval-why.mjs`, this reads the ask
// card's branch of the markup, since a template cannot be run as a function.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Chat.svelte'), 'utf8');

const open = "{:else if entry.kind === 'question'}";
const start = src.indexOf(open);
if (start < 0) throw new Error('Chat.svelte no longer has an ask-card branch');
// The branch nests its own {#if}/{#each}, so it ends where the entry loop does.
const end = src.indexOf('{#if streaming}', start);
if (end < 0) throw new Error('Chat.svelte: the ask-card branch has no end marker');
const branch = src.slice(start, end);

let failed = 0;
function check(name, ok) {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}`);
  if (!ok) failed += 1;
}

const decline = branch.indexOf('{ decline: true }');
check('the ask card can decline', decline >= 0);
const tag = branch.slice(branch.lastIndexOf('<button', decline), decline);
check('the decline control is a .qbtn, sized like Answer', /class="qbtn"/.test(tag));

const row = branch.lastIndexOf('<div class="qrow">', decline);
const rowEnd = branch.indexOf('</div>', decline);
const inRow = row >= 0 ? branch.slice(row, rowEnd) : '';
check(
  'in the same row as Answer',
  row >= 0 && !inRow.includes('</div>') && /\{ answer: [^}]*\}\)\}\s*>Answer</.test(inRow)
);
check('the free-text box is not squeezed into that row', !inRow.includes('<input'));

if (failed) {
  console.error(`${failed} ask-card check(s) failed`);
  process.exit(1);
}
