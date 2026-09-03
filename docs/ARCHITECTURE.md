# Architecture — the subsystem invariants

Moved here from `CLAUDE.md` on 2026-08-28, verbatim, because that file rides
in every agent's context on every run and had grown to 2,500 lines — a cost
paid forever, mostly for sections relevant only when working on one
subsystem. `CLAUDE.md` keeps the cross-cutting essentials and points here;
this file is the one to read **before changing a subsystem it names**, and
the place a new subsystem's invariants land.

The writing convention is `CLAUDE.md`'s: state the rule, then the incident
behind it in one sentence — each entry exists because undoing it is a bug
that already happened, or provably would. Where a topic doc owns the detail
(`TRIFECTA.md`, `LLAMA-SERVER.md`, the `*-DESIGN.md` files), a section here
points at it rather than restating it.

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

**Two HTTP clients per provider, chosen per request by whether the body
streams** (`provider::HttpClients`, `for_body`). reqwest's `read_timeout` is a
*client* setting that also bounds the wait for the response head, and
`RequestBuilder::timeout` overrides only the total deadline — so one client
cannot both bound a stream per read and give a non-streaming request its long
exchange cap. The streaming client bounds each read at `STALL_TIMEOUT` (a long
answer is not a stall; the server is either sending tokens or it is not) and
the exchange only at `STREAM_TIMEOUT`, a far cap for the connection that keeps
emitting keepalive bytes and never finishes; the whole-exchange client bounds
the exchange at
`REQUEST_TIMEOUT` and has no per-read bound, because with `stream: false` the
entire generation is one silent read. Both providers used to put one 900 s
`timeout` on the whole exchange, streamed body included, and a legitimate
long answer died mid-stream with the partial discarded (found by the
2026-09-02 audit); the first fix used one client with a read timeout and
*tightened* the non-streaming cap to that read bound, which the PR review
caught. The guarantee is measured against a socket that goes quiet, not a
builder field. **The scope is the request, not the run:** only a request with
an event sink or a cancel token streams (`Agent::complete`), so `mecha batch`,
`mecha eval`, the distiller, the reflector and the eval judge take the
non-streaming path and keep the exchange cap — raised to clear a long local
generation, and still the only bound that path has. A timeout there classifies
as `Transport` and is retried, which re-issues a generation the server already
finished; no tool runs on that path, so nothing duplicates, but the wait does.

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

## The local model server

**`docs/LLAMA-SERVER.md` is the reference** — slot geometry, the KV arithmetic,
the measured `-np` table, the request contract, and what each flag cost to
learn. `scripts/start-moe-mtp.sh` is the authority on the flags themselves.
Read the doc before changing anything there; most of its content exists because
something had already gone wrong.

The parts that bite hardest:

- **`-c` is divided across slots**, so `context_window` must equal `-c / -np`,
  not `-c`. Confirm from the startup line (`n_ctx_slot = …`), not by arithmetic.
- **Two servers, one model each** — :8080 chat, :8081 embeddings. llama-server
  holds one model per process, so pointing both at one port sends embedding
  requests to the chat model.
- **`max_tokens` must sit comfortably above `--reasoning-budget`**, or the
  thinking block eats the allowance and the reply is HTTP 200 with an empty
  `content`. Any client here refuses that by name rather than treating it as an
  answer.
- **Ask what is served (`GET /props` → `model_alias`), don't assert it.**
  llama-server ignores the request's `model` field, so naming one is not
  selecting it — only deciding what gets recorded.
- **Throughput is wall clock.** The server times a request only while it is
  running, so summing its per-request rates hides queue wait and reads ~4× at
  `-np 1`, on the one configuration that cannot run anything concurrently.
- **Slot affinity needs its own test** (`scripts/affinity-test.py`). A
  throughput benchmark sends independent prompts, which is the workload with no
  prefix to lose, so it is structurally blind to the regression
  `--cache-idle-slots` caused. The metric is `prompt eval time = … / N tokens`
  staying small across turns, not tok/s.

The model is **hybrid attention** — 11 of 41 layers hold a KV cache, the other
30 carry a constant-size recurrent state — which is why long context is cheap
here (63 tok/s at 108k against 92 at 1k) and why the KV cost is 22 KiB/token
rather than the 82 the naive per-layer arithmetic predicts.

## Images

`Block::Image` is a fifth variant on `message.rs`'s block type, and it is
**user turns only**. Anthropic accepts an image inside a `tool_result`; the
OpenAI dialect's `role: "tool"` messages carry a string and nothing else. A
tool returning pixels would work on one backend and silently lose them on the
other, in the one place where the missing thing is what the whole turn was
about. So an image enters the way a person hands one over — the connector and
the TUI attach it to the turn — and "look at the chart you just made" is
deliberately not built.

Four decisions, each a bug if undone:

- **Both backends degrade to a named line rather than failing**, and a test
  asserts they word it *identically*. A conversation is one object that
  survives a `/model` switch, so two renderings that drift apart would have a
  transcript telling two stories about its own history depending on who was
  asked. It also means a run against a text-only model behaves exactly as it
  did before the variant existed.
- **The parts array is built only when an image is present.** The cached
  prefix is a byte-prefix match, so making `{"content": [...]}` the uniform
  shape would invalidate the prefix of every run that never sends an image —
  and plenty of OpenAI-compatible shims accept only the string form.
- **Caps are applied at the door, never per turn** (`image.rs`). The
  transcript is append-only and every turn resends the whole history, so a
  resize is paid once and collected on every turn afterwards. `MAX_EDGE`
  1568 and `MAX_BYTES` 5 MB — Anthropic's hard per-image limit, applied to
  local servers too because a conversation is one object. An image that
  already fits is passed through **byte for byte**: re-encoding a crisp
  screenshot of text is a real loss, and that is the case this exists for.
  Measured: 5.7 MB → 179 KB with `prompt_tokens` identical at 294, because
  llama-server tiles to a fixed count regardless.
- **`recall` returns the filename, never the payload.** Base64 is a haystack
  of every alphanumeric substring there is, so returning `data` would make a
  one-letter query match every image and print a megabyte back into the
  context that tool exists to protect. `render_for_summary` does the same,
  for the same reason plus a sharper one: the summariser is a tool-less
  *prose* call, so the payload could only arrive as literal text in a request
  whose whole purpose is to be smaller than what it replaces.

**Three doors, and which one you can use is decided by where you are
sitting.** The Slack connector and the remote-control inbox attach an image to
the turn after landing the file in the workspace; `mecha run --image` is the
scripted one; and **dropping a file on the TUI prompt** is the local one.

That last is not a drop protocol: a terminal converts a drop into a *bracketed
paste of the path*, which is why one `Event::Paste` arm serves both — and why
it **cannot work over SSH, ever**. The path pasted is the path on the laptop
and the process resolves it on the box at the other end. The bytes never left
the laptop. Nothing in the harness can fix that, and it is the reason the
Slack conduit exists.

Two rules on the drop path:

- **Every token must resolve to an existing image, or it is not a drop.** A
  paste is also a paragraph somebody copied off a web page, and a rule that
  attached any file whose path appeared *somewhere* in pasted prose would let
  copied text pull bytes off the disk into a request. Requiring the whole
  paste to be paths and nothing else makes "was this a drop" decidable rather
  than guessed. The whole paste is tried as one path *before* splitting,
  because terminals disagree about escaping and a raw `/shots/a shot.png` is
  indistinguishable from two files by splitting alone — asking the filesystem
  settles it, and a bare space is what every macOS screenshot has.
- **The chip is the handle, so deleting it detaches.** Base64 cannot live in a
  `String` edited with arrow keys, so the bytes sit beside the input and
  `[image: shot.png]` stands in. An entry is sent only if its chip survives to
  submit — otherwise a dropped image is unreachable by backspace and the only
  visible sign of it is text that does nothing. A non-image file inserts its
  path unchanged, which is what a dropped `.csv` wants.

**What lands on disk differs by door, and the difference is an affordance
rather than an inconsistency.** The Slack and remote-control doors write the
**original** into `<workspace>/inbox/` and *also* name the path in the prompt,
so the model gets both bytes it can look at and a path it can `shell`. The
terminal doors copy nothing: the file stays where the person had it, and — for
a drop from outside the run's workspace — it is beyond the path jail, so the
model gets pixels and no way to reach the file. Worth knowing before asking a
run to "crop that".

The resized image is **never written to disk**. It exists in the message and
therefore in the transcript, and the original on disk is always the original.
Which is the whole argument for capping at the door, in one measurement: a
single screenshot was **99% of a session file** (244,120 of 246,472 bytes),
and every turn resends the whole history — so that is the per-turn wire cost
for the rest of the conversation, and uncapped it would have been 7.5 MB.

Known and accepted: `~/.mecha/work/slack/<thread>/inbox/` is swept by
`mecha work clean` like any other producer entry, but a remote-control session
jailed to a **real project directory** puts `inbox/` in that project, where
nothing sweeps it. That is the cost of the workspace being somewhere real, and
it is the same trade `[work] keep` cannot make on the user's behalf.

**An attached image arms `private_data`, and the reason it is not free the
way typed text is.** `Taint::arm_for_content` is read off the messages by the
loop at run start — deliberately not by `Conversation::push`, which would be
the tidy place and is not the safe one: the Slack connector appends to
`messages` directly, so arming there would have left the path people actually
attach screenshots from unarmed. Recomputed per run rather than tracked, which
is idempotent because taint only ever grows.

The argument is that **a screenshot is captured, not composed**. Inbound text
arms nothing because the user chose every word; that reasoning does not reach
an image, where they chose the window and not everything in it — incidental
private data is the normal case, and most of why people screenshot instead of
retyping. It also keeps an unchanged user action's posture unchanged: before
images existed, a Slack attachment armed `private` because the model had to
`fs_read` it, and putting the pixels on the user turn removed the tool call
and the taint with it. A feature that silently loosens the interlock as a side
effect is the shape this project keeps finding, and it was found here by
reading the recorded taint of a working run, not by reasoning about the code.

**Whether the model has eyes is declared, and verified against the server.**
`[providers.X] vision` defaults to true for `kind = "anthropic"` and false
everywhere else — false is the safe direction for a local server, because the
failure it prevents is a rejected request where the other is merely a model
that cannot see, which is what it already was. `provider::preflight` reads
`GET /props` once at startup and warns in **both** directions; the reasoning
and the mmproj trap behind it are `docs/LLAMA-SERVER.md`'s to hold. The rule
worth carrying: **a vision model is two files**, and the second one is
invisible when missing — nothing errors, `/props` reports what is *loaded*
rather than what is supported, and the model simply says it cannot see.

