# Mecha memory prototype

An opt-in, **synthetic-data-only** first slice of the
[memory design](../../docs/MECHA-MEMORY-DESIGN.md). It runs alongside the current
system in a new SQLite file. It does not open/discover the live graph or register
timers. The explicit model comparisons, `distill-concept`, and opt-in `run` adapter use models;
ordinary memory commands do not. Nightly activity migration is explicitly deferred.

The implementation is in `mecha-core/src/memory`, behind the default-off
`memory-prototype` Cargo feature. The main `mecha` CLI exposes it when compiled
with that feature; normal builds do not include the command. This independent,
unpublished harness compiles the **same source and synthetic fixture**, without
pulling in unrelated Mecha dependencies. It is not another implementation or
a new production service.

## Explicit encrypted storage

The feature-gated main CLI accepts an owner-selected `--key-file` for native
initialization/sessions and the supported inspect/query/recall/forget operations.
Unconverted trial commands refuse encrypted selection before creating outputs.
Core callers use `StorageAccess`, `Memory::open_with_storage`,
`run_native_session_with_storage` and `appraise_native_session_with_storage`.
Storage access is separate from source and scope authorization.

For a new synthetic store, keep the key directory separate from the database,
authority and future task roots. Canonicalize the temporary path on macOS:

```bash
MEMORY_DEMO_ROOT=$(mktemp -d)
MEMORY_DEMO_ROOT=$(cd "$MEMORY_DEMO_ROOT" && pwd -P)
mkdir -m 700 "$MEMORY_DEMO_ROOT/keys" "$MEMORY_DEMO_ROOT/store"
target/debug/mecha memory --experimental \
  --db "$MEMORY_DEMO_ROOT/store/demo.memory-prototype.sqlite" \
  --key-file "$MEMORY_DEMO_ROOT/keys/memory.key" key-generate
target/debug/mecha memory --experimental \
  --db "$MEMORY_DEMO_ROOT/store/demo.memory-prototype.sqlite" \
  --key-file "$MEMORY_DEMO_ROOT/keys/memory.key" \
  init-native --synthetic-input --domain encrypted-demo \
  --authority "$MEMORY_DEMO_ROOT/store/authority"
target/debug/mecha memory --experimental \
  --db "$MEMORY_DEMO_ROOT/store/demo.memory-prototype.sqlite" \
  --key-file "$MEMORY_DEMO_ROOT/keys/memory.key" inspect
```

Build with `cargo build -p mecha-cli --features memory-prototype` first. The CLI
excludes its current directory and relevant database/task/authority roots from
key storage; repeat `--key-exclude-root` for additional host-selected exclusions.
Key generation never overwrites. Loading requires exactly 64 hexadecimal bytes
with an optional newline, private ownership/modes and no links or unsafe path
components. macOS extended ACLs and ownership-ignoring mounts are refused.
Wrong/missing keys do not trigger plaintext fallback or provider discovery.

The database and its rollback pages are encrypted. Session JSONL, authority files,
task artifacts and provider audits remain separate plaintext surfaces. Keep an
owner-controlled key backup apart from database snapshots; key recovery is not
provided. Removing a key file does not revoke an already loaded capability.
This remains synthetic-only; operational checks do not qualify private ingestion.

## Encrypted database snapshots

`memory backup` accepts an encrypted v8 source and a new destination with an
explicit destination key. It copies canonical tables, revisions, provenance,
restrictions, receipts and FTS state, then independently reopens and verifies the
copy before publishing it without overwriting another file. Source and destination
keys may differ. For the synthetic store above:

```bash
mkdir -m 700 "$MEMORY_DEMO_ROOT/backups"
target/debug/mecha memory --experimental \
  --db "$MEMORY_DEMO_ROOT/backups/copy.memory-prototype.sqlite" \
  --key-file "$MEMORY_DEMO_ROOT/keys/backup.key" key-generate
target/debug/mecha memory --experimental \
  --db "$MEMORY_DEMO_ROOT/store/demo.memory-prototype.sqlite" \
  --key-file "$MEMORY_DEMO_ROOT/keys/memory.key" \
  backup --destination "$MEMORY_DEMO_ROOT/backups/copy.memory-prototype.sqlite" \
  --destination-key "$MEMORY_DEMO_ROOT/keys/backup.key"
```

This snapshots the database only. External source authority is still required to
use copied native records. Later forgetting does not erase an older snapshot.
An error after publication can mean that a valid output exists but directory
sync failed; inspect the reported status rather than overwriting it on retry.
Snapshot verification uses host-memory row hashes proportional to row count;
corpus-scale performance is not yet qualified.

## Restore with current deletion and access rules

`memory restore` creates a new offline historical copy. It requires both the
snapshot and the current authoritative encrypted v8 store. It applies current
forgetting to the copied history, including derived concepts and capture records,
and refuses a surviving object whose restored access scopes would be weaker than
its current scopes. Missing or incompatible current authority is a refusal.

```bash
mkdir -m 700 "$MEMORY_DEMO_ROOT/restored"
target/debug/mecha memory --experimental \
  --db "$MEMORY_DEMO_ROOT/restored/copy.memory-prototype.sqlite" \
  --key-file "$MEMORY_DEMO_ROOT/keys/restored.key" key-generate
target/debug/mecha memory --experimental \
  --db "$MEMORY_DEMO_ROOT/backups/copy.memory-prototype.sqlite" \
  --key-file "$MEMORY_DEMO_ROOT/keys/backup.key" \
  restore --current "$MEMORY_DEMO_ROOT/store/demo.memory-prototype.sqlite" \
  --current-key "$MEMORY_DEMO_ROOT/keys/memory.key" \
  --destination "$MEMORY_DEMO_ROOT/restored/copy.memory-prototype.sqlite" \
  --destination-key "$MEMORY_DEMO_ROOT/keys/restored.key"
```

The result reports the snapshot and current-authority sequences and purge counts.
The current store is locked against writers during this local operation; a busy
store can cause a bounded refusal. Neither input is replaced. Retain current
external source authority when reading native records from the restored copy.

This does **not** merge newer facts, concept revisions or withdrawals: the old
heads remain locally current within the historical copy. It does not activate
that copy in Mecha or propagate future forgetting across copies. If the current
store is lost, this operation cannot safely infer later deletion decisions from
an old snapshot. Full disaster recovery requires a separately durable deletion
record and key-recovery procedure; those are not implemented here.

## Explicit native test sessions

The feature-gated `init-native` and `native-session` commands reuse the ordinary
agent loop with only jailed file tools and guarded GoalMemory. They retain actual
invocation/observation evidence in v8, independently check the saved JSON rows,
and reopen the explicit database for each invocation. They do not load ambient
configuration, activate normal chat/web memory, or migrate personal information.

```bash
cargo build -p mecha-cli --features memory-prototype
NATIVE_DEMO_ROOT=$(mktemp -d /tmp/mecha-native-demo.XXXXXX)
target/debug/mecha memory --experimental --db "$NATIVE_DEMO_ROOT/demo.memory-prototype.sqlite" \
  init-native --synthetic-input --domain synthetic-native-demo --authority "$NATIVE_DEMO_ROOT/authority"
target/debug/mecha memory --experimental --db "$NATIVE_DEMO_ROOT/demo.memory-prototype.sqlite" \
  native-session --synthetic-input --request "$PWD/prototypes/memory/fixtures/native-session.json" \
  --authority "$NATIVE_DEMO_ROOT/authority" --root "$NATIVE_DEMO_ROOT/task-1" \
  --endpoint http://127.0.0.1:8080 --model qwen3.6-35b-a3b-uncensored \
  --holds-dir "$HOME/.mecha/holds" --seed 70511
```

The chosen model must already be resident; requests never auto-load or switch it.
The request file is host input, not a model-authored tool argument. It supplies
bounded initial files, public confirmed criterion, expected string-valued JSON
rows, account/domain/scopes and an optional opaque interpretation context ID.
That ID selects an existing interpretation; it does not collect private needs.

The new task root contains `started.json`, a native transcript, provider audit and
`report.json`; its `workspace/` alone is accessible to tools. A started root cannot
replay an action. A later process uses the same database/authority with a new task
root and explicit request. Keep original artifacts/transcripts intact: current
source authority is rechecked, not assumed from historical success. Execution or
capture failures preserve reports and exit nonzero. A completed task with a failed
independent artifact check is retained as a failed outcome, distinct from those
execution failures.

Core callers can select canonical `CaptureReceipt.originals` using
`Memory::{prepare_native_concept,retain_native_concept}` and
`Memory::{prepare_native_concept_rumination,finish_native_concept_rumination}`.
They must open memory with the corresponding `LocalAuthority` source guard.
Support must span two distinct native sessions. All metadata and source dependencies
remain attached, including uncited inputs and restrictions; all outcomes recheck
current authority. Provisional concepts may be recalled and recaptured, but those
influenced episodes cannot count as independent support. Legacy native appraisal
still refuses Concept-bearing closure. The explicit
`Memory::{prepare_native_concept_evidence,recheck_native_concept_evidence,finish_native_concept_appraisal}`
path instead separates factual observations from provisional hypotheses.
`memory_situated_workflow::appraise_native_session` invokes that read-only path
for an already captured session using a caller-supplied local provider. It adds
no provider discovery, persistence, CLI verb or default activation. Each offered
hypothesis requires an applicability assessment; hypothesis IDs cannot serve as
factual premises. All consulted sources and exact scopes are rechecked after
inference, including unsuccessful outcomes. Older guardless concept APIs still
refuse native sources.

`mecha-core/tests/memory_integrated_native.rs` demonstrates the complete authored
lifecycle, including actual action, reopened concept-guided recall, an in-scope
contradiction, revision and source withdrawal. Its full revised packet fits the
existing 20 KB bound at 19,562 bytes; a larger authored version measured 21,042 bytes
and was omitted in full. These are mechanics tests, not evidence of model learning
or production scale. Larger-packet presentation and semantic qualification remain
open for those earlier measurements. The compact native reader now preserves full
original text while avoiding repeated metadata encoding: a reconstructed
fuller-citation case measures 20,352 canonical bytes versus 18,438 reader bytes
with the same 19 source revisions. This is a new fixture, not a byte-identical
reproduction of the earlier oversized case. Only native v8 consulted lineage
expands to 64 records; legacy v6/v7 remain at 32. Oversized mandatory content is
still withheld. Concept-aware appraisal has a separate 16 KiB response bound;
legacy appraisal retains 8 KiB. Authored tests cover reopened appraisal and
the 64-record response boundary. Spark passed both workspace suites, CLI checks,
the standalone harness and strict Clippy; HISTORY records the test-only
compatibility correction and exact evidence boundary. Real-model semantic
qualification remains open.

## Complete native evidence and provisional concept rumination

The default-off library now has two separate experimental interfaces. Neither
changes production capture, installs instructions, or replaces mecha-graph.

`Memory::prepare_native_evidence` derives complete typed values from authenticated
prepared sources. Action inputs, execution receipts, observed artifact bytes,
historical criteria and checks remain distinct. Host-issued fact IDs bind source
revision, field and metadata. Ordinary JSON-looking prose remains attributed text.
`finish_native_appraisal` checks source freshness and exact ID membership, but its
result remains a provisional model judgment; it is not a durable support report.
Oversized values are explicitly omitted, never clipped into misleading facts.

The paired runner uses the same facts, criterion, IDs, schema and one-call budget
in both arms: original envelope strings versus typed values. It preserves source
and database hashes, requests, responses, usage and failures. Run with a **new**
absolute output directory, an already resident local model and the service's
shared holds directory:

```sh
MECHA_SESSION_KIND=test cargo run -p mecha-core --features memory-prototype \
  --example memory_native_trial -- --synthetic-only /tmp/mecha-native-trial \
  http://127.0.0.1:8080 RESIDENT_MODEL /absolute/shared/holds 70510 CASE_FILE 0
```

An explicit trailing `--checks-first` instead compares typed direct v1 with typed
checked v2. Both arms receive the same complete facts and current criterion.
The checked response assesses artifact availability and compares evidence before
emitting its verdict. `finish_native_checked_appraisal` verifies authenticated
artifact roles, issued citations and comparison/verdict consistency; it refuses
inconsistency without rewriting the answer. Execution-only criteria can leave
artifact contents unassessed. These checks do not prove factual support or causality.

