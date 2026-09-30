# Features — design

> **Status (2026-09-30):** designed, not built. §7 holds the rulings the
> build waits on; nothing in §4–§6 should be built before they are made.

**2026-09-30.** One question: *how does a new user install only the parts of
mecha they want — and how do `mecha setup`, the web app, the CLI, the API and
the tool list all agree about which parts those are?*

The owner's framing, the same day:

> "I would like the [system] to be fairly modular. For example slack should be
> optional, image generation, personas, ocr, voice, and other features. If
> users don't enable that feature it shouldn't show up in the web ui. Cli could
> say not enabled. Are we making model recommendations too for the additional
> features?"

and, a turn later: *"Web should also be optional feature."*

§1 is what exists today, read from `origin/main` at `259231d6`. §2 is how
eight other systems handle the same problem, with sources. §3–§6 are the
design. §7 is the rulings. §8 is what this deliberately does not do.

---

## 0. The short answer

| Question | Answer | Section |
|---|---|---|
| How many features does `mecha setup` cover today? | Four integrations (mail, docs, Slack, graph), plus the provider, the timezone, the charter and the scheduler. Image, documents/OCR, voice, personas, search, the web app and the front door have no step at all | §1.1 |
| Does the web app hide what is off? | No. `Nav.svelte`'s `items` are hard-coded and every one is `true`; an off feature's tab renders, and its routes fail in one of five different ways | §1.2 |
| Are there model recommendations? | For the chat model only (`getting-started/hardware.md`, four memory tiers). None for OCR, image generation, voice or embeddings — and the embeddings page names the wrong model | §1.3 |
| What do other harnesses do? | A closed registry of features in code; one `list` command with a typed status per row; hide rather than refuse in the UI; declared requirements; a section per re-runnable setup step; per-function model slots with a local default | §2 |
| Lighter installs for experiments? | Already the default shape once features are off until configured: the heavy parts are services, not the binary. A trial records its feature set beside `levers_off`, and an environment can require features | §5.1 |
| What does this design add? | One closed `Feature` registry in `mecha-core`, read by six consumers: setup, `mecha features`, `/api/features`, route guards, CLI guards and tool registration | §3–§5 |
| What turns a feature on? | Its config table being present, global file only (recommended; ruling F1) | §4.1 |
| What does an off feature look like? | Hidden in the web app, 404 `feature_off` from its routes, one sentence and the `mecha setup <feature>` command from its CLI verbs, absent from the tool list. A feature that is *configured but not answering* is **never** hidden | §4.2 |
| Model recommendations? | Yes, as data on each feature: model, download command, memory, and whether it was **measured** or is arithmetic. Printed, never written into config | §6 |

---

## 1. What is here today

Read from `origin/main` at `259231d6` (2026-09-30), by symbol.

### 1.1 Setup covers the old integrations and none of the new ones

`onboarding::plan` builds the list `mecha setup` shows. Its integration part,
`onboarding::integration_steps`, has four steps — `mail`, `docs`, `slack`,
`graph` — each built with `.optional()`, so each can be declined into
`~/.mecha/setup-declined.json` (`onboarding::decline`) and taken back with
`mecha setup --undecline`. The rest of `plan` is the config file, the
provider, the local server checked against its own `/props`, the timezone,
the charter and the scheduler.

Nothing in it mentions `[image]`, `[documents]` (OCR and layout), voice,
personas, `[[search]]`, the web app, the embeddings server, or the front door.
Two of the four steps that do exist check the wrong thing: `mail` and `graph`
look for a binary on PATH (`onboarding::on_path`) and never for the `[[mcp]]`
entry that actually puts the tools on the surface — an installed binary with
no entry reads as done.

The decline machinery is already right, and this design keeps it:

- A decline is per step id, stored in the mecha home. A repository cannot
  decline for you.
- A step that ships *after* you declined others is simply `Missing`, so it is
  offered. Hermes had to add `known_builtin_toolsets` and
  `_RECENTLY_SHIPPED_TOOLSETS` to get the same property (§2.1).
- A decline never hides `Done`, `Wrong` or `Unknown` — "I don't want mail" is
  not "don't tell me my mail is broken".

### 1.2 The web app shows everything, and off fails five different ways

`web/src/lib/Nav.svelte` holds `items` as `[label, svgPath, enabled]` tuples.
All eight are `true`. The `.disabled` class and its "coming in a later phase"
tooltip exist and are never used. `App.svelte`'s `views` render with no
configuration check. The only configuration-aware places in the app are
Settings → Voice (`/api/settings/voice`) and `ModelChip` (`/api/model`).

There is no `/api/features`. When a feature is not configured, its routes fail
in whichever way the code underneath happens to fail:

