# Personal assistant follow-through

2026-09-08. This design connects delegated work, review, delivery and verification
without moving the task board out of the graph or weakening the owner boundary.
Implementation authorized by the owner after the harness review.

Merged as PR #216 and installed from `main` at `c3f33f4c` on 2026-09-08; see
`HISTORY.md` and the verified update in `HANDOFF.md`. The first implementation accepts
commitments through owner commands, produces in-app reminders, and verifies
explicit workflow criteria. Automatic promise extraction/proposal acceptance,
external push notifications and a general in-loop convergence engine remain
future extensions. Tool profiles are opt-in with `--tool-profile`; capability guidance also accompanies
any configured outbox route so a minimal install knows how to stage drafts.
The daily priority list joins workflows, drafts and questions; existing dashboard
links still lead to the broader mail, calendar and board surfaces. Owner-action
counts measure requested verbs, not human time. Restart/ambiguous-failure scenarios
are deterministic tests; the model lifetime covers sequential reply/calendar work
and injection resistance.

The grounding follow-up adds explicit fixture time per task and rubric checks
against actual tool evidence. See `HISTORY.md` for the calibrated live results.
The rubric judge can detect unsupported claims, but is an evaluation tool rather
than a runtime truth guarantee. Mail rows name the owner-mailbox scope and unknown
recipient read status; ordinary assistant guidance limits claims to read sources.

## Decisions

1. Preserve global outbox routes and review-store identity across project layers.
2. Keep per-result provenance in local transcript metadata, never provider payloads.
   Old recordings remain unknown; replay conservatively treats unknown as external.
3. Before any remote release, durably record the delivery attempt. A crash or
   ambiguous failure leaves outcome unknown and blocks automatic resending.
   Reconciliation records evidence of delivery or non-delivery; a local record
   alone cannot promise exactly-once effects on an arbitrary remote API.
4. Core owns a workflow journal referencing the existing graph task, transcript,
   question and outbox IDs. It records checkpoints and partial effects; the graph
   continues to own tasks and the owner continues to close them.
5. Verification is evidence against explicit postconditions. It distinguishes
   pending review, failed, unknown and verified; a model stopping is not proof.
6. Commitments record source, responsibility, due time, next check and resolution.
   Model extractions are proposals until accepted. Follow-up is deterministic,
   deduplicated, respects quiet hours and produces a digest for the owner.
7. Structured output is a provider-neutral request capability with explicit failure
   when unsupported, adopted first for quarantined extraction and classification.
8. Tool profiles narrow a stable registry. Capability-dependent prompt text comes
   from the registry and cannot advertise unavailable tools or taint escape routes.
9. The daily view joins existing stores around urgency, decisions, completed work
   and waiting. It reports missing sources as unknown and explains ordering.
10. Assistant evaluations check final fixture state across changing plans,
    interruptions and failures, with repeated trials and owner-effort measurements.

## Verification and rollout

Each correctness fix has a regression reproducing its prior failure. Stores are
version-tolerant, locked for mutations and atomically persisted. Tests cover restart,
ambiguous delivery, duplicate action, stale evidence and denied approval. Provider
encoding tests and fixture-backed assistant evaluations supplement scripted loops.
The final checks are workspace build/test, formatting, Clippy, frontend tests and
build, documentation updates, and the repository handoff procedure.

The implementation phase did not deploy services. The owner subsequently authorized
the 2026-09-08 installation recorded in `HANDOFF.md`; no release was published.
Native provider idempotency is used only
where its contract is known; unsupported effects retain the reconciliation boundary.
