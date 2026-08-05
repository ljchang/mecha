# Context management: what is established, what is fashionable

Research pass, 2026-08-04, prompted by a session dying on
`exceed_context_size_error`. Four threads: vendor guidance, what real harnesses
implement, the long-context evidence base, and summarization fidelity.

Written down because most of it contradicts something plausible — including
things this project already believes.

**Evidence keys**: [PAPER] peer-reviewed · [PREPRINT] arXiv only · [SRC] read
from source · [DOCS] official docs · [BLOG] prose with no controlled
evaluation.

---

## The four findings that should change decisions

### 1. Relatedness, not volume, is what does the damage

This is the best-evidenced section of the whole review and it reframes the
problem. Irrelevant content is nearly free; **semantically-related but wrong
content is catastrophic.**

- **Cuconasu et al., SIGIR 2024** [PAPER]: with the gold document always
  present, adding 18 *retriever-ranked, topically related, answer-free*
  distractors takes Llama2-7b from 0.5642 to **0.1795 (−68%)**. A single one
  costs 24%. Meanwhile **random unrelated documents are neutral to helpful**
  (+35% in a realistic setting), and nonsensical random-word documents helped
  *more* than random Wikipedia. Killer detail: swapping in a *better* retriever
  (trained with hard negatives) made RAG output **worse**.
- **Wu et al., COLM 2024** [PAPER]: a single distractor's misrepresentation
  rate — unrelated 5.5%, partly related 10.0%, **highly related 22.5%**
  (GPT-3.5). **4× more likely to be misled by a related distractor.**
- **Shi et al., GSM-IC, ICML 2023** [PAPER]: one irrelevant sentence added to a
  math problem drops CoT from 95.0 to 72.4. The number that matters is
  **macro accuracy — solving a problem correctly under *every* distractor
  variant — which is 6.0%.** Single-run scores look fine while consistency is
  destroyed. Nearly-free mitigation: the instruction *"feel free to ignore
  irrelevant information"* buys **+5.4pp micro and 2.5× macro**.

**What this means for mecha.** A superseded file read left in context after an
edit is *exactly* the near-miss shape — same path, same symbols, wrong content.
That is the 25–68% case, not the "wastes tokens" case. **Staleness-aware
eviction beats size-aware compaction**, and it is strictly safer: you delete
content you know to be false instead of paraphrasing content you know to be
true. Cline implements this and gates on it — if a lossless dedup pass alone
saved ≥30% of characters, it **skips truncation entirely that turn**. forge
dedupes by operation target, keeping only the latest op per file/command/URL.

Corollary: **size-triggered thresholds fire far too late for this problem.**
FLenQA (ACL 2024) [PAPER] puts measurable damage at **~500–3,000 tokens** —
about 2% of a 128k window — and Databricks measured Llama-3.1-405B, a 128k
model, **peaking at 4–16k**. Compaction-at-a-fraction addresses *capacity*.
Distraction is a *content* problem and needs its own mechanism.

### 2. Reliability, not accuracy, is the metric — and multi-turn is where it dies

**"LLMs Get Lost In Multi-Turn Conversation" (Laban et al., ICLR 2026
Outstanding Paper)** [PAPER] is the strongest credential in this review: 15
LLMs, 8 families, 6 tasks, 200,000+ simulated conversations. Same information
delivered across turns instead of all at once:

- **−39% average performance**, decomposed as **aptitude −16%, unreliability
  +112%**. Models vary ~50 percentage points between their best and worst run
  of the *same* instruction.
- **Tool-calling degrades hardest of the six tasks**, and worst for the models
  that lead single-turn: Gemini 2.5 Pro **97.8 → 36.3**, Claude 3.7 Sonnet
  **95.4 → 33.3**.
- Two controls make it real: CONCAT (same shards, one turn) retains 95.1%, so
  it is not information loss; and a 7th *episodic* task (translation) shows
  **zero** degradation. The predictive properties are generative, shardable,
  and **non-decomposable** — the shape of agent work.
- **Two shards is already enough** to trigger it. Reasoning models degrade
  identically. Temperature 0.0 still leaves ~30% unreliability.
