---
name: update
description: Update and restart everything after a change — installed binaries, the long-running services, the mecha-graph MCP server, the benchmark binary, the factory client, and the droplet. Use when asked to "update everything", "deploy", "restart the services", after cutting a release, or whenever something running looks older than the repo.
---

# Updating everything

There is no single command, and that is the whole problem. "Update" names six
independent surfaces on two machines from three repositories, each with its own
version line. Every one of them can be stale while the others are current, and
none of them complain.

The failure this skill exists to prevent, from 2026-08-15: mecha 0.1.4 was
tagged, pushed, and published to crates.io, and the services were restarted —
and `mecha --version` still said 0.1.3 afterwards, because **nothing had run
`cargo install`**. Four surfaces were stale at once that day: the installed
binaries, `mecha-triggers` (left out of the restart), the benchmark's musl
binary (eight days old, so a benchmark would have measured old code and
reported it as new), and the factory client.

## The one rule

**Verify the running thing, never the repo.**

A tag proves a commit exists. A green CI run proves it compiles. `git log`
proves someone wrote it. None of them prove that the bytes executing on this
machine contain it. Ask the process what it is, not the source tree:
`mecha --version`, `ls -l ~/.cargo/bin/`, `/proc/<pid>/exe`.

Three specific confusions worth naming, because each one has actually happened:

- **A release is not an install.** Tagging and publishing changes crates.io,
  not `~/.cargo/bin`.
- **A restart is not a reinstall.** `systemctl restart` re-executes the *same
  file on disk*. Restarting before installing accomplishes nothing.
- **A debug build is not the one that runs.** Both mecha's MCP config and the
  benchmark point at *release* paths.

## The six surfaces

Work them in this order — later ones depend on earlier ones.

### 1. Installed binaries (`~/.cargo/bin`)

From `~/Github/mecha`:

```bash
cargo install --path mecha-cli  --locked --force   # mecha
cargo install --path mecha-mail --locked --force   # mecha-mail, mecha-google, mecha-outlook
```

And the graph, from `~/Github/personalized_knowledge_graph` — installed *with*
mecha since 2026-08-16, because `~/.mecha/config.toml` runs
`~/.cargo/bin/mecha-graph-mcp`, not a repo path.

**That path is right, and it is not the `mecha-graph` checkout beside it.**
The project is called mecha-graph and its crates are `mecha-graph*`, so
`~/Github/mecha-graph` looks like the obvious source and is not one: it is a
**generated artifact**, `git archive HEAD` from the private repo minus a
hardcoded exclusion list, run through `check-public-denylist.sh` — a gate that
*deletes* the tree it refuses rather than flagging it. The two histories are
disjoint. Building from the public mirror would drop `eval/gold.jsonl`, the
operational docs and the export tooling, and would put authoring on the far
side of the gate that keeps life-derived text out of a public repo. Develop in
the private repo; export to publish. Confirmed 2026-08-22, after this
parenthetical's earlier wording ("the mecha-graph repo") sent a session looking
for which of the two was authoritative.

```bash
cargo install --path mecha-graph     --locked --force   # mecha-graph (CLI)
cargo install --path mecha-graph-mcp --locked --force   # mecha-graph-mcp (MCP server)
```

