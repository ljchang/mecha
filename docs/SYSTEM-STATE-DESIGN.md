# System state — design

**2026-10-10. Proposed; R1, R2, R3 and R3b ruled the same day, R4 open
(§9).** `mecha system` is one layer through which mecha reads the state of
the machine it runs on and the services it runs beside. Each measurement in
it has its own specific name, drawn from one closed list. Every consumer
reads the system through it, and none probes for itself:
- the homeostat;
- the HUD;
- the doctor;
- the situation brief;
- the gates;
- a model tool, once built.

This document covers machine state: memory, CPU, GPU, disk, the model
server, and the runs in flight. The goal system's charter has *sensors*
(`charter::SensorKind`), which are a different thing. §1.3 explains why,
and the naming keeps the two apart: nothing in this layer is called a
sensor.

---

## 0. Why: the system is read in many places today

An inventory of `origin/main` at `5669f2579`, plus the host sampler on #630,
found every reader below written for one consumer:

| Source | Readers today |
|---|---|
| `/proc/meminfo` | **Four parsers**: `homeostat::mem_available_kb`, `imagegen::mem_available_mb`, `recommend::parse_meminfo_total`, `hud::host::parse_meminfo` |
| `/proc/loadavg` | Two: `homeostat::load_avg_1m`, `hud::host::parse_loadavg` |
| `nvidia-smi` | `recommend::nvidia_smi` runs it under a 10 s kill timeout. `hud::host::run` runs it with **none**, so on a wedged driver only its unit's `TimeoutStartSec` stops it. `model-idle.sh` and `comfyui-idle-reset` each call it themselves |
| llama-server `/slots` | `brief::slots_of`, plus `scripts/model-idle.sh` restating its rule in shell |
| llama-server `/props` | `provider::preflight::fetch`, `router::is_router`, `scripts/served-props.sh` |
| "is a voice call live?" | **Two answers that can disagree**: `brief::VoicePresence` reads a stamp file, while `replay_persona::voice_live` greps the TTS journal for `RTF` |
| `/health` waits | Five separate implementations: `engine_gate`, `llama_units`, `router_unit`, `document`, `mecha-wait-healthy` |
| Unit files on disk | `feature::Facts::read` searches one directory; `sidecar::Machinery::check` searches seven |

Each copy re-decides the questions that carry the risk:
- Is an unreadable source a zero, or unknown?
- Does the read have a timeout?
- Can the probe load a model, or wake a server it only meant to watch?

The HUD sampler shows what that costs. Three review passes on #630 each
found the same mistake one level further down: a value that could not be
read was recorded as a believable number.
1. An unreadable `/proc/meminfo` or cgroup tree read as an idle machine.
2. The core count came from the sampler's own parallelism.
3. A missing `memory.current` read as zero bytes.

Every reader above has to get those same rules right on its own.

The goal design asked for this. `GOAL-SYSTEM-DESIGN.md` §13.3 is "one
scan, three views": the scan is the expensive part, so the readers should
differ only in what they compute from it. Its §4.1 tables what there is to
read, by cost. This document is that scan, for machine state.

## 1. What a measurement is

### 1.1 A name, a unit, a cost, and a reading that can say "unknown"

A measurement is an entry in one closed enum, `system::Measurement`.

**Its name** is dotted and specific: `memory.available`, `gpu.temperature`,
`model.slots.busy`. The name is a wire format, exactly as
`hud::host::Category` is:
- its discriminants are pinned by a test;
- it is spelled the same on the CLI, in the series and in the tool;
- a name this build does not know reads back as `None`, never as a
  neighbour.

**Each entry declares:**
- **Unit:** bytes, percent, count, °C, watts, or a closed label set.
- **Cost tier**, one of four:
  - `File`: a read of `/proc` or `/sys`.
  - `Loopback`: one HTTP call to a local server, always with a timeout.
  - `Fork`: a subprocess, always with a timeout.
  - `Scan`: a directory walk.
- **Recordable:** whether its value may enter the series (§1.2). A
  measurement whose value is a *name*, such as the resident model, is
  probe-only.

**A read keeps unknown apart from zero**, which is the rule every reader
above already states for itself:

```rust
pub enum Reading<T> {
    Observed(T),
    /// The source exists here and could not be read now; `why` is for the
    /// doctor and the CLI, never a value.
    Unread { why: String },
    /// This machine does not have it: no NVIDIA GPU, a provider that is
    /// not local, a unit that is not installed.
    NotHere,
}
```

