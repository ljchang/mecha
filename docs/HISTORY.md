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

**2026-08-09 (night) — the agent gets the factory's other four capabilities,
and drift becomes a build failure.** `factory-publish` had grown to twenty
commands while its MCP server stayed at seven, all `bundle_*`: notebooks,
request types, availability slots and **polls** were unreachable by any agent,
and nothing anywhere failed while that was true. The way anyone found out was
asking mecha to make a poll and being told, correctly, that it had no such
tool.

Why it stayed drifted decided the work. The capabilities were command bodies
inside the *binary*, where `mcp.rs` in the library could not reach them —
`polls_command` alone was 470 lines. So `polls.rs` is the extraction, and the
rule it encodes is the transferable part: **a capability is a function, and a
front end is a printer.** The CLI's output and its `--json` shapes are
unchanged, which is why the printers still compute from the same pure tally
functions rather than re-reading a serialised tally — a "refactor" that
quietly reformats a number an agent workflow parses is not one.

Eight tools joined: `poll_create` and `poll_meeting_create` (two tools rather
than one with a mode flag — the modes take entirely different inputs, and a
mode flag is what a model gets wrong), `poll_status`, `poll_close`,
`notebook_render`, `type_check`, `type_push`, `type_list`. Everything that
mints a public URL carries `openWorldHint` and is named in `[outbox] tools`, so
it stages instead of reaching the world. The meeting poll takes freebusy as a
**file path**, since MCP cannot pipe stdin the way the CLI does; the
under-an-hour freshness and horizon refusals sit untouched behind it.

**A poll collects other people's words, so the front door's rule was borrowed
whole.** `poll_status` returns typed tallies and the *counts* of free-text
answers, never the answers: `Status::for_privileged_run` is the function, and
there is deliberately no argument that hands the prose over. `polls export` is
excluded from the tool surface for the same reason — it would write those words
into a file `fs_read` cannot tell from bytes we wrote ourselves, which is the
boundary with an extra step rather than a boundary. A residual is recorded
rather than papered over: question prompts still come back from the box, so a
compromised origin could rewrite the user's own question text.

The durable half is `surface::REACH`: every CLI command is either exposed as
named tools or excluded with a written reason, and three tests fail the build
when a command is neither, when a row names a tool the server does not serve,
or when a served tool no row accounts for. `drain` had been the one
*documented* exclusion; `slots push`, `operator`, `connect`, `serve` and the
rest now say in writing why they are not an agent's. The test was verified
non-vacuous by deleting a row and watching it name the gap — which is the only
thing that distinguishes a coverage test from a comment.

The mecha side took two fixes, the second found by the first. `local_paths`
learned `spec` and `manifest`, so a staged poll or form has a file to open
rather than a JSON blob to squint at. Then staging the first real poll end to
end reported its spec — a file sitting right there — as "⚠ gone", because
`show` resolved a relative argument against wherever the reviewer was standing
while `send` had always used the recorded jail. See the trap below.

**2026-08-10 — the poll page is looked at, and two beliefs do not survive it.**
The session began as "expose the factory's tools" and turned into a review of
what those tools actually produce, which is the only reason any of the following
was found: every defect here was visible on a rendered page and invisible in the
source.

**The gallery's survey page had no survey CSS at all.** Every page in that family
links one asset named `booking.css`, and two builders produced it — the booking
pages omitted `SURVEY_STRUCTURE`, the survey pages included it, and the gallery
writes the first one it meets and marks the job done. So the page a reader learns
the component from shipped with none of its rules: list markers where
`list-style:none` was meant to be, rank buttons wearing the full-size accent pill
that `button` is. The live box built its assets down a third path that happened
to be correct, which is exactly what kept it hidden. `page_style` is the single
definition now, with a test that holds a really-rendered page against it and
names the rules that must be present.

**Ranking's drag moved exactly one row.** Pointer capture was taken on the grip;
the first reorder moves the row's `li`; `insertBefore` on an attached node is a
remove and an insert; removing the capturing element releases the capture. Every
`pointermove` after the first went nowhere. Capture moved to the list, the one
node in the widget that never moves, and the row now follows the pointer and
swaps on midpoints rather than on whatever is under the cursor — which returns
the dragged row itself once it moves, a stall of its own.

Likert and VAS were laid out by `flex-wrap`, which is not a layout: the fifth
point of a five-point scale wrapped to a second line, where it stops reading as a
scale, and the VAS anchors flowed inline so "100 — …" landed below the slider
describing nothing. The scale became a grid of equal columns; the anchors got
half the width each. A choice became **a card you press** rather than a dot you
aim at — the radio stays in the markup and the tab order, since it is what posts,
what a screen reader announces, and what works with the script blocked, but
`:has(:checked)` moves the selected state onto the whole card.

**Two features, both presentation kept out of meaning.** `layout = "horizontal" |
"vertical"` lives on `PollQuestion` rather than in `QuestionKind`, because the
kind is what a question means and a tally must never move because somebody
rearranged a page. `media = { src, alt }` puts a picture on a question or an
option — and *where the bytes may come from was decided by the
Content-Security-Policy, not by taste*. Every class sends `img-src 'self' data:`,
so the obvious design (publish a bundle of figures, point at it) does not
survive: the artifact host is a different origin and the browser blocks it. An
off-origin `src` is refused when the spec is parsed. What remains is inline
images capped at 512 KB, which is a generous figure and a hopeless photograph —
recorded in the handoff as needing an asset endpoint on the box rather than
quietly shipped as though it were finished.

**Two reversals, both of things this project had written down as true.** The
notebook export does **not** execute the notebook: measured on marimo 0.23.16,
where a cell body that writes a file does not write it, a statement at module top
level does not run, and a notebook importing a nonexistent package and raising
`SystemExit` exports cleanly with exit 0. `export html-wasm` parses the file
without importing it. That claim had been load-bearing — it was the stated reason
the notebook template stayed off unattended paths. And `poll_status` went back to
returning free text, after a first version withheld it on the front door's
reasoning; in a poll the prose *is* the data, and `openWorldHint` already marks
it untrusted and arms the interlock exactly as a mail body does, so withholding
was stricter than mecha's treatment of the user's own inbox.

The documentation caught up in the same pass: `factory/onboarding.md`,
`factory/artifacts.md`, `factory/notebooks.md` and `features/slack.md`, plus the
layout and picture sections in `polls.md` — four of the gaps a reader hits in
their first hour and could previously close only by reading source.

**2026-08-10 (later) — two releases, a deploy, and a ceiling that was never
there.** `v0.1.1` and `v0.2.1` are cut and published: four mecha crates at
0.1.1, three factory crates at 0.2.1, and a checksummed static `factory` binary
attached to its release. That closes "verify the release workflow" and the
crates.io split, both open since 2026-08-07. `factory-deploy v0.2.1` then ran
the whole procedure against a real release for the first time — download,
checksum, prove the binary and the config while the old one still served, swap,
health check — so the poll rendering work is in front of visitors rather than
sitting in a repository. Home switched to installing from crates.io rather than
the repo build, which is the better habit once tags exist: it installs exactly
what the world gets.

Then the day's largest finding, which was not planned work at all. `-c 32768`
had been pinned for three days by a single bad afternoon, and the rule written
from it — *do not raise `-c`* — turned out to be four times larger than its
evidence. Re-measured across 32k, 64k, 128k and 256k at matched prompt lengths:
**`-c` costs nothing.** The server now runs at **262,144**, the model's whole
trained window, and a needle at 21% depth in a 188,546-token prompt came back
exactly. The compaction threshold moved from 21,845 to 174,762 as a side
effect, which is recorded in the handoff as a decision somebody still owes.
`[slack] tools` was emptied in the same session, so a Slack thread now carries
the same surface as `chat` — the token argument that justified narrowing it was
real, and stopped mattering when the window grew eightfold.

**2026-08-10 (evening) — the empty turns were half ours, and the model gets
its reasoning back.** `v0.1.2`. The Terminal-Bench subset was relaunched at
the 262k window with `max_turns` 80 and a 2.0x agent-timeout multiplier; 27 of
75 trials in, it had 12 passes, five agent-timeouts and one trial lost to a
crash that turned out to be a bug worth the whole run. Reading that crash
opened the day's real finding.

`reasoning_content` appeared nowhere in the tree. A local reasoning model's
entire think channel was being discarded on every turn — invisible in the TUI,
absent from every transcript, and unavailable to the diagnosis that had been
chasing "empty turns" since 2026-08-07. Replaying the exact prefixes that went
quiet reproduced one, and its reasoning held a complete, unparsed tool call;
in a second case, 120 characters that were *only* a tool call with no
deliberation at all. The model emits its call before closing `</think>`, so
llama.cpp files the whole turn as reasoning and reports a clean stop
(ggml-org/llama.cpp #20837, #22684, #20809 — all unfixed, same failure
reported against ollama). Which killed the standing explanation: 120
characters is nowhere near any limit, so `--reasoning-budget`, `max_tokens`
and the window were never relevant, and every mitigation aimed at "the model
reasons too long" had been aimed at the wrong failure for three days.

Then the half that was ours. Because the history sent back stripped every
`<think>` block, the model was shown turn after turn of itself apparently
calling tools without thinking, and it obliged. Same server, same template,
same prompt, varying only whether the history carried reasoning: **6 of 6
empty turns without it, 0 of 6 with it**, p ~= 0.001 on a reproducer that
fails byte-identically. A third-party replacement chat template fixes it too
(6/6 to 0/6) but only by *instructing* the model, and only for Qwen; the
history fix addresses the cause and names no vendor anywhere.

So reasoning now decodes to a `Block::Thinking`, streams as `ThinkingDelta`,
rides back to the provider on the next request, and is recorded — with the
full trace of an empty turn going to the debug log, since such a turn reaches
no transcript. Preserving all of it is affordable because the prefix cache
absorbs it: measured, 5,000-token prompts prefill 16-211 tokens, better than
95% reuse. That reuse had been invisible because `decode_usage` never read
`prompt_tokens_details.cached_tokens`, which is now read and — the part that
matters — *subtracted* from `input_tokens`, because `total_input` sums the
tiers and the compaction threshold reads the sum.

The crash that started it was its own bug: `pytorch-model-recovery` describes
itself as a bulleted list, so its instruction opens with `- `, clap read it as
a flag, and the run exited 2 before starting — which Harbor scores 0.0,
indistinguishable from a model that tried and failed.

**2026-08-11 — the fix is measured, and it moves the constraint rather than
removing one.** The v0.1.2 relaunch ran 46 of 75 before being stopped, and on
the 28 tasks both runs reached: passes **13 → 15**, wall clock **10.5h → 6.0h**,
timeouts **5 → 2**, crashes **1 → 0**, empty-turn nudges **41 → 0**. The speed
is the mechanistic part — per-turn cost collapsed toward a floor, and
`break-filter-js-from-html`, already at that floor at 14s/turn, did not move
while trials at 35–100s/turn fell to 6–39s. The score is the modest part, and
its shape is one story: three of four flips to a pass were trials the old run
stopped *too early* (3 turns, 12 turns, one that never finished), and the one
flip away was the same persistence overrunning a wall clock. The model did not
get smarter; the harness stopped hiding what it could already do.

Two open questions closed with numbers rather than argument. **Unbounded
reasoning replay does not drive compaction** — 1 trial in 28 before, 2 in 46
after, and the trials hit the turn cap long before the 174,762-token
threshold, so the "bound it to the last N turns" design would have solved
nothing. And **`max_turns` is now the binding constraint**: 26% of trials
ended at exactly 80, passing 30% against 50–67% in every band below. 80 was a
ceiling this project chose while the real constraints were empty-turn waste
and the clock; removing those moved the binding one and nobody re-derived the
number. Terminal-Bench's own default is 200, so the third run uses that.

One correction worth recording because it nearly became a change: the session
proposed adding tool-output offloading, and `ToolCtx::spill_dir` had done
exactly that since 2026-08-05 — 12 of 46 trials spilled, with the marker
naming its own `fs_read` recovery. The follow-on proposal, to lower the
budget so spilling fires more often, was refuted by this repo's own
`CONTEXT-RESEARCH.md`: one arm cut tool-output tokens 38.4% and cost **6.8%
more**, r = 0.154, because trajectory length dominates and re-reading spilled
output costs turns. Reasoning from context *occupancy* without checking the
*cost* evidence already collected here is what produced both errors.

**2026-08-10 (night) — an artifact gets the URL a person can open, and the
review bot turns out never to have run.** Two arcs, one on each side of the
repo boundary.

The factory's publish path reported one URL for a bundle — the bytes on the
artifact origin — and reported it whatever the visibility. Since a publish key
cannot move an alias by design, the state every agent publish *lands in* is
private, where that URL answers 404; the tool said "It is live at" about it,
and a model repeated the dead link to whoever asked. Measured against the live
box before anything was written: `…/b/morning-brief/` → 404, and the gate's
viewer page for the same bundle → 200. The server now answers with both
(`viewer_url`, `viewer_version_url`, minted by one helper so the publish and
alias responses cannot drift), the publisher funnels every report through one
`Mirrored` type, and the MCP tool descriptions say which URL is for a person —
that last part being where it actually gets fixed, since a model repeats
whatever string the tool returned. Documented in `artifacts.md` as a named
section, because the page taught the wrong URL as *the* one you send someone.

Three things fell out that were not the arc. Following the URL a publish
reports, while signed out, landed an owner on the reader sign-in form — which
mails a link to an address a *share* names, and an owner holds no share to
their own bundle, so the form answered "check your email" and nothing came;
the page carries the tenant sign-in corner now, which leaks nothing because it
is identical on every refusal. The account page linked every bundle through
`Role::Artifacts` unconditionally, so each published notebook pointed at an
origin it does not live on and was rescued by a redirect. And `factory-publish
push`, which has no `--visibility`, read one out of the *local* store and sent
it — a store that never hears about a release made from the account page, so
the next push silently took public bundles down. Visibility is `Option` end to
end now, and `None` omits the field, which is the box's own spelling of "leave
who may read it alone" and was always what the server implemented.

A 26-agent review over the branch found ten issues, nine fixed. The worst was
the arc's own: a `for_a_person()` fallback substituted the bytes URL when the
box named no page, so the private sentence read "only you can open it: <URL>.
<the same URL> serves nobody" — the exact lie the change existed to remove,
reinstated in the one arm the deployed box still exercises. Its guard had
certified it, because the test only covered the single state where the
substitution happens to be true.

The other arc was the CI bots, and it began from a wrong premise — that
mecha-factory needed switching from API billing to plan billing. It had never
had the workflows at all. Checking the run logs to port them turned up three
things: the model was `claude-sonnet-5` everywhere, since neither repo ever
pinned one; `claude.yml`'s condition used a literal block scalar and the job
had **never once run** (a real `@claude` comment left it skipped in 0s with no
steps); and the review job had failed on every run it had ever made — 31
failures in its last 40, identical before and after the OAuth switch, so
subscription auth neither caused nor fixed it. That last was the credential:
an API key that had presumably run dry, then an OAuth token that was never
valid. A fresh token took the same job from `is_error: true`, 1 turn, 304ms to
a real nine-turn review. Both bots now name `claude-opus-5` and both work.

---

**2026-08-14/15 — Slack acts, the doctor reads across the stores, and 0.1.3/0.1.4
ship.** (Bridge entry, reconstructed from `CHANGELOG.md` — the sessions that
built these predate this handoff pass; the changelog entries for 0.1.3 and
0.1.4 are the detailed record.) The hardening branch merged 2026-08-14 (ten
review findings fixed), Slack gained executable actions behind the closed
`Action` enum with the tainted two-step, and `mecha doctor` landed as the one
reader over every store after the 2026-08-11 incident where a revoked OAuth
token took scheduling down for three days while five stores each recorded the
distress correctly and nothing read across them. v0.1.4 was tagged 2026-08-15
— and the deploy that followed is the reason the `update` skill exists: the
tag published crates while `mecha --version` still said 0.1.3, because
nothing had run `cargo install`.

**2026-08-16 (afternoon) — DeepSeek's harness is read, and mecha keeps four
ideas.** A survey of the just-released DeepSeek Harness (dsh) produced a
ranked steal-list, and the same day shipped it as v0.1.5: `recall` (the
session transcript is searchable, including what compaction rewrote away —
taint-neutral by construction, since everything it returns entered the
conversation once and its arrival is what armed the interlock), the cache
lens (per-run observer that names why prompt-cache reuse broke and warns only
on unexplained re-payment), the Landlock sandbox backend (no-privilege file
confinement that deliberately never earns the interlock relaxation, because
UDP is unrestrictable at any ABI), and the recording-gap fix
(`Conversation::rewritten` — a run long enough to compact itself no longer
loses its own head, because `record_run` now takes the conversation and walks
the pre-rewrite states a caller could otherwise skip). The survey also found
dsh has no taint tracking, no send-sink concept, and no summary validation —
the trifecta work has no counterpart there. Spill, the fourth idea, turned
out to have shipped here already (`cap_result`), which the research memory
now records against its own earlier wrong claim.

**2026-08-16 (evening) — the knowledge graph becomes a public sibling.** pkg
went public as **mecha-graph** in a clean-room extraction: fresh history
(the private repo's 137 commit bodies narrate live measurements about named
people — no filter makes a journal safe), a canonical replacement map that
preserved every test's linguistic property (Ana⊂Anastasia⊂Lana for the
word-boundary family, Ada B/Ada B. for initials dedup, the June trio for
same-first-name disambiguation), a synthetic eval world measured at 23/24
with one deliberately hard bridging query left as headroom, and a
private-only denylist gate plus export script that every publish must pass.
Two independent audits ran, the second given no list of what the first
removed — and it found what the first had rationalized: a "replacement"
address that was a real Vermont locality, a real founder kept under a
public-figure excuse, and household facts renamed rather than fictionalized.
Three crates published to crates.io (`mecha-graph` took the bare name; the
CLI installs as `cargo install mecha-graph`), the repo flipped public the
same night.

**2026-08-16 (night) — the graph's names reach all the way down, and 0.1.6
ships.** The rename went end to end: crates and binaries
(`mecha-graph-core/-cli/-mcp`, dirs included), env vars (`MECHA_GRAPH_*`),
the store (`~/pkg` → `~/.mecha-graph`, migrated live and verified by reading
13k episodes back), and — the piece that needed a mecha feature — the tool
names: `prefix_tools = false` on `[[mcp]]` registers a server's tools under
their raw names, so the model calls `kg_search`, not `pkg__kg_search`.
Unprefixed is a promise of distinct names, enforced by a loud startup
failure on collision (a prefixed name always contains the `__` marker, so
the cases cannot be confused). The eval followed — `graph-cases.jsonl`
grades the surface that actually runs, with the comparability break across
the rename stated in the case file rather than discovered later — and
running the renamed set immediately caught a latent `--mcp-file` bug that
had silently broken fixture evals since MCP servers started spawning in the
run's workspace. v0.1.6 tagged, published, deployed, and the graph now
installs with mecha (`~/.cargo/bin/mecha-graph-mcp` in the config, the
update skill's step 1, and the README's install block).

**2026-08-17 (small hours) — the signup door opens.** mecha-factory v0.2.5
tagged and deployed via `factory-deploy` (download, checksum, prove, swap,
health-check; `factory.prev` kept): the two commits that had sat unreleased
on main — `d11cf10`, anyone may ask for an account at
`gate.mecha-factory.ai/signup`, and `9823f26`, redirect hosts get their own
certificates — are serving. Verified live: `factory --version` 0.2.5,
`/signup` answers 200 with "Create an account". Found during the handoff
pass, deployed on the owner's word an hour later — which is the handoff doc
doing its job.

**2026-08-17 — the operator gets the door back, with the arithmetic in
view.** v0.2.6, designed and deployed the same night on the owner's call:
an ask now files a `signup_asks` row (schema 15) and nothing else — the
certificate budget is spent at *approval*, on the panel, where a new
Signups section shows committed-of-40 split into accounts and pending
invites, the free count, and the hour the oldest slot returns, computed by
the same function the approval handler enforces so the panel can never
disagree with the refusal. Approve mints and mails through the one
definition every mint uses; Deny closes silently, which is what the ask
page now promises; a spent week queues instead of pausing — the
forty-burner-addresses attack that used to spend the week's certificates
now fills a queue the operator bulk-denies, and the budget never moves.
Same bytes for every address, still. 486 tests, the approval journey
end-to-end among them.

**2026-08-18 — mail learns to triage, and the rules budget stops being one
number.** `mecha-mail` could read, send and write the calendar but not
archive; the whole write surface was send/reply plus calendar CRUD, so any
triage UI over it could move a cursor and draft and never empty an inbox.
`mail_triage` adds archive/read/unread/spam/trash as a closed `TriageAction`
enum, thread-level, landing in a capability quadrant the surface did not have:
`destructive` but not `external_send`, because it mutates the user's own
mailbox and reaches nobody — so it must never sit in `[outbox] tools` (staging
it would make the loop review a queue in order to fill another queue) and must
not be `readOnlyHint` either, or a read-only trigger could empty the inbox at
07:00. `assert_tool_surface` grew a third slice asserting both negatives.
Tagging was deliberately left out of the provider entirely: Gmail labels and
Graph categories are different objects, a tag meaning different things per
account fails at the one job tags have, and a mecha tag costs no scope at all.

The scopes moved with it — `gmail.modify` (stopping short of
`https://mail.google.com/`, whose only addition over it is irreversible
deletion) and `Mail.ReadWrite` — and both accounts re-consented the same day.
The expectation going in was that Dartmouth would be blocked on IT and Google
would be easy; it was the exact reverse. Dartmouth's Entra registration
already had `Mail.ReadWrite` Delegated granted tenant-wide, while the Google
client turned out to be in Testing publishing status, which caps its refresh
tokens at seven days and needs a CASA security assessment to escape.

The same pass raised the learned-rules cap 15 → 25 with `RULES_CHAR_BUDGET`
1600 → 2600 beside it, and — because that made the cost of the existing leak
visible — stopped every domain riding in every run's prefix.
`rules_prompt_block_for(RUN_DOMAINS)` gives a run `behavior` + `writing` and
nothing else, new domains are opt-in, and `unrouted_domains` warns at startup
about any domain holding rules no run carries. Everything reconstructing "what
a run sees" moved with it, because the validation ledger is keyed to the rule
set measured. Two research docs landed alongside: `MAIL-UX-RESEARCH.md` and
`SKILLS-RESEARCH.md`. 904 tests.

**2026-08-18 (afternoon) — mail learns to be a queue, and the release waits
for it.** Phases 1-3 of `MAIL-UX-RESEARCH.md` landed: the triage store, the
quarantined classifier, `mecha mail`, the escalation rule and a nightly timer.
The classifier is the front door's construction one directory over — no tools,
no history, no system prompt, no shared cache prefix, one isolated call per
thread — and `Record::for_privileged_run` is a function with no argument that
returns the prose. `one_line` stays behind it, which is the judgement call:
short, exactly what a summary wants, and model prose derived from attacker
prose, which is the laundering path the front door withholds `reading` to
close. Verified on 51 real Dartmouth threads. The documents work merged the
same day from a parallel session, and Luke then held 0.1.7 until the mail
feature is complete through phase 6.

**2026-08-19 — the harness learns to watch itself.** A research pass on what
separates good harnesses (`docs/HARNESS-RESEARCH.md`) found the largest
measured lever is plumbing rather than cognition — Claw-SWE-Bench moves the
same backbone from 19.1% to 73.4% purely by making patches apply — and that
persistence inverts: models self-condition on their own errors, an effect that
does not go away with model size. Three fixes followed the same day.
`compact::collapse_repeated_failures` folds a pile of identical failures onto
its newest member, since eviction exempts errors by construction and thinning
only truncates long results, so eight failed attempts at one call rode in every
subsequent request forever. `RunOutcome::ended_on_failed_call` names the run
that stopped of its own accord with its last call failed — the silent-failure
shape no judge catches at better than chance — with an `expect` check beside
it. And doctor learned to see a trigger degrading quietly, then a trigger that
succeeded having done nothing at all, the second found by a sibling session
hitting the identical shape one layer down.

**2026-08-19 (afternoon) — the self-improvement loop, built and unstarted.**
A second pass (`docs/SELF-IMPROVEMENT-RESEARCH.md`, 733 lines) asked whether
the harness can notice its own problems and fix them; the answer was that
rumination's only sensor is a human stepping in, so a harness problem produces
no intervention and nothing downstream ever sees it. Six commits built the
missing half: `Record::Outcome(RunStats)` written by every front-end,
`runlog.rs` reading the corpus back with `mecha sessions health`,
`candidate.rs` as a pure gate (paired comparison, deterministic holdout, a work
guardrail that rejects a gain bought by attempting less), `RunStats` threaded
through the replay driver, `mecha eval --ab-config` as the content-sensitive
arm, three population checks in doctor, and `diagnose.rs` plus `mecha diagnose`
— the one place a model authors a change, safe there because being wrong costs
one measurement and unsafe at the accept gate for the same reason. Luke's
rulings moved the gate from provenance to proof: the diagnostician may search
the web and its output is not restricted to numbers, architecture changes are
proposable behind a human gate, everything else auto-accepts on measured
improvement. Nothing acts on the numbers yet, deliberately — the corpus was
empty at build time (178 sessions read, 0 outcomes), and the research's own
finding is that agents update their harnesses without benefiting, so the next
thing to learn is whether doctor's findings would actually have been acted on.

**2026-08-19 (midday to evening) — mail stops being a queue and becomes a
surface you work.** Recorded on 2026-08-25, six days late: this arc shipped
between 11:08 and 18:10 and neither document closed it out, so the plan
written that morning went on reading as a to-do list. Every phase the
`MAIL-UX-DESIGN.md` roadmap named landed in one run. **Phase 4′, the
pre-filter** (`3547cb1`): `List-Unsubscribe` plus an automated-sender rule
ahead of the classifier, disposing of a little under half of all threads
without a model call — and keeping three properties, that it only ever
produces `ignore`, that it reads the envelope and never the body (which is
where an injection lives), and that `List-Unsubscribe` alone is not enough,
because it finds marketing, which must offer an unsubscribe, and misses
institutional and transactional senders, which need not. `PrefilterRule` rides
on the verdict so the rule can be **graded rather than believed** — without it
a thread the pre-filter dropped and one the classifier called `ignore` are
indistinguishable afterwards, and "is this too aggressive" has no way to be
asked. **The corpus became a scorecard** (`e2af8a5`, `4cafaf1`): `mecha mail
eval` grades against a year of mail whose outcome is known, sampling both
strata rather than the corpus uniformly, because answered threads are rare and
a uniform run of 200 would hold a handful of the only threads carrying ground
truth. **Phase 6, corrections** (`3329c39`, `f6bac35`, `0166120`, `c440700`,
`40a1dc7`) — field-level, feeding a few-shot pool and a `triage`-domain
reflection — and `mecha mail score` (`5151f22`) for the live store, which is a
different question from the corpus and so a different verb. **Phase 4″**
(`05581fb`): a thread becomes a board task or parks with a reason. **Phase
4‴** (`0001ae6`, `a14c136`, `8231163`): day two as a store-side primitive, so
the morning briefing is one reader of it rather than the owner — and
"nothing new to surface" stopped being written as "nothing is waiting".
**Phase 5** (`3ef4e60`, `78d73d9`, `037d9c8`, `a34a1c2`): `/mail` as a modal,
ending with `r`, `f` and `e` no longer stubs — `f` being **forward**, which had
had no key bound to it at all, though the receipts-to-the-finance-person case
was one of the five that motivated the whole feature. An eleven-finding review
ran over it the same evening, and its first finding was the whole loop.

**2026-08-19 (night) — skills, and the queue stops asking people to read
JSON.** `mecha-core/src/skill.rs` and the Agent Skills format: a procedure the
*user* writes and the model loads when it decides one is relevant, at three
levels of disclosure — name and description in the prompt, the body on a
`skill` call, a bundled file only when the procedure points at one. The
absences are the safety argument and are structural: no install command, no
registry, no remote body, nothing derived from a session, no project-layer
store, and therefore no taint when one loads. It is also the pressure valve for
the per-domain rule cap, since a procedure too long for a rule and irrelevant
on most runs now has somewhere to live. Alongside it the review surfaces
stopped rendering arguments at people: a staged message shows as the letter it
is, `edit` opens the prose instead of a string literal, resolved drafts go
behind `h`, and `/mail` grew a pinned key strip and an `enter` that opens the
thread instead of fetching one and printing its subject into a title bar.

**2026-08-20 — documents, and four arcs in one night across three sessions.**
`mecha-docs` shipped as the fourth binary on `mecha-mail` (`google/docs.rs`,
`google/docs_server.rs`), and the scope *is* the design: `drive.file` is the
one non-sensitive scope in the family and covers only files the app created or
the user handed it through Google's own chooser, which is the path jail applied
to Drive — provable by reading a scope string rather than by reviewing every
future diff. Then, in one evening: **the task board reached the session** —
`mecha tasks` and `/tasks` onto the graph's GTD board over `kg_task_*`, with
the same status letters as `mecha-graph tui` screen 6 and a test saying so;
**`/skills`** as the slash counterpart to `/tools`, reading what a run carries
from the running agent rather than re-deriving it from config; **a reply became
reviewable with the message it replies to**, recovered from the transcript that
already held it rather than from a re-fetch, joined by identifying arguments
matched by key and value so nothing about mail is special-cased; and **eight
modals stopped taking the session down when the window shrank**. Released as
0.1.9 the same night, with the site's pages for the three arcs that had
documented nothing outside `CLAUDE.md`.


**2026-08-20 (later) — five surfaces that stated their size in one unit and
drew in another.** Reported as "the prompt gets wonky — the cursor is
misaligned, things are not clearing and we get collisions", and it was two
causes wearing one costume. The collisions were not a rendering bug at all:
`tracing` writes to stderr, and under `mecha tui` stderr *is* the alternate
screen, so a `warn!` mid-run painted its bytes through the frame and stayed
there forever — ratatui repaints by diffing its own buffer and therefore never
repaints cells it did not write. `mecha-cli/src/logs.rs` holds the lines while
a front-end owns the screen and the loop drains them into the transcript,
bounded, colour stripped, with the remainder flushed to the real stderr on the
way out. The caret was the second cause: `input_layout` counted characters and
broke at `width` while the `Paragraph` beside it word-wrapped and measured in
display cells, so two implementations of one question drifted apart on every
line of prose. `input_layout` now returns a partition of the text into row
ranges and `draw` renders those rows, with no `.wrap()` left to disagree.
Underneath three more fixes was one repeated mistake — `body.len()` is not the
drawn height when the paragraph wraps — which had `/help` truncating `/doctor`
and `/review` mid-sentence, `/tools` hiding its declared-capability block below
an unreachable fold, and `/tasks` pushing the task id off the one view that
exists to show it. `typed_char` closed a separate class in the same pass:
`KeyCode::Char('c')` with CONTROL held is Ctrl-C, and `/mail` had been feeding
it to `action_for`, so Ctrl-A archived the thread under the cursor and Ctrl-R
started a drafting run. Shipped alongside `/docs`, the ninth modal, which
exists because `mecha-docs pick` printed a URL and then blocked on stdin — the
browser leg split into `pick --url` and `pick --redirect` so it can span two
commands minutes apart, which is what lets a document enter `drive.file` scope
from a headless box with no tunnel and no forwarded port. `cfa2cc2`.

**2026-08-21 (later) — the model had eyes, and the file being served held
half of it.** A screenshot sent from Slack was answered with "I don't have the
ability to view image files directly", which reads as a limitation and was a
misconfiguration. Qwen3.6-35B-A3B is multimodal — `general.tags` says
`image-text-to-text`, `rope.dimension_sections` is `[11, 11, 10, 0]` (mRoPE,
which a text-only model does not have), and the chat template emits
`<|vision_start|>` — but the GGUF holds 750 `blk.*` tensors and no vision tower
at all. That ships separately as `mmproj-*.gguf`, and nothing had downloaded it.
Four of four multimodal models on the machine were being served text-only, one
of them with its projector already on disk, unused, since the day it arrived.

Both halves shipped. On the server, `scripts/mmproj.sh` is sourced by every
start script and *refuses to start* without a projector, printing the `curl`
that fetches it. In the harness, `Block::Image` is a fifth block variant —
user turns only, because Anthropic accepts an image inside a `tool_result` and
the OpenAI dialect's `role: "tool"` messages carry a string and nothing else,
so a tool returning pixels would work on one backend and silently lose them on
the other. Both backends degrade to a named line rather than failing and are
tested to word it *identically*, because a conversation crossing a `/model`
switch must not tell two stories about its own history. The parts array is
built only when an image is present: the cached prefix is a byte-prefix match,
so making it the uniform shape would invalidate every run that never sends one.

`mecha_core::image` caps at the door rather than per turn — the transcript is
append-only and every turn resends the whole history, so a resize is paid once
and collected forever after. Measured on the screenshot that started it:
2222x1548 / 5.7 MB → 1568x1092 / 179 KB, with llama-server reporting
`prompt_tokens` 294 **either way**, because it tiles to a fixed count
regardless. 32x off the wire and out of the session file for nothing in
context. An image already under the caps passes through byte for byte, since
re-encoding a crisp screenshot of text is a real loss and that is the case this
exists for.

Four doors reach it: the Slack connector, the remote-control inbox,
`mecha run --image`, and a file **dropped on the TUI prompt** — which turned
out to be half-built already, since a terminal converts a drop into a bracketed
paste of the path and the `Event::Paste` arm had always inserted it. What it
never did was look at what the path was. Now a paste whose every token resolves
to an existing image becomes a chip, `[image: shot.png]`, with the bytes held
beside the input; an image is sent only if its chip survives to submit, which
is the only undo there is for something backspace cannot reach. That
conjunction is the safety property rather than a convenience: a paste is also a
paragraph off a web page, and attaching any file whose path appeared *somewhere*
in pasted prose would let copied text pull bytes off the disk into a request.
It cannot work over SSH and never will — the path pasted is the laptop's and
the process resolving it is on the far box, which is precisely what the Slack
conduit is for.

`provider::preflight` is the thing that stops this recurring: one `GET /props`
at startup, compared against config in **both** directions, because they fail
differently — declared-but-not-served silently degrades every image to text,
while served-but-not-declared means a projector is loaded, paid for in memory,
and never used. The same request checks `context_window` against the per-slot
`n_ctx` (so the `-c` versus `-c / -np` rule is read rather than restated) and
the configured model against `model_alias`. It warns and never refuses — the
opposite of the sandbox's bargain, and for the opposite reason: there, falling
through means running unconfined; here a mismatch means compacting at the wrong
moment, and a preflight that can stop a working machine from booting is one
people turn off.

`mecha setup` generalises that into onboarding, on the finding that
documentation had already stated all of this correctly while the machine stayed
wrong for months. It does not ask what you meant; it reads `/props` and writes
down what the server reports, and `--write` edits the table in place rather
than reserialising, because a round trip through a TOML parser discards every
comment and in this project the comments are most of the file. Its planner is
pure and tested without a machine, on the `compact.rs` split: getting
onboarding wrong is silent. Two boundaries have tests naming them — the
knowledge graph is *named and never driven*, since spawning `mecha-graph
source` would be exactly the coupling the MCP-only rule prevents, and nothing
schedules anything, with the runner offered only once a trigger already exists.

The last finding came from a run that **worked**. The fixed Slack test answered
correctly and recorded `taint {private: false}`, where the same user action
before the arc recorded `{private: true}` — because the model used to have to
`fs_read` the attachment, and putting the pixels on the user turn removed the
tool call and the taint with it. A feature that loosens the interlock as a side
effect, with a correct-looking answer sitting on top of it. `Taint::
arm_for_content` now arms the private leg for an attached image, enforced in
the loop rather than in `Conversation::push` — `slack/connector.rs` appends to
`messages` directly, so arming at the tidy place would have left unarmed
precisely the surface people attach screenshots from. The argument for treating
an image differently from typed text is that **a screenshot is captured, not
composed**: you choose every word you type, and you choose the window rather
than everything in it.

**2026-08-21 (later still) — onboarding, audited by running it.** The
new-user path was walked against a genuinely fresh `MECHA_HOME` rather than
read, which is the only reason five defects turned up. `first-run.md` — the
page a beginner follows — taught `context_window = 32768  # the -c the server
was started with`, the exact mistake `features/serving.md` calls "the trap
worth knowing before you meet it": two pages describing one number, and the
beginner's page lost. `triggers.md` said `cp scripts/mecha-triggers.service`,
which `cargo package --list` shows ships **zero** files, so it could not be
followed by anyone installing the way the docs lead with. Nothing anywhere
mentioned vision, `mmproj` or images, so the day's expensive bug was
undiscoverable. `config init` wrote a local block with neither
`context_window` nor `vision`. And nothing at all said how to obtain
llama-server, a GGUF, or a projector — the README's whole local story was the
clause "or point at a local server".

`mecha setup` is the answer to the half of that which documentation had
already failed at: the docs stated these settings correctly and the machine
stayed wrong for months, because each degrades quietly instead of failing. So
it does not ask what you meant — it reads `/props` and writes down what the
server reports. `mecha trigger daemon --print-unit` is the answer to the
other half, emitting a unit that names the binary by `current_exe` rather than
the string `mecha`, because a unit resolving against systemd's `PATH` is the
version-skew trap one layer down. That flag is also this arc's neatest
process lesson: it was *documented before it existed*, and the only reason a
commit fixing a broken instruction did not ship the identical defect is that
the next step was to check whether the command was real.

One filed defect was withdrawn, and the withdrawal is the transferable part.
`website/docs/graph/*` is synced from the mecha-graph repository at build time
and gitignored here; that repo had replaced Ollama with llama-server the day
before. What had been read was a **stale local build artifact** — one
`sync-graph-docs.mjs` corrected it. **A gitignored synced file is a cache, not
a source:** grep it and you are auditing your own last build, not the truth.
Verified good in the same pass, so nobody re-audits it: `cargo install
mecha-graph-mcp` plus the `[[mcp]]` block from `features/memory.md` brings the
`kg_*` tools up unprefixed against a fresh store with no errors.

