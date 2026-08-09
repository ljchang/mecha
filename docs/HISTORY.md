# mecha — HISTORY

The record of what was built, when, and what was learned the hard way. Split
out of [`HANDOFF.md`](HANDOFF.md) so that document can stay a description of
the current state instead of an accumulating log.

Nothing here is a plan. The design decisions that still govern the code live in
[`../CLAUDE.md`](../CLAUDE.md); what remains here is the evidence behind some of
them, plus the incidents that produced the rules. Where a note has been
superseded, it says so rather than being deleted — a retracted measurement is
still worth knowing about, because the next person will otherwise re-derive it.

---

## What shipped, and when

**2026-08-02 — the harness.** The first day put the whole spine in place: the
provider-agnostic message types, the Anthropic and OpenAI-compatible backends,
the tool registry and its approver, and the agent loop itself. `mecha eval`
landed the same day as the bake-off rig, and the trifecta interlock was
hardened almost immediately after — capabilities on tools, taint on the run,
and a refusal of any `external_send` once private and untrusted data were both
present. Subagents, a forced final-answer turn, a default system prompt, the
`todo` tool, web search with its own leak guard, and token and cost budgets all
arrived before the day was out. So did the first real bake-off numbers, a
grader fix for a check that was measuring formatting rather than content, and
the observation that the case set had already saturated. The day closed with
`docs/HANDOFF.md` and the model start scripts checked in.

**2026-08-03 — confinement, concurrency, and the TUI.** `shell` and MCP servers
stopped being merely labelled and started being confined (`sandbox.rs`, bwrap
and docker backends). `RunContext` split what belongs to a *run* — the jail, the
approver, the budget, a cancellation token, a steering queue — from what belongs
to the agent, which is what made concurrent differently-jailed runs possible and
made interruption and steering expressible at all. `mecha tui` shipped on top of
that, with an input line that stays live while the agent works. The eval rig
gained sandboxed workspaces, real test runs via `expect.verify`, and an LLM
judge. Taint moved from the run to the `Conversation`, closing the hole where
pressing Enter reset the interlock. Compaction landed, and then — more usefully —
was *measured* rather than asserted, which is where the record below begins. The
Anthropic provider was verified against the live API. Late in the day the TUI
grew slash commands, mid-session model/provider/permission/MCP switching, real
menus, `ask_user`, and phase-gated planning on Shift+Tab; config gained the
ability to distrust an MCP server further than the server declares itself; and
`thin_old_results` shortened old tool results before anything was summarised
away.

**2026-08-04 — learning, hooks, the outbox, and mail.** The day opened by
measuring the todo-list idea and concluding no (below), pinning the sampler, and
building the replay driver so a recorded trajectory could be re-run against
recorded tools. Then the memory work: several design passes (Reflexion split
into two systems and three stages, nightly rumination, scoped rules, a gated
hyperagent layer, and a review of flowmail's own drafting learner) followed by
the implementation — the learning store, the transcript miner, rules injection,
`mecha learn` to consolidate reflections into rules, and `mecha validate`, whose
first probe caught the store's first false lesson. Hooks attached to the loop
between the interlock and the human. Rumination ran nightly on a timer, with a
proposal gate so unattended learning never applies its own output. The outbox
shipped: outbound calls staged for review, and the edits mined back as writing
lessons. `mecha-google` was extracted from flowmail, Outlook and Graph followed
with device-code auth (and turned up a real interlock hole), and the agent
learned its context window and its timezone. The day ended with three research
passes: the context-management evidence, the verification backlog, and
sandboxing.

**2026-08-05 — measurement, hardening, the TUI push, triggers, and going
public.** `mecha eval --runs k` reported pass^k beside pass@k. Compaction gained
superseded-result eviction, summary validation against the transcript being
replaced, and a summariser with its own budget; the Anthropic backend gained a
moving cache breakpoint so the message history caches too. `-np 1` was pinned on
both llama-servers after the default quietly quartered the context and
confounded a week of scorecards. Provider failures were classified, transient
ones retried, and fallbacks made deliberate. Learning gained provenance gating,
a validation ledger, rule tenure and gated retirement, and a hard cap on the
always-loaded block. A long TUI sequence landed — nested subagent rendering,
Shift+Enter via the kitty protocol, synchronized output, a live tool-output
toggle, a `?` overlay, a `/tools` modal, `!command`, `@path` completion, a live
todo pane, `^G` to compose in `$EDITOR`, and the first rendered-frame tests on
ratatui's `TestBackend`. `mecha distill` turned closed sessions into episodes
staged to the knowledge graph; `mecha-mail` unified every mailbox behind one
account-based surface. A security pass fixed header smuggling in drafted mail,
vetted outbound addresses, capped tool output as it streamed, and made the
on-disk stores owner-only. Triggers shipped last: five-field cron, a store and a
ledger, a CLI where `tick` is the primitive and `daemon` a loop over it, and a
`/triggers` modal in the TUI.

The repository went public under the MIT license and was tagged **v0.1.0** on
2026-08-05, with CI, a documentation site, and a changelog alongside it.

**2026-08-06 (later) — the public surface, built and measured.**
`mecha-factory` was created as its own repository and taken through build steps
1–5 of `PUBLIC-SURFACE-DESIGN.md` §12 plus the MCP surface, 104 tests. In order:
`mecha-manifest`, the versioned data contract that turns one TOML request type
into a JSON Schema, an HTML form and the validator both ends run; a
content-addressed immutable bundle store with a moving alias, and a markdown
`report` template; the external-reference gate, which **fails** a publish rather
than warning and distinguishes a link a reader clicks from a resource the page
fetches; the `notebook` template on `marimo export html-wasm`; and an MCP server
whose seven tools mecha reaches with two config blocks.

The part worth the day was step 4. `marimo export html-wasm` is not
self-contained — it loads Pyodide, the standard library and every wheel from
three hosts at runtime — so a vendorer fetches from a hardcoded allowlist,
verifies each wheel against the sha256 in Pyodide's own lock file, caches per
version and copies into each bundle. Verified in a browser rather than asserted:
a notebook boots and computes under the full compute CSP with **zero off-origin
loads**. Two design corrections came out of measuring instead of quoting notes —
`unsafe-eval` was never needed, and §7.3's `data:` URL problem belongs to the
islands path and not to ours.

The loop closed end to end the same day: an agent asked to publish got "drafted,
not sent", `mecha outbox show` led with the rendered page, `edit` was refused
naming the real action, and `send` executed the call and landed an immutable
version with its source recorded.

**2026-08-06 — the work directory, and closing the jail default.** The
prerequisites the public-surface design exposed, built ahead of `mecha-factory`
itself. `~/.mecha/work/<producer>/` became a run's workspace (`work.rs`,
`mecha work list/path/clean`), which closed four open items with one change: it
roots the path jail somewhere holding nothing sensitive, gives an unattended run
a durable artifact, makes yesterday's output an ordinary file in today's run,
and replaces the morning trigger's `mkdir -p && cat >` improvisation. `setup`
now refuses any workspace that contains the mecha home — the bug was live for
`mecha chat` from `$HOME`, not just for triggers, and the shipped `morning`
trigger was safe only by accident of its tool allowlist. Retention shipped with
it (keep the last `[work] keep` = 10 per producer, run nightly, protected
sources named rather than skipped silently), settling open decision §13.3.

The outbox gained `OutboxKind`, which is the half of §2.2b that had to land
*before* anything stages a publish rather than after: `show` leads with the
rendered page, `edit` is refused with the real action named, and the
writing-reflection miner excludes publishes. The last one is why the ordering
mattered — a `writing` reflection becomes a rule in every future run's cached
prefix, so mining a changed directory path would have taught voice rules from
bookkeeping, and the damage would have been retroactive by the time anyone
noticed. Same class of mistake as learning from `"Blocked by a hook:"`, and it
carries a test named on it for the same reason.

**2026-08-06 (later still) — the factory's server, the scheduler, and the plan
that stopped evaporating.** Three things, of which the middle one is the only
one that had to be *run* to be found.

`mecha-factory` reached §12 step 6: three origins under three CSPs told apart
by `Host`, two Argon2id-hashed scoped keys, SQLite as the index with the bytes
on disk, its own ACME over TLS-ALPN-01, and a home side that pushes. Built and
verified end to end against a real server; never yet deployed. Three findings
came out of building it, all recorded in that repository's history: `visibility`
stopped being decorative and is now enforced (a private bundle answers what a
nonexistent one answers, byte for byte); the manifest's `sources` array had to
be stripped at the boundary, because `bundle.json` is itself served publicly and
that array holds absolute paths inside the user's home; and `unpublish` was
flipping visibility to private as well as clearing the version, which made the
honest "this has been taken down" page exist and be unreachable.

**`mecha trigger daemon` was installed**, which had been three lines and a
blocker for two days. It fired the morning briefing for that day's 07:00 slot
within a second of starting — `catch_up = 3h` and the slot was two and a half
hours old — which is exactly the designed behaviour and had never once happened
unattended. See the trap below for what the first real run found.

**A tool's own state now crosses a compaction.** The `todo` list only reached
the model through the echo in the last `todo` result, which is a message, and
therefore exactly what a compaction summarises away — so the mechanism was
quietly conditional on the transcript never getting long, in the one situation
the list matters most. `Tool::carried_state` lets any tool hand state to the
compaction to be carried verbatim; the loop learns that some tools have state,
never which. Exactly one copy survives a second compaction, because two
contradictory task lists in a prompt are worse than none.

**Batch review landed in the outbox**, which had taken exactly one id per verb
— so an overnight triage staging nine replies was nine invocations and nine
startups of every configured MCP server. The tension worth naming is that bulk
approval is how a review queue becomes a rubber stamp, so the batching saves
invocations and never saves reading: `outbox review` walks the pending items
one at a time, with the draft and its taint warning in front of you at the
moment you decide. Ids may be given several at a time, `--all` is narrowed by
`--kind` and `--via`, and the selection rules are a pure function with a test
each — a selection naming nothing is an error rather than "everything", and a
filter matching nothing is an error too, because a typo'd filter acting on zero
items reads exactly like an empty queue.