`NotHere` and `Unread` are opposite findings. "There is no GPU" is a fact
about the machine. "The GPU did not answer" is an incident, and only the
incident belongs in the doctor.

### 1.2 Two layers: the probe and the series

**The probe** reads now: `system::read(Measurement) -> Reading`.
- Parsers are pure functions of the text a source printed, kept apart from
  the I/O that fetched it. That split is how `hud::host` is built
  (`parse_meminfo` and `parse_gpu` beside `collect`), and it is why every
  parser can be tested on fixtures.
- **Each source has exactly one parser.**

**The series** is what the per-minute sampler records, in
`~/.mecha/system/series.sqlite`.
- It holds numbers and closed labels only, with rollups and retention.
- It is #630's `host.sqlite`, generalised and moved (§7, S0).

Who reads which:
- **History** comes from the series: the HUD, `mecha system read --since`,
  and the minutes a run spanned.
- **Now** comes from the probe, or from the series' latest minute when that
  minute is fresh (§3.1).

### 1.3 Not the charter's sensors

`charter::SensorKind` is a closed set of observables the owner sets
setpoints against. Its rule is that **every kind is an observable a store
holds, with an id per item that a run's own trace can touch.** Attribution
(`appraisal::of_session`) joins on that id, never on a before-and-after
delta of the store (`GOAL-SYSTEM-DESIGN.md` §11.1, containment 6).

A machine measurement has no per-item id: no run owns "memory available".
So a measurement cannot be a charter sensor under the charter's own rule,
and this design does not try to make it one.

The two are kept apart by words as well as by types:
- The charter keeps "sensor": `[line.sensor]`, `SensorKind`, and its
  surfaces.
- This layer uses "system", "measurement" and "reading".
- Neither module imports the other.

The owner's ruling (R1) is to minimise the clash for now and allow the
naming to change later. If it does change, this section is the boundary to
preserve: attributable versus not attributable. The vocabulary can move;
that line cannot.

## 2. The measurements

These are the initial names, grouped by what they describe. "Series" says
whether the sampler records the value. "Today" says who reads that source
now.

| Measurement | Source | Unit | Cost | Series | Today |
|---|---|---|---|---|---|
| `memory.total` | `/proc/meminfo` `MemTotal` | bytes | File | yes | recommend, hud |
| `memory.available` | `/proc/meminfo` `MemAvailable` | bytes | File | yes | homeostat, imagegen, hud |
| `memory.swap_used` | `/proc/meminfo` swap | bytes | File | yes | hud |
| `cpu.busy` | `/proc/stat` aggregate line | % | File | yes | hud |
| `cpu.cores` | `/proc/stat` `cpuN` lines, never `available_parallelism` | count | File | yes | hud |
| `cpu.load_1m` | `/proc/loadavg` | load | File | yes | homeostat, hud |
| `cpu.tasks` | `/proc/loadavg` | count | File | yes | hud |
| `disk.root.used`, `disk.root.total` | `statvfs("/")` | bytes | File | yes | hud |
| `gpu.utilization` | `nvidia-smi --query-gpu` | % | Fork | yes | hud, model-idle.sh |
| `gpu.temperature` | `nvidia-smi --query-gpu` (the hottest GPU) | °C | Fork | yes | hud |
| `gpu.power` | `nvidia-smi --query-gpu` (summed, only when every GPU answered) | W | Fork | yes | hud |
| `gpu.unified` | memory total `[N/A]` on every GPU | bool | Fork | yes (sticky: a property of the machine) | hud, recommend |
| `category.memory` | user cgroups `memory.current`, plus GPU memory on unified hardware, per `Category` | bytes | Fork | yes | hud |
| `category.cpu` | user cgroups `cpu.stat`, per `Category` | % | File | yes | hud |
| `category.tasks` | user cgroups `pids.current`, per `Category` | count | File | yes | hud |
| `category.gpu_memory` | `nvidia-smi --query-compute-apps`, per `Category` | bytes | Fork | yes | hud |
| `model.slots.busy`, `model.slots.total` | `GET /slots?autoload=false` on :8080, local provider only | count | Loopback | yes | brief, `mecha model use`, model-idle.sh |
| `model.resident` | `GET /models` on the router | a model id | Loopback | **no: a name** | router, `mecha model list` |
| `runs.live` | `runmarker` dirs, plus pid checks | count | Scan | yes | brief, serve board |
| `holds.live` | the `hold` dir, plus pid checks | count | Scan | yes | follow, `mecha model` |
| `permits.live` | the `permit` dir, plus pid checks | count | Scan | yes | brief |
| `voice.live` | **one answer**, chosen in S2 (§6) | bool | File | yes | brief and replay_persona, which disagree |
| `image.loaded` | ComfyUI `GET /system_stats` | bool | Loopback | yes | imagegen |
| `services.failed` | `systemctl --user --failed` | count | Fork | yes | CLI doctor |
| `backlog.outbox`, `backlog.questions`, `backlog.frontdoor`, `backlog.proposals`, `backlog.candidates` | `backlog::Backlog::survey`, one per field | count | Scan | yes | homeostat, charter readings |

