# Controlled gossip comparison

The question is whether sharing a peer's cited answer before generating follow-up
questions finds more supported facts than continuing to investigate one's own
sources. `scripts/gossip-comparison.py` registers and runs the comparison through
`mecha-core/examples/gossip_compare.rs`, which calls the production
`gossip::exchange_with_followup` and executes real MCP searches.

```bash
cargo build -p mecha-core --example gossip_compare
python3 scripts/gossip-comparison.py --out /tmp/new-gossip-comparison
```

The actor uses only the local `qwen3.6-35b-a3b` endpoint at port 8080, with 262,144
context tokens per slot. The runner checks the served alias/context and records
server generation settings before and after. No operator config, learned rules,
private correspondence or live graph are loaded. The fixture advertises only
`kg_search`; its writes are local execution receipts, with no graph filings.

## Conditions registered before execution

Four synthetic cases cover complementary evidence, conflicting statements,
changes over time, and a namesake whose statements must not become the target's
biography. `build.py` generates the frozen `cases.json`; every source episode and
reference fact is reviewable. The gold field never enters an MCP result.

Each case runs at seeds 1 and 2 under both conditions, in a registered shuffled
pair order with alternating arm order: **16 exchanges**, each with two readers
and three rounds. Readers have disjoint Slack/Bee source lenses, the same fixed
time window and four model turns per answer. Each question generator has one
turn plus the production protocol's one retry. Each exchange therefore has a
**32-request ceiling**: 6 answers × 4 turns + 4 question generations × 2 attempts.
There is no forced final answer outside that allocation. Requests can contain
multiple searches; tool-call count and tokens are not capped by this number.

- **Peer:** the ordinary protocol. Question generators see both readers' admitted
  cited answers and route questions to the peer. Each answerer sees only its own
  earlier answers before committing, plus its assigned question.
- **Own evidence:** question generators see only their reader's admitted answer,
  receive an explicit self-follow-up instruction and route questions back to the
  same reader. Answer prompts, lenses, retries and limits are unchanged.

The control changes both available evidence and the role instruction needed to
use that evidence coherently. It is not a byte-identical prompt ablation. The
ordinary CLI's `asker`/`exchange` entry points always select peer mode.

`server.py` performs deterministic lexical retrieval, capped at two results per
search and ranked by query overlap weighted by within-source document frequency.
Ties keep registered episode order. It checks the actual target, source, time,
private-data and `probe` arguments. Ranking never reads arm, seed, history or gold.
This tests the protocol against a small retrieval world, not live semantic-search
quality. All episodes' retrieval timestamps are September 1; dates *inside* the
evidence describe plans and events and must be interpreted separately.

## Outcome audit

Each trial retains `config.json`, every model request/response/error in
`provider.jsonl`, actual search arguments/results in `search.jsonl`, and the
exchange in `result.json`. The runner refuses to reuse an output directory and
does not rerun a failed trial. The source and binary hashes, seed/order and server
conditions are recorded before execution. Raw answers remain available for audit.

`--audit-only` prepares `audit-packet.json`: every unique admitted statement,
its citation and source text, target identity and reference facts, ordered by a
stable hash with arm, seed and round removed. An evaluator writes
`audit-verdicts.json`, one record per ID:

```json
{"id":"claim hash", "label":"supported", "facts":["reference episode id"], "reason":"Why the cited text supports this statement about the target."}
```

Labels are `supported`, `contradicted`, or `unresolved`. A citation alone is not a
supported verdict. Namesake facts require an explicit distinction; a current-role
claim contradicted by later evidence fails unless appropriately source/time
qualified. Conflicting contemporaneous sources establish what each source says;
they do not establish an unqualified current-world fact. Unknowns stay unknown.
A supported contrast about the namesake may have no target fact ID.

This pilot scores the readers' admitted claims and their supported source-item
coverage. It does not run production extraction, final verification, adjudication
or graph filing, and does not separately grade joint contradiction discovery.
A statement requiring multiple sources needs evidence for all of its clauses;
one valid source citation does not establish an entire cross-source inference.

Run `--report-only` after that review. The report counts distinct reference
episode facts per trial, deduplicating repetition across rounds, and reports newly
supported facts and contradicted claims per round. A partial statement about a
reference episode is evidence coverage, not recall of every proposition in that
episode. Two sources can supply overlapping assertions under separate reference
IDs; the primary measure is supported source-item coverage, not a count of unique
new world facts. The audit covers **structurally admitted claims**, not all generated
prose; ungrounded answers, abstentions and stalled question generators are counted
separately. Include evaluator identity and limitations with every published run.

Round 1 is a prefix measurement, not another independently randomized treatment.
The main comparison is the three-round result under equal allocated request
ceilings. Report actual model requests, searches and tokens alongside yield;
claiming equal actual cost would erase an outcome of the protocol. Four synthetic
cases and two seeds do not establish transfer, independent task-family holdout,
or a statistically reliable production winner. No report promotes a configuration.