**2026-08-21 — the remote control.** A live TUI session and a named Slack
thread became the same conversation. `/remote-control <name>` claims a durable
name, opens or re-opens its thread in the owner's DM, and tees the run's
`AgentEvent`s into `slack::pump` — which needed no change at all, because it
had been written as a standalone consumer months earlier. What the run does and
what you type appear in both places; typing in the thread steers a run in
flight or starts a turn; files move both directions, out through `/send` and
`show_file` and in through a staging directory the connector owns. The design
is `docs/REMOTE-CONTROL-DESIGN.md`, written before any of it and settled the
same day.

The shape came from one constraint: a `Conversation` — messages *and* taint —
lives in the running process's memory and the session JSONL has one writer, so
there is no symmetric design to look for. One owner, many terminals. The
connector consequently stopped answering for a mirrored thread, which it had
been doing invisibly: a reply used to mint a thread record and start a fresh
conversation in a different workspace under a different permission mode,
answering in a scrollback it knew nothing about. Never a leak — that
conversation is clean, taint and all — but a stranger wearing the thread's
clothes, which reads worse and is just as wrong to act on.

Two code reviews found twenty-two defects across the arc, four of them real
bugs in code that already had passing tests, and **two that predated the arc
entirely**: a `/model`, `/provider` or `/mcp` switch had always rebuilt the
agent from config alone, silently dropping `ask_user` and `recall`, and had
always ignored the permission mode in force (see *Containment and state*).
Both are fixed; `install_frontend_tools` and `approver_for` exist so the two
call sites cannot disagree again. `d266fe8`, `d0a1499`.

**2026-08-21 (0.1.11) — four things that read as working and were not.** A
patch release with a single shape running through it. The TUI held mouse
capture for the whole session, which made the wheel scroll and made a drag
impossible, so `/docs`'s own documented fallback — "the URL stays on screen to
be selected by hand" — had never existed for a 420-character link whose only
other route out was an OSC 52 write no terminal acknowledges; any modal now
releases the mouse and `^s` toggles selection in the main view, reconciled once
a frame from the drawn state rather than restored at each pane's exit. Two more
blocked the same picker: a paste while it was up landed in the message box
behind the modal, and the link was drawn inside a bordered box, so a drag
collected a `│` at both ends of every row. In `search.rs`, a chain whose
backends all answered and all found nothing reported `every search backend
failed`, which one model read as broken infrastructure and answered by
rewording the query eight times — an exhausted chain of empties is now an
answer, while a chain where nothing *answered* is still an error; and a SearXNG
instance with every engine suspended or CAPTCHA'd returns `results: []` at HTTP
200, byte-identical to an empty web, until `unresponsive_engines` is read. The
cache lens scored re-payment from `input_tokens` — everything *not* read from
cache, which on this workload is overwhelmingly the turn's new content — so it
shouted loudest when tool results were biggest and called the real failure
stable; it now measures what did not come back, and the field says `repaid`.
`[[search]] prefer_deep` arrived alongside: depth had changed only *how* a
backend searched, never which one ran, so a paid backend bought for hard
questions was reached only when the free one came up empty. It reorders and
never filters. 1,244 tests.

**2026-08-21 (0.1.11, the second half) — a draft you could neither read nor
approve.** Folded into the same tag, because it was one arc. Eleven drafts had
piled up and the docs edits among them could not be released: pressing `s`
raised the approval confirmation and nothing after that did anything. The
confirmation put a tainted draft's arguments on screen "in full" and drew them
with an unscrolled `Paragraph`, which renders from the top — so a
`docs_replace` whose `find` was an entire syllabus section pushed the question
and the `y` prompt off the bottom of the box, and `modal.confirm.take()` meant
every other key dismissed the confirmation, so there was no way to scroll down
to the instruction that had gone missing. The box was sized from `body.len()`
besides, which counts *logical* lines where one long argument is a single
`Line` and many rendered rows, so the height reported "it fits" in exactly the
case where it did not. The arguments scroll now, the prompt is pinned to the
bottom border where nothing can push it off, the height is measured with
`Paragraph::line_count` *after* wrapping, and scroll keys no longer count as
"anything else". The sibling detail view had done all of this correctly for
months; the confirmation was simply never given the same treatment. Renamed in
the same pass: the verb is `approve` on `a`, beside `e` edit and `r` reject,
because the queue holds more than mail and a `docs_replace` is approved rather
than *sent*. `outbox send` and `s` both still work, and the stored status
stays `"sent"` — an append-only value that `mineable_as_writing` keys on, so
renaming it would orphan every resolved item and silently stop the writing
miner. 1,246 tests.

**2026-08-21 — the web search a scraper could not win.** Every *general*
engine behind the local SearXNG instance refused the box's IP at once — brave
and google cse `Suspended: too many requests`, duckduckgo and startpage
`CAPTCHA`, mojeek `access denied` — while its specialist engines answered
fine. Enabling more engines did not help and was not going to: a self-hosted
metasearch is in a standing race against anti-bot walls, and losing it looks
exactly like a quiet web. Exa was wired as the second backend and Tavily as a
third, both already implemented and needing only a key and three lines of
TOML. Ordering was measured rather than reasoned: a quick Exa search bills
$0.007 against Tavily's $0.008, read off Exa's own `costDollars`, and the
`contents` block mecha sends costs nothing despite a price list that bills
Contents separately. Merging the paid backends *into* SearXNG was considered
and refused — SearXNG fans out in parallel to every enabled engine while the
chain stops at the first answer, so every trivial lookup would have paid for
Exa too; it would have collapsed the chain to one link whose only backend
cannot report why it failed; and it would have kept the appearance of "the
query never leaves your network" while destroying the fact.

**2026-08-22 — the self-improvement loop closed, on the owner's ruling.** The
three gaps that had kept detect→diagnose→measure→gate a set of parts — no
nightly stage, no session-corpus measurement arm, no accept path — closed in
one arc (`69ac0e9`), after Luke ruled for §13.3 auto-accept in so many words:
verified improvements apply themselves, architecture reaches him, and the
loop must be useful without adding work. `mecha harness ruminate` runs
nightly from `ruminate.sh`: diagnose one change from the run corpus, persist
it as a candidate (`~/.mecha/learning/harness/candidates/`), measure it by
counterfactual replay of recent sessions — both arms replayed whole under the
recorded config, the candidate arm differing only by the change, divergent
episodes dropped rather than scored — and dispose through `candidate::judge`.
A confirmed config win lands in an override layer applied *between* config
defaults and every file layer, so the user's own config wins by assignment
order and revert is deleting a line; the closed override set moved to
`mecha_core::harness` with `eval --ab-config` parsing through it, so the two
arms cannot drift. The brief now carries prior candidates as
do-not-re-propose history, doctor flags one staged past 72h, and the first
live run the same evening had the diagnostician correctly decline on a
healthy corpus. Deployed the same day: binary installed, services restarted,
the nightly's `ExecStart` confirmed to run the repo script.

**2026-08-22 (later) — doctor sees the starved learner.** The morning's
investigation had found the *rule* learner reporting `ok` every night for
seventeen days while producing nothing: 14 of 16 reflections excluded by the
origin gate, the clean pool stuck at 2 below the learn floor of 3 — the
null-run bug one layer up from the trigger version doctor already catches.
The classification itself was verified precise (positional, per-intervention,
`timeline.covering(at)`), and the excluded sessions had genuinely read the
open web (`http_fetch`, `research`) before their interventions, so the gate
was doing exactly its job on exactly its content. What was missing was a
reader: `check_learning` (`52a0755`) fires when ten or more exclusions have
piled up, no domain reaches the floor, and new ones keep arriving — quiet on
young installs, met floors, and dormant loops — and its finding proposes a
decision rather than a command, because the only throughput levers (steer-
text-only reflections, a custodied-source origin subclass, a lower floor) all
cross the recorded fail-closed ruling and were surveyed and left untaken. The
floor became `learning::LEARN_MIN_REFLECTIONS`, shared by `learn --min` and
the check, on the `MAX_ACTIVE_RULES_PER_DOMAIN` lesson.

**2026-08-22 (evening) — the queue could not be read, and then could be.**
The knowledge graph's merge queue had grown 1,035 → 6,434 in nineteen days and
the nightly alert printed only its depth, which reads identically whether the
accept lane admits 5% or 95%. Reading across it turned up an ordinary
structural gap and one genuinely misleading number.

The gap: the queue is a ratchet. Auto-accept requires a `(proposer, predicate)`
class on a hand-maintained allowlist or an earned ladder rung, and 91.2% of the
queue can reach neither — the ladder has `Staged → Sampled → Trusted` and *no
rung below Staged*, so a class can earn its way into autonomy and never earn
its way out of the queue. Vet's ceiling (40 × 10 predicates) also sits below
extraction's output (500–970 candidates a night), so the drain was specified
slower than the fill, against a slice that is 8.8% of the problem.

The number: `precheck::review_clusters` counted the pipeline's own dedup and
ephemeral rejections as the owner's, in the one view a person reads
*immediately before verdicting a whole class*. `llm/has` displayed 18% against
a true 67% over 48 human verdicts; `llm/has_role` 7% against 53%; three classes
displayed 0% on which nobody had ever voted. `ladder::human_record` had carried
the correct filter (`reject_reason NOT LIKE 'precheck:%'`) all along — two
queries, one filtered, one not, and the unfiltered one was the one on screen.
A first pass of the analysis reached the confident conclusion that half the
queue was demonstrably unwanted and a Wilson-upper-bound suppression rule would
clear 31.3% of it; recomputed on human verdicts alone the same rule clears
**nothing**, because there is no class the owner has judged often enough and
rejected consistently enough to condemn. The queue is not full of junk; it is
full of unknowns — 40.5% of it in 660 classes with no human verdict at all.

What shipped from it: `review --proposers` and the TUI's `p` roll the queue up
by proposing mechanism (the level a decision is actually made at — the LLM
extractor measures 59% over 1,984 verdicts, `linker:knn` 16% over 57);
`review --sample` draws uniformly at random, seeded and printed, because the
queue has an order and judging its head measures the ordering; and on mecha's
side `mecha review` plus the `/queues` modal put all five human-facing stores
in one list. The graph half of that modal shells out to `mecha-graph`, which is
a departure from "reach the graph only through MCP" taken deliberately: the
tool surface has no `kg_accept` and must not gain one, since every MCP tool
lands in the model's registry and a model that can accept candidates can accept
the ones its own extractor proposed.

Six review findings were fixed before the merge, one of them severe: the
class-level verdict passed `--proposer`/`--predicate` to a `mecha review
accept` that declared neither, so the headline "one decision worth hundreds"
could never have worked. It shipped that way because the mutating path was
never exercised — the live graph was deliberately left alone and no scratch
fork was made instead. `mecha-graph fork` turned out to be broken anyway
(768-vs-1024 vector dimensions, from the harrier embedding switch), which is
its own finding and still open.

**2026-08-23 — the queue became workable, and the learner finally learned.**
The morning's question was "how did nightly go", and the answer was: cleanly,
and with three systems quietly not working. What followed was two sessions in
one checkout (coordinated by cross-session messages; the morning one wired the
item-review `b`/`A` keys and taught a stale session to spawn children through
its own `/proc/self/exe`) and, by evening, all three fixed and a set of
surfaces that did not exist at breakfast.

The learner: nineteen nights of correct provenance exclusions had produced
zero rules ever — structural, not a bug, since any session touching mail,
docs or the web is untrusted and those are exactly the sessions where
corrections happen. The recorded fail-closed ruling was kept and the evidence
moved to its trusted side instead: `learning::evidence_for` hands the
reflector the user's own typed words plus registry-owned tool names, with
every assistant-authored byte withheld — the front door's split, except the
withheld half is never read at all — so the reflection's inputs are clean by
construction and it classifies learnable (`Evidence::UserTurns` records what
was shown). `mecha reflect --remine-untrusted` re-mined the archive: 11
lessons recovered, `learn` staged the store's **first-ever proposal** (5
rules from 10 reflections), and doctor's starved-learner finding cleared the
same afternoon.

The ladder: promotion fired only inside `note_verdict` — at the instant a
human filed a verdict, and at no other moment — so classes whose strong
records predate the 2026-08-16 Wilson switch sat at `staged` indefinitely,
invisible because no CLI printed a rung. `mecha-graph ladder` shows every
class (union of ledger rows and verdict history, since pre-ladder classes
have no row), and `ladder --promote` applies a one-rung-per-pass recompute —
never demoting, exactly `note_verdict`'s arithmetic minus the verdict. Run
live it promoted `works_on` to trusted and `member_of`/`located_in` to
sampled, and with the day's review work the queue fell 7,296 → 6,569.

Under it, provenance the cascade made unaffordable to skip: an accepted
candidate row carried no record of who accepted it, so machine accepts —
the durable lane's, a cascade's — counted toward the human rate that
promotes classes, the accept-side twin of the 2026-08-22 cluster-view
contamination, running in the direction that widens autonomy. V017 adds
`fact_candidate.reviewed_by` ('user' / 'auto' / 'cascade:<seed>'), and
`HUMAN_VERDICT_SQL` is the one predicate — ladder promotion and the
cluster/proposer views share it, legacy NULLs keep counting as they did
(rewriting history would gut the record the ladder runs on), and every row
since is exact.

On top of that, the queue's repetition became reviewable as repetition:
`similar.rs` groups a class's pending candidates by semantic similarity
(deterministic leader clustering at precheck's own flag threshold), the
listing covers the whole class — groups largest-first, singletons after,
since a view that hides most of the work is a view people leave — and one
keystroke verdicts a whole group as **one human verdict**: the seed is the
owner's, the members cascade machine-labeled, and the fan-out lands on the
explicit ids that were on screen (`--cascade`, vetted per-id against the
seed's class), never a re-derivation of a queue that may have moved. `b`
binds a group's unresolvable subject, `A` accepts creating it (the cascade
then resolves against the node the seed just made), and `[`/`]` re-group at
a threshold stepped from the value the child reports it ran at. The first
live grouping justified the feature by itself: bee's largest group was
fifteen hallucinated family members.

The TUI stopped freezing while children work: `/mail`'s archive/spam/task
ran an MCP server plus a network call inline per keystroke (the code's own
comment admitted it), and grouping embeds a whole class — both now spawn on
threads and land through watches. The graph reached the person at the
keyboard: `mecha kg search|entity|note` over the same `kg_*` MCP surface the
model uses, `/find` as the search modal (entities open their full record,
facts and episodes open in place), `/note` as deterministic capture with
entities linked on landing. Slack gained `note` and `queues` as owner-gated
command words on the doctor pattern — a capture matched before the text can
become a prompt, because a capture that depends on a model's mood is not a
capture, and a read-only backlog rollup whose verdict buttons are
deliberately a future design pass. The morning trigger's daily
subagent-skip warning went quiet under a trigger's own durable allowlist,
on the outbox warning's own reasoning, and one bug was shipped and fixed
within the hour — the NULL predicate, recorded under Traps.

**2026-08-24 — the phone became a terminal, and mecha learned to talk.** Two
arcs, three sessions coordinating over the inter-session mailbox, one
production system by the end of it. The **voice arc** went from
`docs/VOICE-RESEARCH.md` to a running stack in a day: three speech services
(llama-served STT, Chatterbox TTS with Kokoro beside it, the Pipecat worker),
the loopback OpenAI facade over the shared agent (`mecha-cli/src/voice/`),
D5 ratified (owner speech is typed text, no taint), and the whole thing
systemd-managed and reboot-proof — its build log is `VOICE-RESEARCH.md` §7.
The **remote-surface arc** asked whether Slack was still the right remote
control and answered by building the alternative: `REMOTE-SURFACE-RESEARCH.md`
(the field's convergence on local-loop/remote-view/approval-as-the-verb;
Telegram as the only credible Slack successor; the tailnet web app as the
recommendation), `REMOTE-SURFACE-DESIGN.md` (one process, identity from the
network, verbs as CLI children), brand-held phone mockups, and then `mecha
serve` itself: a `[web]` config section stripped from project layers like
`[slack]`, an owner-login guard on every request (the header `tailscale
serve` injects), a read-only dashboard over `review queues --json` and
`doctor --json`, streaming chat over SSE with steering and cancel on the
Slack connector's one-agent/many-conversations pattern, the outbox as a
phone review surface rendering the whole reviewable object (`DraftView`,
source reads behind an amber gutter, taint sheet with the exact bytes,
`outbox edit --body-file` so the no-terminal edit shares the one
implementation), the graph queue's sample deck (seed printed, verdicts
one at a time), tasks and notes over `tasks`/`kg` verbs, and a keyed
session rail. The two arcs then **unified**: the voice facade mounted
inside `mecha serve` (one agent, one cached prefix, two dialects), the
page's WebRTC offer proxied same-origin behind the owner guard, voice-core
embedded in the chat page as an in-call overlay with a live transcript
thread and a cloned-track mic meter, and production flipped to one
systemd-managed process serving web and voice both. The day's last build
put a **present human** on the page: per-session `ask` mode turning tool
calls into live approval cards (allow, or deny-with-reason mined as a real
correction; timeout is `Blocked`, machine policy), `ask_user` routed to the
session that owns the calling run's jail through a new `Asker::ask_in`
context seam, cancel draining parked cards, and pending cards riding the
transcript read so a locked phone reloads into its questions instead of a
silently parked run. The voice arc's closing act was structural: **Voxtral
answered speech instead of transcribing it** — question-shaped audio came
back as first-person answers, spoken instructions were obeyed — so Parakeet
took the STT seat, probes exact at 92ms, and the injection probe now
transcribes as faithful words obeyed by nobody. An orphaned Slack
tasks/board WIP from an earlier session was adopted, reviewed, and landed
the same night. The `update` skill gained the web assets as surface 1b.

**2026-08-25 (afternoon) — three things the phone could not do, reported
from the phone.** All three were found by using it, and the pattern under
two of them was the same: **a surface reporting success it had not
verified.**

*Notes could not be opened or edited.* The page listed them and stopped
there, and the reason was a missing key rather than a missing button:
`kg_notes` returned each episode's `uid`, which names the row and cannot
write to it — the graph's episode key is `(source, source_id)`, and
re-upserting under that is an update. Without it the best any surface could
do was capture a second note beside the first. The graph now returns
`source_id` too, `mecha kg note --edit <source_id>` is the verb, and the page
drives it. The trap was the *other* field: `upsert_episode` writes every
field it is handed and `occurred_at` defaults to **now**, so an edit that did
not carry it would move the note to today — a notebook rewriting when things
happened because somebody fixed a typo. It is read back from the same
listing the id came from, so no surface can get it wrong on its own.

*A graph verdict could fail with nowhere to go.* Accepting a similarity group
failed with `cannot resolve subject 'X'` and the page printed it at the top
of the screen with nothing to do about it — while both ways through had
existed in the TUI since the modal was written (`b` binds the subject, `A`
accepts it as a new topic). They are on the card that failed now, offered
only after a failure, because `--create-subjects` invents a topic node.
Underneath was the invisible half: **`mecha-graph accept <id>` reports a
per-candidate failure on stdout and exits zero.** Right for a bulk run where
one of five hundred cannot resolve; a lie for a page that keyed on the exit
code, dropped the card it had just sent, and counted it as one of the twelve
verdicts the sitting claims to describe — while the candidate sat pending.
The child's report is tallied through `review::tally_report` now, the same
function the TUI reads.

*Calls ended by themselves*, for two independent reasons and neither was
network flakiness in the way it looked. The common one is a default nobody
chose: **pipecat cancels an idle pipeline — and the runner with it — after
300 seconds**, where idle means neither side produced a speaking frame. Five
minutes of silence is a conversational pause on a phone, and the log names
it outright (`11:36:23 connected` → `11:41:50 Idle timeout detected`). The
timeout is kept, because an abandoned tab otherwise holds VAD, turn
detection, STT and TTS open on a box with one GPU, but it is raised past any
pause that is still a conversation and it now *announces itself* over the
data channel. The rarer one was ours: `voice-core.js` treated ICE
`disconnected` as loss, when it is the browser reporting that packets have
stopped arriving *for now*. Only `failed` and `closed` are terminal now.
Deliberately no ICE restart to shorten the grace window — pipecat's
`restart_pc` fires the very `disconnected` event this worker cancels the
pipeline on, so reconnecting would destroy the bot being reconnected to.

**2026-08-25 (evening) — a draft you can say yes to, including out loud.**
`ReviewMode::Now` — *a draft you just asked for is a draft you are about to
read* — had been the default in the TUI and Slack since the policy was
written, and `mecha serve` was the one surface with no release policy at all:
every staged draft went silently to the outbox and the badge. On the page it
is now a card, built from `/api/outbox/{id}` rather than from the event that
announced it — ids on the wire, bytes from the store, because a reviewer
reading one thing while approving another is the failure the outbox exists to
prevent.

In a call the same offer has to be spoken and answered aloud, and that is a
different problem, because the answer arrives as text in the model's own
medium. **The harness asks and the harness hears**: the offer is composed
from the store through `DraftView::spoken`, the reply is matched by
`review_policy::parse_answer` before the request reaches a model, and the
release decision never enters a context window at any point. It is
`mecha review`'s oldest rule in new clothes — the graph's tool surface has no
`kg_accept` because a model that can accept candidates can accept the ones
its own extractor proposed, and a model that could release drafts could
release the ones an injection wrote.

Four rules carry it. The match is **whole-utterance, never substring**, and
the failure direction is the argument: "yes" is an answer, "yes but change
the time first" is not, and reaches the model as ordinary words with nothing
released — verified live before it landed. An unanswered offer is *dropped*
rather than held, or every later "yes" in the call lands on a forgotten
draft. The draft is **uttered whole or not offered**, because a listener
cannot skim back over the line where the extra recipient was, so a spoken
paraphrase is not a smaller review but a different document missing exactly
the field an injection would add; under 400 characters it is read out entire,
over it the choice of hearing it is the owner's, and a publish is never
offered by ear at all because its reviewable object is a rendered page.
Taint does not block — it is *spoken*, since the listener is the one person
who cannot re-read the addressing line. And **nothing spoken can discard a
draft**: rejecting takes a reason the learning miner reads, so "no" parks it
in the outbox where it already was, and the safe answer to every ambiguity is
the same one.

Two bugs the tests caught before a person would have. A datetime with no
offset read out as `2026-08-28T16:00:00`, so timestamps are spoken as dates
and times **in the offset the string itself carries** — converting one would
be the wrong-bytes review arriving through the ear. And a reply promised
"say next to hear it", a word `parse_answer` does not know: every listener
who said it would have been answered by the model while the draft sat there.
No surface may offer a verb the policy cannot recognise, so the follow-on is
a whole question. Proven end to end on a real calendar event, which was
created by a spoken "yes" and then deleted.

**2026-08-26 — the board learned to delegate.** Phases 1–4a of
`docs/TASK-AGENT-DESIGN.md`, plus the graph change underneath them. `mecha
tasks work <id>` turns a board item into a seeded run in its own session —
outbox-bound, its status moved by the harness, and unable to close its own
task because `kg_task_update` leaves the model's surface and stays with the
harness (`setup::withhold_tool`, which hands the tool back so one Arc serves
D5 and D6 at once). A run that needs a decision **ends** rather than waiting:
`mecha_core::questions` is the outbox's inbound twin, the run cancels its own
token, and `mecha questions answer` resumes the session with the owner's
words as the next turn — so nobody holds one of four llama-server slots
overnight for an answer that comes at breakfast. `/queues` gained a sixth row
and doctor watches it at 24h, shorter than the outbox's 48h because a pending
draft is finished work sitting safely while an unanswered question is a
delegation frozen mid-flight.

The graph side made the board able to say *who* has a task: `waiting_on` and
`session` on `kg_task_update`, an `agent` node kind — deliberately not a
person, because delegation is not assignment and a person node would answer
"who is responsible" with the wrong kind of thing — and `@owner`, so a
harness never carries the owner's name. D9 was **reversed while building
it**: the design proposed a `worked_on` fact, and an episode turned out to
exist only for the runs the distiller judged worth remembering, which are
precisely not the runs a person needs help finding. The session id is a task
attribute instead, and is strictly the more general of the two — the
episode's idempotence key *is* the session id.

A second review at high effort over phases 3–4 found ten more, and the first
would have shipped a feature that works once: **the D11 guard keyed on
`status == "waiting"`, which every finished run leaves behind**, so the
second *ask mecha* on any task bailed — detached from the web it exited 1
into `/dev/null` while the page had already said "handed to mecha". The
guard's own comment named the gap ("without `waiting_on` … this cannot tell
the agent from a person") and phase 3 had closed it hours earlier. Also
fixed: the resume door moved the board to the agent and never moved it back
on failure, and wrote no run marker while rendering as running — so *stop*
found nothing and the card kept pulsing; "nothing to stop" was returned to
the page as success, defeating the exact confusion its own comment claimed
to prevent; a stale `.cancel` could stop the *next* run two seconds in; and
every task run shared one `TodoTool` key, so the card showed another task's
plan the moment it began rendering one.

A second lane shipped **task provenance** the same afternoon: a task now
carries a pointer at what asked for it, so a person deciding "done or do
next" can read the original instead of searching their own inbox for a
subject line they half-remember. `captured_from` on the task node
(`gtd.rs:273`, `CAPTURE_KINDS` at `:234`), a **closed kind set of
mail | frontdoor | session** — deliberately not slack, because nothing on
this side can render a Slack thread and a kind with no reader is a button
that opens nothing — and unknown keys refused, which is what structurally
keeps it a pointer rather than a copy of an email body. `mail task` writes
it at capture; `mecha tasks source <id>` (`tasks.rs:401`) follows it with
one reader per kind; `POST /api/tasks/source` and a `read the …` control on
the card; the TUI's `o` deliberately off the key strip, because it is inert
on any task somebody typed and a legend advertising a dead key is the
dead-affordance problem arriving through the legend.

The two arcs interleaved in `commands/tasks.rs` and `Tasks.svelte`, which
is its own entry under Traps.

On the web surface: *ask mecha*, *stop*, *open the conversation*, an agent
chip derived from the board rather than self-reported, `task:` sessions in
the chat drawer, and the plan rendered in both places you watch a run —
reading the live tool in chat and the transcript on the card, because a
`tasks work` run is a separate process and its list is not in `serve`'s
memory. `TodoTool` was keyed per run and taught to rehydrate from a
transcript on the way (D14/D15), which is what made the card possible at all.
Two code reviews at high effort ran against it; the first found ten things,
including delegation as a way around D6.

**2026-08-26 (second pass) — the delegation loop closed at the phone end.**
Phase 4 could *start* a delegation and could not finish one: *ask mecha* had
been on the task row since that morning, a run that needed a decision ended
and stored its question (D13), and the only surface that could answer was a
terminal. So the gesture the phone exists for opened a loop the phone could
not close — and the board sat in `waiting` with nobody able to see why from
the device it was read on. `GET /api/questions` is a direct `QuestionStore`
read on `review.rs`'s pattern (mecha's own store, unlike the board, which
must go through the CLI because its store belongs to another repository);
answering spawns `mecha questions answer --unattended` **detached**, because
answering *is* a whole agent run; abandoning is synchronous, because writing
one record is instant. The card lands on its task rather than on a new page,
which is D13's own argument — the Waiting view *becomes* the queue of blocked
delegations, with no new noun — and a question whose task is off the board
still gets a card, because a question nothing renders is a delegation frozen
forever.

Two things that were not on anyone's list came out of building it, and both
were the same shape: a rule this project states everywhere, quietly untrue in
one place. **`--unattended` on the resume is load-bearing, not ergonomic** —
`answer_and_resume` built an *interactive* agent, so a detached child would
have installed `TerminalApprover`, read `/dev/null`, taken EOF as a refusal
and filed every one as `Decision::Deny("the user declined this call")`, which
the loop renders `"Denied by the user: "`: the exact string the learning
miner reads a **correction** out of. A question answered from a phone would
have taught mecha rules from a person who was never asked. And **no task run
had ever written a `RunStats`**: `record_outcome` had ten call sites and
neither `tasks work` nor `questions answer` was among them, so the corpus
CLAUDE.md describes as written "once per finished run by every front-end" had
never seen a delegation.

That second one is what made D16 buildable honestly rather than by guessing.
The card's state is derived from three sources and none of them is the run's
account of itself: the board says who holds the ball, the question store says
whether it is blocked, the transcript's outcome record says how the last run
stopped. Seven states, no two rendering alike, and the seventh is the
interesting one — `outcome unknown`, for a transcript with no outcome record,
which is a run that never got as far as saying how it went (a crash, a kill,
or a session written before the record existed). Folding that into `failed`
would have made every delegation from before this shipped shout that it
broke; folding it into `ready` would have made one that really died read as
finished, which is the exact rule D16 states outright. `Interrupted` reads as
ready and never as failed, on doctor's own rule for the same field: a person
stopping a run is the system working.

One unrelated bug fell out of looking: **`/api/tasks` never passed
`--closed`**, so the drawer's `done` view — which filters on
`done | dropped` — had been filtering a list that could contain neither since
the day it shipped. It read as "you have finished nothing", which is the
failure mode a filter that cannot match always takes.

Verified by driving the real page in a headless browser rather than by
reading it: the answer button disabled while empty and enabled on the first
keystroke, an option tap posting the option's own words, abandon posting the
id, and *open the conversation* landing on `#chat/<session>`.

**2026-08-26 (third pass) — the plan gate was decided against, and the seed
got the cheap half.** D12 proposed stopping a delegated run after its first
`todo` write to take the owner's edits. The owner's question — *"how is this
different from todos? should a human ever edit or delete or add a todo? we
already have an ask question system"* — is the defect: the gate made the
**todo list** the human-editable object, and every system that has solved
this keeps the reviewable plan and the agent's execution ledger apart
(superpowers writes a plan document and then *"create todos for the plan
items"*; Claude Code's plan mode writes `~/.claude/plans/` while `TodoWrite`
is separate). `todo.rs` had forbidden the collapse in its own module doc
since it was written — *"a list set by anything other than the model's own
`todo` write … is a second author of state the tool is supposed to own"* —
and nobody had noticed that this was the conflict, because D12 answers a
*different* `todo.rs` objection (the stale plan *phase*) convincingly enough
to look like it had answered them all.

Two things then decided it, both already in the repository. **The project's
own research argues against plan-first on this hardware**:
`VERIFICATION-RESEARCH.md` has FORGE 2026 finding straight-shot often equals
or beats Plan-and-Execute, small models *collapsing* under it (Llama 3.2 3B,
0.23 → 0.05), and a bad plan measuring worse than no plan. And **the gate's
trigger rested on a behaviour measured absent**: the 2026-08-04 probe found
this model called `todo` zero times in 20 eval case-runs from prompting, and
keeps a list reliably only when the *user turn* asks — so the gate would have
fired when the model felt like letting it. D12's own cited evidence turned
out to point elsewhere too: Copilot's 38.1% → 69% came *"purely on tuning
`copilot-instructions.md`"*, which is the **seed**, not a checkpoint.

So `work_prompt` now front-loads: work out what you need and ask it first, in
one `ask_user` call covering everything, with an explicit guard against the
opposite failure — *"do not ask what you can find out"* — because a prompt
that only says "ask first" produces a run that asks instead of working. On
the user turn, deliberately, since that is the one delivery channel the probe
found this model obeys, and the tool's own "in one sentence" schema is
*overridden here* rather than widened for everyone.

One gap the change made likelier and therefore worth closing with it:
**several `ask_user` calls in one turn all park**, and both surfaces rendered
only the first — `parked().first()` in the CLI, `find` on the card — so the
rest were reachable only from a verb nobody runs after being told what to do
next. Both now render every one, and both say the surprising part out loud:
answering *any* of them resumes the run, and the others stay open.

What none of this reaches is the failure D12 was actually after —
misalignment the model does not notice, where a confidently wrong plan asks
nothing. That is now **countable** rather than arguable, because task runs
record `RunStats` as of this morning: delegations that ended `ready for
review` and were then dropped or reworked rather than marked done. The
reviewable-document version gets built when that number argues for it, and
gets built as a document separate from the todos.

**2026-08-26 (fourth pass) — the graph queue was asking for verdicts it
could not show, on candidates it could not accept.** Four bugs, found by the
owner reviewing on a phone, and they compound in that order.

The card said **`undefined — undefined — undefined`**. `Queue.svelte` spelled
its fallback ``payload.statement ?? `${subject} — ${predicate} — ${object}` ``
— and a template literal is never nullish, so `??` stops at it. A commitment
payload carries `{who, what, when, direction}` and no s/p/o at all, so all
**695** `llm:commitment` cards asked for a verdict on a belief nobody could
read. The correct chain existed twice in Rust with a test named on the
commitment case (`tui/queues.rs`, `items_from_json`); the page was the third
reader and re-derived it wrong. One `faceOf()` now, matched to the Rust rather
than improved on.

Underneath it, **`linker:knn` was writing node ids into `subject` and
`object`, which are names.** `ProposedFact.subject` is what `accept_candidate`
hands to `resolve_entity` — canonical name, alias, identifier, fuzzy — with no
tier that reads `nodes.id`, and it is also the `kg_upsert` wire format. The
linker looked both names up for its statement and then stored the ids
(`linkers.rs`), so **every candidate it staged was unacceptable**, `bind` could
never suggest anything (`suggest_entities` matches names; a uuid is not a
misspelling), and `accept --create-subjects` — one of the two ways the card
offered through the failure — minted topic nodes whose *display name* was
another node's id.

That last part is what made it worse than a stuck queue. **Thirty** such
placeholders were in the live graph, and once one exists the id *resolves* —
to the placeholder — so the next candidate carrying it accepts cleanly and
asserts a belief about a node standing for nothing. Candidate #16644 no longer
failed; it answered `already resolves — nothing to bind`. A queue item that
fails loudly is a bug; one that succeeds into a fiction is the shape this cost
an afternoon to see. `repair-id-payloads` merged the 30 placeholders into the
nodes they were named after, rewrote **121 of 8,988** pending payloads to
names, and re-pointed **23 accepted facts** at real entities; idempotent on a
second run. Applied to the live store the same day.

**The bind target existed on the server and on no surface.** `BindBody.to` had
always been there and `mecha review bind --to` too, but neither the web nor the
TUI could send one — so the card displayed the graph's own instruction, *name a
target with `--to`*, and could not carry it out. The phone gets a field after a
failed bind (never after a failed accept, where naming a target is not the
answer); the TUI gets `B`, and a failed `b` opens the same prompt. The prompt
owns the keyboard while it is up, which it has to: `a`, `r` and `d` are verdict
keys, so a target named "Dana" typed into a live list would have filed three.

Last, **`--ids` was bounded by `--top`, which defaults to 10** — so a named id
set was silently trimmed. Found by the other session's review of a smaller bug
in the same code (a progress counter reading the *requested* count rather than
the returned rows) and worse than what it was reviewing: the TUI's group dive
has shown at most ten members of any group since that level shipped. Enter on
a group of seventeen, get ten, nothing says so. `mecha review items` now asks
for exactly as many as it names.

What made all of this reviewable in the first place is the fifth change:
**a group can be opened and its members verdicted one at a time** on the web
(`GET /api/queue/items`, "Review each of the N"), which the TUI has had and
the phone had not. The case for it is one real group — seventeen near-repeats
naming Emmy, Sage, Katie, Joseph, Eni, Justin and Jesse as the owner's
children, mostly Bee mishearing two names. Similarity is the grouping key, not
agreement, so "Accept all 17" would have asserted every one of them. A verdict
inside a group is deliberately plain — no cascade — because telling the
members apart is the reason for being in there.

**2026-08-26 (evening) — B1 and B2, after checking whether they still
described the row.** Both board decisions predated phase 4, and re-reading
them against the shipped implementation changed both; the originals stay in
`TASK-AGENT-DESIGN.md` with amendments beside them, the way D12 does.

**B1 was right about its evidence and wrong about its translation.** §1.3's
invariant is *"complete and schedule are one gesture; move, **delegate** and
edit open a sheet"* — delegation is on the sheet list explicitly, so *ask
mecha* belongs behind the tap and a later reading that promotes it because
it is this project's differentiator is re-deciding a settled question on
taste. What did not survive: §1.3 surveys **swipe** actions on a *collapsed*
row, and this row has no actions until it is tapped open. The six chips the
owner called *"a little bizarre"* were never in anyone's way, and the
expanded card already **is** the sheet — a `…` inside it nests a sheet in a
sheet. Their problem was shape, not depth. `✓` moved to the collapsed row
(`Tasks.svelte:722`), because in Things complete is *"a tap on the circle
only"* and this board had no circle, so its most frequent action cost an
expand; what remains is grouped under two labels (`:550`).

That amendment made a long-standing HTML bug obvious rather than fiddly.
The card was a `<button>` wrapping everything, so every control in the strip
was interactive content nested in one — invalid, browsers disagree,
assistive tech cannot traverse it — and *three separate comments in the file
existed to explain choosing a `<button>` over an `<a>` to make the nesting
less bad*. "The expanded card is the sheet" means its contents are siblings
of the header, not children: the card became a `<div>` with a `cardhead`
button (`:739`). Six compiler warnings, five of them predating the branch,
went to zero. **A design correction can retire a workaround that had been
mistaken for a constraint.**

**B2's worked example turned out to be unrepresentable.** It promised *"call
Bob tomorrow at 3"* would set a date; `gtd::parse_due` accepts exactly
`today | tomorrow | +Nd | YYYY-MM-DD` and `due_at` is written `%Y-%m-%d`.
**The board has no time of day.** So the chip reads `tomorrow` and *"at 3"*
stays in the name — consistent with the Things side of the disagreement B2
had already picked. What B2 never said is the actual win: **capture
collapsed from three fields to one**, which is §2.1's own invariant
(organization is deferred at capture time) that the sheet had been quietly
breaking. And it is worth more than when written, because dictation landed
in between: a spoken capture arrives as one string with no second field to
fill, so without the parse the microphone could only ever produce an undated
inbox item.

