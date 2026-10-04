// The Personas tab's pure logic, imported from the shipped module.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  isPersonaKey, withUnlock, listUrl, personaUrl, chatUrl, relationshipLabel, emptyRun, applyEvent, ENDPOINTS, settle, keptEdits,
  taintLabel, doseLine, fileUnsaved, unsavedFiles, lockWaits, callTime, hangUpReport, personaName, authoringUrl, keptCharacter, OWNER_FILES, toolStatus, waitingLine, withWorking,
  fileUrl, uploadUrl,
} from '../src/lib/persona.js';
import { pictureOf } from '../src/lib/picture.js';

// Only a key the server could have minted is a persona chat's.
assert.ok(isPersonaKey('p-0123456789ab'));
for (const bad of ['main', 'chat-abc', 'p-', 'p-0123456789AB', 'p-0123456789abc', '../p-0123456789ab', null]) {
  assert.ok(!isPersonaKey(bad), String(bad));
}

// Every URL is the persona door's; an assistant key is refused outright, so
// this page can never read or write an assistant chat.
assert.equal(chatUrl('p-0123456789ab'), '/api/persona-chat/p-0123456789ab');
assert.equal(chatUrl('p-0123456789ab', '/events', 't k'), '/api/persona-chat/p-0123456789ab/events?unlock=t%20k');
assert.throws(() => chatUrl('main', '/send'));
assert.throws(() => chatUrl('chat-abcdef'));
assert.equal(listUrl(null), '/api/personas');
assert.equal(listUrl('abc'), '/api/personas?unlock=abc');
assert.equal(personaUrl('devils_advocate', '/chats', null), '/api/personas/devils_advocate/chats');
assert.equal(withUnlock('/x?a=1', 'z'), '/x?a=1&unlock=z');

// A builder reaches only what ENDPOINTS lists — the list check-demo holds
// the docs demo to — so a new endpoint cannot slip past that guard.
assert.throws(() => personaUrl('mara', '/delete', null));
assert.throws(() => chatUrl('p-0123456789ab', '/mode'));
assert.ok(ENDPOINTS.includes('/api/persona-chat/X/events'));
assert.equal(ENDPOINTS.length, 26);
// A proposal is read, approved and turned away through the persona door.
for (const s of ['review', 'approve', 'reject']) assert.ok(ENDPOINTS.includes(`/api/personas/X/${s}`), s);
assert.ok(ENDPOINTS.includes('/api/personas/X/frame'));
assert.ok(ENDPOINTS.includes('/api/personas/X/sources') && ENDPOINTS.includes('/api/personas/X/sources/remove'));
assert.ok(ENDPOINTS.includes('/api/personas/authoring') && ENDPOINTS.includes('/api/personas/X/files'));
assert.equal(authoringUrl('t'), '/api/personas/authoring?unlock=t');
assert.equal(personaUrl('mara', '/files', null), '/api/personas/mara/files');
// A name as the store holds it, or null for the obvious refusals.
assert.equal(personaName('  Mara '), 'mara');
assert.equal(personaName('devils_advocate'), 'devils_advocate');
for (const bad of ['', 'files', '-x', 'a b', 'a/b', 'x'.repeat(65)]) assert.equal(personaName(bad), null, bad);
assert.deepEqual(OWNER_FILES.map(([f]) => f), ['identity', 'motivation', 'settings']);

assert.equal(relationshipLabel({ relationship: ['colleague', 'devils_advocate'] }), 'colleague · devils advocate');
assert.equal(relationshipLabel({}), '');

// A run, folded event by event: the owner's turn, a tool, streamed text,
// then the end — the streamed text becomes the answer.
let s = emptyRun([{ kind: 'user', text: 'earlier' }]);
s = applyEvent(s, { type: 'user', text: 'Hello, Mara', spoken: false });
assert.equal(s.running, true);
s = applyEvent(s, { type: 'tool', id: 't1', name: 'image_view' });
s = applyEvent(s, { type: 'tool_result', id: 't1', name: 'image_view', is_error: false, preview: '' });
s = applyEvent(s, { type: 'delta', text: 'Hello ' });
s = applyEvent(s, { type: 'delta', text: 'back.' });
assert.equal(s.streaming, 'Hello back.');
s = applyEvent(s, { type: 'done', ok: true, stop: 'EndTurn' });
assert.equal(s.running, false);
assert.equal(s.streaming, null);
assert.deepEqual(
  s.entries.map((e) => [e.kind, e.text ?? e.name, e.is_error ?? null]),
  [
    ['user', 'earlier', null],
    ['user', 'Hello, Mara', null],
    ['tool', 'image_view', false],
    ['assistant', 'Hello back.', null],
  ],
);

