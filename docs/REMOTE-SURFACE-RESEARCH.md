# The remote surface — research

**2026-08-24.** One question: *now that voice is becoming a browser page, is
Slack still the right remote control — and what should the phone-and-laptop
surface be?*

Researched in three web passes on this date (messaging platforms as agent
control surfaces; self-hosted chat UIs and the PWA-over-Tailscale mechanics;
what other harnesses ship as their remote/mobile/voice surfaces). Vendor and
project claims were pinned to primary sources where possible; anything that
could not be is marked **UNVERIFIED**. The harness-survey half is perishable
the way `SLACK-RESEARCH.md` warned about its subject — three of the surfaces
below shipped or changed within the last six months — so re-check before
building against a specific product's behaviour. The platform-API half
(Telegram/Discord transport shapes, Web Push) moves slower.

This document deliberately does not re-open `SLACK-RESEARCH.md`'s question.
That one asked *how* Slack should drive mecha and answered it; this one asks
whether Slack should remain the only remote surface, and the answer below
assumes everything already built (the connector, the outbox-as-approval
surface, `/remote-control`) stays built. `VOICE-RESEARCH.md` D11 already
rules that Slack survives as the durable side-channel; nothing here
contradicts that.

---

## 0. The short answer

| Question | Answer | Section |
|---|---|---|
| Is there a better messaging platform than Slack? | **Not enough to migrate.** Telegram is the only one that beats Slack on merits (native voice notes, a real streaming primitive, lowest ops) and it still cannot carry live voice | §2 |
| What is the better alternative? | **The tailnet web surface the voice work already commits to** — extended from one voice page into a small page family: chat, transcript, and the review queues | §3, §5 |
| "Real web app" vs "something hosted over Tailscale"? | **A false choice.** The recommendation is a real web app whose only door is the tailnet. Serving is the deployment detail; the app is the build | §3 |
| What does the field do? | Every serious 2026 surface converged on mecha's existing shape: **local loop, remote view, approval as the remotely-exercisable verb** | §1 |
| Does Slack retire? | **No — it demotes to the far door**, the surface that works when the VPN is down, exactly as D8 made the phone number the far door for voice | §3.4 |

The one-sentence version: **the OpenAI-compatible facade built for voice (D2)
is quietly the general remote-control answer — any client that can speak
streaming chat-completions becomes a mecha terminal without the loop learning
it exists — and the cheapest good client is a page mecha serves itself, over
the network that already is the owner check.**

---

## 1. What the field converged on

The strongest finding of the survey is convergence. Four independent products
arrived at the same architecture, and it is the one mecha already has:

