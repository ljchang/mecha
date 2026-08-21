---
name: handoff
description: Update docs/HANDOFF.md and docs/HISTORY.md after a work session — verify which open items actually shipped, move them to the history doc, record any trap hit, and re-check the facts that go stale. Use at the end of a session that changed behaviour, before handing off, or whenever the handoff doc has drifted out of date.
---

# Closing out a session

`docs/HANDOFF.md` is the file someone reads before deciding what to build next.
That makes staleness worse than absence: a to-do list that mixes shipped work
with open work sends the reader to re-implement something that already exists,
or to skip something that does not.

This skill keeps that from happening. It takes ten minutes and it is the whole
reason the doc split is worth maintaining.

Read [`docs/README.md`](../../../docs/README.md) first if you are unsure which
document a given piece of writing belongs in — this skill only covers moving
things between HANDOFF and HISTORY.

## The one rule

**Verify against source, never against a commit message.**

A commit that says "add the rules cap" may have added a constant and no gate.
A commit that says "wire up X" may have wired half of it. The only evidence
that an item shipped is the code that implements it, cited as `file:line`.

This is not pedantry — it is the specific failure that produced a 1,965-line
handoff. Items were struck through when their commit landed, and the ones that
were never struck through silently became indistinguishable from the ones that
were never built.

## Procedure

### 1. Find out what actually changed

```bash
git log --oneline --since="<date the handoff was last updated>"
git diff --stat <last-handoff-commit>..HEAD
```

The handoff's own git history tells you when it was last touched:

```bash
git log -1 --format=%ad --date=short -- docs/HANDOFF.md
```

### 2. Test every open item, not just the ones you worked on

Walk `## What to do next` top to bottom. For each item, decide:

| Verdict | What to do |
|---|---|
| **Shipped** | Confirm with `file:line`, then move it to HISTORY (step 3) |
| **Partial** | Rewrite the item to describe *only the part that is still missing* — a half-done item described as whole is the worst kind of stale |
| **Still open** | Leave it. If the reason it is open has changed, say so |
| **Obsolete** | Move to HISTORY with one line on why it stopped making sense. Do not simply delete — the next person will re-propose it |

Fast ways to check:

```bash
# Does a command exist?
grep -n 'Subcommand' -A40 mecha-cli/src/main.rs
# Does a config key exist?
grep -n '<key>' mecha-core/src/config.rs
# Does a subsystem exist?
grep -nE '^pub mod' mecha-core/src/lib.rs
```

If an item is large, delegating the verification sweep to a subagent works
well — give it a line range and demand `file:line` evidence for every "shipped"
verdict.

### 3. Move shipped work to HISTORY, do not strike it through

Delete the item from HANDOFF. Add it to `docs/HISTORY.md` under
`## What shipped, and when`, in the prose paragraph for its date — not as a
bullet, and not as `~~struck through~~`.

Strikethrough is how the old handoff got long: it kept every completed item in
the reader's way forever. HISTORY is where completed work lives.

### 4. Record any trap you hit

If something cost you more than about half an hour, it belongs in
`docs/HISTORY.md` under `## Traps already hit`, in the matching cluster
(Measuring / Learning / Providers / Environment).

Write it as **what broke, then the general lesson**. The lesson is the part
that transfers:

> The hook timeout covered the wait but not the stdin write, so a hook that
> never read its input hung forever. Audit what sits *outside* every timeout,
> not just what is inside it.

A trap with no general lesson is a changelog entry — put it in the changelog.

### 5. Re-verify the facts that rot

These go stale silently. Check them every time:

```bash
# Test counts (the handoff states a per-suite breakdown)
cargo test --workspace 2>&1 | grep -E '^test result'

# Eval case and tag counts
python3 -c "
import json, collections
t=collections.Counter(); n=0
for line in open('eval/cases.jsonl'):
    s=line.strip()
    if not s or s.startswith('//'): continue
    n+=1
    for tag in json.loads(s).get('tags',[]): t[tag]+=1
print(f'{n} cases, {len(t)} tags')"

# Machine state, if the handoff's Environment section claims it
# What the server is actually serving. `n_ctx` is the PER-SLOT figure, which
# is what `context_window` must equal — never `-c`, which is divided across
# slots. `mecha setup` compares all of this against the config for you.
curl -s localhost:8080/props | jq '{total_slots, n_ctx: .default_generation_settings.n_ctx, vision: .modalities.vision}'
systemctl --user list-unit-files | grep mecha
```

Anything in `## Environment as left` that you verified should carry the date
you verified it. Anything you could not verify should say so rather than
carrying an old claim forward.

### 6. Check for material that belongs elsewhere

Read the file for things that are not current state or open work. There is no
line budget — a project with a lot genuinely open has a long handoff, and
truncating it to hit a number is how a real item gets deleted instead of
finished. What matters is that everything in it is *the right kind of thing*:

- Explaining *why* the code is shaped a certain way → `CLAUDE.md`
- A completed thing, or a lesson → `docs/HISTORY.md`
- A question you researched → its own `docs/*-RESEARCH.md`
- A thing designed but not yet built → its own `docs/*-DESIGN.md`
- How a user operates the feature → `website/docs/`

Length is a symptom worth reading, not a rule to enforce. If the file has
grown, ask *what* grew: more open work is honest, and a section that has
quietly become an essay is the thing to move.

### 7. Follow the cross-references

Moving a section breaks any pointer into it. Before committing:

```bash
grep -rn "HANDOFF" --include=*.md --include=*.rs --include=*.sh . \
  --exclude-dir=target --exclude-dir=node_modules
```

Repoint anything that referred to content you moved.

## What good looks like

After this pass, a reader who has never seen the project should be able to
open `docs/HANDOFF.md` and answer three questions without opening any other
file, and without opening the source to check whether the doc is lying:

1. Does it build and pass, and what should I run first?
2. What is actually true about the system right now?
3. What is genuinely unbuilt, and which piece is cheapest to start on?

If any answer requires reading the code to confirm the doc, the pass is not
finished.

## Anti-patterns

- **Striking items through instead of moving them.** The list only grows.
- **Trusting your own commit message.** You wrote it before you finished.
- **Recording a measurement without its conditions.** A number with no arm,
  no `n`, and no date is not a result and will mislead someone later.
- **Carrying an unverified environment claim forward.** Say "unverified" —
  a stale fact stated confidently costs more than a gap.
- **Adding a "future ideas" section.** That is what the research docs are for;
  ideas with no verified gap behind them turn the handoff back into a wishlist.
