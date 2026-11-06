// The #mail/rules page's logic, kept out of the component so it is tested
// as plain node (web/test/mail-rules.mjs). Display only: every decision —
// modes, holds, rates, what can be undone — arrives from `mecha mail rule …
// --json` (MAIL-RULES-DESIGN.md §8), and this file only shapes it.
//
// Two rules carry the page, both from the store's wire format:
// - **Unknown is never a known value.** A mode, group or outcome this build
//   does not name renders as `unknown`, never as the nearest known word.
// - **A rate over nothing is a dash, never 0%.** The JSON carries `null` for
//   an empty denominator; `pct` keeps it a dash.

export const GROUPS = ['mailbox', 'private_record', 'report', 'board'];

/**
 * Every `/api/mail/rules…` path the page reaches, `X` for a segment it fills
 * in. MailRules.svelte calls through `get`/`call` wrappers, which the docs'
 * `check-demo.mjs` literal scan cannot see, so the guard imports this list
 * instead — as it does persona.js's — and `web/test/mail-rules.mjs` checks
 * the component names no path outside it.
 */
export const RULES_ENDPOINTS = [
  '/api/mail/rules',
  '/api/mail/rules/ledger',
  '/api/mail/rules/X',
  '/api/mail/rules/add',
  '/api/mail/rules/X/edit',
  '/api/mail/rules/X/mode',
  '/api/mail/rules/X/pause',
  '/api/mail/rules/pause-all',
  '/api/mail/rules/undo',
  '/api/mail/rules/wrong-match',
  '/api/mail/rules/X/test',
];

const GROUP_LABEL = {
  mailbox: 'Mailbox',
  private_record: 'Private record',
  report: 'Report',
  board: 'Board',
  fixed_words: 'Fixed reply',
  verbatim_forward: 'Forward',
  typed_fields: 'Typed send',
  drafted_prose: 'Drafted reply',
};

export const groupLabel = (g) => GROUP_LABEL[g] ?? 'unknown';

export const MODES = ['off', 'shadow', 'propose', 'auto'];
const RANK = { off: 0, shadow: 1, propose: 2, auto: 3 };

/** A mode this build cannot name is `unknown`; it never ranks as any rung. */
export const modeOf = (m) => (Object.hasOwn(RANK, m) ? m : 'unknown');

/**
 * Whether moving `from` → `to` raises. An unknown current mode counts as a
 * raise, so the page always takes the approval path when it cannot tell —
 * the store decides either way, and the raise path is the one that shows
 * the files.
 */
export function isRaise(from, to) {
  if (!Object.hasOwn(RANK, to)) return false;
  if (!Object.hasOwn(RANK, from)) return true;
  return RANK[to] > RANK[from];
}

const OUTCOME_LABEL = {
  pending: 'pending',
  undo_pending: 'undo pending',
  done: 'done',
  would_have: 'would have',
  offered: 'offered',
  staged: 'staged',
  held: 'held',
  sent: 'sent',
  released_unchanged: 'released unchanged',
  edited: 'edited',
  rejected: 'rejected',
  undone: 'undone',
  wrong_match: 'wrong match',
  expired: 'expired',
  failed: 'failed',
  skipped: 'skipped',
  approval_lapsed: 'approval lapsed: files changed',
};

/** The ledger's outcome as words; `failed`/`skipped`/`held` carry their reason. */
export function outcomeLabel(row) {
  const word = OUTCOME_LABEL[row?.outcome];
  if (!word) return 'unknown';
  if (['failed', 'skipped', 'held'].includes(row.outcome) && row.reason) return `${word}: ${row.reason}`;
  return word;
}

/** Tone for an outcome chip: what happened, what could have, what went wrong. */
export function outcomeTone(outcome) {
  if (['done', 'sent', 'undone'].includes(outcome)) return 'done';
  if (['would_have', 'offered', 'staged', 'pending', 'undo_pending'].includes(outcome)) return 'quiet';
  if (['failed', 'wrong_match', 'approval_lapsed'].includes(outcome)) return 'bad';
  if (['held', 'skipped', 'expired'].includes(outcome)) return 'held';
  if (Object.hasOwn(OUTCOME_LABEL, outcome)) return 'quiet';
  return 'unknown';
}

