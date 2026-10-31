# Apple's `container` as a macOS confinement for the monty worker

**Research pass, 2026-10-30.** No code or config changed.

**The question.** Is Apple's Containerization framework, with its
`container` CLI, a better macOS backend than Seatbelt for confining code
mode's monty worker, or a second one? `container` runs each container in
its own lightweight Linux VM through Virtualization.framework.

**Why it is asked.** On Linux the worker runs under bwrap with no network,
a cleared environment and only its own static binary. `systemd-run` adds
cgroup memory and CPU limits (`ARCHITECTURE.md` §Code mode's interpreter,
`interpreter::monty::Confinement`). A Seatbelt backend for macOS is being
built in parallel. Its known gap is that macOS has no cgroups, so the
sandbox cannot cap a worker's memory. Seatbelt's design for documents is
`DOCUMENT-EXTRACTION-DESIGN.md` §3.1. The brief cited
`SANDBOX-RESEARCH.md` §3.1 for it, but that file has no such section.

**How it was researched:**
- Apple's own repositories, read in source at a named commit:
  - `apple/container` at tag 1.6.1 (`aa8d76b4`);
  - `apple/containerization` at `main` (`fd4a84a9`, 2026-10-30). The CLI
    pins containerization 0.53.0, so a framework detail read on `main` is
    marked as such.
- The GitHub releases, advisories and issues APIs.
- `monty-pool` 1.4.0 from the cargo registry, and `pydantic/monty` at
  `main` (`66fff6bf`) for the worker's subcommands.
- Apple's developer documentation JSON and the Developer Forums.
- GitHub's runner documentation.
- A peer lane's macOS CI probe (run 41145654354 on branch
  `scratch/seatbelt-monty-probe`, 2026-10-30).
- Published third-party benchmarks.

Nothing ran on a Mac from this lane. §8 lists what must.

**Marks.** Each claim carries one:
- **[V]** Verified here: read in source at the named commit, read from an
  API, or read from a CI log.
- **[M3]** Measured by a third party and published. The method is stated
  where the source states it. Not reproduced here.
- **[C]** A vendor claim: Apple's own words, unmeasured.
- **[I]** Inferred. Reasoning, not evidence. §8 lists the measurement that
  would settle it.

---

## 0. The answer in one paragraph

**`container` is a second backend, not a better default.** It is the only
route on a Mac to an *external* memory ceiling for the worker, because a
VM's RAM is fixed at boot. Its escape boundary is a hypervisor rather than
a sandbox profile on mecha's own kernel. It is now 1.x, Apache-2.0, and
shipped by Apple. Against that:
- **It needs Apple silicon and macOS 26 or later**, and an admin password
  to install.
- **It needs a launchd daemon** started from a login session.
- **It costs about 0.7 s per cold VM** [M3], against bwrap's 4 ms [V]
  (`CODE-MODE-DESIGN.md` §10, M1).
- **GitHub-hosted runners cannot run it** [V], so no PR could gate on it.
- **Its kill semantics break the pool's contract the way docker's did**
  [I]. The pool kills the pid it spawned, which is the CLI client, not the
  VM.
- **There is no warm VM to hand a worker.** A VM's CPUs, memory and init
  filesystem are fixed at boot, so a booted VM cannot be pooled and given a
  workload later (maintainer, #738) [C]. Every new worker pays a boot. If
  Containerization ever ships VM snapshots, this comparison moves (§7,
  "When to revisit").

So the recommendation is the third option in the brief:
- **Seatbelt is the default.**
- **`container` is an opt-in "strong" backend** on Apple silicon and
  macOS 26+, one VM per worker.
- **It is built only after §8's measurements**, M-C9 (what a SIGKILLed
  client leaves running) before any other.
- **The memory gap does not wait on it.** Monty's own heap limit is the
  first wall on every platform today, and a Seatbelt worker should say
  plainly that it has no second one.

---

## 1. Current state

| | Finding | Mark |
|---|---|---|
| Latest release | **1.6.1, 2026-10-23.** 1.6.0 was 2026-10-20 and 1.5.0 2026-09-29 | [V] releases API |
| First 1.0 | **1.0.0, 2026-06-09**, a year after 0.1.0 (2025-06-09). 0.12.3 (2026-04-30) was the last 0.x | [V] |
| Versioning | "Its release versions are product versions, not semantic versions." The CLI "generally" keeps compatibility within a major release, but "breaking changes may occur occasionally". `cluster` is experimental | [V] README at 1.6.1 |
| Breaking changes since 1.0 | ⌨️ CLI breaks in 1.3.0 (`--scheme auto` removed) and 1.5.0 (`k8s start` removed). 👩‍💻 API breaks in 1.4.1 (`system status` shape; JSON no longer escapes `/`). 1.0.0 itself reshaped the `ls`/`inspect` JSON of containers, images, networks and volumes | [V] release notes |
| License | Apache-2.0, both `container` and `containerization` | [V] `LICENSE`, repo API |
| Hardware | "You need a Mac with Apple silicon." Homebrew's formula requires `arch: arm64` | [V] README; formulae.brew.sh |
| macOS | "Supported on macOS 26 … We do not support older versions." `Package.swift` still builds for macOS 15, with documented networking limits there. Homebrew requires macOS 26 and ships bottles for `arm64_tahoe` and `arm64_golden_gate`. Apple's CI moved to macOS 27 runners on 2026-10-28 (`apple/container` #2436) | [V] |
| Install | A signed `.pkg` from the release page, installed under `/usr/local` with an **administrator password**, then `container system start`. Alternatively `brew install container` (1.6.1) | [V] README; brew API |
| First start | `container system start` registers `container-apiserver` as a launch agent and prompts to download a kernel: Kata 3.32.0's `vmlinux-6.18.35-197-debug`, by URL and sha256 digest in `config.toml`. It also pulls the `vminit` image (`ghcr.io/apple/containerization/vminit:0.34.0`) | [V] `docs/container-system-config.md` |

### Could mecha pin and provision it like other sidecars?

**Partly. mecha could pin it, but not install it.**

