# Planning-check registration repair

**Goal:** remove phantom shell checks from ordinary runs whose registry has no shell, retaining explicit unverified completion and existing security dispatch for registered tools.

**Spec:** `docs/ARCHITECTURE.md` goal-system plan-prediction contract; `docs/HANDOFF.md` next MVP repair; the user's explicit continuation on 7 November. This is a small repair to the existing planning machinery, not a new executor or permission.

**Baseline:** `a162f7db1` source qualified on isolated Spark: workspace build/fmt/strict Clippy; serial default 6303/0/25 and memory 6667/0/29. Draft PR #846 and Site 0.28 publish the frozen first native comparison. All 24 attempts/112 calls retained; later exact artifacts 6/8 versus 4/8, three exhausted runs and 22 captures. Two native traces had harness-generated missing-shell checks and 18/19 passages beyond the 16-passage capture bound. Original scores and ambiguity remain unchanged.

## Constraints and rulings

- Keep the existing default-off memory prototype and isolated synthetic stores; no production install, service restart, owner-store change, mecha-graph/nightly migration or merge.
- Gate check admission only on stable registration plus existing config. A registered but restricted/denied/staged shell still goes through ordinary guards. Do not snapshot dynamic availability or infer permission.
- No new ToolCtx/config fields, checker trait, command parser, tool or grant. Reuse absent queue -> Verification::Skipped and the existing unverified-step accounting.
- No capture-limit changes. Failed or long episodes remain measured, not silently truncated or regenerated.
- Repeat the exact v1 development cases after the code repair in a fresh independently identified run; no prompt/schema/gold/seed/budget/order change and no selective reruns. Preserve the extra-field grading ambiguity. Schema clarification is future v2 work.
- The structural regression is primary proof. Historical before/after model results are descriptive, with fresh A experiences and possible stochastic/shared-server variation; no controlled causal or generalization claim.
- Parent owns docs/HANDOFF.md and docs/HISTORY.md, qualification, source/binary retention, PR and private Site. Existing authorization covers updates; no merge.

## Interfaces and shared-file scan

| Tasks | Interface | Decision |
|---|---|---|
| 1/2 | ordinary todo output and check dispatch | One bounded unverified notice, no phantom shell calls; no tool schema change |
| 1/3 | no-shell and registered-shell behaviors | Source tests prove skipped vs genuine guarded checks; real runs only assess workflow |
| 2/3 | Python helper/native executable | Cases and all frozen controls stay identical; add authored preflight coverage only |
| 1 | absent queue and freeze/tamper | Existing Skipped state and frozen command remain intact |
| 2 | new probe vs planned denominator | New authored probe is outside 24 model slots; no new real cases |
| 3 | baseline vs new result | Archive baseline helper/binary by hash before overwriting build target; original manifest untouched |

## Task 1 — ordinary planning repair

Owner files: `mecha-core/src/agent.rs`, `mecha-core/src/tool/todo.rs` and their existing tests. No docs edits or commits.

- First write old-fails regressions: checks enabled but no registered shell must not create harness `mecha-check-*` ToolUse or CHECK_TRACE/unknown-shell result; feedback Skipped, completed step still unverified, explicit todo notice.
- In per-run `Agent::run_in` setup create the existing step_checks queue iff cfg.step_checks and registry.get("shell").is_some(). Preserve registered-shell guarded dispatch, denied/staged behavior and check limits.
- TodoTool adds one notice per write when newly completed retained checks have no queue: `A declared plan check was not run; completion remains unverified.` No repeated notice for unchanged completed-plan echo. Mark as normal harness output, never external/error or success proof.
- Retain command freeze/tamper behavior under missing executor; standalone default ToolCtx still works. Reuse existing Pending->Skipped feedback and verified-set removal.
- Run focused ordinary agent/todo regressions and registered-shell enabled/disabled/denied regressions. Independent review before inference.

## Task 2 — native scripted qualification

Owner files: `prototypes/memory/bridge_model_trial.py`, `prototypes/memory/tests/test_bridge_model_trial.py`. No core/docs edits or commits.

- Add a separate authored preflight probe with todo pending/completed declared check under the same four-tool registry, then write an artifact and finish.
- Assert no harness shell ToolUse/CHECK_TRACE or unknown-shell text; explicit unverified notice; step remains unverified, never passed. Preserve existing seven probes and all24planned native preflight slots.
- Keep frozen_cases(), planned_pairs(), protocol, budgets, tool schemas and scoring unchanged. Tests pin case/input/oracle equivalence against retained baseline; do not introduce new real-model cases or metadata APIs.
- Observe meaningful red on old executable then green on repaired binary. Coordinate builds with Task1; no concurrent cargo or real inference. Report paths and hashes.

## Task 3 — parent integration, evaluation and publication

- Archive baseline helper/executable with manifest-matching hashes; do not rewrite original evidence.
- Independent task and whole-change review; fix major/medium findings. Verify same local/Spark source hashes in isolated checkout.
- Required fmt/build/strict Clippy/default+feature workspace tests; targeted example and authored native preflight on exact binary. Preserve failed runs/logs.
- Before real inference, write a dated follow-up identity/reference-manifest and unchanged-case contract, then freeze all 24 slots through unchanged protocol controls. Use the already-resident model and shared holds; no retry/load/switch.
- Report raw scores, abstention, completion, captures, retrieval exposure and declared-check handling beside baseline. No retrospective rescue of ambiguous cases or cap changes.
- Update source-grounded docs, PR #846 and owner-private Site with verified results and remaining gap. Required owner-corpus privacy scan before push; follow CI and automated review to no major/medium findings. No merge.