`capture::find_when` (`mecha-core/src/capture.rs:61`) **detects; it does not
resolve** — the span goes to the graph's `parse_due` as `--due`, so one date
parser lives in the repo that owns what `+3d` means. Weekdays are
deliberately undetected: `parse_due` cannot take one, so honouring a
"friday" would mean resolving dates locally and detecting it without
honouring it would draw a chip that lies. Served in-process by
`POST /api/tasks/parse` (`serve/board.rs:409`) — the one handler on that
page that spawns nothing, because a child process per keystroke would pay a
fork and an MCP startup to answer a question with no state in it.

**2026-08-25 (night) — real people out of a public repository.** The
2026-08-07 history rewrite stripped *operational inventory* and did not touch
a second kind that kept accumulating afterwards: real people used as
convenient fixture data. Sixty-six replacements across fifteen files — two of
the owner's children by name, a spouse in three statements about their
marriage, four colleagues (two with working addresses), an old personal
address, a real Slack id, and the tailnet hostname of the machine all of it
runs on. Sites were three TUI modules, four core modules, twelve
`results/*.json` benchmark artifacts, and — worst — `website/docs/features/
queues.md`, which is *published*, and demonstrated similarity grouping using
the marriage statements. The feature's own screenshot was the private data it
exists to help review.

None of it was load-bearing: a fixture asserts a shape, and a shape does not
care whose name is in it. They were there because they were the nearest real
data to hand while building against a live personal graph, which is exactly
the pressure that will produce the next one. The tailnet host got a different
fix, because `--allowed-origins` is load-bearing security config: the tracked
unit ships a placeholder and the real values moved to gitignored
`OPERATIONS.md`, and an unedited copy fails in the safe direction — no
matching origin means the worker refuses every offer, where the wrong way to
be wrong is a value that parses as permissive. **Forward-only, by the owner's
ruling**: the names remain in git history and in published crates.io
tarballs, which a force-push over a public repo would not reach in forks and
clones anyway, and published crates can only be yanked rather than edited.
The only actively served copy was the docs site, redeployed and verified.

**2026-08-26 (fifth pass) — a goal system, and the first signal that says
something went well.** `docs/GOAL-SYSTEM-DESIGN.md` and rungs 0–3 of its
phasing, in five PRs (#61–#65). The finding behind the whole arc is one a
`grep` makes: **every evaluative signal mecha had was a cost or a
correction.** `learning::Trigger` is four ways of saying a person stepped in;
every `candidate::Metric` is phrased so lower is better, deliberately, so a
comparison cannot invert silently. A run could therefore be recorded as having
gone badly and never as having gone well, and nothing could start a learning
loop unless the world acted first — `grep -i goal` over `mecha-core` returned
four hits before this, all incidental prose.

Rung 0 was a consolidation the arc created the need for. **Nine call sites
established the same safety property by hand** — no tools, no conversation,
nothing an injected instruction can reach — each spelling out `tools:
Vec::new()` beside a one-element `messages` vector, two of them visibly copies
of a third. `quarantine.rs` puts the property in the type: no field for tools,
no way to add a second message, and `ask()` as the only exit. The ninth site
was found by review, in `mecha-cli`, after a sweep that had grepped
`mecha-core` and stopped — which is the argument for the module restated at
its author's expense, and the doc says so. `Agent::final_answer` was
deliberately *not* migrated: it is tool-less for an unrelated reason and sends
the whole conversation, so **the distinction is history, not tools**. The same
PR replaced `ProbePrep`'s `steer: bool` with `ProbeKind`, because grading read
*not a steer* as *a denial* by assumption — undoing, one struct field later,
the care `locate_denial` takes explicitly.

Rung 1 is `GoalRef`, and building it corrected the design twice. The goal
belongs to the **plan**, not to each `TodoItem`: D14 keys `TodoTool` by the
run's workspace and `b877e41` gave each task run its own, which together give
one list, one task. Both are cited because half the footing was hours old.
The first draft cited D11 — a one-writer rule about two runs racing — which
does not say a run serves one task; **the wrong citation is kept on the record
beside the right one**, because a wrong citation is load-bearing in the same
way a right one is. Parsing landed with two policies: strict toward the model,
which can fix it, and lenient toward a record, which is append-only and may
have been written by a newer binary.

Rung 2 read a signal the outbox had recorded since the day it existed.
`args_before` sits beside `args`, so `sent && !edited()` — the owner read a
letter written in their name and sent it as drafted — needed no new recording,
and `mineable_as_writing`'s own docstring had been naming the gap for months.
`WritingOutcome` reads it; `SentUnchanged` is deliberately **not** mined as a
correction, which is the `"Blocked by a hook:"` rule in its positive form. It
is also immune to reward hacking by construction rather than by policy:
nothing the model does can produce one except drafting something a person then
chose to send unaltered. Its rate is `None` over an empty denominator, never
zero — the null-run bug arriving in the one measure whose job is to say
something went well.

Rung 3 records the conditions a run happened under, read-only. `backlog.rs` is
one walk over five stores for three readers' questions; `Homeostat` lands on
`RunStats`, opt-in on `cancel`'s precedent so `eval` and the replay probes stay
unsampled — **a scorecard that varies with how busy the box was is not a
scorecard**. Two things were left deliberately absent rather than stubbed:
context pressure, because `RunOutcome::usage` is the run's *total* and would
read as impossible pressure, and `/slots`, because a sensor with no consumer
should not put an HTTP call in every run's start. And the backlog is recorded
as a **delta**, not a level: a run that stages nine drafts raises the outbox by
nine, so a level at run end cannot separate a run's own output from what it
inherited.

**2026-08-27 — admission control, and five findings out of the arc that
prompted it.** R1 shipped on the number R3 measured rather than the one it
was proposed with (`permit.rs`): three background seats against the server's
four, held as files on `runmarker`'s rules because a delegation is now either
a chat session inside `mecha serve` or a detached child, sharing nothing but
the filesystem — so `batch.rs`'s in-process bound, which the design called
for, would bound each process separately and none together. **Interactive
work takes no permit at all**: the reserve is an absence rather than a
control, because a pool that could refuse the person at the keyboard is a
mechanism failing closed against the only user it exists for.

**And a review of the context-pressure arc (#78) found eight things, of which
five were fixed the same day.** Two were the same failure in different
places: a subagent got the `compact` tool and its channel but not the
threshold — the channel rides on `ToolCtx`, which the child clones wholesale,
while the threshold rides on the `Agent`, which the child rebuilds field by
field, so only one made the trip — and the loop took the request with a
destructive `swap` *before* the guard that decides whether it can be served,
so the one path that cannot honour a request was the one consuming it. Both
told the model a summary would happen and produced none.

The third is the one with the longest reach: **`mecha eval`'s tool surface
had come to depend on local config.** `compact` is registered from whether
this machine's settings give the run a compaction threshold, and the tool
list is the front of the cached prefix — so two differently-configured boxes
graded different prefixes and neither scorecard recorded which. The list of
things eval forces off had been written in prose across forty lines, each
entry added when it occurred to somebody, and one was missed; it is now
`force_reproducible`, one function with a test over the whole set, so the
next addition to the tool surface has to be decided about rather than
remembered.

Also: `compact`'s description promised `recall`, which only the
session-recording front-ends register, so on a trigger or a Slack thread it
both wasted a turn and asserted the reversibility that justifies the tool
being unapproved to exactly the runs where it is false; and `Forecast::used`
documented a total it does not compute — it excludes the results of the turn
being executed and cannot include them, being an argument to the call that
produces them. Left understated rather than padded to an upper bound, since
trading a small understood undercount for a large invented one is the wrong
direction on a number the model plans against.

**2026-08-26 (eighth pass) — delegation became a conversation, which is what
D2 said it was.** The owner tapped *ask mecha*, the card moved to `waiting`
and vanished out of the view it was tapped in, and nothing happened that
could be talked to: *"there is no chat UI or any way for me to communicate
with the agent."* The card was telling the truth — `planning` is what
`stateOf` derives while a run is in flight — but the web path spawned a
**detached unattended child**, so the only conversation on offer was the one
you could read afterwards. D2 has said the opposite since the design was
written: *"the run is a conversation from the start, not a fire-and-forget
job."*

So the tap opens the task's conversation as an ordinary chat session, and
everything asked for arrives with it because it *is* the chat surface —
voice, uploads, the live todo panel, approval cards, and typing at a run in
flight being the steering the loop already understands. The model speaks
first (*"it doesn't make suggestions or ask me anything"* was the complaint
under the complaint), and nothing on the board moves, because `waiting_on`
names who has the ball and while the owner is in the conversation they do.

**Three things had to become true for that to be safe, and each is a rule
that had only ever been enforced one way.**

D6 — *the agent may not close its own task* — worked by a spawned child
taking `kg_task_update` off its **own private registry**, and a web process
holds one `Arc<Agent>` for every session. So the narrowing moved onto the
run: `RunContext::withheld`, a denylist beside the skill allowlist, checked
at the dispatch seam and landing on the same `Blocked by policy` refusal
(never an environment error — the counters read that rate), inherited by
subagents the way hooks and the outbox route are, and matched through a
server prefix so `prefix_tools` cannot switch it off silently. A resumed task
transcript keeps it, because D6 is a property of the conversation rather than
of how it was opened.

*One conversation, one writer* was enforced **within** a process and nowhere
else, which was fine while every resume-capable surface owned its own runs.
A detached child broke it: resuming a delegation mid-flight would have put
two writers on one JSONL. The run marker now names the session it is writing,
`live_writer_of` asks that question of the other processes, and both doors —
`/api/resume` and `mecha chat --resume` — answer with the task named. A guard
on one door is a UI condition.

And *the map is a cache; the transcript is the record*. A task conversation
lives in serve's memory and in a JSONL, and only the second survives a
restart — so re-opening after one minted a blank conversation under the same
key, losing the thread, the header and the withholding together. The board
had held the link since the conversation opened; nothing read it back.

**Hand-over is a transfer of the single writer, not a copy.** serve releases
the session, the child loads the same transcript — messages *and* recorded
taint — and continues in it; the turn that starts it says only what changed,
that the owner has gone, because the plan is already above it and restating
it would replace what was agreed with a paraphrase of it. That one shipped
**broken and looking healthy**: `--resume` was parsed, passed, and never
used, so the child opened a new transcript whose first line was *"carry on
from what you have both agreed above"* with nothing above it. The board
moved, the run started, exit 0. Only asking which transcript *grew* found it.

**A question with nobody there now parks.** The obvious reading of
"interactive when the page is open, autonomous when it is closed" is a switch
on whether anyone is connected, and that gets the one case that matters
wrong: a backgrounded phone stays connected, so the card is shown to an empty
room and expires into a refusal. So the card is offered whenever anyone might
see it and *both* ways of going unanswered end the same — stored question,
ended run, no slot and no cached prefix held. Waiting indefinitely costs
nothing because nothing is left waiting.

**And delegations got their own turn ceiling, after the two limits turned out
not to compose.** `cx.budget.max_turns.unwrap_or(cfg.max_turns)` is an
*override*, not a minimum, so a task run inherited whichever surface it
started from — 12 from `[agent] max_turns` on this machine, or a hardcoded 40
in the web chat, neither chosen for autonomous work. Twelve tool round-trips
is an errand; a real delegation stopped mid-way reporting `MaxTurns`, which
reads to the owner as the model giving up. 200 now (Terminal-Bench's figure),
on the argument that a turn ceiling is a backstop rather than a policy: the
loop guard, the token budget and compaction are what stop a runaway run, so
the ceiling should only ever stop an honest one.

Verified live end to end rather than asserted: a conversation opened on a real
board task ran 8 turns and 13 tool calls unprompted — graph, web, two mail
threads, the `research` subagent — and came back with a proposal and its
questions; a restart of `mecha-serve` then returned the same session with 15
entries and the header intact; and a hand-over took the transcript from 19
records to 38 in one file, carrying `private + untrusted` across the change of
hands, staging a draft rather than sending it, and returning the board to the
owner.

**2026-08-26 (seventh pass) — a run in another process can be told
something.** The report was *"I pressed ask mecha, the card said planning,
and nothing happened"* — and the card was right: a run was in flight, and
`planning` is exactly what `stateOf` derives when `waiting_on` names the
agent and no `todo` has been written. What was missing was any way to say a
word to it while it ran. `open the conversation` already existed and was
hidden on purpose, because a delegated run is a **detached child** and its
transcript has one writer.

So steering travels the way stopping already does: a file the runner polls,
drained into the run's own `queued_input` — the same queue a TUI's typed
steering goes into. `agent.rs` is untouched;
`run_interruptible_watching` gained a `pump`, deliberately named for nothing
more specific than *something to do on every watch tick*, so the loop never
learns that a steer can arrive as a file any more than it learns where a
tool came from. Verified live rather than asserted: the instruction arrived
as a text block appended to the user message carrying three `tool_result`s —
the shape the API requires and the one the loop's own doc insists on — and
the model obeyed it on its next turn, answering in one line as told.

**And it closed a hazard that was already open.** Every surface that picks a
session back up refuses to mint a twin of one *this* process holds, and none
of them could see a detached child, so resuming a delegation mid-flight
would have given one JSONL two writers. The run marker now names the
transcript it is writing and `live_writer_of` asks that question of the
other processes; `resume` answers 409 with the task named. The UI's
`!working(t)` condition was never the guard — it was the only thing standing
in for one.

Three rules the marker store already knew, applied to the new file: a steer
is **appended** (two sentences typed a second apart are two things the owner
meant), **drained** (a file that survives its own delivery arrives again on
every later turn), and a run starts **uninstructed** the way it starts
uncancelled — which is the stale-cancel bug this module was extracted for,
arriving in a second costume.

The seed's bullet order moved in the same pass, and the reason did not
survive its own measurement: pooled across the day it looked like the new
bullets had suppressed `ask_user` (5 of 6 against 0 of 4), but the arms ran
different tasks, and the one within-task series splits 1-of-2 / 0-of-2 /
0-of-2 across the three seed orders. The order stands on reading order and
is pinned by a test; the pooled number is confounded and is recorded here so
it is not quoted later as a result.

**2026-08-26 (sixth pass) — D4, and the assembler that points instead of
pasting.** The last open item of the task-agent arc, and it needed less
building than the design implied, because three things had become true that
D4 predated. `kg_task_list` returns `captured_from` on every row — the
provenance pointer that shipped that morning — and `work_prompt` never
mentioned it, so a task captured from an email reached the agent as a bare
sentence while `mecha tasks source` sat on the CLI able to fetch the thread.
`defer_until` was dropped the same way. And the run's own surface already
held `kg_search`, `kg_entity`, `kg_related` and `kg_timeline`, so the
project neighbourhood D4 wanted assembled into the prompt was one call away.

So the seed **points**: the provenance line, `defer_until`, a bullet naming
the mail reader when the capture is mail, and a bullet naming the graph
lookups. A seed is the front of a cached prefix every turn of every task run
re-sends, so pasted context is paid for on all of them where a sentence
naming a tool is paid once and followed only by the runs that need it —
`skill.rs`'s progressive disclosure one door over.

**The constraint that decided the shape is not the token budget.**
`captured_from` can point at *mail*. Pasting a thread body into the seed
would arm `untrusted` before the run's first turn **and** put
attacker-controlled bytes into a privileged run's opening instruction, which
is `frontdoor::Record::for_privileged_run`'s argument arriving through a
third door. The seed therefore carries kind, id, account and timestamp, and
never the `label` — a subject line is prose somebody else composed, and the
test fixture's label is an injection so that the assertion is the boundary
rather than a formatting check. The bytes arrive as a tool result, where the
interlock accounts for them and the `<untrusted-content>` envelope is
already around them.

**Tools are named by their registered name, and this deployment proved why
in one command.** `mecha tools --json` here shows the graph tools bare
(`kg_search`) and mail prefixed (`mail__mail_get_thread`) — one machine, two
conventions — so a seed hardcoding bare names would have pointed the run at
a tool it could not dispatch in exactly one of the two bullets. That is the
level-3 skill bug, which was found by running it rather than by reading it.
`Reach::of` reads the registry, after the D6 withholding and the D13
`ask_user` insertion, for the same reason `RunConfig::of` does.

And D4's own line — *"measured in Phase 4, not assumed in Phase 1"* — was
finally honoured, on two runs. A throwaway task named only `mecha` as its
project drew **seven graph calls** (`kg_search` ×4, `kg_entity` ×3) with no
prompting but the bullet. Then a task captured from a real three-message
mail thread called `mail__mail_get_thread` **as its first act**, before
anything else, and the run's recorded taint came back `private +
untrusted` — the bytes arriving as a tool result the interlock accounted
for, which is the entire argument for carrying a pointer instead of a body,
observed rather than asserted. Two runs are a direction and not a result;
pasting stays available if later ones ignore the pointer.

**The same run found the next thing to look at, which is not D4.** It ended
`completed` with six numbered decisions in its closing text and **zero
`ask_user` calls**, on a surface that held the tool (63 tools, `ask_user`
among them; `kg_task_update` correctly absent, so D6 held). The seed's
front-loading was obeyed in content — ask first, one question, list every
unknown — and ignored in mechanism, so the questions live only in a
transcript: `mecha questions list` cannot see them, the phone's card cannot
offer them, and the delegation reads as *finished* rather than as *blocked*,
which is the state D13 exists to prevent. The store is not broken (two
questions are parked from other tasks), so this is prompt adherence on one
run. It is worth naming because it is the precise seam D12 was decided
against on: the argument for putting the intervention on the user turn was
that this model obeys that channel, and here it obeyed the words while
skipping the call.


**2026-08-26 (sixth pass) — the harness stopped finding out by being refused.**
Rungs 4 and 5 of the goal system, and one measurement that had to exist before
either could be judged.

Rung 4 made the replay corpus a *draw* rather than a recency slice
(`harness_probe.rs`), and review caught the two things that decided what it
measures. `holdout_n` was clamped to the pool while `selection_n` came off the
unclamped want, so a corpus smaller than asked for went almost entirely to the
holdout — five held and one selected at the defaults over six eligible
episodes, which is five sixths of the real-model-run budget spent on the slice
that cannot decide anything. And headroom was read from `Session::last_outcome`
while an episode is the *whole session*: `drive_episode` replays every recorded
user turn and folds each with `absorb`, so the priority signal was sized in a
different unit from the arms it feeds, and it inverted — nine error-heavy runs
and a clean tenth scored zero and sorted last. Fixing the second needed a fold
over recorded rows, and writing a second fold would have been the hazard the
fold exists to close, so `absorb` split into `of_run` + `merge` with
`Session::episode_stats` folding through the same `merge`.

Rung 5 is `pressure.rs`. `compact_at` is checked at the top of the loop against
what the provider charged for the *previous* request — but by then the
assistant turn and a batch of tool results are already in `messages` and nobody
has priced them, so the reading the decision is made from describes a list one
turn out of date. §4.4 of the design proposed predicting from an observed
growth rate; building it corrected that, because **there is nothing to
extrapolate**: the un-priced tail is measurable in bytes, and the provider
re-supplies the byte-to-token conversion every turn by pricing a list whose
size is known. So the predictor is arithmetic on two measurements — no tuned
parameter, no model call, which §7.4 requires of anything running during a
turn.

Four things carry it. The **delta form** anchors on the last real measurement
and adds only the marginal cost of what changed, which removes the system
prompt and tool specs from the arithmetic because they are already in the
anchor. The rate is **clamped into the band a tokenizer can occupy** — never
below the plain-text rate, never above one token per byte, and not measured at
all from a delta under 512 bytes. `over` is spelled `reported || predicted` and
never the prediction alone, which is §7.3's monotonicity as one line of code
with a property test over a grid of states. And a **rewrite retires the reading
it invalidated**, which is the one thing here that can move a decision later
and is not an exception: a reported size is a measurement *of a particular
message list*, and once eviction rewrites that list the number is not a reading
of anything.

That last one fixed something that had never worked. After the free passes
freed space the loop `continue`d, meaning to "give it a turn to take effect
before paying for a summary" — but it jumped to the top without sending a
request, `prompt_tokens` is assigned in exactly one place and only after a
response, and the three passes are idempotent, so the re-entered check saw the
identical stale value and the summary was paid for anyway one iteration later.
Measured on the test fixture: **three summary requests, all waste.**

Rung 5 then took the tool-output budget too. `[tools] output_budget_bytes`
sizes "the gap between the threshold and the window" from the *window*, once,
at startup; the tracker knows where the transcript actually is, so under
pressure the cap is `min(configured, affordable)` — a `min`, so it can only
narrow, and gated on `spill_dir` being set, because `cap_result` relocates
over-cap bytes to a file the jail admits only when there is somewhere to put
them. That condition is what §7's table needs to be true and does not state.

And the series moved onto `Conversation`, beside taint. One submission is one
run in chat and the TUI, so a per-run tracker started empty on every user turn;
it resets when the model, system prompt or tool surface changes, because an
anchor is a token count for a byte count under one tokenizer and there is
nothing to convert it *to*.

Underneath all of it, `context_overflows` on `RunStats` — the measurement that
had to land first, in its own change, because a baseline established in the
same commit as the thing it grades is not one. `compactions` counts summaries,
so an overflow answered by eviction and thinning alone incremented nothing and
was invisible in every store; the harness caught a 400, rebuilt the transcript
and retried, and nothing said it had. It is the one counter on `RunStats` typed
`Option`, because the corpus it is read from spans the commit that introduced
it and a plain `u32` would read every older row as a run that overflowed zero
times — quietly diluting the rate it exists to establish.

**2026-08-27 — the harness stopped taking the model's word for it.**
Rung 5's model-facing half, then the whole of rung 6, which is the last rung of
the goal system with no model in it.

Rung 5 finished where §7.1 said the second-order win was: the harness had been
predicting context pressure and acting on it — compacting early, narrowing the
tool-output budget — while the *model* could not see any of it, so an
anticipatory signal that could act on the **plan** was going unclaimed.
`ContextTracker::forecast` returns headroom, observed cost per recent turn and
turns remaining **as measurements**, so nothing asks the model to estimate its
own token use; the reading rides on the **`todo` result**, where it appears
when the plan is being revised and costs nothing on other turns; and a
`compact` tool, unapproved and argument-free, hands the *when* to the one party
that knows how much of its own plan is left. Monotonicity is structural rather
than promised — the floor stays `reported || predicted`, so nothing the model
decides can make a run compact later than it does today.

Rung 6 is `step.rs` and `boredom.rs`, and both exist because of the same
asymmetry read one tier apart.

**A step is closed by the model and nothing checked it.** A board task is
closed by the owner (D6), so a person is the check; a todo step has no person,
so the check has to be structural — D5's *derived from the record, never
self-reported*, reaching one tier below where it was written. The loop folds
its own trace into counters and stamps them on `ToolCtx` without learning which
tool cares (the `taint`/`context` precedent), and the plan tool — the only
party that knows where a step began — differences two of them into a span.
Two readings, both facts rather than thresholds: nothing was attempted, and the
last attempt did not succeed.

Four of its five decisions are about *not* firing, which is the half that
decides whether a check survives contact. A refusal is not a failure
(`unknown || (is_error && !denied)`, the eval rig's rule one tier down), so a
blocked step is told it was blocked. Only the last attempt decides, so a
recovered failure is the model working. A sibling still in flight supports no
finding, because mecha runs a turn's calls concurrently and doing the work and
ticking the box in one batch would otherwise read as the null step. And plan
revision is subtracted from every span — three `todo` writes mid-step are three
trace entries, which is precisely how a step where nothing happened reads as
busy.

The fifth was found by asking where the counters come from. **The trace is per
run and a conversation is many runs**, so in chat a step started before the
user last spoke differences against a larger number, saturates to zero, and
announces the null step on the commonest shape there is — the loudest reading
firing on ordinary work, which is how a check gets switched off. A mark from
another run is therefore unmeasurable rather than empty, which needed a run
identity minted at the loop's existing per-run stamping seam: a `RunContext` is
shared across every chat turn, so an id on the context would have been one
value for the life of the conversation and invisible to the reset it exists to
catch.

**And a run going nowhere had exactly two states, proceeding and dead.** The
loop guard is §9.1's rung 5 and was the only rung: dormant until a compaction,
and its response is to end the run. `boredom.rs` is the graded version, keyed
on the call *and* its result on the guard's own rule, and on
`compact::target_of` rather than the raw arguments — so two tools that reach
one file and get the same bytes count as one thing learned twice, while
identical arguments with a *changing* result stay polling. It spends nothing,
which is what makes it ungated where curiosity is not.

Its bounds all come from one piece of evidence, the same one behind
`collapse_repeated_failures`: a model is measurably likelier to fail a step
when its context holds its own earlier errors. So a rung is crossed once
(`==`, never `>=`), one notice per turn, three per run — nagging a stuck run is
a way of keeping it stuck.

**It shipped as rungs 1 and 3, and the missing one is a stated reason rather
than a to-do.** Rung 2 — consult — has two halves and neither is reachable: a
§7.4 marker does not exist, and while a skill does, nothing in the `Tool` trait
identifies the tool that loads one. `narrows_surface_to` is the closest and
answers `None` until a skill is already loaded, so it recognises the state the
notice exists to escape only after the escape has been taken. Rung 3 needed the
same kind of property and got one: `Tool::runs_a_fresh_conversation`, fourth in
the family with `carried_state`, `fixed_workspace` and `narrows_surface_to`, so
the loop learns that *some* tool starts clean and never that subagents are a
thing. Nothing else could have answered it — a delegate's name is whatever the
user called it in config, and its capabilities are derived from its child's
tools.

Then rung 7's observation half, which is the first rung whose *result* is a
number rather than a mechanism. `appraisal.rs` is the signed record §0 says
mecha never had — five channels, an agency, an exposure flag, and a label that
is a **pure function of the record**, because a model that reads a run and says
"frustrated" is a self-report: unfalsifiable, drifting, and precisely what a
fetched page saying *you have failed your owner* is aimed at.

Working §6's derivation table answered most of the degeneracy question before
any corpus did. Six of the ten labels are unreachable from what mecha
measures: `Pride` needs a charter line and a task well done is deliberately not
one, `Guilt` needs a notion of harm that nothing computes (`visible` is
exposure, a different claim), `Shame` is a cross-run pattern a per-event
function cannot see, `Excitement` needs a predicted error — and `Regret` and
`Disappointment` split on the counterfactual verdict, which costs a real model
run per arm. `Affect::reachable_today` makes that testable rather than only
written down.

Then the corpus agreed, harder. Over the live store `mecha sessions appraise`
reads 459 sessions, appraises the 120 that recorded an outcome, records **119
signed goal errors** and derives **neutral for every one**. Eleven of those
errors are positive — a draft written in mecha's name that the owner read and
sent unchanged — which is the one channel in this system that can say something
went well, recorded since the outbox existed and never counted anywhere until
now.

The useful part is what that implies about build order, which is not §14's. A
counterfactual verdict would give all 102 intervention errors a label
immediately, where the charter at rung 10 buys only the eleven positive ones. So
the probe is the cheaper half of the readout, and rungs 8–10 must not be built
on a label that is currently a constant.

Two design corrections came out of building it, both about units. **It is per
session, not per run** — the §5 record carries a session id and no run index,
and that turns out to be load-bearing: both working channels are session-scoped,
so a per-run appraisal multiplies them by the number of times a session was
resumed, measured at 5.9× on the intervention channel before it was caught, the
same mismatch rung 4 paid for in the other direction. And **there is no store**,
against §10: every deterministic channel is a pure function of records the
machine already keeps, so a store would be `runlog`'s rejected ledger — faster,
and a second source of truth that can disagree with the first. It earns one with
the first channel that costs something to compute.

**And building it turned up something already wrong.** `agent.rs` prefixes a
refusal it did not author with `"Denied by the user: "`, and the rule from that
is in CLAUDE.md; this is its mirror. `learning::extract_interventions` guarded
one harness voice, `FINAL_ANSWER_NUDGE`, and `EMPTY_TURN_NUDGE` was never in the
list — so every run the harness had to nudge contributed an "intervention" whose
text was mecha's own. Found by adding a third voice (boredom's notice, which
uses steering's slot) and asking what already read that slot. **Two reflections
in the live store were mined from it**, one of them `origin: clean`, unprocessed,
and therefore a candidate for a rule in every future prompt — and its lesson is
the nudge's own sentence handed back, *do not restart or re-derive steps already
processed*, which is what makes the shape hard to see: mecha teaching itself
something it was already obeying reads exactly like the loop working. Nothing had
consolidated, so no rule carries it. Fixed at both ends, because they fail
differently: `agent::is_harness_voice` is the closed list, owned by the party
that adds a voice, and `Reflexion::learnable` refuses one whatever its origin,
which is what reaches the records already on disk.

Underneath both, `RunStats::boredom_notices`, on `context_overflows`' rules and
for its reason: every threshold in `boredom.rs` was argued rather than
measured, and a detector nobody can count fires either constantly or never with
no way to tell which. `Option`, so rows written before the sensor do not read
as runs that were never bored. Recorded *after* the thing it grades, which is
the wrong order and is worth saying so — rung 5's own `context_overflows`
landed in its own change first, because a baseline established alongside what
it measures is not one.

**2026-08-27 (second pass) — the loop that had never once run, and the comment
that named three risks and missed the fourth.**

Two lanes worked the counterfactual probe from opposite ends and found the same
thing from different sides: `mecha validate`'s steer and denial probes had
**never been able to run on an interactive session**, and `validations.jsonl`
was empty not because nobody ran the nightly but because every probe skipped.
The counter reported it faithfully. Nothing read it as *this entire class is
unreachable*, which is a rate over a zero denominator printed as zero, in the
one store whose whole job is to be the evidence a gate is replaced by.

`replay_registry` refuses to build when a recorded tool is missing, and that is
right — the tool list is the front of the cached prefix, so a smaller toolbox is
a different agent and its divergences say nothing about the question. The tools
it caught were the ones registered **only by a front-end**: `ask_user` in 246 of
408 sessions, `recall` in 122, `show_file` in 55. The coupling closes on itself
— a probe needs an intervention, interventions happen interactively, interactive
sessions carry `ask_user` — so the bail fired on exactly the population the
probes exist to read. **319 of 407 sessions.** A `surface_only` registry
supplies them where nothing executes, and the store goes from 22% replayable to
76%.

The gate is on what a mode *does*, never on its name: `Stop` **and** `Error`,
because they run identically and differ only in the policy a caller applies to
the report. Not `Live`, where the replay continues as a genuine fresh run and a
fresh run holding a permanently-erroring tool is not one — its divergence would
read as a finding about the model when it is a finding about the harness. That
is measurement validity rather than fail-fast, and it took two wrong answers to
get to: gating on `Stop` alone leaves a non-executing mode still bailing, and
gating on nothing at all was the correction that overshot.

**Then the probes ran, and 12 of 13 came back inconclusive.** Three hypotheses
died in order, each killed by the lane that had not proposed it. Sampling
nondeterminism: the store records `seed: 42` on 388 of ~394 sessions, so the
sampler was pinned three ways over. Contention: `/slots` polled throughout every
run, maximum one slot busy, which was the probe itself. Recency of the one
success: it is *older* than the sessions that fail. What replaced them is
better than any of them — **yield is a race between the probe point and the
divergence point**, and the one success has its intervention at turn 2, the
earliest in the set.

The signature that settled the cause is worth keeping. Divergence indices
`#0:2 #1:6 #2:2 #3:1 #5:1`, median **one call**, against steer points from 3 to
33 — and one session contributing six probes with steer points at 10, 17, 20,
21, 28 and 33, **all six diverging at the same call**. A trajectory-dependent
cause cannot produce that. A per-session constant can.

**The constant is the tool surface, and the comment that should have caught it
was one word short.** `RunConfig` keeps the system prompt in full and says why —
*"the text lets a replay rebuild the request"* — and keeps the tools as names,
under a comment naming the risk it saw: *"a tool added, removed or renamed
between recording and replay changes what the model could have done."* Add,
remove and rename are the three that almost never happen. **Re-describe happens
constantly** — 49 commits touched tool definitions in three weeks of this store
— and a list of names cannot see it. Tools render *before* the system prompt, so
the replay was rebuilding the back half of the prefix byte-exactly and the front
half from whatever the registry said today.

`surface.rs` is the fix and it was decided by a measurement rather than a
preference: the specs are 69 KB against a 25 KB average session, so inlining
them would have quadrupled the session store, and a hash alone gives up the
rebuild that is the point of recording it. So they are written once per distinct
surface and cited by hash, on `ValidationRecord`'s `rules_hash` precedent.
`Option`, so a recording from before the field lands in **Unknown** and can
never read as matching — and `Differs` needs no blob at all, so legibility
arrives the day the field ships while rebuildability accumulates after.

**It recovers nothing already recorded, and the honest form of that is a
sentence nobody wanted to write**: the appraisal corpus and the validation
ledger start from zero the day it ships. The prediction it makes — restoring the
surface should push the divergence index up — is therefore **untestable on the
current corpus**, and both lanes agreed to say so rather than re-run the 13 and
read something into the number.

One thing did come out end to end. The first non-neutral affect label this
system has produced: `{neutral: 119, regret: 1}` — one session where the replay
went elsewhere without the steer, carried from the probe through `apply_probe`'s
`Agency::Own` to `affect_of`. One of 120 is not a result. It is the seam
working.

And two smaller findings, both of the same shape as the big one. `serves:` has
**never carried a value in production** — 112 of 120 sessions wrote a plan, none
named what it serves, including 15 delegated task runs, which is the case the
field exists for. And `sessions appraise` reported that zero **by construction**:
it passed `&[]` for goals unconditionally, so the command built to measure
whether the labels were degenerate could never have reported anything else.
Absent and zero conflated inside the instrument.

**2026-08-27 (third pass) — the stack this section describes landed, and one
finding named twice did not travel with it.**

Six PRs merged the day these two sections were first written: #86 (`bcd7d4f`),
#87 (`c81d0fb`), #93 (`4f28221`, carrying forward #86's fourth-round fix that
missed the merge by eight minutes — see the trap below), #88 (`18a6fcf4`), #89
(`1e04c114`) and #90 (`c630ff94`). Every review finding attached to #88, #89
and #90 was checked against the merged source rather than against the review
thread, on this skill's own rule.

**#88's two open findings both hold up as fixed.** `affect_of` now computes
`label_of`'s most-negative reduction *before* checking repetition, and only
lets the repeated-error branch return `Frustration` when
`says_more(Frustration) >= says_more(reduced)` — so a repeated self-inflicted
error can promote a `Neutral`/`Anger`/`Disappointment` verdict but can never
step in front of an exposed one, closing the case where a ceiling nobody
caused plus a visible mistake used to report as mere repetition
(`mecha-core/src/appraisal.rs:378-391`). `says_more` is exhaustive with no
`_ => 0` arm, so a future `Affect` variant fails to compile rather than
silently tying with `Neutral` (`appraisal.rs:294-302`). The doc-count drift is
fixed too: the printed line now reads "six of the ten `Affect` variants"
(`mecha-cli/src/commands/sessions.rs:570`), matching `reachable_today`'s true
count of four reachable against ten total (`appraisal.rs:233-238,963`).
Underneath both, three more fixes from the same review pass: `of_session`
takes `end_taint: Option<crate::agent::Taint>` rather than a bare `Taint`, so
a caller with no coverage passes `None` and the fail-closed `Untrusted` arm in
`classify_origin` stays reachable instead of being permanently shadowed by an
always-`Some` wrapper (`appraisal.rs:432-447`); `visible` for a drafted
message now reads `item.writing_outcome() == Some(WritingOutcome::SentUnchanged)`
rather than the item's raw `status`, so an edited-then-sent draft — the
owner's own catch — no longer reports as an exposure error
(`appraisal.rs:606-613`); and `RunStats::merge` folds `boredom_notices`
through the same `Option`-sum as `context_overflows` instead of leaving it
fixed at whatever the first row carried (`mecha-core/src/session.rs:397-405`).
And `sessions appraise` collapsed three separate reads of the same transcript
— outcome, interventions, goal — into the one `Session::read` the module's
own doc says exists to replace exactly that pattern
(`mecha-cli/src/commands/sessions.rs:372-398`).

**#90's three findings hold up too.** The surface store hashes with
`learning::rules_hash` — the same FNV-1a function `ValidationRecord` already
uses — rather than a fresh `DefaultHasher`, which this repository has banned
twice before for the same reason: a `DefaultHasher`'s algorithm is a `std`
implementation detail with no stability guarantee across a toolchain bump, and
a persisted key built from one silently stops matching itself
(`mecha-core/src/surface.rs:118,141`). The store root is created through
`create_private_dir`, matching every sibling `~/.mecha` store instead of
leaving it world-readable by omission (`surface.rs:161`). And the
probe-budget warning now gates on `tally.over_budget > 0` rather than on
`wanted` — which counted every intervention including the unprobeable
`followup`/`edit` ones — so a corpus that is mostly followups no longer
prints "budget stopped" when nothing was actually capped
(`mecha-cli/src/commands/sessions.rs:453-499`). The `OnDivergence` gate for
the surface-only registry is an exhaustive match (`Stop | Error` explicitly,
not `Live`) rather than a `matches!` denylist, and `Fidelity::of` is wired
into `appraisal_probe::annotate_with_fidelity` rather than sitting unused
(`mecha-core/src/replay_run.rs:223-230`, `mecha-cli/src/appraisal_probe.rs:75`).

**#93 is exactly what it says: a same-content carry-forward, no open
findings.** `runlog::boredom_rate` and the denominator-aware "went nowhere"
line both exist as `sessions health` prints them
(`mecha-core/src/runlog.rs:227`, `mecha-cli/src/commands/sessions.rs:776,789`).

**#89 is the exception, and it is worth stating precisely because the shape
is a trap rather than a one-off.** Five real findings from its review rounds
are fixed and verified against `main`: the proposals pane reads `kind`/
`detail`, the keys `proposals list --json` actually emits, instead of a
`status` key the command never produces
(`mecha-cli/src/tui/learning.rs:454-455`); `mecha rules show` exists
(`mecha-cli/src/commands/rules.rs:36,77,240`); `find_rule` refuses an empty
needle instead of prefix-matching every identified rule
(`commands/rules.rs:175-186`); the Reflections pane loads with `--all` so a
dropped row stays visible for `u restore` to reach
(`mecha-cli/src/tui/mod.rs:6010-6025`); and `wrapped()`'s word-break branch
uses `saturating_sub` instead of an unchecked subtraction that panics in
debug and loops forever in release on a narrow, deeply-indented terminal
(`mecha-cli/src/tui/queues.rs:1182`).

But at the time this section was first written, three findings that were
named — one of them twice, in two separate review rounds — were still true
of the code on `main`: `App::a_modal_is_up` did not check `self.learning`,
so the modal's mouse capture never released; `learning_act` called
`self_cli` synchronously from the key handler, and every verb it ran took
the learning store's flock that `reflect`/`learn` hold across a model call;
and `reject`/`drop` passed no `--reason`, contradicting the modal's own help
text — with `doctor`'s starved-learner finding compounding the last one by
counting every owner-dropped reflection as "excluded by origin" alongside
the ones the provenance gate actually excluded.

The general lesson stood before the fix and still does: **a later review
round approving a PR is not evidence that an earlier round's findings were
addressed** — a reviewer re-reads the current diff, not the accumulated
list of everything anyone has said about it, so a finding raised on round 2
and never mentioned again on round 3 through 6 can merge exactly as it was
found. The fix is the one this skill already prescribes for a different
failure — verify against source, not against the review thread — applied
one layer up: a merged PR's own review comments are commit-message-shaped
evidence too.

**2026-08-27 (fourth pass) — the rest of the stack landed the same evening:
#91, #94 and #92, in that order.**

**#91** (`f0ca8ca`, mecha-80's) is the probe half neither the second- nor
third-pass entries above had landed yet. It fills `GoalError::controllable`
from a real counterfactual: rebuild the recorded run up to a `steer`/`denial`, replay it
**without** the steering text, and read whether the run arrives at the same
place anyway (`Some(true)` if not — the agent could have done otherwise —
`Some(false)` if so; wired at `mecha-core/src/appraisal.rs:692,695`). It is
also where the reach and yield numbers already recorded in this section's
"(second pass)" entry (13 of 102, 12 of 13 inconclusive) were actually
produced, and where the one genuinely new label came from: `Agency::Own` +
`controllable: Some(true)` reaching `affect_of` as `Regret` — the first
non-neutral label this system has ever emitted from a real run rather than
a unit test, `{neutral: 119, regret: 1}` over the 120-session corpus.

**#92** (`a0638c8`, mecha-4c's) fixed the `serves:` seed gap this document
had attributed to "nobody was ever told" — the truer diagnosis, caught in
#92's own review, is that `TodoTool::description` already told the model to
pass `serves`, but nothing bound that generic reminder to *this run's*
specific task id, so all 15 delegated runs in the corpus carried the
instruction and their own id on the seed and still wrote nothing. Both
delegated postures in `tasks.rs` now bind the id explicitly. #92's own
commit is the record of what changed in the code; this entry does not
re-derive it, on the same rule that keeps this file from re-deriving #86's
fix once #93 already carried it.

**#94** (`2f432b3`) is a second cleanup pass on #89, filed the same evening
this section's "third pass" entry was written — before that entry's own
four still-open findings could be called old news, a re-read of *every*
review comment on #89 (not just the ones the first cleanup pass covered)
turned up two more of the same shape, and all six shipped together. Verified
against the current tree: `App::a_modal_is_up` now checks
`self.learning.is_some()` (`mecha-cli/src/tui/mod.rs:601`); `reject` on a
proposal now passes `--reason "rejected from /learning"`, the exact
fixed-string pattern `/queues` already used for the same command
(`tui/mod.rs:6274`); `doctor`'s `excluded` counter now skips a row with
`dropped_at` set (`mecha-core/src/doctor.rs:1442`), and `mecha learn` reports
its own `dropped_by_owner` count instead of folding it into "excluded by
origin" (`mecha-cli/src/commands/learn.rs:107-150`); `learning_act` now runs
every verb through the detached-and-watched shape `/outbox` and `/triggers`
already use, polled by a new `Watch::Learning`, instead of blocking the key
handler on `self_cli` (`tui/mod.rs:6140-6161`); `LearningStore::reflexion`
carries the same empty-needle guard `rules.rs::find_rule` got in #89's first
review pass, never carried to this sibling lookup until now
(`mecha-core/src/learning.rs:1103-1110`); and `/queues`' `move_sel` now
clears `review_detail`/`detail_scroll` on every move, including the
`g`/`G`/Home/End jumps that bypass the delta path
(`mecha-cli/src/tui/queues.rs:500-508`) — before this, scrolling past a long
proposal's fetched text with `j`/`k` could leave the previous proposal's
text on screen while the cursor sat on the next one, so an `a`/`r` right
after could act on something never actually read.

Two traps in one PR sequence, same shape, six hours apart: #86's fourth
review round missed the merge window by eight minutes and needed #93 to
carry it forward; #89's second and sixth review rounds named findings a
different pass's cleanup did not re-check and needed #94. Both are the same
lesson — a standing "merge once reviewed" authorization, or a cleanup pass
scoped to "the findings I already know about," answers a narrower question
than the one that matters, which is *has everything anyone has said about
this diff been addressed*. Neither failure was expensive to fix once
noticed; both were invisible from the outside for as long as nobody looked
past the PR's own final "looks good."

**2026-08-27/28 — rung 9's first two pieces landed: an episode now carries
how the session went, and the world disagreeing with the graph is noticed.**
Two PRs, `docs/GOAL-SYSTEM-DESIGN.md` §10 and §10.1: **#97** (`4d9f27f`,
episode tagging) and **#98** (`c32eaca`, surprise/gossip-seeding). A third,
**#101** (`be9e32b`), is four rounds of review fixes on top of both — the
built-in review bot had hit a transient action-infra failure on every one of
its last several attempts, so neither #97 nor #98 had actually been reviewed
before merging, and a Codex-driven review was requested instead.

`appraisal::for_session` (`mecha-core/src/appraisal.rs:736`, returning a
`SessionAppraisal` at line 714) is the one assembly `mecha sessions appraise`
and `mecha distill`'s new episode-tagging path now both call — `Session::read`
once, the outcome/interventions/goal off the same pass, on the three-reads-of-
one-file rule `Session::read`'s own doc already names. `mecha-cli/src/commands/
sessions.rs`'s `appraise` function shrank to a call at line 432; `mecha-cli/src/
commands/distill.rs` gained the same call and threads the result into
`distill::upsert_args` (`mecha-core/src/distill.rs:427`), which stamps the
pushed episode's `meta.affect` and, when non-empty, `meta.goal_errors`. Neither
is gated on the session's taint the way a correction is — they are structured
facts the harness computed about its own run, not prose a fetched page could
have authored — with one deliberate exception: a `GoalError`'s `goal` is the
model's own `serves:` argument, so `upsert_args` redacts it to its bare kind
word (`task`, never the id) before it crosses into pkg's data, since the id
itself is unconstrained text an injected plan could have populated.

**#98** extends the same quarantined `Distiller` pass (the one that already
finds `corrections`) to also report `Surprise { predicted, actual, about }`
(`mecha-core/src/distill.rs:134`) — a moment where something the agent said,
sourced from the graph, was contradicted by something else in the same
session ("I said the 14th because the graph says so; the email says the
9th"). Unlike affect and goal errors, a surprise **is** gated on taint —
`surprises_for` (`distill.rs:278`) mirrors `corrections_for` (`distill.rs:253`)
exactly, because a surprise's `predicted`/`actual`/`about` are the model's own
free-text reading of transcript prose, indistinguishable in kind from a
correction's `wrong`/`right`. Deliberately **not** wired to run
`mecha gossip --entity <about>` automatically — `mecha distill` only prints
each one, on this project's standing rule that real model spend needs a human
gate rather than a session's own say-so.

**#101 is four rounds on one PR, and the shape repeats hard enough to be its
own trap** (see Traps → Review process, below, for the general lesson). Round 1
(the first Codex review to actually complete, after several transient
action-infra failures on #97/#98 themselves) found two real gaps: `mecha
distill`'s surprise print used unescaped `println!`, and the "a person's own
terminal is a safe context" argument for that assumed a live terminal —
`scripts/ruminate.sh` actually redirects the nightly run's output to a dated
logfile, exactly as exposed to a deferred read as any other log. And a
genuinely unreadable outbox during episode tagging used to warn-and-continue
(`mecha-cli/src/commands/distill.rs`), after which `mark_distilled` made the
resulting incomplete `Edit` channel permanent — no later run could ever
revisit it. Round 1 fixed both: `strip_ansi` (`mecha-cli/src/logs.rs:175`,
made `pub(crate)`) on every printed field, and a bail via `.context(...)?`
instead of `eprintln!`-and-continue.

Round 2 (the built-in bot, having finally run) found `strip_ansi` only
strips ESC-introduced sequences — a bare `\r`/`\n` survives it and can
rewrite the printed line or forge an extra one, defeating the very "⚠
untrusted" marker round 1 just added. New `strip_ansi_and_controls`
(`logs.rs:240`) closes it. And round 1's outbox fix only bailed on a hard
I/O error; `OutboxStore::items` (`mecha-core/src/outbox.rs:376`) also
silently *skips* a merely malformed item file behind an invisible
`tracing::warn!` (the nightly runs with no `MECHA_LOG`) and still returns
`Ok` — a silently short list indistinguishable from an outbox with fewer
drafts. New `OutboxStore::items_strict` (`outbox.rs:401`, sharing
`items_impl` at line 405) bails on that too; `items()` itself is unchanged
for its other callers.

Round 3 (the bot again, on round 2's fix) found `char::is_control()` is
Unicode category Cc only — U+2028/U+2029 (line/paragraph separator) forge a
line break exactly like a bare `\n`, and U+202A–E/U+2066–9 (bidi
overrides/isolates) can visually reorder the rendered line around the
warning marker, the Trojan Source shape. `strip_ansi_and_controls` was
widened to name both categories explicitly, deliberately not generalized to
a "printable only" filter — the field is free-text prose that may
legitimately carry non-Latin scripts, and a filter that cannot say which
characters it distrusts is guessing rather than closing a described class.

Round 4 found two more, both corrections to claims rather than new
mechanism. `strip_ansi`'s own doc comment claimed its one call site was safe
because `Writer::write` cuts the stream at `\n` first — untrue even at that
function's original two call sites (`Writer::write` and `release()` in
`logs.rs`): `trim_end` only strips a *trailing* `\r`, not an interior one, so
a server-supplied error string or an unparsed model reply with a `\r` in the
middle had been reaching the TUI transcript and a live terminal unstripped
the whole time — a pre-existing bug in the TUI's own log capture, newly
exposed by a doc comment that had never actually been true. Both original
call sites now use `strip_ansi_and_controls` too. And the stated rationale
for `items_strict` — a half-written `.json` mid-save — turned out to be
structurally impossible in this store: `outbox.rs`'s own module header
already says temp-sibling-and-rename means a reader never sees a partial
write. The real cause is persistent, not transient — a stray file, or an
item written by a schema this binary cannot read — and all four places that
had repeated the wrong claim (a code comment, a doc comment, the error
message, and the CHANGELOG) were corrected to say so, including dropping the
implication that a retry would clear it. **Noted, not built**: a `mecha
doctor` finding for a stalled distill ledger — today the nightly fails
silently behind one line in a dated logfile, with no `MECHA_LOG` and no
doctor check for it.

**What's left of rung 9**: review-queue salience, the rest of §10, needs
changes in the private `personalized_knowledge_graph` repository — a
different codebase mecha only reaches through the MCP tool surface — to read
`meta.affect`/`meta.goal_errors` and reorder pkg's review queue on them. Not
started, and not scoped beyond `GOAL-SYSTEM-DESIGN.md` §10's own paragraph
naming it.

**2026-08-28 — rung 10: the charter, and a guilt sensor that shipped
deliberately unconsumed.** PR #100. Landed ahead of rung 8 in §14's build
order on purpose — argued from a measurement, same as the probe reordering
above: both pieces of this rung depend on neither the probe nor the affect
label, so there was no reason to wait on either.

The charter (`mecha-core/src/charter.rs`, `mecha-cli/src/commands/charter.rs`)
shipped as designed in §11: `~/.mecha/charter.toml`, an ordered list of
standing priorities the owner authors and edits by hand, ranked by file order
rather than a weight, with no config field, no project layer, and no write
path anywhere a model or this command could use — `mecha charter` is
read-only. Rendered straight into the system prompt (no progressive
disclosure, unlike skills) via `setup.rs::prepare_tools`. `--no-charter`
matches the skills/rules opt-out and `mecha eval` forces it off with
everything else a scorecard must not depend on.

Anticipated guilt (§7.4) shipped as a **recorded sensor only** —
`crate::guilt::anticipated_guilt`, folding the age of the oldest commitment
recorded in the outbox/questions/front-door stores, how many are waiting, and
the run's own peak context pressure into `Homeostat::anticipated_guilt`.
**Nothing consumes it.** Confirmed before building it that nothing today
wires `Backlog`/"owner-attention debt" to the model at all, so there was no
existing seam to narrow — inventing one under time pressure is exactly the
scope §7.2 warns a guilt mechanism must not acquire. Same precedent as the
homeostat (rung 3) and boredom (rung 6): ship the sensor, let a corpus exist,
decide the consumer deliberately later. Also folded the homeostat into
`diagnose::Evidence` (mean context pressure, mean anticipated guilt, both
`None` over unsensed rows, never zero) so the nightly diagnostician's brief
carries machine conditions beside outcome counters.

**The automated PR review earned its keep here, five rounds deep, and the
last round found the most important bug of the five.** Summarized rather
than itemized, since the PR's own review thread carries the full record:

- A partial backlog read (one store unreadable, others empty) collapsed into
  `Some(0.0)` — indistinguishable from "genuinely nothing owed" — and a
  counted-but-undated commitment scored as fresh instead of unknown. Fixed by
  requiring all three stores readable and by making pressure itself unknown
  rather than a measured zero when a provider declares no `context_window`
  (the same floor `Homeostat`'s own doc already warns against, reintroduced
  one struct over).
- The character budget crossing `CHARTER_CHAR_BUDGET` refused to load the
  whole document — inverting the learned-rules precedent
  (`over_budget_domains` warns and still loads) and meaning an owner's
  eleventh priority line would silently un-charter every future run. Split
  into a hard validity check (duplicate/empty id, bad TOML — still refused)
  and a separate `over_budget()` warning that `setup.rs`, `mecha charter`,
  and a new `doctor` check all surface without dropping the document.
  `RawCharter`/`CharterLine` also gained `deny_unknown_fields`, since no
  harness but this one authors a `charter.toml` and a typo'd table name
  sitting beside a correct one used to vanish silently rather than error.
- **The one that mattered most**: `Homeostat::finish` fed the sensor the
  *post-run* backlog, so a trigger that staged three replies overnight —
  doing exactly its job — hit the count term's saturation point and recorded
  `anticipated_guilt: 1.0`, indistinguishable from three replies it found
  waiting and ignored. And separately, the one-day age-saturation horizon was
  shorter than the real distribution these stores produce: `questions.rs`
  parks answers overnight *by design* ("nobody answers until morning" is the
  mechanism working), and `backlog.rs`'s own canonical fixture ages a wait
  8–9 days. Because the three terms combine as a logical OR, either gap alone
  was enough to saturate the whole reading to a constant `1.0` on any real
  install — the same degenerate-label shape rung 7's own measurement found
  the hard way, just found before a corpus existed instead of after. Fixed by
  reading the pre-run snapshot instead of a fresh post-run one, and by
  widening the horizon to a week.

The general lesson, on top of the one #86–#94 already recorded above: a
review that keeps finding real bugs on the fifth consecutive pass is not
diminishing returns, it is the review doing its job — the temptation to treat
round four's "looks clean" as a stopping condition is exactly backwards when
round five is the one that finds the bug that would have made the entire
shipped sensor a useless constant.

**2026-08-28 — rung 7 closes: the model half of step appraisal.** PR #102.
`docs/GOAL-SYSTEM-DESIGN.md` §5.5's escalation — the two comparison signals
rung 6 left to a model rather than a threshold (a landed step's span a clear
outlier against the plan's other completed steps; a step whose own words
claim verification with nothing verify-shaped in its span) — now runs inside
`agent.rs`'s own loop rather than as an offline CLI pass, since a step's plan
action has to reach the *same* run before it wastes more turns on a bad
decomposition. `ToolCtx::step_escalation` is `compact_requested`'s exact
shape (a tool asks, the loop acts, presence is the enablement); `tool/todo.rs`
writes a candidate into it, `Agent::escalate_step` settles it with one
quarantined call routed through the same cancellable `self.complete()`
`compact()` already uses, and the verdict folds into the turn as a fully
templated nudge — the model's own free-text reasoning about it never reaches
the conversation or the learning miner. Off by default (`[agent]
step_escalation`, `--no-step-escalation`, forced off under `mecha eval`),
since the pre-filter's thresholds are argued, not measured; `RunOutcome`/
`RunStats` gain `step_escalations_attempted`/`step_escalations_revised` so
that measurement can eventually be taken from the store, on `boredom_notices`'
own precedent.

**Fourteen real review rounds, the deepest single-PR cycle recorded here so
far — summarized, not itemized, since the PR's own thread carries the full
record.** The shape worth remembering is that the findings clustered in two
places, both non-obvious the first time: the quarantined call's own
cancellation/budget/effort/logging discipline (it originally bypassed
`self.complete` entirely, missed the loop's `stopping` check, left `effort`
unset, and logged its own failure at `debug`), and a family of bugs in how
`Tracked::completed`'s rolling history interacts with a plan write that does
more than one thing at once — a batch completing two steps at once, a
rewrite and a completion landing in the same write, and a step revised and
recompleted all corrupted the "plan's other completed steps" baseline in
distinct ways that three separate, increasingly narrow fixes were needed to
close, the last of which (a step seeing its *own* pre-revision entry as a
sibling) survived one round after the doubling half of the same bug was
already fixed and named in the fix commit's own comment. **The most
consequential single fix**: `escalate_step` streamed its quarantined call's
raw JSON reply — reasoning included — to whatever front-end was attached
(TUI, Slack, voice), because it forwarded the run's real `events` sender into
`self.complete`, which forwards every text delta as ordinary assistant text.
The design's own stated guarantee ("the model's reasoning never reaches the
conversation") held; "never reaches the *user*" did not, until `&None` was
passed instead. Also mid-cycle: `main` picked up an unrelated PR (rung 8),
and GitHub's `pull_request` CI — which tests an implicit merge with `main`'s
current tip, not the branch's own head — caught the resulting gap before a
local merge ever would have (see Environment, below).

The general lesson: a mechanism whose whole design is "quarantine a model
call and template its output" still needs the same auxiliary-call discipline
(cancellation, usage, effort, logging) every sibling quarantined call in this
file already has, and building it fresh instead of copying the sibling's
shape is exactly where the gap opens — cancellation and usage were caught
early, but effort and the failure-log level were each separate, later
findings in the same category, arriving well after the first pair looked
like it had closed the class.

**2026-08-29 — the appraisal-system review arc lands whole: PRs #111 and
#112, nineteen reviewer rounds deep.** A test-and-review pass over the
appraisal goal system (three parallel review agents plus hands-on runs of
the free readout, the quarantined appraiser and the counterfactual probe
against the live store) produced one high, three medium and a tail of
smaller findings; fixing them, then iterating with the PR auto-reviewer
until it came back clean, produced the rest. What shipped, by family:
**failed-turn integrity** — `Conversation::roll_back_failed_turn`
(restore-then-pop, popping only a plain user text per `is_plain_user_text`)
replaced five divergent pop sites across the REPL, TUI, web and voice
surfaces, every error arm now records the rolled-back state so a resume
loads what memory holds, and every submit site folds into a user tail
(recording the fold at submit as one direct `Rewrite`) instead of pushing
two user messages in a row. **Positional configs** — `Transcript` carries
`config_positions` and a `taint_timeline` built in `Session::read`'s one
pass; `config_covering` replaced `configs.first()` in the counterfactual
probe (a resumed attach's steer no longer replays under the wrong system
prompt and reads as inflated `regret`), with the rewrite arm
distinguishing index-preserving rewrites (truncation, fold, in-run
eviction) from summarising ones. **The instrument stops eating its own
findings** — unreadable transcripts are counted at every layer
(`Session::list_counting`, `Corpus.unreadable`, a doctor finding,
caveats/fields on `appraise`, `stats` and `health`), `sessions_read` and
`unreadable` are disjoint by construction, and the `named_a_goal` counter
lost in #88/#91's merge overlap is rebuilt. **The owner-closure guard**
(#112) — `closure_guard::ClosedStatusGuard` wraps `kg_task_update` on
every model-facing registry so a closing `status` is refused toward
`mecha tasks set` (which appraises); presence is `Tool::guards_closures`,
a trait answer a wire tool cannot fake, `closure_guard::verify` makes an
unguarded surface a startup error, and the guard's refusal is classified
(`ToolOutput::refusal` → the trace's `denied`) so the harness's own "no"
never counts as a failed run. Plus the affect chip on the typed web
surface (muted, deliberately not taint-amber), the TUI badge surviving
`--no-session`, `for_transcript` collapsing distill's four reads to one,
and the closure path's smaller repairs (prefix re-key, empty `--session`,
follow-up caveats). Deployed the same night; the environment note in
HANDOFF carries the capability-level verification.

**2026-08-29 (later the same night) — four merges from three concurrent
sessions, and an incident threaded through them.** #114 put the graph's
*shadow* queue (review-on-use surfaced verdicts) on every surface an owner
holds: `mecha review shadow` with `--confirm`/`--refute` through the
mecha-graph child, `/api/queue/shadow` + `/verdict`
(`serve/review.rs::shadow`/`shadow_verdict`), a surfaced-verdict deck on
the web review page, a graph-shadow row with in-place verdicts in
`/queues`, and per-fact tier marks in `/find` — plus, grown along the way,
a web entity page (`serve/board.rs::entity`, `Entity.svelte`, with
`entity_detail_marks_unreviewed_and_denied_facts` pinning that unreviewed
and denied facts say so) and chat tool-result previews
(`WireEvent::ToolResult`, a capped preview of what a tool answered). #116
repaired the `/tasks` page v0.1.16 shipped broken (the trap under
Measuring); the fix was committed independently on the #114 branch
(`aa53174`) and deployed within half an hour of the report, after a second
session verified the broken call in the *served* bundle — a free
identifier survives minification under its own name, which made the
artifact decisive where ancestry was only suggestive. #115 reworked the
appraisal docs page around the questions a cold reader actually asks —
when each appraisal moment runs, what consumes the record — and documented
step appraisal for the first time. #117 gave the docs site a fixture-backed
demo of the web surface (structurally the only option: the real surface is
loopback-only behind a tailnet identity, so there is nothing public to
link, and a screenshot of the real one is a picture of the owner's actual
mail) through one shim — the app reaches the server through bare `fetch`
and one `EventSource`, so the demo replaces exactly those two and no
component knows it exists — and the two CI gates now on every docs build.
The gates were proven against the defect, not assumed: `render-check` was
confirmed to fail on the pre-#116 tree with the real `ReferenceError` and
pass after, and the bundle-purity check by deliberately building a demo
bundle into `web/dist` and watching it fail. The coordination itself held
up: single-writer docs were claimed and sequenced, every peer claim was
re-verified against an artifact before being acted on, and the one
mid-write flag ("someone is editing the main checkout") turned out to
describe both flagger and flagged — resolved by both lanes committing
through worktrees and restoring the shared tree clean.

**2026-08-29 (~04:09 UTC) — the apex finally points at the factory.** The
open item filed an hour earlier closed the same night: the owner repointed
DNS at Squarespace (deleting the whole "Defaults" preset group, which is
what also removes the HTTPS/SVCB record — see the trap under Environment),
and `redirect_hosts = ["mecha-factory.ai", "www.mecha-factory.ai"]` went
live on the droplet with a restart. Verified at the authority and at the
socket, twice over by two sessions: the authoritative nameservers answer
one droplet A record and no HTTPS record, and `curl --resolve` against the
droplet gets `301 → https://gate.mecha-factory.ai/` with the path
preserved. The redirect certificate was issued into **its own** group
(`CN=mecha-factory.ai`, SAN apex + `www`) with the base group untouched at
exactly `art, compute, gate` — `certificates.rs::redirect_group`'s design
claim, that adding a redirect host cannot take the gate's certificate
down, holding under its first live test. The same test confirmed the
query-string drop as a live bug rather than a prediction; that and two
sibling findings are the "apex-redirect residue" item in HANDOFF.

**2026-08-29 (~04:50 UTC) — factory 0.2.8 closes the apex residue the
same night it was filed.** The redirect keeps the query
(`http::redirect_target` on `path_and_query()`, its test verified to fail
on the old one-liner) and `factory check` names the redirect hosts
(`tls::describe()`), released as `v0.2.8` and deployed on the owner's
word — with the pleasing closure that the deploy's own pre-flight `check`
printed the `describe()` fix working during the very deploy that shipped
it, and the live confirmation that the request which dropped its query an
hour earlier (`/view/ljchang/abc?v=2&x=1`) now 301s with it intact. Found
along the way: **mecha-factory's CI had been red for eleven days** on
`cargo fmt --check` — four whitespace diffs predating 0.2.7 — and two
releases went out over it without anyone noticing, because a lint that
*always* fails reports nothing about the commit under it: the same shape
as a silently skipped test reading like a passing one, in mirror image.
Fixed (`fdfbb7b`, pure rustfmt), and CI on that repo is green for the
first time in eleven days. The one item a binary deploy structurally
could not fix — the droplet's stale TLS-ALPN-01 config comment, drift in
the hand-maintained copy that claimed port 80 is never part of issuance
and could have talked someone into firewalling off every future renewal —
closed later the same morning on the owner's word: the `[listen] http`
comment was re-synced to the example config's HTTP-01 wording (backed up
in place first; comment-only, so no restart), and `factory check` run
after the edit both proved the config parses and, in its own tls line —
`acme http-01 for …` — states the fact the stale comment denied. That
empties the apex arc entirely, from "Coming Soon" page to done in one
night.