**And the factory went from built to deployed.** Step 6.5 made it
multi-tenant — a user owns every row, and their artifacts are served from
`<handle>.art.…` rather than a path prefix, because origin is the only
isolation a browser enforces and a published URL has to stay resolvable
forever. Step 7's server half followed: a form rendered from the manifest, a
single-use verification link, and a state machine where nothing reaches the
queue until somebody clicks. Then it was actually stood up — a small VPS
serving the gate and the first tenant's artifact origin
under a Let's Encrypt certificate the binary obtained for itself over
TLS-ALPN-01, with the first bundle published from this machine to that one.
The design doc gained §14 (multi-tenancy, the two axes, the request-path
invariant, the attachment lease) and §15 (users, handles never reused,
withhold versus purge, the outbound-mail abuse surface); the deployment
procedure and its two traps live in that repository's `docs/DEPLOY.md`.

**2026-08-06 (last) — the inbound half, and the quarantine at the centre of
it.** The drain client landed on the home side: `mecha-factory-publish drain`
speaks `GET /v1/queue` with the scoped key and writes each row into
`~/.mecha/requests/`. A CLI rather than a tool on purpose — the common case is
"nothing new", and it has to cost zero tokens and no model.

Then `mecha frontdoor` (`frontdoor.rs`, `commands/frontdoor.rs`), which is
everything that happens to a stranger's request afterwards and exists to serve
one sentence: **the privileged run sees the extraction, never the prose.** A
run holding the calendar and the mailbox is the most dangerous context in this
system, and a free-text field is the one place a stranger controls the bytes.
The typed form already does most of the work — nothing anyone types can change
what *kind* of request theirs is, because those are enums the origin
validated — so what is left is prose, and prose is where an instruction hides.
CaMeL's dual-LLM split, at a size where it is cheap.

Four things carry it. `Record::for_privileged_run` is a **function with no
argument that returns the prose**, so the boundary is unreachable rather than
remembered; the extractor's own `reading` stays behind too, because a
paraphrase of an injection is the injection rearranged. `extract` is issued a
request with an **empty tool list and one user message** — not a model told not
to use tools. An extraction failure routes to a human instead of passing the
prose on, which is the one behaviour that would make the layer decorative. And
which fields are prose is decided by the drain from the manifest, never guessed
at on this side by looking for long strings.

**2026-08-07 — a request can reach an answer.** The state machine stopped at
`extracted`. `Record` named the chain `drained → extracted → triaged →
awaiting_me → answered`, but the only states anything ever wrote were the first
two and `extraction_failed` — so a stranger's request could arrive, verify,
queue, drain and pass safely through the quarantine, and then there was nowhere
for it to go. The queue only grew.

`triage`, `needs-info` and `close` finish it, and the join needed no building: a
staged outbox item already records the session that drafted it, so a triage run
with its own session says which drafts belong to which request. `reconcile`
reads the outbox and updates the request store, and runs on its own rather than
on a verb someone has to remember — a state that is only correct after you run a
command is a state nobody can trust. A rejected draft returns the request to
`extracted` rather than closing it, because "not this reply" is not "not this
request"; a partly-resolved set is left alone, because some sent and some
pending is a person mid-review; and `triage` refuses to run without the outbox
route rather than running unrouted, since a stranger's inbox is not where you
want to find out `[outbox] tools` was unset.

**2026-08-07 — a brand, and two documents catching up with the code.** The 9A
mark landed in `brand/` with `scripts/build-brand-assets.py` generating the
three rasters SVG cannot serve, and the Docusaurus scaffold's own artwork was
deleted — shipping another project's mark as this one's is worse than having
none. The site had had no logo at all until then, for exactly that reason.

The public documentation caught up with everything above: three pages that did
not exist (the work directory, publishing, the front door), and three gaps in
pages that did — `web_search` was missing from the built-in tool table,
`Tool::carried_state` was undocumented on both sides of itself, and the MCP
working-directory fix was unrecorded. The CLI and config references were
checked against the binary rather than by reading, which is the only way that
check means anything.

Then the same pass on `CLAUDE.md`, which had drifted further and matters more,
being loaded into every session as project instructions: its architecture map
was missing nine modules, and it had no account of the front door, the search
backends, subagents, or replay.

**2026-08-07 — the box sends the one message it owes a stranger, and stops
trusting the client.** A form rendered, validated, stored a row and told the
visitor to check their email, and then nothing arrived: `Mailer` had one
implementation and it wrote the link to the journal. Amazon SES over HTTPS
with SigV4 signed by hand closed it — not the SDK, because the box picks
`ring` over `aws-lc-rs` for ACME so building needs no cmake, and
`aws-sdk-sesv2` would bring a second HTTP stack. The dependency list's claim
that the box "never initiates" had to be spent to do it, and was rewritten
rather than quietly dropped; the claim that survives is stronger and is the
one that mattered — *the box holds no credential that reaches home*.

Setting the account up produced the more transferable finding. The zone
already published `v=spf1 -all` and `p=reject; adkim=s; aspf=s`, so a naive
SES setup would not have landed in spam — receivers would have **refused the
message outright** while SES reported success. Easy DKIM signing as the domain
itself is what satisfies strict alignment. Delivery to a mailbox at a
strict-DMARC host is therefore the proof it passed, since that host would have
rejected it otherwise. Production access came free, inherited from another
project on the same account.

Then the review gate moved off the client. `Scope::Publish` had authorised all
five write endpoints, so one key could push a version *and* move the alias
that publishes it — and the only thing between an agent and a publication was
mecha's `[outbox] tools`, a list in another repository. Point a different MCP
client at the same server, or typo one entry, and there was no review and
nothing said so. The split the data model already implied: publishing writes
an immutable version nobody can read, releasing is what a reader sees. Now an
agent holds publish-only, and the worst a stolen agent key does is write
versions nobody can see. Deployed and verified against the live box.

**2026-08-07 — a new handle gets a certificate, and nothing restarts.** The
factory's certificate was ordered once at startup for a fixed list, so a user
created while the server ran had no hostname until a restart — the assumption
`SELF-SERVE.md` exists to remove, and the one part of self-serve that was not
ordinary web work. Shipped in the factory repository:
`mecha-factory/src/certificates.rs` plus a rewritten `tls.rs`. Issuance moved
from TLS-ALPN-01 to HTTP-01 (the `pub(crate)` acceptor was only ever needed by
the challenge, not the goal), one `AcmeState` per certificate group sits
behind an SNI-dispatching resolver, and the certificate set is *reconciled
from the ledger* every thirty seconds rather than announced — `factory user
create` runs in another process, so a notification channel would only have
served the signup endpoint that does not exist yet. Port 80 became
load-bearing for issuance, so `[listen] http` is refused-if-absent beside
`[tls]`, and the DEPLOY.md sentences that said otherwise changed with the
code. A high-effort review before deploy confirmed eight defects — nearly all
one shape, the machinery degrading with no log line on the port that had just
become load-bearing. The fixes: seed one ACME account before any order (a
cold cache raced N groups into N registrations, capped at 10 per IP per 3
hours), migrate the old combined certificate into the per-group cache keys
(the upgrade would have re-ordered everything against the 50/week budget with
every name failing handshakes meanwhile), answer no-SNI handshakes with the
base certificate (monitors and curl-by-IP send none), release a dead ACME
task's claim so the reconcile loop can actually repair it, and survive a
transiently locked ledger at startup instead of exiting before the listener
was up. Deployed the same day: the migration made the restart zero-downtime,
and a throwaway `smoketest` user went from `user create` to a served Let's
Encrypt production certificate in about thirty seconds with nothing
restarting. The unclaimed-handle property survived — no resolver, dead
handshake, the 404 still the second line of defence. The plan's own surprise:
the wildcard `A` records the deployment already had meant the Cloudflare zone
move gated nothing after all.

**2026-08-07 — an invited stranger becomes a tenant, no operator anywhere in
the path.** Steps 3 and 4 of `SELF-SERVE.md`, built and deployed the same
afternoon. `factory invite create` mints the right to claim one handle (token
hashed at rest, seven days); `/signup/<token>` on the gate claims it through
the same `create_user_in` the CLI calls, spending the invite in the same
transaction, and the welcome page ends with the exact `factory-publish
connect` command — code included — because the moment a person has just
proved themselves at a browser is the moment to hand them the next step. The
pairing confirmation became the protocol rather than a prompt: `POST
/v1/pair` takes the code *and the asserted handle*, the server redeems only
on a match, and a mismatch spends nothing and answers byte-for-byte what a
nonexistent code answers — no client, human or agent, can wave it through,
which is what the Claude-Code review demanded. Redemption mints the machine's
own publish and drain keys (never release) in the transaction that spends the
code. Proven live within minutes of deploying: a real invite, a claimed
handle, a wrong assertion refused with the code surviving, keys installed at
0600, a push landing on the box, and the unreleased bundle serving to nobody
— which is the scope split working for a stranger's account end to end. The
live proof also caught a real bug: `mirror()`'s "bytes up, not published,
release elsewhere" arm was unreachable (a missing `release.key` hard-errored
first), found by the first machine ever to be in the designed
publish-without-release state, fixed as `Remote::installed()`. The schema
work established two standing rules: **migrations are additive from 3 on**
("delete the database" retired the day the box went live), and **a guarded
ALTER must be idempotent** — a half-run migration that wedges forever on
"duplicate column" turns a one-off transient failure into a bricked ledger.