**A `category.*` measurement is one name with a closed dimension.**
- `mecha system read category.memory --by category` gives one row per
  `Category`, so it is one name, not seven.
- The dimension's values are the enum's labels and nothing finer:
  `chat_model`, `embeddings`, `ocr`, `voice`, `image_gen`, `mecha`, `other`.
- That is `LIVE-DASHBOARD-DESIGN.md` §0 R13, applied to the whole layer.

**A name is added by a reviewed change, never at runtime.** The list is an
enum, so a new measurement takes four things:
- a new variant;
- a pinned discriminant;
- a parser with fixtures;
- a row in this table.

**Not readable today: prompt-cache evictions.** This is the one homeostatic
variable `GOAL-SYSTEM-DESIGN.md` §4.5 names to add. Llama-server reports it
on `/metrics`, but on 2026-10-10 the router answered `GET /metrics` with
HTTP 400, and no script passes `--metrics`. Adding
`model.cache_evictions` therefore takes two steps:
1. Add the flag in `scripts/start-router.sh`.
2. Take the counter's name from a live response, not from memory.

**Never probed: the on-demand servers.** Embeddings (:8081) and OCR (:8085)
sit behind systemd sockets (`LLAMA-SERVER.md` §Document OCR), so any
connection to their ports **starts the model**. A per-minute "is it up?"
check would therefore load them every minute. Their measurements read the
unit's active state from systemd instead, which observes without waking
anything. The router has the matching rule: every `Loopback` read of it
sends `autoload=false` on anything that names a model.

## 3. Who reads what

### 3.1 The homeostat (R4, open)

Today `Homeostat::at_start` reads the load average and `MemAvailable`
itself. It does not read `/slots`, because that is an HTTP call, and the
homeostat is sampled wherever a front-end records a session.

With the series, it can read the **latest minute** instead of probing,
whenever that minute is under two minutes old.
- That costs one indexed SQLite read.
- It brings in what the homeostat could not afford to probe: slot
  occupancy, per-category memory and GPU load.
- When the series is stale it falls back to the `File` tier, so it still
  works with the sampler stopped.

A run's start and end then select its minutes from the series. The record
says what the machine was doing *while the run ran*, not just at the
instant it began. That is the distinction the appraiser needs to tell
regret from disappointment (`homeostat.rs` module docs).

**Unchanged:** only front-ends attach the homeostat. `mecha eval`, batch
and the replay probes never read live state, and anything reconstructing a
run reads the recorded snapshot.

### 3.2 The HUD

The only change is a path. The `host` board's source becomes
`kind = "sqlite"` over `~/.mecha/system/series.sqlite`, and its loaders keep
their SQL. The privacy property R13 demands now belongs to the whole series
(§4, rule 2), not to one board.

### 3.3 The doctor

- **The sampler has stopped:** the newest minute is more than ten minutes
  old. This is #630's `check_host_sampler`, moved.
- **A measurement stays unread:** it has been `Unread` in every minute of
  the last hour after being `Observed` before. That is a finding, and it
  names the measurement and the last `why`. `NotHere` never produces one.

### 3.4 The brief, the gates and the scripts

- **The brief:** `brief::read_slots`, `seats_under`, `runs_under` and
  `VoicePresence` become calls into `system`. Its own rule is unchanged: it
  reads before assembly, so `brief::assemble` does no network.
- **The gates:** the image tool's memory gate reads `memory.available` and
  `image.loaded` through the same probe.
- **The scripts:** `model-idle.sh` and `comfyui-idle-reset` call
  `mecha system probe <measurement> --json`, and stop parsing `/slots` and
  `nvidia-smi` themselves.

### 3.5 The CLI