A trailing `--verdict-controls` runs six literal-copy diagnostics: met, not_met
and unknown through each response contract. Targets appear only in this mode.
Exact target equality, host acceptance, raw responses and call accounting are
recorded separately. Run controls before semantic comparisons; a failed control
is not evidence about reasoning quality. Each mode requires a new output root.
The core legacy runner and response contract remain available unchanged.
The standalone harness now preserves JSON insertion order, matching core. Stored
payload formats are unchanged, but serialized requests and recomputed request
identities are not guaranteed byte-identical across this build change. Use new
experiment roots rather than resuming old in-flight standalone requests.

`Memory::{prepare_concept_rumination,finish_concept_rumination,ruminate_concept}`
adds one explicit revision cycle around existing concept learning. Supply a current
concept revision and exact new counterexample excerpts. The request retains every
previously consulted episode and mandatory counterexample, refusing a source union
above eight records. A revised concept inherits its previous access restrictions
and remains provisional. Unchanged or abstained outcomes write nothing, including
none of the newly consulted evidence. Every outcome rechecks current source and
concept revisions; malformed output cannot replace the previous concept.

These original concept APIs accept the existing unbound synthetic source path;
the explicit native siblings above provide the separately guarded alternative.
Native external-source admission remains deliberately unsupported. No scheduler,
production adapter or automatic candidate discovery is added. Lifecycle regression
tests establish mechanics; useful abstraction, transfer and better decisions need a
separate held-out model comparison.

## Connected task, appraisal and recall demonstration

The feature-gated `mecha-core` example connects ordinary `Agent::run_in` tools
to an independently checked artifact, existing deterministic appraisal and an
explicitly created v6 memory file. Run from the repository root with a **new**
absolute directory:

```sh
MECHA_SESSION_KIND=test cargo run -p mecha-core --features memory-prototype \
  --example memory_bridge -- --synthetic-only /tmp/mecha-appraisal-bridge-demo
```

It uses an authored provider, makes no model/network calls, and retains synthetic
artifacts for inspection. The first task converts quoted-newline CSV to JSON
using ordinary file and todo tools. A separate OS process opens `GoalMemory`,
recalls the observation, and executes a second task with an uppercase-output
criterion. Both outputs are independently checked. This verifies a connected
workflow; the authored answers do not measure model learning or memory benefit.

Native JSONL remains authoritative. Full appraisal and sensor context live in
the private host authority directory, outside the task workspace and reader
database. SQLite stores bounded source passages, typed tool receipts, the frozen
criterion and the independent check result, linked by real dependencies. A
successful file write and an incorrect artifact remain different observations.
Library callers must preserve the example's confinement: recorded and actual
workspaces match, and tools have no grants or alternate access to host records.
The adapter does not establish those conditions for an arbitrary tool registry.

Native code-mode/program traces are refused by admission and source guards;
this bridge currently supports direct-tool captures. Nested-call provenance is
required before code-mode capture can be enabled.

The host adapter accepts explicit Test sessions only. It freezes one criterion
before dispatch, rejects ambiguous/changed goal histories, and retries capture
without executing tools. `GoalMemory::open_with_source_guard` injects current
authority checks; a missing guard, source change or withdrawal withholds the
bound records. Forgetting purges the receipt and its dependent content and blocks
late recapture; it does not delete external JSONL or host audit files.

This example does not enable automatic capture in normal chat or the deployed
service. Existing v1–v5 files and CLI experiments remain separate. The v6 reader
rejects intermediate v6 files lacking the complete-lineage format marker; create
a fresh synthetic demo rather than migrating those development artifacts. It
supports guarded evidence recall; unsupported concept/durable producers do not
gain authority from it. This v6 example has no interpretation generation; the explicit v7 workflow below adds it.
Production/private-data qualification remains separate work. The bounded model comparison below measures this bridge directly.

## Situated appraisal and changed-goal interpretation

### Evaluated statements and action/outcome evidence (v8)

The next opt-in format, `Memory::create_supported_situated`, creates a **new**
v8 database. It adds bounded native action arguments and actual artifact contents
to captured evidence, then requires separate proposal and factual-support passes
before retaining an interpretation. Existing v7 stores and examples keep their
original behavior; there is no automatic migration or production registration.

`LocalAuthority::capture_with_evidence` distinguishes a requested write, its call
receipt, and the independently observed artifact. Complete small UTF-8 artifacts
are readable; oversized, missing, refused and non-UTF8 artifacts retain explicit
omission states. No private sensor rationale or expected grader answer enters the
reader packet.

`memory_factual_support::{generate,evaluate}` makes bounded, local, tool-free
calls. `Memory::prepare_support_with_model` binds the proposal to its prepared
source context; `retain_evaluated_interpretation` rechecks that context and all
dependencies. Every offered unit and goal judgment receives a check. Unsupported
factual units are omitted, speculative units are labeled, and a Good/Bad judgment
requires its own supported assessment plus every originally named factual
premise. The host renders the retained text; rejected prose and evaluator
diagnostics remain outside reader memory.

These are **fallible model assessments**, not truth certificates. Quotation
membership alone did not catch false historical claims in the earlier comparison.
The separate `support_trial.py` qualification therefore tests fresh annotated
cases and externally audits the actual admitted statements before permitting a
downstream decision comparison. It scores evaluator classification separately
from final admission: dropping a duplicate goal judgment must not hide a wrong
evaluator answer. Concept learning and rumination remain behind this reliability
gate. The existing v2 concept/revision store and counterfactual replay are reuse
candidates; nightly behavior-rule learning is not automatically repointed.

Build the native qualifier, then freeze a new run before inference:

```sh
cargo build -p mecha-core --features memory-prototype --example memory_support_trial
python3 prototypes/memory/support_trial.py freeze /tmp/mecha-support-new \
  target/debug/examples/memory_support_trial http://127.0.0.1:8080 \
  RESIDENT_MODEL /absolute/path/to/model-holds
python3 prototypes/memory/support_trial.py run /tmp/mecha-support-new
python3 prototypes/memory/support_trial.py score /tmp/mecha-support-new
```

The eight synthetic cases use native Agent file actions, independent artifact
observations and ordinary GoalMemory recall. Each permits at most three local
model calls; the coordinator neither retries nor reloads a model. Record the
successful build and its source/binary hashes: freezing those hashes separately
does not establish that they correspond. Interrupted or malformed attempts stay
in the denominator. The gate also requires externally reviewed, digest-bound
audits under the run's `audits/` directory; scoring alone cannot supply those
judgments. The detailed report/audit contract is in `support_trial.py`.

Native memory reads keep their complete consulted-source dependencies and call
receipts without copying the whole recalled packet into another original passage.
The exact historical tool response remains in JSONL. Recapture therefore preserves
provenance without treating previously remembered evidence as a new observation.

The first frozen eight-case resident-model run did **not** qualify: 0/8 complete
workflows, 2/8 valid generated proposals, 1/2 valid generated-proposal evaluations
and 0/8 valid probes. One case retained four independently supported units and an
Unknown bearing. Raw rejected replies also contained semantic errors, so repairing
JSON alone is insufficient. See `docs/HISTORY.md` and the current repository
handoff for the next bounded repair; this is not production-ready appraisal.

### Explicit support wire-v2

The native example accepts `--grounded-v2` before `--synthetic-only`. It uses
`LocalTrialProvider::new_structured` and the existing llama JSON-schema transport;
providers without constrained-output capability refuse before inference. The
ordinary command retains wire-v1 behavior. Both protocols use the same v8 store,
source guards, canonical admission and ordinary GoalMemory reader.

Freeze this explicit protocol through the same coordinator:

```sh
python3 prototypes/memory/support_trial.py freeze /tmp/mecha-support-v2-new \
  /absolute/path/to/memory_support_trial http://127.0.0.1:8080 \
  RESIDENT_MODEL /absolute/path/to/model-holds --grounded-v2 \
  --fixture /absolute/path/to/independently-authored-cases.json
```

Omit `--fixture` only for an explicitly labeled development run on the existing
cases. `run` and `score` infer the frozen protocol from the manifest; changing
source, binary, helper or fixture after freeze refuses further calls.

The generator selects role-prefixed, request-bound citation handles instead of
copying source hashes and quotes. Mecha expands them to exact source references and supplies the
confirmed goal. The evaluator separately compares the current criterion and each
unit's factual premises, then assesses whether each offered judgment's original
premises support its bearing. Host conversion prevents contradictory supported
Good/Bad judgments, but cannot guarantee the model chose the correct criterion
state. Exact quotation membership remains distinct from entailment.

All original sources remain visible; catalog chunks cover every eligible source.
Whole requests include frame, input and schema in their byte budget. Raw and
expanded responses remain bounded and strict: no repair, retry, clipped diagnostics
or silent legacy fallback. Opaque context keys may differ while the semantic
source/criterion input stays invariant to private needs; no sensor values enter
model requests. Producer schemas use conservative 128-character text/reasons and
64-character questions, while canonical byte limits remain unchanged. Generated
Supported/retained bearings are scored separately from host-authored probe
judgments. Recorded bodies cannot show URL parameters: no-autoload is enforced
by the provider URL policy and verified by authored HTTP tests.

New development and fresh qualification must preserve their own
frozen artifacts and cannot replace the original failed eight-case result.

### Explicit evidence-aware support wire-v3

Use `--evidence-v3` in place of `--grounded-v2`; the coordinator requires an
explicit independently authored `--fixture`. This keeps SQLite v8 and the old
adapters unchanged. New requests include authenticated source roles, capture
associations, known timestamps and interpretation influence. Selected capture
means selected for this appraisal, not globally newest; missing times remain
unknown. The metadata participates in request binding and final admission checks.

The evaluator must assess every original citation as DirectSupport, ContextOnly,
Contradiction or Uncertain. Supported units require all original citations to
provide direct support. Optional diagnostic handles cannot replace an irrelevant
original quotation. The versioned report considers every candidate judgment:
the first eligible known bearing wins, conflicting eligible bearings become
Unknown, and otherwise the first candidate renders as Unknown. Original premises
and every disposition remain auditable. This does not withdraw separate historical
interpretation records or promote observations into learned instructions.

V3 exports full reopened `retained-generated.json` and `retained-probe.json`
records after successful retention. Qualification separately checks source
metadata, original-citation relevance, evaluator support, and the actual retained
judgment. A valid schema or mechanical regression does not establish semantic
reliability. Preserve prior frozen cohorts and keep concept learning deferred
until fresh evaluation meets the gate.

Before model qualification, exercise the complete native/auditor contract with
three authored HTTP responses and no real inference:

```sh
python3 prototypes/memory/support_native_smoke.py \
  --binary /absolute/path/to/memory_support_trial --root /new/smoke-directory
```

This explicit command refuses missing binaries or failed audits. CI builds the
example and sets `MECHA_SUPPORT_NATIVE_BINARY` for the corresponding unittest,
so the native path runs alongside the independent Python checks.

### Atomic observations and derived appraisal (v4)

Atomic interpretations use a distinct `interpretation_reader_v1` presentation in
contextual output. It keeps the complete support report, original claim-source
locators and context identity while removing repeated rendered draft text. The
stored canonical record and receipt remain unchanged. The full authorized bundle
must still fit 20,000 bytes; nothing truncates citations or drops dependencies.
Native recapture recognizes the compact record explicitly and retains its lineage.

The native report separates `retain`, `stored_export` and `recall`. A successful
write remains successful when later export or contextual recall fails. Available
receipt, database and canonical-record artifacts survive independently of recall;
recall reports selected_current, budget_omitted and actual packet bytes. Archived
reports without stored_export remain supported by the independent auditor.
These diagnostics and compact serialization do not repair semantic judgment errors.

The explicit `memory_atomic_trial` example compares a three-call policy with a
single direct judgment over the same captured source evidence. Generation emits
observations without IDs or judgments; the host assigns IDs. A second call checks
each original citation. The final appraisal combines supported observation IDs
against the current criterion. Its v4 report labels that judgment as derived,
preserving the original observations and checks. Direct answers are unverified
comparison outputs and are not retained as independently checked memory.

Both arms receive 6,144 output tokens in total and a shared 540-second deadline
per arm, including provider setup. Pipeline allocations are 1,536 / 3,072 / 1,536.
All calls explicitly disable the model's thinking mode and use the already
resident model. Actual repeated input, output and elapsed cost are reported.
Legacy trials retain their 180-second provider limit and existing wire behavior.

