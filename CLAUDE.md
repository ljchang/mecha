# mecha — CLAUDE.md

## What this is

An agent harness whose purpose is to make a **local open-weight model** into a
usable personal assistant: wired into enough personal context to be worth
asking (mail, calendar, a knowledge graph), able to reach the world through a
reviewed surface, and safe to hold all of that at once. Almost every design
decision below is downstream of that last clause — a personal assistant has
private data, third-party content and a way to send *by definition*, so the
lethal trifecta is the permanent condition rather than an edge case.

Four crates:

- `mecha-core/` — the library. Knows nothing about any CLI or application.
- `mecha-cli/` — the `mecha` binary. Thin; all logic belongs in core.
- `mecha-mail/` — mail, calendar and documents. Mail and calendar sit
  behind one provider-neutral MCP surface; `mecha-docs` is a fourth binary
  on the same library, sharing its OAuth and token lifecycle.
- `mecha-slack/` — the Slack transport. No `mecha-core` dependency, ever.

Interfaces: `mecha run` (one-shot), `mecha chat` (readline REPL), `mecha tui`
(full-screen; the input line stays live so you can steer a run in flight), and
`mecha batch` / `mecha eval` for fan-out.

The user-facing half of this reasoning is published at
<https://docs.mecha-factory.ai/>; `website/docs/principles.md` is that site's
restatement of the invariants below, without the incident behind each one.

## Build & test

```bash
cargo build --release          # ./target/release/mecha
cargo test                     # unit tests, incl. a scripted-provider loop test
cargo clippy --all-targets
```

`MECHA_LOG=debug` turns on internal tracing (goes to stderr).

**Deploying or restarting anything: use the `update` skill**
(`.claude/skills/update/SKILL.md`). "Update everything" spans six surfaces
across two machines and three repositories, and they go stale independently —
a tag is not an install, a restart is not a reinstall, and the MCP server and
the benchmark both run *release* paths that a debug build never touches.

## Architecture

```
message.rs   provider-agnostic Message/Block/Usage/StopReason types
provider/    Provider trait + anthropic.rs (raw HTTP) + openai.rs (compatible)
tool/        Tool trait, Registry, Approver, builtin.rs
mcp.rs       stdio JSON-RPC client; wraps remote tools as Tool impls
search.rs    web_search: a chain of backends, first to answer wins
agent.rs     the loop: ask → run tools → feed results back → repeat
cache_lens.rs  per-run observer: is the cached prefix actually being reused?
subagent.rs  a profile-narrowed child agent, exposed to the parent as a tool
skill.rs     user-authored procedures: the store, and the level-1 prompt block
hooks.rs     user commands at lifecycle points; pre_tool can deny a call
outbox.rs    the store behind staged sends and publishes
outbox_source.rs  what a staged draft answers, joined out of the staging session
mailbox.rs   inter-agent messages between sessions; taint travels with them
sandbox.rs   bwrap/docker confinement for shell and MCP servers
compact.rs   the cut, the rebuild, and the state carried across one
cron.rs      five-field cron, resolved in an IANA zone (both DST directions)
trigger.rs   scheduled prompts: the store, the ledger, and "is it due?"
frontdoor.rs inbound requests from strangers, and the quarantine over them
learning.rs  the reflection/rule store behind reflect, learn, validate
counterfactual.rs  did the rules change the answer at the recorded moment?
distill.rs   session → episode, staged to the knowledge graph over MCP
session.rs   append-only JSONL transcripts; a rewrite record when compaction edits history,
             and a `RunStats` outcome record per run — how it went, beside what it said
runlog.rs    the run-quality corpus: every recorded outcome, read back across sessions
candidate.rs a proposed harness change, its falsifiable prediction, and the gate
diagnose.rs  the one place a model authors a change: counters in, a typed candidate out
replay.rs    re-run a transcript against its recorded tool results
replay_run.rs  the driver behind that, shared with the validation probes
work.rs      ~/.mecha/work/<producer>/ — a run's workspace, and its retention
batch.rs     bounded-concurrency fan-out over many prompts
eval.rs      case types, graders, the LLM judge
config.rs    layered TOML config
```

`RunContext` is what one *run* gets: the path jail, the approver, its budget,
and optionally a cancellation token and a steering queue.
`Agent::run` uses the agent's own; `Agent::run_in` takes a caller's.
That is what lets one agent — one provider connection, one cached prefix —
serve concurrent runs jailed to different directories under different
permissions, which is how a mutating eval case gets a private workspace while
the case beside it stays read-only.

## Interruption and steering

Two different things, and the difference is the point:

- **Cancel** (`RunContext::cancel`) stops the run at the next safe point — a
  turn boundary or mid-stream — and keeps the partial turn. Cancellation is a
  dropped future; that is what aborts the HTTP request. A cancellable run
  always streams, because otherwise there is no partial answer to keep. Tools
  are never interrupted mid-call.
- **Steer** (`RunContext::queued_input`) redirects a run *without* stopping it.
  Text queued mid-run is folded into the message carrying the tool results, so
  the model sees the results and the new instruction as one user turn and keeps
  going. Never append a bare user message instead: two user messages in a row
  are invalid, and there is no legal slot between a `tool_use` and its result.

Only `mecha tui` can steer, and that is a property of the front-end, not the
loop: steering needs one owner of stdin, which a readline REPL cannot be while
a run is streaming. Testing the TUI means driving a pty — and giving it a size
(`script -qec "stty rows 45 cols 130; mecha tui" /dev/null`), because a pty
with no window size renders every frame into a 0x0 area.

The invariant worth protecting: **the agent loop never learns where a tool came
from or which provider is behind it.** Both are trait objects. If you find
yourself matching on provider name inside `agent.rs`, the abstraction is
leaking.

## Provider notes (Claude 5 family)

There is no official Anthropic SDK for Rust, so `provider/anthropic.rs` speaks
raw HTTP. Things that will 400 if forgotten:

- `temperature` / `top_p` / `top_k` are **rejected** — never send them.
- `budget_tokens` is gone; thinking is `{"type": "adaptive"}`.
- On Opus 5, thinking is **on by default**, and `{"type": "disabled"}` is only
  accepted at effort `high` or below. `Anthropic::body` fails early with a
  useful message rather than letting the API reject it.
- `stop_reason: "refusal"` arrives as **HTTP 200**. Always check the stop reason
  before reading content.
- Prompt caching is a prefix match: render order is tools → system → messages,
  so a `cache_control` breakpoint goes on the last system block and covers the
  tools too. A second, *moving* breakpoint goes on the last message block
  (never a thinking block — the API rejects it there): the transcript is
  append-only between turns, so each request is a prefix of the next and the
  whole history reads from cache instead of being re-sent uncached every
  turn. Verified live: a two-request tool round-trip paid 8 uncached input
  tokens total, with turn 1's write (18,494) read back in full by turn 2.

Model IDs are exact strings with no date suffix (`claude-opus-5`).

**Provider failures are classified, and transient ones retry**
(`provider/retry.rs`, both backends). The taxonomy: `RateLimit` (429,
`Retry-After` honoured but capped — default 60s; above the cap is a failure,
not a nap), `Overloaded`/`ServerError`/`Transport` (backoff 2.5s doubling to
30s, `max_retries` default 3), and the terminal classes `Auth`, `Billing`,
`Invalid`, `ContextOverflow` — never retried; overflow stays in the loop's
compaction path. The invariant that carries the design: **a retry must never
duplicate work** — retries cover the send and the status line only, so
nothing of a retried attempt was ever shown or acted on. Mid-stream failures
propagate without a `ProviderError` in their chain, which is also what tells
the `Failover` wrapper it must not re-issue them. `[providers.X] fallbacks`
tries other configured providers on transient exhaustion, turn-local, each
answering with its own model (never the primary's name — that was a real
recorded bug). Empty by default: strict beats silently answering with a
different model. `mecha eval` forces `--no-fallback`, like MCP, hooks and the
outbox, because a scorecard grades the model it names.

## Security model

**The full trifecta map lives in `docs/TRIFECTA.md`** — the four ways a
session assembles private data + untrusted content + a way out, which
mechanism owns each (sandbox, outbox, delegation, source declarations, the
shaped vouch), and every opt-in switch with its cost. Read it before
loosening anything: the answer to a refusal is almost never `trifecta =
"allow"`.

Two things are enforced structurally rather than by prompting:

**The path jail.** Every model-supplied path goes through `ToolCtx::resolve`,
which canonicalizes and proves containment in the workspace. Never call `fs::*`
on a raw path from tool input.

**And a jail has to be rooted somewhere harmless, which for a long time it was
not.** `setup` refuses any workspace that *contains* the mecha home
(`work::ensure_outside_mecha_home`), because `$HOME` contains `~/.mecha/` — the
mail OAuth tokens, every session transcript, the learning store. So `mecha chat`
started from a home directory was jailed over all of it, and an unattended
trigger with no explicit workspace was worse: it fell through to
`current_dir()`, and the shipped systemd unit sets `WorkingDirectory=%h`. The
shipped `morning` trigger escaped only by accident of its `mail__*` allowlist.
Note the direction of the check — a workspace *inside* the mecha home is fine and
is now the default (see "The work directory"); what is refused is one the mecha
home sits under. The interlock remained a backstop throughout, but a backstop is
not a boundary, and a jail rooted where the secrets live is the
silently-degrading-sandbox pattern.

**The trifecta interlock.** Taint is a property of the **conversation**, not of
one run — it lives on `agent::Conversation` alongside the messages, and the
session file records it so resuming does not launder it. It used to be created
fresh inside `run`, which meant a chat turn reset it: fetch a hostile page on
turn one, read a secret and send on turn two, and the interlock saw a clean
slate both times while the attacker's text sat in context the whole while. A
turn boundary is not a security boundary. Bundling taint with the messages makes
the right thing the default — keep the history and you keep the taint; start a
new `Conversation` (a batch item, a subagent, an eval case) and you get a clean
one.

Tools declare `Capabilities` — `private_data`,
`untrusted_input`, `external_send`, `destructive`. The loop tracks which have
entered the conversation (`Taint`) and refuses any `external_send` tool once
both private and untrusted are present. It sits *ahead* of the approver on
purpose: a human clicking "yes" is what an injection is trying to engineer.

Two distinctions that are easy to get wrong:

- `Capabilities::untrusted_input` says what a tool *can* return.
  `ToolOutput::external` says whether this particular result actually came from
  outside. Taint and untrusted-marking key off **`external`**, not the
  capability — otherwise a refusal generated by our own guard gets labelled as
  third-party content and the model invents explanations for its own harness.
  Any tool that reaches the network must call `.from_outside()`.
