# Contributing to mecha

Thanks for your interest. This document covers what you need to build the
project, what the review will look for, and the handful of invariants that are
easy to break by accident.

If you are an AI coding agent working in this repository, read
[`CLAUDE.md`](CLAUDE.md) first. It is longer than this file and records *why*
each subsystem is shaped the way it is, which is what you need in order to
change one safely.

## Build and test

```bash
cargo build --release          # ./target/release/mecha
cargo test --workspace         # unit + integration tests
cargo clippy --all-targets --all-features
cargo fmt --all
```

`MECHA_LOG=debug` turns on internal tracing, which goes to stderr.

`mecha tools` runs with no provider configured, which makes it the cheapest
end-to-end check that the binary starts and the tool registry builds.

CI runs the same commands, plus a job that installs `bubblewrap` and sets
`MECHA_TEST_REQUIRE_BACKENDS=1`. That variable turns every "backend missing, so
skip" into a failure, because in CI a silently skipped test reads exactly like
a passing one.

## The documentation site

The site lives in `website/` and is a Docusaurus project.

```bash
cd website
npm ci
npm start        # local preview at http://localhost:3000/mecha/
npm run build    # what CI builds; fails on broken links
```

`CHANGELOG.md` at the repository root is the single source of truth for release
history. `website/docs/changelog.md` is generated from it before every build and
is gitignored — edit the root file.

## Which document to write in

[`docs/README.md`](docs/README.md) maps every document to its job and gives a
decision rule for where a given piece of writing belongs. It is worth two
minutes before you add to any of them.

Two conventions carry most of the weight. `docs/HANDOFF.md` is bounded by
scope rather than by length — it holds current state and open work only, and
every item in it has been verified unbuilt against source. Completed work
moves to `docs/HISTORY.md` rather than being struck through, which is how that
file stayed readable after growing to 1,965 lines once already: what made it
unreadable was finished work nobody removed, not the line count.

The `handoff` skill (`.claude/skills/handoff/`) is the procedure for that move.
Run it at the end of a session that changed behaviour.

## Invariants

These are enforced structurally rather than by convention, and a change that
erodes one is the most expensive kind of defect this project can ship. Each cost
something to learn; `CLAUDE.md` records the incident behind most of them.

**Every model-supplied path goes through `ToolCtx::resolve`.** It canonicalizes
and proves containment in the workspace. Never call `fs::*` on a raw path from
tool input.

**Taint belongs to the conversation, not the run.** It lives on
`agent::Conversation` next to the messages, and the session file records it so
that resuming does not launder it. A turn boundary is not a security boundary.

**Any tool that reaches the network must call `.from_outside()`.** Taint keys off
`ToolOutput::external` — what this particular result actually was — rather than
off the capability declaring what the tool *could* return.

**The dispatch order is interlock, then hook, then approver.** A hook may narrow
policy and never loosen it, and a `pre_tool` denial never reaches the human:
mechanical policy is cheaper than an interruption, and a hook cannot be talked
into clicking yes.

**Things that fail closed must keep failing closed.** `pre_tool` hooks, sandbox
preflight, outbox staging and provenance classification all deny on error,
timeout, or ambiguity. A policy layer that cannot run and quietly allows is worse
than no policy layer, because everything downstream still believes it ran.

**The loop must not know which provider is behind it, or where a tool came
from.** Both are trait objects. Matching on a provider name inside `agent.rs`
means the abstraction is leaking.

**A tool result must exist for every `tool_use` id**, or the next request is a
400 and the run is over.

**A new field on `Config` is two edits, not one.** Files parse into
`ConfigLayer`, where every field is optional so a project file can override a
single setting. A field added to `Config` alone makes its whole TOML table a
parse error that kills startup — while every unit test stays green, because
tests build the types directly. `every_field_of_config_is_reachable_from_a_file`
exists to catch this.

**Tool order in the registry is stable** (`BTreeMap`) because the tool list sits
at the front of the cached prompt prefix. Reordering it invalidates the cache
every turn.

## Testing

Three layers, and the split is deliberate:

- **Unit tests** for anything that is a function of our own code: the request
  body, the stream decoder, session round-trips, the compaction cut. Free,
  deterministic, and they never expire. Note the limit — `ScriptedProvider`
  replays what we *believe* providers do, so it is structurally blind to a
  provider violating that belief, which is where this project's expensive bugs
  came from.