```sh
cargo build -p mecha-core --features memory-prototype --example memory_atomic_trial
target/debug/examples/memory_atomic_trial --synthetic-only \
  /new/absolute/run-root http://127.0.0.1:8080 RESIDENT_MODEL \
  /absolute/shared/holds 70510 /absolute/case.json 0
```

`CASE_INDEX` alternates the execution order. Case envelopes use the existing
native source fields; v4 does not evaluate probes or expose their annotations.
`atomic_trial.py` separately freezes source/binary/auditor hashes before new case
authoring, freezes eight cases afterward, and audits raw requests, replies,
bindings, source SQLite and actual retained/recall records. Failed cases stay in
the planned denominator. Exact source membership is not semantic support;
independent semantic review is required before interpreting the comparison.

Run the actual native/auditor contract with authored HTTP replies before model
qualification (CI supplies the binary explicitly):

```sh
MECHA_ATOMIC_NATIVE_BINARY="$PWD/target/debug/examples/memory_atomic_trial" \
  python3 -m unittest discover -s prototypes/memory/tests -p test_atomic_trial.py -v
```

The first frozen eight-case comparison matched labels in 5/8 for both policies.
All 32 calls returned, but five three-call workflows failed at post-storage recall;
only 3/8 completed, versus 8/8 direct answers. The inspected complete provenance
bundle exceeded the reader's 20,000-byte cap. The gate failed; no accuracy advantage
or production reliability is established. Independent review covered all 32 replies
and found accepted factual/citation defects and insufficient selected premises;
both arms failed to abstain on two unknown cases. See HISTORY for complete counts,
cost and audit limitations.
Build all test variants first, then copy and freeze the feature-enabled example:
a later default-feature build can overwrite the same target path with its stub.

These are synthetic prototype experiments, not production or private-data
qualification. Current measured outcomes and remaining gates are in
[`MECHA-MEMORY-DESIGN.md`](../../docs/MECHA-MEMORY-DESIGN.md) and the research Site.

### Legacy v7 demonstration

The explicit v7 workflow adds durable, source-linked interpretation hypotheses
beside the unchanged episode. It reuses existing interpretation parsing,
qualified grounding, deterministic appraisal and the ordinary agent loop.
Run the authored mechanics check from the repository root with a new directory:

```sh
MECHA_SESSION_KIND=test cargo run -p mecha-core --features memory-prototype \
  --example memory_situated_appraisal -- --synthetic-only /tmp/mecha-situated-demo
```

This makes no model calls. The same route evidence supports a speed goal and a
calibration-preservation goal. The example reopens the store, plans with the
current interpretation, checks the output independently, captures that task,
and reappraises its history under the changed goal. Withdrawing the source
withholds dependent interpretations and captured descendants.

`Memory::create_situated` creates a new v7 store; it does not migrate v6 files.
`prepare_interpretation` authenticates the original capture and produces a
bounded source packet. `memory_situated_appraisal::generate` generates without
holding a database transaction and verifies the served model identity.
`retain_interpretation` rechecks source authority,
quotes, criterion and draft limits before saving the hypothesis and
its exact revision/field citation map. These checks establish source membership,
not semantic truth. Every supplied source remains a dependency, even if uncited.

Default readers withhold interpretations and interpreted descendants. A host
explicitly opts into `GoalMemory::with_interpretations` using the exact current
context. Returned hypotheses are marked `SelectedCurrent` or
`HistoricalDependency`; old meanings retain their original criteria. Both are
private, untrusted evidence, never action permission or accepted learned rules.
V7 projections reference existing original evidence instead of copying its text;
this preserves complete lineage within the unchanged contextual byte budget.

`NeedsInput::assess` retains the existing reading states, withdrawal, observed
zero and freshness for the host. Private readings and their rationales are not
serialized into memory or prompts. Needs-only changes leave the generation
request identical; automatic needs-driven goal reprioritization is not implemented.
Callers must explicitly supply the current confirmed goal and opaque context.

The separate `memory_situated_trial` example accepts explicit local endpoint,
resident model, shared holds directory, seed, goal and arm order. It clones the
same source store before generation and planning; both arms see the same original
facts, while treatment alone can retrieve a generated current interpretation.
It records provider requests, partial failures, exact artifacts and new captures.
The fixed development protocol and its retained results are documented in HISTORY;
this is not automatic production capture or a held-out learning benchmark.

## Bounded native model comparison

`memory_bridge_trial` runs one synthetic task per process through the same
native agent loop. `bridge_model_trial.py` freezes four development cases and
two replicates: eight initial experiences and sixteen later tasks, paired between
memory available and an empty valid memory store. Both later arms retain the same
prompt, inputs, current criterion, tools, budgets and hidden grader output.
These are 24 planned **agent runs**, potentially making several model requests.

Build and qualify the actual executable with authored HTTP responses first:

```sh
cargo build -p mecha-core --features memory-prototype --example memory_bridge_trial
python3 prototypes/memory/bridge_model_trial.py \
  --binary "$PWD/target/debug/examples/memory_bridge_trial" \
  --output /tmp/mecha-bridge-scripted-new --preflight
```

The preflight retains all 24 scheduled slots and adds eight authored probes.
The unavailable-check probe completes a todo step with a declared command while
shell is absent: no shell call is generated, feedback remains skipped/unverified,
and artifact writing still completes. Registered checks retain ordinary guards.

For the real comparison, supply the already resident model and the same holds
directory used by the local Mecha router controller:

```sh
python3 prototypes/memory/bridge_model_trial.py \
  --binary "$PWD/target/debug/examples/memory_bridge_trial" \
  --output /tmp/mecha-bridge-real-new \
  --endpoint http://127.0.0.1:8080 --model RESIDENT_MODEL \
  --holds-dir /absolute/mecha-home/holds
```

The explicit local provider requests fast mode, temperature 0.6 and a frozen
seed; it has no automatic model load, switch, fallback or inference retry.
It yields to a requested switch. Each task has eight turns, at most 4096 output
tokens per request, 16384 total output tokens and a 600-second deadline. Unknown
usage remains unknown and conservatively consumes the requested allowance.

A completed first experience is copied into a separate later-run store and
host authority; its original transcript and artifact remain outside the later
workspace. Withdrawal changes only the copied authority. The coordinator freezes
source, binary, helper, inputs and grader hashes; records all planned slots;
checks original files remain unchanged; and refuses resume or replacement of an
interrupted comparison. Failed captures leave the later tasks explicitly
unstarted in their original denominator. Wrong artifacts and failed capture are
separate results; neither is silently repaired or excluded.

Report task correctness separately from evidence-appropriate abstention: an
empty-store run may correctly request a missing calibration parameter without
completing that task. These small authored cases measure information use and
workflow reliability, not generalization, learned concepts, or an appraisal-only
causal effect. Scripted preflight is integration evidence, never model benefit.

The first frozen real run (7 November 2026) completed all 24 attempts and 112
model calls. Exact later artifacts passed 6/8 with memory versus 4/8 without it;
both improvements supplied prior calibration information, while controls
appropriately abstained. This is not an appraisal-specific effect. Three runs
exhausted their turn budget and two exceeded capture's passage bound. Todo checks
also exposed an unavailable-shell dispatch in the fixed registry. Two other exact
failures reflect an extra output field under ambiguous schema wording. Preserve
these results; see [HISTORY](../../docs/HISTORY.md) and [HANDOFF](../../docs/HANDOFF.md) for the diagnosis and
remaining repairs and separately retained follow-up comparisons. Planning-check
registration is now repaired; capture coverage and dependable stopping still need
qualification. This prototype is not yet a dependable production memory loop.

## Budget-notice comparison

The native example accepts optional `--budget-notice true|false` (default false).
It advises the actor before its last two turns while leaving the turn cap,
available tools and execution guards unchanged. Recognized harness advice stays
in the raw transcript but is not admitted as an owner-request passage.

`budget_notice_trial.py` is a separate protocol: four evidence conditions, two
fixed seeds, and notice/control arms (16 later tasks plus two source-setup runs).
Its unavailable case reuses the previously failing calibration task. The other
conditions test information available now, an authorized replacement reachable
after a missing-file read, and withdrawn prior evidence. This is development
evidence, not a held-out benchmark.

```sh
python3 prototypes/memory/budget_notice_trial.py \
  --binary "$PWD/target/debug/examples/memory_bridge_trial" \
  --output /tmp/mecha-budget-scripted-new --preflight
python3 prototypes/memory/budget_notice_trial.py \
  --binary "$PWD/target/debug/examples/memory_bridge_trial" \
  --output /tmp/mecha-budget-real-new \
  --endpoint http://127.0.0.1:8080 --model RESIDENT_MODEL \
  --holds-dir /absolute/mecha-home/holds
```

Run the authored preflight first. Its deliberate last-turn write can produce a
correct artifact and still end at `MaxTurns`: `preflight_passed` checks that
expected behavior separately from natural completion. The real comparison
records actual notice exposure, correct artifacts, appropriate and premature
abstention, missing artifacts, stop status and cost. A run that finishes before
the notice is unexposed. Neither an absent artifact nor a host refusal counts as
model abstention. All planned slots remain in the denominator; no retry replaces
a failure. Earlier protocols and results remain unchanged.

## Main Mecha CLI

From the repository root, build the opt-in binary:

```sh
cargo build -p mecha-cli --features memory-prototype --locked
target/debug/mecha memory --experimental --db /tmp/main-demo.memory-prototype.sqlite demo
target/debug/mecha memory --experimental --db /tmp/main-demo.memory-prototype.sqlite inspect
target/debug/mecha memory --experimental --db /tmp/main-demo.memory-prototype.sqlite query works --valid-at 2026-07-01T00:00:00Z
target/debug/mecha memory --experimental --db /tmp/main-demo.memory-prototype.sqlite query works --known-at 4 --valid-at 2026-07-01T00:00:00Z
target/debug/mecha memory --experimental --db /tmp/main-demo.memory-prototype.sqlite forget employment
```

Both `--experimental` and `--db` are required, before the subcommand. Query
defaults to current knowledge, definite time matches, and 20 results; use
`--include-possible` to include unknown bounds and `--limit N` to change the
result cap. These commands do not load global/project configuration, acquire
model holds, start connectors, or register a runtime feature. Existing `kg`
behavior is unchanged.

## Read-only planning context (synthetic only)

With the feature build, `mecha run` can replace its existing `goal_context`
reader for one invocation. The database is opened read-only, with a frozen
knowledge snapshot, one explicit source account, and owner-selected scopes.
Current restrictions and forgetting still override that snapshot. Other CLI
surfaces and existing subagents keep their ordinary tools. The replacement
requires `goal_context` to already be available in the selected tool profile.

For example, create a **new** demo file and use a configured provider:

```sh
target/debug/mecha memory --experimental --db /tmp/planning.memory-prototype.sqlite demo
MECHA_SESSION_KIND=test target/debug/mecha --tool goal_context --no-mcp --no-code-tool run \
  'Use goal_context memory query Person to outline a source-grounded summary; distinguish dated evidence from current facts.' \
  --goal task:summary \
  --memory-context /tmp/planning.memory-prototype.sqlite --memory-synthetic \
  --memory-domain synthetic-demo --memory-account synthetic --memory-scope fixture-mail
```

Unlike `mecha memory`, `run` uses ordinary agent/provider setup. This option is
an explicit synthetic experiment, not permission to load private data. It does
not change the default installed service, graph, tool registry, or config.

The tool accepts `memory: {"query":"launch"}`. For a follow-up, repeat that
initial query and add up to two `bindings`, each containing `source` (the full
object/revision reference), `field` (original text pointer, or empty for a plain
capture), `quote` (exact contiguous text, e.g. `Ilex`), and `query` (e.g. `cost`).
The source must appear in the same initial packet; source metadata and composed
names such as `Ilex pipeline` from `Ilex or Quill pipeline` are rejected. The host
finds byte offsets and derives the mention. No invalid selection falls back to
broad search. This is mention matching, not alias/coreference resolution.