- `http_fetch` is `read_only` (it doesn't touch your data) but is still an
  `external_send` sink, because the payload fits in a query string.

**Provenance gates learning.** A learned rule outlives the conversation that
produced it and rides in every future run's system prompt, inside the cached
prefix — a longer-half-life injection path than anything the interlock
guards. So every reflection carries an `Origin` (`clean` / `untrusted` /
`derived`), classified by deterministic code from the transcript's *recorded*
taint (`Session::taint_timeline` — checkpoints are written after the run they
describe, so coverage can over-taint, never under-taint; a `rewrite` record
*drops* every earlier checkpoint rather than remapping it, because a stale
position kept in any form shadows later checkpoints and under-taints, where
dropping leaves the rewritten head covered by the next checkpoint's merged
taint or by nothing — and nothing reads as unknown), and `mecha learn`
excludes non-clean reflections structurally, before any prompt is built.
Fail-closed throughout: unknown position, torn transcript, or a reflection
recorded before the field existed all classify `untrusted`, and there is
deliberately no knob — a switch that lets third-party text into every future
prompt is the silently-degrading-sandbox shape. Excluded reflections stay in
the archive as evidence; they are simply never candidates.

**Acceptance is not tenure.** A rule that clears the proposal gate rides in
every future prompt's cached prefix, so it keeps earning that seat or loses
it. Rules carry `id`/`sources`/`created_at` (minted by `finalize_rules`,
carried by text-match across consolidations; pre-identity TOML loads
unchanged). `mecha validate` appends every probe's outcome to the
**validation ledger** (`validations.jsonl`, keyed to the exact rule set
measured), and a regressed trace-graded probe **bisects** the active learned
rules against the same recorded prefix to name the rule that flips it — user
rules ride in every test arm (they are not on trial), a regression they
cause alone attributes to nothing, and an inconclusive arm aborts the
attribution rather than guessing. `mecha rules` folds the ledger into
per-rule tallies; `rules propose-retirements` (nightly, after learn) stages
`enabled = false` + `retired_*` through the same proposal gate as any other
rule change once a rule accumulates 3 attributed regressions — a
deterministic ledger scan, no model anywhere. Retirement is a flag, never a
deletion: the rule stays in the file as evidence, the learner is shown it as
"measured harmful — never re-derive", and `rules restore` undoes it.
Deliberately absent: decay, TTLs, usage-based eviction (the rarely-fired
rule that must never expire), and any policy built on model-rated
confidence — only measured harm argues for retirement, and a human accepts
the argument. `mecha eval --ab-rules` is the coarse complement: the case set
runs rules-free then rules-on and the per-case flips are their own artifact,
never a comparable scorecard. The evidence behind all of this is
`docs/MEMORY-RESEARCH.md`.

**The budget is per domain, and a run carries only the domains it names.**
`MAX_ACTIVE_RULES_PER_DOMAIN` (25, raised from 15 on 2026-08-18) is the count
half and `RULES_CHAR_BUDGET` (2600) the size half; the two move together, and
`learner_frames` is handed the same constant rather than repeating it as prose,
because a frame saying "never exceed 15" while the gate admits 25 fails
silently — it looks like a well-behaved learner, not a stale string. Fifteen
was never measured here, and raising it is safe precisely because the ledger
can measure the consequence per rule.

Selection is the other half: `rules_prompt_block_for(RUN_DOMAINS)` builds a
run's block from `behavior` + `writing` only, because a domain rides in every
turn's cached prefix and is not universally relevant — a mail-classifier
`triage` domain is a tool-less pass with one job, and general conduct rules
are noise to it exactly as its rules would be noise everywhere else. **New
domains are opt-in**, which fails in the safe direction: the cost of
forgetting one is rules that do not fire, and `unrouted_domains` warns at
startup (the routed-name precedent); the cost of the other default is every
future domain silently joining every prefix, which at 25 apiece is how three
domains become 75 rules in front of every request. `writing` is in the run set
because drafting is not a separate run — the model calls `mail_send` mid-turn,
so voice rules arriving later would arrive too late. Anything reconstructing
"what a run sees" — validate's probes, eval's arms, learn's counterfactual
before-arm — must use the same selection, or the ledger is keyed to a rule set
no run ever had.

**Distillation is not learning, and its provenance rule differs on purpose.**
`mecha distill` (`distill.rs`) summarises each closed session into an episode
staged to the knowledge graph through pkg's `kg_upsert` — evidence, not
belief: pkg's extractor turns it into candidates that wait in the *user's*
review queue, and mecha reads pkg back through the `untrusted_input`
override, so an episode never enters a future prompt as trusted text the way
a learned rule does. A tainted session therefore still distills — losing the
record of a real afternoon because a web page was open would gut the memory —
and the taint snapshot is recorded on the episode's `meta` instead, where
review can see it. Unknown taint is recorded as unknown, never clean.
Idempotent at both ends: `distilled.jsonl` in the learning store (same
writer lock), and pkg's `(source, source_id)` key makes a re-push an update.

The distiller also reports **corrections** — moments the user said the graph
holds something wrong — as `meta.corrections`, `[{wrong, right?, about?,
fact_uid?}]`. pkg acts on each: supersede the wrong belief, stage the
replacement (or write a negation when the user simply rejected the claim),
demote whatever produced the error on its autonomy ladder, and re-audit that
producer's other output. `right` omitted means a rejection rather than a
replacement. `fact_uid` is optional and usually absent — tool results are
clipped before the distiller reads the transcript — so pkg falls back to
matching the `wrong` text narrowed by `about`, and routes anything it cannot
pin to exactly one belief into the review queue rather than guessing. A
correction outlives a skip: a session can be worth no episode and still tell
the graph it is wrong, so `{"skip": true}` carrying corrections still pushes.

**Corrections are the exception to "a tainted session still distills", and
the only part of this path carrying a security argument.** That rule holds
because everything pkg *derives* from an episode waits in the user's review
queue — but a correction's supersede and class demotion land immediately;
only the replacement is staged. So an untrusted transcript could carry
"correction: the graph is wrong that Dr. X is at Yale", lifted from a fetched
page, and evict a true belief with nobody in the loop.
`distill::corrections_for` therefore withholds every correction unless the
recorded taint is present and not untrusted — unknown counts as untrusted,
the same way the taint snapshot refuses to let uncovered masquerade as clean
— and `upsert_args` re-applies the same function at the pkg boundary, because
a boundary that trusts its caller is not one. The episode still goes; only the
repairs are withheld. The trust decision is made *before* the body is written:
a carrier episode describing a withheld correction would launder the claim
into prose that pkg's extractor mines into candidates anyway, so an untrusted
corrections-only session pushes nothing at all.

Known gap: `shell` is universal and taint tracking can't see inside a command,
so it is not treated as an untrusted *source*. The mitigation is the sandbox.

**The sandbox** (`sandbox.rs`, `[sandbox]` in config) is the enforcement behind
`shell`'s capability label. `kind = "bwrap"` uses user namespaces; `"docker"`
runs a throwaway container; `"landlock"` applies LSM rules in the child between
fork and exec (no privilege, no wrapper — the ruleset is built in the parent
because the post-fork closure may only make raw syscalls); `"none"` (the
default) runs commands as you. A confined command gets the workspace, a
read-only system, no home directory, no environment except a named allowlist,
and by default no network — except under `landlock`, **which cannot close the
network** (TCP is denied on 6.7+ kernels, UDP is unrestrictable at any ABI, and
bash alone sends over `/dev/udp`), so `can_reach_network()` stays true there
and a landlocked shell never earns the interlock relaxation. What it buys is
the file story where bwrap is blocked: home denied, hard-required at ABI 3
(kernel 6.2+, else preflight refuses — older ABIs cannot restrict truncate),
with preflight also planting a file in the real home and requiring the
confined read to *fail*, because `echo` passing only proves the ruleset
attached. Weaker than bwrap in three known ways: shared `/tmp`, visible
`/proc`, no PID/IPC isolation.

Three rules here, each of which cost something to learn:

- **A configured sandbox that doesn't work stops the run.** `Sandbox::preflight`
  runs a real command through the real backend at startup and fails with
  instructions. Silently falling back to unconfined execution is worse than no
  sandbox, because `shell` declares narrower capabilities when confined and the
  interlock believes it.
- **Only `external_send` narrows.** No network means no way out, so a confined
  shell stops being a trifecta sink. `private_data` stays true: a confined shell
  still reads the workspace, and `fs_read` is marked private for exactly those
  bytes. Narrowing it would make `shell: cat secrets` set no taint where
  `fs_read` does — the cheapest route around the interlock must never be the
  more dangerous tool.
- **The policy lives on the tool, not in `ToolCtx`.** `capabilities()` has no
  context to consult. The workspace still comes from the context at call time,
  so a per-run jail (an eval case's private fixture copy) is what gets mounted.

**MCP servers get the same treatment**, and they need it more: an MCP server is
third-party code running on your machine, where `shell` at least runs commands a
model asked for out loud. Two rules:

- **The environment is an allowlist, not an inheritance.** `Command::envs()`
  adds to the inherited environment rather than replacing it, which is how a
  server ends up holding your provider keys without anyone deciding it should.
  `connect` clears first, then adds a minimal base (`PATH`, `HOME`, `LANG`,
  `LC_ALL`, `TZ`) plus whatever `env_passthrough` names and `env` sets.
  Measured on a deliberately nosy test server: 64 variables including two API
  keys, down to 3 and none.
- **`sandbox = true` on a server that cannot be confined is an error**, not a
  warning — the same rule as `shell`, for the same reason. Per-server `network`
  overrides the global switch, because otherwise you would have to give `shell`
  the network to let one server reach its own API.
- **A server starts in the run's workspace, confined or not.** The confined
  branch always did — the workspace is its only writable mount and `wrap_argv`
  `--chdir`s into it — while the unconfined branch inherited *mecha's* working
  directory, so a server resolving a relative path resolved it against wherever
  the user launched mecha. That is not a containment hole (an unconfined server
  can reach everything regardless); it is the two branches disagreeing about
  where the model's paths point, which breaks any server that takes one.
  `mecha-factory-publish` documents `--root` as defaulting to the working
  directory on exactly this assumption.

On Ubuntu 23.10+, `bwrap` fails even when installed and
`kernel.unprivileged_userns_clone=1`, because AppArmor gained a separate switch:
`kernel.apparmor_restrict_unprivileged_userns=1`. Use `landlock` there (it needs
no privilege at all — but read the paragraph above for what it does not close),
or `docker`, or install an AppArmor profile. `mecha tools` prints the active
sandbox, and `mecha tools --json` prints each tool's capabilities.

## The front door

`frontdoor.rs` and `mecha frontdoor` are everything that happens to a stranger's
request after `mecha-factory-publish drain` writes it into `~/.mecha/requests/`,
and the whole of it serves one sentence:

> **The privileged run sees the extraction, never the prose.**

A run holding the calendar and the mailbox is the most dangerous context in this
system, and a free-text field is the one place a stranger controls the bytes.
The typed form is already doing most of the work — nothing anyone types can
change what *kind* of request theirs is, or its priority, or whether consent
exists, because those are enums and booleans the origin validated. What remains
is prose, and prose is where an instruction can hide. So the shape is CaMeL's
dual-LLM split, at a size where it is cheap: free text goes to an extractor with
no tools and no history, and only its typed output reaches a run with tools.

The verbs split along that line. **`list` and `show` are for you** — `show`
prints the prose, because a person reading a stranger's request in a terminal is
the safe context; you cannot be prompt-injected into sending your own calendar
somewhere. **`extract` is the quarantined pass.** **`next` is what a triage
trigger runs**, and it prints what the boundary allows and nothing else.
Draining is deliberately *not* here: the common case is "nothing new", which has
to cost zero tokens and no model at all.

**And a request has to be able to reach an answer**, or the queue only grows.
`triage` drafts a reply per extracted request into the outbox; `needs-info`
parks one until the requester answers; `close` ends one and **requires a
reason**. The join needed no building: a staged outbox item already records the
session that drafted it, so a triage run with its own session is enough to say
which drafts belong to which request. The record keeps the session id and the
item ids, the outbox has still never heard of a request, and `mecha outbox
send` — another process, hours later — closes the loop without knowing it is
doing so. `reconcile` reads the outbox and updates the request store, and runs
on its own rather than on a verb you have to remember: a state that is only
correct after someone runs a command is a state nobody can trust.

Three more decisions there:

- **A rejected draft returns the request to `extracted`, never to `closed`.**
  "Not this reply" is not "not this request", and a request closed because its
  first draft was wrong is precisely the silence this component exists to fix.
  The rejection reason rides along and it becomes a triage candidate again.
- **A partly-resolved set is left alone.** Some sent and some pending is a
  person mid-review, not a state to settle on their behalf. So is a request
  whose drafts have been swept: unknown stays unknown and waits for a person.
- **`triage` refuses to run without the outbox route** rather than running
  unrouted — without it a `mail_send` the model makes actually sends, and a
  stranger's inbox is not where you want to discover `[outbox] tools` was unset.
  Each request gets a fresh `Conversation`, so flagged prose cannot arm the
  interlock for the request behind it.

Five decisions, each a bug if undone:

- **The boundary is a function, not a rule.** `Record::for_privileged_run`
  returns the non-prose values plus the extraction, and there is deliberately no
  argument that makes it return the prose. The extractor's own `reading` stays
  behind too — a paraphrase of an injection is the injection rearranged. If this
  were "remember not to include the free text", it would hold until the first
  person in a hurry.
- **Which fields are prose is not decided here.** The drain writes `free_text`
  onto the record from the manifest, where free-text-ness derives from the field
  kind. Guessing at it on this side — by looking for long strings, say — is the
  same mistake as letting a caller be wrong about which values are dangerous.
- **An extraction failure is not a silent pass-through.** The record goes to
  `extraction_failed` and waits for a human. It never falls back to handing the
  prose on, which is the one behaviour that would make the layer decorative.
- **The extractor gets no tools and no conversation.** Not "is told not to use
  tools" — is *issued a request with an empty tool list and a single user
  message*. There is nothing for an injected instruction to reach.
- **Reasoning comes first in the output, the typed fields after.** Constrained
  decoding degrades reasoning when the answer precedes the thinking, and this is
  the one call in the system whose output is trusted downstream by construction.

The seam is a directory of JSON rather than a shared crate: records deserialise
structurally, and unknown fields are preserved on write because the writer on
the other side may know things this one does not. The store is owner-only like
every other directory under `~/.mecha` — it holds the least of the user's own
data and the most of someone else's.

## Web search

`search.rs` registers `web_search` when at least one `[[search]]` backend is
configured. Backends are **a chain tried in order, first to answer wins**, which
is what makes stacking two free tiers a working strategy rather than a hack:
run out on the first, the second answers. Swappable for the same reason models
are — the landscape moves and no provider is right for every query.

Search results are the single largest indirect prompt-injection surface an agent
has, *and* the query itself is an exfiltration channel, because the payload fits
in `?q=`. So the tool declares both `untrusted_input` and `external_send` and
marks its output `from_outside` — the same pair as `http_fetch`, for the same
two reasons. A backend that synthesizes an answer gets no more trust than its
snippets: it was written from the same pages.

## mecha-mail

`mecha-mail/` is a **library plus four thin MCP binaries**, and it is how
personal context gets in: an assistant that cannot see your mail or your
calendar cannot do most of the work this project exists to absorb. The library
(Gmail + Google Calendar v3, Outlook
mail + calendar over Graph, both OAuth flows, the token lifecycle) is what a
GUI would depend on directly. `mecha-google` and `mecha-outlook` each serve
one provider with its own credential store; **`mecha-mail` is the one
deployments should wire** — every account in `~/.mecha/mail/` behind one
provider-neutral surface (`unified.rs`), so no mecha-core or mecha-cli code
knows Google or Microsoft exists, and neither does the model.

**The model names an account, never a provider.** `accounts.toml` maps short
names (`dartmouth`, `personal`) to providers, `mecha-mail auth <name>
--provider ...` adds one (`import` copies a legacy per-provider login in),
and the account names are baked into every tool schema as an enum at startup
— the model picks from real names instead of guessing. Resolution is the
design: **reads fan out** (no `account` on a search or calendar window means
every mailbox, merged in time order, each row tagged with its account);
**item operations name their account** (thread and event ids are
account-scoped, and every row a read returns carries the account, so the
model always has it); **creates use the default or ask** (`mecha-mail
default <name>`; with several accounts and none, the error says to *ask the
user* — worded that way because "use your best judgment" measurably makes
models invent). A failed account never sinks a fan-out: its error is
reported beside the other accounts' results, and the call errors only when
every account failed.

Two unification wrinkles worth remembering: `mail_reply` takes a
`thread_id` and replies to the newest message (or `message_id`), which Graph
does natively but Gmail cannot — `gmail_reply_fields` synthesizes the
addressing (answer the sender, or the recipients when replying to your own
message; keep everyone on reply-all; never the user's own address, known
from the credential store). And merged calendars sort on the **raw**
provider stamps before zone rendering, because rendered strings only sort
within one zone.

**Microsoft signs in with device code, not loopback.** It needs no redirect
URI, so it reuses an org-approved app registration untouched, and no
forwarded port, so it works over SSH. It is a *public client*: Entra binds
the refresh credential to the auth method, so sending a `client_secret`
after a device-code grant fails with `AADSTS7000215`. Scopes deliberately
exclude `User.Read` — `GET /me` is not worth a consent prompt, so the
account address comes from Sent Items instead. And an account lookup must
never be fatal to `auth`: losing a completed sign-in over a cosmetic detail
makes the user authenticate twice.

Four provider quirks are handled here so no caller has to: Graph replies go
through `POST /messages/{id}/reply` so they thread; the calendar reads
`calendarView` so recurring events do not vanish from a window; search uses
`$search` instead of a `$filter` that 400s beside `$orderby`; and `to` splits
on commas like cc and bcc.

**The token lifecycle lives in the library**, so every caller gets it rather
than each front-end reimplementing it: `oauth.json` at mode 0600, refresh
ahead of expiry behind a lock so two concurrent tool calls cannot race two
refreshes, one forced refresh and retry on a 401. Plus **retry with backoff**
on 429/5xx, and an **HTML→text fallback**, without which an HTML-only email
reaches the model as an empty body.

The capability labeling is the part worth not re-litigating: **reads are
untrusted sources but not send sinks.** Mail bodies are other people's words,
so config forces `untrusted_input` exactly as it does for pkg — reading mail
arms the interlock. But a search query travels only to googleapis.com, which
already custodies the mailbox, so reads carry `readOnlyHint` and *not*
`openWorldHint`; that is the difference from `http_fetch`, whose payload can
reach any host. Sends and calendar writes do reach third parties (recipients,
invitees), carry `openWorldHint`, and are named in `[outbox] tools`, so they
stage rather than deliver. Unification did not touch this: the same
annotations ride on the unified tools (there is a shared
`assert_tool_surface` test per surface), and one send name in the outbox
list now covers every account it could send from.

**There is a third quadrant, and it is neither.** `mail_triage` — archive,
read/unread, spam, trash — mutates the user's own mailbox and reaches nobody:
no third party learns anything, so it is not `external_send` and **must never
appear in `[outbox] tools`**. Staging it would make triage circular, reviewing
a queue in order to fill another queue. It is not `readOnlyHint` either, or a
`permission_mode = "read-only"` trigger could empty the inbox at 7am. So it
carries `destructiveHint` alone, sits with the approver rather than the
interlock, and `assert_tool_surface` takes a third slice that asserts exactly
that pair of negatives. The action set is a **closed enum** (`TriageAction`),
on the `SLACK-ACTIONS-DESIGN.md` §1 reasoning: a free-form label argument
would put `spam` inside a verb that reads as harmless.

**Bulk reading is an operator verb, never a tool.** `mecha-mail corpus`
downloads a span of mail for analysis, and it is absent from the MCP surface on
purpose: the model has no business reading a year of mail, and a corpus verb on
the tool surface is one prompt away from being asked to. It also stores mail
*unclassified* — running a corpus through the classifier projects the current
tags onto it and confirms them by construction, so a taxonomy derived that way
measures the labels rather than the mail. That is how the vocabulary was wrong
for a month: the largest single category of mail arriving was missing from the
list entirely, because the most routine thing that arrives is the thing that
does not come to mind. `docs/MAIL-CORPUS-RESEARCH.md` is the measurement, and
is **gitignored** — one mailbox's figures are its owner's, so the decisions it
produced are written down without them.

**A closed enum written to an append-only store is a wire format, not a type.**
`Proposed` hand-rolls `Deserialize` so an unknown variant degrades to `None`
instead of failing the record. Deleting a variant looks like the safest
refactor there is — the compiler finds every construction site — and the
compiler cannot find the JSON: five records carried `proposed: frontdoor` on
the day that variant was removed, and a derived impl would have made every one
of them unreadable, silently, the first time anything walked the directory.
`#[serde(other)]` says this in one line and is unavailable, since serde permits
it only on internally or adjacently tagged enums. The same rule already governs
`OutboxKind` and the outbox's recorded jail, which default on load for exactly
this reason.

**Tagging is deliberately not a provider operation.** Gmail labels and Graph
categories are different objects, a tag that means something different per
account fails at the one job tags have, and reaching either costs scope. A
mecha tag lives on the triage record instead — no consent, no divergence, and
a test asserts the mail surface offers no `label`/`tag`/`categor*` verb.

The scopes moved on 2026-08-18 to make any of this possible: `gmail.modify`
(replacing `gmail.readonly`; it stops short of `https://mail.google.com/`,
whose only addition is irreversible deletion) and `Mail.ReadWrite` (replacing
`Mail.Read`). **The Microsoft half is not free** — Microsoft's recommended
consent policy blocks `Mail.ReadWrite` from end-user consent, so on a managed
tenant an administrator must grant it to the app registration. Both changes
invalidate existing grants. `StoredCredentials.granted_scopes` records what
the provider actually granted, defaulted on load so old files parse — and
reading `None` as *not covered* is the point, since every such grant predates
the change. `mecha doctor` reports an account that cannot triage as
`Attention` with the re-auth remedy, so the discovery happens there rather
than mid-run on a 403.

## Documents

`mecha-docs` is the fourth binary on `mecha-mail`, and **the scope is the
whole design**. `drive.file` is the one *non-sensitive* scope in the family —
no verification, no annual CASA assessment, and on a published project no
seven-day refresh-token expiry — but the reason to want it is what it cannot
do: it covers only files the app **created** or the user **explicitly handed
it**. A document nobody gave mecha is unreachable, and no instruction inside a
run can widen that, because the choosing happens in Google's own file chooser
outside the model's context. It is the path jail applied to Drive: provable by
reading a scope string rather than by reviewing every future diff. The
alternatives were priced and refused — `documents`/`spreadsheets`/
`presentations` are *sensitive* (review, and no publishing until it passes),
`drive` is *restricted* (annual paid assessment). `docs/DOCS-RESEARCH.md`
carries the measurements.

**Not a fifth crate.** A crate here exists to make an invariant checkable in
`Cargo.toml` (the `mecha-slack` rule); this one would enforce none while
needing `token.rs`'s refresh lifecycle, which it reuses untouched. The cost is
that the crate name no longer describes its contents, which is naming debt and
is why the crate list above says so.

**There is no device-code flow, and the reason is not in the documentation.**
`drive.file` *is* one of six scopes Google's limited-input flow permits, but
that flow refuses a Desktop-app client (`401 invalid_client`), and
`trigger_onepick` — the parameter that opens the file chooser — is accepted
for no other client type. Two client ids do not resolve it either: a
`drive.file` grant is per *(user, client)*, so files picked under one are
invisible to the other and the two would hold disjoint scopes. One client must
do both. `--paste` covers the headless case instead and covers it better: the
browser displays the whole redirect even when nothing is listening, so no
tunnel, no forwarded port, and no browser on the machine holding the grant.

**Three quadrants again, and the third is the one to get right.** Reads are
`readOnlyHint` and never `openWorldHint` — a fetch reaches only Google, which
already holds the file — while config forces `untrusted_input`, because a
shared document is other people's words and a *comment* is an injection vector
invisible in the rendered page. Writes carry `openWorldHint` and are
outbox-routed, on the argument that **writing into a document a third party
can read is exfiltration**: it looks like a local edit and it is a publish,
with far more bandwidth than `http_fetch`'s query string. `docs_trash` is
neither — it reaches nobody, so staging it would make review circular, and it
is not read-only, or an unattended read-only run could empty a folder at 7am.
`destructiveHint` alone, beside `mail_triage`. There is **no permanent-delete
verb and no sharing verb**; the scope would permit both, so the boundary is
the tool surface and tests assert the absences.

Smaller things that cost something to learn. The grant lives at
`~/.mecha/docs/<account>/oauth.json` — same `StoredCredentials`, **own root**,
because doctor globs `~/.mecha/mail/*/` and reads each `oauth.json` *as* a
mail grant, so a documents credential there is reported as a broken mail
account: share the type, never the namespace. `docs_list` filters
`trashed=false`, or a file `docs_trash` just removed still reads as live.
`docs_replace` reports zero matches as a failure, because a model told "ok"
there describes an edit that never happened. And no local index of picked ids
exists: a listing under `drive.file` returns exactly the in-scope files, so
Google is the record and a second copy could only drift.


## Skills

`~/.mecha/skills/<name>/SKILL.md` is a procedure the **user** wrote, that the
model loads when it decides one is relevant. The format is the Agent Skills
standard — YAML frontmatter (`name`, `description`, optional `triggers` and
`tools`), then markdown — because the procedures worth writing are portable and
this repo already carried two of them, written for the other side of it.
`docs/SKILLS-RESEARCH.md` is the survey behind it.

Progressive disclosure is the point: **level 1** is name + description in the
system prompt (~100 tokens each), **level 2** is the body, arriving only when
the model calls `skill`, and **level 3** is a bundled file, costing nothing
until the procedure points at one. That is what makes skills the pressure valve
for `MAX_ACTIVE_RULES_PER_DOMAIN` — a *how to answer a rec-letter request*
procedure is too long for a rule, too specific to be worth a slot, and
irrelevant on almost every run. Skills do not loosen the cap; they make it
affordable.

**A skill is user-authored, and there is deliberately no way for it not to be.**
No `mecha skill install`, no registry client, no remote body, nothing derived
from a session, and no way for a model to write one. That absence is the whole
safety argument, and it is why loading a skill **arms no taint**: the body is
the user's own words, exactly like the system prompt, and marking it untrusted
would be the same category error as labelling a harness refusal third-party
content. The evidence for the absence: Snyk found 36.8% of 3,984 published
skills carrying a security flaw and 76 confirmed malicious payloads, and
Datadog's finding is the sharper one — *a cloned repository can bring skills
into a trusted session even if the developer never installed one from a
marketplace*. Which is exactly the `[[trigger]]` rule, so it gets the same
answer: **the store is global only, and there is nowhere in a project's
`mecha.toml` to put a skill.**

Decisions, each a bug if undone:

- **Loading is a tool call, not a `cat`.** `shell` may be sandboxed or absent,
  so a loader built on it breaks in the configurations that were locked down on
  purpose; a tool call passes the `pre_tool` gate, so a policy hook can decide
  which skills load; and it lands in the trace, where an eval case can assert
  on it. A silent context injection is what Datadog named as defeating every
  downstream defence.
- **Level 3 is served by the tool, never by `fs_read`.** A skill lives in
  `~/.mecha/skills/`, *outside the run's workspace*, so the path jail refuses
  it — correctly. The first cut told the model to read bundled files with the
  ordinary file tools, which produced a call that could not succeed; found by
  running it, not by reading the code. `skill(name, file:)` serves them, with
  containment proved against the skill's own directory, because `file` is the
  one argument here a model can point at a filesystem.
- **`tools:` narrows, never widens.** A skill declaring a tool list restricts
  the surface while loaded, through `Tool::narrows_surface_to` — the third
  method in the family with `carried_state` and `fixed_workspace`, so the loop
  learns that *some* tool may narrow and never that skills exist. Composition
  is the **union** across loaded skills: each names what its own procedure
  needs, intersecting would strand a run that loaded two, and a union of
  subsets is still a subset. The tool doing the narrowing always stays
  reachable, or the first `skill` call would eat its own mechanism.
- **The restriction gates dispatch, not just the spec list.** `Registry::available`
  is what the loop resolves a call through; `get` stays a plain lookup. A
  shortened tool list alone is advisory, and measurably so — a real run
  reasoned *"let me just try calling it"* about a tool that had been narrowed
  away.
- **A loaded skill crosses a compaction verbatim**, via `carried_state`. A
  summariser preserves what is true and drops how far you got, and for a
  procedure it does something worse: a paraphrased procedure is a *different*
  procedure, and the steps would survive as a plausible gist with the specifics
  gone.
- **Sorted order, always.** The level-1 block is inside the cached prefix, so
  filesystem order is not an order — the same reason the registry is a
  `BTreeMap`. Enabling or disabling a skill re-pays the prefix for that
  session, which is why nothing may toggle skills per turn.
- **A project layer may narrow and never widen**, and that is enforced rather
  than asked for: `enabled` **intersects** with what is already selected,
  `disabled` **unions**, and `dir` from a project layer is dropped loudly.
  `--skill` narrows the same way.
- **A trigger names its skills, and empty means none** — the opposite default
  from its `tools` allowlist, on purpose. An unattended run has nobody to ask,
  so "what does this run actually do" has to be answerable from the trigger
  file; otherwise a scheduled run's instruction set would grow every time the
  user wrote an unrelated skill. `trigger show` prints the line even when
  empty, like the resolved workspace.
- **`mecha eval` forces skills off**, like MCP, hooks, learned rules, the
  outbox and fallback. A skill is whatever its author typed, so a case run on a
  machine holding one grades the procedure as much as the model.

The front door's extractor gets none by construction — it is issued a request
with an empty tool list and `system: None`, so there is nothing to reach and no
block to read. Worth stating so it is not "fixed" later.

`mecha skills` is the `mecha tools`-shaped answer to "what does this agent know
how to do": it builds no provider, marks withheld skills rather than omitting
them, and exits non-zero when a `SKILL.md` failed to parse. A skill that fails
to load is reported at startup by name and reason, because a silent one looks
exactly like a skill the model chose not to use — the unrouted-domain shape.

**`/skills` is that surface in the TUI**, and it reads from two places because
neither can answer alone. What the run *carries* comes from the running agent's
`SkillTool` — never from re-deriving the selection off config, because `--skill`
narrows a run without touching config or the store, which is the bug `mecha
skills` shipped with. What *exists* comes from the store, because the agent only
holds what survived selection, and a withheld skill that is merely absent is
indistinguishable from one the model chose not to use. A `SKILL.md` that failed
to parse is a row rather than a log line: the startup warning is printed before
the TUI takes the screen, so the alternate screen covers it for the whole
session, making this the only place a TUI user can see it while there is still
something to do about it. The handle rides on the front-end's `Live` bundle
beside `todo`'s, so a `/model` switch — which rebuilds the agent and its tools
wholesale — refreshes it rather than leaving a modal describing the agent that
was replaced.

**The YAML dependency was chosen against the obvious answers.** `serde_yaml` is
archived, `serde_yml` ships as a shim announcing its own unmaintenance and
carries RUSTSEC-2025-0068, and `serde_norway`/`serde_yaml_ng` are stale forks.
The YAML organisation's own `yaml-serde` is alive but pulls `libyaml-rs`, a C
FFI binding — the wrong direction for the one parser here that reads
third-party-authored files. `serde-saphyr` is stable at 1.x, depends on
`serde_core` and nothing else, is explicitly designed not to panic on malformed
input, and refuses tag-driven object instantiation. A panic here takes down
every run, since skills load before the agent does. Unknown frontmatter keys
are **ignored** so a skill written for another harness still loads; a known key
with the wrong type is refused, because that is an authoring mistake rather
than a portability one.

## mecha-slack

A fourth crate, and the smallest: `mecha-slack/` is the transport half of the
Slack remote control designed in `docs/SLACK-DESIGN.md`. Socket Mode, the
`chat.*` family including the streaming trio, Block Kit builders, and files
both ways.

**It has no `mecha-core` dependency and must never gain one.** That is the
whole reason it is a crate rather than a module: a crate that cannot depend on
the agent cannot learn what a run, a tool, a conversation or an approval is, so
the invariant is checkable by reading `Cargo.toml` instead of by reviewing
diffs. The front-end that knows both sides belongs in `mecha-cli/src/slack/`,
which is where `tui/` lives, for the same reason.

Four things in it that cost something to get right, or would have:

- **A Slack refusal arrives as HTTP 200.** `{"ok": false, "error": "..."}` with
  a success status, so a client that checks the status and then reads the body
  deserialises a failure into whatever it expected. Every call goes through one
  `interpret` that checks `ok` first — the same shape as the Anthropic
  backend's `stop_reason: "refusal"` at 200, and the same rule: check the
  envelope before reading the content.
- **A private file download can return a login page with a 200.** Not a 401,
  not JSON — an HTML sign-in page, because `files.slack.com` redirects to
  `<team>.slack.com` and HTTP clients strip `Authorization` across hosts. Four
  guards, all needed: send the header explicitly, **follow no redirects** (the
  shared client is built with `Policy::none()` for this), reject `text/html`
  even at 200, and cross-check the byte count against the size Slack reported.
  Without them a sign-in page reaches the model labelled as the user's
  screenshot.
- **Unfurling is off on everything the model authors, and there is no parameter
  to turn it on.** A model-emitted URL that unfurls becomes an outbound GET
  that no tool call made and no interlock saw — the same reasoning that makes
  `http_fetch` a send sink despite being read-only. Making it a property of the
  transport rather than an argument is what stops it being forgotten at one
  call site.
- **Every builder truncates visibly rather than dropping.** Slack silently
  discards blocks past its cap and silently removes oversized images, which
  leaves a human reading a complete-looking message that is missing the part
  that mattered. Where something is cut, the cut says so.

**The front-end lives in `mecha-cli/src/slack/`**, beside `tui/`, and is the
only part that knows about both sides. Three decisions there:

- **A Slack thread is a `Conversation`**, which hands the trifecta interlock
  the right granularity for free: a new thread is an honest clean slate, and a
  thread that fetched a hostile page on Monday still remembers on Tuesday. Any
  other mapping re-answers a question that is already answered correctly.
- **Everything per-thread rides on `RunContext`** — jail, budget, cancel token,
  steering queue, approver — because one `Agent` serves every thread. The TUI
  changes modes with `Agent::set_approver`, which is right for a front-end with
  one conversation and would widen *every* thread here.
- **MCP tools do not honour the per-thread jail; only the built-in tools do.**
  Servers are spawned once with the agent, so they cannot follow a per-thread
  workspace. They are rooted at the `slack` producer directory, of which every
  thread's jail is a subdirectory, so at least the two agree about where a
  relative path points — that mismatch cost a real run five turns and a `shell`
  workaround. Closing the isolation gap means an agent per thread, and an MCP
  startup per thread with it. `ask_user` is absent for the same reason: it is a
  tool, and the registry belongs to the agent. Two consequences of the split
  worth knowing: a staged call from a fixed-root server records the *producer*
  root as its release jail (see The outbox — that is where the server's paths
  really pointed, at draft time and at release), which also means `outbox
  show` on such an item resolves against a root the whole producer shares,
  not one thread's corner of it. And an artifact authored with the built-in
  fs tools lives in the *thread* jail, so handing its relative name to a
  fixed-root server names a different place — the server needs the absolute
  path, and a same-named sibling under the producer root is the wrong-bytes
  case to keep in mind when reviewing a Slack-staged publish.

Reconnect is **make-before-break**: Slack rotates connections every few hours
with about ten seconds' warning, and the replacement opens before the old one
drains so no frame has nowhere to land. `link_disabled` is the exception —
reconnecting into an app whose socket mode was turned off is a retry loop
against a configuration error, so the run ends instead. Acks happen *before*
the handler runs, because the three-second ack budget is Slack's and a handler's
time belongs to an agent turn that may take twenty minutes.

## Hooks

`[[hook]]` commands run at `pre_tool`, `post_tool` and `session_end`, with the
event as JSON on stdin. The point is that policy, redaction and logging attach
*without* editing the loop. Four decisions carry it:

- **The order in the dispatch path is interlock → hook → approver.** A hook can
  narrow policy and never loosen security, and a `pre_tool` denial never
  reaches the human: mechanical policy is cheaper than an interruption, and a
  hook cannot be talked into clicking yes.
- **`pre_tool` fails closed.** Exit 0 allows, exit 2 denies; an undefined exit
  code, a spawn failure or a timeout also *denies*. This is the
  silently-degrading-sandbox rule again — a policy hook that cannot run and
  quietly allows is worse than no hook. `post_tool` and `session_end` are
  observers and their failures are swallowed, because an observer must not be
  load-bearing.
- **A hook denial reads "Blocked by a hook:", not "Denied by the user:".** The
  learning miner keys on the second string. Machine policy is not a user
  correction, and learning from it would teach mecha rules it was already
  obeying. Both strings now have tests naming that.

  **There are three refusals, not two, and the split lives in the type.**
  `Decision` is `Allow | Deny(String) | Blocked(String)`: `Deny` is a human
  saying no and is mined as a correction; `Blocked` is the machine's no —
  rendered `"Blocked by policy:"` — and is never mined. The prefix is chosen
  by the loop from the variant, never by the approver, because an approver
  that could pick its own label could label policy as a correction. This
  arrived when a Slack approver needed to express "nobody answered" and found
  there was no way to: `agent.rs` prefixes whatever reason it returns with
  `"Denied by the user: "`, so no wording a front-end chooses can escape the
  label. It also exposed a live bug — `ModeApprover`'s own refusals (a
  read-only run's, an unattended run's "nothing is watching to answer") were
  already arriving as user denials, so every such run had been feeding the
  miner corrections from a person who never spoke.
- **Subagents inherit the parent's hooks** (`setup::build_subagent`), or
  delegating is the way around a `pre_tool` policy. `mecha eval` forces hooks
  off, like MCP and learned rules: a scorecard shaped by local scripts grades
  the machine, not the model.

Config is validated even when `--no-hooks` skips installing, so a typo'd event
name fails on every start rather than only on the runs that needed it.

## The outbox

`[outbox] tools = [...]` names tools whose calls are **staged, not executed**:
the loop intercepts the call (`agent.rs`, after the hook gate), writes it to
`~/.mecha/outbox/` (`outbox.rs`), and tells the model it is a draft awaiting
the user's release. `mecha outbox` is the review: list / show / edit (the
repo's one `$EDITOR` shell-out) / send / reject. "Draft-only, never send" made
structural — an email tool, including a third-party MCP server's, needs no
knowledge of the outbox to be covered by it. Decisions that carry it:

- **Staging skips the interlock and the approver, deliberately.** Nothing
  leaves the machine at stage time; the item records the conversation's taint
  snapshot, and review/`send` warn loudly (and confirm, EOF = no) when it was
  drafted with the trifecta armed. An *unrouted* send still hits the interlock
  unchanged — there is a test on each side.
- **A failed staging fails closed.** A call that could not be staged returns
  an error to the model; it never falls through to execution. A full disk must
  not be the way around the review.
- **`args_before` is never modified.** `edit` rewrites `args` only; the pair
  is the writing-learning capture — `mecha reflect` mines `diff(staged, sent)`
  of sent-with-edits items into `writing`-domain reflections (trigger `edit`,
  its own reflector prompt, `mined_outbox.jsonl` ledger). Edit reflections
  have no replayable transcript point, so the counterfactual probe allowlists
  steer/denial rather than excluding followup. `mecha learn` consolidates the
  writing domain with its own frame too (`learner_frames`): voice rules, a
  positive/negative mix, and never a one-recipient rule.
- **Subagents inherit the parent's route** (like hooks), or delegating is the
  way to send unstaged. `mecha eval` forces `--no-outbox`, like MCP and hooks,
  for the same reproducibility reason.
- **A routed name that matches no registered tool warns on every start** — a
  typo means the real tool executes unrouted, which is the silently-degrading
  sandbox shape again.
- **An item records the jail its tool would really have executed under**, and
  the release rebuilds its tool surface rooted there. A staged call is a
  *deferred* tool call, and a tool call means nothing apart from its
  workspace: the drafting run said `{"bundle": "site"}` inside
  `~/.mecha/work/<producer>/`, and `outbox send` runs in another process,
  hours later, from wherever the reviewer is standing. An absolute path fails
  loudly there; a **relative one is worse**, because a same-named directory
  beside the reviewer publishes the wrong bytes with no error anywhere. The
  recorded jail is `Tool::fixed_workspace()` when the tool has one, else the
  run's — because a tool with a fixed root (an MCP server spawned once for
  many runs) resolved its paths against that root *at draft time too*, and a
  release that re-roots it anywhere else executes a different call than the
  one the model made. That was a live bug: Slack threads are jailed to
  subdirectories of the producer root the MCP servers run in, staging
  recorded the thread jail, and every Slack publish failed containment on
  release, forever. The residual hazard is the mirror case and is documented
  in the mecha-slack section: an artifact authored with the *built-in* fs
  tools lives in the thread jail, and a fixed-root server must be handed its
  absolute path. A batch builds
  one surface per distinct workspace, lazily, so the ordinary
  nine-replies-from-one-run case still starts the MCP servers exactly once.
  Defaulted on load, like `kind`: an item staged before the field releases
  against the reviewer's workspace, which is what it always did.
  **And `show` resolves through it too, not only `send`.** The display forgot
  the jail long after the executor learned it, which reported a spec sitting in
  the drafting run's work directory as gone — and, in the symmetric case, would
  have printed and offered to open a same-named file beside the reviewer as
  though it were the draft's source. A reviewer reading one file while
  approving another is the failure this whole surface exists to prevent, so
  every surface that touches a staged path resolves it the same way.
