# AGENTS.md

Instructions for AI coding agents working in this repository.

**Read [`CLAUDE.md`](CLAUDE.md) first, in full.** It is the canonical guide and
this file deliberately does not duplicate it. `CLAUDE.md` records *why* each
subsystem is shaped the way it is — the incident behind each invariant, and the
bug that reappears when one is undone. That reasoning is what you need in order
to change something safely, and it is not recoverable from the code alone.

[`CONTRIBUTING.md`](CONTRIBUTING.md) covers build commands, the testing layers,
and pull request expectations.

## Orientation

```
mecha-core/     the library; knows nothing about any CLI or application
mecha-cli/      the `mecha` binary; thin, all logic belongs in core
mecha-mail/     library plus three MCP binaries for mail and calendar
eval/           the case set and its fixtures
website/        the Docusaurus documentation site
docs/           design and research notes, not user documentation
results/        recorded scorecards; the baselines comparisons are made against
```

## Before you start

```bash
cargo build --workspace --all-targets
cargo test --workspace
```

Both should pass on a clean checkout. If they do not, fix that before making
changes — you cannot tell your breakage from pre-existing breakage otherwise.

## Before you finish

```bash
cargo fmt --all
cargo clippy --all-targets --all-features
cargo test --workspace
```

CI treats warnings as errors, so a clippy warning will fail the build.

## The traps

Each of these is a real bug this project has already shipped once. They share a
shape: the code compiles, every test stays green, and the failure appears
somewhere else entirely.

**Adding a field to `Config` without adding it to `ConfigLayer`.** The TOML
table becomes a parse error that kills startup, while every unit test passes
because tests build the types directly. This is exactly how hooks shipped
unreachable.

**Calling `fs::*` on a path from tool input.** It must go through
`ToolCtx::resolve` first. That function is the path jail.

**Resetting taint at a turn boundary.** Taint lives on `agent::Conversation`
because a turn boundary is not a security boundary. Keep the history and you
keep the taint.

**Marking a tool's output as external when the harness generated it.** Taint
keys off `ToolOutput::external`, not off the capability. A refusal produced by
our own guard, labelled as third-party content, makes the model invent
explanations for its own harness.

**Making something that failed closed fail open.** `pre_tool` hooks, sandbox
preflight, outbox staging and provenance classification all deny on error,
timeout, or ambiguity. A layer that cannot run and quietly allows is worse than
no layer at all, because everything downstream still believes it ran.

**Matching on a provider name inside `agent.rs`.** The loop must not know which
provider is behind it, or where a tool came from. Both are trait objects.

**Reordering the tool registry.** It is a `BTreeMap` because the tool list is
the front of the cached prompt prefix; reordering invalidates the cache every
turn.

## Testing guidance

Verify a fix by making it **fail on the old behaviour**. A test that passes both
before and after your change has measured nothing.

`ScriptedProvider` in `agent.rs` replays a fixed list of turns, so you can test
loop behaviour without network access. Know its limit: it replays what we
*believe* providers do, which makes it structurally blind to a provider
violating that belief. That blindness is where this project's expensive bugs
came from.

Integration tests skip themselves when a sandbox backend is missing. Set
`MECHA_TEST_REQUIRE_BACKENDS=1` to turn every skip into a failure — a silently
skipped test reads exactly like a passing one.

## Documentation

User-facing behaviour changes belong in `website/docs/`. Release-note-worthy
changes go in the `## [Unreleased]` section of `CHANGELOG.md` at the repository
root — not in `website/docs/changelog.md`, which is generated from it at build
time and gitignored.

If you learn something that would have saved you an hour, add it to `CLAUDE.md`.
That file is the reason the next agent will be faster than you were.