`memory::Memory::context` is also a reusable provider-free API: a `ContextRequest`
(query/selections), host `ContextPolicy` (access/snapshot/filters/budget), and
`Situation` (event/goal/optional caller-described needs) produce a `ContextPacket`.
It returns original episode records, source revision references, original text
fields, occurrence timestamps, and supported relations with their valid-time
intervals and dependency records. Relations and their full dependency bundles
are included together or omitted together. Unknown relation time is marked
`possible_time`; an absent episode timestamp stays absent. No numeric confidence
is invented. Empty results and budget omissions are explicitly inconclusive.

The `run` adapter uses three anchors per search, no neighbors, up to two selected
follow-ups, at most eight directly supported relations, and a 20,000-byte
serialized **memory packet** budget including its envelope (the existing goal
lessons/examples remain separately bounded). Selected sources precede follow-up
hits, then other initial hits. All stages share one read transaction. Retrieved
records arm both private and untrusted taint; validation errors remain local.

The CLI supplies its prompt and confirmed goal. It leaves needs unavailable;
internal charter/homeostat readings are not exposed. The older generic context API accepts
caller-described needs, but does not itself evaluate needs, rank plans, learn
concepts or generate interpretations. The separate v7 producer and host-only
needs assessment above are explicit opt-ins. Retrieval remains
a correctness-first eligible-corpus lexical scan, not a production latency claim.
The older model-comparison protocols and measurements are unchanged.

## Capture and retrieve synthetic sessions

The main CLI can now capture native Mecha JSONL, retaining exact physical lines,
speaker roles, source IDs and ordering. A session is the initial episode boundary;
semantic segmentation is deferred. Re-importing unchanged input is a no-op;
appends are atomic. Edited prefixes, truncated files, malformed records and
unfinished final lines fail without partially importing. Limits are 32 MiB per
file, 1 MiB per record and 100,000 records. Scope changes create revisions.

```sh
target/debug/mecha memory --experimental --db /tmp/session-demo.memory-prototype.sqlite init
target/debug/mecha memory --experimental --db /tmp/session-demo.memory-prototype.sqlite import-session prototypes/memory/fixtures/session.jsonl --account synthetic --synthetic-input
target/debug/mecha memory --experimental --db /tmp/session-demo.memory-prototype.sqlite recall permit --account synthetic --session atlas --neighbors 1
target/debug/mecha memory --experimental --db /tmp/session-demo.memory-prototype.sqlite forget-session-record --account synthetic --session atlas --line 3
```

`recall` ranks captured evidence with BM25 over literal query terms (OR), then
expands neighboring records within the same account/session and snapshot. Each
anchor and neighbor passes current access restrictions before output. Results
include exact revision references, raw records and JSON pointers to original
text fields. `--max-bytes` bounds total serialized evidence payloads, excluding
outer result framing; records that do not fit are omitted, never truncated.
Anchors take budget precedence over neighbors. An empty result can mean no
match, inaccessible evidence or insufficient budget; it is not proof of absence.

BM25 statistics are computed over eligible evidence, not over inaccessible or
future records. The prototype copies eligible indexed text into a temporary
in-memory FTS scoring index per query: no scratch text is written to disk. This
is deliberately a correctness baseline with corpus-sized query work, not a
large-corpus performance claim. Cached/partitioned statistics can be evaluated
later without changing the access and historical-ranking contract.

`--known-at` selects knowledge sequence. `--occurred-after` (inclusive) and
`--occurred-before` (exclusive) filter explicit record timestamps; unknown dates
are excluded when either filter is present. Native messages generally have no
timestamp: the adapter never substitutes session creation time. Occurrence time
is separate from the validity intervals supported by `query` for relations.

Unknown records/blocks and compaction rewrites are preserved as raw evidence.
Only known `message` text blocks are indexed; tool payloads, extensions,
configuration and rewrite summaries are not indexed in this first baseline.
This reads source records, not the reconstructed current conversation. Raw
record strings can contain terminal control characters; consumers must render
them as data. Nothing recalled grants instruction or execution authority.

Forgetting a line purges its local revisions/dependents and suppresses re-import;
it does not modify the original JSONL file or prior exported results. Re-import
of a file still containing that suppressed line fails atomically. Source-wide
deletion coordination remains a production requirement.

## Evidence retrieval experiment

```sh
target/debug/mecha memory --experimental --db /tmp/retrieval-eval.memory-prototype.sqlite evaluate > /tmp/retrieval-eval.json
```

Choose a new database filename. This runs 32 fixed synthetic cases across eight
scenarios: direct recall, corrections, temporal filtering, indirect relevance
and paired changed goals. It reports baseline hits, exact oracle evidence,
per-case gold coverage, complete coverage counts, evidence bytes and local
retrieval timings. Gold labels are not passed to retrieval. Changed-goal pairs
hold the query/source constant and vary goals and expected evidence; the
baseline deliberately does not route from those goals or homeostatic context.

This is an evidence diagnostic, not a reasoning or planning benchmark. Oracle
coverage verifies that the required evidence was captured and accessible; no
answer model runs. The cases are development fixtures, not a held-out claim of
generalization. Future model/strategy comparisons must freeze separate held-out
episode families and common compute budgets. The full evaluation fixture lives
inside `mecha-core/src/memory/fixtures` so published core source is self-contained.

## Standalone qualification harness

From the repository root:

```sh
cargo run --manifest-path prototypes/memory/Cargo.toml --offline -- demo /tmp/example.memory-prototype.sqlite
```

Choose a new filename; creation refuses to overwrite anything. The parent
directory must already exist. The suffix `.memory-prototype.sqlite` is required,
and directories beneath `.mecha` or `.mecha-graph` are refused.

The fixture contains Person A, Organization B and two source messages:

1. A June message reports employment beginning May 1. Snapshot sequence **4**
   contains a grounded relation with no asserted end date.
2. A July message reports departure on June 30. Snapshot sequence **6** contains
   the revised relation, preserving the earlier version and both source inputs.

The message occurrence timestamps are synthetic event dates. Commit timestamps
are the actual time you run the demo; snapshot sequences identify knowledge
states. This does not fabricate historical database commits.

```sh
# Current knowledge says the relation held in May: one relation.
cargo run --manifest-path prototypes/memory/Cargo.toml --offline -- query /tmp/example.memory-prototype.sqlite works latest 2026-05-15T00:00:00Z

# Current knowledge says it ended before July: no relation.
cargo run --manifest-path prototypes/memory/Cargo.toml --offline -- query /tmp/example.memory-prototype.sqlite works latest 2026-07-01T00:00:00Z

# Earlier knowledge had no asserted end date: one relation, referencing its old evidence.
cargo run --manifest-path prototypes/memory/Cargo.toml --offline -- query /tmp/example.memory-prototype.sqlite works 4 2026-07-01T00:00:00Z

# Forget the original employment source and all dependent claim revisions.
cargo run --manifest-path prototypes/memory/Cargo.toml --offline -- forget /tmp/example.memory-prototype.sqlite employment

# Even the old snapshot now returns no relation.
cargo run --manifest-path prototypes/memory/Cargo.toml --offline -- query /tmp/example.memory-prototype.sqlite works 4 2026-07-01T00:00:00Z
```

Commands emit JSON. `inspect FILE` reports the instance and current sequence.
Omitting valid time searches evidence/entities/relations; supplying it restricts
results to relations covering that instant. Standalone harness queries include
uncertain time matches labeled `possible_time`; the main CLI requires explicit
`--include-possible` for these matches.

## Implemented contracts

- Explicit new-file creation and prototype identity/version checks on reopen.
- Stable source identity and immutable revisions; retries are no-ops, while
  A→B→A edits create distinct revisions. Capture includes occurrence time.
- Entity references, positive/negative relations or literals, exact UTF-8 source
  spans and complete caller-declared consulted inputs. Admission checks structure
  and current revisions; it does **not** prove semantic entailment.
- Optimistic revision checks and atomic SQLite transactions. Changes in source
  heads make dependent claims stale for current search without erasing history.
- Snapshot and valid-time queries; UTC normalization, half-open intervals,
  distinct unknown and unbounded endpoints, withdrawal without resurrection.
- `query` provides FTS5 lexical matching; current scope restrictions and dependency eligibility
  are checked before the result limit. Ordering is deterministic recency, not
  an evaluated relevance model. `recall` provides the separate BM25 evidence
  baseline with bounded source windows. Results conservatively label private/untrusted
  exposure; nothing grants instruction or action authority.
- Conservative forgetting across every revision of affected objects and their
  dependents; source suppression persists across restart. Independent entities
  and evidence remain. Search storage is rebuilt and SQLite vacuumed.

`Access` is supplied by trusted host code. It is **not** authentication and may
not be exposed as model-controlled tool arguments. This prototype has no tool
registry or external service endpoint. Returned JSON, process memory, copied
files and terminal logs are outside its local-database deletion claim. There
is no claim of physical SSD or filesystem-backup erasure.

## Provisional concepts

`distill-concept` makes one isolated call to the explicitly named resident local
model using 2–8 original source records selected by the caller. At least two
independent episodes must support the result. Use object/revision pairs from
import or recall output:

```sh
target/debug/mecha memory --experimental --db /tmp/planning.memory-prototype.sqlite distill-concept \
  --source OBJECT_A:REVISION_A --source OBJECT_B:REVISION_B --source OBJECT_C:REVISION_C \
  --endpoint http://127.0.0.1:8080 --model RESIDENT_MODEL \
  --holds-dir /explicit/shared/holds --seed 7
```

The retained object contains a name, principle, applicability, exact supporting
quotes, counterexamples, and **all consulted source revisions**, including
uncited ones. It is always `provisional`; citation validation establishes
provenance, not truth or semantic entailment. Retention needs no per-fact approval
and never creates a trusted behavioral rule. `--revise OBJECT:REVISION` revises
an existing concept with an optimistic concurrency check. Withdrawal is exposed
in core, not yet as a concept CLI command.

Core `prepare_concept` creates an opaque receipt; `retain_concept` rechecks
current revisions and unchanged source scopes in a write transaction after
inference. `learn_concept` combines those with a provider-neutral async callback.
No database transaction is held across inference. Source restriction, withdrawal,
or forgetting removes dependent concepts from eligible context. First concept
retention advances the disposable database vocabulary version to 2; older
binaries reject it, while new readers accept versions 1 and 2.

The runtime context path can retrieve up to two lexically matching concepts by
recency, even when their vocabulary is absent from original episodes. Each
concept enters with its complete source bundle under the same host filters and
packet budget, carrying private/untrusted taint. This is not vector retrieval or
a quality ranking. Support selection, concept quality evaluation, automatic
consolidation and live rumination remain future work.

`planning_trial.py` provides a small six-case, memory-off/on integration trial
through the real CLI, using isolated synthetic homes and the existing resident
model. It records choice, visible citations, tool errors, usage and latency.
It is not a matched-information/compute benchmark and does not grade free-form
plan correctness. Run its no-model isolation checks with
`python3 -m unittest discover -s prototypes/memory -p test_planning_trial.py`.

## Assessing and revising plans

`memory evaluate-plans` is an explicit advisory evaluator over the same bounded
memory context. Supply a synthetic JSON file containing `query`, `event`, `goal`
and 1–4 `candidates`, each with an `id` and 1–8 short `steps`:

```json
{
  "query": "customs clearance alternate route",
  "event": "Plan lawful delivery. Investigate unresolved prerequisites before booking.",
  "goal": "task:shipment",
  "candidates": [
    {"id": "reroute", "steps": ["Book an alternate carrier assuming it bypasses customs."]},
    {"id": "investigate", "steps": ["Check whether the alternate carrier still needs customs clearance before booking."]}
  ]
}
```

```sh
target/debug/mecha memory --experimental --db /tmp/planning.memory-prototype.sqlite evaluate-plans \
  /tmp/synthetic-plans.json --synthetic-input --account synthetic-demo \
  --endpoint http://127.0.0.1:8080 --model RESIDENT_MODEL \
  --holds-dir /explicit/shared/holds --seed 7 --revise
```

The account must match the imported fixture records. This command uses the
synthetic fixture domain/scopes, current snapshot, up to five lexical anchors,
and a 20,000-byte context packet. It reads no live config and executes no steps.
It does not automatically run on ordinary chats or the main web app.

Each candidate receives checks for **prerequisites**, **factual support**,
**outcome**, and **concept applicability**, with exact source quotations and explicit unresolved
questions. The host verifies complete check/ranking coverage and citation
provenance, then derives `revise`, `needs_evidence`, or `consider`. Only candidates
marked `consider` can appear as `recommended`. The model supplies the ranking;
these are fallible judgments, not verified feasibility, optimality, or permission
to act. A provisional concept alone cannot support a factual/prerequisite check.

