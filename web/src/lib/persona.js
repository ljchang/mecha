// The Personas tab's pure logic: where its requests go, and how a streamed
// run folds into the transcript. Kept out of the component so a test can
// import it (`web/test/persona.mjs`), as `library.js` is.
//
// A persona chat has a door of its own on the server (`persona_chat.rs`):
// every URL here is under `/api/personas` or `/api/persona-chat/{key}`, never
// the assistant's `/api/chat/{key}` — a page that mixed them up would be
// reading one conversation's words into the other.

// A persona chat key, exactly as the server mints one: `p-` and twelve hex.
const KEY = /^p-[0-9a-f]{12}$/;

export function isPersonaKey(key) {
  return typeof key === 'string' && KEY.test(key);
}

// A path with the library's unlock token, when there is one. The token rides
// as a query parameter because `EventSource` cannot send a body or a header.
export function withUnlock(path, token) {
  if (!token) return path;
  const sep = path.includes('?') ? '&' : '?';
  return `${path}${sep}unlock=${encodeURIComponent(token)}`;
}

// The suffixes each builder below accepts, and so every endpoint this page
// can reach. The docs demo's guard (`website/scripts/check-demo.mjs`) cannot
// see through a builder — its scan wants a literal after `fetch(` — so it
// imports `ENDPOINTS` instead, and the builders refuse any suffix not listed
// here: a new endpoint is added to this list or it throws, and the list is
// then what `check-demo` holds the demo's routes to (review of #415).
const PERSONA_SUFFIXES = ['/chats', '/resume', '/files', '/lock', '/frame', '/sources', '/sources/remove', '/sources/file', '/sources/text'];
const CHAT_SUFFIXES = ['', '/events', '/send', '/cancel', '/file', '/upload', '/cited', '/save', '/call'];

export const ENDPOINTS = [
  '/api/personas',
  '/api/personas/authoring',
  '/api/personas/relationships',
  '/api/personas/groups',
  ...PERSONA_SUFFIXES.map((s) => `/api/personas/X${s}`),
  ...CHAT_SUFFIXES.map((s) => `/api/persona-chat/X${s}`),
];

export function listUrl(token) {
  return withUnlock('/api/personas', token);
}

// What a new persona can be made from: templates, groups, characters.
export function authoringUrl(token) {
  return withUnlock('/api/personas/authoring', token);
}

// The portrait a half-made persona keeps when the character list changes
// under it — a relock takes locked characters out of the list, and a choice
// the page can no longer show must not be sent as if it could.
export function keptCharacter(chosen, characters) {
  return chosen && (characters ?? []).includes(chosen) ? chosen : '';
}

// A name as the store will hold it — or null when it cannot be one: the
// server says why on create, this only saves a round trip for the obvious.
export function personaName(typed) {
  const name = (typed ?? '').trim().toLowerCase();
  const reserved = ['files', 'groups', 'relationships', 'voices', 'scenarios', 'removed', 'sessions'];
  return /^[a-z0-9][a-z0-9_-]{0,63}$/.test(name) && !reserved.includes(name) ? name : null;
}

// The three files the page edits, in the order it shows them.
export const OWNER_FILES = [
  ['identity', 'Who they are'],
  ['motivation', 'What they want'],
  ['settings', 'Settings'],
];

// One of a persona's files, by its listed name: to download (`file`) or to
// read as text (`text`). The server finds it in the persona's own listing.
export function sourceFileUrl(name, file, what, token = null) {
  const base = personaUrl(name, what === 'text' ? '/sources/text' : '/sources/file', token);
  return `${base}${base.includes('?') ? '&' : '?'}file=${encodeURIComponent(file)}`;
}

// An earlier chat's line in the list: what it was about, as the memory
// writer summed it up, else the goal it was opened with, else how the owner
// opened it — and `null` when none, for the page to show the day.
export function chatHeadline(h) {
  for (const v of [h?.summary, h?.goal, h?.opener]) {
    const t = (v ?? '').trim();
    if (t) return t;
  }
  return null;
}

