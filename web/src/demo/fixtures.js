// The invented box behind the documentation demo.
//
// Every name, mailbox, thread and note in this file is fiction. That is not a
// stylistic choice: this repository is public, the docs site embeds this build
// at <https://docs.mecha-factory.ai/demo/>, and the real surface is a view onto
// one person's mail, calendar and knowledge graph. There is no redaction pass
// that could make a real screenshot safe, so the demo never starts from one.
//
// The *shapes* here are not invented. Each one was read off the handler that
// serves it (`mecha-cli/src/commands/serve/`) and the component that renders it
// (`web/src/lib/`), so a field that appears here appears because something
// draws it. `npm run check-demo` in the docs site re-checks the route list
// against the router, which is the half a person forgets.
//
// The cast, once, so it stays consistent across pages:
//
//   Priya Raghavan   a postdoc in the owner's group
//   Tomas Lindqvist  an editor at the Journal of Applied Cognition
//   Amara Osei       a collaborator in Cape Town
//   Wen Li           a second-year graduate student
//   Hollis Barnett   the department's administrator
//   Fairhaven        the university; the Ostrander Prize is its award

export const OWNER = 'demo@example.com';

// --- the dashboard -------------------------------------------------------

export const summary = {
  owner: OWNER,
  // All eight rows `collect_queues()` pushes, in its order, with its `opens`
  // strings verbatim. The depths and details are invented; the names and the
  // commands are not, because the home page now *prints* `opens` on any queue
  // with no page behind it — a fixture that made one up would be teaching the
  // demo's reader a command that does not exist.
  queues: [
    {
      queue: 'graph candidates',
      depth: 12,
      detail: 'from last night’s extraction',
      opens: 'mecha review list',
    },
    {
      queue: 'graph entities',
      depth: 4,
      detail: '2 detector(s) with something to say',
      opens: 'mecha-graph proposals list',
    },
    {
      queue: 'graph shadow',
      depth: 2,
      detail: '31 unreviewed facts live, 2 ever served',
      opens: 'mecha review shadow',
    },
    {
      queue: 'outbox drafts',
      depth: 3,
      detail: '2 replies, 1 calendar hold',
      opens: 'mecha outbox',
    },
    {
      queue: 'blocked questions',
      depth: 1,
      detail: '1 asked with third-party content in the conversation',
      opens: 'mecha questions',
    },
    {
      queue: 'front-door requests',
      depth: 1,
      detail: 'a speaking invitation, extracted',
      opens: 'mecha frontdoor list',
    },
    {
      queue: 'rule proposals',
      depth: 1,
      detail: '1 retirement proposed',
      opens: 'mecha proposals',
    },
    {
      queue: 'harness changes',
      depth: 0,
      detail: 'nothing accepted since Tuesday',
      opens: 'mecha harness list',
    },
  ],
  // A finding rather than an empty list, because a dashboard that can only
  // ever say "fine" is a dashboard nobody believes.
  doctor: [
    {
      component: 'mail',
      summary: 'the personal account’s token expires in 6 days',
    },
  ],
  errors: [],
};

// --- mail ----------------------------------------------------------------

export const mail = [
  {
    thread_id: 'thr-8812',
    account: 'work',
    from: 'Tomas Lindqvist',
    subject: 'Review request — manuscript JAC-2291',
    summary:
      'Asks whether you can review a 9,000-word manuscript on retrieval practice. Wants an answer by Friday; the review itself would be due in three weeks.',
    needs_me: true,
    urgency: 'soon',
    state: 'triaged',
    tags: ['review', 'deadline'],
    deadline: '2026-09-04',
  },
  {
    thread_id: 'thr-8809',
    account: 'work',
    from: 'Hollis Barnett',
    subject: 'Ostrander Prize nominations close Monday',
    summary:
      'The department needs one nomination letter per faculty member. You told Hollis in March you would nominate someone from the group.',
    needs_me: true,
    urgency: 'soon',
    state: 'triaged',
    tags: ['admin', 'promised'],
    deadline: '2026-09-01',
  },
  {
    thread_id: 'thr-8804',
    account: 'personal',
    from: 'Amara Osei',
    subject: 'Re: the Cape Town visit in November',
    summary:
      'Confirms the 12th–16th works on her side and asks whether you want the seminar slot on the Thursday or the Friday.',
    needs_me: true,
    urgency: 'whenever',
    state: 'triaged',
    tags: ['travel'],
    deadline: null,
  },
  {
    thread_id: 'thr-8799',
    account: 'work',
    from: 'Wen Li',
    subject: 'draft of chapter 2 — no rush',
    summary:
      'Sends a chapter draft and says explicitly there is no deadline. Parked until the review decision is out of the way.',
    needs_me: false,
    urgency: null,
    state: 'parked',
    tags: ['students'],
    deadline: null,
  },
  {
    thread_id: 'thr-8791',
    account: 'work',
    from: 'Fairhaven IT',
    subject: 'Scheduled maintenance, Sunday 02:00–06:00',
    summary: 'Notification only. Nothing is asked of you.',
    needs_me: false,
    urgency: null,
    state: 'triaged',
    tags: ['noise'],
    deadline: null,
  },
];

export const mailInbox = [
  {
    thread_id: 'thr-8812',
    account: 'work',
    from: 'Tomas Lindqvist',
    subject: 'Review request — manuscript JAC-2291',
    summary: 'Journal of Applied Cognition · 14:02',
    needs_me: true,
    urgency: 'soon',
    state: 'triaged',
    tags: [],
    deadline: null,
  },
  {
    thread_id: 'thr-8811',
    account: 'personal',
    from: 'Fairhaven Library',
    subject: 'Your hold is ready for collection',
    summary: 'Today · 13:41',
    needs_me: false,
    urgency: null,
    state: 'triaged',
    tags: [],
    deadline: null,
  },
  {
    thread_id: 'thr-8809',
    account: 'work',
    from: 'Hollis Barnett',
    subject: 'Ostrander Prize nominations close Monday',
    summary: 'Today · 11:20',
    needs_me: true,
    urgency: 'soon',
    state: 'triaged',
    tags: [],
    deadline: null,
  },
];