`--revise` preserves an existing recommended candidate unchanged. If none passes,
it adds one minimal-repair call and reassesses that proposal against the original
evidence packet: one call when preserved, at most three otherwise.
`revision_status` distinguishes `not_requested`, `kept_existing`, and `reassessed`. Both rounds stay
in the JSON report, even if the revision remains unresolved. It adds no approval
queue, persistent trusted lesson, or open-ended rumination loop. Reports include
model identity, usage, latency and explicitly untrusted model output; a rejected
model reply exits nonzero with a JSON failure audit when a call was recorded.
Source revisions, scopes and forgetting are checked again after inference.

Assessment citation protocol `host-passages-v1` gives the model short IDs such
as `p3`; only the host constructs the source revision, field, half-open UTF-8 byte
range and exact quotation. Each field is shown completely in adjacent passages
of at most 1024 bytes (512 passages maximum; input budgets still apply). IDs are
local to the receipt, never persistent memory IDs. Repeated/unknown IDs and the
old model-authored citation objects are rejected. Correct binding alone does not
prove the selected passage supports the judgment. Concept generation retains its
separate original-field quotation protocol.

`assessment_calibration.py` freezes six synthetic cases covering satisfied,
contradicted, unknown and safely guarded prerequisites, including two misleading
route analogies. A deterministic fixture writer supplies the same provisional
concept and counterexample; this isolates evaluation from concept-generation
errors. Build and run it only with a resident local model:

```sh
cargo build --manifest-path prototypes/memory/Cargo.toml --example calibration_fixture
python3 prototypes/memory/assessment_calibration.py \
  --binary /ABS/PATH/mecha \
  --fixture-writer /ABS/PATH/calibration_fixture \
  --output /ABS/PATH/NEW-calibration-results \
  --model RESIDENT_MODEL --holds-dir /ABS/PATH/holds
```

The manifest is saved before inference; expected statuses and revision rubrics
never enter the evaluator input. `report.json` includes failed trials, token
counts and latency. Missing usage stays null, with known-token subtotals and
coverage separate; recorded-call counts exclude attempts lost to process timeout.
`review-template.json` requires independent review of the
actual revisions against the authored constraints: improved, unchanged,
regressed or invalid, with new assumptions and unnecessary timing/steps noted.
A model recommending its own revision is deliberately **not** scored as a gain.
These are development diagnostics, not a leaderboard or generalization result.

At source `687eaa7ea`, the first Spark calibration completed five of six workflows
and got four initial prerequisite judgments correct across all six trials. All 54
returned quotations matched their original byte ranges, but one analogy assessment
failed for concept-only factual evidence and another mislabeled a known blocker
as unknown. Revision review was mixed, including extra steps on an already valid
plan. See `docs/HISTORY.md` for conditions and limitations; valid citations do not
establish semantic correctness or an improved policy.

The earlier evaluation protocol `constraint-assessment-v2` asks the model for prerequisite and
factual checks, plus concept applicability only when a provisional concept was
retrieved. Otherwise Mecha adds `not_applicable` with `judgment_source: host`;
model judgments are labelled `model`. Model output has separate original-evidence
`citations` and provisional `concept_citations` arrays, each with its own allowed
IDs. The host validates these categories and returns resolved citations together.
Known blockers take precedence over unrelated unknowns in the evaluation prompt;
minimal repairs must retain valid conditional branches and explicit action guards.
These instructions remain model reasoning, not a host entailment proof.

Use `assessment_calibration_v2.py` with the same arguments as the original runner
above for the earlier V2 protocol; use the V3 option below for the current binary. It reuses the frozen six cases and supports skipped
revision without counting it as a new or improved plan. The original runner and
its results remain unchanged; its mandatory-revision protocol expects an older
binary. Replays of these cases are development diagnostics, not fresh holdouts.

At source `ea16b5304`, V2 completed all six workflows and matched five initial
prerequisite labels. Both adequate plans were retained unchanged; the known shared
clearance case still failed semantically and its revision was over-endorsed. All 59
returned quotes were exact. Fourteen model calls used 20,633 input/3,966 output tokens;
fewer calls than before did not reduce input-token cost. The runner currently
expects V2 successful output and does not gracefully grade older binary protocols.
See `docs/HISTORY.md` for the separate artifact review and comparison limits.

Core APIs are `prepare_plan_assessment`, `finish_plan_assessment` and the
provider-neutral async `evaluate_plans`; callbacks receive `EvaluationCall`.
`assessment_trial.py` contains three development integration cases, including a
concept-conditioned customs case. Gold labels grade outputs only. The trial does
not prove model judgment quality or counterfactual simulation accuracy.

The runtime `GoalMemory` schema now restricts `serves` to its host-confirmed goal
and recommends omitting it. Invalid IDs still fail closed with a repair hint;
the ordinary tool without memory opt-in remains unchanged.
The knowledge snapshot stays fixed for the invocation, while relation validity
is checked at the host's current time on each memory call. Reusing the tool does
not keep expired relations current or admit records captured after its snapshot.
Current source restrictions still apply; model input cannot set the policy clock.

## Intentional limits

This is a compact executable subset, not the full proposed 26-table production
schema. Relations use typed JSON plus revision/dependency/search tables to test
semantics before optimizing relational projections. No schema upgrade path is
promised; fixtures are disposable.

Do **not** import private data: encryption is explicit and database-only, while
existing plaintext fixture entry points remain available. Source suppression uses
an instance-salted digest rather than a secret-key HMAC; production key recovery,
retention, external-file protection and permission management remain unqualified. Filename and
domain checks prevent common mistakes; they do not classify arbitrary supplied
text as synthetic or form a filesystem security sandbox.

Also deferred: actual mecha-graph export/import and paired retrieval evaluation,
automatic identity resolution, model extraction/generation retirement, semantic
or vector ranking, persistent query receipts, date-only/approximate/recurring-event reasoning,
scenario branches, autonomous concept selection/evaluation, live appraisal/rumination integration, task storage,
background job leases and all nightly activity migration. `read` deliberately
dereferences old evidence; callers needing current beliefs must use `search`.
An unknown or forgotten reference currently returns `None`; richer unavailable
reasons are needed before production integration.

The model comparison below measures one bounded query-expansion candidate.
Vector retrieval and matched-compute context ablations remain future comparisons. Private-data evaluation still requires encryption, source-fidelity
inventory and a protected snapshot bridge. Keep the current graph authoritative
until paired evaluation and forgetting/restriction propagation pass.

## Verify

```sh
cargo test --manifest-path prototypes/memory/Cargo.toml --offline
cargo clippy --manifest-path prototypes/memory/Cargo.toml --offline --all-targets -- -D warnings
cargo fmt --manifest-path prototypes/memory/Cargo.toml --check
cargo test -p mecha-cli --features memory-prototype --test memory_cli --locked
cargo test -p mecha-cli --test memory_cli --locked
```

The full repository still requires its normal build/test/clippy checks. The
initial missing pinned `monty-pool` dependency was resolved by fetching the
existing lockfile; no dependency versions changed. Workspace tests need local
socket and subprocess permissions; sandbox denials are not passing checks.

## Local model comparison (synthetic only)

The main CLI's `evaluate-model` command compares lexical retrieval, goal-aware
query expansion, and oracle evidence on 16 newly authored cases across eight
sessions. Four pairs keep the episodes and question fixed while changing goals
or resources. The other cases test direct recall, corrections, time filters and
abstention. The prior 32-case fixture is unchanged.

```sh
mecha memory --experimental --db /tmp/comparison.memory-prototype.sqlite \
  evaluate-model --endpoint http://127.0.0.1:8080 \
  --model ALREADY_RESIDENT_MODEL --holds-dir "$HOME/.mecha/holds" --seed 7 \
  > /tmp/comparison.json
```

Run this on the machine serving the model. It requires an explicit loopback
llama-server router and checks the model is resident before each inference;
it refuses to load or switch models. `--holds-dir` explicitly supplies that
router's existing coordination directory. A pending switch fails the request;
forced switches cancel an active request. No global/project config, production
memory, tools, hooks, learned rules or connectors are loaded. Holds are transient
coordination records, not captured memories. No services are restarted.

`memory::model_evaluation::run` owns the experiment; the CLI only supplies
isolated `QuarantinedPass` completions, with no thinking, temperature 0.6,
seed 7 by default, 8,192 output tokens and a 180-second request timeout. No retries
or fallback model are used. Failed/truncated/malformed calls remain failed rows;
the command outputs a diagnostic report even when rows fail, so inspect row
errors rather than interpreting exit zero as a successful model evaluation.

All arms share the same answering prompt and a maximum of four whole evidence
records within an 8,000-byte serialized evidence-array budget. Lexical search
uses the question; goal-aware search adds one model call proposing at most two
short keyword queries. Fusion uses reciprocal ranks over eligible results,
with each search retaining the same account, session, occurrence interval,
snapshot and access scope. The oracle receives the specified supporting records
through the normal authorized read path, checked against time and byte limits.

The report retains prompts, replies, exact evidence revisions, query plans,
errors, fixture digest, seed/sampler settings, latency and reported token counts.
A deterministic scorer checks a selected answer/action and its citations,
including abstention. It does **not** grade arbitrary prose, execute plans,
measure energy cost, learn concepts or establish compositional generalization.
Extra searches also spend extra model compute: this comparison cannot attribute
any improvement specifically to goals without a later matched-compute ablation.
This authored set is held out from the earlier diagnostic, not an independent
benchmark; once used for tuning, treat it as development data.

The strict local adapter requires a numeric loopback host (for example
`127.0.0.1`), disables proxies and redirects for inference and router probes,
and leaves ordinary provider behavior unchanged. Token usage inherits one
existing provider limitation: when only one usage field is reported, a missing
counterpart may appear as zero. Both counters absent appear as unknown. Do not
interpret partial provider usage as a complete compute-cost measurement.

## Evidence-informed search follow-up

Add `--follow-evidence` to `evaluate-model` to compare question-only search
planning, evidence-informed planning, and oracle support with constrained
citations. This protocol defaults to twelve new synthetic cases; add
`--development` to debug on the original sixteen cases. The legacy command and
its frozen results remain unchanged without `--follow-evidence`.

```sh
mecha memory --experimental --db /tmp/hop-development.memory-prototype.sqlite \
  evaluate-model --follow-evidence --development \
  --endpoint http://127.0.0.1:8080 --model ALREADY_RESIDENT_MODEL \
  --holds-dir "$HOME/.mecha/holds" --seed 7 > /tmp/hop-development.json
mecha memory --experimental --db /tmp/hop-holdout.memory-prototype.sqlite \
  evaluate-model --follow-evidence \
  --endpoint http://127.0.0.1:8080 --model ALREADY_RESIDENT_MODEL \
  --holds-dir "$HOME/.mecha/holds" --seed 7 > /tmp/hop-holdout.json
```

Both search conditions perform the same initial recall and spend one planning
call plus at most two additional searches. Only the evidence-informed planner
receives the initial evidence, letting it follow named alternatives or
prerequisites. The initial search is reused, not executed twice. Both conditions
share anchor-first reciprocal-rank fusion, so incidental neighbor context cannot
displace a direct search hit. Initial and final evidence are each capped at
four complete records and 8,000 bytes; filters and conservative taint are retained.
This is an experimental retrieval policy, not a general graph traversal API.

Every planner and answer request carries a llama.cpp JSON schema. The answer
schema lists all visible citation IDs and public choice keys, without gold
answers or gold-only filtering. The prompt explicitly requires copying those
IDs. Host grading independently rejects duplicate, unavailable, irrelevant or
missing support; schemas do not establish factual support. Empty evidence forces
an empty citation array. No model text creates queries with broader access,
executes tools, or writes inferred memories.

Reports identify `protocol`, `dataset_split`, fixture digest, initial/final
records, proposed queries, per-call schemas, errors, usage and elapsed time.
The two search conditions match call/search/output limits, **not input-token
use**: exposing initial evidence spends additional prompt tokens. Oracle is a
one-call diagnostic. Compare the two new conditions to isolate the information
shown to the planner; comparing against the old protocol also changes citation
handling and the shared ranking policy. Keep development results separate from
new-scenario evaluation, and preserve the first held-out result before tuning.