**2026-08-29 (~13:50 UTC) — settings becomes a place, and the charter is
edited by hand.** PR #118. The gear moved out of `Home.svelte` into the shell
(`App.svelte`), so it sits in the same corner on every view — layered *below*
the app's scrims, sheets and drawers, because a button floating over an open
drawer is a bug only a phone meets — and settings itself became an index of
three features rather than one scroll of three stacked sections, each pane
routed at `#settings/<charter|learning|voice>` so back, forward and reload land
where they should. The charter is now edited as a list: tap a line, add one,
delete behind a two-tap arm, and **re-rank by dragging its grip**. That is not
a convenience but the only rank control there can be — `CharterLine` denies
unknown fields and §11 gives the charter no rank key, so position in the file
*is* the ranking, and moving a line is the design's own "only editing gesture
that cannot produce a tie". Nothing moved on the server: the same route, the
same 64 KB cap, the same `Charter::parse` before disk, the same two-tap
confirm, and the owner still authors every line.

Two gates came out of it and outlive it. `check-charter-toml.mjs` reads the
`WEB_EDITOR_SAMPLE` literal *out of* `charter.rs` by marker comments and
asserts the web serialiser emits it byte-for-byte, so a regression in either
language fails the other — the serialiser had to be lifted out of the
component into `web/src/lib/charter-toml.js` before it could be tested at all,
which is the general shape: code that can only be exercised by driving a
browser is code nothing will pin. And `render-check` gained the three
`settings/<pane>` routes, where nearly all the new code lives; a gate visiting
only `#settings` was exercising three rows and a chevron. It repaid itself
inside the same PR, catching a `ReferenceError: unreadable is not defined`
that the Vite build passed green, because Vite does not resolve identifiers.

Ten review rounds, twenty-eight findings, none disputed. The two that would
have cost data both concerned an *unread* charter being treated as an empty
one: a failed GET rendered "No charter yet — add the first priority" over a
charter that exists, one save from replacing it; and a `parse_error` arriving
with `raw: ""` (`charter_state` reads the file with `unwrap_or_default()`)
would have opened an empty TOML buffer whose save truncates a file whose bytes
were never read. Both fail closed now, in `unreadable` and `blocked`, which
between them refuse every writing surface on a document the page could not
fully account for — including a comment sitting among the tables, which the
regenerating serialiser would otherwise eat. Three of the ten rounds found
defects in the previous round's fix; that is the honest cost of repairing
under review, and the reason the last two rounds were smaller than the first.

