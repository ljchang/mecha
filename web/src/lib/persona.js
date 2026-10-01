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
const PERSONA_SUFFIXES = ['/chats', '/resume', '/files', '/lock', '/sources', '/sources/remove'];
const CHAT_SUFFIXES = ['', '/events', '/send', '/cancel', '/file', '/upload', '/cited'];

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

// A reply cut at its citations: plain text, and each citation the server
// checked (§10.4) with its check. Found by the citation's own text, as the
// persona wrote it; a later check of the same text wins.
export function citeSegments(text, checks) {
  const byRaw = new Map((checks ?? []).map((c) => [c.raw, c]));
  const found = [];
  for (const [raw, check] of byRaw) {
    if (!raw) continue;
    for (let at = text.indexOf(raw); at !== -1; at = text.indexOf(raw, at + raw.length)) {
      found.push({ at, end: at + raw.length, check });
    }
  }
  found.sort((a, b) => a.at - b.at);
  const out = [];
  let pos = 0;
  for (const f of found) {
    if (f.at < pos) continue;
    if (f.at > pos) out.push({ text: text.slice(pos, f.at) });
    out.push({ text: text.slice(f.at, f.end), check: f.check });
    pos = f.end;
  }
  if (pos < text.length || out.length === 0) out.push({ text: text.slice(pos) });
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

// What the safety layer can do for a persona, in a line (§12): said as it
// is, so "keywords only" never reads as a check that passed.
export function safetyLine(safety) {
  if (!safety) return '';
  // Three states, each said as it is: both tiers answering, keywords only
  // because the model check could not answer, or switched off.
  const crisis = {
    off: 'crisis detection off',
    on: 'crisis detection on',
    // A persona's own setting, before any chat has asked the judge.
    enabled: 'crisis detection: keywords + a model check on each message',
  }[safety.crisis] ?? 'crisis detection: keywords only (the model check could not answer)';
  const off = ['disclosure', 'reanchor', 'dose'].filter((k) => safety[k] === false);
  return off.length ? `${crisis} · off: ${off.join(', ')}` : crisis;
}

// The dose meters, in a line; '' when they are off.
export function doseLine(dose) {
  if (!dose) return '';
  // A store that could not be read is not zero turns (review of #418).
  if (dose.unread) return 'usage meters unreadable';
  const parts = [`${dose.turns_today} today`, `${dose.turns_7d} this week`];
  if (dose.late_night_7d) parts.push(`${dose.late_night_7d} late at night`);
  if (dose.skipped) parts.push(`${dose.skipped} unreadable record${dose.skipped === 1 ? '' : 's'} not counted`);
  return parts.join(' · ');
}

// A file in a persona's reach, as its row says it (§10): whether a chat can
// read it without waiting, and how big it is. Shared files say where from —
// `@kelp/…` is a group's, `@all/…` everyone's.
export function sourceLine(s) {
  const size = s.bytes >= 1048576 ? `${(s.bytes / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(s.bytes / 1024))} KB`;
  // `unreadable` is why it never will be read (the reader's own words,
  // shown whole in the row's title); it outranks a queued read.
  const state = s.unreadable
    ? 'not readable'
    : s.processing
      ? 'reading…'
      : s.ready
        ? 'ready'
        : s.on_request
          ? 'read when asked'
          : 'not read yet';
  // `@group:all/` is a group literally called `all` (files.rs `roots`).
  const group = s.name.slice(1, s.name.indexOf('/')).replace(/^group:/, '');
  const from = !s.shared ? '' : s.name.startsWith('@all/') ? ' · every persona' : ` · group ${group}`;
  return `${size} · ${state}${from}`;
}