// A steer resolves: delivered into the run, or discarded as too late —
// never left reading "queued" (review of #415).
s = emptyRun();
s = applyEvent(s, { type: 'queued', text: 'also this', request_id: 'r-1' });
s = applyEvent(s, { type: 'queued', text: 'and this', request_id: 'r-2' });
s = applyEvent(s, { type: 'queued_delivered', request_id: 'r-1' });
s = applyEvent(s, { type: 'queued_discarded', request_ids: ['r-2'] });
assert.deepEqual(s.entries.map((e) => e.delivery), ['delivered', 'discarded']);

// A re-read after the run keeps what only the page held: the server's
// entries, then the notice and the undelivered steer — not dropped.
{
  let r = emptyRun([{ kind: 'user', text: 'Hi' }]);
  r = applyEvent(r, { type: 'queued', text: 'late', request_id: 'r-9' });
  r = applyEvent(r, { type: 'notice', text: 'switching models' });
  r = applyEvent(r, { type: 'queued_discarded', request_ids: ['r-9'] });
  const server = [{ kind: 'user', text: 'Hi' }, { kind: 'assistant', text: 'Hello.' }];
  assert.deepEqual(
    settle(server, r).map((e) => [e.kind, e.text]),
    [['user', 'Hi'], ['assistant', 'Hello.'], ['user', 'late'], ['notice', 'switching models']],
  );
}

// A crisis pause is drawn as its own card and survives a re-read; the run
// ends, and what it touched is kept for the chip.
{
  let r = emptyRun([{ kind: 'user', text: 'hi' }]);
  r = applyEvent(r, { type: 'user', text: 'something hard' });
  r = applyEvent(r, { type: 'crisis', text: 'This is mecha, not the character… 988' });
  r = applyEvent(r, { type: 'done', ok: true, stop: 'CrisisPause', taint_private: true, taint_untrusted: false });
  assert.equal(r.running, false);
  assert.deepEqual(r.taint, { private: true, untrusted: false });
  assert.deepEqual(settle([{ kind: 'user', text: 'hi' }], r).map((e) => e.kind), ['user', 'crisis']);
}
assert.equal(taintLabel({ private: true, untrusted: true }), 'private + untrusted');
assert.equal(taintLabel({ private: false, untrusted: false }), '');
assert.equal(taintLabel(null), '');
// Two crisis cards get two ids, and a re-read keeps each card's own.
{
  let r = emptyRun();
  r = applyEvent(r, { type: 'crisis', text: 'a' });
  r = applyEvent(r, { type: 'crisis', text: 'b' });
  const ids = r.entries.map((e) => e.id);
  assert.equal(new Set(ids).size, 2);
  assert.deepEqual(settle([{ kind: 'user', text: 'x' }], r).filter((e) => e.kind === 'crisis').map((e) => e.id), ids);
}
assert.equal(doseLine({ turns_today: 3, turns_7d: 12, late_night_7d: 2 }), '3 today · 12 this week · 2 late at night');
assert.equal(doseLine({ turns_today: 0, turns_7d: 0, late_night_7d: 0 }), '0 today · 0 this week');
// Call time sits beside the turns, never in them; under a minute is said,
// never rounded to a zero that reads as no call (§11).
assert.equal(
  doseLine({ turns_today: 1, turns_7d: 4, late_night_7d: 0, call_secs_today: 30, call_secs_7d: 4000 }),
  '1 today · 4 this week · calls under a minute today, 1 h 7 min this week',
);
assert.equal(doseLine({ turns_today: 1, turns_7d: 1, late_night_7d: 0, call_secs_today: 0, call_secs_7d: 0 }), '1 today · 1 this week');
assert.equal(callTime(0), '0 min');
assert.equal(callTime(150), '3 min');
// A hang-up reports its seconds and binding; one that never connected still
// reports, so serve releases the binding; one with neither has nothing to say.
assert.deepEqual(hangUpReport({ since: 1000, callId: 7, now: 61_400 }), { seconds: 60, call: 7 });
assert.deepEqual(hangUpReport({ since: null, callId: 7, now: 5000 }), { seconds: 0, call: 7 });
assert.equal(hangUpReport({ since: null, callId: null, now: 5000 }), null);
assert.equal(doseLine(null), '');
assert.equal(doseLine({ unread: 'Permission denied' }), 'usage meters unreadable');
assert.equal(doseLine({ turns_today: 1, turns_7d: 2, late_night_7d: 0, skipped: 3 }), '1 today · 2 this week · 3 unreadable records not counted');

