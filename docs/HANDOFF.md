# mecha — handoff

Where the project stands and what is actually left to do. Written to be picked
up cold.

Two companion documents, so this one can hold open work and nothing else:

- [`CLAUDE.md`](../CLAUDE.md) — the cross-cutting invariants — and
  [`ARCHITECTURE.md`](ARCHITECTURE.md) — why each subsystem is shaped the way
  it is. This file deliberately does not restate either.
- [`HISTORY.md`](HISTORY.md) — what was built and when, and the traps hit along
  the way. Everything here that turned into "done" moved there.

**Keeping this file honest is a procedure, not a habit.** Run the `handoff`
skill (`.claude/skills/handoff/`) at the end of a session that changed
behaviour: it walks every open item below, verifies against source whether it
shipped, moves the ones that did into `HISTORY.md`, and re-checks the counts
and machine facts that rot silently. [`README.md`](README.md) in this directory
maps which document holds what.

---

## Where the work is

Public at **github.com/ljchang/mecha**, MIT licensed, released as **v0.1.16**
(2026-08-29 — the appraisal system survives its own review: PRs #111/#112,
failed-turn transcript integrity, positional configs for the probe,
unreadable-store accounting, and the owner-closure guard; v0.1.15 shipped
2026-08-26 — five surfaces that described themselves wrongly, found by
using them; 0.1.14 shipped 2026-08-25 — voice calls and chat became one
conversation, three review surfaces stopped hiding what they were asking
people to approve, and the nightly mail classifier took both mailboxes;
0.1.13 shipped 2026-08-24 night with the web surface, voice, and the graph
queue's similarity groups; 0.1.12 on 2026-08-22, 0.1.11 and 0.1.10 both on
2026-08-21, 0.1.9 on 2026-08-20, and 0.1.7/0.1.8 on 2026-08-19/20 after the
mail hold lifted).
**Released as v0.1.17 on 2026-08-31**, carrying the ten merges beyond
v0.1.16 that this paragraph used to list as untagged:
the 2026-08-30 five — #125 (**four home-page queue cards did nothing**,
plus the ExecStart-check rewrite and the `js_string_array` guard
loosening); #126 (**the graph tab grows its notebook, composer, and the
whole entity-curation surface** — alias add/remove, merge-with-audit-trail,
create-on-miss, identifiers, plus the three-store `/api/proposals` pane;
deployed the same night, so the installed binary is `main` at `ab0097b`);
#124 (**the retirement drill ran the NoGo path
whole, and fixed the probation leash it proved unreachable**); #120, the
notes and graph tabs become one graph tab; #123, the docs deploy stops
being evictable by a PR build — then #122 (2026-08-30, fifteen commits —
**the learning loop runs itself and the instruments that grade it were
fixed**: `learn --auto` with probation, nightly direct retirement,
branched counterfactual probes, the surface rebuild from recorded specs);
and the 2026-08-29 four — #114, the shadow queue on every owner surface
(plus a web entity page and chat tool-result previews); #116, the
`/tasks` page repair; #115, the appraisal docs-page rework; #117, the
docs site's fixture-backed web demo and its two CI gates. All in the
`CHANGELOG.md`. The tag was cut with the
release workflow green, and all four crates confirmed live on crates.io
at 0.1.17 (`mecha-core`, `mecha-cli`, `mecha-mail`, `mecha-slack`) by
querying the registry rather than by watching the job go green. Seven of
the merges in it had landed with no changelog entry (#119, #121, #123,
#125, #127, #128, #130) and were written up at release time.
**`main` now carries nine merges beyond v0.1.17 that are not yet tagged**,
all 2026-09-02/03: the audit lane's #139 and #142 and its approval-rules
pair #143 and #148 (each has a `CHANGELOG.md` entry under Unreleased as of
this pass); the appraisal lane's #140, #141 and #147 (**no changelog
entry yet** — the same trap as last time, to be written at release or by
that lane); and the two docs merges #146 and #149. The next tag is
v0.1.18, and the installed binary is older than all nine.

**Three lanes were off `main` on the evening of 2026-09-02: the audit
lane, whose approval-rules PR landed the next morning UTC and whose
follow-up is still live; the appraisal lane, landed that day as two PRs in
the agreed order with its two small follow-ups landed that same night; and
a third session's pair
(#144, #145), open and not either lane's to sequence.** `fix/harness-review`
(**PR #139**, the audit lane, merged at `089952f`) carried the six-lane
review's nine fixes — the jail's dangling-symlink follow, the subagent
send-laundering, the cumulative usage frame, `Approver::escalate`,
`last_assistant_text`, `&None` events on in-run side calls, lenient
`stop_cause` reads — and `docs/AUDIT-RESEARCH.md`; its follow-ups landed as
#142 at `9a5ca23`; that lane's `feat/approval-policy` (**PR #143**:
`policy.rs`, `Approver::consult`/`permit`, `[[rule]]`/`[approval]` config)
**merged at `1d21d6b` on 2026-09-03** after nine review passes on the
rebased tree, every one of which found something real and smaller than the
last — the arc is in `HISTORY.md` under 2026-09-02/03. Its follow-up,
**#148** (`feat/approval-policy-followups`), **merged at `c88adf0`** later
the same day after eleven passes of its own: the owner's two rulings of
2026-09-03 — an opaque command matching no rule falls through to the
approver rather than cliffing every glob in every trigger into `Blocked`;
an `allow`/`prompt` on a live-routed tool is refused at `setup`, `forbid`
kept and noted as unreached — plus what the loop found in implementing
them: the fall-through had quietly been doing the inline-eval floor's job
for opaque commands, a redirect target glued to a separator swallowed the
next command, `2>&1` had to be stripped before the separator split, a
here-string's payload is not a file name, `$IFS` and `${…}` glued to
pattern words, the routed-tool check was false under `--no-outbox` and so
moved from `validate` to `setup::live_rules` (a pure function with a table
test), and a project file could launder or un-route the operator's own
rules. `docs/ARCHITECTURE.md` §Approval rules on `main` now states the
rulings and the residue. Nothing was reinstalled or restarted for any of
these: the installed binary predates them.
The third session's **#144** (`fix/draft-shows-its-account`: `OutboxItem`
gains `call_id` and `filled_defaults`, `OutboxStore::stage` takes an
`outbox::Provenance`) merged `main` after both appraisal PRs and
auto-merged the seven code files it shares with them — `appraisal.rs`,
`doctor.rs`, `tool/mod.rs`, `mecha-cli`'s `commands/mail.rs` and
`slack/connector.rs` (all #140), and `outbox.rs` and `frontdoor.rs` (both
#141), plus `docs/ARCHITECTURE.md` — so its only textual conflict was
#142's in `agent.rs` and its remaining risk is the semantic one an empty
conflict list does not cover: `outbox.rs` is the one to re-check, since
#144 changes the staging store's shape while #141 added readers of it
(`Channel::Commitment`, `Depth::given_up`); **#145**
(`fix/voice-echo-on-speakers`) touches
`scripts/voice/`, `VOICE-RESEARCH.md` and one CI job, and nothing any
other lane touches. **The appraisal lane is merged**:
#140 (`feat/appraisal-record`, phase A of `docs/APPRAISAL-RESEARCH.md` §3
and the prediction record) at `15c628d` and #141 (`feat/appraisal-phase-b`,
phase B) at `49166e3`, both on 2026-09-02 after #139 and #142 — see the
goal-system section below for what each holds. Its two follow-ups have
landed: **#147** (`fix/backlog-read-creates-nothing`: `backlog.rs`,
`harness.rs`, `work.rs`'s test guard, `mecha-cli`'s `testenv.rs`), out of
#141's post-merge review pass, at `ac51c93`, and **#146**, the docs change,
at `4cbb196`. Still open from that lane, in order: the
`StopCause::Interrupted` split (Parked vs Cancelled; the `ask_user` park
in `questions.rs` is the one park site, stopping through
`ToolCtx::cancel`) and cancel-then-re-prompt as a steer, both now
unblocked by #139's lenient `stop_cause` read; phase B's three trajectory
counters and the trigger read receipt; the audit lane's planner ask and
critic call, which base on the merged record; charter sensors, designed
at `GOAL-SYSTEM-DESIGN.md` §11.1 and unbuilt. The untracked
`docs/APPRAISAL-RESEARCH.md` the main checkout held from before the
branch existed was a strict subset of the tracked one and was deleted on
2026-09-02.

**0.1.14 is thirty-one commits from three sessions working the same day**,
which is the thing to know about reading its history: the lanes interleave,
so `--ancestry-path` answers "what is in this release" and a date range does
not (see the Measuring trap that count produced).

**0.1.13 is a patch bump carrying two whole subsystems**, which is worth
knowing rather than discovering: 91 commits, including everything the voice
stack and `mecha serve` are made of. That follows this project's own
practice for 0.x — every release since 0.1.1 has shipped features under a
patch bump — but if the version line is ever meant to signal size, this is
the release that argues for it. **Four** crates are on crates.io — `mecha-core`, `mecha-mail`,
`mecha-slack`, `mecha-cli` (the bare name `mecha` was taken, so the CLI
crate installs the `mecha` binary) — published through the tag-driven
`release` workflow with Trusted Publishing, so no registry token exists
anywhere. From v0.1.1 on, a tag and a crate version are the same number.