- **It is a provided sidecar, never an installed one.** The pkg writes to
  `/usr/local` as root. mecha never runs `sudo`, and `sidecar.rs` (F8) treats
  a hand install as provided and never installs over it. A user who runs
  `brew install container` has done the provisioning [I].
- **Pin the installed binaries by sha256, per release.** This is what
  `interpreter::worker` does for monty. A reviewed constant names the
  `container` CLI's hash and the `container-runtime-linux` helper's hash
  for one version, and every pool start re-hashes them [I].
  `container system version --format json` gives the version [V], but a
  version string never stands in for a build (CLAUDE.md). The binaries'
  paths under `/usr/local` are owed to §8 (M-C14).
- **The kernel, init image and worker image can all be mecha's own.**
  - `container run -k <path>` takes a kernel file [V], so mecha can fetch
    the Kata archive through `fetch.rs` by its pinned sha256. It need not
    rely on the owner's `config.toml`.
  - `--init-image` takes a vminit image [V].
  - The worker image needs no registry. mecha can write a one-layer OCI
    archive holding the pinned static aarch64-linux worker, the one
    already in `worker::PINS` (`989eb5ba…`), and `container image load`
    it [I].

  Loading beats pulling for a second reason. Before 1.6.0, a pull by
  digest did not check that the registry returned that digest
  (GHSA-vgrw-36j7-r93w, fixed in 1.6.0 and containerization 0.52.1) [V].
