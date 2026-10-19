# Serving to others: mecha as a HyperStudy engine

*2026-10-19. Design, not built.* How a mecha persona — or a general
assistant — talks, sees, draws and acts for people who are not the owner:
participants in a HyperStudy experiment, by voice first and text later,
without a listener on this box and without anything of the owner's in reach
of the conversation. PERSONA-DESIGN §14 deferred serving and listed what it
would need; this is the design it asked for.

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
| S3 | **The owner's main chat is never served.** What is served is a *profile* — a persona, or a general assistant holding nothing of the owner's — and only from a sealed guest home (§3). | this design (§1) | 2026-10-19 |
| S4 | **Engine-owned turn-taking for one human and one agent.** The engine decides when a turn ends and when it is interrupted (HRP engine mode); platform mode, for two or more humans, is a later phase on both sides. | owner, via the HyperStudy session | 2026-10-17 |
| S5 | **A seat is more than conversation:** general language, images in and out, and actions when needed (§4a). Voice first; text chat over the same connection is a later feature. | owner | 2026-10-19 |

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

**Why main chat cannot be served even with every tool off (S3).** The
trifecta interlock guards tools whose destination the model chooses. In a
conversation with a stranger, the *reply itself* is a way out: spoken to the
person who asked, chosen by no tool, seen by no interlock. A context that
holds mail, calendar or the graph leaks by answering "what was in the last
email from…". The only main chat safe to serve is one holding nothing of the
owner's. So the line S3 draws is around the owner's *stores*, not around
capability: a general assistant profile with language, images and tools
(§4a) is servable, because what makes main chat dangerous is what it holds,
not what it can do. What mecha adds for a study is the harness around a
local model — tools, images, background jobs — and for a persona, a written
character, a chosen voice, local speech recognition and synthesis, and the
per-sentence delivery director.

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
 session socket  ◄──────── wss, per seat ─── voice worker: HRP transport
   (single-use token, 60 s)                    │ Silero · SmartTurn · Parakeet · Breeze
   PCM16 24 kHz both ways                      ▼
                                            guest facade (own process, own home, loopback)
 LiveKit room ◄─► HyperStudy's voice bridge    the profile's loop, untrusted interlocutor
