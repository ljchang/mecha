# Three harnesses, read against this one

Research pass, 2026-08-05. The question was what **openclaw**, **codex** and
**hermes-agent** do that mecha does not, and which of it is worth building.

What was actually read, so the confidence level is legible:

| Project | What was read | What was not |
|---|---|---|
| [openclaw](https://github.com/openclaw/openclaw) | `docs/concepts/` (agent-loop, queue-steering, compaction, session-pruning, model-failover, retry, context-engine, memory-architecture, delegate-architecture), `docs/tools/` (loop-detection, tool-search, code-mode, exec-approvals, trajectory, self-learning), `docs/security/THREAT-MODEL-ATLAS.md` | the source |
| [codex](https://github.com/openai/codex) | `docs/` (mostly stubs) and the pages they redirect to on `learn.chatgpt.com` — sandboxing, execution-policy rules | the `codex-rs` source, beyond the crate list |
| [hermes-agent](https://github.com/nousresearch/hermes-agent) | README, `SECURITY.md`, `docs/micro-compaction.md`, `docs/session-lifecycle.md`, `docs/security/network-egress-isolation.md` | the Python source |

**Docs, not code.** Every claim below is a claim about what these projects say
they do. Where a claim is load-bearing for a spec, the spec says so and the
spec's tests establish mecha's behaviour independently. That distinction cost
this project real time before (~480 lines of Anthropic provider written to
spec with no evidence behind them), and the same discount applies here.

---

## The short answer

| Idea | Source | mecha today | Verdict |
|---|---|---|---|
| Provenance gates promotion into persistent memory | openclaw | taint exists, dies with the conversation | **build** — §1 |
| Provider error taxonomy, retry, model fallback | all three | **nothing**; any non-2xx bails | **build** — §2 |
| Post-compaction loop guard | openclaw | turn budget only | **build** — §3 |
| Per-command approval policy | codex, openclaw | approver ignores the input entirely | **build** — §4 |
| Tool-result pruning on its own cadence | openclaw | thinning exists, but only inside compaction | **build small** — §5 |
| Pre-compaction memory flush | openclaw | none | consider |
| Queue modes beyond steer/cancel | openclaw | steer (TUI only) + cancel | consider |
| Trajectory bundle export | openclaw, hermes | session JSONL; replay is stronger | consider |
| Directory-mode tool search | openclaw | in the backlog already | consider |
| Landlock backend | codex | already the recommendation in `SANDBOX-RESEARCH.md` | queued there |
| Micro-compaction every turn | hermes | — | **reject**, §6 |
| Channels, gateways, plugin marketplaces | all three | — | **reject**, §6 |
| Model-reviewed approvals | codex, openclaw | — | **reject**, §6 |

Two corrections to first impressions, both from reading mecha's source rather
than its docs:

- **mecha does thin tool results already.** `compact::thin_old_results` and
  `evict_superseded_results` exist and are unit-tested. What it lacks is what
  §5 is about: they run *only* when the compaction threshold trips
  (`agent.rs:706`) or during overflow recovery (`agent.rs:836`), never on their
  own cheaper cadence.
- **mecha's replay/counterfactual machinery is ahead of all three.** None of
  them has anything like `replay_run.rs` driving a recorded prefix, or
  `counterfactual.rs` treating an intervention as a test case. Do not import a
  weaker version of that under the name "trajectories".

---

## 1. Provenance on reflections — the one real hole ~~— built 2026-08-05~~

> **Built 2026-08-05**, to this spec: `Origin` on `Reflexion` (old lines load
> `Untrusted`), `Intervention.at` + `Session::taint_timeline` for
> position-aware classification (checkpoints are written after the run they
> describe, so coverage over-taints within a run, never under-taints),
> `learn` excludes structurally with a printed count, outbox `Edit`
> reflections classify from the item's taint snapshot. No knob, as specified.
> Verified live: a real untrusted session's followup classified `(untrusted)`,
> flipped to `(clean)` when its taint records were flipped, and a learn pass
> over a clean + hostile + pre-origin trio absorbed exactly the clean one.
> Deviations: `Derived` exists in the schema but nothing classifies to it yet
> (subagent/eval/batch conversations do not record sessions today), and
> recall-loop prevention (item 5) needed nothing — the rules block rides in
> the system prompt, which extraction never mines.

**The finding.** openclaw's `concepts/memory-architecture.md` is the best
document in any of these repos, and its central rule is one mecha half-has:

> Origin labels live in SQLite columns written by classification code, never
> parsed out of memory text. Prose claiming to be from the owner does not make
> it owner content.

Content whose origin is `untrusted` is **excluded before any prompt is built**
— a precondition, not a score penalty. "No amount of recall frequency promotes
untrusted content into the curated core."

**Why it matters here.** mecha has the unforgeable primitive already:
`Taint { private, untrusted }` (`agent.rs:308`), set from
`ToolOutput::external` rather than from anything the model writes. And it
already applies exactly this pattern once — an outbox item snapshots the
conversation's taint, and `send` warns when the snapshot was armed.

`learning.rs` does not. `mecha reflect` mines any session's interventions into
reflections, `mecha learn` consolidates them into rules, and
`setup::prepare` appends those rules to the **system prompt of every future
run**. Taint dies with the conversation, so nothing along that path knows the
session that produced a lesson had a hostile web page in it.

That is a persistent prompt-injection path with a longer half-life than
anything the interlock guards. The interlock stops exfiltration *in the tainted
conversation*. A learned rule outlives the conversation, outlives compaction,
and is injected inside the cached prefix of every subsequent run — including
runs with no taint at all, where nothing will ever check it again. The attack
does not even need the model to be tricked into a tool call: it needs one
sentence that survives into a lesson.

This is also the exact shape of the failure this project already hit once and
recorded: the first learned rule was reverted because it came from a poisoned
reflection. That one was poisoned by a bad reflector inference rather than by
an attacker, but the machinery that let it reach the prompt is the same
machinery.

### Spec

**Carry the taint, then gate on it.**

1. `Reflection` gains `origin: Origin` — a closed set, serialised in
   `reflections.jsonl`:

   ```rust
   pub enum Origin {
       /// The session held no untrusted content when the intervention happened.
       Clean,
       /// Third-party content had entered the conversation. Never promoted.
       Untrusted,
       /// Not an interactive session: subagent, eval, batch item.
       Derived,
   }
   ```

2. **The source is the transcript, not the reflector.** `session.rs` already
   records taint and merges records on load, so extraction can read the taint
   as of the intervention's position without new plumbing and without asking a
   model anything. Classification is deterministic code; if the position cannot
   be established, the answer is `Untrusted`, never `Clean`.

3. **`mecha learn` excludes non-`Clean` reflections structurally** — filtered
   before the consolidation prompt is built, not scored inside it. They stay in
   `reflections.jsonl` as evidence and remain readable by a human; they are
   simply never candidates.

4. **`Derived` covers openclaw's session-kind gating.** Subagent, eval and
   batch conversations get a fresh `Conversation` by design, which is what
   makes them cheap to classify. A subagent's steer is not the user correcting
   mecha; it is mecha correcting itself, and learning from it is a feedback
   loop, not a lesson.

5. **Recall-loop prevention.** A reflection whose evidence is the injected
   rules block is a rule re-learning itself. `setup::prepare` already renders
   that block into `RunConfig`; extraction should skip interventions whose
   context is only that block — the same treatment
   `agent::FINAL_ANSWER_NUDGE` and recorded slash commands already get, and for
   the identical reason (mecha must not learn from its own voice).

**Config:** none. There is no knob that should turn this off; a switch that
lets untrusted content into the prompt is the silently-degrading-sandbox shape.

**Tests that must fail on the old behaviour:**

- A session whose recorded taint has `untrusted: true` at the intervention
  produces a reflection with `origin: Untrusted`, and a `learn` pass over it
  writes **no rule** — asserted on the resulting `learned.toml`, not on a log
  line.
- The same session with `untrusted: false` produces a rule. Without this the
  first test passes vacuously, which is the negative-control rule from
  `CLAUDE.md`'s testing section.
- A subagent-origin session classifies `Derived`.
- Round-trip: a reflection's origin survives `reflections.jsonl` and back.

**Eval hook:** nothing needed. This is deterministic and belongs in unit tests.

**Cost:** small. One field, one filter, one classification function.

---

## 2. A provider error taxonomy, and retry ~~— built 2026-08-05~~

> **Built 2026-08-05**, close to this spec: `provider/retry.rs` (taxonomy,
> per-class policy, capped `Retry-After`, shared `send_with_retry`), both
> backends wired, `Failover` in `provider/mod.rs`, config fields on
> `ProviderConfig`, eval forces `--no-fallback`. The turn-level retry layer
> was **not** built separately: retries live at the request level, before
> the response body is consumed, so the never-after-a-tool-ran invariant
> holds by construction — pinned by a mock-HTTP loop test in which turn 2's
> 429 is retried and the tool executes exactly once. Mid-stream failures
> carry no `ProviderError` marker, which is what keeps them out of both
> retry and failover. Deviation: per-turn provider recording in `RunConfig`
> is deferred — fallback logs loudly instead; replay of a fallen-back
> session is a known approximation. Verified live: a dead primary retried
> (transport, 2.5s backoff), fell back to the healthy server, and answered;
> `--no-fallback` failed strictly. The motivating failure was reproduced
> twice on cue during the spillover work — llama-server closing an idle
> pooled connection, reqwest's write dying mid-send.

**The finding.** All three have this; openclaw's `concepts/model-failover.md`
is the most worked-out treatment of provider failure I have seen anywhere.
mecha has **none of it**: `provider/anthropic.rs:198` and `:213` bail on any
non-2xx. A 429, a 529 overload, or a transient 500 kills the run — and in
`batch` or `eval`, kills it in the middle of a fan-out that has already spent
real time.

Three ideas from openclaw are worth taking, and one is worth taking verbatim:

- **Full-turn retry is legal only before tool execution or assistant output
  has started.** After that, retrying duplicates mutations. That is a mecha-
  shaped invariant — the same reasoning as "tools are never interrupted
  mid-call" — and it is the load-bearing one.
- **Classify, then apply a policy per class.** mecha already does this exactly
  once, by hand: `is_context_overflow` recognises the refusal across backends
  by message text because no backend gives it a usable code. The taxonomy is
  that function generalised, not a new concept. Note openclaw routes context
  overflow *away* from failover deliberately, so it stays in the compaction
  path — which is mecha's current behaviour, independently arrived at.
- **Explicit selection is strict; only configured defaults may fall back.**
  openclaw's reason is user surprise. mecha's reason is stronger: a scorecard
  produced by a different model than the one named measures nothing, and
  `--compare` across that boundary is worse than no comparison.

### Spec

**A. The taxonomy.** In `provider/`, shared by both backends:

```rust
pub enum ProviderError {
    RateLimit { retry_after: Option<Duration> },
    Overloaded,
    ServerError,          // 5xx that isn't overload
    Auth,                 // 401/403 — terminal, never retried
    Billing,              // credit exhausted — terminal
    ContextOverflow,      // stays in the compaction path, never retried here
    Invalid(String),      // 400 — retrying the same payload fails the same way
    Transport,            // connect/read timeouts, stream aborts
}
```

Classification is by status **and** by message text, because the text is
sometimes the only signal — the lesson `is_context_overflow` already encodes.
Keep that function; it becomes the `ContextOverflow` arm.

**B. The retry policy**, in the provider, per request:

| Class | Policy |
|---|---|
| `RateLimit` | honour `Retry-After`, **capped** (below); up to `max_retries` |
| `Overloaded`, `ServerError`, `Transport` | exponential backoff from 2.5s, doubling, 30s cap |
| `Auth`, `Billing`, `Invalid`, `ContextOverflow` | never retried at this layer |

The cap on `Retry-After` is openclaw's sharpest small detail: a provider can
name a wait long enough that the process is simply asleep, and control never
returns to the layer that could fall back instead. Cap it at 60s by default
(`retry_after_cap_secs`), surface anything longer as a failure, and let the
caller decide.

**C. Where the retry lives.** Two layers, and the split is the invariant:

- **Inside the provider**, retry the HTTP request. Always safe: no tool has
  run, no output has been shown.
- **In the loop** (`agent.rs`), retry the *turn* only when
  `no tool has executed and no assistant text has been emitted this turn`. The
  loop already tracks both. If either is true, the error propagates.

Streaming complicates this and the rule handles it: once a delta has reached
the front-end, the turn is not retryable. That is the same boundary
cancellation already respects.

**D. Model fallback** — `[providers.X] fallbacks = ["small", "gemma26"]`,
tried in order on `RateLimit`/`Overloaded`/`ServerError`/`Transport`
exhaustion, never on `Auth`/`Invalid`/`ContextOverflow`. Turn-local: the next
turn starts from the selected provider again.

**The eval rule, non-negotiable:** `mecha eval` **forces fallback off**, like
it already forces MCP, hooks, outbox and learned rules off, and for the
identical reason. A scorecard grades the model it names. If a run does fall
back anywhere else, `RunConfig` records which provider and model actually
answered each turn — otherwise a replay of a fallen-back session replays the
wrong model and reports divergence that is really an artifact.

**Config:**

```toml
[providers.anthropic]
max_retries = 3            # 0 disables; default 3
retry_after_cap_secs = 60
fallbacks = []             # empty = strict, the default
```

Both fields go on `ConfigLayer` as well as `Config` — the round-trip guard
(`every_field_of_config_is_reachable_from_a_file`) will catch it if not, which
is what it is for.

**Tests that must fail on the old behaviour:**

- A mock HTTP server returning `429, 429, 200` yields a successful call with
  the pinned sampler intact; the same server with `max_retries = 0` fails.
- `401` is not retried — asserted by request count, not by elapsed time.
- **The invariant test**: a scripted run where turn 2 executes a tool and then
  the provider errors retryably does **not** re-execute the tool. This is the
  one that matters; write it first.
- `Retry-After: 3600` returns an error rather than sleeping.
- A `ContextOverflow` still reaches the existing compact-and-retry-once path
  and is not consumed by the new retry layer.

**Cost:** medium. The classifier and provider-level retry are contained; the
turn-level guard needs care in `agent.rs`.

---

## 3. The post-compaction loop guard ~~— built 2026-08-05~~

> **Built 2026-08-05** to this spec, with three deltas. It arms on *any*
> compaction, not only overflow recovery — same failure shape, superset
> coverage, and openclaw's own guard is post-compaction generally. The hash
> is a 64-bit `DefaultHasher`, not sha256 — nothing adversarial is being
> resisted, and a collision needs two different calls inside a window of
> three. And a detected loop also suppresses further compaction: a summary
> spent on a transcript about to be abandoned is pure waste (found by the
> test scripts, not foresight). Four loop tests cover trip, polling
> (same args + changing result never trips), dormant-until-compaction, and
> the off switch. `expect.stop_cause: "loop"` is gradeable; no shipped case
> asserts it, since a case cannot reliably make a model loop.

**The finding.** openclaw ships two loop detectors and makes them asymmetric on
purpose: general rolling-history detection is **off** by default, but the
post-compaction guard is **on unless explicitly disabled**, "because the guard
exists to escape compaction loops that would otherwise burn unbounded tokens,
and a no-config user still gets the protection." It keys on
`(toolName, argsHash, resultHash)` and aborts with a distinct cause.

mecha has turn budgets, which make a loop expensive rather than caught — and
`max_turns` fires with a `stop_cause` that says "hit the turn limit", which
reads like the task was too big when it was actually stuck.

The place mecha needs this is precisely openclaw's: `agent.rs:830` compacts and
retries once on overflow. If the model then repeats the same call and gets the
same result, compaction did not help, and the run is burning the largest
prompts it will ever send.

### Spec

- After an overflow-recovery compaction, hash each subsequent call as
  `(tool_name, sha256(canonical_args), sha256(result))`.
- A repeat within a window of 3 calls stops the run with a new
  `StopCause::Loop` (`agent.rs:410`), described as "repeated an identical tool
  call after compacting".
- **On unless `[agent] loop_guard = false`.** Same asymmetry, same reason.
- The general rolling-history detector is *not* built. openclaw's own guidance
  is that it is for smaller models, and mecha's local models are exactly that —
  so it is a reasonable follow-up, but it needs a measurement to justify it and
  the compaction guard does not.

**Tests:** a `ScriptedProvider` that emits the same call three times after a
forced compaction stops with `StopCause::Loop`; the same script with
`loop_guard = false` runs to the turn ceiling. Argument-identical calls with
*different* results do not trip it — otherwise a polling loop is
indistinguishable from a stuck one.

**Eval hook:** `expect.stop_cause: "loop"` becomes gradeable by the existing
run-metadata check, which is the machinery for grading the harness rather than
the model. Worth one case.

**Cost:** small.

---

## 4. Per-command approval policy

**The finding.** `ModeApprover::approve` takes `_input: &Value` and **ignores
it** (`tool/mod.rs:234`). Approval is tool-granularity, so `shell` is
all-or-nothing: allow every command or ask about every command.

Codex's execution policy is the best-designed version surveyed. Rules live in
`.rules` files in Starlark; the core construct is:

```python
prefix_rule(
    pattern = ["git", ["status", "diff", "log"]],
    decision = "allow",
    match = [["git", "status"]],
    not_match = [["git", "push"]],
)
```

Four properties worth keeping, each of which mecha has an existing reason to
like:

- **Decisions are `allow` / `prompt` / `forbidden`, and the most restrictive
  match wins.** A later rule can never widen an earlier one.
- **`match`/`not_match` examples are validated at load time.** A rule that does
  not do what its author claims fails at startup — the same principle as
  validating hook config even under `--no-hooks`, so a typo fails on every
  start rather than only on the run that needed it.
- **Shell splitting is conservative.** Linear chains of plain words joined by
  `&&`, `||`, `;`, `|` are split and each segment matched. Anything with
  redirections, substitutions, variable expansion, globs or control flow is
  treated as **one opaque invocation** — which it will not match, so it prompts.
  Fail-closed by construction rather than by a parser trying to be clever.
- openclaw adds **`strictInlineEval`**: an allowlisted interpreter is not an
  allowlisted command. `python -c`, `node -e`, `ruby -e`, `awk`, `sed`,
  `find -exec`, `xargs` are approval-only even when the binary is allowed, and
  an inline-eval command never persists a new allowlist entry.

And one hardening detail that is a genuine bug class here: openclaw **binds an
approval to exact argv + cwd + resolved executable path**, and rejects a
forwarded run whose command changed after approval. mecha approves
sequentially and executes concurrently (`CLAUDE.md`, Conventions), so the gap
between "the human said yes to this" and "this ran" is real, even if nothing
in-process currently exploits it.

**How this relates to the sandbox.** It does not replace it. The sandbox is the
enforcement behind `shell`'s capability label and stays load-bearing. A command
policy is what makes `kind = "none"` survivable, and what lets you tighten
*inside* a sandbox — a confined shell that can still `curl` inside its own
network namespace is confined, not harmless.

### Spec

**A. Widen the trait, minimally.** `Approver::approve` already receives
`&dyn Tool` and `&Value`; nothing in the signature changes. `ModeApprover`
consults a policy before falling back to the mode:

```rust
pub enum RuleDecision { Allow, Prompt, Forbid }

pub struct ExecPolicy { rules: Vec<PrefixRule> }

impl ExecPolicy {
    /// None when no rule matched — the caller falls back to PermissionMode.
    pub fn decide(&self, tool: &str, input: &Value) -> Option<RuleDecision>;
}
```

`Forbid` returns `Decision::Deny` **without asking a human**, which is the
existing "mechanical policy is cheaper than an interruption" argument from the
hook ordering. `Prompt` falls through to the interactive approver. `Allow`
short-circuits it.

**B. Not Starlark.** Codex embeds a Starlark interpreter; mecha should not
acquire a scripting language for this. TOML expresses the same shape:

```toml
[[rule]]
tool = "shell"
pattern = ["git", ["status", "diff", "log", "show"]]
decision = "allow"
match = ["git status", "git diff --stat"]
not_match = ["git push", "git commit"]

[[rule]]
tool = "shell"
pattern = ["rm", "-rf"]
decision = "forbid"
match = ["rm -rf build"]
justification = "never recursive-force from a model-supplied path"
```

`match`/`not_match` are **required to be non-empty for `allow` rules** (and,
as built, `match` for every patterned rule — a `forbid` too narrow to fire is
the guard that protects nothing) and
checked at startup. An `allow` rule that matches its own `not_match` example is
a config error, in the class that already kills startup.

**C. Splitting.** One function, pure, heavily unit-tested — this is where the
bugs will be:

```rust
/// Split a command into segments that can be judged independently.
/// Returns None when the command is not safely splittable, which means
/// "judge the whole string as one opaque invocation" — i.e. it will not
/// match a prefix rule and will prompt.
pub fn split_segments(cmd: &str) -> Option<Vec<Vec<String>>>;
```

Returns `None` for any `$(`, `` ` ``, `<`, `>`, `*`, `?`, `[`, `${`, newline,
or shell keyword (`if`, `for`, `while`, `case`, `{`). This is deliberately
over-conservative: a false `None` costs one approval prompt, a false `Some`
costs the whole point of the feature.

**D. Inline eval.** A hardcoded set of `(binary, flag)` pairs — `python -c`,
`python3 -c`, `node -e`, `node --eval`, `node -p`, `ruby -e`, `perl -e`,
`perl -E`, `php -r`, `lua -e`, `osascript -e`, plus `awk`, `sed -e`, `find`
with `-exec`, `xargs`, `make`. Matching any of these forces `Prompt` regardless
of what the rules say. `[approval] strict_inline_eval = true` by default;
turning it off is a decision someone makes on purpose.

**E. Argv binding.** When an interactive approver returns `Allow`, bind the
decision to `(tool_name, canonical_args, cwd)`. The dispatch path re-checks
that binding immediately before execution and treats a mismatch as a denial.
This is cheap — the values are already in hand — and closes the
approve-then-execute window.

**Ordering is unchanged:** interlock → hook → approver, with the policy inside
the approver. A rule can narrow and never loosen; the interlock still refuses
`external_send` under an armed trifecta no matter what any rule says. Worth a
test that names that.

**Tests:**

- `split_segments` table test: `a && b` splits; `a > b`, `a $(b)`, `a *`,
  `for x in ...` all return `None`.
- A `forbid` rule denies without the approver being consulted at all —
  asserted with an approver that panics if called.
- `python3 safe.py` allowed by rule, `python3 -c "..."` prompts, under the same
  rule set.
- Argv rebinding: a call approved with one argument set and executed with
  another is denied.
- A rule whose `not_match` example matches is a startup error.

**Cost:** the largest of the five. The splitter and the rule matcher are each a
day; the binding is small; the config surface needs the `ConfigLayer` treatment.

---

## 5. Pruning on its own cadence

**The finding, stated accurately.** openclaw separates two things mecha has
coupled. Its `session-pruning` is per-request, in-memory, deterministic, no
model call: soft-trim oversized old tool results to head + tail with `...`
between, hard-clear older ones past a ratio threshold, protect the last three
assistant turns and the bootstrap. Compaction is the heavy, lossy, model-
calling fallback.

mecha has the *mechanisms* — `compact::thin_old_results(keep_recent,
keep_chars)` and `evict_superseded_results` — but they fire only when the
compaction threshold trips or during overflow recovery. So the transcript
carries full tool results all the way to two-thirds of the window, then gets
thinned and summarised in one event.

**The part worth stealing is the cadence, not the mechanism.** openclaw's
default mode is `cache-ttl`: prune when the prompt cache is going to expire
anyway. That is the right shape for mecha specifically, because the cached
prefix is load-bearing here (`CLAUDE.md`, Provider notes) and trimming message
history on every turn would spend more on recaching than it saves in tokens.

### Spec

- `[agent] prune_at_fraction = 0.5` — when reported prompt tokens exceed this
  fraction of `context_window`, run `thin_old_results` + `evict_superseded_results`
  **without** summarising, and only when the last prune was longer ago than the
  cache TTL (5 minutes at mecha's current `cache_control`, which sends no
  `ttl`).
- Compaction keeps its own threshold and stays the fallback. Nothing about the
  cut-point logic changes.
- **Off by default.** Same argument as compaction: it is lossy, and a
  measurement should justify it. Unlike compaction it is cheap and reversible
  per-run, so it is a smaller decision.

**Measure before believing it.** The right experiment is the one this project
already knows how to run: the compacted chain cases at k=5 with pruning on and
off, plus cache-read token counts from the recorded usage. If cache reads
collapse, the cadence is wrong.

**Cost:** small — the mechanisms exist, this is a trigger and a clock.

---

## 6. What not to take, and why

**Micro-compaction every turn (hermes).** Fold the single oldest exchange into
a rolling summary after each turn. Occupancy stays flat near 40% instead of
sawtoothing toward 90%, and there are no long stalls. It also **breaks the
cache prefix on every single turn**, which their own doc concedes: "which side
wins depends on numbers specific to you." Against Anthropic with a placed
breakpoint, mecha's numbers say no. Against a local llama-server, where there
is no cache discount to lose, it is arguably right — worth remembering if the
local-only path ever needs it, and not worth building now.

**Channels, gateways, pairing (all three).** openclaw and hermes are both
fundamentally "your assistant in WhatsApp/Telegram/Discord". That is a
different product. The one adjacent idea — approvals forwarded to a chat client
— is worse than mecha's outbox, which stages the *artifact* for review rather
than asking a human to adjudicate a call they cannot see the consequences of.

**Plugin marketplaces (ClawHub, agentskills.io).** openclaw's own
THREAT-MODEL-ATLAS rates malicious skill installation **P0 critical**, with the
note that detection layers exist but skills run with agent privileges, and
lists "implement skill execution sandboxing" as an open P0 recommendation.
mecha's MCP posture — cleared environment, named allowlist, mandatory sandbox
preflight — is stronger than what they are trying to retrofit. Do not trade it
for a distribution channel.

**Model-reviewed approvals** (codex `approvals_reviewer`, openclaw
`mode: "auto"`, where a reviewer model adjudicates approval misses). mecha's
stated reason for putting the interlock *ahead* of the approver is that a human
clicking yes is what an injection is engineering. A model clicking yes is
strictly easier to engineer, and it launders the decision as policy. Skip.

**Whole-process sandboxing (hermes).** Worth recording as an honest limit
rather than as a rejection. Their SECURITY.md:

> nothing inside the agent process constitutes containment — not the approval
> gate, not output redaction, not any pattern scanner, not any tool allowlist.

That is mecha's silently-degrading-sandbox rule from the other direction, and
it points at something true: mecha confines the *tool*, so `http_fetch`, the
search backends, the MCP client and the provider clients all run unconfined in
the agent process. Hermes's answer is to wrap the whole process tree. mecha's
answer is that the confined surface is the one that runs model-chosen code —
which is a real answer, but it is narrower than "the agent is contained", and
the docs should not imply otherwise.

---

## Not researched

- **The source of any of the three.** Everything above is from documentation.
- **openclaw's `context-engine` plugin interface** — a swappable
  ingest/assemble/compact/afterTurn slot, with the engine quarantined and
  downgraded to `legacy` on failure. Interesting as an abstraction shape for
  `compact.rs` if a second strategy ever exists. There is currently one
  strategy, so this would be an interface with a single implementation.
- **codex's `code-mode` crate and openclaw's QuickJS-WASI code mode**, beyond
  noting they exist and that they are the same idea as the programmatic-tool-
  calling lever in `SANDBOX-RESEARCH.md`. The rule to write down before either
  is ever built: **every call the bridge makes must route back through the
  registry**, or code mode is a hole straight through interlock, hooks and
  approver. openclaw's own framing — "model code is hostile" — is the right
  one.
- **openclaw's `queue-steering` modes** (`followup`, `collect`) as an actual
  design for `mecha chat`. The observation that they need no ownership of
  stdin, and so are available to a readline REPL where steering is not, is
  worth following up.