| Response | Where |
|---|---|
| **503** | chat, personas, `/api/today`, `/api/entity/{create,merge}` |
| **502** | mail reads, graph reads, the graph queue, `/api/dictate`, `/api/offer` |
| **409** | every write that runs a child verb — `review::verb_output` maps any failed child to CONFLICT |
| **501** | voice cloning without `[web] voices_dir` |
| **200, empty** | `/api/mail` (no triage store), `/api/questions`, `/api/frontdoor` — which also **creates** `~/.mecha/requests` as a side effect |

`/api/library*` never checks `[image]` at all: the Library tab works as a
store browser on an install that cannot draw. The voice-call button in Chat
and `Dictate.svelte` in Graph and Tasks are always shown.

### 1.3 Recommendations: the chat model only, measured on one machine

`website/docs/getting-started/hardware.md` is good: memory arithmetic, four
tiers (16 / 32 / 64 / 128 GB), and a measured table for the GB10 with
Qwen3.6-35B-A3B at Q4_K_M. It is careful to say that every throughput figure
comes from that one machine.

Every other model mecha runs is named somewhere, but only as *what this
machine runs*, never as advice:

| Feature | Model | Where it is written down |
|---|---|---|
| Embeddings (graph) | `harrier-oss-v1-0.6b` f16 | `scripts/llama/mecha-embed-server`, `LLAMA-SERVER.md`. **Drift:** `website/docs/features/memory/graph/index.md` still says ollama with `nomic-embed-text` |
| OCR | PaddleOCR-VL 1.6 (GGUF + mmproj) | a comment in `scripts/llama/install.sh`, `DOCUMENT-EXTRACTION-DESIGN.md` §5–6 |
| Layout | `PP-DocLayoutV3.onnx` | `scripts/layout/install.sh` |
| Image generation | Qwen-Image 2.1 Q4, `qwen3vl_8b_w4a8`, the 2.1 VAE | `ImageConfig` defaults, `features/tools/image-generation.md` (~15 GB peak, measured) |
| Voice | Parakeet TDT 0.6B v3 int8, Chatterbox Turbo, Silero VAD, smart-turn v3 | `features/interfaces/voice.md`, `VOICE-RESEARCH.md` |
| Vision | the chat model's own `mmproj` | `LLAMA-SERVER.md` §Vision |
| Personas | the chat model; the judge model is open | `PERSONA-DESIGN.md` §12.2 (R19) |

### 1.4 Install is this machine's install

What a new user cannot reproduce from the repository:

- The router unit (`llama-local.service`), ComfyUI and Chatterbox have no unit
  or compose file in the repo.
- The voice, serve, front-door and ruminate units hard-code `/home/ljchang`
  or a checkout path.
- The Parakeet URL and the voice worker's URL are literals in `serve::dictate`
  and serve's flags, not config.
- `scripts/start-e4b.sh` binds :8081, the embedder's port.

`scripts/llama/install.sh` (OCR) is the exception and the pattern to copy: it
copies rather than symlinks, enables a socket, and has `--remove`.

---

## 2. How other systems do it

**Researched 2026-09-30.** Two passes, both reading primary sources.