```

Four pieces, each with one job:

1. **`mecha-hyperstudy`** — a transport crate, the shape `mecha-slack`
   already has: it dials out (Slack's Socket Mode there, HyperStudy's control
   socket here), so no port on this machine is ever exposed, and it has **no
   `mecha-core` dependency**, checkable in `Cargo.toml`. It holds the control
   socket, reconnects with backoff, parses offers, refuses an offer whose
   `sessionUrl` is not on the control socket's origin (HyperStudy §4.1), and
   sends typed rejections. It knows nothing about personas or runs.
2. **`mecha-cli/src/hyperstudy/`** — the wiring that does know: admission
   (§5), building the guest home (§3), starting the guest facade, and handing
   the worker a seat.
3. **The worker's HRP transport** — a new transport in
   `mecha-core/units/voice/worker.py`, beside `SmallWebRTCTransport` and
   `UplinkTransport` (the transport is already the swap point). It dials the
   session URL it is handed and plays the *engine's* side of the Realtime
   protocol over a socket it opened. Who opens a socket and who serves the
   protocol are separate choices; that separation is what lets an engine sit
   behind NAT with no tunnel. Audio stays one hop: HyperStudy's PCM goes
   straight to the VAD.
4. **The guest facade** — the profile's agent loop, served from the guest
   home on loopback to the worker (§3, §4, §4a).

The judgments — is the origin right, may this persona be served, is the box
free — are made in Rust before the worker is told where to dial. The worker
stays what its module doc says it is: "deliberately dumb".

## 3. The sealed guest home

PERSONA-DESIGN §14's third option, built with `trial_env`'s pattern: an
isolated home per seat, assembled from an explicit list, never from the
operator's home.

**It holds:** the profile — for a persona, its card (the files the owner
wrote), voice settings and reference audio, and only memories the owner
picked for serving (`seedMemories`, in HyperStudy's vocabulary); for a
general assistant, a study system prompt — plus the profile's tool list
(§4a), the image library characters it was given, and any materials the
researcher supplied for the study. All copied in; the profile's own files
read-only.

**It never holds:** the owner's persona memory or chat history with that
persona; mail, calendar, graph, outbox, learning store or triggers; the
owner's `[[mcp]]` servers — including those marked `personas = true`, which
were marked for the owner's chats, not for strangers; provider keys beyond
the loopback model endpoints. `trial_env::refuse_operator_home` already
refuses a world rooted in the operator's home; the guest home uses it.

**It is its own process.** The guest facade runs as a child with
`MECHA_HOME` set to the guest home, bound to loopback on a port chosen per
seat, never the owner's `voice-serve`. Confinement under the existing
sandbox (`sandbox.rs`, bwrap) binding only the guest home is the stronger
form and costs a preflight; a confinement that is configured and fails stops
the seat, never falls through (the silently-degrading guard).

**The interlocutor is untrusted — a run mode, not a prompt:**

| Surface | Owner's persona chat | Guest seat |
|---|---|---|
| Who the human side is | the owner | a stranger; every turn `external`, so the conversation is armed untrusted from the first word |
| Persona memory | written and recalled | neither: no writer, no recall over owner stores; seeds only |
| Confirmations, outbox | spoken yes can release | none exist: no outbox route, no `parse_answer` path |
| Approver | the owner's | refuses every request, `escalate_access` included — a test drives both doors |
| Grants | `[[grant]]` and `/grant` | off by type, as an incognito chat's are |
| Tools | the persona's eligible set | the profile's explicit list from §4a's guest set, never inherited; HyperStudy's declared tools as ClientTools (§4) |
| Transcript | the chat's session file | nothing kept past the seat: HyperStudy is the system of record |

**Researcher guidance ranks below the persona.** HyperStudy sends each
state's guidance as a `session.update` that replaces `instructions`
mid-session. Under HyperStudy §4.4's engine-owned persona, mecha reads all
of `instructions` as the researcher's guidance slot, placed after the
persona's own card: it can steer the conversation, never rewrite who is
speaking. That ranking is enforced in mecha's prompt assembly, because
HyperStudy does not and cannot enforce it.

## 4. The worker transport

What the HRP transport must do, by protocol area (the events themselves are
HyperStudy's to define):

- **Session.** Accept `session.update`; map `turn_detection` (below);
  `instructions` to the guidance slot (§3); `tools` to ClientTools;
  `audio.output.voice` ignored when the persona owns its voice (`preset:
  null`). A field mecha does not support is **refused, not ignored** —
  HyperStudy's conformance asks for exactly that.
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
  `output_audio.delta`; the director runs as in an incognito call — it
  directs, and nothing is kept.
- **Truncate.** `conversation.item.truncate` cuts the reply's text at the
  heard point proportionally to audio played: grade `approximate`.
- **ClientTool.** A registry tool that emits a function call outward, awaits
  `function_call_output`, and returns `is_error` on timeout — so the agent
  loop never learns where a tool came from. `send_chat` (typed reply to the
  participant) is the first; participant typed chat arrives as a quoted
  context item, untrusted like speech.

## 4a. What a seat can do (S5)

A seat is mecha's agent loop with a short, explicit tool list. The sealed
home removes the private leg of the trifecta by construction, so what is
left to guard is the other two — a stranger's words steering what the box
does, and what leaves it — plus the box's own resources.

| Capability | How | Where it runs | Needs from HyperStudy |
|---|---|---|---|
| Language | the loop over the router's resident model | guest facade | — |
| Act *in the study* | ClientTools: HyperStudy declares a tool in `session.update`, the call goes out, its result comes back (`send_chat` today; responses, ratings, state moves are HyperStudy's to add) | HyperStudy | the tools it chooses to declare |
| See images | an image the study shows the participant, or one they share, arrives as an image content item and reaches the model as `Block::Image` — capped at the door, as every image is | guest facade | sending images on the session, not only `[context]` text |
| Make images | `image_generate` / `image_library` over the guest's characters, as a background job (`jobs.rs`) so the seat keeps talking while it draws | the image server on demand | a way to show a picture: an upload or an `hs.*` media event — the socket mecha opened is the only way bytes leave, because this box serves nothing |
| Look things up | `web_search`, `Blind`, at quick depth | the `[[search]]` chain | — |
| Read study materials | `document_read` / `file_read` over files the researcher supplied, jailed to the guest workspace | guest facade | a way to hand materials to a seat (part of the profile, or an offer field) |

**The guest set is closed, and these are never in it:** mail, calendar,
graph, outbox, shell, `http_fetch` or any `Chosen` sender, grants, the
owner's `[[mcp]]` servers. `http_fetch` deserves the reason: with nothing
private to steal it is not a trifecta tool here, but it would make this box
a fetcher that strangers aim — at the owner's LAN and tailnet included.

**Per-seat budgets** bound what a stranger can spend: tool rounds per turn,
images per seat, GPU seconds, and the seat's wall time. A budget that runs
out is an `is_error` result the model can talk around, never a dead seat.

**Text chat (later).** The same accepted socket carries a text seat with
text-only output: the same guest home, the same tool list, no worker. No
second door — HyperStudy's existing custom agent endpoint is inbound, and
stays `hyperstudy-agent`'s.

## 5. Offers and admission

An offer is accepted only when every check passes; each refusal is typed
(`busy | unknown_persona | capacity | policy`):

- **The profile is servable.** A per-profile switch the owner sets, off by
  default; an offer naming any other profile is `unknown_persona`, so the
  list of the owner's personas is never confirmed to a caller. Per
  PERSONA-DESIGN §15, a persona on the romantic template cannot be made
  servable.
- **The owner comes first.** A live owner call, or a voice-lock hold, is
  `busy` — and an owner call that starts mid-seat does not preempt the seat
  silently; what happens then is owed (§9).
- **One seat at a time,** and only with GPU room for STT, TTS and the model
  beside whatever else holds the card (image and video servers start on
  demand); otherwise `capacity`. `mecha system` is the reading.
- **The credential** for the control socket is a connected-agent token
  HyperStudy issues and can revoke; mecha stores it as it stores the Slack
  app token, and an offer carries only a single-use, sixty-second session
  token.

## 6. Latency is measured, not quoted

OpenAI's Realtime models are speech-to-speech; mecha is a cascade — speech
recognition, the model, synthesis. Vendor figures are not ours. Before a
study depends on mecha, the conformance run records end-of-turn-to-first-
audio on this box, per turn-detection mode, with the router's resident model
named from `served_props`, not assumed. HyperStudy's response-delay and
response-cap settings bound what a study will tolerate; a seat that cannot
meet them is a finding for the experimenter, not something to tune away.

## 7. Mechanisms, not policy (S2)

mecha provides; the study decides:

- **Crisis flagging.** The harness-side crisis sensor (`persona::safety`,
  keyword tier plus model judge) runs on every participant turn and raises
  HRP's `hs.flag` when it trips. What the study does with a flag is the
  study's. Until HyperStudy reads `hs.flag`, a flag is logged on mecha's
  side without its words.
- **Retention.** The guest home keeps nothing, and mecha declares
  `hs.retention: none`.
- **Disclosure.** Whether a participant is told the other party is an AI is
  study design. mecha neither inserts nor suppresses a statement; the
  persona card and the researcher's guidance say what they say.

## 8. Build order

Each step is testable before the next, and only step 4 waits on HyperStudy.

0. **Export** — `mecha persona export --hyperstudy`: a servable persona's
   card and chosen seeds in HyperStudy's persona format, for HyperStudy's
   own engine. No infrastructure; a study can pilot on OpenAI's voice now.
1. **The HRP transport on loopback** — the worker serves HRP on a local
   socket for development only, and HyperStudy's conformance suite runs
   against it (its runner takes any HRP client; a URL wrapper is all it
   needs). Exit: both turn-detection modes pass, truncate `approximate`.
2. **The guest facade** — guest home, untrusted run mode, the §3 table as
   tests (the approver test drives `escalate_access`; a seat writes nothing
   under the operator's home).
3. **The guest tool set** — §4a's closed list, per-seat budgets, ClientTool;
   images in and out over the loopback harness first, against a stand-in
   for HyperStudy's media path.
4. **Admission** — servable switch, busy, capacity, typed rejects.
5. **`mecha-hyperstudy` + wiring** — once HyperStudy's `accept` exists; the
   worker transport switches from serving a local socket to dialing the
   session URL.
6. **`hs.flag`, images over the session** — as HyperStudy builds each.
7. **Text seats** — later (S5).

**What HyperStudy builds** (its document's Phase 3): connected-agent
credentials, the control socket, offers and rejects, session tokens, an
adapter that gives its voice bridge an HRP client over an accepted socket, a
`connected` engine in its voice config naming credential and persona,
§4.4's persona ownership, and for §4a: images on the session in both
directions and the study tools it wants agents to call. **Pin the offer and reject frames before either
side builds against them** — with a direct connection, HyperStudy's spec is
the contract mecha codes to.

## 9. Out of scope, and owed

**Deliberately out:** main chat (S3); any public listener, tunnel or Funnel
on this box; mecha joining LiveKit — HyperStudy's voice bridge is the only
room presence and engines never hold room credentials; `hyperstudy-agent`
(S1); platform mode for two or more humans, until HyperStudy builds it;
HyperStudy's inbound chat-completions endpoint for text agents — text seats
ride the accepted socket instead (§4a).

**Owed:**

- What happens to a live seat when the owner starts a call: end the seat
  with a reason, or refuse the owner's call while a study runs.
- The budget numbers — images per seat, GPU seconds — set from a measured
  seat, not guessed.
- Whether `web_search` is on by default in the guest set or per profile: it
  holds nothing private, but it is a second outside reader of what the
  stranger asked.
- Whether bwrap confinement of the guest facade is required from step 2 or
  added after.
