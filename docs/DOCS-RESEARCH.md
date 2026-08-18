# Google Docs, Sheets and Slides — how an agent gets a writable seat

*2026-08-18. The survey behind "let mecha create, edit and delete my
Google documents". Motivated by the user's existing
`ljchang/google-docs-plugin` — a Claude Code plugin wrapping
`@piotr-agier/google-drive-mcp` over a service account — which works but
is "finicky and hard to keep working". Four web-research passes plus two
live probes against Google's own endpoints; claims carry their sources,
and the two probes are recorded in §6 with what they actually returned.*

## 0. The short answer

The finickiness was never the MCP server. It was the **auth model**, and
every option on the table inherits its shape from one question Google
asks before anything else:

> Which tier is the scope you need — non-sensitive, sensitive, or
> restricted?

That single answer decides whether you owe Google nothing, a demo video,
or **$540 a year forever**. It also decides whether your refresh token
lives for seven days or indefinitely. Every workaround in this space —
the service account in the current plugin, the `drive` scope in most
third-party servers — exists to route around that question rather than
to answer it.

There is now a way to answer it cheaply. Google shipped a **desktop
Picker flow** (`trigger_onepick=true`) that turns a loopback OAuth
redirect into a file chooser: the user picks documents in a real Google
UI, and their ids come back on the redirect. That buys per-file write
access under `drive.file` — the one **non-sensitive** scope in the
family — with no verification and no CASA, and, once that project is
*published* rather than left in Testing, no seven-day token expiry
either (§6.2).

So: **build it natively into `mecha-mail`**, reusing the OAuth and token
machinery that is already there, scoped to `drive.file`, with the picker
as the grant surface. Not an MCP server, and not the official Google
one.

## 1. What is being asked for

Create, edit and delete Google Docs, Sheets and Slides. Three verbs, and
they are not equally priced:

- **Create** is free in every model. `files.create` with the right MIME
  type, or `documents.create`. Nothing here is hard.
- **Edit** is the whole problem. Anthropic's first-party Drive connector
  cannot edit an existing Doc's body — which is exactly why the user's
  plugin exists — and editing an *arbitrary existing* document is the
  operation every scope tier prices differently.
- **Delete** should be **trash, never destroy**. This is the
  `gmail.modify` versus `mail.google.com` reasoning from the mail
  surface, verbatim: a triage verb driven by a model should not hold the
  one action with no undo.

## 2. The landscape

### 2.1 Google's own MCP servers — real, and unusable here

Google ships eight remote MCP servers, one per product
(developers.google.com/workspace/guides/configure-mcp-servers):

| Product | Endpoint |
|---|---|
| Docs | `https://docsmcp.googleapis.com/mcp/v1` |
| Sheets | `https://sheetsmcp.googleapis.com/mcp/v1` |
| Slides | `https://slidesmcp.googleapis.com/mcp/v1` |
| Drive | `https://drivemcp.googleapis.com/mcp/v1` |
| Gmail, Calendar, Chat, People | …`mcp.googleapis.com/mcp/v1` |

Three things rule them out, in descending order of how permanent they
are:

- **They are remote HTTP.** `mecha-core/src/mcp.rs` opens with "Minimal
  MCP client over stdio", and every field of `McpServerConfig` —
  `command`, `args`, `env`, `env_passthrough`, `sandbox`, `network` —
  presumes a child process. Adding an HTTP variant is not one codepath;
  it is a config type half of whose security fields become silently
  meaningless for the new variant. That is the silently-degrading-sandbox
  shape the whole project is organised against.
- **They are Developer Preview**, behind the Google Workspace Developer
  Preview Program, with no GA date published.
- **The Docs surface is two tools** — `read_doc` and `update_doc`. Slides
  is `read_presentation` / `update_presentation`. There is less here than
  the API underneath offers, and creation routes through Drive's
  `create_file` anyway.

Worth revisiting if mecha ever grows an HTTP MCP transport for other
reasons. Not worth growing one *for* this.

### 2.2 Third-party stdio servers

`taylorwilsdon/google_workspace_mcp` (3k stars, ~2.7k commits, MIT) is
the serious one: Python + `uv`, stdio **and** streamable HTTP, 12
services, 120+ tools, and it supports OAuth 2.0, OAuth 2.1 + PKCE, and
service accounts with domain-wide delegation. Docs alone is 19 tools
including tables, tabs and comments.

It would work today over `[[mcp]]`. What it costs is the thing the user
already objected to: a large third-party Python surface, 83 open issues,
covering twelve services when the ask is three, and a tool namespace
mecha does not control. Its own README flags the injection risk —
"emails, docs, and events can contain hidden instructions" — which is
correct and is §5 here.

