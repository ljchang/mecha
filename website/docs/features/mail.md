---
title: Mail and calendar
sidebar_position: 21
description: mecha-mail — every Gmail and Outlook account behind one provider-neutral MCP surface, where the model names an account and never a provider, plus the triage queue over it.
---

# Mail and calendar

`mecha-mail` is a third crate: a **library plus three thin MCP binaries**.

The library holds Gmail and Google Calendar v3, Outlook mail and calendar over
Microsoft Graph, both OAuth flows, and the token lifecycle. It is what a GUI
would depend on directly.

| Binary | Serves |
|---|---|
| `mecha-google` | one Google account, its own credential store |
| `mecha-outlook` | one Microsoft account, its own credential store |
| **`mecha-mail`** | **every account in `~/.mecha/mail/` behind one provider-neutral surface** |

`mecha-mail` is the one deployments should wire. Behind it, no mecha-core or
mecha-cli code knows that Google or Microsoft exists — and neither does the
model.

```toml
[[mcp]]
name = "mail"
command = "~/.cargo/bin/mecha-mail"
# The server renders event times, so it needs the zone as well as the agent.
env = { MECHA_TZ = "America/New_York" }

[mcp.capabilities]
untrusted_input = true

[outbox]
tools = [
  "mail__mail_send",
  "mail__mail_reply",
  "mail__calendar_create_event",
  "mail__calendar_update_event",
  "mail__calendar_delete_event",
]
```

## The model names an account, never a provider

`accounts.toml` in `~/.mecha/mail/` maps short names to providers:

```toml
default = "dartmouth"

[[account]]
name = "personal"
provider = "google"

[[account]]
name = "dartmouth"
provider = "outlook"
```

```bash
mecha-mail auth dartmouth --provider outlook --tenant <tenant-id>
mecha-mail auth personal  --provider google
mecha-mail import personal --provider google    # copy a mecha-google / mecha-outlook login in
mecha-mail accounts                              # names, providers, addresses, the default
mecha-mail default dartmouth                     # set the standing default
mecha-mail serve                                 # the MCP server (also the no-subcommand default)
```

`auth` takes the OAuth client configuration where it needs it: `--client-id`
(the Google Desktop app id or the Entra application id, also read from
`GMAIL_CLIENT_ID` / `OUTLOOK_CLIENT_ID`), `--client-secret` for Google's
desktop pseudo-secret, `--tenant` for the Entra directory, and `--port` for the
Google loopback redirect. **A second mailbox on the same app registration needs
none of them**: with no `--client-id`, the client configuration is taken from
this account's own stored login, or failing that from a configured sibling of
the same provider. Adding your second Gmail account is one command with two
arguments.

Credentials live at `~/.mecha/mail/<name>/oauth.json`, one store per account.
Names are lowercase letters, digits, `-` and `_`; duplicates and a default that
names no configured account are refused at load.

**The account names are baked into every tool schema as an enum at startup.**
`accounts.toml` is read once, and every tool's `account` property is emitted as
`{"type": "string", "enum": ["dartmouth", "personal"]}`, plus a sentence naming
the default where one exists. The model picks from real names instead of
guessing at them, and the schemas are built once rather than per request.

A configured account whose credentials will not load **fails startup** with the
command that fixes it, rather than being quietly skipped:

```
account `dartmouth`: run `mecha-mail auth dartmouth --provider outlook`
```

## Resolution: the rule that shapes the surface

Eleven tools — `mail_search`, `mail_recent`, `mail_get_thread`, `mail_send`,
`mail_reply`, `calendar_list`, `calendar_list_events`, `calendar_freebusy`,
`calendar_create_event`, `calendar_update_event`, `calendar_delete_event` — and
three resolution modes.

`calendar_freebusy` is the scheduling one: busy intervals merged across every
account, with no event details in them. "When am I free on Thursday?" is
answered from it; `calendar_list_events` is for when the events themselves
matter.

### Reads fan out

No `account` on a search, a recents listing, a calendar list, or a calendar
window means **every mailbox**, queried concurrently and merged. Mail rows are
sorted newest first; calendar events are sorted by start time. Every row carries
the account it came from:

```json
[
  {"account": "dartmouth", "thread_id": "AAQk...", "message_id": "AAMk...",
   "from": "Priya Nair <priya@example.edu>", "subject": "Retreat agenda",
   "date": "2026-08-04T09:12:00Z", "snippet": "…", "unread": true,
   "has_attachments": false}
]
```

That tagging is what makes the next rule workable: the model always already has
the account by the time it needs to name one.

### Item operations name their account

Thread ids and event ids are **account-scoped**. `mail_get_thread`,
`mail_reply`, `calendar_update_event` and `calendar_delete_event` require
`account` when more than one is configured, and say where to find it:

```
several accounts are configured (dartmouth, personal) and this id is
account-scoped — pass `account` (every search and list row carries it)
```

A single-account install never needs to name anything: with one account, every
mode resolves to it.

### Creates use the default, or ask

`mail_send` and `calendar_create_event` fall back to the default account. With
several accounts and no default, the error says to **ask the user**:

```
several accounts are configured (dartmouth, personal) and no default is set —
ask the user which account to use, then pass it as `account`.
(They can set a standing default with `mecha-mail default <name>`.)
```

The wording is deliberate and there is a test pinning it. "Ask the user" rather
than "use your best judgment": the second phrasing was measured to make models
invent an answer instead of stopping. The same finding shaped `ask_user`'s
decline wording elsewhere in mecha.

### A failed account never sinks a fan-out

Failures are collected separately from successes. If at least one account
answered, the results are returned with the failures appended as a note:

```
note — some accounts could not be read:
account `personal`: request timed out
```

The call reports an error **only when every account failed**. One expired
refresh token does not cost you the other mailbox.

## Two commands with no model in them

`mecha-mail` also serves the scheduling pipeline directly, as data, on a timer:

```bash
mecha-mail freebusy --days 60 --json     # merged busy intervals across every account
mecha-mail bookings --dry-run            # what drained bookings would become events
```

**`freebusy` deliberately inverts the rule above: it fails when *any* account
is unreadable.** The MCP surface answers a person who can see the note about
which mailbox was skipped; this one feeds a public booking page. A mailbox that
could not be read is not a mailbox with free time, and a slot list built from a
partial answer offers strangers hours the user does not have. `--from`/`--to`
name an explicit window instead of `--days`, and `--account` narrows to one.

**`bookings` is the inbound sibling**: it turns drained booking records into
calendar events, deterministically, with no model anywhere. It is idempotent
against `~/.mecha/mail/bookings.jsonl` — a record already ledgered is skipped,
so re-running after a partial failure picks up exactly where it stopped — and
each event is re-verified against live free/busy before it is created, because
the slot was sold from a cache and home holds the fresher truth. A collision is
parked loudly for a human rather than double-booked. `--account` names the
calendar that receives the events, defaulting to the default account; an absent
request store is "nothing drained yet" rather than an error, because this runs
on a timer that must not cry wolf.

## Capability labeling: reads are untrusted sources, not send sinks

This is the part worth not re-litigating.

**Reads carry `readOnlyHint` and deliberately not `openWorldHint`.** A search
query travels only to googleapis.com or graph.microsoft.com — hosts that already
custody the mailbox. There is no payload channel to a third party. That is
precisely the difference from `http_fetch`, whose query string can reach any
host in the world, and it is why the read tools are not
[trifecta](/docs/features/security) sinks.

**But mail bodies are other people's words.** Reading mail must arm the
interlock, so config forces `untrusted_input = true` on the server — the same
treatment the knowledge graph gets. That override only ever *widens*: config can
distrust a server further than its own annotations, never less.

**Sends and calendar writes do reach third parties** — recipients, invitees — so
they carry `openWorldHint`, and `calendar_update_event` / `calendar_delete_event`
add `destructiveHint`. Those names go in `[outbox] tools`, so they **stage
rather than deliver**. See [the outbox](/docs/features/outbox).