export function personaUrl(name, suffix, token) {
  if (!PERSONA_SUFFIXES.includes(suffix)) throw new Error(`not a persona endpoint: ${suffix}`);
  return withUnlock(`/api/personas/${encodeURIComponent(name)}${suffix}`, token);
}

// A chat's URL. A key that is not a persona key is refused here, so this
// page can never address an assistant chat by mistake.
export function chatUrl(key, suffix = '', token = null) {
  if (!isPersonaKey(key)) throw new Error(`not a persona chat key: ${key}`);
  if (!CHAT_SUFFIXES.includes(suffix)) throw new Error(`not a persona chat endpoint: ${suffix}`);
  return withUnlock(`/api/persona-chat/${key}${suffix}`, token);
}

// A picture in the chat's workspace, as an `<img>` asks for it: the tool's
// own workspace-relative path, the chat's own door (`persona_chat::download`).
export function fileUrl(key, path, token = null) {
  return withUnlock(`${chatUrl(key, '/file')}?path=${encodeURIComponent(path)}`, token);
}

// A cited page as the chat read it, the quote marked (`persona_chat::cited`):
// the page the persona says it cited, or — where the quote was found on
// another — that one.
export function citedUrl(key, check, token = null) {
  const page = check.found ?? check.cited;
  const q = new URLSearchParams({ file: check.file, quote: check.quote ?? '' });
  if (page != null) q.set('page', String(page));
  return withUnlock(`${chatUrl(key, '/cited')}?${q}`, token);
}

// A reply cut at its citations: plain text, and each citation with its
// check (§10.4). `checks` is a list of [raw, check] pairs already matched
// to this reply's citations, in order (`citeEntries`).
export function citeSegments(text, matched) {
  const out = [];
  let pos = 0;
  for (const [raw, check] of matched ?? []) {
    const at = text.indexOf(raw, pos);
    if (at === -1) continue;
    if (at > pos) out.push({ text: text.slice(pos, at) });
    out.push({ text: raw, check });
    pos = at + raw.length;
  }
  if (pos < text.length || out.length === 0) out.push({ text: text.slice(pos) });
  return out;
}

// Each answer's citations paired with the check the server made of *that*
// citation (review of #465). The server checks each reply against what the
// chat had read by then, so one quote can be "no such file" before its page
// was read and "quoted" after: keyed by text alone, the last would answer
// for both. Paired from the end, occurrence by occurrence, because the
// server's checks run over everything the chat ever held while a compacted
// chat's page shows only its later answers. Returns, per entry index, the
// [raw, check] pairs `citeSegments` takes.
export function citeEntries(entries, checks) {
  const byRaw = new Map();
  for (const c of checks ?? []) {
    if (!c?.raw) continue;
    if (!byRaw.has(c.raw)) byRaw.set(c.raw, []);
    byRaw.get(c.raw).push(c);
  }
  const out = new Map();
  if (byRaw.size === 0) return out;
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e?.kind !== 'assistant' || !e.text) continue;
    const found = [];
    for (const raw of byRaw.keys()) {
      for (let at = e.text.indexOf(raw); at !== -1; at = e.text.indexOf(raw, at + raw.length)) {
        found.push({ at, raw });
      }
    }
    found.sort((a, b) => a.at - b.at);
    // Overlapping finds keep the first.
    const kept = [];
    let end = 0;
    for (const f of found) {
      if (f.at < end) continue;
      kept.push(f);
      end = f.at + f.raw.length;
    }
    const pairs = [];
    for (let k = kept.length - 1; k >= 0; k--) {
      const list = byRaw.get(kept[k].raw);
      // More on the page than were checked: the earliest check stands in.
      const check = list.length > 1 ? list.pop() : list[0];
      pairs.unshift([kept[k].raw, check]);
    }
    out.set(i, pairs);
  }
  return out;
}

