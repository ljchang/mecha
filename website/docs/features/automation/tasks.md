---
title: Delegated tasks
sidebar_position: 1.5
description: The task board in the knowledge graph, handing a task to a background run, stopping and steering it, and answering the questions it parks.
---

# Delegated tasks

Your task board lives in the [knowledge graph](/docs/features/memory/graph), and
mecha can work items on it for you. Handing a task over starts a run in its own
conversation and workspace. You can stop or redirect the run while it works,
and if it needs a decision it parks a question for you instead of waiting. The
task stays yours throughout. The agent is delegated to, never assigned, and it
cannot close the task.

## The board

`mecha tasks` reads and edits the board through the same `kg_task_*` tools the
model uses. There is no separate copy of the board. A configuration with no
graph server reports that fact instead of showing an empty board.

```bash
mecha tasks                                   # the board, actionable first
mecha tasks add --due tomorrow -- Reply to the reviewers
mecha tasks set task-1a2b3c4d --status next
mecha tasks list --closed                     # include done and dropped
mecha tasks source task-1a2b3c4d              # read what the task was captured from
```

A new capture lands in `inbox`. The other statuses are `next`, `scheduled`,
`waiting`, `done` and `dropped`. Closing a task is reversible: setting any other
status reopens it, and nothing is deleted. The full flag list is in the
[CLI reference](/docs/reference/cli#tasks).

### Only you close a task

A model cannot move a task to `done` or `dropped`. Every model-facing copy of
`kg_task_update` is wrapped so that it refuses those two statuses and names
`mecha tasks set` as the command to use. It can still change due dates,
contexts and the other fields. The wrapper is applied before subagents are
built, so a run cannot get around it by delegating to a subagent. Delegated runs
go further: `kg_task_update` is removed from their tool surface entirely, and
`tasks work` refuses to start if a configured subagent allowlists it.

Closure goes through `mecha tasks set` because closing a task also appraises the
run that worked it. The transition into `done` or `dropped` builds an appraisal
from the delegated session and prints the verdict. See
[Closing a task appraises it](/docs/features/appraisal/reference#closing-a-task-appraises-it).

## Handing a task to a run

```bash
mecha tasks work task-1a2b3c4d
mecha tasks work task-1a2b3c4d --note use the revised budget figures
mecha tasks work task-1a2b3c4d --unattended
```

`tasks work` starts a fresh session titled `task: <name>`, seeds it with a
prompt built from the task record, and runs it in the task's own work
directory, `~/.mecha/work/<task-id>/`, unless you pass `-w`. The run gets up to
200 turns. It refuses to start in four cases:

- **The outbox is not configured.** A delegated run must stage its sends.
  Without `[outbox] tools` it refuses rather than sending for real.
- **The task is closed.** Reopen it with `mecha tasks set <id> --status next`.
- **A run is already working the task.** Stop it, or pass `--again` to start a
  second run alongside it.
- **The task's workflow was cancelled.** See [Workflows](#workflows) below.

While the run is going, the board shows the task as `waiting` on `mecha`. When
it finishes, the task moves to `waiting` on you, and the command prints how many
drafts it staged in the outbox. Nothing is sent. If the run fails, the task goes
back to the status it had before.

A terminal run is attended: it asks you before any tool call that needs
approval. `--unattended` uses the trigger posture instead. Reads run, sends
stage, and anything that needs approval is refused rather than left waiting for
nobody.

To take over a conversation you started elsewhere, such as a task planned in
the web chat, pass `--resume <session>`. The run continues that transcript,
including its taint, so changing hands does not clear what the conversation
already read. mecha refuses the hand-over if another live run is still writing
that session.

### Stopping and steering

```bash
mecha tasks stop task-1a2b3c4d
mecha tasks steer task-1a2b3c4d focus on the budget section first
```

These commands reach a run in another process through files in
`~/.mecha/taskruns/`. A running task has a `.running` marker there. `stop` writes
a `.cancel` file, and `steer` appends to a `.steer` file. The run checks both
about every two seconds.

- **Stop** does not kill the process. It cancels the run's own token, so the run
  ends at the next safe point and keeps its partial work, just as Ctrl-C does
  in a terminal.
- **Steer** queues your text. It is added to the message that carries the run's
  next tool results, so the model sees the results and your instruction as one
  turn and keeps going.

Both commands exit with an error if nothing is running on the task. If a run
crashes, its marker names a process that no longer exists, so mecha treats the
task as not running and cleans up the marker. A new run clears any leftover
cancel or steer file before it starts, so an old request cannot affect it.

## Parked questions

A delegated run has an `ask_user` tool. It does not wait for your answer.
Instead, it saves the question to `~/.mecha/questions/` and ends the run. The
partial work is kept, and the run stops holding a model seat while nobody is
there to answer. `tasks work` prints the question and the command that answers
it.

```bash
mecha questions                          # what is waiting on you
mecha questions show ab12cd34
mecha questions answer ab12cd34 use the revised budget
mecha questions abandon ab12cd34
```

Answering a question resumes the run. Your answer becomes the next user turn of
the conversation that asked, in the same workspace and with its plan restored.
The resumed run has the same restrictions as before: it still cannot close its
task, and it can park another question. `abandon` resolves the question without
resuming the run. Nothing is deleted, and `list --all` shows answered and
abandoned questions.

If the asking conversation had read third-party content, `list` marks the
question with ⚠, and `show` and `answer` warn you before continuing. An injected
model can ask a well-formed question too. The question will not resume if
another run is already working the task, so stop that run first.

`answer --unattended` resumes without terminal prompts. Use it when nothing is
reading a terminal. Anything that needs approval is then refused by policy
instead of being recorded as a denial from you. See the
[CLI reference](/docs/reference/cli#questions).

## Model seats

The local model server can only serve a few requests at once, so background
runs share a small pool of seats. There are three by default
(`DEFAULT_BACKGROUND_PERMITS`), which is one fewer than the server's four slots.
That keeps one slot free for your own turns. The seats are files in
`~/.mecha/permits/`. As with run markers, a seat held by a process that no longer
exists is reclaimed.

Only unattended runs take a seat. Chat, voice, Slack and an attended
`tasks work` never do. When the pool is full, an unattended run refuses to start
and does not queue:

- `tasks work --unattended` fails with *"the model is busy with N background
  run(s) (…)"*, names what holds the seats, and suggests running from a
  terminal, where the run is attended and does not need a seat.
- `questions answer --unattended` fails with *"your answer is saved; run
  `mecha questions answer` again when one ends"*.

## Workflows

Each delegated task gets a [workflow](/docs/features/automation/workflows) record
the first time it runs. The same record follows the task through `tasks work`,
a web chat on the task, and answered questions, and it collects the drafts and
questions each run leaves behind. Every one of those launches checks the
workflow first. If you cancelled it with `mecha workflow cancel`, or closed it,
all three are refused until you run `mecha workflow reopen`. The workflow also
refuses a new run while an earlier runner is still alive. Triggers do not read
workflow records, so cancelling a workflow does not affect scheduled runs.

## Dates in a capture

When you type or dictate a task in the web capture box, mecha looks for one date
phrase, such as `today`, `tonight`, `tomorrow`, `the day after tomorrow`,
`in 3 days`, `in 2 weeks` or a `YYYY-MM-DD` date. It shows the phrase as a chip
you can dismiss. The task name is saved exactly as you wrote it. The phrase is
passed to the graph's own `--due` parser, so mecha never works out the date
itself. Weekday names are not detected, and the due field stores only a date,
not a time of day.

## On the web

The **Tasks** tab of the [web surface](/docs/features/interfaces/web) is the same
board, with one tap per status change. It shows the task a run is working and
lets you stop or steer that run from there. Both buttons call the same `tasks stop` and
`tasks steer` commands.