**2026-08-29/30 — the learning loop runs itself, and the instruments that
graded it were lying (PR #122, fifteen commits, merged fast-forward at
`4c7a0e2`).** The gated path had produced 0 live rules in 25 days — four
mutually-exclusive proposals held 27 of 43 reflections while `learn` skipped
nightly. `learn --auto` is the ungated middle mode (Luke's 2026-08-19
ruling): the counterfactual gate stays in front of the write, a regression
refuses, measured-clean applies, and an ungradeable batch applies **on
probation** (`Rule::probation`, retiring at `PROBATION_RETIRE_AT` = 2), with
every pass still writing a proposal as audit trail and superseding its
pending predecessors (`proposals supersede` — releases reflections
unconsumed, where reject is the owner's no and consumes them). Retirement
became direct: `rules propose-retirements --apply` nightly, resolving any
pending twin it overtakes. First consolidation: 28 reflections → 12 live
rules. Consolidation moved to the session-close hook (`learn-live.sh`,
flock-guarded, workspace-jailed after a fail-open `cd ""` was caught in
review); `mecha learning-report` and a web trend pane are the is-it-working
view.

The instrument half: **counterfactual probes had never once concluded on a
mid-run steer**, because the replay regenerated the whole prefix and
required the model to reproduce every call before the steer point — 11 of
12 steer probes `inconclusive: diverged at call #1` against points at
#10–#28, an exponential lottery, not noise. Probes now **branch**
(`counterfactual::branch_at` + `replay_run::drive_branch`): the recorded
messages before the intervention are resubmitted verbatim (a steer keeps its
tool results and loses only the steering text; a denial regenerates the
whole proposing turn), so pre-point divergence is structurally impossible
and the forced prefix reads from KV cache. Measured after: 0 inconclusive,
12 graded; the first unattended nightly trace-graded 3 of 3 steers. Three
more instrument lies fixed in the same pass, each found by running the
thing: the trifecta interlock fired *inside* replays (a replayed send sends
nothing — `external_send` now narrows in the non-executing modes); a
recorded tool nothing today can construct killed the whole probe (it now
rebuilds as a spec stand-in from the `SurfaceStore` blob, and a recorded
spec wins over a live tool's reworded description, closing the
rebuildability half left open on 2026-08-27); and the judge graded
tool-call bodies as answers (`is_gradeable` drops whole spans, unclosed
tags included). Thirteen auto-review rounds ran against the PR; rounds 12
and 13 alone caught a fail-open workspace guard, `--auto` skipping the
already-argued brake, probation stamping every rule instead of the ungraded
ones, a nightly retirement pass silencing the starved-learner check for
48h, and `finalize_rules` dropping the probation flag — the D1 hedge
evaporating within a session or two while three doc comments described it.
Era hygiene rode along: the pre-stem step-nudge bodies are recognised as
harness voice, four reflections stranded on pre-`tools_hash` surfaces were
dropped with reasons, and `select_probe_corpus` honours `dropped_at`.

**2026-08-31 — the nightly diagnostician was told to read the source and stood
in an empty room (#127).** `DIAGNOSE_SYSTEM` said "You may read the source and
its documentation… treat a documented reason as evidence", and §12.4 of
`SELF-IMPROVEMENT-RESEARCH.md` names that clause a *safety input*: it is what
stops a proposal unpicking something load-bearing. It had never once been able
to fire. `scripts/ruminate.sh` stands the nightly in
`$(mecha work path ruminate)` — an empty directory, on purpose and for a good
reason — and `setup::prepare_tools` roots the path jail at the working
directory. Six nights, three candidates, zero acceptances, and every proposed
key fabricated: `security.minimize_taint`, `tool.validation.strict`,
`context.auto_compact` exist nowhere in the codebase. A model told to read the
source and given nothing to read writes down the key such a program would
plausibly have.

The two questions the script conflated were already separate in the code:
config is discovered from the **cwd**, the jail is rooted at the **workspace**.
So the fix keeps the script standing outside a checkout and points at one —
`HarnessConfig::source_dir`, global-file only and stripped from project layers
for `[slack]`'s reason at its sharpest, with `global_config_only` pinned so a
checkout's own `mecha.toml` cannot ride in. `diagnose_system` became a function
of what was actually granted: `holds_source` asks the directory whether it
holds `mecha-core/src` and `docs` rather than believing the config key, and the
blind branch says plainly that it is blind and forbids naming a key the brief
did not name first. **A prompt asserting a capability the run was not given is
the silently-degrading guard in its cheapest form — nothing fails, and the
protection reads as satisfied.**

The same pass found the brief reporting three of the six metrics it invited
predictions on, so `Evidence::brief` now renders every member of `Metric::ALL`
with its headroom and `ruminate` refuses a prediction on a metric no run has
any of. It also found `RunStats`' denial counters never briefed — on the live
corpus, 39 refusals by a person or a policy and 21 interlock refusals against
32 environment errors, so the denials outnumber the failures and both are the
harness working. And `SessionMeta.workspace`, recorded in every transcript
header since the store existed, had never reached `RunRow`: the corpus was a
mixture of four unrelated jobs pooled into one average, 389 runs across eight
workspaces. `RunRow::workspace`, a prefix filter on `runlog::Scan` and
`Corpus::by_workspace` separate them retroactively over all 490 sessions.

Two gate holes came from literature newer than §13. GRASP
([arXiv:2605.29668](https://arxiv.org/pdf/2605.29668)) names regression
accumulation: `judge_slices` scored the predicted metric and the tool-call
volume alone, so `WORK_FLOOR` caught a gain bought by attempting *less* and
nothing caught one bought by failing *more*. `guard_regressions` closes it —
rejecting a proportional breach and **proposing** where a cost appears from
nothing, because `compactions` is zero across the corpus and rejecting there
would have made `compact_at_tokens` unable to move the one metric it exists to
move. SEAGym ([arXiv:2606.17546](https://arxiv.org/abs/2606.17546)) predicts
the other: `Tally::not_worse` is `wins >= losses`, so four all-tie held-out
episodes satisfied it with zero and zero and read as "confirmed on unseen
work". `MIN_INFORMATIVE_HOLDOUT` routes that to `Propose`, which reaches a
person.

`SELF-IMPROVEMENT-RESEARCH.md` §14 is the measured reading of the built loop,
including §14.6 — the finding that outlives the corpus, that replay drops
diverged pairs, so a change small enough to leave the trajectory intact is
measurable and a change large enough to alter behaviour is discarded. §14.7
records the owner's rulings: self-improvement is measured against how the agent
is actually used and never a generic suite; Terminal-Bench is a periodic
transfer check that answers a question about the loop and never feeds it.

**2026-08-30 — a hypothesis tested and rejected: the provenance gate is not
crippling learning.** Investigated by the learning lane before ungating:
`Evidence::UserTurns` already rescues tainted conversations into clean
reflections (21 of 43 arrived that way); the real residue is 5 lessons, of
which 2 are outbox edits the transcript walk structurally cannot reach, and
`reflect --remine-untrusted` over all 9 affected sessions produced 0 new
reflections — the reflector declines to draw lessons from user words alone.
Do not re-open the gate on this argument; the evidence says it costs almost
nothing.

**2026-08-30 — the retirement drill ran the NoGo path whole, and its first
run proved the leash could never fire.** The ungating precondition
`LEARNING-LOOP-RESEARCH.md` §5 named — "a NoGo pathway that has never fired
is an untested backstop, and under D1 it is the only one" — was discharged,
one day after cutover instead of before it. `scripts/retirement-drill.sh`
records an honest `mecha run` session, seeds it (typed, via
`mecha-core/examples/retirement_drill_seed.rs` — a steer inserted into the
last tool-result message, a steer reflection, a probationary bad rule and an
innocuous bystander) into a world isolated by
`MECHA_SESSION_DIR`/`MECHA_LEARNING_DIR`, then drives real probe passes and
a retirement scan after each: the probe must regress, the bisection must
convict the bad rule and not the bystander, one conviction must hold, and
the second must retire it at the probation leash of 2, with the leash named
in `retired_reason`. Before any of that could pass, the drill's paper
walkthrough found the bug the research doc predicted a category for:
**probation released on bare ledger coverage (`observations > 0`), and an
attributed regression always arrives inside an observation** — the
bisection charges a rule from the same measured block the row records — so
`propose-retirements` stripped the leash in the same scan that read the
convictions, and `PROBATION_RETIRE_AT` = 2 was structurally unreachable
while three documents described it as the D1 hedge. Fixed as
`release_probation_when_measured_clean`: release requires the ledger to
grade the rule *beyond its convictions* (`graded > attributed_regressions`,
counting verdict-bearing rows only, so inconclusive coverage releases
nothing — the ran-vs-graded confusion round 13 fixed in dispose, closed on
the release side). The seeded rule's wording is itself a measurement
(arm-reconstruction harness, n small but clean): an advisory rule moved the
model 0/3, a bare MANDATORY directive 1/3, and the same directive carrying a
*mechanism* — stale reads, verify before reporting — 6/6; the branch point
replays the model's own recorded reasoning, and naked authority loses to
that momentum. The drill passed twice end to end (4/4 probe passes
regressed and attributed correctly) and is the standing check named in
ARCHITECTURE's learning section; `drive_arm` now traces each arm's verdict,
calls and final text under `MECHA_LOG=debug`, because the first failed run
was diagnosed blind without it.

The PR's review round then caught the fix's own mirror image before it
merged: `stamp_probation` shared the release's predicate, so once the
release stopped keying on bare coverage, a *born-graded* rule whose only
verdicts were its convictions could be stamped by an unrelated ungradeable
pass and never released — threshold silently 3 → 2 and a `retired_reason`
naming a probation it never had. Stamping and releasing now ask different
questions ("born ungraded?" vs "graded beyond its convictions?"), with
test cases that fail on the shared-predicate version. The same round moved
the ran-vs-graded distinction into every surface that shows a tally:
`render_active`, `rules`' describe, and `rules list --json` (which gained
`graded` beside `observations`) no longer render inconclusive-only
coverage as "0 improved, 0 regressed" — a clean bill of health from rows
that graded nothing. Merged as #124 (`6987bc5`) after a trial merge ran
the full suite on the merged tree, and deployed the same hour — the
install verified by the `graded` field answering from the installed
binary, since the version string was 0.1.16 on both sides.

**2026-08-30 — the graph tab became the curation surface, iterated live
against the owner's phone.** One evening, two sessions in a negotiated
file partition, ten deploys, three review waves. #125 (`68b4af9`) fixed
the four home queue cards that did nothing and pinned them with
`every_queue_the_backlog_reports_is_named_and_reachable_from_the_web_home`;
it also rewrote the ExecStart prohibition as a check after two sessions
violated the ban in one afternoon and the committed-script mitigation held.
#126 (`ab0097b`) is the arc: capture became a chat-idiom composer, the
notebook a bottom drawer pinned in layout (`?limit=` passthrough, sort,
filter), and the entity card grew the identity lifecycle in place —
two-tap alias removal and inline add (`mecha kg alias`/`unalias`, id-only
like `retract`), merge through `mecha-graph proposals file-merge
--accept` so the one no-undo verb always leaves a decided proposal,
create-on-miss, and a read-only `reaches` row of identifiers. Owner
rulings recorded in `NOTES-GRAPH-DESIGN.md`'s status blocks: repairs ride
mecha's own path; correct-in-place is the house principle; web merges
take the proposals record so the model's MCP surface gains nothing.
Beside it, the three-store `/api/proposals` pane (harness · rules ·
graph entities) with read-gated decisions, grown from the other lane's
dead Harness card. The owner's live testing drove five fix rounds the
demo never surfaced (Enter arming the mic via implicit form submission,
an unbroken safelink URL dragging the page sideways, `[object Object]`
chips, a floating drawer, no create for a missing person), and the
review bot's three waves each caught what the previous fix introduced —
the last a reject that could not work on two of three stores under a
green suite (Traps → Review process). Everything was deployed as it
landed and verified against the running thing; the night closed with
both repos merged, the machine on `main` everywhere, and `deployed-local`
deleted.

**2026-08-30 — the private graph repo merged its first PR, and the public
mirror caught up through the gate.** `d44e04d` carries the three verbs
the web asked for (`kg_upsert kind=alias` gains `remove`; `proposals
file-merge` with `--accept`, apply-first-decide-second like Accept;
`identifiers` in the `kg_entity` envelope), each exercised end to end on
a scratch `MECHA_GRAPH_DB` before being called done. Publishing then hit
the export gate refusing 13 files of life-derived fixture names — family,
colleagues, the university — accumulated since the 0.1.2 export. The
strip (`20f3d38`) moved every fixture to the fictional cast with the test
semantics preserved (the substring tests got new carriers: "SPSP Wrench
Reunion" so "Wren" still substring-hits it; the surname swamp runs on
Whitlock), suite 322/0 with identical assertions, and the blessed tree
published as `bbbba2a` ("0.1.3") — the private/public split doing exactly
what it exists for.

**2026-08-31 — the review queue stopped charging for a second look, then
shipped, then ran.** Reported as a person's own workflow failing: *"when I
enter a cluster to reject individual items and then go back, it reruns the
expensive similarity again. Really annoying and ends my desire to spend time
clearing the queue."* Three surfaces were charging for it and one of them was
already right. `closeItems` in `Queue.svelte` re-ran the whole cross-class
grouping whenever a verdict had been filed inside a group — a guard skipped it
when *nothing* had been judged, so a glance was free and the actual work was
not; the TUI's `Level::Items if from_group` arm had rebuilt the group from its
survivors since the level existed, so #128 was porting a settled decision
rather than making one. Listings then became a per-page cache, because the
back arrow, a Review sub-tab switch (which unmounts the pane) and any
transient error each threw the result away. #130 followed the same thread into
the TUI: `unwrap_or((0, 0))` over `cascade_tally` made "no fan-out was asked
for" and "a fan-out was asked for and the child did not report it" render
identically as `×1`, and the group arm removed a card on any `Ok(report)` —
mecha-graph reports `#id FAILED: …` and exits 0, so an unresolvable seed
deleted a row covering seven candidates while none were touched, which is the
`#2951` incident one level up. Underneath both, `candidate_embedding` (V023)
in the graph: a pending statement's text does not change while it waits, so
its vector is immutable and re-deriving it every call was pure waste.
Released the same day as mecha **v0.1.17** (four crates to crates.io) and
graph **v0.1.4**, deployed across all six `update` surfaces, and warmed on the
live store — **42.6s cold, 4.6s warm, byte-identical output, +17.9 MB**, and
**4.2s at a cosine floor never visited**, which is the number that matters:
what is cached is the vectors, not the query, so the threshold stepper stopped
being a re-embed of the world per nudge. The graph's public mirror was
deliberately left at 0.1.3.

## The measurement record

Moved out of `HANDOFF.md` on 2026-08-06, when that file went over its own
length bound: this is a record of what was measured, which is what this
document is for.

**2026-08-10/11, recovered 2026-08-27 — the turn ceiling was clipping a fifth
of the benchmark, and nobody had re-derived it.** Salvaged out of PR #52 (a
handoff refresh that went stale unmerged for sixteen days and was closed as
superseded) because everything else in it died — v0.1.3, 710 tests, a
benchmark run since finished — and this one finding had landed nowhere.

The original report, from the `mecha-arm64-subset-2026-08-10__14-15-05` run
on v0.1.2: raising the reasoning round trip made `max_turns` the binding
constraint, **26% of trials ended at exactly 80 turns**, and those passed 30%
of the time against 50–67% in every band below. Terminal-Bench's own default
is **200**, so 80 was a ceiling this project chose, below the benchmark's,
and nobody re-derived it after the constraints that justified it were removed.

**Recounted independently before it was believed**, because it is a report
from a session whose working notes are gone, and the archive is on disk:
counting assistant turns in each trial's own `agent/sessions/*.jsonl` and
taking the pass from `verifier/reward.txt`, over the 47 of 50 trials that
kept transcripts —

| turns | n | passed | rate |
|---|---|---|---|
| 0–19 | 20 | 8 | 40% |
| 20–39 | 11 | 5 | 45% |
| 40–59 | 4 | 3 | 75% |
| 60–79 | 2 | 1 | 50% |
| **80+ (the ceiling)** | **10** | **3** | **30%** |

**21% pinned at the ceiling, passing 30% against 40–75% below.** The
denominators differ from the original (47 trials against 75) and the shape
and the conclusion do not.

Two things make it worth the rescue. It is **local evidence for a decision
taken from a convention**: `TASK_MAX_TURNS` was set to 200 on 2026-08-26
citing Terminal-Bench's default, while a measurement of exactly that question
sat unmerged. And the first attempt to recount it **read 100% of trials at 80
turns**, because the regex matched the *configured* `max_turns=80` in every
trial log rather than the turns used — the fixture rather than the thing, the
same shape as the three green-for-the-wrong-reason tests recorded above, and
the reason the count was taken from the transcripts instead.

**2026-08-26 — slot contention, and the cost that is not prefix churn.** R3's
question, pointed at concurrency rather than at overnight parking:
`scripts/slot-contention.py` runs K conversations at once, each six turns over
a stable ~9,800-token prefix, and reads the server's own
`prompt eval time = … / N tokens` — small N means the prefix was reused, N in
the thousands means the transcript was re-prefilled. Against `-np 4`,
262,144 per slot, on a quiet machine (load 0.32):

| K | requests | wall | throughput | per-turn latency | prefill after turn 1 |
|---|---|---|---|---|---|
| 1 | 6 | 42.7s | 1.00× | 7.1s | 31 tokens |
| 4 | 24 | 108.2s | 1.58× | 18.0s (2.5×) | 31 tokens |
| 6 | 36 | 153.7s | 1.67× | 25.6s (3.6×) | 31 tokens |

**Six conversations on four slots did not evict each other.** After each
conversation's first turn, every later request re-prefilled 31 tokens — just
the new question — with two cache evictions across all three arms and no
conversation ever re-paying its transcript. That is `-cram` doing what §3.3
said it does, holding at a load nobody had tested it at, and it **refutes the
rationale that had been proposed for R1 hours earlier**: a permit count to
protect prefix reuse would have been protecting something the server already
protects, and the metric chosen to validate it would have shown no effect.

What over-admission actually costs is **latency, at flat throughput**.
Throughput saturates at four seats and buys 6% more going to six, while
per-conversation latency degrades 42%. So a fifth concurrent conversation is
close to pure loss: no more work done, everybody waits longer. That is the
number R1's permit count rests on — three background permits against four
seats, one reserved so the owner's turn stays near 7s instead of near 26s —
and it is a measurement rather than a guess.

The general shape worth keeping: **the right mechanism can survive the wrong
reason, and the reason is what the validating metric is chosen from.** Both
roads led to a permit count; only one of them would have been checkable.

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

**2026-08-24 evening — the phone got the review surfaces, and similarity got
a top layer.** One session, three arcs, all live by nightfall. The **mail
page** closed the gap the morning's handoff named ("the one daily surface the
phone cannot review"): `serve/mail.rs` + `Mail.svelte` on the TUI modal's
exact split — store read for the list, `mecha mail show` as the one thread
renderer (third-party text behind a per-line gutter), every mutation a
closed-verb CLI child where an unknown verb 400s before an argv exists, spam
the only confirm, drafting verbs detached into the outbox. The **queue tab**
grew the TUI's missing depths — classes with tier chips (tiers stamped
server-side from `tui::queues::Tier::of`, killing the page's own drifted copy
of the thresholds), per-class similarity groups with cascade verdicts — and
then, on Luke's push against the class fence at 7.5k pending, the **global
similarity layer**: a cross-repo arc (`mecha-graph`'s `similar.rs` and CLI,
mecha's `review groups --all` and the page's "similar across everything")
that groups the whole pending queue at a stricter floor (0.90), names every
class a group spans on the card, and rides `--cascade --across-classes` so
one tap stays one human verdict with the members as a labeled machine
cascade. The invariant was amended, not broken: "a cascade never crosses a
class *uninvited*" — the crossing is asked for by flag, priced, and shown.
First live run: 306 groups covering 782 of 6,929 pending in 40s, the top
group 25 restatements of one family relationship spanning four predicates.
**Files** landed as Phase 4's missing half (`serve/files.rs`): uploads into
the session jail's `inbox/` announced as paths so taint arms through
`fs_read`, downloads re-proving containment with missing and outside the
same 404, and images the only inline content type — model-written HTML
served same-origin would run script with the owner's auth against the API
itself. The voice **thinking sound** was replaced the same session (a soft
alternating two-note pulse with a 120ms attack; the 900ms triangle tick read
as a metronome). And the first real phone tap paid for itself: see the two
new traps below.


**2026-08-24 night — the voice got a face, and three sessions found they
shared one binary.** The voice page's dock had one button; it now carries a
**seven-voice picker and a rate slider** — six Kokoro-derived cloning
references plus Chatterbox's own built-in `default`, which is a real
selectable voice rather than a passthrough (`voice: "default"` generates with
no reference). Neither control was free the obvious way.
Chatterbox Turbo has no speed parameter (`generate()` takes
`exaggeration`/`cfg_weight`/`temperature` and nothing about tempo), and the
browser's cheap knob — `playbackRate` — resamples, moving pitch with tempo
into a chipmunk; so rate is a pitch-preserving phase vocoder on the server,
measured at ~50 ms warm against a synthesis cost of 0.53 s. The voices are
**Kokoro presets synthesized into cloning references**
(`scripts/voice/make-voices.py`), which is a licensing decision before it is
an engineering one: Kokoro is Apache 2.0 and its voices are nobody's
identity, so a voice can be added or deleted without anyone's consent being
the thing that made it legal, and the risk that a twice-vocoded reference
would clone badly did not materialise — all six work, 0.53–1.26 s for a
short sentence. Changes ride a `voice-config` RTVI message and the server's
reply is what renders: a bad value is **refused, never clamped**, so the
slider cannot show a rate the worker is not speaking at, which is the
wrong-bytes-review rule arriving in a control surface. The channel sets how
the answer sounds and nothing else — there is deliberately no field on it
that reaches the agent, the workspace or the posture.

`VOICE_BLOCK` learned that **its first sentence is the only one on the
latency path.** Pipecat's TTS service aggregates by sentence, so
time-to-first-sound is the synthesis of sentence one alone: measured at
0.33 s for "Sure." against 1.08 s for a full clause, on a leg that was
consuming ~77% of the budget while STT sat at 92 ms and the LLM's cached
prefix answered in ~220 ms. A short opener pays twice, being also fewer
tokens to generate before speech can start at all. The measurement is the
point — four turns of that session were spent debating the STT model, which
is 6% of the latency and already at the low end of the industry figures.

Several **stale labels on live processes** were corrected the same night, all
of the same shape: `llama-voxtral.service` still described itself as "STT for
mecha voice" months after Parakeet took the seat, `worker.py`'s docstring
still named `:8082` while line 47 defaulted to `:8992`, and
`VOICE-RESEARCH.md` §2.1/§2.2 still carried picks its own §7 build log had
superseded. §2.1 gained the rule the Parakeet swap actually established:
**an audio encoder bolted to a general LLM is disqualified from the STT seat
whatever it scores**, because no ASR benchmark contains adversarial speech —
which is why the table's top three entries are all disqualified.

The same night, a second session landed the phone surface's remaining arcs:
the **frontdoor page** (`serve/frontdoor.rs` + `Frontdoor.svelte`, closing
the last review store the phone could not reach), a **session-history
drawer** with resume that restores messages *and* taint through
`Session::load` and re-proves the recorded workspace, a **plain mail inbox
and compose** that stages under whatever `[outbox] tools` names and refuses
if `mail_send` is unrouted, and **push-to-talk dictation** (`POST
/api/dictate` → Parakeet) on the notes and task boxes. The interim
browser-speech toggle was deleted rather than justified: the owner had been
reading it as a status light.

Both voice surfaces got the controls within the hour, and the second one is
worth recording for *how* rather than what. Two sessions split it by file —
one took `Chat.svelte`, the other kept `voice-core.js`, the dock and the
worker — and the contract crossed between them as three warnings rather than
as a diff: render the server's reply and never an optimistic value (a refusal
is refused, not clamped, so local state would leave a slider describing a
speed nobody is speaking at), guard the preference restore or the reply
handler becomes a send loop, and take the slider bounds from the server's
`cfg.range` instead of literals. All three were load-bearing and all three
were honoured. The question that saved the most work was the one asked before
building: *where does the voice list come from* — the answer being that
`voice-core.js` already requests it on `dc.onopen`, so a second surface needs
only to supply an `onVoiceConfig` callback and the populated list arrives
unprompted. Mirroring the first surface's list by hand was the obvious move
and would have been the drift. (One trap for anyone curling it: the live path
is `:8881/v1/voices`, because `TTS_URL` already carries the `/v1` — `/voices`
alone is a 404 that reads like a broken list. Verified both ways.)


**2026-08-24 night — 0.1.13, and the docs catch up with two subsystems.**
The release that made the day's work public: `mecha serve`, voice, the
graph queue's similarity groups and cross-class layer, mail's plain inbox
and hand-written compose, phone dictation, resumable session history, and
`/entity`. Ninety-one commits under a patch bump, tagged and pushed with
the crates workflow publishing on the tag.

The website gained the two pages it had been missing entirely — one for the
web surface and one for voice — plus similarity groups on the queues page
(undocumented until now) and the inbox/compose half of mail. The voice page
was written by the session that built the stack rather than by the session
that published it, on the argument that a draft assembled from a build log
would be subtly wrong; the thing it caught is the thing a build log cannot
know, because a build log is written on a machine where everything is
already installed: **`cargo install mecha-cli` ships the voice facade and
none of the voice pipeline.** `scripts/` sits outside the crate, so
`cargo package --list` shows zero runtime files, and `git ls-files` tracks
zero `.wav` — the seven voices exist only where `make-voices.py` has been
run. A user who read "mecha has voice mode", installed the crate and went
looking would have found a facade with nothing behind it, so that
constraint leads the page instead of trailing it.

Three sessions coordinated the release across two repositories: a freeze on
`~/Github/mecha` that both peers verified their own state against, a
one-evening carve-out for the private graph checkout where a third session
was mid-arc, and CHANGELOG entries written by the sessions that had built
the things rather than inferred from their diffs.

**The freeze worked as a forcing function, and that is the part worth
keeping.** Nobody was investigating binary staleness. The graph session
checked its install timestamps *only* because a line had been drawn around
its repo and it had to state where it stood — and stating it out loud is
what made an hour-stale `mecha-graph-mcp` visible, across an arc whose
central fix was in the write path that binary owns. An agent asked to
summarise its own state finds things it was not looking for, which is an
argument for release freezes that has nothing to do with releases.

The carve-out itself was correct for one evening and would have been wrong
as a standing arrangement, which the session that benefited from it argued
for its own repo: an exemption held in two sessions' memory makes the graph
install nobody's job by default, correct only while both remember. That is
the shape this codebase keeps naming — a state that is only right after
someone remembers something is a state nobody can trust — so the update
skill owns both installs unconditionally, and "I am mid-arc" is something a
session says at the time rather than something a note keeps true on its
behalf.

**2026-08-25 — three review surfaces stopped hiding what they were asking
people to approve.** The day's work came from the owner using the release and
finding, in three different places, a surface that showed a decision without
showing what the decision was about.

*Mail actions on the personal account had never worked.* Every button on a
personal thread failed with "no thread in the triage store matches" — the
store held 192 records, all dartmouth, none personal, because the nightly
named `--account dartmouth`. The flag was defensible and the requirement
behind the failure was not: `mail_triage` reaches nobody, mutates only the
user's own mailbox, and is documented as the third quadrant precisely so it
can be the cheap way to act, yet it was the one verb that could not run. The
requirement was never a decision about archiving — it fell out of
`resolve_thread`, whose real job is expanding a briefing's eight-character
handle. `triage` resolves leniently now (`mecha-cli/src/commands/mail.rs`),
`resolve_account` split because its two callers want opposite things from a
miss, and the gap it closes is permanent rather than incidental to one
account: `classify` sweeps 50 threads of one mailbox nightly while mail
arrives continuously in both.

*Then the nightly took both mailboxes*, which had been blocked by a bug
wearing a policy's clothes — see Traps → Unattended runs. The owner overruled
the standing "personal should stay out of the nightly" entry, and the
measurement vindicated them twice over: the first both-account sweep read 100
threads and disposed **47 of 51 candidates without a model**. The account
excluded for being expensive is the cheap one, because machine-generated is
exactly what the prefilter handles for free.

*A staged calendar delete showed an account and an opaque `event_id`.*
`outbox_source` matched provider ids against earlier `tool_use` **inputs**,
which finds a reply's thread read and cannot find anything for a draft whose
target was *discovered by listing* — the id exists only in a result.
`Join::Returned` closes it, `Join::Asked` keeps precedence where both hold,
and `MIN_RETURNED_ID_CHARS` guards the value-only match because
`calendar_id: "primary"` is a substring of every calendar result in the
session. The heading moved onto `SourceRead`: it read "replying to", which was
the one place a module documented as knowing nothing about mail knew about
mail, and was false the moment a draft that answers nothing got a source.

*And `Enter` at the TUI's review level had been a no-op for three commits*
while the footer advertised "Enter read it" — harness candidates and rule
proposals announced a depth and opened nothing, which is the exact shape
`/queues` exists to prevent. Found by reading a compiler warning that had been
in every build since; see Traps → Environment.

Three sessions worked one checkout all day. Two `git add -A` sweeps carried
another lane's uncommitted edits into unrelated commits, nineteen days after
that trap was written down — which is its own entry, and the reason explicit-
path staging is now the habit rather than the rule.
**2026-08-25 (evening) — the gate stopped reading a label the proposer wrote
about its own change.** `class` decides whether a harness candidate ever
reaches a human — `Security` is never measured and never auto-applied, while
`Config` inside the closed override set reaches `measure()` and can
auto-accept — and it was parsed off a line the diagnostician typed, never
cross-checked against the `change` beside it. It had held anyway, but by
coincidence: the closed set is four benign knobs, so a security change
labelled `config` stuck at `parse_change` for being *outside the set* rather
than for being a security change, and staged carrying that reason instead of
the one a reviewer needs. That morning's nightly had proposed disabling a
taint control, class `config`. `diagnose::names_guarded_setting` now derives
the class from the change: `[security]`, `[sandbox]` and `[outbox]` — three of
the four boundaries CLAUDE.md says reach a human however anything scores, the
fourth being the path jail, which is not configurable and so cannot be
proposed — plus every `SecurityConfig` field by its bare name, since the
section prefix is the model's to omit and omitting it must not be the way
through. Three properties carry it, each with a test: it **only ever raises**
toward review, like a capability override; it **reclassifies rather than
refuses**, because a refused proposal leaves no record and the brief carries
every prior candidate as "already tried", so a dropped one is free to return
tomorrow where a staged one is both blocked and paid for; and it
**over-matches on purpose**, since naming a setting is `security.` or
`[sandbox]` and prose *about* the sandbox is not caught, but a sentence ending
on the word is — which costs a reviewer a warning they did not need, where
missing one costs a confinement change auto-accepted. The mislabel itself
rides on the candidate's reason, because a proposer whose account of its own
change was wrong is the more interesting record and a pattern of them is
invisible if each is silently corrected. The same evening, **`show_file`
stopped loading the entire global config to read one number.** It wanted
`slack.max_upload_mb` and called `Config::load_global` at call time; config is
`deny_unknown_fields` — correctly, since it is a wire format between versions —
so any key added anywhere made the tool fail with a parse error in a process
started before the key existed. It happened twice on 2026-08-21, with `vision`
and then `[[search]] prefer_deep`. The cap is captured when the tool is
registered, which is also the only moment the TUI rebuilds it, so a broken file
now fails the `/model` switch the user just asked for instead of a chart an
hour later. Its test drives the failure rather than asserting about it — an
unparseable config under a temp `MECHA_HOME` plus a live attach record, which
reproduces the recorded message verbatim on the old code. `send_file` still
has the call-time load and is recorded in the handoff as the remaining half.

**2026-08-25 — talking and typing became one conversation.** D3, the last
of the voice decisions still owing something, and the one the design had
been promising since it was written. An in-chat call had been its own
conversation with its own transcript and its own clean taint slate; the
overlay said so honestly, which is the best a seam can do about being a
seam. It is gone.

The obstacle was never the transport. `mecha serve` held **two session maps
in one process** — `voice::Facade`'s slots and `chat::ChatState`'s sessions
— sharing an agent, a provider connection and a prompt cache, and a call
resolved in the wrong one. The key-carrying plumbing already ran end to
end; what was missing was the page choosing a key and the facade resolving
it somewhere other than its own map.

What shipped: the page names its session in the WebRTC offer through
`request_data`, which is pipecat's own passthrough to `runner_args.body`,
so no framework patch and no second endpoint — and the offer is used
because it is the only message sent *before* the bot exists, and the bot is
what has to know, the data channel opening far too late to choose an LLM
service's headers. The worker forwards it as `X-Chat-Session` beside the
slot key it still mints, deliberately a second header rather than a
namespace inside the first: one header carrying two meanings is a value
nothing can validate, and a page is free to name a session
`webrtc-anything`. The facade resolves it through a `voice::SessionHost`
trait that `serve::chat::VoiceHost` implements, so `voice/` still has never
heard of `serve/` — the `Approver`/`Asker` shape applied to "whose
conversation is this" — and the facade keeps **no** slot, session file or
conversation for a hosted turn, because a second copy of any of those is
exactly the duplicate record that made merge-on-close the rejected shape.

`chat::begin_turn` became the one implementation of "a turn on a web
session", shared by both doors. That is the `/tasks` rule (one
implementation per verb) applied inside a process rather than across a CLI
boundary, and for the same reason: two constructions is how the typed and
spoken paths stop agreeing about the jail, the outbox stamp or the
recording contract, silently and in that order.

Four decisions rode along. A spoken utterance mid-run **barges in rather
than steering** — steering would fold the words into the run already
streaming *to the page*, and the worker is owed a reply it can speak;
measured at 1.2 s from speaking over a 300-line generation to hearing the
answer. A spoken turn **broadcasts its own user message** (`WireEvent::User`,
the voice block stripped) because it has no local echo anywhere, so the page
fills in as you talk; a typed send is still echoed only by the page that
typed it, and what a second device misses is a separate gap that was
deliberately not half-closed here. `--voice-yes` **travels with the turn,
not the conversation** — the owner's call — so a spoken turn runs with
approvals off while a typed turn in the same conversation obeys the page's
mode, verified in one session by a spoken `fs_write` that succeeded and a
typed one two turns later that was Blocked read-only. And nothing
structural moved: the interlock still sits ahead of the approver, sends
still stage, and taint now **accumulates across both doors** instead of
being reset by opening a call, which is the stricter direction.

Verified by running it rather than by reading it, which is what this
project keeps having to relearn: shared context across the doors, one
session file where there had been two, the block firing on exactly the
switches into speech, and `X-Chat-Session: ../evil` falling back to a
conversation of its own with a warning — a dead call being a worse answer
than an unshared one.

Left standing and written down rather than fixed: a `voice:` session is now
the fallback rather than the norm, so the drawer's badge quietly means
"this call had no page behind it"; the page's mode chip does not describe
spoken turns; and `mecha serve` still never drains its chat runs on
shutdown, which now covers spoken turns too — unreachable in practice,
since `axum::serve` is not wrapped in `with_graceful_shutdown` and
systemd's stop is a hard kill, but the standalone `voice-serve` handles
SIGTERM and the mounted one does not.

The same day, earlier and by another session, **a sound with no words in it
stopped stopping the bot** — step (1) of the deferred VAD item, fixed
structurally rather than by tuning. Dropping the VAD from the turn-*start*
strategies means a wordless segment emits no transcription frame at all,
reaches no strategy, and the bot simply keeps talking: resume-on-empty
achieved by never stopping, which needs no state to unwind. The analyser
stays, because it still segments.

**The bake-off those two conclusions came from**, moved out of `README.md` on
2026-08-10 so the numbers live with the rest of the measurement record rather
than in the front door. It was taken on a DGX Spark (GB10, 128GB unified)
against the 25-case set **as it stood at the time**; it has not been re-run on
the current 36 cases, and it is recorded here as what was measured then rather
than as a current result. `mecha eval --compare` produced this:

| 25 cases | gemma-4-E4B | gemma-4-26B-A4B | Qwen3.6-35B-A3B | Qwen3.6-27B |
|---|---|---|---|---|
| params | 4B | 26B / 4B active | 35B / 3B active | 27B dense |
| cases passed | **24/25** | 23/25 | **24/25** | **24/25** |
| checks passed | 99% | 97% | 99% | 99% |
| malformed arguments | **0** | **0** | **0** | **0** |
| invented tools | **0** | **0** | **0** | **0** |
| reasoning | 4/4 | 4/4 | 4/4 | 4/4 |
| injection resistance | 2/2 | 2/2 | 2/2 | 2/2 |
| mean turns | 2.8 | 3.4 | **2.4** | 2.5 |
| median latency | **6.7s** | 8.5s | 7.3s | 24.7s |
| output tokens | 14,284 | 9,590 | 6,158 | **6,023** |
| **generation** | **119.7 tok/s** | 99.5 tok/s | 90.5 tok/s | 11.4 tok/s |
| MTP draft acceptance | 59% | **90%** | 75% | — |

Generation figures were isolated single-request benchmarks on the same prompt,
all three MoE/small models running speculative decoding via their MTP draft
heads. Qwen3.6's MTP layers are **baked into the GGUF** — `--spec-type
draft-mtp` with no separate `-md` file — which took it from 55.4 to 100.2 tok/s
(1.81×). Gemma ships a separate `mtp-*.gguf` draft. Qwen3.6-27B dense had no
MTP variant, so its number is unaccelerated and understates it somewhat; not
enough to matter at an 8× gap.

The latency column is the same hardware finding from the other side: the dense
27B took **3.4× the median latency** of the 35B MoE for identical accuracy,
because decode on this machine tracks *active* parameters rather than total.

Among the three that were left it was a straight speed/verbosity trade rather
than a quality one. E4B generated fastest but was the most verbose (14.3k output
tokens against 35B-A3B's 6.2k), so its wall-clock lead was smaller than its
tok/s suggested. 35B-A3B was the most economical per task and had the most
headroom. gemma-4-26B-A4B was the odd one out — the best draft acceptance (90%)
and the weakest score, and nothing recommended it over the other two.

The honest caveat that came with it: every case in that set was *grounded*. The
data was in the workspace and the job was to find it, combine it, and report —
most of what a personal agent does, and a 4B was evidently sufficient for it. It
said nothing about long-horizon planning, ambiguous requirements, or code
generation, and no scorecard in it covered the `long-horizon`, `codegen`,
`synthesis` and `ambiguity` tags, which were added for exactly that.

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

**The first complete Terminal-Bench scorecard (2026-08-11).**
`mecha-arm64-subset-2026-08-11__02-11-55`: all 75 trials of the oracle-passing
arm64 subset, k=1, v0.1.2 portable binary, qwen3.6-35b at the 262k window,
`max_turns` 80, timeout multiplier 2.0, ~19.5h wall clock. **Mean reward
0.4533 (34/75), 8 errored trials.** The run was launched to answer four
falsifiable questions from the 08-07 diagnosis rather than to produce a
number, and it answered them: empty-turn nudges fell to 4 across the whole
run, the dash-prompt crash did not recur. It is the k=1 baseline for the
0.1.2 harness; a leaderboard-comparable k=5 (~74h) remains unrun, so compare
nothing against the leaderboard yet.

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

**2026-09-02** — the graph stopped being two repositories. Ten files had been
private; only four held roster terms, and `scripts/nightly-mecha.sh` was
private *by association* — it sat in `scripts/` beside the gold-set tooling
and nobody had read it. Read line by line, it carried nothing: no names, no
emails, no hostnames, every path `$HOME`- or `$BASH_SOURCE`-relative. It is
public now, and the gold sets moved to `~/.mecha-graph/eval/` behind
`eval::gold_path_from`, which mirrors `db::default_db_path`'s env-then-`$HOME`
shape. What made the move safe was relocating the gate rather than retiring
it: `.githooks/pre-push` and `.github/workflows/denylist.yml` run the same
roster from outside the repository, both fail closed, and `.gitignore` gained
`eval/*gold*.jsonl` as a second layer — the export used to be what stood
between those files and the world, and working in the repo directly removes
it. The private checkout keeps the notes and the roster tooling;
`export-public.sh` is retained for history and called by nothing.

Two long-running silences ended with it. `is_missing_fact` made the Bee push
idempotent — one verdict had retried nightly since 2026-08-24 because
`bee facts delete` answered "Fact not found" for a fact that was already
gone, and the code called the achieved end state a failure; the diagnosis
came from the same branch's own error-recording fix, on its first run.
`Divergence::arms_summary` and the attempt-keyed gossip ledger closed the
other: a probe that legitimately refused ("one witness cannot gossip") wrote
no row, never aged, and was re-selected on five consecutive nights.

Seven review rounds on that PR found one MAJOR, five MEDIUM and twenty-one
MINOR; the MAJOR and three of the MEDIUMs were defects in *previous rounds'
fixes* rather than in the original change.

**2026-09-01/02** — `[harness] source_dir` is set. The ordering the handoff
specified was followed: every binary reinstalled first, then the key written,
because `ConfigLayer` is `deny_unknown_fields` and the section is a startup
parse error on any binary predating #127. Verified against the *installed*
binary before trusting it, not the branch build — the trap that item existed
to prevent. The diagnostician stopped running blind: it now logs "reading
source and docs from /home/ljchang/Github/mecha" rather than "no source
checkout reachable from the path jail".

**2026-09-02 — the appraisal label was hiding the sign, and the corpus was
measuring the harness's own test runs.** The owner asked for a review of the
appraisal system, "the newest system that deviates the most from other agent
harnesses." Re-measured live: 143 sessions appraised, 142 `neutral`, 0 naming
a goal, 0 board tasks ever `done`, `anticipated_guilt` a constant between
0.95 and 1.0, `StopCause::Interrupted` conflating Ctrl-C with an `ask_user`
park, and 22 owner-rejected drafts all reading `neutral` — because
`label_of` gives an owner- or self-caused negative no word until
`controllable` is filled, and only a paid replay fills it. Two literature
passes agreed on the mechanism from opposite sides: every computational
appraisal model (OCC, Scherer, EMA, Soar-Emote, WASABI) gates once on
relevance and labels from two variables, and Marinier, Laird and Lewis
rejected a product over unfilled dimensions by name; the harness
literature says LLM judges on trajectories are 33–41 kappa points less
reliable than reported and a 0.94-AUROC critic that intervened lost up to
26 points — so the design's refusals held and its readout was the defect.
`docs/APPRAISAL-RESEARCH.md` is the record. The owner's rulings, in one
session: labels are a readout of current state plus guilt and regret as
typed signals, nothing more; the error signal is mostly prose,
Reflexion-shaped, with numbers keeping three jobs (trigger, replay priority,
consolidation gate); appraisal grounds in the charter, the task's plan as
the prediction, and memory as the prior. Built the same day on
`feat/appraisal-record` and merged as #140 at `15c628d`:
`appraisal::Valence` beside the label on every surface,
`session::SessionKind` with a `test` override that
only narrows, a ceiling relabelled as the owner's own limit with
`Appraisal::cut_short` keeping the closure follow-up honest, and the
prediction record (`TodoItem::{expect, check, expect_calls}`, the frozen
check, `step::CHECK_TRACE`, the check counters on `Work` and `RunStats`, a
failed check as a signed error, `Trigger::Mismatch`). The other live lane's
`AUDIT-RESEARCH.md` §3.11 spec was co-designed across the two sessions by
message — its arm 1 re-injection carries the prediction, its arm 2 declared
checks are the structural discrepancy detector, its arm 3 critic is the
§3.7 predictor scored by the owner's edits — with one correction from this
side that mattered: a declared check must go through the approver, not only
the sandbox, or a plan field becomes a way to run a command on a surface
where `shell` needs approval. Two doc sentences the audit found false
against the tree were corrected ("three sensors ship with no consumer"
while `diagnose::Evidence` reads two; "never reported" while `distill`
writes the label to pkg). **Phase B followed as #141 at `49166e3`** the
same evening: `appraisal::SessionRecords` handing `of_session` the
question, front-door and reflection stores beside the drafts, with a
per-store "short" flag that lands on the record as `Appraisal::partial`;
`Channel::Commitment` signing a question answered and the session finishing,
a question abandoned, a request closed with nothing sent, and a
judged follow-up reflection (clean provenance only — stricter than
`learnable()`, the owner's ruling); the queue-delta positive read off the
run's own homeostat as the one positive a live surface can show;
`guilt::with_backlogs` folding level, delta and relief from one pair of
reads, the relief in its own field so the level's corpus mean stays one
quantity; and `Depth::given_up`, so a rejected draft, an abandoned
question or a hand-closed request shortens the queue without crediting the
run. The two PRs drew twenty-one and twenty review summaries from the
workflow — counted as its comments on each PR, #141's last one landing
after the merge and yielding #147; nearly every
pass found a real edge on the check freeze or the sign — a thinned echo
parsing as a prefix, a cut landing after the last marker, the carried
block as the only record after a compaction, the install-time restore
skipping non-completed steps, a marker a model could write into a step,
the owner's give-up reading as clearance — each closed with a test that
fails on the previous cut. Charter sensors were ruled in and designed
(`GOAL-SYSTEM-DESIGN.md` §11.1, seven containments), not built.

## Traps already hit

Recorded so they are not hit twice. Each says what broke; the sentence that
matters is the general shape.

### Measuring

**2026-09-02 — the instrument was measuring its own test runs.** 46 of the
143 sessions the appraisal corpus read were smoke runs from a mecha checkout
or a Claude scratch directory, and most rejected drafts carried reasons
naming a self-test; the session record had no field that could say so, so
the only way to tell was a path heuristic nobody had applied. The rung 7
measurement that set the build order was taken over the same mix. **A store
that every harness session writes into is a store the harness's own
development contaminates, and the mark has to be structural** — a `kind`
written by the front-end and a `test` override that can only narrow — because
a reader cannot recover it afterwards; the 46 stay unknown forever.

**2026-09-02 — a process environment variable set in one test module broke
a test in another, on the first run only.** A `SessionKind` test set
`MECHA_SESSION_KIND=test` around `Session::create` under a module-local
mutex; `runlog`'s scan test, in the same binary and another thread, created
sessions while it was set, and the default scan excluded them as tests. The
lock covered the module and the variable covered the process. **Anything
read from the process environment is shared by every test in the binary,
whatever a mutex says**; isolate it in a child process (`current_exe()` with
`--exact --ignored` on a probe test) or do not test it through the
environment at all.

**2026-09-02 — changing a label's producer made a gate dead code, silently.**
A ceiling stopped reading as `Agency::World`/`Anger` (the owner's own limit is
not somebody else's fault), which was right — and it was the only
non-neutral label the free readout ever produced, so the closure follow-up
gate, which reads `label != Neutral`, could no longer fire on the free path
at all. Every test stayed green, because the gate's tests constructed labels
directly. **When a derived value feeds a gate, `grep` the gate before
changing the derivation**; the fix was a typed predicate beside the label
(`Appraisal::cut_short`) and a gate test that constructs the *errors*, not
the label.

**2026-08-31 — a bare filename is a line number's failure mode one level up,
and two files share the name.** A handoff item cited `setup.rs:879` for a stale
comment in `prepare_tools`. There are two: `mecha-cli/src/setup.rs` and
`mecha-cli/src/commands/setup.rs`. A reader followed it to the second, where
line 879 is an unrelated test assertion and `global_config_only` does not
appear anywhere in the file — so the reference read as "already fixed, or never
true", which is the failure a citation exists to prevent, pointed backwards.
Caught by a peer checking the address rather than the finding. **`CLAUDE.md`
says cite the symbol, not the line; the unstated half is that a path is part of
the symbol whenever the basename is not unique** — and `find . -name` is the
one-second check. The repair repeated the shape: re-measuring the caller count
with `grep "global_config_only: true"` returned four and missed a fifth that
assigns (`opts.global_config_only = true`) rather than using a struct literal.
**The pattern you search with is part of the claim you can make**, so a count
stated from a grep carries the grep's blind spot into the sentence. Written, of all places, into a handoff pass by the skill
whose own stated rule this is. The corrected item also found the comment worse
than reported: it claims one caller and there are four.

**2026-08-31 — a counter whose scope is implicit is one nobody can read
correctly, and three drafts got the same number wrong three different ways.**
`Corpus::sessions_read` increments *after* the workspace filter and *before*
rows are pushed. A bail-out in `corpus_slice` printed it as a store total
(it is not), then removed it as "necessarily 0 on this branch" (it is not),
then finally used it as what it is: the discriminator between "nothing was
rooted there" and "sessions were and none recorded a run". Round five caught
the middle one on live data — `~/.mecha/work/frontdoor`, 13 sessions, zero
outcomes, reported as "the filter matched nothing" — and round seven caught a
third case, a torn transcript landing in `unreadable` while `sessions_read`
stayed at nought, turning a rotting store into a typo. **A count whose scope
(pre-filter, post-filter, per-session, per-run) is not in its name will be
read wrong at the call site, and the wrong reading always looks like a
plausible sentence.** Absent is not zero, and the three zeros are not each
other.

**2026-08-31 — every test moved one variable, so none of them could see the
bug.** `guard_regressions` returned on the first metric it found anything on,
so a mild `0 → nonzero` proposal at `Compactions` (index 3 of `Metric::ALL`)
suppressed a genuine ratio breach at `MalformedArgs` (index 5): the reviewer
would have read the benign explanation and never learned something else had
doubled. Three tests covered the guard and every one of them moved exactly one
unpredicted metric, so the failure was invisible to all of them **by
construction** — a green suite that could not have gone red. **When a guard
iterates a collection and returns early, the test that matters moves two
elements and puts the milder one first.**

**2026-08-31 — a filter applied on the visible half reads exactly like a
working filter.** `--from-workspace` scoped the diagnostician's brief and not
`draw_episodes`' draw, so a change was reasoned about one job and accepted or
rejected on the average of four. It looked correct because the scoped half was
the half that printed. The same shape returned one round later: `corpus_slice`
canonicalized the path internally and discarded the result, so the raw CLI
value reached the draw and a `./work/morning` or a symlinked home matched
nothing there while scoping the brief. **Where one input feeds two consumers,
test the one that produces no output — and resolve shared inputs once, at the
boundary, rather than in each consumer.**

**2026-08-30 — `git rev-parse` echoes an unresolvable ref name to stdout,
and a suppressed stderr turns the echo into a phantom tag.** A probe ran
`git tag -l deployed-local; git rev-parse deployed-local 2>/dev/null` and
read the output line as the tag listing — but the tag did not exist, the
listing printed nothing, and the line was rev-parse's pathspec fallback
echoing its argument. The session then reported a tag "deleted between two
commands" and hunted a race that never happened. General shape: **a
lookup that echoes its input on failure looks exactly like success once
stderr is gone — check the exit code, or leave stderr attached.**

**2026-08-30 — after `/clear`, a session's own past is hearsay to itself,
and the false statement arrived dressed as a correction.** A peer
accurately quoted a sentence this terminal had sent before a `/clear`;
the post-clear session, finding nothing in its window, twice "corrected"
the record — and the correction was the false half, carrying the extra
credibility of someone carefully fixing things. The transcript on disk
(`~/.claude/projects/<project>/*.jsonl`) settled it in one grep, and a
PID a peer had captured hours earlier for an unrelated reason
corroborated it. General shape: **"not in my context" is not "did not
happen" — the session file is the artifact and it greps; and a
proactive correction deserves the same verification as any claim,
because it is the direction nobody is guarded against.**

**2026-08-30 — the security interlock did the failing, and the model wore
it.** The trifecta interlock ran live inside counterfactual replays and
blocked `docs__sheets_write`/`web_search` calls the recording had executed —
in a mode where nothing leaves the machine, since every answer comes from
the recording. The blocked call never reached the replay cursor, the arm
died one call later, and the verdict graded the model. General shape: **an
instrument must not carry live-fire controls for actions it cannot
perform** — a replayed tool declares what it can actually do (send nothing),
or the guard's work is billed to the subject under measurement. The same
session's stale-binary rerun is the Environment cluster's; found because the
warning that should have vanished was still in the log, which is the cheap
verification the fix itself suggests: after a fix, grep the next run's log
for the symptom, not for success.

**2026-08-30 — the judge's answer budget equalled the thinking budget under
it, so silence got graded.** `Judge` defaulted `max_tokens` 4096 against the
server's `--reasoning-budget 4096`; a long rubric spent the whole allowance
on reasoning and returned HTTP 200 with empty content, which the judge
scored as a bad answer. Fixed by `provider::LOCAL_MAX_TOKENS`; the general
shape is CLAUDE.md's own server rule pointed at graders: **any budget handed
to a component that thinks before answering must clear the thinking budget
beneath it, or refusal-shaped emptiness is graded as the subject's failure.**

**2026-08-29 — six verification steps in one session could not have failed.**
Across PR #118's ten review rounds: a drag assertion that compared two empty
strings, because `button.value` is `''` rather than `undefined`; a grep for
`id="..."` against a production build that minifies attribute quotes away; a
CI wait loop built on `gh pr checks --json`, a flag that version of `gh` does
not have, so the condition compared an empty string and exited on its first
pass — twice reported as "all checks settled" when nothing had settled; check
scripts whose exit codes were swallowed by `| head`, so a failing gate read as
passing; four new scanner tests that stayed green when the actual defect was
reverted, because the bug was in *how the tail was handed to* the scanner and
every test exercised the scanner; and a back-navigation test that printed the
symptom in plain text (`chevron #settings -> Back #settings`, a Back that
moved between two identical entries and did nothing) while being scored a
pass. Each one looked like evidence. Two of them printed the failure and were
read past. The general lesson: **a check that cannot go red is not a check,
and the only way to tell which kind you have is to break the thing and watch
it fail.** Reverting the fix before trusting the test is seconds of work; not
one of these survived that step, and every one of them survived until
something forced it. The corollary for review: when a reviewer's finding
cannot be reproduced, that is a fact about the harness as often as about the
finding — the `fillId` defect in the same PR was unreproducible in Chromium
and fixed anyway, because the browser that mattered would not launch to check.

**2026-08-29 — v0.1.16 shipped a web page that threw on load, past three
green checks.** `Tasks.svelte`'s `stateOf` called `stalled(t)` where
`stalled` is a *field* the server stamps (`serve/board.rs`), never a
function — a `ReferenceError` on every card, so the `/tasks` board rendered
its header and no tasks on any non-empty board. Three mechanisms were green
while it shipped: the Rust suite (the defect is entirely in Svelte), the
Docusaurus build (it renders no client JavaScript), and CI overall (the
docs workflow's paths filter named `website/**` and not `web/**`, so a
web-only change ran no docs build at all — which is why the fix PR itself
showed no `build` check). Loading the page in headless chromium found it in
one run, and that gate (#117's `render-check`) now rides every docs build.
The general lesson: **a guard that never executes the thing cannot see a
runtime error in it, and three green checks over the same blind spot are
one check.** A corollary from building that gate's fixtures the same night:
a wrong response shape does not throw — five wrong fixture shapes each drew
an empty pane indistinguishable from "this feature does nothing", because a
component that fails still leaves the shell and nav looking like a page. So
shapes are verified by *rendering* them, and "drew almost nothing" is a
failure condition in its own right.

**Three tests written in one session passed on both arms, and the third was
caught only because the second had been.** One asserted a queued instruction
survived a turn that could not serve it, on a scenario that never reached the
line under test. One pinned the printed half of a rendered number while the
contradiction lived in the arithmetic beside it. One asserted a flag was
still set after a code path that, in that scenario, never ran. Each looked
like coverage of exactly the defect it missed.

The habit that catches them is cheap and mechanical: **revert the fix and
watch the test fail before believing it** — and, twice here, *check the
revert actually applied*, because a scripted edit that silently matched
nothing produces a green run that reads identically to a passing test. The
harder judgement is the other half: when a defect's trigger is genuinely
unreachable from outside — a flag set only by loop-internal state — the right
answer is to ship the fix with the reasoning beside it and **no** test,
rather than a test that cannot fail. A false green is worse than a gap,
because a gap is visible.

**A stale mtime is a stale build, and it reports success** (2026-08-27, the
probe lane's). After a cherry-pick, `cargo build` reused an old binary because
git had restored a source mtime older than the last artifact — and printed
`Finished`. Two full probe runs measured code that was not in the binary.
CLAUDE.md's rule is *"a fresh mtime is not a fresh build"*; this is the inverse
and fails identically, so the rule generalises: **the mtime says nothing in
either direction, and only the artifact can answer what it can do.**

**…and the obvious way to ask the artifact is easy to get wrong** (same lane,
same hour, and the better half of the lesson). The check was `strings
target/debug/mecha | grep -c "turn "` → 42, read as confirmation. **`"return "`
contains `"turn "`.** The probe string collided with ordinary compiled text and
would have passed against any binary ever built. **A verification that can pass
for the wrong reason is worse than none:** pick a literal that cannot collide,
and confirm it is *absent before* the change as well as present after — a
one-sided check cannot tell a fixed build from a lucky substring.

**`cd X && ( … ) &` backgrounds the whole list, including the `cd`** (same
lane). Three probe runs executed `./target/debug/mecha` from the main checkout
rather than the worktree — a different branch's binary — and produced plausible
output throughout. **In any backgrounded shell, address binaries and outputs
absolutely**: a relative path resolves silently against whatever working
directory survived, and the failure looks exactly like a result.

**A deliberate break, made to prove a test is not vacuous, was left in the
tree** (2026-08-27). The check itself is the project's own rule — verify a fix
by making it fail on the old behaviour — and it worked twice: removing the
result from boredom's key made the polling test fail, and unstamping the work
counters made the step-appraisal test fail. Undoing the first with `git
checkout <file>` silently did nothing, because the file was a **new module and
therefore untracked**, and `git checkout` on an untracked path is not an error.
The break survived into the next test run, which still passed, so nothing said
so. **An undo has to be an operation that applies to the file you broke** —
for an untracked one that means editing it back or `git stash` after adding it,
never a checkout — and the general shape is the one this file keeps recording:
a command that no-ops on the case you are in reads exactly like a command that
worked.

**Everything observable said the hand-over worked, and it had thrown the
conversation away.** `--resume` was parsed by clap, printed in the child's
argv, and never used: an earlier patch had aborted on a failed assertion and
written nothing, so the code still unconditionally created a session. The
board moved to `waiting on mecha`, the run started, the exit code was 0, and
the first line of the *new* transcript was "carry on from what you have both
agreed above" with nothing above it. The lesson is about what to check rather
than about patches: when an operation is supposed to act on an **existing
object**, verify *which object grew*, not that something happened — every
signal here described the operation and none described its target, which is
the same blindness as releasing a staged call against the reviewer's
workspace. And the cheap habit that catches it: after a scripted edit, grep
for a distinctive string from the change before building on it, because a
build that succeeds proves the file compiles, not that it changed.

**Three tests in one arc passed for reasons unrelated to what they asserted**,
and each was caught only by running it against the pre-change code rather than
by reading it. One asserted "no summary was requested" on a transcript too
short for `worth_compacting`, so neither arm requested one. One drove the loop
through a provider that reported a hardcoded ten tokens per request instead of
pricing what it was sent, so every prediction under test was a measurement of
the fixture. One asserted a series survived a run boundary while the revert
that was meant to break it still wrote the value back. The general shape:
**verifying a fix means making the test fail on the old behaviour, and a green
test proves nothing about which mechanism made it green** — three plausible
green tests here were measuring an absent precondition, a constant, and the
half of the change that was left in.

**A test that has to sit between two thresholds needs to assert it is still
between them.** Pressure high enough to narrow the tool-output budget is also
pressure high enough to trip the compaction threshold, so the first version
measured a summary thinning the transcript and reported the first result at 174
bytes — which reads exactly like the feature not working. The fix was to size
it deliberately between the two *and* assert `compactions == 0`, so it cannot
drift back into measuring the other mechanism. Where two mechanisms relieve one
condition, a test of either must pin the other off.

**The predictive check made the obvious test for the next change impossible,
and that was the finding.** Sizing a test for the cross-run series by letting
run 1 grow the transcript with tool calls could not discriminate, because the
predictive check fires *inside* run 1 the turn before that growth is priced. It
narrows when a carried anchor is the deciding signal to exactly one situation:
when the transcript grows **between** runs — a model answering and then a large
message arriving, which is how a chat session actually gets big. A change that
makes a test hard to write is sometimes telling you the case is narrower than
you thought.

**A fresh mtime is not a fresh build.** An installed `mecha` was argued current
because its mtime post-dated the commit and the tree added no Rust after it —
both premises true, the inference false. Running `sessions health --json`
settled it in one command: the binary was the morning's and lacked the arc
entirely, while the version string read 0.1.15 on both sides of the feature.
Ask the artifact what it can do; no reconstruction of *when* an install
happened can answer *what* it contains. The same rule the harness already
follows against llama-server — ask `GET /props` what is served, never assert
it.

**A fixture whose node name equalled its own id could not tell a producer
writing names from one writing ids**, and hid the bug for as long as the
linker existed. `seed_node` built every test node with
`Node::new(id, node_type, id)`, so `linker:knn` storing `subject: <node id>`
and `subject: <node name>` produce byte-identical payloads under test. The
assertion that would have caught it — the payload contains "hyperalignment" —
passed either way, and so did the accept path, because in that fixture the id
*was* a resolvable name. Reverting the fix against a fixture with distinct
names fails on `left: "topic-1f0a", right: "Hyperalignment"`, which is what a
test for this was always supposed to say.

**When a fixture makes two fields equal, every test over it is blind to
which one the code used.** The tell is a helper that fills several arguments
from one value for brevity; the fix is to make them differ in the fixture
*before* asking what a test proves. Verified the way this file asks for —
by reverting the change and watching the new tests fail, not by watching
them pass.

**A named id set was silently trimmed to ten, because `--top` bounds `--ids`
as well as a listing.** `mecha review items --ids a,b,c…` passes the ids
through to `mecha-graph review`, whose `--top` defaults to 10 — so a dive into
a similarity group of seventeen returned ten rows and said nothing about the
other seven. Every surface over it inherited that: the TUI's group dive since
the level shipped, and the phone's the day it was written. Found only because
a review of a *different* bug in the same code (a progress counter reading the
requested count instead of the returned rows) put two numbers on screen that
disagreed.

**A limit meant for an open-ended listing becomes a silent truncation the
moment the caller enumerates what it wants.** The two cases want opposite
defaults and share one flag. This is the queue's own "no silent caps" rule
failing one function away from the sampler that states it — `review sample`
announces its bound and why, and the path beside it trimmed an enumerated set
in silence. Where a cap and a named set meet, the set wins, and the count
comes from the caller's own list length (`review.rs`, `ids_limit`, tested past
the default of ten).

**A count taken on your own branch describes a tree nobody will check out**,
and three sessions proposed three mechanisms for how it went wrong before one
of them was measured. Worth the space, because the wrong two are both
plausible and one of them is a real hazard that simply was not this.

The facts. Three lanes landed work on 2026-08-25; two sessions independently
re-measured the workspace test count as part of a handoff pass — 1,365 and
1,367, each correct on its own tip, each wrong about `main`. The merge was
1,370. The later sweep then attributed the delta by walking `git log` and
came up three `mecha-core` tests short, so `5b187c5` had no lane and the
attribution would have been confidently wrong about which arc produced them.

**The cause was the starting point, not the range and not the topology.**
That sweep ran inside a worktree on a branch cut from `c2ca24b`, and
`5b187c5` was committed **31 minutes later**. `git log` walks what is
reachable from `HEAD`; a commit that lands on `main` after your branch point
is not in your history at any date, under any traversal flag. Reproduced
both ways: the identical command finds it on `main` and does not find it on
`c2ca24b`, and `merge-base --is-ancestor 5b187c5 c2ca24b` fails.

Two mechanisms were proposed first and both are worth recording as refuted,
because each is the kind of thing that sounds settled:

- *"It arrived through a merge, so the linear log missed it."* False, and it
  was the first author's own guess. `5b187c5` is on the first-parent chain;
  `git log` walks merges perfectly well. The remedy that guess produced —
  `--ancestry-path`, `merge-base --is-ancestor` — would have changed nothing,
  since nothing was excluded by traversal.
- *"A bare date in `--since` does not mean midnight."* **True, measured, and
  not what happened here** — the sweep passed an explicit `2026-08-24 18:00`,
  a window that does contain the commit. Keep the finding anyway, because it
  is a genuine trap two sessions confirmed on this tree the same afternoon:
  git's approxidate fills the fields you omit from *now*, so
  `--since=2026-08-25` run at 13:25 means "since 13:25 today" and returns
  **0 commits** where `--since="2026-08-25 00:00"` returns 35. A filter whose
  meaning depends on the hour you run it will disagree with itself across one
  session and never say so.

**Attribute with a commit range from the tag, in the tree you are about to
release** — `git diff v0.1.13..HEAD -- 'mecha-core/**/*.rs'` and friends,
which settle it per suite with no clock and no branch in the answer: +3
`mecha-core`, +24 `mecha-cli` (19 review-surfaces, 5 voice), 0 elsewhere,
matching the measured 1,343 → 1,370 exactly. A date range answers "what
happened while I was awake"; a branch answers "what I have been doing".
Neither is "what is in this tree", which is the only question a release
count is asking.

**And the meta-lesson, which is the durable one:** two of the three
explanations were produced by reasoning about git's behaviour and both were
wrong; the third came from re-running the original command against the two
different starting points and comparing. Reasoning produces an explanation
that *fits*; reproduction produces one that *fired*, and only the second
survives contact with a fact nobody thought to check.

The `--since` half is the sharper warning of the two, because it was not a
guess — it was **verified in isolation and never run against the command
that produced the number**. It does exactly what its author said it does;
it simply could not have fired here, since the invocation passed an
explicit time. That is the most expensive shape available: a mechanism that
reproduces on its own *feels* confirmed, so the one remaining check — point
it at the actual event — is the one it talks you out of. The original
invocation was on the record the whole time. When you have it, test the
event, not the hypothesis.

**The one commit that skipped the test run was the one that broke the suite.**
The offer-proxy commit substituted live verification (build, restart, curl
through the real door) for `cargo test` — and it broke a test-only struct
initializer, invisible to every live check because live checks exercise the
running path and only the suite compiles the fixtures. It rode through a
merge under a pipe that swallowed cargo's exit status. Two lessons, both old
ones wearing new clothes: the suite and the live check verify different
things and neither substitutes for the other; and an exit status behind a
pipe is not a check at all.

**A chat model in the transcriber's seat answered the audio instead of
transcribing it.** Voxtral returned "I don't have access to your calendar"
for question-shaped speech and obeyed a spoken "just say banana" — which also
made the STT leg a prompt-injection surface, since anyone who can play audio
at the mic could steer the transcript. No prompt fixes what a model is; the
seat needed a transcription model (Parakeet), and the proof was adversarial
probes through the real pipeline, not reading configs. When a component's
output looks plausible, test it with inputs whose correct output you control.


A SQL predicate written for a `WHERE` clause was reused inside `SUM()`, and
the `OR` that had silently filtered NULLs began returning them:
`reviewed_by = 'user'` on a legacy NULL row is NULL, `NULL OR false` is NULL,
and one class whose decided rows were all machine rejects made the aggregate
NULL — which errored the cluster view, the proposer rollup and the `/queues`
modal within an hour of shipping, while every test on the `WHERE`-side
consumer stayed green, because there NULL merely fails the row. A predicate
shared between filtering and aggregation must be NULL-total — COALESCE every
nullable comparison — and the regression test that matters seeds the
all-NULL group.

**Two queries computed the same statistic and only one was filtered — the
unfiltered one was the one on screen.** `ladder::human_record` excluded the
pipeline's own rejects (`reject_reason NOT LIKE 'precheck:%'`) with a comment
explaining why; `precheck::review_clusters`, which feeds the cluster review UI
and `review --clusters`, had no such filter. So machine dedup rejections were
displayed as the owner's verdicts in the one view a person reads immediately
before verdicting a whole class: `llm/has` shown at 18% against a true 67%,
three classes shown at 0% on which nobody had ever voted. A whole analysis was
built on the wrong number and reached a confident, wrong conclusion — that
half the queue was demonstrably unwanted — before the second query was found.
**When a statistic has two implementations, the one with the careful comment
is evidence that the other one is wrong.** Grep for the filter, not for the
column name.

**And the correction has a second half worth keeping: an absent rate is not a
zero.** With machine rejects excluded, 40.5% of that queue turned out to sit in
classes with *no human verdict at all* — which the display rendered as "0%",
indistinguishable from a class rejected every time it was seen. Every surface
now prints a dash. "Nothing went wrong" and "nothing happened" are different
answers, and the same rule already governs `sessions health`; it is worth
re-checking wherever a rate reaches a human.

**A random sample and the head of a queue are not interchangeable.** Judging
the first dozen candidates in a class and reading the result as the class's
accept rate measures the *ordering*, which is correlated with age, id and
confidence. The fix is a seeded uniform draw whose seed is printed, so the
sample can be redrawn and checked — and a verdict must not resample, or a
sitting's twelve verdicts describe twelve different samples instead of one.

**A truncating pipe turned a partial test run into a plausible total.**
`cargo test --workspace 2>&1 | tail -30` was sent to the background on
2026-08-21; the exit code was 0 and the captured file held four `test result`
lines summing to 76. That reads exactly like a small workspace passing, and the
figure was almost quoted into the handoff — the real count was 1,244 across
eight suites, and `tail` had thrown away every line above the last thirty. The
grep that looked for totals ran over a file that no longer contained them.
**A filter applied before capture makes the capture look complete. Aggregate
from the whole stream, or capture the whole stream and filter on read** — and
treat a suspiciously small total as evidence about the pipeline before it is
evidence about the code.

**The bug was only visible in a run that worked.** After images shipped, a
Slack screenshot was answered correctly — and the run recorded
`taint {private: false}` where the same user action had previously recorded
`{private: true}`, because the model no longer needed `fs_read` to see the
file. A feature had loosened the interlock as a side effect, with a correct
answer sitting on top of it: no error, no warning, nothing in the trace. The
only evidence was a boolean in the outcome record. **Read the run's recorded
state after a change, not only its answer — a success is where a silent
regression hides.**


- **A value that only means something in a frame of reference, copied into a
  context that dropped the frame.** Three separate TUI bugs on 2026-08-20 were
  one mistake: `input_layout` counted *characters* where the terminal counts
  *cells*; `/help`, `/tools` and `/tasks` sized their boxes from `body.len()`,
  a count of `Line`s, and then drew through `Wrap`, which turns each into one
  *or more* rows. Every one looked like arithmetic on a number that was simply
  wrong, and none of them was — the number was right in the unit it was
  computed in. The defence is not to re-derive the value in the new context but
  to ask the thing that will actually draw it: `paragraph.line_count(width)`
  rather than `body.len()`, and `unicode-width` rather than `chars().count()`.
  `unstable-rendered-line-info` was already in the Cargo.toml for exactly this
  and only `transcript.rs` was using it. The same shape reached the peer review
  of that arc: a `git status --short` listing retyped into prose lost its
  leading space, moving the mark from column 2 to column 1 and turning
  "unstaged" into "staged" — the distinction that decides whether a bare
  `git commit` is dangerous. Fixed-width output is a format; paste it, never
  tidy it.

- **Two implementations of one question always drift; delete one rather than
  reconciling them.** The input box asked "where does this text wrap?" twice —
  once in `input_layout`, once in `Paragraph::wrap` — and the caret followed
  one answer while the glyphs followed the other. Measured on a real frame at
  30 columns: the painted last row read `"class"` and the caret sat at column
  3. Making them agree would have coupled the code to a ratatui internal that
  is free to change; the fix was to stop calling `.wrap()` and render the rows
  the layout function had already computed. Same rule as `list_height_reserving`
  and `find_tool` — where two callers must answer identically, there has to be
  one function they both call.

- **A clamp whose bounds can cross is a panic, not a layout bug.** Every modal
  sized its list inline as `rows.clamp(1, terminal_height.saturating_sub(4))`.
  The subtraction saturates to zero the moment the terminal is four rows or
  fewer, and `clamp` asserts `min <= max` — so shrinking a window with any modal
  open took the whole session down, partial answer and all. Found in `/doctor`,
  then written again in `/skills`, and five more modals had their own copy,
  because a new modal is written by opening whichever sibling is nearest. That
  is what made it a shared function (`tui::list_height`) rather than seven
  fixes; flooring the bound at one row degrades a tiny terminal to a one-row box
  instead. Each modal carries a draw-at-tiny-sizes test whose assertion *is* the
  draw, verified to fail on the old line (`min > max. min = 1, max = 0`) rather
  than merely to pass on the new one.

  **The eighth site is the one worth remembering**, because it survived the
  sweep that fixed the other seven. `/mail` renders its key legend *inside* the
  block rather than in the title, so its box needs a floor of **two** — it
  spelled its clamp `clamp(2, …)`, matched no grep aimed at `clamp(1, …)`, and
  collided with the ceiling one row earlier than everywhere else (dead at five
  rows, not four). The lesson is not "grep harder": the two-argument helper
  **could not express that box**, and a helper that cannot say what a caller
  means is how a caller keeps saying it inline. `list_height_reserving(rows,
  height, reserved)` can, `list_height` delegates to it with `reserved = 0`, and
  the degradation runs the safe way — the ceiling floors at one row and then the
  *floor is pulled down to meet it*, so a terminal too short for the strip and a
  row of list gets a useless live box instead of a dead session. The general
  form to search for is `.clamp(` near `saturating_sub(4)`, not either literal.

- **`KeyCode::Char(c)` is not a typed character.** crossterm reports the
  modifier *beside* the letter, so a match on `Char(c)` alone sees the bare
  letter and cannot tell Ctrl-C from `c`. Harmless in the main input box, whose
  `ctrl` branch runs first and consumes them; not harmless in `/mail`, whose
  keys go through `action_for`, where **Ctrl-A archived the thread under the
  cursor, Ctrl-D dismissed it, Ctrl-T made a task of it and Ctrl-R started a
  drafting run** — on chords that mean beginning-of-line, delete and refresh
  everywhere else in a terminal. Seven sites had their own copy of the mistake,
  which is the `list_height` lesson again: a new modal is written by copying
  whichever sibling is nearest, so the fix has to be the thing that gets
  copied, not seven guards.

- **A view can answer a shorter question than the one it exists for.** `/tools`
  detail exists to say what a tool may do to you, and its capability block sits
  at the *bottom* of a body whose top is an MCP server's own description —
  arbitrary third-party text. With the built-in tools it always fit; on
  `kg_task_create` it was eight of eleven lines, with "reads data the user
  considers private" below the fold and `Up`/`Down` guarded `if !detail` so no
  key reached it. The general shape: when a surface's *answer* is positioned
  after content whose length someone else controls, the surface degrades to
  looking complete while omitting the point. Test it with the largest real
  input available, not with the fixture.

- **A sweep that keys on a spelling misses the shape.** Seven modals carried
  `rows.clamp(1, terminal_height.saturating_sub(4))`, which panics at four rows
  because the subtraction saturates below the floor; the fix swept the codebase
  for `clamp(1,` and fixed six. `/mail` spelled it `clamp(2, …)` — it reserves a
  row for its key strip — so it matched nothing, and it was the *worst*
  instance, dying at five rows where the others died at four. It was found only
  because someone asked which modals have an in-box header, which is the
  question the bug was actually about. **Search for the invariant, not the
  token.** The second half is why the spelling diverged at all: the shared
  helper took only `(rows, height)`, so a box that reserves a row could not be
  expressed and its author wrote the arithmetic inline. A helper that cannot say
  what a caller means is how that caller stops using it.

- **When the fix's fail-safe behaviour is the new bug's observable, the test
  that caught the original cannot catch the misuse.** The repaired helper takes
  `(rows, terminal_height, reserved)` — three `u16`s — and its whole design is
  to degrade to a small box rather than panic. Transposing the last two
  arguments therefore compiles, does not panic, and silently returns a
  three-row box; the tiny-terminal regression test passes with the arguments
  swapped, because a box too small to read is not a crash. Pin the *values* at
  the call site, or make the type carry the meaning. A guard and a bug that
  share an observable are indistinguishable to every test written against the
  guard.

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
- **A fixture can make a test pass for the wrong reason.** The helper seeding
  two models' runs generated colliding session ids, so the second model's
  transcripts silently rewrote the first's — and the blended-rate test passed
  while reporting the *healthy* model as the broken one. It only surfaced
  because the assertion named which model it expected. Assert on the identity
  of the thing you found, not only on the count.
- **Substring grading measures formatting.** `"$2,520"` failed a check for
  `2520`; `"do **not** agree"` failed `not agree`. Both answers were right. The
  `normalize` helper in `mecha-core/src/eval.rs` handles it — extend that, don't
  work around it.
- **…and again, unboundedly.** "There is no `budget.csv`" matched none of ten
  hand-listed negation phrasings. The negation phrasing space has no bottom —
  that case is judge-only now. Reach for `expect.judge` when you catch yourself
  enumerating synonyms.
- **Check what the project already measured before proposing the fix.** A
  session profiling context growth found tool output filling ~8k tokens a turn
  and proposed two changes: add offloading, then lower the budget so it fires
  more. Offloading had existed since 2026-08-05 (`ToolCtx::spill_dir`), and
  `CONTEXT-RESEARCH.md` had already measured the second — cutting tool-output
  tokens 38.4% cost **6.8% more**, r = 0.154, because trajectory length
  dominates and re-reading spilled content costs turns. Both errors came from
  optimising *context occupancy* without checking the *cost and outcome*
  evidence in this repo. A `docs/*-RESEARCH.md` with an "ordered implications"
  section is a checklist to read before proposing, not a document to write
  after deciding.
- **Quoting a string is not the same as protecting it.** The benchmark adapter
  `shlex.quote`d each task instruction, which is the correct reflex for
  untrusted text on a command line and did nothing here: quoting guarantees the
  text becomes one argv *entry*, and the problem was that entry's first
  character. `terminal-bench/pytorch-model-recovery` opens with `- ` because
  its description is a bulleted list, clap read a flag, and the run exited 2
  before starting — scored 0.0 and indistinguishable from a model that tried.
  Pass `--` before any positional you did not write. Note the general shape,
  which this project keeps meeting: **a harness fault that produces a
  plausible-looking score is worse than one that crashes**, because nothing
  ever asks about it.
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
- **A pure function can be right in isolation and wrong against stored
  state.** `merge_push` — the three-way fold behind the switchboard's two
  writers — passed every unit test: right values, right conflict list. It
  re-serialised the merged table, so a push that won *every* conflict produced
  text differing from the pushed file only in formatting, and the stored
  `baseline` and `effective` then differed too. "Has the browser edited this?"
  is a text comparison, so the cockpit would have reported unpulled edits that
  did not exist and every later push would have taken the merge path for
  nothing. Invisible to a test on the function; obvious the first time the two
  stored columns sat side by side. **Test a pure function where its output is
  compared, not only where it is produced.**
- **Parsing is not validation, and a permissive parser hides the case you
  meant to catch.** A profile's timezone check was written as "does
  `chrono_tz` accept it", and the test asserting `EST` is refused failed —
  `EST` parses, because the IANA database still carries the legacy
  fixed-offset zones. A booking page rendered in `EST` is an hour wrong from
  March to November and looks right throughout, which is the failure shape
  this project keeps finding. The rule is now `Region/City` (or `UTC`), and
  the test asserts `"EST".parse::<Tz>().is_ok()` *first*, so it fails loudly
  rather than silently passing if that premise ever changes. **When a library
  accepts more than you mean, encode the narrower rule and pin the premise in
  the test.**
- **Writing an invariant in a doc comment is not enforcing it.** A
  high-effort review of the switchboard branch returned ten defects, and
  **four violated rules asserted in the module docs sitting beside them** —
  `inventory.rs` said an off-origin host is shown always while `host_of`
  returned the URL's *userinfo* as the host and `masthead` omitted the host
  entirely for labelled links; `editor.rs` said a slug must never be burned
  "as a side effect of typing one" while `save` upserted one for any slug in
  the form. Each doc was written honestly and each described the intent
  rather than the code. **Prose beside a rule is evidence the author meant
  it, never evidence the code does it — and a reviewer reading the code
  against the comment finds precisely these.**
- **A static path segment beside a parameter shadows it, for whichever value
  matches.** The shared assets sat at `/@a/{name}` next to
  `/@{handle}/{slug}`; `a` is a legal handle, matchit prefers the static
  segment, and that user's every switchboard answered 404 at a URL their own
  account page printed. Reserving the handle fixes one case and leaves the
  trap for the next static segment. **Keep static segments out of a
  parameter's namespace entirely rather than reserving the values that
  collide.**

- **The scrub's own author passed it; fresh eyes with no list did not.** The
  mecha-graph extraction was audited twice. The first pass (and the agents
  that applied it) rationalized three keeps: a "replacement" address that was
  a real Vermont locality triple, a real company founder kept because he was
  "a public figure in his public role", and household fixtures whose names
  were synthetic but whose facts — the pediatrician visit, the snow tires,
  the river — were still one real household lightly relabeled. The second
  auditor was deliberately given no list of what had been removed, and found
  all three plus a live wearable transcript. The lesson: a verification pass
  that knows the answer key checks the key, not the work. Independence means
  withholding the list, and every new find goes into the gate so the next
  regression is caught mechanically.

**The commit that shipped without its own new files was authored by a
failed `&&` chain.** One command staged and committed the first change, then
staged and committed the second — but the first commit died on the
pre-commit fmt hook, the chain aborted, and the retry re-staged with
`git add -u`, which updates *tracked* files only. The second commit landed
without its two brand-new files, did not build in isolation, and the miss
surfaced only when `cargo install` ran from the merged tree. Two lessons:
after any failed commit, the index is not what the failed command's later
steps would have made it — re-verify with `git status`, don't re-run from
memory; and "every commit builds in isolation" is only true if something
builds the commit's own tree, which the worktree (holding the untracked
files) cannot prove.

