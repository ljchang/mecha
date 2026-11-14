# A benchmark for agent memory: does experience change an outcome?

2026-11-12, status added 2026-11-13. **Plan for the owner's review.** Built so
far: the memory axis on an arm, the opaque-service lifetimes (tool quirk and
two lessons, two worlds), and the first four arms in §3. Step 1 of
[MEMORY-RETENTION-DESIGN.md](MEMORY-RETENTION-DESIGN.md) §7. It is built on two
things that exist: `mecha exp` in lifetime mode
([EXPERIMENT-DESIGN.md](EXPERIMENT-DESIGN.md)) and the controls of the
lifetime harness the memory prototype was measured with
(`prototypes/memory/ordinary_lifetime.py`).

## 1. What it has to show

One question per arm pair: with the same model and the same budget, does the
agent reach a verified outcome it did not reach without that kind of memory,
and what did the memory cost. Chat-recall benchmarks cannot answer it, and
the prototype's own pilots sat at ceiling (6/6 against 6/6), so the cases here
are built to fail cold.

## 2. Shape

A **lifetime** is an ordered list of tasks in one isolated home. Early tasks
are *experience*: the agent does work and something happens. Later tasks are
*probes*: they can be solved well only by using what the earlier ones showed.
A probe is graded on the state of the fixture world or a saved artifact, by
code, against an answer key the agent cannot read.

`mecha exp` already provides this: one home per arm, seed and repetition that
persists across the task list; stateful mail, calendar, board and document
worlds with a clock per task; a task source with `list`, `setup` and `grade`,
where `grade` reads the world's state. The workspace is restaged for every
task, so anything that must carry over lives in a fixture store or in memory,
which is the property the benchmark wants.

## 3. Arms

| Arm | What the agent has | Exists today |
|---|---|---|
| **none** | No memory of earlier tasks. | Yes |
| **transcripts** | A keyword search over its own earlier transcripts, nothing curated, made by the harness for the task and handed over beside the prompt. The honest baseline: is plain retrieval enough. | Yes (`tool::transcripts`, `mecha run --transcripts-deliver`) |
| **everything** | The prototype's recorder: every passage of every run stored and recalled by keyword, when the model asks. | Yes (the integration slices) |
| **delivered** | The same store, and the harness recalls for the task before the first request and hands the model what it found. | Yes (`mecha run --memory-deliver`) |
| **episodes** | Appraisal-gated episodes only. | To build (design step 2) |
| **concepts** | Episodes plus relational concepts, recalled first. | To build (steps 3 and 4) |
| **recompose** | The above plus role binding across episodes. | To build (step 5) |

The first four run before anything new is built, and are the baseline every
later arm must beat. **Recall is delivered by the harness, not left to the
model** (owner, 2026-11-13): in the first pilot the store held every earlier
task and the model never asked it, so an arm that waits to be asked measures
whether the model asks. Later arms deliver too. `eval/fixtures/opaque_readout.py`
reports, per probe, whether the rule it needed was in what was handed over. All arms get the same tools apart from memory, the same
turn and token caps, and the same model.

## 4. Task families

Each family is a template with two or three instances that differ in surface
content (names, domain, wording) and share the hidden structure.

**Tier A, assistant work** (mail, calendar, board, documents; fictional staff).

| Family | Experience | Probe | Passes when |
|---|---|---|---|
| Goal switch | A route that was fast and lost detail. | The goal now values the detail. | The slower route is chosen. |
| Recombination | Two unrelated episodes, each holding half of a method. | A task that needs both halves. | The combined method is carried out. |
| Refused analogy | An approach that worked. | A look-alike case where a stated rule forbids it. | The approach is not used, or the owner is asked. |
| Contradiction | A pattern seen twice, then a counterexample. | A case on the counterexample's side. | The revised expectation is followed. |
| Changed state | A request handled when capacity was free. | The same request when a charter sensor reads over its setpoint. | The cheaper or deferring action is taken. |
| Far analog | An episode in one domain. | A structurally identical case in another domain with no shared wording. | The earlier method is applied. |
| No change | Ordinary work. | The same kind of work, nothing changed. | Passes as well as without memory, at no worse cost. |

**Tier B, out of reach cold** (files, shell, a fixture tool with hidden
behaviour). Built so the no-memory arm fails or exhausts its budget.

