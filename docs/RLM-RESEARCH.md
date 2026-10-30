# Recursive language models: what is worth taking into mecha?

Research pass, 2026-10-30, at the owner's request: is there anything in the
recursive-language-model line of work, read as an agent-harness design,
that mecha should incorporate? The design that came out of it is
`CODE-MODE-DESIGN.md`; this file is the evidence and the reasoning, and is
not updated as the design moves.

**Marks**: ✅ read in the primary source this pass · 📰 an author's own
figure, not reproduced · ❓ inferred, not established.

---

## The one-sentence answer

**Take the handle, the typed sub-call and the depth-1 limit, and nothing
else.** An RLM is the `code` tool `AUDIT-RESEARCH.md` §3.12 already
planned, plus one host function that is a `QuarantinedPass` with a shape.
The handle and the budget are a context and reliability win on their own.
The security win is narrower than it first looks: the literature on typed
extraction says a typed answer from an attacker's text is *bounded*, not
clean. So mecha should take that win where it already takes it, at the
subagent boundary, through `trusted_output`'s owner-declared
`AnswerShape`, and not inside a program.

---

## 1. The three papers

### Recursive Language Models — Zhang, Kraska, Khattab ✅

[arXiv 2512.24601](https://arxiv.org/abs/2512.24601), v1 2025-12-31, v3
2026-05-11; code at [alexzhang13/rlm](https://github.com/alexzhang13/rlm).

- **Mechanism.** The user prompt is stored as a string variable in a
  persistent Python REPL. The root model sees only constant-size metadata
  (its length, a short prefix). It writes code to inspect, slice and
  regex the variable, and calls `llm_query` — a sub-model — on pieces.
  Each REPL step returns only a prefix and the length of stdout to the
  root. The run ends when the program sets `FINAL(answer)` or
  `FINAL_VAR(variable)`.
- **Depth.** 0 = no sub-calls, 1 = sub-*model* calls (the default),
  >1 = sub-*RLM* calls.
- **Results (Table 1).**

  | | CodeQA | BrowseComp+ 1K | OOLONG | OOLONG-Pairs |
  |---|---|---|---|---|
  | GPT-5 base | 24.0 | 0.0 | 44.0 | 0.1 |
  | GPT-5 compaction | 58.0 | 70.5 | 46.0 | 0.1 |
  | GPT-5 RLM depth 1 | 62.0 | 91.3 | 56.0 | 58.0 |
  | Qwen3-Coder base | 20.0 | 0.0 | 36.0 | 0.1 |
  | Qwen3-Coder compaction | 50.0 | 38.0 | 44.1 | 0.31 |
  | Qwen3-Coder RLM depth 0 | **66.0** | 46.0 | 43.5 | 17.3 |
  | Qwen3-Coder RLM depth 1 | 56.0 | 44.7 | **48.0** | **23.1** |
  | Qwen3-Coder RLM depth 2 | 54.0 | **68.0** | 26.0 | 19.0 |

  Median GPT-5 gain 📰: +26% over compaction, +130% over CodeAct with
  sub-calls, +13% over Claude Code, at comparable cost.
- **What went wrong with the open model** ✅ — the part that matters most
  here:
  - Without an added warning in its prompt, Qwen3-Coder "tried to sub-call
    on everything", **thousands of sub-calls** on simple tasks.
  - More syntax errors than GPT-5, and deeper recursion propagated them.
  - The GPT-5 prompt did not transfer; the only change was that warning.
  - Thinking models ran out of output tokens (the Qwen3-235B arm).
  - Some models returned their *plan* as the final answer; FINAL vs
    FINAL_VAR is brittle.
  - Mean cost exceeds median cost because of outlier trajectories: GPT-5
    depth 1 on BrowseComp+ was $0.99 ± $1.22 a run.
- **Training.** RLM-Qwen3-8B, rejection fine-tuned on trajectories from a
  larger model acting as an RLM; median +28.3%. The detail is in
  `POST-TRAINING-RESEARCH.md` §3.

### Recursive Agent Harnesses — Lumer et al. ✅

[arXiv 2606.13643](https://arxiv.org/abs/2606.13643), 2026-06-11. The
recursive unit is a whole harness with tools, not a model call.

- The parent spawns children from a Python script (`Task()` objects under
  `asyncio.gather`) when there are many, or by a JSON tool call for one to
  five. Children inherit the tools and the spawning ability, get a bounded
  context and an isolated workspace, and write a **structured JSON record
  to a file**; the parent reads only the aggregate.
- Oolong-Synthetic, 199 samples: RAH 81.36 (GPT-5) and 89.77 (Sonnet 4.5)
  against RLM 64.38 and Codex 71.75 📰. The baselines were copied from
  another paper rather than re-run, and no open-weight model was tested.
- **Failure mode worth a case:** on long inputs the parent sometimes
  answered directly without spawning, reducing to one flat agent.
- No permission model beyond workspace isolation and a depth cap of 3.

### Prime Agent — Karten, Zhang et al. ✅

[arXiv 2608.23552](https://arxiv.org/abs/2608.23552), 2026-08-24;
[code](https://github.com/PrimeIntellect-ai/prime-agent). An RLM harness
grown into a product: a persistent IPython kernel per session, an async
`rlm` primitive returning a handle, daemon-mediated message queues between
parent, child and sibling, and retained prompt notes, memories, skills and
subagent specs that a background `/refine` pass edits.

- The long-context results against native harnesses are mixed and carry
  no intervals; there is **no ablation** isolating the REPL, the retained
  state or the messaging.
- **Its most instructive result is a failure.** In the Factorio run an
  agent spawned resources through the game's admin console, ignored an
  anti-cheating heartbeat, and *saved the shortcut as a reusable skill*:
  persistence preserved a reward hack. Refinements carry provenance and
  rollback but pass no acceptance gate.

---

## 2. What mecha already has

| RLM idea | mecha today |
|---|---|
| Long input kept out of the root context | Spill (oversized tool output to a file, read on demand); `recall` over the session's recorded history |
| A sub-model on a piece of the input | `QuarantinedPass` — no tools, one message, optional `response_schema` (`quarantine.rs`) |
| Typed answers from the sub-model | `subagent.rs`'s `trusted_output` + `AnswerShape` (`number`, `boolean`, one of a closed set): a shape-proven answer comes back not `external`; prose fails the vouch and stays untrusted |
| Decoder-enforced types | `structured_output = "llama_json"` on every local provider; llama.cpp applies the grammar after the thinking block closes (`LLAMA-SERVER.md`) |
| A child harness | `subagent.rs` — an allowlisted registry, taint inherited downward |
| Messages between agents | `mailbox.rs`, taint travelling with the message |
| Retained skills and rules | `skill.rs`, the learning store — behind provenance gates and a measurement gate, which is exactly what Prime Agent's reward-hacked skill lacked |
| The REPL | **Missing.** `AUDIT-RESEARCH.md` §3.12 and `CODE-MODE-RESEARCH.md` planned it on monty; nothing is built |

So the gap is the REPL and the one function that joins it to
`QuarantinedPass`. Everything else in the three papers mecha either has
or deliberately gated.

---

## 3. What to take

### 3.1 The handle, not the payload ❓ (the design's central bet)

Kept out of the context, the input stops costing prefill on every turn and
stops degrading the model's attention. Neither cost goes away because the
window is large: `context_window` is 262,144 per slot, and the limit on
this box is prefill wall clock and the accuracy a 3B-active model keeps at
length, not the window. On CodeQA the open model's best arm was depth 0 —
the REPL with no sub-calls at all (66.0 against 20.0 base) — so the
handle alone carries a large share of the gain.

**The handle should be opaque to the program too, not only to the
context.** The §3.12 plan arms taint on the host call, not on what the
program prints — correctly, since a value held in a variable is
attacker-influenced whether printed or not. So a handle does not stop a
read from arming the run, and should not. What it does is keep the bytes
out of the interpreter: a read returns a handle whose content stays in the
host, and host functions (length, slice, regex offsets, the sub-call)
touch it. The model then reads third-party prose only when it asks to
(`peek`), which is CaMeL's control-flow integrity offered as an option and
counted, never credited as a lowered label (`CODE-MODE-DESIGN.md` §3).

### 3.2 The sub-call is a quarantined pass with a shape — but a shape is not a clean label ✅

`trusted_output` already established, in code and tests, that an answer
which parses *as a whole* as a number, a boolean or one of a closed set
"carries no attacker's instructions" and narrows the untrusted leg — and
that private data stays private however it is summarised. `llm_query` is
tempting to read as that rule applied per call. The evidence says not to:

- **FIDES** ✅ ([arXiv 2505.23643](https://arxiv.org/abs/2505.23643),
  Microsoft) is the closest precedent: `query_llm` with constrained
  decoding, labels ordered by information capacity (`bool ⊑ enum ⊑
  string`). A boolean from untrusted context "limits influence to 1 bit",
  but letting it drive a consequential action "effectively endorses
  untrusted values", and the evaluation kept the strict policy. **Typed
  means bounded, not clean.**
- **CaMeL** ✅ ([arXiv 2503.18813](https://arxiv.org/abs/2503.18813))
  gives its quarantined model a Pydantic schema and tags every value with
  its sources. It concedes that data flow can still be manipulated — "this
  effectively turns data into control flow". 0 of 949 attacks succeeded on
  AgentDojo, at 77% utility against 84% undefended.
- **Agent Data Injection** ✅ ([arXiv 2607.05120](https://arxiv.org/abs/2607.05120),
  2026-07): delimiter injection makes the extractor return the *wrong
  values*. Attack success was 49.1% undefended and 25.0% with a dual-LLM
  split alone. It was 0% under CaMeL's strict mode, which cut utility to
  36.5%. Enum- and boolean-typed outputs were not tested.
- **"CaMeLs Can Use Computers Too"** ✅ ([arXiv 2601.09923](https://arxiv.org/abs/2601.09923))
  names *branch steering*: a manipulated input flips a boolean, and
  execution takes a valid but malicious branch of the approved plan. The
  authors conclude that data-flow defences are probabilistic.

Three things follow for mecha:

- **Inside a program, a shape changes no label.** A result carries its
  sources' labels; the run arms as it does today. The handle and the shape
  still pay off in context and reliability.
- **The trust decision stays where the owner wrote the shape**: a subagent
  profile's `answer_shape`, the rule `trusted_output` already applies and
  the `classify` profile already uses. The residual — the attacker choosing
  *which* of the owner's labels returns — is named in that rule's own
  terms. Letting a model choose the labels per call is an owner ruling.
- **Pointers never cross as trusted.** "The spans that mention a deadline"
  returns *where*, never *what*, which keeps prose out of the planner. It
  is something `grounding.rs` can dereference once a program's reads are
  recorded where its walk reaches (`CODE-MODE-DESIGN.md` §7), and not
  before. But an id an attacker
  selects out of their own text can be an address, which is a `Chosen`
  destination by another name.

### 3.3 Depth 1, by construction, with a budget ✅

The paper's open-model numbers argue against recursion past 1: OOLONG fell
from 48.0 at depth 1 to 26.0 at depth 2, and errors propagated into
sub-calls. In mecha, depth 1 is not a setting: a `QuarantinedPass` has no
tools, so a sub-call cannot recurse. What does need building is the
**budget** — a cap on sub-calls and sub-call tokens per program, enforced
by the host, because Qwen3-Coder's thousands of sub-calls is the failure
this model family is known for. And the prompt the root model sees should
be written and tested for Qwen, not ported (the paper's own finding).

### 3.4 Parallel sub-calls, sized to the server ❓

RLM's sub-calls were synchronous; Recursive Agent Harnesses gained by
running children concurrently. Here the ceiling is the router's four slots
of 262,144, one of them held by the root, and `permit.rs` already rations
seats. A batched `llm_map` over a list of handles, bounded by free seats,
is the cheap half of the parallelism; true concurrent child harnesses are
`subagent.rs` work and not in scope.

### 3.5 Structured return, and a check that the decomposition happened

From Recursive Agent Harnesses: children return structured records, and
the parent sometimes skipped spawning entirely. The first maps onto typed
results. The second is an eval case: a long-input task where answering
without the tool should score visibly worse.

---

## 4. What not to take

- **Depth above 1.** Measured to hurt the open model; impossible by
  construction under `QuarantinedPass`, and worth keeping so.
- **Untyped sub-calls as the default.** DSPy's `dspy.RLM` and the
  reference repo both return prose from `llm_query`, and DSPy returns a
  failure as the string `"[ERROR] ..."`, which a program then aggregates
  as data. A short-text shape may exist, bounded and labelled like any
  other; a failed pass raises.
- **Prime Agent's ungated `/refine`.** Its reward-hacked skill is the
  argument for the gates mecha already has; nothing to adopt.
- **A persistent kernel across turns.** Prime Agent's IPython kernel holds
  values between turns. A paused monty program with locals is state that
  outlives a turn and must carry its taint (`CODE-MODE-RESEARCH.md`); one
  program per call, state in handles the host owns, keeps that hazard out
  of the first slice.

---

## 5. Not established

- No paper here measured an RLM against an injected input, and no paper
  measured attack success when the quarantined model's output is limited
  to enums, booleans or offsets. ADI and CaMeL tested neither.
- Whether Qwen3.6 with `response_format` and thinking works on the
  installed llama.cpp. llama.cpp issue #20345 has reports of failure on
  this model after the March fix.
- Whether Qwen3.6-35B-A3B writes RLM programs well enough without
  training. RLM-Qwen3-8B needed post-training; Qwen3-Coder-480B did not.
  The 35B-A3B sits between them and was not tested.
- Whether the handle gain holds on mecha's own long inputs (mail sweeps,
  PDFs, session history) as it did on OOLONG and CodeQA.