**A doc written from a build log is written on the machine where it
already works.** The voice page's most load-bearing sentence — that the
published crate contains the facade and none of the pipeline — appears
nowhere in the build log that documented the stack, because the person
writing that log had every service running locally and never had to
discover the packaging boundary.

The mechanism, sharpened by the session that found it: **explaining a
system to someone who does not have it is a different act from documenting
it, and only the first has to answer "what do I need before any of this
works?"** A maintainer writing docs answers *what does it do* — a question
that never reaches the packaging boundary, which is why
`cargo package --list` had never been run against this feature by anyone.
The general lesson: when documenting for
users, the facts most worth stating are the ones the author's environment
makes invisible, and `cargo package --list` / `git ls-files` answer in
seconds what memory answers wrongly. Check what ships, not what works.

**A plan and a to-do list are the same words, and only a timestamp tells
them apart.** On 2026-08-19 at 04:23 the handoff gained a section reordering
the remaining mail phases by what the corpus had just measured. Between 11:08
and 18:10 the same day every one of those phases shipped, in commits that name
them by number. Nothing went back to strike the plan, and nothing recorded the
arc in this file either — so for six days the section read as a to-do list for
work that was finished, headed "four phases remain". On 2026-08-25 a session
picking the project up cold read it, chose the pre-filter as the best
value-per-effort item, cut a worktree and started scoping it before a grep
found `mail_triage::prefilter` already there with both rules and a
`PrefilterRule` recorded on every verdict.

Two mechanisms, and the second is the one that generalises. The narrow one:
**the reflex that updates prose about a subsystem is not the reflex that
strikes an item off a list.** That section's narrative was revised repeatedly
in the days after — it discusses the 2026-08-25 both-account sweep in
detail — while the list beside it was not touched, because you revise the
story when you *learn* something and the list only when you *finish*
something, which is the moment you are least inclined to write anything down.
When a session executes a plan it wrote the same day, closing out is part of
that arc rather than a chore for later; by the next session the plan has
become indistinguishable from a backlog.

The broader one: **a blanket claim of verification is worth exactly the
evidence beside it.** The handoff opened with "every item below was
re-verified against source on 2026-08-24" — six days after this work landed,
and it did not cover this section. That sentence is what made the stale list
credible; without it a reader would have checked. A coverage claim should
either name what it checked or be narrowed until it can.

**And the diagnosis nearly went the same way as the entry above it.** The
session's first explanation was that the 08-24 sweep had missed the section —
a mechanism that fits the symptom perfectly. Checking with
`git merge-base --is-ancestor` showed the bullets were authored at 04:23,
*before* any of the work existed, so there was nothing for a sweep to miss at
the time they were written; the failure was never re-checking them, not a bad
check. That is the same lesson the branch-cut-too-early entry reached
independently, arriving from the other direction: **a mechanism that fits the
symptom is not yet evidence it caused it**, and the cheap test is almost
always re-running one command against two starting points.