**And there is a third quadrant, which is neither.** `mail_triage` — archive,
mark read or unread, report spam, trash — mutates your own mailbox and reaches
nobody. It carries `destructiveHint` alone:

- Not `openWorldHint`, so it must never appear in `[outbox] tools`. Staging it
  would make triage circular — you would review a queue in order to fill
  another queue.
- Not `readOnlyHint`, or an unattended run under
  `permission_mode = "read-only"` could empty your inbox at seven in the
  morning.

The shared surface test takes a third slice that asserts exactly that pair of
negatives, so neither mistake can ship quietly. [Documents](/docs/features/documents)
land `docs_trash` in the same quadrant, from the other direction.

A shared test runs against each provider's tool list and asserts all of it:
every read is `readOnlyHint` and is *not* `openWorldHint` ("reaches only the
provider that already custodies this data — not a send sink"), every write is
`openWorldHint` and is *not* `readOnlyHint`, and every tool has an object schema
and a description worth reading. A new provider cannot ship a mislabelled
surface. Unification did not weaken this: the same annotations ride on the
unified tools, and one send name in the outbox list now covers every account it
could send from.

## Triage: the queue over the mailbox

`mail_triage` is the verb. `mecha mail` is the surface you actually use, and it
exists because an inbox is not a thing you read once — it is a queue you work.

```bash
mecha mail classify --account dartmouth   # read recent mail, decide what each thread is
mecha mail list                           # what needs you, newest first
mecha mail list --aged                    # day two: what you meant to answer
mecha mail show <thread_id>               # read one, in full
mecha mail reply <thread_id>              # draft an answer — stages, never sends
mecha mail task <thread_id>               # track it on the graph's board
mecha mail correct <thread_id> --bucket respond   # the classifier got it wrong
mecha mail dismiss <thread_id>            # drop it from the queue without acting
```

Threads are named by an **eight-character handle** — the *last* eight characters
of the id, and any unique suffix is accepted wherever a thread id is. A suffix
rather than a prefix because Outlook conversation ids share a 57-character
common prefix: in a real 68-thread store every prefix handle collapsed to the
same eight characters and identified nothing. Ambiguity is an error rather than
a guess, because acting on the wrong thread is silent and, for `mail_triage`,
irreversible.

`classify` writes one typed verdict per thread to `~/.mecha/mail-triage/`: a
bucket (`respond` / `notify` / `ignore`), an urgency, a proposed action, tags
from a closed vocabulary, a deadline if the thread implies one, and the kind of
standard request it is if it recognises one. On a fifty-thread sample of real
academic mail, twenty-eight were archivable and twenty-two needed attention.

### The prefilter: half the mailbox never reaches a model

`prefilter` disposes of a thread **from its envelope alone, ahead of the
classifier**: a `List-Unsubscribe` header, or a sender address or display name
that reads as a system. Measured on a year of real mail with exactly the shipped
marker list, the two rules match a little under half of all threads, and five of
the threads they caught had ever received a reply — about one in a thousand.

`List-Unsubscribe` alone is not enough, and that is the finding underneath the
rule: it catches marketing, which is obliged to offer an unsubscribe, and misses
every institutional and transactional sender, which is not. If you are tuning
this, a sender-address rule is worth more than a better prompt.

Three properties keep it safe rather than merely cheap, each with a test named
on it:

- **It only ever produces `ignore`.** A deterministic rule may say "nothing
  here" and may never say "this needs a reply" — the cases it would have to get
  right to do that are exactly the ones that need judgement.
- **It reads the envelope and never the body**, so it is not a second place a
  stranger's prose gets interpreted outside the classifier's quarantine. Markers
  written into a subject line do not fire it.
- **The sender list is portable rather than maximal.** An exploratory pass
  scored five points higher by matching one institution's own systems, and a
  shipped default tuned to one mailbox quietly underperforms in every other.

A pre-filtered thread still gets a verdict and a one-line summary, because it
still appears in `mecha mail list` and a list you cannot recognise a thread in
is not a list.

**The store is an index, not a copy of your mailbox.** It holds ids, envelope
metadata and the verdict. Bodies are fetched on demand and never written there,
so the retention question stays with your provider and there is no second place
for mail to leak from.

### The classifier never talks to a run that has tools

This is the whole design, and it is the [front door's](/docs/features/frontdoor)
shape applied one directory over:

> **The privileged run sees the extraction, never the prose.**

Reading mail arms `untrusted_input`. A loop that reads fifty threads into one
conversation therefore arms the interlock for all fifty, and every draft it
stages comes out tainted — correct, and useless, because a warning that fires on
everything has stopped being a warning.

So the prose goes to a classifier issued **no tools, no history, no system
prompt and no shared cache prefix**. It is a fresh one-shot call per thread, and
only its typed output travels. What a run with tools is given is the verdict and
the sender's address; what stays behind is the subject, the sender's chosen
display name, the classifier's reasoning, and its one-line summary. That last
one is the tempting one to pass — it is short, and it is exactly what a summary
line wants — but it is model prose derived from prose a stranger wrote, and
paraphrasing an injection does not remove it. A run that genuinely needs to know
what a thread says calls `mail_get_thread` and takes the taint honestly.

`mecha mail show` prints the prose, deliberately. A person reading their own
mail in a terminal is the safe context: you cannot be prompt-injected into
mailing your own calendar somewhere. `mecha mail list --json` serves the typed
view instead, because a script has no human's excuse.

### Snippet first, body only where it matters

A preview settles the newsletters. The full message is read only when the
verdict is `respond` or names a request kind — the cases where the answer
changes what happens next. Roughly a quarter of threads escalate.

It is deliberately **not** triggered by how short the snippet looks. A provider
caps its preview at a couple of hundred characters, so nearly every real email
appears truncated, and escalating on that would escalate everything.

### Working the queue: what you can do to a thread

Every verb is a separate command rather than `mecha mail act --action <x>`, on
the same reasoning that made `mail_triage`'s actions a closed enum: a free-form
label argument would put `spam` inside a verb that reads as harmless.

| Command | What it does | Where it lands |
|---|---|---|
| `reply` | a model reads the thread and composes an answer | **staged in the [outbox](/docs/features/outbox)** |
| `forward --to <addrs>` | passes it on with a covering line | **staged in the outbox** |
| `schedule` | turns it into a calendar event | **staged in the outbox** |
| `archive` | out of the inbox — reversible, nobody notified | the mailbox |
| `spam` | trains the provider's filter | the mailbox |
| `task` | tracks it on the knowledge graph's board | the graph |
| `needs-info --missing <what>` | parks it until somebody answers | the store |
| `dismiss` | drops it from the queue without acting | the store |

Three of these reach a third party, and all three **stage rather than send**.
`reply` is the one action here that needs an agent rather than a tool call — a
model has to read the thread and write prose — and the run that does so reads
the thread, which arms both interlock legs. So the draft arrives in `/outbox`
flagged tainted, which is correct: it was written after reading a stranger's
words. Drafting from the classifier's one-line summary instead would produce
*clean* drafts written from a paraphrase, which is worse exactly where it
matters.

`archive` and `spam` reach nobody outside your own mailbox, so they are not
staged — staging them would make triage circular, reviewing a queue in order to
fill another queue. `spam` is separated from `archive` because it is the one
triage action with an effect outside your mailbox: it trains the provider's
filter.

`task` carries the **deadline the classifier already found**, which is the whole
point — a task somebody has to re-read the mail to schedule is a task they will
schedule later or never. Its `--project` must already exist on the graph and is
passed through untouched, never invented from a subject line: a project node
conjured out of an email is a board nobody can query.

`needs-info` is not `dismiss`. Dismissing says *I am not doing this*; parking
says *I have asked and cannot proceed yet*, and the thread stays your problem.

### Day two: the threads you meant to answer

```bash
mecha mail list --aged                    # respond threads old enough to have been answered
mecha mail list --aged --surface          # …and record that they were surfaced
```

A thread still unanswered after a day is overwhelmingly unlikely ever to be
answered, and by then the person has stopped looking at mail — so a queue that
only works as a *pull* surface is exactly why those threads die. `--aged` is the
list the morning [trigger](/docs/features/triggers) reads.

Two decisions in it:

- **It keys on the bucket, never on silence.** Most unanswered mail correctly
  needed no reply, so only `respond` threads age into this list.
- **`--surface` is separate from reading the list on purpose.** A list command
  that mutates as a side effect of being run cannot be used to look, and looking
  is most of what anyone does with a queue. The briefing passes it; a person
  checking what day two would say does not.

The default age is 30 hours rather than 24 — a working day, so an email that
arrived in the evening is not nagged about at breakfast.

### Corrections: telling the classifier it was wrong

```bash
mecha mail correct <thread> --bucket respond --urgency today
mecha mail correct <thread> --deadline none      # `none` clears a field
```

**Field-level on purpose.** A misread bucket, a missed deadline and a wrong
request kind are different errors with different fixes, and a correction that
only says "this was wrong" teaches a learner noise.

The verdict is fixed **immediately**, so the list you read is right straight
away — and the before/after pair is kept on the record, because the mistake is
what a learner has to see. A learner shown only the right answer cannot tell
what to stop doing. A correction that agrees with the classifier records
nothing.

### Corrections become rules

```bash
mecha mail reflect          # corrections → triage-domain reflections
```

One tool-less, history-less model call per unmined correction — the same shape
as the classifier it is reasoning about — and it is idempotent, each correction
keyed into its own ledger so a nightly pass never re-argues one.

**Most corrections produce nothing, deliberately.** The frame asks for a rule
about a *kind* of mail and says outright that declining is the common case: a
wrong rule rides in every future classification, and a missing one costs a
single verdict.

Those reflections feed the `triage` [learning domain](/docs/features/learning#the-triage-domain),
whose rules ride only in the classifier's own pass and never in a run that has
tools. That distinction is what makes learning from mail possible at all — see
the learning page for the provenance argument, which is the subtlest thing in
either subsystem.

### Bulk reading is an operator verb, never a tool

```bash
mecha-mail corpus --since 2026-07-01 --account dartmouth
```

`corpus` downloads a span of mail for analysis into
`~/.mecha/mail-corpus/<account>.jsonl`, walking **all** folders including Sent
so a reply can be joined back to the thread that prompted it. It is what `score`
and `eval` read.

**It is absent from the MCP surface on purpose.** The model has no business
reading a year of mail, and a corpus verb on the tool surface is one prompt away
from being asked to.

It also stores mail **unclassified**, which is the subtler half. Running a
corpus through the classifier projects the current tags onto it and confirms
them by construction, so a taxonomy derived that way measures the labels rather
than the mail. That is how the vocabulary was wrong for a month: the largest
single category of mail arriving was missing from the list entirely, because the
most routine thing that arrives is the thing that does not come to mind.

The analysis that produced those decisions is **gitignored**. One mailbox's
figures are its owner's, so what the measurement decided is written down and
what it counted is not.

### Measuring it: `score` and `eval`

Classification accuracy stops being a feeling. Two instruments, answering
different questions:

```bash
mecha mail score                  # the live store, against what actually happened
mecha mail eval --account dartmouth --out graded.jsonl   # the classifier, against a known corpus
```

**`score`** grades the live triage store. Behaviour (did a reply actually go
out) and testimony (what you corrected) are reported **apart**, because a reply
is one-sided evidence and a correction is not. Threads younger than 48 hours are
excluded: most replies that ever happen land on the first day, so a same-day
thread has no outcome yet, and counting it would punish every rule equally for
how recently the mail arrived. Reply evidence comes from the
[corpus](#bulk-reading-is-an-operator-verb-never-a-tool) rather than from `mail_get_thread`,
because that tool renders prose for a model to read and a measurement keyed on a
display format breaks silently the day the format changes.

**`eval`** grades the classifier against a corpus whose outcome is already
known, with no human grading anything. **The ground truth is one-sided and the
output says so**: a thread you answered proves the thread mattered, so burying
it is a countable error — a thread you never answered proves nothing, because
most unanswered mail correctly needed no answer and some was settled in a
meeting. So it reports a false-`ignore` rate on the *answered* stratum and a
*volume* on the other, and never a single blended accuracy. Both strata are
sampled to the same size, because answered threads are rare and a uniform sample
of 200 would hold a handful of the only threads carrying ground truth.

`--out` keeps every graded verdict. A measurement that discards its evidence has
to be re-run to be re-read: the first run of this eval reported a merged figure
and threw away the 120 judgements behind it, so splitting `respond` from
`notify` afterwards cost another hour of inference rather than a `grep`. Grading
the artifact is this project's rule for models; it applies to its own instruments
too.

`eval` writes nothing to the triage store — grading year-old mail is not
triaging it, and a scorecard that mutated the queue it measures would be
unrepeatable.

### `/mail` — the queue as a modal

The TUI works the queue without leaving it, on the `/outbox` pattern: **the
store is read for display, and every mutation is a `mecha mail …` child
process.** Nothing there reimplements a verb.

That is not tidiness. The store and the CLI are the product and every front-end
is one reader — the nightly, the morning briefing and the modal all act through
the same commands — so anything the modal can do a script can do, and a
modal-only action would be a feature no trigger could ever use. Slow work
(a reply builds a whole tool surface and can take minutes) spawns detached and
is watched by polling the store, never the child.

**A reply's result lands in `/outbox`, not here.** There is exactly one approval
surface and this is not it: `/mail` decides *whether* something needs an answer,
`/outbox` decides whether *this* answer goes.

Unlike the front door, this modal shows prose — the same reasoning as
`mecha mail show`. What must not see the prose is a privileged run, and none
happens here.

### Tags are mecha's own

Not a Gmail label, not a Graph category. Those are different objects, and a tag
that means something subtly different per account fails at the one job a tag
has. Keeping them internal costs no OAuth scope and works identically on both
providers. The cost, stated plainly: **tags are invisible in Gmail, Outlook and
on your phone.** Mail triaged by mecha looks untouched in every other client.

### Recognising a request is not routing it

If a thread is really a standard request arriving as an email — a
recommendation letter, someone asking to join the lab — the classifier names the
kind. Whether it can then be handed to the front door is a separate question,
answered by whether a form for that kind actually exists. A kind with no form
keeps its name, because that is evidence about what your mail actually contains,
and loses only a promotion there would be nothing behind.

### Running it nightly

`scripts/mecha-mail-classify.{service,timer}` sweeps at 05:30 UTC:

```bash
cp scripts/mecha-mail-classify.{service,timer} ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now mecha-mail-classify.timer
```

A timer rather than a [trigger](/docs/features/triggers), because a trigger's
action is a *prompt* on purpose and this is a deterministic command. The unit
names its workspace explicitly: a user unit without one runs in `$HOME`, which
contains `~/.mecha`, and a workspace the mecha home sits under is refused.
Failures need no special handling — `mecha doctor` already watches every
`mecha-*` unit.

**A failed classification is retried; a dismissed one is not.** The sweep asks
whether a thread *needs* classifying rather than whether the store has heard of
it — absent or `failed` means classify, `classified` is done, and `dismissed`
was a person's decision. The distinction is not academic: the store holds
failures as well as verdicts, so a sweep skipping everything it had a record for
would skip a failed thread **forever**, on the strength of a record saying the
classification never happened. Found by the outage that produced it — a model
server down overnight left 17 threads recorded `failed`, among them a manuscript
review invitation, and every later sweep would have reported "0 to classify"
like any quiet morning. A transient infrastructure failure must not become a
permanent editorial one.

## Microsoft signs in with device code

```bash
mecha-mail auth dartmouth --provider outlook --tenant <tenant-id>
```

```
To sign in, open https://microsoft.com/devicelogin on any device
and enter this code:

    F7KQ2XM9B
```

Three properties follow from choosing device code over loopback:

- **No redirect URI**, so it reuses an org-approved app registration untouched.
- **No forwarded port**, so it works over SSH.
- **It is a public client.** Entra binds the refresh credential to the auth
  method that minted it, so sending a `client_secret` after a device-code grant
  fails with `AADSTS7000215` even when the secret is correct. The stored
  credential keeps `client_secret` empty and the flow never sends one.

Scopes are exactly four, and deliberately no more:

```
https://graph.microsoft.com/Mail.Read
https://graph.microsoft.com/Mail.Send
https://graph.microsoft.com/Calendars.ReadWrite
offline_access
```

`Mail.ReadWrite` is excluded because nothing here modifies a message in place.
`User.Read` is excluded because `GET /me` is not worth a consent prompt — so the
account's own address is read from **Sent Items** instead (`/me/mailFolders/
sentitems/messages?$top=1&$select=from`), using a scope the account already
needs.

**An account lookup must never be fatal to `auth`.** If the address cannot be
determined, the flow prints a note and saves the tokens anyway. Losing a
completed sign-in over a cosmetic detail makes the user authenticate twice.

Entra error codes are translated rather than passed through raw — admin consent,
app-not-registered-in-tenant, wrong-org, unrecognised tenant, and the
public-client-flows switch each get a sentence saying what to change, with the
raw description kept in parentheses.

Google, by contrast, uses a loopback PKCE flow on `127.0.0.1:8924` with
`access_type=offline&prompt=consent` (Google needs both to reliably return a
refresh token every time), four scopes (`gmail.readonly`, `gmail.send`,
`calendar`, `calendar.events` — `gmail.modify` excluded), and a 120-second
timeout on the redirect.

## The token lifecycle

Storage, refresh, and retry-on-401 live in Rust, in the library, so every caller
gets them:

- **`oauth.json` at mode 0600**, written to a temp sibling that is *created* with
  that mode before any bytes land, then renamed. The directory is 0700.
- **Refresh ahead of expiry, behind a lock.** The cached token is used only while
  more than 120 seconds of life remain — clock skew plus the duration of the call
  the token is about to make. The credentials sit behind an async mutex held
  across the refresh, so two concurrent tool calls cannot race two refreshes;
  both providers rotate the refresh token, and the loser of that race would
  persist a stale one.
- **One forced refresh and retry on a 401.** An HTTP 401 from either API is
  recognised as auth expiry, triggers a refresh regardless of the clock, and the
  call is retried exactly once.
- **Retry with backoff on 429 and 5xx**: three attempts total, 500 ms then
  1000 ms. Transport errors retry; other 4xx never do. A streaming request that
  cannot be cloned gets one try.

## Provider quirks handled for you

Four places where the obvious call is the wrong one, each settled in the
library so no caller has to know:

- **Graph replies go through `POST /messages/{id}/reply`**, so they thread
  rather than arriving as a new conversation.
- **The calendar reads `calendarView`**, so recurring events do not vanish from
  a window.
- **Search uses `$search`**, not a `$filter` that 400s beside `$orderby`.
- **`to` splits on commas**, exactly as `cc` and `bcc` do.

## Two unification wrinkles

**`mail_reply` takes a `thread_id`** and answers the newest message in it (or
`message_id` when one is named). Graph does that natively; Gmail cannot, so the
library synthesizes the addressing: answer the sender, or the
recipients when you are replying to your own message; keep everyone on
reply-all; drop the user's own address, which is known from the credential
store; and add `Re:` only if the subject does not already carry it.

**Merged calendars sort on the raw provider stamps**, before any zone rendering,
because rendered strings only sort within one zone. Rendering happens afterwards
in `MECHA_TZ` (falling back to `TZ`, then to leaving the stamp alone) — and
all-day events skip zone conversion entirely and keep their bare date, or a
Monday retreat gets announced as Sunday at 8pm. See
[Timezones](/docs/reference/configuration).

## HTML-only mail

Taking only the `text/plain` part is how an HTML-only email reaches the model as
an empty body. So the body falls back through `body_text` → HTML converted to
markdown → the snippet, and everything on that path is then sanitized: HTML
comments stripped, long base64 runs replaced with a placeholder, and
`<system` / `<tool` / `<function` escaped. Outbound header values containing a
line break are refused outright, so a model-supplied subject cannot inject a
header.
