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
| The gate | `candidate.rs` | Paired by episode, selection slice plus a holdout split by **hash of the episode id, never random**, a work guardrail that outranks the score, single-arm episodes dropped, counts rather than a significance test. Pure and unit-tested. |
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
   split has to mean the same thing across all of them.
5. **Whether branch labels belong on `Record::Branch` or beside it.** §5
   proposes on the record. The counter-argument is that the session format
   then carries a field only one consumer reads.
6. **Retention for snapshots.** Per-turn container images have no policy yet,
   and the work store's rule says a policy is required before the pile
   exists, not after.