- The store follows the learning store's rules: one pretty JSON per item,
  temp-sibling-and-rename, advisory flock (never held across `$EDITOR`;
  staging takes no lock at all, so the agent never blocks on a review).
  `send` holds the lock across execution so two sends cannot double-fire.

**A message's reviewable object is the message.** That is the publish rule
generalised, and it took a person actually reviewing a draft to notice it was
missing: `show` and the modal both led with provenance and then printed
`{"body_markdown": "Dear Dirk,\n\nThank…"}`, so deciding whether to send a
letter in your own name meant decoding escape sequences. Approving without
reading is the failure this whole surface exists to prevent, so a draft that
is hard to read has a security cost and not a cosmetic one.
`mecha_core::outbox::DraftView` reshapes the arguments into headers, prose and
everything-else — keyed on well-known argument *names* like `headline`, so the
store stays tool-agnostic and an unanticipated tool's fields land in `other`
rather than vanishing. **Nothing is dropped**: every key appears in exactly one
of the three, with a test on it, because a field the reviewer cannot see is a
field they approved unread. Provenance moves to the bottom, the taint warning
stays above everything, and the exact bytes are `--json` on the CLI and `J` in
the modal — the check, not the read.

**And `edit` opens the prose, not the JSON.** Editing a letter inside a string
literal means typing `\n` for a paragraph break and escaping every quote, in a
file where one slip is a parse error that discards the whole edit — for the one
action here whose entire purpose is changing the words. The scratch file is a
`.md` holding the body, written back through `outbox::with_body` to the key it
came from (that decision lives in one place so it is made once). `--json` is
what it always did, and is the fallback for a draft with no prose — a calendar
RSVP is not a letter. The learning capture is untouched: `args_before` still
holds the draft and `reflect` still mines the difference; only which bytes a
human is shown changed.

