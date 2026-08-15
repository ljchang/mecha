# The trifecta map

The lethal trifecta is **private data + untrusted content + a way out**. An
agent holding all three can be instructed, by text hidden in what it reads, to
send what it knows to whoever wrote the text. mecha's answer is structural —
capability bits on every tool, a taint that arms as content arrives, and an
interlock that refuses the third leg once two are present (`agent.rs`, the
`TrifectaPolicy::Block` arm).

One interlock — but **four distinct ways a session assembles the legs**, each
broken by a different mechanism. No single fix covers them all, and knowing
which mechanism owns which channel is the difference between tightening the
right screw and loosening policy in frustration. That frustration is not
hypothetical: every mechanism below exists because the alternative was an
operator reading refusals until they set `trifecta = "allow"`.

The design goal, stated once: **the restrictions are load-bearing and the
system is shaped so you never feel them.** Local work runs confined without
asking. Outbound mail becomes a draft you review. Web reading happens in a
clean conversation. When something *is* refused, the refusal names the way it
could stop being refused ([`denial_remedy`](../mecha-core/src/tool/mod.rs)).
A control that dead-ends its operator teaches them to weaken policy, which is
the worst outcome of a control working correctly.

## Channel 1 — local shell as the way out

Unconfined, `shell` can `curl` anything anywhere, so it truthfully declares
`external_send` and the interlock refuses it once mail or graph content is in
the conversation — including for `git log`, which cannot exfiltrate anything.

**Owned by: the sandbox.** `[sandbox]` with `network = false` confines
commands (bwrap or docker) with no network; `shell` derives `external_send`
from `Sandbox::can_reach_network()`, so confinement flips the bit off and the
interlock simply never fires on local work. Fail-closed: `Sandbox::preflight`
refuses to start if it cannot actually confine. The accepted cost is that a
confined `cargo build` cannot fetch crates — run dependency-fetching builds
yourself.

What it does NOT do: a confined shell still reads the workspace, so
`private_data` stays true deliberately — otherwise `shell: cat secrets` would
taint nothing while `fs_read: secrets` does, and the cheapest route around the
interlock would be the more dangerous tool.

## Channel 2 — a genuinely outbound tool as the way out

`mail_send`, `mail_reply`, `calendar_*`, a Slack post, `http_fetch`,
`web_search`. These send by definition — a GET exfiltrates via the query
string — and no sandbox changes that.

**Owned by: the outbox, and delegation.** Two sub-cases:

- *Sending things* — outbox-routed tools skip the interlock because staging
  sends nothing: the call becomes a draft in a local file, release requires a
  human to read exactly what would leave, and the draft records the
  conversation's taint so the review can say "possibly an attacker's words"
  out loud.
- *Reading the outside world* — delegate to a child whose only capabilities
  are fetch-shaped (`research`). The refusal suggests this route by
  capability signature alone: reads untrusted, holds no private data, cannot
  send, destroys nothing. The suggestion says it is for READING — a delegate
  cannot do local work, and pointing shell-needing work at it was the
  measured dead end that `denial_remedy` ended.

The residue — a fetch whose *content depends on tainted context* — is
structurally indistinguishable from the attack, and no mechanism makes it
safe. That is what `trifecta = "ask"` is for: a human decides, per call.

## Channel 3 — the untrusted content itself

Before any send is attempted, the injected text has to be read. This channel
is not about stopping exfiltration but about arming the taint honestly and
keeping instructions from being followed.

**Owned by: source declarations and wrapping.** MCP servers that custody
third-party text carry `untrusted_input = true` in config (pkg, mail — no
annotation can say "untrusted", so the config override is load-bearing).
Tool results that really came from outside are wrapped in
`<untrusted-content>` markers telling the model to treat them strictly as
data. The declared capability is what arms the leg; the wrapper is advisory
armor on top, never the enforcement.

## Channel 4 — a subagent laundering the legs apart

A child reads the mail and hands the parent "a summary" — is the summary
clean? No, twice over, and `Subagent::new` derives both answers from the
child's own tools so nobody has to remember:

- A summary of attacker-influenced text can still carry instructions, so a
  child with untrusted-capable tools yields an untrusted-capable subagent.
- A summary *made of* private data is still private, so the private leg
  survives the return no matter what. `trusted_output` never touches it.

**Owned by: capability derivation, plus the shaped vouch.** `trusted_output`
is the one deliberate narrowing, and it is an *offer*, not a waiver: it must
name an `answer_shape` (`"number"`, `"boolean"`, or a closed list), a bare
`trusted_output = true` refuses to construct, and each answer earns the trust
at return time by parsing as the declared shape. Instructions cannot hide in
`42` or `yes`; they hide in prose, and prose never matches a shape. A
mismatch comes back marked untrusted with a note saying why. There is
deliberately no bounded-string shape — "ignore previous instructions" fits in
very few characters, so a length cap vouches for nothing.

## The switches, and what each one costs

| Switch | Default | What it changes |
|---|---|---|
| `[sandbox] network = false` | sandbox off entirely | Confines `shell`; local work stops tripping the interlock. Costs: no network in confined commands. |
| `trifecta = "block" \| "ask" \| "allow"` | `block` | `ask` escalates armed sends to a human; `allow` waives the injection interlock entirely — the leak guard below still applies. |
| `block_sends_after_private` | off | The leak guard: refuses sends once private data is present, injection or not. Off by default because it breaks ordinary work — the default posture defends the injection path, not deliberate egress. |
| `trusted_output` + `answer_shape` | off | Per-answer trust for shape-provable child answers. Costs: the child's answer must literally be a value. |
| `subagent trusted_output` without shape | — | Refuses to construct. This is not a switch; it is the hole that used to be one. |

## What to do when something is refused

The refusal itself now says — that is `denial_remedy`. But the general moves,
in order of preference: confine it (channel 1), stage it (channel 2, sends),
delegate it (channel 2, reads), shape it (channel 4) — and only then
`trifecta = "ask"`, which puts a human where a structure should have been.