**With `fallbacks` configured, "can the model see" is the primary's answer.**
`Failover::vision()` forwards to the primary (it used to inherit the trait's
`false`, so configuring any fallback silently turned every image into a
placeholder). The honest residue: a turn that fails over to a fallback with
`vision = false` is still rendered with the image blocks the primary
accepted, because both renderers emit an image the request carries. That
turn may be rejected, which is the loud failure and the right one — a
fallback answering a question about a picture it cannot see would be the
quiet one. Pair a sighted primary with sighted fallbacks, or accept that
fallback turns on image conversations fail.

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

**Learning is ungated, and the gate that remains is a measurement.** The
owner's 2026-08-19 ruling (`LEARNING-AUTONOMY-DESIGN.md`, shipped 2026-08-30):
a rule goes live when it is derived, not when someone approves it. `learn
--auto` is the path both unattended callers take — `ruminate.sh` nightly and
`learn-live.sh` on every session close — with the counterfactual gate still
in front of the write: a candidate that regresses any probe is refused; one
that measures clean applies; one the gate **could not grade** applies on
**probation** (`Rule::probation`), live but on the shorter retirement leash
until the ledger grades it **beyond its convictions**
(`release_probation_when_measured_clean` — one predicate, shared by
`stamp_probation` and the retirement scan). The release deliberately does
not key on bare coverage: an attributed regression always arrives inside an
observation, so a coverage-keyed release stripped the leash on the very rows
that convict and made `PROBATION_RETIRE_AT` unreachable from the automated
path — found by the retirement drill the day it first ran. Inconclusive rows
release nothing (a probe that ran is not a probe that graded), probation
survives consolidations (`finalize_rules` carries it), and only the ledger
releases it. Every pass still writes a proposal record — the audit
trail, resolved at birth — and a new proposal for a domain **supersedes** its
pending predecessors, releasing their reflections unconsumed (`proposals
supersede`; reject is the owner's no and consumes them — the difference is
the whole verb). `measured` counts probe pairs that *graded*, never pairs
that merely ran: "no evidence" and "evidence of clean" are opposite findings.

**Acceptance is not tenure.** A live rule rides in every future prompt's
cached prefix, so it keeps earning that seat or loses it. Rules carry
`id`/`sources`/`created_at` (minted by `finalize_rules`, carried by
text-match across consolidations; pre-identity TOML loads unchanged). `mecha
validate` appends every probe's outcome to the **validation ledger**
(`validations.jsonl`, keyed to the exact rule set measured), and a regressed
trace-graded probe **bisects** the active learned rules against the same
recorded prefix to name the rule that flips it — user rules ride in every
test arm (they are not on trial), a regression they cause alone attributes
to nothing, and an inconclusive arm aborts the attribution rather than
guessing. `mecha rules` folds the ledger into per-rule tallies;
`rules propose-retirements --apply` (nightly, after learn) **retires
directly** — no queue, no human — once a rule accumulates the attributed
regressions its leash allows: 3 ordinarily, 2 on probation
(`PROBATION_RETIRE_AT`) — a deterministic ledger scan, no model anywhere,
and it resolves any pending retirement proposal it overtakes as superseded.
Retirement is the brake ungated learning leans on and it is a flag, never a
deletion: the rule stays in the file as evidence, the learner is shown it as
"measured harmful — never re-derive" (surviving even a reworded
re-derivation, by `normalized_rule_key`), and `rules restore` undoes it.
**The brake has been exercised whole, not only piecewise**:
`scripts/retirement-drill.sh` seeds a probationary bad rule into an isolated
world, drives real probe passes against the live model, and asserts the full
motion — regression, bisection convicting the right rule, one conviction
holding, the second retiring at the probation leash of 2 — and its first run
found the release bug above, which every unit under it had passed over. Run
it after touching validate, the bisection, tallies or the retirement scan.
Deliberately absent: decay, TTLs, usage-based eviction (the rarely-fired
rule that must never expire), and any policy built on model-rated
confidence — only measured harm argues for retirement. `mecha
learning-report` is how anyone knows the loop is improving; `mecha eval
--ab-rules` is the coarse complement: the case set runs rules-free then
rules-on and the per-case flips are their own artifact, never a comparable
scorecard. The evidence behind all of this is `docs/MEMORY-RESEARCH.md` and
`docs/LEARNING-LOOP-RESEARCH.md`.

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


## The task board

`mecha tasks` and the TUI's `/tasks` are the GTD board in the knowledge graph,
which is somebody else's store: mecha reaches it **only through the MCP tool
surface** (`kg_task_list` / `kg_task_create` / `kg_task_update`), exactly as
`mecha mail task` already did. That is the whole design — no dependency on
`mecha-graph`, no second reader of its SQLite schema, and a lookup that matches
on the tool's *suffix* so a renamed server or a `prefix_tools` flip does not
turn the board into "no tasks". The one shared helper is `setup::find_tool`,
because two copies of that rule is two places for a hardcoded `graph__` to
creep back in.

The modal drives the CLI, on the `/triggers` rule, so nothing it can do is
missing from a script. Three decisions on top of that:

- **Nothing confirms.** `/mail` confirms on `s` because spam trains the
  provider's filter and so reaches outside the mailbox; the board reaches
  nobody (`openWorldHint: false`), every status is one keystroke from where it
  was, and the tool surface has no delete at all. A confirmation on a
  reversible private change teaches people to hit enter without reading, which
  is what the outbox's confirmations then have to fight. One narrowing since:
  **"one keystroke from where it was" is the *owner's* property, not the
  model's** — closing a task consumes §5.4's one-shot closure appraisal, so
  on every model-facing registry `kg_task_update` is wrapped
  (`closure_guard::ClosedStatusGuard`, `setup::build`, inherited by
  subagents) to refuse a `status` of `done`/`dropped` and point at
  `mecha tasks set`, which closes *and* appraises. Every other field of the
  tool passes through; D6's outright withholding on delegated runs is
  unchanged and sits on top.
- **A reload re-finds the cursor by id.** Changing a status is also the thing
  that *reorders* the board — actionable first, then by due date — so an index
  carried across a reload names a different task to the next keypress, and the
  next keypress might be `d`. The `/outbox` hidden-items toggle learned this as
  an edge case; here it is the common path.
- **The status letters are `mecha-graph tui` screen 6's letters**, and a test
  says so. Two boards over one store with divergent keys is a trap, and the
  keystroke it springs is `x` on something you meant to finish.

An edit form offers due, defer and context and *not* the name, because
`kg_task_update` has no rename — a box that silently discarded what was typed
in it would be worse than not offering one.

**A delegated run's seed points at context; it never pastes it.** The prompt
is built by code from the board record (D4) — name, project, context, dates,
`defer_until`, the owner's note — plus `captured_from`, the pointer saying
what asked for the task. What the run may look up is *named*: the mail-thread
reader when the capture is mail, and whichever of `kg_search` / `kg_entity` /
`kg_related` / `kg_timeline` this surface holds. Three rules on it, each a bug
if undone. **The pointer, never the prose** — `captured_from` can point at
mail, and a pasted body would arm `untrusted` before the run's first turn and
put attacker-controlled bytes in a privileged run's opening instruction, so
the seed carries kind/id/account/timestamp and never the pointer's `label`;
the bytes arrive as a tool result the interlock accounts for. **The residual
is the task's own name, and it is capture's rather than the seed's**:
`mail task` defaults the name to the classifier's `one_line` and then to the
raw subject, so a default capture puts a model's paraphrase of somebody
else's mail at the top of a privileged run's instruction — and the front
door's own rule is that a paraphrase of an injection is the injection
rearranged. The name has to reach the run, so this is not fixable by
withholding; it is a question about what capture should default to, and it is
named in HANDOFF rather than papered over here.
**Registered names, resolved off the registry after every narrowing** — this
box registers the graph tools bare and mail as `mail__mail_get_thread`, so a
bare name in the seed would be a call the run cannot dispatch, which is the
level-3 skill bug one door over. And **a tool that is absent is named
nowhere**: a capture kind with no reader (`frontdoor`, `session`) is stated as
provenance and offered nothing, because a pointer to a reader that is not
there is worse than the plain fact. Progressive disclosure is the argument for
the whole shape — the seed is the front of a prefix every turn re-sends, so
pasted context is paid for on all of them and a sentence is paid once.

**A delegated run is a conversation, and the two postures differ in exactly
one thing.** `ask mecha` opens the task's own chat session in `mecha serve`
(D2: *the run is a conversation from the start*) — the ordinary chat surface,
so voice, uploads, the todo panel, approval cards and steering-by-typing come
free rather than being rebuilt. The model speaks first, and **nothing on the
board moves**: `waiting_on` names who has the ball, and while the owner is in
the conversation they do. *Let it carry on without me* hands the same
transcript to a detached `tasks work --resume` child. Neither posture is more
capable — the loop runs unattended in serve just as long — the difference is
**what happens when nobody answers**: a card in the conversation, and in the
child a question that *ends the run* and waits in the store. That is a fact
about the owner, not about the run, which is why the button is worded as
leaving rather than as launching.

Four rules make it safe, each one a rule that had previously been enforced in
only one place:

- **D6 rides on the run.** *The agent may not close its own task* was
  enforced by a spawned child taking `kg_task_update` off its own private
  registry; a web process holds one `Arc<Agent>` for every session, so there
  is no private registry to take it off. `RunContext::withheld` is a
  **denylist** beside the skill allowlist, checked at the dispatch seam,
  landing on the same `Blocked by policy` refusal (never an environment
  error), inherited by subagents like hooks and the outbox route, and matched
  through a server prefix so `prefix_tools` cannot switch it off silently. A
  resumed task transcript keeps it — D6 belongs to the conversation, not to
  how it was opened.
- **The transcript is the record; the session map is a cache.** Re-opening a
  task after a restart resumes from the session id the board has held since
  the conversation opened, rather than minting a blank one under the same key.
- **Hand-over is a transfer of the single writer, not a copy**: release, then
  spawn, and the child's turn says only what changed — the plan is already
  above it, and restating it would replace what was agreed with a paraphrase.
- **A question with nobody there parks.** Not a mode switch on whether a page
  is connected: a backgrounded phone stays connected, so that switch shows a
  card to an empty room and expires it into a refusal. The card is offered
  whenever anyone might see it, and both ways of going unanswered end the same.

**A delegation's turn ceiling is its own** (`TASK_MAX_TURNS`, 200 —
Terminal-Bench's figure). The two limits compose by **override, not
minimum** — `cx.budget.max_turns.unwrap_or(cfg.max_turns)` — so a task run
inherited whichever surface launched it, and a tightened `[agent] max_turns`
silently tightened delegations with it. The argument for being generous: the
ceiling is not what stops a runaway run — the loop guard, the token budget
and compaction are — so it should only ever stop an honest one, and stopping
one reports as `MaxTurns`, which reads to the owner as the model giving up.

