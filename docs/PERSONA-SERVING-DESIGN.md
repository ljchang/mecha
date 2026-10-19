# Serving to others: mecha personas as HyperStudy agents

*2026-10-19. Design, not built.* How a mecha persona takes part in a
HyperStudy experiment — reading what the study shows, talking by text and
voice, and acting in the study (a rating, the next screen) — for people who
are not the owner, without a listener on this box and without anything of
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
| S5 | **What a seat does:** reads text and images the study presents, chats by text and by voice, and takes the study's actions — make a rating, advance the screen. **No image generation** for now (§4a). | owner | 2026-10-19 |
| S6 | **A feature of its own,** off until the owner turns it on in setup (§6). | owner | 2026-10-19 |

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
 session socket  ◄──────── wss, per seat ─── text seat: the wiring itself
   (single-use token, 60 s)                  voice seat: the worker's HRP transport
   text · images · PCM16 24 kHz                │ Silero · SmartTurn · Parakeet · Breeze
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
   sends typed rejections. It knows nothing about personas or runs.
2. **`mecha-cli/src/hyperstudy/`** — the wiring that does know: admission
   (§5), building the guest home (§3), starting the guest facade, and the
   seat itself — a text seat speaks HRP with text-only output directly, a
   voice seat is handed to the worker.
3. **The worker's HRP transport** — a new transport in
   `mecha-core/units/voice/worker.py`, beside `SmallWebRTCTransport` and
   `UplinkTransport` (the transport is already the swap point). It dials the
   session URL it is handed and plays the *engine's* side of the Realtime
   protocol over a socket it opened. Who opens a socket and who serves the
   protocol are separate choices; that separation is what lets an engine sit
   behind NAT with no tunnel. Audio stays one hop: HyperStudy's PCM goes
   straight to the VAD.
4. **The guest facade** — the persona's agent loop, served from the guest
   home on loopback to the seat (§3, §4, §4a).

The judgments — is the origin right, may this persona be served, is the box
free — are made in Rust before a seat opens. The worker stays what its
module doc says it is: "deliberately dumb".

## 3. The sealed guest home

PERSONA-DESIGN §14's third option, built with `trial_env`'s pattern: an
isolated home per seat, assembled from an explicit list, never from the
operator's home.

