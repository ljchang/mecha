# Running experiments in mecha — design

> **Status: unbuilt.** Written 2026-08-26, after the *Communication as
> Inference-Time Scaling* research spec (49 sections) and its initial
> literature review, and against GitHub issue #60. Verified against `main` at
> **be75b73**, and cited by symbol rather than by line — see the end of §5 for
> the incident that made that a rule.
>
> [`BRANCHING-DESIGN.md`](BRANCHING-DESIGN.md) is a **dependency** of this
> file rather than a neighbour — §5 is why. Issue #60 holds the
> communication *policy* question; this file holds the instrument that would
> measure any policy, and §8 is the only part specific to communication.
>
> **Part II (§13–§20) added 2026-09-03**, after the appraisal sprint
> (PRs #140, #141, #147, #151), against `main` at `4a888ad`. It asks the
> same instrument to measure the appraisal system and to ablate subsystems
> one at a time, adds two trial kinds and a closed switch set, and reorders
> §11's build so the appraisal questions — none of which need branching —
> come first. Nothing in it is communication-specific; §8 still is.

---

## 0. What this is for

The research programme asks whether several instances of a fixed, pretrained
smaller model, organised through a communication harness, can solve problems
the individual instance cannot — treating organisation as an inference-time
architecture rather than a training objective. It needs an instrument that
can state, from artifacts alone, what differed between two runs and what it
cost.

mecha is already the substrate: a run's configuration is recorded in full, a
transcript is append-only and replayable, and a gate exists that judges two
arms against a falsifiable prediction. What it has never had is a **unit
larger than a run**. Every store here — the outbox, the trigger ledger, the
candidate store, the run-quality corpus — answers a question about one run or
about all runs. An experiment is a designed comparison over a chosen set, and
there is nowhere for its design to live.

**The sentence that decides the shape of this file** is the literature
review's own summary of what the field has not done:

> Message-level causal replay: no direct precedent in the screened
> multi-agent LLM set. The reviewed set contains no study that snapshots
> execution state and replays the same trajectory while deleting, delaying,
> compressing, or rerouting an individual message.

That is a harness capability, not a modelling one, and it is the reason to
build here rather than adopt an orchestration framework. Everything below is
ordered by how close it sits to that sentence.

---

## 1. What already exists

Stated once, because most of the instrument is built and the expensive
mistake would be rebuilding it beside itself.

| Piece | Where | What it already gives an experiment |
|---|---|---|
| Full run configuration | `RunConfig` (`session.rs`) | The resolved system prompt, ceilings, permission mode, sandbox, seed and temperature — written on every attach, not once per session. Its stated rule is the experimental one: *anything that shapes the request or constrains the run is a confound if it is not recorded.* |
| Per-run outcome | `RunStats` (`session.rs`) | Stop cause, tool calls/errors/denials, compactions, blocked sends, taint. Written by every front-end. |
| The corpus reader | `runlog.rs` | Bounded scans across the whole session store; a rate over a zero denominator is `None`, never zero. |
| The gate | `candidate.rs` | Paired by episode, a selection slice plus a holdout, a work guardrail that outranks the score, single-arm episodes dropped, counts rather than a significance test. Pure and unit-tested. **How the holdout is drawn depends on how the pool was gathered**: `judge_drawn` takes two slices the caller drew — holdout first and uniformly — because hashing a pool ordered by `Metric::headroom` yields two slices biased the same way and the holdout stops correcting the selection; `judge_with` still partitions on `is_holdout` for `eval --ab-config`, where every case runs and the pool is already uniform. An instrument that adds arms inherits this distinction rather than the hash. |
| Paired replay | `harness_probe.rs`, `replay_run.rs` | Two arms driven over recorded sessions against recorded tool results, both arms replayed so neither measures replay artifacts. |
| Divergence diffing | `replay.rs` | Structural versus argument-only divergence between a recording and a replay. |
| A multi-actor orchestration | `gossip.rs` | Commit-then-reveal enforced by program structure; per-role capability narrowing welded into the tool schema; a recorded exchange that records its own stalls. |
| Cross-process messaging | `mailbox.rs` | Producer-addressed durable mailboxes, advisory flock, refuse-don't-drop caps, dedup, `reply_to`, taint stamped by the harness and merged at delivery. |
| Prefix-reuse measurement | `cache_lens.rs` | Per-run observation of whether the cached prefix is actually being reused. |

Two absences worth naming beside them, because they are the ones this design
is mostly about: there is **no record of a message having been delivered**
(`AgentEvent::MessageDelivered` in `agent.rs` is a live UI event and
nothing persists it), and there is **no way to re-run a trajectory from a
point in the middle** — `replay_run.rs` replays a whole recorded session,
not a chain forked out of one.

---

## 2. Decisions taken

**D1. An experiment is a stored object, on the conventions the other five
stores already keep.** One pretty JSON per item, temp-sibling-and-rename,
advisory flock, closed enums that default on load, resolved items hidden
rather than deleted. Not a new set of conventions: a sixth store that
invents its own is what `mecha review` exists to complain about, and the
enum-as-wire-format rule (`Proposed` hand-rolling `Deserialize` so an
unknown variant degrades to `None`) applies from the first record written.

**D2. An intervention is a branch, not a copied file.** §5.

**D3. Actors are separate processes.** §7. Independence is the treatment
variable, and a shared process is a shared failure domain, a shared
provider connection and a shared registry.

**D4. The gate is the existing one, extended to arm sets.** `judge_with`
already grades anything that names an episode and produces a cost. Two
extensions: a designated control against N treatment arms rather than a
pair, and a task outcome entering as a cost. **Every metric stays
lower-is-better**, which is a deliberate constraint against a comparison
inverting silently — so a solve rate enters as `1 − solve_rate` and never as
a benefit axis.

**D5. An arm may only vary what the closed override set names.**
`harness::OverrideKey` is four keys today and is shared with
`eval --ab-config` on the stated reasoning that *a second spelling of the
set is how the measurement arm and the acceptance arm silently stop being
comparable*. Anything an experiment needs to vary is added there, once.

**D6. Message and artifact events are recorded always, not behind a flag.**
The precedent is `Record::Outcome`: fifteen fields of `RunOutcome` were
computed on every run and two were kept, so every interactive run was less
observable than a trigger. A flag reproduces that by making the default
un-analysable. The cost is bytes; the benefit is that an experiment can be
designed against sessions that were not run as experiments.

**D7. The experiment runner is a peer of `mecha eval`, never a flag on it.**
`eval` forces off MCP, hooks, learned rules, the outbox, fallback, skills
and messaging (`opts.no_messages` in `commands/eval.rs`) so a scorecard grades
the model it names. Those forcings are correct and stay. An experiment
needs the opposite — messaging on, an isolated mailbox root, N actors — and
bending eval into it would silently change what every existing scorecard
means.

**D8. Topology is structural, not prompted.** §8.

**D9. The two replay regimes are named separately and never blended into one
number.** §6.

**D10. A trial records the server geometry it ran against.** §9.

---

## 3. The experiment store

```
~/.mecha/experiments/<exp-id>/
  manifest.toml          the design: arms, factors, tasks, seeds, N, budgets
  trials/<trial-id>.json one row per (arm × task × seed × repetition)
  mailboxes/             this experiment's mailbox root — never the real one
  snapshots/             environment checkpoints, when the backend makes them
```

Sessions stay where sessions live. The join is by id, in both directions:
a trial names its actors' session ids, and each session's `RunConfig`
carries the trial reference (§4). Two pointers rather than one, because a
trial file that is the only index makes an orphaned session unreadable, and
a session record that is the only index makes "which sessions were in arm
C" a full-store scan.

The manifest is **the design, written before the run** — arms, the control,
the prediction each treatment arm makes, and the split seed. That is
`candidate.rs`'s rule carried up a level: a candidate carries a falsifiable
prediction *made before either arm was measured*, because otherwise a
proposal cannot be refuted by the next measurement. An experiment whose arms
are chosen after looking at the trials is the same multiple-comparisons trap
the holdout exists to close.

`mecha exp` is the surface: `new`, `run`, `status`, `show`, `judge`,
`export`. Reading is a store read; every mutation is a child process, on
`/tasks`' and `/queues`' rule.

---

## 4. Run identity and the event record

**`RunConfig` gains one optional field.**

```rust
/// Which trial this run is one actor of. `None` for every ordinary run.
pub experiment: Option<ExperimentRef>,

pub struct ExperimentRef {
    pub exp_id: String,
    pub trial_id: String,
    pub arm: String,
    /// This actor's producer name — also its mailbox address.
    pub actor: String,
    /// Free-form: `planner`, `verifier`, `worker`. Never interpreted here.
    pub role: Option<String>,
    /// The task the trial is on, e.g. a Terminal-Bench task id.
    pub task: String,
    pub repetition: u32,
    /// Hash of the resolved arm definition. Two runs with the same hash were
    /// configured identically; two with different hashes differ somewhere,
    /// even if nobody remembers where.
    pub condition_hash: String,
}
```

**A new session record, `Record::Event`.** The transcript already carries
`Meta | Message | Summary | Outcome | Config | Taint | Rewrite`, and the
rule that justified `Rewrite` justifies this one: *if the loop put something
into the conversation that the model did not produce, the transcript has to
say so, or the record is a lie about what the model saw.* A delivered
message is indistinguishable from an ordinary user turn once
`render_delivery` has run, and nothing else can recover it — the mailbox
store prunes resolved messages by policy (`keep_resolved`) and never knew
the turn index.

What one event carries, per the spec's §13 list, minus what the transcript
already holds:

```
kind            message_sent | message_delivered | message_refused
                | artifact_read | artifact_write | barrier_release
at              RFC3339, and the turn index within this run
actor           who this session is
peer            the other end, when there is one
message_id      the mailbox id; `reply_to` when threaded
bytes, tokens   what it cost to send and what it cost to receive
outcome         sent | duplicate | mailbox_full | held | delivered
path            for artifact events, workspace-relative
```

Two rules on it:

- **It is written by the code that did the thing, never derived from
  `ToolCtx.events`.** That channel is explicitly untrusted — its own doc comment
  in `tool/mod.rs` carries the warning that a tool (including an MCP server's) can send
  fabricated events down it, so *nothing that decides anything reads from
  events*. An analysis substrate decides things.
- **Artifact events come from `ToolCtx::resolve`**, which
  is already the single funnel every model-supplied path passes through. This
  is the one place mecha is structurally ahead of every framework in the
  harness survey: the path jail means artifact-mediated coordination can be
  logged completely, without asking any tool to cooperate. A read of a path
  another actor wrote is the artifact analogue of a message, and without those
  edges the spec's H6 (shared artifacts versus conversation) is not testable
  at all.

**`RunStats` gains communication counters**, phrased as costs so they can
enter the gate unchanged: `messages_sent`, `messages_delivered`,
`message_tokens_out`, `message_tokens_in`, `messages_refused`. The
zero-denominator rule applies — a rate over no messages is `None`, never
zero, because "communicated nothing" and "had no channel" are opposite
findings and the whole hypothesis set turns on telling them apart.

---

## 5. Branching: an intervention is a branch

[`BRANCHING-DESIGN.md`](BRANCHING-DESIGN.md) designs forking a session in
place: message ids, a `Record::Branch { from }`, and a `load` that selects
the active chain by walking parent pointers. It was written as a TUI
feature. Read against the research spec's §14 it is the experimental
substrate, and every intervention on that list is the same mechanic:

| Spec §14 intervention | As a branch |
|---|---|
| Message deletion | branch at the delivery point, omit the delivered turn |
| Communication-burst deletion | branch before the first message of the burst |
| Delay / advancement | branch, re-insert the same text at a different message id |
| Compression | branch per compression level, substitute the text |
| Semantic perturbation | branch, substitute a reworded body |
| Recipient ablation | branch one actor's chain, leave the other's alone |
| Incorrect communication | branch, inject authored false text |
| Oracle communication | branch, inject the text retrospective analysis says was needed |

The spec's §32 "World A / World B" are then two chains in one session file
sharing a byte-identical prefix, and `mecha replay --compare-chains` —
already named in the branching doc as the CLI surface that gets the diff
first — is the divergence measurement.

**Why the shared prefix is the cost model and not just tidiness.** On the
local server, every variant forked at turn *N* re-reads the same cached KV
prefix. Eight compression levels of one message cost roughly one
trajectory's uncached prompt plus eight divergent tails, where eight
independent re-runs from turn zero would pay eight times for the identical
part. Compression ladders and oracle probes are affordable *because* the
representation is a branch.

**The cost, which is why this is a dependency and not a detail.** The
branching doc names it: `taint_timeline` positions taint checkpoints
against message indices **in file order**, and provenance classification
fail-closes on unknown positions. Branching makes file order meaningless.
That doc calls it *the one place a mistake is a security regression, not a
bug*, and its order of work — message ids and a `SessionMeta.version` first,
then per-chain taint with tests proving an ambiguous branch classifies
`untrusted`, then `Record::Branch`, then miners and replay walking chains —
is the order an experiment harness needs anyway.

Two things this design adds to that doc:

- **A branch needs a label.** `Record::Branch` as designed carries only
  `from`. An intervention branch must also carry what was done to it
  (`{intervention: "compress", level: 3, target_event: "..."}`) or the
  chains are indistinguishable a week later. Free-form on the record,
  interpreted only by `mecha exp`.
- **Its line references should be replaced with symbols** when it is picked
  up. It cites `session.rs:233` for `load` and `:265` for `taint_timeline`;
  both had drifted twice over by 2026-08-26, and the repair written that
  morning went stale the same hour when another lane landed. That incident is
  why CLAUDE.md's *Working alongside other sessions* now says to name the
  function, type or constant instead.

---

## 6. The environment: two replay regimes

This is the distinction the research spec conflates, and getting it wrong
would produce numbers that look like outcomes and are not.

**Regime 1 — frozen environment.** Replay against recorded tool results,
which is what `replay_run.rs` does today. Valid only until the trajectory
structurally departs from the recording; after that the arm is answering
questions nobody asked. It measures the spec's §32 near-field: next action,
tool selection, hypothesis, immediate divergence. Cheap, and already built.

**Regime 2 — live environment.** Restore the workspace (for Terminal-Bench,
the container) to its state at the intervention point and re-run for real.
The only way to reach §32's last bullet — *eventual outcome* — and the only
way to run the spec's Phase 0 step 7, the oracle-rescue probes it calls
especially useful because they bound the whole programme: *if perfect
communication at this point does not rescue the small model, communication
probably isn't the limiting resource.* **Not built.**

**The rule that must invert, and the reason to write it down.**
`harness_probe.rs` drops a divergent episode rather than scoring it, on the
correct argument that replay answers from the recording. For harness A/B
that is right. Here **divergence is the dependent variable**, so an
experiment must never reuse that predicate — a run under regime 1 reports
divergence and stops; a run under regime 2 keeps going. A single number
mixing the two would be a distance measured in two units.

Snapshotting is mechanical but real work: `sandbox.rs` already has a docker
backend, and a checkpoint at each turn boundary is a `docker commit` or an
overlay snapshot. Retention is a policy, not an intention — the work store's
rule — because per-turn container images fill a disk in an afternoon.

---

## 7. The trial runner: separate processes

**Decision: each actor is its own `mecha` process.** The alternative — one
`Agent` serving N conversations through `run_in`, which is what that method
exists for — is cheaper and was rejected. Four reasons, in order of weight:

1. **Independence is the treatment variable.** The spec's §22 and hypotheses
   H4/H6 turn on information *not* crossing a boundary. A boundary that is a
   convention inside one process is the thing `gossip.rs` refuses to rely on:
   *the rule is a property of the program, not an instruction.* Separate
   processes make the barrier an operating-system fact.
2. **The mailbox is already a cross-process store.** Producer-addressed
   directories, advisory flock per recipient, `announce`/`depart` liveness
   markers. In-process actors would be using a filesystem IPC mechanism to
   talk to themselves, and the taint stamp — *a per-turn snapshot of the
   sending run's own conversation* — would need a per-conversation identity
   that does not exist.
3. **`message_send` is refused to subagent registries outright**, a hard
   startup error, because a child's taint stamp is either freshly clean or a
   frozen parent snapshot and either laundered. So actors cannot be
   subagents, which removes the obvious in-process implementation anyway.
4. **It matches the server's slot geometry.** N processes map onto N
   llama-server slots, each keeping its own prefix — which is the
   configuration slot affinity was measured on. N conversations round-robining
   through one connection is the prefix-thrash case.

What that costs, stated so nobody rediscovers it: an MCP startup per actor,
N provider connections, and no shared cached prefix *between* actors (there
is none to share — their contexts differ by construction). Startup is paid
once per trial and the trials are minutes long.

What the runner owns:

- **Identity.** Each actor gets a producer name scoped to the trial
  (`exp-<id>-<actor>`), which is also its mailbox address. `work::valid_producer`
  already constrains the namespace.
- **An isolated mailbox root** under the experiment directory. Never
  `~/.mecha/mailbox/` — a stray message from last night's trigger folded into
  a trial mid-run is the contamination `eval`'s `no_messages` forcing exists
  to prevent, and here the answer is a different root rather than no mailbox.
- **Lifecycle**: spawn, watch, cancel. The trigger runner's precedents apply
  unchanged — a `.running` marker per actor with a pid range check (because
  `kill(-1, 0)` succeeds and would report every dead run as alive), and
  cancellation as a sentinel file the runner polls rather than a signal.
- **The barrier**, for independent-first designs: hold every mailbox, then
  release on a condition. `InboundPolicy::Hold` already leaves messages
  pending; what is missing is the release trigger, and that is the whole of
  the spec's *explore independently → commit → communicate → reconcile*.
- **Budgets**, per actor and per trial.

---

## 8. What the communication experiments add

Everything above is general. This section is the part that only a
communication experiment needs, and it is where issue #60's proposal lands.

**Topology is welded into the tool schema.** `MailboxStore::send`
(`mailbox.rs`) accepts any well-formed `to`. The spec's §11 topologies —
star, ring, sparse dependency graph — need the permitted recipient set to be
an **enum in the schema the actor is shown**, not a sentence in a prompt.
Two precedents, and they agree: `gossip.rs`'s `LensedSearch` removes
`sources` from the schema entirely so a reader *cannot* widen its own lens,
and `mecha-mail` bakes account names into every tool schema as an enum at
startup so the model picks from real names instead of guessing. A topology a
model can route around is not a treatment condition.

**Delivery timing is a receiver-side policy.** `InboundPolicy` is
`Accept | Hold | Refuse`, evaluated at every turn boundary. The spec's §28
needs: never, every N steps, task-boundary only, on-request,
harness-scheduled, and model-decided. This extends the existing enum, and
the existing rule carries over verbatim: admission policy is *set only by
config, never inferred from any prompt*, because it must not be decidable by
anything sharing a context window with third-party text.

**Communication budgets need their own ceilings and their own refusal.**
`Budget` carries turns, output tokens and cost. The spec's §11 wants tokens
per message, messages per task, total communication tokens, and *context
allocated to received communication*. The mailbox already has the right
failure shape and it should not be re-derived: refuse rather than drop, and
say why, because *the sender is an agent that can be told the mailbox is full
and act on it.*

**Bursts are derived, not stored.** The spec's §31 argues the message is the
wrong unit and a communication episode is the right one. That is a query
over the event log — which is why §4's events carry both a turn index and a
wall clock, and why they are recorded always.

**The policy layer itself is issue #60's**, and the one thing this design
asserts about it is a constraint rather than an answer: whatever carries the
policy must be **recoverable from artifacts**. `RunConfig` records the
resolved system prompt in full and records tool *names* only, so a policy in
a tool description is unrecoverable; it records nothing at all about skills,
so a policy in a skill is unrecoverable and is disabled under `eval` besides.
That does not settle where the policy should live — it rules out two of the
three places without a further change.

---

## 9. Confounds this machine introduces

Neither the research spec nor the literature review mentions these, and all
three are properties of running against a local llama-server rather than of
any architecture under test. `docs/LLAMA-SERVER.md` is the reference.

- **Context window is `-c / -np`.** N concurrent actors do not merely
  contend for slots; each gets a *smaller context window* than a single
  agent would. Smaller windows compact sooner, compaction is lossy, and
  compaction has measured 1/5 against 5/5 on a benchmark case. An arm that
  compacts more is handicapped for a reason with nothing to do with
  communication. **A trial must record its compaction count per actor, and a
  judged comparison must check that arms do not differ in it for reasons
  unrelated to the treatment.** `pressure.rs` (landed on `main` 2026-08-26)
  makes *when* a run compacts a prediction of the next request's size rather
  than a reading of the last one's — which changes the size of this effect
  and does not remove it, so the compaction count stays a recorded number
  here and never a constant assumed equal across arms.
- **Throughput is wall clock.** The server times a request only while it is
  running, so per-request rates hide queue wait and read about 4× high at
  `-np 1`. The spec's §15 latency metric, compared across arms with
  different agent counts, measures `-np` and queue depth. Either match
  concurrency across arms or subtract queue wait out; do not report it raw.
- **Multi-actor arms thrash the KV cache** in a way single-agent arms do
  not, because N contexts evict each other. So a control matched on *tokens*
  is not matched on *cost or time*. `cache_lens.rs` already answers "was the
  prefix actually reused"; its output belongs on the trial record.

Hence D10: a trial records `-np`, the startup line's `n_ctx_slot`, the model
alias from `GET /props` (*ask what is served, do not assert it* — the server
ignores the request's `model` field), and whether the seed was pinned. That
last one because `eval` already records the trap: a pinned seed at
concurrency 1 replays token-for-token, making k runs one sample counted k
times.

---

## 10. Deliberately out of scope

- **Training anything.** The programme is explicit that post-training is out
  of phase one, and nothing here should make a learned communication policy
  easier to reach by accident.
- **A message ontology.** Issue #60's argument is adopted: message fields are
  prompting conventions, not transport-level enums, while the useful policy
  is still an empirical question. `mailbox.rs` stays a body and an address.
- **Changing what `mecha eval` measures.** D7.
- **Any loosening of the interlock, the path jail or the sandbox for
  experimental convenience.** An experiment is exactly the context in which
  "just for this run" gets typed. If a trial is blocked by the interlock,
  that is a finding about the trial's design — and `RunStats.blocked_sends`
  already counts it, so arms that differ in it are comparing two different
  things.
- **A distributed scheduler.** The runner spawns processes on one machine
  and watches them. Multi-host is not in this.
- **Deleting anything.** Trials, branches and events are evidence. The
  branching doc already defers branch GC for the same reason.

---

## 11. Build order

Each phase is useful on its own, which is the test of whether the split is
real.

**A — Observability.** `Record::Event`; `RunStats` communication counters;
`RunConfig.experiment`; artifact edges at `ToolCtx::resolve`. No new
subsystem. This alone satisfies every acceptance criterion in issue #60 and
makes ordinary single-agent runs analysable, which is what the spec's
Phase 0 trajectory annotation actually needs.

**B — Branching, properly.** The branching doc's order of work, with the
per-chain taint rewrite done first and fail-closed. Delivers fork-and-inject.

**C — Environment snapshot and restore.** Regime 2. With B, this delivers
**oracle rescue**, which is the spec's Phase 0 step 7 and needs no
coordination machinery whatsoever.

**D — The trial runner.** `mecha exp`, actors as processes, isolated mailbox
root, topology in the schema, delivery policy, budgets.

**E — Analysis.** Bursts, uptake, divergence, and the gate extended to arm
sets.

A+B+C produces findings. Nothing before D is multi-agent, and that is
deliberate — the spec's own §47: *let observed failures determine the
architecture rather than letting the architecture determine which failures
we notice.*

---

## 12. Open at design time

1. **Where Terminal-Bench executes, and which side drives.** If `mecha exp`
   drives the benchmark, snapshot control (§6) is ours. If the benchmark
   drives mecha as a registered agent, it is not, and regime 2 may be
   unreachable. A project-specific adapter harness is expected either way;
   what it adapts in which direction is unsettled. **This blocks C, not A or
   B.**
2. **Where the communication policy lives.** Issue #60 argues against the
   tool description and against a skill; §8 rules both out on recoverability
   grounds without establishing that the system prompt is right. A composable
   prompt-layer that `RunConfig` records by name, version and hash is the
   candidate; nothing has been decided.
3. **What an arm may vary.** D5 says the closed override set, which is four
   keys. The real list for these experiments is larger — at minimum the
   policy reference, the topology, the delivery policy and the communication
   budget. Each addition is a widening of a set that two subsystems share.
4. **How the gate handles more than two arms.** A designated control against
   N treatments is not the same statistical object as a pair, and the holdout
   has to mean the same thing across all of them — which, per §1, is a
   question about *drawing* rather than about hashing: one draw shared by
   every arm, or one per arm, and only the first keeps the arms paired on the
   same episodes.
5. **Whether branch labels belong on `Record::Branch` or beside it.** §5
   proposes on the record. The counter-argument is that the session format
   then carries a field only one consumer reads.
6. **Retention for snapshots.** Per-turn container images have no policy yet,
   and the work store's rule says a policy is required before the pile
   exists, not after.

---

# Part II — Ablation, lifetimes, and the appraisal experiments

Added 2026-09-03. Part I designed an instrument around one programme —
communication — and left the appraisal system as one more subsystem the
instrument could in principle measure. This part asks what it would take to
actually measure it, finds that the answer generalises (the same shape
covers the learning loop, harness rumination, and any subsystem whose effect
lands on a *later* run rather than on this one), and changes the build order
accordingly. The store (D1), the gate (D4), the closed set (D5) and the
always-on event record (D6) carry over unchanged; §14, §15 and §16 extend
them.

---

## 13. Three questions, three units of analysis

"Does the appraisal system improve the agent" is three questions with
different experimental units, and an experiment that does not say which one
it is asking will answer none of them.

**Q1 — Is the instrument valid?** Does the readout (`appraisal::Valence`,
`Appraisal::cut_short`, the per-channel signed errors, the `Homeostat`'s
sensors) agree with an *independent* verdict about how a run went? The unit
is one run with a ground-truth verdict. No ablation and no runner: it is a
correlation over records that already exist. It is also the prerequisite
for everything below — a readout that cannot tell a failed run from a clean
one cannot improve anything downstream, and the live corpus cannot say
whether it can (`APPRAISAL-RESEARCH.md` §1: 142 neutral of 143, no task
ever closed `done`).

**Q2 — Do the consumers change future runs for the better?** Everything the
appraisal *does* acts across sessions. `worth_a_follow_up` stages a board
item after a closure. Interventions become reflections (`reflect`, per
`learning::Trigger`) and reflections become rules in the next run's cached
prefix (`learn --auto`). The homeostat's aggregates enter the
diagnostician's brief (`diagnose::Evidence`) and come out as config
overrides (`harness ruminate`). Prioritised replay by |valence| (§3.9 of the
appraisal review; a `feat/prioritised-replay` worktree exists at `main`
with no commits) would decide which sessions those probes spend their
budget on. None of it is visible inside the run it was computed from. The
unit is a **lifetime**: an ordered sequence of runs sharing one store, with
the loop's stages run between them. The dependent variable is the
trajectory over the sequence, never a point on it.

**Q3 — Do the in-run dispositions help?** Boredom, predictive compaction,
step escalation, `compact_validate`, carried state, and the audit lane's
plan re-injection and declared checks when they land. Each acts inside a
run and is on or off per run. The unit is one run, paired by task and seed
across arms — the shape `eval --ab-config` and `--ab-rules` already have.

| question | unit | already exists | missing |
|---|---|---|---|
| Q1 validity | run + verdict | `appraisal::of_session`, `RunStats`, `Homeostat`; ~170 kept Terminal-Bench sessions with test-script verdicts (§17); 36 owner draft verdicts in the live outbox | the join between a session and its verdict, and the readout run over it |
| Q2 consumers | lifetime | `MECHA_HOME` isolates a whole store (`work::mecha_home`); every loop stage is a CLI verb | the lifetime driver, the principal (§16), experiment-home admission (§14) |
| Q3 dispositions | run, paired | `eval --ab-config`, `--ab-rules`, `-k`, `candidate::judge_with` | one switch per disposition, in one closed set, recorded (§15) |

**Why frozen replay answers none of them.** `harness_probe` drops a
divergent episode rather than scoring it, on the correct argument that
replay answers from the recording. A disposition is behaviour-visible by
definition — a nudge that changes no tool call cost its tokens for nothing —
so every ablation of one diverges, and the regime that was built to grade
config knobs cannot grade the thing being asked about. Ablations are live
runs. (Part I §6 already inverts the predicate for communication, for the
same reason from the other side: there divergence is the dependent variable.)

---

## 14. Trial kinds

A trial is one of three shapes. The store, the manifest, the event record
and the gate are shared; only the driver differs.

- **`single`** — one run per (arm × task × seed × repetition). `eval`'s
  shape with the switch set (§15) in place of eval's fixed forcings. Answers
  Q3.
- **`lifetime`** — one *home* per (arm × seed × repetition), an ordered task
  sequence, and a **schedule** of loop stages between tasks: *after every
  task, `reflect`; after every fifth, `learn --auto` then `validate`; after
  every tenth, `harness ruminate`.* Sequence and schedule live in the
  manifest; the store is
  `~/.mecha/experiments/<exp>/trials/<trial>/home/`. Answers Q2.
- **`ensemble`** — N actor processes, an isolated mailbox root, topology in
  the schema: Part I §7–§8, unchanged. Communication.

**D12. Isolation is the whole store, not the mailbox.** Part I isolated the
mailbox root because that was the contamination `eval`'s `no_messages`
guards. A lifetime trial runs `learn`, and a rule learned inside a trial that
landed in `~/.mecha/learning/` would ride every real run's cached prefix
from then on — a longer half-life than any injection the interlock guards
against. So the runner sets `MECHA_HOME` to the trial's own home for every
process it spawns, and **refuses to start if the resolved home is inside
the real one** — `setup`'s rule for a workspace that contains the mecha
home, applied to the store. The arm's config *is* the trial home's
`config.toml`, written from the manifest, so nothing about an arm is
ambient. Nothing in a trial home is ever copied back.

**D13. A session in an experiment home is `SessionKind::Experiment`.**
`runlog`'s default admission hides `SessionKind::Test`, so an experiment
home whose sessions were all marked `test` would have a learning loop that
reads nothing — and remembering `include_tests` at every reader the runner
invokes is how counters go unread. The kind travels with the session, so a
file that leaked into the real store is still hidden where it would
contaminate, and admitted by default only where `MECHA_HOME` is an
experiment home. `MECHA_SESSION_KIND=test` stays what it is: the mark for a
hand smoke test against the real store.

---

## 15. The switch set: what an ablation is here

An ablation is a subsystem that is **structurally absent** from a run —
chosen by config, recorded on `RunConfig`, hashed into `condition_hash`,
and never a sentence in a prompt. `force_reproducible` in
`commands/eval.rs` is already that vocabulary: eleven `no_*` forcings, and
a test that asserts each one *or a scorecard measures this machine*.
`harness::OverrideKey` is the knob half. Three things are missing: most
dispositions have no switch beyond `[agent] boredom` and
`[agent] step_escalation`; the loop's stages have no switch because nothing
ever ran them under an experiment; and no record says which switches a run
carried except by the absence of their effects.

**D14. One closed set of levers, beside the closed set of knobs.** `Lever`
is on/off; `OverrideKey` carries a value; both are recorded on `RunConfig`
and both are what D5's "an arm may only vary the closed set" now means.
`eval`'s forcings become *every lever off except the two eval allows as
opt-in* (`--mcp`, `--ab-rules`), expressed over the set by the same
function, so `eval` and `exp` cannot disagree about what "bare" means. An
unknown lever name in a manifest is a load error, never a skipped line.

Per-run levers:

| lever | today's spelling | what turning it off removes |
|---|---|---|
| `learned_rules` | `--no-learned-rules` | the rules block from the cached prefix |
| `charter` | `--no-charter` | the charter block |
| `skills` | `--no-skills` | the level-1 skill block |
| `boredom` | `[agent] boredom` | the in-run notice (`boredom::NOTICE_STEM`) |
| `step_escalation` | `[agent] step_escalation` | the quarantined revise-the-step pass |
| `predictive_compaction` | **none** — `pressure.rs` has no off | compacting on the *forecast* of the next request; the threshold stays, because a lever may only remove a disposition above a structural check, never the check (`GOAL-SYSTEM-DESIGN.md` §7.3) |
| `compact_validate` | `[agent] compact_validate` | the summary check |
| `carried_state` | **none** — `Tool::carried_state` is unconditional | the plan block surviving compaction |
| `plan_reinjection` | unbuilt (`AUDIT-RESEARCH.md` §3.11 arm 1) | the periodic re-read |
| `declared_checks` | unbuilt (arm 2's executor) | the harness running a step's `check` |
| `appraiser` | `--appraise` on the readout only | the quarantined appraiser pass, wherever it is invoked |

Loop-stage levers, `lifetime` only:

| stage lever | verb | what turning it off removes |
|---|---|---|
| `reflect:<trigger>` | `reflect`, per `learning::Trigger` | one trigger's reflections — the way the follow-up channel (86% of live interventions, never counterfactually probed) gets its first measurement |
| `learn` | `learn --auto` | consolidation into rules; reflections are still mined |
| `validate` | `validate` | the probes, probation release, retirement |
| `ruminate` | `harness ruminate` | config overrides |
| `followup_staging` | `tasks set` → `worth_a_follow_up` | the board item after a closure |
| `prioritised_replay` | unbuilt | the \|valence\| ordering; the arm without it draws uniformly, which is `sample.rs` as it stands |
| `sensors_in_brief` | **none** | the homeostat's and guilt's entry into `diagnose::Evidence` |

**"The whole system off" is the bare arm, and it already exists** — it is
what `mecha eval` runs. The appraisal *readout* is a pure function of the
record and cannot be ablated; nor need it be, since it changes nothing by
being computed. "Appraisal off" is therefore a **preset over levers** — every
consumer off — and the manifest names presets as such, never as a lever, or
a reader a month later cannot tell what was actually absent.

**Designs, in order of what they can show.** Full-versus-bare first: the
effect size that says whether any of this is worth its tokens. Then
leave-one-out from full, because the audit's non-additivity note applies —
four re-check mechanisms already stack, and a fifth measured alone against
bare can read positive while adding nothing to the stack. Add-one-to-bare
only for a lever with a prior worth testing in isolation (plan re-injection
is the one replicated positive in the literature). A full factorial over
these levers is tens of thousands of arms; the manifest should refuse a
design whose arm count cannot be paired at the gate's minimum within the
declared budget, rather than run a fraction of it and report the fraction.

---

## 16. The principal simulator

The appraisal's richest channels are the owner's: a rejected draft, an
edited one, a steer, a denial, a closure. In the live store that is 36
draft verdicts and zero closures in a month, and every one of them was the
owner's real time. An experiment that waits for them has no N.

τ²-bench's answer, adopted: a *simulated user* in the loop, with the task
graded on end state rather than on the conversation. Here it is the
**principal** — an actor that plays the owner for a lifetime trial: closes
tasks, releases or rejects drafts, edits, steers, answers questions. Three
rules:

1. **Verdicts come from gold wherever gold exists, and from a model only
   where it does not.** A task with a `verify` command is closed `done` when
   the command passes and `dropped` when the budget is spent. A draft is
   rejected when a deterministic check on it fails — it names the wrong
   date, it is addressed to nobody on the fixture cast — and released
   unchanged otherwise. An edit is a scripted diff. The model-driven
   principal (τ²'s shape: a persona and hidden information) is the *second*
   version, for channels no check can express — tone, usefulness — and its
   variance is a recorded confound, the way the ruminate judge's correlation
   with the model under test is a recorded one.
2. **It is a separate process, writing through the owner's own verbs**
   (`tasks set`, `outbox release|reject|edit`, `questions answer`), on D3's
   argument and one more: the appraisal then reads exactly the records it
   would read from a person, through the same stores. It never writes a
   session, a reflection or a rule.
3. **Its interventions classify `Origin::Clean` inside the trial home, and
   that is the point.** The learning loop treats them as the owner's, and
   the experiment is asking whether the loop learns from an owner. It is
   also why D12 is not negotiable: the same interventions against the real
   home would be a machine authoring the owner's corrections, which is the
   one thing the charter rule and the provenance gate exist to prevent.

**It drives the CLI, and nothing else.** Every owner channel already has a
headless verb, because delegated board tasks needed one: the principal
hands each task to the agent with `mecha tasks work`, closes it with
`tasks set --status`, redirects it with `tasks steer` (the run marker's
steer file, which the loop drains exactly once), stops it with
`tasks stop`, answers a parked question with `questions answer`, and
judges drafts with `outbox release|reject|edit` (`edit` honours `$EDITOR`,
so a scripted editor applies the diff). It reads through `--json`. Those
runs record as `SessionKind::Task`, the surface the appraisal was built
for. The TUI is ruled out by its own design — steering there needs one
owner of stdin and a pty with a size — and Slack and the web surface are
owner conveniences with nothing the principal needs. **The one channel
with no headless path is denial**: `Ask` prompts a terminal, `Allow` never
denies, and a `pre_tool` hook that refuses renders as "Blocked by a hook",
which is by design never mined as a correction. A principal that denies
needs an approver that reads its decision from a file the runner owns, on
the steer file's shape — the only new mechanism the gold-verdict principal
requires.

What it makes measurable for the first time: `Channel::Commitment` (a board
task with a `due_at` the principal set), guilt's delta (a backlog the
principal grows and clears), the follow-up staging gate (closures happen),
the `Edit` trigger at volume, and the appraiser's yield against a known
verdict.

---

## 17. Datasets, by the question each answers

What makes a task good *for appraisal* is not what makes it good for a
model bake-off. Two properties. First, the outcome must sometimes be
**invisible to the counters**: a run that ends cleanly on a successful call,
under budget, without compaction, and is wrong. That is the case
`GOAL-SYSTEM-DESIGN.md` §8.2 says only affect-prioritised replay can reach,
and a dataset with none of them cannot test the claim. Terminal-Bench has
them by construction — a test script fails a run the model declared done —
and the bake-off cases mostly do not, because a substring grader and a
clean stop cause tend to agree. Second, **the ceiling must have a truth**:
`cut_short` says the run was cut off, and only a task with a known
solution length can say whether cutting it off lost anything.

Ranked:

1. **The kept Terminal-Bench sessions.** About 170 session files under
   `jobs/mecha-arm64-subset/<run>/<task>__<id>/sessions/`, from four runs
   between 2026-08-07 and 2026-08-11, with Harbor's per-trial verdict beside
   each. **Q1, offline, today, zero model calls**: run `of_session`, the
   counters and the homeostat over each, join to the verdict, and report
   discrimination per channel and for `Valence`; then the appraiser's
   marginal yield (§3.10 of the appraisal review) on a subset. Caveat:
   recorded by the 0.1.2–0.1.6 loop, so fields that did not exist read
   `None` — the correct reading, never to be filled in.
2. **`eval/cases.jsonl`** — 70 cases, 15 tags, deterministic graders, a
   minute or two each. Q3 at k=5: the per-run levers, paired. Too small and
   too clean for Q2.
3. **A Terminal-Bench subset as a lifetime sequence** — twenty tasks in a
   fixed order, about four hours per lifetime at the measured rate, a
   container per task (regime 2 for free, since the container *is* the
   environment). Q2 for the coding channels: do the intervention rate, the
   tool-error rate and the pass rate move over the sequence with `learn` on
   versus off. The principal is trivial here: the test script is the
   closure.
4. **A synthetic assistant home** on the fictional cast — mail and calendar
   fixtures, a board with due dates, an outbox, a charter — the environment
   the owner channels need and the one nothing in the repo has (the docs
   site's fixture-backed demo, PR #117, is the nearest seed;
   `eval/fixtures/` holds workspaces, not homes). Q2 for commitment, guilt,
   charter and follow-up staging; needs the principal. This is the dataset
   to *build*, and the one only this project can.
5. **AgentDojo** — the interlock's false-refusal cost beside its catch rate.
   A lever set that changes `blocked_sends` is a different experiment, and
   this is the dataset that prices it.
6. **The live corpus** — observational only, and the calibration target for
   the synthetic home: if the principal's rejection rate and the owner's
   differ by an order of magnitude, the synthetic home is measuring a
   different owner.

---

## 18. Metrics and analysis

- **Every metric is lower-is-better** (D4), and **no appraisal quantity is
  ever an objective** (`GOAL-SYSTEM-DESIGN.md` §8.3). Valence, labels,
  guilt and boredom notices enter a trial record as *covariates*, and as
  Q1's *predictions*; the gate never sees one as a cost. The reason is the
  null run: an agent graded on its own appraisal optimises the appraiser.
- **Q1**: discrimination (AUROC) and calibration per channel against the
  verdict; the appraiser's added yield over the deterministic record.
- **Q2's primary outcome is the correction rate over the lifetime** —
  interventions per run, by trigger, against position in the sequence —
  which `learning-report` already computes for the real store. Secondary:
  pass rate and the `Metric` set per position. Report the **slope**, paired
  across arms by position, not the mean: a loop that learns has a slope, a
  loop that does not has a mean. Cost includes the loop's own tokens
  (`reflect`, `learn`, the probes) — a lifetime arm that learns slightly and
  pays a night's replay every five tasks has a cost the single-run number
  hides.
- **Q3**: pass^k at k ≥ 5, paired by (task, seed), the gate's counts. The
  seed trap `eval` records applies: a pinned seed at `-np 1` replays
  token-for-token, so repetitions vary the seed and record it.
- **N.** The gate's floors — eight paired episodes in the selection slice,
  four in the holdout — are the minimum. A lifetime is one episode per
  *position*, so five lifetimes of twenty tasks pair a hundred points per
  arm, enough for a slope. At the measured rates, Q3 over the bake-off set
  is about twelve hours per arm at k=5, and one Q2 lifetime pair about
  forty. Nightly-scale, which is why the runner must resume per trial (the
  trigger runner's `.running` marker and pid-range check, unchanged).
- **Confounds** are §9's, plus one: **the loop's stages are model calls on
  the same server**, so an arm with `learn` on contends for slots with the
  task runs and its wall clock differs for a reason that is not the
  treatment. Stages run between tasks, never beside them.

---

## 19. What this changes in the build order

§11 ordered observability → branching → snapshot → runner → analysis,
because branching is the substrate for message interventions. The appraisal
questions need none of that: Q1 needs no code, Q3 needs the switch set, Q2
needs the lifetime driver and the principal. The order becomes:

- **0 — Q1 offline**, on the kept sessions. A script over existing readers
  (in `scripts/`, on `build-eval-fixtures.py`'s precedent); the result goes
  into `APPRAISAL-RESEARCH.md` beside its §1 table. **If the readout has no
  discrimination, stop here and fix the readout** before spending a
  lifetime on its consumers.
- **A — Observability**, unchanged, plus `RunConfig` recording the lever
  set and `SessionKind::Experiment`.
- **A′ — The switch set.** `Lever`; the per-run switches that are missing
  (`predictive_compaction`, `carried_state`, `appraiser`, `sensors_in_brief`);
  `force_reproducible` re-expressed over the set with its test intact.
- **D₁ — `mecha exp` with `single` and `lifetime`.** The store, the
  manifest, the isolated home, the stage schedule, resume. Delivers Q3 and
  the coding half of Q2.
- **P — The principal**, gold-verdict version, and the synthetic assistant
  home. Delivers the owner half of Q2.
- **B, C, D₂, E** as in §11 — branching, snapshot, `ensemble`, analysis.
  Communication starts here and inherits the store, the levers and the
  gate.

Each step is useful alone, which is still the test of whether the split is
real: step 0 is a finding by itself, A′ makes today's `eval` honest about
what it forces, D₁ is a runner other subsystems (the learning loop, harness
rumination) can be ablated under without anything in P.

---

## 20. Open, and named so it is not rediscovered

- **One enum or two.** `Lever` and `OverrideKey` as one set is D5's
  spirit; a value-carrying knob and an on/off switch validate differently,
  and `ConfigChange` already carries a value. Undecided; the recorder is
  shared either way.
- **The model-driven principal.** Which model, whether its persona is part
  of the manifest (it must be, for the artifact to be recoverable), and
  whether a principal driven by the model under test is the confound the
  gold version exists to avoid or an acceptable one on the ruminate judge's
  precedent.
- **Real scripts or bare verbs for the stages.** `ruminate.sh` carries
  policy (`validate --unprocessed-only` *before* `learn`, so rules are not
  graded on their own training data); the verbs are what a lever switches.
  Probably the verbs, with the script's ordering restated in the manifest
  schema and a test that the two agree.
- **Retention for lifetime homes.** Each is a full store. Small at this
  scale; the work store's rule says a policy before the pile.
- **Where the synthetic assistant home lives, and who maintains the cast.**
  The no-real-people rule makes it a fixture that has to be authored, not
  sampled.
- **Whether Q1's join belongs in `mecha sessions appraise`** as a
  `--verdicts <file>` that reports discrimination, or stays a script. A
  flag makes the validity check repeatable on every corpus; a script keeps
  the readout's surface from growing a grader.

---

## 21. Where the pieces live — proposed 2026-09-03, not yet ruled

Spitballed with the owner the same day Part II was written; recorded so
the split is argued once.

**The rule.** Anything the harness must *trust or refuse* lives in mecha;
anything that only *reads* mecha's artifacts or *plays a role outside* the
loop lives in a separate scaffolding repository. That is §0's premise
stated as a boundary: an experiment is analysable from artifacts alone, so
the artifacts are the only interface.

| in mecha | in the scaffolding repo |
|---|---|
| `Lever` and its recording on `RunConfig`; `SessionKind::Experiment`; `Record::Event`; the isolated-home refusal (D12); the file-driven approver (§16); `mecha exp`'s core — manifest and trial store, spawn/resume, the stage schedule, `judge` over the gate | task adapters (Terminal-Bench via Harbor, AgentDojo); the principal; the synthetic assistant home and its fake mail/calendar MCP server; analysis past the gate's counts (discrimination, slopes, pass^k); the dashboard. Python, as `bench/mecha_agent.py` and the MCP fixtures already are; `bench/` likely migrates |

A trial record a script could fabricate is not evidence, which is why the
store's only writer is the binary. **The checkable rule, on
`mecha-slack`'s precedent: the scaffolding never links `mecha-core`.** It
drives mecha through the CLI with `--json` and reads the experiment store's
files; anything it needs beyond that is a missing `mecha exp` verb, never
an import.

**The dashboard is the scaffolding's, not `mecha serve`'s.** The web
surface is the owner's page against the real home, and a launch-arms
control beside the outbox is the wrong neighbour. Svelte 5 runes, tailnet
only, every mutation a child `mecha exp` process (`/tasks`' rule). Screens:
experiments with manifest, arms, status and budget; a live trial view (run
markers, turn and token counts, the session tail, the event timeline); arm
comparison with the gate's tally and §9's confound panel (compactions per
arm, `-np`, `n_ctx_slot`, `/props` alias); probe logs per lifetime stage;
a transcript viewer over session JSONL (chain comparison once §5 lands);
export as a zip of trial homes plus manifest and results.

**HyperStudy** (the owner's human/agent experiment platform) was weighed
and kept separate. Its data model matches — experiment, roles, rooms of
two to five participants, variables, export, analytics, and LLM agent
participants through `hyperstudy-agent`'s llama-server endpoint; a room is
the `ensemble` kind — but its runtime is a browser, LiveKit and Firebase
around synchronised media, which a trial of headless processes writing
JSONL would carry for nothing. Two things borrowed: its vocabulary, so the
manifest maps one-to-one and a later bridge is cheap; and the bridge
itself, which already exists — `mecha voice-serve` is an OpenAI-compatible
chat endpoint over the agent loop, the contract `hyperstudy-agent`
verifies, so mecha can be a HyperStudy participant in human-plus-agent
coordination studies without any work on this side.

### 21.1 Who drives a lifetime, and what a task is — recommended, not ruled

**Recommendation: mecha owns the trial; the scaffolding owns the task
source and the principal, as executables mecha spawns.** `mecha exp run`
drives a lifetime — isolated home, task order, stage schedule, resume,
records — because those are what make the trial's artifacts trustworthy
and they should be recorded by the same binary that enforces them. The
two things it cannot know are plugged in through a contract, on
`hooks.rs`'s shape (a command at a lifecycle point, JSON in and out,
fail-closed):

- a **task source**: `list`, `setup <task> <workspace>`, `grade <task>` →
  `{passed, detail}`;
- a **principal**: `act <trial-state>` → the verbs it ran.

Against the alternative — a Python orchestrator that calls `mecha run`
per task — the costs are honest: Rust iterates slower than Python for
experiment logic, and the Terminal-Bench backend means mecha spawning a
container with mecha inside it (the outer is the experimenter, the inner
the subject; the home is a mounted volume). The Python orchestrator's
costs are the ones §21's rule exists for: trial records a script can
write, a stage order the harness never recorded, and a second
implementation of resume, markers, permits and the isolation refusal —
the silently-degrading-guard shape, in a language with no compiler to
find the sites. `permit.rs` is the concrete reason: stages and task runs
must not contend for llama-server seats (§18), and the seats are mecha's.

This also settles Part I's open question 1 for the appraisal work:
**mecha drives, and Harbor's runner is used only for leaderboard
submissions** (`bench/`), where the benchmark must drive for
comparability. Two drivers, one task format.

**The task format is Harbor's, and the trace graders are mecha's.** There
is no standard eval interface; there are two de-facto ones for agents and
everything else is bespoke. Harbor's task directory — `instruction.md`,
`task.toml` (timeouts, resources, `[environment] mcp_servers`),
`environment/Dockerfile`, `tests/test.sh`, `solution/` — is the one the
Terminal-Bench leaderboard runs and the one other terminal datasets are
being ported to. Inspect AI (`inspect_evals`) is the more general one —
dataset, solver, scorer, docker sandboxes, dozens of benchmarks including
AgentDojo — but its agent is a Python solver or a bridge to one, so it
stays an adapter target (`BENCHMARK-RESEARCH.md`), not the base format.
τ²-bench, SWE-bench, BFCL and HAL each carry their own.

So: **our own tasks are written in Harbor's format**, which makes them
runnable by Harbor against other harnesses (little-coder, Terminus) on
the same model with no extra work — the only payoff of a standard worth
having here — and by `mecha exp` for lifetimes. `task.toml`'s
`mcp_servers` is exactly where the synthetic assistant home's fake mail
and calendar servers are declared, and `tests/test.sh` grades store state
through `mecha … --json` inside the container. `eval/cases.jsonl` stays
for what no public format has — assertions on the *trace*: tools called
and in what order, arguments, `no_tools`, stop cause, taint,
`blocked_sends`, `min_compactions`, `ended_on_failed_call` — and the
scaffolding carries one grader that applies the same `expect` block to
any session JSONL, so a Harbor task can carry trace expectations beside
its test script. The task source reads both.

### 21.2 AgentDojo is an MCP server away, and it is the assistant environment's seed

Checked 2026-09-03 against its documentation. A suite is a `TaskSuite`
over a pydantic `TaskEnvironment` loaded from `environment.yaml` with
injection placeholders; tools are typed Python functions with `Depends`
for state, executed by `FunctionsRuntime.run_function(env, name, args)`;
a user task grades with `utility(pre, post, output)` and an injection task
with `security(pre, post)`. Nothing requires its agent pipeline. So the
adapter is **one Python MCP server** wrapping a loaded environment —
schemas from the type hints, one process per task, connected to mecha as
an untrusted server on `mcp.rs`'s ordinary path so the interlock sees
exactly what it would see in production — and a task source whose
`grade` calls the two functions on the end state. That is the second
task-source backend beside Harbor, and the only one AgentDojo needs.

The larger consequence: the **workspace suite is a synthetic mail,
calendar and drive environment with fictional data by construction**, and
its injection vectors are the third-party content the trifecta design is
about. §17's item 4 — the synthetic assistant home — should start from it
rather than from a fixture written by hand: AgentDojo's environment
served over MCP for the world, mecha's own stores in the trial home
(board, outbox, questions, charter) for the owner channels. Its tool
schema differs from `mecha-mail`'s, which is a recorded condition, not a
problem: the appraisal's owner channels never read a mail tool.