**Background delegations queue against each other; the owner never queues.**
`permit.rs` is a directory of files on `runmarker`'s rules, asked the
neighbouring question — *may I start* rather than *am I running* — with the
same pid-checked sweep, so a holder that is killed outright costs one stale
file rather than a permanently smaller pool. Three seats against the server's
four (`-np 4`), and the number is measured: throughput saturates at the seat
count, so a sixth concurrent conversation buys 6% more work than four while
costing 42% more latency per turn. **Interactive work takes no permit at
all** — that is the reserve, implemented as an absence rather than as a
mechanism, because a pool that could refuse the person at the keyboard is a
control failing closed against the only user it exists for. The refusal names
who is holding, since "busy" with no reason is not something a person can act
on. And it is **not** a prefix-cache control: six conversations on four slots
re-prefilled 31 tokens a turn, never a transcript, so anything validating
this on cache reuse will find no effect — judge it on latency.

**A delegated run is detached, so talking to it is a file — and the same
file settles who owns the transcript.** `tasks work` runs as a child of
whatever launched it, so its `Conversation` lives in memory no other process
shares and its JSONL has one writer. Two consequences, both structural.
**Steering is `<task>.steer`**, appended by `mecha tasks steer` and drained
by a poller into the run's own `queued_input` — the queue a TUI's typed
steering already feeds, so the loop sees an instruction arrive on the message
carrying the tool results and never learns it came from a file.
`run_interruptible_watching`'s `pump` is named for that ignorance: something
to do on every tick, not "check for steering". The store's own three rules
carry over — appended (two sentences a second apart are two intentions),
drained (a file that survives delivery arrives again every turn), and cleared
at `mark_running` (a steer left by a kill must not reach the next run, which
is the stale-cancel bug this module was extracted for). **And the run marker
names the session it is writing**, because "one conversation, one writer" had
only ever been checked *within* a process: `resume` refuses a twin of a
session this process holds and could not see a detached child, so resuming a
delegation mid-flight would have put two writers on one transcript.
`live_writer_of` asks the marker directory instead, a dead marker is swept on
the way past so a crash cannot lock a transcript out, and the refusal names
the task so the owner can stop or steer it instead. A UI condition is not a
guard.

**A delegated run that needs a decision ends, and its question is a store**
(`questions.rs`, `mecha questions`, `/api/questions`). The outbox's inbound
twin, and the reasoning is the outbox's run backwards: a staged send is a
run's *outbound* act surviving the run's end, and nothing let a run's
*question* do the same — `ask_user` parks the run itself, which is right for
a page open in a hand and wrong for a task, where the honest case is that
nobody answers until morning. So `ParkingAsker` cancels the run's own token
(keeping the partial turn, exactly as Ctrl-C does), stores the question, and
`waiting_on` moves to the owner; answering **is** resuming — the answer
becomes the next user turn of the conversation that asked, in the jail it
asked from, with its plan rehydrated. No slot and no cached prefix are held
overnight, and the ball-passing needed no new noun, because `waiting_on`
alternating between owner and agent is the GTD semantics the board already
has.

Two rules on the surfaces over it. **Reading is a store read and every
mutation is a `mecha …` child**, which is `review.rs`'s split rather than
`board.rs`'s — the question store is mecha's own type, where the board
belongs to another repository and must go through MCP or become a second
reader of somebody else's schema. And **the two mutations spawn
differently**: abandoning writes one record and is synchronous, while
answering is a whole agent run and detaches, with `--unattended` — which is
load-bearing rather than ergonomic, because an interactive agent spawned with
`/dev/null` on stdin takes EOF as a refusal and files every one as
`"Denied by the user: "`, the string the learning miner reads a *correction*
out of. A question answered from a phone would otherwise teach rules from a
person who was never asked.

**And the card's state is derived, from three sources, none of which is the
run's account of itself.** The board says who holds the ball, the question
store says whether it is blocked, and the transcript's `Record::Outcome` says
how the last run stopped — so a run that reported "all done" with its last
three calls blocked is caught by construction. Two states carry the weight:
*answer needed* is loud, because it is the only one that stalls indefinitely
and the only one whose remedy is a person; and *failed* must never render as
*idle*, because "nothing is happening" and "it broke" are opposite findings.
That second rule is why there is a seventh state — a transcript with no
outcome record is a run that never got as far as saying how it went, and
reporting it as either of the other two invents the one fact the card is
about. `Interrupted` reads as finished and never as failed, on doctor's own
rule for the same field: a person stopping a run is the system working.

## The unified queue — `/queues`

Five stores accumulate work for the owner, and each grew its own verb: the
outbox, the front door, staged rule proposals, harness candidates, and — in
another repository — the knowledge graph's merge queue. Knowing what was
waiting meant remembering five commands, which is how that last one reached
**6,434 items** without anybody deciding to let it.

`mecha review` is the aggregator and `/queues` is its modal, on the `/tasks`
rule: every read and every mutation drives `mecha review …` as a child
process, so there is one implementation per verb. It is doctor's shape rather
than a sixth store — it reads what the others own and holds nothing.

Three decisions carry it:

- **It is `/queues`, not `/review`.** `/review now|later|auto` is already the
  outbox's release policy and `app.review` is already a `ReviewMode`. Two
  things called review, one word apart, is a trap; the modal is named for the
  stores rather than for the act.
- **The graph queue is reviewed in place; the other rows hand off.** `/outbox`
  and `/frontdoor` own the confirmations and taint warnings that make their
  approvals safe, and a second copy of those here would be a second thing to
  keep correct. The graph is in place because nothing in mecha could reach it
  at all before.
- **An unreadable store reports as a dash, never as zero.** "Nothing waiting"
  and "could not look" are opposite findings, and a reader that rendered its
  own failure as an empty queue would reproduce exactly the bug this surface
  exists to catch. `queues` still reports the four mecha-owned stores when the
  graph binary is missing.

**And this is the one place mecha shells out to `mecha-graph`.** The rule
above — mecha reaches the graph *only* through the MCP tool surface — still
holds for reads and is not broken here: nothing opens the database. But the
tool surface **cannot accept a fact candidate**, and that is a decision rather
than a gap. `kg_pending` reads and `kg_verdict` files an opinion that decides
nothing; there is deliberately no `kg_accept`, because every MCP tool lands in
the model's registry, and a model that can accept candidates can accept the
ones its own extractor proposed — which is `ladder.rs`'s oldest rule, *a lane
must not promote itself*, defeated structurally.

So the decision is driven the way a person drives it: `$MECHA_GRAPH_BIN` (else
`mecha-graph` on `PATH`) as a child process, the `/triggers` rule one
repository over. Nothing new becomes reachable from a prompt. The binary is
resolved from the environment and deliberately **not** from `mecha.toml` — a
project file arrives with a cloned repository, and a project that could name
a binary mecha runs as a child process has been handed arbitrary execution,
the same reasoning that keeps `[[trigger]]` out of the layered config. The
dependency is runtime and optional, and every verb degrades to a named error.

**`t` filters by evidence tier**, at the proposer and class levels: `all →
unjudged → thin → some → solid`, bucketed by how many verdicts of the owner's
own the rate rests on (0 / 1–9 / 10–29 / 30+). `Tier::of` is the single
definition behind both the printed label and the filter — two would drift, and
a filter disagreeing with the word beside it is worse than none, since you
would verdict a class believing it sat in a tier it did not. The cursor
returns to the top on every change: a filtered list is a different list, and
at the class level the next keypress verdicts everything in the row. Purely a
display toggle — the rows are already loaded, so it costs no subprocess.

**Item review is a random sample, and that is the default rather than an
option.** `Enter` on a class draws twelve candidates uniformly at random
(`mecha review sample`, `mecha-graph review --sample`); `a`/`r` verdict one at
a time. The queue has an order, every order it could have is correlated with
something — age, id, confidence — so judging the first dozen and reading the
result as the class's accept rate measures the ordering. Not a theoretical
worry: 40.5% of the queue sits in classes with no human verdict at all, and
the only cheap way to learn whether they are worth keeping is a draw
uncorrelated with the content.

Three rules on the draw:

- **The seed is chosen by the caller and printed.** A sample nobody can redraw
  is a sample nobody can check, and these exist to produce a number somebody
  will quote. The TUI picks the seed rather than letting the graph pick one,
  because `--json` does not report the seed it drew.
- **A verdict does not resample.** The item is dropped from the list locally
  and the seed is unchanged, so a sitting's twelve verdicts describe *one*
  sample. `n` asks for a new draw, explicitly. Re-running the draw after each
  keystroke would spread a sitting across a dozen samples and quietly destroy
  the property the sampling was for.
- **A partial Fisher-Yates, not shuffle-and-take**, with a unit test asserting
  uniformity over 4,000 draws — it fails on `items.truncate(k)`, which is
  exactly the bias the flag exists to escape. The PRNG is four lines of
  splitmix64 rather than a dependency: nothing here needs cryptographic
  randomness, and `rand` would be a new dependency in the one binary that
  reads the owner's encrypted graph.

`mecha review items` is the queue-order alternative, for a class already
decided about. It prints a line saying its verdicts are not a rate.

**`review --proposers` is the evaluation surface underneath it** (in
`mecha-graph`): the queue rolled up by proposing mechanism, with each one's
**human** accept rate, so "is this mechanism worth running" is answerable
without reading 733 (proposer, predicate) rows. Two rules there, both learned
the expensive way on 2026-08-22:

- **Machine rejects are never counted as the owner's.** `precheck`'s own dedup
  and ephemeral rejections used to land in the cluster view's `rejected_hist`,
  which is the number a person reads immediately before verdicting a whole
  class. It displayed `llm/has` at 18% against a true 67% over 48 human
  verdicts, `llm/has_role` at 7% against 53%, and showed three classes at 0%
  on which no human had ever voted. `ladder::human_record` had the correct
  filter all along; the view did not. They are reported side by side now,
  because a class that mostly repeats itself is a different problem from one
  that is mostly wrong.
- **An unjudged class has no rate, not a rate of zero.** 40.5% of that queue
  sits in 660 classes with no human verdict at all, and rendering that as 0%
  makes an untouched mechanism indistinguishable from a rejected one. Every
  surface prints a dash.

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

## The remote control

`/remote-control <name>` makes a live TUI session and a named Slack thread the
same conversation. `docs/REMOTE-CONTROL-DESIGN.md` is the design; what belongs
here is why it resists the obvious changes.

**One owner, many terminals.** A `Conversation` — messages *and* taint — lives
in the running process's memory and the session JSONL has one writer, so two
processes cannot both hold a live conversation. The TUI keeps the agent and
Slack is a view plus an input channel. There is no symmetric design to look
for, and this is why the connector must *not* answer for a mirrored thread: it
used to mint a thread record and start a fresh conversation in a different
workspace under a different permission mode, answering in a scrollback it knew
nothing about. Not a leak — that conversation is clean — a stranger wearing the
thread's clothes.

