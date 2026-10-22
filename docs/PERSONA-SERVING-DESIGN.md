# Serving to others: mecha personas as HyperStudy agents

*2026-10-19; transport and persona export revised 2026-10-22 (S11, S12).
Design, not built.* How a mecha persona takes part in a
HyperStudy experiment — reading what the study shows, talking by text and
voice, and acting in the study (a rating, a chat line; advancing the screen
is not an agent capability in HyperStudy yet) — for people who
are not the owner, without anything of the owner's in reach of the
conversation. PERSONA-DESIGN §14 deferred
serving and listed what it would need; this is the design it asked for.

The wire contract is HyperStudy's, not this document's: the **Agent Adapter
Spec**, published at `docs.hyperstudy.io/developers/agent-adapters/` and
generated from HyperStudy's source, one JSON file per version
(`agent-spec/<version>/agent-spec.json`). It defines two adapter kinds, and
mecha serves both: a **model endpoint** for text agents (OpenAI-compatible
Chat Completions) and a **voice engine** speaking the HyperStudy Realtime
Profile (HRP), a pinned subset of OpenAI's Realtime API with a conformance
suite. In both, HyperStudy connects to the adapter. This document is written
against spec **0.2.0** and conformance suite **6**; it decides only what mecha
builds against them, and points there rather than restating them, because
two copies of a protocol drift into disagreeing. HyperStudy's
`dev_docs/VOICE_AGENTS_DESIGN.md` is the design behind the spec, and where
the two differ the published spec is what HyperStudy runs.

## 0. Rulings

| # | Ruling | By | Date |
|---|---|---|---|
| S1 | ~~**mecha connects to HyperStudy directly**~~, dialing out over HyperStudy's `accept` transport. *Superseded by S11:* `accept` is HyperStudy's Phase 3, unbuilt, and its published spec has HyperStudy connect to the adapter. What survives: no `hyperstudy-agent` in the path — mecha serves the contract itself. | owner | 2026-10-19 |
| S2 | **Study policy is the experimenter's.** Disclosure, concealment, consent, age and institutional review belong to whoever runs the study. mecha builds infrastructure: it provides mechanisms (§7) and takes no position on how a study uses them. | owner | 2026-10-19 |
| S3 | **Personas only, for now; the owner's live mecha is never served.** A seat runs a persona from a sealed guest home (§3). A general assistant may come later, and when it does it is a sandboxed instance launched for the purpose — never the owner's running mecha, its stores or its main chat (§1). | owner | 2026-10-19 |
| S4 | **Engine-owned turn-taking for one human and one agent.** The engine decides when a turn ends and when it is interrupted (HRP engine mode); platform mode, for two or more humans, is a later phase on both sides. | owner, via the HyperStudy session | 2026-10-17 |
| S5 | **What a seat does:** reads text and images the study presents, chats by text and by voice, and takes the study's actions — answering on-screen questions and ratings. **No image generation** for now (§4a). | owner | 2026-10-19 |
| S6 | **A feature of its own,** off until the owner turns it on in setup (§6). | owner | 2026-10-19 |
| S7 | **mecha mints the key;** the owner pastes it into HyperStudy (§5.1). Since S11 it is a key HyperStudy presents to mecha, naming the personas it serves. | owner | 2026-10-19 |
| S8 | **A seat is recorded in full.** The point of a local model is that what it did can be investigated afterwards: transcripts, directions and the voice journal are kept as an owner's call's are, not silenced (§3). | owner | 2026-10-19 |
| S9 | **`max_seats` caps concurrent seats,** so studies cannot slow every conversation on the box, the owner's included (§5). | owner | 2026-10-19 |
| S10 | **The system prompt is the study's.** A seat is prompted with what HyperStudy sends as its instructions and nothing of the owner's persona text — no `BASE`, no card, no `DISCLOSED`, no `SINCERE`; the persona a seat names contributes its voice (§3) and, since S12, its picture. | owner | 2026-10-19 |
| S11 | **HyperStudy connects to mecha,** as its published adapter spec has it: an API is registered in HyperStudy and assigned to an agent persona, and HyperStudy calls it — the model endpoint for text, an HRP engine URL for voice. mecha mints the key, and a key names the personas it serves (S7, §5.1). That puts a public address on this box (§5.1); what answers it is a confined listener that holds nothing of the owner's (§2). | owner | 2026-10-22 |
| S12 | **The persona's voice and picture come from mecha,** exported into HyperStudy's persona (§5.2). For now mecha is their only source; whether HyperStudy's users may edit them there is decided later. The prompt stays the study's (S10). | owner | 2026-10-22 |

## 1. Why no existing door fits

Every door mecha has assumes the speaker is the owner, and each says so in
code:

- **`mecha voice-serve`** binds `LISTEN_HOST` (`127.0.0.1`, pinned by a
  test) and treats the newest user message (`last_user_text`) as the owner
  talking. A spoken yes is matched by `review_policy::parse_answer` and can
  release a staged draft (`voice/confirm.rs`). A stranger's "yes" would be
  the owner's.
- **Persona memory** (`persona::writer`, `persona::recall`) records the human
  side of a persona chat as facts about the owner and recalls the owner's
  past into every turn. A stranger's sentences would become the owner's
  remembered life, and the owner's life would be recalled to the stranger.
- **`mecha serve`** is the owner's page: `owner_guard` admits only the
  tailnet login that is the owner.