export const mailRead = {
  'thr-8812': {
    subject: 'Review request — manuscript JAC-2291',
    meta: 'Tomas Lindqvist <editor@jac.example.org> · work · 2026-08-28 14:02',
    body: `Dear Professor,

Would you be willing to review the attached manuscript, "Spacing effects in
applied retrieval practice", for the Journal of Applied Cognition? It runs to
about 9,000 words.

I would need to know by Friday whether you can take it on. If you can, the
review itself would be due three weeks after that.

With thanks,
Tomas Lindqvist
Associate Editor`,
  },
};

// --- the outbox ----------------------------------------------------------

const draftReply = {
  id: 'ob-4417',
  tool: 'mail__mail_reply',
  kind: 'message',
  label: 'Reply',
  headline: 'Re: Review request — manuscript JAC-2291',
  snippet:
    'Thank you for thinking of me. I can take this one on — three weeks from Friday puts the review just before I travel, which works. Please send…',
  status: 'pending',
  created_at: '2026-08-29T07:41:12Z',
  tainted: true,
  edited: false,
};

const draftNomination = {
  id: 'ob-4418',
  tool: 'mail__mail_send',
  kind: 'message',
  label: 'New mail',
  headline: 'Ostrander Prize nomination — Priya Raghavan',
  snippet:
    'I am nominating Priya Raghavan for the Ostrander Prize. Priya joined the group in 2024 and has since led the replication effort that…',
  status: 'pending',
  created_at: '2026-08-29T07:41:48Z',
  tainted: false,
  edited: true,
};

const draftHold = {
  id: 'ob-4419',
  tool: 'mail__calendar_create',
  kind: 'call',
  label: 'Calendar',
  headline: 'Seminar — Cape Town (hold)',
  snippet: 'account: personal · duration: 90 minutes',
  status: 'pending',
  created_at: '2026-08-29T07:42:03Z',
  tainted: false,
  edited: false,
};

export const outbox = {
  pending: [draftReply, draftNomination, draftHold],
  // A count, not a list: the pane renders `{pending.length} pending ·
  // {resolved} resolved` and never draws the resolved rows.
  resolved: 2,
};

// Kept for the record of what those two were, though nothing renders them.
export const outboxResolved = [
    {
      id: 'ob-4402',
      tool: 'mail__mail_reply',
      kind: 'message',
      label: 'Reply',
      headline: 'Re: draft of chapter 2 — no rush',
      snippet: 'Got it — I will read this properly next week rather than skim it now.',
      status: 'sent',
      created_at: '2026-08-28T18:12:40Z',
      tainted: false,
      edited: false,
    },
    {
      id: 'ob-4398',
      tool: 'mail__mail_send',
      kind: 'message',
      label: 'New mail',
      headline: 'Re: conference travel budget',
      snippet: 'Rejected — the figure was wrong and I would rather write this one myself.',
      status: 'rejected',
    created_at: '2026-08-28T09:03:11Z',
    tainted: false,
    edited: false,
  },
];

export const outboxDetail = {
  'ob-4417': {
    id: 'ob-4417',
    tool: 'mail__mail_reply',
    label: 'Reply',
    headline: 'Re: Review request — manuscript JAC-2291',
    kind: 'message',
    status: 'pending',
    created_at: '2026-08-29T07:41:12Z',
    summary: 'mail_reply to thr-8812',
    // Both flags set: the run read the owner's calendar and a stranger's mail
    // in one conversation. The card says so, and the send stayed staged.
    taint: { private: true, untrusted: true, armed: true },
    headers: [
      ['to', 'Tomas Lindqvist <editor@jac.example.org>'],
      ['subject', 'Re: Review request — manuscript JAC-2291'],
      ['account', 'work'],
    ],
    body: `Dear Tomas,

Thank you for thinking of me. I can take this one on — three weeks from Friday puts the review just before I travel, which works.

Please send the manuscript when you are ready.

Best wishes`,
    other: [['thread_id', 'thr-8812']],
    edited: false,
    error: null,
    args: {
      thread_id: 'thr-8812',
      account: 'work',
      body_markdown: 'Dear Tomas,\n\nThank you for thinking of me…',
    },
    session_id: '20260829T074002-a91c',
    sources: [
      {
        tool: 'mail__mail_read',
        keys: ['thread_id'],
        heading: 'read mail__mail_read(thread_id: thr-8812)',
        join: 'returned',
        text: 'Would you be willing to review the attached manuscript… I would need to know by Friday whether you can take it on.',
      },
      {
        tool: 'mail__calendar_list',
        keys: ['from', 'to'],
        heading: 'asked mail__calendar_list(from: 2026-09-19, to: 2026-09-26)',
        join: 'asked',
        text: 'Sep 22–26 — Cape Town (tentative). Nothing else held that week.',
      },
    ],
  },
  'ob-4418': {
    id: 'ob-4418',
    tool: 'mail__mail_send',
    label: 'New mail',
    headline: 'Ostrander Prize nomination — Priya Raghavan',
    kind: 'message',
    status: 'pending',
    created_at: '2026-08-29T07:41:48Z',
    summary: 'mail_send to awards@fairhaven.example.edu',
    taint: { private: true, untrusted: false, armed: false },
    headers: [
      ['to', 'awards@fairhaven.example.edu'],
      ['subject', 'Ostrander Prize nomination — Priya Raghavan'],
      ['account', 'work'],
    ],
    body: `I am nominating Priya Raghavan for the Ostrander Prize.

Priya joined the group in 2024 and has since led the replication effort that became the lab's most-cited output of the year. She did the unglamorous half of that work — the pre-registration, the analysis plan, the two failed pilots — and then wrote it up honestly.`,
    other: [],
    edited: true,
    error: null,
    args: { to: 'awards@fairhaven.example.edu', account: 'work' },
    session_id: '20260829T074002-a91c',
    sources: [],
  },
  'ob-4419': {
    id: 'ob-4419',
    tool: 'mail__calendar_create',
    label: 'Calendar',
    headline: 'Seminar — Cape Town (hold)',
    kind: 'call',
    status: 'pending',
    created_at: '2026-08-29T07:42:03Z',
    summary: 'calendar_create',
    taint: { private: true, untrusted: true, armed: true },
    headers: [
      ['title', 'Seminar — Cape Town (hold)'],
      ['starts', 'Thursday 12 November, 15:00 SAST'],
      ['ends', 'Thursday 12 November, 16:30 SAST'],
    ],
    body: null,
    other: [
      ['account', 'personal'],
      ['location', 'Department of Psychology, seminar room 2'],
    ],
    edited: false,
    // A staged call that has already failed once, with the reason on the card
    // rather than two fields away in the store.
    error: null,
    args: { account: 'personal' },
    session_id: '20260829T074002-a91c',
    sources: [],
  },
};

