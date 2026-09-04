// Behaviour checks for what a refused call does to the chip above it.
//
// `npm test` in web/. Same rig and same reason as `queue-logic.mjs`.
//
// **Why it needs a test at all.** Three of the four denial paths in
// `Agent::run_tools` — the trifecta interlock, a `pre_tool` hook deny, and the
// approver — emit `AgentEvent::ToolDenied` and write the tool-result block
// straight into `results[i]`, with no `AgentEvent::ToolResult` behind it. Only
// the planning-phase refusal emits both. So the page is the only thing that
// can close the chip its `tool` event opened, and when it pushed a second
// entry instead, the first stayed `pending` for the rest of the session.
//
// That was an inert row until the chip carried the call. Once it did, the row
// became a working disclosure that opened onto "still running" — the harness
// rendering its own guard's refusal as work in flight, which is the failure
// the interlock's own comments keep naming. Found on review.
//
// The function is read OUT of the component rather than copied here, so this
// exercises the text that ships.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Chat.svelte'), 'utf8');

const marker = '  function resolveDenial(entries, ev) {';
const start = src.indexOf(marker);
if (start < 0) throw new Error('Chat.svelte no longer defines resolveDenial');
const end = '\n  }\n';
const fnSrc = src.slice(start, src.indexOf(end, start) + end.length);
const resolveDenial = new Function(`${fnSrc} return resolveDenial;`)();

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

const call = (name) => ({ kind: 'tool', name, pending: true, draft: null, args: '{}' });

// The ordinary case: `Agent::run_tools` emits the call, then refuses it in the
// same iteration, so the chip to close is the last pending one of that name.
{
  const entries = [{ kind: 'user', text: 'send it' }, call('mail__mail_send')];
  const resolved = resolveDenial(entries, {
    name: 'mail__mail_send',
    reason: 'blocked outbound call: trifecta armed',
  });
  is(resolved, true, 'a refusal closes the call it refused');
  is(entries.length, 2, 'and adds no second chip for the same call');
  is(entries[1].pending, false, 'the row stops claiming it is running');
  is(entries[1].blocked, true, 'and reads as blocked');
  is(
    entries[1].preview,
    'blocked outbound call: trifecta armed',
    'the reason the guard gave is what the row shows'
  );
}

// Concurrent calls: only the named one closes. Execution is concurrent here,
// so closing "the last pending chip" regardless of name would attribute one
// tool's refusal to another tool's row.
{
  const entries = [call('fs_read'), call('shell')];
  resolveDenial(entries, { name: 'shell', reason: 'Denied by the user: no' });
  is(entries[0].pending, true, 'an unrelated call in flight is left alone');
  is(entries[1].pending, false, 'and the refused one is closed');
}

// Two calls of one name: the refusal belongs to the most recent.
{
  const entries = [call('fs_write'), call('fs_write')];
  resolveDenial(entries, { name: 'fs_write', reason: 'Blocked by a hook: no writes' });
  is(entries[0].pending, true, 'the earlier call of the same name keeps running');
  is(entries[1].pending, false, 'the later one takes the refusal');
}

// A refusal with no call above it is still a refusal. The caller pushes a
// standalone chip on `false`; dropping it would be the quietest failure here.
{
  const entries = [{ kind: 'user', text: 'hi' }];
  is(
    resolveDenial(entries, { name: 'shell', reason: 'nope' }),
    false,
    'a refusal with no open call reports that it closed nothing'
  );
  is(entries.length, 1, 'and does not invent a row to close');
}

// An already-closed call is not reopened by a later refusal of the same name.
{
  const entries = [{ ...call('fs_read'), pending: false, preview: '42 lines' }];
  is(
    resolveDenial(entries, { name: 'fs_read', reason: 'nope' }),
    false,
    'a finished call is not retroactively refused'
  );
  is(entries[0].preview, '42 lines', 'and keeps what it answered');
}

// A reason the wire did not carry renders as empty, never as `undefined`.
{
  const entries = [call('shell')];
  resolveDenial(entries, { name: 'shell' });
  is(entries[0].preview, '', 'a missing reason is empty, not the string "undefined"');
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
