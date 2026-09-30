# Features — design

> **Status (2026-09-30):** this design merged as #427; **step 0 shipped** —
> the registry and a read-only `mecha features`, merged as #428 and deployed
> the same day, with the store-location fixes it surfaced in `doctor`, Slack
> and the booking sweep as #432 and #433. `docs/ARCHITECTURE.md` §Features
> describes what is built. Steps 1–8 are unbuilt. The owner
> ruled F1–F6 the same day (§7): the switch is a `[features]` table of
> bools — not a table's presence, which this doc first recommended — and §5
> is written to that ruling; F5 is `hardware.md`'s four tiers, in two
> columns (unified memory, and a separate GPU beside system RAM). Step 8 (how
> to add a feature, in `ARCHITECTURE.md` and `CLAUDE.md`) is the owner's
> addition.

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
| Lighter installs for experiments? | Already the default shape, since every `[features]` bool ships `false`: the heavy parts are services, not the binary. A trial records its feature set beside `levers_off`, and an environment can require features | §5.1 |
| What does this design add? | One closed `Feature` registry in `mecha-core`, read by six consumers: setup, `mecha features`, `/api/features`, route guards, CLI guards and tool registration | §3–§5 |
| What turns a feature on? | A bool in one `[features]` table, global file only, every feature listed so a user can see what exists (F1, ruled). A settings table is only settings; enabled without them is *unready*, shown with the fix | §5 |
| What does an off feature look like? | Hidden in the web app, 404 `feature_off` from its routes, one sentence and `mecha features enable <feature>` from its CLI verbs, absent from the tool list. A feature that is *configured but not answering* is **never** hidden | §4.2 |
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
Three of the four steps that do exist check the wrong thing: `mail`, `docs`
and `graph` look for a binary on PATH (`onboarding::on_path`) and never for the `[[mcp]]`
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