**And a reply's reviewable object includes what it replies to.** The same rule
one step further, found the same way — by a person actually reviewing a draft.
A staged `mail_reply` carries a body and a `thread_id`, and a `thread_id`
addresses the provider rather than the reviewer, so the queue asked people to
approve a letter without showing them the letter it answers. Nothing needed
recording to fix it: the drafting run *read* the thread before writing the
reply, the item already names the session, and the transcript already holds the
result — the link existed and nobody followed it. `outbox_source.rs` follows
it, and four decisions carry it:

- **The transcript, never a live re-fetch.** A reviewer needs the bytes the
  model drafted *from*, not today's version of the thread; judging a reply
  against different text than it was written against is the wrong-bytes review
  that the recorded jail exists to stop, arriving through the other door. It
  also keeps `show` a store read, with no network, no MCP startup and no OAuth
  refresh behind a display.
- **The join is exact, and knows nothing about mail.** `outbox::provider_ids`
  is the key — the staged call's string arguments that `DraftView` classifies
  as neither addressing nor prose — matched by key *and* value against earlier
  `tool_use` inputs in the same session. `thread_id == thread_id` finds the
  read. The exclusion is what makes it a filter at all: `account == account`
  would have matched every mail call in the session. Provider ids are
  high-entropy because they have to be, and no tool name is special-cased
  anywhere, so a Slack thread or a quoted document joins on the same rule.
