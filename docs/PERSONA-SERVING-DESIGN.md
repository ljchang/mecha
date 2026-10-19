# Serving to others: mecha personas as HyperStudy agents

*2026-10-19. Design, not built.* How a mecha persona takes part in a
HyperStudy experiment — reading what the study shows, talking by text and
voice, and acting in the study (a rating, the next screen) — for people who
are not the owner, without a public listener on this box and without anything of
the owner's in reach of the conversation. PERSONA-DESIGN §14 deferred
serving and listed what it would need; this is the design it asked for.

The wire contract is HyperStudy's, not this document's: its
`dev_docs/VOICE_AGENTS_DESIGN.md` defines the HyperStudy Realtime Profile
(HRP) — a pinned subset of OpenAI's Realtime API with a conformance suite —
and the `accept` transport mecha uses. This document decides only what mecha
builds against it, and points there rather than restating it, because two
copies of a protocol drift into disagreeing.

## 0. Rulings

| # | Ruling | By | Date |
|---|---|---|---|
| S1 | **mecha connects to HyperStudy directly.** No `hyperstudy-agent` in the path: it publishes a local server through a public tunnel, which is the inbound door this design exists to avoid, and an outbound mode added there would be one more process between two ends mecha already owns. | owner | 2026-10-19 |
| S2 | **Study policy is the experimenter's.** Disclosure, concealment, consent, age and institutional review belong to whoever runs the study. mecha builds infrastructure: it provides mechanisms (§7) and takes no position on how a study uses them. | owner | 2026-10-19 |
| S3 | **Personas only, for now; the owner's live mecha is never served.** A seat runs a persona from a sealed guest home (§3). A general assistant may come later, and when it does it is a sandboxed instance launched for the purpose — never the owner's running mecha, its stores or its main chat (§1). | owner | 2026-10-19 |
| S4 | **Engine-owned turn-taking for one human and one agent.** The engine decides when a turn ends and when it is interrupted (HRP engine mode); platform mode, for two or more humans, is a later phase on both sides. | owner, via the HyperStudy session | 2026-10-17 |
| S5 | **What a seat does:** reads text and images the study presents, chats by text and by voice, and takes the study's actions — answering on-screen questions and ratings. **No image generation** for now (§4a). | owner | 2026-10-19 |
| S6 | **A feature of its own,** off until the owner turns it on in setup (§6). | owner | 2026-10-19 |
| S7 | **mecha mints the token;** the owner pastes it into HyperStudy (§5.1). | owner | 2026-10-19 |
| S8 | **A seat is recorded in full.** The point of a local model is that what it did can be investigated afterwards: transcripts, directions and the voice journal are kept as an owner's call's are, not silenced (§3). | owner | 2026-10-19 |
| S9 | **`max_seats` caps concurrent seats,** so studies cannot slow every conversation on the box, the owner's included (§5). | owner | 2026-10-19 |
| S10 | **The system prompt is the study's.** A seat is prompted with what HyperStudy sends as its instructions and nothing of the owner's persona text — no `BASE`, no card, no `DISCLOSED`, no `SINCERE`; the persona a seat names contributes its voice (§3). | owner | 2026-10-19 |

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
 HyperStudy backend                         this box (dials out only)
 ──────────────────                         ───────────────────────────────────────
 control socket  ◄──────── wss, held open ── mecha-hyperstudy  (transport crate)
   hs.session.offer ─────────────────────►     │ offer: origin pin, typed reject
   hs.session.reject ◄──────────────────────   ▼
                                            mecha-cli/src/hyperstudy/  (wiring)
                                               │ admit? build guest home, start guest facade
                                               ▼
 session socket  ◄──────── wss, per seat ─── mecha-hyperstudy: frames validated here
   (single-use token, 60 s)                    │ text seat: the wiring itself
   text · images · PCM16 24 kHz                │ voice seat: loopback relay to the worker's
                                               │ HRP transport · Silero · SmartTurn · Parakeet · Breeze
                                               ▼
 LiveKit room ◄─► HyperStudy's voice bridge  guest facade (own process, own home, loopback)
                                               the persona's loop, untrusted interlocutor