// A failed turn says so rather than ending silently.
s = applyEvent(emptyRun(), { type: 'done', ok: false, error: 'model unavailable' });
assert.deepEqual(s.entries, [{ kind: 'notice', text: 'model unavailable' }]);

// An event this page does not know changes nothing.
const before = emptyRun([{ kind: 'user', text: 'x' }]);
assert.equal(applyEvent(before, { type: 'affect', label: 'calm' }), before);

// An editor save keeps the other tabs' unsaved edits — text and form alike —
// and each tab's mode; the saved file's own drafts are the saved file now.
{
  const files = {
    identity: { text: 'a', draft: 'a2', formDraft: { title: 'x' }, asText: false },
    motivation: { text: 'm', draft: 'm', formDraft: { title: 'y' }, asText: true },
    settings: { text: 's', formDraft: { 'safety.dose': false } },
  };
  assert.deepEqual(keptEdits(files, 'identity'), {
    identity: { asText: false },
    motivation: { asText: true, formDraft: { title: 'y' } },
    settings: { asText: undefined, formDraft: { 'safety.dose': false } },
  });
}

// A relock drops a locked portrait the form had chosen; an unlock keeps
// whatever was chosen, since the list only grows.
assert.equal(keptCharacter('priya', ['john', 'priya']), 'priya');
assert.equal(keptCharacter('maya', ['john', 'priya']), '');
assert.equal(keptCharacter('', ['john']), '');
assert.equal(keptCharacter('priya', undefined), '');

// A slow run says what it is doing; a refused-then-retried call is not
// "failed" (owner, 2026-09-30). Timed on entries as a re-read builds them,
// with the server's start (`withWorking`), not the page's clock.
{
  let r = applyEvent(emptyRun(), { type: 'user', text: 'a picture?' });
  assert.equal(waitingLine(r, 'Maya', 0), 'Maya is typing');
  // The server says 84 s have passed; the page's own clock does the rest,
  // whatever the server's clock reads.
  r = { ...r, entries: withWorking(r.entries, { id: 't1', name: 'image_generate', since: '2099-01-01T00:00:00Z', elapsed_ms: 84_000 }, 1_000_000) };
  assert.equal(waitingLine(r, 'Maya', 1_000_000), 'drawing a picture… 1:24');
  assert.equal(waitingLine(r, 'Maya', 1_010_000), 'drawing a picture… 1:34');
  r = applyEvent(r, { type: 'tool_result', id: 't1', name: 'image_generate', is_error: true });
  r = applyEvent(r, { type: 'tool', id: 't2', name: 'image_generate' });
  assert.equal(toolStatus(r.entries, 1), 'retried');
  assert.equal(toolStatus(r.entries, 2), 'running');
  assert.ok(/^drawing a picture… \d+:\d\d$/.test(waitingLine(r, 'Maya', Date.now())));
  r = applyEvent(r, { type: 'tool_result', id: 't2', name: 'image_generate', is_error: false });
  assert.equal(toolStatus(r.entries, 2), 'done');
  assert.equal(waitingLine(r, 'Maya', 99_000), 'Maya is typing');
  // A refused call is closed, not waited on for the rest of the run.
  r = applyEvent(r, { type: 'tool', id: 't3', name: 'web_search' });
  r = applyEvent(r, { type: 'denied', name: 'web_search', reason: 'blocked' });
  assert.equal(waitingLine(r, 'Maya', 99_000), 'Maya is typing');
  assert.equal(toolStatus(r.entries, 3), 'failed');
  r = applyEvent(r, { type: 'delta', text: 'Here' });
  assert.equal(waitingLine(r, 'Maya', 99_000), null);
  r = applyEvent(r, { type: 'done', ok: true });
  assert.equal(waitingLine(r, 'Maya', 99_000), null);
  // A lone failure stays a failure — and a same-named call in a later turn
  // is not its retry.
  const lone = [{ kind: 'tool', name: 'web_search', is_error: true }];
  assert.equal(toolStatus(lone, 0), 'failed');
  const later = [
    { kind: 'tool', name: 'web_search', is_error: true },
    { kind: 'user', text: 'something else' },
    { kind: 'tool', name: 'web_search', is_error: false },
  ];
  assert.equal(toolStatus(later, 0), 'failed');
}

