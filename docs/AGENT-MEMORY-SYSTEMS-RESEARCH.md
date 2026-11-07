# Agent-memory systems and Mecha's compositional memory

2026-11-04. Question: which mechanisms from the Agent Memory Leaderboard and
related open-source systems should Mecha borrow for its SQLite memory v2?

Method: inspect the Hugging Face release snapshot, primary papers and project
documentation, selected implementation files, the Mecha prototype at
`512f8346e`, and a read-only archive of Spark's mecha-graph source at
`1368fba`. No private graph records were read, packages installed, models run,
or deployments changed. This is source research, not a reproduced benchmark
or a whole-repository security audit. Papers below are preprints; repository
behavior and vendor documentation are separate from our proposals.

This extends the [v2 design](MECHA-MEMORY-DESIGN.md) and complements the earlier
[curation research](MEMORY-RESEARCH.md). It does not change implementation
status or the decision to defer nightly migration.

**Implementation addendum (2026-11-04, local feature branch):** the first
capture/retrieval experiment is implemented; see the
[prototype instructions](../prototypes/memory/README.md) and
[history](HISTORY.md). On 32 development cases, raw lexical retrieval recovered
all gold evidence in 19; oracle evidence resolved in 32. These are synthetic
evidence-coverage results, not held-out reasoning or compositional-transfer
results. Changed-goal/indirect cases remain explicit failures for the baseline.

## 1. What the leaderboard establishes