| Project | What was read | Strength |
|---|---|---|
| [hermes-agent](https://github.com/NousResearch/hermes-agent) | source at `02e41181` (`hermes_cli/setup.py`, `tools_config.py`, `doctor_tools.py`, `tool_availability_notices.py`) and its docs site | source |
| [openclaw](https://github.com/openclaw/openclaw) | source at `2663c6b6` and `docs.openclaw.ai` (wizard, configure, plugins, skills, doctor, control UI, llama-cpp) | source + docs |
| [codex](https://github.com/openai/codex) | `codex-rs/features/src/lib.rs` and `cli/src/main.rs` at `ab84d71f`, the config docs | source |
| Claude Code | plugin manifest, settings and MCP reference pages | docs |
| Open WebUI | env configuration reference, `MessageInput.svelte` | docs + source |
| LibreChat | `librechat.yaml` reference (`interface`, agents, speech, OCR) | docs |
| Goose | extensions docs | docs |
| Home Assistant | integration manifest, config flows, config entries, repairs | developer docs |
| LM Studio, Ollama, Jan, llmfit | load/fit docs; llmfit's `how-it-works.md` | docs |

Issue citations were spot-checked with `gh issue view` (openclaw #75279,
#146433; hermes #120832, #13024 — all exist with the quoted titles). The
rest are as the research pass reported them.

### 2.1 The two personal-assistant harnesses

**Hermes** has no single features table. A feature is a *toolset* enabled per
platform (`platform_toolsets.<platform>`, written by the `hermes tools`
picker), minus a global `agent.disabled_toolsets`, plus per-session
`--toolsets` and mid-session `/tools enable|disable`. Some toolsets are off by
default (`_DEFAULT_OFF_TOOLSETS`: homeassistant, spotify, discord, video, …).
Tools declare `check_fn` and `requires_env`; a tool whose check fails is left
out of the model's schema, and startup prints a notice naming the fix ("Web
search is off — no search provider is set up yet … Run `hermes setup
tools`"). Setup is sectioned and each section re-runs alone (`hermes setup
model|tts|terminal|gateway|tools|…`); a first run offers Quick, Full, or
**Blank Slate**, which turns on only the model, files and a terminal.
Per-function models live in `auxiliary.<task>.{provider,model,…}` (vision,
compression, title generation, approval, …), defaulting to the main model.
The desktop app has a hardware-tiered local catalog that keeps models that do
not fit **visible, with the reason**.

Its recurring complaint is configuration that is silently ignored:
`auxiliary.*` overrides not honoured (#109111, #125707),
`delegation.orchestrator_enabled: 0` read as on (#120832), `/voice on`
refusing silently (#120118), a quoted-string plugin list leaving every plugin
unmounted until doctor learned to flag it. Its security scanner defaults to
**fail open** (`security.tirith_fail_open: true`).

**OpenClaw** is closest to what §4 proposes: **a config section being present
turns it on** ("each channel starts automatically when its config section
exists, unless `enabled: false`"). Plugins carry a manifest with
`enabledByDefault`; skills declare `requires.{bins, anyBins, env, config}`,
`os` and install specs, and are filtered at load; `openclaw skills check`
lists *missing requirements* apart from *disabled* and *blocked*. The Control
UI hides mutation actions the gateway does not advertise and shows a dash, not
a zero, for a missing metric. `openclaw configure --section …` re-runs any
wizard section. `openclaw doctor` is a detect/repair contract — stable check
ids, a fix hint, re-detection after a repair, `--lint` as a read-only CI gate
— and plugins register their own checks. Model choice has named slots
(`imageModel`, `pdfModel`, `utilityModel`, `voiceModel`, …) and the llama.cpp
plugin ships hardware recipes (Qwen3.5 4B at 8 GiB up to Qwen3.8 27B at
32 GiB + GPU), reserving memory for the embedder and verifying a real tool
call before switching.

Its complaints are the cost of the same choices: a fresh Windows install with
"66 plugins loaded, 49 disabled, 48 skills missing requirements" and 3–7
minute responses (#75279); a requirement check run on a different PATH from
the user's shell, reporting working binaries as missing (#80206); and an
allowlist whose entries were all invalid collapsing to **unrestricted**
(#146433).

### 2.2 The others, briefly

- **Codex** is the cleanest registry: `FEATURES: &[FeatureSpec]` with
  `{id, key, stage, default_enabled}` and stages `UnderDevelopment →
  Experimental → Stable → Deprecated → Removed`; `[features]` in
  `config.toml`; `codex features list | enable | disable`; dependencies
  normalised in code (`CodeModeOnly` forces `CodeMode`). Its weakness: an
  unknown key is only a `tracing::warn!`, so a typo is a feature silently
  off.
- **Claude Code**: `enabledPlugins` layered user → project → local, a plugin
  manifest with `dependencies` (disabling one another plugin needs is refused
  with the chained command), and `claude mcp list` giving one typed status per
  server — connected, needs authentication, failed, pending approval,
  rejected, disabled — each naming the fix. A repository cannot enable the
  dangerous things: several keys apply only from user or managed settings.
  A reload that would invalidate the prompt cache waits unless forced.
- **Open WebUI** shows a control only when three things hold: the global flag,
  the user's permission, and every selected model supporting it
  (`MessageInput.svelte`). "Hidden rather than offered and refused."
  `ENABLE_PLUGINS=False` removes the pages *and* the permission toggles that
  governed them. Each feature has an engine setting with a local default
  (Whisper `base`, `all-MiniLM-L6-v2`). Its trap: env vars are read once and
  then the database wins, silently (#20830).
- **LibreChat** has an `interface:` block to show or hide UI and a
  capabilities list for agents (`…, "ocr"`); OCR falls back to a local
  `document_parser` when unconfigured. Its trap: YAML seeds database
  permissions and later clobbers them (#12306).
- **Home Assistant** is the mature version of this problem: a manifest with
  `dependencies` and `requirements` ("if steps fail … the component will fail
  to load"), config flows with `reauth` and `reconfigure`, per-entry states
  that distinguish **setup error** from **setup retry**, and *Repairs* — a
  persistent list of fixable issues, badged on Settings. It recommends
  hardware per voice model (Whisper `base` wants at least an Intel N100).
- **Fit tools.** llmfit makes fit "a pure function of one number",
  `memory_required / memory_available`, banded (≤60 % Perfect … >98 % Too
  Tight), labels every estimate `calibrated` or `estimated`, and reports an
  unknown as `null`, deliberately not `0.0`. It dropped a `size × 2`
  heuristic after it called a 96 %-full load Perfect. Jan shows a fit pill and
  downloads nothing to decide it. LM Studio's `--estimate-only` prints the
  estimate before loading.

### 2.3 What that means for mecha

**Take:**

1. **A closed registry in code** (Codex, Kubernetes feature gates). The binary
   knows every feature; config only chooses among them. This is also the
   answer to "can a plugin add a feature" — no, for the same reason triggers
   and skills never come from layered config.
2. **One list command with a typed status and the next action per row**
   (`codex features list`, `claude mcp list`, `openclaw skills check`).
3. **Hide rather than refuse in the UI, and enforce underneath** (Open
   WebUI). Hiding alone is presentation.
4. **Requirements and dependencies are data, checked when a feature is turned
   on** (Home Assistant, Claude Code, OpenClaw `requires`).
5. **Every setup step can be re-run on its own** (`hermes setup <section>`,
   `openclaw configure --section`).
6. **One row per sub-capability**, so a half-configured feature cannot look
   healthy (Hermes reports web search and web extract separately).
7. **Fit as a ratio with a confidence label, unknown as `null`** (llmfit).
8. **Keep models that don't fit visible, with the reason** (Hermes, Jan).

**Reject:**

1. **Guards that fail open or fail empty** — Hermes's scanner, OpenClaw's
   all-invalid allowlist. mecha's rule already forbids both, and every guard
   here must fail closed.
2. **Configuration silently ignored** — Codex's unknown key, Hermes's
   ignored overrides, Open WebUI's env-versus-database. mecha's `ConfigLayer`
   is already `deny_unknown_fields`; any new table keeps that.
3. **Two sources of truth for one switch** — Open WebUI's env *and* database,
   LibreChat's YAML *and* roles. The enabling fact lives in one place.
4. **Many parallel switches for one feature** — Hermes has per-platform lists,
   a global disable, a flag, a slash command and per-tool toggles.
5. **Lots of it on by default** — OpenClaw's plugins-on-by-default cost is
   #75279. In mecha everything in §5 is off until its table exists.
6. **A requirement check run somewhere other than where the feature runs**
   (#80206). A probe says where it ran.

---

## 3. What counts as a feature

A **feature** is something a new user could reasonably not want, whose
absence the rest of mecha survives, and which has a surface to hide: a web
tab or card, a CLI verb, a tool, or a service. Four things are **not**
features, and stay out of the registry:

- **The core** — a provider, the agent loop, sessions, the outbox store, the
  path jail. mecha does not run without them.
- **The sandbox** — a security posture, not a feature. `Sandbox::preflight`
  already stops a run whose configured backend does not work; putting it
  behind an on/off switch would give it a second, weaker way to be off.
- **Harness levers** (`harness::Lever`: learned rules, hooks, skills, the
  charter, boredom, step checks, …) — behaviours of the loop, already a
  closed set with its own on/off machinery that `mecha eval` depends on.
  They stay levers.
- **Model capabilities** — vision is something a *model* has, detected from
  `/props` and `ProviderConfig::vision_enabled`, not something a user turns
  on. It appears under the chat model's row, not as a feature.

---

## 4. The design

### 4.1 One registry, in `mecha-core`

A new module, `feature.rs`, holds a closed enum `Feature` and one static
description per variant:

```rust
pub struct FeatureSpec {
    pub id: &'static str,           // "image" — the wire and CLI name
    pub label: &'static str,        // "Image generation"
    pub requires: &'static [Feature],
    pub parts: &'static [Part],     // sub-capabilities, each with its own state
    pub recommend: &'static [Recommendation],   // §6
}
```

and one function that is the only place "is it on?" is answered:

```rust
pub fn state(cfg: &Config, facts: &Facts, f: Feature) -> FeatureState
```

`FeatureState` keeps apart the things a bool would merge:

| State | Meaning | Web | Routes | CLI | Tools |
|---|---|---|---|---|---|
| `Off` | not configured (or declined) | hidden | 404 `feature_off` | one sentence + the setup command | not registered |
| `Blocked(Feature)` | configured, but something it needs is off | hidden, and Settings says what it waits on | 404 `feature_off`, naming the dependency | names the dependency | not registered |
| `Unready(reason)` | configured, not answering | **shown**, with a banner | 503 with the reason | the reason | registered |
| `On` | configured and, where probed, answering | shown | normal | normal | registered |
| `Unknown(reason)` | could not be read | **shown**, with a banner | normal | warns | registered |

Three rules carry the design:

- **Tools follow configuration, never liveness.** `Unready` still registers
  its tools. The tool list is the front of the cached prefix and is built
  once per session; a server that is down for a minute must not change the
  bytes of every request after it. A tool whose server is down already
  returns `is_error` and the model routes around it.
- **Configured-but-broken is never hidden.** This is the onboarding rule — a
  decline never hides a failure — carried to the web app. Hiding a feature
  the owner turned on because its server is down is a silently-degrading
  guard: the owner would read "not there" as "not configured".
- **Unknown is never clean.** An unreadable store or credential file is
  `Unknown`, shown with a banner, never `Off`.

### 4.2 Six readers, one answer

1. **`mecha features`** (new; `--json`). One row per feature and per part:
   state, the reason, and the next command. Modelled on `codex features list`
   and `claude mcp list`. With `--probe` it checks reachability; without it,
   it reads only config and the disk.
2. **`mecha setup`** iterates the registry instead of the hand-written
   `integration_steps`. Every optional feature becomes a declinable step, and
   **`mecha setup <feature>`** runs just that one (Hermes and OpenClaw both
   ended up here). A step for a feature whose dependency is off says so and
   offers the dependency first, as Claude Code does when a disable would
   break another plugin.
3. **`GET /api/features`** returns the same rows. `Nav.svelte`'s `enabled`
   column becomes that answer rather than a literal, and `Off` and `Blocked`
   entries are **removed**, not greyed out — the existing greyed-out style
   stays for `Unready`. Home cards, the voice-call button and `Dictate` take
   the same test. A direct link to a hidden view (`#library`) lands on Home
   with a one-line notice rather than an empty page. **Settings → Features**
   lists everything, including `Off`, with the command that turns each one on —
   hidden from navigation, never undiscoverable.
4. **Route guards.** Each API route group declares its owner, and one axum
   layer answers for all of them, so the five ways an off feature fails today
   (§1.2) become one. Side-effecting reads (`/api/frontdoor` creating its
   store) stop happening for a feature that is off.
5. **CLI guards.** A verb that belongs to a feature calls
   `feature::require(cfg, Feature::Image)?` first, and every verb says the
   same thing the same way: *"image generation is not enabled — `mecha setup
   image`"*.
6. **Tool registration** in `setup::prepare_tools` asks the registry rather
   than repeating `if let Some(image) = cfg.image`. Most of these gates are
   already correct (§1.1); the change is that they share one predicate with
   the other five readers, so they cannot drift apart.

`mecha doctor` stays what it is — no network, no model, the stores' distress.
A feature that is `Unready` is `mecha features --probe`'s to report, not the
doctor's.

### 4.3 Probing without waking anything

Several servers here are **started by being asked**: OCR and embeddings are
socket-activated, the router loads any model a request names unless
`autoload=false` is passed, and ComfyUI loads weights on first use. A probe
that wakes the thing it probes turns `/api/features` into a way to fill
memory from a page load. So:

- `/api/features` reads **configuration and the disk only** — config tables,
  binaries on PATH, credential stores, installed unit files. It never opens a
  socket. Its states are `Off`, `Blocked`, `On (not probed)` and `Unknown`.
- `mecha features --probe` and `mecha setup` may probe, using only calls that
  load nothing: `served_props` / `GET /models` against the router, the
  systemd unit state for a socket-activated server, ComfyUI's
  `/system_stats`. A server that runs on demand reports **"on demand"**, which
  is a state, not a failure.
- Every probe row says **where** it ran (#80206).

---

## 5. The catalogue

What each feature is, what turns it on under ruling F1, and what hides when
it is off. "Today" is the current switch; "Proposed" is the one §4 reads.

| id | Feature | Today | Proposed switch | Requires | Hidden when off |
|---|---|---|---|---|---|
| `web` | The web app (`mecha serve`) | `[web] owner_login` set; serve refuses without it | unchanged — `owner_login` present | — | everything web; `mecha serve` refuses with the setup command |
| `slack` | Slack remote control | tokens in `~/.mecha/slack`, `mecha-slack.service` | tokens present (the `[slack]` table stays tunables-only) | — | `mecha slack …` verbs except `auth` |
| `mail` | Mail and calendar | `mecha-mail` on PATH + accounts; tools via `[[mcp]]` | an enabled `[[mcp]]` entry exposing `mail_*` (the fact setup does not check today) | — | Mail tab, Home mail card, Outbox event editor, `mecha mail` |
| `docs` | Google Docs, Sheets, Slides | `mecha-docs` + account + `[[mcp]]` | an enabled `[[mcp]]` entry exposing it | — | its tools |
| `graph` | Knowledge graph | `mecha-graph-mcp` on PATH + `[[mcp]]` | an enabled `[[mcp]]` entry exposing `kg_*` | — | Graph tab, Review → graph queue, Proposals → entities, `kg`, `gossip`, `corroborate`, `vet`, `distill` |
| ↳ `tasks` | The task board | — (rides the graph) | part of `graph` | `graph` | Tasks tab, Home tasks card, `tasks`, `workflow`, `questions` |
| `search` | Web search and open | `[[search]]` non-empty | unchanged | — | `web_search`, `web_open` |
| `documents` | PDF extraction | `[documents]` present | unchanged | — | `document_read`, `mecha document` |
| ↳ `ocr` | OCR pages | `[documents] ocr` | unchanged | `documents` | its row in `features` |
| ↳ `layout` | Region-by-region layout | `[documents] layout` | unchanged | `ocr` | its row in `features` |
| `image` | Image generation | `[image]` present | unchanged | — | `image_generate` |
| ↳ `library` | Characters and styles | always on | part of `image` | `image` | Library tab and Home card, `image_library*`, `mecha imagelib` writes |
| `personas` | Characters the owner talks to | always on (store) | **new** `[personas]` table | — (web for the tab) | Personas tab, `/api/personas*`, `mecha persona` |
| `voice` | Talking to mecha | serve flags, literals, units | **new** `[voice]` table (see below) | `web` | voice-call button, Dictate, Settings → Voice |
| ↳ `dictate` | Speech to text | Parakeet at a literal URL | `[voice] stt_url` | `voice` | Dictate |
| ↳ `calls` | Spoken conversation | `--offer-target`, the worker | `[voice] offer_target` | `voice` | voice-call button |
| ↳ `cloning` | New voices | `[web] voices_dir` | `[voice] voices_dir` | `voice` | Settings → Voice → clone |
| `incognito` | A chat that leaves no trace | always on in web | derived: `web` on and a local provider | `web` | Chat's incognito toggle |
| `frontdoor` | Inbound requests, publishing, polls | `factory-publish`, units | `[outbox] publish_tools` non-empty | `mail` | Review → Front door, Home card, `frontdoor`, `polls` |
| `messages` | Messages between sessions | `[messages] enabled` | unchanged (see F1) | — | `message_send`, `mecha msg` |

Notes on the rows that change:

- **`voice` gets a table** because today it has none: its switches are serve
  flags (`--voice-port`, `--offer-target`), a literal in `serve::dictate`,
  and `[web] voices_dir`. A `[voice]` table holds all of them, so the
  Parakeet and worker URLs become config for the first time — which a new
  user needs anyway, since theirs will not be this machine's. `voices_dir`
  moves from `[web]` with a one-release alias.
- **`personas` gets a table** because it has no config at all; its switch has
  to live somewhere. Once it exists, it is also where the persona safety
  settings belong (the crisis-pause cooldown, `PERSONA-DESIGN.md` §16).
- **`frontdoor`'s switch is a guess** — the factory side has its own binary
  and units, and which fact best means "this install has a public surface"
  is for whoever owns that arc to confirm.

Every new table is **global-file only**, like `[web]`, `[image]` and
`[documents]` today — `merge_file` strips them from project layers — so a
cloned repository can never turn a feature on. And each is **three edits**,
not two: `Config`, `ConfigLayer`, and `ConfigLayer::apply`.
`every_field_of_config_is_reachable_from_a_file` and
`every_field_a_layer_can_read_is_a_field_a_layer_applies` catch a missed one
at the top level only, so the build step adds a nested-layer test alongside.

`mecha config init` writes every optional table **commented out**, each with
one line on what it turns on, so the starter config doubles as the feature
list.

### 5.1 Light installs, and experiments

The owner, 2026-09-30: *"I would like to be able to do lighter installs when
running experiments where we likely won't need many of the heavier and
complex features."*

**A light install is the default shape, not a mode.** Everything heavy in
§5 is a *service with a model*: the router's presets, ComfyUI (~15 GB peak),
Chatterbox in docker, Parakeet, the OCR and embedding servers. None of it is
compiled into `mecha`: the binary is ~47 MB, `mecha-cli` has no cargo
features, and it links no ML runtime. So `cargo install mecha-cli` plus a
provider is already the light install; what makes an install heavy is which
services were set up next to it, and under this design that is exactly the
set of tables present. Compile-time cargo features would buy little today
and are out of scope (§8) until a heavy Rust dependency lands.

Three things make it deliberate rather than accidental:

- **`mecha setup --minimal`** declines every optional step in one pass
  (Hermes's *Blank Slate*). It writes declines, never config, so `mecha setup
  <feature>` still turns any one on later.
- **A trial's features are a condition of the trial.** A trial home's config
  already comes from its environment (`trial_env`), so its features follow
  that environment's tables with no second switch. But which features were on
  is as much a condition as which levers were off, so the experiment manifest
  and the session record carry the feature set beside `levers_off`, from the
  same registry — otherwise two arms that differ only in whether `[[search]]`
  was present read as identical.
- **An environment declares what it needs.** An experiment environment may
  name the features its tasks need (`requires = ["search", "documents"]`),
  and a trial whose home has one of them `Off` or `Blocked` refuses to start
  and says which. Running anyway would score the model on a task it could not
  attempt, and record that as a result.

---

## 6. Model recommendations

Yes — and they belong in the registry, not in prose scattered across six
pages, because a recommendation written in two places is two
recommendations (the embeddings page already disagrees with the server).

Each feature carries `Recommendation` rows:

```rust
pub struct Recommendation {
    pub tier_gb: u32,             // 16, 32, 64, 128 — hardware.md's tiers
    pub model: &'static str,      // "PaddleOCR-VL 1.6 (GGUF + mmproj)"
    pub fetch: &'static str,      // "hf download PaddlePaddle/PaddleOCR-VL-1.6-GGUF"
    pub peak_mb: Option<u32>,     // None = not measured, never 0
    pub residency: Residency,     // Resident | OnDemand | PerRequest
    pub evidence: Evidence,       // Measured { machine, date } | Arithmetic | Unmeasured
}
```

Rules:

- **Printed, never written.** `mecha setup <feature>` prints the row for this
  machine's tier and the download command. It does not write a model name
  into config: the config value is read back from the running server
  (`mecha setup --write`), per `onboarding.rs`'s rule that setup never writes
  down a number the user merely believes.
- **Evidence on every row.** Only the GB10 has measurements. Rows for other
  tiers are `Arithmetic` (from `hardware.md`'s formula) or `Unmeasured`,
  and the output says which. llmfit's `calibrated` / `estimated` split is the
  model; its `null`-not-zero rule is ours already.
- **Rows that don't fit stay visible, with the reason** — "needs ~15 GB peak;
  this machine has 9 GB available" — rather than disappearing (Hermes, Jan).
- **The budget is the sum, not the row.** The real question on a 32 GB
  machine is not "does image generation fit" but "does it fit *beside* the
  chat model". `mecha features --probe` adds up the resident and peak memory
  of everything enabled and reports llmfit's ratio band against the machine's
  total, with `null` for any feature whose peak is unmeasured. This is why
  `residency` is on the row: an on-demand OCR server costs nothing until a
  PDF arrives; an image generation borrows ~15 GB for its duration (and
  `[image] min_available_mb` already refuses one that would not fit).
- **`hardware.md` is generated from the rows, or checked against them.** A
  test that fails when the page and the registry disagree ends the drift
  that §1.3 found.

The first rows to write are the ones already measured here — chat
(Qwen3.6-35B-A3B Q4_K_M, ~28.5 GB for one 262k slot), image generation (~15 GB
cold peak at 1024²) — and then the models named in §1.3 as `Unmeasured` until
someone measures them. What a 16 GB user should run for OCR or voice is
genuinely not known yet, and the output must say so rather than guess.

---

## 7. Rulings the build waits on

| # | Decision | Options | Recommendation |
|---|---|---|---|
| **F1** | What turns a feature on | (a) its table being present, global file only, with `enabled = false` to keep settings while off; (b) a central `[features]` table of bools, as Codex has; (c) both | **(a).** One fact per feature, in the table that holds its settings — no second switch to disagree with (§2.3, reject 3–4). `[messages] enabled` becomes presence-based for consistency, with the old key read for one release |
| **F2** | Off in the web app | (a) removed from navigation; Settings → Features lists everything; (b) greyed out with a tooltip | **(a)**, as the owner asked. `Unready` and `Unknown` are shown with a banner, never removed (§4.1) |
| **F3** | Web as a feature | (a) optional like the rest: no `owner_login`, no web — CLI, TUI and Slack are complete without it; (b) always installed, just unstarted | **(a).** It is already true in the code (serve refuses without `owner_login`); the change is that setup offers it as a step and the features that need it — voice, incognito, the Personas and Library tabs — report `Blocked(web)` instead of existing with nowhere to appear. `mecha persona` still works from the CLI |
| **F4** | What an off route returns | 404 / 403 / 409 / 503 | **404** with `{"error":"feature_off","feature":"image","fix":"mecha setup image"}` — on this install the route does not exist. 503 is kept for `Unready`, where it is true |
| **F5** | Recommendation tiers | `hardware.md`'s four (16/32/64/128 GB), or finer | **The four**, so one page and one table agree |
| **F6** | Existing installs, when the switch changes | (a) a feature whose new table is absent is off — this machine's personas and voice would disappear until the tables are added; (b) `mecha setup` detects a feature in use (a non-empty persona store, a running voice unit) and offers to write its table; (c) grandfather features in use as on | **(b).** Never (c): inferring "on" from a store is exactly the second source of truth F1 rejects. On this machine the deploy that ships step 4 adds the two tables by hand, in the same change |

---

## 8. Out of scope

- **Plugins, or features from anywhere but the binary.** The registry is
  closed. A repository, a skill or an MCP server cannot add a feature or turn
  one on — the same reason triggers and skills live only in `~/.mecha/`.
  (MCP servers stay what they are: tools, subject to the interlock.)
- **Turning features on from the web app.** Settings → Features shows each
  command; it does not run it. Enabling most features means installing a
  binary, authorising an account or starting a server, which is a terminal's
  job on a machine the owner is proving they control — the same argument
  that binds the Slack owner by a nonce printed on this machine. Worth
  revisiting for the ones that are pure config (`personas`).
- **Per-user permissions.** mecha has one owner. Open WebUI's third condition
  (the user's permission) has no counterpart here.
- **Lifecycle stages** (`Experimental`, `Deprecated`, …). Codex and
  Kubernetes need them for a large user base; mecha can add a `stage` field
  the day it has a feature it wants to ship turned off by default for
  everyone. Not before.
- **Compile-time cargo features** for a smaller binary (§5.1): the weight is
  in services and models, not in the binary.
- **Downloading models, or detecting the machine's hardware beyond total and
  available memory.** Setup prints the command; the user runs it.
- **macOS and Windows install paths.** §1.4's units are Linux user units;
  what replaces them elsewhere is its own question.
- **The harness levers** (§3). They already have their own closed set.

---

## 9. Build order

Each step is a PR, and each leaves every surface working.

0. **`feature.rs` and `mecha features`**, read-only. The registry, `state`,
   and the list command. Nothing else changes; its output on this machine is
   checked by hand against §5.
1. **`/api/features` and the web app.** Nav, Home cards, the voice button and
   Dictate take their answer from it; Settings → Features. No route changes
   yet, so a stale page still works.
2. **One guard for routes and CLI verbs.** Every route group and verb names
   its owner; the five failure shapes become `feature_off` / 503. The
   side-effecting reads stop.
3. **Setup iterates the registry.** A step per feature, `mecha setup
   <feature>`, `mecha setup --minimal`, dependencies offered first, and the
   `[[mcp]]` checks that `mail` and `graph` are missing today.
   **Experiments** record the feature set beside `levers_off`, and an
   environment's `requires` refuses a trial that lacks one (§5.1).
4. **The new tables**, after F1 and F6: `[voice]` (with the URLs out of the
   code), `[personas]`, `[messages]` to presence. Three edits each, plus the
   nested-layer test. The owner's config gets both tables in the same deploy.
5. **Recommendations**: the rows, the probe that sums memory, and a test that
   `hardware.md` matches them. Fix the embeddings page.
6. **Installers**: one `scripts/<feature>/install.sh` per feature that needs a
   service, each with `--remove`, copying rather than symlinking (the
   `scripts/llama/install.sh` pattern), with no `/home/<user>` or checkout
   path in any unit. The router, ComfyUI and Chatterbox get units in the repo
   for the first time.

### How to know it works

- A table-driven test over the registry: for each feature, build a config
  without it and assert that its tools are absent from `mecha tools --json`,
  its verbs exit with the one sentence, its routes return 404 `feature_off`,
  and `/api/features` reports `Off`.
- **Every route belongs to exactly one feature or to the core.** The only
  route list today is written by hand inside a test
  (`the_settings_routes_sit_behind_the_owner_guard`), so step 2 declares each
  route's owner where it is registered, and a test walks that declaration so
  an unowned route fails the build — otherwise the next tab added is visible
  on every install.
- The negative is not vacuous: the same test with each feature **on** must
  see the tools, the verbs and the routes. A guard that refuses everything
  passes the first test.
- `/api/features` opens no socket: a test with a listener on the OCR port
  asserts that it was never connected to.