- **Integration tests** (`mecha-core/tests/`) for what is deterministic but needs
  real execution: docker actually confining a command, an MCP server actually
  receiving an environment.
- **Eval cases** for what only emerges with a real model in the loop. Expensive
  and non-deterministic; use them where the other two cannot reach.

Verify a fix by making it **fail on the old behaviour**. Where the assertion is
about the environment rather than about scripted state, check that the negative
is not vacuous — the confinement tests only mean something on a machine that
does have `~/.ssh` and can reach the network.

## Pull requests

- Branch off `main`. Keep a pull request to one concern.
- **Parallel work never shares a working tree** — including two AI sessions.
  Use a git worktree per effort (`git worktree add ../mecha-<arc> -b
  <branch>`). The rule traces to a real afternoon in the sibling repository:
  two sessions in one tree, one stashed the other's changes to verify its own
  commit, and the only copy of code already running in production sat in a
  stash.
- Run `cargo fmt --all`, `cargo clippy --all-targets`, and `cargo test
  --workspace` before pushing. CI treats warnings as errors.
- Every commit builds and passes tests alone — history is bisectable and
  stays that way.
- Explain *why* in the description. The code shows what changed; the reasoning
  is the part reviewers cannot reconstruct.
- If you changed behaviour a user would notice, add an entry to the
  `## [Unreleased]` section of `CHANGELOG.md`.
- Claude reviews pull requests automatically. Its comments are advisory — a
  maintainer merges.
- Merge by rebase or fast-forward — linear history, no merge commits. Squash
  only when a branch's intermediate commits are not worth keeping.

Commit messages should say what changed and why in the subject line. The history
here favours a declarative sentence over a conventional-commits prefix.

## Versioning

The project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the major version is `0`, the minor version carries breaking changes.

`mecha-core` is a library, so its public API is part of the contract. Changing a
trait that an external implementor could depend on — `Provider`, `Tool`,
`Approver` — is a breaking change even when nothing inside this repository
notices.

### The minimum supported Rust version

`rust-version` in the workspace manifest is the oldest Rust that builds the
tree, and CI pins an arm to exactly that number so breaking it is a failure
rather than a discovery. Keep the two in step; they are one promise written
twice.

**It moves when a dependency we want needs it to, and not otherwise.** Never
to use a new language or standard-library feature ourselves — that trades
somebody's ability to install mecha for a convenience, which is the wrong way
round. Raise it in the same pull request as the dependency that forced it,
and name that dependency in the message, so the number always has a reason
attached to it.

Two things follow from that rule, and they are why it is written down. Raise
it to the **minimum** the dependency needs, not to current stable: the
difference is headroom for anyone on a toolchain their distribution chose.
And expect the rule to get harder to apply over time rather than easier —
today the libraries here have no reverse dependencies on crates.io, so the
number costs nobody anything; the first crate that depends on `mecha-core` is
the moment it starts to.

*Raised to 1.89 on 2026-08-10 for `rustyline` 18, which uses `file_lock`.*

## Releases

A release is a **tag on main**, nothing else:

1. Move the `## [Unreleased]` entries in `CHANGELOG.md` under the new version,
   and bump `version` in the workspace `Cargo.toml` (one PR; every crate
   inherits it).
2. Tag the merge commit `vX.Y.Z` and push the tag.
3. The `release` workflow re-runs the full test suite (a tag can be pushed
   from anywhere; "it was green when I looked" is not a gate), refuses a tag
   that does not match the workspace version, and publishes the crates to
   crates.io in dependency order: `mecha-core` → `mecha-mail` → `mecha-cli`.

The same workflow shape lives in the
[`mecha-factory`](https://github.com/ljchang/mecha-factory) repository, which
adds the production-box binary and deploy; its `CONTRIBUTING.md` is the
canonical description of the shared workflow.

Publishing uses crates.io **Trusted Publishing** (GitHub OIDC), so no
long-lived registry token exists anywhere. One-time setup per crate: the
first version is published by hand by an owner (`cargo login && cargo publish
-p <crate>` in dependency order — Trusted Publishing can only be configured
on a crate that already exists), then crates.io → Settings → Trusted
Publishing adds this repository and `release.yml`. A published version is
forever: yankable, never deletable or reusable.

## Security

Do not open a public issue for a vulnerability. See [`SECURITY.md`](SECURITY.md).

## License

By contributing, you agree that your contributions are licensed under the MIT
License, the same terms that cover the project.