// A page re-read mid-run puts back the tool still running, from the
// server's own report of it — not a start the page invented.
{
  const working = { id: 't9', name: 'image_generate', since: '2026-09-30T04:00:00Z', elapsed_ms: 84_000 };
  const entries = withWorking([{ kind: 'user', text: 'a picture?' }], working, 5_000);
  const run = { ...emptyRun(entries), running: true };
  assert.equal(waitingLine(run, 'Maya', 5_000), 'drawing a picture… 1:24');
  // Not twice, and not when nothing is running.
  assert.equal(withWorking(entries, working).length, entries.length);
  assert.equal(withWorking(entries, null), entries);
}

// A picture the persona drew: fetched and edited through this chat's own
// door, the path encoded and the unlock riding last, as every URL here.
assert.equal(
  fileUrl('p-0123456789ab', 'images/a b.png'),
  '/api/persona-chat/p-0123456789ab/file?path=images%2Fa%20b.png',
);
assert.equal(
  fileUrl('p-0123456789ab', 'images/a.png', 't'),
  '/api/persona-chat/p-0123456789ab/file?path=images%2Fa.png&unlock=t',
);
assert.equal(
  uploadUrl('p-0123456789ab', 'mask-a-1.png', 't'),
  '/api/persona-chat/p-0123456789ab/upload?name=mask-a-1.png&unlock=t',
);
assert.throws(() => fileUrl('chat-abcdef', 'images/a.png'));
assert.throws(() => uploadUrl('main', 'mask.png'));

// The streamed result keeps its preview, so the picture shows as soon as the
// call ends — not only after a reload re-reads the transcript. A row still
// running has none.
{
  let r = emptyRun();
  r = applyEvent(r, { type: 'tool', id: 'g1', name: 'image_generate' });
  assert.equal(pictureOf(r.entries[0]), null);
  r = applyEvent(r, {
    type: 'tool_result', id: 'g1', name: 'image_generate', is_error: false,
    preview: 'image: images/20260930-120000-1.png\nGenerated a 1024×1024 image in 40 s',
  });
  assert.equal(pictureOf(r.entries[0]), 'images/20260930-120000-1.png');
}

// A turn that asked for pictures and got none says so under the reply,
// from the tool rows' own results; a drawn one, a running one, a turn with
// a call still out, or one that asked for none says nothing.
{
  const { turnsWithoutPicture } = await import('../src/lib/picture.js');
  const img = (is_error) => ({ kind: 'tool', name: 'image_generate', is_error });
  const entries = [
    { kind: 'user', text: 'draw yourself' },
    img(true), img(true),
    { kind: 'assistant', text: 'Here you go.' },
    { kind: 'user', text: 'again' },
    img(true), img(false),
    { kind: 'assistant', text: 'There.' },
    { kind: 'user', text: 'hello' },
    { kind: 'tool', name: 'document_read', is_error: true },
    { kind: 'assistant', text: 'hi' },
    { kind: 'user', text: 'one more' },
    img(true),
    { kind: 'user', text: 'and with the cat', queued: true },
    img(true),
    { kind: 'assistant', text: 'Done!' },
  ];
  assert.deepEqual([...turnsWithoutPicture(entries)], [3, 15]);
  assert.deepEqual([...turnsWithoutPicture(entries, true)], [3], 'a running turn is not judged');
  assert.deepEqual([...turnsWithoutPicture([{ kind: 'user', text: 'x' }, img(null)])], [], 'a call still out');
  // The same steered turn as a reload reads it: the steer after the tool
  // rows, marked `steered` by the server (`transcript_entries`).
  const reloaded = [
    { kind: 'user', text: 'one more' },
    img(true), img(true),
    { kind: 'user', text: 'and with the cat', steered: true },
    { kind: 'assistant', text: 'Done!' },
  ];
  assert.deepEqual([...turnsWithoutPicture(reloaded)], [4], 'the note stays under the reply');
  // A page-only notice carried after the server's entries is not the reply.
  const noticed = [...reloaded, { kind: 'notice', text: 'upload failed' }];
  assert.deepEqual([...turnsWithoutPicture(noticed)], [4], 'not under the notice');
}

