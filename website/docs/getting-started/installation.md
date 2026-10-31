---
title: Installation
sidebar_position: 1
description: Install mecha from crates.io with cargo, switch on what you want with mecha setup, know which parts are Linux-only, and add the optional dependencies for the sandbox and the eval fixtures.
---

# Installation

mecha is published on crates.io, so installing it is one `cargo install`. There
are no prebuilt binaries and no package in any distribution — `cargo` compiles
it on your machine either way; the difference is only whether the source comes
from the registry or from a checkout.

## Requirements

**Rust 1.96 or newer.** The workspace pins `rust-version = "1.96"` on edition
2021, so an older toolchain fails at build time with a clear message rather than
part-way through compiling a dependency. The pin is raised only to the minimum
a dependency actually needs, never to current stable — the difference is
headroom for anyone on the toolchain their distribution chose.

```bash
rustc --version        # must be 1.96.0 or later
rustup update stable   # if it is not
```

**A C compiler** (`cc`/`gcc` or `clang` — `build-essential` on Debian and
Ubuntu, the Xcode command-line tools on macOS). TLS comes from `rustls` rather
than the system OpenSSL (`reqwest` is pulled in with `default-features = false`
and the `rustls-tls` feature), so there is no `libssl-dev` step — but `rustls`
uses `ring` for its cryptography, and `ring` compiles a little C and assembly
at build time. Nothing else is required to build.

## Installing

```bash
cargo install mecha-cli mecha-mail --locked   # installs into ~/.cargo/bin
```

`mecha-cli` installs the binary named `mecha`, which is the name every command
in these docs uses. `mecha-mail` installs the mail, calendar and documents
servers (`mecha-mail`, `mecha-google`, `mecha-outlook`, `mecha-docs`); nothing
else needs them, so leave the crate off if you will never want mail or Google
Docs. `--locked` builds against the dependency versions the release was tested
with rather than resolving fresh ones.

The knowledge graph is a separate project with its own crates. You do not
install its server by hand: `mecha features enable graph` builds
`mecha-graph-mcp` from a pinned commit into `~/.mecha/sidecars/graph-mcp/` when
none is on your `PATH` (a few minutes; it needs cargo and a C compiler). The
graph's own command-line tool is optional:

```bash
cargo install mecha-graph --locked   # optional: the graph's CLI
```

See [Mail and calendar](/docs/features/tools/mail),
[Documents](/docs/features/tools/documents) and [Memory](/docs/features/memory)
for what each needs after it is installed.

Check what landed:

```bash
mecha --version
mecha tools            # runs without any provider configured
```

## Then run `mecha setup`

```bash
mecha setup
```

