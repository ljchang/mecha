# Sensors — design

**2026-10-10. Proposed; the rulings in §9 are open.** One layer through
which mecha reads the state of the machine it runs on and the services it
runs beside. Every consumer (the homeostat, the HUD, the doctor, the
situation brief, the gates, and a model tool if one is ruled in) reads the
system through it, and none probes for itself.

This document is about *machine* state: memory, CPU, GPU, disk, the model
server, runs in flight. The goal system's charter also has sensors
(`charter::SensorKind`), and §1.3 says why those are a different thing that
stays where it is.

---

## 0. Why: the system is read nine ways today

An inventory of `origin/main` at `5669f2579` (plus the host sampler on
#630) found every reader below written for one consumer and re-deciding the
same questions:

| Source | Readers today |
|---|---|
| `/proc/meminfo` | **four parsers**: `homeostat::mem_available_kb`, `imagegen::mem_available_mb`, `recommend::parse_meminfo_total`, `hud::host::parse_meminfo` |
| `/proc/loadavg` | two: `homeostat::load_avg_1m`, `hud::host::parse_loadavg` |
| `nvidia-smi` | `recommend::nvidia_smi` runs it under a 10 s kill timeout. `hud::host::run` runs it with **none**, and on a wedged driver only its unit's `TimeoutStartSec` stops it. `model-idle.sh` and `comfyui-idle-reset` each run it themselves |
| llama-server `/slots` | `brief::slots_of`, and `scripts/model-idle.sh` restating its rule in shell |
| llama-server `/props` | `provider::preflight::fetch`, `router::is_router`, `scripts/served-props.sh` |
| "is a voice call live?" | **two answers that can disagree**: `brief::VoicePresence` reads a stamp file, and `replay_persona::voice_live` greps the TTS journal for `RTF` |
| `/health` waits | five separate implementations (`engine_gate`, `llama_units`, `router_unit`, `document`, `mecha-wait-healthy`) |
| unit files on disk | `feature::Facts::read` searches the config dir; `sidecar::Machinery::check` searches seven |

Each copy re-decides the questions that carry the risk. Is an unreadable
source a zero or unknown? Does the probe have a timeout? Can it load a
model or wake a server it was only meant to watch? The HUD sampler shows
what that costs. Three review passes on #630 each found the same mistake
one level further down: a value that could not be read was recorded as a
believable number.
1. An unreadable `/proc/meminfo` or cgroup tree read as an idle machine.
2. The core count came from the sampler's own parallelism rather than the
   machine's.
3. A missing `memory.current` read as zero bytes.

Each of the other readers has to get those same rules right on its own.

The goal design already asked for this. `GOAL-SYSTEM-DESIGN.md` §13.3
("one scan, three views") argues that the expensive part is the scan and the
readers should differ only in what they compute from it, and §4.1 tables the
sensors by cost. This document is that scan, for machine state.

## 1. What a sensor is

### 1.1 A closed name, a unit, a cost, and a reading that can say "unknown"

A sensor is an entry in a **closed enum** (`sensor::Id`), so its name is a
wire format exactly as `hud::host::Category` is: discriminants pinned by a
test, an unknown one read back as `None`, never as a neighbour. Each entry
declares:

- **unit**: bytes, percent, count, °C, watts, or a closed label set;
- **cost tier**: `File` (a `/proc` or `/sys` read), `Loopback` (one HTTP call
  to a local server, always with a timeout), `Fork` (a subprocess, always with
  a timeout), or `Scan` (a directory walk);
- **recordable**: whether its value may enter the series store (§1.2). A
  sensor whose value is a *name*, such as the model currently resident, is
  probe-only.

What a read returns keeps unknown apart from zero, which is the rule every
reader above states for itself:

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
about the machine; "the GPU did not answer" is an incident, and only the
second belongs in the doctor.

### 1.2 Two layers: a probe, and a series

- **The probe** reads now: `sensor::read(Id) -> Reading`. The parsers are
  pure functions of the text a source printed, kept separate from the I/O
  that fetched it. That is the split `hud::host` uses (`parse_meminfo`,
  `parse_gpu`, `collect`), and it is what lets every parser be tested
  against fixtures. **Each source has one parser.**
- **The series** is what the per-minute sampler records, in
  `~/.mecha/sensors/series.sqlite`. It holds numbers and closed labels only,
  with rollups and retention. It is #630's `host.sqlite`, generalised and
  moved (§7, S0). Readers of history (the HUD, `mecha sensor read --since`,
  the homeostat's view of a run's minutes) read the series. Readers of
  *now* read the probe, or the series' latest minute when it is fresh
  (§3.1).

### 1.3 Not the charter's sensors

`charter::SensorKind` is a closed set of observables that the owner sets a
setpoint against, and its rule is strict: **every kind is an observable a
store holds with an id per item that a run's own trace can touch**, because
attribution (`appraisal::of_session`) joins on that id and never on a
before-and-after delta (`GOAL-SYSTEM-DESIGN.md` §11.1, containment 6).
Machine readings have no per-item id. A run does not own "memory
available". So a machine sensor cannot be a charter sensor under the
charter's own rule, and this design does not try to make it one.

The shared word is the cost. The CLI is `mecha sensor`, as the owner
proposed, and the code module is `sensor`. The charter keeps
`charter::SensorKind` and `[line.sensor]`. The two never import each other,
and §9 R1 is the ruling on whether that naming is acceptable.

## 2. The catalogue

What exists to read on this class of machine, by tier. "Today" says who
reads the source now; the series column is whether the sampler records it.

| Sensor | Source | Unit | Tier | Series | Today |
|---|---|---|---|---|---|
| `mem.total`, `mem.available`, `swap.used` | `/proc/meminfo` | bytes | File | yes | homeostat, imagegen, recommend, hud |
| `cpu.busy`, `cpu.cores` | `/proc/stat` (cores from the `cpuN` lines, never `available_parallelism`) | %, count | File | yes | hud |
| `load.1m`, `tasks` | `/proc/loadavg` | load, count | File | yes | homeostat, hud |
| `disk.root` | `statvfs("/")` | bytes used, total | File | yes | hud |
| `by_category.{mem,cpu,tasks,gpu_mem}` | user cgroups + `nvidia-smi --query-compute-apps`, folded into `Category` | bytes, %, count | Fork | yes | hud |
| `gpu.util`, `gpu.temp`, `gpu.power` | `nvidia-smi --query-gpu` | %, °C, W | Fork | yes | hud, model-idle.sh |
| `gpu.unified` | `nvidia-smi` memory total `[N/A]` on every GPU | bool | Fork | yes (machine state, sticky) | hud, recommend |
| `model.slots` | `GET /slots?autoload=false` on :8080, local provider only | busy, total | Loopback | yes | brief, `mecha model use`, model-idle.sh |
| `model.resident` | `GET /models` on the router | a model id | Loopback | **no: a name** | router, `mecha model list` |
| `runs.live`, `holds.live`, `permits.live` | `runmarker`, `hold`, `permit` dirs + pid checks | count | Scan | yes | brief, follow, `mecha model` |
| `voice.live` | **one answer**, chosen in S2 (§6) | bool | File | yes | brief, replay_persona (disagreeing) |
| `image.loaded` | ComfyUI `GET /system_stats` | bool | Loopback | yes | imagegen |
| `services.failed` | `systemctl --user --failed` | count | Fork | yes | CLI doctor |
| `backlog.*` | `backlog::Backlog::survey`, per kind | count | Scan | yes | homeostat, charter readings |

**Not readable today:** prompt-cache evictions, which `GOAL-SYSTEM-DESIGN.md`
§4.5 names as the one sensor to add. Llama-server reports them on
`/metrics`, and on 2026-10-10 the router answered `GET /metrics` with
HTTP 400, with no `--metrics` flag in any script. Adding the sensor means
adding the flag in `scripts/start-router.sh` first and confirming the
counter's name from a live response, not from memory.

**Never probed: the on-demand servers.** Embeddings (:8081) and OCR (:8085)
sit behind systemd sockets (`LLAMA-SERVER.md` §Document OCR), so any
connection to the port **starts the model**. A sensor that asked whether
they were up would load them every minute. Their sensor reads the unit's
active state from systemd instead, which observes without waking anything.
The same rule covers every `Loopback` read of the router: `autoload=false`
on anything that names a model.

## 3. Who reads what

### 3.1 The homeostat

Today `Homeostat::at_start` reads the 1-minute load and `MemAvailable`
itself. It does not read `/slots`, because that is an HTTP call and the
homeostat is sampled wherever a front-end records a session.

With a series, it can read the **latest minute** instead of probing, when
that minute is under two minutes old. That costs one indexed SQLite read
and brings in what the homeostat could not afford to probe: slot
occupancy, per-category memory, and GPU load. Falling back to the `File`
tier when the series is stale keeps it working without the sampler.

A run's record then says what the machine was doing while the run ran, not
just in the instant it started. A run's start and end times select its
minutes from the series, which is what the appraiser needs to tell regret
from disappointment (`homeostat.rs` module docs).

**Unchanged:** only front-ends attach the homeostat. `mecha eval`, batch and
the replay probes never read live state, and anything reconstructing a run
reads the recorded snapshot.

### 3.2 The HUD

The HUD has one change, and it is a path: the `host` board's source becomes
`kind = "sqlite"` over `~/.mecha/sensors/series.sqlite`, and its loaders
keep the SQL they have. The privacy property that R13 in
`LIVE-DASHBOARD-DESIGN.md` §0 demands now belongs to the series as a whole
(§4, rule 2), not to one board.

### 3.3 The doctor

- **The sampler has stopped:** the newest minute is more than ten minutes
  old. This is #630's `check_host_sampler`, moved.
- **A sensor stays unread:** a sensor that is `Unread` in every minute of the
  last hour, while it was `Observed` earlier, is a finding, and the finding
  names the sensor and the last `why`. A sensor that is `NotHere` never
  produces one.

### 3.4 The brief, the gates and the scripts

- **The brief:** `brief::read_slots`, `seats_under`, `runs_under` and
  `VoicePresence` become calls into `sensor`.
- **The gates:** the image tool's memory gate reads `mem.available` and
  `image.loaded` through the same probe.
- **The scripts:** `model-idle.sh` and `comfyui-idle-reset` call
  `mecha sensor read <id> --json` and stop parsing `/slots` and `nvidia-smi`
  themselves.

The brief's own rule is unchanged: it reads before assembly, so
`brief::assemble` does no network.

### 3.5 The CLI

| Command | What it does |
|---|---|
| `mecha sensor` | Every sensor, its latest reading and state (`Observed` / `Unread: why` / `NotHere`), and its age. |
| `mecha sensor read <id> [--since 6h] [--step 5m] [--by category] [--json]` | One sensor sliced from the series. `--by` groups by a closed dimension, which in this design is only `category`. |
| `mecha sensor probe <id> [--json]` | Reads now, bypassing the series. This is the diagnostic, and it works with the sampler stopped. |
| `mecha sensor sample` | The per-minute writer (#630's `mecha hud sample`, moved), run by `mecha-sensor-sample.timer`. |

### 3.6 The model, if R3 rules it in (§5)

## 4. Rules

1. **Unknown is never zero.** Every read returns a `Reading`, so the
   distinction lives in the type. The sampler refuses a sample when a
   denominator cannot be read: memory total, the cgroup tree, the core
   count. A minute it cannot fully attribute is recorded with NULLs, never
   with a figure that looks complete.
2. **Names stop at the fold.** The series holds numbers and closed labels.
   A unit, process, pid or model name never reaches it, and #630's
   byte-grep test (`no_unit_name_ever_reaches_the_database_file`) moves
   with the store and covers every sensor. A sensor whose value is a name
   is probe-only, and its value never reaches the series, the HUD or a
   dataset.
3. **Every external read is bounded and is a pure observer.**
   - `Fork` and `Loopback` reads have timeouts.
   - Nothing loads a model: `autoload=false` on any probe that names one.
   - Nothing connects to a socket-activated port (§2).
   - Nothing writes anywhere but the series.
   A probe that can change what it observes is not a sensor.
4. **Never in the system prompt** (`GOAL-SYSTEM-DESIGN.md` §4.3). Readings
   change by the minute, and the cached prefix is sacred. A reading reaches
   a model only as a tool result (§5).
5. **Never live in eval or replay.** The model tool is a member of
   `harness::Lever`, so `mecha eval` forces it off. Replay reuses the
   recorded tool result. The homeostat's rule stands as written.
6. **A reading is not a metric.** No sensor feeds `candidate::Metric`, which
   stays closed at the type, exactly as `reading.rs` containment 1 keeps
   the charter's readings out. Harness rumination auto-accepts config
   changes measured against metrics, and "the machine was less loaded"
   is not evidence that a config change was better.
7. **Sensing must not become load.** The sampler runs once a minute, under
   `nice`, with `TimeoutStartSec`. A `Fork`-tier read costs about 80 ms
   (`GOAL-SYSTEM-DESIGN.md` §4.1), and a minute takes a handful of them.
   Nothing is sampled per turn that the series can answer.
8. **Nothing acts on a reading here.** Permits and holds own scheduling,
   and the gates that refuse (the image tool's memory floor) keep their own
   decisions. This layer reports state. When something should act on a
   reading, it is designed where the action lives, and it reads the state
   through this layer.

## 5. The model tool (R3)

If ruled in, `sensor_read` lets the assistant answer the owner's questions
about the machine. "Why is chat slow right now?" becomes a factual answer:
the chat model's slots are both busy, and image generation is holding
14 GiB. The tool is for answering the owner. It is not a way for a run to
reason about load (§4, rule 8).

**Schema: closed values only.**
- `sensor`: an `Id` from the registry.
- `window`: `now`, `1h`, `6h`, `24h` or `7d`.
- `by`: `none` or `category`.

It takes no SQL and no paths. It returns numbers and closed labels, and its
rows are capped.

**Capabilities.**
- `untrusted_input: false`, and its results are never `.from_outside()`:
  our own sampler wrote them, and the labels are a closed enum, so there is
  no third-party text for an injection to ride in.
- `egress: None`: it has no destination.
- `destructive: false`.
- `private_data` is the ruling (§9 R3b). `LIVE-DASHBOARD-DESIGN.md` §11.4
  already names what categories do not hide: a category's series is a
  record of *when* that kind of work happened, and the voice series is, in
  effect, a log of when calls took place. That is a pattern of life, so
  unknown is never clean and the default is `true`. R14 keeps the host board
  private for now, and coarsens anything published to fifteen-minute
  buckets. A publishable series at that grain could justify `false` for a
  coarsened read, but that is a ruling, not a default.

**Where it appears.** The tool is on the assistant's tool list, behind its
own feature switch. It is not on persona tool lists, because a persona has
no use for the machine's state (`PERSONA-DESIGN.md` §3 keeps the persona
boundary narrow). Its registry position is fixed like every tool's, so it
does not move the cached prefix.

## 6. What consolidating fixes on the way

These defects come from the duplication itself, and S1 and S2 close each one
by deleting a copy:

- **`nvidia-smi` without a timeout** in the host sampler goes through
  `recommend::nvidia_smi`'s 10 s kill.
- **Two "is a voice call live?" answers** become one. The stamp file
  (`brief::VoicePresence`) is the better primitive: it is written by the
  voice worker and checked against a live pid. The journal grep infers
  liveness from a log line. The sensor reads the stamp. If the stamp
  misses a case the journal catches, the fix is to write the stamp in that
  case, not to keep a second reader. The live-call build gate in the
  memory notes becomes `mecha sensor probe voice.live`.
- **Four `/proc/meminfo` parsers** become one. `imagegen` and `recommend`
  keep their units (MB) as conversions at the call site, not as parsers.
- **`/slots`, `/props` and the `/health` waits** each get one client:
  `/slots` and `/props` in `sensor`, the `/health` wait in one helper that
  the install paths share.

## 7. Phasing

| Step | What | Depends on |
|---|---|---|
| **S0** | Move `hud::host` to `sensor`. The store becomes `~/.mecha/sensors/series.sqlite`, `mecha hud sample` becomes `mecha sensor sample`, and the unit becomes `mecha-sensor-sample`. The `host` board's source and the doctor check follow the move. **Before step 2b of the dashboard is deployed**, so that nothing migrates. | #630 merged |
| **S1** | The probe API, `Reading`, and the `File` and `Fork` tiers. The homeostat, imagegen and recommend switch to the shared parsers, and the four `/proc/meminfo` parsers become one. | S0 |
| **S2** | The model server, run and voice sensors (`model.slots`, `model.resident`, which is probe-only, `runs`/`holds`/`permits`, `voice.live`, `image.loaded`). The brief reads them through `sensor`, and they are recorded in the series. | S1 |
| **S3** | The rest of `mecha sensor` (`read`, `probe`, listing), the doctor's unread-sensor finding, and the scripts moved onto `--json`. | S2 |
| **S4** | The homeostat reads the series' latest minute (§3.1), and run records carry per-category conditions. | S3 |
| **S5** | `sensor_read`, behind its switch and in `harness::Lever`. | R3; S3 |

S0 is small, and it is the only step that is cheaper now than later.

## 8. Deliberately out of scope

- **Acting on readings.** That means no alerts, no automatic throttling,
  and no "defer because the machine is busy" inside a run. Permits and
  holds own contention (`TASK-AGENT-DESIGN.md` R1), and §4 rule 8 keeps
  this layer to reporting.
- **Setpoints on machine sensors.** Cache evictions are the obvious first
  candidate (`GOAL-SYSTEM-DESIGN.md` §4.5), once they are readable at all.
  A setpoint needs a place to live and a reader. It is designed then, and
  not before.
- **Other machines.** The factory box and the workstation each have their
  own state. One series per machine, readable from another, is a later
  design. The factory constraints in `LIVE-DASHBOARD-DESIGN.md` §6 apply to
  any version that is published.
- **Export formats.** There is no Prometheus endpoint and no
  OpenTelemetry. The CLI's `--json` and the HUD are the surfaces.
- **Per-process detail, ever.** This is R13's rule, applied to the whole
  layer rather than to one board.
- **The charter.** §1.3 explains why.

## 9. Rulings this waits on

- **R1. The name.** The CLI is `mecha sensor` and the module is `sensor`,
  with the charter's `SensorKind` left as it is and the difference
  documented (§1.3). The alternative is a word that does not collide, such
  as `vitals`. Recommended: `sensor`, as proposed.
- **R2. Timing.** Do S0 before deploying the dashboard's step 2b.
  Recommended: yes. #630 merges as it is, and S0 is the first commit of
  this arc.
- **R3. The model tool**, and **R3b**, whether its results carry
  `private_data`. Recommended: build S0 to S3 first, then add the tool
  behind its own switch with `private_data: true` until the owner rules
  the series publishable.
- **R4. The homeostat reads the series** (S4), so a run's record carries
  the minutes it ran in, rather than a probe at start. Recommended: yes.