**2026-08-07 — the second release door, and the operator puts down SSH.**
Steps 5 and 6, same evening. `/account` is the tenant surface: magic-link
sign-in (oracle-free, budgeted per account per day), a `__Host-`-prefixed
session cookie — the prefix is what stops a tenant's page on `alice.art`
tossing a `Domain=` cookie onto the gate, deferring the move-the-gate
question instead of forcing it — CSRF tokens derived from the session on top
of `SameSite=Lax`, release/unrelease driving the same `alias_set` the
release key drives, and machines-connected as the keys ledger with
`last_used_at` stamped on every authenticated call (a key that is used is a
machine that is alive, and a silent compromise shows as life where none was
expected). A session deliberately cannot publish. `factory-publish
disconnect` lets a credential retire itself — `POST /v1/disconnect`
authenticated by the key being revoked, which is what makes a compromised
laptop recoverable by its owner. The operator surface is a fourth scope
rather than a second session system: `Scope::Operate`, bound to the box (no
tenant), minted once over SSH, driving `/v1/admin/*` through
`factory-publish operator …` — users with queue depths, suspend/restore,
invites mailed by the box, every key, break-glass revoke, withhold. The two
surfaces are kept apart by the credential (tenant keys die at the admin
door, the operate key dies at the tenant door, tested both ways), and the
operator verbs are CLI-only, never MCP tools — suspending users is not
power an agent wields as a side effect of conversation. Proven live from
home the same hour: users, keys with last-used, invites, a suspend/restore
round-trip. What stays on SSH, deliberately: deploys, and minting a
replacement operate key if every one is lost.

