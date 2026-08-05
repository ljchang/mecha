---
title: Mail and calendar
sidebar_position: 14
description: mecha-mail — every Gmail and Outlook account behind one provider-neutral MCP surface, where the model names an account and never a provider.
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
mecha-mail import personal --provider google    # adopt a legacy per-provider login
mecha-mail accounts                              # names, providers, addresses, the default
mecha-mail default dartmouth                     # set the standing default
mecha-mail serve                                 # the MCP server (also the no-subcommand default)
```

Credentials live at `~/.mecha/mail/<name>/oauth.json`, one store per account.
Names are lowercase letters, digits, `-` and `_`; duplicates and a default that
names no configured account are refused at load.

**The account names are baked into every tool schema as an enum at startup.**
`MailTools::load` reads `accounts.toml` once and calls `tool_definitions(&names,
default)`, which emits `{"type": "string", "enum": ["dartmouth", "personal"]}`
for the `account` property of all ten tools, plus a sentence naming the default
where one exists. The model picks from real names instead of guessing at them,
and the schemas are built once rather than per request.

A configured account whose credentials will not load **fails startup** with the
command that fixes it, rather than being quietly skipped:

```
account `dartmouth`: run `mecha-mail auth dartmouth --provider outlook`
```

## Resolution: the rule that shapes the surface

Ten tools — `mail_search`, `mail_recent`, `mail_get_thread`, `mail_send`,
`mail_reply`, `calendar_list`, `calendar_list_events`, `calendar_create_event`,
`calendar_update_event`, `calendar_delete_event` — and three resolution modes.

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

A shared test helper, `assert_tool_surface`, is run against each provider's tool
list and asserts all of it: every read is `readOnlyHint` and is *not*
`openWorldHint` ("reaches only the provider that already custodies this data —
not a send sink"), every write is `openWorldHint` and is *not* `readOnlyHint`,
and every tool has an object schema and a description worth reading. A new
provider cannot ship a mislabelled surface. Unification did not weaken this: the
same annotations ride on the unified tools, and one send name in the outbox list
now covers every account it could send from.

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
sentitems/messages?$top=1&$select=from`), which is the conclusion flowmail
reached independently.

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

flowmail kept storage, refresh, and retry-on-401 in its JS frontend. Here it is
in Rust, in the library, so every caller gets it:

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

## Four flowmail behaviours fixed rather than ported

Each was filed upstream (`ljchang/flowmail` issues 3–6):

- Graph replies go through `POST /messages/{id}/reply` so they **thread**.
- The calendar reads `calendarView`, so recurring events do not vanish from a
  window.
- Search uses `$search` instead of a `$filter` that 400s beside `$orderby`.
- `to` splits on commas, like `cc` and `bcc` already did.

## Two unification wrinkles

**`mail_reply` takes a `thread_id`** and answers the newest message in it (or
`message_id` when one is named). Graph does that natively; Gmail cannot, so
`gmail_reply_fields` synthesizes the addressing: answer the sender, or the
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

flowmail took only the `text/plain` part, so an HTML-only email reached the model
as an empty body. Here the body falls back through `body_text` → HTML converted
to markdown → the snippet, and everything on that path is then sanitized: HTML
comments stripped, long base64 runs replaced with a placeholder, and
`<system` / `<tool` / `<function` escaped. Outbound header values containing a
line break are refused outright, so a model-supplied subject cannot inject a
header.