```

Four pieces, each with one job:

1. **`mecha-hyperstudy`** — a transport crate, the shape `mecha-slack`
   already has: it dials out (Slack's Socket Mode there, HyperStudy's control
   socket here), so no port on this machine is ever exposed, and it has **no
   `mecha-core` dependency**, checkable in `Cargo.toml`. It holds the control
   socket, reconnects with backoff, parses offers, refuses an offer whose
   `sessionUrl` is not on the control socket's origin (HyperStudy §4.1), and
   sends typed rejections. It also dials and holds every **session** socket,
   validating each frame against HRP's schema before anything else sees it,
   so the only code on this box that parses bytes from the internet is this
   crate. It knows nothing about personas or runs.
2. **`mecha-cli/src/hyperstudy/`** — the wiring that does know: admission
   (§5), building the guest home (§3), starting the guest facade, and the
   seat itself — a text seat speaks HRP with text-only output directly, a
   voice seat is handed to the worker.
3. **The worker's HRP transport** — a new transport in
   `mecha-core/units/voice/worker.py`, beside `SmallWebRTCTransport` and
   `UplinkTransport` (the transport is already the swap point). It plays the
   *engine's* side of the Realtime protocol over a loopback relay from the
   wiring, never over the internet: the owner's worker is unconfined, holds
   the owner's environment and every persona's reference audio, and so does
   not terminate a stranger's socket or parse its frames — the crate does
   (piece 1). The cost is one loopback hop for audio. Who opens a socket and
   who serves the protocol are separate choices; that separation is what
   lets an engine sit behind NAT with no tunnel.
4. **The guest facade** — the persona's agent loop, served from the guest
   home on loopback to the seat (§3, §4, §4a).

The judgments — is the origin right, may this persona be served, is the box
free — are made in Rust before a seat opens. The worker stays what its
module doc says it is: "deliberately dumb".

## 3. The sealed guest home

The isolated-home pattern of PERSONA-DESIGN §14's third option, kept on
this box rather than a separate one, and built with `trial_env`'s: an
isolated home per seat, assembled from an explicit list, never from the
operator's home.

**It holds:** the named persona's voice — its voice settings and reference
audio, copied in, read-only — and nothing of its text: who the agent is
comes from the study (S10).

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
launch: the guest facade is spawned with its environment cleared first and
an explicit allowlist added back, in `Sandbox::child_env`'s shape — the
`mcp.rs` `env_clear()` rule, since `Command::envs()` alone layers on an
inherited environment and hands the child the owner's provider keys. That
holds whether or not the bwrap question in §9 is settled either way. The guard on the home's *location* is the trial home's,
`experiment::refuse_unsafe_home` — it refuses a home that is or contains
the real one and allows one inside it, which is where §3 keeps seats — not
`trial_env::refuse_operator_home`, which guards an environment's source
data and would refuse the seat directory itself.

**The agent loop is its own process; the audio leg is shared.** The guest
facade runs as a child with `MECHA_HOME` set to the guest home, bound to
loopback on a port chosen per seat, never the owner's `voice-serve`; its
workspace jail is rooted inside the seat's own directory, never the owner's
working tree, though a seat has no file tool to use it (§4a). A voice
seat's audio runs through the owner's running voice worker — the one worker
on the box, its journal the owner's (S8) — which is part of why a live owner
call makes a voice seat `busy` (§5): one worker's state, not only GPU room.
The worker sees a seat only through the wiring's loopback relay of frames
the crate has already validated (§2); it never holds a socket to
HyperStudy.

**Confinement is filesystem confinement.** Under the existing sandbox
(`sandbox.rs`, bwrap) the guest facade binds the guest home, with
`MECHA_HOME` named in the sandbox's environment, since bwrap clears it.
bwrap's network is all or nothing — `--unshare-net` leaves the jail unable
to reach the loopback router, and without it the jail has the host's
network, LAN and tailnet included — so it adds nothing on the egress axis.
What keeps a seat off the network is that it has no network tool (§4a): a
property of the tool catalogue, not of the jail. A confinement that is
configured and fails stops the seat, never falls through (the
silently-degrading guard).

**The interlocutor is untrusted — a run mode, not a prompt:**

| Surface | Owner's persona chat | Guest seat |
|---|---|---|
| Who the human side is | the owner | a stranger. Inbound text arms nothing today, on the premise that the user chose every word (`Taint::arm_for_content`'s comment), and `external` is a tool result's field, not a message's — so a seat sets `Conversation.taint.untrusted` at construction, and the conversation records it, so the seat is armed from the first word |
| System prompt | `BASE`, the card, plus `DISCLOSED` while `disclosure` is on | HyperStudy's `instructions`, verbatim, and nothing of mecha's persona text (S10), `SINCERE` among it — the persona prompt's one unconditional line (a sincere "am I talking to a person?" gets the truth) is left out because whether an agent answers that truthfully is a disclosure decision, the study's by S2; a study that wants the floor writes it into its own prompt |
| Persona memory | written and recalled | neither: no writer, no recall over owner stores |
| Confirmations, outbox | spoken yes can release | none exist: no outbox route, no `parse_answer` path |
| Approver | the owner's | allows **exactly the ClientTool catalogue** at `approve` — the loop sends every tool that is not `read_only` there, and the answer tools and `send_chat` genuinely act — and refuses every other tool there and every other door (`escalate`, `escalate_access`, `consult`, `permit` today). Catalogue membership, checked in mecha's code, is what makes a tool runnable, the same place §4a earns `Blind`; no catalogue tool claims `read_only`. What holds a future door is the trait's defaults, which fail closed (`escalate`, `consult` → `Blocked`) or delegate (`escalate_access` → `escalate`, `permit` → `approve`); so the guest approver overrides `approve` and `escalate` and nothing that delegates. A test listing today's doors cannot catch a new one with a default body — exhaustiveness the compiler checks would need a door enum on `Approver`, named in §8 step 2 as a choice, not assumed |
| Grants | `[[grant]]` and `/grant` | off by type, as an incognito chat's are |
| Tools | the persona's eligible set | HyperStudy's declared tools only, as ClientTools — mecha registry tools whose call goes out over the session and whose result HyperStudy sends back (§4a); none of mecha's own |
| Transcript | the chat's session file | kept in full (S8): the seat's session file, with every record a run writes — tool calls, ClientTool results, context items, spoken directions — and the seat's guest home is left in place when it closes |
| Voice worker logging | an ordinary call's journal | the same: a seat is an ordinary call to the worker, never incognito |

**Where a seat is kept.** The guest home is built under the owner's home at
`hyperstudy/seats/<seat>/` and left there when the seat closes — the
experiment store's pattern, a trial's home kept as its record. The seat's
manifest names the study's session, the persona whose voice it used, the
model `served_props` reported, and the HRP settings it ran under, so a seat
can be read back the way a trial is. Retention of these records is the
owner's (S2 hands participants' consent to the study); nothing here deletes
them on a clock. mecha declares to HyperStudy that the engine keeps
transcripts (`hs.retention`), so a study can tell its participants.

**The study's prompt is the whole prompt (S10).** HyperStudy builds the
agent's identity, objective and per-state guidance into `instructions`, and
sends a `session.update` replacing them when the study's state changes.
mecha uses them as the system prompt verbatim — this is HyperStudy §4.4's
`persona: "hyperstudy"`, the ownership it builds today. Each replacement
re-pays the cached prefix once, which is the study's cost to choose. The
only harness text a seat adds is what the loop needs mechanically (tool
results, the context items of §4a, and the clock), never a claim about who
the agent is or who it is talking with. The clock needs saying: the
calendar note renders only when the system prompt carries
`date_context::GUIDANCE_HEADING`, so a verbatim study prompt would earn no
date on any request — the "Sunday on a Monday" failure by construction.
The gate stays keyed on the heading ("so the two cannot drift"), and a seat
gets the date the way it gets everything else mechanical: as a labelled
context item (§4a), read from the clock per request, so the model can tell
what it is and the system prompt stays the study's byte for byte.

## 4. The voice transport

What the HRP transport must do, by protocol area (the events themselves are
HyperStudy's to define):

- **Session.** Accept `session.update`; map `turn_detection` (below);
  `instructions` to the system prompt (§3); `tools` to ClientTools;
  `audio.output.voice` ignored, since the named persona supplies the voice
  (`preset: null`). A field mecha does not support is **refused, not ignored** —
  HyperStudy's conformance asks for exactly that.
- **Silent forced answers.** HyperStudy asks an on-screen question with a
  `response.create` that is text-only and forces one named tool
  (`tool_choice: {type: "function", name}`); its suite's `silent_tool`
  check fails an engine that speaks or skips the call. So that response
  produces no audio, and the forcing must not change the tool list — the
  cached prefix (§4a) — whether through llama-server's `tool_choice` for a
  named function, checked on the installed build rather than assumed, or
  `required` with any other name refused.
- **Audio in.** `input_audio_buffer.append` (PCM16, 24 kHz, mono) into the
  existing Silero VAD → SmartTurn v3 → Parakeet chain.
- **Turns, engine mode (S4).** Two modes, both certified by HyperStudy's
  suite (version 2 runs every turn-dependent check once per mode):
  - `server_vad` — `speech_started` on the VAD edge, the reply in flight
    self-cancelled when `interrupt_response` is set, `speech_stopped` at the
    VAD stop, `committed` at end of turn. `threshold` maps to Silero
    confidence, `silence_duration_ms` to the VAD stop; `prefix_padding_ms`
    is approximate.
  - `semantic_vad` — SmartTurn's hold; `eagerness` maps approximately.
  This differs from the owner's calls on purpose: there, a turn starts on a
  transcript (`TranscriptionUserTurnStartStrategy(use_interim=False)`); here
  the protocol pins the VAD edge.
- **Audio out.** Breeze, streaming, resampled to 24 kHz as
  `output_audio.delta`; the director runs as in any call, and its
  directions are recorded in the seat's transcript (S8).
- **Truncate.** `conversation.item.truncate` cuts the reply's text at the
  heard point proportionally to audio played: grade `approximate`.

## 4a. What a seat can do (S5)

A seat is mecha's agent loop with no tools of its own: everything it reads
and everything it does goes through the session HyperStudy opened, so the
only way out of a seat is the session it answers. The sealed home keeps the
*owner's* data out; it does not keep the private leg from arming. An image
arms `private` wherever it sits in a conversation
(`Taint::arm_for_content`), and with the stranger arming `untrusted` from
the first word, a seat is armed from its first study screen. That is
correct — the interlock cannot know a study image from the owner's — and it
makes what a ClientTool declares the question that decides whether a seat
can act at all.

| Capability | How | Needs from HyperStudy |
|---|---|---|
| Talk by text | a text seat: HRP with text-only output, no worker; typed chat in a voice seat arrives the same way | text seats on the accepted socket |
| Talk by voice | §4 | Phase 3 `accept` |
| Read the study's text | screen text, instructions and typed chat arrive as quoted context items, untrusted like speech | — (sent today as `[context]` items) |
| Read images | a study image or one a participant shares arrives inline as an image content item and reaches the model as `Block::Image`, capped at the door as every image is; mecha never fetches a URL a session names | images on the session, inline |
| Act in the study | **ClientTools**: HyperStudy declares its study tools — `send_chat`, and the answer tools for on-screen questions (`answer_choice`, `answer_choices`, `answer_scale`, `answer_ranking`, `answer_text`, `answer_dimensions`); advancing the screen is not an agent capability in HyperStudy yet — the call goes out over the session, `function_call_output` comes back marked `.from_outside()`, a timeout returns `is_error` — so the agent loop never learns where a tool came from | the study tools, declared per session |

**Where context lands.** Screen changes, typed chat and images arrive on
HyperStudy's clock, including while a ClientTool call is in flight — and
there is no legal slot between a `tool_use` and its result, nor for two user
messages in a row (images are user turns only). So context items queue and
fold at the next legal boundary through the steering path
(`ARCHITECTURE.md` §Interruption and steering) mid-round, into the message
carrying the tool results. The steering path alone is not enough: its one
drain site skips a closing turn by design — a steer taken there could not
be acted on, and "the front end says it came too late" — and a seat has no
front end to say it. So a seat folds at a second boundary the wiring owns:
an item that arrives during a closing turn, or between turns, joins the
**tail user message**. If the conversation ends on the agent's reply, the
item opens a new user message of context alone; if it already ends on a
user message — the participant's pending turn, or context with no reply
yet — the item is appended to it. A participant turn that arrives while the
tail is a context-only user message is appended to that message, never
added after it, so two user messages in a row never occur. Whether any of
it asks for a reply is the study's, by its own `response.create`. Nothing is dropped: an item still queued when the
seat closes is recorded as unfolded in the seat's transcript. The
mid-round half is not free either: the steering queue carries text
(`queued_input` is a `VecDeque<String>`), and an image context item needs it
to carry `Block`s. That widening is a core change that touches every
steering caller, the TUI and web chat included, and §8 step 3 names it.

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
schema has no destination field. A tool HyperStudy declares that is not in
the catalogue, or whose schema differs from the pinned one, is refused at
`session.update`, never registered as `Chosen` and never relabelled. A new
study tool is a catalogue entry in a reviewed change, on both sides. What
HyperStudy does past the session with a rating is the study's; what mecha
guarantees is that nothing it sends goes anywhere else. The private leg a
seat carries is the study's own material shown back to the study, but the
send-axis leak guard (`block_sends_after_private`, which refuses `Blind`
too) is a `[security]` switch — a machine fact and the owner's standing
word, which `trial_env`'s `MACHINE_TABLES` keeps any derived home from
loosening. So a seat reads it from the owner's config, never writes it. With
it off (the default) the catalogue works as above; with it on, a seat
cannot act after its first image, so the engine's hello offers no study
tools and seats are conversation only. Exempting the catalogue from an
owner's guard would be an owner's ruling, not a config value (§9).

**`Blind` reduces risk; it is not safety** (`EGRESS-DESIGN.md` §6). Blind
calls compose and nothing bounds their volume, and here the payload is
model-authored prose (`send_chat`, `answer_text`), not a sixty-byte query.
The bound is the per-seat budget below, in bytes as well as rounds.

**Not in a seat:** image generation (S5); any mecha tool — no `web_search`,
no file reading, no `http_fetch`, no shell, no owner `[[mcp]]` server, no
grants. Adding one later is a ruling, and the bar is that it reaches no
destination a stranger could aim: `http_fetch` would make this box a fetcher
pointed at the owner's LAN and tailnet.

**The tool list is declared once.** mecha's tool list is the front of its
cached prefix, so a study that swaps tools per screen re-pays the prefix on
every screen. The cheaper contract, asked of HyperStudy: declare every study
tool at session start, and let each state say which are available; a call to
an unavailable one is refused by HyperStudy with an error the model reads.

**Per-seat budgets** bound what a stranger can spend: tool rounds per turn,
turns per seat, bytes sent through ClientTools, and the seat's wall time. A budget that runs out is an
`is_error` result the model can talk around, never a dead seat.

## 5. Offers and admission

An offer is accepted only when every check passes; each refusal is typed
(`busy | unknown_persona | capacity | policy`):

- **The persona is servable.** A per-persona switch the owner sets, off by
  default. The engine's hello lists only servable personas, as opaque refs;
  an offer naming anything else is `unknown_persona`, so the rest of the
  owner's personas are never confirmed to anyone. Since a seat takes only
  the persona's voice (S10), servable means "this voice may speak in
  studies". That also settles PERSONA-DESIGN §15's line that a romantic
  persona must never reach a minor, "binding when it is designed": no
  persona's character — its template, card or relationship — reaches a
  seat at all, so a romantic one cannot reach a participant of any age by
  construction, not by a check on the switch.
- **The owner comes first.** A live owner call is `busy` for a voice seat —
  and an owner call that starts mid-seat does not preempt the seat
  silently; what happens then is owed (§9).
- **`max_seats`, counted where admission runs (S9).** `[hyperstudy]
  max_seats` caps text and voice seats together; an offer past it is
  `capacity`. The count is kept by the one long-running process that admits
  every seat, in the owner's home. `permit.rs` cannot do this: a seat is
  interactive work, which takes no permit by design, and each seat's guest
  home would give it a pool of its own. Every seat holds a router slot while
  it answers, so the default is set against the router's `-np` with one
  slot left for the owner — measured, not guessed (§9). Within the cap,
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
  collision.
- **The credential** for the control socket is a token mecha mints and the
  owner registers in HyperStudy (§5.1); an offer carries only a single-use,
  sixty-second session token.

### 5.1 The token, and why there is no URL

**mecha mints it (S7).** `mecha hyperstudy token` — and the same step in
guided setup — generates a connected-agent token: 256 bits from the OS
random source, prefixed `mhs_` so a leaked one is recognisable in a scan.
It is printed once, for the owner to paste into HyperStudy's settings, and
kept in the owner's home with the other credentials, owner-readable only.
mecha presents it in the control socket's handshake each time it dials;
HyperStudy keeps only its hash and checks the presented token against it.
`--rotate` mints a replacement; the old one stops working when the owner
replaces it in HyperStudy, and revoking it there cuts the connection on
mecha's next dial. The token is the familiar `hyperstudy-agent` shape — mint
locally, paste into HyperStudy — and the connection still runs outward:
which side generates a shared secret is independent of which side dials.

There is no public URL for this box, by any provider:

- **A tunnel** — Cloudflare's, which `hyperstudy-agent tunnel` runs, or
  Tailscale Funnel, which gives a public `*.ts.net` HTTPS address without
  putting the caller on the tailnet — publishes a listener on this box to
  the internet. `REMOTE-SURFACE-DESIGN.md` §2 ruled that out for the owner's
  own page ("Funnel is never the answer"); a door for strangers is the
  weaker case for it, not the stronger.
- **What a leaked key costs differs.** Behind a tunnel, a leaked bearer key
  reaches the engine — the box's model and whatever the endpoint can do.
  Over the dial-out, a leaked token lets someone pose as this engine to
  HyperStudy, and HyperStudy revokes it; nothing on this box answers it.
- **HyperStudy never joins the tailnet,** and needs no custom-URL guard for
  this engine: it calls nothing of ours.

## 6. The feature (S6)

`hyperstudy`, a new variant in the closed registry (`feature.rs`,
`FEATURES-DESIGN.md`) that requires `personas`: its own switch in
`[features]`, off by default and
off on every install that predates it, so nothing dials HyperStudy until the
owner says so. Guided setup offers it once personas are on and asks for the
two things it cannot work without — the HyperStudy origin, and a minted
token (§5.1), which setup mints and shows for pasting — and `Unready` names
whichever is missing. Voice
seats additionally need the voice worker installed — `voice`, not
`calls`: `calls` is what buys `mecha serve`'s relay to the worker (and so
requires `web`), and a seat reaches the worker through the wiring's own
loopback relay (§2), so it needs neither the owner's page nor its tailnet
listener. Without `voice` the engine's hello offers text only. Building it follows `ARCHITECTURE.md` §Adding a
feature item by item rather than a subset copied here; the one choice the
checklist leaves to the feature is made here: `switchable_from_environment`
is **no**, because the credential lives in the owner's home and an
environment that could switch `hyperstudy` on would be a trial home dialling
HyperStudy.

## 7. Mechanisms, not policy (S2)

mecha provides; the study decides:

- **Crisis handling.** The harness-side crisis sensor (`persona::safety`,
  keyword tier plus model judge) runs on every participant turn — **forced
  on in a seat**, whatever the named persona's `safety.crisis` switch says:
  that switch is the owner's choice about the owner's chat, and it does not
  travel with the voice into a study's. Nothing here is inherited: the
  sensor is invoked per surface (`persona_chat.rs`, `voice/direct.rs`,
  `listen.rs`; the loop has no call site), so a seat is a fourth surface
  wiring it, and it fails closed — a seat whose crisis path is not wired,
  or whose sensor cannot be built, does not open. In a
  seat it keeps the actions it takes today: on a trip the harness steps out
  of character with `SAFE_MESSAGE` and pauses the persona for
  `CRISIS_COOLDOWN`, and mecha raises HRP's `hs.flag`. Two seat-specific
  differences. The crisis resources come from the study when the session
  supplies them, since a participant may be anywhere and `SAFE_MESSAGE`'s
  numbers are US ones; the default text stands otherwise. And the handover
  is spoken or typed into the session, so until HyperStudy reads `hs.flag`
  the trip is still visible to the study in its own record of the
  conversation. Whether a study may turn the handover off is owed (§9).
- **The check's state is said, whole.** The seat declares its
  `CrisisState` to HyperStudy at session start and on every change, beside
  `hs.flag` rather than through it — all of it, not one variant, so "on",
  "switched off" and "couldn't check" never pass for each other on the far
  end either (`safety.rs`'s own rule). In a seat `Off` cannot occur (the
  check is forced on); `Degraded` — the judge could not answer, keywords
  only — can, and is said. The declaration needs only the session socket,
  so it lands with the text seat (§8 step 3), not with `hs.flag`; until
  HyperStudy reads it, a dropped declaration still leaves the state in the
  seat's transcript and manifest, and the study, reading only its own
  record, cannot exclude the session on it — which is why HyperStudy reading
  it is on its build list.
- **Retention.** Seats are kept in full (S8, §3), and mecha declares that
  through `hs.retention`, so a study can tell its participants.
- **Disclosure is in the study's prompt.** Whether a participant is told
  the other party is an AI is study design (S2), and the study writes the
  whole system prompt (S10), so it says what the study wants said. mecha
  adds no statement either way; the owner's per-persona `disclosure` switch
  governs the owner's page, where its truth-maker is, and does not reach a
  seat.

## 8. Build order

Each step is testable before the next; only step 5 waits on HyperStudy.

1. **The feature and the token** — every item of `ARCHITECTURE.md`
   §Adding a feature, the `[hyperstudy]` table (origin, `max_seats`) in both
   `Config` and `ConfigLayer`, and `mecha hyperstudy token` with setup's
   step; nothing behind it yet but `Unready` until a token exists.
2. **The guest facade** — guest home, untrusted run mode, the §3 table as
   tests (the approver allows the catalogue and nothing else — and whether `Approver` gains
   a door enum so the compiler holds a future door is decided here; the
   seat is armed untrusted before its first turn; a seat writes nothing
   under the operator's home outside its own seat directory; the system
   prompt is the study's `instructions` byte for byte, with no `BASE`, card,
   `SINCERE` or `DISCLOSED` even with the persona's `disclosure` on; a closed seat's
   transcript and manifest are readable afterwards; while a seat is open the
   wiring's hold is visible
   to `mecha model use`), the
   ClientTool catalogue (an armed seat with an image in it still answers a
   question; an uncatalogued tool is refused), context folding at a
   mid-round boundary and on a closing turn, unfolded items recorded, the
   crisis handover and the degraded state, a seat refusing to open without
   its crisis path, the facade's environment holding only the allowlist, the
   date context item present with a verbatim study prompt, the guest
   config holding no `[[hook]]`, skill, charter or subagent, the owner's
   `[security]` read and never written, budgets.
3. **A text seat on loopback** — HRP with text-only output against a local
   stand-in for HyperStudy's session: context items, inline images, declared
   tools, the steering queue widened to carry `Block`s (§4a), and the
   `CrisisState` declaration on the session (§7). Exit: a scripted study (read a screen, chat, answer a rating
   through a silent forced call) runs end to end, including a context item
   with no `response.create` followed by the participant speaking — the
   sequence that would otherwise send two user messages.
4. **The voice transport on loopback** — the worker's HRP transport takes
   the wiring's loopback relay, which is its only shape (§2), and
   HyperStudy's conformance suite runs against it through a local stand-in
   for the crate (its runner takes any HRP client; a URL wrapper is all it
   needs). Exit: both turn-detection modes pass, truncate `approximate`.
5. **`mecha-hyperstudy` + wiring + admission** — once HyperStudy's `accept`
   exists; the crate replaces the local stand-in, dialing the control and
   session sockets and validating frames, and nothing behind it changes.
6. **`hs.flag`** — once HyperStudy reads it.

**What HyperStudy builds** (its document's Phase 3): connected-agent
credentials, the control socket, the engine hello, offers and rejects,
session tokens, an adapter that gives its voice bridge an HRP client over an
accepted socket, text seats on the same socket, a `connected` engine naming
the registered token and the persona whose voice speaks, a settings page to
register a mecha-minted token (stored hashed, revocable), inline images on the
session, and the study tools as a catalogue pinned in HRP, declared once per
session (§4a). **The frames
are pinned in its §4.1 before either side builds against them** — with a
direct connection, HyperStudy's spec is the contract mecha codes to.

## 9. Out of scope, and owed

**Deliberately out:** the owner's running mecha and main chat (S3); a
general assistant, until it is designed as its own sandboxed instance;
image generation (S5); any public listener, tunnel or Funnel on this box;
mecha joining LiveKit — HyperStudy's voice bridge is the only room presence
and engines never hold room credentials; `hyperstudy-agent` (S1); platform
mode for two or more humans, until HyperStudy builds it — and then the
catalogue's `Blind` labels are re-derived, not inherited, since `send_chat`
would reach participants the seat's own reply does not; HyperStudy's
inbound chat-completions endpoint for text agents — text seats ride the
accepted socket instead.

**Owed:**

- Whether a seat's study tools may run when the owner has `block_sends_after_private` on (§4a); conversation only until ruled.
- What happens when the owner switches the router's model mid-seat (§5):
  wait out the seat, or end it with a reason.
- What happens to a live voice seat when the owner starts a call: end the
  seat with a reason, or refuse the owner's call while a study runs.
- The budget numbers and `max_seats`' default, set from a measured seat,
  not guessed.
- Whether a study may turn the crisis handover off (§7); on until ruled.
- What a trip does to a seat shorter than `CRISIS_COOLDOWN`'s fifteen
  minutes: a pause that outlasts the seat ends it, which is not today's
  action.
- Whether bwrap's filesystem confinement of the guest facade is required
  from step 2 or added after, and whether a seat ever needs a loopback-only
  network namespace built for it (§3).
- Latency, measured on this box: OpenAI's Realtime models are
  speech-to-speech and mecha is a cascade, so vendor figures are not ours.
  End-of-turn to first audio is recorded per turn-detection mode, with the
  router's resident model read from `served_props`, before a study depends
  on it.