Verify: `mecha --version` matches mecha's workspace `Cargo.toml`, and the
graph server answers from the *installed* path:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | ~/.cargo/bin/mecha-graph-mcp | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["result"]["tools"]), "tools")'
```

### 2. The long-running services

These hold an open file handle on the old binary and must be restarted *after*
step 1:

```bash
systemctl --user restart mecha-slack.service mecha-triggers.service mecha-drain.service
```

**The timer-driven units need nothing.** `mecha-frontdoor`, `mecha-ruminate`
and `mecha-slots` are `.timer`-fired and exec fresh on each firing, so they
pick up a new binary by themselves. Knowing which list a unit is on is the
difference between a restart that matters and cargo-culting six of them.

Verify: `journalctl --user -u mecha-slack.service --since "2 minutes ago"`
should show the reconnect line ("Connected to … N owner(s)"), and
`mecha-triggers` should log "N trigger(s), N enabled · ticking every minute".
A service that comes back but logs nothing is not evidence of success.

### 3. The graph MCP server — folded into step 1 since 2026-08-16

`~/.mecha/config.toml` runs `~/.cargo/bin/mecha-graph-mcp` (server alias
`graph`, `prefix_tools = false`, tools are bare `kg_*`), so the graph updates
through the same `cargo install` ritual as everything else — step 1 covers
it, and step 2's restart is what hands the running Slack connector the new
server (its MCP children are spawned at connector start). What this surface
still owns: **a repo build is no longer an install here either** — a
`cargo build --release` in the graph repo changes nothing mecha can see,
which is the same trap as mecha's own binaries wearing a different repo.
The graph repo's *own* nightly (`scripts/nightly.sh`, cron 01:30) builds and
runs from its repo tree and is not mecha's concern.

mecha itself needs no change when graph tools change — it discovers tools
via `tools/list`.

### 4. The benchmark binary

`bench/run.sh` uses `target-musl/release/mecha`, a static build — never the
installed one, because the glibc build will not start in most task containers.
It is rebuilt by `bench/build-portable.sh` (which `bench/run.sh` calls), but
**check its date before trusting a scorecard**: a stale one measures old code
and labels the result with today's model.

```bash
ls -l target-musl/release/mecha && target-musl/release/mecha --version
```

### 5. The factory client (different repository, different version line)

`factory-publish` comes from `~/Github/mecha-factory`, versioned `0.2.x`
independently of mecha's `0.1.x`. `mecha-drain-follow` is a hand-installed bash
wrapper around it and is not in that repo, so it needs nothing.

**Fetch before installing.** On 2026-08-15 the local checkout was 16 commits
behind origin and still said `0.2.1` while the *installed* binary was `0.2.2` —
installing from that tree would have been a silent downgrade of a running
service.

```bash
cd ~/Github/mecha-factory && git fetch origin --tags && git status -sb
factory-publish --version          # compare against the newest tag
```

### 6. The factory server (the DigitalOcean droplet)

**Not this machine, and not covered by anything above.** The server runs on a
DigitalOcean droplet in NYC1 (`ubuntu-s-1vcpu-1gb-nyc1`, a 1 vCPU / 1 GB box)
behind `mecha-factory.ai`; everything local is a client of it. Note the apex
domain resolves to Squarespace, not the droplet — the origin you want is
`gate.mecha-factory.ai` (`compute.` is the same host).

spark reaches it as **root**, using a dedicated key generated here for this
purpose (`~/.ssh/mecha_factory_deploy`, comment `spark-8c43`). No `~/.ssh/config`
entry exists, so the key has to be named explicitly:

```bash
# Read-only: what is actually deployed right now
ssh -i ~/.ssh/mecha_factory_deploy root@gate.mecha-factory.ai \
    'factory --version; systemctl is-active mecha-factory.service'

# Deploy a released version
ssh -i ~/.ssh/mecha_factory_deploy root@gate.mecha-factory.ai \
    'factory-deploy vX.Y.Z'      # download, checksum, prove binary+config, swap, health-check
ssh -i ~/.ssh/mecha_factory_deploy root@gate.mecha-factory.ai \
    'factory-deploy --rollback'  # reinstall /usr/local/bin/factory.prev