- **The approver** now has a second door, `Approver::escalate_access`, which
  asks a person to grant a path (#716). There is no person on the far end of
  a study session entitled to answer it.
- **The persona prompt itself.** `persona::agent::BASE` presents the
  interlocutor as the character's author, with an assistant of their own
  that the chat is separated from. `DISCLOSED` (appended while the
  persona's `disclosure` switch is on, the default) relies on `mecha
  serve`'s AI tag having done the disclosing — a tag no participant sees.
  In a seat both premises are false, and the second steers the model away
  from a disclosure the study may want — which is why a seat uses none of
  it (S10).

**Why not the front door's shape.** The one door that already faces
strangers, `frontdoor.rs`, quarantines a request and hands the privileged
run a typed extraction, never the prose. That fits a request to be acted
on later; a seat's whole job is to converse in prose with the stranger in
real time, so an extraction would remove the conversation itself. A seat
answers the same threat the other way: it holds none of the owner's
stores, so there is nothing for the stranger's prose to reach (§3).

**Why the owner's main chat cannot be served even with every tool off
(S3).** The trifecta interlock guards tools whose destination the model
chooses. In a conversation with a stranger, the *reply itself* is a way out:
spoken to the person who asked, chosen by no tool, seen by no interlock. A
context that holds mail, calendar or the graph leaks by answering "what was
in the last email from…". So the line is drawn around the owner's *stores*,
not around capability — which is why a general assistant can be served
later, as a fresh sandboxed instance holding none of them, and why the
running mecha never can.

## 2. The shape

```
 HyperStudy backend (the caller)            this box
 ───────────────────────────────            ─────────────────────────────────────────────
 text agent:  POST /v1/chat/completions ──┐
 voice agent: GET  /v1/realtime (wss) ────┤  https://<machine>.ts.net:10000
   Authorization: Bearer mhs_…            │  Tailscale Funnel, on a port of its own,
                                          │  never one that serves the owner's page
                                          ▼
                              mecha-hyperstudy  (the listener: its own process, bwrap,
                                no network; key first, then bounds, then closed types;
                                listens on hyperstudy/run/public.sock)
                                          │ listener.sock: a closed message set
                                          ▼
                              mecha-cli/src/hyperstudy/  (the wiring, in the owner's home)
                                keys · admission · seat records · crisis sensor · export
                                  │                                     │
                    text decision (§2a): one model         voice seat: a guest facade (§3)
                    call through the router; nothing        and the worker's HRP transport:
                    runs here, the tool call goes back      Silero · SmartTurn · Parakeet · Breeze
                                          │
 LiveKit room ◄─► HyperStudy's voice bridge: the only room presence; mecha never joins
```

Four pieces, each with one job:

1. **`mecha-hyperstudy`** — the listener, a crate and a small binary with
   **no `mecha-core` dependency**, checkable in `Cargo.toml`: the shape
   `mecha-slack` already has, inbound here where Slack's Socket Mode is
   outbound. It serves the spec's three routes and nothing else:
   `GET /v1/models`, `POST /v1/chat/completions`, and `GET /v1/realtime`
   upgraded to a WebSocket. It checks the bearer key before it reads a
   body, so a request without a valid key gets a bare 401 whatever its
   path, and nothing at the address says which personas exist or that
   mecha is behind it. Then it bounds what it reads: header and body size,
   images per request, a WebSocket frame's size (from the spec's 2000 ms
   cap on one audio delta), and connections per key. Then it parses what
   remains into closed types: the chat-completions request shape the spec
   documents, and HRP's client events. An unknown event type, an unknown
   field in a closed place, or a value out of bounds is refused with a
   typed error, never passed on. It is the only code on this box that
   parses bytes from the internet, and it knows nothing about personas or
   runs.

   **It runs as its own confined process.** Under S1 the crate would have
   parsed bytes only from an origin mecha chose to dial. Under S11 anyone
   who learns the address can send bytes before any key is checked. So the
   parser runs where a bug in it reaches nothing. The wiring launches it
   under bwrap with `--unshare-net`, its environment cleared. No part of
   the owner's home is bound into it except its own run directory and,
   read-only, the directory that holds the key *hashes* (§5.1). It listens on a pathname
   socket, `hyperstudy/run/public.sock`, which `tailscale serve` proxies
   to, and it reaches the wiring only through
   `hyperstudy/run/listener.sock`: the same pathname-socket crossing §3
   uses for a seat's facade. With no network namespace it cannot reach
   the router, the owner's page on loopback, the LAN or the tailnet. A
   compromise of it is a process that talks to tailscaled's proxy and to
   one socket whose every message the wiring checks again. Two things
   this rests on are checked on the installed tailscale before step 2
   commits to `public.sock`, not assumed. First, that `tailscale serve`
   takes a `unix:` target at all: this box's 1.98.10 says so in `tailscale
   serve --help` ("you can also specify a Unix domain socket"), but nothing
   in this repo exercises one yet, and every current `tailscale serve` use
   here is a loopback TCP origin. Second, that Funnel carries a WebSocket
   upgrade through to such a target. If either fails, the crossing becomes
   a loopback forwarder inside the listener's namespace, the second in-jail
   process §3 already prices for the facade, with tailscaled proxying to a
   loopback port. That is a change to this paragraph, reviewed as one, not
   a detail of step 2. A box where the confinement cannot run serves
   nothing.

   **What crosses `listener.sock`** is a closed set, enumerated here as
   `parent.sock`'s is in §3, and the wiring checks the key again on every
   message that carries one, against the store as it is at that moment
   (§5.1):
   - a models query, naming the key that asked, answered by the wiring
     with the personas that key names; the listener holds no persona
     names, so `GET /v1/models` is the wiring's answer, carried back;
   - a text decision: the key, the run identifier when there is one, and
     the parsed request, answered by the response or a typed refusal (§5);
   - a voice session's open, with the key and the safety identifier,
     answered by an admission or a typed refusal; then its frames, each a
     parsed HRP client event inward or a server event outward; then its
     close, from either side.

   Anything else is refused, logged, and closes the connection it came
   on.
2. **`mecha-cli/src/hyperstudy/`** — the wiring that does know: the keys
   and the personas each one names (§5.1), admission (§5), the seat records
   (§3), the crisis sensor (§7), the export (§5.2), and the two kinds of
   seat. It answers a text decision itself (§2a) and hands a voice seat to
   a guest facade and the worker.
3. **The worker's HRP transport** — a new transport in
   `mecha-core/units/voice/worker.py`, beside `SmallWebRTCTransport` and
   `UplinkTransport`. Below `run_bot` the pipeline is transport-agnostic;
   above it, both existing transports are WebRTC (`UplinkTransport`
   subclasses `SmallWebRTCTransport`) and are built at one site inside the
   WebRTC runner's entry, so this is the first non-WebRTC transport: pipecat's
   transport surface implemented from scratch, plus its own entrypoint and
   route beside the offer endpoint. It plays the *engine's* side of the
   Realtime protocol over a loopback relay from the wiring, never over the
   internet: the owner's worker is unconfined and holds the owner's
   environment, so it does not terminate a stranger's socket or parse its
   frames. The listener does (piece 1). The cost is one loopback hop for
   audio.
4. **The guest facade** — a voice seat's agent loop, served from the guest
   home over a Unix socket (§3, §4, §4a). A text decision has none: it runs
   no tool and holds no conversation between calls (§2a).

The judgments are made in Rust before a seat opens: is the key valid,
which persona does it bind, is the box free. The worker stays what its
module doc says it is: "deliberately dumb".

**The public address.** Funnel publishes one HTTPS port of the machine's
`*.ts.net` name, and it allows three: 443, 8443 and 10000. On this box 443
and 8443 already serve the owner's page to the tailnet, and Funnel on
either would publish that page to the internet. So the listener takes
**10000**. The wiring reads the machine's serve configuration with one
command, `tailscale serve status --json`, at start and before it admits a
seat, and nothing else in this design reads that fact another way. Funnel
on for any port whose handler is not `public.sock` stops the wiring with
the reason. So does a configuration it cannot read: `tailscale` missing,
a non-zero exit, JSON that does not parse, tailscaled down. "No other
Funnel found" and "could not look" are opposite findings, and only the
first lets a seat in. This is a silently-degrading guard in the sandbox
preflight's shape. Funnel itself is a tailnet setting, the `funnel` node attribute in
the tailnet policy, which the owner turns on once and setup names but
cannot grant (§6). Another tunnel (`cloudflared`) could stand in front of
the listener instead; Funnel is the one built, because the box already runs
tailscaled, and no other is checked here.

## 2a. A text decision

HyperStudy's model endpoint is stateless by design. Each call sends the
whole situation:

- a system message with the agent's persona and instructions;
- a user message describing the screen, with the recent chat and any
  images on it as image parts;
- the tools usable right now: `send_chat`, `wait`, and `submit_response`
  with that question's value schema.

It expects a tool call back. HyperStudy keeps the agent's state, carries
out the tool call itself and calls again on the next event, and no tool
result ever comes back to the endpoint. A failed or late request is a
failed decision, never retried, inside a 60 s default limit the
experimenter can set from 10 to 300 s.

So a text seat is not a conversation held here, and needs none of a voice
seat's machinery: no guest facade, no fold, no ClientTools. One decision is
this:

- **One model call, made by the wiring.** The listener's parsed request is
  turned into mecha's own `Message`s and sent through the seat's one
  router entry, derived from the owner's as §3 derives a voice seat's,
  with its model pinned (`follow_loaded = false`). The request's `model`
  field names a persona, one of those its key names, and that persona
  contributes its voice and picture and nothing else. Its own provider pin
  (`persona::Settings::model`) does not travel into a seat, as its prompt
  does not (S10). So the caller's choice among a key's personas selects no
  model, and every seat on the box answers on the one model the router has
  resident, which is what §5's hold protects. The request's `model` field
  never reaches the router either: on the router that field *selects* a
  preset, and a stranger's request must not load one, directly or one hop
  removed. On the session engine (`SESSION-ENGINE-DESIGN.md`) a decision
  is a session of one turn opened like any seat's, so its record carries
  all three axes.
- **Nothing runs here.** The declared tools go to the model as declarations
  only, and the model's tool call goes back to HyperStudy as the response.
  mecha executes nothing, so a text decision reaches nothing but its own
  response, the party every reply already reaches (§1). The interlock has
  no send to judge, and the catalogue's `Blind` labels (§4a) are not
  needed.
- **The system message is the study's, verbatim (S10),** under the bound
  and with the clock head note of §3. mecha adds no persona text. The
  conversation is armed `untrusted` at construction, as a voice seat's is
  (§3), so the record says so.
- **Recorded in full (S8).** Each decision is appended to its seat's
  transcript under `hyperstudy/seats/<seat>/`: the request as the model
  saw it, the response, and the model `served_props` reported. No guest
  home is built for it, since nothing runs: its `home/` holds the
  transcript alone (§3).
- **A seat is a HyperStudy run.** The seat is found by an opaque per-run
  identifier HyperStudy sends, and named by mecha (§3). Spec 0.2.0 sends
  none to a model endpoint (its voice engine gets one, as
  `OpenAI-Safety-Identifier`), so it is on HyperStudy's list (§8): the same
  opaque value, in the request's `safety_identifier`. A text decision
  without one is **refused `policy`**, with the reason, as §4 refuses a
  voice connection without one: the crisis cooldown (§7) and the per-seat
  budgets (§4a) are per seat, and a check that cannot run must stop the
  run, not fall through. Until HyperStudy sends the identifier, step 2 runs
  against its dev server under `[hyperstudy] dev_unidentified_runs = true`,
  off by default and named by `Unready` while on. In that mode every
  decision is a seat of its own, with no cooldown and no budget across
  decisions, which is why it is a switch the owner sets and not a
  build-order note.
- **The crisis sensor screens what is new.** Each request repeats the
  recent chat, so the wiring screens only the participant text it has not
  already screened for that seat. On a trip it answers with a `send_chat`
  carrying the handover (§7) in place of the model's response, and
  answers `wait` until the cooldown ends. This also needs the per-run
  identifier.
- **A decision that cannot start in time is refused at once.** When the
  router has no slot within a share of the request's limit, the answer is
  `capacity` (§5), not a timeout, so HyperStudy's record says why.

## 3. The sealed guest home

The isolated-home pattern of PERSONA-DESIGN §14's third option, kept on
this box rather than a separate one, and built with `trial_env`'s: an
isolated home per seat, assembled from an explicit list, never from the
operator's home. The guest home is a voice seat's. A text decision runs
no loop and keeps nothing between calls (§2a), so a text seat's directory
holds only its transcript and manifest, and what this section says of the
interlocutor, the prompt and where a seat is kept holds for it as well.

**The seat directory** (`hyperstudy/seats/<seat>/`) holds four things side
by side, none inside another: `home/` (the guest home, `MECHA_HOME`),
`workspace/` (the jail, which contains no part of `home/` — `setup`'s rule
that a workspace never contains the mecha home), `run/` (the sockets, under
*The agent loop is its own process* below) and the manifest.

**mecha names the seat.** `<seat>` is minted on this box when a seat
opens, a random id from the OS source, and is the only name a path or a
record of the owner's ever carries: the directory, the ledger's `seat`
field (§7), the hold. HyperStudy's run identifier is kept inside the
seat's manifest, where the wiring looks it up to find the seat again, and
is never a path component or a ledger field. In the OpenAI API that header
is borrowed from, `safety_identifier` is a stable per-user value (OpenAI
suggests hashing a username or email), and HyperStudy is asked for a
per-run one. If it ever sends the convention's value instead, nothing on
this box is named by it, and a study cannot link seats across studies
through the owner's directory listing or `safety.jsonl`.

**It holds:** the named persona's voice settings (the voice name the
owner's worker is handed, its speed and instructions), copied in,
read-only, and not its reference audio: a voice is a name on the wire,
the owner's worker resolves it through the speech server, nothing in a
seat reads the file, and a seat directory is kept after close (S8). And
nothing of its text: who the agent is
comes from the study (S10). Its config is written fresh by the wiring, not
derived: `[security]` is taken from the owner's field by field — the
owner's value where it narrows a seat, the strictest value where it would
widen one (§4a). "Strictest" is defined only for the three fields that
bear on a seat: `trifecta` (pinned `block`), `mark_untrusted_output`
(always on) and `block_sends_after_private` (the owner's, §4a — the named
departure). Of the other three, which guard the fetch and search tools a
seat does not have: `block_private_ips` is set `true`, its strictest value,
by the rule, and inert in a seat under either posture, since only
`check_url` reads it and only `http_fetch` and `web_open` reach that; the two domain lists have no
strictest value, since an empty list is the *widest*, so they are copied
as the owner's and nothing is inferred from them; `[sandbox]` is not the owner's
at all but written by the wiring as a seat property, as `trifecta` is
enforced (below, after the launch);
`[providers]` names only the loopback router, with no key, and copies
the three fields the seat depends on from the owner's router entry:
`context_window` (it must equal the router's `-c / -np`, as everywhere —
without it compaction never fires and the output budget falls back to a
ceiling too large for the slot), `vision` (without it every study image is
replaced by a text placeholder while still arming `private`),
`structured_output` (it defaults to `Disabled`, and the crisis judge sends
its `response_schema` only when it is on, so without it §7's forced-on
judge runs without its schema; IMAGE-DESIGN §9's 2026-10-08 ruling sets
`llama_json` on every router-served provider). It pins a fourth,
`follow_loaded = false`, as a seat property rather than inherited, as
`trial_env` sets it for every arm — following whatever the owner last
loaded would change a seat's model mid-conversation, the other half of §5's
hold. The agent's model comes from the owner's, never from the named
persona's own `model` pin, which stays with the owner's chats (§2a); its `timezone` is written
as `UTC`, never copied, because the calendar head note renders in the
configured zone (`Agent::calendar_note`) and a participant reading the
owner's zone learns where the owner is — and a participant may be
anywhere, so no zone is theirs either; a study that wants local time says
so in its own instructions. `max_tokens` is copied from the owner's, as
`context_window` is, since both are facts of the one router: it must sit
above the preset's `--reasoning-budget`, or the reply is HTTP 200 with
empty `content`, which in a seat reads as a model-server fault rather
than the misconfiguration it is. `max_turns` is pinned one above §4a's
tool-rounds-per-turn budget, so that budget's `is_error` always answers
first and `max_turns` is only the backstop that ends a runaway run; two
bounds on one quantity with opposite failure modes must not race. Every
other `[agent]` field is its default on purpose. No other
table is written —
no `[[mcp]]`, `[[hook]]`, `[rule]`, `[approval]` or `[search]`. That is
deliberately not `trial_env`'s shape: a trial home inherits the operator's
`[providers]` and keys along with `[security]`, and a seat inheriting them
would hand a stranger's conversation a cloud key and a non-local model with
the seal intact around it.

**The floor is a function, not a list.** The six `[security]` fields named
above are today's, and a seventh added for the owner's surfaces must not
reach a seat by default, since the default would be "copy the owner's",
the widening direction. So the seat's `[security]` is built by one
function that destructures `SecurityConfig` with no `..` and returns, per
field, `Owner`, `Strictest` or `Pinned(value)`. A new field does not
compile until a seat's answer for it is written, which is the shape
`Feature::switchable_from_environment`'s exhaustive `match` takes for the
same reason. §8 step 4's floor test runs over every field the function
returns, not over the two it names.

**It never holds:** the owner's persona memory or chat history with that
persona; mail, calendar, graph, outbox, learning store or triggers; the
owner's `[[hook]]`s, skills, charter or subagents — the list `trial_env` was
rewritten to exclude after a derived home's inherited `session_end` hook put
synthetic sessions into the owner's graph (2026-09-23), and hooks run shell
where a stranger would be firing them; skills and the charter are prompt
content, which S10 rules out; the
owner's `[[mcp]]` servers — including those marked `personas = true`, which
were marked for the owner's chats, not for strangers; provider keys beyond
the loopback model endpoints. What keeps those out of the *directory* is
the explicit source list; what keeps them out of the *process* is the
launch, and the launch's confinement has a stated source and a floor.
`SandboxConfig::default()` is `Backend::None`, under which
`wrap_argv_with_env` returns the bare command — host network, no namespace,
no cleared environment — and the trial-home precedent inherits the
operator's `[sandbox]` among its machine tables. A seat does neither: its
backend is bwrap, pinned by the wiring whatever the owner's `[sandbox]`
says, and a box where that backend cannot run opens no seat. The rule
about a configured sandbox that fails covers the broken case; this one
covers the never-configured one, which would otherwise make "egress no
longer rests on the tool catalogue alone" silently false on a default
install. So: the guest facade is spawned with its environment cleared first and
an explicit allowlist added back, in `Sandbox::child_env`'s shape — the
`mcp.rs` `env_clear()` rule, since `Command::envs()` alone layers on an
inherited environment and hands the child the owner's provider keys. The
load path is decided beside it: the facade loads with `global_config_only`
set, as `voice-serve` sets it a few lines from the `system_extra` this
design inherits, so a project config in whatever directory it starts in
never layers in. Today the start directory would be an empty `workspace/`,
so nothing would load either way, but a layered config is a load-path
decision and is made here rather than left to where the process happens to
start. That
holds whether or not the bwrap question in §9 is settled either way. The guard on the home's *location* is the trial home's,
`experiment::refuse_unsafe_home` — it refuses a home that is or contains
the real one and allows one inside it, which is where §3 keeps seats — not
`trial_env::refuse_operator_home`, which guards an environment's source
data and would refuse the seat directory itself.

**The agent loop is its own process; the audio leg is shared.** The guest
facade runs as a child with `MECHA_HOME` set to the guest home, bound to
loopback on a port chosen per seat, never the owner's `voice-serve`; its
workspace jail is the seat's `workspace/`, a sibling of its home, so the
guarantee is structural rather than resting on a seat having no file tool
(§4a). A voice
seat's audio runs through the owner's running voice worker — the one worker
on the box, its journal the owner's (S8) — which is part of why a live owner
call makes a voice seat `busy` (§5): one worker's state, not only GPU room.
The worker sees a seat only through the wiring's loopback relay of frames
the listener has already validated (§2); it never holds a socket that
HyperStudy opened.

**Confinement: bwrap with no network, and Unix sockets as the only way
out.** Nothing confines a whole mecha process today — `shell`, MCP servers
and two document helpers are wrapped, and experiment trials, the closest
precedent, get a cleared environment and their own home but no jail. The
listener (§2) and the guest facade are the first, both through
`Sandbox::wrap_argv_with_env` (built for long-lived MCP children). The seat directory is the bind root, because
bwrap's bound directory must include `run/` and its sockets. The path jail
is narrower: `ToolCtx::resolve`'s root is the `workspace/` inside it.
`MECHA_HOME` goes in the environment, since bwrap clears it.

bwrap's network is all or nothing — with the host's network the jail
reaches the LAN and the tailnet, and with `--unshare-net` it reaches not
even the loopback router. The way through is that a *pathname* Unix socket
crosses a network namespace. The wiring, outside the jail, creates
`seat/run/model.sock` (forwarding to the router on `127.0.0.1:8080`) and
`seat/run/parent.sock` (to itself) inside the bound seat directory; the
facade runs under `--unshare-net` and reaches the model and its parent
through those and nothing else. The facade's clients either take a socket
path (reqwest's `ClientBuilder::unix_socket`, which the locked 0.12.28's
source shows under `cfg(unix)` with no feature flag; read, not yet
compiled, so step 4's first act is the compile check that settles it) or keep
their `http://127.0.0.1` URLs through a forwarder on the jail's own `lo`.
The first is cheap on the worker side and not on the facade's: the
provider's HTTP clients are built centrally with no socket parameter
(`provider/mod.rs`), and `base_url` is URL-typed through
`provider::router`, so a socket would mean a new provider field (two
edits, `Config` and `ConfigLayer`) and a placeholder `base_url` such as
`http://seat/v1`. The forwarder needs no core change, since the entry keeps
`http://127.0.0.1:8080`, but it is not free: it runs inside the
`--unshare-net` namespace, and bwrap execs one child, so it is a second
in-jail process with its own lifecycle and its own preflight leg, failing
the seat closed if it is not up. Which one is built is decided on cost measured
in step 4. Either way `follow_loaded = false` leaves
`router::followed_bases` empty, so a confined facade sends no `/props`
probe into a namespace that cannot answer it. The result is a seat with no route to any
host, so egress no longer rests on the tool catalogue alone (§4a).

**The bridge runs both ways, and every seat needs the other way.** Those
two sockets are the facade's way *out*; every seat also needs a way *in*,
because the facade is served, not a client. One client is the wiring,
which relays the session's context, participant turns and tool results
to it over HTTP: reqwest's `unix_socket` again, the same check step 4
makes first. The worker is a second client, unconfined and the owner's:
completions go to `FACADE_URL` (`MECHA_VOICE_LLM`), and the affect and
direction reads to `AFFECT_URL` and `DIRECT_URL`, derived from it at
import. A facade listening on loopback inside `--unshare-net` listens on
the jail's own `lo`, which the host cannot reach, so without a third leg
nothing reaches the seat: neither the wiring's relay, nor the worker's
reply, affect read or direction call. The third leg is the same pathname
trick reversed: the facade listens on `seat/run/facade.sock`, created
inside the bound seat directory by the facade itself, and the wiring and
the worker connect to it from outside. It lands with the confined
launch in step 4, not with voice in step 6. The worker's side is real work, not a URL change. Its `httpx` (0.28.1 in the worker's
environment) takes a socket through `AsyncHTTPTransport(uds=…)`, which
covers the affect and direction clients. Completions go through Pipecat's
`OpenAILLMService`, whose `create_client` (pipecat-ai 1.7.0,
`BaseOpenAILLMService`) builds its own `DefaultAsyncHttpxClient` and
accepts no client from outside — so a seat uses a subclass that overrides
`create_client` to hand the OpenAI client one on that transport. The
request URL keeps a placeholder host (`http://seat/v1`) that names the
request and routes nothing. `parent.sock`'s closed message set is not
widened to carry any of this: it is the facade's channel to the wiring, and
completions are the worker's channel to the facade, a different direction
with a different peer.

**`parent.sock` is pinned like the catalogue.** It is the one channel from
the process holding a stranger's conversation into the process holding the
owner's home, the router hold and the crisis ledger — and §7 needs it, since
the facade's bind root is its seat directory, so the owner's ledger is not
in its filesystem to write. That holds in §9's filesystem-only interim
too, which drops the network namespace and keeps the jail, so the crisis
record's message on `parent.sock` is needed either way. So
the wiring accepts a closed set of messages on it, checking a ClientTool
call's name and arguments against the catalogue again on arrival, not only
at registration: a ClientTool call (and
returns its result), a crisis trip record, a `CrisisState` change, and seat
close. Anything else is refused and logged, failing the seat closed, never
ignored.

The alternatives, weighed: Landlock (already a backend) can allow TCP
connect by port, but UDP is restrictable at no ABI — which is why
`can_reach_network()` is unconditionally true for it in `sandbox.rs` — so a
landlocked seat remains a route out whatever its TCP rules; and TCP rules
go only by port — `8080` would also allow
`<any LAN host>:8080` and the tailnet's — and it leaves `/proc` readable,
the owner's processes' environments included. Docker's `--network none`
needs the same socket bridge plus an image and a daemon. Filesystem-only
bwrap with the host network is the cheaper first step if the bridge is not
ready; whether step 4 ships the bridge or that interim is owed (§9), and
the preflight of every leg belongs to the bridge. The interim needs no
`facade.sock`: with the host network the facade's loopback is the host's,
so the worker's URLs reach it unchanged — the third leg is a cost of the
bridge, not of the interim. **What the interim gives up** is the sentence
that closes the bridge paragraph above. With the host's network the facade
reaches the owner's loopback services, the LAN and the tailnet, as every
other mecha surface does, and the closed `Blind` catalogue (§4a) is then
the only thing between a stranger's conversation and those hosts; no
config switch stands second, since `block_private_ips` guards tools a seat
does not have (above). So §9's choice is between two
described postures: no route to any host, or a route held shut by the
catalogue alone. Step 4 says which tests each one ships with.

Three things the build must not miss: the `mecha` binary and any model or
cache file the facade reads live under the owner's home, which bwrap does
not bind, so they are named read-only; `bwrap` is resolved through `PATH`,
and this box's `/usr/bin/bwrap` cannot create user namespaces under its
AppArmor setting (the local `/usr/local/bin` build can), so the launch names
the binary it preflighted; and the preflight proves every leg — the model
answers through `model.sock`, the wiring answers through `parent.sock`, a
voice seat's facade answers the worker through `facade.sock`, and a LAN
address does not answer at all. A confinement that is configured and fails stops the seat, never
falls through (the silently-degrading guard).

**The interlocutor is untrusted — a run mode, not a prompt:**

| Surface | Owner's persona chat | Guest seat |
|---|---|---|
| Who the human side is | the owner | a stranger. Inbound text arms `untrusted` today only through harness-written stems (`DIGEST_STEM`, `persona::files::FILES_STEM`, `persona::recall::stem_of`, `scene::stem_of`), on the premise that the user chose every word (`Taint::arm_for_content`'s comment; it can arm `private` off its stems, §4a), and `external` is a tool result's field, not a message's — so a seat sets `Conversation.taint.untrusted` at construction, and the conversation records it, so the seat is armed from the first word |
| System prompt | `BASE`, the card, plus `DISCLOSED` while `disclosure` is on | HyperStudy's `instructions`, verbatim, and nothing of mecha's persona text (S10), `SINCERE` among it — the persona prompt's one unconditional line (a sincere "am I talking to a person?" gets the truth) is left out because whether an agent answers that truthfully is a disclosure decision, the study's by S2; a study that wants the floor writes it into its own prompt |
| Persona memory | written and recalled | neither: no writer, no recall over owner stores |
| Confirmations, outbox | spoken yes can release | none exist: no outbox route, no `parse_answer` path |
| Approver | the owner's | allows **exactly the ClientTool catalogue** at `approve` — the loop sends every tool that is not `read_only` there, and the answer tools and `send_chat` genuinely act — and refuses every other tool there, and every other door that does not delegate into it (`escalate`, `escalate_access` through it, and `consult` by its own fail-closed default). `permit` delegates to `approve`, so it can allow nothing `approve` would not — and a seat writes no `[approval]` table for it to fire on. Catalogue membership, checked in mecha's code, is what makes a tool runnable, the same place §4a earns `Blind`; no catalogue tool claims `read_only`. What holds a future door is the trait's defaults, which fail closed (`escalate`, `consult` → `Blocked`) or delegate (`escalate_access` → `escalate`, `permit` → `approve`); so the guest approver overrides `approve` and `escalate` and nothing that delegates. Every refusal it returns is `Decision::Blocked`, never `Deny`: `Deny` renders as the owner's correction ("Denied by the user: "), which the learning miner reads from the result string and compaction preserves across a cut, and no human speaks in a seat. A test listing today's doors cannot catch a new one with a default body — exhaustiveness the compiler checks would need a door enum on `Approver`, named in §8 step 4 as a choice, not assumed |
| Grants | `[[grant]]` and `/grant` | off by type, as an incognito chat's are |
| Tools | the persona's eligible set | HyperStudy's declared tools only, as ClientTools — mecha registry tools whose call goes out over the session and whose result HyperStudy sends back (§4a); none of mecha's own. A text decision passes the request's declared tools to the model as declarations and runs none (§2a) |
| Transcript | the chat's session file | kept in full (S8): the seat's session file, with every record a run writes — tool calls, ClientTool results, context items, spoken directions — and the seat's guest home is left in place when it closes |
| Voice worker logging | an ordinary call's journal | the same: a seat is an ordinary call to the worker, never incognito |

**Where a seat is kept.** The guest home is built under the owner's home at
`hyperstudy/seats/<seat>/` and left there when the seat closes — the
experiment store's pattern, a trial's home kept as its record. A text
seat's transcript lands at the same depth, `home/sessions/`, in a home that
holds nothing else, so every reader and the guard below see one layout.
The seat's manifest names the study's session, the persona whose voice it used, the
model `served_props` reported, and the HRP settings it ran under, so a seat
can be read back the way a trial is. Seat transcripts stay out of the
owner's run corpus and reflection mining by a recorded fact, not only by
where they live: a seat's session carries its own `SessionKind`, and the
corpus `Scan` excludes it as it excludes `Test` and `Experiment`. The
variant alone is not enough, and the reason is across binary versions: a
reader built before the variant exists loads the unknown kind as `None`
(`de_lenient_kind`), and `Scan::admits` admits `None` like every kind it
does not name — so the variant guards readers built after it, and the
location guards the ones built before — which is why the location stays
the guard every reader honours (PERSONA-DESIGN §3.2), and the kind is the
second check. Their location (outside `$MECHA_HOME/sessions`, which the corpus
reads without recursing) is the one every binary honours, old and new,
and the kind is what still holds against a recursive reader added later,
so neither guard is alone.
The repo's own guard must see them too: `scripts/check-private.py` refuses
a commit that quotes a conversation, but it reads explicit, non-recursive
globs (`personas/*/sessions`, `sessions`), so a seat's
`hyperstudy/seats/*/home/sessions/*.jsonl` would be invisible to it — and
participants' words, held under someone else's consent and kept in full,
are the case the guard exists for. Step 2 adds the seat glob to the script's `whole` list, the one it reads
entire. Its `sessions/` list reads a file whole only when its header's
`kind` is `voice` and otherwise only for spoken turns, and a seat's new
`SessionKind` matches neither, so a text seat's transcript joined there
would contribute nothing while the guard looked applied. The glob follows
the depth a seat's sessions actually land at, and anything else the seat
home writes conversation text into joins it. Retention of these records is the
owner's (S2 hands participants' consent to the study); nothing here deletes
them on a clock. mecha declares to HyperStudy that the engine keeps
transcripts through `hs.retention` once HyperStudy reads it, so a study can
tell its participants. That is an HRP extension in HyperStudy's design, not
in spec 0.2.0, and a voice session's alone; a model endpoint has no channel
for it, so it is on HyperStudy's list (§8).

**The study's prompt is the whole prompt (S10).** HyperStudy builds the
agent's identity, objective and per-state guidance into `instructions`, and
sends a `session.update` replacing them when the study's state changes.
mecha uses them as the system prompt verbatim, which makes `instructions`
a trusted slot on a socket that also carries participant text (the
study's crisis resources are the other, §7): the
contract with HyperStudy is that `instructions` carry only
experimenter-authored text, and anything a participant wrote — a
free-text answer, typed chat — comes back as a quoted context item, never
interpolated into the next state's guidance (§8). Said plainly: this is
the one guarantee in the design that a remote party's discipline keeps,
not mecha's code — a study that breaks it puts a participant's words in
the system prompt silently. The blast radius is bounded by everything
mecha does enforce (no owner stores, no mecha tools, a crisis sensor no
prompt can turn off), but the contract is HyperStudy's to keep. A text
decision's system message is the same slot under the same contract: the
spec says participant text reaches a model endpoint wrapped in tags, inside
the user message. This is the `persona: "hyperstudy"` ownership of
HyperStudy's design (its §4.4), the one it builds today, with the one
difference S12 makes: the voice is the engine's (§5.2). That design
describes mecha as a `connected` engine that owns its persona (its §6.2);
under S10 and S11 mecha is neither, and that text is HyperStudy's to change.
Each replacement re-pays the cached prefix once, which is the study's cost
to choose.

**`instructions` is bounded, and an oversized one is refused.** The system
prompt is the one part of a request compaction cannot cut: the loop's only
overflow recovery is summarising `messages`, and `ContextOverflow` never
retries. So a `session.update` whose `instructions` would not fit leaves a
permanently dead seat whose cause reads as a model-server fault. The seat
refuses it at the door with a typed rejection on that `session.update`,
never truncating and never accepting it, under §4's "refused, not ignored"
rule. The bound is a share of the seat's `context_window` and is counted in
bytes: a byte-level tokenizer spends at least one byte per token, so bytes
bound tokens from above with no tokenizer call, and the bound errs toward
refusing. The share is set with the budgets, from a measured seat (§9).
The crisis-resources slot already has its own bound and fallback (§7);
this is the other trusted slot getting the same treatment. The
only harness text a seat adds is what the loop needs mechanically (tool
results, the context items of §4a, and the clock), never a claim about who
the agent is or who it is talking with. The clock needs saying: the
calendar note renders only when the system prompt carries
`date_context::GUIDANCE_HEADING`, so a verbatim study prompt would earn no
date on any request — the "Sunday on a Monday" failure by construction.
The note's vehicle is right and stays: it is attached at wire time, once
per request, never persisted, so it cannot go stale across midnight or
accumulate in history — which a context item folded into the conversation
would do. What changes is the gate: a seat-mode allowance beside the
heading test, so a seat's requests carry the head note, with a one-line
label saying what it is, rendered in the seat's `UTC` (above) rather than
the owner's zone, while the system prompt stays the study's byte for byte.

## 4. The voice transport

What the HRP transport must do, by protocol area (the events themselves are
HyperStudy's to define, in the spec's voice engine page):

- **Connection.** HyperStudy opens one WebSocket per run to
  `wss://<machine>.ts.net:10000/v1/realtime`, with `Authorization: Bearer
  mhs_…` and `OpenAI-Safety-Identifier: hs-<opaque>`, an opaque per-session
  label and never a participant's identity. That label names the seat, and
  a connection without one is refused `policy`. Today HyperStudy dials
  `openai` only (`realtimeTarget`). Its `custom` provider is planned for its
  Phase 2 (`voiceConfig.js`) and needs its SSRF guard extended to
  WebSocket, so it heads HyperStudy's list (§8).
- **Session.** Accept `session.update`; map `turn_detection` (below);
  `instructions` to the system prompt (§3), and a later `session.update`
  carrying only `instructions` replaces it; `tools` to ClientTools, declared
  once, which the spec now promises. `audio.output.voice` is accepted only
  as null (`preset: null`, since the persona's voice is the engine's, S12)
  and refused otherwise; HyperStudy already sends null for a persona with
  no preset (`voiceAgentSession.js`), which is what the export sets (§5.2).
  `audio.output.speed` is accepted only when absent, which is how
  HyperStudy sends a persona with no speed (`sessionUpdate` omits it), and
  the export sets none: the rate is the persona's `voice_speed`.
  `instructions` over the seat's bound (§3) are refused. `transcription` is accepted only as null: mecha transcribes
  every committed turn with Parakeet anyway, and emits
  `conversation.item.input_audio_transcription.completed` for each turn
  whatever the field says, since HyperStudy's conversation record reads it.
  A transcription model by name is refused, since it is not one mecha
  runs, and HyperStudy's list asks it to send null to an engine that
  transcribes on its own. A field mecha does not support is **refused, not
  ignored** — HyperStudy's conformance asks for exactly that.
- **Silent forced answers.** HyperStudy asks an on-screen question with a
  `response.create` whose `output_modalities` is `["text"]` and which forces
  one named tool (`tool_choice: {type: "function", name}`); its suite's
  `silent_tool` check fails an engine that speaks or skips the call. So that
  response produces no audio, and the forcing must not change the tool
  list — the cached prefix (§4a) — whether through llama-server's
  `tool_choice` for a named function, checked on the installed build rather
  than assumed, or `required` with any other name refused.
- **Audio in.** `input_audio_buffer.append` (PCM16, 24 kHz, mono) into the
  existing Silero VAD → SmartTurn v3 → Parakeet chain.
- **Turns, engine mode (S4).** Two modes, both certified by HyperStudy's
  suite (version 6 runs every turn-dependent check once per mode, at
  `server_vad` with 500 ms of silence and `semantic_vad` at `auto`):
  - `server_vad` — `speech_started` on the VAD edge, the reply in flight
    self-cancelled when `interrupt_response` is set, `speech_stopped` at the
    VAD stop, `committed` at end of turn. `threshold` maps to Silero
    confidence, `silence_duration_ms` to the VAD stop; `prefix_padding_ms`
    is approximate.
  - `semantic_vad` — SmartTurn's hold; `eagerness` maps approximately.
  This differs from the owner's calls on purpose: there, a turn starts on a
  transcript (`TranscriptionUserTurnStartStrategy(use_interim=False)`); here
  the protocol pins the VAD edge.
- **The listening pause.** When the study moves to a screen without the
  video chat, HyperStudy sends a `session.update` with `turn_detection:
  null`, then `input_audio_buffer.commit`. Switching detection off ends the
  turn in progress. The commit keeps what was said so far as that same
  participant turn, transcribed and recorded, with no reply: a turn the
  facade parks without a run (§4a's context endpoint). No audio arrives
  while detection is off. When it comes back on, the next speech is a new
  turn and the audio clock restarts. `audio_start_ms` counts from the
  moment detection came back, with the first appended audio at 0. In the
  worker that edge resets Silero's and SmartTurn's state and the
  transport's sample counter. The suite checks it twice, mid-speech
  (`listening_pause`) and in silence (`listening_pause_silent`).
- **Audio out.** Breeze, streaming, resampled to 24 kHz as
  `response.output_audio.delta`, each delta at most 2000 ms of audio (the
  spec ends the session on a larger one), with the spoken text as
  `response.output_audio_transcript`. The director runs as in any call, and
  its directions are recorded in the seat's transcript (S8).
- **Truncate.** `conversation.item.truncate` cuts the reply's text at the
  heard point proportionally to audio played: grade `approximate`.
- **Context items.** The spec's `[context]` events arrive as user text
  items, each holding one JSON object, with images attached once per
  session as `input_image` data URLs: `participants`, `screen`, `question`,
  `typed` (chat a participant typed, quoted), `timer`,
  `experimenter_message`, `revealed`, `paused` and `resumed`. All are
  untrusted, like speech, and all fold through §4a. A `paused` seat is cut
  off by HyperStudy until `resumed`, so mecha treats both as context and
  enforces nothing of its own between them.

**Conformance, suite 6.** HyperStudy starts a voice agent only on an engine
and model that have a stored, passing report at the current suite version.
So every suite bump is a re-run against this box. The suite is HyperStudy's
to run; it is not yet packaged for engine authors. Two of its demands bear
on what is resident here:

- `image_context` asks the engine, silently, which colour fills a solid red
  image, so the router's model must see images (`vision` in the seat's
  config, §3).
- Its thresholds are 1000 ms to onset, 5000 ms to the start of a response,
  and 1500 ms from cancel to done. On a cascade these are this box's to
  meet, and §9's latency item measures them before a study depends on them.

The report names a model. The seat's pinned model and the router hold (§5)
are what keep that name true for a seat's whole session.

## 4a. What a seat can do (S5)

This section is a voice seat's. A text decision runs no tool and holds no
conversation (§2a): its one action is the tool call it returns, which
HyperStudy carries out. A voice seat is mecha's agent loop with no tools of
its own: everything it reads and everything it does goes through the
session HyperStudy opened, so the only way out of a seat is the session it
answers. The sealed home keeps the
*owner's* data out; it does not keep the private leg from arming. An image
arms `private` wherever it sits in a conversation
(`Taint::arm_for_content`), and with the stranger arming `untrusted` from
the first word, a seat is fully armed from the moment its first study
*image* lands. A text-only study can arm `private` too, since
`arm_for_content` arms it off its stems in user-role text, which a
participant can type; nothing here assumes a study without images stays
unarmed, and it costs nothing while every catalogue tool is `Blind`. The image case needs saying at the fold (below), because an image the loop
did not produce is otherwise counted only from the next run. That is
correct — the interlock cannot know a study image from the owner's — and it
makes what a ClientTool declares the question that decides whether a seat
can act at all.

| Capability | How | Needs from HyperStudy |
|---|---|---|
| Talk by text | a text agent on HyperStudy's model endpoint, one model call per decision (§2a); typed chat in a voice seat arrives as a `typed` context item and gets its reply by the study's `response.create` | a per-run identifier on model-endpoint requests (§2a) |
| Talk by voice | §4 | the `custom` realtime provider (its Phase 2), with its SSRF guard extended to WebSocket |
| Read the study's text | screen text, instructions and typed chat arrive as quoted context items, untrusted like speech | — (sent today as `[context]` items) |
| Read images | a study image or one a participant shares arrives inline as an image content item and reaches the model as `Block::Image`, capped at the door as every image is; mecha never fetches a URL a session names | — (inline since spec 0.2.0, each image once per session) |
| Act in the study | **ClientTools**: HyperStudy declares its study tools — `send_chat`, and the answer tools for on-screen questions (`answer_choice`, `answer_choices`, `answer_scale`, `answer_ranking`, `answer_text`, `answer_dimensions`); advancing the screen is not an agent capability in HyperStudy yet — the call goes out over the session, `function_call_output` comes back marked `.from_outside()`, a timeout returns `is_error` — so the agent loop never learns where a tool came from. A timeout is the case §4a's restore cannot see: the run did not fail, and the answer may be in the study's data with only its acknowledgement lost. So a timed-out answer tool's `is_error` says the answer may already be recorded and must not be sent again, and the proxy answers any later answer to that same on-screen question "Not run" while that question is current, in that run and every later one. It is keyed on the on-screen question, never on the tool. Spec 0.2.0 gives a question no identifier, in its `question` item or its answer tools, and an answer names no question: it answers the current one. So the key is the `question` context item itself, numbered by the seat as it arrives, and the lock holds while that item is the current question; a second question on the same screen arrives as an item of its own and is answerable. HyperStudy's backstop needs an identifier of its own to key on, which is on its list (§8), and once one exists the guard keys on it instead. Never on the tool, because the answer tools are generic by kind, so a guard keyed on `answer_scale` would leave a second scale question on that screen unanswerable with no unlock coming: the participant's next turn is a new run, and the likeliest moment to answer again. So the guard is conversation-scoped state held by the seat's study tools themselves, not run-scoped like `RunPictures` (which shares only its "Not run" shape). It survives a cut by construction, since compaction rewrites the messages and never the registry; `Tool::carried_state` only restates the answered questions in the rebuilt prompt so the model does not retry blind, and step 4's test asserts the refusal itself after a compaction, with the carried block as the secondary check; the tool list never changes, so the cached prefix holds. A timed-out `send_chat` says its line may have been shown. A second answer is a fresh call with a fresh `call_id`, so HyperStudy's backstop (§8) is keyed on the screen's question, not the call | — (declared once per session, as the spec now pins); a question identifier, for HyperStudy's own backstop |

**Where context lands.** Screen changes, typed chat and images arrive on
HyperStudy's clock, including while a ClientTool call is in flight — and
there is no legal slot between a `tool_use` and its result, nor for two user
messages in a row (images are user turns only). So context items queue and
fold at the next legal boundary through the steering path
(`ARCHITECTURE.md` §Interruption and steering) mid-round, into the message
carrying the tool results. The steering path alone is not enough: its one
drain site skips a closing turn by design — a steer taken there could not
be acted on — and leaves what becomes of an unread one to the host. Since
#731 both chat hosts give each a turn of its own, in the order typed
(`chat::next_turn`, `PersonaChats::next_turn`). A seat's host decides too,
by folding at a boundary of its own, and that boundary is in the facade
child, not the wiring. The conversation
lives there, in the seat's `SessionHost`, as voice's facade keeps it
server-side today (`Slot` holds `convo`, and the handler calls `run_in`
and `roll_back_failed_turn` in-process). The wiring only sends items to
it. It is a **deferred** fold: `run_in` holds `&mut Conversation` for the
whole run, so even the host cannot touch the messages while a run is in
flight, and a message already sent must not change — the transcript would
record words the model never received, and the next request would stop
extending the last one's prefix. Four cases:

- **A run is in flight** (a closing turn, or a reply with no tool call):
  the item waits, and cannot be acted on in that turn — the consequence the
  steering drain already accepts. When `run_in` returns it is handled as
  below.
- **The conversation is empty, or ends on the agent's reply:** the item
  (for an empty one, the opening screen's text, the first event in every
  seat) opens a new user
  message of context alone. This case and the next need a facade path that
  *parks* an item in the conversation without starting a run. Today every
  facade POST runs a turn, and a body with no user message is a 400. So
  the seat's facade gains a context endpoint (append, no run), and §8
  step 4 builds it.
- **It ends on an unsent user message** (context with no reply yet, or a
  participant turn not yet sent; unsent means the host parked it since the
  last run returned, a mark the host keeps, since `Conversation` does not
  record what was sent): the item is appended to it, and a
  participant turn arriving on a context-only tail is appended to that
  message too, so two user messages in a row never occur.
- **The run failed, or stopped on a ceiling:** `run_in` does not roll back
  on `Err`, so the tail is a tool-result message or a user message already
  sent — neither case above. A ceiling (`max_turns`, a budget, loop
  detection) is the same tail with `Ok`: it folds `FINAL_ANSWER_NUDGE` into
  the tool-result message and sends it, and if that request errors or is
  interrupted the run returns with no reply after it. §3 pins `max_turns`
  so that this is a runaway seat turn's designed exit, so it is expected,
  and the host tells it from the case above by its mark, not by the tail's
  shape. A cancelled run (a barge-in through `interrupt_response`) also
  returns `Ok`, with one of three tails: a partial reply the loop kept,
  which is the second case; a completed tool round, which is this one; or
  the participant turn just sent with nothing after it, also this one.
  It never takes the voice host's pop: `voice/mod.rs` pops a plain user
  tail because its front end resends one, which a seat's never does, and
  popping a tool-result message would orphan its `tool_use`, the 400 that
  pop once wrote into a slot and every resume of it. §8 step 5 tests a
  barge-in mid-tool-round. The seat restores to the **last complete round**, not to the
  snapshot taken before the run. A round whose ClientTool calls already ran
  has had effects outside mecha: a rating written into the study's data, a
  chat line shown to the participant, each acknowledged by
  `function_call_output` before the failure. Restoring to before the run
  (the first half of `Conversation::roll_back_failed_turn`) would erase
  the record of them, and the next request would ask again: two answers
  for one screen. That is CLAUDE.md's "a retry must never duplicate work"
  one layer up from the provider. So the restore keeps every message up to
  and including the last tool-result message whose results all exist,
  which is legal because every `tool_use` there has its result, and drops
  only what followed it, the partial reply. When no ClientTool ran in the
  failed run, that point *is* the pre-run snapshot. Either way the
  helper's second half (popping a plain user tail because a front end will
  resend it) is never taken. A seat has no front end: HyperStudy sent that
  context and that participant turn once, so popping them would lose them
  with nothing to record their loss. After the restore the tail is either
  an unsent user message (the third case) or a kept tool-result message. A
  later item or participant turn is appended to that message as the
  steering path appends to one, so the next request extends the kept round
  and the model sees what it has already done. Changing a message the
  failed request carried costs that one request's prefix tail, and the
  rewrite records it. Nothing will resend it. The restore is a new helper
  beside `roll_back_failed_turn`, named in step 4. The rolled-back state is
  recorded as a rewrite in the seat's transcript (the helper's contract,
  and the transcript is kept in full, S8), followed by a `Record::Taint`
  with no message between, as every failed-turn arm writes today:
  `TaintTimeline::from_records` clears its checkpoints on a `Rewrite`, so
  without it the record would lose that the seat was armed `untrusted`
  from construction and `private` from its first study image. Whether any of
it asks for a reply is the study's, by its own `response.create`. Nothing is dropped: an item still queued when the
seat closes is recorded as unfolded in the seat's transcript. The
mid-round half is not free either: the steering queue carries text
(`queued_input` is a `VecDeque<String>`), and an image context item needs it
to carry `Block`s. That widening is a core change that touches every
steering caller, the TUI and web chat included, and §8 step 5 names it.

**A folded image arms at the fold, not at the next run.** Taint is
recomputed at run start (`Agent::arm_at_start`, over the messages and the
notes), and the loop's one mid-run arming site is the tool-result image
branch, which exists for exactly this reason: so the pixels "are counted
from the turn they arrive — the same answer, a run sooner". A context
image folded mid-round through the steering path passes neither. Left as
it is, a send later in that same run is judged on the taint from before
the screen arrived, which matters when the owner has
`block_sends_after_private` on and in what the session records. So the
drain that folds steered `Block`s arms them in the same place, through
`arm_for_content` over the folded blocks, beside the tool-result branch.
The host's deferred fold needs nothing new: its folds land between runs,
and the next run's `arm_at_start` counts them before anything is sent.

**Compaction.** A long voice seat reaches the slot's `context_window`, and
compaction applies as it does anywhere: the cut lands on an assistant
message, taint survives it, and the session file records the rewrite. The
study's `instructions` are the system prompt, not a message, so a
`session.update` never falls inside a cut; deferred context items wait for
the run that compacts to return like any other. "Nothing is dropped" means
no context item is lost unrecorded — compaction's summary of earlier turns
is the seat's history, as it is the owner's.

**What a ClientTool declares.** An armed loop refuses every
`Egress::Chosen` tool, so a study tool labelled `Chosen` fails the first
action after the first image; and `mcp.rs` records why a wire-declared tool
can never be labelled anything else — "a remote tool's input schema is the
server's to write", so nothing can prove it holds no destination. The way
through is the one CLAUDE.md already names: `Blind` is earned in code by a
tool this repo can read. So ClientTools are a **closed catalogue in mecha's
code** — `send_chat` and the six answer tools above — each with the
schema HRP pins for it, each `Blind` because its one recipient is the
session the seat answers (the party every reply already reaches) and its
schema has no destination field. Each one's `Capabilities` are pinned whole,
not only on the egress axis: `untrusted_input` (its result is the study's,
marked `.from_outside()`), never `private_data` (it reads none of the
owner's stores; a study image arms `private` by its content, at the fold,
whatever a tool declares), never `destructive`. A tool HyperStudy declares that is not in
the catalogue, or whose schema differs from the pinned one, is refused at
`session.update`, never registered as `Chosen` and never relabelled. A new
study tool is a catalogue entry in a reviewed change, on both sides. What
HyperStudy does past the session with a rating is the study's; what mecha
guarantees is that nothing it sends goes anywhere else. The private leg a
seat carries is the study's own material shown back to the study, but the
send-axis leak guard (`block_sends_after_private`, which refuses `Blind`
too) is a `[security]` switch — a machine fact and the owner's standing
word. The wiring takes `[security]` field by field (§3; §8 step 4 tests
it): the owner's value where it narrows a seat, the strictest value where it
would widen one, with one departure. `block_sends_after_private` is the
one field where the owner's default (off) is the *wider* value, and it is
taken as the owner's rather than as the strictest, until §9's ruling. So it
binds a seat when the owner has it on, while `trifecta` is pinned to `block`, its strictest value — an owner's `allow`,
set for trusted content on their own network, never reaches a stranger's
seat, and neither does `ask`, which a seat would otherwise refuse only
because its approver overrides `escalate`, a second mechanism agreeing
rather than this floor — and `mark_untrusted_output` is always on. A seat's floor is a
property of the seat code, not of the operator's config. With
it off (the default) the catalogue works as above. With it on, a seat
cannot act after its first image, and a study that declares study tools
would lose its answers mid-run, one refusal at a time. So the seat refuses
that `session.update` with `policy` and the reason, before the
participant's first word, and only a study that declares no study tools
runs, conversation only. A text decision runs no send on this box (§2a),
so the switch has nothing there to refuse; whether it should bind a text
seat anyway is part of the same ruling. Exempting the catalogue from an
owner's guard would be an owner's ruling, not a config value (§9).

**`Blind` reduces risk; it is not safety** (`EGRESS-DESIGN.md` §6). Blind
calls compose and nothing bounds their volume, and here the payload is
model-authored prose (`send_chat`, `answer_text`), not a sixty-byte query.
The bound is the per-seat budget below, in bytes as well as rounds.

**Not in a seat:** image generation (S5); any mecha tool — no `web_search`,
no file reading, no `http_fetch`, no shell, no owner `[[mcp]]` server, no
grants. Adding one later is a ruling, and the bar is that it reaches no
destination a stranger could aim and no content a stranger should read:
`http_fetch` would make this box a fetcher pointed at the owner's LAN and
tailnet. A file tool aims nowhere outside, and by §3's layout its jail is
the seat's empty `workspace/`, which holds no audio and not
the transcript — so it would be dead weight rather than a risk; the bar
for adding one is that the study has material to put there, and what that
material may contain.

**The tool list is declared once.** mecha's tool list is the front of its
cached prefix, so a study that swaps tools per screen re-pays the prefix on
every screen. The cheaper contract, asked of HyperStudy: declare every study
tool at session start, and let each state say which are available; a call to
an unavailable one is refused by HyperStudy with an error the model reads.

**Per-seat budgets** bound what a stranger can spend, and they come in two
kinds. Tool rounds per turn and bytes sent through ClientTools answer with
an `is_error` result the model can talk around, never a dead seat. Turns
per seat and the seat's wall time cannot work that way — a cap whose only
effect is an error the model talks around bounds nothing, since a turn is
the participant's to start — so when either runs out the wiring ends the
seat with a reason sent to HyperStudy, the shape §9 owes for the
owner-call and model-switch collisions. A text seat has the same two kinds,
counted in decisions rather than runs: past its bound each later decision
is refused `policy` with the reason (§5), since there is no session to
close.

## 5. Admission

HyperStudy opens a seat by calling. A voice seat is one WebSocket for the
whole run; a text seat is a series of decisions (§2a). Each is admitted
only when every check passes, and each refusal is typed (`unknown_persona
| busy | capacity | policy`). It comes back in shapes HyperStudy already
reads: an HTTP status with an OpenAI-style JSON error body, or for a voice
seat HRP's `error` event and a close. So HyperStudy's account of why a room's agent could not start
names the reason to the experimenter.

- **The key names a servable persona.** A persona is servable when a key
  names it, and only the owner mints one (§5.1). A request whose `model`
  names a persona its key does not is `unknown_persona`, and the rest of
  the owner's personas are never confirmed to anyone. Since a seat
  takes only the persona's voice and picture (S10, S12), servable means
  "this voice and this picture may appear in studies". That also settles PERSONA-DESIGN §15's line that a romantic
  persona must never reach a minor, "binding when it is designed": no
  persona's character — its template, card or relationship — reaches a
  seat at all, so a romantic one cannot reach a participant of any age by
  construction, not by a check on the key. Its picture does, by the
  owner's export (§5.2), which shows the picture before it goes.
- **The owner comes first.** A live owner call is `busy` for a voice seat —
  and an owner call that starts mid-seat does not preempt the seat
  silently; what happens then is owed (§9).
- **`max_seats`, counted where admission runs (S9).** `[hyperstudy]
  max_seats` caps text and voice seats together; a seat past it is
  `capacity`. A voice seat counts for its whole session. A text seat holds
  router slots only while a decision is in flight, so it counts then and
  not between decisions: it has no other presence on this box, because
  HyperStudy holds its state. The count is kept by the one long-running process that admits
  every seat, in the owner's home. `permit.rs` cannot do this: a seat is
  interactive work, which takes no permit by design, and each seat's guest
  home would give it a pool of its own. A text seat can hold two router
  slots while it answers, and a voice seat three: the forced-on crisis judge (§7) is screened beside the
  reply, not after it (`persona_chat.rs` spawns `judge::screen` while the
  run goes on, which is how a verdict can stop a run in flight), and a
  voice seat's director (§4) asks a direction per sentence while the reply
  is still streaming (`voice/direct.rs`: it never waits on the reply). So the
  default is set against the router's `-np` at two slots a text seat and
  three a voice seat, with one slot left for the owner — measured, not guessed (§9). Counting one would
  queue the owner's turn behind a participant's judge call, and per-request
  rates hide queue wait, so it would show only as a slow owner chat. Within the cap,
  voice seats are further bounded by GPU room for STT, TTS and the model
  beside whatever else holds the card (image and video servers start on
  demand), read from `mecha system`; one at a time until measured.
- **A seat holds the router's model in the owner's home.** The same
  per-home trap breaks `hold.rs`, and there it is a correctness bug:
  `Holds::open_default` roots at `MECHA_HOME`, so a guest facade's hold
  would land where `mecha model use` never reads, and a model switch could
  land mid-seat — one conversation answered by two models, the thing holds
  exist to prevent. So the **wiring** takes the hold in the owner's home
  when a seat opens and releases it when the seat closes; the guest facade
  never writes one. The seal stays literal — a seat writes nothing under
  the owner's home outside its seat directory — and the hold's liveness is
  the seat's lifetime, which the wiring already tracks. That widens
  `hold.rs`'s unit from a run to a seat of many runs, and its ruling has no
  time limit, so the owner's `mecha model use` waits on a participant for
  the whole session — or forces the switch and gets the mid-seat swap the
  hold prevents. Which way it goes is owed (§9), beside the owner-call
  collision. A text seat has no open and no close that HyperStudy tells
  mecha about, so the wiring holds the model for each decision; a switch
  between two decisions answers one run with two models, which each
  decision's record shows (`served_props`). Whether a text seat holds the
  model between decisions is owed too (§9).
- **The credential** is the persona's key (§5.1), checked by the listener
  against its hashes before anything else is read, and again by the
  wiring on the message the listener sends.

### 5.1 The key, and the public address

**mecha mints it (S7).** `mecha hyperstudy key <persona>…`, and the same
step in guided setup, generates a key naming the personas it serves: 256
bits from the OS random source, prefixed `mhs_` so a leaked one is
recognisable in a scan. It is printed once. The owner pastes it into
HyperStudy as the API key of the custom provider (text) and of the custom
voice engine (voice). mecha keeps only its hash, beside the personas it
names, owner-readable. `--add` and `--remove` change which personas a key
names. `--rotate` mints a replacement, and the old key stops working at
once. `--revoke` removes it, and its personas stop being reachable through
it. Revocation is mecha's act now, not HyperStudy's: a leaked key is dead
the moment the owner says so, on this box.

**The wiring is the authority for that, not the listener.** The listener
sees the hashes through a read-only bind of their *directory*, never of the
file. A file bind pins its inode, and the store is written as every store
here is, a temporary file renamed over the old (`experiment::write_atomic`'s
shape), which such a bind never follows. It re-reads them when the file
changes, so its early refusal stays current, but that is an optimisation:
the claim "at once" rests on the wiring checking the key again on every
message (§2), against the store it writes itself. A revoked key the
listener has not yet dropped reaches the wiring and is refused there.

**A key names a set of personas, because HyperStudy keeps one custom
endpoint per account.** Its custom text provider resolves a single
`{ baseUrl, apiKey }` credential for an experiment's owner or organization
(`resolveCustomEndpoint`, Settings → API Keys), so every persona that
account serves through mecha shares the account's key, and each picks its
mecha persona by its own `model` field, which the export sets (§5.2). A key for
each persona, as S11 first put it, is the narrowest case of this and the
default: a key minted with one name serves that persona alone. Choosing
among a key's personas by `model` grants nothing beyond what the owner
named for that key. `GET /v1/models` answers with the personas the key
names. The spec's connection test takes the first one listed as the model
name, and the export sets each persona's own name, so the test's choice
decides nothing. A credential per persona on HyperStudy's side would make
one key per persona possible; it is on its list (§8), and nothing here
waits on it.

**The public address, and what it costs.** S11 puts this box on the
internet at one HTTPS port. `REMOTE-SURFACE-DESIGN.md` §2 ruled that out
for the owner's own page ("Funnel is never the answer"), and the two cases
differ in what answers:

- **Behind the owner's page** is the owner's whole assistant, with mail,
  calendar and the graph. Behind this port is a listener that holds
  nothing (§2), then a wiring that admits only a valid key, then a seat
  that holds none of the owner's stores (§3).
- **A leaked key** lets a stranger talk to that persona's seat on this
  box's model. That costs GPU time, bounded by `max_seats` and the per-seat
  budgets, and it exposes a voice, a picture, and whatever the caller's own
  prompt says. Nothing of the owner's is reachable through it, and the
  owner revokes it here.
- **A bug in the listener** reaches a confined process with no network (§2).
- **What S1 kept stays true:** HyperStudy never joins the tailnet, and
  nothing on this box dials HyperStudy except the owner's export (§5.2).

The port is the listener's alone (§2's Funnel guard), and the owner's page
stays tailnet-only, as that ruling has it.