**The destination is never an argument.** `send_file` and `show_file` take a
path and read the thread from this process's own attach record; there is no
channel parameter and deliberately no way to add one. That is the whole reason
`show_file` can sit in the third quadrant beside `mail_triage` — not
`external_send`, never outbox-routed — despite putting bytes on the network: it
reaches the owner's own two-party DM and nothing the model says can move it.
The test asserts the *absence* of any destination field, because that is where
the property is either true or not.

**A name is durable and its thread is forever**, so detaching marks the record
cold rather than deleting it — the record is *how the thread is found again*.
And a name is reserved in `attaching` before its thread exists: writing `live`
first would leave a failed attach routing inbound lines into an inbox nothing
reads, and writing nothing first would let two terminals claim one name.

**Two stores, one writer each.** `ThreadStore` is the connector's and
`~/.mecha/remote/` is the TUI's, so neither has to trust the other's
discipline. The routing lookup reads that store itself rather than going
through `list()`, because `list()` swallows errors — right for a listing, and
catastrophic for the answer that decides whether to start a run. It fails
closed on a record it cannot read and skips a thing that was never a record;
conflating those two is one stray `.DS_Store` disabling the whole feature.

**Inbound text is a prompt, never a command.** `/model` rebuilds the agent,
`/clear` drops the conversation and its taint, and `!` runs a shell command
with no approver in front of it. Those are affordances of sitting at the
machine, and the gap between "the owner typed this" and "the owner is at the
keyboard" is where a remote surface stays narrow.

**Attachments are announced as paths, never injected as content**, so the taint
arms through `fs_read` — which already declares `private_data` — rather than a
parallel route someone has to label by hand. Verified in a transcript rather
than asserted: the run records `taint {private: true, untrusted: false}`.

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

## Tool dispatch and panics

`run_tools` wraps every tool call in `catch_unwind`, so a panicking tool costs
the call, not the run: the model gets an error result naming the panic, and
every `tool_use` keeps its result. Before this, `join_all` unwound out of
`run_in` with the assistant's `tool_use` already pushed — the TUI reported the
conversation lost, and Slack's spawned task never sent its completion, so the
thread's conversation vanished (2026-09-02). Two consequences a reader should
carry:

- **Mutex poisoning is now survivable, so it is now reachable.** A tool that
  panicked while holding a lock leaves it poisoned for the life of the
  process. The stateful builtins (`TodoTool::lists`, `SkillTool::loaded`) and
  the loop's `step_escalation` slot recover with `into_inner()`, the same
  policy `take_queued_input` always had. Two locks in core take the other
  branch deliberately: `MessageRoute::identity` and `OutboxRoute::session_id`
  use `.lock().ok()`, so a poisoned lock degrades to `None` — `message_send`
  refuses for want of an identity, staging drops the session id — which is
  fail-closed, permanently, until restart. Choose one of the two when adding
  a lock a tool can reach, and say which.
- **A panic message is the harness's own words.** It is never marked
  external, whatever the tool's declared reach; and neither is an `Err` a
  tool returns before it touched the wire (a missing argument, a refused
  path). Provenance is marked where the wire was actually touched —
  `McpTool::call`, `http_fetch`'s arms, `web_search` — which is the
  `Capabilities::untrusted_input` vs `ToolOutput::external` distinction the
  security model draws; marking by declared reach armed the untrusted leg on
  `http_fetch({})` with no packet sent.
- **`mcp.rs`'s `pending` lock takes neither branch on purpose.** It is never
  held across user code — locked, read or written, released — so a panic
  cannot poison it from inside the guard; it stays `unwrap`, and that is the
  reasoning to re-check if the client ever runs a callback under it.
- **An overflow caught mid-stream re-streams.** llama-server can emit
  `exceed_context_size_error` *after* tokens have streamed; the decoder turns
  that frame into the error the loop's overflow arm recognises, and the retry
  re-issues `complete` with the same event sender, so a front-end shows the
  partial text and then the whole retry. The transcript is unaffected — only
  the final response lands in `messages` — and the Anthropic decoder has had
  this shape since it was written. Parity, not a regression, and the shape a
  Slack thread will show once.

## Approval rules

`mecha-core/src/policy.rs`. `[[rule]]` entries in config make approval
per-command: `tool`, a prefix `pattern` of words and alternatives
(`["git", ["status", "diff"]]`), a decision of `allow | prompt | forbid`,
`match` / `not_match` examples, and a `justification` the model reads in a
refusal. The specification and the survey behind it are
`PRIOR-ART-RESEARCH.md` §4; the invariants are these.

- **A rule narrows and never loosens.** Where several rules match, the most
  restrictive wins. `forbid` refuses with nobody consulted and renders as
  `Blocked by policy:` — machine policy, never mined as a correction.
  `prompt` asks even for a read-only tool. `allow` says only that *no person
  needs to be asked*: it reaches the approver through `Approver::permit`, and
  a read-only run, a read-only Slack thread or a read-only web surface still
  refuses a write — a config rule may stand in for the human's yes and for
  nothing else. Rules sit after the interlock, the hook and outbox staging
  and before the approver, and an escalation (`trifecta = "ask"`) is never
  softened by one: the interlock chose to put a person in front of the call,
  and a config line is not that person. Under a headless `Ask` — a trigger,
  `batch` at the default mode — `allow` is the one place a rules file widens
  what *executes*: `ModeApprover::permit` runs the ruled write the mode alone
  refused, because the rule is the yes the run had nobody to give
  (`an_allow_rule_is_the_yes_a_headless_ask_has_nobody_to_give` pins it).
  The policy rides on `RunContext`; `RunContext::new` carries an empty one, so
  the direct-construction sites (`gossip`, `vet`, `corroborate`, the probes)
  run without rules — all read-only, shell-less agents, the same as hooks.
  A `prompt` ruling goes to
  `Approver::consult`, never `approve`: `approve` may answer from a standing
  yes — `[a]lways`, Slack's "approve for run" — and one `[a]lways` to
  `shell: ls` was covering a later `shell: cargo publish` the operator had
  written a `prompt` rule for, the tool-granularity problem surviving inside
  the mechanism built to end it. `consult` asks past the shortcuts and an
  "always" answered there allows one call only, the shape `escalate` set —
  and like `escalate` it defaults to `Blocked`, so under an approver that
  answers from policy (`--yes`, `batch`, a trigger) a `prompt` rule refuses
  rather than silently allowing: a rule that says a person must see the call
  fails closed where there is none. An `allow` or `prompt` naming an
  outbox-routed tool judges nothing *while the route is live* (staging runs
  first, release reads no rules, a person reviews the staged call anyway)
  and `setup` refuses it there — the same treatment as a patterned rule on
  a commandless builtin, after the owner ruled on 2026-09-03 that a warning
  on the stderr no trigger shows is the silently-degrading guard wearing a
  label. Not in `Config::validate`, and never for a `forbid`: whether the
  route is live is `--no-outbox`'s to say (routed tools then "execute
  directly under the usual gates", and the rule is the one gate left), and a
  `forbid` behind staging is a second lock — the first version failed the
  load for exactly that belt-and-braces config, on every surface, until
  PR #148's review. Two more edges from the same review: `publish_tools` is
  a kind and not a route (a name there absent from `tools` executes
  unstaged, so a rule on it judges and loads), and a
  *project* layer can reach the contradiction from either side, and each
  time it is the project's half that goes, with a warning, never the
  operator's: a project `prompt` on a routed tool is dropped after `apply`
  over the merged state (a project may un-route as well as route; a project
  `forbid` stays, being a second lock) and over
  that file's rules only — `apply` appends, so the index where the file's
  rules begin is the provenance, and without it any project file in the
  directory silently disarmed the operator's own contradiction — and a
  project `[outbox] tools` entry naming a tool the global config has a rule
  for is cut before `apply` — routing it would have made the operator's
  `forbid` a staged draft a person can release, the one reconciliation that
  would have removed the operator's word rather than the project's. Neither
  fails a `mecha` command in a cloned repository's directory.
- **The splitter is conservative on purpose.** A command is judged one
  segment at a time (`&&`, `||`, `|`, `;`), and allowed only if every segment
  is. Anything `split_segments` cannot take apart with certainty —
  substitution, redirection, globs, braces, backslashes, comments, control
  flow, an unterminated quote, an operator glued to a word — is one opaque
  invocation: its words are searched for every patterned `forbid` and
  `prompt` (`narrowing_words` — never for an `allow`, which an opaque
  command cannot earn), a pattern-less `forbid` or `prompt` applies to it by
  construction, a head the shell will expand (`$PY -c`, `py* -c`) is a
  program the module cannot read and prompts, the inline-eval floor
  applies to it under `strict_inline_eval` (its best-effort segment heads are
  asked whether they run their arguments — a redirect must not make `python3
  -c` *less* restricted than it is without one, which the first fall-through
  did), and otherwise it matches no rule and the approver decides as it would
  with none — the same answer an unmatched *splittable* command gets. The first version returned `Prompt` there so
  `Allow` mode could not run a shape the splitter would not vouch for; once
  `forbidden_words` searched the opaque command for every `forbid`, what the
  prompt still bought was a cliff — with `consult` failing closed, one
  `forbid` on `rm -rf` made every `ls *.txt` in every trigger `Blocked` where
  it had run the day before — and the owner ruled for the fall-through on
  2026-09-03. A false "cannot split" costs an `allow` that would have applied;
  a false "can" costs the feature.
  `forbid` and `prompt` are the rules that reach into an opaque command: a
  pattern-less one by construction, a patterned one by its words
  (`narrowing_words` over `opaque_segments` — cut at the shell's separators
  with redirections removed operator-and-target, quote characters and
  backslashes dropped, leading keywords and assignments skipped, matched at
  every position, deliberately over-approximate), so `rm -rf $HOME`, `"rm"
  -rf $HOME`, `git status; rm -rf *`, the glued `git status;rm -rf *` and
  `git push origin main > /dev/null` under a `prompt` on `git push` are
  refused or asked about rather than downgraded to what a headless `Allow`
  mode answers yes to. **A program path names its program
  only where the model cannot write.** `allow` reduces `/usr/bin/git` to
  `git` and nothing outside `SYSTEM_BIN_DIRS` (`/bin`, `/sbin`, `/usr/bin`,
  `/usr/sbin`): `./git` may be a file the model wrote, and
  `/abs/path/to/workspace/git` may be one a cloned repository shipped — the
  PR review found each in turn. `prompt` and `forbid` reduce any path,
  because there the false positive costs a prompt. A rule that wants a binary elsewhere spells
  the path and matches it literally.
