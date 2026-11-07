// Behaviour checks for the #mail/rules page's logic (src/lib/mail-rules.js).
//
// `npm test` in web/. Plain node, like the other files here.
//
// **What is worth pinning.** The store's two wire-format rules, as the page
// shows them: a value this build cannot name is `unknown`, never the nearest
// known word, and a rate over an empty denominator is a dash, never 0%
// (MAIL-RULES-DESIGN.md §8). And the drafts a person installs from the page
// are TOML the store parses — a quote in a description must not end it.
import {
  modeOf,
  isRaise,
  outcomeLabel,
  outcomeTone,
  pct,
  groupLabel,
  verbOf,
  measuredBuckets,
  digestSections,
  messageLine,
  matchedBecause,
  addressOf,
  slug,
  prefillFromThread,
  draftToml,
  draftProblem,
  errorOf,
  editNotice,
  pauseAllNotice,
  when,
  RULES_ENDPOINTS,
} from '../src/lib/mail-rules.js';
import { readFileSync } from 'node:fs';

let pass = 0;
let fail = 0;
const t = (name, cond) => {
  if (cond) {
    pass += 1;
    console.log('  ok   ', name);
  } else {
    fail += 1;
    console.log('  FAIL ', name);
  }
};

console.log('mail-rules.js');

// ---- unknown stays unknown ----
t('a known mode is itself', modeOf('auto') === 'auto' && modeOf('off') === 'off');
t('a newer build’s mode is unknown, not the nearest rung', modeOf('graduated') === 'unknown' && modeOf(undefined) === 'unknown');
t('an unknown outcome reads unknown', outcomeLabel({ outcome: 'escalated' }) === 'unknown');
t('an unknown outcome has the unknown tone', outcomeTone('escalated') === 'unknown');
t('an unknown group reads unknown', groupLabel('carrier_pigeon') === 'unknown');
t('a missing action verb reads unknown', verbOf({}) === 'unknown' && verbOf({ action: { verb: 'archive' } }) === 'archive');
t('skipped rows say which guard', outcomeLabel({ outcome: 'skipped', reason: 'sender unknown' }) === 'skipped: sender unknown');
t('wrong match is a known outcome', outcomeLabel({ outcome: 'wrong_match' }) === 'wrong match');

// ---- raises take the approval path ----
t('shadow → auto raises', isRaise('shadow', 'auto'));
t('auto → shadow lowers', !isRaise('auto', 'shadow'));
t('an unknown current mode is treated as a raise', isRaise('graduated', 'shadow'));
t('an unknown target is never a raise the page sends', !isRaise('shadow', 'graduated'));

// ---- a rate over nothing is a dash ----
t('null rate is a dash', pct(null) === '—');
t('missing rate is a dash', pct(undefined) === '—');
t('zero is zero, when it was measured', pct(0) === '0%');
t('a measured rate rounds', pct(0.125) === '13%');
t('NaN never prints', pct(NaN) === '—');

// ---- measurements are never pooled ----
const group = {
  group: 'mailbox',
  match_digest: { keys: 'current' },
  statistics: [
    { verdict_input: 'not_applicable', matched_actions: 4, automatic_mailbox_actions: 2, undo_rate: 0.5 },
    { verdict_input: 'snippet', matched_actions: 0, automatic_mailbox_actions: 0, undo_rate: null },
    { verdict_input: 'full_body', matched_actions: 0, automatic_mailbox_actions: 0, undo_rate: null },
    { verdict_input: 'unknown', matched_actions: 1, automatic_mailbox_actions: 0, undo_rate: null },
  ],
};
const buckets = measuredBuckets(group);
t('empty buckets are omitted, not shown as zero', buckets.length === 2);
t('each evidence kind keeps its own line', buckets[0].verdict_input === 'not_applicable' && buckets[1].verdict_input === 'unknown');

