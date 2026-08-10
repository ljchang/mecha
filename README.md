<!-- The mark is accent-400, a dark-ground colour, and the wordmark is near-white.
     GitHub honours prefers-color-scheme in a <picture>, so the light theme gets
     the accent-700 twin instead of two invisible words. -->
<p align="left">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/logo-lockup.svg">
    <img src="brand/logo-lockup-light.svg" alt="mecha — agent harness · Rust" height="54">
  </picture>
</p>

# mecha

**A harness that turns a local open-weight model into a personal assistant with
your context, your permissions, and a safe way to reach the world.**

[Documentation](https://docs.mecha-factory.ai/) ·
[Design principles](https://docs.mecha-factory.ai/docs/principles) ·
[Security model](https://docs.mecha-factory.ai/docs/features/security)

---

In mecha anime the pilot is an ordinary person. What makes them formidable is
the suit: it gives them reach, senses, armour, and a way to act on the world.
The suit does not think for the pilot, and it is answerable to the person inside
it.

mecha is that suit for a language model — and the model it is built for is one
running on **hardware you own, on data that never leaves it**. Such a model is
entirely capable of being an excellent personal assistant, and is nowhere near
being one out of the box. It has no memory of you, cannot see your mail or your
calendar, can produce text and nothing else, and has no defence against the
first web page that tells it to forward your inbox to a stranger.

Everything here closes one of those gaps without opening a fifth.

## What it is for

Academic work carries a long tail of tasks that are tedious rather than hard:
the fourth meeting request this week, deciding whether you can take on a review,
the letter you promised in March, finding the slot that works for five people,
the form somebody needs back by Friday.

What stops a model from absorbing that work is not intelligence — it is that
every one of those tasks needs **your** context. Who this person is, what you
already promised them, what is actually on your calendar, how you write when you
say no. A model with no context produces something generic that you then have to
rewrite, which is slower than doing it yourself.

So the assistant that would help is one that can see a great deal about you.
That is also the one it is most dangerous to build, which is why the security
model is the centre of the design rather than a layer on top of it.

## The lethal trifecta

An agent holding **private data**, **untrusted content**, and **a way to send**
can be turned against you by instructions hidden in the content it reads. A
personal assistant has all three by definition: reading your mail is the job,
the mail was written by other people, and answering it is the point.

You cannot design the trifecta out of the work. You can only decide what happens
when all three are present. Most harnesses ask the model to be careful, which
does not work — the injected instruction arrives through the same channel as the
legitimate data. mecha makes it structural:

```rust
fn capabilities(&self) -> Capabilities {
    Capabilities::default().untrusted().sends()   // http_fetch
}
```

Every tool declares what it can do, the **conversation** tracks what has entered
it, and an outbound call is refused once both private data and third-party
content are present. Taint lives on the conversation rather than the run, so a
new turn does not launder it. The refusal happens *before* the human is asked,
because a person clicking "yes" is what an injection is trying to engineer.

## Install

```bash
cargo install mecha-cli --locked          # installs the `mecha` binary
cargo install mecha-mail --locked         # optional: mail + calendar MCP servers
```

Or from source:

```bash
cargo build --release                     # ./target/release/mecha
```

## Quick start

```bash
export ANTHROPIC_API_KEY=sk-ant-...       # or point at a local server
mecha config init                         # writes ~/.mecha/config.toml

mecha run "summarise what changed in this repo today"
mecha chat                                # interactive
mecha tui                                 # full screen; steer a run in flight
mecha tools                               # what the agent can do (no credentials needed)
```

By default the agent may read anything in the workspace but asks before it
writes or runs a command. `--yes` approves everything; `--read-only` refuses
everything that is not a read.

## What is here

| Crate | What it is |
|---|---|
| [`mecha-core`](mecha-core/) | The library: the loop, tools, MCP client, sessions, security. Knows nothing about any CLI or application. |
| [`mecha-cli`](mecha-cli/) | The `mecha` binary. Deliberately thin — front-end concerns only. |
| [`mecha-mail`](mecha-mail/) | Gmail, Outlook and their calendars behind one provider-neutral surface, as stdio MCP servers. The model names an *account*, never a provider. |
| [`mecha-slack`](mecha-slack/) | A Slack client — Socket Mode, the Web API, files both ways. Has no `mecha-core` dependency and must never gain one. |

Alongside it, [**mecha-factory**](https://github.com/ljchang/mecha-factory) is
the public surface in both directions: what the agent makes becomes a durable,
versioned, permissioned URL, and what other people need from you arrives as a
**typed request** rather than free-form prose — so a stranger's words never reach
a privileged run. See [the factory
docs](https://docs.mecha-factory.ai/docs/factory/overview).

Personal context is wired in over MCP, which is what keeps it open-ended:
mail and calendar through `mecha-mail`, a personalized knowledge graph for who
people are and what happened when, and anything else you can expose as a
server. Connecting a new source is configuration, not a code change.

## Commands

| Command | What it does |
|---|---|
| `mecha run "<task>"` | One task, one answer. `--json` for machine-readable output, `--resume <id>` to continue. |
| `mecha chat` | Terminal REPL with history and slash commands. |
| `mecha tui` | Full-screen. The input line stays live, so you can steer a run in flight. |
| `mecha batch items.jsonl` | Same agent over many prompts, bounded concurrency, JSONL results. |
| `mecha eval [cases.jsonl]` | Score a model on a case set, graded on the tool-call trace. |
| `mecha replay <session>` | Re-drive a recorded session against today's code, reporting where it diverged. |
| `mecha tools` | List the tool surface. `--schema` shows exactly what the model sees. |
| `mecha sessions` | Inspect saved transcripts: `list` / `show` / `path` / `stats`. |
| `mecha config` | See what settings are in effect: `show` / `path` / `init`. |
| **Review and release** | |
| `mecha outbox` | Review staged sends: `list` / `show` / `edit` / `review` / `send` / `reject`. Tools named in `[outbox]` stage drafts instead of executing. |
| `mecha frontdoor` | Requests from strangers: `list` / `show` / `extract` / `next` / `triage` / `needs-info` / `close`. |
| **Unattended** | |
| `mecha trigger` | Prompts on a cron schedule: `add` / `list` / `show` / `edit` / `rm` / `enable` / `disable` / `next` / `run` / `cancel` / `runs` / `tick` / `daemon`. |
| `mecha work` | The per-producer output directories a run is jailed to: `list` / `path` / `clean`. |
| `mecha slack` | Slack as a remote control: `status` / `auth` / `link` / `threads` / `connect` / `sweep` / `notify` / `unlink`. |
| `mecha msg` | Messages between agent sessions on this machine: `send` / `list` / `show` / `dismiss` / `agents`. |
| **Memory** | |
| `mecha reflect` | Mine transcripts for the moments you stepped in. |
| `mecha learn` | Turn those reflections into rules. `--propose` stages them instead of applying. |
| `mecha validate` | Measure whether the rules actually changed an answer. |
| `mecha proposals` | Review gated rule changes: `list` / `show` / `accept` / `reject`. |
| `mecha rules` | Rule tenure: ledger tallies, `retire` / `restore`, `propose-retirements`. |
| `mecha distill` | Summarise closed sessions into episodes staged to the knowledge graph. |

Exit codes for `run`: `0` success (including a run stopped by a budget that
still produced an answer — `--json`'s `stop_cause` says which), `1` error, `2`
the model refused, `3` it produced no answer at all.

## Configuration

Layered, each level overriding only the fields it names: built-in defaults →
`~/.mecha/config.toml` → `./mecha.toml` → `MECHA_PROVIDER` / `MECHA_MODEL` /
`MECHA_EFFORT` → CLI flags.

```toml
default_provider = "local"

[providers.local]                     # llama-server, vLLM, Ollama
kind = "local"
base_url = "http://127.0.0.1:8080"
model = "qwen3-moe"
context_window = 32768                # whatever the server's `-c` was

[providers.anthropic]
kind = "anthropic"
model = "claude-opus-5"
api_key_env = "ANTHROPIC_API_KEY"

[agent]
max_turns = 40
effort = "high"                       # low | medium | high | xhigh | max
timezone = "America/New_York"         # IANA name; the model has no clock

[tools]
permission_mode = "ask"               # ask | allow | read-only

[sandbox]                             # how `shell` is confined
kind = "none"                         # none | bwrap | docker
network = false                       # no network means `shell` cannot exfiltrate

[outbox]                              # these tools stage drafts instead of running
tools = ["mail__mail_send"]

[[mcp]]
name = "pkg"
command = "pkg-mcp"

[[hook]]
event = "pre_tool"                    # pre_tool | post_tool | session_end
tools = ["shell"]                     # empty means every tool
command = "~/.mecha/hooks/no-force-push.sh"
```

`context_window` is worth setting even though nothing requires it. No provider
reports how much context is *left* — only what a prompt cost — so the derived
compaction threshold, the per-turn tool-output budget and the TUI's gauge all
fall back to nothing without it. A stale value is worse than none.

Every key is documented in the [configuration
reference](https://docs.mecha-factory.ai/docs/reference/configuration).

## What makes mecha different

- **Built for local open-weight models first**, not as a fallback when the API
  budget runs out. That changes what the engineering is about: the binding
  constraint on a small model in a loop is **tool-call reliability**, not
  intelligence, so `mecha eval` grades the tool-call trace before the prose. A
  model that is 5% smarter but malforms arguments one call in twenty is worse in
  a loop, because every bad call costs a recovery turn.
- **Security is structural, not prompted** — the interlock and the path jail live
  in the type system and the loop. A configured sandbox that cannot confine
  anything **stops the run** rather than silently running unconfined.
- **Sending is staged and reviewed.** Naming a tool in `[outbox]` makes
  "draft-only, never send" a property of the harness, covering third-party MCP
  tools that have never heard of it. The useful configuration and the safe one
  turn out to be the same: an overnight run that drafts nine replies needs no
  write permission, because staging executes nothing.
- **It expects to run unattended.** A missed week owes one briefing rather than
  seven; each scheduled run is jailed to its own work directory; a trigger
  deliberately cannot read a repository's `mecha.toml`, because a cloned repo
  must not be able to shape a job on your machine.
- **What it learns has to keep earning its place.** Rules mined from your
  corrections are gated on **provenance** (a lesson from a conversation that read
  untrusted content is excluded structurally) and on **measurement** (a
  validation ledger records whether each rule changed an answer; one that
  accumulates attributed regressions is proposed for retirement).
- **Everything a model says about its own work is hearsay.** Eval cases can end
  in a `verify` command whose exit status is the grade, runs replay against
  today's code, and repeated runs report **pass^k** beside pass@k.

## Documentation

Full documentation is at **[docs.mecha-factory.ai](https://docs.mecha-factory.ai/)**:

- [What mecha is](https://docs.mecha-factory.ai/docs/intro) and
  [design principles](https://docs.mecha-factory.ai/docs/principles)
- [Getting started](https://docs.mecha-factory.ai/docs/getting-started/installation)
- [Features](https://docs.mecha-factory.ai/docs/category/features) — security,
  sandbox, tools and MCP, outbox, triggers, learning, compaction, evaluation,
  mail, Slack
- [The factory](https://docs.mecha-factory.ai/docs/category/factory) — publishing,
  the front door, polls, notebooks
- [CLI](https://docs.mecha-factory.ai/docs/reference/cli) and
  [configuration](https://docs.mecha-factory.ai/docs/reference/configuration) reference

In this repository, [`CLAUDE.md`](CLAUDE.md) is the canonical design document —
why each subsystem is shaped the way it is, and the incident behind each
invariant. [`docs/README.md`](docs/README.md) maps which document holds what.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Briefly: one branch per arc, one git
worktree per session, and `cargo fmt --all && cargo clippy --all-targets &&
cargo test --workspace` before any push.

## License

MIT. See [`LICENSE`](LICENSE).