### 2.3 The current plugin, and why it chafes

`@piotr-agier/google-drive-mcp` over a **service account**, credentials
at `~/.config/google-drive-mcp/service-account.json`, run via `npx` at
plugin-install time.

The service account is doing one job: **dodging OAuth verification
entirely**. A service account consents to itself, so no scope tier ever
applies. That is why it is the default choice for this problem across
the ecosystem, and it is a real answer. It costs four things:

- **Every document must be shared with the robot's address**, one at a
  time. (Sharing a whole *folder* removes most of this and is the
  cheapest available improvement to the status quo — no code at all.)
- **Documents it creates land in the service account's own Drive**, and
  service accounts have **no storage quota** — the documented
  `403 storageQuotaExceeded` on an apparently empty Drive
  (developers.google.com/workspace/drive/api/guides/limits, and the
  n8n and Google forum threads in Sources). The recommended fix is a
  shared drive, which is more setup, not less.
- **Edits are authored by the robot**, not by you. Revision history,
  comment attribution and "last edited by" all name a service account.
- `npx` fetches a package at runtime, so a working install is a function
  of the network and the registry on the morning you need it.

## 3. Scope tiers — the fact that decides everything

| Scope | Tier | What it grants | What it costs |
|---|---|---|---|
| `drive.file` | **non-sensitive** | see/edit/create/**delete** only files this app created or the user picked | nothing |
| `documents`, `spreadsheets`, `presentations` | **sensitive** | full access to all Docs / Sheets / Slides | verification: justification + demo video. No CASA |
| `drive`, `drive.readonly` | **restricted** | all of Drive | verification **+ ADA-CASA AL1**, ~$540/yr, re-assessed every 12 months |

Sources: developers.google.com/workspace/docs/api/auth,
…/sheets/api/scopes, …/slides/api/scopes, and
developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification.

Two consequences worth stating plainly.

**`drive.file` is a real scope, not a toy.** It carries delete. It works
with the Docs, Sheets and Slides APIs, not only Drive — all three scope
pages list it, marked "Recommended · Non-sensitive". A document in scope
is fully writable through `documents.batchUpdate` and friends.

**Restricted-tier costs are now unavoidable at that tier.** The Tier 2
self-scan was withdrawn; every restricted-scope app must pass a lab
assessment yearly. This is not a hypothetical for this repo — it is the
live reason `HANDOFF.md` records the `personal` Google account
re-consenting **every seven days**: the FlowMail Cloud project is stuck
in Testing/External because `gmail.modify` is restricted, and a Testing
app's refresh tokens expire after exactly seven days regardless of
refresh.

**The documents work does not have to inherit that.** Verification is
assessed against a *consent screen's* scope set, and a consent screen
belongs to a Cloud project. A second project asking only for
`drive.file` is a non-sensitive app, so it may be **published without
verification** — and publishing, not the tier itself, is what removes
the seven-day expiry. The distinction is load-bearing and easy to get
backwards: `drive.file` in a project left in *Testing* expires in seven
days exactly like `gmail.modify` does. §6.2 has the quotes.

## 4. The desktop Picker — the finding that changes the design

`drive.file` has always had one catch, and it is the reason nobody uses
it for agents: it covers files the app *created*, or that the user
explicitly *hands* it — and handing meant the Google Picker, a
JavaScript web API. There is no Picker in a terminal, so CLI tools
reached for a service account or for `drive`.

Google now documents a **desktop and mobile Picker** driven entirely by
OAuth query parameters
(developers.google.com/workspace/drive/picker/guides/desktop-mobile-picker):

```
https://accounts.google.com/o/oauth2/v2/auth
  ?client_id=…
  &response_type=code
  &access_type=offline
  &redirect_uri=http://127.0.0.1:PORT/callback
  &scope=https://www.googleapis.com/auth/drive.file
  &prompt=consent
  &trigger_onepick=true
  &allow_multiple=true
  &allow_folder_selection=true
```

The browser shows the real Google Picker. The redirect returns

```
…/callback?picked_file_ids=ID,ID&code=CODE&scope=…
```

— a comma-separated list of file ids alongside the ordinary
authorization code. Per-file access persists; the ids are the app's from
then on.

**This is mecha's existing sign-in flow plus two query parameters.**
`mecha-mail/src/google/auth.rs` already builds an authorization URL with
PKCE, already runs a loopback listener on its own port, and already
exchanges the code. `build_auth_url` grows the parameters; the callback
handler reads one more field.

**The grant and the picking separate, and only one of them needs a
browser it can reach.** Google's device-code flow
(developers.google.com/identity/protocols/oauth2/limited-input-device)
permits exactly six scopes — `openid`, `email`, `profile`,
`drive.appdata`, two YouTube scopes, and **`drive.file`**. So the token
can be minted with no redirect at all, the way the Microsoft mail
account already signs in: a code typed on any other device, no forwarded
port, works over SSH. But the device flow has no redirect *by
definition*, so `picked_file_ids` has nowhere to arrive — the picker
structurally requires the loopback.

That is a two-command surface rather than one, and the split falls in a
useful place:

| | mints a token | needs a reachable loopback | covers |
|---|---|---|---|
| `docs auth` (device code) | yes | **no** | everything mecha creates — the common case |
| `docs pick` (loopback + `trigger_onepick`) | yes | yes | adopting documents that predate mecha |

The consequence worth stating plainly: **the frictionless majority of
this feature needs no browser reachability at all.** A headless box can
run `docs auth` over SSH and immediately create and edit its own
documents forever. Reaching a *pre-existing* document is the only
operation that ever needs a tunnel or a local browser, and it is a
one-time cost per document rather than per run.

Open question for the console step: whether the device flow requires the
separate "TV and Limited Input devices" client type or works against the
Desktop-app client the picker needs. If it needs its own, the project
carries two client ids — which is bookkeeping, not a problem, but it
should be discovered before the auth path is written rather than after.

Two limits, both real:

- **`trigger_onepick` cannot be combined with any other scope.** The
  documentation is explicit. So this is its own grant, separate from the
  mail grant — which is fine, and is another reason it wants its own
  Cloud project.
- **`allow_folder_selection` picks a folder**, but the Picker "doesn't
  allow users to organize, move, or copy files". Whether a picked folder
  extends scope to its future children is the one behaviour to measure
  before promising it in a UI; the answer decides whether "put mecha's
  documents here" is one grant or one grant per document.

## 5. The security shape

A documents surface is **all three trifecta legs at once**, and it is
worse than mail on the third:

- **`untrusted_input`.** A shared document is other people's text, and a
  document *comment* is a better injection vector than an email body:
  it is invisible in the rendered doc, it is short, and it is attributed
  to a human.
- **`private_data`.** Self-evident.
- **`external_send` — and this is the leg people miss.** Writing into a
  document a third party can read *is* exfiltration. It reads like a
  local edit and it is a publish. The same reasoning that makes
  `http_fetch` a send sink despite being read-only applies here with far
  more bandwidth than a query string.

So the labelling, following the mail surface's three quadrants:

| Verb | Capabilities | Where it sits |
|---|---|---|
| `docs_read`, `sheets_read`, `slides_read` | `untrusted_input`, `private_data`, `readOnlyHint`; result `.from_outside()` | arms the interlock; never a sink |
| `docs_update`, `sheets_update`, `slides_update`, `*_create` | `private_data`, `external_send`, `openWorldHint` | **`[outbox] tools`** — staged, never sent |
| `docs_trash` | `private_data`, `destructiveHint`, **not** `external_send`, **not** `readOnlyHint` | the approver, not the interlock — the `mail_triage` quadrant |

Two further rules that fall out of the mail precedent:

- **No sharing or permissions verb, at least not first.** Changing who
  can read a document is the one action where a successful injection
  costs the entire corpus rather than one file. It is also the action
  `drive.file` would happily permit, so the boundary here has to be the
  tool surface rather than the scope.
- **Deletion is trash.** `files.update` with `trashed: true`, never
  `files.delete`.

And the load-bearing structural point: **`drive.file` plus the Picker is
a path jail for Drive.** Every other option asks the model's tool
surface to be trusted with the whole account and then constrains it by
prompting or by approvals. Here mecha *cannot* reach a document the user
did not pick, and you verify that by reading a scope string rather than
by reviewing a diff — the same argument that makes `ToolCtx::resolve`
worth having, and the same inversion recorded in the
"invert before declaring impossible" note.

## 6. The two probes

Everything above is documentation. Two claims were load-bearing enough
to measure, and both were run on 2026-08-18 against Google's live
endpoints. Results are in §6 of this file rather than in prose above,
so a reader can tell measurement from citation.

### 6.1 Does `trigger_onepick` accept a loopback redirect?

**Motivation.** Every other Picker integration is a web page with a real
origin. mecha has no origin — it listens on `http://127.0.0.1:<port>`,
picked at runtime. If `trigger_onepick` refused loopback redirects the
whole design collapses back to a service account.

**Method.** Seven unauthenticated `GET`s at
`https://accounts.google.com/o/oauth2/v2/auth`, varying one parameter at
a time, reading the `authError` protobuf off the 302. No consent screen
was completed and no token was issued. The client id used is the public
Google Cloud SDK desktop client (`32555940559.…`), because the machine's
own client id lives in a credential file that should not be read to run
a probe.

| # | redirect_uri | `trigger_onepick` | error returned |
|---|---|---|---|
| A1 | `http://127.0.0.1:8765/callback` | absent | `restricted_client` — *Unregistered scope(s): …/drive.file* |
| A2 | `http://127.0.0.1:8765/callback` | `true` | `restricted_client` — *Unregistered scope(s): …/drive.file* |
| A3 | `http://127.0.0.1:8765/callback` | `true`, scope combined with `gmail.readonly` | `restricted_client` — *Unregistered scope(s): gmail.readonly, drive.file* |
| A4 | `http://example.com/cb` | `true` | **`redirect_uri_mismatch`** |
| A5 | `http://localhost:9999/x` | `true` | `restricted_client` — *Unregistered scope(s): …/drive.file* |
| A6 | `http://localhost:9999/x` | replaced by a bogus parameter name | `restricted_client` — *Unregistered scope(s): …/drive.file* |
| A7 | `https://example.com/cb` | `true` | **`redirect_uri_mismatch`** |

**What this establishes.** A4 and A7 carried the *same* unregistered
scope as A2 and A5 and still failed on the redirect, which fixes the
order: **`redirect_uri` is validated before `scope`.** So reaching a
scope error is proof the redirect was accepted. A2 and A5 reach it, on
two different loopback hosts and two arbitrary ports, **with
`trigger_onepick=true` present**. A4 and A7 show the check is real and
fires — the negative is not vacuous.

> `trigger_onepick=true` does not disable loopback redirects. mecha can
> keep the listener it already has, on the port it already picks.

**What this does not establish**, and the honest limit of the probe: A6
substitutes a nonsense parameter and returns byte-identical output, so
this method cannot tell *honoured* from *silently ignored*. The gcloud
client has a fixed scope allowlist that `drive.file` is not on, so every
request dies at scope validation before any Picker behaviour is
reachable. Settling that needs a client id with `drive.file` registered
— i.e. §6.2's project — and one interactive sign-in where the answer is
simply whether a file chooser appears and whether the redirect carries
`picked_file_ids`. That is a two-minute test once the project exists,
and it is the first thing to run.

A3 is likewise inconclusive on the documented "cannot be combined with
any other scope" rule: it failed on both scopes being unregistered
rather than on the combination.

### 6.1b Measured: the picker runs, end to end

**2026-08-18, resolved.** §6.1 could not distinguish *honoured* from
*silently ignored*. A real project settles it.

Setup: Cloud project `mecha-docs`, one **Desktop app** OAuth client,
consent screen carrying `drive.file` and nothing else (sensitive and
restricted lists both empty), publishing status **Testing** with the
signing-in account added under Test users. The probe ran on the headless
workstation; the browser was on a laptop, reaching the listener through
`ssh -L 8765:127.0.0.1:8765`.

First attempt returned `error=access_denied` with no code — the account
was not yet a registered test user. Worth recording because it is *not*
the failure it looks like: §6.1 established that a misconfigured scope
returns `restricted_client`, so `access_denied` positively indicated the
scope set was correct and the refusal was about **who may consent**.
Adding the test user was the whole fix.

Second attempt, after picking one document:

```
state:            (echoed unchanged)
iss:              https://accounts.google.com
picked_file_ids:  13ISYxLr2KqwgOpbKioH7gFpoqgpH1AqbjpujvpTmvx4
code:             present (73 chars)
scope:            https://www.googleapis.com/auth/drive.file
```

**A real Google file chooser rendered, and the id came back on a
loopback redirect** — on a machine with no browser, through an SSH
tunnel, under a scope that costs nothing. Every element the design in §7
depends on is now measured rather than cited:

- `trigger_onepick=true` is honoured for a Desktop-app client;
- the loopback redirect carries `picked_file_ids` alongside the ordinary
  authorization `code` (confirming §6.1's inference from error ordering);
- the flow survives an SSH tunnel, which is the arrangement `docs pick`
  will use permanently, since the picker cannot use device code (§4).

Two notes for whoever writes the auth path. An `iss` parameter comes
back that the documentation does not mention — ignore it or verify it
equals `https://accounts.google.com`, but do not fail on its presence.
And in Testing the consent screen shows an "unverified app"
interstitial requiring *Advanced → Go to …*, which a scripted flow
cannot click; another reason publishing matters beyond token lifetime.

**Still open: the folder question.** This run picked a single loose
document, so whether `allow_folder_selection=true` puts a folder's
*contents* in scope — the difference between one grant and one grant per
document — remains unmeasured. It is the same probe with a folder
selected, plus one `documents.get` against a child to prove the grant
reaches through. Until that is answered, the honest claim is only that
picking works per-document.

### 6.1c Measured: the token works; the folder answer is probable, not proven

Four further runs against `mecha-docs` on 2026-08-18, exchanging the
authorization code for a real token (held in memory, never stored).

**Settled.**

- **The code exchanges.** `grant_type=authorization_code` against the
  loopback redirect returns a token with `refresh_token: yes` and
  `scope: https://www.googleapis.com/auth/drive.file` — so the picker
  flow yields a durable grant, not a one-shot.
- **Picked items are readable through the Drive API**, across kinds: a
  folder (`UndergradCommittee`, `Writing`), a document (`Py-FEAT v2.0
  Manuscript`) and a **spreadsheet** (`PsychUndergradCourses`) all
  resolved by `files.get`. The picker is not Docs-only, which matters
  given the ask spans Docs, Sheets and Slides.
- **Re-picking a file returns the same id**, so a pick is idempotent and
  a second grant does not fork the identity of a document.

**Probable, with a named residual doubt: a folder grant does not reach
its contents.** Two runs, two folders, `files.list` with the folder as
parent returned zero children. The second run closed the obvious
confound — `Writing` is in **My Drive**, not a shared drive, and the
query carried `supportsAllDrives=true&includeItemsFromAllDrives=true`,
so a shared-drive blind spot cannot explain it. (The first run's folder
had a legacy `0B…` id, which is shared-drive shaped; that run alone
would have been uninterpretable, and the probe printed a verdict it had
not earned. Recorded because the mistake is instructive: an empty result
means nothing until you have shown the query could have returned
something.)

What remains unproven is that either folder *contained* a Google Doc.
`drive.file` is precisely the scope that prevents checking, so the
negative cannot be validated from inside the measurement. The designed
fix is to pick **a folder and a document inside it** in one flow: the
child is then in scope by direct pick, so the listing must return at
least that one if the mechanism works at all, and any siblings missing
is a real negative. That run has not been completed — the attempt
returned a lone spreadsheet — and it is the one measurement this
section still owes.

Treat "no reach-through" as the planning assumption. It is consistent
with the documentation (`drive.file` is defined per-file, and the Picker
guide notes it "doesn't allow users to organize, move, or copy files"),
and it is the conservative direction: designing for per-document picking
and discovering folders work is a pleasant surprise, while the reverse
ships a UI promising something that does not happen.

**Still untested: the Docs API on a picked document.** Every read so far
went through `drive/v3/files`. `documents.get` — the call `docs_update`
actually makes — has not been exercised, because no run picked a
document while the check was in the script. Google lists `drive.file` as
a valid Docs API scope, so this is expected to work; it is simply not
yet observed, and it is a one-liner once a token is stored rather than
held in memory.

### 6.2 Can a `drive.file`-only project publish without verification?

**Motivation.** This is the seven-day-token question. If the answer is
no, the documents grant inherits the mail grant's weekly re-consent.

**Method.** Originally documentary. **Settled empirically on
2026-08-18**: `mecha-docs` was created with `drive.file` as its only
scope and **published to In production with no verification and no
review**. The console's own Verification Center states it in as many
words:

> *Data access status* — "Verification is not required since your app is
> not requesting any sensitive or restricted scopes."

So the citation below is now confirmed by observation, and the seven-day
expiry is gone with it.

**One trap on the way, worth recording because it cost an hour.** Before
publishing, the console showed a banner reading "Your app requires
verification. When you have finished configuring your information,
please submit your app for review" — on a project whose sensitive and
restricted scope lists were both visibly empty. That banner is
**brand verification**, not scope verification, and the two are distinct
tracks that the Verification Center finally separates on screen:

| Track | Triggered by | Costs | Governs |
|---|---|---|---|
| Scope verification | a sensitive or restricted scope | review; CASA (~$540/yr) if restricted | whether the app may request the scope at all |
| Brand verification | wanting a name and logo on the consent screen | domain ownership; no video, no CASA | whether the name and logo *render* |

The brand warning does not clear by deleting the logo, because the check
is about branding being *verified*, not about branding *existing* — it
persists as "your branding is not being shown to users", which is a
description of the resulting state rather than a fault. It blocks
nothing. Read generically, though, it is easily mistaken for the
expensive track, and the natural response — "submit for review" — starts
a weeks-long process the app does not need and resets on any scope
change. **Do not submit.** Publish, and read the Verification Center's
two cards rather than the banner.

**Answer: yes, and the reason is narrower than it first looks.** Two
statements from Google, and the interaction between them is the whole
result:

- *"If your app utilizes only non-sensitive scopes, it is not mandatory
  for your app to complete the app verification process."*
  (support.google.com/cloud/answer/13463073). The unverified-app warning
  is likewise triggered only by sensitive or restricted scopes.
- *"A Google Cloud Platform project with an OAuth consent screen
  configured for an external user type and a publishing status of
  'Testing' is issued a refresh token expiring in 7 days, unless the only
  OAuth scopes requested are a subset of name, email address, and user
  profile."* (developers.google.com/identity/protocols/oauth2)

**The correction that matters:** the seven-day exception is *not* "any
non-sensitive scope". It is specifically name/email/profile.
**`drive.file` in a Testing project still expires in seven days.** What
saves it is not the tier directly — it is that the tier permits
publishing, and publishing is what removes the expiry. A new project
left in Testing reproduces the exact problem it was created to escape.

**And the coupling runs both ways, on a client that is already large.**
Google grants are per *(user, client)*, not per scope, so revoking a
token revokes every scope that client holds
(github.com/googleworkspace/drive-picker-element#47). FlowMail already
carries four delegated Google scopes — `gmail.modify`, `gmail.send`,
`calendar`, `calendar.events` — so its blast radius is already all of
the user's mail and both calendars. Adding documents to it means
revoking documents takes mail and calendar down, and revoking mail takes
documents down. On a client that also cannot leave Testing, that is a
coupling with no compensating benefit: the shared project pays mail's
weekly re-consent tax on a scope that does not owe it.

So the requirement is a conjunction, and both halves must hold:

1. a **separate** Cloud project, because verification is assessed
   against a consent screen's scope set and FlowMail's contains
   `gmail.modify`; and
2. that project **published to In production**, not left in Testing.

Also recorded from the same page, since it bounds any design that mints
grants per document: **100 refresh tokens per Google Account per client
id**, oldest silently invalidated past that. A picker flow that
re-consents on every grant must therefore reuse one token and store the
file ids beside it, never mint a token per document.

**Left for the user**, being console work on their own account: create
the project, add a Desktop-app OAuth client, list only `drive.file`,
publish, then run §6.1's interactive test. If the chooser appears and
`picked_file_ids` comes back on the loopback redirect, the design in §7
is confirmed end to end.

### 6.4 Measured during implementation

Building it broke two things the research had assumed. Both are recorded
here rather than quietly fixed, because both were derived from
documentation that is individually accurate.

**There is no device-code path, and §4's two-command split does not
survive.** `drive.file` really is one of the six scopes Google's
limited-input flow permits — but the flow **refuses a Desktop-app
client** (`401 invalid_client: "Invalid client type."`), and
`trigger_onepick` is accepted for *no other* client type. Two client ids
do not rescue it: a `drive.file` grant is per *(user, client)*, so files
picked under the Desktop client are invisible to a TV client's token,
and the two would hold disjoint scopes with `pick` extending a grant
`auth` could never read. **One client must do both, and it must be
Desktop-app.** The device-code implementation was deleted rather than
left behind a flag; code that reads as an available option and cannot
work is worse than its absence.

**What replaced it costs nothing and is strictly more general.** The
value device code carried was headless sign-in. The browser leg is
unavoidable, but *receiving* the redirect is not: a browser displays the
whole `127.0.0.1` address even when the connection fails, which is how
§6.1c's ids were read in the first place. `--paste` prints the URL,
takes the resulting address back on stdin, and needs no tunnel, no
forwarded port and no browser on the machine holding the grant. The
listener remains the default where a loopback is reachable.

**Two smaller things, both confirmed against the live grant.**
`GET /drive/v3/about?fields=user(emailAddress)` answers under
`drive.file`, so the account can be named without the profile scope the
mail flow uses — worth knowing, because an unnamed grant is one a human
cannot tell from another. And **per-file grants accumulate on the client
across separate consents**: files picked during the §6.1 probes are
visible to the shipped binary's grant, because it is the same client id.
That is what makes `pick` incremental rather than replacing scope each
time, and it is why no local index of picked ids exists — a Drive
listing under `drive.file` returns exactly the in-scope files, so Google
is the record and a second copy could only drift.

## 7. What this means for mecha

**Build it into `mecha-mail`** — a `google/docs.rs` beside `gmail.rs`
and `calendar.rs`, plus a fourth binary `mecha-docs` alongside the
existing three.

**Not a separate crate, and not a separate repository.** The rule this
repo actually uses is that a crate exists to make an invariant checkable
in `Cargo.toml` rather than by reviewing diffs — that is stated outright
for `mecha-slack`, whose whole reason for being a crate is that "no
`mecha-core` dependency, ever" can be verified by reading a file. A
documents crate would enforce no new invariant, and it needs the token
lifecycle, so it would either duplicate that (two implementations of
refresh-under-lock, the exact bug class this codebase is organised
against) or take a semantically backwards `docs → mail` dependency.
`mecha-mail` is already one library behind three thin MCP binaries;
this is the fourth. It also already has no `mecha-core` dependency, so
the isolation property is inherited rather than re-argued. A separate
*repository* is what `mecha-graph` needed for public release, which is
a publishing decision and not an architectural one.

The honest cost is that the crate name stops describing its contents:
`mecha-mail` becomes the Google/Microsoft personal-data surfaces rather
than mail and calendar. That is naming debt, and the same change should
update `CLAUDE.md`'s crate list rather than leave the description wrong.

**The principled alternative, and when to take it.** Extract
`mecha-oauth` — `StoredCredentials`, the refresh lock, the loopback
listener, device code — so mail and documents become peers over a shared
credential layer. Two consumers is exactly when an extraction earns
itself, so this is not a hypothetical. It is deferred for a sequencing
reason rather than a design one: `token.rs` is under active change (the
`granted_scopes`/`granted_at` work landed 2026-08-18), and extracting
beneath an in-flight session trades a clean boundary for a merge
conflict. Revisit when a third consumer appears or the coupling starts
to chafe; nothing here forecloses it.

The expensive parts already exist and are already debugged:

- `token.rs` (658 lines): refresh-ahead-of-expiry behind a lock, forced
  refresh and retry on 401, mode-0600 storage, backoff on 429/5xx.
- `google/auth.rs`: PKCE, the loopback listener, `access_type=offline`
  and `prompt=consent`, the post-consent identity probe.
- `accounts.rs` and `unified.rs`: the named-account model, and the
  "the model names an account, never a provider" surface rule.
- `doctor.rs`: the place a grant that stopped working gets reported.

What is genuinely new is three REST surfaces that are each one
`batchUpdate`, a `files.create`, a `files.update` for trashing, and a
`grant` subcommand that is the existing auth flow with two extra query
parameters and one extra field read off the callback.

**One trap on the way in, found by the mail work rather than by this
research.** `mecha-mail`'s Entra refresh path was sending its whole
`SCOPES` list on every refresh, so widening a scope list asked for a
superset of the stored grant and came back `invalid_grant` — a
revocation that had not happened, reported as one. Fixed in `2043a8f`;
`google/auth.rs` never had it. The rule any new grant path inherits:
**send scopes when minting, none when renewing** (RFC 6749 §6). It bites
hardest here because the picker grant is minted separately from the mail
grant and stores its own token, so there are two mint sites and two
renew sites where there used to be one of each.

**Two more, from the same source.**

*Reuse the credential **type**, not the credential **directory**.*
`StoredCredentials` records `granted_scopes` verbatim and `granted_at`
(stamped at consent, never touched by a refresh), and carries the
refresh lock, the serde defaults, the 0600 handling and the marker
convention. All of that is worth having, and none of it is tied to a
path.

The path is a different question, and the first instinct — a sibling
file in `~/.mecha/mail/<account>/` — is wrong. `mecha doctor` globs
`~/.mecha/mail/*/` and two checks read the grant in each account
directory *as the mail grant*: one asserts `granted_scopes` contains the
provider's triage scope (`gmail.modify` / `Mail.ReadWrite`) and reports
"cannot archive, spam or mark mail read" otherwise, the other reads
`granted_at` against the account's declared `grant_lifetime_days`. Both
open the file named exactly `oauth.json`. So a sibling named anything
else is **invisible** to both — the coverage this was supposed to buy
never arrives — and a sibling picked up by any future loosening of that
glob is **worse**: a `drive.file` grant fails the triage-scope check and
gets reported as a broken mail account, which is a finding nobody can
explain and that names the wrong subsystem.

So: **`~/.mecha/docs/<account>/oauth.json`, same type, own root.**
Doctor coverage then arrives as a new check reading a new directory,
rather than as a change to a check that already has two callers keyed on
"a directory under `mail/` is a mail account". The alternative — keeping
the sibling and teaching the check which grant is which, via a `purpose`
field or a filename convention — is a real option, but it modifies an
existing invariant to save creating a directory, which is the wrong
trade in both directions.

The generalisation, since it will come up again the next time a second
grant appears: the reuse that is safe here is *structural* — a type,
its invariants, its locking. The reuse that is not is *locational* — a
namespace whose meaning other code already depends on. Sharing the first
costs nothing; sharing the second silently recruits every existing
reader of that namespace into your new case.

One field that differs, and it turns out to need no work: a published
`drive.file` project has no `grant_lifetime_days` at all. That must
record as **absent**, never as some large number, or "this grant does
not expire" stops being distinguishable from "nobody measured" — the
taint snapshot's rule, that unknown must never masquerade as safe.
`grant_lifetime_days` is already `Option<u32>` and `check_grant_age`
returns no finding when it is `None`, so the semantics wanted here are
the ones that already exist.

*If any of this ever runs from a systemd user unit, set
`WorkingDirectory` explicitly.* A unit with none runs in `$HOME`, which
contains `~/.mecha`, and `prepare_tools` refuses a workspace the mecha
home sits under — the Security-model rule, arriving as a first-run
failure rather than as a warning. The mail classify timer hit exactly
this today and was fixed with an explicit
`--workspace ~/.mecha/work/<producer>`.

Phasing, cheapest first:

1. **`mecha-mail docs grant`** — the picker flow, storing picked ids
   beside the token. Nothing else needs to exist for this to be
   testable.
2. **Read tools**, marked `from_outside`. This is where the injection
   surface arrives, so it arrives alone.
3. **Create and update**, routed through `[outbox] publish_tools`-style
   staging from the first commit, never added afterwards.
4. **Trash**, with the approver.

Deliberately absent from the plan: sharing verbs, comment authoring,
Apps Script, and anything that needs the `drive` scope.

**The interim answer, if this is wanted before it is built:** wire
`taylorwilsdon/google_workspace_mcp` over `[[mcp]]` with `sandbox =
true`, `network = true`, and an `[mcp.capabilities]` override forcing
`untrusted_input`. That is under an hour and turns build-versus-buy into
evidence — which is what the deferred-integration note said to do before
rewriting, and it was right.

## 8. Left out on purpose

- **Domain-wide delegation.** It impersonates any user in a tenant,
  Google's own guidance is to avoid it where a normal grant will do, and
  Dartmouth's tenant is not the user's to widen.
- **The `drive` scope.** $540/yr and an annual audit to gain "documents
  you did not pick", which is the access this design is deliberately
  refusing.
- **Office file editing** (.docx/.xlsx/.pptx in Drive). Different
  problem, different toolchain; see SLIDES-RESEARCH.md §3 and the
  deferred pptx work.
- **A GUI file browser inside mecha.** The Picker is Google's, it is
  already written, and it is the surface the user's own account already
  trusts.

## Sources

- Configure the Google Workspace MCP servers — developers.google.com/workspace/guides/configure-mcp-servers
- Choose Google Docs API scopes — developers.google.com/workspace/docs/api/auth
- Choose Google Sheets API scopes — developers.google.com/workspace/sheets/api/scopes
- Choose Google Slides API scopes — developers.google.com/workspace/slides/api/scopes
- Choose Google Drive API scopes — developers.google.com/workspace/drive/api/guides/api-specific-auth
- Integrate the Google Picker into desktop and mobile apps — developers.google.com/workspace/drive/picker/guides/desktop-mobile-picker
- Overview of desktop and mobile apps (Picker) — developers.google.com/workspace/drive/picker/guides/overview-desktop
- Restricted scope verification — developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification
- Drive API usage limits — developers.google.com/workspace/drive/api/guides/limits
- taylorwilsdon/google_workspace_mcp — github.com/taylorwilsdon/google_workspace_mcp
- ljchang/google-docs-plugin — github.com/ljchang/google-docs-plugin
- Google OAuth refresh token expiration — unipile.com/google-oauth-refresh-token/
- CASA cost, first-hand — yurudeep.com/posts/aicoding/2026/20260717/en/
- Service accounts have no storage quota — github.com/n8n-io/n8n/issues/26050, discuss.google.dev/t/194265
- OAuth App Verification Help Center (non-sensitive scopes need no verification) — support.google.com/cloud/answer/13463073
- Using OAuth 2.0 to Access Google APIs, refresh-token expiration — developers.google.com/identity/protocols/oauth2
- Unverified apps — support.google.com/cloud/answer/7454865