- **There is no unprivileged install route that is safe to rely on.**
  `container system start` takes `--install-root` and `--app-root` [V]. That
  suggests an expanded pkg could run from `~/.mecha/sidecars/`. But the
  launchd labels default to the prefix `com.apple.container.`, and a second
  copy would collide with an owner's own install. Nothing here shows
  whether the start can take a different prefix (`system stop` and
  `status` take `--prefix`; `start`'s documented flags do not) [V/I].
  **Not recommended.**

---

## 2. Start-up latency and memory per VM

### Start-up

| What | Figure | Mark |
|---|---|---|
| Apple's claim | "Sub-second start times" (WWDC25 *Meet Containerization*, repeated at WWDC26 *Discover container machines*). Boot times "comparable to containers running in a shared VM" (`docs/technical-overview.md`) | [C] |
| `container run --rm alpine echo`, cold microVM | **~700 ms.** `create` is ~40 ms and `start` ~650 ms. Medians of 5, image pre-pulled, container 1.0.0, Mac16,12, macOS 26 (dcon wiki) | [M3] |
| Four concurrent `run -d` | **~11% faster than four sequential runs**: "Apple's apiserver serializes VM boots" (dcon wiki) | [M3] |
| `container exec` into a live VM | **~60–90 ms**, of which the Swift CLI's own start is ~70 ms (dcon wiki) | [M3] |
| `container run -it --rm hello-world` | 0.80 s against OrbStack's 0.20 s (`apple/container` #738, 2025-10, version not stated). A maintainer's answer: a dedicated VM starts slower than a namespace in a running VM | [M3] |
| `container run ubuntu echo foo` | 0.93 s with the unsigned build and 0.70 s signed, on an M3 Pro (#58, 0.1.0) | [M3] |
| Ranges with no method | "~1.0–3.0 s" first, "~0.5–1.5 s" after (a DEV Community post, v1.3.1, hardware unstated) | [M3], weak |
| **For comparison: the Linux path today** | Cold program 3.5–4.0 ms (spawn, bwrap, `Configure`, feed `1`). Warm 0.05–0.09 ms | [V] `CODE-MODE-DESIGN.md` §10 M1 |

**Warm is not a VM state.** VMs cannot be pre-booted and handed a
workload. "CPU, network, and memory resources, init filesystem, etc all
need to be determined at boot time" (maintainer, #738) [C]. So "warm" means
reusing a booted VM, either through `exec` (§3, option B) or through the
pool's own worker reuse, which holds within one VM.

### Memory

| What | Finding | Mark |
|---|---|---|
| VM size | The container's `--memory` **plus 128 MiB** (`VMResources.guestMemoryOverhead`). vCPUs are `--cpus` plus `cpuOverhead` (1) | [V] `RuntimeService.swift`, `ContainerConfiguration.swift` at 1.6.1 |
| Defaults | 4 CPUs and 1 GiB per container | [V] `config.toml` reference |
| Host footprint | "The underlying virtual machine only uses the amount of memory that the containerized application needs" | [C] technical overview |
| Freed memory | "Memory pages freed to the Linux operating system by processes running in the container's VM are **not relinquished to the host**." Ballooning is only partial | [C] technical overview, as a known limitation |
| Per-VM overhead, measured | "~100–200 MB per container", no method given. No source found measured a minimal VM's host footprint | [M3], weak |
| Host processes per VM | One `container-runtime-linux` helper and one `com.apple.Virtualization.VirtualMachine` service process. There is no stable interface for the VM's host-side memory (#1921, open) | [V] technical overview; #1921 |

**What this costs code mode** [I]:
- A worker that once reached its 256 MiB heap limit (`Backstops::max_memory`'s
  default) keeps those pages charged to the host until its VM stops.
- The pool already recycles a worker every 100 checkouts
  (`max_checkouts_per_worker`). With one VM per worker, that recycle is
  also what returns the memory.
- At `max_workers` concurrent workers, the reservation is
  `max_workers × (memory + 128 MiB)` at worst.

---

## 3. How it fits `monty-pool`'s model

### The launcher contract, read in `monty-pool` 1.4.0

| | Contract | Mark |
|---|---|---|
| Spawn | `Command::new(binary).arg("subprocess").env_clear()`, stdin and stdout piped, stderr inherited, `kill_on_drop(true)` | [V] `worker.rs`, `Worker::subprocess` |
| Kill | A discarded worker is `start_kill()`ed, which is SIGKILL, and reaped by its pid | [V] `kill_and_reap` |
| No handshake | "A wrong or broken binary surfaces as an error on the first request" | [V] |
| What mecha passes | One executable, a 0700 script that `exec`s the confinement (`interpreter::monty::Launcher`). The pool appends `subprocess`. `min_processes = 0`, `max_checkouts_per_worker = 100`, `checkout_timeout` 60 s, and a 30 s `preflight` | [V] `monty.rs` |
| The other transport | `MontyTransport::Websocket` dials a `ws://`/`wss://` URL per checkout. Its workers are single-use, never pooled. The worker binary has no *server* mode: it serves the pool over stdio (`subprocess`), and also runs one-off programs (`monty -c …`, which §8 uses), but cannot listen. So a WebSocket child needs a relay | [V] `lib.rs`; `monty-runtime/src/main.rs` |

**The constraint that decides everything.** Whatever the launcher script
`exec`s *is* the worker, as far as the pool knows. The pool kills that pid
and nothing else. This is why docker was refused
(`Confinement::for_sandbox`): the pool kills `docker run`'s client, and
the container keeps running. `container run` has the same shape [I]:
- The CLI forwards SIGINT and SIGTERM to the container process
  (`ProcessIO.handleProcess`) [V].
- SIGKILL cannot be forwarded.
- The stdio handles are passed to the apiserver in `bootstrap(id:stdio:)`
  [V], so the VM's pipes plausibly outlive the client [I].
- A worker blocked reading stdin would then see EOF and exit. A worker
  spinning in a loop, which is exactly the one the pool is killing, would
  not.

**M-C9 measures this, and it gates every option below.**

### Option A: one VM per worker

The launcher script is
`exec container run --rm -i --network none --no-dns --read-only --cap-drop ALL --user 65534:65534 --memory <m> --cpus 1 --cidfile <launcher-dir>/$$.cid <pinned image> "$@"`.
The image's `ENTRYPOINT` is `["/monty"]`, as in §8's setup, so the
arguments the pool appends (`subprocess`) reach the worker as its own.
Naming `/monty` here as well would hand the worker `/monty subprocess`.
The `$$` in the cidfile path is a problem of its own (the Kill bullet
below).

- **Start cost.** About 0.7 s per worker [M3], serialized across
  concurrent boots [M3]. This is paid on the first program of each worker,
  then amortized over up to 100 programs.
  - **Per cold worker:** ~0.7 s against bwrap's ~4 ms, about 175× [M3]/[V].
  - **Per program at full reuse:** ~7 ms of VM start, plus a warm program,
    against bwrap's ~0.04 ms of start plus a warm program at 0.05–0.09 ms
    (`CODE-MODE-DESIGN.md` §10, M1). That is about 7 ms against about
    0.1 ms [I]. The ratio stays large; amortizing only makes the absolute
    cost small.
  - **The first `code` call in a process pays it bare**, and so does
    `preflight`. Both fit inside the 30 s and 60 s timeouts.
  - A cold stall of under a second is visible in a chat, not fatal.
- **Isolation is the best of the three.** Each worker has its own kernel
  and memory ceiling, nothing shared between workers, and no virtiofs share
  at all, because the binary is in the image.
- **Kill.** If M-C9 shows the VM orphaned, closing it needs a reaper.
  - Each launcher writes its container ID to a file named by its pid
    (`--cidfile`; `exec` keeps the pid). The pid is the one the pool will
    kill.
  - **Today's launcher writer cannot express that.**
    `interpreter::monty::Launcher::write` shell-quotes every argv element
    (`sh_quote`), so a `$$` arrives literally and every worker would write
    the same file [V]. The reaper therefore needs a new, deliberate
    affordance in the generated script: one fragment mecha writes
    unquoted, built from no input but the launcher directory. That is a
    change to the one piece of code that turns the confinement into an
    executable, and it raises option A's cost.
  - The backend sweeps its launcher directory after a discarded checkout,
    and on `Drop`. For every cidfile whose pid is gone it runs
    `container kill` and then `container delete`.
  - `container list --all --format json` with a `--label` finds whatever a
    crash left behind.
  - That is new code with its own failure mode: a reaper that cannot run
    must stop the pool, not leak VMs.
- **Memory returns per recycle** (§2).

### Option B: one long-lived VM hosting the pool's workers

mecha boots one VM per pool (`container run -d`). The launcher is
`exec container exec -i <vm> /monty "$@"`.

- **Start cost.** ~60–90 ms per worker [M3] after one VM boot per pool.
- **Isolation between workers is gone.**
  - **They share one kernel and one cgroup.** `exec` lands the process in
    the container's cgroup [I], so one worker's growth can get a sibling
    OOM-killed. A per-worker cap would need cgroups managed inside the
    guest.
  - **They share one uid and pid namespace**, so a worker that escaped
    the interpreter can signal or trace its siblings [I]. That is
    cross-program interference, which Linux's per-worker bwrap rules out.
- **Kill.** It has the same SIGKILL problem through `exec` [I] (M-C9b).
  It is bounded only by the VM's CPU and memory ceilings, and cleared when
  mecha stops the VM.
- **A known leak.** `container exec -i` leaks one host pipe descriptor in
  the runtime helper per completed invocation (#2439, open, 1.6.1) [M3].
  The VM would have to be recycled before the helper runs out.
- **The VM needs a resident process**, so the image needs something to
  idle on: a second pinned binary, or an `--init` that waits [I].

### Option C: the pool itself inside the VM

The VM runs a mecha-built static Linux "pool host" linking `monty-pool`,
and its workers are local processes. Inside the guest, the Linux backend
works as it does today: namespaces, and cgroups written directly, since
the guest has no systemd [I]. The kill problem disappears, because the
pool and its worker share a kernel.

The cost is a **new wire protocol** across the VM boundary:
- every checkout, feed and `TurnEvent`;
- every host call answered by mecha's gate (`CODE-MODE-DESIGN.md` §7);
- Stop and `cancel_grace`, which must cross it promptly;
- and a second mecha binary to build, pin and test.

That is a project, not a backend.

`monty-pool`'s own remote transport does not shortcut it. It dials
TCP WebSocket URLs and wants a relay. `--network none` leaves no TCP
route, and `--publish-socket` relays a Unix socket, not a `ws://` URL [V].

### Option D: no CLI at all

A small Swift helper links Containerization directly, as the framework's
own `cctl` example does. It owns its VM, with no apiserver, no launchd
daemon and no CLI JSON to parse. Its death plausibly takes the VM with it,
because the VM is its child XPC service [I].

The costs:
- **The helper must carry the `com.apple.security.virtualization`
  entitlement**, as `container-runtime-linux` does [V]
  (`signing/*.entitlements`). Whether an ad-hoc signature suffices for
  that entitlement is unverified here [I].
- **mecha's release pipeline would build Swift with Xcode for the first
  time.**

**Worth keeping in view if A's reaper proves ugly. Not first.**

| | Start per worker | Worker isolation | Memory cap per worker | Kill contract | New code |
|---|---|---|---|---|---|
| A, VM per worker | ~0.7 s [M3] | full | yes | needs a reaper unless M-C9 says otherwise | launcher, reaper, image builder |
| B, shared VM + `exec` | ~60–90 ms [M3] | none between workers | no, shared | needs a reaper; plus #2439 | as A, plus VM lifecycle |
| C, pool in VM | one VM per pool | Linux-grade | yes, in-guest cgroups | native | a protocol and a second binary |
| D, Swift helper | ~0.7 s [I] | full | yes | plausibly native [I] | a Swift build |

---

## 4. What it isolates, against Seatbelt and Linux

| Axis | Linux: bwrap + `systemd-run` | macOS: Seatbelt | macOS: `container` (option A) |
|---|---|---|---|
| **Network** | Every namespace unshared, nothing bound [V] (`Confinement::argv`) | `(deny network*)`, as designed for poppler's per-call profile (`DOCUMENT-EXTRACTION-DESIGN.md` §3.1) [I for the worker] | `--network none` creates **no interface at all** (`Utility.containerConfigFromFlags` sets `config.networks = []`; another network beside `none` is refused) [V]. The VM keeps vsock to its own runtime helper, which carries stdio and the agent [V] (`vminitd` `StandardIO`). Whether a guest process can open `AF_VSOCK` is M-C7: the framework's seccomp profile denies it, but no seccomp is on by default (next row) [V] |
| **Filesystem** | Only the static binary, read-only at `/monty`. The root is empty and read-only [V] | Deny-default with explicit reads (the libraries a Mach-O names), and the home denied, as designed for poppler (§3.1 there) [I for the worker] | The guest sees its image and only the shares named with `-v`/`--mount`, each a virtiofs mount. Option A names none [V] (technical overview, "Privacy"). `--read-only` exists [V]; whether a scratch image boots with it is M-C4, since #1737 reports bootstrap failing on images without a writable `/etc` |
| **Memory** | cgroup `memory.max` (`[sandbox] memory_mb`); a kill reads `End::Backstop` [V] | **None.** `ulimit -v`, `-d` and `-m` of 1 GiB all fail to set on macOS 26.6.2, and a 3 GiB string then allocates (RSS 3.23 GB) [V] (CI run 41145654354). Monty's own allocator limit is the only bound | **The VM's RAM is a hard ceiling** (`--memory` + 128 MiB), with a cgroup limit of `--memory` inside the guest [V] (`LinuxContainer.Configuration.memoryInBytes`, `RuntimeService`). Enforcement by the guest's OOM killer is M-C5 |
| **CPU** | cgroup `cpu.max` (`[sandbox] cpus`) [V] | `RLIMIT_CPU` and the wall clock, as designed for poppler (§3.1 there) [I for the worker] | `--cpus` vCPUs (+1 overhead) and a guest cgroup limit [V] |
| **Environment** | `monty-pool` `env_clear()` plus bwrap `--clearenv` [V] | `env_clear()` [V] | `env_clear()` reaches the CLI client, not the guest. The guest gets the image's `Config.Env` plus any `-e`. An image built by mecha has none [I]. Two traps in the CLI [V]: before 1.2.0, a bare-name `Config.Env` entry copied the *host's* value in (GHSA-xwgf-4rc5-p4m4); and `run` forwards `SSH_AUTH_SOCK` from its own environment (`ContainerRun.swift`), which the cleared environment happens to close. Whether the CLI runs at all under an empty environment is M-C8 |
| **Seccomp, capabilities** | `--unshare-user --disable-userns` [V] | n/a | **No seccomp filter by default.** Containerization's default is `.unconfined`, and its containerd-derived profile needs an experimental OCI runtime path [V] (`main`). `--cap-drop ALL` and `--user` exist [V] |
| **What an escape requires** | An interpreter escape, then a Linux kernel bug reachable from an empty namespace | An interpreter escape, then **an XNU or system-service bug reachable through the profile, on the kernel mecha itself runs on**. SBPL is an undocumented, unsupported language (§6), so the profile's reach is whatever this macOS release makes of it | An interpreter escape, then container-to-guest-root (a low bar: no seccomp, a debug kernel by default [V]), then **a Virtualization.framework device or host-helper bug**, reachable only through virtio and vsock [I] |

**The Seatbelt column describes a design, not a worker profile.** No
Seatbelt profile for the worker exists yet: `Confinement::for_sandbox`
refuses `Backend::Seatbelt` with `SEATBELT_IS_DOCUMENTS_ONLY` [V]. Its
network, filesystem and CPU rows are what `DOCUMENT-EXTRACTION-DESIGN.md`
§3.1 built for poppler, carried over by inference. The peer probe's
deny-default profiles all aborted the macOS worker (exit 134) before it
printed anything (CI run 41145654354) [V]. So what the worker's profile
must allow is still being found. Only the memory row is measured for the
worker itself.

**A boundary in Apple's own history** [V]. The guest-to-host escapes Apple
has published were all in host tooling that trusted guest output, not in
the hypervisor:
- GHSA-44v5-vx46-ghv6, high: a container-written kubeconfig ran a command
  on the host. It was in the k8s plugin and fixed in 1.5.0.
- GHSA-x7pf-2jmj-pgcq, high: an unchecked id deleted host files outside
  the bundle.
- The run of image-unpacking advisories in containerization at 1.3.1 and
  1.4.1.

**What it means here** [I]. The VM boundary is real, but mecha's own host
side must never parse what the guest says beyond the pool's framed
protocol, which `monty-pool` already decodes under `MAX_FRAME_LEN`. And the
version pin must sit above the advisories.

---

## 5. Risks

- **JSON output is not a stable interface** [V].
  - 1.0.0 reshaped the `ls`/`inspect` JSON.
  - 1.4.1 changed `system status`'s shape and slash escaping.
  - The README promises compatibility only "generally" within a major
    version.

  What follows [I]: mecha would read as little JSON as possible. Ideally
  that is only `container list --all --format json` filtered by its own
  label, for the reaper. The parse would be strict, with unknown read as a
  refusal, and the release pin would carry a fixture of the shape it was
  checked against.
- **A daemon, and a login session** [V].
  - `container system start` registers `container-apiserver` as a launch
    agent in `gui/<uid>` under Aqua, or `user/<uid>` in a Background
    session (`ServiceManager.getDomainString`). Helpers for images,
    networks and each container run under it.
  - In a session with neither, the bootstrap failure is swallowed, and the
    start reports an XPC error (#2008, open, reproduced on a GitHub
    `macos-26` runner).
  - It cannot run from a LaunchDaemon (#1514, open).
  - Under per-container churn, operations can hang with no timeout and
    block one another (#1884, open, 1.0.0).

  What follows [I]: mecha would `preflight` through the real path, and
  refuse code mode when the apiserver does not answer within the preflight
  timeout. It must never fall back. A `mecha serve` launch agent
  (`launchd.rs`) needs M-C12.
- **Privileges** [V].
  - The install needs an admin password.
  - Running needs none.
  - The runtime and network helpers are signed with
    `com.apple.security.virtualization`.
  - The kernel download is an interactive prompt unless
    `--enable-kernel-install` is passed.
  - Attaching a network can meet macOS Local Network privacy (#1514).
    `--network none` should not [I].
- **Sleep.** No open or closed issue reports a VM that misbehaved across a
  host sleep [V] (issue search, 2026-10-30).
  - The framework re-syncs the guest's wall clock from the host every
    30 s through the agent (`TimeSyncer`, `main`) [V]. So the guest's
    realtime can lag by up to 30 s after a wake.
  - mecha's program clock is host-side (`tokio::time::Instant`), and
    `max_feed_duration` is measured by the worker [V] (`monty-pool`
    `PoolConfig`). A wall-clock jump inside the guest should not move a
    monotonic one [I].
  - M-C11 measures this.
- **CI cannot test it** [V].
  - GitHub's arm64 macOS runners: "Nested-virtualization is not supported
    due to the limitation of Apple's Virtualization Framework."
  - Virtualization.framework nests only Linux guests, and a GitHub macOS
    runner is itself a macOS VM (maintainer, #191).
  - Apple's own CI for `container` runs on self-hosted Macs
    (`runs-on: [self-hosted, macos, tahoe, ARM64]`).

  So a container backend could be measured only on the owner's Mac, with
  `MECHA_TEST_REQUIRE_BACKENDS=1`. No pull request could ever gate on it,
  unlike Seatbelt, which macOS CI already measures
  (`mecha-core/tests/seatbelt_backend.rs`).
- **Churn.** Ten releases in the twenty weeks after 1.0.0, and nineteen
  published advisories across the two repositories [V]. Pinning to an
  exact release (§1) means a mecha PR for every security fix.

---

## 6. Is `sandbox-exec` deprecated, and what does Apple recommend?

**Deprecated in its documentation for years, still shipped, and with a
new Apple-recommended alternative in macOS 27 that mecha cannot use.**

- **The man page says "The sandbox-exec command is DEPRECATED"** and
  points to the App Sandbox. A Hacker News commenter quotes it from macOS
  10.13.6 (2017) and says it still reads so on 26.3 [M3]. As of 2026 it
  still ships and works:
  - mecha's macOS CI runs it [V] (`seatbelt_backend.rs`).
  - The peer probe ran it on macOS 26.6.2 (25G83) [V].

  A runtime warning on stderr ("WARNING: sandbox-exec is deprecated…") is
  claimed by `apple/containerization` #737, filed 2026-05-12 by a third
  party and reproduced there on macOS 15 [M3]. **The probe saw no such
  line**: its permissive-profile run's stderr held only the worker's own
  output [V]. Treat the banner as unverified.
- **Apple Developer Technical Support, March 2025** [C]: SBPL "has never
  been documented for third-party use, and thus is not considered API.
  That's why everything within `<sandbox.h>` is deprecated." Apple DTS
  treats it as "an implementation detail".
- **Apple DTS, August and September 2026** [C]: "macOS 27 introduced an
  alternative approach for third-party developers to sandbox child
  processes, namely the `es_new_descendants_client` mechanism within
  Endpoint Security." The supported options DTS lists are "ES, App
  Sandbox, and Virtualization".
- **What `es_new_descendants_client` is** [V] (Apple's documentation
  JSON):
  - macOS 27.0+.
  - A client that gets "auth+notify events for its descendant processes."
  - "Requires the com.apple.developer.endpoint-security.client
    entitlement. Does NOT require root privilege. Does NOT require TCC
    approval."

  That entitlement is restricted, granted by Apple per developer. DTS says
  a narrower one is an enhancement request it "can't promise" [C]. It is
  an authorization hook over the events Endpoint Security models, not a
  declarative profile. It covers no network (that is Network Extension's)
  and no memory [I].
- **Whether Apple has a removal date: none was found** [V], 2026-10-30.
  #737's question is unanswered; one comment adds a Feedback number. One
  tracker reads a Claude Code issue (#26095) as "the first in-the-wild
  failure of sandbox-exec on Darwin 25". **The issue itself says
  `sandbox-exec -n no-network echo "test"` succeeds**, so it is no evidence
  of breakage [V].

**What it means for the Seatbelt backend** [I]:
- Its footing is "unsupported, unremoved", the same footing Chrome, Codex
  and Claude Code stand on.
- Apple's two supported answers are the App Sandbox, which is not built
  for a CLI's child, and Endpoint Security's descendants client, behind a
  restricted entitlement an open-source CLI cannot count on.
- The third, Virtualization, is this document's subject. So `container`
  is also the hedge if `sandbox-exec` ever goes.
- The Seatbelt backend's existing rule covers a removal. A profile that
  fails to apply stops the run (`Sandbox::preflight`).

---

## 7. Recommendation

**Seatbelt is the default. `container` is an opt-in strong backend on
Apple silicon with macOS 26+, using option A. Not `container` only, and
not Seatbelt only.**

- **Why not `container` only:**
  - It needs macOS 26, Apple silicon, an admin install and a running
    daemon.
  - It adds ~0.7 s per worker.
  - No CI can measure it.

  Making it the only Mac backend would turn code mode off for every Mac
  that lacks one of those, and leave the backend unmeasured by CI.
- **Why not Seatbelt only.** It leaves the memory axis with no external
  wall on a Mac:
  - Monty's allocator limit is the interpreter's own accounting, the
    first wall.
  - Linux has a second, the cgroup (`[sandbox] memory_mb`).
  - A Mac under Seatbelt has none. The CI probe measured rlimits failing
    to bind.

  `container` is the only route found to a second wall. Its escape
  boundary is also a different kernel, which matters for an interpreter
  whose round 1 of Hack Monty was an escape (`CODE-MODE-DESIGN.md` §7).
- **Why option A.** It is the only option that keeps Linux's
  per-worker isolation without a new protocol. Its cold start amortizes
  over the pool's 100-checkout reuse.
- **What the strong backend would be.** A `Confinement` variant whose
  `for_sandbox` refuses anything but macOS 26+ on arm64, a pinned
  `container` release whose binaries hash to the pin, and a reachable
  apiserver. It refuses by name, never falls back to Seatbelt silently,
  and `preflight`s through the real launcher, like bwrap.
  - `--memory` is set just above `Backstops::max_memory`, so the
    interpreter's own `MemoryError` fires first and the VM is the outer
    wall, as `memory_mb` is on Linux.
  - A kill inside the guest reaches the pool as a dead worker, so it reads
    as `End::Backstop`, not `Err`, as the cgroup kill does.
- **What to record against Seatbelt meanwhile.** The Seatbelt backend
  should say what it lacks, not imply parity. `preflight` or `doctor`
  states that the worker has no external memory ceiling on this platform.
  "Unknown is never clean" applies to a missing wall as much as to a
  missing reading.

**Recommended against:**
- **Option B.** It shares a kernel and a cgroup between workers, and
  inherits #2439.
- **Option C.** It needs a second wire protocol and a second binary.
- **Driving the CLI's JSON beyond one filtered list.**
- **An unprivileged install from an expanded pkg.**
- **Building any of this before M-C9.** If a SIGKILLed client orphans the
  VM, option A needs the reaper in §3, and its cost belongs in the
  decision. If the VM dies with its client, A is a launcher script and an
  image.

**When to revisit:**
- An Apple answer on #737.
- A supported SBPL successor that needs no restricted entitlement.
- Containerization adding VM snapshots, which would make warm starts
  real.
- GitHub enabling nested virtualization on its macOS runners.

---

## 8. Measurements to run on a Mac

Each measurement runs on the owner's Apple-silicon Mac under macOS 26 or
27, with `container` 1.6.1 installed and `container system start` done.
Prefix each with `export MECHA_SESSION_KIND=test` if any mecha binary is
involved. Record `sw_vers`, `sysctl -n machdep.cpu.brand_string` and
`container system version --format json` beside every result.

`hyperfine` comes from `brew install hyperfine`. Alpine and Python images
are used only to *probe* the boundary. The worker image is built from the
pinned binary.

**Setup: the pinned Linux worker as an image.** The wheel and binary
hashes are `worker::PINS`' `aarch64-linux` entry.

```bash
mkdir -p ~/mecha-ac && cd ~/mecha-ac
curl -sSfLo m.whl https://files.pythonhosted.org/packages/80/85/317a61daca57f3211d53f868977dbd1d491e0212082f0e79cd6df50137de/pydantic_monty_runtime-1.4.0-py3-none-musllinux_1_1_aarch64.whl
echo "ae4e68d623b9038f8a41baa95e06e8d32402ad972120b068d14c336bb924eb03  m.whl" | shasum -a 256 -c
unzip -p m.whl pydantic_monty_runtime-1.4.0.data/scripts/monty > monty && chmod +x monty
echo "989eb5bacc6844fce2472820d28f0ab3247846d29ab62d0dbb521004f3224cc7  monty" | shasum -a 256 -c
printf 'FROM scratch\nCOPY monty /monty\nENTRYPOINT ["/monty"]\n' > Containerfile
# Build the image the way §1 proposes mecha ship it: a one-layer OCI
# archive loaded with `container image load`, not `container build`. The
# latter needs the BuildKit builder VM (`container builder start`), which
# reserves its own memory and would perturb M-C13. Either works for the
# measurements, but only the load path is the one that would ship.
container build -t mecha-monty:1.4.0 .   # if used: run `container builder stop` before M-C13
container image inspect mecha-monty:1.4.0 > image.json
F='--rm --network none --no-dns --cap-drop ALL --user 65534:65534'
```

**M-C1: cold start per worker**, against Seatbelt with the macOS worker.

```bash
hyperfine -w 2 -r 20 --export-json c1.json "container run $F mecha-monty:1.4.0 -c 1"
curl -sSfLo mac.whl https://files.pythonhosted.org/packages/cb/9b/0eedee19d738a168cb4fbcefb1be38cb7c729c750d0293379483e38fa71a/pydantic_monty_runtime-1.4.0-py3-none-macosx_11_0_arm64.whl
echo "317edf6c62ea62c8a0f456ec13be889387cd16a917441fc087953cae042e6d75  mac.whl" | shasum -a 256 -c
unzip -p mac.whl pydantic_monty_runtime-1.4.0.data/scripts/monty > monty-mac && chmod +x monty-mac
echo "6c1d5b14023d213b4a862131739c5431ac980b5f89a3267c75fb6be2c2d39fdd  monty-mac" | shasum -a 256 -c
hyperfine -w 2 -r 50 "sandbox-exec -p '(version 1)(allow default)(deny network*)' ./monty-mac -c 1"
```

**M-C2: concurrent boots**, which tests the serialization claim.

```bash
hyperfine -r 5 "seq 4 | xargs -P4 -I{} container run $F mecha-monty:1.4.0 -c 1" \
               "seq 4 | xargs -P1 -I{} container run $F mecha-monty:1.4.0 -c 1"
```

**M-C3: `exec` into a live VM** (option B's spawn cost).

```bash
container run -d --name ac-host --network none -v ~/mecha-ac:/w:ro alpine:3.22 sleep infinity
hyperfine -w 3 -r 30 "container exec ac-host /w/monty -c 1"
```

**M-C4: the scratch image under the flags option A would use.** This
tests #1737.

```bash
container run $F --read-only mecha-monty:1.4.0 -c 'print(21 * 2)'; echo "exit=$?"
container run $F --read-only --memory 64M --cpus 1 mecha-monty:1.4.0 -c 'print(1)'; echo "exit=$?"   # the smallest VM that boots
```

**M-C5: the memory ceiling holds.** Expect a kill (137) or `MemoryError`,
and never `536870912`.

```bash
container run $F --memory 128M mecha-monty:1.4.0 -c 'x = "a" * (512 * 1024 ** 2); print(len(x))'; echo "exit=$?"
```

**M-C6: CPU.** Expect 1, and a ceiling near 100%.

```bash
container run $F --cpus 1 alpine:3.22 nproc
container run -d --name ac-spin --network none --cpus 1 alpine:3.22 sh -c 'yes >/dev/null & yes >/dev/null & wait'
sleep 10; container stats --no-stream --format json ac-spin; container kill ac-spin; container delete ac-spin
```

**M-C7: network and vsock.** Expect only `lo`, a failed fetch, and every
vsock connect refused.

```bash
container run --rm --network none alpine:3.22 sh -c 'ip -o link; wget -T 3 -qO- http://1.1.1.1 && echo REACHED || echo blocked'
container run --rm --network none python:3.13-alpine python3 -c '
import socket
for cid in (1, 2):
    for port in (1024, 1025, 2000, 5000):
        s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM); s.settimeout(2)
        print(cid, port, s.connect_ex((cid, port))); s.close()'
```

**M-C8: the environment.** The pool's launcher runs with an empty
environment. Expect the CLI to work under `env -i`, and the guest to show
nothing of the host's.

```bash
env -i "$(command -v container)" run $F alpine:3.22 env; echo "exit=$?"
FOO_SECRET=leak SSH_AUTH_SOCK=/tmp/x container run $F alpine:3.22 env | grep -E 'FOO_SECRET|SSH_AUTH' || echo none-leaked
```

**M-C9 (decisive): what a SIGKILLed client leaves running.** (a) Run a
busy worker, as the pool would kill it. (b) Run an idle `subprocess`
worker whose stdin closes with the kill. In (b) the client is killed
*before* stdin closes, deliberately: the question is whether the VM
outlives its client, so the EOF must not arrive first and end the worker
on its own. (c) Run the `exec` path. For each,
note whether the VM or process is still running 10 s and 60 s later, and
whether the host CPU stays busy.

```bash
container run -i $F --name ac-busy mecha-monty:1.4.0 -c 'while True: pass' & p=$!
sleep 5; kill -9 $p; sleep 10; container list --all --format json | grep -c ac-busy; top -l 1 -o cpu | head -15
python3 -I -c '
import subprocess, time, os, signal
p = subprocess.Popen(["container","run","-i","--rm","--network","none","--name","ac-idle","mecha-monty:1.4.0","subprocess"], stdin=subprocess.PIPE)
time.sleep(5); os.kill(p.pid, signal.SIGKILL); p.stdin.close(); time.sleep(10)
print(subprocess.run(["container","list","--all","--format","json"], capture_output=True, text=True).stdout.count("ac-idle"))'
container exec -i ac-host /w/monty -c 'while True: pass' & p=$!
sleep 5; kill -9 $p; sleep 10; container exec ac-host ps -o pid,stat,time,args
```

**M-C10: the `exec -i` descriptor leak** (#2439). The pipe count should
not climb by 50.

```bash
pgrep -fl container-runtime-linux   # set rt to the pid of the one serving ac-host
lsof -p "$rt" | grep -c PIPE
for i in $(seq 50); do echo | container exec -i ac-host /w/monty -c 1 >/dev/null; done
lsof -p "$rt" | grep -c PIPE
```

**M-C11: sleep and wake.** Expect the VM alive and the program done. Note
the guest clock's offset at the first `date` after wake, and again 30 s
later.

```bash
container run -d --name ac-sleep --network none -v ~/mecha-ac:/w:ro alpine:3.22 sh -c '/w/monty -c "
import time
t = time.monotonic()
while time.monotonic() - t < 300: pass
print(\"done\", time.monotonic() - t)"; sleep infinity'
sleep 60; pmset sleepnow   # wake the Mac by hand after about two minutes
date -u; container exec ac-sleep date -u; sleep 30; date -u; container exec ac-sleep date -u
sleep 300; container logs ac-sleep
```

**M-C12: the daemon from the contexts mecha runs in.** Run it from an SSH
session (the Background domain) and from a launch agent like `mecha
serve`'s.

```bash
container system stop
ssh localhost 'container system start --enable-kernel-install && container system status --format json'
launchctl print gui/$(id -u) | grep -i com.apple.container; launchctl print user/$(id -u) | grep -i com.apple.container
```

**M-C13: idle and peak host memory per VM, and whether freed memory comes
back.**

```bash
# Size the VM well above the allocation, so the guest's own limit is not
# what gets measured. Allocate through the pinned worker itself: an anonymous
# allocation the guest kernel charges and then releases, which is the path
# code mode takes. (A /dev/shm write is tmpfs-limited and cgroup-charged, so
# it fails or OOM-kills instead of freeing, and the before/after readings
# would wrongly agree.)
container run -d --name ac-mem --network none --memory 1G --entrypoint /bin/sh mecha-monty:1.4.0 -c 'sleep infinity' 2>/dev/null \
  || container run -d --name ac-mem --network none --memory 1G alpine:3.22 sleep infinity
sleep 5
ps -axo pid,rss,command | grep -E 'Virtualization.VirtualMachine|container-runtime-linux' | grep -v grep   # idle
container cp ./monty ac-mem:/w/monty 2>/dev/null || true
container exec ac-mem /w/monty -c 'x = "a" * (300 * 1024 ** 2); print(len(x))'
ps -axo pid,rss,command | grep -E 'Virtualization.VirtualMachine' | grep -v grep   # after the allocation was freed
sleep 30
ps -axo pid,rss,command | grep -E 'Virtualization.VirtualMachine' | grep -v grep   # and 30 s later
```

**M-C14: what the pin would hash, and the JSON it would parse.** Record
the shapes per release.

```bash
pkgutil --pkgs | grep -i container   # then: pkgutil --files <that id> | head -50
find /usr/local -path '*container*' -type f -perm -u+x -exec shasum -a 256 {} + 2>/dev/null
codesign -dv --verbose=2 "$(command -v container)" 2>&1 | grep -E 'Identifier|TeamIdentifier|Authority'
container system version --format json > version.json; container list --all --format json > list.json
```

**Cleanup:**

```bash
container kill ac-host ac-sleep ac-mem 2>/dev/null; container delete ac-host ac-sleep ac-mem 2>/dev/null; container image delete mecha-monty:1.4.0
```

**What decides it:**
- M-C9 decides whether option A needs a reaper.
- M-C1, M-C2 and M-C13 decide whether the cost is acceptable beside
  bwrap's 4 ms.
- M-C5, M-C7 and M-C8 decide whether the isolation table in §4 holds as
  written.
- M-C12 decides whether `mecha serve` can drive it at all.

---

## Sources

Apple, in source and release data:
- [apple/container](https://github.com/apple/container) at tag 1.6.1:
  - `README.md` and `docs/technical-overview.md`;
  - `docs/command-reference.md` and `docs/container-system-config.md`;
  - `Sources/ContainerCommands/Container/ContainerRun.swift`;
  - `Sources/Services/ContainerAPIService/Client/{Utility,ProcessIO}.swift`;
  - `Sources/Services/RuntimeLinux/Server/RuntimeService.swift`;
  - `Sources/ContainerPlugin/ServiceManager.swift`;
  - `signing/*.entitlements` and `.github/workflows/common.yml`.
- [apple/container releases](https://github.com/apple/container/releases):
  1.0.0 and 1.6.1. [PR #2436](https://github.com/apple/container/pull/2436),
  macOS 27 runners.
- [apple/containerization](https://github.com/apple/containerization) at
  `fd4a84a9`:
  - `Sources/Containerization/{LinuxContainer,VMResources,TimeSyncer}.swift`;
  - `Sources/ContainerizationOCI/LinuxSeccomp+DefaultProfile.swift`;
  - `vminitd/Sources/VminitdCore/{AgentCommand,StandardIO}.swift`.
- Security advisories:
  [container](https://github.com/apple/container/security/advisories) and
  [containerization](https://github.com/apple/containerization/security/advisories).
  Cited: GHSA-44v5-vx46-ghv6, GHSA-xwgf-4rc5-p4m4, GHSA-vgrw-36j7-r93w and
  GHSA-x7pf-2jmj-pgcq.
- Issues:
  - [#58](https://github.com/apple/container/issues/58) and
    [#738](https://github.com/apple/container/issues/738): start-up.
  - [#191](https://github.com/apple/container/issues/191): nested
    virtualization.
  - [#386](https://github.com/apple/container/issues/386): `--network none`.
  - [#1514](https://github.com/apple/container/issues/1514) and
    [#2008](https://github.com/apple/container/issues/2008): daemon
    contexts.
  - [#1737](https://github.com/apple/container/issues/1737): read-only
    `/etc`.
  - [#1884](https://github.com/apple/container/issues/1884): hangs.
  - [#1921](https://github.com/apple/container/issues/1921): VM memory.
  - [#2439](https://github.com/apple/container/issues/2439): the `exec -i`
    leak.
  - [containerization #737](https://github.com/apple/containerization/issues/737):
    the `sandbox-exec` timeline.
- WWDC25 [Meet Containerization](https://developer.apple.com/videos/play/wwdc2025/346/)
  and WWDC26 [Discover container machines](https://developer.apple.com/videos/play/wwdc2026/389/).
- Apple Developer Forums [thread 777509](https://developer.apple.com/forums/thread/777509)
  (Apple DTS on SBPL, 2025 and 2026) and
  [thread 842442](https://developer.apple.com/forums/thread/842442).
- [`es_new_descendants_client(_:_:)`](https://developer.apple.com/documentation/endpointsecurity/es_new_descendants_client(_:_:)),
  read from the documentation's JSON.

Others:
- `monty-pool` 1.4.0, from crates.io: `src/lib.rs` and `src/worker.rs`.
- [pydantic/monty](https://github.com/pydantic/monty) at `66fff6bf`:
  `crates/monty-runtime/src/main.rs`.
- [GitHub-hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners):
  the arm64 macOS limitations.
- [Homebrew `container` formula](https://formulae.brew.sh/formula/container).
- [dcon benchmarks](https://github.com/o1x3/dcon/wiki/Benchmarks-and-Comparison)
  and the [DEV Community benchmark post](https://dev.to/aairom/inside-apple-containers-architectural-deep-dive-cli-parity-and-benchmarking-against-podman-48ja).
- [anthropics/claude-code #26095](https://github.com/anthropics/claude-code/issues/26095).
- [Hacker News, "macOS's little-known command-line sandboxing tool"](https://news.ycombinator.com/item?id=47101200).
- This repository:
  - `DOCUMENT-EXTRACTION-DESIGN.md` §3.1;
  - `CODE-MODE-DESIGN.md` §5, §7, §10 and §12;
  - `ARCHITECTURE.md` §Code mode's interpreter;
  - `mecha-core/src/interpreter/{monty,worker}.rs`;
  - CI run 41145654354 (`scratch/seatbelt-monty-probe`).
