# Branching sessions — design, for review before any code

Written 2026-08-05, alongside the TUI feature batch that deliberately did
*not* build this. The TUI survey (`TUI-RESEARCH.md` §4) found two independent
implementations of the idea — codex's `Esc Esc` walk-back plus `/fork`, and
pi's in-place session tree — and concluded mecha is the project most ready
for it and least likely to build it. This file is the design that would make
it buildable, and the reasons it is not a weekend change.

## Why it is worth having

- **A fork is a counterfactual the user runs by hand.** `replay_run.rs`
  already drives a recorded prefix through a fresh agent; `counterfactual.rs`
  already exists on the premise that *an intervention is a test case* —
  "would the model do what the steer asked, without being steered".
  "Rewind three turns and say it differently" is the same machinery pointed
  at the user instead of the nightly learning pass, and the diff view for
  comparing the two branches (`replay.rs`, structural vs argument-only) is
  already written. No surveyed project has that half; they all have the UI
  half.
- **Rewinding composes with the prompt cache.** The transcript up to the fork
  point is a prefix of both branches, so the cached prefix survives the fork
  — rewriting the last exchange costs the suffix, not the session.
- **Recovering without destroying evidence.** Today the only exits from a bad
  turn are `/clear` (loses everything) or pressing on (the failed attempt
  stays in context, and the self-conditioning literature in
  `CONTEXT-RESEARCH.md` says 20–30pp of damage at long horizons for exactly
  that). A fork keeps the dead end recorded — the learning pass wants it —
  without making the model re-read it every turn.

## Format: branch in place, one file

pi's conclusion, adopted, and one project-specific reason on top: the
learning store keeps per-session mined/unmined bookkeeping keyed by session
id (`mined.jsonl`, `mined_outbox.jsonl`). Fork-to-new-file would either
double-mine the shared prefix or need cross-file dedup; branch-in-place
keeps one id, one file, one mining pass.

Today `Session::load` (session.rs:233) reconstructs the conversation purely
from **append order** — messages have no identity, so there is nothing a
branch could point at. Two additions:

1. **Message ids.** `Record::Message` grows an `id` (monotonic within the
   file — `m0`, `m1`, …) and a `parent` (the id of the message it follows;
   `None` for the first). Written by `Session::append_messages`; old files
   have neither and load exactly as today (a nameless message's implicit
   parent is its predecessor in file order).
2. **A branch record.** `Record::Branch { from: <message id> }` says: what
   follows appends to the chain ending at `from`, not to the file's tail.
   The current tip is always "the chain the last record extended", so
   resuming lands on the branch you left off — and `Record::Branch` written
   again with an older `from` is also how you *switch back*: branch switching
   is itself an append, and the file stays append-only.

`Session::load` then selects the active chain by walking parent pointers
back from the tip. A `load_tree` variant returns all chains for `/tree`-style
navigation. Everything stays one JSONL file; `git log`-style history falls
out of the record order.

**Compatibility**: old readers skip unknown record types with a warning
(session.rs:248), so a branched file read by an old binary silently
*linearises* all branches — wrong, not crashed. Acceptable for a
single-user tool mid-development, but the session format should grow a
`version` field in `SessionMeta` at the same time, so a reader that is too
old can say so instead of misreading.

## What it must not break — the real cost of the feature

These are the reasons this is a design doc and not commit 14:

- **`taint_timeline`** (session.rs:265) positions taint checkpoints against
  *message indices in file order*, and provenance classification
  (`learning.rs`) fail-closes on unknown positions. Branching makes "index in
  file order" meaningless — the timeline must become per-chain, and the
  fail-closed rule must survive the rewrite: a reflection mined from a branch
  whose taint coverage is ambiguous classifies `untrusted`. This is the one
  place a mistake is a security regression, not a bug.
- **Taint is grow-only per conversation, and a fork is a *new* growth path.**
  A branch from before a hostile fetch must start with the taint *as of the
  fork point*, not the file's merged taint — otherwise forking never sheds
  taint and half the value is gone. That means taint checkpoints need chain
  positions too, and `load` must merge only the checkpoints on the active
  chain. Fail-closed fallback: a checkpoint that cannot be positioned taints
  the whole chain.
- **Replay** (`replay_run.rs`) feeds recorded user turns in order. It must
  learn to replay *a chain*, and `--on-divergence` semantics stay per-chain.
  Cheap, but it is a consumer that would otherwise silently replay a
  linearised mixture of branches.
- **The learning miners** read sessions as linear transcripts. Steers and
  denials on abandoned branches are still real interventions (arguably the
  *best* ones — the user cared enough to rewind), so mining should walk every
  chain, with lineage recording which chain a reflection came from.
- **`RunConfig` records** apply "from here on" — chain-scoped like everything
  else once branches exist.

## UX sketch

- **`Esc Esc` on an empty input** walks back through *user* messages, most
  recent first, filling the input with each; editing and submitting forks at
  that point. A third press walks further. Esc with text typed still clears
  back to the walk, and any other key cancels the walk. (codex's gesture,
  unchanged, because it needs no new chrome.)
- **`/fork`** forks at the current tip — "keep this conversation, but let me
  try something else from here" — and `/tree` lists chains with their first
  diverging user message as the label, picker-style, Enter to switch.
- **A `⑂` marker** in the transcript at the fork point, and a dim
  `branch 2/3` badge in the status line *only when not on the trunk* — the
  plan-badge rule again: a badge that is always there stops being read.
- **The fuel gauge already works**: a fork's prompt is the chain's prefix, so
  the context percentage drops back to the fork point's cost on switch —
  visible confirmation that rewinding bought context back.

## What deliberately stays out of v1

- **Cross-branch diff view** — `replay.rs` has the diffing; surfacing it in
  the TUI is real UI work and the CLI (`mecha replay --compare-chains`) can
  have it first for free.
- **Branch pruning/GC** — append-only files never shrink; a `mecha sessions
  compact` that drops abandoned chains is a separate decision because it
  destroys the learning evidence.
- **Branching mid-run** — forks happen between runs, at an assistant
  boundary, same rule as compaction's legal cut. A fork from the middle of a
  tool batch has the same 400-shaped failure modes compaction solved, and
  nothing about the UX needs it.

## Order of work, when picked up

1. Message ids + `SessionMeta.version` + chain-aware `load` (pure, heavily
   unit-tested — this is the compaction-cut kind of code).
2. `taint_timeline` and provenance per chain, fail-closed, with tests that
   prove an ambiguous branch classifies `untrusted`.
3. `Record::Branch` + `/fork` + `Esc Esc` in the TUI.
4. Miners and replay walk chains.
5. `/tree`, the `⑂` marker, the status badge.
