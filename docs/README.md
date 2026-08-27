# Which document holds what

This project keeps several kinds of writing, and they have different jobs.
Putting something in the wrong one is how a document becomes too long to
trust — the handoff reached 1,965 lines mostly by absorbing material that
belonged in three other places.

## The map

| Document | Holds | Tense | Grows? |
|---|---|---|---|
| [`README.md`](../README.md) | The front door: what mecha is, how to run it | present | slowly |
| [`CLAUDE.md`](../CLAUDE.md) | **Why** the code is shaped the way it is — invariants and the incidents behind them | present | slowly |
| [`AGENTS.md`](../AGENTS.md) | Orientation for an AI agent working here; points at CLAUDE.md rather than restating it | present | rarely |
| [`CONTRIBUTING.md`](../CONTRIBUTING.md) | Build, test, review expectations | present | rarely |
| [`SECURITY.md`](../SECURITY.md) | Reporting a vulnerability; the threat model and accepted limitations | present | rarely |
| [`OPERATIONS.md`](OPERATIONS.md) | This deployment's specifics — hosts, keys, mailboxes, timers. **Gitignored**; the transferable lesson goes in `HISTORY.md` instead | present | with the machines |
| `MAIL-CORPUS-RESEARCH.md` | What a year of the user's own mail measured. **Gitignored** for the same reason as `OPERATIONS.md`: no correspondence, but aggregates of one mailbox are still its owner's. The decisions it produced are in `MAIL-UX-DESIGN.md` without the figures | past | once |
| [`HANDOFF.md`](HANDOFF.md) | Current state and **only the open work** | present | **only with open work — completed items leave** |
| [`HISTORY.md`](HISTORY.md) | What shipped and when; what was learned the hard way | past | append-only |
| [`TRIFECTA.md`](TRIFECTA.md) | The four ways a session assembles the lethal trifecta, which mechanism owns each, and every opt-in switch with its cost. Read before loosening anything | present | rarely |
| [`LLAMA-SERVER.md`](LLAMA-SERVER.md) | The local model server: slot geometry, the KV arithmetic, the measured `-np` table, and what each flag cost to learn | present | with the server |
| `*-RESEARCH.md` | One question, researched once, with evidence and a date | past | one per question |
| `*-DESIGN.md` | One thing, designed before it is built — the decisions, and what is deliberately not in scope | present, then past | one per thing |
| [`LEARNING-AUTONOMY-DESIGN.md`](LEARNING-AUTONOMY-DESIGN.md) | Why learning is ungated in every domain, what replaces the gate, and the cost of that in `behavior`. Read §3 before loosening anything further | present | once |
| [`GOAL-SYSTEM-DESIGN.md`](GOAL-SYSTEM-DESIGN.md) | The goal representation, the signed error signal it produces, and its three consumers. Read §7 before letting any disposition stand in for a structural check | present | once |
| [`CHANGELOG.md`](../CHANGELOG.md) | User-visible changes per release | past | append-only |
| [`website/docs/`](../website/docs) | User-facing documentation for the published site | present | with features |

## The index

Every research and design document, and the one question each answers. The
pattern rows above say what *kind* of thing these are; this says which one to
open. **Status is deliberately not repeated here** — each document carries its
own, and a second copy is how two documents start disagreeing about whether
something shipped.

### `*-RESEARCH.md` — one question, researched once