## Required-mention retrieval diagnostic

`Memory::recall_mention(query, mention, access)` constrains each anchor to a
literal name or phrase before top-k selection. Descriptive query terms still
rank the matching records. Mention matching uses unicode61 case/diacritic and
punctuation normalization without stemming; it does not resolve aliases or
distinguish homonyms. A mention is limited to 256 bytes and 32 alphanumeric
tokens. Empty/invalid mentions fail instead of broadening recall. Adjacent
context need not mention the name and still passes its own access checks.
Ordinary `recall` and both model comparison protocols remain unchanged.

Run the deterministic diagnostic through the main CLI:

```bash
cargo run -p mecha-cli --features memory-prototype --   memory --experimental --db /tmp/mention-replay.memory-prototype.sqlite   evaluate-mentions > /tmp/mention-replay.json
```

The database must be new. No model, endpoint, global config or live store is
used. All 12 previously evaluated cases are replayed with the saved
evidence-informed searches. The treatment uses explicit **manually annotated**
names for the six alternative-comparison cases, checked against each saved
query and initially visible message text. Other searches are unchanged. This
isolates a retrieval capability given correct bindings; it does not measure
automatic extraction of those bindings or a new held-out reasoning score.

Both conditions use one initial plus at most two additional searches, two
anchors per search, and the same four-record/8,000-byte final evidence cap.
The report includes annotation/source hashes, per-query anchors, final IDs,
coverage and elapsed retrieval time. Required-mention searches construct an
additional eligible-only RAM index; equal search counts do not mean equal
compute costs. Large-corpus cost remains unqualified. See research §9 for
results. The existing model reports and strict support scores are unchanged.


## Automatic name-binding comparison

`evaluate-model --auto-bindings` runs one source-grounded planner per case,
then compares ignoring versus enforcing its proposed name bindings. It also
answers with oracle evidence. Use `--development` for the previous 12 cases;
the default is a new 14-case fixture. `--auto-bindings` and `--follow-evidence`
are exclusive; omitting both preserves the first model comparison.

```bash
mecha memory --experimental --db /tmp/bindings-dev.memory-prototype.sqlite \
  evaluate-model --auto-bindings --development \
  --endpoint http://127.0.0.1:8080 --model RESIDENT_MODEL --holds-dir /path/to/holds
mecha memory --experimental --db /tmp/bindings-fresh.memory-prototype.sqlite \
  evaluate-model --auto-bindings \
  --endpoint http://127.0.0.1:8080 --model RESIDENT_MODEL --holds-dir /path/to/holds
```

Each of up to two planned searches carries `query`, nullable `mention` and
nullable `source_id`. A bound name must appear in the query and original text
of the cited initially visible episode. The host uses the same non-stemming
unicode61 phrase semantics as recall. Empty/unknown/unsupported names or mixed
null fields reject the plan for both search arms; they never trigger a broad
fallback. A null pair explicitly requests an ordinary search. Source grounding
does not prove that the name is relevant or unambiguous.

Reports group results by case. `planner_calls` records the shared call once;
`rows[].calls` records each answer. Sum those for actual experiment cost.
Initial retrieval time is also shared; row retrieval times exclude it. Both
search arms use the same initial packet, planned queries and four-record/
8,000-byte cap. Arm order rotates across cases. Authored target-name recall
(for comparison cases only), plan failures, evidence coverage, choices, exact
support scores, citation-ID membership and abstention-format consistency are
reported separately. No new support rubric or repair changes old scores.

This is a synthetic diagnostic with strict local resident-model transport,
no tools, no retries and no implicit config. It does not integrate runtime
appraisal, import private data or demonstrate general entity resolution.


### Requirements and guards (historical V4)

`requirements-assessment-v4` adds a bounded requirement list per candidate:
1-based step, condition, universal/specific/unknown scope and its description,
met/unmet/unknown current state, original passage citations, and an explicit
nullable guard. A guard selects the complete existing step verbatim from its
candidate-specific vocabulary; the host requires that it match the indexed step.
Every requirement, including an unresolved one, cites original task/rule evidence.
The host checks membership and exact text, not whether the model interpreted them
correctly. V3 allowed free-text guard fragments and overly broad step numbers; its
failed model replay is retained rather than silently retried.

`requirement_status` aggregates these model-extracted records: an unguarded unmet
condition dominates unknowns; unknown scope or an unguarded unknown condition
withholds consideration. The existing model checklist can independently withhold
it too. Unknown outcome alone does not block a permitted information request;
contradicted outcomes still require revision. A recommendation means consideration
of next steps, not proven goal completion or execution permission. Omitted
requirements, scope semantics and guard adequacy remain model-quality risks.

The explicit `assessment_calibration_v2.py --expected-protocol
requirements-assessment-v4 --cases fixtures/requirements_development.json` option
runs eleven synthetic scheduling, approval and data-access cases, including
timeout-bypass and unguarded-tail probes. The original
six cases and default V2 expectation remain available. Protocol/shape failures
stay failed rows; raw model prerequisite labels and host-combined labels are both
reported. Expected labels and reviewer rubrics are kept out of evaluator input.
Goal identifiers are neutral: the original run exposed diagnostic case names in
this field, and the model sometimes cited them as evidence. Preserve those original
artifacts separately; the neutral-goal replay is still development data. These are development cases,
not a blinded holdout or evidence of executed outcomes.

V4 remains unqualified: the eleven-case neutral-goal replay completed 7/11 workflows
(3/11 completed with the correct initial label), compared with V2's 11/11 (10/11).
Fourteen Python checks and 84 standalone unit +2 CLI checks pass, but they do not
establish semantic quality. See `docs/HISTORY.md` and retained local
`results/memory-requirements-20261105/` for all failures, cost and artifact review.
Do not use model recommendation as action permission.

### Bound step repair (V5)

`bound-repair-v5` replaces free-text replacement plans with a bounded edit record:
`{"candidate":"candidate","edits":[{"step":1,"operation":"gate"}]}`.
Operations are `keep`, `gate` and `defer`, constrained per existing step. The host
assigns requirement IDs and copies every bound condition and its scope into a
whole-step gate. The model cannot add new actions, change condition text, reorder
steps or append a timeout fallback through this interface. Unknown scope or an
oversized gate leaves defer available. Invalid edits and no-op repairs fail closed.

The returned `repair` maps each generated step back to its original candidate,
step and requirement IDs. `all_deferred` marks abstention; no recommended revision
is returned in that case. This deliberately cannot generate a new information
request or alternative; it qualifies a small repair contract first. It is not a
world-model planner, execution permission, or proof that inferred requirements
are relevant. Initial assessments still preserve recommended candidates.

Routing goal IDs are retained only in audit context, not citable model input.
The event must describe the substantive task. Repair inputs omit operational
storage metadata. Unknown checks require a question in both schema and host.
Use `assessment_calibration_v2.py --expected-protocol bound-repair-v5 --cases
fixtures/requirements_development.json` for the neutral development replay.
The grader verifies the repair map and records abstention separately from a
recommended revision. Older reports remain under their original protocols.

### V5 qualification (2026-11-05)

Bounded repair is implemented, but semantic qualification failed. The neutral
11-case replay completed 11/11, with 4/11 correct initial prerequisite labels,
2/4 adequate originals preserved, and one unsafe original recommended unchanged.
Exact quotations and retained requirement IDs did not establish condition polarity
or action coverage. Keep this synthetic and advisory. See `docs/HANDOFF.md` for
the proposed typed action/control-flow fixture experiment and `docs/HISTORY.md`
for source hashes, costs, retained failures and citation-schema regression results.

## Explicit action/branch qualification

`memory::guard_coverage::check(&Contract, &Plan)` is a pure structural checker.
The contract declares scoped condition IDs, snapshot states and action prerequisites
with separate rule applicability. A plan names actions and branches into explicit
`when_met`, `when_unmet`, and `when_unknown` sequences. Every reachable action
occurrence is checked; guards do not escape their branch. Unknown applicability
is unresolved, and no reachable actions is reported separately from coverage.

This increment has no main CLI command, model call, storage write, runtime hook,
or action executor. The small standalone adapter is a qualification harness for
core, not a separate Mecha product. Run the frozen synthetic cases:

```sh
cargo build --manifest-path prototypes/memory/Cargo.toml --bin guard_coverage
python3 prototypes/memory/guard_coverage_trial.py \
  --binary prototypes/memory/target/debug/guard_coverage \
  --output /tmp/mecha-guard-trial-new
```

The output directory must be new. Each invocation sends only `contract` and `plan`
to the checker on stdin; expected labels remain in the runner. Reports preserve
each action's JSON location, path assumptions, uncovered conditions, and whether
that path matches the supplied snapshot. Current observations do not substitute
for future guards. Input is capped at 64 KiB, 32 conditions/actions, 16 requirements
per action, 128 plan nodes and depth 8; malformed hidden arms are rejected too.

The 16 development cases include the unguarded tail, inverted/unknown conditions,
wrong-target permission, timeout fallback, multiple prerequisites, explicit local
exemption, permitted inquiry, dead branches and malformed references. These
fixture-authored contracts supply additional information. Correct coverage does
not demonstrate automatic extraction, semantic truth, useful goal progress,
complete real-world constraints or execution authority. Conditions are immutable
in this experiment; live observations, effects and invalidation are future work.
The V5 language evaluator and its recorded failures remain unchanged.

### Source-linked action and branch extraction

`memory extract-plan` translates explicitly supplied synthetic evidence and plan
paragraphs into the guard checker's contract/tree. It **does not repair** the plan.
In V2, each condition, action, requirement and plan node carries inline passage
IDs; Mecha derives the report pointers and constructs the original-text citations.
Raw JSON with duplicate object members is rejected before any conversion. The host rejects missing links, wrong passage kinds, foreign IDs
and malformed/budget-exceeding structures. Exact original text and hashes establish
source membership, **not semantic entailment or complete extraction**.

```sh
cargo build -p mecha-cli --features memory-prototype --locked
python3 prototypes/memory/extraction_trial.py \
  --binary target/debug/mecha \
  --model RESIDENT_MODEL \
  --holds-dir /explicit/router/holds \
  --output /tmp/new-extraction-trial
```

Run on the machine already serving the selected model at numeric loopback :8080.
No model switch, API credentials, live configuration, database reads/writes, tool
execution or production integration. The inherited `--db` argument passed by this
trial is an unused placeholder and must not be created. A direct command requires
`--experimental`, `--synthetic-input`, an explicit JSON file, endpoint, model and
holds directory. Input is `{"evidence":[{"source_id":"synthetic:revision",
"text":"Original evidence"}],"plan":["Original plan paragraph"]}`.

The frozen `fixtures/extraction_development.json` cohort has 12 development cases.
Only `input` goes to extraction; rubric and expected structural labels stay in the
runner/review artifacts. The runner preserves per-case raw replies/errors and usage,
input/case/harness/binary hashes. `host_valid` and `coverage_label_matches` cannot
fill `faithful_extraction`: independent review must judge action completeness,
condition identity/state, prerequisite bindings, branch scope/polarity/order and
citation entailment. The typed system still assumes immutable conditions; no
live refresh or action effects exist. Both contract and plan are model-authored.
An empty or underconstrained translation can therefore pass host checks without
being faithful. No successful diagnostic may authorize an action.

### Representation and reasoning diagnostic

`extract-plan --direct` returns an untrusted source-linked coverage judgment in
`direct_judgment`; the normal mode keeps V2 extraction in `extraction`. Optional
`--thinking` requests reasoning; default remains off. Optional
`--thinking-budget 512` bounds that request through the existing provider field
and requires `--thinking`. The accepted range is 1–4096, reserving at least half
of the 8192-token total allowance for the answer; it does not change server settings.
Output records returned
reasoning character count only. This baseline answers structural guard coverage,
not immediate action permission, and never opens the `--db` placeholder.