// What a citation's check says, in a word and in full. A quote that is
// there is quoted — never "verified": a real quote can support the wrong
// claim, and that is not checked (§10.4).
export function citeNote(check) {
  switch (check?.status) {
    case 'quoted':
      return { tone: 'ok', label: 'quoted', title: 'The quote is in the file. Quoted, not checked for support.' };
    case 'other_page':
      return { tone: 'warn', label: `on p. ${check.found}`, title: `The quote is in the file, on page ${check.found}, not the page cited. Quoted, not checked for support.` };
    case 'not_found':
      return { tone: 'bad', label: 'not in the file', title: 'This quote is not in what the chat read of the file.' };
    case 'no_such_file':
      return { tone: 'bad', label: 'no such file', title: 'Not one of the files this chat read.' };
    case 'not_read':
      return { tone: 'muted', label: 'file not read', title: 'One of its files, but this chat had not read it when it quoted it.' };
    case 'cannot_check':
      return { tone: 'muted', label: "can't check", title: 'This file is in a script without spaces between words, which the check cannot compare.' };
    case 'too_short':
      return { tone: 'muted', label: 'too short to check', title: 'Too short to tell a quote from a coincidence.' };
    default:
      return { tone: 'muted', label: 'unchecked', title: 'Not checked.' };
  }
}

// Whether a citation opens a page: only where the quote was found.
export function citeOpens(check) {
  return check?.status === 'quoted' || check?.status === 'other_page';
}

// Where the edit modal's mask goes up: this chat's `inbox/`, never the
// assistant's (`persona_chat::upload`).
export function uploadUrl(key, name, token = null) {
  return withUnlock(`${chatUrl(key, '/upload')}?name=${encodeURIComponent(name)}`, token);
}

// What an editor save must carry across its reload, per file: the mode each
// tab was in, and for every file but the one just saved, its unsaved text
// draft and its unsaved form draft. The reload replaces every file's entry,
// so anything not named here is dropped — which is how form drafts went
// missing on the first cut (review of #430), as text drafts had (#420).
export function keptEdits(files, saved) {
  const out = {};
  for (const [f, v] of Object.entries(files ?? {})) {
    const keep = { asText: v.asText };
    if (f !== saved) {
      if (v.draft != null && v.draft !== v.text) keep.draft = v.draft;
      if (v.formDraft) keep.formDraft = v.formDraft;
    }
    out[f] = keep;
  }
  return out;
}

export function relationshipLabel(p) {
  const r = p?.relationship ?? [];
  return r.length ? r.join(' · ').replaceAll('_', ' ') : '';
}

// The state a streamed run folds into: the transcript so far, the answer
// still arriving, and whether a run is live.
// What a tool row says. A call the tool refused with instructions and the
// persona then made again reads as retried, not failed — "failed" twice
// before a picture arrived read as broken to the owner (2026-09-30).
export function toolStatus(entries, i) {
  const e = entries[i];
  if (e.is_error == null) return 'running';
  if (!e.is_error) return 'done';
  // Within the same turn only: the owner's next message ends the search, or
  // a failure reads "retried" because the tool ran again days later
  // (review of #431).
  const after = entries.slice(i + 1);
  const turnEnd = after.findIndex((x) => x.kind === 'user');
  const sameTurn = turnEnd === -1 ? after : after.slice(0, turnEnd);
  const again = sameTurn.some((x) => x.kind === 'tool' && x.name === e.name);
  return again ? 'retried' : 'failed';
}

const DOING = {
  image_generate: 'drawing a picture',
  image_view: 'looking at an image',
  web_search: 'searching the web',
  fs_read: 'reading a file',
};

const clockOf = (ms) => {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
};