// A file's row: size, whether a chat can read it without waiting, and where
// a shared one comes from.
{
  const { sourceLine } = await import('../src/lib/persona.js');
  assert.equal(sourceLine({ name: 'paper.pdf', bytes: 3870000, shared: false, ready: false, processing: true }), '3.7 MB · reading…');
  assert.equal(sourceLine({ name: 'notes.md', bytes: 300, shared: false, ready: true, processing: false }), '1 KB · ready');
  assert.equal(sourceLine({ name: '@kelp/s.pdf', bytes: 2048, shared: true, ready: false, processing: false }), '2 KB · not read yet · group kelp');
  assert.equal(sourceLine({ name: '@all/g.md', bytes: 2048, shared: true, ready: true, processing: false }), '2 KB · ready · every persona');
  assert.equal(sourceLine({ name: '@group:all/g.md', bytes: 2048, shared: true, ready: true, processing: false }), '2 KB · ready · group all');
  assert.equal(sourceLine({ name: 'paper.pdf', bytes: 2048, shared: false, ready: false, on_request: true, processing: false }), '2 KB · read when asked');
  assert.equal(sourceLine({ name: 'scan.heic', bytes: 2048, shared: false, ready: false, processing: false, unreadable: 'scan.heic: a HEIC/HEIF photo' }), '2 KB · not readable');
}

// A file's tile: its state alone, and a badge from its extension.
{
  const { sourceState, fileKind } = await import('../src/lib/persona.js');
  assert.equal(sourceState({ ready: true, processing: false }), 'ready');
  // Unreadable outranks a read in progress.
  assert.equal(sourceState({ unreadable: 'x', processing: true }), 'not readable');
  assert.equal(fileKind('paper.PDF'), 'PDF');
  assert.equal(fileKind('@kelp/photo.jpeg'), 'IMG');
  assert.equal(fileKind('notes.markdown'), 'MD');
  assert.equal(fileKind('plain.txt'), 'TXT');
  assert.equal(fileKind('README'), 'FILE');
  assert.equal(fileKind('dir.d/README'), 'FILE');
}