/** A rate as a percentage; `null`/missing (an empty denominator) is a dash. */
export function pct(rate) {
  if (rate === null || rate === undefined || !Number.isFinite(rate)) return '—';
  return `${Math.round(rate * 100)}%`;
}

export const verbOf = (row) => (typeof row?.action === 'object' ? row?.action?.verb : row?.action) ?? 'unknown';

const VERDICT_LABEL = {
  not_applicable: 'header match',
  snippet: 'snippet',
  full_body: 'full body',
  unknown: 'unknown evidence',
};
export const verdictLabel = (v) => VERDICT_LABEL[v] ?? VERDICT_LABEL.unknown;

/**
 * A group's statistics buckets that hold anything, in the store's order.
 * Never pooled (§8): each `verdict_input` is its own line, and an empty one
 * is omitted rather than shown as zero.
 */
export function measuredBuckets(group) {
  return (group?.statistics ?? []).filter((b) => (b.matched_actions ?? 0) > 0 || (b.automatic_mailbox_actions ?? 0) > 0);
}

const key = (d) => JSON.stringify(d ?? null);

/**
 * The ledger split by match digest, per group (§8): rows under the group's
 * current digest first, then each earlier digest under a heading dated by
 * its newest row. Rows keep their order (newest first). Rule-level rows
 * with no group — an approval lapse — stay with the current rows, where the
 * change they report is read.
 */
export function digestSections(rows, groups) {
  // A group with no current digest (its definition does not load) has no
  // "before": every row of it stays current rather than being misfiled.
  const current = new Map((groups ?? []).filter((g) => g.match_digest != null).map((g) => [g.group, key(g.match_digest)]));
  const now = [];
  const earlier = new Map();
  for (const row of rows ?? []) {
    const cur = current.get(row.group);
    if (row.outcome === 'approval_lapsed' || cur === undefined || key(row.match_digest) === cur) {
      now.push(row);
      continue;
    }
    const k = `${row.group}\u0000${key(row.match_digest)}`;
    if (!earlier.has(k)) earlier.set(k, { group: row.group, newest: row.at ?? null, rows: [] });
    earlier.get(k).rows.push(row);
  }
  return { current: now, earlier: [...earlier.values()] };
}

/** Message line for a ledger row: sender and subject when held, else the pointer. */
export function messageLine(row) {
  // `||`, not `??`: the ledger view nulls empty strings, a test report's rows
  // (straight from ApplyReport) do not, and a blank line is no subject at all.
  const who = row?.sender || null;
  const what = row?.subject || null;
  if (who || what) return { who: who ?? 'unknown sender', what: what ?? '(no subject)' };
  return { who: row?.account ?? 'unknown account', what: row?.thread_id ? `thread ${row.thread_id}` : 'message unknown' };
}

/** "matched because" from the row's serialized matcher: the keys that were set. */
export function matchedBecause(row) {
  let m;
  try {
    m = typeof row?.matched_because === 'string' ? JSON.parse(row.matched_because) : row?.matched_because;
  } catch {
    return row?.matched_because || '';
  }
  if (!m || typeof m !== 'object') return '';
  const parts = [];
  for (const [k, v] of Object.entries(m)) {
    if (k === 'header' && v && typeof v === 'object') {
      for (const [h, vals] of Object.entries(v)) if (vals?.length) parts.push(`${h}: ${vals.join(', ')}`);
    } else if (Array.isArray(v) && v.length) {
      parts.push(`${k.replaceAll('_', ' ')}: ${v.join(', ')}`);
    } else if (typeof v === 'boolean') {
      parts.push(`${k.replaceAll('_', ' ')}: ${v ? 'yes' : 'no'}`);
    } else if (typeof v === 'string' && v) {
      parts.push(`${k}: ${v}`);
    }
  }
  return parts.join(' · ');
}

// ---- new rules ----

/** The address in a From display (`Name <a@b>` or a bare address), lowercased. */
export function addressOf(from) {
  const s = (from ?? '').trim();
  const m = s.match(/<([^<>\s]+@[^<>\s]+)>\s*$/);
  const a = (m ? m[1] : s).trim().toLowerCase();
  return /^[^@\s]+@[^@\s]+$/.test(a) ? a : null;
}