**OpenClaw** is closest to what this doc first proposed and F1 overruled
(§7) — **a config section being present turns it on** ("each channel starts automatically when its config section
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

- **Goose** toggles built-in and external *extensions* from `goose
  configure`, a desktop page or `config.yaml` (`enabled: true`), and per
  session with `--with-builtin`; some built-ins are on by default. It is the
  plugin-shaped answer — an extension is a tool bundle — and adds nothing the
  registry here needs beyond what Claude Code's manifest already shows.
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
   is already `deny_unknown_fields`; any new table keeps that — **except
   `[features]`**, where an unknown key warns by name and is listed by `mecha
   features` rather than failing startup, because one `config.toml` is read by
   several builds at once and a key cannot turn on a feature a binary does
   not know (§9 step 1).
3. **Two sources of truth for one switch** — Open WebUI's env *and* database,
   LibreChat's YAML *and* roles. The enabling fact lives in one place.
4. **Many parallel switches for one feature** — Hermes has per-platform lists,
   a global disable, a flag, a slash command and per-tool toggles.
5. **Lots of it on by default** — OpenClaw's plugins-on-by-default cost is
   #75279. In mecha every bool in §5's `[features]` table ships `false`.
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

A new module, `feature.rs`, holds a closed enum `Feature` — **one enum for
features and their parts**, each variant answering `id()` (the wire and CLI
name), `label()`, `part_of()` and `requires()`, and later `recommend()`
(§6). A part (`ocr`, `library`, `publishing`) is a variant pointing *up* at
its parent with `part_of`, not a list hanging off the parent, so every row
in `mecha features --json` — feature or part — has one shape and one id.
Two things follow from choosing this, and are fixed here rather than
discovered later (found on review of #427):

- **Part ids are on the wire** in `--json` and `/api/features`, and are
  never renamed, like feature ids.
- **The recorded feature set (§5.1) holds top-level features only** — the
  `[features]` bools. A part has no switch of its own, so recording it would
  put a derived fact on an append-only store whose loader is all-or-nothing:
  a build that added a part would collapse every older reader's set to
  `None`. Parts are recomputable from the recorded config; the switches are
  what the owner chose.

and one function that is the only place "is it on?" is answered:

```rust
pub fn state(facts: &feature::Facts, f: Feature) -> State
```

**No `Config` parameter.** `feature::Facts` — its own type, not
`onboarding::Facts`, which the CLI fills field by field for `setup` — is
built by `Facts::read(home, &global)` and carries the global configuration
itself, so a caller holding a project-layered `Config` has nowhere to hand
it. As built in step 0 (#428). It matters for the `[[mcp]]`, `[[search]]`,
`[tools]` and `default_provider` a project layer keeps; `[features]` itself
is stripped from project layers, so for the bools a layered value would
agree anyway. `setup` builds both `Facts` until step 4 has it iterate the
registry.

`State` keeps apart the things a bool would merge:

| State | Meaning | Web | Routes | CLI | Tools |
|---|---|---|---|---|---|
| `Off` | not configured (or declined) | hidden | 404 `feature_off` | one sentence + the setup command | not registered |
| `Blocked(Feature)` | configured, but something it needs is off — the **first** unmet need in `needs()` order (the parent, then `requires`) | hidden, and Settings says what it waits on | 404 `feature_off`, naming that dependency | names it; `mecha features enable` names every unmet switch in its chained command (`dictate` is off because `web` is: `mecha features enable web`) | not registered |
| `Unready(reason)` | enabled, but config or disk says it cannot work yet — settings missing or refused, no account authorised | **shown**, with a banner | 503 with the reason | the reason | whatever registration's own rule builds — nothing from an absent `[image]`; a mail server with no account still connects and says so per call |
| `Down(reason)` | configured, and a probe found it not answering — **`mecha features --probe` only**, and so not a variant `state(facts, f)` returns: like *pending restart* (§4.2), it is the probe's annotation over an `On` row, added with the probe itself | — (never produced: the web reads `On`, and the handler's own error is what the owner sees) | — (a route cannot probe per request) | the reason | registered |
| `On` | enabled and usable as far as config and disk can say | shown | normal | normal | registered |
| `Unknown(reason)` | could not be read | **shown**, with a banner | normal | warns | registered |

Three rules carry the design:

- **Tools follow configuration, never liveness.** `Down` still registers
  its tools. (`Unready` is a configuration fact, so it follows the same rule
  from the other side: registration builds what the settings allow, and an
  absent settings table allows nothing — the existing rule that "a tool that
  always errors is worse than no tool at all".) Two states rather than one,
  because the first is readable from config and the second needs a probe,
  and a reader that must never probe (§4.3) has to be able to tell them
  apart (found on review of #427). The tool list is the front of the cached prefix and is built
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
   it reads only config and the disk. **`mecha features enable|disable
   <id>`** writes the bool into the global `[features]` table (`codex
   features enable`); an enable whose dependency is off is refused with the
   chained command (`mecha features enable web incognito`), as Claude Code
   refuses a disable that would break another plugin. A **part** id (`ocr`,
   `dictate`, `library`) has no bool, so `enable ocr` is refused by name and
   points at the parent's switch and the setting that turns the part on
   (`[documents] ocr = true`).
2. **`mecha setup`** iterates the registry instead of the hand-written
   `integration_steps`. Every optional feature becomes a declinable step, and
   **`mecha setup <feature>`** runs just that one (Hermes and OpenClaw both
   ended up here). A step for a feature whose dependency is off says so and
   offers the dependency first, as Claude Code does when a disable would
   break another plugin.
3. **`GET /api/features`** returns the same rows. `Nav.svelte`'s `enabled`
   column becomes that answer rather than a literal, and `Off` and `Blocked`
   entries are **removed**, not greyed out. `Unready` entries stay
   **clickable**, marked with a badge, and the view carries the banner with
   the fix: the existing greyed-out style (`.disabled`, with its "coming in a
   later phase" tooltip) is unclickable, so reusing it would put the fix out
   of reach — hiding by styling what §4.1 says is never hidden (found on
   review of #427, pass 10). That style is retired. Home cards, the voice-call button and `Dictate` take
   the same test. A direct link to a hidden view (`#library`) lands on Home
   with a one-line notice rather than an empty page. **Settings → Features**
   lists everything, including `Off`, with the command that turns each one on —
   hidden from navigation, never undiscoverable.
4. **Route guards.** Each API route group declares its owner, and one axum
   layer answers for all of them, so the five ways an off feature fails today
   (§1.2) become one. Side-effecting reads (`/api/frontdoor` creating its
   store) stop happening for a feature that is off. **The layer sits inside
   `owner_guard`.** Serve's middleware chain applies the last `.layer` as the
   outermost, so a feature layer appended to it would answer before
   authentication, and F4's body — the feature and the command that turns it
   on — would give an unauthenticated probe an inventory of the install. The
   existing `*_sit_behind_the_owner_guard` tests pin that a probe without the
   header "learns nothing, not even that these routes exist"; step 2 extends
   them with an off feature's route, asserting the probe gets the guard's 403
   and never `feature_off`.
5. **CLI guards.** A verb that belongs to a feature calls
   `feature::require(&facts, Feature::Image)?` first — with `facts` built
   from `Config::load_global()` and the home, **never** from the layered
   `Config` the verb itself runs with, which is exactly what §4.1's signature
   refuses to accept (found on review of #427, pass 9) — and every verb says the
   same thing the same way: *"image generation is not enabled — `mecha
   features enable image`"*.
6. **Tool registration** in `setup::prepare_tools` asks the registry rather
   than repeating `if let Some(image) = cfg.image`. The existing gates
   (`cfg.search`, `cfg.image`, `cfg.documents`, `vision_enabled`) already key
   on configuration, not liveness — read in the step-0 audit, not in §1; the change is that they share one predicate with
   the other five readers, so they cannot drift apart. **The switch comes
   from the global config, the settings from the layered one** (found on
   review of #427, pass 11). `prepare_tools` runs on the session's layered
   `Config`, and a project may legitimately declare a `[[search]]` backend or
   an `[[mcp]]` server for its own sessions — so registration asks the
   registry only *whether the feature is on* (the global `[features]` bool,
   which no project layer can set), then builds from the layered settings as
   it does today. A project's own `[[search]]` backend therefore works in its
   sessions when `search = true`; a project-declared `mecha-mail` entry under
   `mail = false` is **not connected** — the bool gates the known servers
   whichever layer declared them — and an unknown server a project declares is
   not a registry feature at all and connects as it does today.

**When a switch takes effect.** A bool written by `mecha features enable`
is read at different times by different readers, and the design says which,
because "the config says yes and the running thing does not know it" is the
one state this design otherwise refuses to leave unsaid (found on review of
#427, pass 8):

- **`/api/features` re-reads the global file on every request.** It is config
  plus disk — cheap, no socket — so the nav, Home and Settings follow a flip
  on the next page load for everything whose gate is a page or a route.
- **Tools and `[[mcp]]` connections follow the next session**, never a live
  flip: the tool list is the front of the cached prefix. So is `mecha serve`
  itself for its routes' state, which it loads once. `mecha features enable
  graph` says *"enabled — its tools arrive in the next session; restart
  `mecha serve` for the web"*, and `/api/features` compares the file with
  what the serving process loaded and marks a difference **pending restart**.
  That is an annotation on the row (`"pending": true`), not a seventh
  `State`: `state(facts, f)` answers what the file says, which is all `Facts`
  can know, and only `serve` knows what it loaded, so the comparison is
  `serve`'s to make. The same holds for `mecha slack
  connect` and the units that load once (ruminate, front door).

**The upgrade is announced.** An install that predates `[features]` has
every bool absent and so every feature `Off`, and step 1 also moves tool
registration to the registry — so without a word, one `cargo install` would
stop mail, docs and the graph connecting and have `mecha serve` refuse to
start. F6 has `mecha setup` offer the bools; nothing would tell anyone to run
it. So, in step 1, the `tool_availability_notices` shape Hermes uses:

- every start prints one line per feature whose **bool is absent but which
  would otherwise be usable** — *"`mail`: configured but not enabled —
  `mecha features enable mail` (or `mecha setup`)"* — on stderr, like the
  routed-outbox-name warning that fires on every start. "Would otherwise be
  usable" is the full `state` with every switch **absent from** `[features]`
  treated as on — and it iterates only features that *have* a switch
  (`part_of().is_none()`), since a part has no bool to announce and `enable`
  refuses a part id by name. The substitution covers every absent switch,
  not just the one being announced, because on this install the dependencies
  are absent too: substituting `incognito` alone would leave `web` off and
  short-circuit it to `Blocked`, so the line would never print. An explicit
  `false` is an answer, not an unanswered question, and is never substituted:
  an owner who wrote `web = false` is not told to enable `incognito` on every
  start (found on review of #435, passes 3 and 4) — **not** a
  settings table being present: four features' evidence is not a table at
  all — `slack` a token store, `personas` a non-empty store, `voice` an
  installed unit file (installed, not running: a socket-activated unit is
  idle until asked, and §4.3 reads unit files, never sockets), `frontdoor` a
  binary on PATH — and a table-keyed notice could not fire for any of them.
  The notice and F6's detector are one function. **Two obligations on step 1
  before it can key on this** (found on review of #435): `personas` and
  `voice` are unconditional `On` in step 0 ("no switch yet"), so step 1 must
  give them the evidence named here first, or both lines print on every
  light install forever; and because it is `state`, not `own_state`, a
  feature whose dependency is off (`incognito` without `web`) is announced
  with the dependency first — *"`incognito`: needs `web` — `mecha features
  enable web incognito`"* — never offered alone into a `Blocked` it cannot
  leave;
- `mecha features` shows that pair as its own row (settings present, switch
  absent), not a bare `off`, as it does an off front door with requests
  waiting;
- `mecha serve` refusing for `web` tells *"this install predates the
  switch — `mecha features enable web`"* apart from *"the web app is turned
  off"*: the first is a one-command fix, the second a choice.

`mecha doctor` stays what it is — no network, no model, the stores' distress.
A feature that is `Down` is `mecha features --probe`'s to report, not the
doctor's.

### 4.3 Probing without waking anything

Several servers here are **started by being asked**: OCR and embeddings are
socket-activated, the router loads any model a request names unless
`autoload=false` is passed, and ComfyUI loads weights on first use. A probe
that wakes the thing it probes turns `/api/features` into a way to fill
memory from a page load. So:

- `/api/features` reads **configuration and the disk only** — config tables,
  binaries on PATH, credential stores, installed unit files. It never opens a
  socket, so a server that is down reads `On` there and the web app learns of
  it from the failing request, not from a banner — the price of never waking
  anything from a page load (found on review of #427, pass 8). A later
  `?probe=1` could use exactly the load-free probes named below and nothing
  else; it is not in this design. Its states are `Off`, `Blocked`, `Unready`, `On` and `Unknown` —
  every state but `Down`, which only a probe can produce.
- `mecha features --probe` and `mecha setup` may probe, using only calls that
  load nothing: `served_props` / `GET /models` against the router, the
  systemd unit state for a socket-activated server, ComfyUI's
  `/system_stats` (to be confirmed load-free on this install before step 6
  relies on it — "loads weights on first use" is why ComfyUI is on this
  list). A server that runs on demand reports **"on demand"**, which
  is a state, not a failure.
- Every probe row says **where** it ran (#80206).

---

## 5. The catalogue

**The switch is a bool in one `[features]` table** (F1, ruled 2026-09-30).
The owner's reason: *"users need to know what features are available"* — a
table that turns a feature on by existing is invisible until you already
know its name, and a list of toggles is its own documentation.

```toml
# ~/.mecha/config.toml — written in full by `mecha config init`, every
# feature listed, every one off; `mecha features enable <id>` flips one.
[features]
web = false        # the web app (`mecha serve`)
slack = false      # Slack remote control
mail = false       # mail and calendar
docs = false       # Google Docs, Sheets and Slides
graph = false      # the knowledge graph and the task board
search = false     # web search and open
documents = false  # PDF extraction (OCR and layout are [documents] settings)
image = false      # image generation and the character library
personas = false   # characters you write and talk to
voice = false      # dictation and voice calls
incognito = false  # a web chat that leaves no trace
frontdoor = false  # inbound requests, publishing, polls
messages = false   # messages between sessions
```

Three rules keep one switch from becoming two:

- **The bool is the only switch; a settings table is only settings.** An
  `[image]` table with `image = false` is off, its settings kept for later.
  `image = true` with no `[image]` table is **`Unready`** — enabled, not yet
  usable — shown with the fix, never hidden, because the owner said yes to
  it. Presence of a table means nothing on its own, so there is nothing for
  the bool to disagree with.
- **`[features]` is global-file only.** `merge_file` strips it from project
  layers like `[web]` and `[image]`, so a cloned repository can never turn a
  feature on — and that holds for the four `[[mcp]]` server rows too, which
  presence could not guarantee: `merge_file` deliberately keeps a project's
  servers and `ConfigLayer::apply` replaces the list wholesale, so under
  presence a project file would have switched the owner's Mail and Graph tabs.
  A project's servers still give sessions in that directory tools — where the
  feature they belong to is on, or where they belong to none — and never
  change what the install says is on (§4.2 item 6). (The registry also reads the server
  rows' settings from the global layer's `[[mcp]]` only — enforced from step
  0 by `Facts::read` carrying the global configuration, so `state` has no
  `Config` parameter a layered value could be handed to.)
- **A key that is absent is off**, and a feature shipped after your table
  was written is simply absent — so `mecha features` lists it, and `mecha
  setup` offers it as `Missing`, never as declined. (Hermes had to add
  `known_builtin_toolsets` to tell "declined" from "never offered"; a decline
  here is already per id in `setup-declined.json`.)

Parts have no bool of their own: `tasks` and `library` ride their parent,
and `ocr`, `layout`, `dictate`, `calls` and `cloning` are switched by their
parent's settings, where they already live.

A `(parent)` in the Requires column is `part_of`, not a `requires` edge:
`needs()` is the parent, then `requires`, so writing the parent into
`requires()` as well would add a duplicate edge.

| id | Feature | Settings it needs (Unready without them) | Requires | Hidden when off |
|---|---|---|---|---|
| `web` | The web app (`mecha serve`) | `[web] owner_login` (serve refuses without it) | — | everything web; `mecha serve` refuses with the fix |
| `slack` | Slack remote control | tokens in `~/.mecha/slack` (`mecha slack auth`) | — | `mecha slack …` verbs except `auth` |
| `mail` | Mail and calendar | a global `[[mcp]]` entry running `mecha-mail`, and an authorised account | — | Mail tab, Home mail card, Outbox event editor, `mecha mail` |
| `docs` | Google Docs, Sheets, Slides | a global `[[mcp]]` entry running `mecha-docs`, and an account | — | its tools |
| `graph` | Knowledge graph | a global `[[mcp]]` entry running `mecha-graph-mcp` | — | Graph tab, Review → graph queue, Proposals → entities, `kg`, `gossip`, `corroborate`, `vet`, `distill` |
| ↳ `tasks` | The task board | — | `graph` (parent) | Tasks tab, Home tasks card, `mecha tasks` |
| `search` | Web search and open | a `[[search]]` backend not disabled | — | `web_search`, `web_open` |
| `documents` | PDF extraction | a `[documents]` table | — | `document_read`, `mecha document` |
| ↳ `ocr` | OCR pages | `[documents] ocr` | `documents` (parent) | its row in `features` |
| ↳ `layout` | Region-by-region layout | `[documents] layout` | `ocr` (parent) | its row in `features` |
| `image` | Image generation | an `[image]` table | — | `image_generate` |
| ↳ `library` | Characters and styles | — | `image` (parent) | Library tab and Home card, `image_library*` (registered only inside `[image]`), `mecha imagelib` writes. Its reads — `mecha imagelib list`/`show` — are store reads and stay ungated |
| `personas` | Characters the owner talks to | — (a **new** `[personas]` table later holds its safety settings) | — (web for the tab) | Personas tab, `/api/personas*`, `mecha persona` |
| `voice` | Talking to mecha | a **new** `[voice]` table (below) | — (`mecha voice-serve` is its own loopback surface) | `mecha voice-serve` |
| ↳ `dictate` | Speech to text in the browser | `[voice] stt_url` | `voice` (parent), `web` | Dictate |
| ↳ `calls` | Spoken conversation in the browser | `[voice] offer_target` | `voice` (parent), `web` | voice-call button |
| ↳ `cloning` | New voices | `[voice] voices_dir` | `voice` (parent), `web` | Settings → Voice → clone |
| `incognito` | A chat that leaves no trace | a local provider without fallbacks (`provider_is_local`) | `web` | Chat's incognito toggle |
| `frontdoor` | Inbound requests and polls | `factory-publish` on PATH — its drain fills `~/.mecha/requests` | — | Review → Front door, Home card, `frontdoor`, `polls` |
| ↳ `publishing` | The model's publishing tools | a global `[[mcp]]` entry running `factory-publish` | `frontdoor` (parent) | the `factory__*` tools |
| `messages` | Messages between sessions | — (`[messages]` keeps its tunables) | — | `message_send`, `mecha msg` |

**Not every surface belongs to a feature.** A `requires` edge means
*cannot work without*, never *shares a subsystem*: `questions`, `workflow`
and `/api/today` read local stores (`~/.mecha/questions`, the outbox, the
workflows) and have no graph in them, so they are core, and a graph that is
off must not hide a delegated run's blocking question or a staged draft. The
same holds for every **cross-feature reader** — `/api/today`,
`Backlog::read`, `mecha doctor`, `/api/summary`: they are core, ungated by
construction, and degrade **per section** (a Today card for mail simply has
no rows when mail is off), which is the decline rule — a feature being off
never hides a failure somewhere else — applied to aggregates (found on review
of #427, pass 6). **Counts of what is waiting never degrade.** The front
door's queue is filled by an external drain the bool does not stop, so
`frontdoor = false` over a non-empty `~/.mecha/requests` would hide strangers'
requests the drain keeps delivering — the failure the two-row split below
exists to prevent, reintroduced by the switch. So `Backlog`'s sections
(`frontdoor`, `requests_on_owner`) count whatever is on disk whatever the
bool says, and a feature that is off with work waiting in its store is a
condition the owner is **shown** — a banner on Home and a row in `mecha
features` ("off, 3 requests waiting — `mecha features enable frontdoor`") —
never hidden. `doctor` does not cover it: a queue nobody is looking at is not
distress, and `backlog` is the reader that counts (found on review of #427,
pass 7).

Notes on the rows that change:

- **`voice` gets a settings table** because today it has none: its switches
  are serve flags (`--voice-port`, `--offer-target`), a literal in
  `serve::dictate`, and `[web] voices_dir`. `[voice]` holds all of them, so
  the Parakeet and worker URLs become config for the first time — which a
  new user needs anyway, since theirs will not be this machine's.
  `voices_dir` moves from `[web]` with a one-release alias.
- **The four server rows (`mail`, `docs`, `graph`, `publishing`) read the
  entry, not the tools.** Which tools a server exposes is known only after
  `connect` spawns it and `tools/list` answers, and the registered names
  then depend on `prefix_tools` — so "exposes `mail_*`" is not a fact
  configuration holds, and learning it per page load would start four
  third-party processes, the failure §4.3 exists to prevent. The setting is
  an enabled `[[mcp]]` entry whose `command` runs the known program; whether
  the tools answered is `--probe`'s question. With `mail = false`, that
  entry is not connected at all — the bool gates the server, not just the
  tab. **The entries are read from the global layer, and the type says so**:
  `state` takes them from `Facts::mcp`, which `Facts::read` fills from the
  global config, never from the `Config` a caller holds — every reader but
  `mecha features` holds a project-layered one, and a doc comment asking
  each to comply would be kept by some (found on review of #427).
- **The front door is two rows.** Its queue is filled by `factory-publish
  drain`, a binary on PATH, and read by the Review page and `mecha frontdoor`
  with no MCP entry at all; the entry exists only to give the model
  publishing tools. Keyed on the entry alone, an install with a running
  drain and no entry would hide a live queue of strangers' requests. Neither
  half needs mail — only booking settlement reads the mail ledger, and that
  fails closed on its own (found on review of #427 and #428).
- **`[messages] enabled` becomes an alias** for `[features] messages`, read
  for one release and **or**-ed in: both default to `false`, so the alias
  can only keep on what was already on — it cannot switch messaging on for a
  table present only to raise `pending_cap`, which is the fail-open case a
  presence rule would have had. **`Lever::Messages` follows it.** The lever
  is defined as `--no-messages` or `[messages] enabled = false`
  (`harness.rs`), and is recorded in every `RunStats` row and eval
  scorecard; after the move it reads `--no-messages` or the **switch** —
  the `[features] messages` bool, or-ed with the alias — not the six-state
  readout. The lever records whether a mailbox was *asked for*; a messages
  feature has no settings, so its readout is only ever `On` or `Off` anyway,
  and a lever that folded `Unknown` or `Down` one way or the other would
  claim either that messaging worked or that the owner had it off, both
  false. A test asserts the lever and the switch never disagree — otherwise a run with no mailbox records messaging as on, the
  wrong condition `levers_off` exists to prevent. The same test covers
  `Lever::Mcp` beside the four server bools: the lever forces every server
  off, a bool gates one, and the recorded feature set says which bool was on
  whatever the lever did (found on review of #427).

`[features]` and every new settings table are **four places** each, not two:
`Config`, `ConfigLayer`, `ConfigLayer::apply`, and the project-layer strip in
`merge_file`. `every_field_of_config_is_reachable_from_a_file` and
`every_field_a_layer_can_read_is_a_field_a_layer_applies` catch a missed one
at the top level only, so the build step adds a nested-layer test alongside.

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
set of `[features]` bools that are `true`. Compile-time cargo features would buy little today
and are out of scope (§8) until a heavy Rust dependency lands.

Three things make it deliberate rather than accidental:

- **A fresh `[features]` table is the light install**: every bool `false`.
  `mecha setup --minimal` (Hermes's *Blank Slate*) then declines every
  optional step in one pass, so setup stops offering them; it writes
  declines, never config, and `mecha features enable <id>` still turns any
  one on later.
- **A trial's features are a condition of the trial.** A trial home's config
  already comes from its environment (`trial_env`), so its features follow
  that environment's `[features]` table with no second switch. **An
  environment may set `[features]`**, though a project layer may not: it is
  stripped at `LayerTrust::Project` and deliberately **not** added to
  `trial_env::OPERATOR_ONLY_TABLES`. That is safe for turning features
  *off*, which is what a light trial needs, and for the few it may turn on,
  named below — a trial home never inherits the operator's servers except
  through the manifest's `live_servers` (the 2026-09-23 rule `trial_env`
  exists for). **But an
  environment may switch a feature *on* only if it could configure it**,
  judged by the trust of the tables the feature's switch and settings live
  in, not by who happens to supply them: a feature whose table is in
  `trial_env::OPERATOR_ONLY_TABLES` (`web`, `slack`, `image`, `messages` —
  whose switch is the alias of `[messages] enabled` — and, once step 1 and
  step 5 add them, `documents`, `voice` and `personas`) or in
  `MACHINE_TABLES`
  (`search`, whose backends and keys `config_at` copies from the operator —
  `Egress::Chosen` at deep search, with nothing to degrade to `Unready`)
  cannot be turned on from an environment. **Nor can `mail` or `docs`**,
  though their `[[mcp]]` entry is environment-declarable: their account
  stores are found by the mail crate's rule — `~/.mecha/{mail,docs}` under the
  real home, never `$MECHA_HOME` (`onboarding::mail_store_dir`, #428) — so an
  environment that declared a `mecha-mail` entry and set `mail = true` would
  read `On` against the owner's live mailbox. The test is therefore not only
  the tables' trust but **where the feature's credentials live**: a feature
  whose credentials are the operator's, wherever its entry is declared, is
  operator-only to switch on (found on review of #427, pass 13). **The list
  is a function, not a fourth hand-kept list**: `config_at` refuses by table
  name and cannot see a `[features]` key, so step 1 adds
  `Feature::switchable_from_environment(self) -> bool` as an exhaustive
  `match` — a new variant does not compile until it decides — and
  `config_at` refuses any `[features]` key set `true` for which it is
  `false`. The permitted set, said out loud, is **`frontdoor`** alone — a
  binary on PATH and the trial's own `requests` store; every other
  top-level feature is `false` (found on review of #427, pass 14). Not
  `incognito`: its evidence is `provider_is_local` over `default_provider`
  and `providers`, both `MACHINE_TABLES` copied from the operator, so it
  fails the test on its own terms, not only by needing `web` (#435). **Not `graph`**, though an
  environment may declare its own graph server: a manifest's `live_servers =
  ["graph"]` carries the operator's live server into the trial's `[[mcp]]`,
  and the graph row finds a server by its command, so it cannot tell a
  carried-in entry from a declared one — an environment's `graph = true`
  beside that manifest would read `On` against the owner's `graph.db`, the
  2026-09-23 incident this section exists for. So a trial's `graph` bool is
  **defaulted by `trial_env::config_at`**, and the environment file may only
  narrow it: `graph = true` there is refused at load like any key the
  predicate forbids; `graph = false` is honoured, so a light arm stays light
  even beside a manifest that carries a server; and with the key absent,
  `config_at` sets it on exactly when the environment declares its own graph
  server or the manifest's `live_servers` carries one in — the operator's
  existing, explicit opt-in — so experiments that use `live_servers` keep
  working once step 1 gates the server on the bool (found on review of #435,
  passes 1 and 3). A new settings table gets the
  same test the day it is added (§9 step 8). An environment that sets such a
  key to `true` is **refused at load**, with `config_at`'s `ensure!` and its
  reason. **`[documents]` is not on `OPERATOR_ONLY_TABLES` today**, though
  `merge_file` strips it from project layers for `[image]`'s reason — its
  `ocr_url` is where the owner's documents go and `confine` is the PDF
  parser's sandbox — and the constant's own comment still counts "the five a
  project layer is stripped of". An environment can set both today, with or
  without this design; step 1 adds `documents` to the list and corrects the
  comment, since this rule is only as good as that list (found on review of
  #427, pass 6). The refusal is
  reason, never ignored with a warning: a warning fails open in exactly the
  way the `requires` bullet below exists to close — the arm would run without
  the feature it asked for and be scored anyway. An environment may set any
  key to `false`. Three tests pin it: `graph = true`, `search = true` or
  `messages = true` in an environment file refuses the trial; `graph` absent
  with a manifest carrying no graph server reads off; and `graph = false`
  beside `live_servers = ["graph"]` reads off (found on review of #427,
  passes 4 and 5, and #435). Turning features off is how a trial is made light; this is the switch
  it uses. But which features were on
  is as much a condition as which levers were off, so the experiment manifest
  and the session record carry the feature set beside `levers_off`, from the
  same registry. Two arms that differ only in whether `[[search]]` was
  present differ today only in effects — `RunConfig::tools`' tool names,
  `condition_hash` — and `harness.rs`'s rule is to record *the switch, not
  the effect*: `web`, `personas` and `voice` change no tool names at all. **Recorded with `levers_off`'s wire rule,
  not just beside it.** A feature set is a closed enum on an append-only
  store, so its loader is all-or-nothing like `session::lenient_levers`: one
  name a later build does not know collapses the whole set to `None`, because
  an entry dropped on its own would read as *off* — the mirror of a dropped
  lever reading as on — and bring back exactly the identical-arms case. A
  reader that partitions trials by feature set puts `None` with the sessions
  recorded before the field existed, never with "nothing on".
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
    pub tier_gb: u32,             // 16, 32, 64, 128 — hardware.md's tiers (F5)
    pub memory: Memory,           // the column and its cost, as one value (F5)
    pub model: &'static str,      // "PaddleOCR-VL 1.6 (GGUF + mmproj)"
    pub fetch: &'static str,      // "hf download PaddlePaddle/PaddleOCR-VL-1.6-GGUF"
    pub residency: Residency,     // Resident | OnDemand | PerRequest
}

/// F5's second column. `Unified`: one pool holds everything (a GB10, a Mac),
/// and `tier_gb` is that pool. `Discrete`: `tier_gb` is the GPU's own memory,
/// the chat model is sized to it, and the row may put OCR, embeddings and
/// speech to text in system RAM or on the CPU instead — so a discrete row's
/// cost is **two** numbers, which `Peak` alone cannot carry. `mecha setup`
/// reads which shape the machine is before choosing a row. A card between
/// tiers (24 GB, 48 GB) takes the row at or below it for the model family;
/// whether it fits is `--probe`'s ratio against the card's actual memory,
/// never the row's label (found on review of #435).
pub enum Memory {
    Unified { peak: Peak },
    Discrete { gpu: Peak, host: Peak },
}

pub enum Peak {
    Measured { mb: u32, machine: &'static str, date: &'static str },
    Arithmetic { mb: u32 },       // hardware.md's formula
    Unmeasured,                   // no number at all — reported as null, never 0
}
```

Rules:

- **Printed, never written.** `mecha setup <feature>` prints the row for this
  machine's tier and the download command. It does not write a model name
  into config: the config value is read back from the running server
  (`mecha setup --write`), per `onboarding.rs`'s rule that setup never writes
  down a number the user merely believes.
- **Evidence on every row.** Only the GB10 has measurements. Rows for other
  tiers are `Peak::Arithmetic` (from `hardware.md`'s formula) or
  `Peak::Unmeasured` — one enum, so a number without a source or a source
  without a number cannot be written (found on review of #427, pass 14),
  and the output says which. llmfit's `calibrated` / `estimated` split is the
  model; its `null`-not-zero rule is ours already.
- **Rows that don't fit stay visible, with the reason** — "needs ~15 GB peak;
  this machine has 9 GB available" — rather than disappearing (Hermes, Jan).
- **The budget is the sum, not the row.** The real question on a 32 GB
  machine is not "does image generation fit" but "does it fit *beside* the
  chat model". `mecha features --probe` adds up the resident and peak memory
  of everything enabled and reports llmfit's ratio band against the machine's
  memory, with `null` for any feature whose peak is unmeasured. On a
  `Discrete` machine that is **two sums against two totals** — the GPU's
  memory and the host's — never one: a single sum would pass a chat model
  that does not fit the card and fail an OCR server that fits host RAM with
  room to spare (found on review of #435). This is why
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

## 7. Rulings

| # | Decision | Ruling (2026-09-30) |
|---|---|---|
| **F1** | What turns a feature on | **A `[features]` table of bools** in the global config, every feature listed. The owner, overruling the doc's recommendation of table presence: *"The problem with table existing is that users need to know what features are available. I feel like a registry or having to toggle bools is a better design."* §5's three rules are what keep the bool from being a second source of truth |
| **F2** | Off in the web app | **Removed from navigation**, as the owner asked in the opening message; Settings → Features lists everything. `Unready` and `Unknown` are shown with a banner, never removed (§4.1) |
| **F3** | Web as a feature | **Optional like the rest** — the owner: *"Web should also be optional feature."* CLI, TUI and Slack are complete without it; incognito, and voice's browser parts (`dictate`, `calls`, `cloning`), report `Blocked(web)`. `voice` itself does not: `mecha voice-serve` is its own loopback surface, and `Blocked` would refuse it (found on review of #427, pass 7). A tab's visibility is not a `requires` relation: with `web` off there is no navigation at all, so the Personas and Library tabs need no dependency on it — and giving `personas` one would make `Blocked` refuse `mecha persona` from the CLI, which works without the web (found on review of #427) |
| **F4** | What an off route returns | **404** with `{"error":"feature_off","feature":"image","fix":"mecha features enable image"}`, only behind `owner_guard` (§4.2 item 4). 503 stays for `Unready` |
| **F5** | Recommendation tiers | **`hardware.md`'s four — 16, 32, 64 and 128 GB — in two columns**: unified memory, and a separate GPU beside system RAM, where the tier is the GPU's memory and the auxiliary models (OCR, embeddings, speech to text) may run from system RAM or the CPU. One page and one table agree on the tiers; the column is what keeps a 24 GB GPU with 64 GB of RAM from being steered as a 24 GB machine. Only the 128 GB unified row is measured (this GB10); every other cell says `Arithmetic` or `Unmeasured`. Ruled by the owner 2026-09-30 |
| **F6** | Existing installs, when `[features]` arrives | **`mecha setup` offers.** It detects a feature in use (an `[image]` table, a mail `[[mcp]]` entry, a non-empty persona store, an installed voice unit file) and offers to write its bool — the same predicate as the upgrade notice (§4.2), so the two cannot disagree. Never grandfathered as on: that is a second source of truth. On this machine the deploy that ships the table writes it by hand, in the same change, so nothing disappears |

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
1. **The `[features]` table.** `Config`, `ConfigLayer`, `apply`, the
   project-layer strip and the nested-layer test; `state` reads the bool
   first and the settings second; `mecha features enable|disable <id>`
   writes it, refusing an enable whose dependency is off with the chained
   command; `mecha config init` writes the table in full; `[messages]
   enabled` read as an alias, and `Lever::Messages` reading the registry
   (§5). **`mecha features enable|disable` needs its own writer**: the only
   config writer today, `setup::apply`, bails when the table header is
   absent — the state every upgrading install is in — and a writer that
   rewrote the file would drop a newer build's unknown key, the hazard this
   step accepts warn-and-ignore to avoid. So it edits in place, creating
   `[features]` when missing and touching only the one line. The upgrade
   notices (§4.2) land here too. F6's offer lands in `mecha setup` here, and
   this machine's table is written in the same deploy. **An unknown key in
   `[features]` warns and is ignored, unlike every other config table**:
   `deny_unknown_fields` everywhere else makes a typo a startup failure, but
   here one `config.toml` is read by the installed release, the long-running
   units and several worktree builds at once, and a key a worktree build
   adds would take the older binaries down at their next restart with a bare
   serde error. An unknown key cannot turn anything on in a binary that does
   not know the feature, so ignoring it fails closed; the warning names the
   key and says the binary predates it or it is misspelled, and `mecha
   features` lists it — so a typo still shows as the feature it meant being
   off (found on review of #427, pass 6). **Tool registration
   switches to the registry in this step** — a server whose feature is off is
   not connected — since that is the step that could otherwise make a tool
   disappear unannounced.
2. **`/api/features` and the web app.** Nav, Home cards, the voice button and
   Dictate take their answer from it; Settings → Features. The endpoint
   re-reads the global file per request and marks *pending restart* (§4.2).
   No route changes yet, so a stale page still works.
3. **One guard for routes and CLI verbs.** Every route group and verb names
   its owner; the five failure shapes become `feature_off` / 503. The
   side-effecting reads stop. The layer goes inside `owner_guard`, and the
   owner-guard tests gain an off feature's route (§4.2 item 4).
4. **Setup iterates the registry.** A step per feature, `mecha setup
   <feature>`, `mecha setup --minimal`, dependencies offered first, and the
   `[[mcp]]` checks that `mail`, `docs` and `graph` are missing today.
   **Experiments** record the feature set beside `levers_off`, with a
   `lenient_features` loader of `lenient_levers`' all-or-nothing shape, and
   an environment's `requires` refuses a trial that lacks one (§5.1).
5. **The new settings tables**: `[voice]` (with the URLs out of the code)
   and `[personas]`. Four places each, and a place on `trial_env`'s lists:
   `[voice]` goes on `OPERATOR_ONLY_TABLES` for `[image]`'s reason —
   `stt_url` is a destination the owner's audio goes to. `[personas]` goes
   there too, for a different reason: it will hold the crisis-pause cooldown
   (today `persona::safety::CRISIS_COOLDOWN`, a constant), and configuration
   supplied with a checkout may only narrow, never loosen — an environment
   file that could shorten a safety pause is the wrong direction whatever the
   field carries. "No destination or credential" was the wrong test (found on
   review of #427, pass 11). A study that needs a different cooldown asks the
   operator.
6. **Recommendations**: the rows, the probe that sums memory, and a test that
   `hardware.md` matches them. `hardware.md` changes first: its tier
   sections (`### 16 GB` … `### 128 GB and up`) describe one pool, though
   the page's own description already says "unified-memory or VRAM tier". It
   gains F5's discrete column, with every discrete cell marked `Arithmetic`
   or `Unmeasured` until someone measures one. The Mac note that "unified
   memory has no separate GPU pool" stays: it is scoped to Macs and still
   true (#435). Fix the embeddings page.
7. **Installers**: one `scripts/<feature>/install.sh` per feature that needs a
   service, each with `--remove`, copying rather than symlinking (the
   `scripts/llama/install.sh` pattern), with no `/home/<user>` or checkout
   path in any unit. The router, ComfyUI and Chatterbox get units in the repo
   for the first time.
8. **How to add a feature, written down** — the owner, 2026-09-30: *"we
   should make sure we document design pattern for adding new features in
   docs and Claude.md."* A new `docs/ARCHITECTURE.md` §Features holds the
   checklist and the incident behind each line; it is **opened in step 1 and
   extended by every step after**, so each line is written by the step that
   learned it rather than reconstructed at the end. The checklist, as far as
   this design knows it:
   - a `Feature` variant, with its `id` (a wire name: never renamed), label,
     `part_of` and `requires`, listed after everything it needs;
   - a key in `[features]`, and `mecha config init` writing it;
   - its settings table, if any: `Config`, `ConfigLayer`, `apply`, and the
     `merge_file` project strip — four places, and the nested-layer test —
     **and a decision on `trial_env`'s two lists**: `OPERATOR_ONLY_TABLES`
     (a checkout may not name it: a destination, a credential, a mailbox),
     `MACHINE_TABLES` (the operator's copy is used), or neither (an
     environment may declare it). That decision is also what says whether an
     environment may switch the feature on (§5.1);
   - an `own_state` arm that asks **what registration asks** (`[tools]`,
     the loopback validators, `SearchBackendConfig::problem`), never a
     field's presence. Where the real predicate lives in `mecha-cli`, it
     moves to core so both can call it — `build_search_chain`'s refusals
     became `SearchBackendConfig::problem` in #428, held to the builder by a
     test beside it, which is how `search` and its tool cannot drift;
   - its tools registered through the registry; its route group and CLI
     verbs naming it as owner; its web entries keyed on its `id`;
   - a setup step, and its `Recommendation` rows with their evidence;
   - if it replaces a config switch a `harness::Lever` reads, the lever
     follows it.

   `CLAUDE.md` gets **one bullet**, not the checklist — it rides in every
   agent's context — under Conventions: *a new optional feature starts at
   `ARCHITECTURE.md` §Features*, with the one incident that earns the line.
   The table-driven test in "How to know it works" is what makes the
   checklist enforceable rather than advisory: a feature missing a step
   fails it.

### How to know it works

- A table-driven test over the registry: for each feature, build a config
  without it and assert that its tools are absent from `mecha tools --json`,
  its verbs exit with the one sentence, its routes return 404 `feature_off`,
  and `/api/features` reports `Off`.
- **Every route belongs to exactly one feature or to the core.** The route
  lists today are written by hand inside tests (the four
  `*_sit_behind_the_owner_guard` tests, one per group), so step 3 declares each
  route's owner where it is registered, and a test walks that declaration so
  an unowned route fails the build — otherwise the next tab added is visible
  on every install.
- The negative is not vacuous: the same test with each feature **on** must
  see the tools, the verbs and the routes. A guard that refuses everything
  passes the first test.
- `/api/features` opens no socket: a test with a listener on the OCR port
  asserts that it was never connected to.