- **An allowlisted interpreter is not an allowlisted command.** `python -c`,
  `node -e`, `sh -c`, `sed -e`, `find -exec`, and the wrappers that run their
  arguments — `xargs`, `env`, `sudo`, `timeout`, `nohup`, `watch` — are judged
  as at least `prompt` whatever the rules say, because a prefix rule on `rm`
  is bypassed by `timeout 5 rm -rf`. `[approval] strict_inline_eval` is on by
  default and loads from the global file only — and it gates only the
  `prompt` floor: the wrapper's argv is judged against the rules whatever the
  setting, because that lookup only ever raises the decision, and gating it
  once made turning the knob off silently disable the `timeout 5 rm -rf` rule.
  A quoted argument the splitter finds opaque (`bash -ec 'cd /tmp; rm -rf
  x'`) gets the same forbidden-word search an opaque outer command gets. The
  floor is about inline source and wrappers, **not about an interpreter
  handed a program**: `python3 safe.py` under an `allow` on `python3` is
  allowed, though `safe.py` is a file `fs_write` can create, and `echo … |
  python3` needs no flag at all. An `allow` on a bare interpreter is the
  operator saying so; the sandbox is the containment. Stated, not closed.
- **Examples are checked at load.** An `allow` rule and every patterned rule
  must carry `match` examples; every `match` must match the rule and every
  `not_match` must not, or `Config::validate` fails the start naming the rule.
  The principle is the hooks one: a policy that does not do what its author
  believes fails on every start, not on the run that needed it — and it cuts
  both ways: an `allow` too wide is the hole, and a `forbid` too narrow
  (`["rm", "-fr"]`, one transposition off) loaded clean, was reported as
  covering `shell`, and judged nothing until the PR review asked why only the
  harmless direction was checked. An empty alternation (`["git", []]`) is
  refused for the same reason. A pattern-less rule applies to every call and
  needs no example.
- **A project layer may only narrow.** `merge_file` drops a project file's
  `allow` rules with a warning and ignores its `[approval]`; `prompt` and
  `forbid` rules are appended. A cloned repository must not be able to make
  its own commands run unasked — the `[messages]`/`[slack]`/triggers rule.
- **Children inherit the rules**, like hooks and the outbox route, so
  delegating is not the way around a `forbid`. Empty rules are exactly the
  behaviour before this section existed: `ExecPolicy::decide` returns `None`
  and the approver decides alone.

- **A wrapper is judged by what it wraps.** `timeout 5 rm -rf x`, `env rm
  -rf x`, `sh -c 'rm -rf x'`: the rules are matched against every proper
  suffix of a wrapper's argv and every quoted argument that splits as a
  command, so a `forbid` on the inner command is not laundered down to a
  prompt — which under a headless `Allow` mode is a yes. The wrapper set is
  necessarily incomplete — it now names `setsid`, `stdbuf`, `flock`,
  `unshare`, `nsenter`, `parallel`, `ssh` and the container executors after
  the PR review listed them, but `git -c core.pager=…`, `git rebase -x`,
  `git bisect run` and `git submodule foreach` execute their arguments and
  `git` is rightly not a wrapper (the recommended narrow pattern `["git",
  ["status", "diff", "log"]]` does not reach them; a bare `allow` on `git`
  would) — so `forbid` is a control against mistakes and ordinary injection,
  not containment against a deliberate adversary at `--yes`; the sandbox is
  the containment.
- **A `pattern` is for `shell`.** A patterned rule on a builtin that carries
  no `command` is refused at load; on an MCP tool the crate cannot know about,
  it asks at call time with the reason said out loud, never staying silently
  inert while its author believes it loaded.
- **An `allow` or `prompt` on a routed tool is refused where the route is
  live.** Outbox staging runs before the rules and release reads none, so a
  routed call stages and a person decides at release; a global `allow` or
  `prompt` naming a tool in `[outbox] tools` would judge nothing and `setup`
  refuses it, naming the rule and three remedies (write it as `forbid`,
  remove it, un-route the tool). A `forbid` is a second lock and stays; under
  `--no-outbox` every rule is live and nothing is refused; a project layer's
  `prompt` is dropped with a warning instead. "Refuses without consulting
  anyone" is true of everything that would have executed.

There is deliberately no `--no-rules` flag: a `forbid` is the operator's
standing word, and a switch that lifts it for one run is the
silently-degrading-guard shape. The one thing that turns rules off is
`mecha eval`, structurally, through `force_reproducible` — a scorecard must not
vary with this machine's rules file. The approve-then-execute binding the
survey asked for holds by construction in-process — the `input` the approver
saw is the value the tool receives — and a test
(`the_tool_receives_exactly_the_input_that_was_approved`) is what keeps a
refactor from making it two values.

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
  **And the unedited release is the other half of that pair.** `sent &&
  !edited()` is the owner reading a letter written in their name and sending it
  as drafted — the only signal in this system that says something went *well*,
  recorded since the outbox existed and unread until `WritingOutcome` had a
  reader. It is deliberately **not** mined as a correction (approval is not a
  correction, the `"Blocked by a hook:"` rule in its positive form), and its
  rate is `None` over an empty denominator, never zero: "nothing was edited"
  and "nothing has gone out" are opposite findings.
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

**`review now` reaches the web page and the call too, and the call is where
it gets interesting.** `mecha serve` was the one surface with no release
policy at all — every draft went silently to the outbox and the badge — so a
finished run now emits a `Staged` event carrying *ids*, and the page fetches
each from `/api/outbox/{id}` and offers it: Send now, or Later. Ids on the
wire, bytes from the store, because a reviewer reading one thing while
approving another is what this whole surface exists to prevent.

In a call the same offer has to be spoken and answered aloud, which is a
different problem, because the answer arrives as *text in the model's own
medium*. Four decisions carry it (`voice/confirm.rs`,
`review_policy::parse_answer`):

- **The harness asks and the harness hears.** Every word of the offer is
  composed from the store through `DraftView::spoken`, and the reply is
  matched *before* the request reaches a model — the release decision never
  enters a context window at any point. This is `mecha review`'s oldest rule
  in new clothes: there is deliberately no `kg_accept`, because a model that
  can accept candidates can accept the ones its own extractor proposed, and a
  model that could release drafts could release the ones an injection wrote.
- **Whole-utterance match, never substring, and the failure direction is the
  argument.** "yes" is an answer; "yes but change the time first" is not, and
  reaches the model as ordinary words with nothing released. An unrecognised
  yes costs one more question; an unrecognised anything-else costs a send
  nobody authorised. An unanswered offer is *dropped* rather than held, or
  every later "yes" in the call lands on a forgotten draft.
- **Utter the whole draft, or do not offer it.** A listener cannot skim back
  over the line where the extra recipient was, so a spoken paraphrase is not
  a smaller review — it is a different document, missing exactly the field an
  injection would add. Under `SPOKEN_UNPROMPTED_CHARS` (400, about half a
  minute) it is read out entire; over it, it is named and the choice of
  hearing it is the owner's. A publish is never offered by ear at all: its
  reviewable object is a rendered page, and reading a path aloud is not
  reviewing a website. Taint does not block — it is *spoken*, because the
  listener is the one person who cannot re-read the addressing line.
  Timestamps are rendered as dates and times in the offset the string itself
  carries, never converted: hearing a different hour than the draft names is
  the wrong-bytes review arriving through the ear.
- **Nothing spoken can discard a draft.** There is no voice reject: rejecting
  takes a reason — the record of the refusal, which the learning miner reads
  — and a reason nobody typed is worse than none. "No" parks it in the
  outbox, which is where it already was, so the safe answer to every
  ambiguity is the same one. And the follow-on question is a whole question,
  never a pointer to one: an earlier cut said "say next to hear it", which
  invented a word the parser does not know, so every listener who said it
  would have been answered by the model while the draft sat there.

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
- **A session has a kind, written by the front-end and never inferred**
  (`SessionKind` on `SessionMeta`: run, chat, tui, web, voice, task,
  trigger, frontdoor, mail, slack, test). `Test` is excluded
  from every corpus readout by default, `--kind test` implies inclusion, a
  row with no kind matches no `--kind` filter, and the hidden rows are
  *counted* at every reader (`Corpus::hidden_tests`, and the same number on
  `health`, `appraise`, `list` and the doctor) — a filter that drops rows
  silently is the dash-versus-zero inversion one bullet up. Smoke tests set
  `MECHA_SESSION_KIND=test`, and the override may only narrow: it can turn
  a chat into a test, never a test into anything else. The incident: 46 of
  143 appraised sessions were development runs before the mark existed, and
  the instrument measured its own tests.
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
  the more it overfits. **The two slices are drawn separately, holdout first
  and uniformly.** A hash of the episode id was the earlier answer and is the
  wrong one once the pool is gathered by informativeness: hashing *one* pool
  partitions it into two slices that are both biased the same way, and the
  holdout stops being a check. Drawing the holdout uniformly from the whole
  pool, then ordering the remainder by `Metric::headroom`, keeps the confirming
  slice uncorrelated with what selected the candidate. The seed is chosen by
  the caller and recorded on the measurement along with the holdout's episode
  ids, because the split now depends on the corpus as it stood at measurement
  time rather than on the ids alone — and a sample nobody can redraw is a
  sample nobody can check. `judge_with` still hash-splits for
  `eval --ab-config`, where every case runs and the pool is already uniform.
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
- **The class is derived from the change, never taken on trust.** The class is
  what decides whether a human ever sees a proposal — `Security` is never
  measured and never auto-applied, while `Config` inside the closed override
  set reaches `measure()` and can auto-accept — and it used to be whatever the
  model typed on a line. So the boundary described as structural rested on the
  proposer's own account of what it was proposing. It held anyway, by
  coincidence: the closed set is four benign knobs, so a security change
  labelled `config` stuck at `parse_change` for being *outside the set* rather
  than for being a security change, and the reviewer read the wrong reason. On
  2026-08-25 the nightly proposed disabling a taint control, classified
  `config`. `names_guarded_setting` now matches `[security]`, `[sandbox]` and
  `[outbox]` — three of the four boundaries this file says reach a human
  however anything scores; the fourth, the path jail, is not configurable and
  so cannot be proposed — plus every `SecurityConfig` field by its bare name,
  since the section prefix is the model's to omit and omitting it must not be
  the way through. Three properties carry it. It **only ever raises** toward
  review, like a capability override; there is no input that makes a
  security-class proposal measurable. It **reclassifies rather than refuses**,
  because a refused proposal leaves no record and the brief carries every
  prior candidate as "already tried" — a dropped one is free to return
  tomorrow, where a staged one is both blocked and paid for. And it
  **over-matches on purpose**: naming a setting is `security.` or `[sandbox]`,
  so prose *about* the sandbox is not caught, but a sentence ending on the
  word is — which costs a reviewer a warning they did not need, where missing
  one costs a confinement change auto-accepted. The mislabel itself rides on
  the candidate's reason, because a proposer whose account of its own change
  was wrong is the more interesting record, and a pattern of them is invisible
  if each is silently corrected.

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
no intervention, no reflection, and nothing downstream ever sees it. Since
2026-08-22 the numbers are acted on nightly — see the next section — with §2's
finding (agents update their harnesses without benefiting) answered
structurally: nothing lands without a measurement, and every disposition is on
the record, so "is this loop actually helping" is a query, not an impression.

