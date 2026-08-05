# Memory curation: what holds up, and what mecha should do about it

Research pass, 2026-08-05, prompted by the question "Claude Code has a memory
md file — what are the best practices for curating agent memory?" Three
parallel sweeps: the academic literature, production systems (Claude Code,
ChatGPT, Letta, Mem0, Zep, LangMem, Cursor/Windsurf), and curation-lifecycle
policies specifically.

**Venue key**: ✅ peer-reviewed · 📄 preprint · 📰 vendor/blog · 🔮 folklore
(no measurement exists anywhere).

> **Addendum, 2026-08-05 (same day): R1 and R2 shipped.** Rule
> identity/provenance/tenure (`Rule.id`/`sources`/`created_at`/`retired_*`,
> `finalize_rules` carrying identity across consolidations); the
> `validations.jsonl` ledger with regression **bisection** attributing a
> regressed probe to the single rule that flips it (user rules pinned in
> every arm, interactions and inconclusive arms attribute nothing); `mecha
> rules` (list with tallies / retire / restore / propose-retirements — the
> deterministic ledger scan staging `enabled = false` + `retired_*` through
> the existing proposal gate, wired into `ruminate.sh` after learn); and
> `mecha eval --ab-rules`, the paired-arm task-outcome A/B reporting flips
> as its own artifact. R3 has since shipped too: `MAX_ACTIVE_RULES_PER_DOMAIN`
> caps a domain at 15 *active* learned rules (user rules uncounted — they are
> the user's own budget), `budget_refuses` rejects any candidate set that grows
> past the cap, and `over_budget_domains` reports an already-over set. The size
> half, `RULES_CHAR_BUDGET`, stays a warning: over budget, `learn` keeps the
> set and says the next pass should consolidate harder. Still open: nothing yet
> retires on the R1 staleness signal — by design; see R2's "what this
> deliberately is not".

---

## The one-sentence answer

The field's best-evidenced findings are that **curation beats accumulation**,
that **extraction loses to verbatim retention**, that **the write path — not
input filtering — is where memory security lives**, and that **a rule store
nobody retires from goes negative**; mecha already implements the first three
structurally, and the fourth is the gap this doc proposes closing —
**rules get an identity and an outcome ledger, and retirement becomes a
proposal like any other rule change**.

---

## What the literature established

### Curation beats accumulation — the anchor result

✅ **"How Memory Management Impacts LLM Agents"** (ACL 2026, arXiv:2505.16067).
Agents exhibit *experience-following*: high input similarity to a retrieved
memory produces highly similar output, so stored errors compound and stale
experiences mislead. Selective addition + deletion gave **~10% absolute**
average gain over naive growth; a reported add-everything store of 2,400
records scored **13%** on medical reasoning vs **39%** with 248 curated
records. Their practical insight, and the cheapest credible evaluation method
in the field: **downstream task outcomes are free per-memory quality labels**
— grade a memory by whether the tasks that retrieved it succeeded.

### Self-growing rule/skill libraries drift negative without retirement

📄 **"Library Drift"** (arXiv:2605.19576, 2026). Voyager-style self-growing
skill libraries measured *below* the no-skill baseline: LLM-authored skills
contributed **+0.0pp** where human-curated gave **+16.2pp** — near-duplicates
and stale entries degrade retrieval precision, then stale entries silently
misdirect the solver. Their fix ("ratchet recipe"): **outcome-driven
retirement** (retire an entry once enough trials show negative contribution),
a **hard cap** on active entries (~50) with eviction, and authoring priors —
lifting pass@1 from 0.258 to 0.584. The counterweight: retiring too
aggressively (small minimum-trial thresholds) *hurt*. 📄 A companion result
(arXiv:2606.15017) found budget-constrained agents often never recoup a
memory module's token cost at all — riding in every prompt is not free.

This is the failure mode a learned-rules store is exposed to by construction:
every rule rides in every future system prompt, inside the cached prefix, and
nothing measures whether it is still earning its tokens.

### Extraction is lossy; keep the verbatim record

📄 **"Verbatim Chunks Beat Extracted Artifacts"** (arXiv:2601.00821, 2026):
controlled ablation; verbatim transcript chunks consistently beat LLM-extracted
facts/summaries downstream. The accuracy ranking wherever history fits in
context: **full context ≥ RAG-over-verbatim > extraction pipelines** — Mem0's
own paper shows its full-context baseline beating its memory system on
accuracy (📄 arXiv:2504.19413; the memory layer buys ~90% token cost and
~91% p95 latency, not accuracy). Structure belongs *on top of* the raw
record, never in place of it.

### Stale memories are the worst distractors, not just waste

📰→✅ **Chroma's context-rot study** (18 frontier models):
semantically-similar-but-wrong content harms far more than unrelated bulk, and
distractors compound. A superseded fact is by definition maximally similar to
the query and wrong about current state — evicting it is **damage removal,
not compression**. Corollary for always-loaded memory: keep it brutally
small; adherence degrades with size, and contradictory instructions get
resolved arbitrarily (📰 Claude Code's own docs say both).

### Invalidate, don't delete

📄 **Zep/Graphiti** (arXiv:2501.13956): bi-temporal edges — event-time
validity (`t_valid`/`t_invalid`) beside system time. A contradicting fact
*invalidates* the old edge rather than deleting it: history stays queryable,
and a fallible classifier's mistake is recoverable. The alternative —
Mem0's LLM choosing ADD/UPDATE/DELETE/NOOP — is simpler and destructive;
LongMemEval's two most-failed categories (knowledge updates, temporal
reasoning) are exactly what destructive updates make unanswerable.

### Consolidation belongs in the background

📄 **Sleep-time compute** (Letta + Berkeley, arXiv:2504.13171): a background
agent rewriting memory during idle time gave **~5x** test-time compute
reduction at matched accuracy, up to **+13–18%** accuracy where queries are
predictable — and better-curated memory, because consolidation gets dedicated
compute instead of being squeezed between user turns. 📰 LangMem ships both
modes and states the tradeoff plainly (hot-path taxes latency and the agent's
attention; background delays availability).

### Memory is a privileged injection channel; defend the write path

The founding incident is **SpAIware** (📰 Rehberger 2024, fixed by OpenAI): a
prompt injection from a fetched page wrote persistent instructions into
ChatGPT's memory — exfiltration across all future sessions, surviving chat
deletion. 📄 **MINJA** (arXiv:2503.03704): >95% injection success with
*query-only* access. 📄 The systematic study (arXiv:2606.04329) is the one to
remember: across 6 attack classes, prompt-injection detectors collapse from
**84% to 42.5%** on *weak-signal* attacks — poison that reads as legitimate
content (repeat a claim until the summarizer deems it important). Conclusion:
**defenses must target the storage decision, not input anomalies** — write-time
admission control plus provenance binding, with untrusted-origin entries
structurally excluded or demoted. Almost no published system implements this.

### Evaluation: trust outcomes, not vendors

The Zep↔Mem0 LoCoMo fight (claims of 84% vs corrected 58.44% vs
counter-corrected 75.14%, plus a ~6% label-error rate found in the benchmark
itself) established that **vendor memory benchmarks are not comparable
experiments**. ✅ **LongMemEval** (ICLR 2025, arXiv:2410.10813) is the trusted
probe suite, and its *knowledge-update* and *abstention* splits are the
discriminative ones. The cheapest credible method remains a task-outcome A/B
with and without the memory — which is the verification doc's conclusion
wearing a different hat: everything a memory system says about its own
memories is hearsay; grade the artifact.

### Where production systems converged

Claude Code, Cursor, Windsurf, and Anthropic's memory tool independently
landed on the same shape, worth naming because it validates mecha's:

- **A small always-loaded index + on-demand depth** (MEMORY.md loads its
  first 200 lines / 25KB; topic files read on demand; Letta pins size-capped
  blocks over a searchable archive).
- **Two trust tiers**: human-written config (version-controlled, reliable)
  above model-written memory (local, best-effort). Windsurf's docs say it
  outright; Cursor requires user approval to promote a memory into a rule.
- **Files over embeddings for anything curated**: markdown diffs, git
  history, and review queues make audit and poisoning-detection tractable;
  you cannot diff a vector store. Claude Code stamps `modified` timestamps
  on memory files as a staleness signal.
- The cautionary contrast is **ChatGPT's implicit dossier**: a standing
  summary injected invisibly into every chat — zero user effort, but wrong
  inferences persist uninspectably and output stops being reproducible
  (Willison's critique). Opacity is also what made SpAIware possible.

---

## Audit: mecha against the field

Most of the field's advice is already implemented here — in several cases
before the papers naming it. Stating this explicitly so the recommendations
that follow are read as the delta, not a rebuild:

| Literature finding | mecha today |
|---|---|
| Write-path admission control + provenance binding (2606.04329's consensus defense) | `Origin: clean/untrusted/derived`, classified deterministically from recorded taint, fail-closed, no knob. `learnable()` is structural exclusion |
| Queue-before-belief for extracted knowledge | `mecha distill` stages episodes as *evidence*; pkg's review queue holds candidates; mecha reads pkg back as untrusted |
| Keep the verbatim record; structure on top | Sessions are append-only JSONL, never replaced by their episodes; compaction summaries live in the transcript, the file keeps everything |
| Evict superseded facts as damage removal | `evict_superseded_results` runs before any summary, citing the same distractor literature |
| Small always-loaded surface + on-demand depth | Rules block in the system prompt; episodes behind pkg tools |
| Two trust tiers, human above machine | `*.user.toml` is never written by any pass; learned rules are a separate file, behind a gate |
| Human approval before memory becomes belief | The proposal gate: unattended learning proposes, `learned.toml` changes only on acceptance |
| Background consolidation, not hot-path | `reflect`/`learn` are offline passes, not in-run extraction |
| Outcome measurement over self-report | `mecha validate`'s counterfactual probes; trace-graded for steers/denials, judge only where unavoidable |
| Files, git-versioned, humanly editable | The learning store: TOML/JSONL, advisory flock, `commit()` after passes |

**What the audit does *not* find: any lifecycle after acceptance.** A rule
that clears the gate is immortal. Concretely, `Rule` is four fields — `text`,
`enabled`, `confidence`, `based_on_count` — so today a rule:

- has **no identity** (rules are matched by text position in a TOML array)
  and **no provenance** (no link back to the reflexions it came from, though
  `Proposal.reflexion_ids` records it at the batch level and then loses it);
- has **no timestamps** — nothing distinguishes a rule learned yesterday from
  one learned before a refactor made it wrong;
- is **never re-measured**. `mecha validate` probes the *whole block* with
  and without, on demand, and prints a summary that goes nowhere. There is no
  per-rule attribution, no ledger, no retirement path but a human noticing;
- accumulates without bound. `rules_prompt_block` concatenates every enabled
  rule in every domain into the cached prefix, with no cap and no warning.

This is precisely the library-drift setup: LLM-authored entries, always
loaded, additions gated but tenure unexamined. The drift paper's measured
endpoint for that configuration is a store that costs tokens and contributes
nothing — or worse, since a stale rule is a semantically-similar-but-wrong
distractor riding in *every* prompt.

---

## Recommendations

Ordered by value. R1–R2 are the substance; R3–R4 are small; R5 is a list of
things the literature says *not* to build, so the backlog doesn't reacquire
them.

### R1 — Rules get an identity, provenance, and tenure fields

Extend `Rule` (all optional-with-default, so existing TOML files load
unchanged, same trick as `Reflexion.origin`):

```toml
[[rules]]
id = "r-2026-08-05-a3f1"          # minted at proposal acceptance
text = "..."
enabled = true
confidence = 0.8
based_on_count = 3
sources = ["refl-...", "refl-..."] # the reflexions this rule consolidates
created_at = "2026-08-05T..."
retired_at = ""                    # set instead of deleting — see R2
retired_reason = ""
```

Rationale: identity is what an outcome ledger keys on; `sources` completes
the provenance chain (today it dead-ends at the proposal batch — an
untrusted-origin audit of a *live rule* currently requires archaeology);
`created_at` is the staleness signal every production system converged on
(Claude Code's `modified` stamp). Retirement-as-field rather than deletion is
Zep's invalidation argument at TOML scale: the git history already preserves
bytes, but a retired rule with a reason is *evidence* — the learner's frame
can be told "this was tried and measured harmful; do not re-derive it," which
a deleted line cannot say. (The loop-until-dry lesson in another costume:
dedup against everything *seen*, not everything *kept*, or rejected entries
reappear every round.)

### R2 — An outcome ledger, and retirement through the existing gate

The main recommendation. Two evidence sources feed one ledger
(`validations.jsonl` in the learning store, same conventions as `runs.jsonl`):

1. **`mecha validate` writes down what it measures.** Today's probe results
   (improved/regressed/unchanged per reflection, with/without the block) are
   printed and discarded. Append them, keyed by rule-set content hash and
   probe id. Zero new measurement — just stop throwing it away.
2. **Per-rule attribution, lazily.** Full leave-one-out ablation is N× the
   probe cost — don't. Attribute on suspicion: when the whole-block arm
   regresses a probe, a second pass bisects *which* rule flips it (the probe
   machinery already renders candidate rule sets rather than the store —
   `domain_rules_section` takes slices — so ablated blocks need no new
   rendering path). Accumulate per-rule tallies in the ledger.
3. **Eval as the coarse A/B.** `mecha eval` rightly forces learned rules off
   — a scorecard shaped by local rules grades the machine, not the model. Add
   an explicit paired mode (`mecha eval --ab-rules`) that runs the same cases
   both arms and reports the delta *as its own artifact*, never as the
   comparable scorecard. This is the literature's "task outcomes are free
   quality labels," bought with machinery that already exists; pass^k even
   covers the reliability half.

**Retirement policy**, applying the drift paper's ratchet with its own
caveat: a rule with ≥N ledger observations (N generous — their small-N
variant *hurt*) and net-negative contribution becomes a **retirement
proposal** — `enabled = false`, `retired_at`, `retired_reason` naming the
measurements — flowing through the same `Proposal` gate and human acceptance
as any other rule change. No new machinery, no autonomous self-editing: the
hyperagent gate's rule (a self-improvement loop must never apply its own
output) covers removal exactly as it covers addition. `mecha rules` (or
`learn status`) surfaces the pressure: per-rule tallies, never-validated
rules, rules older than their last validation by months.

What this deliberately is **not**: automatic decay. Usage-based eviction's
canonical failure is the rarely-retrieved fact that must never expire (the
literature's example is a penicillin allergy; ours might be "never force-push
to main"). Low usage is a review signal; only *measured harm* argues for
retirement, and a human accepts the argument.

### R3 — Cap the always-loaded block

**Shipped.** The count half landed as `MAX_ACTIVE_RULES_PER_DOMAIN = 15` with
`budget_refuses`; the paragraph below is the original proposal, kept for the
reasoning behind the number.

A per-domain budget on `rules_prompt_block` (count or tokens; the drift
paper capped at ~50 entries — for a system prompt, 15–20 per domain is
likely nearer the adherence cliff). Over budget: warn at startup (the
routed-name-matches-no-tool precedent) and refuse *additions* at the
proposal gate until consolidation shrinks the set — the learner already
proposes whole rewritten rule sets, so "merge before you may add" is a frame
instruction plus a gate check, not new plumbing. This is Claude Code's
MEMORY.md limit-and-nudge loop, applied to the artifact mecha actually
loads every run.

### R4 — One scheduled consolidation cadence

The pieces exist as manual commands; the sleep-time result says running them
off the hot path *with dedicated compute* is the right architecture — which
is already true here; what's missing is only that they run regularly and in
order: `reflect` (new sessions + outbox edits) → `validate
--unprocessed-only` (feed the R2 ledger on held-out reflections *before*
learn consumes them) → `learn` (propose: consolidations, additions,
retirements) → `distill`. A cron/systemd-timer job and a short doc section;
proposals wait for the human regardless, so the cadence changes when
evidence accumulates, never who decides.

### R5 — What the literature says not to build

- **No vector/embedding memory store.** The curated artifacts are small
  enough to load or grep; embeddings would make review, diff, and
  poisoning-audit opaque — the exact properties the security literature says
  to keep. pkg is the semantic layer, behind its review queue.
- **No extraction pipeline replacing transcripts** (Mem0-shape). Verbatim
  beats extracted; sessions stay the ground truth; episodes stay evidence.
- **No LLM-adjudicated destructive DELETE.** Retirement is a gated,
  evidenced, reversible flag — never a classifier's unilateral erasure.
- **No implicit profile injection** (ChatGPT-dossier-shape). Everything that
  rides in the prompt stays a file the user can open, diff, and blame.
- **No auto-decay/TTL on rules** (R2's rationale), and no importance scores
  from the model at write time — LLM-rated importance is the Generative
  Agents mechanism everyone copied and nobody validated; mecha's
  `confidence` field should not grow a policy on top of it. Measured
  contribution is the only score that has evidence behind it.

---

## Sources

Anchors: arXiv:2505.16067 (curation beats accumulation, experience-following)
· arXiv:2605.19576 (library drift, retirement ratchet) · arXiv:2601.00821
(verbatim beats extraction) · arXiv:2606.04329 (write-path defense; detector
collapse on weak signals) · arXiv:2501.13956 (Zep bi-temporal invalidation) ·
arXiv:2504.13171 (sleep-time compute) · arXiv:2410.10813 (LongMemEval) ·
Chroma context-rot (trychroma.com/research/context-rot) · SpAIware
(embracethered.com, 2024) · arXiv:2503.03704 (MINJA).

Context: arXiv:2310.08560 (MemGPT) · arXiv:2304.03442 (Generative Agents) ·
arXiv:2305.16291 (Voyager) · arXiv:2502.12110 (A-Mem) · arXiv:2504.19413
(Mem0) · arXiv:2309.02427 (CoALA taxonomy) · arXiv:2602.06052 (2026 survey) ·
arXiv:2601.18642 (FadeMem, forgetting helps) · arXiv:2606.15903
(control-plane forgetting) · arXiv:2606.15017 (module token costs) ·
Claude Code memory docs (code.claude.com/docs/en/memory) · Anthropic memory
tool + context management (platform.claude.com, claude.com/blog) · Letta
memory blocks / sleep-time (letta.com) · LangMem (langchain.com) · Cursor
rules (cursor.com/docs) · Windsurf memories (docs.devin.ai) · Zep↔Mem0
LoCoMo dispute (github.com/getzep/zep-papers/issues/5, blog.getzep.com).
