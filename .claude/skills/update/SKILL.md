---
name: update
description: Update and restart everything after a change — installed binaries, the long-running services, the pkg MCP server, the benchmark binary, the factory client, and the droplet. Use when asked to "update everything", "deploy", "restart the services", after cutting a release, or whenever something running looks older than the repo.
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

Verify: `mecha --version` matches the workspace `Cargo.toml`.

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

### 3. The pkg MCP server (different repository)

`~/Github/personalized_knowledge_graph`. `~/.mecha/config.toml` points at
`target/release/pkg-mcp`, so **a debug build changes nothing mecha can see**,
and a newly added `kg_*` tool stays invisible until:

```bash
cargo build --release -p pkg-mcp
```

Verify by asking the server what it serves, rather than trusting the build:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | ./target/release/pkg-mcp | python3 -c 'import json,sys; print([t["name"] for t in json.load(sys.stdin)["result"]["tools"]])'
```

mecha itself needs no change — it discovers tools via `tools/list`.

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
- **Never build while a benchmark is running.** This host has unified memory
  and llama.cpp inference is bandwidth-bound; a parallel `cargo` build starves
  it. `scripts/start-moe-mtp.sh` records the day a "50x slowdown" was blamed on
  a flag when the real cause was memory pressure. Either finish the benchmark
  or stop it, then build.

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