export const domainOf = (address) => (address ? address.split('@')[1] : null);

/** A valid store name (lowercase, digits, `-`, `_`; leads alphanumeric; ≤ 64). */
export function slug(text) {
  const s = (text ?? '')
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^[-_]+|[-_]+$/g, '')
    .slice(0, 64)
    .replace(/[-_]+$/, '');
  return s || 'new-rule';
}

/** Names the page's own routes take (`RESERVED_RULE_NAMES` in the store). */
export const RESERVED = ['add', 'ledger', 'pause-all', 'undo', 'wrong-match'];

/**
 * Prefill from a thread (§8 "Make a rule from this thread"): sender, its
 * domain, and a suggested archive — the recoverable action. No model, and
 * no read the reader has not already made: the list id is not in what the
 * mail views hold, so it stays for the owner to add.
 */
export function prefillFromThread({ from, subject } = {}) {
  const address = addressOf(from);
  const domain = domainOf(address);
  return {
    name: slug(domain ? domain.replace(/\./g, '-') : 'new-rule'),
    description: domain ? `Mail from ${domain}` : (subject ?? '').slice(0, 80) || 'New rule',
    match: 'from_domain',
    from: address ?? '',
    from_domain: domain ?? '',
    list_id: '',
    subject_contains: '',
    verb: 'archive',
  };
}

export const DRAFT_VERBS = ['archive', 'read', 'tag', 'digest', 'trash'];

// A JSON string literal is a valid TOML basic string: TOML accepts every
// escape JSON.stringify emits (\" \\ \b \f \n \r \t \uXXXX).
const tomlString = (s) => JSON.stringify(String(s));
const tomlList = (csv) =>
  `[${csv
    .split(',')
    .map((v) => v.trim())
    .filter(Boolean)
    .map(tomlString)
    .join(', ')}]`;

/**
 * rule.toml from the guided form. Only the chosen matcher key is written,
 * so a draft never ANDs keys the owner did not mean to combine; the owner
 * edits the text before installing.
 */
export function draftToml(d) {
  const lines = [`description = ${tomlString(d.description || 'New rule')}`, '', '[match]'];
  const value = (d[d.match] ?? '').trim();
  if (value) lines.push(`${d.match} = ${tomlList(value)}`);
  lines.push('', '[[action]]', `do = ${tomlString(d.verb)}`);
  if (d.verb === 'tag') lines.push(`tag = ${tomlString(d.tag || 'rule')}`);
  return `${lines.join('\n')}\n`;
}

/** What is wrong with a rule name, or null — the store's own grammar. */
export function nameProblem(name) {
  if (!/^[a-z0-9][a-z0-9_-]{0,63}$/.test(name ?? '')) return 'Name: lowercase letters, digits, - and _, starting with a letter or digit.';
  if (RESERVED.includes(name)) return `“${name}” is reserved by this page; choose another name.`;
  return null;
}

/** What is wrong with a guided draft before it is sent, or null. The store decides the rest. */
export function draftProblem(name, d) {
  return nameProblem(name) ?? ((d[d.match] ?? '').trim() ? null : 'Give the rule something to match on.');
}

/**
 * The server's refusal: `{code, message, body}` from the typed JSON
 * (`{ok:false, error:{code,message}}`, 409), else the plain stderr line the
 * route returned, else the status. `code` is what the page branches on —
 * `stale_digest` reloads the approval screen rather than retrying it.
 */
export function errorOf(status, text) {
  let v = null;
  try {
    v = JSON.parse(text);
  } catch {
    // plain text: a stderr line or a gate's refusal
  }
  if (v && typeof v === 'object') {
    const message = v.error?.message ?? v.stderr ?? null;
    return { code: v.error?.code ?? null, message: message || `HTTP ${status}`, body: v };
  }
  return { code: null, message: (text ?? '').trim() || `HTTP ${status}`, body: null };
}

/** A short "when": today as a time, this year as a date, else a full date. */
export function when(iso, now = new Date()) {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '—';
  if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  if (d.getFullYear() === now.getFullYear()) return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
  return d.toLocaleDateString([], { year: 'numeric', month: 'short', day: 'numeric' });
}