// A reply cut at its checked citations (§10.4). Each citation gets the
// check made of it: the same quote cited before its page was read and
// after reads "no such file" then "quoted" — never "quoted" twice (review
// of #465). Paired from the end, so a compacted chat whose first answer is
// gone still pairs its later ones right. "quoted", never "verified".
{
  const { citeSegments, citeEntries, citeNote, citeOpens, citedUrl, applyEvent, emptyRun } = await import('../src/lib/persona.js');
  const a = '[kelp.pdf, p. 1: "urchins graze kelp"]';
  const b = '[kelp.pdf, p. 2: "otters eat forty a day"]';
  const before = { raw: a, file: 'kelp.pdf', cited: 1, quote: 'urchins graze kelp', status: 'no_such_file' };
  const made = { raw: b, file: 'kelp.pdf', cited: 2, quote: 'otters eat forty a day', status: 'not_found' };
  const after = { raw: a, file: 'kelp.pdf', cited: 1, quote: 'urchins graze kelp', status: 'quoted', found: 1 };
  const entries = [
    { kind: 'user', text: 'go' },
    { kind: 'assistant', text: `Before reading: ${a}.` },
    { kind: 'tool', name: 'file_read' },
    { kind: 'assistant', text: `They graze ${a} and ${b}.` },
  ];
  const paired = citeEntries(entries, [before, after, made]);
  assert.deepEqual(paired.get(1).map(([, c]) => c.status), ['no_such_file']);
  assert.deepEqual(paired.get(3).map(([, c]) => c.status), ['quoted', 'not_found']);
  const segs = citeSegments(entries[3].text, paired.get(3));
  assert.equal(segs.map((s) => s.text).join(''), entries[3].text, 'nothing lost or doubled');
  assert.deepEqual(segs.filter((s) => s.check).map((s) => s.check.status), ['quoted', 'not_found']);
  // Compacted: the first answer is off the page, its check is not.
  const later = citeEntries(entries.slice(2), [before, after, made]);
  assert.deepEqual(later.get(1).map(([, c]) => c.status), ['quoted', 'not_found']);
  assert.deepEqual(citeSegments('plain words', []), [{ text: 'plain words' }]);
  assert.deepEqual(citeSegments('', null), [{ text: '' }]);
  assert.equal(citeEntries(entries, []).size, 0);
  assert.equal(citeNote({ status: 'quoted' }).label, 'quoted');
  assert.match(citeNote({ status: 'quoted' }).title, /not checked for support/);
  assert.equal(citeNote({ status: 'other_page', found: 4 }).label, 'on p. 4');
  assert.equal(citeNote({ status: 'not_found' }).tone, 'bad');
  // What the check cannot speak for, it does not accuse (review of #465).
  assert.equal(citeNote({ status: 'not_read' }).tone, 'muted');
  assert.equal(citeNote({ status: 'cannot_check' }).tone, 'muted');
  assert.ok(citeOpens({ status: 'other_page' }) && !citeOpens({ status: 'not_found' }));
  // The page it was found on, not the one cited; the quote to mark.
  const url = citedUrl('p-0123456789ab', { file: '@kelp/s.pdf', cited: 2, found: 4, quote: 'a b' }, null);
  assert.match(url, /^\/api\/persona-chat\/p-0123456789ab\/cited\?/);
  const q = new URLSearchParams(url.split('?')[1]);
  assert.deepEqual([q.get('file'), q.get('page'), q.get('quote')], ['@kelp/s.pdf', '4', 'a b']);
  // The stream's checks replace the page's.
  const run = applyEvent({ ...emptyRun([], null, [before]) }, { type: 'citations', checks: [made] });
  assert.deepEqual(run.citations, [made]);
}

// The owner's bubble shows their words, not the goal framing the harness
// sends ahead of them; and consecutive calls to one tool draw as one row,
// except a call with a picture or one that failed (the owner's ask,
// 2026-10-01).
{
  const { ownWords, toolRun } = await import('../src/lib/persona.js');
  assert.equal(ownWords('(What I want from this conversation: Plan the trip)\n\nGive me a summary'), 'Give me a summary');
  assert.equal(ownWords('No goal here'), 'No goal here');
  assert.equal(ownWords('(What I want from this conversation: x) but on one line'), '(What I want from this conversation: x) but on one line');
  const t = (name, extra = {}) => ({ kind: 'tool', name, is_error: false, ...extra });
  const es = [t('file_search'), t('file_search'), t('file_search'), t('file_read'), t('file_read', { is_error: true }), t('file_read'), { kind: 'assistant', text: 'x' }, t('image_view', { pic: 1 }), t('image_view', { pic: 1 })];
  const plain = (e) => !e.pic;
  const failed = (e) => e.is_error === true;
  const runs = es.map((_, i) => toolRun(es, i, plain, failed)).map((r, i) => (es[i].kind === 'tool' && r.first ? `${es[i].name}x${r.count}` : null)).filter(Boolean);
  assert.deepEqual(runs, ['file_searchx3', 'file_readx1', 'file_readx1', 'file_readx1', 'image_viewx1', 'image_viewx1']);
}

// The earlier-chats line, and a file's two doors (the owner's asks).
{
  const { chatHeadline, sourceFileUrl } = await import('../src/lib/persona.js');
  assert.equal(chatHeadline({ summary: 'Went over the winter transects', goal: 'g', opener: 'o' }), 'Went over the winter transects');
  assert.equal(chatHeadline({ summary: ' ', goal: 'Plan the survey', opener: 'o' }), 'Plan the survey');
  assert.equal(chatHeadline({ opener: 'What do urchins do?' }), 'What do urchins do?');
  assert.equal(chatHeadline({}), null);
  assert.equal(sourceFileUrl('mara', '@kelp/a b.pdf', 'file'), '/api/personas/mara/sources/file?file=%40kelp%2Fa%20b.pdf');
  assert.equal(sourceFileUrl('mara', 'n.md', 'text', 'tok'), '/api/personas/mara/sources/text?unlock=tok&file=n.md');
}