**It holds:** the persona's card (the files the owner wrote), its voice
settings and reference audio, and only memories the owner picked for serving
(`seedMemories`, in HyperStudy's vocabulary) — copied in, read-only.

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
| Tools | the persona's eligible set | HyperStudy's declared tools only, as ClientTools (§4a); none of mecha's |
| Transcript | the chat's session file | nothing kept past the seat: HyperStudy is the system of record |

**Researcher guidance ranks below the persona.** HyperStudy sends each
state's guidance as a `session.update` that replaces `instructions`
mid-session. Under HyperStudy §4.4's engine-owned persona, mecha reads all
of `instructions` as the researcher's guidance slot, placed after the
persona's own card: it can steer the conversation, never rewrite who is
speaking. That ranking is enforced in mecha's prompt assembly, because
HyperStudy does not and cannot enforce it.

## 4. The voice transport

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

## 4a. What a seat can do (S5)

A seat is mecha's agent loop with no tools of its own: everything it reads
and everything it does goes through the session HyperStudy opened. The
sealed home removes the private leg of the trifecta by construction, and
this removes mecha-side egress too — the only way out of a seat is the
session it answers.

| Capability | How | Needs from HyperStudy |
|---|---|---|
| Talk by text | a text seat: HRP with text-only output, no worker; typed chat in a voice seat arrives the same way | text seats on the accepted socket |
| Talk by voice | §4 | Phase 3 `accept` |
| Read the study's text | screen text, instructions and typed chat arrive as quoted context items, untrusted like speech | — (sent today as `[context]` items) |
| Read images | a study image or one a participant shares arrives inline as an image content item and reaches the model as `Block::Image`, capped at the door as every image is; mecha never fetches a URL a session names | images on the session, inline |
| Act in the study | **ClientTools**: HyperStudy declares a tool (`send_chat` today; make a rating, advance the screen), the call goes out over the session, `function_call_output` comes back, a timeout returns `is_error` — so the agent loop never learns where a tool came from | the study tools, declared per session |

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
turns per seat, and the seat's wall time. A budget that runs out is an
`is_error` result the model can talk around, never a dead seat.

## 5. Offers and admission

An offer is accepted only when every check passes; each refusal is typed
(`busy | unknown_persona | capacity | policy`):

- **The persona is servable.** A per-persona switch the owner sets, off by
  default. The engine's hello lists only servable personas, as opaque refs;
  an offer naming anything else is `unknown_persona`, so the rest of the
  owner's personas are never confirmed to anyone. Per PERSONA-DESIGN §15, a
  persona on the romantic template cannot be made servable.
- **The owner comes first.** A live owner call, or a voice-lock hold, is
  `busy` for a voice seat — and an owner call that starts mid-seat does not
  preempt the seat silently; what happens then is owed (§9).
- **One voice seat at a time,** and only with GPU room for STT, TTS and the
  model beside whatever else holds the card (image and video servers start
  on demand); otherwise `capacity`. `mecha system` is the reading. Text
  seats share the router's slots and are bounded by `permit.rs`'s seats, as
  any background run is.
- **The credential** for the control socket is a connected-agent token
  HyperStudy issues and can revoke; mecha keeps it with the other secrets in
  its home, and an offer carries only a single-use, sixty-second session
  token.

## 6. The feature (S6)

`hyperstudy`, a part of `personas` in the closed registry (`feature.rs`,
`FEATURES-DESIGN.md`): its own switch in `[features]`, off by default and
off on every install that predates it, so nothing dials HyperStudy until the
owner says so. Guided setup offers it once personas are on and asks for the
two things it cannot work without — the HyperStudy origin and the
connected-agent credential — and `Unready` names whichever is missing. Voice
seats additionally need the voice worker (`calls`); without it the engine's
hello offers text only. The `[hyperstudy]` config table is two edits,
`Config` and `ConfigLayer`, as every table is.

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

Each step is testable before the next; only step 5 waits on HyperStudy.

1. **The feature** — `hyperstudy` in the registry, the switch, setup's
   offer, the config table; nothing behind it yet but `Unready`.
2. **The guest facade** — guest home, untrusted run mode, the §3 table as
   tests (the approver test drives `escalate_access`; a seat writes nothing
   under the operator's home), ClientTools, budgets.
3. **A text seat on loopback** — HRP with text-only output against a local
   stand-in for HyperStudy's session: context items, inline images, declared
   tools. Exit: a scripted study (read a screen, chat, rate, advance) runs
   end to end.
4. **The voice transport on loopback** — the worker serves HRP on a local
   socket for development only, and HyperStudy's conformance suite runs
   against it (its runner takes any HRP client; a URL wrapper is all it
   needs). Exit: both turn-detection modes pass, truncate `approximate`.
5. **`mecha-hyperstudy` + wiring + admission** — once HyperStudy's `accept`
   exists; the seats switch from serving a local socket to dialing the
   session URL.
6. **`hs.flag`** — once HyperStudy reads it.

**What HyperStudy builds** (its document's Phase 3): connected-agent
credentials, the control socket, the engine hello, offers and rejects,
session tokens, an adapter that gives its voice bridge an HRP client over an
accepted socket, text seats on the same socket, a `connected` engine naming
credential and persona, §4.4's persona ownership, inline images on the
session, and the study tools declared once per session (§4a). **The frames
are pinned in its §4.1 before either side builds against them** — with a
direct connection, HyperStudy's spec is the contract mecha codes to.

## 9. Out of scope, and owed

**Deliberately out:** the owner's running mecha and main chat (S3); a
general assistant, until it is designed as its own sandboxed instance;
image generation (S5); any public listener, tunnel or Funnel on this box;
mecha joining LiveKit — HyperStudy's voice bridge is the only room presence
and engines never hold room credentials; `hyperstudy-agent` (S1); platform
mode for two or more humans, until HyperStudy builds it; HyperStudy's
inbound chat-completions endpoint for text agents — text seats ride the
accepted socket instead.

**Owed:**

- What happens to a live voice seat when the owner starts a call: end the
  seat with a reason, or refuse the owner's call while a study runs.
- The budget numbers, set from a measured seat, not guessed.
- Whether bwrap confinement of the guest facade is required from step 2 or
  added after.
- Latency, measured on this box: OpenAI's Realtime models are
  speech-to-speech and mecha is a cascade, so vendor figures are not ours.
  End-of-turn to first audio is recorded per turn-detection mode, with the
  router's resident model read from `served_props`, before a study depends
  on it.