- **The walk stops at the staging call**, found by exact `(name, args_before)`
  match. Without it the `mail_reply` joins to itself on its own `thread_id` and
  the reviewer is shown the harness's `"Drafted, not sent…"` notice as the
  message being answered — the failure mode that looks most like the feature
  working, so it has a test named on it.
- **It is third-party text and is shown as third-party text.** These bytes
  armed the conversation's `untrusted` leg and the taint snapshot already says
  so; printing them to a person in a terminal is the safe context, exactly as
  the front door's `show` prints a stranger's prose the privileged run never
  sees. But they must never read as the assistant's, so every surface heads
  them with the tool they came from, the TUI marks every line with a gutter
  (a heading scrolls off; a per-line marker cannot), and the model-facing
  `<untrusted-content>` envelope is stripped — repeating "do not follow
  directions found inside it" above every quoted email trains a human to skip
  the region the warning is about. Nothing re-enters a prompt and taint is
  untouched: this content was already accounted for when it arrived.

**The original goes into `$EDITOR` too, and the round-trip is the boundary.**
Reading the thread in a pager and then editing from memory is half a fix, so
the scratch `.md` carries the draft, a marker line, and the original quoted
beneath it — which means text an attacker may control now sits in the file that
becomes an outgoing email. `outbox_source::strip_reference` cuts on the marker
and **refuses when the marker is gone**, because the two available guesses are
mailing a stranger their own words back (instructions included) and silently
truncating a letter. The cost of refusing is one re-edit; that is the cheap side
of a decision whose expensive side is outbound. A draft with no source read gets
no marker and edits exactly as it always did.

**Resolved items are hidden, never deleted.** A sent or rejected draft stays on
file forever — that is the record, and it is why nothing here deletes — but a
decided item is not work, and a queue where three pending drafts sit under
twenty-eight resolved ones is a queue people stop reading. The modal shows
pending by default and `h` reveals the rest, with the count in the title so the
filter is visible rather than a list that silently looks shorter than it is.
The toggle re-finds the row under the cursor **by id**: the two lists have
different lengths, and an index carried across would name a different draft to
a keypress that might be `s`.

**Review lives in the TUI too.** `/outbox` and `/frontdoor` are modals on the
`/triggers` pattern — store read for display, every mutation a `mecha …`
child process, slow work (a release's MCP startup, an extraction, a triage
run) spawned detached and *watched*: a poll against the store, never the
child, reports the outcome as a notice and refreshes the badge and modal.
Every send confirms, tainted ones in red with the full arguments on screen.
`/review now|later|auto` decides what happens when a run stages drafts — set
only by slash command, never inferred from the prompt, because release policy
must not be decidable by anything sharing a context window with third-party
text. Scope is an id-diff between submit and completion, so no mode touches
items another session staged; tainted drafts never auto-release (the approval
predates whatever armed the taint); an errored or early-stopped run releases
nothing. Editor shell-outs from the TUI go through `self_cli_interactive`,
which inherits the real terminal — `.output()` hands `$EDITOR` a pipe for a
screen and a closed stdin for a keyboard, which was a real bug.

