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
  queues: [
    {
      queue: 'outbox drafts',
      depth: 3,
      detail: '2 replies, 1 calendar hold',
      opens: 'mecha outbox',
    },
    {
      queue: 'front-door requests',
      depth: 1,
      detail: 'a speaking invitation, extracted',
      opens: 'mecha frontdoor',
    },
    {
      queue: 'graph candidates',
      depth: 12,
      detail: 'from last night’s extraction',
      opens: 'mecha review sample',
    },
    {
      queue: 'rule proposals',
      depth: 1,
      detail: '1 retirement proposed',
      opens: 'mecha learn',
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
  // `prompt_block(..).chars().count()`, not the file length. Measured with
  // the real code; the pane now shows it unconditionally, so a stale number
  // here is visible on the docs demo.
  char_count: 716,
  budget: 2000,
  over_budget: false,
  parse_error: null,
  error: null,
};

export const rules = [
  {
    title: 'Never accept a review without checking the calendar first',
    domain: 'mail',
    active: true,
    retired: false,
    observations: 14,
    attributed_regressions: 0,
    user: true,
  },
  {
    title: 'Nominations and letters name the specific work, not the person’s qualities',
    domain: 'writing',
    active: true,
    retired: false,
    observations: 6,
    attributed_regressions: 0,
    user: true,
  },
  {
    title: 'Summarise threads before quoting them',
    domain: 'mail',
    active: false,
    retired: true,
    observations: 21,
    attributed_regressions: 3,
    user: false,
  },
];

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
    {
      kind: 'tool',
      name: 'mail__mail_triage',
      pending: false,
      is_error: false,
    },
    {
      kind: 'tool',
      name: 'mail__calendar_list',
      pending: false,
      is_error: false,
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
  [420, { type: 'tool', name: 'recall' }],
  [900, { type: 'tool_result', name: 'recall', is_error: false }],
  [
    260,
    {
      type: 'delta',
      text: 'You reviewed for this journal in 2024 and told Tomas afterwards that ',
    },
  ],
  [340, { type: 'delta', text: 'you would take one a year. This is the first of 2026, so ' }],
  [340, { type: 'delta', text: 'accepting is consistent with what you said.\n\n' }],
  [300, { type: 'tool', name: 'mail__mail_reply' }],
  [
    900,
    {
      type: 'notice',
      text: 'mail__mail_reply is routed to the outbox — staged, not executed.',
    },
  ],
  [80, { type: 'tool_result', name: 'mail__mail_reply', is_error: false }],
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
    name: 'Priya Raghavan',
    node_type: 'person',
  },
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