| Document | The question it went and answered |
|---|---|
| [`BENCHMARK-RESEARCH.md`](BENCHMARK-RESEARCH.md) | How to measure this harness against public agent benchmarks, and what separates harness from model |
| [`CANVAS-RESEARCH.md`](CANVAS-RESEARCH.md) | Can mecha reach Canvas LMS — and what Dartmouth's token policy makes impossible |
| [`CLOUD-HOSTING-RESEARCH.md`](CLOUD-HOSTING-RESEARCH.md) | What it would cost to run the model somewhere other than this box |
| [`CONTEXT-RESEARCH.md`](CONTEXT-RESEARCH.md) | What is actually established about context management, compaction and distractors |
| [`DOCS-RESEARCH.md`](DOCS-RESEARCH.md) | Which Google scope buys document access, and what each one costs in review |
| [`HARNESS-RESEARCH.md`](HARNESS-RESEARCH.md) | Where agent performance actually comes from — planning, the loop, or the tools |
| `MAIL-CORPUS-RESEARCH.md` | What a year of this mailbox actually contains. **Gitignored** |
| [`MAIL-UX-RESEARCH.md`](MAIL-UX-RESEARCH.md) | What the field has converged on for agent-driven email |
| [`MEMORY-RESEARCH.md`](MEMORY-RESEARCH.md) | Whether agent memory should accumulate or be curated, and what the evidence says |
| [`MESSAGING-RESEARCH.md`](MESSAGING-RESEARCH.md) | How separate mecha sessions should message each other, and what travels with a message |
| [`POLL-RESEARCH.md`](POLL-RESEARCH.md) | What polling products ship, and which parts are worth copying |
| [`PRIOR-ART-RESEARCH.md`](PRIOR-ART-RESEARCH.md) | What openclaw, codex and the other harnesses do that this one does not |
| [`PUBLIC-SURFACE-RESEARCH.md`](PUBLIC-SURFACE-RESEARCH.md) | How an agent should meet the world: artifacts, reports, a booking page |
| [`REMOTE-SURFACE-RESEARCH.md`](REMOTE-SURFACE-RESEARCH.md) | Once voice is a browser page, is Slack still the right remote surface |
| [`SANDBOX-RESEARCH.md`](SANDBOX-RESEARCH.md) | Which confinement backend to use, and what each one cannot close |
| [`SELF-IMPROVEMENT-RESEARCH.md`](SELF-IMPROVEMENT-RESEARCH.md) | Whether a harness can measure and improve itself, and where that goes wrong |
| [`SKILLS-RESEARCH.md`](SKILLS-RESEARCH.md) | What the Agent Skills standard is, and what published skills measured as carrying |
| [`SLACK-RESEARCH.md`](SLACK-RESEARCH.md) | How mecha should be driven from Slack, and which trust tiers that needs |
| [`SLIDES-RESEARCH.md`](SLIDES-RESEARCH.md) | What a presentation integration would have to reach, per platform |
| [`TASK-RESEARCH.md`](TASK-RESEARCH.md) | What a day of real use said about delegation and the task tier |
| [`TUI-RESEARCH.md`](TUI-RESEARCH.md) | What the good agent TUIs do that this one does not |
| [`VERIFICATION-RESEARCH.md`](VERIFICATION-RESEARCH.md) | What verification loops other agents run, and what mecha has instead |
| [`VOICE-RESEARCH.md`](VOICE-RESEARCH.md) | How the owner talks to mecha out loud, and from where |

### `*-DESIGN.md` — one thing, decided before it was built