### 5.2 The export (S12)

`mecha hyperstudy export <persona>` writes the persona into HyperStudy as
an agent persona. It goes through HyperStudy's v3 API (`POST
/api/v3/personas`, and `PUT` on a later export) with a HyperStudy API key
the owner creates there (`/api/v3/auth/api-keys`) and gives to mecha. That
key is an outbound credential, and the one secret here whose theft reaches
off this box. So it is kept in the owner's home in a directory of its own,
apart from the directory of `mhs_` hashes, which is the only one bound
into the listener (§2, §5.1). It is owner-readable only, written by setup
or `mecha hyperstudy export --set-key` from a paste, and never in
`[hyperstudy]` or any config file. Only the export reads it; the listener
and the seats never see it, and no bind reaches it.
It sends:

- **The name:** the persona's `display` (its folder name when empty), as
  the HyperStudy persona's name and display name.
- **The picture:** the persona's portrait (its image-library `character`),
  cropped as the owner framed it for the avatar (`State`'s `frame`), as the
  HyperStudy persona's avatar.
- **The voice, as a binding, not as audio.** The HyperStudy persona's
  `voice` block names mecha's engine (`engine: "realtime"`, provider
  `custom`, wherever HyperStudy's Phase 2 puts its URL) with `preset: null`, which means the
  engine's own voice. The voice itself is rendered on this box, by the
  persona's voice-library voice at its `voice_speed`, and no reference
  audio ever leaves (§3). No speed is sent: it is mecha's, and HyperStudy's
  range (0.25–1.5) is not mecha's (0.5–2.0). Once HyperStudy has a field
  for an engine-owned voice's preview, a short clip of a fixed, made-up
  sentence in that voice goes with it, so its "hear a voice before choosing
  it" works (§8).
- **The text adapter:** provider `custom`, and this persona's name as the
  model, which a key that names it accepts. The base URL and the `mhs_`
  key are the account's custom endpoint in HyperStudy's settings, which the
  owner sets once by pasting (S7); the export sends neither.
- **Nothing of the persona's text.** The prompt fields are left as the
  study writes them (S10): not the card, not a relationship, not a memory.

**Export is the owner's act, one persona at a time, never automatic.** It
shows exactly what will be sent (the picture itself, the voice's name, each
field) and sends on the owner's yes; `--dry-run` shows it and sends
nothing. The avatar becomes a **public file** in HyperStudy's storage,
served to every participant who sees the agent, and the confirmation says
so in those words. A later export overwrites the picture and the voice
binding, since mecha is their source (S12), and touches no other field of
the HyperStudy persona. Whether HyperStudy's users may edit those two there
is S12's open half; until it is ruled, an edit made there is overwritten by
the next export.

## 6. The feature (S6)

`hyperstudy`, a new variant in the closed registry (`feature.rs`,
`FEATURES-DESIGN.md`) that requires `personas`: its own switch in
`[features]`, off by default and
off on every install that predates it, so nothing listens until the
owner says so. Guided setup offers it once personas are on and asks for the
two things it cannot work without: the public address, and a key for at
least one persona (§5.1), which setup mints and shows for pasting. The
address needs Funnel, which is the owner's tailnet setting (§2). Setup
turns Funnel on for port 10000 and reads the result back through §2's one
command. When the tailnet policy lacks the `funnel` attribute, tailscale
refuses with its own message, and setup shows it beside the policy line
it needs; it cannot grant it. `Unready` names whichever is missing. The export
(§5.2) needs a third thing, the HyperStudy origin and a HyperStudy API key,
and is offered, not required: serving works without it, with the
HyperStudy persona set up by hand. Voice
seats additionally need the voice worker installed — `voice`, not
`calls`: `calls` is what buys `mecha serve`'s relay to the worker (and so
requires `web`), and a seat reaches the worker through the wiring's own
loopback relay (§2), so it needs neither the owner's page nor its tailnet
listener. The registry already allows that: `requires` puts `web` on
voice's browser parts only (`dictate`, `calls`, `cloning`), and `voice`
itself stands alone (FEATURES-DESIGN F3: `mecha voice-serve` is its own
loopback surface), so no registry edge changes. `hyperstudy` requires
`personas` only, and the wiring checks `voice` at runtime, when it
answers a voice session's open over `listener.sock` (§2): on a machine
without it the open is refused `policy`, and text seats work. The listener
cannot check it and does not, since it holds no part of the owner's home
but the hash directory. A
`requires` edge on `voice` would block text seats on a machine without it.

The coupling to `mecha serve` is in code, not in the registry, and the
seat work has to undo it: persona calls are hosted only inside serve
(`VoiceHost` → `PersonaChats`, mounted only when `calls` is on), and the
worker reads its facade URL once, process-wide (`FACADE_URL` from
`MECHA_VOICE_LLM`). So a guest facade is built on `voice`'s `Facade` and
`Mount` with a seat's own `SessionHost`, which means exposing them beyond
serve. Two `Mount` fields are pinned for a seat, not inherited from
`voice-serve`'s mount. `approve_all` is `false`: it is the owner-present
posture, a mounted facade allowing every call, and a seat's approver is the
catalogue's (§4a). `inject_voice_block` takes neither existing value,
because both name the owner, and it is a `bool`, so the third value means
widening the field to an enum. That touches `mecha serve`'s and
`voice-serve`'s mounts too, which keep their two values. `false` writes no
note at all (only `true` appends one; `VOICE_BLOCK` reaches a prompt
through `voice-serve`'s `system_extra`, which a seat does not set), so a
voice seat would speak with none of the streaming guidance. `true` adds the per-request
voice note (`VOICE_NOTE`), which says the reply is read "to the owner, who
hears it". A seat needs a third spelling, derived from `VOICE_NOTE_STREAMING`, not
`VOICE_NOTE`. The two are chosen per call by engine, a seat streams (§4),
and the streaming note carries no length rules by the owner's 2026-10-03
ruling. So the seat's note is the streaming note's guidance with the
listener left out, for the reason `BASE` is left out (§1), and it
re-imposes nothing that ruling removed. It is a per-request note like the
clock, never in the system prompt. The worker takes the facade's address per call — a URL for the
owner's calls, `seat/run/facade.sock` for a confined seat (§3) — and with
it the affect and direction endpoints derived from it at import
(`AFFECT_URL`, `DIRECT_URL`), carried by the wiring's relay as a trusted
field the way the persona voice binding is today; and
the voice binding check that `offer_proxy` does for a browser call moves
into the seat wiring. Building it follows `ARCHITECTURE.md` §Adding a
feature item by item rather than a subset copied here; the one choice the
checklist leaves to the feature is made here: `switchable_from_environment`
is **no**, because the credential lives in the owner's home and an
environment that could switch `hyperstudy` on would be a trial home
answering the internet.

## 7. Mechanisms, not policy (S2)

mecha provides; the study decides:

- **Crisis handling.** The harness-side crisis sensor (`persona::safety`,
  keyword tier plus model judge) runs on every participant turn — **forced
  on in a seat**, whatever the named persona's `safety.crisis` switch says:
  that switch is the owner's choice about the owner's chat, and it does not
  travel with the voice into a study's. Nothing here is inherited: the
  sensor runs in one place today, interleaved through `persona_chat.rs`'s
  handler — the keyword tier, the judge and its answered state, the
  cooldown, `record_crisis`. (`voice/direct.rs` and `listen.rs` only
  recognise `SAFE_MESSAGE` as harness speech.) So a seat is the second
  surface, and step 2 begins by lifting the sensor out into something both
  can call; it fails closed — a seat whose crisis path is not wired, or
  whose sensor cannot be built, does not open. The ledger follows the
  owner, not the seal: `record_crisis` appends to `<home>/personas/
  safety.jsonl`, which under the guest home would hide every study trip
  from the owner's host report, so the wiring writes a seat's records to the
  owner's store — they carry no words by construction — with the surface
  named `hyperstudy` rather than today's literal `web`. `CrisisRecord` holds
  no persona and no seat, so the record alone would say only "a study,
  somewhere". It gains an optional `seat` field naming the seat directory
  by mecha's own name for it (§3), never HyperStudy's identifier
  (a closed, append-only record, so absent loads as `None` and today's
  lines still read), set by the wiring, and §8 step 2 tests that a seat's
  trip leads back to its transcript. Its neighbour does
  not travel: `record_dose` and `record_call` — the owner-facing usage
  measures, turns and call minutes, written beside `record_crisis` and by
  `PersonaChats::call_ended` into the owner's store under the persona's
  name — are never written for a seat. A stranger's turns or minutes
  counted as the owner's would be the instrument measuring someone else,
  and §11's call meter exists because voice raises attachment. In a
  seat it keeps the actions it takes today: on a trip the harness steps out
  of character with `SAFE_MESSAGE` and pauses the persona for
  `CRISIS_COOLDOWN`, and in a voice seat mecha raises HRP's `hs.flag` once
  HyperStudy reads it. Two seat-specific
  differences. The crisis resources come from the study when the session
  supplies them, since a participant may be anywhere and `SAFE_MESSAGE`'s
  numbers are US ones. That field is a second trusted slot, and it speaks in
  the one utterance that is deliberately the harness and not the persona,
  so it is held to more than `instructions` is: experimenter-authored only
  under the same contract (§3), plain text within a length bound, and it
  supplies only the resources — mecha's own out-of-character sentence
  frames them. Absent, or failing the bound, the shipped `SAFE_MESSAGE`
  stands. Spec 0.2.0 has no field for the resources, so a study's own
  resources wait on HyperStudy's list (§8), and until then the shipped
  message stands for every seat. And the handover is spoken into a voice
  session, or returned from a text decision as a `send_chat` (§2a), so
  until HyperStudy reads `hs.flag` the trip is still visible to the study
  in its own record of the conversation. Whether a study may turn the handover off is owed (§9).
  A seat whose judge becomes unreachable mid-session continues on
  keywords (`Degraded`), at parity with the owner's chats, rather than
  dropping a participant mid-sentence, while a seat whose sensor cannot be
  built never opens: the first is a check that went partial and is
  declared, the second a check that never existed. Whether a study may
  instead end a seat on `Degraded` is owed (§9) beside the handover.
- **The check's state is said, whole.** The seat declares its
  `CrisisState` to HyperStudy at session start and on every change, beside
  `hs.flag` rather than through it — all of it, not one variant, so "on",
  "switched off" and "couldn't check" never pass for each other on the far
  end either (`safety.rs`'s own rule). In a seat `Off` cannot occur (the
  check is forced on); `Degraded` — the judge could not answer, keywords
  only — can, and is said. In a voice seat the declaration needs only the
  session, so it lands with the HRP session (§8 step 5), not with
  `hs.flag`. A text decision has no channel for it in spec 0.2.0, so for
  text it is on HyperStudy's list beside the run identifier. Until
  HyperStudy reads it, a dropped declaration still leaves the state in the
  seat's transcript and manifest, and the study, reading only its own
  record, cannot exclude the session on it — which is why HyperStudy reading
  it is on its build list.
- **Retention.** Seats are kept in full (S8, §3), and mecha declares that
  through `hs.retention` once HyperStudy reads it (§3), so a study can tell
  its participants.
- **Disclosure is in the study's prompt.** Whether a participant is told
  the other party is an AI is study design (S2), and the study writes the
  whole system prompt (S10), so it says what the study wants said. mecha
  adds no statement either way; the owner's per-persona `disclosure` switch
  governs the owner's page, where its truth-maker is, and does not reach a
  seat. Said beside §3's: this is the second guarantee a remote party
  keeps rather than mecha's code — and the half of PERSONA-DESIGN §15's
  host baseline that §15 attributes to statute, not to good practice.

## 8. Build order

Each step is testable before the next. Only step 7 waits on HyperStudy to
build something. Steps 2 and 3 run against what HyperStudy already serves:
its custom text provider and its v3 persona API.

1. **The feature and the keys** — every item of `ARCHITECTURE.md`
   §Adding a feature, the `[hyperstudy]` table (`max_seats`, the
   HyperStudy `origin` the export writes to, and `dev_unidentified_runs`,
   off, §2a) in both `Config` and
   `ConfigLayer`, **stripped from project layers in `merge_text`,
   loudly**, as `[image]`, `[web]` and `[features]` are. A `mecha.toml`
   arriving with a cloned repository must not raise `max_seats`
   (`[system]`'s reason), nor point the export, which sends the owner's
   picture, at another origin; a test that a project layer's
   `[hyperstudy]` is dropped with a warning. The public port is 10000 in
   code, not a config value (§2). Keys are not config at all, layered or
   not: the `mhs_` hashes are minted into the owner's home (§5.1), and the
   export's HyperStudy API key in a directory of its own (§5.2). That is
   two stores, one per direction, in two directories, and only the first
   is bound into the listener; a test asserts the listener's binds name
   no path under the second. The same reason puts
   `hyperstudy` on `trial_env::OPERATOR_ONLY_TABLES`, the decision
   `ARCHITECTURE.md` §Adding a feature asks of every new table: an
   experiment environment's `config.toml` also arrives with a checkout,
   and `[documents]` once sat on the project strip list but not this one.
   It is inert today only because `switchable_from_environment` is `no`
   (§6), a second mechanism's accident rather than this table's guard. And
   `mecha hyperstudy key` with setup's step; nothing behind it yet but
   `Unready` until a key exists. A test on the chained `enable_command`,
   since `hyperstudy` is the first switch-bearing feature with a `requires`
   edge and that branch of `plan_enable` has never run. And `hyperstudy`'s
   row in `TRIFECTA.md`'s switch table, with what it costs: a public port,
   and the named `block_sends_after_private` departure (§4a).
2. **The listener and text decisions** — the `mecha-hyperstudy` crate and
   binary (§2): the three routes, the key checked before any body is read,
   the bounds, the closed types. Its confined launch: bwrap with
   `--unshare-net`, a cleared environment, the hashes' directory read-only, and
   `public.sock` and `listener.sock` as its only ways in and out, every leg
   preflighted, no listener where bwrap cannot run, and the step's bwrap
   tests run under `MECHA_TEST_REQUIRE_BACKENDS=1` in CI. Whether Funnel
   carries a WebSocket upgrade through to a `unix:` target, checked on the
   installed tailscale. The Funnel guard, with a test that Funnel on 443 or
   8443 stops the wiring, and one that an unreadable serve status stops it
   too. `listener.sock`'s closed message set (§2), anything else refused,
   logged, and the connection closed; `GET /v1/models` answered by the
   wiring through it; and a revoked key refused by the wiring while the
   listener still holds its hash. CLAUDE.md's crate paragraph and module
   map gain `mecha-hyperstudy` as a fifth crate when it exists, and
   `ARCHITECTURE.md` a section for the subsystem's invariants, where
   CLAUDE.md says a new subsystem's belong. `mecha-hyperstudy` on
   `release.yml`'s hand-kept publish list ahead of `mecha-cli` (it depends
   on nothing in the workspace), since a published crate's missing
   dependency fails the publish after earlier crates uploaded, as
   `mecha-slack` once did; and its no-`mecha-core` property tested by
   reading its `Cargo.toml`, rather than resting on review as
   `mecha-slack`'s does.

   Then the text decision (§2a), and with it everything a seat record
   needs, which step 4 reuses:
   - the crisis sensor lifted out of `persona_chat.rs` into something the
     owner's chats and both kinds of seat call (§7), a trip recorded in the
     owner's ledger as `hyperstudy` with `CrisisRecord`'s new `seat` field,
     `Degraded` declared, and no seat served without it;
   - a seat's own `SessionKind`, excluded by an arm of its own in
     `Scan::admits` (which admits any kind it does not name), not by the
     variant alone; the seat glob in `check-private.py`'s `whole` list; and
     no dose or call record for a seat;
   - the system message byte for byte, with no `BASE`, card, `SINCERE` or
     `DISCLOSED` even with the persona's `disclosure` on, under its bound;
     the calendar head note present and rendered in `UTC` under an owner
     config with another zone; and no request containing "owner" in any
     note;
   - the router entry's `context_window`, `vision`, `structured_output` and
     `max_tokens`, the model pinned with `follow_loaded = false`, and a
     request's `model` that never reaches the router.

   `TRIFECTA.md` gains a row for each channel a seat opens, here, where the
   surface first exists. One is the reply itself, a way out no interlock
   sees (§1), owned by the sealed home, by a text decision running nothing,
   and by the closed `Blind` catalogue. The other is the study's slot in the
   system prompt, third-party bytes that `arm_for_content` never reads and
   `mark_untrusted_output` never wraps (§3), owned by the bound on its size,
   the seat being armed `untrusted` from construction, and the
   experimenter-authored-only contract HyperStudy keeps. Exit: HyperStudy's
   connection test passes against the public address (`GET /v1/models`
   and a one-token completion). A text agent on the custom provider runs a
   scripted study on HyperStudy's dev server end to end: it reads a screen
   with an image, chats, and submits a rating. And a corpus of hostile
   requests is each refused with nothing past the listener reaching the
   wiring: no key, a wrong key, a valid key naming another persona's model,
   a body or an image past its bound, an unknown field, a slow upload, and
   a decision with no run identifier, refused `policy` unless
   `dev_unidentified_runs` is on (§2a). Seat names minted locally, with a
   test that a run identifier holding `../` or a path separator never
   reaches a path.
3. **The export** — `mecha hyperstudy export` (§5.2), with `--dry-run`; a
   confirmation that shows the picture and says it becomes public; a later
   export touching only the name, the picture, the voice binding and the
   adapter fields; and a test that no persona text is in the request body.
   The avatar goes up once HyperStudy's v3 API takes one (below). Until
   then the export sets every other field and names the avatar as the one
   thing left for the owner to set in HyperStudy's Agent Designer. Exit: an
   exported persona appears in HyperStudy with its picture, and a text
   agent using it runs step 2's study.
4. **The guest facade** — a voice seat's. `Facade`, `Mount` and a seat `SessionHost` usable
   outside serve; the confined launch (§3 — the socket bridge with its
   preflight of every leg, or the filesystem-only interim, per §9 — and,
   first, a one-line compile check that reqwest's
   `ClientBuilder::unix_socket` exists in the locked version, since the
   bridge's shape rests on it); guest home, untrusted run mode, the §3 table as
   tests (the sensor step 2 lifted out, now called from a facade, with a
   seat's trip recorded in the owner's ledger as `hyperstudy`; seat
   transcripts absent from the owner's corpus; the approver allows the catalogue and nothing else, every refusal
   rendering `Blocked by policy:` and never the user-denial prefix — and whether `Approver` gains
   a door enum so the compiler holds a future door is decided here; the
   seat is armed untrusted before its first turn; the guest facade process
   writes nothing under the operator's home outside its own seat directory,
   `parent.sock` accepts only its closed message set and fails the seat on
   anything else, and re-checks a ClientTool call against the catalogue on
   arrival; the seat host's fold machine (§4a's four cases) with its
   context endpoint, which parks an item without a run; a run that fails
   after context and a participant turn were folded into its tail restores
   without popping, so both are still in the next request (the test
   asserts the words survive, not only that a rewrite was recorded, and
   that a taint checkpoint follows the rewrite); and
   a run that fails after a forced answer ran keeps that round, so the next
   request carries the `tool_use` and its result and no second answer call
   goes out for the same screen;
   and the wiring's writes there are a closed list of two — the router hold
   (§5) and the crisis record (§7) — among the owner's stores, beside the
   seat directory itself and §5's admission counter, which are the wiring's
   own state and not stores the owner's sessions read; a seat's session carries its own
   `SessionKind` and the corpus scan excludes it, by an arm of its own in
   `Scan::admits` (which admits any kind it does not name), not by the
   variant alone; `check-private.py` reads the seat
   transcripts; no dose or call record is written for a seat; a seat's
   config carries `context_window`, `vision` and `structured_output` (so
   the crisis judge's `response_schema` applies, at parity with the owner's
   chats), the owner's `max_tokens`, and `max_turns` one above the round
   budget and pins its model with
   `follow_loaded = false`; a study's crisis
   resources that fail their bound fall back to `SAFE_MESSAGE`; the `[security]` floor
   holds against an owner config with `trifecta = "allow"` and with
   `trifecta = "ask"`, each refused by the interlock itself, and over every
   field the floor function of §3 returns; the system
   prompt is the study's `instructions` byte for byte, with no `BASE`, card,
   `SINCERE` or `DISCLOSED` even with the persona's `disclosure` on; a closed seat's
   transcript and manifest are readable afterwards; while a seat is open the
   wiring's hold is visible
   to `mecha model use`), the
   ClientTool catalogue (an armed seat with an image in it still answers a
   question; an uncatalogued tool is refused), context folding at a
   mid-round boundary and on a closing turn, unfolded items recorded, the
   crisis handover and the degraded state, a seat refusing to open without
   its crisis path, the facade's environment holding only the allowlist and
   its config loaded `global_config_only`, its sandbox bwrap under an owner
   config with `[sandbox]` absent or `none` (and no seat opening where bwrap
   cannot run, with the step's bwrap tests run under
   `MECHA_TEST_REQUIRE_BACKENDS=1` in CI, since a skipped confinement test
   reads like a passing one), `facade.sock` answering the wiring
   from a confined facade (the wiring's only route in) — under §9's
   filesystem-only interim there is no `facade.sock` and no namespace, so
   that test and the one proving a LAN address does not answer give way to
   the interim's own negative: nothing in the seat's registry has a
   destination, every registered tool is a catalogue entry, and a scripted
   seat that asks for a fetch, a search or a file reaches no host — `approve_all` pinned
   false with the approver test driven *through* the facade's mount so it
   fails if the mount allows everything, no seat request containing "owner"
   in any note, voice or text, the
   calendar head note present with a verbatim study prompt and rendered
   in `UTC` under an owner config with another zone, the guest
   config holding no `[[hook]]`, skill, charter or subagent, the owner's
   `[security]` read and never written, budgets.
5. **The HRP session on loopback, before audio** — HRP's text-only
   responses, which HyperStudy's silent answers use, against a local
   stand-in for HyperStudy's voice bridge: context items, inline images, declared
   tools, the steering queue widened to carry `Block`s (§4a), a
   `ToolChoice` variant forcing one named function (today it has only
   `Auto` and `None`), rendered by both providers and decided for at
   `cache_lens`, which hashes `tool_choice` into the prefix identity, for
   §4's silent forced answer, and the
   `CrisisState` declaration on the session (§7), and the listening pause's
   commit parked as a participant turn with no run (§4). Exit: a scripted study (read a screen, chat, answer a rating
   through a silent forced call) runs end to end, including a context item
   with no `response.create` followed by the participant speaking, once
   before any turn and once after one — the
   sequence that would otherwise send two user messages. And a barge-in mid-tool-round, after which the next request still
   carries every `tool_use` with its result. And a context item
   arriving mid-reply in a turn with no tool call, which must wait for the
   run rather than enter a message already sent. And an image folded
   mid-round with `block_sends_after_private` on: the next send in that same
   run is refused, and the session records the arming in that round; plus
   an `instructions` over the bound refused on its `session.update` with the
   seat still open on its previous instructions.
6. **The voice transport on loopback** — the first non-WebRTC transport,
   with its own entrypoint (§2); the worker takes its facade address (and the
   two endpoints derived from it) per call from the relay — over
   `facade.sock` for a confined seat, through an `httpx` socket transport
   for the affect and direction clients and an `OpenAILLMService` subclass
   whose `create_client` hands the OpenAI client the same transport (§3) — and its HRP transport takes
   the wiring's loopback relay, which is its only shape (§2), and
   HyperStudy's conformance suite runs against it through a local stand-in
   for the listener (its runner takes any HRP client; a URL wrapper is all
   it needs). Exit: suite 6 passes in both turn-detection modes, the
   listening pause both ways, `image_context` with the router's resident
   model, and truncate `approximate`.
7. **Voice over the public address** — once HyperStudy's `custom` realtime
   provider exists. The listener's `/v1/realtime` route replaces the local
   stand-in, and nothing behind it changes. Steps 5 and 6 were proved
   against a stand-in that replays what this design believes HyperStudy
   sends; this is where that belief meets HyperStudy. Exit: HyperStudy's
   conformance suite, run by HyperStudy against the public address, passes
   at suite 6 and its report is stored; and a corpus of malformed and
   hostile frames each gets a typed rejection, with the seat left closed
   and nothing reaching the facade: truncated, an `instructions` past its
   bound, an uncatalogued or mismatched tool, an audio delta past 2000 ms,
   an unknown event type, a connection with no safety identifier.
8. **`hs.flag`** — once HyperStudy reads it.

**What HyperStudy builds.** Less than before: S11 takes its Phase 3 (the
control socket, offers, connected-agent credentials, text seats on HRP) off
what mecha needs. In order of need:

1. The `custom` realtime provider (its Phase 2), with its SSRF guard
   extended to WebSocket, so a voice persona can name mecha's address
   (step 7).
2. An opaque per-run identifier on every model-endpoint request: the value
   its voice engine already gets as `OpenAI-Safety-Identifier`, in the
   request's `safety_identifier`. It is needed before a real participant
   meets a text seat (§2a).
3. A preview-clip field for an engine-owned voice (`preset: null`, which
   its `session.update` already sends as `voice: null`), so the export can
   fill it (§5.2).
4. `transcription: null` sent to an engine that transcribes on its own,
   with its completed transcription events read regardless (§4).
5. Avatar upload on the v3 API, so the export needs no browser session
   (§5.2).
6. A question identifier on the `question` context item and each answer
   tool, and at most one answer recorded per run and question, keyed on
   the question rather than the `call_id`, since a model's second answer is
   a new call (§4a).
7. The system message and `instructions` kept to experimenter-authored
   text, with participant-authored bytes only ever sent as tagged data or
   context items (§3).
8. Later: a crisis-resources field; reading `hs.flag`, `hs.retention` and
   the crisis-state declaration on a voice session, and a channel for the
   last two on the model endpoint (§7).
9. Its design's §4.4 and §6.2 changed to say mecha is a `custom` realtime
   engine with HyperStudy's persona and the engine's voice, not a
   `connected` engine with a persona of its own (§3).
10. Optional: a custom-endpoint credential per persona rather than per
    account, so one account can hold a key for each persona (§5.1).

Each lands in the published spec under a version bump, and mecha records
the spec version it was built against, as this document records 0.2.0.

## 9. Out of scope, and owed

**Deliberately out:** the owner's running mecha and main chat (S3); a
general assistant, until it is designed as its own sandboxed instance;
image generation (S5); any public listener but §2's, and Funnel on any
port but its own, the owner's page above all (§5.1); mecha joining LiveKit — HyperStudy's voice bridge is the only room presence
and engines never hold room credentials; `hyperstudy-agent` (S1's surviving
clause); HyperStudy's `accept` transport (S11); platform
mode for two or more humans, until HyperStudy builds it — and then the
catalogue's `Blind` labels are re-derived, not inherited, since `send_chat`
would reach participants the seat's own reply does not.

**Owed:**

- A timed-out answer whose acknowledgement was lost and whose answer never
  landed: §4a's guard then leaves that question unanswerable for the rest
  of its screen, a datum silently missing from the study. Whether the seat
  reports it to HyperStudy (so the study can re-ask, or mark it), and how
  that meets HyperStudy's per-question backstop, is owed before step 5.
- Video perception, owned by another session (2026-10-21), not this design:
  a time-indexed text track of the movie participants watch, released to a
  seat no further than the playback position. It is expected to arrive
  through the paths here, as context items (§4a) or a catalogue tool, and
  any change it needs to HRP events or the catalogue is a reviewed change
  on both sides, coordinated with that session.
- Whether a seat's study tools may run when the owner has `block_sends_after_private` on (§4a); until ruled, a voice seat whose study declares them is refused `policy`, and whether the switch binds a text seat, which runs no send here, is part of the same ruling.
- Whether a text seat holds the router's model between decisions (§5): it has no open or close that HyperStudy reports, so holding it needs the run identifier and an idle end chosen by mecha.
- Whether HyperStudy's users may edit an exported picture or voice there (S12's open half); until ruled, the next export overwrites an edit made there (§5.2).
- What happens when the owner switches the router's model mid-seat (§5):
  wait out the seat, or end it with a reason. `hold.rs` makes the run the
  side that yields, and here the wiring holds for the seat, so the answer
  is the wiring's: refuse to yield until the seat ends (the owner waits,
  with no time limit), or end the seat with a reason and yield.
- What happens to a live voice seat when the owner starts a call: end the
  seat with a reason, or refuse the owner's call while a study runs.
- A reader for `hyperstudy/seats/`: it grows without bound by design
  (S8), so `doctor.rs` should report its count and size, and how the owner
  prunes it is theirs to say.
- The budget numbers and `max_seats`' default, set from a measured seat,
  not guessed.
- Whether a study may turn the crisis handover off (§7); on until ruled.
- What a trip does to a seat shorter than `CRISIS_COOLDOWN`'s fifteen
  minutes: a pause that outlasts the seat ends it, which is not today's
  action.
- Whether a voice seat's Unix-socket bridge (§3) lands with step 4 or follows a
  filesystem-only first cut — before any real participant either way. The
  bridge is three legs, not two: `model.sock` and `parent.sock` out of the
  facade, and `facade.sock` into it, for the wiring (step 4) and for the
  worker too (Python work in step 6). The interim needs none of the three.
  The listener has no interim: it is confined, with no network, from its
  first build (§2), because it is what the internet reaches.
- Latency, measured on this box: OpenAI's Realtime models are
  speech-to-speech and mecha is a cascade, so vendor figures are not ours.
  End-of-turn to first audio is recorded per turn-detection mode, with the
  router's resident model read from `served_props`, before a study depends
  on it, against the suite's thresholds (§4); and a text decision's time,
  against HyperStudy's request limit (§2a).
