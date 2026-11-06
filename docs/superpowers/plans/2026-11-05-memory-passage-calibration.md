# Memory passage citations and evaluator calibration

2026-11-05. Approved scope: make citations reliable, distinguish satisfied,
contradicted and unknown prerequisites, and measure one-step revision quality.
Base: `85c6196fe`. Execute inline; one independent final review.

## Design and constraints

The host divides each visible original field into contiguous UTF-8 passages
of at most 1024 bytes, preserving every byte and field order. Short passage IDs
are local to the opaque assessment receipt. Models select IDs only; the host
constructs source revision, field, byte range and exact quote. Long fields remain
visible as adjacent passages, never summaries. Invalid, repeated or legacy
citations fail closed. Correct binding does not establish relevance or entailment.
Concept generation keeps its existing independent citation protocol.

Keep the bounded, default-off, synthetic-only evaluator: no tools, execution,
trusted rules, new storage or production configuration. Recheck every consulted
source after each model call, preserve the frozen packet during revision.

## Tasks

1. Confirm baseline. Add failing passage-selection tests, implement host passage
   catalog and strict ID resolution; adapt existing tests and preserve security
   checks. Verify exact UTF-8 coverage, unknown/duplicate IDs and old wire rejection.
2. Freeze a small synthetic calibration set covering supported, contradicted,
   unknown and guarded prerequisites plus misleading analogies. Keep labels out
   of prompts. Record failures in the denominator. Grade statuses and citation
   relevance against authored references; review revisions against explicit
   case constraints, distinguishing improvement, unchanged and regression.
3. Run focused and required repository checks, then one fresh independent review.
   Fix substantiated important findings. Build and install separately on Spark,
   run the frozen set once with the resident model and retain raw artifacts.
4. Update existing prototype documentation and handoff/history with measured
   results and limitations. Commit on this branch; no merge or production restart.

## Completion evidence

Passing source/receipt tests and required workspace checks; actual CLI exercise;
Spark report including invalid responses and revision failures, model/seed/compute;
source-grounded handoff. No claim of general evaluator reliability from this set.