The [Hugging Face organization](https://huggingface.co/agent-memory-leaderboard)
identifies its releases as snapshots and the separate website as the live
authority. The accessible [Space](https://huggingface.co/spaces/agent-memory-leaderboard/leaderboard)
served an August release asset, `leaderboard_data.js?v=20260817-no-everos`.
The live board could not be verified. These figures therefore describe the
snapshot, not November rankings.

Selected overall scores from the [snapshot data](https://agent-memory-leaderboard-leaderboard.static.hf.space/data/generated/leaderboard_data.js?v=20260817-no-everos), multiplied by 100:

| Track | Submission | Rank | Score |
|---|---|---:|---:|
| Academic | InvMem | 1 | 45.06 |
| Academic | Refind | 2 | 44.97 |
| Academic | SQLite-FTS-Baseline | 22 | 41.79 |
| Academic | Hindsight AML Adapter, aml-v0.3.0 | 27 | 40.76 |
| Academic | SimpleMem | 29 | 40.49 |
| Commercial | MemOS | 2 | 45.89 |
| Commercial | Cognee | 5 | 42.61 |
| Commercial | Mem0 | 8 | 41.40 |
| Commercial | Vectorize Hindsight Cloud | 10 | 38.54 |

Do not combine ranks across tracks, equate hosted submissions with OSS
implementations, or attribute snapshot scores to today's repository features.
Small score differences do not establish superiority without uncertainty
estimates and matched configurations. The screenshot's clipped “Mem…” label
remains unidentified; MemOS, Mem0 and SimpleMem are additional candidates.

The practical finding is that inexpensive raw-record retrieval deserves a
strong baseline. These scores do not demonstrate role abstraction, changed-goal
reappraisal, calibrated counterfactual planning, or Mecha's security properties.

## 2. Mechanisms worth borrowing

### ReFind: preserved context and bounded search

The [repository](https://github.com/imlrz/ReFind) and
[preprint](https://arxiv.org/abs/2608.12888) describe searching preserved chat
records with lexical retrieval, session context, nearby turns and date
constraints. An agent can search, retain evidence references, or finish. The
submission returns evidence for a separate answer model. Its default planner
budget is four actions, not necessarily four searches.

Use this as Mecha's first strong comparator: persistent FTS over exact captured
records, expandable episode windows and a bounded evidence-seeking loop. A
record can matter later even when the original extractor omitted it. The
inspected `app/retriever.py` rebuilds a BM25 index from the user's chunks per
search; Mecha should use persistent indexes. Competition-server authentication
and retention defaults are not a deployment template.

### Hindsight: evidence-linked consolidation and reusable views

Hindsight's [observations](https://hindsight.vectorize.io/developer/observations)
consolidate related facts while retaining supporting records and quotes.
[Mental models](https://hindsight.vectorize.io/developer/mental-models) provide
refreshable answers to standing questions. These suggest a useful distinction:
a materialized view can answer a recurring question without claiming to be a
newly learned general principle.

Its [retrieval](https://hindsight.vectorize.io/developer/retrieval) combines
semantic, keyword, graph and temporal routes. The inspected
`hindsight-api-slim/hindsight_api/engine/search/fusion.py` includes per-source
caps and rank fusion. Measure diversity and freshness alongside similarity.

The [consolidator](https://github.com/vectorize-io/hindsight/blob/1a37e009cbe9b8cd76b2c5d5b6c5745374e19507/hindsight-api-slim/hindsight_api/engine/consolidation/consolidator.py)
rechecks source liveness before committing after model work. Borrow that
race-handling pattern. Repeated assertions and their summaries must not count
as independent confirmations.

### Cognee: decomposed queries and explicit provenance

Cognee's [decomposition retriever](https://github.com/topoteretes/cognee/blob/525b1d1179c550cd5b5661bcbf8f00f57a676928/cognee/modules/retrieval/graph_completion_decomposition_retriever.py)
splits a question into subqueries and deduplicates retrieved triplets. One mode
combines evidence; another generates intermediate answers. Prefer the evidence
path for Mecha, keeping generated claims explicitly marked as interpretations.

Its [provenance manager](https://github.com/topoteretes/cognee/blob/525b1d1179c550cd5b5661bcbf8f00f57a676928/cognee/modules/provenance/manager.py)
offers source-quote, location and validity fields, but tracking methods catch
storage failures and return `None` so host operations can continue. That policy
is incompatible with Mecha's required lineage: derived records and dependencies
must commit atomically or remain unavailable. This is a policy mismatch, not
a claim that every Cognee operation lacks provenance.

### MemOS: experience to policies to abstractions

The [local plugin](https://github.com/MemTensor/MemOS/tree/d326a6419467c2205041b58533b0ebfdf4f38173/apps/memos-local-plugin)
is particularly relevant to concept learning. Its
[L3 types](https://github.com/MemTensor/MemOS/blob/d326a6419467c2205041b58533b0ebfdf4f38173/apps/memos-local-plugin/core/memory/l3/types.ts)
represent abstractions over policies as environment descriptions, inference
rules and constraints, retaining policy and episode references. This is a
concrete analogue for part of Mecha's proposed loop.

In [`abstractDraft` and `buildWorldModelRow`](https://github.com/MemTensor/MemOS/blob/d326a6419467c2205041b58533b0ebfdf4f38173/apps/memos-local-plugin/core/memory/l3/abstract.ts),
an LLM proposes an abstraction and confidence value, adjusted by cluster
cohesion. Mecha should treat this as a proposal, not calibrated probability or
held-out evidence of transfer. Its
[merge decisions](https://github.com/MemTensor/MemOS/blob/d326a6419467c2205041b58533b0ebfdf4f38173/apps/memos-local-plugin/core/memory/l3/merge.ts)
use domain, shared policies and vector similarity; Mecha must also preserve
incompatible hypotheses and revision history.

The separate [skill verifier](https://github.com/MemTensor/MemOS/blob/d326a6419467c2205041b58533b0ebfdf4f38173/apps/memos-local-plugin/core/skill/verifier.ts)
checks declared-tool coverage and token overlap with evidence. These are
screening heuristics, not semantic entailment or successful-transfer tests.
Borrow the staged abstraction and lifecycle while retaining stronger evaluation.
The local plugin is not established to be the implementation behind the
commercial leaderboard result.

### SimpleMem, Mem0 and InvMem: narrower lessons

[SimpleMem](https://arxiv.org/abs/2601.02553) motivates self-contained retrieval
units, resolved references and temporal normalization. Preserve original spans
beside normalized text: “lossless” in a method name does not guarantee retention
of information needed by a future goal. Its current
[repository](https://github.com/aiming-lab/SimpleMem) describes guarded retrieval
configuration optimization; test this offline before background tuning.

[Mem0's current implementation](https://github.com/mem0ai/mem0/blob/ed1f10fc87080f0b67f99be370817504888462ab/mem0/memory/main.py)
offers a compact API and identity-scoping checks. It uses additive extraction,
so older accounts of its automatic update/delete loop should not be treated as
descriptions of this revision. It does not replace Mecha's temporal contract.

[InvMem](https://github.com/wenxiaof345-ctrl/vanilla-rag-memory) reinforces the
value of a strong baseline. Its current README describes dense and BM25 fusion;
do not attribute the earlier submitted score to that configuration without the
submission manifest.

Reviewed project roots identify ReFind, Hindsight and SimpleMem as MIT, and
Cognee, Mem0 and MemOS as Apache-2.0. Before copying code, check specific files,
notices and dependencies at the selected revision; model-weight licenses are
separate. Open source does not guarantee continued maintenance. Borrowing
mechanisms does not require depending on these runtimes.

## 3. What Mecha already has

Verified in Spark's mecha-graph source at `1368fba`, rather than inferred from
the older Mac checkout:

| Existing source | Capability | v2 implication |
|---|---|---|
| `search.rs::{bm25_ranked,vec_ranked,rrf_fuse}` | FTS5, sqlite-vec and rank fusion | Reuse the strategy; measure scoped recall. |
| `enrich.rs::Envelope` | Entities, dates, commitments, decisions/rationales, open questions | Extend into source-grounded event roles. |
| `fact.rs` | Validity intervals, negative facts, observations, contradictions, supersession | Preserve these semantics during migration. |
| `fact.rs::{mint_shadow_candidate,convert_pending_to_shadow}` | Retrievable, labeled, discounted unreviewed facts; some classes remain queued | Review-on-use already exists in source; live deployment/backlog was not audited. |
| `context.rs::set_summary`, `summarize.rs` | Generated summaries separated from owner instructions; stale detection and refresh | Add full dependencies and revisions to reusable views. |
| `linkers.rs` | Co-occurrence, temporal, nearest-neighbor and structural proposals | Association alone does not establish a causal principle. |

Two specific differences matter:

1. **Candidate filtering order:** lexical/vector queries apply a limit before
   `filter_candidates`. Ineligible records can occupy the initial top-k and
   crowd out eligible evidence. Output filtering still exists; this is a
   recall issue, not evidence of an access leak. v2 should constrain eligibility
   before ranking limits where possible, or use a bounded, measured strategy
   that avoids silent candidate starvation.
2. **Full derivation history:** `fact.rs::attach_derivation` computes maximum
   sensitivity over contributors but stores the newest episode as citation and
   clock anchor. That preserves a privacy tier, not the complete contributing
   set. v2 needs every dependency for forgetting, correction, independent
   evidence accounting and recomputation.

`enrich.rs::store_enrichment` replaces the current payload; revisions should
preserve which extraction informed a decision. Mecha already has learning and
counterfactual evaluation modules. Connect role-bound concepts to those
evaluations rather than building an unrelated learning system.

## 4. Retrieval must notice implications

The [InMind preprint](https://arxiv.org/abs/2607.24368) identifies a relevant
failure: on 125 tasks, supplied decisive memories enabled 84% indirect-query
accuracy, while tested memory systems reached at most 14.4% when retrieving
those memories, despite strong direct recall. It separates missing storage,
missing bridging knowledge and failure to surface a relevant memory. This
small benchmark does not establish Mecha's performance, but identifies an
evaluation dimension ordinary recall can miss.

Our inference: retrieve from the current situation as well as query similarity.
Keep a small, explicitly selected context of active goals, constraints and
homeostatic readings. Search from that context, relational roles and plausible
consequences; inspect supporting episodes before relying on derived claims.
This does not require putting every memory in the prompt or granting recalled
instructions authority.

## 5. Implications for the v2 design

Keep the store contract and shared operations: capture, encode, retrieve,
consolidate and evaluate. Appraisal, planning and rumination call those
operations rather than owning separate memory backends.

- **Capture:** exact versioned records with source identifiers/timestamps;
  episode membership and ordering allow expansion around hits. JSONL can stay
  the runtime log, SQLite holds captured evidence, and managed artifacts hold
  large media. Coordinate raw and derived deletion.
- **Encode:** typed event roles, conditions, actions, outcomes, validity,
  negation and source spans. Retain uncertainty and distinguish simulation.
- **Retrieve:** persistent lexical/vector routes, bounded graph expansion,
  optional iterative search, source diversity and a shared token budget.
  Return revisions, spans, scope and selection reasons. Enforce eligibility
  and inherited taint independently of relevance.
- **Consolidate:** distinguish summaries from concepts. Refresh affected
  dependents. Store attributed hypotheses automatically without promoting
  them to trusted-prefix rules or recreating thousands of owner approvals.
- **Evaluate:** distinguish source support, applicability, predictive
  calibration, task utility and execution permission. One model confidence
  score cannot answer all five.

The proposed `ConceptV1` already has roles, relations, applicability,
consequences, examples, counterexamples, exceptions and assessments. The new
work is generating and testing those representations.

Synthetic example: a late equipment delivery and a delayed collaborator review
can instantiate **unresolved prerequisite**. Bind prerequisite, dependent task,
responsible actor and deadline; predict deadline risk and consider resolving
the prerequisite. Test transfer to a new domain with different names, and
abstention where the dependency is optional. The same past delay may matter
under one goal and be irrelevant under another. Summarizing two incidents
does not itself demonstrate that transfer.

## 6. Small experiments before a larger migration

Freeze the source corpus, answer model, question splits and compute budgets.
Start with 30–50 synthetic cases with known evidence and outcomes:

| Arm | Capability | Question |
|---|---|---|
| A | Raw FTS plus episode windows | How far does cheap evidence retrieval get? |
| B | A plus vectors and rank fusion | Does it recover paraphrases at acceptable cost? |
| C | B plus bounded iterative search | Does another search recover missing evidence? |
| D | B plus event-role/graph retrieval | Does structure help multi-event questions? |
| E | Best prior arm plus concepts/current context | Does abstraction transfer and improve decisions as goals change? |

Include oracle evidence to separate retrieval from reasoning failures. Hold out
episode families/domains for transfer tests, not just paraphrases of questions.
Compare E with and without current-context routing to separate its contribution.

Test direct recall, temporal corrections, indirect implications, unfamiliar role
bindings, changed goals/resources, counterexamples, deletion and duplicate-source
support. Check isolation and taint deterministically. Grade action feasibility
and task outcomes separately from model-judged prose. Simulated outcomes must
not become actual outcomes or independent confirming evidence.

Measure grounding and recall alongside p50/p95 latency, model calls, tokens,
ingestion throughput, RAM/index size, abstention and owner interruptions. A
successful run that read a memory does not establish the memory's causal value;
use paired ablations and held-out outcomes where feasible.

Recommended next implementation: source/episode capture, raw retrieval and the
evaluation harness. Add one improvement at a time. Defer framework adoption,
nightly migration, automatic privileged-rule installation and broad concept
generation until smaller comparisons show benefit. Retain SQLite and
Mecha-owned Rust APIs; borrow algorithms and contracts where they earn their cost.


## 7. First model-backed comparison on Spark

2026-11-04 local date (2026-11-05 04:18:21–04:20:59 UTC). This is a new,
small synthetic diagnostic, not a leaderboard reproduction. Source
`d3ca03b82`, installed in an isolated prototype directory; production Mecha
remained `fec8d55c7`, web service active, no private-data ingestion or nightly
migration. The eight new session families and 16 cases were authored before
inference. Review corrected a battery-reserve arithmetic error and a missing
oracle affordability premise before freezing the run. The older 32-case fixture
was not changed.

The resident model was `qwen3.6-35b-a3b-uncensored`, served by llama.cpp
`b1915-27637851` with four slots and 262,144 tokens per slot. All arms used
seed 7, temperature 0.6, thinking disabled, an 8,192-token output cap, no tools,
no retries, and the same answer prompt. Evidence was limited to four whole
records and 8,000 serialized bytes. The goal-aware arm spent one additional
model call proposing up to two literal searches from the question, goal and
resource state; its planner did not receive the initial retrieved evidence.
All search requests retained host-set scope, session, snapshot and time filters.

| Condition | Correct choice /16 | Strict grounded /16 | Complete support /14 answerable | Median seconds/case | Model calls | Input / output tokens |
|---|---:|---:|---:|---:|---:|---:|
| Lexical |7|5|5|2.849|16|19,455 /406|
| Goal-aware query expansion |7|5|5|4.479|32|21,449 /673|
| Oracle evidence |16|11|14|2.818|16|10,277 /592|

All 48 condition rows completed without transport/parse errors. Both deliberately
unanswerable cases were answered by abstention in all three arms. Many retrieval
failures on answerable cases also produced abstention; those are failed task
completion, not necessarily unsafe answers given the incomplete visible evidence.
The report's `abstention_correct` field tracks full-corpus answerability, not
whether abstaining was prudent given a particular retrieved subset.

**What the traces establish.** In the eight changed-goal cases, both retrieval
arms made zero correct choices; oracle evidence produced eight correct choices,
including all four goal/resource pairs. The query planner repeatedly retained
project names (for example, `Topaz algorithm accuracy`) while the decisive
records named the alternatives (`Sol` and `Neri`). BM25 plus neighboring-record
fusion filled the limited evidence set with project mentions and nearby filler.
One extra search recovered one side of the Juniper comparison, but no additional
answerable case received complete support. This particular query-expansion
method therefore added model calls and latency without improving task success.
It does not establish that goal-aware retrieval in general is ineffective.

Five oracle failures were **citation-ID errors despite correct choices**:
responses copied the illustrative `episode:line` format into IDs such as
`episode:4` instead of using the provided `topaz:4`. The strict scorer correctly
refused them. The analogous errors also accounted for both direct-recall failures
in each retrieval arm. Do not interpret 11/16 grounded as five failures to reason
about the supplied facts, or silently normalize these citations and replace the
frozen score. Prompt wording and citation interface contributed to this result.

**Next small experiment.** First make citation selection explicit (copy from an
enumerated set, ideally constrained output) and verify it with separate controls.
Then compare bounded retrieval that follows the alternatives named in initial
source evidence against the question-only planner, keeping the answering model,
evidence budget and model-call count matched. Preserve separately ranked anchors
when allocating neighbor context so incidental nearby records do not consume the
whole budget. This targets the demonstrated missing relation—project to named
alternatives to their consequences—before adding embeddings or concept learning.
Use new scenario families for the next held-out claim; this run is now available
for development and failure analysis.

**Reproduction and limits.** The CLI and full parameter record are documented in
`prototypes/memory/README.md`. This is one seed on a shared server; timing includes
inference/queueing and does not isolate hardware cost. Exact support-set grading
is deliberately strict and is not a universal factuality score. Partial provider
usage can lose per-field missingness (documented adapter limitation); token totals
are not energy or financial estimates. No executed action, concept induction,
private-corpus scale test or runtime appraisal integration was measured.

Machine-readable files are retained locally under
`results/memory-model-20261104/` (gitignored): `report.json`, `summary.json`,
`failures.jsonl`, `served-props.json`, and `build.txt`. Spark retains the same
run plus its synthetic SQLite database under
`~/.local/share/mecha-prototypes/memory-model-20261104/run-seed7/`.
Fixture SHA-256:
`e17425d51064764d1931b0de15f7c104ff675e8df21b52046dc1c6af0e51cd4c`.
Binary SHA-256:
`40ab2c8a15eac860ca586ead5d615128778f30aac35f9e29779c270f89f2f74a`.


## 8. Evidence-informed search and constrained citations

2026-11-05, 05:39:52–05:42:03 UTC. Source `27e15d8a2`; isolated Spark
prototype, resident `qwen3.6-35b-a3b-uncensored`, llama.cpp `b1915-27637851`,
four slots and 262,144 tokens per slot. The implementation is
`memory::model_evaluation::followup::run`, selected by `evaluate-model
--follow-evidence`; `--development` selects the old 16 cases, otherwise the
new 12-case holdout is used. Neither fixture nor prompt changed after inference.
The original comparison remains separately reproducible without the new flag.

Both search conditions reuse the same initial recall, allow one planner call
and at most two additional searches, and share anchor-first rank fusion and
account/session/snapshot/time filters. Only the evidence-informed planner sees
initial records. Neither planner sees answer choices or gold support. All three
conditions share an answer prompt and response schema enumerating every visible
citation ID and public choice. Host grading remains independent of that schema.
The four-record/8,000-byte evidence cap, seed 7, temperature 0.6, no thinking,
8,192-token output cap, no tools and no retries apply throughout. Input tokens
are measured, not matched; reading initial evidence costs additional tokens.

The development run completed before the holdout. Both reports passed parsing,
transport, visible-ID and unique-ID checks: **84 answers, no invented or duplicate
citation IDs**. The frozen holdout comprises six new session families: three
pairs with changed goals/resources, two correction cases, two temporal cases
and two absent-answer cases. These remain small authored diagnostics.

| Fresh holdout condition | Correct choice /12 | Strict grounded /12 | Complete support /10 answerable | Median / p95 seconds | Calls | Input / output tokens |
|---|---:|---:|---:|---:|---:|---:|
| Question-only planner |8|3|5|1.760 /2.076|24|17,281 /834|
| Evidence-informed planner |8|5|6|2.244 /2.508|24|29,883 /720|
| Oracle evidence |12|10|10|0.882 /1.120|12|7,818 /429|

| Development condition | Correct choice /16 | Strict grounded /16 | Complete support /14 answerable | Median / p95 seconds | Calls | Input / output tokens |
|---|---:|---:|---:|---:|---:|---:|
| Question-only planner |11|6|7|1.671 /1.950|32|23,519 /1,044|
| Evidence-informed planner |11|6|6|2.241 /2.524|32|39,979 /1,017|
| Oracle evidence |14|11|14|0.846 /1.250|16|11,014 /611|

**What improved, and what did not.** Constrained citation selection eliminated
the observed malformed-ID problem in these runs. The held-out evidence-informed
condition recovered complete support in one additional case and passed strict
grounding in two additional cases, but did not improve total correct choices.
It recovered the bacteria-cartridge choice while losing the local-pipeline
choice. Both retrieval conditions selected only two of six changed-goal choices
correctly; oracle evidence selected all six. The higher token use and latency
therefore have not yet earned a general task-success claim. Development support
coverage was slightly worse with the evidence-informed planner. Comparisons with
§7 bundle schema, prompt and fusion changes and do not isolate their effects;
the within-run planner contrast is the controlled comparison.

**The traces localize the remaining retrieval gap.** The new planner does follow
named alternatives: `Vireo room noise level`, `Rill room capacity`, `Ilex pipeline
specifications`, and `Quill pipeline status`. Yet Lumen and Lyra evidence bundles
still contain the linking summary and filler, missing the alternative-property
records. `Memory::recall` uses literal OR-term BM25 and only two anchors per
search. Naming the entity in a natural-language query does not require that
entity to be present in a returned anchor. This motivates testing explicit entity
binding and per-alternative evidence coverage; it does not establish that an
embedding model, larger graph or additional reasoning loop is necessary.

**Grounding is a strict protocol score, not factual accuracy.** The two oracle
holdout failures selected the correct option and cited its supporting record,
but omitted the other alternative required by the authored comparison rubric.
Some retrieval answers cited a relevant linking or superseded record in addition
to the exact gold set; those fail the unchanged scorer. Both retrieval conditions
correctly chose insufficient evidence and set `abstain=true` for both absent
answers, but supplied citations, violating the required empty-citation contract.
Some of those records explicitly say that the information is missing. Thus the
zero strict abstention passes in those conditions do not mean the model fabricated
an opening time or technician. All conditions obeyed the citation-ID vocabulary;
that is distinct from citation relevance, sufficiency and response consistency.

**Next bounded experiment.** First diagnose candidate retrieval deterministically
using the existing failure traces: separate entity identifiers from descriptive
search terms, preserve authorized per-entity anchors, and measure whether both
alternatives' properties reach the evidence packet. Keep the byte/search budget
fixed and compare against this frozen baseline before another model run. Review
support scoring separately: report answer correctness, premise coverage and format
compliance, allowing defensible support sets where appropriate in a newly versioned
protocol. Do not rewrite the old scores. A subsequent generalization claim needs
another unseen set, because this holdout is now available for failure analysis.
Vector retrieval, concept induction, private-corpus imports and runtime appraisal
integration remain deferred.

**Verification and reproducibility.** Required all-feature workspace checks passed
5,937 tests (25 ignored); standalone 39 and main CLI 8, with strict Clippy and
format checks. Fresh review found no Critical/Important issues. One Minor remains:
a dedicated duplicate-valid-citation regression; current host grading rejects
duplicates and none occurred in the measured responses. No production service,
store or nightly job changed. Production `fec8d55c7` and active web service were
verified after inference. Generic provider per-field usage missingness remains
an existing limitation. One seed, shared-server latency and synthetic episodes
cannot establish production-scale robustness, energy cost or compositional learning.

Local artifacts: `results/memory-hop-20261105/` contains `development.json`,
`holdout.json`, their `*-summary.json` and `*-failures.jsonl`, progress logs,
`served-props.json` and `build.txt`. Spark retains both databases and reports under
`~/.local/share/mecha-prototypes/memory-hop-20261105/run-seed7/`, with the binary
in the adjacent `bin/` directory. The development fixture hash is unchanged from
§7. Holdout SHA-256:
`07b967eb940ce162b21ad83c3f68bdc0ff7496190636953a6c3feb0ac2df5c83`.
Binary SHA-256:
`9e6066324dac3627ec9d7a97f060860f854cb00ce1064f2d674ae73fb4e3e288`.


## 9. Required-mention retrieval replay

2026-11-05. This development diagnostic replays all 12 evidence-informed plans
from §8 on the local Mac, without any model calls. `Memory::recall_mention`
adds a literal name/phrase requirement before top-k selection. The existing
eligible-only BM25 scorer still ranks results, and a non-stemming unicode61
index gates names. Descriptive terms cannot exclude a record that contains the
name. Names are bounded and safely quoted; aliases and homonyms remain unresolved.
Ordinary recall and the previous model protocols are unchanged.

The replay freezes the exact saved query strings and supplies manual bindings
for the six alternative-comparison cases: Vireo/Rill, Noma/Tavi, Ilex/Quill.
Each binding must appear in its saved query and an initially visible original
message field. Other queries are unchanged. Both conditions use three searches
(one initial and two additional), two anchors per search, shared anchor-first
fusion and a maximum of four records/8,000 bytes. Gold appears only in coverage
scoring. The original condition reproduced all 12 frozen final evidence-ID
sequences exactly.

| Retrieval condition | Complete support /10 answerable | Complete support in changed-goal cases /6 |
|---|---:|---:|
| Original saved searches |6|2|
| Same searches with explicit required names |10|6|

The four repaired cases are both Lumen room choices and both Lyra pipeline
choices. In each, the returned packet now contains both alternatives' property
records instead of nearby filler. Watershed's two packets already contained
complete support and retained it. Correction, temporal and absent-answer
packets remained identical. Absent answers have no support-set coverage score;
this replay makes no new correctness or abstention claim.

**Interpretation.** This identifies a fixable retrieval mechanism: using the
right name as an optional BM25 term did not require its presence in returned
anchors. An explicit lexical constraint repairs these observed failures given
correct bindings. The result does not establish an automatic entity extractor,
a generally successful retrieval system, or improved model reasoning. The names
were annotated after seeing the earlier failures and these cases are now
**development data**. No answer is regenerated and no earlier score is replaced.

A single local pass took about eight milliseconds median per case in each
condition. That coarse timing is not evidence of equivalent compute or a speedup:
the new method builds an extra in-memory index over eligible records, the corpus
is tiny, and this is not a repeated load benchmark. The source/index/filter
invariants and bytes-per-packet checks are more informative at this stage.

**Next experiment.** Have the evidence-informed planner return a typed optional
name binding alongside each property query, grounded in its visible sources.
Compare ignoring versus enforcing the same proposed bindings under the same
search/evidence caps. Measure binding errors, evidence coverage, answer accuracy
and support/format compliance separately, using new unseen scenarios. Preserve
this manual-binding diagnostic as a component check. No need to add a larger
graph framework, vector service or concept learner to test that step.

**Verification and artifacts.** The main CLI command is `evaluate-mentions`;
its black-box test runs with invalid global/project config and verifies refusal
to overwrite its database. Tests cover names versus stems, literal FTS operators,
phrase adjacency, combining-mark/quoted names, scope/source/snapshot/time limits,
byte caps, replay coverage and annotation provenance. The deferred duplicate-valid
citation regression from §8 is now included. Fresh review found a combining-mark
bug: pre-tokenizing the name could prevent even identical source text from
matching. Its regression failed before preserving the safely quoted raw phrase,
and passes after the fix. Full check results are recorded in HISTORY.

Local `results/memory-mentions-20261105/` retains the JSON report, summary,
synthetic SQLite database and build metadata. The tracked
`memory/fixtures/mention_replay.json` records every saved query, manual binding,
source report SHA-256 and source fixture SHA-256; the output also hashes that
manifest. This run changes neither Spark's installed prototypes nor production
Mecha, source data, nightly jobs or runtime appraisal.


## 10. Automatic source-grounded name bindings

2026-11-05. Source `83753f9ea` adds `evaluate-model --auto-bindings`. One planner
sees the initial episode packet, question, goals and resources, then proposes
at most two searches with nullable names and source IDs. The host verifies each
name against its query and original text of the cited initially visible episode,
using the same unicode61 phrase semantics as required-mention retrieval. IDs in
the schema enumerate all visible sources, not the useful ones. Choices, oracle
records and authored target-name labels never enter the planner. Invalid plans
fail both search conditions without a broad-search fallback; the oracle can
still run.

The experiment calls the planner **once per case**. The unbound and bound arms
receive that identical validated plan and initial packet; only enforcement of
its names differs. Null bindings use ordinary search in both. Each search arm
has at most three searches, two anchors per search, shared fusion and four whole
records/8,000 bytes of final evidence. Answer order rotates across cases. All
three arms share the answer prompt/schema and unchanged strict support rubric.
Planner calls/tokens and initial retrieval time are recorded once per case,
separately from answer calls and per-arm retrieval; totals must not count the
shared planner three times. Equal search limits still do not imply equal index
construction cost.

Development uses the previous 12 cases. The fresh set has 14 cases: four changed-
goal pairs, two corrections, two historical readings and two missing answers.
Names include multiword phrases, a diacritic and the distinct names Bank/Banks.
Review verified all feasibility premises and gold records before inference.
Authored target-name recall is scored only for the eight comparison cases and
only after planning; source grounding alone does not prove relevance.

The isolated Spark run uses resident `qwen3.6-35b-a3b-uncensored`, seed 7,
temperature 0.6, thinking off, an 8,192-token output cap, no tools and no retries.
Development runs before the frozen fresh set, with separate new databases.
The original model reports remain unchanged. This is an authored synthetic
comparison, not a public leaderboard reproduction or a private-corpus scale test.


**Measured results (06:46:07–06:48:05 UTC).** The resident server reported
llama.cpp `b1915-27637851`, four slots and 262,144 tokens per slot. No source,
fixture, prompt or score changed between the development and fresh runs.

| Fresh condition | Correct choices /14 | Strict grounded /14 | Complete support /12 answerable | Median pipeline seconds |
|---|---:|---:|---:|---:|
| Ignore proposed names (unbound) |9|4|7|2.950|
| Enforce proposed names (bound) |13|7|11|3.118|
| Oracle evidence |14|13|12|0.997|

| Development condition | Correct choices /12 | Strict grounded /12 | Complete support /10 answerable | Median pipeline seconds |
|---|---:|---:|---:|---:|
| Ignore proposed names |9|4|6|2.852|
| Enforce proposed names |11|6|9|2.278|
| Oracle evidence |12|9|10|0.910|

Fresh plans passed source validation in 13/14 cases; the 13 valid plans contained
24 non-null grounded bindings. Both authored target names were selected in all
8 comparison cases. The unbound and bound arms answered 4/8 versus 8/8 changed-goal
cases correctly. Both selected the correct insufficient-evidence choice in both
absent-answer cases. The rejected plan remains in every denominator, rather than
being excluded to improve the headline accuracy.

**The gain is partly evidence recovery and partly answer sensitivity.** Binding
recovered complete support for Palisade load, Magnetar urgent, Compass color and
Compass fast. Three changed from incorrect abstention to correct choices; Compass
color was already correct with partial support. Palisade budget changed from the
wrong to the right choice despite complete support in both packets: those packets
contained the same records in different orders. Thus the four-choice gain is not
four demonstrated new-fact recoveries. One seed and stochastic model/server behavior
cannot separate evidence order from sampling sensitivity; do not claim a general
four-case causal gain from evidence coverage alone.

**The source checks exposed a concrete binding error.** For Tessel owner, the
planner proposed `Tessel R8 dispatch`, citing a sentence that contained those words
in separate places. The literal phrase was absent, so both retrieval arms stopped.
The second proposed binding (`Rumi`) was valid, but the current policy rejects the
whole plan. Development similarly rejected `Ilex pipeline` against text that said
`Ilex or Quill pipeline`. These are failed literal-copy contracts, not proof of
unknown entities or malicious content. No broad fallback, source repair or retry
hid either failure. All 74 generated answers used visible unique citation IDs; no
transport or JSON parsing failures occurred across the 100 actual model calls.

**Strict grounding still mixes several requirements.** In the bound fresh arm,
all 13 generated choices were correct, but six failed the strict rubric: four added
records outside the exact gold support set, one omitted the other alternative,
and one correctly abstained while citing a record explicitly saying no booking
existed. The oracle's sole strict failure omitted the other alternative while
selecting and citing the correct kit. These frozen protocol failures are distinct
from wrong choices and invented citations; no score was retrospectively relaxed.

**Cost accounting.** The fresh experiment used 14 shared planner calls, 13 answer
calls in each search arm and 14 oracle calls: 54 actual calls. Shared planner input/
output tokens were 17,399/1,317. Unbound answer tokens were 16,657/491; bound
16,942/583; oracle 9,756/557. Development used 12 shared planners, 11/11/12 answers
(46 calls), with planner 14,746/1,039 and answer tokens 14,159/433, 14,613/476,
7,815/429 respectively. Missing usage was not reported in these responses, but
the generic decoder's partial-field limitation remains. Pipeline medians above
include shared planning and initial retrieval for each search path; actual total
experiment costs count shared work once. Medians include the failed-plan cases.
Shared-server timing and a tiny corpus do not establish speed or energy savings.

**Next bounded work.** Constrain names to exact source spans and measure the
associated failure/rejection rate on a new test set; retain fail-closed source
checks. Add the deferred scripted non-null binding regression and an explicit
binding-count field (the current summary derives it from valid plans). Improve
support/response-contract evaluation in a separately versioned protocol before
interpreting its strict score as factuality. Then qualify a narrow opt-in memory
context adapter for planning/appraisal on synthetic sessions; this experiment
does not authorize a production memory cutover. Aliases, unseen names, larger
corpora, private storage and nightly jobs remain separate work.

Required verification passed 5,950 all-feature workspace tests (25 ignored), 50
standalone tests, 10 main CLI tests, strict Clippy and formatting. Fresh review
found no Critical/Important findings; the two test/reporting minors above remain
deferred. The compiled source is installed only at
`~/.local/share/mecha-prototypes/memory-bindings-20261105/bin/mecha` on Spark.
Production `fec8d55c7` and active web service were verified after the run; no
production stores, model switch or nightly activity changed.

Artifacts: local `results/memory-bindings-20261105/` retains both full reports,
per-split summaries/failure traces, progress logs, summarizer, `build.txt` and
`served-props.json`. Spark's adjacent `run-seed7/` retains those reports and
both synthetic databases. Fresh fixture SHA-256:
`3ed5c86c44d740b146a5ea566d6f86d19304db80377834ac7a2944966445e0d3`.
Binary SHA-256:
`91982d9e409e8cd77632a9724d91049dec8e04293a62e3a3aaeff0aa3655bcd4`.


## 11. Search follow-up implementation status

**2026-11-05 · implementation addendum; earlier measurements are unchanged.**
The immediate search follow-up in §§7–10 was to follow alternatives named in
initial evidence and retrieve each alternative's properties without generic
query terms crowding them out. That sequence is now implemented: the
`model_evaluation::followup` comparison supplies evidence-informed queries and
explicit citation schemas; `Memory::recall_mention` constrains anchors before
ranking limits; `binding_evaluation::run` compares the same validated names with
and without enforcement. `Memory::context` adds exact contiguous original-field
selections in the runtime path, and `GoalMemory` supplies packets to the existing
`goal_context` planning tool under an explicit synthetic-only `run` option.

The previously deferred valid non-null routing regression is present in
`automatic_non_null_binding_changes_the_executed_retrieval_arm`: the same query
misses the named fact unbound and recovers it bound. New automatic-binding reports
also expose `grounded_binding_count` per case: `null` for a rejected plan, `0` for
a valid plan with no named searches, and the number of validated named searches
otherwise. It counts bindings, not unique entities; repeated bindings count
separately. This additive field does not change the `auto-bindings-v1` prompts,
fixtures, retrieval, scoring or existing report artifacts.

The remaining near-term qualification is whether a real model uses the runtime
source-selection interface effectively while planning, including rejection and
evidence-coverage behavior. Hybrid lexical/vector retrieval, aliases/coreference,
persistent-index scaling, and a more flexible separately versioned support scorer
remain distinct follow-ups. The current small-corpus results do not establish
that those additions are necessary, nor that retrieval latency is production-ready.