```

The unit is `mecha-factory.service`; the binary is `/usr/local/bin/factory`,
with the previous one kept beside it as `factory.prev`, which is what
`--rollback` restores. Adding a `Host factory` block to `~/.ssh/config` would
make all of this shorter and is worth doing.

**Check before assuming it is behind.** On 2026-08-15 the droplet was already
serving `factory 0.2.4` — the newest release — while the *local* client was two
releases back at `0.2.2`. The staleness ran the opposite direction from the
guess, and one read-only `factory --version` settled it in seconds.

This is production, on a 1 GB box, serving people who are not the owner. The
read-only check above is always fine. **Ask before deploying**, and never infer
that "update everything" included the droplet — it is a different machine from
a different repository on a different version line.

## Ordering constraints

- **Install before restarting.** Reversing them restarts the old binary and
  looks like success.
- **Install before editing `~/.mecha/config.toml`, and restart anything
  long-lived after.** A key a binary does not know is a *fatal parse error* —
  `ProviderConfig` and friends carry `deny_unknown_fields`, deliberately, so a
  typo'd setting fails loudly rather than being silently ignored. The cost is
  that a new key breaks every older binary, including one **already running**.

  That failure is deferred and partial, which is what makes it expensive.
  A long-lived process parsed the config at startup, before the edit, so it
  keeps working — until it hits a path that **re-reads** config, which then
  fails in a subsystem with no apparent connection to what changed. Measured
  2026-08-21: a TUI session started at 01:20 kept running after the 03:07
  install (`/proc/<pid>/exe` showed `mecha (deleted)` — a process executes the
  inode it was launched from, so an install is invisible to it), and the
  symptom was `show_file` reporting a config parse error two hours later,
  which sent the model reading `config.toml` to investigate a version skew.

  **`/proc/<pid>/exe` ending in `(deleted)` is the check**, and it is the one
  that catches a stale *process* where `mecha --version` catches a stale
  *file*:

  Walk `exe`, never `argv`. A process launched off `$PATH` has argv `mecha
  tui`, so any `pgrep` for the install path misses precisely the interactive
  session most likely to be stale — which the first version of this check did:

  ```bash
  for p in $(ls /proc | grep -E '^[0-9]+$'); do
    t=$(readlink /proc/$p/exe 2>/dev/null) || continue
    case "$t" in
      "$HOME"/.cargo/bin/*"(deleted)") echo "stale: pid $p -> $t";;
    esac
  done
  ```

  Since 0.1.12 a stale session *degrades less*: TUI modals spawn children via
  the `/proc/self/exe` link (`mecha-cli/src/exe.rs`), so they keep working —
  running the session's own version — instead of dying with `os error 2`.
  The sweep still matters: a stale session is still running old code.

  The services in step 2 clear themselves, since restarting them re-execs.
  What this finds is everything *else*: a TUI, a `mecha chat`, anything a
  person left open in another terminal — none of which any `systemctl` line
  reaches.
- **Never build while a benchmark is running.** This host has unified memory
  and llama.cpp inference is bandwidth-bound; a parallel `cargo` build starves
  it. `scripts/start-moe-mtp.sh` records the day a "50x slowdown" was blamed on
  a flag when the real cause was memory pressure. Either finish the benchmark
  or stop it, then build.

### 7. The sandbox image

`~/.mecha/config.toml`'s `[sandbox]` runs confined `shell` in the
`mecha-sandbox` docker image (built from `scripts/sandbox.Dockerfile`). It
bakes in its own Rust toolchain, so **it goes stale independently of
everything above** — after a host toolchain bump, rebuild it or confined
`cargo` quietly diverges from the cargo that CI and the host run:

```bash
docker build -t mecha-sandbox -f scripts/sandbox.Dockerfile .
docker run --rm mecha-sandbox bash -lc 'cargo --version'   # compare to host
```

(bwrap would be the cheaper backend but is blocked on this host:
`kernel.apparmor_restrict_unprivileged_userns=1`. The config comment above
`[sandbox]` records the targeted fix if that's ever wanted.)

## What no amount of this reaches

`~/.mecha/` **is not a git repository.** `config.toml` and `triggers/*.toml`
live on one disk and are in no repo's history, so nothing above updates,
restores, or reviews them. When a change lands there — a trigger's tool
allowlist, a provider block — say so explicitly in the handoff, because the
next clone will not have it.

Related coupled values that no build step checks, each silent when wrong: a
llama-server `-c` against `context_window` in `config.toml`, a `--alias`
against that provider's `model`, and `temperature` — which mecha *sends*, so
the config overrides the server's `--temp`.
