---
title: Installation
sidebar_position: 1
description: Install mecha from crates.io with cargo, build it from source, and add the optional dependencies for the sandbox and the eval fixtures.
---

# Installation

mecha is published on crates.io, so installing it is one `cargo install`. There
are no prebuilt binaries and no package in any distribution — `cargo` compiles
it on your machine either way; the difference is only whether the source comes
from the registry or from a checkout.

## Requirements

**Rust 1.89 or newer.** The workspace pins `rust-version = "1.89"` on edition
2021, so an older toolchain fails at build time with a clear message rather than
part-way through compiling a dependency. The pin is raised only to the minimum
a dependency actually needs, never to current stable — the difference is
headroom for anyone on the toolchain their distribution chose.

```bash
rustc --version        # must be 1.89.0 or later
rustup update stable   # if it is not
```

Nothing else is required to build. TLS comes from `rustls` rather than the
system OpenSSL (`reqwest` is pulled in with `default-features = false` and the
`rustls-tls` feature), so there is no `libssl-dev` step and no vendored C
build.

## Installing

```bash
cargo install mecha-cli --locked      # installs `mecha` into ~/.cargo/bin
```

The crate is `mecha-cli`; the binary it installs is named `mecha`, and that is
the name every command in these docs uses. `--locked` builds against the
dependency versions the release was tested with rather than resolving fresh
ones.

Mail and calendar tools come from a second crate, and are optional — nothing
else needs it:

```bash
cargo install mecha-mail --locked     # mecha-mail, mecha-google, mecha-outlook
```

See [Mail and calendar](/docs/features/mail) for what to do with them.

Check what landed:

```bash
mecha --version
mecha tools            # runs without any provider configured
```

:::warning[A fresh publish can install as a stale version]
`cargo install` resolves against a cached registry index, so for a few minutes
after a release it can quietly pick up the *previous* version — no warning, no
error, just an older binary than the one you asked for. `cargo install mecha-cli
--locked --version 0.1.2` pins it, and `mecha --version` is what confirms it.
:::

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
| — | `mecha-core` | The library every interface is built on. No binary of its own. |
| — | `mecha-slack` | The Slack transport: Socket Mode, the Web API, files both ways. No binary of its own — the connector is `mecha slack connect`, run as a systemd unit (`scripts/mecha-slack.service`). |

You do not need the mail binaries unless you want mail and calendar tools; see
[Mail and calendar](/docs/features/mail) and [Slack](/docs/features/slack).

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
sudo apt install bubblewrap      # kind = "bwrap"
# or use Docker                  # kind = "docker"
```

`bwrap` uses unprivileged user namespaces and costs a few milliseconds per
command. Docker starts a throwaway container, which costs more, but works where
user namespaces are locked down.

On Ubuntu 23.10 and later, `bwrap` fails even when it is installed and
`kernel.unprivileged_userns_clone=1`, because AppArmor gained a separate switch
(`kernel.apparmor_restrict_unprivileged_userns=1`). Use `docker` there, or
install an AppArmor profile.

A configured sandbox that does not work is a startup failure, not a warning:
`Sandbox::preflight` runs a real command through the real backend and fails with
instructions. Silently falling back to unconfined execution would be worse than
having no sandbox at all, because `shell` declares narrower capabilities when
confined and the trifecta interlock believes it. See
[Sandbox](/docs/features/sandbox).

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
[Evaluation](/docs/features/evaluation).

### A local model, if you want one

mecha talks to any OpenAI-compatible endpoint, so this is optional — an
Anthropic key is enough to start. If you want the model on your own machine,
there are two pieces and the second is the one people miss.

**The server.** [`llama.cpp`](https://github.com/ggml-org/llama.cpp) is what
these docs assume; vLLM and Ollama also work. Build it or take a release
binary, and check `llama-server --version` runs.

**The weights, and the projector.** Model files are GGUFs from Hugging Face.
Fetch one, and — if the model is multimodal — **fetch its `mmproj-*.gguf`
too**, from the same repository:

```bash
REPO=unsloth/Qwen3.6-35B-A3B-MTP-GGUF        # whatever you chose
curl -L -O "https://huggingface.co/$REPO/resolve/main/mmproj-BF16.gguf"
```

That second file is the vision tower, it is not inside the weights, and
without it the server runs happily and the model simply says it cannot see
images. [Images](/docs/features/images) is the whole story; if you only
remember one thing, remember that a multimodal model is two files.

Then start it and let mecha read the settings off it rather than typing them:

```bash
llama-server -m model.gguf --mmproj mmproj-BF16.gguf --host 127.0.0.1 --port 8080 --jinja
mecha setup --write        # writes model, context_window and vision from /props
```

[Serving a local model](/docs/features/serving) covers slots, what `-c`
actually divides, and how to measure whether a restart made things slower.

### Everything else

Search backends, MCP servers, and mail accounts are configured rather than
installed. They are covered in [Tools and MCP](/docs/features/tools-and-mcp) and
the [configuration reference](/docs/reference/configuration).

## Next

[First run](/docs/getting-started/first-run) — point mecha at a provider and get
an answer out of it.