- **OpenAI Codex** put agent control in the ChatGPT mobile app (2026-05-14):
  monitor running tasks, **approve commands**, steer, start threads
  (https://openai.com/index/work-with-codex-from-anywhere/). Pairing a phone
  to a *local* session is a **QR code scanned against the desktop app** — a
  physical-copresence proof that the phone and the machine share an owner.
  Slack integration shipped with GA 2025-10-06; ChatGPT Voice can drive
  Codex on desktop since 2026-07 (voice on iOS only via remote access to a
  desktop session — **UNVERIFIED** beyond help-center phrasing).
- **Claude Code Remote Control** (research preview, 2026-02): the loop stays
  local; claude.ai/code, the mobile app and Desktop attach as views with
  approve/deny and a subset of slash commands
  (https://simonwillison.net/2026/Feb/25/claude-code-remote-control/).
  Willison's complaints are a checklist for anyone building this: every
  action needs manual approval with no skip path, and — the load-bearing
  one — **a restarted host process leaves remote clients showing mysterious
  API errors instead of "session ended."** A dead session must say so.
- **Happy** (https://github.com/slopus/happy, MIT) and **Omnara**
  (commercial) productized phone control of Claude Code/Codex: push when
  the agent needs you, one-tap approvals, two-way voice. Happy's
  architecture is the one worth studying — an **end-to-end-encrypted relay**
  (AES-256-GCM client-side, keys paired by QR, server stores opaque blobs),
  so reachability without a VPN costs no reading party. Self-hostable
  in-repo (**UNVERIFIED** how well documented).
- **DeepSeek's harness (`dsh`)** (open-sourced 2026-08-13) refuses to be
  remote: the web UI binds 127.0.0.1 and `--host 0.0.0.0` is a hard usage
  error — remote access is "bring your own reverse proxy + TLS + auth"
  (https://atoms.dev/blog/deepseek-harness). The refusal is a design
  position, and the Slack conduit is the thing they make you build.
- **Google Jules** dissolves remote approval by making every output a PR —
  the outbox pattern with git as the outbox. Gemini CLI has no remote story
  at all (open feature requests only).
- **OpenClaw** (ex-Clawdbot, ~180k stars) is the counterexample at scale:
  ~20 messaging channels, ambitious voice — and 2026's security crisis. A
  three-CVE chain achieved **host RCE from one inbound WhatsApp message**
  (https://thehackernews.com/2026/07/researcher-details-whatsapp-to-host.html);
  **ClawHavoc** put 800+ malicious skills (~20% of the registry) in its
  marketplace; 30,000+ instances sit exposed on the internet. It is the
  measured cost of making the channel the agent and of a skill marketplace —
  two things this harness structurally refuses (the front-door split; "a
  skill is user-authored, and there is deliberately no way for it not to
  be").

The pattern to keep: **the loop stays home; remote surfaces are views plus
an input channel; the verb a remote surface must have is approval.** Mecha
holds all three already — the gap is that its approval verb (the outbox)
reaches a phone only through Slack.

## 2. The messaging platforms, priced

None forces a public endpoint — Telegram long polling and Discord's gateway
match Socket Mode's outbound-only posture. The differences are elsewhere.

**Telegram** is the strongest text alternative and the designated successor
if Slack ever has to go. Voice notes are first-class both directions
(OGG/Opus; `sendVoice` up to 50 MB); Bot API 9.3 (2025-12-31) added
`sendMessageDraft`, a purpose-built partial-message streaming primitive,
opened to all bots in 9.5 (https://core.telegram.org/bots/api-changelog) —
Slack's streaming trio has no mobile equivalent. Inline keyboards match
Block Kit; a bot is free, workspace-less, zero ban risk; ops burden is the
lowest surveyed. The ceiling: **bots cannot join calls, at all** — the
tgcalls ecosystem requires a user account over MTProto (ToS-gray, its own
phone number). Telegram does nothing for live voice.

**Discord** is the one platform where the *same surface* could carry the
real-time loop: bots officially send audio into voice channels, and
receiving works — Nous's Hermes agent runs a full VAD→STT→TTS loop in
production Discord VCs
(https://hermes-agent.nousresearch.com/docs/guides/use-voice-mode-with-hermes).
But audio *receive* is undocumented API, tolerated for years and promised
never (https://discord.js.org/docs/packages/voice/main), which is sand under
a load-bearing surface. Worth remembering as a fallback far door for voice
if the tailnet page proves unusable somewhere; not worth a migration.

**Matrix** buys sovereignty mecha already has. The price: run a homeserver,
and for calls a LiveKit SFU + JWT service (ElementX dropped Element's hosted
backend in 2025); no native buttons; bot E2EE is a recurring tax (device
verification, key stores, undecryptable-message bugs filed against real
agents). A bot has *joined* an Element Call (the Nether bridge,
matrix.org TWIM 2026-06-26) — self-described pre-1.0 and unreviewed.

**WhatsApp / Signal / iMessage — no.** WhatsApp: official API needs Meta
business verification, a public webhook and per-message billing; unofficial
(Baileys) is a documented ban-magnet, sometimes within hours. Signal:
signal-cli is unofficial and Signal now enforces protocol currency — accounts
on stale versions were mass-unregistered 2026-03-06
(https://github.com/AsamK/signal-cli/issues/1993); your account is the
hostage. iMessage: no API; BlueBubbles means an always-on signed-in Mac to
babysit.

## 3. The tailnet web surface

### 3.1 "Real web app" versus "hosted over Tailscale" is a false choice

The two halves answer different questions. *What is built* — a real front-end
in `mecha-cli`, serving pages and driving `Agent::run_in` behind the D2
facade — is the app. *Who can reach it* — `tailscale serve`, HTTPS
terminated with a real Let's Encrypt certificate for the `ts.net` name, on
an origin that does not exist off the tailnet — is the door. The kludgy
version to avoid is real: pointing a generic UI someone else wrote at the
facade and calling it a surface (§4). But serving your own app over the
tailnet is not the kludge; it is the security model. The voice page (D1)
already made this exact choice, for the same reason: **owners-only as a
property of the network rather than a login screen.**

### 3.2 Identity is the network, and serve will even say who

`tailscale serve` injects a `Tailscale-User-Login` header identifying the
tailnet user behind each request; self-hosters use it as their entire auth
model (https://github.com/AltanS/collie). For a one-owner tailnet the header
is belt-over-suspenders — reachability already is the check — but it is free,
and it means a second device class (a family member's node, someday) is a
policy decision rather than a rebuild. No login page, no session tokens to
steal, nothing to phish.

### 3.3 Notifications: Web Push is self-hosted and content-blind

The expected blocker is not one. A tailnet-only origin can still do Web
Push, because the box needs only *outbound* HTTPS to APNs/FCM — nothing
inbound — and **RFC 8291 encrypts payloads end-to-end** against the
subscription's ECDH key (https://www.rfc-editor.org/rfc/rfc8291): Apple and
Google carry the nudge and never read it. That is a materially better story
than Slack, where Slack reads everything. The costs, all iOS: push works
only for a **home-screen-installed** PWA (16.4+), permission must follow a
user gesture, delivery is flakier than Android's, and iOS may drop a
subscription whose notifications are repeatedly not shown (**UNVERIFIED**
at what rate). The side-channel alternatives are all worse here: ntfy's
instant iOS delivery requires publishing through ntfy.sh upstream and the
app must reach your server to fetch content — a tailnet-only ntfy breaks
exactly when the VPN is down; Gotify has no iOS app; Pushover is reliable,
five dollars, and a hosted party that reads the message.

### 3.4 The honest regression, and why Slack stays

The page works only while the phone's Tailscale VPN is up. A tapped
notification with the VPN down fails; there is no graceful half-state. That
is the one thing Slack does that this cannot — Slack is reachable from any
network because Slack is the public endpoint, which is also everything else
about Slack. So the web surface is the *near* door and Slack the *far* door,
the same split D8 drew for voice (tailnet page near, phone number far), and
D11's side-channel ruling already keeps the connector alive. **Tailscale
Funnel is refused as the escape hatch**: it makes the origin public, which
un-decides the one decision doing the security work. If VPN-free
reachability is ever genuinely needed, the named pattern is Happy's blind
relay — E2E-encrypted blobs through a server that cannot read them — which
costs a build and keeps the property; Funnel costs nothing and spends it.

### 3.5 Voice on iOS: the page, not the install

`getUserMedia` in *installed* PWAs on iOS has regressed repeatedly — broken
again in an iOS 26.1 beta
(https://developer.apple.com/forums/thread/802555). The voice page should be
expected to run in a Safari tab, where it always works; installed-PWA mic is
a bonus when it happens to hold. Android has no such problem. (The install
still matters on iOS for push — §3.3 — so the likely end state is an
installed PWA for chat and queues, and the same origin opened in a tab for
voice. One origin, two entry habits.)

## 4. Why not an off-the-shelf UI

Open WebUI, LibreChat and LobeChat all take a custom OpenAI-compatible base
URL, so any of them *runs* against the facade today. Three reasons none of
them is the surface:

- **A second conversation store.** Every one keeps its own history and
  replays it per request, while mecha's `Conversation` — messages *and
  taint* — is the state. D3 already resolved this exact mismatch for voice
  by keying on a session id and ignoring the framework's re-sent history;
  a UI built around owning the history fights that resolution forever.
- **Hidden completions become agent runs.** Open WebUI fires background
  model calls after every reply — title, tags, follow-up suggestions
  (https://github.com/open-webui/open-webui/discussions/15058). Against a
  bare LLM that is wasted tokens; against the facade **each one is an agent
  run with tools.** Disabling them is config that has to stay disabled.
- **Weight and licenses.** Open WebUI is multi-gigabyte with its own user
  DB, and since v0.6.6 carries a branding clause (fine at personal scale,
  no longer plain BSD); LibreChat wants MongoDB + Meilisearch; LobeChat's
  server mode wants Next.js + Postgres. All of it to render a conversation
  the agent already owns.

**Hollama** (https://github.com/fmaclen/hollama, MIT) is the existence proof
for the other direction: a static page, no backend of its own, pointed at
any OpenAI-compatible endpoint. As a stopgap it demonstrates the facade end
to end; as a lesson it says the real surface is *less* code than de-fanging
any of the big three — because mecha's server side already exists, and the
page is a rendering of state the harness owns.

## 5. What the surface would hold

Not designed here — this is the research doc; a REMOTE-SURFACE-DESIGN.md
would decide server shape, session keying and push storage. But the scope
that motivates the build, and the rules that already govern it:

- **Chat** — the facade, streamed into a page; the D9 transcript pane is
  this pane. Voice and chat are two inputs to the same conversation, which
  V2 of D11 already sketches for Slack-thread steering.
- **The review queues** — outbox, front door, tasks, `/queues` — are where
  a web surface earns its keep over Slack: approval as the remote verb,
  with real rendering instead of Block Kit. The structural rule carries
  over from the TUI unchanged: **every modal drives the CLI** (`mecha
  outbox …`, `mecha review …`, `mecha frontdoor …`) — a web verb is a page
  over the same child process, one implementation per verb, nothing
  reachable from a browser that a script cannot do.
- **The reviewable-object rules travel.** A web outbox renders `DraftView`
  (headers, prose, everything-else — nothing dropped), shows a reply's
  source as marked third-party text, and leads a publish with the rendered
  page — because approving-without-reading is the failure the outbox
  exists to prevent, and a surface that renders less than the TUI's would
  reintroduce it on the device where people read least carefully.
- **The new surface is honestly new attack surface.** The page renders
  third-party text (mail bodies, drafts' sources, strangers' extracted
  requests) inside an origin whose buttons release drafts — so escaping
  and a strict CSP are load-bearing, not hygiene: **XSS in this page is an
  approval clicked by script.** CSRF wants same-site enforcement even
  though the origin is tailnet-only. And release policy stays set by owner
  interaction, never inferred — the `/review` rule, which a page makes
  easier to honour, not harder.

**Amended same day — the owner named the wants, and each lands on
machinery that already exists.** Recorded here so the design doc starts from
rulings rather than guesses:

- **Parallel sessions, switchable in one place** (the thing liked about
  Claude Code's remote UI). This is what `RunContext` was *built* for — one
  agent, one provider connection, one cached prefix, serving concurrent runs
  jailed to different directories — and the Slack connector already runs
  many threads on one agent this way. A session rail over live
  conversations is a rendering of that, not new loop machinery. The two
  known costs arrive with it, both already documented for Slack: MCP
  servers are spawned once and do not follow per-session jails, and
  concurrent runs contend for llama-server slots, where prefix affinity
  decides whether a switch costs a cold prompt (D11's contention note).
- **File sharing, both directions.** Inbound is the Slack door verbatim:
  original into `<workspace>/inbox/`, path named in the prompt, images
  attached to the user turn (arming `private_data` as captured-not-composed
  requires). Outbound is *cleaner* than Slack: the page pulls files over
  the authenticated tailnet origin, so bytes never leave the box's network
  at all and "the destination is never an argument" holds by construction —
  there is no push, only an owner's GET.
- **A rich UI for inbox, outbox, queues, notes, tasks** — not chat panes.
  The owner is explicitly open to the UI living outside a chat interface,
  and the TUI already proves the shape: `/mail`, `/outbox`, `/queues`,
  `/tasks` are modals over CLI verbs, so the web versions are pages over
  the same verbs — a triage deck for mail (the closed `TriageAction` enum
  is already button-shaped), a card per draft rendering `DraftView`, the
  sample-of-twelve queue review as a deck with the seed printed, a board
  for tasks. The chat pane is one room in the house, not the house.
- **Voice is a mode of the chat, not a sibling page** (styled after the
  ChatGPT/Claude apps, at the owner's request): the chat input carries a
  voice button, and the call takes over the same screen — same
  conversation, same taint, D9's transcript pane rendered live, with the
  mark's slot as the state light. This changes nothing in D1/D2 (the
  transport and facade are identical); it moves only where the page lives.
- **Notes get their own surface** — a capture bar and a recent-notes list
  over the graph's `/note` path, with the honest footnote that a note is
  evidence the graph stages for review, never direct belief.
- **`ask_user` with choices.** A web front-end owns a human, so it may
  register `ask_user` — the thing the Slack connector structurally cannot
  do — and options render as buttons. Two design questions ride along:
  routing (one agent, one registry, many sessions — the prompt must reach
  the page of the run that asked, which the front-end can know but the
  tool cannot), and absence (a page is not always open; an unanswered ask
  needs the trigger's honesty — timeout to a recorded decline, never a
  guess, per the measured decline-wording finding).

## 6. Worth stealing regardless

- **QR pairing as the trust ceremony** (Codex): copresence proof for
  binding a new device, if the surface ever outgrows "the tailnet is the
  allowlist."
- **A dead session says so** (Willison on Claude Remote Control): the end
  chime rule from D7, generalised — the client learns about death from its
  own connection-state machine, never from the server that died.
- **The blind relay** (Happy): the only acceptable shape for VPN-free
  reachability, if ever needed.
- **The translator in front** (Happy's voice agent): a quarantined
  intermediary shaping rambling speech into structured requests — the
  front-door extractor pattern worn as UX.
- **LiveKit Agents' native SIP** — phone numbers with no Twilio bridge
  (https://www.forasoft.com/blog/article/livekit-ai-agents-guide) —
  simplifies D8's far door if it is ever built.

## 7. Open decisions

- **O1 — Build the web surface as the voice page's siblings?** Everything
  above says yes: one origin, one serve config, one facade; the voice page
  becomes the first resident of a small page family rather than a
  one-feature app. **Amended same day: the owner's stated wants (§5) assume
  the build**, including surfaces beyond chat; formal ruling and sequencing
  against the voice phases belong to the design doc.
- **O2 — Live approval prompts, or outbox-async only?** The field's remote
  verb is the *live* prompt (approve this tool call now); mecha's is the
  staged draft. The outbox is the better default for an absent human — that
  was §6 of `SLACK-RESEARCH.md` and it has not changed — but a page that is
  open in a hand is a present human, and a live `ask_user`-shaped prompt on
  the web surface would be the first front-end able to offer both. Decide
  at design time; nothing above depends on it.
- **O3 — Push now or later?** Web Push is the right mechanism (§3.3), and
  also the fiddliest iOS surface in this document. A v1 without push —
  badge counts on an open page — loses the "something staged while you
  were away" nudge that D11's Slack side-channel still provides. Cheap to
  defer while Slack carries it.
- **The stack, ruled same day: Svelte 5 (runes).** The owner's working
  front-end stack, and the right shape for this surface anyway: compiled to
  small static output `tailscale serve` can front directly, no framework
  runtime to ship over a phone link, and Hollama — the thin-page existence
  proof in §4 — is itself Svelte. The backend stays mecha's: pages are
  renderings of state the harness owns, fetched from the front-end's own
  process.
- **O4 — Stopgap or bespoke first?** Hollama against the facade is an
  afternoon and proves the plumbing; it renders none of the queues. Decide
  by whether the first user of the facade is voice (then the stopgap is
  pointless) or chat (then it buys a working phone surface immediately).
- **O5 — What owns a web session's agent?** A session rail implies several
  live conversations at once. The Slack connector's answer (one agent,
  per-thread `RunContext`) fits and is proven; the TUI's (one agent, one
  conversation, `set_approver`) does not scale past one session. The open
  part is `ask_user` routing and page-presence semantics (§5), and whether
  a web session and a TUI `/remote-control` attachment can ever be the
  same conversation — the one-owner rule says one process holds it, so a
  web view of a TUI session is the connector's mirror problem again.
