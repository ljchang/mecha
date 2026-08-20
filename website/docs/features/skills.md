---
title: Skills
sidebar_position: 14
description: Procedures you write as files, that the agent loads when it decides one is relevant — and why it can never install one itself.
---

# Skills

A skill is a procedure **you** write, sitting in a directory, that the model
loads when it decides the procedure is relevant.

```
~/.mecha/skills/rec-letter/SKILL.md      the procedure
~/.mecha/skills/rec-letter/TEMPLATE.md   a file it can point at
```

```markdown
---
name: rec-letter
description: How to handle a request for a recommendation letter. Use whenever
  someone asks for a letter, a reference, or an endorsement.
triggers: [letter, reference, recommendation]
---

# Answering a letter request

1. Check the deadline against the calendar before agreeing to anything.
2. Ask for a CV and the submission link if either is missing.
3. Draft from `TEMPLATE.md`, never from scratch.
```

```bash
mecha skills                 # what a run would carry
mecha skills --show          # …with the full bodies
mecha run --no-skills "…"    # carry none this time
```

## Progressive disclosure is the whole point

| Level | Loaded | Cost |
|---|---|---|
| 1 · metadata | always, in the system prompt | ~100 tokens per skill |
| 2 · the body | when the model calls `skill` | the body, once |
| 3 · bundled files | when the procedure points at one | nothing until read |

So a store full of skills costs almost nothing until one is relevant. That is
what makes this the pressure valve for the
[learned-rule cap](/docs/features/learning): rules are capped because the
always-on prefix is finite, and *how to answer a rec-letter request* is too long
for a rule, too specific to be worth a slot, and irrelevant on 95% of runs —
which is precisely the profile progressive disclosure was built for. **Skills do
not loosen that cap. They make it affordable.**

## The format is not ours, on purpose

`SKILL.md` is the Agent Skills standard: YAML frontmatter, then markdown.
`name` and `description` are required; `triggers` and `tools` are mecha's
optional extras.

Taking a standard rather than inventing a dialect is a portability decision with
evidence behind it — the two `SKILL.md` files this repository already carried,
written for a different harness entirely, load unmodified. Unknown frontmatter
keys are therefore **ignored**, so a skill carrying a field some other tool
understands still works here. What *is* refused is a key mecha knows and cannot
use — a `description` that is a list, an empty `tools` — because that is an
authoring mistake rather than a portability one, and a field silently dropped is
a procedure with a step missing.

`description` carries the whole discovery burden, since it is all the model sees
until it loads anything. Say **what it does and when to use it**, not just what
it is. (The same sentence [subagent](/docs/features/tools-and-mcp) profiles
already carry — two independent designs reaching the same instruction is a good
sign it is load-bearing.)

## You write them. Nothing else can.

**There is no `mecha skill install`, no registry client, no remote body, and no
way for a model to author one.** Nothing here is derived from a session or
proposed by `reflect`. That absence is the entire safety argument, and it is
worth stating why it is an absence rather than a validated install path.

Snyk scanned 3,984 published skills: **36.8% carried at least one security flaw,
13.4% a critical one, and 76 held confirmed malicious payloads** — credential
exfiltration, remote binaries in password-protected archives, and instructions
telling the agent to disable its own safeguards. 91% of the malicious ones used
prompt injection *as well as* code, which is the combination that defeats code
scanners and model safety training at once.

Datadog's finding is the sharper one for a harness:

> A cloned repository can bring skills into a trusted session even if the
> developer never installed a skill from a marketplace.