### Learning

**2026-08-30 — the safeguard's release condition was satisfied by the
evidence it existed to act on.** Probation's stricter retirement threshold
released once the ledger "measured" the rule — but the measurement that
matters, an attributed regression, only ever arrives *inside* an
observation, so any rule with conviction evidence had already been released
to the ordinary threshold by the scan reading it. `PROBATION_RETIRE_AT` was
unreachable from the day it shipped, every unit under it green, three
documents describing it. General shape: **when a hedge's release predicate
is implied by its trigger evidence, the hedge is decoration — check the
implication direction between "what releases it" and "what it exists to
catch", and fire the joined path once before relying on it.** Every piece
was unit-tested; only the drill that ran the motion whole could have found
it, and it did, on its first run.

**2026-08-30 — a closed list gates live text, but recordings are of their
era.** `STEP_ESCALATION_STEM` shipped mid-day 2026-08-28; a transcript from
that morning carried the same fully-templated nudge body with no stem, and
the miner recorded it as a user's steer — it reached the probe corpus and
was counterfactually replayed as if a person had typed it.
`is_harness_voice` now also matches the frozen pre-stem bodies. General
shape: **matching harness voice against recordings needs every wording that
ever shipped, not the current one** — the historical strings are frozen and
can never drift, so adding them costs nothing; omitting them lets the
harness's own words earn rules.

All found by pre-push review or by running it.

- **Suppressing an alarm is a claim that needs its own evidence.** Startup
  warned that `triage` rules could never fire. Two readings were equally
  consistent with what anyone knew: the warning is a false positive because
  the domain is loaded by a named pass, or the warning is *correct* and the
  feature is incomplete. `PASS_DOMAINS` was added on the first reading, by two
  sessions who agreed with each other and tested neither — and the second was
  true: nothing loaded triage rules at all, so the loop wrote into a file no
  reader existed for, with the alarm muted by the same hand. Before silencing
  a check, verify the thing it is complaining about, not the reason you think
  it is complaining.

- **"Every front-end writes this" was true of eight of ten.** `Record::Outcome`
  is documented as written once per finished run by every front-end, and
  `record_outcome` had ten call sites — `run`, `chat`, `tui`, `trigger`,
  `frontdoor`, `slack`, `voice`, `serve/chat` — with `tasks work` and
  `questions answer` in neither. So the run-quality corpus had never seen a
  delegated run, and the task-agent design's own open question ("`RunStats`
  already records enough to answer this later") was false for exactly the runs
  it was about. Nothing failed: both callers bound the outcome and checked only
  its `Err`, which reads as complete. **A claim of the form "every X does Y" is
  a grep, not a belief** — and the component that is missing Y cannot see that
  it is, because from inside it there is nothing to compare against. Found by
  counting call sites while looking for something else.

- **A tool that looks scoped and is not.** `cargo fmt -- path/to/one.rs`
  does not format one file: rustfmt walks module declarations from the crate
  root, so the argument narrows *what it starts from*, not what it touches.
  It silently reformatted two files belonging to another lane in a shared
  checkout, by a session that had spent the afternoon refusing `git add -A`
  and staging by explicit path precisely to avoid that. The reformatting was
  how a pre-existing nine-site drift became visible at all, so it ended
  usefully — which is luck, not process. **`git add <path>` at least
  advertises its scope; `cargo fmt -- <path>` advertises a scope it does not
  honour.** Check `git status` after any whole-tree tool, however narrow the
  invocation looked.

- **A CI that did not run is indistinguishable from one that ran and
  passed.** On 2026-08-26 that happened **four ways in one afternoon**,
  across two sessions, and every one of them produced a green-looking repo:

  1. **Push events dropped.** Two pushes produced *zero* runs — Actions
     enabled, all six workflows `active`, GitHub holding the new sha as
     `main`, `total_count: 0` for it. A GitHub Actions **major outage** from
     15:11 UTC. Nothing errors; the push simply succeeds and nothing happens.
  2. **Runs cancelled by their own successors.** `ci.yml` sets
     `concurrency: cancel-in-progress` keyed on `github.ref`, and five PRs
     merged inside four minutes, so each merge's run killed the one before
     and the last was killed too. The merge commit `efa04e2` ended with
     **no successful run at all** while the branch looked healthy.
  3. **Two sessions cancelling each other.** `github.ref` is
     `refs/heads/main` for *every* run on main — push or dispatch, whichever
     sha — so two people dispatching CI serialise into mutual cancellation.
     Same collision as (2), one level up.
  4. **Every convenient query is addressed by position, not by sha.**
     `gh run list --limit 1`, `gh pr checks`, the Actions tab — all move
     under you. One session read the top row, reported `main` green, and had
     in fact read *the other session's* run on a different sha; the trap was
     named to them one message earlier and they did it on the next command.

  **The only query that answers the question is
  `gh api "…/actions/runs?head_sha=$(git rev-parse main)"`.** And the
  reporting rule that falls out: *clean locally* and *CI passed* are
  different claims about different objects, and a handoff that merges them
  is asserting something nobody checked — which is exactly what happened,
  in the one file whose job is to be reliable, because one session took a
  peer's verification claim as fact and wrote it down.

  The narrower rule, offered by the session that made the unchecked claim
  and worth more than "verify": **a verification claim should travel with
  its run id.** A conclusion — "main is green" — can only be believed or
  not; an id can be checked in one command, and `run 32992960643` would
  have shown `head_sha=11a179d` immediately. This generalises past CI to
  anything one agent tells another it confirmed: **send the handle, not the
  verdict**, so the recipient checks the key rather than your reading of
  it. Every misattribution this day produced had the same shape — a
  conclusion transmitted where a checkable reference would have cost the
  same to send.

- **A cron job's binary is a running thing that answers to no `--version`.**
  The `update` skill said the graph repo's nightly "builds and runs from its
  repo tree and is not mecha's concern". It does not build — `nightly.sh` sets
  `PKG="$REPO_DIR/target/release/mecha-graph"` and executes whatever is there —
  and it is very much mecha's concern, because its `link --auto` step writes
  into the owner's graph at 01:30. Found on 2026-08-26 hours after a session
  repaired a linker bug out of the live graph (30 placeholder nodes merged, 121
  payloads rewritten, 23 accepted facts re-pointed); that binary was dated Aug
  25, so the nightly would have re-run the old linker and re-staged the same
  damage while every version string on the machine read current. The general
  shape: **an inventory of "what is running" that only lists things with a
  `--version` will miss every scheduled job**, and a skill's own claim about a
  script is worth checking against the script — this one was refuted by two
  lines of `grep`. Fixed in the skill, with the date check spelled out and a
  warning not to assume `cargo install --path` refreshed that path.

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

- **A field you never read is a behaviour you never had.** The whole
  reasoning channel of every local model was discarded because
  `reasoning_content` appeared nowhere in the tree, and nothing failed: the
  turns still worked, the tests still passed, and the only symptom was
  "sometimes the model returns nothing" — which acquired a plausible causal
  story ("it reasons without terminating") that survived three days and drove
  three mitigations, none of which touched the cause. **When a symptom has an
  explanation nobody has measured, the first move is to record what actually
  arrived, not to mitigate.** The instrument was twenty lines and it inverted
  the diagnosis on its first firing.

- **The harness can be the thing teaching the model to misbehave.** Stripping
  reasoning out of the history meant every prior assistant turn showed the
  model calling tools with no think block — and the failure being chased was
  the model emitting a tool call with no think block. Measured at 6/6 versus
  0/6 on the same prefix. A conversation is not just what you send *this*
  turn; it is the in-context demonstration of what a turn looks like, and a
  harness that edits it is authoring that demonstration whether it means to or
  not. Corollary: `anthropic.rs` had replayed thinking correctly all along, so
  the asymmetry between two backends of the same codebase was visible evidence
  and nobody had read it as such.

- **Two dialects can disagree about what a number contains.** Anthropic reports
  `input_tokens` *beside* the cache tiers; OpenAI reports `prompt_tokens` with
  `cached_tokens` already *inside* it. `Usage::total_input` sums all three, and
  the compaction threshold reads that sum — so mapping the field across without
  subtracting would have reported every prompt at nearly twice its size and
  started summarising long runs at half the window they had. **A field with the
  same meaning in two APIs may not have the same arithmetic**, and the failure
  would have looked like working.

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

- **A refresh must never ask for a scope the grant may not have.** Entra's
  `refresh_token` sent the whole `SCOPES` list on every refresh; Google's
  never did. The moment that list widened — `Mail.Read` becoming
  `Mail.ReadWrite` on 2026-08-18 — every refresh asked Entra for a *superset*
  of what the stored grant had consented to, which it refuses with
  `invalid_grant`, which the classifier correctly reads as permanent and
  reports as a dead login. Every already-working account would have gone dark
  about an hour after the new binary was installed, reporting a revocation
  that had not happened. RFC 6749 §6 makes `scope` optional on a refresh and
  defaults it to the original grant. **A credential widening is a two-sided
  change: the request that mints the grant and every request that renews it
  disagree about what the grant contains, and only one of them was written by
  the person doing the widening.** Found by reading the refresh path while
  answering an unrelated question, not by a test.

- **Two different failures can be the same error string, and the recurring one
  wins on priors.** Google expires the refresh token of an OAuth client in
  *Testing* publishing status exactly 7 days after consent, and returns
  `invalid_grant` — "Token has been expired or revoked" — which is
  indistinguishable from a real revocation. The 2026-08-11 outage that
  motivated `mecha doctor` was recorded as a revoked token; it was six days
  after the grant, and the client was in Testing. Doctor was built to *report*
  it and does so correctly, but a marker written on failure can only ever
  describe an outage that already started. The fix is a `granted_at` stamp
  never touched by a refresh, a `grant_lifetime_days` the user declares
  (no API reports publishing status, so inferring it would either nag every
  verified app or stay silent on every Testing one), and a warning two days
  out. **When a failure recurs on a schedule, the store has to know the
  schedule — otherwise every instance looks like the first one.**

- **Grading the wrong axis is worse than not grading.** The mail classifier's
  escalation rule was instrumented to record when a second pass over the full
  body *changed the bucket*. The first real measurement said 13 of 51 threads
  escalated and one bucket moved — which, by the criterion written into the
  field's own doc comment, said the rule was wasteful and should narrow. It
  was measuring the wrong thing: a second pass that leaves the bucket alone
  while fixing `request_type` (the input front-door routing runs on), a
  deadline, or a `one_line` that read "message cuts off" has earned its call
  and registered as nothing. **An instrument that reports one axis of a
  multi-axis change does not produce a weak signal, it produces a confident
  wrong one** — and a number gets acted on where a gap gets investigated. Fixed
  by recording which fields differed, with `reasoning` excluded because prose
  differs on every re-read and would make every escalation look like a change.

- **`systemctl is-active` exits non-zero for `activating`**, so
  `until ! systemctl is-active <unit>` returns immediately on a oneshot that
  is still starting. A wait loop built on it reported a 25-minute sweep as
  finished within seconds, and the run was confirmed "clean" twice before
  anyone noticed it was still going. Poll `SubState` against `start`/`start-pre`
  instead. The general shape: **an exit code that distinguishes "no" from "not
  yet" cannot be used as a boolean**, and the failure is silent because the
  wrong answer is the one you were hoping for.

### Containment and state

**Two sessions in one checkout is fine until one of them operates on a file
the other is writing.** Two lanes worked the same tree for an afternoon —
reading each other's state cost nothing and caught real things (a misplaced
brace, a wrong install timestamp, a guard one lane needed and the other did
not). It broke exactly once: told "you go first", one lane lifted the
other's hunks out of a shared file to commit around them, and in the window
between the lift and the restore the other built new code against the
stripped version. Their tree stopped compiling and the cause was not
theirs. The rule that would have prevented it: **commit order follows who
is mid-flight, not who offers to go second** — "you go first" is only safe
once the other party has actually stopped. Two smaller ones that held up:
restore from a whole-file snapshot rather than a reconstruction (md5 the
result), and when a whole-tree gate like `cargo fmt --all --check` fails on
files outside your commit, bypass with the reason written down rather than
reformatting someone else's work into your change.

**A workaround for a gap that had since closed read as a deliberate rule,
and broke the feature it was protecting.** `tasks work`'s one-run-per-task
guard keyed on `status == "waiting"` because phase 1 had no way to tell the
agent from a person, and said so in a comment naming `waiting_on` as the
thing that would fix it. Phase 3 shipped `waiting_on` the same day and
nobody came back. Every finished run leaves a task `waiting`, so the second
delegation on any task refused — silently, from the web, because the child
exited 1 into `/dev/null` after the endpoint had already answered ok. The
general shape: **a temporary check outlives the reason for it and stops
looking temporary.** A comment naming the thing that will replace it is not
a reminder; the only reminders that work are the ones that fail. Where a
workaround cannot be made to fail loudly, it belongs on the same checklist
as the change that will retire it.

**A tool removed from the parent registry was still reachable through a
subagent, because the child's copy was made before the removal.**
`tasks work` withholds `kg_task_update` so a delegated run cannot close its
own task — but `build_subagent` clones each allowed tool out of the pool into
a separate registry while the agent is being *prepared*, so removing it from
the parent afterwards left any profile that allowlisted it holding a live
handle. A run told "you have no tool that sets status" could simply delegate.
Found by a review, not by testing, because every test had an empty subagent
list. The general shape: **a removal only reaches the copies that had not
been made yet — before trusting one as a control, ask who already took a
reference.** The fix refuses to start rather than stripping silently, on
`build_subagent`'s own precedent that a child quietly missing a tool its
description promises is worse than a loud failure.

**A session recorded the tool list before the tools were changed, so its
config record described a surface that never existed.** `RunConfig::of`
snapshots `agent.registry()` at call time, and both task entry points
appended the record *before* withholding `kg_task_update` and inserting
`ask_user` — so the transcript claimed a run had the tool it had been denied
and lacked the one it used, and `mecha replay` would rebuild the opposite
surface. Found while trying to *prove* the withholding had happened and
discovering the evidence disagreed with the code. The lesson: **a record that
snapshots live state has to be written after the state is final, and a record
of the wrong state is worse than no record** — the wrong one is believed.

**Seeding one row of data in a migration broke encrypt, decrypt and fork.**
The graph's V020 seeds an `agent-mecha` node so a task can be delegated on a
graph nobody has hand-populated. Every copy path runs migrations on the
*target* first and then copies the source's rows in, so the target already
held the row the source was about to send and the straight `INSERT` hit the
primary key. Caught by `test_copy_tables_covers_schema`'s neighbours rather
than the canary itself, and confirmed by running them on `main` first. The
general shape: **seeding schema is free and seeding data is not, wherever a
copy or fork path exists** — `nodes` joined the `INSERT OR IGNORE` pass that
`predicate` had been in all along, for one row.

**A "must already exist" check that mutated before it validated cleared the
real answer on a typo.** `set_task_waiting_on` retired the old `waiting_on`
fact and then resolved the new name, so `--waiting-on Nadai` did not merely
fail — it turned "Nadia owes me this" into "nobody owes me this", with an
error message that mentioned neither. The test written *for* the typo
protection is what caught the protection being incomplete. **Resolve
everything that can fail before touching anything that persists**, and where
a store has no transaction to lean on, that ordering is the whole guarantee.

