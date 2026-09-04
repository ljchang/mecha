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

function readOut(marker) {
  const start = src.indexOf(marker);
  if (start < 0) throw new Error(`Chat.svelte no longer defines ${marker.trim()}`);
  const end = '\n  }\n';
  return src.slice(start, src.indexOf(end, start) + end.length);
}

const openCallSrc = readOut('  function openCall(entries, ev) {');
const denialSrc = readOut('  function resolveDenial(entries, ev) {');
const fillSrc = readOut('  function fillRefusal(entries, ev) {');
const [openCall, resolveDenial, fillRefusal] = new Function(
  `${openCallSrc}${denialSrc}${fillSrc} return [openCall, resolveDenial, fillRefusal];`
)();

// The two strings the planning-phase path really emits. Written out because
// a test that invents its own refusal text asserts its own invention back:
// the previous version of the case below fed `resolveDenial` the sentence and
// then checked the sentence came out, so it passed just as happily while the
// row showed the two-word label instead. Keep these equal to
// `mecha-core/src/agent.rs` — the label on `ToolDenied`, the sentence on the
// `ToolResult` it emits beside it.
const PLANNING_LABEL = 'planning phase';
const PLANNING_SENTENCE =
  '`fs_write` is not available while planning. Work out what to do and say so; ' +
  'leave the phase to carry it out.';

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

const call = (name, id = null) => ({
  kind: 'tool',
  name,
  id,
  pending: true,
  draft: null,
  args: '{}',
});

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

// Two concurrent calls of one name: results come back in CALL order, because
// `join_all` preserves it, so pairing by name alone matched them in reverse
// and both rows closed showing the other one's output under their own
// arguments. This is the PR's own motivating turn — two `fs_write`s — so it
// is the case the change would have been most confidently wrong about.
{
  const entries = [call('fs_write', 't1'), call('fs_write', 't2')];
  const first = openCall(entries, { id: 't1', name: 'fs_write' });
  is(first === entries[0], true, "a.txt's result finds a.txt, not the later row");
  const second = openCall(entries, { id: 't2', name: 'fs_write' });
  is(second === entries[1], true, "and b.txt's finds b.txt");
}

// An id that matches nothing means the row is closed — the result is dropped,
// never moved onto another row of the same name. This is the case a name
// fallback on an unmatched id would get wrong: the refused write is closed,
// and its result must not land on the write still running.
{
  const entries = [
    { ...call('fs_write', 't1'), pending: false, blocked: true, preview: 'refused' },
    call('fs_write', 't2'),
  ];
  is(
    openCall(entries, { id: 't1', name: 'fs_write' }),
    undefined,
    'a closed row is not reopened, and its twin is not borrowed'
  );
  is(entries[1].pending, true, 'the other write is still running');
}

// A `web/dist` older than the binary serving it sends no id; the name path is
// what it falls back to, which is what this page did before ids existed.
{
  const entries = [call('shell')];
  is(
    openCall(entries, { name: 'shell' }) === entries[0],
    true,
    'an event with no id still finds its call by name'
  );
}

// The turn order `Agent::run_tools` actually produces, driven through both
// handlers together — the interaction neither one shows on its own.
//
// The approval loop is sequential and emits every `ToolCall` before any
// approved call runs (`join_all` comes after), so a planning-phase turn that
// allows `fs_read` and refuses `fs_write` has two chips open at once. The
// planning refusal is the one denial path that emits `ToolDenied` *and*
// `ToolResult`, and that result arrives while `fs_read` is still pending.
// Resolving by position put `fs_write`'s refusal under `fs_read`'s arguments
// and dropped the real read result.
{
  const entries = [];
  // `tool` events, both calls, before either result.
  entries.push({ kind: 'tool', name: 'fs_read', id: 't1', pending: true, args: '{"path":"agent.rs"}' });
  entries.push({ kind: 'tool', name: 'fs_write', id: 't2', pending: true, args: '{"path":"out.txt"}' });

  // The refusal, inline in the same loop. `ToolDenied` carries the label.
  resolveDenial(entries, { name: 'fs_write', reason: PLANNING_LABEL });
  is(entries[1].preview, PLANNING_LABEL, 'the label is what closes the row');

  // The refusal's own result, which the planning path emits too — carrying
  // the sentence, which is the half worth reading.
  const strayEvent = { id: 't2', name: 'fs_write', preview: PLANNING_SENTENCE };
  const strayTarget = openCall(entries, strayEvent);
  is(strayTarget, undefined, "the refused call's result finds no open chip to close");
  is(fillRefusal(entries, strayEvent), true, 'and fills in the row it belongs to');
  is(
    entries[1].preview,
    PLANNING_SENTENCE,
    'so the row says what to do about it, not just that it happened'
  );

  // Then the approved call's result, after join_all.
  const readTarget = openCall(entries, { id: 't1', name: 'fs_read' });
  is(readTarget === entries[0], true, 'the read result lands on the read');
  readTarget.pending = false;
  readTarget.preview = '80 lines';

  is(entries.length, 2, 'the turn drew one chip per call');
  is(
    entries[0].preview,
    '80 lines',
    'and the read shows what it read, not the write refusal'
  );
  is(
    entries[1].preview,
    PLANNING_SENTENCE,
    'while the refused write shows why it was refused'
  );
}

// A result may only ever fill in the row its own call opened.
{
  const entries = [
    { kind: 'tool', name: 'fs_write', id: 't1', pending: false, blocked: true, preview: 'planning phase' },
    { kind: 'tool', name: 'fs_write', id: 't2', pending: false, blocked: true, preview: 'planning phase' },
  ];
  fillRefusal(entries, { id: 't2', name: 'fs_write', preview: 'the sentence for t2' });
  is(entries[0].preview, 'planning phase', "another call's refused row is untouched");
  is(entries[1].preview, 'the sentence for t2', 'and its own row is filled');
}

// The three denial paths that emit no result leave the label standing —
// there is no better string coming, and inventing one would be worse.
{
  const entries = [{ ...call('mail__mail_send', 't1'), pending: false, blocked: true, preview: 'blocked outbound call: trifecta armed' }];
  is(
    fillRefusal(entries, { id: 't9', name: 'mail__mail_send', preview: 'unrelated' }),
    false,
    'a result for a different call fills nothing'
  );
  is(
    fillRefusal(entries, { name: 'mail__mail_send', preview: 'no id' }),
    false,
    'and an event with no id cannot claim a closed row'
  );
  is(
    entries[0].preview,
    'blocked outbound call: trifecta armed',
    "the interlock's own words survive"
  );
}

console.log(`\n${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