// The line under a run that has not answered yet, so a slow local model
// never looks broken (owner, 2026-09-30): the tool it is waiting on and for
// how long, else that the persona is typing. Nothing once text streams, and
// nothing when no run is live.
export function waitingLine(run, display, now) {
  if (!run?.running || run.streaming) return null;
  const pending = [...(run.entries ?? [])].reverse().find((e) => e.kind === 'tool' && e.is_error == null);
  if (pending) {
    const doing = DOING[pending.name] ?? `using ${pending.name}`;
    return pending.started ? `${doing}… ${clockOf(now - pending.started)}` : `${doing}…`;
  }
  return `${display} is typing`;
}

// The transcript a page re-reads mid-run has every finished tool call, not
// the one still running; the server names that one (`working`), and it is
// put back as a pending row with its real start, so a reload during a long
// render still reads "drawing a picture… 1:24" (review of #431).
// `now` is this page's clock. The server says how long the tool has run
// (`elapsed_ms`), not when it began by its clock, so a phone whose clock is
// off still counts from the right moment (review of #431).
export function withWorking(entries, working, now = Date.now()) {
  if (!working?.id || entries.some((e) => e.kind === 'tool' && e.id === working.id)) return entries;
  const elapsed = Number(working.elapsed_ms);
  const started = Number.isFinite(elapsed) ? now - elapsed : undefined;
  return [...entries, { kind: 'tool', id: working.id, name: working.name, is_error: null, started }];
}

export function emptyRun(entries = [], taint = null, citations = []) {
  // `crisisSeq` carries on past the entries it numbered, so ids never repeat
  // within a page's life of the chat.
  const seq = entries.filter((e) => e.kind === 'crisis').length;
  return { entries, streaming: null, running: false, taint, crisisSeq: seq, citations };
}

// Move text still streaming into the transcript as the answer it became.
function flush(state) {
  if (state.streaming == null || state.streaming === '') return { ...state, streaming: null };
  return {
    ...state,
    entries: [...state.entries, { kind: 'assistant', text: state.streaming }],
    streaming: null,
  };
}

// What only the page holds: the server's transcript has users, answers and
// tools, and nothing of a notice or a steer that never reached the run.
function pageOnly(e) {
  return e.kind === 'notice' || e.kind === 'crisis' || (e.queued && e.delivery === 'discarded');
}

// The transcript after a finished run: the server's, which has the whole
// answer — the part that streamed before a late subscriber arrived too —
// with what only the page held carried across, in the order it arrived.
// Re-reading without this deleted the failure notice and the "not
// delivered" receipt one round trip after drawing them (review of #415).
export function settle(serverEntries, local) {
  return [...(serverEntries ?? []), ...local.entries.filter(pageOnly)];
}

function markDelivery(state, ids, delivery) {
  const wanted = new Set(ids);
  return {
    ...state,
    entries: state.entries.map((e) => (e.queued && wanted.has(e.request_id) ? { ...e, delivery } : e)),
  };
}

