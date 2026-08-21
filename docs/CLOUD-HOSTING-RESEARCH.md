# Hosting mecha in the cloud

Research pass, 2026-08-21. The local story is written down — `docs/LLAMA-SERVER.md`
for the server, `website/docs/getting-started/` for the install, the `update`
skill for the six surfaces. This is the other question: what changes when the
harness runs on a machine you do not own.

The short answer is that **the harness is trivially cloud-hostable and the model
is not**, and almost every real decision here is about where that split falls.

---

## The recommendation

**A small always-on Arm VPS on a tailnet, with the model left on spark and a
same-weights API as the declared fallback.** Roughly $9/month, no weights move,
no inbound port, and the trust story is unchanged except for one thing that must
be said out loud (below).

```
Hetzner CAX21 (8 GB, Ampere Arm64, $8.49/mo)
  ├─ mecha-slack.service      outbound WebSocket, no ingress
  ├─ mecha-triggers.service   ticks every minute
  ├─ ~/.mecha                 153 MB today, on the VPS disk
  └─ [providers.local] base_url = http://spark.tailnet:8080
     [providers.local] fallbacks = ["deepinfra"]   # same weights, see below
```

If the point of the exercise is that spark can be off, skip to
[Where the model goes](#where-the-model-goes) — that is the expensive half, and
the cheap half of it is $10–40/month while the honest half is ~$500.

---

## What mecha actually asks of a host

This is the part that disqualifies most of the market, so it goes first. None of
these are preferences; each one is a property of code already in the tree.

### 1. Always on, never scale-to-zero

`mecha slack connect` holds a Socket Mode WebSocket open for its whole life, and
`mecha trigger daemon` is a once-a-minute loop. Both are `Type=simple` units with
`Restart=on-failure`. A host that suspends on idle does not pause them — it drops
the socket and stops the clock, and `catch_up` then decides whether the missed
briefing runs at all.

This rules out essentially the entire "AI sandbox" product category, which is
built for the opposite shape: a fresh VM per task, torn down after. E2B's session
model, Vercel Sandbox, Daytona, Modal sandboxes and Fly.io Sprites all bill on
that assumption and advertise sleeping-when-idle as the feature. mecha is a
resident, not a job.

### 2. A durable, owner-only disk

`~/.mecha` is **153 MB** today and holds the mail OAuth tokens, every session
transcript, the learning store, the outbox, the trigger ledger and the front-door
queue. It needs a real persistent volume — not a container filesystem, and not an
object store bolted on afterwards, because the stores are `flock`-coordinated
temp-sibling-and-rename writers that assume POSIX semantics on a local disk.

Size is a non-issue. `[work] keep` already bounds `~/.mecha/work/`, and 40 GB is
generous.

### 3. Its own kernel — or `shell` degrades, loudly

This is the sharpest selection criterion and it maps exactly onto the
microVM-versus-container split.

`Sandbox::preflight` runs a real command through the real backend at startup and
**fails the run** if it cannot confine. That is deliberate: a configured sandbox
that silently does nothing is worse than none, because `shell` declares narrower
capabilities when confined and the interlock believes it. So the question "which
backend works on this host" is not a tuning question — it decides whether the
harness starts.

| Backend | Needs | On a shared-kernel PaaS |
|---|---|---|
| `bwrap` | unprivileged user namespaces | usually blocked |
| `docker` | a usable Docker daemon | needs privileged / DinD |
| `landlock` | kernel 6.2+, ABI 3 | usually works |

Landlock working is not the same as landlock being enough. Per the security
model, **landlock cannot close the network** — TCP is denied on 6.7+ kernels,
UDP is unrestrictable at any ABI, and bash alone sends over `/dev/udp` — so
`can_reach_network()` stays true, `shell` keeps declaring `external_send`, and it
never earns the interlock relaxation. The observable cost is more trifecta
refusals in ordinary local work: the exact symptom that motivated turning the
sandbox on here in the first place.

So: **prefer a host that gives you a real kernel.** Firecracker microVMs (Fly.io
Machines, exe.dev, E2B's substrate) and ordinary KVM VPSes both do; gVisor (Modal)
and plain container platforms do not. On a real kernel you get bwrap or Docker,
and with them the no-network branch that makes `shell` stop being a trifecta sink.

Note the local irony worth carrying: spark itself cannot run bwrap, because this
Ubuntu sets `kernel.apparmor_restrict_unprivileged_userns=1`, so `~/.mecha/config.toml`
is on the docker backend. A stock cloud Ubuntu image frequently has that switch
*off*, which means a cloud mecha can be **better** confined than the local one,
for one sysctl.

### 4. No inbound port, and that is a feature

Both long-running services dial out — Socket Mode by design, and the factory
drain for the same stated reason ("home holds the connection open so nothing ever
has to dial home"). Nothing about mecha wants ingress.

That means: no domain, no TLS termination, no reverse proxy, no firewall
exception, no certificate renewal. It also means the *right* answer for shell
access is a tailnet rather than a public SSH port, which removes the only listener
on the box.

Contrast the closest published precedent for this shape — OpenClaw's exe.dev
install guide — which spends most of its length on nginx, port forwarding, HTTPS
and `trustedProxies`, because its gateway is a web service. mecha needs none of
that, and the difference is worth a paragraph in whatever we eventually publish.

### 5. Arch, because this project has already been bitten

`spark` is aarch64. The existing DigitalOcean droplet is x86, and the recorded
lesson from that is "never scp a local build; `file` the binary first" — a 203/EXEC
restart loop was the symptom.

An **Arm VPS keeps one build target**, which is the cheap way to not meet that
trap again. Hetzner's CAX line is Ampere Altra Arm64 and is the obvious pick;
most other providers' Arm offerings (AWS Graviton, Oracle Ampere) work too.

Practical note either way: `cargo install mecha-cli --locked` links a large Rust
binary. On a 4 GB box, add swap or build elsewhere. 8 GB is comfortable.

---

## Where the model goes

Three placements. The cost spread between them is ~50×, and the security spread
runs the other way.

### (a) Model stays on spark, harness in the cloud — ~$9/mo

The harness is an ordinary HTTP client; nothing in it holds weights. Point
`[providers.local] base_url` at spark over a tailnet and the entire local server
document still applies unchanged — the slot geometry, `-cram 32768`, the
`--cache-idle-slots` finding, `context_window = -c / -np`. Weights never move,
mail bytes only ever reach a machine you own, and the trust story is exactly what
it is today.

The failure mode is obvious: spark down, or the tunnel down, is the assistant
down — and an unattended 7am trigger is precisely when nobody notices.

**The fallback is the interesting part.** `[providers.X] fallbacks` is empty by
default on the argument that strict beats silently answering with a different
model. That argument does not reach here, because the same open weights are
served by third parties: `Qwen3.6-35B-A3B` is on DeepInfra, OpenRouter and
several others. A fallback that answers with *the same model* is the one case the
default was not written against.

Two things to get right if we do it, and both are silent when wrong:

- `context_window` on the fallback is the *provider's* window, not spark's
  1048576/4. The compaction threshold and the tool-output budget are derived from
  it and trust it.
- `provider::preflight` reads `GET /props` to verify `vision`. A hosted
  OpenAI-compatible endpoint does not serve `/props`, so the both-directions
  warning goes quiet — the declaration becomes an assertion again, which is the
  mmproj trap's shape.

### (b) Rent a GPU and run llama-server yourself — ~$460–570/mo

A 48 GB card (RTX 6000 Ada, L40S) holds the 20.7 GB of weights with room for real
context, and roughly $0.63–0.79/hr is ~$460–570/month at 24/7. An 80 GB A100 is
~$780/mo on demand, ~$440–490 on spot or marketplace hosts.

This keeps every *process* property: you own the server, `/props` answers, the
slot arithmetic is yours, `-cram` is yours. What it does not keep is custody —
the weights and the KV cache holding your mail sit in someone else's RAM.

**Serverless GPU is not the cheap version of this.** Modal, RunPod serverless and
friends scale to zero, and mecha's performance model is built on a *persistent*
prompt cache: a stable prefix reused across turns, slot affinity holding a
conversation on one slot, `cache_lens.rs` watching that it actually happens.
Every cold start throws that away and re-prefills the whole transcript at
~1,570 tok/s. The measured local version of this mistake cost 20.5 s for 29,570
tokens on a single turn. Scale-to-zero makes that the normal case.

### (c) A hosted open-weights API — ~$10–40/mo

`kind = "openai"` already speaks this. Current rates for the model we actually
run:

| Provider | Input /M | Output /M | Cached input /M |
|---|---|---|---|
| DeepInfra | $0.10 | $0.95 | ~$0.15 |
| OpenRouter (cheapest route) | $0.08 | $0.75 | varies by route |

Two things make this cheaper than it looks, and one makes it more expensive than
it looks.

Cheaper: **`provider/openai.rs` already reads `prompt_tokens_details.cached_tokens`
into `cache_read_input_tokens`**, so DeepInfra's automatic prompt caching is
visible to `cache_lens.rs` and priced through `Usage::cost` at the cache-read
multiplier. The instrumentation we built for the local server transfers without
a line of code.

More expensive: every turn resends the whole history. A 30k-token conversation
over 25 turns is ~750k input tokens, and it is the *input* side that dominates.
At $0.10/M cached-miss that is still under a dollar per long session, so the
arithmetic survives — but it is why the cached-token rate is the number to
compare providers on, not the headline input rate.

**The real cost is the thesis.** This project exists to make a *local* open-weight
model into a usable assistant, and the lethal-trifecta reasoning in CLAUDE.md is
downstream of the assistant holding mail, calendar and a knowledge graph. Routing
that to a third party does not break any interlock — the interlock is about what
leaves via tools — but it does mean mail bodies transit an inference provider.
OpenRouter's ZDR controls and DeepInfra's stated no-content-logging reduce that;
they do not remove it, and "the effective guarantee is the union of your setting
and the provider's policy" is the load-bearing sentence.

Worth being precise about what is gained: **(c) is the only option that survives
spark being permanently off**, and it is the right *fallback* even if it is the
wrong primary.

---

## Host shortlist

### Hetzner CAX21 — the boring recommendation

8 GB, Ampere Arm64, 20 TB traffic, **$8.49/mo**; CAX11 (4 GB) is $4.99 and CAX31
(16 GB) is $16.49. Real KVM VM, own kernel, so bwrap is available. Arm matches
spark. Note prices rose across 2026 — the Arm line took the smallest increase
(~1.3–1.4×), which is part of why it is now the value pick rather than merely the
cheap one.

### exe.dev — the interesting one

$20/mo personal: 50 VMs, 100 GB pooled disk, 200 GB transfer, extra disk
$0.08/GB/mo. Real VMs with sub-second start, root, `apt` and `systemd`, persistent
disks, private-by-default with shareable HTTPS hostnames and TLS handled for you.
Standard tier is described as 2 vCPU / 8 GB. Created and managed over SSH
(`ssh exe.dev new`), which is a genuinely nice fit for a CLI-shaped project.

What it buys over a VPS is **many cheap VMs out of one pool**, and that maps onto
something mecha actually has: `mecha eval` and `mecha batch` fan out, and eval's
sandboxed cases stage a private workspace per run. A pool of disposable VMs is a
better answer to "run the case set 5× in parallel" than one box. It is also a
plausible home for the *factory* side of this repo's world.

What it does not buy: GPUs (none advertised), and no stated always-on guarantee to
verify against — which for the Slack connector is the one thing that has to be
confirmed before committing, not assumed.

### Fly.io Machines — works, but none of its agent features apply

Firecracker, own kernel, persistent volumes, machines pinnable always-on. An
always-on `shared-cpu-1x` with 256 MB is ~$1.94/mo; a realistic 1 CPU / 1 GB with
a 10 GB volume lands around $10–20/mo. The free tier is gone as of 2026.

Two constraints decide the architecture, and both point at "one box":

- **A volume attaches to exactly one Machine.** *"A Machine can only mount one
  volume at a time and a volume can be attached to only one Machine"*, volumes
  cannot be shared between apps, and they are host-local NVMe rather than network
  storage. So the tempting split — an always-on Slack machine plus a scheduled
  trigger machine — is impossible: they would both need `~/.mecha`. Fly's own
  advice is to provision at least two volumes per app for redundancy, which for a
  single-writer store means replication mecha does not do.
- **`--schedule` is fuzzy `hourly` / `daily` / `weekly` / `monthly`.** It cannot
  express `0 7 * * *` in `America/New_York`, which is the entire point of
  `cron.rs`. Scheduled Machines also cannot be started manually via flyctl or the
  API, so they are not a testable form of `mecha trigger run`.

Net: on Fly you run **one always-on Machine with a volume**, running both units —
which is a VPS with extra steps and a per-GB volume bill that continues while the
Machine is stopped. Fine if you already live there; not a reason to move.

**Fly Sprites are the wrong product here** despite being the newer and more
agent-branded one: unlimited persistence and checkpoint/rollback are lovely, and
idle-sleep is fatal to a Socket Mode connection.

### The droplet we already have

`ubuntu-s-1vcpu-1gb-nyc1` behind `gate.mecha-factory.ai`. Too small (1 GB will not
link mecha, and running the harness beside the factory server couples two
independent version lines), and x86, which reintroduces the cross-arch trap. Keep
it doing its one job.

---

## The security delta, stated plainly

Nothing in the interlock, the path jail or the outbox changes by moving hosts —
they are properties of the process, and `work::ensure_outside_mecha_home` behaves
identically whether `$HOME` is on spark or a VPS. Two things do change, and one of
them is not fixable by configuration.

**The mailbox's blast radius moves to the host's control plane.** `oauth.json` is
mode 0600, and the scopes are now `gmail.modify` and `Mail.ReadWrite` — read *and
write* on both mailboxes. Owner-only file modes say nothing to a hypervisor, a
snapshot, or a support engineer with console access. Provider-managed disk
encryption does not help, because the provider holds the key. This is the honest
cost of cloud-hosting a personal assistant and it should be written in whatever we
publish rather than implied.

What genuinely reduces it:

- **No public SSH.** Tailnet or WireGuard only; the box then has zero listeners.
- **A dedicated app registration**, so a revoke is scoped to the cloud instance
  and does not take spark's grants down with it.
- **`mecha doctor` already reports dead auth** with the re-auth remedy, and exits
  77 (`EX_NOPERM`) for "re-auth needed" — which is what makes a revoke recoverable
  rather than a silent three-day outage. That precedent (2026-08-11) is exactly
  the failure mode a second, less-watched machine invites.

**Dropping an image on the TUI cannot work over SSH, ever.** A terminal turns a
drop into a bracketed paste of *the laptop's* path, and the process resolves it on
the box at the other end; the bytes never leave the laptop. This is already
documented, and cloud hosting makes it permanent rather than situational: the
Slack conduit becomes the only image door, which is precisely why it exists.

One thing that is already right: `[agent] timezone` exists because the machine
runs UTC and the user does not. Every cloud box runs UTC. No change needed.

---

## What is disqualified, and why

Recorded so the next pass does not re-derive it.

| Candidate | Why not |
|---|---|
| E2B, Vercel Sandbox, Daytona, Modal sandboxes | Ephemeral session model. mecha is a resident with two daemons and 153 MB of state. |
| Fly.io Sprites | Idle-sleep is the advertised feature and the disqualifier. |
| Modal (as harness host) | gVisor: shared kernel, syscall interception. bwrap and Docker both unavailable; you land on landlock, which cannot close the network. |
| Serverless GPU (Modal, RunPod serverless) | Destroys the prompt cache and slot affinity on every cold start — the thing `cache_lens.rs` exists to watch. |
| Container PaaS (Railway, Render, Fly apps-without-volumes) | No own kernel, no reliable persistent POSIX disk. Both #2 and #3 fail. |
| The existing DigitalOcean droplet | 1 GB, x86, and already has a job. |

---

## The trigger daemon's once-a-minute tick

Raised as a cloud-hosting worry: a resident process waking every minute looks
expensive on a rented box. **Measured, it is not** — and the design already ships
the alternative for anyone who disagrees.

### What a tick actually is

Read `~/.mecha/triggers/*.toml`, read the run ledger, and for each trigger
evaluate `Trigger::due` — `prev_at_or_before(now)` against the last accounted
slot. Pure arithmetic on a handful of small files. **No network, no model, no
tokens, no provider connection.** The daemon then sleeps with
`until_next_minute()`, which sleeps to the next *wall-clock* minute rather than
for sixty seconds, so ticks stay aligned however long a run took.

### What it costs, measured on spark

From the running unit after 8.8 hours (~528 ticks):

| Metric | Value |
|---|---|
| CPU consumed | 1.237 s over 31,716 s wall |
| Share of one core | **0.0039%** |
| Per tick | ~2.3 ms |
| Per day | ~3.4 CPU-seconds |
| `MemoryCurrent` | 8.5 MiB (RSS 15.4 MiB) |

A cold `mecha trigger tick --dry-run` as a fresh process — binary load included —
is **under 10 ms and 11.8 MB peak RSS**.

So the tick is not the thing to optimise. On a VPS billed by wall clock it is
free; the box is paid for whether or not it thinks.

### The counterintuitive part

"Replace the poll with a systemd `OnCalendar` timer" is the obvious event-driven
move, and it is **more expensive, not less**. systemd would `exec` a fresh
`mecha` 1,440 times a day — process creation plus an 11.8 MB binary load each
time — against ~2.3 ms of syscalls in a resident process. Polling inside one
process beats process-per-tick by a wide margin here.

What the timer form *is* good for is robustness and ops: no daemon to die,
`systemctl list-timers` as the status surface, and coalescing. **It needs no code
either way** — `tick` is the primitive precisely so that a crontab line or a
timer reaches the same answer, because due-ness is a function of the ledger and
the clock rather than of anything the scheduler remembers. One caveat if we
document it: systemd's `Persistent=true` would double up with mecha's own
`catch_up`, and mecha's is the better of the two (it computes backwards, so a
week's downtime owes one briefing rather than forty).

### The real improvement available, and why it is small

Sleep until the **next due slot** rather than the next minute. The parts already
exist: `Schedule::next_after` is public, and `tick` already computes it in the
`Due::Not { next }` arm for `--dry-run`. Taking `min(next_after)` across enabled
triggers turns 1,440 wakeups/day into roughly one.

Three things it would cost, in descending order of importance:

- **The store would go unwatched.** Today the minute tick is what makes
  `mecha trigger add` / `enable` take effect without restarting the daemon. A
  sleeping daemon needs an inotify watch on `~/.mecha/triggers/`, or a capped
  sleep (say 5 minutes), which is a poll again with a longer stride.
- **`TICK_GRACE` loses its justification.** The two-minute grace exists *because*
  "the scheduler ticks on the minute, so a slot is always a few tens of seconds
  old by the time anything looks at it". Waking at the slot changes what
  `catch_up = "never"` means, which is a behaviour change to a documented knob.
- **It buys ~3.4 CPU-seconds/day.**

Where it would genuinely matter is a **laptop**: 1,440 wakeups/day does block
deep C-states, which is exactly what timer coalescing exists to prevent. That is
a battery argument, not a cloud one.

### The argument that settles it for cloud hosting

**The trigger daemon is not what keeps the VM awake.** `mecha slack connect`
holds a Socket Mode WebSocket open with keepalives — a more frequent wake source
than a once-a-minute arithmetic pass, and non-negotiable if Slack is a front-end.
Optimising the tick to reduce wakeups saves nothing while the connector runs, and
the connector is the reason the box has to be always-on in the first place.

### The one thing that does degrade

`TriggerStore::last_slots` parses **the whole `runs.jsonl`** on every tick, and
the ledger never shrinks — it is the record, and nothing here deletes. Today that
is 8.4 KB and one trigger, so it is noise. With a dozen triggers over a few years
it becomes a few MB re-parsed 1,440 times a day: still trivial in CPU terms, but
it is the only part of this loop whose cost is a function of history rather than
of the schedule. If anything here is worth changing first, it is this — a tail
read or a cached high-water mark — not the tick interval.

### Genuinely event-based scheduling: what it would take

Worth separating "wake less often" (above, marginal) from "stop scheduling on the
clock at all" (a feature, not a refactor).

- **External schedulers are not it.** Cron, systemd timers, Fly scheduled
  Machines, Cloudflare Cron Triggers, EventBridge — all of them fire a *process*,
  and a mecha fire needs `~/.mecha` (tokens, ledger, sessions) plus a model
  connection. On Fly specifically the volume constraint forbids a separate
  scheduled Machine, and `--schedule` cannot express a 7am `America/New_York`
  slot. They move the clock, not the work.
- **The real version is mail-driven triage**, and the providers split on the one
  property that matters here. Gmail's push goes through Cloud Pub/Sub, and a
  **pull** subscription needs no public endpoint — the worker dials out, exactly
  like Socket Mode, so it preserves the no-ingress property that makes cloud
  hosting cheap. Microsoft Graph offers webhooks only, and *"Graph needs a
  publicly reachable HTTPS endpoint"* — which reintroduces ingress, TLS and a
  validation handshake on the box holding the mail tokens.
- That asymmetry means an event-driven trigger would work for one account and not
  the other, which collides with the unified surface's rule that **the model names
  an account, never a provider**. A trigger that only exists for Gmail is a
  provider leaking through. Not a blocker, but it is the design question to answer
  first — before any of it is worth building.

---

## Open questions

- **Does exe.dev guarantee always-on for a VPS-tier VM?** Everything else about it
  fits; this is the one property that decides it, and it is not stated on the
  pricing page. One test: a VM holding a WebSocket for a week.
- **Is `kernel.apparmor_restrict_unprivileged_userns` off on the target image?**
  If yes, the cloud instance runs bwrap where spark cannot — cheaper confinement
  and the network-closed branch. Check before choosing `kind`, and let preflight
  be the proof, not the release notes.
- **Two mechas, one mailbox.** Nothing in the design says a second instance is
  safe: the outbox, the trigger ledger and the learning store are all
  single-writer-per-machine, and two triggers daemons on one Google account would
  both draft the morning replies. If a cloud instance runs, spark's triggers
  should be disabled, not duplicated — and that is worth a doctor check rather
  than a note.
