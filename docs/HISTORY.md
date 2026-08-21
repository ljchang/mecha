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

## Traps already hit

Recorded so they are not hit twice. Each says what broke; the sentence that
matters is the general shape.

### Measuring

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

### Learning

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

### Environment

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