// --- the front door ------------------------------------------------------

export const frontdoor = {
  requests: [
  {
    seq: 41,
    type_id: 'speaking',
    topic: 'Keynote — Nordic Cognition Meeting, June 2027',
    state: 'open',
    created_at: '2026-08-29T05:14:00Z',
    urgency_claimed: 'high',
    valid: true,
      extraction_error: null,
      reading: null,
    },
  ],
};

// --- the graph review queue ---------------------------------------------

export const queue = [
  {
    proposer: 'nightly-extractor',
    pending: 9,
    accepted_hist: 212,
    rejected_hist: 46,
    tier: 'solid',
  },
  {
    proposer: 'mail-distiller',
    pending: 3,
    accepted_hist: 18,
    rejected_hist: 11,
    tier: 'some',
  },
];

// --- notes ---------------------------------------------------------------

export const notes = {
  notes: [
    {
      uid: 'note-1181',
      body: 'Told Hollis in March I would nominate someone for the Ostrander Prize. Should be Priya — the replication work was hers end to end.',
      occurred_at: '2026-08-29T07:12:00Z',
    },
    {
      uid: 'note-1180',
      body: 'Amara wants a decision on Thursday vs Friday for the Cape Town seminar. Thursday is better: Friday I would be flying the same evening.',
      occurred_at: '2026-08-28T19:40:00Z',
    },
    {
      uid: 'note-1178',
      body: 'Wen asked whether the second study needs its own pre-registration. It does. Write this down somewhere the group can find it.',
      occurred_at: '2026-08-27T11:05:00Z',
    },
  ],
};

export const find = {
  results: [
    {
      kind: 'person',
      name: 'Priya Raghavan',
      statement: 'Postdoc in the group since 2024; led the retrieval-practice replication.',
      subject: 'Priya Raghavan',
      text: null,
    },
    {
      kind: 'commitment',
      name: null,
      statement: 'Promised Hollis Barnett a prize nomination — March 2026.',
      subject: 'Ostrander Prize',
      text: null,
    },
  ],
};

// --- the task board ------------------------------------------------------

export const tasks = {
  items: [
    {
      id: 'tsk-3301',
      name: 'Answer Tomas about the JAC review',
      status: 'next',
      project: 'service',
      context: '@desk',
      due_at: '2026-09-04',
      overdue: false,
      waiting_on: null,
      captured_from: 'mail thr-8812',
      session: null,
      run: null,
    },
    {
      id: 'tsk-3302',
      name: 'Write the Ostrander nomination for Priya',
      status: 'waiting',
      project: 'service',
      context: '@writing',
      due_at: '2026-09-01',
      overdue: false,
      // The agent is the one holding it: a delegated run is in flight.
      waiting_on: 'mecha',
      captured_from: 'note-1181',
      session: '20260829T074002-a91c',
      run: {
        recorded: true,
        cut_short: false,
        turns: 6,
        tool_calls: 9,
        tool_errors: 0,
        tool_denied: 0,
        tool_staged: 1,
        stop_cause: null,
        ended_on_failed_call: false,
      },
    },
    {
      id: 'tsk-3303',
      name: 'Pick Thursday or Friday for the Cape Town seminar',
      status: 'next',
      project: 'travel',
      context: '@anywhere',
      due_at: null,
      overdue: false,
      waiting_on: null,
      captured_from: 'mail thr-8804',
      session: null,
      run: null,
    },
    {
      id: 'tsk-3298',
      name: 'Read Wen’s chapter 2 properly',
      status: 'scheduled',
      project: 'students',
      context: '@reading',
      due_at: '2026-09-08',
      overdue: false,
      waiting_on: null,
      captured_from: null,
      session: null,
      run: null,
    },
    {
      id: 'tsk-3290',
      name: 'Book the Cape Town flights',
      status: 'waiting',
      project: 'travel',
      context: '@admin',
      due_at: null,
      overdue: false,
      waiting_on: 'Amara Osei',
      captured_from: null,
      session: null,
      run: null,
    },
    {
      id: 'tsk-3277',
      name: 'Send the revised budget to Hollis',
      status: 'done',
      project: 'admin',
      context: '@desk',
      due_at: null,
      overdue: false,
      waiting_on: null,
      captured_from: null,
      session: null,
      run: null,
    },
  ],
};