At a terminal, `mecha setup` is one guided pass: the chat model first, then
each optional feature with what it does and what it downloads, answered `y`,
`N` or `never`, then one summary of everything you said yes to and one
`Start?`, then the sign-ins those features need. Nothing is downloaded or
switched on before that `Start?`. [Setting up](/docs/getting-started/setting-up#3-mecha-setup)
walks through it.

**Every optional feature starts switched off.** Mail, documents, the graph,
web search, PDF extraction, image generation, voice, the web app and the rest
each have a switch in the `[features]` table of `~/.mecha/config.toml`, and a
switch that is absent reads as off. `mecha setup` writes the ones you say yes
to; `mecha features` lists them all with what each still needs, and
`mecha features enable <id>` switches one on later. A project's `mecha.toml`
cannot switch a feature on — only your global file can.

## On a Mac

mecha builds and runs on macOS, and installs its local chat model there as
it does on Linux. Most of the other parts that install or supervise programs
are Linux-only today. What that means on a Mac:

- **The local model installs.** `mecha setup chat` (or the first question
  of the guided `mecha setup`) installs llama.cpp's Metal build, the model
  recommended for your Mac's memory and the router that serves it, as a
  launchd agent (`~/Library/LaunchAgents/ai.mecha-factory.llama-local.plist`)
  that starts again at each login. Its log is
  `~/Library/Logs/mecha/llama-local.log`. A Mac's memory is one pool, so it
  gets the same recommendation as any unified-memory machine of its size.
- **The sandbox is Docker only.** `[sandbox] kind = "bwrap"` and `"landlock"`
  are Linux kernel features; on a Mac, `kind = "docker"` is the backend that
  confines `shell`.
- **The embeddings and OCR servers install too**, on demand: a launchd agent
  holds the port and starts the server at the first request, then stops it
  after ten idle minutes.
- **No other services.** The trigger runner, `mecha serve` as a service and
  the Slack connector are shipped as systemd units. On a Mac, run those
  commands yourself.
- **Image generation and voice install on Apple silicon.** `mecha features
  enable image` installs ComfyUI with PyPI's torch, drawing on the GPU through
  MPS with the bf16 model files (about 79 s a picture on an M5 Max; `--turbo`
  adds the 8-step turbo, about 16 s). `mecha features enable voice` builds the
  speech engine for Metal, which needs Homebrew's `cmake`. On an Intel Mac,
  each switches on for a server you start yourself.
- **The web app needs a git checkout and an npm build**, on any platform: the
  pages are a build artifact and do not come with `cargo install` (see
  [the web surface](/docs/features/interfaces/web#installing-the-app-itself)).

## Updating an existing install

Repeat `cargo install mecha-cli mecha-mail --locked` (leave off `mecha-mail` if
you never installed it), then check `mecha --version` and
`mecha <command> --help`. To install a specific release, add
`--version <VERSION>` using the version you intend to run.

:::note Upgrading to 0.1.23: switch on what you use
Since 0.1.23 every optional feature has a switch in `[features]`, and an
install from before then has none written, so each reads as off. Until you
switch them on, mail and calendar, Google Docs, the knowledge graph, web
search, PDF extraction, image generation, Slack, personas, voice and
`mecha serve` stop working: their tools are not registered, their servers are
not started and their commands refuse, each naming the command that fixes it.
Run `mecha setup` and answer each, or `mecha features enable <id>` for the
ones you want. A session's start also prints one line per feature this install
had set up whose switch is still unanswered.
:::

These docs track the repository's main branch and can describe work listed as
[Unreleased](/docs/changelog). If an option is absent from your binary's help,
check the release history or build from source. The browser assets are a separate
build; refresh them when upgrading a [web installation](/docs/features/interfaces/web#installing-the-app-itself).

## Building from source

The contributor path, and what you want if you are changing mecha rather than
running it:

```bash
git clone https://github.com/ljchang/mecha
cd mecha
cargo build --release
```

The binary is `./target/release/mecha`. To put a checkout's build on your
`PATH`:

```bash
cargo install --path mecha-cli    # installs `mecha` into ~/.cargo/bin
```

Or symlink the release build, which keeps `cargo build` as the way you update
it:

```bash
ln -s "$PWD/target/release/mecha" ~/.local/bin/mecha
```

The release profile uses thin LTO, so the first `--release` build takes a few
minutes. Iterating on the code is faster with a debug build (`cargo build`,
binary at `./target/debug/mecha`); use the release build for anything you
actually run against a model, because the debug build spends noticeable time in
JSON handling on large transcripts.

### The crates, and what each produces

The workspace has four members. `cargo build --release` builds all of them:

| Binary | Crate | What it is |
|---|---|---|
| `mecha` | `mecha-cli` | The agent CLI. |
| `mecha-mail` | `mecha-mail` | One MCP server over every configured account, whatever provider each uses. This is the one to wire up. |
| `mecha-google` | `mecha-mail` | Gmail and Google Calendar only, with its own credential store. |
| `mecha-outlook` | `mecha-mail` | Outlook mail and calendar over Microsoft Graph, its own credential store. |
| `mecha-docs` | `mecha-mail` | Google Docs, Sheets and Slides under `drive.file`, sharing the mail crate's OAuth and token handling. See [Documents](/docs/features/tools/documents). |
| — | `mecha-core` | The library every interface is built on. No binary of its own. |
| — | `mecha-slack` | The Slack transport: Socket Mode, the Web API, files both ways. No binary of its own — the connector is `mecha slack connect`, run as a systemd unit (`scripts/mecha-slack.service`). |

You do not need the mail binaries unless you want mail and calendar tools; see
[Mail and calendar](/docs/features/tools/mail) and [Slack](/docs/features/interfaces/slack).

## Verifying the build

```bash
cargo test                 # unit tests, including a scripted-provider loop test
cargo clippy --all-targets
```

The unit tests need no credentials and no network. A `ScriptedProvider` replays
a fixed list of turns, which is how loop behaviour — tool dispatch, denials,
budget exhaustion, error recovery — is tested without a model.

Integration tests under `mecha-core/tests/` do need real execution: docker
actually confining a command, an MCP server actually receiving an environment.
They skip when the backend is absent. In CI that is a hazard, because a silently
skipped test reads exactly like a passing one, so:

```bash
MECHA_TEST_REQUIRE_BACKENDS=1 cargo test    # every skip becomes a failure
```

## Optional dependencies

None of these are needed to run an agent. Each unlocks one subsystem.

### A sandbox backend, for confining `shell`

By default `shell` runs commands as you, unconfined — the only sane default for
a supervised CLI on a machine where the alternatives may not be installed.
Confinement is opt-in through `[sandbox] kind`, and needs one of:

```bash
sudo apt install bubblewrap      # kind = "bwrap"     (Linux)
# nothing to install             # kind = "landlock"  (Linux, kernel 6.2 or later)
# or use Docker                  # kind = "docker"    (the only one on a Mac)
```

`bwrap` uses unprivileged user namespaces and costs a few milliseconds per
command. Docker starts a throwaway container, which costs more, but works where
user namespaces are locked down.

On Ubuntu 23.10 and later, `bwrap` fails even when it is installed and
`kernel.unprivileged_userns_clone=1`, because AppArmor gained a separate switch
(`kernel.apparmor_restrict_unprivileged_userns=1`). Use `landlock` or `docker` there, or
install an AppArmor profile.

A configured sandbox that does not work is a startup failure, not a warning:
`Sandbox::preflight` runs a real command through the real backend and fails with
instructions. Silently falling back to unconfined execution would be worse than
having no sandbox at all, because `shell` declares narrower capabilities when
confined and the trifecta interlock believes it. See
[Sandbox](/docs/features/security/sandbox).

### Python 3, for regenerating eval fixtures

The eval case set reads fixture files under `eval/workspace/`. Those fixtures
are checked in, so running `mecha eval` needs no Python. Regenerating them does:

```bash
python3 scripts/build-eval-fixtures.py
```

It rewrites `eval/workspace/{audit,reports,kata}`, prints the gold answers the
cases must assert, and checks that each kata fails as shipped and is solvable by
a reference fix. The reason it is a generator rather than a directory of
hand-written files: a gold answer typed by hand is a guess, and one shipped in
this case set was wrong because a base rate got double-counted. A wrong gold
answer measures nothing — every model fails it, and the failure means nothing.

Python 3 is also what runs the fixture MCP servers used by
`eval/graph-cases.jsonl` (`eval/fixtures/graph_server.py`), a frozen fake of a
knowledge graph. The real one answers from live machine-local data, and a case
graded against that measures nothing repeatable. See
[Evaluation](/docs/features/experiments/evaluation).

### A local model

The default provider is a model on this machine: `mecha config init` (and
`mecha setup`, which seeds the same file) writes `default_provider = "local"`
and a `[providers.local]` table pointing at `http://127.0.0.1:8080`. Something
has to be serving there, and on Linux mecha installs it:

```bash
mecha setup chat
```

That installs llama.cpp from a pinned release if the machine has no
`llama-server` (building it where no release fits), fetches the model recommended for this machine's memory —
or takes a GGUF you already have — and installs the router that serves it on
`:8080` as a systemd user unit. It then reads the model's settings back off
the server into `[providers.local]`. The guided `mecha setup` asks the same
question first, so you rarely run it on its own. A server you already run on
`:8080` is left alone.

**Bringing your own server** — on a Mac, or with vLLM or Ollama — start it,
then let mecha read the settings off it rather than typing them:

```bash
llama-server -m model.gguf --mmproj mmproj-BF16.gguf --host 127.0.0.1 --port 8080 --jinja
mecha setup --write        # writes model, context_window and vision from /props
```

If the model is multimodal, **fetch its `mmproj-*.gguf` too**, from the same
Hugging Face repository as the weights. That second file is the vision tower,
it is not inside the weights, and without it the server runs happily and the
model simply says it cannot see images. [Images](/docs/features/interfaces/images)
is the whole story.

A hosted model needs no server: set `ANTHROPIC_API_KEY` and choose it in
`mecha setup`, or see [Setting up](/docs/getting-started/setting-up#anthropic).
Whatever a run sends then goes to Anthropic.

[Serving a local model](/docs/features/models/serving) covers slots, what `-c`
actually divides, and how to measure whether a restart made things slower.

### Everything else

Search backends, MCP servers, and mail accounts are configured rather than
installed. They are covered in [Tools and MCP](/docs/features/tools) and
the [configuration reference](/docs/reference/configuration).

## Next

- [Choosing hardware](/docs/getting-started/hardware) — if you are still
  deciding what to run the model on, or what will fit in the memory you have.
- [Setting up](/docs/getting-started/setting-up) — the guided `mecha setup`
  in full: the chat model, the features, and the context you want it to have.
