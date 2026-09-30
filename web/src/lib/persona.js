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
const PERSONA_SUFFIXES = ['/chats', '/resume', '/files', '/lock'];
const CHAT_SUFFIXES = ['', '/events', '/send', '/cancel'];

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

export function relationshipLabel(p) {
  const r = p?.relationship ?? [];
  return r.length ? r.join(' · ').replaceAll('_', ' ') : '';
}

// The state a streamed run folds into: the transcript so far, the answer
// still arriving, and whether a run is live.
export function emptyRun(entries = [], taint = null) {
  // `crisisSeq` carries on past the entries it numbered, so ids never repeat
  // within a page's life of the chat.
  const seq = entries.filter((e) => e.kind === 'crisis').length;
  return { entries, streaming: null, running: false, taint, crisisSeq: seq };
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
      return { ...s, entries: [...s.entries, { kind: 'tool', id: ev.id, name: ev.name, is_error: null }] };
    }
    case 'tool_result':
      return {
        ...state,
        entries: state.entries.map((e) => (e.kind === 'tool' && e.id === ev.id ? { ...e, is_error: ev.is_error } : e)),
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
    case 'denied':
      return { ...state, entries: [...state.entries, { kind: 'notice', text: `${ev.name} refused: ${ev.reason}` }] };
    case 'notice':
      return { ...state, entries: [...state.entries, { kind: 'notice', text: ev.text }] };
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
  // The farewell check arrives as a state, not a flag (review of #418).
  if (safety.farewell === 'off') off.push('farewell');
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