mecha already refuses that shape for [triggers](/docs/features/triggers), and it
refuses it here the same way: **the store is global only, and there is nowhere
in a project's `mecha.toml` to put a skill.** A project may narrow the set by
name — useful, and always safe — and structurally cannot widen it (see
[Configuring](#configuring) below).

### Which is why a skill arms no taint

A skill body is the user's own words, exactly like the system prompt and your
`*.user.toml` rules. So the `skill` tool declares no capabilities and its output
is not marked as coming from outside. Treating it as third-party content would
be a category error in the direction that makes a model invent explanations for
its own harness — the same mistake as labelling a
[harness refusal](/docs/features/security) as somebody else's text.

This is the trade the whole design buys: skills can be **liberal** precisely
because [learning](/docs/features/learning) is strict. One is user-authored and
on-demand; the other is machine-derived and always-on, which is why one is
simply trusted and the other is provenance-gated.

## Loading is a tool call, not a `cat`

Some harnesses load level 2 by having the model run bash. mecha registers an
explicit `skill` tool instead, for four reasons that are all mecha's rather than
the standard's:

- **`shell` may be sandboxed or withheld entirely.** A loader built on it breaks
  in exactly the configurations that were locked down on purpose.
- **A tool call passes the [`pre_tool` gate](/docs/features/hooks)**, so a policy
  hook can decide which skills may load. A `cat` is invisible to hooks.
- **It lands in the trace**, so an [eval case](/docs/features/evaluation) can
  assert on it. A silent context injection is the thing Datadog named as
  defeating every downstream defence.
- The model does not have to know where the filesystem keeps things.

### Level 3 is served by the tool, never by `fs_read`

A skill lives in `~/.mecha/skills/`, which is **outside the run's workspace** —
so the [path jail](/docs/features/security) refuses to read it, correctly. The
first version of this told the model to open bundled files with the ordinary
file tools, which produced a call that could not possibly succeed; it was found
by running it, not by reading the code.

So bundled files come back through the same tool, `skill(name, file:)`, with
containment proved against the skill's **own** directory. `file` is the one
argument here a model can aim at a filesystem, so it gets the treatment every
model-supplied path gets: canonicalize, then prove containment, so neither `..`
nor a symlink climbs out.

## `tools:` narrows the surface, and can never widen it

A skill may declare the tools its procedure needs:

```yaml
tools:
  - fs_read
  - fs_list
```

While it is loaded, the surface is restricted to those. Three rules:

- **Union across loaded skills.** Each names what its own procedure needs, so
  intersecting would strand a run that loaded two. A union of subsets is still a
  subset, which is the invariant that matters: never larger than the
  unrestricted surface. Naming a tool that is not registered adds nothing.
- **The `skill` tool always stays reachable**, whatever a skill declared, or the
  first load would eat its own mechanism and a procedure saying *then load the
  follow-up skill* would name a tool that had just vanished.
- **It gates dispatch, not just the tool list.** A shortened list alone is
  advisory: a model that saw `fs_write` three turns ago can still name it. This
  is not hypothetical — a real run, having lost `fs_write` to a narrowing,
  reasoned *"let me just try calling it"* and was refused.

There is no unload, so a narrowing lasts the rest of the run. That is the
fail-closed direction, and it means a skill's `tools` list should name
everything its procedure needs rather than the minimum for its first step.

## A loaded skill survives compaction verbatim

[Compaction](/docs/features/compaction) replaces the middle of a transcript with
prose, and a summariser preserves *what is true* while dropping *how far you
got*. For a procedure it does something worse: **a paraphrased procedure is a
different procedure**, and the steps would survive as a plausible gist with
exactly the specifics gone that the skill was written to pin down.

So loaded skills ride across on `carried_state`, reproduced in full, and land
after the summary — the part of the rebuilt prompt known to be current rather
than paraphrased.

## Configuring

```toml
[skills]
enabled = []            # empty means every skill in the store
disabled = ["noisy"]    # applied after enabled, so it wins
```

A project's `mecha.toml` may narrow this and **structurally cannot widen it**:
`enabled` intersects with what is already selected, `disabled` unions, and a
`dir` from a project layer is dropped loudly. `--skill <name>` narrows the same
way, per run, and cannot enable something config withheld.

The level-1 block sits inside the cached prefix, so skills are listed in **sorted
order** — filesystem order is not an order, the same reason the tool registry is
a `BTreeMap`. Enabling or disabling one re-pays the prefix for that session,
which is why nothing may toggle skills per turn.

## Unattended runs name their skills

A [trigger](/docs/features/triggers) carries only the skills its file names, and
**empty means none** — the opposite default from its `tools` allowlist.

An unattended run has nobody to ask, so *what does this run actually do* has to
be answerable from the trigger file. If the model could load anything in the
store, a scheduled run's effective instruction set would grow every time you
wrote an unrelated skill. `trigger show` prints the line even when it is empty,
for the same reason it prints the resolved workspace: that question must not be
answered by a line that is not there.

```bash
mecha trigger add briefing --schedule "0 7 * * *" --prompt "…" --skill morning-brief
```

`mecha eval` forces skills off, like MCP, hooks, learned rules and the outbox: a
skill is whatever its author typed, so a case run on a machine holding one would
grade the procedure as much as the model.

The [front door's](/docs/features/frontdoor) extractor gets none by construction
— it is issued a request with an empty tool list and no system prompt, so there
is nothing to reach and no block to read.

## When a skill will not load

Reported at startup by name and reason, and `mecha skills` exits non-zero:

```
mecha: skill `broken` did not load — frontmatter says `name = mismatch` but the
directory is `broken` — they have to match
```

A skill that silently failed to load looks exactly like a skill the model chose
not to use, which is the same shape as a
[learning domain nothing routes](/docs/features/learning) — and reported for the
same reason. A name in `[skills]` matching nothing on disk is called out too.

## What is deliberately not here

- **An install command, a marketplace, or a registry client.**
- **Project-layer skills** — only project-layer *narrowing*.
- **Model-authored skills**, including promoting a reflection into one.
- **Remote bodies or runtime-fetched instructions.** A `SKILL.md` naming a URL
  is prose the model may act on through ordinary tools under the ordinary
  interlock, not a loading mechanism.
- **Bundled executables.** Level 3 scripts are the best part of the standard and
  the worst part of its threat model, and mecha's default sandbox is `none`. If
  they land later they run confined or not at all — the same rule as `shell` and
  MCP servers.
- **A skill that can widen the tool surface**, add a capability, or relax the
  interlock.
- **Auto-loading by keyword without a tool call.** The load has to be visible in
  the trace and gateable by a hook.

## Where to go next

- [Learning](/docs/features/learning) — the always-on, machine-derived half, and
  why it is gated where this is not.
- [Tools and MCP](/docs/features/tools-and-mcp) — subagents, the *delegate*
  shape to this *instruct* shape.
- [Triggers](/docs/features/triggers) — unattended runs, and why they name
  skills explicitly.
