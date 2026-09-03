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

And the graph, from **`~/Github/mecha-graph`** — installed *with* mecha since
2026-08-16, because `~/.mecha/config.toml` runs `~/.cargo/bin/mecha-graph-mcp`,
not a repo path.

**This inverted on 2026-09-01, and the old rule is the trap now.** Until then
the public checkout was a generated artifact — `git archive HEAD` from
`~/Github/personalized_knowledge_graph` minus an exclusion list, through a
gate that *deletes* the tree it refuses — and this file said in bold that
`~/Github/mecha-graph` was not a source. It is the source now. Work happens
there; the private repo keeps only the ten files the export used to strip
(the gold eval sets derived from real episodes, the operator docs, and the
roster tooling) and is no longer where code is written.

What made that safe was moving the gate rather than dropping it. The public
repo carries `.githooks/pre-push` and `.github/workflows/denylist.yml`, both
running the same roster from outside the repository (`~/.mecha-graph/
denylist.txt`, 0600, and the `PUBLIC_DENYLIST` secret), and both fail closed:
a missing or empty roster refuses the push rather than waving it through.
Install the hooks once per clone — `git config core.hooksPath .githooks` —
because a clone without it is a clone with no gate.

The version drift is what forced it: the export existed to keep two repos
level and had let them reach 0.1.3 against 0.1.4, which is the cost the
split was charging. The last export ran on 2026-09-01 and is
`mecha-graph#2`; the histories stay disjoint, and nothing was back-published.

**The boundary is not clean, and the exception is operational.**
`scripts/nightly-mecha.sh` is private-only *by design* — the export strips
it — and the 08:00 crontab line runs it from
`~/Github/personalized_knowledge_graph`. So the private repo is not retired:
it still holds that script, the gold eval sets and the roster tooling, and
both crontab lines still point at it. `scripts/nightly.sh` exists in both;
the cron runs the private copy.

A change spanning the two must land **public first**. The gossip cooldown
arc is the worked example: `--min-sources` is a flag on the public binary
and `nightly-mecha.sh` is the private caller that passes it, so landing the
private half first gives the nightly a flag the installed binary does not
have.

```bash
cargo install --path mecha-graph     --locked --force   # mecha-graph (CLI)
cargo install --path mecha-graph-mcp --locked --force   # mecha-graph-mcp (MCP server)
```

**Both lines, every time — and the second one is the one that matters at
runtime.** They are separate crates that both link `mecha-graph-core`, so
installing `mecha-graph` alone leaves the MCP server running whatever
library code it was last built against, silently. Found 2026-08-24: a
session developing in the graph repo had been running
`cargo install --path mecha-graph` all evening and left `mecha-graph-mcp`
an hour stale — across an arc whose central fix was in
`fact::normalize_predicate`, which had been auto-registering any predicate
it could not alias (49 of 83 predicates arrived unreviewed that way). The
CLI stopped doing it; the MCP server, which reaches that code through
`kg_upsert` → `assert_fact`, would have gone on minting predicate fragments
through the one write path nobody watches. **A stale MCP binary is worse
than a stale CLI**, because everything mecha does at runtime goes through
it and nothing about it announces its version.

Verify: `mecha --version` matches mecha's workspace `Cargo.toml`, and the
graph server answers from the *installed* path:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | ~/.cargo/bin/mecha-graph-mcp | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["result"]["tools"]), "tools")'
```

### 1b. The web app's assets

`mecha serve` serves the UI from `[web] assets` in the live config, which
points at **`~/.mecha/web/dist`** — a stable home, deliberately not a repo
or worktree path (production once served from a Claude session's worktree,
which is the deferred-failure shape: fine until the session directory is
cleaned). The dist is a build artifact, not in git, so a binary install
does not update it:

```bash
cd <clean worktree>/web && npm ci && npm run build
rsync -a --delete dist/ ~/.mecha/web/dist/
```

**Before that rsync, ask what is deployed — the dist may be another
lane's live test.** `git tag -l deployed-local` in the main checkout names
the commit whose build is on the box when a session deployed something
other than main (re-point it whenever you deploy; delete it when main is
deployed). And read the mismatch correctly even when the tag is missing:
a served bundle hash that matches no tree you can see is far more likely
to be **another session's deployment than a leftover** — on 2026-08-29 it
was exactly that, filed as "stale", and the rsync that followed silently
reverted a surface the owner was live-testing. If the dist is not what
you are about to install, announce to the live sessions before replacing
it.

Then restart `mecha-serve.service` (step 2). Verify the *served* page, not
the directory: the 8443 door returning 200 with the new bundle hash.

When a header probe echoes a value back (`If-Modified-Since` from a
`Last-Modified` you just grepped), strip carriage returns first —
`| tr -d '\r'` — or the CR rides into the outgoing header and the server
answers 400, which reads exactly like a server bug and is your curl. Two
sessions hit it independently within an hour on 2026-08-29.

### 2. The long-running services

These hold an open file handle on the old binary and must be restarted *after*
step 1:

```bash
systemctl --user restart mecha-slack.service mecha-triggers.service \
                         mecha-drain.service mecha-serve.service \
                         mecha-voice-worker.service
