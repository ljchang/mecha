---
title: Triggers
sidebar_position: 10
description: Prompts that run unattended on a cron schedule, and the five decisions that make an unwatched agent run safe.
---

# Triggers

`mecha trigger` runs a prompt on a cron schedule, unattended — the morning
briefing, the overnight inbox triage that stages replies, the calendar prep
before a meeting.

It is a small feature because everything a scheduled run needs already
existed: the [outbox](/docs/features/outbox) stages what would be sent, the
[interlock](/docs/features/security) refuses exfiltration, the
[sandbox](/docs/features/sandbox) confines `shell`, budgets bound the spend,
and the session recording feeds [reflection](/docs/features/learning).
`cron.rs` adds the clock, `trigger.rs` the store and the ledger, and the CLI
the runner.

```bash
mecha trigger add briefing \
  --schedule '0 7 * * 1-5' \
  --prompt "Summarise anything in my inbox that needs an answer today, \
            and what's on my calendar." \
  --catch-up 3h --notify 'notify-send "mecha briefing"'

mecha trigger list            # what is scheduled, and when each next fires
mecha trigger next            # upcoming fire times, running nothing
mecha trigger show briefing --last   # settings, recent runs, the last answer
mecha trigger run briefing    # fire now, without consuming the scheduled slot
mecha trigger tick --dry-run  # what would fire, and why
mecha trigger runs            # the ledger, newest first
```

Everything that shapes a run — `--provider`, `--model`, `--workspace`,
`--tool`, `--no-mcp`, `--max-turns`, `--max-output-tokens`, `--max-cost`,
`--yes` / `--read-only` — is already a global flag, and `add` records exactly
those into the trigger file. One vocabulary, whether you run it now or every
morning.

The action is a **prompt**, never a command. Scheduled commands are what cron
is for, and giving one a home here would mean re-answering how it gets
confined and which environment it sees.

## `tick` is the primitive; `daemon` is a loop over it

Being due is a function of the ledger and the clock, not of anything the
scheduler remembers. So a scheduler that was asleep, restarted, or never
started at all reaches the same answer as one that has been running all week:

```bash
mecha trigger daemon          # ticks once a minute, on the minute
# or, equivalently:
* * * * * mecha trigger tick  # a crontab line
# or a systemd timer pointed at the same command
```

Two things follow. `tick --dry-run` is an **honest preview** rather than a
second implementation of the schedule — it walks the same decision and prints
it instead of firing. And the daemon can be dumb: one process, no state, a
sleep to the next minute boundary. `scripts/mecha-triggers.service` is that
daemon as a systemd user unit:

```bash
cp scripts/mecha-triggers.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now mecha-triggers
loginctl enable-linger "$USER"     # so it runs while logged out
```

The unit's `WorkingDirectory` is the home directory on purpose, and its
`TimeoutStopSec` is sized to outlast a tool call in progress: SIGTERM reaches
the in-flight run itself, which stops at its next safe point, keeps the
partial answer, and still writes its ledger row.

## Five decisions

### 1. Due-ness is computed backwards

The primitive is `Schedule::prev_at_or_before(now)`, not `next_after`. It
names the most recent slot at or before now, and the trigger fires if that
slot is newer than the last one already accounted for.

A laptop closed for a week therefore wakes owing **one** briefing, not forty,
and a tick that arrives late has lost nothing. Iterating forward from the last
fire would have to enumerate every missed slot just to find out how many it
was about to throw away.

`catch_up` decides whether a stale slot still runs at all:

| Value | Behaviour |
|---|---|
| `always` (default) | Run the missed slot whenever it is noticed. systemd's `Persistent=true`. |
| `never` | Only run a slot that is still fresh. |
| a duration (`3h`) | Run a missed slot if it is younger than that. |

Both extremes are legitimate and neither is a safe default for the other: a
nightly rumination wants to catch up whenever the machine comes back, and a
07:00 briefing delivered at 23:00 is noise. A small grace window applies to
`never` and to short durations, because the scheduler ticks on the minute and
a slot is always some tens of seconds old by the time anything looks at it.

A skipped slot is **written to the ledger**, with the reason:

```
missed by 9h 12m, past catch_up = 3h
```

"Why did I not get my briefing" has to be answerable.

The first fire is anchored on `created_at`, so a trigger added at 08:00 does
not immediately fire for this morning's unfired 07:00. A hand-written file
with no `created_at` is anchored to its own mtime, rather than firing for
every slot since the epoch.

### 2. Triggers live in `~/.mecha/triggers/`, never in the layered config

