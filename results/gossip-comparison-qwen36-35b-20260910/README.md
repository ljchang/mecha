# Gossip comparison — Qwen 3.6 35B, 2026-09-10

Peer dialogue supplied no additional supported source coverage after round 1 in
this synthetic pilot. At three rounds it covered **23/42** reference source items,
versus **24/42** for own-evidence followups: seven paired ties, one control win,
no peer wins. This does not justify increasing nightly gossip rounds or promoting
a new default. It does identify target attribution and unsupported elaboration
as concrete problems that more dialogue did not reliably repair.

## Results

| Measure, summed over eight exchanges per arm | Peer | Own evidence |
| --- | ---: | ---: |
| Supported source items after round 1 | 23/42 | 23/42 |
| After round 2 | 23/42 | 23/42 |
| After round 3 | 23/42 | 24/42 |
| Model requests | 136 | 130 |
| Executed searches | 64 | 65 |
| Input tokens | 138,792 | 132,897 |
| Output tokens | 140,624 | 185,967 |
| Supported admitted claim occurrences | 36 | 34 |
| Contradicted admitted claim occurrences | 6 | 10 |
| Unresolved admitted claim occurrences | 6 | 13 |
| Explicit no-evidence answers | 19 | 16 |
| Ungrounded answers | 3 | 3 |

Coverage deduplicates reference episode IDs within an exchange. The denominator
is 21 reference items across four cases, repeated at two seeds. Source items may
overlap in meaning; these are **not 42 distinct world facts**, and partial episode
coverage is not complete proposition recall. Claim occurrences can repeat across
rounds, so they are a different denominator from coverage or distinct audit IDs.

| Case | Seed | Peer coverage | Own-evidence coverage |
| --- | ---: | ---: | ---: |
| Complementary evidence | 1 | 3/6 | 3/6 |
| Complementary evidence | 2 | 2/6 | 3/6 |
| Conflicting evidence | 1 | 3/6 | 3/6 |
| Conflicting evidence | 2 | 4/6 | 4/6 |
| Namesake identity | 1 | 2/3 | 2/3 |
| Namesake identity | 2 | 2/3 | 2/3 |
| Temporal change | 1 | 3/6 | 3/6 |
| Temporal change | 2 | 4/6 | 4/6 |

The one additional control item was `complement-1`, a correctly qualified Harbor
handoff dependency. There were no peer-only supported items in any pair.

Stopping after the shared first round would have consumed 32 requests, 19 searches,
40,718 input tokens and 31,956 output tokens per arm. Peer rounds 2–3 added
**104 requests, 45 searches, 98,074 input tokens and 108,668 output tokens**, with
zero additional supported items. Own-evidence rounds 2–3 added 98 requests,
46 searches, 92,179 input tokens and 154,011 output tokens for one additional item.
Question generation is charged to the answer round it prepares. These are prefix
costs, not a separately randomized one-round treatment.

All 16 exchanges completed under unchanged frozen inputs and recorded server
settings. There were no provider request errors or malformed tool arguments.
One control answer (`conflict-1-own_evidence`, Bee) reached the 32,768-token response
limit; its full cost and resulting behavior remain included. This explains part,
not necessarily all, of the larger control token spend. Completion of the exchange
is not proof that every individual response finished normally.

## Conditions and judgment

The native `gossip_compare` actor ran the production exchange orchestration with
real MCP `kg_search` execution over four frozen synthetic worlds: complementary,
conflicting, temporal and namesake evidence. Two readers had disjoint Slack/Bee
lenses and identical date boundaries. Each case used seeds 1 and 2 in a registered
shuffled pair order with alternating arm order. Both arms had three rounds and a
32-request ceiling per exchange; actual requests, searches and tokens were outcomes.