Run `extraction_comparison.py --binary /explicit/mecha --output /new/result-dir
--model <resident-model> --holds-dir /explicit/holds` on Spark. It rotates the
four extraction/direct × fast/thinking arms over the frozen development inputs.
Both thinking arms explicitly request a 512-token reasoning cap; the manifest
retains the separate inherited server budget.
Server process arguments must expose a positive reasoning budget below the total
8192 output cap; read-only `/props` uses `autoload=false`. The manifest records
configured budget and props; every arm checks unchanged props/budget. Inherited
`MECHA_LOG` is forced to `warn` to exclude provider debug reasoning. Interrupted
preflights mark `progress.json` aborted; only complete matrices support four-arm
summaries. Failures retain their rows and null unknown usage. Independent review
must distinguish matching labels from source-supported explanations or faithful
extraction. This is reused synthetic development data, not a holdout benchmark.

### Experience → decision → outcome → recall

`mecha memory --experimental --db /new/experience.memory-prototype.sqlite
workflow-trial --synthetic-input --endpoint http://127.0.0.1:8080
--model <already-resident-model> --holds-dir /explicit/holds` emits one JSONL row
per completed call, then a final report. The fixture host explores three methods
for four synthetic workloads, storing distinct event/proposal/observed-outcome
captures with dependency edges. It closes and reopens SQLite before recall.
Three goal/resource contexts per workload are tested in rotated no-memory,
retrieved and oracle arms (36 calls). Oracle supplies observations, not answers.
The comparison corpus is frozen before evaluation so later arms cannot benefit
from earlier evaluation outcomes. The current CLI uses fast inference, 2048 output
tokens, seed7, no retries, and the existing local-model holds/residency checks.

`python3 prototypes/memory/experience_trial.py --binary /explicit/mecha
--output /new/result-dir --model <resident-model> --holds-dir /explicit/holds`
preserves streamed rows, raw replies, prompts, source hashes/revision receipts,
usage, trace references, host outcomes, binary/source provenance and final
readback. A 1200-second runner deadline preserves completed rows and records
missing rows; interrupted UTF-8/JSON cannot discard previous complete lines.

Outcomes are deterministic **fixture observations**, never a model's self-score.
The model may choose a known method or abstain; it cannot specify a cost, success
flag or external action. Before writing, the host checks the sealed world/store
receipt and the current authorization/freshness of every consulted source,
including uncited alternatives and indirect ancestors. All three records commit
atomically. Scope inheritance and forgetting use existing store operations.

Score feasible/optimal outcomes separately from appropriate abstention,
evidence coverage and complete citations. No-memory abstention is appropriate
uncertainty. Citation completeness does not verify free-text explanations.
These authored development fixtures have exact-name search and deterministic
repeatable outcomes; they do not establish held-out transfer, extraction quality,
concept learning, changing-environment recovery, or production action authority.

`workflow-trial --recall-views` compares original outcome text, a deterministic
measurements-only projection, and measurements plus historical feasibility bound
to the original event state. The original source bytes/hash are retained beside
the projected text/hash; scoped event context has its own source receipt and is
included in pending/saved decision dependencies. The prompt and output schema
remain unchanged. This is a projection of fixture records, not automatic semantic
extraction. The ordinary command preserves the original three-arm experiment.

`workflow-trial --unseen` uses a fixed measurements view on four new workloads
and values. Vireo lacks one method observation; Willow's Nori measurement is
corrected from 1 to 7 minutes before the evaluation snapshot. These test appropriate
abstention and current-source selection, not online revision of an earlier model
decision. Both options are mutually exclusive. Each protocol makes 36 matched
calls in a fresh DB. The runner selects these with `--experiment views` or
`--experiment unseen`; default `baseline` remains available. Unknown/missing usage
and incomplete matrices remain explicit. `justified_decisions` counts complete-
citation optimal choices plus appropriate abstention, not explanation entailment.


`workflow-trial --checked` (runner `--experiment checked`) freezes a new 12-case
measurements-view comparison. Baseline and checked policies share each initial
model draft exactly; only the checked policy can make one reconsideration call.
The fixture-specific check requires all three observations and citations, recomputes
energy/quality feasibility under current state, and advises on suboptimal choices.
It uses supplied observations, never hidden simulator outcomes. An inadmissible
final proposal is withheld; a feasible suboptimal choice may proceed and lose.
Abstention is allowed even when unnecessary. Transport and malformed reply errors
remain failures. Raw proposal, both attempts, checks and actual outcome are retained.
This produces 24 rows from 12–24 actual calls; per-policy token totals include
shared first-call cost, while `actual_*` totals count it only once. Host withholding
is separate from appropriate model abstention and grounded task success. The check
does not validate prose entailment, extract contracts or grant execution authority.
The protocol is mutually exclusive with `--recall-views` and `--unseen`.


### One bounded observation request

`workflow-trial --observe` (exclusive with the other workflow flags) runs the
`experience-observe-v2` synthetic diagnostic. The runner accepts
`--experiment observe` with the same binary/output/model/holds-dir arguments as
other workflow trials. Twelve shared requests feed control/observe branches:
24 checked final decisions, at most 36 inference calls, no retries. Elm/Grove can
supply missing measurements; Fern/Heather report unavailable. Grove/Heather first
supersede an old Nori measurement with a correction notice. Original history is
retained, and every final decision depends on the acquisition trace. Evidence is
explicitly source-selected; this does not benchmark retrieval ranking. Diagnostic
units are reported separately from task energy, and withholding is not model
abstention. No production action authority or live data is involved.


Add `--reconsider` to `workflow-trial --observe` for at most one reconsideration
per final decision (`experience-observe-v3`). The runner selects this with
`--experiment observe-reconsider`. It preserves v2's initial prompts and source
receipts, records the draft/check/retry separately, and commits only the final
checked result. There are 36 initial calls and at most 24 extra calls; failed retries
stay errors and host withholding is distinct from model abstention. This is a
reused-case development comparison with additional inference cost.


### Durable guarded read (synthetic runtime)

This is a separate runtime qualification, without model calls or production config.
Create a new v3 store with `memory --experimental --db FILE init-durable --domain
synthetic-demo`. Existing `init` and v1/v2 stores are unchanged; durable operations
require v3. Import an explicit synthetic JSONL session as before, then use its
`object:revision` from recall as a task dependency.

Every lifecycle command starts with:

```text
mecha memory --experimental --db FILE durable-read \
  --synthetic-input --domain synthetic-demo --scope fixture-mail
```

Append one operation:

- `start --workspace DIR --file relative.txt --source OBJECT:REVISION` binds the
  request and returns a task ID without reading the file. Defaults:128 lines,
  16384-byte tool-result budget; explicit limits are available.
- `resume --task ID --workspace DIR` reads once through the guarded dispatcher,
  or reuses the stored result. The current workspace must match. If execution
  was interrupted after reservation but before settlement, it stays indeterminate
  and spent; this command does not replay it.
- `status --task ID` reads operational state only, including erased-task tombstones.
- `cancel --task ID` prevents subsequent work/commit while retaining an actual
  observation under its existing source permissions.
- `finish --task ID --proposal JSON` retains one untrusted supplied proposal against
  the original current evidence. It reserves a single attempt before committing;
  it neither runs a model nor certifies the proposal or grants action authority.

Scopes/domain identify current host authority, not facts inferred from input text.
All commands need explicit synthetic acknowledgment. No default task/session/store
is discovered. Read results preserve actual provenance and conversation taint.
Content remains in the revision store; the lifecycle row holds opaque identifiers,
phase and spent allowances. Forgetting a supporting source erases dependent task
content and prevents a late result from recreating it. Original source files and
previously emitted CLI JSON remain separate artifacts; local forgetting cannot erase
those exports. No production encryption or general action scheduler is claimed.


## Durable model decisions (explicit schema v4)

Create a new database with `memory --experimental --db NEW.memory-prototype.sqlite
init-model --domain synthetic-demo`. This does not migrate any earlier database.
Import synthetic provenance and use the existing `durable-read start/resume` to
retain a successful read. The v3 supplied-JSON `finish` path is unavailable in v4.

The file must be strict JSON with `kind: "host_authored_synthetic_measurements_v1"`,
`workload`, `complete: true`, and `observations` containing zero to three unique
Nori/Arden/Quill measurements (`choice`, `observed_minutes`, `energy`, `quality`).
Numbers are unsigned integers. `complete` terminates the format; it does not prove
that FsRead returned the whole original file. Provenance hashes the received bytes.

```sh
mecha memory --experimental --db /tmp/new.memory-prototype.sqlite durable-decide \
  --synthetic-input --domain synthetic-demo --scope fixture-mail bind \
  --read READ_ID --contract '{"workload":"Synthetic task","objective":"minimize_minutes","energy_budget":10,"minimum_quality":6}' \
  --model RESIDENT_MODEL --seed 7 --view observed
mecha memory --experimental --db /tmp/new.memory-prototype.sqlite durable-decide \
  --synthetic-input --domain synthetic-demo --scope fixture-mail run \
  --task MODEL_ID --endpoint http://127.0.0.1:8080 --holds-dir /explicit/shared/holds
```

`status --task MODEL_ID` and `cancel --task MODEL_ID` use the same explicit authority
prefix. Status returns lifecycle metadata even when revised/restricted sources
prevent content readback. `--view none` provides a paired evidence-free control;
its sealed read is still validated, but no measurements enter the model prompt.
The model/seed/contract/view are sealed at bind. The transport is resident-loopback
only with zero HTTP retries, 8192 output tokens, no thinking, and a 180-second limit.

`memory_decision_runtime::advance` performs one resumable step; `decide` drives up to
five steps: initial call, pure assessment, optional revision, reassessment, final.
Every call spends its own allowance before inference. A response is saved before
checking it, permitting deterministic recovery after a crash. An interrupted call
without a saved response stays spent/indeterminate; cancellation can retain a
started response but cannot authorize more assessment or a recommendation.

Malformed/refused/wrong-model/oversized responses are retained as failures with no
repair call. Only a valid draft with source-check findings permits one revision.
No recommendation executes a method or becomes an observed outcome. The checker
covers the fixed synthetic arithmetic contract, not general semantic correctness.

`durable_model_trial.py` freezes eight paired none/observed tasks before inference,
with sixteen initial allowances and at most sixteen revisions. It independently
scores source-supported optimum, appropriate abstention, host withholding, errors,
citations and actual available usage separately. Results do not measure general
retrieval quality, policy learning, or benefit on real tasks.

## Durable synthetic actions (explicit schema v5)

Create a new database with `memory --experimental --db NEW.memory-prototype.sqlite
init-action --domain synthetic-demo --synthetic-input`. Earlier schemas are not
migrated. Import only synthetic source records, perform the existing guarded read,
and finish a `durable-decide` workflow in this v5 store first. The source envelope
and model contract must both name the closed workload `durable-action-fixture-v1`.
A recommendation is still advisory until the host binds an explicit fixture:

```sh
mecha memory --experimental --db /tmp/new.memory-prototype.sqlite durable-act \
  --synthetic-input --domain synthetic-demo --scope fixture-mail bind \
  --model MODEL_WORKFLOW_ID \
  --spec '{"fixture":"measurements_v1","state":"baseline"}'

mecha memory --experimental --db /tmp/new.memory-prototype.sqlite durable-act \
  --synthetic-input --domain synthetic-demo --scope fixture-mail run --task ACTION_ID
```

`--model` here is the saved model workflow ID, not a provider/model name.
`status` and `cancel` accept the same action ID. There is no endpoint argument,
provider discovery or inference in `durable-act`. The closed fixture state is
`baseline`, `refused` or `unknown`; neither a tool/path nor a method can be supplied
in its spec. The method is independently authenticated from the saved model reply,
request, source measurements and host check. Abstained/withheld/failed decisions
cannot bind an action.

`memory_action_runtime::reserve` returns an opaque live `PendingAction`;
`execute` consumes it and returns an opaque actual `ExecutedAction`; `settle`
retains that result. This boundary permits interruption/recovery tests without a
model-directed execution callback or production fault-injection flags. `advance`
performs one resumable action-or-feedback step, and `run` drives at most two steps.
A lost reserved action remains indeterminate and spent. A saved outcome can resume
pure feedback without another action. Cancellation may retain an already-completed
handler result but blocks further evaluation; changed/restricted/forgotten sources
refuse late content.

The simulator supplies fixed actual measurements independently of source claims:
Nori `(minutes=4, energy=9, quality=8)`, Arden `(8,3,9)`, Quill `(6,5,6)`.
`expected_from_evidence` remains a separate optional field, not a numeric model
prediction. Feedback checks actual energy/quality against the original contract;
it does not establish optimality or free-text rationale correctness. Unknown and
refused outcomes have no fabricated measurements and produce `Pending`/`Refused`,
not a measured policy failure. Saved `StepFeedback` has no invented task-board goal
and does not repurpose model-call count fields.