```

**`mecha-serve` and `mecha-voice-worker` were missing from this list until
2026-08-25**, which is worth naming because the omission is the shape this
skill exists to catch. `mecha-serve` is three surfaces in one process — the
web app, the chat agent, and the mounted voice facade — so it holds a handle
on the new `mecha` binary *and* serves `[web] assets`; step 1b mentions
restarting it, but only in the branch a session reaches when it changed the
assets, so a pure-Rust change would have installed a binary nothing
re-executed. `mecha-voice-worker` is the other direction and easier to miss:
it runs `scripts/voice/worker.py` **from the repo working tree**, so it is
the one unit here that goes stale on a change that never touched Rust at
all and never appears in `cargo install` output.

**And "from the repo working tree" means from whatever branch the shared
checkout is on — check that before the restart, not after.** On 2026-09-03
`~/Github/mecha` was on a merged feature branch (a session had branched
there, as the HANDOFF banner says not to), so restarting the worker would
have relaunched the pre-merge `worker.py` and reported success; the fix
that was being deployed (#145) would not have been live. Three things
follow, each learned that morning:

```bash
cat ~/Github/mecha/.git/HEAD                      # expect: ref: refs/heads/main
git -C ~/Github/mecha status --porcelain          # expect: empty
```

- **A worktree-isolated session cannot fix this.** Its guard refuses git
  aimed at the shared checkout, and relaying the owner's instruction to
  the session that works there does not lift *that* session's gate either
  — a peer's report of the owner's word is exactly the shape a permission
  gate exists to refuse. So the switch is the owner's, or the session
  whose own user says so in its own session. Plan for that before you
  start the restart list, not when the worker is the last unit left.
- **The switch has a runnable recipe in `docs/HANDOFF.md`** (§Machine
  state, dated, 2026-09-03), reviewed eight times: prove the fast-forward
  for *both* `HEAD` and `refs/heads/main`, prove `scripts/start-moe-mtp.sh`
  unchanged across the move (that file is `llama-local`'s literal
  `ExecStart`), read any dirty file before discarding it, land on
  `origin/main` in one hop, and only then restart the worker. Copy that
  block; do not improvise a `switch main && pull`, which is the version
  that failed silently on a dirty file and would have passed through a
  stale local `main`.
- **Name the digest.** The launch-script check compares git blob ids
  (`git rev-parse <ref>:path`, `d76da36c…` that day). A peer re-checking
  with `sha256sum` got a different number for the same bytes and nearly
  read it as "the file changed", which would have restarted `llama-local`
  for nothing. `git diff --quiet A B -- path` is the algorithm-free form.

Verify the worker the same way as every other unit: its own `Uvicorn
running on http://127.0.0.1:7860` line, from a journal window that opens
*at* the restart (`--since "$since UTC"`, with `since` taken just before
`systemctl restart`), or the old process's line passes the check.

**`mecha-parakeet` is deliberately not in the list.** It runs
`scripts/voice/parakeet_server.py`, so restart it when *that* file changes —
and only then, because coming back costs a model load and voice is deaf
until it finishes.

**The timer-driven units need nothing.** `mecha-frontdoor`, `mecha-ruminate`
and `mecha-slots` are `.timer`-fired and exec fresh on each firing, so they
pick up a new binary by themselves. Knowing which list a unit is on is the
difference between a restart that matters and cargo-culting six of them.

Verify — and take a startup line, never `is-active`, since a unit that
crashes on its first request is active for a while first.
`journalctl --user -u mecha-slack.service --since "2 minutes ago"`
should show the reconnect line ("Connected to … N owner(s)"),
`mecha-triggers` should log "N trigger(s), N enabled · ticking every minute",
`mecha-serve` should print both its doors ("voice facade on
http://127.0.0.1:8990" and "mecha serve on http://127.0.0.1:63242"), and
`mecha-voice-worker` should reach "Uvicorn running on http://127.0.0.1:7860".
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
**And there is a seventh binary, which no `cargo install` reaches.** This
paragraph used to say the graph repo's own nightly "builds and runs from its
repo tree and is not mecha's concern". It does not build, and it is:
`scripts/nightly.sh` sets `PKG="$REPO_DIR/target/release/mecha-graph"` and
**executes whatever is sitting there**. Its `link --auto` step runs the kNN
linker, straight into the owner's graph, at 01:30.

Found 2026-08-26, hours after a session repaired a linker bug out of the live
graph — 30 placeholder nodes merged, 121 payloads rewritten, 23 accepted facts
re-pointed. That binary was dated **Aug 25**. Left alone, the nightly would
have re-run the old linker and re-staged the same damage while every version
string on the machine read current. So:

```bash
cd ~/Github/mecha-graph && cargo build --release
ls -l target/release/mecha-graph     # must be today
```

**Do not assume `cargo install --path` refreshed it** — where cargo puts an
install's intermediate artifacts is not a promise about that path, so check
the date rather than reason about it. This is the skill's own thesis failing
inside the skill's own text: *verify the running thing, never the repo* — and
a cron job's binary is a running thing that answers to no `--version` anybody
types.

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
purpose (`~/.ssh/mecha_factory_deploy`, commented with the box's hostname).
No `~/.ssh/config`
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

  **Some of what it finds will not be yours.** On a machine running several
  Claude sessions against one checkout, the sweep surfaces other sessions'
  test servers and their MCP children — 2026-08-26 it found three on a peer's
  `./target/debug/mecha serve --port 8894`. Walk the parent (`/proc/<pid>/status`
  `PPid`, then that pid's `cmdline`) before doing anything: a stale process is
  a thing to *report to its owner*, not to kill. Killing another session's
  process in a shared tree is the same accident the worktree rule exists to
  prevent.
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