export const questions = {
  items: [
    {
      id: 'q-77',
      qid: 'q-77',
      task: 'tsk-3302',
      question:
        'The nomination asks for a named seconder. Priya’s co-author on the replication is Wen Li, but Wen is a student — should I put Amara Osei instead?',
      options: ['Amara Osei', 'Wen Li', 'leave it blank'],
      handle: '20260829T074002-a91c',
      session: '20260829T074002-a91c',
      tainted: false,
    },
  ],
};

// --- settings ------------------------------------------------------------

export const charter = {
  // The file's real shape: `[[line]]` tables with `id` and `text`, header
  // comments above them. `CharterLine` denies unknown fields, so the
  // `[[priority]]`/`name`/`detail` this fixture used to carry is a document
  // the product refuses — a docs reader who opened the TOML editor was shown
  // TOML that could never have been saved.
  raw: `# What mecha is for, most important first.
#
# Order is rank: when two lines conflict, the higher one wins outright.
# There is no priority field — moving a line is how you re-rank it.

[[line]]
id = "protect-the-groups-people"
text = "Students and postdocs come before service work. A deadline that costs me an evening is cheaper than one that costs Wen a week."

[[line]]
id = "keep-promises-i-actually-made"
text = "If the graph records that I said I would do something, that outranks anything that merely arrived in the inbox."

[[line]]
id = "say-no-early"
text = "A refusal on Monday is a kindness. A refusal on Friday is a problem I handed to someone else."
`,
  template: '',
  exists: true,
  path: '~/.mecha/charter.toml',
  // `id` is the line's slug, as the route derives it from the name — a bare
  // ordinal here renders as "1. 1" beside the list marker, which is not what
  // the page does on a box.
  lines: [
    {
      id: 'protect-the-groups-people',
      text: 'Students and postdocs come before service work. A deadline that costs me an evening is cheaper than one that costs Wen a week.',
    },
    {
      id: 'keep-promises-i-actually-made',
      text: 'If the graph records that I said I would do something, that outranks anything that merely arrived in the inbox.',
    },
    {
      id: 'say-no-early',
      text: 'A refusal on Monday is a kindness. A refusal on Friday is a problem I handed to someone else.',
    },
  ],
  // What `Charter::char_count` renders for the three lines above —
  // `prompt_block(..).chars().count()`, not the file length. Measured by
  // building a `Charter` from those lines and printing `char_count()`.
  //
  // Deliberately ungated, unlike the TOML shape beside it: nothing in CI
  // re-derives this, so editing `prompt_block`'s header prose will drift it
  // silently. Re-measure rather than adjust by eye if that happens.
  char_count: 716,
  budget: 2000,
  over_budget: false,
  parse_error: null,
  error: null,
};

// A learned rule carries an `id` and a user rule does not — that difference
// is not decoration here. The settings page offers retire/restore only on a
// rule it can name, because both stores resolve by prefix and an empty needle
// matches every record; a fixture without ids would draw the rules pane with
// no verbs at all and quietly under-describe the surface.
export const rules = [
  {
    id: null,
    title: 'Never accept a review without checking the calendar first',
    domain: 'mail',
    active: true,
    retired: false,
    observations: 14,
    attributed_regressions: 0,
    user: true,
  },
  {
    id: null,
    title: 'Nominations and letters name the specific work, not the person’s qualities',
    domain: 'writing',
    active: true,
    retired: false,
    observations: 6,
    attributed_regressions: 0,
    user: true,
  },
  {
    id: 'r-20260812-4c1f9a02',
    title: 'Answer a thread before summarising it — the summary is not the reply',
    domain: 'mail',
    active: true,
    retired: false,
    // Measured and clean. The pane says "N probe(s), none attributed"; a rule
    // no probe has reached says so instead, which is the next one.
    observations: 9,
    attributed_regressions: 0,
    user: false,
  },
  {
    id: 'r-20260819-b7e3d144',
    title: 'Offer a specific slot rather than asking what works',
    domain: 'writing',
    active: true,
    retired: false,
    // Absent, not zero: nothing has ever measured this one, which is a
    // different finding from a rule that passed every probe.
    observations: null,
    attributed_regressions: null,
    user: false,
  },
  {
    id: 'r-20260722-91ac30de',
    title: 'Summarise threads before quoting them',
    domain: 'mail',
    active: false,
    retired: true,
    retired_reason: 'the bisection attributed three regressions to it',
    observations: 21,
    attributed_regressions: 3,
    user: false,
  },
];

