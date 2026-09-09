# Attribution and context pilot

Registered before execution in [the manifest](../../eval/appraisal-attribution-v2.toml).
Runtime commit: `c7071ad3`. Conditions frozen immediately before execution on 2026-09-09.
[conditions.json](conditions.json) records the binary and fixture hashes and
server-confirmed `qwen3.6-35b-a3b` alias: four slots, 262,144 tokens per slot.
The full tracked source, including transitive fixture helpers, is pinned by the
commit. This is a local synthetic mechanism experiment, not live owner-policy work.

There are 72 trials: control and learning each execute twelve ordered tasks for
seeds 1, 2 and 3, with a fresh isolated home for each arm/seed. The first six tasks
provide training diagnostics; the last six form the predeclared unseen transfer
slice. The native gate's random selection/holdout split is a separate measure.
The native runner executes all control lifetimes before learning lifetimes; arm
order and shared server conditions are not randomized, which limits causal
interpretation of differences between arms.
Both arms have the same five file/planning tools, a sixteen-turn budget,
confirmed task goals, with step checks and learned-rule loading enabled. Goal guidance, MCP,
hooks, skills, messaging, fallback, escalation and outbox routing are off.
Enabled configuration is not exercised behavior: the registered tools are
`fs_edit`, `fs_list`, `fs_read`, `fs_write` and `todo`. With no `shell`, declared
checks cannot execute. The archive records two check declarations across all
72 trials and zero check executions in either arm. Learned-rule loading also
had no effect because no rule was created. This design does not measure the
benefit of executing checks or applying learned rules.

Each task resolves greatest revisions, selects approved records and sums signed
amounts, then evaluates a queue threshold from supplied context. Fixtures cover
below, equal and above threshold, plus missing queue evidence on transfer.
The missing-evidence answer is explicitly the string `unknown`. The tasks contain
an outdated summary as a distractor. All inputs are maintainer-authored local
synthetic files; this experiment does not establish safe learning from untrusted
network content. These flat-file tasks differ from the previous linked-file
pilot, so their scores must not be pooled or compared as a controlled before/after.

Only the first six tasks register artifact criteria. After the acting run ends,
the harness records pass/fail/skipped diagnostics for selection, total and review
priority. The priority diagnostic binds `/outbox_waiting` and `/review_threshold`
from the pinned, preserved `context.json` to the `greater_than` relation and the
owner-supplied `charter:review-pending` association. This is a pointer association,
not a read or edit of a live charter. Context support is currently bounded to
nonnegative integer counts; missing or changed context cannot become zero.

Feedback contains criterion identity and context evidence, not expected artifact
values or arbitrary output prose. It is added to the persisted session after the
run, outside the acting conversation; only the subsequent quarantined reflector
receives this diagnostic. Existing provenance gates remain: tainted/unknown
transcripts cannot supply clean mismatch learning. All twelve cases retain their
independent gold for grading, but the transfer cases have no registered criteria
and therefore produce no artifact-derived diagnostic feedback.

Call-count estimates, actual spans and completion-batch counts remain observations.
An overrun alone does not support a behavioral rule or establish unnecessary work.
The learning arm reflects after each task and validates, consolidates and retires
after positions six and twelve: 54 scheduled stages total. No reflections or rules
are seeded. The evidence minimum remains three. Native consolidation uses
`--holdout 0.25 --auto`: its deterministic every-fourth-ID selection reserves zero
when a pool contains only three reflections. Existing gates, probation and
retirement remain unchanged. A successful stage need not create a rule; recorded
configuration determines actual exposure on later tasks.

The runtime, fixture, manifest and bootstrap configuration are frozen throughout
execution. No task is excluded or retried to improve its score. The operator's
learning store, installed binaries, model defaults and services are not changed.
Task artifact correctness, native task grades, criterion feedback, causal claims,
rule creation, rule exposure and paired validation are reported separately.

The initial pilot was stopped after discovering a harness-voice mining defect.
Its records are archived separately and are excluded here. This replacement
uses fresh homes and no learning state or trial outcomes from that run.