The outcome is source-linked evidence that ordinary recall can return under its
current authorization and dependency rules. This does not wire v2 into production
appraisal, promote a lesson, execute external tools or demonstrate learning gains.
Forgetting a lifecycle sibling erases the read/model/action content and suppresses
its capture keys; exported trial reports remain outside database erasure.

The deterministic qualification runner is `durable_action_trial.py --binary
/path/to/feature-enabled/mecha --output /new/output/directory`. It uses an authored
loopback response server and reports scripted HTTP requests separately from zero
real model inference calls. Its expected-versus-actual mismatch, pass, refusal,
unknown and abstention cases exercise the actual CLI, restart and later recall.

Current freshness is transitive: revising a consulted source makes its dependent
actual outcome and feedback ineligible too. This prototype does not yet separate
historical causal provenance from present evidential support.

## Reappraising a recalled outcome (read-only paired evaluation)

`evaluate-outcome-recall` compares the same original source expectations with and
without one actual retained action observation. It requires a finalized synthetic
v5 action and the same workload; it neither creates a new action nor revises the
original episode. The actual observation must appear in bounded ordinary recall
and match the authenticated operational revision and bytes. A recall miss fails
preparation rather than silently substituting a direct lookup.
The retained action must come from an authenticated recommendation. In the
current three-method fixture, incomplete measurements yield a withheld choice
or an abstention; neither can bind an action or reach this paired evaluation.

```sh
mecha memory --experimental --db /tmp/new.memory-prototype.sqlite \
  evaluate-outcome-recall --synthetic-input --domain synthetic-demo \
  --scope fixture-mail --action ACTION_ID \
  --contract '{"workload":"durable-action-fixture-v1","objective":"minimize_minutes","energy_budget":4,"minimum_quality":7}' \
  --query 'Nori' --model RESIDENT_MODEL --prepare-only
```

The contract is an explicit synthetic decision contract with the original workload
and current energy/quality constraints. Save the exported snapshot and pass its
JSON through `--snapshot` on every later preparation and run. Remove
`--prepare-only` and supply explicit `--endpoint http://127.0.0.1:8080` and
`--holds-dir /path/to/holds` to make one first-response call per arm. `--seed`
defaults to 7; `--outcome-first` reverses the order. There is no repair call or
implicit production configuration. Current source checks run again after model
discovery and after each inference; source invalidation aborts the pair without a
new source-bearing report. Ordinary model failures remain failed rows.

Original expectations and actuals remain visibly distinct. For this same-workload
fixture only, a numeric actual supersedes its expectation in current arithmetic.
Refused/unknown outcomes supply no numeric override. Historical feedback and old
recommendations do not enter either prompt. All comparison passages must be cited,
including for a proposed abstention; a withheld proposal is not a correct model
abstention. Returned reports are private, untrusted, advisory and `executed:false`.

The independent `outcome_reappraisal_trial.py` runner consumes an existing saved
mismatch database and action. It freezes five pairs at seeds 7 and 19 before
calling the resident model, threads the exact snapshot through every invocation,
compares full preparations including raw recall, and refuses reused output paths.
It scores visible-evidence reasoning separately from hidden simulator performance;
the fifth pair deliberately contains an unobserved error in an alternative's
source quality. That diagnostic must not be described as a reasoning failure when
the answer follows all visible evidence. The runner records cost, missing usage,
failed or incomplete calls, and database content before/after. These development
cases do not establish generalization or equal-token efficiency. This evaluation
has no durable inference-resume contract and does not replay an interrupted pair.

### Constraint application and abstention diagnostic

`evaluate-outcome-recall --response-format choice-first` opts into protocol v3
with explicit inclusive energy/quality constraints, an explicit empty-feasible-set
abstention rule, and a 4,000-character reason limit. `--response-format
feasibility-first` additionally requests `candidate_checks` for all three methods:
effective minutes, energy, quality, source ID and separate constraint/feasibility
Booleans. The host reconstructs these independently; `checks_consistent:false`
withholds the proposal while retaining its original choice. Correct checks cannot
make an infeasible choice admissible. This is still read-only, untrusted advice.

Optional `--thinking-budget` accepts 1–4096 only with a response format and turns
on bounded thinking within the 8,192-token output allowance. The diagnostic uses
2,048. Reports retain requested controls and returned `reasoning_char_count` per
row; a requested flag alone does not prove the model returned reasoning. Omit
both new flags to retain the original v1 protocol exactly.

The separate runner creates three new fixture stores through the existing durable
read/model/action CLI, with three authored HTTP replies and zero real inference:

```bash
python3 prototypes/memory/constraint_abstention_trial.py \
  --create-fixtures --binary /absolute/prototype/mecha --output /new/fixtures
python3 prototypes/memory/constraint_abstention_trial.py \
  --qualify-scripted --binary /absolute/prototype/mecha \
  --fixtures /new/fixtures/fixtures.json --output /new/scripted-qualification
```

Scripted qualification exercises all 24 corrective responses through the actual CLI and
compares captured HTTP prompts, schemas, order and generation settings against
frozen requests. This is transport/implementation qualification, not a model score.
To run the fixed development experiment with an already-resident local model:

```bash
python3 prototypes/memory/constraint_abstention_trial.py \
  --binary /absolute/prototype/mecha --fixtures /new/fixtures/fixtures.json \
  --endpoint http://127.0.0.1:8080 --model RESIDENT_MODEL \
  --holds-dir /absolute/holds --output /new/constraint-comparison
```

Four corrective contracts, seed 59, both evidence arms and three response treatments
produce 24 first responses. The initial run was stopped after a schema-order defect;
its 22 completed responses and two uncertain dispatch slots remain separate. The
corrective run keeps total possible calls within the original 48-call ceiling.
This one-seed diagnostic is not a holdout or a replacement for the missing rows. All preparations precede inference; interrupted runs
retain incomplete slots and are never resumed or repaired. The independent rubric
uses fixture evidence, not host assessments; model choice, model abstention,
check accuracy, host withholding and hidden simulator score remain separate.
Source/setting drift aborts; original stores are audited before and after.
See `docs/MECHA-MEMORY-DESIGN.md` section 28 for the design and `docs/HISTORY.md`
for measured results. The old outcome pilot is unchanged.

## Separate comparison provenance (opt-in response protocol v4)

`memory --experimental --db <explicit-v5-store> evaluate-outcome-recall` accepts
`--response-format feasibility-provenance` alongside the existing response modes.
All existing explicit action/contract/query/model/snapshot/endpoint/holds arguments
still apply. `--prepare-only` freezes the pair without inference. This is a
response/report version; it does not migrate the database.

V4 asks for the same feasibility-first candidate checks, but requires model
citations for every effective measurement, including alternatives ruled out and
an abstaining comparison. Extra supplied citations are allowed. A superseded
expectation can be omitted from the model's citation array without losing its host
lineage. Missing active sources and inconsistent checks remain withholding.

The original citation array and raw reply remain intact. `comparison_provenance`
records every supplied source revision/hash/role, effective method-source mapping,
model-omitted packet IDs and missing effective citation IDs. `reason_status` is
`unverified_model_prose`: arithmetic and source membership cannot certify the
explanation. Existing suboptimal-choice and unnecessary-abstention findings stay
advisory. Full packet access/freshness checks still apply, including superseded
material; this is not a fix for historical-outcome transitive freshness.

The v1 default and v3 modes/rubrics/results are unchanged. V4 qualification uses
authored HTTP replies, not a claim of better model reasoning. The separate frozen
multi-seed runner below tests the v4 contract without rescoring those earlier responses.

CI uses `python3 prototypes/memory/ci_qualification.py --binary target/debug/mecha`
after `cargo build --locked -p mecha-cli --features memory-prototype`. The runner
creates its own disposable synthetic fixture through authored loopback replies,
sets the explicit qualification environment before discovery, and refuses empty,
skipped or failing suites. It makes no real model calls and needs no owner store.
Ordinary developer unittest discovery retains explicit skips when those fixture
variables are absent. The exact fresh-fixture runner passed 81 tests on Spark.

## Frozen paired memory benefit diagnostic

`memory_benefit_trial.py` implements `memory-benefit-pilot-v1`: six authored cases,
seeds 71/83, two evidence arms and fast/thinking2048, for 48 first responses.
It preserves first replies and errors, refuses output-directory reuse, freezes
all preparations before dispatch, and checks fixture/binary/helper/model drift.

```sh
python3 prototypes/memory/memory_benefit_trial.py --create-fixtures \
  --binary /absolute/path/to/mecha --output /new/path/fixtures
python3 prototypes/memory/memory_benefit_trial.py --qualify-scripted \
  --binary /absolute/path/to/mecha --fixtures /new/path/fixtures/fixtures.json \
  --output /new/path/authored-qualification
python3 prototypes/memory/memory_benefit_trial.py \
  --binary /absolute/path/to/mecha --fixtures /new/path/fixtures/fixtures.json \
  --endpoint http://127.0.0.1:8080 --model <already-resident-model> \
  --holds-dir /explicit/router/holds --output /new/path/real-comparison
```

Fixture creation and qualification use authored loopback replies, not inference;
actual observations come only through the durable host handler. Preserve fixture
paths and exact runner/helper/binary identities between qualification and the real
run. The last command performs real inference without loading a model or retrying.

Visible-evidence choice correctness uses each arm's own packet. Paired memory
benefit scores both choices against one shared reference: source expectations with
only the retained actual substituted. Unobserved alternatives remain expectations;
full hidden-simulator optimality is a separate secondary diagnostic. Effective
citations, checks, host withholding and independent explanation fidelity are
separate from these choice scores. Metadata-invalid reports retain raw responses
but cannot earn qualified success. This small authored diagnostic measures neither
production learning nor superiority over v3. See MECHA-MEMORY-DESIGN section 30
and HISTORY for the dated run and limitations.


## Equal-evidence concept and revision pilot

`memory_concept_workflow` reuses the existing concept and rumination APIs in one
explicit synthetic lifecycle: induce once, run three baseline/concept decision
pairs, supply a counterexample and revise once, then run three new pairs. Both
arms receive the same original evidence and current facts. Only the provisional
concept differs. No owner store, native captured source, instruction-producing
learner, nightly job or production configuration is involved.

Build the feature-gated example and supply a fresh absolute output root, the
selected already-resident model and explicit router holds directory:

```sh
cargo build -p mecha-core --features memory-prototype --example memory_concept_trial
target/debug/examples/memory_concept_trial --synthetic-only \
  /absolute/new/cohort http://127.0.0.1:8080 \
  qwen3.6-35b-a3b-uncensored /absolute/router/holds 70510 \
  "$PWD/mecha-core/src/memory/fixtures/concept_benefit_pilot.json"
python3 prototypes/memory/concept_benefit_trial.py \
  /absolute/new/cohort/report.json --output /absolute/new/scores.json
```

The example performs real inference: at most fourteen first responses, seed 70510,
temperature 0.6, thinking disabled, 6144 output tokens and 540 seconds per stage
including discovery. It refuses existing roots, model mismatch and pending
switches; it never loads a model or retries. Qualify the code, public fixture,
offline gold and engine schemas before running. All fourteen planned stages and
twelve decision denominators survive failures. A durable started checkpoint is
indeterminate after a crash, not permission to resume. The unchanged transport
ceiling can reject a response smaller than the token allowance.

The report preserves exact requests, raw replies, provider events, source/phase
hashes, structural admission and durable retention separately. Whole database
seals and current-source checks bracket each call. The offline scorer uses
independently authored finite-world rules and enumerates unknown observations;
a lucky match to a hidden state is not evidence-supported certainty. Choices,
premise coverage and host acceptance are distinct; prose remains unreviewed until
a separate audit. Costs include induction and revision. Authored-provider tests
verify these contracts without demonstrating model benefit.

The six explicit-rule cases can have a ceiling and do not establish general
transfer. The post-phase measures revised-concept utility against equal raw
evidence. Without an unchanged-concept third arm on those same cases, it cannot
isolate revision's causal effect. Unchanged/abstained rumination is labeled
unchanged-concept follow-up, never revision benefit. See the dated results in
HISTORY and the current next step in HANDOFF.