`[[hook]]`, `[[mcp]]` and `[[subagent]]` are all declarable in a project's
`mecha.toml` — a file that arrives with a cloned repository. A trigger is a
*scheduled unattended agent run*, so a repository that could declare one has
been handed a cron slot on your machine.

:::danger
For the same reason, a trigger run loads `Config::load_global()` — the global
file only, no project layer. A scheduled run must not inherit the tool surface
of whatever repository the daemon happened to be started in.
:::

Definitions are one TOML file per trigger, which you can edit by hand
(`mecha trigger edit <name>` opens it in `$EDITOR`), and every fire appends to
`runs.jsonl` beside them. Storage follows the outbox's rules:
temp-sibling-and-rename for writes, an advisory flock for read-modify-write,
an append-only ledger. `MECHA_TRIGGERS_DIR` overrides the location.

```toml
# ~/.mecha/triggers/briefing.toml
schedule = "0 7 * * 1-5"
prompt = "Summarise anything in my inbox that needs an answer today."
timezone = "America/New_York"
enabled = true
permission_mode = "read-only"
catch_up = "3h"
timeout = "20m"
notify = 'notify-send "mecha briefing"'
```

### 3. Read-only unless the file says otherwise

`permission_mode` defaults to `read-only`, and `mecha trigger add` writes
`allow` only when `--yes` was passed. Nobody is watching to approve anything,
and `ask` in that situation means "deny with a message telling you to pass
`--yes`", which is useless advice at 03:00. (`ask_user` is absent from an
unattended run by construction: it is only ever registered by a front-end that
owns a human.)

Note what read-only does **not** block: an **outbox-routed call still stages**,
because staging executes nothing. Draft-my-replies-overnight needs no
privilege at all — which makes the safe shape also the useful one.

`tools = [...]` narrows further, and is the sharpest control available: a
briefing that can read mail and nothing else cannot be talked into anything
else.

The model is told the situation it is in, because three facts change what a
good answer looks like: nobody is there to answer a question, anything
outbound is a draft rather than a send, and the answer will be read later, out
of context, by someone who has not seen the conversation.

### 4. A manual run is evidence, not a fire

`mecha trigger run briefing` records a ledger row with **no slot**, so it never
advances the marker and never cancels tomorrow morning's fire. Testing a
trigger must not silently disarm the schedule it was testing. The "last
accounted-for slot" scan simply ignores rows without a slot.

### 5. One run per trigger at a time

A non-blocking `flock` is claimed before firing. If it is already held, the
tick records a `skipped (overlap)` row and moves on: a five-minute trigger
whose run takes six minutes skips rather than stacking into an unbounded
fan-out. The kernel releases the lock if the process dies, so a crashed run
does not wedge a trigger forever.

The per-trigger `timeout` (`20m` by default) **cancels rather than aborts**:
the run stops at the next safe point and keeps its partial answer, exactly as
Ctrl-C does. Killing the future would throw the work away and leave a tool
mid-call.

`mecha trigger cancel <name>` stops a run in flight the same way. It works by
writing a sentinel file that the runner polls every two seconds, **not** by
sending a signal — the run may be inside the daemon's own process, where
SIGTERM would take the whole scheduler down.

## The cron parser

Five fields — `minute hour day-of-month month day-of-week` — plus the
`@daily` / `@hourly` / `@weekly` / `@monthly` / `@yearly` aliases.

It is hand-rolled, and that is the point. Every available crate speaks
Quartz's six-or-seven-field dialect where the **first field is seconds**, so
`0 7 * * *` — what a person types, and what every crontab on the machine means
by "seven in the morning" — parses as something else entirely rather than
failing. A scheduler that silently fires at the wrong time is the worst shape
of bug this project keeps finding.

Details that follow the same rule:

- `@reboot` is **rejected**, not accepted-and-ignored. There is no boot to
  hang it on, and a schedule that parses but never fires is exactly the
  failure this module is shaped to avoid.
- Vixie cron's day rule is reproduced: when *both* day-of-month and
  day-of-week are restricted, a day matches if **either** does.
  `0 0 13 * 5` is "the 13th, and every Friday", not "Friday the 13th".
- A schedule that can never match — `0 0 30 2 *` — terminates the search after
  four years rather than looping forever.

## Daylight saving, in both directions

The schedule is wall-clock, resolved in an IANA timezone, and wall-clock time
is not monotonic:

- **Spring forward.** A daily 02:30 job has no 02:30 to run at, so it fires at
  the first instant that exists after the gap. Late, not lost — a job that
  silently skips a day twice a year is a job you cannot trust.
