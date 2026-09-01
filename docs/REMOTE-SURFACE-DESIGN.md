# The remote surface — design

**2026-08-24.** One question: *how does the tailnet web surface get built
without weakening anything the harness enforces — which process owns it, who
it answers to, how its verbs reach the stores, and in what order the work
lands?*

`REMOTE-SURFACE-RESEARCH.md` (same day) is the why: the field survey, the
platform comparison, and the owner's scope rulings. The mockup canvas ("mecha
Remote", claude.ai artifact `e3afdf82`) is the look: nine phone screens held
to `brand/brand.md`, with the owner's style rulings applied — no radio
buttons, no left-accent card borders (the amber triangle glyph carries
warnings; the amber gutter survives only as a quote marker on third-party
text), chips and controls rectangular at 6px inside the 8px card family, and
voice as an in-chat mode rather than a separate page. `VOICE-RESEARCH.md`
D1–D11 are taken as given; nothing here reopens them.

---

## 0. The short answer

| Question | Answer | Section |
|---|---|---|
| What process serves this? | **One front-end, working name `mecha serve`** — the voice facade's process grown sideways: static pages + JSON/SSE routes on the tailnet door, the OpenAI facade staying loopback for the Pipecat worker | §1 |
| Who may connect? | The tailnet, verified: bind 127.0.0.1, `tailscale serve` fronts it, and the injected `Tailscale-User-Login` must equal the configured owner — absent or wrong fails closed | §2 |
| What owns a web session? | `mecha serve` does — one agent, per-session `RunContext`, the Slack connector's proven shape. Live TUI sessions are not mirrored in v1 | §3 |
| How do buttons reach stores? | **Every mutation is a `mecha …` child process**, exactly the TUI-modal rule; the build's CLI gaps (a non-interactive outbox body edit, missing `--json`) are closed as CLI features first | §4 |
| Live prompts? | Yes — the page is the first front-end that can own a present human *and* fall back honestly to an absent one: `ask_user` routed per session, timeout is a recorded decline, never a guess | §5 |
| Build order? | Dashboard (read-only) → chat → review verbs → multi-session + ask + files → voice merge + PWA | §10 |

## 1. One front-end process — D1

The voice plan already requires a long-lived front-end process holding an
agent behind an OpenAI-compatible facade (`VOICE-RESEARCH.md` D2). The box already
runs two long-lived agent-owning processes (the Slack connector and the
trigger daemon) against llama-server's four slots, so a third is
incremental rather than a doubling — the sharper argument is **prompt-cache
affinity**: voice's whole latency budget is TTFT on a cached prefix, and a
second front-end competing for the same slots splits the affinity that
budget depends on (measure with `scripts/affinity-test.py`, not a
throughput benchmark, which is structurally blind to affinity loss).
So there is **one process** — working name **`mecha serve`**, and whether it
is `voice-serve` renamed or `web-serve` absorbing it is a naming decision to
settle with the voice build, not an architectural one. It owns:

- **the agent** — one provider connection, one cached prefix, many runs via
  `RunContext` (§3);
- **the loopback facade** — `POST /v1/chat/completions` on 127.0.0.1 for the
  Pipecat worker, byte-for-byte what voice D2 specifies;
- **the tailnet routes** — static app, JSON reads, SSE streams, uploads —
  on the port `tailscale serve` fronts.

The code lives in **`mecha-cli/src/web/`**, beside `tui/` and `slack/`, on
the standing rule: the front-end that knows both sides belongs in the CLI
crate, and `mecha-core` never learns HTTP exists. The HTTP dependency
(axum or bare hyper — decide at build; leaning axum, it is the smaller
delta over hyper that tokio already implies) is therefore quarantined to
the binary crate, checkable in `Cargo.toml` — the `mecha-slack` rule
pattern.