- Recapping helps but does not rescue: GPT-4o **93.0 full → 59.1 sharded →
  76.6 with recap**. The authors' own advice is to **start a new conversation
  restating what matters** rather than continue a lost one.

Related: **τ-bench**'s pass^k (all k trials succeed) versus pass@k — GPT-4o
**61.2% pass^1 → <25% pass^8**. And **self-conditioning** (ICLR 2026) [PAPER]:
injecting synthetic *error* histories causes **20–30pp drops at turn 100**, and
it does not go away with scale — a model with its own past mistakes in context
becomes likelier to repeat them.

**What this means for mecha.** Every single-run scorecard in `results/` is
measuring the wrong thing. Running the eval set k times and reporting pass^k
is a cheaper and larger intervention than anything about compaction — and the
handoff already flags pass@k as wanted. It also argues that mecha's
`/clear`-and-restate path deserves promotion to a first-class recommendation,
not a last resort.

### 3. Compaction beats truncation, and loses to keeping history

The cleanest ablation, same model and tools throughout (**Context-Folding**)
[PREPRINT]:

| Strategy | BrowseComp-Plus | SWE-Bench Verified |
|---|---|---|
| Truncate (32K) | 0.286 | 0.436 |
| Summarize (32K × 10) | 0.386 | 0.488 |
| **Keep everything (327K)** | **0.478** | **0.552** |

Summarization buys +10.0 / +5.2 points over truncation and gives up
**−9.2 / −6.4** against not compacting at all. **ACON** (ICML 2026) [PAPER]
measures the same shape: naive prompted summarization takes an AppWorld agent
**56.0% → 43.5%**.

Three refinements that matter more than the headline:

- **Omission dominates, 3–30× over hallucination.** PolyTope (EMNLP 2020)
  [PAPER]: omission 47.9% of errors, extrinsic fabrication **0.93%**. Entity
  recall 75.4% against source precision 99.4%. **For a coding agent this is
  the worst possible failure profile** — a summary will not invent a file
  path, it will silently drop one.
- **Incremental summarization is ~2× worse than hierarchical** on every
  omission category (BooookScore, ICLR 2024 [PAPER]: entity omission 7.30 vs
  3.71). **mecha's compaction is incremental** — the weaker arm.
- **Validating the compaction is the best cost/benefit in the review.**
  Slipstream (Princeton) [PREPRINT]: a trajectory-grounded judge rejects
  **5.4–8.5%** of candidate summaries on SWE-bench Verified, **~90% of
  compaction failures are omission**, and validation gains **+6.4 to +8.8
  points at <1% latency overhead**. Errors surface fast — 60% of first
  manifestations at k=1.

And the domain split is consistent: **summarization helps search and hurts
coding.** ACM measured BrowseComp-Plus 0.570 → 0.608 while SWE-Bench went
0.489 → 0.475. Prose destroys precise state; page text is disposable.

**Externalize instead.** VISTA (ICML 2026) [PAPER] on LOCA-bench: ReAct 22.7%
→ summarization 29.3% → Claude Code 42.7% (6.72M tokens) → **lossless archive
with on-demand recovery 50.7% at 2.86M tokens** — better accuracy at 43% of
the tokens. On the strongest model in a separate LOCA-bench run, **compaction
scored *below* doing nothing** (GPT-5.2 38.7% baseline vs 36.0% compacted).

### 4. Where the tokens actually are

**"Token Reduction Is Not Cost Reduction"** (arXiv:2607.12161) [PREPRINT],
2,908 provider-billed runs:

| Billed cost | Share | | Addressable surface | Share |
|---|---|---|---|---|
| Cache creation | 44.3% | | **System prompt + tool definitions** | **74.7%** |
| Cache reads | 35.4% | | Hidden thinking + residual | 19.4% |
| Output | 10.4% | | **Tool outputs** | **3.3%** |
| Uncached input | 1.3% | | Other | 2.3% |

One arm cut tool-output tokens **38.4%** and cost **6.8% more**. Correlation
between output reduction and cost saving: **r = 0.154**, i.e. none. *"Each
added turn re-transmits the cached prefix"* — trajectory length dominates.

**Tool-surface size is separately quantified, and it is large:**

