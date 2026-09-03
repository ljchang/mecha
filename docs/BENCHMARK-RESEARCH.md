# Public benchmarks, and whether any of them can compare *harnesses*

Research pass, 2026-08-05. Three questions, asked in this order because the
third one changes what the first two are worth:

1. Which public agent benchmarks are any good?
2. What do they cost in wall clock and money?
3. Is there a leaderboard that would let mecha be compared against other
   harnesses **on the same model**?

---

## The answer to (3) first, because it is the load-bearing one

Almost every leaderboard varies the *model* and leaves the harness
uncontrolled — which is exactly backwards for this project. mecha's whole
premise is that the harness is the thing being built, and a leaderboard that
reports "GPT-5.6: 89.5%" tells you nothing about a harness.

**Three exceptions exist, and two of them are directly usable.**

| Leaderboard | Separates harness from model? | Usable here |
|---|---|---|
| [Terminal-Bench 2.0/2.1](https://www.tbench.ai/leaderboard/terminal-bench/2.0) | **Yes** — `Agent`, `Model`, `Agent Org`, `Model Org` are separate columns; entries are audited and trajectories are public | **yes, and it has our exact model** |
| [SWE-bench Bash Only](https://www.swebench.com/) | **Yes, inverted** — the harness is *fixed* (mini-swe-agent) so the model is the only variable | as a **baseline to beat**, not to submit to |
| [HAL](https://hal.cs.princeton.edu/) (Princeton, ICLR 2026) | **Yes** — same model appears under different agents, with dollar cost per run | yes, and it wants submissions |

### The headline finding

**Qwen3.6-35B-A3B is already on the Terminal-Bench 2.0 leaderboard.** That is
the exact model on port 8080 — not a sibling, not a different quant, the same
model name.

| Rank | Agent harness | Model | Accuracy |
|---|---|---|---|
| 116 | little-coder | **Qwen3.6-35B-A3B** | **24.6% ± 3.2** |
| 121 | little-coder | **Qwen3.6-35B-A3B** | 23.0% |
| 138 | little-coder | Qwen3.5-9B | 9.2% ± 2.4 |
| 119 | Terminus 2 | Qwen 3 Coder 480B | 23.9% ± 2.8 |
| 110 | Dakou Agent | Qwen 3 Coder 480B | 27.2% ± 2.6 |
| 60 | Terminus 2 | GLM 5 | 52.4% ± 2.6 |
| 86 | Terminus 2 | DeepSeek-V3.2 | 39.6% ± 2.8 |
| 126 | Terminus 2 | GPT-OSS-120B | 18.7% ± 2.7 |
| 50 | Claude Code | Claude Opus 4.6 | 58.0% ± 2.9 |
| 1 | NexAU-AHE | GPT-5.5 | 84.7% ± 2.1 |

(Leaderboard read 2026-08-05; two `little-coder` × Qwen3.6-35B-A3B entries
exist with different scores, which is itself a useful reminder about variance.)

So the comparison the user asked for is available: **run mecha on
Terminal-Bench 2.0 with qwen3.6-35b-a3b and the number lands beside 24.6%,
with the model held exactly constant.** That is the single most informative
measurement available to this project, and it is the recommendation.

### How much can a harness actually move a score?

Worth calibrating expectations before spending the wall clock. The
Terminal-Bench paper ([arXiv:2601.11868](https://arxiv.org/abs/2601.11868),
ICLR 2026) ablates it directly and the conclusion is not flattering to
harnesses:

> Codex CLI resolution rate increases by 52% when using GPT-5.2 instead of
> GPT-5-Nano, while Gemini-2.5-Pro sees a 17% increase in resolution rate when
> paired with Terminus 2 instead of OpenHands, implying that **model selection
> is usually more important than agent scaffold**.

Read honestly: swapping the harness moved one model by **17%**, swapping the
model moved one harness by **52%**. A harness is worth roughly a third of what
a model is worth, on this benchmark. Secondary sources claim scaffolds move
SWE-bench by 10–20 points; that number appears in blog posts rather than in a
paper here and is repeated with that caveat attached.

17% is still a large, real effect, and it is the effect this project is
actually trying to produce. But a scorecard that comes back at 22% instead of
24.6% is not evidence that mecha is broken, and one at 30% would be a genuinely
notable result rather than a rounding error.

---

## The benchmarks, ranked by fit

| Benchmark | Tasks | Grades | Harness-separable | Rough cost/run | Fit here |
|---|---|---|---|---|---|
| **Terminal-Bench 2.0** | 89 | terminal work, verified by tests | **yes, first-class** | $1–100 API; free locally | **best fit** |
| **SWE-bench Bash Only** | 500 (Verified) | patch passes the repo's tests | harness fixed = baseline | free locally, slow | **best baseline** |
| **AgentDojo** | 97 tasks + 629 security cases | utility **and** injection resistance jointly | yes | cheap | **the interlock's benchmark** |
| **τ²-bench** | 375 across 4 domains | tool-agent-user interaction, `pass^k` | partly | 2× (user simulator) | good, noisy |
| **HAL** | 9 benchmarks | accuracy vs **dollar cost** | yes | ~$1.84/rollout | submit later |
| **BFCL v4** | large | function-calling correctness (AST) | no — a model benchmark | very cheap | sanity check only |
| **SWE-bench Verified** (full agent board) | 500 | same as above | **no** | expensive | skip |

### Terminal-Bench 2.0 — the recommendation

**89 tasks** (4 easy / 55 medium / 30 hard, 16 categories), selected from 229
crowd-sourced candidates by 93 contributors, each **manually verified by three
human reviewers** for solvability, realism and specification quality. Apache
2.0. Each task is a container, an English instruction, a test script, and an
oracle solution — the same shape as mecha's own `expect.verify` cases, which is
not a coincidence: outcome-graded, not answer-graded.

**Why it fits mecha specifically:** it grades an agent that has a shell in a
container and has to actually finish work. That is precisely mecha's surface —
`shell` plus the file tools plus the sandbox — and the harness properties this
project has invested in (path jail, sandbox confinement, budgets, compaction,
loop behaviour) are the ones that show up.

**Running it.** The harness is [Harbor](https://harborframework.com):

```bash
uv tool install harbor
harbor run -d terminal-bench/terminal-bench-2 -a oracle          # sanity check
harbor run -d terminal-bench/terminal-bench-2 \
  --agent-import-path "path.to.agent:MechaAgent" -k 5
```

Docker required locally; Daytona for cloud fan-out (`--env daytona -n 32`).

**Wrapping mecha.** Two interfaces, and mecha should use the second:

- `BaseAgent` — external: `name()`, `version()`, `setup()`, `run()`, driving a
  `BaseEnvironment` by executing bash. This would make Harbor the thing running
  commands, with mecha as a planner. Wrong shape.
- `BaseInstalledAgent` — `install()` (via `exec_as_root` / `exec_as_agent`),
  `run()`, `populate_context_post_run()` (parses trajectory files into an
  `AgentContext`). This installs the `mecha` binary *inside the task container*
  and runs it there, which is the honest configuration: mecha's own sandbox,
  path jail and tools are what get measured.

`populate_context_post_run` is the piece that already exists here — mecha's
session JSONL is a trajectory file, and the adapter is a parser over it.

**Cost and wall clock, honestly.** The paper: most trials finish in **under 20
minutes**, most use **fewer than 25 model calls and under 10M tokens**, and
extreme cases run 2 hours and ~100M tokens. A full run costs "one to a hundred
dollars depending on the model's price."

On the DGX with a local model the money is zero and the wall clock is the
constraint. 89 tasks × k=5 = **445 rollouts**. At a 10-minute average that is
~74 hours sequential. Two things follow:

- **k=1 first** (89 rollouts, ~15 hours) to shake out the adapter, then k=5
  once for a leaderboard-comparable number.
- **Concurrency breaks the seed.** `ARCHITECTURE.md` already records this:
  llama-server's continuous batching perturbs numerics, so a seeded run only
  replays token-for-token at `--concurrency 1`. For a benchmark that is fine —
  pass^k *wants* independent samples — but do not expect a Terminal-Bench run
  to be reproducible the way a `mecha eval --concurrency 1` run is, and say so
  in whatever gets published.

**Leaderboard submission** is currently *"coming soon"* per the docs; the
HuggingFace mirror README describes a PR-based path. Worth confirming before
counting on it. Nothing stops the comparison itself — the 24.6% figure is
public and the run is reproducible locally either way.

**Version discipline:** 2.0 and 2.1 have separate leaderboards. Pick one, and
compare only within it. There is also a Terminal-Bench Hard and a
Long-Horizon-Terminal-Bench (46 tasks, ~9.9M tokens and 85 minutes *per task*,
53–71 hours for one pass) — the latter is out of reach here and is noted only
so nobody starts it by accident.

### Running it on this box — what the first passes cost to learn

Everything above was research. This is what actually happened on the DGX
(aarch64), and every item below is a thing that produced a *plausible wrong
number* rather than an error.

**The denominator is 75, not 89.** The prebuilt task images are amd64-only and
this host is aarch64, so `bench/run.sh` passes `--force-build` and each task is
built from its own Dockerfile. Fourteen tasks then fail *their own reference
solution* here — mostly the numerical and scientific-toolchain ones
(`mcmc-sampling-stan`, `rstan-to-pystan`, `caffe-cifar-10`, `mteb-*`,
`largest-eigenval`, `protein-assembly`) plus four builds and the two MIPS
tasks. A mecha score on those measures the architecture, not the agent. The
list lives in `bench/oracle-arm64-excluded.txt`, derived from the sweep's
`result.json` rather than typed by hand, and **any published number must name
the 75**.

**The oracle sweep is calibration, not a score.** `harbor run` with no `-a`
runs the `oracle` agent — each task's shipped reference solution, no model
anywhere. The 2026-08-05 sweep took **14.4 hours** and answered exactly one
question: which tasks work here. Nearly all of that was building 89 images;
the reference solutions themselves are fast. Worth stating plainly because the
duration reads like a benchmark run and is not one — a 14-hour job had
completed and the agent had still never been scored.

**`-x` and `-i` match dataset-qualified names, and a miss is silent.** Harbor
matches globs against `terminal-bench/<task>`, so a bare `-x make-mips-interpreter`
matches **nothing**, excludes **nothing**, warns about nothing, and exits 0.
The job then runs all 89 while every artifact still says "subset", and the
scorecard is indexed by a denominator that was never true. This cost two runs
before it was noticed — the 2026-08-05 job carrying `exclude_task_names:
["make-mips*"]` has `make-mips-interpreter` in its own results, which is the
tell nobody read. Two guards now: `bench/run-subset.sh` refuses a name without
a `/`, and `bench/check-subset.py` compares a job's `lock.json` against the
calibrated list. `lock.json` is written before any trial runs, so the check is
a genuine preflight — and it compares *name sets*, not counts, because 75 of
the wrong 75 is still wrong.

**Pin the dataset ref to the one the oracle measured.** `bench/run.sh` defaults
to `terminal-bench/terminal-bench-2`, which resolves `latest` at launch. The
exclusion list is a claim about 89 *particular* tasks, so a dataset that gained
or changed a task would silently invalidate it. `bench/run-subset.sh` pins the
sha the sweep ran against; moving it means re-running the sweep.

**`--n-concurrent-agents 1` is load-bearing, not tuning.** The host runs one
llama-server with `-np 1` — a single slot holding the whole 32768 context,
which `bench/run.sh` refuses to start without (4 default slots quarter the
context to 8192, the confound that voided a day of scorecards). Concurrent
trials do not get concurrent slots: they queue, and each switch evicts the
other's KV prefix, so prompt caching collapses and agent timeouts start firing
on the queue rather than on the work. The run therefore uses `-n 4` with the
agent phase capped at 1 — builds and verifiers overlap, since those are CPU
work with no model in them.

**The binary must be statically linked, or it does not start.** A host
`cargo build --release` links glibc 2.39; the task containers are other
people's images — Debian 11 (2.31), Ubuntu 22.04 (2.35), Alpine (musl). The
binary is uploaded into each one and fails there with

```
/installed-agent/mecha: /lib/aarch64-linux-gnu/libc.so.6:
    version `GLIBC_2.39' not found (required by /installed-agent/mecha)
```

and — this is the part that matters — it fails **as an agent error**. The trial
records `NonZeroAgentExitCodeError` and reward 0.0, which in a scorecard is
indistinguishable from a model that tried and failed. `bench/build-portable.sh`
builds static musl in a container (`ring` needs a C toolchain, musl-tools needs
root; rustls means no OpenSSL), asserts the result is actually static, and
`bench/run.sh` points `MECHA_BENCH_BINARY` at it. Verified the way this project
requires — the old binary fails on all three bases, the new one runs on all
three, Alpine included, where an older-glibc build would still be "not found".

**The per-turn budget is a *reasoning* budget, and 8192 was not enough.** This
is a thinking model. Measured directly against the local server: a hard task at
`max_tokens: 8192` returns `finish_reason: length`, **23,682 characters of
`reasoning_content`, and an empty `content`**. Raising to 16384 and 24576 gave
49,587 and 59,917 characters of reasoning and still no answer; easy and moderate
prompts finish with `stop` and real content in a few hundred tokens. So the
failure is specific to hard tasks, which is exactly the set a benchmark is made
of.

What mecha does with that is the harness half of the bug: an assistant turn with
no content ends the run — **exit 0, no answer, reward 0.0, no exception**. Both
completed trials of the first real run died that way, one after 2 turns and one
after 7. It reproduces outside the benchmark in one command, which is how it was
confirmed rather than inferred.

**Raising the budget is not the fix, and the first attempt at it did real
damage.** The server went to `-c 131072` and the posture to `max_tokens: 32768`
on the theory that thinking needed room. Every part of that was wrong:

- `circuit-fibsqrt` spent 33,019 output tokens across 2 turns and *still* ended
  empty; `overfull-hbox` never finished a single turn inside the harness's
  20-minute agent timeout. A bigger budget buys a longer runaway, because the
  reasoning has no upper bound of its own — at `max_tokens: 12288` an answer
  appeared in **2 of 4 samples**, a coin flip rather than a fix, and the reason
  some trials passed and some died.
- **`-c 131072` cost a 50x slowdown**: 64 tokens went from 1.06s to 52.6s, the
  server pinned at ~97% of one core, because the KV cache stopped fitting
  alongside the model. Nothing errored. The server answered every request. The
  only symptom was trials sitting on their first turn for forty minutes — which
  reads exactly like a hard task. The context is back at 32768, and the lesson
  is to **measure tokens/sec after touching `-c`**, not just that the server
  came back up.

**The fix is `--reasoning-budget`, and it has to be a server flag.** llama-server
caps the thinking block, closes the tag when the cap is hit, and injects
`--reasoning-budget-message`, after which the model answers. At 4096 the same
hard prompt produced content in **4 of 4** samples. The per-request
`reasoning_budget` field is *silently ignored* by this server — A/B tested at
identical `max_tokens`, the "with" arm still returned empty content — so it
cannot be set per run and belongs in `scripts/start-moe-mtp.sh`. `max_tokens`
then only needs to exceed the budget with room for an answer (16384).

Four numbers now move together, and a mismatch is silent in every direction:
`--reasoning-budget` and `-c` in `scripts/start-moe-mtp.sh`, and a `max_tokens`
above the budget beside a `context_window` equal to the `-c` in both
`bench/mecha_agent.py` and `~/.mecha/config.toml`. That last file had
`max_tokens = 4096` — *at* the reasoning budget, leaving nothing for an answer,
which would have made hard everyday turns come back empty too. The final
posture is the original one plus the budget: `-c 32768`,
`--reasoning-budget 4096`, `max_tokens 8192`.

**Both halves were needed, and the measurement is what proved it.** The first
75-task run was launched with only the server-side budget, on the reading that
an empty turn was now rare enough. It was not: at 28 trials, **15 ended empty
and none of those 15 passed**, while 8 of the 13 that reached a real conclusion
did. That is a scorecard measuring whether mecha survived, not whether the model
could do the work, and it is why the run was stopped rather than finished.

So `agent.rs` now treats a turn with no text and no tool calls as recoverable:
it folds a nudge into the preceding user message — the steering rule, because
two user messages in a row are invalid — and retries, up to
`EMPTY_TURN_RETRIES`. The empty assistant message is deliberately never pushed,
since some providers reject an assistant turn with empty content and keeping it
would make the retry unsendable. Detection keys on *the turn carrying nothing*
rather than on the stop reason, because providers disagree about the label —
`max_tokens` from one, plain `stop` from another with the reasoning silently
truncated.

When the retries are spent the run stops as `StopCause::NoOutput` with
`exhausted: true`. That half matters as much as the recovery: `finish` reported
`Completed` with `exhausted: false`, so **a run that produced nothing reported
success**, which is precisely why 15 dead trials read as ordinary failures. The
existing test asserting that behaviour — *"It completed; it just had nothing to
say. Don't misreport that."* — was rewritten, since the premise was the bug.

Verified against the real model with the server budget removed, which is the
only way to exercise it (a `ScriptedProvider` replays what you *believe* a
provider does): three nudge retries fired, the run gave up naming the cause and
the fix, and exited non-zero instead of 0.

**What a first-pass smoke does and does not tell you.** The single-task smoke
(`overfull-hbox`, 2026-08-05) scored 0.0 at 61.7k input / 24.6k output tokens.
That is a working adapter, not a signal about capability: one task at k=1 from
a set whose published top entry is 84.7% carries no information about the
harness at all. The sharper lesson from 2026-08-07 is that **three separate
defects each produced a plausible 0.0** — wrong task set, a binary that could
not start, and a budget too small to answer in. None of them raised an error a
scorecard would show. Read a trial's session transcript before believing any
number, and check `turns` against the budget: a run that stops at 2 of 40 did
not lose, it died.

### The 2026-08-07 subset run, diagnosed (2026-08-09)

The interrupted 75-task run (21 completed before it was stopped: **8 passed,
13 at 0.0**) was read trial by trial. The 13 failures, by cause:

| Cause | Trials |
|---|---|
| Empty turns → `NoOutput` death | `break-filter-js-from-html`, `dna-assembly` |
| Context-overflow 400, fatal | `path-tracing` |
| Harness agent-timeout, no transcript survived | `overfull-hbox`, `polyglot-rust-c` |
| `MaxTurns` at 40 | `video-processing`, `compile-compcert`, `db-wal-recovery` |
| Model concluded and was wrong | `distribution-search`, `circuit-fibsqrt`, `log-summary-date-ranges`, `cancel-async-tasks`, `dna-insert` |

Four hypotheses were on the table, and the evidence sorted them cleanly:

- **The trifecta interlock costs the benchmark nothing.** Every session's
  taint record reads `untrusted: false` — the bench surface has no web tools,
  and `shell` is not an untrusted source — so the interlock structurally
  cannot fire, and no transcript shows a blocked send or a denial.
- **The 32k context is genuinely overloaded, with a named mechanism.** The
  per-turn output budget (24 KB flat) is ~8–12k tokens of numeric data,
  larger than the 10.9k-token gap between the compaction threshold and the
  window, so one turn's results leapt the gap (`path-tracing`: `fs_read` of a
  2004-line PPM, dead at 45,325 tokens). The budget now derives from the
  window: 12,288 bytes at 32k.
- **The loop had real structural gaps.** The overflow-recovery give-up flag
  was set by "nothing worth summarising" and then gated the whole recovery,
  so the next overflow died raw; the empty-turn allowance was cumulative
  across the run, so long runs died `NoOutput` mid-task after early
  recoveries; and every exhausted stop exited 3, which Harbor records as
  `NonZeroAgentExitCodeError` — `headless-terminal` hit MaxTurns, was
  counted an agent *error*, and passed verification at 1.0 anyway. All three
  fixed (see the changelog).
- **Compaction itself was mostly exonerated** — the two heaviest trials (525k
  and 409k input) compacted repeatedly and both passed. Its harms were the
  give-up flag above and a recording bug: session files sliced "what the run
  added" off a list compaction had rewritten, so a 28-turn trial recorded 8
  assistant turns starting mid-conversation, and crashed runs recorded
  nothing at all. Transcripts now record rewrites explicitly and survive
  crashes; the bench adapter also captures stderr and `MECHA_LOG=debug`,
  because Harbor's `stderr: None` had discarded the compaction notices that
  would have made this diagnosis a day shorter.

Empty turns deserve their own line: they persisted **with
`--reasoning-budget 4096` active** (server restarted 11:16, run launched
11:18), which the 4-of-4 raw-prompt probe had said was the fix. In real agent
conversations the model still goes quiet routinely — `circuit-fibsqrt` billed
28 turns for 8 recorded assistant messages, and `cache_read: 0` in every
summary means each retry re-paid the full prefill (partly a reporting gap:
`decode_usage` reads only `prompt_tokens`/`completion_tokens`, and
llama-server's server-side prefix cache is invisible to it). The nudge
demonstrably works — `crack-7z-hash` and `headless-terminal` each recovered
from one and passed — which is what justified resetting the allowance instead
of raising it.

Scale check before the next run: 8/21 is 38% of *completed* trials against
little-coder's 24.6% on the full set — not comparable, since the 50 pending
tasks are not a random sample. The recoverable third is the harness deaths;
the five model failures are the model.

### The empty turns, explained (2026-08-10) — and the 08-09 reading corrected

The section above ends by noting that empty turns persisted **with
`--reasoning-budget 4096` active**, and treats that as the model going quiet
"routinely". That reading was wrong, and the mitigation built on it — nudge,
retry, reset the allowance — was treating a symptom.

The 2026-08-10 relaunch was read the same way, and this time the prefixes were
replayed against a second server rather than reasoned about. Method: take the
transcript up to each nudge point, strip the nudge back off (it did not exist
when the model went quiet), re-send with the real 7-tool bench surface at the
same unpinned temperature. Empty turns reproduce at ~15%, and one prefix
reproduces deterministically.

What an empty turn actually contains:

```
finish_reason: "stop"   content: 0   tool_calls: []   reasoning_content: 3681
  ...What about `<SCRIPT>` in uppercase?
  <tool_call><function=shell><parameter=command>python3 -c "..."
```

and in the deterministic case, **120 characters that are only a tool call**,
with no deliberation at all. The model emits its call before closing
`</think>`; llama.cpp files the whole turn as `reasoning_content` and reports a
clean stop. Upstream: ggml-org/llama.cpp #20837, #22684, #20809 — all unfixed,
with the same failure reported against ollama, so it is not purely a Qwen
property even though every public report names Qwen.

**This retires the budget explanation.** 120 characters is ~30 tokens of an
8,192 `max_tokens` with a 4,096 reasoning budget. No token limit was ever
involved, which is why raising `max_tokens`, capping reasoning, and enlarging
the window all failed to end it.

And half of it was mecha's. The harness stripped every `<think>` block from the
history it sent back, so the model saw turn after turn of itself calling tools
without thinking — the exact malformation being chased. Same server, same
template, same prompt, varying only the history:

| history | empty turns |
|---|---|
| without reasoning (mecha ≤0.1.1) | **6 / 6** |
| with reasoning (mecha 0.1.2) | **0 / 6** |

Fisher exact p ≈ 0.001 on a reproducer that fails byte-identically. A
third-party replacement chat template
([froggeric/Qwen-Fixed-Chat-Templates](https://huggingface.co/froggeric/Qwen-Fixed-Chat-Templates))
fixes it equally well (6/6 → 0/6) by instructing the model to close `</think>`
first — but it is prompt-level, therefore probabilistic, and Qwen-only. The
history fix addresses the cause and names no vendor.

Two numbers worth carrying into the next run. Empty-turn waste was **2.5% of
wall clock** on the 08-10 run, so this is not what the timeouts were made of —
23% of trials died on the clock and most of them were the model grinding.
And prefix-cache reuse is **better than 95%** (5,000-token prompts prefilling
16–211 tokens), which is what makes replaying reasoning affordable at all.

### SWE-bench Bash Only — the baseline to beat

The interesting split is not the main leaderboard. It is **Bash Only**, which
fixes the harness to [mini-swe-agent](https://github.com/SWE-agent/mini-swe-agent)
— ~100 lines of Python, bash as the *only* tool, and deliberately **not using
the model's tool-calling interface at all**, so it runs with any model. It
scores >74% on SWE-bench Verified with a frontier model.

That makes it the cleanest available control in the whole survey: it is the
minimum viable harness. Anything mecha does beyond it — the tool set, the path
jail, compaction, budgets, learned rules — is the hypothesis. If mecha does not
beat mini-swe-agent on the same model, the extra machinery is not paying for
itself on this workload, and that is worth knowing.

Note it also runs under bubblewrap, docker/podman, and singularity, so its
confinement story overlaps mecha's and can be configured to match rather than
confound.

Cost warning: SWE-bench Verified is 500 instances. HAL uses **SWE-bench
Verified Mini** (50 instances) for exactly this reason, and even there reports
$259–$1,790 per model with frontier APIs. Locally, use the 50-instance subset
first.

**Caveats, which are serious:** the benchmark is mature and heavily exposed in
public training data; contamination and saturation are real and acknowledged by
the people hosting the leaderboard. Frontier scores near 95% mean almost
nothing now. For a 35B local model in the 15–30% band, contamination pressure
is lower and the signal is better — Qwen3-32B at 15.2% and DeepSeek-R1-0528 at
30.3% on a 100-instance sample are the neighbourhood to expect.

### AgentDojo — the one that grades what mecha is actually built around

**97 realistic tasks across four suites (workspace, Slack, travel, banking) and
629 security test cases**, and it measures **utility and security jointly**:
benign utility, utility under attack, and attack success rate. Published, and
the environment is explicitly extensible for new defenses.

No other benchmark in this survey grades the thing this project has spent the
most design effort on. mecha's trifecta interlock, the `external`/capability
split, the taint-on-`Conversation` fix, the batching hole that was found and
closed — all of that is currently evidenced by unit tests and one fixture-based
eval case (`interlock-blocked`). AgentDojo would turn it into a number
comparable with the literature, and — the part that matters — it would measure
the **cost** of the defense, because an interlock that refuses too much shows
up as lost benign utility.

That is the honest risk, and it is why this is worth running rather than
assuming: mecha's interlock is deliberately blunt. It refuses *any*
`external_send` once both taint legs are armed. On a benchmark where the task
*is* "read this email and send a reply", a blunt interlock may score well on
ASR and badly on utility. Finding that out is the point.

Cheap to run relative to the others (no container-per-task compilation, short
episodes). Reachable via UK AISI's `inspect_evals`, which is a second harness
to integrate against — worth checking whether their agent interface is easier
to satisfy than writing a native adapter.

### τ²-bench — good, and noisier than it looks

**375 tasks: airline 50, telecom 114, retail 114, banking 97.** Dual-control —
in the telecom domain both the user *and* the agent can call tools. Text and
voice modes.

Two reasons it is a strong fit on paper: it grades **tool-call traces against
verifiable database outcomes**, which is exactly what `eval/cases.jsonl` does;
and **`pass^k` is its metric** — all k runs must pass — which is the metric
mecha implemented on 2026-08-05. The τ-bench line "61% pass^1 → <25% pass^8" is
already quoted in `HANDOFF.md` as the justification for that work. Running
τ²-bench would be measuring mecha with its own yardstick, borrowed from the
people who made it.

Three cautions:

- **A user-simulator LLM is in the loop**, so every task costs two models'
  tokens and inherits the simulator's variance. On the DGX that means two local
  servers or one serving both roles.
- **The public leaderboard reports Pass^1**, not pass^k, despite pass^k being
  the contribution. Comparing requires care about which is quoted.
- **A grading change in v1.0.1 (July 2026) altered banking scores, and results
  across that boundary are explicitly not comparable.** Pin the version, and
  record it — the same rule this project already applies to its own scorecards
  after the fixture expansion.

### HAL — the right leaderboard to *submit* to, later

Princeton's Holistic Agent Leaderboard: 9 benchmarks (AssistantBench, GAIA,
Online Mind2Web, CORE-Bench Hard, SciCode, ScienceAgentBench, SWE-bench
Verified Mini, τ-bench Airline, USACO), 21,730 rollouts across 9 models for
~$40,000 — about **$1.84 per rollout**. ICLR 2026.

Two things make it the right eventual home:

- It reports **Pareto frontiers of accuracy vs cost** rather than a 1-D
  ranking, and it shows the same model under different agents with different
  costs. That is the comparison this project wants, made explicit.
- It takes agent submissions through `princeton-pli/hal-harness`.

Its headline finding is also worth carrying into mecha's own eval design:
**higher reasoning effort reduced accuracy in the majority of runs.** That is
directly testable here — the TUI has a reasoning toggle and `eval` has
`--runs k` — and it would be a cheap, genuinely interesting local experiment
independent of any submission.

### BFCL v4 — a sanity check, not a harness benchmark

Berkeley Function Calling Leaderboard: AST-based grading of function calls,
now with agentic sections (web search, memory, format sensitivity). Cheap and
fast.

It measures the **model**, not the harness — mecha would contribute nothing but
a wrapper. Its one use here is diagnostic: this project already knows
`llama-server --jinja` grammar-constrains tool calls and that malformed-argument
counts are consequently zero. BFCL's format-sensitivity section is the public
version of that finding, and a quick run would confirm the local stack is not
leaving accuracy on the table in argument construction before blaming the
harness for a low Terminal-Bench score.

---

## Recommended order

1. **Terminal-Bench 2.0, k=1, qwen3.6-35b-a3b.** ~15 hours. Deliverable: a
   `BaseInstalledAgent` adapter and a number to put beside little-coder's
   24.6%. Everything else is downstream of this working.
2. **Same, k=5.** ~74 hours, one weekend. Now it is leaderboard-comparable and
   has a confidence interval, which every published entry carries.
3. **AgentDojo.** The only measurement of the security model that is comparable
   to anything outside this repo, and the only one that will price the
   interlock's false-refusal cost.
4. **SWE-bench Verified Mini (50) under mini-swe-agent, then under mecha.** The
   minimum-viable-harness control. Cheap to state, expensive to argue with.
5. **τ²-bench, one domain (airline, 50 tasks)** before committing to 375.
6. **HAL submission** once 1–4 have produced numbers worth publishing.

Two things to build once, that all of the above need:

- **An adapter layer.** Harbor's `BaseInstalledAgent`, `inspect_evals`' agent
  interface, and `hal-harness` all want "install this binary, run it with this
  prompt, hand back a trajectory". mecha already emits session JSONL; the
  adapters are parsers over it. Write it as one crate-external Python shim, not
  three.
- **A `--benchmark` posture.** Every one of these must run with MCP, hooks,
  outbox and learned rules **off**, exactly as `mecha eval` already forces —
  a scorecard shaped by local machinery grades this machine, not the harness.
  Fallback (if §2 of `PRIOR-ART-RESEARCH.md` gets built) must be off too.

---

## Caveats worth carrying

- **Contamination and saturation.** SWE-bench Verified frontier scores are
  approaching 95% and the hosts themselves flag training-data exposure.
  Terminal-Bench 2.0 is newer and harder (top entry 84.7%), which is part of
  why it is the recommendation.
- **Everything here is one number with a wide interval.** Published entries
  carry ±2–3 points at k=5 on 89 tasks. A 2-point difference is noise. This
  project already learned that at a smaller scale — a single `chain-largest`
  failure that looked like a regression was variance at n=5.
- **A leaderboard entry is not a controlled experiment.** Two `little-coder` ×
  Qwen3.6-35B-A3B rows differ by 1.6 points. Reasoning effort, context window,
  quantization and sampler are all uncontrolled across entries and mostly
  unreported. The comparison is indicative, not decisive — and mecha's local
  server settings (`-c`, MTP draft, `temperature 0.8`, `seed 42`) should be
  published alongside any number so someone else can say the same about ours.
- **These benchmarks grade task completion, not the properties this project
  cares most about.** Nothing on Terminal-Bench measures whether taint survived
  compaction, whether the outbox staged a send, or whether a learned rule came
  from a poisoned reflection. `eval/cases.jsonl`'s run-metadata checks remain
  the only instrument for those, and public benchmarks do not replace them.
  AgentDojo is the one partial exception.

---

## Not researched

- **Whether Terminal-Bench's leaderboard submission is actually open.** The
  docs say "coming soon"; the HuggingFace mirror describes a PR path. Confirm
  before planning around a public entry.
- **What `little-coder` is.** It is the harness holding the directly comparable
  Qwen3.6-35B-A3B slot, and reading it before running would be worth an hour —
  a harness scoring 24.6% where 9B models score 9.2% is doing something.
- **`inspect_evals` as a general adapter target.** UK AISI's harness hosts
  AgentDojo and many others; if its agent interface is easy to satisfy, it may
  be a cheaper integration than per-benchmark adapters.
- **OSWorld, WebArena, GAIA-2, MLE-bench, Cybench, SWE-Lancer, SWE-bench Pro,
  Long-Horizon-Terminal-Bench.** Surveyed only as names. None is a better first
  move than Terminal-Bench, and LHTB (53–71 hours for a single pass) is
  actively out of reach on this hardware.
- **Aider's polyglot leaderboard**, which fixes its own harness across models —
  possibly a second cheap control alongside mini-swe-agent, unexamined.

---

## Addendum 2026-09-03: Terminal-Bench 2.1, 3.0 and 4.0, and which one to run

Checked because the 2026-08-05 pass above cites "2.0/2.1" and the
comparability argument rests on one leaderboard entry. Sources: the
tbench.ai news posts for 2.1, 3.0 and 4.0; the Harbor Hub dataset pages;
the snorkel.ai leaderboard mirrors (the tbench.ai leaderboard is
client-rendered and unreadable by fetch); and both task sets downloaded
and read locally (`harbor datasets download`, `task.toml` per task).

| version | date | tasks | what it is | leaderboard |
|---|---|---|---|---|
| 2.0 | 2025-11-07 | 89 | the set every number above was measured on | 142 entries, **archived**; the little-coder + Qwen3.6-35B-A3B 24.6% entry lives here |
| 2.1 | 2026-05-06 | 89, same names | 2.0 with 28 tasks fixed (image drift, resource mismatches, misspecification); agent–model pairs gain +0.9 to +12.1 pp, so 2.0 and 2.1 scores are not interchangeable | 17 entries, top 83.8%; no Qwen, no local-model agent |
| 3.0 | 2026-07-30 | 74 | an entirely new "frontier" set: GPUs up to one H100, multi-container topologies, seven domains | 12 entries, top 42.7%; only open-weight is GLM-5.3 |
| 4.0 | 2026-08-28 | 66 | 3.0 minus 8 (saturated, refusals, public solutions, quality), 20 revised, resources calibrated, flat 8 h agent timeout; `terminal-bench/terminal-bench@4.0.0` | 13 entries, top 57.9%; GLM-5.3 at 41.8% the only open-weight; Sonnet 5 at 12.4%; **no Qwen, no small model** |

**What the 4.0 task set actually asks for**, read from its 66 `task.toml`s:
3 GPU tasks, 11 docker-compose environments, 1 task declaring
`mcp_servers`, internet **off** on all 66 (on for all 89 of 2.1), memory
median 4 GB and max 32 GB, up to 16 CPUs, storage up to 1 TB, and 8 hours
per task — a 528-hour ceiling per k=1 pass against 42 hours for 2.1. **Zero
task overlap** with 2.x; our 75-task subset survives into 2.1 exactly and
into 4.0 not at all. The metadata schema changed too: no `difficulty`,
new category names.

**Ruling proposed.** *2.1 is the working set; 4.0 is a ceiling slice, not
a pool.* The owner's framing (2026-09-03): the purpose is research and
papers, not submission, so the leaderboard is a sanity anchor and the
reasons that decide are headroom, a frozen set, and power — a small model
near floor on 4.0 makes every ablation read as no difference, 2.1 stops
moving, and on the x86 box all 89 tasks at k=5 give 445 paired trials per
arm.

- **Run experiments on 2.1.** Same subset, fixed tasks (fewer spurious
  failures to argue with), a 42-hour ceiling, and the only version family
  with a small-local-model comparator — even if that comparator is on the
  archived 2.0 board. Re-baseline once, because the 28 fixes move scores.
- **Do not make 4.0 the experiment pool.** It is calibrated to
  discriminate *frontier* models; Sonnet 5 scores 12%; a Qwen3.6-35B run
  would be a lone point near zero with nothing on the board to compare a
  harness against, at ten times the wall clock. That is the premise of
  §"The answer to (3) first" inverted.
- **Take a 4.0 slice as a ceiling set.** Tasks the local model is expected
  to fail are exactly what the appraisal validity dataset needs
  (`EXPERIMENT-DESIGN.md` §17: failures invisible to the counters) and
  what the communication programme's oracle-rescue probes need — *if
  perfect communication cannot rescue the small model here, communication
  is not the limiting resource.* Pick the non-GPU, non-compose tasks first;
  the three GPU tasks contend with the llama-servers for the same cards.
- **Author our own tasks to 4.0's conventions**, since the format evolves
  with the newest version: internet off by default, `mcp_servers`
  declared in `task.toml`, compose environments allowed.

Not verified: whether the 2.1 leaderboard still accepts submissions;
whether 4.0's task ids are `terminal-bench/<name>` for `-i`/`-x` (the
adapter's dataset-qualified names must be re-checked before a run);
whether Harbor's local docker backend runs the compose tasks as Modal
does. The downloaded task sets are in this session's scratchpad, not the
repo.