| Document | What it decides |
|---|---|
| [`BRANCHING-DESIGN.md`](BRANCHING-DESIGN.md) | Branching a conversation, and why the TUI batch deliberately left it out |
| [`EXPERIMENT-DESIGN.md`](EXPERIMENT-DESIGN.md) | The instrument that states, from artifacts alone, what differed between two runs and what it cost. §5 depends on `BRANCHING-DESIGN.md`; issue #60 holds the communication policy question |
| [`FACTORY-DOCS-DESIGN.md`](FACTORY-DOCS-DESIGN.md) | The published documentation site and what belongs on it |
| [`GOAL-SYSTEM-DESIGN.md`](GOAL-SYSTEM-DESIGN.md) | What a run is *for*, the signed error signal that follows, and its three consumers. Read §7 before letting a disposition stand in for a structural check |
| [`LEARNING-AUTONOMY-DESIGN.md`](LEARNING-AUTONOMY-DESIGN.md) | Why learning is ungated per domain, what replaces the gate, and the cost in `behavior`. Read §3 before loosening anything |
| [`MAIL-UX-DESIGN.md`](MAIL-UX-DESIGN.md) | Mail as a surface you work: the phases, and what each settled |
| [`POLL-DESIGN.md`](POLL-DESIGN.md) | Polls as a general-purpose instrument — the six kinds and the lecture mode |
| [`PUBLIC-SURFACE-DESIGN.md`](PUBLIC-SURFACE-DESIGN.md) | The public surface: what mecha may publish, and under what review |
| [`REMOTE-CONTROL-DESIGN.md`](REMOTE-CONTROL-DESIGN.md) | One live TUI session and a named Slack thread as the same conversation |
| [`REMOTE-SURFACE-DESIGN.md`](REMOTE-SURFACE-DESIGN.md) | How the tailnet web surface gets built, and what it replaces |
| [`SCHEDULING-DESIGN.md`](SCHEDULING-DESIGN.md) | The scheduling instrument: booking, the admin door, the frontend |
| [`SLACK-ACTIONS-DESIGN.md`](SLACK-ACTIONS-DESIGN.md) | Executable actions from a phone: the closed `Action` enum and the tainted two-step |
| [`SLACK-DESIGN.md`](SLACK-DESIGN.md) | How mecha is driven from Slack: the transport, the allowlist, the thread state machine |
| [`SWITCHBOARD-DESIGN.md`](SWITCHBOARD-DESIGN.md) | The switchboard over the public surface |
| [`TASK-AGENT-DESIGN.md`](TASK-AGENT-DESIGN.md) | The medium tier: delegated tasks, the resource model, and who holds the ball |

## Where does this go?

Ask in this order; the first match wins.

1. **Would a user of mecha need it to operate a feature?** → `website/docs/`
2. **Does it explain why the code resists an obvious change?** → `CLAUDE.md`
3. **Is it a completed thing, or a lesson from a mistake?** → `HISTORY.md`
4. **Does a reader need it to decide what to build next?** → `HANDOFF.md`
5. **Is it the answer to one question you went and researched?** → a new
   `docs/<TOPIC>-RESEARCH.md`
6. **Is it a thing you are about to build, worked out before writing code?** →
   a new `docs/<TOPIC>-DESIGN.md`
7. **Is it a user-visible change in this release?** → `CHANGELOG.md`

If two of these match, it usually belongs in the *later* one, with a one-line
pointer from the earlier. Duplication across documents is how they drift into
disagreeing, and the reader has no way to tell which is current.

## Conventions per document

### `CLAUDE.md` — the expensive one

It rides in every agent's context on every run, so length there is a running
cost paid forever. Add to it only when a change would otherwise look like an
improvement to the next reader: an invariant that is not obvious from the
code, and the bug that would come back if it were undone.

State the rule, then the incident in one sentence. Not the other way round.

### `HANDOFF.md` — bounded by what it holds, not by how long it is

Current state and open work only.

**There is no line limit.** There used to be one, and it was the wrong
instrument: the file reached 1,965 lines by absorbing completed work and
material that belonged in three other documents, and a number cannot tell
those apart from a project that genuinely has a lot open. Trimming to hit a
target deletes real items instead of finishing them.

What keeps it trustworthy is the two rules underneath. Every open item must
have been verified unbuilt, against source, with `file:line`. Every completed
item **leaves** — moved to `HISTORY.md`, never struck through, because
strikethrough keeps finished work in the reader's way forever and that is
what actually produced the 1,965 lines. See the `handoff` skill
(`.claude/skills/handoff/`), which is the procedure for that pass.

Length is then a symptom to read rather than a rule to enforce: if the file
grew, ask what grew. More open work is honest. A section that has quietly
become an essay belongs somewhere else.

Do not add a "future ideas" section. Ideas with no verified gap behind them
are what research docs are for.

### `HISTORY.md` — append-only

Two things go here: what shipped (grouped by date, as prose) and what was
learned the hard way (grouped by area).