- **LongFuncEval** (IBM) [PREPRINT]: scaling a tool catalog 8K → 120K tokens
  costs GPT-4o **10.6–13.8%**, Llama-3.1-70B **43.6–72.1%**.
- **Meta** [PREPRINT]: with the correct tool present *either way*, showing
  **fewer** candidates (K=2.2 vs K=5) is worth **+6pp overall, +16pp on
  medium-difficulty** queries.
- **Enterprise routing** [PREPRINT]: even when the right tool is guaranteed in
  the shortlist, the ceiling falls **79.0% → 68.8%** from semantically
  overlapping alternatives alone.
- **RAG-MCP** [PREPRINT]: 13.62% → **43.13%** by retrieving to top-1 instead
  of putting every schema in the prompt — *with prompt tokens falling*.

**mecha has 31 tools and caches tools+system but never the messages.**

---

## What every harness does (and mecha does not)

Surveyed with source access: Claude Code, Codex CLI, Cursor, Cline, Roo, Aider,
SWE-agent, OpenHands, Goose, Zed, Gemini CLI, OpenCode, Amp, Amazon Q, plus the
Rust ecosystem.

**Universal**: a hard cap on tool results *with a marker naming the recovery
command*; compaction at a fraction of the window **clustered at 0.8–0.9**
(mecha's two-thirds is the most conservative surveyed); head + recent tail kept
with **recent *user* turns preserved verbatim** (Codex 20k tokens, Zed 80 KB —
two teams landing on the same number independently); subagents with a fresh
window returning only a summary, nesting capped at 1; tool-pair legality across
any cut; a context gauge in the UI.

**Rising fast — "spill to a file, hand the model a path."** Claude Code (bash
over the limit writes to a file and returns path + preview), Goose (200k chars
→ temp file), Codex (hook spill), Zed (over 16 KB `read_file` returns a
*symbol outline with line numbers* and says "do NOT retry without line
numbers"), Cursor (**measured 46.9% fewer agent tokens**). OpenHands has the
most refined form: path **plus the line number where elision begins**.

**Rare and worth stealing:**

- **Zed's threshold is a sum type** — `Percentage | TokensUsed |
  TokensRemaining`. *"Compact when 30k tokens remain"* survives a model swap in
  a way *"compact at two-thirds"* does not. Zed also **refuses to auto-compact
  below an 80k window** at all, offering "start a new thread" instead.
- **Codex's baseline-adjusted gauge**: subtract a `BASELINE_TOKENS = 12000`
  reserve from **both** numerator and denominator, so the bar reads 100% after
  the first prompt instead of 94% — and the reserve explicitly includes room
  to *run* the compaction call.
- **Codex exposes context state to the model as tools**: `get_context_remaining`
  → `{"tokens_left"}` and `new_context` ("Start a new context window. Does not
  clear, reset, or otherwise affect environment state."). The model can check
  its own fuel and declare bankruptcy without paying for a summary.
- **Amazon Q divides the tool-output byte budget equally across all results in
  a turn**, so one runaway tool cannot starve its siblings — directly relevant
  to a harness that dispatches tools concurrently, as mecha does. Q also sets
  client caps at **half** the real service limit, and rounds token estimates to
  the nearest 10 *"to avoid giving users a false sense of precision."*
- **Deterministic fallback when the summarizer itself fails**: Cline forces its
  non-LLM strategy on overflow *"so recovery never depends on another
  successful LLM call"*; Goose retries dropping 0/10/20/50/100% of tool
  responses.
- **Compaction as a projection over an append-only log**: OpenHands appends a
  tombstone and replays; Cline keys edits against a pristine on-disk copy so
  **checkpoint restore can undo a context edit**. Relevant to mecha's
  append-only JSONL.
- **Goose's structured summary** — a typed nine-field schema, **lists ordered
  most-important-first so consumers can cut from the tail**, and deliberately
  lenient deserialization so one malformed field cannot discard a good summary.

**The Rust ecosystem is mostly a negative result.** `rig`, `swiftide`,
`Kowalski`, `AutoAgents` and the official `rmcp` SDK have no tool-output caps
and no compaction; rmcp's reference example sends `messages.clone()` every
turn. `swiftide` has a `ChatMessage::Summary` variant that nothing in the crate
ever creates. Only **forge** and **stakpak** are real. **mecha is already ahead
of nearly all of them.**

---

## Long-context degradation: the numbers

- **NoLiMa** (ICML 2025) [PAPER] — effective context is 4–60× smaller than
  advertised. **11 of 13 models below half their short-context baseline at
  32K.** GPT-4o 99.3 → 69.7; Claude 3.5 Sonnet's *effective length* is **4K**.
  o1 goes 99.9 → 31.1 on the hard variant. **mecha's window is 32768.**
- **Lost in the Middle** (TACL 2023) [PAPER] — GPT-3.5 at 20 documents:
  **53.8% with the answer mid-context vs 56.1% closed-book.** Worse than no
  context. A 16K version did not fix it.
- **RULER** (COLM 2024) [PAPER] — only half of 17 models claiming ≥32K
  maintain performance at 32K.
- **Chroma's Context Rot** [BLOG, good methodology] — 18 models, 194,480 calls;
  degradation is **cliff-shaped, not a gradient**, contradicting Anthropic's
  "performance gradient rather than a hard cliff." ⚠️ The circulating "30–50%
  drop" figures are third-party readings of unlabelled charts — **do not cite
  them**.
- **Monitors rot too** [PREPRINT, Anthropic] — a safety classifier reading a
  transcript drops **98.6% → 88%** recall on real attacks with 800K tokens of
  benign activity preceding, and **99.7% → 69%** on obvious injected ones.
  Anything mecha gates on an LLM reading a transcript — the eval judge, the
  reflector, `validate` — has reliability that decays with transcript length.

---

## Multi-agent: the burden of proof is on it

**Vendor case.** Anthropic reports **90.2%** over single-agent Opus 4 — on an
**internal, unreleased, LLM-judged** eval with no independent replication — at
**~15× the tokens of chat** (single agent ~4×). Their own variance
decomposition undercuts it: **token usage alone explains 80%** of BrowseComp
variance, and a model upgrade beat doubling the budget. They exclude coding
explicitly. Cursor: *"five subagents in parallel uses roughly five times the
tokens."*

**Independent case, better evidenced.** "The Illusion of Multi-Agent Advantage"
[PREPRINT] runs six MAS frameworks against a strong CoT-Self-Consistency single
agent: **MAS consistently underperform at up to 10× the cost** (SWE-Bench Lite
MAS 32–57% vs CoT-SC 57%). Tran & Kiela (Stanford/Contextual) [PREPRINT] argue
from the Data Processing Inequality — every handoff can only lose information —
and find single-agent matches or beats MAS **when reasoning tokens are held
constant**. Berkeley's failure-mode study [PREPRINT] annotates 1,600+ traces
into 14 failure modes and concludes *"simple fixes are still insufficient."*

**METR's scaffold comparison** [PAPER-adjacent] is the sobering one: Claude
Code beat a simple ReAct scaffold in only **50.7% of bootstrap samples** — a
coin flip — and raising the token budget **8M → 32M "barely changed"** the
measured time horizon. If context capacity were binding at frontier horizons,
4× should have moved something.

**Honest synthesis**: measured multi-agent wins concentrate in *read-heavy,
decomposable, parallelizable* work with independent subtasks. On write-heavy or
dependency-laden work both vendor camps and every independent evaluation agree
— **keep writes single-threaded**. Cognition's 2026 revision lands on exactly
that: *"multi-agent systems work best when writes stay single-threaded and
additional agents contribute intelligence rather than actions."*

For mecha specifically: subagents buy **context isolation and wall-clock
parallelism**, which are real and sufficient reasons. They do not buy quality
at a fixed token budget, and nothing here supports using them to *minimize
taint* — the tool allowlist is the security mechanism, not the fresh context.

---

## What is genuinely unmeasured

Worth stating out loud, because mecha is positioned to measure some of it:

1. **Degradation across N rounds of compaction.** A July 2026 survey states the
   gap explicitly: *"the repeated compaction that agents actually perform is
   almost never measured."* mecha's own `chain-total` result — 5/5 uncompacted,
   1/5 compacted, 4/5 with thinning — is better evidence than anything
   published, for this workload.
2. **Whether "don't do X" survives a summary.** No study measures
   negative-constraint retention. FRANK, SNaC and BooookScore have no negation
   category.
3. **Whether evicting the agent's own failed attempts beats summarizing them.**
   Self-conditioning measures 20–30pp of damage from error histories; Manus
   argues the opposite from experience, with no measurement.
4. **Whether compaction canonicalizes an early wrong turn**, converting a
   recoverable error into a permanent one.

Items 3 and 4 are cheap on mecha's existing rig with `compact_at_tokens` as the
independent variable, and would be novel rather than replications.

---

## Ordered implications for mecha

1. ~~**Staleness-aware eviction, before and above compaction.**~~ Built
   2026-08-05: `compact::evict_superseded_results`, run ahead of thinning at
   both compaction sites. Same `path` supersedes across tools (a write
   invalidates an earlier read of the file it changed); identical calls dedup
   otherwise; errors neither supersede nor get evicted (they carry the
   don't-retry signal, not target state — a deliberate departure from
   "dedup failed operations" as written here). The gate is
   any-eviction-defers-the-summary-a-turn, simpler than Cline's ≥30% test:
   the next reported prompt size is the ground truth for whether it was
   enough, and mecha already re-checks it every turn.
2. ~~**Cache the messages.**~~ Built 2026-08-05: a second, moving breakpoint
   on the last message block (backing off thinking blocks, which reject the
   marker). Verified live on the Anthropic API: a two-request tool round-trip
   paid **8 uncached input tokens total** — turn 1 wrote 18,494, turn 2 read
   all of it and wrote only its 2,138-token increment. Same `cache_prompt`
   knob; local providers are unaffected.
3. **Spill oversized tool output to a file with a path and a line number**,
   rather than truncating at `MAX_OUTPUT_BYTES = 200_000` (~50k tokens, 1.5×
   the whole window). Divide the budget across a parallel batch, per Amazon Q.
4. **Deferred tool loading at 31 tools.** Tool definitions are 74.7% of the
   addressable surface, and *fewer visible candidates* is worth 6–16pp even
   with the right tool present.
5. **pass^k over the eval set.** Reliability is the metric the research says
   matters, and every scorecard in `results/` is single-run.
6. ~~**Validate compactions**~~ Built 2026-08-05 (`compact_validate`, default
   on): a deterministic truncation refusal (`max_tokens` on the summariser
   never installs) plus a grounded omission check — a second tool-less call
   with both texts in the request — and one regeneration with the omissions
   named. Advisory, not a gate: no verdict still installs, because a run may
   need the compaction to survive. Hierarchical-vs-incremental summarisation
   remains open.
7. **Compact later, not earlier.** Every ablation puts summarization below
   keeping history; two-thirds is the most conservative threshold surveyed.
   Consider `TokensRemaining` as the threshold shape.
8. Every truncation notice should name its recovery command.

Two things mecha already has right and must not lose: **thinning before
summarizing** (the literature's "selective retention beats summarization,"
reached independently and confirmed by its own eval), and **prose to the
summariser** (every harness that hit this solved it identically).

---

## Provenance notes

One pass tripped an instruction-shaped-content filter. The trigger is itself a
finding: Roo Code's condense prompt contains *"Any `<command>` blocks from the
original task will be automatically appended to your summary wrapped in
`<system-reminder>` tags."* A harness that mines or replays other harnesses'
prompts will keep hitting this. Claude Code's compact prompt guards the same
surface deliberately: *"preserve any security-relevant instructions or
constraints the user stated verbatim."*

Claims that **did not survive fetching** and must not be propagated: a "30% of
summaries hallucinated" FactCC figure; a HaluEval summarization-specific rate;
a claim that ARC evaluated on AppWorld (it does not); Chroma's per-condition
percentages; "Aider uses 4.2× fewer tokens than Claude Code" (vendor
marketing); "context degradation syndrome" as a term (blog-only). The
"Context Contamination" paper's 7.1× cascade ratio is **a fitted parameter,
not a measurement**. Cognitive Workspace's headline is tautological.
