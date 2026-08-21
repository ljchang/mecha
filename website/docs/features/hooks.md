---
title: Hooks
sidebar_position: 8
description: Commands that attach policy, redaction and logging to the loop's lifecycle points without editing the loop.
---

# Hooks

A `[[hook]]` is a shell command that runs at a lifecycle point, with the event
as one JSON object on stdin. The point is that policy, redaction and logging
attach to the agent **without** anyone editing `agent.rs`.

```toml
[[hook]]
event = "pre_tool"                     # pre_tool | post_tool | session_end
tools = ["shell"]                      # empty means every tool
command = "~/.mecha/hooks/no-force-push.sh"
timeout_secs = 10                      # default
```

Each hook runs via `sh -c`, as you, in the workspace. The order in config is
the order they run; for `pre_tool` the first denial wins and later hooks do not
fire.

## The three events

| Event | Payload on stdin | Can it decide? |
|---|---|---|
| `pre_tool` | `event`, `tool`, `input` | Yes — exit 2 denies the call |
| `post_tool` | `event`, `tool`, `input`, `is_error`, `content` (first 4000 chars) | No |
| `session_end` | `event`, `session_id`, `path` | No |

```json
{"event": "pre_tool", "tool": "shell", "input": {"command": "git push --force"}}
```

`post_tool`'s `content` is bounded deliberately: a hook that wants the whole
output can read the session file. Stdin is for deciding, not archiving.

A minimal policy hook:

```bash
#!/usr/bin/env bash
# exit 0 allows, exit 2 denies with this output as the reason
jq -e '.input.command | test("push +--force")' >/dev/null || exit 0
echo "force-push is not allowed in this workspace"
exit 2
```

## Where hooks sit in the dispatch path

```
interlock  →  pre_tool hook  →  approver (the human)  →  execute
```

Two properties follow from that order, and both are load-bearing.

**A hook can narrow policy and never loosen security.** The trifecta interlock
runs first, so a hook that exits 0 does not un-block a refused send. Hooks are
an additional gate, not a replacement for the structural one.

**A `pre_tool` denial never reaches the human.** Mechanical policy is cheaper
than an interruption, and — the part that matters — a hook cannot be talked
into clicking yes. Escalating a rule that a script can decide would put a
dialog in front of a user for something already settled, and dialogs are what
an injection is trying to produce.

## `pre_tool` fails closed

| Outcome | Verdict |
|---|---|
| exit 0 | allow |
| exit 2 | **deny**, with the hook's stdout (or stderr) as the reason |
| any other exit code | **deny** |
| spawn failure | **deny** |
| timeout (10s default) | **deny** |

:::warning
A policy hook that cannot run and quietly allows is worse than no hook. It is
the silently-degrading-sandbox mistake with different spelling: the operator
believes a rule is being enforced and it is not. So anything that is not an
explicit exit 0 denies.
:::

The timeout covers the stdin write as well as the wait. That was a real bug: a
hook that never reads stdin blocks the write once the payload outgrows the
pipe buffer, so a `pre_tool` hook fed a large `fs_write` input hung the run
forever with the timeout never starting.

`post_tool` and `session_end` are **observers**. Their failures are logged and
swallowed, because an observer must not be load-bearing. If something has to
be able to stop a call, it is a `pre_tool` hook.

## A hook denial reads differently from a human denial

```rust
// hook
content: format!("Blocked by a hook: {reason}"),
// approver
content: format!("Denied by the user: {reason}"),
```

The strings are not cosmetic. The [learning](/docs/features/learning) miner
keys on `"Denied by the user:"` to find the moments you stepped in. Machine
policy is not a user correction, and mining it would teach mecha rules it was
already obeying mechanically — filling the system prompt with restatements of
a script that already runs on every call. Both strings have tests naming that.

## Subagents inherit the parent's hooks

`setup::build_subagent` installs the parent's `HookSet` on every child, for
the same reason a subagent inherits the outbox route: otherwise delegating
would be the way around a `pre_tool` policy.

## Validation, and when hooks are off

Hook config is validated on **every** start, including when `--no-hooks` skips
installing it:

```rust
// Validated even when --no-hooks skips installing them.
let hooks = mecha_core::hooks::HookSet::from_config(&cfg.hooks)?;
let hooks = (!opts.no_hooks && !hooks.is_empty()).then(|| Arc::new(hooks));
```

An unknown event name or an empty command is a startup error. A policy hook
that never fires because of a spelling should fail on every start, not only on
the runs that needed it.

`mecha eval` forces hooks off, the same way it forces MCP, learned rules, the
outbox and provider fallbacks off: a scorecard shaped by this machine's local
scripts grades the machine, not the model.

## A worked example: self-driving learning

The reflection pass can be triggered by the close of every session, detached
so the hook's timeout never kills a model call in flight:

```toml
[[hook]]
event = "session_end"
command = "nohup mecha reflect -p local >/dev/null 2>&1 &"
```

## Where to go next

- [Security model](/docs/features/security) — the interlock that runs ahead of hooks.
- [The outbox](/docs/features/outbox) — staging, which happens after the hook gate.
- [Learning](/docs/features/learning) — what the denial strings feed.
- [Configuration reference](/docs/reference/configuration) — every `[[hook]]` key.