**2026-08-07, evening — a second client is verified real, and the front door
stops needing anyone to remember it.** The factory's MCP surface was driven
from a Claude Code session over raw stdio JSON-RPC — the initialize
handshake, `tools/list`, and `bundle_render` through `tools/call` under the
`--root` jail — which turned "any MCP client can drive it" from a design
claim into an observation. Two things the drive confirmed structurally: the
seven tools are `bundle_*` only, with no drain tool for a client that owns no
quarantine, and a machine holding all four keys (the operator's own) is the
one place `bundle_alias` over MCP would really release — which is the
SELF-SERVE review's warning made concrete. `docs/SECOND-CLIENT.md` in the
factory repository is the resulting user path; the release workflow
(`release.yml`, static musl with an asserted-static gate) and the crates.io
packaging check landed beside it. On this machine, the front door became
standing machinery: `mecha-frontdoor.timer` runs drain → extract → triage
hourly (`scripts/frontdoor.sh`, ruminate's conventions), verified with a live
tick — drain acknowledged a held record at the gate, and the flagged request
correctly stayed parked for a human. The empty-turn recovery
(`StopCause::NoOutput`, the nudge, three attempts at a measured ~50%
per-attempt recovery) was committed the same evening, with the benchmark
tooling that motivated it.

**2026-08-07 — the first agent scorecard attempt, twice interrupted, once
useful.** The 05:22 launch was voided by the glibc trap (recorded in
BENCHMARK-RESEARCH); the 11:18 relaunch with the portable binary ran ~4 hours
before being stopped to free the box. What the fragment says: 21 trials
completed, **8 solved, 13 failed** — a real 38% at k=1 on a self-selected
early slice, not a scorecard. The `NonZeroAgentExitCodeError`s in it were not
crashes: `mecha run` exits non-zero on exhaustion, so harbor records a
40-turn `MaxTurns` run as an agent error (the verifier still grades the
artifact — one such trial scored 1.0). Relaunch is an open decision.

**2026-08-07/08, night — a stranger's file crosses the whole system, and the
factory grows a face.** The attachment arc shipped in seven staged commits,
each green in isolation: `FieldKind::File` in the manifest (sniffed magic as
the gate, a validation `Phase` so submit and complete are two named moments,
`take_attachments` as the filename quarantine no caller can skip); the box's
blob store where blob lifetime *is* queue-row lifetime, plus the sweep timer
the manual-only sweep needed before attachments made it a disk-fill vector;
the verified-requesters-only upload page (the one multipart route); the
drain protocol's attachment array and blob route; home's
blobs-before-record-before-ack with digest proof; and the frontdoor's
metadata-only brief — a poisoned-values test pins that even a regressed
drain leaks no filename to a privileged run. Verified twice: a 13-step
loopback rehearsal, then live — `letter` v2 with its CV field is deployed.

The same night the public surface got its chrome: header band, splash,
sign-in and account dropdowns, per-version artifact controls (which caught
"Make private" silently also un-aliasing — two decisions wearing one
label), a `/v/` version switcher, and finally the **viewer inversion**: the
signed-in viewer lives on the gate, where the session is, and frames the
bundle cross-origin — owner controls with the account page's own CSRF, a
return address honoured only for `/view/` paths, and bundles granting
exactly one new frame ancestor, the configured gate. The old
`frame-ancestors 'none'` rule's stated fear (a notebook framed by the gate)
was always enforced by origin isolation; what the directive governs is UI
redressing, and the only admitted pages are ones tenants cannot author.
Magic links became scanner-proof the same night, after Microsoft Safe Links
ate the first real sign-in (the trap below). Eight hand deploys; the CI
release tag is the standing fix.

**2026-08-07/08 — the operator's browser, and private pages that open to
named inboxes.** The parallel lane to the scheduling arc, sharing the tree
and the deploy. The **operator admin panel** resolved its queued open
question — the operate key never enters a browser: `factory-publish operator
signin` asks `POST /v1/admin/signin` for a one-time URL, whose scanner-proof
GET/POST interstitial becomes a 12-hour session in its own
`operator_sessions` table, bound to the *key id* under its own
`__Host-factory-operator` cookie. The session dies with the key (the lookup
joins on the key being live and `operate`), neither cookie means anything at
the other surface, and `/admin` renders accounts, invites, keys and
withholds with the same rows the CLI drives — signed out it offers
instructions and nothing to type into. A 25-agent review of the arc found
one theme worth naming: database failures dressed as benign states — a
sign-out that survived a failed revoke, a valid session rendered as
signed-out, empty security ledgers rendered as truth, a one-time link burned
by a failure between redeem and session-mint. All fixed (`86df0a7`), the
last by making redeem-and-mint one transaction — and the same
burn-the-link flaw was then found and fixed on the *tenant* sign-in
(`d5b833d`), so all three sign-ins now share the
spend-only-when-the-session-lands shape. **Private sharing** resolved the
last queued design pass (`0573bb1`): a grant names an *email* — a `shares`
row per (owner, bundle, address), managed from the viewer's Manage menu,
budgeted per owner per day. The box mails the bare viewer URL; the reader
proves the inbox through the magic-link machinery and becomes the third
session surface (`__Host-factory-viewer`, joining on an email — never a
user, never a key). Bytes still never meet identity on artifact origins:
the gate mints a short-lived capability and frames `/g/<cap>/`, whose
lookup re-proves the grant at every fetch — revoking a share kills the
bytes mid-page, and there is a test watching it happen. Readers see the
live version only; an owner's private preview now frames real bytes where
it used to frame the world's 404. Oracle-free throughout: one sign-in page
for every private-or-absent viewer URL, one answer from the form whoever
asks. Deliberately *not* built: folding the three parallel session surfaces
into one parameterised abstraction — the tables are the boundary, and
`Db::signin`'s doc comment says so to whoever next tries to deduplicate it.

**2026-08-08 — the scheduling instrument, designed to deployed in two days.**
The youcanbookme replacement and its when2meet half, end to end:
`calendar_freebusy` on the unified mail surface (fail-closed at every parse —
an unreadable calendar is never a free one); the pure availability engine,
moved into `mecha-manifest` when it turned out to be contract; the slot push
under a new fifth key scope (`slots`, the narrowest, sized for the systemd
timer it lives beside); the booking page at `/s/<handle>/<id>` with its
two-phase atomic claim (soft hold at submit, conversion at the magic-link
click, a lapsed hold *deleting* its queue row so no phantom meeting ever
reaches a calendar); the manage capability with every state answered
honestly; and the group poll — box-minted capability URLs over names only
(addresses never leave home), tri-state answers that work with JavaScript
off, `rank_poll`/`clean_winner` refusing to auto-book past a tie, an
if-needed, or a silent participant. Mid-arc the mail design inverted at the
user's direction: booking mail comes from the user's own account as the
provider's *native* invite (attendee + notifications on), SES narrowed to
account plumbing, and the hand-rolled ICS module, the raw-MIME work and the
deliverability matrix were deleted unbuilt. Deployed the same day and proven
by a live self-test: book → confirm → Outlook event blocking real freebusy →
cancel via the manage link → native withdrawal, with the page, the box and
the calendar agreeing at every step. The deploy also carried the parallel
lane's admin panel and private sharing onto the box. Two timer lessons came
home immediately: drain rides the fifteen-minute sweep (an invite must not
wait on the hourly front door), and the sweep names its account because a
timer cannot ask.

**2026-08-08 — the factory gets a documented face, generated by the code it
documents.** The docs site had been on the factory's own domain while
documenting only the harness; it now opens a Factory section with a live
component gallery. Nothing on it is drawn: `mecha-manifest`'s `gallery`
example walks every field kind, loops over `BUILT_IN` themes, and writes 46
pages — the forms, the rejected view whose eleven messages come out of the
validator rather than a designer, the conditional and multi-step pages, the
upload page, all five starters, and (after the scheduling arc) the week view,
a week on, and a poll. The output is committed at `mecha-factory/gallery/`
and CI regenerates it, refuses a diff, and runs the publish gate over the
whole tree, so a renderer change arrives as a reviewable diff of rendered
HTML — a golden-file test wearing the right clothes. Two guards keep it from
rotting into a smaller manifest format than the one that ships: the generator
matches `FieldKind` exhaustively, so a new variant fails to compile and then
fails the coverage assert until a real field exists; and themes come from the
built-in array, with the docs page building its palette buttons from the
gallery's own `index.json` rather than a list of its own. `mecha`'s side is
`sync-gallery.mjs` on the prebuild hook — sibling checkout first, public
tarball otherwise, warning rather than failing — and a `GalleryFrame`
component that stamps `data-theme` on the framed document, because an iframe
reads `prefers-color-scheme` from the operating system and would otherwise sit
dark inside a light page. Rendering the booking states early paid twice: it
caught that `book.toml` had been rendering through `form()`, a page the box
never serves, and it surfaced two gaps in the not-yet-built POST error path —
the summary lists raw field names where a form's lists labels, and no `_slot`
comes back checked, so a visitor who mistypes an address loses the time they
picked. One decision worth not undoing: **the gallery's clock is a literal.**
Booking was the first surface here with a time in it, and a golden file
rendered against `Utc::now()` differs from itself daily until somebody deletes
the check that keeps saying so.

**2026-08-08, evening — the scheduler grows a front end worth handing out.**
Committed as `1d531a8` in the factory repo and deployed to the box the same
evening. The booking page's clunk was structural, and each piece came out
structurally: the meeting length became a server-side `?mins=` link switch —
week paging's exact shape, so it dedupes with JavaScript off and each start
time renders once as a mono time chip; the details form hides behind a CSS
`:has` reveal until a time is picked (browsers without `:has`, and readers
with CSS off, get the whole page — the reveal has no script behind it), with
a picked-time chip written by `booking.js` as pure restatement. The page is
live now: `GET /s/<handle>/<id>/slots.json` serves the same subtraction the
POST judges (`http/booking.rs:174`, `no-store`), and the script polls it
every 30s and on tab re-focus — a slot someone else holds collapses out of
an open tab, a taken *pick* says so out loud, and fresh slots reload only a
pristine page (anything typed downgrades the reload to an offered link),
which also closed the stale-tab item as a side effect. The POST error path
gaps the gallery had surfaced are fixed at the server (`page_back`,
`http/booking.rs:420`): a rejected submission keeps the typed values, names
failed fields by label, and re-checks the picked `_slot`; the race loser and
the lapsed hold keep the visitor's details too — losing a slot must not
also cost the answers. The poll grid got its polish pass: `poll.js` upgrades
the tri-state radios to tap-to-cycle cells with drag painting (the anchor's
new state decides the whole stroke, when2meet's mechanic), autosaving over
fetch against the same POST (`Accept: application/json` → bare 204/409, the
radios and Save button staying as the JS-off path), and heat rendered
server-side as six discrete `heat-N` classes with "n of m yes" in text —
classes because the gate's CSP forbids inline styles, text because colour
must never carry the information alone. Verified by screenshot across both
themes, both schemes, and mobile, plus a live sweep test against a real
HTTP server; eight new tests took the factory workspace to 325.

**2026-08-08, evening — the admin panel grows an email door.** A parallel
session's arc, committed as `347142b` beside the scheduler pass and
deployed the same evening: with `operator_email` in `factory.toml`, a
signed-out `/admin` offers one button that mails a one-time sign-in link to
the address only configuration knows — admin from any browser, the operate
key never pasted anywhere. The link redeems into the same session the CLI
door mints, anchored to a well-known `email-door` key row that can never
authenticate a bearer, which keeps "an operator session resolves to a key"
true and makes revoking that row the door's kill switch. Links are budgeted
per day; sessions from both doors last 30 days and roll on use. The detail
in that repo's `DEPLOY.md`.

**2026-08-08, late evening — a release becomes a tag, and the deploy becomes
a command.** The email door's deploy was the last hand-deploy, and it earned
the pipeline that ended the era: the locally built binary crash-looped the
service (`status=203/EXEC` — the workstation is aarch64, the droplet x86_64),
and the config line appended over SSH landed inside `[mail]`, where TOML
swallowed it silently. Both repos now release by tag: a `v*` tag re-runs the
suite, refuses a tag that disagrees with the workspace version, and publishes
the crates in dependency order through crates.io Trusted Publishing — all six
live at 0.1.0 (`mecha` itself is taken; the CLI ships as `mecha-cli`). The
factory tag also builds a static musl binary whose workflow *refuses to ship
it dynamically linked* — a guard that paid for itself on its first verified
run, catching a crt-static toolchain-default drift that would have replayed
the morning's outage; `RUSTFLAGS` now states the contract instead of
inheriting it. `scripts/deploy.sh`, installed on the box as `factory-deploy`,
makes a deploy one command — download by tag, checksum, prove the binary and
`factory check` the config while the site is still up, swap keeping
`factory.prev`, health-check, roll back unaided on failure — and `v0.1.0`
went out through it, verified live by the queue holding an empty `?wait=4`
long-poll the full four seconds. The workflow the day taught is
`CONTRIBUTING.md`'s in both repos now: branch per arc, one worktree per
session (two sessions shared a tree that afternoon, and production code
spent an hour existing only in a stash), PR-gated landings, rebase merges,
and no attribution trailers.

**2026-08-08, night — review moves into the TUI, and a run's drafts meet
you at the door.** `/outbox` and `/frontdoor` landed as modals on the
`/triggers` pattern — the store read directly for display, every mutation a
`mecha …` child process, long work (a send's MCP startup, an extraction, a
triage run) spawned detached with the store as the record. The decisions
that differ from `/triggers`: **every send confirms**, because it is the
one keystroke that cannot be taken back, and a tainted draft confirms in
red with its full arguments on screen; a publish's `edit` is refused with
the real action named, as on the CLI; the frontdoor detail prints the
stranger's prose under its framing, because a person reading a terminal is
the safe context; `close` refuses an empty reason. On top of that, the
status line grew an ` outbox N ` badge, and `/review now|later|auto` now
decides what happens when a run *this session started* finishes having
staged drafts: open a modal scoped to exactly those items, just notice, or
release them — where the mode is set only by slash command (a
model-interpreted "preapproved" is a directive an injected page could also
write), scope is an id-diff between submit and completion so the overnight
backlog is never touched, tainted drafts never auto-release (the approval
predates whatever armed the taint), and an errored or early-stopped run
releases nothing. Detached work reports back instead of asking for a
reopen: a watch list polls the *stores* (never the child — the store is the
record) on a one-second tick while anything is live, and a landed release
or a moved request becomes a transcript notice plus a badge and modal
refresh; a watch that outlives its cap is dropped with a still-working
notice so a wedged child cannot pin the fast tick. And staged summaries
learned to lead with who and what: `summarize` in core now prefers the
conventional argument names (`to`, `subject`, `title`) over raw compact
JSON — keyed on argument names, never on the tool, so the store stays
tool-agnostic. Along the way a latent bug: the TUI's trigger edit ran
`$EDITOR` through `.output()` — a pipe for a screen, a closed stdin for a
keyboard — found only when a second editor shell-out forced the question;
the suspend dance is now one helper and interactive children inherit the
real terminal, with only stderr captured. `mecha-cli` went from 83 to 100
tests, including the tainted confirmation rendered through the real draw
path.

**2026-08-09 — agents can leave each other messages, and taint rides
along.** Researched (Claude Code's cross-session messaging over per-session
sockets; A2A/ACP/MCP-Tasks; Morris-II and Prompt Infection on cross-agent
injection — `docs/MESSAGING-RESEARCH.md` holds the survey, the design, and
six decided questions) and then built the same day: `mecha-core/src/mailbox.rs`
is a file-based mailbox under `~/.mecha/messages/<recipient>/` on the
outbox's store conventions, addressed by producer name, claimed under a
per-recipient flock at the top of a turn and folded in at the steering fold
point. The piece no deployed system had: the harness stamps the sender's
conversation taint on every message (the conservative per-turn snapshot,
via `ToolCtx::taint`, `Option` so unstamped fails closed to fully tainted)
and delivery merges it into the receiving conversation before the body
lands — a hop between agents launders nothing. Provenance header says it is
another agent, not the user; untrusted senders get the standard wrapper;
unknown taint reads as armed. `[messages]` in config is global-only (the
section is stripped from a project `mecha.toml`, loudly); attended surfaces
default to `hold` with a waiting-mail notice, unattended runs to `accept`;
eval forces `--no-messages`. Full mailboxes refuse rather than drop-oldest
— against Claude Code's choice, because the sender is an agent that can be
told "full" — with `mecha msg dismiss` as the human's way to clear a
backlog no run will claim; identical pending sends deduplicate as the loop
brake; malformed files quarantine as `.bad` instead of wedging the mailbox
(Claude Code shipped that bug). `mecha msg send/list/show/dismiss/agents`
is the CLI, with `agents` reading a per-session liveness registry that
generalises the trigger `RunMarker`. Phase 2 (TUI badge and `/messages`
modal, live delivery into an in-flight run) is scoped in the research doc,
whose §5 also records what the machinery unlocks beyond messaging: inbound
webhooks and file watchers were both blocked on exactly this labeled
taint-carrying prompt path, and headless steering of a trigger run is now
`mecha msg send` away.
**2026-08-09 — the benchmark run is read, and the loop stops dying of
recoverable things.** The salvaged 21-trial fragment of the 2026-08-07
Terminal-Bench run was diagnosed trial by trial — 8 passes, 5 genuine model
failures, and 8 deaths the harness owned some share of — and the diagnosis
answered a standing four-way question: the trifecta interlock costs the
benchmark *nothing* (no trial ever armed the untrusted leg — the surface has
no web tools and `shell` is not an untrusted source); the 32k context is
genuinely overloaded, with the flat 24 KB per-turn output budget as the named
mechanism (8–12k tokens of numeric data, larger than the
threshold-to-window gap, so `path-tracing` leapt from under the threshold to
45k tokens in one turn and died); the loop had real structural gaps; and
compaction itself was mostly exonerated — the two heaviest trials compacted
repeatedly and both passed. Five fixes landed on one branch (PR #21), each
with a regression test that fails on the old behaviour: overflow recovery no
longer disables itself when a summary was not worthwhile (the give-up flag
gated the whole arm, so the *next* overflow died raw); the empty-turn
allowance resets on productive turns instead of accumulating (two trials died
`NoOutput` mid-task with retries spent hours earlier, while two others
recovered from a nudge and passed — and the empties persisted with
`--reasoning-budget 4096` active, so the server-side fix reduced the problem
rather than ending it); `mecha run` exits non-zero only for produced-nothing
runs (every exhausted stop exited 3, which Harbor records as an agent crash —
`headless-terminal` hit MaxTurns, was counted an error, and its verifier
scored the work 1.0); transcripts survive crashes and record rewrites (a
`rewrite` session record carries the compacted state, `load` replaces, the
taint timeline clamps stale positions toward over-taint); and the tool-output
budget derives from the context window when unpinned — 12,288 bytes at 32k,
the old 24,000 at wide windows. The bench adapter now captures stderr and
`MECHA_LOG=debug` beside the transcript. The full write-up is
`docs/BENCHMARK-RESEARCH.md`, "The 2026-08-07 subset run, diagnosed".

**2026-08-09 — mecha grows a remote control, and it is a Slack thread.**
Researched, designed and built in one session, then run against a real
workspace: `docs/SLACK-RESEARCH.md` is the evidence and `docs/SLACK-DESIGN.md`
the decisions. **Socket Mode**, so home dials out — no inbound port, no
certificate, no tunnel, and no request signature to verify, which is the same
argument `mecha-drain.service` already made about the factory queue. A fourth
crate, `mecha-slack`, holds the transport and **cannot depend on
`mecha-core`**, so the invariant is checkable by reading a manifest rather
than by reviewing diffs; the front-end that knows both sides lives in
`mecha-cli/src/slack/`, beside `tui/`. **Two trust tiers and no third** — an
allowlist of Slack user ids bound by a nonce the local CLI prints, which
proves shell access to the machine where an email address proves only what
the workspace claims; everyone else is ignored. A thread is a `Conversation`,
so the interlock gets the right granularity for free; a thread's jail is a
subdirectory of one `slack` work producer, so retention reaches it. The
answer streams with a `task_update` card per tool call, keyed on the
`tool_use` id so a call's lifecycle is one card changing state. Approvals are
durable cards rewritten into terminal records, with **"Allow for this run"**
after the first real task raised seven of them. Drafts a run stages come back
as review cards with Send and Reject, scoped by an id-diff of pending outbox
ids — **which closes `PUBLIC-SURFACE-DESIGN.md` §11's deferred "phone UI for
releasing outbox drafts", and it needed no home-side server at all.** Files
go both ways: an attachment lands in the jail and is named to the model as a
path (so taint arms through `fs_read`, the route that already exists), and
what a run creates is uploaded back. `mecha slack notify` puts a trigger's
briefing on a phone for the price of a config line, and
`scripts/mecha-slack.service` is the third always-on unit. One flock means one
connector. Verified live on `cosanlab`: binding, streaming, approvals, mode,
orphan recovery, attachments in, artifacts out.

Two things were left undone deliberately and are named in the handoff:
`ask_user`, because it is a tool and the registry belongs to the agent that
serves every thread; and per-thread isolation for MCP tools, because servers
are spawned once with the agent. Both want an agent per thread, and an MCP
startup per thread with it.

The arc also changed `mecha-core`. `Decision` gained a third variant,
`Blocked(String)`, rendered `"Blocked by policy:"` — see the trap below —
and `process_alive` moved to `mecha_core` when a second and third subsystem
turned out to want the pid range check that is its whole correctness.

---

## The measurement record

Moved out of `HANDOFF.md` on 2026-08-06, when that file went over its own
length bound: this is a record of what was measured, which is what this
document is for.

On the original 25 grounded cases, all four local models score 23–24/25 with
zero malformed arguments and zero invented tools. **That set saturated** — it is
a floor test, not a ranking test, and it stays in the file as exactly that.

Two conclusions hold from it anyway:

1. **MoE wins on this hardware.** Decode tracks *active* parameters. The dense
   27B is 8× slower than the 3B-active MoE for identical accuracy.
2. **Constrained decoding is doing real work.** `llama-server --jinja`
   grammar-constrains tool calls; that is why malformed-argument counts are zero
   across the board. Don't conclude anything about a model's tool reliability
   from an unconstrained sampler.

The cases added since (`long-horizon`, `codegen`, `synthesis`, `ambiguity`) do
discriminate. qwen3.6-35b-a3b judged by gemma-4-26b-a4b scored **32/34** on the
set as it stood then (`results/qwen-hard-v2.json`):

- **long-horizon 2/2**, at ~17.5 turns — it walks a 16-link chain without losing
  the running total, and does not take the shortcut of summing the decoys.
  Confirmed at n=5 on 2026-08-03: `chain-total` is **5/5** uncompacted,
  `chain-largest` **4/5**. A single earlier failure looked like a regression and
  was variance — which is the whole argument for pass^k.
- **codegen 2/2** — implements `median`, finds the one-line duration-parsing
  bug, and runs the tests itself. Graded by running them, not by asking.
- **synthesis 2/2** — finds the majority figure and the outlier, and notices
  which report supersedes which.
- **ambiguity 8/9 across the tag**, once `ask_user` existed *and* the cases
  graded the trace rather than the answer. `ambiguous-rate` asserts
  `tools: ["ask_user"]`; `false-premise` asserts `forbid_tools: ["ask_user"]`,
  because the right move there is *not* to ask — the file simply does not
  exist. How that was arrived at, and why a clean A/B said the tool made no
  difference while the transcripts said otherwise, is in
  [`HISTORY.md`](HISTORY.md) under Traps → Measuring. Read the transcripts
  before believing a score.

Only `ambiguity` and `synthesis` have a judge in the loop, and judges disagree
with themselves across runs. Read the answer before believing a single verdict.

**Scorecards in `results/` taken before the fixture expansion are not comparable
to ones after it.** The new fixtures took the shared workspace from 11 files to
44, so every case that searches the whole workspace got harder — two of them
started failing on turn ceilings calibrated against the smaller tree. If you add
fixtures, expect to recalibrate, and re-baseline every model rather than
comparing across the boundary.

The compaction arc — seven measured arms, and the finding that a summariser
preserves *what is true* while dropping *how far you got* — is in
[`HISTORY.md`](HISTORY.md). It is the reason compaction is shaped the way it is,
and it is worth reading before changing that code.

---

## The compaction measurement record

Kept because it is the only place in the repository where a design decision is
backed by an arm-by-arm measurement rather than by argument, and because the
numbers are what stop the next person re-trying the two treatments that did not
work. Copied essentially verbatim from the version of `HANDOFF.md` that
preceded this split.

An earlier claim in that file — that compaction "compacted four times and still
answered 16 entries / 847" on the audit chain — was **retracted**: it was one
sample and did not hold up under repetition. What follows is what replaced it.

**Measured, and it is worse than the file used to claim.** Two cases, same
model, same threshold, on 2026-08-03:

| Case | Result |
|---|---|
| `compaction-carries-the-task` — recall a token stated in turn 1 after 8 filler turns | **3/3** |
| `chain-total-compacted` — the 16-link traversal, `compact_at_tokens: 1200` | **1/5** |
| `chain-total` — the identical task, uncompacted | **5/5** |

5/5 against 1/5 on the same task with one variable changed (Fisher's exact
p≈0.05).

The failure mode names the cause. The two logged walks lost their *place*, not
their facts: one invented `next: END` five links early, the other read 14 links
correctly, re-read an entry it had already seen, and restarted from `START.md`.
Meanwhile a stated fact survives compaction 3/3.

So the summariser preserves **what is true** and drops **how far you got**. Read
`SUMMARY_INSTRUCTION` (`mecha-core/src/compact.rs`) with that in mind: it asks
for established facts with their values, for what failed so it is not repeated,
and for what remained — but never for position in a sequence, and "which entries
I have already visited" is neither a fact about the world nor a failed attempt.

Two things were tried. Measured on qwen3.6-35b-a3b at `compact_at_tokens: 1200`:

| arm | `chain-total-compacted` | `carries-the-task` |
|---|---|---|
| original summariser | 1/3 | 3/3 |
| + a clause asking for traversal position | 2/5 | 5/5 |
| + tiered thinning | **4/5** | 5/5 |
| + todo instruction, prompt only (2026-08-04) | 4/5 | 5/5 |
| + todo instruction, prompt + tool description (2026-08-04) | 4/5 | 5/5 |
| 4-slot server era, either validation arm (confounded — see below) | 2/5 | 5/5 |
| + eviction + validation + own-budget summariser, `-np 1` (2026-08-05) | **5/5** | 5/5 |
| uncompacted control | 5/5 | — |

The 2026-08-05 arm (`results/compaction-k5-np1.json`) is the first in which the
compacted case matches its uncompacted control. It bundles four changes
(eviction, summary validation, the summariser's own budget, spill-capped
results) plus the server fix, so it does not isolate any one of them — but the
2/5 rows above it were the same code measured against the quartered 8192-token
server, which is what "a stale `context_window` is worse than none" looks like
when the *server* moves the window. (The llama-server build in use defaulted to
four parallel slots and split `-c` across them; past the real limit it
context-shifts rather than erroring, so the model saw a mangled transcript and
returned empty completions. Check `curl :8080/props | jq .total_slots` is 1
before believing any measurement.)

The two todo arms are not really separate treatments: the model never called
`todo` inside the eval in either one, so both are further samples of the
thinning arm — which pools to **12/15**, and every failure in the pool is a
wrong *total* over a correctly-completed walk.

**The prompt clause did nothing** (1/3 → 2/5 is noise). **Thinning appears to
close most of the gap**, but be careful with that number: 4/5 against the pooled
3/8 of both earlier arms is p≈0.27, which is not significance at n=5. What makes
it more believable than the clause is not the p-value but the mechanism — the
claim is "the sequence of tool calls survives", and that is a unit test rather
than a hope about what a summariser noticed. Run n≈15 per arm if the number
needs to be citable.

The design is in `thin_old_results` (`mecha-core/src/compact.rs`): a call and
its result differ enormously in size *and* value, so shorten the results and
keep the calls. Position stops being something a summary has to preserve and
becomes something the transcript structurally still contains.

**The todo-list instruction was measured on 2026-08-04 and the answer was no.**
qwen3.6-35b-a3b called `todo` **zero times in 20 eval case-runs** whether the
directive sat in the system prompt, the tool description, or both, and
`chain-total-compacted` stayed 4/5 in every arm. Three probes localised why: the
model keeps a list flawlessly when the *user turn* asks for one and ignores the
identical directive in the system prompt (delivery was verified in the recorded
`RunConfig`, so this is an instruction-following gap, not a wiring bug); moving
it into the tool description got adoption once, as a single static item that
never updated — a checkmark, not a position ledger; and across all 15 compacted
chain runs taken 2026-08-03/04, **no failure was a position failure**. Thinning
had already fixed the mode todo was meant to fix. The residual failure is value
accumulation — wrong totals over correct walks — which a running total kept in
the list would address and which this model will not maintain from prompting
alone. Both changes were kept, since a stronger model may follow them and they
cost nothing; note that the `todo` description change alters the tool surface of
every eval case, so re-baseline before comparing scorecards across that
boundary. If it is ever revisited, the machinery worth considering is
re-injecting the list at compaction time, not more prompting.

---

## Traps already hit

Recorded so they are not hit twice. Each says what broke; the sentence that
matters is the general shape.

### Measuring

- **A wrong gold answer measures nothing.** One was shipped ($2,450 vs the
  correct $1,750) by double-counting a base rate. Verify arithmetic with a
  script — `scripts/build-eval-fixtures.py` now computes them.
- **A case with more than one right answer has none.** `pick-search` asked
  "which file mentions Nadia" when three do, and asserted one of them. It only
  surfaced when a model named the other two. Grep the fixture before writing
  the gold.
- **A grading ceiling can measure the ceiling.** Two ambiguity cases had turn
  budgets tight enough that the model got cut off mid-exploration, so the case
  graded budget exhaustion rather than judgement. Discovering that a request is
  under-specified takes reading; leave room for it.
- **Substring grading measures formatting.** `"$2,520"` failed a check for
  `2520`; `"do **not** agree"` failed `not agree`. Both answers were right. The
  `normalize` helper in `mecha-core/src/eval.rs` handles it — extend that, don't
  work around it.
- **…and again, unboundedly.** "There is no `budget.csv`" matched none of ten
  hand-listed negation phrasings. The negation phrasing space has no bottom —
  that case is judge-only now. Reach for `expect.judge` when you catch yourself
  enumerating synonyms.
- **The transcript you are reading may not be the run that happened.** A
  28-turn benchmark trial's session file held 8 assistant messages starting
  mid-conversation — recording sliced "what the run added" off a list
  compaction had rewritten in place, so the rebuilt head (and the summary in
  it) never landed, and crashed runs recorded nothing because messages were
  written only after a successful return. Half a day of the diagnosis went to
  reconstructing what the recorder had dropped. A recorder that assumes an
  invariant (append-only) the recorded system deliberately breaks (compaction)
  is silently wrong exactly when the record matters; reconcile against what
  was actually recorded, don't index into what you assume was.
- **Check where every output stream lands before a long run.** Harbor captured
  `stderr: None` on all 21 trials, and stderr is where mecha's compaction
  notices and tracing go — the one channel that would have said which trials
  compacted, recovered, or gave up. The evidence for a day of forensics was
  discarded by a default nobody had looked at. Before any multi-hour run,
  confirm each stream's destination the way `-np 1` gets confirmed: by
  checking, not assuming.
- **A judge needs room to think before it answers.** At `max_tokens: 512` the
  judge spent the entire budget on reasoning and returned empty content with
  `finish_reason: length`. It is 4096 now, and an unparseable verdict reports
  the stop reason rather than just the empty string.
- **A shared stylesheet is verified against every surface it styles.** The
  booking page's picked-slot fill was keyed on `.slot:has(input:checked)` —
  and the poll page's cells are also `.slot`s, each always holding a checked
  tri-state radio, so every cell flooded solid accent. Unit tests on both
  pages stayed green; a screenshot of the *other* page caught it in one
  glance. State selectors on a shared sheet name the control
  (`input[name="_slot"]`), never the element — and restyling one surface
  means rendering them all.
- **A test can pass for a reason you did not write.** The first version of the
  broken-sandbox test named `image:` before `..cfg` in a helper, so Rust's
  struct-update ordering silently overrode the caller's deliberately broken
  image and the test ran against a working one. It failed only because it
  *passed* when it should not have. Put the forced fields where they cannot
  shadow the caller's, and comment why.
- **A negative assertion needs a machine where the positive holds.** The
  confinement tests assert no network and no `~/.ssh`; both would pass
  vacuously on a host that had neither. Check the host has them before
  believing the sandbox took them away.
- **An exit code is an interface, and exhaustion speaks it badly.** `mecha
  run` exits non-zero when a run stops on `MaxTurns`, so harbor recorded every
  turn-limit exhaustion as `NonZeroAgentExitCodeError` — 7 of the 21 salvaged
  benchmark trials read as agent crashes when the agent had run 40 full turns
  and given a partial answer (one of them *scored 1.0*, because the verifier
  grades the artifact). Any harness that keys on your exit code will read
  "gave up" as "broke". Decide which meanings your exit codes carry before a
  third party starts parsing them.
- **Read the transcripts before believing the score.** The clean A/B on
  `ask_user` said it made no difference: 6/9 either way. The transcripts said
  otherwise, and they were right — without the tool the model burned **30 tool
  calls** and died on the turn ceiling *with a correct answer*; with it, it
  asked in **3** and failed a rubric demanding it ask for two missing things at
  once. Identical scores, opposite reasons, and a large real improvement
  invisible to the grader. Rewriting the cases to assert on the trace
  (`tools: ["ask_user"]`, and `forbid_tools: ["ask_user"]` where asking is the
  *wrong* move) took the tag from 6/9 to 8/9.
- **A tool's failure text is part of its behaviour.** `ask_user`'s decline
  originally said "proceed with your best interpretation", and the model duly
  invented a contractor name and rate — precisely the failure the case exists to
  catch. A decline must not read as permission to guess.
- **Changing the tool surface invalidates the comparison, including
  accidentally.** Adding pkg to `~/.mecha/config.toml` gave every eval case five
  extra tools in the middle of an A/B. Same trap as the fixture change, entered
  from a different door.
- **An override that widens should not narrow something unrelated.** Forcing
  `untrusted_input` on an MCP server also revoked `read_only`, on the reasoning
  that a distrusted tool should not skip the approval gate. But distrusting what
  a tool *returns* says nothing about whether it *writes* — and the result was
  every memory retrieval demanding approval. Only a forced `destructive`
  contradicts read-only.
- **Grade the control before believing the treatment.** `chain-total-compacted`
  failed on its first run, which looked like a compaction finding — until the
  uncompacted control failed too, which made it look like nothing. Both readings
  were n=1. Five runs of each turned it back into a finding, and a real one.
  A case that isolates one variable proves nothing while its control is
  unmeasured, in *either* direction.
- **Do not couple a diagnostic to your hardest case.** The first compaction
  case was the 16-link traversal, so a compaction failure and a long-horizon
  failure were indistinguishable in the result. The replacement states a token
  in turn one and asks for it after eight filler turns: the underlying task is
  trivial, so the case can only fail for the reason it is named after.
- **Assert on the trace, not only on the answer.** `chain-total` was graded on
  its total, so a model that stopped early and summed its own truncated list
  *correctly* failed with "847 not in the answer" — true, useless. Asserting it
  read `entry-d084.md`, the one real terminator, names the failure instead. It
  also closed a hole in `chain-largest`, whose answer sits at link 9 of 16 and
  could be reached without ever finishing the walk.

### Learning

All found by pre-push review or by running it.

- **The "edit-distance gate" was never code, and the handoff carried it as an
  open item for weeks.** It was described as observed working live; a
  verification sweep on 2026-08-09 found no threshold, no `levenshtein`, and
  nowhere one could have been removed from — the behaviour was always the
  reflector model declining to mine a trivial edit. Closed as obsolete rather
  than deleted, because the next person would otherwise re-propose it.
  **An item whose evidence is "I saw it work" and not `file:line` is a
  hypothesis**, and it should be written as one.

- **A refusal nobody made must not be labelled as one.** `agent.rs` prefixes
  whatever reason an approver returns with `"Denied by the user: "`, which is
  the exact string the miner keys on — so a remote approval nobody answered,
  and every read-only run's mode refusal, arrived as corrections from a person
  who never spoke. The fix had to be a third `Decision` variant in the core,
  not a wording choice in the front-end. **When a string carries meaning
  downstream, the type has to carry it instead** — and check who else is
  already producing that string before adding a producer.

- **A timeout that starts after the blocking part is not a timeout.** The hook
  runner wrote the JSON payload to the child's stdin *before* entering the timed
  wait — so a hook that never read stdin blocked `write_all` forever once the
  payload outgrew the pipe buffer, and the run hung with the timeout never
  started. Wrap the write and the wait in one timed future. The general shape:
  audit what sits *outside* every timeout.
- **A "did it repeat the call" scan must start at the decision point.** The
  denial verdict scanned the whole replayed trajectory, and the faithful prefix
  legitimately contains whatever the recording contains — including, sometimes,
  an earlier instance of the very call later denied.
- **An unattended generator with a rejection path is a loop.** Gate-rejected
  reflections return to the pool by design, so an unchanged pool re-argued the
  same batch every night. Deduplicate on the exact batch, not on time.
- **Interactive-mode manners become data loss unattended.** Reflect printed a
  provider error and marked the session mined anyway; fine with a human
  watching, silent permanent loss from a hook. Mining is all-or-nothing per
  session now.

### Providers

- **A guessed wire format has tests that agree with it.** Slack's
  `task_update` chunk was implemented from a plausible shape — nested under a
  `task_update` key, no id — and every unit test asserted that shape and
  passed. Live, Slack answered `invalid_arguments`, and said nothing at all
  about the missing `id` that keys a task card, so without it a call's
  lifecycle would have rendered as three unrelated lines. Fixing it exposed a
  second: a stream has one mode, and mixing a `chunks` array with the
  top-level `markdown_text` argument is `streaming_mode_mismatch` — invisible
  until the first bug stopped hiding it. **Tests written from a belief about a
  third party confirm the belief, not the behaviour.** Read the reference for
  the exact field names, assert the documented shape *and* the negative, and
  treat the first live call as the real test. This is the `ScriptedProvider`
  blindness `CLAUDE.md` names, in a new costume.

- The unified `calendar_create_event` schema said "attendees receive
  invitations", and on Google nobody ever had: `events.insert` defaults to
  `sendUpdates=none`, while Graph mails attendees unconditionally. A unified
  surface's documented behaviour is a *claim about every backend* — enforce
  it per backend, or the schema lies for some providers and the lie is
  invisible until someone waits for a mail that never came.


- **Never believe `finish_reason`.** llama-server reports `stop` alongside
  `tool_calls`. The loop believed it, dropped the calls, ended the run and
  returned an empty string — which graded as a model failure and was a harness
  failure. Any turn containing tool_use blocks is now a tool turn regardless.
  Assume the same class of bug exists for other local servers.

### Containment and state

- **Per-run jails and shared subprocesses do not mix, and the model finds out
  first.** MCP servers are spawned once with the agent, so a Slack connector
  giving each thread its own `RunContext` workspace left the servers rooted
  wherever it was launched: `bundle_render` resolved against the repo while
  `fs_write` wrote into the thread's jail. The model reported "the workspace
  and render tool have different root paths" and burned five turns working
  around it with `shell`. **Anything spawned once cannot follow a per-run
  value** — either root it somewhere both agree on, or accept that the
  isolation only covers what the loop itself resolves. Say which, in writing.

- **`Path::starts_with` is lexical, so it is not a containment check.**
  `<staging>/../escape.html` starts with `<staging>` by that test and lands one
  directory up. The factory's bundle installer used it to prove a path stayed
  inside a staging directory; a test caught it. Build the path from components
  and refuse anything that is not a normal one — a check that reasons about
  strings is not a check about where a write lands.
- **`last_insert_rowid()` is per-connection and per-*any*-table.** Using it to
  select back a row you just `UPDATE`d works right up until something else
  inserts anything on that connection — in this case minting a key between a
  form submission and its confirmation click, which made verification silently
  fail. If you need the row you changed, read it and change it by its key,
  inside one transaction. The general shape: a value that is *usually* the one
  you meant is worse than one that never is, because it passes the test you
  wrote first.
- **A privileged dry-run can create state with the wrong owner.** Running
  `factory check` as root before the service had ever started created the
  SQLite ledger owned by root, after which the service could read it and not
  write it — and the failure surfaced two steps later as "attempt to write a
  readonly database" from an unrelated command. Anything that lazily creates
  state should be run as whoever will own it, which is why the systemd unit
  runs its pre-flight as the service user.

### Unattended runs

- **An edit that silently matches nothing ships a false claim.** The fix for
  the startup banner below was written three times before it existed: rustfmt
  had wrapped the `tracing::info!` across lines, a single-line string replace
  found nothing, the build passed, the tests passed, and it was reported as
  done. Nobody noticed until the unit was installed and the journal showed a
  started service with no evidence it was working. **A build that succeeds is
  not evidence an edit applied** — assert on the match, or grep for the new
  text afterwards. The same silent no-op hit twice more the same day.

- **A daemon that prints nothing at startup is indistinguishable from a wedged
  one.** The Slack connector logged "connector up" through `tracing`, which is
  invisible without `MECHA_LOG`, so the first run of it looked like a hang.
  The same session had spent a day citing that exact confusion in other
  people's software. **Anything that waits should say so on the way in, and on
  stdout rather than through a log filter** — and a refusal it makes should be
  visible too, because silence is what makes a working gate look broken.

- `mecha-mail freebusy --days 60 | slots push` could *never* satisfy a 60-day
  horizon: the freebusy window was stamped from one process's clock and the
  horizon deadline computed from another's, a second later. Anchor every
  derived deadline to the data's own stamp, never the reader's clock — which
  also makes the pipeline deterministic for free. Found by running the
  documented pipeline, not by reading it.
- The bookings sweep's "no new bookings" early-return silently skipped the
  cancellation and reminder passes on every quiet tick — the common case.
  When a verb grows a second duty, audit every `return` that predates it.


- **A systemd unit gives its children a minimal environment, and that is where
  `notify` runs.** The first real scheduled trigger run produced its briefing
  and then exited **127**: the unit named `%h/.cargo/bin/mecha` in `ExecStart`
  so the daemon started, but `factory-publish` was not on the child's `PATH`.
  Works by hand, fails under the scheduler — the shape of bug this project keeps
  finding. The unit now sets `PATH`; more importantly, **the failure goes into
  the ledger**, because the run itself is `ok` either way and a briefing that
  has quietly not rendered for a week has to look different from one that works.
  Same argument as `stop_cause`, and it applies to any observer whose failure
  is invisible: report it where the thing it failed at is recorded, not only on
  stderr that nobody reads for an unattended process.
- **Installing a thing is how you find out about it.** Everything above was
  built, tested and documented for two days before anybody ran it on a
  schedule, and one minute of real scheduling produced a bug no test had.
- **A new global precondition breaks the callers you did not think of, and a
  lazy caller breaks selectively.** Refusing a workspace that contains the
  mecha home fixed a real jail bug and silently broke the nightly rumination:
  `validate` and `learn --propose` both build a tool surface and so both hit
  the new check, and a systemd *user* unit with no `WorkingDirectory` runs in
  `$HOME`, which contains `~/.mecha`. The part worth the entry is that they
  build that surface **lazily** — only when there is a proposal to gate or a
  probe to replay — so quiet nights passed and the nights with work aborted.
  A precondition added centrally needs its callers enumerated, and a caller
  that only sometimes trips it is worse than one that always does: the failure
  correlates with the work being worth doing, which is exactly when nobody is
  watching. Found by an automated review, not by running it.
- **Check-then-act across a human is a race, not a formality.** `outbox send`
  holds the store lock across execution so two sends cannot both pass the
  pending check. `outbox review` checked pending, printed the draft, and then
  *waited at a prompt* — the check and the act separated by however long a
  person takes to read. Anywhere a human sits between the test and the action,
  the test has to be repeated on the far side of them.

### Environment

- A `grep -o 'mk_slt_[A-Za-z0-9_-]*'` clipped a freshly minted key to 24 of
  its 88 bytes, and the truncated credential *looked* installed. The tell was
  indirect: `/v1/health` answers anonymous callers too, so the broken key
  produced a valid-looking 200 with the authenticated fields silently
  absent. Prove a just-installed credential against an endpoint that *only*
  answers authenticated callers — an endpoint serving both cannot fail
  loudly.


- **A magic link spent on GET is spent by the mail scanner, not the
  person.** The first real sign-in from a university mailbox arrived already
  dead: Microsoft Safe Links fetches a mail's URLs on delivery, and both the
  sign-in and form-verification links redeemed on GET — so the robot's fetch
  consumed them seconds before the human's click, and worse, robots were
  verifying submissions no human had confirmed. The fix is the shape every
  magic-link system eventually grows: GET renders one button and touches no
  token at all (not even a peek — a page that varied on token state would
  hand the scanner an oracle), and only the POST spends. **Any single-use
  URL that will travel through institutional email must not have side
  effects on GET**, and the test to write is that the interstitial answers
  byte-identically for a live, spent, and invented token alike.
- **A Docusaurus admonition with a title renders as plain text if you use the
  old syntax, and never says so.** Docusaurus 3 takes the title in brackets
  (`:::note[Title]`); the v2 spelling `:::note Title` does not error, it falls
  through the directive parser and ships as an ordinary paragraph with the
  literal colons in it. Two pages had been live and wrong for weeks, and the
  reason nobody caught it is that the untitled form — which is most of them —
  still works, so the site looked fine everywhere anyone had looked.
  **A markup extension that degrades to visible plain text is worse than one
  that fails the build**, because the failure mode is a page that renders,
  deploys, and reads as intentional. When a build has an `onBrokenLinks: throw`
  it is worth asking what *else* is silently permitted; grep the whole tree for
  the deprecated spelling rather than fixing the one page you were looking at.
- **A tmux pane is a systemd scope, and an OOM inside it kills everything in
  it.** A runaway test allocated 80 GB and was OOM-killed four times in half an
  hour; ninety seconds after each kill, systemd tore down the whole
  `tmux-spawn-*.scope` (`OOMPolicy=stop`), taking `bash`, `claude`, `pkg-mcp`
  and eventually a two-day-old `llama-server` with it. The sessions looked like
  they were crashing on their own. **When a process dies for no local reason,
  check whether something else in its cgroup died first** — and start anything
  you want to survive as its own transient unit (`systemd-run --user
  --unit=…`), not from the pane.
- **`ulimit -v` is the wrong cap for a tokio program.** It bounds *virtual*
  address space, and thread stacks are reserved there, so a memory cap that
  looks generous makes `thread::spawn` fail in a way that reads as a real test
  failure. `systemd-run --user --scope -p MemoryMax=…` is RSS-based and
  behaves. The general shape: **a limit that measures something other than
  what you meant produces failures in an unrelated subsystem.**
- **`-np 1` is load-bearing.** The llama-server build in use defaults to **4
  parallel slots**, which silently splits `-c` across them: for a period on
  2026-08-04 every request ran against **8192 tokens of context, not 32768**,
  while mecha's `context_window` said otherwise. Past 8192 the server
  context-shifts instead of erroring, so the model saw a mangled transcript and
  returned *empty completions* — the mysterious empty-EndTurn deaths in the k=5
  compaction runs were this, not a mecha regression, and every scorecard taken
  between the two restarts is confounded. Check
  `curl :8080/props | jq .total_slots` is 1 before believing any measurement.
- **`pkill -f llama-server` kills your own shell**, because the pattern matches
  the command line running it. Use `pkill -x llama-server`.
- **A git worktree sharing the main `CARGO_TARGET_DIR` poisons the cache with
  its own paths.** Verifying a commit in a temporary worktree with the shared
  target dir rebuilt a test binary whose `env!("CARGO_MANIFEST_DIR")` was baked
  as the worktree path; after the worktree was removed, the cached binary
  matched fingerprints and kept being reused, and seven tests in an untouched
  crate failed with `NotFound` only on full-workspace runs — hours later,
  looking exactly like a regression in that day's work. **Compile-time paths
  live in the artifact, not the fingerprint: give a throwaway worktree its own
  target dir.** (2026-08-07, in the factory repo; the lesson is generic.)
- **A binary built here crash-looped the box with `status=203/EXEC` — this
  workstation is aarch64, the droplet is x86_64.** glibc versions matched,
  which was the check that got made; architecture was the one that mattered,
  and a wrong-arch ELF fails `exec()` itself, so systemd logs no reason at
  all — the restart loop was the only symptom. **`file` the artifact before
  it ships, and make the pipeline state its target as a contract** — the
  factory release workflow now refuses a non-static binary for the same
  reason. (2026-08-08; the box kept serving because the swap kept
  `factory.prev`.)
- **A config key appended to the end of a TOML file was silently swallowed:**
  the file ended in a `[mail]` table, so the new top-level key parsed as
  `[mail]`'s, serde ignored the unknown field, and the feature simply did not
  exist — no error anywhere, just a button that never rendered. **Appending
  to TOML is position-sensitive** (top-level keys go above the first table
  header), and a deploy should validate the config it is about to serve
  (`factory-deploy` now runs `factory check` before the swap) rather than
  trust that an edit landed where it reads like it landed. (2026-08-08.)
- **A one-shot HTTP stub that answers after one `read()` races the client's
  body write.** The connect tests' stub gate read once, replied, and closed;
  under a loaded parallel run the request arrived in two segments and the
  close became a reset, surfacing as a flaky connection error in whichever
  test drew the slow lane — while passing every isolated run. **A test stub
  must consume the full request (headers plus declared Content-Length)
  before answering**, or the flake lands on whoever loads the machine next.
  (2026-08-07, factory repo.)
- **Two tests that set the same environment variable flake only under
  parallel load.** `set_var`/`remove_var` are process-global and the test
  harness is threaded, so one test's `remove_var` lands mid-way through
  another's read — it never fails in isolation, only when the whole workspace
  runs at once, which reads as a bug in whatever else changed that day. A
  shared `env_lock()` both tests take is the fix; the general shape is that
  **anything process-global touched by a threaded test harness needs a lock,
  and the flake will not reproduce under `-p <crate>`.** (2026-08-07, factory
  repo, `MECHA_HOME`.)
- **Two agent sessions sharing one working tree: a broad `git add` sweeps
  the other lane's files into your commit.** The scheduling lane's commits
  (`37e9a50`, `ad4d8ff` in the factory repo) silently carried the admin-panel
  lane's uncommitted files, so an unrelated arc landed under commit messages
  that never mention it. Nothing broke — every commit still built — but
  blame and bisect now attribute that work to the wrong story. **When two
  lanes share a tree, stage by explicit path, never `git add -A`/`-u`**, and
  before committing check `git status` for files you did not touch.
  (2026-08-07/08, factory repo.)
- **`hf download repo --include X Y`** silently ignores `--include` when
  positional filenames are given. Pass filenames positionally *or* use
  `--include`, not both.
- Free-tier claims in comparison articles are often stale. Exa's own page says
  $10/month recurring credits (~1,400 searches), not the 20,000 some
  aggregators report.

---

## Design notes worth keeping

The rest of the original design-notes section duplicated `CLAUDE.md` and was
dropped in the split. These are the fragments that had no other home.

### The public surface

**A hand-copied list of variants is a bug waiting for the next variant.**
`keys::split` named `mk_pub_` and `mk_drn_` inline, so adding a third scope
minted tokens nothing could parse — and the symptom was `401: a valid bearer
token is required`, a server-side omission reported as the caller's mistake.
It derives from `Scope::ALL` now, with a test that mints every scope. The
general shape: **when a match would have caught it, a copied list will not**,
and the giveaway is an error blaming the other side.

**A guarantee that depends on which client connected is not a guarantee.**
"An agent drafts, a human releases" was a property of mecha's `[outbox] tools`
rather than of the factory, so any other MCP client — or a typo in that list —
silently had none of it. Ask of any safety property: *what happens if the
component holding it is replaced by a different one?* If the answer is "it
quietly stops", the property is in the wrong place.

**Reading the library beat reading its docs, twice.** `rustls-acme` cannot add
a domain at runtime and has no DNS-01 at all — so wildcards are foreclosed by
the crate, not by the DNS provider, which reversed a plan built on moving DNS.
And the blocker on per-user certificates turned out to be one `pub(crate)`
constructor reachable only from the TLS-ALPN-01 path, which HTTP-01 avoids
entirely. Both answers were twenty lines of source and neither was in a doc.

**A manifest is read for resolution, not just for downloads.** Vendoring
Pyodide, the obvious economy was to drop the 359 lock-file entries we were not
fetching. It broke the notebook with no console error at all — a kernel that
never finished booting — because Pyodide reads that file to answer *what is this
package and what does it depend on*, so a missing name is not a missing download
but a resolver that gives up. Keeping every entry and letting an unvendored one
fail at *fetch* made the next bug announce itself by name. **When you prune a
manifest, ask what reads it besides the thing you are pruning for; and prefer
the failure that names itself over the tidiness that does not.**

**A CSP violation and a broken page look identical in a console.** The compute
policy blocked an `eval` in marimo's bundle, which read as "the policy must be
relaxed". It was zod's memoized feature probe — `try { Function("") } catch` —
detecting that dynamic evaluation is unavailable and taking a slower path. The
browser reports a violation; the code degrades. **Before relaxing a policy to
fix a violation, find out whether anything actually broke.**

**Verify the layer you are about to change, not the one you suspect.** When the
vendored notebook still failed, serving the same bundle with the policy *off*
produced the same failure — which proved the CSP was not the cause and sent the
search to the lock file instead of the headers. One differential run replaced an
afternoon of guessing at directives.

**A record-and-replay of a live run misses what it cannot see.** Watching a
browser load the notebook gave a precise list of what it fetched — and omitted
`pyodide.asm.mjs`, which is loaded by dynamic `import()` and does not surface as
a request event. **An observed list is a lower bound.**

**A deferred call carries its arguments and forgot its jail.** Wiring the
factory into this machine (2026-08-06) turned up two bugs that only exist where
a *path* crosses between processes, and neither had bitten because no routed
tool had ever taken one — mail and calendar arguments are text.

First: an unconfined MCP server inherited mecha's working directory rather than
the run's workspace, because only the confined branch of `build_command` set
one. Every server that resolves a relative path — the factory publisher roots
its own `--root` there — was resolving against wherever mecha was launched.
Confinement was never the point; the two branches simply disagreed about where
the model's paths pointed.

Second, and the one worth the entry: `mecha outbox send` runs in a *different
process from a different directory* than the run that staged the draft. The
first release failed loudly, because the agent had written an absolute path and
the reviewer's jail rejected it. The second attempt is the one that mattered —
the agent wrote `{"bundle": "site"}`, and a relative path resolves silently
against whatever the reviewer is standing in. Had a `site/` directory existed
beside them, that release would have published the wrong bytes and reported
success. The fix is to record the drafting workspace on the item and rebuild
the release's tool surface rooted there, which is also the stricter of the two
jails. **A staged call is a deferred tool call, and a tool call means nothing
apart from the workspace it was made in — so the workspace is part of the
draft, not part of the reviewer.** The loud failure was luck: an absolute path
is the case that announces itself, and the design has to hold for the quiet
one.

### The TUI

Written before Shift+Tab phase gating and the `/triggers` modal, both of which
shipped later; what it describes is still how the event loop works.

One event loop owns the terminal for the session; the agent runs in a task
beside it. Enter starts a run when idle and *steers* one already going. Ctrl-C
cancels the run rather than killing the process; Ctrl-C again at an idle prompt
quits. Approval is a modal, because the terminal approver's `read_line` would
fight the event loop for stdin and its prompt would tear the frame —
`setup::prepare_with_approver` exists for that and only swaps the approver in
`Ask` mode.

The steering case that matters, from a real run:

```
● shell  sleep 6 && echo one
● shell  sleep 6 && echo two
● shell  sleep 6 && echo three
↳ change of plan: skip the rest and just reply with the single word PIVOT  (steering)
PIVOT
```

The fourth command never ran, and the run was never stopped and restarted. This
is the only recorded demonstration of steering in the repository.

### The usage frame

A dropped future takes more than the text with it. An interrupted run used to
report **zero** tokens after spending them, because the usage frame arrived at
the end of the stream and the cancellation dropped it. Providers now emit
`StreamEvent::Usage` as counts arrive, and the loop keeps them where it keeps
the partial text: outside the future. Input is known from the very first frame,
which is the expensive half when a cached prefix is in play. The cut turn's
*output* is still unknown, so `RunOutcome::usage_complete` is false and the CLI
prints "at least" — a floor that admits to being one, rather than a guess
dressed as a measurement in the same field a cost budget reads.

### Docker confinement, verified end to end

Measured through the agent on the docker backend, not asserted about an argv:
uid 1000, `~/.ssh` absent, container hostname, 6 environment variables, DNS
dead, and files written into the workspace owned by the user rather than root.
That last one is the `--user` flag, without which the agent leaves root-owned
files you cannot delete.

### `env_passthrough` is a breaking change

Replacing environment inheritance with an allowlist took a nosy test server from
64 variables including two API keys to 3 and none. Any MCP server that relied on
inheriting a token stops working until the variable is named in
`env_passthrough` or set outright in `env`.

### Subagents inherit the caller's workspace

Not the one that existed when they were built. This closed a jail hole: a parent
running in a sandbox used to delegate to a child still pointed at the original
directory.

### Taint was verified across a process restart

A page fetched in one session, a file read in the resumed one, and the outbound
call refused. Provenance cannot be recovered by reading a transcript back, so
without the taint record in the session file, resuming laundered it. The
regression test was checked to **fail against the old behaviour**, not merely
pass against the new.

### Four eval-rig details

- **`verify` hashes the test file first**, so a model that edits the tests until
  they pass fails. Grade the artifact, not the claim.
- **A judge that cannot answer must fail the case, never skip it.** A case whose
  only real assertion silently evaporates is worse than one that fails loudly.
- The judge is selected with **`--judge-provider` / `--judge-model`**.
- **`min_compactions` exists** so a compaction case fails loudly when the
  transcript never crossed the threshold, rather than passing and reporting
  fidelity it never tested.

### One authoring convention

Write tool error messages **for the model**. "not found; the directory contains
a.md, b.md" is a self-correcting loop; "No such file" is a dead end.