// ---- earlier match digests stay apart ----
const rows = [
  { id: 'a', group: 'mailbox', match_digest: { keys: 'current' }, at: '2026-11-05T10:00:00Z', outcome: 'done' },
  { id: 'b', group: 'mailbox', match_digest: { keys: 'older' }, at: '2026-11-03T10:00:00Z', outcome: 'done' },
  { id: 'c', group: 'mailbox', match_digest: { keys: 'older' }, at: '2026-11-02T10:00:00Z', outcome: 'would_have' },
  { id: 'd', outcome: 'approval_lapsed', at: '2026-11-04T10:00:00Z' },
];
const sections = digestSections(rows, [group]);
t('current-digest rows lead', sections.current.map((r) => r.id).join() === 'a,d');
t('an earlier digest is its own section', sections.earlier.length === 1 && sections.earlier[0].rows.length === 2);
t('its heading is dated by its newest row', sections.earlier[0].newest === '2026-11-03T10:00:00Z');

// ---- message lines never invent a sender ----
t('sender and subject when the ledger holds them', messageLine({ sender: 'a@example.com', subject: 'Hi' }).who === 'a@example.com');
t('the pointer when it does not', messageLine({ account: 'campus', thread_id: 't1' }).what === 'thread t1');

// ---- matched because ----
t(
  'matcher keys that were set, and only those',
  matchedBecause({ matched_because: JSON.stringify({ from_domain: ['example.com'], to: [], header: { 'X-Notice': ['update'] } }) }) ===
    'from domain: example.com · X-Notice: update',
);
t('an unparsable matcher shows its text', matchedBecause({ matched_because: 'list-id' }) === 'list-id');

// ---- prefill from a thread ----
t('address from a display form', addressOf('Build Bot <Builds@Example.com>') === 'builds@example.com');
t('a bare address', addressOf('notices@example.edu') === 'notices@example.edu');
t('no address is null, never a guess', addressOf('Someone') === null);
t('a slug is a valid store name', slug('News.Example.COM!!') === 'news-example-com');
t('an empty slug still names something', slug('***') === 'new-rule');
const pre = prefillFromThread({ from: 'Notices <notices@lists.example.edu>', subject: 'Weekly' });
t('prefill suggests archive, the recoverable action', pre.verb === 'archive');
t('prefill matches on the sender domain', pre.match === 'from_domain' && pre.from_domain === 'lists.example.edu');
t('prefill names the rule after the domain', pre.name === 'lists-example-edu');
t('prefill leaves the list id to the owner', pre.list_id === '');

// ---- drafts are TOML the store can parse ----
const toml = draftToml({ ...pre, description: 'Say "hi"\\now' });
t('a quote in the description stays inside its string', toml.includes('description = "Say \\"hi\\"\\\\now"'));
t('only the chosen matcher key is written', toml.includes('from_domain = ["lists.example.edu"]') && !toml.includes('from ='));
t('the action is written', toml.includes('[[action]]\ndo = "archive"'));
t('a list splits on commas', draftToml({ ...pre, match: 'from', from: 'a@example.com, b@example.com' }).includes('from = ["a@example.com", "b@example.com"]'));
t('a tag action carries its tag', draftToml({ ...pre, verb: 'tag', tag: 'lists' }).includes('tag = "lists"'));
t('a bad name is refused before sending', draftProblem('Bad Name', pre) !== null);
t('a reserved name is refused before sending', /reserved/.test(draftProblem('ledger', pre)));
t('a draft with nothing to match is refused', draftProblem('ok', { ...pre, from_domain: '' }) !== null);
t('a good draft passes', draftProblem('lists-example-edu', pre) === null);
t('a tag action with no tag is refused, never given a default', draftProblem('ok', { ...pre, verb: 'tag', tag: ' ' }) !== null);

// ---- refusals ----
const stale = errorOf(409, JSON.stringify({ ok: false, error: { code: 'stale_digest', message: 'the rule changed since it was shown; reload' } }));
t('a typed refusal keeps its code', stale.code === 'stale_digest' && /reload/.test(stale.message));
t('a plain stderr line is the message', errorOf(502, 'unknown rule\n').message === 'unknown rule');
t('an empty body is the status', errorOf(500, '').message === 'HTTP 500');