// Fold one server-sent event into the run. Unknown events change nothing:
// the server may grow a kind this page does not draw yet.
export function applyEvent(state, ev) {
  switch (ev?.type) {
    case 'user': {
      const s = flush(state);
      return { ...s, running: true, entries: [...s.entries, { kind: 'user', text: ev.text }] };
    }
    case 'queued':
      return {
        ...state,
        entries: [...state.entries, { kind: 'user', text: ev.text, queued: true, request_id: ev.request_id ?? null, delivery: null }],
      };
    // A steer's receipt: taken into the run, or arrived too late for it.
    // Without these a steered bubble reads "queued" forever (review of #415).
    case 'queued_delivered':
      return markDelivery(state, [ev.request_id], 'delivered');
    case 'queued_discarded':
      return markDelivery(state, ev.request_ids ?? [], 'discarded');
    case 'delta':
      return { ...state, running: true, streaming: (state.streaming ?? '') + ev.text };
    case 'tool': {
      const s = flush(state);
      // `started` is the page's clock, for the "drawing a picture… 1:24"
      // line: a local model can take minutes on an image. (A re-read takes
      // it from the server's `working.since` instead — `withWorking`.)
      return { ...s, entries: [...s.entries, { kind: 'tool', id: ev.id, name: ev.name, is_error: null, started: Date.now() }] };
    }
    // The preview is kept: its first line names the picture a finished
    // `image_generate` drew, which the page shows under the row (`picture.js`).
    case 'tool_result':
      return {
        ...state,
        entries: state.entries.map((e) =>
          e.kind === 'tool' && e.id === ev.id ? { ...e, is_error: ev.is_error, preview: ev.preview } : e,
        ),
      };
    // The crisis sensor fired and the persona paused: the plain voice's
    // message, drawn as its own card — not the persona's words (§12.2).
    // Each card has its own id, so closing one survives a re-read that moves
    // it (review of #418: keyed by position, a closed card re-opened).
    case 'crisis': {
      const s = flush(state);
      const seq = (s.crisisSeq ?? 0) + 1;
      return { ...s, crisisSeq: seq, entries: [...s.entries, { kind: 'crisis', text: ev.text, id: `crisis-${seq}` }] };
    }
    // A call refused before it ran: the row says so, with the reason.
    // A refused call gets no result: its pending row is closed as failed, or
    // the waiting line would count it for the rest of the run (review of #431).
    case 'denied': {
      const at = state.entries.findLastIndex((e) => e.kind === 'tool' && e.name === ev.name && e.is_error == null);
      const entries = at === -1 ? state.entries : state.entries.map((e, i) => (i === at ? { ...e, is_error: true } : e));
      return { ...state, entries: [...entries, { kind: 'notice', text: `${ev.name} refused: ${ev.reason}` }] };
    }
    case 'notice':
      return { ...state, entries: [...state.entries, { kind: 'notice', text: ev.text }] };
    // Every citation in the chat, checked: the server sends them all, so
    // they replace what the page had (§10.4).
    case 'citations':
      return { ...state, citations: ev.checks ?? [] };
    case 'done': {
      const s = flush(state);
      const entries = ev.ok ? s.entries : [...s.entries, { kind: 'notice', text: ev.error || 'the turn failed' }];
      // What the run touched, as the transcript's chip reads it.
      const taint = { private: !!ev.taint_private, untrusted: !!ev.taint_untrusted };
      return { ...s, entries, running: false, taint };
    }
    default:
      return state;
  }
}

// The chip's words for what a conversation has touched, or '' for nothing.
export function taintLabel(taint) {
  if (!taint) return '';
  return [taint.private && 'private', taint.untrusted && 'untrusted'].filter(Boolean).join(' + ');
}

// The dose meters, in a line; '' when they are off.
// Seconds on calls, as the meters say them: minutes, an hour and minutes past
// the hour, and "under a minute" rather than a zero that reads as no call.
export function callTime(secs) {
  if (secs <= 0) return '0 min';
  if (secs < 60) return 'under a minute';
  const m = Math.round(secs / 60);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
}

export function doseLine(dose) {
  if (!dose) return '';
  // A store that could not be read is not zero turns (review of #418).
  if (dose.unread) return 'usage meters unreadable';
  const parts = [`${dose.turns_today} today`, `${dose.turns_7d} this week`];
  if (dose.late_night_7d) parts.push(`${dose.late_night_7d} late at night`);
  // Call time, beside the turns rather than in them (§11: voice raises
  // attachment, so the meters count minutes on calls too).
  if (dose.call_secs_7d) parts.push(`calls ${callTime(dose.call_secs_today ?? 0)} today, ${callTime(dose.call_secs_7d)} this week`);
  if (dose.skipped) parts.push(`${dose.skipped} unreadable record${dose.skipped === 1 ? '' : 's'} not counted`);
  return parts.join(' · ');
}