A trap entry is worth keeping only if it carries a **general lesson**. "The
hook timeout did not cover the stdin write" is a changelog line; "audit what
sits outside every timeout, not just what is inside it" is why the entry
exists.

Never edit a recorded measurement to match a later result. If a finding is
superseded, say so beside it and keep both — a retracted measurement is
evidence about how much to trust the next one. The compaction record does this
in place, and it should stay that way.

### `*-RESEARCH.md` — one question, dated

Name the file after the question, not the answer:
`MEMORY-RESEARCH.md`, `SANDBOX-RESEARCH.md`.

Open with the date, the question, and how it was researched. Mark the strength
of each claim — peer-reviewed, preprint, vendor blog, folklore — because the
whole point is to be able to tell later how much weight a finding can carry.

End with what it means *for this project* specifically, and be explicit about
what you are recommending against as well as for.

When something from a research doc ships, add an addendum saying so rather
than silently leaving the proposal in the present tense. A research doc that
describes shipped behaviour as unbuilt is worse than one that says nothing,
because it is the file someone reads before deciding what to build next.

### `*-DESIGN.md` — one thing, decided before it is built

Open with the date and one sentence on what is being designed. Record the
decisions and, just as importantly, **what is deliberately out of scope** —
that is the half a reader cannot reconstruct later, and the half that stops
the same argument being had twice.

Written in the present tense, and it stays that way while the thing is
unbuilt. When it ships, add a line at the top saying so and pointing at
`HISTORY.md`, rather than rewriting the body: the design as proposed is
evidence about how the built thing came to be shaped that way. A design doc
that describes shipped behaviour as unbuilt is the same failure as a research
doc that does, and for the same reason — someone reads it to decide what to
build next.

### `website/docs/` — the published site

Plain Markdown, front matter with `title`, `sidebar_position`, `description`.
Links are checked at build time (`onBrokenLinks: 'throw'`), so a stale
cross-reference fails CI rather than shipping.

Explain what a thing does *and* why it is that way — the rationale is what
makes the reference usable. Keep it accurate over complete: the reference
pages are verified against the binary's own `--help` and against
`ConfigLayer`, and they should stay that way.

**The overview layer is a separate job from the feature pages, and it rots
differently.** `intro.md` and `principles.md` answer *what this is for* and
*what rules it keeps*; the feature pages answer *how one subsystem works*.
Writing a good page for a new subsystem does not update the overview, so the
overview drifts by omission rather than by becoming wrong — which is exactly
what happened by 2026-08-10, when the whole site described a reusable harness
library and no page anywhere said the project exists to make a local
open-weight model into a personal assistant. When a feature changes what mecha
is *for*, the overview is part of the change.

## Facts that go stale

These appear in more than one document and rot silently. When you touch a doc
that states one, re-verify it:

| Fact | Check |
|---|---|
| Test counts | `cargo test --workspace` |
| Eval case and tag counts | count `eval/cases.jsonl` — do not trust a prose number |
| Command surface | `mecha-cli/src/main.rs`, or `mecha <cmd> --help` |
| Config surface | `mecha-core/src/config.rs` (and `ConfigLayer` — a field on one and not the other is a startup parse error) |
| Machine state (ports, timers) | `curl localhost:8080/props`, `systemctl --user list-unit-files` |
| Model IDs and prices | the provider's own documentation, not memory |

Anything you could not verify should say it is unverified rather than carrying
an old claim forward with confidence.

## Starting a new document

Before creating one, check the map above — most new writing belongs in a file
that already exists. A new document is justified when it answers a question
that is genuinely its own, and when you can say in one sentence what belongs
in it and what does not.

Add it to **the index** in the same commit — the map's `*-RESEARCH.md` and
`*-DESIGN.md` rows say what kind of thing it is, and only the index says which
one to open. A document nobody can find from here will be rewritten by the next
person who needs it, and 32 of them were unreachable from `CLAUDE.md` on
2026-08-26 for exactly that reason.