// ---- the page names only the paths the docs demo is checked for ----
// MailRules.svelte calls through wrappers check-demo.mjs cannot scan, so it
// reads RULES_ENDPOINTS instead; a path missing from that list would make the
// guard's pass vacuous again (review of #850).
const page = readFileSync(new URL('../src/lib/MailRules.svelte', import.meta.url), 'utf8');
const used = [...page.matchAll(/[`'"](\/api\/mail\/rules[^`'"]*)/g)].map(([, p]) => p.split('?')[0].replace(/\$\{[^}]*\}/g, 'X'));
t('the page reaches rules paths at all (the scan is not vacuous)', used.length >= 10);
t('every rules path the page reaches is in RULES_ENDPOINTS', used.every((p) => RULES_ENDPOINTS.includes(p)));

// ---- review of #850 ----
t('an empty subject from a test row is "(no subject)", not a blank', messageLine({ sender: 'a@example.com', subject: '' }).what === '(no subject)');
t('an empty sender is never shown blank', messageLine({ sender: '', subject: 'Hi' }).who === 'unknown sender');
const unreadable = digestSections([{ id: 'x', group: 'mailbox', match_digest: { keys: 'any' }, outcome: 'done' }], [{ group: 'mailbox', match_digest: null }]);
t('a group with no current digest files nothing under "before the matcher changed"', unreadable.current.length === 1 && unreadable.earlier.length === 0);

// ---- review of #850, pass 4: replies say what the store did ----
const outside = editNotice({ changed_files: [], lapsed_groups: ['mailbox'] });
t('a save that changed no bytes but lapsed an approval says so, never "no change"', outside.tone === 'held' && /lapsed for Mailbox/.test(outside.text) && !/No change/.test(outside.text));
t('a save that changed bytes and lapsed says both', /^Saved\. Approval lapsed/.test(editNotice({ changed_files: ['rule.toml'], lapsed_groups: ['report'] }).text));
t('no bytes and no lapse is no change', editNotice({ changed_files: [], lapsed_groups: [] }).text === 'No change to save.');
t('a plain save is saved', editNotice({ changed_files: ['rule.toml'], lapsed_groups: [] }).text === 'Saved.');
const skipped = pauseAllNotice({ groups: 3, notices: ['broken-rule: unreadable, not paused'] });
t('pause all names what it could not pause', skipped.tone === 'bad' && /broken-rule/.test(skipped.text));
t('pause all with nothing skipped is plain', pauseAllNotice({ groups: 1, notices: [] }).text === 'Paused 1 group to shadow.');

// ---- description rules (P4a) ----
const described = matchedBecause({ matched_because: JSON.stringify({ from_domain: ['example.edu'], describe: 'Room booking confirmations', quote: 'your booking for Friday' }) });
t('a description row leads with the quoted words', described === '“your booking for Friday” · from domain: example.edu');
t('the owner’s own sentence is not repeated as a reason', !/Room booking/.test(described));
const ddraft = { ...pre, match: 'describe', describe: 'Confirmations, of a room booking' };
t('a description is one TOML string, never split on commas', draftToml(ddraft).includes('describe = "Confirmations, of a room booking"'));
t('a description draft passes', draftProblem('bookings', ddraft) === null);
t('an empty description is refused', draftProblem('bookings', { ...ddraft, describe: '  ' }) !== null);
t('an over-long description is refused before sending', /at most/.test(draftProblem('bookings', { ...ddraft, describe: 'x'.repeat(501) })));

// ---- when ----
const now = new Date('2026-11-06T15:00:00');
t('no time is a dash', when(null, now) === '—' && when('nonsense', now) === '—');
t('an older year carries the year', /2024/.test(when('2024-03-01T10:00:00', now)));

console.log(`\n${pass} passed, ${fail} failed`);
if (fail) process.exit(1);