// --- the learning store, one stage earlier -------------------------------
//
// A reflection is one lesson mined from one intervention, before anything
// consolidates several into a rule. The four here are the four states the
// pane has to draw differently, because each asks something different of the
// owner: one that can become a rule, one the provenance gate excludes (the
// case the edit verb exists for), one already rewritten in the owner's own
// words, and one refused and kept as evidence.
export const reflections = [
  {
    id: '20260826T143000-7f21a9c4',
    domain: 'mail',
    trigger: 'steer',
    title:
      'When a thread asks a direct question, answer it in the first line — the context belongs underneath, not in front.',
    origin: 'clean',
    learnable: true,
    blocked: null,
    edited: false,
    dropped: false,
    processed: false,
    created_at: '2026-08-26T14:30:00+00:00',
    session_id: '20260826T140012-3a8b91cc',
  },
  {
    id: '20260824T091500-2b4e77a1',
    domain: 'writing',
    trigger: 'denial',
    title: 'Check a nomination deadline against the calendar before promising a date.',
    origin: 'untrusted',
    learnable: false,
    blocked: 'third-party content was in context when it was mined (edit to make it yours)',
    edited: false,
    dropped: false,
    processed: false,
    created_at: '2026-08-24T09:15:00+00:00',
    session_id: '20260824T085500-11c2f0de',
  },
  {
    id: '20260821T171200-d90c4415',
    domain: 'behavior',
    trigger: 'followup',
    title: 'Say what a run could not finish before saying what it did.',
    origin: 'clean',
    learnable: true,
    blocked: null,
    edited: true,
    dropped: false,
    processed: true,
    created_at: '2026-08-21T17:12:00+00:00',
    session_id: '20260821T164400-5e7d2a03',
  },
  {
    id: '20260818T102400-6ac1b8f9',
    domain: 'behavior',
    trigger: 'followup',
    title: 'Prefer shorter replies in the evening.',
    origin: 'clean',
    learnable: false,
    blocked: 'dropped — that was one evening, not a rule',
    edited: false,
    dropped: true,
    processed: false,
    created_at: '2026-08-18T10:24:00+00:00',
    session_id: '20260818T100100-cc4419b7',
  },
];

// What `reflections show` returns: the lesson plus the evidence a refusal
// rests on. `context` is the field that holds third-party bytes, so the
// excluded record shows what an owner would actually be reading when they
// decide whether to adopt the lesson or drop it — and the edited one shows
// the withholding that a rewrite performs.
export const reflectionDetail = {
  '20260826T143000-7f21a9c4': {
    id: '20260826T143000-7f21a9c4',
    domain: 'mail',
    trigger: 'steer',
    reflexion_text:
      'When a thread asks a direct question, answer it in the first line — the context belongs underneath, not in front.',
    context: 'drafting a reply to Tomas Lindqvist about the review deadline',
    intervention: 'you buried the answer again — lead with it',
    provenance: 'clean',
    recorded_origin: 'clean',
    evidence: 'full',
    session_id: '20260826T140012-3a8b91cc',
    created_at: '2026-08-26T14:30:00+00:00',
  },
  '20260824T091500-2b4e77a1': {
    id: '20260824T091500-2b4e77a1',
    domain: 'writing',
    trigger: 'denial',
    reflexion_text: 'Check a nomination deadline against the calendar before promising a date.',
    context:
      '(withheld — the conversation held third-party content; the assistant was working with these tools: mail_read, calendar_list)',
    intervention: 'Denied by the user: the Ostrander deadline is the 30th, not the 13th',
    provenance: 'untrusted',
    recorded_origin: 'untrusted',
    evidence: 'user_turns',
    session_id: '20260824T085500-11c2f0de',
    created_at: '2026-08-24T09:15:00+00:00',
  },
  '20260821T171200-d90c4415': {
    id: '20260821T171200-d90c4415',
    domain: 'behavior',
    trigger: 'followup',
    reflexion_text: 'Say what a run could not finish before saying what it did.',
    context: '(withheld — the lesson was rewritten by the owner)',
    intervention: 'I had to read three paragraphs to find out the export failed',
    provenance: 'clean',
    recorded_origin: 'clean',
    evidence: 'user_turns',
    edited_at: '2026-08-21T18:02:00+00:00',
    session_id: '20260821T164400-5e7d2a03',
    created_at: '2026-08-21T17:12:00+00:00',
  },
  '20260818T102400-6ac1b8f9': {
    id: '20260818T102400-6ac1b8f9',
    domain: 'behavior',
    trigger: 'followup',
    reflexion_text: 'Prefer shorter replies in the evening.',
    context: 'wrapping up a long working session',
    intervention: 'that is plenty for tonight',
    provenance: 'clean',
    recorded_origin: 'clean',
    evidence: 'full',
    dropped_at: '2026-08-18T11:00:00+00:00',
    dropped_reason: 'that was one evening, not a rule',
    session_id: '20260818T100100-cc4419b7',
    created_at: '2026-08-18T10:24:00+00:00',
  },
};

export const voice = {
  worker_reachable: true,
  offer_target: 'http://127.0.0.1:8990/offer',
  cloned: [
    // `created` is unix seconds, as the route returns it — a date string here
    // renders 'Invalid Date' on the demo and nowhere else.
    { name: 'reading-voice', seconds: 42, created: 1786665600, length: 42 },
  ],
  cloned_error: null,
};

// --- chat ----------------------------------------------------------------

export const sessions = {
  sessions: [
    { key: 'main', running: false, title: 'morning triage', taint: { private: true, untrusted: true } },
    { key: 'w2', running: false, title: 'task: Ostrander nomination', taint: { private: true, untrusted: false } },
  ],
};

export const history = {
  sessions: [
    {
      id: '20260828T190412-77bd',
      snippet: 'walking home — talk me through the Cape Town dates',
      kind: 'voice',
      created_at: '2026-08-28T19:04:12Z',
      attached_key: null,
    },
    {
      id: '20260828T081955-2c04',
      snippet: 'what did I promise Hollis in March?',
      kind: 'chat',
      created_at: '2026-08-28T08:19:55Z',
      attached_key: null,
    },
  ],
};