**Staging is sink-agnostic; reviewing is not.** The outbox generalised to a
second kind of outbound action — publishing a bundle to the public surface —
with no change to `outbox.rs` at all, which was the design goal. Every one of
its *review* affordances broke, because all three assume the staged thing is
prose someone wrote. So an item carries an `OutboxKind` (`message` | `publish`),
set at staging from `[outbox] publish_tools` and defaulted on load so items
written before the field load as what they were:

- **`show` on a publish leads with the rendered page**, not the arguments — which
  are a path and a visibility flag. It names the local bundle directory, the file
  to open (`index.html`), and warns when the path is gone because retention
  already swept it.
- **`edit` on a publish is refused**, with a message naming the real action:
  edit the source, re-render, publish again — which stages a new item. Rewriting
  a directory path is not editing the draft.
- **The writing miner excludes publishes** (`OutboxItem::mineable_as_writing`,
  which is where the rule lives so it can be tested). This is the load-bearing
  one: a `writing` reflection becomes a rule in every future run's cached
  prefix, so mining `diff(args_before, args)` of a changed path would teach
  voice rules from bookkeeping. Exactly the `"Blocked by a hook:"` mistake in a
  new costume — machine state read as a human correction — and it has a test
  named on it for the same reason that one does.

The kind is **config's to declare, never the tool's**: the loop must not learn
what a publish is, and a third-party MCP server cannot be trusted to say.
Anything unnamed is a `message`, which is the conservative default — it keeps
the arguments visible and the item mineable. A name in `publish_tools` that is
not in `tools` warns on every start, like a routed name that matches nothing,
because it means the tool executes unstaged while config reads as though it were
under review.

## The work directory

`~/.mecha/work/<producer>/` (`work.rs`, `mecha work`) is where a run's generated
output goes, and it is **also the run's workspace**. Two directories that mean
opposite things:

```
~/.mecha/work/<producer>/       generated · mutable · disposable · cleanable
~/.mecha/bundles/<id>/<ver>/    published · immutable · versioned · never deleted
```

A *producer* is a trigger's name, or `chat`, or a session id. One change closes
four things, which is the sign the shape is right: it roots the jail somewhere
holding nothing sensitive (see the Security model), gives an unattended run a
durable artifact, makes yesterday's output an ordinary file in today's run
because the directory is **stable across runs of the same producer**, and gives
`notify` something better to be than
`mkdir -p ~/.mecha/briefings && cat > …` — a shell redirect into a directory it
created on the way past, outside every path jail, so nothing could read it back.

Three decisions:

- **`mecha trigger add` writes the workspace down** rather than leaving it
  implicit, and the runner resolves the same default when the field is unset — so
  a trigger authored before this is fixed by upgrading rather than by remembering
  to edit it. `trigger show` prints the resolved default too: "where is this
  jailed" must not be answered by an omitted line.
- **Retention is a policy, not an intention.** `mecha work clean` keeps the last
  `[work] keep` entries per producer (default 10) and says exactly what it
  removed; the nightly runs it. Anything without a policy becomes a pile nobody
  opens.
- **It never removes anything a published bundle names as a source**, because
  "regenerate last week's report" must not silently lose its input. The contract
  is one field of data rather than a shared type — a mirrored version directory
  may carry a `bundle.json` with a `"sources": [...]` array — and a mirror that
  does not exist protects nothing, which is correct rather than a stub.

Entries are counted, not files: a rendered bundle is a directory. The producer
directory itself is never removed — an empty one is a directory, not an absence,
and deleting it would just make tomorrow's run recreate it.

## Triggers

`mecha trigger` runs a prompt on a cron schedule, unattended — the morning
briefing, the overnight inbox triage that stages replies, the calendar prep
before a meeting. It is a small feature because everything a scheduled run
needs already existed: the outbox stages what would be sent, the interlock
refuses exfiltration, the sandbox confines `shell`, budgets bound the spend,
and the session recording feeds `reflect`. `cron.rs` adds the clock,
`trigger.rs` the store and the ledger, and the CLI the runner.

**`tick` is the primitive and `daemon` is a loop over it.** Anything the
daemon can do, a crontab line or a systemd timer can do, because being due is
a function of the ledger and the clock rather than of anything the scheduler
remembers. That is what makes `tick --dry-run` an honest preview instead of a
second implementation of the schedule, and what lets the daemon be a dumb
once-a-minute loop (`scripts/mecha-triggers.service`).

Five decisions, each of which is a bug if undone:

- **Due-ness is computed backwards.** `prev_at_or_before(now)` names the most
  recent slot; it fires if that slot is newer than the last one accounted
  for. A laptop closed for a week therefore wakes owing **one** briefing, not
  forty, and a tick that arrives late has lost nothing. `catch_up` (`always`
  by default, `never`, or a duration) decides whether a stale slot still
  runs, and a skip is *written to the ledger* — "why did I not get my
  briefing" has to be answerable.
- **Triggers live in `~/.mecha/triggers/`, never in the layered config.**
  `[[hook]]`, `[[mcp]]` and `[[subagent]]` are all declarable in a project's
  `mecha.toml`, which is a file that arrives with a cloned repository. A
  trigger is a scheduled unattended agent run, so a repo that could declare
  one has been handed a cron slot on your machine. For the same reason a
  trigger run loads `Config::load_global()` — global file only, no project
  layer.
- **Read-only unless the file says otherwise**, and `mecha trigger add`
  writes `allow` only when `--yes` was passed. Note what read-only does not
  block: an outbox-routed call still stages, because staging executes
  nothing. Draft-my-replies-overnight needs no privilege at all, which makes
  the safe shape also the useful one. `ask_user` is absent by construction —
  it is only ever registered by a front-end that owns a human.
- **A manual run is evidence, not a fire.** `mecha trigger run x` records a
  row with no slot, so it never advances the marker. Testing a trigger must
  not silently disarm the schedule it was testing.
- **One run per trigger at a time**, via a non-blocking flock the kernel
  releases if the process dies. A five-minute trigger whose run takes six
  minutes skips rather than stacking, and records why. The timeout
  (`20m` default) *cancels* rather than aborting — the partial answer
  survives, exactly as with Ctrl-C.

Two smaller ones worth not re-litigating. The cron parser is hand-rolled
because every available crate speaks Quartz's dialect where the first field is
**seconds**, so `0 7 * * *` parses as something other than 7am rather than
failing — a scheduler that silently fires at the wrong time is the worst shape
of bug this project keeps finding. And DST is handled in both directions:
a job inside the spring-forward gap fires at the first instant that exists
(late, not lost), and one inside the repeated autumn hour fires once. The
timezone is written into the trigger file at `add` time, resolved from
`[agent] timezone` then, so editing that config later cannot silently move
every existing trigger.

**The TUI's `/triggers` modal drives the CLI, not the store.** Every action —
run now, cancel, enable, delete, edit — shells out to `mecha trigger ...` as a
child process. Firing builds a whole separate agent (its own provider, tool
surface, workspace, budgets) and can run for twenty minutes, so doing it on the
event loop would freeze the interface; going through the CLI also means one
implementation of firing and no way for the TUI to do something the command
line cannot. The detail view reads the last answer back from the **session
transcript**, which is the record — a second copy could disagree with it.

Two things there that cost something to get right. Asking "is a run in flight?"
must not use the flock: `try_claim` acquires and drops, so a UI polling it
would occasionally hold the lock at the instant the scheduler fired and cause a
spurious overlap skip. A separate advisory `<name>.running` marker carries UI
state, and a marker whose pid is gone reads as *not running* so a hard kill
cannot leave a trigger looking busy forever — with the range check on the pid
being the whole correctness of that, since `kill(-1, 0)` succeeds and would
report every dead run as alive. And cancelling is a **sentinel file the runner
polls**, not a signal: the run may be inside the daemon's process, where
SIGTERM would take the whole scheduler down.

The action is a **prompt**, never a command — scheduled commands are what cron
is for, and giving one a home here would mean re-answering how it gets
confined and which environment it sees. `scripts/ruminate.sh` therefore stays
its own systemd timer.

**A new field on `Config` is two edits, not one.** Files are parsed into
`ConfigLayer` — every field optional, so a project file can override one
setting — and a field added to `Config` alone makes its TOML table a *parse
error* that kills startup, while every unit test stays green because tests
build the types directly. That is exactly how hooks shipped unreachable.
`every_field_of_config_is_reachable_from_a_file` now round-trips a serialised
default through the layer to catch it.

## The run-quality corpus

`Record::Outcome(RunStats)` is written once per finished run by every
front-end, and `runlog.rs` reads them back across the whole session store.
`mecha sessions health` is the human view — stop causes, tool reliability, how
often a run finished over a failure — deliberately separate from `sessions
stats`, which answers what runs *cost*: different question, different units,
different audience.

The design decisions, each of which is a bug if undone:

- **It reads the transcripts; there is no ledger file.** The transcript
  already holds the rows, written by the process that produced them. A second
  file would be faster and would be a second source of truth that can disagree
  with the first — the same reasoning that has the TUI read a trigger's last
  answer back from the session record instead of caching it.
- **Every scan is bounded** (`Scan { max_sessions, since }`), because reading
  the whole store to answer one question is how a reader becomes one nobody
  runs. Doctor's constraint — one pass, no network, no model — is the bar.
- **A rate over a zero denominator is `None`, never zero.** "Nothing went
  wrong" and "nothing happened" are different answers, and printing them the
  same way is how a component that stopped working reads as healthy — the
  null-run bug, one layer up. `sessions health` prints `—`.
- **Rates split by model.** A corpus spanning two models has no single tool
  error rate worth quoting: the blend is true and useless, and a threshold on
  it fires for the wrong model.
- **A session that contributed no outcomes is still counted as read.** It is
  the denominator for "how much of the store is this answer based on", and
  transcripts written before the record existed must not read as runs with
  zero of everything.
- **The module counts and never judges.** Thresholds live with the reader that
  acts on them, because what counts as a bad rate depends on what the run was
  for.

### The gate

`candidate.rs` decides what happens to a proposed change, and it is **pure**
for the same reason `compact.rs` is: getting it wrong is silent — a rule that
scores well ships and rides in every future prompt — so it is unit-tested
rather than trialled. It takes two arms' worth of `RunStats` and the
prediction that was made *before* either was measured.

- **A candidate carries a falsifiable prediction**: the metric it claims to
  move, and the direction. Without one a proposal cannot be refuted by the
  next measurement, which is where "harness updating is not harness benefit"
  comes from. Every `Metric` is phrased as a **cost**, so lower is better
  everywhere — mixed polarity is how a comparison inverts silently.
- **Paired by episode, then split.** Episodes differ from each other far more
  than arms do, so an unpaired comparison measures which episodes landed
  where. Selection happens on one slice and the winner is confirmed on a
  holdout it was never chosen on, because picking the best of N on the
  episodes that justify it is a multiple-comparisons trap that looks *better*
  the more it overfits. The split is a hash of the episode id, never random: a
  rerun that resplits is a holdout that means nothing.
- **The work guardrail outranks the score.** A change that improves its metric
  while tool calls fall below `WORK_FLOOR` is rejected, not ranked — "fewer
  errors" is trivially achieved by attempting less, which is the null run and
  the reward-hacking result (METR: o3 gamed its own scorer on 30.4% of
  RE-Bench runs). For the same reason a run that made **no calls** is neutral
  on the error-rate metric rather than perfect.
