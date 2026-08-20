---
title: The work directory
sidebar_position: 9
description: Where a run's generated output goes, why it is also the run's path jail, and how retention keeps it from becoming a pile.
---

# The work directory

`~/.mecha/work/<producer>/` is where a run's generated output goes, and it is
**also the run's workspace** — the directory the path jail is rooted at.

A *producer* is whatever made the output: a trigger's name, or `chat`, or a
session id. The directory is **stable across runs of the same producer**, which
is most of the point. Yesterday's briefing is an ordinary file in today's run,
readable with `fs_read` like anything else, rather than something that has to
be fetched back from somewhere outside the jail.

```bash
mecha work                       # what each producer has generated
mecha work path briefing         # print one producer's directory, creating it
mecha work clean                 # keep the newest N per producer
mecha work clean --dry-run       # say what would go, remove nothing
mecha work clean --producer briefing --keep 3
```

## Two directories that mean opposite things

```text
~/.mecha/work/<producer>/       generated · mutable · disposable · cleanable
~/.mecha/bundles/<id>/<ver>/    published · immutable · versioned · never deleted
```

Work is scratch that a retention policy sweeps. A [published
bundle](/docs/features/publishing) is a durable, versioned URL that nothing
here ever removes. Keeping them apart is what lets the sweep be aggressive.

## One change, four things closed

The work directory is small, and it exists because a single change closed four
separate problems — which is usually the sign that the shape is right.

| It fixes | How |
|---|---|
| **The jail default** | An unattended run's workspace now holds nothing sensitive. |
| **Cross-run read-back** | The directory is stable, so yesterday's output is today's input. |
| **A durable artifact** | An unattended run has somewhere to leave something you can open later. |
| **`notify`** | It has a designated place to write instead of inventing one. |

The jail one is the load-bearing fix. A trigger with no explicit workspace used
to fall through to `std::env::current_dir()`, and the shipped systemd unit sets
`WorkingDirectory=%h`. So an unattended run holding filesystem tools was jailed
to `$HOME` — which *contains* `~/.mecha/`: the mail OAuth tokens, every session
transcript, the learning store. The shipped `morning` trigger escaped only by
accident of its `mail__*` allowlist.

:::note[Note the direction of the check]
A workspace *inside* the mecha home is fine, and is now the default. What
`setup` refuses is a workspace the mecha home sits **under** — which is what
`mecha chat` in `$HOME` was doing. See [the security
model](/docs/features/security#a-jail-has-to-be-rooted-somewhere-harmless).
:::

And `notify` used to end with `mkdir -p ~/.mecha/briefings && cat > …`: a shell
redirect into a directory it created on the way past, outside every path jail,
so nothing could ever read it back. That existed only because there was no
designated place to write.

## Where a trigger's workspace comes from

`mecha trigger add` **writes the workspace down** rather than leaving it
implicit, and the runner resolves the same default when the field is unset — so
a trigger authored before this behaviour existed is fixed by upgrading, not by
remembering to edit it. `mecha trigger show` prints the resolved default too:
"where is this jailed" must never be answered by an omitted line.

```bash
mecha trigger show briefing
# ...
# workspace   ~/.mecha/work/briefing   (default)
```

## Retention is a policy, not an intention

Anything without one becomes a pile nobody opens.

```toml
[work]
keep = 10          # entries per producer that survive `mecha work clean`
```

`mecha work clean` keeps the newest `keep` entries per producer and says
exactly what it removed. The nightly maintenance run calls it. Three rules:

- **Entries are counted, not files.** A rendered bundle is a directory, and it
  counts as one entry.
- **The producer directory itself is never removed.** An empty directory is a
  directory, not an absence, and deleting it would only make tomorrow's run
  recreate it.
- **It never removes anything a published bundle names as a source.**
  "Regenerate last week's report" must not silently lose its input. Entries
  that survive for this reason are *reported*, not silently skipped — an
  unexplained survivor reads as a bug in the sweep.

The default of 10 holds about a week and a half of a daily producer, so both
"what did yesterday's run say" and "what changed since Monday" are still on
disk. It is a placeholder in the honest sense: it wants a week of real output
to tune, and `[work] keep` is where that tuning goes.

### How a bundle protects its sources

The contract is **one field of data**, not a shared type. A mirrored version
directory may carry a `bundle.json` with a `sources` array:

```json
{
  "sources": ["/home/you/.mecha/work/briefing/2026-08-05"]
}
```

Anything else in that file is the publisher's business. A mirror that does not
exist yet — which is every install until `mecha-factory-publish` is wired —
protects nothing, and that is correct rather than a stub.

## Naming

A producer name has to be a single safe path segment. `mecha work path` and
`trigger add` both validate it, so a trigger called `../../etc` is an error at
creation rather than a traversal at run time.

`$MECHA_HOME` relocates the whole tree. It exists for tests and for running two
mechas side by side; nothing in a normal install sets it.

## Where to go next

- [Triggers](/docs/features/triggers) — the producer that most needs a durable place to write.
- [Publishing](/docs/features/publishing) — what turns a work directory into a URL.
- [Security model](/docs/features/security) — the path jail this is rooted at.
