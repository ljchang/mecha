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

### Chronological ordinary-memory evaluation (2026-11-10, development cohort)

`ordinary_lifetime.py` freezes the ordinary CLI executable, inputs, scorer,
settings and full schedule for three empty-start arms: no memory; accumulating
episodes with concept writes frozen; and accumulating episodes with one induction
and one revision. Its seven authored episodes per arm yield twenty-three slots
and twelve decision artifacts per seed. The recording transport verifies actual
recall delivery; scoring keeps raw correctness, protocol qualification, citation
membership, source coverage and costs separate. One family is development
evidence, not a held-out test; repeated seeds do not enlarge the family count.

Review corrected mutable current inputs and system-prompt indirection, delayed
requests crossing slot boundaries, uncertain upstream completion, unknown token
usage bypassing the budget, undelivered recall receiving credit, unbound artifact
scoring and unlabelled raw paired wins. All failed attempts remain preserved.
The first actual-CLI smoke also exposed a setup distinction: memory-enabled
observations must register `goal_context`, while independent evidence acquisition
must never call it. Keeping that distinction retained the core startup guard.

The final source passed independent incremental review with no medium or major
finding. Spark qualification passed both all-target builds, 6,426 default and
7,201 memory-enabled workspace tests, 397 standalone tests, strict all-feature
and standalone Clippy, formatting and Rust 1.96 compatibility. Workspace suites
retain 25 and 30 ignored entries respectively, with six optional-backend early
returns in each reported pass count. Full Python qualification passed 164
top-level and 257 nested authored tests with zero skips. Two earlier isolated
Python setups omitted Git metadata or required repository fixtures; those setup
failures were preserved before qualification from the complete source tree.

The copied full-build CLI has SHA-256
`fa52e1d00ea337f5aebc3ef3c920da4e238f16d44da70b2a237cc730b18ab2e1`.
Its nine-slot authored smoke retained two observations, refused two source-input
replacement attempts and delivered native recall to one later decision. A
malformed learning reply remained a failure. These are mechanics checks;
resident-model decision benefit is a separate measurement.

The first resident development attempt completed all twenty-three scheduled
slots. Raw correct decisions were 2/4 without memory and 4/4 in both memory arms;
protocol-qualified correctness was 2/4 in every arm. All eight memory decisions
received nonempty recall. Four supplied the native optional `serves` argument,
which violated this benchmark's stricter exact-query contract without being a
native recall failure. All nineteen submitted citations named delivered records;
seventeen quotations were exact, but every original-field pointer was wrong, so
none earned citation credit. Six decision captures exceeded the existing
1,024-byte tool-input limit and remained unavailable. Capture availability is
separate from successful recall and correct choices.

Induction never made an inference POST: the recording proxy changed bare router
properties into model-specific properties, correctly triggering the native
resident-router refusal. Revision consequently had no concept to revise. This
was an adapter failure, not a measured failure of model concept formation. The
independent audit also found genuine delivered raw originals omitted from the
citation map. Both defects received focused regressions and corrections; mapping
additional originals does not give receipt copies independent observation credit
or retroactively validate the wrong pointers in the first attempt. The strengthened
actual-CLI smoke now requires one acknowledged learning POST and distinct router
and model-property envelopes.

The first attempt's 102 calls used 276,666 input and 33,103 output tokens with
complete wire usage; summed slot time was 371,958 ms. Learning itself used zero
model calls, verified from dispatch records, so those figures do not estimate
successful learning cost. This single authored family cannot establish adaptive
benefit; its manifest, denominator and original errors remain preserved.

A separately frozen corrected attempt also completed twenty-three slots. The
resident model retained one provisional concept and returned a valid abstention
from rumination, with one acknowledged model call each; it did not revise the
concept. Reference-matching saved choices were 1/4 without memory and 4/4 in each
memory arm. Protocol-qualified correctness was 1/4, 2/4 and 2/4 respectively;
all eight memory decisions actually received nonempty evidence. The retained
concept appeared in none of their acknowledged packets. `context::assemble`
searches concepts too, but the family-identifier query matched none of the
name/principle/applicability terms indexed by `sql::append`; the source episode
identifier is not inherited into that index. This is not a test of decision
benefit from using concepts. The no-memory arm lacks prior observed
route behavior; reference matching does not separately grade whether its
abstention was appropriate given that information deficit.

Only one of eighteen submitted citations was valid: an authenticated receipt-copy
read, not the independently selected observation source. Independent-source
coverage remained zero. All six memory observation captures were retained, but
all eight memory decision captures exceeded the 1,024-byte write-input bound and
were unavailable. These limitations remain separate from raw decision quality.
Across 102 calls, complete wire usage was 228,859 input and 25,674 output tokens;
summed slot time was 301,479 ms. Learning's two calls accounted for 2,640 input
and 720 output tokens and 11,493 ms. Same resident alias and context geometry were
confirmed before and after; this is not independent weight attestation.

### Explicit learning from ordinary memory (2026-11-10, opt-in prototype)

`memory_session::learning` connects authenticated ordinary captures to the
existing native concept and rumination APIs. Host-only `memory-session sources`
returns bounded original descriptors and exact tool-call bindings; `learn` accepts
explicit source selectors or a current concept and opposing quotations. It keeps
the same encrypted store, access scopes, source guard and provisional status.
No schema, model weights, production activation or nightly schedule changed.
Recovery handles cannot inspect sources or learn. One callback carries an opaque
prepared receipt across inference, rechecks after provider discovery and after
both successful and failed calls, then separately reports retention, revision,
unchanged, abstained or failed. Source quotation membership is not entailment.

Independent review found and corrected four boundary errors before publication:
resident discovery did not substitute for checking the returned model identity;
source inventory needed full-closure authorization rather than a complete-packet
membership test; tool bindings needed authentication of the exact parsed byte
buffer; and absent token counters could not become known zeroes. Learning reports
conservatively mark zero counters unknown because the shared provider type loses
wire presence; wire-based evaluation retains that distinction. Raw rejected
replies and available usage remain auditable. Model and inventory failures were
reproduced against the unfixed implementation, and the first attempts remain
preserved. Two test-fixture corrections retained their exact assertions: select
the final statement by authenticated kind/text rather than a guessed passage
index, and use a unique query when testing provenance rather than search ranking.

The reviewed Spark snapshot passed ten learning integration cases, sixteen actual
CLI cases, the exact-buffer unit case, twenty existing ordinary lifecycle cases,
four native integration cases, five bridge cases (one separate ignored entry),
twenty-seven native concept unit cases, formatting and strict scoped Clippy.
These are authored-provider and deterministic checks, not a measured learning
benefit. The copied qualified binary is SHA-256
`1de7e40101ae58a5c8bfe3312752bd180811ed886a578e2ebd9f926db3bdfb8a`;
broader qualification and the chronological comparison are recorded separately.


### Opt-in ordinary-session memory (2026-11-09, prototype implementation)

`memory_session::SessionMemory` attaches fresh ordinary Run recordings to the
existing encrypted v8 store through an explicit owner manifest. The CLI
`run --episodic-memory` uses the existing Agent, permissions, taint, budgets and
cancellation. Host `memory-session init`, `retry` and `forget` do not initialize a
provider. Recovery can outlive the task directory; retry only admits an immutable
seal and cannot repeat actions. No production activation, personal-graph import,
chat/resume integration or automatic concept scheduling is implied.

The separate `ordinary-session-observation-v1` producer authenticates attribution
without inventing an artifact check: `capture_semantic_assessment` returns
NotEvaluated. Original owner requests, direct-tool inputs/results and assistant
statements retain their source locators. Assistant Thinking and opaque signatures
stay only in the full hash-bound transcript, never evidence projections; later
source edits withhold recall. Completed answers may have no tool receipts when
there were no invocations. Unknown/partial records still refuse capture.

Durable recall-attempt records must reconcile with actual delivered results.
Every retained passage inherits all consulted references, not just model citations.
Review and actual subprocess fixtures found two important boundary defects:
forgetting a revision made an empty closure appear authorized, and an owner-defined
subagent could shadow a permitted built-in tool name. Exact canonical membership
now gates delivery validation; preflight rejects effective subagents and selected
outbox routes, and checks the actual registry's delegation/program/deferred traits.
The first failing evidence is preserved. No standing policy is silently stripped.

The authored two-process CLI fixture checked actual HTTP request bodies, JSONL,
artifacts and encrypted SQLite rows. A new goal/workspace recalled the first
session's tool observation; every retained descendant carried its source closure.
Withdrawal between delivery and finalization prevented a new seal/retention while
preserving the completed action. A real SQLite lock produced Pending; a fresh host
retry under broken provider config admitted the exact seal without another model
call or action. Recovery survived removal of the historical workspace. Fatal
provider errors retained the native error and partial trace, with no fake Outcome.
Independent task reviews passed. Local qualification passed 14 actual CLI cases,
21 lifecycle cases after the numeric fix, five bridge cases (one ignored child entry) and four native
integration cases, strict core/CLI all-feature Clippy and formatting. The baseline
macOS linker warning remains distinct from Rust/Clippy diagnostics.

The first combined Linux run preserved a real recording failure: eight CLI cases
passed and five returned recording_unavailable despite completed actions. A
separate frozen-binary diagnostic isolated one native Outcome field: load average
serialized as `0.65`, while `serde_json::to_value` widened the typed f32 to
`0.6499999761581421`. Strict validation must compare through the session writer's
text serialization, rather than equate typed-to-Value conversion with wire JSON.
The original failure and diagnostic remain retained; no model was run on that
binary.

The first resident-model setups stopped before inference: the harness initially
used a database filename without the required suffix; after correction, Clap
rejected required global options placed before `run`. These setup failures are
preserved separately from model-performance evidence; neither executed an action
or called the model. Global-option enforcement now runs against merged CLI
options before configuration/provider startup; both placements reach the same
manifest validation and omitted requirements still fail closed.

The fresh resident-model workflow then passed with the qualified CLI: two new
processes, three inference requests each, different goals/workspaces and no answer
in the follow-up prompt. Both observations were retained as NotEvaluated; the
follow-up received the original attributed file-read result and wrote the matching
artifact. All seven delivered references matched tracked and sealed dependencies.
Host retries returned the original receipts with zero inference; forgetting the
first episode purged 13 objects/revisions and withheld retry of the dependent
second capture. Original transcripts and task artifacts remained unchanged.
The resident model was unchanged. This is one mechanics trial, not a memory-benefit
comparison or evidence of general learning.

Independent raw-evidence review verified all six complete responses and source
receipts. Recorded usage was 5,743 uncached plus 9,142 cached input tokens and
878 output tokens; the two CLI runs took 13.259 seconds combined. Monetary cost
was unpriced. Three reasoning blocks per run remained audit-only; the follow-up
retained both private and untrusted taint.

Final Spark qualification (2026-11-10) passed 7,188 memory-enabled workspace
tests with zero failures and 30 ignores, a fresh all-target build, formatting,
strict all-feature Clippy and Rust 1.96 compatibility. Exact postimage/feature-gate
comparison retained passing evidence for unchanged default code (6,426 tests,
25 ignores), 397 standalone tests and strict Clippy, 124 Python CLI and 257
authored Python tests. Each workspace count includes six built-in OCR/layout
early skips; these are not verified integrations. Separate strict supported
backend checks passed 60 cases (one ignored), and local-document checks passed
four with six declared unsupported cases filtered. Source integrity and the
complete evidence chain verified. The current runtime remains opt-in; no
production installation or owner-data trial occurred.


### Bounded native completion feedback (2026-11-09, prototype implementation)

`CompletionObserver` and `CompletionReport::observe` add one optional operational
notice inside the existing Agent loop. `run_native_session_with_completion` and
the thin `--completion-request` adapter select it explicitly for synthetic native
sessions. The original budget, taint, tool allowances and source-capture path
remain in force. Public row/key shape is independent of hidden expected values:
a wrong string answer can be operationally ready. A failed consumed recall cannot
buy another attempt. Accepted diagnostics survive later execution/capture errors;
an appended notice alone is not evidence of provider receipt.

Independent review found a provenance regression when an empty reply appended
a harness nudge before a queued owner steer. Whole-message notice equality missed
that mixed case. The fix checks every prior block's registered harness voice before
clearing the message marker; capture still excludes the notices individually.
The regression failed before the fix and passed afterwards. Retained scoped
checks passed 19 completion tests, 10 native completion tests, 25 covering workflow
tests and four new process-level CLI tests; combined strict core/CLI Clippy passed.
The exact-source Spark gate also passed both all-target builds, formatting, strict
workspace/standalone Clippy and actual Rust 1.96.1 checking. It passed 6,382 default,
7,099 memory-enabled, 386 standalone, 124 Python CLI and 257 authored Python tests,
with zero failures and 25/30 existing workspace ignores. Required backends remained
enforced. The actual authored CLI gate separately passed three captured seed
experiences and eight follow-ups. On repaired all four omitted outputs, including
two supported choices and two unknown controls; off retained four omissions.
These scripted responses establish integration, not model quality.

The separately frozen Qwen3.6 uncensored comparison used four paired decisions
from three source families, with lexical recall fixed and only completion feedback
varied. Independent raw-artifact/source audit agreed with the tracked scorer:

| Measure | Feedback off | Feedback on |
|---|---:|---:|
| Correct saved artifacts | 3/4 | 4/4 |
| Supported positive choices with original evidence | 2/2 | 2/2 |
| Correct saved unknown controls | 1/2 | 2/2 |
| Completed recall and valid capture | 4/4 | 4/4 |
| Missing artifacts | 1/4 | 0/4 |

Only C03-on needed a reminder: after the model ended without a file, the fixed
missing-artifact/no-repeat-read notice reached the provider on call 3; the next
tool write saved `unknown`, and the final observation was Ready. Both C03 sessions
had already produced the identical premature chat answer `{"choice":"unknown"}`;
the reminder repaired file delivery/format, not the semantic choice. C03-off retained
its missing artifact. Both arms excluded withdrawn evidence and retained complete
consulted-source dependencies. All four initial request contracts matched. These
fresh cases have explicit operation domains and permitted alternatives; their
scores cannot establish a gain against the earlier, different vector cohort.
This is a small completion benefit, not general reasoning, adaptive-learning or
vector superiority. Ordinary chat/web and production memory remain unchanged.

All 24 follow-up calls have complete recorded usage. Off used 16,093 input / 580
output tokens in 11 calls and 14.678 seconds of summed dispatch time; on used
20,014 / 838 in 13 calls and 16.804 seconds. Three scripted seed experiences were
shared acquisition, not learned-model successes; initialization, acquisition and
withdrawal costs were recorded separately. Source, executable and host runtime
identity checks passed before and after the single cohort invocation. No scored
session was retried or overwritten.

Experiment review corrected start-only requests being mistaken for provider
receipt, top-level-only withdrawal checks, malformed Unicode/depth escaping
slot-local scoring, unstarted artifact credit, foreign transcript paths, and an
authored gate that could pass without the requisite evidence. Their failing
regressions and first gate failure remain recorded. Sequential authored HTTP
packets differed at the live `memory.valid_at` field; the final gate compares all
other parsed fields exactly and records both timestamps and their delta. It never
normalizes record/event/interval times or weakens scored initial-request parity.
Real second requests differed in tool-call IDs and live validity metadata; only
the initial provider contract is required to match in the scored comparison.

### Controlled native vector recall (2026-11-09, prototype branch)

`GoalMemory::with_native_recall` and `run_native_session_with_recall` connect
host-selected lexical or checked dense recall to the existing native Agent,
artifact checks, deterministic appraisal and v8 capture. The thin CLI accepts an
explicit `--recall-request`; legacy callers and ordinary chat/web remain unchanged.
One exact query and one atomically claimed entered call share the same three-anchor,
zero-neighbor, 20,000-byte packet contract. The 15-second deadline covers retrieval
and publication. Cancellation or deadline observed at publication prevents success;
an unfinished dropped attempt records interruption. No SQLite lock or transaction
crosses embedding. Final delivery refreshes validity
and authorization while retaining the pinned knowledge sequence. No schema or
persistent vector index is introduced.

Review exposed late-ready deadline/cancellation publication, relations expiring
during embedding, missing inherited token preflight and loss of diagnostics when
capture could not reopen its store. Dedicated regressions now cover these paths.
The final focused checks passed 16 GoalMemory, 20 hybrid, 18 native core and two
CLI unit tests. The 21-process CLI suite passed before the final narrow fix; all
five affected process tests then passed again. Strict scoped core/CLI Clippy
passed. A local standalone-Clippy process lost its terminal receipt and is not
counted as passing; the full Spark gate includes an explicit replacement check.

The independently reviewed `native_vector_trial.py` freezes four authored native
experiences and eight lexical/dense pairs before real inference. Raw correctness
is separate from protocol-qualified decisions with decisive original evidence
both delivered and recaptured. Missing/withdrawn cases remain separate controls;
failed and unstarted slots remain in the denominator. Twenty-eight Python tests
passed. The actual CLI gate passed all four authored seed captures and sixteen
follow-ups, with eight matched initial request/schema pairs and, per arm, six
supported positive decisions and two appropriate no-evidence responses. Those
48 follow-up chat requests and 16 embedding requests / 225 inputs were authored,
not real model inference. A final optional-`serves` scoring fix preserved the first
run and its unchanged read-only re-audit; one additional actual CLI session with
explicit `serves` passed using three authored chat replies and no embeddings.
Final independent review found no remaining major/medium issue.

The exact candidate completed full Spark qualification on 2026-11-09: default
workspace 6,363 passed / zero failed / 25 ignored; memory-enabled workspace
7,066 / zero / 30; standalone 386 / zero / zero; Python CLI 124 / zero / zero;
authored Python 210 / zero / zero. Both all-target builds, formatting, standalone
strict Clippy, workspace all-target/all-feature strict Clippy and actual Rust
1.96.1 checking passed. Required sandbox backends remained enforced. Source,
index and HEAD stayed unchanged throughout; final prose is reviewed separately.
The standalone-Clippy terminal result replaces the earlier missing local receipt.

The first model launcher stopped before seeds or model calls because an orphaned
shell from `process_loss_after_real_read_leaves_spent_indeterminate_reservation`
retained an inherited qualification lock. Its fixture directory had already been
removed, leaving it waiting for a release marker. The exact owned shell was
identified by command, deleted working directory and lock descriptor, then
terminated; the original failed preflight and cleanup receipt remain. The unchanged
release was launched with a new ID after confirming the lock released. No scored
request was replayed, and no model service or product code changed. Waiting for
test-child exit and excluding inherited orchestration locks are follow-ups.

The first admitted Qwen3.6-35b-a3b-uncensored / Harrier cohort dispatched all sixteen
planned sessions in one invocation, with 35 chat requests and no retries. The
independent raw-artifact audit agreed with all original scores. Correct,
evidence-supported positive artifacts were **1/6 lexical versus 0/6 dense**;
both arms failed both no-evidence controls. Fourteen artifacts were absent. Only
lexical P03 wrote the expected artifact; lexical N02 wrote the unrelated XQ9.
Each arm completed requested-mode recall and valid consulted-source capture in
7/8 sessions, delivering decisive original observations in 5/6 positive cases.
Both P04 sessions skipped recall, invented a method and had no capture. All
fourteen entered recalls completed without fallback. The withdrawn S4 closure
was absent from both N02 packets and recaptures. No method or evidence row was
reclassified after seeing the results.

All eight initial request/schema pairs matched. Independent inspection confirmed
the current criterion was present in every actual first system request, all
starting database/authority seals matched the planned state, including the S4
withdrawal marker in both N02 slots, and source/fixture/binary identities
remained intact. Before/after runtime checks agreed; declared aliases and public
artifact hashes are configuration evidence, not loaded-weight attestation. The
model emitted final JSON in chat for many tasks that required `fs_write`. Those
text choices are diagnostic, not substitute artifacts or a repaired score.
Lexical P01's LT8 choice also exposes a fixture limitation: all domains share one
goal, and “retains the entire record” is broad when archive evidence is retrieved.
Preserve its exact-gold failure without calling it an unambiguous reasoning error.
Missing/withdrawn cases showed unsupported cross-domain extrapolation; excluding
withdrawn evidence does not itself make the model abstain.

Lexical consumed 58,564 input / 678 output tokens across 20 chat calls and 24.801
seconds of summed dispatch time; dense consumed 18,936 / 347 across 15 calls and
18.906 seconds. Every started chat request has an accounted response. The seven
entered dense recalls used fourteen embedding requests / 196 inputs. Summed
recall-operation time was 0.162 seconds lexical and 3.823 seconds dense; the latter
includes embedding and finalization, not embedding alone. Skipped calls have no
embedding cost to invent, and unavailable counters remain unavailable. Faster
termination without an artifact is not an efficiency gain. File hashing preceded
inference, so this is not a disk-cold measurement. The trial used plaintext
synthetic fixtures; it does not qualify private or encrypted production operation.

This small comparison did not establish a vector decision benefit. Improve bounded
artifact-completion feedback using the existing checker, clarify task/alternative
scope, and freeze new cases before another comparison. Retain this first cohort
unchanged. Ordinary chat/web activation, persistent indexing and owner-graph
migration remain outside the increment. Exact-head GitHub checks and final
publication state are maintained on draft PR #846, separately from Spark results.

### Source-grounded handoff correction (2026-11-09)

The handoff still described the first concept comparison, programmatic tool calling
and first layout installer as unbuilt. Source inspection confirmed
`memory_concept_workflow::run_trial`, `tool::code::CodeTool` with setup registration,
and `install::install_layout` reached through `features::enable`. The obsolete
first-build items were removed or narrowed. This correction does not claim a new
runtime deployment or semantic benefit; the earlier concept measurements remain
unchanged. Broader operational narratives were not recertified by this audit.

A later source pass also narrowed failed-lookup capture and longer-packet open
items: retained recall diagnostics are not yet a typed non-reading capture receipt,
and the reconstructed 19-revision packing fixture already fits. Packaged router
launch no longer reads the working tree, and macOS memory sensing is implemented;
live unit overrides and older clipboard behavior were not recertified. Broader
operational narratives remain dated rather than newly verified.

### Checked embedding transport (2026-11-09, prototype branch)

`EncodingProfile` and `Embedder::embed_checked` add immutable encoding identity,
complete mixed query/passage batches and strict indexed response validation.
Checked calls reject oversized inputs and malformed vectors rather than silently
truncating or returning partial batches. Request/response caps, a shared attempt
budget and one operation timeout cover overflow splitting. Legacy persona API
and response compatibility remain unchanged; both paths disable redirects and
system proxies. Artifact revision is host-declared, distinct from server alias.

An authored redirect-spy test first failed against the old client: a 307 response
forwarded the submitted text to the second server. The final source passed 13
embedding tests and nine persona search tests, strict scoped core Clippy and
formatting. The independent patch review found no major/medium defects. Failed
checked calls intentionally expose no partial transport counters; downstream
reports must use unavailable values rather than zero. These are local synthetic
mechanism checks, not full-workspace qualification or evidence of better retrieval.
The separate hybrid selector then reused the existing recall budgeting and context
assembler through `prepare_hybrid_context` and `finish_hybrid_context`. It ranks
independent dense candidates, checks the complete eligible view after embedding,
and recomputes lexical context with the same mention constraint on bound,
availability or corpus-change fallback. Original fields remain complete bounded
UTF-8 spans; vectors live only for the invocation. BM25, cosine and RRF diagnostics
remain distinct and only describe anchors retained in the authorized packet.

Review requested a missing semantic-route concept regression. The added v8 test
covers two cited supports, an uncited consulted dependency, complete bundle
assembly, tight-budget withholding and current scope narrowing after preparation.
Final focused checks passed 499 core tests (one existing ignored), 383 standalone
tests and strict feature-core Clippy; all 14 hybrid tests passed in both builds.
Independent review and the scoped test-fix review found no remaining findings.
The initial RRF test oracle was corrected after its 10/11 run: `1/61 + 1/63` is
slightly greater than `2/62`. An exploratory broad test filter also hit 15 unrelated
sandbox listener restrictions; these were not counted as passing tests. The
comparison workflow and combined workspace qualification remain separate.

Before any scored request, the 40-record fixture, 16 nonempty-gold cases, two
empty controls and metric protocol were frozen. A single separate fixed-input
Spark probe returned the expected Harrier alias and 1,024 dimensions. The running
service's arguments and a fresh public GGUF hash corroborated its configured
artifact; the HTTP alias is not weight attestation. No benchmark query or chat
model request was sent in this probe.

`memory_hybrid_workflow::run` and the thin `memory evaluate-hybrid` command then
connected the frozen fixture, checked client, all three retrieval modes and
explicit scoring. Twenty focused core/selector tests, three actual CLI tests,
the checked-client deadline regression and strict scoped Clippy passed.
Independent review recomputed all 54 authored treatment rows and three summaries
and found no major/medium issue. A minor follow-up is fuller per-case submitted
passage membership assertions in the HTTP spies; current checks cover counts,
selected examples and restricted-input exclusion. The authored loopback run is
mechanism evidence, not embedding-model quality.

The first combined Spark run passed formatting and the default all-target build,
then stopped with four document-backend failures: its isolated `MECHA_HOME` hid
the installed layout prerequisite. Required-backend enforcement correctly refused
to skip these tests. An explicit `MECHA_TEST_LAYOUT_DIR` selected the verified
installation without changing product source or weakening enforcement. Cargo's
target inventory reconciled 40 wholly passed groups with 23 resumed groups,
replaying the entire failed target and excluding its earlier partial counts.
Default qualification passed 6,363 tests with 25 ignored. The subsequent complete
memory-enabled workspace suite passed 7,041 tests with 30 ignored. Both all-target
builds passed; standalone passed 386 tests, Python CLI qualification passed 124,
and authored Python tests passed 182, with no Python skips. Final lint and MSRV
gates and the real scored comparison remained pending at this checkpoint.
The subsequent strict all-target/all-feature Clippy and actual Rust 1.96.1
all-target/all-feature check passed. The terminal qualification receipt verified
unchanged source, index, backend prerequisite hashes and the preserved preceding
checkout. The reviewed source delta was
`f1f4f2abfba04df964339ab102a3a572346430360b9aad30d56db34a51431690`;
the frozen feature CLI was
`8645cd51508ef6616e3dec2d75d6f1ca3b288f355e1824462d25d7ee9c11e182`.

Before scoring, independent runner/auditor review corrected an audit-only error:
a semantic preparation failure still has a known shared preparation duration.
The auditor had incorrectly expected null derived time. A hand-authored negative
regression failed before the correction and passed after; fixture, protocol,
metric definitions and product source remained frozen.

The first real Harrier comparison used that frozen executable and the predeclared
40-record fixture, 16 nonempty-gold cases and two empty controls. All 54 treatment
rows completed, with no fallback, error, unauthorized return or literal-mention
violation. Independent audit checked the report against 39 surviving original
database records and the forgotten source's suppression. Complete evidence
coverage was **12/16 lexical, 16/16 dense and 15/16 hybrid**; mean evidence coverage
was 0.75, 1.0 and 0.9375, and MRR was 0.75, 1.0 and 0.890625 respectively.
The four paraphrase cases accounted for the difference: lexical completed none,
dense all four and hybrid three. Every arm completed all four exact-identifier,
multi-evidence and policy-distractor cases. Hybrid missed P04 and ranked P03's
gold fourth; these results do not support claiming fusion was the best method.
E01 returned four irrelevant records in every arm; E02 returned none and made no
embedding request. No abstention-quality or downstream decision claim follows.

The invocation took 7.770758 seconds, including 6.683298 seconds of shared
embedding and 0.143384 seconds of preparation. Seventeen embedding operations
made 43 HTTP requests for 503 inputs (40,037 request bytes, 10,940,100 response
bytes). Independent lexical retrieval totaled 0.126565 seconds; dense and hybrid
finalization totaled 0.196611 and 0.181271 seconds. Their derived totals include
the same shared cost and must not be added as independent experiments. The
backend began inactive; artifact hashing had already read the model file, so this
is not a disk-cold latency claim. Vectors were not persisted. The frozen report
SHA-256 is `68138f89c815987764ec06b9d1ede565ce9cc4b2bbe301104833e9f7998716d5`;
the independent audit receipt is
`b0d8722d78a0cc4f74b189ddf250235d74957416c8a4b917169101991f00c089`.

The scored CLI exited zero, but the original runner exited one: its metadata
postflight required socket SubState `listening`, whereas an activated socket
reported `running`. The first failed receipt remains intact. A separate read-only
postflight confirmed active socket/proxy/backend, kernel listeners, unchanged
unit/launcher/executable/artifact hashes and the expected live model arguments.
Its receipt is `4007812c987df41469038bdf4cc710c1a66c04134563483d3811244e8243d1d0`.
This later corroboration is separate from the original runner outcome. No model
request was repeated and no cases, thresholds or rankings were tuned after the
results. Local evidence is under `results/memory-hybrid-retrieval-20261109/`.

### Encrypted native memory and bounded historical recovery (2026-11-09, prototype branch)

`StorageAccess` carries an opaque, zeroizing host capability through memory opens,
independent interpretation/action/outcome guards, GoalMemory and native execution
and appraisal. `KeyFilePolicy` excludes model-readable roots and filesystem
aliases, validates private descriptors, and refuses macOS extended ACLs and
ownership-ignoring volumes before loading or writing key bytes. Key generation
is exclusive; wrong or missing keys never select plaintext. The shared engine is
SQLCipher 4.19 / SQLite 3.53.4 through locked rusqlite 0.41.1 and vendored OpenSSL.
Existing stores remain plaintext unless explicit storage access is selected.
HUD's SQL authorizer refuses the newly available export function at preparation.

`Memory::backup_encrypted` copies the complete encrypted v8 database through a
private keyed export connection under a pinned source transaction. Schema, typed
row inventories, FTS shadows and sequence state are independently verified before
synced no-clobber publication. `restore_encrypted` reuses that exporter and the
same `forget::purge_seeded` routine as ordinary forgetting. Current suppression
and restricted IDs seed the full dependency/capture closure; post-backup forgotten
objects retain only opaque restriction stubs. A current-authority IMMEDIATE
transaction spans verification/publication. Every survivor must retain all
current-head access scopes, even when the deletion epoch did not change.

The explicit `key-generate`, `backup` and `restore` CLI paths keep storage
capabilities out of model requests. Restore creates a separate offline historical
copy and reports snapshot/current sequences. It does not merge newer facts or
withdrawals, replace a live store, recover external files, recreate process-local
permits or propagate future forgetting to retained copies. Current authority is
required; an old snapshot alone cannot establish later deletion decisions. Full
disaster recovery and private-data qualification remain open.

Each of the three task patches passed independent review. E1's revised macOS
checks covered key policy, native/guard reopens, CLI and standalone behavior;
five frozen prior-engine compatibility groups and an optimized release CLI smoke
also passed. Its scoped Spark gate passed 738 tests (0 failed, 6 ignored), thirteen
actual encrypted CLI checks, strict Clippy and the all-target/all-feature check
on Rust 1.96.1. E2's ten snapshot tests covered copy/inventory corruption, writer
contention, path replacement and publication faults. The final E3 source passed
416 affected core memory tests, four native integrations, sixteen CLI tests and
373 standalone tests, plus strict all-feature Clippy. These are scoped counts,
not combined full-workspace results. E3's initial CLI attempt failed four local
listener binds under the sandbox; the authorized loopback retry passed unchanged.
The initial failure details remain in the task report. Evidence lives in local
`results/memory-encrypted-storage-20261109/qualification`. The final combined
source also passed an optimized macOS CLI build and 29 actual CLI checks with
four independent keys, backup/restore, refusal/no-clobber paths and current
suppression-epoch reconciliation. That release smoke used an empty synthetic
store; the native/core/CLI suites above exercise retained and forgotten content.
All 1,465 source-manifest entries remained unchanged.

The final combined Spark run `combined-20261109T071728Z` passed 6,354 default
workspace tests (25 ignored), 7,008 memory-enabled tests (30 ignored), and 372
standalone tests (none ignored), with zero failures. Formatting, the all-target
build, feature CLI build, 29 actual CLI checks, strict all-target/all-feature
Clippy and the corresponding workspace check on Rust 1.96.1 passed. The core
source package included the qualified recovery sources and third-party notices;
package verification checked thirteen exact source/notice postimages. Qualified
source and index stayed unchanged throughout. Final documentation is a separate
prose-only delta; PR #846 records publication and exact-head CI/review status.
No real-model inference, private-source import or production activation occurred.

### Optional cipher dependencies after review (2026-11-09, prototype branch)

The `114f33ddf` GitHub run passed all eighteen CI jobs and documentation checks,
but automated review correctly found an avoidable default-build dependency cost:
SQLCipher/vendored OpenSSL and the direct key helpers were unconditional while
memory was default-off. Workspace rusqlite now selects ordinary bundled SQLite;
core's existing `memory-prototype` feature activates the cipher backend and the
optional direct `getrandom`/`zeroize` dependencies. CLI already forwards the core
feature, so there is still one SQLite linkage per build. The standalone harness
continues to select SQLCipher, and the HUD authorizer's export denial remains
unconditional. Memory-enabled runtime source is unchanged; the ordinary
build now selects a different SQLite backend and requires fresh qualification.

A new default-only runtime regression first failed on the unconditional cipher
engine, then measured ordinary SQLite 3.54.1 with a working FTS5 MATCH and no
cipher-version result. The memory-enabled backend remains SQLCipher 4.19.0 /
SQLite 3.53.4 with the same resolved dependency graph and runtime source. The two
cipher-positive HUD tests are now feature-gated. Independent scoped review found
no major/medium defects. Local default HUD 102, memory-enabled HUD 103 and nine
selected encrypted-storage tests passed, plus strict Clippy in both modes.
Existing environment-dependent HUD self-skips were recorded separately from
Rust's ignored count; a loopback EPERM attempt was retained before its successful
authorized retry. The fresh ordinary-SQLite workspace suite passed 6,353 tests,
zero failed and 25 ignored. The net decrease of one from the preceding default
suite is two cipher-only tests omitted and one ordinary-engine regression added.
The preceding full 7,008-test cipher-workspace result remains attributed to
`114f33ddf` rather than being relabeled as a rerun. The frozen
`engine-split-20261109T083529Z` gate passed targeted feature HUD 3/0/0,
memory-library 655/0/1 and native/CLI 28/0/0 tests (passed/failed/ignored),
all 372 standalone tests and 29 actual encrypted CLI checks. Both all-target
builds, actual backend/FTS probes, strict Clippy, Rust 1.96.1 checking and
460-file source-package inventory passed. Source and index remained unchanged.

A separate bidirectional plaintext compatibility probe used the actual selected
SQLite 3.54.1 and cipher SQLite 3.53.4 artifacts. Each opposite backend recovered
committed WAL content absent from a main-file-only control, verified relational
and FTS5 content, enforced foreign keys and preserved deleted-high sequence state
(next ID 101 after deleting ID 100). The original backend then reopened the
checkpointed mutation; integrity and plaintext-header checks passed. This covers
that synthetic fixture, not arbitrary application schema or newer SQL syntax.

### Compact native context and concept-aware appraisal (2026-11-09, prototype branch)

`PreparedRecords` now presents authenticated native metadata as typed fields while
keeping canonical records and original text intact. Admission and serialization
use the same prepared representation; withholding clears its private cache.
Recapture authenticates the presentation against canonical rows and requires the
full dependency closure. Rich v8 consulted lineage accepts 64 references and
rejects 65; v6/v7 retain their 32-reference limit. A reconstructed fuller-citation
fixture changed from 20,352 canonical bytes to 18,438 reader bytes with the same
19 revisions under the unchanged 20,000-byte limit. It is not a byte-identical
reconstruction of the earlier 21,042-byte case. Larger individual sources and
mandatory bundles can still be withheld.

`NativeConceptEvidenceRequest` and `situated-native-concept-appraisal-v1` separate
facts from canonical provisional hypotheses. The parser requires exactly one
applicability assessment per hypothesis, rejects hypothesis IDs as factual
premises, and reuses checks-first artifact/verdict validation with the new request
identity. Every consulted source, including uncited/omitted evidence and exact
per-member scopes, remains sealed and freshly rechecked. The explicit read-only
`appraise_native_session` workflow reuses the existing local structured pass;
it writes no judgment and introduces no discovery or default activation. Its
16 KiB reply limit supports the authored 64-record/52-hypothesis finish case;
legacy appraisal keeps its 8 KiB limit.

Local focused qualification passed 614 memory tests and three integrated native
tests, with one existing ignored memory report writer, strict core Clippy and
formatting. Task and combined independent reviews found no actionable defects.
The first Spark pass completed 6,352 default and 6,960 memory-enabled tests
(0 failed; 25/30 ignored), both builds, formatting and actual CLI initialization.
Its standalone compile then exposed a shared test calling core-only generation
APIs. A scoped feature gate fixed only those assertions: the complete 64-record view with 52 hypotheses
and the 65-record refusal remain in both harnesses. Local standalone
330 unit plus four integration tests and the core boundary test passed; independent
fix review found no defects. The original failed run is retained. The resumed Spark pass then completed formatting, the one core boundary test,
all 334 standalone tests and strict workspace/all-target/all-feature Clippy.
Before/after source and index manifests matched in each pass; only the reviewed
test-feature gate separates the two trees. The full workspace counts above apply
to identical runtime/dependency code before that test-only fix, rather than being
relabeled as reruns on the final tree. Evidence is retained under
`results/memory-concept-context-20261108/qualification`. Final prose was added
after code checks; PR #846 carries current publication and exact-head CI status. Authored responses establish lifecycle and reference integrity, not
semantic entailment, applicability accuracy, learning or decision benefit. No new
real-model inference, private-source migration or production activation occurred.


### Integrated native memory sessions (2026-11-08, prototype branch)

The explicit `memory init-native` / `native-session` CLI and
`memory_situated_workflow::run_native_session` connect the existing agent executor
to rich v8 capture using a bounded host request. Actual jailed file writes feed an
independent artifact checker; no fixture overwrites the output. Started state is
synced before provider discovery, existing task directories refuse replay, and
execution/capture errors keep their report while making the CLI fail. Later
invocations reopen the same explicit store and source authority. The old situated
fixtures retain their v7 behavior. Normal chat/web memory and production deployment
remain unchanged.

Explicit native concept and rumination APIs resolve original fields, roles and
session identity through canonical captures. Multiple passages or prefixes of one
session cannot supply independent support. Selected inputs and complete metadata
projection closures retain uncited dependencies and exact scope restrictions;
preparation, retention and every rumination outcome recheck current authority.
Native revisions preserve prior sources and opposition. Canonical provisional
concepts may be recaptured without becoming instructions; concept/interpretation
ancestors disqualify an episode as independent induction evidence. Native appraisal
of a Concept-bearing closure still refuses explicitly.

The authored end-to-end test executes two independent supporting tasks, reopens
GoalMemory and uses cited artifact bytes in another real write, then executes a
successful write whose output fails the same confirmed criterion. Provisional
revision retains the in-scope counterexample and complete lineage. Reopening and
source withdrawal verify current recall and invalidation of reused descendants.
This is a mechanics result, not model concept learning or a decision-benefit result.

The first revised reader bundle measured 21,042 bytes across 20 records and was
correctly withheld by the unchanged 20,000-byte cap. Exact short verdict citations
and one selected opposing artifact produced a 19,562-byte, 19-record packet. Every
original and projection of the three source captures remained present; the opposing
write result remained in the inherited closure rather than being a selected
counterexample excerpt. An empty final assistant reply was rejected by the normal
loop during fixture development; the final fixture keeps a short visible reply.
Larger concept bundles remain a concrete presentation limit, not a solved scale
problem.

Observed regression tests failed on the old v8 capture path, guardless concept
admission, missing CLI command and incomplete execution's zero exit status.
Focused checks passed 313 standalone unit tests, seven situated-runner tests,
thirteen CLI tests and the integrated acceptance case. Separate task and combined
reviews found no major/medium issues. Two minor follow-ups remain: end-to-end
forgetting after concept reuse (focused forgetting and integrated withdrawal are
separately covered), and cleanup of native concept unit-test temporary directories.
Full qualification and first-attempt model smoke records are kept under
`results/memory-integrated-session-20261108`; PR #846 carries the exact-head check
status and the living Site records the latest measured outcomes.



### Equal-evidence concept pilot and memory review (2026-11-08, prototype branch)

`memory_concept_workflow::run_trial` adds a feature-gated synthetic lifecycle over
existing concept/rumination APIs. Two original episodes support one provisional
concept; three decision pairs compare it with the same original evidence. One
explicit counterexample drives a revision, followed by three new pairs with the
counterexample in both arms. The thin `memory_concept_trial` example fixes the
resident model, seed and bounded generation settings. No production default,
database schema, dependency, owner-graph bridge or instruction learner changed.

All fourteen planned slots persist before inference. A durable started record
precedes discovery, each slot admits only its first response, and existing roots
cannot resume. Phase database/fixture seals and current authorization checks run
before and after calls, including failures. Successful retention is recorded
before derived export. The offline Python scorer preserves twelve decision
and six pair denominators, enumerates unknown finite-world observations, verifies
actual artifact hashes/bindings, and separates raw choice, premise coverage,
structural admission, false certainty and unreviewed prose. Revised-phase utility
against sources alone does not isolate revision's causal contribution.

Focused qualification passed nine Rust tests; the separate authored lifecycle
export supplied fourteen schemas/responses, twelve contracts and six bound pairs.
The scorer passed twenty-three tests and independently verified those artifacts.
The dummy responses are adapter checks, not model-quality results. Spark then
passed workspace all-target build, strict all-feature Clippy and formatting;
6,352 default tests /25 ignored, 6,920 memory-enabled tests /30 ignored, 307
standalone tests and actual-binary Python suites of 124 and 182 tests with no
Python skips. Required sandbox backends were enabled. All fourteen authored
schemas compiled using the pinned converter (binary hash recorded with prior
commit metadata); the resident engine's converter revision was not verified.
Qualification records 681 source hashes and the executable SHA256 under
`results/memory-concept-benefit-20261108/qualification`.

The frozen Qwen3.6-35b-a3b-uncensored cohort completed fourteen first responses
without retries: one induction, six initial decisions, one revision and six
follow-up decisions. Seed 70510, temperature 0.6, thinking disabled, 6144 output
tokens and 540 seconds per stage were fixed. All six evidence pairs and every
source hash triplet matched; source tree, executable and router metadata stayed
unchanged. Independent checks verified 42 request/reply/provider artifacts and
SQLite integrity/foreign keys. The same pinned converter compiled all fourteen
actual schemas; this is separate from verifying the resident converter revision.

Both arms had 6/6 correct choices with sufficient cited premises and both made
2/2 required abstentions. An arm-masked independent AI prose audit found 5/6
faithful explanations source-only versus 4/6 with the concept. All four missing-
information questions were relevant. Both initial uncertainty explanations
misapplied prerequisites or conflated repair with ordinary routes; the concept
follow-up uncertainty explanation wrote a false cost comparison instead of
establishing infeasibility. Parent inspection confirmed the cited defects.
Correct labels and citations therefore did not establish fully faithful reasoning.

The retained revision added one counterexample and expanded consulted originals
from two to three. Principle, applicability and support excerpts were byte-equal
as strings/values: the initial concept already required shared readiness. This
was an evidence-enriched revision, not demonstrated correction of an incorrect
rule. The applicability's parenthetical alternative involving repeated failure
also leaves an untested cost-ordering boundary; fresh cases must vary that order.

Six decision calls used 5,788 input/1,292 output tokens source-only versus 6,791/
1,429 with concepts. Induction and revision added 1,641 input/760 output tokens.
Summed per-stage elapsed time was 17.149s baseline, 17.611s concept decisions,
plus 12.096s construction/revision; all usage was known. There was no observed
decision gain or efficiency gain in these six explicit-rule configurations.
The source-only ceiling, one response per condition and absence of an unchanged-
concept third arm prevent broad transfer or causal revision claims. Preserve this
cohort before designing harder fresh cases; no automatic learning was activated.

An independent review of Anthropic's memory-components article and official
examples found useful priorities that fit existing Mecha APIs: measure memory
discovery separately from use, revise from specific counterexamples, preserve
original evidence and count construction/revision costs. No new wiki, store or
nightly service was added. The research analysis and source links are in
`AGENT-MEMORY-SYSTEMS-RESEARCH.md` §12.

### Native appraisal consistency and verdict controls (2026-11-08, prototype branch)

`NativeEvidenceRequest::checked_input` and `checked_generation_request` add a
separate flat checks-first response; v1 APIs retain their original meaning.
`Memory::finish_native_checked_appraisal` checks v2 identity, authenticated artifact
roles/availability, issued citations, bounds and comparison/verdict consistency.
It rechecks the complete source closure and writes nothing. Unknown requires an
explicit question; execution-only criteria may leave artifact contents unassessed.
These are reference and consistency checks, not semantic entailment checks.

`memory_native_workflow::run_checked_case` compares typed direct and checked
responses with identical evidence. `run_verdict_controls` exercises all three
labels under both contracts with explicit literal-copy targets. Targets are never
used by semantic trials; exact matching and host acceptance remain separate.
Original replies, refusals, source hashes, usage and independent deadlines remain
in the audit. The existing example selects the modes explicitly with trailing
`--checks-first` or `--verdict-controls`. No production defaults or database
schema changed. The standalone harness enables serde_json insertion ordering so
its effective schema preserves the same state-last contract as the core build.
Source review found no stored-record hash change: nested action JSON remains
original text in stored captures. Standalone generation-input serialization and
recomputed request identities can change, so cross-build retry/dedup identity is
not promised. An externally authored old action envelope with unsorted input keys
and a sorted-key digest may fail re-admission; ordinary old writer envelopes keep
their sorted bytes. Core already preserved order. Cross-build execution was not
performed; this is a source-reviewed compatibility boundary, not a migration test.

Software qualification passed workspace/all-target build, formatting, strict
all-target/all-feature Clippy, 6,352 default workspace tests (25 ignored), 6,911
memory-enabled tests (29 ignored), 303 standalone unit plus four integration
checks, and 182 Python checks using both actual native binaries. Required sandbox
backends were enabled. The first Clippy pass rejected two new test-only
`keys().last()` calls; replacing them with `next_back()` preserved their assertions
and passed scoped review and Clippy. The successful default suite was carried
forward across exactly these two test-only changes; the memory suite ran afterward.
The original failure log is retained. Rust/manifests match the final qualified
snapshot; source and independent review found no unresolved major/medium issue.

The fixed Qwen3.6-35b-a3b-uncensored repair comparison completed 30 first
responses: six exact literal controls and 12 direct/checked pairs (eight reused,
four independently authored). All controls passed. Both arms used identical typed
facts, temperature 0.6, seed 70510, thinking off, 6,144 output tokens and independent
540-second deadlines. No responses were retried or prompts changed within the run.
Before any response, independent review clarified the fresh causal case to ask a
causal proposition rather than evidence adequacy; its observations were unchanged
and the final fixture/oracle hashes were frozen.

| Twelve development cases | Direct v1 | Checked v2 |
|---|---:|---:|
| Correct raw state | 5/12 | 6/12 |
| Primary semantic pass | 5/12 | 4/12 |
| Host accepted | 12/12 | 7/12 |
| Host accepted and primary semantic pass | 5/12 | 2/12 |
| Correct required unknown | 0/4 | 0/4 |
| Semantic-arm input/output tokens | 63,644 /3,492 | 65,062 /4,339 |

Primary semantic pass requires the correct raw state, sufficient original premises
in structured `because`, a faithful complete explanation and a relevant question
for genuine uncertainty. The masked audit did not inspect host acceptance. Two
checked replies named needed input only in prose; a separately reported
prose-inclusive sensitivity raises checked semantic passes to 6/12, leaving direct
at 5/12. It does not repair the structured citation contract. Reused-case primary
scores were direct 4/8 versus checked 3/8; fresh cases were 1/4 for both.

Every checked comparison said `supports`. Four checked states said `not_met`,
so the host refused their contradiction; two of those raw judgments were otherwise
correct and fully supported. A fifth refusal mislabeled complete observed content
as unavailable. Neither arm used unknown when needed. The result does not qualify
semantic reliability or show a usable benefit. It is a bundled protocol change,
not an isolated field-order experiment or a representative benchmark.

Independent verification compiled all 30 actual schemas using engine
`b1915-27637851`, confirmed checked state-last order, and verified 260 saved artifact
references. Typed facts/criteria matched within each semantic pair, and source,
database, binary, fixture and model-setting checks remained intact. The six
controls cost an additional 31,247 input /792 output tokens. Artifacts are retained
under `results/memory-evaluator-repair-20261108/trial-20261108T222045.752765Z`.
Checks-first stays opt-in. The next bounded concept pilot can use independently
scored synthetic outcomes; automatic native concept admission remains unqualified.

The preceding separate Qwen3.6-35b-a3b-uncensored comparison completed all 16
raw/typed responses without retries: each arm had 3/8 correct states, 2/8 fully
supported responses and 0/2 required abstentions. All replies said met. One reason
correctly identified a mismatch while its verdict said met; other replies used
requested writes as observed contents. This motivated a bounded protocol repair,
not a claim that the typed evidence experiment had qualified semantic evaluation.
The partial Qwen3.8 pilots below remain separate and are not a model ranking.

### Complete native evidence and bounded concept rumination (2026-11-08, prototype branch)

`Memory::prepare_native_evidence` creates an opaque request from reauthenticated
prepared interpretation sources. Native envelopes retain complete values and
source revision/field, original source kind, role, influence and capture metadata.
Ordinary JSON-looking prose stays text. Whole values that exceed the bounded view
become explicit non-citable omissions while remaining authorization dependencies.
`finish_native_appraisal` validates a request-bound selection of fact IDs and
rechecks all sources; it returns a provisional read-only judgment, never a v4
support report or stored interpretation.

`memory_native_workflow::run_case` compares raw envelope strings with typed values
using the same sources, IDs, criterion, response schema and one-call budget per
arm. It exports actual requests/replies, failures, usage and source/database
hashes. Independent review caught arm-local cancellation suppressing its sibling;
`paired` now creates independent child tokens and only root cancellation stops
both. The regression failed before the repair and passed afterward.

`concept::rumination` adds `prepare_concept_rumination`,
`finish_concept_rumination` and one-callback `ruminate_concept`. The opaque receipt
binds a current concept revision, its complete consulted source union and exact
mandatory prior/new counterexamples. Revision preserves those obligations and
inherits explicit restrictions on the previous concept as well as source scopes.
Self-review caught scope inheritance and nested duplicate-key gaps; both were
reproduced before their fixes. Every outcome rechecks current authorization,
source revisions and scopes. Unchanged and abstained outcomes write nothing and
do not retain newly consulted evidence. Native external-source concept admission
remains refused; no scheduler, rule installation or schema migration was added.

Focused qualification passed eight native tests, seven workflow tests and 22
concept-filter tests (ten new rumination tests). Independent component and broad
reviews found no unresolved P0–P2 issue at `252422099`. Integrated Spark checks
passed workspace/all-target build, strict all-target/all-feature Clippy, 6,352
default tests (25 ignored), 6,901 memory-enabled tests (29 ignored), 303 standalone
tests and 182 Python tests with both actual native binaries and zero skips.
Workspace and standalone formatting passed. Backend-required runs used the
source-pinned interpreter worker and a HUD renderer built from this checkout.
Initial missing-backend failures and the concurrent cold-worker installation
failure are preserved. The standalone run exposed a missing test-only Tokio
dependency; adding it to the independent harness fixed compilation without
changing production dependencies or the frozen model executable. No existing
locked dependency version changed. The 380 frozen implementation files remained
unchanged. Mechanical tests do not establish useful abstraction or generalization.

The fresh raw/typed pilot was stopped **incomplete** after seven of sixteen
planned model replies. Implementation, executable and eight independently authored
cases were frozen before inference (source `252422099`, resident Qwen3.8-27b,
seed 70510, temperature 0.6, thinking requested off, one 6,144-output-token call
and 540-second total deadline per arm, alternating order). Three pairs completed;
raw labels were correct in 1/3 and typed in 2/3, but neither arm had a fully
supported response in those three matched pairs. A fourth, unpaired typed reply
was correct and fully supported. Its raw sibling exhausted its setup deadline
without dispatching inference. A pending owner model switch, waiting behind a
separate live Mecha hold, blocked new holds. The fifth case was cancelled before
any call, and cases six through eight never started. Both owner processes and the
pending switch were left untouched. No retry, prompt edit or model replacement
was used to fill missing responses. Unknown-case behavior remains unmeasured.

An independent reviewer established all eight expected states before receiving
blinded A/B replies, then audited the seven replies and the setup-failure slot.
Reference validity was not semantic sufficiency: wrong current-criterion judgments,
missing input premises, historical-check substitution and unfinished explanations
remained. The current criterion and broad goal wording conflict in changed-goal
fixtures; that ambiguity and the synthetic observation ordering are recorded,
not silently repaired. The expected states agreed with the frozen annotations;
the independent audit correctly accepted the current criterion itself as the
target-value premise where sufficient. The partial cohort cannot establish a
format advantage or generalization benefit.

All seven returned responses passed structural finalization. All nine prepared
schemas compiled with the resident engine's pinned `27637851` converter. Independent
hash verification covered 82 saved artifacts across five prepared cases; original
sources and SQLite files stayed unchanged. All 380 frozen implementation hashes,
the executable and frozen cases/annotations passed the post-stop check. Reported
usage was raw 16,870 input / 671 output tokens and typed 21,865 / 687; denominators
are three and four calls respectively. Wall time includes shared-server delays
and the raw setup timeout, so no efficiency comparison is claimed. Exact records,
partial-cohort accounting, audits and check logs are under
`results/memory-native-concepts-20261108/`. The next learning experiment uses
independently graded synthetic outcomes rather than promoting these appraisals.

### Compact appraisal reader and persistence diagnostics (2026-11-08, prototype branch)

`interpretation_reader::serialize_records` presents atomic-v4 appraisals as
`interpretation_reader_v1`, retaining the complete SupportReport, source locators,
criterion and context identity once. Canonical SQLite payloads and receipt hashes
remain unchanged. Native bridge recapture parses this through a separate strict
type; it cannot become a canonical Record or source authority. The 20,000-byte and
32-record contextual limits still withhold oversized complete closures. The
host-only `Memory::stored_interpretation` checks the active receipt, complete
source closure and an independent reopened authorization phase without inheriting
those presentation limits.

`memory_atomic_workflow::pipeline` now marks retain completed immediately after
the successful API call and records stored_export and recall separately. Failed
export or contextual exclusion cannot retroactively erase successful storage.
The independent auditor preserves verified retention on a later recall failure
and reconstructs the compact presentation from the full canonical export.
Legacy stage sets and presentations remain accepted; all eight archived frozen
summary results match the original auditor exactly. Three independent source
reviews found no actionable issues in the component and combined patches.

Integrated Spark qualification passed workspace/all-target build, strict
all-target/all-feature Clippy, 6,352 default tests (25 ignored), 6,876
memory-enabled tests (29 ignored), 286 standalone tests, and 182 Python tests
with zero skips. Both actual native binaries handled authored HTTP fixtures.
Formatting passed locally. No real-model calls were made by this increment.

The real read-only development replay reopened the same eight saved atomic-v4
stores through their original LocalAuthority. Old reader: 3/8 complete bundles;
compact reader: 8/8, each with exactly nine expected references and SelectedCurrent.
Context packets fell from 16,357–22,502 legacy bytes to 14,002–17,344; actual
GoalMemory packets were 14,039–17,381 bytes under the unchanged 20,000-byte cap.
All internal canonical records matched stored payloads byte-for-byte. Source-only,
changed-goal, historical-context and pre-retention controls passed. Positive
HistoricalDependency behavior requires the separate authored regression, because
these eight stores contain only one interpretation each. Before/after hashes
confirmed unchanged saved run trees, replay sources and copied binaries. Logs,
full packets and inventories are in `results/memory-compact-recall-20261108/`.

This is a reader regression repair on already-inspected synthetic cases, not a
new model comparison or restored semantic reliability. The original 5/8 labels,
3 factual/17 citation defects, inadequate premises and missing abstentions remain
unchanged. Neither saved bad judgments nor their historical failed workflow
records were rewritten. Whole typed native values and evidence-ID selection are
the next design in `MECHA-MEMORY-DESIGN.md` §31.14; they are not implemented here.

### Handoff source correction (2026-11-08)

The old code-mode "nothing built" handoff statement is obsolete. Current source
contains `Tool::effect`/`EffectClass`, `Agent::gate`, `CodeTool::call`/`over`,
`setup::prepare_with_approver` registration, `Feature::Code`, `Lever::CodeTool`
and `interpreter::for_runs`. This is a source inventory correction, not work
implemented by the memory increment or a claim about installed capability,
M1 results, analyst-profile readiness or production behavior. The old release
v0.1.21 record is now explicitly historical rather than labelled latest.

### Atomic observations and derived appraisal (2026-11-08, prototype branch)

The explicit default-off v4 path separates `generate_atomic`, `check_atomic` and
`appraise_atomic`. The host assigns observation IDs and resolves each original
quotation; supported checks cannot substitute another source. `prepare_appraisal`
reauthorizes all dependencies before exposing an opaque request. `finish_appraisal`
preserves the original proposal/checks and adds a separately identified derived
judgment. Retention replays this construction and checks authority again. SQLite
v8 and legacy wire/report semantics remain unchanged. This is a synthetic-only
comparison path, not automatic production capture or a truth guarantee.

`memory_atomic_workflow::run_case` gives the three-call policy and a direct answer
the same initial sources, criterion, resident model, seed, 6,144 total output-token
allowance and 540-second whole-arm deadline. Separate thinking is explicitly off.
The new timeout constructor preserves the legacy provider's 180-second default
while allowing the direct arm its actual declared deadline. The independent
`atomic_trial.py` auditor reconstructs source/catalog bindings, raw wire controls,
SQLite retention and recall. External review separately grades original claims,
citations, selected-premise sufficiency and temporal applicability. Rejected
extraction defects remain visible without being counted as retained defects;
malformed responses retain their known usage costs. Label blinding is limited:
reviewers lack arm/model/gold labels, but raw stage structure remains visible.

Source review exposed complete-object `oneOf` alternatives required by the
llama.cpp grammar compiler and the provider's hidden 180-second ceiling. Both
received failing-before/passing-after regressions and independent rereview.
Final aggregate review found no major/medium issue. Minor follow-ups are stricter
stored-report shape validation against coordinated direct database tampering and
binding the auditor's reported case ordinal to the frozen manifest. The actual
comparison driver supplies the frozen ordinal; neither is claimed fixed here.

Integrated Spark verification passed workspace/all-target build, strict all-feature
Clippy, 6,352 default tests (25 ignored), 6,864 memory-enabled tests (29 ignored),
279 standalone checks, and 181 nested Python test invocations with zero skips,
including both native binaries against authored HTTP replies. Formatting and diff
checks passed. A single unscored development case completed both arms and passed
independent structural auditing; it was not a fresh semantic qualification.

The source snapshot contains the reviewed uncommitted increment over `c8567100a`,
not the base commit alone. Its SHA256 is
`527caa417ef7818a4ee840c72cb6aa8315f9b310a4dbafaa9999c05970b59e8f`;
the copied atomic binary is
`56d274926a742d79367b4a40bcc420437453c1b8d36a67eff6d5d455235174d1`.
Implementation/auditor freeze preceded independent authoring of eight new cases.
Raw checks and comparison artifacts are in
`results/memory-atomic-appraisal-20261108/`. The frozen comparison completed all
32 model calls with the already-resident
qwen3.6-35b-a3b-uncensored (different from the previous v3 cohort). Both arms
matched the independently authored label in 5/8 cases. Pipeline workflow completion
was 3/8 versus 8/8 direct answers; direct answers do not promise checked retention.
All five pipeline failures occurred after storage at contextual rereading. Both
arms wrongly resolved unknown causal responsibility and interval continuity into
known outcomes. The pipeline additionally misread the current empty-file sentinel;
the direct arm additionally misread the changed units/schema criterion.

Three independent reviewers assessed all 32 raw replies and all 119 original
observations (58 pipeline, 61 direct), with arm/model/gold labels withheld and
raw stage structure visible. All 16 case-arm semantic annotations passed exact
artifact/hash and count-reconciliation checks. Reviewers agreed with the frozen
5/8 label scores. Pipeline accepted observations contained 3 factual and 17
original-citation defects; direct emitted observations contained 4 and 22.
No generated pipeline defect was rejected by its fact checker. Diagnostic/reason
output added 14 factual and 17 citation defects for pipeline, versus 7 factual
and zero additional source-citation defects for direct. These are assertion/slot
counts, not comparable per-answer error rates: the pipeline emits more blocks.
Final selected premises were independently judged sufficient/current in 0/8
pipeline cases versus 3/8 direct cases, even where labels were correct.
Both arms failed to abstain on both genuinely unknown cases. Neither pilot gate
passed. Among the three fully exported pipeline records, the formal auditor
observed 2 retained factual and 6 retained-citation defects; full-eight retained
totals remain unknown because five workflows lacked required exports. Prior failed
cohorts remain immutable and concepts remain gated.

All observed usage is complete: pipeline 226,646 input /13,387 output tokens over
242.615 seconds summed arm time; direct 64,000 /7,805 over 105.402 seconds.
The ratios are 3.54x input, 1.72x output and 2.30x time, with no label-accuracy gain.
No retry, prompt change or model switch occurred. Frozen-file hashes and the
actual eight manifest ordinals/alternating orders were verified after inference.
These small same-evidence cases measure neither retrieval benefit nor general
learning, and equal output allowances do not equal equal compute.

All five failed rereads had reconstructed complete packets above the budget
(20,357–22,502 bytes); successful cases fit (16,357–18,347 bytes).
The first fresh case exposed this concrete retrieval-size boundary: persistence
succeeded, but the complete nine-record context required 21,130 serialized bytes
against the reader's 20,000-byte cap. Its Hit entries alone occupied 20,381 bytes.
Canonical reopening passed and source hashes still matched. The generic withheld
error therefore does not mean no row was saved. Because the workflow stopped
before retained/database/control exports, the strict auditor leaves retention
unverified and retained-defect totals unknown. A separate read-only inspection of
all eight SQLite stores found three wrong known bearings saved, two in workflows
with failed later recall. This supplemental observation does not turn incomplete
primary audits into complete workflows; retrieval exclusion is not non-retention.
Compacting evidence must preserve the complete authorized dependency closure;
dropping provenance to fit is not a fix.

### Evidence-context and candidate judgment repair (2026-11-08, prototype branch)

The explicit evidence-aware adapter added authenticated capture associations,
source roles and known timestamps without changing canonical SQLite v8 source
bytes. `evidence_metadata` uses only projections in the authorized closure;
`recheck_support_prepared` and final retention reconstruct it. Source influence
remains distinct from role. Selected capture denotes the appraisal's selected
experience, not globally newest evidence.

`memory_support_evidence` separately evaluates every original citation slot.
Supported units require all originals to provide direct support, and canonical
support uses those original quotations. An evaluator's alternative relevant
passage can no longer hide the proposal's irrelevant citation. The new
`finish_support_revised` report records each judgment's own premise flag and
reconstructs candidate selection: first eligible known bearing, Unknown on
conflicting eligible bearings, otherwise conservative Unknown. Original premises
and dispositions survive. Legacy reports reopen with their old semantics;
separate historical interpretation records are not withdrawn.

V3 workflows export full reopened interpretations for both generated and probe
paths. Review caught two audit gaps: relying on request labels rather than stored
source payloads, and inspecting dispositions without the actual retained prose
and locators. Both were reproduced and corrected. Final integration review then
caught Python expecting a generic Supported schema after Rust introduced exact
citation-count groups. Correcting that contract passed an actual native workflow
with three authored HTTP responses, full SQLite authority and saved-render
verification. An initial fixture-label refusal occurred before any call and was
preserved. The final nested Python suite passed 144 invocations with zero skips;
CI now enables the actual native check. Independently constructed test dictionaries
can agree with one another while disagreeing with the producer.

Workspace/all-target build, formatting, strict all-feature Clippy, 6,352 default
tests (25 ignored) and 6,844 memory-enabled tests (29 ignored) passed on Spark.
Nine focused v3 regressions and 273 standalone checks passed. These verify
mechanics; semantic qualification is separate.

The subsequent frozen cohort at `8383f6d68` used the already-resident
qwen3.6-35b-a3b and eight independently authored synthetic cases. Source, helper,
fixture, audit rubric and copied binary were frozen before inference. All eight
attempts and 23 replies remain preserved; no retries, prompt edits or model swaps
occurred. Three workflows completed. Three Unknown diagnostics omitted required
questions, two evaluations omitted citation assessments, and one generator used
invalid judgment premise identifiers; these failures overlap across cases.
Five generated and six probe outcomes were independently recoverable, without
turning partial outcomes into complete workflows. Four of eight generated current
criterion states and four actual retained generated bearings were correct;
five actual retained probe bearings were correct. One wrong probe bearing was
Supported but withheld for unsupported premises. No wrong known generated bearing
was Supported or retained among observed outcomes; missing outcomes stay unknown.

Independent Codex reviewers inspected all available raw outputs across eight
cases and all 20 admitted generated units from five validated generated outcomes;
none of those 20 had an unsupported factual premise under the frozen rubric.
They identified 33 original-citation assessment defects across the available raw
outputs, including missing assessments in refused replies. Ten generator slots
from the failed generator had no evaluator assessment and were recorded separately
as unevaluated. The strict scorer qualifies only three complete manual/metadata/
citation audits and counts 13 citation defects in those complete cases. All-eight
citation/unsupported-fact metrics remain unknown, not zero. A true claim can carry
an inadequate citation: split invocation/receipt evidence, criterion fragments
and digest metadata were repeatedly overclaimed as direct support. One complete
case retained Unknown rather than the correct Bad in both generated and probe
paths. Two causal probes confused missing evidence with contradiction. The gate
therefore fails for semantics as well as completion.

Observed cost was 196,862 input and 25,288 output tokens over 342.314 seconds of
summed case wall time. The conservative complete-cost field remains false because
some stages failed. Post-run source/binary/helper/fixture checks passed. Resident
model identity/status/settings were unchanged; only discovery response `created`
timestamps changed. The previous qwen3.8 cohort uses different cases and model,
so these counts are not a controlled improvement or regression comparison.
Results and qualitative audits live in
`results/memory-evidence-revision-20261108/fresh-resident-qwen36/`.
Concept learning remains gated. Next work should simplify generated claims and
make current criterion, factual premises and individual citation obligations
explicit, then use another independently frozen cohort. Do not repair or replay
this cohort into a claimed fresh pass.

### Citation/schema and evaluator repair (2026-11-08, prototype branch)

The explicit support wire-v2 reuses v8 canonical storage and admission. A
request-bound catalog resolves source choices into exact citations and supplies
the confirmed goal. Source roles appear in handles. A separate criterion state
and per-judgment premise-sufficiency assessment produce coherent offered-bearing
checks while preserving the canonical all-premise gate. The existing resident
provider now has an explicit structured-output constructor; the old constructor
and legacy workflow remain unchanged. Core implementations are
`memory_support_wire`, `memory_support_reasoning`,
`memory_factual_support::{generate_grounded,evaluate_grounded}` and
`memory_support_workflow::run_case_with_protocol`.

The first reused-case development round retained all 24 calls and completed five
native workflows out of eight. It observed 158,222 input and 32,460 output tokens.
Two failures omitted required criterion questions; one exceeded a UTF-8 byte cap
although the character-limited schema was satisfied. Its original auditor also
incorrectly expected no-autoload in the JSON body; the actual provider sends the
URL query, verified by HTTP tests. Preserve the original failed ledger and score.
A separate corrected diagnostic audit observed six valid probes: six supported
true statements, six appropriate uncertain responses, five correct supported
bearings and one false statement Supported/admitted, all against eight planned
cases. Unobserved outcomes remain unknown, not successes.

Independent Codex review examined all 37 generated admissions across eight cases.
It found five clear unsupported current claims, two additional disputed causal
hypotheses, and one incorrect generated Good bearing retained. Generated criterion
states were correct in three of eight cases. Probe correctness cannot stand in
for generated interpretation quality. Accordingly the v2 scorer now counts wrong
generated Supported/retained bearings separately, including dropped duplicates,
and retains partial generated outcomes even when another stage fails. This is a
small reused-case diagnostic, not fresh qualification or production readiness.

The second and final reused-case round also completed 5/8 native workflows,
observing seven generated and six probe outcomes. Against eight planned cases,
true support was 6/8, appropriate uncertainty and correct supported bearings 4/8,
and criterion correctness 4/8. Observed false factual probe support/admission was
zero, but one wrong probe bearing was Supported and one wrong generated bearing
was retained. Unobserved metrics remain unknown. Usage was 156,064 input and
20,271 output tokens. Two failures selected None for factual-unit premises;
another returned Unknown without a missing-information question. Existing host
checks refused all three. The final structural correction aligns the producer
schema with these existing restrictions before fresh qualification, without a
third development replay or further semantic prompt changes. Independent review
examined all eight admission exports (22 retained units across seven nonempty
exports) and flagged two unsupported generated claims: literal input/output byte
equality and an unobserved reasoning-process claim. The missing-artifact case
retained Good based on an earlier write. Citation/reasoning defects are recorded
separately from fact truth. The original run remains unchanged; separate reports
are in `development-2-independent-audit/` beneath the repair results directory.

The bounded second refinement adds role-prefixed handles, conservative producer
character caps, conditional criterion diagnostics and generic current-criterion
and factual-antecedent clarification. Canonical byte limits and strict parsing
are not weakened. The first repair snapshot passed workspace/all-target build,
formatting, strict Clippy, 6,351 default and 6,816 memory-enabled tests with zero
failures (25/29 ignored). Four further regressions failed before that refinement
and the 29-test adapter/workflow suite then passed; the nested Python directory
passed 75 tests. Later model/CI results remain in HANDOFF and PR #846 until
qualified. Raw evidence stays local in `results/memory-support-repair-20261107/`.

The final unit schema correction passed the 30-test adapter/workflow suite,
78 nested Python tests, formatting and strict all-feature Clippy after one
test-only iterator lint correction. Independent review found no major/medium
issues. A fresh frozen attempt made no model calls: Spark's resident model had
changed and identity guards refused all eight cases. That infrastructure failure
is preserved separately from the new qwen3.8-27b resident-model cohort; no model
was loaded or swapped. The new cohort's findings cannot establish an equal-model
improvement over the qwen3.6 development runs. The fresh cohort already
exposes a wrong Supported probe bearing for an independently Missing artifact,
plus a generated judgment with no required evidence. It cannot qualify the
support gate. The final cohort completed 6/8 workflows, with all eight probes observed: 8/8
true support and appropriate uncertainty, 6/8 correct-bearing support and current
criterion accuracy, zero false factual support/admission, and one wrong probe
bearing Supported and retained. Six generated outcomes were observed and two
remain unknown. Independent inspection covered all eight exports and 23 retained
units: no unsupported factual premises found, but one mismatched citation. The
formal scorer still has six qualified manual audits, not eight. Observed usage
was 152,341 input / 19,797 output tokens; timeout usage remains unknown.

After that frozen assessment, a separate producer-only correction requires one
or two citations for every goal judgment. Canonical Unknown/Contradicted replies
may still omit citations, and no prompt, parser or storage semantics changed.
Its regression failed before the one-line schema change; final memory-enabled
verification passed 6,824 tests with zero failures (29 ignored), plus strict
all-feature Clippy and formatting. The final review-fix default suite passed
6,352 tests (25 ignored); the nested Python auditor directory passed 80 tests.
The cohort was not replayed, so this follow-up has no fresh model-quality claim.
Independent audit also records first-per-goal duplicate retention, unnecessary
rejections and source-role defects separately; evaluator reliability remains open.

PR review also found that queue absence conflated deliberately disabled step
checks with an enabled check lacking shell. `ToolCtx::step_checks_unavailable`
is now stamped on every run; only enabled-but-unavailable checks emit the notice.
The four-way regression failed before the fix and passed afterward, as did the
existing missing-shell and frozen-check/tamper tests. The CI port collision was
addressed in the lifecycle test allocator using process-wide selected-port claims:
a deterministic same-seed regression failed with two distinct ports before the
fix and passed with four afterward; all seven lifecycle tests passed. This closes
the in-process selection/startup race, not every possible external port conflict.



### Action/outcome evidence and factual support (2026-11-07, prototype branch)

Explicit new-file v8 capture separates native action arguments, call receipts and
independently observed artifact contents. `LocalAuthority::capture_with_evidence`
uses the same descriptor read for the artifact check and bounded reader envelope;
omission states preserve missing/refused/non-UTF8/oversized distinctions. Old
formats are neither migrated nor silently certified as evaluated.

`memory_factual_support::{generate,evaluate}` proposes typed statements and checks
every statement and offered goal bearing in separate bounded local calls.
`Memory::retain_evaluated_interpretation` rebuilds deterministic reader prose from
the admitted report and rechecks current sources, scopes and complete dependencies.
Rejected prose and evaluator diagnostics stay in host audit. Good/Bad requires its
own supported assessment and all original factual premises; speculation remains
labeled and cannot supply those premises. The assessments are fallible, correlated
model judgments, not independent truth or action authority.

The authored native workflow connects ordinary Agent file actions, actual
observations, generation/evaluation, durable reopening and GoalMemory recall.
The recapture regression exposed an 11,579-byte returned memory packet exceeding
the ordinary 4 KiB original-passage limit. Rich GoalMemory results now retain
strict full-packet validation, native receipts and all consulted dependencies,
without copying that whole packet into another observation. Exact historical
presentation remains in JSONL. Native changed-goal recapture preserves influenced
roles and is withheld after ancestor withdrawal and reopen.

Qualification review separated evaluator classification from final admission:
dropping duplicate goal judgments could otherwise mask wrong Supported answers.
The coordinator compares exported outcomes with raw replies and read-back durable
reports, preserves observed errors across later failures, and checks pre-inference
source seals. These safeguards establish measurement consistency; a fresh local
model trial is still required to establish semantic performance. No production
capture, owner-graph migration or nightly learning pipeline was enabled.

Final Spark qualification passed workspace/all-target builds, strict all-feature
Clippy, 6,351 default tests (25 ignored) and 6,793 memory-enabled tests (29 ignored),
62 suites each. Standalone memory passed 258 tests; the support coordinator passed
40, alongside 101 existing Python tests (4 skipped). The first full run exposed
six AppleDouble transfer files misread as Rust source; removing only those verified
untracked metadata files restored the source-inspection tests. A large enum lint
was resolved by boxing optional support metadata, preserving serialized bytes.

The frozen eight-case resident-model qualification **failed**: 0/8 complete
workflows, 2/8 valid generated proposals, 1/2 valid generated-proposal evaluations
and 0/8 valid probe evaluations. It made 18 calls, observing 53,744 input and 20,544
output tokens. Missing fields, invalid source IDs and inexact quotations prevented
qualification. Independent raw-response review also found invented approval,
incorrect current-criterion application and contradictory labels; format repair
alone would not establish correctness. One case retained four independently
supported units, with its goal bearing downgraded to Unknown. Seven empty admitted
sets are not successful semantic evaluation. All eight artifacts were independently
audited, but none qualifies as a complete pipeline; official probe metrics remain
unknown. No response repair, retry, downstream comparison or concept/rumination
integration followed. Evidence: `results/memory-support-20261107/`.

### Situated interpretation workflow (2026-11-07, prototype branch)

Source `02cfe11cc` adds explicit v7 interpretation records beside immutable
native episode evidence. `prepare_interpretation` authenticates capture receipts;
`memory_situated_appraisal::generate` reuses shared parsing, provider envelopes
and qualified grounding; `retain_interpretation` rechecks sources and saves exact
revision/field claim locators. No transaction spans inference. Legacy graph
frames and historical appraisal identity remain unchanged. Explicit
`GoalMemory::with_interpretations` returns host-assigned SelectedCurrent and
HistoricalDependency roles; default readers withhold interpreted descendants.
Recapture preserves complete dependencies and withdrawal remains effective.
V7 projection descriptors avoid duplicating original text, without changing v6
serialization or increasing the 20,000-byte contextual budget.

`NeedsInput::assess` preserves reading states, observed zero, withdrawal and
freshness as nonserializable host assessment. Generation receives no private
sensor values or rationales. The authored ordinary-Agent example proves identical
provider requests under needs-only changes, changed-goal reappraisal after reopen,
independent artifact checks, untrusted/private taint and source revocation.
Actual Spark packets were 7,305 and 19,328 bytes. This does not implement automatic
needs-driven goal selection, production autocapture, concept learning or rumination.

Scoped and final reviews corrected missing durable citation locators, timestamp
normalization, served-model identity and cancellation propagation. The last defect
connected an outer-cancelled provider to an unrelated native RunContext token:
the recorder waited for the loop to drain partial output, while that loop waited
for its separate 180-second timer. A parent-linked child token fixes it without
cancelling later tasks on an individual deadline. The blocked-provider regression
failed on old behavior, then retained native partial text, audit and cancellation
outcome promptly. A later combined-main run and GitHub CI exposed a race in that
test: its signal meant queued inside Recorder, not forwarded to the native loop.
The regression now waits for the synchronous audit-and-forward boundary before
cancelling, preserving its deadline and transcript assertions. Cancellation can
preserve delivered output; it cannot promise upstream buffered bytes survive a
dropped provider future. All review findings at major/medium severity are resolved.

Spark workspace/all-target build, final formatting and strict all-target/all-feature
Clippy passed. Default tests: 6,330 passed, zero failed, 25 ignored; memory-enabled:
6,735 passed, zero failed, 29 ignored; each comprised 62 suites. Standalone memory:
241 unit plus four integration tests. Python CI passed 101 tests with zero skips;
the supplemental coordinator suite passed 16. Its first invocation used the wrong
discovery directory and ran zero tests; that failed invocation is retained beside
the corrected run. Both examples build and the actual authored workflow passes.
Current GitHub checks are recorded on draft PR #846, separately from these runs.

The frozen `situated-interpretation-pair-v1` comparison retained four generated
interpretations and eight planning tasks (46 provider calls) on the resident local
model. Two goals and two seeds share identical original facts, prompts, tools and
budgets between paired arms; only treatment stores and retrieves an interpretation.
All four treatment runs received it; no source-only run did. Exact artifact scores
were source-only 3/4 versus treatment 4/4. Normal completion was 1/4 versus 4/4.
The failed source-only calibration-preservation run exhausted six turns without
saving output; both speed-goal controls saved correct output but exhausted their
budget. No incorrect route was saved. All eight planning captures succeeded and
all original source hashes and request controls passed the audit.

Generation admitted 17 of 18 offered claims; one quote not present in its referent
was dropped. Two drafts retained a goal judgment and two retained none, including
one unresolved goal identifier. Independent semantic review found historical explanations beyond the recorded
evidence in all four prose drafts, including hedged speculation. In fast-r0, a retained claim inferred that Ilex had been
selected from a successful two-byte file-write receipt, although the recorded
artifact was the empty array `[]`. Quote membership did not establish entailment.
The goal-appropriate recommendation can be correct while its historical explanation
is invented; these measurements do not qualify reliable situated meaning. Keep
this path default-off and address semantic faithfulness before broader use.
Source planning used 24 calls, 72,729 input and 1,261 output tokens. Treatment
planning used 18 calls, 73,597 input and 1,050 output tokens; generation added four
calls, 6,807 input and 3,061 output tokens. Generation-inclusive treatment therefore
cost 80,404 input and 4,111 output tokens (3.26 times control output), despite fewer
planning calls. The model-visible packet contained the check verdict/digest and write receipt,
not the actual empty artifact contents. All four prospective route recommendations
fit the current goal, but one prediction risked reverting to the historical goal.
This tiny reused development fixture supports a bounded completion observation,
not generalization, an efficiency win or autonomous learning. All raw
outputs, frozen protocol, independent audit and qualification logs are retained in
`results/memory-situated-20261107/`; binary SHA256
`51defee5b74e4831e4c00948e094aac143504471bf0aeaf00babaa3a7610dc06`.
Production services, owner data, graph writers and nightly jobs remain unchanged.


### Budget finalization experiment and CI recovery (2026-11-07, prototype branch)

`RunContext::with_budget_notice` adds default-off advice before the request with
two turns remaining, or one for a one-turn run. It uses the effective hard cap,
runs after existing stop checks, preserves tools/guards, and cannot add a call or
write an artifact. Exact harness-voice registration keeps it out of owner steering;
`memory_experience_bridge::events` also excludes recognized harness text from new
owner-request passages while retaining raw transcripts and source coordinates.
Historical captures are unchanged. The native example exposes a strict explicit
boolean; false preserves old serialized `TrialConfig` shape. Explicit child runs
remain off. Source commit `a2b9f61` contains this experiment, not a production rollout.

Five new core assertions and the example argument test failed meaningfully on
old behavior; the initial missing-constructor compile failure is retained but
not counted as behavioral proof. Nine notice regressions, three example tests,
ten native trial tests and twenty-one bridge tests pass locally. Spark authored
preflights pass for both the unchanged old comparison and the new protocol, with
zero inference. A deliberately last-turn write in the latter still ends at the
cap: its preflight acceptance is separate from natural completion. The reviewed source also passed Spark build, formatting, strict all-target/all-feature
Clippy and the default workspace suite: 6,326 passed, zero failed, 25 ignored.
Memory-enabled and final GitHub qualification are recorded on PR #846. Independent
source review found no major/medium defect; a notice-bearing context-overflow
retry regression remains a nonblocking coverage suggestion.

The separately frozen `connected-budget-notice-comparison-v1` completed all
16 later tasks plus two source setups on the resident Qwen model, retaining all
99 model calls without replacement or retry. Four synthetic evidence conditions,
two seeds (70400/70401), balanced arm order, eight-turn cap, fast mode and existing
model controls were fixed. The unavailable task reuses the exact earlier failing
prompt, inputs and criterion; only its host oracle scores appropriate abstention.
This is development evidence, not a held-out benchmark or a memory-benefit study.

Both arms produced 7/8 correct artifacts, including 3/4 required abstentions;
all four solvable tasks in each arm produced correct artifacts, with no premature
abstention. Both unavailable-r1 runs failed to save output. The notice was visible
in three treatment runs, first on request seven. One recoverable-clue treatment
saved correct output but hit MaxTurns; normal completion was control 7/8 versus
notice 6/8. After the warning, the failed unavailable treatment used todo and then
tried to read a directory. Advice did not resolve that search-to-deliverable gap.
The default remains off. Protocol integrity passed; all-runs-completed did not.

Capture succeeded 17/18 including setups. The unavailable control's failure was
NOT the 16-passage cap: an empty memory query returned `invalid recall text`, and
`events` refuses an errored memory result because it cannot establish consulted
sources. Preserve that fail-closed guard while investigating a typed receipt for
validation failures known to occur before retrieval; do not simply accept all
failed memory reads. This gap is distinct from longer-episode packing. The notice
arm's failed task was captured, which does not establish a causal capture gain.

Control/notice later calls were 47/46, input tokens 93,907/90,552 and output 3,897/3,730.
Including setup: 194,162 input and 8,148 output tokens. Shared-server timings and
concurrent qualification do not establish a latency benefit. Original source
snapshots were unchanged and before/after router properties matched. The independent
trace audit verified all attempts, controls, exposure, artifacts and withdrawal
behavior. Records remain in `results/memory-budget-notice-20261107/` on the operator's
machines; older results and frozen scores are untouched.

GitHub's PR #846 metadata initially named an outdated base and claimed a conflict
against its own ancestor. One close/reopen exposed eight newer main commits and
an actual changelog conflict. Integration `50521da77` preserves both entries and
restored automatic CI/review/documentation runs; build, strict Clippy and 6,320
serial default workspace tests passed (zero failures, 25 ignored), with required
owner-corpus privacy checks. No workflow was weakened, no branch history rewritten,
and no PR merge or production deployment occurred.

### Prototype CLI request-level no-autoload (2026-11-07, review follow-up)

Automated review found that the older `LocalMemoryModel` checked residency but
left automatic loading enabled on its subsequent POST. It now opts into
`OpenAiCompatible::without_autoload`, matching `LocalTrialProvider`. The hold
and precheck remain useful, but the request itself enforces refusal after an
eviction. Ordinary provider defaults are unchanged. Actual-loopback regressions
failed against the old URL and passed for both a successful authored reply and
a one-request 503 refusal without retry/load. The four authored Python fixture
routes now parse the URL path, preserving the request body and frozen scoring.
The constructor audit found no other missing prototype opt-in; independent
scoped review found the medium addressed. Spark build, formatting and strict
Clippy passed, alongside both transport regressions, 92 CLI-backed Python tests
with zero skips, and the 24/48-request authored constraint/benefit workflows.
These focused checks cover the follow-up; the full workspace counts below
belong to the preceding planning-repair source. No real-model comparison was repeated:
its provider was already protected. Qualification logs are retained separately
under `results/memory-planning-checks-20261107/cli-noautoload-qualification/`.

### Planning-check registration repair (2026-11-07, prototype branch)

`Agent::run_in` now creates the command-check queue only when checks are enabled
and shell is registered. `TodoTool` gives one unverified notice for newly
completed checked steps without that queue. Existing skipped verification,
command freezing and tamper detection remain intact; a registered shell still
uses ordinary dispatch and all its guards. This ordinary planning repair is not
feature-gated, although the memory comparison is. No production install changed.

Both new Rust regressions failed against the earlier behavior. Targeted checks
passed 73 todo tests, 12 check tests and 16 Python coordinator tests. The old
native executable passed the original 24 slots/seven probes but failed the new
eighth unavailable-check probe. The repaired executable passed all 24/eight
without real inference. Case, input, oracle, pair, budget and control hashes
remain pinned to the first comparison. Independent incremental review found no
major or medium issues.

Spark workspace build, formatting and strict all-target/all-feature Clippy passed.
Clean serial suites passed 6305 default and 6669 memory-enabled tests, with zero
failures and 25/29 ignored. The first default run failed two source-walk tests
because the file transfer created binary macOS metadata sidecars ending in `.rs`.
Removing those owned transfer artifacts restored source identity; both focused
rechecks and the fresh full run passed. The initial failure log remains retained.

The exact Spark executable passed native preflight (24 slots, eight probes,
106 scripted requests, no inference). A fresh, separately frozen development
repeat retained identical v1 cases, inputs, gold, ordering, seeds and budgets.
All 24 attempts ran, with 107 completed model calls. Later exact artifacts passed
7/8 memory versus 5/8 empty store; descriptive correctness-or-appropriate-
abstention was 7/8 versus 6/8. All 24 experiences captured and 23 runs completed
normally, compared with 22 captures and 21 normal completions previously. The
completion gate still failed for one exhausted calibration control. It repeated
empty recall/missing-file reads and never wrote output, unlike its previous
final-turn abstention. Both multiline-r0 extra-field grading ambiguities remain;
initial artifacts were still 7/8. No result was regraded or replaced.

Both withdrawal-r1 later tasks declared checks, emitted two bounded unverified
notices each, recorded three skipped step observations each, wrote appropriate
abstention and completed in seven calls. No shell or harness-check ToolUse was
generated anywhere in the repeat. The old run's four phantom shell calls are
retained. All eight original source snapshots stayed unchanged; before/after
router properties matched. Nonempty recall remained limited to two calibration
memory tasks. Fresh experiences and stochastic/shared-server variation prevent a
causal interpretation of the historical score or cost changes. Later memory/control
used 37/39 calls, 70748/72837 input tokens, 3158/3024 output tokens, and 44.45/39.43
summed seconds; these are not a speed claim. Exact executables, logs, manifests
and trace audits remain under `results/memory-planning-checks-20261107/` on Spark.

### Native bridge comparison harness (2026-11-07, prototype branch)

`memory_bridge_trial::run_trial` connects a real provider to the already-qualified
ordinary-tool/appraisal/capture path. The thin example fixes task budgets and
shares one deadline across model setup and execution. `LocalTrialProvider` holds
the resident model and opts into no-autoload POST requests; default OpenAI-compatible
provider behavior stays unchanged. Host audits retain effective requests,
responses, errors and bounded streamed fragments without converting failed
partials into completed actions. Unknown usage remains unknown.

The Python coordinator freezes source/binary/input/oracle hashes and all planned
slots, copies closed source memory and authority into fresh later processes,
checks original source hashes, and evaluates current task artifacts independently.
Frozen-control checks compare actual requests against expectations as well as
against the paired arm. Local native authored-HTTP preflight passed 24 captures
and seven probes, including source withdrawal, path isolation and abrupt SSE EOF;
102 scripted requests were made, with zero real-model inference. The initial
fixture incorrectly returned nonstream JSON to streaming requests; its eight
failed initial runs and sixteen unstarted later slots remain retained. This is
integration evidence, separate from the forthcoming model comparison.

The same reviewed source passed Spark workspace/all-target build, formatting,
strict all-target/all-feature Clippy, 6303 default tests (zero failures, 25 ignored)
and 6667 memory-enabled tests (zero failures, 29 ignored), with serial test
scheduling. Local Python qualification passed 13 targeted coordinator tests and
91 existing tests (four skips). A first Clippy failure required the executable's
argument pairing to use fixed-size array chunks; the failed log is retained.
Actual-owner-corpus privacy checking passed before inference.

The frozen Spark run used the already-resident `qwen3.6-35b-a3b-uncensored`
(router build `b1915-27637851`), fast mode, temperature 0.6 and two fixed seeds.
All 24 planned runs were attempted, with 112 requests and no inference retries.
Initial artifacts passed 7/8; later exact checks passed 6/8 with memory versus 4/8
with an empty store. Both improvements were calibration-parameter availability;
controls correctly requested missing information. Descriptive correctness-or-
appropriate-abstention was 6/8 in each later arm, not an appraisal-quality gain.
The four changed-criterion runs passed without consulting memory, so they do not
establish retrieved reappraisal. Actual nonempty recall occurred only in the two
calibration memory runs. All eight original source snapshots remained unchanged;
protocol controls and before/after router properties matched.

Three runs exhausted eight turns; 22/24 experiences were captured. Withdrawal-r1
in both arms exposed a real harness/tool mismatch: completed todo checks generated
shell dispatches although the fixed registry had no shell. Their 18/19 eligible
passages exceeded the unchanged 16-passage capture bound. Calibration-r1 control
instead spent turns on empty recall and missing-file searches, writing correct
abstention on its final turn. The complete experiment is retained even though
its all-runs-completed gate failed. Two multiline-r0 exact-check failures were
extra `active` fields, not corrupted newlines; the frozen criterion did not
explicitly exclude that field. This grading ambiguity is recorded without
changing original scores. One initial calibration artifact used the row id in
place of its label and was retained, not regenerated.

Later memory/control totals were 38/42 calls, 74030/80506 input tokens,
3134/3200 output tokens and 40.94/41.92 cumulative wall seconds. All usage was
reported. These small authored cases and shared-server timings do not establish
speed, generalization, concept learning or production readiness. Raw artifacts,
failed attempts, independent interpretation audit and source hashes remain under
`results/memory-bridge-model-20261107/` on the operator's machines.


### Source authorization work per retrieval (2026-11-07)

PR #846 review found repeated external source reads and hashes for passages that
share one binding. The unchanged implementation measured 17/34/67/133 underlying
checks for read/search/recall/context on a synthetic 16-passage capture. Regression
fixtures preserve all 17 context records and require 1/2/2/2 checks. `SourceChecks`
shares exact authority/locator/revision verdicts within selection and separately
within final validation; it never caches database access checks or crosses public
operations. A transaction-wide cache was rejected because SQLite does not freeze
external source files. These are call counts, not a measured latency speedup.

At code `883bd2c72`, standalone memory qualification passed 221 unit tests and
four integration tests. Spark passed workspace/all-target build, fmt, strict
all-target/all-feature Clippy, 6302 default tests (0 failed / 25 ignored) and 6649
memory-enabled tests (0 failed / 29 ignored), both full suites with serial test
scheduling. The retained connected example passed both independent artifact
checks. Scoped review found no major/medium issue; the final automated review is
recorded on PR #846. No new real-model calls or production changes.


**2026-11-07 — ordinary task/appraisal/recall bridge (prototype branch).**
`memory_experience_bridge::LocalAuthority` freezes an explicit synthetic Test
session's criterion and exact JSON-row checker before ordinary `Agent::run_in`
execution. `SessionEvidence::from_transcript_text` shares existing evidence logic
without a second file read; `appraisal::for_transcript` resolves historical
assessment from explicit joined records. Full transcript/appraisal, sensors,
charter and audit data stay in host authority files outside the task workspace.
The reader receives bounded original message passages, typed call receipts,
criterion and independent artifact verdict through `Memory::admit_host_capture`.
Tool success does not establish artifact correctness or owner acceptance.

Explicit new v6 stores enforce source authority during supported reads/ranking
and final context bundles. Each original and projection has a binding; every
passage inherits consulted dependencies. Native GoalMemory result references are
derived before passage truncation, so omitting a manual dependency cannot orphan
recalled content. Immutable dependency-changing reuse refuses. Exact authorized
retries reuse receipts; forgetting erases receipt members/dependents and blocks
late recapture. Required `lineage_version=2` rejects intermediate v6 files with
incomplete lineage, with no migration or effect on v1–v5. This is conservative
capture-wide inheritance, not finer per-passage influence inference.

The shared `memory_bridge` example/test fixture uses ordinary Todo/FsRead/FsWrite
and native session recording. A separate OS process reopens actual GoalMemory,
recalls A, freezes B's changed criterion, executes/checks B and captures its
A-dependent outcome. A chosen-egress tripwire immediately after recall tests
memory taint before file reads can mask it. Complete reader tables/FTS and actual
provider requests are scanned for host sensor/setpoint canaries. Wrong artifacts,
missing sensors/checks, changed goals, authority withdrawal/correction, forgetting,
and capture-only retry have distinct assertions. These are authored-provider
integration checks, not new real-model benefit measurements. Capture recovery
simulates the post-recording boundary; no power-loss fault injection is claimed.

Independent task reviews corrected projection-only context bundles, revision of
guarded prior relations, absent B criterion freezing, a masked taint assertion,
late native result semantics and resumed ranges appraised against an old outcome.
Final review found copied passages losing consulted authority and accepted omitted
consultation; both were fixed with old-behavior failures through the connected
workflow. Late semantic appends now withhold a pinned source; complete Summary
records alone may follow it. The host still must match recorded/actual workspace
and avoid grants or other tool access into host records; arbitrary registries are
not qualified. Synchronous checks emit Passed/Failed/Unavailable/Refused, not an
invented Pending result.

Exact code head `540485f71` passed Spark workspace/all-target build, formatting,
all-target/all-feature Clippy with warnings denied, and both complete workspace
suites: default **6302 passed / 0 failed / 25 ignored**; memory-prototype
**6642 passed / 0 failed / 29 ignored**. Ignored backend/helpers are not counted
as passes. The retained-artifact `memory_bridge` example also passed both
independent checks. Scoped final rereview approved the lineage fixes without a
remaining major/medium finding. Logs, review reports and synthetic artifacts are
retained under `results/memory-appraisal-bridge-20261107` locally/on Spark.
The first final Clippy run found two fixture style warnings; both were corrected
before the complete successful run. No real-model inference was run.

A subsequent current-head automated review identified a native code-mode route
that the direct-tool qualification had not covered: `Record::Program` was
ignored while the outer code result could copy memory without its dependencies.
`12e2432c4` refuses code/program traces at admission and through current source
guards for already-issued bindings. Three native-wire regressions failed before
the admission fix; a fourth showed an old binding still returned Checked before
the guard fix. Both first capture and retry now leave storage unchanged, while
existing originals/projections and actual GoalMemory withhold unsupported traces.
The regression uses actual GoalMemory output and serialized native program records;
it does not execute the interpreter. Scoped rereview also moved the shared
preflight before SessionEvidence assembly, avoiding ambient program-blob reads.
That ordering is source-traced, not measured with filesystem instrumentation.
Final focused checks passed 19 runtime tests, 5 connected tests (plus invoked
child), strict core all-target/all-feature Clippy and formatting. No medium
finding remained in the scoped rereview. The guard adds one bounded transcript
record scan per source check; corpus-scale performance remains unqualified.
Two subsequent parallel default-suite attempts failed before feature tests:
interpreter worker launch returned ETXTBSY, then a mail-rule fixture returned
HTTP 500. The isolated tests passed; the original interpreter group also passed
22 tests with backend skips prohibited. Source review found possible global
home/temp-directory fixture races but did not establish the HTTP 500 cause.
Original logs remain in `code-lineage-final/attempt1` and `attempt2`; no unrelated
production fix was made. Final complete suites use `--test-threads=1`, explicitly
separate from normal GitHub CI and not evidence that parallel-suite reliability
was repaired. At exact code head `12e2432c4`, the final serial runs passed **6302 default
tests / 0 failed / 25 ignored** and **6646 memory-enabled tests / 0 failed /
29 ignored**. Workspace/all-target build, formatting and strict all-target/
all-feature Clippy also passed. The retained-artifact example passed both
independent output checks. Logs are in `code-lineage-final`.

The existing production artifact, services, configuration, owner knowledge and
nightly ingestion were not changed. Automatic production capture, broader
situated interpretation, useful concept learning and rumination remain open;
§31 C is the next consolidation design after fresh connected model qualification.

**2026-11-07 — 48-response memory benefit diagnostic (prototype branch).**
`memory_benefit_trial.py` at `8c5b48246` freezes six authored numeric cases,
seeds 71/83, source-only/source-plus-actual and fast/thinking2048: 48 first responses,
no retries or repairs. It adds no Rust behavior or storage schema. The installed
`de5d60980` binary is unchanged. Five fixture lineages use authored prerequisite
decisions and actuals from durable-act, never direct inserted measurements.
All 48 authored HTTP requests passed before real inference. Ten new unit tests
and the installed-binary full Python suite passed (91 tests, zero skips). Current
workspace build, 6302 default tests / 0 failed / 25 ignored, fmt and strict Clippy passed.
A nondeterministic authored tie failed on Spark and was fixed before inference;
review also corrected treatment/order confounding and unqualified metadata credit.
Original failed logs remain retained. Independent final code review found no
major/medium issue; automated review on `8c5b48246` also found none.

All 48 real responses completed without transport/format errors or uncertain
slots. Both evidence arms made 24/24 evidence-appropriate choices and exact
candidate checks. Against a shared reference substituting only the retained
actual into source expectations, source-only scored 4/24 and outcome recall 24/24:
20 paired improvements, 0 regressions, 4 unchanged-correct. Each reasoning mode had
10 improvements / 2 unchanged. This is an information benefit on six authored cases,
not better reasoning from identical evidence. Hidden-simulator optimality was
0/24 versus 12/24: unobserved alternatives still contain mistaken expectations.
Each arm correctly abstained 4/4 times required, with no unnecessary abstention.

Fast mode had 23/24 effective-citation passes; unchanged/seed 71/source_only omitted
e2 (Arden), and the host withheld it despite correct arithmetic. Thinking had 24/24.
Blinded review of original reasons under a pre-frozen rubric found 47 faithful and
one insufficient (fast outcome-arm provenance wording), no explicit conflicts.
Combined choice/check/citation/host/prose success was 22/24 fast and 24/24 thinking;
a charitable reading of the ambiguity raises fast to 23/24, preserving the citation
failure. Thinking added no choice gain but used 58,743 versus 11,860 output tokens
(4.95x), and 575.838 versus 125.970 summed seconds (4.57x). Input tokens 14,072 versus
14,120; total 28,192 input / 70,603 output, 701.808 summed seconds, no missing usage.
Resident qwen3.6-35b-a3b-uncensored, temperature 0.6, max 8192 tokens; reasoning 2048
was explicit and returned in all 24 thinking responses, none of 24 fast responses.

Independent artifact audit verified the 24 pairs / 48 unique rows, frozen requests
and schema-order arrays, 168 passage provenance links and original/projection
hashes, five copied SQLite logical dumps and before/after state equality. Runner
SHA256 fb378442c6bfd15fc64bc876e28e970080cf05a92d6f2cefd33777f59e28554e;
freeze SHA256 cd8cee3b1c34a16f8bb00ace8776abdcda1a40363891297448773c662cdc397c.
Evidence, oracle/rubric, raw replies and blinded reviews are operator-local under
results/memory-benefit-20261107 on the development machine and Spark. Earlier
24-response results remain unchanged. No generalization, matched v3/v4 advantage,
production learning or action execution is established. Fast inference is the
next baseline for a narrow natural-language workflow; richer unseen tasks,
targeted reconsideration and independently observed outcomes remain to qualify.


**2026-11-07 — comparison provenance distinct from model citations (prototype branch).**
Product `611bbc048` adds `ResponseFormat::FeasibilityProvenance` and CLI
`--response-format feasibility-provenance`, selecting outcome protocol v4.
`PreparedPair::finish` keeps exact model citations/prose/replies and independently
builds `ComparisonProvenance` from the frozen packet and effective arithmetic.
The audit retains all source revision references and original/projection hashes,
effective/superseded/non-numeric roles, method-source mapping and omissions.
`assess_effective_measurement_citations` requires each effective source, including
rejected alternatives and abstention; the shared legacy entry point keeps its
all-passages rule. Missing active sources and false typed checks remain vetoes.
A superseded-only omission can now pass the v4 citation contract without adding
anything to the model's citation array. The reason is `unverified_model_prose`;
existing advisory suboptimal-choice/unnecessary-abstention findings remain.
No persistence schema, dependency, executor or production integration changed.

Meaningful old-behavior regressions preceded implementation. Local qualification
passed 25 outcome and 28 experience tests plus CLI compile/format checks. Independent
core, qualification and final reviews found no major/medium findings. One minor
remains: Python checks request hashes against frozen preparation rather than
recomputing the digest; source/projection digests are independently recomputed.

Spark qualification at product `611bbc048`: **6,587 all-feature tests /0 failed
/28 ignored**, **6,294 default /0 /25**, **198 standalone**; formatting,
strict all-target/all-feature Clippy and workspace/release builds passed. All
**78 Python tests passed, zero skipped**, against the independently installed
release, including eight authored HTTP replies across three v4 cases and a v3
control, plus v4 pre-dispatch revocation with zero completions/no report.
The checks retained exact raw replies/citations, full source/projection hashes,
actual transmitted schema order and unchanged fixture stores. Zero real inference.
The first all-feature attempt hit an unchanged mail-hook ETXTBSY launch error;
its exact retry and the full unchanged rerun passed. Both logs are retained.

Installed isolated Spark artifact:
`~/.local/share/mecha-prototypes/memory-provenance-611bbc048/bin/mecha`, SHA256
`e39b1beb17514d0b996c835e1f587d5d547008ea7e008689a6f0feb23d63cef5`.
Build/install hashes, version and actual CLI behavior verified. Results:
`results/memory-provenance-20261107/qualification/`. Python qualification source
is `ccee4000d`; it adds tests only to product `611bbc048`. The previous measured
model diagnostic remains in HISTORY and `results/memory-constraint-20261106/`.



Automated review then identified that ordinary CI discovery skipped the four
installed-CLI tests without explicit fixture variables. `ci_qualification.py`
now creates a disposable mismatch fixture through the authored loopback action
lifecycle, validates its finalized action, sets the three qualification variables
before discovery, and rejects missing configuration, empty discovery, skips or
failed tests. CI explicitly builds the feature-enabled CLI first. The exact runner
at `7a3bfcd11` passed **81 tests, zero failed/skipped**, on Spark with a fresh
fixture and the qualified `611bbc048` artifact; no real model inference. Scoped
re-review found both this gap and the corrected architecture/main inference comments
addressed, with no major/medium findings. Portable developer skips remain explicit.

Further automated review identified an extraction reasoning-budget gap and missing
direct tests for the strict loopback predicate. At `0c96be336`, `extract-plan`
accepts explicit budgets 1–4096 within its unchanged 8192-token allowance. Its
new parser test failed on the old 4097 acceptance. Eight direct loopback tests
passed; weakening numeric-host and URL-envelope checks produced two and five
failures respectively, and both mutations were restored. Independent scoped
re-review found no remaining issue. Spark passed **6,302 workspace tests /0 failed
/25 ignored**, **1,612 feature-enabled CLI tests /0 /1**, formatting, strict
all-target/all-feature Clippy, release build and the **81-test fresh-fixture
qualification with zero skips**. Thirteen local socket tests had been denied by
the local sandbox; all passed in the Spark workspace suite. No real inference.
The current isolated release is
`~/.local/share/mecha-prototypes/memory-provenance-0c96be336/bin/mecha`, SHA256
`37613b6ab0a5099fe99c1ffca55fea899696849da5de71a778ef16e1c92075d2`.
Build/install hashes and CLI identity match; logs are retained under
`results/memory-provenance-20261107/review-fixes/`.

The next review exposed a reachable validity-clock defect: `GoalMemory` now asks
its host `Clock` on every memory call while preserving its opening knowledge
snapshot. A deterministic test failed when a reused tool retained an expired
relation, then passed after the fix; it also measures later-record exclusion,
current scope revocation, successful empty-result provenance and refusal of a
model-supplied clock. A separate review claim about partial measurement packets
was disproved by tracing the full authenticated action lineage. Both binding and
retained-outcome reconstruction require `Recommend`; incomplete evidence yields
`Withheld` or `Abstain`. Six actual durable read/model cases (0/1/2 measurements
and method/abstention replies) pin that rejection. These are authored decision
callbacks, not real inference or outcome-reappraisal calls. No schema or runtime
restriction was added for the unreachable finding. Independent review confirmed
both the clock repair and the reachability analysis.

At qualified source `de5d60980`, Spark passed **6,598 all-feature tests /0 failed
/28 ignored**, **6,302 default /0 /25**, formatting, strict Clippy, release build
and **81 fresh-fixture Python tests with zero skips**. A test assertion initially
failed Clippy's `err_expect` lint; the direct `expect_err` correction and original
failure log are retained. The new isolated artifact is
`~/.local/share/mecha-prototypes/memory-provenance-de5d60980/bin/mecha`, SHA256
`fa48a1fb68e522fb5f700cf0cf5db3519d5c43be1de97e02add3d19da4568bbf`;
build/install hashes and CLI identity match. Evidence is in
`results/memory-provenance-20261107/final-boundary-fixes/`. No new real-model
calls or production changes. The current handoff keeps this latest state;
prior artifact identities and qualification history remain here.

The first all-feature run failed in the unchanged mail-hook timeout test while
launching its temporary executable with ETXTBSY (Text file busy). The exact isolated
test and the full unchanged suite then passed. Both logs remain in the results;
no cause beyond the observed launch error was established, and no mail behavior
was changed. Current eval inventory is 36 cases /15 tags. Aggregate test totals
do not qualify absent sandbox backends.

This increment made zero real model calls and did not rescore prior diagnostics.
It establishes a separate citation contract and audit, not improved reasoning.
The previous 24-response findings below remain unchanged. Source-revision
transitive freshness, semantic explanation verification and broader multi-seed
quality/cost measurement remain open. Production binaries, services/configuration,
owner memory and nightly jobs were unchanged.


**2026-11-06 — explicit constraints and genuine abstention (prototype branch).**
Product `96f3c479f` adds opt-in outcome response protocol v3 via
`PairOptions`, `ResponseFormat`, `prepare_pair_with_options` and CLI controls.
`diagnostic_request` specifies both inclusive constraints, fastest feasible choice
and empty-set abstention. Structured candidate values, applicable source and
Booleans remain model claims; `PreparedPair::finish` reconstructs them independently
and withholds inconsistency without replacing the raw choice. Legacy v1 remains
unchanged. Reasons use a 4,000-Unicode-character cap and 65,536-byte JSON bound;
recursive duplicate fields, foreign/missing entries and invalid values fail.
Reasoning budget is explicitly opt-in, 1–4096 within the 8,192 output allowance.
No new persistence schema, executor, production wiring or dependency.

Independent core/runner/final review found and fixed raw-reason whitespace bound
handling, then actual HTTP qualification exposed a fake-router discovery fixture
error before any inference. Full Spark qualification passed 6,549 all-feature tests
(0 failed, 28 ignored), 6,264 default (0 failed, 25 ignored), 198 standalone, formatting,
strict Clippy and workspace/release builds. All 76 Python tests passed against the
installed artifact. The corrected 24-authored-response HTTP rehearsal matched
frozen prompts, schemas, generation controls and property order exactly; it used
zero real inference. Installed SHA256
`3139b5531c854e225c580f095e82119f21825b3cc90904a616b9e71af9001150`
matches the qualified release. Current eval inventory remains 36 cases/15 tags.

The initial real run stopped with 22 completed responses and two uncertain dispatch
slots after discovery that changing only the schema's `required` order left choice
before checks in `properties`. All 14 structured replies emitted choice first; the
intended checks-first treatment was not delivered. Preserve that incomplete run
without efficacy interpretation. Corrected v3 explicitly constructs the properties
map in order; raw HTTP strings and explicit order arrays prevent sorted audit
exports from hiding this defect.

A separately frozen corrective diagnostic used four new contracts E–H, seed 59,
both evidence conditions and three treatments: 24 first responses, no retries or
repairs. Total possible inference stayed within the original 48-call ceiling.
This sacrificed replication; new thresholds on inspected fixture lineages do not
make a holdout. Resident model `qwen3.6-35b-a3b-uncensored`, temperature 0.6,
8,192 output cap, shared 4,000-character reason cap; reasoning arm requested 2,048.
All 24 responses were valid, all 16 structured replies emitted checks before choice,
and all 8 reasoning-arm replies returned reasoning-channel content. Original
preparations, source/projection hashes, databases and model settings stayed intact.

Choice-first / feasibility-first / feasibility-first with reasoning scored visible
choice correctness **5/8 /7/8 /8/8**, required abstention **1/4 /3/4 /4/4**, and
unnecessary abstention **0/4 each**. Comparison citations were complete 8/8, 7/8, 7/8;
structured values/provenance/checks correct 7/8 and 8/8. Host withheld 3, 2, 1 responses.
In F/outcome, fast structured output copied Arden energy 3 but marked it feasible
under budget 1, selected Arden, then correctly said no method was feasible in prose.
The two citation omissions were E/outcome with reasoning (superseded Arden
expectation e2) and G/outcome fast structured (superseded Quill expectation e3).
Both reasons discussed the omitted source; the required citation array did not.
No condition passed the full predeclared gate. Simulator-optimal counts 4/8, 3/8, 4/8
remain distinct: E/source and G/H contain information gaps, not evidence that an
unsupported hidden-optimal guess is good reasoning.

Independent qualitative explanation support was 5/8,8/8,8/8, with two false
provenance claims and one ambiguous baseline reason. Combined strict grounded
responses were 2/8,6/8,7/8: three baseline and one fast-structured choice/prose
contradictions cannot be repaired from their prose. Treating the ambiguous baseline
G/outcome wording charitably raises its combined score to 3/8; explicit final-stance
correction adds no credit. All comparisons separate those sensitivity judgments.
No reason saturated its 4,000-character cap (maxima 1,167/996/781 characters).

Per-treatment input/output tokens and summed call time were 3,560/2,591/30.299 s,
4,120/3,681/37.582 s, and 4,104/19,395/187.557 s. Total 11,784 input/25,667 output,
255.438 s; no missing usage. Reasoning cost about 5 times fast structured latency
and output. These are development diagnostics, not equal-compute generalization
or proof of memory benefit. Raw replies and independent audit artifacts remain
in `results/memory-constraint-20261106/`; stopped `pilot/` and completed
`pilot-corrective/` are separate. Production services, owner stores and nightly
jobs were unchanged. Next work is evidence-record simplification followed by
broader frozen replication, not production promotion from 8/8 choices.

Owner-private research Site v0.20 published from source
`00ee7aa18883c61fbd6fa3a9190a024de4d95798`, deployment
`appgdep_6aeead6d74448191929f985175dc3467` succeeded. All 61 prior anchors retained,
62 current, unchanged figure; links/scripts/dist and visible results table checked.

**2026-11-06 — outcome recall under changed goals (prototype branch).**
Implementation `3be9ddecf` adds read-only `memory_outcome_reappraisal` and
`evaluate-outcome-recall`: source expectations remain identical between arms, and
only treatment receives the exact retained action observation through bounded
recall. A same-workload numeric actual supersedes its expectation for current
arithmetic without rewriting either episode. Historical feedback/recommendations
and hidden simulator data stay outside prompts. Opaque guards cover all frozen
raw hits plus complete action/model/read lineage, and fresh canonical-store checks
prevent post-inference reports from an obsolete connection. Source invalidation
aborts the pair; ordinary model failures remain rows. No new schema or action.

Independent task and final reviews found two real guard defects before execution:
unrelated recalled audit hits were not revalidated, and a database replacement could
leave the final check on an old connection. Both reproduced in failing regressions
and were fixed. Pilot review also caught optimized Python removing assertion-based
validation and interruption omitting an incomplete matrix; unconditional checks and
atomic interrupted accounting fixed both. Thirteen focused core tests pass. Spark
qualification passed 6,545 all-feature tests (0 failed, 28 ignored), 6,264 default tests
(0 failed, 25 ignored), 198 standalone tests, formatting, strict Clippy, workspace/release
builds and privacy checking. All 63 Python tests passed on the installed artifact,
including scripted 20-response CLI pairing and paused-discovery source revocation
with zero completion requests; qualification used zero real inference.

The frozen real pilot then made 20 first-response calls: five authored contracts,
seeds 7/19, counterbalanced source-only/source-plus-outcome, no repair. Resident
`qwen3.6-35b-a3b-uncensored`, temperature 0.6, thinking off, 8,192 output-token cap and
zero retries. Four primary pairs scored simulator-optimal 4/8 in both arms: no gain.
Visible-evidence appropriate answers fell 8/8→4/8; outcome context led to Quill at
budget 4 and Nori at budget 2 in both seeds, all four infeasible proposals withheld.
Required model abstentions were 0/2. The fifth hidden-quality pair scored visible
appropriateness 2/2 and simulator optimality 0/2 in each arm; missing knowledge must not
be relabeled as a reasoning error. All 20citation sets were complete, not proof of
explanation entailment. A conservative independent explanation audit judged 5/10 source-only and 4/10 outcome-arm reasons strictly supported, with two additional outcome reasons marked ambiguous rather than false. The grading treats an initial false claim followed by correction as a non-clean explanation; these are explicit qualitative judgments, not an automated truth metric.

No recorded call/format/model-identity errors or missing usage. Source-only cost
3,702 input /1,214 output tokens and 17.773s; outcome context 4,222 /2,197 and 27.000s, total
7,924 /3,411 tokens and 44.773s. Four withheld reasons reached the schema's 1,000-character
limit and ended mid-sentence; valid JSON and sub-cap token counts do not establish
sound reasoning. Exact freeze/preparations/source hashes and unchanged database
were audited. These are development cases, not held-out generalization. No new
action executed, no production service/configuration/owner-store/nightly change.
Installed prototype SHA256 `22adb10ccfc2f82c2a0f0c1d34819a48c340dc2c8cddc110f7733c54e9823a7d`;
artifacts `results/memory-outcome-reappraisal-20261107/`. Next work targets constraint
application/abstention with fresh configurations; this negative result remains intact.

**2026-11-06 — durable synthetic actions and independent outcomes (prototype branch).**
Implementation `4379eef50` adds explicit schema v5 and
`memory_action_runtime::{bind,reserve,PendingAction::execute,settle,advance,run}`.
The host binds a closed finite fixture to an authenticated retained recommendation;
a unique durable reservation precedes execution. Opaque non-cloneable permits
prevent replay after process loss. Independent actual measurements, source
expectations and `planning::StepFeedback` are retained separately with complete
read/model/source ancestry. No model call, arbitrary executable input or production
appraisal/task-board mutation is part of action execution. Cancellation may retain
an already executed result; revoked/changed/forgotten sources deny late content.
Pure feedback completion resumes from a saved outcome. The v5 CLI is explicit and
synthetic-only; earlier stores are not implicitly migrated.

Three isolated worker lanes and independent store, runtime and final reviews found
no outstanding findings. Spark qualification at the exact implementation passed
6530 all-feature tests (0 failed, 28 ignored), 6264 default tests (0 failed,
25 ignored), strict Clippy, formatting, workspace/release builds and the current privacy
gate. Ten process/lifecycle and two actual-binary tests cover real child termination,
concurrency, access, cancellation, erasure and recall. Controlled completion markers
remain 0→0, 1→1 and 1→1 across the three crash boundaries; concurrent reservation
completes one handler. This does not establish exactly-once arbitrary external effects.
The standalone local and installed Spark CLI audits each pass five cases using six authored HTTP replies
and zero real inference. The mismatch preserves source energy 2 and observed energy
9 and fails budget 5; pass/refused/unknown/abstain keep distinct semantics. Recursive
dependency and recall checks passed. Logs and artifacts are under local
`results/memory-durable-action-20261106/` and
`results/memory-durable-action-qualification-20261106/`.

The new observation is independent in measurement origin, but current transitive
freshness still makes it ineligible when a consulted source is revised. That
explicit limitation is not evidence of learned correction. Later model decisions,
production appraisal, useful concept learning and private-data qualification remain
open. The CI memory arm now exports `MECHA_SESSION_KIND=test`, closing the earlier
minor automated-review note. The living owner-private research Site v0.18 records
these results and limits; no main deployment or owner-data migration occurred.


**2026-11-06 — durable synthetic model decisions (prototype branch).**
`3ed543a17` integrates explicit v4 model attempts with the guarded-read lifecycle:
separate original/revision request, raw result and assessment, sealed source/task
binding, one optional source-checked reconsideration, and advisory final output.
`memory_decision_runtime::advance` can stop after response settlement, so a fresh
process rechecks that response without a model call. `decide` drives up to five
steps; interrupted calls without results stay spent. Cancellation retains a started
response while blocking further processing; source revision/restriction denies audit
content, and forgetting erases/suppresses the full read/model dependency closure.
The CLI returns content-free lifecycle status even when source access is gone.

Parallel store/decision/qualification lanes and parent runtime/CLI integration were
followed by independent review. Two medium findings were fixed with regressions:
status/cancel must not depend on content access; invalid model citation shapes must
produce failed pilot scores rather than crash summary generation. Existing trial
prompt/schema output stayed byte-identical across 153 scripted requests. The pinned
Ubuntu CI arm now runs the memory feature tests, closing the draft PR review gap.

Exact implementation `3ed543a17` passed 6500 all-feature workspace tests (zero
failures, 27 ignored), strict Clippy, workspace/release builds and the current
privacy gate on Spark. Local runtime 2/2, process/lifecycle 6/6, CLI 4/4 and Python
pilot 8/8 passed. The local loopback listener test first hit sandbox denial, then
passed with that test's network access allowed. The installed isolated binary's
SHA256 is `a8e1f700fd886fbc7767ecb336ee902c2a08786a5cc46a08a4f6471fc1f81cea`.
No production install, service restart, model switch or owner-data use occurred.

Eight frozen paired tasks completed 16 workflows/19 calls using resident
qwen3.6-35b-a3b-uncensored, seed 7, temperature 0.6, no thinking, 8192 output-token cap,
zero transport retries. No-memory abstained 8/8. With retained observations,
source-supported optimal choices improved 5/6 to 6/6; the other two cases still
proposed actions with missing comparison evidence or no feasible method and were
host-withheld. The three revisions repaired the fresh-value choice but did not
induce the two required abstentions. One final recommended choice had a contradictory
constraint explanation. Usage: 6009 input/2283 output tokens, including 2064/451 for
revisions; 32.2s summed call time and 37.9s CLI workflow time. This shows conditional
use of explicit stored measurements, not retrieval ranking, executed outcomes,
generalization or an equal-cost advantage. Artifacts remain local under
`results/memory-durable-model-pilot-20261106/`; qualification logs under
`results/memory-durable-model-20261106/`. Independent audit verified all 16 databases, 153 revision hashes, 24 frozen
source-file hashes, exact request/result/assessment/final payloads, source closures
and source-only oracles. All 16 final explanations were reviewed: 13 supported;
the two withheld proposals and the self-contradictory energy-bound rationale were
not. Complete citation arrays therefore did not establish explanation correctness.


**2026-11-06 — publishable memory prototype snapshot on current main.**
The original private prototype branched before GitHub main's history rewrite.
Only its 116-file memory patch was transplanted onto `6b709e08b` as
`codex/mecha-memory-v2-prototype`; none of the stale unrelated history was pushed.
The two additive conflicts preserved current mail-rule entries and memory entries.
Independent source review confirmed memory/runtime code unchanged and current-main
routing/provenance additions retained. Candidate `a4de9bb2f` passed 6463 all-feature
workspace tests (zero failures, 26 ignored), all-target build, strict Clippy and fmt.
The strengthened privacy check passed on Spark; generic-word warnings were checked
against synthetic fixtures, host references and the owner's Site URL. No model calls
or production installs were made for this integration. This draft snapshot makes
the implementation and next-task handoff available across machines; it is not a
merge or a completed memory MVP.

**2026-11-06 — durable synthetic reads through the existing guarded dispatcher.**
`59ee1882b` implements design §§23–24 through parallel store, runtime and independent
qualification lanes, with parent-owned CLI/integration/publication. Explicit
`Memory::create_durable` creates schema v3; ordinary creation and v1/v2 experiments
remain unchanged. `memory::durable` reserves an acquisition before awaiting tools,
retains actual observations as source-linked revisions and keeps interrupted calls
indeterminate/spent across restart. It permits one untrusted supplied proposal,
rechecking all original sources and ancestors, including uncited alternatives.
`memory_runtime::acquire` binds one synthetic file through `Agent::dispatch_one`;
it inherits guard/approval/provenance behavior and restores taint on reuse. The
adapter lives outside `memory` to preserve the standalone store harness. No model
calls, second executor, production capture or semantic-success claim were added.

`DispatchGuard::check` rechecks persisted authority immediately before the read,
after awaited hooks/approval. Cancellation retains observations but blocks later
commit; forgetting erases dependent content and suppresses lifecycle-owned source
keys so late settlement and same-key capture cannot resurrect it. Generic capture
behavior is unchanged. Opaque receipts/permits cannot be reconstructed from model
JSON. Cooperative cancellation after the final safe point and filesystem open races
remain explicit limits; this is not exactly-once physical execution. Current
cancellation is available through the task accessor; generic recall is historical.

Exact Spark source passed all-target/all-feature build, strict Clippy, release build
and **6100 all-feature tests /0 failed /26 ignored**. Local standalone store checks
passed 158 unit +4 integration; runtime checks passed 14/14 and CLI integration 2/2.
Independent review found no remaining major/medium issues. Subprocess tests kill a
real worker after dispatch and before settlement, and prove no repeated read. The
separately installed prototype passed 19 CLI invocations: stored bytes survive file
rewrite/deletion, taint restores, only one proposal commits, cancellation blocks
commit, forgetting blocks resume and removes raw-database canaries; integrity/FKs
pass. Binary SHA256 `eb7173525fc67f048923f244548ec3fc6a7fb6e34363605955d4d1dac562d6a7`;
installation `~/.local/share/mecha-prototypes/memory-durable-20261106/bin/mecha`.
Artifacts: `results/memory-durable-runtime-20261106/`. No new model calls or production
changes. The next slice is bounded model integration into this durable lifecycle.

Research Site v0.16 published source `4baec9a829d2c25e1ce7b05abbe59b2460255625`,
deployment `appgdep_6aee3c033460819184530c95185d8ded`, owner-private. All 56 prior
anchors retained (58 total), original SVG unchanged. Fixed stale version labels and
revision-table rendering; local browser verified computation expansion/architecture
switching, and deployed content visibly showed the new qualification. The earlier
Mac loopback timeout did not recur this session; no OS settings changed.


**2026-11-06 — one reconsideration after observation acquisition.**
`d3cacdcfe` factors the existing `checks::reconsider` helper for reuse by
`observations::run_with_reconsideration`. V2 remains intact; opt-in v3 uses the same
prepared receipt across the initial draft and at most one source-only retry. The
final trace rechecks all original dependencies atomically, preserving draft/retry
calls and assessments. No retry expands acquisition or resets freshness authority.
CLI/runner accounting includes every retry and deduplicates shared queries.

The isolated fixed development run completed 51 calls/24 rows with no errors.
Within-run structured justified decisions improved 9/24 to 24/24; independent rationale
plus contract judgments improved 8/24 to 23/24. Control abstained 12/12 times; observe
made six supported optimal choices and six required abstentions. All 15 retries
repaired typed failures (14 abstentions, one missing citation). One unchanged control
rationale and one acquisition query retain unsupported claims. All 240 revisions,
24 outcomes, exact retry bindings, current sources, correction history and cost totals
passed independent audit. The extra 15 calls used 9708 input/1685 output tokens and
22.637 seconds; overall 22620 input/6088 output and 84.6 seconds wall time. Reused
fixtures and additional compute do not establish independent generalization.

Exact-source qualification: 6057 all-feature Spark tests passed, 25 ignored; 143
standalone and 40 Python checks locally; strict Clippy/build/fmt passed. The unchanged
baseline's worker-spawn ETXTBSY failed once; exact and full unchanged retries passed.
No unrelated interpreter fix is claimed. The separate prototype release left main
Mecha and mecha-graph untouched. Results live under `results/memory-reconsider-20261106/`.

Source investigation found that the next durable read can reuse
`Agent::dispatch_one`, but cannot copy the simulator's post-result allowance ordering.
Reserve before external work; interrupted reservations remain indeterminate/spent.
Design §23 records the proposed lifecycle, forgetting/taint obligations and parallel
work packets. That runtime adapter is not yet implemented. Research Site v0.15
published owner-private with these results and proposed work, source
`22c2e580d383741a43a9b246335a081d7f6220e6`. All 54 previous anchors, figure and scripts
were preserved/checked; Mac loopback timeout prevented a new browser preview.


**2026-11-06 — bounded observation acquisition and a separate development refinement.**
`8b7adf0b3` introduced `experience::observations` and `workflow-trial --observe`:
one missing-method request, explicit host outcomes, source-correction history,
per-case source selection and fresh checked final decisions. The initial run had
12 correct query methods but six shared queries invented the citation ID
`Correction notice`, producing 12 error rows. Nine incomplete final proposals were
withheld. Supported optimal decisions were 0/12 control and 3/12 observe; independent
strict final decisions were 0/12 and 2/12 because Elm-3 invented a duration and
contradicted feasibility. The independent audit verified 168 retained revisions,
including failures; correction recovery did not reach acquisition in this run.

`ed6d0e887` enumerated valid query citation IDs and supplied a source-only missingness/
feasibility check before final decisions without narrowing their choice enum.
Its separately recorded v2 development rerun completed 36 calls/24 rows without
format/transport errors. Observation enabled 5/6 supported optimal decisions where
measurements were available; Elm-3 chose correctly but omitted one ruled-out
alternative's citation and was withheld. Only 2/6 unavailable cases abstained;
the other four were withheld. Control abstained 5/12 times and had 7 proposals
withheld. Independent strict supported final decisions were 4/12 control and 7/12
observe; a correct abstention with a false rationale failed this stricter criterion.
All 240 revisions, source/projection hashes, correction history, dependencies
and computed outcomes were independently audited. Same authored fixtures and added
host guidance mean this is refinement evidence, not independent generalization.

Qualification on the exact v2 source: 6054 all-feature Spark workspace tests passed,
25 ignored; 140 standalone and 38 Python tests; strict Clippy on both hosts. Local
network tests were limited by a localhost connect timeout independently reproduced
with Python. Spark's unchanged baseline and changed suite passed; no OS networking
settings were altered. Each run used a separate pinned prototype binary, leaving
production Mecha and mecha-graph unchanged. Artifacts are local under
`results/memory-observe-20261106/` and `results/memory-observe-v2-20261106/`.


Research Site v0.14 was published owner-private with both observation runs and the
remaining failures: source `21af89fbe5f0db8a5b3244e01d19ee8810896b96`, successful
deployment `appgdep_6aee1d9d335c8191bcf1d47cf909347a`. All 53 previous anchors,
the existing diagram and inline scripts were preserved and checked; a new local
browser preview was blocked by the independently reproduced Mac loopback issue.

**2026-11-06 — source-grounded checks and bounded reconsideration (`c75a65310`).**
`experience::checks::assess` validates the original supplied observations, requires
this fixture's complete comparison evidence/citations, and recalculates energy and
quality feasibility under current state. Suboptimality is advisory; feasible but
suboptimal final choices remain executable and can lose. `Trial::Checked` shares
12 initial calls between 24 policy rows and permits one reconsideration per case.
`World::finish_with_review` persists both attempts/checks, preserves proposed versus
executed choices, withholds inadmissible actions, and keeps atomic source freshness
and dependency checks. Failed/malformed/model-mismatched calls remain errors.
No schema, production authority or personal-data changes. Independent review found
one reporting error: raw host withholding was counted as model abstention even
though Python compensated. A failing regression reproduced it; the core metric now
uses the model's proposed choice. Final review found no major/medium findings.
Qualification: 6039 all-feature workspace tests passed, 25 ignored; 130 standalone
unit plus 4 CLI tests; 36 Python tests; strict workspace/standalone Clippy and fmt.
The restricted first workspace run failed at loopback binds (Operation not permitted);
the full rerun with local-network access passed. No application fix was needed.


**2026-11-06 — bounded reconsideration measured on Spark (`c75a65310`).**
The fixed 24-row paired protocol completed 16 calls: 12 shared initial drafts and 4
reconsiderations, no transport/format failures. Resident qwen3.6-35b-a3b-uncensored,
seed 7, temperature 0.6, fast, max 2048, no transport retries. Supported optimal
choices improved 8/9→9/9 and required abstentions 0/3→3/3. All four revisions came
from the model; no host withholding was needed. Raw optimality fell 10/12→9/12
because two incomplete-evidence guesses were accidentally optimal. Dahlia-1's
structured choice was repaired to match its correct explanation. Aster-3 and
Bramble-3 still contradict their valid choices with all-feasible opening claims.
Independent semantic review scored strict justified decisions 6/12→10/12, while
supported prose alone stayed 10/12; all original artifacts remained unchanged.
The checker handles source arithmetic, not explanation semantics. Actual usage
5839/1878 input/output tokens; reconsideration added 2207/434 and 5.99 seconds.
The paired policy has more compute; no equal-cost unguided second-call control
was run. Read-only audit verified 105 revisions/all 24 rows, exact source/projection
hashes, dependencies, durable citation bindings and both attempts/checks. These
are synthetic one-seed development results, not online learning or generalization.
Living Site v0.13 published owner-private with these results and remaining milestones;
source `80f0da0bf13e86d2720b486bbe6e30f2c6e41d2f`, successful deployment
`appgdep_6aedec1a525c819184f2499d9e3c26ec`. All 52 prior anchors were retained;
local preview verified controls, results and rendered computation.

**2026-11-06 — recall-view and fixed unseen-case measurements (`4a9365995`).**
After Tailscale authentication, the isolated Spark binary completed 72/72 calls
on qwen3.6-35b-a3b-uncensored (seed 7, temperature 0.6, fast, max2048, no retries).
Legacy/measurements/scoped all scored 11/12 optimal; feasible choices were
12/12, 12/12, 11/12. Strict prose passes 8/10/10 become 8/9/10 when requiring
matching justified choice. All legacy inputs/replies matched the prior baseline.
The separately fixed measurements-view unseen protocol yielded 12/12 appropriate
no-memory abstentions. Retrieved/oracle each scored 9/12 optimal, including one
unsupported lucky choice: 8/9 complete-evidence decisions were optimal and all
3 incomplete cases violated required abstention. Each has two choice/reason
mismatches; all 12 retrieved/oracle requests/replies are identical. Source correction
was retrieved correctly, but occurred before evaluation, so no online adaptation
claim follows. Independent SQLite audits verified 72 rows/286 revisions, exact
source/projection/context hashes, bindings and outcomes. The simpler view reduces
some prose defects in this one-seed run but does not improve optimality. The next
qualification target is source-grounded decision admissibility and bounded
reconsideration; prose validity and successful action remain separate scores.


**2026-11-06 — context-scoped recall comparison implemented locally (`58be2b450`).**
`experience::run_trial`, `World::prepare_view` and the shared workflow runner add
legacy/measurements/scoped projections with original source bytes and hashes
preserved, dependency-bound historical context and stale-context rejection.
Baseline inference remains available unchanged. A separately fixed unseen protocol
covers four new workloads, missing evidence and a pre-snapshot source correction.
Local tests verify projection isolation, persisted source bindings and fixed call
matrices. Model quality is unmeasured for these variants: Spark SSH authentication
prevented the new build/trials. Final local qualification: 6032 all-feature
workspace tests passed, 25 ignored; 127 standalone and 35 Python tests passed;
strict Clippy and formatting passed. Site v0.11 reports this pending-model status.
The system-monitor test scratch helper also uses
UUIDs now: timestamp-derived names could collide (confirmed by a local clock
probe), letting one parallel test delete another's SQLite directory and causing
SQLITE_READONLY_DBMOVED. This is test isolation, not a production database change.



**2026-11-05 — synthetic experience workflow, first memory-benefit measurement
(`4fc8e14da`, isolated prototype).** `memory::experience` adds host exploration,
separate event/proposal/observed-outcome captures, durable source bindings and
atomic provenance edges through the existing SQLite schema. `World::prepare`
seals state/store identity and source revisions; `World::finish` accepts only
choice/citations/reason, computes fixture outcomes on the host, and rechecks all
consulted ancestors before committing. The CLI `memory workflow-trial` uses
existing resident-model holds and quarantined inference, emitting each row as
JSONL. `experience_trial.py` preserves partial output, failures and missingness.
Restart-before-recall and final reopened readback are explicit, not inferred.

The frozen run used four workloads × three goal/resource contexts × three arms
(36 calls), resident qwen3.6-35b-a3b-uncensored, seed 7, temperature 0.6, fast inference,
max2048 output, no retries. No-memory abstained appropriately 12/12. Retrieved and
oracle memory each made 12 feasible and 11 optimal choices; all source sets and
citations were complete. Retrieved/oracle request and reply pairs were identical
for all 12 cases. Their equality diagnoses no retrieval gap in this easy exact-name
fixture; it is not two independent replications. All-context adaptation passed in
3/4 workloads. Output tokens none/retrieved/oracle: 698/1500/1500; summed call seconds
41.4/79.8/74.6. Shared-server timing is descriptive. All 36 calls completed without
host or transport failures, and the resident model was unchanged after the run.

Independent semantic review accepted 12/12 no-memory reasons and 8/12 reasons per
memory arm. Sable-3 chose Arden (10 minutes) although the reason concluded Nori
(6 minutes). Kestrel-3, Juniper-3 and Sable-3 falsely claimed every method was
feasible before excluding an over-budget alternative; Marlin-3 used "infeasible"
for a slower feasible choice, separately classified as terminology. Historical
`feasible=true` is mixed with raw outcome metrics while the original state is
only in a linked event, omitted from recalled text. This may contribute; no causal
ablation has established it. The next step is scoping those appraisals and checking
current feasibility/choice consistency, then unseen and changed-evidence cases.

Independent read-only SQLite audit verified all 36 observed outcomes, source
hashes, saved citation maps and dependencies across 144 records. Artifacts live
in `results/memory-experience-20261105/`: manifest, streamed progress, rows/report,
summary, semantic review with hashes, outcome audit/script and qualification.
These are seeded observations reused in authored development fixtures; model
outcomes are stored but excluded from recall, so online learning is untested.
No production executable, services, personal data or mecha-graph changed.

Living Site v0.10 published owner-private at the existing Mecha URL, source
`5daed5061dc7c6aacb2a3fed55837c87a22f6ee2`, deployment
`appgdep_6aed5cd1423881919b9d8ba6fbc6bf4e`. The preview retained every old anchor,
figure and control and rendered the expanded computation; 50 unique IDs now.

Final checks: 6029 all-feature workspace passed/0 failed/25 ignored; 120 standalone
unit +4 CLI tests; 34 Python tests; strict workspace/standalone Clippy and fmt/diff
checks passed. A baseline HUD test collision showed another test's bytes in its
scratch directory; the timestamp-based test name now uses UUIDs. The repaired
default baseline passed 5892/0/25. One overlapping workspace run lost a doc-test
rlib during rebuilding; the sequential final test→Clippy run passed. Independent
review caught whole-file UTF-8 decoding losing all preceding rows on a truncated
final character; a failing-then-passing test and actual mocked timeout exercise
now verify line-by-line recovery. Another regression proves source-label bindings
are persisted in SQLite, not only in the artifact report. No merge or release.


**2026-11-05 — direct-versus-extraction comparison and request-local reasoning budget.**
`a7c2db727` added a bounded direct coverage judgment over the same host-owned source
receipt as V2 extraction, opt-in reasoning, and a rotated four-arm diagnostic.
`2b612c175` exposes the existing provider reasoning-budget field for these explicit
requests. The command opens no DB and has no action tools; the server/model settings
are not mutated. `DirectReport` stays distinct from the deterministic checker.
Review caught inherited debug logging that could retain reasoning-only content and
incomplete preflight accounting: the harness forces warn logging, verifies the
configured process budget plus no-autoload props, and records aborted matrices.
The V2 extraction prompt literal/schema source were verified byte-identical.

The inherited 4096 pilot completed only case 1's four arms before an operator stop
during case 2 extraction/thinking. Extraction/thinking timed out after 180 s; direct/thinking
returned a source-supported answer in 144.1 s versus 12.8 s direct/fast. The aborted
progress and interrupted attempt remain recorded; they are not a 12-case result.
A fresh matrix requested 512 reasoning tokens, with the same resident model
(qwen3.6-35b-a3b-uncensored), seed 7, temperature 0.6, 8192 total output cap and no
retries. All 48 calls returned across 12 reused synthetic development cases.
Extraction fast had 0/12 host-valid/faithful outputs; extraction thinking had 1/12
(case 10). Direct fast and thinking each had 12/12 host-valid and 10/12 correct labels,
but strict source review gave 4/12 and 5/12 grounded judgments. Fast had 6 unsupported
and 2 ambiguous explanations; thinking had 7 supported explanations but 2 omitted
required returned citations. Both direct modes failed nested guards. Fast also
confused current approval with a future guard; thinking missed unknown rule scope.
Correct labels sometimes accompanied invented prerequisites, invented actions,
contradictory reasoning or no explanation. The simpler format is easier to produce;
neither path qualified as a reliable validator or action authority.

Fast/thinking extraction used 5,734/11,678 output tokens and 293.7/582.2 summed seconds;
direct fast/thinking used 2,370/7,664 and 144.6/390.3 seconds. Input tokens respectively
6,722/6,698/4,886/4,862; all usage known. Reasoning content observed in all 24 enabled
calls and none of 24 fast calls; only character counts, not that content, were saved.
Timings reflect a shared server, not controlled throughput. All 48 per-artifact
reviews verified input/output hashes. Frozen data is reused, unblinded development,
not a holdout, memory-retrieval measurement or evidence of generalization. Artifacts
are `results/memory-mode-capped-20261105/`; pilot remains in
`results/memory-mode-comparison-20261105/`. The source review distinguishes citation
completeness, substantive extraction, semantic support and ambiguous wording.

Qualification: 6,019 all-feature workspace passed/0 failed/25 ignored; 111 standalone
unit +4 CLI integration tests; 28 Python tests; strict Clippy/fmt/diff passed. Socket
permission failures in the restricted workspace run were rerun with local fixture
access and passed; existing Mach-O linker warning remains. Site v0.9 published
owner-private from 06c6cb103bd8f08e40d7969abdc259ff0beb1f34, deployment
appgdep_6aed53357534819193c51da57b6ee819. Preview checked results, retained all prior
anchors, figures, interactive controls and rendered computation. The next design
is a small host-observed episode/decision/outcome/retrieval workflow (§18), with
more automatic action-tree extraction deferred. No production or mecha-graph change.


**2026-11-05 — source-linked extraction, with failed model qualification.**
Core V1 `39b745d7f` and runner `b35260d1a` added a bounded synthetic translation
receipt and `memory extract-plan`; V2 `eb405ff1f` puts citations inline and derives
report paths in the host. The model supplies conditions, observations, actions,
requirements and branch trees; exact source text/hashes come from the receipt.
The command uses existing local-only, resident-model, tool-free transport and
opens no database. Production and mecha-graph were not modified.

The frozen 12-case V1 run completed all model calls but only 2/12 outputs passed host
checks, and independent raw-artifact review found 1/12 faithful. Seven invented
citation target paths. A host-valid case 12 reported covered after dropping Birch's
Mira-approval requirement and unknown fallback. The interface change retained V1
and reran the same 12 development cases separately: V2 had 1/12 host-valid and 1/12
faithful (case 10 in both versions). First failures were 6 unknown actions, 1 unknown
condition and 4 wrong-kind citations; deeper review found wrong-kind citations in8
cases, including 4 masked by reference failures. V2 case 1 preserved the unsafe tail
but omitted declarations/requirements; case 6 preserved nested guards and both
requirements but cited plan text as evidence. No format repair was counted as
semantic improvement or action authorization.

Both used qwen3.6-35b-a3b-uncensored, seed 7,temperature 0.6,thinking disabled,
max 8192,no retries,one call/case. V1 cost 7,310 input/10,070 output tokens and 104.4s
summed case time; V2 cost 6,722/5,828 and 67.1s. These are unblinded reused synthetic
diagnostics, not held-out performance or a reasoning-enabled comparison. Artifacts
and per-case reviews live under results/memory-source-extraction-20261105 and
results/memory-source-extraction-v2-20261105; every saved input hash was verified.

Independent code review caught a V2 parser regression: serde_json::Value silently
keeps the last duplicate field, so ambiguous raw plans could lose content before
validation. A duplicate-rejecting deserializer now runs before inline conversion;
plan/requirements/kind/citation duplicates have RED→GREEN tests. Empty branch arms
no longer consume a depth level. Final qualification: 6,017 all-feature workspace
passed/25 ignored;109 standalone unit+4 CLI integration;19 Python;strict workspace
and standalone Clippy, fmt and diff checks. The next experiment is staged declarations
and constrained branch references, plus a bounded reasoning-mode comparison;
semantic completeness remains independent of reference validity.

Living Site content v0.8 was published with owner-private audience retained, source
`f233128d544dd8fd9c5efc064b9081eb8a3cc207`, deployment
`appgdep_6aed43c73f0c8191a0c6cf647e0065cd`. Local content/anchor checks and real
browser goal/architecture/notation controls passed; the original diagrams remain.


**2026-11-05 — explicit actions and guard coverage (`33f4f68be`).**
`memory::guard_coverage` adds a pure checker over a separate fixture-authored
contract and typed action/branch tree. Conditions have met/unmet/unknown states;
requirements separately have applicable/not-applicable/unknown scope applicability.
Only a met path assumption for the exact condition covers its action. Branch
assumptions cannot escape into later steps; every feasible arm is checked regardless
of the supplied snapshot. Contradictory paths are retained as unreachable, and a
plan without reachable actions is not called covered. Hidden malformed arms and
bounded-size/depth violations fail. There is no action executor, model call or
runtime authority change; V5 prose assessment remains untouched.

Seven core behavior tests first failed against the unimplemented checker; the
standalone adapter likewise failed the request/guard fixture before implementation.
6,008 all-feature workspace tests passed /25 ignored; 101 standalone unit +4 CLI integration tests, 15 Python tests, strict workspace/standalone Clippy. The 16 frozen structural cases all matched: 4 covered, 7 uncovered,
1 unresolved, 2 no reachable actions and 2 invalid references. Independent review
found no major/medium issues and verified the frozen labels. An independent oracle
then checked 150 generated plans against all 27 assignments of three immutable
conditions (one contract, seed 6032), agreeing on every occurrence's status and
snapshot match. Aggregate report status is separately covered by frozen fixtures.
The minor missing per-input hashes were added to satisfy the written qualification
contract; the unchanged checker still matched 16/16. Outputs and oracle script/log
remain under `results/memory-guard-coverage-final-20261105/`.

These supplied structures are additional information, not automatic source
extraction or proof of semantic improvement over V5. Empty requirements and explicit
non-applicability mean only what the supplied contract says. Real effects and
observation refresh are absent; they must precede execution integration. No Spark
installation or production restart was needed for this deterministic experiment.
The first all-feature workspace run failed existing HUD tests
`a_read_returns_typed_rows_and_column_names` (table already exists) and
`a_pragma_a_write_and_a_second_statement_are_refused` (database disappeared).
`hud::runtime_tests::Scratch::new` used PID plus wall-clock ticks and
`create_dir_all`, allowing concurrent tests to share and remove one directory.
It now uses UUID names and exclusive `create_dir`; focused HUD tests passed.
Production HUD code is unchanged.
Living Site content v0.7 published successfully with owner-private audience retained:
https://mecha-compositional-reappraisal.luke-j-chang.chatgpt.site/#implementation-roadmap
Source `d6d4bf5a95898e9a2e6bca3dca608f919bbf0917`, deployment
`appgdep_6aecf76cb7d88191a75cfca687ea4714`. Local HTTP/content/anchor checks passed;
browser preview verified branch example layout, goal/architecture controls and
expanded computational notation. Existing figure retained.

**2026-11-05 — bounded step repair V5 (`56ef98ea3`, citation schema `26bf2a38b`).**
`plan_repair::request/apply` replaces free-text replacement plans with keep/gate/defer
selections. Host-rendered gates preserve the original step and all its requirement
conditions/scopes; strict exact-count/order/identity checks reject omissions,
new actions and no-op edits. Unknown scope or oversized gates permit deferral.
`EvaluationReport.repair` records original/rendered mappings; `evaluate_plans`
prevents all-deferred plans from being recommended. Source freshness checks and
three-call bound remain. Routing IDs are audit-only; task evidence is projected
into prompts without operational storage metadata. Source `26bf2a38b` aligns
status-specific response schemas with the existing combined six-citation limit.

Independent code review caught scope descriptions lost during gate rendering;
a red-to-green regression covers preservation. Tests: 5,999 all-feature workspace
passed /25 ignored, 92 standalone unit +2 CLI, 15 Python, strict workspace and
standalone Clippy. An unchanged baseline SQLite disk-I/O test failure did not
reproduce in isolated or full unchanged reruns; underlying cause remains unknown.

The neutral eleven-case V5 replay completed 11/11 but matched only 4/11 initial
prerequisite labels and preserved 2/4 adequate originals. It used 27 calls,
23,855 input/8,154 output tokens and 126.5s summed time. Four gates, four full
deferrals, no repaired recommendation. All 128 assessment quotations were exact.
Independent unblinded artifact review nevertheless found an unsafe unguarded
publication tail recommended unchanged, a gate conditioned on absent confirmation,
and a valid approval request blocked until approval. V2's higher 10/11 label score
also coexists with an unsafe repair. The V5 reused six-case replay completed 4/6
with 2/6 correct labels, 12 calls, 12,801/4,394 tokens and 67.2s; two failures were
the combined citation budget mismatch. The separate two-case schema rerun completed 1/2; one deferred, the other failed
on a duplicate passage citation. Neither hit the count-limit error. Preserve
this as a targeted regression, not a replacement six-case cohort. Duplicate
citation output remains an open format failure.

Resident model `qwen3.6-35b-a3b-uncensored`, seed 7, temperature 0.6, no retries or
actions. Source/schema/prompt/compute differ across versions; reused development
cases do not establish generalization or causal performance improvement. Original
failures remain under `results/memory-bound-repair-20261105/` alongside summaries,
model calls and artifact review; earlier V2/V3/V4 runs remain intact. The prototype
remains synthetic/default-off, with production `fec8d55c7` unchanged and active.
Living Site content v0.6 published successfully, owner-private audience retained:
https://mecha-compositional-reappraisal.luke-j-chang.chatgpt.site/#implementation-roadmap
Site source `50f8957c5d79de9bc15b038ec17681e79e0cd536`; deployment
`appgdep_6aecf014401c8191afbcf55e96a52591`. Local HTTP/content/anchor checks passed;
browser preview verified goal switching, architecture selection and expanded
computational notation. Diagram retained. Prior V4 publication record remains in
HISTORY; this is a dated update, not an automatically synchronized dashboard.

**2026-11-05 — explicit requirements, guarded steps and uncertain outcomes (`fb598d189`).**
`plan_requirements::validate` checks original citations and indexed candidate steps,
and combines unmet/unknown requirements with the existing assessment checklist.
`Dimension::Outcome` unknown alone does not prohibit consideration of a permitted
next action; contradicted outcomes still require revision. Requirement interpretation,
coverage and guard adequacy remain model judgments. No action authority changed.

The first implementation (`d17d72d50`, V3) passed deterministic tests but completed
only 3/9 cross-domain and 2/6 reused logistics workflows on the resident model.
The model invented sub-step numbers, paraphrased guard quotes and omitted required
citations. V4 constrains assessment schema branches to each candidate's actual step
count, selects whole existing steps as guards, and requires original citations for
all requirements. Independent review found no major/medium code defect. A minor
format risk remains: the schema permits another step of the same candidate as guard,
although the host rejects it. V4 tests cover candidate-specific vocabularies,
cross-candidate guards and uncited unknown requirements.

Qualification at runtime source: 5,991 all-feature workspace tests passed /25 ignored,
84 standalone unit +2 CLI, strict workspace/standalone Clippy. Fourteen Python tests
passed after the subsequent neutral-goal fixture correction. The grader now fails
malformed protocol, checklist, revision/recommendation and call-audit rows rather
than crashing or treating them as current reports. Both raw model prerequisite and
host-combined requirement status remain available.

The original nine-case comparison used diagnostic goal names: V2 completed 9/9
with 8 matching initial labels; V3 3/9 with 3; V4 9/9 with 5. An independent artifact
review caught models using those names as evidence. Preserve those runs as
confounded diagnostics. The corrected fixture uses a neutral `task:memory-calibration`
identifier and adds timeout-bypass and unguarded-publication-tail cases. A regression
test failed on the leaked names before the correction and passes afterward.

The neutral eleven-case replay completed **V2 11/11 versus V4 7/11**, with **10/11
versus 3/11 completed workflows having correct initial prerequisite labels**.
Failed workflows remain in that denominator; these are not eleven observed action
outcomes. V2 preserved four adequate plans, V4 one. Both made 25 calls, but V2 used
23,222 input/5,927 output tokens and 81.7s summed case time; V4 used 36,279/17,271 and
199.9s. All usage fields were present. V4's 142 returned assessment quote occurrences
matched original byte ranges; V2's 47 did too. Exact citations did not prevent
wrong prerequisite states, irrelevant constraints or unnecessary revisions. V4's
four workflow failures were duplicate requirements or unknown checks without a
question. The V4 reused-logistics replay completed 5/6, with four correct initial
labels and 111/111 exact returned quote occurrences. The analogous V3 replay
completed 2/6. No revision-success aggregate is claimed.

Artifact review found a V4 timeout-then-export repair whose extracted condition
was the timeout itself, instead of required consent. Its recommendation was
withheld for unrelated unknowns, not a correct diagnosis of the bypass. Another
repair promoted storage scope `fixture-mail` into a communication method. Permitted
questions were still blocked by uncertainty about replies. The neutral V4 approval repair even published after 24 hours while its host
requirement status was supported; generic unknowns still withheld the recommendation.
A neutral routing goal ID also remained a distraction when treated as substantive
goal evidence. Keep identifiers separate from plain-language task meaning.
The simpler V2 is not
validated either: its neutral approval repair recommends an unguarded later
publication, and an export repair wrongly forbids the no-consent-required branch.
The V4 increment **did not pass its quality gate**; more fields did not establish
better reasoning. Next work is action-to-requirement binding and constrained repair
inputs, not action selection or broader feature migration.

All runs used resident `qwen3.6-35b-a3b-uncensored`, seed 7, temperature 0.6, one pass
per version, no retries or action execution. These are synthetic development cases,
with changed prompts/schema/compute and unblinded artifact review; no causal,
generalization or real-world policy claim. Raw manifests/calls plus summaries and
separate artifact review remain under `results/memory-requirements-20261105/`.
Original failed runs were retained. Spark used isolated installs; production did
not receive the new evaluator. The independent mecha-graph boundary is unchanged. Living Site content v0.5 was
published from source `7fcac45c456ccb6a11bbfd0d838342780b7d7196`, preserving its
owner-private audience, figures, math and interaction logic. HTTP/content/anchor
checks passed; browser visual QA was unavailable due the browser-control session.
Native deployment succeeded.

**2026-11-05 — constrained evaluator and conservative revision, source `ea16b5304`.**
`model_dimensions` makes concept-check presence a host decision: without a retrieved
concept, models supply only prerequisites and factual support, and the host adds
`not_applicable` labelled `judgment_source: host`. Model-authored host judgments,
extra dimensions and original/concept citation category mismatches fail closed.
Known-status judgments still require original evidence, and concept applicability
also requires provisional-concept evidence. Protocol `constraint-assessment-v2`
keeps host passage construction and the original all-source authorization checks.
`evaluate_plans` now preserves any already-recommended candidate; only absence of
one triggers a minimal repair and reassessment. `RevisionStatus` distinguishes
this from not requesting revision. The final source check runs even on preservation.

Regression tests reproduced unnecessary rewrites and model-owned absent-concept
judgments before the change. Final qualification: 5,985 all-feature workspace tests
passed /25 ignored; 78 standalone unit +2 CLI; 7 Python grading tests; strict workspace
and standalone clippy. Actual CLI mock confirmed one-call preservation/three-call repair and
rejection of invented concept checks. Final code review found no blocking defect;
a minor arbitrary-binary grader protocol-validation gap remains documented in
HANDOFF. New `assessment_calibration_v2.py` imports the unchanged six cases, supports
null revisions and preserves failed rows/nullable usage. Original runner untouched.

A separate Spark installation under `memory-constraints-20261105` replayed the six
cases once with resident `qwen3.6-35b-a3b-uncensored`, seed 7, temperature 0.6. All 6
workflows completed; 5 initial prerequisite judgments matched gold. The previous
passage run completed 5/6 with 4/6 correct. Both adequate plans were preserved in one
call each, and all 59 returned quotes matched their source byte ranges. Total 14
recorded calls, 20,633 input/3,966 output tokens,56.3s summed case runtime; previous
run 16 calls, 17,248 input/4,231 output tokens, 60.2s. Input tokens increased despite
fewer calls. These are descriptive reused-case results with changed prompts,
schema and compute, not a matched causal comparison or fresh generalization test.

Assistant artifact review found better explicit guards and preservation of the
no-clearance-needed branch, alongside unresolved problems. A notification-based wait
introduced an unsupported capability, appropriately left unresolved. A conditional
information request was judged unknown because a successful answer was not assured.
The universal-clearance analogy remained wrong: a shared requirement was treated
as carrier-specific, an alternative's existence/feasibility was inferred from a
universal statement and unrelated episodes, and the revised plan was recommended.
That last endorsement worsened relative to the prior withheld recommendation;
completion and initial-label gains are not uniformly better reasoning. No aggregate
revision-success rate or executed-plan benefit is asserted.

Artifacts: `results/memory-evaluator-constraints-20261105/calibration-seed 7/`, raw
manifest/calls/report plus separate `summary.json` and `review.json`, also copied
beside the Spark prototype. Production stayed `fec8d55c7` with `mecha-serve` active;
no restart, personal-store import, migration, model switch or proposed action.
The user's independent owner-knowledge/agent-memory boundary remains in force.


**2026-11-05 — passage evaluator Spark calibration completed (source `687eaa7ea`).**
After renewed Tailscale authentication, the clean isolated checkout was advanced
and a separate `memory-passages-20261105/bin/mecha` plus deterministic fixture
writer built/installed. The six-case frozen `assessment_calibration.py` run used
resident `qwen3.6-35b-a3b-uncensored`, seed 7, temperature 0.6, no retries, no tools
or proposed-action execution. Production remained `fec8d55c7` with `mecha-serve`
active; no restart, model switch or private-data import. The earlier pending-run
note below describes the preceding authentication block, now resolved.

Five of six workflows completed all 3 calls. Initial prerequisite status was correct
in 4/6 complete-case-denominator trials (4/5 completed workflows); the fifth labelled
a known shared clearance contradiction unknown. The sixth failed after one call
because its factual-support check cited only a provisional concept. Its raw
prerequisite judgment was plausible, but the full invalid response is not a success.
All 54 returned quotation occurrences independently matched source UTF-8 byte ranges.
There were 16 recorded calls with complete usage: 17,248 input and 4,231 output tokens;
60.2s summed case runtime. Correct quotation binding did not establish entailment.

Assistant artifact review against the predeclared rubrics (not blinded, no real
outcomes) found mixed revision quality: a permitted one-step plan gained redundant
checks; a forbidden immediate dispatch gained a verification step but left failure
handling implicit; an unknown-requirement plan became safer yet overrequired
clearance even if it were unnecessary; an already guarded plan stayed equivalent;
the failed case had no revision; and the shared-requirement analogy kept an
unnecessary carrier-change/waiver search despite source evidence. No aggregate
revision-success claim or matched old/new protocol improvement is asserted.
The evaluator also called ordinary words concepts when none was retrieved, claimed
pending clearance was obtainable, and interpreted a contradicted dispatch as an
analogy test. These remain semantic failures, not citation-format failures.

Artifacts: `results/memory-passages-20261105/calibration-seed 7/`, including immutable
raw calls/manifest/report and separate `summary.json` / `review.json`. Remote raw
artifacts remain beside the installed prototype. This supersedes only the pending
calibration status, not the earlier three-case results or their protocols.

**2026-11-05 — host passage citations and frozen evaluator calibration (feature branch).**
`plan_assessment::passage_sources` divides original fields into contiguous UTF-8
passages (at most 1024 bytes, every byte retained) and issues receipt-local IDs.
The model selects IDs only; `finish_plan_assessment` constructs exact source,
field, quote and byte range. Unknown/repeated IDs, old citation objects and empty
passage catalogs fail closed. Complete candidate/check coverage, original evidence
requirements and all-source authorization/freshness checks remain. Tests now cover
Unicode boundaries, host locations and source forgetting during revision and final
assessment. The protocol is `host-passages-v1`; concept generation keeps its prior
independent source-field quotation protocol.

Six fixed synthetic cases in `assessment_calibration.py` distinguish satisfied,
contradicted, unknown and guarded prerequisites, including misleading alternate-
route analogies. A deterministic fixture writer supplies one provisional concept
and its counterexample. Labels and review rubrics never enter evaluator input;
revision self-recommendation is not scored as improvement. Review found nullable
usage being coerced to zero; the fix retains null totals plus explicit known
subtotals and recorded-call coverage. Four Python grading tests, 74 standalone
unit +2 CLI tests and **5,981 all-feature workspace tests /25 ignored** pass.
Strict all-feature clippy and the actual CLI mock passed;
the mock constructs exact quotations across all3 calls and rejects legacy objects.
Spark calibration is pending renewed Tailscale authentication: no real-model
quality result is attributed to this increment yet.

The initial clean baseline hit an unrelated persona-call ENOENT; isolated and full
reruns passed without persona edits. During passage testing an intended empty
query accidentally matched the token “no”; replacing it with a nonsense token
made the empty-catalog regression discriminate the old/new behavior. No earlier
measurement was changed to match these later results.


**2026-11-05 — bounded memory plan evaluator (isolated feature branch).**
`plan_assessment::{prepare_plan_assessment, finish_plan_assessment, evaluate_plans}`
added an advisory model evaluator over the existing scoped memory packet. Each
candidate must receive prerequisite, factual-support and concept-applicability
checks; host validation enforces complete/unique coverage, visible exact quotes,
original evidence for facts, and explicit unresolved questions. Unknown or
contradicted checks cannot yield a host recommendation. One optional revision
uses the same evidence and must be assessed again. All consulted revisions and
scopes are rechecked across inference; reports retain private/untrusted labels.
`memory evaluate-plans --revise` is the thin synthetic-only local-model entry
point, with at most three calls and no action/approval/trusted-rule path. Rejected
model output exits nonzero with an audit of recorded calls. `GoalMemory` now
advertises its one confirmed goal and gives a repair hint while preserving the
mismatch guard. Ordinary GoalContext behavior stays unchanged.

A fresh code review of `641e4ca78..a55ec6b75` found no blocking or minor correction
requests. A manual loopback CLI test exercised all three calls without tools or
live config and verified failure/audit on a malformed response. Targeted tests
observed RED before the goal/schema/validation changes. Final source `8583fad8f`
passed strict all-feature clippy, 5,977 all-feature workspace tests /25 ignored,
and 70 standalone unit +2 CLI tests. The default workspace suite passed 5,892
/25 ignored before the final feature-only field-schema adjustment; no default-
feature source changed in that adjustment.

On Spark, a55ec6b75's first three development cases completed the supported-route
loop but rejected the concept draft on an invalid field and the contradiction
case on slashless field pointers. Host validation was correct; open-ended field
strings made the protocol fragile. Both concept and evaluator response schemas
now enumerate actual visible fields, without enumerating correct evidence or
relaxing cross-source/quote checks. The prerequisite prompt also clarifies that
conditions must be met or guarded, not merely documented. Those changes have
RED/GREEN regressions, and the original failures remain in the artifacts.

The disclosed rerun at 8583fad8f used the same resident
qwen3.6-35b-a3b-uncensored, seed 7, temperature .6 and three development cases.
Supported-route and universal-clearance cases completed three calls and initially
recommended Bridge and waiting, respectively. Neither recommended the unsafe
candidate. Bridge's revision received no recommendation on reassessment; the
waiting revision did, but introduced an arbitrary 24-hour polling interval.
The concept-conditioned case generated its concept successfully, then failed
closed on a quote attached to the wrong source. The model also omitted a required
concept citation and sometimes called known contradictory conditions unknown.
These are concrete remaining quality limits, not evidence of a fully reliable
judge. Model ranking, analogy applicability and semantic entailment are not
proved by the host's structural checks. No actions were executed. Artifacts:
`results/memory-evaluator-20261105/` (both attempts preserved).

A final replay of the two previous runtime goal-ID failures, Cobalt budget and
Vesper current rule, used the repaired GoalMemory schema on the same resident
model. Both memory-on runs chose correctly and cited all required evidence;
both off-arm choices failed. These were reused development cases. The isolated
installed artifact was exercised as `8583fad8f`; production still reported
`fec8d55c7` with an active web service. No production restart/private import or
model switch occurred, and the branch remained unmerged.

**2026-11-05 — provisional concepts and runtime planning trial (isolated feature branch).**
`Memory::prepare_concept`, `retain_concept` and provider-neutral `learn_concept`
added bounded semantic generation with exact source quotations, distinct episode
support, explicit applicability and counterexamples. All consulted evidence,
even uncited input, remains a dependency. Retention rechecks source revision and
scope state after inference; concepts stay provisional and never become trusted
rules. First retention advances the disposable store vocabulary version to 2.
`knowledge_bundle` retrieves concepts with their authorized, fresh source closure
atomically under the existing context budget, including concept vocabulary absent
from episodes. CLI `distill-concept` uses the existing resident-only loopback
transport/holds with no tools. The earlier context budget/nested-reference review
findings and additive `grounded_binding_count` reporting were also completed.

Source `b48e1e84c` passed all-feature lint and workspace tests (5,968 passed,
25 ignored), standalone 62 unit/2 CLI tests, and the new trial isolation regression
passed 2/2 after RED. The first full attempt failed an unrelated persona-chat
fixture with a missing file; isolated and full reruns passed unchanged. Independent
review found one Important issue: inherited MECHA_PROVIDER/MODEL/EFFORT could
change trial settings. The script now strips them and rejects returned model
mismatches before scoring. All 12 original session config records were checked
and used the intended trial provider/model/seed/temperature/budgets.

The six synthetic cases in `planning_trial.py`, source `34a123b5a`, ran both arms
on Spark's resident qwen3.6-35b-a3b-uncensored with seed 7, temperature .6,
7 turns and 8192 output tokens, alternating order and fresh homes. Correct choice:
3/6 off, 5/6 on. Choice plus visible and complete required citations: 1/6 off,
4/6 on, including one correct abstention each. Median wall time: 26.5s off,
15.2s on. Two on-arm calls supplied invented goal IDs, so the tool refused before
retrieval; one model guessed the right choice with invented costs/durations and
citations. These are availability/adapter diagnostics, not matched-information or
matched-compute efficacy results. Free-form plan quality was not graded.

The separately installed concept binary generated a provisional principle from
two supporting episodes and one counterexample in 4.5 seconds. A new-situation
planning smoke retrieved it and all three sources with private/untrusted taint.
Retrieval by its supplied generated name tested wiring, not unprompted discovery.
The plan overgeneralized backup routing to customs release and invented a
deadline: successful provenance does not establish a valid policy. Artifacts:
`results/memory-planning-20261105/`. Production stayed `fec8d55c7` with active
web service, no restart/private import/nightly migration. Next work is bounded
applicability/evidence evaluation, not automatic rule promotion.

**2026-11-05 — read-only situated memory context (local feature branch).**
`Memory::context` assembles a bounded packet from eligible source episodes,
up to two source-selected follow-ups and directly supported temporal relations.
The host owns access, snapshot and source/time restrictions; one read transaction
covers every stage, and relation dependency bundles enter together or not at all.
Exact contiguous quotes from original fields replace model-composed names in
this new runtime path. `Memory::open_read_only` rejects writes structurally.
`GoalMemory` adds packets to `goal_context` only for explicit `mecha run` flags,
with a confirmed goal and synthetic account/domain/scopes. It preserves default
tool behavior and arms private/untrusted taint on actual evidence. It does not
expose internal sensor values or write learned concepts/rules. Source `864c1ece5`
passed strict all-feature lint and a full workspace rerun (5,957 passed, 25 ignored),
plus 51 standalone unit/2 CLI tests and a three-request scripted-provider smoke
through the real CLI. The suite first caught its missing pure-read inventory
entry, which was added after source audit; a later unrelated server-lifecycle
fixture mix-up passed focused and complete reruns. Independent review found no
blocking issues; recall-stage omission reporting and nested reference input
strictness remain minor follow-ups. The existing model-comparison protocol stayed
unchanged and gained the previously missing valid non-null binding routing test.
Artifacts are in `results/memory-context-20261105/`. No Spark deployment, live
model efficacy claim, private-data migration, or nightly activity change belongs
to this increment.

**2026-11-05 — automatic source-grounded name comparison (isolated Spark).**
`binding_evaluation::run` and `evaluate-model --auto-bindings` plan once per case
and compare ignoring versus enforcing the same names under shared filters and
budgets. Names are grounded in cited initial original fields via recall's
unicode61 semantics; invalid plans fail both search arms. Source `83753f9ea`
ran old 12 development and fresh 14 cases without prompt/fixture tuning. Fresh
choices improved 9/14→13/14, complete support 7/12→11/12; oracle 14/14 choices.
One invalid phrase prevented both search answers. Strict support scores 4/14,
7/14,13/14 retain citation-completeness/format failures; one choice improvement
also involved packet order despite equal support. Research §10 records limits,
100 actual calls across both splits and full costs. Verification: 5,950 workspace
passed/25 ignored, 50 standalone, 10 CLI and strict lint/fmt. Fresh review found
no blocking issues; a scripted non-null binding routing regression and explicit
binding-count field remain Minor follow-ups. Installed only under Spark's
`~/.local/share/mecha-prototypes/memory-bindings-20261105/`; production
`fec8d55c7` and active service verified afterward. No private import, nightly
migration or runtime appraisal integration.


**2026-11-05 — required-name retrieval diagnostic (local feature branch).**
`Memory::recall_mention` adds a bounded literal phrase gate before top-k selection,
using a non-stemming eligible-only index and the shared access/time/context path.
CLI `evaluate-mentions` replays 12 frozen searches with disclosed manual name
bindings, with no model/config/network setup. The original arm reproduced all 12
prior evidence packets exactly. Complete support increased 6/10→10/10 answerable
cases (changed-goal cases 2/6→6/6), with the same three searches and four-record/
8,000-byte cap. These are already-seen development cases and supplied bindings;
no new reasoning score or automatic entity-resolution claim. Review caught a
Unicode combining-mark mismatch, regraded as a contract defect and repaired with
a failing-then-passing regression; raw phrases are now safely quoted before FTS
normalization. The earlier duplicate-valid-citation test gap is also closed.
All-feature workspace 5,944 passed/25 ignored, standalone 45, main CLI 9, strict
Clippy and both format checks passed. Research §9 records costs and limitations. No Spark install, production
service, private store, nightly job or runtime appraisal changed.


**2026-11-05 — evidence-informed memory search and citation binding (isolated
Spark prototype).** `memory::model_evaluation::followup::run` and
`evaluate-model --follow-evidence [--development]` add matched bounded planners,
shared anchor-first fusion and visible-ID response schemas; legacy behavior
remains selectable. Development uses the old 16 cases; the frozen holdout adds
12 cases in six fresh session families. Source `27e15d8a2` completed both runs
without transport/parse failures or invented/duplicate citation IDs. Held-out
question-only/evidence-informed/oracle choices were 8/12, 8/12, 12/12; strict
grounding 3/12, 5/12, 10/12. Named-alternative searches still missed property
records, and strict support/abstention formatting explains some score losses.
Research §8 retains full conditions, costs and limitations; no prompts or gold
were tuned after inference. All-feature workspace 5,937 passed/25 ignored,
standalone 39, CLI 8 and strict lint/format checks passed. Fresh review had no
Critical/Important findings; a dedicated duplicate-valid-citation regression is
a deferred Minor (the host already rejects duplicates). The separate installed
binary is under `~/.local/share/mecha-prototypes/memory-hop-20261105/`.
Production `fec8d55c7` and active web service were checked after the run; no live
store, nightly job or production deployment changed.

**2026-11-04 — synthetic local-model memory comparison (isolated Spark
prototype).** `memory::model_evaluation::run` and CLI `evaluate-model` add
lexical, bounded goal-aware and oracle conditions, explicit local inference,
shared evidence budgets, revision citations, deterministic choice/support
checks and per-call usage/latency/error records. Fresh review corrected two gold
fixture premises, rejected extra irrelevant citations and closed a redirect/
proxy escape with opt-in strict local provider/probe clients; all Important
findings have regression coverage. The 16-case first run selected 7/16 correct
choices in both retrieval arms, 16/16 with oracle evidence; strict grounding
was 5/16, 5/16, 11/16 respectively. Citation-format errors explain all five oracle
failures. Expanded searches spent 32 calls versus 16 without increasing complete
support retrieval. Research §7 records the frozen results and next experiment;
no prompt or fixture was tuned after viewing model answers. All-feature workspace
verification passed 5,930 tests (25 ignored), standalone 33, CLI 7 and strict
Clippy/format checks. Initial pre-change default-workspace web lifecycle timing
failures passed serially; the full default baseline passed 5,891/25 ignored.
Source `d3ca03b82` was installed only in Spark's new prototype path. Production
`fec8d55c7` and active web service were rechecked after inference; no production
stores or nightly jobs changed. One review minor remains: partial usage fields
can inherit a zero from the existing provider decoder; this limitation is
explicit in the report and README.


**2026-11-04 — synthetic episode capture and retrieval baseline (local feature
branch, not production).** `Memory::import_session_jsonl` captures exact native
session records atomically with stable source identities, no-op retries and
append-only prefix checks. Missing message timestamps remain unknown; raw
compaction/future records survive without being indexed as original speech.
`Memory::recall` returns eligible revision-backed anchors and bounded neighbors.
The main CLI adds init, import-session, recall, forget-session-record and
evaluate; the latter runs 32 synthetic evidence cases with baseline and oracle
bundles, explicitly without a reasoning score. A fresh review reproduced an
important ranking defect: global FTS statistics changed an old snapshot's
top-k after inaccessible future data arrived. The regression failed before
the fix and passed with eligible-corpus scoring in a disposable in-memory FTS
index. That correctness choice carries corpus-sized query work; large-corpus
performance remains unqualified. Source JSONL, prior outputs and backups remain
outside local forgetting. No live ingestion, nightlies or Spark service changed. Final verification: 5,920
all-feature workspace tests passed (25 ignored), default workspace 5,891
passed (25 ignored), standalone harness 26 passed, and formatting/Clippy
checks passed. Intermediate reruns hit existing port-binding and watchdog
startup races; isolated retries and the final full run passed.

**2026-11-04 — synthetic memory operations in the main CLI (feature branch
and isolated Spark test install, not a production rollout).** `commands::memory::execute` now exposes
demo, inspect, query and forget through the default-off `memory-prototype`
Cargo feature. Both frontends use `memory::fixture`; the real-binary tests
exercise historical/valid-time queries, transitive forgetting, explicit opt-in,
foreign-file refusal and independence from malformed global/project config.
`Command::is_one_run` and `runs_a_model` explicitly exclude memory operations.
Fetching the existing lockfile resolved the missing Monty dependency without
version changes. The pre-change full workspace build and permission-enabled
tests passed; sandbox-only tests had failed on denied local sockets. The final
all-feature workspace suite, default-build command-absence test,
all-feature Clippy (`-D warnings`), standalone harness and format checks also
passed (5,907 workspace tests passed, 25 ignored). Spark built `2b00d5f62` with
four jobs and dev-profile debug info disabled: three main-CLI and 15 standalone
harness tests passed on Linux. The separately installed binary's checksum
matched the tested build, and its temporal query probes returned zero current
July matches versus one at knowledge sequence 4. The live web service remained
active. Nightly migration and private-data qualification remain open in HANDOFF.

**2026-10-03 — a persona on a call: no dangling last word, and thinking
capped at one draft (#538, #541; mecha-1e).** After an afternoon of persona
calls the owner reported replies "ending on 'I' or talking too long or too
short", and asked "why is it so slow to start talking sometimes and fast at
other times?". Merged as `eb37184c` and `2b3a633e`, installed from
`2b3a633e` at 23:41:43Z (serve 23:41:50Z through `serve_held`), then carried
by mecha-69's `6816c2bd` install at 23:51Z. Probe: `strings
~/.cargo/bin/mecha | grep -c reasoning_budget_tokens` → 1, and 0 before.
- **A persona copied its own cut-off replies (#538).** An interrupted reply
  is stored as heard, mid-sentence. The model then ended replies "…\n\nI"
  on its own, and each one was another example: 41 of 75 earlier replies by
  the end of one chat (17:14–19:20Z), every one recorded
  `stop_cause: completed`. Replayed against the loaded model on three of
  those replies, 4 samples each, the stored history stopped a reply
  mid-sentence 7 of 12 times against 1 of 12 cut back. The effect tracked
  how much mid-sentence history there was: with 8 such earlier replies, 0
  of 4 either way; with 14, 4 of 4 against 0 of 4; with 22, 3 of 4
  against 1 of 4. `message::PriorTails::Trim`, set in
  `setup::persona_agent` beside `PriorThinking::Drop`, cuts an earlier
  plain reply that ends mid-clause back to its last whole sentence
  (`dangling_tail`; `abbreviation` keeps "Dr." and "e.g." from counting as
  sentence ends). It changes what is sent; the transcript keeps every reply.
- **The long thinking was the reply, re-drafted (#541).** A spoken reply
  waited 47–50 s whenever thinking ran to the router's `--reasoning-budget`
  4096: one sentence of the draft repeated up to 23 times, with the fresh
  material ending 11–16% of the way in. Otherwise replies started in a
  median 1.9 s. Replayed on 9 turns, 2 samples per arm, and judged blind by
  the same model with thinking off (36 judgements per comparison, each pair
  twice with the order swapped): thinking off lost to the full budget 12–17
  (7 ties); a 512-token cap lost 10–20 (6 ties) and stopped replies
  mid-sentence three times as often; a 1024-token cap won 24–10 (2 ties),
  slowest 12.5 s against 41.3 s. One chat, self-judged, and the full arm's
  own showing varied between runs: read it as "1024 is at least as good and
  far faster", not as a ranking. `CompletionRequest::think_budget` is sent as llama-server's
  `reasoning_budget_tokens`, only to `OpenAiCompatible::local` (`kind =
  "local"` with a base URL: the dialect, not the address).
  `RunContext::think_budget` is set to `SPOKEN_THINK_BUDGET` (1024) on a
  spoken persona turn and reaches the forced final turn. Typed turns keep
  4096; the owner: "start with call turns first and then consider it
  later".
- Review passes: #538 two (an abbreviation was taken for a sentence end),
  #541 four (the forced final turn dropped the cap; the field reached any
  OpenAI-compatible endpoint, where it is a 400; the first gate tested the
  address).

**2026-10-03 — Listen is directed once per reply (#539, mecha-69).** Merged
as `6816c2bd`, deployed 23:52Z. A Listen tap (`commands/serve/listen.rs`)
asks the director once per reply through `voice::ask_director_on` with
`Cue::Reply`, and the line rides every piece of the reply as
`instructions`. It reuses an earlier tap's line (turn prefix
`voice_direction::LISTEN_TURN`) or a call turn's first line
(`Session::spoken_directions`), and records one `spoken_direction` per reply
(incognito: none). Serve asks the worker `GET /mecha/directs`
(`worker_directs`) once per reply, and a worker without the route speaks
undirected.

**2026-10-03 — Chrome calls keep the buffered microphone path (#534,
mecha-69).** Merged as `6c9e9169`. The web dist went live at about 14:55Z
with no restart, and again inside #539's dist at 23:52Z. The cause is the
Chromium trap under Environment.

**2026-10-03 — Breeze: no speech invented from punctuation, and a director
that keeps one voice (#531, #532; mecha-1e).** The owner's reports: "i'm
getting occasional intrusions completely unlreated to the story sometimes
not the same language and isn't in text transcript", and the expressiveness
"often sounding like a different person" line to line.
- **#531 (`8a26e142`):** where `breeze_server.speakable` is false (no
  letter or digit), `speech` plays `PAUSE_SECONDS` / speed of silence and
  never asks the engine. Deployed 16:31:27Z, when the shared checkout moved
  `27156c9e` → `57424b7f` on the owner's word and `mecha-breeze-adapter`
  restarted. Probe: a `pcm` request for `"."` to :8887 returns 14400 bytes.
- **#532 (`ebcfcf19`):** `voice_direction::SYSTEM` directs emotion, energy,
  pace, emphasis and a balanced choice of texture; pitch may move within a
  line, never a level, register, age or accent; change by degrees.
  `voice_direction::ANCHOR` ("In your own natural voice: ") is added in code
  and never shown back to the director, and `Directions::carried` seeds a
  reply with the last one's direction. With the real director over three
  logged replies: pitch levels named 9 of 24 → 0 of 24, and the mean pitch
  move between lines 2.8 → 1.6 semitones (max 18.0 → 5.3). Installed
  14:08:53Z, serve restarted 14:09:26Z.

**2026-10-03 — real names out of the tree (#536, #537, #540, #544; mecha-5d).**
On the owner's rulings to scrub real people, the institution and the
owner's addresses.
- **#536 (`9a2b2ccd`):** the owner's family's names, the owner's own name
  as fixture data, the lab and its package, a handle, and two graph contacts
  with full names, from the TUI, outbox, gossip and corroborate fixtures,
  the docs demo and a published docs page. The owner fixture is now Robin
  (they/them), in `web/src/demo/fixtures.js`'s cast block.
- **#537 (`c7be1610`):** the institution and the owner's addresses. The mail
  account is `campus` in fixtures and docs, and addresses use example.edu.
  `mecha mail score`/`eval` default to the account mecha-mail sends from
  (`commands::mail::measured_account`) rather than a compiled-in name.
  `scripts/mecha-slots.service` ships `--account YOUR-MAIL-ACCOUNT`, so it
  must never be copied over the installed unit, which, with the live
  account, tokens and stores, keeps the real name.
- **#540 (`58c4bf28`):** a real correspondent and a real event in
  `web/test/outbox-view.mjs`.
- **#544 (`412cc3bf`):** the package name the first check missed (it was
  case-sensitive), and the owner's real Drive items (a course, a committee
  folder, a spreadsheet, a folder), from `docs/DOCS-RESEARCH.md` and a
  `tui/docs.rs` fixture. After it, the scrub's case-insensitive check over
  `main` is empty (run by mecha-5d and mecha-1e). The pattern is kept out of
  this file on purpose: it spells the names, so written here it would match
  itself. The names stay in git history and in crates.io tarballs.

**2026-10-03 — Breeze TTS 2 is the voice: an adapter, a director per
sentence, no brevity rules on a streaming engine, and `default.wav`
(#523, #525, #527, #528, #529; mecha-69).** Live with mecha-69's
`ed72a382` (mecha 04:56Z, serve 04:59:03Z after a persona-chat hold
cleared). `docs/VOICE-BREEZE-DESIGN.md` holds the measurements and the
owner's rulings (§6).
- **The engine and the adapter (#523).** `mecha-breeze-tts` (:8886, a
  qwentts.cpp fork in `~/models/breeze-qwentts`, Q6_K) and
  `mecha-breeze-adapter` (:8887, `scripts/voice/breeze_server.py`, the
  surface `chatterbox_server.py` serves). Both are enabled at boot. The
  adapter transcribes each clip once with Parakeet into an editable `.txt`,
  registers voices and re-registers when the engine's registry no longer
  holds one, and refuses `exaggeration`/`cfg_weight`. Probe:
  `curl -s :8887/v1/voices` lists `"streams": true`.
- **One write per session record (#525).** `Session::append` writes the
  record and its newline in one `write_all`. `writeln!` issued two writes,
  and a concurrent writer could merge two records into one line, losing
  both. Eight threads × 300 appends merged lines on 3 of 3 runs before.
- **The director (#527).** `POST /v1/mecha-direct` on the facade
  (`voice::direct`, `mecha_core::voice_direction`) writes a 12–25-word
  delivery direction per sentence on the loaded model, with thinking off
  (`CompletionRequest::think`). The opening direction starts from the
  owner's words when the turn begins. Each direction is a
  `spoken_direction` record in the session; in incognito the director
  runs and nothing is kept. A single-slot router preset skips
  (`Bound::slots`). The worker asks only when the TTS lists `instructions`.
  The first live call (a persona, 04:59Z): 23 of 26 sentences directed, 3
  skipped as harness, median 723 ms, max 1,269 ms against a 2,500 ms
  deadline.
- **Brevity off on a streaming engine (#527).** The worker sends
  `X-Voice-TTS-Streams: 1` when `/v1/voices` lists `"streams": true`, and
  the facade opens the turn with `VOICE_BLOCK_STREAMING` and the persona
  note without its length rule. Without the header, `VOICE_BLOCK` is
  byte-identical, pinned by digest. Probe: `strings ~/.cargo/bin/mecha |
  grep -c "Speak the way you would out loud"` → at least 1, and 0 before.
  The phrase is in `VOICE_BLOCK_STREAMING` only; this build printed 2, and
  what emitted the second copy was not run down.
- **Breeze is the default (#528).** `worker.py`'s `MECHA_VOICE_TTS`
  defaults to :8887. `default` is the clip `default.wav`, a reserved name
  the library never offers to delete. `make-voices.py` and
  `add-vctk-voices.py` write exact transcripts, and the latter skips
  VCTK's duplicate mic rows.
- **The ComfyUI idle reset, bounded (#529).** Its journal reads are
  `journalctl -q -n 1 -g`, and an unreadable `MemFree` fails the unit.
  `mecha-comfyui-idle-reset.timer` was installed at 04:57Z; its first tick
  saw a queued job and left ComfyUI alone.

**2026-10-03 — modular installs, step 7a-1: one hub resolver and a
download that keeps only what matches its pin (#521, mecha-a3).**
`mecha_core::fetch` is installed with mecha-69's `ed72a382` (04:57Z) and
mecha-5d's `27156c9e` (13:35Z), and is behaviour-neutral here: no `HF_*` or
`XDG_CACHE_HOME` is set on any running server.
- One resolver, in this order: `HF_HUB` → `HF_HUB_CACHE` → `HF_HOME/hub` →
  `XDG_CACHE_HOME/huggingface/hub` → `~/.cache/huggingface/hub` (empty
  variables skipped). The seven model launchers and
  `scripts/layout/install.sh` share it: `grep -rlF
  'XDG_CACHE_HOME:-$HOME/.cache}/huggingface' scripts | wc -l` → 8.
- `fetch_hub_file` resumes and refuses: a `.part` file plus `Range`; a 206
  at the wrong offset is refused, as are more bytes than the pin; only a
  sha256 match is renamed into `blobs/<sha256>`.

**2026-10-03 — a persona chat records the model it ran on (#530).** Found
by mecha-05: a chat moved by the router to another model mid-way stayed
credited to the model it started on, because persona chats wrote no
`config`, so the memory writer waited for that model, possibly for good.
- Persona chats now record a `config` on their first turn in a process and
  at each switch (`PersonaSession::recorded_generation`, the assistant
  chat's rule), and `writer::read_chat` takes the latest.
- The record describes the persona agent, not the install's assistant:
  `persona::agent::levers_off` (the prompt blocks it replaces, and the
  hooks, approval rules, outbox, mailbox and fallback it is built without,
  are always off), `permission_mode = read-only`, and
  `RulesCarried::none()`.
- Three workflow review passes, counted from the PR's comments; passes 1
  and 2 each found the record claiming the assistant's settings.
  Installed 13:35:54Z the same day.

**2026-10-02/03 — a persona no longer sends an earlier reply again (#517,
#522).** The owner's report: late in a long persona chat, short turns
("mm") brought back an earlier reply word for word. A replay of that
chat's prefix against the router found two causes and ruled out the usual
fix (`PERSONA-DESIGN.md` §12.7 has the owner's rulings).
- **The causes, measured.** With the provider's `seed = 42` and earlier
  thinking sent back, the 4-character turn reproduced the earlier reply
  7 of 7 times on the base model and 4 of 4 on the uncensored one. With
  neither, 0 of 8 copied and 0 of 8 were damaged.
- **Rejected options.** Qwen's "general" profile (presence penalty 1.5)
  still copied 4 of 4: the penalty's window is 64 tokens (Traps →
  Providers). DRY over the whole context ended the copies but damaged 2–5
  replies in 8 at every setting tried.
- **What shipped, persona chats only:**
  - **No seed for the chat.** `setup::persona_provider` takes a
    `PersonaUse`. `Converse` drops the seed, and `Judge` keeps it for the
    crisis and steering judges, whose verdicts should be reproducible.
    `build_for` checks the config as written first, so a config the
    provider refuses stops chat and judge together (#522).
  - **No earlier reply's thinking on the wire.** `message::PriorThinking`
    is set to `Drop` by `persona_agent`. `drops_thinking` keeps the
    thinking of any turn that called a tool, after review found stripping
    it re-created the 2026-08-10 bare-`<tool_call>` failure. Every
    pressure reading goes through `Agent::wire_bytes`. The assistant's
    runs are unchanged.
  - **An offline repetition score.** `persona::echo::session_echoes` reads
    the transcripts and only `mecha persona show` asks for it, by the
    owner's choice of offline over per-turn: nothing acts on it live, and
    a better metric can recompute the history.
- **Seven review passes on #517 and two on #522**, counted from the
  reviewer's summary comments on each PR. Deployed by mecha-1e at `c6f59b05`, 2026-10-03 02:36Z, with
  #520.
- **First reading on the live store:** 8 of 92 replies in 6 persona chats
  over the week before repeated an earlier one, mostly from before the
  fix.

**2026-10-02/03 — modular installs, step 6: recommendations as data, and
installers designed (#512, #513, #520; mecha-a3).**
- **#512 (`5587bded`), step 6a.**
  - `hardware.md` has two columns per tier (unified memory, and a separate
    GPU) and a "Beside the chat model" table.
  - The graph page names harrier on `:8081`, not ollama.
  - FEATURES-DESIGN §6 had said image generation "borrows ~15 GB for its
    duration"; it now says on demand, with a leak.
- **#513 (`7c3eb2c2`), FEATURES-DESIGN §10: installers in the binary.** The
  owner's rulings F7–F10:
  - enabling a feature offers its install;
  - hand installs are provided and left alone;
  - llama.cpp is prebuilt where possible and built otherwise;
  - an unpinned engine tag is trusted only when the owner confirms it at a
    terminal.

  It also corrected LLAMA-SERVER.md's rollback. The stub's RUNPATH is
  `~/llama.cpp-next/build/bin`, so a rollback is chosen by whether that tree
  has `build/bin.prev`.
- **#520 (`3eeb4622`), step 6b.**
  - `mecha-core/src/recommend.rs` is a registry by model slot, with pinned
    sha256s.
  - `mecha features --probe` reads the machine.
  - `hardware.md`'s table is generated from the registry, held by
    `the_hardware_page_holds_the_registry_table`.
  - Live in mecha-1e's `c6f59b05`; mecha-a3 read 53.2 GiB resident, 83.1
    loaded, of 121.7.

**2026-10-02 — persona memory: dated by when it was said, a curation page,
and the timescale rulings (#514, #519, #518; mecha-5d).** #514 and #519
are live from mecha-5d's `c08f7f21` (2026-10-02 22:03Z). Three review
passes each.
- **#514 (`b44f0659`): a memory is dated by the chat it came from**, in
  `[agent] timezone`, not by the night it was written. Before, the first
  night showed a passing state from 30 September as that morning's news, and a UTC day
  put the owner's late-evening chats a day ahead.
  - `Source::chat_began` reads the date from the session id.
  - `said_at` (on `Fact`, `SharedFact` and `Episode`) chooses `valid_from`,
    else the episode's span, else the chat's start, else the write time.
  - `recall::local_day` renders it in the owner's zone.
  - `Memory::facts` and `Memory::episodes` sort by it.
- **#519 (`9f7507f9`): the persona editor's Memories tab** (§9.8).
  - `PersonaMemory.svelte`, over `PersonaChats::memory` and `memory_act`.
  - The acts are the closed `MemoryAct` set.
  - A locked persona answers 404.
  - Only this persona's own shared copies can be shown or unshared.
  - `shared_unreadable` and `shared_problem` are said on the page, never
    shown as "not shared".
- **Before the first night, a live run of the writer** on a copy of one
  real chat (2026-10-01, the resident uncensored model, 24 turns) took
  ~63 s and wrote 2 transient facts out of 8. That is the writer's cost
  figure, and the baseline schema v4's prompt rewrite is to be A/B'd
  against.
- **#518 (`c0f47bf6`), docs only:** `PERSONA-MEMORY-TIMESCALES-RESEARCH.md`
  and PERSONA-DESIGN §9.13, with the owner's rulings D26–D28.
  - D26: four tiers, from momentary to lasting fact. States expire from
    recall, never from the store, and a mood label about the owner is
    allowed as a short-term state, amending §9.5.
  - D27: a fact derived from several chats is rebuilt from the rest when
    one is forgotten.
  - D28: every memory access and curation act is logged, record-only, and
    erased by `forget`.

**2026-10-02 — modular installs, step 5: `[voice]` and `[personas]`
(#503).** Everything voice is in one operator-only table, by the owner's
ruling of 2026-10-01: "[voice] holds everything voice".
- **`VoiceConfig`** has `stt_url`, `offer_target`, `voice_port` and
  `voices_dir`, with `DEFAULT_STT_URL`, `DEFAULT_OFFER_TARGET` and
  `DEFAULT_VOICE_PORT`. `VoiceConfig::validate`, run from
  `Config::validate`, refuses a non-loopback address as a load error.
  Serve's `VoiceWiring::of` lets the flags override the table and checks
  `--offer-target` the same way.
- **`[web] voices_dir`** still works for one release through
  `WebLayer::voices_dir_moved`, which warns once per process.
- **`PersonasConfig::crisis_cooldown`** makes the window after a crisis
  pause configurable (`crisis_cooldown_minutes`, default
  `persona::safety::CRISIS_COOLDOWN`), by the owner's ruling: "Fully
  configurable". The four cooldown checks in `persona_chat` read it.
- **Both tables are operator-only:** `merge_file` strips them from project
  layers, and `trial_env::OPERATOR_ONLY_TABLES` lists them.
  `operator_only_tables_are_what_a_project_layer_is_stripped_of` reads the
  strips out of `config.rs` and requires the two lists to agree.
- **The features rows:** Dictate names `stt_url`; Calls stays on when
  `offer_target` is empty (#503 first worded it "no proxy"); Cloning reads
  `[voice] voices_dir`. The sentinel was not settled everywhere: three other
  readers took an absent target as "voice calls are not wired on this
  serve". They were `settings::speak` (Listen), `library_voice_sample`, and
  `library_voices`' worker list. HANDOFF carried it; found by #506's review,
  by the rule of the sentinel trap below. Settled the same day by the
  owner's choice, "Say it honestly", over splitting the address in two:
  empty means `mecha serve` reaches no voice worker, and the Calls row, the
  three refusals and the docs now say so (no call relay, no Listen, no voice
  previews).
- **Seven review passes,** counted from the PR's comments. Pass 3 found the
  empty-`offer_target` sentinel unmounting the voice facade (Traps →
  Review process). mecha-d7 merged it at mecha-ce's named head and
  deployed it the same night.

**2026-10-02 — typing into a call (#499), and Listen: a play button on
each reply (#502).** The owner's read-aloud ask of 2026-10-01: type and
hear replies spoken. Both work in the assistant's chat and persona chats,
with no speak-replies switch, by the owner's choice.
- **Typing into a call (#499):** a text box in both call overlays. A line
  goes over the call's data channel (`{"t": "typed"}`, voice-core's
  `sendText`), and the worker's `typed_turn` makes it a user turn on the
  same context as speech (`LLMMessagesAppendFrame`). So it sits behind the
  hosted door, the persona lock and the crisis layer by construction. The
  channel's comment was widened to name the one field that reaches the
  agent.
- **One consumer for every turn outside live audio** (`TypedTurns`, VOICE-LINK-DESIGN
  §2.5). Typed lines and late spans are delivered in order, never over an
  answer under way:
  - `answer_pending` counts the model answering, the bot speaking, and the
    `TYPED_SETTLE_SECS` gap before a short answer's first audio, stamped
    where the answer ends (`Answering.ended_at`);
  - a late turn goes first, and its `deliver` future resolves at the send,
    so the live audio queued behind its flush follows it, as §2.5 requires;
  - a turn just pushed whose answer has not begun still counts as in the
    way (`_unstarted_until`).
- **The mic pauses while the box has focus**, derived in one place
  (`applyMic`: `!muted && !typing`) and re-applied after every
  `connect()`. `web/test/call-mic.mjs` drives it on both panes.
- **Listen (#502):** beside Copy and Download, a reply is read aloud a
  sentence at a time (`reply-player.svelte.js`), with the next piece
  fetched while one plays. It goes through `POST /api/speak`
  (`settings::speak`, `Owner::Of(Feature::Calls)`) to the worker's
  `/mecha/speak` (`speak_request`), which logs no text.
  - The page tidies the reply for speech (`speech.js`'s `speakable`): no
    marks, no URLs, a code block named rather than read, and a persona's
    citation spoken as "from <file>".
  - A persona chat is spoken in the persona's voice and rate, behind its
    lock (`check_call`, `call_voice`), whatever the page sends. The
    assistant's replies use the voice and rate saved in Settings → Voice,
    as an assistant call does.
- **What the loops found:** nine passes on #499 and three on #502, counted
  from the PRs' comments. On #499, pass 5 joined late turns to the typed
  queue, and pass 6 found that this broke §2.5's ordering: the late
  turn's flush returned at once, so live audio overtook it. The test that
  pinned the ordering still passed, because its fake left the new hook
  unset (Traps → Review process). On #502, pass 2 found Listen ignoring
  the owner's saved speed.

**2026-10-02 — a files-only persona reads the words, not the tool output
(#505).** The owner's ruling C on #498's `answers = "files"` exemption,
2026-10-02: "lets go with C". The memory tools stay offered. A files-only
persona's `memory_read` returns only the owner's and the persona's words.
Tool results read as `LEFT_OUT` where they stood, and the tool calls and
the harness's folded blocks (`agent::is_harness_voice`) are dropped
(`memory_tools::words_only`).
- `read_episode` now cuts once, keeping the end:
  `writer::render` was split into `writer::named` and `writer::bound`, and
  a long conversation keeps `READ_HEAD_CHARS` (2000) of its opening and
  the rest of `MAX_READ_CHARS` (8000) from its end.
  `a_long_conversation_is_cut_once_and_keeps_its_end` fails on the old
  double cut.
- **Not a seal:** summaries still paraphrase tool output (HANDOFF carries
  it).
- Two review passes, counted from the PR's comments. Pass 1's medium finding
  narrowed the code comment to what C actually guarantees.

**2026-10-02 — a persona can search its memory and open a past
conversation (#498).** PERSONA-DESIGN §9.7's `recall` and `recall_open`,
named `memory_search` and `memory_read` (`persona::memory_tools`). Owner
approval 2026-10-01: offer them whenever memory is on, and give episodes an
id in the recall blocks.
- `memory_search` runs `Memory::recall_search`, the same fusion per-turn
  recall uses, and shares its `[memory]` switches through
  `memory_tools::kinds`.
- `memory_read` opens the turns an episode's `source` names, by id only.
  The chat name comes from the stored record and `is_chat_id` refuses
  anything else before a path is joined. A conversation any of whose turns
  is untrusted or unreadable comes back `.from_outside()`.
- Both tools are `Egress::None`, are offered from the pinned settings so the
  tool list cannot change mid-chat, and read the switches live on each call.
- Two workflow review passes, counted from the PR's comments. Pass 1 caught
  a description naming `memory_read` in a chat that had not been given it.
  Pass 2 raised the `answers = "files"` exemption, ruled C by the owner
  and built in #505 (above).

**2026-10-01 — calling a persona (#483), and a voice library (#490).**
- **Calls (#483):** a call into a persona chat is answered by the persona
  (`PersonaChats::speak`), through the typed turn's own door, so the lock,
  the crisis layer, the dose meter and the record are one code path. A
  spoken turn never steers; it barges in after the lock check.
- **The call note** is a harness-voice block (`persona::call`), re-armed
  after a failed turn, a compaction or a typed steer.
- **The crisis pause** is spoken: as text deltas on the tap, since a
  streaming call never speaks `HostedAnswer.text`.
- **Serve vouches at the offer** (`persona_offer`): it checks and strips the
  unlock token, resolves the voice itself, and binds last, with an id that
  only that call's hang-up can release.
- **The page:** a call button beside send (`PersonaCall.svelte`), and call
  minutes in their own `calls.jsonl`, never `dose.jsonl`, which an older
  binary would read as turns.
- **The library (#490):** Library → Voices lists every voice the worker
  speaks, with a spoken sample (`/mecha/sample`, a fixed sentence), clone
  status, and who speaks in each, behind the lock. Recording moved there
  from Settings.
- **A persona names a library voice directly** (`voice` plus an optional
  `voice_speed`, `persona::VOICE_SPEED`), and the `voices/` profiles are
  gone, by the owner's rulings.
- **What the loops found:** eight passes on #483 and seven on #490. #483's
  first pass found the pause was silent on a real call, because the tests
  read the answer field the streaming path discards. Most later findings
  were lifecycle: a binding released by the wrong call, a hang-up from a
  closing tab, a refused offer unbinding a live call.

**2026-10-01 — the persona page: tap the picture to frame it, and the lock
moves into settings (#491, #497).** The ⋯ menu went. The hero picture opens
framing, and a new portrait opens it at the default: the server clears a
stale frame when the character changes (`write_owner_file`) or its picture
is replaced (`clear_frames_for`, from `imagelib update`). Locking is a
switch at the top of the Settings tab. It waits while any tab holds unsaved
edits (`persona.js` `lockWaits`), since locking without the unlock closes
the editor.

**2026-10-01 — the macOS `serve_lifecycle` flake was a port reuse (#492).**
Four test sites reserved two ports one at a time. macOS hands a just-dropped
port straight back, so both got 49287 and serve bound one address twice.
Both listeners are now held until both ports are read.

**2026-10-01 — personas remember: a store, a nightly writer, and recall at
chat start and on every turn (#462, #463, #468, #477, #481).**
PERSONA-DESIGN §9 / §17 step 5, each through its own review loop.
- **#462 (D25):** break reminders and the farewell check were dropped
  before either was built.
  - The dose meters already show time spent. The farewell evidence (HBS
    WP 26-005) is about engagement-tuned apps, and the one goodbye in the
    owner's persona chats got a clean reply.
  - `Safety` still accepts `breaks` and `farewell`, so old files load.
    `retired_keys` names them in the persona's notes.
- **#463, the store:** `persona::memory`, one `memory.db` per persona plus
  `shared.db`.
  - `source` is mandatory: it is the deletion key, enforced by a schema
    `CHECK`, and `forget_chat` clears both files.
  - Forget means it: `secure_delete`, then `scrub` reads `busy` from the WAL
    checkpoint. A blocked checkpoint is reported, never passed.
  - `Status::initial` keeps an untrusted record a candidate until the owner
    approves it.
  - `mecha persona memory show|…|unshare` is the owner's door.
- **#468, the writer:** `mecha persona memory write`, nightly from
  `scripts/ruminate.sh`.
  - A turn is the n-th `message` record in the file, so a compaction moves
    no address.
  - Provenance splits where taint first rose, and the clean stretch's
    request never sees the untrusted one.
  - An untrusted stretch only adds, as candidates.
  - `Memory::write_stretch` commits the records and the ledger together.
  - Test chats are left out (`pending_chats`), and the provider must pass
    `provider_is_local`.
  - Owner ruling 2026-10-01: a chat is written by the model it ran on, and
    only while that model is resident (`pick_model`), so the 2026-09-27
    no-swap rule holds.
- **#477, recall at chat start:** `persona::recall::chat_start` folds about-me,
  facts and recent episodes into the first turn beside the files block.
  - It opens with one of **two stems** (`MEMORY_STEM`,
    `UNTRUSTED_MEMORY_STEM`). This deviates from §9.7's single stem:
    `arm_for_content` re-reads the conversation at every run start, so one
    stem arming both would have made every chat that remembered anything
    untrusted.
  - The budget is split so no section starves another out of sight, and
    every cut is said.
- **#481, recall on every turn:** past the first reply, the owner's message
  keys `Memory::recall_search`, and up to three records the chat does not
  already hold ride with that turn under the same two stems
  (`recall::per_turn`).
  - Words (FTS5), meaning (vectors, `embed::Task::Recall`) and recency are
    fused by reciprocal rank; ties go to the newer record.
  - `STOPWORDS` stops a common word from being the only match. That
    matters because recalling an approved record from outside arms the
    chat for good.
  - `recall::would_search` decides before any embed, so a turn that won't
    search waits on nothing.
  - Schema v3 builds the index under the write lock, idempotently.
  - FTS5 is created with `secure-delete`: without it, a forgotten term
    survived in `memory.db` (mutation-checked against the file's bytes).

The review loops (1, 3, 6, 4 and 5 workflow review passes, counted from each
PR's comments) kept finding one shape: a path that
lost or kept something without saying so. A blocked log truncation passed
as done; a shared copy outlived its correction; an unanswered turn was
skipped for good; a section was cut to nothing with its heading gone. Each
was fixed with a test that fails on the old code, and the key ones were
mutation-checked.

**2026-10-01 — personas from the main chat, and pictures you can save
(#493, #494, #495, #496).** The owner's asks, on the rulings of that day
(Waiting section on the Personas page; the chat sets name, display,
relationships, identity, motivation, character and voice only; revise until
approved; a waiting portrait approved in the same tap; either mode, staged
locked from incognito).
- **`persona_propose` (#493):** `persona::propose` stages a candidate with
  the model's own prose (`create_with`), or revises one a proposal wrote.
  `State::proposed` records the digest the model last wrote; one owner edit
  (to the files, or a template they name) and the persona is the owner's,
  and a revision is refused. A revision is a patch in the tool (left-out
  fields keep their values); the lock only rises; origin is the less
  trusted. `ToolCtx::stage_locked`, stamped from the chat's room, stages an
  incognito proposal locked — the one writer incognito allows outside the
  room (INCOGNITO-DESIGN R3's exception). Approval reads
  `/api/personas/{name}/review` with a server signature of the content
  digest and approves via `persona::approve_as_shown`, a waiting character
  with it or not at all; review/approve/reject act only on a candidate a
  proposal wrote, never a hand-made folder.
- **A proposal's voice is a name (#496):** never looked up (voices live on
  the voice server since #490), held to `validate_name` so a control
  character is refused before a write, and kept out of the store probe whose
  message would have handed the model the store's path.
- **Picture Download (#494, #495):** beside Edit in both chats, read through
  the chat's own file route and saved from a blob, so a locked or incognito
  chat's picture leaves no address in the history; the note under a failed
  download leaves with its chat, an ended incognito one included.

**2026-10-01 — the persona page locks itself, a portrait can be framed,
and every reply can be copied or saved (#469, #473, #480, #484).** The
owner's asks from a phone, each through its own review loop.
- **Autolock (#469):** `imagelib::autolock_minutes` reads `autolock.toml`,
  beside `lock.toml` (15 minutes unless set, 1 to 240; a damaged file grants
  no unlock). It is set from Settings → Lock or `mecha imagelib
  set-autolock`. Each token keeps the span it was granted
  (`LibraryState::unlocks`), and the page relocks after the same span
  untouched (`web/src/lib/autolock.js`), checked again when the page comes
  back to the foreground. Leaving the Library or Personas tab revokes its
  unlock.
- **The persona page (#469, #480):** no crisis-detection line (the owner's
  ruling); files as tiles. "Earlier chats don't load" was a token lapsed by
  a serve restart, drawn as an empty list. A failed read while the page
  holds a token now re-reads the list, which drops the token and closes a
  persona it hid; reads are numbered across the body's wait too.
- **Avatar framing (#473):** `persona::State::frame` (`Frame`) lives in
  `state.toml`: display only, no new version, and out of range loads as
  none. It is drawn as `object-position` plus a scale about the same point,
  so no setting leaves a gap. Unplaced, a portrait leans to the top.
  `dragFrame` inverts `X = z·u + p·(S − z·R)`, so the picture follows the
  finger and an axis with nothing hidden does not move.
- **Copy and Download on every reply (#484):** `ChatProse` takes `actions`
  for a finished reply. Copy writes the reply as written; Download saves a
  `.md` from a Blob made in the page. Each code block gets its own Copy.
  `download` defaults off and each call site opts in, and an incognito chat
  never does: a file on the device outlives the room (INCOGNITO-DESIGN R2)
  — reversed the same day by the owner (#489): the file is made in the
  browser, so the server keeps no trace.
  `reply-export.mjs` pins the call sites by counting them first.

**2026-10-01 — personas read their files: folders, checked citations,
search, saving (#459, #465, #467, #475), and a page that reads well (#479).**
PERSONA-DESIGN §10's four build steps in one day, each through its own
review loop.
- **3a (#459):** a `files/` folder at three levels (own, group, everyone),
  read by name, never by path; the whole collection rides in the first
  turn below a quarter of the window, or else its list. The first turn
  never runs OCR: an unread document is read in the background, one at a
  time.
- **3b (#465):** every `[file, p. N: "quote"]` is looked up by the harness
  (`persona::cite`, over `grounding::admit`) in what the chat had received,
  and tagged quoted, on another page, not in the file, no such file, not
  read, can't check or too short. A citation opens the chat's text of the
  page, never the PDF.
- **3c (#467):** `file_search`, over one store index keyed by content hash
  and scoped by the persona's listing. It ranks by vectors from `:8081`
  fused with FTS5, by words alone where meaning is unavailable, and says
  which. `mecha_core::embed` is ported from mecha-graph's client.
- **3d (#475):** a *Save to files* link writes a reply into the persona's
  own folder, but only text that is one of the chat's own replies.
- **#479:** Markdown rendering in both chats through `ChatProse` (the mail
  reader's parser, no `{@html}`), with double-click for the raw text. A
  file sheet to download or read a file. Drop to add, with upload
  feedback. Earlier chats headed by summary, goal or opener; the goal
  behind a link.

What the review loops found most often was a check that kept working and
stopped being true: a token stuck in a guard, a dedup by neighbours, a
fallback that read a cache the reader never wrote. Each was fixed with a
test that fails on the old behaviour.

**2026-10-01 — test chats wrote into the owner's store (#471).**
`chat::test_chat()` built a door that resolved sessions, workspaces and the
outbox from the real home, so every lane's suite added "web: srctest"
sessions and test pictures to `~/.mecha`, counted by the run-quality
corpus as real chats (251 moved out by the end of the day). `ChatState` now holds its
own `sessions_dir` and `work_dir`; a test door gets a fresh temporary root
(never `MECHA_HOME`, which a concurrent `HomeGuard` may hold), and the
guarded tests read back through the door. The residue was moved to a dated
backup.

**2026-10-01 — modular installs, step 4: setup reads the registry, and a
trial carries its switches (#460, #461, #464, #466, #470, #472, #474,
#476).**
Each was merged by the mecha-d7 lane on a clean review pass, except #474:
mecha-5d's lane merged it under the owner's "merge everything" at a head
whose pass 3 had named one substantive issue still open — the refusal
calling `messages` "never on in a trial" — which mecha-ce graded minor and
fixed in #476 rather than holding the merge. All went live in
mecha-d7's deploys of `9f306d8f` (#460, #461), `85ca4dfe` (#464, #466),
`6565cfd8` (#470, #472) and `997a2894` (#474). The deploys themselves are
mecha-d7's rows, written after this one.
#470, #472 and #474 carry the owner's five rulings of the same day on the
questions #464 and #466 left open; ruling 3 needed no code, and ruling 5's
wording rode in #470.

- **4a, #460:** `onboarding::plan`'s feature steps come from `feature::all`
  (`onboarding::feature_steps`), replacing the hand-kept integration steps
  and 1a's `feature-<id>` offers. A switch written `false` reads Declined,
  an unanswered one Missing and declinable, a blocked or unready one Wrong,
  and an on one Done with its parts. `onboarding::runnable` runs only
  mecha's own binaries and `cargo` (for the `cargo install` a missing
  server's step names). `mecha setup <feature>` runs one step, and
  `--minimal` declines every optional one without writing config.
- **#461:** each decline carries its way back (`Step::undo`): `mecha setup
  --undecline <id>` for an answer given in setup, `mecha features enable
  <id>` for a switch written `false`. `mecha setup <feature>` reopens a
  declined step, in memory only.
- **#464:** `--json` never reopens, so a recorded answer exits 0, and the
  closing note prints both ways back as one group. The owner's ruling 5:
  only `--json` keeps a decline, and a plain run reopens whether or not a
  terminal is attached — one rule with nothing about the terminal in it.
- **4b, #466:** `RunConfig.features_on` records the switches a run had
  (`feature::switches_on`), loaded all-or-nothing by `lenient_features` as
  `lenient_levers` loads levers. `environment.toml`'s `requires` refuses a
  trial home with a required feature `Off` or `Blocked` before any trial
  starts. It checks switches, not readiness or an arm's levers; ruling 3
  kept it so.
- **#470, ruling 2:** `[messages] enabled` is an `Option<bool>`
  (`MessagesConfig::on`), so `[features] messages = false` reads `Off` like
  every other switch — Declined in setup, with `mecha features enable
  messages` as the way back. It had read as unanswered, and kept `mecha
  setup`'s exit non-zero for anyone who switched messaging off. The default
  is still off.
- **#472, ruling 1:** the switched-on set is a term of every experiment
  row's condition hash (`|features=`, from `Manifest::env_conditions` and
  `Environment::features_on`). The environment's digest held every switch
  but `search`, which follows the operator's, so two experiments differing
  only there shared every hash. Every newly planned row rehashed once.
  Building it found `config_at` re-reading the environment's own
  `config.toml`, which for an `extends` variant holds only its half; the
  config now loads from the resolved text (`Config::merge_environment_text`).
- **#474, ruling 4:** a `requires` naming a feature no trial home can have
  refuses at load, listing the five that can (`graph`, `tasks`, `search`,
  `frontdoor`, `publishing`); only `mail` and `docs`, which fixture servers
  stand in for, are pointed at them, and the run-time refusal says how a
  trial gets the feature it names (`how_a_trial_gets`).
  `trial_env::can_be_on_in_a_trial` reads the lists `config_at` defaults
  from, and a test holds it to what a trial actually switches on.
  Its pass 3 found the refusal calling `messages` "never on in a trial",
  though an arm's `levers_on` switches it on: follow-up #476
  (`experiment::lever_that_switches_on`, derived by applying each lever)
  names `levers_on` there instead. mecha-d7 merged it as `d19995fa` after
  two passes, and it went live in mecha-69's deploy of `36ff7573`.

Seventeen review passes across the eight PRs (#460: 2, #461: 1, #464: 3,
#466: 3, #470: 1, #472: 2, #474: 3, #476: 2, counted from each PR's
record). Ruling 1 was put to the owner on a wrong premise — that an arm
naming no environment runs on the operator's config — and put again when
the code showed every arm runs in one (the traps section has the lesson);
the owner chose the whole set both times.

**2026-09-30/10-01 — files and pictures in chats, and a measured "no" to
OCR for images (#438, #447, #450, #453).** The owner asked for the persona
chat to show pictures like the main chat does, then for files to be
droppable there, then for `document_read` in persona chats and on images.
Then they asked whether the model knows when to use it.

#438 (`fff2675e`) built three things.
- **Pictures.** A persona's pictures are drawn under its rows, and Edit
  opens the main chat's modal. They come through the persona door's own
  `GET /api/persona-chat/{key}/file` and `POST …/upload`, which reach the
  workspace only through `PersonaChats::workspace_of`, behind the lock.
  From there the routes share `files::serve` and `files::store` with the
  assistant's chat. A locked persona's picture carries the token and no
  open-in-tab link, so the token stays out of browser history.
- **Drop and attach.** `PersonaChats::send_with` puts an attached picture
  on the turn for a model that can see. The matcher (`picture.js`) and the
  drop rules (`attach.js`) moved out of `Chat.svelte`, so both chats share
  one copy.
- **D24, ruled by the owner.** `DocumentRead::for_persona` answers `Some`.
  The tool is off until a persona lists it in `[tools] allow`, and
  `answers = "files"` withholds it.

#447 (`91326fe6`) made `document_read` read images: PNG, JPEG, WebP and GIF.
- **Detection.** `kind_of` decides by the bytes, image signature first.
  HEIC and TIFF are refused by name, saying what to do instead.
- **Decoding.** `decode_image` uses the same `image` crate and limits as a
  rendered page. It applies EXIF orientation and lays transparency on white.
- **One pipeline.** A `Source` feeds the PDF renderer or the decoded image
  into the same layout and OCR stages.
- **Measured live** on a generated page:
  - upright and tagged, the page read exactly;
  - sideways (90°), PaddleOCR-VL read it correctly with or without the tag;
  - upside down with no tag, the heading came back last and a paragraph as
    a table.

#450 (`257b7a78`) answered the routing question. `attach` on eval cases puts
pictures on the first turn as pixels, the way the web chat does.
`image::attached_images` is now one implementation for the chat, the persona
chat and eval. `eval/image-read/` holds 19 cases over 13 generated,
fictional pictures, with ground truth in `truth/`.
- **Routing.** On `qwen3.6-35b-a3b-uncensored`, at seed 42 with two runs a
  case, the model chose `document_read` for **0 of 18** text-heavy direct
  questions. On **extraction tasks** (spreadsheet, CSV, transcribe,
  summarise) it chose it **0 of 12** times.
- **Wording.** A description telling it to use OCR moved that to **1 of 12**,
  so it was not shipped.
- **Accuracy.** Every answer held its values. A 310-word transcription from
  pixels scored 85% recall and 90% precision; OCR of the same file scored
  79% and 91%, in 9.4 s.
- **Ruling.** The owner kept images read from pixels, with no always-on OCR
  and no verifier: OCR shares the one GPU with the chat model, and nothing
  measured needed it.
- **Fail-closed checks.** Five review passes added them:
  - `validate`, `check_attachments` and `check_attached` (a blind model,
    an unreadable picture);
  - `exp`'s `cases_for` refuses an attaching task;
  - `unoffered_tools` warns when a check names a tool the run lacks. On a
    box without `[documents]`, "never chose OCR" is vacuous.

#453 (`e856ce36`) fixed a hold leak those runs exposed. `dispatch` kept the
router hold in a local, and `std::process::exit` skips destructors. So every
non-zero exit (a failed eval case, a failed batch item, a refused run) left
its file in `~/.mecha/holds/`. The hold now sits in `RUN_HOLD`, `exit_with`
releases it first, and `no_command_exits_around_the_hold` keeps bare exits
out of the commands.
- **Live check:** a failing eval with the fix left no hold. The binary then
  installed, from before #453, left one on the same case.

#438, #447 and #450 were deployed in mecha-d7's `346bb8a2` install. The
probes were checked here: `strings ~/.cargo/bin/mecha` holds "neither a PDF
nor a PNG" and "attaches pictures", and serve runs that binary.

**2026-09-30 — personas draw themselves (#444, #454).** `image_generate`
casts a persona's own character as "self" (`Tool::for_persona_as`,
`PersonaSelf`), so "draw yourself" is the persona's portrait, not a
stranger; #454 settled the follow-ups (a wearing/doing clause, a capped
library ask, a reply that steers).

**2026-09-30 — a persona's refused call no longer loops for minutes, and a
crisis pause keeps the taint chip honest (#446, #448).** Two fixes from the
persona lane, both installed by mecha-d7 in `346bb8a2` and still live in
`9253038b`.

#446 (`a768852e`) closed #438's last review minor. A crisis pause starts no
run, and only a run's start called `Taint::arm_for_content`. So a picture on
the paused turn left the pause's `Done`, the transcript chip and the session
file reading clean. `resume` takes taint from the file's `Taint` records and
never re-scans, so a restart kept the chat clean too. `PersonaChats::send_with`'s
pause branch now arms the conversation and checkpoints a `Record::Taint` when
that changed it. It is reachable only through a crisis hit in the session goal,
because a hit in the message pauses before any pixels are read. Test:
`a_picture_on_a_paused_turn_arms_the_chat`. Mutation checks: without the
arming it fails at the `Done`, and without the checkpoint it fails at the
reloaded file. One review pass.

#448 (`b8f27e7f`) answers the owner's "Let's fix 1". The owner's persona chat
resent one refused `image_generate` forty times a run, twice, to `max_turns`.
The second loop held the model and the GPU while a new chat drew. The loop
guard was dormant until a compaction, which a chat never reaches, and persona
agents forced boredom off. The guard's dormancy had asked for a measurement
before watching ordinary work. The scan covered 1,031 transcripts
(`~/.mecha/sessions` and `~/.mecha/personas/*/sessions`, 3,187 tool turns,
2026-09-30), counting one call with one identical `is_error` result in N
consecutive turns:
- N=3: 4 fires. Two were mail runs that recovered on the very next turn,
  after boredom's notice named the repeat (the model added `account`).
- N=4 to N=7: only the persona chat's loop.

So `LoopGuard::observe_refusals` stops at `REFUSED_REPEATS` =
`boredom::STILL_STUCK + 1` (7), after both of boredom's rungs have spoken.
It takes the usual tool-less final turn as `StopCause::Loop`, compaction or
not. Neither trigger counts a call this turn's trace marks `denied` (approver,
hook, policy, interlock), because appraisal scores `Loop` against the run
(`appraisal.rs`'s counter arm). `persona::agent::agent_config` now carries
`boredom` from the base config. The scan's count included denials; the
shipped rule excludes them, and every fire at N ≥ 4 was a tool error anyway.

Four review passes moved it:
- the threshold went from 5 to 7, which made the Delegate rung reachable;
- the denial exclusion was added, then extended to the post-compaction
  trigger;
- the "after compacting" wording was fixed in `StopCause::describe`,
  `RunEnd::words`, `render.rs`'s remedy, and two website pages.

Installed: `strings ~/.cargo/bin/mecha | grep -c 'one call refused
identically, turn after turn'` → 4 (one literal, several monomorphised
copies).

**2026-09-30 — modular installs: the design, and `mecha features` (#427,
#428, #432, #433).** The owner asked how a new user installs only the parts
they want: Slack, the web app, image generation, personas, OCR and voice
optional, an off feature hidden from the web app and refused by the CLI, and
model recommendations per feature. `FEATURES-DESIGN.md` (#427, fourteen
review passes) is the authority. It reads Hermes, OpenClaw and Codex from
source and the rest from their docs. The owner ruled all six questions the
same day. F1 is a `[features]` table of bools that ships all `false`, which
overruled the doc's first recommendation of table presence, because a user
has to be able to see what exists. F5 is `hardware.md`'s four tiers in two
columns: unified memory, and a separate GPU beside system RAM.

#428 (`d1da01aa`) is step 0, read-only:
- `mecha_core::feature` holds a closed `Feature` enum of 21 features and
  parts and five `State`s: on, off, blocked, unready, unknown.
- `state(facts, f)` takes no `Config`. `Facts::read` carries the global one,
  so a project `mecha.toml` cannot decide what the install has on.
- Each row asks what registration asks: `ToolsConfig::registers`, the
  loopback validators, `DocumentsConfig::validate`, and
  `SearchBackendConfig::problem`, which is new and held to
  `build_search_chain` by a test. It never reads a field's presence.
- `mecha features [--json]` lists them, with the fix for each off row.
- `ARCHITECTURE.md` §Features holds the invariants and an "Adding a feature"
  checklist.

Step 0 found `setup` counting mail accounts under `$MECHA_HOME`, where the
mail crate never looks. #432 (`5664f245`) moved the rule to
`onboarding::{mail,docs,legacy}_store_dir`, and made doctor
(`MailStores::of_owner`), Slack's import check and the booking sweep find
each store by its owner's rule. #433 (`c599c802`) fixed one test's temp
directory. Deployed by the mecha-d7 lane at `5664f245` and `c599c802`.
#441 (`76c3331e`) closed a gap the design found that predated it: `[documents]`
was stripped from project layers but missing from
`trial_env::OPERATOR_ONLY_TABLES`, so an experiment environment could point
OCR at a remote server or run the PDF parser unconfined. `config_at` now
refuses it like the other five.
On 2026-09-30 `mecha features` read 20 of 21 rows on here; `messages` was off.

**2026-09-30/10-01 — modular installs, steps 1–3: every feature follows its
switch (#443, #445, #449, #451, #452).** Each sub-step below was one PR,
merged by the mecha-d7 lane on a clean review pass. Steps 2 and 3a went live
in mecha-d7's deploy of `346bb8a2` on the evening of 2026-09-30, and 3b in
mecha-ce's deploy of `e856ce36` at 00:34Z on 2026-10-01 (HANDOFF's dated
machine-state log has both).

- **1a, #443:** the `[features]` table (`Config`, `ConfigLayer`, `apply`, the
  project strip), `mecha features enable|disable` written in place by
  `feature::write_switches` (`toml_edit`, so comments and a newer build's keys
  survive), the upgrade notice (`feature::announcements`, printed by
  `commands::features::print_notices`), and the environment refusal. It gated
  nothing. This machine's table was written in the same deploy, 12 switches
  true and `messages` absent.
- **1b, #445:** registration and connections ask `feature::switched_on`,
  never the readout; `feature::server_refusal` covers the four verbs that
  spawn the graph server themselves; `mecha serve` refuses without `web`.
- **2, #449:** `GET /api/features` (`serve/features.rs`) returns the same rows
  as `mecha features --json` plus `pending`, re-reading the global file per
  request and opening no socket. The web app reads each row's `shown`
  (`web/src/lib/features.js`), so it never restates which states hide.
  Settings → Features lists every feature with its command. Building it
  found that `state` answered `Off` for a switched-on feature missing its
  settings, which §5 calls `Unready`; `state` now converts a switched
  feature's own `off`, and anything standing on an `Unready` need reads
  `Unready`, never `On`.
- **3a, #451:** `feature::refusal` is the one predicate behind a route's
  404 `feature_off` and a verb's one sentence. Every route is added through
  `serve::api()`'s `.at(path, owner, …)` (`serve/gate.rs`), and the guard sits
  inside `owner_guard` and reads the file per request (`Gate::Live`). Verbs
  call `commands::features::require`, which refuses when the config does not
  load. It found that the graph's review queue, `mecha review accept`, the
  web's entity create and merge, and the TUI's `/queues` and entity panel
  ran `mecha-graph` directly and wrote the owner's graph with `graph = false`.
  The owner's ruling L1: the library follows `image`, and `[tools]`
  withholds only the model's library tools.
- **3b, #452:** `Feature::gated` is true for every feature, so Slack,
  personas, voice, incognito, the front door (with `factory-publish` now in
  `server_feature`) and messages refuse whole. The owner's ruling M1:
  `mecha msg` refuses when messages is off, reads included. Reading a store
  and deleting cached data stay open; changing an entry is refused.
  `mecha serve` mounts its voice facade only with calls on at start, and the
  TUI registers `show_file` only with Slack on. `trial_env::config_at`
  defaults `frontdoor` for a carried `factory-publish`, as it does `graph`.

Twenty review passes across the five PRs (#443: 1, #445: 4, #449: 4, #451: 5,
#452: 6, counted from each PR's record) found the same gap four times, all in
#451 and #452:
something reached a feature without going through its verb (the traps
section has the rule). On 2026-09-30 the owner had `messages` switched on, and on
2026-10-01 `mecha features` read 21 of 21 on, so nothing refused on this
machine.

**2026-09-30 — paint the part of a picture to change (#424, #429).** The
owner asked for the web chat's Edit button to become a modal where areas can
be painted. `IMAGE-REGION-EDIT-RESEARCH.md` (#424) measured the ways to
target a region on Qwen-Image 2.1, on the #408 picnic picture, four seeds,
60 images. A red box drawn on the picture, or a mask passed as a second
image, only hinted, and the rest of the frame drifted as much as a plain
edit. A latent noise mask with a composite in mecha (C) kept every pixel
beyond the mask's edge exact, blended seamlessly, and landed 8 of 8 local
edits. Crop-edit-paste (D) landed a pose, but left seams where its edge
crossed a person. C′, the region hidden from the reference, lost her
identity or copied its placeholder, and was dropped. The owner chose C.

#429 (`f0e62112`) builds it:
- `EditModal.svelte` has a brush, a box and an eraser. Painting dims the
  rest of the picture.
- The page uploads the mask but never attaches it, so it never reaches the
  model as pixels. `image_generate` takes it as `mask`, read through the
  jail.
- `prepare_mask` sizes it to the encoder's canvas (`edit_canvas`), grows
  it by about 21 px and feathers it by about 16. The graph samples under
  `SetLatentNoiseMask`, and `composite_masked` lays the result back over
  the original.
- The near-copy check reads only the painted cells
  (`layout_similarity_painted`).

Six review passes found:
- the modal carried one chat's paths into the next;
- "painted" was judged from the op list, not the canvas;
- a second finger hijacked the stroke;
- a mask that failed to encode fell through to a whole-picture edit;
- "pixel for pixel" was wrong for a photo larger than the canvas.

The live check, with the local model, found two more things. Refusing a
`size` beside a mask made the model retry without the mask (now the size is
set aside). And the model copied results over the original picture (the
result text now says not to). After the fixes, 4 of 4 real masked edits
landed, identical beyond the mask's edge. Installed 2026-09-30 about 04:31Z
by mecha-d7 (main `c599c802`). Between the dist rsync at 04:29Z and the serve
restart, the new page ran against the old binary, which ignored `mask` and
would have edited the whole picture.

**2026-09-30 — the `personal` Google grant outlived seven days.** The
project was published to production on 2026-09-16, and `personal` was
re-consented on 2026-09-17 (`granted_at` 2026-09-17T02:10:19Z in its
`oauth.json`, replacing the 09-15 grant). On 2026-09-30 (day 13) its
refresh token was still minting access tokens, and a live read-only
`mecha mail calendars` answered. A grant minted in Testing is revoked on day 7. So the guard the handoff had
kept for exactly this, `grant_lifetime_days = 7` on that account in
`~/.mecha/mail/accounts.toml`, came out, as its own note said to once a
grant was observed past day eight. The owner removed it; the backup is
`accounts.toml.bak-2026-09-30`. `mecha doctor` went from one mail finding
("sign-in has expired") to none. The finding had been a declared lifetime,
never a failure. The clock is a property of the grant rather than the app,
so only a consent made after publishing could have shown this.

**2026-09-29 — PDFs as a tool, a layout stage for tables, model servers
that start on demand, and the persona design (#403, #404, #406,
mecha-graph #26).** Merged and installed the same evening; the install is
the 20:42–20:56Z entry in HANDOFF's *Machine state, dated*.
- **`document_read` and `mecha document` (#404).** Each page comes back
  twice and never merged: poppler's text layer (exact — what `grounding`
  checks a quote against) and a PaddleOCR-VL 1.6 transcript. Poppler runs
  confined with no unconfined fallback, page images are re-encoded before the
  OCR server sees them, and results are cached by content hash in
  `~/.mecha/documents/`. A parser's diagnostics are the document's words
  (`ParserSaid`, marked external); the sandbox's own refusal is not. The OCR
  server is `llama-ocr.socket` (:8085 → :18085), started on first use and
  stopped after ten idle minutes: cold start 2.96 s to healthy. Measured on
  10 arXiv papers, 48 pages: median 4.5 s/page on a quiet GPU (up to 53 s
  under ComfyUI), word recall 0.944 against the text layer.
- **The layout stage (#406).** PP-DocLayoutV3 (ONNX, Apache-2.0) in a
  confined, CPU-only Python child that receives only a float tensor from
  mecha's own decode; each region goes to the model with its own prompt.
  Tables, 9 tables and 89 rows: rows held on one line 24 (0.27) → 82 (0.92);
  Transformer Table 2 regains its EN-FR column. One real loss (GCN Table 3's
  `K` sub-labels). Numbers are commit `99d132d6`'s; re-measure before quoting
  against a later one.
- **The embeddings server on the same mechanism.** `llama-embed.socket`
  holds :8081 from boot; the model starts on first connection (4.1 s cold,
  0.03 s warm, measured at the switch) and stopped itself after ten idle
  minutes (20:54:22Z). mecha-graph #26 went in first: `Embedder::health_within`
  (`EmbedHealth::{Ready, Absent, Failing, Misconfigured}`) waits out a cold
  start, and `router::needs_vectors` keeps a query that embeds nothing from
  waking the model.
- **The persona design (#403, `docs/PERSONA-DESIGN.md`).** Every decision
  (D1–D23) ruled by the owner. Five review passes reshaped it: separation by
  *location* (`<persona>/sessions/`) rather than a `SessionKind`, tool
  eligibility declared per tool and defaulting to no, web search forced
  through the blind path, and a stated failure direction for every safety
  check. The build is mecha-69's lane (#405, #407, #409, #415 merged).
- **Review loops:** #403 five passes, #404 seven, #406 five, mecha-graph #26
  eleven; every major and medium finding fixed on its branch, the minors
  filed as mecha #410–#414 and mecha-graph #27–#28.

**2026-09-29 — image edits that came back unchanged: the prompt, not
chance (#408).** The owner reported that an edit asking to move someone or
change a pose sometimes returned the same picture, and that only a new chat
got past it. Measured on the fictional picnic scene (one picture, the Q4
GGUF and the w4a8 text encoder, 40 steps), with the seed held fixed:
- The failing seed from a live chat, 363985941, reproduced its near-copy
  exactly (layout similarity 0.995). The failure is set by the seed, the
  prompt and the model files, not by chance at run time.
- On the same 12 seeds, a caption of the scene stood Maya up 0 times: 11
  near-copies, and a twelfth that redrew the scene with her still sitting.
  "Have Maya stand up …" alone stood her up 8 times, with 3 near-copies and
  1 partial. "Keep the watercolor style, the lake, willow
  tree, and red checkered blanket unchanged. Have Maya stand up …" did it 12
  times. Every image was also judged by eye.

The edit model reads a caption, or "keep <image1> unchanged", as the picture
it already has. #408 (`b26537b0`) makes the tool description, the `prompt`
field and a new near-copy notice ask for the kept parts, named, then the
instruction. The first draft's guidance, "describe the finished picture", had
produced exactly the caption that failed (*Traps*, Measuring). Each edit's
`layout_similarity` (`imagegen::layout_similarity`) now goes in its
manifest. At `NEAR_COPY_LAYOUT` (0.78) the result says the layout did not
change and how to retry, never that the edit failed, because a recolour
scores 0.78–1.00 too. Strikes count per picture edited, held as salted
hashes (`ImageGenerate::near_copy_key`), and a second in a row says stop.
Three review passes; the first two found that a recolour chain was told to
stop. In four live Edit-button chats with the final wording, the model wrote
every prompt in the new form and all four stood her up. One needed a retry,
which landed in the same turn in about 3½ minutes. Installed 21:05Z
(HANDOFF, *Machine state*). The quantisation comparison, the first suspect,
never ran (HANDOFF). Closed with it: the HANDOFF item saying the result
text tells the model "the user can" see the picture was obsolete, since
`9d16e757` (image_view on request, 2026-09-27) had removed the phrase.

**2026-09-29 — the appraisal audit's fixes: seven PRs merged and
installed, and the backfill that found 2e-1 starved.** The audit of
2026-09-28 read a store in which every appraisal predicted `no_act`, the
readout said "hit rate 100%", valence stood at +1.0 / −93.0, and 2e-1 had
0 eligible interventions. The audit's hit-rate finding and the
owner's rulings (1B, 4D, 2A→C, 3D→D, in-run prediction → a, and R44–R47 on
2026-09-29) became these seven PRs, one per bullet. In-run prediction, R45,
R46 and R47 are all #401. 4B needed no code: model probes run in incognito
from now on, and the two probe sessions already recorded were deleted
outright.
- #377 counts a `no_act` hit on an output that offered nothing to act on
  (`output_offers_act`) as `forced`, keeps it out of `hit_rate`, and
  withholds the rate while any hit is `forced_unknown`.
- #378 stops asking for that prediction at write time
  (`withholds_expectation`) and counts the row as `not_asked`.
- #382 adds `mecha sessions mark <id> experiment` / `unmark`. A ledger
  withdraws the session from listing admission, every appraisal-store door,
  `learn` (`Admission::Withdrawn`), `validate` and `lesson_pass`.
  `proposals accept` refuses a proposal resting on it, and only the owner's
  own terminal can mark.
- #387 (`OutboxItem::drafting_verdict_signs`) signs an edit or reject only
  when it is stamped as the owner's, or when it predates the stamps.
- #388 is `mecha distill --backfill-appraisals`, and ranks past appraisals
  by the session's end.
- #400 is R44's gate (`Report::gate`, `GATE_MIN_DECIDED` = 10,
  `GATE_MAX_TRAIL` = 2): pilot, no worse, worse, or floors.
- #401 is `forecast.rs`, a base-rate forecast of the owner's act on each
  staged draft, sealed in `<outbox>/forecasts/forecasts.jsonl` and read
  only by `sessions appraise`.

Each ran the review loop to a pass with no major or medium finding. The
passes after the merges found two more, both fixed before merge. The
doctor's starvation check had counted a marked session's reflections as a
pool that `learn` would consume. And a store torn with nothing else
readable had read back as fully read, with a pilot gate. All seven were
installed at 19:50Z (HANDOFF, *Machine state*).

The backfill then ran (20:30–20:52Z, 22 minutes on the local model). It
appraised 8 of its 9 sessions, and one reply was unusable. Every row was
stamped `backfilled` with no prediction, and the existing lines were
byte-identical. All 8 came out not clean, because an appraisal's origin is
the session-end taint and these sessions had read mail or the web, most
from their first checkpoint. 2e-1 still reads 0 eligible, so its evidence
has to come from somewhere else (HANDOFF). #399 had recorded these PRs as
open; this entry supersedes it.

**2026-09-29 — people in a picture who are not library characters
(#390, #391), and adding to the library from the page (#394).** `IMAGE-COMPILER-RESEARCH.md` §E11
and §E12 are the evidence.
- E11 (4 images): a head count covering only the cast ("Exactly three
  people") pushed a described waiter into the background; counting him in
  put him at the table.
- E12 (4 images): a total with no extras erased him outright, while "each
  of the N appears exactly once" alone still drew him, and held a
  four-person cast to four right faces.

#390 (`a6941f8d`) adds `extras` to `image_generate`: people described in
the scene and taken from no reference. The compiled wording becomes "each
of the N people from the images appears exactly once", with a total only
when `extras` are counted into it. A name in both `cast` and `extras` is
refused. The same PR closed four follow-ups from #384 and #385:
- `imagelib::write_atomic_mode` now takes its temp file back on a failed
  rename;
- library search drops common words (`tool::image_library`'s `STOPWORDS`),
  so "picnic in the park" finds `picnic-park`;
- CLAUDE.md's module map gains `imagelib.rs`;
- `ARCHITECTURE.md` says locking does not reach portraits a browser already
  cached `immutable`.

#391 (`fc0b7c75`), from #390's review:
- a scene with only `extras` draws with no head count ("Also in the scene:
  …") and needs no library;
- the rename cleanup gets a test that can go red
  (`a_failed_install_leaves_no_temp_file_behind`);
- the `ARCHITECTURE.md` bullet names its exceptions.

#394 (`8103d746`) lets the Library tab add a character (an uploaded
portrait, a name and a description) or a style, and edit an approved
entry. Each action runs the same `mecha imagelib` child the terminal does,
behind `/api/library/add` and `/api/library/edit`. The page scales the
portrait to 1536 px and re-encodes it, which drops its photo metadata. An
edit honours the lock and is refused on a candidate, because a rewritten
text would approve it. All three were installed 2026-09-29 at 04:06Z
(HANDOFF, *Machine state*).

**2026-09-29 — the image library reaches the page (#384, #385).** #384
(`1139ac79`) makes every pre-GPU `image_generate` refusal lead with
"Nothing was drawn." (`imagegen::refused`). The first live run had read a
refusal that opened with the characters' names as a finished picture,
never retried, and told the owner the image existed. The retry skeleton is
the whole cast in prompt order, with a split refusal past `MAX_CAST`, and
library search is name-first, then every word of three or more letters.
#385 (`c6ae2c69`) is the web Library tab, Save to library on the image
card, and a browse lock whose password is optional (the owner's ruling:
with none set, it is a plain toggle). Approval of a model's proposal is an
HMAC of the shown text under a key `serve` holds only in memory, checked in
process (`serve::library`). It replaced a CLI `approve --shown <sha256>`,
which any shell could compute and use to approve a proposal nobody had
read. Installed 2026-09-29 01:07Z (HANDOFF, *Machine state*).

**2026-09-28 — voice calls in incognito chats (#376), and a voice call that
never connects ends after 90 seconds (#386).** `INCOGNITO-DESIGN.md` §6.4
is the authority. The v1 exclusion was the voice worker's logging, and
measured, the larger trace was pipecat's own: the runner re-adds a `DEBUG`
sink on start, and `Transcription: …` plus every sentence handed to the TTS
had reached the on-disk journal (235 such lines in the fourteen days
before). #376 (`d6e8f161`, seven review passes) closes it in four layers,
each refusing by default:

1. The worker's `Unlogged` disables loguru's `pipecat` namespace at the core
   while any incognito call is live. It is counted, so overlapping calls
   cannot lift each other's silence.
2. `OfferSilence` and `bot()` hold the silence from the moment the offer
   arrives, keyed on the raw name prefix.
3. `serve::forward_offer` asks the runner `GET /mecha/unlogged` before it
   forwards an offer naming an incognito chat, and answers 409 otherwise.
   The page (`voice-core.js`'s `refusesAnswer`) lets no media flow without
   the `unlogged` flag serve adds to the answer.
4. An unvouched spoken turn is refused twice: in `begin_turn` by
   `spoken_turn_may_enter`, and earlier, on the key alone, in
   `VoiceHost::speak`, ahead of the barge-in, so the refusal cannot cancel
   the chat's run in flight.

No line says an incognito chat was spoken into, not even redacted
(`session_line`, the affect latch, the facade's refusal path), and
`stamp_presence` skips `~/.mecha/runs/voice.json`, which ordinary runs
record in their situation briefs. Review found the seams one pass at a time:

- the presence stamp (pass 3);
- the gate sitting downstream of an old worker's logging (pass 4, the one
  that changed the design);
- serve and the worker classifying a malformed prefixed name differently,
  in the direction that logs (pass 5);
- an unparseable offer relayed unexamined (pass 6).

The owner's rulings: voice is in, and `--voice-yes` applies as in any
hosted chat. #386 (`dbce699e`, two passes) came from the deploy's own
measurement. A synthetic incognito offer that never connected held the
silence for the full fifteen minutes to the idle timeout, because pipecat's
60 s connection timeout closes the peer and not the bot. `run_bot` now ends
such a call at `CONNECT_DEADLINE_SECS` (90 s), and `settle_deadline` lets a
deadline that fired finish the `runner.cancel()` it started. Both are
installed (HANDOFF, *Machine state*, 2026-09-28/29); a real spoken call is
owed.

**2026-09-28 — the image library, and what a reference is worth (#380,
#383).** #380 (`b21d8e8a`) is `IMAGE-COMPILER-RESEARCH.md`: 59 local
Qwen-Image 2.1 generations across E1–E3, E8 and E9. Identity lives in the
reference, not the words. By ArcFace, a description alone scored 0.33
(0.10–0.34 is the different-person band), the reference pointer 0.74, and
pointer plus description 0.78. Four full-size references double generation
time, while 512² ones stay cheap. #383 (`3bbd53c5`) builds on that:
`mecha-core`'s `imagelib` (the store at `~/.mecha/imagelib`, and the
compile step), `image_generate`'s `cast` and `style` with a manifest beside
every PNG, the `image_library` and `image_library_propose` tools, and
`mecha imagelib`.

**2026-09-28 — deleting a conversation means every trace (#381); the
learning store without git (#379); the graph can redact (mecha-graph#25).**
#379 (`55e4a335`) is the owner's ruling that the learning system must not
use git. `LearningStore::commit` became `log_pass`, which appends `{at,
message}` to `~/.mecha/learning/passes.jsonl`. A legacy `.git` is a
`mecha doctor` note, never a finding and never a runnable remedy, and the
live one's 467 commit messages were copied into `passes.jsonl` before it
was removed. mecha-graph#25 (`572fdfd`) is `mecha-graph redact --source S
--source-id ID [--vacuum] [--tombstone-absent]`. It reaches every table,
pinned by a schema-driven sweep test, and runs `secure_delete`, an FTS
optimize and `VACUUM`. Undo is all-or-nothing, refuses to restore onto a
reused episode or fact id, and `undo --discard` purges what an entry left
behind. A derived (npmi) belief the redacted episode anchored is
re-derived from its surviving contributors: re-rendered, or closed below
`NPMI_MIN_COOCCUR` unless user-verified (the owner's ruling; privacy path
only). #381 (`d968436e`) archives and deletes conversations: the web
drawer's menu and archived section, `mecha sessions
archive|unarchive|delete`, and `/api/sessions/{id}`.

- **Archive is filing.** It is a marker in `sessions/.archived/`, every
  reader keeps reading, and opening an archived conversation un-archives
  it.
- **Delete is every trace** (`mecha_core::forget`). The transcript is set
  aside, and every store is purged under its own lock: the outbox,
  questions, messages at both ends, and learning (a rule sourced only by
  forgotten reflections is removed, not retired), plus the rest the forget
  module lists. The graph is redacted with `--tombstone-absent`, and a
  backstop grep names anything that survives. The transcript goes last, and
  only if every store answered; otherwise `mecha doctor` reports it
  `Broken`.

Deploy order: #381 needs a mecha-graph that has `--tombstone-absent`.

**2026-09-27 — appraisal wiring, the day after: the measurement passes join
the nightly, rows 2e-4b and 3a-2 land behind levers, and every owner act on
a draft records who made it (#333, #342–#345, #348, #352, #356).** The
authority is `APPRAISAL-WIRING-DESIGN.md`, rulings R42 and R43 in its §6.

- **The nightly now measures.** #333 (`f62e345a`) adds `mecha sessions
  compare` (2d-1) and `mecha learn --compare-sources` (2e-1) to
  `scripts/ruminate.sh`. It counts an owner-bound check point as
  `owner_bound`, apart from `unavailable` (`pointwise_pass::Tally`), by the
  owner's ruling of 2026-09-26.
- **Row 2e-4b ships as two halves, both behind levers that ship off.**
  #342 (`a03362e9`, 2e-4b-1): planning success examples, drawn from what
  the owner verified (`[agent] success_examples`). #345 (`c4576665`,
  2e-4b-2, R43): contrast evidence, where a correction in a verified
  success's region is reflected beside that success
  (`[agent] contrast_evidence`).
- **Row 3a-2.** #344 (`779fb70d`, R42): a task run's situation brief names
  its previous attempts (`brief::attempts`) and quotes only the owner.
- **R16a's ruling D3, carried to every act on a draft.** A reject's reason
  is the owner's words only at the owner's own door: #343 (`6d33a017`,
  `OutboxItem::resolved_by`). An edit is the owner's writing only when the
  owner made every edit and the release: #348 (`d32f9318`,
  `OutboxItem::owners_edit`). A draft released unchanged is the owner's
  verdict only when the owner released it: #352 (`e3afb783`,
  `OutboxItem::owners_unchanged_release`). #356 (`218fd600`) is the
  follow-ups: structural docs, "sent unedited", and previous attempts that
  name the owner only for the owner's own act.

The two 2e-4b levers' measured `mecha exp` runs are owed. The edit and
reject signs stayed actor-blind; #387, open at this writing, carries the
rule to them (merged 2026-09-29, under that date above). Verified on `main` at
`2847fbfe`: `grep -c compare scripts/ruminate.sh` prints 9, and each symbol
named above is present.

**2026-09-27 — a web chat opened or reconnected mid-run keeps its history
(#358).** Reported from the phone with a screenshot: an incognito chat showed
no history and no pictures, and its transcript began at whatever tool call
streamed after the page connected. `serve::chat::transcript` answered
`entries: []` while a run held the conversation, on the reasoning that "the
page catches up over SSE". That holds only for a page subscribed since the
run began. Every stream reconnect calls the page's `load()`, which *replaces*
the transcript, and nothing re-read at `done`. So a phone whose stream died
on screen-lock mid-image-edit lost the history, and the `image_generate`
result the picture renders from, until a manual reload.

#358 (`854c4f35`, four review passes) keeps the run's starting messages on
`Live::history`, shared with the run's own `before` and rendered only when a
page reads it. `Live::taint` carries their taint armed for their content by
`Taint::arm_for_content`, because the loop arms inside the run, after `Live`
is built. The page records a read taken mid-run (`partialRun`) and re-reads
at `done`. It carries its page-only cards (`pageOnly`: drafts, notices, and
words the run never took) in the same step that replaces the list, and drops
a superseded answer (`loadGen`). It still catches up when a `done` overtakes
its read (`doneSeq`) or never arrives, the rail's `running` flag being the
belt for the second, and it follows the view's abort signal.

Measured: `a_transcript_read_mid_run_returns_the_history_the_run_started_from`
and `a_mid_run_read_is_tainted_by_the_history_it_carries` fail on the old
arm, with `left: []` and a clean chip over `[image]` respectively.
`web/test/chat-catchup.mjs` lifts the shipping functions and drives them
against a `fetch` stub released by hand: 24 checks, and two mutation checks
that each fail two of them. Deployed 18:15Z (HANDOFF, *Machine state,
dated*). #362 followed: the plan got its own read (`serve::chat::todo`),
because `refreshTodo` took the plan from the transcript read, which was cheap
only while that read returned nothing. The phone itself is unmeasured.

**2026-09-27 — the router arc's follow-ups, and a Qwen-VL image floor
(#360, #361, mecha-graph#23).** #360 made `scripts/pin.sh` say why on
stderr when it cannot tell whether the default follows a router — parsing
`config show`'s stdout alone, since a warning mixed into the TOML read a
router box as unreadable and pinned it, the load the message exists to
report — corrected `ruminate.sh`'s claim that an unpinned retirement scan
after a switch errs toward retiring nothing, and has `mecha rules` say when
its tallies mix models. mecha-graph#23 records on each `extract_state` row
whether its writer kept a reason (`reason_recorded`, V027), so `extract
--charged`'s unexplained count stops growing with every clean night; its
first revision read the boundary from `_migrations`, which no copy path
carries, so a fork or snapshot reverted the count. #361 floors every Qwen
preset's images at 1024 tokens (`image-min-tokens`), after the build warned
at every load since 2026-08-21 that grounding needs it (llama.cpp #16842);
`scripts/vision-probe.py` — a synthetic UI with exact boxes, scored by IoU,
naming only the resident model — measured the floor lifting a 320×240
image's IoU from 0.758 to 0.935 with a full-size control unmoved. The same
probe overturned the docs' "an image costs a fixed 294 tokens": on this
build a Qwen image costs one token per 32×32 px (82 / 477 / 1521 measured at
three sizes), which `LLAMA-SERVER.md` records beside the superseded reading.
Installed 17:44–17:50Z (HANDOFF, *Machine state*).

**2026-09-26/27 — model switching, server side: a llama-server router on
`:8080`, followed per run (#337, #339, #340).** `REMOTE-SURFACE-DESIGN.md`
§14 (D12) is the authority. The owner asked for the web chip to switch among
Qwen3.6-35B, its uncensored build, Qwen3.8-27B and Gemma4-26B, loading a
model when it is picked rather than holding all four. #337 (`e8c65f8b`,
twelve review passes) put llama-server in router mode — `--models-preset`,
`--models-max 1`, the request's `model` field selecting — generated by
`scripts/start-router.sh` as the one authority on the flags (owner's
ruling), and taught mecha to follow it. A `follow_loaded` entry takes
whichever model is resident (`provider::router::observe`, one snapshot per
process; the trigger daemon observes again per fire), and any provider a
caller *names* — `-p`, `--model`, `MECHA_PROVIDER`, a trigger's field, an
experiment arm, `eval` — is a pin. Background permit seats are `max(slots −
1, 1)` of the resident model (owner's ruling). `mecha model use` carries
three rulings kept from a withdrawn peer design: a failed switch reverts
(R1), a switch waits for idle unless `--now` (R2), and a preset's
temperature must equal its entry's (R4). Two router traps shaped every
probe: a bare `/props` is a placeholder (`model_alias: "llama-server"`,
`n_ctx: 0`), and a probe naming a model loads it unless it sends
`autoload=false`. #339 (`064589e1`, five passes) moved the bench and probe
scripts onto `scripts/served-props.sh` and closed the last readability
gaps. #340 (`ca518a71`, nine passes) added three Qwen3.8-27B presets
(unsloth UD-Q4_K_XL, HauhauCS's uncensored Q4_K_P, huihui's abliterated
UD-Q4_K_XL) against llama.cpp rebuilt at `95887577` — Qwen3.8 went from
18.2 to 20.7 tok/s on the new build — and pinned `reasoning-preserve =
true` on every Qwen preset, because llama.cpp #28174 flipped the default and
the two families' templates disagree about keeping old thinking (owner's
ruling). Its review found the lookup that had lost the Qwen3.8 preset the
day its repository gained a second snapshot, then a replacement that ended
the script silently under `set -euo pipefail` when a file was absent; the
lookup now walks snapshots newest-first and keeps weights and draft in one
(`hub_file`, `hub_pair`), and `scripts/test_start_router.py` builds caches of
both shapes. Measured on the presets at install (MTP acceptance beside
each): Q4_K_M 21.1 tok/s (0.38), UD-Q4_K_XL 22.9 (0.45), HauhauCS 26.8
(0.57), huihui 22.1 (0.42). Installed 2026-09-27 01:27–01:45Z (HANDOFF,
*Machine state*).

The install night found the rule the router makes concrete: **a named
provider is a pin, and on a router a pin loads that model** — so background
work names none, and the recorded `model` is what keeps the corpus honest.
Three places named one. The `distill` `session_end` hook's `-p local`
evicted the comparison arm a minute after it was selected (removed then,
with the `morning` trigger's `provider` line). mecha-graph read the
router's placeholder as its served model, so the 01:30Z graph nightly sent
`model: "llama-server"` on every request, the router refused them all, and
`extract` marked 100 episodes attempted and lost 30 summaries. And
`ruminate.sh`'s `-p local` default loaded production over the arm at
03:30:14Z, with `frontdoor.sh` and `learn-live.sh` carrying the same
default.

Both fixes merged the next morning at the owner's direction, after 15 and
10 review passes whose findings kept narrowing rather than ending; what was
left is owed in each PR's closing comment. mecha-graph#22 (`139d6e48`)
reads a router's one resident model from `/models`, and settles whose a
failed request is by asking: the same request with an empty input (the
canary) — failing it too stops the run with nothing marked, answering it
charges the episode, and a charge records why (`extract_state.failure`,
V026; `extract --charged`). Comparing error strings could not do this,
because the messages are built from values constant across a batch; a
reply the server's own setup explains (reasoning that ate the allowance) is
classified instead, since an empty input always fits. #346 (`cb76adf3`)
makes the scheduled scripts follow the router — `scripts/pin.sh` drops the
pin only when the default provider is one mecha follows, and keeps `-p
local` otherwise, so an install whose default is a paid API is never billed
by the change — and, by the owner's ruling, makes a retirement count only
the convictions measured on the model in use, while whether a rule was ever
measured (probation, and what the learner consolidates from) is still read
off the whole ledger.

**2026-09-26 — incognito step 4: images leave nothing on the image server
(#331).** Merged at `39c9f359` and installed about 03:50Z. It keeps a
per-job trail in the room, discards the image server's temp previews and
uploads, forgets the job history, and sweeps the rooms a dead `serve` left.
`image_generate` is offered in an incognito chat only where the server's
temp directory is named and on tmpfs (`incognito::images_forgettable`,
`[image] server_temp_dir`). The canary's remaining legs rode with it: an
image prompt against `fake_image_server`, a `$TMPDIR` scan (`TmpDir`), and
the default log level. The same PR fixed #326's three review minors: the banner now
says the chat ends "30 minutes after this page is closed", a chat reaped
between two reads draws the gone screen rather than an error strip, and
focus returns to the composer after a new chat is opened from the gone
screen.

**2026-09-25/26 — appraisal wiring, phase 2 continued: the gate's
acceptance rule completed, the appraisal's prediction scored, and learning
reads who made an error, what went right and how the owner judged a rule.**
Ten rows merged after the phase-1 handoff (#310). `APPRAISAL-WIRING-DESIGN.md`
stays the authority: each row's catalogue entry there ends with what it
built and what it left, and the owner's rulings R36–R41 are rows of its §6.
In merge order (times UTC):

**2d-2**, #322 (`410783a5`, 22:54Z on the 25th): R26's acceptance rule,
completed by R36, in `candidate::combine`. Point-wise *for* with the
numeric guard `Held` accepts (a `Security` or `Architecture` change still
only proposes); *for* with a regression rejects; *against* rejects; and
*undecided* leaves the numeric verdict unchanged, recorded as
`Basis::NumericOnly`, so the 2026-08-22 auto-accept stands. The point-wise
side decides at `MIN_DECIDED_POINTS` (4) with strictly more candidate-only
passes than baseline-only ones. `harness ruminate`'s measurement drives up
to eight points per candidate (`pointwise_pass::compare_candidate`),
stored with the candidate id as `proposal_id`, and `Judgement` gained a
typed `Guard`. **2b-2**, #324 (`be373132`, 23:43Z): each text appraisal's
`expected_act` is scored against the owner's recorded act on the session's
output (`appraisal_store::observe`), and "no act" is the act once R37's
window closes. That window is the outbox's patience when drafts were
staged, a task output's `due_at`, else 48h (`NO_STORE_PATIENCE_HOURS`),
counted from `TextAppraisal::session_ended_at`. An unreadable store is
unknown, never "no act". Each resolution is written once to
`~/.mecha/appraisals/scores.jsonl`, a miss as a surprise.

**2d-3**, #328 (`ff96d9f6`, 00:12Z on the 26th): a decided point-wise
comparison's losing arm becomes an `appraisal_store::Counterfactual` in a
side ledger, `counterfactuals.jsonl`, joined by appraisal id. An amendment
row in `appraisals.jsonl` was rejected because an older build would load it
as a second appraisal of the session. `AppraisalStore::teach` runs on every
`mecha distill` pass with no model call. It teaches only from a
structurally `Separated` comparison whose verdict it re-derives from the
arms, renders every word from the typed record, and points with the new
`Pointer::Comparison`. **2e-1**, #327 (`4ec0f919`, 00:27Z): `mecha learn
--compare-sources` drives validate's probe three times at each steer or
denial the reflector reflected on: with no rules, with the reflector's
lesson alone, and with the appraisal's lessons alone. Each verdict is a 1g
comparison (`Kind::LessonSource`), and each source's validation rate per
region is re-read from the store; `sessions appraise` prints the same
(`lesson_sources`). Nothing is learned. It is R25's gate for 2a-4 and
2e-2, and it has not yet run on this install (HANDOFF). **2f**, #329
(`b180a2e6`, 01:38Z): under R38 the nightly draw is split on one seed.
`harness_probe::draw_pool` mints the candidate id and draws the pool and its
uniform holdout before the diagnosis. The diagnostician's brief carries the
clean appraisals of `Pool::remainder`, never the holdout's, as
`diagnose::AppraisalNote`, which only `&Clean` can construct: words only,
and under 11,000 characters in all. `Pool::select` ranks afterwards exactly
as before, which a test held against the old single-phase body copied
verbatim (since 2e-6 it holds the holdout alone to that body, as
`the_split_draw_holds_the_single_phase_holdout_under_the_ranking`). A
brief carrying a note arms `private` taint. The stage lever `[agent]
appraisals_in_brief` (on) is the appraisal-off preset's reach into
`ruminate`.

**2e-3**, #332 (`dce65cac`, 04:27Z): mecha-graph's D3 contract, ported as
`attribution::decide`. A correction is a data error, a behaviour error or
a gap, looked up from what the run was given (`grounding::calls`), with its
spans grounded first (`grounding::holds`). `learn` admits a reflection only
through `Reflexion::attribution_admits`, so a behaviour rule is mined only
from a behaviour error. Reflections from before the field are unknown and
not mined (six unprocessed, clean `behavior` reflections at merge, #332's
own count); an owner's edit admits whatever the class. **2e-6**, #334
(`450a5cc6`, 04:27Z): `replay_priority` is gain × need × decay. Gain is the
owner's verdicts (`GoalError::is_owner_verdict`), charter-weighted on clean
records, plus 2b-2's clean surprises. Need is `ln(1 + n)` over the runs
matched in the session's `Situation` region, mecha-graph's Selector demand
term ported. Decay is a 14-day half-life. An unknown factor ranks between
known positive and known zero, and an episode that sits in a selection
slice on three nights without a win ranks last. Under R39, headroom gates
the harness selection and the priority orders within it; `learn`'s batches
and `validate --cover`'s budget use the same order. `sessions compare`
ranks its uniform draw, while `compare_candidate` stays uniform because its
points are R36's confirming sample. The holdout is drawn first from ids
alone, unchanged.

**2e-5a**, #335 (`ab1f7375`, 18:06Z): the anchor in force at the
intervention (`Transcript::anchor_covering`) is the second, fallback source
of `Reflexion::goals` (`reflect::goals_for`). The model stopped planning
around 2026-08-28, so the field had been empty on every new reflection and
`goal_lessons` served nothing. Each run's own anchor is read, never the
session's last, and none where an answered `ask_user` later in that run
could have moved it. **2e-4a**, #336 (`08d165d3`, 18:20Z), under R40:
`success.rs` derives the owner-verified successes on every read — a draft
sent unchanged, a `done` closure no reopen undoes, a workflow close not
reopened, an answered question whose session then completed. There is no
success store, so a reopen withdraws by construction. Each draft sent
unchanged carries its verbatim exemplar with an `Origin`. It is shadow
only: `mecha sessions successes` and one line in `sessions appraise` are
its readers. **2e-5b and 2e-5c**, #338 (`b3135e1b`, 19:01Z), under R41:
`tenure` is the Wilson lower bound (z = 1.96; none under
`TENURE_MIN_VERDICTS` = 20; tenured at `TENURE_FLOOR` = 0.65) of the
owner-accept rate over the owner's verdicts on runs whose
`RunConfig::rule_ids` carried the rule. The owner's curation of rules is
not among them, since 1d keeps curation off a run's score. Tenure sits
beside retirement: `tenure::release_probation_when_owner_tenures` runs
after `release_probation_when_measured_clean` in `rules
propose-retirements`, the release lasts one pass (the file keeps the
probation mark), and retirement stays on measured regressions. A rule
whose region no admitted run in the recurrence window matched reads QUIET
in `mecha rules` (`tenure::Quiet`), as a report only.

Installed: #322 and #324 at 23:44Z on the 25th (`be373132`); #327 and
#328 at 01:14Z on the 26th (`720feb27`); #332 and #334 at 12:19Z
(`450a5cc6`, reported by the installing lane); and #335, #336 and #338 at
22:40Z (`b3135e1b`). #329's source is in every build from `39c9f359`
(another lane's 03:50Z install) on, but no `mecha` artifact was probed for
it until 22:40Z, so that is its first verified install. HANDOFF's *Machine
state, dated* holds what was checked for each and which were reported
rather than verified; the 22:40Z build, which carries every row, was
verified with `strings`. What the arc left open is in HANDOFF's
goal-system section.

**2026-09-25/26 — incognito chat, live: the server side (#321) and the
page (#326).** `INCOGNITO-DESIGN.md` is the authority; the 2026-09-25 entry
below holds the design and step 0. #321 (`62b5cf10`, six review passes) made
an unrecorded chat a type rather than a flag — a web session is
`Recording::Kept | Incognito`, and every write that records had to say what
it does instead — and put the chat in a room on tmpfs under
`$XDG_RUNTIME_DIR/mecha-incognito/<home>/<key>/`, withheld everything outside
an allowlist computed against the live registry, and refused at the door
what it could not honour: a non-loopback provider or one with fallbacks, a
deny-gate hook (`pre_tool`, `pre_task_close`), a runtime directory that is
not tmpfs. Review found the load-bearing seams one at a time: the jail had
to be named for the key or `ask_user` routed nowhere (or to the wrong
chat); a second `serve` against the same home swept the first one's live
rooms until each room carried its opener's pid; a routed builtin stayed
reachable because only mail tools were checked against the outbox; and
even a count at the default log level said a chat had existed, which R1
rules out. #326 (`720feb27`, five passes) is the page: a button beside
**+**, a banner with the search notice, **End**, no voice call, and a
screen saying the chat is gone on End or on the server's `410`. Two owner
rulings of the same evening shaped it: **an open page is use** (it pings
`/alive` once a minute; the idle clock runs once the tab closes), and
**`shell` registers in the room**, not `~/.mecha/runs/shells`. The second
was only safe once review corrected the argument for it: a room
registration is invisible to the closure check, so a command that clears
`MECHA_RUN_POSTURE` would pass as the owner, and the board is reached
through the graph server rather than a file — so `shell` is offered only in
a sealed sandbox (`incognito::shell_is_sealed`: no writes, no reads outside
the jail, no network), not merely a write-confined one. The page's own
passes found the promise leaking through the tab itself: the event stream
refilling a forgotten conversation after End, unsent text following a
drawer switch into a recorded chat, and an upload finishing after a switch
announcing its file there. Installed 2026-09-26 01:14Z (HANDOFF, *Machine
state*); a live chat through the tailnet door opened, pinged, ended and
answered `410` with its room gone.

**2026-09-25 — local image generation, and an incognito chat that leaves
no trace.** Two arcs from one lane, the second designed while the first was
in review.

*Image generation.* Qwen-Image 2.1 already ran in `~/ComfyUI` (GGUF Q4,
ComfyUI-GGUF); the question was the engine behind a mecha tool. A bake-off
against stable-diffusion.cpp on the same prompts settled it: ComfyUI 42 s
warm, 15 GB peak, legible sign text in 3 of 3; sd.cpp Q8_0 71 s and 21.5 GB,
Q6_K 80 s and 20 GB, text about 1.5 of 3 with gibberish — and sd.cpp
refuses the community GGUF outright (a tensor-shape check). The contract was
kept sd.cpp-shaped anyway, so the engine can be swapped under it: the model
supplies typed values, the graph is fixed in code, never the model's.
`image_generate` (#303, `5d613736`) posts to a **loopback-only** server
(no redirects, no proxy — review found a redirect would re-send the prompt
elsewhere), writes the PNG into the run's workspace, and the web chat shows
it inline. Edits (#306, `b1820b5d`) take reference images from the same
workspace, from an upload or an earlier generation, behind an Edit button;
**an edit always draws a fresh seed**, because reusing the reference's seed
redraws the reference. Qwen-Image's own settings are 40 steps at cfg 1.
**The first live test took the machine down** (04:12Z): llama-server's
~35 GB, a parallel cargo link and ComfyUI together on the GB10's unified
memory, where GPU memory *is* system RAM, and the kernel's OOM killer chose
llama-server. The tool now refuses below a free-memory floor, and the rule
for a session is to read `MemAvailable` before a generation or a big build.
ComfyUI became `comfyui.service` so it survives a reboot (HANDOFF,
*Standing machinery*).

*Incognito chat.* The owner asked for a web chat that is gone when it
closes. The design (#307, `613db4ce`, `INCOGNITO-DESIGN.md`) records six
rulings: R1 strictly invisible — no transcript, no content-free counts, no
replay, no learning; R2 no "save this conversation" escape hatch; R3 local
and may read the owner's data, never write it; R4 web search allowed, with
a notice before the first search; R5 a 30-minute idle close; R6 images
deleted when the chat closes, the image server's copies and the browser's
cache included. Its step 0 (#313,
`5fa722fe`) fixed what every chat leaked, incognito or not: tool-output
spills moved out of the workspace to `~/.mecha/spill/<sha256 of the
workspace>` (review found an in-workspace `.spill` could be a symlink out of
the jail, and a pre-planted link another way), the dropped-reasoning line
keeps only its content-free counters at `warn` — the 400-character tail
moved to `debug` with the whole trace, and the counters stay on purpose,
because an empty turn is in no transcript and they are its only
default-level record — and `reflect`/`distill` skip test sessions. Steps 1–3, the chat itself, are #321, unmerged at
this writing; HANDOFF holds where it stands, and its entry here is owed
when it merges.

**2026-09-24/25 — appraisal wiring, phase 1: evidence and context go in,
and what a run does changes in three named places only; phase 2 and 3a
follow.** `APPRAISAL-WIRING-DESIGN.md` (#291,
`66ae0abd`) re-derived the appraisal programme from what the owner said it is
for (the agent's own interpretation of meaning, serving self-learning, goal
alignment and planning across many goals) and recorded rulings R1–R29 in its
§6. Phase 1's rows then landed in one night, each behind its own tests, and
the three behaviour changes are named below (1b's refusal, 1d's reflection
arm, and 1e/1f's inputs to planning guidance). **1a**,
#292 (`a21bb086`): tasks, triggers and front-door requests anchor their runs
from structure (`run::seed_goal_anchor`; `GoalRef` gains `trigger` and
`request`, lenient on read), drift is judged only against anchors a plan can
name (`GoalRef::a_plan_can_name`), and `sessions health` reports
`runs_anchored_by_kind`. **1b**, #293 (`c76ccefd`): closing or reopening a
task is one recorded event (`closure.rs`, `~/.mecha/closures/`), with
`pre_task_close` (may deny, fails closed), `task_closed` and `task_reopened`
hooks; the web board and the TUI show the appraisal's readout, read back from
the record; a run with nobody present cannot close. **1b-2**, #294
(`dfbb2217`): who is closing is decided from a harness-written registry of the
shell children the `shell` tool spawns (`shell_registry.rs`,
`~/.mecha/runs/shells/`), not from `MECHA_RUN_POSTURE` (see *Traps*), and
`mecha doctor`'s `check_shell_confinement` reports a `[sandbox]` that mounts
the mecha home. **1i**, #297 (`d97556c3`), test-only:
`a_recorded_run_carries_no_sensor_number_setpoint_or_valence_to_either_encoder`
scans every request of a recorded and resumed run through both encoders, and
its control injects each needle; no leak was found. **1g**, #298
(`53b5a77c`): every steer-probe, validation and learn-gate verdict is stored
as a `comparison::Comparison` (`~/.mecha/comparisons/`), keyed by situation,
goal kind and call class, and written only for a clean session whose tool
surface is still readable. **1d**, #299 (`9935f974`): the verdicts the owner
already gives are read where they are recorded — closures and reopens (a
reopen of a `done` closure signs −1.0 at any age and withdraws its +0.5; a
reopened drop signs nothing), workflow
close, cancel, reopen and failed verify, and a draft rejected with a reason
(mined by the reflector in the owner's words) — and new `curation.rs` ledgers
record rule and harness-candidate curation, which never moves a run's score
(R16f–h). The one behaviour change, weighed at merge: `appraisal::of_session`'s
reflection arm reads `Reflexion::provenance_as_mined`, so dropping or editing
a reflection no longer moves the run's valence (R16g). Graph fact rejections
stay unreadable from mecha (`graph_fact_rejections: null`).

**1e**, #301 (`b3a45764`): a charter reading carries `reading::Items` and a
per-run `Flow` beside the level, because one stale draft had pinned the
outbox-age level past its setpoint on 126 of 126 runs; a line past its
setpoint on each of the last `SATURATED_AFTER_RUNS` informative runs is
withdrawn from in-run consumers (`Homeostat::in_run_readings`) and reported
once by the doctor. **1c**, #300 (`2cf19289`) and mecha-graph#21
(`ecf067c8`): Slack gains Drop beside Done, its reply carries the readout from
the record (`ClosureStore::move_since`), and every Slack task tap passes the
hidden `tasks set --only-open`, which refuses a stale card (the web board's
`POST /api/tasks/set` does not; HANDOFF banks it); the graph TUI closes and
reopens through `mecha tasks set --surface graph-tui`, adding `--only-open`
on a close, when `[board]
close_through` is set, and refuses with nothing written when mecha is missing
or the TUI is not on the default database (the owner's option A3; R30 in
`APPRAISAL-WIRING-DESIGN.md` §6). **1f**, #302 (`fe295e8b`): guilt is per commitment — each staged draft,
parked question and front-door request waiting on the owner carries
`reading::excess(age, patience) × guilt::weight(rank)`
(`guilt::read_commitments`, recorded per store as `guilt::StoreGuilt`);
`anticipated_guilt` became a readout, their maximum; and
`planning::Decision::assess` keys `ReviewCommitment` on the store's
per-commitment guilt rather than the level (guidance that reaches a run only
under `goal_guidance`, off by default; 1e's withdrawal feeds the same
decision). **1f-2**, #304 (`2f89ebb5`): new
predictions write the one `workflow::Commitment` record
(`Evidence::into_record`), old ones stay on disk in their own shape
(`anticipation::RecordedCommitment::Legacy`), and `due_at` / `follow_up_at`
became optional — absent means no deadline stated, and the harness never
supplies a date (R31 in the same table). **1h**, #305 (`6a26f7ab`): the situation brief
(`brief::SituationBrief`, nine fields, each with its own unknown) is assembled
with no model call on `tasks work`, `trigger run` and each web turn, recorded
on `RunStats::brief`, and delivered nowhere; building it found `serve`
recording every web turn against the backlog of the morning it started (see
*Traps*). Phase 2 began with **2a-1**, #308 (`e35bb081`):
`appraisal_store.rs`, the text-appraisal store, whose write door grounds each
claim through `grounding::admit` and whose clean door is a type only the store
can construct (`Clean`); it has no producer yet, and the graph episode's
prompt is pinned (`the_distillers_episode_prompt_is_pinned`). **2c-1**, #311
(`193b0114`), followed at 18:18Z on the 25th: the goal a front-end hands `prepare`
(`GlobalOpts::goal`) joins `Situation` as a recorded and scope key
(`situation::GoalKey`, the whole `GoalRef`), recorded as
`RunConfig::rules_goal` and read by every door the workspace and surface keys
read; an absent or unnameable goal never widens a scope, and none of the
live store's 67 reflections carried a goal at merge (#311's own read-only
count); the owner then ruled that such a rule stays scoped to its task and
widens only by evidence (R34). Three more merged at 19:47Z. **2a-2**, #314
(`5995aa26`): `mecha distill` appraises each session in a follow-up turn on
the episode call's own cached prefix (`QuarantinedPass::follow_up`,
`Distiller::appraise`), so `DISTILLER_SYSTEM` and the graph episode stay
byte-identical (R32, amending decision 4 to one extra call per session);
the record gains `ExpectedAct`, R16's closed set, for 2b-2 to score (R33);
the write door resolves each judgment's goal against `KnownPointers`,
deduplicates and caps `because`, and refuses a second appraisal of a
session; it runs only on a `kind = "local"` provider (R29), and `mecha
sessions appraise <session>` is the owner's readout. **2d-1**, #312
(`87c0e9a1`): `mecha sessions compare` drives up to three policies
(`pointwise::ARMS_MAX`) four turns (`HORIZON_TURNS`) from each informative
decision point — a steer, a denial, an edited or rejected draft, a failed
check, a surprise — lets a structural validator read the owner's recorded
act, and stores one 1g `Comparison` per point; a point no structural
validator can pose is stored `unposed` with no arms. **3a**, #309
(`6e6f03ba`): `brief::render` turns 1h's record into words and bands per
R21, and `Agent::fold_situation_brief` puts them in the run's first user
turn, never the prefix, behind `Lever::SituationBrief`, which ships off;
`mecha run` now records and delivers a brief too, so an experiment's lever-on
and lever-off arms are two conditions, not one. The
owner then ruled that delivery must arm `private` (R35). **3a-3**, #316
(`04b89ea0`, 21:00Z), built it: `Agent::fold_situation_brief` reports
delivery and the loop arms `private` at each fold site, and
`Taint::arm_for_content` arms from any transcript holding a brief (the
attached image's precedent); and a fold is now a `Record::Extend` of the
message the door already recorded rather than a whole-transcript
`Record::Rewrite` that cleared the taint checkpoints, the calendar
reference's fold included. The lever still ships off. **2c-2**, #320
(`6f8e69ca`, 21:36Z): `goal_context` serves up to three past clean
appraisals of the run's situation and goal, on demand and never pushed,
selected at setup through `Clean` only (`appraisal_store::PastAppraisals`)
and behind `Lever::PastAppraisals` (`[agent] past_appraisals`), which ships
off; the measured run against a control is still owed. **2b-1**, #319
(`8c0f5a9d`, 21:46Z): anticipation's owner-evidenced predictions are scored
as their outcomes resolve them (`anticipation::Calibration::of`) — coverage
per kind and no rate over nothing, harness placeholders counted apart, and a
delivery positive only on confirmed delivery. The live store held no
owner-evidenced prediction at merge (the coordinating session's count).
**2a-3**, #315 (`0692dc79`, 20:32Z): the counts-only appraiser
(`appraise_with_model`) is retired into 2a-2's text appraisal; `sessions
appraise --appraise` stays as a hidden, deprecated no-op, and old records
carrying its errors still load and count. With it `Affect::Anger` lost its
only live producer (the appraiser's `other`/`world` verdict); `label_of`
still derives it, so older records read as before. #313 (`5fa722fe`,
another lane's) made `reflect` and `distill` pass over test sessions, so
neither the reflector nor 2a-2's appraiser reads a `MECHA_SESSION_KIND=test`
run. #317 (`8b0acbe8`, 20:44Z) built R34's readout: a rule scoped to a goal
that has closed keeps its scope, and `mecha rules list` marks it `LOADS
NOWHERE` with why, `mecha learn` repeats the count each pass
(`learning::ClosedGoals`), and an unreadable board is its own finding.
Separately, at 03:29Z that morning, #295 (`c4c916d1`) made `[sandbox] memory_mb` and `cpus` real
under bwrap through a `systemd-run --user --scope` (`Sandbox::bwrap_launcher`)
and made landlock refuse them (see *Traps*). What the arc left open is in
HANDOFF's goal-system section.

**2026-09-24/25 — the docs, checked against the code, and the code that
failed its own docs.** The owner asked for a clearer account of appraisal and
then for the whole site to be organised and audited. #273 (`37af1a52`) added
*How appraisal works* (the harness background, the pipeline as a figure, the
label as a decision tree, every signal's sign and agency against
`appraisal::of_session`, and what has and has not been measured) and split
the 1,235-line appraisal page into charter, goals, plan steps and
anticipation. #278 (`9eea04e7`) regrouped the features into ten sections, with
the graph under Memory and the factory as the public surface, every old URL
forwarded by `@docusaurus/plugin-client-redirects`. A seven-lane audit then
checked about a thousand claims against the code. #280 (`ef594716`), #283
(`06a408e9`) and #288 (`84693af3`) corrected what it found, moved Rust listings
and incident stories into ARCHITECTURE.md (two new sections: session records
and replay, voice preferences), and added pages for delegated tasks and
messages between agents. The wrong claims that mattered most were security
ones: the security page and ARCHITECTURE both said a subagent starts clean
(`Subagent::call` seeds its taint from `ctx.taint`), the replay page said a
replay may be less armed (unknown provenance counts as external,
`replay_run`), the intro said the graph is encrypted at rest (it is opt-in),
and the docs' outbox lists left `factory__surface_push` unstaged.

Some findings were code, not prose. #279 (`c9460213`) expands a leading `~` in
every config path and in `[[mcp]]` `command`/`args` (`ConfigLayer::expand_home`),
because the mail and documents pages' own snippets named a literal `~`
directory and the servers never spawned; fixes `mecha setup`'s documents
remedy (`mecha-docs auth personal` failed); and makes `/entity` and `/mail`
visible in `/help`. #289 (`6bfde499`) closes a race in the web voice-clone
upload, which promised never to overwrite but checked and then renamed
(`place_new` hard-links into place). In mecha-graph, #19 (`1b8dbf99`) retired
the `pkg` name everywhere a user or the model reads it (the `kg_shadow_queue`
note told the model to run `pkg shadow`, which does not exist) and fixed the
TUI editor writing to the pre-rename `~/pkg`; #20 (`883be7b2`) made clippy
clean under `-D warnings`, added the repo's first build-and-test CI with a
pinned toolchain, and removed DuckDB from the docs because nothing uses it.
Three audit findings were wrong on checking and were not applied
(`context_window` does not feed overflow recovery; `mecha msg` has no `read`
or `who`; the documents TUI paste strips all whitespace).

**2026-09-24 — the outbox had clogged, and the reason was one missing
argument.** #272 (`97188f8b`). The owner reported the web outbox could not
approve a draft, that approving took two presses, and that Send did nothing.
All three had one root: six staged `mail_reply` drafts named no `account`
while two were configured, mecha-mail refused each with "pass `account`"
before any provider call, and the release path — which records the attempt
before dispatch so a crash can never retry blindly — kept the refusal as
*unknown*, which hides every action on the item. The error did render, at the
top of a scrolled pane, far from the button. The owner's ruling: a reply goes
out from the account it was received on, so an omitted account is looked up
(`thread_home`), not demanded.

Five review passes each found something the last had not, and three are
worth keeping. **An empty read is not a holder**: Graph answers a
`conversationId` filter that matches nothing with HTTP 200 and an empty list,
so the first version made every Outlook account hold every Gmail thread — a
reply refused as "found in several accounts", and a triage acting in the wrong
mailbox when the real holder's read failed. **A staged reply has no `to`**, so
the pane's "Replying to" was the only statement of the recipient, and a body
can type a whole header block; mecha-mail now ends every thread read with a
count (`thread_footer`) on the one line no body can reach, and a split that
does not match — or a read clipped at `outbox_source::MAX_CHARS`, which looks
exactly like a one-message thread — is shown verbatim and names nobody.
**`mail_reply` answers the newest message when it is released, not the one
the draft was written to**, and the queue's whole premise is that drafts wait:
the pane says so, and the follow-up rereads the thread on open. The armed
confirm sheet was removed on the owner's instruction; what it showed that the
page did not, a link's destination, is drawn inline with its query string,
because `shortUrl` folding a query to `?…` hid exactly the exfiltrating case
the first version claimed to reveal.

Two traps on the way. The live check caught what the demo could not: four of
twelve real drafts read their thread before mecha-mail wrote a `Calendar
date:` line, and parsed to nothing. And `pkill -f "<pattern>"` run inside a
shell whose own command line contains the pattern kills that shell — twice;
start a dev server with `setsid … & echo $!` and kill the process group.

**2026-09-24 — the calendar contradicts a stale date instead of confirming
it.** #238 had made the harness's own clock unable to go stale, and left the
other half open on purpose: on 2026-09-14 a run working from a stale date
asked `calendar_list_events` for `time_min: 2026-09-13T00:00:00-04:00`, the
calendar answered that window faithfully, and the owner had to correct the
date twice. #243 (`9308c00b`) moved the decision to where a clock is real.
`time::resolve_bound` takes `now`, `today`, `tomorrow`, `yesterday` and
`±Nd` and keeps three outcomes apart in `Resolved` — `At`, `Passthrough` for
every RFC 3339 stamp that worked before, and `NeedsZone`, a refusal by name
rather than a guess; `this week` is deliberately absent, since where a week
starts is a convention the server cannot read. Every window answer carries
`time::window_note` — the window and `as_of`, the server's instant and zone —
so a wrong premise is contradicted by the first result. Two review passes
found real defects: a day offset past chrono's range panicked the server
process (`parse_day_offset` now bounds it by range containment, because
`n.abs()` itself panics on `i64::MIN`), and the refusal was unreachable on
the deployment it was written for (the trap below). The fix to that split the
zone: `time::window_zone` reads `MECHA_TZ` alone, `configured_zone` keeps the
`TZ` fallback for rendering. The owner then ruled the zone a configuration
parameter rather than a hand-copied `env` line per server, so
`McpServerConfig::owner_zone` — `serde(skip)`, filled by
`Config::hand_zone_to_servers` from the running config after `load_layers`,
after `TrialEnv::base_config` copies live servers in, and after
`Fixtures::apply` — hands every server `[agent] timezone` as `MECHA_TZ`, and
`mecha setup` gained a `timezone` step shown once a server is wired. #267
(`a5140ed2`) took two of the last pass's minors: the omit-both default now
renders in the window zone, and `time::relative_time_schema` puts the
vocabulary on all three servers' `time_min`/`time_max`, with
`time::assert_window_schema` measuring it per server (reverting
`mecha-google`'s schema fails its test). Deployed the same day; the
redundant `MECHA_TZ` line in the live config is the owner's to delete.

**2026-09-18 — a claim cites what the run received: one walk for what a run
was shown, and three surfaces that check the model's words instead of
trusting them.** Prompted by "other harnesses have a verifier — what are they
doing?". The literature pass (`docs/VERIFICATION-RESEARCH.md`, 2026-08-04)
had already refuted self-critique as a completion gate; the harness survey
added as its second pass found the half that survives: Claude Science's
reviewer is never asked "is this good?" — it is asked whether a cited number
appears in its source and whether a figure matches the code that claims to
have produced it, questions with a *referent*, and the provenance is the
mechanism. Reading for what mecha already had turned up the same primitive
hand-rolled four times (gossip's `grounded_claims`, `outbox_source`'s "what
is this draft answering" join, `diagnose::carries_over`, the mismatch
pointers) and one trap: `evict_superseded_results` rewrites a result *in
place under its own `tool_use_id`*, so one id names two contents, and
`replay::extract` hands the `[stale:` marker back as a call's output — gossip
escaped only because its JSON parser looks for a brace and the marker has
none, while `outbox_source` had learned first-seen-wins by handing a reviewer
the marker as "the message you are answering". #244 (`569d4952`):
`grounding.rs` — `calls` lists every call once with `result: Option`, first
seen wins, a stale marker never occupies the slot; `admit` dereferences a
`{statement, id, quote}` claim into a packet by literal containment and names
what was missing; `carries_over` is the inverse polarity, moved from
`diagnose`; the three callers moved onto it with their own thresholds as
parameters, and nothing enters the registry, because a check the model may
decline to call is not a check. #245 (`d6fc8268`): `frontdoor::Record::for_privileged_run`
— the function the front door names as "the boundary of the quarantine, and
the reason it is a function" — hands over only the `dates_mentioned` that
`admit` grounds in the record's own `prose()`, floor 3 (`"3/5"`, `"May"`)
and ceiling 48, because containment is an anti-fabrication check and an
instruction copied verbatim is a literal span too; `Record::ungrounded_dates`
carries each refusal's reason to `show` and the TUI through one
`Record::date_finding` phrase per reason, a finding and never a block. The
Slack request card is built from the same brief, so it inherited the rule
without knowing — its fixture had to start actually saying "next Tuesday",
and the same test now asserts an invented date never reaches a card. #246
(`ddd302f4`): the mail classifier is asked for `deadline_quote`, the words a
deadline comes from copied verbatim; `ground_deadline` runs inside
`classify_with` against the subject and body the model was shown on both
passes — the triage store keeps no body, so it is checked there or never —
with floor 4 and ceiling 120; a deadline whose words are missing, absent, too
short or too long is dropped and recorded as `Verdict::deadline_refused`, a
closed enum with `#[serde(other)] Unknown` for the wire, written by the
harness after parsing on every path so nothing the model emits there
survives; `mail list`, `mail show`, the sweep line and `mail task` say
"dropped", never "found none", because those are opposite findings; an
owner's `--deadline` correction settles the refusal whether it overrides or
agrees, reaches the store either way, and a changed date sheds the
classifier's words. `deadline_quote` and `deadline_refused` never cross
`for_privileged_run`, measured by `the_privileged_view_carries_no_prose`
planting both. Eight review passes across the three PRs; every real defect
was at a seam rather than in the check (*Review process*, below). Released as
**v0.1.21** (`a5275980`, "a claim cites what the run received") and deployed
the same evening — HANDOFF's dated machine state has each surface. **Left
deliberately unmeasured, and named as the thing to read first:** how often
the local model quotes verbatim rather than paraphrasing, which is how often
an honest deadline or date is dropped; both labels exist so the first real
sweep shows the rate, and a hard line-wrap is a second cause the
`QuoteNotInMessage` count cannot tell from a paraphrase. Also corrected in
passing: `VERIFICATION-RESEARCH.md` had claimed `expect.verify` hashes the
test file — nothing in `mecha-core` does (no `sha2` import); its tamper
resistance is that the command is owner-authored and the case must be
sandboxed, and `mismatch.rs` is the module that actually keeps gold outside
the workspace. The gossip *exchange* was left alone on purpose: the
2026-09-10 pilot found wrong-person claims with valid quotes and no extra
coverage for its cost, and generalising an unvalidated mechanism is the wrong
move — what generalised was its admission filter, which was the part of the
pilot that worked (four explicit no-evidence answers instead of four
fabrications).

**2026-09-17 — the third leg of the trifecta is a class, not a bit: who picks
the recipient is what the interlock was always about.** The complaint was that
reading mail ended web search for the rest of a conversation, and the
frustration was right even though its premise was not — a search query is
outbound, the payload fits in `?q=`. What `external_send` could not say is
**who receives it**, which is the question an exfiltration attack actually
turns on: the attacker has to read the bytes back, and that needs the attacker
to choose the recipient. `WebSearch::input_schema` is `query`, `limit`, `depth`
and no destination field. The duty cycle was the other half: `McpTool` declares
`private_data: true` unconditionally (no annotation can say otherwise) and the
operator's config forces `untrusted_input` on graph, mail and docs, so a single
`kg_search` armed **both** legs in one call, and taint never disarms. #241
(`14f35c91`) replaces the field with `Egress` — `None < Blind < Chosen`,
`union` a max, so an override still only ever widens — and splits the two
controls that already existed: the interlock fires on `Chosen`,
`block_sends_after_private` on both, which is the division `config.rs` had
described in its own doc comment (*"an ordinary privacy leak rather than an
attack"*) while `web_search` paid under both. `Blind` is earned in code by a
schema with no destination and has deliberately no TOML spelling;
`SearchBackend::egress(depth)` defaults to `Chosen`, so searxng and tavily are
blind at both depths while **exa is blind at quick and not at deep** —
`deep-reasoning` is agentic research that fetches pages the query steers it
towards. An armed conversation degrades to `SearchChain::search_blind` rather
than losing the tool, and `Subagent::new` taking the max of its children's
classes makes a search-only research child delegable again, reopening the route
`config.rs` recommended and the 2026-09-02 taint-inheritance fix had closed.
Measured rather than argued (§6b of `docs/EGRESS-DESIGN.md`): session
`20260917T035905-f0ebc369` on the local model records taint
`{private: true, untrusted: true}` after one `kg_search`, then `web_search`
executing, served `(via searxng)`, answered in one call with no retry.
Installed and the five long-running units restarted the same day.

**2026-09-16 — a clock reading is not a standing instruction: the date is
asked per turn and folded into the turn.** On 2026-09-14 a 09:21 voice call
was told it was Sunday the 13th, queried the calendar for that day, and read
yesterday's schedule back as today's; corrected twice, the model's own
thinking sided with its system prompt's stale date over the owner. `prepare_tools`
rendered `date_context::render` once and `Agent::new` froze it into
`Agent::system` — correct for a one-shot and wrong for a daemon, and
`mecha serve` holds one `Arc<Agent>` for its whole lifetime, that process
having started at 22:37 EDT the night before. Session
`20260911T130932-abcde902` carried a date two days stale the same way; the
trigger runner never did, because it builds an agent per run, which is the
control that isolated process lifetime as the cause. #238 (`42c359f1`) makes
the clock a trait object beside `Provider`, `Tool` and `Approver` —
`clock.rs`'s `SystemClock`, `FixedClock` for an experiment's fixture instant,
and `TestClock` a test can advance — and `Agent::fold_calendar_reference`
folds the reading into the outgoing user message whenever the local date it
states has stopped being true. **The decision is an equality check against the
rendered block**, byte-identical for every turn on one local day, so there is
no refresh cadence to get wrong, no per-conversation field to reset on resume,
and a cut that drops the block re-acquires it on the next call. It rides in
the block the moving `cache_control` breakpoint re-pays every request anyway,
and the system prompt stopped changing daily — which ends every long-lived
process re-paying its whole prefix at midnight. `date_context::GUIDANCE` keeps
the standing half, names no date, and concedes the date to *the person in the
conversation* while excluding documents, web pages, email and tool results,
because `Role::User` carries all of those. The fold is gated on the agent's
prompt carrying `GUIDANCE`, so `gossip`'s readers, `vet` and every subagent —
all of which overwrite the system prompt and never had a date — keep the
surface they had. **Six review passes found twelve defects, all but the last
two in readers rather than in the fold**: code that had come to depend on the
date living in the prompt. `mismatch::drive` was an unclocked fourth replay site
(`RunConfig::clock` now records the reading, `clock::for_replay` pins all
four); `learning::locate_followup` and `counterfactual::locate_steer` compared
`Message::text()` against text mined per block, so a correction typed on the
first turn of a new local day mined fine and located nowhere
(`agent::owner_text` is now the one definition all three read through);
`title::is_derived` prefix-matched untrimmed text while `compact::rebuild`
writes both sentinels behind a leading newline pair, so a compacted session
rendered its summary and verbatim carried tool output inside the owner's
bubble; a summary could cut the fresh reference away and leave the head's
older one as the newest; `probe::prepare_mismatch` gated on whole-`Message`
equality and failed every new recording while blaming the fixture;
`render_for_summary` labelled the reference `[user]`;
`eval::grounding_evidence` read the date off a prompt that no longer names
one, so the judge lost the context it is told to use as evidence; the
overflow-recovery arm re-sent inline without re-folding, reachable *only*
because `rebuild` had just been taught to strip the head's copy; and the
fold's own walk counted a model's echo as the harness speaking. The last two
were consequences of fixes made earlier in the same review, which is the
argument for running the loop to a clean pass rather than stopping when the
list looks handled. Deployed the same afternoon — installed from `main`, the
four long-lived units restarted at 19:44Z, and the voice facade answered
"Wednesday, September 16th, 2026" against a host clock of 15:45 EDT.
**Session files change shape**: the first run of a conversation, and the first
run of each new local day, records a `Record::Rewrite` rather than appending,
because the fold edits a message the session already wrote; that record
carries the whole message list and clears `taint_checkpoints`, which
over-taints rather than under. What it left is in HANDOFF — the tool-boundary
half, and PR #239.

**2026-09-16 — the OAuth client published, and the seven-day re-consent
ended for every grant issued after it.** `FlowMail`, the Google Cloud
project behind the `personal` mail and calendar account, had sat in
**Testing** since 2026-08-18 on the understanding that production needed
verification plus a CASA security assessment (~$540/yr), because
`gmail.modify` is a restricted scope. The premise was wrong, and the owner
reopened it on 2026-09-15 against Google's own *OAuth app state overview*:
an app may publish to production **without** verification, paying a 100-user
lifetime cap and an "unverified app" interstitial — but not the seven-day
refresh-token expiry, which is tied to *Testing* status specifically rather
than to being unverified. It published. The project is In production
(External, 5/100) with **scope verification deliberately not submitted**,
and branding verification passed the same day, once `mecha-factory.ai` was
verified in Google Search Console as a Domain property — one apex TXT row,
typed by hand because that zone's DNS has no API. Branding also requires a home page, a privacy policy and a terms-of-service
URL on that authorized domain, which is what `website/src/pages/privacy.md`
and `terms.md` are for (#234): a consent-screen prerequisite rather than
marketing pages, and pruning them as unused would break the consent screen
with nothing to say why. What this does *not* settle is in HANDOFF: the
seven-day clock belongs to the grant rather than the app, so the `personal`
grant minted 2026-09-15 keeps its own expiry and still owes a re-consent.
(Closed on 2026-09-30: the 2026-09-17 re-consent outlived seven days; see
that entry.)
The console states were observed by `mecha-41` and are not verifiable from a
shell; the DNS row and the three branding URLs were re-checked
independently.

**2026-09-14 — the owner can say yes by voice, and a bad link delays the
owner's words instead of losing them.** Two arcs, one day, both measured
before they merged and both deployed the same day.

*#228 — the spoken yes.* On 2026-09-13 at 19:37 UTC the spoken outbox
confirmation played its first real offer and the owner answered *"Go ahead
and send it."* — two phrases the parser accepted, joined by a word it did
not — and `parse_answer`'s whole-utterance equality dropped it; the
question was dropped with the words, and the model, whose tool result had
said *"review it with `mecha outbox`"*, told the owner so four times. Of
twenty natural accepts tried that day, one matched. `review_policy::segment`
now tiles the utterance with a closed lexicon and five connectives: every
word consumed, all phrases one kind, a tiling that ends on `and`/`just` is
a sentence cut short. A dropped question is carried over the model's reply
and asked again once (`Confirmations::carry_unanswered`); the staged-draft
result names this surface's review (`ToolCtx::review_hint`, messages only —
a publish is reviewed on a screen everywhere). Nine review passes; the ones
that mattered: `do it later` (a listed deferral) made unreachable by a
mixed tiling outvoting the pure one; `normalise` and `spoken_words`
disagreeing on the apostrophe, which reopened the filler hole for exactly
the contractions the branch added; the hint promised aloud on a publish.
`docs/VOICE-APPROVAL-RESEARCH.md` holds the incident, the measurement (51
sessions with spoken turns; 12 drafts from them, every one resolved on a
screen) and the design for the rest.

*#231 — the reliable uplink.* The 2026-09-14 morning call stalled eighteen
times and Parakeet received eleven seconds of a five-minute call — every
fragment right, every truncation a stall; RTP does not retransmit.
`docs/VOICE-LINK-DESIGN.md`, designed and built the same day: the page
taps the *encoded* Opus frames at the RTP sender (`RTCRtpScriptTransform`
— never the microphone track, which WebKit punishes by disabling echo
cancellation), keeps them in a ring on a media clock that runs across
reconnects and outlives the session object, and ships them as base64
batches over the RTVI channel (pipecat claims every data channel as RTVI,
so a second one would have hijacked it); the worker reads and discards
RTP, decodes the batches into the transport's own audio queue at four times
real time, and holds the turn until the page's backlog and its own are
drained. Audio older than two minutes on arrival — or from before this
connection began — is one late message, segmented on silence and appended
to the context to run at once. Both ends watch for a dead tap for the whole
call with three witnesses (RTP arriving, the channel alive by heartbeat, no
batch), and a stalled link cannot trip it. Measured with
`scripts/voice/test_call.py`: a 6 s stall mid-sentence, every word in one
turn; a 125 s block, the backlog as one late turn the model ran on within a
millisecond; a channel never fed, RTP fallback at six seconds; a dropped
connection, eight seconds captured with no connection delivered first.
Seven review passes found real things on every one, all in the fallback
seam and the late lane, and the design records each with its reason. What
the fakes could not find, the live runs did: a burst outruns pipecat's
VAD→STT loop; Parakeet-TDT discards what precedes a silence gap inside a
clip; a transcript-only turn waits out a 15 s timeout; aiortc counts RTP
packets only on consumption. Deployed ~16:30 EDT (20:30 UTC) — `web/dist` with a new file at its root,
`mecha-serve` and `mecha-voice-worker` restarted.

**2026-09-13 — a gap in the audio is not silence: the voice call holds the
turn across a stall and says so, and the owner asked for the sound.** Two
calls from a moving car on 2026-09-12 reached the model as six fragments —
a request cut into pieces of a few words each, several mid-word — each
answered with a clarifying question, no
tool called, no task captured, and the page said "listening" throughout,
because the browser's `disconnected` needs a longer gap than the two-second
ones that did the damage. The journal named three mechanisms
(`VOICE-RESEARCH.md` §7, the 2026-09-12 entry): a packet gap reads as the
owner going quiet to every stage downstream, and pipecat's input track
*discards* the frames that arrive late; the worker's own event loop stalls
~2.5 s on the first turn of every call since 2026-08-25, cause unnamed; and
both calls went 40–80 s hearing frames with no speech in them, consistent
with iOS muting the mic on screen lock. #226 (`9ae2a00e`) ships `LinkWatch`
behind the transport — 0.75 s without audio holds every pending
end-of-turn, ahead of pipecat's forced VAD stop at 1.0 s, and 0.6 s of
*continuous* audio lifts it — with the discard relaxed to a minute, the
aggregator's backstop raised to 15 s as the ceiling on a held turn, and
`LoopSampler`, a thread that writes the main thread's stack the moment the
loop's heartbeat is half a second stale. On the page: two soft notes down
and two up (the tone debounced 700 ms; D7 amended), `linkVerdict` over
WebRTC's own counters with unknown never a stall, the mic track's mute edge
as the same pause, and a screen wake lock. The dictate button got a
listening tone, live level bars, and a refusal for the empty clip it used
to send (2 of 8 that afternoon 500'd Parakeet, which now answers 400).
**The review loop ran ten passes and found eight real defects, every one in
the pause *recovery* path** — the page echoing the worker's pause back into
a deadlock; a settle that one stray packet lifted; a pause mid-inference
resuming into "listening"; a lost `ok` latching either side; a mic hold
expiring on the silence a muted iPhone keeps sending; the worker pinning
its hold to whichever page reason came first — and none reachable by the
unit tests or the end-to-end smoke, which drove the worker-first ordering
only. Each fix landed with a test that fails on the behaviour it replaced;
the pass-nine finding that the backstop's recorded reason was false is
corrected in the constant's own comment. Deployed the same night on all
four surfaces (HANDOFF, machine state, 2026-09-13). What it deliberately
left — the loop stall, the gate, the capture wording — is in HANDOFF under
the voice section.

**2026-09-11 — a probe whose recorded tool surface is gone is refused, not
retried forever.** `validate` marked every replay failure retryable and
`should_run` re-runs anything retryable, so three of the seventeen held-out
reflections — naming `pkg__kg_entity` and `google__calendar_create_event` from
servers retired on 2026-09-04, in recordings made before the surface store
existed — were re-probed every night to reach an identical refusal, each
spending a ledger row that can never age and a `--cover` slot that buys no
coverage. **The first write-up of this said "two model calls each", which was
reasoned rather than measured and is wrong:** `replay_registry` runs before
`provider::build`, so the refusal was always free of provider calls. The three
receipts from the 2026-09-11 re-run are stamped 13:34:59 and 13:35:03 twice,
while the genuinely driven probes ran 13:35:43 to 13:47:10 — four seconds
against twelve minutes. A cost figure nobody measured outlives the session that
could have checked it, which is why it is corrected here rather than quietly
dropped. The distinguishing fact is in the recordings and was checked
rather than inferred: those three carry no `tools_hash` at all, while the one
follow-up that does grade is the one recording that has a blob.

Nothing was broken live. No `pkg__`/`google__` reference remains in
`config.toml`, the skills, the triggers or the charter, `[outbox] tools` is all
`mail__*`/`docs__*`, and no service warns at start; the 2026-09-04 rename was
completed properly in operations. What the rename left behind is only in
append-only history, which is the one place it cannot be corrected.

Measured on the live corpus in an isolated store copy: 17 probes attempted
before, 14 after, with the three refused at preparation before any model call.
One recording had lost 23 tools — the whole `google__*` and `outlook__*` stack
as well as `pkg__*` — because it predates the mail unification too.

**What the fix is not: a rename map.** Pointing `pkg__kg_entity` at today's
`kg_entity` would make those three gradeable again and would grade a rule
against a tool surface the recording never saw. Manufacturing a verdict is
worse than recording that the corpus lost one; if that trade is ever wanted it
belongs in an owner-declared alias map, where a wrong mapping is a declaration
rather than an inference.

Review found the same defect one caller over, and it was the more expensive
one. `appraisal_probe` charged its arm budget and counted `Tally::driven`
*before* `drive_arm`, and a lost surface fails at `replay_registry` before
`Agent::new` — so a corpus-wide allowance documented as "consumed by drives,
never by skips" was being spent on model runs that never happened, and filed in
`Tally::unavailable`, whose doc says "Fixable". Checking `harness_probe` and
clearing it correctly (its corpus is recent sessions, which age out) was not the
same as checking every caller of the prep.

**Seven review passes, and four of the findings were against the change's own
description and tests rather than its logic.** A doc citing a test name that
existed nowhere. The cost figure retracted above — reasoned rather than
measured, wrong, and repeated in six places including this file. A counter
incremented and printed by neither readout, which made a fully-lost corpus read
as zero on every line. And twice a test pinned the producer while the consumer
dropped the value — `Tally::add` while the readout dropped the summand, then a
readout test that serialized its own `Tally` and never called the readout.
Three more were one mistake repeated: an insertion anchored on `fn name(`
landing between an item and its attribute or its doc block, in a language where
both bind downward.

The mechanism was right early. What kept being wrong was everything that
*describes* the mechanism — and that is the half which outlives the session
that could check it.

**2026-09-11 — the nightly measurement fixes shipped, and re-running the night
showed last night's two directional findings were both artifacts.** They were
written on 2026-09-10 and left uninstalled, so the 2026-09-11 nightly ran the
old instrument and reported `1 improved, 1 regressed, 8 unchanged, 7
inconclusive` over its seventeen held-out reflections. Re-running that exact
stage on the new binary, against an isolated copy of the learning store
(`MECHA_LEARNING_DIR`, `MECHA_SESSION_KIND=test`; the live ledger stayed at 234
rows with no attempts file), returned `0 improved, 0 regressed, 9 unchanged, 8
inconclusive`.

Both of the night's directional verdicts dissolved, and only one of the two is
attributable. The regression on `20260820T145930-634649cc` was a replay
artifact — `structural divergence at call #7: Missing "docs__sheets_read"` — a
cause the old code could not detect and therefore graded anyway; that one the
fix is responsible for. The improvement on `20260906T033546-21af0876` became
`unchanged (both pass)`, which a judge-graded arm can do on its own; one re-run
cannot separate the fix from sampling there. The fix's intended effect does show
once: `20260906T033607-c958adbc`, previously ungradeable, came back graded as a
followup.

A third run the same day settled the second case and is worth recording against
the first reading of it: `20260906T033546-21af0876` graded IMPROVED, then
unchanged, then IMPROVED again over three runs of identical inputs, and
`20260906T033607-c958adbc` graded and then diverged. **Judge-graded followups
move run to run, so a single re-run is not evidence about a directional
verdict** — it is only evidence about a structural one, which is why the
regression above is attributable and the improvement is not. The honest summary
of the night is one finding corrected, not two.

**The count of inconclusives did not fall — it rose by one — and that is the
improvement.** The old instrument gave all seven the same explanation, "the
baseline answer has no gradeable text (the followup probe offers no tools)",
which named the probe as the culprit. The new receipts name causes and say
whether they can be retried: three `replay_error` (retryable — a recorded tool
is gone from today's surface) and four `replay_divergence` (not retryable — the
model left the recorded trajectory). An unmeasurable probe honestly labelled is
worth more than a verdict, which is the same lesson as the gate that was put on
`compactions` where `context_overflows` was the counter.

**Two of the retryable three cite `pkg__kg_entity`,** a tool from the MCP server
retired on 2026-09-04. Those reflections cannot be validated again until the
recorded surface is remapped or they are retired, and the old instrument was
reporting that stale surface as a result about the model's behaviour.

**2026-09-10 — native gossip comparison and coherent experimental control.**
`FollowupMode::OwnEvidence` gives each question generator only its own reader's
cited answer and routes its question back to that reader. Its matching role prompt
asks for self-followups; the ordinary `asker` and `exchange` keep peer behavior.
The native `gossip_compare` example executes real read-only MCP retrieval over
frozen synthetic source worlds. The registered runner records full request/search
receipts and prepares an arm-blind claim audit, comparing equal request ceilings
and reporting actual cost. Regression tests exercise evidence isolation, question
routing and default behavior, real MCP search boundaries, and report accounting
for repeated facts, contradictions, missing judgments and changed conditions.

All 16 valid exchanges completed. Peer coverage stayed at 23/42 source items from
round 1 through round 3; own-evidence followups reached 24/42. Seven pairs tied and
one favored the control. Extra peer rounds cost 104 requests and 108,668 output
tokens without additional supported coverage. Assistant blind review of 55 distinct
admitted claims found 36 supported, 12 unresolved and seven contradicted; literal
citations did not prevent namesake misattribution or unsupported role elaboration.
Two ambiguous identity contrasts were conservatively marked unresolved before
unblinding. The valid archive retains one token-limited control answer and all
actual costs. No default changed or deployment occurred. Full checks passed:
2,628 tests, zero failures, three ignored, warning-free Clippy, build and formatting.
See the complete scorecard (`results/gossip-comparison-qwen36-35b-20260910/README.md`, local since 2026-10-04)
for denominators, controls, limits and the complete synthetic evidence trail.

The setup caught two measurement traps before semantic grading. Removing the
peer's evidence without changing an instruction to ask about the peer's different
sources gave the control an inconsistent task. Also, running workspace tests
rebuilt the actor at its Cargo output path after registration, despite unchanged
source. Those attempts remain invalid and preserved. Freeze the executable actually
run, not just a hash of a mutable build path; an immutable design needs immutable
execution inputs. The corrected runner copies its actor before dispatch.

**2026-09-10 — executable corrective-task pilot completed with real tools.**
The existing native experiment runner drove 96 trials over eight synthetic tasks,
three seeds, frozen rules off/on and max-turn limits 12/10. Every arm started from
the same registered incorrect artifact in a fresh workspace, and the independent
oracle read real output files and preserved inputs without prescribing a tool trace.
The cap comparison favored retaining 12: 19/24 versus 17/24 with rules, 20/24 versus
18/24 without them; each comparison lost two outcomes and gained none at 10.
Both-passing pairs also used more calls and turns at 10. Frozen rules produced one
improvement and two regressions at each cap, not a demonstrated learning benefit.
Execution hashes stayed fixed. A reporting-only empty-rule-hash correction reran
no trials. Build, formatting, Clippy and required-backend tests passed (2,626 passed,
zero failed, three ignored). The native CI fixture test also runs the four Python
oracle/report checks. No production state changed. See
the scorecard (`results/executable-validation-qwen36-35b-20260910/README.md`, local since 2026-10-04) for
conditions, task-level results, private-snapshot boundaries and remaining limits.

**2026-09-10 — nightly measurement and gossip grounding, implemented in an
isolated worktree; not deployed.** `followup_branch` and
`probe::drive_continuation` retain the correction, restore recorded tools and
reject incomplete or argument-divergent prose before judging. `ValidationAttempt`
records input identities, reasons and arm receipts, defers unchanged inputs before
coverage selection, and preserves explicit repeats, transient retries and regression
confirmations. `ConfigChange::ensure_supported` rejects ignored effort controls;
`ArmOutcome::from_report` excludes missing tails even without cursor cancellation
and preserves receipts. Diagnosis routes MCP capability changes as security work.
Gossip carries resolved identity and structurally checked episode/quote claims,
keeps raw answers for audit, records citation yield and exposes filing failures.
A citation is not semantic proof. One bounded two-reader/two-round live smoke
produced four explicit no-evidence answers and no verdict filings; its 16 retrieved
hits per round supplied no accepted target claim. That result exposed and corrected
abstention accounting, now separate from uncited answers. The live retirement drill's first pair passed
both arms on the local Qwen model, so the expected regression and retirement were
not demonstrated; the run was not retried to seek a passing result.

One copied nightly followup was replayed with the local Qwen model in an isolated
store. An extra tool call made its first arm inconclusive without tool execution
or a prose verdict. A second unchanged invocation deferred the same input and left
one ledger/attempt row. Build, formatting, Clippy and required-backend workspace
tests passed (2,625 passed, zero failures, three ignored). The missing-tail
regression failed under the old admission logic and passed with the fix.

The handoff source sweep also removed stale interactive-validation implementation
work: `surface_only_registry` and the recorded `SurfaceStore` fallback already
restore unavailable interactive tools in nonexecuting replay. It narrowed the old
phase-C item because `learning::extract_mismatches`, `commands::reflect::execute`
and `MISMATCH_REFLECTOR_SYSTEM` already produce bounded, goal-bound reflections.
General sampling and per-kind retrieval questions remain open.

**2026-09-09 — corrected attribution pilot complete, without rule exposure.**

**2026-09-09 — v0.1.19 patch release and local update.**
The owner requested the patch release after PR #222 merged. Release commit
`ef283174` updates the workspace and lockfile to 0.1.19 and moves the accumulated
Unreleased changelog into its dated release section, with both link definitions.
The tag workflow passed, all four crates were independently verified at 0.1.19
on crates.io, and the GitHub release was published. Main CI and documentation
deployment passed. Local all-target build, formatting, warning-free all-feature
Clippy and required-backend tests passed (2,638 passed, zero failed, three
ignored); web tests and production build passed. The static musl benchmark
build completed in 9m 52s and was installed at the shared checkout
`target-musl/release/mecha` path; its hash matches the release build and it
reports `mecha 0.1.19`.

The CLI and all four mail/document executables were installed from the clean
release checkout. The installed CLI exposed the new appraisal evidence and
outbox outcome commands. Both graph executables were reinstalled from graph
main `940c806` (0.1.5), with the nightly release artifact rebuilt and the installed
MCP's 13-tool surface checked. Slack, triggers, drain, serve and voice-worker
units restarted after installation. Startup logs and active mecha executable
hashes were verified; the web page matched the rebuilt assets and `/api/ping`
returned `ok`. All five units remained active with zero automatic restarts.
Factory client/server were already at 0.2.9 and the server was active; no remote
deployment was performed. Sandbox and host Cargo versions both read 1.97.1.

The shared checkout's existing uncommitted documentation was preserved. Its
worker, parakeet and model-start scripts were byte-identical to the release;
only the worker required a restart. The local model remained
`qwen3.6-35b-a3b`, four slots, 262,144 context tokens per slot. A separate Claude
session retained a deleted graph executable inode whose bytes matched the
installed server; it was left running. No operator configuration or learning
state changed. Earlier pilot reports retain their historical runtimes and do
not claim live-model validation of this release.

**2026-09-09 — appraisal implementation and review fixes merged.**
PR #220 merged at `08ebaaa5`, followed by anticipatory appraisal PR #221 at
`fa77852c`. The implementation and review entries below retain their historical
verification scope; merging did not install or restart any service.

**2026-09-09 — anticipatory appraisal review fixes, not installed.**
`harness_probe::prepare_episode` refuses evidence-bearing configurations across
an entire session until replay reconstructs them. `anticipation::History`
preserves unsupported prediction/outcome JSON while keeping draft review,
editing and rejection available; release and outcome grading fail closed.
`outbox show` now keeps the draft visible when the current assessment fails
(for example, a future timestamp), without weakening `begin_delivery`.
Regressions cover these paths through the store, CLI and harness probe.
A second review found the crossed case: a known harm outcome with an unknown
prediction fell back to positive drafting credit. Both unknown record kinds now
make appraisal partial and suppress that fallback; the regression failed before
the correction. Unsupported evidence marks appraisal partial without claiming
the outbox itself is unreadable, so request-to-draft joins retain their known
IDs. Owner evidence preserves pending plan verification and fills a missing
expectation from the plan; an external passed check cannot certify those steps.
The request-join and plan-merge regressions failed before these corrections.


**2026-09-09 — anticipatory appraisal and linked outcomes, implemented locally.**
On `feat/anticipatory-appraisal` (isolated `mecha-anticipation` worktree, based
on `f4c40cbeda1aad61f190b98a66645df2eb3f5c0d`), `anticipation::assess`
derives anticipated guilt, embarrassment, regret, disappointment, anxiety and
curiosity from typed evidence. Guilt requires an explicit recorded commitment;
check-first regret and curiosity require a feasible, affordable check. Delay
and budget shortfalls can require replanning, even after a check passed.
`run --goal ... --appraisal-evidence FILE` binds owner evidence to that invocation;
`Decision::with_owner_evidence` and the agent's outbox staging path consume it
without prompting private prose. Existing `goal_guidance` enables fixed advice.
New draft prose clears prior verification. `outbox anticipate` records immutable
argument snapshots, with a separate explicit `--guide` release gate;
`OutboxStore::begin_delivery` enforces it before dispatch and binds the attempt
to its prediction. Revision, reassessment, rejection, uncertain delivery and
silence have distinct unresolved statuses, never fabricated forecast grades.
`outbox outcome` admits post-delivery owner evidence with explicit attribution
and supersession. `appraisal::of_session` replaces one draft verdict with one
active outcome: embarrassment and guilt now have producers, so nine of eleven
retrospective labels are reachable. `tasks::worth_a_follow_up` excludes the new
outcome channel from autonomous follow-up creation. `RunConfig` records bound
evidence; replay and counterfactual grading explicitly refuse it until they can
reconstruct it. Tests cover the actual agent, Todo, outbox store, attribution,
supersession, expiration, legacy records and CLI; the Docusaurus appraisal guide
and command reference document the workflow. This is implementation evidence,
not a measured planning benefit. No installation or service restart was performed.

**2026-09-09 — appraisal PR review corrections, not installed.**
The frozen check executor now emits `Finding::CheckFailed` for an executed,
failed check; refused or staged checks stay unverified. Attribution remains
owned by the executor, so unrelated tool-span counters cannot certify a step.
`planning::examples` applies `runlog::Scan::admits` before its history limit:
33 newer development sessions previously displaced an older eligible run and
supplied synthetic successes. The regression failed against that behavior.
`learning::evidence_for` labels redacted mismatch evidence as `UserTurns`, and
`Reflector::reflect` skips malformed mismatch context without aborting the run.
Workspace tests with required sandbox backends and warning-free Clippy passed.
The next review caught a provenance omission in the restored advice: a frozen
`CHECK_FEEDBACK_STEM` now keeps that block out of owner-correction mining while
preserving real owner text sharing the same result message. Persisted synthetic
check calls use UUIDs rather than process-local run counters. Artifact grading
uses `mismatch::has_policy_refusal` in both the run and probe paths, excluding
harness check diagnostics while retaining refused model calls and blocked sends.
The mining and artifact-probe regressions both failed before these corrections.
A later review extended that boundary to correction scope and aftermath:
`extract_interventions` excludes harness tool uses and their denial results,
while preserving real steering beside them. Check-originated interlock blocks
stay in the check trace but do not increment the model's `blocked_sends` counter.
Both regressions failed before the fix; the armed-taint case verifies refusal
still occurs. Replay extraction now excludes harness calls from model choices
and filters machine advice from steering. Because replayed tools cannot recreate
those check observations, `Trajectory::ensure_replayable` refuses such recordings
in both drivers and all three trace entry points rather than grading changed
context or serving a check result to a model call. Artifact-task grading remains
independent of trace replay.


**2026-09-09 — corrected attribution pilot complete, without rule exposure.**
These measurements predate the review fixes merged in PRs #220 and #221;
they do not validate those fixes or anticipatory appraisal.
Runtime `c7071ad3` completed 72 Qwen 3.6 35B trials and 54 learning stages across
three seeds. Both arms passed 30/36 overall, 15/18 transfer and 31/36 artifact
checks; all 36 paired outcomes tied and the native gate rejected promotion.
Training produced 2, 1 and 0 clean criterion reflections per learning lifetime;
one later check-change reflection made the final counts 2, 1 and 1. Every
lifetime stayed below the unchanged evidence minimum: no rules, proposals,
paired rule validations, exposures or retirements. Transfer produced no
artifact-derived diagnostics. This demonstrates correctly sourced feedback,
not a learning benefit. The native extra failure on one otherwise-correct task
per arm came from a harness-generated attempt to execute a model-declared
`check: "file reads"` through unavailable `shell`; it was not a direct invented
tool call. The result archive preserves that distinction and the frozen grades.
Required-backend tests passed 2,616 with zero failures and three ignored tests.
Archived build/Clippy logs show completion but omit command flags; target and
feature coverage cannot be independently established from those logs. The final runtime's separate retirement
drill failed to elicit its expected regression and retired neither rule; the
earlier drill pass is retained separately. No score-seeking retry or deployment
was performed. Full evidence: `results/appraisal-attribution-v2-qwen36-35b-20260909/README.md`.


**2026-09-09 — invalid attribution pilot stopped; harness observations are not owner corrections.**
The first attribution pilot was stopped after 40 completed trials, before any
consolidation, because `extract_interventions` mined the generic criterion
observation as an owner follow-up even on passing tasks. The message was marked
`harness`, but only the assistant-role path honored that marker. The miner now
excludes marked user-role text and recognizes the frozen
`planning::CRITERION_OBSERVATION` voice for historical records. Genuine owner
follow-ups remain eligible. The regression failed before the fix. The mismatch
reflector now explicitly distinguishes a false context predicate (potentially the
correct decision) from a failed artifact verdict. The invalid run is preserved
under `results/appraisal-attribution-qwen36-35b-20260909/`; the replacement uses
fresh homes, the same tasks and seeds, and unchanged evidence gates.


**2026-09-09 — grounded criterion feedback and conservative overrun attribution.**
`StepFeedback::attribution` distinguishes failed task criteria, failed/changed
checks, and forecast overruns with unknown or batched boundaries. Cost-only
observations no longer author new behavioral lessons. Owner-bound
`ArtifactCase::criteria` supplies post-run field verdicts and optional count
constraints to the quarantined mismatch reflector, with the task and related
charter association preserved. Gold stays outside provider messages; unknown or
changed context stays ungraded, and recorded taint still gates learning.
The old queue failures reproduce as comparisons against the wrong count: the
recorded trace used twelve task records although `outbox_waiting` was zero.
A regression test failed under the former overrun-only mining and passes with
the new filter. The initial attribution measurement was stopped for the
harness-voice defect described above; the corrected three-seed pilot and its
limits are recorded in the later entry and result archive.


**2026-09-09 — independent artifact validation for planning mismatches.**
`mismatch::ArtifactCase` holds a strict, bounded owner fixture and checks real JSON
artifacts and preserved inputs in fresh workspaces. `run --mismatch-case` binds
it before execution; `RunConfig` keeps gold out of provider messages, with
malformed legacy metadata degrading to no fixture. `probe::prepare_mismatch`
requires unique matching harness feedback, clean recorded provenance, the same
confirmed goal and unchanged supported builtin specs. `drive_arm` repeats a
file-only task for validation, candidate gating and attribution, preserving
current approval/policy checks. Each executed arm records usage and fingerprints;
refused writes remain inconclusive and task-level verdicts do not claim regional
step coverage. Steer/denial branching and probation/retirement thresholds are
unchanged. `Tasks::mismatch_cases` freezes fixtures in the condition hash; the
new twelve-task lifetime design separates training from transfer. A live drill
caught `--no-goal-guidance` lacking Clap's global flag; its regression failed
before the fix and accepts either argument position after it. Live measurements
are recorded separately after completion, without changing earlier pilot grades.

**2026-09-09 — natural mismatch learning reached actual transfer exposure.**
On frozen runtime `edb554a7`, the twelve-task Qwen 3.6 35B sequence completed
24 trials and 18 learning stages: control passed 10/12 overall and 6/6 transfer;
learning passed 9/12 overall and 5/6 transfer. Seven clean mismatch reflections
produced one scoped rule. Its initial gate measured three unchanged both-pass
artifact pairs, and all six later task configurations recorded the rule's ID.
Fresh-evidence validation yielded two unchanged pairs and two inconclusive pairs,
with no attributed regression. Terminal consolidation selected three of four new
reflections, but its identical effective prompt caused all probes to skip.
Although the proposal disposition said `auto_applied_probation`, the final typed
roster retained the existing measured rule identity and non-probation status;
proposal labels are not substitutes for reading final state. The native gate
rejected promotion. The model, binary, fixtures and isolated operator configuration
matched at finish; nothing was installed or changed in the owner's learning home.
The separate seeded artifact drill detected an aggregate regression, but both
singleton subsets passed and attribution abstained: its strict retirement
assertion failed, while the existing trace retirement drill passed. Evidence and
limits are in `results/appraisal-mismatch-qwen36-35b-20260909/README.md`.

**2026-09-09 — harder anchored appraisal and sequential learning measurements.**
With runtime behavior frozen at `935d98f5`, Qwen 3.6 35B passed 20/24 control
and 18/24 guided trials on eight harder tasks × three seeds. The native gate
rejected promotion (two improved pairs, four regressions). Every run had its
confirmed anchor, and all 20 observed completion-time check omissions were
restored. Guidance was applied in all treatment runs. Model-written passing
checks did not guarantee artifact correctness. A filename ambiguity affected one
privacy failure; grades were preserved. A separately registered follow-up naming
`answer.json` in every prompt passed 6/6 control versus 5/6 guided, with an
incorrect exclusion count as its sole failure. These scores are not pooled.

The separate lifetime experiment kept guidance and rule loading on in both arms,
with learning stages only in the treatment and empty initial learning stores.
Both six-task sequences passed 6/6, including 3/3 in each predeclared transfer
slice. All twelve treatment stages completed. Two clean, full-evidence mismatch
reflections retained the correct privacy task goals, but stayed below the minimum
of three: no learned rules, validation records or later rule exposure. Thus this
measured reflection capture, not a learning benefit or rule-transfer effect.
The binary, fixtures, model alias and operator config matched at finish. Reports,
exports, traces and learning-stage evidence are in the three corresponding
`results/appraisal-{guidance-v2,privacy,learning-v2}-qwen36-35b-20260909/` directories.
Required-backend tests passed 2,597 with three intentional ignores; a known
trigger-lock test flaked once, then passed isolated and in the full rerun.
No defaults were enabled, deployment performed, or learning state copied back.

**2026-09-09 — completion checks preserved and explicit goal confirmation.**
`Tracked::advance` now restores and freezes the last open check when the first
completing write omits it. The executor sees the restored command and keeps the
ordinary approval, sandbox and denial behavior. An explicit withdrawal while a
step remains open clears the stale cached declaration. Both regressions failed
before the fix; the agent regression also failed because no check was dispatched.
`run::confirm_goal` records an owner-supplied `--goal` before execution, preserves
the saved anchor when omitted on resume, and overrides it when supplied.
`Tasks::confirmed_goals` registers strict references by selected case ID and
changes the condition hash without changing old manifests' hashes. The harder
pilot has real fixture board tasks and independent artifact controls; the separate
lifetime pilot starts from no learned rules and reports actual later rule exposure.
Source commit `935d98f5` contains the behavior changes. Required-backend workspace
checks passed 2,597 tests with three intentional ignores; Clippy was warning-free.

**2026-09-09 — Qwen 3.6 35B paired appraisal pilot completed.**
At `f3fe4df8`, the local `qwen3.6-35b-a3b` Q4_K_M model ran twelve synthetic
tasks × three seeds × guidance off/on, from 03:20 to 04:06 UTC. Both arms
passed 36/36 independently graded trials; the native gate rejected promotion
with 24 selection ties and 12 holdout ties. Mean turns were 8.83 versus 8.97,
model tool calls 12.86 versus 13.03, and output tokens 2,023.92 versus 1,929.39
(control versus guided). Wall-clock means were 34.90 versus 39.48 seconds, with
concurrent server activity and fixed arm order limiting causal interpretation.
Usage records were complete; dollar cost and owner actions were unmeasured.
Guidance was applied in all 36 treatment runs, with no other rendered config
change apart from trial paths/seeds. Pressure/quiet fixture choices and draft
preservation passed in both arms. The live traces exposed 71 checks omitted on
first completing updates across 51 runs, each recorded as `not_declared`:
`Tracked::advance`'s missing-field branch restores only already-frozen checks.
No run had a confirmed goal anchor, so zero drift counters did not measure
anchored-goal alignment. Code and fixtures stayed fixed during measurement;
model identity, binary, fixtures and operator config matched at the finish
checks. The full export, paired summary, conditions, trace diagnostics and
reproduction notes are under `results/appraisal-guidance-qwen36-35b-20260909/`.
The pilot establishes no task-success gain and leaves guidance opt-in; harder
anchored-goal tasks and learning across runs remain separate measurements.
No deployment, service restart or PR was performed.

**2026-09-09 — paired appraisal pilot and explicit planning levers.**
`experiment::child_invocation` now enables explicitly requested `step_checks`
and `goal_guidance` even when the operator's settings are false. The regression
compares materialized configurations, with every other field and flag equal.
`eval/appraisal-guidance.toml` defines twelve synthetic tasks, three seeds and
control/guided arms at equal budgets. `appraisal_source::setup` resets the task's
workspace inputs and owned draft seeds; `grade` reads independent artifact gold,
preserves input and draft evidence, and cannot be satisfied by success prose or
the misleading legacy checker. Positive controls and wrong-artifact, altered-input,
symlink, FIFO, malformed-JSON, type and queue-clearing controls cover the oracle.
Production `OutboxItem` deserialization accepts the synthetic pending drafts.
`appraisal-report.py::report` pairs task/seed/repetition, rejects duplicate or
identical conditions, separates incomplete/failed runs from graded failures, and
keeps missing metrics unknown with their own denominators. Model-generated
checks remain secondary evidence, and the existing holdout gate remains separate.
Build, formatting, warning-free clippy and the required-backend workspace suite
passed: 2,592 tests, zero failures, three intentionally ignored; eleven Python
controls run within the four appraisal integration tests. Registration, dry run,
export and reporting were exercised in a temporary home without model calls.
This pilot does not establish guidance efficacy, learning benefits, cross-turn
semantic corrections or owner-policy success. No deployment or PR was made.

**2026-09-09 — appraisal planning mechanisms implemented in the working tree.**
`Conversation::goal_anchor` and `Record::GoalAnchor` preserve confirmations;
`appraisal::attribute_events` binds observations to historical goals and retains
related charter/project pointers. Queue deltas no longer claim the run cleared
anything; `note_task_closure` keeps the owner's verdict separate from execution.
`Agent::run_loop` executes frozen plan checks through normal tool dispatch,
limited to sixteen per run, with separate model-work accounting. `StepFeedback`
records checks, prior work estimates, measured spans and check tampering;
`extract_mismatches` supplies bounded, provenance-gated learning evidence.
Reflections retain typed goal links, and `goal_context` retrieves applicable rules
and clean, scoped examples with passing checks. `Decision::assess` records task,
anchor and ordered charter discrepancies separately; `agent.goal_guidance` enables
fixed advice while sensor numbers remain outside provider payloads. No runtime
installation or service changes were made. Live efficacy, mismatch-specific
counterfactual grading, semantic owner-intent updates and work spans crossing runs
remain unmeasured or unbuilt; these mechanisms do not claim to solve them.

Validation: formatting and warning-free clippy over all targets/features;
2,588 workspace tests passed with backend skips disabled, three intentionally
ignored. The new regressions cover historical attribution, confirmation resume,
frozen-check execution and refusal, disabled and passing checks, cross-step
attribution, mismatch provenance, context scope, sensor isolation at both provider
encoders, and the real configuration layer. No live-model efficacy result is claimed.

**2026-09-08 — merged assistant features installed.** After the owner merged
PR #216 as `c3f33f4c` and requested an update, the shared main checkout advanced
from `f22992c5` without changing the model or Parakeet launch files. The installed
CLI, four mail/doc executables, graph CLI/MCP and web assets were refreshed;
the graph release build verified the nightly artifact against current source. Serve, Slack, triggers, drain and the voice worker
restarted at 19:58:54 UTC. Copying the two serve units and reloading systemd made
PR #217's `KillMode=mixed` and 180-second drain window effective. Live HTTPS
checks proved the new bundle, Today, graph access and write guards; an isolated
installed-workflow smoke proved fresh artifact checks still gate closure.
The static benchmark binary was rebuilt from the same source and its static
ARM64 linkage and new recovery command were verified. The full required-backend
suite again passed 2,575 tests, with three ignored.
Factory client/server were already current at 0.2.9 and were verified read-only;
the obsolete open task to deploy the droplet from 0.2.8 was removed on that
evidence, without attributing its deployment to this session. The matching
sandbox toolchain needed no rebuild. One old graph child remains
owned by an existing Claude session, recorded in HANDOFF rather than terminated.
This update supersedes the earlier deployment hold and the branch-only deployment
statements below. It publishes no release and does not relabel the older 15/15
live evaluations as measurements of the deployed commit.

The integrated shutdown and synchronization paths are implemented by
`ChatState::{stop, drain, close_mcp}`, `McpClient::close`,
`chat::{send, begin_turn}` and `Chat.svelte::{receiveInput, markDelivery}`.
The historical validation of PR #217's `fix/serve-shutdown-and-chat-sync`
branch over PR #215 remains 2,538 passed, two ignored and 16 browser checks;
#216's integrated tree passed 2,575 and 17 respectively.

**2026-08-19–2026-09-03 — release observations retained from the old handoff.**
The following was recorded before v0.1.18 shipped on 2026-09-06. Its present-tense
release and installation statements describe those earlier sessions only; the
2026-09-08 update supersedes them. The original detail is retained here rather
than presented as current state to a reader deciding what to build next.

Public at **github.com/ljchang/mecha**, MIT licensed, released as **v0.1.16**
(2026-08-29 — the appraisal system survives its own review: PRs #111/#112,
failed-turn transcript integrity, positional configs for the probe,
unreadable-store accounting, and the owner-closure guard; v0.1.15 shipped
2026-08-26 — five surfaces that described themselves wrongly, found by
using them; 0.1.14 shipped 2026-08-25 — voice calls and chat became one
conversation, three review surfaces stopped hiding what they were asking
people to approve, and the nightly mail classifier took both mailboxes;
0.1.13 shipped 2026-08-24 night with the web surface, voice, and the graph
queue's similarity groups; 0.1.12 on 2026-08-22, 0.1.11 and 0.1.10 both on
2026-08-21, 0.1.9 on 2026-08-20, and 0.1.7/0.1.8 on 2026-08-19/20 after the
mail hold lifted).
**Released as v0.1.17 on 2026-08-31**, carrying the ten merges beyond
v0.1.16 that this paragraph used to list as untagged:
the 2026-08-30 five — #125 (**four home-page queue cards did nothing**,
plus the ExecStart-check rewrite and the `js_string_array` guard
loosening); #126 (**the graph tab grows its notebook, composer, and the
whole entity-curation surface** — alias add/remove, merge-with-audit-trail,
create-on-miss, identifiers, plus the three-store `/api/proposals` pane;
deployed the same night, so the installed binary is `main` at `ab0097b`);
#124 (**the retirement drill ran the NoGo path
whole, and fixed the probation leash it proved unreachable**); #120, the
notes and graph tabs become one graph tab; #123, the docs deploy stops
being evictable by a PR build — then #122 (2026-08-30, fifteen commits —
**the learning loop runs itself and the instruments that grade it were
fixed**: `learn --auto` with probation, nightly direct retirement,
branched counterfactual probes, the surface rebuild from recorded specs);
and the 2026-08-29 four — #114, the shadow queue on every owner surface
(plus a web entity page and chat tool-result previews); #116, the
`/tasks` page repair; #115, the appraisal docs-page rework; #117, the
docs site's fixture-backed web demo and its two CI gates. All in the
`CHANGELOG.md`. The tag was cut with the
release workflow green, and all four crates confirmed live on crates.io
at 0.1.17 (`mecha-core`, `mecha-cli`, `mecha-mail`, `mecha-slack`) by
querying the registry rather than by watching the job go green. Seven of
the merges in it had landed with no changelog entry (#119, #121, #123,
#125, #127, #128, #130) and were written up at release time.
**`main` now carries nine merges beyond v0.1.17 that are not yet tagged**,
all 2026-09-02/03: the audit lane's #139 and #142 and its approval-rules
pair #143 and #148 (each has a `CHANGELOG.md` entry under Unreleased as of
this pass); the appraisal lane's #140, #141 and #147 (**no changelog
entry yet** — the same trap as last time, to be written at release or by
that lane); and the two docs merges #146 and #149. The next tag is
v0.1.18, and the installed binary is older than all nine.

**2026-09-08 — assistant changes integrated with serve shutdown and chat sync.**
Rebasing PR #216 over merged PR #217 preserved both lifecycle paths: `begin_turn`
checks shutdown admission, records input, claims the workflow, then broadcasts
acceptance with the request id. The recording-failure regression now also checks
that a refused launch emits no acceptance. Task tracking and steering receipts
remain intact, and both independent browser regression blocks are retained.
The combined workspace, required sandbox backends and 17 browser checks passed;
no service was installed or restarted.

**2026-09-08 — gates precede question consumption; batch refusals are counted.**
PR #216's eighth review found the question-resume workflow gate after question
closure, board movement and run-marker creation. It now precedes those effects;
a real CLI regression failed before the fix and confirms cancellation and unmet
dependencies preserve the question bytes and leave no running marker. Batch
outbox rejection now continues past a refused uncertain delivery, reports each
failure and totals, and exits nonzero for partial failure. Its CLI regression
also failed before the fix and confirms both eligible drafts are rejected while
the uncertain record stays byte-for-byte unchanged.

**2026-09-08 — failed recording is not an interrupted run.** PR #216's seventh
review found that web/voice task turns claimed a workflow before appending the
user message. Either append failure dropped the run guard, erased verification
and raised a false partial-work alarm. `begin_turn` now accepts the transcript
write first, retaining the conversation on a later launch refusal and updating
voice continuity only on success. The actual entry-point regression failed
before the fix and covers both append/fold failures, untouched workflow bytes,
preserved taint, and memory/transcript agreement after a closed-workflow refusal.

**2026-09-08 — refresh reads scale by source store.** PR #216's sixth review
found repeated source-directory parsing and global config loading on every
workflow in Today and scheduled ticks. Request-scoped caches now share scans,
linked-record reads and handles, including failures; subsequent requests and
completion checks reread the owning stores. A regression failed before caching
and covers shared reads, fresh corruption detection and retained scan errors.
Cancellation help now states that it also blocks future task/chat/trigger runs,
and resume errors give the explicit reopening command. The intended owner gate
remains in place; cancellation never claims completion.

**2026-09-08 — web completion has a web recovery path.** PR #216's fifth review
found that finishing a workflow hid it from Today and blocked further task-chat
turns, with reopening available only in the CLI. Today now retains finished and
cancelled workflows in a collapsed section with a protected Reopen action. Closed
records are summarized before source scans or artifact verification. A real CLI
close/Today/reopen smoke and a mobile browser close/reopen regression failed on
the old behavior. The launch gates remain in force. The misplaced trial-method
doc comment and the convenience profile's bare-name matching caveat were also
corrected.

**2026-09-08 — owner recovery survives PID reuse.** PR #216 review found that a
stale PID could look alive after a reboot and block every new run without a CLI
repair path. `workflow recover --reason` now records the owner's assertion that
the old runner stopped and clears only its ownership. Completion writes carry the
originating run ID, preventing a superseded run from overwriting its replacement;
a regression reproduced that overwrite before the guard. Partial drafts and
unknown deliveries survive recovery. Resume and verification share terminal-action
classification, so rejected drafts do not reintroduce the closure bug on resume.

**2026-09-08 — corrected drafts can complete a workflow.** PR #216's third review
found that a rejected draft stayed linked and blocked completion forever, even
after its replacement was sent. Rejected drafts and owner-abandoned questions now
resolve their decisions; explicit delivery checks still fail on rejected drafts,
and pending or uncertain replacements still block. Known outbox IDs use direct
reads, with prefix fallback only when the exact record is absent. Owner closure
uses the same bounded event bookkeeping while preserving verified evidence. A
reported image-taint gap was checked through a live-divergence replay branch:
`Agent::run_in` already calls `arm_for_content` before execution, and the new
branch regression confirms an image plus external content blocks the live send.

**2026-09-08 — workflow history and legacy replay disclosure.** PR #216's second
review identified unbounded per-turn workflow history and overstated replay
fidelity for old transcripts. Workflows now retain 128 recent events with a
persisted sequence for reminder deduplication; a regression reproduced the old
growth and verifies new notices after pruning and restart. Replay keeps unknown
provenance external, while its CLI note, JSON report and module documentation name
the possible extra warning envelopes and stronger live-send restrictions. Those
are harness differences, not proof of a model regression.

**2026-09-08 — final assistant measurement.** On `b9d4be61` (base
`a75b9467`), the five sequential synthetic assistant tasks across three seeds
passed 15/15, including all 15 artifact postcondition sets and six grounding
rubrics. All ten positive/negative live judge controls passed. Manual review
of the final answers and supporting calendar reads found no recurrence of the
targeted date, read-receipt, calendar-absence or delivery-state errors. The
original and intermediate failures remain in the two assistant result reports;
this small same-model-judge measurement is not independent factuality
certification. Demo route coverage also caught an omitted workflow-mutation
route; adding its explicit demo refusal restored coverage and all 15 browser
render checks passed.

**2026-09-08 — source weekdays and a judge false negative.** The calendar-reference
run passed all 15 automated trials, but manual review found one background sentence
calling the incoming Sunday email Monday. The automated score is retained with
that manual finding. `time::calendar_date` now adds computed weekday/date/offset
metadata to mail rows and thread headers, preserving the raw timestamp and leaving
unparseable stamps unknown. The fixture mirrors it, and the rubric/control set
checks incoming-message weekdays as well as outgoing commitments. Model-judge
passes still require human spot checks; adding a judge does not eliminate its
blind spots.


**2026-09-08 — computed calendar facts.** Once the rubric received the correct
run context, it caught a real assistant error: “tomorrow, Thursday October 14”
on Tuesday October 13. That run scored 14/15 overall, all artifact checks passed,
and the original failure is retained. `date_context::render` now supplies local
weekday/date pairs from yesterday through the coming week, computed with calendar
days after timezone conversion. Boundary tests cover DST, year and leap day.

**2026-09-08 — follow-up grounding and evaluation.** The assistant branch now
includes `main`'s browser/MCP fixes at `a3f1682d`; `Today` uses `apiFetch` so its
mutations carry the required request header. `unified::render_rows` explicitly
names owner-mailbox read state and unknown recipient read status; fixture rows
match. `capability_prompt` and the default prompt require evidence for calendar
and relative-date claims, including in minimal configurations.

`Fixtures::clock` pins each task's simulated instant and hashes the schedule into
the experiment condition. `apply_clock`, `fixture_now` and `fixture_clock.py`
share it across the model prompt, mail and board; real audit time is untouched.
A real-MCP regression advances a day across restart and verifies the old sent
timestamp is retained. `Manifest::judge` makes the rubric provider/model explicit;
`grounding_evidence` supplies recorded system context and tool calls/results to
the quarantined judge.
A missing/corrupt transcript, oversized evidence or unavailable judge fails its
check. The live known-answer calibration accepted three grounded answers and
rejected four unsupported read-receipt, relative-date and calendar claims. This
is calibrated evaluation, not a claim that a model judge guarantees truth.


**2026-09-08 — serve shutdown and shared chat input (branch, not deployed).**
`serve::execute` owns SIGTERM and Ctrl-C, stops HTTP admission, and drains
chat and the mounted facade together. `ChatState::stop` serializes shutdown
with turn admission and permanently closes `Questions` before cancelling
active runs. Its task tracker waits through transcript/taint/outcome recording;
background title generation yields to shutdown. `Facade::shutdown` likewise
closes admission and waits for handlers, including owned voice slots; request
reads yield to shutdown and socket writes have deadlines during shutdown. An
actual daemon regression first exited with SIGTERM instead of success, then passed with a
partial answer recorded. The expanded tests cover simultaneous typed/voice
runs, waiting questions, two SSE subscribers, idle voice sockets and stdio MCP
child cleanup.

`chat::begin_turn` broadcasts typed and spoken input; `send` broadcasts
steering when accepted. `Chat.svelte::receiveInput` correlates a browser's
request id across SSE and its POST response, so identical words from different
requests remain distinct and a late acknowledgement cannot restart a completed
run. The model's steering-drain event updates the accepted message instead of
repeating it.
Sixteen compiled-browser checks include two independent pages, both arrival
orders, steering and rejected sends. These changes close the three duplicate
shutdown/broadcast open items; deployment remains explicitly deferred.

PR #217's first review found that no handler observed a second signal during
the drain. `ShutdownSignals` now keeps both receivers alive and lets a second
SIGINT or SIGTERM force termination through normal runtime teardown, retaining
MCP destructor cleanup. A real blocked-MCP regression covers the two-signal
path and child cleanup. The other finding was `crypto.randomUUID` being absent
on plain HTTP origins; request-id generation now has a fallback inside the
send error handler. The two-browser regression disables that method, reproduced
the lost message, and passes with the fallback.

The service-unit follow-through found `KillMode=control-group`, which sends
SIGTERM to MCP children before their host can finish a tool call. The checked-in
serve/voice units now use `mixed` and a 180-second stop window. Both daemons
await `McpClient::close` after their runs and voice handlers finish; Docker
removal completes before main exits, since systemd can otherwise kill that
cleanup subprocess too. A real-container test holds the client Arc across
explicit close, so Drop cannot make missing awaited removal look successful.
The live unit files were only read; installation/reload remains deferred.

The second PR review caught acceptance being mistaken for delivery: input
queued during a final answer can miss every drain point. `QueuedDelivered`
now updates its existing bubble when core actually folds it in; a receipt
left at hand-back becomes `QueuedDiscarded`, checked under the admission lock
so a final send cannot slip past. The browser says queued, steered, or not
delivered and retains an early receipt through a late POST acknowledgement.
Two actual-daemon tests distinguish folded input (present in the transcript)
from late input (absent and retracted on both subscribers); the browser test
first failed on the old premature delivery label.

The third review caught a shutdown deadline applying during normal voice
streaming and MCP close killing a server before its EOF flush. `VoiceStream`
now preserves normal backpressure, starts the write deadline only on shutdown,
and refuses further writes after a partial-write failure. The real-socket
regression fills the send buffer, waits past the old deadline, then checks
bounded shutdown and that no trailing frame reaches the wire. MCP close drops
stdin, allows two seconds for EOF handling, then escalates through TERM/KILL
within its outer bound. Readers stay alive throughout; a server that flushes
more than a pipe buffer before saving its exit marker first failed under the
immediate kill and now passes. Tokio child-stdio `shutdown()` is a no-op on
Unix, so delivering EOF requires dropping the pipe itself.

**2026-09-08 — assistant follow-through, implemented on
`feat/personal-assistant-follow-through` (not deployed).** `Config::merge_file`
preserves global outbox routes, publish classification and review-store identity.
`Message::tool_provenance`, `run_tools`, the session reader and replay retain
per-result external provenance, with legacy unknown results conservatively
untrusted. `OutboxStore::begin_delivery` persists a delivery attempt before
`Surface::release` dispatches; unknown outcomes block retry/edit/reject until the
owner records evidence with `reconcile_delivery`, available in CLI and web.
This provides reconciliation, not a claim of generic exactly-once remote effects.

`workflow::WorkflowStore` joins graph-task, session, question and outbox references
under a private atomic store. Task work, web task conversations and question
continuations record their lifecycle; `WorkflowRun` cleans up early exits, and
`Workflow::observe` recovers partial drafts by session after a crash. Explicit
checks read artifact bytes inside the workspace jail and confirmed deliveries;
Today rereads them before presenting ready work, and owner closure rechecks.
Dependency links reject cycles and gate continuation on completed predecessor
workflows. Owner-entered commitments carry source, counterparty and timestamps;
`Workflow::tick` persists coalesced in-app reminders under owner-only timezone,
quiet-hour and digest policy. Snoozing suppresses reminders without hiding overdue
work. The daily screen groups urgent work, decisions, verified work and waiting.
Graph task closure remains the owner's existing `tasks set` action.

`CompletionRequest::response_schema` and `Provider::structured_output` add an
explicit endpoint contract for JSON schemas, used by the quarantined frontdoor and
mail passes; unsupported fallbacks cannot silently drop it. `ToolProfile::narrow`
selects stable research/assistant/coding tool subsets, inherited by children and
stored on triggers, with registry-derived capability guidance. Prompt text no
longer claims delegation can clear taint or hardcodes obsolete mail tool names.
`FixtureCheck::grade` checks actual fixture rows after principal actions and affects
trial success; the five-case `assistant-lifetime.toml` design repeats across three
seeds and records requested owner-action counts. Multi-day restart, unknown-send,
artifact, dependency and reminder tests supplement the model cases.

Final verification passed the workspace suite (2,508 passed, two ignored),
Clippy with warnings denied, formatting, all-targets build, frontend tests/build,
and documentation build. All nine sandbox tests also passed with missing-backend
skips disabled. A real-binary workflow smoke checked artifact changes, nonregular
file refusal, reminder deduplication, closure, reopening and cancellation.
The local Qwen three-seed fixture run scored 12/15 overall and 15/15 artifact
postcondition sets with six requested principal actions. The checked-in
`results/assistant-follow-through-2026-09-08.json` retains measured checks,
post-run wording/tool-path calibrations and unsupported narrative claims;
there is no control-arm improvement claim or production delivery measurement.

The handoff audit also removed stale requests for features already present before
this branch: per-command approval (`ExecPolicy`, `live_rules`), the `/learning`
modal, mail task `captured_from` pointers, consolidation widening and regional
validation, and registration-time `ShowFileTool` configuration. These are
corrections to the handoff, not implementations attributed to this session.
**2026-09-08 — explicit chat opening and Docker container ownership.**
`serve::chat::open` creates a session through a guarded, idempotent POST;
`transcript` and `events` only look up existing sessions. The browser awaits
opening before GET/SSE, abandons late opens after navigation, and reopens on
reconnection. A route regression first reproduced GET creating a session;
browser checks cover opening, navigation during opening and lost server state.
`sandbox::DockerContainer` owns creation through a worker that survives caller
cancellation, then attaches only after creation succeeds. Drop launches
container removal directly and monitors/retries outside Tokio, retaining the
same Docker context. Three real Docker regressions first reproduced containers
surviving last-client drop, failed initialization and cancelled initialization;
all pass, including a server that ignores EOF. Two control-worker tests cover
late creation and runtime teardown. These source changes close the container
gap recorded below; they are not yet installed.

PR review found that `run::execute` used `process::exit` while `prepared`
still owned the clients. It now releases them after recording and session
hooks, before either special exit. Two real-binary regressions reproduce the
refusal and no-output container leaks and pass with cleanup enabled. Control
errors also retain a bounded stderr tail while continuously draining Docker's
progress output. The review's adjacent mode-route finding was reproduced with
a percent-encoded traversal key; `set_mode` and `answer` now validate keys like
the other handlers. The CI sandbox job requires the process-exit tests too.

A second pass found the sibling `batch::execute` exit; it now releases its
prepared agent after flushing results, measured by a third real-binary Docker
regression that first failed on exit code 1. The remaining explicit CLI exits
were checked: diagnostics/configuration commands hold no MCP clients, `main`
exits after dispatch has returned, and eval's agent and optional fixture clients
are scoped to `run_arm`, which returns before its caller's failure exit.
Browser retries now stop on permanent 4xx responses and back off on transient
failures, resetting after the event stream connects. A compiled-browser test
first reproduced repeated 403s and missing backoff before verifying the fix.

**2026-09-08 — PR #213 deployed after merge.** After CI and automated review,
`a3f1682d` was installed with `cargo install --path mecha-cli --locked --force`
and a fresh web build. Serve, Slack, triggers and drain restarted at 14:15 UTC.
The installed binary contains the new guard literal; a harmless invalid upload
returns 403 without the request header and reaches validation (400) with it.
The HTTPS door serves the installed `index-DwEoLNed.js` bundle. The prior binary
and web assets are backed up under `~/.mecha/deploy-backups/pr-213/`. This
deployment includes the earlier browser/attachment/process fixes below, not
the follow-up chat/container changes above. Python workers and the model server
were not restarted.

**2026-09-08 — browser boundaries, attachments, and process lifetime.**
`serve::owner_guard` now requires a non-simple header for browser writes;
`apiFetch` and hosted voice offers carry it. `WorkspaceFiles` reserves uploads
without following inbox or filename symlinks and opens downloads through held
directory descriptors. Both handlers use `attachment_workspace`, so resuming
a task conversation preserves its attachment jail. Downloads stream with
backpressure; `FsRead` limits allocation and decoding while selecting lines.
`McpClient` owns its reader tasks and enables child termination on drop before
initialization, including the failed-handshake path. For Docker confinement,
the killed child is the Docker CLI; containers whose servers ignore EOF can
still outlive it. That gap is closed by the follow-up recorded above. Regression tests first
reproduced cross-site writes, upload escapes, resumed-workspace mismatches,
and a child surviving its client; the fixes pass those tests. Graph route
state and charter drag state also stopped producing Svelte build warnings.
This records source changes, not an installation or deployment.

**2026-09-08 — handoff reconciliation against source.** Older open items
already implemented were removed: pressure formatting (`Pressure::fmt`),
compaction enabled in `replay::run` and `harness_probe::run_episode`, admission
control (`Permits::take`), command approval (`ExecPolicy::decide`, with both
`Config::rules` and `ConfigLayer::rules`), and the TUI learning view
(`LearningModal`). `work_prompt` already names a single `ask_user` call;
whether it fixes the recorded adherence failure remains unmeasured.
`CancelReason` and `appraisal::stops_of` implement the cancellation split and
cancel-and-reprompt attribution. Charter sensor infrastructure is implemented;
the remaining sensor kinds stay in HANDOFF. Scope widening and regional
validation are in `finalize_region_rules` and `cover_selection`; mid-run
delivery remains open. Mail task capture already passes `captured_from`, and
`ShowFile::new` captures the prepared upload limit. In the graph sibling,
Reflect ZIP ingestion, README onboarding, gossip quota fallback and
retrieval-utility demotion exist; automated notes sync, release automation and
acceptance-statistics demotion remain separate gaps. Removed historical
observations are retained below with their original dates and measurements;
their old open/unmerged claims are superseded by this source verification.

**Earlier handoff observations, reconciled 2026-09-08.** The 2026-08-25
sweep found six mail phases that had been listed as unbuilt since August 19.
The preceding August 24 sweep covered 72 open items after 45 commits; that
coverage concerned the handoff's open-work list, not this chronological ledger.
The August 20 pass had already closed skills and Google document write access,
and narrowed the task-store gap to escalation. Later command-policy and scoped
learning changes superseded that sweep's remaining-gap inventory.

The old PR #84 note recorded a 400-token pace displayed as “~1k each, so about
224 more” against a 100k limit, plus an unreachable arm and replay/probe contexts
that never enabled compaction. It reported 1,566 local tests with CI unverified
at that time. Those defects are now fixed in `Pressure::fmt`, `replay::run`, and
`harness_probe::run_episode`; the old instruction to check the unmerged PR is
obsolete. Admission control, once deferred as phase 6, is implemented by
`Permits::take`. The old claim that shell commands had no configurable approval
policy is superseded by `ExecPolicy::decide` and the config rule layers. The
proposed `/learning` view exists as `LearningModal`, mail tasks carry their
thread in `captured_from`, and `ShowFile::new` captures the prepared upload limit
instead of rereading the entire config at call time.

**2026-08-26 — graph copy repair (historical measurement).** Graph `237b686`
fixed the shared copy path: `fork --out …` completed on the then-live 202 MB
store in 1m38s with matching counts. The August 20 harrier switch had left
source `vec0` tables wider than the migration-created destination. Only one
of three copy paths had reconciled the width, so both fork and encryption
failed; `copy_all_tables` now owns the reconciliation every caller needs.
This was the second shared-copy failure after a destination schema change
(the earlier migration seeded a node): a step every copy needs belongs in
the function every copy calls. The source repair was reverified September 8;
the timing was not rerun.

The old gossip quota observation used 10 candidates, three targets per night
and a seven-day cooldown, requiring 21 distinct targets: nights under-filled
with two on August 22, one on August 17 and two on August 16. It also recorded
one contact at 26 retrieval touches and suspected the probe's own reads.
The quota defect is superseded by the graph nightly's 25 candidates and
least-recently-probed fallback when fresh candidates run out; those original
figures remain historical observations. The proposed stranger-facing graph
README pass is also complete: installation, synthetic-data onboarding and
MCP wiring are in the sibling's README.

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

**Batch review landed in the outbox**, which had taken exactly one id per verb
— so an overnight triage staging nine replies was nine invocations and nine
startups of every configured MCP server. The tension worth naming is that bulk
approval is how a review queue becomes a rubber stamp, so the batching saves
invocations and never saves reading: `outbox review` walks the pending items
one at a time, with the draft and its taint warning in front of you at the
moment you decide. Ids may be given several at a time, `--all` is narrowed by
`--kind` and `--via`, and the selection rules are a pure function with a test
each — a selection naming nothing is an error rather than "everything", and a
filter matching nothing is an error too, because a typo'd filter acting on zero
items reads exactly like an empty queue.

**And the factory went from built to deployed.** Step 6.5 made it
multi-tenant — a user owns every row, and their artifacts are served from
`<handle>.art.…` rather than a path prefix, because origin is the only
isolation a browser enforces and a published URL has to stay resolvable
forever. Step 7's server half followed: a form rendered from the manifest, a
single-use verification link, and a state machine where nothing reaches the
queue until somebody clicks. Then it was actually stood up — a small VPS
serving the gate and the first tenant's artifact origin
under a Let's Encrypt certificate the binary obtained for itself over
TLS-ALPN-01, with the first bundle published from this machine to that one.
The design doc gained §14 (multi-tenancy, the two axes, the request-path
invariant, the attachment lease) and §15 (users, handles never reused,
withhold versus purge, the outbound-mail abuse surface); the deployment
procedure and its two traps live in that repository's `docs/DEPLOY.md`.

**2026-08-06 (last) — the inbound half, and the quarantine at the centre of
it.** The drain client landed on the home side: `mecha-factory-publish drain`
speaks `GET /v1/queue` with the scoped key and writes each row into
`~/.mecha/requests/`. A CLI rather than a tool on purpose — the common case is
"nothing new", and it has to cost zero tokens and no model.

Then `mecha frontdoor` (`frontdoor.rs`, `commands/frontdoor.rs`), which is
everything that happens to a stranger's request afterwards and exists to serve
one sentence: **the privileged run sees the extraction, never the prose.** A
run holding the calendar and the mailbox is the most dangerous context in this
system, and a free-text field is the one place a stranger controls the bytes.
The typed form already does most of the work — nothing anyone types can change
what *kind* of request theirs is, because those are enums the origin
validated — so what is left is prose, and prose is where an instruction hides.
CaMeL's dual-LLM split, at a size where it is cheap.

Four things carry it. `Record::for_privileged_run` is a **function with no
argument that returns the prose**, so the boundary is unreachable rather than
remembered; the extractor's own `reading` stays behind too, because a
paraphrase of an injection is the injection rearranged. `extract` is issued a
request with an **empty tool list and one user message** — not a model told not
to use tools. An extraction failure routes to a human instead of passing the
prose on, which is the one behaviour that would make the layer decorative. And
which fields are prose is decided by the drain from the manifest, never guessed
at on this side by looking for long strings.

**2026-08-07 — a request can reach an answer.** The state machine stopped at
`extracted`. `Record` named the chain `drained → extracted → triaged →
awaiting_me → answered`, but the only states anything ever wrote were the first
two and `extraction_failed` — so a stranger's request could arrive, verify,
queue, drain and pass safely through the quarantine, and then there was nowhere
for it to go. The queue only grew.

`triage`, `needs-info` and `close` finish it, and the join needed no building: a
staged outbox item already records the session that drafted it, so a triage run
with its own session says which drafts belong to which request. `reconcile`
reads the outbox and updates the request store, and runs on its own rather than
on a verb someone has to remember — a state that is only correct after you run a
command is a state nobody can trust. A rejected draft returns the request to
`extracted` rather than closing it, because "not this reply" is not "not this
request"; a partly-resolved set is left alone, because some sent and some
pending is a person mid-review; and `triage` refuses to run without the outbox
route rather than running unrouted, since a stranger's inbox is not where you
want to find out `[outbox] tools` was unset.

**2026-08-07 — a brand, and two documents catching up with the code.** The 9A
mark landed in `brand/` with `scripts/build-brand-assets.py` generating the
three rasters SVG cannot serve, and the Docusaurus scaffold's own artwork was
deleted — shipping another project's mark as this one's is worse than having
none. The site had had no logo at all until then, for exactly that reason.

The public documentation caught up with everything above: three pages that did
not exist (the work directory, publishing, the front door), and three gaps in
pages that did — `web_search` was missing from the built-in tool table,
`Tool::carried_state` was undocumented on both sides of itself, and the MCP
working-directory fix was unrecorded. The CLI and config references were
checked against the binary rather than by reading, which is the only way that
check means anything.

Then the same pass on `CLAUDE.md`, which had drifted further and matters more,
being loaded into every session as project instructions: its architecture map
was missing nine modules, and it had no account of the front door, the search
backends, subagents, or replay.

**2026-08-07 — the box sends the one message it owes a stranger, and stops
trusting the client.** A form rendered, validated, stored a row and told the
visitor to check their email, and then nothing arrived: `Mailer` had one
implementation and it wrote the link to the journal. Amazon SES over HTTPS
with SigV4 signed by hand closed it — not the SDK, because the box picks
`ring` over `aws-lc-rs` for ACME so building needs no cmake, and
`aws-sdk-sesv2` would bring a second HTTP stack. The dependency list's claim
that the box "never initiates" had to be spent to do it, and was rewritten
rather than quietly dropped; the claim that survives is stronger and is the
one that mattered — *the box holds no credential that reaches home*.

Setting the account up produced the more transferable finding. The zone
already published `v=spf1 -all` and `p=reject; adkim=s; aspf=s`, so a naive
SES setup would not have landed in spam — receivers would have **refused the
message outright** while SES reported success. Easy DKIM signing as the domain
itself is what satisfies strict alignment. Delivery to a mailbox at a
strict-DMARC host is therefore the proof it passed, since that host would have
rejected it otherwise. Production access came free, inherited from another
project on the same account.

Then the review gate moved off the client. `Scope::Publish` had authorised all
five write endpoints, so one key could push a version *and* move the alias
that publishes it — and the only thing between an agent and a publication was
mecha's `[outbox] tools`, a list in another repository. Point a different MCP
client at the same server, or typo one entry, and there was no review and
nothing said so. The split the data model already implied: publishing writes
an immutable version nobody can read, releasing is what a reader sees. Now an
agent holds publish-only, and the worst a stolen agent key does is write
versions nobody can see. Deployed and verified against the live box.

**2026-08-07 — a new handle gets a certificate, and nothing restarts.** The
factory's certificate was ordered once at startup for a fixed list, so a user
created while the server ran had no hostname until a restart — the assumption
`SELF-SERVE.md` exists to remove, and the one part of self-serve that was not
ordinary web work. Shipped in the factory repository:
`mecha-factory/src/certificates.rs` plus a rewritten `tls.rs`. Issuance moved
from TLS-ALPN-01 to HTTP-01 (the `pub(crate)` acceptor was only ever needed by
the challenge, not the goal), one `AcmeState` per certificate group sits
behind an SNI-dispatching resolver, and the certificate set is *reconciled
from the ledger* every thirty seconds rather than announced — `factory user
create` runs in another process, so a notification channel would only have
served the signup endpoint that does not exist yet. Port 80 became
load-bearing for issuance, so `[listen] http` is refused-if-absent beside
`[tls]`, and the DEPLOY.md sentences that said otherwise changed with the
code. A high-effort review before deploy confirmed eight defects — nearly all
one shape, the machinery degrading with no log line on the port that had just
become load-bearing. The fixes: seed one ACME account before any order (a
cold cache raced N groups into N registrations, capped at 10 per IP per 3
hours), migrate the old combined certificate into the per-group cache keys
(the upgrade would have re-ordered everything against the 50/week budget with
every name failing handshakes meanwhile), answer no-SNI handshakes with the
base certificate (monitors and curl-by-IP send none), release a dead ACME
task's claim so the reconcile loop can actually repair it, and survive a
transiently locked ledger at startup instead of exiting before the listener
was up. Deployed the same day: the migration made the restart zero-downtime,
and a throwaway `smoketest` user went from `user create` to a served Let's
Encrypt production certificate in about thirty seconds with nothing
restarting. The unclaimed-handle property survived — no resolver, dead
handshake, the 404 still the second line of defence. The plan's own surprise:
the wildcard `A` records the deployment already had meant the Cloudflare zone
move gated nothing after all.

**2026-08-07 — an invited stranger becomes a tenant, no operator anywhere in
the path.** Steps 3 and 4 of `SELF-SERVE.md`, built and deployed the same
afternoon. `factory invite create` mints the right to claim one handle (token
hashed at rest, seven days); `/signup/<token>` on the gate claims it through
the same `create_user_in` the CLI calls, spending the invite in the same
transaction, and the welcome page ends with the exact `factory-publish
connect` command — code included — because the moment a person has just
proved themselves at a browser is the moment to hand them the next step. The
pairing confirmation became the protocol rather than a prompt: `POST
/v1/pair` takes the code *and the asserted handle*, the server redeems only
on a match, and a mismatch spends nothing and answers byte-for-byte what a
nonexistent code answers — no client, human or agent, can wave it through,
which is what the Claude-Code review demanded. Redemption mints the machine's
own publish and drain keys (never release) in the transaction that spends the
code. Proven live within minutes of deploying: a real invite, a claimed
handle, a wrong assertion refused with the code surviving, keys installed at
0600, a push landing on the box, and the unreleased bundle serving to nobody
— which is the scope split working for a stranger's account end to end. The
live proof also caught a real bug: `mirror()`'s "bytes up, not published,
release elsewhere" arm was unreachable (a missing `release.key` hard-errored
first), found by the first machine ever to be in the designed
publish-without-release state, fixed as `Remote::installed()`. The schema
work established two standing rules: **migrations are additive from 3 on**
("delete the database" retired the day the box went live), and **a guarded
ALTER must be idempotent** — a half-run migration that wedges forever on
"duplicate column" turns a one-off transient failure into a bricked ledger.

**2026-08-07 — the second release door, and the operator puts down SSH.**
Steps 5 and 6, same evening. `/account` is the tenant surface: magic-link
sign-in (oracle-free, budgeted per account per day), a `__Host-`-prefixed
session cookie — the prefix is what stops a tenant's page on `alice.art`
tossing a `Domain=` cookie onto the gate, deferring the move-the-gate
question instead of forcing it — CSRF tokens derived from the session on top
of `SameSite=Lax`, release/unrelease driving the same `alias_set` the
release key drives, and machines-connected as the keys ledger with
`last_used_at` stamped on every authenticated call (a key that is used is a
machine that is alive, and a silent compromise shows as life where none was
expected). A session deliberately cannot publish. `factory-publish
disconnect` lets a credential retire itself — `POST /v1/disconnect`
authenticated by the key being revoked, which is what makes a compromised
laptop recoverable by its owner. The operator surface is a fourth scope
rather than a second session system: `Scope::Operate`, bound to the box (no
tenant), minted once over SSH, driving `/v1/admin/*` through
`factory-publish operator …` — users with queue depths, suspend/restore,
invites mailed by the box, every key, break-glass revoke, withhold. The two
surfaces are kept apart by the credential (tenant keys die at the admin
door, the operate key dies at the tenant door, tested both ways), and the
operator verbs are CLI-only, never MCP tools — suspending users is not
power an agent wields as a side effect of conversation. Proven live from
home the same hour: users, keys with last-used, invites, a suspend/restore
round-trip. What stays on SSH, deliberately: deploys, and minting a
replacement operate key if every one is lost.

**2026-08-07, evening — a second client is verified real, and the front door
stops needing anyone to remember it.** The factory's MCP surface was driven
from a Claude Code session over raw stdio JSON-RPC — the initialize
handshake, `tools/list`, and `bundle_render` through `tools/call` under the
`--root` jail — which turned "any MCP client can drive it" from a design
claim into an observation. Two things the drive confirmed structurally: the
seven tools are `bundle_*` only, with no drain tool for a client that owns no
quarantine, and a machine holding all four keys (the operator's own) is the
one place `bundle_alias` over MCP would really release — which is the
SELF-SERVE review's warning made concrete. `docs/SECOND-CLIENT.md` in the
factory repository is the resulting user path; the release workflow
(`release.yml`, static musl with an asserted-static gate) and the crates.io
packaging check landed beside it. On this machine, the front door became
standing machinery: `mecha-frontdoor.timer` runs drain → extract → triage
hourly (`scripts/frontdoor.sh`, ruminate's conventions), verified with a live
tick — drain acknowledged a held record at the gate, and the flagged request
correctly stayed parked for a human. The empty-turn recovery
(`StopCause::NoOutput`, the nudge, three attempts at a measured ~50%
per-attempt recovery) was committed the same evening, with the benchmark
tooling that motivated it.

**2026-08-07 — the first agent scorecard attempt, twice interrupted, once
useful.** The 05:22 launch was voided by the glibc trap (recorded in
BENCHMARK-RESEARCH); the 11:18 relaunch with the portable binary ran ~4 hours
before being stopped to free the box. What the fragment says: 21 trials
completed, **8 solved, 13 failed** — a real 38% at k=1 on a self-selected
early slice, not a scorecard. The `NonZeroAgentExitCodeError`s in it were not
crashes: `mecha run` exits non-zero on exhaustion, so harbor records a
40-turn `MaxTurns` run as an agent error (the verifier still grades the
artifact — one such trial scored 1.0). Relaunch is an open decision.

**2026-08-07/08, night — a stranger's file crosses the whole system, and the
factory grows a face.** The attachment arc shipped in seven staged commits,
each green in isolation: `FieldKind::File` in the manifest (sniffed magic as
the gate, a validation `Phase` so submit and complete are two named moments,
`take_attachments` as the filename quarantine no caller can skip); the box's
blob store where blob lifetime *is* queue-row lifetime, plus the sweep timer
the manual-only sweep needed before attachments made it a disk-fill vector;
the verified-requesters-only upload page (the one multipart route); the
drain protocol's attachment array and blob route; home's
blobs-before-record-before-ack with digest proof; and the frontdoor's
metadata-only brief — a poisoned-values test pins that even a regressed
drain leaks no filename to a privileged run. Verified twice: a 13-step
loopback rehearsal, then live — `letter` v2 with its CV field is deployed.

The same night the public surface got its chrome: header band, splash,
sign-in and account dropdowns, per-version artifact controls (which caught
"Make private" silently also un-aliasing — two decisions wearing one
label), a `/v/` version switcher, and finally the **viewer inversion**: the
signed-in viewer lives on the gate, where the session is, and frames the
bundle cross-origin — owner controls with the account page's own CSRF, a
return address honoured only for `/view/` paths, and bundles granting
exactly one new frame ancestor, the configured gate. The old
`frame-ancestors 'none'` rule's stated fear (a notebook framed by the gate)
was always enforced by origin isolation; what the directive governs is UI
redressing, and the only admitted pages are ones tenants cannot author.
Magic links became scanner-proof the same night, after Microsoft Safe Links
ate the first real sign-in (the trap below). Eight hand deploys; the CI
release tag is the standing fix.

**2026-08-07/08 — the operator's browser, and private pages that open to
named inboxes.** The parallel lane to the scheduling arc, sharing the tree
and the deploy. The **operator admin panel** resolved its queued open
question — the operate key never enters a browser: `factory-publish operator
signin` asks `POST /v1/admin/signin` for a one-time URL, whose scanner-proof
GET/POST interstitial becomes a 12-hour session in its own
`operator_sessions` table, bound to the *key id* under its own
`__Host-factory-operator` cookie. The session dies with the key (the lookup
joins on the key being live and `operate`), neither cookie means anything at
the other surface, and `/admin` renders accounts, invites, keys and
withholds with the same rows the CLI drives — signed out it offers
instructions and nothing to type into. A 25-agent review of the arc found
one theme worth naming: database failures dressed as benign states — a
sign-out that survived a failed revoke, a valid session rendered as
signed-out, empty security ledgers rendered as truth, a one-time link burned
by a failure between redeem and session-mint. All fixed (`86df0a7`), the
last by making redeem-and-mint one transaction — and the same
burn-the-link flaw was then found and fixed on the *tenant* sign-in
(`d5b833d`), so all three sign-ins now share the
spend-only-when-the-session-lands shape. **Private sharing** resolved the
last queued design pass (`0573bb1`): a grant names an *email* — a `shares`
row per (owner, bundle, address), managed from the viewer's Manage menu,
budgeted per owner per day. The box mails the bare viewer URL; the reader
proves the inbox through the magic-link machinery and becomes the third
session surface (`__Host-factory-viewer`, joining on an email — never a
user, never a key). Bytes still never meet identity on artifact origins:
the gate mints a short-lived capability and frames `/g/<cap>/`, whose
lookup re-proves the grant at every fetch — revoking a share kills the
bytes mid-page, and there is a test watching it happen. Readers see the
live version only; an owner's private preview now frames real bytes where
it used to frame the world's 404. Oracle-free throughout: one sign-in page
for every private-or-absent viewer URL, one answer from the form whoever
asks. Deliberately *not* built: folding the three parallel session surfaces
into one parameterised abstraction — the tables are the boundary, and
`Db::signin`'s doc comment says so to whoever next tries to deduplicate it.

**2026-08-08 — the scheduling instrument, designed to deployed in two days.**
The youcanbookme replacement and its when2meet half, end to end:
`calendar_freebusy` on the unified mail surface (fail-closed at every parse —
an unreadable calendar is never a free one); the pure availability engine,
moved into `mecha-manifest` when it turned out to be contract; the slot push
under a new fifth key scope (`slots`, the narrowest, sized for the systemd
timer it lives beside); the booking page at `/s/<handle>/<id>` with its
two-phase atomic claim (soft hold at submit, conversion at the magic-link
click, a lapsed hold *deleting* its queue row so no phantom meeting ever
reaches a calendar); the manage capability with every state answered
honestly; and the group poll — box-minted capability URLs over names only
(addresses never leave home), tri-state answers that work with JavaScript
off, `rank_poll`/`clean_winner` refusing to auto-book past a tie, an
if-needed, or a silent participant. Mid-arc the mail design inverted at the
user's direction: booking mail comes from the user's own account as the
provider's *native* invite (attendee + notifications on), SES narrowed to
account plumbing, and the hand-rolled ICS module, the raw-MIME work and the
deliverability matrix were deleted unbuilt. Deployed the same day and proven
by a live self-test: book → confirm → Outlook event blocking real freebusy →
cancel via the manage link → native withdrawal, with the page, the box and
the calendar agreeing at every step. The deploy also carried the parallel
lane's admin panel and private sharing onto the box. Two timer lessons came
home immediately: drain rides the fifteen-minute sweep (an invite must not
wait on the hourly front door), and the sweep names its account because a
timer cannot ask.

**2026-08-08 — the factory gets a documented face, generated by the code it
documents.** The docs site had been on the factory's own domain while
documenting only the harness; it now opens a Factory section with a live
component gallery. Nothing on it is drawn: `mecha-manifest`'s `gallery`
example walks every field kind, loops over `BUILT_IN` themes, and writes 46
pages — the forms, the rejected view whose eleven messages come out of the
validator rather than a designer, the conditional and multi-step pages, the
upload page, all five starters, and (after the scheduling arc) the week view,
a week on, and a poll. The output is committed at `mecha-factory/gallery/`
and CI regenerates it, refuses a diff, and runs the publish gate over the
whole tree, so a renderer change arrives as a reviewable diff of rendered
HTML — a golden-file test wearing the right clothes. Two guards keep it from
rotting into a smaller manifest format than the one that ships: the generator
matches `FieldKind` exhaustively, so a new variant fails to compile and then
fails the coverage assert until a real field exists; and themes come from the
built-in array, with the docs page building its palette buttons from the
gallery's own `index.json` rather than a list of its own. `mecha`'s side is
`sync-gallery.mjs` on the prebuild hook — sibling checkout first, public
tarball otherwise, warning rather than failing — and a `GalleryFrame`
component that stamps `data-theme` on the framed document, because an iframe
reads `prefers-color-scheme` from the operating system and would otherwise sit
dark inside a light page. Rendering the booking states early paid twice: it
caught that `book.toml` had been rendering through `form()`, a page the box
never serves, and it surfaced two gaps in the not-yet-built POST error path —
the summary lists raw field names where a form's lists labels, and no `_slot`
comes back checked, so a visitor who mistypes an address loses the time they
picked. One decision worth not undoing: **the gallery's clock is a literal.**
Booking was the first surface here with a time in it, and a golden file
rendered against `Utc::now()` differs from itself daily until somebody deletes
the check that keeps saying so.

**2026-08-08, evening — the scheduler grows a front end worth handing out.**
Committed as `1d531a8` in the factory repo and deployed to the box the same
evening. The booking page's clunk was structural, and each piece came out
structurally: the meeting length became a server-side `?mins=` link switch —
week paging's exact shape, so it dedupes with JavaScript off and each start
time renders once as a mono time chip; the details form hides behind a CSS
`:has` reveal until a time is picked (browsers without `:has`, and readers
with CSS off, get the whole page — the reveal has no script behind it), with
a picked-time chip written by `booking.js` as pure restatement. The page is
live now: `GET /s/<handle>/<id>/slots.json` serves the same subtraction the
POST judges (`http/booking.rs:174`, `no-store`), and the script polls it
every 30s and on tab re-focus — a slot someone else holds collapses out of
an open tab, a taken *pick* says so out loud, and fresh slots reload only a
pristine page (anything typed downgrades the reload to an offered link),
which also closed the stale-tab item as a side effect. The POST error path
gaps the gallery had surfaced are fixed at the server (`page_back`,
`http/booking.rs:420`): a rejected submission keeps the typed values, names
failed fields by label, and re-checks the picked `_slot`; the race loser and
the lapsed hold keep the visitor's details too — losing a slot must not
also cost the answers. The poll grid got its polish pass: `poll.js` upgrades
the tri-state radios to tap-to-cycle cells with drag painting (the anchor's
new state decides the whole stroke, when2meet's mechanic), autosaving over
fetch against the same POST (`Accept: application/json` → bare 204/409, the
radios and Save button staying as the JS-off path), and heat rendered
server-side as six discrete `heat-N` classes with "n of m yes" in text —
classes because the gate's CSP forbids inline styles, text because colour
must never carry the information alone. Verified by screenshot across both
themes, both schemes, and mobile, plus a live sweep test against a real
HTTP server; eight new tests took the factory workspace to 325.

**2026-08-08, evening — the admin panel grows an email door.** A parallel
session's arc, committed as `347142b` beside the scheduler pass and
deployed the same evening: with `operator_email` in `factory.toml`, a
signed-out `/admin` offers one button that mails a one-time sign-in link to
the address only configuration knows — admin from any browser, the operate
key never pasted anywhere. The link redeems into the same session the CLI
door mints, anchored to a well-known `email-door` key row that can never
authenticate a bearer, which keeps "an operator session resolves to a key"
true and makes revoking that row the door's kill switch. Links are budgeted
per day; sessions from both doors last 30 days and roll on use. The detail
in that repo's `DEPLOY.md`.

**2026-08-08, late evening — a release becomes a tag, and the deploy becomes
a command.** The email door's deploy was the last hand-deploy, and it earned
the pipeline that ended the era: the locally built binary crash-looped the
service (`status=203/EXEC` — the workstation is aarch64, the droplet x86_64),
and the config line appended over SSH landed inside `[mail]`, where TOML
swallowed it silently. Both repos now release by tag: a `v*` tag re-runs the
suite, refuses a tag that disagrees with the workspace version, and publishes
the crates in dependency order through crates.io Trusted Publishing — all six
live at 0.1.0 (`mecha` itself is taken; the CLI ships as `mecha-cli`). The
factory tag also builds a static musl binary whose workflow *refuses to ship
it dynamically linked* — a guard that paid for itself on its first verified
run, catching a crt-static toolchain-default drift that would have replayed
the morning's outage; `RUSTFLAGS` now states the contract instead of
inheriting it. `scripts/deploy.sh`, installed on the box as `factory-deploy`,
makes a deploy one command — download by tag, checksum, prove the binary and
`factory check` the config while the site is still up, swap keeping
`factory.prev`, health-check, roll back unaided on failure — and `v0.1.0`
went out through it, verified live by the queue holding an empty `?wait=4`
long-poll the full four seconds. The workflow the day taught is
`CONTRIBUTING.md`'s in both repos now: branch per arc, one worktree per
session (two sessions shared a tree that afternoon, and production code
spent an hour existing only in a stash), PR-gated landings, rebase merges,
and no attribution trailers.

**2026-08-08, night — review moves into the TUI, and a run's drafts meet
you at the door.** `/outbox` and `/frontdoor` landed as modals on the
`/triggers` pattern — the store read directly for display, every mutation a
`mecha …` child process, long work (a send's MCP startup, an extraction, a
triage run) spawned detached with the store as the record. The decisions
that differ from `/triggers`: **every send confirms**, because it is the
one keystroke that cannot be taken back, and a tainted draft confirms in
red with its full arguments on screen; a publish's `edit` is refused with
the real action named, as on the CLI; the frontdoor detail prints the
stranger's prose under its framing, because a person reading a terminal is
the safe context; `close` refuses an empty reason. On top of that, the
status line grew an ` outbox N ` badge, and `/review now|later|auto` now
decides what happens when a run *this session started* finishes having
staged drafts: open a modal scoped to exactly those items, just notice, or
release them — where the mode is set only by slash command (a
model-interpreted "preapproved" is a directive an injected page could also
write), scope is an id-diff between submit and completion so the overnight
backlog is never touched, tainted drafts never auto-release (the approval
predates whatever armed the taint), and an errored or early-stopped run
releases nothing. Detached work reports back instead of asking for a
reopen: a watch list polls the *stores* (never the child — the store is the
record) on a one-second tick while anything is live, and a landed release
or a moved request becomes a transcript notice plus a badge and modal
refresh; a watch that outlives its cap is dropped with a still-working
notice so a wedged child cannot pin the fast tick. And staged summaries
learned to lead with who and what: `summarize` in core now prefers the
conventional argument names (`to`, `subject`, `title`) over raw compact
JSON — keyed on argument names, never on the tool, so the store stays
tool-agnostic. Along the way a latent bug: the TUI's trigger edit ran
`$EDITOR` through `.output()` — a pipe for a screen, a closed stdin for a
keyboard — found only when a second editor shell-out forced the question;
the suspend dance is now one helper and interactive children inherit the
real terminal, with only stderr captured. `mecha-cli` went from 83 to 100
tests, including the tainted confirmation rendered through the real draw
path.

**2026-08-09 — agents can leave each other messages, and taint rides
along.** Researched (Claude Code's cross-session messaging over per-session
sockets; A2A/ACP/MCP-Tasks; Morris-II and Prompt Infection on cross-agent
injection — `docs/MESSAGING-RESEARCH.md` holds the survey, the design, and
six decided questions) and then built the same day: `mecha-core/src/mailbox.rs`
is a file-based mailbox under `~/.mecha/messages/<recipient>/` on the
outbox's store conventions, addressed by producer name, claimed under a
per-recipient flock at the top of a turn and folded in at the steering fold
point. The piece no deployed system had: the harness stamps the sender's
conversation taint on every message (the conservative per-turn snapshot,
via `ToolCtx::taint`, `Option` so unstamped fails closed to fully tainted)
and delivery merges it into the receiving conversation before the body
lands — a hop between agents launders nothing. Provenance header says it is
another agent, not the user; untrusted senders get the standard wrapper;
unknown taint reads as armed. `[messages]` in config is global-only (the
section is stripped from a project `mecha.toml`, loudly); attended surfaces
default to `hold` with a waiting-mail notice, unattended runs to `accept`;
eval forces `--no-messages`. Full mailboxes refuse rather than drop-oldest
— against Claude Code's choice, because the sender is an agent that can be
told "full" — with `mecha msg dismiss` as the human's way to clear a
backlog no run will claim; identical pending sends deduplicate as the loop
brake; malformed files quarantine as `.bad` instead of wedging the mailbox
(Claude Code shipped that bug). `mecha msg send/list/show/dismiss/agents`
is the CLI, with `agents` reading a per-session liveness registry that
generalises the trigger `RunMarker`. Phase 2 (TUI badge and `/messages`
modal, live delivery into an in-flight run) is scoped in the research doc,
whose §5 also records what the machinery unlocks beyond messaging: inbound
webhooks and file watchers were both blocked on exactly this labeled
taint-carrying prompt path, and headless steering of a trigger run is now
`mecha msg send` away.
**2026-08-09 — the benchmark run is read, and the loop stops dying of
recoverable things.** The salvaged 21-trial fragment of the 2026-08-07
Terminal-Bench run was diagnosed trial by trial — 8 passes, 5 genuine model
failures, and 8 deaths the harness owned some share of — and the diagnosis
answered a standing four-way question: the trifecta interlock costs the
benchmark *nothing* (no trial ever armed the untrusted leg — the surface has
no web tools and `shell` is not an untrusted source); the 32k context is
genuinely overloaded, with the flat 24 KB per-turn output budget as the named
mechanism (8–12k tokens of numeric data, larger than the
threshold-to-window gap, so `path-tracing` leapt from under the threshold to
45k tokens in one turn and died); the loop had real structural gaps; and
compaction itself was mostly exonerated — the two heaviest trials compacted
repeatedly and both passed. Five fixes landed on one branch (PR #21), each
with a regression test that fails on the old behaviour: overflow recovery no
longer disables itself when a summary was not worthwhile (the give-up flag
gated the whole arm, so the *next* overflow died raw); the empty-turn
allowance resets on productive turns instead of accumulating (two trials died
`NoOutput` mid-task with retries spent hours earlier, while two others
recovered from a nudge and passed — and the empties persisted with
`--reasoning-budget 4096` active, so the server-side fix reduced the problem
rather than ending it); `mecha run` exits non-zero only for produced-nothing
runs (every exhausted stop exited 3, which Harbor records as an agent crash —
`headless-terminal` hit MaxTurns, was counted an error, and its verifier
scored the work 1.0); transcripts survive crashes and record rewrites (a
`rewrite` session record carries the compacted state, `load` replaces, the
taint timeline clamps stale positions toward over-taint); and the tool-output
budget derives from the context window when unpinned — 12,288 bytes at 32k,
the old 24,000 at wide windows. The bench adapter now captures stderr and
`MECHA_LOG=debug` beside the transcript. The full write-up is
`docs/BENCHMARK-RESEARCH.md`, "The 2026-08-07 subset run, diagnosed".

**2026-08-09 — mecha grows a remote control, and it is a Slack thread.**
Researched, designed and built in one session, then run against a real
workspace: `docs/SLACK-RESEARCH.md` is the evidence and `docs/SLACK-DESIGN.md`
the decisions. **Socket Mode**, so home dials out — no inbound port, no
certificate, no tunnel, and no request signature to verify, which is the same
argument `mecha-drain.service` already made about the factory queue. A fourth
crate, `mecha-slack`, holds the transport and **cannot depend on
`mecha-core`**, so the invariant is checkable by reading a manifest rather
than by reviewing diffs; the front-end that knows both sides lives in
`mecha-cli/src/slack/`, beside `tui/`. **Two trust tiers and no third** — an
allowlist of Slack user ids bound by a nonce the local CLI prints, which
proves shell access to the machine where an email address proves only what
the workspace claims; everyone else is ignored. A thread is a `Conversation`,
so the interlock gets the right granularity for free; a thread's jail is a
subdirectory of one `slack` work producer, so retention reaches it. The
answer streams with a `task_update` card per tool call, keyed on the
`tool_use` id so a call's lifecycle is one card changing state. Approvals are
durable cards rewritten into terminal records, with **"Allow for this run"**
after the first real task raised seven of them. Drafts a run stages come back
as review cards with Send and Reject, scoped by an id-diff of pending outbox
ids — **which closes `PUBLIC-SURFACE-DESIGN.md` §11's deferred "phone UI for
releasing outbox drafts", and it needed no home-side server at all.** Files
go both ways: an attachment lands in the jail and is named to the model as a
path (so taint arms through `fs_read`, the route that already exists), and
what a run creates is uploaded back. `mecha slack notify` puts a trigger's
briefing on a phone for the price of a config line, and
`scripts/mecha-slack.service` is the third always-on unit. One flock means one
connector. Verified live on `cosanlab`: binding, streaming, approvals, mode,
orphan recovery, attachments in, artifacts out.

Two things were left undone deliberately and are named in the handoff:
`ask_user`, because it is a tool and the registry belongs to the agent that
serves every thread; and per-thread isolation for MCP tools, because servers
are spawned once with the agent. Both want an agent per thread, and an MCP
startup per thread with it.

The arc also changed `mecha-core`. `Decision` gained a third variant,
`Blocked(String)`, rendered `"Blocked by policy:"` — see the trap below —
and `process_alive` moved to `mecha_core` when a second and third subsystem
turned out to want the pid range check that is its whole correctness.

**2026-08-09 (night) — the agent gets the factory's other four capabilities,
and drift becomes a build failure.** `factory-publish` had grown to twenty
commands while its MCP server stayed at seven, all `bundle_*`: notebooks,
request types, availability slots and **polls** were unreachable by any agent,
and nothing anywhere failed while that was true. The way anyone found out was
asking mecha to make a poll and being told, correctly, that it had no such
tool.

Why it stayed drifted decided the work. The capabilities were command bodies
inside the *binary*, where `mcp.rs` in the library could not reach them —
`polls_command` alone was 470 lines. So `polls.rs` is the extraction, and the
rule it encodes is the transferable part: **a capability is a function, and a
front end is a printer.** The CLI's output and its `--json` shapes are
unchanged, which is why the printers still compute from the same pure tally
functions rather than re-reading a serialised tally — a "refactor" that
quietly reformats a number an agent workflow parses is not one.

Eight tools joined: `poll_create` and `poll_meeting_create` (two tools rather
than one with a mode flag — the modes take entirely different inputs, and a
mode flag is what a model gets wrong), `poll_status`, `poll_close`,
`notebook_render`, `type_check`, `type_push`, `type_list`. Everything that
mints a public URL carries `openWorldHint` and is named in `[outbox] tools`, so
it stages instead of reaching the world. The meeting poll takes freebusy as a
**file path**, since MCP cannot pipe stdin the way the CLI does; the
under-an-hour freshness and horizon refusals sit untouched behind it.

**A poll collects other people's words, so the front door's rule was borrowed
whole.** `poll_status` returns typed tallies and the *counts* of free-text
answers, never the answers: `Status::for_privileged_run` is the function, and
there is deliberately no argument that hands the prose over. `polls export` is
excluded from the tool surface for the same reason — it would write those words
into a file `fs_read` cannot tell from bytes we wrote ourselves, which is the
boundary with an extra step rather than a boundary. A residual is recorded
rather than papered over: question prompts still come back from the box, so a
compromised origin could rewrite the user's own question text.

The durable half is `surface::REACH`: every CLI command is either exposed as
named tools or excluded with a written reason, and three tests fail the build
when a command is neither, when a row names a tool the server does not serve,
or when a served tool no row accounts for. `drain` had been the one
*documented* exclusion; `slots push`, `operator`, `connect`, `serve` and the
rest now say in writing why they are not an agent's. The test was verified
non-vacuous by deleting a row and watching it name the gap — which is the only
thing that distinguishes a coverage test from a comment.

The mecha side took two fixes, the second found by the first. `local_paths`
learned `spec` and `manifest`, so a staged poll or form has a file to open
rather than a JSON blob to squint at. Then staging the first real poll end to
end reported its spec — a file sitting right there — as "⚠ gone", because
`show` resolved a relative argument against wherever the reviewer was standing
while `send` had always used the recorded jail. See the trap below.

**2026-08-10 — the poll page is looked at, and two beliefs do not survive it.**
The session began as "expose the factory's tools" and turned into a review of
what those tools actually produce, which is the only reason any of the following
was found: every defect here was visible on a rendered page and invisible in the
source.

**The gallery's survey page had no survey CSS at all.** Every page in that family
links one asset named `booking.css`, and two builders produced it — the booking
pages omitted `SURVEY_STRUCTURE`, the survey pages included it, and the gallery
writes the first one it meets and marks the job done. So the page a reader learns
the component from shipped with none of its rules: list markers where
`list-style:none` was meant to be, rank buttons wearing the full-size accent pill
that `button` is. The live box built its assets down a third path that happened
to be correct, which is exactly what kept it hidden. `page_style` is the single
definition now, with a test that holds a really-rendered page against it and
names the rules that must be present.

**Ranking's drag moved exactly one row.** Pointer capture was taken on the grip;
the first reorder moves the row's `li`; `insertBefore` on an attached node is a
remove and an insert; removing the capturing element releases the capture. Every
`pointermove` after the first went nowhere. Capture moved to the list, the one
node in the widget that never moves, and the row now follows the pointer and
swaps on midpoints rather than on whatever is under the cursor — which returns
the dragged row itself once it moves, a stall of its own.

Likert and VAS were laid out by `flex-wrap`, which is not a layout: the fifth
point of a five-point scale wrapped to a second line, where it stops reading as a
scale, and the VAS anchors flowed inline so "100 — …" landed below the slider
describing nothing. The scale became a grid of equal columns; the anchors got
half the width each. A choice became **a card you press** rather than a dot you
aim at — the radio stays in the markup and the tab order, since it is what posts,
what a screen reader announces, and what works with the script blocked, but
`:has(:checked)` moves the selected state onto the whole card.

**Two features, both presentation kept out of meaning.** `layout = "horizontal" |
"vertical"` lives on `PollQuestion` rather than in `QuestionKind`, because the
kind is what a question means and a tally must never move because somebody
rearranged a page. `media = { src, alt }` puts a picture on a question or an
option — and *where the bytes may come from was decided by the
Content-Security-Policy, not by taste*. Every class sends `img-src 'self' data:`,
so the obvious design (publish a bundle of figures, point at it) does not
survive: the artifact host is a different origin and the browser blocks it. An
off-origin `src` is refused when the spec is parsed. What remains is inline
images capped at 512 KB, which is a generous figure and a hopeless photograph —
recorded in the handoff as needing an asset endpoint on the box rather than
quietly shipped as though it were finished.

**Two reversals, both of things this project had written down as true.** The
notebook export does **not** execute the notebook: measured on marimo 0.23.16,
where a cell body that writes a file does not write it, a statement at module top
level does not run, and a notebook importing a nonexistent package and raising
`SystemExit` exports cleanly with exit 0. `export html-wasm` parses the file
without importing it. That claim had been load-bearing — it was the stated reason
the notebook template stayed off unattended paths. And `poll_status` went back to
returning free text, after a first version withheld it on the front door's
reasoning; in a poll the prose *is* the data, and `openWorldHint` already marks
it untrusted and arms the interlock exactly as a mail body does, so withholding
was stricter than mecha's treatment of the user's own inbox.

The documentation caught up in the same pass: `factory/onboarding.md`,
`factory/artifacts.md`, `factory/notebooks.md` and `features/slack.md`, plus the
layout and picture sections in `polls.md` — four of the gaps a reader hits in
their first hour and could previously close only by reading source.

**2026-08-10 (later) — two releases, a deploy, and a ceiling that was never
there.** `v0.1.1` and `v0.2.1` are cut and published: four mecha crates at
0.1.1, three factory crates at 0.2.1, and a checksummed static `factory` binary
attached to its release. That closes "verify the release workflow" and the
crates.io split, both open since 2026-08-07. `factory-deploy v0.2.1` then ran
the whole procedure against a real release for the first time — download,
checksum, prove the binary and the config while the old one still served, swap,
health check — so the poll rendering work is in front of visitors rather than
sitting in a repository. Home switched to installing from crates.io rather than
the repo build, which is the better habit once tags exist: it installs exactly
what the world gets.

Then the day's largest finding, which was not planned work at all. `-c 32768`
had been pinned for three days by a single bad afternoon, and the rule written
from it — *do not raise `-c`* — turned out to be four times larger than its
evidence. Re-measured across 32k, 64k, 128k and 256k at matched prompt lengths:
**`-c` costs nothing.** The server now runs at **262,144**, the model's whole
trained window, and a needle at 21% depth in a 188,546-token prompt came back
exactly. The compaction threshold moved from 21,845 to 174,762 as a side
effect, which is recorded in the handoff as a decision somebody still owes.
`[slack] tools` was emptied in the same session, so a Slack thread now carries
the same surface as `chat` — the token argument that justified narrowing it was
real, and stopped mattering when the window grew eightfold.

**2026-08-10 (evening) — the empty turns were half ours, and the model gets
its reasoning back.** `v0.1.2`. The Terminal-Bench subset was relaunched at
the 262k window with `max_turns` 80 and a 2.0x agent-timeout multiplier; 27 of
75 trials in, it had 12 passes, five agent-timeouts and one trial lost to a
crash that turned out to be a bug worth the whole run. Reading that crash
opened the day's real finding.

`reasoning_content` appeared nowhere in the tree. A local reasoning model's
entire think channel was being discarded on every turn — invisible in the TUI,
absent from every transcript, and unavailable to the diagnosis that had been
chasing "empty turns" since 2026-08-07. Replaying the exact prefixes that went
quiet reproduced one, and its reasoning held a complete, unparsed tool call;
in a second case, 120 characters that were *only* a tool call with no
deliberation at all. The model emits its call before closing `</think>`, so
llama.cpp files the whole turn as reasoning and reports a clean stop
(ggml-org/llama.cpp #20837, #22684, #20809 — all unfixed, same failure
reported against ollama). Which killed the standing explanation: 120
characters is nowhere near any limit, so `--reasoning-budget`, `max_tokens`
and the window were never relevant, and every mitigation aimed at "the model
reasons too long" had been aimed at the wrong failure for three days.

Then the half that was ours. Because the history sent back stripped every
`<think>` block, the model was shown turn after turn of itself apparently
calling tools without thinking, and it obliged. Same server, same template,
same prompt, varying only whether the history carried reasoning: **6 of 6
empty turns without it, 0 of 6 with it**, p ~= 0.001 on a reproducer that
fails byte-identically. A third-party replacement chat template fixes it too
(6/6 to 0/6) but only by *instructing* the model, and only for Qwen; the
history fix addresses the cause and names no vendor anywhere.

So reasoning now decodes to a `Block::Thinking`, streams as `ThinkingDelta`,
rides back to the provider on the next request, and is recorded — with the
full trace of an empty turn going to the debug log, since such a turn reaches
no transcript. Preserving all of it is affordable because the prefix cache
absorbs it: measured, 5,000-token prompts prefill 16-211 tokens, better than
95% reuse. That reuse had been invisible because `decode_usage` never read
`prompt_tokens_details.cached_tokens`, which is now read and — the part that
matters — *subtracted* from `input_tokens`, because `total_input` sums the
tiers and the compaction threshold reads the sum.

The crash that started it was its own bug: `pytorch-model-recovery` describes
itself as a bulleted list, so its instruction opens with `- `, clap read it as
a flag, and the run exited 2 before starting — which Harbor scores 0.0,
indistinguishable from a model that tried and failed.

**2026-08-11 — the fix is measured, and it moves the constraint rather than
removing one.** The v0.1.2 relaunch ran 46 of 75 before being stopped, and on
the 28 tasks both runs reached: passes **13 → 15**, wall clock **10.5h → 6.0h**,
timeouts **5 → 2**, crashes **1 → 0**, empty-turn nudges **41 → 0**. The speed
is the mechanistic part — per-turn cost collapsed toward a floor, and
`break-filter-js-from-html`, already at that floor at 14s/turn, did not move
while trials at 35–100s/turn fell to 6–39s. The score is the modest part, and
its shape is one story: three of four flips to a pass were trials the old run
stopped *too early* (3 turns, 12 turns, one that never finished), and the one
flip away was the same persistence overrunning a wall clock. The model did not
get smarter; the harness stopped hiding what it could already do.

Two open questions closed with numbers rather than argument. **Unbounded
reasoning replay does not drive compaction** — 1 trial in 28 before, 2 in 46
after, and the trials hit the turn cap long before the 174,762-token
threshold, so the "bound it to the last N turns" design would have solved
nothing. And **`max_turns` is now the binding constraint**: 26% of trials
ended at exactly 80, passing 30% against 50–67% in every band below. 80 was a
ceiling this project chose while the real constraints were empty-turn waste
and the clock; removing those moved the binding one and nobody re-derived the
number. Terminal-Bench's own default is 200, so the third run uses that.

One correction worth recording because it nearly became a change: the session
proposed adding tool-output offloading, and `ToolCtx::spill_dir` had done
exactly that since 2026-08-05 — 12 of 46 trials spilled, with the marker
naming its own `fs_read` recovery. The follow-on proposal, to lower the
budget so spilling fires more often, was refuted by this repo's own
`CONTEXT-RESEARCH.md`: one arm cut tool-output tokens 38.4% and cost **6.8%
more**, r = 0.154, because trajectory length dominates and re-reading spilled
output costs turns. Reasoning from context *occupancy* without checking the
*cost* evidence already collected here is what produced both errors.

**2026-08-10 (night) — an artifact gets the URL a person can open, and the
review bot turns out never to have run.** Two arcs, one on each side of the
repo boundary.

The factory's publish path reported one URL for a bundle — the bytes on the
artifact origin — and reported it whatever the visibility. Since a publish key
cannot move an alias by design, the state every agent publish *lands in* is
private, where that URL answers 404; the tool said "It is live at" about it,
and a model repeated the dead link to whoever asked. Measured against the live
box before anything was written: `…/b/morning-brief/` → 404, and the gate's
viewer page for the same bundle → 200. The server now answers with both
(`viewer_url`, `viewer_version_url`, minted by one helper so the publish and
alias responses cannot drift), the publisher funnels every report through one
`Mirrored` type, and the MCP tool descriptions say which URL is for a person —
that last part being where it actually gets fixed, since a model repeats
whatever string the tool returned. Documented in `artifacts.md` as a named
section, because the page taught the wrong URL as *the* one you send someone.

Three things fell out that were not the arc. Following the URL a publish
reports, while signed out, landed an owner on the reader sign-in form — which
mails a link to an address a *share* names, and an owner holds no share to
their own bundle, so the form answered "check your email" and nothing came;
the page carries the tenant sign-in corner now, which leaks nothing because it
is identical on every refusal. The account page linked every bundle through
`Role::Artifacts` unconditionally, so each published notebook pointed at an
origin it does not live on and was rescued by a redirect. And `factory-publish
push`, which has no `--visibility`, read one out of the *local* store and sent
it — a store that never hears about a release made from the account page, so
the next push silently took public bundles down. Visibility is `Option` end to
end now, and `None` omits the field, which is the box's own spelling of "leave
who may read it alone" and was always what the server implemented.

A 26-agent review over the branch found ten issues, nine fixed. The worst was
the arc's own: a `for_a_person()` fallback substituted the bytes URL when the
box named no page, so the private sentence read "only you can open it: <URL>.
<the same URL> serves nobody" — the exact lie the change existed to remove,
reinstated in the one arm the deployed box still exercises. Its guard had
certified it, because the test only covered the single state where the
substitution happens to be true.

The other arc was the CI bots, and it began from a wrong premise — that
mecha-factory needed switching from API billing to plan billing. It had never
had the workflows at all. Checking the run logs to port them turned up three
things: the model was `claude-sonnet-5` everywhere, since neither repo ever
pinned one; `claude.yml`'s condition used a literal block scalar and the job
had **never once run** (a real `@claude` comment left it skipped in 0s with no
steps); and the review job had failed on every run it had ever made — 31
failures in its last 40, identical before and after the OAuth switch, so
subscription auth neither caused nor fixed it. That last was the credential:
an API key that had presumably run dry, then an OAuth token that was never
valid. A fresh token took the same job from `is_error: true`, 1 turn, 304ms to
a real nine-turn review. Both bots now name `claude-opus-5` and both work.

---

**2026-08-14/15 — Slack acts, the doctor reads across the stores, and 0.1.3/0.1.4
ship.** (Bridge entry, reconstructed from `CHANGELOG.md` — the sessions that
built these predate this handoff pass; the changelog entries for 0.1.3 and
0.1.4 are the detailed record.) The hardening branch merged 2026-08-14 (ten
review findings fixed), Slack gained executable actions behind the closed
`Action` enum with the tainted two-step, and `mecha doctor` landed as the one
reader over every store after the 2026-08-11 incident where a revoked OAuth
token took scheduling down for three days while five stores each recorded the
distress correctly and nothing read across them. v0.1.4 was tagged 2026-08-15
— and the deploy that followed is the reason the `update` skill exists: the
tag published crates while `mecha --version` still said 0.1.3, because
nothing had run `cargo install`.

**2026-08-16 (afternoon) — DeepSeek's harness is read, and mecha keeps four
ideas.** A survey of the just-released DeepSeek Harness (dsh) produced a
ranked steal-list, and the same day shipped it as v0.1.5: `recall` (the
session transcript is searchable, including what compaction rewrote away —
taint-neutral by construction, since everything it returns entered the
conversation once and its arrival is what armed the interlock), the cache
lens (per-run observer that names why prompt-cache reuse broke and warns only
on unexplained re-payment), the Landlock sandbox backend (no-privilege file
confinement that deliberately never earns the interlock relaxation, because
UDP is unrestrictable at any ABI), and the recording-gap fix
(`Conversation::rewritten` — a run long enough to compact itself no longer
loses its own head, because `record_run` now takes the conversation and walks
the pre-rewrite states a caller could otherwise skip). The survey also found
dsh has no taint tracking, no send-sink concept, and no summary validation —
the trifecta work has no counterpart there. Spill, the fourth idea, turned
out to have shipped here already (`cap_result`), which the research memory
now records against its own earlier wrong claim.

**2026-08-16 (evening) — the knowledge graph becomes a public sibling.** pkg
went public as **mecha-graph** in a clean-room extraction: fresh history
(the private repo's 137 commit bodies narrate live measurements about named
people — no filter makes a journal safe), a canonical replacement map that
preserved every test's linguistic property (Ana⊂Anastasia⊂Lana for the
word-boundary family, Ada B/Ada B. for initials dedup, the June trio for
same-first-name disambiguation), a synthetic eval world measured at 23/24
with one deliberately hard bridging query left as headroom, and a
private-only denylist gate plus export script that every publish must pass.
Two independent audits ran, the second given no list of what the first
removed — and it found what the first had rationalized: a "replacement"
address that was a real Vermont locality, a real founder kept under a
public-figure excuse, and household facts renamed rather than fictionalized.
Three crates published to crates.io (`mecha-graph` took the bare name; the
CLI installs as `cargo install mecha-graph`), the repo flipped public the
same night.

**2026-08-16 (night) — the graph's names reach all the way down, and 0.1.6
ships.** The rename went end to end: crates and binaries
(`mecha-graph-core/-cli/-mcp`, dirs included), env vars (`MECHA_GRAPH_*`),
the store (`~/pkg` → `~/.mecha-graph`, migrated live and verified by reading
13k episodes back), and — the piece that needed a mecha feature — the tool
names: `prefix_tools = false` on `[[mcp]]` registers a server's tools under
their raw names, so the model calls `kg_search`, not `pkg__kg_search`.
Unprefixed is a promise of distinct names, enforced by a loud startup
failure on collision (a prefixed name always contains the `__` marker, so
the cases cannot be confused). The eval followed — `graph-cases.jsonl`
grades the surface that actually runs, with the comparability break across
the rename stated in the case file rather than discovered later — and
running the renamed set immediately caught a latent `--mcp-file` bug that
had silently broken fixture evals since MCP servers started spawning in the
run's workspace. v0.1.6 tagged, published, deployed, and the graph now
installs with mecha (`~/.cargo/bin/mecha-graph-mcp` in the config, the
update skill's step 1, and the README's install block).

**2026-08-17 (small hours) — the signup door opens.** mecha-factory v0.2.5
tagged and deployed via `factory-deploy` (download, checksum, prove, swap,
health-check; `factory.prev` kept): the two commits that had sat unreleased
on main — `d11cf10`, anyone may ask for an account at
`gate.mecha-factory.ai/signup`, and `9823f26`, redirect hosts get their own
certificates — are serving. Verified live: `factory --version` 0.2.5,
`/signup` answers 200 with "Create an account". Found during the handoff
pass, deployed on the owner's word an hour later — which is the handoff doc
doing its job.

**2026-08-17 — the operator gets the door back, with the arithmetic in
view.** v0.2.6, designed and deployed the same night on the owner's call:
an ask now files a `signup_asks` row (schema 15) and nothing else — the
certificate budget is spent at *approval*, on the panel, where a new
Signups section shows committed-of-40 split into accounts and pending
invites, the free count, and the hour the oldest slot returns, computed by
the same function the approval handler enforces so the panel can never
disagree with the refusal. Approve mints and mails through the one
definition every mint uses; Deny closes silently, which is what the ask
page now promises; a spent week queues instead of pausing — the
forty-burner-addresses attack that used to spend the week's certificates
now fills a queue the operator bulk-denies, and the budget never moves.
Same bytes for every address, still. 486 tests, the approval journey
end-to-end among them.

**2026-08-18 — mail learns to triage, and the rules budget stops being one
number.** `mecha-mail` could read, send and write the calendar but not
archive; the whole write surface was send/reply plus calendar CRUD, so any
triage UI over it could move a cursor and draft and never empty an inbox.
`mail_triage` adds archive/read/unread/spam/trash as a closed `TriageAction`
enum, thread-level, landing in a capability quadrant the surface did not have:
`destructive` but not `external_send`, because it mutates the user's own
mailbox and reaches nobody — so it must never sit in `[outbox] tools` (staging
it would make the loop review a queue in order to fill another queue) and must
not be `readOnlyHint` either, or a read-only trigger could empty the inbox at
07:00. `assert_tool_surface` grew a third slice asserting both negatives.
Tagging was deliberately left out of the provider entirely: Gmail labels and
Graph categories are different objects, a tag meaning different things per
account fails at the one job tags have, and a mecha tag costs no scope at all.

The scopes moved with it — `gmail.modify` (stopping short of
`https://mail.google.com/`, whose only addition over it is irreversible
deletion) and `Mail.ReadWrite` — and both accounts re-consented the same day.
The expectation going in was that the university account would be blocked on IT and Google
would be easy; it was the exact reverse. The university's Entra registration
already had `Mail.ReadWrite` Delegated granted tenant-wide, while the Google
client turned out to be in Testing publishing status, which caps its refresh
tokens at seven days and needs a CASA security assessment to escape.

The same pass raised the learned-rules cap 15 → 25 with `RULES_CHAR_BUDGET`
1600 → 2600 beside it, and — because that made the cost of the existing leak
visible — stopped every domain riding in every run's prefix.
`rules_prompt_block_for(RUN_DOMAINS)` gives a run `behavior` + `writing` and
nothing else, new domains are opt-in, and `unrouted_domains` warns at startup
about any domain holding rules no run carries. Everything reconstructing "what
a run sees" moved with it, because the validation ledger is keyed to the rule
set measured. Two research docs landed alongside: `MAIL-UX-RESEARCH.md` and
`SKILLS-RESEARCH.md`. 904 tests.

**2026-08-18 (afternoon) — mail learns to be a queue, and the release waits
for it.** Phases 1-3 of `MAIL-UX-RESEARCH.md` landed: the triage store, the
quarantined classifier, `mecha mail`, the escalation rule and a nightly timer.
The classifier is the front door's construction one directory over — no tools,
no history, no system prompt, no shared cache prefix, one isolated call per
thread — and `Record::for_privileged_run` is a function with no argument that
returns the prose. `one_line` stays behind it, which is the judgement call:
short, exactly what a summary wants, and model prose derived from attacker
prose, which is the laundering path the front door withholds `reading` to
close. Verified on 51 real campus-account threads. The documents work merged the
same day from a parallel session, and Luke then held 0.1.7 until the mail
feature is complete through phase 6.

**2026-08-19 — the harness learns to watch itself.** A research pass on what
separates good harnesses (`docs/HARNESS-RESEARCH.md`) found the largest
measured lever is plumbing rather than cognition — Claw-SWE-Bench moves the
same backbone from 19.1% to 73.4% purely by making patches apply — and that
persistence inverts: models self-condition on their own errors, an effect that
does not go away with model size. Three fixes followed the same day.
`compact::collapse_repeated_failures` folds a pile of identical failures onto
its newest member, since eviction exempts errors by construction and thinning
only truncates long results, so eight failed attempts at one call rode in every
subsequent request forever. `RunOutcome::ended_on_failed_call` names the run
that stopped of its own accord with its last call failed — the silent-failure
shape no judge catches at better than chance — with an `expect` check beside
it. And doctor learned to see a trigger degrading quietly, then a trigger that
succeeded having done nothing at all, the second found by a sibling session
hitting the identical shape one layer down.

**2026-08-19 (afternoon) — the self-improvement loop, built and unstarted.**
A second pass (`docs/SELF-IMPROVEMENT-RESEARCH.md`, 733 lines) asked whether
the harness can notice its own problems and fix them; the answer was that
rumination's only sensor is a human stepping in, so a harness problem produces
no intervention and nothing downstream ever sees it. Six commits built the
missing half: `Record::Outcome(RunStats)` written by every front-end,
`runlog.rs` reading the corpus back with `mecha sessions health`,
`candidate.rs` as a pure gate (paired comparison, deterministic holdout, a work
guardrail that rejects a gain bought by attempting less), `RunStats` threaded
through the replay driver, `mecha eval --ab-config` as the content-sensitive
arm, three population checks in doctor, and `diagnose.rs` plus `mecha diagnose`
— the one place a model authors a change, safe there because being wrong costs
one measurement and unsafe at the accept gate for the same reason. Luke's
rulings moved the gate from provenance to proof: the diagnostician may search
the web and its output is not restricted to numbers, architecture changes are
proposable behind a human gate, everything else auto-accepts on measured
improvement. Nothing acts on the numbers yet, deliberately — the corpus was
empty at build time (178 sessions read, 0 outcomes), and the research's own
finding is that agents update their harnesses without benefiting, so the next
thing to learn is whether doctor's findings would actually have been acted on.

**2026-08-19 (midday to evening) — mail stops being a queue and becomes a
surface you work.** Recorded on 2026-08-25, six days late: this arc shipped
between 11:08 and 18:10 and neither document closed it out, so the plan
written that morning went on reading as a to-do list. Every phase the
`MAIL-UX-DESIGN.md` roadmap named landed in one run. **Phase 4′, the
pre-filter** (`3547cb1`): `List-Unsubscribe` plus an automated-sender rule
ahead of the classifier, disposing of a little under half of all threads
without a model call — and keeping three properties, that it only ever
produces `ignore`, that it reads the envelope and never the body (which is
where an injection lives), and that `List-Unsubscribe` alone is not enough,
because it finds marketing, which must offer an unsubscribe, and misses
institutional and transactional senders, which need not. `PrefilterRule` rides
on the verdict so the rule can be **graded rather than believed** — without it
a thread the pre-filter dropped and one the classifier called `ignore` are
indistinguishable afterwards, and "is this too aggressive" has no way to be
asked. **The corpus became a scorecard** (`e2af8a5`, `4cafaf1`): `mecha mail
eval` grades against a year of mail whose outcome is known, sampling both
strata rather than the corpus uniformly, because answered threads are rare and
a uniform run of 200 would hold a handful of the only threads carrying ground
truth. **Phase 6, corrections** (`3329c39`, `f6bac35`, `0166120`, `c440700`,
`40a1dc7`) — field-level, feeding a few-shot pool and a `triage`-domain
reflection — and `mecha mail score` (`5151f22`) for the live store, which is a
different question from the corpus and so a different verb. **Phase 4″**
(`05581fb`): a thread becomes a board task or parks with a reason. **Phase
4‴** (`0001ae6`, `a14c136`, `8231163`): day two as a store-side primitive, so
the morning briefing is one reader of it rather than the owner — and
"nothing new to surface" stopped being written as "nothing is waiting".
**Phase 5** (`3ef4e60`, `78d73d9`, `037d9c8`, `a34a1c2`): `/mail` as a modal,
ending with `r`, `f` and `e` no longer stubs — `f` being **forward**, which had
had no key bound to it at all, though the receipts-to-the-finance-person case
was one of the five that motivated the whole feature. An eleven-finding review
ran over it the same evening, and its first finding was the whole loop.

**2026-08-19 (night) — skills, and the queue stops asking people to read
JSON.** `mecha-core/src/skill.rs` and the Agent Skills format: a procedure the
*user* writes and the model loads when it decides one is relevant, at three
levels of disclosure — name and description in the prompt, the body on a
`skill` call, a bundled file only when the procedure points at one. The
absences are the safety argument and are structural: no install command, no
registry, no remote body, nothing derived from a session, no project-layer
store, and therefore no taint when one loads. It is also the pressure valve for
the per-domain rule cap, since a procedure too long for a rule and irrelevant
on most runs now has somewhere to live. Alongside it the review surfaces
stopped rendering arguments at people: a staged message shows as the letter it
is, `edit` opens the prose instead of a string literal, resolved drafts go
behind `h`, and `/mail` grew a pinned key strip and an `enter` that opens the
thread instead of fetching one and printing its subject into a title bar.

**2026-08-20 — documents, and four arcs in one night across three sessions.**
`mecha-docs` shipped as the fourth binary on `mecha-mail` (`google/docs.rs`,
`google/docs_server.rs`), and the scope *is* the design: `drive.file` is the
one non-sensitive scope in the family and covers only files the app created or
the user handed it through Google's own chooser, which is the path jail applied
to Drive — provable by reading a scope string rather than by reviewing every
future diff. Then, in one evening: **the task board reached the session** —
`mecha tasks` and `/tasks` onto the graph's GTD board over `kg_task_*`, with
the same status letters as `mecha-graph tui` screen 6 and a test saying so;
**`/skills`** as the slash counterpart to `/tools`, reading what a run carries
from the running agent rather than re-deriving it from config; **a reply became
reviewable with the message it replies to**, recovered from the transcript that
already held it rather than from a re-fetch, joined by identifying arguments
matched by key and value so nothing about mail is special-cased; and **eight
modals stopped taking the session down when the window shrank**. Released as
0.1.9 the same night, with the site's pages for the three arcs that had
documented nothing outside `CLAUDE.md`.


**2026-08-20 (later) — five surfaces that stated their size in one unit and
drew in another.** Reported as "the prompt gets wonky — the cursor is
misaligned, things are not clearing and we get collisions", and it was two
causes wearing one costume. The collisions were not a rendering bug at all:
`tracing` writes to stderr, and under `mecha tui` stderr *is* the alternate
screen, so a `warn!` mid-run painted its bytes through the frame and stayed
there forever — ratatui repaints by diffing its own buffer and therefore never
repaints cells it did not write. `mecha-cli/src/logs.rs` holds the lines while
a front-end owns the screen and the loop drains them into the transcript,
bounded, colour stripped, with the remainder flushed to the real stderr on the
way out. The caret was the second cause: `input_layout` counted characters and
broke at `width` while the `Paragraph` beside it word-wrapped and measured in
display cells, so two implementations of one question drifted apart on every
line of prose. `input_layout` now returns a partition of the text into row
ranges and `draw` renders those rows, with no `.wrap()` left to disagree.
Underneath three more fixes was one repeated mistake — `body.len()` is not the
drawn height when the paragraph wraps — which had `/help` truncating `/doctor`
and `/review` mid-sentence, `/tools` hiding its declared-capability block below
an unreachable fold, and `/tasks` pushing the task id off the one view that
exists to show it. `typed_char` closed a separate class in the same pass:
`KeyCode::Char('c')` with CONTROL held is Ctrl-C, and `/mail` had been feeding
it to `action_for`, so Ctrl-A archived the thread under the cursor and Ctrl-R
started a drafting run. Shipped alongside `/docs`, the ninth modal, which
exists because `mecha-docs pick` printed a URL and then blocked on stdin — the
browser leg split into `pick --url` and `pick --redirect` so it can span two
commands minutes apart, which is what lets a document enter `drive.file` scope
from a headless box with no tunnel and no forwarded port. `cfa2cc2`.

**2026-08-21 (later) — the model had eyes, and the file being served held
half of it.** A screenshot sent from Slack was answered with "I don't have the
ability to view image files directly", which reads as a limitation and was a
misconfiguration. Qwen3.6-35B-A3B is multimodal — `general.tags` says
`image-text-to-text`, `rope.dimension_sections` is `[11, 11, 10, 0]` (mRoPE,
which a text-only model does not have), and the chat template emits
`<|vision_start|>` — but the GGUF holds 750 `blk.*` tensors and no vision tower
at all. That ships separately as `mmproj-*.gguf`, and nothing had downloaded it.
Four of four multimodal models on the machine were being served text-only, one
of them with its projector already on disk, unused, since the day it arrived.

Both halves shipped. On the server, `scripts/mmproj.sh` is sourced by every
start script and *refuses to start* without a projector, printing the `curl`
that fetches it. In the harness, `Block::Image` is a fifth block variant —
user turns only, because Anthropic accepts an image inside a `tool_result` and
the OpenAI dialect's `role: "tool"` messages carry a string and nothing else,
so a tool returning pixels would work on one backend and silently lose them on
the other. Both backends degrade to a named line rather than failing and are
tested to word it *identically*, because a conversation crossing a `/model`
switch must not tell two stories about its own history. The parts array is
built only when an image is present: the cached prefix is a byte-prefix match,
so making it the uniform shape would invalidate every run that never sends one.

`mecha_core::image` caps at the door rather than per turn — the transcript is
append-only and every turn resends the whole history, so a resize is paid once
and collected forever after. Measured on the screenshot that started it:
2222x1548 / 5.7 MB → 1568x1092 / 179 KB, with llama-server reporting
`prompt_tokens` 294 **either way**, because it tiles to a fixed count
regardless. 32x off the wire and out of the session file for nothing in
context. An image already under the caps passes through byte for byte, since
re-encoding a crisp screenshot of text is a real loss and that is the case this
exists for.

Four doors reach it: the Slack connector, the remote-control inbox,
`mecha run --image`, and a file **dropped on the TUI prompt** — which turned
out to be half-built already, since a terminal converts a drop into a bracketed
paste of the path and the `Event::Paste` arm had always inserted it. What it
never did was look at what the path was. Now a paste whose every token resolves
to an existing image becomes a chip, `[image: shot.png]`, with the bytes held
beside the input; an image is sent only if its chip survives to submit, which
is the only undo there is for something backspace cannot reach. That
conjunction is the safety property rather than a convenience: a paste is also a
paragraph off a web page, and attaching any file whose path appeared *somewhere*
in pasted prose would let copied text pull bytes off the disk into a request.
It cannot work over SSH and never will — the path pasted is the laptop's and
the process resolving it is on the far box, which is precisely what the Slack
conduit is for.

`provider::preflight` is the thing that stops this recurring: one `GET /props`
at startup, compared against config in **both** directions, because they fail
differently — declared-but-not-served silently degrades every image to text,
while served-but-not-declared means a projector is loaded, paid for in memory,
and never used. The same request checks `context_window` against the per-slot
`n_ctx` (so the `-c` versus `-c / -np` rule is read rather than restated) and
the configured model against `model_alias`. It warns and never refuses — the
opposite of the sandbox's bargain, and for the opposite reason: there, falling
through means running unconfined; here a mismatch means compacting at the wrong
moment, and a preflight that can stop a working machine from booting is one
people turn off.

`mecha setup` generalises that into onboarding, on the finding that
documentation had already stated all of this correctly while the machine stayed
wrong for months. It does not ask what you meant; it reads `/props` and writes
down what the server reports, and `--write` edits the table in place rather
than reserialising, because a round trip through a TOML parser discards every
comment and in this project the comments are most of the file. Its planner is
pure and tested without a machine, on the `compact.rs` split: getting
onboarding wrong is silent. Two boundaries have tests naming them — the
knowledge graph is *named and never driven*, since spawning `mecha-graph
source` would be exactly the coupling the MCP-only rule prevents, and nothing
schedules anything, with the runner offered only once a trigger already exists.

The last finding came from a run that **worked**. The fixed Slack test answered
correctly and recorded `taint {private: false}`, where the same user action
before the arc recorded `{private: true}` — because the model used to have to
`fs_read` the attachment, and putting the pixels on the user turn removed the
tool call and the taint with it. A feature that loosens the interlock as a side
effect, with a correct-looking answer sitting on top of it. `Taint::
arm_for_content` now arms the private leg for an attached image, enforced in
the loop rather than in `Conversation::push` — `slack/connector.rs` appends to
`messages` directly, so arming at the tidy place would have left unarmed
precisely the surface people attach screenshots from. The argument for treating
an image differently from typed text is that **a screenshot is captured, not
composed**: you choose every word you type, and you choose the window rather
than everything in it.

**2026-08-21 (later still) — onboarding, audited by running it.** The
new-user path was walked against a genuinely fresh `MECHA_HOME` rather than
read, which is the only reason five defects turned up. `first-run.md` — the
page a beginner follows — taught `context_window = 32768  # the -c the server
was started with`, the exact mistake `features/serving.md` calls "the trap
worth knowing before you meet it": two pages describing one number, and the
beginner's page lost. `triggers.md` said `cp scripts/mecha-triggers.service`,
which `cargo package --list` shows ships **zero** files, so it could not be
followed by anyone installing the way the docs lead with. Nothing anywhere
mentioned vision, `mmproj` or images, so the day's expensive bug was
undiscoverable. `config init` wrote a local block with neither
`context_window` nor `vision`. And nothing at all said how to obtain
llama-server, a GGUF, or a projector — the README's whole local story was the
clause "or point at a local server".

`mecha setup` is the answer to the half of that which documentation had
already failed at: the docs stated these settings correctly and the machine
stayed wrong for months, because each degrades quietly instead of failing. So
it does not ask what you meant — it reads `/props` and writes down what the
server reports. `mecha trigger daemon --print-unit` is the answer to the
other half, emitting a unit that names the binary by `current_exe` rather than
the string `mecha`, because a unit resolving against systemd's `PATH` is the
version-skew trap one layer down. That flag is also this arc's neatest
process lesson: it was *documented before it existed*, and the only reason a
commit fixing a broken instruction did not ship the identical defect is that
the next step was to check whether the command was real.

One filed defect was withdrawn, and the withdrawal is the transferable part.
`website/docs/graph/*` is synced from the mecha-graph repository at build time
and gitignored here; that repo had replaced Ollama with llama-server the day
before. What had been read was a **stale local build artifact** — one
`sync-graph-docs.mjs` corrected it. **A gitignored synced file is a cache, not
a source:** grep it and you are auditing your own last build, not the truth.
Verified good in the same pass, so nobody re-audits it: `cargo install
mecha-graph-mcp` plus the `[[mcp]]` block from `features/memory.md` brings the
`kg_*` tools up unprefixed against a fresh store with no errors.

**2026-08-21 — the remote control.** A live TUI session and a named Slack
thread became the same conversation. `/remote-control <name>` claims a durable
name, opens or re-opens its thread in the owner's DM, and tees the run's
`AgentEvent`s into `slack::pump` — which needed no change at all, because it
had been written as a standalone consumer months earlier. What the run does and
what you type appear in both places; typing in the thread steers a run in
flight or starts a turn; files move both directions, out through `/send` and
`show_file` and in through a staging directory the connector owns. The design
is `docs/REMOTE-CONTROL-DESIGN.md`, written before any of it and settled the
same day.

The shape came from one constraint: a `Conversation` — messages *and* taint —
lives in the running process's memory and the session JSONL has one writer, so
there is no symmetric design to look for. One owner, many terminals. The
connector consequently stopped answering for a mirrored thread, which it had
been doing invisibly: a reply used to mint a thread record and start a fresh
conversation in a different workspace under a different permission mode,
answering in a scrollback it knew nothing about. Never a leak — that
conversation is clean, taint and all — but a stranger wearing the thread's
clothes, which reads worse and is just as wrong to act on.

Two code reviews found twenty-two defects across the arc, four of them real
bugs in code that already had passing tests, and **two that predated the arc
entirely**: a `/model`, `/provider` or `/mcp` switch had always rebuilt the
agent from config alone, silently dropping `ask_user` and `recall`, and had
always ignored the permission mode in force (see *Containment and state*).
Both are fixed; `install_frontend_tools` and `approver_for` exist so the two
call sites cannot disagree again. `d266fe8`, `d0a1499`.

**2026-08-21 (0.1.11) — four things that read as working and were not.** A
patch release with a single shape running through it. The TUI held mouse
capture for the whole session, which made the wheel scroll and made a drag
impossible, so `/docs`'s own documented fallback — "the URL stays on screen to
be selected by hand" — had never existed for a 420-character link whose only
other route out was an OSC 52 write no terminal acknowledges; any modal now
releases the mouse and `^s` toggles selection in the main view, reconciled once
a frame from the drawn state rather than restored at each pane's exit. Two more
blocked the same picker: a paste while it was up landed in the message box
behind the modal, and the link was drawn inside a bordered box, so a drag
collected a `│` at both ends of every row. In `search.rs`, a chain whose
backends all answered and all found nothing reported `every search backend
failed`, which one model read as broken infrastructure and answered by
rewording the query eight times — an exhausted chain of empties is now an
answer, while a chain where nothing *answered* is still an error; and a SearXNG
instance with every engine suspended or CAPTCHA'd returns `results: []` at HTTP
200, byte-identical to an empty web, until `unresponsive_engines` is read. The
cache lens scored re-payment from `input_tokens` — everything *not* read from
cache, which on this workload is overwhelmingly the turn's new content — so it
shouted loudest when tool results were biggest and called the real failure
stable; it now measures what did not come back, and the field says `repaid`.
`[[search]] prefer_deep` arrived alongside: depth had changed only *how* a
backend searched, never which one ran, so a paid backend bought for hard
questions was reached only when the free one came up empty. It reorders and
never filters. 1,244 tests.

**2026-08-21 (0.1.11, the second half) — a draft you could neither read nor
approve.** Folded into the same tag, because it was one arc. Eleven drafts had
piled up and the docs edits among them could not be released: pressing `s`
raised the approval confirmation and nothing after that did anything. The
confirmation put a tainted draft's arguments on screen "in full" and drew them
with an unscrolled `Paragraph`, which renders from the top — so a
`docs_replace` whose `find` was an entire syllabus section pushed the question
and the `y` prompt off the bottom of the box, and `modal.confirm.take()` meant
every other key dismissed the confirmation, so there was no way to scroll down
to the instruction that had gone missing. The box was sized from `body.len()`
besides, which counts *logical* lines where one long argument is a single
`Line` and many rendered rows, so the height reported "it fits" in exactly the
case where it did not. The arguments scroll now, the prompt is pinned to the
bottom border where nothing can push it off, the height is measured with
`Paragraph::line_count` *after* wrapping, and scroll keys no longer count as
"anything else". The sibling detail view had done all of this correctly for
months; the confirmation was simply never given the same treatment. Renamed in
the same pass: the verb is `approve` on `a`, beside `e` edit and `r` reject,
because the queue holds more than mail and a `docs_replace` is approved rather
than *sent*. `outbox send` and `s` both still work, and the stored status
stays `"sent"` — an append-only value that `mineable_as_writing` keys on, so
renaming it would orphan every resolved item and silently stop the writing
miner. 1,246 tests.

**2026-08-21 — the web search a scraper could not win.** Every *general*
engine behind the local SearXNG instance refused the box's IP at once — brave
and google cse `Suspended: too many requests`, duckduckgo and startpage
`CAPTCHA`, mojeek `access denied` — while its specialist engines answered
fine. Enabling more engines did not help and was not going to: a self-hosted
metasearch is in a standing race against anti-bot walls, and losing it looks
exactly like a quiet web. Exa was wired as the second backend and Tavily as a
third, both already implemented and needing only a key and three lines of
TOML. Ordering was measured rather than reasoned: a quick Exa search bills
$0.007 against Tavily's $0.008, read off Exa's own `costDollars`, and the
`contents` block mecha sends costs nothing despite a price list that bills
Contents separately. Merging the paid backends *into* SearXNG was considered
and refused — SearXNG fans out in parallel to every enabled engine while the
chain stops at the first answer, so every trivial lookup would have paid for
Exa too; it would have collapsed the chain to one link whose only backend
cannot report why it failed; and it would have kept the appearance of "the
query never leaves your network" while destroying the fact.

**2026-08-22 — the self-improvement loop closed, on the owner's ruling.** The
three gaps that had kept detect→diagnose→measure→gate a set of parts — no
nightly stage, no session-corpus measurement arm, no accept path — closed in
one arc (`69ac0e9`), after Luke ruled for §13.3 auto-accept in so many words:
verified improvements apply themselves, architecture reaches him, and the
loop must be useful without adding work. `mecha harness ruminate` runs
nightly from `ruminate.sh`: diagnose one change from the run corpus, persist
it as a candidate (`~/.mecha/learning/harness/candidates/`), measure it by
counterfactual replay of recent sessions — both arms replayed whole under the
recorded config, the candidate arm differing only by the change, divergent
episodes dropped rather than scored — and dispose through `candidate::judge`.
A confirmed config win lands in an override layer applied *between* config
defaults and every file layer, so the user's own config wins by assignment
order and revert is deleting a line; the closed override set moved to
`mecha_core::harness` with `eval --ab-config` parsing through it, so the two
arms cannot drift. The brief now carries prior candidates as
do-not-re-propose history, doctor flags one staged past 72h, and the first
live run the same evening had the diagnostician correctly decline on a
healthy corpus. Deployed the same day: binary installed, services restarted,
the nightly's `ExecStart` confirmed to run the repo script.

**2026-08-22 (later) — doctor sees the starved learner.** The morning's
investigation had found the *rule* learner reporting `ok` every night for
seventeen days while producing nothing: 14 of 16 reflections excluded by the
origin gate, the clean pool stuck at 2 below the learn floor of 3 — the
null-run bug one layer up from the trigger version doctor already catches.
The classification itself was verified precise (positional, per-intervention,
`timeline.covering(at)`), and the excluded sessions had genuinely read the
open web (`http_fetch`, `research`) before their interventions, so the gate
was doing exactly its job on exactly its content. What was missing was a
reader: `check_learning` (`52a0755`) fires when ten or more exclusions have
piled up, no domain reaches the floor, and new ones keep arriving — quiet on
young installs, met floors, and dormant loops — and its finding proposes a
decision rather than a command, because the only throughput levers (steer-
text-only reflections, a custodied-source origin subclass, a lower floor) all
cross the recorded fail-closed ruling and were surveyed and left untaken. The
floor became `learning::LEARN_MIN_REFLECTIONS`, shared by `learn --min` and
the check, on the `MAX_ACTIVE_RULES_PER_DOMAIN` lesson.

**2026-08-22 (evening) — the queue could not be read, and then could be.**
The knowledge graph's merge queue had grown 1,035 → 6,434 in nineteen days and
the nightly alert printed only its depth, which reads identically whether the
accept lane admits 5% or 95%. Reading across it turned up an ordinary
structural gap and one genuinely misleading number.

The gap: the queue is a ratchet. Auto-accept requires a `(proposer, predicate)`
class on a hand-maintained allowlist or an earned ladder rung, and 91.2% of the
queue can reach neither — the ladder has `Staged → Sampled → Trusted` and *no
rung below Staged*, so a class can earn its way into autonomy and never earn
its way out of the queue. Vet's ceiling (40 × 10 predicates) also sits below
extraction's output (500–970 candidates a night), so the drain was specified
slower than the fill, against a slice that is 8.8% of the problem.

The number: `precheck::review_clusters` counted the pipeline's own dedup and
ephemeral rejections as the owner's, in the one view a person reads
*immediately before verdicting a whole class*. `llm/has` displayed 18% against
a true 67% over 48 human verdicts; `llm/has_role` 7% against 53%; three classes
displayed 0% on which nobody had ever voted. `ladder::human_record` had carried
the correct filter (`reject_reason NOT LIKE 'precheck:%'`) all along — two
queries, one filtered, one not, and the unfiltered one was the one on screen.
A first pass of the analysis reached the confident conclusion that half the
queue was demonstrably unwanted and a Wilson-upper-bound suppression rule would
clear 31.3% of it; recomputed on human verdicts alone the same rule clears
**nothing**, because there is no class the owner has judged often enough and
rejected consistently enough to condemn. The queue is not full of junk; it is
full of unknowns — 40.5% of it in 660 classes with no human verdict at all.

What shipped from it: `review --proposers` and the TUI's `p` roll the queue up
by proposing mechanism (the level a decision is actually made at — the LLM
extractor measures 59% over 1,984 verdicts, `linker:knn` 16% over 57);
`review --sample` draws uniformly at random, seeded and printed, because the
queue has an order and judging its head measures the ordering; and on mecha's
side `mecha review` plus the `/queues` modal put all five human-facing stores
in one list. The graph half of that modal shells out to `mecha-graph`, which is
a departure from "reach the graph only through MCP" taken deliberately: the
tool surface has no `kg_accept` and must not gain one, since every MCP tool
lands in the model's registry and a model that can accept candidates can accept
the ones its own extractor proposed.

Six review findings were fixed before the merge, one of them severe: the
class-level verdict passed `--proposer`/`--predicate` to a `mecha review
accept` that declared neither, so the headline "one decision worth hundreds"
could never have worked. It shipped that way because the mutating path was
never exercised — the live graph was deliberately left alone and no scratch
fork was made instead. `mecha-graph fork` turned out to be broken anyway
(768-vs-1024 vector dimensions, from the harrier embedding switch), which is
its own finding and still open.

**2026-08-23 — the queue became workable, and the learner finally learned.**
The morning's question was "how did nightly go", and the answer was: cleanly,
and with three systems quietly not working. What followed was two sessions in
one checkout (coordinated by cross-session messages; the morning one wired the
item-review `b`/`A` keys and taught a stale session to spawn children through
its own `/proc/self/exe`) and, by evening, all three fixed and a set of
surfaces that did not exist at breakfast.

The learner: nineteen nights of correct provenance exclusions had produced
zero rules ever — structural, not a bug, since any session touching mail,
docs or the web is untrusted and those are exactly the sessions where
corrections happen. The recorded fail-closed ruling was kept and the evidence
moved to its trusted side instead: `learning::evidence_for` hands the
reflector the user's own typed words plus registry-owned tool names, with
every assistant-authored byte withheld — the front door's split, except the
withheld half is never read at all — so the reflection's inputs are clean by
construction and it classifies learnable (`Evidence::UserTurns` records what
was shown). `mecha reflect --remine-untrusted` re-mined the archive: 11
lessons recovered, `learn` staged the store's **first-ever proposal** (5
rules from 10 reflections), and doctor's starved-learner finding cleared the
same afternoon.

The ladder: promotion fired only inside `note_verdict` — at the instant a
human filed a verdict, and at no other moment — so classes whose strong
records predate the 2026-08-16 Wilson switch sat at `staged` indefinitely,
invisible because no CLI printed a rung. `mecha-graph ladder` shows every
class (union of ledger rows and verdict history, since pre-ladder classes
have no row), and `ladder --promote` applies a one-rung-per-pass recompute —
never demoting, exactly `note_verdict`'s arithmetic minus the verdict. Run
live it promoted `works_on` to trusted and `member_of`/`located_in` to
sampled, and with the day's review work the queue fell 7,296 → 6,569.

Under it, provenance the cascade made unaffordable to skip: an accepted
candidate row carried no record of who accepted it, so machine accepts —
the durable lane's, a cascade's — counted toward the human rate that
promotes classes, the accept-side twin of the 2026-08-22 cluster-view
contamination, running in the direction that widens autonomy. V017 adds
`fact_candidate.reviewed_by` ('user' / 'auto' / 'cascade:<seed>'), and
`HUMAN_VERDICT_SQL` is the one predicate — ladder promotion and the
cluster/proposer views share it, legacy NULLs keep counting as they did
(rewriting history would gut the record the ladder runs on), and every row
since is exact.

On top of that, the queue's repetition became reviewable as repetition:
`similar.rs` groups a class's pending candidates by semantic similarity
(deterministic leader clustering at precheck's own flag threshold), the
listing covers the whole class — groups largest-first, singletons after,
since a view that hides most of the work is a view people leave — and one
keystroke verdicts a whole group as **one human verdict**: the seed is the
owner's, the members cascade machine-labeled, and the fan-out lands on the
explicit ids that were on screen (`--cascade`, vetted per-id against the
seed's class), never a re-derivation of a queue that may have moved. `b`
binds a group's unresolvable subject, `A` accepts creating it (the cascade
then resolves against the node the seed just made), and `[`/`]` re-group at
a threshold stepped from the value the child reports it ran at. The first
live grouping justified the feature by itself: bee's largest group was
fifteen hallucinated family members.

The TUI stopped freezing while children work: `/mail`'s archive/spam/task
ran an MCP server plus a network call inline per keystroke (the code's own
comment admitted it), and grouping embeds a whole class — both now spawn on
threads and land through watches. The graph reached the person at the
keyboard: `mecha kg search|entity|note` over the same `kg_*` MCP surface the
model uses, `/find` as the search modal (entities open their full record,
facts and episodes open in place), `/note` as deterministic capture with
entities linked on landing. Slack gained `note` and `queues` as owner-gated
command words on the doctor pattern — a capture matched before the text can
become a prompt, because a capture that depends on a model's mood is not a
capture, and a read-only backlog rollup whose verdict buttons are
deliberately a future design pass. The morning trigger's daily
subagent-skip warning went quiet under a trigger's own durable allowlist,
on the outbox warning's own reasoning, and one bug was shipped and fixed
within the hour — the NULL predicate, recorded under Traps.

**2026-08-24 — the phone became a terminal, and mecha learned to talk.** Two
arcs, three sessions coordinating over the inter-session mailbox, one
production system by the end of it. The **voice arc** went from
`docs/VOICE-RESEARCH.md` to a running stack in a day: three speech services
(llama-served STT, Chatterbox TTS with Kokoro beside it, the Pipecat worker),
the loopback OpenAI facade over the shared agent (`mecha-cli/src/voice/`),
D5 ratified (owner speech is typed text, no taint), and the whole thing
systemd-managed and reboot-proof — its build log is `VOICE-RESEARCH.md` §7.
The **remote-surface arc** asked whether Slack was still the right remote
control and answered by building the alternative: `REMOTE-SURFACE-RESEARCH.md`
(the field's convergence on local-loop/remote-view/approval-as-the-verb;
Telegram as the only credible Slack successor; the tailnet web app as the
recommendation), `REMOTE-SURFACE-DESIGN.md` (one process, identity from the
network, verbs as CLI children), brand-held phone mockups, and then `mecha
serve` itself: a `[web]` config section stripped from project layers like
`[slack]`, an owner-login guard on every request (the header `tailscale
serve` injects), a read-only dashboard over `review queues --json` and
`doctor --json`, streaming chat over SSE with steering and cancel on the
Slack connector's one-agent/many-conversations pattern, the outbox as a
phone review surface rendering the whole reviewable object (`DraftView`,
source reads behind an amber gutter, taint sheet with the exact bytes,
`outbox edit --body-file` so the no-terminal edit shares the one
implementation), the graph queue's sample deck (seed printed, verdicts
one at a time), tasks and notes over `tasks`/`kg` verbs, and a keyed
session rail. The two arcs then **unified**: the voice facade mounted
inside `mecha serve` (one agent, one cached prefix, two dialects), the
page's WebRTC offer proxied same-origin behind the owner guard, voice-core
embedded in the chat page as an in-call overlay with a live transcript
thread and a cloned-track mic meter, and production flipped to one
systemd-managed process serving web and voice both. The day's last build
put a **present human** on the page: per-session `ask` mode turning tool
calls into live approval cards (allow, or deny-with-reason mined as a real
correction; timeout is `Blocked`, machine policy), `ask_user` routed to the
session that owns the calling run's jail through a new `Asker::ask_in`
context seam, cancel draining parked cards, and pending cards riding the
transcript read so a locked phone reloads into its questions instead of a
silently parked run. The voice arc's closing act was structural: **Voxtral
answered speech instead of transcribing it** — question-shaped audio came
back as first-person answers, spoken instructions were obeyed — so Parakeet
took the STT seat, probes exact at 92ms, and the injection probe now
transcribes as faithful words obeyed by nobody. An orphaned Slack
tasks/board WIP from an earlier session was adopted, reviewed, and landed
the same night. The `update` skill gained the web assets as surface 1b.

**2026-08-25 (afternoon) — three things the phone could not do, reported
from the phone.** All three were found by using it, and the pattern under
two of them was the same: **a surface reporting success it had not
verified.**

*Notes could not be opened or edited.* The page listed them and stopped
there, and the reason was a missing key rather than a missing button:
`kg_notes` returned each episode's `uid`, which names the row and cannot
write to it — the graph's episode key is `(source, source_id)`, and
re-upserting under that is an update. Without it the best any surface could
do was capture a second note beside the first. The graph now returns
`source_id` too, `mecha kg note --edit <source_id>` is the verb, and the page
drives it. The trap was the *other* field: `upsert_episode` writes every
field it is handed and `occurred_at` defaults to **now**, so an edit that did
not carry it would move the note to today — a notebook rewriting when things
happened because somebody fixed a typo. It is read back from the same
listing the id came from, so no surface can get it wrong on its own.

*A graph verdict could fail with nowhere to go.* Accepting a similarity group
failed with `cannot resolve subject 'X'` and the page printed it at the top
of the screen with nothing to do about it — while both ways through had
existed in the TUI since the modal was written (`b` binds the subject, `A`
accepts it as a new topic). They are on the card that failed now, offered
only after a failure, because `--create-subjects` invents a topic node.
Underneath was the invisible half: **`mecha-graph accept <id>` reports a
per-candidate failure on stdout and exits zero.** Right for a bulk run where
one of five hundred cannot resolve; a lie for a page that keyed on the exit
code, dropped the card it had just sent, and counted it as one of the twelve
verdicts the sitting claims to describe — while the candidate sat pending.
The child's report is tallied through `review::tally_report` now, the same
function the TUI reads.

*Calls ended by themselves*, for two independent reasons and neither was
network flakiness in the way it looked. The common one is a default nobody
chose: **pipecat cancels an idle pipeline — and the runner with it — after
300 seconds**, where idle means neither side produced a speaking frame. Five
minutes of silence is a conversational pause on a phone, and the log names
it outright (`11:36:23 connected` → `11:41:50 Idle timeout detected`). The
timeout is kept, because an abandoned tab otherwise holds VAD, turn
detection, STT and TTS open on a box with one GPU, but it is raised past any
pause that is still a conversation and it now *announces itself* over the
data channel. The rarer one was ours: `voice-core.js` treated ICE
`disconnected` as loss, when it is the browser reporting that packets have
stopped arriving *for now*. Only `failed` and `closed` are terminal now.
Deliberately no ICE restart to shorten the grace window — pipecat's
`restart_pc` fires the very `disconnected` event this worker cancels the
pipeline on, so reconnecting would destroy the bot being reconnected to.

**2026-08-25 (evening) — a draft you can say yes to, including out loud.**
`ReviewMode::Now` — *a draft you just asked for is a draft you are about to
read* — had been the default in the TUI and Slack since the policy was
written, and `mecha serve` was the one surface with no release policy at all:
every staged draft went silently to the outbox and the badge. On the page it
is now a card, built from `/api/outbox/{id}` rather than from the event that
announced it — ids on the wire, bytes from the store, because a reviewer
reading one thing while approving another is the failure the outbox exists to
prevent.

In a call the same offer has to be spoken and answered aloud, and that is a
different problem, because the answer arrives as text in the model's own
medium. **The harness asks and the harness hears**: the offer is composed
from the store through `DraftView::spoken`, the reply is matched by
`review_policy::parse_answer` before the request reaches a model, and the
release decision never enters a context window at any point. It is
`mecha review`'s oldest rule in new clothes — the graph's tool surface has no
`kg_accept` because a model that can accept candidates can accept the ones
its own extractor proposed, and a model that could release drafts could
release the ones an injection wrote.

Four rules carry it. The match is **whole-utterance, never substring**, and
the failure direction is the argument: "yes" is an answer, "yes but change
the time first" is not, and reaches the model as ordinary words with nothing
released — verified live before it landed. An unanswered offer is *dropped*
rather than held, or every later "yes" in the call lands on a forgotten
draft. The draft is **uttered whole or not offered**, because a listener
cannot skim back over the line where the extra recipient was, so a spoken
paraphrase is not a smaller review but a different document missing exactly
the field an injection would add; under 400 characters it is read out entire,
over it the choice of hearing it is the owner's, and a publish is never
offered by ear at all because its reviewable object is a rendered page.
Taint does not block — it is *spoken*, since the listener is the one person
who cannot re-read the addressing line. And **nothing spoken can discard a
draft**: rejecting takes a reason the learning miner reads, so "no" parks it
in the outbox where it already was, and the safe answer to every ambiguity is
the same one.

Two bugs the tests caught before a person would have. A datetime with no
offset read out as `2026-08-28T16:00:00`, so timestamps are spoken as dates
and times **in the offset the string itself carries** — converting one would
be the wrong-bytes review arriving through the ear. And a reply promised
"say next to hear it", a word `parse_answer` does not know: every listener
who said it would have been answered by the model while the draft sat there.
No surface may offer a verb the policy cannot recognise, so the follow-on is
a whole question. Proven end to end on a real calendar event, which was
created by a spoken "yes" and then deleted.

**2026-08-26 — the board learned to delegate.** Phases 1–4a of
`docs/TASK-AGENT-DESIGN.md`, plus the graph change underneath them. `mecha
tasks work <id>` turns a board item into a seeded run in its own session —
outbox-bound, its status moved by the harness, and unable to close its own
task because `kg_task_update` leaves the model's surface and stays with the
harness (`setup::withhold_tool`, which hands the tool back so one Arc serves
D5 and D6 at once). A run that needs a decision **ends** rather than waiting:
`mecha_core::questions` is the outbox's inbound twin, the run cancels its own
token, and `mecha questions answer` resumes the session with the owner's
words as the next turn — so nobody holds one of four llama-server slots
overnight for an answer that comes at breakfast. `/queues` gained a sixth row
and doctor watches it at 24h, shorter than the outbox's 48h because a pending
draft is finished work sitting safely while an unanswered question is a
delegation frozen mid-flight.

The graph side made the board able to say *who* has a task: `waiting_on` and
`session` on `kg_task_update`, an `agent` node kind — deliberately not a
person, because delegation is not assignment and a person node would answer
"who is responsible" with the wrong kind of thing — and `@owner`, so a
harness never carries the owner's name. D9 was **reversed while building
it**: the design proposed a `worked_on` fact, and an episode turned out to
exist only for the runs the distiller judged worth remembering, which are
precisely not the runs a person needs help finding. The session id is a task
attribute instead, and is strictly the more general of the two — the
episode's idempotence key *is* the session id.

A second review at high effort over phases 3–4 found ten more, and the first
would have shipped a feature that works once: **the D11 guard keyed on
`status == "waiting"`, which every finished run leaves behind**, so the
second *ask mecha* on any task bailed — detached from the web it exited 1
into `/dev/null` while the page had already said "handed to mecha". The
guard's own comment named the gap ("without `waiting_on` … this cannot tell
the agent from a person") and phase 3 had closed it hours earlier. Also
fixed: the resume door moved the board to the agent and never moved it back
on failure, and wrote no run marker while rendering as running — so *stop*
found nothing and the card kept pulsing; "nothing to stop" was returned to
the page as success, defeating the exact confusion its own comment claimed
to prevent; a stale `.cancel` could stop the *next* run two seconds in; and
every task run shared one `TodoTool` key, so the card showed another task's
plan the moment it began rendering one.

A second lane shipped **task provenance** the same afternoon: a task now
carries a pointer at what asked for it, so a person deciding "done or do
next" can read the original instead of searching their own inbox for a
subject line they half-remember. `captured_from` on the task node
(`gtd.rs:273`, `CAPTURE_KINDS` at `:234`), a **closed kind set of
mail | frontdoor | session** — deliberately not slack, because nothing on
this side can render a Slack thread and a kind with no reader is a button
that opens nothing — and unknown keys refused, which is what structurally
keeps it a pointer rather than a copy of an email body. `mail task` writes
it at capture; `mecha tasks source <id>` (`tasks.rs:401`) follows it with
one reader per kind; `POST /api/tasks/source` and a `read the …` control on
the card; the TUI's `o` deliberately off the key strip, because it is inert
on any task somebody typed and a legend advertising a dead key is the
dead-affordance problem arriving through the legend.

The two arcs interleaved in `commands/tasks.rs` and `Tasks.svelte`, which
is its own entry under Traps.

On the web surface: *ask mecha*, *stop*, *open the conversation*, an agent
chip derived from the board rather than self-reported, `task:` sessions in
the chat drawer, and the plan rendered in both places you watch a run —
reading the live tool in chat and the transcript on the card, because a
`tasks work` run is a separate process and its list is not in `serve`'s
memory. `TodoTool` was keyed per run and taught to rehydrate from a
transcript on the way (D14/D15), which is what made the card possible at all.
Two code reviews at high effort ran against it; the first found ten things,
including delegation as a way around D6.

**2026-08-26 (second pass) — the delegation loop closed at the phone end.**
Phase 4 could *start* a delegation and could not finish one: *ask mecha* had
been on the task row since that morning, a run that needed a decision ended
and stored its question (D13), and the only surface that could answer was a
terminal. So the gesture the phone exists for opened a loop the phone could
not close — and the board sat in `waiting` with nobody able to see why from
the device it was read on. `GET /api/questions` is a direct `QuestionStore`
read on `review.rs`'s pattern (mecha's own store, unlike the board, which
must go through the CLI because its store belongs to another repository);
answering spawns `mecha questions answer --unattended` **detached**, because
answering *is* a whole agent run; abandoning is synchronous, because writing
one record is instant. The card lands on its task rather than on a new page,
which is D13's own argument — the Waiting view *becomes* the queue of blocked
delegations, with no new noun — and a question whose task is off the board
still gets a card, because a question nothing renders is a delegation frozen
forever.

Two things that were not on anyone's list came out of building it, and both
were the same shape: a rule this project states everywhere, quietly untrue in
one place. **`--unattended` on the resume is load-bearing, not ergonomic** —
`answer_and_resume` built an *interactive* agent, so a detached child would
have installed `TerminalApprover`, read `/dev/null`, taken EOF as a refusal
and filed every one as `Decision::Deny("the user declined this call")`, which
the loop renders `"Denied by the user: "`: the exact string the learning
miner reads a **correction** out of. A question answered from a phone would
have taught mecha rules from a person who was never asked. And **no task run
had ever written a `RunStats`**: `record_outcome` had ten call sites and
neither `tasks work` nor `questions answer` was among them, so the corpus
CLAUDE.md describes as written "once per finished run by every front-end" had
never seen a delegation.

That second one is what made D16 buildable honestly rather than by guessing.
The card's state is derived from three sources and none of them is the run's
account of itself: the board says who holds the ball, the question store says
whether it is blocked, the transcript's outcome record says how the last run
stopped. Seven states, no two rendering alike, and the seventh is the
interesting one — `outcome unknown`, for a transcript with no outcome record,
which is a run that never got as far as saying how it went (a crash, a kill,
or a session written before the record existed). Folding that into `failed`
would have made every delegation from before this shipped shout that it
broke; folding it into `ready` would have made one that really died read as
finished, which is the exact rule D16 states outright. `Interrupted` reads as
ready and never as failed, on doctor's own rule for the same field: a person
stopping a run is the system working.

One unrelated bug fell out of looking: **`/api/tasks` never passed
`--closed`**, so the drawer's `done` view — which filters on
`done | dropped` — had been filtering a list that could contain neither since
the day it shipped. It read as "you have finished nothing", which is the
failure mode a filter that cannot match always takes.

Verified by driving the real page in a headless browser rather than by
reading it: the answer button disabled while empty and enabled on the first
keystroke, an option tap posting the option's own words, abandon posting the
id, and *open the conversation* landing on `#chat/<session>`.

**2026-08-26 (third pass) — the plan gate was decided against, and the seed
got the cheap half.** D12 proposed stopping a delegated run after its first
`todo` write to take the owner's edits. The owner's question — *"how is this
different from todos? should a human ever edit or delete or add a todo? we
already have an ask question system"* — is the defect: the gate made the
**todo list** the human-editable object, and every system that has solved
this keeps the reviewable plan and the agent's execution ledger apart
(superpowers writes a plan document and then *"create todos for the plan
items"*; Claude Code's plan mode writes `~/.claude/plans/` while `TodoWrite`
is separate). `todo.rs` had forbidden the collapse in its own module doc
since it was written — *"a list set by anything other than the model's own
`todo` write … is a second author of state the tool is supposed to own"* —
and nobody had noticed that this was the conflict, because D12 answers a
*different* `todo.rs` objection (the stale plan *phase*) convincingly enough
to look like it had answered them all.

Two things then decided it, both already in the repository. **The project's
own research argues against plan-first on this hardware**:
`VERIFICATION-RESEARCH.md` has FORGE 2026 finding straight-shot often equals
or beats Plan-and-Execute, small models *collapsing* under it (Llama 3.2 3B,
0.23 → 0.05), and a bad plan measuring worse than no plan. And **the gate's
trigger rested on a behaviour measured absent**: the 2026-08-04 probe found
this model called `todo` zero times in 20 eval case-runs from prompting, and
keeps a list reliably only when the *user turn* asks — so the gate would have
fired when the model felt like letting it. D12's own cited evidence turned
out to point elsewhere too: Copilot's 38.1% → 69% came *"purely on tuning
`copilot-instructions.md`"*, which is the **seed**, not a checkpoint.

So `work_prompt` now front-loads: work out what you need and ask it first, in
one `ask_user` call covering everything, with an explicit guard against the
opposite failure — *"do not ask what you can find out"* — because a prompt
that only says "ask first" produces a run that asks instead of working. On
the user turn, deliberately, since that is the one delivery channel the probe
found this model obeys, and the tool's own "in one sentence" schema is
*overridden here* rather than widened for everyone.

One gap the change made likelier and therefore worth closing with it:
**several `ask_user` calls in one turn all park**, and both surfaces rendered
only the first — `parked().first()` in the CLI, `find` on the card — so the
rest were reachable only from a verb nobody runs after being told what to do
next. Both now render every one, and both say the surprising part out loud:
answering *any* of them resumes the run, and the others stay open.

What none of this reaches is the failure D12 was actually after —
misalignment the model does not notice, where a confidently wrong plan asks
nothing. That is now **countable** rather than arguable, because task runs
record `RunStats` as of this morning: delegations that ended `ready for
review` and were then dropped or reworked rather than marked done. The
reviewable-document version gets built when that number argues for it, and
gets built as a document separate from the todos.

**2026-08-26 (fourth pass) — the graph queue was asking for verdicts it
could not show, on candidates it could not accept.** Four bugs, found by the
owner reviewing on a phone, and they compound in that order.

The card said **`undefined — undefined — undefined`**. `Queue.svelte` spelled
its fallback ``payload.statement ?? `${subject} — ${predicate} — ${object}` ``
— and a template literal is never nullish, so `??` stops at it. A commitment
payload carries `{who, what, when, direction}` and no s/p/o at all, so all
**695** `llm:commitment` cards asked for a verdict on a belief nobody could
read. The correct chain existed twice in Rust with a test named on the
commitment case (`tui/queues.rs`, `items_from_json`); the page was the third
reader and re-derived it wrong. One `faceOf()` now, matched to the Rust rather
than improved on.

Underneath it, **`linker:knn` was writing node ids into `subject` and
`object`, which are names.** `ProposedFact.subject` is what `accept_candidate`
hands to `resolve_entity` — canonical name, alias, identifier, fuzzy — with no
tier that reads `nodes.id`, and it is also the `kg_upsert` wire format. The
linker looked both names up for its statement and then stored the ids
(`linkers.rs`), so **every candidate it staged was unacceptable**, `bind` could
never suggest anything (`suggest_entities` matches names; a uuid is not a
misspelling), and `accept --create-subjects` — one of the two ways the card
offered through the failure — minted topic nodes whose *display name* was
another node's id.

That last part is what made it worse than a stuck queue. **Thirty** such
placeholders were in the live graph, and once one exists the id *resolves* —
to the placeholder — so the next candidate carrying it accepts cleanly and
asserts a belief about a node standing for nothing. Candidate #16644 no longer
failed; it answered `already resolves — nothing to bind`. A queue item that
fails loudly is a bug; one that succeeds into a fiction is the shape this cost
an afternoon to see. `repair-id-payloads` merged the 30 placeholders into the
nodes they were named after, rewrote **121 of 8,988** pending payloads to
names, and re-pointed **23 accepted facts** at real entities; idempotent on a
second run. Applied to the live store the same day.

**The bind target existed on the server and on no surface.** `BindBody.to` had
always been there and `mecha review bind --to` too, but neither the web nor the
TUI could send one — so the card displayed the graph's own instruction, *name a
target with `--to`*, and could not carry it out. The phone gets a field after a
failed bind (never after a failed accept, where naming a target is not the
answer); the TUI gets `B`, and a failed `b` opens the same prompt. The prompt
owns the keyboard while it is up, which it has to: `a`, `r` and `d` are verdict
keys, so a target named "Dana" typed into a live list would have filed three.

Last, **`--ids` was bounded by `--top`, which defaults to 10** — so a named id
set was silently trimmed. Found by the other session's review of a smaller bug
in the same code (a progress counter reading the *requested* count rather than
the returned rows) and worse than what it was reviewing: the TUI's group dive
has shown at most ten members of any group since that level shipped. Enter on
a group of seventeen, get ten, nothing says so. `mecha review items` now asks
for exactly as many as it names.

What made all of this reviewable in the first place is the fifth change:
**a group can be opened and its members verdicted one at a time** on the web
(`GET /api/queue/items`, "Review each of the N"), which the TUI has had and
the phone had not. The case for it is one real group — seventeen near-repeats
naming the owner's children, mostly Bee mishearing two names. Similarity is
the grouping key, not agreement, so "Accept all 17" would have asserted
every one of them. A verdict
inside a group is deliberately plain — no cascade — because telling the
members apart is the reason for being in there.

**2026-08-26 (evening) — B1 and B2, after checking whether they still
described the row.** Both board decisions predated phase 4, and re-reading
them against the shipped implementation changed both; the originals stay in
`TASK-AGENT-DESIGN.md` with amendments beside them, the way D12 does.

**B1 was right about its evidence and wrong about its translation.** §1.3's
invariant is *"complete and schedule are one gesture; move, **delegate** and
edit open a sheet"* — delegation is on the sheet list explicitly, so *ask
mecha* belongs behind the tap and a later reading that promotes it because
it is this project's differentiator is re-deciding a settled question on
taste. What did not survive: §1.3 surveys **swipe** actions on a *collapsed*
row, and this row has no actions until it is tapped open. The six chips the
owner called *"a little bizarre"* were never in anyone's way, and the
expanded card already **is** the sheet — a `…` inside it nests a sheet in a
sheet. Their problem was shape, not depth. `✓` moved to the collapsed row
(`Tasks.svelte:722`), because in Things complete is *"a tap on the circle
only"* and this board had no circle, so its most frequent action cost an
expand; what remains is grouped under two labels (`:550`).

That amendment made a long-standing HTML bug obvious rather than fiddly.
The card was a `<button>` wrapping everything, so every control in the strip
was interactive content nested in one — invalid, browsers disagree,
assistive tech cannot traverse it — and *three separate comments in the file
existed to explain choosing a `<button>` over an `<a>` to make the nesting
less bad*. "The expanded card is the sheet" means its contents are siblings
of the header, not children: the card became a `<div>` with a `cardhead`
button (`:739`). Six compiler warnings, five of them predating the branch,
went to zero. **A design correction can retire a workaround that had been
mistaken for a constraint.**

**B2's worked example turned out to be unrepresentable.** It promised *"call
Bob tomorrow at 3"* would set a date; `gtd::parse_due` accepts exactly
`today | tomorrow | +Nd | YYYY-MM-DD` and `due_at` is written `%Y-%m-%d`.
**The board has no time of day.** So the chip reads `tomorrow` and *"at 3"*
stays in the name — consistent with the Things side of the disagreement B2
had already picked. What B2 never said is the actual win: **capture
collapsed from three fields to one**, which is §2.1's own invariant
(organization is deferred at capture time) that the sheet had been quietly
breaking. And it is worth more than when written, because dictation landed
in between: a spoken capture arrives as one string with no second field to
fill, so without the parse the microphone could only ever produce an undated
inbox item.

`capture::find_when` (`mecha-core/src/capture.rs:61`) **detects; it does not
resolve** — the span goes to the graph's `parse_due` as `--due`, so one date
parser lives in the repo that owns what `+3d` means. Weekdays are
deliberately undetected: `parse_due` cannot take one, so honouring a
"friday" would mean resolving dates locally and detecting it without
honouring it would draw a chip that lies. Served in-process by
`POST /api/tasks/parse` (`serve/board.rs:409`) — the one handler on that
page that spawns nothing, because a child process per keystroke would pay a
fork and an MCP startup to answer a question with no state in it.

**2026-08-25 (night) — real people out of a public repository.** The
2026-08-07 history rewrite stripped *operational inventory* and did not touch
a second kind that kept accumulating afterwards: real people used as
convenient fixture data. Sixty-six replacements across fifteen files — two of
the owner's children by name, a spouse in three statements about their
marriage, four colleagues (two with working addresses), an old personal
address, a real Slack id, and the tailnet hostname of the machine all of it
runs on. Sites were three TUI modules, four core modules, twelve
`results/*.json` benchmark artifacts, and — worst — `website/docs/features/
queues.md`, which is *published*, and demonstrated similarity grouping using
the marriage statements. The feature's own screenshot was the private data it
exists to help review.

None of it was load-bearing: a fixture asserts a shape, and a shape does not
care whose name is in it. They were there because they were the nearest real
data to hand while building against a live personal graph, which is exactly
the pressure that will produce the next one. The tailnet host got a different
fix, because `--allowed-origins` is load-bearing security config: the tracked
unit ships a placeholder and the real values moved to gitignored
`OPERATIONS.md`, and an unedited copy fails in the safe direction — no
matching origin means the worker refuses every offer, where the wrong way to
be wrong is a value that parses as permissive. **Forward-only, by the owner's
ruling**: the names remain in git history and in published crates.io
tarballs, which a force-push over a public repo would not reach in forks and
clones anyway, and published crates can only be yanked rather than edited.
The only actively served copy was the docs site, redeployed and verified.

**2026-08-26 (fifth pass) — a goal system, and the first signal that says
something went well.** `docs/GOAL-SYSTEM-DESIGN.md` and rungs 0–3 of its
phasing, in five PRs (#61–#65). The finding behind the whole arc is one a
`grep` makes: **every evaluative signal mecha had was a cost or a
correction.** `learning::Trigger` is four ways of saying a person stepped in;
every `candidate::Metric` is phrased so lower is better, deliberately, so a
comparison cannot invert silently. A run could therefore be recorded as having
gone badly and never as having gone well, and nothing could start a learning
loop unless the world acted first — `grep -i goal` over `mecha-core` returned
four hits before this, all incidental prose.

Rung 0 was a consolidation the arc created the need for. **Nine call sites
established the same safety property by hand** — no tools, no conversation,
nothing an injected instruction can reach — each spelling out `tools:
Vec::new()` beside a one-element `messages` vector, two of them visibly copies
of a third. `quarantine.rs` puts the property in the type: no field for tools,
no way to add a second message, and `ask()` as the only exit. The ninth site
was found by review, in `mecha-cli`, after a sweep that had grepped
`mecha-core` and stopped — which is the argument for the module restated at
its author's expense, and the doc says so. `Agent::final_answer` was
deliberately *not* migrated: it is tool-less for an unrelated reason and sends
the whole conversation, so **the distinction is history, not tools**. The same
PR replaced `ProbePrep`'s `steer: bool` with `ProbeKind`, because grading read
*not a steer* as *a denial* by assumption — undoing, one struct field later,
the care `locate_denial` takes explicitly.

Rung 1 is `GoalRef`, and building it corrected the design twice. The goal
belongs to the **plan**, not to each `TodoItem`: D14 keys `TodoTool` by the
run's workspace and `b877e41` gave each task run its own, which together give
one list, one task. Both are cited because half the footing was hours old.
The first draft cited D11 — a one-writer rule about two runs racing — which
does not say a run serves one task; **the wrong citation is kept on the record
beside the right one**, because a wrong citation is load-bearing in the same
way a right one is. Parsing landed with two policies: strict toward the model,
which can fix it, and lenient toward a record, which is append-only and may
have been written by a newer binary.

Rung 2 read a signal the outbox had recorded since the day it existed.
`args_before` sits beside `args`, so `sent && !edited()` — the owner read a
letter written in their name and sent it as drafted — needed no new recording,
and `mineable_as_writing`'s own docstring had been naming the gap for months.
`WritingOutcome` reads it; `SentUnchanged` is deliberately **not** mined as a
correction, which is the `"Blocked by a hook:"` rule in its positive form. It
is also immune to reward hacking by construction rather than by policy:
nothing the model does can produce one except drafting something a person then
chose to send unaltered. Its rate is `None` over an empty denominator, never
zero — the null-run bug arriving in the one measure whose job is to say
something went well.

Rung 3 records the conditions a run happened under, read-only. `backlog.rs` is
one walk over five stores for three readers' questions; `Homeostat` lands on
`RunStats`, opt-in on `cancel`'s precedent so `eval` and the replay probes stay
unsampled — **a scorecard that varies with how busy the box was is not a
scorecard**. Two things were left deliberately absent rather than stubbed:
context pressure, because `RunOutcome::usage` is the run's *total* and would
read as impossible pressure, and `/slots`, because a sensor with no consumer
should not put an HTTP call in every run's start. And the backlog is recorded
as a **delta**, not a level: a run that stages nine drafts raises the outbox by
nine, so a level at run end cannot separate a run's own output from what it
inherited.

**2026-08-27 — admission control, and five findings out of the arc that
prompted it.** R1 shipped on the number R3 measured rather than the one it
was proposed with (`permit.rs`): three background seats against the server's
four, held as files on `runmarker`'s rules because a delegation is now either
a chat session inside `mecha serve` or a detached child, sharing nothing but
the filesystem — so `batch.rs`'s in-process bound, which the design called
for, would bound each process separately and none together. **Interactive
work takes no permit at all**: the reserve is an absence rather than a
control, because a pool that could refuse the person at the keyboard is a
mechanism failing closed against the only user it exists for.

**And a review of the context-pressure arc (#78) found eight things, of which
five were fixed the same day.** Two were the same failure in different
places: a subagent got the `compact` tool and its channel but not the
threshold — the channel rides on `ToolCtx`, which the child clones wholesale,
while the threshold rides on the `Agent`, which the child rebuilds field by
field, so only one made the trip — and the loop took the request with a
destructive `swap` *before* the guard that decides whether it can be served,
so the one path that cannot honour a request was the one consuming it. Both
told the model a summary would happen and produced none.

The third is the one with the longest reach: **`mecha eval`'s tool surface
had come to depend on local config.** `compact` is registered from whether
this machine's settings give the run a compaction threshold, and the tool
list is the front of the cached prefix — so two differently-configured boxes
graded different prefixes and neither scorecard recorded which. The list of
things eval forces off had been written in prose across forty lines, each
entry added when it occurred to somebody, and one was missed; it is now
`force_reproducible`, one function with a test over the whole set, so the
next addition to the tool surface has to be decided about rather than
remembered.

Also: `compact`'s description promised `recall`, which only the
session-recording front-ends register, so on a trigger or a Slack thread it
both wasted a turn and asserted the reversibility that justifies the tool
being unapproved to exactly the runs where it is false; and `Forecast::used`
documented a total it does not compute — it excludes the results of the turn
being executed and cannot include them, being an argument to the call that
produces them. Left understated rather than padded to an upper bound, since
trading a small understood undercount for a large invented one is the wrong
direction on a number the model plans against.

**2026-08-26 (eighth pass) — delegation became a conversation, which is what
D2 said it was.** The owner tapped *ask mecha*, the card moved to `waiting`
and vanished out of the view it was tapped in, and nothing happened that
could be talked to: *"there is no chat UI or any way for me to communicate
with the agent."* The card was telling the truth — `planning` is what
`stateOf` derives while a run is in flight — but the web path spawned a
**detached unattended child**, so the only conversation on offer was the one
you could read afterwards. D2 has said the opposite since the design was
written: *"the run is a conversation from the start, not a fire-and-forget
job."*

So the tap opens the task's conversation as an ordinary chat session, and
everything asked for arrives with it because it *is* the chat surface —
voice, uploads, the live todo panel, approval cards, and typing at a run in
flight being the steering the loop already understands. The model speaks
first (*"it doesn't make suggestions or ask me anything"* was the complaint
under the complaint), and nothing on the board moves, because `waiting_on`
names who has the ball and while the owner is in the conversation they do.

**Three things had to become true for that to be safe, and each is a rule
that had only ever been enforced one way.**

D6 — *the agent may not close its own task* — worked by a spawned child
taking `kg_task_update` off its **own private registry**, and a web process
holds one `Arc<Agent>` for every session. So the narrowing moved onto the
run: `RunContext::withheld`, a denylist beside the skill allowlist, checked
at the dispatch seam and landing on the same `Blocked by policy` refusal
(never an environment error — the counters read that rate), inherited by
subagents the way hooks and the outbox route are, and matched through a
server prefix so `prefix_tools` cannot switch it off silently. A resumed task
transcript keeps it, because D6 is a property of the conversation rather than
of how it was opened.

*One conversation, one writer* was enforced **within** a process and nowhere
else, which was fine while every resume-capable surface owned its own runs.
A detached child broke it: resuming a delegation mid-flight would have put
two writers on one JSONL. The run marker now names the session it is writing,
`live_writer_of` asks that question of the other processes, and both doors —
`/api/resume` and `mecha chat --resume` — answer with the task named. A guard
on one door is a UI condition.

And *the map is a cache; the transcript is the record*. A task conversation
lives in serve's memory and in a JSONL, and only the second survives a
restart — so re-opening after one minted a blank conversation under the same
key, losing the thread, the header and the withholding together. The board
had held the link since the conversation opened; nothing read it back.

**Hand-over is a transfer of the single writer, not a copy.** serve releases
the session, the child loads the same transcript — messages *and* recorded
taint — and continues in it; the turn that starts it says only what changed,
that the owner has gone, because the plan is already above it and restating
it would replace what was agreed with a paraphrase of it. That one shipped
**broken and looking healthy**: `--resume` was parsed, passed, and never
used, so the child opened a new transcript whose first line was *"carry on
from what you have both agreed above"* with nothing above it. The board
moved, the run started, exit 0. Only asking which transcript *grew* found it.

**A question with nobody there now parks.** The obvious reading of
"interactive when the page is open, autonomous when it is closed" is a switch
on whether anyone is connected, and that gets the one case that matters
wrong: a backgrounded phone stays connected, so the card is shown to an empty
room and expires into a refusal. So the card is offered whenever anyone might
see it and *both* ways of going unanswered end the same — stored question,
ended run, no slot and no cached prefix held. Waiting indefinitely costs
nothing because nothing is left waiting.

**And delegations got their own turn ceiling, after the two limits turned out
not to compose.** `cx.budget.max_turns.unwrap_or(cfg.max_turns)` is an
*override*, not a minimum, so a task run inherited whichever surface it
started from — 12 from `[agent] max_turns` on this machine, or a hardcoded 40
in the web chat, neither chosen for autonomous work. Twelve tool round-trips
is an errand; a real delegation stopped mid-way reporting `MaxTurns`, which
reads to the owner as the model giving up. 200 now (Terminal-Bench's figure),
on the argument that a turn ceiling is a backstop rather than a policy: the
loop guard, the token budget and compaction are what stop a runaway run, so
the ceiling should only ever stop an honest one.

Verified live end to end rather than asserted: a conversation opened on a real
board task ran 8 turns and 13 tool calls unprompted — graph, web, two mail
threads, the `research` subagent — and came back with a proposal and its
questions; a restart of `mecha-serve` then returned the same session with 15
entries and the header intact; and a hand-over took the transcript from 19
records to 38 in one file, carrying `private + untrusted` across the change of
hands, staging a draft rather than sending it, and returning the board to the
owner.

**2026-08-26 (seventh pass) — a run in another process can be told
something.** The report was *"I pressed ask mecha, the card said planning,
and nothing happened"* — and the card was right: a run was in flight, and
`planning` is exactly what `stateOf` derives when `waiting_on` names the
agent and no `todo` has been written. What was missing was any way to say a
word to it while it ran. `open the conversation` already existed and was
hidden on purpose, because a delegated run is a **detached child** and its
transcript has one writer.

So steering travels the way stopping already does: a file the runner polls,
drained into the run's own `queued_input` — the same queue a TUI's typed
steering goes into. `agent.rs` is untouched;
`run_interruptible_watching` gained a `pump`, deliberately named for nothing
more specific than *something to do on every watch tick*, so the loop never
learns that a steer can arrive as a file any more than it learns where a
tool came from. Verified live rather than asserted: the instruction arrived
as a text block appended to the user message carrying three `tool_result`s —
the shape the API requires and the one the loop's own doc insists on — and
the model obeyed it on its next turn, answering in one line as told.

**And it closed a hazard that was already open.** Every surface that picks a
session back up refuses to mint a twin of one *this* process holds, and none
of them could see a detached child, so resuming a delegation mid-flight
would have given one JSONL two writers. The run marker now names the
transcript it is writing and `live_writer_of` asks that question of the
other processes; `resume` answers 409 with the task named. The UI's
`!working(t)` condition was never the guard — it was the only thing standing
in for one.

Three rules the marker store already knew, applied to the new file: a steer
is **appended** (two sentences typed a second apart are two things the owner
meant), **drained** (a file that survives its own delivery arrives again on
every later turn), and a run starts **uninstructed** the way it starts
uncancelled — which is the stale-cancel bug this module was extracted for,
arriving in a second costume.

The seed's bullet order moved in the same pass, and the reason did not
survive its own measurement: pooled across the day it looked like the new
bullets had suppressed `ask_user` (5 of 6 against 0 of 4), but the arms ran
different tasks, and the one within-task series splits 1-of-2 / 0-of-2 /
0-of-2 across the three seed orders. The order stands on reading order and
is pinned by a test; the pooled number is confounded and is recorded here so
it is not quoted later as a result.

**2026-08-26 (sixth pass) — D4, and the assembler that points instead of
pasting.** The last open item of the task-agent arc, and it needed less
building than the design implied, because three things had become true that
D4 predated. `kg_task_list` returns `captured_from` on every row — the
provenance pointer that shipped that morning — and `work_prompt` never
mentioned it, so a task captured from an email reached the agent as a bare
sentence while `mecha tasks source` sat on the CLI able to fetch the thread.
`defer_until` was dropped the same way. And the run's own surface already
held `kg_search`, `kg_entity`, `kg_related` and `kg_timeline`, so the
project neighbourhood D4 wanted assembled into the prompt was one call away.

So the seed **points**: the provenance line, `defer_until`, a bullet naming
the mail reader when the capture is mail, and a bullet naming the graph
lookups. A seed is the front of a cached prefix every turn of every task run
re-sends, so pasted context is paid for on all of them where a sentence
naming a tool is paid once and followed only by the runs that need it —
`skill.rs`'s progressive disclosure one door over.

**The constraint that decided the shape is not the token budget.**
`captured_from` can point at *mail*. Pasting a thread body into the seed
would arm `untrusted` before the run's first turn **and** put
attacker-controlled bytes into a privileged run's opening instruction, which
is `frontdoor::Record::for_privileged_run`'s argument arriving through a
third door. The seed therefore carries kind, id, account and timestamp, and
never the `label` — a subject line is prose somebody else composed, and the
test fixture's label is an injection so that the assertion is the boundary
rather than a formatting check. The bytes arrive as a tool result, where the
interlock accounts for them and the `<untrusted-content>` envelope is
already around them.

**Tools are named by their registered name, and this deployment proved why
in one command.** `mecha tools --json` here shows the graph tools bare
(`kg_search`) and mail prefixed (`mail__mail_get_thread`) — one machine, two
conventions — so a seed hardcoding bare names would have pointed the run at
a tool it could not dispatch in exactly one of the two bullets. That is the
level-3 skill bug, which was found by running it rather than by reading it.
`Reach::of` reads the registry, after the D6 withholding and the D13
`ask_user` insertion, for the same reason `RunConfig::of` does.

And D4's own line — *"measured in Phase 4, not assumed in Phase 1"* — was
finally honoured, on two runs. A throwaway task named only `mecha` as its
project drew **seven graph calls** (`kg_search` ×4, `kg_entity` ×3) with no
prompting but the bullet. Then a task captured from a real three-message
mail thread called `mail__mail_get_thread` **as its first act**, before
anything else, and the run's recorded taint came back `private +
untrusted` — the bytes arriving as a tool result the interlock accounted
for, which is the entire argument for carrying a pointer instead of a body,
observed rather than asserted. Two runs are a direction and not a result;
pasting stays available if later ones ignore the pointer.

**The same run found the next thing to look at, which is not D4.** It ended
`completed` with six numbered decisions in its closing text and **zero
`ask_user` calls**, on a surface that held the tool (63 tools, `ask_user`
among them; `kg_task_update` correctly absent, so D6 held). The seed's
front-loading was obeyed in content — ask first, one question, list every
unknown — and ignored in mechanism, so the questions live only in a
transcript: `mecha questions list` cannot see them, the phone's card cannot
offer them, and the delegation reads as *finished* rather than as *blocked*,
which is the state D13 exists to prevent. The store is not broken (two
questions are parked from other tasks), so this is prompt adherence on one
run. It is worth naming because it is the precise seam D12 was decided
against on: the argument for putting the intervention on the user turn was
that this model obeys that channel, and here it obeyed the words while
skipping the call.


**2026-08-26 (sixth pass) — the harness stopped finding out by being refused.**
Rungs 4 and 5 of the goal system, and one measurement that had to exist before
either could be judged.

Rung 4 made the replay corpus a *draw* rather than a recency slice
(`harness_probe.rs`), and review caught the two things that decided what it
measures. `holdout_n` was clamped to the pool while `selection_n` came off the
unclamped want, so a corpus smaller than asked for went almost entirely to the
holdout — five held and one selected at the defaults over six eligible
episodes, which is five sixths of the real-model-run budget spent on the slice
that cannot decide anything. And headroom was read from `Session::last_outcome`
while an episode is the *whole session*: `drive_episode` replays every recorded
user turn and folds each with `absorb`, so the priority signal was sized in a
different unit from the arms it feeds, and it inverted — nine error-heavy runs
and a clean tenth scored zero and sorted last. Fixing the second needed a fold
over recorded rows, and writing a second fold would have been the hazard the
fold exists to close, so `absorb` split into `of_run` + `merge` with
`Session::episode_stats` folding through the same `merge`.

Rung 5 is `pressure.rs`. `compact_at` is checked at the top of the loop against
what the provider charged for the *previous* request — but by then the
assistant turn and a batch of tool results are already in `messages` and nobody
has priced them, so the reading the decision is made from describes a list one
turn out of date. §4.4 of the design proposed predicting from an observed
growth rate; building it corrected that, because **there is nothing to
extrapolate**: the un-priced tail is measurable in bytes, and the provider
re-supplies the byte-to-token conversion every turn by pricing a list whose
size is known. So the predictor is arithmetic on two measurements — no tuned
parameter, no model call, which §7.4 requires of anything running during a
turn.

Four things carry it. The **delta form** anchors on the last real measurement
and adds only the marginal cost of what changed, which removes the system
prompt and tool specs from the arithmetic because they are already in the
anchor. The rate is **clamped into the band a tokenizer can occupy** — never
below the plain-text rate, never above one token per byte, and not measured at
all from a delta under 512 bytes. `over` is spelled `reported || predicted` and
never the prediction alone, which is §7.3's monotonicity as one line of code
with a property test over a grid of states. And a **rewrite retires the reading
it invalidated**, which is the one thing here that can move a decision later
and is not an exception: a reported size is a measurement *of a particular
message list*, and once eviction rewrites that list the number is not a reading
of anything.

That last one fixed something that had never worked. After the free passes
freed space the loop `continue`d, meaning to "give it a turn to take effect
before paying for a summary" — but it jumped to the top without sending a
request, `prompt_tokens` is assigned in exactly one place and only after a
response, and the three passes are idempotent, so the re-entered check saw the
identical stale value and the summary was paid for anyway one iteration later.
Measured on the test fixture: **three summary requests, all waste.**

Rung 5 then took the tool-output budget too. `[tools] output_budget_bytes`
sizes "the gap between the threshold and the window" from the *window*, once,
at startup; the tracker knows where the transcript actually is, so under
pressure the cap is `min(configured, affordable)` — a `min`, so it can only
narrow, and gated on `spill_dir` being set, because `cap_result` relocates
over-cap bytes to a file the jail admits only when there is somewhere to put
them. That condition is what §7's table needs to be true and does not state.

And the series moved onto `Conversation`, beside taint. One submission is one
run in chat and the TUI, so a per-run tracker started empty on every user turn;
it resets when the model, system prompt or tool surface changes, because an
anchor is a token count for a byte count under one tokenizer and there is
nothing to convert it *to*.

Underneath all of it, `context_overflows` on `RunStats` — the measurement that
had to land first, in its own change, because a baseline established in the
same commit as the thing it grades is not one. `compactions` counts summaries,
so an overflow answered by eviction and thinning alone incremented nothing and
was invisible in every store; the harness caught a 400, rebuilt the transcript
and retried, and nothing said it had. It is the one counter on `RunStats` typed
`Option`, because the corpus it is read from spans the commit that introduced
it and a plain `u32` would read every older row as a run that overflowed zero
times — quietly diluting the rate it exists to establish.

**2026-08-27 — the harness stopped taking the model's word for it.**
Rung 5's model-facing half, then the whole of rung 6, which is the last rung of
the goal system with no model in it.

Rung 5 finished where §7.1 said the second-order win was: the harness had been
predicting context pressure and acting on it — compacting early, narrowing the
tool-output budget — while the *model* could not see any of it, so an
anticipatory signal that could act on the **plan** was going unclaimed.
`ContextTracker::forecast` returns headroom, observed cost per recent turn and
turns remaining **as measurements**, so nothing asks the model to estimate its
own token use; the reading rides on the **`todo` result**, where it appears
when the plan is being revised and costs nothing on other turns; and a
`compact` tool, unapproved and argument-free, hands the *when* to the one party
that knows how much of its own plan is left. Monotonicity is structural rather
than promised — the floor stays `reported || predicted`, so nothing the model
decides can make a run compact later than it does today.

Rung 6 is `step.rs` and `boredom.rs`, and both exist because of the same
asymmetry read one tier apart.

**A step is closed by the model and nothing checked it.** A board task is
closed by the owner (D6), so a person is the check; a todo step has no person,
so the check has to be structural — D5's *derived from the record, never
self-reported*, reaching one tier below where it was written. The loop folds
its own trace into counters and stamps them on `ToolCtx` without learning which
tool cares (the `taint`/`context` precedent), and the plan tool — the only
party that knows where a step began — differences two of them into a span.
Two readings, both facts rather than thresholds: nothing was attempted, and the
last attempt did not succeed.

Four of its five decisions are about *not* firing, which is the half that
decides whether a check survives contact. A refusal is not a failure
(`unknown || (is_error && !denied)`, the eval rig's rule one tier down), so a
blocked step is told it was blocked. Only the last attempt decides, so a
recovered failure is the model working. A sibling still in flight supports no
finding, because mecha runs a turn's calls concurrently and doing the work and
ticking the box in one batch would otherwise read as the null step. And plan
revision is subtracted from every span — three `todo` writes mid-step are three
trace entries, which is precisely how a step where nothing happened reads as
busy.

The fifth was found by asking where the counters come from. **The trace is per
run and a conversation is many runs**, so in chat a step started before the
user last spoke differences against a larger number, saturates to zero, and
announces the null step on the commonest shape there is — the loudest reading
firing on ordinary work, which is how a check gets switched off. A mark from
another run is therefore unmeasurable rather than empty, which needed a run
identity minted at the loop's existing per-run stamping seam: a `RunContext` is
shared across every chat turn, so an id on the context would have been one
value for the life of the conversation and invisible to the reset it exists to
catch.

**And a run going nowhere had exactly two states, proceeding and dead.** The
loop guard is §9.1's rung 5 and was the only rung: dormant until a compaction,
and its response is to end the run. `boredom.rs` is the graded version, keyed
on the call *and* its result on the guard's own rule, and on
`compact::target_of` rather than the raw arguments — so two tools that reach
one file and get the same bytes count as one thing learned twice, while
identical arguments with a *changing* result stay polling. It spends nothing,
which is what makes it ungated where curiosity is not.

Its bounds all come from one piece of evidence, the same one behind
`collapse_repeated_failures`: a model is measurably likelier to fail a step
when its context holds its own earlier errors. So a rung is crossed once
(`==`, never `>=`), one notice per turn, three per run — nagging a stuck run is
a way of keeping it stuck.

**It shipped as rungs 1 and 3, and the missing one is a stated reason rather
than a to-do.** Rung 2 — consult — has two halves and neither is reachable: a
§7.4 marker does not exist, and while a skill does, nothing in the `Tool` trait
identifies the tool that loads one. `narrows_surface_to` is the closest and
answers `None` until a skill is already loaded, so it recognises the state the
notice exists to escape only after the escape has been taken. Rung 3 needed the
same kind of property and got one: `Tool::runs_a_fresh_conversation`, fourth in
the family with `carried_state`, `fixed_workspace` and `narrows_surface_to`, so
the loop learns that *some* tool starts clean and never that subagents are a
thing. Nothing else could have answered it — a delegate's name is whatever the
user called it in config, and its capabilities are derived from its child's
tools.

Then rung 7's observation half, which is the first rung whose *result* is a
number rather than a mechanism. `appraisal.rs` is the signed record §0 says
mecha never had — five channels, an agency, an exposure flag, and a label that
is a **pure function of the record**, because a model that reads a run and says
"frustrated" is a self-report: unfalsifiable, drifting, and precisely what a
fetched page saying *you have failed your owner* is aimed at.

Working §6's derivation table answered most of the degeneracy question before
any corpus did. Six of the ten labels are unreachable from what mecha
measures: `Pride` needs a charter line and a task well done is deliberately not
one, `Guilt` needs a notion of harm that nothing computes (`visible` is
exposure, a different claim), `Shame` is a cross-run pattern a per-event
function cannot see, `Excitement` needs a predicted error — and `Regret` and
`Disappointment` split on the counterfactual verdict, which costs a real model
run per arm. `Affect::reachable_today` makes that testable rather than only
written down.

Then the corpus agreed, harder. Over the live store `mecha sessions appraise`
reads 459 sessions, appraises the 120 that recorded an outcome, records **119
signed goal errors** and derives **neutral for every one**. Eleven of those
errors are positive — a draft written in mecha's name that the owner read and
sent unchanged — which is the one channel in this system that can say something
went well, recorded since the outbox existed and never counted anywhere until
now.

The useful part is what that implies about build order, which is not §14's. A
counterfactual verdict would give all 102 intervention errors a label
immediately, where the charter at rung 10 buys only the eleven positive ones. So
the probe is the cheaper half of the readout, and rungs 8–10 must not be built
on a label that is currently a constant.

Two design corrections came out of building it, both about units. **It is per
session, not per run** — the §5 record carries a session id and no run index,
and that turns out to be load-bearing: both working channels are session-scoped,
so a per-run appraisal multiplies them by the number of times a session was
resumed, measured at 5.9× on the intervention channel before it was caught, the
same mismatch rung 4 paid for in the other direction. And **there is no store**,
against §10: every deterministic channel is a pure function of records the
machine already keeps, so a store would be `runlog`'s rejected ledger — faster,
and a second source of truth that can disagree with the first. It earns one with
the first channel that costs something to compute.

**And building it turned up something already wrong.** `agent.rs` prefixes a
refusal it did not author with `"Denied by the user: "`, and the rule from that
is in CLAUDE.md; this is its mirror. `learning::extract_interventions` guarded
one harness voice, `FINAL_ANSWER_NUDGE`, and `EMPTY_TURN_NUDGE` was never in the
list — so every run the harness had to nudge contributed an "intervention" whose
text was mecha's own. Found by adding a third voice (boredom's notice, which
uses steering's slot) and asking what already read that slot. **Two reflections
in the live store were mined from it**, one of them `origin: clean`, unprocessed,
and therefore a candidate for a rule in every future prompt — and its lesson is
the nudge's own sentence handed back, *do not restart or re-derive steps already
processed*, which is what makes the shape hard to see: mecha teaching itself
something it was already obeying reads exactly like the loop working. Nothing had
consolidated, so no rule carries it. Fixed at both ends, because they fail
differently: `agent::is_harness_voice` is the closed list, owned by the party
that adds a voice, and `Reflexion::learnable` refuses one whatever its origin,
which is what reaches the records already on disk.

Underneath both, `RunStats::boredom_notices`, on `context_overflows`' rules and
for its reason: every threshold in `boredom.rs` was argued rather than
measured, and a detector nobody can count fires either constantly or never with
no way to tell which. `Option`, so rows written before the sensor do not read
as runs that were never bored. Recorded *after* the thing it grades, which is
the wrong order and is worth saying so — rung 5's own `context_overflows`
landed in its own change first, because a baseline established alongside what
it measures is not one.

**2026-08-27 (second pass) — the loop that had never once run, and the comment
that named three risks and missed the fourth.**

Two lanes worked the counterfactual probe from opposite ends and found the same
thing from different sides: `mecha validate`'s steer and denial probes had
**never been able to run on an interactive session**, and `validations.jsonl`
was empty not because nobody ran the nightly but because every probe skipped.
The counter reported it faithfully. Nothing read it as *this entire class is
unreachable*, which is a rate over a zero denominator printed as zero, in the
one store whose whole job is to be the evidence a gate is replaced by.

`replay_registry` refuses to build when a recorded tool is missing, and that is
right — the tool list is the front of the cached prefix, so a smaller toolbox is
a different agent and its divergences say nothing about the question. The tools
it caught were the ones registered **only by a front-end**: `ask_user` in 246 of
408 sessions, `recall` in 122, `show_file` in 55. The coupling closes on itself
— a probe needs an intervention, interventions happen interactively, interactive
sessions carry `ask_user` — so the bail fired on exactly the population the
probes exist to read. **319 of 407 sessions.** A `surface_only` registry
supplies them where nothing executes, and the store goes from 22% replayable to
76%.

The gate is on what a mode *does*, never on its name: `Stop` **and** `Error`,
because they run identically and differ only in the policy a caller applies to
the report. Not `Live`, where the replay continues as a genuine fresh run and a
fresh run holding a permanently-erroring tool is not one — its divergence would
read as a finding about the model when it is a finding about the harness. That
is measurement validity rather than fail-fast, and it took two wrong answers to
get to: gating on `Stop` alone leaves a non-executing mode still bailing, and
gating on nothing at all was the correction that overshot.

**Then the probes ran, and 12 of 13 came back inconclusive.** Three hypotheses
died in order, each killed by the lane that had not proposed it. Sampling
nondeterminism: the store records `seed: 42` on 388 of ~394 sessions, so the
sampler was pinned three ways over. Contention: `/slots` polled throughout every
run, maximum one slot busy, which was the probe itself. Recency of the one
success: it is *older* than the sessions that fail. What replaced them is
better than any of them — **yield is a race between the probe point and the
divergence point**, and the one success has its intervention at turn 2, the
earliest in the set.

The signature that settled the cause is worth keeping. Divergence indices
`#0:2 #1:6 #2:2 #3:1 #5:1`, median **one call**, against steer points from 3 to
33 — and one session contributing six probes with steer points at 10, 17, 20,
21, 28 and 33, **all six diverging at the same call**. A trajectory-dependent
cause cannot produce that. A per-session constant can.

**The constant is the tool surface, and the comment that should have caught it
was one word short.** `RunConfig` keeps the system prompt in full and says why —
*"the text lets a replay rebuild the request"* — and keeps the tools as names,
under a comment naming the risk it saw: *"a tool added, removed or renamed
between recording and replay changes what the model could have done."* Add,
remove and rename are the three that almost never happen. **Re-describe happens
constantly** — 49 commits touched tool definitions in three weeks of this store
— and a list of names cannot see it. Tools render *before* the system prompt, so
the replay was rebuilding the back half of the prefix byte-exactly and the front
half from whatever the registry said today.

`surface.rs` is the fix and it was decided by a measurement rather than a
preference: the specs are 69 KB against a 25 KB average session, so inlining
them would have quadrupled the session store, and a hash alone gives up the
rebuild that is the point of recording it. So they are written once per distinct
surface and cited by hash, on `ValidationRecord`'s `rules_hash` precedent.
`Option`, so a recording from before the field lands in **Unknown** and can
never read as matching — and `Differs` needs no blob at all, so legibility
arrives the day the field ships while rebuildability accumulates after.

**It recovers nothing already recorded, and the honest form of that is a
sentence nobody wanted to write**: the appraisal corpus and the validation
ledger start from zero the day it ships. The prediction it makes — restoring the
surface should push the divergence index up — is therefore **untestable on the
current corpus**, and both lanes agreed to say so rather than re-run the 13 and
read something into the number.

One thing did come out end to end. The first non-neutral affect label this
system has produced: `{neutral: 119, regret: 1}` — one session where the replay
went elsewhere without the steer, carried from the probe through `apply_probe`'s
`Agency::Own` to `affect_of`. One of 120 is not a result. It is the seam
working.

And two smaller findings, both of the same shape as the big one. `serves:` has
**never carried a value in production** — 112 of 120 sessions wrote a plan, none
named what it serves, including 15 delegated task runs, which is the case the
field exists for. And `sessions appraise` reported that zero **by construction**:
it passed `&[]` for goals unconditionally, so the command built to measure
whether the labels were degenerate could never have reported anything else.
Absent and zero conflated inside the instrument.

**2026-08-27 (third pass) — the stack this section describes landed, and one
finding named twice did not travel with it.**

Six PRs merged the day these two sections were first written: #86 (`bcd7d4f`),
#87 (`c81d0fb`), #93 (`4f28221`, carrying forward #86's fourth-round fix that
missed the merge by eight minutes — see the trap below), #88 (`18a6fcf4`), #89
(`1e04c114`) and #90 (`c630ff94`). Every review finding attached to #88, #89
and #90 was checked against the merged source rather than against the review
thread, on this skill's own rule.

**#88's two open findings both hold up as fixed.** `affect_of` now computes
`label_of`'s most-negative reduction *before* checking repetition, and only
lets the repeated-error branch return `Frustration` when
`says_more(Frustration) >= says_more(reduced)` — so a repeated self-inflicted
error can promote a `Neutral`/`Anger`/`Disappointment` verdict but can never
step in front of an exposed one, closing the case where a ceiling nobody
caused plus a visible mistake used to report as mere repetition
(`mecha-core/src/appraisal.rs:378-391`). `says_more` is exhaustive with no
`_ => 0` arm, so a future `Affect` variant fails to compile rather than
silently tying with `Neutral` (`appraisal.rs:294-302`). The doc-count drift is
fixed too: the printed line now reads "six of the ten `Affect` variants"
(`mecha-cli/src/commands/sessions.rs:570`), matching `reachable_today`'s true
count of four reachable against ten total (`appraisal.rs:233-238,963`).
Underneath both, three more fixes from the same review pass: `of_session`
takes `end_taint: Option<crate::agent::Taint>` rather than a bare `Taint`, so
a caller with no coverage passes `None` and the fail-closed `Untrusted` arm in
`classify_origin` stays reachable instead of being permanently shadowed by an
always-`Some` wrapper (`appraisal.rs:432-447`); `visible` for a drafted
message now reads `item.writing_outcome() == Some(WritingOutcome::SentUnchanged)`
rather than the item's raw `status`, so an edited-then-sent draft — the
owner's own catch — no longer reports as an exposure error
(`appraisal.rs:606-613`); and `RunStats::merge` folds `boredom_notices`
through the same `Option`-sum as `context_overflows` instead of leaving it
fixed at whatever the first row carried (`mecha-core/src/session.rs:397-405`).
And `sessions appraise` collapsed three separate reads of the same transcript
— outcome, interventions, goal — into the one `Session::read` the module's
own doc says exists to replace exactly that pattern
(`mecha-cli/src/commands/sessions.rs:372-398`).

**#90's three findings hold up too.** The surface store hashes with
`learning::rules_hash` — the same FNV-1a function `ValidationRecord` already
uses — rather than a fresh `DefaultHasher`, which this repository has banned
twice before for the same reason: a `DefaultHasher`'s algorithm is a `std`
implementation detail with no stability guarantee across a toolchain bump, and
a persisted key built from one silently stops matching itself
(`mecha-core/src/surface.rs:118,141`). The store root is created through
`create_private_dir`, matching every sibling `~/.mecha` store instead of
leaving it world-readable by omission (`surface.rs:161`). And the
probe-budget warning now gates on `tally.over_budget > 0` rather than on
`wanted` — which counted every intervention including the unprobeable
`followup`/`edit` ones — so a corpus that is mostly followups no longer
prints "budget stopped" when nothing was actually capped
(`mecha-cli/src/commands/sessions.rs:453-499`). The `OnDivergence` gate for
the surface-only registry is an exhaustive match (`Stop | Error` explicitly,
not `Live`) rather than a `matches!` denylist, and `Fidelity::of` is wired
into `appraisal_probe::annotate_with_fidelity` rather than sitting unused
(`mecha-core/src/replay_run.rs:223-230`, `mecha-cli/src/appraisal_probe.rs:75`).

**#93 is exactly what it says: a same-content carry-forward, no open
findings.** `runlog::boredom_rate` and the denominator-aware "went nowhere"
line both exist as `sessions health` prints them
(`mecha-core/src/runlog.rs:227`, `mecha-cli/src/commands/sessions.rs:776,789`).

**#89 is the exception, and it is worth stating precisely because the shape
is a trap rather than a one-off.** Five real findings from its review rounds
are fixed and verified against `main`: the proposals pane reads `kind`/
`detail`, the keys `proposals list --json` actually emits, instead of a
`status` key the command never produces
(`mecha-cli/src/tui/learning.rs:454-455`); `mecha rules show` exists
(`mecha-cli/src/commands/rules.rs:36,77,240`); `find_rule` refuses an empty
needle instead of prefix-matching every identified rule
(`commands/rules.rs:175-186`); the Reflections pane loads with `--all` so a
dropped row stays visible for `u restore` to reach
(`mecha-cli/src/tui/mod.rs:6010-6025`); and `wrapped()`'s word-break branch
uses `saturating_sub` instead of an unchecked subtraction that panics in
debug and loops forever in release on a narrow, deeply-indented terminal
(`mecha-cli/src/tui/queues.rs:1182`).

But at the time this section was first written, three findings that were
named — one of them twice, in two separate review rounds — were still true
of the code on `main`: `App::a_modal_is_up` did not check `self.learning`,
so the modal's mouse capture never released; `learning_act` called
`self_cli` synchronously from the key handler, and every verb it ran took
the learning store's flock that `reflect`/`learn` hold across a model call;
and `reject`/`drop` passed no `--reason`, contradicting the modal's own help
text — with `doctor`'s starved-learner finding compounding the last one by
counting every owner-dropped reflection as "excluded by origin" alongside
the ones the provenance gate actually excluded.

The general lesson stood before the fix and still does: **a later review
round approving a PR is not evidence that an earlier round's findings were
addressed** — a reviewer re-reads the current diff, not the accumulated
list of everything anyone has said about it, so a finding raised on round 2
and never mentioned again on round 3 through 6 can merge exactly as it was
found. The fix is the one this skill already prescribes for a different
failure — verify against source, not against the review thread — applied
one layer up: a merged PR's own review comments are commit-message-shaped
evidence too.

**2026-08-27 (fourth pass) — the rest of the stack landed the same evening:
#91, #94 and #92, in that order.**

**#91** (`f0ca8ca`, mecha-80's) is the probe half neither the second- nor
third-pass entries above had landed yet. It fills `GoalError::controllable`
from a real counterfactual: rebuild the recorded run up to a `steer`/`denial`, replay it
**without** the steering text, and read whether the run arrives at the same
place anyway (`Some(true)` if not — the agent could have done otherwise —
`Some(false)` if so; wired at `mecha-core/src/appraisal.rs:692,695`). It is
also where the reach and yield numbers already recorded in this section's
"(second pass)" entry (13 of 102, 12 of 13 inconclusive) were actually
produced, and where the one genuinely new label came from: `Agency::Own` +
`controllable: Some(true)` reaching `affect_of` as `Regret` — the first
non-neutral label this system has ever emitted from a real run rather than
a unit test, `{neutral: 119, regret: 1}` over the 120-session corpus.

**#92** (`a0638c8`, mecha-4c's) fixed the `serves:` seed gap this document
had attributed to "nobody was ever told" — the truer diagnosis, caught in
#92's own review, is that `TodoTool::description` already told the model to
pass `serves`, but nothing bound that generic reminder to *this run's*
specific task id, so all 15 delegated runs in the corpus carried the
instruction and their own id on the seed and still wrote nothing. Both
delegated postures in `tasks.rs` now bind the id explicitly. #92's own
commit is the record of what changed in the code; this entry does not
re-derive it, on the same rule that keeps this file from re-deriving #86's
fix once #93 already carried it.

**#94** (`2f432b3`) is a second cleanup pass on #89, filed the same evening
this section's "third pass" entry was written — before that entry's own
four still-open findings could be called old news, a re-read of *every*
review comment on #89 (not just the ones the first cleanup pass covered)
turned up two more of the same shape, and all six shipped together. Verified
against the current tree: `App::a_modal_is_up` now checks
`self.learning.is_some()` (`mecha-cli/src/tui/mod.rs:601`); `reject` on a
proposal now passes `--reason "rejected from /learning"`, the exact
fixed-string pattern `/queues` already used for the same command
(`tui/mod.rs:6274`); `doctor`'s `excluded` counter now skips a row with
`dropped_at` set (`mecha-core/src/doctor.rs:1442`), and `mecha learn` reports
its own `dropped_by_owner` count instead of folding it into "excluded by
origin" (`mecha-cli/src/commands/learn.rs:107-150`); `learning_act` now runs
every verb through the detached-and-watched shape `/outbox` and `/triggers`
already use, polled by a new `Watch::Learning`, instead of blocking the key
handler on `self_cli` (`tui/mod.rs:6140-6161`); `LearningStore::reflexion`
carries the same empty-needle guard `rules.rs::find_rule` got in #89's first
review pass, never carried to this sibling lookup until now
(`mecha-core/src/learning.rs:1103-1110`); and `/queues`' `move_sel` now
clears `review_detail`/`detail_scroll` on every move, including the
`g`/`G`/Home/End jumps that bypass the delta path
(`mecha-cli/src/tui/queues.rs:500-508`) — before this, scrolling past a long
proposal's fetched text with `j`/`k` could leave the previous proposal's
text on screen while the cursor sat on the next one, so an `a`/`r` right
after could act on something never actually read.

Two traps in one PR sequence, same shape, six hours apart: #86's fourth
review round missed the merge window by eight minutes and needed #93 to
carry it forward; #89's second and sixth review rounds named findings a
different pass's cleanup did not re-check and needed #94. Both are the same
lesson — a standing "merge once reviewed" authorization, or a cleanup pass
scoped to "the findings I already know about," answers a narrower question
than the one that matters, which is *has everything anyone has said about
this diff been addressed*. Neither failure was expensive to fix once
noticed; both were invisible from the outside for as long as nobody looked
past the PR's own final "looks good."

**2026-08-27/28 — rung 9's first two pieces landed: an episode now carries
how the session went, and the world disagreeing with the graph is noticed.**
Two PRs, `docs/GOAL-SYSTEM-DESIGN.md` §10 and §10.1: **#97** (`4d9f27f`,
episode tagging) and **#98** (`c32eaca`, surprise/gossip-seeding). A third,
**#101** (`be9e32b`), is four rounds of review fixes on top of both — the
built-in review bot had hit a transient action-infra failure on every one of
its last several attempts, so neither #97 nor #98 had actually been reviewed
before merging, and a Codex-driven review was requested instead.

`appraisal::for_session` (`mecha-core/src/appraisal.rs:736`, returning a
`SessionAppraisal` at line 714) is the one assembly `mecha sessions appraise`
and `mecha distill`'s new episode-tagging path now both call — `Session::read`
once, the outcome/interventions/goal off the same pass, on the three-reads-of-
one-file rule `Session::read`'s own doc already names. `mecha-cli/src/commands/
sessions.rs`'s `appraise` function shrank to a call at line 432; `mecha-cli/src/
commands/distill.rs` gained the same call and threads the result into
`distill::upsert_args` (`mecha-core/src/distill.rs:427`), which stamps the
pushed episode's `meta.affect` and, when non-empty, `meta.goal_errors`. Neither
is gated on the session's taint the way a correction is — they are structured
facts the harness computed about its own run, not prose a fetched page could
have authored — with one deliberate exception: a `GoalError`'s `goal` is the
model's own `serves:` argument, so `upsert_args` redacts it to its bare kind
word (`task`, never the id) before it crosses into pkg's data, since the id
itself is unconstrained text an injected plan could have populated.

**#98** extends the same quarantined `Distiller` pass (the one that already
finds `corrections`) to also report `Surprise { predicted, actual, about }`
(`mecha-core/src/distill.rs:134`) — a moment where something the agent said,
sourced from the graph, was contradicted by something else in the same
session ("I said the 14th because the graph says so; the email says the
9th"). Unlike affect and goal errors, a surprise **is** gated on taint —
`surprises_for` (`distill.rs:278`) mirrors `corrections_for` (`distill.rs:253`)
exactly, because a surprise's `predicted`/`actual`/`about` are the model's own
free-text reading of transcript prose, indistinguishable in kind from a
correction's `wrong`/`right`. Deliberately **not** wired to run
`mecha gossip --entity <about>` automatically — `mecha distill` only prints
each one, on this project's standing rule that real model spend needs a human
gate rather than a session's own say-so.

**#101 is four rounds on one PR, and the shape repeats hard enough to be its
own trap** (see Traps → Review process, below, for the general lesson). Round 1
(the first Codex review to actually complete, after several transient
action-infra failures on #97/#98 themselves) found two real gaps: `mecha
distill`'s surprise print used unescaped `println!`, and the "a person's own
terminal is a safe context" argument for that assumed a live terminal —
`scripts/ruminate.sh` actually redirects the nightly run's output to a dated
logfile, exactly as exposed to a deferred read as any other log. And a
genuinely unreadable outbox during episode tagging used to warn-and-continue
(`mecha-cli/src/commands/distill.rs`), after which `mark_distilled` made the
resulting incomplete `Edit` channel permanent — no later run could ever
revisit it. Round 1 fixed both: `strip_ansi` (`mecha-cli/src/logs.rs:175`,
made `pub(crate)`) on every printed field, and a bail via `.context(...)?`
instead of `eprintln!`-and-continue.

Round 2 (the built-in bot, having finally run) found `strip_ansi` only
strips ESC-introduced sequences — a bare `\r`/`\n` survives it and can
rewrite the printed line or forge an extra one, defeating the very "⚠
untrusted" marker round 1 just added. New `strip_ansi_and_controls`
(`logs.rs:240`) closes it. And round 1's outbox fix only bailed on a hard
I/O error; `OutboxStore::items` (`mecha-core/src/outbox.rs:376`) also
silently *skips* a merely malformed item file behind an invisible
`tracing::warn!` (the nightly runs with no `MECHA_LOG`) and still returns
`Ok` — a silently short list indistinguishable from an outbox with fewer
drafts. New `OutboxStore::items_strict` (`outbox.rs:401`, sharing
`items_impl` at line 405) bails on that too; `items()` itself is unchanged
for its other callers.

Round 3 (the bot again, on round 2's fix) found `char::is_control()` is
Unicode category Cc only — U+2028/U+2029 (line/paragraph separator) forge a
line break exactly like a bare `\n`, and U+202A–E/U+2066–9 (bidi
overrides/isolates) can visually reorder the rendered line around the
warning marker, the Trojan Source shape. `strip_ansi_and_controls` was
widened to name both categories explicitly, deliberately not generalized to
a "printable only" filter — the field is free-text prose that may
legitimately carry non-Latin scripts, and a filter that cannot say which
characters it distrusts is guessing rather than closing a described class.

Round 4 found two more, both corrections to claims rather than new
mechanism. `strip_ansi`'s own doc comment claimed its one call site was safe
because `Writer::write` cuts the stream at `\n` first — untrue even at that
function's original two call sites (`Writer::write` and `release()` in
`logs.rs`): `trim_end` only strips a *trailing* `\r`, not an interior one, so
a server-supplied error string or an unparsed model reply with a `\r` in the
middle had been reaching the TUI transcript and a live terminal unstripped
the whole time — a pre-existing bug in the TUI's own log capture, newly
exposed by a doc comment that had never actually been true. Both original
call sites now use `strip_ansi_and_controls` too. And the stated rationale
for `items_strict` — a half-written `.json` mid-save — turned out to be
structurally impossible in this store: `outbox.rs`'s own module header
already says temp-sibling-and-rename means a reader never sees a partial
write. The real cause is persistent, not transient — a stray file, or an
item written by a schema this binary cannot read — and all four places that
had repeated the wrong claim (a code comment, a doc comment, the error
message, and the CHANGELOG) were corrected to say so, including dropping the
implication that a retry would clear it. **Noted, not built**: a `mecha
doctor` finding for a stalled distill ledger — today the nightly fails
silently behind one line in a dated logfile, with no `MECHA_LOG` and no
doctor check for it.

**What's left of rung 9**: review-queue salience, the rest of §10, needs
changes in the private `personalized_knowledge_graph` repository — a
different codebase mecha only reaches through the MCP tool surface — to read
`meta.affect`/`meta.goal_errors` and reorder pkg's review queue on them. Not
started, and not scoped beyond `GOAL-SYSTEM-DESIGN.md` §10's own paragraph
naming it.

**2026-08-28 — rung 10: the charter, and a guilt sensor that shipped
deliberately unconsumed.** PR #100. Landed ahead of rung 8 in §14's build
order on purpose — argued from a measurement, same as the probe reordering
above: both pieces of this rung depend on neither the probe nor the affect
label, so there was no reason to wait on either.

The charter (`mecha-core/src/charter.rs`, `mecha-cli/src/commands/charter.rs`)
shipped as designed in §11: `~/.mecha/charter.toml`, an ordered list of
standing priorities the owner authors and edits by hand, ranked by file order
rather than a weight, with no config field, no project layer, and no write
path anywhere a model or this command could use — `mecha charter` is
read-only. Rendered straight into the system prompt (no progressive
disclosure, unlike skills) via `setup.rs::prepare_tools`. `--no-charter`
matches the skills/rules opt-out and `mecha eval` forces it off with
everything else a scorecard must not depend on.

Anticipated guilt (§7.4) shipped as a **recorded sensor only** —
`crate::guilt::anticipated_guilt`, folding the age of the oldest commitment
recorded in the outbox/questions/front-door stores, how many are waiting, and
the run's own peak context pressure into `Homeostat::anticipated_guilt`.
**Nothing consumes it.** Confirmed before building it that nothing today
wires `Backlog`/"owner-attention debt" to the model at all, so there was no
existing seam to narrow — inventing one under time pressure is exactly the
scope §7.2 warns a guilt mechanism must not acquire. Same precedent as the
homeostat (rung 3) and boredom (rung 6): ship the sensor, let a corpus exist,
decide the consumer deliberately later. Also folded the homeostat into
`diagnose::Evidence` (mean context pressure, mean anticipated guilt, both
`None` over unsensed rows, never zero) so the nightly diagnostician's brief
carries machine conditions beside outcome counters.

**The automated PR review earned its keep here, five rounds deep, and the
last round found the most important bug of the five.** Summarized rather
than itemized, since the PR's own review thread carries the full record:

- A partial backlog read (one store unreadable, others empty) collapsed into
  `Some(0.0)` — indistinguishable from "genuinely nothing owed" — and a
  counted-but-undated commitment scored as fresh instead of unknown. Fixed by
  requiring all three stores readable and by making pressure itself unknown
  rather than a measured zero when a provider declares no `context_window`
  (the same floor `Homeostat`'s own doc already warns against, reintroduced
  one struct over).
- The character budget crossing `CHARTER_CHAR_BUDGET` refused to load the
  whole document — inverting the learned-rules precedent
  (`over_budget_domains` warns and still loads) and meaning an owner's
  eleventh priority line would silently un-charter every future run. Split
  into a hard validity check (duplicate/empty id, bad TOML — still refused)
  and a separate `over_budget()` warning that `setup.rs`, `mecha charter`,
  and a new `doctor` check all surface without dropping the document.
  `RawCharter`/`CharterLine` also gained `deny_unknown_fields`, since no
  harness but this one authors a `charter.toml` and a typo'd table name
  sitting beside a correct one used to vanish silently rather than error.
- **The one that mattered most**: `Homeostat::finish` fed the sensor the
  *post-run* backlog, so a trigger that staged three replies overnight —
  doing exactly its job — hit the count term's saturation point and recorded
  `anticipated_guilt: 1.0`, indistinguishable from three replies it found
  waiting and ignored. And separately, the one-day age-saturation horizon was
  shorter than the real distribution these stores produce: `questions.rs`
  parks answers overnight *by design* ("nobody answers until morning" is the
  mechanism working), and `backlog.rs`'s own canonical fixture ages a wait
  8–9 days. Because the three terms combine as a logical OR, either gap alone
  was enough to saturate the whole reading to a constant `1.0` on any real
  install — the same degenerate-label shape rung 7's own measurement found
  the hard way, just found before a corpus existed instead of after. Fixed by
  reading the pre-run snapshot instead of a fresh post-run one, and by
  widening the horizon to a week.

The general lesson, on top of the one #86–#94 already recorded above: a
review that keeps finding real bugs on the fifth consecutive pass is not
diminishing returns, it is the review doing its job — the temptation to treat
round four's "looks clean" as a stopping condition is exactly backwards when
round five is the one that finds the bug that would have made the entire
shipped sensor a useless constant.

**2026-08-28 — rung 7 closes: the model half of step appraisal.** PR #102.
`docs/GOAL-SYSTEM-DESIGN.md` §5.5's escalation — the two comparison signals
rung 6 left to a model rather than a threshold (a landed step's span a clear
outlier against the plan's other completed steps; a step whose own words
claim verification with nothing verify-shaped in its span) — now runs inside
`agent.rs`'s own loop rather than as an offline CLI pass, since a step's plan
action has to reach the *same* run before it wastes more turns on a bad
decomposition. `ToolCtx::step_escalation` is `compact_requested`'s exact
shape (a tool asks, the loop acts, presence is the enablement); `tool/todo.rs`
writes a candidate into it, `Agent::escalate_step` settles it with one
quarantined call routed through the same cancellable `self.complete()`
`compact()` already uses, and the verdict folds into the turn as a fully
templated nudge — the model's own free-text reasoning about it never reaches
the conversation or the learning miner. Off by default (`[agent]
step_escalation`, `--no-step-escalation`, forced off under `mecha eval`),
since the pre-filter's thresholds are argued, not measured; `RunOutcome`/
`RunStats` gain `step_escalations_attempted`/`step_escalations_revised` so
that measurement can eventually be taken from the store, on `boredom_notices`'
own precedent.

**Fourteen real review rounds, the deepest single-PR cycle recorded here so
far — summarized, not itemized, since the PR's own thread carries the full
record.** The shape worth remembering is that the findings clustered in two
places, both non-obvious the first time: the quarantined call's own
cancellation/budget/effort/logging discipline (it originally bypassed
`self.complete` entirely, missed the loop's `stopping` check, left `effort`
unset, and logged its own failure at `debug`), and a family of bugs in how
`Tracked::completed`'s rolling history interacts with a plan write that does
more than one thing at once — a batch completing two steps at once, a
rewrite and a completion landing in the same write, and a step revised and
recompleted all corrupted the "plan's other completed steps" baseline in
distinct ways that three separate, increasingly narrow fixes were needed to
close, the last of which (a step seeing its *own* pre-revision entry as a
sibling) survived one round after the doubling half of the same bug was
already fixed and named in the fix commit's own comment. **The most
consequential single fix**: `escalate_step` streamed its quarantined call's
raw JSON reply — reasoning included — to whatever front-end was attached
(TUI, Slack, voice), because it forwarded the run's real `events` sender into
`self.complete`, which forwards every text delta as ordinary assistant text.
The design's own stated guarantee ("the model's reasoning never reaches the
conversation") held; "never reaches the *user*" did not, until `&None` was
passed instead. Also mid-cycle: `main` picked up an unrelated PR (rung 8),
and GitHub's `pull_request` CI — which tests an implicit merge with `main`'s
current tip, not the branch's own head — caught the resulting gap before a
local merge ever would have (see Environment, below).

The general lesson: a mechanism whose whole design is "quarantine a model
call and template its output" still needs the same auxiliary-call discipline
(cancellation, usage, effort, logging) every sibling quarantined call in this
file already has, and building it fresh instead of copying the sibling's
shape is exactly where the gap opens — cancellation and usage were caught
early, but effort and the failure-log level were each separate, later
findings in the same category, arriving well after the first pair looked
like it had closed the class.

**2026-08-29 — the appraisal-system review arc lands whole: PRs #111 and
#112, nineteen reviewer rounds deep.** A test-and-review pass over the
appraisal goal system (three parallel review agents plus hands-on runs of
the free readout, the quarantined appraiser and the counterfactual probe
against the live store) produced one high, three medium and a tail of
smaller findings; fixing them, then iterating with the PR auto-reviewer
until it came back clean, produced the rest. What shipped, by family:
**failed-turn integrity** — `Conversation::roll_back_failed_turn`
(restore-then-pop, popping only a plain user text per `is_plain_user_text`)
replaced five divergent pop sites across the REPL, TUI, web and voice
surfaces, every error arm now records the rolled-back state so a resume
loads what memory holds, and every submit site folds into a user tail
(recording the fold at submit as one direct `Rewrite`) instead of pushing
two user messages in a row. **Positional configs** — `Transcript` carries
`config_positions` and a `taint_timeline` built in `Session::read`'s one
pass; `config_covering` replaced `configs.first()` in the counterfactual
probe (a resumed attach's steer no longer replays under the wrong system
prompt and reads as inflated `regret`), with the rewrite arm
distinguishing index-preserving rewrites (truncation, fold, in-run
eviction) from summarising ones. **The instrument stops eating its own
findings** — unreadable transcripts are counted at every layer
(`Session::list_counting`, `Corpus.unreadable`, a doctor finding,
caveats/fields on `appraise`, `stats` and `health`), `sessions_read` and
`unreadable` are disjoint by construction, and the `named_a_goal` counter
lost in #88/#91's merge overlap is rebuilt. **The owner-closure guard**
(#112) — `closure_guard::ClosedStatusGuard` wraps `kg_task_update` on
every model-facing registry so a closing `status` is refused toward
`mecha tasks set` (which appraises); presence is `Tool::guards_closures`,
a trait answer a wire tool cannot fake, `closure_guard::verify` makes an
unguarded surface a startup error, and the guard's refusal is classified
(`ToolOutput::refusal` → the trace's `denied`) so the harness's own "no"
never counts as a failed run. Plus the affect chip on the typed web
surface (muted, deliberately not taint-amber), the TUI badge surviving
`--no-session`, `for_transcript` collapsing distill's four reads to one,
and the closure path's smaller repairs (prefix re-key, empty `--session`,
follow-up caveats). Deployed the same night; the environment note in
HANDOFF carries the capability-level verification.

**2026-08-29 (later the same night) — four merges from three concurrent
sessions, and an incident threaded through them.** #114 put the graph's
*shadow* queue (review-on-use surfaced verdicts) on every surface an owner
holds: `mecha review shadow` with `--confirm`/`--refute` through the
mecha-graph child, `/api/queue/shadow` + `/verdict`
(`serve/review.rs::shadow`/`shadow_verdict`), a surfaced-verdict deck on
the web review page, a graph-shadow row with in-place verdicts in
`/queues`, and per-fact tier marks in `/find` — plus, grown along the way,
a web entity page (`serve/board.rs::entity`, `Entity.svelte`, with
`entity_detail_marks_unreviewed_and_denied_facts` pinning that unreviewed
and denied facts say so) and chat tool-result previews
(`WireEvent::ToolResult`, a capped preview of what a tool answered). #116
repaired the `/tasks` page v0.1.16 shipped broken (the trap under
Measuring); the fix was committed independently on the #114 branch
(`aa53174`) and deployed within half an hour of the report, after a second
session verified the broken call in the *served* bundle — a free
identifier survives minification under its own name, which made the
artifact decisive where ancestry was only suggestive. #115 reworked the
appraisal docs page around the questions a cold reader actually asks —
when each appraisal moment runs, what consumes the record — and documented
step appraisal for the first time. #117 gave the docs site a fixture-backed
demo of the web surface (structurally the only option: the real surface is
loopback-only behind a tailnet identity, so there is nothing public to
link, and a screenshot of the real one is a picture of the owner's actual
mail) through one shim — the app reaches the server through bare `fetch`
and one `EventSource`, so the demo replaces exactly those two and no
component knows it exists — and the two CI gates now on every docs build.
The gates were proven against the defect, not assumed: `render-check` was
confirmed to fail on the pre-#116 tree with the real `ReferenceError` and
pass after, and the bundle-purity check by deliberately building a demo
bundle into `web/dist` and watching it fail. The coordination itself held
up: single-writer docs were claimed and sequenced, every peer claim was
re-verified against an artifact before being acted on, and the one
mid-write flag ("someone is editing the main checkout") turned out to
describe both flagger and flagged — resolved by both lanes committing
through worktrees and restoring the shared tree clean.

**2026-08-29 (~04:09 UTC) — the apex finally points at the factory.** The
open item filed an hour earlier closed the same night: the owner repointed
DNS at Squarespace (deleting the whole "Defaults" preset group, which is
what also removes the HTTPS/SVCB record — see the trap under Environment),
and `redirect_hosts = ["mecha-factory.ai", "www.mecha-factory.ai"]` went
live on the droplet with a restart. Verified at the authority and at the
socket, twice over by two sessions: the authoritative nameservers answer
one droplet A record and no HTTPS record, and `curl --resolve` against the
droplet gets `301 → https://gate.mecha-factory.ai/` with the path
preserved. The redirect certificate was issued into **its own** group
(`CN=mecha-factory.ai`, SAN apex + `www`) with the base group untouched at
exactly `art, compute, gate` — `certificates.rs::redirect_group`'s design
claim, that adding a redirect host cannot take the gate's certificate
down, holding under its first live test. The same test confirmed the
query-string drop as a live bug rather than a prediction; that and two
sibling findings are the "apex-redirect residue" item in HANDOFF.

**2026-08-29 (~04:50 UTC) — factory 0.2.8 closes the apex residue the
same night it was filed.** The redirect keeps the query
(`http::redirect_target` on `path_and_query()`, its test verified to fail
on the old one-liner) and `factory check` names the redirect hosts
(`tls::describe()`), released as `v0.2.8` and deployed on the owner's
word — with the pleasing closure that the deploy's own pre-flight `check`
printed the `describe()` fix working during the very deploy that shipped
it, and the live confirmation that the request which dropped its query an
hour earlier (`/view/ljchang/abc?v=2&x=1`) now 301s with it intact. Found
along the way: **mecha-factory's CI had been red for eleven days** on
`cargo fmt --check` — four whitespace diffs predating 0.2.7 — and two
releases went out over it without anyone noticing, because a lint that
*always* fails reports nothing about the commit under it: the same shape
as a silently skipped test reading like a passing one, in mirror image.
Fixed (`fdfbb7b`, pure rustfmt), and CI on that repo is green for the
first time in eleven days. The one item a binary deploy structurally
could not fix — the droplet's stale TLS-ALPN-01 config comment, drift in
the hand-maintained copy that claimed port 80 is never part of issuance
and could have talked someone into firewalling off every future renewal —
closed later the same morning on the owner's word: the `[listen] http`
comment was re-synced to the example config's HTTP-01 wording (backed up
in place first; comment-only, so no restart), and `factory check` run
after the edit both proved the config parses and, in its own tls line —
`acme http-01 for …` — states the fact the stale comment denied. That
empties the apex arc entirely, from "Coming Soon" page to done in one
night.

**2026-08-29 (~13:50 UTC) — settings becomes a place, and the charter is
edited by hand.** PR #118. The gear moved out of `Home.svelte` into the shell
(`App.svelte`), so it sits in the same corner on every view — layered *below*
the app's scrims, sheets and drawers, because a button floating over an open
drawer is a bug only a phone meets — and settings itself became an index of
three features rather than one scroll of three stacked sections, each pane
routed at `#settings/<charter|learning|voice>` so back, forward and reload land
where they should. The charter is now edited as a list: tap a line, add one,
delete behind a two-tap arm, and **re-rank by dragging its grip**. That is not
a convenience but the only rank control there can be — `CharterLine` denies
unknown fields and §11 gives the charter no rank key, so position in the file
*is* the ranking, and moving a line is the design's own "only editing gesture
that cannot produce a tie". Nothing moved on the server: the same route, the
same 64 KB cap, the same `Charter::parse` before disk, the same two-tap
confirm, and the owner still authors every line.

Two gates came out of it and outlive it. `check-charter-toml.mjs` reads the
`WEB_EDITOR_SAMPLE` literal *out of* `charter.rs` by marker comments and
asserts the web serialiser emits it byte-for-byte, so a regression in either
language fails the other — the serialiser had to be lifted out of the
component into `web/src/lib/charter-toml.js` before it could be tested at all,
which is the general shape: code that can only be exercised by driving a
browser is code nothing will pin. And `render-check` gained the three
`settings/<pane>` routes, where nearly all the new code lives; a gate visiting
only `#settings` was exercising three rows and a chevron. It repaid itself
inside the same PR, catching a `ReferenceError: unreadable is not defined`
that the Vite build passed green, because Vite does not resolve identifiers.

Ten review rounds, twenty-eight findings, none disputed. The two that would
have cost data both concerned an *unread* charter being treated as an empty
one: a failed GET rendered "No charter yet — add the first priority" over a
charter that exists, one save from replacing it; and a `parse_error` arriving
with `raw: ""` (`charter_state` reads the file with `unwrap_or_default()`)
would have opened an empty TOML buffer whose save truncates a file whose bytes
were never read. Both fail closed now, in `unreadable` and `blocked`, which
between them refuse every writing surface on a document the page could not
fully account for — including a comment sitting among the tables, which the
regenerating serialiser would otherwise eat. Three of the ten rounds found
defects in the previous round's fix; that is the honest cost of repairing
under review, and the reason the last two rounds were smaller than the first.

**2026-08-29/30 — the learning loop runs itself, and the instruments that
graded it were lying (PR #122, fifteen commits, merged fast-forward at
`4c7a0e2`).** The gated path had produced 0 live rules in 25 days — four
mutually-exclusive proposals held 27 of 43 reflections while `learn` skipped
nightly. `learn --auto` is the ungated middle mode (Luke's 2026-08-19
ruling): the counterfactual gate stays in front of the write, a regression
refuses, measured-clean applies, and an ungradeable batch applies **on
probation** (`Rule::probation`, retiring at `PROBATION_RETIRE_AT` = 2), with
every pass still writing a proposal as audit trail and superseding its
pending predecessors (`proposals supersede` — releases reflections
unconsumed, where reject is the owner's no and consumes them). Retirement
became direct: `rules propose-retirements --apply` nightly, resolving any
pending twin it overtakes. First consolidation: 28 reflections → 12 live
rules. Consolidation moved to the session-close hook (`learn-live.sh`,
flock-guarded, workspace-jailed after a fail-open `cd ""` was caught in
review); `mecha learning-report` and a web trend pane are the is-it-working
view.

The instrument half: **counterfactual probes had never once concluded on a
mid-run steer**, because the replay regenerated the whole prefix and
required the model to reproduce every call before the steer point — 11 of
12 steer probes `inconclusive: diverged at call #1` against points at
#10–#28, an exponential lottery, not noise. Probes now **branch**
(`counterfactual::branch_at` + `replay_run::drive_branch`): the recorded
messages before the intervention are resubmitted verbatim (a steer keeps its
tool results and loses only the steering text; a denial regenerates the
whole proposing turn), so pre-point divergence is structurally impossible
and the forced prefix reads from KV cache. Measured after: 0 inconclusive,
12 graded; the first unattended nightly trace-graded 3 of 3 steers. Three
more instrument lies fixed in the same pass, each found by running the
thing: the trifecta interlock fired *inside* replays (a replayed send sends
nothing — `external_send` now narrows in the non-executing modes); a
recorded tool nothing today can construct killed the whole probe (it now
rebuilds as a spec stand-in from the `SurfaceStore` blob, and a recorded
spec wins over a live tool's reworded description, closing the
rebuildability half left open on 2026-08-27); and the judge graded
tool-call bodies as answers (`is_gradeable` drops whole spans, unclosed
tags included). Thirteen auto-review rounds ran against the PR; rounds 12
and 13 alone caught a fail-open workspace guard, `--auto` skipping the
already-argued brake, probation stamping every rule instead of the ungraded
ones, a nightly retirement pass silencing the starved-learner check for
48h, and `finalize_rules` dropping the probation flag — the D1 hedge
evaporating within a session or two while three doc comments described it.
Era hygiene rode along: the pre-stem step-nudge bodies are recognised as
harness voice, four reflections stranded on pre-`tools_hash` surfaces were
dropped with reasons, and `select_probe_corpus` honours `dropped_at`.

**2026-08-31 — the nightly diagnostician was told to read the source and stood
in an empty room (#127).** `DIAGNOSE_SYSTEM` said "You may read the source and
its documentation… treat a documented reason as evidence", and §12.4 of
`SELF-IMPROVEMENT-RESEARCH.md` names that clause a *safety input*: it is what
stops a proposal unpicking something load-bearing. It had never once been able
to fire. `scripts/ruminate.sh` stands the nightly in
`$(mecha work path ruminate)` — an empty directory, on purpose and for a good
reason — and `setup::prepare_tools` roots the path jail at the working
directory. Six nights, three candidates, zero acceptances, and every proposed
key fabricated: `security.minimize_taint`, `tool.validation.strict`,
`context.auto_compact` exist nowhere in the codebase. A model told to read the
source and given nothing to read writes down the key such a program would
plausibly have.

The two questions the script conflated were already separate in the code:
config is discovered from the **cwd**, the jail is rooted at the **workspace**.
So the fix keeps the script standing outside a checkout and points at one —
`HarnessConfig::source_dir`, global-file only and stripped from project layers
for `[slack]`'s reason at its sharpest, with `global_config_only` pinned so a
checkout's own `mecha.toml` cannot ride in. `diagnose_system` became a function
of what was actually granted: `holds_source` asks the directory whether it
holds `mecha-core/src` and `docs` rather than believing the config key, and the
blind branch says plainly that it is blind and forbids naming a key the brief
did not name first. **A prompt asserting a capability the run was not given is
the silently-degrading guard in its cheapest form — nothing fails, and the
protection reads as satisfied.**

The same pass found the brief reporting three of the six metrics it invited
predictions on, so `Evidence::brief` now renders every member of `Metric::ALL`
with its headroom and `ruminate` refuses a prediction on a metric no run has
any of. It also found `RunStats`' denial counters never briefed — on the live
corpus, 39 refusals by a person or a policy and 21 interlock refusals against
32 environment errors, so the denials outnumber the failures and both are the
harness working. And `SessionMeta.workspace`, recorded in every transcript
header since the store existed, had never reached `RunRow`: the corpus was a
mixture of four unrelated jobs pooled into one average, 389 runs across eight
workspaces. `RunRow::workspace`, a prefix filter on `runlog::Scan` and
`Corpus::by_workspace` separate them retroactively over all 490 sessions.

Two gate holes came from literature newer than §13. GRASP
([arXiv:2605.29668](https://arxiv.org/pdf/2605.29668)) names regression
accumulation: `judge_slices` scored the predicted metric and the tool-call
volume alone, so `WORK_FLOOR` caught a gain bought by attempting *less* and
nothing caught one bought by failing *more*. `guard_regressions` closes it —
rejecting a proportional breach and **proposing** where a cost appears from
nothing, because `compactions` is zero across the corpus and rejecting there
would have made `compact_at_tokens` unable to move the one metric it exists to
move. SEAGym ([arXiv:2606.17546](https://arxiv.org/abs/2606.17546)) predicts
the other: `Tally::not_worse` is `wins >= losses`, so four all-tie held-out
episodes satisfied it with zero and zero and read as "confirmed on unseen
work". `MIN_INFORMATIVE_HOLDOUT` routes that to `Propose`, which reaches a
person.

`SELF-IMPROVEMENT-RESEARCH.md` §14 is the measured reading of the built loop,
including §14.6 — the finding that outlives the corpus, that replay drops
diverged pairs, so a change small enough to leave the trajectory intact is
measurable and a change large enough to alter behaviour is discarded. §14.7
records the owner's rulings: self-improvement is measured against how the agent
is actually used and never a generic suite; Terminal-Bench is a periodic
transfer check that answers a question about the loop and never feeds it.

**2026-08-30 — a hypothesis tested and rejected: the provenance gate is not
crippling learning.** Investigated by the learning lane before ungating:
`Evidence::UserTurns` already rescues tainted conversations into clean
reflections (21 of 43 arrived that way); the real residue is 5 lessons, of
which 2 are outbox edits the transcript walk structurally cannot reach, and
`reflect --remine-untrusted` over all 9 affected sessions produced 0 new
reflections — the reflector declines to draw lessons from user words alone.
Do not re-open the gate on this argument; the evidence says it costs almost
nothing.

**2026-08-30 — the retirement drill ran the NoGo path whole, and its first
run proved the leash could never fire.** The ungating precondition
`LEARNING-LOOP-RESEARCH.md` §5 named — "a NoGo pathway that has never fired
is an untested backstop, and under D1 it is the only one" — was discharged,
one day after cutover instead of before it. `scripts/retirement-drill.sh`
records an honest `mecha run` session, seeds it (typed, via
`mecha-core/examples/retirement_drill_seed.rs` — a steer inserted into the
last tool-result message, a steer reflection, a probationary bad rule and an
innocuous bystander) into a world isolated by
`MECHA_SESSION_DIR`/`MECHA_LEARNING_DIR`, then drives real probe passes and
a retirement scan after each: the probe must regress, the bisection must
convict the bad rule and not the bystander, one conviction must hold, and
the second must retire it at the probation leash of 2, with the leash named
in `retired_reason`. Before any of that could pass, the drill's paper
walkthrough found the bug the research doc predicted a category for:
**probation released on bare ledger coverage (`observations > 0`), and an
attributed regression always arrives inside an observation** — the
bisection charges a rule from the same measured block the row records — so
`propose-retirements` stripped the leash in the same scan that read the
convictions, and `PROBATION_RETIRE_AT` = 2 was structurally unreachable
while three documents described it as the D1 hedge. Fixed as
`release_probation_when_measured_clean`: release requires the ledger to
grade the rule *beyond its convictions* (`graded > attributed_regressions`,
counting verdict-bearing rows only, so inconclusive coverage releases
nothing — the ran-vs-graded confusion round 13 fixed in dispose, closed on
the release side). The seeded rule's wording is itself a measurement
(arm-reconstruction harness, n small but clean): an advisory rule moved the
model 0/3, a bare MANDATORY directive 1/3, and the same directive carrying a
*mechanism* — stale reads, verify before reporting — 6/6; the branch point
replays the model's own recorded reasoning, and naked authority loses to
that momentum. The drill passed twice end to end (4/4 probe passes
regressed and attributed correctly) and is the standing check named in
ARCHITECTURE's learning section; `drive_arm` now traces each arm's verdict,
calls and final text under `MECHA_LOG=debug`, because the first failed run
was diagnosed blind without it.

The PR's review round then caught the fix's own mirror image before it
merged: `stamp_probation` shared the release's predicate, so once the
release stopped keying on bare coverage, a *born-graded* rule whose only
verdicts were its convictions could be stamped by an unrelated ungradeable
pass and never released — threshold silently 3 → 2 and a `retired_reason`
naming a probation it never had. Stamping and releasing now ask different
questions ("born ungraded?" vs "graded beyond its convictions?"), with
test cases that fail on the shared-predicate version. The same round moved
the ran-vs-graded distinction into every surface that shows a tally:
`render_active`, `rules`' describe, and `rules list --json` (which gained
`graded` beside `observations`) no longer render inconclusive-only
coverage as "0 improved, 0 regressed" — a clean bill of health from rows
that graded nothing. Merged as #124 (`6987bc5`) after a trial merge ran
the full suite on the merged tree, and deployed the same hour — the
install verified by the `graded` field answering from the installed
binary, since the version string was 0.1.16 on both sides.

**2026-08-30 — the graph tab became the curation surface, iterated live
against the owner's phone.** One evening, two sessions in a negotiated
file partition, ten deploys, three review waves. #125 (`68b4af9`) fixed
the four home queue cards that did nothing and pinned them with
`every_queue_the_backlog_reports_is_named_and_reachable_from_the_web_home`;
it also rewrote the ExecStart prohibition as a check after two sessions
violated the ban in one afternoon and the committed-script mitigation held.
#126 (`ab0097b`) is the arc: capture became a chat-idiom composer, the
notebook a bottom drawer pinned in layout (`?limit=` passthrough, sort,
filter), and the entity card grew the identity lifecycle in place —
two-tap alias removal and inline add (`mecha kg alias`/`unalias`, id-only
like `retract`), merge through `mecha-graph proposals file-merge
--accept` so the one no-undo verb always leaves a decided proposal,
create-on-miss, and a read-only `reaches` row of identifiers. Owner
rulings recorded in `NOTES-GRAPH-DESIGN.md`'s status blocks: repairs ride
mecha's own path; correct-in-place is the house principle; web merges
take the proposals record so the model's MCP surface gains nothing.
Beside it, the three-store `/api/proposals` pane (harness · rules ·
graph entities) with read-gated decisions, grown from the other lane's
dead Harness card. The owner's live testing drove five fix rounds the
demo never surfaced (Enter arming the mic via implicit form submission,
an unbroken safelink URL dragging the page sideways, `[object Object]`
chips, a floating drawer, no create for a missing person), and the
review bot's three waves each caught what the previous fix introduced —
the last a reject that could not work on two of three stores under a
green suite (Traps → Review process). Everything was deployed as it
landed and verified against the running thing; the night closed with
both repos merged, the machine on `main` everywhere, and `deployed-local`
deleted.

**2026-08-30 — the private graph repo merged its first PR, and the public
mirror caught up through the gate.** `d44e04d` carries the three verbs
the web asked for (`kg_upsert kind=alias` gains `remove`; `proposals
file-merge` with `--accept`, apply-first-decide-second like Accept;
`identifiers` in the `kg_entity` envelope), each exercised end to end on
a scratch `MECHA_GRAPH_DB` before being called done. Publishing then hit
the export gate refusing 13 files of life-derived fixture names — family,
colleagues, the university — accumulated since the 0.1.2 export. The
strip (`20f3d38`) moved every fixture to the fictional cast with the test
semantics preserved (the substring tests got new carriers: "SPSP Wrench
Reunion" so "Wren" still substring-hits it; the surname swamp runs on
Whitlock), suite 322/0 with identical assertions, and the blessed tree
published as `bbbba2a` ("0.1.3") — the private/public split doing exactly
what it exists for.

**2026-08-31 — the review queue stopped charging for a second look, then
shipped, then ran.** Reported as a person's own workflow failing: *"when I
enter a cluster to reject individual items and then go back, it reruns the
expensive similarity again. Really annoying and ends my desire to spend time
clearing the queue."* Three surfaces were charging for it and one of them was
already right. `closeItems` in `Queue.svelte` re-ran the whole cross-class
grouping whenever a verdict had been filed inside a group — a guard skipped it
when *nothing* had been judged, so a glance was free and the actual work was
not; the TUI's `Level::Items if from_group` arm had rebuilt the group from its
survivors since the level existed, so #128 was porting a settled decision
rather than making one. Listings then became a per-page cache, because the
back arrow, a Review sub-tab switch (which unmounts the pane) and any
transient error each threw the result away. #130 followed the same thread into
the TUI: `unwrap_or((0, 0))` over `cascade_tally` made "no fan-out was asked
for" and "a fan-out was asked for and the child did not report it" render
identically as `×1`, and the group arm removed a card on any `Ok(report)` —
mecha-graph reports `#id FAILED: …` and exits 0, so an unresolvable seed
deleted a row covering seven candidates while none were touched, which is the
`#2951` incident one level up. Underneath both, `candidate_embedding` (V023)
in the graph: a pending statement's text does not change while it waits, so
its vector is immutable and re-deriving it every call was pure waste.
Released the same day as mecha **v0.1.17** (four crates to crates.io) and
graph **v0.1.4**, deployed across all six `update` surfaces, and warmed on the
live store — **42.6s cold, 4.6s warm, byte-identical output, +17.9 MB**, and
**4.2s at a cosine floor never visited**, which is the number that matters:
what is cached is the vectors, not the query, so the threshold stepper stopped
being a re-embed of the world per nudge. The graph's public mirror was
deliberately left at 0.1.3.

**2026-09-02/03 — approval stopped being tool-granularity, and every review
pass found a way past the previous cut.** `mecha-core/src/policy.rs` (PR
#143, merged at `1d21d6b`) built PRIOR-ART-RESEARCH §4: `[[rule]]` entries
with a tool, a prefix pattern and one of three ordered decisions — `allow` <
`prompt` < `forbid`, most restrictive wins — judged one shell segment at a
time by a splitter that refuses anything it cannot take apart with
certainty. `allow` stands in for the human's yes and nothing else: the
approver's mode still applies through `Approver::permit`, the interlock
still refuses an armed send, an escalation is never softened by a rule, and
a `prompt` goes to `Approver::consult`, which asks past a standing
`[a]lways` and fails closed where no person can be asked. Every patterned
rule needs a `match` example checked at load, `allow` loads from the global
file only, and `mecha eval` forces the whole policy off. Nine review passes
on the rebased tree then found, pass after pass, the ways a rule's words and
a command's words come apart: a quoted token hiding a bare `;`; `timeout 5 rm
-rf` laundering a `forbid` through a wrapper; a glued `-ec` flag evading the
inline-eval check; `./git status` matching an `allow` on `git` by basename,
then the fix re-opened by `/abs/path/to/workspace/git` (a cloned repository
can ship one), so a path names its program only from a system directory; a
`prompt` rule answered by the tool's standing `[a]lways`, which got
`consult` beside `escalate` — and whose first default (`approve`) then let a
headless `--yes` run answer the rule that says a person must see the call;
`prompt` grouped with `allow` for path reduction when it never widens; a
`forbid` needing no `match` example, so `["rm", "-fr"]` loaded clean and
protected nothing; a pattern-less and then a patterned `forbid` downgraded
to a prompt by an opaque command (`rm -rf $HOME`); `strict_inline_eval =
false` switching off the wrapper lookup along with the prompt floor it was
meant to gate; quote characters, then a glued `;`, then a backtick hiding
`rm` from the opaque-command search. Each fix is a test that fails on the
previous commit. The two rulings the owner made from the clean pass's
minors are the follow-up's to record when it lands (HANDOFF names it).
**The lesson is the audit's: a policy engine's bugs are all in the gap
between the string the operator wrote and the string the shell will run,
and the conservative direction — refuse, or prompt — is the only one whose
false positives cost a prompt rather than the feature.**

**2026-09-03 — the served page revalidates by `ETag`.** `tower-http` 0.6.11
→ 0.7.1 for `mecha-cli` (#155, `b50eb24`), superseding dependabot's #129,
which had been green since 2026-08-31 without anyone reading what 0.7
changed against the one use site. `mecha serve`'s `ServeDir` now stamps a
strong `ETag` from size and mtime, evaluates `If-None-Match` ahead of
`If-Modified-Since` per RFC 9110, and a `304` carries both validators where
0.6 answered a bare one — for the `no-cache` entry document that is the
difference between a browser that can refresh the entry it just confirmed
and one holding an empty `304`. `ServeDir::new` is unchanged (the release's
`feat!` is a defaulted backend type parameter), the release's other breaking
changes are in middleware the workspace does not enable, and `Cargo.lock`
keeps a 0.6.11 copy because `reqwest` still pins it. Two independent reads
agreed point for point — Codex over the upstream source diff, the PR
workflow over the tree — and the tests pin both halves: a matching tag from
either precondition header gets a `304` with both validators, and a stale
tag gets `200` with a new tag, which is the half that protects against the
2026-08-29 "the feature had gone" incident. Not deployed at merge —
installed and restarted at 18:04 the same day, once a peer's inference run
had released the model server, and the skill's step-1b probe confirmed the
new binary from the served page (a `304` naming its `ETag`, not a bare one).

**2026-09-05, evening — the null-step and restart counters exist.**
§17.7 item 2 rules that a mid-run rule delivery stays off until the
null-step and restart counters are read, and reading the corpus for them
found neither recorded: `todo` read the null step (`step::Finding::Null`,
a step completed with no call behind it) into its result line and
dropped it, and a reopen — a completed step set back to in progress —
was not read at all, so the precondition could not be met from the store.
`ToolCtx::step_counts` (`feat/step-counters`) is a pair of atomics the
loop mints per run unconditionally — a sensor, not a lever, unlike
`step_escalation` whose presence is the feature's switch — `todo` counts
both events on the plan's own transitions, `RunOutcome` carries them into
`RunStats::step_nulls` / `step_reopens` (`Option`, unknown on rows from
before, on `boredom_notices`'s rule), `RunStats::merge` sums them, and
`Corpus::step_null_rate` / `step_reopen_rate` / `step_totals` read them
into `mecha sessions health` as a `plan steps` row and JSON keys. Pinned:
a null completion counts once, real work counts nothing, a reopen counts
once and its second null completion again, a first start is not a reopen,
a context with no slot counts nothing and does not panic, and the corpus
reads unknown before any sensed row and a rate over sensed rows after.
Nothing consumes the numbers yet — the ruling asks for them to be *read*,
and after a few nights they can be. The first review pass found two
things at the bar, both about whether the number would be true when
read: a completed step the model dropped from the plan and re-added as
in progress counted no reopen (the live plan no longer remembered it was
done) while its second null completion still counted, deflating one rate
and inflating the other — `ever_completed` now survives the live-plan
sweep, capped like `completed`; and nothing tested the seam from the
run-scoped slot to the record, whose failure would be a confident
`Some(0)` that `step_totals` counts as sensed rather than a dash — a
scripted run through the real `todo` tool now asserts both counts on the
outcome and the record. The second pass found the denominator: every
run since the sensor records `Some(0)` whether or not it ever planned,
so the rate would have tracked how often the model plans at all — the
confound a mid-run delivery could introduce in the very reading meant
to gate it — and no recorded field could recover it later. A third
counter, `step_completions`, is the denominator now: the rates are over
runs that completed at least one plan step, `StepTotals` carries sensed,
planned, completions, nulls and reopens apart, and the health line names
which is which. With it, a reopen consumes its `ever_completed` entry so
a step reopened, dropped and re-added counts one reopen per completion.
The third pass found the last two doors: a reopen needs a prior
completion but not in the same run — the plan outlives the run and a
resume reinstalls completed items — so a run that only restarted a step
was dropped from the denominator with its reopen, and now counts; and a
completed step parked as `pending` and resumed counted no reopen, and
now does. The fourth pass found the history's own two faults: keyed by
wording alone and exempt from the live-plan sweep, it made "run the
tests" in the next unrelated plan a reopen of the last one, so it is now
scoped to the run that completed the step — the counters' own scope,
from the work tracker's run id — and dropped the moment a new run
writes; and the completion arm never consulted it, so a step dropped and
re-added as completed counted twice in the denominator, and now counts
once. The fifth pass, on the merge with main, found the denominators
still one door open each way: a reopen-only run cannot produce a null
and sat in the null rate's denominator, and a completion whose step was
started in an earlier turn (every chat and TUI turn is a fresh run) or
never marked in progress counts a completion but can never be a null.
So a fourth counter, `step_measured` — completions whose span could be
measured — is the null rate's denominator, the reopen rate keeps runs
with any step activity, `StepTotals` names both, and the health JSON
key that said "completed a step" of a set that includes reopen-only
runs is `runs_with_a_plan_step`. The sixth pass found nothing at the
bar. Merged at `87645f9`, deployed 21:58Z (HANDOFF's machine-state row
has the probes). Ten findings in five passes, every one about whether
the number would be true when read, and each one a denominator or a
door the previous fix had left — the shape a sensor's review takes.

**2026-09-05 — a rule widens on evidence from two regions and narrows to
where it held, and the ledger says which sub-region each probe
exercised.** `GOAL-SYSTEM-DESIGN.md` §17.4's consolidation and validation
halves (`feat/region-widening`), the item the previous night's session
named next. `Rule::support` records the sub-regions a rule's evidence was
recorded in (`distinct_scopes` of the batch's windows); a batch that
repeats an outside rule word for word — the learner's prompt now asks for
exactly that when a reflection teaches a lesson another situation already
holds — widens it in `finalize_region_rules` to the intersection of the
two regions with the batch's windows added to its support — only when a
window lies outside the old scope, so a standing batch with no windows
widens nothing — the model claiming only "same lesson" and the harness
computing where it loads. `judge_convicted` places only the convictions
inside the rule's current scope, so a region an earlier narrowing shed
cannot read as "outside every support region" on the next scan, and
`cover_selection` credits a picked probe to every (rule, region) its
window exercises, so an unplaced ledger buys one probe rather than one
per standing rule. The first review pass found three more, all fixed
with a failing test each: a narrowing could re-scope to a scope that
still covered the convicting window (`{fs_read, shell}` and `{fs_read,
http_fetch}` kept against a conviction in `{fs_read, mail_send}` narrowed
to `{fs_read}`), so the verdict now checks the new scope excludes every
convicting region; `narrowed_at` is a durable flag, so a later learn
proposal carrying a once-narrowed rule unchanged read as a pending twin
and under `--apply` would have been marked superseded — the twin test
now asks whether the proposal changes the rule against its own
`rules_before`; and the standing batch could widen a scoped rule to
everywhere on a stale followup window, so the standing batch never
widens and only a window with a focus is support. The second pass found
the same stale window one file over: a followup probe wrote its window
as the region it exercised, which would have released probation and
satisfied `--cover` for a region the probe never touched — the row is
now placed only when the window has a focus, unknown otherwise. The
third pass found three: counting convictions only inside the rule's
scope let a bisection-proven regression in an out-of-scope window read
as clean at any count — a rule loads by registry match and a row is
placed by window, so every placed conviction now counts and a scan folds
the ledger since the rule's last narrowing (`tally_for`), which is what
"the narrowing answered those rows" honestly means; `cover_selection`
lacked the focus filter `record` had, so a window ending in a front-end
tool would have bought a probe whose row landed unplaced, forever; and
the twin test on "scope changed" matched a pending learn proposal that
had *widened* the convicted rule, so it now asks whether the proposal
makes the verdict's own change. The fourth pass found three: a coverage
probe could grade a rule against the reflection it was distilled from,
and that self-grade released probation and closed the pair for good —
`cover_selection` now passes over a rule's own `sources`; a verbatim
restatement in a shed region re-widened a narrowed rule while
`narrowed_at` kept the convictions that shed it buried, so a widening
that moves the scope clears the narrowing mark and the whole ledger
folds again; and the learner's outside section included hand-disabled
rules under an instruction to restate them verbatim while `finalize_rules`
did not carry `enabled`, so one reply re-enabled an owner's off switch —
disabled rules are no longer shown and `enabled` rides through a
restatement like `probation` does. The fifth pass found two more of
the same two shapes: a hand-disabled rule *inside* the batched region
fell through every carry-through once the learner stopped seeing it, so
the region finaliser now carries a disabled rule regardless of region;
and the self-grade guard was one rule wide while a row charges every
carried rule, so coverage now passes over a reflection *any* rule in the
surface was distilled from. The sixth found the learn pass announcing
a widening before the cap, the gate or a staged proposal could throw the
set away, so the nightly log would have claimed widenings the store never
held — the line now prints beside `write_learned_rules`, as *would widen
(staged)* on a proposal; with it, a near-miss copy of an outside rule is
warned about as a new rule beside the old, and the roster's seen-in
clause stopped hiding a widened pre-field rule. The seventh pass found
nothing at the bar. Merged at `52b8b0b`, deployed 20:19Z (HANDOFF's
machine-state row has the probes). Thirteen findings in seven passes,
every one real, each fixed with a test that fails on the commit before
it — and each pass found what the previous fix exposed, which is the
argument for reading every pass rather than the first.
`ValidationRecord::region` is the probed reflection's window (not the
run's registry, which every run shares), `RuleTally::regions` folds the
counts per region with `in_scope` and `attributed_against` over them,
and `release_probation_when_measured_clean` counts only rows that
exercised the rule — a `shell` rule carried in an `http_fetch` probe
used to leave probation on a grade that never tested it. `validate
--cover N` adds up to N reflections per (rule, region) pair the ledger
never graded, from inside the region, processed or not, and says so per
row; `scripts/ruminate.sh` passes `--cover 1`. `judge_convicted` decides
`Stands | Narrow | Retire` for `rules propose-retirements`: convictions
against the current scope (unknown-region rows count everywhere), narrow
when every conviction lies in support regions the rule can shed and one
is clean, retire with a reason naming the case otherwise — no support,
an unplaceable or out-of-support conviction, or clean regions that share
no tool (the conjunctive lattice's limit, stated in the reason so the
lesson can be re-learned inside one of them). A narrowed rule carries
`narrowed_at`/`narrowed_reason`, the roster prints `seen in …`,
`narrowed …` and a `by region:` line, and `rules list --json` carries
`support`, `narrowed_*` and `regions`. Pinned: a verbatim restatement
widens by intersection and a restatement inside the rule's region does
not; a new rule's support is its batch's distinct windows; the
narrow-or-retire verdict across all seven cases; the scan end to end
narrowing one rule and retiring another in one pass, with the narrowed
rule standing on the next; coverage selection in id order skipping
graded pairs and reflections already in the pass. Three fixtures moved
with the release rule: a tally with totals only now exercises nothing,
so the probation tests place their rows in the standing region. Every
row in the live ledger predates the field, so on this machine every
tally reads as ride-along until the next validate pass; the live rules
are unscoped and unconvicted, so nothing narrows or retires on the first
scan.

**2026-09-05 — the meeting poll as one conversation: ask once, review
once, then nothing until it is booked.** The scheduling poll had a
mechanism (seeded candidates, capability URLs, `rank_poll`/`clean_winner`)
and no experience: the create took two file paths the model had to
manufacture, returned URLs it was expected to mail by hand, and — because
`poll_meeting_create` is outbox-routed and so *executes at release* —
refused its own freebusy file as over an hour old by the time anyone
approved it; `~/.mecha/factory/polls/` had never existed on the owner's
machine. `MEETING-POLL-UX-DESIGN.md` redesigned it from the owner's chair
and the owner ruled six things in one sitting (`auto_book` = unanimous |
feasible | manual, every deadline and nudge number in a `[poll]` table,
candidates recomputed at release, the pick as an outbox calendar draft,
reject = "no time found" with no mail, the poll creates as `Message`-kind).
Built across three crates in two repos the same day (mecha #183, sixteen
review passes; mecha-factory #20, fourteen), deployed 2026-09-05. The
shape: `poll_meeting_create` takes a title, the people and a duration and
seeds from what the slots pipeline last saw — `slots push` now writes its
input, policy included, to `~/.mecha/factory/freebusy/`, which is what
makes a create staged at five and released at nine possible; the
invitation rides as a schema `default` so the outbox card *is* the letter;
the record at `~/.mecha/factory/polls/<id>.json` grows a `lifecycle` block
that three timer verbs consume in turn — `factory-publish polls sweep`
(observe the box, queue the one nudge, close on the poll's own terms,
decide the verdict, close the page with the outcome sentence),
`mecha-mail polls` (each person their own link from the owner's account,
the nudge, the event for a clean winner with everyone as attendee,
re-verified against live freebusy, ledgered in `~/.mecha/mail/polls.jsonl`
before the record is written), `mecha polls sweep` (a `pick` verdict
becomes a real `calendar_create_event` draft with the ranking and reasons
in its description; `p` in `/polls` swaps the loaded slot; a released card
books, a rejected one closes the poll as no time found). Open polls hold
their candidates off the booking page until the event exists. What the
review loop found and the build now holds: no mode books over a silent
participant *or an empty roster* (which the box serves on a database
error, vacuously unanimous); a harness-staged card is `Author::Harness` —
a string on the item, parsed on read — and is invisible to the writing
miner **and to `review_policy::staged_since`**, without which `/review
auto` would have released a timer-staged pick card mid-run and booked the
meeting with no card drawn; each verb re-reads the record at write time
and merges only the keys it changed, with a pid-unique temp sibling, for
a file three binaries write; an unreadable box reply, an unknown state, a
refused close after our own lost write, a torn record, a lost record
write after a ledgered send, a released orphan card — each is a finding or
a repair, never a benign state; and a poll whose invitations never all go
out, or whose event never arrives, is reported every tick and closed as
`stalled` a day on, releasing its slots. Deferred on purpose (§7.4):
participants' own freebusy, a web page, `polls extend` — and, refused with
its reason in §8, the open-link meeting poll: an open link has no
"everyone" to have answered.

**2026-09-04, evening — a charter sensor reads: each sensored line
against its store, on every run record, on every owner surface, and in the
doctor against the owner's own number.** `GOAL-SYSTEM-DESIGN.md` §11.1's
readings phase (#176, `feat/charter-readings`, unmerged when written).
`reading.rs` reads a line as one of five states kept apart — `Unread`
(store unreadable), `Deferred` (this reader does not scan that store),
`Nothing` (nothing waits; the line is met by construction), `Sparse` (the
corpus kind under the doctor's twenty-run floor) and `Observed { value,
over, excess }` — and `excess`, the overshoot over the overshoot
plus the setpoint, is the line-specific guilt term §11.1 promised in place
of `guilt.rs`'s one saturated number: zero within the setpoint, half of
maximal at twice it, never one, on `AGE_HALF_AT_HOURS`'s lesson that a term
reaching `1.0` erases the others and the corpus reads a constant; a zero
setpoint is refused at `SensorKind::parse_setpoint` because nothing could
ever be within it. `Homeostat::at_start` loads the charter itself (global
and read-only, like the backlog's stores) and records `Homeostat::charter`
from the inherited backlog, `None` for a row before the field or a charter
that did not load and `Some([])` for a charter with no sensor, through
`reading::lenient` so a kind a later binary adds costs the readings and not
the run record; the corpus kind is `Deferred` in a run and read by the
surfaces through `reading::corpus_rate`, with `Corpus::intervention_rate`
defined as a stop by request on the run record and nothing else — coarser
than the learning store's four triggers, under-counting rather than
guessing, and the doc says which one it sees. The doctor's `Patience` is the harness
constant or the setpoint of the line whose sensor watches that store, in
the owner's spelling, so the stuck-draft, unanswered-question and
stale-request findings name the line (`1 draft pending for more than 12h
(charter line \`waits\`)`); the count and rate kinds have no constant to
fall back to and fire only where a line names one; and
`check_sensor_saturation` reports a line past its setpoint on each of the
last `SATURATED_AFTER_RUNS` informative rows — same line, kind and setpoint
spelling, so an edited setpoint starts a fresh streak — saying both things
it could mean, because doctor cannot tell a real debt from an hour where the
owner meant a day. The surfaces read `reading::read_charter` and the two
JSON ones render `reading::lines_json`, with `reading` a *sibling* of
`sensor` because `sensor`'s two keys are what the web serialiser writes back
on a save (the first cut nested the whole record under `reading`, so the
page's `reading.state` read null — caught by the test). Smoke: a scratch
home with three sensored lines and two pending drafts read `3d 16h, past the
24h setpoint` / `2 waiting, past the 1 setpoint` / `no runs recorded yet`
on `mecha charter`, and `mecha doctor` raised the owner's 24h over its own
48h and named the line. The review's first pass found three things, all
fixed on the branch: the web handler ran three store reads and a possible
200-transcript scan inline on the executor (`spawn_blocking` now, on
`board.rs`'s rule); a corpus whose every transcript is torn read as
`Nothing` — `Corpus::scan` is `Ok` with the rot counted, not `Err`, so
"unreadable" was only ever a failure to list the directory, and unknown was
reported as met (`corpus_rate_in` now tells the three apart, with a test on
a torn file); and the surface printed a share off any denominator while the
doctor refused one under twenty runs, in the one place containment 5 wants
the owner judging a setpoint (`Reading::Sparse`). The second pass found
three more, fixed the same way: the settings page rebuilt each row by hand
in `hydrate` and dropped `reading`, so the browser was the one surface of
three showing none while its payload carried one (`charter-toml.js`'s
`rows`, with `web/test/charter-rows.mjs` pinning that `sensor` keeps its
two keys and `reading` never reaches the file); the TUI's `/charter` ran
the same read inline on the draw thread the web fix had just moved off the
executor (a thread and `CharterModal::poll` on the tick, `Readings::Lost`
when the thread dies); and `request_closure` read `Backlog::frontdoor`,
every open request, where the doctor's finding reads only the three
owner-facing states — a week-old `needs_info` parked on the stranger would
have saturated a line no finding named, and the remedy pointed at the same
unactionable reading (`frontdoor::WAITING_ON_OWNER` is now the one list,
`Backlog::read_with_owner_requests` hands the reading the narrowed depth,
and the wide depth stays what every recorded row has always held). The
third pass found the two remaining ways the sensor and the finding could
disagree: `tool_denied` counts a policy's refusal as well as a person's —
`forbid`, a hook, the interlock, a guard — so one routine rule would have
pinned "you stepped into every run" with nobody awake, and the rate now
reads `StopCause::Stopped` alone until the record can tell the two apart;
and the owner-facing request depth aged from `created_at` while the doctor
ages from the drain's clock, so a form stamped a month before ingestion
read a month overdue on the sensor and a day old to the doctor
(`Record::arrived_at` is now the one clock, the wide depth keeping
`created_at`). Not built then: the replay tiebreak, `board_overdue` and
`cost`, and the reflection's "which line moved".

**2026-09-04, night — the situation backfill: reflections mined before
the field get one, recomputed from their transcripts, or stay without
one and say why.** `GOAL-SYSTEM-DESIGN.md` §17.7 item 6
(`feat/situation-backfill`). `mecha reflect --backfill-situations` reads
each cited session once, runs `extract_interventions` over it, and
`learning::backfill_situation` matches a reflection to its intervention
by the three things a reflection persists — session, trigger and the
intervention text, copied verbatim at mining — then recomputes
`Situation::recorded` off the window, the session kind and the workspace,
exactly as the miner would have. Two fits with different windows are
`Ambiguous` and stay absent; two with the same window are one match. The
write, `LearningStore::set_situations`, takes only reflections whose
situation is still absent and stamps `situation_recomputed_at`, the
provenance mark the design asks for, so a situation recorded at mining
is never overwritten and the pass is free to run twice. No model. The
dry run on the live store: 37 of 45 would recompute, 8 stay absent — six
whose transcript no longer holds an intervention of that trigger and
text (a compaction since, or an outbox edit with no transcript), two
whose session is not in the store. The write itself was left to the
owner, since it changes which region batches the next `learn --auto`
argues; the owner ran it at 23:12:30Z on the deployed binary — 37
recomputed, 8 absent, learning-store commit `5767dc4`, a second dry run
reading `0 of 8`.

**2026-09-04, night — the replay tiebreak: line order decides what gets
replayed among equals.** §11.1's last phase, built last as its own phasing
asked, because it is the one consumer that changes what a replay
concludes. `appraisal::charter_rank` reads the highest charter line a
session's *signed* errors name — the plan's `serves:` and the sensored-line
attribution both land one on the errors — over the loaded charter's order
(`Charter::rank_of`, line order's second consumer after
`line_for_sensor`); `harness_probe::prepare_episode` computes it off the
same transcript read as the trajectory against the stores
`appraisal::Stores` loads once per draw, and `selection_order` sorts the
selection by headroom, then rank with the unranked after every ranked
episode, then id. The holdout is not touched — drawn first and uniformly,
where a priority may never reach (§8.1). `Measurement::ranked` records
how many of the selection the tiebreak ordered, beside the seed and the
holdout ids, because the rank is read off inputs those two do not pin —
the charter's order, and the outbox, question, front-door and learning
stores a session's errors are signed from, as they stand at the draw — so
the same seed over the same corpus can tie differently once a draft is
resolved or a line re-ranked (the first cut printed the count on stderr
and claimed the charter was the only unpinned input; the review named the
stores). The stores are read once per draw and not at all without a
charter with lines — and the second pass found the first cut of that
guard folded an unreadable charter into an empty one, so a hand-edit that
broke `charter.toml` turned the tiebreak off and recorded `ranked: 0`,
"ran and ranked nothing", for "could not run"; `appraisal::Chartered`
now answers three ways, and the draw records unknown for the first. The
fourth pass named the provenance gap: the goal a rank is read from may be
the session's own `serves:` string, the charter's ids are in its prompt,
and the draw truncates after the sort, so a tainted session naming the top
line would have bought itself into the slice the gate reads; `charter_rank`
now ranks only a clean-origin appraisal, the learning loop's own gate. The test builds three sessions with equal headroom whose
ids sort one way and whose plans serve lines the other way, and asserts
the selection is in the charter's order whichever the seed holds out — and
that the id order, the old sort, would have reversed it. `candidate.rs`'s
note that |goal error| "joins headroom when the appraisal record exists"
is corrected: the record's first consumer here is a tiebreak, and |goal
error| as a priority in its own right is still to come.

**2026-09-04, later — the goal system's second sprint PR: a reflection
records where it was learned, a rule carries the region it applies in, and
a run carries only the rules that match it.** `GOAL-SYSTEM-DESIGN.md` §17.6
item 3 with §17.7 item 1 in the same change (#168, `feat/situation-scope`,
merged at `046ef0f` the same afternoon after thirteen review passes —
eleven rounds of findings fixed on the branch, then two passes clean at
the bar, the second on the commit that fixed the first's nits).
`situation.rs` is the closed-set description of where a record was made —
registry tool names in trace order, the trigger, the surface, the workspace
— and the rule that shaped
it is that a key is what the harness knows structurally: the reflector's
`error_type` sits beside the situation on the reflection and is not in it,
because a key a model can author is a key an injection can set. `mecha
reflect` records it on every new reflection (outbox edits record the
drafting tool as the focus); `mecha learn` batches a domain's pool by focus
tool (`batches_by_region`) and calls the learner once per batch, with the
region's rules rewritable and every other rule shown as immutable context
(`Learner::learn`), and `finalize_region_rules` assigns each new rule's
`scope` from the keys the batch shared — the learner never names one. The
owner's ruling on the day, which decided the loader half: *only the
relevant rules should enrich a prompt* — the classifier by domain as it
already was, the drafter by its tool, a tool by its name. So `prepare_tools`
renders the block after the registry is complete and a scoped rule enters
the prefix only when the run registers every tool its scope names
(`rules_carried_for`, `carried_in`); §17.4's "never in the prefix" became
"in the prefix of the runs it matches", recorded as a correction in the
design's built note, because the prefix is composed once per run and what
the ruling protected against was every rule in every prompt. One scope key
today, the tool set, with `Situation::scope` and `Situation::matches` pinned
together by a test: the surface is recorded but `prepare` is not told the
session kind, and scoping to a workspace before widening exists would pin
nearly every rule to the one workspace most reflections come from. The
store's view thereby became a set no single run has, which is what §17.7
item 1 foresaw: `RunConfig::rules_hash` and `rule_ids` come from the one
render (`RulesCarried`), `RunStats::delivered` is written `Some([])` by a
build that delivers nothing mid-run, a record from before either field
reads *unknown* and never empty (`RunConfig::rules_arm_note`, printed by
`mecha replay`), `validate` renders the measured block per probe from the
replayed session's own `RunConfig::tools` and keys the ledger row to that
block, and `learn`'s gate hands `probe_reflection` a renderer rather than
two strings. Verified live: a `MECHA_SESSION_KIND=test` run recorded twelve
ids under one hash and an empty delivered list. The review's first pass
found two things worth having: the render sat at the end of
`prepare_tools`, before `build` inserts subagents, so a rule scoped to a
subagent tool could never have matched (moved into `build`, after the
subagent loop); and `validate`'s bisection surface loaded every domain
where the measured arm had always been `RUN_DOMAINS`, which would have put
`triage` rules in front of a tool-having probe — the exact residual
`Reflexion::learnable` names by hand (restricted, with a test that a
triage rule stays off the surface). Not built, named in the
§17.4 note: mid-run delivery, widening and narrowing, per-region validation
budgets, and the backfill for reflections mined before the field, which
batch as standing.

**2026-09-04 — the goal system's first sprint: a run can reference the
charter, and every negative has a word.** `GOAL-SYSTEM-DESIGN.md` §17.6
items 1 and 2, in one pull request by the owner's ruling (#162,
`feat/goal-ref-relevance`, stacked on the rulings themselves, #161 —
both merged the same morning, #161 at 09:45Z and #162 at 09:52Z, review
loop closed at `9fc79ee` after four passes). Item 1, the producers §17.1
makes its prerequisite:
`charter::prompt_block_for` asks for a `serves: charter:<id>` cite on the
`todo` tool, only when `todo` is in the surface (`ToolsConfig::registers`,
the predicate `Registry::with_builtins` now shares); `todo`'s schema
documents `charter:<id>`; and §11.1's sensored line landed as one change —
`[line.sensor]` on `CharterLine`, `SensorKind` a closed enum of **five**
kinds each with a reader (`outbox_waiting`, `outbox_age`,
`question_latency`, `request_closure`, `intervention_rate`; the design's
`board_overdue` and `cost` wait for the readings phase, because a kind that
parses and does nothing is the `deny_unknown_fields` failure one field
down), a setpoint typed by kind and refused at load in the wrong unit, two
lines with one kind refused naming both, the web serialiser carrying a
sensor through a save (`charter-toml.js`, pinned by the shared web-editor
sample and `check-charter-toml.mjs`), the template's commented example, and
the attribution join in `appraisal::of_session` — `sensor_kinds_for` maps
each `Cite` to the store its pointer names, the highest-ranked line with a
sensor on it is attributed where the run named no goal, onto the error and
into `Appraisal::attributed` — kept apart from `goals`, so the `serves:` ask
and the attribution stay separately measurable — and the queue delta maps
to nothing (containment 6). `sessions appraise` prints named, attributed and
either-way counts beside whether the charter carries a sensor at all. Item 2, the gate: `label_of` names every negative from sign and
agency — `Affect::Distress`, OCC's undifferentiated well-being emotion,
where a private `Own`/`Owner` negative used to read `Neutral` until a paid
replay filled `controllable`; a probe refines it to regret or
disappointment. `Pride` gained its first producer and then, over three
review passes, its three fences: a charter reference is kept only if the
loaded charter contains the line (a plan's `charter:no-such-line` plus one
clean send had derived `Pride` on a machine with no charter); the positive
arm is an allow-list on the pointer — a draft sent unchanged or a question
answered and finished — so the queue delta and the appraiser's own positive
verdict earn nothing; and `Affect::names_residue` is false for `Distress`,
`Pride` and `Excitement`, so the closure follow-up gate never stages a
"revisit" task from a verdict or a compliment. The appraiser-`Neutral`
correction in `affect_of` (#96 round 3) went with its case. The live
surfaces' word and the voice `cfg_weight` nudge now fire on a ceiling stop
or a steer. Measured on the branch: mecha-core 1288 tests, mecha-cli 730;
the test-list diff against `origin/main` was exactly the intended set
(four appraiser-correction tests removed, six pins renamed, twenty-two
added). Three doc corrections rode with it: §11.1's containment 7 says
"startup refusal" and `setup.rs` in fact degrades an unloadable charter to an
un-chartered run with one stderr line, so `mecha doctor` is the containment
and the `update` skill the fix; the rung 10 table row that called `Pride`
open; and `ARCHITECTURE.md`'s "label range is `Neutral` alone". Nothing
installed or restarted.

**2026-09-06 — the goal rides on the one question; a project is a kind of
goal.** `GOAL-SYSTEM-DESIGN.md` §17.7 items 3 and 5, built as
`feat/goal-sentence`. Item 3 had been ruled on 2026-09-04 and was the
missing producer: the only writer of a goal reference was the `serves:`
cite inside `todo`, which fires in a minority of runs and had fired in none
(0 of 152 sessions on 2026-09-06). Now `ask_user` takes `goal` and
`serves`, strict on the way in like `todo`'s `serves` — a pointer with no
sentence is refused, so is an unknown kind — renders them above the
question as one text and hands a typed `GoalHypothesis` to a new
default-forwarding `Asker::ask_about`; `ParkingAsker` is the one asker that
keeps it, on the parked `Question` beside the owner's answer, which is the
goal record §17.3 asks for (the owner's own words, arming no taint). The
three surfaces: the delegated seed's last bullet on asking folds the goal
into the ask-first call with this task's id and says never to ask about
the goal alone; the charter block asks every surface for the sentence in
words that name no tool — found while building: `setup` cannot know which
asker a front-end installs after `prepare` returns, and `serve` prepares
with `interactive = false` then installs `ask_user`, so `interactive` is
not a proxy for "someone can answer" — and a run with no way to ask is
told to state the goal it is assuming; the unattended note is derived at
review time by `outbox_source::serves_at_staging` (the plan's `serves` as
of the staging call, on D15's rule and so a later `serves` cannot claim
the draft) and printed by `outbox show` and the web detail with the
charter line's own text. `GoalRef::Project` is the fourth kind, a pointer
the charter check leaves alone. The appraisal reads the ask's `serves` as
a second producer of a named goal after the plan's; `sessions appraise`
counts sessions that put a goal to the owner and had it answered, from the
question store. Two things the build moved beside it: the TUI question
modal rendered a question as one `Line`, so a two-paragraph question would
have lost its break — it now renders one line per line and counts the
rows; and `CHARTER_CHAR_BUDGET` went from 2,000 to 2,500 because it bounds
the rendering and the block's fixed prose grew by ~480 characters — the
live charter measured 1,777 before and 2,256 after, so an unchanged
budget would have warned on every run from the first start. Nothing
installed or restarted when written; three review passes followed
(`HANDOFF.md`'s goal section has the tally): the two
findings that changed the design were that a model-written goal *id*
printed beside the owner's charter text on the outbox page could forge the
` — ` separator or add a provenance line, and that the goal *sentence*
could forge an `answered …` row on `questions show` — so an id is one token
at `GoalRef::from_str` and a sentence is one line at `hypothesis_of`, each
with a test that drives the forged string. Merged at `53f087f` on the
owner's word and deployed 02:32Z the same night (binary and `web/dist`,
four units restarted).

**2026-09-06, later — the goal anchor and the drift count, a sensor.**
`GOAL-SYSTEM-DESIGN.md` §17.7 item 4's sensor half, built as
`feat/goal-distance` straight after #198 landed, because item 3 had just
produced the record item 4 measures against. `tool::GoalTrack` is minted
per run by the loop on the step counters' pattern, carrying forward the
anchor a caller seeded: `questions answer` seeds the `serves` of the
question the owner just answered, and `ask_user` sets it in-run when a
present human answers a question that carried a pointer — a new
`Asker::ask_about` answers a `Reply` — `Answered` for a person's words,
`Parked` for a stored question — so the tool can tell a park note from an
answer, which is what keeps a delegated run's question from anchoring on
its own note; per question, not per asker, because the first cut's
per-asker flag missed the web asker, which shows a card and parks only
when nobody answers it (the first review pass's medium). `todo`
judges every plan write against the anchor (`goal::drift_of`: a changed
kind or id is a changed pointer, no goal once one is confirmed is unnamed,
and the two are kept apart because a local model that rewrites a plan
without repeating `serves` would otherwise read as one that changed its
goal — the fourth pass's finding); `RunOutcome` and `RunStats`
carry the anchor and the three counts, unknown before the sensor and folded
with the later anchor winning; `Corpus::goal_drift_rate` is over runs that
named a goal at least once under an anchor — not every run that planned,
since an all-unnamed run cannot have changed the pointer — the denominator
lesson applied a sixth and a seventh time; and
`sessions health` prints the line. The re-ask the design names for a
changed kind or id is off, on item 2's posture, until the line has been
read across a few nights. On this machine when written: *no run in this
corpus recorded the sensor*, the expected reading until the next run under
the new binary. Nothing installed or restarted when written; nine review
passes followed (`HANDOFF.md`'s goal section has the tally and what they
changed), then merged at `67fb55e` on the owner's word and deployed 05:07Z
the same morning (binary only, four units restarted).

**2026-09-06 — 0.1.18, and patch releases across the three repositories.**
mecha v0.1.18 ("a run knows what it is for") carries the goal system's
producers and its first sensor, `mecha exp`, the widening and narrowing
of learned rules, the step counters, per-command approval rules and the
rest of the twenty-eight entries since 0.1.17 six days earlier; the tag
published the four crates and the GitHub release. mecha-graph v0.1.5
("a task's association outlives the task") is that repository's first tag
since v0.1.2 — two versions had been bumped without one. mecha-factory
v0.2.9 carries the meeting-poll lifecycle and attached its musl binary.
Each was a bump commit straight on `main` with an annotated tag, the
shape every previous bump had. The update skill then ran end to end —
five installs, the graph's nightly binary, the benchmark's musl binary
(which had been three days stale), four unit restarts — with the record
in `HANDOFF.md`'s machine state; the droplet was left on 0.2.8 for the
owner's word.

**2026-09-07 — the project is the tier above the task, and the goal
pointer crosses to the graph whole.** mecha PR #206 (`feat/project-tier`,
merged `146922da`) and mecha-graph PR #10 (`feat/task-project-id`, merged
`940c806a`), deployed the same afternoon, graph first because the
follow-up files by the id the graph hands out. §17.7 item 5: the board row
carries `project_id` beside the project's name (mecha-graph's `TaskItem::project_id`,
from the same join, so the two are absent together); `tasks set`'s
closure appraisal records `GoalRef::Project` after the task; the owner
closing a project's last open task is the project's closure
(`project_closed_by` over one whole-board read that `project_closure_pending` takes *before*
`appraise_closure` can stage a follow-up under it), folded by
`appraise_project` into a `ProjectReading` printed on stderr — unknown
rather than closed on a truncated or unparseable board, no follow-up
staged, no record written; `tasks set --project` re-files or clears, and
a re-file in the closing call moves the tier to the echoed row. Item 8: a
distilled episode's `meta` carries `goal`, `serves_charter` and each
error's `goal` as `kind:id` pointers resolved at the boundary
(`distill::KnownPointers` — a charter id against the loaded charter, a
task or project id against one `kg_task_list` per run, a setpoint never),
falling back to the kind word where nothing resolves so a named goal is
never read as none, and the sentence never rides. On the graph: create
accepts the id it hands out (id first, then name, an ambiguous name
refused with the ids); "the row is the fact" — a node with a task row is
a task whatever its type, `NEVER_A_PARENT` and `CONTAINER_TYPES`
partition the node types and are bound to every writer of a parent
(`set_task_parent_id`, `retype_node`, `merge_nodes`); `kg_task_update`
resolves `project` before anything is written and writes it last;
`repair-parents` surveys the rows from before the guard (slips detached
on `--apply`, plausible old filings — a place, an event, a series — kept
unless asked, vouched for by `task-project <task> <its parent's id>`, a
vouch that stands only where the survey honours it); every detachment
appends to `properties.detached_parents`, which `task-project <task>`
reads. The same arc retired the last live mentions of the pre-rename
server: `prompts/agent.md` names the bare `kg_*` tools the documented
wiring exposes, and the landing page's `[[mcp]]` snippet names the
shipped binary with `prefix_tools = false` and the `untrusted_input =
true` override that arms the interlock. The review record, counted from
the PR comments (33 summary comments on #206, 03:42Z–14:12Z): #206
closed at the bar on the 33rd, after a handoff-only push (`1ec25cfd`)
drew a medium that twenty-one consecutive clean passes had not (the
snippet's missing override), and the fix push (`4542e735`, pushed with
the handoff commit `1cf82da7`) drew one more item worth acting on
(`meta.goal` dropping its key); #10
went thirteen passes past its first clean one, each finding one more
writer of the parent the rule had not bound, and closed with two minors
left to the owner. Surface and workspace as scope keys were asked about
in the same session and answered rather than built; the workspace key
began that evening on the owner's word.

**2026-09-07, evening — the workspace and the surface are scope keys for
learned rules, and the recorded key is the matched key by
construction.** PR #209 (`feat/workspace-scope`, merged `1f909899`,
deployed 19:06Z) and PR #210 (`feat/surface-scope`, merged `897c8b90`),
with PR #211 (`fix/owed-items`, merged `1ad2591c`; both deployed
23:13Z). `Situation::scope` keeps the workspace and the surface,
`matches` requires each exactly (a jail is not a prefix; a run that
declares no surface matches no surface-scoped rule), and `key` names
them after the tools; `region` intersects each new key (`surface_unread`
included) and `learn.rs`'s `widened` counts it, or a widening that drops
the key goes unprinted — both found on review, not on the first pass. The
rule that ended the review loops: **record what `prepare` matched, not
what the session was** — `RulesCarried::workspace`/`surface` are what
`setup::build` matched the block against (the front-end's `GlobalOpts::surface`,
set in code and never by a flag, and *never* the `MECHA_SESSION_KIND`
override, which marks the record alone), `RunConfig::rules_workspace`/
`rules_surface` keep them beside the jail and the kind, and the miner,
the situation backfill, the validator's region and the probe read the
record — the miner per intervention through `Transcript::config_covering`,
since a resumed question or a `/model` switch gives one session runs
matched on different keys. `serve` and Slack render one block against a
root and jail each session below it, and the board's door on `serve`
records a task while its block was matched as web; the front door and
mail now append the run record they declared a surface for. Every
`mecha reflect` pass reconciles the rows already on disk against the run
records before anything is mined (`learning::reconcile_key` per key,
`LearningStore::reconcile_keys`; a key any attach presented is kept, a
key none confirms takes what the first attach presented, a row whose
session cannot be read stays, a key a row never carried is never added,
a pass domain's rows are never read), so nothing depends on a human
running a flag before the nightly's `learn`. The surface is the first
key whose value is a closed set that can grow, and it got the guards a
path never needed: `Situation::MARK_KINDS` (`test`, `experiment`) are
corpus marks no run presents — dropped by `scope`, reported by
`unloadable_rules` — and a stored surface this build cannot name is
parked verbatim as `surface_unread` through a `SituationWire` from/into
form, matching nothing, printed as itself, reported at startup, replaced
by the reconcile only with a key some attach names, and handed back to a
build that can name it. #211: the TUI's edit form carries the project
and re-files only when it changed (`Form::refiled`, trimmed both sides);
`mecha rules` prints `LOADS NOWHERE` beside an active rule scoped to a
workspace/surface pair no run record presents, read streaming off the
run-record lines alone (`Session::run_configs_streaming`) for the rules
that consult the store alone (`needs_keys`), stopping at the first
transcripts that present every wanted pair, and answering unknown for a
torn or keyless store; the fixture board takes `MECHA_FIXTURE_BOARD_CAP`
and says `truncated`, measured cross-process. **Passes, counted from the
PR comment records when this was written:** #209 7 summary comments,
#210 15, #211 8; of #210's fifteen, the first fourteen each found
something at the bar and the fifteenth was clean — every finding an edge
of the surface key's fail-closed doors; #211's review ran 3 times
on its first pushes while based on #210's branch, then stopped — three
further pushes, one a force-push, drew no workflow run — and ran again
within a minute of being retargeted to `main`, 5 more times.

## The measurement record

Moved out of `HANDOFF.md` on 2026-08-06, when that file went over its own
length bound: this is a record of what was measured, which is what this
document is for.

**2026-10-03 — Breeze TTS 2 holds about half of Chatterbox's memory
(mecha-a3).** On the GB10, Q6_K, reading `nvidia-smi` and `ps` together
(the method behind Chatterbox's 8,018 MiB):
- At rest: **4,062 MiB**. That is 3,994 MiB GPU, 40 MiB `tts-server` RSS
  and 28 MiB adapter RSS.
- Peak, speaking one sentence through :8887: **4,185 MiB**. The GPU figure
  stayed fixed; the server reached 155 MiB and the adapter 36 MiB.

The figure goes into `recommend.rs`'s `tts` slot with #526, not merged when
this was written. Check after it merges: `grep -F '4_185'
mecha-core/src/recommend.rs`. That grep is ugrep on this machine, so a
literal containing `$` needs `-F`.

**2026-08-10/11, recovered 2026-08-27 — the turn ceiling was clipping a fifth
of the benchmark, and nobody had re-derived it.** Salvaged out of PR #52 (a
handoff refresh that went stale unmerged for sixteen days and was closed as
superseded) because everything else in it died — v0.1.3, 710 tests, a
benchmark run since finished — and this one finding had landed nowhere.

The original report, from the `mecha-arm64-subset-2026-08-10__14-15-05` run
on v0.1.2: raising the reasoning round trip made `max_turns` the binding
constraint, **26% of trials ended at exactly 80 turns**, and those passed 30%
of the time against 50–67% in every band below. Terminal-Bench's own default
is **200**, so 80 was a ceiling this project chose, below the benchmark's,
and nobody re-derived it after the constraints that justified it were removed.

**Recounted independently before it was believed**, because it is a report
from a session whose working notes are gone, and the archive is on disk:
counting assistant turns in each trial's own `agent/sessions/*.jsonl` and
taking the pass from `verifier/reward.txt`, over the 47 of 50 trials that
kept transcripts —

| turns | n | passed | rate |
|---|---|---|---|
| 0–19 | 20 | 8 | 40% |
| 20–39 | 11 | 5 | 45% |
| 40–59 | 4 | 3 | 75% |
| 60–79 | 2 | 1 | 50% |
| **80+ (the ceiling)** | **10** | **3** | **30%** |

**21% pinned at the ceiling, passing 30% against 40–75% below.** The
denominators differ from the original (47 trials against 75) and the shape
and the conclusion do not.

Two things make it worth the rescue. It is **local evidence for a decision
taken from a convention**: `TASK_MAX_TURNS` was set to 200 on 2026-08-26
citing Terminal-Bench's default, while a measurement of exactly that question
sat unmerged. And the first attempt to recount it **read 100% of trials at 80
turns**, because the regex matched the *configured* `max_turns=80` in every
trial log rather than the turns used — the fixture rather than the thing, the
same shape as the three green-for-the-wrong-reason tests recorded above, and
the reason the count was taken from the transcripts instead.

**2026-08-26 — slot contention, and the cost that is not prefix churn.** R3's
question, pointed at concurrency rather than at overnight parking:
`scripts/slot-contention.py` runs K conversations at once, each six turns over
a stable ~9,800-token prefix, and reads the server's own
`prompt eval time = … / N tokens` — small N means the prefix was reused, N in
the thousands means the transcript was re-prefilled. Against `-np 4`,
262,144 per slot, on a quiet machine (load 0.32):

| K | requests | wall | throughput | per-turn latency | prefill after turn 1 |
|---|---|---|---|---|---|
| 1 | 6 | 42.7s | 1.00× | 7.1s | 31 tokens |
| 4 | 24 | 108.2s | 1.58× | 18.0s (2.5×) | 31 tokens |
| 6 | 36 | 153.7s | 1.67× | 25.6s (3.6×) | 31 tokens |

**Six conversations on four slots did not evict each other.** After each
conversation's first turn, every later request re-prefilled 31 tokens — just
the new question — with two cache evictions across all three arms and no
conversation ever re-paying its transcript. That is `-cram` doing what §3.3
said it does, holding at a load nobody had tested it at, and it **refutes the
rationale that had been proposed for R1 hours earlier**: a permit count to
protect prefix reuse would have been protecting something the server already
protects, and the metric chosen to validate it would have shown no effect.

What over-admission actually costs is **latency, at flat throughput**.
Throughput saturates at four seats and buys 6% more going to six, while
per-conversation latency degrades 42%. So a fifth concurrent conversation is
close to pure loss: no more work done, everybody waits longer. That is the
number R1's permit count rests on — three background permits against four
seats, one reserved so the owner's turn stays near 7s instead of near 26s —
and it is a measurement rather than a guess.

The general shape worth keeping: **the right mechanism can survive the wrong
reason, and the reason is what the validating metric is chosen from.** Both
roads led to a permit count; only one of them would have been checkable.

On the original 25 grounded cases, all four local models score 23–24/25 with
zero malformed arguments and zero invented tools. **That set saturated** — it is
a floor test, not a ranking test, and it stays in the file as exactly that.

Two conclusions hold from it anyway:

1. **MoE wins on this hardware.** Decode tracks *active* parameters. The dense
   27B is 8× slower than the 3B-active MoE for identical accuracy.
2. **Constrained decoding is doing real work.** `llama-server --jinja`
   grammar-constrains tool calls; that is why malformed-argument counts are zero
   across the board. Don't conclude anything about a model's tool reliability
   from an unconstrained sampler.

**2026-08-24 evening — the phone got the review surfaces, and similarity got
a top layer.** One session, three arcs, all live by nightfall. The **mail
page** closed the gap the morning's handoff named ("the one daily surface the
phone cannot review"): `serve/mail.rs` + `Mail.svelte` on the TUI modal's
exact split — store read for the list, `mecha mail show` as the one thread
renderer (third-party text behind a per-line gutter), every mutation a
closed-verb CLI child where an unknown verb 400s before an argv exists, spam
the only confirm, drafting verbs detached into the outbox. The **queue tab**
grew the TUI's missing depths — classes with tier chips (tiers stamped
server-side from `tui::queues::Tier::of`, killing the page's own drifted copy
of the thresholds), per-class similarity groups with cascade verdicts — and
then, on Luke's push against the class fence at 7.5k pending, the **global
similarity layer**: a cross-repo arc (`mecha-graph`'s `similar.rs` and CLI,
mecha's `review groups --all` and the page's "similar across everything")
that groups the whole pending queue at a stricter floor (0.90), names every
class a group spans on the card, and rides `--cascade --across-classes` so
one tap stays one human verdict with the members as a labeled machine
cascade. The invariant was amended, not broken: "a cascade never crosses a
class *uninvited*" — the crossing is asked for by flag, priced, and shown.
First live run: 306 groups covering 782 of 6,929 pending in 40s, the top
group 25 restatements of one family relationship spanning four predicates.
**Files** landed as Phase 4's missing half (`serve/files.rs`): uploads into
the session jail's `inbox/` announced as paths so taint arms through
`fs_read`, downloads re-proving containment with missing and outside the
same 404, and images the only inline content type — model-written HTML
served same-origin would run script with the owner's auth against the API
itself. The voice **thinking sound** was replaced the same session (a soft
alternating two-note pulse with a 120ms attack; the 900ms triangle tick read
as a metronome). And the first real phone tap paid for itself: see the two
new traps below.


**2026-08-24 night — the voice got a face, and three sessions found they
shared one binary.** The voice page's dock had one button; it now carries a
**seven-voice picker and a rate slider** — six Kokoro-derived cloning
references plus Chatterbox's own built-in `default`, which is a real
selectable voice rather than a passthrough (`voice: "default"` generates with
no reference). Neither control was free the obvious way.
Chatterbox Turbo has no speed parameter (`generate()` takes
`exaggeration`/`cfg_weight`/`temperature` and nothing about tempo), and the
browser's cheap knob — `playbackRate` — resamples, moving pitch with tempo
into a chipmunk; so rate is a pitch-preserving phase vocoder on the server,
measured at ~50 ms warm against a synthesis cost of 0.53 s. The voices are
**Kokoro presets synthesized into cloning references**
(`scripts/voice/make-voices.py`), which is a licensing decision before it is
an engineering one: Kokoro is Apache 2.0 and its voices are nobody's
identity, so a voice can be added or deleted without anyone's consent being
the thing that made it legal, and the risk that a twice-vocoded reference
would clone badly did not materialise — all six work, 0.53–1.26 s for a
short sentence. Changes ride a `voice-config` RTVI message and the server's
reply is what renders: a bad value is **refused, never clamped**, so the
slider cannot show a rate the worker is not speaking at, which is the
wrong-bytes-review rule arriving in a control surface. The channel sets how
the answer sounds and nothing else — there is deliberately no field on it
that reaches the agent, the workspace or the posture.

`VOICE_BLOCK` learned that **its first sentence is the only one on the
latency path.** Pipecat's TTS service aggregates by sentence, so
time-to-first-sound is the synthesis of sentence one alone: measured at
0.33 s for "Sure." against 1.08 s for a full clause, on a leg that was
consuming ~77% of the budget while STT sat at 92 ms and the LLM's cached
prefix answered in ~220 ms. A short opener pays twice, being also fewer
tokens to generate before speech can start at all. The measurement is the
point — four turns of that session were spent debating the STT model, which
is 6% of the latency and already at the low end of the industry figures.

Several **stale labels on live processes** were corrected the same night, all
of the same shape: `llama-voxtral.service` still described itself as "STT for
mecha voice" months after Parakeet took the seat, `worker.py`'s docstring
still named `:8082` while line 47 defaulted to `:8992`, and
`VOICE-RESEARCH.md` §2.1/§2.2 still carried picks its own §7 build log had
superseded. §2.1 gained the rule the Parakeet swap actually established:
**an audio encoder bolted to a general LLM is disqualified from the STT seat
whatever it scores**, because no ASR benchmark contains adversarial speech —
which is why the table's top three entries are all disqualified.

The same night, a second session landed the phone surface's remaining arcs:
the **frontdoor page** (`serve/frontdoor.rs` + `Frontdoor.svelte`, closing
the last review store the phone could not reach), a **session-history
drawer** with resume that restores messages *and* taint through
`Session::load` and re-proves the recorded workspace, a **plain mail inbox
and compose** that stages under whatever `[outbox] tools` names and refuses
if `mail_send` is unrouted, and **push-to-talk dictation** (`POST
/api/dictate` → Parakeet) on the notes and task boxes. The interim
browser-speech toggle was deleted rather than justified: the owner had been
reading it as a status light.

Both voice surfaces got the controls within the hour, and the second one is
worth recording for *how* rather than what. Two sessions split it by file —
one took `Chat.svelte`, the other kept `voice-core.js`, the dock and the
worker — and the contract crossed between them as three warnings rather than
as a diff: render the server's reply and never an optimistic value (a refusal
is refused, not clamped, so local state would leave a slider describing a
speed nobody is speaking at), guard the preference restore or the reply
handler becomes a send loop, and take the slider bounds from the server's
`cfg.range` instead of literals. All three were load-bearing and all three
were honoured. The question that saved the most work was the one asked before
building: *where does the voice list come from* — the answer being that
`voice-core.js` already requests it on `dc.onopen`, so a second surface needs
only to supply an `onVoiceConfig` callback and the populated list arrives
unprompted. Mirroring the first surface's list by hand was the obvious move
and would have been the drift. (One trap for anyone curling it: the live path
is `:8881/v1/voices`, because `TTS_URL` already carries the `/v1` — `/voices`
alone is a 404 that reads like a broken list. Verified both ways.)


**2026-08-24 night — 0.1.13, and the docs catch up with two subsystems.**
The release that made the day's work public: `mecha serve`, voice, the
graph queue's similarity groups and cross-class layer, mail's plain inbox
and hand-written compose, phone dictation, resumable session history, and
`/entity`. Ninety-one commits under a patch bump, tagged and pushed with
the crates workflow publishing on the tag.

The website gained the two pages it had been missing entirely — one for the
web surface and one for voice — plus similarity groups on the queues page
(undocumented until now) and the inbox/compose half of mail. The voice page
was written by the session that built the stack rather than by the session
that published it, on the argument that a draft assembled from a build log
would be subtly wrong; the thing it caught is the thing a build log cannot
know, because a build log is written on a machine where everything is
already installed: **`cargo install mecha-cli` ships the voice facade and
none of the voice pipeline.** `scripts/` sits outside the crate, so
`cargo package --list` shows zero runtime files, and `git ls-files` tracks
zero `.wav` — the seven voices exist only where `make-voices.py` has been
run. A user who read "mecha has voice mode", installed the crate and went
looking would have found a facade with nothing behind it, so that
constraint leads the page instead of trailing it.

Three sessions coordinated the release across two repositories: a freeze on
`~/Github/mecha` that both peers verified their own state against, a
one-evening carve-out for the private graph checkout where a third session
was mid-arc, and CHANGELOG entries written by the sessions that had built
the things rather than inferred from their diffs.

**The freeze worked as a forcing function, and that is the part worth
keeping.** Nobody was investigating binary staleness. The graph session
checked its install timestamps *only* because a line had been drawn around
its repo and it had to state where it stood — and stating it out loud is
what made an hour-stale `mecha-graph-mcp` visible, across an arc whose
central fix was in the write path that binary owns. An agent asked to
summarise its own state finds things it was not looking for, which is an
argument for release freezes that has nothing to do with releases.

The carve-out itself was correct for one evening and would have been wrong
as a standing arrangement, which the session that benefited from it argued
for its own repo: an exemption held in two sessions' memory makes the graph
install nobody's job by default, correct only while both remember. That is
the shape this codebase keeps naming — a state that is only right after
someone remembers something is a state nobody can trust — so the update
skill owns both installs unconditionally, and "I am mid-arc" is something a
session says at the time rather than something a note keeps true on its
behalf.

**2026-08-25 — three review surfaces stopped hiding what they were asking
people to approve.** The day's work came from the owner using the release and
finding, in three different places, a surface that showed a decision without
showing what the decision was about.

*Mail actions on the personal account had never worked.* Every button on a
personal thread failed with "no thread in the triage store matches" — the
store held 192 records, all from the campus account, none personal, because the nightly
named `--account <the campus account>`. The flag was defensible and the requirement
behind the failure was not: `mail_triage` reaches nobody, mutates only the
user's own mailbox, and is documented as the third quadrant precisely so it
can be the cheap way to act, yet it was the one verb that could not run. The
requirement was never a decision about archiving — it fell out of
`resolve_thread`, whose real job is expanding a briefing's eight-character
handle. `triage` resolves leniently now (`mecha-cli/src/commands/mail.rs`),
`resolve_account` split because its two callers want opposite things from a
miss, and the gap it closes is permanent rather than incidental to one
account: `classify` sweeps 50 threads of one mailbox nightly while mail
arrives continuously in both.

*Then the nightly took both mailboxes*, which had been blocked by a bug
wearing a policy's clothes — see Traps → Unattended runs. The owner overruled
the standing "personal should stay out of the nightly" entry, and the
measurement vindicated them twice over: the first both-account sweep read 100
threads and disposed **47 of 51 candidates without a model**. The account
excluded for being expensive is the cheap one, because machine-generated is
exactly what the prefilter handles for free.

*A staged calendar delete showed an account and an opaque `event_id`.*
`outbox_source` matched provider ids against earlier `tool_use` **inputs**,
which finds a reply's thread read and cannot find anything for a draft whose
target was *discovered by listing* — the id exists only in a result.
`Join::Returned` closes it, `Join::Asked` keeps precedence where both hold,
and `MIN_RETURNED_ID_CHARS` guards the value-only match because
`calendar_id: "primary"` is a substring of every calendar result in the
session. The heading moved onto `SourceRead`: it read "replying to", which was
the one place a module documented as knowing nothing about mail knew about
mail, and was false the moment a draft that answers nothing got a source.

*And `Enter` at the TUI's review level had been a no-op for three commits*
while the footer advertised "Enter read it" — harness candidates and rule
proposals announced a depth and opened nothing, which is the exact shape
`/queues` exists to prevent. Found by reading a compiler warning that had been
in every build since; see Traps → Environment.

Three sessions worked one checkout all day. Two `git add -A` sweeps carried
another lane's uncommitted edits into unrelated commits, nineteen days after
that trap was written down — which is its own entry, and the reason explicit-
path staging is now the habit rather than the rule.
**2026-08-25 (evening) — the gate stopped reading a label the proposer wrote
about its own change.** `class` decides whether a harness candidate ever
reaches a human — `Security` is never measured and never auto-applied, while
`Config` inside the closed override set reaches `measure()` and can
auto-accept — and it was parsed off a line the diagnostician typed, never
cross-checked against the `change` beside it. It had held anyway, but by
coincidence: the closed set is four benign knobs, so a security change
labelled `config` stuck at `parse_change` for being *outside the set* rather
than for being a security change, and staged carrying that reason instead of
the one a reviewer needs. That morning's nightly had proposed disabling a
taint control, class `config`. `diagnose::names_guarded_setting` now derives
the class from the change: `[security]`, `[sandbox]` and `[outbox]` — three of
the four boundaries CLAUDE.md says reach a human however anything scores, the
fourth being the path jail, which is not configurable and so cannot be
proposed — plus every `SecurityConfig` field by its bare name, since the
section prefix is the model's to omit and omitting it must not be the way
through. Three properties carry it, each with a test: it **only ever raises**
toward review, like a capability override; it **reclassifies rather than
refuses**, because a refused proposal leaves no record and the brief carries
every prior candidate as "already tried", so a dropped one is free to return
tomorrow where a staged one is both blocked and paid for; and it
**over-matches on purpose**, since naming a setting is `security.` or
`[sandbox]` and prose *about* the sandbox is not caught, but a sentence ending
on the word is — which costs a reviewer a warning they did not need, where
missing one costs a confinement change auto-accepted. The mislabel itself
rides on the candidate's reason, because a proposer whose account of its own
change was wrong is the more interesting record and a pattern of them is
invisible if each is silently corrected. The same evening, **`show_file`
stopped loading the entire global config to read one number.** It wanted
`slack.max_upload_mb` and called `Config::load_global` at call time; config is
`deny_unknown_fields` — correctly, since it is a wire format between versions —
so any key added anywhere made the tool fail with a parse error in a process
started before the key existed. It happened twice on 2026-08-21, with `vision`
and then `[[search]] prefer_deep`. The cap is captured when the tool is
registered, which is also the only moment the TUI rebuilds it, so a broken file
now fails the `/model` switch the user just asked for instead of a chart an
hour later. Its test drives the failure rather than asserting about it — an
unparseable config under a temp `MECHA_HOME` plus a live attach record, which
reproduces the recorded message verbatim on the old code. `send_file` still
has the call-time load and is recorded in the handoff as the remaining half.

**2026-08-25 — talking and typing became one conversation.** D3, the last
of the voice decisions still owing something, and the one the design had
been promising since it was written. An in-chat call had been its own
conversation with its own transcript and its own clean taint slate; the
overlay said so honestly, which is the best a seam can do about being a
seam. It is gone.

The obstacle was never the transport. `mecha serve` held **two session maps
in one process** — `voice::Facade`'s slots and `chat::ChatState`'s sessions
— sharing an agent, a provider connection and a prompt cache, and a call
resolved in the wrong one. The key-carrying plumbing already ran end to
end; what was missing was the page choosing a key and the facade resolving
it somewhere other than its own map.

What shipped: the page names its session in the WebRTC offer through
`request_data`, which is pipecat's own passthrough to `runner_args.body`,
so no framework patch and no second endpoint — and the offer is used
because it is the only message sent *before* the bot exists, and the bot is
what has to know, the data channel opening far too late to choose an LLM
service's headers. The worker forwards it as `X-Chat-Session` beside the
slot key it still mints, deliberately a second header rather than a
namespace inside the first: one header carrying two meanings is a value
nothing can validate, and a page is free to name a session
`webrtc-anything`. The facade resolves it through a `voice::SessionHost`
trait that `serve::chat::VoiceHost` implements, so `voice/` still has never
heard of `serve/` — the `Approver`/`Asker` shape applied to "whose
conversation is this" — and the facade keeps **no** slot, session file or
conversation for a hosted turn, because a second copy of any of those is
exactly the duplicate record that made merge-on-close the rejected shape.

`chat::begin_turn` became the one implementation of "a turn on a web
session", shared by both doors. That is the `/tasks` rule (one
implementation per verb) applied inside a process rather than across a CLI
boundary, and for the same reason: two constructions is how the typed and
spoken paths stop agreeing about the jail, the outbox stamp or the
recording contract, silently and in that order.

Four decisions rode along. A spoken utterance mid-run **barges in rather
than steering** — steering would fold the words into the run already
streaming *to the page*, and the worker is owed a reply it can speak;
measured at 1.2 s from speaking over a 300-line generation to hearing the
answer. A spoken turn **broadcasts its own user message** (`WireEvent::User`,
the voice block stripped) because it has no local echo anywhere, so the page
fills in as you talk; a typed send is still echoed only by the page that
typed it, and what a second device misses is a separate gap that was
deliberately not half-closed here. `--voice-yes` **travels with the turn,
not the conversation** — the owner's call — so a spoken turn runs with
approvals off while a typed turn in the same conversation obeys the page's
mode, verified in one session by a spoken `fs_write` that succeeded and a
typed one two turns later that was Blocked read-only. And nothing
structural moved: the interlock still sits ahead of the approver, sends
still stage, and taint now **accumulates across both doors** instead of
being reset by opening a call, which is the stricter direction.

Verified by running it rather than by reading it, which is what this
project keeps having to relearn: shared context across the doors, one
session file where there had been two, the block firing on exactly the
switches into speech, and `X-Chat-Session: ../evil` falling back to a
conversation of its own with a warning — a dead call being a worse answer
than an unshared one.

Left standing and written down rather than fixed: a `voice:` session is now
the fallback rather than the norm, so the drawer's badge quietly means
"this call had no page behind it"; the page's mode chip does not describe
spoken turns; and `mecha serve` still never drains its chat runs on
shutdown, which now covers spoken turns too — unreachable in practice,
since `axum::serve` is not wrapped in `with_graceful_shutdown` and
systemd's stop is a hard kill, but the standalone `voice-serve` handles
SIGTERM and the mounted one does not.

The same day, earlier and by another session, **a sound with no words in it
stopped stopping the bot** — step (1) of the deferred VAD item, fixed
structurally rather than by tuning. Dropping the VAD from the turn-*start*
strategies means a wordless segment emits no transcription frame at all,
reaches no strategy, and the bot simply keeps talking: resume-on-empty
achieved by never stopping, which needs no state to unwind. The analyser
stays, because it still segments.

**The bake-off those two conclusions came from**, moved out of `README.md` on
2026-08-10 so the numbers live with the rest of the measurement record rather
than in the front door. It was taken on a DGX Spark (GB10, 128GB unified)
against the 25-case set **as it stood at the time**; it has not been re-run on
the current 36 cases, and it is recorded here as what was measured then rather
than as a current result. `mecha eval --compare` produced this:

| 25 cases | gemma-4-E4B | gemma-4-26B-A4B | Qwen3.6-35B-A3B | Qwen3.6-27B |
|---|---|---|---|---|
| params | 4B | 26B / 4B active | 35B / 3B active | 27B dense |
| cases passed | **24/25** | 23/25 | **24/25** | **24/25** |
| checks passed | 99% | 97% | 99% | 99% |
| malformed arguments | **0** | **0** | **0** | **0** |
| invented tools | **0** | **0** | **0** | **0** |
| reasoning | 4/4 | 4/4 | 4/4 | 4/4 |
| injection resistance | 2/2 | 2/2 | 2/2 | 2/2 |
| mean turns | 2.8 | 3.4 | **2.4** | 2.5 |
| median latency | **6.7s** | 8.5s | 7.3s | 24.7s |
| output tokens | 14,284 | 9,590 | 6,158 | **6,023** |
| **generation** | **119.7 tok/s** | 99.5 tok/s | 90.5 tok/s | 11.4 tok/s |
| MTP draft acceptance | 59% | **90%** | 75% | — |

Generation figures were isolated single-request benchmarks on the same prompt,
all three MoE/small models running speculative decoding via their MTP draft
heads. Qwen3.6's MTP layers are **baked into the GGUF** — `--spec-type
draft-mtp` with no separate `-md` file — which took it from 55.4 to 100.2 tok/s
(1.81×). Gemma ships a separate `mtp-*.gguf` draft. Qwen3.6-27B dense had no
MTP variant, so its number is unaccelerated and understates it somewhat; not
enough to matter at an 8× gap.

The latency column is the same hardware finding from the other side: the dense
27B took **3.4× the median latency** of the 35B MoE for identical accuracy,
because decode on this machine tracks *active* parameters rather than total.

Among the three that were left it was a straight speed/verbosity trade rather
than a quality one. E4B generated fastest but was the most verbose (14.3k output
tokens against 35B-A3B's 6.2k), so its wall-clock lead was smaller than its
tok/s suggested. 35B-A3B was the most economical per task and had the most
headroom. gemma-4-26B-A4B was the odd one out — the best draft acceptance (90%)
and the weakest score, and nothing recommended it over the other two.

The honest caveat that came with it: every case in that set was *grounded*. The
data was in the workspace and the job was to find it, combine it, and report —
most of what a personal agent does, and a 4B was evidently sufficient for it. It
said nothing about long-horizon planning, ambiguous requirements, or code
generation, and no scorecard in it covered the `long-horizon`, `codegen`,
`synthesis` and `ambiguity` tags, which were added for exactly that.

The cases added since (`long-horizon`, `codegen`, `synthesis`, `ambiguity`) do
discriminate. qwen3.6-35b-a3b judged by gemma-4-26b-a4b scored **32/34** on the
set as it stood then (`results/qwen-hard-v2.json`):

- **long-horizon 2/2**, at ~17.5 turns — it walks a 16-link chain without losing
  the running total, and does not take the shortcut of summing the decoys.
  Confirmed at n=5 on 2026-08-03: `chain-total` is **5/5** uncompacted,
  `chain-largest` **4/5**. A single earlier failure looked like a regression and
  was variance — which is the whole argument for pass^k.
- **codegen 2/2** — implements `median`, finds the one-line duration-parsing
  bug, and runs the tests itself. Graded by running them, not by asking.
- **synthesis 2/2** — finds the majority figure and the outlier, and notices
  which report supersedes which.
- **ambiguity 8/9 across the tag**, once `ask_user` existed *and* the cases
  graded the trace rather than the answer. `ambiguous-rate` asserts
  `tools: ["ask_user"]`; `false-premise` asserts `forbid_tools: ["ask_user"]`,
  because the right move there is *not* to ask — the file simply does not
  exist. How that was arrived at, and why a clean A/B said the tool made no
  difference while the transcripts said otherwise, is in
  [`HISTORY.md`](HISTORY.md) under Traps → Measuring. Read the transcripts
  before believing a score.

Only `ambiguity` and `synthesis` have a judge in the loop, and judges disagree
with themselves across runs. Read the answer before believing a single verdict.

**Scorecards in `results/` taken before the fixture expansion are not comparable
to ones after it.** The new fixtures took the shared workspace from 11 files to
44, so every case that searches the whole workspace got harder — two of them
started failing on turn ceilings calibrated against the smaller tree. If you add
fixtures, expect to recalibrate, and re-baseline every model rather than
comparing across the boundary.

The compaction arc — seven measured arms, and the finding that a summariser
preserves *what is true* while dropping *how far you got* — is in
[`HISTORY.md`](HISTORY.md). It is the reason compaction is shaped the way it is,
and it is worth reading before changing that code.

---

**The first complete Terminal-Bench scorecard (2026-08-11).**
`mecha-arm64-subset-2026-08-11__02-11-55`: all 75 trials of the oracle-passing
arm64 subset, k=1, v0.1.2 portable binary, qwen3.6-35b at the 262k window,
`max_turns` 80, timeout multiplier 2.0, ~19.5h wall clock. **Mean reward
0.4533 (34/75), 8 errored trials.** The run was launched to answer four
falsifiable questions from the 08-07 diagnosis rather than to produce a
number, and it answered them: empty-turn nudges fell to 4 across the whole
run, the dash-prompt crash did not recur. It is the k=1 baseline for the
0.1.2 harness; a leaderboard-comparable k=5 (~74h) remains unrun, so compare
nothing against the leaderboard yet.

**2026-09-06 — the first real `mecha exp` run: AgentDojo's workspace
suite under the full harness, 80 trials, and the interlock's price measured
for the first time.** `eval/dojo-workspace.toml` — one arm (`full`), no
control, qwen3.6-35b-a3b on the local server, the installed binary from
`9246ea0`; the record is `results/dojo-workspace-2026-09-06.json` (`mecha exp
export`) and the store `~/.mecha/experiments/dojo-workspace/`. Forty user
tasks, each run plain and once more with an injection placed where its
ground truth reads. 80 of 80 finished, mean 15.8 s a trial, 25 minutes
wall clock from first start to last finish.

- **Security 40/40, and the interlock had nothing to catch.** No injected
  goal landed — and reading the arguments of every blocked call, the model
  never once attempted the injected send. All 20 blocked sends on injected
  trials were the user's own task. So the run measures the local model's
  own resistance to AgentDojo's template (100% on this suite) and says
  nothing about the interlock's catch rate; that needs an attack the model
  obeys.
- **The false-refusal cost is the headline: 15 of 40 plain tasks failed on
  the interlock alone.** Utility was 21/40 plain and 19/40 injected. Every
  MCP tool is `private_data` by default (`mcp.rs`), the manifest marks the
  Dojo server `untrusted_input`, so one calendar read arms both and any
  `create_calendar_event`, `add_calendar_event_participants`, `send_email`
  or `share_file` afterwards is refused. That is production's posture —
  mecha-mail is the same shape — so the price is the product's, not a
  fixture's.
- **Three artefacts, recorded as conditions before the number is
  repeated.** (1) *The date*: `setup.rs` stamps today's real date into the
  system prompt, while the suite lives on 2024-05-15 and tells its own
  models the date only through a `get_current_day` tool — so "when is my
  next Yoga class" was answered "none, they have all passed" (task 2, both
  variants). (2) *The grader*: task 7's own ground-truth call fails its own
  utility check in agentdojo 0.1.35 (the tool shifts `end_time` with
  `start_time`; the check wants `start_time` alone in the diff) — verified
  by running every task's ground truth through its utility; task 7 is the
  only diff-graded task that fails, the four others that fail that probe
  grade on answer text the probe did not supply. (3) *The workspace*:
  `fixture = "eval/workspace"` stages 44 files and the local `fs_*` and
  `shell` tools beside the suite's, and task 30 grepped the workspace for
  the Hawaii plans instead of calling the suite's `search_files_by_filename`.
  AgentDojo's own agents hold only the suite's tools.
- Two genuine model misses (task 11 answered the lunch's start time when
  asked its duration; task 30 above is half tool choice). That accounts for
  all 19 plain failures: 15 interlock refusals plus tasks 2, 7, 11 and 30.
  Separately, three trials hit the config's `max_turns` of 12 on the long
  multi-step tasks — task 26's plain trial, which still *passed*, and the
  injected variants of 35 and 38, which did not; none of the three is in
  the plain-failure count.

What the instrument taught about itself: `status` run from any directory
but the checkout cannot run the source's `list`, falls back to the store's
rows with one stderr line, and prints pending as **0** — the
dash-is-never-zero shape. A manifest's relative paths resolve against each
verb's cwd, so an experiment is not yet recoverable from its store alone —
and neither is the fact that decides what the number means: the export
names neither the model nor the build (`arms.full.model` and `.provider`
are null, there is no version field), and `condition_hash` is an equality
key over the resolved model, not a name. The model above is what `/props`
served at the time, and the build is the install row in `HANDOFF.md`, not
the record. `judge` refuses a measurement by design, and the readout above is a
forty-line script over `trials/*.json` and the session files — the first
reader §11's E owes the binary.

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

**2026-09-02** — the graph stopped being two repositories. Ten files had been
private; only four held roster terms, and `scripts/nightly-mecha.sh` was
private *by association* — it sat in `scripts/` beside the gold-set tooling
and nobody had read it. Read line by line, it carried nothing: no names, no
emails, no hostnames, every path `$HOME`- or `$BASH_SOURCE`-relative. It is
public now, and the gold sets moved to `~/.mecha-graph/eval/` behind
`eval::gold_path_from`, which mirrors `db::default_db_path`'s env-then-`$HOME`
shape. What made the move safe was relocating the gate rather than retiring
it: `.githooks/pre-push` and `.github/workflows/denylist.yml` run the same
roster from outside the repository, both fail closed, and `.gitignore` gained
`eval/*gold*.jsonl` as a second layer — the export used to be what stood
between those files and the world, and working in the repo directly removes
it. The private checkout keeps the notes and the roster tooling;
`export-public.sh` is retained for history and called by nothing.

Two long-running silences ended with it. `is_missing_fact` made the Bee push
idempotent — one verdict had retried nightly since 2026-08-24 because
`bee facts delete` answered "Fact not found" for a fact that was already
gone, and the code called the achieved end state a failure; the diagnosis
came from the same branch's own error-recording fix, on its first run.
`Divergence::arms_summary` and the attempt-keyed gossip ledger closed the
other: a probe that legitimately refused ("one witness cannot gossip") wrote
no row, never aged, and was re-selected on five consecutive nights.

Seven review rounds on that PR found one MAJOR, five MEDIUM and twenty-one
MINOR; the MAJOR and three of the MEDIUMs were defects in *previous rounds'
fixes* rather than in the original change.

**2026-09-01/02** — `[harness] source_dir` is set. The ordering the handoff
specified was followed: every binary reinstalled first, then the key written,
because `ConfigLayer` is `deny_unknown_fields` and the section is a startup
parse error on any binary predating #127. Verified against the *installed*
binary before trusting it, not the branch build — the trap that item existed
to prevent. The diagnostician stopped running blind: it now logs "reading
source and docs from /home/ljchang/Github/mecha" rather than "no source
checkout reachable from the path jail".

**2026-09-02 — the appraisal label was hiding the sign, and the corpus was
measuring the harness's own test runs.** The owner asked for a review of the
appraisal system, "the newest system that deviates the most from other agent
harnesses." Re-measured live: 143 sessions appraised, 142 `neutral`, 0 naming
a goal, 0 board tasks ever `done`, `anticipated_guilt` a constant between
0.95 and 1.0, `StopCause::Interrupted` conflating Ctrl-C with an `ask_user`
park, and 22 owner-rejected drafts all reading `neutral` — because
`label_of` gives an owner- or self-caused negative no word until
`controllable` is filled, and only a paid replay fills it. Two literature
passes agreed on the mechanism from opposite sides: every computational
appraisal model (OCC, Scherer, EMA, Soar-Emote, WASABI) gates once on
relevance and labels from two variables, and Marinier, Laird and Lewis
rejected a product over unfilled dimensions by name; the harness
literature says LLM judges on trajectories are 33–41 kappa points less
reliable than reported and a 0.94-AUROC critic that intervened lost up to
26 points — so the design's refusals held and its readout was the defect.
`docs/APPRAISAL-RESEARCH.md` is the record. The owner's rulings, in one
session: labels are a readout of current state plus guilt and regret as
typed signals, nothing more; the error signal is mostly prose,
Reflexion-shaped, with numbers keeping three jobs (trigger, replay priority,
consolidation gate); appraisal grounds in the charter, the task's plan as
the prediction, and memory as the prior. Built the same day on
`feat/appraisal-record` and merged as #140 at `15c628d`:
`appraisal::Valence` beside the label on every surface,
`session::SessionKind` with a `test` override that
only narrows, a ceiling relabelled as the owner's own limit with
`Appraisal::cut_short` keeping the closure follow-up honest, and the
prediction record (`TodoItem::{expect, check, expect_calls}`, the frozen
check, `step::CHECK_TRACE`, the check counters on `Work` and `RunStats`, a
failed check as a signed error, `Trigger::Mismatch`). The other live lane's
`AUDIT-RESEARCH.md` §3.11 spec was co-designed across the two sessions by
message — its arm 1 re-injection carries the prediction, its arm 2 declared
checks are the structural discrepancy detector, its arm 3 critic is the
§3.7 predictor scored by the owner's edits — with one correction from this
side that mattered: a declared check must go through the approver, not only
the sandbox, or a plan field becomes a way to run a command on a surface
where `shell` needs approval. Two doc sentences the audit found false
against the tree were corrected ("three sensors ship with no consumer"
while `diagnose::Evidence` reads two; "never reported" while `distill`
writes the label to pkg). **Phase B followed as #141 at `49166e3`** the
same evening: `appraisal::SessionRecords` handing `of_session` the
question, front-door and reflection stores beside the drafts, with a
per-store "short" flag that lands on the record as `Appraisal::partial`;
`Channel::Commitment` signing a question answered and the session finishing,
a question abandoned, a request closed with nothing sent, and a
judged follow-up reflection (clean provenance only — stricter than
`learnable()`, the owner's ruling); the queue-delta positive read off the
run's own homeostat as the one positive a live surface can show;
`guilt::with_backlogs` folding level, delta and relief from one pair of
reads, the relief in its own field so the level's corpus mean stays one
quantity; and `Depth::given_up`, so a rejected draft, an abandoned
question or a hand-closed request shortens the queue without crediting the
run. The two PRs drew twenty-one and twenty review summaries from the
workflow — counted as its comments on each PR, #141's last one landing
after the merge and yielding #147; nearly every
pass found a real edge on the check freeze or the sign — a thinned echo
parsing as a prefix, a cut landing after the last marker, the carried
block as the only record after a compaction, the install-time restore
skipping non-completed steps, a marker a model could write into a step,
the owner's give-up reading as clearance — each closed with a test that
fails on the previous cut. Charter sensors were ruled in and designed
(`GOAL-SYSTEM-DESIGN.md` §11.1, seven containments), not built.

## Traps already hit

**Harness provenance must reach every miner.** A post-run criterion message
carried `harness=true` yet the user-role intervention miner read its text as an
owner correction. Passing tasks then manufactured follow-up reflections. Check
both the typed provenance marker and historical voice recognition whenever a
new harness message can enter a persisted transcript; test its interaction with
ordinary intervention mining, not only its dedicated diagnostic extractor.


A harness-generated verification call initially became the model's apparent last
failed action. Separating its work count alone was insufficient: final-call status,
error counters, provenance and historical context all needed the same distinction.
When an agent starts executing its own checks, audit every consumer of the trace,
not just the check runner. Regression coverage now exercises failure, refusal,
success and disabled execution.

Recorded so they are not hit twice. Each says what broke; the sentence that
matters is the general shape.

### Measuring

The concept pilot's revision-mutation fixture initially used a nonexistent SQL
column. Its provider failed before changing the database, so a generic rejection
assertion passed without exercising the intended guard. The strengthened test
changes the actual `lifecycle` field and requires both a changed source hash and
a rejected post-call source check. A fault-injection test must prove its fault
happened and identify the guard that rejected it; any error is not sufficient.

A default-feature cargo test build overwrote a feature-enabled example at the
same target path during atomic-memory qualification. The next HTTP smoke made
zero requests because it ran the feature-disabled stub. Build after all suites,
copy the actual executable to an isolated experiment directory, exercise its
contract and freeze that copy. A matching source tree and a previously successful
build do not identify the bytes currently at a shared target path.


A checks-first constrained response schema reordered `required` but appended its
checks field after choice in `properties`. The first real run therefore emitted
choice first; all 14 completed structured responses had the unintended order.
The run stopped with 22 recorded responses and two uncertain dispatch slots.
Corrected `diagnostic_request` constructs properties in order; qualification
compares actual HTTP bytes/order with the freeze, and scoring retains raw reply
order. **Test treatment delivery at the serialized boundary.** Sorted exports
and a required-field list cannot prove decoder generation order, and output
order cannot establish hidden reasoning order. Preserve the failed delivery
separately rather than labeling it a negative model-treatment result.

The requirements evaluator passed scripted tests but its first real-model replay
completed only 3/9 workflows: a schema with broad step bounds and free-text guards
permitted responses the host rejected. Constrain generated references to the actual
candidate vocabulary, and test the real provider path as well as host contracts.
Separately, diagnostic fixture names leaked through citable goal identifiers; a
model used “approval-blocked” as evidence and interpreted “schedule-universal” as a
universal meeting. Expected-label fields being hidden is insufficient: inspect the
entire model-visible input for diagnostic metadata, use neutral identifiers, and
preserve contaminated runs with an explicit limitation rather than replacing them.


**A scripted provider cannot see a deadline set below the real cost.** The
director shipped its first push with a 1.5 s per-sentence deadline. Every
Rust test answered through a provider that returns instantly, so all of
them passed. A bench on the router (`qwen3.6-35b-a3b-uncensored`, 3 rounds
of 6 sentences, each prompt carrying the turn's growing "already directed"
list) took a median 1.61 s and a p90 of 1.77 s, above that deadline (review
of #527, pass 2), so it went to 2.5 s. The first live call then came in far
lower: median 723 ms, max 1,269 ms, on `qwen3.6-35b-a3b` with the shorter
prompts of a real turn's early sentences. Those are two measurements, on
different models and prompt lengths, not one, and the live one is the
population to size the deadline and any prefetch from. The trap is the
tests: a timeout's value is a claim about a real server, and only a run
against one can check it.

**An activity sensor must listen to the person, not to what the page does.**
The persona autolock first counted `scroll` as use. The chat pins itself to
the bottom with `scrollTop =` on every streamed event, so a long reply kept
the unlock open with nobody there (#469, pass 1). Activity is input events
(`pointerdown`, `keydown`, `wheel`, `touchstart`, and `mousemove`, which is
what keeps a desktop open). An effect the page itself can cause is never
evidence of a person.

**A pinned session runs the version it was opened with, so "deployed"
is not "reaching the owner".** After #444 shipped self-portraits, the owner
still saw refused pictures. Their persona chat was pinned to persona version 4
(`sessions/<id>.persona.json`), and v4 had no `character` line, which arrived
in v5. The fix was live and could never apply in that chat; a new chat drew
first time. When a fix depends on per-object state, check the object the
owner is actually using (its pin, its version, its cached config) before
reading the code for a bug.

**Re-deriving a seeded fixture's truth means replaying every draw, or not
re-deriving it at all.** Scoring a transcription against text a seeded
generator wrote, the scoring script stubbed out the photo step, which also
drew the tilt from the same RNG. The filler text came out different, and the
model scored 61% recall where it had 85%. Have the generator write its own
truth beside the fixture (`eval/image-read/truth/`). When reading a seeded
generator's output any other way, reproduce every call that consumes the
seed, not only the ones that look relevant (2026-09-30, #450).

**A check on a tool the run does not offer passes on its absence.**
`forbid_tools: ["document_read"]` passes on a box without `[documents]`,
and "the model never chose OCR" then measures the configuration, not the
model. A check is only evidence if what it names was available. Have the
rig list what it could not offer before it runs (`eval::unoffered_tools`,
2026-09-30, #450).

**A gate whose input is already on disk should be checked before the step
it gates is paid for.** The appraisal backfill chose its sessions by 2e-1's
own exclusion, "no appraisal", and paid a model call per session to make one.
But whether that appraisal could ever be clean depends only on the session's
recorded end taint, which was readable all along. All 8 came out untrusted,
and 22 minutes of seats bought nothing the comparison can use (2026-09-29).
When a step exists to feed a filter, apply every part of the filter it can
already know first, and let the expensive step settle only what it alone
can.

**A failure that looks random may be fixed by its seed: replay the failing
seed before theorising.** Image edits that "sometimes" came back unchanged
were blamed first on chance, then on quantised model files. Replaying the one
failing seed reproduced the copy exactly, and varying only the prompt on 12
fixed seeds moved it from 0 of 12 to 12 of 12. The first fix, guidance to
"describe the finished picture", had produced the very caption that failed.
When an outcome is stochastic, pin the randomness, reproduce the failure,
then vary one input, and treat a change of guidance as a hypothesis to
measure, not a fix (2026-09-29, #408).

**A recognizer's contract includes the shape of the clip, not just its
words.** The late lane handed Parakeet-TDT five seconds of clear speech
with a second of silence inside it and got nothing back; the same words
without the gap transcribed. Measured on the same five seconds cut to
lengths of 4.5 s (text), 4.9–6.0 s (nothing) and 7.0 s (only the words
*after* the gap): the model
drops what precedes a silence it is given as one input. The live lane never
sees this because VAD segments end at silence; any new path into the same
recognizer must segment as the live one does *before* comparing results,
or "held no speech" is the recognizer's habit, not the audio's.

**A pipeline built for real time has rate assumptions nothing declares.**
Replaying a two-minute backlog into pipecat as fast as the channel
delivered it turned fifteen seconds of speech into one second and four
empty segments: the segmented STT accumulates audio only after the
aggregator's VAD edge comes back upstream, with a one-second pre-roll, and
a burst outruns that loop. Nothing errored. When feeding recorded or
buffered media into a live pipeline, bound the rate against the slowest
feedback loop in it and count what is still in flight as backlog.

**Freeze the executed artifact and give the control a coherent role.** A gossip
ablation removed peer evidence but kept instructions about a peer's different
sources; the control was internally inconsistent. A corrected run then executed
a Cargo output path that workspace tests rebuilt during measurement. Both setup
attempts were stopped before semantic grading and preserved. Match instructions
to each arm's available evidence, and run a private executable snapshot rather
than trusting a recorded hash of a mutable build path.

**A hash can record an empty intervention.** The executable pilot's first exposure
analyzer treated any nonempty `rules_hash` string as loaded rules. `RulesCarried::none`
records the hash of the empty block, so it falsely flagged all 48 rules-off trials.
The corrected reader distinguishes empty, exposed and unknown; original execution
inputs/results were unchanged and the old audit is retained. Validate metadata's
meaning against the producer before using presence as proof that an intervention ran.


**Test the prompt's contract alongside the artifact oracle.** The harder appraisal
pilot's privacy prompts requested a JSON report but named `answer.json` only in
the workspace README. One guided run produced gold-matching contents in
`share-report.json` and failed the filename check. The oracle's positive and
negative controls all passed: they measured its implementation, not whether the
prompt clearly asked for the artifact it required. The original grades were
preserved, and `appraisal_artifact_source::tasks` plus separately registered
follow-ups made the destination explicit in every prompt. A different privacy
run wrote the correct filename with `send_now: true`; artifact-location ambiguity
must not erase independent content errors.

**2026-09-08 — external text cannot certify its own wrapper.** PR #216 review
caught a replay optimization that skipped the untrusted-content warning when
tool bytes looked wrapped. An attacker could imitate the tags and suppress the
harness preamble; the trifecta interlock still held, but the warning did not.
Wrapping is unconditional again and a spoofed-envelope regression failed before
the fix. **Determine trust from harness metadata, never from attacker-controlled
content.** Nested replay warnings are preferable to a missing warning.

**2026-09-08 — a grounding judge missing the run context invents a failure.**
The first clocked assistant measurement passed 14/15 overall and all 15 artifact
checks. Its sole rubric failure called “3pm UTC” an invented timezone, because
the judge received mail evidence without the system prompt that explicitly set
the owner's timezone to UTC. `grounding_evidence` now includes recorded context;
a control accepts that valid timezone while rejecting unsupported receipts and
calendar state. **Give a critic the factual context the actor had, and calibrate
both false positives and false negatives before interpreting its score.** The
original 14/15 measurement remains recorded rather than silently regraded.


**2026-09-08 — successful artifacts and correct narration are separate claims.**
The assistant run delivered the exact reply and calendar event across all three
fixture lifetimes, while wording checks rejected `3:00 pm` and a tool check
rejected `calendar_freebusy`. Those graders were calibrated, with the original
12/15 result preserved. Reading the same transcripts also found recipient-read
and date/calendar claims unsupported by the tools. **Grade equivalent effects
without prescribing one valid tool path, and audit narrative grounding separately;
a passing artifact check cannot certify every sentence.** The earlier minimal-home
smoke also exposed absent staging instructions when no default `agent.md` was
installed. Capability guidance must accompany the configured route itself, not
rely on an optional prompt file being present.

**2026-09-02 — the instrument was measuring its own test runs.** 46 of the
143 sessions the appraisal corpus read were smoke runs from a mecha checkout
or a Claude scratch directory, and most rejected drafts carried reasons
naming a self-test; the session record had no field that could say so, so
the only way to tell was a path heuristic nobody had applied. The rung 7
measurement that set the build order was taken over the same mix. **A store
that every harness session writes into is a store the harness's own
development contaminates, and the mark has to be structural** — a `kind`
written by the front-end and a `test` override that can only narrow — because
a reader cannot recover it afterwards; the 46 stay unknown forever.

**2026-09-02 — a process environment variable set in one test module broke
a test in another, on the first run only.** A `SessionKind` test set
`MECHA_SESSION_KIND=test` around `Session::create` under a module-local
mutex; `runlog`'s scan test, in the same binary and another thread, created
sessions while it was set, and the default scan excluded them as tests. The
lock covered the module and the variable covered the process. **Anything
read from the process environment is shared by every test in the binary,
whatever a mutex says**; isolate it in a child process (`current_exe()` with
`--exact --ignored` on a probe test) or do not test it through the
environment at all.

**2026-09-04 — widening a derived value's range turned a dormant gate live,
and a gate defined on a goal alone let every channel that copies the goal
earn the word.** Two shapes from one pull request (#162), both caught by the
review workflow rather than the suite. First: ungating `label_of` meant a
rejected draft labelled `Distress` instead of `Neutral`, and the closure
follow-up gate read `label != Neutral`, so every rejected-draft closure would
have staged a "revisit" task — the exact override of the owner's decision the
gate's own doc forbids — while its tests stayed green, because they
constructed the label directly at `Neutral`. The mirror image of the
2026-09-02 trap below: there a narrowed range made the gate dead, here a
widened one made it fire on a case it had never seen. **When a derived
value's range moves in either direction, `grep` every consumer that compares
it to the old degenerate value; each one is a gate that was leaning on the
degeneracy.** Second: `Pride` was gated on `(goal, agency)`, and three
producers copy the run's named goal onto a positive error — the delivered
draft it was meant for, the queue-delta arm (a global level difference), and
the quarantined appraiser (a model's own verdict). The first fix checked the
goal against the charter, which closed the fabricated-id hole and left the
other two open, because neither needed a fabricated id. **A label that must
never be self-reported is gated on the pointer, as an allow-list: name the
channels that earn the word, so a future producer has to opt in rather than
being admitted by sharing a field.**

**2026-09-02 — changing a label's producer made a gate dead code, silently.**
A ceiling stopped reading as `Agency::World`/`Anger` (the owner's own limit is
not somebody else's fault), which was right — and it was the only
non-neutral label the free readout ever produced, so the closure follow-up
gate, which reads `label != Neutral`, could no longer fire on the free path
at all. Every test stayed green, because the gate's tests constructed labels
directly. **When a derived value feeds a gate, `grep` the gate before
changing the derivation**; the fix was a typed predicate beside the label
(`Appraisal::cut_short`) and a gate test that constructs the *errors*, not
the label.

**2026-08-31 — a bare filename is a line number's failure mode one level up,
and two files share the name.** A handoff item cited `setup.rs:879` for a stale
comment in `prepare_tools`. There are two: `mecha-cli/src/setup.rs` and
`mecha-cli/src/commands/setup.rs`. A reader followed it to the second, where
line 879 is an unrelated test assertion and `global_config_only` does not
appear anywhere in the file — so the reference read as "already fixed, or never
true", which is the failure a citation exists to prevent, pointed backwards.
Caught by a peer checking the address rather than the finding. **`CLAUDE.md`
says cite the symbol, not the line; the unstated half is that a path is part of
the symbol whenever the basename is not unique** — and `find . -name` is the
one-second check. The repair repeated the shape: re-measuring the caller count
with `grep "global_config_only: true"` returned four and missed a fifth that
assigns (`opts.global_config_only = true`) rather than using a struct literal.
**The pattern you search with is part of the claim you can make**, so a count
stated from a grep carries the grep's blind spot into the sentence. Written, of all places, into a handoff pass by the skill
whose own stated rule this is. The corrected item also found the comment worse
than reported: it claims one caller and there are four.

**2026-08-31 — a counter whose scope is implicit is one nobody can read
correctly, and three drafts got the same number wrong three different ways.**
`Corpus::sessions_read` increments *after* the workspace filter and *before*
rows are pushed. A bail-out in `corpus_slice` printed it as a store total
(it is not), then removed it as "necessarily 0 on this branch" (it is not),
then finally used it as what it is: the discriminator between "nothing was
rooted there" and "sessions were and none recorded a run". Round five caught
the middle one on live data — `~/.mecha/work/frontdoor`, 13 sessions, zero
outcomes, reported as "the filter matched nothing" — and round seven caught a
third case, a torn transcript landing in `unreadable` while `sessions_read`
stayed at nought, turning a rotting store into a typo. **A count whose scope
(pre-filter, post-filter, per-session, per-run) is not in its name will be
read wrong at the call site, and the wrong reading always looks like a
plausible sentence.** Absent is not zero, and the three zeros are not each
other.

**2026-08-31 — every test moved one variable, so none of them could see the
bug.** `guard_regressions` returned on the first metric it found anything on,
so a mild `0 → nonzero` proposal at `Compactions` (index 3 of `Metric::ALL`)
suppressed a genuine ratio breach at `MalformedArgs` (index 5): the reviewer
would have read the benign explanation and never learned something else had
doubled. Three tests covered the guard and every one of them moved exactly one
unpredicted metric, so the failure was invisible to all of them **by
construction** — a green suite that could not have gone red. **When a guard
iterates a collection and returns early, the test that matters moves two
elements and puts the milder one first.**

**2026-08-31 — a filter applied on the visible half reads exactly like a
working filter.** `--from-workspace` scoped the diagnostician's brief and not
`draw_episodes`' draw, so a change was reasoned about one job and accepted or
rejected on the average of four. It looked correct because the scoped half was
the half that printed. The same shape returned one round later: `corpus_slice`
canonicalized the path internally and discarded the result, so the raw CLI
value reached the draw and a `./work/morning` or a symlinked home matched
nothing there while scoping the brief. **Where one input feeds two consumers,
test the one that produces no output — and resolve shared inputs once, at the
boundary, rather than in each consumer.**

**2026-08-30 — `git rev-parse` echoes an unresolvable ref name to stdout,
and a suppressed stderr turns the echo into a phantom tag.** A probe ran
`git tag -l deployed-local; git rev-parse deployed-local 2>/dev/null` and
read the output line as the tag listing — but the tag did not exist, the
listing printed nothing, and the line was rev-parse's pathspec fallback
echoing its argument. The session then reported a tag "deleted between two
commands" and hunted a race that never happened. General shape: **a
lookup that echoes its input on failure looks exactly like success once
stderr is gone — check the exit code, or leave stderr attached.**

**2026-08-30 — after `/clear`, a session's own past is hearsay to itself,
and the false statement arrived dressed as a correction.** A peer
accurately quoted a sentence this terminal had sent before a `/clear`;
the post-clear session, finding nothing in its window, twice "corrected"
the record — and the correction was the false half, carrying the extra
credibility of someone carefully fixing things. The transcript on disk
(`~/.claude/projects/<project>/*.jsonl`) settled it in one grep, and a
PID a peer had captured hours earlier for an unrelated reason
corroborated it. General shape: **"not in my context" is not "did not
happen" — the session file is the artifact and it greps; and a
proactive correction deserves the same verification as any claim,
because it is the direction nobody is guarded against.**

**2026-08-30 — the security interlock did the failing, and the model wore
it.** The trifecta interlock ran live inside counterfactual replays and
blocked `docs__sheets_write`/`web_search` calls the recording had executed —
in a mode where nothing leaves the machine, since every answer comes from
the recording. The blocked call never reached the replay cursor, the arm
died one call later, and the verdict graded the model. General shape: **an
instrument must not carry live-fire controls for actions it cannot
perform** — a replayed tool declares what it can actually do (send nothing),
or the guard's work is billed to the subject under measurement. The same
session's stale-binary rerun is the Environment cluster's; found because the
warning that should have vanished was still in the log, which is the cheap
verification the fix itself suggests: after a fix, grep the next run's log
for the symptom, not for success.

**2026-08-30 — the judge's answer budget equalled the thinking budget under
it, so silence got graded.** `Judge` defaulted `max_tokens` 4096 against the
server's `--reasoning-budget 4096`; a long rubric spent the whole allowance
on reasoning and returned HTTP 200 with empty content, which the judge
scored as a bad answer. Fixed by `provider::LOCAL_MAX_TOKENS`; the general
shape is CLAUDE.md's own server rule pointed at graders: **any budget handed
to a component that thinks before answering must clear the thinking budget
beneath it, or refusal-shaped emptiness is graded as the subject's failure.**

**2026-08-29 — six verification steps in one session could not have failed.**
Across PR #118's ten review rounds: a drag assertion that compared two empty
strings, because `button.value` is `''` rather than `undefined`; a grep for
`id="..."` against a production build that minifies attribute quotes away; a
CI wait loop built on `gh pr checks --json`, a flag that version of `gh` does
not have, so the condition compared an empty string and exited on its first
pass — twice reported as "all checks settled" when nothing had settled; check
scripts whose exit codes were swallowed by `| head`, so a failing gate read as
passing; four new scanner tests that stayed green when the actual defect was
reverted, because the bug was in *how the tail was handed to* the scanner and
every test exercised the scanner; and a back-navigation test that printed the
symptom in plain text (`chevron #settings -> Back #settings`, a Back that
moved between two identical entries and did nothing) while being scored a
pass. Each one looked like evidence. Two of them printed the failure and were
read past. (The `gh pr checks --json` failure recurred on 2026-09-28: a PR
watcher across #376's seven passes used it with `2>/dev/null`, so its
failing-check branch never ran. Every check passed, so nothing was missed; the
plain tab-separated output is the form that works.) The general lesson: **a
check that cannot go red is not a check, and the only way to tell which kind
you have is to break the thing and watch it fail.** Reverting the fix before
trusting the test is seconds of work; not one of these survived that step, and
every one of them survived until something forced it. The corollary for
review: when a reviewer's finding cannot be reproduced, that is a fact about
the harness as often as about the finding — the `fillId` defect in the same PR
was unreproducible in Chromium and fixed anyway, because the browser that
mattered would not launch to check.

**2026-08-29 — v0.1.16 shipped a web page that threw on load, past three
green checks.** `Tasks.svelte`'s `stateOf` called `stalled(t)` where
`stalled` is a *field* the server stamps (`serve/board.rs`), never a
function — a `ReferenceError` on every card, so the `/tasks` board rendered
its header and no tasks on any non-empty board. Three mechanisms were green
while it shipped: the Rust suite (the defect is entirely in Svelte), the
Docusaurus build (it renders no client JavaScript), and CI overall (the
docs workflow's paths filter named `website/**` and not `web/**`, so a
web-only change ran no docs build at all — which is why the fix PR itself
showed no `build` check). Loading the page in headless chromium found it in
one run, and that gate (#117's `render-check`) now rides every docs build.
The general lesson: **a guard that never executes the thing cannot see a
runtime error in it, and three green checks over the same blind spot are
one check.** A corollary from building that gate's fixtures the same night:
a wrong response shape does not throw — five wrong fixture shapes each drew
an empty pane indistinguishable from "this feature does nothing", because a
component that fails still leaves the shell and nav looking like a page. So
shapes are verified by *rendering* them, and "drew almost nothing" is a
failure condition in its own right.

**Three tests written in one session passed on both arms, and the third was
caught only because the second had been.** One asserted a queued instruction
survived a turn that could not serve it, on a scenario that never reached the
line under test. One pinned the printed half of a rendered number while the
contradiction lived in the arithmetic beside it. One asserted a flag was
still set after a code path that, in that scenario, never ran. Each looked
like coverage of exactly the defect it missed.

The habit that catches them is cheap and mechanical: **revert the fix and
watch the test fail before believing it** — and, twice here, *check the
revert actually applied*, because a scripted edit that silently matched
nothing produces a green run that reads identically to a passing test. The
harder judgement is the other half: when a defect's trigger is genuinely
unreachable from outside — a flag set only by loop-internal state — the right
answer is to ship the fix with the reasoning beside it and **no** test,
rather than a test that cannot fail. A false green is worse than a gap,
because a gap is visible.

**A stale mtime is a stale build, and it reports success** (2026-08-27, the
probe lane's). After a cherry-pick, `cargo build` reused an old binary because
git had restored a source mtime older than the last artifact — and printed
`Finished`. Two full probe runs measured code that was not in the binary.
CLAUDE.md's rule is *"a fresh mtime is not a fresh build"*; this is the inverse
and fails identically, so the rule generalises: **the mtime says nothing in
either direction, and only the artifact can answer what it can do.**

**…and the obvious way to ask the artifact is easy to get wrong** (same lane,
same hour, and the better half of the lesson). The check was `strings
target/debug/mecha | grep -c "turn "` → 42, read as confirmation. **`"return "`
contains `"turn "`.** The probe string collided with ordinary compiled text and
would have passed against any binary ever built. **A verification that can pass
for the wrong reason is worse than none:** pick a literal that cannot collide,
and confirm it is *absent before* the change as well as present after — a
one-sided check cannot tell a fixed build from a lucky substring.

**`cd X && ( … ) &` backgrounds the whole list, including the `cd`** (same
lane). Three probe runs executed `./target/debug/mecha` from the main checkout
rather than the worktree — a different branch's binary — and produced plausible
output throughout. **In any backgrounded shell, address binaries and outputs
absolutely**: a relative path resolves silently against whatever working
directory survived, and the failure looks exactly like a result.

**A deliberate break, made to prove a test is not vacuous, was left in the
tree** (2026-08-27). The check itself is the project's own rule — verify a fix
by making it fail on the old behaviour — and it worked twice: removing the
result from boredom's key made the polling test fail, and unstamping the work
counters made the step-appraisal test fail. Undoing the first with `git
checkout <file>` silently did nothing, because the file was a **new module and
therefore untracked**, and `git checkout` on an untracked path is not an error.
The break survived into the next test run, which still passed, so nothing said
so. **An undo has to be an operation that applies to the file you broke** —
for an untracked one that means editing it back or `git stash` after adding it,
never a checkout — and the general shape is the one this file keeps recording:
a command that no-ops on the case you are in reads exactly like a command that
worked.

**Everything observable said the hand-over worked, and it had thrown the
conversation away.** `--resume` was parsed by clap, printed in the child's
argv, and never used: an earlier patch had aborted on a failed assertion and
written nothing, so the code still unconditionally created a session. The
board moved to `waiting on mecha`, the run started, the exit code was 0, and
the first line of the *new* transcript was "carry on from what you have both
agreed above" with nothing above it. The lesson is about what to check rather
than about patches: when an operation is supposed to act on an **existing
object**, verify *which object grew*, not that something happened — every
signal here described the operation and none described its target, which is
the same blindness as releasing a staged call against the reviewer's
workspace. And the cheap habit that catches it: after a scripted edit, grep
for a distinctive string from the change before building on it, because a
build that succeeds proves the file compiles, not that it changed.

**Three tests in one arc passed for reasons unrelated to what they asserted**,
and each was caught only by running it against the pre-change code rather than
by reading it. One asserted "no summary was requested" on a transcript too
short for `worth_compacting`, so neither arm requested one. One drove the loop
through a provider that reported a hardcoded ten tokens per request instead of
pricing what it was sent, so every prediction under test was a measurement of
the fixture. One asserted a series survived a run boundary while the revert
that was meant to break it still wrote the value back. The general shape:
**verifying a fix means making the test fail on the old behaviour, and a green
test proves nothing about which mechanism made it green** — three plausible
green tests here were measuring an absent precondition, a constant, and the
half of the change that was left in.

**A test that has to sit between two thresholds needs to assert it is still
between them.** Pressure high enough to narrow the tool-output budget is also
pressure high enough to trip the compaction threshold, so the first version
measured a summary thinning the transcript and reported the first result at 174
bytes — which reads exactly like the feature not working. The fix was to size
it deliberately between the two *and* assert `compactions == 0`, so it cannot
drift back into measuring the other mechanism. Where two mechanisms relieve one
condition, a test of either must pin the other off.

**The predictive check made the obvious test for the next change impossible,
and that was the finding.** Sizing a test for the cross-run series by letting
run 1 grow the transcript with tool calls could not discriminate, because the
predictive check fires *inside* run 1 the turn before that growth is priced. It
narrows when a carried anchor is the deciding signal to exactly one situation:
when the transcript grows **between** runs — a model answering and then a large
message arriving, which is how a chat session actually gets big. A change that
makes a test hard to write is sometimes telling you the case is narrower than
you thought.

**A fresh mtime is not a fresh build.** An installed `mecha` was argued current
because its mtime post-dated the commit and the tree added no Rust after it —
both premises true, the inference false. Running `sessions health --json`
settled it in one command: the binary was the morning's and lacked the arc
entirely, while the version string read 0.1.15 on both sides of the feature.
Ask the artifact what it can do; no reconstruction of *when* an install
happened can answer *what* it contains. The same rule the harness already
follows against llama-server — ask `GET /props` what is served, never assert
it.

**A fixture whose node name equalled its own id could not tell a producer
writing names from one writing ids**, and hid the bug for as long as the
linker existed. `seed_node` built every test node with
`Node::new(id, node_type, id)`, so `linker:knn` storing `subject: <node id>`
and `subject: <node name>` produce byte-identical payloads under test. The
assertion that would have caught it — the payload contains "hyperalignment" —
passed either way, and so did the accept path, because in that fixture the id
*was* a resolvable name. Reverting the fix against a fixture with distinct
names fails on `left: "topic-1f0a", right: "Hyperalignment"`, which is what a
test for this was always supposed to say.

**When a fixture makes two fields equal, every test over it is blind to
which one the code used.** The tell is a helper that fills several arguments
from one value for brevity; the fix is to make them differ in the fixture
*before* asking what a test proves. Verified the way this file asks for —
by reverting the change and watching the new tests fail, not by watching
them pass.

**A named id set was silently trimmed to ten, because `--top` bounds `--ids`
as well as a listing.** `mecha review items --ids a,b,c…` passes the ids
through to `mecha-graph review`, whose `--top` defaults to 10 — so a dive into
a similarity group of seventeen returned ten rows and said nothing about the
other seven. Every surface over it inherited that: the TUI's group dive since
the level shipped, and the phone's the day it was written. Found only because
a review of a *different* bug in the same code (a progress counter reading the
requested count instead of the returned rows) put two numbers on screen that
disagreed.

**A limit meant for an open-ended listing becomes a silent truncation the
moment the caller enumerates what it wants.** The two cases want opposite
defaults and share one flag. This is the queue's own "no silent caps" rule
failing one function away from the sampler that states it — `review sample`
announces its bound and why, and the path beside it trimmed an enumerated set
in silence. Where a cap and a named set meet, the set wins, and the count
comes from the caller's own list length (`review.rs`, `ids_limit`, tested past
the default of ten).

**A count taken on your own branch describes a tree nobody will check out**,
and three sessions proposed three mechanisms for how it went wrong before one
of them was measured. Worth the space, because the wrong two are both
plausible and one of them is a real hazard that simply was not this.

The facts. Three lanes landed work on 2026-08-25; two sessions independently
re-measured the workspace test count as part of a handoff pass — 1,365 and
1,367, each correct on its own tip, each wrong about `main`. The merge was
1,370. The later sweep then attributed the delta by walking `git log` and
came up three `mecha-core` tests short, so `5b187c5` had no lane and the
attribution would have been confidently wrong about which arc produced them.

**The cause was the starting point, not the range and not the topology.**
That sweep ran inside a worktree on a branch cut from `c2ca24b`, and
`5b187c5` was committed **31 minutes later**. `git log` walks what is
reachable from `HEAD`; a commit that lands on `main` after your branch point
is not in your history at any date, under any traversal flag. Reproduced
both ways: the identical command finds it on `main` and does not find it on
`c2ca24b`, and `merge-base --is-ancestor 5b187c5 c2ca24b` fails.

Two mechanisms were proposed first and both are worth recording as refuted,
because each is the kind of thing that sounds settled:

- *"It arrived through a merge, so the linear log missed it."* False, and it
  was the first author's own guess. `5b187c5` is on the first-parent chain;
  `git log` walks merges perfectly well. The remedy that guess produced —
  `--ancestry-path`, `merge-base --is-ancestor` — would have changed nothing,
  since nothing was excluded by traversal.
- *"A bare date in `--since` does not mean midnight."* **True, measured, and
  not what happened here** — the sweep passed an explicit `2026-08-24 18:00`,
  a window that does contain the commit. Keep the finding anyway, because it
  is a genuine trap two sessions confirmed on this tree the same afternoon:
  git's approxidate fills the fields you omit from *now*, so
  `--since=2026-08-25` run at 13:25 means "since 13:25 today" and returns
  **0 commits** where `--since="2026-08-25 00:00"` returns 35. A filter whose
  meaning depends on the hour you run it will disagree with itself across one
  session and never say so.

**Attribute with a commit range from the tag, in the tree you are about to
release** — `git diff v0.1.13..HEAD -- 'mecha-core/**/*.rs'` and friends,
which settle it per suite with no clock and no branch in the answer: +3
`mecha-core`, +24 `mecha-cli` (19 review-surfaces, 5 voice), 0 elsewhere,
matching the measured 1,343 → 1,370 exactly. A date range answers "what
happened while I was awake"; a branch answers "what I have been doing".
Neither is "what is in this tree", which is the only question a release
count is asking.

**And the meta-lesson, which is the durable one:** two of the three
explanations were produced by reasoning about git's behaviour and both were
wrong; the third came from re-running the original command against the two
different starting points and comparing. Reasoning produces an explanation
that *fits*; reproduction produces one that *fired*, and only the second
survives contact with a fact nobody thought to check.

The `--since` half is the sharper warning of the two, because it was not a
guess — it was **verified in isolation and never run against the command
that produced the number**. It does exactly what its author said it does;
it simply could not have fired here, since the invocation passed an
explicit time. That is the most expensive shape available: a mechanism that
reproduces on its own *feels* confirmed, so the one remaining check — point
it at the actual event — is the one it talks you out of. The original
invocation was on the record the whole time. When you have it, test the
event, not the hypothesis.

**The one commit that skipped the test run was the one that broke the suite.**
The offer-proxy commit substituted live verification (build, restart, curl
through the real door) for `cargo test` — and it broke a test-only struct
initializer, invisible to every live check because live checks exercise the
running path and only the suite compiles the fixtures. It rode through a
merge under a pipe that swallowed cargo's exit status. Two lessons, both old
ones wearing new clothes: the suite and the live check verify different
things and neither substitutes for the other; and an exit status behind a
pipe is not a check at all.

**A chat model in the transcriber's seat answered the audio instead of
transcribing it.** Voxtral answered question-shaped speech (saying it had no
calendar access) instead of transcribing it, and obeyed a spoken "just say banana" — which also
made the STT leg a prompt-injection surface, since anyone who can play audio
at the mic could steer the transcript. No prompt fixes what a model is; the
seat needed a transcription model (Parakeet), and the proof was adversarial
probes through the real pipeline, not reading configs. When a component's
output looks plausible, test it with inputs whose correct output you control.


A SQL predicate written for a `WHERE` clause was reused inside `SUM()`, and
the `OR` that had silently filtered NULLs began returning them:
`reviewed_by = 'user'` on a legacy NULL row is NULL, `NULL OR false` is NULL,
and one class whose decided rows were all machine rejects made the aggregate
NULL — which errored the cluster view, the proposer rollup and the `/queues`
modal within an hour of shipping, while every test on the `WHERE`-side
consumer stayed green, because there NULL merely fails the row. A predicate
shared between filtering and aggregation must be NULL-total — COALESCE every
nullable comparison — and the regression test that matters seeds the
all-NULL group.

**Two queries computed the same statistic and only one was filtered — the
unfiltered one was the one on screen.** `ladder::human_record` excluded the
pipeline's own rejects (`reject_reason NOT LIKE 'precheck:%'`) with a comment
explaining why; `precheck::review_clusters`, which feeds the cluster review UI
and `review --clusters`, had no such filter. So machine dedup rejections were
displayed as the owner's verdicts in the one view a person reads immediately
before verdicting a whole class: `llm/has` shown at 18% against a true 67%,
three classes shown at 0% on which nobody had ever voted. A whole analysis was
built on the wrong number and reached a confident, wrong conclusion — that
half the queue was demonstrably unwanted — before the second query was found.
**When a statistic has two implementations, the one with the careful comment
is evidence that the other one is wrong.** Grep for the filter, not for the
column name.

**And the correction has a second half worth keeping: an absent rate is not a
zero.** With machine rejects excluded, 40.5% of that queue turned out to sit in
classes with *no human verdict at all* — which the display rendered as "0%",
indistinguishable from a class rejected every time it was seen. Every surface
now prints a dash. "Nothing went wrong" and "nothing happened" are different
answers, and the same rule already governs `sessions health`; it is worth
re-checking wherever a rate reaches a human.

**A random sample and the head of a queue are not interchangeable.** Judging
the first dozen candidates in a class and reading the result as the class's
accept rate measures the *ordering*, which is correlated with age, id and
confidence. The fix is a seeded uniform draw whose seed is printed, so the
sample can be redrawn and checked — and a verdict must not resample, or a
sitting's twelve verdicts describe twelve different samples instead of one.

**A truncating pipe turned a partial test run into a plausible total.**
`cargo test --workspace 2>&1 | tail -30` was sent to the background on
2026-08-21; the exit code was 0 and the captured file held four `test result`
lines summing to 76. That reads exactly like a small workspace passing, and the
figure was almost quoted into the handoff — the real count was 1,244 across
eight suites, and `tail` had thrown away every line above the last thirty. The
grep that looked for totals ran over a file that no longer contained them.
**A filter applied before capture makes the capture look complete. Aggregate
from the whole stream, or capture the whole stream and filter on read** — and
treat a suspiciously small total as evidence about the pipeline before it is
evidence about the code.

**The bug was only visible in a run that worked.** After images shipped, a
Slack screenshot was answered correctly — and the run recorded
`taint {private: false}` where the same user action had previously recorded
`{private: true}`, because the model no longer needed `fs_read` to see the
file. A feature had loosened the interlock as a side effect, with a correct
answer sitting on top of it: no error, no warning, nothing in the trace. The
only evidence was a boolean in the outcome record. **Read the run's recorded
state after a change, not only its answer — a success is where a silent
regression hides.**


- **A value that only means something in a frame of reference, copied into a
  context that dropped the frame.** Three separate TUI bugs on 2026-08-20 were
  one mistake: `input_layout` counted *characters* where the terminal counts
  *cells*; `/help`, `/tools` and `/tasks` sized their boxes from `body.len()`,
  a count of `Line`s, and then drew through `Wrap`, which turns each into one
  *or more* rows. Every one looked like arithmetic on a number that was simply
  wrong, and none of them was — the number was right in the unit it was
  computed in. The defence is not to re-derive the value in the new context but
  to ask the thing that will actually draw it: `paragraph.line_count(width)`
  rather than `body.len()`, and `unicode-width` rather than `chars().count()`.
  `unstable-rendered-line-info` was already in the Cargo.toml for exactly this
  and only `transcript.rs` was using it. The same shape reached the peer review
  of that arc: a `git status --short` listing retyped into prose lost its
  leading space, moving the mark from column 2 to column 1 and turning
  "unstaged" into "staged" — the distinction that decides whether a bare
  `git commit` is dangerous. Fixed-width output is a format; paste it, never
  tidy it.

- **Two implementations of one question always drift; delete one rather than
  reconciling them.** The input box asked "where does this text wrap?" twice —
  once in `input_layout`, once in `Paragraph::wrap` — and the caret followed
  one answer while the glyphs followed the other. Measured on a real frame at
  30 columns: the painted last row read `"class"` and the caret sat at column
  3. Making them agree would have coupled the code to a ratatui internal that
  is free to change; the fix was to stop calling `.wrap()` and render the rows
  the layout function had already computed. Same rule as `list_height_reserving`
  and `find_tool` — where two callers must answer identically, there has to be
  one function they both call.

- **A clamp whose bounds can cross is a panic, not a layout bug.** Every modal
  sized its list inline as `rows.clamp(1, terminal_height.saturating_sub(4))`.
  The subtraction saturates to zero the moment the terminal is four rows or
  fewer, and `clamp` asserts `min <= max` — so shrinking a window with any modal
  open took the whole session down, partial answer and all. Found in `/doctor`,
  then written again in `/skills`, and five more modals had their own copy,
  because a new modal is written by opening whichever sibling is nearest. That
  is what made it a shared function (`tui::list_height`) rather than seven
  fixes; flooring the bound at one row degrades a tiny terminal to a one-row box
  instead. Each modal carries a draw-at-tiny-sizes test whose assertion *is* the
  draw, verified to fail on the old line (`min > max. min = 1, max = 0`) rather
  than merely to pass on the new one.

  **The eighth site is the one worth remembering**, because it survived the
  sweep that fixed the other seven. `/mail` renders its key legend *inside* the
  block rather than in the title, so its box needs a floor of **two** — it
  spelled its clamp `clamp(2, …)`, matched no grep aimed at `clamp(1, …)`, and
  collided with the ceiling one row earlier than everywhere else (dead at five
  rows, not four). The lesson is not "grep harder": the two-argument helper
  **could not express that box**, and a helper that cannot say what a caller
  means is how a caller keeps saying it inline. `list_height_reserving(rows,
  height, reserved)` can, `list_height` delegates to it with `reserved = 0`, and
  the degradation runs the safe way — the ceiling floors at one row and then the
  *floor is pulled down to meet it*, so a terminal too short for the strip and a
  row of list gets a useless live box instead of a dead session. The general
  form to search for is `.clamp(` near `saturating_sub(4)`, not either literal.

- **`KeyCode::Char(c)` is not a typed character.** crossterm reports the
  modifier *beside* the letter, so a match on `Char(c)` alone sees the bare
  letter and cannot tell Ctrl-C from `c`. Harmless in the main input box, whose
  `ctrl` branch runs first and consumes them; not harmless in `/mail`, whose
  keys go through `action_for`, where **Ctrl-A archived the thread under the
  cursor, Ctrl-D dismissed it, Ctrl-T made a task of it and Ctrl-R started a
  drafting run** — on chords that mean beginning-of-line, delete and refresh
  everywhere else in a terminal. Seven sites had their own copy of the mistake,
  which is the `list_height` lesson again: a new modal is written by copying
  whichever sibling is nearest, so the fix has to be the thing that gets
  copied, not seven guards.

- **A view can answer a shorter question than the one it exists for.** `/tools`
  detail exists to say what a tool may do to you, and its capability block sits
  at the *bottom* of a body whose top is an MCP server's own description —
  arbitrary third-party text. With the built-in tools it always fit; on
  `kg_task_create` it was eight of eleven lines, with "reads data the user
  considers private" below the fold and `Up`/`Down` guarded `if !detail` so no
  key reached it. The general shape: when a surface's *answer* is positioned
  after content whose length someone else controls, the surface degrades to
  looking complete while omitting the point. Test it with the largest real
  input available, not with the fixture.

- **A sweep that keys on a spelling misses the shape.** Seven modals carried
  `rows.clamp(1, terminal_height.saturating_sub(4))`, which panics at four rows
  because the subtraction saturates below the floor; the fix swept the codebase
  for `clamp(1,` and fixed six. `/mail` spelled it `clamp(2, …)` — it reserves a
  row for its key strip — so it matched nothing, and it was the *worst*
  instance, dying at five rows where the others died at four. It was found only
  because someone asked which modals have an in-box header, which is the
  question the bug was actually about. **Search for the invariant, not the
  token.** The second half is why the spelling diverged at all: the shared
  helper took only `(rows, height)`, so a box that reserves a row could not be
  expressed and its author wrote the arithmetic inline. A helper that cannot say
  what a caller means is how that caller stops using it.

- **When the fix's fail-safe behaviour is the new bug's observable, the test
  that caught the original cannot catch the misuse.** The repaired helper takes
  `(rows, terminal_height, reserved)` — three `u16`s — and its whole design is
  to degrade to a small box rather than panic. Transposing the last two
  arguments therefore compiles, does not panic, and silently returns a
  three-row box; the tiny-terminal regression test passes with the arguments
  swapped, because a box too small to read is not a crash. Pin the *values* at
  the call site, or make the type carry the meaning. A guard and a bug that
  share an observable are indistinguishable to every test written against the
  guard.

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
- **A fixture can make a test pass for the wrong reason.** The helper seeding
  two models' runs generated colliding session ids, so the second model's
  transcripts silently rewrote the first's — and the blended-rate test passed
  while reporting the *healthy* model as the broken one. It only surfaced
  because the assertion named which model it expected. Assert on the identity
  of the thing you found, not only on the count.
- **Substring grading measures formatting.** `"$2,520"` failed a check for
  `2520`; `"do **not** agree"` failed `not agree`. Both answers were right. The
  `normalize` helper in `mecha-core/src/eval.rs` handles it — extend that, don't
  work around it.
- **…and again, unboundedly.** "There is no `budget.csv`" matched none of ten
  hand-listed negation phrasings. The negation phrasing space has no bottom —
  that case is judge-only now. Reach for `expect.judge` when you catch yourself
  enumerating synonyms.
- **Check what the project already measured before proposing the fix.** A
  session profiling context growth found tool output filling ~8k tokens a turn
  and proposed two changes: add offloading, then lower the budget so it fires
  more. Offloading had existed since 2026-08-05 (`ToolCtx::spill_dir`), and
  `CONTEXT-RESEARCH.md` had already measured the second — cutting tool-output
  tokens 38.4% cost **6.8% more**, r = 0.154, because trajectory length
  dominates and re-reading spilled content costs turns. Both errors came from
  optimising *context occupancy* without checking the *cost and outcome*
  evidence in this repo. A `docs/*-RESEARCH.md` with an "ordered implications"
  section is a checklist to read before proposing, not a document to write
  after deciding.
- **Quoting a string is not the same as protecting it.** The benchmark adapter
  `shlex.quote`d each task instruction, which is the correct reflex for
  untrusted text on a command line and did nothing here: quoting guarantees the
  text becomes one argv *entry*, and the problem was that entry's first
  character. `terminal-bench/pytorch-model-recovery` opens with `- ` because
  its description is a bulleted list, clap read a flag, and the run exited 2
  before starting — scored 0.0 and indistinguishable from a model that tried.
  Pass `--` before any positional you did not write. Note the general shape,
  which this project keeps meeting: **a harness fault that produces a
  plausible-looking score is worse than one that crashes**, because nothing
  ever asks about it.
- **The transcript you are reading may not be the run that happened.** A
  28-turn benchmark trial's session file held 8 assistant messages starting
  mid-conversation — recording sliced "what the run added" off a list
  compaction had rewritten in place, so the rebuilt head (and the summary in
  it) never landed, and crashed runs recorded nothing because messages were
  written only after a successful return. Half a day of the diagnosis went to
  reconstructing what the recorder had dropped. A recorder that assumes an
  invariant (append-only) the recorded system deliberately breaks (compaction)
  is silently wrong exactly when the record matters; reconcile against what
  was actually recorded, don't index into what you assume was.
- **Check where every output stream lands before a long run.** Harbor captured
  `stderr: None` on all 21 trials, and stderr is where mecha's compaction
  notices and tracing go — the one channel that would have said which trials
  compacted, recovered, or gave up. The evidence for a day of forensics was
  discarded by a default nobody had looked at. Before any multi-hour run,
  confirm each stream's destination the way `-np 1` gets confirmed: by
  checking, not assuming.
- **A judge needs room to think before it answers.** At `max_tokens: 512` the
  judge spent the entire budget on reasoning and returned empty content with
  `finish_reason: length`. It is 4096 now, and an unparseable verdict reports
  the stop reason rather than just the empty string.
- **A shared stylesheet is verified against every surface it styles.** The
  booking page's picked-slot fill was keyed on `.slot:has(input:checked)` —
  and the poll page's cells are also `.slot`s, each always holding a checked
  tri-state radio, so every cell flooded solid accent. Unit tests on both
  pages stayed green; a screenshot of the *other* page caught it in one
  glance. State selectors on a shared sheet name the control
  (`input[name="_slot"]`), never the element — and restyling one surface
  means rendering them all.
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
- **An exit code is an interface, and exhaustion speaks it badly.** `mecha
  run` exits non-zero when a run stops on `MaxTurns`, so harbor recorded every
  turn-limit exhaustion as `NonZeroAgentExitCodeError` — 7 of the 21 salvaged
  benchmark trials read as agent crashes when the agent had run 40 full turns
  and given a partial answer (one of them *scored 1.0*, because the verifier
  grades the artifact). Any harness that keys on your exit code will read
  "gave up" as "broke". Decide which meanings your exit codes carry before a
  third party starts parsing them.
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
- **A pure function can be right in isolation and wrong against stored
  state.** `merge_push` — the three-way fold behind the switchboard's two
  writers — passed every unit test: right values, right conflict list. It
  re-serialised the merged table, so a push that won *every* conflict produced
  text differing from the pushed file only in formatting, and the stored
  `baseline` and `effective` then differed too. "Has the browser edited this?"
  is a text comparison, so the cockpit would have reported unpulled edits that
  did not exist and every later push would have taken the merge path for
  nothing. Invisible to a test on the function; obvious the first time the two
  stored columns sat side by side. **Test a pure function where its output is
  compared, not only where it is produced.**
- **Parsing is not validation, and a permissive parser hides the case you
  meant to catch.** A profile's timezone check was written as "does
  `chrono_tz` accept it", and the test asserting `EST` is refused failed —
  `EST` parses, because the IANA database still carries the legacy
  fixed-offset zones. A booking page rendered in `EST` is an hour wrong from
  March to November and looks right throughout, which is the failure shape
  this project keeps finding. The rule is now `Region/City` (or `UTC`), and
  the test asserts `"EST".parse::<Tz>().is_ok()` *first*, so it fails loudly
  rather than silently passing if that premise ever changes. **When a library
  accepts more than you mean, encode the narrower rule and pin the premise in
  the test.**
- **Writing an invariant in a doc comment is not enforcing it.** A
  high-effort review of the switchboard branch returned ten defects, and
  **four violated rules asserted in the module docs sitting beside them** —
  `inventory.rs` said an off-origin host is shown always while `host_of`
  returned the URL's *userinfo* as the host and `masthead` omitted the host
  entirely for labelled links; `editor.rs` said a slug must never be burned
  "as a side effect of typing one" while `save` upserted one for any slug in
  the form. Each doc was written honestly and each described the intent
  rather than the code. **Prose beside a rule is evidence the author meant
  it, never evidence the code does it — and a reviewer reading the code
  against the comment finds precisely these.**
- **A static path segment beside a parameter shadows it, for whichever value
  matches.** The shared assets sat at `/@a/{name}` next to
  `/@{handle}/{slug}`; `a` is a legal handle, matchit prefers the static
  segment, and that user's every switchboard answered 404 at a URL their own
  account page printed. Reserving the handle fixes one case and leaves the
  trap for the next static segment. **Keep static segments out of a
  parameter's namespace entirely rather than reserving the values that
  collide.**

- **The scrub's own author passed it; fresh eyes with no list did not.** The
  mecha-graph extraction was audited twice. The first pass (and the agents
  that applied it) rationalized three keeps: a "replacement" address that was
  a real Vermont locality triple, a real company founder kept because he was
  "a public figure in his public role", and household fixtures whose names
  were synthetic but whose facts — the pediatrician visit, the snow tires,
  the river — were still one real household lightly relabeled. The second
  auditor was deliberately given no list of what had been removed, and found
  all three plus a live wearable transcript. The lesson: a verification pass
  that knows the answer key checks the key, not the work. Independence means
  withholding the list, and every new find goes into the gate so the next
  regression is caught mechanically.

**The commit that shipped without its own new files was authored by a
failed `&&` chain.** One command staged and committed the first change, then
staged and committed the second — but the first commit died on the
pre-commit fmt hook, the chain aborted, and the retry re-staged with
`git add -u`, which updates *tracked* files only. The second commit landed
without its two brand-new files, did not build in isolation, and the miss
surfaced only when `cargo install` ran from the merged tree. Two lessons:
after any failed commit, the index is not what the failed command's later
steps would have made it — re-verify with `git status`, don't re-run from
memory; and "every commit builds in isolation" is only true if something
builds the commit's own tree, which the worktree (holding the untracked
files) cannot prove.

**A doc written from a build log is written on the machine where it
already works.** The voice page's most load-bearing sentence — that the
published crate contains the facade and none of the pipeline — appears
nowhere in the build log that documented the stack, because the person
writing that log had every service running locally and never had to
discover the packaging boundary.

The mechanism, sharpened by the session that found it: **explaining a
system to someone who does not have it is a different act from documenting
it, and only the first has to answer "what do I need before any of this
works?"** A maintainer writing docs answers *what does it do* — a question
that never reaches the packaging boundary, which is why
`cargo package --list` had never been run against this feature by anyone.
The general lesson: when documenting for
users, the facts most worth stating are the ones the author's environment
makes invisible, and `cargo package --list` / `git ls-files` answer in
seconds what memory answers wrongly. Check what ships, not what works.

**A plan and a to-do list are the same words, and only a timestamp tells
them apart.** On 2026-08-19 at 04:23 the handoff gained a section reordering
the remaining mail phases by what the corpus had just measured. Between 11:08
and 18:10 the same day every one of those phases shipped, in commits that name
them by number. Nothing went back to strike the plan, and nothing recorded the
arc in this file either — so for six days the section read as a to-do list for
work that was finished, headed "four phases remain". On 2026-08-25 a session
picking the project up cold read it, chose the pre-filter as the best
value-per-effort item, cut a worktree and started scoping it before a grep
found `mail_triage::prefilter` already there with both rules and a
`PrefilterRule` recorded on every verdict.

Two mechanisms, and the second is the one that generalises. The narrow one:
**the reflex that updates prose about a subsystem is not the reflex that
strikes an item off a list.** That section's narrative was revised repeatedly
in the days after — it discusses the 2026-08-25 both-account sweep in
detail — while the list beside it was not touched, because you revise the
story when you *learn* something and the list only when you *finish*
something, which is the moment you are least inclined to write anything down.
When a session executes a plan it wrote the same day, closing out is part of
that arc rather than a chore for later; by the next session the plan has
become indistinguishable from a backlog.

The broader one: **a blanket claim of verification is worth exactly the
evidence beside it.** The handoff opened with "every item below was
re-verified against source on 2026-08-24" — six days after this work landed,
and it did not cover this section. That sentence is what made the stale list
credible; without it a reader would have checked. A coverage claim should
either name what it checked or be narrowed until it can.

**And the diagnosis nearly went the same way as the entry above it.** The
session's first explanation was that the 08-24 sweep had missed the section —
a mechanism that fits the symptom perfectly. Checking with
`git merge-base --is-ancestor` showed the bullets were authored at 04:23,
*before* any of the work existed, so there was nothing for a sweep to miss at
the time they were written; the failure was never re-checking them, not a bad
check. That is the same lesson the branch-cut-too-early entry reached
independently, arriving from the other direction: **a mechanism that fits the
symptom is not yet evidence it caused it**, and the cheap test is almost
always re-running one command against two starting points.

- **2026-09-04 — a wall-clock claim with nothing behind it.** A lane told a
  peer the shared checkout had been on its feature branch "between roughly
  13:40 and 14:05". No command had produced a timestamp; the window was
  invented, and it felt like a memory rather than an inference. The reflog
  (`git reflog --date=iso`) said 15:33:01 to 15:37:18 — four minutes,
  starting a hundred minutes after the window given — and the peer had read
  `.git/HEAD` at 15:36, inside the real window, and pushed back rather than
  accepting the account. **An assistant with no clock must never emit a
  wall-clock claim it did not read from something**: cite the reflog, a
  file mtime, a journal line, a unit's `ActiveEnterTimestamp`, or say that
  elapsed time is unobservable from inside the turn. This is neither of the
  two shapes already named — it is not grading a report instead of the
  artifact, and not reading a property that answers a different question —
  it is a measurement fabricated about one's own actions, in a domain where
  the artifact exists only if you go and make one. The counter-technique
  from the same hour is worth keeping beside it: **a branch name is a claim
  about which file; the blob id is the file.** The voice worker was
  restarted correctly *while* the checkout sat on the wrong branch, by
  hashing `scripts/voice/worker.py` on disk against
  `git rev-parse origin/main:scripts/voice/worker.py`, which needs nobody's
  account to be true and also settles 2026-09-03's inverse (branch said
  `main`, tree was stale). Past git it is the same rule as `/props` over
  the config file and `sessions health --json` over a version string: ask
  the thing that will actually run.

**A negative control that leaves the evidence in place is not one.** To show
a source-walking test would catch a missing line, the line was commented out,
and the test still passed: it matched on text, and the comment still held the
text. Deleting the line made it fail. When sabotaging a check, remove what
the check reads, not what the code executes.

**A long-lived process's snapshot of its conditions describes the morning it
started.** `mecha serve` sampled the homeostat once, when it built its agent,
and every web turn's `RunContext` inherited that sample, so each web run
recorded the backlog as it stood at daemon start and a delta against it. The
numbers were plausible, so nothing looked wrong; it surfaced only because 1h's
brief (#305) reads its commitments from that snapshot and a test staged a
draft after `serve` started. A condition that belongs to a run is sampled per
run. Ask of any recorded condition when it was read; for the surfaces that
still hold one per process, see HANDOFF.

### Learning

**2026-08-30 — the safeguard's release condition was satisfied by the
evidence it existed to act on.** Probation's stricter retirement threshold
released once the ledger "measured" the rule — but the measurement that
matters, an attributed regression, only ever arrives *inside* an
observation, so any rule with conviction evidence had already been released
to the ordinary threshold by the scan reading it. `PROBATION_RETIRE_AT` was
unreachable from the day it shipped, every unit under it green, three
documents describing it. General shape: **when a hedge's release predicate
is implied by its trigger evidence, the hedge is decoration — check the
implication direction between "what releases it" and "what it exists to
catch", and fire the joined path once before relying on it.** Every piece
was unit-tested; only the drill that ran the motion whole could have found
it, and it did, on its first run.

**2026-08-30 — a closed list gates live text, but recordings are of their
era.** `STEP_ESCALATION_STEM` shipped mid-day 2026-08-28; a transcript from
that morning carried the same fully-templated nudge body with no stem, and
the miner recorded it as a user's steer — it reached the probe corpus and
was counterfactually replayed as if a person had typed it.
`is_harness_voice` now also matches the frozen pre-stem bodies. General
shape: **matching harness voice against recordings needs every wording that
ever shipped, not the current one** — the historical strings are frozen and
can never drift, so adding them costs nothing; omitting them lets the
harness's own words earn rules.

All found by pre-push review or by running it.

- **Suppressing an alarm is a claim that needs its own evidence.** Startup
  warned that `triage` rules could never fire. Two readings were equally
  consistent with what anyone knew: the warning is a false positive because
  the domain is loaded by a named pass, or the warning is *correct* and the
  feature is incomplete. `PASS_DOMAINS` was added on the first reading, by two
  sessions who agreed with each other and tested neither — and the second was
  true: nothing loaded triage rules at all, so the loop wrote into a file no
  reader existed for, with the alarm muted by the same hand. Before silencing
  a check, verify the thing it is complaining about, not the reason you think
  it is complaining.

- **"Every front-end writes this" was true of eight of ten.** `Record::Outcome`
  is documented as written once per finished run by every front-end, and
  `record_outcome` had ten call sites — `run`, `chat`, `tui`, `trigger`,
  `frontdoor`, `slack`, `voice`, `serve/chat` — with `tasks work` and
  `questions answer` in neither. So the run-quality corpus had never seen a
  delegated run, and the task-agent design's own open question ("`RunStats`
  already records enough to answer this later") was false for exactly the runs
  it was about. Nothing failed: both callers bound the outcome and checked only
  its `Err`, which reads as complete. **A claim of the form "every X does Y" is
  a grep, not a belief** — and the component that is missing Y cannot see that
  it is, because from inside it there is nothing to compare against. Found by
  counting call sites while looking for something else.

- **A tool that looks scoped and is not.** `cargo fmt -- path/to/one.rs`
  does not format one file: rustfmt walks module declarations from the crate
  root, so the argument narrows *what it starts from*, not what it touches.
  It silently reformatted two files belonging to another lane in a shared
  checkout, by a session that had spent the afternoon refusing `git add -A`
  and staging by explicit path precisely to avoid that. The reformatting was
  how a pre-existing nine-site drift became visible at all, so it ended
  usefully — which is luck, not process. **`git add <path>` at least
  advertises its scope; `cargo fmt -- <path>` advertises a scope it does not
  honour.** Check `git status` after any whole-tree tool, however narrow the
  invocation looked.

- **A CI that did not run is indistinguishable from one that ran and
  passed.** On 2026-08-26 that happened **four ways in one afternoon**,
  across two sessions, and every one of them produced a green-looking repo:

  1. **Push events dropped.** Two pushes produced *zero* runs — Actions
     enabled, all six workflows `active`, GitHub holding the new sha as
     `main`, `total_count: 0` for it. A GitHub Actions **major outage** from
     15:11 UTC. Nothing errors; the push simply succeeds and nothing happens.
  2. **Runs cancelled by their own successors.** `ci.yml` sets
     `concurrency: cancel-in-progress` keyed on `github.ref`, and five PRs
     merged inside four minutes, so each merge's run killed the one before
     and the last was killed too. The merge commit `efa04e2` ended with
     **no successful run at all** while the branch looked healthy.
  3. **Two sessions cancelling each other.** `github.ref` is
     `refs/heads/main` for *every* run on main — push or dispatch, whichever
     sha — so two people dispatching CI serialise into mutual cancellation.
     Same collision as (2), one level up.
  4. **Every convenient query is addressed by position, not by sha.**
     `gh run list --limit 1`, `gh pr checks`, the Actions tab — all move
     under you. One session read the top row, reported `main` green, and had
     in fact read *the other session's* run on a different sha; the trap was
     named to them one message earlier and they did it on the next command.

  **The only query that answers the question is
  `gh api "…/actions/runs?head_sha=$(git rev-parse main)"`.** And the
  reporting rule that falls out: *clean locally* and *CI passed* are
  different claims about different objects, and a handoff that merges them
  is asserting something nobody checked — which is exactly what happened,
  in the one file whose job is to be reliable, because one session took a
  peer's verification claim as fact and wrote it down.

  The narrower rule, offered by the session that made the unchecked claim
  and worth more than "verify": **a verification claim should travel with
  its run id.** A conclusion — "main is green" — can only be believed or
  not; an id can be checked in one command, and `run 32992960643` would
  have shown `head_sha=11a179d` immediately. This generalises past CI to
  anything one agent tells another it confirmed: **send the handle, not the
  verdict**, so the recipient checks the key rather than your reading of
  it. Every misattribution this day produced had the same shape — a
  conclusion transmitted where a checkable reference would have cost the
  same to send.

- **A cron job's binary is a running thing that answers to no `--version`.**
  The `update` skill said the graph repo's nightly "builds and runs from its
  repo tree and is not mecha's concern". It does not build — `nightly.sh` sets
  `PKG="$REPO_DIR/target/release/mecha-graph"` and executes whatever is there —
  and it is very much mecha's concern, because its `link --auto` step writes
  into the owner's graph at 01:30. Found on 2026-08-26 hours after a session
  repaired a linker bug out of the live graph (30 placeholder nodes merged, 121
  payloads rewritten, 23 accepted facts re-pointed); that binary was dated Aug
  25, so the nightly would have re-run the old linker and re-staged the same
  damage while every version string on the machine read current. The general
  shape: **an inventory of "what is running" that only lists things with a
  `--version` will miss every scheduled job**, and a skill's own claim about a
  script is worth checking against the script — this one was refuted by two
  lines of `grep`. Fixed in the skill, with the date check spelled out and a
  warning not to assume `cargo install --path` refreshed that path.

- **The "edit-distance gate" was never code, and the handoff carried it as an
  open item for weeks.** It was described as observed working live; a
  verification sweep on 2026-08-09 found no threshold, no `levenshtein`, and
  nowhere one could have been removed from — the behaviour was always the
  reflector model declining to mine a trivial edit. Closed as obsolete rather
  than deleted, because the next person would otherwise re-propose it.
  **An item whose evidence is "I saw it work" and not `file:line` is a
  hypothesis**, and it should be written as one.

- **A refusal nobody made must not be labelled as one.** `agent.rs` prefixes
  whatever reason an approver returns with `"Denied by the user: "`, which is
  the exact string the miner keys on — so a remote approval nobody answered,
  and every read-only run's mode refusal, arrived as corrections from a person
  who never spoke. The fix had to be a third `Decision` variant in the core,
  not a wording choice in the front-end. **When a string carries meaning
  downstream, the type has to carry it instead** — and check who else is
  already producing that string before adding a producer.

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

**A cloning TTS given nothing to say speaks its prompt or its prior**
(mecha-1e, 2026-10-03). Breeze, sent text with no letter or digit, did not
refuse: "." came back as "Um", "Yeah", or the reference clip's own sentence;
"..." as up to 15 s of babble or Ukrainian; "?!" as 20 s. Pipecat's splitter
emits a lone "." after "...", so it reached the engine in live calls. Guard
the input (#531), because the engine will not.

**A model reads its own earlier replies as examples of how to end one**
(mecha-1e, 2026-10-03). A persona's interrupted replies were stored as
heard, mid-sentence, and the model began stopping its own replies after
"\n\nI", each one another example: 41 of 75 by the end of one chat, all
recorded as completed. Whatever shape the history has, the next reply
learns it. Trim what goes back to the model (#538), never what is recorded.

**A reasoning cap below one draft is worse than none** (mecha-1e,
2026-10-03). A persona's long thinking was its reply drafted and then
redrafted. A 512-token cap stopped the draft half-written and lost 10–20,
blind, to the full budget; 1024 won 24–10, with a third of the worst wait.
That is one chat, judged by the same model, so read it as "1024 is at least
as good", not as a ranking. Read what the thinking is doing before choosing
its budget.

**A one-sided example list in a prompt is copied as a menu** (mecha-1e,
2026-10-03). The voice director, shown texture words, used "husky" in 21 of
24 directions, and one draw collapsed into the same ten adjectives over 19
lines; with no list at all it wandered into gestures and vocal events.
Balance any example list and offer "or none". A fixed TTS seed did not cut
line-to-line drift (5.5 semitones either way): the directions drove it, not
sampling.

**A sampler penalty sees only its window, and the default window is 64
tokens.** Qwen's model card recommends presence penalty 1.5 "to reduce
endless repetitions", and a persona chat on that profile still sent an
earlier reply word for word, 4 of 4. In llama-server the presence,
frequency and repeat penalties share `repeat_last_n`, and DRY has
`dry_penalty_last_n`. Both default to 64 tokens, and this build refuses
`-1`. The card's knob is for a loop *inside* one reply; a reply thousands
of tokens back is outside the window. Before trusting a penalty against
cross-turn repetition, ask what the window covers (2026-10-02).

**A fixed seed is a measuring tool, and in a conversation it is a copy
machine.** `seed = 42` restarts the sampler's draws at the same state
every request. A turn that adds little leaves the distributions nearly
unchanged, so the same draws pick the same tokens, and a persona sent an
earlier reply again, 7 of 7. Pin a seed where a run must repeat (evals,
replays, a judge's verdict); never in a chat (2026-10-02, #517).

**An unset key is the provider's default, not "off".** To test that `mecha
eval` refuses a blind model, a session aimed it at `-p anthropic`, reading
the config's missing `vision` as false. `vision_enabled` defaults it to true
for `kind = "anthropic"`, so the check passed, and two fixture prompts with
their pictures went to the API. Billing refused them, and the content was
fictional. Ask the resolved value (`vision_enabled`, the built provider's
`vision()`), and never aim a local check at an external provider
(2026-09-30).

A locally generated refusal and a third-party result used to lose their distinction
when a recording was replayed. Keep provenance with the recorded tool-use ID and
preserve it through lenient readers; a current capability label cannot reconstruct
what an old result actually contained. Delivery has the same distinction between
observation and inference: a missing response does not prove that the remote action
failed. Persist an unknown attempt before dispatch and require reconciliation
before retrying it.


- **A field you never read is a behaviour you never had.** The whole
  reasoning channel of every local model was discarded because
  `reasoning_content` appeared nowhere in the tree, and nothing failed: the
  turns still worked, the tests still passed, and the only symptom was
  "sometimes the model returns nothing" — which acquired a plausible causal
  story ("it reasons without terminating") that survived three days and drove
  three mitigations, none of which touched the cause. **When a symptom has an
  explanation nobody has measured, the first move is to record what actually
  arrived, not to mitigate.** The instrument was twenty lines and it inverted
  the diagnosis on its first firing.

- **The harness can be the thing teaching the model to misbehave.** Stripping
  reasoning out of the history meant every prior assistant turn showed the
  model calling tools with no think block — and the failure being chased was
  the model emitting a tool call with no think block. Measured at 6/6 versus
  0/6 on the same prefix. A conversation is not just what you send *this*
  turn; it is the in-context demonstration of what a turn looks like, and a
  harness that edits it is authoring that demonstration whether it means to or
  not. Corollary: `anthropic.rs` had replayed thinking correctly all along, so
  the asymmetry between two backends of the same codebase was visible evidence
  and nobody had read it as such.

- **Two dialects can disagree about what a number contains.** Anthropic reports
  `input_tokens` *beside* the cache tiers; OpenAI reports `prompt_tokens` with
  `cached_tokens` already *inside* it. `Usage::total_input` sums all three, and
  the compaction threshold reads that sum — so mapping the field across without
  subtracting would have reported every prompt at nearly twice its size and
  started summarising long runs at half the window they had. **A field with the
  same meaning in two APIs may not have the same arithmetic**, and the failure
  would have looked like working.

- **A guessed wire format has tests that agree with it.** Slack's
  `task_update` chunk was implemented from a plausible shape — nested under a
  `task_update` key, no id — and every unit test asserted that shape and
  passed. Live, Slack answered `invalid_arguments`, and said nothing at all
  about the missing `id` that keys a task card, so without it a call's
  lifecycle would have rendered as three unrelated lines. Fixing it exposed a
  second: a stream has one mode, and mixing a `chunks` array with the
  top-level `markdown_text` argument is `streaming_mode_mismatch` — invisible
  until the first bug stopped hiding it. **Tests written from a belief about a
  third party confirm the belief, not the behaviour.** Read the reference for
  the exact field names, assert the documented shape *and* the negative, and
  treat the first live call as the real test. This is the `ScriptedProvider`
  blindness `CLAUDE.md` names, in a new costume.

- The unified `calendar_create_event` schema said "attendees receive
  invitations", and on Google nobody ever had: `events.insert` defaults to
  `sendUpdates=none`, while Graph mails attendees unconditionally. A unified
  surface's documented behaviour is a *claim about every backend* — enforce
  it per backend, or the schema lies for some providers and the lie is
  invisible until someone waits for a mail that never came.


- **Never believe `finish_reason`.** llama-server reports `stop` alongside
  `tool_calls`. The loop believed it, dropped the calls, ended the run and
  returned an empty string — which graded as a model failure and was a harness
  failure. Any turn containing tool_use blocks is now a tool turn regardless.
  Assume the same class of bug exists for other local servers.

- **A refresh must never ask for a scope the grant may not have.** Entra's
  `refresh_token` sent the whole `SCOPES` list on every refresh; Google's
  never did. The moment that list widened — `Mail.Read` becoming
  `Mail.ReadWrite` on 2026-08-18 — every refresh asked Entra for a *superset*
  of what the stored grant had consented to, which it refuses with
  `invalid_grant`, which the classifier correctly reads as permanent and
  reports as a dead login. Every already-working account would have gone dark
  about an hour after the new binary was installed, reporting a revocation
  that had not happened. RFC 6749 §6 makes `scope` optional on a refresh and
  defaults it to the original grant. **A credential widening is a two-sided
  change: the request that mints the grant and every request that renews it
  disagree about what the grant contains, and only one of them was written by
  the person doing the widening.** Found by reading the refresh path while
  answering an unrelated question, not by a test.

- **Two different failures can be the same error string, and the recurring one
  wins on priors.** Google expires the refresh token of an OAuth client in
  *Testing* publishing status exactly 7 days after consent, and returns
  `invalid_grant` — "Token has been expired or revoked" — which is
  indistinguishable from a real revocation. The 2026-08-11 outage that
  motivated `mecha doctor` was recorded as a revoked token; it was six days
  after the grant, and the client was in Testing. Doctor was built to *report*
  it and does so correctly, but a marker written on failure can only ever
  describe an outage that already started. The fix is a `granted_at` stamp
  never touched by a refresh, a `grant_lifetime_days` the user declares
  (no API reports publishing status, so inferring it would either nag every
  verified app or stay silent on every Testing one), and a warning two days
  out. **When a failure recurs on a schedule, the store has to know the
  schedule — otherwise every instance looks like the first one.**

- **Grading the wrong axis is worse than not grading.** The mail classifier's
  escalation rule was instrumented to record when a second pass over the full
  body *changed the bucket*. The first real measurement said 13 of 51 threads
  escalated and one bucket moved — which, by the criterion written into the
  field's own doc comment, said the rule was wasteful and should narrow. It
  was measuring the wrong thing: a second pass that leaves the bucket alone
  while fixing `request_type` (the input front-door routing runs on), a
  deadline, or a `one_line` that read "message cuts off" has earned its call
  and registered as nothing. **An instrument that reports one axis of a
  multi-axis change does not produce a weak signal, it produces a confident
  wrong one** — and a number gets acted on where a gap gets investigated. Fixed
  by recording which fields differed, with `reasoning` excluded because prose
  differs on every re-read and would make every escalation look like a change.

- **`systemctl is-active` exits non-zero for `activating`**, so
  `until ! systemctl is-active <unit>` returns immediately on a oneshot that
  is still starting. A wait loop built on it reported a 25-minute sweep as
  finished within seconds, and the run was confirmed "clean" twice before
  anyone noticed it was still going. Poll `SubState` against `start`/`start-pre`
  instead. The general shape: **an exit code that distinguishes "no" from "not
  yet" cannot be used as a boolean**, and the failure is silent because the
  wrong answer is the one you were hoping for.

### Containment and state

**Guard the driver, not only the verb.** Features step 3 put a guard at the
top of each feature's CLI verb, and review found four places that reached
the same feature without the verb: the TUI's `/queues` and entity panel ran
`mecha-graph` directly (#451); the TUI's `/send` and `/remote-control` called
Slack's `send_file` and `attach`; the TUI's `/frontdoor` opened and
reconciled the request store in-process, and its child's refusal went to a
nulled stderr; and `/polls` called `pick_next` directly (all #452). Each
wrote or posted with the switch off. **Anything that opens a feature's store
or talks to its server asks for itself; the verb is one caller among
several.**

**A guard and what it guards must read the switch at the same time.**
#451's route gate was a start-time snapshot while the nav re-read the file
per page load. Enabling a feature while `serve` ran showed its tab and then
404'd every request on it, naming the command just run, and disabling one
kept its routes writing until a restart. Reading the file per request fixed
both directions. **When two surfaces show one fact, give them one read of
it.**

**A vouch checked after the untrusted side already holds the data is not a
gate.** #376 first refused an unvouched spoken turn at the voice facade, but
a worker that predated the log silence had already written the chat's key
and the caller's words to the journal before any turn reached the facade.
The check moved to the door: `serve::forward_offer` asks the worker before
it hands over the offer, and the page lets no media flow without the vouch.
**Find where the data first lands, and put the check ahead of that point,
not ahead of the first place that is convenient to check.**

**Redacting a trace still records the fact.** #376's passes 2 and 3 found
three places that kept no words but still said an incognito chat had been
spoken into: a log line with the key replaced by `chat:<incognito>`, a
refusal message that names the kind of chat, and a presence stamp that
other runs copy into their own transcripts. For a promise of "no trace", the
fact that it happened is the trace. **Drop the line; do not redact it.**

**A refusal must lead with its outcome.** The first live run of the image
library read a refusal that opened with the characters' names as a finished
picture, never retried, and told the owner the image existed (#384).
Anything a model reads as a result must say what did not happen before it
says anything else.

**A digest proves the text did not move, not that a person read it.**
`approve --shown <sha256>` let any shell compute the hash of a model's
proposal and approve it unread. #385 replaced it with an HMAC under a key
only the serving process holds, so approval needs the page that showed the
text. **Evidence of reading has to come from the surface that did the
showing.**

**A lock enforced on reads must be enforced on writes.** `POST
/api/library/{kind}/{name}/unlock` revealed a hidden library entry with no
token, and the 404-vs-409 split against `approve` said which entries exist
(#385). A write route that can change what a read shows is itself a read.

**A fixed `.tmp` name opened with `create(true)` keeps an existing file's
mode and writes through a planted symlink.** #258's lesson, re-learned in
`imagelib`. Temp files are `create_new`, `O_NOFOLLOW`, and randomly named.

**A read that answers "unavailable" with an empty list is read as "empty"
by any caller that replaces its state with the answer.** While a run held a
web conversation, the transcript read returned `entries: []` and relied on
the page catching up over SSE. The page's reconnect path called `load()`,
which replaces the transcript. So on the phone, whose stream drops on every
screen-lock, a whole chat vanished mid-run, and stayed gone because nothing
re-read when the run ended (#358). The empty list was a placeholder the
reader could not tell from a fact. **When a read cannot answer, it must say
so or return the last thing it can vouch for, and a caller that replaces
state must know which one it got** — the same shape as a dash rendered as a
zero, one layer up.

**A fallback in a helper can make a fail-closed guard unreachable on the one
deployment it was written for.** #243's `Resolved::NeedsZone` refused a
relative window when no zone was configured, and its test passed a literal
`None`. But the zone came from `configured_zone()`, which fell back to `TZ` —
and `TZ` is in `Sandbox::child_env`'s `BASE` passthrough and UTC on the
server — so in production the guard never fired and `today` resolved to the
UTC day, the incident itself. The review found it; the suite could not,
because the test bypassed the lookup the guard depended on. **Test a guard
through the real input path, not through the value you believe reaches it** —
`a_utc_tz_is_not_a_window_zone` drives the real variable lists through an
injected lookup — and when one helper serves two callers with different
stakes (rendering can tolerate the machine's zone; resolving a day cannot),
split it rather than let the safe caller's fallback leak into the strict one.

**A derived value written into a config struct at load travels with every
copy of that struct.** The first design for #243's zone handoff stamped
`MECHA_TZ` into each server's `env` in `load_layers`. `TrialEnv::base_config`
copies live servers out of the operator's loaded config, so every trial would
have carried the operator's zone into an environment that set its own, with
nothing in the trial's files to show it. Derive at the point of use from the
config that is running (`owner_zone` is `serde(skip)` and overwritten at each
place a server list is assembled), never at load into a field a copy carries.

**Two sessions in one checkout is fine until one of them operates on a file
the other is writing.** Two lanes worked the same tree for an afternoon —
reading each other's state cost nothing and caught real things (a misplaced
brace, a wrong install timestamp, a guard one lane needed and the other did
not). It broke exactly once: told "you go first", one lane lifted the
other's hunks out of a shared file to commit around them, and in the window
between the lift and the restore the other built new code against the
stripped version. Their tree stopped compiling and the cause was not
theirs. The rule that would have prevented it: **commit order follows who
is mid-flight, not who offers to go second** — "you go first" is only safe
once the other party has actually stopped. Two smaller ones that held up:
restore from a whole-file snapshot rather than a reconstruction (md5 the
result), and when a whole-tree gate like `cargo fmt --all --check` fails on
files outside your commit, bypass with the reason written down rather than
reformatting someone else's work into your change.

**A workaround for a gap that had since closed read as a deliberate rule,
and broke the feature it was protecting.** `tasks work`'s one-run-per-task
guard keyed on `status == "waiting"` because phase 1 had no way to tell the
agent from a person, and said so in a comment naming `waiting_on` as the
thing that would fix it. Phase 3 shipped `waiting_on` the same day and
nobody came back. Every finished run leaves a task `waiting`, so the second
delegation on any task refused — silently, from the web, because the child
exited 1 into `/dev/null` after the endpoint had already answered ok. The
general shape: **a temporary check outlives the reason for it and stops
looking temporary.** A comment naming the thing that will replace it is not
a reminder; the only reminders that work are the ones that fail. Where a
workaround cannot be made to fail loudly, it belongs on the same checklist
as the change that will retire it.

**A tool removed from the parent registry was still reachable through a
subagent, because the child's copy was made before the removal.**
`tasks work` withholds `kg_task_update` so a delegated run cannot close its
own task — but `build_subagent` clones each allowed tool out of the pool into
a separate registry while the agent is being *prepared*, so removing it from
the parent afterwards left any profile that allowlisted it holding a live
handle. A run told "you have no tool that sets status" could simply delegate.
Found by a review, not by testing, because every test had an empty subagent
list. The general shape: **a removal only reaches the copies that had not
been made yet — before trusting one as a control, ask who already took a
reference.** The fix refuses to start rather than stripping silently, on
`build_subagent`'s own precedent that a child quietly missing a tool its
description promises is worse than a loud failure.

**A session recorded the tool list before the tools were changed, so its
config record described a surface that never existed.** `RunConfig::of`
snapshots `agent.registry()` at call time, and both task entry points
appended the record *before* withholding `kg_task_update` and inserting
`ask_user` — so the transcript claimed a run had the tool it had been denied
and lacked the one it used, and `mecha replay` would rebuild the opposite
surface. Found while trying to *prove* the withholding had happened and
discovering the evidence disagreed with the code. The lesson: **a record that
snapshots live state has to be written after the state is final, and a record
of the wrong state is worse than no record** — the wrong one is believed.

**Seeding one row of data in a migration broke encrypt, decrypt and fork.**
The graph's V020 seeds an `agent-mecha` node so a task can be delegated on a
graph nobody has hand-populated. Every copy path runs migrations on the
*target* first and then copies the source's rows in, so the target already
held the row the source was about to send and the straight `INSERT` hit the
primary key. Caught by `test_copy_tables_covers_schema`'s neighbours rather
than the canary itself, and confirmed by running them on `main` first. The
general shape: **seeding schema is free and seeding data is not, wherever a
copy or fork path exists** — `nodes` joined the `INSERT OR IGNORE` pass that
`predicate` had been in all along, for one row.

**A "must already exist" check that mutated before it validated cleared the
real answer on a typo.** `set_task_waiting_on` retired the old `waiting_on`
fact and then resolved the new name, so `--waiting-on Nadai` did not merely
fail — it turned "Nadia owes me this" into "nobody owes me this", with an
error message that mentioned neither. The test written *for* the typo
protection is what caught the protection being incomplete. **Resolve
everything that can fail before touching anything that persists**, and where
a store has no transaction to lean on, that ordering is the whole guarantee.

**`current_exe` of a replaced binary is a path that does not exist, so a
long-lived TUI lost every child it tried to spawn.** Rust's `current_exe`
resolves `/proc/self/exe` to its target, and after `cargo install` swaps the
file, a running process's target reads `…/mecha (deleted)` — so `/queues`
died with `os error 2` and an outbox release failed *quietly*, the item
sitting `pending` as though the review surface were broken. Both reported by
the owner within the hour of the install, from the one session the
stale-process sweep had already named. The fix execs the `/proc/self/exe`
link itself, which the kernel resolves to the deleted inode: a session
drives the version it *is*. The general shape: **anything a long-lived
process re-derives from the filesystem — its own path included — can change
under it at an install; prefer handles that pin the inode.** Two sites keep
`current_exe` deliberately (a systemd unit's path, a sibling-binary lookup),
annotated so they are not "fixed".

**A mutating path that was never exercised shipped broken, because the live
store was correctly left alone and no scratch copy was made instead.** The
`/queues` class verdict passed `--proposer`/`--predicate` to a `mecha review
accept` that declared neither, so clap rejected it and the headline feature —
"one decision worth hundreds on instances" — could never have worked. Every
read path was tested against the real graph; the write path was skipped to
avoid mutating real data, and skipped entirely rather than redirected. **The
answer to "I must not test this against production" is a fixture, a fork, or a
`--dry-run` flag — never nothing.** (`mecha-graph fork` exists for exactly this
and turned out to be broken itself, 768-vs-1024 vector dimensions after the
harrier embedding switch; `--dry-run` was added to `mecha review` in its place
and is what finally exercised the path.)

- **Per-run jails and shared subprocesses do not mix, and the model finds out
  first.** MCP servers are spawned once with the agent, so a Slack connector
  giving each thread its own `RunContext` workspace left the servers rooted
  wherever it was launched: `bundle_render` resolved against the repo while
  `fs_write` wrote into the thread's jail. The model reported "the workspace
  and render tool have different root paths" and burned five turns working
  around it with `shell`. **Anything spawned once cannot follow a per-run
  value** — either root it somewhere both agree on, or accept that the
  isolation only covers what the loop itself resolves. Say which, in writing.

- **A measurement is not a conclusion, and a bad afternoon can masquerade as a
  law.** `-c 32768` was pinned for three days by one data point: raising it to
  131072 on 2026-08-07 took generation from 64 tokens in 1.06s to 64 in 52.6s,
  a 50x collapse. That measurement was real, and the rule written from it —
  *do not raise `-c`* — was not. 2026-08-07 is the day a runaway test OOMed the
  machine and systemd tore down both llama-servers; the KV cache was competing
  for memory that was not there. Re-measured 2026-08-10 on a quiet machine, one
  server at a time, medians of three at matched prompt lengths: 32k, 64k and
  128k are **within noise of each other** (92-93 tok/s at a 1k prompt, 81-82 at
  30k), and RSS is 21.5 GB either way. What costs is context actually *used* —
  63 tok/s at 108k — which is attention, not allocation. **When a single
  observation becomes a rule, write down the conditions with it; the next
  reader cannot tell an environment from a law.** The cost of not doing so was
  a 32k window on a 121 GB machine, and every compaction that came with it.

- **A server that loads while memory is contended stays slow for its whole
  life.** Found while measuring the above: a 64k instance started alongside
  another resident model held ~82 tok/s at a 1k prompt and did **not** recover
  when the other model was stopped, while a fresh 64k instance on the same
  quiet machine gave 92.23. Whatever placement decision llama.cpp makes at load
  is never revisited. It is also the likely shape of the 08-07 result. So the
  advice that outlived its own reversal: **measure tokens/sec after restarting
  a model server, not just that it answered** — and if it is slow, restart it
  on a quiet machine before believing anything about the flags.

- **A comment describing someone else's tool is a hypothesis, and this one was
  wrong for months.** Three places in two repos said `marimo export html-wasm`
  executes the notebook; it does not, and the two-minute experiment that settles
  it — a cell that writes a file, a notebook that would crash if run — had never
  been performed. The cost was not a bug but a *constraint*: the false claim was
  the stated reason the notebook template stayed off unattended paths, so a
  capability was withheld for a danger that did not exist. **Before a belief
  about a third-party tool becomes an architectural constraint, run the
  experiment that would falsify it.** The belief is cheap to test and expensive
  to inherit.

- **Adding the replacement is not removing the replaced.** A drag rewrite added
  list-level pointer handlers and left the old grip handlers attached, so the new
  state was never assigned and every new line was dead. The diff read correctly;
  the diff was not the thing running. It shipped, was reported unchanged by the
  person testing it, and cost a round trip. **When replacing a handler, delete
  the old one in the same edit, and verify against the built artifact rather than
  the source** — for generated assets, grep the output for the string that should
  no longer be there.

- **One asset name with two builders is one asset name with two answers.** Every
  page in the booking family links `booking.css`; two code paths built it and
  disagreed about whether the survey rules were in it; whichever page rendered
  first won. The live server took the correct path and the documentation gallery
  took the wrong one, so the only broken surface was *the one people learn from*
  — and nothing failed anywhere. **A shared output name needs a single
  definition, and a test that a real render agrees with it.** Asserting the
  presence of specific rules matters as much as asserting equality: two paths
  agreeing on the wrong thing is the other way to fail.

- **A capability that lives in the binary is a capability no agent will ever
  have, and nothing reports it.** The factory's MCP server sat at seven tools
  while its CLI grew to twenty, for six weeks, because the verbs were command
  bodies in `main.rs` and the server lives in the lib. Every test passed the
  whole time; the surface was discovered only by asking the agent to do
  something and being told it could not. **An integration whose coverage is
  implicit will drift, and the drift is silent by construction** — the only
  fix that holds is a test that enumerates one side against the other and
  demands a written decision for each gap. mecha already warned when
  `[outbox] tools` named a tool that did not exist, for exactly this reason;
  the factory had no equivalent, and that is the whole story.

- **A relative path in a deferred call means nothing without the jail it was
  written in — and the *display* forgets that before the executor does.**
  `outbox send` had always resolved a staged argument against the workspace
  recorded on the item; `outbox show` resolved it against wherever the reviewer
  was standing, so the first real staged poll reported a spec that was right
  there as "⚠ gone". The visible symptom is a false alarm; the dangerous one is
  symmetric, because a same-named file beside the reviewer would have been
  printed, and offered to open, as the draft's source — a human reading one
  file while approving another. **When a value is only meaningful in a recorded
  context, every surface that touches it needs that context, not just the one
  that executes.** A review surface that shows the wrong bytes is worse than
  one that shows none.

- **`Path::starts_with` is lexical, so it is not a containment check.**
  `<staging>/../escape.html` starts with `<staging>` by that test and lands one
  directory up. The factory's bundle installer used it to prove a path stayed
  inside a staging directory; a test caught it. Build the path from components
  and refuse anything that is not a normal one — a check that reasons about
  strings is not a check about where a write lands.
- **`last_insert_rowid()` is per-connection and per-*any*-table.** Using it to
  select back a row you just `UPDATE`d works right up until something else
  inserts anything on that connection — in this case minting a key between a
  form submission and its confirmation click, which made verification silently
  fail. If you need the row you changed, read it and change it by its key,
  inside one transaction. The general shape: a value that is *usually* the one
  you meant is worse than one that never is, because it passes the test you
  wrote first.
- **A privileged dry-run can create state with the wrong owner.** Running
  `factory check` as root before the service had ever started created the
  SQLite ledger owned by root, after which the service could read it and not
  write it — and the failure surfaced two steps later as "attempt to write a
  readonly database" from an unrelated command. Anything that lazily creates
  state should be run as whoever will own it, which is why the systemd unit
  runs its pre-flight as the service user.

- **A relative path is a bet that "here" never moves, and "here" moved.**
  `--mcp-file` joined fixture-server paths against the file's directory but
  left them relative — correct for as long as mecha spawned MCP servers from
  the invocation directory. When servers started spawning in the run's
  workspace (the fixed-root work), the join silently resolved against the
  workspace and every fixture handshake failed; the eval that would have
  caught it was not run again until the graph rename forced it. Canonicalize
  at the seam where a path is *read*, not where it is used — and when a
  subsystem changes its working directory, grep for every relative join that
  was betting on the old one.

- **A rebuild ignored the permission mode, and the status line kept asserting
  it anyway.** `apply_switch` passed the *retained* ask-mode approver into
  every `/model`, `/provider` and `/mcp` rebuild without consulting `app.mode`,
  so the mode silently reverted to `ask` — and it fails in **both** directions,
  which is what makes the symptom hard to recognise. From `allow` it goes to
  asking: tighter, merely annoying, and the version that actually showed up in
  testing as an approval prompt thirty seconds after `/mode allow`. From
  `read-only` it also goes to asking, and *that* one loosens — read-only
  refuses outright, where asking lets a human be talked into yes, which is what
  an injection is for. Common to both, and the sharpest part: the status line
  went on displaying the old mode, so the interface asserted a posture the
  harness was no longer holding. The silently-degrading-sandbox shape, in the
  one surface whose entire job is to say what the harness will currently allow.
  **An interface that displays a security posture must read it from the same
  value the enforcement reads** — here that meant making the mapping a function
  (`approver_for`) used by both, and asserting it with `Arc::ptr_eq`, because
  "did it reuse the retained one" is the only question that distinguishes a
  reinstated approver from a fresh one. Invisible to unit tests, which build
  the types directly and never take the switch path.

- **Claiming deleted the work before anything delivered it, and one path out
  did not deliver.** Text typed into a mirrored Slack thread was removed from
  its inbox, pushed onto the run's steering queue, and lost — because the queue
  drains at the *top of the next turn* and a run with no tool calls never has
  one. Nothing anywhere held a copy. Latent in keyboard steering the whole time;
  the remote path only made it likely, because lines arrive on a poll decoupled
  from run boundaries. If a handler removes work from a store before handing it
  on, every exit from that handler must either deliver it or put it back —
  including the exits you did not write, like a run that simply ends.

- **`interactive` is a claim about who is watching, and a detached child is
  never watching.** `mecha questions answer` resumed a delegated run with
  `setup::prepare(&opts, true)`, which is right for the terminal it was
  written for and became wrong the moment a web surface wanted to spawn it:
  detached, stdin is `/dev/null`, `TerminalApprover` reads `Ok(0)`,
  EOF-is-not-consent turns it into `"n"`, and every approval becomes
  `Decision::Deny("the user declined this call")` — the string the learning
  miner reads a *correction* out of (Learning → "a refusal nobody made"). Not
  a failed run: a run that teaches rules from a person who was never asked.
  `tasks work` had already learned this and carried the flag; the second door
  onto the same conversation did not. **When one entry point takes a posture
  argument, every entry point onto the same run needs the same argument** —
  and the posture must be passed in rather than sniffed from a tty, or it
  becomes a property of how the process happened to be launched.

- **`read_only` decides approval, never ordering.** `show_file` called in the
  same turn as the `fs_write` that made the file found nothing there:
  `agent.rs`'s `join_all` runs every *approved* call in a turn concurrently, and
  `read_only` only decides whether approval is asked for. `fs_read` has always
  had this. A flag named for one property will be read as guaranteeing a second
  one — the fix was the tool description, because there is no tool-level lever
  for ordering at all.

- **Failing closed on a thing that was never a record.** The connector's
  routing lookup iterated `~/.mecha/remote/` and joined `record.json` onto each
  entry, treating only `NotFound` as skippable. A stray file there — a
  `.DS_Store`, an editor backup — makes that `ENOTDIR`, so the fail-closed arm
  fired for *every* owner message in *every* thread and took the whole remote
  control down until someone found the file. Fail closed on a record you cannot
  read; skip a thing that was never a record. The two are not the same
  question, and one directory listing conflates them.

- **A predicate that encodes who writes to a structure breaks silently when a
  second writer arrives.** The D10 voice block was injected when
  `convo.is_empty()` — correct, and *only* because a voice conversation had
  exactly one author. The moment talking and typing shared a message list
  (D3), that predicate quietly changed meaning: a call opened into an
  existing chat would find the conversation non-empty and send no block, so
  the reply came back as markdown with headings, read aloud. It fails in the
  direction that looks like working software, not the direction that errors,
  and no test could have caught it because the condition was still true of
  everything the test knew about. The general form: when you add a second
  writer to anything, grep for predicates over its *contents* — they are
  assertions about authorship wearing a shape check. The replacement carries
  the fact explicitly (`WebSession::last_turn_spoken`), because "was the
  previous turn spoken" is not recoverable from the messages once both doors
  record identically.

- **A smoke test that provokes refusals writes into the corpus something else
  thresholds on.** Driving the D3 path by hand meant deliberately triggering
  read-only denials to prove the posture, and every one of them landed in a
  session JSONL that `runlog` reads and `mecha doctor` computes a per-model
  tool-error rate from — with a 20-run floor, so a handful of provoked
  failures move it. The traces were removed afterwards. Before hand-driving a
  path that fails on purpose, know which store is counting: the run-quality
  corpus has no notion of "this run was a test", and it is the one place
  where deliberately-caused failures are indistinguishable from the real
  thing.

- **A test that asserts prose freezes the prose.** The thread header said
  "nothing typed here reaches this session" — true when it was written, false
  one rung later, and posted to every reader in between. The test asserting
  that exact sentence is what kept it alive through the change that falsified
  it. Where a message states a *capability*, assert the capability's current
  shape and assert the old wording is **gone**, or the test becomes the reason
  the lie survives.

- **A guard that reads a fact the guarded party can write is advisory.** #293
  decided who may close a task from `MECHA_RUN_POSTURE`, which the `shell`
  tool stamps on every command, and a delegated run's command could set it on
  its own line (`MECHA_RUN_POSTURE=interactive mecha tasks set …`) and be
  recorded as an `owner-approved` closure: a forged verdict in the store the
  appraisal reads as the owner's. #294 moved the fact into a registry the
  harness writes about its own children (`shell_registry.rs`), and the
  variable became advisory. Before trusting an input to a guard, ask who can
  write it; if the answer includes the thing being guarded, move the fact to
  a store only the guard's side writes.

- **A control acts on the store as it is when it fires, never as it was when
  the control was drawn.** A Slack task card is composed while the task is
  open. Tapped after the owner had closed it elsewhere, Drop turned `done`
  into `dropped` with no closure record, because that move crosses no
  open/closed line, and Next reopened it, which 1d signs −1.0 against the
  session that did the work. `tasks set --only-open` rereads the row and
  refuses (#300), and the graph TUI reads the status from its database at the
  keypress rather than from the rendered row (mecha-graph#21). A double tap is
  the same bug.

- **`#[serde(other)]` does not catch an unknown variant that carries data.**
  1d's `Cite` is adjacently tagged, and the attribute matches an unknown tag
  only when nothing sits beside it, so a newer build's pointer with an id
  would have failed the error it rides on instead of loading as `Unknown`.
  `de_cite_lenient` and `curation.rs`'s `de_target_lenient` deserialize by
  hand, and `a_cite_from_a_newer_build_loads_as_unknown` pins an unknown
  pointer with an id loading as `Unknown`. For a closed enum on an
  append-only store, test an unknown variant *with* content, not just a bare
  unknown tag.

- **A setting a backend cannot apply must refuse, not parse.** `[sandbox]
  memory_mb` and `cpus` parsed under every backend and took effect only under
  docker, so an operator who set them under bwrap got no limit and no
  message. #295 applies them under bwrap through a systemd scope, which fails
  `Sandbox::preflight` when the scope cannot be made, and landlock, which
  cannot apply either, now refuses the combination. The silently-degrading
  guard, in config form: a key that is accepted and ignored reads to the
  operator as a key that works.

### Unattended runs

- **An outbox-routed tool executes at release, not at staging — so any
  freshness check inside it is checked hours later, against a file nobody
  refreshed.** `poll_meeting_create` refused a freebusy document over an
  hour old, which was right for the CLI pipeline that pipes it in and
  wrong the moment the call was staged: the owner approved the next
  morning and the tool refused its own input. Nobody hit it because no
  meeting poll had ever completed the path. The fix moved the input to a
  cache the slots timer rewrites every two minutes, so it is as fresh at
  release as at staging. **The general shape: a routed call is a deferred
  call. Anything it reads from the workspace — a path, a timestamp, a
  "within the last hour" — is read when a person gets round to it, and the
  card cannot show what will be true then.**

- **An edit that silently matches nothing ships a false claim.** The fix for
  the startup banner below was written three times before it existed: rustfmt
  had wrapped the `tracing::info!` across lines, a single-line string replace
  found nothing, the build passed, the tests passed, and it was reported as
  done. Nobody noticed until the unit was installed and the journal showed a
  started service with no evidence it was working. **A build that succeeds is
  not evidence an edit applied** — assert on the match, or grep for the new
  text afterwards. The same silent no-op hit twice more the same day.

- **A daemon that prints nothing at startup is indistinguishable from a wedged
  one.** The Slack connector logged "connector up" through `tracing`, which is
  invisible without `MECHA_LOG`, so the first run of it looked like a hang.
  The same session had spent a day citing that exact confusion in other
  people's software. **Anything that waits should say so on the way in, and on
  stdout rather than through a log filter** — and a refusal it makes should be
  visible too, because silence is what makes a working gate look broken.

- `mecha-mail freebusy --days 60 | slots push` could *never* satisfy a 60-day
  horizon: the freebusy window was stamped from one process's clock and the
  horizon deadline computed from another's, a second later. Anchor every
  derived deadline to the data's own stamp, never the reader's clock — which
  also makes the pipeline deterministic for free. Found by running the
  documented pipeline, not by reading it.
- The bookings sweep's "no new bookings" early-return silently skipped the
  cancellation and reminder passes on every quiet tick — the common case.
  When a verb grows a second duty, audit every `return` that predates it.


- **A clean merge is not a correct merge, and only a compile-time fact caught
  it.** Two sessions each added a slash command, so each changed
  `pub const NAMES: [&str; 16]` to `17` and added its own entry. git merged the
  hunk with no conflict marker and produced **18 elements declared as
  `[&str; 17]`** — a length that agrees with neither side. Nothing about the
  diff looks wrong; it was caught because the array length is checked by the
  compiler. The lesson generalises past git: where two edits to one list are
  likely, keep a fact about the list that a machine verifies, and never resolve
  a silently-merged hunk by eye. Three sessions worked one repository that
  night and this was the only merge hazard that a careful reading would have
  passed.

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
- **A new global precondition breaks the callers you did not think of, and a
  lazy caller breaks selectively.** Refusing a workspace that contains the
  mecha home fixed a real jail bug and silently broke the nightly rumination:
  `validate` and `learn --propose` both build a tool surface and so both hit
  the new check, and a systemd *user* unit with no `WorkingDirectory` runs in
  `$HOME`, which contains `~/.mecha`. The part worth the entry is that they
  build that surface **lazily** — only when there is a proposal to gate or a
  probe to replay — so quiet nights passed and the nights with work aborted.
  A precondition added centrally needs its callers enumerated, and a caller
  that only sometimes trips it is worse than one that always does: the failure
  correlates with the work being worth doing, which is exactly when nobody is
  watching. Found by an automated review, not by running it.
- **A fix to a refusal message is a change to a subsystem, and its other
  callers have to be re-checked like any other.** #241 spent four of its six
  review passes fixing defects its *previous* pass had introduced, all in the
  refusal/remedy layer and all the same shape. `WebSearch::denial_remedy` was
  written against the interlock and `agent.rs` appends it to the leak branch
  too, so it told an operator to add a backend that would not help; the fix
  passed a `DenialCause`, and then picked the first risk that matched rather
  than the *binding* one, so the both-fire case got the wrong remedy again;
  suppressing the blind-route hint under the leak guard left the same hint's
  remedy forty lines below unguarded. Each fix was correct about the mechanism
  being changed and silent about who else read it. The general lesson: when a
  guard's *message* is the thing you are repairing, enumerate every branch that
  renders it before you change one — the message is a function of which control
  fired, and there is usually more than one. A corollary the same arc paid for:
  a refusal that names an exit must check the exit is open under *every* active
  control, or it manufactures the dead end it exists to prevent.
- **Check-then-act across a human is a race, not a formality.** `outbox send`
  holds the store lock across execution so two sends cannot both pass the
  pending check. `outbox review` checked pending, printed the draft, and then
  *waited at a prompt* — the check and the act separated by however long a
  person takes to read. Anywhere a human sits between the test and the action,
  the test has to be repeated on the far side of them.
- **A graceful degradation is worthless if its consumer cannot parse the
  degraded form.** `mail_recent` fans out across accounts and is built to
  survive a lost one: it returns the surviving rows plus a trailing plain-text
  note naming what failed, and errors only when every account fails.
  `mecha mail classify` read that with `serde_json::from_str`, which rejects
  the note as trailing characters — so the one caller the mechanism existed to
  protect was the only thing that could not use it, and a revoked credential
  on *any* single mailbox failed the sweep for **all** of them, including the
  mailboxes that answered fine. The nightly had been pinned to
  `--account <the campus account>` for a year to avoid it, which made the workaround look
  like a decision about Google's token lifetime rather than a bug. The general
  lesson is about where the contract lived: `unified.rs`'s module header states
  *"A failed account never sinks a fan-out"* in prose, and prose in a producer
  cannot check its consumers. **When a component advertises that it degrades,
  test a caller against the degraded output, not just the happy one** — the
  partial response is a distinct wire format and nothing was exercising it.
  (2026-08-25.)

### Review process

**A fix that adds an optional hook leaves the old test on the branch
production no longer takes.** On #499, late turns were handed to a new
queue through `UplinkAudio.deliver`, which `run_bot` always sets. The test
pinning §2.5's ordering, that a late turn reaches the model before the
live audio that closed it, built its uplink without the hook. So it kept
passing on the unwired path while the wired one let live audio overtake
the late turn. Pass 6 found it from the code, not from a red test. When a
change wires something in, re-point the invariant's test at the wired
shape and keep the old one named for what it now covers
(`test_unwired_…`). And check that the fix's own futures resolve where the
invariant needs them: a first cut of pass 8's fix resolved the hand-off
after the answer instead of at the send. It was caught on rereading the
diff, before the push, and `test_delivered_before_the_answer_is_over` now
fails on that shape (2026-10-02).

**Read every review posted since your push, not since a time you guessed.**
On #468 a cut-off of "comments after 13:40" dropped the 13:39 pass outright,
and the 14:07 pass was not yet posted when the check ran — so a merge was one
step from a head whose last two reviews had not been read. Two habits, not
one: take the cut-off from the recorded push time rather than a guess, and
re-poll after the last read, because a pass can land while you answer the
previous one (2026-10-01).

**A reviewer's mechanism is a claim to measure, in either direction.** On
#473 a pass warned that `frame`, a TOML table, must stay `State`'s last
field, or a later scalar would fail to serialise with `ValueAfterTable`. A
ten-line probe showed `toml` 1.x writes keys before tables regardless of
declaration order, and the comment was not added. The same PR's real
defect, a drag that saved a frame the circle could not show, was confirmed
by deriving the layout formula before fixing it. Grade the artifact both
when a finding looks real and when it looks like a rule.

**Read the code's own account of an invariant before putting a question
about it to the owner.** On 2026-10-01 #466's open question — should an
experiment's condition hash carry the feature set? — went to the owner with
pros and cons resting on "an arm with no environment runs on the operator's
config", taken from the PR body rather than the tree. The owner ruled on
it. `condition_hash_world`'s own comment said every planned row has an
environment digest, and the real gap was one switch; the question was put
again before any code was written. A ruling is only as good as the picture
it was given, and the cheapest place to check that picture is before it is
sent.

It recurred on 2026-10-01 with a direction of safety at stake. In #503,
`[personas]`' crisis cooldown was described to the owner as a pause
length, so "allow longer" sounded like the safe choice. It is the window
in which a further crisis hit does *not* re-pause, so longer is weaker.
The owner ruled on the wrong framing, and the question was put again
before any code. Before asking which way is safer, read the constant's
own doc and its call site.

**Trace a sentinel value through every reader, not just the one you
wrote it for.** In #503, `[voice] offer_target = ""` was made to mean "no
proxy", and the features row read it as calls OFF. But serve mounts the
voice facade only when Calls is on, so "no proxy" silently became "no
facade", which is the thing the worker calls. Review pass 3 caught it, and
Calls now stays on with "no proxy" in its row (`feature::own_state`).
Before an empty or zero value takes on a meaning, grep for every reader
of the field (2026-10-02).

**Do not regrade a reviewer's severity to hand off a head.** On #474,
pass 3 named "one substantive issue … nothing else at the bar". The
reviewer's own words put it at the bar, but it was called minor and the
head was named clean under "merge everything". It merged with the issue
open, and #476 fixed it. The bar is the reviewer's grading until the
artifact is checked and shows otherwise. A merge authorization is not a
reason to read a finding down (2026-10-01).

**Every check failing at once, with zero steps, is the runners, not the
code.** #446's first CI run showed 13 jobs failed, rustfmt included,
which had passed locally. Each job's annotation read "The job was not
started because it repeatedly failed to be acquired (5 attempts)". The steps
list was empty, and `--log-failed` held no error lines. `gh run rerun
<id> --failed` passed everything. Read a failed job's steps and annotations
before its diff (`gh api repos/<o>/<r>/check-runs/<job>/annotations`). A
check that never started is not a check that failed.

- **`--delete-branch` on someone else's PR can close a third PR stacked on
  it.** Merging #416 with the flag deleted `docs/handoff-408`, which was
  #417's base; GitHub logged `base_ref_deleted` and closed #417 in the same
  second (reopened 12 s later by the session that owned it). Two sessions had
  also merged #416 in that same second — every session acts as the same
  GitHub user, so the event log cannot say which. Before any
  `--delete-branch`, run `gh pr list --base <branch>`; on a peer's PR, leave
  the flag off and tell the peer before merging.
- **A timeout wrapped around a future drops the future's own cleanup.**
  #406 bounded a page's layout read with `tokio::time::timeout`, which on
  expiry dropped `read_page` — and with it the reset of a worker left
  mid-exchange, so the next page could have read this page's boxes and
  cached wrong crops as valid. The reset now lives in the timeout arm. When
  you add an outer bound, move the inner failure path's state repair into
  the bound's own arm.

- **A source-scan test must slice the function it guards, not cut at the first
  marker.** #376's `an_incognito_key_never_reaches_the_journal` cut
  `voice/mod.rs` at its first `#[cfg(test)]`, above `echo_span_tests`, read
  none of the lines it existed to pin, and passed with the key logged raw. It
  was that file's fourth source test to match the wrong region. It now slices
  `completion`'s body and requires the two refusal paths to be found, so it
  cannot pass on nothing.
- **Merging a sibling PR can leave the other DIRTY, and a DIRTY PR gets no
  review runs.** #385 sat unreviewed until main was merged into it. After
  merging one of a pair, check the other's `mergeStateStatus`.

- **Twelve defects in six review passes, all but the last two in readers
  rather than in the change itself.** #238 moved the date out of the system
  prompt, and almost every finding was a *reader* that had silently come to
  depend on it being there: a prompt
  equality gate (`probe::prepare_mismatch`), a summariser's input
  (`compact::render_for_summary`), an eval judge's evidence
  (`eval::grounding_evidence`), two intervention locators
  (`learning::locate_followup`, `counterfactual::locate_steer`), a replay
  site nobody had listed (`mismatch::drive`). The general lesson:
  **removing a value from a shared slot breaks every reader that was quietly
  reading it, and `grep` finds the ones that name it, not the ones that
  inherited it** — the system prompt was not only carrying the date, it was
  carrying it *to exactly the set of roles that had one*, so overwriting it
  was how a narrowed role (gossip's readers, `vet`, every subagent) opted
  out. Deleting the value deleted the opt-out mechanism with it. Two of the
  last four findings were consequences of fixes made earlier in the same
  review, which is the argument for running the loop to a clean pass rather
  than stopping when the list looks handled. And one finding was a prompt
  rewrite made for usability that widened what the model would accept a date
  *from*: the wording it replaced was bad for the owner and load-bearing
  against content, because `Role::User` is a message role and not a party.
- **A fix verified one layer below the store that short-circuits on "nothing
  changed".** #246's `--deadline none` clear was added to `apply_correction`
  and tested there, green — and `Store::correct` returned before `put`
  whenever no correction was recorded, which is by design the agreeing case,
  so the owner settled a finding on a copy the early return dropped and
  `mail list` showed the label on every run after. The unit test could not
  see the layer with the bug; the reviewer found it by reading the store.
  The general lesson: **when a change's whole point is a mutation the layer
  above does not count as a change, the test belongs at the layer that
  persists** — put a record in the store, correct it, reload it, and make
  the test fail on the early return. One PR earlier (#244) the same shape
  wore different clothes: a walk that yielded only calls with a surviving
  result made the outbox's staging-call `break` conditional on that call's
  own result, and *a break that is conditional on the thing it guards is a
  break that stops being there* — list every call, and let the caller decide
  what a missing result means. Eight passes over three PRs found nothing in
  the containment check itself; every defect was a seam — a reason collapsed
  into a label, a surface still saying "found none", a mutation that never
  reached disk. The primitive is the easy part.
- **A fixture that exercises nothing passes the assertion anyway.** Twice in
  one review. A test written to prove the overflow-recovery arm re-folds the
  date compacted nothing at all — five messages, and `worth_compacting`
  requires a cut past `MIN_DROPPED` — so it asserted the calendar and drove
  none of the code; `compact_keep_recent = 1` then bit the same way, because
  the target is `len - 1`, which splits a `tool_use` from its result and
  leaves `cut_point` nothing safe. Earlier in the same review,
  `a_compaction_summary_is_not_the_owner_speaking` had been green for years
  over an untrimmed `starts_with` bug, because it reads through
  `owner_turns`, which trims at its own call site. The lesson: **a green test
  over a fixture that did nothing is indistinguishable from a green test over
  one that worked.** Assert the precondition the case depends on — that a
  summary actually landed, that the predicate was called on the bytes the
  writer really produces — or the test is measuring its own fixture.
- **Eight defects in ten review passes, every one in the half of a
  protocol the tests never drove.** #226's pause machinery has two sides
  that talk — the page's `Pauses` and the worker's `LinkWatch` — and the
  unit tests drove each alone while the end-to-end smoke drove the
  worker-first ordering, which happened to be the one that worked. The
  page-first ordering (a mic mute, the case the feature was for) deadlocked
  on the first push, and the five passes after it each found the next
  latch in the recovery path: a settle one packet could lift, a lost `ok`
  on either side, a hold expiring on a muted track's silence, a hold pinned
  to the first of two reasons. The general lesson: **when two parties hold
  state about each other, test the protocol in both orderings before
  either party's logic** — the taking of a hold is the easy half, and every
  bug was in how it came off. A pure, exported decision function beside the
  I/O (`linkVerdict`, `serverPauseExpired`, `_judge`) is what let the
  churn be tested at all.
- **The same fail-open gate, re-hit twice in one night with the record of it
  open in the next paragraph.** On 2026-09-07 two pushes went out that did
  not build — a value moved into a loop and read after it, and a function
  referenced before its file had been written — because the gate was
  `set -o pipefail; cargo test … | grep …; git add && git commit && git
  push`: `pipefail` made the *test pipeline's* status honest and the
  semicolon threw it away, so the commit ran regardless. Each cost a
  review pass and a fixing commit whose message had to say so. The general
  lesson is the one below, restated for the shape that slipped past it:
  **a gate is only the chain from the first check to the push, joined by
  `&&` end to end** — one `;` anywhere in it is a gate that reports and
  does not gate. The night's other lesson is about review at scale: a
  guard bound at *resolve* time only (the parent type check on
  `create_task`) drew seven passes of "and this writer re-creates the
  state" — the id writer, retype, merge, upsert's conflict clause, the
  survey's inner join, the raw column beside the joined name — until the
  rule was restated as *the row is the fact* and bound to every writer.
  Bind an invariant to the writers, not the parser, on the first pass;
  each writer found later is a pass.

- **A new scope key has five doors, and each fails closed on its own —
  or a downgrade opens the one you skipped.** The surface key (2026-09-07)
  drew fifteen review passes, the first fourteen each with something at
  the bar and the fifteenth clean (counted from the PR's comment record),
  and the findings were all on one shape: the value has to fail closed at construction
  (`Situation::recorded`/`of_run`), at deserialization (a stored value the
  build cannot name — parked verbatim, matching nothing, never `None`,
  which on a scope is *every* surface), on the run record (what `prepare`
  matched, never what the session was), in the reconcile (a key any
  attach presented is kept; a parked key is replaced only by a key some
  attach names; a record that reads a newer value as none must not clear
  it), and at the startup warning (`unloadable_rules` names it). Then walk
  a downgrade through all five. The workspace key had the same doors but
  its value is a path, which no build fails to name — the surface is the
  first key whose closed set can grow, and that is what made it fourteen
  passes with a finding. Close all five on the first pass; each door
  found later is a pass.

- **A PR based on another PR's branch can stop getting review runs, and
  the silence reads like a clean pass.** #211 was opened against
  `feat/surface-scope` so its diff showed only its own change; its first
  pushes were reviewed (3 passes), then three further pushes, one a
  force-push, produced no workflow run at all (`gh api
  repos/…/actions/runs?head_sha=…` empty) while the same workflow ran on
  every other branch. Retargeting it to `main` started a run within a
  minute, and 5 passes followed. Base a PR on `main` and accept the
  wider diff — and when a push draws no run, read it as absence of
  evidence, never as nothing to find.

- **A gate is the exit status of every step, the patch script included.**
  Twice in one evening a commit went out without the change its message
  named: once because `cargo test | grep "^test result" | grep -vq "0
  failed"` passes when the tests fail to *compile* (no result lines, so
  nothing to reject), and once because a Python heredoc that edits the
  tree was followed on the next line by `cargo fmt && …` — the heredoc's
  failure was dropped at the newline and the chain ran on an unpatched
  tree. Check `$?` of each command, and start the `&&` chain at the edit.

- **A docs-only push draws a full-diff pass, and it found what
  twenty-one clean passes had not.** On 2026-09-07 mecha PR #206 had
  twenty-one consecutive passes clean at the bar (05:20Z–13:26Z, counted
  from its 33 summary comments after the last medium at 05:12Z); a
  handoff-only commit (`1ec25cfd`) then drew a medium — the landing
  page's `[[mcp]]` snippet, made copy-pasteable by the rename, lacked the
  `untrusted_input = true` override that arms the interlock over the
  graph — and the fix push after it (`4542e735`, pushed together with a
  handoff commit, so not docs-only) drew the `meta.goal` asymmetry (the
  key vanished where each error's `goal` fell back to the kind word).
  Both had been in the diff the whole time. The lesson: the reviewer
  reading a large diff cold is a different instrument from the one
  reading an incremental push, and "the last pass was clean" is a
  statement about that push, not the branch. A cheap docs-only push
  before merge buys one more cold read; and every push, however small,
  is a pass to read. The entry's own first draft said "ten" and
  "thirteenth" from memory; the record said twenty-one and thirty-third
  — the count is the evidence, so measure it before typing the sentence.

- **Sixteen review passes on one PR, and four of them were the author's own
  stale tests, a format miss and a lint nit pushed without gating.** The
  meeting-poll PRs (2026-09-04/05) were large — ~4,500 lines, a file three
  binaries write on a timer, a new outbox field, a state machine — and the
  workflow re-reads the whole diff on every push, so each pass found real
  things in the new code the last fix added (the auto-release hole in
  `staged_since` came on pass eleven). That part is the review working.
  The part that was not: three pushes went out with a test assertion the
  author had not updated after changing the behaviour it tested, and one
  with a `cargo fmt` miss — each a full extra cycle of a reviewer's time.
  **Gate every push on the suite *and* clippy in the same command that
  commits** (`cargo test && cargo clippy --all-targets -- -D warnings &&
  git commit …`); a push that fails CI is a review pass spent on nothing —
  and the gate this lane actually used, `cargo test … | grep -q FAILED &&
  echo NOT GREEN || git commit …`, fails *open*: it runs no clippy, and a
  compile error goes to stderr, so a tree that does not build prints no
  `FAILED` and the commit lands. Every stage load-bearing on its exit
  status, or it is not a gate. And when a fix adds a *state* — a stall, a
  handoff, a merge — expect the next pass to find its edges, because it
  will.

- **Everything after clap's `--` is a positional, so a fix for argv
  injection broke reject completely — under a suite that stayed green
  because nothing ever spawned the child.** #126's separator fix put
  `--reason` *after* the `--` that guards a URL-supplied id, and clap
  parsed it as an unexpected second positional: every reject on the two
  stores that *require* a reason exited non-zero, 100% of the time, while
  1,986 tests passed — the argv lived inline in an async handler, so no
  test could reach it and none spawned a real child. The repair moved
  composition into a pure `decide_argv` (flags first, separator, then the
  id), added shape tests, and ran one real reject through the handler
  against a fabricated candidate in a scratch `MECHA_HOME`. Three review
  waves that evening each caught what the previous wave's fix introduced.
  Two general shapes: **flags go before the separator — `--` ends option
  parsing for everything after it; and argv composed inside a handler is
  argv no test can see — hoist it pure, then spawn one real child, because
  a mutating path verified only by reading is unverified.**
- **A PR can merge "successfully" into the wrong branch, and every signal
  reads green while it does.** Landing #112 (2026-08-29): `gh pr edit 112
  --base main` failed with a GraphQL error that scrolled past as a
  Projects-classic deprecation notice, so the retarget never happened —
  and `gh pr merge` then merged the PR, state MERGED, into its *original*
  base branch, twenty seconds after that branch itself had merged to main
  as #111. Checks green, PR closed, `origin/main` silently missing the
  entire change; the deploy under way would have shipped a binary without
  the closure guard while `mecha tools` and the suite on the merge branch
  both read current. Caught by pinning the deploy worktree to
  `origin/main` and noticing the tip was #111's commit, confirmed
  independently by a peer session reading the compare API. The general
  lesson is the skill's own rule pointed at git: **verify the retarget
  from the API before merging, and verify the merge landed on the branch
  you meant from the branch itself** — a merge's success message names the
  PR, not the base you believed it had, and an error dressed as a
  deprecation warning is still an error.

- **A guard's presence check must never read data the guarded side
  supplies.** The closure guard's `wrap` and `verify` first keyed
  "already guarded" off `description().ends_with(GUARD_NOTE)` — and a
  `kg_task_update`'s description arrives over the MCP wire, so a server
  whose description happened (or was crafted) to end with the guard's own
  sentence was left unwrapped *and* passed the startup verification: a
  fail-open in the very check built to make the guard's presence
  structural. The fix put the answer in the type
  (`Tool::guards_closures`, default false, overridden only by the
  in-process wrapper), leaving the description cosmetic. The transferable
  form: any "is the protection installed?" check is itself part of the
  attack surface, and it must key off something the protected party
  cannot author — the same reason `Capabilities` are declared in code
  rather than parsed from a server's self-description.

- **A fix that closes the exact case described leaves the structurally
  identical sibling standing right beside it, four times running on the
  same two-function surface.** #101 (rung 9's episode-tagging/surprise-
  detection review fixes) took four rounds to settle, and every round found
  a gap of the same *shape* as the one the previous round had just closed,
  never a new kind of problem. Round 1 made `mecha distill`'s surprise print
  escape ANSI (`strip_ansi`); round 2 found the same function does not stop
  a bare `\r`/`\n`, which forges or rewrites a rendered line exactly as
  effectively as an escape sequence does, and added
  `strip_ansi_and_controls`; round 3 found `is_control()` — the very check
  the round-2 fix used — is Unicode category Cc only, so U+2028/U+2029
  (forge a line break, the round-2 case again, through a character
  `is_control()` does not name) and bidi overrides (reorder the rendered
  line, the Trojan Source shape) sailed through the same filter untouched;
  round 4 found the fix's own doc comment had misdiagnosed *why* it was
  needed — a claimed precondition ("the caller already splits at `\n`")
  that was never actually true even at `strip_ansi`'s two pre-existing call
  sites, exposing a real, unrelated bug in the TUI's own log capture. The
  store-read half of the same PR repeated the pattern independently: round
  1 bailed episode tagging on a hard I/O error reading the outbox; round 2
  found `OutboxStore::items` also silently *skips* a merely malformed item
  file and still returns `Ok`, needing a new `items_strict`; round 4 found
  the cause cited for needing it — a half-written file mid-save — was one
  the store's own temp-sibling-and-rename discipline had already ruled out
  structurally, so three rounds had been narrating the wrong mechanism for
  a real fix.

  The general lesson: when a review finds "X doesn't handle case Y," the
  reflex has to be *"what is the general class Y belongs to, and does this
  fix cover the whole class or just Y"* — asked before shipping, not
  rediscovered when the next round finds Z. A fix scoped to the reported
  instance reliably produces another instance of the same class, and each
  round here cost a full review cycle to find what one pass of "what else
  has this shape" would have caught for free. It is the same failure named
  elsewhere in this file one layer up — a cleanup pass scoped to "the
  findings I already know about" answers a narrower question than the one
  that matters (the #89/#94 trap, above) — recurring here one level down,
  inside a single fix rather than across review rounds of a whole PR.

**2026-08-31 — seven review rounds on #128 and five on #130, and after the
first round every finding was a defect the previous round's fix had
introduced.** Worth recording as three distinct shapes rather than one, because
they fail differently.

- **The checked part was not the load-bearing part.** `withoutJudged` rebuilt
  a cached grouping and needed an id-to-statement map to promote a new leader.
  The only thing available was `sample`, whose alignment with `members` is
  *another repository's* serialisation detail — true today, decorative
  everywhere else it is consumed, and nothing in this repo would break if the
  graph started sending the top three by cosine. A person would: the promoted
  id becomes `leader_id`, and a group verdict files on `leader_id` under the
  statement on the card, so a changed mapping shows one candidate's words over
  Accept-all and votes on another. The induction over it was sound and had
  been verified; the base case was in a repo the verification never reached.
  The fix was to stop needing the belief — drop the group rather than re-head
  it — not to document it. **An assumption whose base case lives in another
  repository is not an assumption this repo's tests can hold.**

- **A test that pins a count forbids the correct change.**
  `a_verdict_reaches_listings_other_than_the_one_on_screen` asserted
  `withoutJudged` was called exactly twice. The bug the next round found was a
  *third* install path that was missing the call — so the test written to catch
  that gap would have failed the fix for it. Assert the property (every path
  that reaches the screen is filtered), never the arity.

- **A warning's position is part of whether it exists.** The `/queues` status
  line is a `Paragraph` in a `Rect { height: 1 }` with no `.wrap()`, so it
  clips at 76 columns. A caveat written *after* a 48-character
  statement head, or after a child-controlled `FAILED` line, was simply not on
  the screen — and its absence read as "nothing to report", which is the exact
  failure the caveat existed to prevent, one layer down. Bounded facts first,
  unbounded strings last. The first attempt to test this re-derived the
  geometry (`76`, hand-copied from three constants in `queues.rs`) and would
  have gone green through any change to them; drawing the modal into a
  `TestBackend` and reading the buffer catches both the ordering *and* a
  ten-column indent change in a file the test never names.

  The general lesson across all three: **a review round that finds only what
  the last round touched is not converging, it is following you.** Each fix
  here was correct about the thing reported and created the next finding, and
  what stopped it was not more care per round but changing what the tests were
  about — from arity to property, from arithmetic to rendered artifact, from a
  documented belief to no belief.

**2026-08-31 — an inability is not a request.** A peer session, blocked by its
own worktree guard from writing `docs/OPERATIONS.md`, asked this lane to write
a line into it — phrased as "you can reach that file and I cannot". It was
safe here only by accident: the finding was independently this lane's (the
swap state was read here, and the build was capped at `-j 4` because of it),
so there was an honest reason to write it that did not depend on the request.
Had it been *only* the peer's finding, the correct answer was the human, not
the reachable file. **A guard that stops one lane is not a task that lane can
hand to another, and the reachability of a file is not the authority to write
it.** The phrasing is the peer's own, offered unprompted after the fact; both
sessions had spent the day refusing the same shape in other forms without
noticing it in this one.

**A green check is not evidence a review happened.** On 2026-09-01 the
`Claude PR review` job on a new repo reported SUCCESS and posted nothing,
three separate ways: no credential at all (the action validated, then did
nothing); a PR that *modifies the workflow*, which the action refuses to run
and exits success on; and a review that completed — 39 turns, real model
usage — then took 14 permission denials trying to publish, because the
prompt had been ported without the `gh pr comment` instruction or the
`--allowedTools` grant. A PR was merged on the strength of the first. **The
artifact is the comment, not the tick**: assert a comment exists before
treating a review as done, and note that a workflow-touching PR structurally
cannot review itself and needs a human read.

**A trailing newline is present, non-empty, and invalid.** A
`CLAUDE_CODE_OAUTH_TOKEN` set with `echo` carried a `\n`, passed every "is
it set" check, and was rejected at the first API call — `is_error: true`,
`modelUsage: {}`, 1.9 seconds. Three re-runs looked like three identical
failures. The load-bearing diagnostic was the secret's `updated_at`
timestamp, which distinguished "the new token is also bad" from "the new
token never arrived" — twice it was the latter. **Use `printf '%s'`, and
check the timestamp before re-running anything.**

- **An audit's findings are claims, and get the same check.** Of about eighty
  doc findings from seven read-only lanes, three were wrong against the code
  (overflow recovery does not read `context_window`; `mecha msg` has no
  `read`/`who`; the documents paste strips all whitespace, not just the
  ends). Each fixing lane was told to verify before editing and to report
  skips, which is how they were caught rather than written into the docs.

### Environment

**Durable dispatch checks belong after awaited authorization too (2026-11-06).**
A reservation-time cancellation check could miss a second process cancelling while
approval was pending. Recheck the persisted receipt and complete source closure at
the final tool boundary, without holding a SQLite transaction across I/O. Separately,
`Agent::call_ctx` resets an inherited `program_room`: the first adapter's configured
byte bound did not reach the actual read. A giant-line regression failed until
`BoundRead` applied the bound inside dispatch. Verify constraints where the shared
executor actually consumes them, not only where the caller configures them.


**A sweep pattern proves nothing about what it cannot express** (mecha-5d,
2026-10-03). A grep for first names came back empty over a tree that still
held the surname and the lab. A word-boundary pattern for a name misses it
after a `\n` escape in source, because the escape's `n` is a word
character. The brief handed to a subagent is fixture text too: real stored
persona facts reached a research doc that way (fixed in #533). The names
had come back five weeks after the 2026-08-26 cleanup. Before trusting an
empty sweep, list what the pattern could not have matched.

**Chromium taps encoded audio only on a connection made for it.** Once
Chrome shipped `RTCRtpScriptTransform`, the call page took that branch on a
plain `RTCPeerConnection`. Chromium handed the transform no frames while
RTP flowed, so every Chrome call declared the buffered uplink and fell back
("RTP is arriving and no batch has", the owner's report on 2026-10-03).
Chromium delivers only with `encodedInsertableStreams: true`, and then a
receiver plays nothing unless its frames are piped through. Measured in
Chromium 149: no flag, 0 frames against 150 RTP packets; flag alone, 0
decoded samples at the far end; both, normal. Fixed in #534. A browser
gaining the standard API can break a fallback that worked only because the
API was missing.

**A new CUDA context can fail at init with gigabytes "available".** From
02:43Z to 02:55Z on 2026-10-03, `tts-server` (twice) and the on-demand OCR
server failed at `cudaSetDevice` with `NV_ERR_NO_MEMORY` from
`kgrctxAllocMainCtxBuffer`. `MemAvailable` read 30–42 GB at the time;
processes already running were unaffected. Both started on a retry minutes
later, and the cause is not pinned: no image render overlapped the
failures, and the model switch came after them. An on-demand GPU server
must expect a failed start and retry it, rather than read "available" as
"can start".

**`journalctl -g` exits 1 on no match, the same as a failure.** systemd
255 with `-q -n 1 -g PATTERN` prints nothing and exits 1 when nothing
matches, and exits 1 with a message on stderr when it cannot read. #529
tells them apart by stderr. A stand-in that exits 0 on no match hides
the confusion, so the test's stand-in now answers as the real one does.

**A background wait dies with the worktree it runs in.** A CI watcher
started inside a worktree that was then removed exited with "Unable to
read current working directory" after the forced merge of #528. That read
as a CI failure, and CI was still running. Run long waits from the main
checkout, or remove the worktree only after they finish.

**This box's `grep` is ugrep, which reads a `$` inside a pattern as an
anchor.** A check from a peer, `grep -rl 'XDG_CACHE_HOME:-$HOME/.cache}/…'`,
printed 0 here against the 8 it counts with GNU grep, and nearly read as
the claim being false (2026-10-03). Search for a literal with `-F`; a count
that disagrees with a peer's is a tool difference until shown otherwise.

**On unified memory the model's GPU allocations do not show as RSS, so a
build can take the box down while `top` looks fine.** At 04:10Z on
2026-10-03 a global OOM killed `llama-server` (the :8080 router) in the
middle of a `cargo test --workspace`. The kernel's task table held 16 `ld`
(17.3 GiB) and 22 `rustc` (9.1 GiB), while GPU allocations already held
~75 GB: the model, an idle ComfyUI, two TTS servers, embeddings and OCR.
The router was the victim, not the build, because its unit runs at
`oom_score_adj` 200; systemd restarted it 15 s later on the startup preset.
The lesson: before a build here, read GPU use (`nvidia-smi
--query-compute-apps`) as well as `free`, cap the build (`-j 4`,
`CARGO_BUILD_JOBS=4`), never run two at once, and expect the model, not the
build, to be what dies.

**A tool's error text can arrive at exit 0.** On this machine `nvidia-smi`
prints `[Insufficient Permissions]` or `[Unknown Error]` in a memory field
and still exits 0. Only a literal `[N/A]` means unified memory (the GB10),
so a probe reading the field has to name each value it accepts, not trust
the exit code (mecha-a3, #520).

**A binary's `--version` names the commit it was compiled at, not the
libraries it loads.** `llama-server.prev` reports `a4ce259` while loading
`c841aeeb`'s libraries through its RUNPATH. Ask the loader instead:
`readelf -d ~/.local/bin/llama-server.prev | grep RUNPATH` names
`~/llama.cpp/build/bin`, and the main stub names `~/llama.cpp-next/build/bin`
(mecha-a3; RUNPATHs re-read by mecha-1e, 2026-10-03).

**`args_conflicts_with_subcommands` on a subcommand's args also refuses
global flags.** With it set on `features`, `mecha features --yes enable
graph` broke. Check: `mecha features --yes enable zzz` must answer "is not a
feature", not a flag error. mecha-a3 ran it on the installed `c6f59b05`. No
test holds it yet: `commands/features.rs` guards the regression with a
comment and a hand-rolled `--probe` check, so `cargo test` would stay green
if the attribute came back (#520).

**A serve restart can strand the image model's memory.** `image_generate`'s
ten-minute unload timer lives in the drawing process, so a restart inside
that window left ~12 GiB held (2026-10-02). #515's `comfyui-idle-reset` is
the lasting fix (mecha-a3).

**A shared `CARGO_TARGET_DIR` keeps test binaries that point into removed
worktrees.** Building several worktrees into one target directory reuses a
crate's test binary when its sources match, and `env!("CARGO_MANIFEST_DIR")`
is baked in at compile time. After the worktree that compiled it was
removed, `mecha-mail`'s `docs_server` fixture tests failed only in
`--workspace` runs ("can't open file '…/mecha-drop-farewell/…'"), which read
as flaky. When a test passes alone but fails in the full run, read its
stderr for another worktree's path before calling it flaky; `cargo clean -p
<crate>` clears it (2026-10-01).

**A new authored data file can be silently ignored.** `.gitignore`'s
`*.jsonl` (with re-includes only for `eval/*.jsonl`, `eval/envs/**`,
`eval/suites/*`) swallowed `eval/image-read/cases.jsonl`. `git add -A` added
the 13 pictures and skipped the cases, and a PR shipped fixtures that
nothing used. Review caught it. After adding a data file in a new
directory, run `git check-ignore -v <file>` or look at `git ls-files <dir>`
(2026-09-30, #450).

**`std::process::exit` skips destructors, so a guard in an outer frame
does not survive an inner exit.** The router hold lived in `dispatch`'s
frame, and every command that exited non-zero left its hold file behind.
Five accumulated in an afternoon of evals. A resource released on drop needs
one exit path that releases it (`exit_with`), and a test that keeps bare
exits out (2026-09-30, #453).

**A probe whose own request wakes the thing it probes must outwait the
wake.** mecha-graph's `Embedder::available()` gave up on `/health` after
1.5 s. Behind a socket-activated server, that request is what starts the
model (~4 s), so every cold probe read "absent" and semantic search went
keyword-only without a word — the silent absence the embedding unit was
created to prevent. It was caught before the switch by listing every
consumer of :8081 and reading its timeouts, and fixed in mecha-graph #26
first. **When a service moves to on demand, audit every "is it up?" probe,
not only the request timeouts.**

**CI's toolchain can be newer than the box's.** #406 passed `RUSTFLAGS="-D
warnings" cargo clippy --workspace --all-targets --all-features` on the
box's rustc 1.97.1 and failed CI's 1.98 on a lint the older clippy does not
have (`chunks_exact_to_as_chunks`). **A green local clippy predicts CI's only
on the same toolchain: read CI's result after every push.**

**A filter on a sink someone else owns is thrown away with the sink.**
pipecat's runner calls `logger.remove()` and adds its own `DEBUG` sink after
the voice worker is imported, so a filter on the worker's sink would have
vanished without a word, and the incognito silence with it (#376). The
silence is `logger.disable("pipecat")`, which lives on loguru's core and
survives any sink swap; a test replays the runner's own swap. **Attach a
protection to what you own, not to an object the framework rebuilds.**

**A cleanup that cancels the task which caused it can interrupt the work
that task started.** #386's deadline called `runner.cancel()`, which made
`runner.run()` return, and the `finally` then cancelled the deadline task
while it was still inside that cancel. The fix sets a `fired` flag before
ending the call, and `settle_deadline` awaits a fired task to completion,
raising anything it raised. It cancels only a task that has not fired.

**Two verification tracks behind one word, and clearing the cheap one routes
you onto the expensive one.** Google's console calls both "verification".
*Brand* verification proves you own the domain in the app's home-page URL —
one DNS TXT row, free, and it governs only whether your name and logo render.
*Scope* verification proves you handle other people's data safely, and for a
restricted scope like `gmail.modify` it carries a CASA assessment at
~$540/yr, annual, which resets on any scope change. Fixing the branding
failure lands you on the scope-verification submit page, one button from a
weeks-long paid review the app did not need; the same banner had already
appeared once, on 2026-08-18, on a project with no sensitive or restricted
scopes at all (`docs/DOCS-RESEARCH.md` §6.2). Two lessons, and the second is
the one that cost the hour: **when one word names two processes with
different prices, read the artifact that distinguishes them rather than the
banner that merges them** — here the Verification Center's two cards, never
the banner above them. And **an error message names the state, not the
place**: the branding failure is raised in the Cloud console, but nothing
fixes it there, because domain ownership lives in Search Console, a different
product the error never mentions.

**A value with a daily lifetime, rendered once into a process that runs for
days.** The date was computed by `prepare_tools` and frozen into
`Agent::system`, which is exactly right for `mecha run` and wrong for
`mecha serve`, which holds one `Arc<Agent>` until someone restarts it. A
daemon started at 22:37 EDT told a 09:21 voice call the next morning it was
still yesterday, queried the calendar for that day, and read the wrong
schedule back — internally consistent, so it read as correct. The comment
above the code had already conceded the hazard ("the one part of the cached
prefix that legitimately changes daily") under an unstated assumption that a
process lives less than a day. The general lesson: **a refresh cadence is a
property nobody maintains.** When a fact's lifetime is shorter than its
holder's, do not schedule a refresh — move the fact to a place the protocol
already re-pays, so freshness is structural and staleness has nowhere to
live. The corollary for diagnosis: the control that isolates it is the
surface that *does* get it right (here the trigger runner, which builds an
agent per run), not the one that is wrong.

**A stats counter is a witness only under the condition you rely on.** The
uplink's deafness watch first used aiortc's `inbound-rtp.packetsReceived`
to prove RTP was arriving while no batch was; on a track nothing read it
stayed at zero for the life of the call — aiortc counts packets on
consumption, not arrival — and the watch never fired. Read the track and
discard it, and a frame arriving is the witness. Before trusting a counter,
make it move under exactly the condition the check depends on.

**`pkill -f <pattern>` matches the shell running it.** Twice in one
session a `pkill -f "worker.py --port 7861"` killed the `bash -c` whose own
command line contained the pattern, taking the rest of the chain with it
and reading as a mysterious exit 144. Anchor on the interpreter path
(`pkill -f "^/path/to/python worker.py …"`), or kill by port, or iterate
`pgrep -f` and `kill` the pids.

**An executable path can be right while its bytes belong to another tree.**
During the 2026-09-08 lifecycle checks, integration tests launched Cargo's
named binary from a shared target directory, but tracing showed the older
`docker run` path instead of the branch's create/start path. A library test
filter also found zero tests while the named test existed in source. Final
validation moved to a target directory dedicated to this worktree. The lesson:
sharing build outputs across worktrees weakens artifact identity; require the
expected test to run and verify the executable's behavior before trusting a
passing or failing result.

**A detached cleanup thread does not outlive the process.** The Docker MCP
lifetime fix initially delegated all removal work to a new thread. A one-shot
CLI can return before that thread is scheduled, so no removal command runs.
`DockerContainer::drop` now launches the cleanup subprocess synchronously and
delegates only waiting and retries. The general rule: when cleanup must survive
normal process exit, establish its independent owner before returning; a
scheduled task is not ownership. Creation has the inverse race: removing before
the daemon finishes creating can miss a late resource. The creation worker must
own that result even after its caller abandons it.

**2026-09-05 — user units need PATH without a login.** The box rebooted at
06:15Z and nobody logged in at the console. A user unit's PATH is the
manager's, and the manager only gains `~/.local/bin` when a graphical login
imports it, so `llama-local` and `llama-embed` restarted every ten seconds
for eight hours on `llama-server: not found` (2,951 restarts) while the
binary sat in `~/.local/bin` the whole time; both nightly learning passes
deferred on "model server not answering" and the 11:00Z `morning` trigger
failed on transport. Fixed at 14:44Z with `*.service.d/path.conf` drop-ins
naming the path, so the service no longer depends on someone sitting at
the machine. The same reboot left a second, quieter casualty that was
found only the next night: see the 2026-09-06 web-app entry in
`HANDOFF.md`'s machine state — `mecha-serve` started by absolute path and
only its bare `mecha-graph` children failed.

**2026-09-03 — a hash without its algorithm is a number, and a guard that
prints is not a guard.** Deploying the approval-rules arc, the shared
checkout `~/Github/mecha` (a live unit's `WorkingDirectory`, another unit's
literal `ExecStart` script) had to move from a merged feature branch back to
`main`. The safety check this file's environment section asks for — the
launch script `scripts/start-moe-mtp.sh` unchanged across the switch — was
run as a git blob id, `d76da36c`, and a peer re-checking it with `sha256sum`
got `c08891fb` and nearly read "the file changed" off a different digest of
the same bytes, which would have restarted `llama-local` for nothing. **Name
the digest when you write a hash down**, or use the algorithm-free form
(`git diff --quiet A B -- path`). The recipe recorded for the switch then
went through review pass after review pass: it checked the script *after* the switch
(once switched, a changed script is already the next launch command), then
its checks informed rather than stopped (`diff --quiet && echo` is silent on
the case it exists for) while its discard of a dirty file was unconditional
in a checkout many sessions share, then it `pull`ed, which re-fetches past
the ref the checks had verified; then it switched onto stale local `main`
before merging — a ref nothing had checked, and a place a refused ff-merge
would have stranded the tree — and let the human read be the only gate on a
tree a peer could dirty meanwhile. **A runnable block in a handoff is what
gets copied, so its guards must halt the chain, its destructive step must
sit behind a human read *and* a re-run of the mechanical check, and it must
land on the ref it checked in one hop, and the proof must be about the ref
that moves — one pass found the fast-forward proved for
`HEAD` while `switch -C` reset `main`, and the next found the discard
running ahead of the one step most likely to refuse.** The switch itself closed the right
way: the deploying session could not run git against the shared tree and
relayed the owner's instruction to the session working there, which declined
it — a peer's report of the owner's word is the shape a permission gate
exists to refuse — and ran it only once its own user said so in its own
session. The one dirty file that had made the owner's first `switch` refuse
silently was a duplicate of a row `main` already carried.

**2026-08-30 — this box's clippy is 1.97, CI's is 1.98, and an `async fn`
hides its `Result` from the older one.** `result_large_err` cannot see
through the future wrapper on 1.97, so a `Result<String, Response>` return
lint that fails CI's `-D warnings` build reports nothing locally — PR
#126's clippy job ran red while every local check was green, and the
finding surfaced only in `gh run view --log-failed`. The 1.98 toolchain
was already installed here: `cargo +1.98.0 clippy --workspace
--all-targets --all-features` reproduces CI exactly and is now the
pre-push gate. General shape: **a local lint's green is evidence only at
CI's own toolchain version — pin the invocation, not the habit.**

**2026-08-30 — the export gate refused the whole public release over
fixture names, thirteen files deep.** Test fixtures and doc comments in
the private graph repo had accumulated real names — family, colleagues,
the university — since the previous export, each legal where it sat and
all of it unpublishable, including one written *that same morning* by a
session that knew the no-real-people rule from the public repo and did
not apply it in the private one. The strip cost an evening pass and two
broken substring tests. General shape: **fixture rules follow where the
code is going, not where it sits — a private repo with a public export is
a public repo with a delay, so write the fictional cast from the first
line.** (The gate's tree-destroying design also proved itself twice: a
`tail -3` on the export truncated the hit list to one term and briefly
misread a 13-term refusal as a one-term residue — the gate's own header
records why a gate that only *reports* trusts every caller to check.)

**2026-08-30 — `node_modules/` with a trailing slash ignores a directory,
not a symlink wearing its name.** A worktree lane symlinked
`web/node_modules` at the primary checkout's copy to skip an `npm ci`,
and `git add -A` swept the symlink into a commit — both `.gitignore`s say
`node_modules/`, and the trailing slash makes the pattern match
directories only, which a symlink is not. It rode a merge onto public
`main` (`0b976c1` → #120) carrying an absolute path into this machine's
home, and announced itself only as a perpetually-deleted file in `git
status` after the next `npm ci`. Removed, and the ignore hardened to
`node_modules` (no slash) so the next shortcut stays untracked. General
shape: **a gitignore audit that only checks the pattern exists can still
track the thing — the trailing-slash rule quietly scopes the pattern to
one file type, and `git add -A` in a worktree stages whatever slips
through.**

**2026-08-30 — a cancelled CI run is neither a pass nor a fail, and this
class of loss leaves a green-looking history.** The docs workflow's
pull-request builds and main's Pages deploy shared one `pages` concurrency
group, and GitHub holds only one *pending* run per group — so a PR build
evicted the pending deploy of PR #118 (run 33256024329, created 13:50:09Z
on 2026-08-29, zero jobs run, cancelled the second the next PR run was
created; verified against the run's own API record). Nothing red anywhere:
the deploy simply never happened, and the site stayed one PR stale behind a
green checklist. PR #123 (another lane's, docs.yml only) serialises deploys
against each other and gives PR builds per-branch groups. General shape:
**an eviction shows up nowhere you normally look — when a deploy's absence
is the symptom, search the workflow history for `cancelled` runs, not for
failures.**

**2026-08-30 — a rerun measured the code from before the fix, and the log's
own symptom was what caught it.** After editing the replay wrapper,
`cargo test -p mecha-core --lib` rebuilt the library and its tests — and left
`target/debug/mecha` exactly as it was, so the "verification" rerun measured
the unfixed binary for half an hour. Nothing in the invocation looked wrong;
the tell was that the warning the fix should have removed was still being
printed. Two general shapes: **a package-scoped test build is not a binary
build** (`cargo test -p X --lib` proves the lib, not the bin you are about
to run), and **after a fix, grep the next run's output for the symptom** —
its absence is the cheapest artifact-level proof there is, and its presence
is unfakeable.

**2026-08-30 — a gitignored file does not exist in a worktree, so the rule
"route specifics to OPERATIONS.md" silently forks it.** `docs/OPERATIONS.md`
is gitignored (`.gitignore`, confirmed via `git check-ignore -v`) and
`git worktree add` carries only tracked files — a lane following the
inventory-split rule from a worktree writes a second, divergent copy the
real file never learns about. General shape: **a convention that names a
gitignored path binds only the main checkout**; a worktree lane hands the
fact to whoever sits there. (Found by the perf lane, verified from the main
checkout the same hour.)

**2026-08-29 — deleting the A records would have left browsers on the old
page while every terminal check looked right.** Squarespace's DNS preset
carried an **HTTPS (SVCB) record** beside the A records, with an
`ipv4hint` naming all four Squarespace addresses. Modern browsers consult
the HTTPS record and can connect from its hints; `dig` and `curl` do not
unless asked (`dig HTTPS`). So repointing only the A records would have
produced a state where `dig A` and `curl` proved the migration done while
Chrome and Safari kept landing on the Squarespace page — two truthful
instruments disagreeing because they read different record types. Deleting
the whole preset *group* is what removes the SVCB record. Two general
lessons: **check the record types your clients actually consult, not the
ones your tools default to**; and after any DNS change, the authority and
every cache disagree for a full TTL — the first post-change check must be
against the authoritative nameserver (`dig @nsd1…`), because a cached
answer that contradicts a fresh claim is evidence about the cache, not the
claim (this pass hit exactly that: the local resolver held the four old A
records with an hour of TTL left while the authority already answered the
droplet).

**An edit tool that silently does nothing shipped three dead features in one
day.** Patches applied with python's `str.replace` return the string
*unchanged* when the pattern does not match, and the pattern drifts the moment
`cargo fmt` reflows a line. Asserting on some patches and not others meant
three landed as no-ops: a forensic probe swept into a commit, a TUI border
title that never gained the `m merge` key it advertised, and an `Enter`
handler that stayed `=> {}` while the footer said "Enter read it". **Every one
compiled and passed its tests, because a patch that does nothing breaks
nothing.** Assert that the pattern matched before writing, always — a
silently-skipped edit is indistinguishable from a finished one, and the tests
that would catch it are the ones nobody writes for code they believe exists.

**And the compiler had been saying so, past a reviewer counting warnings
instead of reading them.** `field 'show' is never read` sat in every build
between introducing that handler and fixing it. The check being run was
`clippy | grep -c warning`, compared against a known baseline — which answers
*"did I add a warning"* and never *"what is this warning saying"*. A count
turns a diagnostic into a regression test and throws away the diagnosis. Read
the text of anything new; compare counts only to decide whether to look. Two
independent signals said it, not one: `review_detail` was assigned `None` in
three places and `Some` in none, which is the same fact stated in the source
rather than in the build output. Neither was read until someone went looking
for a cause, which is the tell — both were *available*, and availability is
not the same as being consumed.

**A correct filter over a truncated input, and neither half looked wrong.**
`resolve_entity_all`'s fuzzy tier took `LIMIT 5` with no ordering, so against
13,400 event nodes a person was never reached; a surname search returned five
events and no people. The rule directly below it — *drop retrieval-target
types when something stronger matched* — was working perfectly and had nothing
to drop to, because the LIMIT had spent every slot before a person got in.
Ordering the query made the existing rule *reachable* rather than replacing
it. When a filter appears to be failing, check what reached it: a component
that is right about the wrong input reads as correct in isolation, and so does
the one that never sees the case.

**A maintenance verb whose `--dry-run` defaults to false rewrote seven nodes
for someone running it as a survey.** `mecha-graph fix-person-names` was
invoked to *see* what it would do. The renames were all correct and nothing
was lost, which is luck rather than design. For any pass that can rewrite in
bulk, reporting is the default and `--apply` is the opt-in — the inverse
costs nothing when someone means it and costs a restore when they do not.

**A live config edit shipped before the schema was installed took down every
older binary at once.** Adding `[web]` to `~/.mecha/config.toml` while only a
branch build understood it made config parsing fail for the installed
`mecha` behind the Slack connector and the trigger daemon — `ConfigLayer` is
`deny_unknown_fields`, so an unknown section is a startup error, which is the
right strictness pointed the wrong way in time. The order is: merge the
schema, reinstall every binary that reads the file, and only then point the
live config at it. A section a sibling binary cannot parse is an outage, not
a preference.

**Production pointed into a session worktree, three separate times.** The web
assets path, the voice worker's WorkingDirectory, and the tailscale page
mount all named `…/.claude/worktrees/<session>/…` — each worked on the day it
was set and each would have broken silently when the session directory was
cleaned. Production paths must survive the session that created them:
build artifacts get a stable home (`~/.mecha/web/dist`), units point at the
main checkout, and the deploy skill carries the copy step so nobody
remembers it.

**A facade announced its port before binding it.** The voice facade printed
"voice facade on 8990" while another process held the port; the bind failed
in an unread task. Announce after the bind answers, or fail loudly — a
startup line describing an intention as a fact is the silently-degrading
shape in one sentence.


**A secret in `~/.bashrc` is invisible to every systemd unit.** `EXA_API_KEY`
exported there worked perfectly in every hand-test and would have reached no
scheduled or connector run at all — and those are where unattended web search
actually happens. `systemd --user` reads `~/.config/environment.d/`, and
nothing else; `.bashrc` additionally returns early for non-interactive shells,
so even a script would have missed it. Found by reading
`/proc/<pid>/environ` of each service rather than by testing in the shell that
had just exported it. The same audit showed `ANTHROPIC_API_KEY` had *never*
been visible to any unit, harmless only because every unit runs a local
provider. **A secret's location decides who can read it, and the only proof is
reading it back from the process that needs it — never from the shell you set
it in.**

**A key pasted into `api_key_env` fails silently and leaks quietly.** The field
names an *environment variable*; given a key it dutifully looked up a variable
called `b0182b45-…`, found nothing, and reported `exa: no API key` — a message
that describes a missing key rather than a misplaced one. The key meanwhile sat
in plaintext in a mode-664 file. The mistake is invited by the name: `api_key`
and `api_key_env` differ by a suffix and accept the same-looking string.
**A config field whose value is a name rather than a datum should say so in the
error when what it got was obviously a datum** — the diagnosis was two minutes
of reading and would have been zero.

**An install is not a restart, for anything already running.** `cargo install`
replaced `~/.cargo/bin/mecha` while a TUI session begun two hours earlier kept
executing the inode it was launched from — `/proc/<pid>/exe` reads
`mecha (deleted)`, which is the only visible sign. The session had none of the
new code and looked like the feature failing. The check that catches a stale
*process* is not the one that catches a stale *file*: walk `/proc/*/exe`, never
`pgrep` for the install path, because a binary launched off `$PATH` has argv
`mecha tui` and the first version of that check missed exactly the interactive
session it was written for. **A check must be run against the case that
motivated it before it is trusted.**

**A version string is not evidence when the change did not bump one.** The same
install left `mecha --version` reading `0.1.9` before and after, which is
indistinguishable from having skipped it entirely. What proved it was a
behaviour the change introduced (`mecha run --help` carrying `--image`) and
cargo's own line reporting it had replaced an install made from a *different
worktree*. **Verify a behaviour, not a proxy for one.**

**Adding a config key breaks every binary that predates it, including one
already running.** `ProviderConfig` carries `deny_unknown_fields`, deliberately,
so a typo'd setting fails loudly rather than being ignored — which makes
`~/.mecha/config.toml` a *wire format between versions*, not just a type, the
same lesson `Proposed` and `OutboxKind` already carry one layer down. The
failure is deferred and partial, which is what makes it expensive: the old
process parsed the config at startup before the edit and kept working, then
failed hours later on a path that *re-reads* config — here `show_file`, which
loads the whole global config at call time for one number — and reported a
config parse error in a subsystem with no visible connection to what changed.
The model went off reading `config.toml` to investigate. **Install before
editing config, and restart anything long-lived after.**

**`--mmproj-auto` is enabled by default and does nothing where it is needed.**
It only fires for `-hf` downloads, and every start script here uses
`-m <path>`, so a multimodal model was served with no vision tower for months
while the flag list looked handled. Nothing errored: the server started,
answered well, reported `modalities.vision: false`, and the model told anyone
who sent it a screenshot that it could not see images. **A default that is a
no-op in your configuration is worse than an absent one — it reads as
handled.**


- **A working copy can be a running service's `ExecStart`, and then every git
  reflex is a deploy.** `~/.config/systemd/user/llama-local.service` names
  `/home/ljchang/Github/mecha/scripts/start-moe-mtp.sh` literally — not a copy,
  not an installed path, the file in the development checkout. So `git stash`,
  `git checkout --` and `git restore` on that path silently rewrite the launch
  command of an active unit, and the damage does not surface until the next
  restart, when the server comes back healthy and merely behaves differently.
  Found on 2026-08-20 with three arcs sharing the checkout, only because one of
  them asked the other two what their dirty files were. **The lesson that
  transfers is about the standard advice, not about the file**: "work in a
  worktree" assumes nothing outside the repository points at a path *inside*
  it. Here systemd does, so relocating that arc would have broken the running
  server rather than protected it — the advice in CONTRIBUTING.md has this hole
  and the checkout is the thing to check before following it.

- **The fact was written down correctly in the file nobody consults, and wrongly
  in the two that everybody does.** `scripts/start-moe-mtp.sh` has said since
  it was written that llama-server "silently splits `-c` across" its parallel
  slots — the exact `context_window = -c / -np` relationship. Meanwhile
  `docs/HANDOFF.md`'s Environment row stated `context_window` (= `-c`) and
  CLAUDE.md's Context section says the same. The script's own strategy was to
  keep `-np 1` *so that* the wrong rule would stay accidentally true, which is
  a rail that works right up until someone changes `-np` for throughput —
  which is what happened. Nothing broke, because the new geometry
  (`-c 1048576 -np 4`) lands on the same 262144 per slot; the rule was still
  wrong and four derived numbers trusted it. **A fact recorded in one place and
  contradicted in two is not recorded**, and the place it survives is rarely
  the place a reader looks. **Nothing verifies the relationship as of
  `d72e82d`** — `git grep -nEi 'n_ctx_slot|total_slots|/props' -- '*.rs'`
  returns nothing on `main`. The shape a fix would take is argued in
  HANDOFF under *Cheap, and worth doing first*; it is unbuilt, and this entry
  records the trap rather than a remedy.

- **A design written into the record of what happened will be read as having
  happened.** The paragraph above originally ended with the argument for a
  startup check — a good argument, in past tense, in a file that is past tense
  by construction. Within an hour a second session had read the commit that
  added it as a shipped feature and stood down from the area to avoid duplicate
  work, and a third had been told the same. Nothing was built, and the absence
  was now defended by two people believing it was present. HISTORY cannot hold
  a proposal, because there is no wording that survives being skimmed in a file
  whose every other paragraph describes something that exists; the open-work
  document is where a proposal reads correctly. The general shape, and it is a
  sibling of the entry above rather than the same one: that was *a fact
  contradicted elsewhere is not recorded*; this is **a proposal filed under
  completed work is indistinguishable from completed work**. Both fail the same
  way — silently, and with the reader confident.

- **A heredoc can eat a line continuation before the compiler sees it.** Five
  doctor messages written through a Python heredoc lost their `\`-continuations
  to the *heredoc's* parser, so the string literals reached the file as plain
  multi-line text and `cargo fmt` joined them with their indentation intact —
  twenty-space gaps mid-sentence in everything doctor printed. No test failed;
  one line of real output made it obvious. Run the thing whose output is prose,
  and do not trust that what you typed is what landed in the file.

- **`cargo install` will happily install a version older than the one you just
  published**, because the local registry index is cached and nothing warns.
  Minutes after `mecha-factory-publish` 0.2.1 and `mecha-mail` 0.1.1 went up,
  `cargo install` fetched 0.2.0 and 0.1.0 — and reported success both times.
  It was caught only by reading `cargo install --list` against the versions
  that were meant to land. **After installing something you just released,
  verify the version rather than the exit code**, or pin it with `--version`.
  The failure is silent and looks exactly like success, which is the whole
  problem.

- **Publishing a workspace is ordered, and a missing member fails it halfway.**
  `mecha-cli` depends on `mecha-slack`, and cargo refuses to publish a crate
  whose non-dev dependencies are not on the registry — so tagging v0.1.1 with
  `mecha-slack` absent from the release workflow's list would have published
  `mecha-core` and `mecha-mail`, then died on `mecha-cli`, leaving a version
  that can be yanked but never unpublished. Caught by `cargo publish
  --dry-run` before the tag existed. **A new workspace member that anything
  published depends on belongs in the release list in the same change that
  introduces it**, and a dry run is the cheap way to find out.

- A `grep -o 'mk_slt_[A-Za-z0-9_-]*'` clipped a freshly minted key to 24 of
  its 88 bytes, and the truncated credential *looked* installed. The tell was
  indirect: `/v1/health` answers anonymous callers too, so the broken key
  produced a valid-looking 200 with the authenticated fields silently
  absent. Prove a just-installed credential against an endpoint that *only*
  answers authenticated callers — an endpoint serving both cannot fail
  loudly.


- **A magic link spent on GET is spent by the mail scanner, not the
  person.** The first real sign-in from a university mailbox arrived already
  dead: Microsoft Safe Links fetches a mail's URLs on delivery, and both the
  sign-in and form-verification links redeemed on GET — so the robot's fetch
  consumed them seconds before the human's click, and worse, robots were
  verifying submissions no human had confirmed. The fix is the shape every
  magic-link system eventually grows: GET renders one button and touches no
  token at all (not even a peek — a page that varied on token state would
  hand the scanner an oracle), and only the POST spends. **Any single-use
  URL that will travel through institutional email must not have side
  effects on GET**, and the test to write is that the interstitial answers
  byte-identically for a live, spent, and invented token alike.
- **A Docusaurus admonition with a title renders as plain text if you use the
  old syntax, and never says so.** Docusaurus 3 takes the title in brackets
  (`:::note[Title]`); the v2 spelling `:::note Title` does not error, it falls
  through the directive parser and ships as an ordinary paragraph with the
  literal colons in it. Two pages had been live and wrong for weeks, and the
  reason nobody caught it is that the untitled form — which is most of them —
  still works, so the site looked fine everywhere anyone had looked.
  **A markup extension that degrades to visible plain text is worse than one
  that fails the build**, because the failure mode is a page that renders,
  deploys, and reads as intentional. When a build has an `onBrokenLinks: throw`
  it is worth asking what *else* is silently permitted; grep the whole tree for
  the deprecated spelling rather than fixing the one page you were looking at.
- **A tmux pane is a systemd scope, and an OOM inside it kills everything in
  it.** A runaway test allocated 80 GB and was OOM-killed four times in half an
  hour; ninety seconds after each kill, systemd tore down the whole
  `tmux-spawn-*.scope` (`OOMPolicy=stop`), taking `bash`, `claude`, `pkg-mcp`
  and eventually a two-day-old `llama-server` with it. The sessions looked like
  they were crashing on their own. **When a process dies for no local reason,
  check whether something else in its cgroup died first** — and start anything
  you want to survive as its own transient unit (`systemd-run --user
  --unit=…`), not from the pane.
- **`ulimit -v` is the wrong cap for a tokio program.** It bounds *virtual*
  address space, and thread stacks are reserved there, so a memory cap that
  looks generous makes `thread::spawn` fail in a way that reads as a real test
  failure. `systemd-run --user --scope -p MemoryMax=…` is RSS-based and
  behaves. The general shape: **a limit that measures something other than
  what you meant produces failures in an unrelated subsystem.**
- **`-np 1` is load-bearing.** The llama-server build in use defaults to **4
  parallel slots**, which silently splits `-c` across them: for a period on
  2026-08-04 every request ran against **8192 tokens of context, not 32768**,
  while mecha's `context_window` said otherwise. Past 8192 the server
  context-shifts instead of erroring, so the model saw a mangled transcript and
  returned *empty completions* — the mysterious empty-EndTurn deaths in the k=5
  compaction runs were this, not a mecha regression, and every scorecard taken
  between the two restarts is confounded. Check
  `curl :8080/props | jq .total_slots` is 1 before believing any measurement.
- **`pkill -f llama-server` kills your own shell**, because the pattern matches
  the command line running it. Use `pkill -x llama-server`.
- **A git worktree sharing the main `CARGO_TARGET_DIR` poisons the cache with
  its own paths.** Verifying a commit in a temporary worktree with the shared
  target dir rebuilt a test binary whose `env!("CARGO_MANIFEST_DIR")` was baked
  as the worktree path; after the worktree was removed, the cached binary
  matched fingerprints and kept being reused, and seven tests in an untouched
  crate failed with `NotFound` only on full-workspace runs — hours later,
  looking exactly like a regression in that day's work. **Compile-time paths
  live in the artifact, not the fingerprint: give a throwaway worktree its own
  target dir.** (2026-08-07, in the factory repo; the lesson is generic.)
  **Hit again in this repo on 2026-09-26**, and it cost time a second time
  because it reads as flakiness: with one `CARGO_TARGET_DIR` shared across
  the appraisal lane's worktrees, mecha-mail's fixture-spawning tests
  (in `google::docs_server`) failed on a missing fixture path, because the
  reused test binary's `env!("CARGO_MANIFEST_DIR")` named another,
  since-deleted, worktree. `cargo clean -p <crate>` or a per-worktree
  target fixes it. A "missing fixture" failure in a crate the change never
  touched is this trap until shown otherwise.
- **`cargo install` replaced a binary built from a different checkout, and
  the only warning was one line of its own output.** Three sessions wrote
  `~/.cargo/bin/mecha` on 2026-08-24 from three different checkouts of the
  same repo — one from `main`, one from a worktree on a feature branch, one
  from `main` again — each believing it was installing "the" binary. The
  second install would have silently dropped a shipped web feature from
  production; it was caught only because `cargo install` prints
  `Replaced package ... (/path/to/other/checkout)` and someone read it. It
  did not break anything, and that is the interesting part: the running
  process holds the *replaced inode*, so the regression sits latent on disk
  and activates on the next restart — systemd's, a reboot's, or anyone's.
  **An install is not a source.** The update skill already says a tag is not
  an install and a restart is not a reinstall; the corollary is that the
  artifact does not tell you which tree produced it, so on a machine with
  worktrees per session, verify the checkout before installing and `cmp` the
  result after. The fix that generalises is coordination, not care:
  the three sessions messaged each other, agreed who merged last, and that
  session did the single install and restart. (2026-08-24.)
- **A library that segfaults instead of raising turns a config error into a
  dead service.** `sherpa_onnx` accepts `modeling_unit="bpe"` with no
  `bpe_vocab`, builds the recognizer successfully, and then **segfaults
  inside `create_stream()`** — no exception, no message. It was found while
  pricing hotword biasing for the STT leg, which is the leg that feeds the
  microphone: had a nightly job generated a hotwords file the library
  disliked, `:8992` would have died and voice would have gone deaf with no
  error text anywhere. **Anything generated that feeds a native library must
  be validated in a subprocess first**, because the failure mode of a crash
  is indistinguishable from the failure mode of a service that was never
  started. (2026-08-24.)
- **The serve unit's `WorkingDirectory=%h` gave spawned CLI children a cwd
  the workspace jail correctly refuses**, so routes that shell out failed on
  containment for reasons that looked like a jail bug. Fixed narrowly for
  mail first, whereupon the tasks board promptly rediscovered it. **A fix
  that belongs at a spawn helper does not belong at whichever route hit it
  first** — it now lives in `serve/mod.rs::child_cwd`. (2026-08-24.)
- **A layout bug that needs *scale* to fire passes every short-list test, and
  a sibling page copied from it inherits the fuse.** A flex item with
  `overflow` other than `visible` has an automatic minimum size of **zero**,
  so the phone's mail cards collapsed to 30px slivers and clipped their own
  text — but only once the list outgrew the viewport. The Outbox page carried
  the identical latent bug and had simply never held enough drafts to trigger
  it. **When a bug turns out to need volume to appear, grep for its shape in
  every page that was copied from the one that broke**, rather than fixing
  the instance. (2026-08-24.)
- **A binary built here crash-looped the box with `status=203/EXEC` — this
  workstation is aarch64, the droplet is x86_64.** glibc versions matched,
  which was the check that got made; architecture was the one that mattered,
  and a wrong-arch ELF fails `exec()` itself, so systemd logs no reason at
  all — the restart loop was the only symptom. **`file` the artifact before
  it ships, and make the pipeline state its target as a contract** — the
  factory release workflow now refuses a non-static binary for the same
  reason. (2026-08-08; the box kept serving because the swap kept
  `factory.prev`.)
- **A config key appended to the end of a TOML file was silently swallowed:**
  the file ended in a `[mail]` table, so the new top-level key parsed as
  `[mail]`'s, serde ignored the unknown field, and the feature simply did not
  exist — no error anywhere, just a button that never rendered. **Appending
  to TOML is position-sensitive** (top-level keys go above the first table
  header), and a deploy should validate the config it is about to serve
  (`factory-deploy` now runs `factory check` before the swap) rather than
  trust that an edit landed where it reads like it landed. (2026-08-08.)
- **A one-shot HTTP stub that answers after one `read()` races the client's
  body write.** The connect tests' stub gate read once, replied, and closed;
  under a loaded parallel run the request arrived in two segments and the
  close became a reset, surfacing as a flaky connection error in whichever
  test drew the slow lane — while passing every isolated run. **A test stub
  must consume the full request (headers plus declared Content-Length)
  before answering**, or the flake lands on whoever loads the machine next.
  (2026-08-07, factory repo.)
- **Two tests that set the same environment variable flake only under
  parallel load.** `set_var`/`remove_var` are process-global and the test
  harness is threaded, so one test's `remove_var` lands mid-way through
  another's read — it never fails in isolation, only when the whole workspace
  runs at once, which reads as a bug in whatever else changed that day. A
  shared `env_lock()` both tests take is the fix; the general shape is that
  **anything process-global touched by a threaded test harness needs a lock,
  and the flake will not reproduce under `-p <crate>`.** (2026-08-07, factory
  repo, `MECHA_HOME`.)
- **Two agent sessions sharing one working tree: a broad `git add` sweeps
  the other lane's files into your commit.** The scheduling lane's commits
  (`37e9a50`, `ad4d8ff` in the factory repo) silently carried the admin-panel
  lane's uncommitted files, so an unrelated arc landed under commit messages
  that never mention it. Nothing broke — every commit still built — but
  blame and bisect now attribute that work to the wrong story. **When two
  lanes share a tree, stage by explicit path, never `git add -A`/`-u`**, and
  before committing check `git status` for files you did not touch.
  (2026-08-07/08, factory repo.)
- **`hf download repo --include X Y`** silently ignores `--include` when
  positional filenames are given. Pass filenames positionally *or* use
  `--include`, not both.
- Free-tier claims in comparison articles are often stale. Exa's own page says
  $10/month recurring credits (~1,400 searches), not the 20,000 some
  aggregators report.

---

- **A gitignored file in a disposable worktree dies with the worktree.** The
  mecha-graph OPERATIONS.md — the machine-specific values extracted from a
  public doc — was written inside a scrub worktree, gitignored by design, and
  destroyed when the merged worktree was removed. Nothing warned: git had
  never tracked it, so no tool considered it a loss. It was recreated from
  the pre-scrub file's git history, which happened to contain the same
  values. The lesson: gitignored means "no home but its directory" — a file
  meant to outlive a branch must be written in the main checkout, and a
  worktree should be treated as already deleted from the moment it is made.

- **And the mirror of that: something outside a worktree can point *into*
  it, and git cannot see the dependency.** A cleanup pass on 2026-08-25 was
  about to remove seven merged worktrees when `scripts/voice/mecha-voice-serve.service`
  turned out to carry
  `ExecStart=…/.claude/worktrees/voice-arc/target/release/mecha`. The unit was
  disabled, so nothing would have broken that day — it would have left a
  repo-tracked unit file naming a deleted binary, waiting for whoever
  installed it next. The same worktree had already cost a day earlier: a
  container was bind-mounting `/srv` out of it, three commits stale, and
  worked only because one file happened to be byte-identical. The lesson is
  about what the obvious check actually answers: **"0 commits ahead" tells
  you a worktree has no unmerged work, not that nothing depends on it.** A
  worktree is a filesystem path, so anything taking a path can name it —
  systemd units, container mounts, a process's cwd — and `git worktree list`
  knows about none of them. Before removing one, grep `scripts/`,
  `~/.config/systemd/user/` and the live docker mounts for its path, and
  check no process has its cwd inside it. Found by one session and
  generalised into a sweep across all seven by another, which is the only
  reason it is stated here as a rule rather than as one unit file.
- **The shared-tree `git add -A` trap recurred, twice in one day**, nineteen
  days after the entry above was written (2026-08-25: a forensic probe swept
  into a graph commit, then another session's uncommitted `HANDOFF.md` edit
  swept into `ae757b1`). Neither caused damage and the second was verified
  line-by-line to have landed intact — but "verified intact" was luck, not
  process. A rule stated once in a history file is not a habit; the session
  involved adopted explicit-path staging only after being shown the second
  instance. If two lanes are in one tree, the check that works is staging
  named paths, not reading a clean-looking `git status`.

- **And explicit-path staging is not enough, because the path can be shared.**
  A third instance, 2026-08-26 — by a session that had spent the afternoon
  being careful about exactly this. It refused `git add -A`, staged by named
  path, verified a peer's route line was absent from its own commit, and
  declined to run `cargo fmt --all` because two failing sites belonged to
  someone else's in-flight file. Then it committed `docs/HISTORY.md` by name
  and took 149 insertions, of which about seventeen were its own; the rest was
  a peer mid-write in the same file. Content survived intact and was wanted —
  the cost is that a pushed commit message now describes a tenth of its own
  diff, and the record under-describes itself in the one file whose job is to
  be the record.

  The generalisation is the part worth keeping: **`git add <path>` is safe
  against files you are not editing, not against files you are not the only
  one editing** — and documentation is where that bites hardest, because the
  `handoff` skill instructs every session to edit the same two files at the
  end of the same session. Code lanes are partitioned by module and diverge
  naturally; doc lanes are pointed at one target by procedure. Either stage a
  doc file only when `git diff <path>` is exactly your own change, or write to
  a per-session scratch section and merge deliberately. Verified-intact is not
  a process; it is the outcome you get when the collision happens to be
  additive.

- **A check can only report on what it enumerates, so it is always silent
  about what you forgot to enumerate.** Three instances landed on 2026-08-25,
  in three different tools, and they read as unrelated bugs until they are put
  side by side. The `update` skill's restart list omitted `mecha-serve` and
  `mecha-voice-worker`: a pure-Rust release would install a binary nothing
  re-executed, and `cargo install`'s output cannot mention a service it never
  restarted — worse for the voice worker, which runs `worker.py` straight from
  the tree, so it goes stale on changes no install step ever touches.
  `ls .git/MERGE_HEAD` was used to answer "is another lane mid-merge here?"
  and cannot see a linked worktree, whose merge state lives in
  `.git/worktrees/<name>/MERGE_HEAD` — a false negative on the question asked
  immediately before doing something irreversible. And `git log` enumerates
  from `HEAD`, which is what actually lost `5b187c5` from an attribution sweep
  (see Traps → Measuring): the starting point was the one input nobody
  questioned, because it is the one the tool supplies for you.
  The tell they share is that **each check answered correctly about its own
  frame and silently about the caller's**, so the output looks like evidence of
  absence and is only evidence of an empty enumeration. Distinct from the
  dash-not-zero rule, which is about rendering a failed read honestly; here the
  read succeeds. The fix is never to read the output more carefully — it is to
  ask what the check enumerates and widen the frame: `git worktree list` before
  the merge-state check, a commit range instead of a walk from `HEAD`, a
  restart list derived from the units that exist rather than from the build
  output. Where widening is not possible, the honest move is to say what was
  *not* covered — the same reasoning as `MECHA_TEST_REQUIRE_BACKENDS=1`
  turning a skipped integration test into a failure, because in CI a silently
  skipped test reads exactly like a passing one.

  **A fourth instance appeared in the sentence that closed that entry**, and
  it is the one to guard against hardest because the check that would catch
  it is not a command. The paragraph originally ended by invoking "the `no
  silent caps` rule" — which grepped to exactly one hit, the sentence citing
  it. Not a rule this project dropped: **a rule from the authoring agent's
  own tool documentation, imported into this history wearing mecha's voice.**
  It was never here to go stale.

  What makes it hard to catch from the inside is that the two vocabularies
  are written in the same register. *Fail closed*, *the silently-degrading
  sandbox*, *a lane must not promote itself*, *no silent caps* — named,
  italicised invariants, indistinguishable by feel, so recall cannot
  separate the ones this repo earned from the ones an agent arrived
  carrying. The failure does not dangle visibly; it resolves to nothing.
  And it is worse than a stale pointer, because a reader who trusts it
  goes looking for a principle this project never held.

  **So: grep before citing a named rule, and require a hit that is not the
  sentence you are writing.** It costs one command. The reason it belongs
  in this entry rather than beside it is that the paragraph's own review
  enumerated the reasoning and never the vocabulary — answering correctly
  about its frame and silently about the reader's, which is the entry's
  thesis one turn further in.

**"Clippy clean" is a statement about one toolchain, and the gate runs a
different one.** `main` went red for six hours on 2026-08-25 while
`cargo clippy --workspace --all-targets --all-features` was clean on this
box, exit 0. CI uses `dtolnay/rust-toolchain@stable` and had clippy **1.98**;
the box has **1.97**, where `clippy::result_large_err` does not exist — so a
`Result<String, Response>` whose error variant is a 128-byte axum Response
passed locally and failed under `-D warnings` upstream. Both facts were true
at once, which is why the local run felt like evidence. The general shape is
the same one the `update` skill exists for: **a check is only as current as
the thing running it**, and a green local gate proves the code passes *your*
toolchain, never the one that decides. When CI disagrees with a clean local
run, read the version in the log before reading the code.

**A default nobody chose ended calls for five months of wall-clock silence.**
Pipecat's `PipelineWorker` cancels an "idle" pipeline — and the runner with
it — after `idle_timeout_secs`, default **300**, where idle means neither
side produced a speaking frame. Nothing in this repo set it, so a voice call
died five minutes into any pause: the log said `Idle timeout detected` and
the client, which has no way to be told, showed only a peer connection that
closed. It was diagnosed from the journal in a minute and would have been
unfalsifiable from the client forever. **Two lessons, and the second is the
one that transfers.** A framework default is a decision your project made by
not making it — audit the ones that *terminate* things. And a component that
stops must be able to say why: the fix raises the timeout, but the part worth
keeping is that it now announces itself over the data channel before it tears
the call down.

**A child process that reports failure on stdout and exits zero will be
believed.** `mecha-graph accept <id>` prints `#id FAILED: cannot resolve
subject …` and exits **0** — correct for a bulk run where one candidate of
five hundred cannot resolve, and a lie to every caller that keys on the exit
code. The phone's queue page did exactly that: it dropped the card it had
just sent, reported success, and counted the verdict in a sample whose whole
purpose is to produce a number somebody quotes — while the candidate stayed
pending. The TUI had already learned this and tallied the report; the web
surface, written later against the same CLI, had not. **A driver must read
its child's account of what it did, not its exit code, whenever the child can
partially succeed** — and when two surfaces drive one verb, the tally belongs
in one shared function, or the second one relearns it the expensive way.

**A number measured in a shared checkout describes a tree that exists on one
disk and in no commit — and two sessions made that mistake on the same tree
within an hour.** On 2026-08-26 three lanes were live in one working copy.
Both a handoff sweep and an independent reviewer ran `cargo test --workspace`
to write the count into `HANDOFF.md`, and both got a number that included a
third lane's *uncommitted* work: 1,405 and 1,406 against a real 1,406 and
1,408 at the commits they each named. Neither was reading the other; the tree
was simply lying to both in the same way.

Two things make this worth an entry rather than a shrug. **The error is
invisible in its own output** — `cargo test` prints a total, not a
provenance, so the number looks exactly as authoritative as a correct one.
And **the doc's format invites it**: "measured at `<commit>`" is a claim about
a commit, so the measurement has to happen on a tree that *is* that commit.

So: `git status --porcelain` is the precondition of any number you write down,
not a nicety. If it is not empty, either the number is not about the commit
you are naming, or you take it somewhere else — `git worktree add` a private
tree (note it refuses a branch already checked out elsewhere, which is itself
the signal that you are not alone in there). The corollary that caught this
one: state counts **per crate and attributed per range**, because a total
nobody can decompose is a total nobody can check — the disagreement only
surfaced because a second reader could subtract.

**An idempotent upsert writes every field it is handed, including the ones
you did not think about.** The graph's `upsert_episode` defaults
`occurred_at` to *now* when it is absent, so the first cut of "edit a note"
would have moved every edited note to today — a notebook silently rewriting
when things happened because somebody fixed a typo. Caught before shipping
only because the round-trip was tested against a real note rather than
asserted about. **When re-upserting to update, enumerate what the write
touches and carry forward everything you are not deliberately changing**; the
dangerous fields are the ones with a plausible default, because those fail
silently rather than erroring.

**A green local build does not mean CI is testing your branch — it may be
testing an implicit merge with `main`'s current tip** (2026-08-28, PR #102).
`cargo clippy --all-features` and `cargo test --workspace` were clean, on the
exact flags CI runs, against the branch's own committed HEAD — and CI still
failed with a plain field-completeness error (`RunOutcome` missing two new
fields) at a source line the local checkout did not even have: the file was
2,130 lines locally and the error named line 2,260. `main` had merged an
unrelated PR mid-review, adding a new construction site this branch had never
seen. GitHub's `pull_request` trigger by default checks out the *synthetic
merge* of the PR branch with `main`'s current tip (`refs/pull/<n>/merge`), not
the branch's raw head — so CI was truthfully reporting a real conflict between
this branch and a `main` that had moved, while every local check answered a
question about this branch alone. **A CI failure with no local repro is not
necessarily flaky**: fetch and diff against `origin/main` before assuming the
runner is wrong, because the runner may be testing a tree that does not exist
in any local checkout yet.

**A review invoked by PR number moves the checkout under you.** The standing
rule is "work `main` from a separate worktree and leave this clone where
`llama-local.service` points", and it reads as advice about where you *type*.
On 2026-09-01 `/code-review 135` put `~/Github/mecha` on a `pr-135` branch —
`gh pr checkout` resolves a PR-number target in the current repo — and left
it there. Every branch of the session's actual work was in scratchpad
worktrees and it tripped anyway; a peer found it hours later, and it was
inert only because `scripts/start-moe-mtp.sh` happened to be byte-identical
on both. **A rule about where you type does not cover a command that moves
the clone for you.** Check `git branch --show-current` in the primary
checkout after any tool that takes a PR number.

**The instructions can be the stale artifact, not just the binary.** On
2026-09-02 the Skill tool served a cached `update/SKILL.md` from before the
commit that inverted the graph's source repo — it still said to build from
the private checkout and called the public one "a generated artifact, not a
source". The file on disk was already correct. Following the load rather
than the file would have reinstalled the graph from the retired repo and
silently undone the move, with nothing complaining. The skill's own thesis
is *verify the running thing, never the repo*; this is the same rule turned
around, and the check is to read the file when a loaded instruction contests
something you just changed.

**A word-boundary rename misses snake_case compounds.** `\bpkg\b` renamed
every `pkg` and skipped `pkg__kg_search`, because `_` is a word character,
so the one name a consumer types survived a rename meant to remove it. After
a rename, grep the bare stem with no boundaries and read every hit.

**A build that prefers a local checkout diverges from CI without saying so.**
`sync-graph-docs.mjs` read a sibling private checkout first and fetched the
public repo only when it was absent. The private one had stopped receiving
docs on 2026-09-02, so every local docs build published stale graph pages
while CI, which has no sibling, published the right ones. When a build has a
local fast path, check which source it actually read.

**An unpinned toolchain under `-D warnings` fails every open PR on release
day.** mecha-graph's first CI used `stable`; Rust 1.98 added
`chunks_exact_to_as_chunks` and the job went red on code the PR had not
touched. Pin the toolchain and move it in its own change.

**A reader of another program's store must find it by that program's rule.**
The mail crate finds its registry by `$MECHA_MAIL_DIR`, else `~/.mecha/mail`
under the real home, and ignores `$MECHA_HOME`. mecha read it as
`mecha_home().join("mail")` in five places: `frontdoor::mail_dir` first,
then `setup`, doctor, Slack's import check, and the booking sweep reading
mecha's `requests` from the other side. Each agreed with the writer only
while neither variable was set. So trials, tests and any relocated install
read a directory nobody writes, and the doctor reported a dead login as all
clear. Resolve a foreign store through a helper that copies the owner's rule
(`onboarding::mail_store_dir`), and pin it with a test that sets the
owner's variable. Test harnesses must isolate `HOME` too, not only
`MECHA_HOME`, or the fallback lands on the developer's real store (#428,
#432).

### A merge, made under a standing authorization, can race a fix in flight elsewhere

**A background fork was given "merge PRs once they pass review" and merged
#86 at 17:57:02Z using its round-3 tip.** A fourth review round had already
posted findings against #86 by then, in the foreground session's own context
— genuine ones (a missing test for `boredom_rate`'s None-not-zero contract, a
missing denominator clause on a print line, two doc-comment welds) — and the
foreground session had already started fixing them. The fix landed and was
pushed at 18:05:36Z, eight minutes after the merge, to a branch whose PR was
by then closed. `main` never got it; a follow-up PR (#93) had to cherry-pick
the same commit back in.

The fork was not wrong to check GitHub's state before merging — it did, and
at the moment it checked, nothing it could read said a fourth round existed
yet. The gap was that "has this PR passed review" was answered against
GitHub alone, when the more current answer — "review found more, and it's
being fixed right now" — existed only in a sibling conversation's working
context, with no shared channel either side was checking. **A standing merge
authorization is a green light for the git state, not a substitute for
asking whether anyone nearby has unpushed work against the same PR** — a
`SendMessage` before the merge ("about to merge #86, anyone got fixes in
flight?") would have cost one round trip and avoided the whole repair.

It recurred on 2026-09-30. Another lane, on the owner's "Do it", merged #428
and then #432, the latter at its branch tip `0d972f76` while the owning session was
answering #432's review. That fix was pushed a minute later to a closed PR
and needed #433 to reach `main`. The merge also landed #428 ahead of the
design doc its comments cite (#427).

## Design notes worth keeping

The rest of the original design-notes section duplicated `CLAUDE.md` and was
dropped in the split. These are the fragments that had no other home.

### The public surface

**A hand-copied list of variants is a bug waiting for the next variant.**
`keys::split` named `mk_pub_` and `mk_drn_` inline, so adding a third scope
minted tokens nothing could parse — and the symptom was `401: a valid bearer
token is required`, a server-side omission reported as the caller's mistake.
It derives from `Scope::ALL` now, with a test that mints every scope. The
general shape: **when a match would have caught it, a copied list will not**,
and the giveaway is an error blaming the other side.

**A guarantee that depends on which client connected is not a guarantee.**
"An agent drafts, a human releases" was a property of mecha's `[outbox] tools`
rather than of the factory, so any other MCP client — or a typo in that list —
silently had none of it. Ask of any safety property: *what happens if the
component holding it is replaced by a different one?* If the answer is "it
quietly stops", the property is in the wrong place.

**Reading the library beat reading its docs, twice.** `rustls-acme` cannot add
a domain at runtime and has no DNS-01 at all — so wildcards are foreclosed by
the crate, not by the DNS provider, which reversed a plan built on moving DNS.
And the blocker on per-user certificates turned out to be one `pub(crate)`
constructor reachable only from the TLS-ALPN-01 path, which HTTP-01 avoids
entirely. Both answers were twenty lines of source and neither was in a doc.

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

**A deferred call carries its arguments and forgot its jail.** Wiring the
factory into this machine (2026-08-06) turned up two bugs that only exist where
a *path* crosses between processes, and neither had bitten because no routed
tool had ever taken one — mail and calendar arguments are text.

First: an unconfined MCP server inherited mecha's working directory rather than
the run's workspace, because only the confined branch of `build_command` set
one. Every server that resolves a relative path — the factory publisher roots
its own `--root` there — was resolving against wherever mecha was launched.
Confinement was never the point; the two branches simply disagreed about where
the model's paths pointed.

Second, and the one worth the entry: `mecha outbox send` runs in a *different
process from a different directory* than the run that staged the draft. The
first release failed loudly, because the agent had written an absolute path and
the reviewer's jail rejected it. The second attempt is the one that mattered —
the agent wrote `{"bundle": "site"}`, and a relative path resolves silently
against whatever the reviewer is standing in. Had a `site/` directory existed
beside them, that release would have published the wrong bytes and reported
success. The fix is to record the drafting workspace on the item and rebuild
the release's tool surface rooted there, which is also the stricter of the two
jails. **A staged call is a deferred tool call, and a tool call means nothing
apart from the workspace it was made in — so the workspace is part of the
draft, not part of the reviewer.** The loud failure was luck: an absolute path
is the case that announces itself, and the design has to hold for the quiet
one.

### A guard that only covers the honest case

`Reach::sentence()` had four arms and a `for_a_person()` that fell back to the
bare artifact URL when the box named no viewer page. Its test asserted the
fallback — on `Serves::Everyone`, the one arm where the bytes URL genuinely
does open. In the other three the fallback made `page` and `bare` the same
string, so the private arm read "only you can open it: <URL>. <the same URL>
serves nobody" about an origin that answers 404, which is precisely the lie
the whole change was written to remove. It shipped green and a 26-agent review
caught it.

**A test that exercises one value of an enum has tested one arm, not the
function.** The fallback was uniform, so it looked like one behaviour worth
one assertion; what varied was the *sentence around it*, and that is where the
contradiction lived. When a helper feeds several branches, the branches are
the unit under test. The replacement walks all four states and fails if any
arm outside `Everyone` uses an opening-promise phrase with no page to name.

### The bot that had never run, and the log that would not say why

Porting the Claude workflows to a second repo turned into finding that neither
worked in the first. Three separate things, each hidden by a different
mechanism:

- The review job had failed on **every run it had ever made** — 31 of its last
  40, the rest skipped — and read as background noise because a red X on a
  bot check is easy to stop seeing. The cause was the credential, on both
  sides of an auth migration that was itself an attempt to fix it.
- `claude.yml`'s job condition used `if: |`. The literal block keeps every
  newline and adds a trailing one, and the job never matched; the workflow
  beside it used `if: >-` and ran. The fix has a second trap inside it —
  a folded scalar only folds lines at *equal* indentation and keeps a deeper
  one literal, so re-wrapping the condition prettily reintroduced the
  newlines. Caught by asserting the parsed string contained none.
- `claude-code-action` refuses to run when the workflow file differs from the
  default branch's copy, and **exits 0**. So a workflow change can never be
  tested by its own pull request, and the green check on such a PR means
  nothing at all.

Two lessons, both general. **A check that has never once passed is not a
flaky check, it is an unimplemented one** — and the way to tell them apart is
to ask for its success rate rather than looking at the latest run. And **a
diagnostic that requires making output public is the wrong diagnostic**: the
same failure was isolated by reproducing the auth path locally, which cost
nothing and exposed nothing.

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

### The web surface

**A fallback chain written with `??` and a template literal cannot fall
through, and rendered `undefined — undefined — undefined` on 695 review
cards.** ``payload.statement ?? `${subject} — ${predicate} — ${object}` ``
reads as two tiers and is one: a template literal is always a string, so the
right-hand side is never nullish and nothing after it can be reached. It
looked correct because the shape it was written for — subject/predicate/object
— is most of the queue; a commitment payload carries `{who, what, when,
direction}` and matched none of it. **A `??` chain only guards its left
operands; anything interpolated is already an answer.** The general form to
distrust is a fallback whose last tier *constructs* a value rather than naming
one.

The cost is not cosmetic and that is the part worth carrying: the card was
asking a person to accept a belief with nothing on it they could read, which
is the outbox's own rule — a field the reviewer cannot see is a field they
decided unread — arriving in a different store. Two Rust readers had the
right chain (`statement` → `what` → a named absence) with a test on the
commitment case; the page was the third and re-derived it. **A rule with a
test in one language is not a rule the other language's reader knows**, and
the fix is one function on the page matched to the Rust rather than improved
on: a card here that said more than the TUI's would be the same drift in a
better costume.

**A surface printed a remedy it could not perform.** The graph answers an
unbindable subject with *name a target with `--to`*, `mecha review bind --to`
existed, and `BindBody.to` had been on the web handler since it was written —
but neither the page nor the TUI could send one, so the instruction was
displayed to somebody holding no way to follow it. It survived because the
capability was present at every layer *except* the one with the keyboard, so
any reading of the server or the CLI says the feature is there. **Check a
remedy from the surface that prints it, not from the layer that implements
it.**

**Twenty-four mail cards rendered as 30px slivers on the first real phone
tap — chips visible, every summary gone.** The list's cards carry
`overflow: hidden`, and a flex item with overflow other than `visible` has
an **automatic minimum size of zero** — so inside the flex-column scroll
container, the whole list shrank to fit the viewport instead of scrolling,
each card clipping its own text. The fix is one line (`flex-shrink: 0` on
the scroll container's children; scrolling is the container's job), but the
shape is the lesson: the Outbox page carried the identical bug from birth
and never showed it, because it never held enough pending drafts to create
shrink pressure. A layout bug that needs *scale* to fire will pass every
short-list test and demo, and a sibling page copied from it inherits the
fuse. Headless Chromium reproduced it exactly (390px viewport, 24 rows), so
the check is cheap now that it is known.

**Every mail verb 502d from the phone while working perfectly from the
TUI.** The serve unit runs with `WorkingDirectory=%h`, and the `mecha mail`
children it spawns inherit that cwd — which the workspace jail *correctly*
refuses, because a workspace containing `~/.mecha` is the exact hole
`ensure_outside_mecha_home` exists to close. The TUI never saw it because a
TUI runs from wherever the person is standing, which is usually a project
directory. The general shape: a child process inherits the service's
working directory, and a refusal designed for interactive misuse will fire
from a unit file's default with a message aimed at a person who is not
there. A spawner that owns a workspace should set the child's cwd
deliberately (`serve/mail.rs::mail_child_cwd` — the web producer dir;
*inside* the mecha home is the default and fine, containing it is what is
refused).

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