// The conversation the demo opens on. It has already run once, so the page
// shows a transcript rather than an empty state — and the run it shows is one
// where the interlock did its job, because that is the thing worth seeing.
export const transcript = {
  key: 'main',
  path: '~/.mecha/work/web/main',
  running: false,
  started: '2026-08-29T07:40:02Z',
  steered: false,
  model: 'qwen3.6-35b-a3b',
  mode: 'read_only',
  taint: { private: true, untrusted: true },
  task: null,
  todo: [],
  questions: [],
  usage: { prompt_tokens: 24118, context_window: 32768 },
  entries: [
    {
      kind: 'user',
      text: 'What needs me this morning? Draft what you can.',
    },
    // A chip carries the call, not only its name: `draft` is the shaped
    // view, `args` the exact bytes behind it, `preview` what came back. A
    // reader who taps one on the docs site sees what an owner sees, which
    // is the whole point of embedding the app rather than a screenshot.
    {
      kind: 'tool',
      name: 'mail__mail_search',
      pending: false,
      is_error: false,
      draft: {
        headers: [['account', 'personal']],
        body: null,
        other: [
          ['max_results', '20'],
          ['query', 'is:unread newer_than:2d'],
        ],
      },
      args: '{\n  "query": "is:unread newer_than:2d",\n  "account": "personal",\n  "max_results": 20\n}',
      preview: `14 threads, newest first:

  Tomas Lindqvist   Review request — manuscript 2026-0413
  Hollis Barnett    Ostrander nomination closes Monday
  Amara Osei        Cape Town seminar — Thursday or Friday?
  … 11 more`,
    },
    {
      kind: 'tool',
      name: 'mail__calendar_list',
      pending: false,
      is_error: false,
      draft: {
        headers: [
          ['start', '2026-08-29'],
          ['end', '2026-09-19'],
        ],
        body: null,
        other: [['account', 'personal']],
      },
      args: '{\n  "account": "personal",\n  "start": "2026-08-29",\n  "end": "2026-09-19"\n}',
      preview: `4 events in 21 days:

  Sep 08–Sep 12  Cape Town (travel)
  Sep 15  10:00  lab meeting
  Sep 16  14:00  Ostrander committee
  Sep 18  09:30  dentist`,
    },
    {
      kind: 'assistant',
      // No markdown: the transcript is `white-space: pre-wrap` plain text,
      // so a reply's own line breaks are the only formatting there is. A
      // fixture full of `**bold**` would have shown asterisks, which is what
      // the real page does too — and would have taught a docs reader the
      // wrong thing about what a reply looks like.
      text: `Three things want you today, and one of them has a deadline.

Tomas Lindqvist wants a review decision by Friday — a 9,000-word manuscript, with the review due three weeks after you accept. Your calendar is clear that fortnight apart from the Cape Town trip, so I have drafted an acceptance.

Hollis Barnett needs the Ostrander nomination by Monday. Your notes from March say you meant to nominate Priya, so I have drafted that too, built on the replication work rather than adjectives.

Amara Osei is waiting on Thursday-or-Friday for the Cape Town seminar. Your own note says Thursday, because on the Friday you fly that evening — so I have staged a hold rather than answering for you.`,
    },
    {
      kind: 'notice',
      text: '3 drafts staged in your outbox — nothing was sent.',
    },
  ],
};

// What a scripted send replays, as `[delayMs, event]`. The point of scripting
// this rather than showing a still is that the two things worth understanding
// about the loop are both events: a tool call resolving, and a send that
// stages instead of going out.
export const script = [
  [120, { type: 'delta', text: 'Checking what you already told him.\n\n' }],
  [
    420,
    {
      type: 'tool',
      name: 'recall',
      draft: {
        headers: [],
        body: null,
        // Sorted, because the wire delivers `other` in BTreeMap order — a
        // fixture written in schema order would demo an ordering the page
        // never sees, and the digest is picked out of exactly this.
        other: [
          ['limit', '5'],
          ['query', 'Tomas Lindqvist review commitment'],
        ],
      },
      args: '{\n  "query": "Tomas Lindqvist review commitment",\n  "limit": 5\n}',
    },
  ],
  [
    900,
    {
      type: 'tool_result',
      name: 'recall',
      is_error: false,
      preview: `2 episodes, closest first:

  2024-11-04  reviewed for this journal; told Tomas afterwards
              that you would take one a year
  2026-02-17  declined a second review in the same quarter,
              citing the same rule`,
    },
  ],
  [
    260,
    {
      type: 'delta',
      text: 'You reviewed for this journal in 2024 and told Tomas afterwards that ',
    },
  ],
  [340, { type: 'delta', text: 'you would take one a year. This is the first of 2026, so ' }],
  [340, { type: 'delta', text: 'accepting is consistent with what you said.\n\n' }],
  [
    300,
    {
      type: 'tool',
      name: 'mail__mail_reply',
      draft: {
        headers: [
          ['to', 'tomas.lindqvist@example.org'],
          ['subject', 'Re: Review request — manuscript 2026-0413'],
        ],
        body: 'Dear Tomas,\n\nYes — I can take this one. Three weeks from today puts my report with you on the 25th.\n\nBest,\nLuke',
        other: [['account', 'personal']],
      },
      args: '{\n  "account": "personal",\n  "to": "tomas.lindqvist@example.org",\n  "subject": "Re: Review request — manuscript 2026-0413",\n  "body": "Dear Tomas,\\n\\nYes — I can take this one. …"\n}',
    },
  ],
  [
    900,
    {
      type: 'notice',
      text: 'mail__mail_reply is routed to the outbox — staged, not executed.',
    },
  ],
  [
    80,
    {
      type: 'tool_result',
      name: 'mail__mail_reply',
      is_error: false,
      preview: 'staged as ob-4417 — nothing was sent',
    },
  ],
  [200, { type: 'staged', ids: ['ob-4417'] }],
  [400, { type: 'usage', prompt_tokens: 26402, context_window: 32768 }],
  [
    200,
    { type: 'done', ok: true, error: null, taint_private: true, taint_untrusted: true },
  ],
];

// --- the deeper queue panes ---------------------------------------------