- **Fall back.** The local time happens twice, and the earlier instant wins,
  so the job runs **once** rather than twice.

This is why the zone is an IANA name throughout and never an offset: an offset
is wrong twice a year.

The timezone is written into the trigger file at `add` time, resolved from
`[agent] timezone` then. Editing that config later cannot silently move every
existing trigger.

## The TUI modal

In `mecha tui`, `/triggers` is the same thing with a keyboard: the list shows
what is scheduled and when it next fires, `enter` opens the prompt, settings,
recent runs and the last answer, `e` edits, `space` enables or disables, `r`
runs one now, `c` cancels one that is running, `x` deletes.

**The modal drives the CLI, not the store.** Every action shells out to
`mecha trigger ...` as a child process. Firing builds a whole separate agent —
its own provider, tool surface, workspace, budgets — and can run for twenty
minutes, so doing it on the event loop would freeze the interface. Going
through the CLI also means one implementation of firing, and no way for the
TUI to do something the command line cannot. The detail view reads the last
answer back from the **session transcript**, which is the record; a second
copy could disagree with it.

Two details there that cost something to get right:

- **Asking "is a run in flight?" must not use the flock.** `try_claim`
  acquires and drops, so a UI polling that question would occasionally hold
  the lock at the instant the scheduler fired, causing a spurious overlap
  skip. Watching must never perturb what is watched. A separate advisory
  `<name>.running` marker carries UI state beside the real lock.
- **A marker whose pid is gone reads as not running**, so a hard kill cannot
  leave a trigger looking busy forever. The range check on that pid is the
  whole correctness of it: `kill(-1, 0)` succeeds, and without the check every
  dead run would report as alive.

## The ledger

Every fire appends one row: the slot, start and finish, status
(`ok` / `error` / `skipped (overlap)` / `skipped (stale)`), the session id,
turns, cost, `blocked_sends`, how many calls were **staged** (the number to
look at in the morning), the taint snapshot, the stop cause, and how the run's
tool calls went — `tool_calls`, `tool_errors`, `ended_on_failed_call`.

The stop cause is recorded because without it a run cut short by a timeout or
a budget records as plain `ok`, and a trigger that has been quietly truncating
its answer every morning looks exactly like one that works.

The call counts are there for the same reason, one level down. **An unattended
run has nobody watching it fail**: the briefing still arrives, the ledger still
says `ok`, and a trigger failing a third of its calls reads exactly like one
that works. Per-step reliability is what decides how long a task a run can
finish, so a third of the calls failing is not a third of the work lost.
`mecha doctor` is the reader, and it raises two findings from these counters:

- **A trigger failing a third of its tool calls** across its last five runs.
  Silent below ten calls in that window, because a rate over three of them is
  noise.
- **A trigger whose most recent run succeeded having done nothing.** The rate
  check cannot see this one — a rate over zero calls is undefined rather than
  bad — so a trigger that made thirty calls every morning and now makes none is
  silent in every other signal the ledger carries. Measured against the
  trigger's *own* earlier runs and never an absolute floor: a prompt that
  legitimately needs no tools makes zero calls every morning, and calling that
  broken would be wrong about the healthiest trigger on the machine.
  Suppressed when the run also errored, since that already has a finding.

See [Run quality](/docs/features/run-quality).

The full answer stays in the session transcript rather than being copied into
the ledger. `--notify` is a command handed that answer on stdin — an observer,
like `post_tool`: its failure is logged and never fails the run.

**`notify` runs in the run's [workspace](/docs/features/work)**, like a hook
already did. It used to inherit the daemon's working directory, and the shipped
systemd unit sets `WorkingDirectory=%h` — so the only way to put the answer
somewhere useful was to spell out an absolute path. That is how the shipped
morning briefing came to end in

```bash
mkdir -p ~/.mecha/briefings && cat > ~/.mecha/briefings/$(date +%F).md
```

which wrote outside every path jail, into a directory it created on the way
past, where no later run could read it back. The workspace is the answer to all
three.

## Where to go next

- [The work directory](/docs/features/work) — where an unattended run's output goes, and its path jail.
- [The outbox](/docs/features/outbox) — what makes overnight triage safe.
- [Security model](/docs/features/security) — the interlock, which applies unchanged to an unattended run.
- [Sessions and replay](/docs/features/sessions-and-replay) — where a trigger's answer actually lives.
- [Run quality](/docs/features/run-quality) — the corpus that makes an unattended run's reliability visible.
- [CLI reference](/docs/reference/cli) — every `mecha trigger` subcommand and flag.