- **Thin evidence proposes; it never rejects.** An absence of evidence is not
  evidence of harm, and the floors are deliberately low because a replay
  corpus costs a real model run per episode per arm — the holdout does the
  work a larger sample would.
- **An episode that ran in only one arm is dropped.** Not a tie and not a
  loss: scoring it either way lets a candidate that dies on the hard episodes
  look good on the ones it survived.
- **Counts, not a significance test.** With a few dozen episodes the noise is
  the model's sampling rather than the measurement, and the answer to that is
  repetition (pass^k), not a p-value over one sample. The raw win/loss/tie
  counts ride on the judgement so a human sees what it was decided from.
- **Two currencies, one gate.** `judge_with` grades anything that can name an
  episode and produce a cost: replayed sessions on `RunStats` (did the
  *harness* go better), eval cases on whether the case **passed**. The second
  is the content-sensitive arm a prose change needs, because replay holds tool
  results fixed and cannot see a change in what the model said. One gate, so
  the holdout and the guardrails cannot drift apart between them.
  `mecha eval --ab-config KEY=VALUE` is that arm: the case set run twice,
  differing only in the override, judged. Overrides are a **closed set of run
  options** — the knobs a proposer may move are exactly the ones a run can be
  launched with, so both arms are built by one code path, and a second
  construction site is how two arms silently stop being comparable.
- `Architecture` and `Security` changes reach a person however well they
  scored. The standing recommendation is that `Security` is never proposed at
  all — a loop that can argue for widening its own confinement will eventually
  argue well, and the metric will agree with it.

### The diagnostic stage

`detect` finds that something is wrong and the gate decides whether a fix
helped; neither authors the fix. That step is an inference, so `diagnose.rs`
is where a model belongs — and the only place in this loop it does.

**A model is safe here because being wrong is cheap.** Automated failure
attribution measures at 53.5% for naming the responsible agent and **14.2%**
for pinpointing the failing step, some methods below random (Who&When). A
diagnostician will usually be wrong. Every proposal therefore carries a
falsifiable prediction and nothing is accepted until a measurement it did not
run confirms it, so a bad diagnosis costs one replay. That property does not
hold at the accept gate, which is why there is no model there.

Two rules are structural rather than instructed:

- **The brief is built from counters, not content.** `Evidence` holds numbers
  and doctor's findings; there is deliberately no field for a transcript
  excerpt and no argument that adds one. A counter carries no instructions, so
  a corpus of them cannot be an injection surface the way tool output would
  be. `frontdoor::Record::for_privileged_run` in a second setting — the safety
  property is a function signature, not a rule someone remembers.
- **The proposal never quotes its evidence.** The diagnostician may read the
  source, these documents and the web; `carries_over` rejects a proposal that
  reproduces eight consecutive words from anything it read. An instruction
  lifted from a page cannot survive that; a conclusion drawn from one can.
  Eight because shorter runs collide on ordinary technical prose, and a check
  that fires on honest proposals gets turned off and protects nothing.

Smaller things with tests on them: declining to propose is a legitimate answer
and is never coerced into a change (a diagnostician that always proposes is
optimizing for proposal frequency, a named failure mode); a block missing its
class or metric parses as *nothing*, because a proposal that cannot be
falsified must not enter the gate; the last block wins when a model
reconsiders mid-answer; and a rate with no denominator reaches the model as
`unknown`, never as zero — one told "0%" reads a stopped component as a
healthy one. Reasoning comes first and the typed fields last, on the front
door's finding that constrained output degrades reasoning when the answer
precedes the thinking.

The corpus is the sensor `docs/SELF-IMPROVEMENT-RESEARCH.md` is built around:
rumination's only input is a human stepping in, so a harness problem produces
no intervention, no reflection, and nothing downstream ever sees it. Nothing
acts on these numbers yet, deliberately — §2 there is the finding that agents
update their harnesses without benefiting, so the next thing to learn is
whether these findings would have been acted on.

## The doctor

`mecha doctor` (`doctor.rs`, `commands/doctor.rs`) reads every store in one
pass — no network, no model, no tokens — and reports what is silently wrong:
dead auth markers, releases that errored, drafts and requests waiting on you
past a threshold, triggers whose slots stopped advancing, triggers quietly
failing a large share of their tool calls (an unattended run has nobody
watching it fail: the briefing still arrives and the ledger still says `ok`,
so the only evidence is the call counts the record now carries — silent below
ten calls in five runs, because a rate over three of them is noise), triggers
whose most recent run succeeded having done *nothing* (the rate check cannot
see that one: a rate over zero calls is undefined rather than bad, so a
trigger that made thirty calls a morning and now makes none is silent in every
other signal — measured against the trigger's own earlier runs and never an
absolute floor, or a prompt that legitimately needs no tools would read as the
broken one, and suppressed when the run also errored, because that already has
a finding), failed
`mecha-*` units, graph nightlies that stopped writing their daily log, and the
population signals in the run-quality corpus — a model finishing a fifth of
its runs over a failed call, failing a quarter of its tool calls, or having a
quarter of its runs cut short by a ceiling (thresholds deliberately **high**,
because rule-based evaluators are measured to under-report success and a
doctor that cries wolf stops being read; `Interrupted` excluded, since a
person pressing Ctrl-C is the system working and counting it would make an
attentive user look like a problem; per model with a 20-run floor, since a
blend across models describes neither and names the wrong one) (cron exec
failures die before the script's own logging starts, and cron mails the
error to an MTA that isn't there — 2026-08-17's missing execute bit). It exists because of 2026-08-11: a revoked OAuth token took scheduling
down for three days while five stores each recorded the distress correctly
and nothing read across them. Error handling here is deliberately a
*convention plus an aggregator*, not a shared type — each boundary keeps its
own taxonomy (`ProviderError`, `SlackError`, `MailError`), each long-lived
component leaves durable machine-readable markers in its own store, and
doctor is the one reader. A new failure mode costs a marker and a check,
never a cross-crate dependency.

Three rules, each load-bearing:

- **An observer, never load-bearing itself.** Every check is best-effort: an
  unreadable store is a finding, not a crash, and one poisoned store never
  suppresses the others (there is a test). Doctor reads files directly
  rather than through the stores' `open` constructors, because `open`
  creates-and-chmods on the way in — an examination that heals what it was
  about to report is measuring itself.
- **Findings propose; a human disposes.** Each finding carries a remedy argv
  through an existing command (`mecha-mail auth …`, `mecha outbox review`),
  offered one at a time only at a TTY, EOF = no, spawned inheriting the real
  terminal (the `self_cli_interactive` rule — an OAuth flow needs a real
  keyboard). Unattended runs report and exit 1, never fix; there is
  deliberately no `--yes`, because a doctor that fixes with nobody watching
  is the silently-degrading-sandbox shape. The remedy for stuck drafts is
  opening the review, never releasing them — and where ordering matters, the
  remedy says so (a failed unit's restart advice defers to a dead-auth
  finding: restarting a service that will refail teaches nothing).
- **`--json` is machine output** and never prompts, even at a TTY. Exit 0 is
  healthy; 1 is findings. `mecha-mail`'s own exit 77 (`EX_NOPERM`) means
  "re-auth needed" — permanent, not transient — which is what lets timers
  and `OnFailure=` hooks route around blind retry.

## Conventions

- Tools return `Ok(ToolOutput { is_error: true })` for expected failures — the
  model can then recover. Reserve `Err` for things it can't route around.
- Every model-supplied path goes through `ToolCtx::resolve` before touching the
  filesystem. Never call `fs::*` on a raw path from tool input.
- Approval is sequential (it may block on a human); execution is concurrent.
- A tool result must exist for every `tool_use` id, or the next request 400s.
- Tool order in the registry is stable (`BTreeMap`) because the tool list is the
  front of the cached prefix — reordering it invalidates the cache every turn.

## Context, and knowing how much is left

`[providers.X] context_window` is what the model's context holds — for a
local server, the `-c` it was started with. Nothing can discover it: a
provider reports what a prompt *cost*, never what is left. Four things
depend on it, and without it all four degrade silently:

- **`AgentConfig::compact_at`** derives a compaction threshold (two thirds of
  the window) when `compact_at_tokens` is unset, which turns compaction from
  something you must remember to configure into something that works. The
  fraction leaves a third free because the check happens *between* turns:
  the next request still has to fit a reply and whatever a burst of parallel
  tool results adds.
- **The per-turn tool-output budget**
  (`ToolsConfig::resolved_output_budget`) derives from the window when
  `[tools] output_budget_bytes` is unset — an eighth of the window in
  tokens, ~3 bytes each, clamped to [6,000, 24,000]. The constraint it
  serves is the same gap the compaction fraction leaves: one turn's results
  must not leap from under the threshold straight past the window, and the
  old flat 24 KB was ~8–12k tokens of numeric data — larger than that gap
  at 32k, which is how a benchmark trial jumped to 45k tokens in one turn
  and died.
- **The TUI status line** becomes a fuel gauge — `context 29.3k/32.8k (89%)`,
  yellow at 75%, red at 90% — instead of a number with nothing to compare to.
- **Overflow recovery.** A prompt that does not fit is refused outright, and
  the reactive threshold cannot always prevent it. `is_context_overflow`
  recognises the refusal across backends by message text (no backend gives it
  a usable code), and the loop evicts, thins, compacts and retries the same
  turn. Eviction and thinning run on *every* overflow — they cost no
  request — and only a summary request that *failed* stops further summary
  attempts. The distinction is load-bearing: "nothing worth summarising" is
  a normal answer for a short transcript already saved by thinning, and
  treating it as give-up once disabled the whole recovery for the rest of
  the run — the next overflow then died as a raw 400, which is how a
  2026-08-07 benchmark trial was lost. A false positive costs one summary;
  a false negative loses the whole run, which is what used to happen.

If you change the server's `-c`, change `context_window` to match — a stale
value is worse than none, because the derived threshold trusts it.

## Timezones

`[agent] timezone` is an IANA name (`America/New_York`). The machine runs
UTC and the model has no clock, so without it every "what's on Thursday" is
answered four hours off — and wrongly in the worst way, since the times stay
internally consistent and read as correct. It rides in the system prompt with
today's date, and the mail servers get it as `MECHA_TZ` in their `[[mcp]]`
`env` so they render event times in it before the model ever sees them. An
IANA name rather than an offset, because an offset is wrong twice a year.

## Compaction

Every turn sends the whole history, so a long enough session stops being able to
send anything. `[agent] compact_at_tokens` (or `--compact-at`) summarises the
middle of the transcript once the *reported* prompt size passes it — reported,
not estimated, so it counts cached tokens too. Off by default: compaction is
lossy, and paraphrasing someone's conversation because it got long is their
decision.

The things that decide the design:

- **Stale results are evicted before anything is summarised.**
  `evict_superseded_results` runs first at both compaction sites (threshold
  and overflow recovery): when a later call covers the same target — the same
  `path` across tools, so a write supersedes an earlier read of the file it
  changed, or an identical repeated call otherwise — the older result is
  replaced with a marker naming the recovery. This is the only pass that
  *removes damage* rather than trading tokens for fidelity: a superseded read
  is related to the current state and wrong about it, which the distractor
  literature puts at 25–68% harm where unrelated bulk is near-free. Errors
  neither supersede nor get evicted — a failed call says nothing about the
  target, and "what failed" is what stops it being retried. If eviction (or
  thinning) freed anything, the summary is deferred a turn to see if it was
  enough.