// One proposer's predicate classes. The tier is the evidence tier the handler
// stamps from the accept/reject history, which is what the pane filters on.
export const queueClasses = [
  {
    predicate: 'works_with',
    pending: 5,
    tier: 'solid',
    accepted_hist: 96,
    rejected_hist: 14,
  },
  {
    predicate: 'promised',
    pending: 3,
    tier: 'some',
    accepted_hist: 21,
    rejected_hist: 9,
  },
  {
    predicate: 'attended',
    pending: 1,
    tier: 'thin',
    accepted_hist: 4,
    rejected_hist: 2,
  },
];

// `mecha frontdoor show <seq>` — plain text, not JSON. What a privileged run
// is allowed to see of a stranger's request: the typed extraction, never the
// prose it was pulled from.
export const frontdoorShow = `request #41 · speaking · open
received 2026-08-29 05:14 UTC · valid against the type

  event         Nordic Cognition Meeting
  role          keynote
  when          June 2027 (exact dates not given)
  where         Helsinki
  travel        offered
  honorarium    not stated
  deadline      reply by 2026-10-01
  urgency       high (claimed by the requester)

The prose this was extracted from is not shown to a privileged run, and this
extraction was produced by a quarantined pass with no tools and no history.
Read the request itself with \`mecha frontdoor show 41 --raw\`.`;

// --- the shadow tier, and the entity page (PR #114) --------------------

// `mecha review shadow --json`. The shadow tier is review-on-use: a fact the
// graph served to a run but nobody has ruled on. `surfaced_total` is the
// store's count before truncation, which is why the page never uses the page
// length as the depth — a page is not a depth.
export const shadow = {
  surfaced: [
    {
      fact: {
        uid: 'f-90218',
        statement: 'Priya Raghavan led the retrieval-practice replication.',
        predicate: 'led',
        extractor: 'nightly-extractor',
      },
      reasons: ['served to a run 3 days ago', 'never ruled on'],
    },
    {
      fact: {
        uid: 'f-90233',
        statement: 'Amara Osei hosts the Cape Town seminar series.',
        predicate: 'hosts',
        extractor: 'calendar-linker',
      },
      reasons: ['served to a run yesterday'],
    },
  ],
  surfaced_total: 2,
  shadow_live: 9,
  shadow_served: 148,
};

// `mecha kg entity <name> --json`. Note `tier` and `polarity` arrive from the
// server and are never derived in page script: an unreviewed fact carries the
// Confirm/Refute pair, and a refuted one renders dimmed and settled — a
// recorded no rather than a weak yes.
export const entity = {
  found: true,
  query: 'Priya Raghavan',
  node: {
    // The id rides in the real envelope and the page's write paths key on
    // it (facts, unalias, merge) — a fixture without it makes those forms
    // silently inert in the demo, which reads as a broken feature.
    id: 'person-priya-demo',
    name: 'Priya Raghavan',
    node_type: 'person',
    aliases: ['Priya R.', 'priya.raghavan'],
    // Deterministic keys — how sources reach the node, rendered apart from
    // the aliases (how it is spoken of).
    identifiers: [{ kind: 'email', value: 'priya.raghavan@ostrander.edu' }],
  },
  // Per-source coverage — which stores actually saw this person, and how
  // often. The graph tab renders it on the head card.
  sources: [
    { source: 'mail', episodes: 148, first: '2024-09-03', last: '2026-08-28' },
    { source: 'calendar', episodes: 61, first: '2024-09-12', last: '2026-08-21' },
    { source: 'note', episodes: 5, first: '2026-01-19', last: '2026-08-14' },
  ],
  interaction: {
    interaction_count: 214,
    last_seen_at: '2026-08-28T16:22:00Z',
    last_channel: 'mail',
  },
  facts: [
    {
      uid: 'f-90218',
      statement: 'Priya Raghavan led the retrieval-practice replication.',
      predicate: 'led',
      extractor: 'nightly-extractor',
      valid_from: '2026-02-11T00:00:00Z',
      tier: 'shadow',
      polarity: 'positive',
    },
    {
      uid: 'f-88104',
      statement: 'Priya Raghavan joined the group in 2024.',
      predicate: 'joined',
      extractor: 'mail-distiller',
      valid_from: '2024-09-01T00:00:00Z',
      tier: 'reviewed',
      polarity: 'positive',
    },
    {
      uid: 'f-88790',
      statement: 'Priya Raghavan is a co-author on the 2025 methods paper.',
      predicate: 'co_authored',
      extractor: 'nightly-extractor',
      valid_from: '2025-06-04T00:00:00Z',
      tier: 'reviewed',
      polarity: 'negative',
    },
  ],
  episodes: [
    {
      uid: 'ep-33021',
      occurred_at: '2026-08-28T16:22:00Z',
      source: 'mail',
      preview:
        'Priya: the second pilot came back null again — I think the spacing interval is the problem, not the materials.',
    },
    {
      uid: 'ep-32887',
      occurred_at: '2026-08-21T09:05:00Z',
      source: 'calendar',
      preview: 'Group meeting — Priya presenting the replication write-up.',
    },
  ],
};

// The bounded neighborhood the graph tab shows on an entity — 1–2 hops of
// current facts, never a global view. Chips, each opening its entity.
export const related = {
  root: { id: 'n-priya', name: 'Priya Raghavan' },
  items: [
    {
      id: 'n-retrieval',
      name: 'retrieval-practice replication',
      type: 'project',
      depth: 1,
      via: { predicate: 'led' },
    },
    {
      id: 'n-methods',
      name: '2025 methods paper',
      type: 'paper',
      depth: 1,
      via: { predicate: 'co_authored' },
    },
    {
      id: 'n-lab',
      name: 'the lab',
      type: 'org',
      depth: 1,
      via: { predicate: 'member_of' },
    },
  ],
  truncated: false,
};