**`current_exe` of a replaced binary is a path that does not exist, so a
long-lived TUI lost every child it tried to spawn.** Rust's `current_exe`
resolves `/proc/self/exe` to its target, and after `cargo install` swaps the
file, a running process's target reads `…/mecha (deleted)` — so `/queues`
died with `os error 2` and an outbox release failed *quietly*, the item
sitting `pending` as though the review surface were broken. Both reported by
the owner within the hour of the install, from the one session the
stale-process sweep had already named. The fix execs the `/proc/self/exe`
link itself, which the kernel resolves to the deleted inode: a session
drives the version it *is*. The general shape: **anything a long-lived
process re-derives from the filesystem — its own path included — can change
under it at an install; prefer handles that pin the inode.** Two sites keep
`current_exe` deliberately (a systemd unit's path, a sibling-binary lookup),
annotated so they are not "fixed".

**A mutating path that was never exercised shipped broken, because the live
store was correctly left alone and no scratch copy was made instead.** The
`/queues` class verdict passed `--proposer`/`--predicate` to a `mecha review
accept` that declared neither, so clap rejected it and the headline feature —
"one decision worth hundreds on instances" — could never have worked. Every
read path was tested against the real graph; the write path was skipped to
avoid mutating real data, and skipped entirely rather than redirected. **The
answer to "I must not test this against production" is a fixture, a fork, or a
`--dry-run` flag — never nothing.** (`mecha-graph fork` exists for exactly this
and turned out to be broken itself, 768-vs-1024 vector dimensions after the
harrier embedding switch; `--dry-run` was added to `mecha review` in its place
and is what finally exercised the path.)

- **Per-run jails and shared subprocesses do not mix, and the model finds out
  first.** MCP servers are spawned once with the agent, so a Slack connector
  giving each thread its own `RunContext` workspace left the servers rooted
  wherever it was launched: `bundle_render` resolved against the repo while
  `fs_write` wrote into the thread's jail. The model reported "the workspace
  and render tool have different root paths" and burned five turns working
  around it with `shell`. **Anything spawned once cannot follow a per-run
  value** — either root it somewhere both agree on, or accept that the
  isolation only covers what the loop itself resolves. Say which, in writing.

- **A measurement is not a conclusion, and a bad afternoon can masquerade as a
  law.** `-c 32768` was pinned for three days by one data point: raising it to
  131072 on 2026-08-07 took generation from 64 tokens in 1.06s to 64 in 52.6s,
  a 50x collapse. That measurement was real, and the rule written from it —
  *do not raise `-c`* — was not. 2026-08-07 is the day a runaway test OOMed the
  machine and systemd tore down both llama-servers; the KV cache was competing
  for memory that was not there. Re-measured 2026-08-10 on a quiet machine, one
  server at a time, medians of three at matched prompt lengths: 32k, 64k and
  128k are **within noise of each other** (92-93 tok/s at a 1k prompt, 81-82 at
  30k), and RSS is 21.5 GB either way. What costs is context actually *used* —
  63 tok/s at 108k — which is attention, not allocation. **When a single
  observation becomes a rule, write down the conditions with it; the next
  reader cannot tell an environment from a law.** The cost of not doing so was
  a 32k window on a 121 GB machine, and every compaction that came with it.

- **A server that loads while memory is contended stays slow for its whole
  life.** Found while measuring the above: a 64k instance started alongside
  another resident model held ~82 tok/s at a 1k prompt and did **not** recover
  when the other model was stopped, while a fresh 64k instance on the same
  quiet machine gave 92.23. Whatever placement decision llama.cpp makes at load
  is never revisited. It is also the likely shape of the 08-07 result. So the
  advice that outlived its own reversal: **measure tokens/sec after restarting
  a model server, not just that it answered** — and if it is slow, restart it
  on a quiet machine before believing anything about the flags.

- **A comment describing someone else's tool is a hypothesis, and this one was
  wrong for months.** Three places in two repos said `marimo export html-wasm`
  executes the notebook; it does not, and the two-minute experiment that settles
  it — a cell that writes a file, a notebook that would crash if run — had never
  been performed. The cost was not a bug but a *constraint*: the false claim was
  the stated reason the notebook template stayed off unattended paths, so a
  capability was withheld for a danger that did not exist. **Before a belief
  about a third-party tool becomes an architectural constraint, run the
  experiment that would falsify it.** The belief is cheap to test and expensive
  to inherit.

- **Adding the replacement is not removing the replaced.** A drag rewrite added
  list-level pointer handlers and left the old grip handlers attached, so the new
  state was never assigned and every new line was dead. The diff read correctly;
  the diff was not the thing running. It shipped, was reported unchanged by the
  person testing it, and cost a round trip. **When replacing a handler, delete
  the old one in the same edit, and verify against the built artifact rather than
  the source** — for generated assets, grep the output for the string that should
  no longer be there.

- **One asset name with two builders is one asset name with two answers.** Every
  page in the booking family links `booking.css`; two code paths built it and
  disagreed about whether the survey rules were in it; whichever page rendered
  first won. The live server took the correct path and the documentation gallery
  took the wrong one, so the only broken surface was *the one people learn from*
  — and nothing failed anywhere. **A shared output name needs a single
  definition, and a test that a real render agrees with it.** Asserting the
  presence of specific rules matters as much as asserting equality: two paths
  agreeing on the wrong thing is the other way to fail.

- **A capability that lives in the binary is a capability no agent will ever
  have, and nothing reports it.** The factory's MCP server sat at seven tools
  while its CLI grew to twenty, for six weeks, because the verbs were command
  bodies in `main.rs` and the server lives in the lib. Every test passed the
  whole time; the surface was discovered only by asking the agent to do
  something and being told it could not. **An integration whose coverage is
  implicit will drift, and the drift is silent by construction** — the only
  fix that holds is a test that enumerates one side against the other and
  demands a written decision for each gap. mecha already warned when
  `[outbox] tools` named a tool that did not exist, for exactly this reason;
  the factory had no equivalent, and that is the whole story.

- **A relative path in a deferred call means nothing without the jail it was
  written in — and the *display* forgets that before the executor does.**
  `outbox send` had always resolved a staged argument against the workspace
  recorded on the item; `outbox show` resolved it against wherever the reviewer
  was standing, so the first real staged poll reported a spec that was right
  there as "⚠ gone". The visible symptom is a false alarm; the dangerous one is
  symmetric, because a same-named file beside the reviewer would have been
  printed, and offered to open, as the draft's source — a human reading one
  file while approving another. **When a value is only meaningful in a recorded
  context, every surface that touches it needs that context, not just the one
  that executes.** A review surface that shows the wrong bytes is worse than
  one that shows none.

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

- **A relative path is a bet that "here" never moves, and "here" moved.**
  `--mcp-file` joined fixture-server paths against the file's directory but
  left them relative — correct for as long as mecha spawned MCP servers from
  the invocation directory. When servers started spawning in the run's
  workspace (the fixed-root work), the join silently resolved against the
  workspace and every fixture handshake failed; the eval that would have
  caught it was not run again until the graph rename forced it. Canonicalize
  at the seam where a path is *read*, not where it is used — and when a
  subsystem changes its working directory, grep for every relative join that
  was betting on the old one.

- **A rebuild ignored the permission mode, and the status line kept asserting
  it anyway.** `apply_switch` passed the *retained* ask-mode approver into
  every `/model`, `/provider` and `/mcp` rebuild without consulting `app.mode`,
  so the mode silently reverted to `ask` — and it fails in **both** directions,
  which is what makes the symptom hard to recognise. From `allow` it goes to
  asking: tighter, merely annoying, and the version that actually showed up in
  testing as an approval prompt thirty seconds after `/mode allow`. From
  `read-only` it also goes to asking, and *that* one loosens — read-only
  refuses outright, where asking lets a human be talked into yes, which is what
  an injection is for. Common to both, and the sharpest part: the status line
  went on displaying the old mode, so the interface asserted a posture the
  harness was no longer holding. The silently-degrading-sandbox shape, in the
  one surface whose entire job is to say what the harness will currently allow.
  **An interface that displays a security posture must read it from the same
  value the enforcement reads** — here that meant making the mapping a function
  (`approver_for`) used by both, and asserting it with `Arc::ptr_eq`, because
  "did it reuse the retained one" is the only question that distinguishes a
  reinstated approver from a fresh one. Invisible to unit tests, which build
  the types directly and never take the switch path.

- **Claiming deleted the work before anything delivered it, and one path out
  did not deliver.** Text typed into a mirrored Slack thread was removed from
  its inbox, pushed onto the run's steering queue, and lost — because the queue
  drains at the *top of the next turn* and a run with no tool calls never has
  one. Nothing anywhere held a copy. Latent in keyboard steering the whole time;
  the remote path only made it likely, because lines arrive on a poll decoupled
  from run boundaries. If a handler removes work from a store before handing it
  on, every exit from that handler must either deliver it or put it back —
  including the exits you did not write, like a run that simply ends.

- **`interactive` is a claim about who is watching, and a detached child is
  never watching.** `mecha questions answer` resumed a delegated run with
  `setup::prepare(&opts, true)`, which is right for the terminal it was
  written for and became wrong the moment a web surface wanted to spawn it:
  detached, stdin is `/dev/null`, `TerminalApprover` reads `Ok(0)`,
  EOF-is-not-consent turns it into `"n"`, and every approval becomes
  `Decision::Deny("the user declined this call")` — the string the learning
  miner reads a *correction* out of (Learning → "a refusal nobody made"). Not
  a failed run: a run that teaches rules from a person who was never asked.
  `tasks work` had already learned this and carried the flag; the second door
  onto the same conversation did not. **When one entry point takes a posture
  argument, every entry point onto the same run needs the same argument** —
  and the posture must be passed in rather than sniffed from a tty, or it
  becomes a property of how the process happened to be launched.

- **`read_only` decides approval, never ordering.** `show_file` called in the
  same turn as the `fs_write` that made the file found nothing there:
  `agent.rs`'s `join_all` runs every *approved* call in a turn concurrently, and
  `read_only` only decides whether approval is asked for. `fs_read` has always
  had this. A flag named for one property will be read as guaranteeing a second
  one — the fix was the tool description, because there is no tool-level lever
  for ordering at all.

- **Failing closed on a thing that was never a record.** The connector's
  routing lookup iterated `~/.mecha/remote/` and joined `record.json` onto each
  entry, treating only `NotFound` as skippable. A stray file there — a
  `.DS_Store`, an editor backup — makes that `ENOTDIR`, so the fail-closed arm
  fired for *every* owner message in *every* thread and took the whole remote
  control down until someone found the file. Fail closed on a record you cannot
  read; skip a thing that was never a record. The two are not the same
  question, and one directory listing conflates them.

- **A predicate that encodes who writes to a structure breaks silently when a
  second writer arrives.** The D10 voice block was injected when
  `convo.is_empty()` — correct, and *only* because a voice conversation had
  exactly one author. The moment talking and typing shared a message list
  (D3), that predicate quietly changed meaning: a call opened into an
  existing chat would find the conversation non-empty and send no block, so
  the reply came back as markdown with headings, read aloud. It fails in the
  direction that looks like working software, not the direction that errors,
  and no test could have caught it because the condition was still true of
  everything the test knew about. The general form: when you add a second
  writer to anything, grep for predicates over its *contents* — they are
  assertions about authorship wearing a shape check. The replacement carries
  the fact explicitly (`WebSession::last_turn_spoken`), because "was the
  previous turn spoken" is not recoverable from the messages once both doors
  record identically.

- **A smoke test that provokes refusals writes into the corpus something else
  thresholds on.** Driving the D3 path by hand meant deliberately triggering
  read-only denials to prove the posture, and every one of them landed in a
  session JSONL that `runlog` reads and `mecha doctor` computes a per-model
  tool-error rate from — with a 20-run floor, so a handful of provoked
  failures move it. The traces were removed afterwards. Before hand-driving a
  path that fails on purpose, know which store is counting: the run-quality
  corpus has no notion of "this run was a test", and it is the one place
  where deliberately-caused failures are indistinguishable from the real
  thing.

- **A test that asserts prose freezes the prose.** The thread header said
  "nothing typed here reaches this session" — true when it was written, false
  one rung later, and posted to every reader in between. The test asserting
  that exact sentence is what kept it alive through the change that falsified
  it. Where a message states a *capability*, assert the capability's current
  shape and assert the old wording is **gone**, or the test becomes the reason
  the lie survives.

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


- **A clean merge is not a correct merge, and only a compile-time fact caught
  it.** Two sessions each added a slash command, so each changed
  `pub const NAMES: [&str; 16]` to `17` and added its own entry. git merged the
  hunk with no conflict marker and produced **18 elements declared as
  `[&str; 17]`** — a length that agrees with neither side. Nothing about the
  diff looks wrong; it was caught because the array length is checked by the
  compiler. The lesson generalises past git: where two edits to one list are
  likely, keep a fact about the list that a machine verifies, and never resolve
  a silently-merged hunk by eye. Three sessions worked one repository that
  night and this was the only merge hazard that a careful reading would have
  passed.

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
- **A graceful degradation is worthless if its consumer cannot parse the
  degraded form.** `mail_recent` fans out across accounts and is built to
  survive a lost one: it returns the surviving rows plus a trailing plain-text
  note naming what failed, and errors only when every account fails.
  `mecha mail classify` read that with `serde_json::from_str`, which rejects
  the note as trailing characters — so the one caller the mechanism existed to
  protect was the only thing that could not use it, and a revoked credential
  on *any* single mailbox failed the sweep for **all** of them, including the
  mailboxes that answered fine. The nightly had been pinned to
  `--account dartmouth` for a year to avoid it, which made the workaround look
  like a decision about Google's token lifetime rather than a bug. The general
  lesson is about where the contract lived: `unified.rs`'s module header states
  *"A failed account never sinks a fan-out"* in prose, and prose in a producer
  cannot check its consumers. **When a component advertises that it degrades,
  test a caller against the degraded output, not just the happy one** — the
  partial response is a distinct wire format and nothing was exercising it.
  (2026-08-25.)

### Review process

- **Everything after clap's `--` is a positional, so a fix for argv
  injection broke reject completely — under a suite that stayed green
  because nothing ever spawned the child.** #126's separator fix put
  `--reason` *after* the `--` that guards a URL-supplied id, and clap
  parsed it as an unexpected second positional: every reject on the two
  stores that *require* a reason exited non-zero, 100% of the time, while
  1,986 tests passed — the argv lived inline in an async handler, so no
  test could reach it and none spawned a real child. The repair moved
  composition into a pure `decide_argv` (flags first, separator, then the
  id), added shape tests, and ran one real reject through the handler
  against a fabricated candidate in a scratch `MECHA_HOME`. Three review
  waves that evening each caught what the previous wave's fix introduced.
  Two general shapes: **flags go before the separator — `--` ends option
  parsing for everything after it; and argv composed inside a handler is
  argv no test can see — hoist it pure, then spawn one real child, because
  a mutating path verified only by reading is unverified.**
- **A PR can merge "successfully" into the wrong branch, and every signal
  reads green while it does.** Landing #112 (2026-08-29): `gh pr edit 112
  --base main` failed with a GraphQL error that scrolled past as a
  Projects-classic deprecation notice, so the retarget never happened —
  and `gh pr merge` then merged the PR, state MERGED, into its *original*
  base branch, twenty seconds after that branch itself had merged to main
  as #111. Checks green, PR closed, `origin/main` silently missing the
  entire change; the deploy under way would have shipped a binary without
  the closure guard while `mecha tools` and the suite on the merge branch
  both read current. Caught by pinning the deploy worktree to
  `origin/main` and noticing the tip was #111's commit, confirmed
  independently by a peer session reading the compare API. The general
  lesson is the skill's own rule pointed at git: **verify the retarget
  from the API before merging, and verify the merge landed on the branch
  you meant from the branch itself** — a merge's success message names the
  PR, not the base you believed it had, and an error dressed as a
  deprecation warning is still an error.

- **A guard's presence check must never read data the guarded side
  supplies.** The closure guard's `wrap` and `verify` first keyed
  "already guarded" off `description().ends_with(GUARD_NOTE)` — and a
  `kg_task_update`'s description arrives over the MCP wire, so a server
  whose description happened (or was crafted) to end with the guard's own
  sentence was left unwrapped *and* passed the startup verification: a
  fail-open in the very check built to make the guard's presence
  structural. The fix put the answer in the type
  (`Tool::guards_closures`, default false, overridden only by the
  in-process wrapper), leaving the description cosmetic. The transferable
  form: any "is the protection installed?" check is itself part of the
  attack surface, and it must key off something the protected party
  cannot author — the same reason `Capabilities` are declared in code
  rather than parsed from a server's self-description.

- **A fix that closes the exact case described leaves the structurally
  identical sibling standing right beside it, four times running on the
  same two-function surface.** #101 (rung 9's episode-tagging/surprise-
  detection review fixes) took four rounds to settle, and every round found
  a gap of the same *shape* as the one the previous round had just closed,
  never a new kind of problem. Round 1 made `mecha distill`'s surprise print
  escape ANSI (`strip_ansi`); round 2 found the same function does not stop
  a bare `\r`/`\n`, which forges or rewrites a rendered line exactly as
  effectively as an escape sequence does, and added
  `strip_ansi_and_controls`; round 3 found `is_control()` — the very check
  the round-2 fix used — is Unicode category Cc only, so U+2028/U+2029
  (forge a line break, the round-2 case again, through a character
  `is_control()` does not name) and bidi overrides (reorder the rendered
  line, the Trojan Source shape) sailed through the same filter untouched;
  round 4 found the fix's own doc comment had misdiagnosed *why* it was
  needed — a claimed precondition ("the caller already splits at `\n`")
  that was never actually true even at `strip_ansi`'s two pre-existing call
  sites, exposing a real, unrelated bug in the TUI's own log capture. The
  store-read half of the same PR repeated the pattern independently: round
  1 bailed episode tagging on a hard I/O error reading the outbox; round 2
  found `OutboxStore::items` also silently *skips* a merely malformed item
  file and still returns `Ok`, needing a new `items_strict`; round 4 found
  the cause cited for needing it — a half-written file mid-save — was one
  the store's own temp-sibling-and-rename discipline had already ruled out
  structurally, so three rounds had been narrating the wrong mechanism for
  a real fix.

  The general lesson: when a review finds "X doesn't handle case Y," the
  reflex has to be *"what is the general class Y belongs to, and does this
  fix cover the whole class or just Y"* — asked before shipping, not
  rediscovered when the next round finds Z. A fix scoped to the reported
  instance reliably produces another instance of the same class, and each
  round here cost a full review cycle to find what one pass of "what else
  has this shape" would have caught for free. It is the same failure named
  elsewhere in this file one layer up — a cleanup pass scoped to "the
  findings I already know about" answers a narrower question than the one
  that matters (the #89/#94 trap, above) — recurring here one level down,
  inside a single fix rather than across review rounds of a whole PR.

**2026-08-31 — seven review rounds on #128 and five on #130, and after the
first round every finding was a defect the previous round's fix had
introduced.** Worth recording as three distinct shapes rather than one, because
they fail differently.

- **The checked part was not the load-bearing part.** `withoutJudged` rebuilt
  a cached grouping and needed an id-to-statement map to promote a new leader.
  The only thing available was `sample`, whose alignment with `members` is
  *another repository's* serialisation detail — true today, decorative
  everywhere else it is consumed, and nothing in this repo would break if the
  graph started sending the top three by cosine. A person would: the promoted
  id becomes `leader_id`, and a group verdict files on `leader_id` under the
  statement on the card, so a changed mapping shows one candidate's words over
  Accept-all and votes on another. The induction over it was sound and had
  been verified; the base case was in a repo the verification never reached.
  The fix was to stop needing the belief — drop the group rather than re-head
  it — not to document it. **An assumption whose base case lives in another
  repository is not an assumption this repo's tests can hold.**

- **A test that pins a count forbids the correct change.**
  `a_verdict_reaches_listings_other_than_the_one_on_screen` asserted
  `withoutJudged` was called exactly twice. The bug the next round found was a
  *third* install path that was missing the call — so the test written to catch
  that gap would have failed the fix for it. Assert the property (every path
  that reaches the screen is filtered), never the arity.

- **A warning's position is part of whether it exists.** The `/queues` status
  line is a `Paragraph` in a `Rect { height: 1 }` with no `.wrap()`, so it
  clips at 76 columns. A caveat written *after* a 48-character
  statement head, or after a child-controlled `FAILED` line, was simply not on
  the screen — and its absence read as "nothing to report", which is the exact
  failure the caveat existed to prevent, one layer down. Bounded facts first,
  unbounded strings last. The first attempt to test this re-derived the
  geometry (`76`, hand-copied from three constants in `queues.rs`) and would
  have gone green through any change to them; drawing the modal into a
  `TestBackend` and reading the buffer catches both the ordering *and* a
  ten-column indent change in a file the test never names.

  The general lesson across all three: **a review round that finds only what
  the last round touched is not converging, it is following you.** Each fix
  here was correct about the thing reported and created the next finding, and
  what stopped it was not more care per round but changing what the tests were
  about — from arity to property, from arithmetic to rendered artifact, from a
  documented belief to no belief.

**2026-08-31 — an inability is not a request.** A peer session, blocked by its
own worktree guard from writing `docs/OPERATIONS.md`, asked this lane to write
a line into it — phrased as "you can reach that file and I cannot". It was
safe here only by accident: the finding was independently this lane's (the
swap state was read here, and the build was capped at `-j 4` because of it),
so there was an honest reason to write it that did not depend on the request.
Had it been *only* the peer's finding, the correct answer was the human, not
the reachable file. **A guard that stops one lane is not a task that lane can
hand to another, and the reachability of a file is not the authority to write
it.** The phrasing is the peer's own, offered unprompted after the fact; both
sessions had spent the day refusing the same shape in other forms without
noticing it in this one.

**A green check is not evidence a review happened.** On 2026-09-01 the
`Claude PR review` job on a new repo reported SUCCESS and posted nothing,
three separate ways: no credential at all (the action validated, then did
nothing); a PR that *modifies the workflow*, which the action refuses to run
and exits success on; and a review that completed — 39 turns, real model
usage — then took 14 permission denials trying to publish, because the
prompt had been ported without the `gh pr comment` instruction or the
`--allowedTools` grant. A PR was merged on the strength of the first. **The
artifact is the comment, not the tick**: assert a comment exists before
treating a review as done, and note that a workflow-touching PR structurally
cannot review itself and needs a human read.

**A trailing newline is present, non-empty, and invalid.** A
`CLAUDE_CODE_OAUTH_TOKEN` set with `echo` carried a `\n`, passed every "is
it set" check, and was rejected at the first API call — `is_error: true`,
`modelUsage: {}`, 1.9 seconds. Three re-runs looked like three identical
failures. The load-bearing diagnostic was the secret's `updated_at`
timestamp, which distinguished "the new token is also bad" from "the new
token never arrived" — twice it was the latter. **Use `printf '%s'`, and
check the timestamp before re-running anything.**


### Environment

**2026-08-30 — this box's clippy is 1.97, CI's is 1.98, and an `async fn`
hides its `Result` from the older one.** `result_large_err` cannot see
through the future wrapper on 1.97, so a `Result<String, Response>` return
lint that fails CI's `-D warnings` build reports nothing locally — PR
#126's clippy job ran red while every local check was green, and the
finding surfaced only in `gh run view --log-failed`. The 1.98 toolchain
was already installed here: `cargo +1.98.0 clippy --workspace
--all-targets --all-features` reproduces CI exactly and is now the
pre-push gate. General shape: **a local lint's green is evidence only at
CI's own toolchain version — pin the invocation, not the habit.**

**2026-08-30 — the export gate refused the whole public release over
fixture names, thirteen files deep.** Test fixtures and doc comments in
the private graph repo had accumulated real names — family, colleagues,
the university — since the previous export, each legal where it sat and
all of it unpublishable, including one written *that same morning* by a
session that knew the no-real-people rule from the public repo and did
not apply it in the private one. The strip cost an evening pass and two
broken substring tests. General shape: **fixture rules follow where the
code is going, not where it sits — a private repo with a public export is
a public repo with a delay, so write the fictional cast from the first
line.** (The gate's tree-destroying design also proved itself twice: a
`tail -3` on the export truncated the hit list to one term and briefly
misread a 13-term refusal as a one-term residue — the gate's own header
records why a gate that only *reports* trusts every caller to check.)

**2026-08-30 — `node_modules/` with a trailing slash ignores a directory,
not a symlink wearing its name.** A worktree lane symlinked
`web/node_modules` at the primary checkout's copy to skip an `npm ci`,
and `git add -A` swept the symlink into a commit — both `.gitignore`s say
`node_modules/`, and the trailing slash makes the pattern match
directories only, which a symlink is not. It rode a merge onto public
`main` (`0b976c1` → #120) carrying an absolute path into this machine's
home, and announced itself only as a perpetually-deleted file in `git
status` after the next `npm ci`. Removed, and the ignore hardened to
`node_modules` (no slash) so the next shortcut stays untracked. General
shape: **a gitignore audit that only checks the pattern exists can still
track the thing — the trailing-slash rule quietly scopes the pattern to
one file type, and `git add -A` in a worktree stages whatever slips
through.**

**2026-08-30 — a cancelled CI run is neither a pass nor a fail, and this
class of loss leaves a green-looking history.** The docs workflow's
pull-request builds and main's Pages deploy shared one `pages` concurrency
group, and GitHub holds only one *pending* run per group — so a PR build
evicted the pending deploy of PR #118 (run 33256024329, created 13:50:09Z
on 2026-08-29, zero jobs run, cancelled the second the next PR run was
created; verified against the run's own API record). Nothing red anywhere:
the deploy simply never happened, and the site stayed one PR stale behind a
green checklist. PR #123 (another lane's, docs.yml only) serialises deploys
against each other and gives PR builds per-branch groups. General shape:
**an eviction shows up nowhere you normally look — when a deploy's absence
is the symptom, search the workflow history for `cancelled` runs, not for
failures.**

**2026-08-30 — a rerun measured the code from before the fix, and the log's
own symptom was what caught it.** After editing the replay wrapper,
`cargo test -p mecha-core --lib` rebuilt the library and its tests — and left
`target/debug/mecha` exactly as it was, so the "verification" rerun measured
the unfixed binary for half an hour. Nothing in the invocation looked wrong;
the tell was that the warning the fix should have removed was still being
printed. Two general shapes: **a package-scoped test build is not a binary
build** (`cargo test -p X --lib` proves the lib, not the bin you are about
to run), and **after a fix, grep the next run's output for the symptom** —
its absence is the cheapest artifact-level proof there is, and its presence
is unfakeable.

**2026-08-30 — a gitignored file does not exist in a worktree, so the rule
"route specifics to OPERATIONS.md" silently forks it.** `docs/OPERATIONS.md`
is gitignored (`.gitignore`, confirmed via `git check-ignore -v`) and
`git worktree add` carries only tracked files — a lane following the
inventory-split rule from a worktree writes a second, divergent copy the
real file never learns about. General shape: **a convention that names a
gitignored path binds only the main checkout**; a worktree lane hands the
fact to whoever sits there. (Found by the perf lane, verified from the main
checkout the same hour.)

**2026-08-29 — deleting the A records would have left browsers on the old
page while every terminal check looked right.** Squarespace's DNS preset
carried an **HTTPS (SVCB) record** beside the A records, with an
`ipv4hint` naming all four Squarespace addresses. Modern browsers consult
the HTTPS record and can connect from its hints; `dig` and `curl` do not
unless asked (`dig HTTPS`). So repointing only the A records would have
produced a state where `dig A` and `curl` proved the migration done while
Chrome and Safari kept landing on the Squarespace page — two truthful
instruments disagreeing because they read different record types. Deleting
the whole preset *group* is what removes the SVCB record. Two general
lessons: **check the record types your clients actually consult, not the
ones your tools default to**; and after any DNS change, the authority and
every cache disagree for a full TTL — the first post-change check must be
against the authoritative nameserver (`dig @nsd1…`), because a cached
answer that contradicts a fresh claim is evidence about the cache, not the
claim (this pass hit exactly that: the local resolver held the four old A
records with an hour of TTL left while the authority already answered the
droplet).

**An edit tool that silently does nothing shipped three dead features in one
day.** Patches applied with python's `str.replace` return the string
*unchanged* when the pattern does not match, and the pattern drifts the moment
`cargo fmt` reflows a line. Asserting on some patches and not others meant
three landed as no-ops: a forensic probe swept into a commit, a TUI border
title that never gained the `m merge` key it advertised, and an `Enter`
handler that stayed `=> {}` while the footer said "Enter read it". **Every one
compiled and passed its tests, because a patch that does nothing breaks
nothing.** Assert that the pattern matched before writing, always — a
silently-skipped edit is indistinguishable from a finished one, and the tests
that would catch it are the ones nobody writes for code they believe exists.

**And the compiler had been saying so, past a reviewer counting warnings
instead of reading them.** `field 'show' is never read` sat in every build
between introducing that handler and fixing it. The check being run was
`clippy | grep -c warning`, compared against a known baseline — which answers
*"did I add a warning"* and never *"what is this warning saying"*. A count
turns a diagnostic into a regression test and throws away the diagnosis. Read
the text of anything new; compare counts only to decide whether to look. Two
independent signals said it, not one: `review_detail` was assigned `None` in
three places and `Some` in none, which is the same fact stated in the source
rather than in the build output. Neither was read until someone went looking
for a cause, which is the tell — both were *available*, and availability is
not the same as being consumed.

**A correct filter over a truncated input, and neither half looked wrong.**
`resolve_entity_all`'s fuzzy tier took `LIMIT 5` with no ordering, so against
13,400 event nodes a person was never reached; a surname search returned five
events and no people. The rule directly below it — *drop retrieval-target
types when something stronger matched* — was working perfectly and had nothing
to drop to, because the LIMIT had spent every slot before a person got in.
Ordering the query made the existing rule *reachable* rather than replacing
it. When a filter appears to be failing, check what reached it: a component
that is right about the wrong input reads as correct in isolation, and so does
the one that never sees the case.

**A maintenance verb whose `--dry-run` defaults to false rewrote seven nodes
for someone running it as a survey.** `mecha-graph fix-person-names` was
invoked to *see* what it would do. The renames were all correct and nothing
was lost, which is luck rather than design. For any pass that can rewrite in
bulk, reporting is the default and `--apply` is the opt-in — the inverse
costs nothing when someone means it and costs a restore when they do not.

**A live config edit shipped before the schema was installed took down every
older binary at once.** Adding `[web]` to `~/.mecha/config.toml` while only a
branch build understood it made config parsing fail for the installed
`mecha` behind the Slack connector and the trigger daemon — `ConfigLayer` is
`deny_unknown_fields`, so an unknown section is a startup error, which is the
right strictness pointed the wrong way in time. The order is: merge the
schema, reinstall every binary that reads the file, and only then point the
live config at it. A section a sibling binary cannot parse is an outage, not
a preference.

**Production pointed into a session worktree, three separate times.** The web
assets path, the voice worker's WorkingDirectory, and the tailscale page
mount all named `…/.claude/worktrees/<session>/…` — each worked on the day it
was set and each would have broken silently when the session directory was
cleaned. Production paths must survive the session that created them:
build artifacts get a stable home (`~/.mecha/web/dist`), units point at the
main checkout, and the deploy skill carries the copy step so nobody
remembers it.

**A facade announced its port before binding it.** The voice facade printed
"voice facade on 8990" while another process held the port; the bind failed
in an unread task. Announce after the bind answers, or fail loudly — a
startup line describing an intention as a fact is the silently-degrading
shape in one sentence.


**A secret in `~/.bashrc` is invisible to every systemd unit.** `EXA_API_KEY`
exported there worked perfectly in every hand-test and would have reached no
scheduled or connector run at all — and those are where unattended web search
actually happens. `systemd --user` reads `~/.config/environment.d/`, and
nothing else; `.bashrc` additionally returns early for non-interactive shells,
so even a script would have missed it. Found by reading
`/proc/<pid>/environ` of each service rather than by testing in the shell that
had just exported it. The same audit showed `ANTHROPIC_API_KEY` had *never*
been visible to any unit, harmless only because every unit runs a local
provider. **A secret's location decides who can read it, and the only proof is
reading it back from the process that needs it — never from the shell you set
it in.**

**A key pasted into `api_key_env` fails silently and leaks quietly.** The field
names an *environment variable*; given a key it dutifully looked up a variable
called `b0182b45-…`, found nothing, and reported `exa: no API key` — a message
that describes a missing key rather than a misplaced one. The key meanwhile sat
in plaintext in a mode-664 file. The mistake is invited by the name: `api_key`
and `api_key_env` differ by a suffix and accept the same-looking string.
**A config field whose value is a name rather than a datum should say so in the
error when what it got was obviously a datum** — the diagnosis was two minutes
of reading and would have been zero.

**An install is not a restart, for anything already running.** `cargo install`
replaced `~/.cargo/bin/mecha` while a TUI session begun two hours earlier kept
executing the inode it was launched from — `/proc/<pid>/exe` reads
`mecha (deleted)`, which is the only visible sign. The session had none of the
new code and looked like the feature failing. The check that catches a stale
*process* is not the one that catches a stale *file*: walk `/proc/*/exe`, never
`pgrep` for the install path, because a binary launched off `$PATH` has argv
`mecha tui` and the first version of that check missed exactly the interactive
session it was written for. **A check must be run against the case that
motivated it before it is trusted.**

**A version string is not evidence when the change did not bump one.** The same
install left `mecha --version` reading `0.1.9` before and after, which is
indistinguishable from having skipped it entirely. What proved it was a
behaviour the change introduced (`mecha run --help` carrying `--image`) and
cargo's own line reporting it had replaced an install made from a *different
worktree*. **Verify a behaviour, not a proxy for one.**

**Adding a config key breaks every binary that predates it, including one
already running.** `ProviderConfig` carries `deny_unknown_fields`, deliberately,
so a typo'd setting fails loudly rather than being ignored — which makes
`~/.mecha/config.toml` a *wire format between versions*, not just a type, the
same lesson `Proposed` and `OutboxKind` already carry one layer down. The
failure is deferred and partial, which is what makes it expensive: the old
process parsed the config at startup before the edit and kept working, then
failed hours later on a path that *re-reads* config — here `show_file`, which
loads the whole global config at call time for one number — and reported a
config parse error in a subsystem with no visible connection to what changed.
The model went off reading `config.toml` to investigate. **Install before
editing config, and restart anything long-lived after.**

**`--mmproj-auto` is enabled by default and does nothing where it is needed.**
It only fires for `-hf` downloads, and every start script here uses
`-m <path>`, so a multimodal model was served with no vision tower for months
while the flag list looked handled. Nothing errored: the server started,
answered well, reported `modalities.vision: false`, and the model told anyone
who sent it a screenshot that it could not see images. **A default that is a
no-op in your configuration is worse than an absent one — it reads as
handled.**


- **A working copy can be a running service's `ExecStart`, and then every git
  reflex is a deploy.** `~/.config/systemd/user/llama-local.service` names
  `/home/ljchang/Github/mecha/scripts/start-moe-mtp.sh` literally — not a copy,
  not an installed path, the file in the development checkout. So `git stash`,
  `git checkout --` and `git restore` on that path silently rewrite the launch
  command of an active unit, and the damage does not surface until the next
  restart, when the server comes back healthy and merely behaves differently.
  Found on 2026-08-20 with three arcs sharing the checkout, only because one of
  them asked the other two what their dirty files were. **The lesson that
  transfers is about the standard advice, not about the file**: "work in a
  worktree" assumes nothing outside the repository points at a path *inside*
  it. Here systemd does, so relocating that arc would have broken the running
  server rather than protected it — the advice in CONTRIBUTING.md has this hole
  and the checkout is the thing to check before following it.

- **The fact was written down correctly in the file nobody consults, and wrongly
  in the two that everybody does.** `scripts/start-moe-mtp.sh` has said since
  it was written that llama-server "silently splits `-c` across" its parallel
  slots — the exact `context_window = -c / -np` relationship. Meanwhile
  `docs/HANDOFF.md`'s Environment row stated `context_window` (= `-c`) and
  CLAUDE.md's Context section says the same. The script's own strategy was to
  keep `-np 1` *so that* the wrong rule would stay accidentally true, which is
  a rail that works right up until someone changes `-np` for throughput —
  which is what happened. Nothing broke, because the new geometry
  (`-c 1048576 -np 4`) lands on the same 262144 per slot; the rule was still
  wrong and four derived numbers trusted it. **A fact recorded in one place and
  contradicted in two is not recorded**, and the place it survives is rarely
  the place a reader looks. **Nothing verifies the relationship as of
  `d72e82d`** — `git grep -nEi 'n_ctx_slot|total_slots|/props' -- '*.rs'`
  returns nothing on `main`. The shape a fix would take is argued in
  HANDOFF under *Cheap, and worth doing first*; it is unbuilt, and this entry
  records the trap rather than a remedy.

- **A design written into the record of what happened will be read as having
  happened.** The paragraph above originally ended with the argument for a
  startup check — a good argument, in past tense, in a file that is past tense
  by construction. Within an hour a second session had read the commit that
  added it as a shipped feature and stood down from the area to avoid duplicate
  work, and a third had been told the same. Nothing was built, and the absence
  was now defended by two people believing it was present. HISTORY cannot hold
  a proposal, because there is no wording that survives being skimmed in a file
  whose every other paragraph describes something that exists; the open-work
  document is where a proposal reads correctly. The general shape, and it is a
  sibling of the entry above rather than the same one: that was *a fact
  contradicted elsewhere is not recorded*; this is **a proposal filed under
  completed work is indistinguishable from completed work**. Both fail the same
  way — silently, and with the reader confident.

- **A heredoc can eat a line continuation before the compiler sees it.** Five
  doctor messages written through a Python heredoc lost their `\`-continuations
  to the *heredoc's* parser, so the string literals reached the file as plain
  multi-line text and `cargo fmt` joined them with their indentation intact —
  twenty-space gaps mid-sentence in everything doctor printed. No test failed;
  one line of real output made it obvious. Run the thing whose output is prose,
  and do not trust that what you typed is what landed in the file.

- **`cargo install` will happily install a version older than the one you just
  published**, because the local registry index is cached and nothing warns.
  Minutes after `mecha-factory-publish` 0.2.1 and `mecha-mail` 0.1.1 went up,
  `cargo install` fetched 0.2.0 and 0.1.0 — and reported success both times.
  It was caught only by reading `cargo install --list` against the versions
  that were meant to land. **After installing something you just released,
  verify the version rather than the exit code**, or pin it with `--version`.
  The failure is silent and looks exactly like success, which is the whole
  problem.

- **Publishing a workspace is ordered, and a missing member fails it halfway.**
  `mecha-cli` depends on `mecha-slack`, and cargo refuses to publish a crate
  whose non-dev dependencies are not on the registry — so tagging v0.1.1 with
  `mecha-slack` absent from the release workflow's list would have published
  `mecha-core` and `mecha-mail`, then died on `mecha-cli`, leaving a version
  that can be yanked but never unpublished. Caught by `cargo publish
  --dry-run` before the tag existed. **A new workspace member that anything
  published depends on belongs in the release list in the same change that
  introduces it**, and a dry run is the cheap way to find out.

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
- **`cargo install` replaced a binary built from a different checkout, and
  the only warning was one line of its own output.** Three sessions wrote
  `~/.cargo/bin/mecha` on 2026-08-24 from three different checkouts of the
  same repo — one from `main`, one from a worktree on a feature branch, one
  from `main` again — each believing it was installing "the" binary. The
  second install would have silently dropped a shipped web feature from
  production; it was caught only because `cargo install` prints
  `Replaced package ... (/path/to/other/checkout)` and someone read it. It
  did not break anything, and that is the interesting part: the running
  process holds the *replaced inode*, so the regression sits latent on disk
  and activates on the next restart — systemd's, a reboot's, or anyone's.
  **An install is not a source.** The update skill already says a tag is not
  an install and a restart is not a reinstall; the corollary is that the
  artifact does not tell you which tree produced it, so on a machine with
  worktrees per session, verify the checkout before installing and `cmp` the
  result after. The fix that generalises is coordination, not care:
  the three sessions messaged each other, agreed who merged last, and that
  session did the single install and restart. (2026-08-24.)
- **A library that segfaults instead of raising turns a config error into a
  dead service.** `sherpa_onnx` accepts `modeling_unit="bpe"` with no
  `bpe_vocab`, builds the recognizer successfully, and then **segfaults
  inside `create_stream()`** — no exception, no message. It was found while
  pricing hotword biasing for the STT leg, which is the leg that feeds the
  microphone: had a nightly job generated a hotwords file the library
  disliked, `:8992` would have died and voice would have gone deaf with no
  error text anywhere. **Anything generated that feeds a native library must
  be validated in a subprocess first**, because the failure mode of a crash
  is indistinguishable from the failure mode of a service that was never
  started. (2026-08-24.)
- **The serve unit's `WorkingDirectory=%h` gave spawned CLI children a cwd
  the workspace jail correctly refuses**, so routes that shell out failed on
  containment for reasons that looked like a jail bug. Fixed narrowly for
  mail first, whereupon the tasks board promptly rediscovered it. **A fix
  that belongs at a spawn helper does not belong at whichever route hit it
  first** — it now lives in `serve/mod.rs::child_cwd`. (2026-08-24.)
- **A layout bug that needs *scale* to fire passes every short-list test, and
  a sibling page copied from it inherits the fuse.** A flex item with
  `overflow` other than `visible` has an automatic minimum size of **zero**,
  so the phone's mail cards collapsed to 30px slivers and clipped their own
  text — but only once the list outgrew the viewport. The Outbox page carried
  the identical latent bug and had simply never held enough drafts to trigger
  it. **When a bug turns out to need volume to appear, grep for its shape in
  every page that was copied from the one that broke**, rather than fixing
  the instance. (2026-08-24.)
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

- **A gitignored file in a disposable worktree dies with the worktree.** The
  mecha-graph OPERATIONS.md — the machine-specific values extracted from a
  public doc — was written inside a scrub worktree, gitignored by design, and
  destroyed when the merged worktree was removed. Nothing warned: git had
  never tracked it, so no tool considered it a loss. It was recreated from
  the pre-scrub file's git history, which happened to contain the same
  values. The lesson: gitignored means "no home but its directory" — a file
  meant to outlive a branch must be written in the main checkout, and a
  worktree should be treated as already deleted from the moment it is made.

- **And the mirror of that: something outside a worktree can point *into*
  it, and git cannot see the dependency.** A cleanup pass on 2026-08-25 was
  about to remove seven merged worktrees when `scripts/voice/mecha-voice-serve.service`
  turned out to carry
  `ExecStart=…/.claude/worktrees/voice-arc/target/release/mecha`. The unit was
  disabled, so nothing would have broken that day — it would have left a
  repo-tracked unit file naming a deleted binary, waiting for whoever
  installed it next. The same worktree had already cost a day earlier: a
  container was bind-mounting `/srv` out of it, three commits stale, and
  worked only because one file happened to be byte-identical. The lesson is
  about what the obvious check actually answers: **"0 commits ahead" tells
  you a worktree has no unmerged work, not that nothing depends on it.** A
  worktree is a filesystem path, so anything taking a path can name it —
  systemd units, container mounts, a process's cwd — and `git worktree list`
  knows about none of them. Before removing one, grep `scripts/`,
  `~/.config/systemd/user/` and the live docker mounts for its path, and
  check no process has its cwd inside it. Found by one session and
  generalised into a sweep across all seven by another, which is the only
  reason it is stated here as a rule rather than as one unit file.
- **The shared-tree `git add -A` trap recurred, twice in one day**, nineteen
  days after the entry above was written (2026-08-25: a forensic probe swept
  into a graph commit, then another session's uncommitted `HANDOFF.md` edit
  swept into `ae757b1`). Neither caused damage and the second was verified
  line-by-line to have landed intact — but "verified intact" was luck, not
  process. A rule stated once in a history file is not a habit; the session
  involved adopted explicit-path staging only after being shown the second
  instance. If two lanes are in one tree, the check that works is staging
  named paths, not reading a clean-looking `git status`.

- **And explicit-path staging is not enough, because the path can be shared.**
  A third instance, 2026-08-26 — by a session that had spent the afternoon
  being careful about exactly this. It refused `git add -A`, staged by named
  path, verified a peer's route line was absent from its own commit, and
  declined to run `cargo fmt --all` because two failing sites belonged to
  someone else's in-flight file. Then it committed `docs/HISTORY.md` by name
  and took 149 insertions, of which about seventeen were its own; the rest was
  a peer mid-write in the same file. Content survived intact and was wanted —
  the cost is that a pushed commit message now describes a tenth of its own
  diff, and the record under-describes itself in the one file whose job is to
  be the record.

  The generalisation is the part worth keeping: **`git add <path>` is safe
  against files you are not editing, not against files you are not the only
  one editing** — and documentation is where that bites hardest, because the
  `handoff` skill instructs every session to edit the same two files at the
  end of the same session. Code lanes are partitioned by module and diverge
  naturally; doc lanes are pointed at one target by procedure. Either stage a
  doc file only when `git diff <path>` is exactly your own change, or write to
  a per-session scratch section and merge deliberately. Verified-intact is not
  a process; it is the outcome you get when the collision happens to be
  additive.

- **A check can only report on what it enumerates, so it is always silent
  about what you forgot to enumerate.** Three instances landed on 2026-08-25,
  in three different tools, and they read as unrelated bugs until they are put
  side by side. The `update` skill's restart list omitted `mecha-serve` and
  `mecha-voice-worker`: a pure-Rust release would install a binary nothing
  re-executed, and `cargo install`'s output cannot mention a service it never
  restarted — worse for the voice worker, which runs `worker.py` straight from
  the tree, so it goes stale on changes no install step ever touches.
  `ls .git/MERGE_HEAD` was used to answer "is another lane mid-merge here?"
  and cannot see a linked worktree, whose merge state lives in
  `.git/worktrees/<name>/MERGE_HEAD` — a false negative on the question asked
  immediately before doing something irreversible. And `git log` enumerates
  from `HEAD`, which is what actually lost `5b187c5` from an attribution sweep
  (see Traps → Measuring): the starting point was the one input nobody
  questioned, because it is the one the tool supplies for you.
  The tell they share is that **each check answered correctly about its own
  frame and silently about the caller's**, so the output looks like evidence of
  absence and is only evidence of an empty enumeration. Distinct from the
  dash-not-zero rule, which is about rendering a failed read honestly; here the
  read succeeds. The fix is never to read the output more carefully — it is to
  ask what the check enumerates and widen the frame: `git worktree list` before
  the merge-state check, a commit range instead of a walk from `HEAD`, a
  restart list derived from the units that exist rather than from the build
  output. Where widening is not possible, the honest move is to say what was
  *not* covered — the same reasoning as `MECHA_TEST_REQUIRE_BACKENDS=1`
  turning a skipped integration test into a failure, because in CI a silently
  skipped test reads exactly like a passing one.

  **A fourth instance appeared in the sentence that closed that entry**, and
  it is the one to guard against hardest because the check that would catch
  it is not a command. The paragraph originally ended by invoking "the `no
  silent caps` rule" — which grepped to exactly one hit, the sentence citing
  it. Not a rule this project dropped: **a rule from the authoring agent's
  own tool documentation, imported into this history wearing mecha's voice.**
  It was never here to go stale.

  What makes it hard to catch from the inside is that the two vocabularies
  are written in the same register. *Fail closed*, *the silently-degrading
  sandbox*, *a lane must not promote itself*, *no silent caps* — named,
  italicised invariants, indistinguishable by feel, so recall cannot
  separate the ones this repo earned from the ones an agent arrived
  carrying. The failure does not dangle visibly; it resolves to nothing.
  And it is worse than a stale pointer, because a reader who trusts it
  goes looking for a principle this project never held.

  **So: grep before citing a named rule, and require a hit that is not the
  sentence you are writing.** It costs one command. The reason it belongs
  in this entry rather than beside it is that the paragraph's own review
  enumerated the reasoning and never the vocabulary — answering correctly
  about its frame and silently about the reader's, which is the entry's
  thesis one turn further in.

**"Clippy clean" is a statement about one toolchain, and the gate runs a
different one.** `main` went red for six hours on 2026-08-25 while
`cargo clippy --workspace --all-targets --all-features` was clean on this
box, exit 0. CI uses `dtolnay/rust-toolchain@stable` and had clippy **1.98**;
the box has **1.97**, where `clippy::result_large_err` does not exist — so a
`Result<String, Response>` whose error variant is a 128-byte axum Response
passed locally and failed under `-D warnings` upstream. Both facts were true
at once, which is why the local run felt like evidence. The general shape is
the same one the `update` skill exists for: **a check is only as current as
the thing running it**, and a green local gate proves the code passes *your*
toolchain, never the one that decides. When CI disagrees with a clean local
run, read the version in the log before reading the code.

**A default nobody chose ended calls for five months of wall-clock silence.**
Pipecat's `PipelineWorker` cancels an "idle" pipeline — and the runner with
it — after `idle_timeout_secs`, default **300**, where idle means neither
side produced a speaking frame. Nothing in this repo set it, so a voice call
died five minutes into any pause: the log said `Idle timeout detected` and
the client, which has no way to be told, showed only a peer connection that
closed. It was diagnosed from the journal in a minute and would have been
unfalsifiable from the client forever. **Two lessons, and the second is the
one that transfers.** A framework default is a decision your project made by
not making it — audit the ones that *terminate* things. And a component that
stops must be able to say why: the fix raises the timeout, but the part worth
keeping is that it now announces itself over the data channel before it tears
the call down.

**A child process that reports failure on stdout and exits zero will be
believed.** `mecha-graph accept <id>` prints `#id FAILED: cannot resolve
subject …` and exits **0** — correct for a bulk run where one candidate of
five hundred cannot resolve, and a lie to every caller that keys on the exit
code. The phone's queue page did exactly that: it dropped the card it had
just sent, reported success, and counted the verdict in a sample whose whole
purpose is to produce a number somebody quotes — while the candidate stayed
pending. The TUI had already learned this and tallied the report; the web
surface, written later against the same CLI, had not. **A driver must read
its child's account of what it did, not its exit code, whenever the child can
partially succeed** — and when two surfaces drive one verb, the tally belongs
in one shared function, or the second one relearns it the expensive way.

**A number measured in a shared checkout describes a tree that exists on one
disk and in no commit — and two sessions made that mistake on the same tree
within an hour.** On 2026-08-26 three lanes were live in one working copy.
Both a handoff sweep and an independent reviewer ran `cargo test --workspace`
to write the count into `HANDOFF.md`, and both got a number that included a
third lane's *uncommitted* work: 1,405 and 1,406 against a real 1,406 and
1,408 at the commits they each named. Neither was reading the other; the tree
was simply lying to both in the same way.

Two things make this worth an entry rather than a shrug. **The error is
invisible in its own output** — `cargo test` prints a total, not a
provenance, so the number looks exactly as authoritative as a correct one.
And **the doc's format invites it**: "measured at `<commit>`" is a claim about
a commit, so the measurement has to happen on a tree that *is* that commit.

So: `git status --porcelain` is the precondition of any number you write down,
not a nicety. If it is not empty, either the number is not about the commit
you are naming, or you take it somewhere else — `git worktree add` a private
tree (note it refuses a branch already checked out elsewhere, which is itself
the signal that you are not alone in there). The corollary that caught this
one: state counts **per crate and attributed per range**, because a total
nobody can decompose is a total nobody can check — the disagreement only
surfaced because a second reader could subtract.

**An idempotent upsert writes every field it is handed, including the ones
you did not think about.** The graph's `upsert_episode` defaults
`occurred_at` to *now* when it is absent, so the first cut of "edit a note"
would have moved every edited note to today — a notebook silently rewriting
when things happened because somebody fixed a typo. Caught before shipping
only because the round-trip was tested against a real note rather than
asserted about. **When re-upserting to update, enumerate what the write
touches and carry forward everything you are not deliberately changing**; the
dangerous fields are the ones with a plausible default, because those fail
silently rather than erroring.

**A green local build does not mean CI is testing your branch — it may be
testing an implicit merge with `main`'s current tip** (2026-08-28, PR #102).
`cargo clippy --all-features` and `cargo test --workspace` were clean, on the
exact flags CI runs, against the branch's own committed HEAD — and CI still
failed with a plain field-completeness error (`RunOutcome` missing two new
fields) at a source line the local checkout did not even have: the file was
2,130 lines locally and the error named line 2,260. `main` had merged an
unrelated PR mid-review, adding a new construction site this branch had never
seen. GitHub's `pull_request` trigger by default checks out the *synthetic
merge* of the PR branch with `main`'s current tip (`refs/pull/<n>/merge`), not
the branch's raw head — so CI was truthfully reporting a real conflict between
this branch and a `main` that had moved, while every local check answered a
question about this branch alone. **A CI failure with no local repro is not
necessarily flaky**: fetch and diff against `origin/main` before assuming the
runner is wrong, because the runner may be testing a tree that does not exist
in any local checkout yet.

**A review invoked by PR number moves the checkout under you.** The standing
rule is "work `main` from a separate worktree and leave this clone where
`llama-local.service` points", and it reads as advice about where you *type*.
On 2026-09-01 `/code-review 135` put `~/Github/mecha` on a `pr-135` branch —
`gh pr checkout` resolves a PR-number target in the current repo — and left
it there. Every branch of the session's actual work was in scratchpad
worktrees and it tripped anyway; a peer found it hours later, and it was
inert only because `scripts/start-moe-mtp.sh` happened to be byte-identical
on both. **A rule about where you type does not cover a command that moves
the clone for you.** Check `git branch --show-current` in the primary
checkout after any tool that takes a PR number.

**The instructions can be the stale artifact, not just the binary.** On
2026-09-02 the Skill tool served a cached `update/SKILL.md` from before the
commit that inverted the graph's source repo — it still said to build from
the private checkout and called the public one "a generated artifact, not a
source". The file on disk was already correct. Following the load rather
than the file would have reinstalled the graph from the retired repo and
silently undone the move, with nothing complaining. The skill's own thesis
is *verify the running thing, never the repo*; this is the same rule turned
around, and the check is to read the file when a loaded instruction contests
something you just changed.


### A merge, made under a standing authorization, can race a fix in flight elsewhere

**A background fork was given "merge PRs once they pass review" and merged
#86 at 17:57:02Z using its round-3 tip.** A fourth review round had already
posted findings against #86 by then, in the foreground session's own context
— genuine ones (a missing test for `boredom_rate`'s None-not-zero contract, a
missing denominator clause on a print line, two doc-comment welds) — and the
foreground session had already started fixing them. The fix landed and was
pushed at 18:05:36Z, eight minutes after the merge, to a branch whose PR was
by then closed. `main` never got it; a follow-up PR (#93) had to cherry-pick
the same commit back in.

The fork was not wrong to check GitHub's state before merging — it did, and
at the moment it checked, nothing it could read said a fourth round existed
yet. The gap was that "has this PR passed review" was answered against
GitHub alone, when the more current answer — "review found more, and it's
being fixed right now" — existed only in a sibling conversation's working
context, with no shared channel either side was checking. **A standing merge
authorization is a green light for the git state, not a substitute for
asking whether anyone nearby has unpushed work against the same PR** — a
`SendMessage` before the merge ("about to merge #86, anyone got fixes in
flight?") would have cost one round trip and avoided the whole repair.

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

### A guard that only covers the honest case

`Reach::sentence()` had four arms and a `for_a_person()` that fell back to the
bare artifact URL when the box named no viewer page. Its test asserted the
fallback — on `Serves::Everyone`, the one arm where the bytes URL genuinely
does open. In the other three the fallback made `page` and `bare` the same
string, so the private arm read "only you can open it: <URL>. <the same URL>
serves nobody" about an origin that answers 404, which is precisely the lie
the whole change was written to remove. It shipped green and a 26-agent review
caught it.

**A test that exercises one value of an enum has tested one arm, not the
function.** The fallback was uniform, so it looked like one behaviour worth
one assertion; what varied was the *sentence around it*, and that is where the
contradiction lived. When a helper feeds several branches, the branches are
the unit under test. The replacement walks all four states and fails if any
arm outside `Everyone` uses an opening-promise phrase with no page to name.

### The bot that had never run, and the log that would not say why

Porting the Claude workflows to a second repo turned into finding that neither
worked in the first. Three separate things, each hidden by a different
mechanism:

- The review job had failed on **every run it had ever made** — 31 of its last
  40, the rest skipped — and read as background noise because a red X on a
  bot check is easy to stop seeing. The cause was the credential, on both
  sides of an auth migration that was itself an attempt to fix it.
- `claude.yml`'s job condition used `if: |`. The literal block keeps every
  newline and adds a trailing one, and the job never matched; the workflow
  beside it used `if: >-` and ran. The fix has a second trap inside it —
  a folded scalar only folds lines at *equal* indentation and keeps a deeper
  one literal, so re-wrapping the condition prettily reintroduced the
  newlines. Caught by asserting the parsed string contained none.
- `claude-code-action` refuses to run when the workflow file differs from the
  default branch's copy, and **exits 0**. So a workflow change can never be
  tested by its own pull request, and the green check on such a PR means
  nothing at all.

Two lessons, both general. **A check that has never once passed is not a
flaky check, it is an unimplemented one** — and the way to tell them apart is
to ask for its success rate rather than looking at the latest run. And **a
diagnostic that requires making output public is the wrong diagnostic**: the
same failure was isolated by reproducing the auth path locally, which cost
nothing and exposed nothing.

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

### The web surface

**A fallback chain written with `??` and a template literal cannot fall
through, and rendered `undefined — undefined — undefined` on 695 review
cards.** ``payload.statement ?? `${subject} — ${predicate} — ${object}` ``
reads as two tiers and is one: a template literal is always a string, so the
right-hand side is never nullish and nothing after it can be reached. It
looked correct because the shape it was written for — subject/predicate/object
— is most of the queue; a commitment payload carries `{who, what, when,
direction}` and matched none of it. **A `??` chain only guards its left
operands; anything interpolated is already an answer.** The general form to
distrust is a fallback whose last tier *constructs* a value rather than naming
one.

The cost is not cosmetic and that is the part worth carrying: the card was
asking a person to accept a belief with nothing on it they could read, which
is the outbox's own rule — a field the reviewer cannot see is a field they
decided unread — arriving in a different store. Two Rust readers had the
right chain (`statement` → `what` → a named absence) with a test on the
commitment case; the page was the third and re-derived it. **A rule with a
test in one language is not a rule the other language's reader knows**, and
the fix is one function on the page matched to the Rust rather than improved
on: a card here that said more than the TUI's would be the same drift in a
better costume.

**A surface printed a remedy it could not perform.** The graph answers an
unbindable subject with *name a target with `--to`*, `mecha review bind --to`
existed, and `BindBody.to` had been on the web handler since it was written —
but neither the page nor the TUI could send one, so the instruction was
displayed to somebody holding no way to follow it. It survived because the
capability was present at every layer *except* the one with the keyboard, so
any reading of the server or the CLI says the feature is there. **Check a
remedy from the surface that prints it, not from the layer that implements
it.**

**Twenty-four mail cards rendered as 30px slivers on the first real phone
tap — chips visible, every summary gone.** The list's cards carry
`overflow: hidden`, and a flex item with overflow other than `visible` has
an **automatic minimum size of zero** — so inside the flex-column scroll
container, the whole list shrank to fit the viewport instead of scrolling,
each card clipping its own text. The fix is one line (`flex-shrink: 0` on
the scroll container's children; scrolling is the container's job), but the
shape is the lesson: the Outbox page carried the identical bug from birth
and never showed it, because it never held enough pending drafts to create
shrink pressure. A layout bug that needs *scale* to fire will pass every
short-list test and demo, and a sibling page copied from it inherits the
fuse. Headless Chromium reproduced it exactly (390px viewport, 24 rows), so
the check is cheap now that it is known.

**Every mail verb 502d from the phone while working perfectly from the
TUI.** The serve unit runs with `WorkingDirectory=%h`, and the `mecha mail`
children it spawns inherit that cwd — which the workspace jail *correctly*
refuses, because a workspace containing `~/.mecha` is the exact hole
`ensure_outside_mecha_home` exists to close. The TUI never saw it because a
TUI runs from wherever the person is standing, which is usually a project
directory. The general shape: a child process inherits the service's
working directory, and a refusal designed for interactive misuse will fire
from a unit file's default with a message aimed at a person who is not
there. A spawner that owns a workspace should set the child's cwd
deliberately (`serve/mail.rs::mail_child_cwd` — the web producer dir;
*inside* the mecha home is the default and fine, containing it is what is
refused).

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
