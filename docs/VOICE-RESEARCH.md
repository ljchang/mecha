# Voice — research and plan

**2026-08-23.** One question: *how does the owner talk to mecha out loud — from
a phone or a computer — without weakening anything the harness enforces?*

Researched in four web passes on this date (Slack's voice surface; the
open-weight speech-model landscape; a deliberate gap sweep plus a GB10
platform check; the real-time transport landscape). Vendor claims were pinned
to primary sources where possible; anything that could not be is marked
**UNVERIFIED**. The speech-model half of this document is perishable the way
`SLACK-RESEARCH.md` warned about its subject — the open-weight field moved
visibly *between the first pass and the gap sweep* — so re-check the model
tables before building against them. The transport half (Slack's API ceiling,
WebRTC, SIP) moves slower.

**The ruling that shapes everything below: voice is for owners only.** That
one sentence deletes the hardest problems this feature could have had. A
dialable phone number is reachable by anyone who dials it, caller ID is
spoofable, and a voice call is *worse* than the front door's typed form —
the caller's prose would stream straight into a privileged run with no
extraction layer between. So the *default* door is the owner's own devices
over the owner's own network — and the phone-number door, reopened the same
day by a coverage constraint (the owner's cellular data is weak), is
admitted only behind a structural gate that no agent sits behind unattended
(D8, Phase 5).

---

## 1. Slack: async voice notes work; real-time never will

Two findings, one per direction, and a hard ceiling.

**Inbound voice clips are fully reachable today, with zero new Slack
surface.** A voice clip recorded in the owner DM arrives as an ordinary
`message` event with `subtype: "file_share"` — the same event
`fetch_attachments` (`connector.rs`) already handles for images. The file
object carries:

- `subtype: "slack_audio"` — the discriminator for a native voice clip;
- `duration_ms`, `aac` (an AAC rendition URL), `vtt` (WebVTT captions);
- a `transcription` object — Slack's own ASR: `{status, locale, preview}`,
  with the full text fetchable as the clip's `.vtt` sibling. Free, mediocre
  on names and technical terms; a zero-infrastructure fallback while a local
  STT sidecar is down. **UNVERIFIED: whether `transcription` populates on
  every plan tier; `status: "processing"` may require re-polling
  `files.info`.**

**The trap, found in production codebases doing exactly this: voice clips
are served with a `video/mp4` / `video/webm` Content-Type** (the container
is MP4/WebM even though the sound is AAC). A pipeline that routes on
`mimetype.starts_with("audio/")` silently skips every voice message —
microclaw (Rust) ships this bug; openclaw #4008 documents the failure.
Detection must key on `subtype == "slack_audio"`, never mimetype. Download
rides the already-hardened `url_private` path (explicit auth header, no
redirects, reject `text/html`) — the login-page-at-200 guards in
`mecha-slack/src/files.rs` were built for exactly this fetch.

**Outbound: a bot can upload mp3/m4a via `files.uploadV2` and it renders
with an inline player.** That is the ceiling: no API can mint a native
waveform-bubble voice clip — `files.getUploadURLExternal` /
`files.completeUploadExternal` expose no subtype, waveform, or
`media_display_type` argument; only Slack's own clients produce the bubble
at record time. Commercial voice-notes products are all built on the plain
upload. An inline player is adequate.

**Huddles: no.** No public API joins, streams into, or reads a huddle —
confirmed unchanged through 2026. The only "solutions" are headless-Chrome
plus virtual-audio-device hacks, ToS-gray and architecturally alien to the
Socket Mode transport. This is what forces real-time voice off Slack
entirely, and onto §3.

Prior art for the async loop (transcribe clip → agent → TTS reply → upload):
microclaw (`src/channels/slack.rs`, Rust, closest match), openclaw's Slack
extension (`media.ts`, has the MIME rewrite), KiroCrew, agents-party.

## 2. The models

("Octal" from the original question is Mistral's **Voxtral**.)

### 2.1 STT

For *async* notes and for *turn-based* real-time alike, VAD-segmented
utterances go to an offline (non-streaming) recognizer — streaming ASR buys
nothing this design needs.

| Model | WER | Speed | License | Runs via |
|---|---|---|---|---|
| **Parakeet TDT 0.6B v3** (NVIDIA) | 6.32% | RTFx ~3,300 | CC-BY-4.0 | ONNX / sherpa-onnx, **CPU is enough** |
| **Voxtral Mini 3B** (2507) | beats Whisper large-v3 | ~9.5 GB bf16 | Apache 2.0 | **llama.cpp natively (libmtmd, GGUF)**; vLLM |
| Canary-Qwen 2.5B | 5.63% | RTFx ~418 | CC-BY-4.0 | NeMo only — container-or-bust here |
| IBM Granite Speech 4.1 2B | 5.33% | RTFx ~231 | Apache 2.0 | transformers |
| Whisper large-v3 | ~7.4% | RTFx ~69 | MIT | whisper.cpp — compatibility pick now, not quality |

The 2026 leaderboard top is separated by under one WER point; license and
tooling decide, not rank. Voxtral is the one that is *understanding* rather
than transcription — audio into an LLM, "what is her tone?" answerable —
which is the upgrade path, not the default: mecha's local model already owns
the reasoning, so the STT leg should stay a dumb, fast transcriber.

Gap-sweep verdicts: Moonshine (edge-class, loses to Parakeet on a real box),
Distil-Whisper 3.5 / Phi-4-multimodal / Gemma audio / Ultravox / MiniCPM-o /
GLM-4-Voice (each loses to a standing pick at its own game), Reverb ASR
(non-commercial license trap), Canary-1B-v2 (the multilingual upgrade path
if ever needed). Newer Voxtral: *Transcribe 2* batch is **API-only**; the
open 4B Realtime model's streaming buys nothing for VAD-segmented turns.

**Pick (amended same day, owner ruling): Voxtral Mini 3B GGUF behind a
third llama-server process.** The owner chose to consolidate serving on
llama.cpp rather than add an ONNX stack, and the trade favours it anyway on
this machine: llama.cpp is the GB10's best-supported runtime, the sequential
pipeline means no GPU contention with the chat model, and understanding
comes free later. Parakeet-on-CPU (sherpa-onnx) stays recorded as the
alternative if utterance latency ever needs the ~instant transcriber.

### 2.2 TTS

| Model | Size | License | Notes |
|---|---|---|---|
| **Kokoro-82M** | 82M | Apache 2.0 | CPU-capable ONNX, 54 preset voices, no cloning; outranks most GPU models on preference Elo |
| **Chatterbox / Turbo** (Resemble) | ~0.5B | MIT | beat ElevenLabs in blind prefs; 5-sec cloning; ~2–3 GB; streaming forks hit ~80 ms TTFB |
| **Zonos2** (Zyphra, 2026-06) | 8B MoE (~900M active) | Apache 2.0 | the gap-sweep find: *working* zero-shot cloning, 44.1 kHz; footprint irrelevant at 128 GB unified |
| **Qwen3-TTS** | 0.6B/1.7B | Apache 2.0 | best cloning+emotion package at low VRAM; ~97 ms streaming |
| NeuTTS Air | 0.5B | Apache 2.0 | cloning **in GGUF/llama.cpp form** — dark horse for this box |
| Voxtral-4B-TTS | 4B | Apache 2.0 | arena-strong but public checkpoint **omits the speaker encoder** (no local cloning) and wants vLLM-Omni — both wrong for this box |

License traps recorded so they stay avoided: Spark-TTS re-licensed
Apache→CC-BY-NC-SA after release; Higgs Audio v3 went non-commercial (v2 is
Apache — do not upgrade blindly); MegaTTS3 withholds its cloning encoder;
XTTS-v2/F5-TTS non-commercial; VibeVoice was pulled entirely. Piper is
obsolete — Kokoro beats it on CPU.

**Pick: Kokoro-82M first (zero friction, fast first chunk), Chatterbox or
Zonos2 as the quality/cloning upgrade once the PyTorch container exists.**

### 2.3 The platform is part of the spec

The box is a **GB10 (DGX Spark): aarch64 + Blackwell sm_121, 128 GB unified
memory**, and support is uneven in a way that re-sorts the picks:

- **llama.cpp: first-class** — NVIDIA ships an official DGX Spark playbook.
  Everything GGUF (Voxtral Mini audio, whisper.cpp, NeuTTS Air) is the path
  of least resistance.
- **vLLM: painful** — shipped kernels stop at sm_120 and crash on GB10;
  workarounds cost 20–30%. Demotes every vLLM-first option on this box.
- **sherpa-onnx: use the CPU build.** The prebuilt aarch64-GPU tarballs pin
  a Jetson-era onnxruntime that cannot target sm_121 — and Parakeet int8 on
  the Grace cores is already many times faster than real-time, so the GPU
  build is not worth building. This also means **the STT leg cannot be
  starved by llama-server saturating the GPU** — the sidecar that feeds the
  model does not compete with it.
- **PyTorch models (Chatterbox, Zonos2, Qwen3-TTS): one-time NGC container
  or NVIDIA wheel-index setup**, then fine. **NeMo: container-or-bust** —
  another point against Canary-Qwen here.

Deployment-reality inversion worth remembering: Voxtral-4B-TTS looked like a
co-leader in the abstract and drops to last on this machine — no local
cloning *and* the worst-supported serving stack. Best model is a function of
the machine.

## 3. Real-time transports

### 3.1 The field

- **LiveKit** — Apache 2.0 across server (SFU with embedded TURN), Agents
  framework (Python primary, Node too), and even the semantic turn-detection
  model (open weights, CPU). Local STT/LLM/TTS is a documented first-class
  path: `openai.LLM(base_url=…)` against any OpenAI-compatible server,
  official local-Kokoro guide, Silero VAD local. Their DevRel ships
  `local-voice-ai` — LiveKit + llama-server + local STT + Kokoro in one
  compose. Cloud free tier: 1,000 agent-min/mo, **one free US phone
  number**, 50 inbound telephony min; workers connect *outbound*, so a home
  worker answers cloud calls with no inbound ports.
- **Pipecat** (Daily) — BSD-2, ~14.5k stars, the open alternative. The
  decisive feature for this use case: **`SmallWebRTCTransport`, peer-to-peer
  WebRTC with no SFU and no external infrastructure at all.** Also speaks
  Daily, LiveKit, WebSocket, WhatsApp, and Twilio/Telnyx transports — the
  swap path if a number is ever wanted. Local models plug in through
  OpenAI-compatible service classes; Silero VAD and a smart-turn model
  handle barge-in and end-of-turn.
- **Kyutai Unmute** — MIT, docker-compose turnkey: streaming semantic-VAD
  STT → any OpenAI-compatible LLM (`KYUTAI_LLM_URL`) → streaming TTS, web
  frontend included. Best measured self-hosted latency (~450–750 ms
  voice-to-voice) — but you adopt *Kyutai's* voices and models, and it is a
  reference system more than a framework. Useful as a benchmark bar.
- **FastRTC** (Hugging Face) — WebRTC plumbing reduced to a Python handler,
  free Cloudflare TURN. Fine for a demo; you own the turn-taking edge cases.
- **Managed platforms (Vapi, Retell, Bland)** — bring-your-own-LLM exists
  (~$0.07–0.15/min) but audio and transcripts transit their cloud and the
  BYO-LLM route means a permanent inbound tunnel to the box. Call-center
  products; nothing here for a single private owner.
- **WhatsApp Business Calling API** — real (WebRTC media, Pipecat
  integration, free inbound) but needs a verified business account and an
  API-only number. **Telegram bots still cannot take calls.**
- **Hosted S2S (OpenAI Realtime, Gemini Live)** — the latency ceiling to
  measure against, and the opposite of the premise.

### 3.2 Latency

Budget arithmetic (industry figures): STT 60–120 ms, LLM first token
100–250 ms, TTS first chunk 40–100 ms, transport 20–60 ms. Sub-1s
voice-to-voice is realistic self-hosted; cloud-stack production medians run
1.4–1.7 s. Two mecha-specific notes:

- **The gating term is llama-server TTFT on a long mecha-style prompt.**
  Prompt-cache discipline — which the harness already curates and
  `cache_lens.rs` already measures — matters more than tok/s. A voice
  front-end must reuse one agent across turns exactly as the Slack connector
  does, or every utterance pays a cold prefix.
- **Turn-detection padding is the largest hidden term.** VAD silence
  timeouts (~300–500 ms) routinely dwarf compute; a reported 1 s of compute
  became 3–4 s perceived until turn-taking config was fixed. Tune the
  detector before blaming the models.

## 4. Decisions

**D1 — Transport: Pipecat `SmallWebRTCTransport`, reached over the
tailnet.** Owners-only removes the one thing LiveKit uniquely offered (the
free dialable number), and with it the need for an SFU at all: one owner,
one peer connection, no LiveKit server to run, no TURN, no public ports.
Phone and computer are the same client — a browser page — and Tailscale
makes NAT vanish because the phone and the box share a flat WireGuard
network. `tailscale serve` provides the HTTPS the browser requires before it
will open a microphone (a secure context is mandatory for `getUserMedia`),
with certificates that are valid *only inside the tailnet* — the transport
is unreachable and un-TLS-able from the public internet, which makes
"owners only" a property of the network rather than a login screen.
LiveKit remains the named escape hatch: Pipecat can swap transports without
touching the pipeline, so this decision is cheap to revisit.

**D2 — mecha is the LLM, behind an OpenAI-compatible facade; the voice
layer never learns what a tool is.** Every framework's easy path points at
llama-server directly — which would be a voice mode that silently bypasses
tools, taint, the outbox, and the interlock: the whole harness. Instead a
new front-end (`mecha voice-serve`) exposes `POST /v1/chat/completions`
(streaming) **bound to 127.0.0.1 only** — the Pipecat worker runs on the
same box, so the facade never touches even the tailnet — and drives
`Agent::run_in` behind it. Tool calls happen invisibly inside a turn; the
worker sees token deltas. This is the invert-before-declaring-impossible
move: relocate the feature to the trusted side of the invariant. It is also
the `frontdoor::for_privileged_run` shape — the boundary is a function
signature, not a rule someone remembers.

**D3 — a voice session is a `Conversation`.** One call, one taint slate,
recorded as an ordinary session JSONL (so distillation, `recall`, and the
run-quality corpus see it for free). The facade keys the conversation on a
session id the worker passes (header or `user` field), and ignores the
chat-completions history the framework re-sends — mecha's `Conversation` is
the state, exactly as it is for a Slack thread. Same-conversation semantics
across a dropped-and-rejoined call can come later; a fresh slate per call is
the honest default (the Slack-thread precedent).

**D4 — barge-in is `RunContext::cancel`; a mid-run utterance is
`queued_input`.** The interruption model was already built: cancel stops at
the next safe point and keeps the partial turn (a cancellable run always
streams — and the facade always streams); steering folds new text into the
tool-results turn. Voice maps onto both primitives with nothing new in the
loop. When the client aborts the HTTP stream (user spoke over the reply),
the facade cancels; when the worker submits an utterance while a run is in
flight, it steers.

**D5 — owner speech arms nothing, like typed text.** The taint rule for
images is "captured, not composed" — the user chose the window, not
everything in it. Push-to-talk speech is the opposite case: the owner chose
every word, exactly like typing, and there is no rendered third-party
content inside an utterance the way there is inside a screenshot. STT
mis-recognition does not change whose words they are. The counterargument —
an open mic can catch someone else's voice in the room — is real but thin:
VAD-segmented turn capture on a surface only the owner can reach, and the
ambient-capture risk is the owner's own room, not a third party's rendered
payload. Decision: transcripts enter as ordinary user text. **This is the
one decision in this file that loosens nothing but should still be ratified
by the owner before building** — it is recorded here so it is deliberate
rather than incidental. **Ratified 2026-08-24: the owner ruled voice is
typed text — no taint.**

**D6 — one serving family, plus one small exception (amended same day,
owner ruling).** STT is Voxtral Mini 3B GGUF behind a **third llama-server
process** (:8082 — one model per process, per `docs/LLAMA-SERVER.md`; the
audio projector is a second file, and the vision-mmproj trap applies
verbatim: a Voxtral missing its mmproj loads fine and says it cannot hear).
No sherpa/ONNX-ASR stack exists in the design any more. TTS is the one leg
that cannot follow GGUF, and the owner ruled on the voice (2026-08-24):
**Chatterbox Turbo is the launch voice** — MIT, blind-preference winner
against ElevenLabs, ~80 ms streaming TTFB, 5-second cloning — served from
the NGC PyTorch container behind an OpenAI-compatible wrapper (community
Chatterbox servers exist; **UNVERIFIED which to pin — decide at build**).
**Kokoro-FastAPI stays installed beside it as the always-alive fallback**:
no container, no GPU, no PyTorch, so a broken container degrades the voice
instead of killing it — the same fail-to-a-lesser-mode shape as provider
fallbacks, and like them, never silent: the worker should say which voice
is speaking when it is not the configured one. NeuTTS Air (GGUF, cloning)
stays noted as the strict-purity alternative if the container is ever
retired. Caveat carried
to Phase 2/3: llama-server takes audio via chat-completions content parts
(mtmd), not a Whisper-style `/v1/audio/transcriptions` endpoint, so the
worker needs a thin STT adapter — **UNVERIFIED at build time: the exact
llama-server audio request shape**. The worker config remains three local
base URLs, every leg swappable, and the same STT endpoint serves the Slack
voice-notes track without a second integration.

**D7 — voice runs carry the standard tool surface, and the session has a
sound design (expanded 2026-08-24 at the owner's request).** Three earcons,
and the rule that places them: **each sound is played by the party that can
observe its trigger.**

- *Start chime* — **client-side**, on WebRTC connection established
  (fires again on a successful reconnect, which is how the owner learns a
  drop healed).
- *Thinking loop* — **worker-side**: only the worker can see "request
  sent, no first token yet." After ~800 ms it loops a soft sound and
  stops on the first token — zero facade/protocol change, the
  ChatGPT-voice pattern. V2 layers spoken tool-start notices on top
  ("searching the web…") streamed as ordinary tokens, since the facade
  knows which tool is running — the way the TUI prints them.
- *End chime* — **client-side, preloaded in the page**, fired by the
  connection-state machine on graceful end *and* on abrupt loss. The
  preload is the point: its most important trigger is the network dying,
  and a server cannot announce a drop over the connection that dropped.
  Silence during a twenty-second mail search reads as a dead line; a dead
  line that sounds like nothing at all is worse. Deliberately *not* narrowing the tool list for voice —
the approver and outbox already govern consequence, and a voice run that
cannot do what a typed run can would make the safe surface the useless one
(the read-only-trigger lesson, inverted).

**D8 — the phone number is the far door, and the gate is structural**
(amended same day — the coverage constraint arrived after D1 was written).
A carrier call rides a dedicated priority bearer (VoLTE QCI 1: guaranteed
bit rate, highest user-plane priority), which is why calls survive signal
where best-effort data — and WebRTC is only ever best-effort data —
stutters and dies; on weak cellular, WebRTC also tends to relay through
DERP because carrier NAT defeats the punch-through. So the tailnet page is
the *near* door (home, Wi-Fi: wideband audio, no intermediaries) and a
dialable number is the *far* door (out of coverage for data, still in
coverage for voice). The gate, verified against the docs and enforced by
the SIP layer before any agent exists: the inbound trunk's
`allowed_numbers` rejects unknown caller IDs *at the trunk* — a filter,
never authentication, since LiveKit's own security blog calls caller ID
"trivially spoofable" — and the dispatch rule's `pin` makes the SIP
bridge itself collect a DTMF PIN, with a wrong PIN disconnected before a
room is ever created. Agent dispatch is a property of the room, so **no
agent hears a byte of unauthenticated audio** — the
`for_privileged_run` shape again, as configuration in somebody else's SIP
service rather than a rule an agent follows. And the door defaults to the
trigger posture — read-only, sends staged — because that costs a phone
call nothing (drafts stage regardless) and prices a defeated PIN at
"drafts in a queue," not actions.

**D9 — built for thinking out loud (added 2026-08-24, when the owner named
the use pattern: plan, brainstorm, research, learn).** Four consequences,
each a requirement rather than a polish item:

- **Resume is day-one, not deferred.** A brainstorm returned to tomorrow
  must continue, so the facade offers resume-last (mecha sessions already
  resume; this is wiring). §6.3's deferral is overridden.
- **Voice sessions set `compact_at_tokens` and register `recall`.** Long
  sessions will cross the threshold, and a session-recording front-end is
  exactly what `recall` was built for. A compaction's summarizer call is
  silence — it gets the thinking sound like any other wait.
- **The page carries a live transcript pane.** Long research answers are
  spoken as conversational summaries while the full text is visible and
  scrollable on the device already in hand — the voice is the interface,
  the pane is the record. This is also the long-reply pressure valve: the
  TTS never has to read out a table.
- **The workspace is the stable `voice` producer**
  (`~/.mecha/work/voice/`), so Tuesday's sketch is an ordinary file on
  Thursday, retention already governs it, and — because voice sessions
  are ordinary session JSONLs — `mecha distill` turns brainstorms into
  knowledge-graph episodes with no new machinery. Research turns arm
  `untrusted` exactly as anywhere else; sends stage; the one voice-side
  rule is that the agent *says* "drafted, waiting in your outbox" so a
  staged send never reads as a silent failure.

**D10 — one load-bearing prompt: the voice system block (added
2026-08-24).** The facade adds one block to voice sessions, and three
rules govern it. It is where speakable output lives — short sentences, no
markdown or lists, numbers as words, gist-not-recitation for long
results, announce long work, and say out loud when a send was staged —
the free half of voice quality, as prompt rather than code. It is
**static, byte for byte, across sessions**, because it rides in the
cached prefix and TTFT is the whole latency game; nothing per-call may
vary it. And the learned-rules system needs no change: `behavior` +
`writing` already ride every run; a learned *spoken-style* domain, if
corrections ever accumulate, is a new opt-in `voice` domain routed only
into voice runs — the existing machinery, noted here so it is not
rediscovered. Pane interaction, decided simply: v1 is one text,
ear-shaped, on both channels — the transcript pane shows exactly what was
spoken; a split speak-the-summary/display-the-detail output is v2, not
worth prompt structure until the pane exists and the need is felt.

**D11 — Slack as the durable side-channel (added 2026-08-24).** Two
stages. **V1: a `push_to_slack` tool on the voice front-end, destination
never an argument** — it reaches the owner's own DM and nothing the model
says can move it, which is the `show_file` rule verbatim and earns the
same third quadrant (not `external_send`, not outbox-routed; the test
asserts the absence of any destination field). The workflow it buys:
brainstorm on a walk, "send me that summary," full text waiting in Slack.
**V2: generalize the remote-control attach from TUI session to voice
session** — the voice session attaches to a named thread, the transcript
mirrors there, and typed replies in that thread steer the *same*
conversation (speech and keyboard as two inputs to one owner). The
connector's must-not-answer-for-a-mirrored-thread rule already exists for
exactly this; the generalization is that the owning process is the facade
rather than the TUI. Concurrency is already sound: connector and voice
stack are separate processes, agents, and conversations — the one shared
resource is llama-server, where `-np` slots carry simultaneous runs but a
Slack-triggered run mid-generation will degrade a voice turn's TTFT
(slot/prefix affinity per agent matters — the affinity-test territory).
Rare for one owner; written down so a laggy reply gets diagnosed as
contention, not regression.

## 5. The plan

**Phase 0 — benchmark (optional, half a day).** Stand up Unmute against
llama-server (`KYUTAI_LLM_URL=http://localhost:8080`). Establishes the
latency bar on this hardware and answers "is sub-second real here" before
any integration work. Throwaway.

**Phase 1 — the speech servers (a day).** A third llama-server for
Voxtral Mini 3B GGUF + its audio mmproj (own systemd unit, own port,
`context_window` written down against its `-c`/`-np` like the others — and
confirm audio actually decodes, not just that the model loads: the mmproj
failure mode is silent). The NGC PyTorch container
(aarch64 + sm_121, the one-time platform cost) with **Chatterbox Turbo
behind an OpenAI-compatible wrapper — the launch voice** — and
Kokoro-FastAPI on CPU beside it as the always-alive fallback. Measure: STT
seconds-per-utterance on real speech, **streaming** time-to-first-sound
for both voices, and chat-model TTFT while an utterance transcribes
(expected fine — the pipeline is sequential — but measured, not
assumed). This phase also unblocks the
Slack voice-notes track independently of everything below.

**Phase 2 — the facade (the real work, in mecha).** `mecha voice-serve`:
an OpenAI-compatible streaming chat endpoint on 127.0.0.1, one
`Conversation` per session id, `Agent::run_in` per turn, disconnect →
cancel, mid-run POST → steer. Session recording on. Needs an HTTP *server*
dependency in a codebase that is client-only today — the crate choice
(axum vs. a hand-rolled hyper loop for one route) is an open question
(§6.1). Testable end-to-end with `curl` and no audio at all — which is the
point of cutting here: the facade is a text feature with text tests.

**Phase 3 — the worker and the page (a day or two).** Pipecat:
SmallWebRTC transport, Silero VAD + smart-turn, STT/LLM/TTS pointed at the
three local endpoints. A page served via `tailscale serve`
(mic-permission UX, push-to-talk toggle, the D7 client-side chimes, and
the D9 live transcript pane). systemd unit. The worker is
Python and deliberately dumb — pipeline config, no logic; everything with
judgment lives behind the facade.

**Phase 4 — polish, and the committed voice upgrade.** Turn-detection
tuning against the §3.2 padding trap; the earcon and tool-notice speech
(D7); the cache lens run over a voice session to confirm the prefix is
actually reused across utterances; and **speakable output** — the facade's
voice-session system prompt must ask for ear-shaped replies (short
sentences, no lists or markdown, numbers as words), because half of
perceived voice quality is the text, and this item is free.

The voice question was then settled further (2026-08-24): the target is
ChatGPT/Claude-voice-class naturalness and **Chatterbox Turbo launches as
the voice from Phase 1** (container and all — see Phase 1), so what
remains here is the audition, not the adoption: listen to **Zonos2**
(Apache; the permissive quality ceiling — measure its streaming TTFB,
throughput is the documented number) and **Qwen3-TTS 1.7B** (Apache;
natural-language emotion control — judge whether it helps daily use or is
a demo trick) beside Chatterbox on real assistant replies, and set up
**cloning** (5 seconds of reference audio makes the winner speak in a
chosen voice). Criteria stand: naturalness, time-to-first-sound
streaming, long-reply stability (autoregressive TTS can drift on long
passages; Kokoro cannot, which is part of why it stays the fallback).
Honest ceiling, stated once: ChatGPT/Claude voice are natively-speech
models whose prosody is driven by meaning; a cascade approaches them on
naturalness and matches them on latency, and closes most of the remaining
distance with the speakable-output rule above. Voxtral Mini's
audio-understanding turns remain the STT-side half of this phase.

**Phase 5 — the far door (optional, after Phase 3 holds up).** LiveKit
Cloud free tier: claim the free US number, `allowed_numbers` = the owner's
numbers, a dispatch rule with `pin`, dispatched into the same worker (the
pipeline is unchanged; Pipecat swaps the transport). Two costs, accepted
with eyes open and one of them measured first: telephony audio is
narrowband (~8 kHz — plan for AMR-WB *not* surviving the trunk leg), and
the measured ASR penalty on phone-band audio is real — whisper-large-v3
was clocked at ~28.7% WER on 8 kHz call-center audio against single digits
on clean wideband; a single cooperative speaker does much better, but
**run the Phase 1 STT sidecar over a downsampled test set before relying
on this door**, planning for 1.5–3× the clean-audio WER. And privacy:
call audio is plaintext to the carrier, the trunk provider, and LiveKit's
SFU (hop-by-hop SRTP; true E2EE is structurally impossible when an agent
must hear the audio and the far end is a phone). Self-hosting the SIP
bridge removes exactly one of those intermediaries and adds open UDP
ports to the home box; not worth it here.

**Parallel track — Slack voice notes (small, independent).** An audio
branch beside the image branch in `fetch_attachments`: key on
`subtype == "slack_audio"`, download via the hardened path, STT sidecar,
transcript into the prompt marked as transcribed speech; reply optionally
TTS-rendered and uploaded via `files.uploadV2` with `thread_ts`. Needs
Phase 1 only.

## 6. Open questions and deferred ends

1. **HTTP server dependency** for the facade: axum pulls a tree into a
   deliberately lean codebase; one hand-rolled route on hyper is plausible
   for a single streaming endpoint. Decide at Phase 2.
2. **Facade auth**: 127.0.0.1 binding is the boundary; whether a bearer
   token is still worth adding (defense against other local processes) —
   probably yes, it is one header.
3. **Reconnect semantics** (D3, revised by D9): resume-last is a day-one
   requirement for the thinking-out-loud use pattern. What stays open is
   the *shape* — an explicit "continue" affordance vs. auto-resume within
   a time window, and whether named sessions ever need the remote-control
   attach model. Decide at Phase 2, where the session-id mapping lives.
4. **Speaker verification** is deliberately absent: the tailnet is the
   authentication. If the threat model ever includes "someone else holding
   the owner's unlocked phone," that is a device problem, not a mecha one.
5. **PSTN residual risks (D8, Phase 5)**: the PIN is a static shared
   secret spoken in DTMF — shoulder-surfable, and LiveKit documents no
   retry limit or rate-limiting on PIN entry (**UNVERIFIED — test before
   trusting it against slow brute force**; if absent, a Twilio
   `<Gather>`-before-`<Connect>` front puts the lockout in webhook code
   you control, with no media reaching anything before the gate). LiveKit
   Cloud does not surface STIR/SHAKEN attestation; a Telnyx trunk delivers
   it in SIP headers reachable via `headers_to_attributes` — worth wiring
   so an A-attestation check backs up the allowlist. Caller-ID spoofing
   remains practical in 2026 (international and non-IP origination bypass
   validation), which is why the allowlist is a filter and the PIN is the
   gate. Keep anything truly sensitive behind in-conversation
   verification, per LiveKit's own recommendation — which the read-only
   default posture (D8) already implies.
6. **UNVERIFIED items carried from research**: Slack `transcription`
   availability per plan tier (§1); LiveKit agent-minute metering for
   self-hosted workers on Cloud (now relevant — Phase 5 attaches a home
   worker to a Cloud project); Kokoro streaming quality through the
   generic OpenAI plugin vs. a native one; AMR-WB availability on the
   actual trunk leg; the telephony-band WER of *our* STT picks, measured,
   not the literature's (§Phase 5).
7. **Watch item — Voxtral-4B-TTS in the ggml family.** Today it is the
   wrong TTS for this box (vLLM-Omni-only, no llama.cpp support, public
   checkpoint ships without the speaker encoder so no cloning). But a
   pure-C port exists (`mudler/voxtral-tts.c`); if the ggml ecosystem
   absorbs it properly, an all-llama.cpp STT+TTS stack becomes possible
   and the Kokoro sidecar exception (D6) could close. Re-check when
   revisiting the TTS leg — not before.


---

## 7. Build log — Phase 1 (2026-08-24, same day)

**The STT leg is live.** llama-server (build a4ce259, 2026-07) on
127.0.0.1:8082 with `bartowski/mistralai_Voxtral-Mini-3B-2507` Q8_0 +
f16 audio mmproj, `-c 8192 -np 1`. Proof: whisper.cpp's canonical 11 s
JFK sample transcribed **word-perfect, punctuation included, in 1.10 s**.
The §D6 UNVERIFIED seam is settled: audio goes in as a chat-completions
`input_audio` content part (base64 + format), and it works. Four findings,
each of which cost something:

- **Slot cache reuse mangles multimodal context, presenting as deafness.**
  llama-server's LCP-similarity slot reuse spliced a new request onto a
  partially-kept KV cache (`f_keep = 0.756`, 393 of 1611 tokens
  evaluated), landing mid-audio — and the model answered "I don't have
  the capability to transcribe audio," which reads exactly like a missing
  mmproj. **The adapter must send `cache_prompt: false` on every
  request.** Transcription has no reusable prefix, so this costs ~300 ms
  and buys correctness.
- **Prompt wording flips this model into refusal.** "Transcribe this
  audio exactly. Output only the transcription." works; the same request
  worded "transcribe the entire audio from beginning to end" refuses,
  full audio context and all. The working string is part of the adapter
  contract — pin it, and treat any rewording as a change to test, not a
  paraphrase (the ask_user decline-wording lesson, again).
- **An utterance costs ~1,611 prompt tokens flat.** A 6 s clip and an
  11 s clip both tokenized to 1,611 — mtmd pads audio to its 30 s chunk,
  so cost is per-chunk-constant, the image-tiling finding with ears. A
  voice turn's STT cost is fixed and small; nothing to optimize.
- **Synthetic TTS clips are unreliable ASR test fixtures.** ffmpeg's
  flite voice rendered one sentence intelligibly and one not; Voxtral
  transcribed the intelligible one and refused the garble — correct
  behaviour that looks like a bug. Ground-truth clips must be real
  speech (whisper.cpp's `jfk.wav` is the canonical one).

Also landed: Kokoro-FastAPI arm64 CPU image pulled; NGC
`nvcr.io/nvidia/pytorch:25.11-py3` (the tag NVIDIA's own DGX Spark
playbook pins) is the Chatterbox runtime, pull pending. Not yet: the
systemd unit for :8082, the Chatterbox container, the facade.

**The TTS leg followed, the same night, after an ABI gauntlet worth
recording.** Chatterbox Turbo speaks: 7.9 s of audio generated on the
GB10, warm rate **~4× realtime (2.1 s of audio in 0.54 s)**, `[chuckle]`
tag rendered. The working environment is committed as the docker image
**`mecha/chatterbox:base`** — the gauntlet is paid once. What it took,
each step a finding:

- The NGC container (`pytorch:25.11-py3`, torch 2.10.0a0 nightly,
  **verified seeing the GPU at capability (12,1)** — the sm_121 risk is
  retired) ships **no torchaudio**, and `chatterbox-tts` pins
  `torch==2.6.0` exactly — a naive `pip install` replaces NVIDIA's torch
  and breaks the GPU. Install `--no-deps` with a constraints file pinning
  the container's torch.
- PyPI torchaudio wheels are ABI-incompatible with NVIDIA's torch
  (dlopen fails). Source-building v2.10.0 fails on a torch header newer
  than the nightly (`torch/csrc/stable/device.h`); source-building
  release/2.9 compiles but its extension wants `torch_library_impl`,
  which NVIDIA's libtorch does not export.
- **The resolution: the C extension is not needed.** Patch torchaudio's
  loader to treat the extension as optional (`try/except` around
  `_load_lib("_torchaudio")`); Chatterbox's whole pipeline runs on the
  pure-Python paths. Saving goes through `soundfile` — torchaudio 2.9's
  `save` wants torchcodec, another dependency not worth its own gauntlet.
- A `python -c "import torchaudio"` check run from inside the source
  tree imports the *source package* and proves nothing about the
  installed one — the vacuous-check shape; verify from a neutral cwd.

Kokoro-FastAPI is also live on 127.0.0.1:8880 (arm64 CPU image,
OpenAI-compatible `/v1/audio/speech`): first request 2.2 s cold, 1.4 s
warm for ~4.6 s of audio. The fallback pair now both speak; Phase 1's
remaining items are the serving wrapper around `mecha/chatterbox:base`
(OpenAI-compatible, streaming) and the systemd units.

**The serving wrapper closed Phase 1's server work, same night.**
`scripts/voice/chatterbox_server.py` (FastAPI, in `mecha/chatterbox:serve`)
runs as the `chatterbox` container on 127.0.0.1:8881 with
`--restart unless-stopped` — reboot-durable via the docker daemon, no
sudo. Measured through the HTTP surface: **8.6 s of speech in 2.2 s.**
The `voice` parameter maps to `~/models/voices/<name>.wav` cloning
references — dropping a 5-second file there is the whole act of adding a
voice — and an unknown name is a 400, never the wrong voice. Both TTS
servers now answer the same OpenAI `/v1/audio/speech` shape (:8880
Kokoro, :8881 Chatterbox), so the worker's voice choice is one base URL.
No token-level streaming (the official Turbo API has no streaming
method); sentence-level pipelining in the worker is the TTFB plan, with
the community streaming fork as the named upgrade if sentence granularity
ever feels laggy. `scripts/voice/llama-voxtral.service` is written and
ready; installing it is the one step needing sudo, and until then :8082
runs under nohup.

Phase 1 status: **speech servers done** (:8082 ears, :8881 launch voice,
:8880 fallback voice, all measured). Next: Phase 2, the facade — where
mecha itself enters.

**Phase 2 — the facade — built and verified live, same day.**
`mecha voice-serve` (`mecha-cli/src/voice/`): a hand-rolled loopback HTTP
server on tokio + `httparse` (§6.1 resolved: httparse was already in the
lock as a leaf; no axum, no new tree), serving `POST /v1/chat/completions`
(streaming and not) and `GET /health`. Verified against the live local
model, each of these observed rather than assumed:

- **The conversation is server-side state.** Two requests on one `user`
  key: the second answered from the first's content. The client's re-sent
  history is ignored (D3 as built).
- **Tools work through the facade, jailed and governed.** A "write a
  haiku to a file" turn wrote through the path jail into the voice
  producer workspace. Under the default posture the approver *blocked*
  the write and the model said so out loud — voice-serve inherits
  `--yes`/`--read-only`, so the launch flag chooses the posture, and the
  refusal reads "Blocked by policy", never a user correction.
- **Barge-in preserves the partial turn.** A hang-up two seconds into a
  streamed count-to-two-hundred recorded `stop_cause: "interrupted"`, and
  the next request was answered with "I'd made it to thirty-three when
  you interrupted" — the Ctrl-C guarantee through an HTTP disconnect.
- **Sessions are ordinary transcripts** (`voice: <key>` titles), so
  distill and the run-quality corpus see voice for free (D9).
- A voice turn's prompt is ~14k tokens — the full prefix (tools, skills,
  rules, D10 voice block). The prefix cache is the TTFT story, as
  predicted; `cache_lens` over a voice session is the Phase 4 check.

Deviations and finds, recorded: **`recall` is absent** (the registry
belongs to the one shared agent; per-session transcripts would cross-wire
it — the Slack rule; an agent per session is the known price of changing
this). **`GlobalOpts.system_extra`** is a new append-only seam in
`setup.rs`, because `--system` replaces the base prompt and a front-end's
standing block must never clobber the user's. **A busy session treats a
new request as barge-in** (cancel, wait, run) rather than steering —
steering needs a side-channel the OpenAI protocol lacks, and hanging up
before speaking again is what a voice client actually does. And a v2
worth wanting: a **VoiceApprover** that streams the approval question as
text — "may I write this file?" — so the Ask posture becomes usable by
voice the way SlackApprover made it usable from a phone.

Remaining: Phase 3 (Pipecat worker + tailnet page + earcons + transcript
pane), the systemd units, Phase 4 polish.

**Phase 3 — the worker — running; the tailnet publish awaits one click.**
`scripts/voice/worker.py` (Pipecat 1.7, venv at `~/models/voice-worker-venv`)
serves the prebuilt client UI on 127.0.0.1:7860 and runs the pipeline:
SmallWebRTC → Silero VAD → **VoxtralSTT** (a custom service — the stock
OpenAI STT speaks `/v1/audio/transcriptions`, Voxtral speaks
chat-completions `input_audio`; the pinned transcribe prompt and
`cache_prompt: false` from §7 are encoded here, and an "I'm sorry"-shaped
transcription is dropped rather than becoming a user turn) → the facade
as the LLM → **LocalTTS** (a custom subclass — the stock service
hard-validates `voice` against OpenAI's own list, rejecting every voice a
local server actually has; same wire, pcm streaming) → back out. The
three legs are env-configurable base URLs; Kokoro is the wired default,
Chatterbox needs pcm added to its wrapper before it takes this seat.

Found while building: the GitHub examples track unreleased main
(`ProcessorUnusablePolicy` does not exist in 1.7.0), and the prebuilt UI
moved packages — the runner imports `pipecat_ai_prebuilt`, not the older
`pipecat-ai-small-webrtc-prebuilt`, and fails silently into 404s when
only the old one is installed.

**Blocked on the owner, deliberately:** `tailscale serve` is not enabled
on the tailnet — enabling it is a one-click admin consent at the URL
`tailscale serve` prints, and it is exactly the kind of decision the
harness must not make for its owner. Until then the laptop path works
today with no tailnet feature: `ssh -L 7860:localhost:7860 <box>` and
open http://localhost:7860 — localhost is a secure context, so the mic
works. Remaining after the click: `tailscale serve --bg 7860`, an
end-to-end phone test, earcons and the transcript pane (a custom page
replacing the prebuilt one), Chatterbox pcm, systemd units.

**First conversation: 2026-08-24.** Serve enabled, the page published at
the tailnet HTTPS name, and the owner spoke to the agent from a phone and
it answered — Voxtral ears, the local model with its full tool surface
behind the facade, Chatterbox Turbo's voice, WebRTC inside the tailnet,
no byte leaving the machine's own network. Owner verdict: "it did work" —
more testing to come. Open polish, in order: the real page (D7 chimes,
thinking loop, D9 transcript pane — the stock Pipecat debug UI serves
today), turn-taking tuning against the §3.2 padding trap, systemd units
for worker and facade (both run under nohup tonight; the Voxtral unit
still awaits its sudo), and the cache-lens pass over a voice session.

**The stack is reboot-proof, 2026-08-24.** `llama-voxtral` is a system
unit (installed by the owner's sudo, verified hearing after the switch);
`mecha-voice-serve` and `mecha-voice-worker` are **user units** with
linger — no privilege needed, running as the owner whether or not a
session is open (copies in `scripts/voice/`, live ones in
`~/.config/systemd/user/`); Chatterbox and Kokoro ride docker restart
policies; the tailscale serve config persists on its own. The facade
unit points at the voice-arc worktree's release binary until the branch
merges and the installed `mecha` carries voice-serve — the one
deliberate impermanence, noted so the update skill knows to repoint it.
D5 was ratified the same day: voice is typed text, no taint.

**The page — D7 and D9 built, 2026-08-24.** `scripts/voice/page/index.html`,
one self-contained file (no external fetch ever — the page must work when
the tailnet is all there is), served by `tailscale serve` as a file mount
at `/` with `/api` proxied to the worker; the stock Pipecat UI remains at
:7860/client for debugging. The worker gained `RTVIProcessor` +
`RTVIObserver`, which is how the page knows anything: transcripts both
directions, speaking edges, and `bot-llm-started` — which is *exactly*
D7's thinking trigger (request in flight, no first token), so the
thinking sound's observer is the worker and its player is the client,
the D7 placement rule satisfied by relay. The chimes are **synthesized
in WebAudio rather than fetched** — the end chime's most important
trigger is the network dying, and a sound that must be downloaded cannot
play then; synthesis makes "preloaded" true by construction. The
signature control is the **core**: one button whose ring is the state —
breathing with the real mic level (an AnalyserNode drives a CSS custom
property) while listening, a copper spark orbiting while thinking,
radiating rings while speaking. The machine's transcript lines are
copper, the owner's are steel. Reduced-motion collapses the ring states
to static color. The owner's earlier verdict on the stock UI ("the voice
button is very subtle") is the brief this page answers.