// A file in a persona's reach, as its row says it (§10): whether a chat can
// read it without waiting, and how big it is. Shared files say where from —
// `@kelp/…` is a group's, `@all/…` everyone's.
export function sourceLine(s) {
  const size = s.bytes >= 1048576 ? `${(s.bytes / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(s.bytes / 1024))} KB`;
  // `@group:all/` is a group literally called `all` (files.rs `roots`).
  const group = s.name.slice(1, s.name.indexOf('/')).replace(/^group:/, '');
  const from = !s.shared ? '' : s.name.startsWith('@all/') ? ' · every persona' : ` · group ${group}`;
  return `${size} · ${sourceState(s)}${from}`;
}

// The read state alone, for a file's tile; the tile's title carries the
// whole `sourceLine`. `unreadable` is why it never will be read (the
// reader's own words); it outranks a queued read.
export function sourceState(s) {
  return s.unreadable
    ? 'not readable'
    : s.processing
      ? 'reading…'
      : s.ready
        ? 'ready'
        : s.on_request
          ? 'read when asked'
          : 'not read yet';
}

// A file's kind as its tile badges it, from the name's extension — the
// same set the add control accepts. Anything else says its own extension.
export function fileKind(name) {
  const ext = /\.([^./]+)$/.exec(name)?.[1]?.toLowerCase() ?? '';
  if (['png', 'jpg', 'jpeg', 'webp', 'gif'].includes(ext)) return 'IMG';
  if (['md', 'markdown'].includes(ext)) return 'MD';
  return ext ? ext.slice(0, 4).toUpperCase() : 'FILE';
}

// ─── The avatar's framing ────────────────────────────────────────────────
// A portrait drawn into the round avatar with `object-fit: cover`. The frame
// (`State::frame`, set by the owner) is `object-position` — `x` and `y` say
// how far across what is hidden the picture has slid, 0 its left or top
// edge, 1 its right or bottom, so it never leaves a gap — and how far in,
// scaled about that same point, which stays put as it zooms.
// Unplaced, it leans to the top: portraits are people, and a tall one
// cropped at its middle shows a chest (owner report, 2026-10-01).
export const DEFAULT_FRAME = Object.freeze({ x: 0.5, y: 0.2, zoom: 1 });
export const MAX_FRAME_ZOOM = 4; // `persona::MAX_FRAME_ZOOM`

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

// A frame the server sent, held to what it accepts; anything else is the
// default rather than a broken picture. Always a fresh object: the editor
// binds its zoom slider into what this returns, and a write into the frozen
// default threw (review of #473).
export function frameOf(frame) {
  const ok = (v) => typeof v === 'number' && Number.isFinite(v);
  if (!frame || !ok(frame.x) || !ok(frame.y) || !ok(frame.zoom)) return { ...DEFAULT_FRAME };
  return { x: clamp(frame.x, 0, 1), y: clamp(frame.y, 0, 1), zoom: clamp(frame.zoom, 1, MAX_FRAME_ZOOM) };
}

// The `<img>`'s style for a frame.
export function frameStyle(frame) {
  const f = frameOf(frame);
  const at = `${+(f.x * 100).toFixed(2)}% ${+(f.y * 100).toFixed(2)}%`;
  return `object-position:${at};transform-origin:${at};transform:scale(${+f.zoom.toFixed(3)})`;
}

