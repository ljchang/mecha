# mecha — CLAUDE.md

## What this is

A standalone agent harness, extracted so it can be reused across projects
rather than rewritten per project. Two crates:

- `mecha-core/` — the library. Knows nothing about any CLI or application.
- `mecha-cli/` — the `mecha` binary. Thin; all logic belongs in core.

Interfaces: `mecha run` (one-shot), `mecha chat` (readline REPL), `mecha tui`
(full-screen; the input line stays live so you can steer a run in flight), and
`mecha batch` / `mecha eval` for fan-out.

## Build & test

```bash
cargo build --release          # ./target/release/mecha
cargo test                     # unit tests, incl. a scripted-provider loop test
cargo clippy --all-targets
```

`MECHA_LOG=debug` turns on internal tracing (goes to stderr).

## Architecture

```
message.rs   provider-agnostic Message/Block/Usage/StopReason types
provider/    Provider trait + anthropic.rs (raw HTTP) + openai.rs (compatible)
tool/        Tool trait, Registry, Approver, builtin.rs
mcp.rs       stdio JSON-RPC client; wraps remote tools as Tool impls
agent.rs     the loop: ask → run tools → feed results back → repeat
hooks.rs     user commands at lifecycle points; pre_tool can deny a call
cron.rs      five-field cron, resolved in an IANA zone (both DST directions)
trigger.rs   scheduled prompts: the store, the ledger, and "is it due?"
learning.rs  the reflection/rule store behind reflect, learn, validate
distill.rs   session → episode, staged to the knowledge graph over MCP
session.rs   append-only JSONL transcripts
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

Two things are enforced structurally rather than by prompting:

**The path jail.** Every model-supplied path goes through `ToolCtx::resolve`,
which canonicalizes and proves containment in the workspace. Never call `fs::*`
on a raw path from tool input.

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
describe, so coverage can over-taint, never under-taint), and `mecha learn`
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

Known gap: `shell` is universal and taint tracking can't see inside a command,
so it is not treated as an untrusted *source*. The mitigation is the sandbox.

**The sandbox** (`sandbox.rs`, `[sandbox]` in config) is the enforcement behind
`shell`'s capability label. `kind = "bwrap"` uses user namespaces; `"docker"`
runs a throwaway container; `"none"` (the default) runs commands as you. A
confined command gets the workspace, a read-only system, no home directory, no
environment except a named allowlist, and by default no network.

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

On Ubuntu 23.10+, `bwrap` fails even when installed and
`unprivileged_userns_clone=1`, because AppArmor gained a separate switch:
`kernel.apparmor_restrict_unprivileged_userns=1`. Use `docker` there, or install
an AppArmor profile. `mecha tools` prints the active sandbox, and
`mecha tools --json` prints each tool's capabilities.

## mecha-mail

A third crate, extracted from flowmail: `mecha-mail/` is a **library plus
three thin MCP binaries**. The library (Gmail + Google Calendar v3, Outlook
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
account address comes from Sent Items instead (flowmail reached the same
conclusion). And an account lookup must never be fatal to `auth`: losing a
completed sign-in over a cosmetic detail makes the user authenticate twice.

Four flowmail behaviours are fixed rather than ported, each filed upstream
(`ljchang/flowmail` issues 3–6): Graph replies go through
`POST /messages/{id}/reply` so they thread; the calendar reads `calendarView`
so recurring events do not vanish from a window; search uses `$search`
instead of a `$filter` that 400s beside `$orderby`; and `to` splits on commas
like cc and bcc already did.

What flowmail did not have and this crate does: **the token lifecycle in
Rust** (flowmail kept storage, refresh, and retry-on-401 in its JS frontend)
— `oauth.json` at mode 0600, refresh ahead of expiry behind a lock so two
concurrent tool calls cannot race two refreshes, one forced refresh and
retry on a 401; **retry with backoff** on 429/5xx; and an **HTML→text
fallback**, because flowmail took only the `text/plain` part and an
HTML-only email reached the model as an empty body.

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
- The store follows the learning store's rules: one pretty JSON per item,
  temp-sibling-and-rename, advisory flock (never held across `$EDITOR`;
  staging takes no lock at all, so the agent never blocks on a review).
  `send` holds the lock across execution so two sends cannot double-fire.

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
provider reports what a prompt *cost*, never what is left. Three things
depend on it, and without it all three degrade silently:

- **`AgentConfig::compact_at`** derives a compaction threshold (two thirds of
  the window) when `compact_at_tokens` is unset, which turns compaction from
  something you must remember to configure into something that works. The
  fraction leaves a third free because the check happens *between* turns:
  the next request still has to fit a reply and whatever a burst of parallel
  tool results adds.
- **The TUI status line** becomes a fuel gauge — `context 29.3k/32.8k (89%)`,
  yellow at 75%, red at 90% — instead of a number with nothing to compare to.
- **Overflow recovery.** A prompt that does not fit is refused outright, and
  the reactive threshold cannot always prevent it. `is_context_overflow`
  recognises the refusal across backends by message text (no backend gives it
  a usable code), and the loop compacts and retries the same turn *once*. A
  false positive costs one summary; a false negative loses the whole run,
  which is what used to happen.

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

Four things that decide the design:

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
  `expect.blocked_sends`, `expect.min_compactions`. Deterministic like the trace
  checks, and the only way to grade the *harness* rather than the model: whether
  the interlock fired, whether a budget was what stopped the run, whether a
  summary was ever taken. None of it is visible in the answer text, and a case
  that asserts an outcome it never exercised is worse than no case.
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

`eval/pkg-cases.jsonl` is a second case set, kept out of `cases.jsonl` on
purpose: it needs MCP tools in the surface, and changing the main set's tool
surface would invalidate scorecard comparisons across the boundary. It runs
against **fixture MCP servers** (`eval/fixtures/pkg_server.py`, declared in
`eval/mcp.toml`, connected with `--mcp-file`) — a frozen fake of the pkg graph,
because the real pkg answers from live, machine-local data and a case graded
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
