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
- **A judge needs room to think before it answers.** At `max_tokens: 512` the
  judge spent the entire budget on reasoning and returned empty content with
  `finish_reason: length`. It is 4096 now, and an unparseable verdict reports
  the stop reason rather than just the empty string.
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

- **Never believe `finish_reason`.** llama-server reports `stop` alongside
  `tool_calls`. The loop believed it, dropped the calls, ended the run and
  returned an empty string — which graded as a model failure and was a harness
  failure. Any turn containing tool_use blocks is now a tool turn regardless.
  Assume the same class of bug exists for other local servers.

### Unattended runs

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

### Environment

- **`pkill -f llama-server` kills your own shell**, because the pattern matches
  the command line running it. Use `pkill -x llama-server`.
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