// A citation keeps its badge through the Markdown parser, whatever it
// holds (review of #479): a backtick, a `*` pair, a bare URL.
{
  const { citeMark, citeUnmark } = await import('../src/lib/persona.js');
  const { parseBlocks } = await import('../src/lib/mail-markdown.js');
  const raw = '[notes.md: "the `holdfast` is **not** at https://example.org/kelp"]';
  const check = { raw, status: 'quoted' };
  const reply = `## Point\n\nThey say ${raw} and more.`;
  const { text, marks } = citeMark(reply, [[raw, check]]);
  const blocks = parseBlocks(text);
  const nodes = blocks[1].inline.filter((n) => n.t === 'text');
  const drawn = nodes.flatMap((n) => citeUnmark(n.v, marks));
  const cite = drawn.find((p) => p.check);
  assert.equal(cite?.text, raw, JSON.stringify(drawn));
  assert.equal(drawn.map((p) => p.text).join(''), `They say ${raw} and more.`);
  assert.deepEqual(citeUnmark('plain', marks), [{ text: 'plain' }]);
  assert.equal(citeMark('no cites', null).text, 'no cites');
  // A reply that already holds the placeholder characters cannot forge one.
  const forged = citeMark('a \uE0000\uE001 b', [[raw, check]]);
  assert.deepEqual(citeUnmark(forged.text, forged.marks), [{ text: 'a 0 b' }]);
}

console.log('persona: ok');

// The avatar's framing: a sent frame held to the server's ranges, the style
// it draws with, and a drag.
{
  const { DEFAULT_FRAME, frameOf, frameStyle, dragFrame } = await import('../src/lib/persona.js');
  assert.deepEqual(frameOf(null), DEFAULT_FRAME);
  // A copy the editor can write its zoom into, never the frozen default.
  // The page's zoom ceiling is the server's (`persona::MAX_FRAME_ZOOM`): a
  // drift is a slider that offers what the save refuses.
  const here = path.dirname(fileURLToPath(import.meta.url));
  const rust = fs.readFileSync(path.join(here, '..', '..', 'mecha-core', 'src', 'persona.rs'), 'utf8');
  const { MAX_FRAME_ZOOM } = await import('../src/lib/persona.js');
  assert.equal(Number(/pub const MAX_FRAME_ZOOM: f32 = ([\d.]+);/.exec(rust)?.[1]), MAX_FRAME_ZOOM);
  const fresh = frameOf(null);
  assert.ok(fresh !== DEFAULT_FRAME && !Object.isFrozen(fresh));
  fresh.zoom = 2;
  assert.equal(DEFAULT_FRAME.zoom, 1);
  assert.deepEqual(frameOf({ x: 0.5, y: 'top', zoom: 1 }), DEFAULT_FRAME, 'not a frame: the default');
  assert.deepEqual(frameOf({ x: 2, y: -1, zoom: 9 }), { x: 1, y: 0, zoom: 4 });
  // Unplaced leans to the top, where a portrait's face is.
  assert.equal(frameStyle(null), 'object-position:50% 20%;transform-origin:50% 20%;transform:scale(1)');
  assert.equal(frameStyle({ x: 0.25, y: 0.1, zoom: 1.5 }), 'object-position:25% 10%;transform-origin:25% 10%;transform:scale(1.5)');
  // Nothing hidden, nothing to pan: a square at zoom 1 does not move, and a
  // tall picture does not move sideways — the frame stays as the circle
  // shows it, rather than saving a change that jumps on the next zoom.
  const TALL = 768 / 1344;
  assert.deepEqual(dragFrame({ x: 0.5, y: 0.5, zoom: 1 }, 20, 10, 200, 1), { x: 0.5, y: 0.5, zoom: 1 });
  assert.equal(dragFrame({ x: 0.5, y: 0.2, zoom: 1 }, 40, 0, 200, TALL).x, 0.5);
  // The picture follows the finger: the point under it before the drag is
  // under it after. X(u) = z·u + p·(S − z·R) along each axis.
  const S = 200;
  const at = (p, z, R, u) => z * u + p * (S - z * R);
  for (const [f, dx, dy, aspect] of [
    [{ x: 0.5, y: 0.2, zoom: 1 }, 0, 30, TALL],
    [{ x: 0.3, y: 0.6, zoom: 2.5 }, -25, 12, TALL],
    [{ x: 0.5, y: 0.5, zoom: 1.8 }, 17, -9, 1],
    [{ x: 0.4, y: 0.5, zoom: 1.2 }, 11, 0, 1.6],
  ]) {
    const Rx = S * Math.max(1, aspect);
    const Ry = S * Math.max(1, 1 / aspect);
    const g = dragFrame(f, dx, dy, S, aspect);
    // The picture point at the circle's centre before the drag.
    const ux = (S / 2 - f.x * (S - f.zoom * Rx)) / f.zoom;
    const uy = (S / 2 - f.y * (S - f.zoom * Ry)) / f.zoom;
    assert.ok(Math.abs(at(g.x, g.zoom, Rx, ux) - (S / 2 + dx)) < 1e-6, `x ${JSON.stringify([f, dx, aspect])}`);
    assert.ok(Math.abs(at(g.y, g.zoom, Ry, uy) - (S / 2 + dy)) < 1e-6, `y ${JSON.stringify([f, dy, aspect])}`);
  }
  // It stops at the picture's edge.
  assert.equal(dragFrame({ x: 0.05, y: 0.5, zoom: 2 }, 400, 0, 200).x, 0);
}