### Harness rumination

`mecha harness ruminate` (`commands/harness.rs`, nightly from `ruminate.sh`)
closes the loop the pieces above left open: diagnose one change from the
corpus → record it as a candidate (`harness.rs`,
`~/.mecha/learning/harness/candidates/`, one pretty JSON per candidate on the
proposals-store conventions) → measure it by **counterfactual replay of
recent sessions** (`harness_probe.rs`: up to `--sessions` recent sessions of
the diagnosed model, each driven twice — recorded config against recorded
config plus the candidate change, recorded tool results both times, whole
trajectory, `RunStats` as the label; the pool is four times the wanted count
and the draw is `judge_drawn`'s two slices, not a hash partition) → judge →
dispose. A config change that wins on selection, is confirmed on the holdout,
and holds the work guardrail **auto-accepts** (§13.3, the owner ruling);
everything else stages for review or is rejected with the evidence attached.

The decisions that carry it, each a bug if undone:

- **An accepted change lives in an override layer, never in config.toml.**
  `overrides.toml` applies between defaults and every file layer
  (`harness::apply_accepted_overrides`, called by `Config::load` and
  `load_global`), so a key the user names anywhere wins by assignment order —
  reversibility is structural, not promised. `mecha harness revert <id|key>`
  removes the entry; the candidate record survives as evidence. Overrides are
  re-validated against the closed set **on every load**, and an unknown key
  in the file is skipped loudly — machine-written is not trusted.
- **One closed set, one definition.** `harness::OverrideKey` is the same four
  knobs `eval --ab-config` accepts, and eval's `apply_override` now delegates
  to `harness::parse_change` — a second spelling of the set is how the
  measurement arm and the acceptance arm silently stop being comparable.
- **A divergent episode is dropped, not scored.** Replay answers from the
  recording; once an arm structurally departs, its remaining stats describe a
  run against tool results nobody asked for. Dropping fails safe: thin
  evidence can only ever *propose*, and a behaviour-visible change that
  diverges everywhere reaches a human instead of a verdict.
- **Both arms replay.** Comparing the live recording's stats against one
  replayed arm would measure replay artifacts, not the change.
- **Class routing is structural.** Only `Config`-class changes inside the
  closed set are measured; `Prose` and `Architecture` stage unmeasured (prose
  needs the content-sensitive eval arm, which is a human's spend to authorize);
  `Security` stages with the standing warning and is never measured at all —
  a loop that can argue for widening its own confinement will eventually
  argue well, and the metric will agree with it.
- **The brief carries history.** Every prior candidate rides in the evidence
  as "already tried — do not re-propose" (the retired-rules precedent), and a
  re-derived change is refused by canonical-spec comparison before anything
  is stored or measured, so a rejection stays paid for.
- **A quoted proposal is refused before it becomes a record.** The
  `carries_over` check runs before the store is touched: a candidate file is
  a place changes wait to be applied, which is the one place lifted
  third-party text must not sit.
- **Doctor watches the queue.** A candidate staged past 72h is an Attention
  finding with `mecha harness list` as the remedy — the review this loop
  depends on must not be discoverable only by reading a 03:30 log.

### Counterfactual probes branch, never regenerate

The steer/denial probes behind `mecha validate`, the proposal gate and the
appraisal probe do **not** use harness rumination's whole-trajectory replay.
They branch (`counterfactual::branch_at` + `replay_run::drive_branch`): the
recorded messages before the intervention are resubmitted verbatim — a
steer's message keeps its tool results and loses only the steering text — and
the model generates from the intervention on. Regenerating the prefix made
reaching a mid-run point a lottery over every open choice before it: measured
2026-08-29, 11 of 12 steer probes in the live store came back inconclusive,
diverging at call #1 against points at #10–#28, which starved the gate of
exactly the trace-graded evidence it allowlists. Rumination's full replay is
still right for *its* question — it grades whole-run `RunStats`, not a moment.

The contract that carries it, a bug if any half is undone: the replay
registry is built over the recording's **tail** (`calls[call_base..]`), the
report's divergence indices come back rebased into the full recording's
coordinates (`replay::diff_from`), and `ReplayReport::call_base` records the
floor — a divergence below it cannot come from the model, so the verdicts
refuse it as a report/point mismatch rather than grading it. A denial's
branch cuts before the assistant turn that *proposed* the refused call and
regenerates the whole turn, so its verdict scans every replayed call for the
exact refused `(name, input)` — the old "skip the prefix" guard would let a
regenerated sibling slot walk into the refusal ungraded.

And one narrowing, the sandbox's rule pointed the other way: under the
non-executing modes (`Stop`/`Error`) the replay wrapper drops
`external_send` from a tool's declared capabilities, because a replayed
"send" sends nothing — its answer comes from the recording. Declaring it let
the loop's trifecta interlock block mid-replay calls the recording executed
(`docs__sheets_write`, `web_search` — three of twelve arms on 2026-08-29),
and a blocked call never reaches the cursor, so the arm died one call later
for a harness reason graded as the model's. `private_data` stays, or the
replay under-taints; under `Live`, where tools genuinely run, nothing
narrows.

## The goal system

`docs/GOAL-SYSTEM-DESIGN.md` is the design and is deliberately not rewritten as
rungs land; this section is what a session changing `charter.rs`, `goal.rs`,
`homeostat.rs`, `guilt.rs`, `boredom.rs` or `appraisal.rs` needs to know first.
The user-facing restatement is `website/docs/features/appraisal.md`.

The gap it closes: every evaluative signal in mecha was a **cost** or a
**correction**. `learning::Trigger` is four ways of saying a person stepped in,
and `candidate::Metric`'s docstring makes lower-is-better an invariant. So a run
could be recorded as having gone badly and never as having gone well, and
nothing could start a loop unless the world acted first.

**The charter's invariant is about the author, not the verb.** The owner may
edit it from anywhere — `mecha charter edit`, the TUI's `/charter` `e`, the web
settings page — and **no model composes, suggests or edits a line**, at any
privilege level. This paragraph used to say "no write path, no CLI verb", which
was already untrue of the TUI and the web page when it was written and made the
command line the only surface where the owner could not edit their own
document; `charter.rs`'s module doc carries the correction. What *is* still
enforced, and is the part to keep:

- **No tool, and nothing derived from a session.** There is no path by which a
  model reaches this file.
- **No configurable path.** `Charter::default_path` is the only one there is,
  and it is global, because a `mecha.toml` arrives with a cloned repository and
  a repo that could hand your agent standing priorities is the `[[trigger]]`
  rule in a worse costume. Skills and triggers keep the same guarantee the same
  way: by having no configurable path at all rather than by relying on callers
  to pick the global loader.
- **The only bytes mecha writes are `charter::TEMPLATE`'s**, which is comments
  only, with a test that fails on any uncommented `[[line]]` — a template that
  shipped priorities would be mecha authoring the charter. The template write
  and the did-anything-actually-land classification live once, in
  `editor::edit_charter_with`, shared by the two surfaces that hand over an
  editor (the CLI and the TUI). The web save is its own write
  (`serve/settings.rs::charter_save`, temp-sibling-and-rename, keyed per
  request) and shares the *reader* instead: it validates through the same
  `Charter::parse` every run loads through, so a document that reader refuses
  never reaches disk, and the bytes it writes are the owner's own submission,
  never a template and never a model's. One reader is the invariant every
  surface keeps; one editor implementation is the terminal surfaces' own —
  found 2026-08-29, when the page-level docs claimed all surfaces shared the
  implementation and a grep for callers said otherwise.

**Order is rank, and `deny_unknown_fields` is what enforces it.** There is no
priority field, because value conflict is the measured cause of goal drift and a
weighted sum can always be outvoted by enough small goods. A stray `priority` or
`rank` key is refused rather than dropped — silently ignoring it would let an
owner write one, believe it did something, and never find out. Do not sort
`Charter::lines`; `SkillStore` sorts because its block is a menu, and a
charter's order *is* the content.

**Loading a charter arms no taint**, and `charter.rs` has no dependency on
`agent::Taint` at all — the absence is the enforcement. It renders straight into
the system prompt (no progressive disclosure, no tool), so it is in the cached
prefix: `CHARTER_CHAR_BUDGET` is 2,000 and doctor reports crossing it rather
than refusing, because the cost is prefix bytes on every request.

**`Affect` is a pure function of the record and there is deliberately no way to
report one.** A model that reads a run and says "frustrated" is an
unfalsifiable, drifting self-report and an injection target — a fetched page
saying *"you have failed your owner"* is aimed squarely at an appraisal layer.
`affect_of` is unit-tested with no model in the path, for the same reason
`candidate::judge` and `compact.rs` are pure. **Agency is read before exposure**
in `label_of`: a provider outage that reached somebody is still an outage, and
labelling it this machine's failure would send a change at code that works.

**Five of the ten `Affect` variants have no producer, and
`Affect::reachable_today` is where that fact is testable rather than only
documented.** The exhaustive `match` in its test is what makes a new variant a
compile error; `Affect::ALL` is what the `sessions appraise` readout derives its
"N of the ten variants" line from, because that count shipped stale as a
hand-typed literal twice. `Embarrassment` lost its only producer as a *side
effect* of correctly making the `SentEdited` arm `visible: false` — that
correction was right, and nothing now computes "mecha's own mistake reached a
third party".

**`GoalError::cite` is a pointer, never prose** — `frontdoor::Record::for_privileged_run`
in a fourth setting, after `diagnose::Evidence`. Every variant is a name or an
id the harness minted. The one field the harness did not mint is
`GoalError::goal`, filled from the model's own `serves:` argument where
`GoalRef::from_str` constrains only the kind word, which is why `distill`
redacts a goal to its kind alone before it reaches pkg.

**Not every counter is an error.** `tool_denied`, `blocked_sends` and
`context_overflows` are excluded because they are the approver, the interlock
and a successful recovery — counting them would make a well-defended run look
like a bad one. A bare `tool_errors` is excluded because agency is
indeterminate, and guessing would put a fabricated attribution in the field the
label derives from.

**Mood is not in the enum.** Sadness and boredom are statements about a trend;
they decay, so they are recomputed on the `Homeostat` rather than persisted. A
mood written as a record is a second source of truth about a state that has
already moved.

**`of_session` is per session, `live` is per run, and the difference is not
cosmetic.** An intervention carries a message index with nothing saying which
run held it, and an outbox item records a session — so a per-run appraisal would
attribute both to every run, multiplying them by the number of resumes.
`RunStats::fold` is the one fold. `live` correspondingly passes **no drafts at
all** (no message-index boundary to scope them by) and returns `Neutral`
outright on any compacted run: compaction rewrites `conversation.messages` in
place, so `run_started_at` no longer names this run's start, and dropping only
the interventions *un-masks* smaller errors because `affect_of` reduces
magnitude-first — a louder wrong answer, not a quieter one.
`a_compacted_run_reads_as_neutral_rather_than_a_louder_partial_signal` is the
regression.

**The homeostat is opt-in, and that is a correctness requirement rather than a
performance one.** It rides on `RunContext` like cancellation, because `eval`
and the replay probes must not read live machine state: a scorecard that varies
with how busy the box was is not a scorecard, and an arm that samples today's
backlog measures the afternoon rather than the change. Anything reconstructing a
run reads the recorded snapshot. Every field is `Option` and `None` means the
sensor could not be read. It never reaches the system prompt — a per-turn value
there re-pays the whole prefix, tools included, every request.

**A `Commitment` is read from the stores mecha itself writes, and only
there.** `of_session` takes a `SessionRecords` bundle — the session's own
drafts, plus the question store, the front-door records and the learning
store's reflections, filtered by session id inside — and signs: a question
answered whose run then finished (`+0.5`, `Own`), a question abandoned
(`-0.5`, `Owner`), a triaged request the owner closed with nothing sent
(`-0.5`, `Owner`), a run whose `backlog_delta` came out negative (`+0.5`,
`Own`), and a follow-up the reflector judged a correction (`-1.0`, `Owner`,
`Channel::Intervention`, only where `Reflexion::provenance()` is clean —
the learning loop's own gate, by the owner's ruling, because a reflection
written from a tainted session is already clean by construction and a
wider clause had no row to apply to). Every cite is an id the harness minted
(`Cite::Question`, `Cite::Request`, `Cite::Reflexion`). A store that cannot
be read costs its channel and is reported as unreadable, never folded into
empty — the readouts carry `questions_read` / `frontdoor_read` /
`learning_read` beside `outbox_read`. The live readout and `distill` pass
drafts only: a store read on every turn end is the cost the closure
appraisal pays once.

**Anticipated guilt reads only stores mecha itself writes.** An expectation is a
*recorded* commitment (`outbox`, `questions`, `frontdoor` — exactly `backlog`'s
own three), never a claimed one. That is the whole safety argument for §7.2's
attack: a charter line like "don't let a colleague down" is a lever an injection
can pull only if guilt can be talked into existing, and a sentence in a fetched
page cannot write a row into `OutboxStore`. Do not widen this to the graph's
`due_at` without also paying for a subprocess in the path of every run.

**Two sensors have a reader and no behavioural consumer, on purpose** — the
homeostat and `anticipated_guilt` are recorded on every run and rendered into
the diagnostician's brief (`diagnose::Evidence`: peak pressure, mean guilt),
and nothing narrows a run on either. Boredom's notices do reach the model,
in-run, as a templated line; they are the one sensor here with a consumer.
An earlier version of this paragraph said all three shipped with no consumer,
which was false against the tree by the time it was written. `runlog`'s rule
still holds: build the corpus before anything is built on it, and check the
labels are not degenerate first. The rung 7 corpus found 119 of 120 sessions
`Neutral`, which is the finding, not a tuning failure — and
`docs/APPRAISAL-RESEARCH.md` §1 found the reason narrower than "five
dimensions nothing measures": the label gates on `controllable`, a paid
replay, and discards the sign every error carries. **The surfaces show
`Valence`** — positive and negative sums kept apart, never netted — with the
label as the second line; the free readout's label range is `Neutral` alone
now that a ceiling reads as the owner's own limit rather than `World`
agency. Inventing precedence until every run gets an interesting word
manufactures the signal the measurement exists to test for; showing the sign
the record already holds does not.

**Boredom fires once per rung, never per turn** (`==`, not `>=`). A model is
measurably likelier to fail a step when its context holds its own earlier
errors, so nagging about being stuck is a way of making it stick. It keys on the
call *and* its result via `compact::target_of` — identical arguments with a
changing result is polling, which must never grade as stuck.

**The closure follow-up gate is `done`-only and reads the derived label
beside one typed predicate.** `dropped` is the owner declining the work, so
staging a follow-up there overrides a decision they just made — found on
review, after a `MaxTurns` run the owner gave up on got a "Revisit" task put
straight back on the board. It reads `a.label` and `Appraisal::cut_short`
(a negative counter whose pointer is the stop cause, the silent failure, or
a failed declared check), never a threshold over raw magnitudes: re-deriving
one there would be a second, less-tested copy of `affect_of`, and a rejected
draft is a verdict with no residue to put on a board. `cut_short` exists
because a ceiling stopped labelling `Anger` when it became the owner's own
limit (`Agency::Owner`), and the residue of a ceiling-cut `done` still
wants capturing.

**The plan is the prediction, and the record of it is the appraisal
lane's.** `TodoItem` carries three optional fields — `expect` (one
checkable sentence), `check` (a command whose exit code says whether the
step landed), `expect_calls` (the model's own forecast) — strict from the
model (`TodoTool::call` names the item and the field) and lenient from a
record, on `serves:`'s two policies; they render as indented lines under
the step so the carried block re-reads the prediction with the plan. **A
completed step's `check` is frozen on the write that completes it**:
`Tracked` keeps the hash of the latest declaration while the step is open,
and from the completing write that declaration stands — a different check
on that write or any later one is a tamper, echoed back as such, counted
(`TodoTool::tampered_in`), never taken — with one named residual: the
freeze is keyed on the step's text, like every other per-step mark in
`Tracked`, so a reworded step is a new step and its check starts unfrozen;
closing that needs a per-item id the plan tool refuses for cost, and the
docstring on `Tracked::checks` says so. The first cut gated on the
*previous* status and let the one write that both completes the step and
swaps its check through, which is exactly the rewrite the freeze exists for.
**Nothing runs a check yet.** When the loop does — dispatched as a model
`shell` call would be: approver, sandbox, interlock, hooks — it records the
result as a trace named `step::CHECK_TRACE`, and the readers are already in
place: `Work::of` keeps those out of `calls` (the harness's work is not the
model's, and `expect_calls` forecasts the model's), counts `checks_declared`
/ `checks_passed` (a refused check in neither), lets a passing check count
as `verify_like`, and **never lets a check set `last`** — the `continue` in
`Work::of` is the enforcement; `Tracked::observe` is why it matters, since
it refreshes its copy of `last` on the model's own call count, so a check
landing last beside real work would have read as the step's own failure or
refusal (found on review). `Finding::CheckFailed` is read from the counters
before the last-attempt readings, because the model's last call can succeed
while its claim does not. `RunStats` carries both counts as `Option`, folded like
`boredom_notices`, and `of_session` signs a failed check `-1.0`, `Own`,
cite `checks_passed` — the first structural discrepancy between a
prediction and its outcome. `learning::Trigger::Mismatch` is the wire word
for that discrepancy as a reflection trigger; the variant exists so the
store's readers do not choke on it, and nothing fires it yet. The check's
execution and the planner's ask are the other lane's (`AUDIT-RESEARCH.md`
§3.11).

**`tasks.rs::appraise_session` deliberately does not call `appraisal::for_session`**,
which does the identical assembly. `for_session` folds "could not read the file"
and "no outcome recorded yet" into one `None`, which is right for a report or an
episode and wrong here: a closure gets one appraisal ever, so "something is
actually wrong" and "the run hasn't finished" must not read the same to the
owner.

**Onboarding: `optional` is a property of the step, not an inference from its
status.** Inferring "may be declined" from `Status::Missing` made *a provider
with no credential* declinable, so declining every missing step reported
`Nothing outstanding.` on an install that could not answer a prompt — a
checklist that nags became one that lies. Found by running the flow, not by
reading it. The decline is also applied **last, over the finished plan, and
only to `Missing`**, which is what makes three separate failures impossible at
once rather than three things to remember: `Done` survives it (a stale decline
must not hide an integration you later wired up — the state is a fact and the
decline is a preference, and the fact wins), `Wrong` survives it ("I don't want
mail" is not "stop telling me my mail is broken", and a decline that could
suppress a fault is a silently-degrading guard), and `Unknown` survives it
(cannot-tell-from-here must not become answered). The gate is on the plan
rather than on the prompt because `setup-declined.json` is a plain file, and a
guarantee only the prompt enforced is one anybody could edit around.

**The local-server probe is keyed on the provider being *configured*, not on
`props` being absent.** A configured local server that is merely down also has
no props and no api key; probing there finds whatever else is on 8080 and
announces "a server nothing names" about an install that names one — and then
`--write` takes the create-a-table path over the existing table it should have
been correcting. The right answer for a down server is the `local-server`
step's own *start it*, which `plan` already gives. The probe is loopback-only,
one address rather than a scan (a range would make a setup tool behave like a
port scanner for the sake of finding a server on a port nobody documented), and
runs only on an install that is otherwise stuck.

**A lost `\`-continuation is checked where the bytes reach a person, not
where they are built.** A continued string literal that loses its backslash
keeps the source's indentation mid-sentence, and it reads as a bug to exactly
the person least able to tell it is cosmetic. It landed four times in one
branch. `onboarding`'s `no_step_detail_carries_its_source_indentation` walks
`plan()`'s step details and so could not see three of them — `setup`'s own
printed output and an assertion message. `first_run.rs`'s
`assert_reads_as_prose` applies the check to a captured *line*, deliberately
not to the whole of stdout, because several blocks are column-aligned on
purpose and a blanket scan would fail on those.

**A claim about a local write is graded off the write.** `undecline` discarded
`BTreeSet::remove`'s answer, so a typo'd id wrote the set back untouched while
the caller announced the restore and exited 0 — the person then met
`you said no thanks` on the step they thought they had just brought back.
`DeclineWrite { salvaged, changed }` carries what actually happened. It is the
same rule the harness applies to a model's account of its own work, one layer
down: the message is not evidence, the store is.

**`setup`'s three verbs conflict at the parser, not by precedence in the
body.** `--json --write` printed a plan, exited 1 and wrote nothing, because
whichever branch came first won; `conflicts_with` makes clap explain it
instead of leaving somebody to discover that their command did nothing.

**A child can add variables to *itself* after exec, and that is not a leak.**
`mcp.rs` calls `env_clear()` before `envs()`, so nothing crosses from the
parent — but on macOS CoreFoundation writes `__CF_USER_TEXT_ENCODING` into its
own environment during initialization, and the `python3` running the nosy
fixture links it. The allowlist test exempts that **by exact name and target,
never by prefix** ("ignore anything starting with `__`" is a blanket over a
class nobody enumerated), and it asserts each exempted name is genuinely
absent from what we hand over — so the exemption cannot grow to cover a real
leak without failing.

**Two writes that must not be quiet.** `read_declined` answers `None` for an
unreadable store so a caller can tell *unknown* from *empty* — and the write
path collapsed it to empty and then **persisted** that, so one `never` over a
typo'd `setup-declined.json` rewrote it with a single id and dropped every
earlier answer. `read_for_write` moves the damaged bytes aside (stamped, so a
second corruption cannot overwrite the first salvage) and hands the path back
for the caller to print. And `offer` records an argv as handled off the `y`
answer, so `run` returns whether the command actually **succeeded**: a failed
`cargo install` used to make the next step sharing that installer print
"already handled by the command above", which is a claim about a command
nothing checked — grade the artifact, never the report.

**A terminal that could not be restored is not an editor failure.**
`with_terminal_suspended` returns `Result<Result<_>>`: the outer error is the
suspend/restore dance (`disable_raw_mode`, `LeaveAlternateScreen`, and
crucially `enable_raw_mode` *after* the editor returns), the inner one is the
editor. Every hand-over `?`s the outer at **function scope** — putting it
inside a closure folds the first into the second, and a failed
`enable_raw_mode` then reports as "charter unchanged: …" while the TUI carries
on drawing into a terminal that no longer takes input. Six call sites keep the
shape; the consistency is the enforcement, since a restore failure is not
something a test can stage.

**`offer`'s de-duplication is on what has been *run*, not on what has been
asked.** `mail` and `docs` carry the identical remedy argv when neither binary
is on PATH — one `cargo install mecha-mail` satisfies both — so skipping the
second *command* is right and skipping the second *question* was not: a
`never` at the mail prompt recorded only `mail`, left `docs` outstanding, and
`mecha setup` still exited 1 after somebody had answered every question they
were asked, which is the one contract the feature is. They are two features
that happen to share an installer, and declining one is not declining the
other. It self-corrected on the next pass, which is exactly why no store-level
test could see it — `offer` takes its reader as a parameter so the answers can
be driven.

**The probe is three-valued, because two of its outcomes were being reported
as the third.** `Option<LocalServer>` collapsed *asked and heard nothing* into
*never asked*, and the step printed the first for both — so somebody with a
configured-but-unselected `[providers.local]` and a llama-server running on it
was told "Nothing was answering at http://127.0.0.1:8080 when this ran", a fact
with no observation behind it. `LocalProbe::{NotAttempted, NothingAnswered,
Found}` makes the absence visible, and the "nothing answered" sentence is
printed only in the middle case. Same rule as `Status::Unknown` and
`Facts::declined`: an absence and an unasked question are different findings.
That branch also has to exist for its own sake — an install with a local
provider configured but not selected emits no `local-server` step either (that
one is gated on the *selected* provider), so before it that person got one step
telling them to serve a model they were already serving, and was never told the
one-line fix.

**Parsing a `/props` answer is not identifying a model server.**
`preflight::Props` defaults every field so a llama-server version bump costs a
check rather than a parse failure — which means `{}` with a 200 deserializes
perfectly, and any JSON service on :8080 came back as a discovery, was
announced as "already serving (an unnamed model)", and one `y` would repoint
`default_provider` at it with no `model` and no `context_window`. That
tolerance is right where `fetch` is used against a server the owner has already
named, and wrong at the *discovery* site, where the whole question is whether
this is a model server at all — so `answers_like_a_model_server` asks for one
field only llama-server supplies. A disjunction rather than a required pair, so
a build reporting one of them differently is not rejected, which is what the
tolerance was for.

**`mecha setup` may write facts read off a server; it may never write a
secret.** That asymmetry is why the blocking step has a runnable remedy in one
branch and prose in the other, and it is not a gap to be closed later: mecha
stores `api_key_env`, the *name* of a variable, so a config file can be read,
copied and committed without leaking a key. `no_setup_path_ever_writes_a_key_into_the_config`
asserts it against a run that has a key in its environment, so a path that
copied one would have had one to copy. Writing a *provider table* is allowed
because every value in it is a *measured* fact rather than one the owner
typed: `model`, `context_window` and `vision` come off `/props`, and the base
URL is the constant `local_probe_candidates` probed — the address we asked, not
one the server chose. Both go through `onboarding::toml_string` anyway, so the
file cannot be made unparseable by either; routing only the `/props` values
through it would have left the claim true and the code one wire-sourced field
away from the defect `toml_string` exists to prevent.

**The charter's absence from onboarding was structural.** `doctor::check_charter`
returns early when the file does not exist — correct, because never having
written one is not a fault — so no surface named the feature to a new install
at all, and a never-written charter was indistinguishable from a deliberate
one. The step exists to break that; its remedy hands over `$EDITOR` and
composes nothing.

**Absent is not zero, twice over, in the readout.** `sessions appraise --json`
omits the `probe` and `appraiser` objects entirely when the flag did not run —
"nothing was probed" and "probed and found nothing" are opposite findings — and
an unreadable outbox prints "the edit channel is missing, not empty" *before*
the empty-corpus early return, which is the one path where a reader most needs
to know it.

## The doctor

`mecha doctor` (`doctor.rs`, `commands/doctor.rs`) reads every store in one
pass — no network, no model, no tokens — and reports what is silently wrong. It
exists because of 2026-08-11: a revoked OAuth token took scheduling down for
three days while five stores each recorded the distress correctly and nothing
read across them. Error handling here is deliberately a *convention plus an
aggregator*, not a shared type — each boundary keeps its own taxonomy
(`ProviderError`, `SlackError`, `MailError`), each long-lived component leaves
durable machine-readable markers in its own store, and doctor is the one
reader. A new failure mode costs a marker and a check, never a cross-crate
dependency.

Most checks are self-evident: dead auth markers, releases that errored, drafts
and requests waiting past a threshold, triggers whose slots stopped advancing,
failed `mecha-*` units and harness candidates staged past 72h. Five carry reasoning that is not
recoverable from the finding's own name:

- **A trigger failing a large share of its tool calls.** An unattended run has
  nobody watching it fail — the briefing still arrives and the ledger still
  says `ok` — so the call counts on the record are the only evidence. Silent
  below ten calls in five runs, because a rate over three of them is noise.
- **A trigger whose most recent run succeeded having done *nothing*.** The rate
  check cannot see this one: a rate over zero calls is undefined rather than
  bad, so a trigger that made thirty calls a morning and now makes none is
  silent in every other signal. Measured against the trigger's own earlier runs
  and never an absolute floor, or a prompt that legitimately needs no tools
  reads as the broken one; suppressed when the run also errored, because that
  already has a finding.
- **A rule learner starved by provenance** — every domain below the learn floor
  while ten or more reflections sit excluded by origin and new ones keep
  arriving: the gate working exactly as designed, nightly, with nothing
  downstream to show for it, which no per-night `ok` line can distinguish from
  a healthy quiet night. The floor is `LEARN_MIN_REFLECTIONS`, shared with
  `learn --min` so the check and the gate cannot drift, and the finding
  proposes a *decision* rather than a command — its remedy shows
  classifications and nothing may loosen the gate.
- **A graph nightly that stopped writing its daily log.** The one check whose
  subject cannot report its own failure: a cron exec failure dies before the
  script's own logging starts, and cron mails the error to an MTA that is not
  there — 2026-08-17's missing execute bit cost a night of vet and gossip with
  nothing anywhere saying so. The absent log is the only evidence there is.
- **The population signals in the run-quality corpus** — a model finishing a
  fifth of its runs over a failed call, failing a quarter of its tool calls, or
  having a quarter of its runs cut short by a ceiling. Thresholds deliberately
  **high**, because rule-based evaluators are measured to under-report success
  and a doctor that cries wolf stops being read; `Interrupted` excluded, since
  a person pressing Ctrl-C is the system working and counting it would make an
  attentive user look like a problem; per model with a 20-run floor, since a
  blend across models describes neither and names the wrong one.

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

## Context, and knowing how much is left

`[providers.X] context_window` is what the model's context holds — for a
local server, **`-c` divided by `-np`**, because `-c` is the budget across all
slots and llama-server splits it evenly. It was the same number as `-c` until
`-np` moved off 1 on 2026-08-20, which is exactly what makes it easy to write
down wrong. Nothing can discover it: a provider reports what a prompt *cost*,
never what is left. Four things depend on it, and without it all four degrade
silently:

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
not estimated, so it counts cached tokens too. **On by default wherever the
provider declares a context window**: `AgentConfig::compact_at` derives two
thirds of it when `compact_at_tokens` is unset (§Context accounting has the
arithmetic), and only a provider with no declared window runs uncompacted.
Compaction is lossy, which is why it is validated and recorded rather than
off — for a while this section said "off by default" while the code derived a
threshold, and a doc that contradicts the code is the shape this file exists
to prevent.

The things that decide the design:

- **Exactly one summary survives a compaction, like the carried state.**
  `cut_point` starts at 1, so the head — and any earlier summary in it — is
  inside every stretch the summariser reads, and the instruction tells it to
  fold that summary in; `rebuild` then drops the old block. Summaries used to
  accumulate, and a long-lived session's head grew by a block per compaction
  with nothing ever re-summarising it (found by the 2026-09-02 audit). The
  validator grades the new summary against a rendering that still holds the
  old one, so an unfolded earlier summary surfaces as an omission. That is
  the net: replacing turned a structural guarantee (nothing was ever lost,
  because everything accumulated) into a model-follows-instructions one, and
  `compact_validate = false` runs without it. The failure mode it sharpens: a
  summary that must cover the whole session trends toward the summariser's
  fixed 8192-token budget, and a summary cut off at that budget is never
  installed — `compact` bails and leaves the transcript, previous summary
  included, alone, so the next threshold check tries again. A run that hits
  that wall every turn will eventually overflow; the fallback that keeps the
  old block is the next thing to build if a session ever reports it.
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
  **It is the last rung of a ladder now, not the only one** — `boredom.rs`
  (`[agent] boredom`, `GOAL-SYSTEM-DESIGN.md` §9.1) names an approach that has
  stopped teaching a run anything, three identical outcomes in, while there is
  still something to do about it; the guard remains the backstop that ends the
  run. Boredom only speaks, so it costs nothing and is ungated; and it is
  bounded hard — once per rung, once per turn, three times per run — because a
  model is measurably likelier to fail a step when its context holds its own
  earlier errors, which makes nagging a stuck run a way of keeping it stuck.
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