// Bi-temporal history: a superseded fact stays visible beside what replaced
// it, because when things changed is half of what a graph knows.
export const timeline = {
  entity: { id: 'n-priya', name: 'Priya Raghavan' },
  facts: [
    {
      uid: 'f-90218',
      statement: 'Priya Raghavan led the retrieval-practice replication.',
      valid_from: '2026-02-11T00:00:00Z',
      valid_to: null,
      superseded: false,
    },
    {
      uid: 'f-71455',
      statement: 'Priya Raghavan was a rotation student.',
      valid_from: '2024-09-01T00:00:00Z',
      valid_to: '2025-06-01T00:00:00Z',
      invalidated_at: '2025-06-02T08:00:00Z',
      superseded: true,
    },
  ],
  episodes: [],
};

// What `learning-report --json` returns: the four series behind the trend
// pane. Shaped to exercise the chart's edge cases rather than merely fill it —
// a fixture that only shows the happy path lets a regression through on the
// paths that actually bite.
//
// - A bucket with sessions and **no** reflections is a real, good measurement:
//   nothing needed correcting. It must draw at zero, not be dropped.
// - A bucket with `rate: null` (no sessions ran) must be **skipped entirely**.
//   Drawing a rate over an empty denominator as 0.0 shows a perfect week where
//   there was no week at all — the "a dash is never zero" rule, in a chart.
// - `never_validated` non-zero exercises the warning style; a rule set that is
//   *entirely* unvalidated means the ledger is not running.
// - A retirement step (`rules_after < rules_before`, no reflections) renders
//   differently from a consolidation, so both appear.
// The Proposals pane: three stores off one summary read, then a listing and
// a `show` per item. Shapes mirror serve/proposals.rs (StoreRow, Listing of
// ReviewRow, Detail) — and `depth` is a count or null, never an invented
// zero.
// The `opens` verbs and depths MUST agree with the `queues` fixture above —
// serve's `stores` reads the same `mecha review queues` the home page does,
// so a demo where the two disagree shows one queue with two depths, and an
// `opens` line that names a command that does not exist teaches a docs
// reader a verb that will fail.
export const proposalStores = [
  {
    store: 'harness',
    label: 'harness candidates',
    depth: 0,
    detail: 'nothing accepted since Tuesday',
    oldest: null,
    opens: 'mecha harness list',
  },
  {
    store: 'rules',
    label: 'rule proposals',
    depth: 1,
    detail: '1 retirement proposed',
    oldest: '2026-08-27 06:10:00',
    opens: 'mecha proposals',
  },
  {
    store: 'entities',
    label: 'graph entities',
    depth: 2,
    detail: '2 proposals: 1 rename, 1 merge',
    oldest: '2026-08-25 09:40:00',
    opens: 'mecha-graph proposals list',
  },
];

export const proposalList = {
  label: 'graph entities',
  rows: [
    {
      id: '41',
      kind: 'rename',
      title: 'priya.raghavan@ostrander.edu → Priya Raghavan',
      detail: 'named by an address; already aliased "Priya Raghavan" (148 mentions)',
    },
    {
      id: '42',
      kind: 'merge',
      title: 'fold P. Raghavan into Priya Raghavan',
      detail: 'near-duplicate person: same mail identifier, 9 shared episodes',
    },
  ],
};

export const proposalDetail = {
  text: `#42 · merge · pending
  keep  Priya Raghavan (person-priya-demo)
  fold  P. Raghavan (person-priya-dup-demo)

evidence: near-duplicate person — the same mail identifier reaches both,
and 9 episodes mention the pair within a minute of each other.

accepting moves every fact, mention and alias onto the kept node and
records the decision; rejecting files a durable no the detector will
not re-ask.`,
};

export const learningReport = {
  buckets: [
    {
      period: '2026-07-30',
      sessions: 96,
      reflections: 4,
      rate: 0.042,
      error_types: { 'wrong-approach': 2, 'missed-context': 2 },
    },
    {
      period: '2026-08-06',
      sessions: 52,
      reflections: 0,
      rate: 0,
      error_types: {},
    },
    {
      period: '2026-08-13',
      sessions: 0,
      reflections: 0,
      rate: null,
      error_types: {},
    },
    {
      period: '2026-08-20',
      sessions: 268,
      reflections: 29,
      rate: 0.108,
      error_types: {
        'wrong-approach': 11,
        'missed-context': 9,
        overreach: 5,
        other: 3,
      },
    },
    {
      period: '2026-08-27',
      sessions: 33,
      reflections: 5,
      rate: 0.152,
      error_types: { 'missed-context': 3, 'premature-action': 1, style: 1 },
    },
  ],
  steps: [
    {
      at: '2026-08-04T03:03:21Z',
      domain: 'behavior',
      rules_before: 0,
      rules_after: 1,
      reflections: 1,
    },
    {
      at: '2026-08-29T17:33:43Z',
      domain: 'behavior',
      rules_before: 0,
      rules_after: 12,
      reflections: 28,
    },
    {
      at: '2026-08-29T21:10:02Z',
      domain: 'behavior',
      rules_before: 12,
      rules_after: 11,
      reflections: 0,
    },
  ],
  health: {
    behavior: {
      active: 11,
      retired: 1,
      never_validated: 3,
      attributed_regressions: 2,
    },
  },
  caveat:
    'Observational, over one owner’s real work: the task mix moves under the metric, so a falling correction rate may mean better rules or an easier week. Use `mecha eval --ab-rules` for a controlled comparison.',
};