The knowledge graph is now a **sibling public project**:
**github.com/ljchang/mecha-graph** (fresh history — the private repo's
journal is the data the project exists to hold, so the public tree starts
where every fixture wears a synthetic world), with `mecha-graph`,
`mecha-graph-core` and `mecha-graph-mcp` on crates.io at 0.1.0, hand-published
2026-08-16 (no release workflow there yet; mecha's is the template). The CLI
crate took the bare name deliberately — `cargo install mecha-graph` is the
front door, and mecha-cli's own name was necessity, not preference. mecha
wires it as `[[mcp]] name = "graph"` with `prefix_tools = false`, so the
model calls bare `kg_*` names; the store lives at `~/.mecha-graph/` under
`MECHA_GRAPH_*` env vars.

`mecha-slack` joined at this release, and the reason is worth keeping because
it generalises: **cargo refuses to publish a crate whose non-dev dependencies
are not on the registry**, and `mecha-cli` depends on it. Tagging without
adding it to the workflow's list would have published `mecha-core` and
`mecha-mail`, then failed on `mecha-cli` — a half-published version that can
be yanked but never unpublished. A new workspace member that anything
published depends on belongs in that list in the same change that introduces
it. Its first publish was by hand, because Trusted Publishing can only be
configured on a crate that already exists; TP was added for it immediately
after.

How work lands — branch per arc, one worktree per session, PR-gated, release
by tag — is `CONTRIBUTING.md`'s to state.
CI runs build, test, clippy and rustfmt on every push and pull request; the
Claude review bot runs beside them, and the `@claude` mention bot answers on
issues and pull requests. **Both were repaired on 2026-08-10 night and had
never worked before it** — the review job had failed on every run it ever
made (a credential, on both sides of the OAuth migration that was itself an
attempt to fix it) and the mention job's condition could not be true. Both now
pin `--model claude-opus-5`; without a pin the action runs `claude-sonnet-5`.
Two things about them that will otherwise cost somebody an afternoon:
`claude-code-action` **skips and exits 0** when the workflow file differs from
the copy on `main`, so a workflow change can never be tested by its own pull
request and a green check there means nothing; and the same workflows are now
in mecha-factory but **inert until the Claude GitHub App is installed on that
repository** — the token is not enough, and the job fails with
`Claude Code is not installed on this repository`.
The documentation site builds from `website/` and deploys to
<https://docs.mecha-factory.ai/> — GitHub Pages still hosts it; the custom
domain is asserted by `website/static/CNAME`, which ships inside the deployed
artifact because `actions/deploy-pages` writes no such file itself.
**Since 2026-08-29 (#117) the docs build also builds `web/` and embeds it as
a fixture-backed demo**, so the docs workflow triggers on `web/**` too, and
two gates ride every docs build: `check-demo` (every `/api` endpoint the app
reaches must have a fixture — the boundary list is deliberately enumerated,
never a catch-all, so a new endpoint fails rather than being silently
swallowed) and `render-check` (every page loads in headless chromium; an
uncaught error, a console error, or a page that *drew almost nothing* fails
the build — the last because a wrong fixture shape does not throw, it draws
an empty pane). **The standing obligation this creates: add a page or an
endpoint to `web/`, add its fixture in the same PR** — `web/src/demo/
fixtures.js` is the how-to, `website/README.md` the reasoning; new *views*
are covered free, since the route list is parsed out of `App.svelte`.

The site has a **Factory** section as of 2026-08-08, opening with a live
component gallery at `/docs/factory/gallery`. Its frames are the real pages
generated in mecha-factory (`cargo run --example gallery`), committed there at
`gallery/` and copied in at build time by `website/scripts/sync-gallery.mjs` —
sibling checkout first, public tarball otherwise, warning rather than failing
so the prose still builds offline. The copy is gitignored here; that repo's CI
owns the drift check, because that is where the renderer lives. `overview.md`
landed 2026-08-10; five more pages are planned in
[`FACTORY-DOCS-DESIGN.md`](FACTORY-DOCS-DESIGN.md).

Every commit was verified to build and pass tests **in isolation**, so the
history bisects rather than merely ending in a good state.

First thing to run in a fresh context:

```bash
cargo test --workspace && cargo clippy --all-targets --all-features
```

**Not re-measured at session close (2026-09-03, `main` at `b50eb24`).**
When the close pass began (~17:40 UTC) a peer's 169-call appraisal run held
llama-server and this box's rule is no build during inference; when the run
released it (17:56) the deploy's `cargo install` ran (18:04) but `cargo
test --workspace` did not — the owner was closing the session, and the
verifiable fact without the run was CI's green test jobs on `b50eb24`. So
the count below stands as measured at `49166e3` — eleven merges ago
(`git log --first-parent --merges --reverse 49166e3..b50eb24`: #146, #147,
#143, #149, #148, #150, #151, #145, #144, #152, #155), of which #143's
`policy` suite, #144, #148, #150 and #155 added cargo tests — and CI's
test jobs were green on `b50eb24`, which is the fact that was verifiable
without a build. Four of the five open PRs add tests too (#158, #153,
#157, #154; #156 is docs); mecha-26 reports `mecha-cli` alone goes 707 →
715 across its three.

On **`main` at `49166e3`** (#139, #140, #141 and #142 all in), measured
2026-09-02 (~20:30 UTC): **2,160 tests**, no failures — **691** in
`mecha-cli` (1 ignored), **20** in `first_run`, **1,224** in `mecha-core`
(1 ignored: the `kind_override_probe` child half of an environment test),
6 `mcp_server`, 9 `sandbox_backends`, 133 `mecha-mail`, 1 in its
`mecha-mail` binary, 75 `mecha-slack`, 1 doctest; clippy clean. A delta
against the 2,029 below is four merges' worth and is not itemised here;
diff `cargo test -- --list` between the two commits if a count matters.

Expect **2,029 tests**, no failures — measured 2026-08-31 (~12:50 UTC) in
this checkout on `main` at **b132157** (the 0.1.17 release commit): **682** in
`mecha-cli` with 1 ignored, **1,102** in `mecha-core`, the rest unchanged.

The **+3** over 2,026 at `786d8ec` is #130, and it is named rather than
subtracted, because a delta is a commit delta and not an arithmetic one:
`a_group_verdict_says_what_it_does_not_know`,
`the_verdict_line_survives_an_eighty_column_terminal` and
`a_verdict_that_landed_on_nothing_keeps_its_group`, all in `tui::tests`.
Moving `why_nothing_landed` from `serve::review` to `commands::review` in the
same PR moved no tests — they stayed where they were and follow the new path.
The **38** added over 1,988 at `ab0097b` split at the merge level: **+11**
from #128 (the review queue's regroup-cost arc) and **+27** from #127 (the
diagnostician's sight, the workspace split, and the two gate guards —
`guard_regressions` and `MIN_INFORMATIVE_HOLDOUT`; see below).

Both halves were measured rather than subtracted from a single total:
`cargo test --workspace` at `4cfd57d` gives 1,999 and at `786d8ec` gives
2,026. Worth doing that way — summing the per-suite lines by eye gave 2,006
here, because a `grep -v "0 passed"` filter also swallows a suite reporting
**20** passed, and `$6` in `test result: ok. N passed; N failed; N ignored`
is the *failed* column rather than the ignored one. Two off-by-a-column
errors in one measurement of a number that then gets written down.

The previous figure was **1,988**, measured 2026-08-30 (late, ~23:45
UTC) on `main` at **ab0097b** (the #126 merge): **662**
in `mecha-cli` with 1 ignored, **1,081** in `mecha-core`, the rest
unchanged. The **7** added over 1,981 at `6987bc5` split at the merge
level: **+1** from #125
(`every_queue_the_backlog_reports_is_named_and_reachable_from_the_web_home`,
`commands/review.rs`) and **+6** from #126 (the proposals pane's
store-coverage guards and the review-wave regression tests — the
`decide_argv` flag-ordering pair among them, added after a green suite
sat over a 100%-broken reject; see Traps → Review process).

The previous figure was **1,981**, measured 2026-08-30 in this checkout
on `main` at **6987bc5** (the #124 merge, which also folds in #120 and
#123): **655** in `mecha-cli` with 1 ignored, **1,081** in `mecha-core`,
the rest unchanged. The **4** added over 1,977 at `4c7a0e2` are fully
attributed: `mecha-cli` +2 (#124's probationary-conviction-at-2 test in
`commands/rules.rs`, and #120's `the_graph_routes_sit_behind_the_owner_guard`
in `commands/serve`) and `mecha-core` +2 (#124's probation release tests in
`learning.rs`). A caution from chasing that split: two lanes measuring
trees at *different commits* both reached for a provisioning explanation
before a `--list` diff named the one test and the commit it rode in on —
diff the test lists before theorising about the environment.

The previous figure was **1,980**, measured 2026-08-30 on
`feat/retirement-drill` (the retirement-drill arc, three commits on
`4c7a0e2`, before the merge picked up #120's test).

The previous figure was **1,977**, measured 2026-08-30 on `main` at
**4c7a0e2** (the learning-loop-autonomy merge, PR #122: **653** in
`mecha-cli` with 1 ignored, **1,079** in `mecha-core`, the rest unchanged
from the previous count). The earlier baseline was 1,930 on **12d7f4b**,
the night's four merges (#114/#115/#116/#117) on top of the v0.1.16
release. Breakdown: **630** in `mecha-cli` with 1 ignored plus a
new **20**-test `first_run` integration suite (the v0.1.16 onboarding
arc's, #113 — the previous count predates it), **1,055** in `mecha-core`,
6 + 9 in its two integration suites, **133** in `mecha-mail` plus 1 in a
mail binary, **75** in `mecha-slack`, and 1 doctest. The **99** added over
the previous figure span two arcs no pass counted separately — the
v0.1.16 review-and-onboarding arc (#106–#113) and tonight's #114/#116
(the Rust delta `bddd835..12d7f4b` touches exactly `commands/review.rs`,
`serve/{board,chat,mod,review}.rs` and `tui/{find,queues}.rs`) — and the
split between the two arcs was not measured, so it is deliberately not
stated.

The previous figure was **1,831**, measured 2026-08-28 on `main` at
**b26571f**, rung 7's closing piece (#102, the model half of step
appraisal) merged on top of rung 8 (#99, #103). Breakdown: **582** in
`mecha-cli` with 1 ignored, **1,024** in `mecha-core`, 6 + 9 in its two
integration suites, **133** in `mecha-mail` plus 1 in a mail binary, **75**
in `mecha-slack`, and 1 doctest. The **43** added over the previous figure
(1,788 at `d32b288`) split `mecha-cli` +2 and `mecha-core` +41, consistent
with #102's `agent.rs`/`step.rs`/`tool/todo.rs` — this entry's own prose
already describes step escalation as shipped (the paragraph beginning
"Step escalation is different in kind," below), so the count it opens with
has to be the tree that includes it, not the tree this entry's own PRs
alone produced.

The previous figure was **1,788**, measured 2026-08-28 on `main` at
**d32b288**, which carries rung 8 (#99, #103) on top of rung 9 and rung
10 — the tree *before* rung 7's closing piece. Breakdown: **580** in
`mecha-cli` with 1 ignored, **983** in `mecha-core`, 6 + 9 in its two
integration suites, **133** in `mecha-mail` plus 1 in a mail binary, **75**
in `mecha-slack`, and 1 doctest. The **20** added over the figure before it
(1,768 at `63f88b3`) split `mecha-cli` +15 and `mecha-core` +5 — confirmed
by `git diff --stat 63f88b3..d32b288 -- '*.rs'` touching only the six files
#99 and #103 changed (`appraisal.rs`, `tasks.rs`, `tui/mod.rs`,
`voice/mod.rs`, `serve/board.rs`, and `serve/chat.rs` — the last carries
`WireEvent::Affect`'s own wire-shape test, part of the `mecha-cli` +15);
the one rung 9/10 handoff PR in between, #104, is docs-only and moved
nothing (#105 is not in between — it is `63f88b3` itself). Mail, Slack and
the integration fixtures untouched by either.

The figure before that was **1,768**, measured 2026-08-28 on `main` at
**63f88b3**, which carries both rung 9 and rung 10 (#100, `e124f8a`, plus
its own handoff pass #105) — the tree *before* rung 8. Say which tree,
since a count taken between the two would have read differently and did,
one paragraph below. Breakdown: **565** in `mecha-cli` with 1 ignored, **978**
in `mecha-core`, 6 + 9 in its two integration suites, **133** in
`mecha-mail` plus 1 in a mail binary, **75** in `mecha-slack`, and 1
doctest. The **71** added over the previous figure (1,697 at `a0638c8`)
split `mecha-cli` +7 and `mecha-core` +29 for rung 9 (#97/#98/#101 —
consistent with `appraisal::for_session`, `Distiller`'s new `Surprise`
extraction, and #101's four rounds of terminal-escaping and outbox-read
hardening) plus `mecha-core` +35 for rung 10's `charter.rs`/`guilt.rs`
(#100), with mail, Slack and the integration fixtures untouched by
either — 7 + 29 + 35 = 71.

The previous figure was **1,697**, measured 2026-08-27 at **a0638c8**, the
merge commit of PR #92, the last of the nine appraisal-arc PRs (#86, #87,
#93, #88, #89, #90, #91, #94, #92) to land that day. The **131** added over
the figure two merges back (1,566 at `85b244c`, before any of the nine
landed) split `mecha-core` +99 and `mecha-cli` +32, with every other suite
unchanged — consistent with an arc that was almost entirely appraisal
logic, learning-UI panes and the replay surface store, none of which touch
mail, Slack or the integration fixtures. An intermediate count of 1,690 at
`c630ff9` (after #86, #87, #93, #88, #89, #90 but before #91, #94, #92) was
measured and superseded within the same session; it is not carried forward
as a separate step in this chain because nothing was ever built or read
against it.

Clippy and rustfmt are clean at that commit, **measured locally**. CI is a
separate claim and a weaker one, and it must be made per **sha**. Every PR
in the context-pressure arc (#66, #67, #69, #70, #71, #72) was checked with
`gh api repos/ljchang/mecha/commits/<sha>/check-runs` against its own head
commit before merging, all six jobs green — `gh pr view`'s rollup follows
whatever head the PR currently has, which is correct and moves, where a
sha-addressed query pins the evidence to a commit that cannot slide. The
merge commits themselves and `be75b73` are *unverified*: CI runs on pushes to
`main`, but nobody has confirmed the result on this sha.

That distinction is not pedantry; it cost an hour and a wrong sentence in
this file. Read Traps → Environment before writing "CI passed" anywhere:
three separate mechanisms were producing green-looking evidence for
untested commits on 2026-08-26.

**Measure the merge, not either side of it, and attribute with a commit
range** — the lesson of this particular count, and the reason those numbers
are `git diff v0.1.13..HEAD` per suite rather than a `git log` sweep. Three
sessions landed work the same afternoon; two wrote down a figure measured on
their own tip (1,365 and 1,367), neither describing the tree anyone would
check out, and the attribution sweep then ran inside a worktree cut 31
minutes before `5b187c5` existed, so three `mecha-core` tests had no lane.
Two wrong mechanisms were proposed for that before one was measured — they
are recorded in HISTORY under Traps → Measuring, along with the genuine
`--since` hazard that turned up while chasing it. Note also that the earlier
line said "2 doctests" where there is one plus a single test in the
`mecha-mail` binary; the total was right and the breakdown was not.

The previous figure was **1,343**, re-measured at the v0.1.13 tag
(068a659); the same number was measured earlier at f6a39a5, the
night of 2026-08-24, after the voice-controls and web-surface-arcs merges. The 20 over the count before it are
`mecha-cli` +13 and `mecha-mail` +7 — the phone surface's Arcs A/B/C and the
dictation route, plus the `mecha-mail` tests that the 08-24-evening count had
set aside as "an uncommitted docs WIP that is not this arc's to count" and
which have since landed. The previous count was 1,323, measured the evening
of 2026-08-24 in a clean worktree after the web-review-surfaces arc (680 /
429+1 / 122 / 75 / 15 / 2), and 1,281 before that, measured the evening of
2026-08-23
after the groups/cascade, clean-evidence-learner and graph-surfaces arcs (679
in the `mecha-core` lib suite, 388 in `mecha-cli` with 1 ignored, 122 in
`mecha-mail`, 75 in `mecha-slack`, 15 across the two integration suites that
need real backends, and 1 doctest). The 12 added over the morning's 1,269:
the clean-evidence reflector (4 in core), the cascade/Fan refusals and tally
(3), the groups envelope/parser and /find modal (3), and the Slack `note`
and `queues` command words (2). The morning's 14 over 2026-08-22's 1,255
were `mecha review`'s tally and refusals, the `/queues` modal's parsing,
tier filter, item detail and tiny-size draws, and `exe::self_exe`. The 9 added over 2026-08-21's 1,246 are the harness store,
override layer, and doctor's harness/starved-learner checks; the 24 before
that were the search-chain, cache-lens and outbox-approval arcs. Earlier 2026-08-21 counts were 1,222,
1,213, 1,210 and 1,192; the 2026-08-20 counts were 1,140 at `cfa2cc2` and 1,105 at 0.1.9,
the 2026-08-19 count was 989 and the 2026-08-18 one 936; the growth from 707 (2026-08-10) spans the 0.1.3–0.1.9 arcs, and each
release's CHANGELOG entry names what its tests pin. **A flake has now been seen twice and is still unidentified.** Once in
`mecha-core` on 2026-08-08, and again on 2026-08-19 (`cargo test --workspace`
reported `506 passed; 1 failed` in the `mecha-core` lib suite, then four
consecutive clean runs of the same suite and the whole workspace). The failing
test's *name* was not captured either time, which is the thing to fix next
sighting: run the suite so the `failures:` block survives, rather than grepping
for a summary line. Two sightings eleven days apart in the same crate is a
pattern forming, not noise to keep ignoring — the likeliest suspects are the
tests that share process-global state (`MECHA_HOME`, which the trigger and work
tests set behind a lock) or the `temp_store` helpers keyed on pid plus thread
id.

| Suite | Count |
|---|---:|
| `mecha-core` unit | 1,081 |
| `mecha-cli` unit | 662 (1 ignored) |
| `mecha-cli` `first_run` integration | 20 |
| `mecha-mail` unit | 133 (+1 in the `mecha-mail` binary) |
| `mecha-slack` unit | 75 |
| integration (`mcp_server` 6 + `sandbox_backends` 9) | 15 |
| doctest | 1 |

Measured 2026-08-30 (~23:45 UTC) on `main` at `ab0097b`, same tree as the
first prose figure above (1,988). The table had drifted two counts behind the
prose once already, which is the failure mode of stating one fact twice — read
the prose if they ever disagree again, and fix the table.

The integration tests need docker (with `debian:stable-slim` and `python:3-slim`
pulled) and `python3`; without them they skip and say so. CI sets
`MECHA_TEST_REQUIRE_BACKENDS=1` so a missing backend fails instead of quietly
passing — a silently skipped test reads exactly like a passing one.

**The system prompt is wired.** `~/.mecha/config.toml` points
`[agent] system_prompt_file` at `prompts/agent.md`. It was not, for the whole
life of the project before 2026-08-03 — `RunConfig` recorded
`system_prompt: null`, and the model had none of that file's guidance. Check it
is still set before concluding anything about model behaviour.

## What exists

A working agent harness, used and measured rather than just compiled.

| Area | State |
|---|---|
| Providers | Anthropic (raw HTTP, **verified live**) + OpenAI-compatible (llama-server, vLLM, Ollama) |
| Agent loop | Streaming, tool dispatch, parallel execution, forced final answer |
| Tools | `fs_read/write/edit/list`, `shell`, `http_fetch`, `todo`, `web_search`, `ask_user`; `recall` on session-recording front-ends (chat, TUI, resumed runs) — searches the transcript union, including what compaction rewrote away |
| Planning | `Phase::Plan` hides writing tools structurally — not offered, not dispatchable |
| MCP | stdio client; per-server on/off; capability overrides that only widen; `prefix_tools = false` for a server whose tools carry their own namespace (collision fails startup loudly) |
| Memory | mecha-graph (the artist formerly known as pkg, now public) wired as MCP server `graph`, unprefixed `kg_*` tools — the user's mail, Slack, calendar, conversations |
| Subagents | `Agent` wrapped as a `Tool`, allowlisted registry, per-profile model |
| Search | `SearchBackend` trait — Exa, Tavily, SearXNG — a preference-ordered chain, first to *answer* wins. An exhausted chain of genuine empties is an answer; only a chain where nothing answered is an error. `prefer_deep` promotes a backend to the head on `depth: "deep"`, reordering and never filtering, so every backend stays reachable as a fallback at either depth |
| Security | Path jail, SSRF guard, trifecta interlock, leak guard, capability model |
| Sandbox | `shell` and MCP servers confined via bubblewrap, docker, or landlock (no-privilege file confinement; never narrows `external_send` — UDP is unrestrictable, and the preflight plants a home file and requires the confined read to fail) |
| Budgets | `max_turns`, `max_output_tokens`, `max_cost_usd`, cost accounting |
| Control | Ctrl-C cancels mid-stream and keeps the partial turn; mid-run steering |
| Context | Two-pass compaction: thin tool results, then summarise. Taint preserved, a tool's own state (the todo list) crosses verbatim, the states a mid-run rewrite replaced ride `Conversation::rewritten` into the session record, and a per-run cache lens watches whether the cached prefix is actually reused (warns only on unexplained re-payment) |
| Interfaces | `run`, `chat`, `tui`, `batch`, `eval`, plus `review` / `outbox` / `trigger` / `work` / `proposals` / `rules` for review and upkeep, `slack` for the remote control, and `serve` for the tailnet web surface |
| TUI | Slash commands with menus and completion; switch model/provider/mode/MCP mid-session; shift+tab toggles planning. Review lives here too: `/queues`, `/outbox`, `/frontdoor`, `/mail`, `/tasks`, `/skills`, `/polls`, `/doctor` and (2026-08-23) `/find` modals drive the CLI like `/triggers` does — plus `/note` for one-line graph capture — the status line badges pending drafts, and `/review now\|later\|auto` decides what happens when a run stages some — scoped to that run's items by an id-diff, tainted drafts never auto-released, the mode set only by command (never parsed from the prompt). Detached releases/extractions/triages are watched and their results reported without a reopen |
| Slack | `mecha slack` — a remote control: Socket Mode from home, an owner allowlist bound by a locally printed nonce, a thread as a `Conversation`, streamed answers with a task card per tool call, approval cards (incl. "allow for this run"), outbox review cards, files both ways, `notify`; owner-gated command words `doctor`, `triggers`, `review now|later|auto`, and (2026-08-23) `note <text>` — a deterministic capture matched before the text can become a prompt — `queues`, the read-only backlog rollup, and (2026-08-24, adopted from an orphaned WIP) `tasks`/`task`, the GTD board as command words. **Merged 2026-08-09 (PR #25) and running as `mecha-slack.service`** |
| Web surface | `mecha serve` (2026-08-24, extended the same evening) — the tailnet web app: binds 127.0.0.1 with no flag to widen it, `tailscale serve` is the door (`:8443`), every request must carry `Tailscale-User-Login` equal to `[web] owner_login` (global-file-only config, stripped from project layers like `[slack]`; refuses to start ownerless), strict self-only CSP. Pages: Home dashboard (`review queues --json` + `doctor --json` as child processes, a dash never a zero), streaming chat over SSE (one shared agent, per-session `RunContext` on the Slack connector's pattern — keyed sessions with validated directory-safe names, jails under `~/.mecha/work/web/<key>/`, steering, cancel, context gauge), outbox review (whole `DraftView`, source reads behind a gutter, taint sheet with exact args, approve `--yes`/reject/edit as CLI children), graph-queue sample deck (seed printed, verdict ≠ resample), tasks, notes + `kg search`. Per-session `ask` mode: live approval cards (deny-with-reason is a real user correction; timeout is `Blocked`) and `ask_user` option cards routed by the run's jail (`Asker::ask_in`); pending cards ride the transcript read so a locked phone reloads into them; cancel drains parked cards. Evening additions (2026-08-24): the **mail page** (`serve/mail.rs` + `Mail.svelte` — store read for the list, `mecha mail show` as the one thread renderer behind a gutter, closed-verb `/api/mail/act` with spam the only confirm; drafting verbs spawn detached into the outbox), the **graph queue at all three depths** (classes with server-stamped tiers from `tui::queues::Tier::of`, per-class similarity groups, and the cross-class global layer with a threshold stepper — see the graph repo's `similar.rs` for the invited-crossing rules), and **files** (`serve/files.rs`: uploads into the session jail's `inbox/` announced as paths, downloads that re-prove containment, images the only inline type). Assets are a build artifact at `~/.mecha/web/dist` (update skill surface 1b). **2026-08-25**: `review now` reached this surface — a finished run emits the ids it staged and the page draws a confirm card built from `/api/outbox/{id}` (whole `DraftView`, taint above everything, source reads behind a gutter, Send now / Later); notes open in place and edit through `mecha kg note --edit <source_id>`, which preserves the note's own `occurred_at`; and a failed graph verdict keeps its card and offers the two ways through (`bind`, accept-as-new-topic) that until then existed only in the TUI. **2026-08-26**: the board learned to delegate — *ask mecha* (`/api/tasks/work`, detached and unattended, because `serve`'s approval cards belong to a chat session's `RunContext` and cannot reach a child process), *stop* (`/api/tasks/stop`), *open the conversation* (`#chat/<session>`, which `Chat` resumes from the route), an agent chip derived from `waiting_on` rather than self-reported, `task:` sessions in the drawer, and the plan rendered in chat (live, from the shared `TodoTool` keyed by jail) and on the card (`/api/tasks/plan`, read out of the transcript because a `tasks work` run is another process). **2026-08-26 (second pass)**: the delegation loop closed — `/api/questions` (list, answer, abandon) so a parked question is answerable from the phone rather than only from a terminal, and the task card's state derived from board + questions + the transcript's outcome record (D16). **2026-08-26 (fourth pass)**: the graph queue became reviewable rather than only tappable — a group opens to its members (`GET /api/queue/items`, a named id set re-fetched by id, never a redraw) and each is verdicted on its own with no cascade, because similarity is the grouping key and members can contradict each other; a candidate's face comes from one `faceOf()` matched to `tui::queues::items_from_json`'s chain (`statement` → `what`), which is what stopped every commitment card rendering `undefined — undefined — undefined`; and a failed *bind* offers a target field, distinct from a failed accept, where naming a target is not the answer. `docs/REMOTE-SURFACE-RESEARCH.md` + `-DESIGN.md`, `docs/TASK-AGENT-DESIGN.md`. **2026-08-29 (#114)**: an **entity page** (`/api/entity` via `serve/board.rs::entity`, `Entity.svelte`, nav entry `graph`) marking unreviewed and denied facts as such; a **surfaced-verdict deck** on the review page (`/api/queue/shadow` + `/verdict`, `serve/review.rs::shadow`/`shadow_verdict`); and **chat tool-result previews** (`WireEvent::ToolResult`, a capped preview of what a tool answered). Same night (#116) the `/tasks` page was repaired — `stateOf` had called the server-stamped `stalled` *field* as a function, a `ReferenceError` on every card of a non-empty board, shipped broken in v0.1.16. **2026-08-29 (#118)**: settings became an **index** rather than one scroll — three rows opening panes at `#settings/<charter|learning|voice>` through the same hash router, each row carrying what is actually in there (a count, or a dash where the store could not be read) — and the gear moved out of `Home.svelte` into the shell (`App.svelte`), one button in the same corner on *every* view at `z-index: 3`, below the app's scrims and sheets (4-6) and drawers (40+). The charter is edited as a list with **drag-to-rank** (`SettingsCharter.svelte`; pointer events rather than HTML5 drag-and-drop, because `dragstart`/`dragover` never fire for touch): position in the file is the ranking, so dragging is the only rank control there can be. Everything above the first `[[line]]` survives a save, and a document the page cannot fully account for — a `parse_error`, bytes it never managed to read, or a comment among the tables — refuses the list editor and opens as raw TOML instead (`unreadable`, `blocked`). Routing moved to `pushState` with a depth stamped in `history.state`, so a back gesture can tell an in-app push from a cold deep link. Two gates ride it: `npm run check-charter-toml` pins the serialiser (`web/src/lib/charter-toml.js`) against `charter.rs`'s own `WEB_EDITOR_SAMPLE` fixture in both directions, and `render-check` now visits the three settings panes. **2026-08-30
(#126)**: the graph tab became the curation surface, iterated live against
the owner's phone in one evening — capture is a chat-idiom composer (send
armed by content, ⌘⏎), the notebook is a bottom drawer pinned in layout
(sortable, filtered, `?limit=` passthrough clamped to the graph's own
200), and the restyled entity card does the identity lifecycle in place:
alias chips with two-tap removal and inline add (`kg alias`/`kg unalias`,
id-only like `retract`), **merge-with-audit-trail** (`/api/entity/merge` →
`mecha-graph proposals file-merge --accept`, so the graph's one no-undo
verb always leaves a decided proposal; ambiguity refused with the
candidate list, relayed whole), **create-on-miss** (`new-person`/
`new-node` from the lookup's dead end), and a read-only `reaches` row of
identifiers (aliases are how a node is spoken of, identifiers decide where
future ingest lands). `serve/board.rs::graph_verb` spawns `mecha-graph`
directly — the owner's lane, deliberately not on the model's MCP surface.
Beside it, `serve/proposals.rs` + `Proposals.svelte`: one pane over the
three proposal stores (harness · rules · graph entities), read-gated
decisions (accept refuses until the item's `show` was rendered; a merge
accept gets its own confirmation sheet), depths that are `None` when a
store cannot be read, and a 503 naming `MECHA_GRAPH_BIN` when the graph
binary is absent |
| Voice | The stack from `docs/VOICE-RESEARCH.md`, built and in production 2026-08-24 (§7 is the build log): Pipecat worker (`scripts/voice/worker.py`, `:7860`), **Parakeet TDT** STT (`mecha-parakeet.service`, `:8992` — Voxtral was structurally unfit: a chat model answers speech instead of transcribing it, and obeys spoken instructions), Chatterbox TTS (no standby — Kokoro was removed 2026-08-25; nothing failed over to it automatically), and the loopback OpenAI facade (`mecha-cli/src/voice/`) **mounted inside `mecha serve`** (`--voice-port 8990`) over the shared agent — one process, one cached prefix, two dialects. The WebRTC offer proxies same-origin through serve (`/api/offer`), behind the owner guard — true of **both** doors since 2026-08-25, and only of `:8443` before it, when `:443` was a file mount whose `/api` went straight to the worker. In-chat voice: waveform button → call overlay (voice-core.js embedded by relative import; threaded transcript pane, cloned-track mic meter, mute, end). **A call is the chat session it was started from (D3, 2026-08-25)**: the page names its key in the WebRTC offer (`request_data`, pipecat's own passthrough), the worker forwards it as `X-Chat-Session` beside the slot key it still mints, and `voice::SessionHost` — implemented by `serve::chat::VoiceHost` — runs the turn on that conversation's messages, taint, transcript and jail, with the facade keeping no record of its own. `chat::begin_turn` is the one implementation both doors go through. Spoken turns arrive on the page's SSE feed live (`WireEvent::User`, block stripped) and are marked `spoken` in the transcript; the D10 block now opens a *switch into speech* rather than a conversation (`last_turn_spoken`), and `--voice-yes` travels with the turn, so a spoken turn runs at Allow while a typed one in the same session obeys the page's mode. D5 ratified: owner speech is typed text, arms nothing. **Voice controls (2026-08-24 night):** the in-chat call overlay (`Chat.svelte`) carries a **seven**-voice picker and a 0.5–2.0x rate slider. They persist in `localStorage` (`mecha.voice.prefs`, `{voice, speed}`, read on each connection's first `onVoiceConfig`), so the next call opens where you left it rather than resetting to whatever the worker booted with. That key originally synced two shells; the standalone page was retired in 876580e and the preference is why it stays. Seven is six Kokoro-derived cloning references plus Chatterbox's own built-in `default`, which the server lists as selectable because it is one — `voice: "default"` generates with no reference rather than falling back to anything. The controls are driven by a `voice-config` RTVI message and `session.voiceConfig(patch)` on `voice-core.js`; the server's reply is what renders, so a refused value never leaves the control showing a rate the worker is not speaking at. Rate is a pitch-preserving phase vocoder in `chatterbox_server.py` (~50 ms warm) because Chatterbox Turbo has no speed parameter and resampling moves pitch with tempo. The voices are Kokoro presets synthesized into cloning references by `scripts/voice/make-voices.py` — Apache 2.0, nobody's identity — and the server reads the directory live (`GET /v1/voices`) rather than holding a list. **Call teardown, 2026-08-25:** the pipeline's idle timeout was pipecat's unchosen 300s default and killed a call five minutes into any pause — raised past a conversational silence and it now *announces itself* over the data channel before tearing down; client-side, ICE `disconnected` is a 15s grace window rather than a hang-up, since only `failed`/`closed` are terminal (no ICE restart: pipecat's `restart_pc` fires the very event this worker cancels the pipeline on). **Spoken outbox confirmation, 2026-08-25:** a run that stages drafts is asked about aloud — the offer composed from the store through `DraftView::spoken`, the answer matched by `review_policy::parse_answer` *before* any model sees it, so the release decision never enters a context window |
| Sessions | Append-only JSONL, resume, taint recorded, `RunConfig` per attach |
| Replay | `replay.rs` diffs trajectories, `replay_run.rs` drives them — `mecha replay`, incl. cross-model. Counterfactual probes **branch** rather than regenerate (`counterfactual::branch_at` + `drive_branch`, 2026-08-30): the recorded prefix is resubmitted verbatim and only the continuation is sampled, so pre-point divergence is structurally impossible; replay wrappers narrow `external_send` in the non-executing modes, and a recorded surface rebuilds from its `SurfaceStore` blob — dead tools included, recorded descriptions winning over live rewordings |
| Hooks | `pre_tool` (can deny, fails closed) / `post_tool` / `session_end`, JSON on stdin |
| Outbox | `[outbox] tools` staged for review instead of executed; `mecha outbox` list/show/edit (`--body-file` for surfaces with no `$EDITOR`)/**review**/send/reject, several ids or `--all` narrowed by `--kind`/`--via`; edits mined as writing reflections. Items carry a kind — a publish shows its rendered page, refuses `edit`, and is excluded from the miner — and the jail they were drafted under, so a release resolves paths against the agent's workspace rather than the reviewer's. Release policy is `review_policy.rs` — one encoding for every surface: `now` (the default) puts a finished run's drafts in front of you, `later` leaves them, `auto` releases only untainted drafts of a run that finished clean. Scope is `staged_since`, an id-diff, so no mode touches items another run staged. Since 2026-08-25 `now` reaches `mecha serve` too — a card on the page, a spoken offer in a call |
| Messaging | `[messages]` + `mecha msg send/list/show/dismiss/agents` — a file mailbox between this machine's sessions (`~/.mecha/messages/<recipient>/`, producer-name addressing, per-session liveness registry). Delivery folds in at the steering point with the sender's harness-stamped taint merged first, so a hop launders nothing; attended surfaces hold with a notice, unattended accept; global config only; full mailboxes refuse, identical pending sends dedup. `docs/MESSAGING-RESEARCH.md` is the design record; phase 2 (TUI modal/badge) is scoped there |
| Workspaces | `~/.mecha/work/<producer>/` is a run's workspace and its output directory; `mecha work list/path/clean`, retention nightly. A workspace containing the mecha home is refused |
| Mail | `mecha-mail` crate: Gmail + Google Calendar and Outlook + Graph calendar; **`mecha-mail` is the binary deployments wire** — one account-based surface (`dartmouth`, `personal`) over every mailbox in `~/.mecha/mail/`, reads fanning out, item ops account-scoped; the per-provider `mecha-google`/`mecha-outlook` binaries remain; all sends and calendar writes outbox-routed. **`mail_triage`** (2026-08-18) adds archive/read/unread/spam/trash as a closed `TriageAction` enum, thread-level, in a third capability quadrant — `destructive` but *not* `external_send`, so it never routes through the outbox and a read-only run cannot reach it. Tagging is deliberately absent: a tag is mecha's own, on the triage record, not a Gmail label or a Graph category |
| Tasks | `mecha tasks` list/add/set and the `/tasks` modal onto the graph's GTD board, reached only over `kg_task_*` — no dependency on mecha-graph and no second reader of its schema. Status letters match `mecha-graph tui` screen 6; nothing confirms (the board reaches nobody and has no delete); a reload re-finds the cursor by id because a status change reorders the board. **2026-08-26**: `tasks work <id>` seeds an agent run from a board item (own session titled `task: …`, outbox-bound, `--unattended` for the trigger posture a detached caller gets) and `tasks stop <id>` asks it to stop through a sentinel it polls — keeping the partial turn, never a kill. The harness moves `waiting_on` to `mecha` for the life of the run and to `@owner` when it ends, so "is work happening" is answered by the board rather than by the run; `kg_task_update` is withheld from the model for the same reason `kg_accept` does not exist, and `setup::subagents_holding` refuses to start when a profile would hand it back. `tasks set --waiting-on` is the hand-driven half. **D16 (2026-08-26, second pass)**: the card's state is derived from three sources and none of them is the run's account of itself — the board says who holds the ball, the question store says whether it is blocked, and the transcript's `Record::Outcome` says how the last run stopped. Seven states, no two rendering alike: `working` (with the `[~]` item as its subtitle), `planning`, `answer needed`, `ready for review` (with its evidence — turns, calls, staged, refused), `the run failed`, `outcome unknown`, and quiet. `unknown` is the honest seventh rather than a hedge: a transcript with no outcome record is a run that never got as far as saying how it went, and calling it either `failed` or `ready` invents the one fact the card is about. `Interrupted` reads as ready, never failed — a person stopping a run is the system working. **Provenance** (2026-08-26, a second lane): a task carries `captured_from` — a *pointer*, kind from a closed set of `mail | frontdoor | session` with unknown keys refused, so it can never become a copy of an email body. `mail task` writes it at capture, `mecha tasks source <id>` follows it with one reader per kind, `POST /api/tasks/source` and the card's `read the …` control do the same from the web, and the TUI offers `o` from the detail pane only — off the key strip, because it is inert on any task somebody typed. **B1/B2 (2026-08-26 evening, both amended before building)**: `✓` is a tap on the collapsed row rather than a chip in the strip (Things: complete is "a tap on the circle only"), and the expanded card *groups* under `hand it over` / `move it` instead of hiding four chips behind a `…` — the survey it cites is about swipe actions on a *collapsed* row, and this card only exists once tapped, so it already is the sheet. Capture is **one box**: `capture::find_when` parses a `when` out of the sentence in Rust, shows it as a dismissable chip, and sends the name **verbatim** (Things, not Todoist), with due/context behind `more`. It detects and never resolves — the span goes to the graph's `parse_due`, so one date parser lives where `+3d` means something; weekdays are deliberately undetected because `parse_due` cannot take one and a chip that lies is worse than none. There is no time of day anywhere in the store, so "at 3" stays in the name |
| Questions | `mecha_core::questions` + `mecha questions list/show/answer/abandon` — the outbox's inbound twin. A delegated run that needs a decision **ends**: `ParkingAsker` stores the question and cancels the run's own token, so the partial work survives and no slot is held waiting. Answering *is* resuming — the answer becomes the next user turn of the session that asked, in the jail it asked from, with its plan restored. Taint is recorded at park time and unknown reads as untrusted, because a question is an inbound request for information composed by a model that may have been reading third-party text. Sixth row in `/queues`; doctor flags one unanswered past 24h. **2026-08-26 (second pass)**: the phone can answer one — `GET /api/questions` is a direct store read (mecha's own store, so `review.rs`'s pattern rather than `board.rs`'s CLI child), the card lands on its task in `/tasks` because D13's own argument is that the Waiting view *becomes* the queue of blocked delegations with no new noun, options are one tap and the free-text box is there because the tool's contract says they are never exhaustive, the taint marker travels (stderr's `⚠` is invisible in a browser), a question whose task is off the board still gets a card, and answering spawns `questions answer --unattended` detached because answering *is* a whole agent run |
| Graph reads | `mecha kg search\|entity\|note` (2026-08-23) — the graph for the person at the keyboard, over the same `kg_search`/`kg_entity`/`kg_upsert` surface the model uses. `/find` is the modal (entities open their full record, facts/episodes open in place, `/` re-edits the query); `/note` (or `/notes`) captures an episode with entities linked on landing, identically to `mecha-graph note`. All fetches off the event loop through watches |
| Documents | `mecha-docs`, the fourth binary on `mecha-mail` — Google Docs/Sheets/Slides under **`drive.file` and nothing else**, so only files mecha created or the user picked in Google's own chooser are reachable, and no instruction inside a run can widen that. Reads are `untrusted_input` and never `openWorldHint`; writes are outbox-routed, because writing into a document a third party can read is a publish. No permanent-delete and no sharing verb, with tests on the absences |
| Remote control | `/remote-control <name>` in the TUI mirrors a live session into a named Slack thread, both directions. Store `~/.mecha/remote/<name>/` (record + inbox + staged files), written by the TUI and read by the connector, which no longer starts its own run in a mirrored thread. Out: `/send <path>`, `mecha slack send`, and the `show_file` tool — whose destination is not an argument and cannot be made one. In: attachments land at `./inbox/`, announced as paths so the taint arms through `fs_read`. Slash commands and `!` stay at the terminal. `mecha slack remote [--sweep]`. `docs/REMOTE-CONTROL-DESIGN.md` |
| Front door | `mecha frontdoor` list/show/extract/next/**triage**/**needs-info**/**close** over `~/.mecha/requests/` — the quarantine between a stranger's request and a run with tools, and the state machine that lets one reach an answer. The extractor is issued no tools and no history; `Record::for_privileged_run` has no argument that returns the prose; an extraction failure routes to a human. `triage` drafts into the outbox and refuses to run unrouted; `reconcile` closes the loop from released items on its own, with no verb to remember. `mecha-factory-publish drain` fills the directory |
| Triggers | `mecha trigger` — a prompt on a cron schedule, unattended: `add/list/show/next/run/tick/daemon/runs`, store in `~/.mecha/triggers/`, ledger in `runs.jsonl`, **the daemon is installed and running here**; a failed `notify` is recorded on the run |
| Skills | `~/.mecha/skills/<name>/SKILL.md` in the Agent Skills format, loaded by a `skill` tool call at three levels of disclosure. User-authored with no mechanism for anything else — no install, no registry, no remote body, none derived from a session — which is why loading one arms no taint. `tools:` narrows the surface and can never widen it; a loaded skill crosses compaction verbatim; `mecha eval` forces them off |
| Learning | the full arc, **ungated since 2026-08-30** (Luke's 2026-08-19 ruling, built): reflect-on-close → `learn --auto` per session close (`learn-live.sh`) and nightly — the counterfactual gate in front of the write (regression refuses, clean applies, ungradeable applies on **probation**, retiring at 2 vs the ordinary 3) — with every pass still writing a proposal as audit trail and superseding its pending predecessors; nightly `rules propose-retirements --apply` retires from the ledger with no human. Git-backed store under `~/.mecha/learning`; validate feeds a per-rule outcome ledger with regression bisection; `mecha learning-report` (+ web trend pane) is the is-it-working view. Budget is 25 active rules and 2600 chars **per domain**, and a run carries only `RUN_DOMAINS` (`behavior` + `writing`) — new domains are opt-in and `unrouted_domains` warns at startup on any that ride in no prompt |
| Run quality | `Record::Outcome(RunStats)` per finished run from every front-end; `runlog.rs` reads the corpus back (`mecha sessions health`, rates split by model, `—` where a denominator is zero); three population checks in `doctor`; `candidate.rs` gates a proposed change on a paired comparison with a deterministic holdout and a work guardrail; `mecha eval --ab-config KEY=VALUE` is the content-sensitive arm; `mecha diagnose` proposes one change from the corpus and prints the command that would falsify it; `mecha harness` (2026-08-22) closes the loop nightly — candidates persisted, measured by session replay, a holdout-confirmed config win auto-accepted into a revertible override layer beneath the user's config, everything else staged for review — see the self-improvement section |
| Queues | `mecha review` + the `/queues` modal — every store waiting on a human in one list: the graph's merge queue, the outbox, the front door, staged rule changes, harness candidates. Four hand off to the modal that owns them; the graph queue is reviewed in place, four levels deep (mechanism → class → similarity groups or a *random* sample → items), `t` filtering by evidence tier and `a`/`r` verdicting. `s` on a class groups its whole queue by semantic similarity (2026-08-23) — every pending item, largest groups first, singletons after — and a group verdict is ONE human verdict: the seed is the owner's, members cascade machine-labeled (`reviewed_by = cascade:<seed>`, invisible to the ladder) onto the exact ids that were on screen; `b` binds a group's subject, `A` accepts creating it, `[`/`]` re-group at a threshold stepped from the value the child reports. An unreadable store prints a dash, never a zero. **2026-08-26**: `B` names a bind target by hand and a failed `b` opens the same prompt — the graph answers an unbindable subject with *name a target with `--to`*, and until then no surface could send one; the prompt owns the keyboard while it is up, because `a`/`r`/`d` below it are verdict keys. Same day, `items --ids` stopped being trimmed to ten by `--top`, which had capped the group dive since the level shipped. **The one place mecha shells out to `mecha-graph`** — the MCP surface has no `kg_accept` and must not gain one. **2026-08-29 (#114)**: the graph's *shadow* queue (review-on-use surfaced verdicts) reached every owner surface — `mecha review shadow` lists and decides (`--confirm`/`--refute`, through the same mecha-graph child), the `/queues` modal grew a graph-shadow row with in-place verdicts riding the generic review level, and `/find`'s entity detail marks each fact's tier |
| Eval | 36 cases, 15 tags (both re-counted 2026-08-20, unchanged), scorecard, `--compare`, sandboxes, verify, judge, multi-turn, run-metadata checks; plus `graph-cases.jsonl` — 10 memory/interlock cases against fixture MCP servers (`--mcp-file`), renamed with the graph and expecting the bare `kg_*` names production serves (scorecards across the rename are not comparable) |

`cargo clippy --all-targets` is clean and should stay that way.

## Environment as left

> **This checkout is a live service's `ExecStart`. Do not `git stash`, `git
> checkout --`, `git restore` — or `git checkout <branch>`.**
> `~/.config/systemd/user/llama-local.service` names
> `/home/ljchang/Github/mecha/scripts/start-moe-mtp.sh` **literally**, so the
> file in this working copy *is* the server's launch command. Anything that
> rewrites it — a branch switch, a revert, a stash — changes how the model is
> served on the next restart, and it comes back healthy and merely behaves
> differently, which is the worst way for it to be wrong.
>
> As of 2026-08-21 the clone is on **`main`** and the script is **committed**,
> so the sharper 2026-08-20 hazard (a parameterised but uncommitted script that
> a routine `git checkout main` would have reverted) is retired. The standing
> rule is not: **work on `main` from a separate worktree** and leave this
> checkout where the unit points. That is the correct application of the
> worktree advice rather than a contradiction of it — the arc that owns the
> service-referenced path stays put and everyone else moves.
>
> **The prohibition above got violated twice on 2026-08-30, by two sessions,
> and the mitigation is what held — not either session's care.** Both branched
> in this checkout (`git checkout -b`) without checking, because the rule as
> written is absolute and branch work is not optional, so in practice it gets
> read as advice. Nothing broke: `scripts/start-moe-mtp.sh` hashed
> `d76da36c` identically on `main` and on all three live branches, and
> `llama-local` stayed active. That is the 2026-08-21 fix working, not luck
> being careful. So the rule is worth carrying as a **check** rather than a
> ban, because a check is a thing a session can actually satisfy:
>
> ```
> git rev-parse <branch>:scripts/start-moe-mtp.sh
> git rev-parse main:scripts/start-moe-mtp.sh      # equal => the switch cannot move the server
> ```
>
> Equal hashes mean a branch switch here is invisible to systemd. Unequal —
> or a *dirty* working copy of that path, which no branch comparison can see —
> means stop and use a worktree. Check before the switch: afterwards the file
> has already changed, and it fails by coming back healthy and behaving
> differently.
>
> The general lesson is in HISTORY under Environment: **"move it to a worktree"
> assumes nothing outside the repository points at a path inside it**, and here
> systemd does.

Running on the DGX Spark (GB10, aarch64, 128GB unified). **Re-verified
2026-08-21**: 8080 up with `total_slots=4`, `n_ctx` 262,144 per slot and
`modalities.vision: true`; 8081 up serving embeddings; 8082 and 8083 down;
SearXNG up on 8888. `~/.cargo/bin/mecha` was reinstalled 2026-08-21 (03:33, then again at
~15:00 and ~17:00 for the search and outbox arcs) and the three long-running
services restarted onto it each time.

**2026-08-23 evening**: `mecha`, `mecha-graph` and `mecha-graph-mcp` all
reinstalled from that day's HEADs (several times; the `update` skill's
stale-process sweep run after each) and the three long-running services
restarted onto the final install. The graph DB migrated to V017
(`reviewed_by`). Operationally pending: **one class has an earned ladder
promotion unapplied** (`mecha-graph ladder --promote` shows and applies it),
and the graph queue stood at **6,569** after the day's review work, down
from 7,296 at breakfast; by the evening of 2026-08-24 it read 7,505 in
`review queues` (the nightly proposes faster than class-at-a-time review
clears), which is what prompted the global similarity layer.

**A version string is not evidence here.** The repository and the binary both
say 0.1.9 and the reinstall changed no version, so `mecha --version` cannot
distinguish a current install from a skipped one — check a behaviour the change
introduced (`mecha run --help` carrying `--image`), and check
`/proc/<pid>/exe` for `(deleted)` to catch a *process* still running a replaced
inode. The `update` skill carries both checks.

**2026-08-24 (re-verified after the 0.1.13 release, ~21:45)**: `mecha
--version` reports **0.1.13** from `~/.cargo/bin`, `mecha-mail` reinstalled
beside it, web assets rebuilt and rsynced to `~/.mecha/web/dist` (the served
door returns the current bundle), and `mecha-serve`, `mecha-slack`,
`mecha-triggers` and `mecha-drain` all restarted onto the new inode and
logging their startup lines. **The graph binaries were deliberately NOT
reinstalled**: another session was mid-arc in the private graph checkout
during the release freeze, and installing `mecha-graph`/`mecha-graph-mcp`
from a working tree in flight is worse than leaving a known-good install
alone — so `~/.cargo/bin/mecha-graph*` was that session's to update, not
stale by accident. **That arc has since landed** (graph main `f312eb0`) and
both graph binaries are installed from it and verified answering twelve
`kg_*` tools. The carve-out paid for itself: checking install times against
the merge is what surfaced `mecha-graph-mcp` sitting an hour stale behind
`mecha-graph` — two crates, two installs, and the MCP one is the only one
mecha reaches at runtime. The update skill now says so in as many words. One `mecha tui` started ~15:10 is still on a deleted
inode with its three MCP children; it degrades gracefully since 0.1.12 and
needs its owner to restart it. Also not refreshed by this pass and
independently stale: the musl benchmark binary (`target-musl/release/mecha`,
built 2026-08-23), so **re-run `bench/build-portable.sh` before trusting any
scorecard against 0.1.13**. The factory client is current at 0.2.7 and the
droplet was not touched.

**2026-08-31 (~12:45, after the 0.1.17 / graph 0.1.4 releases)** — the first
pass in a while where all six `update` surfaces were taken in one sitting, so
this supersedes the install claims above rather than adding to them. Verified
by asking each artifact, never the repo:

- `mecha --version` → **0.1.17** from `~/.cargo/bin`; `strings` on it contains
  `left_pending`, which is the literal the release added, so the install is
  the new *build* and not merely a new mtime.
- `mecha-mail` reinstalled beside it (note: it had been installed from a
  *scratchpad worktree* — cargo reported replacing `mecha-mail v0.1.16
  (/tmp/…/wt/appraisal-fixes/mecha-mail)`, which is the deferred-failure shape
  the web assets already had a rule about, one binary over).
- Both graph binaries at **0.1.4**, and `mecha-graph-mcp` answering **13**
  `kg_*` tools from the installed path (twelve previously; `kg_verdict` is the
  addition).
- The seventh binary — the one `scripts/nightly.sh` executes directly out of
  the graph repo's `target/release` — reports 0.1.4 with `candidate_embedding`
  compiled in. Checked rather than assumed to have been refreshed by
  `cargo install --path`.
- Web assets rebuilt from `b132157` and rsynced (`index-WdOIXhHv.js`,
  replacing Aug 30's `index-BlZ4I5_n.js`, whose mtime matched
  `mecha-serve`'s start time exactly — a normal earlier deploy, not another
  lane's live test; no `deployed-local` tag).
- All five services restarted and confirmed by **startup line**, not
  `is-active`: Slack "1 owner(s), 16 thread(s)", triggers "1 trigger(s), 1
  enabled", serve printing both doors, voice worker on 7860.
- Stale-process sweep clean. Factory client **0.2.8**, droplet already serving
  **0.2.8** (read-only check only). Sandbox image cargo 1.97.1, identical to
  host.

**Not verified, and stated as such:** the *served* bundle. `63242` answers 403
to an unauthenticated curl, which is the owner gate working correctly, so the
end-to-end check is the owner loading the page. The dist on disk is right and
the service restarted after it landed.

**The musl benchmark binary does not exist at all** (`target-musl/release/`
is empty), which is a different state from stale and a safer one: it cannot
mislead a scorecard, and `bench/run.sh` builds it via `build-portable.sh`.
Worth knowing that the build will therefore happen *inside* whatever window
someone starts a benchmark in.

> **Superseded on the binary and the assets, 2026-09-01 ~19:44.** Another lane
> deployed a live test over this — `~/.cargo/bin/mecha` rebuilt from an
> **uncommitted** tree, `~/.mecha/web/dist` replaced (`index-klt_h5v8.js`,
> 8443 confirmed serving it; the 0.1.17 bundle is kept at
> `~/.mecha/web/dist.prev`), five services restarted. Everything else above —
> the graph binaries, the nightly's seventh binary, factory, droplet, sandbox
> — is untouched and still current.
>
> **`mecha --version` reports 0.1.17 and is not the release build.** The tree
> it was built from carries that workspace version, so the version string,
> a fresh mtime and a green install all agree with each other and are all
> wrong together. Do not read the paragraph above as describing the binary
> currently on this box.
>
> The discriminator is the artifact, not the version:
> `strings ~/.cargo/bin/mecha | grep -c "You name conversations"` returns
> **1** on the deployed test build and **0** on released 0.1.17 (verified
> both ways). **`deployed-local` is deliberately NOT set** — that tag names
> the *commit* whose build is installed, and an uncommitted tree has none, so
> pointing it at `HEAD` would assert the release is deployed: the same false
> inference, written down more confidently. The tag becomes honest when that
> work is branched and committed, and that lane will set it then.
>
> **The tree has moved since that binary was built, and rebuilding will not
> reproduce it byte-for-byte.** A `cargo fmt --all` landed on
> `mecha-core/src/session.rs` and `title.rs` afterwards, so the install is
> from pre-format source: whitespace only, no behavioural difference, and the
> probe above still answers 1. Deliberately not reinstalled — spending a build
> against ~19 GB free and no swap to change indentation inside a binary is the
> wrong trade while two llama-servers hold the machine, and a server that
> reloads under memory contention does not recover (`LLAMA-SERVER.md`).
> Recorded because "the tree builds what is installed" is the assumption a
> reader makes for free, and here it is false in a way nothing announces.

**2026-08-24 (re-verified this date, evening)**: `~/.cargo/bin/mecha`
reinstalled again for the web-review-surfaces arc (final at merge `cfab345`)
and `~/.cargo/bin/mecha-graph` + `mecha-graph-mcp` reinstalled from the
private checkout's `global-similarity` merge; all mecha-binary services
bounced onto the final inodes. One `mecha tui` (started that evening) was
seen on a `(deleted)` inode and left running — it degrades gracefully since
0.1.12 but is stale until its owner restarts it.
Production is now **one `mecha-serve.service`** (user unit, `mecha serve
--voice-port 8990 --voice-yes`) owning `:63242` (web, fronted by `tailscale
serve :8443`) and the loopback voice facade on `:8990`; `mecha-voice-serve`
is retired (disabled). New units beside it: `mecha-parakeet` (`:8992`, STT)
and `mecha-voice-worker` (`:7860`, Pipecat, reached through `mecha serve`'s
own `/api/offer` proxy rather than directly). Web assets live at `~/.mecha/web/dist`
— a build artifact, not in git; the `update` skill's surface 1b rebuilds
them. 8080 re-verified this date: `total_slots=4`, `n_ctx` 262,144/slot,
vision true. **Re-verified 2026-08-25** and all four unchanged
(`total_slots=4`, `n_ctx` 262,144/slot, `model_alias qwen3.6-35b-a3b`,
`modalities.vision` true). Workspace tests **2026-08-26: 1,408 pass, 0
fail** (`main` at 8ce53dc, clean tree; 1,377 at 897bd13 the previous day).
Eval: 36 cases, 15 tags, plus 10 graph cases — all three re-counted
2026-08-26, unchanged. The `[web]` section is live in `~/.mecha/config.toml` —
safe now that every installed binary parses it; the outage its early
arrival caused is in HISTORY under Traps → Environment. A `llama-voxtral`
unit still exists at the *system* level (voice arc's; healthy per their
2026-08-24 check, no longer the STT seat) — query it with plain
`systemctl`, not `--user`, or it misreads as inactive.

**2026-08-29 (00:25 UTC) — deployed at `main` = `e8cd549` (#111 + #112),
verified by capability, not version.** `mecha` reinstalled from a worktree
pinned to that merge (the main checkout was a peer's mid-merge feature
branch and was left untouched); web assets rebuilt and rsynced; all five
long-running units restarted and each verified by its startup line. The
workspace version did not bump, so verification asked the artifact:
`sessions appraise --json` carries `sessions_unreadable`/`named_a_goal`,
`stats --json` is the new `{rows, sessions_unreadable}` object, `mecha
tools --json` shows the closure-guard note on `kg_task_update` — and
`mecha-serve` starting at all proves `closure_guard::verify` accepted the
model-facing registry. The stale-process sweep found nothing. Note for the
next reader: `mecha sessions stats --json` **changed shape** (bare array →
object); no in-repo consumer read the array, and the CLI reference records
the change.

**2026-08-29 (01:45 UTC) — v0.1.16 released, and every update surface
verified in one pass.** The tag published the four crates (crates.io
answers 0.1.16) and — first time ever — the workflow created the GitHub
Release itself: release.yml had never been given that job (fourteen tags
published crates with no release; `contents: read` could not have made
one), and the new `github-release` job builds it from the tag's changelog
section. Binaries reinstalled (`mecha --version` → 0.1.16, and this time
the version string is evidence), all five services restarted with verified
startup lines, benchmark musl rebuilt (was Aug 23 / 0.1.12 — a scorecard
would have measured six-day-old code), graph nightly binary current (that
lane's own install, same night), factory client and droplet both at 0.2.7,
sandbox toolchain matches host, stale sweep clean.

**2026-08-29 (03:30 UTC) — the night's four merges (#114–#117) deployed as
they landed, and the running services match `main` at `12d7f4b`.** Three
sessions worked this concurrently; every claim below was verified against
an artifact by a second session before being written here.
`mecha-serve` was restarted three times from the #114 branch (01:48, 02:05,
02:16 final), each with `~/.mecha/web/dist` rebuilt. The 01:48 and 02:05
dists served a **broken `/tasks` page** — v0.1.16's `stalled`
field-called-as-function bug (Traps → Measuring in HISTORY) — proven from
the served bytes both ways: `grep "stalled(" ~/.mecha/web/dist/assets/*.js`
hit the free identifier in the 02:05 bundle (an undefined global survives
minification under its own name) and comes back empty since 02:16. The
deployed binary and dist were built from the branch at `9c2f59c` +
`aa53174`; after the merges that differs from `main` by zero code, so **a
deploy from `main` is safe again** — the roll-off caveat that stood from
01:48 to the merge is retired. `mecha-slack` was deliberately **not**
restarted (pre-#114 inode; nothing it serves changed) at the time — then at
~04:16 the update skill's sweep found it *and* `mecha-triggers` still
executing deleted binaries (including the `mecha-graph-mcp` child) and both
were restarted; the sweep is clean since, and the web dist was redeployed
once more the same pass. The private graph repo's main moved twice
(`378ab8d` shadow list/show verbs, `bdce6c2` `surfaced_total` in the shadow
envelope) and both graph binaries were reinstalled — verified by asking the
artifact (`mecha-graph shadow --help` answers with the surfaced-verdict
queue). The docs site deployed twice, each verified past its green tick:
#115's by the workflow run, #117's by fetching the published pages — the
demo app renders in its iframe and a scripted chat run completes against
the published bundle. Branch state: `worktree-shadow-queue-surfaces` still
exists on the remote (its session holds the worktree; left for cleanup
after that session ends); `fix/tasks-stalled-field`, `docs/appraisal-page-
rework` and `docs/web-surface-demo` are merged and deleted.

**2026-08-30 (~12:45 UTC) — deployed at `main` = `6987bc5` (#124 + #120 +
#123), every claim verified by capability.** `mecha` reinstalled from this
checkout (pulled to the merge first) and proven current by asking the
artifact — `mecha rules list --json` answers with the new `graded` field —
because the version string reads 0.1.16 on both sides of the install and
proves nothing. Web dist rebuilt from `main` and rsynced to
`~/.mecha/web/dist`; the `:63242` door serves the new bundle hash. That
rsync *superseded* the graph-tab lane's `deployed-local` test build
(`67022e5`) — checked first, found to be an ancestor of the merge, so
nothing unmerged was reverted — and the tag is deleted, per the update
skill. All five long-running units restarted and each verified by its
startup line (Slack reconnected with 1 owner / 16 threads; triggers
ticking; both serve doors; Uvicorn on 7860); the stale-process sweep found
nothing. The benchmark musl binary was rebuilt the same hour
(`bench/build-portable.sh`, dated Aug 30, answers `graded` too), so
scorecards measure current code. Deliberately untouched, each for a
verified reason: `mecha-mail` binaries (no diff in `4c7a0e2..6987bc5`),
both graph binaries (different repo, no movement), `mecha-parakeet` (its
script unchanged; a restart costs a model load), the factory client and
droplet (different version line, not part of this deploy).

**2026-08-30 (~23:30 UTC) — both repos merged and the whole machine on
`main`, every claim verified by an artifact.** mecha #125 (`68b4af9`) and
#126 (`ab0097b`) merged; the private graph repo merged its first PR
(`d44e04d`: `kg_upsert` alias `remove`, `proposals file-merge`,
`identifiers` in `kg_entity`) plus the fictional-cast strip (`20f3d38`).
`mecha`, `mecha-graph` and `mecha-graph-mcp` all installed from the two
mains; the MCP server answers **13 tools** with `remove` in `kg_upsert`'s
schema (asked, not inferred). The graph nightly's own binary
(`target/release/mecha-graph`) rebuilt from main at 23:28, and the
private checkout is back on `main` — the 01:30 audit runs merged code.
Web dist rebuilt from main and rsynced; **`deployed-local` deleted**, so
its absence honestly means "main is deployed" again. All five long-running
units restarted 23:29 and verified by their own startup lines (serve's two
doors, slack "Connected … 1 owner(s)", triggers "ticking every minute",
drain started, voice-worker's Uvicorn); stale-process sweep clean. The
**public mirror published**: `github.com/ljchang/mecha-graph` at
`bbbba2a` ("0.1.3"), through `export-public.sh` and a **clean** denylist
gate — the first export attempt was refused on 13 files of life-derived
fixture names, which is the strip commit above and a trap in HISTORY.
llama-server re-verified: `total_slots=4`, `n_ctx` 262,144/slot,
`model_alias qwen3.6-35b-a3b`. Eval: 36 cases, 15 tags, re-counted this
date, unchanged. Workspace tests 1,988/0 (the prose figure above).
`mecha-mail` binaries, benchmark musl, factory client/droplet, and the
sandbox image deliberately untouched (no diff on their surfaces this
session).

**2026-08-26 (00:12 UTC) — installed and restarted again, and one commit
of deliberate skew.** `mecha` reinstalled and all five long-running units
restarted during the day's work, each verified by its own startup line;
`mecha-graph-mcp` reinstalled from the private repo (12 tools, answered from
`~/.cargo/bin`) after `kg_notes` gained `source_id`; web assets rebuilt and
rsynced to `~/.mecha/web/dist`, confirmed by the hash the `:8443` door
serves. The stale-process sweep ran twice and found two the first time
(`mecha trigger daemon` and `mecha slack connect`, both cleared by restart)
and none the second.

**The running binary is one commit behind `main` on purpose.** The last
commit (`874eb6a`) boxes an error type to satisfy a clippy lint the CI
toolchain has and this box's does not; it is behaviourally identical, and a
second session was live in this checkout with uncommitted work in
`serve/chat.rs`, `serve/present.rs` and `Chat.svelte`. Reinstall and restart
`mecha-serve` next time the tree is not shared. **`docs/OPERATIONS.md` gained
the voice worker's real `--allowed-origins`**, which the tracked unit no
longer carries — anything reinstalling that unit from the repo copy has to
substitute them back.

**2026-08-26 — the live graph store was repaired in place, and no repository
records it.** `~/.mecha-graph/graph.db` is not in any git history, so this
paragraph is the only account of it. `mecha-graph repair-id-payloads` merged
**30** placeholder topic nodes into the nodes they were named after, rewrote
**121 of 8,988** pending candidate payloads from node ids to names, and
re-pointed **23 accepted facts** at real entities. The merges are the
irreversible part (`merge_nodes` has no clean undo) and were run on the
owner's explicit ruling, declining a backup first. A second `--dry-run`
reports 0/0/0, so the repair is idempotent and re-running it is the check.
What it does *not* undo: those 23 facts were accepted by a human at a time
when their subject stood for nothing, and they now say what their own
statement always said — a repair, and also a belief changing what it is
about.

**2026-08-25 (13:20) — everything installed and restarted at v0.1.14, and
the version skew that stood earlier today is closed.** `mecha --version`
reports **0.1.14** matching the tag; `mecha-mail` and its three sibling
binaries reinstalled at the same version; `mecha-graph` and
`mecha-graph-mcp` both reinstalled from the private repo at 0.1.3 —
**two crates, two installs**, and the MCP one is the only binary mecha
reaches at runtime (12 tools, answered from `~/.cargo/bin`). Five
long-running units restarted (`mecha-slack`, `mecha-triggers`,
`mecha-drain`, `mecha-serve`, `mecha-voice-worker`), each verified by its
own startup line rather than by `is-active`. Web assets rebuilt into
`~/.mecha/web/dist` and confirmed by the hash the `:8443` door actually
serves. The stale-process sweep found none. `factory-publish` and the
droplet are both current at 0.2.7 — checked, not assumed — and the
`mecha-sandbox` image's cargo matches the host's 1.97.1, so neither needed
touching.

Earlier the same day this paragraph recorded the opposite state, and the
reason is worth keeping: the installed binary was built from main while
the workspace version still said 0.1.13, so `mecha --version` reported a
number the bytes had outgrown — the "a version string is not evidence"
trap arriving through the door that looks most like evidence. Cutting
0.1.14 is what made the version line true again, not a fix to the check.

What has landed on main since the tag, beyond the `--json` flags and the
`/queues` review level: **`/entity` merges two nodes you pick yourself**
(`m` marks the survivor, `m` again confirms with `y` — the only key on that
modal that confirms, because it is the only irreversible one; since
2026-08-30 the web entity page has the same gesture through
`proposals file-merge --accept`, which additionally records a decided
proposal — the TUI path still merges bare), **Esc peels
one layer at a time there** (merge → edit → search → modal), and **every
`/queues` row reports how long its oldest item has waited** beside its depth.
That last closed a real gap: four outbox drafts sat five days behind a row
reading `6`, visible only to `mecha doctor`. Depth is a snapshot; the surface
built because a queue reached 6,434 items unnoticed could not show a queue
growing.

**`mecha-slack` and `mecha-triggers` are on a deleted inode, and that is
intended.** Both read `(deleted)` on `/proc/<pid>/exe`; nothing landed today
reaches either, so the `update` skill's stale-process sweep will flag two
false positives. Restart them on the next change that actually reaches them,
not because the sweep says so. `mecha-serve` **was** in that set and is not
any more (pid 2626147, live inode): the outbox source join and the mail
resolver both run inside it, so it earned the bounce that the earlier changes
did not. The rule this pass confirmed twice: only `mecha-serve` shells out
through `/proc/self/exe`, and the `mail-classify` timer needs no restart at
all because it spawns a fresh `mecha` per run and picks up an install the
moment it lands.

**`:443` now serves the app, and the standalone voice page is gone.**
`tailscale serve` proxies both `:443` and `:8443` to `127.0.0.1:63242`, so the
two doors are the same surface; `scripts/voice/page/` was deleted and
`voice-core.js` moved up to `scripts/voice/`. Verified live this date from
`tailscale serve status`. The docs site was rebuilt but **not deployed**, so
the live site served "two places to talk from" until it was pushed. **Deployed
and verified this date** by fetching
<https://docs.mecha-factory.ai/docs/features/voice> rather than by reading the
workflow's green check — the phrase is gone and the app-only wording is
serving.

**The worktree convention has no retirement policy, and it cost 85 GB.**
Fourteen worktrees had accumulated under `.claude/worktrees/` and various
session scratchpads — essentially **one `target/` each** (17 GB in
`tasks-arc`, 11.7 in `vision-arc`) against roughly 100 MB of combined
source. The convention that produced them is right and is what let three
sessions work in parallel on 2026-08-25; what it lacks is any statement of
**when a worktree stops earning its disk**. Note the shape is already solved
one directory over: `[work] keep` answers exactly that question for
generated output, with `mecha work clean` as the policy and the nightly as
the trigger — which is why `~/.mecha/work` stayed at 56 MB while the
worktrees grew unwatched. `CARGO_TARGET_DIR` pointed at one shared directory
is the structural fix if it recurs; a retention verb is the policy one.

**And clearing them nearly destroyed the only copy of the raw benchmark
trials.** `results/` in main tracks the Terminal-Bench *scorecards*; nothing
tracked the per-trial output, and 1,627 files of `jobs/` — `config.json`,
`result.json`, per-trial `trial.log` from 2026-08-11 — lived only in the
`bench-run-*` worktrees, gitignored. Archived to
`~/.mecha/archive/bench-jobs-2026-08-11.tar.gz` (4.4 MB, file count verified
against disk) before removal. The near-miss is instructive because the rule
was already written down: HISTORY's "a gitignored file in a disposable
worktree dies with the worktree" entry predates this by weeks, and the
check that caught it was counting files on disk rather than trusting a
survey of tracked content.

| Port | Model | State |
|---|---|---|
| 8080 | Qwen3.6-35B-A3B | up, **`total_slots=4`**, `-c 1048576` → **262,144 per slot**, and **`--mmproj` loaded since 2026-08-21 so `modalities.vision` is true** — the per-slot figure is the model's whole trained window (`qwen35moe.context_length`), raised from 32768 on 2026-08-10 after re-measuring. **`-c` costs nothing in speed**: 32k/64k/128k/256k are within noise of each other (~92 tok/s at a 1k prompt, ~80 at 30k), and the 50x slowdown recorded on 2026-08-07 was that day's OOM, not the flag. It costs memory as a startup *reservation* — 21.4 GB at 32k to 28.5 GB at 256k, i.e. weights ~20.7 GB plus ~32 KiB/token. **The full tables, the needle test at 188k, the `-np` trade-off and the two traps live in `scripts/start-moe-mtp.sh`** — read it before touching any of this. **`--reasoning-budget 4096`** (2026-08-07) was believed to be the mitigation for this model's "non-terminating reasoning" — **that diagnosis was wrong and is retired as of 2026-08-10 evening**: the empty turns were tool calls emitted before `</think>` closed, one of them 120 characters long, so no token budget was ever involved. The flag is harmless and stays; the real cause and fix are in `CHANGELOG.md` under 0.1.2. The nudge-retry allowance still resets on productive turns, which remains correct for its own reasons. `~/.mecha/config.toml` and `bench/mecha_agent.py` carry `context_window` and `max_tokens` (**above** the budget; 8192) — four numbers that move together. **`context_window` is `-c / -np`, not `-c`** — llama-server divides the context across slots, so the rule this line used to state was right only by accident of `-np 1`. Read it off `/props` (`default_generation_settings.n_ctx`) or the startup line's `n_ctx_slot`, never by arithmetic on the flag. **A vision model is two files.** The weights carry the language model and the vision tower is a separate `mmproj-*.gguf` that `--mmproj` must name; without it the server starts, answers well, reports `modalities.vision: false`, and the model tells anyone who sends it a screenshot that it cannot see images — which reads as a limitation of the weights. `scripts/mmproj.sh` now refuses to start without one. MoE 3B active, in-GGUF MTP (`--spec-type draft-mtp`, no `-md`). **A transient unit** — now `llama-local.service` (`systemctl --user status llama-local`; it was `llama-qwen` when this was written, and that name no longer resolves), not a tmux pane — see below |
| 8081 | harrier-oss-v1-0.6b | **up, serving embeddings** (`--embeddings --pooling last --embd-normalize 2`). This is where the graph's embeddings come from — they moved off Ollama onto llama-server, so any doc still naming `MECHA_GRAPH_OLLAMA_URL` is stale. One model per process, so this cannot be the chat port as well: pointing both at 8080 sends embedding requests to the chat model. |
| 8083 | Qwen3.8-27B | **down as of 2026-08-20** (was up on 2026-08-16). Nothing in config depends on it, so nothing is broken by it — noted because the previous pass recorded it up and a reader would otherwise assume it still is |
| 8082 | gemma-4-26B-A4B | **down — restart it before any judged run.** The eval judge and nightly validate's judge both point here, so `mecha eval` with a `judge` rubric and the nightly validate will fail without it. `scripts/start-gemma26.sh` |
| 8888 | SearXNG | up (docker, JSON format enabled) — **but every *general* engine was refusing this IP on 2026-08-21**: brave and google cse `Suspended: too many requests`, duckduckgo and startpage `CAPTCHA`, mojeek `access denied`. The specialist engines (lib.rs, crossref, arxiv, openalex, stackoverflow) answer fine. Partially recovered the same afternoon. This is why Exa and Tavily were added — a scraping metasearch loses the anti-bot race, and the answer is a backend contractually entitled to the data, not a better scraper |

**Start model servers as transient units, not from a tmux pane.** Both
llama-servers were killed on 2026-08-07 as collateral: a runaway test OOMed,
and systemd then tore down the whole `tmux-spawn-*.scope` they happened to
share (`OOMPolicy=stop`). 8080 was brought back with
`systemd-run --user --unit=llama-qwen scripts/start-moe-mtp.sh`, which puts it
outside any pane's cgroup. 8082 has not been restarted.

**`-np 1` is load-bearing**, and the check before believing any measurement is
`curl :8080/props | jq .total_slots` — it must be 1. The build in use defaults
to 4 parallel slots and silently splits `-c` across them; the story of what
that cost is in [`HISTORY.md`](HISTORY.md) under Traps → Environment.

Start scripts are in `scripts/` (`start-moe-mtp.sh`, `start-e4b.sh`,
`start-gemma26.sh`); they resolve the model through `$HOME` with `HF_HUB` and
`LLAMA_SERVER` overrides. Config is `~/.mecha/config.toml` (providers `local`,
`small`, `gemma26`, `anthropic`).

**A release key sits on an agent machine, and that is against the rule the
factory wrote down.** `~/.mecha/factory/release.key` exists here (mode 0600,
2026-08-07), on the same box that runs `mecha-slack.service` and
`mecha-triggers.service` — both active. `mecha-factory/docs/SECOND-CLIENT.md:58`
says "Keep release keys off machines agents use", and the reason is the
ceiling: a paired agent machine's intended worst case is *immutable versions
nobody can read*, and a release key removes that bound. Under mecha the
publish path is outbox-routed so a human still gates each release, which is
mitigation and not the property the rule protects — an unattended run on this
box is one confused-deputy step from a capability the design says it should not
be able to reach. **Surfaced, not acted on**: moving the key is a decision
about how this machine is operated, and it belongs to whoever operates it.
(Verified 2026-08-20.)

**The KV cache is f16, not q8_0 — on either server.** No `-ctk`/`-ctv` on
mecha's llama-server and no `LLAMA_ARG_CACHE_TYPE_*` in its environment; and
`systemctl show ollama.service -p Environment` carries `PATH` and nothing else,
so `OLLAMA_KV_CACHE_TYPE` is unset there too (all verified 2026-08-20). Nothing
in this repository ever claimed otherwise.

It is written down anyway because **the belief has a live source on disk and
will regenerate.** `~/Github/dgx-spark-playbooks/nvidia/txt2kg/README.md:153` —
NVIDIA's own tuning playbook for this hardware — recommends
`OLLAMA_KV_CACHE_TYPE=q8_0` in a *troubleshooting* table ("Ollama performance
issues → Suboptimal settings for DGX Spark"), and `assets/README.md:99` states
"Q8_0 KV cache for memory efficiency" flatly. Both are advice for **ollama**,
not for mecha's llama-server, and neither was ever applied. A recommendation
read as a description is how a machine acquires a property nobody gave it; the
playbook is still there and still says it, so the next reader will believe it
again unless this measurement is beside it.

**Restarting a model server under memory pressure is permanent for that
server's life.** `scripts/start-moe-mtp.sh` records the finding; what it means
operationally is that the `update` sequence must not restart
`llama-local.service` while anything heavy is resident — a second llama-server
under `mecha-graph extract`, say — because a server that loads under contention
stays slow and never recovers. Check tok/s afterwards, not just that the unit
came back up. Liveness is the check that cannot see this failure.

**Web search is three backends now, and none of it is in any repository.**
As of 2026-08-21 `~/.mecha/config.toml` carries three `[[search]]` blocks in
preference order — `searxng` (`base_url = "http://127.0.0.1:8888"`), `exa`
(`api_key_env = "EXA_API_KEY"`, `prefer_deep = true`), `tavily`
(`api_key_env = "TAVILY_API_KEY"`, no `prefer_deep`, a pure fallback). Ordering
was measured rather than assumed: a quick Exa search bills **$0.007** against
Tavily's $0.008, read off Exa's own `costDollars` in the response, and Exa's
recurring free tier is the larger (~1,430/month against 1,000). The
`contents: {text: …}` block mecha sends adds **nothing** — text extracts are
bundled into a search call, though Exa's price list bills Contents separately
at $1/1k pages and predicts otherwise.

**The keys are in a third place that is also in no repository.**
`~/.config/environment.d/mecha.conf`, mode 0600, holding `EXA_API_KEY` and
`TAVILY_API_KEY`. That path rather than `~/.bashrc` because **only
`environment.d` is read by the systemd user manager** — and unattended search
happens inside `mecha-slack` and `mecha-triggers`, so a key exported from
`.bashrc` works perfectly when tested by hand and is invisible to every run
that matters. `.bashrc` now *sources* that file instead of duplicating the
value, so one edit reaches both; non-interactive shells still miss it, which
is why the units read `environment.d` directly. Verified by reading
`/proc/<pid>/environ` of each service, never by assuming the restart inherited
it. `ANTHROPIC_API_KEY` is still `.bashrc`-only and has therefore never been
visible to any unit — harmless today only because every unit runs
`provider = "local"`.

**Two live config changes exist in no repository.** `~/.mecha/` is not a git
repo, so `config.toml` is on one disk and in no history. As of 2026-08-20 it
carries a `[[mcp]] name = "docs"` block running `~/.cargo/bin/mecha-docs`, and
six `docs__*` names in `[outbox] tools` (`docs_create`, `docs_append`,
`docs_replace`, `sheets_create`, `sheets_write`, `slides_create`) — verified
live on the surface after the 0.1.9 install: writes staged, reads read-only,
and `docs_trash` deliberately unrouted because it reaches nobody. A fresh clone
plus a fresh `~/.mecha` would have the binary and none of this wiring.

### Standing machinery on this machine

**Publishing a release changes nothing about a running system.** The installed
binary is `~/.cargo/bin/mecha`, from crates.io, and every long-lived unit keeps
whatever it started with. After tagging v0.1.2 the box was still executing
0.1.1 everywhere — including `mecha-triggers`, which would have fired that
night's scheduled runs on the pre-fix harness, and `mecha-ruminate` at 03:30,
which mines sessions into learned rules that ride in every future prompt.
Use the three-crate line below rather than upgrading `mecha-cli` alone —
`mecha-mail` ships the MCP server binaries and a run spawns them fresh from
whatever is installed, so a partial upgrade leaves the mail surface a version
behind with nothing to say so. Done 2026-08-10 evening
(`cargo install mecha-cli mecha-mail --locked`, both at 0.1.2), then
`systemctl --user restart mecha-slack mecha-drain mecha-triggers` — verified,
and the connector reconnected with its 12 threads intact at 14:28. The timer-driven units — `mecha-slots`, `mecha-frontdoor`,
`mecha-ruminate` — are oneshot and exec a fresh `mecha` each fire, so they need
no restart; restarting them would be cargo-culting. The set that needs action
is exactly the set holding a long-lived process.

- **Two live changes on 2026-08-19 that exist in no repository**, both under
  `~/.mecha/`, which is not a git checkout. The `morning` trigger's `notify`
  line now appends day two to the briefing by shelling out to
  `mecha mail list --aged --surface`, so **the installed binary must carry
  `--aged` before 07:00 or the briefing pastes a clap usage error into
  itself** — caught on the day by running the hook rather than reading it. And
  `llama-local.service` is new (below). A fresh clone has neither.
- **The local model server is `llama-local.service`** (systemd user, enabled,
  `scripts/start-moe-mtp.sh`, qwen3.6-35b-a3b on 127.0.0.1:8080). **It became a
  unit on 2026-08-19 and the reason generalises.** Before that it was only ever
  started as a transient unit, so a reboot restored every *consumer* —
  `mecha-triggers`, `mecha-slack`, `mecha-drain`, `mecha-mail-classify` are all
  persistent and enabled — and none of the thing they consume. The machine
  rebooted at 02:39 that morning and for nine hours every agent run failed with
  a connection error while systemd reported a healthy system. A reboot does not
  degrade this box evenly: it brings back exactly the half that generates load.
  `Restart=on-failure`, deliberately not `always` — a server that cannot load
  should stay visibly down rather than loop.
  **After restarting it, measure tokens/sec and not merely that it answered**:
  `start-moe-mtp.sh` records that a server which loads while memory is
  contended stays slow for its whole life and never recovers. 100.5 tok/s on a
  short prompt is a healthy load; ~82 is the degraded one.
- **Reflect-on-close**: `~/.mecha/config.toml` carries a `session_end` hook
  running `nohup mecha reflect -p local ... &` — every recorded session is mined
  minutes after it closes.
- **Nightly rumination**: `mecha-ruminate.timer` (systemd user, 03:30,
  `Persistent=true`, linger on) runs `scripts/ruminate.sh`: reflect → distill →
  validate `--unprocessed-only` (judge: gemma26) → learn
  `--holdout 0.25 --propose` → `rules propose-retirements` → `work clean` →
  `harness ruminate --sessions 16` (added 2026-08-22: the self-improvement
  pass — see that section). Logs land in
  `~/.mecha/learning/logs/<date>.log`; pending proposals wait in
  `mecha proposals`, harness candidates in `mecha harness list`.
  **Confirmed enabled 2026-08-05; unit `ExecStart` re-verified 2026-08-22.**
- **The trigger daemon is installed and running.** `mecha-triggers.service`
  (systemd user, linger on), enabled 2026-08-06 and confirmed firing on its own.
  `morning.toml` is jailed to `~/.mecha/work/morning`, and its `notify` writes
  the briefing then renders it. A failed `notify` is recorded on the run, so a
  briefing that has quietly stopped rendering does not read as a healthy one.
- **Booking slots refresh every two minutes, and the drain is a live loop.**
  `mecha-slots.timer` (retimed from fifteen minutes, 2026-08-08 evening —
  the calendar→box window was the one real double-booking risk) → freebusy
  from the named account → `factory-publish slots push`, with the
  booking-drain sweep riding the same tick as backstop. The fast path is
  **`mecha-drain.service`**, an always-on loop long-polling the box's queue
  (`drain --wait 25`) so a confirmed booking becomes a calendar event and
  its invite seconds after the click; live end to end since the box's
  v0.1.0 deploy (2026-08-08, late evening — an empty `?wait=4` poll against
  the gate held the full four seconds). The sweep takes a flock (both
  runners share it) and re-verifies each booking against live freebusy
  before creating the event; a conflict parks loudly in the ledger. The
  account is named in the units because a timer cannot ask; the loop script
  runs from `~/.cargo/bin/mecha-drain-follow`, installed like the binaries
  it drives.
- **The front door runs hourly.** `mecha-frontdoor.timer` →
  `scripts/frontdoor.sh`: drain (zero tokens, runs even with the model down) →
  extract → triage, logging to `~/.mecha/requests/logs/`. Enabled 2026-08-07
  and verified with a live tick the same hour — drain acknowledged a held
  record at the gate, and the one flagged request stayed parked for a human,
  which is the quarantine working. Triage refuses to run unrouted, so the
  `[outbox] tools` list is load-bearing for it.
- **The automation consumes `~/.cargo/bin/`, not the repo build.** Reinstall
  (`cp target/release/mecha ~/.cargo/bin/`) after changing anything in the
  learning path, and `factory-publish` too, since `morning.toml`'s `notify`
  calls it. This bit twice on 2026-08-06: once when `ruminate.sh` gained a
  `work clean` step the installed binary did not have, 41 minutes before the
  nightly, and once when a `factory-publish` fix sat unbuilt in the repo for
  nine hours, and a third time the same evening when the daemon's own `notify`
  could not see `~/.cargo/bin` at all. A fourth near-miss on 2026-08-07: the
  installed `mecha` predated the frontdoor triage verbs, caught only because
  the timer's script was checked against it before enabling.

  **Since 2026-08-10 the installs come from crates.io, not from the repo**,
  which is the better habit now that releases are tagged: it installs exactly
  what the world gets, so a broken package is found here rather than by a
  stranger.

  ```bash
  systemctl --user stop mecha-slack mecha-drain mecha-triggers
  cargo install mecha-cli mecha-mail mecha-factory-publish --locked
  systemctl --user start mecha-slack mecha-drain mecha-triggers
  ```

  Currently installed (2026-08-10 evening): `mecha-cli` 0.1.2, `mecha-mail` 0.1.2,
  `mecha-factory-publish` 0.2.1. `mecha.prev` / `factory-publish.prev` remain
  as the pre-crates.io rollback copies.

  Three things learned doing it, none specific to this change:

  - **`cargo install` can resolve a version older than the one just
    published**, because the local registry index is cached — it fetched
    `mecha-factory-publish` 0.2.0 and `mecha-mail` 0.1.0 minutes after 0.2.1
    and 0.1.1 went up. Check `cargo install --list` against the intended
    versions afterwards, or pass `--version` explicitly. Nothing warns.
  - **Stop the services first.** `cp` over a running binary fails with `Text
    file busy`, and two consumers hold `factory-publish` at all times:
    `mecha-drain.service`'s long-poll loop, and the MCP server the Slack
    connector spawned. (`cargo install` renames rather than copies, so it
    usually survives, but the mail binaries are held by whatever MCP servers
    are alive.)
  - **A long-lived MCP server keeps the tool surface it started with.**
    `mecha-slack.service` served the old seven-tool factory surface until it
    was restarted; a fresh `mecha chat` / `tui` / `run` spawns a new server and
    sees all fifteen immediately. It has now been restarted, though
    `[slack] tools` was emptied the same day, so a Slack thread now carries the
    same surface as `chat` and `tui` — including `mail__*` and the graph.
- The learning store (`~/.mecha/learning`) holds **zero live rules** — the one
  early rule was reverted with its poisoned reflection — so everything from here
  accumulates from real usage through the gate.

### The public box

A single small VPS runs `mecha-factory`. Its deployment posture — host, sizing,
the dedicated deploy key, the firewall policy — is inventory rather than
engineering, so it lives in `docs/OPERATIONS.md`, which is gitignored. The
deploy procedure and its traps are in that repository's `docs/DEPLOY.md` —
and as of 2026-08-08 a deploy is one command: `factory-deploy <tag>` on the
box installs a tagged, CI-built, checksum-verified static binary, proves the
binary and the config while the site is still up, and rolls itself back on a
failed health check. Hand-copied binaries are over; the outage that ended
them is in [`HISTORY.md`](HISTORY.md).

What is worth stating publicly is the invariant the posture exists to protect:
**the box holds no credential that reaches home.** Two Argon2id key hashes, the
published bytes, and a certificate. Packets go one way — mecha publishes and
drains, and the origin never dials home. That is a property you can verify by
inspection, which is why it is the one written down here.

### Provider credentials

Where the keys live is in `docs/OPERATIONS.md`. Two gotchas from wiring a key
through a shell profile, both of which cost time and neither of which is
specific to this machine:

- **`~/.bashrc` returns early for non-interactive shells** (the `case $- in *i*)`
  guard near the top). An export below that line is invisible to a
  non-interactive shell, which is what tooling runs — so a key that works when
  you type a command by hand vanishes under systemd or a hook.
- **Take the *last* match, not the first**, when grepping a profile for an
  export. A placeholder above the real key meant `grep -m1` silently found the
  placeholder and produced a `401 invalid x-api-key` that looked like a bad key.

**Mail OAuth grants, re-consented 2026-08-18** (both were re-issued that day
because the triage scopes widened, and both are recorded in each account's
`oauth.json` under `granted_scopes`):

| Account | Provider | Grant | Expiry |
|---|---|---|---|
| `personal` | Google | `gmail.modify`, `gmail.send`, `calendar`, `calendar.events` | **7 days from consent — next ≈2026-08-25** |
| `dartmouth` | Outlook | `Mail.ReadWrite`, `Mail.Read`, `Mail.Send`, `Calendars.ReadWrite` | none — permanent |

The Google client (Cloud project **FlowMail**, the same registration the old
app used) is in **Testing** publishing status with User type External, and
Google expires a Testing app's refresh token exactly 7 days after consent —
refreshing does not extend it. Moving to production would fix that but needs
verification plus a CASA security assessment (~$540/yr), because
`gmail.modify` is a restricted scope. **Decided 2026-08-18: stay in Testing,
and revisit CASA once the main development features are done** — so this is
deferred rather than open, and should not be re-litigated as though it were
undecided.

Two things to carry into that revisit, both measured on 2026-08-18 by the
parallel documents work (`docs/DOCS-RESEARCH.md` §6.2): the console
distinguishes **brand verification** from **scope verification** and only the
second is the expensive one — a banner reading "your app requires
verification" appeared on a project with no sensitive or restricted scopes at
all and turned out to be branding, which blocks nothing and must not be
answered by submitting for review. Check the Verification Center's two cards,
never the banner. And a Google grant is per (user, client), not per scope, so
anything added to the FlowMail client shares mail's fate in both directions —
which is why the documents work took its own project rather than this one.

Meanwhile
`~/.mecha/mail/accounts.toml` declares `grant_lifetime_days = 7` on `personal`
so `mecha doctor` warns two days out; that file is in no git repository, so a
fresh clone will not have it.

Dartmouth's Entra registration (also named FlowMail, client
`bc6a1e19-…`) already had `Mail.ReadWrite` **Delegated** granted tenant-wide,
so no ITC request was needed — the opposite of what was expected.

**No prices are configured**, so `cost_usd` is `None` and `--max-cost` silently
never fires on a paid provider. For `[providers.anthropic]`:

```toml
input_price_per_mtok = 5.0     # claude-opus-5
output_price_per_mtok = 25.0
```

Sonnet 5 is $3/$15 ($2/$10 introductory through 2026-08-31). We send
`cache_control` with no `ttl`, so the 5-minute tier applies and mecha's default
multipliers (1.25 write, 0.1 read) are already correct.

### Machine state, dated

Deploy and environment entries, newest last. They had been accreting under
`### Provider credentials` above, which is not what they are.

**What is on the box is not main, and `deployed-local` is how you find out
(2026-08-29).** Sessions deploy merged draft branches here for the owner to
live-test, so `~/.mecha/web/dist` routinely holds a build of something that
exists in no branch you can see. The tag `git tag -l deployed-local` in the
main checkout names the commit whose build is installed whenever that is not
main; it is re-pointed on each such deploy and deleted when main is deployed.
**Read it before any `rsync` into that directory, and announce before
replacing a dist you did not build** — the update skill's step 1b carries the
rule and the incident that produced it. The value moves fast: three deploys
landed between 13:53 and 14:26 on 2026-08-29 alone, so treat any commit
written down here as a sample, not the state. Verified at **14:29 UTC**:
`deployed-local` → `1506e32`, a merge of main (`4e282e3`, #118) with all three
then-open PRs — `feat/graph-tab` (#120), `fix/web-cache-headers` (#121) and
`worktree-shadow-queue-surfaces` (#119) — with the `:63242` door returning 200
for `index-0ZAqQKph.js`. No binary changed for #118 and no unit was restarted
for it: `mecha serve` reads assets per request through `ServeDir`, so an asset
deploy needs no restart and a *binary* change still does.

**2026-09-02 (~10:30, after the repo move and a full update)** — the graph
stopped being two repositories. `~/Github/mecha-graph` is now the working
checkout and the private `personalized_knowledge_graph` keeps only notes
(HANDOFF, RESEARCH_LOOP, OPERATIONS) and the roster tooling. What made that
safe was moving the gate rather than dropping it: the public repo carries
`.githooks/pre-push` and `.github/workflows/denylist.yml`, both reading the
roster from `~/.mecha-graph/denylist.txt` (0600) and the `PUBLIC_DENYLIST`
secret, both fail-closed on a missing or empty roster. Verified on four
negatives, including that it refused its own first commit — the comments
explaining the word-boundary rule quoted two roster terms.

Ten files were private; only four held roster terms. `nightly-mecha.sh` was
private *by association* (it sat in `scripts/` beside the gold-set tooling)
and was read line by line before publishing: no names, no emails, no
hostnames, every path `$HOME`- or `$BASH_SOURCE`-relative, one endpoint
`127.0.0.1:8080`. The gold sets moved to `~/.mecha-graph/eval/` (0600) and
`eval::gold_path_from` resolves them on `db::default_db_path`'s shape;
`.gitignore` gained `eval/*gold*.jsonl` as the belt, tested by copying a real
set in and confirming `git add -A` refuses it.

**Installed state, verified by asking the artifacts.** `mecha --version` →
**0.1.17** from `~/.cargo/bin`, and `strings` on it contains both
`never needed, NOT` (the diagnostician brief fix) and `candidate-arm` (the
arm split) — capability, not version string. The graph binaries now install
from the *public* checkout and cargo said so itself: `Replaced package
mecha-graph v0.1.4 (personalized_knowledge_graph) with … (mecha-graph)`.
`mecha-graph-mcp` answers 13 tools from the installed path. The 08:00 crontab
runs `~/Github/mecha-graph/scripts/nightly-mecha.sh`; the 01:30 line still
runs the private `nightly.sh`. Benchmark musl was **stale at 0.1.16** and was
rebuilt to 0.1.17, statically linked — a scorecard run before that would have
measured old code and labelled it current. Factory client 0.2.8 = newest tag;
droplet read-only check `factory 0.2.8`, `active`, untouched. Sandbox image
cargo 1.97.1 matches host exactly. Stale-process sweep clean. All five
long-running units restarted with their own startup lines (slack "Connected
to cosanlab as mecha. 1 owner(s), 16 thread(s)", triggers "1 trigger(s), 1
enabled", serve's two doors, voice-worker's Uvicorn). `~/.mecha/web/dist` was
**not** rsynced: `deployed-local` named `a8e0629`, which is now an ancestor
of main, and `git log -1 -- web/` returns that same commit — so the served
bundle already matches what main builds. `deployed-local` still points there
and is worth deleting now that main is deployed.

**`mecha --version` no longer distinguishes anything on this box.** It reports
0.1.17 and the workspace says 0.1.17, but HEAD is **13 commits past the
`v0.1.17` tag**, so every build since carries the same string. Probe for a
literal instead: `strings ~/.cargo/bin/mecha | grep -c` on
`You name conversations` (the titler, `mecha_core::title`),
`left the recording; no reason recorded` (arm-attributed divergence), or
`never needed, NOT` (the diagnostician brief). All three are present as of
this date. A peer independently reached the same conclusion from the other
direction — rebuilding `web/dist` from main and getting a byte-identical
`index-klt_h5v8.js` — which is why the dist was not rsynced.

**The desktop web layout and conversation auto-naming are deployed but
UNTESTED IN ANGER.** The bundle was built, served and verified by hash, and
the PR merged, but nobody exercised the left-rail layout at 900px, the
session drawer at 1180px, or a rename. Recorded as deployed-and-unexercised
rather than verified, on its author's own flag — the distinction matters
because "deployed" reads as "working" in a handoff and this one has no
evidence behind it yet.

**2026-09-02 (~12:30, the graph move completed)** — `personalized_knowledge_graph`
is no longer a code repo. Its PR #5 removed `eval/gold*.jsonl` and
`scripts/nightly-mecha.sh`; what remains is HANDOFF, RESEARCH_LOOP,
OPERATIONS and the roster tooling. mecha-graph#5 landed the input-set alarm's
memory, so the 54-a-night line becomes one self-naming line carrying the
backlog's age.

**The catch worth carrying: a repo move leaves behind every consumer that
names a PATH rather than a binary.** `nightly.sh`'s
`PKG="$REPO_DIR/target/release/mecha-graph"` still resolved into the retired
checkout, whose `target/` is frozen at Aug 31 — so the 01:30 decay would have
run a five-day-old binary and printed the same 54 lines, while
`mecha-graph --version`, `~/.cargo/bin`, the 08:00 cron and every other
surface read current. Nothing would have complained. Found by asking which
binary the cron executes, not by noticing a symptom. `grep -rn
"target/release" scripts/` is the sweep to run at a move, not after it.

Both cron lines verified end to end this date: 01:30 resolves `PKG` to the
public tree, 08:00 runs the public `nightly-mecha.sh` whose `$REPO_DIR` is
already public. The binaries on those paths carry the alarm memory, the Bee
idempotence fix, `--min-sources`, and the attempt-keyed cooldown;
`~/.cargo/bin/mecha` carries the pressure-headroom brief for 03:30.

Tests this date: **1132** core, 688 + 133 + 75 + 20 + 9 + 6 + 1 across the
other suites, 0 failed. Eval 36 cases, 15 tags, unchanged. `deployed-local`
was deleted by its owner once main was on every surface — its absence again
means "main is deployed".

**2026-09-03 (~10:50 UTC, the approval-rules arc deployed)** — the owner
asked for an update once the thirteen PR merges beyond v0.1.17 were in (`git
log --first-parent --merges v0.1.17..4a888ad`; `rev-list --count --merges`
says 18, counting merges inside feature branches too), and this is what each
of the `update` skill's surfaces was found to be and left as. **Installed
binaries**: `mecha` and `mecha-mail` reinstalled from `main` at `4a888ad` (a
detached worktree, not the shared checkout — see below); `mecha --version`
still says **0.1.17**, because the workspace version has not been bumped, so
the check was capability: `strings` on the installed `mecha` carries `routes
to staging` (the `live_rules` refusal) and `an approval rule asks that this`
(a `prompt` ruling), and the appraisal lane's probes read true — `mecha
sessions health --json` carries `tests_hidden`, `mecha sessions appraise
--json` carries `valence` and `partial`. **Graph binaries untouched**: no
`.rs` under `~/Github/mecha-graph` is newer than the installed
`mecha-graph-mcp` (Sep 2 12:16), the nightly's `target/release/mecha-graph`
is Sep 2 12:15, and the installed server answers 13 tools. **Web dist
rebuilt**: four `web/` commits had landed since the served bundle was built
(Sep 1 18:47, `index-klt_h5v8.js`), including the appraisal chip fix that
otherwise reads a literal `neutral`; built from `main` in the worktree,
rsynced to `~/.mecha/web/dist` (the old dist kept at
`~/.mecha/web/dist.prev-20260903`), and the *served* page verified — the
tailnet door returns 200 with `index-Ciycbb0R.js`; there was no
`deployed-local` tag and both live sessions confirmed the old bundle was
nobody's test. **Services**: `mecha-slack`, `mecha-triggers`, `mecha-drain`
and `mecha-serve` restarted at 10:51:17 with their own startup lines (slack
"Connected to cosanlab as mecha. 1 owner(s), 16 thread(s)", triggers "1
trigger(s), 1 enabled", serve's two doors). `mecha-serve` also prints two
standing warnings worth knowing: `factory__surface_pull` and
`factory__surface_push` "can send and [are] not routed through the outbox" —
a `[outbox] tools` decision for the owner, not a regression.
**`mecha-voice-worker` was deliberately NOT restarted at first**, and was
the one open item until ~11:09 (resolution below): its `WorkingDirectory` is
the shared checkout `~/Github/mecha`, which is on
**`fix/draft-shows-its-account`**, not `main` (`.git/HEAD`; mecha-26's
report, confirmed), so a restart would relaunch the pre-#145 `worker.py` —
#145's echo filter is *not* live until that checkout is on `main`. The
deploying session could not run git against the shared tree (worktree
isolation), so the switch is the owner's, and **the probe is recorded here
rather than its result**, because the two halves of this section's check are
"the launch script hashes the same on both refs" *and* "the working copy is
clean", and only the first could be run from outside. As found on 2026-09-03
~11:00: `scripts/start-moe-mtp.sh` hashed `d76da36c` on
`fix/draft-shows-its-account` and on `origin/main` at `4a888ad`; local
`main` was at `102bacc`, twelve merges behind, so `switch main` alone lands
on a stale ref and the `pull --ff-only` does the real work — and moves the
checkout to whatever `origin/main` is *at execution time*, so re-run the
hash check against that commit, not against `4a888ad`. The working copy was
**not** clean: `git status --porcelain` showed one file, ` M docs/README.md`
— an added `APPRAISAL-RESEARCH.md` row in the research index that `main`
already carries (landed in `1724b2c`, #140), so it is a stale duplicate of
merged work, present before mecha-26's session began and nobody's live edit;
it is what made the owner's first `switch` refuse silently. So, at execution
time, in this order (**run and closed on 2026-09-03 at ~11:08**, see below;
kept here as the recipe for the next time a shared checkout has to move, not
as an open item — a clean tree passes the porcelain test, so the recipe still
runs on a day with nothing to discard):

```
# Block 1 — the checks. Every line is chained on &&, so a failing check
# STOPS the chain rather than printing and moving on. Two is-ancestor lines:
# one for HEAD (the tree that moves) and one for refs/heads/main (the ref
# block 2 force-resets) — each must be behind origin/main, or the one-hop
# `switch -C` would discard something. And no *linked* worktree may have
# main checked out, or `switch -C` refuses — after block 2's discard has
# run; the shared checkout holding main itself is the steady state and
# passes.
R=~/Github/mecha
git -C $R fetch origin \
&& git -C $R merge-base --is-ancestor HEAD origin/main \
&& { ! git -C $R show-ref --verify --quiet refs/heads/main \
     || git -C $R merge-base --is-ancestor refs/heads/main origin/main; } \
&& wt=$(git -C $R for-each-ref --format='%(worktreepath)' refs/heads/main) \
&& { test -z "$wt" || test "$wt" = "$(realpath $R)"; } \
&& git -C $R diff --quiet HEAD origin/main -- scripts/start-moe-mtp.sh \
&& git -C $R diff --quiet HEAD origin/main -- scripts/voice/parakeet_server.py \
&& p=$(git -C $R status --porcelain) \
&& { test -z "$p" || test "$p" = " M docs/README.md"; } \
&& { test -z "$p" || h=$(git -C $R hash-object docs/README.md); } \
&& { test -z "$p" || git -C $R diff -- docs/README.md; } \
&& echo "checks passed — if a diff printed above, READ it: only the one APPRAISAL row? then block 2"
```

If block 1 stops before the echo: the move is not a fast-forward for one of
the two refs, or a *linked* worktree has `main` checked out (`switch -C`
would refuse — find that session), or the launch script differs between
`HEAD` and `origin/main` (do not switch — read the banner), or
`scripts/voice/parakeet_server.py` differs (the move is still fine, but
`mecha-parakeet` runs that file from this tree too and must be restarted
after the switch — a model load, so it is deliberately not in block 2), or
the working copy holds something other than that one file (find its owner
first). Run both blocks in the same shell: block 2 reads `p` and `h` from
block 1. The discard
in block 2 is destructive; it runs only when there is something to discard,
and a person reads the diff between the blocks before it does.

```
# Block 2 — only after a person has read block 1's diff. It re-asserts
# block 1's mechanical precondition first (a peer can dirty a shared
# checkout during the read), lands on the checked ref in one hop
# (`switch -C main origin/main` — never `switch main` then merge, which
# passes through stale local `main` and, if the ff-merge refuses, strands
# the tree there), and opens the journal window at the restart itself.
# Block 1's checks are re-run here — a peer fetching in the shared checkout
# during the read moves origin/main under it — the discard runs only when
# there is something to discard AND its bytes are the ones block 1 showed
# (the blob id `h` carried across; a peer editing the same file during the
# read would otherwise lose bytes nobody read), and the journal window names its zone,
# since journalctl reads --since in local time and this block travels; the
# follow is self-timing (60 s cap) rather than a fixed sleep guessing how
# long the worker takes to come up.
R=~/Github/mecha
p=$(git -C $R status --porcelain) \
&& { test -z "$p" || test "$p" = " M docs/README.md"; } \
&& git -C $R merge-base --is-ancestor HEAD origin/main \
&& { ! git -C $R show-ref --verify --quiet refs/heads/main \
     || git -C $R merge-base --is-ancestor refs/heads/main origin/main; } \
&& wt=$(git -C $R for-each-ref --format='%(worktreepath)' refs/heads/main) \
&& { test -z "$wt" || test "$wt" = "$(realpath $R)"; } \
&& git -C $R diff --quiet HEAD origin/main -- scripts/start-moe-mtp.sh \
&& git -C $R diff --quiet HEAD origin/main -- scripts/voice/parakeet_server.py \
&& { test -z "$p" || test "$(git -C $R hash-object docs/README.md)" = "$h"; } \
&& { test -z "$p" || git -C $R checkout -- docs/README.md; } \
&& git -C $R switch -C main origin/main \
&& since=$(date -u '+%Y-%m-%d %H:%M:%S') \
&& systemctl --user restart mecha-voice-worker.service \
&& timeout 60 journalctl --user -u mecha-voice-worker.service --since "$since UTC" -f \
   | grep -m1 Uvicorn
```

The launch-script check comes **first**, against the commit the pull will
land (`origin/main` after a fetch), because once the switch has happened a
changed script is already the server's next launch command — the first
version of this block checked afterwards, which the banner above says is the
ordering that fails; the second version's checks informed rather than
stopped, and its discard was unconditional in a checkout many sessions
share; the third pulled, which re-fetches past the ref the checks had seen;
the fourth switched onto stale local `main` before merging, a ref nothing
had checked, and did not re-assert the clean-tree check after the human
read; the fifth force-reset `main` with nothing proving the move was a
fast-forward, and re-ran one half of the check but not the other; the
sixth proved the fast-forward for `HEAD` but not for `refs/heads/main`,
the ref that actually moves, and hard-coded the one dirty path so a clean
tree — the safe state — failed; the seventh ran the discard ahead of the
one step most likely to refuse (`switch -C` with `main` held by a linked
worktree); the eighth guarded one launch script and not the other
(`parakeet_server.py` runs from the same tree) and compared the dirty
file's *status* rather than its bytes across the human read. Every pass of PR #152's review landed on this copyable block and
none on the record, which is the point of grading a recipe as code. The lessons are in `HISTORY.md` under Environment. **Run, and
closed, at ~11:08 UTC the same morning** — by mecha-26's session on its own
user's word there, not on the relayed instruction, which that session
declined as it should: a peer's report of the owner's word is the shape a
permission gate exists to refuse. Verbatim end state: `.git/HEAD →
refs/heads/main`, `HEAD → 4a888ad`, `status --porcelain` empty;
`docs/README.md` discarded after `grep -qxF` proved the added line
byte-identical to `origin/main`'s (a copy sits in that session's
scratchpad); `worker.py` carries six `echo_filter`/`take_segment_start`
references, `echo_filter.py` present, `test_echo_filter.py` runs 34 tests
OK. **`mecha-voice-worker` then restarted at 11:09:34** and logged `Uvicorn
running on http://127.0.0.1:7860` — #145's echo filter is live. One
near-miss worth keeping: `d76da36c` is a **git blob id** (`git hash-object`,
`rev-parse <ref>:path`), not a `sha256sum`; the same file digests to
`c08891fb…` under SHA-256, and "the hash changed" read off a different
algorithm would have restarted `llama-local` for nothing. Name the digest
when you write a hash down. `git diff --quiet fix/draft-shows-its-account origin/main --
scripts/start-moe-mtp.sh` — the two trees the switch moves between — is
the algorithm-free form. Also from
#145: `mecha-voice-worker.service` gained a
commented `Environment=MECHA_VOICE_ECHO_RMS=0.020` slot, and the installed
unit is a copy, not a symlink, so the slot is not in
`~/.config/systemd/user/` until someone copies the file — harmless (the
default applies) but invisible to whoever next looks for it.
`mecha-parakeet` left alone (its script did not change). **Benchmark musl**
was **stale at Sep 2 10:30** (0.1.17 by string, pre-arc by content) and was
rebuilt from `main` by `bench/build-portable.sh` in the worktree, then the
static binary copied to the shared checkout's `target-musl/release/mecha` —
the path `bench/run.sh` executes — and verified there: `statically linked`,
and `strings` carries `routes to staging`. A scorecard run before that would
have measured old code and labelled it current. **Factory**: client 0.2.8 =
newest tag `v0.2.8`; droplet read-only `factory 0.2.8`, `active`, untouched.
**Sandbox image**: cargo 1.97.1 matches host, no rebuild. **Stale-process
sweep**: clean — nothing executes a deleted `~/.cargo/bin` binary. **Not in
any repo and not touched**: `~/.mecha/config.toml` carries no `[[rule]]`
yet, so the new subsystem is installed and dormant until the owner writes
one.

**2026-09-03 (~18:20 UTC, the audit lane's session close)** — one more
merge since the deploy entry above, **#155** (`deps/tower-http-0.7`, at
`b50eb24`; what it changes is `HISTORY.md`'s 2026-09-03 entry), and it
**is deployed**: as soon as mecha-2d's 169-call appraisal run released
llama-server (17:56), `mecha` was reinstalled at 18:04 from a worktree
whose code diffs empty against `b50eb24` (the first draft cited a `strings`
literal here that does not exist in the binary; the evidence that the
installed binary is post-bump is the HTTP probe below), and `mecha-slack`,
`mecha-triggers`,
`mecha-drain` and `mecha-serve` restarted at 18:04:29 with their startup
lines (slack "Connected to cosanlab as mecha. 1 owner(s), 16 thread(s)",
triggers "1 trigger(s), 1 enabled", serve's two doors, drain's start).
The probe from the skill's step 1b, run against the tailnet door: `GET /`
→ `200` with `etag: "6a9950bf.0bc7b844-20a"`, `last-modified`,
`cache-control: no-cache`; the same request with `If-None-Match` set to
that tag → `304` carrying the same `etag`, `last-modified` and
`no-cache`. A bare `304` would have meant the old binary; it is not bare.
Stale-process sweep clean. `mecha-voice-worker` and `mecha-parakeet` not
restarted — nothing merged reaches them; the two measurements that need a
worker restart (#153's echo floor, which needs `over_speaker=` on the
`parakeet:` line; #158's deferred tail) wait on those PRs merging first
(mecha-26's report). Open PRs at close: #153, #154, #158
(mecha-26), #156, #157 (mecha-2d). #129 — dependabot's version of the
`tower-http` bump — was closed by the owner at 15:24 UTC, two minutes after
#155 superseded it (the first draft of this entry listed it open; the
review caught it against the API).
The shared checkout `~/Github/mecha` is on `main` at **`b50eb24`**, clean
— re-read from its `.git` at ~18:15 after mecha-26 corrected this entry's
first draft, which had carried the morning's `4a888ad` reading forward
without re-reading it (someone had fast-forwarded it at ~17:47). The deploy
built from a worktree at `main` anyway, because the deploying session
cannot run git against the shared tree; the bytes are the same.
Single-writer docs are held by nobody.

## What the measurements say

Two things a reader needs before trusting any number here, both with the detail
in [`HISTORY.md`](HISTORY.md) under "The measurement record":

- **The original 25-case set is saturated** — all four local models score 23–24
  of 25. It is a floor test, not a ranking test, and it stays in the file as
  exactly that. The tags added since (`long-horizon`, `codegen`, `synthesis`,
  `ambiguity`) do discriminate; qwen3.6-35b-a3b scored 32/34 on the set as it
  stood (`results/qwen-hard-v2.json`).
- **Scorecards taken before the fixture expansion are not comparable to ones
  after it.** The shared workspace went from 11 files to 44, so every case that
  searches it got harder. If you add fixtures, re-baseline every model rather
  than comparing across the boundary.

Everything a judge touched — `ambiguity` and `synthesis` — is worth reading the
answer for rather than trusting the verdict, because judges disagree with
themselves across runs.

**`results/qwen36-after-tuning.json` (2026-08-30) carries its own caveat**:
compare it to `qwen-hard-v2.json` only on the 34 shared cases — 32 passed
both times, one flipped each way, and the one that "broke" (`csv-rows`)
passed 6/6 on rerun. Wall-clock and per-case latency are **not comparable**
(the Aug 25 baseline ran ~2.2x parallel, this one near serial). Net: the
sampling-recipe tuning is *unproven on this instrument*, not proven bad —
the suite sits at 94% with little headroom to show a gain.

**A second instrument arrived 2026-08-19 and has its own caveat**: a year of
real mail at `~/.mecha/mail-corpus/`, which can grade the mail classifier
offline with no human in the loop, because the corpus records whether a reply
went out. **The ground truth is asymmetric and must be quoted that way** — a
reply proves the thread mattered, so classifying it `ignore` is a countable
error; *no* reply proves nothing at all, since most unanswered mail correctly
needed no answer and some was settled in a meeting. It measures false-`ignore`
and is silent on false-`respond`. A scorecard from it that claims precision is
reading the instrument wrong. `docs/MAIL-CORPUS-RESEARCH.md` §3 has the two
caveats in full — **gitignored, like `OPERATIONS.md`**, because its figures are
one person's mailbox rather than a public fact.

---

## What to do next

- **Machine state as of 2026-09-04 10:04, verified surface by surface
  (mecha-26).** `main` is `188b823`; the shared checkout `~/Github/mecha` is
  **on `main` and clean**, fast-forwarded at 10:01:51 *before* the install, so
  the hazard the 05:00 bullet below describes is discharged rather than
  merely aged. #153, #154 and #158 are merged **and deployed**. The `update`
  skill was run end to end: six surfaces checked, **five units restarted**
  (`mecha-slack`, `mecha-triggers`, `mecha-drain`, `mecha-serve`,
  `mecha-voice-worker`), each verified by its own startup line from a journal
  window opened at the restart rather than by `is-active`. `mecha` reinstalled
  from the shared checkout — note the *previous* install had come from
  `.claude/worktrees/harness-review-fixes`, a worktree, which is the shape
  this document keeps warning about. `web/dist` rebuilt and rsynced
  (`index-cix-ATEa.js`); the served 8443 door returns 200 with that bundle and
  a 304 carrying **both** validators, so the binary answering is post
  tower-http 0.7. Stale-process sweep clean. Not restarted, deliberately:
  `mecha-parakeet` — `scripts/voice/parakeet_server.py` is unchanged across
  the whole range and a restart costs a model load. Not reinstalled:
  `mecha-mail` — nothing under it changed and it has no `mecha-core`
  dependency. Graph binaries, the benchmark musl build, the factory client
  (0.2.8) and the droplet (0.2.8, active) all checked and current; the
  sandbox image's toolchain matches the host.

  **The installed binary was verified by what it can do, not by its mtime:**
  `strings ~/.cargo/bin/mecha` finds all three of #158's new literals
  (`"may have been my own echo"`, `"That one is from your"`, `"I keep hearing
  myself"`). A version string would not have distinguished them.

  **`scripts/voice/mecha-voice-worker.service` must not be synced onto the
  installed unit.** The repo copy differs and is *supposed* to: it carries
  placeholder origins (`https://YOUR-HOST.YOUR-TAILNET.ts.net`), being a
  public template, while the installed unit has the real tailnet host.
  Copying it over breaks the voice page's allowed-origins. #153's edits to
  that file are operator documentation for the re-derivation recipe, not
  deployable config. Recorded because "the repo and the installed unit
  differ" otherwise reads as an oversight to whoever notices next.

- **Machine state as of 2026-09-04 ~05:00, verified from this lane:** the
  shared checkout `~/Github/mecha` is **not on `main`, and not a safe build
  or deploy source until it is** — it is on `docs/goal-system-rulings` at
  `6e9d7f9`, the *base of an unmerged PR stack* (#161, with #162 on top),
  which is `origin/main` (`6afd990`, #154) plus two docs commits, clean. It
  has been on that branch since before this session began, so the "on
  `main` at `b50eb24`" in the bullet below was already a misread when
  written. The question is not how stale it is; it is that anything built,
  installed or run from that directory — the voice worker runs
  `scripts/voice/worker.py` out of it — gets a feature branch's content
  silently rather than an error (mecha-83's framing, verified by both
  lanes). Put it back on `main` before the next deploy — the deploy step is
  **check the branch, then pull, then restart**, not "pull, then restart",
  and that ordering is the whole content of the voice-worker item below
  (mecha-26). ~~Three things merged to `main`
  and **not deployed**: #159 (the handoff close), #153 and #154~~ — #153 and
  #154 are still **unrecorded in this document** (mecha-83's flag; their
  owners' entries are owed, this lane did not read their diffs). ~~The
  `update` run is deliberately held until #158 merges, one deploy rather
  than two voice-worker restarts (mecha-26, relaying the owner's call);
  #153's echo-floor and #158's timing tail both wait on that one restart,
  which needs the checkout pulled onto merged `main` *first*.~~ **Struck
  2026-09-04 10:04: the deploy has run — see the bullet above this one.** And a finding
  from #158's lane for the first call after that deploy: `journalctl --user
  -u mecha-voice-worker | grep "Say yes to send it"` is empty over thirty
  days — the spoken outbox confirmation has never played in a real call,
  most likely because no draft was ever staged during a voice turn; stage
  one on purpose. Nothing was installed or restarted by this lane. **Later
  the same morning (~10:00), the owner ruled that nothing calls `pkg` any
  more:** the 01:30 crontab line now runs
  `~/Github/mecha-graph/scripts/nightly.sh` (backup at
  `~/.mecha-graph/crontab.bak.20260904`), Claude
  Code's user-scope MCP server is `graph` on `~/.cargo/bin/mecha-graph-mcp`
  (the `pkg` entry on the private repo's stale binary and empty
  `~/pkg/graph.db` is gone; `~/.claude.json.bak.20260904-pkg` is the
  backup), and Hermes's entry was repointed the same way (its key is still
  `pkg`). Restart Hermes and any long-lived Claude Code session to drop the
  old server processes; mecha itself was already on the public binary. The
  `update` skill's paragraph on the two repos says the rest. Owed in the
  mecha-graph repo, not here: `docs/integrations.md`'s "Consumers (MCP)"
  section still tells a reader to add `pkg-mcp` from the private path.
- **When #153 and #158 merge, restart `mecha-voice-worker` — and check the
  shared checkout first, every time.** The unit's `WorkingDirectory` is
  `~/Github/mecha` and its `ExecStart` runs `scripts/voice/worker.py` from
  there, not from an installed artifact, so "restart the worker" and "the
  checkout is on the merged `main` at that moment" are two facts and only
  one of them is in the unit file (mecha-26's framing). This repo already
  paid for it once, on 2026-09-03 morning. Block 1 of the recipe above is
  the check; the checkout is on `main` at `b50eb24` and clean as of ~18:15,
  so today nothing is owed — the item is the habit, not a repair. #153's
  echo-floor and #158's tail measurements both wait on that restart.
- **Make one voice call, and stage a draft on purpose. It is owed three
  times over.** The voice arc is deployed and three separate things are
  waiting on the *same* call, which is why it is one item and not three:
  (a) `ECHO_SEGMENT_RMS` wants re-deriving — #153 shipped the classification
  method and deliberately **no number**, because the only sample predates the
  2026-09-03 11:09 mic-meter repair and was measured with the browser's echo
  canceller disarmed; (b) the confirmation door's **timing tail** is the one
  constant #158 could not derive, and `VOICE-RESEARCH` records why it cannot
  come from the existing journal (TTS generation runs ahead of playback, so
  `Generating TTS` intervals measure buffering — ~33 chars/s, ≈400 wpm,
  plainly wrong); (c) **the confirmation path has never run.** Thirty days of
  journal contain no `"Say yes to send it"`, most likely because no draft was
  ever staged during a voice turn. That is why a missing HTTP response head
  on that path survived to be found in review rather than in use — so stage
  one deliberately and listen to what happens. The recipe, both populations
  and the `--user` that a system-unit spelling silently omits:
  `journalctl --user -u mecha-voice-worker | grep -E
  "_bot_(started|stopped)_speaking|parakeet( segment gated)?: duration"`
  (mecha-26).

- **Then the timing layer for the spoken confirmation** —
  `docs/VOICE-RESEARCH.md`, designed and blocked only on that call's numbers.
  It is the real fix for the residual #158 states in the direction that
  matters: `MIN_SPAN_WORDS = 2` keeps one-word answers alive on purpose, and
  the same rule makes a bare `"yes"` echoed off *"Say yes to send it"*
  invisible to the gate. No span rule can separate those two without
  silencing every real confirmation, which is precisely why the fix has to be
  something other than the words.

- **Then the spoken override** — issue #165, design in
  `docs/SPOKEN-OVERRIDE-DESIGN.md` (PR #164), designed and not built.
  Changing a harness-supplied parameter (`account`, `reply_all`) by ear at
  review time: state the default, accept an override, re-offer so the new
  value is read back. It closes the half of the owner's original
  account-visibility request that #144 did not. Third in the order and not
  first, because it adds one-word answers to the band the timing layer
  exists to guard — though nothing in it can send, so it is not blocked, only
  better done after. Two rulings are left open in §7 of that doc, one of
  which changes what the appraisal corpus means and should be decided rather
  than defaulted into.

- **Three residues from #152's last review pass**, all in the shared-checkout
  recipe's presentation (HANDOFF §Machine state, dated, 2026-09-03), none
  behavioural: the tolerated dirty path `docs/README.md` is hard-coded in six
  places where one `ALLOW=` at the top of block 1 would do; the recipe's
  version history is narrated in both that entry and `HISTORY.md`
  §Environment — HISTORY's is the copy to keep, the HANDOFF one should
  shrink to the block and the end state; and the `update` skill's "the one
  unit here that goes stale on a change that never touched Rust" now
  undercounts (`mecha-frontdoor` and `mecha-ruminate` run scripts from the
  tree too, but are `.timer`-fired and exec fresh, which is why the recipe
  need not guard them — say so), and `mecha-mail-classify.timer` is a fourth
  timer unit that appears in neither of the skill's two lists (it runs the
  installed `mecha`, so "needs nothing" — but absent is not classified).

- **Two macOS residues from #113's CI arm, parked deliberately** (that
  lane's own flag, so they are not lost): `homeostat.rs` reads
  `/proc/loadavg` and `/proc/meminfo` unconditionally, so on macOS load
  and memory are permanently `None` — degrades honestly, but CI now says
  macOS is supported and that is a silently reduced feature there. And
  `tui/mod.rs` has a lost `\`-continuation from `cfa2cc2` on the OSC 52
  clipboard line — cosmetic garbled output.

**As of the evening of 2026-08-07, self-serve is done.** All six steps of the
factory's `SELF-SERVE.md` are built, deployed, and verified live — a stranger
with an invite claims a handle, gets a certificate in seconds, pairs their
machine, publishes, signs in, and releases from a browser; the operator runs
their day (`factory-publish operator …`) from home. Routine SSH to the box is
over; what remains on it is deploys and disaster recovery, deliberately. The
whole arc is in [`HISTORY.md`](HISTORY.md) under 2026-08-07.

Three live checks are owed — each closes an arc that is built, deployed, and
tested, but has never been exercised against the real box by a person:

1. **The operator panel's first sign-in**: `factory-publish operator signin`,
   open the link, and `/admin` should show accounts, invites, keys and
   withholds. Two minutes, and it closes the panel arc.
2. **A real share**: grant an address you control on a private bundle from
   the viewer's Manage menu, prove the inbox from another browser, watch the
   bytes appear — then revoke and watch them stop. Closes the sharing arc.
3. **A tenant sign-in link**, whenever curiosity strikes: the gate's
   `/account` page is the operator's own tenant page. (The form-verification
   leg, once waiting here too, was run live 2026-08-08 by the booking
   self-test — `submitted → verified → queued`, drained home, acked.)

Everything else below is independent of that.

**Swept again 2026-08-25**, and that pass is the reason this section carries
a warning. The mail section below had listed six shipped phases as unbuilt
since 2026-08-19 — see the trap in [`HISTORY.md`](HISTORY.md) under Measuring.
Two further items closed the same day and moved out (the candidate-class gap
and `show_file`'s call-time config read). The blanket sentence that follows
was true of the items it names and was **not** true of the mail section, which
is why a claim of full coverage now needs the evidence beside it.

Every item below was re-verified against source on **2026-08-24** (a
72-item sweep after the day the phone became a terminal — the web-surface
and voice arcs, 45 commits). Almost everything held its verdict; the items
that changed are rewritten in place below, chiefly: the approval race is
now solved once (`serve/present.rs`) with routing to Slack still open, the
`mecha serve` item part-shipped as a third front-end rather than a shared
backend, and the "Slack `ask_user` is structurally absent" claim is half
false since `Asker::ask_in` landed. A new subsection right after the
remote-control one holds what the new code left open. The prior full pass
was **2026-08-20** — MCP
resources (`mecha-core/src/mcp.rs:206` still advertises `"capabilities": {}`),
HTTP/SSE transports, the subagent workspace field, per-command approval, the
seccomp half of the sandbox item, `Rule`'s missing scope, the raw reflection
window, file watchers and a TUI export are each still absent from the file the
item names, and `gossip`/`vet`/`corroborate` are still in `mecha-cli/src/commands/`.
Three items changed that pass. **Skills** and **Google Docs/Sheets/Slides
write access** both said "not built" and are fully shipped
(`mecha-core/src/skill.rs`; `mecha-mail/src/google/docs.rs`,
`google/docs_server.rs` and a 323-line `bin/mecha-docs.rs`), so both moved to
[`HISTORY.md`](HISTORY.md). The **task store** item shipped in part — the
`/tasks` modal and direct capture exist and the store turned out to be the
graph's board rather than a new one under `~/.mecha/` — and has been rewritten
under Triggers to describe only the escalation half that is still missing.
Ordered by value per unit of effort, not by size.

### The scheduling instrument — live since 2026-08-08

The booking page (`gate…/s/ljchang/book`) and the group poll are deployed and
verified in production: the full lifecycle ran live — book, email confirm,
Outlook event with the manage link in its description, cancel, native
withdrawal — with page, box and calendar agreeing at every step. The arc is
in [`HISTORY.md`](HISTORY.md) under 2026-08-08; the design authority is
[`SCHEDULING-DESIGN.md`](SCHEDULING-DESIGN.md). Open, in the order they bite:

The 2026-08-08 evening front-end pass over the whole instrument —
redesigned booking page, live `slots.json`, the poll paint grid — is
committed (`1d531a8` in that repo) and running on the box; the arc is in
[`HISTORY.md`](HISTORY.md).

- **The calendar→box freshness window is the one real double-booking risk.**
  Page-versus-page is already atomic — `booking_hold` is one INSERT-where-
  no-live-row-overlaps statement (factory `db.rs`), and the losing visitor
  gets the refreshed week; with the front-end pass, an *open tab* also polls
  `slots.json` and drops a slot the moment someone else holds it. What
  remains is the calendar side: an event landing *directly on the calendar*
  only removes its overlapping slots at the next push, up to 15 minutes
  later; `min_notice_hours = 24` makes a collision rare, not impossible. Two
  cheap fixes, complementary: tighten the timer (freebusy is one API call —
  every 1–2 minutes is free, with a hash-unchanged short-circuit so pushes
  stay rare), and teach `mecha-mail bookings` to re-verify each drained
  booking against *live* freebusy before creating the event, flagging a
  conflict loudly instead of proceeding — home always holds fresher truth
  than the box. The second closes the loop entirely.
- **The availability windows are placeholders** (Tue/Thu 13–17, Wed 9–12
  Eastern — invented, not chosen) in **two files that must agree**:
  `~/.mecha/instruments/book-policy.toml` and the `[availability]` section
  of `mecha-manifest/types/book.toml` (then re-`type push`). The page must
  not be handed to anyone before this edit.
- **Booking events land on `dartmouth` (Outlook), named in
  `mecha-slots.service`** because a timer cannot ask the account question.
  First live confusion already happened: the self-test event was "missing"
  from Google Calendar because it was never there. Switch the flag if the
  Google account should own bookings instead.
- **The vanity gate name is deferred by decision.** A redirect
  (`mecha-factory.org/book`-style) upgrades transparently to a gate-alias
  feature later; capability URLs stay minted on the real gate either way.
- **Poll polish, one piece left:** a deterministic auto-book sweep for the
  `clean_winner` case — today `polls status --json` hands the typed verdict
  to the agent, which books and closes. (The tap-to-cycle/drag-paint/heat
  layer shipped with the front-end pass; the axis-locked touch gesture and
  a separate Group heatmap tab from `SCHEDULING-DESIGN.md` §5.3 were left
  out on purpose — inline heat carries most of the value.)
- **A poll cannot hold a real picture, because there is no asset endpoint on
  the box.** Questions and options take `media = { src, alt }` as of
  2026-08-10, and the two sources that render are the only two the policy
  allows: a `data:` URI, or a path this origin already serves. Every class
  sends `img-src 'self' data:`, so the obvious idea — publish a bundle of
  figures and point the poll at it — is refused, and correctly: the artifact
  host is a *different origin* and the browser would block it. An off-origin
  `src` fails when the spec is parsed rather than becoming a hole in a page
  sixty people are looking at.

  What that leaves is inline images capped at 512 KB before base64, which is a
  generous diagram and a hopeless photograph. So "poll a set of images" works
  today only for figures somebody exported small on purpose, and the cap
  cannot simply be raised: a spec travels as one request body through
  `poll_create` and is stored whole, so the ceiling is protecting the store
  and the request, not the page.

  The fix is an **asset endpoint on the box**, scoped to a poll and served
  same-origin, with the questions worth deciding before any code: who may
  upload (the `slots` key, presumably, since that is what creates polls),
  what the per-poll and per-file caps are, which content types are allowed
  and whether the box re-derives the type rather than believing the
  `Content-Type`, and what removes the bytes when a poll is deleted — a poll
  is the one object here with no retention story of its own. Until then
  `polls.md` says the limitation out loud rather than letting a reader
  discover it at the worst moment.
- **Cosmetic:** `factory-publish type push` prints a `/f/<handle>/<id>` URL
  for booking manifests; a booking's page is `/s/…`.

### The self-improvement loop — closed 2026-08-22, now producing its own record

`mecha harness ruminate` runs nightly from `ruminate.sh` (after `work clean`):
diagnose one change from the run corpus → persist it as a candidate → measure
it by counterfactual replay of recent sessions → dispose through
`candidate::judge`. A holdout-confirmed config win auto-accepts into the
override layer (`~/.mecha/learning/harness/overrides.toml`, applied beneath
every config file so the user always wins; `mecha harness revert` undoes);
prose and architecture stage for a person; security-class stages with the
standing warning and is never measured. ARCHITECTURE.md "Harness rumination" holds
the design; `docs/SELF-IMPROVEMENT-RESEARCH.md` §13 records the rulings —
auto-accept per §13.3 is Luke's explicit 2026-08-22 instruction, do not
re-ask it. The corpus is live (64 runs of qwen3.6-35b-a3b across 280 sessions
as of 2026-08-22), and the first nightly pass ran the same evening: the
diagnostician declined on a healthy corpus, which is the designed answer.

What is actually open now:

- **Does the workspace split separate what it claims to?** #127 gave the
  run corpus `RunRow::workspace` and a prefix filter, and the brief now
  reports the mixture — but nothing yet confirms the field sorts sessions
  by the *job* they belong to rather than by something incidental. The
  falsifiable form, raised by the lane working the graph review queue and
  recorded in their words: **every review-queue run has a person present
  and every morning-briefing run does not**, so `work/web`'s "refused by a
  person or a policy" count must be nonzero exactly where `work/morning`'s
  is structurally zero. The briefing half is already measured — 0 denials
  and 0 interlock refusals over 11 runs, against 6.0% environment errors
  and 3.45 turns versus 2.72 pooled. The other half is not. Run
  `mecha diagnose --dry-run --from-workspace ~/.mecha/work/web` beside the
  same for `work/morning`. **If the briefing job comes back showing
  denials, the field is not measuring what §14.3 of
  [`SELF-IMPROVEMENT-RESEARCH.md`](SELF-IMPROVEMENT-RESEARCH.md) says it
  is**, and the stratification argument there needs revisiting. A
  prediction that can only confirm is not one.
- **`prepare_tools` decides inbound mail on the wrong property, and its own
  config doc says so.** `MessagesConfig::inbound` in
  `mecha-core/src/config.rs` states the contract: *"Unset — the default —
  resolves per surface: attended front-ends hold, unattended runs accept."*
  The implementation keys that decision off `global_config_only`, which
  means something else entirely — *ignore the project's `mecha.toml`* —
  and which every remote surface sets for that reason, because a cloned
  repository must not shape a run driven from a phone. Five callers set it
  (`commands/trigger.rs`, `commands/diagnose.rs`, `slack/connector.rs`,
  `commands/serve/chat.rs`, `voice/mod.rs`), and **three of the five are
  attended front-ends receiving `Accept` where the documented contract says
  `Hold`**. The comment beside the branch — "a *scheduled* run (the trigger
  runner is the only caller that sets `global_config_only`) accepts —
  nobody is coming to release a hold" — is the right rationale naming the
  wrong property, and is also wrong about the caller count.

  **Not live on this install, and that is what sets the severity.**
  `MessagesConfig::enabled` defaults to `false` ("a mailbox is a policy
  decision") and `~/.mecha/config.toml` has no `[messages]` section, so
  `prepare_tools` never builds the mailbox and the branch is unreached —
  verified 2026-08-31 at `20bebac`. This is a contract violation to fix
  before the mailbox is ever enabled, not an open exposure today. Whoever
  takes it should read `ARCHITECTURE.md`'s messaging section first; nobody
  has, and it should not be repaired from a grep.

  *Two corrections already, both from a peer checking the address rather
  than the claim. It first cited `setup.rs:879` unqualified — two files
  share that basename, and line 879 of the other is an unrelated test
  assertion, so following the reference read as "already fixed". Then it
  said four callers: a grep for `global_config_only: true` misses
  `voice/mod.rs`, which assigns instead. The pattern you search with is
  part of the claim you can make.*
- **Read the record, weekly at first.** `mecha harness list --all` and the
  nightly log. §2's failure mode (harness updating without benefiting) is now
  answerable from the store instead of from impression — but only if someone
  reads it. Doctor flags a candidate staged past 72h; nothing flags a month
  of rejections that should have been declines, which is the pattern worth
  looking for by eye.
- **The content-sensitive arm is still a human spend.** A prose-class
  candidate stages with its `mecha eval --ab-config` command attached and
  waits; nothing runs eval arms unattended, deliberately (a judge-graded arm
  costs real runs and grades what the model *said*, which replay cannot see).
  If prose candidates accumulate, decide then whether that spend gets a
  budget.
- **Divergence rate is unmeasured.** Replay under a pinned seed
  (`seed = 42` on `[providers.local]`) is reproducible, but a
  behaviour-visible change diverges from the recording by design and drops
  the episode. If most episodes drop on real candidates, the session arm
  yields thin evidence and everything routes to review — which is safe, and
  worth knowing before trusting the loop is "measuring". The candidate
  records carry `diverged` lists, so the answer accumulates on its own.
- **Security boundaries stay unproposable** per §13.2 — interlock, path
  jail, sandbox and outbox routing reach a human however anything scores,
  and the standing recommendation is that they are never proposed at all: a
  loop that can argue for widening its own confinement will eventually argue
  well, and the metric agrees with it, because a run that can reach the
  network fails fewer calls.

### The remote control — built, with one path nobody has been able to run

`/remote-control <name>` mirrors a live TUI session into a named Slack thread,
both directions, files included. Built and merged 2026-08-21 (`d266fe8`), live
on this machine. `docs/REMOTE-CONTROL-DESIGN.md` is the design; the arc is in
[`HISTORY.md`](HISTORY.md) under 2026-08-21. What remains:

- **`/send` still reads the whole global config at call time.**
  `slack/send.rs:141` (`send_file`) loads `Config::load_global` for one number,
  and the TUI's `/send` reaches it. Same root cause as the `show_file` bug
  fixed 2026-08-25 and a milder version of it — the parse error lands on a
  keystroke the user just pressed rather than on a tool call two hours in, so
  it is comprehensible where the other was not. Closing it means a field on
  the TUI's `Live` struct, beside `todo` and `skill`, which ride there for the
  same reason: a `/model` switch rebuilds the agent and refreshes them.
  Deliberately left out of the `show_file` fix rather than overlooked.

- **A top-level DM starts a connector run even while a session is attached.**
  Working as designed — the thread is the unit — but the observed failure mode
  is a person posting a screenshot into the DM rather than the attached
  thread, and getting a fresh connector conversation in a different workspace
  with no sign that the live session exists. The cheap fix is a line in the
  connector's reply naming the live attach; routing it is a bigger decision
  that §2 of the design already settled the other way.

- **Approvals cannot be answered from the mirrored thread — but the race is
  now solved once, elsewhere.** The atomic claim design §15 asked for shipped
  in `serve/present.rs` (2026-08-24): `Questions::open`/`answer` — a second
  device answering an already-claimed card gets `false`, timeouts resolve as
  `Blocked`, cancel drains parked cards. What remains here is routing: the
  TUI still says *waiting for you at the terminal* (`tui/mod.rs:1127`), and
  wiring the mirrored thread means a Slack renderer over the same
  `Questions`, not a second claim mechanism.
- **`mecha serve` shipped 2026-08-24 — as a third front-end, not yet the
  shared backend.** The hosted process exists, survives SSH drops, reaches
  a second machine (the phone, daily), and the voice facade mounts inside
  it. What the item originally promised and remains open: the TUI
  (`tui/mod.rs:943`) and the Slack connector still build their own `Agent`
  each — "both front-ends are thin clients of it" is not true, and deciding
  whether it ever should be is now a real question rather than a design
  sketch, because three agent-owning processes on one llama-server is the
  live shape. Design §15; `REMOTE-SURFACE-DESIGN.md` is the authority for
  what serve itself still owes.

### The phone surface and voice — what is still open

Everything here is verified in source, re-checked 2026-08-25; the arcs' own
docs (`REMOTE-SURFACE-DESIGN.md`, `VOICE-RESEARCH.md` §7) hold the shipped
half.

- **The website's docs build can fail at random on a network blip, and the
  fix is on an unmerged local branch** (`fix/sync-graph-docs-loud`,
  `9328ba8`). `sync-graph-docs.mjs` soft-skips a failed fetch from the
  public graph repo, but `onBrokenLinks: 'throw'` in `docusaurus.config.ts`
  turns the missing generated page into a hard failure blaming two tracked
  docs — so the flake reads as somebody's doc edit (it hit PR #125's first
  CI run; a bare rerun went green). The branch makes fetches retry and a
  still-missing file fail loudly naming the fetch, verified both ways
  (old exits 0 against an unreachable repo, new exits 1). Needs a PR.
- **`graph_verb` and `proposals::run` are near-duplicate spawn-mecha-graph
  helpers in the same crate** (`serve/board.rs::graph_verb` relays stderr
  whole for ambiguity candidate lists; `serve/proposals.rs::run` relays
  first-line-then-stdout) — flagged by #126's review as the divergence that
  will drift. Claimed post-merge: lift one `graph_child` helper with the
  relay style as a parameter, both call sites moved in one commit. Test it
  with a real child in a scratch `MECHA_GRAPH_DB` — a helper over two spawn
  sites is exactly the change whose failure mode is invisible to tests that
  never spawn (the reject regression under a green 1,986-test suite is the
  proof, Traps → Review process).
- **`mecha kg search` never prints its `entities:` line** —
  `commands/kg.rs::search` does `filter_map(|e| e.as_str())` over
  `pack["entities"]`, which is an array of objects (`{name, node_id, …}`),
  so the filter drops every element. Same object-vs-string mismatch the web
  chips hit twice in #126. Found by #126's review, deliberately left out of
  that PR.

D3 shipped 2026-08-25 — a call is the chat session it was started from.
The narrative is in HISTORY under that date; `VOICE-RESEARCH.md` §7 holds
the mechanism and every decision. What it left standing:

- **A `voice:` session is now the fallback, not the norm.** A call from the
  app is a `web:` session, so the drawer's `voice` badge marks only calls
  that named nothing — a direct offer to the worker, or a key no front-end
  held. The badge's meaning quietly changed from "this was spoken" to "this
  call had no page behind it"; the per-turn `spoken` marker in the
  transcript is what carries the first meaning now. Worth a deliberate
  decision about what the drawer should say, rather than leaving two
  meanings one word apart.
- **The page's mode chip does not describe spoken turns.** `--voice-yes`
  travels with the *turn* (`TurnOpts::approve_all`,
  `serve/chat.rs`), so a spoken turn runs at Allow while a typed turn in
  the same conversation runs at whatever the chip says. Luke's call,
  2026-08-25, and defensible because nothing structural moved — the
  interlock is ahead of the approver, sends still stage, and taint
  accumulates across both doors. Still worth revisiting: a label true of
  one door and not the other is the shape this project usually refuses,
  and if voice ever grows spoken approvals, that flag is what they replace.
- **`mecha serve` never drains its chat runs on shutdown**, and since D3
  that gap covers spoken turns too. Not a regression in practice —
  `axum::serve` is not wrapped in `with_graceful_shutdown` and there is no
  SIGTERM handler, so systemd's stop is a hard kill and `facade.shutdown()`
  was already unreachable there. But the standalone `mecha voice-serve`
  *does* handle SIGTERM and the mounted one does not, which is now the only
  place that difference shows. Closing it means deciding who owns SIGTERM
  in a process holding SSE streams and pending approval cards.
- **A second device watching a typed send sees the reply and not the
  prompt.** A spoken turn broadcasts `WireEvent::User` because it has no
  local echo anywhere; a typed send is still echoed only by the page that
  typed it, so broadcasting it too would render it twice there. Ending it
  properly means the page distinguishing its own echo from the broadcast,
  which is a small change nobody has needed yet.
- **The owner's first-day feedback backlog is `REMOTE-SURFACE-DESIGN.md`
  §12** — chat model switching, a plain mail inbox + compose, notes/tasks
  voice capture and listings, the task→agent handoff (the big one), Home
  navigation/widgets. Triaged there, not restated here. The plain inbox +
  compose and the notes/tasks voice capture shipped 2026-08-24 night (Arcs B
  and C); the rest stands.
- **Delegation is a conversation as of 2026-08-26 (eighth pass)**, which
  closes the phone surface's biggest gap and opens two smaller ones.
  `ask mecha` opens the task's chat session — voice, uploads, todo panel,
  approval cards, steering by typing — the model speaks first, the board does
  not move, and *let it carry on without me* hands the same transcript to a
  detached child. HISTORY has the narrative and `ARCHITECTURE.md` the invariants.
  What that leaves open:
  - **Admission control (R1 / phase 6) shipped 2026-08-26**
    (`mecha-core/src/permit.rs:50`), after R3 was run: three background seats
    against the server's four, interactive work uncounted. What is left of it
    is the half that was never the point — **there is still no queue.** A
    refused delegation is told to try again; nothing remembers it and nothing
    starts it when a seat frees. That is deliberate for now (a queue that
    starts work unattended is a scheduler, and the trigger daemon is already
    the place for scheduled runs), but it is the gap between what shipped and
    what the owner asked for — *"when the question is answered it should
    resume in the queue"*. Worth building only if refusals actually happen;
    the permit files make that countable.
  - **PR #84 is open and unmerged**, with the last three findings from the
    #78 review: the pressure line stating a cost and a turn count derived from
    different numbers (a 400-token pace read as *"~1k each, so about 224
    more"* against a 100k limit), an unreachable match arm beside it, and
    `replay.rs` / `harness_probe.rs` building `ToolCtx::default()` so a
    recorded compaction answers *"compaction is not enabled for this run"* —
    false of the run being replayed, a divergence under
    `--on-divergence=live`, and in a harness probe a `compact_at_tokens`
    candidate measured on runs that never compacted. 1,566 tests and clean
    lint locally; **CI unverified** — it had not reported when the session
    ended. Check it before merging rather than trusting this line.
  - **The overnight half of R3 is still unmeasured.** The contention half was
    run (see the measurement record) and answered the question R1 rested on.
    What §3.4 also asks — *does a conversation parked overnight get its prefix
    back, or has a night of triggers and chat pushed it out?* — has not been.
    The contention result makes it more likely to be fine than the design
    assumed, which is a reason to expect a cheap answer, not a reason to skip
    it. `scripts/slot-contention.py` is the shape; the missing arm is time,
    not concurrency.
  - **`mecha serve` still has no graceful shutdown**, and it now costs more
    than it did: a restart kills a conversation-owned run mid-flight, where a
    handed-over one survives. The conversation itself survives either way
    (the transcript is the record), so what is lost is the partial turn. Same
    unresolved question as before — who owns SIGTERM in a process holding SSE
    streams and pending approval cards.
- **The task→agent handoff is built through phase 4** —
  `docs/TASK-AGENT-DESIGN.md` is its authority and HISTORY has what shipped.
  The return path and D16's card states closed on 2026-08-26 (second pass);
  B1 and B2 closed the same evening, both **amended first** — re-reading
  them against the shipped row changed both, and the amendments sit beside
  the originals in the design doc. **D4 shipped 2026-08-26 (sixth pass) —
  the seed points at context rather than assembling it into the prompt; the
  amendment sits beside D4 in the design doc and HISTORY has the narrative.
  What is left of the arc is one measurement and one deferred decision.**
  - **Whether runs follow the pointer, which is D4's own acceptance test.**
    *"Measured in Phase 4, not assumed in Phase 1"* has two runs behind it,
    which is a direction rather than a result. Both followed. A task naming
    only a project drew seven graph calls (`kg_search` ×4, `kg_entity` ×3);
    a task captured from a real three-message mail thread called
    `mail__mail_get_thread` **as its first act**, before anything else, and
    the recorded taint came back `private + untrusted` — the bytes arriving
    as a tool result the interlock accounted for, which is the whole reason
    the seed carries a pointer. Keep watching rather than concluding: the
    query is a scan of task-titled transcripts for a call to the tool the
    seed named, and pasting the context stays available if runs stop
    following it.
  - **A captured task's *name* is untrusted text, and the seed cannot fix
    it.** D4 withholds the pointer's `label` because a subject line is prose
    somebody else composed — but `mail task` defaults the task **name** to
    the classifier's `one_line`, falling back to the raw subject
    (`commands/mail.rs:1947`), so a default capture puts a model's paraphrase
    of a stranger's mail at the top of a privileged run's opening
    instruction. The front door's own rule is that *a paraphrase of an
    injection is the injection rearranged*, and this is that rule's own
    counterexample sitting one store over. Found by the PR reviewer on #68,
    not by the tests — which passed because the fixture's name and label
    differ, and in production they usually will not.
    **Not fixable in the seed**: the name is what the task *is*, and a run
    that cannot see it cannot work it. So the question is what capture should
    default to, and the options are worth pricing before one is picked —
    require `--name` and refuse to default (safest, and friction on the verb
    that exists to be cheap), keep the default but mark a derived name on the
    record so the seed can say where it came from, or leave it and accept
    that a delegated run's subject line is attacker-influenced. Pre-existing
    since capture shipped 2026-08-26; nothing regressed, and it is written
    down here because `CLAUDE.md` previously read as though the boundary held.
  - **The seed's bullet order is now pinned by a test, and the reason it
    moved did not survive its own measurement.** The pointer bullets first
    shipped directly beneath *"do not ask what you can find out"*, so the
    section ended on two consecutive reasons not to ask; pooling every
    substantive run of the day made that look like a regression —
    `ask_user` in 5 of 6 runs under the previous seed against 0 of 4 under
    the new one — and the bullets were moved above the asking block on that
    reading. **The pooled number is confounded by task and should not be
    quoted.** The arms ran different tasks; the only within-task series is
    one board item (*decide whether to submit to a conference*, six runs),
    and there `ask_user` fired once in two old-seed runs, zero in two
    ask-first runs, and zero in two lookup-first runs. So the order change
    is justified on reading order — here is what you can find out, *then*
    ask about what is left — and **not** on a measured effect, and the
    question of whether this model asks less under the new seed is still
    open at n far too small. The test asserts the order rather than the
    outcome, which is the part that is actually decided.
  - **Front-loading landed as prose, not as a parked question, on the one
    run that had questions to ask.** The mail run ended `completed` with six
    numbered decisions in its final answer and **zero `ask_user` calls**,
    with the tool on its surface (63 tools, `ask_user` among them,
    `kg_task_update` correctly withheld). So the questions exist only inside
    a session transcript: `mecha questions list` does not show them, the
    phone's card cannot offer them, and the delegation reads as finished
    rather than as blocked — which is the state D13 exists to make
    impossible. One run, and the mechanism does fire elsewhere (two
    questions are parked from other tasks), so this is a prompt-adherence
    observation and not a broken store. It is also the exact seam D12 was
    decided against on: the seed asks on the user turn because that is the
    channel this model obeys, and here it obeyed the *content* of the
    instruction (ask first, one question, list every unknown) while ignoring
    its *mechanism*. Worth a second look before concluding anything — if it
    repeats, the cheap fix is naming the tool call in the sentence that
    already says what to ask.
  - **D12, the plan gate — decided against as written, on 2026-08-26, and
    the cheap half shipped instead.** Do not build it from the design doc;
    read `work_prompt`'s doc comment in `commands/tasks.rs` first, which
    carries the reasoning. In short: D12 made the *todo list* the
    human-editable object, which conflates the agent's execution ledger with
    a reviewable plan — every other system keeps those apart, and `todo.rs`
    already forbids a second author of its state. Three findings decided it:
    `VERIFICATION-RESEARCH.md` argues against plan-first on this hardware in
    particular (small models *collapse* under plan-and-execute; a bad plan
    measures worse than no plan); the gate's trigger rested on a `todo` write
    this model was measured not to make from prompting (HISTORY 2026-08-04,
    zero calls in 20 runs); and D12's own cited evidence — Copilot's
    38.1% → 69% — was about tuning the *seed*, which is `work_prompt`.
    So the seed now front-loads questions, with a guard against the opposite
    failure, delivered on the user turn because that is the one channel the
    probe found this model obeys.

    **What is still unbuilt is the part front-loading cannot reach**:
    misalignment the model does not notice, where a confidently wrong plan
    asks nothing. The decision to build a reviewable plan *document* —
    separate from the todos, on superpowers'/Claude Code's split — is
    deferred until the corpus argues for it, and the query is now available:
    **delegations that ended `ready for review` and were then dropped or
    reworked rather than marked done.** Board status transitions plus
    `RunStats` plus the session answer it. If that number is small the gate
    is friction; if it is large, the same query names the examples it would
    have caught. The asymmetry that would force the question sooner: a
    web-launched delegation is `--unattended` today, so it can only read and
    stage, and the downside of letting a run go is bounded by construction.
    That stops being true the moment a delegated run can get approval from a
    present human.
  - **Phase 6** is admission control (R1) — explicitly not worth building
    until more than one delegation at a time is routine.
  Two things found while building the return path are worth knowing before
  touching this arc again. **A task run's `Record::Outcome` only exists from
  2026-08-26 (second pass) onward** — `tasks work` and `questions answer`
  were the two front-ends that never wrote one, so every delegation before
  that date is invisible to `runlog` and to `sessions health`, and the
  design doc's own "task shape" question ("`RunStats` already records enough
  to answer it later") was false for exactly the runs it was about. The
  corpus starts accumulating now. And **`/api/tasks` passes `--closed`**,
  because the drawer's `done` view filters on `done | dropped` and the list
  it filtered had never contained either — a view that had been
  structurally empty since it shipped, in the way that reads as "you have
  finished nothing" rather than as a filter that cannot match.

  Part 3 of that document records a refusal worth knowing before it is
  proposed a third time: **no KV-offload manager** — `-cram` already is one,
  and the hand-rolled `--cache-idle-slots` was measured costing 20.5s
  re-prefills and removed.
- **`--voice-yes` is a deliberate posture with a named risk.** Voice runs
  get `ModeApprover { Allow }` (`voice/mod.rs:552`; the unit runs the flag)
  while web chat defaults read-only with live cards — so one process serves
  two dialects at two postures, and the hands-free one is the permissive
  one. The reasoning (a voice call has a present owner and cards cannot be
  tapped mid-sentence) is recorded in the flag's doc; if voice ever grows
  spoken approvals, this flag is what they replace. `allow` mode is equally
  deliberately **not** offered from the page (`serve/chat.rs`) — approve one
  call at a time in `ask` mode instead.
- **The web app imports outside its package root** —
  `../../../scripts/voice/voice-core.js` — which couples `npm run build` to
  the whole checkout's layout. It was deliberate when the module served two
  shells and had to be owned by neither; since the standalone page was
  retired (2026-08-25) the app is the only consumer, so the argument now is
  only that the module must stay framework-free for whatever embeds voice
  next. Moving it under `web/src/lib/` would be defensible and would end the
  outside-the-root import; it has not been done because the module is the
  voice arc's to own and the web app's build is not. **There is now a second
  crossing, in the other direction** (2026-08-29, #118):
  `website/scripts/check-charter-toml.mjs` imports
  `../../web/src/lib/charter-toml.js`, so the docs package reaches into the web
  app's source. That one is deliberate and is the point of the gate — it exists
  to compare the serialiser against `mecha-core`'s reader, so it must import the
  real module rather than a copy — but it means two packages now depend on the
  checkout's layout, and anything that moves `web/src/lib/` breaks a CI gate as
  well as a build.
- **Three cosmetic residues on the settings surface** (2026-08-29, from #118's
  last review round; none blocking, recorded so they are not re-derived).
  `serialize` writes `esc(l.text.trim())` while `Charter::validate` trims only
  the `id`, so saving from the list normalises leading and trailing whitespace
  inside a line's text — the owner is explicitly saving, so it is a
  normalisation rather than a loss. `budget = charter?.budget ?? 2000`
  duplicates `CHARTER_CHAR_BUDGET` as a literal fallback on a path where the
  server always sends it. And `charter_state` reads the file twice
  (`read_to_string` for `raw`, then `Charter::load` for `lines`), which
  pre-dates this arc but #118 is the first consumer to *combine* the two — the
  header comes from `raw` and the rows from `lines` — so a write landing
  between the two reads would hand the page a header and a body from different
  documents. Single-writer file, so the window is theoretical; it is the only
  one of the three with a failure mode rather than an untidiness.
- **A phone verdict has no undo**, which doubles the stakes of the
  `undecide <seed>` design below: `POST /api/queue/verdict` is a second
  irreversible verdict surface — and since the global similarity layer
  (2026-08-24 evening), one tap can now carry a cross-class cascade of
  dozens, so the undo design is worth pulling forward. **Unchanged by the
  2026-08-25 pass**, which only made a *failed* verdict honest: the card now
  stays and offers the two ways through (`bind`, accept-as-new-topic) instead
  of vanishing while the candidate stayed pending. A verdict that lands is as
  irreversible as it ever was, and accept-as-new-topic adds a tap that mints
  a topic node.
- **PWA install and Web Push are unstarted** (design Phase 5). Until push,
  Slack remains the nudge channel for staged drafts — D11 keeps it anyway.
- **A draft released by voice leaves a stale card on the page.** Since
  2026-08-25 both doors offer the same drafts: the page draws a confirm card
  and the call asks about it aloud, deliberately, because they are two views
  of one conversation. Whichever one acts, the other's copy does not know.
  The store refuses the second attempt — `outbox approve` on a resolved item
  errors and the card shows the CLI's own words — so nothing double-sends;
  what is missing is the *notification*, which means a `SessionHost` method
  the facade can call to broadcast a `WireEvent` into the page's stream. Not
  built because it is the only thing the trait would carry and no one has
  been bitten yet.
- **The spoken confirmation is not recorded in the conversation.** A "yes"
  handled by the harness runs no model turn — 0 prompt tokens, 0 completion
  — so the transcript has no record of the exchange, and a later turn cannot
  see that the send was authorised. The *authorisation itself* is on the
  record where it belongs (the outbox item's status and `resolved_at`), so
  this is fidelity rather than an audit hole: `distill` and the run-quality
  corpus simply do not see it. Closing it is the same `SessionHost` seam as
  the item above.
- **The model still narrates a staged draft the harness is about to read
  back.** D10 now tells it to say one short clause and stop, and the local
  model mostly complies ("Drafted and waiting for release."), but not
  reliably — the run that opened the arc said the whole thing twice. The
  duplication is verbose rather than harmful, and arguably a feature: the
  harness readback is the authoritative one, so a *disagreement* between the
  two is audible. Worth a decision rather than more prompt tuning.
- **The real calls happened on 2026-08-25, and what they found is now
  fixed — but three questions the item was really about are still open.**
  Luke made several calls from the phone (`af_heart` voice, a calendar
  question round-tripped, one that ran 5.5 minutes) and reported one thing:
  calls ending by themselves. That had two causes, both closed the same day
  and both in HISTORY — pipecat's 300-second idle timeout, and ICE
  `disconnected` treated as terminal. What the calls did **not** settle,
  because nobody was listening for it, is what the item was written to ask:
  whether the AEC/echo filter holds on a speakerphone walk, which of the
  seven voices is worth keeping (six Kokoro-derived references plus
  Chatterbox's own `default`), and whether the D10 block re-arriving after
  every typed turn keeps replies ear-shaped or is one copy too few. Those
  need a call taken *for* them.
- **One call connected deaf and nobody knows why.** 2026-08-25 13:27:43: the
  peer connection established, the data channel opened, the voice config
  round-tripped — and `Timeout: No audio frame received` repeated every two
  seconds for the whole 21 seconds, from the first one. The retry 8 seconds
  later worked normally. One occurrence, not reproduced, and distinct from
  the teardown bug that *was* fixed. Worth watching for: a call that connects
  and hears nothing is indistinguishable from a call the model is ignoring,
  and the client has no signal for it at all.
- **Voice input has no denoising, and the research says leave it that way —
  but the VAD is untuned, which is the actual lever.** Deferred 2026-08-24
  with the finding attached, because the obvious fix is the wrong one.
  Denoising and false-triggering are different jobs: the VAD decides what
  becomes a turn, the denoiser only cleans what already got through. And
  Pipecat's filter mutates the frame in place
  (`BaseInputTransport`: `frame.audio = await ...filter(frame.audio)`), so
  `audio_in_filter=RNNoiseFilter()` denoises the audio reaching **Parakeet**
  too — and a systematic study that tested speech enhancement against four
  ASR systems *including NVIDIA Parakeet* found it degraded every one, by
  1.1–46.6% absolute semWER (arXiv 2512.17562; at 10 dB SNR, 8.82% → 25.83%),
  through artifacts inaudible to humans and the removal of features the model
  was already using. What you would want is denoise-for-VAD, raw-for-STT, and
  Pipecat's filter API cannot express it.

  **Step (1) shipped 2026-08-25 (`c7fc975`), and not by tuning.** The VAD was
  dropped from the turn-*start* strategies
  (`worker.py:347`, `start=[TranscriptionUserTurnStartStrategy(...)]`), so a
  wordless segment now emits no `TranscriptionFrame` at all, reaches no
  strategy, and the bot simply keeps talking — "resume on an empty
  transcript" achieved by never stopping, which needs no state to unwind.
  The analyser stays because it still *segments*; `start_secs` moved 0.2 →
  0.3 (`worker.py:346`) and `confidence`/`min_volume` were deliberately left
  alone, since the owner's measured speech is ~0.024 RMS against a 0.14
  tuning assumption and raising thresholds to chase noise would start
  dropping quiet real speech. The cost, stated in the source: Parakeet is
  offline, so barge-in is "finish the phrase and it stops" rather than
  instant.

  **What is still open is (2) and the denoiser question.**
  `MIN_SEGMENT_RMS = 0.010` (`worker.py:75`) sits **one thousandth**
  above the measured room noise of 0.009 — tuned in a quiet room, and wind
  will not notice it; re-measure it in the conditions that matter before
  trusting it outdoors. Note the *cost* of a false positive already collapsed
  twice over: with the Parakeet swap a noise segment yields empty or garbage
  text the gates drop where Voxtral produced a confident fabricated answer
  attributed to the owner, and since (1) it no longer interrupts either.
  Only after (2): RNNoise is one
  `pip install pipecat-ai[rnnoise]` away, but wind is non-stationary
  broadband — RNNoise's weakest case and DeepFilterNet3's strength, and DFN
  is not in Pipecat core (issue #3266). Measure Parakeet's WER either way.
- **Parakeet hotword biasing is blocked on a missing `bpe.vocab`, and the
  failure mode is a segfault.** Deferred 2026-08-24. This is the wanted half
  of what an LLM-decoder ASR would give (domain vocabulary — contacts,
  project and place names from the graph) without the half that lost Voxtral
  the seat, because a hotword list is a decoding-time constraint on the
  search and not a prompt: there is no channel down which an instruction can
  travel. `sherpa_onnx` 1.13.6 exposes `hotwords_file` / `hotwords_score` /
  `modeling_unit` / `bpe_vocab`, and `create_stream(hotwords=...)` takes them
  per stream, so a list could be refreshed nightly without reloading the
  model. But the model directory
  (`sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8`) ships `tokens.txt` — 8,193
  `▁`-prefixed sentencepiece tokens — and **no `bpe.vocab`**. Measured: the
  `cjkchar` default does not crash and silently does nothing on a BPE model
  (output identical to baseline); `modeling_unit="bpe"` **segfaults inside
  `create_stream()`**, and a `bpe.vocab` reconstructed from `tokens.txt`
  builds the recognizer and then segfaults in the same place. Needs either a
  correct vocab or a model package that ships one. **The segfault is itself
  the finding**: sherpa-onnx crashes the process rather than raising, so any
  real version must validate a generated list in a subprocess before the
  live recognizer sees it — otherwise a bad nightly write takes `:8992` down
  and voice goes deaf with no error text anywhere.

### The goal system — rungs 0–10 all shipped, out of build order; §17's rulings are in, their first two sprint PRs exist, and rung 9's review-queue salience is unverified from this branch

**2026-09-04, later — §17.6 item 3 is built and merged as PR #168
(`feat/situation-scope`, merged at `046ef0f` after twelve review passes,
the last clean at the bar), with §17.7 item 1's run record in the same
change because it was the merge condition. Not yet deployed: the
installed binary is behind main by several lanes, and the next `update`
run takes them all.** What it built is in
`HISTORY.md` under this date: `situation.rs`, the situation on every new
reflection, `scope` on `Rule` assigned by the harness from the batch's shared
keys, one learner call per focus-tool batch, the loader matching scope
against the run's registry (`rules_carried_for`), and `RunConfig::rules_hash`
+ `rule_ids` beside `RunStats::delivered`. **Open from item 3, named in the
§17.4 built note:** mid-run delivery on a recurring condition (§17.7 item 2
keeps it off by default until the null-step and restart counters are read),
region widening and narrowing in consolidation, per-region validation
budgets, the situation backfill for the 41 reflections mined before the
field (§17.7 item 6), and surface and workspace as *scope* keys — both are
recorded, neither is matched, for the reasons in `situation.rs`'s module doc
(the surface needs `prepare` to be told the session kind; the workspace
waits on widening, or nearly every rule pins to one workspace). The twelve
live rules on this machine are unscoped and load everywhere as before, until
the standing batch rewrites them; the first scoped rule appears when a
region batch of new reflections learns.

**2026-09-04 — the owner's rulings of 2026-09-03 are recorded (§17, PR #161)
and §17.6's first sprint is built as #162, stacked on it; both since merged.**
What #162 built is in `HISTORY.md` under this date: sensored charter lines
(five kinds, each with a reader), the `serves: charter:<id>` ask on the
prompt block, the attribution join in `of_session`, and the relevance gate —
`Affect::Distress` on every free-readout negative, `Pride` for a delivery
against a charter line the loaded charter contains. **Paragraphs below that
say the label is `Neutral` on every session, that `Pride` is genuinely
unbuilt, or that no session names a goal are true of the corpus as measured
and stop being true of the derivation when #162 merges** — they are kept as
the measurement they were. **Open, in the order §17.6 sets** (item 3 is
#168 above, its merge condition met in the same change): §11.1's later phases —
the per-line readings, the line-specific guilt, the doctor's owner
thresholds, the editor showing a reading beside the line, the replay
tiebreak, and `board_overdue`/`cost` as kinds once a reader exists; item 4,
measuring the appraiser at scale (zero build — and #160 has since reported
169 of 169 "nothing further", so retire-or-refeed is the actual question);
item 5, the audit lane's declared check. **The exit for item 1 is not
measurable on this machine yet**: the live charter has seven lines and no
sensor, so `sessions appraise` will print "no charter line carries a
sensor" beside a zero until the owner adds one — and an older binary on the
other machine would then run *un-chartered*, not refuse, until updated
(`mecha doctor` reports it). #157's `APPRAISAL-RESEARCH.md` §1.7 "label
neutral on 169 of 169" was measured by 0.1.17 and would read `distress` on
the 24 signed sessions under #162 — expected, not a conflict (mecha-2d's
check).

`docs/GOAL-SYSTEM-DESIGN.md` is the authority and carries a status header
saying which rungs shipped. The arc's premise, which is what makes the rest
follow: **every evaluative signal mecha had was a cost or a correction**, so a
run could be recorded as having gone badly and never as having gone well.

Rungs 0–6, rung 7's observation half, and rung 7's quarantined appraiser
(`--appraise`, below) are in `main`. **Rung 7 shipped in full on 2026-08-28**
with the model half of step appraisal (§5.5's escalation) — see below. So are
rung 9 (episode tagging + gossip seeding, PRs #97/#98 — landed by another
lane; not re-verified in this entry) and rung 10 (PR #100, 2026-08-28): the
charter (`mecha-core/src/charter.rs`, `mecha-cli/src/commands/charter.rs`)
and the homeostat's aggregate into `diagnose::Evidence` shipped as designed,
and anticipated guilt (`mecha-core/src/guilt.rs`) shipped **as a recorded
sensor only** — `Homeostat::anticipated_guilt`, with no behavioural consumer,
on the same observation-first precedent the appraiser above and boredom both
used. Rung 10 landed ahead of rung 8 in build order on purpose: the charter
and the guilt sensor depend on neither the probe nor the label (see the
corrected caution below). Everything through rung 6 has no model in it, which
is §14's ordering and not a coincidence — the appraiser was the first thing
here that spends a model call at all, on the observation-first shape every
other rung took: offline, budgeted, left without a store until a real corpus
says it earns one.

Step escalation is different in kind — it is a *live* concern (a step's plan
action has to reach the same run before it wastes more turns, so it runs
inside `agent.rs`'s loop rather than as a CLI pass) but the same posture on
cost: off by default (`[agent] step_escalation`, `--no-step-escalation`,
forced off under `mecha eval`), since the pre-filter's thresholds (span ≥3×
the plan's other completed steps' mean, floor of 6 calls; a
verification-claiming step with no verify-shaped call in its span) are argued
rather than measured. Verified live against the local model: a deliberately
oversized step correctly drove the quarantined call, which judged the size
intentional rather than a decomposition problem, and its reasoning never
reached the recorded transcript — only the closed `accept`/`revise_plan`
verdict can, and only as a fully templated nudge.

**And rung 8 shipped after rung 10 too** (PRs #99, #103, merged 2026-08-28,
this entry's own work) — the last rung to land against the caution below,
by the same kind of explicit owner ruling rung 10 landed under: the corpus
still said the affect label was degenerate at build time, and building
anyway was the call that the mechanism is worth having correct independent
of how interesting today's label is, not that the caution's underlying
argument was wrong. Two pieces:

- **§5.4, goal-closure appraisal.** The trigger is `tasks set --status
  done|dropped` (`mecha-cli/src/commands/tasks.rs`'s `is_fresh_closure`),
  which every surface that mutates a task's status already shells out to —
  the TUI's `/tasks` modal, the web board, and Slack's Done tap — so hooking
  there covers all of them for free, and D6 (the agent may not close its own
  task) holds for every **registered** tool: no name on any run's tool
  surface reaches this code path (`RunContext::withheld` is a dispatch-seam
  check, not a process boundary). That is narrower than "holds
  structurally" — `shell` is universal and ARCHITECTURE.md already documents
  taint tracking's own blind spot inside a command, so a shell-capable run
  invoking `mecha tasks set --status done` as a subprocess is the same known
  gap, inherited here rather than introduced by it. A narrower, second gap —
  `kg_task_update` on an ordinary run's tool surface closing a task
  directly, bypassing `is_fresh_closure` and the appraisal — **closed in
  #112 (2026-08-29)**: `closure_guard::ClosedStatusGuard` wraps the tool on
  every model-facing registry (`setup::build`, inherited by subagents via a
  clone-site belt), refusing exactly a `status` of `done`/`dropped`;
  presence is a trait answer (`Tool::guards_closures`) a wire tool cannot
  fake, and `closure_guard::verify` makes an unguarded surface a startup
  error. The `shell: mecha tasks set` residue *appraises* and so stays
  honest; the remaining true bypasses (an out-of-band graph writer, an
  `[outbox]`-routed release) are documented on `appraise_closure`. `appraise_closure`
  resolves the task's linked session (D9),
  builds an `Appraisal` the same four-step way `mecha sessions appraise`
  does for one session instead of a whole-store scan, and prints the label
  to the owner on stderr — never stdout, which Slack's Done tap parses whole
  as one JSON document. On a non-`Neutral` label it stages exactly one
  follow-up task via `kg_task_create`, composed only from typed fields (the
  label, the channel names, the closed task's **id** — never its **name**,
  which `mail task` can default to a raw email subject line ARCHITECTURE.md
  already documents as untrusted). **The follow-up gate is `done`-only,
  never `dropped`** (`worth_a_follow_up`) — found in review after the first
  cut gated on the label alone, which put a "Revisit" task back on the board
  for a run the owner had explicitly walked away from. §5.4's own framing is
  "the owner took it *anyway*"; a drop is the owner not taking it. **Two
  limitations are disclosed on `appraise_closure` itself, not fixed, and
  worth carrying here too**: the stderr readout reaches only someone who
  typed `mecha tasks set` into a terminal, because every non-terminal caller
  of `set` — `tui::self_cli`, `serve::review::verb`, Slack's Done tap —
  discards stderr on success; the trigger covers all four surfaces, the
  readout doesn't. And staging is not atomic — two closures of the same task
  landing together (a Slack tap and a TUI keypress within the same
  `is_fresh_closure` window) can both stage a follow-up, which is a
  different bug from the retry-duplication one review caught and fixed
  below; this one is a known, bounded gap (an extra advisory board item,
  never a lost or corrupted task), not yet closed.
- **§6.2, the readout surfaces.** `mecha_core::appraisal::live` is a new
  per-*run* sibling to `of_session` (which is per-*session*, for §5.4), and
  scopes interventions to one run's own message range (`run_started_at`)
  rather than the whole conversation, so an intervention from an earlier
  run in a resumed session never bleeds into a later clean one's reading.
  **It passes no drafts at all, on purpose** — `OutboxItem` carries
  timestamps rather than a message index, so there is no
  `run_started_at`-equivalent boundary to scope them by, and including
  every session-wide draft (the bug as first written) let a draft edited
  or sent clean turns *earlier* silently override a later run's own
  outcome. Three channels, not `of_session`'s four. **It reads a compacted
  run as `Neutral` outright**, not a
  partial signal computed with the interventions dropped — found in review
  that the naive version was backwards: `affect_of` reduces magnitude-first,
  so dropping a masking `Steer` can *unmask* a smaller raw error instead of
  staying silent, producing a louder label than an uncompacted run of the
  same fixture would show. Wired to three surfaces: the TUI status-strip
  badge (`tui/mod.rs`, showing nothing on `Neutral`), a `WireEvent::Affect`
  on `mecha serve`'s SSE stream tinting the web logo (`Chat.svelte` — a CSS
  `outline`, not a fill, after the first cut violated `brand.md`'s "hazard
  amber never fills an area" rule), and a real per-*answer* voice TTS
  `cfg_weight` nudge. The voice half needed one new piece of plumbing: a
  loopback `GET /v1/mecha-affect` route in `mecha-cli/src/voice/mod.rs`
  (cloned out from under its mutex before the socket write, not held across
  it), polled by `scripts/voice/worker.py`'s `LocalTTS` — latched once per
  answer in `on_turn_context_created`, the base `pipecat` class's own
  once-per-turn hook, after the first cut polled once per *sentence* against
  a set-and-overwrite cache and could switch tone mid-utterance. Logs every
  latch unconditionally (not only the ones that change the params), because
  a silently undispatched hook would otherwise read identical to "every
  session is neutral" — confirmed directly against the installed pipecat
  1.7.0 that the hook is real and dispatched, not assumed from this diff.
  **And it lags by one turn on purpose, the same disclosed-not-fixed shape
  as §5.4's two gaps above**: the latch fires while the turn that will
  *earn* the label is still streaming its own answer out, so what a call
  actually hears is the *previous* turn's mood applied to the current
  turn's words. Closing that would mean holding speech until the whole
  answer is known, which defeats the point of streaming — `LocalTTS.
  on_turn_context_created`'s own docstring in `worker.py` says so.

Both PRs together took **nine rounds of automated review** (Claude's own PR
bot plus one independent Codex pass) to reach that state, catching — beyond
what is described above — a percent-encoding bug that made the voice readout
inert end to end, a retry that could duplicate a task, a redundant MCP
server startup on every status change, a path-traversal guard
(`is_bare_path_component`, using `std::path::Component::Normal` rather than
a denylist) for a board-writable field reaching a bare `dir.join` — the same
vulnerability shape independently found and fixed in `serve/board.rs` as the
sibling PR #103 — and, twice, a doc comment asserting something about the
code (once about the compaction guard's own safety direction, once about
whether `board.rs`'s copy of the join was already guarded) that turned out
to be false against the tree. Full detail belongs in `HISTORY.md`, not
repeated here.

**Open now:**

- **2026-09-02 — the appraisal review, phase A, phase B and the prediction
  record are on `main`** (#140 at `15c628d`, #141 at `49166e3`).
  `docs/APPRAISAL-RESEARCH.md` is the review (corpus measured live, two
  literature passes, a ranked change list); its §3.1, §3.2, §3.7 and §3.11
  carry *Built* notes naming every symbol, and `ARCHITECTURE.md`'s
  goal-system section carries the invariants. The finding that reordered
  the plan: the label gated on `controllable` (a paid replay) and discarded
  the sign every error carries — 22 owner-rejected drafts all read
  `Neutral`. Built: `appraisal::Valence`/`Readout`/`live_readout` shown on
  the TUI badge and a Slack context line as a number and on the web chip
  as a two-sided bar (the owner's per-surface ruling); `session::SessionKind`
  on the meta record with `MECHA_SESSION_KIND=test` and `--kind` /
  `--include-tests` on the three corpus readouts (46 of 143 appraised
  sessions were dev smoke runs — old rows stay unknown); a ceiling as the
  owner's own limit (`Agency::Owner`, label `Neutral`, `-0.5` on the
  valence) with `Appraisal::cut_short` keeping the closure follow-up alive;
  and the record half of the audit lane's §3.11 spec — `TodoItem::{expect,
  check, expect_calls}`, the frozen check with a tamper echo,
  `step::CHECK_TRACE`, `Work::{checks_declared, checks_passed}`,
  `Finding::CheckFailed`, `RunStats::{checks_declared, checks_passed}`, a
  failed check signed `-1.0`/`Own` in `of_session`, and
  `learning::Trigger::Mismatch` as a wire word nothing fires yet. Re-read
  the same day with the branch binary: **18 of 143 sessions signed,
  `+12.0 −19.5` across them, label `neutral` on all 143.** Four owner
  rulings recorded in the research doc and this session: valence per
  surface as above; cancel-then-re-prompt **is** a steer; a model-judged
  follow-up **may** be a channel; charter lines **may** carry owner-written
  sensors (with the seven containments §3.6/§3.11 of the review's reply
  name — never a `Metric`, never in the prompt, id-join attribution, doctor
  reports saturation, the editor shows the reading; designed since at
  `GOAL-SYSTEM-DESIGN.md` §11.1). **Open, in order:** the `Interrupted`
  split (parked vs cancelled; `questions.rs`'s cancel is the one park
  site; unblocked now that #139's lenient `stop_cause` read is on `main`)
  and cancel-and-re-prompt as an intervention; phase B's leftovers — the
  three trajectory counters and the trigger read receipt; phase C — firing
  `Mismatch` (one per step, three per run), the reflection that cites turn
  ids, the next-turn prior and the per-kind retrieval prior; charter
  sensors, from the design section; the tamper count folded into
  `RunStats`; and the experiments and ablations the owner asked for now
  that this round has landed (`EXPERIMENT-DESIGN.md`, structural switches
  forced off under `mecha eval`, never a prompt). The corpus numbers in
  the rest of this section (119 of 120, 120 of 459) are the rung 7
  measurement and are superseded by the valence read above for any
  decision about what to build next. **Deploy note:** `WireEvent::Affect`
  is now sent for a `neutral` label with a signed valence, and a page
  served from a stale `web/dist` sets the chip to `ev.label`
  unconditionally — it will read a literal `neutral` until the bundle is
  rebuilt (update skill surface 1b).
  **Phase B followed the same evening, as #141**: `SessionRecords` with a
  per-store short flag that lands on the record as `Appraisal::partial`,
  `Channel::Commitment` (questions answered or abandoned, a triaged request
  closed with nothing sent, a run that shortened the owner's queue — net of
  what the owner gave up on, `Depth::given_up`), judged follow-ups from
  `reflections.jsonl` as `Intervention` errors on clean provenance only,
  `guilt::with_backlogs` folding level, delta and relief from one pair of
  reads with the relief in its own field. Re-read after review, before the
  give-up counter: **27 of 144 sessions signed, `+16.5 −34.5`**.

- **Rung 7's measurement came back, and it decides what to build next.**
  `mecha sessions appraise` over the live store: 459 sessions read, **120
  appraised, 119 signed goal errors recorded across two channels, and 100% of
  the labels neutral** (2026-08-27). Eleven of the errors are positive — a
  draft the owner read and sent unchanged — which is the one signal in this
  system that says something went well and had never been counted anywhere.

  Nothing is broken: every label that could have fired needs a dimension
  nothing measures, and `appraisal.rs` names which rung buys which. What the
  corpus adds to that table is a **build order**. A counterfactual verdict is
  what gives an intervention error a label at all, and interventions are 102 of
  the 119 (**but see reach, below — the operative figure is 13**); the charter
  — §14's rung 10 — buys only the eleven positive ones. So
  if the readout is the goal, the probe is the cheaper half and should come
  first. That is a change to §14's order, argued from a measurement rather than
  from the design.

  **Three things narrow that, found on 2026-08-27 by building the probe. Read
  them before acting on the paragraph above.** They are separate quantities and
  were fused in the original claim:

  1. **Reach.** A probe can only address a `steer` or a `denial` — a
     `followup` is a later user turn, and removing it does not leave a run that
     would have got there anyway, which is why `validate` reaches followups
     with a judge instead. Most interventions are followups, so the probe
     addresses **13 of the 102**, not all of them. A structural ceiling, not a
     budget.
  2. **Yield.** Of those 13, **12 come back inconclusive** — the replay
     departs before reaching the intervention. See the validation section
     below; the cause is found and the fix is in, but it recovers nothing
     already recorded.
  3. **Neither branch of the comparison was ever measured.** The charter's
     eleven positive errors do not become `Pride` just because a charter
     exists — `Pride` is *against a charter line*, and **no session in the
     store names a goal at all**. `GoalError.goal` is always `None`, so
     frustration is structurally unreachable too. The reordering may still be
     right; it was argued from one number that was never checked and one that
     was wrong.

- **`serves:` has never carried a value in production, including in the runs it
  was built for.** 112 of 120 appraised sessions wrote a plan; **0 named what
  it serves** — and 15 of them are delegated task runs (`task-d063fe34` ×11 and
  two others), which is the case §2's *one list, one task* argument is about.
  The field is in the `todo` schema marked "Optional", and nothing in a
  delegated run's seed tells the model the task uid or asks for one. The seed
  already builds its prompt from the board record, so this is a sentence in
  `tasks.rs` rather than a design question — and until it lands, two of the
  eleven affect labels are unreachable for a reason that is neither the probe
  nor the charter.

  **Fixed 2026-08-27, and the diagnosis is narrower than the paragraph above
  states.** `TodoTool::description` already told the model to pass `serves`
  when work serves a task — all 15 delegated runs above carried that
  instruction *and* this task's own id on the seed's `Id:` line, and still
  wrote nothing. So the gap was not "nobody was ever told"; it was that
  nothing bound the schema's generic reminder to *this run's specific* id.
  Both delegated postures (`work_prompt`, `discuss_prompt` in `tasks.rs`) now
  do that binding explicitly, using the id already in the seed — nothing new
  is fetched or paraphrased. **Caught on review** (#92): the first cut of
  this sentence repeated the "nobody was told" framing the code comment and
  CHANGELOG entry originally used, which the schema text already
  contradicted.

  **Whether the narrower fix moves the number is unmeasured, and the 120
  sessions already appraised stay at 0 regardless** — this changes what
  future delegated runs write, not what past ones did, so the next honest
  read of `serves:` coverage needs sessions recorded after this lands, same
  as the surface fix below.

  **And `sessions appraise` reported that zero by construction**, which is the
  sharper half: it passed `&[]` for goals unconditionally, so the command built
  to measure whether the labels were degenerate could never have reported
  anything but zero goals. Absent and zero conflated inside the instrument.
  **Fixed and confirmed in `main` as of `acceaea` (part of #88, merged
  2026-08-27T19:47:23Z)** — verified by reading `commands/sessions.rs` at
  the current tip rather than trusting an earlier claim about it: `appraise`
  now calls `TodoTool::plan_from_transcript` on the loaded messages and
  passes the real goal through. This superseded an earlier, narrower fix
  (`plan_from_transcript` on `feat/appraisal-probe` at `efaa4b9`) written
  independently by the same review pass that fixed the `Frustration`
  mislabelling bug below — the two landed together because both touch
  `of_session`'s goal handling. **#91 has since merged too** (`f0ca8ca`,
  2026-08-27T20:41:57Z) — `efaa4b9` is an ancestor of `main` now — but its
  `named_a_goal` counter and `· 0 named a goal` print line did not survive:
  confirmed absent from the current tree by grep, same as before #91
  merged. Presumably dropped resolving overlap with #88's independent fix,
  since both touch the same call site. **Rebuilt and shipped in #111
  (2026-08-29)**: `appraise` derives `named_a_goal` from the assembled
  appraisals and prints the `N of M named a goal (\`serves:\`)` line, with
  a matching JSON field — verified by asking the installed artifact
  (`mecha sessions appraise --json` carries `named_a_goal`). `serves:`
  coverage since the seed-binding fix is *still unmeasured for delegated
  runs* — as of 2026-08-28 evening, zero delegated runs had happened since
  it landed, so the honest read still waits on one.

  **A second, more consequential bug was fixed in the same pass, and already
  landed** (`acceaea`, #88's review — this is a record of what shipped, not
  an open finding). `of_session` had cloned `goals.first()` onto every error,
  so `Frustration`'s actual definition — *repeated negative error, one goal,
  self-agency* — degenerated to *any two negative errors, once a session has
  a goal at all*, and the check ran before agency/exposure and could mask a
  more informative label (a turn-ceiling failure alongside an embarrassing
  rewritten draft would have reported `Frustration` and hidden both facts).
  The fix is in `affect_of` now: the `repeated` check is filtered to
  `Agency::Own` and grouped by `error_kind` (`Channel` plus, for a
  `Cite::Counter`, the counter's own name), which restores the real
  definition and keeps the rank comparison from burying an exposed error.
  Recorded here because it changes what "the labels resolve now" will
  actually mean the next time `sessions appraise` runs with goals present —
  which is what this PR's own `serves:` seed fix starts making possible —
  and because it was latent until something populated goals widely enough to
  reach it, so a reader might otherwise assume it is still open. It is not;
  check `affect_of` before re-deriving it.

  **Two corrections to that, from the lane building the probe** (`mecha-80`,
  2026-08-27; its probe-specific work is **#91**, merged (`f0ca8ca`) — the
  counts below are theirs and are not independently reproduced here):

  - **The probe does not reach all 102.** Most interventions are `followup` —
    a later user turn rather than text riding with tool results — and there is
    no counterfactual to drive, because removing a later turn does not leave a
    run that would have got there anyway. That is why `validate` reaches
    followups with a judge instead. A structural ceiling, not a budget, and it
    shrinks the probe's share of the readout accordingly.
  - **And today the reachable ones cannot run either** — see the next item,
    which is the larger finding.

- **`mecha validate`'s steer and denial probes have never been able to run on
  an interactive session**, and that is why `validations.jsonl` does not exist.
  `replay_run::replay_registry` bails on any recorded tool name the live
  registry lacks, and `ask_user` is registered *only* by a front-end that owns
  a human — `setup::prepare`'s `interactive` flag picks the approver and does
  not register it (`setup.rs`, the `TerminalApprover` branch). The coupling
  closes: a probe needs an intervention, interventions happen in interactive
  sessions, interactive sessions carry `ask_user`, and the replay refuses to
  build. **Verified against the store: 246 of 408 sessions with a recorded
  tool list carry `ask_user`.** The nightly has been running `validate` and
  every steer/denial probe has skipped, which its `skipped` counter reports and
  nothing reads as *this whole class is unreachable*.

  The contained fix is a **spec-only stub under `OnDivergence::Stop`**: nothing
  executes in that mode, so the registry exists to reproduce the surface the
  model *saw*, and a stub that errors if actually called restores that without
  changing any executed behaviour. It must stay conditional on the mode —
  under the others tools really do run, and there the bail is correct.

  **This paragraph is stale — the stub is built and both callers already use
  it, verified 2026-08-27 by reading `probe.rs` on `main` rather than trusting
  this note.** `probe::drive_arm` — the one function both `mecha validate`
  (via `prepare_probe`) and `mecha sessions appraise --probe` (via
  `prepare_probe_at`) call to actually run an arm — builds its registry with
  `Some(&crate::setup::surface_only_registry())` as the `ask_user`-shaped
  fallback, under `OnDivergence::Stop`, exactly as described above. Landed as
  part of #90/#91. What is still unmeasured is not whether the fix exists but
  whether it *changes the numbers* — nobody has re-run `mecha validate` since
  it landed to confirm `validations.jsonl` now gets written for a steer/denial
  probe that used to silently skip.

  **This caution held for rung 10, which shipped anyway — on purpose, and
  without contradicting it — and rung 8 then shipped against it too, this
  time by explicit ruling rather than by needing nothing from the label.**
  §8's prioritised replay still keys off affect being non-degenerate and is
  still unbuilt for that reason. Rung 10 (PR #100, 2026-08-28) shipped ahead
  of rung 8 in build order specifically because its two pieces — the
  charter and the anticipated-guilt sensor — depend on neither the probe nor
  the label: the charter is a static, user-authored prompt block, and the
  sensor is deliberately unconsumed (see the summary paragraph above). What
  still waits on the label from rung 10 is charter-driven `Pride`/
  `Frustration` — that half is genuinely unbuilt. **Rung 8 (PRs #99, #103,
  2026-08-28) is different**: it does read the label directly (§6.2's three
  readout surfaces show whatever `affect_of` currently derives, degenerate
  or not, and §5.4's follow-up gate predicates on it), so this caution
  applied to it exactly as written — and it was built anyway, on the ruling
  that the mechanism earns its place independent of today's label, not that
  the corpus argument was wrong. Concretely: on a corpus that is 119/120
  `Neutral`, the TUI badge, the web tint and the voice nudge will show
  nothing on nearly every run today, and the follow-up gate will stage a
  follow-up almost never — both are the honest readout of a label the probe
  (§14 item 7) is what would actually move, not evidence rung 8 was built
  wrong. Whether rung 9's own affect-adjacent pieces (§10's review-queue
  salience) hit the same wall is not verified in this entry — see whoever
  shipped #97/#98 for that.

  **Narrowed 2026-08-28, by rung 9's own first piece — see that bullet
  below.** This caution is about *consuming* the label: reordering a queue
  or a replay pass on a value that mostly resolves `Neutral` would optimise
  for nothing. It does not cover *recording* it, which carries no such
  risk — `mecha distill` now stamps `meta.affect` on every pushed pkg
  episode regardless. The consumer this caution guards against (pkg
  actually reordering its review queue on that field) is still unbuilt.

- **The quarantined appraiser (§5.1) shipped 2026-08-27** — no tools, no
  conversation, typed output, offline via `mecha sessions appraise --appraise`
  (`mecha-core/src/appraisal.rs`'s `AppraiserEvidence`/`appraise_with_model`/
  `apply_appraiser`, `mecha-cli/src/appraiser_pass.rs` for the budget and
  tally, same shape as `--probe`). It reads one already-built `Appraisal`'s
  numbers only — never the transcript, an intervention's text, or a draft's
  body — and returns one more signed `GoalError` (`Channel::Appraisal`,
  `Cite::Appraiser`) or "nothing further", which is the ordinary, correct
  answer and is tallied apart from a budget running out. Smoke tested live
  against 3 real sessions (all came back "nothing further" — too small a
  sample to be the corpus measurement rung 7's observation half already took
  at 120 sessions); running `--appraise` at that scale is the natural next
  step, and it is what decides whether the appraisal **store** now earns its
  place, per this rung's own build note in `GOAL-SYSTEM-DESIGN.md`.

  **The model half of step appraisal shipped 2026-08-28, closing rung 7.**
  Unlike the appraiser it is a live concern, not an offline pass: `agent.rs`'s
  own loop reads a new `ToolCtx::step_escalation` slot (`compact_requested`'s
  exact shape) that `tool/todo.rs`'s `Tracked` writes into when a landed
  step's span is either a clear outlier against the plan's other completed
  steps (≥3× their mean, floor 6 calls, ≥2 siblings to compare against) or
  claims a verification its calls never made (`step::looks_like_verification`,
  a `Work`/`Span` counter beside `calls`/`failed`/`refused`). One quarantined
  call (`Agent::escalate_step`, routed through the same cancellable
  `self.complete()` `compact()` uses, so it honours the run's cancellation
  token and its tokens land in `RunStats`) settles it; the verdict is a closed
  `accept`/`revise_plan`, folded into the turn via `append_user_text`
  exactly where boredom's notices land. The step's own text rides in the
  call — it is this same model's own prior plan output, already trusted
  in-context every turn, unlike the appraiser's numbers-only evidence — but
  the model's free-text reasoning about it never reaches the transcript:
  logged at `debug` only, on `frontdoor`'s "a paraphrase of an injection is
  the injection rearranged" rule, since a model's paraphrase of text it just
  read is the same risk arriving through the one channel that re-enters
  context. Off by default (`[agent] step_escalation`,
  `--no-step-escalation`, forced off under `mecha eval`) on
  `compact_at_tokens`'s posture — the pre-filter's thresholds are argued,
  not measured, and this is the first spend inside the run itself rather
  than a CLI pass. Verified live against the local model: a deliberately
  oversized step drove the call, which correctly judged the size
  intentional rather than a decomposition problem, and confirmed by reading
  the recorded transcript that the reasoning never landed in it — only the
  tally would have.

  Still no store for either channel, on the same argument: what either
  produces is a verdict that needs keeping and not an appraisal, and the
  assembled record stays derivable regardless. `runlog`'s rule and a
  correction to §10.

- **Rung 6 shipped with two gaps that are stated reasons, not sequencing.**
  Both are named in `GOAL-SYSTEM-DESIGN.md` §14 rung 6 and in the modules
  themselves, so neither is rediscovered as an oversight.
  - **Step appraisal reads two of §5.5's five signals** (`step.rs`). The two
    shipped are facts about the span — nothing was attempted, and the last
    attempt did not succeed. Of the three absent, one turned out to belong to
    boredom and is built there; the other two — a span far longer than its
    siblings, a verify-shaped call that passed — each need a threshold nobody
    has measured here or a guess about what a call meant, and the escalation
    that would settle an ambiguous span is rung 7's.
  - **Boredom's rung 2 — consult — is unbuilt** (`boredom.rs`), because
    neither half can be reached: a §7.4 marker does not exist at all, and
    while a skill does, nothing in the `Tool` trait identifies the tool that
    loads one. `narrows_surface_to` answers `None` until a skill is already
    loaded, so it recognises the state the notice exists to escape only after
    the escape has been taken. `Tool::runs_a_fresh_conversation` — added for
    rung 3, fourth in the family with `carried_state`, `fixed_workspace` and
    `narrows_surface_to` — is the shape closing it would take.

- **Nothing has measured rung 6 yet, and the sensor is now there to do it.**
  `RunStats::boredom_notices` and `Corpus::boredom_rate` landed with it and
  `mecha sessions health` prints a `went nowhere` line; as of 2026-08-27 it
  prints a dash, because no run in the store predates nothing — the sensor is
  newer than every row. **Every threshold in `boredom.rs` was argued rather
  than measured** — three identical outcomes, six, three notices a run — and
  the same is true of `step.rs`'s silence on the common path. What to look for
  once a corpus exists: a boredom rate near zero means the constants are too
  slow to be worth the mechanism, and one near one means the transcript is
  mostly the harness talking about the harness. Neither is visible any other
  way.

- **Rung 9's first two pieces shipped 2026-08-27/28** (§10, §10.1 —
  `appraisal::for_session` (`mecha-core/src/appraisal.rs`) is now the
  one assembly `mecha sessions appraise` and `mecha distill`'s episode
  tagging both call; `distill::upsert_args` (`mecha-core/src/distill.rs`)
  writes `meta.affect`/`meta.goal_errors` onto the pushed pkg episode, not
  gated on taint, and the quarantined `Distiller` now also extracts
  `Surprise { predicted, actual, about }` — gated like a correction, printed
  by `mecha distill` for a human to chase with `mecha gossip --entity
  <about>`, never auto-run. See HISTORY's 2026-08-27/28 entry for the
  four-round review saga on top of it). **Review-queue salience — the rest
  of §10 — is still unbuilt**: it needs the private
  `personalized_knowledge_graph` repository (a different codebase mecha only
  reaches through the MCP tool surface) to read `meta.affect`/
  `meta.goal_errors` back and reorder pkg's review queue on them. Not
  started, and not scoped beyond `GOAL-SYSTEM-DESIGN.md` §10's own paragraph
  naming it. (Rung 10, the charter, landed as #100; rung 8 landed as #99 and
  #103 — see the summary paragraph above for what shipped.)

**Two things named rather than done**, recorded so they are not rediscovered
as oversights. (The third — context pressure absent from `Homeostat` — shipped:
`peak_prompt_tokens` and `peak_context_pressure` on `Homeostat`.)

- **`/queues` still walks the stores itself.** `backlog.rs` is the shared walk
  and the homeostat uses it, but rewiring `mecha review`'s `collect_queues`
  (`commands/review.rs`) onto it needs either a wider `Backlog` or the loss
  of its per-item detail lines (*"3 drafted with the trifecta armed"*). That is
  a design choice, not a mechanical port. Doctor is deliberately further out —
  its per-store error isolation is load-bearing.
- **`/slots` is not sampled.** It is the best load signal available —
  occupancy directly rather than by proxy, and a second witness to prompt-cache
  reuse that `cache_lens` cannot get — but nothing reads it yet, and a sensor
  with no consumer should not put an HTTP call in the path of every run's
  start. `homeostat.rs` says so at the point where it would go.

**A session resumed from disk starts unanchored.** The pressure series now
survives a run boundary, so chat and the TUI predict from the first turn — but
a transcript records what runs cost *in total* and never what the last request
weighed, so a resumed session predicts from its second turn on. Closing it
means recording a last-prompt-size beside `peak_prompt_tokens`; cheap, and
nothing depends on it yet. Verified still open 2026-08-27: no `last_prompt_*`
field exists.

**And a step whose span crosses that same boundary gets no reading at all.**
`step.rs` differences the run's own trace, which restarts with the run, so a
mark taken in an earlier run is *unmeasurable* rather than empty — deliberately,
since the arithmetic alone would saturate to zero and announce the null step on
the commonest shape chat has. It is the same missing anchor as the item above,
one mechanism over, and a conversation-scoped work counter would close both.

**The compaction threshold is still a default nobody chose, and only half of
it is fixable by machine.** `AgentConfig::compact_at` derives two thirds of
the window, so the live providers sit at 173,015 (262,144) and 21,626
(32,768). Measured at `max_tokens = 8192`, the worst un-priced tail is 8,192
reply tokens plus the output budget — 16,192 at 262k against 89,129 of margin
(5.5× oversized), and 12,288 at 32k against 11,142 (**negative by ~1,100**).
Predictive compaction fixes the 32k side, because that overflow happens
between the check and the next request whoever decided. It may not fix the
other: §7.3 forbids relief compacting late, so no disposition may argue for
holding more context, and lowering the 262k threshold stays a person setting a
number. Left unset deliberately — but a reader should know it is a default
rather than a decision, and that the arithmetic above says the constant is
wrong in *both* directions.

**Not to be re-litigated** (all argued in the design): decomposition runs
downward and appraisal upward; the affect label is derived, never
self-reported; affect may only *narrow*, so a disposition is a monotone layer
above a structural check and never a replacement for one; and affect is a
priority function, never an objective.

### Validation — the probes run, grade, and feed an autonomous loop

The 2026-08-27 arc got the probes *running* (the `surface_only` registry for
front-end-only tools, `surface.rs` + `tools_hash`); the 2026-08-29/30 arc got
them *grading*. The full story of both is in HISTORY under those dates. What
is true now:

- **Counterfactual probes branch** (`counterfactual::branch_at` +
  `replay_run::drive_branch`): the recorded prefix is resubmitted verbatim,
  steering text stripped, and the model samples only the continuation — so
  "diverged before the steer point", which ate 11 of 12 steer probes on the
  first full pass, is structurally impossible. Measured after: **0
  inconclusive, 12 graded** (2026-08-29), and the first unattended nightly
  (2026-08-30 03:30) trace-graded all 3 of its steer probes.
- **The surface rebuilds from the blob now.** The rebuildability half that
  was open shipped: a recorded tool nothing today can construct becomes a
  spec stand-in from the `SurfaceStore` blob, and a recorded spec **wins
  over a live tool's reworded description** in the non-executing modes, so
  fidelity reads `Matches` instead of `Differs` wherever the blob exists.
  Recordings from before `tools_hash` stay unrecoverable, as `surface.rs`
  always said; the four reflections stranded on them (`google__*`, `pkg__*`
  pre-rename) are dropped with reasons, and `select_probe_corpus` now
  honours `dropped_at`.
- **Replay wrappers narrow `external_send` under `Stop`/`Error`** — a
  replayed "send" sends nothing, and before this the trifecta interlock
  fired *inside* three of twelve arms, desyncing the cursor and grading a
  harness block as the model's failure.
- **The NoGo path has fired as one motion** (2026-08-30):
  `scripts/retirement-drill.sh` seeds a probationary bad rule into an
  isolated world and drives real probe passes to a conviction at the
  probation leash of 2 — and its first run found that the leash had been
  structurally unreachable (probation released on the very coverage its
  convictions ride in; fixed as
  `release_probation_when_measured_clean`). HISTORY has the story under
  2026-08-30; run the drill after touching validate, the bisection,
  tallies or the retirement scan.

**Open:**

- **The followup probe answers with tool calls it cannot make.** It re-asks
  the corrective turn with no tool surface, so on the 2026-08-30 nightly 5
  of 7 followups were `inconclusive` — the `is_gradeable` span check
  correctly refusing to judge a `<tool_call>` body as an answer, but that
  is half the judged corpus producing nothing. Giving that probe its
  recorded tool surface (specs only, nothing executes) is the candidate
  fix.
- **A steer pass is call-for-call.** Tracking the whole steered continuation
  under sampling biases toward `Fail`/`Fail`; both arms share the bias so
  the ledger's comparisons stand, but if improved/regressed stay rare
  across nights, grade a bounded window after the steer point instead.
  Decide from ledger data, not preemptively.

### Cheap, and worth doing first

- **Rule on the `ask_user` decline wording** (measured 2026-08-30,
  deliberately unadopted — the source is restored to control). A/B, 5 runs x
  3 ambiguity cases per arm: dropping "If the task can be done without it,
  do it" and adding "do not search for it again" saved 3 turns on
  `ambiguous-rate` (13→10, zero variance) with 0 invention failures in all
  30 case-runs — but did **not** make the case pass (10 > the ≤8 check), so
  the wording is *a* cause of the thrash, not *the* cause. Unadopted because
  the control string is itself the winner of a prior measured A/B whose
  losing arm made the model invent a contractor name and rate, and 15 runs
  is weak evidence of no regression on a rare failure. Luke's call, unmade
  (`mecha-core/src/tool/ask.rs`, the decline text in `call`).

- **Rule on harness candidate `hc-20260828T033309-d83b`**
  (`context.auto_compact=true`, staged since 08-28, flagged unappliable —
  the key is outside the closed override set). Reject it, or decide the
  closed set grows; the loop rightly refuses to decide. `mecha harness
  list` shows it, and the diagnostician re-derives it nightly and is
  refused by canonical-spec comparison, which is the brake working but also
  a nightly reminder that a human ruling is owed.

- **Decide whether replayed reasoning stays unbounded.** As of 0.1.2 the
  OpenAI-compatible backend sends every `Block::Thinking` back, which is what
  the model's own template expects and what took the empty turns from 6/6 to
  0/6. The cost is bounded in *compute* — the prefix cache absorbs it, measured
  at better than 95% reuse — but not in *context*: on the 08-10 run the model
  averaged ~930 output tokens a turn, most of it reasoning, so an 80-turn trial
  carries roughly 75k extra tokens by the end. Against a 173,015 threshold that
  is survivable and it may bring compaction forward on long trials, trading a
  cheap failure (a wasted nudge turn) for an expensive one (a lossy summary).
  **The measurement that decides it is `expect.min_compactions` and the
  compaction counts in the relaunched benchmark's transcripts**; if those jump
  on the long trials, bound preservation to the last N assistant turns. Left
  unbounded on purpose until there is a number: it is the behaviour the model
  was trained with, and guessing a bound first would be tuning against nothing.
  Note this only became affordable at 262,144 — at the old 32k it would have
  been fatal, so the window raise and this fix are coupled.

- **Decide what the compaction threshold should be, now that it moved 8x on
  its own.** `AgentConfig::compact_at` derives `COMPACT_FRACTION` (0.66) of the
  window when `[agent] compact_at_tokens` is unset, so raising `-c` from 32768
  to 262144 took the threshold from 21,626 to **173,015** as a side effect
  nobody chose. Nothing is broken by it — prompt caching means a growing
  transcript is only prefilled at the delta — but a cache *miss* at that depth
  costs ~120s of prefill before the first token, and a model's useful context
  is generally shorter than its trained one, so a transcript allowed to reach
  173k may be answered worse than one compacted at 100k. **The arithmetic that
  should decide it is in the goal-system section above**, which measures the
  margin at both live window sizes and finds the constant wrong in *both*
  directions — and explains why only one of those is a disposition's to fix.
  Stated once there rather than twice, because two statements of an open
  question drift apart.

- **Re-baseline `ambiguity` and `long-horizon` at k=5.** No scorecard in
  `results/` records `runs: 5` outside the compaction arc, and these are the two
  tags whose single-run numbers move.

### Structural gaps

- **MCP resources are not implemented.** `mecha-core/src/mcp.rs` advertises
  `"capabilities": {}` and speaks only `tools/list` and `tools/call`. No
  `resources/list`, no `resources/read`. Whenever this is closed, resource
  content must be marked `.from_outside()` — it is third-party text by
  definition.
- **HTTP/SSE MCP transports.** `McpServerConfig` carries only `command`/`args`,
  so every server is a local process. Worth knowing before building it: none of
  the process confinement — `env_passthrough`, `sandbox`, per-server `network` —
  means anything for a remote endpoint, so the security story has to be
  rewritten rather than inherited.
- **Subagents cannot narrow the jail.** `SubagentProfile` narrows tools,
  `max_turns`, model, provider and `trusted_output`, but has no workspace field;
  `subagent.rs` passes the caller's `ToolCtx` verbatim, so a child's jail is
  always exactly its parent's. A subagent that should only read one directory
  cannot be expressed.
- **Per-command approval policy.** `ModeApprover::approve` takes the tool input
  and ignores it (`mecha-core/src/tool/mod.rs`), branching only on mode and
  `read_only`. So `shell: ls` and `shell: rm -rf` are the same decision. There
  is no per-command rule surface in config to hang this on yet.
- **Structured output has no provider abstraction.** The `Provider` trait exposes
  only `id`/`default_model`/`complete`. GBNF, `guided_json` and
  `output_config.format` are all spellings of the same idea and none is reachable.
- **A seccomp layer on the sandbox.** Landlock itself shipped 2026-08-16
  (`Backend::Landlock`, `sandbox.rs` — no-privilege file confinement, hard
  ABI-3 floor, a preflight that plants a home file and requires the confined
  read to fail, and deliberately no `external_send` narrowing because UDP is
  unrestrictable). What the research scoped and remains unbuilt is syscall
  filtering: no `seccomp` anywhere in `mecha-core/src/`. Also note this box
  stays on docker for `shell` anyway — docker's `network = false` earns the
  interlock relaxation that Landlock, honestly, cannot.
- **In-run verification / a convergence primitive.** Nothing in `agent.rs` tests
  a post-condition; there is no runtime "is it done yet". The research's own
  answer is the starting point: it has to be a command's exit code, not a
  model's opinion. `compact_validate` is the only in-run verifier that exists.
  Narrowed 2026-08-19: `RunOutcome::ended_on_failed_call` (`agent.rs`) now names
  the *post-hoc* case — a run that stopped of its own accord with its last call
  failed — which is the silent-failure shape a judge cannot catch. It is a
  report, not a convergence test; the gap above is unchanged.
- **Programmatic tool calling** (a `code` tool that calls other tools from inside
  a program). Two hazards to solve first, both named in the research: taint must
  update *within* a running program, and approval for a program that makes
  thirty calls is not thirty approvals.

### The learning system

The arc is complete, **ungated, and running per session close** (2026-08-30,
PR #122 — Luke's 2026-08-19 ruling built): `learn --auto` measures by
counterfactual probe and disposes without staging — regression refused,
clean applied, ungradeable applied on probation with the 2-regression leash
(carried across consolidations, released only by the ledger). Retirement
applies nightly from the ledger with no human. First consolidation under it:
28 reflections → 12 live rules, after 25 days at zero. What follows is the
provenance history and the refinements still open. It had been producing
rules through the gated path since 2026-08-23. The provenance starvation measured on 2026-08-22 was resolved by
moving the evidence to the trusted side of the ruling rather than loosening
it: `learning::evidence_for` (`mecha-core/src/learning.rs`) hands the
reflector only user-typed words plus registry-owned tool names when coverage
cannot prove clean, `Evidence::UserTurns` records what was shown, and the
fail-closed gate is untouched — there is still no knob that lets a
full-context reflection out of an untrusted conversation. The
`--remine-untrusted` backfill recovered 11 lessons from the archive and the
store's first-ever proposal (5 rules) is staged; doctor's starved-learner
finding has cleared. The arc is in HISTORY under 2026-08-23.

What is missing beyond that is refinement:

- **The sliding window of recent raw reflections never shipped.** Prompt assembly
  chains user rules then consolidated rules; the third leg — a window of recent
  unconsolidated reflections — was designed and not built.
- **Rules are scoped by domain, by run, and now by tool set — but nothing
  delivers one mid-run.** `Rule::scope` exists (PR #168, merged
  2026-09-04) and `rules_carried_for` loads a scoped rule only into a
  run whose registry matches it. What is still missing is the §17.4
  *Delivery* half — one line on a tool's result the first time a recorded
  condition recurs, ruled off-by-default in `GOAL-SYSTEM-DESIGN.md` §17.7
  item 2 — plus consolidation's widening across sub-regions and the
  per-region validation budget; without widening a rule stays as narrow as
  the batch that learned it.
- **Rules that are facts should graduate to pkg.** No classifier routes
  fact-shaped rules into `kg_upsert` as staged candidates; `distill.rs` pushes
  episodes only.
- **The positive signal now has one reader** — the appraiser
  (`appraisal.rs` folds `WritingOutcome::SentUnchanged` as positive
  evidence) — but the *learner* still ignores it: consolidation mines only
  edited-then-sent items, so "this voice was right" never reinforces a rule.
- **LEAP-in-production.** Rumination mines interventions only. Learning from
  graded eval cases — sampling known-outcome examples rather than waiting for a
  correction — was ported in design but not in code.
- **The correction-rate query shipped** (`mecha learning-report`, plus
  `/api/settings/learning-report` and the web trend pane) — what remains is
  *reading* it: the pre-cutover baseline is thin, so the trend needs a few
  weeks of nights before "are interventions per session going down" has an
  answer worth acting on.
- **The CIPHER tier** — per-context preferences, embedded and retrieved top-k —
  exists as a comment and nothing else.
- **A `/learning` TUI view.** The store is files by design so it can be read
  without tooling, but nothing surfaces it in the interface.

### Triggers

- **Deadline escalation, on top of the task surface that now exists.** The
  `/tasks` modal and `mecha tasks` shipped 2026-08-20 — direct capture, status,
  scheduling — so *direct capture* and *the modal* are done and the store is
  the graph's board, reached over `kg_task_*` rather than built in `~/.mecha/`.
  What that arc did **not** build is the part that turns silence into a state:
  no task is created from an inbound request's SLA, none from a **commitment
  the user made** (extractable from released outbox items, where mecha already
  knows what went out), there is no `Origin` per task so nothing can decide
  which tasks may escalate unattended, and nothing escalates. Recurrence still
  wants `cron.rs`. An unanswered message is still invisible; there is now
  somewhere to hang the state, and nothing hangs it. Design in
  `PUBLIC-SURFACE-DESIGN.md` §3.2–3.3.
- **Policy questions as a new `proposals` kind — not a third queue.**
  `ask_user` is absent from unattended runs by construction, so a trigger can
  stage but cannot ask. (2026-08-24 sharpened the premise without changing
  the conclusion: `Asker::ask_in` proves a *shared* agent can route a
  question to the right present human — the web surface does it — but
  unattended still means nobody to route to, which is what this item is
  about.) The elicitation that grows autonomy is *policy*
  questions that unblock a class, not per-item approvals. These belong in
  `mecha proposals` beside `rule` and `retirement`, because it is the same
  shape — mecha asks, the user decides, future behaviour changes — and because
  two review surfaces (`outbox` for what leaves, `proposals` for what changes
  behaviour) is a learnable morning routine where three is not. **The
  surface half arrived 2026-08-30 (#126)**: `serve/proposals.rs` +
  `Proposals.svelte` render the three existing stores (harness · rules ·
  graph entities) as one phone pane with read-gated decisions — so a new
  policy-question *kind* would get its cards for free; what remains open is
  the kind itself and the unattended run's path into it. Deliberately
  **not** called an inbox: ambiguous with the user's real one, and this queue is
  capped by design where an inbox is unbounded by definition. §3.1.
- **File watchers.** `Trigger` has no watcher kind. Needs debounce and a
  "what changed" payload injected into the prompt — the injection half got
  cheap on 2026-08-09: the mailbox delivery path (`mailbox.rs`) is exactly a
  labeled, taint-carrying prompt fold, so a watcher's payload can arrive as
  a message instead of needing new loop machinery.
- **Inbound webhooks.** Nothing listens, still. What *was* the interesting
  part — a payload arriving marked untrusted, the interlock applying to a
  prompt rather than a tool result — shipped with messaging on 2026-08-09:
  a webhook receiver is now just another writer into a mailbox with taint
  pre-set untrusted. What remains is only the listener itself and its
  authentication.

### Mail as a surface you work — built; what is open is judgement

**`docs/MAIL-UX-DESIGN.md` is the authority** and its **§7** is where the
remaining questions live. `docs/MAIL-CORPUS-RESEARCH.md` is the measurement
that reordered the plan — **gitignored**, on `OPERATIONS.md`'s split: the
lesson is here, the figures are not.

**Every phase the plan named is shipped.** Phases 1–3 landed 2026-08-18;
**phases 4′ through 6 landed on 2026-08-19 between 11:08 and 18:10**, in
commits that name them by number (`3547cb1` … `a34a1c2`). The arc is in
[`HISTORY.md`](HISTORY.md). Verified against source 2026-08-25:

| Phase | What it is | Evidence |
|---|---|---|
| 4′ pre-filter | Bulk + automated-sender rules ahead of the classifier, only ever producing `ignore`, reading the envelope and never the body | `mail_triage.rs:340` (`prefilter`), `:303` (`PrefilterRule`) |
| Corpus eval | The classifier graded offline against mail whose outcome is known | `commands/mail.rs:333` (`Eval`), `:1307`; `--sample/--seed/--prefilter-only/--out` |
| 4″ tasks + `needs-info` | A thread becomes a board task, or parks naming what is missing | `commands/mail.rs:1937` (`task`), `:2020` (`needs_info`) |
| 4‴ day two | The aged set as a store-side primitive, surfaced by the briefing | `commands/mail.rs:504` (`list`, `--aged --aged-hours --surface`); wired live in `~/.mecha/triggers/morning.toml:59` |
| 5 `/mail` | The queue as a modal, with forward finally bound | `tui/mail.rs:401` `t`, `:403` `n`, `:405` `r`, `:406` `f` |
| 6 corrections | Field-level corrections feeding the few-shot pool *and* `triage`-domain reflections | `mail_triage.rs:600` (`Correcting`), `:626` (`apply_correction`), `commands/mail.rs:1804` (`reflect`) |

`mecha mail score` (`commands/mail.rs:488`) grades the *live* store, which is a
different question from `eval`'s corpus and is deliberately a separate verb.

**Two decisions recorded rather than left open**: tags are mecha's own and never
provider labels, and no mail parser belongs in mecha — the graph already ingests
`email.thread` episodes (`sources/mbox.rs`) with the bulk filter and the
`NEVER_AUTO` guard, so the live path pushes evidence through `kg_upsert` on the
`distill.rs` pattern and lets the graph extract. Push only `respond`/`notify`
buckets. **Front-door routing was the original phase 4 and is deleted**
(2026-08-19), along with `ROUTABLE_TYPES`, `is_routable` and
`Proposed::Frontdoor`. Do not rebuild it: `MAIL-UX-DESIGN.md` §1 has the five
reasons.

#### What is actually open

None of it is a phase. All of it is judgement, and **§7 of the design doc is
the authority** — restated here only far enough to be choosable:

- **The day-two age.** `--aged-hours` defaults to 30. The reply cliff falls at
  24 hours, but a briefing firing the morning after an evening email nags about
  something a working day old. §7.3 leans to one working day, and this is
  measurable against the corpus rather than pickable.
- **How `student-advising` is answered.** The largest category by a wide
  margin, and probably a *substitution* problem rather than an automation one —
  the same handful of questions (prerequisites, petitions, transfer credit),
  which is the profile of something a form or a published answer removes. §7.5,
  and the biggest single piece of the load.
- **Whether `meeting` earns a request kind.** Real volume and the *highest*
  reply rate of any category, which cuts both ways; structurally the greediest
  label, and the booking flow may already cover it. §7.4.
- **Retention on `~/.mecha/mail-triage/`.** Nothing prunes it. `mecha work
  clean` has the policy shape, but an archived verdict is also the eval fixture
  and the few-shot pool, so deleting costs what sweeping the work directory
  does not. §7.6.
- **Whether `t` can point back at the thread.** `kg_task_create` has no field
  for it, so it lives in the name or needs a pkg change. §7.2.

**Local state in no repository**, and the next session will want it:

- The classify timer is installed at `~/.config/systemd/user/`.
- `~/.mecha/mail-triage/` holds **264 records — 222 dartmouth, 42 personal**
  (2026-08-25; personal was 0 until that date), classified across several
  binary generations, so `request_type` is not consistent across the store and
  the taxonomy changed again on 2026-08-19. It wants one `mecha mail classify
  --limit 50 --force` sweep before any measurement is taken from it — note the
  flag no longer names an account, and `--limit` is **per account**, so the
  sweep is twice the size it used to be. Five of those records carried
  `proposed: frontdoor` and now read as `none` — by design, not by damage.
- **The corpus is at `~/.mecha/mail-corpus/{dartmouth,personal}.jsonl`**,
  owner-only, outside the repo, `*.jsonl` gitignored so it cannot be staged.
  both accounts complete as of 2026-08-19. Do not re-fetch to re-run an
  analysis; do re-fetch if the window needs extending. The personal half was
  fetched twice — the first was truncated by a 500-per-month cap, now fixed and
  reported when reached.
- **The personal account is in the nightly as of 2026-08-25**, reversing the
  entry that stood here. The measurement behind the old conclusion was right —
  personal mail is far more machine-generated and carries almost no
  correspondence needing an answer — and the conclusion drawn from it was
  backwards. Machine-generated is exactly what the prefilter disposes of for
  free: the first both-account sweep read 100 threads, and **47 of the 51
  candidates were disposed without a model**, 4 reaching the classifier. The
  account excluded for being expensive is the cheap one. What remains true is
  the *reason* it was excluded originally — the Google grant's 7-day expiry —
  and that is now survivable rather than fatal; see the classify entry under
  Traps.

### TUI polish

- **Steering and queuing are the same key.** Enter starts a run when idle and
  steers one already going; there is no way to queue a follow-up instead.
- **No `/export` or copy.** `NAMES` lists twenty-five commands
  (`tui/command.rs:314`, re-counted 2026-08-25 after `/entity` landed;
  twenty-four on 2026-08-24, twenty-one on 2026-08-21,
  after `/docs`, `/send` and `/remote-control`) and none of them get the
  transcript out. **OSC 52 is no longer the answer to assume.** `/docs` writes
  the link to the system clipboard that way (`tui/docs.rs`,
  `clipboard_escape`), and this file used to record it as the mechanism an
  export would use because the escape rides back down the SSH connection — but
  the 0.1.11 arc found no terminal acknowledges the write, which is why that
  release had to hand the mouse back and add `^s` selection mode instead. The
  working route off the screen today is a human selecting text, so an export
  wants a file, not a clipboard escape.
- **`NO_COLOR` is honoured only by the plain CLI renderer.** The TUI hardcodes
  colours inline; there is no semantic colour table.
- **No keymap configuration.**
- **The "is a modal open?" guard is a list maintained by memory.**
  `open_scoped_review` enumerates every modal field to decide whether something
  already owns the keyboard, so it is wrong the next time one is added — and it
  was wrong three times over, missing `/mail` and `/polls` from the day they
  shipped until 2026-08-20, which meant a run finishing under `/review now`
  opened `/outbox` *underneath* the visible modal and drove it with keys nobody
  could see. Fixed by adding the three names, which is the fix that will be
  needed again. The shape that ends it is one predicate the modals answer
  rather than a list of fields.
- **`pub const NAMES: [&str; N]` states its length twice.** The count is in the
  type, so adding a command means editing two places — and when two sessions
  add one each, the merge is textually *clean* and the array ends up one short,
  because both sides made the same `18 -> 19` edit on one line and git takes it
  once. It fails as a compile error rather than silently, which is the good
  news. `pub static NAMES: &[&str]` says it once; `iter()` and `contains()` are
  unchanged. Deliberately not done on 2026-08-20 with a second TUI arc in
  flight: changing that line would have traded a known one-character merge fix
  for an unknown conflict.
- **Six modals scroll their detail unclamped.** `/skills`, `/outbox`,
  `/frontdoor`, `/triggers`, `/polls` and `/doctor` `saturating_add` the offset
  with no upper bound, so you can scroll past the end into blank space. You can
  still reach everything, so this is "cannot tell where the end is" rather than
  hidden content — the reason it is listed and not fixed. `/tools`, `/tasks`
  and `/help` were given the measured form on 2026-08-20 (`line_count`, clamp
  to `drawn - visible`, hint only when `max_scroll > 0`) and are what to copy.
- **Captured log lines wait for the idle tick.** `logs.rs` drains at the top of
  the event loop, which fires every 200ms during a run but only every 60s at
  idle, so a warning with nothing else happening can appear a minute late. Any
  keypress drains it at once. Ending it means a channel the writer wakes the
  loop on; at idle there is nothing running to warn about, so it was judged not
  worth the complexity rather than overlooked.

### The graph, now a sibling

mecha-graph shipped 2026-08-16 (repo public, three crates at 0.1.0, tools
unprefixed, store at `~/.mecha-graph/`). What that arc left open:

- **Notes have a home and no way in.** The graph already treats notes as a
  first-class *user-authored* source — `reflect.note` is an episode kind keyed
  by the note's stable id, `content_hash` catches an edited note as an update
  rather than a duplicate, and `reflect-process` promotes a structured note
  (`Type: #person/#company/#book`) into entities with identifiers and facts.
  That is the hard half and it is built. What is missing is **ingestion**:
  Reflect appears nowhere in `INTEGRATIONS.md`'s sources table, so notes reach
  the graph only if something else puts them there, and no other note
  application is understood at all.

  Worth doing because notes are the highest-confidence source the graph has —
  they are the user's own words about their own world, where mail and Slack are
  other people's — and because the mecha side already advertises the graph as
  holding "who people are and what you already promised", which is exactly what
  people put in notes.

  **The machinery already exists in both shapes this needs.** `imessage` reads
  a local SQLite file with a path argument, and `slack` authenticates with a
  user token — so a notes importer is a question of *which application*, not of
  new source infrastructure. The two families to price:

  - **Local-file apps** (Obsidian, Logseq — markdown vaults; Apple Notes and
    Bear — local SQLite) follow the `imessage` pattern: a path, a reader, and
    on Apple's stores the same Full Disk Access caveat that one already
    carries. No credential, no network, no rate limit.
  - **API apps** (Notion, Reflect) follow the `slack` pattern: a token, and
    whatever the vendor's export granularity turns out to be.

  The first step is a survey of which of these actually expose a stable read
  path today rather than a guess at it — the `mecha-graph source add <kind>`
  surface is the same either way, so the work is deciding what to support, not
  how. **Do not let this be advertised before it is built**: the
  mecha introduction's integration table deliberately omits notes for exactly
  that reason, and should gain a row only when a source exists.

- **No release workflow.** The three crates were hand-published; the repo has
  no CI at all. mecha's tag-driven workflow with Trusted Publishing is the
  template, and the half-published-workspace trap it documents applies
  verbatim to a three-crate workspace with an internal dependency.
- **The dependency inversion is scoped and unstarted.** `vet`, `gossip`, and
  `corroborate` are graph-curation agents squatting in mecha-cli — the
  2026-08-16 gossip work needed paired commits across the two repos, which is
  the friction the move ends. The sorting rule: a command belongs to the repo
  whose store it curates; `distill` stays (it reads mecha's sessions and is
  the bridge). Move `gossip` first; it builds on published `mecha-core`.

  **`mecha review` (2026-08-22) runs against that rule on purpose, and the
  next person should not read it as an oversight.** By the sorting rule its
  graph half belongs in mecha-graph. It is in mecha-cli because the thing
  asked for was a *unified* surface — one list holding the graph's queue
  beside the outbox, the front door and the staged rule changes, none of
  which mecha-graph knows about. It touches no schema and holds no store; it
  drives `mecha-graph` as a child process, so the coupling is a binary name
  and a flag set rather than a crate. If the inversion happens, `review`'s
  graph verbs are the part that stays, and what moves is whatever
  `mecha-graph review` grows to make them thinner.
- **The queue is a ratchet, and now the instruments say so.** 91.2% of the
  6,434 pending candidates can never reach the accept gate: it wants a
  `(proposer, predicate)` on `DURABLE_CLASSES` (`precheck.rs:837`) or an
  earned rung, and `ladder.rs:120` has `Staged → Sampled → Trusted` with **no
  rung below Staged** — a class earns its way into autonomy and can never earn
  its way out of the queue. The symmetric fix is a `Rung::Suppressed` entered
  on the Wilson *upper* bound, mirroring the promotion rule and reusing
  `wilson_lower_bound`'s own arithmetic.

  **Do not ship it yet, and the reason is the finding.** On human verdicts
  alone, with a floor of ten, that rule currently suppresses **nothing** —
  there is no class the owner has judged often enough and rejected
  consistently enough to condemn. 40.5% of the queue sits in 660 classes with
  no human verdict at all. The prerequisite is evidence, which is what
  `review --sample` was built for: twelve random items across the ten largest
  unjudged classes is 120 decisions covering 1,597 queued items. Suppression
  is the last step, not the first.

- **The ladder reconcile shipped its promote half; the demote half is a
  decision still open.** `mecha-graph ladder` (2026-08-23) prints every
  class's rung, human record, Wilson LB and pending count, and
  `ladder --promote` re-derives rungs one rung per pass — run live, it
  unstuck `works_on`/`member_of`/`located_in` and the queue fell 7,296 →
  6,569 the same day. Deliberately **never demotes**: demotion stays
  correction-driven (D3), so `llm/works_at` still sits at `sampled` on an LB
  of ~0.49 — under the 0.65 floor, auto-accepting nine in ten on evidence
  that no longer justifies it. What remains is that ruling (statistical
  demotion, or leave D3 as the only path down) and wiring `--promote` into
  the nightly before precheck so threshold changes take effect without a
  review session. Note `reviewed_by` (V017, same day) now labels machine
  accepts, so the human record the recompute reads is exact going forward.

- **The nightly alert reports depth, not rate.** `ALERTS: merge queue 6491`
  reads identically whether the accept lane admits 5% of inflow or 95%, which
  is why the queue grew 1,035 → 6,434 over nineteen days unremarked. Print the
  delta and the admit rate (`admitted 56 of 971 added`), and have `doctor`
  raise a finding when admitted trails inflow three nights running. Related:
  vet's ceiling is `VET_LIMIT` × 10 predicates = 400 a night against 500–970
  candidates arriving, so the drain is specified slower than the fill.

- **A queue verdict has no undo, and the cascade raised the stakes.** One
  keystroke now decides up to a couple hundred items and nothing can walk it
  back: `mecha-graph undo` covers only episode delete/edit, a reject keeps
  its row (`status='rejected'`) but no verb flips it, and an accept asserted
  a live fact. The `reviewed_by = 'cascade:<seed>'` label makes the natural
  undo unit cheap to find: an `undecide <seed>` that returns the seed and
  its labeled fan-out to `proposed` (retracting accept-created facts only
  while nothing has built on them), a `mecha review` passthrough, and `u` in
  the `/queues` modal for the sitting's last verdict. Designed in
  conversation 2026-08-23, not built — and now covering a second caller:
  the phone's sample deck (2026-08-24) verdicts through the same CLI verbs,
  one thumb-tap each.

- **Slack verdict cards are deliberately unbuilt.** The `queues` command
  word is read-only; mail-triage cards and queue-verdict buttons each need a
  design pass against `SLACK-ACTIONS-DESIGN.md` first (closed Action enum,
  confirmation ladder — a group cascade from a phone is one tap with a
  two-hundred-item blast radius). Write the design before the code.

- **Extraction progress is unobservable, and the number standing in for it
  measures something else.** `HealthStats` has no backlog field, so
  `~/.mecha-graph/nightly.env` justifies `EXTRACT_LIMIT=400` by citing
  `enriched_pct` — which counts rows in `episode_enrichment`, written from
  exactly one place, `sources/bee.rs`, with `model = "bee-native"`. It equals
  2,411/20,501 = 11.7604%, the Bee share of the corpus, to the digit, and
  moves only when Bee ingests. The real backlog is episodes absent from
  `extract_state` at the current `prompt_version` (`extract.rs:229`) — roughly
  17,600, about 44 nights — and bumping `PROMPT_VERSION` re-queues the whole
  corpus. Add the field; fix the comment before it sizes another batch.

- ~~`mecha-graph fork` is broken~~ **Fixed 2026-08-26 evening** (graph
  `237b686`), so there is a working test bed again — `fork --out …` completes
  on the live 202 MB store in 1m38s with counts matching exactly. It was
  never a bug in `fork`: all three copy paths run migrate-then-copy, the
  harrier switch (2026-08-20) left the source's `vec0` tables wider than
  `run_migrations` builds them, and the reconciliation had been added to
  **one** of the three. `encrypt_in_place` was broken too and nobody noticed,
  because `fork` is the only one of the three people run on a whim — so a bug
  in two paths read as "forking is broken". Now in `copy_all_tables`, which
  every path calls. **A step every copy path needs belongs in the function
  every copy path calls**, and this trio has now broken together twice from a
  change to the destination's schema made before the copy; the first was a
  migration seeding a node.

- **Gossip's rotation cannot fill its quota.** `probe-targets` returns exactly
  10 candidates; `GOSSIP_ENTITIES=3` with `GOSSIP_COOLDOWN_DAYS=7` demands 21
  distinct targets a week from those 10, so nights silently under-fill (2 on
  08-22, 1 on 08-17, 2 on 08-16). Either drop to 2 a night, cut the cooldown
  to 3 days, or do the upstream fix `nightly-mecha.sh` already names — stop
  counting gossip's own reads as `retrieval_touch` demand. The
  self-reinforcement it warns about is visible in the current ranking: Frank
  Chang leads at 26 touches *because* he was probed.

- **`--create-subjects` will mint the next placeholder, and nothing stops
  it.** The 2026-08-26 repair merged away 30 topic nodes whose *display name*
  was another node's id, all created by `accept --create-subjects` answering
  `cannot resolve subject` on a candidate whose subject was never a name. The
  producer that fed it is fixed; the verb is not. It takes whatever string is
  in `subject` and makes a node named that, so the next producer writing
  anything unresolvable — `kg_upsert` shares the payload shape, so an agent
  can — repeats it exactly, and the damage is invisible afterwards because the
  placeholder makes the bad value *resolve*. The cheap guard is to refuse an
  id-shaped subject outright (`linkers::looks_like_node_id` is already the
  predicate, already tested); the fuller question is whether a verb that
  creates an entity as a side effect of a verdict belongs on a review surface
  at all. Rerun `mecha-graph repair-id-payloads --dry-run` to check; it is
  idempotent and reports zero when clean.

- **The graph nightly runs a binary no install refreshes — a seventh surface
  the update skill does not cover.** `scripts/nightly.sh:46` sets
  `PKG="$REPO_DIR/target/release/mecha-graph"` and *executes* it; it does not
  build. So a `cargo install` of both graph crates leaves the 01:30 run on
  whatever `cargo build --release` last produced in the repo tree, which on
  2026-08-26 was three days old — old enough that the night after the
  linker fix would have re-staged up to `KNN_MAX_CANDIDATES` (40) of exactly
  the candidates that day's repair had just cleaned out of the store. Caught
  before it fired. **Half-closed the same evening**: the `update` skill now
  names it as a seventh binary with its own `cargo build --release` and a
  date check, and warns against assuming `cargo install --path` refreshed
  that path (`80750bb`; the skill's own text had claimed the nightly
  "builds"). The binary itself was rebuilt, so the 01:30 run is safe.

  **What is still open is the better fix**: have the nightly build what it
  is about to run, so it cannot go stale between deploys at all. A skill
  step depends on somebody following a skill; a script that builds its own
  binary depends on nothing. The general shape is that "install" and "every
  binary that runs" are different sets — the same confusion the skill
  documents one level up, and an inventory of "what is running" that only
  lists things with a `--version` will always miss a cron job.

- **A stranger-facing README pass.** The public README still reads like the
  private repo's; nothing in it walks a person from `cargo install
  mecha-graph` to a populated graph.
- **Cosmetic**: the private checkout still lives at
  `~/Github/personalized_knowledge_graph` (paths baked into mecha's config
  `command =`, two crontab lines, and the gitignored OPERATIONS.md), and
  mecha's ARCHITECTURE.md still says "pkg" in narrative spots.

### Larger, and deliberately not started

- **Canvas — researched 2026-08-21, blocked on a credential, and the
  unblocking action is a five-minute form.** `docs/CANVAS-RESEARCH.md` is the
  authority. The shape is settled: a **fifth binary on `mecha-mail`** (not
  core, not a fifth crate, and *not* a third-party MCP server — the capability
  override in `mecha-core/src/config.rs:804` is per **server**, so forcing
  `external_send` to cover a grading tool makes every read a send sink and the
  interlock blocks reads the moment a submission arms taint), OAuth2 over
  `urn:ietf:wg:oauth:2.0:oob`, ~16 tools, credentials at
  `~/.mecha/canvas/<account>/oauth.json` on its own root.

  **Nothing can start until Dartmouth issues a credential.** Self-service
  tokens were decommissioned 2025-06-18 and the probe on 2026-08-21 confirmed
  the `+ New Access Token` button is greyed out for faculty, not just students —
  so the service request form is the only door, at roughly five business days.
  Ask for a scoped developer key *and* name a manual token as fallback in one
  submission, and ask for **"Allow Include Parameters"** on the key or scoped
  tokens silently ignore `include[]` and return 200 with the data missing.

  The design decision that makes the rest affordable: **assistive, never
  evaluative**, with completion checking as the sole exception. No verb accepts
  a numeric or letter grade. Two things to verify against the real key before
  relying on them, both recorded as §8 questions: that a scoped key genuinely
  cannot reach GraphQL (which is what makes posting grades structurally
  impossible, and rests on undocumented Instructure behaviour), and whether a
  manual post policy hides submission *comments* or only grades.


- **`mecha-factory` — the public surface. It is deployed, it sends mail, and
  it is self-serve.** Its own repository, public at
  **github.com/ljchang/mecha-factory** (384 tests, re-measured 2026-08-10),
  running on a small VPS:
  the API at `https://gate.mecha-factory.ai`, artifacts at
  `https://<handle>.art.mecha-factory.ai`, notebooks at `…compute…`. Verified
  live 2026-08-06 — a bundle rendered here, pushed there, served under the
  `static` policy behind a certificate the binary obtained for itself. Five
  scoped keys exist — `publish`, `release`, `drain`, `slots` and `operate`;
  which handles and key files exist locally is in `docs/OPERATIONS.md`.

  Everything through self-serve is **built, deployed, and verified live**:
  SES mail, the scope split that moved "a human releases" onto the credential
  (`Scope::Publish` vs `Scope::Release` — the load-bearing decision), and all
  six self-serve steps, ending with the operator running their day from
  `factory-publish operator …` and routine SSH over. The arcs, the
  verification, and the traps are in [`HISTORY.md`](HISTORY.md), the design
  in [`PUBLIC-SURFACE-DESIGN.md`](PUBLIC-SURFACE-DESIGN.md) §14–15, the
  deploy procedure in that repo's `docs/DEPLOY.md`. Do not re-derive any of
  it here. One local fact worth keeping in view: `sandbox = true` is
  deliberately **not** set on the factory MCP server — bwrap does not work on
  this box, docker cannot confine the notebook render subprocess, and the
  config says why at length. Half the premise aged out 2026-08-16: the
  Landlock backend now confines without privilege on exactly this box, with a
  per-server `network` override for the vendor-Pyodide fetch — the decision
  deserves a revisit, and the config comment predates the option.

  **A second client is verified and documented (2026-08-07 evening).** The MCP
  surface was driven from a Claude Code session over raw stdio — handshake,
  `tools/list` (all `bundle_*`, no drain tool), `bundle_render` under the
  `--root` jail. `docs/SECOND-CLIENT.md` in that repo is the onboarding path.
  Still unexercised from a second client: `bundle_publish` against the live
  box (blocked mid-verification by a permission gate — that repo's
  `scripts/mcp-drive.py` is the harness, kept as the second-client smoke
  test).

  **The 2026-08-07/08 night shipped the inbound-attachments arc and the web
  face — deployed and verified live**: form attachments end to end, the gate
  chrome, scanner-proof magic links, and the signed-in artifact viewer. The
  full arcs are in [`HISTORY.md`](HISTORY.md); nothing from that night
  remains open except what the bullets below name.

  **An artifact now has two URLs and the tools say which is which
  (2026-08-10 night, merged, unreleased).** A publish reports the gate's
  viewer page beside the bare artifact URL; the account page and the docs
  point at the page; visibility is `Option` end to end so a caller asserts one
  only when told one. The arc, its three fallout bugs and the review that
  caught the worst of them are in [`HISTORY.md`](HISTORY.md). Two things a
  reader needs from it here:

  - **It is on `main` and not deployed.** Until the box runs a build carrying
    `viewer_url`, `factory-publish` prints one column and the MCP answer names
    one URL — the designed degradation, verified live against the current
    origin. `mecha-manifest`/`mecha-factory`/`mecha-factory-publish` at the
    next tag, then `factory-deploy`, is what turns it on. The version bump was
    deliberately **not** made: 0.2.2 is the intended number (the public-API
    break is knowingly shipped as a patch — crates.io reports zero reverse
    dependencies), and the tag is being held to carry other work with it.
  - **The local store and the box can now disagree about visibility**, and
    nothing reconciles them. `bundle_publish`/`bundle_alias` still derive a
    concrete visibility for the local record — its schema has no absence —
    while sending `None` to the box. That is correct as written and is why
    `factory-publish list` can report a visibility the origin does not hold.
    Closing it means either reading the box's answer back onto the record or
    dropping visibility from the local store; it was left out of a URL change
    on purpose.

  **The personal public surface is merged and deployed** (verified against
  `main` 2026-08-16: switchboard `6fbd2c0`, cockpit records `ff91bfa`, the
  ten review findings `85be8b4` all on `main`; the box serves 0.2.4, which
  contains them). It adds two pages a stranger sees: the **hangar** at `gate…/@<handle>`, which lists everything that
  person has made public, and a **switchboard** at `gate…/@<handle>/<slug>`,
  a hand-patched page of lines meant for an email signature. The design is
  [`SWITCHBOARD-DESIGN.md`](SWITCHBOARD-DESIGN.md) (open here as
  [ljchang/mecha#41](https://github.com/ljchang/mecha/pull/41)); it is the
  authority and nothing about the arc should be re-derived from this
  paragraph.

  Three things a reader needs before touching it:

  - **It closed a live bug on the way.** Withholding is stored per *version*
    and visibility lives on the *alias*, so a public bundle whose live version
    an operator had withheld read "everyone" on its owner's account page and
    served nobody. `bundles_overview` reads the withheld column from the
    aliased version's own row now — the old query's `MAX(b.version)` answered
    about a version nobody serves.
  - **`inventory.rs` is the load-bearing piece, not the pages.** One query
    answers "what has this user got" across all four kinds (bundles, forms,
    booking pages, polls) and computes `Reach` — can a stranger open this —
    once, by the same reasoning `http/artifacts.rs` uses before serving a
    byte. The account page, the hangar and a switchboard all render from it,
    differing in a filter and never in a source. A second opinion about who
    may read what is how a private artifact ends up named on a public page,
    where the title alone is the leak.
  - **Schema 13** adds one `records` table holding a profile and every board,
    each as two texts: `baseline` (the TOML exactly as last pushed) and
    `effective` (what the page renders). The difference between them *is* the
    set of edits made in the cockpit, which is what a later push folds around
    rather than flattens.

  **A high-effort review ran over the branch and its ten findings are fixed
  (`f7350bb`), which took the suite to 467.** Three were reachable by a
  stranger or an injected instruction — an MCP slug that became a filename
  with no validation and skipped the `confined()` jail, `host_of` returning a
  URL's userinfo as its host, and a labelled profile link rendering with no
  host at all on the gate origin. Two were merge bugs: deletions never
  applied to a record first written in the cockpit, and the pull-then-push
  workflow announced data loss that had not happened. The two lessons worth
  carrying are in [`HISTORY.md`](HISTORY.md) — a doc comment is not an
  enforcement, and a static path segment shadows the parameter beside it.

  One decision the fixes made, which the design doc now records: **`theme`
  and `accent` are refused** rather than accepted and ignored. They validated
  and stored and no renderer read them. They return with the renderer, and
  §6.3 names the choice that has to be made first — an inline token block
  keeps the stylesheet handle-free where a per-user sheet would not.

  Left undone deliberately, each named in the PR:

  - **Board deletion does not exist.** A retired slug must never be reissued —
    the argument that makes a handle unreusable — which needs a retired-slugs
    table like `handles`. A delete that silently frees a name somebody has in
    an email signature is worse than no delete, so there is none.
  - **`--unlisted` is unbuilt**, so every public artifact appears on the
    hangar. That *is* the design's default (§3.3); what is missing is the
    per-artifact opt-out, which is a property on the bundle row rather than
    anything about the page.
  - **Avatar and per-line click counts** are §12.2 and §12.3 of the design,
    both past the core on purpose — the first pulls in the blob/upload path,
    the second is a new data class (visitor behaviour) on a box we assume is
    lost and needs its own retention answer.

  **Open there, in the order they bite:**

  - **The MCP surface tracks the CLI again, and two small things are left.**
    The drift is closed — fifteen tools, `surface::REACH` making every command
    exposed-or-excluded in writing, and the arc is in
    [`HISTORY.md`](HISTORY.md). What was deliberately left:

    - **`poll_status` reads its question prompts back from the box**, and now
      the answers too. The prose is the point of a text question, and
      `openWorldHint` marks the whole result untrusted, which is the mechanism
      mecha already uses for mail bodies — so what is left is the user's *own*
      question text round-tripped through an origin the design assumes is
      lost. Smaller than it sounds, and recorded rather than fixed; closing it
      means caching the spec in the local record at create time and rendering
      prompts from home.
    - **`vendor_runtime` is the notebook tool's one reach for the network**,
      fetching Pyodide from a pinned allowlist. It is *not* code execution —
      that belief was measured false on 2026-08-10 (see
      [`HISTORY.md`](HISTORY.md)) — so a confined renderer would need network
      for this template, not an execution exemption.

    Unchanged and worth re-reading before adding more: every exposed tool is
    schema tokens in every run, measured at ~7–8k a turn before this grew the
    surface by eight. That argues for narrowing per surface (`[slack] tools`,
    `[tools] enabled`) rather than for exposing less.

    **The box runs v0.2.6 (deployed and verified live 2026-08-17)** — three releases past the v0.2.1 that first exercised
    `factory-deploy` end to end (download, checksum, prove, swap,
    health-check). The served stylesheet went from 23,397 to 30,939 bytes and now
    carries the rank counters, the card-select rules and the VAS anchors.
    Worth remembering that `poll_render.rs` lives in `mecha-manifest`, which
    the *box* links: a rendering change is a box deploy, not a home reinstall.

  - **The release workflow is verified** — `v0.2.1` (2026-08-10)
    built the static musl `factory`, asserted it, checksummed it, attached
    both to the GitHub release, and published `mecha-manifest`,
    `mecha-factory` and `mecha-factory-publish` at 0.2.1. The download-and-
    verify procedure in `DEPLOY.md` now has a real artifact behind it.
  - **The apex redirect is deployed but dormant, waiting on DNS.**
    `redirect_hosts` (301 to the gate, path kept, riding the base
    certificate) is in the deployed binary; the config line was deliberately
    removed because the apex still points at the registrar's parking
    servers, and an ACME challenge for a name not resolving here would jam
    renewal for the real origins. Sequence: point apex and `www` A-records
    at the box, then add
    `redirect_hosts = ["mecha-factory.ai", "www.mecha-factory.ai"]` to the
    deployed `factory.toml` and restart.
  - **The DNS is at a registrar with no API for custom records** — every
    row is typed by hand, including the five SES ones. Moving the zone to
    Cloudflare (DNS-only, never proxied) is independent of everything else —
    and it gates nothing: the wildcard `A` records already resolve any new
    handle, which is what let per-user issuance ship without touching DNS.
    It does **not** unlock wildcard certificates either: `rustls-acme` speaks
    only HTTP-01 and TLS-ALPN-01, so the library forecloses them whoever
    hosts the zone.
  - **The factory documentation is half written.** Shipped: the component
    gallery, `polls.md`, `slides.md`, and — 2026-08-10 — `onboarding.md`
    (claiming a handle, pairing, the five scoped keys — of which pairing
    installs two, corrected 2026-08-10 — and wiring the MCP surface into an
    agent), `artifacts.md` (versions, the alias, visibility, the
    share/revoke path and its oracle rules, takedown, what retention will not
    sweep) and `notebooks.md`. Also `features/slack.md` on the mecha side,
    which is setup plus what the remote control actually does.
    [`FACTORY-DOCS-DESIGN.md`](FACTORY-DOCS-DESIGN.md) lists the rest with the
    claims each has to make, sourced from the code. `overview.md` landed
    2026-08-10 with the documentation overhaul — both directions of the
    boundary, the three crates and the three origins, the request lifecycle
    end to end, and the notebook confinement gap stated as a warning. Still
    missing, in the order they bite: `field-kinds.md` — the four-column table
    (TOML · JSON Schema · rendered control · what the server enforces) exists
    nowhere but `request.rs`, and `second-client.md` assumes it; `booking.md`,
    since the whole scheduling instrument is still undocumented for readers;
    `request-types.md` and `theming.md`.

    Note for whoever writes the next one: the site's `sync-gallery` step
    fetches **mecha-factory@main**, so a gallery fix on a branch does not show
    up in a local docs build until it merges.
  - **The operator admin panel and private sharing are built, reviewed,
    tested, and deployed** (2026-08-07/08; the arcs are in
    [`HISTORY.md`](HISTORY.md)). What remains is the two live checks at the
    top of this section — nothing else is open on either. The one fact
    worth keeping in view here: the box now runs three session surfaces
    (tenant / operator / reader), deliberately parallel and never unified —
    `Db::signin`'s doc comment records why, so do not "deduplicate" them.

  The mecha-side half of step 7 is **built**: `mecha-factory-publish drain`
  fetches the queue, and `mecha frontdoor` is the quarantine between a drained
  record and a triage run. See `ARCHITECTURE.md`.

- **Images: built, merged and deployed 2026-08-21.**
  `Block::Image`, both provider encoders, `image.rs`'s caps, `provider::preflight`,
  the `scripts/mmproj.sh` guard and four entry points (Slack connector,
  remote-control inbox, `mecha run --image`, and a file dropped on the TUI
  prompt) are on `main`, 1,210 tests green. Deployed the same day, in the
  `update` skill's order:

  - **`:8080` serves vision.** `llama-local.service`'s ExecStart is the
    development checkout, so the merge is what delivered `--mmproj`; the
    startup line now reads `loaded multimodal model, …/mmproj-BF16.gguf` and
    `/props` reports `modalities.vision: true` with `n_ctx_slot = 262144`
    unchanged. Measured after the restart rather than assumed: **72–79 tok/s**
    on a short prompt against the 70.5 the script records for `-np 4`, so the
    projector costs nothing in generation and the server did not load under
    memory pressure — the failure that would have left it permanently slow.
  - **The binary is reinstalled** and `mecha-slack`, `mecha-triggers` and
    `mecha-drain` restarted onto it, each verified from its own log line
    rather than from `is-active`. Worth keeping from this one: **the version
    string did not change**, because the arc bumped no version — so
    `mecha --version` proved nothing, and what proved it was `mecha run
    --help` carrying `--image` plus cargo reporting that it had replaced an
    install made from the *remote-control worktree*.
  - **Verified live** on the production config against the screenshot that
    started this: quoted back verbatim, and with no preflight warning, because
    config and server now agree.

  One trap found on the first live retest, and now in the `update` skill's
  ordering constraints: **a TUI started before the install kept running the
  old inode** (`/proc/<pid>/exe` → `mecha (deleted)`), and because the config
  had meanwhile gained `vision`, its *call-time* config re-read in `show_file`
  failed with a parse error two hours after the change. `deny_unknown_fields`
  is right and is why — the cost is that config is a wire format between
  versions, and long-lived processes must be restarted after a config key is
  added, not only after an install.

  Worth a look if it recurs: `show_file` loads the **whole global config** at
  call time for one number (`slack.max_upload_mb`), which is what couples an
  unrelated section's strictness to a tool call mid-run. Capturing it at
  registration would decouple them.

  **`~/.mecha/config.toml` gained `vision = true` on `[providers.local]`**, and
  that file is in no repository — a fresh clone gets the code, the projector
  guard and the preflight, and still sends no image until somebody writes that
  line. Backup at `config.toml.pre-vision`. The projectors for qwen3.6,
  qwen3.8 and gemma-4-E4B were downloaded; gemma-4-26B's always was, unused.

  **Drag-and-drop onto the TUI prompt works locally** and is the third door.
  It is structurally impossible over SSH — a terminal pastes the *laptop's*
  path and the TUI resolves it on the far box — so testing it means a local
  terminal, which is what made it look absent.

  Also deliberately not built, and named so it is not rediscovered: **an image
  cannot join a run already in flight.** The steering queue is
  `VecDeque<String>` and a mid-run attachment still lands on disk and still
  has its path named in the steered text, so nothing is lost — but the pixels
  wait for the next turn. Widening the queue is a `RunContext` change.

- **Slack as a remote control — built and verified live; two things left.**
  The arc merged on 2026-08-09 (**PR #25**) and is described in
  [`HISTORY.md`](HISTORY.md) under 2026-08-09; the design authority is
  [`SLACK-DESIGN.md`](SLACK-DESIGN.md) and the evidence
  [`SLACK-RESEARCH.md`](SLACK-RESEARCH.md). What is genuinely unbuilt:

  - **`ask_user` is absent — and the structural half of that claim died on
    2026-08-24.** `Asker::ask_in(&ToolCtx, …)` (`mecha-core/src/tool/ask.rs`,
    default forwards to `ask`, TUI untouched) lets a shared `AskUserTool`
    route by the calling run's jail — the web surface ships exactly this
    (`serve/present.rs::WebAsker`, key = the workspace's directory name).
    What remains for Slack is now ordinary work, not architecture: an
    `Asker` mapping a thread's jail back to its thread, plus card rendering
    through the `Action` ladder.
  - **MCP tools do not honour the per-thread jail** — only the built-in tools
    do, because servers are spawned once with the agent. They are rooted at
    the `slack` producer directory so paths at least agree; isolation between
    threads is not there. **Re-scoped 2026-08-24: this is now every
    many-conversation front-end's limitation, not Slack's** — `mecha serve`
    inherits it identically (`serve/chat.rs`, servers rooted at the `web`
    producer). Closing it means an agent per conversation, an MCP startup
    each; the cost is why it stays open.
  - **The outbox review cards have not been exercised live.** Built and unit
    tested; no run has yet staged a draft while the connector was watching.
  - **It is installed and running** (`mecha-slack.service` active, re-verified
    2026-08-24 — on the current install at merge `9e20214`, bounced and
    inode-swept the same night).
    `mecha-slack.service` is enabled with linger, and `[slack] tools` is now
    `[]` — the whole surface, **including `mail__*` and the graph's `kg_*`**
    (re-measured 2026-08-10; the rationale comment sits above the line in
    `~/.mecha/config.toml`). The earlier stance that mail and the graph were
    "deliberately out" is reversed and this bullet is the record of that. The connector answers a
    **shared lab workspace** (`cosanlab`) rather than the personal one §11.1
    of the design chose — safe, since non-owners are ignored by construction,
    but the app is visible to its members.

- **The factory must never become an owner channel.** Recorded here because the
  reuse is tempting and wrong: `GET /v1/queue?wait=` plus `mecha-drain.service`
  is exactly the right-shaped channel, but hosting the Slack app on the box puts
  a workspace credential on a machine the design assumes is lost, and a command
  queue inverts the direction of *authority* even while preserving the direction
  of packets. Anything arriving from the box stays a request, not a command.
  What the factory should carry is artifacts too large for Slack — the split is
  "Slack carries control, the factory carries bytes."
- **Public benchmarks.** The Terminal-Bench adapter (`bench/`) is written, and
  the **oracle arm64 sweep is complete** (2026-08-05, 14.4h): 75 of 89 tasks
  have a reference solution that passes on aarch64, and those 75 are the only
  comparable set — `bench/oracle-arm64-excluded.txt` holds the other 14.
  `bench/run-subset.sh` runs the calibrated subset.

  **A complete scorecard exists as of 2026-08-11**:
  `jobs/mecha-arm64-subset/mecha-arm64-subset-2026-08-11__02-11-55/` (in the
  `bench-run-v012` worktree checkout) finished all 75 trials — **mean reward
  0.4533 (34/75), 8 errored trials, k=1, v0.1.2 binary, 262k window,
  `max_turns` 80, timeout multiplier 2.0**, `scorecard.html` beside
  `result.json`. The four falsifiable checks it was launched to answer came
  back: empty-turn nudges fell to 4 across the whole run (from a rate that
  poisoned the 08-07 fragment), and the dash-prompt crash did not recur.
  Treat it as the k=1 baseline for the 0.1.2 harness; the 08-10 28-trial
  fragment and the 08-07 attempt stay in their worktrees as diagnosis
  evidence, not baselines. Two earlier lessons still stand when launching:
  rebuild the portable binary first (`bench/run.sh` scores whichever
  checkout you launch from — last recorded at 0.1.6 on 2026-08-16 and
  **not re-verified since**, so assume it is stale rather than current), and read any
  job with `bench/check-subset.py` before believing it is a subset.
  k=5 for a leaderboard-comparable number is the follow-up, ~74h.
  AgentDojo (for the interlock) and a SWE-bench Bash Only control are named in
  the research and unstarted.
- **`mecha replay --json` is not wired into CI.** `scripts/replay-regression.sh`
  consumes it locally, which is the standing regression check; making it a
  workflow needs a single-slot llama-server that CI does not have.

---

## Accepted limitations

These are not to-do items. Each is a deliberate trade, recorded so nobody
"fixes" one without knowing what it costs.

- **`shell` is not treated as an untrusted source.** Taint cannot see inside a
  command, so a command that fetches a hostile page does not arm the interlock
  the way `http_fetch` does. The mitigation is the sandbox, and it is why
  `[sandbox] kind` matters.
- **A confined MCP server sees the workspace.** It is the one writable bind on
  both backends. Right for most servers, wrong for some, and worth knowing which
  you are running.
- **`shell` is enabled inside sandboxed eval cases**, with approval forced open.
  The fixture is safe because each case gets a private staged copy, but the
  command still runs as you unless `[sandbox]` is configured — and its default
  is `none`.
- **A failed trigger run is not retried.** The next slot is the retry. Adding
  retry means deciding what a half-completed unattended run owes you.
- **Gmail drafts, a local mail cache, and past-correspondence context** were all
  considered and deferred, not forgotten.
- **pkg gets no self-scoped read tool.** Capability overrides widen only, so a
  memory read arms `untrusted_input`. Revisit only if self-retrieval taint proves
  to hurt in practice.
- **Retrieval at turn start is rejected**, because it arms the trifecta at turn
  zero for every run whether or not memory was needed.
- **Rule retirement has no decay, TTL, or usage-based eviction**, and no policy
  built on model-rated confidence. Only measured harm argues for retirement, and
  a human accepts the argument.