**The Svelte app is static output served by this process.** No Node at
runtime, no SSR, no second server: `vite build` emits files, `mecha serve`
serves them, and the page is a rendering of state the harness owns. The
front-end source lives in-repo (`web/`), built at release time; the binary
embeds or reads the build directory — decide at build, embedding preferred
(one artifact to deploy, the update skill's kind of simplification).

## 2. The door — D2

- `mecha serve` binds **127.0.0.1 only**, like the facade. There is no flag
  to bind wider — the DeepSeek `dsh` refusal, adopted. Reaching it from the
  tailnet is `tailscale serve`'s job, and reaching it from the internet is
  nobody's: **Funnel is never the answer** (research §3.4).
- Every tailnet request must carry `Tailscale-User-Login` equal to
  `[web] owner_login`. Absent header, wrong value, or unset config →
  **refused**, not warned: a header that fails open is a login screen made
  of paper. **There is no loopback exemption**, and the sentence that used
  to promise one here was wrong about the shipped code (corrected
  2026-08-25): everything reaching this process arrives over loopback,
  because `tailscale serve` proxies to it, so "came from loopback" carries
  no information and an exemption would exempt everything.
  `owner_guard` fails closed on absence, not only on mismatch. The Pipecat
  worker never needed the exemption — it talks to the **voice facade on its
  own listener** (`--voice-port`, its own optional token), and never reaches
  this router at all.
- No cookies, no sessions, no login page. Identity is the network plus the
  header. **The header name is a compile-time constant** (`TAILSCALE_LOGIN`),
  so a second identity provider is a rebuild rather than the config change
  this line used to claim. Making it a `[web] auth_header` key was priced on
  2026-08-25 and **deliberately not built** — see VOICE-RESEARCH.md for why
  Cloudflare would not be equivalent anyway, and why the owner's call was to
  stay on one supported path.

## 3. Sessions — D3

`mecha serve` owns web sessions the way the Slack connector owns threads:
one agent, and per-session everything on `RunContext` — jail, budget,
cancel token, steering queue, approver. A session's workspace is
`~/.mecha/work/web/<name>/`, a subdirectory of the `web` producer root,
so retention already governs it and the MCP fixed-root caveat carries over
verbatim (servers are rooted at the producer directory; the absolute-path
rule for staged publishes applies here too, and the session pages should
surface it the way the Slack docs do).

Sessions are ordinary session JSONLs — distill, `recall`, and the
run-quality corpus see them for free (the voice D9 precedent). The rail
lists live sessions first, then resumable recorded ones; resume is
`mecha`'s existing resume, wired.

**A call is one of these sessions, not a session beside it** (voice D3,
built 2026-08-25). The page names its session key in the WebRTC offer and
the facade resolves it through `voice::SessionHost`, so a spoken turn runs
on the same conversation, taint, transcript and jail as a typed one, and
the facade holds nothing of its own. `chat::begin_turn` is the single
implementation both doors go through — the `/tasks` rule (one
implementation per verb) applied inside the process rather than across a
CLI boundary, and for the same reason: two constructions of "a run on a web
session" is how they stop agreeing about the jail and the outbox stamp.

**Not in v1: mirroring a live TUI session.** A `Conversation` has one
owner (the `/remote-control` rule), and a web view of a TUI-owned session
is the connector's mirror problem again. The v2 shape already exists on
paper — voice D11's V2 generalises the remote-control attach so the owning
process is a facade rather than the TUI — and this surface should inherit
that design when it lands rather than inventing a competing one.

## 4. Verbs — D4

Every mutation the page offers **drives a `mecha …` CLI verb as a child
process** — the `/tasks` and `/queues` rule, unchanged: one implementation
per verb, nothing browser-reachable a script cannot do, and the TUI, the
web, and cron all stay honest against the same commands. Reads use
`--json` where it exists.

The build therefore starts with the CLI gaps, each useful on its own:

- **`mecha outbox edit --body <file>`** — the phone has no `$EDITOR`, so
  the prose edit needs a non-interactive path writing through the same
  `outbox::with_body` seam (the scratch-file round-trip rules — the
  reference marker, refuse-when-marker-gone — apply to the web editor
  identically).
- **`--json` on any verb the pages read that lacks it** (survey at build:
  outbox list/show have it; check review, frontdoor, tasks, doctor,
  sessions).
- **A notes verb** if `/note`'s path is TUI-only today — the capture bar
  needs the same one-implementation rule.

## 5. Ask and approvals — D5

The web front-end owns a human, so it registers `ask_user` — the thing the
Slack connector structurally cannot do. Three rules:

- **Routing is the front-end's knowledge, not the tool's.** One agent, one
  registry; the prompt reaches the page of the session whose run asked,
  because `mecha serve` knows which `RunContext` is which. Options render
  as cards (no radios — the owner's ruling, and the mockup's Ask screen is
  the reference).
- **Absence is honest.** Presence is the SSE/WebSocket connection state.
  No page open, or no answer inside the timeout → the ask resolves as a
  **recorded decline** with the measured wording discipline — never
  "proceed with your best interpretation", which measurably makes models
  invent.
- **The approver can go live, and the outbox stays the default.** A
  present owner may approve a tool call from the page (rendered with full
  arguments, taint state visible); an absent one falls back to exactly
  what the mode already says (read-only denies, outbox stages). Release
  policy (`/review now|later|auto`) is set only by an explicit control on
  the page — never inferred from anything sharing a context window with
  third-party text.

## 6. Files and images — D6

- **Inbound**: upload lands in `<session-jail>/inbox/` and the path is
  named in the prompt — the Slack door verbatim. An image also attaches to
  the user turn as `Block::Image`, arming `private_data` (captured, not
  composed).
- **Outbound**: the page **pulls** over authenticated GET. Any path in a
  download route resolves through the same containment proof as tool input
  — a URL is model-adjacent data and `ToolCtx::resolve`'s rule applies to
  it: never serve a raw path. No push exists, so "the destination is never
  an argument" holds by construction.

## 7. Security posture — D7

The threat model in one sentence: **XSS in this page is an approval clicked
by script.** The page renders third-party text (mail bodies, draft sources,
strangers' extracted prose) inside an origin whose buttons release drafts.

- Strict CSP: `default-src 'self'`, no external origins (fonts shipped
  local rather than Google-hosted — the page must work with no internet at
  all, and the tailnet is not the internet), no inline script, no eval.
- All store-derived text renders as text; third-party text additionally
  carries the amber gutter and its tool-provenance header (the outbox
  source rules, ported). The model-facing `<untrusted-content>` envelope is
  stripped for humans, as everywhere.
- Mutations require a custom request header (CSRF belt over the
  no-cookies suspenders) and confirm on the page for sends — tainted ones
  with the full arguments on screen, EOF-equivalent (dismiss) = no.
- The page never holds a credential: it *is* the credential's beneficiary,
  and everything it can do, it does by asking `mecha serve`, which asks
  the CLI.

## 8. The stack — D8

Svelte 5 (runes) — the owner's stack, ruled in the research doc. Static
adapter, no SSR. One page family including the in-chat voice mode; the
voice screen is the same session with the call overlay (mockup's Voice
screen: the mark's slot as the state light, live verbatim transcript,
mute / end / keyboard). PWA manifest from day one (it costs nothing);
*installed*-PWA push and its iOS ceremony wait for Phase 5. Known iOS trap
carried from research §3.5: installed-PWA `getUserMedia` regresses — the
voice mode detects a dead mic and offers "open in Safari" rather than
failing silently.

## 9. What stays Slack's — D9

The far door (works with the VPN down), the push nudge until Phase 5, and
the off-tailnet file conduit. Voice D11's `push_to_slack` and the
side-channel ruling are unchanged. Nothing is retired.

## 10. Phases

**Phase 1 — the door and the dashboard (read-only).** `mecha serve` with
header auth, serving the static build; Home rendering store reads only
(outbox/frontdoor/queues/tasks counts, doctor findings, today) via
`--json` child processes. No agent in the process yet. *Verify: a request
without the header is refused; a phone on the tailnet renders Home; a
phone off the VPN gets nothing.*

**Phase 2 — chat.** The agent moves in (or lands with voice's facade,
whichever ships first — coordinate). One session: streaming over SSE,
steering via queued input, cancel, the context gauge from reported usage,
session recording. *Verify: a run started from the phone appears in
`mecha sessions`; taint recorded; a second turn reads the cached prefix
(cache lens).*

**Phase 3 — the review verbs.** Outbox (list/show with `DraftView`, the
source quote, taint banner; send/reject; edit via the new `--body` verb),
graph-queue sample deck, tasks, frontdoor, notes capture. *Verify: every
button's effect is reproducible as the CLI command it drives; a tainted
send requires the on-screen confirm; nothing here works when the binary
lacks the store (dash, never zero).*

**Phase 4 — many sessions, ask, files.** The session rail over concurrent
`RunContext`s; `ask_user` with options + decline-on-timeout; live approver
mode; uploads to `inbox/`; authenticated downloads. *Verify: two sessions
stream concurrently without cross-talk; an unanswered ask records a
decline with the right wording; a download outside the jail 404s.*

**Phase 5 — voice merge and the installed app.** The in-chat voice mode
lands against the voice build's Pipecat worker (their Phases 1–3 are the
dependency); PWA install; Web Push for staged-draft nudges (research
§3.3), replacing nothing — Slack keeps carrying them too. *Verify: barge-in
cancels at a safe point; the end chime fires on a killed connection; a
push arrives with the VPN up and the page closed.*

## 11. Open at build time

- **Naming — RESOLVED 2026-08-24**: `mecha serve` won, and the unification
  landed the other way around from the plan: the voice arc mounted its
  facade *into* serve (`--voice-port`, `voice::Facade` over a
  `ChatState::voice_parts()` handle) — one process, one agent, one cached
  prefix, two dialects. `VOICE-RESEARCH.md` D2 was amended in the same
  change. `voice-serve` survives as a standalone spelling of the same
  implementation; the production switch to unified serve is sequenced with
  the reinstall.
- **axum vs hyper**, and **embedded static build vs directory** (§1).
- **Presence transport**: SSE everywhere vs one WebSocket — decide when
  the ask/approver work starts; SSE-first is the simpler default.
- **Push timing** stays research O3: Phase 5, Slack carries nudges until.

## 12. Owner feedback backlog — the first real day of use (2026-08-24)

Luke's notes from working the app on his phone, recorded verbatim in
intent and triaged here. Three were fixed the same evening (the
`grant-review` fossil in the voice disclaimer, the interim browser-speech
toggle a user read as a status light, the task sheet with no
tap-outside close). The rest, roughly by weight:

**Chat.**
- Tap the model chip to switch models, plus preset "thinking level"
  controls (the ChatGPT/Claude pattern). Constraint to design around: the
  serve process is ONE shared agent — a model switch rebuilds the agent
  and drops the cached prefix for every session and the voice facade at
  once, the same reason the Slack connector refused `set_approver`. And
  thinking budget on the local stack is a *server* flag
  (`--reasoning-budget`), not a per-request knob, so "thinking level" may
  mean provider choice rather than a slider. Needs a design pass, not a
  button.

**Mail.**
- A plain inbox view — recent mail regardless of triage state — beside
  the classified queue, and manual compose/reply (staged through the
  outbox like everything else). "The standard email functionality in
  addition to our agent-augmented version." Needs a `mecha mail recent`
  (reads via the MCP surface) and a `compose` verb that stages; the page
  is then the established thin shell.

**Notes.**
- Voice capture: dictate a note like a chat turn (candidates: browser
  SpeechRecognition as the cheap first cut, or a hold-to-talk that ships
  audio to Parakeet through serve — the STT is already running).
- A list of recent notes by created/last-touched, not just a capture box.
  Reads come through the kg tool surface.

**Tasks.**
- The horizontal status filters read as noise — prefer the collapsible
  left drawer (chat's pattern now). Underneath: the filters are the GTD
  statuses (actionable/scheduled/waiting/done); research how Things /
  OmniFocus / Todoist shape this before redesigning, per Luke.
- Voice capture, same as notes.
- **Task→agent handoff**: "I can't assign a task to the agent or prompt
  an agent to help complete it." The board is a list; the ask is a verb
  on each task that opens a chat session seeded with the task (or
  spawns a run whose result comes back for review). Flowmail did a
  version of this; Luke is open to research/brainstorm. The biggest item
  here and its own design doc when picked up.
- The per-task actions are unexplained ("a little bizarre") — revisit
  what a row offers and why.

**Home.**
- No mail count on the dashboard; cards should navigate to their surface
  on tap; longer-term, customizable widgets — the daily briefing as a
  card is the motivating example.

**Frontdoor.** Still no page — already an open handoff item.

The through-line worth keeping: every ask is a standard pattern from
apps Luke uses daily (session drawers, tappable dashboards, dictation,
plain inbox). The agent-augmented surfaces earn their keep only when the
ordinary affordances underneath them also exist.

## 13. The desktop window, and naming a conversation — D10, D11 (2026-09-01)

Two asks from the owner, working the app on a laptop rather than a phone.
The mockup canvas was nine phone screens and the build was faithful to it,
which meant a 1500px window rendered a 560px column with the rest of the
screen empty — and starting a fresh conversation lived inside the session
drawer, behind a modal that asked for a name.

**D10 — a wide window lays the same views out side by side; it is not a
second design.** Two breakpoints, and each one exists because a specific
thing stops fitting:

- **900px** — the bottom nav becomes a left rail (`order: -1` on the same
  element; the markup order stays phone-first, which is also the order a
  screen reader wants), and the shell's floating gear docks to the foot of
  that rail. Every view's own content stops stretching and keeps a reading
  measure.
- **1180px** — the chat's session drawer stops being a drawer and simply
  stays open. That is the only thing a phone could not afford, and the
  whole reason the drawer existed; docked, it is the same markup with the
  modal parts (scrim, slide-in, tap-to-close) left off.

Two things carry the implementation and are the parts worth not undoing.
The shell owns a `.viewport` element, because **views do not all have a
single root** — Home and Settings are a `<header>` beside a `<main>`, which
stacks correctly in a column shell and lays out *side by side* the moment
the shell becomes a row. And every view's side margin goes through
`--gutter` / `--gutter-gear` in `web/src/app.css` rather than through a
literal: a percentage inside a custom property resolves where it is *used*,
so one definition centres each view's content in whatever width that view
has — the chat's column is narrower than the board's by exactly the docked
panel, and neither needs to know that. Nine components had their own copy
of `20px`; this is `tui::list_height`'s argument in CSS.

**D11 — a conversation names itself, from the owner's own turns.** Minting
a key is the machine's job (`chat-8f3a`: unique and URL-safe, nothing
else); the *name* is derived after a run and re-derived as the conversation
grows — `mecha_core::title`, recorded as `Record::Title` and applied over
the header by `Session::read`. Three names in a session's life (owner turns
1, 3 and 8), because re-titling every turn spends a generation to move a
label nobody is looking at.

The invariant is the input, not the prompt: **the titler reads only what
the owner typed.** A title is rendered in the owner's session list, on
every surface, for as long as the session exists — a longer-lived display
than any single answer — so a model paraphrase of *third-party* text would
let a page fetched mid-conversation compose the label its own conversation
wears. User turns in a web session are bytes the owner typed or spoke;
summarising those is a paraphrase of the owner, and there is no channel.
The pass is a `QuarantinedPass` on top of that, and what comes back is
bounded to one line of 48 characters with control bytes stripped before it
reaches a record or a row.

Only `web: ` sessions are renamed. A delegation already carries the task's
name and `task_withholding` reads that title; a rename carries the prefix
for the same reason. `titled_at` is deliberately not persisted — a resumed
session earns its name once more from the whole discussion, which is
cheaper than a second record that can disagree with the one beside it.