// A drag of `dx`, `dy` pixels across an avatar `size` pixels wide, over a
// picture `aspect` (width / height) wide. The picture follows the finger.
//
// Along one axis, with the picture `cover`-fitted to `R` pixels in a box of
// `S` and scaled by `z` about the same point `object-position` picks, the
// picture's point `u` lands at `X = z·u + p·(S − z·R)` — so a drag of `dX`
// moves `p` by `−dX / (z·R − S)`, and an axis with nothing hidden to pan to
// (`z·R = S`: a square at zoom 1, or a tall picture's width) does not move
// at all, rather than changing a frame the circle cannot show (review of
// #473).
export function dragFrame(frame, dx, dy, size, aspect = 1) {
  const f = frameOf(frame);
  const S = Math.max(1, size);
  const a = Number.isFinite(aspect) && aspect > 0 ? aspect : 1;
  const pan = (p, d, R) => {
    const room = f.zoom * R - S;
    return room > 0.5 ? clamp(p - d / room, 0, 1) : p;
  };
  return { ...f, x: pan(f.x, dx, S * Math.max(1, a)), y: pan(f.y, dy, S * Math.max(1, 1 / a)) };
}

// The owner's words in their own bubble: a chat opened with a goal sends it
// ahead of the first message, as "(What I want from this conversation: …)",
// so the model reads it — but it is the harness's framing, not something
// the owner typed (the owner's ask, 2026-10-01).
const GOAL_PREAMBLE = /^\(What I want from this conversation: [^\n]*\)\n\n/;
export function ownWords(text) {
  return (text ?? '').replace(GOAL_PREAMBLE, '');
}

// Consecutive calls to the same tool, drawn as one row ("file_read ×6"):
// `first` says whether entry `i` starts a run (the rest are not drawn), and
// `count` how long it is. A call `plain` says no to — one that drew a
// picture, which is the answer and not a detail — stands alone, as does a
// failed one, which the row reports.
export function toolRun(entries, i, plain = () => true, failed = () => false) {
  const e = entries[i];
  const joins = (x) => x?.kind === 'tool' && x.name === e.name && plain(x) && !failed(x);
  if (!joins(e)) return { first: true, count: 1 };
  if (i > 0 && joins(entries[i - 1])) {
    return { first: false, count: 0 };
  }
  let count = 1;
  while (joins(entries[i + count])) count++;
  return { first: true, count };
}

// A reply's checked citations swapped for placeholders before its Markdown
// is parsed, and drawn back from them after (`ChatProse`): a citation
// holding a backtick, a `*` pair or a bare URL would otherwise be split
// across the parser's nodes and lose its badge silently (review of #479).
// The placeholders are private-use characters a reply does not contain.
const MARK_OPEN = '\uE000';
const MARK_CLOSE = '\uE001';
//
// One check per citation text in a reply: `citeEntries` pairs occurrence by
// occurrence across replies, but two occurrences inside one reply were
// checked against the same received state, so they cannot differ — the
// collapse here is known and safe (review of #479).
export function citeMark(text, cites) {
  const byRaw = new Map((cites ?? []).map(([r, c]) => [r, c]).reverse());
  const marks = [];
  // The placeholder characters themselves are dropped from the reply first:
  // a reply relaying a paper could hold them, and one would then show raw
  // or replay another citation's badge (review of #479).
  let out = (text ?? '').replace(/[\uE000\uE001]/g, '');
  for (const [r, check] of byRaw) {
    if (!r || !out.includes(r)) continue;
    out = out.split(r).join(`${MARK_OPEN}${marks.length}${MARK_CLOSE}`);
    marks.push({ text: r, check });
  }
  return { text: out, marks };
}

// A parsed text node cut at the placeholders it holds, each drawn back as
// its citation.
export function citeUnmark(v, marks) {
  if (!marks?.length || !v.includes(MARK_OPEN)) return [{ text: v }];
  const out = [];
  const re = new RegExp(`${MARK_OPEN}(\\d+)${MARK_CLOSE}`, 'g');
  let pos = 0;
  for (const m of v.matchAll(re)) {
    if (m.index > pos) out.push({ text: v.slice(pos, m.index) });
    const mark = marks[Number(m[1])];
    out.push(mark ? { text: mark.text, check: mark.check } : { text: m[0] });
    pos = m.index + m[0].length;
  }
  if (pos < v.length) out.push({ text: v.slice(pos) });
  return out;
}