Peer askers saw both readers' admitted evidence and questioned the peer. Control
askers saw only their own reader's admitted evidence and questioned that reader,
with a coherent self-followup role instruction. The normal CLI retains peer mode.
Initial answer requests were exactly identical in all eight pairs; their measured
first-round usage and coverage also matched. Retrieval was deterministic lexical
ranking with a two-hit cap, independent of arm, seed, history and gold. Gold was
never returned by a tool. No operator rules, private sources or live graph were used.

The local endpoint served `qwen3.6-35b-a3b` with four slots and 262,144 context tokens
per slot. Requests enabled thinking, selected high effort and allowed 32,768 output
tokens. The server reported temperature about 0.6, top-p about 0.95 and top-k 20;
full provider and server settings are preserved. The actor was copied into the
run directory before dispatch, avoiding changes from concurrent Cargo builds.

An arm-blind packet contained 55 distinct structurally admitted claims and their
quotes, full source episodes, target and reference items, with arm/seed/round
removed. The primary assistant graded every claim; a second assistant reviewed
all draft judgments without reading outcomes or arm data. Before unblinding, two
ambiguous North/South identity contrasts changed from contradicted to unresolved;
neither receives target coverage under either reading. The final distinct-claim
counts are **36 supported, 12 unresolved and 7 contradicted**. This is assistant
review, not independent human evaluation. See [the review decisions](audit-review.md).

Literal quotation was insufficient: claims assigned the other Rhea Moss's Ember
activities to the North lab target, converted a planned chair role into actual
tenure, or inferred that a prerequisite was still pending. Wrong-person claims
were contradicted; insufficiently established elaborations stayed unresolved.
The audit covers admitted claims, not every line of raw output. It does not run
production extraction, origin attribution, final adjudication or graph filing,
and does not separately grade joint discovery of contradictions.

Four synthetic worlds and two seeds cannot establish live semantic-search quality,
held-out task-family transfer, independent source origins or a reliable production
winner. Followups changed available evidence and the matching role prompt together.
The observed results support another registered test of identity-aware retrieval
and claim admission, not a claim that dialogue can never help.

## Evidence and reproduction

[The scorecard](scorecard.json), [round costs](round-costs.json),
[verification](verification.json), [registration](design.json) and
[finish audit](finish.json) preserve the calculations and execution conditions.
`trials/` contains all configurations, exchange results and compressed complete
provider/search receipts. `audit-packet.json.gz`, draft/final verdicts and the
review note preserve judgment provenance. `cost-analysis.py` derives round costs
without model calls. `archive-sha256.json` checks every archived file except itself.

The frozen `runner.py`, fixture `server.py` and `cases.json` are included; the
native actor source is `mecha-core/examples/gossip_compare.rs` in the accompanying
commit. Its executable hash is recorded, but the large compiled binary is omitted.
For a fresh model run, follow `eval/fixtures/gossip/README.md`. To recompute this
report, decompress each `*.jsonl.gz` and `audit-packet.json.gz` to its unsuffixed
path in a temporary archive copy, then run `python3 runner.py --out COPY --report-only`.
Paths in registration identify the original run, not portable installation paths.

A final reporting review tightened the current runner to reject duplicate verdict
IDs explicitly; the frozen execution runner is preserved unchanged. The regression
failed before that check and passed afterward. This archive has 55 unique verdicts;
old and corrected reporters produce identical scores, with no model rerun.

Three invalid setup attempts are retained under `prior-attempts/` and excluded
from scoring: v1 used a doubled `/v1` URL and produced 16 HTTP 404s with no model
completions; v2 had an inconsistent control role and was stopped before grading;
v3 used a mutable Cargo binary that changed during tests and was stopped before
grading. Their designs, frozen runners and synthetic receipts are preserved;
old compiled binaries are not. No completed valid trial was retried or selected out.

All-target build, formatting and warning-free Clippy passed. Required-backend
workspace tests passed **2,628**, with zero failures and three ignored, including
the native protocol-control and actual MCP fixture regressions. The fixture
integration also executes six Python retrieval/report checks. No installation,
service restart or configuration promotion was performed.
