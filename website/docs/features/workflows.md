---
title: Workflows and Today
sidebar_position: 6
description: Track commitments, resume delegated work, verify results and review your daily priorities.
---

# Workflows and Today

Today brings urgent work, decisions, verified results and waiting work onto the
home screen. It includes pending drafts and unanswered questions even when you
have not created a workflow. A source that cannot be read is reported as unavailable.

Delegating a task creates a workflow automatically. The same record follows its
web conversation, background work and answered questions. It links the existing
task and conversation to drafts, questions and completion checks. After a crash,
partial drafts remain discoverable and the workflow shows the interruption.

## Track a commitment

For work outside a delegated task, create a workflow and use the returned ID:

```bash
mecha workflow add "Prepare the grant reply" --workspace ./grant
mecha workflow commit FLOW_ID --party "Priya" --source "owner instruction" \
  --due 2026-10-15T17:00:00Z --follow-up 2026-10-14T09:00:00Z
mecha workflow today
mecha workflow show FLOW_ID
```

Commitments are entered by you. Messages are not automatically treated as promises.
Times must include a timezone or UTC offset.

## Check the result

Specify what would establish completion:

```bash
mecha workflow check FLOW_ID --artifact reply.md --contains "tracked-changes version"
mecha workflow check FLOW_ID --delivered OUTBOX_ID
mecha workflow verify FLOW_ID
mecha workflow close FLOW_ID
```

Artifact paths are confined to the workflow's recorded workspace. Checks read
regular UTF-8 files up to 4 MiB. A delivery check requires a recorded successful
send; a staged draft or unknown delivery cannot pass. Today rereads the evidence,
and closing checks it again. A workflow with no checks is not marked verified.
A content check proves the specified text exists, not that an entire document is correct.

`workflow uncheck FLOW_ID 1` removes the first check. `workflow cancel FLOW_ID
--reason "Plans changed"` stops tracking without claiming success; `workflow
reopen FLOW_ID` restores it. Closing a workflow leaves graph task closure to
`mecha tasks set`.

## Continue work in order

```bash
mecha workflow depend FOLLOWUP_ID PREPARATION_ID
mecha workflow resume FOLLOWUP_ID
```

Dependencies must exist, cannot form cycles, and must be completed before a
successor starts. Resume continues the recorded task conversation in its workspace,
retaining approval and taint rules. Resolve outstanding questions and drafts first.

## Reminders that respect your day

```bash
mecha workflow attention --timezone America/New_York \
  --quiet-start 22 --quiet-end 8 --digest-hour 8
mecha workflow tick --dry-run
mecha workflow tick
mecha workflow snooze FLOW_ID 2026-10-16T09:00:00-04:00
mecha workflow ack FLOW_ID
```

The trigger daemon also runs the follow-up tick. Reminders are coalesced in-app,
deduplicated across restarts, and withheld during quiet hours. Unconfigured timing
uses UTC. A missed day produces the current digest, not a backlog of old notices.
Overdue work stays visible when its reminders are snoozed. The tick observes linked
changes and produces reminders; it does not start a model or send external messages.

## Resolve an uncertain delivery

If a send loses its response or its process stops, the outbox retains an unknown
attempt and blocks resending. Check the destination's history, then record what
you established in the outbox's web detail or the CLI:

```bash
mecha outbox reconcile DRAFT_ID --outcome delivered --evidence "Sent message m-123"
mecha outbox reconcile DRAFT_ID --outcome not-delivered --evidence "Destination check established no delivery"
```

Run only the command matching your finding. Reconciliation never sends. Confirming
non-delivery enables a fresh review and retry; confirming delivery resolves the draft.

## Choose a smaller tool set

```bash
mecha --tool-profile assistant chat
mecha --tool-profile research run "Research the public documentation"
mecha --tool-profile coding run "Fix the failing build"
```

Profiles narrow the actual registry once, compose with `--tool` and are inherited
by subagents. Research retains public readers; coding adds workspace/code tools;
assistant retains readers, selected private-work tools and configured staged actions.
The assistant profile excludes shell. Profiles can also be saved by `trigger add`.
They retain all existing approval, sandbox and taint protections.

## Structured extraction

After verifying support on your endpoint, set the provider's `structured_output`
to `json_schema` (OpenAI-shaped or Anthropic schema format) or `llama_json` (the
llama-server JSON-object/schema format). The default is `disabled`. Frontdoor and
mail classification then request constrained responses while keeping their tools
and conversation history absent. Semantic validation still runs.

The wire contracts follow [OpenAI structured outputs](https://developers.openai.com/api/docs/guides/structured-outputs),
[Anthropic structured outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs),
and the [llama-server documentation](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md).
Endpoint compatibility alone does not establish schema enforcement.

## Evaluate follow-through

`eval/assistant-lifetime.toml` runs five sequential fixture tasks over three seeds.
It checks delivered reply content, thread identity, calendar time and attendees,
and duplicate effects after the simulated owner reviews drafts. Trial output also
records requested owner-action counts. These counts include unsuccessful requests
and do not measure human time.

```bash
mecha exp new eval/assistant-lifetime.toml
mecha exp run assistant-follow-through
mecha exp export assistant-follow-through
```

The fixture world is isolated from live mail and calendar accounts. Restart,
ambiguous-delivery, stale-artifact and reminder-timing scenarios also have
deterministic workspace tests.


The assistant lifetime manifest also sets `[fixtures.clock]`: each task gets an
explicit simulated instant shared by the model's date prompt and the fixture
mail and board servers. The follow-up now occurs on the next simulated day.
This does not change the machine clock or audit timestamps.

Cases with `expect.judge` require an explicit `[judge]` provider and model in an
experiment manifest. The judge receives recorded tool evidence, and a failed or
unavailable judge fails its check. These checks supplement artifact checks;
model verdicts still need review. The assistant manifest uses the local Qwen
model as its judge, so its verdict is not independent of the model under test.
Run the opt-in calibration before interpreting its scores:

```bash
MECHA_GROUNDING_ENDPOINT=http://127.0.0.1:8080 \
MECHA_GROUNDING_MODEL=qwen3.6-35b-a3b \
cargo test -p mecha-core --test grounding_judge -- --ignored --nocapture
```