- **And a pile of identical failures collapses onto its newest member.**
  `collapse_repeated_failures` runs beside eviction at both sites. The error
  exemption above is right for one failure and inverts for eight: a model is
  measurably likelier to fail a step when the context holds its own earlier
  errors (self-conditioning, which does not go away with model size), and a
  repeated failure is the same-target near-miss the distractor literature
  prices at 25–68%, not the free kind of bulk. The diagnosis the exemption
  protects is carried by the *newest* failure alone, so that one survives
  verbatim and the older identical ones become markers. The key is target
  **and** exact error text, on the loop guard's precedent: "no such file" then
  "permission denied" on one path are two facts, and collapsing either loses a
  diagnosis — collapsing too little costs tokens, collapsing too much destroys
  information, so narrow is the fail-safe direction. Nothing is removed
  (dropping a `tool_result` block is a 400), and it is deliberately *not*
  counted toward the "freed enough, defer the summary" decision — it removes
  repetition rather than bulk, so treating it as freed space would spend a turn
  arriving back at the same threshold. Distinct from the loop guard, which
  stops a run that has already gone wrong and only after a compaction; this
  runs before there is anything to stop.
- **The cut has to be legal, not convenient.** A `tool_result` whose `tool_use`
  is gone is a 400, and that is the whole run. Tool results arrive in the user
  message right after the assistant turn that asked for them, so the only safe
  place to resume is an assistant message. `compact.rs` is pure and unit-tested
  for exactly this; the loop re-checks the rebuilt transcript before installing
  it, because a guard that fires after the damage is not a guard.
- **A compaction arms the loop guard** (`[agent] loop_guard`, on by default).
  An identical call with an identical result, repeated within a window of
  three calls after any compaction, stops the run with `StopCause::Loop` —
  distinct from `MaxTurns`, because "hit the turn limit" reads as the task
  being too big when a stuck run is a different problem. Keyed on call *and*
  result: polling (same arguments, changing result) never trips it. Dormant
  until a compaction on purpose — repeated calls in ordinary work are the
  model's business, and the failure this catches is specifically the run
  re-living what a summary dropped, at the largest prompts it will ever
  send. Gradeable via `expect.stop_cause: "loop"`; no shipped case asserts
  it, because a case cannot reliably make a model loop, and a case that
  asserts an outcome it may never exercise is worse than no case.
- **The record is searchable after the summary.** `tool/recall.rs` registers
  `recall` on the session-recording front-ends (chat, the TUI, resumed runs):
  it searches the union of everything the transcript ever recorded — including
  what a rewrite replaced — so a run missing a dropped detail can look it up
  instead of re-living the stretch. Taint-neutral by construction: everything
  it can return entered this conversation once, and the transcript path is
  fixed at registration, never model input. Deliberately absent from Slack
  (one shared registry across per-thread conversations would cross-wire
  transcripts) and from fresh one-shots and triggers (a per-run record is
  empty until the run ends).
- **The summariser gets prose, not a replay.** Sending the real messages means
  sending `tool_result`s on a request that declares no tools, and llama-server
  answers that with an empty completion. Found by running it, not by reading the
  spec.
- **Summaries are validated before they install** (`compact_validate`, on by
  default). Two layers: a truncated summary (`stop_reason: max_tokens`) is
  refused deterministically — it lost its ending, which is where "what
  remained" lives — and a second tool-less call reads the summary beside the
  transcript it replaces and lists what is missing. Omissions trigger **one**
  regeneration with the omissions named; the producer cannot see its own
  gaps, and naming them is the intervention. The validator is quality
  improvement, not a gate: an unusable or failed verdict installs the summary
  with a warning, because a run that needs to compact to survive must still
  compact. This is deliberately *not* completion-gating on an LLM judge — it
  is a grounded comparison of two texts both present in the request.
- **Taint survives compaction.** Summarising away the text of a hostile page
  does not un-read it. Taint lives on `Conversation`, which the compaction code
  never touches — the type does the work, and there is a test.
- **The session record survives compaction too.** Compaction rewrites the
  message list in place, and a recorder that slices "what the run added" off
  the end of a rewritten list records a lie — the stale head stays in the
  file, the rebuilt one (summary included) never lands; a 28-turn benchmark
  trial recorded as 8 assistant turns starting mid-conversation, with no
  sign a compaction had happened. `Session::record_run` compares what was
  recorded against what came back and writes a `rewrite` record carrying the
  whole current state when they diverge. Comparison rather than a flag from
  the loop, so any future in-place mutation is caught by construction. And
  the states a rewrite *replaced* are recorded too: the loop snapshots the
  message list before each rewrite pass onto `Conversation::rewritten`
  (bundled with the messages for the same reason taint is), and
  `record_run` — which takes the conversation, so a caller cannot skip what
  it carries — walks those states before the final one. A run long enough
  to compact itself therefore still gets its whole head into the file,
  where `recall`'s union can read it back.
- **A tool's own state crosses a compaction, verbatim.** The measured failure
  mode is that a summariser preserves *what is true* and drops *how far you
  got*, and some of "how far you got" does not live in the messages at all. The
  `todo` list reached the model only through the echo in the last `todo`
  result — a message, and therefore exactly what a summary replaces — so the
  mechanism was quietly conditional on the transcript never getting long, in the
  one situation a plan matters most. `Tool::carried_state` lets any tool hand
  state to the compaction to be kept as-is; `rebuild` puts it *after* the
  summary, because it is the one part of the rebuilt head known to be current
  rather than paraphrased. Three rules keep it from becoming a second source of
  truth: it is read **at compaction time**, so a stale copy is impossible
  because nothing stores one; **exactly one copy survives**, since `rebuild`
  finds the previous block by the `CARRIED_HEADER` sentinel and replaces it
  rather than stacking (two contradictory task lists are worse than none); and
  it is for state the tool *owns*, because a tool returning prose here would be
  smuggling an unvalidated second summariser into the loop. The loop learns that
  some tools have state, never which — `registry.carried_state()`, never a name.

## The eval rig

`eval/cases.jsonl` is graded on the tool-call trace first, text second. Four
kinds of check, in descending order of how much they are worth:

- **Trace and substring checks** — deterministic, free, and they never change
  their mind. Use them wherever they apply.
- **`expect.verify`** — a command run in the case's workspace afterwards,
  passing iff it exits 0. The ground truth for codegen: not whether the model
  said the tests pass, but whether they do. Requires `sandbox`.
- **`expect.judge`** — a rubric graded by a second model, for cases where the
  right answer is a judgement. Not deterministic: the same answer can be graded
  differently across runs, so treat a single judge failure as a prompt to read
  the answer, not as a result.
- **Run-metadata checks** — `expect.stop_cause`, `expect.taint`,
  `expect.blocked_sends`, `expect.min_compactions`,
  `expect.ended_on_failed_call`. Deterministic like the trace checks, and the
  only way to grade the *harness* rather than the model: whether the interlock
  fired, whether a budget was what stopped the run, whether a summary was ever
  taken. None of it is visible in the answer text, and a case that asserts an
  outcome it never exercised is worse than no case.
  `ended_on_failed_call` grades the silent failure — the model stopped on its
  own with its last call failed and answered as though it had not. It is an
  *observation*, not an error condition, because a case whose right answer is
  "that file does not exist" should end on a failed call; a run the harness cut
  short never sets it, or the flag would double-count what `stop_cause` and
  `exhausted` already say. Only the last *executed* call counts: one failure
  among successes is recovery, which is the model working.
  **A denied trace carries `is_error: true` as well as `denied: true`**, so
  every counter that means "the environment refused" must say
  `unknown || (is_error && !denied)` and never `is_error` alone — otherwise a
  read-only run reports the approver doing its job as a harness failure, and
  the rate the candidate gate and doctor threshold on averages in "the harness
  working". The same flag is why `collapse_repeated_failures` skips results
  beginning `Denied by the user:` / `Blocked by policy:` / `Blocked by a hook:`:
  those are the strings the learning miner reads a correction out of, and
  compaction rewrites the transcript in place, so folding three refusals into
  one marker destroys the evidence rather than merely undercounting it.
- Everything a model says about its own work is hearsay. Grade the artifact.

`--runs k` repeats every case k times and reports **pass^k** (all k runs pass)
beside pass@k (any run). Reliability decays much faster than mean success, and
a single-run scorecard cannot tell a flaky case from a solid one — the gap
between the two numbers is the model's unreliability, which is usually the
finding. Sandboxed cases stage one private workspace *per run*, so the k
samples stay independent. Two caveats: a pinned seed at `--concurrency 1`
replays token-for-token, making the k runs one sample counted k times (the
harness warns); and `passed`/`by_tag` in a multi-run scorecard mean pass^k, so
compare it only against scorecards taken at the same k.

What a case can ask for beyond the defaults:

- `"sandbox": true` — a private copy of the fixture, with writes allowed.
  Required for `verify`. The shared fixture is never mutated.
- `"max_turns": N` — a per-case turn budget. A case that genuinely takes twenty
  steps says so, rather than everyone raising the global ceiling for one case
  and quietly changing what every other case may do.
- `"compact_at_tokens": N` — force compaction for this case alone. Same reason:
  turning it on globally would change what every other case is measuring.
- `"prompt": ["...", "..."]` — several turns on **one conversation**. A single
  prompt cannot express anything that only goes wrong across turns, which is
  most of what the harness guarantees: taint accumulating, a transcript growing
  past the compaction threshold. `prompt` stays a bare string for one turn, so
  no existing case had to change.

`eval/graph-cases.jsonl` is a second case set, kept out of `cases.jsonl` on
purpose: it needs MCP tools in the surface, and changing the main set's tool
surface would invalidate scorecard comparisons across the boundary. It runs
against **fixture MCP servers** (`eval/fixtures/graph_server.py`, declared in
`eval/mcp.toml`, connected with `--mcp-file`) — a frozen fake of the knowledge graph,
because the real graph server answers from live, machine-local data and a case graded
against it measures nothing repeatable. The `web` persona's `fetch` tool is
`openWorldHint`, which is what lets `interlock-blocked` grade the trifecta
interlock end to end, offline: memory read arms both taint legs, the fetch is
refused by the harness, `expect.blocked_sends` counts it. `--mcp-file` connects
exactly the servers in the named file (fatal on failure, unlike `setup`'s
warn-and-continue), and resolves relative paths against the file's directory.

Fixtures under `eval/workspace/{audit,reports,kata}` are generated:
`python3 scripts/build-eval-fixtures.py` rewrites them, prints the gold answers
the cases must assert, and checks that each kata fails as shipped *and* is
solvable by a reference fix. A gold answer typed by hand is a guess, and a
wrong one measures nothing.

## Testing without credentials

`agent.rs` has a `ScriptedProvider` that replays a fixed list of turns. Use it
to test loop behavior (tool dispatch, denials, exhaustion, error recovery)
without network access. `mecha tools` also runs without any provider configured,
which makes it a good MCP-server smoke test.

Three layers, and the split is deliberate:

- **Unit tests** for anything that is a function of your own code — the
  Anthropic request body, the OpenAI stream decoder, session round-trips, the
  compaction cut. Free, deterministic, and they never expire. Note the limit:
  a `ScriptedProvider` replays what you *believe* providers do, so it is
  structurally blind to a provider violating that belief — which is where this
  project's expensive bugs came from.
- **Integration tests** (`mecha-core/tests/`) for what is deterministic but
  needs real execution: docker actually confining a command, an MCP server
  actually receiving an environment. A `nosy_mcp_server.py` fixture reports
  everything it can see, so confinement is measured rather than asserted about
  an argv. These skip when the backend is absent — and
  `MECHA_TEST_REQUIRE_BACKENDS=1` turns every skip into a failure, because in
  CI a silently skipped test reads exactly like a passing one.
- **Eval cases** for what only emerges with a real model in the loop:
  compaction fidelity, multi-turn behaviour. Expensive and non-deterministic;
  use them where the other two cannot reach.

Verify a fix by making it **fail on the old behaviour**. Where the assertion is
about the environment rather than about scripted state, establish the same
thing by checking the negative is not vacuous — the confinement tests only mean
something on a machine that *does* have `~/.ssh` and *can* reach the network.
