# Where should the learning store live? — research

**Date:** 2026-09-29. **Question (owner):** now that the learning store no
longer uses git, what is the best storage for it — should it move into
mecha-graph, into a database of its own, or stay as it is? "We've made it
far without a db so far — not sure why, but perhaps because we already have
a better solution."

**Answer:** the owner's instinct holds. **Keep the store as files, and fix
the four latent defects in them** (§5) — three small PRs. Move to a database
only when mecha core needs to query *across* the learning store and the
graph's memory, which happens when the graph converges into mecha; the
destination then is a **mecha-owned SQLite database**, never `graph.db`
(§4). Git was the strange part, and it is gone (#379, 2026-09-28).

The owner's two earlier asks, read precisely: the 2026-09-28 ruling was
against **git**, not against files; "better integrate this with our
mecha-graph memory system" is served by the convergence path in §6, not by
moving the learning store into the graph now.

## 1. What the store is

`~/.mecha/learning/`, 1.9 MB on 2026-09-29: 86 reflections (≈11 a week,
weeks 32–39), one `behavior.learned.toml` with 7 rules, 10 proposals, 378
validation rows (≈75 a week) and 144 attempts, the mined and distilled
ledgers (≈630 each), `runs.jsonl`, `passes.jsonl`, `harness/`, `logs/`.
Written by the `session_end` hook (`learn-live.sh`: reflect, learn;
`distill`), the nightly `ruminate.sh`, owner verbs from the CLI, TUI and web
(as child processes), `forget` (in-process, from the web), and — unlocked —
`mecha mail reflect` and `mecha validate`. Read, without a lock, at every
run start (the rules block and `goal_lessons`).

Files were chosen on purpose on 2026-08-04 (`1f3cafbe`): "Files, not a
database, with the possibility of one noted… The swap to a database happens
behind LearningStore's API if the CIPHER retrieval tier ever demands it."
The module doc records the owner's requirement that the store "can be
inspected and edited"; `MEMORY-RESEARCH.md` adds the security reason —
"everything that rides in the prompt stays a file the user can open, diff,
and blame", after ChatGPT's hidden dossier.

## 2. What files have cost

Searched `HISTORY.md`, `HANDOFF.md`, the git log and the reviews of #14,
#89, #94, #122, #184, #379, #381 and #382. **No recorded lost write, torn
file, corrupted TOML, destructive rewrite, or run that failed to start
because of the learning store in eight weeks.** What was recorded: the TUI
blocking on the store lock that `reflect`/`learn` hold across a model call
(#89 — a database would keep the same lock, so no help); a lossy
whole-store rewrite caught before merge (#184); the same unlocked-append
pattern caught in review in the sessions store (#382).

Measured cost: `mecha rules list --json` (rules plus the whole validation
ledger, with tallies) takes 0.01 s; parsing all three ledgers takes 3 ms.
Whole-file reading would need tens of megabytes of reflections — decades at
today's rate — before a run start noticed.

## 3. What others do

Summarised from the external survey (sources in the PR description):

- **Products that inject memory into the prompt split it by author, not
  type.** Claude Code: CLAUDE.md is "you write / rules", auto memory is
  "Claude writes / learnings", both plain markdown. Windsurf: rules vs
  private auto-memories. Cursor removed Memories (2.1, late 2025) and told
  users to turn them into rule files. Nous Hermes: capped files, frozen at
  session start "to preserve the prefix cache", scanned before injection.
- **Research systems keep rules and episodes in separate pools** (ExpeL's
  insight pool vs trajectories; LEAP, AutoGuide, Agent Workflow Memory).
- **The 2025–26 trend is files as the interface** (Manus, Anthropic's
  memory tool, Letta's "Is a filesystem all you need?" and its git-backed
  MemFS, Cursor). Databases win on integrity — several writing processes and
  invariants across records — which is SQLite's own threshold: concurrency
  logic for "a pile of files" is "a notorious bug-magnet"
  (sqlite.org/appfileformat.html, verified).
- **Nobody puts prompt rules in the store that holds third-party content.**
  The security evidence says why: a four-stage content screen rejected 0 of
  360 poisoned memories, and provenance-*weighted* retrieval was
  indistinguishable from no defense (p=0.80) — only excluding untrusted
  sources worked ("Utility Under Attack", arXiv 2608.21230, 2026-08-21,
  verified). That is mecha's structural provenance gate, and an argument
  for keeping prompt text away from the graph.
- **Embedded engines in Rust:** SQLite is the only one that is multi-process,
  transactional, mature and readable with standard tools. redb's
  multi-process mode is experimental since 4.3.0 (2026-09-14, verified);
  fjall and sled are single-process; LMDB is not human-readable; DuckDB
  allows one writing process.

## 4. The three options

| | Files, fixed | Mecha-owned SQLite | Tables in `graph.db` |
|---|---|---|---|
| Trust | Best: only mecha code touches it | Same | Weaker: every graph process (including the MCP servers Claude Code and Hermes run) holds a writable handle on prompt text; a mecha security rule enforced in another repo; the first graph→prompt path |
| Forget | `forget.rs` today | Needs `secure_delete` + `VACUUM` | Needs redact extended to the tables, and a policy for the 1.56 GB of hand-made `.bak` copies it cannot reach |
| Run start | Absent → no rules; bad TOML → run fails (fixable, §5) | Same | Tied to the graph's key, a 310 MB file and binary version skew; a cache would be a second store |
| Owner inspect/edit | `cat`, `jq`, `$EDITOR`, any binary version | Needs verbs + export | Needs verbs + export, and SQLCipher blocks the `sqlite3` CLI |
| Experiments | Byte-stable digest for free | Needs a deterministic export | Needs seeding per trial |
| Transactions | By discipline (§5) | By construction | By construction |
| New dependencies | None | SQLite (bundled) | None via the CLI; SQLCipher/OpenSSL/sqlite-vec if linked |
| Size of the work | ~3 small PRs | Backend swap, migration gated on `rules_hash`, verbs, export | The swap plus graph migrations, verbs and redact work in a second repo |
| Convergence (R14) | Neutral | The seed of the merged store | Against `APPRAISAL-WIRING-DESIGN` §5: "no new cross-repo reader is built as the long-term shape" — this adds one on every run start, and puts the harness's most privileged state inside the tool being absorbed |

Using the graph's *own* concepts (reflections as episodes, rules as facts,
proposals as candidates) fails outright: `kg_upsert` takes a caller-chosen
`source`, so any MCP client could write a "learning" record; rules would
appear in `kg_search` packs served to other agents; and the graph's precheck
lane could auto-accept a rule once its class climbed the ladder — a second
acceptance authority beside `learn`'s measured gate.

## 5. The defects, and their file-native fixes

Each was confirmed against `c6ae2c69`; none has caused a recorded loss.

1. **Two writers skip the lock.** `mecha mail reflect` appends reflections
   and correction marks; `mecha validate` appends validations, attempts and
   passes. A rewrite under the lock can drop an append that lands between
   its read and its rename — for `mail reflect`, silently and permanently,
   because the mined mark survives. *Fix:* take the lock in both (a few
   lines each); optionally make every mutating `LearningStore` method take
   `&StoreLock` so an unlocked writer does not compile.
2. **An append is two `write` calls.** `append_line` uses `writeln!`, which
   writes the line and the newline separately, so two appenders can merge
   into one corrupt line. *Fix:* one `write_all` of the line with its
   newline.
3. **Rewrites drop what they cannot parse.** `rewrite_reflexions`,
   `write_learned_rules` and `write_proposal` round-trip through typed
   structs with no catch-all, and `rewrite_reflexions` rebuilds from a
   reader that skips unparseable lines — so a row a newer binary wrote, or
   any line this one cannot read, is deleted on the next mark or edit, and
   none of these rewrites fsync. *Fix:* the patterns already in the tree —
   a `#[serde(flatten)] rest` field (as `frontdoor.rs`), pass-through of
   unchanged and unparseable lines (as `forget.rs`), and `write_replacing`'s
   fsync.
4. **Multi-step writes are not atomic.** `learn` writes rules → marks
   reflections → appends a run; `proposals accept` adds the status last. A
   crash between steps re-argues a batch or strands a proposal. *Fix:*
   make both resumable — `accept` finishes a proposal whose rules are
   already live; `learn` writes an intent line first and completes it at
   the next pass (every step is already idempotent).

And one policy question the owner decides (D1 below): a hand-edited rules
file that does not parse currently fails every run start.

## 6. When to move, and where

Move when **a mecha-core feature needs to query across the learning store
and the graph's memory** — a reflection joined to the episode its session
distilled into, a rule's evidence joined to the facts it touched — or when
run start needs indexed retrieval over many rules (the CIPHER tier the
2026-08-04 doc named). Both arrive with the graph's convergence into mecha,
which is unscheduled.

Then: a mecha-owned SQLite database, opened by mecha core, that the graph's
tables move into — WAL, `BEGIN IMMEDIATE`, `secure_delete`, an explicit rule
`ordinal` so `rules_hash` is unchanged, a deterministic export for the
experiment digest and for "ask the artifact", and the graph's proven
conventions ported rather than called (`uid`, bi-temporal
`valid_from/valid_to/ingested_at/invalidated_at`, an `event_log`, redact's
"enumerate every table" discipline, which `forget.rs` already mirrors). A
binary links one SQLite, so if the graph's SQLCipher build comes into mecha
the learning store shares it, and encryption is decided then.

The owner-authored `*.user.toml` stays a file under every option: one
author, one writer, and nothing gained by a database.

## 7. Decisions for the owner

**Ruled 2026-09-29:** the plan in §8 is adopted; D1 as recommended; D3
adopted. **D2 stays open** — see the caveat under it.

- **D1 — A hand-edited rules file that does not parse.** Keep failing every
  run start (a user rule silently not obeyed is a silently-degrading guard),
  or skip the file with a warning and a doctor finding? Recommendation: skip
  a bad `learned.toml` (machine-written; missing it is the safe direction),
  keep failing on a bad `user.toml` (the owner's own rules must not vanish
  silently), and add `mecha rules edit --user`, which parses before it saves.
- **D2 — Rules the learner drops in a rewrite.** 18 of the 25 rule ids the
  validation ledger charges no longer exist in `learned.toml`: consolidation
  drops rules it omits, and their text survives only in proposal snapshots.
  Retire them with a reason instead ("superseded in consolidation"), so the
  rule's history lives on the record? This is a learning-policy question,
  not a storage one. **Caveat before anyone builds it:** the learner is
  shown retired rules as "IMMUTABLE, measured harmful — never restate or
  re-derive", and the carry-forward inherits retirement onto any reworded
  restatement; retiring a rule that was merely dropped would teach the
  learner that a lesson it may still need is harmful. Keeping history this
  way needs a distinct mark ("superseded", shown to nobody as harmful), not
  retirement.
  **Recommendation (2026-09-29, awaiting the owner):** no change to storage
  or to consolidation. The history is already on disk — every
  consolidation passes through a proposal whose `rules_before`/`rules`
  snapshot the text, and `runs.jsonl` counts it — and nothing in the code
  asks for it. A superseded mark in `learned.toml` would grow the file that
  feeds every prompt and make every reader filter it, to answer a question
  only an audit asks. The cheap version answers the audit: `mecha rules show
  <id>` resolves a vanished id from the proposal snapshots ("dropped in
  consolidation on DATE by proposal P; last text …"), read-only, touching
  nothing that reaches a prompt.
- **D3 — The move trigger.** Adopt §6's trigger (cross-store queries or
  indexed retrieval at run start), so the database decision is made when it
  buys something, not before?

## 8. Plan

1. **Locks and single-write appends** (defects 1–2). Ship regardless.
2. **Lossless rewrites** (defect 3).
3. **Resumable multi-step writes and the parse-failure policy** (defect 4,
   D1).
4. Correct the stale descriptions: the `learning.rs` module-doc layout, the
   user guide's file table, `Proposal::status`'s vocabulary, and the git
   mentions left in `SettingsLearning.svelte` and `MEMORY-RESEARCH.md`.

Each is its own PR through the review loop. Nothing here moves or rewrites
the owner's data.