| Family | Experience | Probe | Passes when |
|---|---|---|---|
| Tool quirk | A tool fails in an undocumented way; the working call is found late. | A new task that needs that tool, with a budget too small to rediscover it. | The working call is made early. |
| Split repair | One session observes the cause of a fault. | A later session must fix it, with the cause no longer observable. | The fix addresses the recorded cause. |
| Two lessons | Two tasks each teach one constraint. | A task that violates neither only if both are applied. | The artifact satisfies both. |
| Long task | A task long enough to compact, then a restart. | Continue it. | The work completed before the cut is not redone or contradicted. |

"Far analog" is the family that separates structure from labels: keyword and
embedding retrieval should miss it and structural matching should not. If no
arm separates there, the concept design's main bet has failed.

Far analog is built as three pairs of services, each one server with two
faces: `eval/memory-protocol.toml`, `eval/memory-works.toml` and
`eval/memory-grounds.toml`. In each, the experience face documents an
arrangement and the probe face has other tools, other argument names, several
unlabelled candidates for each step and no documentation; a task passes on
the service's state, when the change was made and the thing was put back.
The second and third pairs were each written before the mechanism they
tested had been run, and run once. All three have since been read closely,
so a further change to `tool::concepts` needs a fourth.

A second kind of arrangement has two more pairs on one server
(`eval/fixtures/twins_world.py`; `eval/memory-twins.toml` for development,
`eval/memory-twins-held.toml` held out and run once): a change that must be
made again on a second thing the first thing's record names. The probe face
answers a change on one thing as a success, so a run that stops there is told
nothing. These pairs depend on the experience tasks more than the bracket
pairs do: the experience face marks the omission with a code and the model
often does not look the code up, and a lifetime whose experience tasks both
fail has no arrangement to carry.

A third pair on the same server (`berths` and `pumps`,
`eval/memory-twins-reappraised.toml`) is held out for the reappraisal of
failed tasks and was run once. Its experience face answers a change on one
berth with a plain `ok`: only a `state` field in the record and a guide the
run has no prompt to open say that a partner is owed, so the first task
fails and the question is whether that failure repairs the second.

## 5. Controls

Carried over from the prototype's harness, because `mecha exp` lacks them:

- The answer key and the binary are frozen by hash before a run, and the key
  is never opened while it runs.
- Every planned task stays in the denominator, failed and interrupted ones
  included; nothing is retried.
- Usage is measured per task. Consolidation and learning are charged to the
  arm that did them, not left out of the total.
- A probe counts as using memory only if recall was delivered to it.
- Families are split into a development set and a held-out set **by family**,
  not by seed or instance. Tuning is done on development; a claim is made on
  held-out.

## 6. What `mecha exp` needs

1. **A memory axis on an arm.** An arm can set levers, knobs and an
   environment, not a flag, and the memory path refuses to run when the
   session is marked as an experiment. An arm gets a memory setting; the
   driver creates the store under the lifetime's home and carries it across
   the tasks; the refusal admits an experiment.
2. **A cap per task** on wall time and tokens. There is none on the run
   child.
3. **Cost in the report:** tokens and time per task, and the consolidation
   and learning stages.
4. **World-state grading without a principal**, for tasks with no owner
   stand-in. A source's `grade` already does this; the manifest's
   postconditions do not.
5. **The transcript search tool** for the second arm.

## 7. Measures

Per arm and per family: probes passed; probes passed among those where recall
was delivered; turns, tokens and seconds per task; consolidation cost; the
no-change family's cost against the no-memory arm. For concept arms also:
predictions made and confirmed, matches whose label similarity was low, and
agreement between two extractions of one episode.

Pairs are matched on family, instance, seed and task. With this many cases
the report gives paired wins, losses and ties and a bootstrap interval; it
does not claim a general rate.

## 8. Order of work

1. The instrument changes in §6, items 1 to 4.
2. Port the prototype's seven-episode family as the first source, to check
   the runner against a result that already exists.
3. Tier B, two families (tool quirk, two lessons): the quickest to author and
   the clearest cold failures.
4. Run **none**, **transcripts** and **everything**. This is the baseline
   report, and the first real measurement of what the integration slices
   bought.
5. Tier A families, far analog first.
6. Freeze the held-out families before any new memory arm is tuned.

## 9. Stop rules

- If **transcripts** matches **everything** on the baseline run, curated
  storage has not yet earned its cost and the first target is retrieval.
- If, on held-out families, **concepts** and **recompose** do not beat
  **episodes** given the same source episodes, the relational layer has not
  earned its place as built, and it is redesigned before more is added.
- If the no-change family gets worse or costlier with memory on, that is a
  regression to fix before any gain is reported.