| Command | What it does |
|---|---|
| `mecha system` | Every measurement's latest reading: value, `Unread: why` or `NotHere`, and its age |
| `mecha system read <measurement> [--since 6h] [--step 5m] [--by category] [--json]` | One measurement, sliced from the series |
| `mecha system probe <measurement> [--json]` | Reads now, bypassing the series. This is the diagnostic, and it works with the sampler stopped |
| `mecha system sample` | The per-minute writer (#630's `mecha hud sample`, moved), run by `mecha-system-sample.timer` |

**A prefix lists a group:** `mecha system gpu` shows every `gpu.*`
measurement.

A name or prefix that matches nothing is an error that lists the valid
names. It never prints an empty table.

### 3.6 The model (R3, ruled; §5)

## 4. Rules

1. **Unknown is never zero.**
   - Every read returns a `Reading`, so the distinction lives in the type.
   - The sampler refuses a sample when a denominator cannot be read: the
     memory total, the cgroup tree, the core count.
   - A minute it cannot fully attribute is recorded with NULLs, never with
     a figure that looks complete.
2. **Names stop at the fold.**
   - The series holds numbers and closed labels. A unit, process, pid or
     model name never reaches it.
   - #630's byte-grep test (`no_unit_name_ever_reaches_the_database_file`)
     moves with the store and covers every measurement.
   - A measurement whose value is a name is probe-only: it never reaches
     the series, the HUD or a dataset.
3. **Every external read is bounded and only observes.** A probe that can
   change what it observes is not a measurement. So:
   - `Fork` and `Loopback` reads have timeouts.
   - Anything that names a model sends `autoload=false`.
   - Nothing connects to a socket-activated port (§2).
   - Nothing writes anywhere but the series.
4. **Never in the system prompt** (`GOAL-SYSTEM-DESIGN.md` §4.3). Readings
   change every minute, and the cached prefix is sacred. A reading reaches
   a model only as a tool result (§5).
5. **Never live in eval or replay.**
   - The model tool is a `harness::Lever` member, so `mecha eval` forces it
     off.
   - Replay reuses the recorded tool result.
   - The homeostat's rule stands as written.
6. **A reading is not a metric.** No measurement feeds `candidate::Metric`,
   which stays closed at the type, exactly as `reading.rs` containment 1
   keeps the charter's readings out. Harness rumination auto-accepts config
   changes measured against metrics, and "the machine was less loaded" is
   not evidence that a config change was better.
7. **Measuring must not become load.**
   - The sampler runs once a minute, under `nice`, with `TimeoutStartSec`.
   - A `Fork` read costs about 80 ms (`GOAL-SYSTEM-DESIGN.md` §4.1), and a
     minute takes a handful of them.
   - Nothing is read per turn that the series can answer.
8. **Nothing here acts on a reading.** This layer reports.
   - Permits and holds own scheduling.
   - The gates that refuse, such as the image tool's memory floor, keep
     their own decisions.
   - When something should act on a reading, it is designed where the
     action lives, and it gets the reading through this layer.

## 5. The model tool (R3 and R3b, ruled)

`system_read` lets the assistant answer the owner's questions about the
machine. "Why is chat slow right now?" becomes a factual answer: the chat
model's slots are both busy, and image generation holds 14 GiB. The tool
exists to answer the owner. It is not a way for a run to reason about load
(§4, rule 8).

**Where it sits (R3):**
- It is built after the CLI and the HUD (S0–S3).
- It sits behind its own feature switch.
- `mecha eval` forces it off.
- It is not on persona tool lists. Personas have no use for the machine's
  state, and `PERSONA-DESIGN.md` §3 keeps their boundary narrow.

**Schema: closed values only.**
- `measurement`: a name from the list, or a group prefix.
- `window`: one of `now`, `1h`, `6h`, `24h`, `7d`.
- `by`: `none` or `category`.

It takes no SQL and no path. It returns numbers and closed labels with
capped rows. Its registry position is fixed, like every tool's, so it never
moves the cached prefix.

**Capabilities:**
- `untrusted_input: false`, and results are never `.from_outside()`. Our own
  sampler wrote them and the labels are a closed enum, so there is no
  third-party text for an injection to ride.
- `egress: None`. It has no destination.
- `destructive: false`.
- **`private_data: true` (R3b).** `LIVE-DASHBOARD-DESIGN.md` §11.4 names
  what categories do not hide: a category's series is a record of *when*
  that kind of work happened. The voice series is, in effect, a log of when
  calls took place. That is a pattern of life, and unknown is never clean.
  R14 already coarsens anything published to fifteen-minute buckets. If the
  owner later rules a coarsened series publishable, a coarsened read could
  be classed differently, but that would be a new ruling, not a default.

## 6. What consolidating fixes

These defects come from the duplication itself, and S1 and S2 close each
one by deleting a copy.

- **`nvidia-smi` with no timeout** in the host sampler goes through
  `recommend::nvidia_smi` instead, with its 10 s kill.
- **Two "is a voice call live?" answers** become one: `voice.live` reads
  the stamp file (`brief::VoicePresence`).
  - The stamp is the better primitive. The voice worker writes it, and it
    is checked against a live pid. The journal grep only infers liveness
    from a log line.
  - If the stamp misses a case the journal catches, the fix is to write the
    stamp in that case, not to keep a second reader.
  - The live-call build gate in the memory notes becomes
    `mecha system probe voice.live`.
- **Four `/proc/meminfo` parsers** become one. `imagegen` and `recommend`
  keep their MB units as a conversion at the call site, not as a parser.
- **`/slots`, `/props` and the `/health` waits** each get one client:
  `/slots` and `/props` live in `system`, and the `/health` wait becomes one
  helper shared by the install paths.

## 7. Phasing

| Step | What | Depends on |
|---|---|---|
| **S0** | Move `hud::host` into `system`. The store becomes `~/.mecha/system/series.sqlite`, `mecha hud sample` becomes `mecha system sample`, and the unit becomes `mecha-system-sample`. The `host` board's source and the doctor check follow. **Before the dashboard's step 2b deploys** (R2), so nothing migrates. | #630 merged |
| **S1** | The probe API, `Reading`, and the `File` and `Fork` tiers. The homeostat, imagegen and recommend switch to the shared parsers, and the four `/proc/meminfo` parsers become one. | S0 |
| **S2** | `model.*`, `runs.live`, `holds.live`, `permits.live`, `voice.live` and `image.loaded`. The brief reads them through `system`, and they are recorded in the series, except `model.resident`. | S1 |
| **S3** | The rest of the CLI (`read`, `probe`, prefixes), the doctor's unread-measurement finding, and the scripts moved onto `--json`. | S2 |
| **S4** | The homeostat reads the series (§3.1), so run records carry per-category conditions. | R4; S3 |
| **S5** | `system_read`, behind its switch and in `harness::Lever`. | S3 |

S0 is small, and it is the only step that is cheaper now than later.

## 8. Deliberately out of scope

- **Acting on readings.** No alerts, no automatic throttling, and no
  "defer because the machine is busy" inside a run. Permits and holds own
  contention (`TASK-AGENT-DESIGN.md` R1), and §4 rule 8 keeps this layer to
  reporting.
- **Setpoints on measurements.** `model.cache_evictions` is the obvious
  first candidate (`GOAL-SYSTEM-DESIGN.md` §4.5), once it is readable at
  all. A setpoint needs a home and a reader, and it gets designed when it
  has both.
- **Other machines.** The factory box and the workstation each have their
  own state. One series per machine, readable from another, is a later
  design. Any published version takes on the constraints of
  `LIVE-DASHBOARD-DESIGN.md` §6.
- **Export formats.** No Prometheus endpoint and no OpenTelemetry. The
  surfaces are the CLI's `--json` and the HUD.
- **Per-process detail, ever.** This is R13, applied to the whole layer.
- **The charter** (§1.3).

## 9. Rulings

- **R1. The name — ruled 2026-10-10:** `mecha system`, with a specific name
  for each measurement.
  - The module is `system`.
  - The closed list is `system::Measurement`.
  - The store is `~/.mecha/system/`.
  - The sampler unit is `mecha-system-sample`.
  - The tool is `system_read`.

  Nothing in this layer is called a sensor, which minimises the clash with
  the charter. The owner noted the naming may change later; if it does,
  the line to keep is §1.3's boundary between attributable and not.
- **R2. Timing — ruled 2026-10-10:** #630 merges as is. S0 then moves the
  sampler, and the dashboard's step 2b deploys once, at the final path.
- **R3. The model tool — ruled 2026-10-10:**
  - It comes after the CLI and the HUD.
  - It sits behind its own feature switch.
  - It is forced off in eval.
  - It is not on persona tool lists.
- **R3b. Private — ruled 2026-10-10:** `system_read` results carry
  `private_data: true`.
- **R4. The homeostat reads the series (S4) — open.** A run's record would
  carry the minutes it ran through, rather than a probe at its start.
  Recommended: yes.
