---
title: Mail and calendar
sidebar_position: 18
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
mecha mail show <thread_id>               # read one, in full
mecha mail dismiss <thread_id>            # drop it from the queue without acting
```

`classify` writes one typed verdict per thread to `~/.mecha/mail-triage/`: a
bucket (`respond` / `notify` / `ignore`), an urgency, a proposed action, tags
from a closed vocabulary, a deadline if the thread implies one, and the kind of
standard request it is if it recognises one. On a fifty-thread sample of real
academic mail, twenty-eight were archivable and twenty-two needed attention.

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
