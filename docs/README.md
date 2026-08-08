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
| [`HANDOFF.md`](HANDOFF.md) | Current state and **only the open work** | present | **only with open work — completed items leave** |
| [`HISTORY.md`](HISTORY.md) | What shipped and when; what was learned the hard way | past | append-only |
| `*-RESEARCH.md` | One question, researched once, with evidence and a date | past | one per question |
| `*-DESIGN.md` | One thing, designed before it is built — the decisions, and what is deliberately not in scope | present, then past | one per thing |
| [`CHANGELOG.md`](../CHANGELOG.md) | User-visible changes per release | past | append-only |
| [`website/docs/`](../website/docs) | User-facing documentation for the published site | present | with features |

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

Add it to the table above in the same commit. A document nobody can find from
here will be rewritten by the next person who needs it.