// Unsaved edits, as the lock switch reads them (review of #491): the open
// tab by its text, another tab by its kept draft — and locking waits only
// when it would close the editor (no unlock, not yet locked).
{
  const editing = {
    file: 'settings',
    text: 'display = "Mara"',
    files: {
      identity: { text: '# Mara', draft: '# Mara' },
      motivation: { text: 'kelp', draft: 'kelp and urchins' },
      settings: { text: 'display = "Mara"' },
    },
  };
  assert.deepEqual(unsavedFiles(editing), ['motivation'], 'a draft in another tab counts');
  assert.equal(fileUnsaved(editing.files.identity), false, 'a draft equal to the file is not a change');
  assert.deepEqual(unsavedFiles({ ...editing, text: 'display = "M"' }), ['motivation', 'settings']);
  const mara = { locked: false };
  assert.equal(lockWaits({ chosen: mara, token: null, editing }), true);
  assert.equal(lockWaits({ chosen: mara, token: 't', editing }), false, 'with the unlock, locking hides nothing');
  assert.equal(lockWaits({ chosen: { locked: true }, token: null, editing }), false, 'unlocking never waits');
  const clean = { ...editing, files: { ...editing.files, motivation: { text: 'kelp' } } };
  assert.equal(lockWaits({ chosen: mara, token: null, editing: clean }), false);
  assert.equal(lockWaits({ chosen: mara, token: null, editing: null }), false);
}

// The Waiting section (ruled 2026-10-01): proposals a chat made, apart from
// the rest; an owner's own unapproved persona is not one.
{
  const { splitWaiting, proposalOrigin } = await import('../src/lib/persona.js');
  const { waiting, rest } = splitWaiting([
    { name: 'mara', waiting: false, origin: 'owner' },
    { name: 'wren', waiting: true, origin: 'model_clean' },
    { name: 'mine', waiting: true, origin: 'owner' },
    { name: 'noor', waiting: true, origin: 'model_untrusted' },
  ]);
  assert.deepEqual(waiting.map((p) => p.name), ['wren', 'noor']);
  assert.deepEqual(rest.map((p) => p.name), ['mara', 'mine']);
  assert.deepEqual(splitWaiting(null), { waiting: [], rest: [] });
  assert.equal(proposalOrigin('model_clean'), 'proposed in a chat');
  assert.match(proposalOrigin('model_untrusted'), /outside content/);
  // An origin this page does not know reads as the cautious one.
  assert.match(proposalOrigin('something_new'), /outside content/);
}
