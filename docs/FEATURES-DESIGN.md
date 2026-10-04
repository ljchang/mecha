# Features — design

> **Status (2026-09-30):** this design merged as #427; **step 0 shipped** —
> the registry and a read-only `mecha features`, merged as #428 and deployed
> the same day, with the store-location fixes it surfaced in `doctor`, Slack
> and the booking sweep as #432 and #433. `docs/ARCHITECTURE.md` §Features
> describes what is built. **Step 1 is split in two**: 1a — the `[features]`
> table, `mecha features enable|disable`, the upgrade notice, `mecha setup`'s
> offers and the environment refusal, gating nothing — shipped as #443;
> 1b — tools and the known servers register only when switched on, trials
> default their switches from the servers they carry, and `mecha serve`
> refuses without `web` — merged as #445. **Step 2 is built**:
> `/api/features`, and the web app showing and hiding by its rows, with
> Settings → Features; building it found that `state` answered `Off` for a
> switched-on feature missing its settings, which §5 calls `Unready`, and
> fixed that in core — merged as #449. **Step 3 is split by feature, not by
> surface**, so no feature is ever half-gated: 3a guards the features whose
> switches already turn something off (mail, docs, graph and its board,
> search, documents, image and its library) on every route and verb, with
> every route declaring its owner; 3b guards Slack, personas, voice,
> incognito and the front door whole, by flipping `Feature::gated` — merged
> as #451 (3a); **3b is built**, with messages among them by the owner's
> ruling M1 (`mecha msg` refuses when messages is off). Ruling
> L1 (2026-09-30): the library follows `image` — `[tools]` withholds only the
> model's library tools, never the page; the library's lock and portrait
> routes are core, because Personas uses them. One deliberate departure from §5: `[features] messages` is
> applied *into* `[messages] enabled` (three-state since 2026-10-01, so its
> `false` is a no like any switch's) rather than or-ed with it, so experiment
> levers keep one field (ARCHITECTURE §Features says why). **Step 4 is split
> too: 4a — `mecha setup` iterates the registry, `mecha setup <feature>` and
> `--minimal` — is built, and so is 4b** (a run records the feature set
> on its session record, and an environment's `requires`). **Step 5 is
> built** (2026-10-01): `[voice]` holds `stt_url`, `offer_target`,
> `voice_port` and `voices_dir` (the owner chose all four; `[web]
> voices_dir` is a one-release alias, and the two serve flags override per
> run), and `[personas] crisis_cooldown_minutes` is fully the owner's to set
> (the owner's ruling — the cooldown is the window in which a further hit
> does *not* re-pause, so longer is weaker). **Step 6 is split in two**:
> 6a — `hardware.md` gains F5's separate-GPU
> column and a *Beside the chat model* table of every feature's model, cost,
> residency and evidence, measured on the GB10 (each row dated), and the graph
> page names the embedder mecha-graph uses — is built; so is 6b: the
> registry in `recommend.rs`, `mecha features --probe`, and the page's table
> generated from the rows, with a test that fails when they disagree. **7a is split in three**: 7a-1 — `fetch.rs`, the one hub
> resolver (the launchers and the layout installer brought onto its order)
> and the resumable, sha256-checked downloader — is built (#521); so is
> 7a-2: `sidecar.rs`, the manifest's reader, provided-detection and the
> read-only `mecha features plan <id>` (#526); and so is 7a-3: `install.rs`,
> a pinned `uv` (owner's choice, 2026-10-03: uv now, not at 7d), layout
> installed through the manifest, and `mecha features enable` offering the
> plan and installing on one yes (`--no-install`, and a refusal without a
> terminal). **7b is split in three** (§10.6): 7b-1 — `engine.rs`, the
> pinned release installed side by side behind a `current` link, the asset
> chosen from the driver, offered by `features enable` where the engine runs
> something — is built; so is 7b-3a: `engine_gate.rs`, the measurement
> gate (each unit's own launcher on a private port, both engines in turn,
> under a held switch that declines rather than waits), its ledger, and
> `mecha setup engine` with `--adopt` and `--rollback`. 7b-2 (the build
> fallback), 7b-3b (`--upgrade` and an upgrade's rollback), 7c–7f and step
> 8 are unbuilt (step 7 redesigned in §10). The
> feature set rides on the session record and, since the owner's ruling
> of 2026-10-01, in every experiment row's condition hash —
> the environment's digest held every switch but `search`, which follows
> the operator's (ARCHITECTURE §Features). The owner
> ruled F1–F6 the same day (§7): the switch is a `[features]` table of
> bools — not a table's presence, which this doc first recommended — and §5
> is written to that ruling; F5 is `hardware.md`'s four tiers, in two
> columns (unified memory, and a separate GPU beside system RAM). Step 8 (how
> to add a feature, in `ARCHITECTURE.md` and `CLAUDE.md`) is the owner's
> addition. **Step 7 was redesigned on 2026-10-02** (§10, rulings F7–F10):
> enabling a feature offers to install its sidecar software and a chosen
> model, from installers that ship inside the binary.

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
design. §7 is the rulings. §8 is what this deliberately does not do. §9 is
the build order, and §10 (2026-10-02) is installing what a feature runs,
with step 7's own build order (§10.6).

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
| Model recommendations? | Yes, as data on each feature: model, pinned source, memory, and whether it was **measured** or is arithmetic. Chosen and downloaded in setup; never written into config | §6, §10.4 |
| Does enabling a feature install what it runs? | Yes, on one yes (F7): the sidecar software and a chosen model, from installers inside the binary; what already runs is left alone (F8); llama.cpp is a pin that moves, measured before it is promoted (F9) | §10 |

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
| **501** | voice cloning without `[voice] voices_dir` (formerly `[web] voices_dir`) |
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
| Embeddings (graph) | `harrier-oss-v1-0.6b` f16 | `scripts/llama/mecha-embed-server`, `LLAMA-SERVER.md`; `website/docs/features/memory/graph/index.md` names it as of step 6a (#512) |
| OCR | PaddleOCR-VL 1.6 (GGUF + mmproj) | a comment in `scripts/llama/install.sh`, `DOCUMENT-EXTRACTION-DESIGN.md` §5–6 |
| Layout | `PP-DocLayoutV3.onnx` | `scripts/layout/install.sh` |
| Image generation | Qwen-Image 2.1 Q4, `qwen3vl_8b_w4a8`, the 2.1 VAE | `ImageConfig` defaults, `features/tools/image-generation.md` (~15 GB peak, measured) |
| Voice | Parakeet TDT 0.6B v3 int8, Breeze TTS 2 Q6_K (Chatterbox Turbo before 2026-10-03), Silero VAD, smart-turn v3 | `features/interfaces/voice.md`, `VOICE-RESEARCH.md` |
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
  treated as on — **not** a settings table being present: four features'
  evidence is not a table at all — `slack` a token store, `personas` a
  non-empty store, `voice` an installed unit file (installed, not running: a
  socket-activated unit is idle until asked, and §4.3 reads unit files, never
  sockets), `frontdoor` a binary on PATH — and a table-keyed notice could not
  fire for any of them. It iterates only features that *have* a switch
  (`part_of().is_none()`), since a part has no bool to announce and `enable`
  refuses a part id by name. The substitution covers every absent switch,
  not just the one being announced, because on this install the dependencies
  are absent too: substituting `incognito` alone would leave `web` off and
  short-circuit it to `Blocked`, so the line would never print. An explicit
  `false` is an answer, not an unanswered question, and is never substituted:
  an owner who wrote `web = false` is not told to enable `incognito` on every
  start (found on review of #435, passes 3 and 4). The notice and F6's
  detector are one function. **Two obligations on step 1
  before it can key on this** (found on review of #435): `personas` and
  `voice` are unconditional `On` in step 0 ("no switch yet"), so step 1 must
  give them the evidence named here first, or both lines print on every
  light install forever; and a feature whose dependency's bool is **also
  absent** (`incognito` with no `web` key) is announced with the dependency
  first, because the substitution holds only inside the notice — on disk
  `web` is still absent, and `enable incognito` alone would land in
  `Blocked` — *"`incognito`: needs `web` — `mecha features
  enable web incognito`"* — never offered alone into a `Blocked` it cannot
  leave;
- `mecha features` shows that case as its own row (switch absent, would
  otherwise be usable), not a bare `off`, as it does an off front door with requests
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
- `mecha features --probe`, `mecha setup` and `mecha features enable` (§10.2
  item 4's provided-detection) may probe, using only calls that
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
at the top level only, so a table with a nested layer gets a nested-layer
test alongside (`[features]` itself is a map, with none). A fifth decision
comes with every new settings table: which of `trial_env`'s two lists it
joins, if either (§9 step 8).

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
  whose switch is the alias of `[messages] enabled` — `documents` since #441,
  and, once step 5 adds them, `voice` and `personas`) or in
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
  reason. **`[documents]` was missing from `OPERATOR_ONLY_TABLES`** though
  `merge_file` strips it from project layers for `[image]`'s reason — its
  `ocr_url` is where the owner's documents go and `confine` is the PDF
  parser's sandbox — so an environment could set both, with or without this
  design. #441 added it and corrected the constant's "the five" comment,
  since this rule is only as good as that list (found on review of #427,
  pass 6). The refusal is
  reason, never ignored with a warning: a warning fails open in exactly the
  way the `requires` bullet below exists to close — the arm would run without
  the feature it asked for and be scored anyway. An environment may set any
  key to `false`. Four tests pin it: `graph = true`, `search = true` or
  `messages = true` in an environment file refuses the trial; `graph` absent
  with a manifest carrying no graph server reads off; `graph = false` beside
  `live_servers = ["graph"]` reads off; and — the positive, and the only one
  a step-1 default of `false` would fail — `graph` **absent** beside
  `live_servers = ["graph"]` reads **on**, so existing `live_servers`
  experiments keep their graph (found on review of #427, passes 4 and 5,
  and #435). Turning features off is how a trial is made light; this is the switch
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

Each feature carries `Recommendation` rows — **built as slots** (6b,
`recommend.rs`): a model is loaded once however many features need it (the
embedder serves `graph`, `documents` and `personas`), so the rows hang off a
`Slot` naming every feature that needs the model, and the probe counts a
slot once if any of them is shown. Keyed by feature, a shared model was
either under-counted or counted twice. A row also says what its figure
`counts` and what it `excludes`, so a figure that leaves something out (the
chat server's process memory) is named in the probe's output rather than
presented as whole:

```rust
pub struct Slot {                 // one per model (6b, `recommend.rs`)
    pub id: &'static str,         // "embeddings"
    pub label: &'static str,
    pub needed_by: &'static [Feature], // every feature that needs it; empty = all
    pub runs_on: RunsOn,          // Gpu | Cpu
    pub residency: Residency,     // Resident | OnDemand | ReleasedOnIdle | PerRequest
    pub rows: &'static [Recommendation],
}

pub struct Recommendation {
    pub tier_gb: u32,             // 16, 32, 64, 128 — hardware.md's tiers (F5)
    pub memory: Memory,           // the column and its cost, as one value (F5)
    pub model: &'static str,      // "PaddleOCR-VL 1.6 (GGUF and projector)"
    pub counts: &'static str,     // what the figure counts
    pub sources: &'static [Source], // pinned (§10.4); empty when the sidecar fetches it
    pub excludes: Option<&'static str>, // what the figure leaves out, named by the probe
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

- **Fetched on a yes, never written.** `mecha setup <feature>` shows the rows
  for this machine's tier and downloads the one chosen (§10.4, the owner's
  ask of 2026-10-02; this line first said *printed*). It does not write a
  model name into config: the config value is read back from the running
  server (`mecha setup --write`), per `onboarding.rs`'s rule that setup never
  writes down a number the user merely believes.
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
  memory and the host's — never one, and a `null` is per pool: a row whose
  `host` is `Unmeasured` nulls the host sum and leaves the GPU sum standing,
  never the reverse and never both: a single sum would pass a chat model
  that does not fit the card and fail an OCR server that fits host RAM with
  room to spare (found on review of #435). This is why
  `residency` is on the row: an on-demand OCR server costs nothing until a
  PDF arrives, and image generation is the same class — mecha asks ComfyUI
  to free its models after `[image] unload_after_secs` (600) — with one leak
  the row must carry: the timer lives in the process that drew the picture,
  so a one-shot `mecha run` or a serve restart inside the window leaves
  ~12 GiB held (measured on 2026-10-02: 11.9 GiB nine hours after a picture
  drawn five minutes before a serve restart). `[image] min_available_mb`
  refuses a start that would not fit.
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

| # | Decision | Ruling (F1–F6 2026-09-30; F7–F10 2026-10-02) |
|---|---|---|
| **F1** | What turns a feature on | **A `[features]` table of bools** in the global config, every feature listed. The owner, overruling the doc's recommendation of table presence: *"The problem with table existing is that users need to know what features are available. I feel like a registry or having to toggle bools is a better design."* §5's three rules are what keep the bool from being a second source of truth |
| **F2** | Off in the web app | **Removed from navigation**, as the owner asked in the opening message; Settings → Features lists everything. `Unready` and `Unknown` are shown with a banner, never removed (§4.1) |
| **F3** | Web as a feature | **Optional like the rest** — the owner: *"Web should also be optional feature."* CLI, TUI and Slack are complete without it; incognito, and voice's browser parts (`dictate`, `calls`, `cloning`), report `Blocked(web)`. `voice` itself does not: `mecha voice-serve` is its own loopback surface, and `Blocked` would refuse it (found on review of #427, pass 7). A tab's visibility is not a `requires` relation: with `web` off there is no navigation at all, so the Personas and Library tabs need no dependency on it — and giving `personas` one would make `Blocked` refuse `mecha persona` from the CLI, which works without the web (found on review of #427) |
| **F4** | What an off route returns | **404** with `{"error":"feature_off","feature":"image","fix":"mecha features enable image"}`, only behind `owner_guard` (§4.2 item 4). 503 stays for `Unready` |
| **F5** | Recommendation tiers | **`hardware.md`'s four — 16, 32, 64 and 128 GB — in two columns**: unified memory, and a separate GPU beside system RAM, where the tier is the GPU's memory and the auxiliary models (OCR, embeddings, speech to text) may run from system RAM or the CPU. One page and one table agree on the tiers; the column is what keeps a 24 GB GPU with 64 GB of RAM from being steered as a 24 GB machine. Only the 128 GB unified row is measured (this GB10); every other cell says `Arithmetic` or `Unmeasured`. Ruled by the owner 2026-09-30 |
| **F6** | Existing installs, when `[features]` arrives | **`mecha setup` offers.** It detects a feature in use (an `[image]` table, a mail `[[mcp]]` entry, a non-empty persona store, an installed voice unit file) and offers to write its bool — the same predicate as the upgrade notice (§4.2), so the two cannot disagree. **Which states count** (owner, 2026-09-30): `On` and `Unready`, the latter named with its reason ("no account is authorised yet"); `Unknown` never, or the offer would write a bool off a store it could not read. Never grandfathered as on: that is a second source of truth. On this machine the deploy that ships the table writes it by hand, in the same change, so nothing disappears |
| **F7** | When a feature's sidecars install | **Enabling offers it.** `mecha features enable <id>` (and `mecha setup`) shows what it will download and install, with sources and sizes, and installs on one yes; `--no-install` flips the switch only. Off never uninstalls; `mecha setup <feature> --remove` does. Ruled by the owner 2026-10-02 (§10.2 item 3) |
| **F8** | This machine's hand installs | **Detected and left alone.** A sidecar whose port answers, or whose unit mecha did not write, is *provided*; nothing is installed over it and no file mecha did not write is touched. Moving this box onto managed copies is a later, explicit step. Ruled by the owner 2026-10-02 (§10.2 item 4) |
| **F9** | How llama.cpp is obtained | **A prebuilt release when one matches, else a build from the pinned commit.** Prebuilt assets are verified by sha256; the build checks the toolchain first. Ruled by the owner 2026-10-02, with the owner's observation that updating it often improves performance — so `--upgrade` measures before it promotes (§10.3) |
| **F10** | Trusting an engine newer than the shipped pin | **Only for a tag the owner confirmed at a terminal.** `--to <tag>` names it; bare `--upgrade` prints the newest tag and asks for confirmation of that tag before downloading. For a confirmed tag the release API's sha256 over TLS is trusted (and, for a build, the commit GitHub names for the tag) — the one exception to item 1's reviewed-pin rule, and only on this path. Ruled by the owner 2026-10-02 (§10.3) |

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
- **Detecting the machine's hardware beyond memory and the GPU's compute
  capability.** (Downloading models was here until the owner's ask of
  2026-10-02; §10 replaces it. The compute capability is what §10.3's build
  fallback needs.)
- **macOS and Windows install paths.** §1.4's units are Linux user units;
  what replaces them elsewhere is its own question.
- **The harness levers** (§3). They already have their own closed set.

---

## 9. Build order

Each step is a PR, and each leaves every surface working.

0. **`feature.rs` and `mecha features`**, read-only. The registry, `state`,
   and the list command. Nothing else changes; its output on this machine is
   checked by hand against §5.
1. **The `[features]` table**, in two PRs. **1a** gates nothing: the table,
   the writer, the upgrade notice, F6's offers and the environment refusal,
   so this machine's answers are written before anything reads them. **1b**
   switches tool registration and server connections to the registry, with
   `config_at`'s `live_servers` default for `graph`. What 1a builds:
   `Config`, `ConfigLayer`, `apply` and the project-layer strip (a map, so no
   nested layer); `state` reads the bool
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
   true (#435). Fix the embeddings page. (Split in #512: 6a is the page,
   6b the rows, the probe and the test.) The rows carry their pinned
   `Source` (§10.4), not a download command, so 7a's downloader can fetch
   what setup lists.
7. **Installers** — §10, which replaced this step's first form (one
   `scripts/<feature>/install.sh` per feature) on 2026-10-02: a user who ran
   `cargo install` has no `scripts/`, so installers ship in the binary,
   enabling a feature offers them, and they fetch the sidecar software and a
   chosen model. Split 7a–7f (§10.6). What carries over is the intent of
   "copy, never symlink": **nothing mecha writes points into a checkout**,
   and no unit names `/home/<user>`. A link *between* two mecha-owned
   directories under `~/.mecha/sidecars/` is allowed — it is how an engine
   is promoted and rolled back (§10.3).
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

---

## 10. Installing what a feature runs

**2026-10-02.** The owner, after step 6a:

> "if possible, i would also like to have all of the sidecar software
> installed if the feature is enabled. Not sure if this is needed for llama
> server, comfyui, python, or anything else we are using. It would be great
> if it was really easy for a user to install and get started, especially
> with us giving default models. Part of the setup process could be
> selecting from the recommended models and downloading it."

and, on llama.cpp: *"I'm finding often updating it improves our model
performance."* This replaces step 7's shell scripts and reverses two lines
of this design: §8's "downloading models" and §6's "printed, never written"
(the model is now fetched on a yes; the *config* rule — never write down a
number the user merely believes — stands). Rulings F7–F10 (§7) settle the
four decisions it raised; F10 came from this section's own review.

### 10.1 What a clean machine cannot reproduce today

Inventoried on 2026-10-02 against this box. A **sidecar** is a program or
model, not mecha's own, that a feature needs running.

| Sidecar | Features | How it got here | In the repo |
|---|---|---|---|
| llama.cpp (`llama-server`) | the chat model (every feature), `graph`, `ocr` | built by hand from a clone (CUDA, sm_121, shared libs); `~/.local/bin/llama-server` is a stub whose RUNPATH names the build tree | no build commands, no pin; `LLAMA-SERVER.md`'s upgrade bullet named `~/llama.cpp/build/bin` where the stub's RUNPATH is `~/llama.cpp-next/build/bin` (corrected there in this change) |
| the router and its chat model | every feature | `llama-local.service`, box only; its drop-in runs the *working tree's* `scripts/start-router.sh` | the launcher; no unit; `start-router.sh` prints an `hf download` line when the model is missing |
| embeddings server | `graph`, persona file search | the always-on unit is box only; `install-embed.sh` converts it to on demand and refuses to run without it | the launcher and the on-demand units |
| OCR server | `ocr` | `scripts/llama/install.sh` | **yes** — units, launcher, `--remove`; the model is a comment, unpinned |
| layout | `layout` | `scripts/layout/install.sh` | **yes** — hash-pinned requirements, model pinned by revision and sha256 |
| ComfyUI, ComfyUI-GGUF, three model files | `image` | `git clone` at `88ab4a06` and `6ea2651`, a venv with torch cu130; downloads in a box-local `dl-logs/download.sh` | file names only; no unit, no requirements, no URLs |
| voice venv (pipecat, sherpa-onnx) and Parakeet | `voice`, `dictate`, `calls` | `python3 -m venv`, unpinned; the Parakeet tarball by hand | the servers' source; units that name `/home/ljchang` |
| Chatterbox | `calls`, read-aloud | a Docker image built by hand from `nvcr.io/nvidia/pytorch`, recipe in prose (`VOICE-RESEARCH.md`), mounting the live checkout | the server's source only |

Only layout is reproducible from the repository, and **none of it is
reachable by a user who ran `cargo install mecha-cli`**: they have no
`scripts/` directory. That one fact decides the shape below.

### 10.2 The design

1. **A closed `Sidecar` registry in `mecha-core`, beside `Feature`.** Each
   entry names the features that need it, the port and health check that
   prove it is running, its install method, its disk size, and its sources —
   every one pinned: a release asset by a sha256 a reviewer read and
   committed (GitHub's per-asset `digest` is how that pin is *authored*,
   never what is trusted at install time — that is F10's exception alone),
   a git commit, a Hugging Face file by revision and sha256, a
   requirements lock with `--require-hashes`. A source enters only by a
   reviewed change to the binary, like a feature (§8): no URL from config,
   a project file, or the model.
2. **Installers ship inside the binary.** Unit templates, launchers and lock
   files are `include_str!`'d and written out by `mecha setup`; units use
   `%h` and name no checkout. What runs is always the written copy, never a
   working tree's file (the 2026-08-20 incident `mecha-embed-server`'s
   header records). Payloads (venvs, clones, engine builds) live under
   `~/.mecha/sidecars/<id>/`; a manifest there records every file mecha
   wrote with its hash, which is what `--remove` removes and what an
   upgrade may replace.
3. **Enable offers the install (F7).** `mecha features enable image` prints
   the plan — each sidecar with its source and size, the model choice (10.4),
   the total download and disk — and installs on one yes; `--no-install`
   only flips the switch, and **answering no to the plan writes nothing** —
   the feature stays as it was, with `--no-install` named for an owner who
   wants the switch without the download. The switch is written **after** a
   successful
   install and its health check — a check that belongs to installing, never
   to enabling, so a provided sidecar (item 4) reaches the switch without
   one and nothing is woken. A failed download leaves the feature as it
   was with the command that resumes, never `Unready` with a half-written
   tree. `mecha setup` offers the same plan per feature. **A part's
   sidecars ride its parent's plan**, each as its own choice: a part has no
   bool and `enable` refuses a part id (§4.2), so `mecha features enable
   documents` offers the OCR server and the layout model separately, and
   `enable voice` offers Parakeet (`dictate`), the worker and Chatterbox
   (`calls`). Later, `mecha setup <part>` — which already resolves a part to
   its parent (`switch_owner`) — offers that part's sidecars alone. Switching a feature
   off never uninstalls; `mecha setup <feature> --remove` does, and keeps
   downloaded models unless `--models` is given, because the cache is shared.
   Even then it keeps any model file another enabled feature has claimed —
   **or could use**: any file that any `Recommendation` row of any switched-on
   feature names, and the chat model's files always, since every feature
   needs it. Over-keeping is the safe direction, because a switch can be on
   with no claim behind it (`--no-install`, F6's offer, a feature
   re-enabled after a failed install). It says which files it kept and why,
   so removing one feature never breaks another. **A claim
   is a manifest record, not a written file**: when setup resolves a
   feature's model — downloaded, found already in the cache, or a path the
   owner brought — it records `(feature, row, file)`, so a model mecha did
   not download (item 4 prices it at zero) is still claimed by the feature
   that uses it. Without the claim, the order of enablement would decide
   whether a shared file survives. The claim is setup's own record and
   writes nothing into config, so §6's rule stands.
   Installing runs only from a terminal: the web app still shows the command
   and never runs it (§8), and no tool exposes it to a model. **Without a
   tty, nothing installs and nothing is silently skipped**: `features enable`
   refuses with the `--no-install` hint (which still writes the switch, as
   today), and `setup engine --upgrade` refuses whether or not `--to` is
   given — a flag typed into a trigger, a hook or `ssh host …` is not the
   owner at a terminal, which is what F10's trust rests on. That is a
   deliberate change for scripted callers of `features enable`, which today
   is a plain config write; none in the repo calls it. **An install that
   succeeds but fails its health check** leaves the switch unwritten and
   says so — installed, not answering, with the check's error and the
   command to retry it — so a manifest with the feature off is never a
   silent half state.
4. **What is already running is provided, not installed (F8).** Before
   planning, each sidecar is asked with **§4.3's load-free probes only, by
   install method** — never a uniform `/health` on its port, because the
   first connection to a socket-activated server *is* its cold start: the
   systemd unit state for the on-demand servers (OCR, embeddings), whose
   socket unit existing and being enabled is enough; `served_props` /
   `GET /models` for the router; ComfyUI's `/system_stats`, which §4.3
   requires to be confirmed load-free before step 6 relies on it (6b's
   `--probe` is the first reader; 7e re-confirms it on the installed
   ComfyUI); the voice
   servers' unit state. **An idle-stopped sidecar is provided, never
   absent** — a probe that read it as absent would plan an install over it,
   the llama-embed incident of 2026-08-19 that `llama-ocr.socket`'s header
   records. **Three things make a sidecar *provided***, and the plan
   installs nothing for it: a running answer; a unit of that name mecha did
   not write; or **a payload on disk, outside `~/.mecha/sidecars/`, that
   mecha's manifest does not claim** — a hand-built clone or venv, a Docker
   image of the sidecar's name, an engine tree. Mecha's own tree is never
   provided: the manifest row is written **before** the first byte, marked
   `incomplete` until the health check passes, so an interrupted install
   reads as *resumable* — with the command that resumes it — never as
   someone else's install. The third is what covers a stopped sidecar with no unit
   (ComfyUI here) and one with no probe at all (Chatterbox, a container).
   Models are the same: a recommended model the hub resolver (item 6)
   already finds **and whose sha256 matches the row's** is priced at zero in
   the plan's download total; one that is present but does not match — a
   truncated or different file — is neither provided nor handed to a
   launcher, and the plan offers to fetch it. A file mecha
   did not write is never overwritten. On this machine every sidecar is
   provided, so setup installs nothing here. Moving this box onto managed
   copies is a separate, explicit step, later, and its first target is named:
   `llama-local.service`, whose drop-in runs the working tree's
   `scripts/start-router.sh` — the 2026-08-20 class item 2 exists to end.
5. **Python through `uv`.** One pinned `uv` binary (sha256) under
   `~/.mecha/sidecars/uv/` when the machine lacks one; every venv is created
   by it from a hash-locked requirements file per platform, with its own
   Python, so a system Python's version is never a requirement. Torch-heavy
   sidecars (ComfyUI, Chatterbox) carry one lock per accelerator (CUDA x64,
   CUDA arm64, Apple, CPU).
6. **Models download natively, resumable and verified.** Rust fetches the
   pinned file from Hugging Face's resolve URL into the standard cache
   layout, through **one hub resolver** that the unit templates and launchers
   share, so the download and the launcher cannot look in different places
   (a verified download the launcher then calls missing). Its order is
   `HF_HUB` (mecha's own, which `start-router.sh` and the embed launcher
   read today), then the `hf` CLI's `HF_HUB_CACHE`, then `HF_HOME/hub`, then
   `XDG_CACHE_HOME/huggingface/hub` (`hf`'s own default for `HF_HOME`, added
   on review of #521), then `~/.cache/huggingface/hub` — the `hf` CLI reads
   the middle three and not the first, and it is still how layout's model and the router's missing-model
   hint arrive, so with `HF_HOME` set today the two already disagree. 7a
   brings the launchers onto the resolver's order. It resumes a partial
   file, and refuses one whose sha256 differs — no `hf`
   CLI and no Python needed for a llama-only feature.

### 10.3 The engine, and keeping it current (F9)

llama.cpp lands ~25 commits a day with a release per merge (`b11347` on
2026-10-02, 161 commits after this box's build of 2026-09-26), and the owner
measures that updates help. So the engine is a **pin that moves**, never a
pin that sits:

- **Getting it:** an official release asset when one matches the OS,
  architecture and backend — the release carries `ubuntu-cuda-13.4-arm64`,
  `-x64`, `macos-arm64`, Vulkan and CPU builds — verified by its sha256;
  otherwise a build from the pinned commit, after a toolchain check (cmake,
  a compiler, `nvcc` and the GPU's compute capability from `nvidia-smi`)
  that names what is missing. **The arm64 CUDA asset runs on the GB10**
  (measured 2026-10-04, `b11391`): its `libggml-cuda.so` carries `sm_121a`
  kernels, CUDA 13.4's runtime runs on the 580 driver (CUDA 13.0) by
  minor-version compatibility, and the embeddings model loaded on the GPU
  answered with a cosine of 1.000000 against this box's from-source
  `b11205` on two inputs. A CUDA asset's runtime comes in a second archive
  (`cudart-llama-…`, 527 MiB on arm64), pinned beside the first and
  unpacked into the same directory. The per-merge `bNNNNN` releases are
  now marked *prerelease*, and `releases/latest` names a semver release
  (`v0.5.0`) that carries no binaries — so "the newest release" for
  `--upgrade` (7b-3) is the newest `b` tag, never the API's `latest`.
- **Self-contained, so a link can move it.** A build from source bakes the
  build tree's absolute path into the binary's RUNPATH (§10.1's stub, and
  `-next` after it), so copying `build/bin` somewhere and swapping a link
  would change nothing that runs. The build fallback therefore installs
  with `cmake --install` into the engine's directory with an
  `$ORIGIN`-relative RUNPATH, and 7b asserts it with the `readelf -d` check
  `LLAMA-SERVER.md` prescribes; a release asset is checked the same way
  rather than assumed.
- **Side by side:** each engine lives in `~/.mecha/sidecars/llama/<tag>/`,
  with the **resolved commit sha** recorded in the manifest and the ledger
  (a tag is a label; a rollback and an audit need the build), and the units
  name a `current` link, so an upgrade is a new directory and
  a link swap, and the previous one stays for `--rollback` (today's `.prev`
  copy, made structural). **A plan does not run the engine**, so a build
  directory emptied by hand still reads installed from the record, and the
  engine's install — which fetches no model — is never re-offered for a
  missing one; the build's own health check (`engine::health`) is what
  would find it, and putting that check where an owner meets it is `doctor`'s
  work in 7c.
- **`mecha setup engine --upgrade [--to <tag>]`** fetches or builds the new
  engine (`engine` is a reserved noun in `setup`'s feature position, never a
  feature id), then **measures before it promotes**. The measurement loads
  another engine's copy of the chat model, so in `hold.rs`'s terms it **is a
  switch**. It measures **one engine at a time** — the old engine, stop, the
  new one, stop — and, since the router keeps its model resident from boot
  (`load-on-startup`, and no idle eviction), it **stops the router first**,
  once it owns the switch and has seen no run live, and restarts it at the
  end on whichever engine won. So it never holds two copies of the chat
  model (two at ~28.5 GB each is an out-of-memory failure on every tier
  below 128 GB), and it refuses up front, with the number, when available
  memory cannot hold the largest single step: the chat model plus whichever
  smoke-test models are installed (§6's sum, applied to the measurement),
  counting the router's resident copy too if it cannot be stopped.
  **Every leg runs against a server the gate starts from the engine under
  test, on a port of its own — never :8080, :8081 or :8085**, which reach
  the live router and the socket-held backends on whatever engine they
  already run, so a pass there would grade the predecessor and credit the
  candidate. As a switch,
  it writes one with `begin_switch` — which makes runs that start
  meanwhile wait — and then checks `live()`. A hold alone would not do:
  `try_hold` is not exclusive and returns `Ok` beside other runs, so a
  benchmark behind it would run under contention and write the number this
  rule exists to keep out of the ledger. **If any run is live it withdraws
  the switch and declines**, saying so, rather than waiting — a deliberate
  departure from the module's ruling that a switch waits without limit,
  because nobody wants an upgrade benchmark holding the router back
  indefinitely; the owner reruns it when the box is quiet. If
  `begin_switch` finds a switch already pending (a `mecha model use`
  waiting on runs), it declines the same way and names it — it never
  queues behind, or cancels, someone else's switch. Then: the chat model loaded on
  both engines with the router's flags, one completion and one embedding
  as a smoke test, then single-stream generation and prefill at fixed
  prompt lengths (`scripts/bench-slots.sh`'s method). It promotes when the
  new engine is no slower beyond the measured noise, says so with both
  numbers, and otherwise keeps the old one unless `--force`. Each run
  appends to a ledger, which is what turns "updates help" into `Measured`
  rows. The switch restarts the router only when no run holds the model
  (`hold.rs`, the update skill's `serve_held`).
- **The shipped pin** is the newest engine the project has measured, bumped
  by a reviewed change, so a new user gets a known build and `--upgrade`
  gets them the latest.
- **`--upgrade` has its own trust rule (F10)**, distinct from item 1's. A tag
  past the shipped pin has no reviewed hash by construction, so the upgrade
  path trusts **the release API's digest over TLS for a tag the owner has
  confirmed at a terminal**. `--to <tag>` names one; bare `--upgrade`
  resolves the newest release, prints its tag, and asks the owner to confirm
  *that tag* before anything is downloaded — printing is disclosure, the
  confirmation is the choice. A build from an unreviewed tag trusts the
  commit GitHub names for that tag. It is the only path that trusts the
  serving channel's own hash, and it is acceptable only because the owner,
  at a terminal, chose the tag — not config, not a project file, not a
  model. Every other source still needs review.
- **Adopting a provided engine is the explicit step F8 deferred.** Today
  all three launchers run `${LLAMA_SERVER:-llama-server}` from `PATH`
  (`path.conf` puts `~/.local/bin` first), so nothing reads a `current` link
  until the units' `LLAMA_SERVER` names it — which is 7c's unit templates on
  a clean machine. On a machine whose engine is *provided* (this one),
  `setup engine --upgrade` **refuses**, naming `mecha setup engine --adopt`:
  a promotion no server reads would write a `Measured` row for an upgrade
  that reached nothing — a half-applied promotion reading as complete.
  `--adopt` installs the shipped pin side by side, points the three units'
  `LLAMA_SERVER` at `current` under the same gate as a promotion, and keeps
  the hand-installed stub untouched as the first rollback. It runs only when
  the owner asks; nothing adopts by default.
- **One engine serves three servers.** Once adopted (or installed by 7c),
  the router, the embeddings server and the OCR server all run the
  `current` link (`LLAMA-SERVER.md`, the bullet *Upgrading llama.cpp: the build tree is the deployment*),
  so a promotion reaches all three: the router at the gated restart, the
  two on-demand servers at their next cold start, ungated. So the gate's
  smoke test covers each — a chat completion, an embedding, **and an OCR
  page** (the mtmd path the chat measurement does not exercise) — and a
  failure in any keeps the old engine. **A smoke test whose model is not
  installed is *not run*, never *passed*** — on a light install that is
  usually two of the three. The gate runs the ones it can, promotes on
  those, and the output and the ledger row name the ones not run, so a
  `Measured` row never covers more than was measured; requiring all three
  would make an engine upgrade depend on enabling features the owner
  declined.
- **The switch is held from before the measurement to the end of the
  promotion.** `begin_switch` is taken before the first load and withdrawn
  only after the router answers on the winning engine, so no run starts on
  either engine in between, and no run is answered by the old engine while
  the link already names the new one. Inside it, a promotion swaps the
  link, stops the two on-demand backends (their sockets stay, so the next
  request starts them on the new engine — a warm OCR or embeddings server
  would otherwise answer on the old one for up to ten idle minutes), and
  restarts the router. A runs-live refusal happens only at the start, before
  anything moved; after that, the one way to a **partial** promotion is a
  step that fails — the router does not come back, a backend will not
  stop — and the output and the ledger row say so, name the step and its
  error, and print the command that finishes it. A half-applied promotion
  must not read as complete.
- **`--rollback` undoes all three, under the same switch.** It takes
  `begin_switch` — and, unlike the measurement, **waits** for live runs as
  any switch does (`hold.rs`'s ruling), because a rollback is the one change
  the owner wants even when the box is busy; `--now` is the way out, as it
  is for `mecha model use` (`request_now` cancels the holders). Once clear,
  it swaps the link back, stops the two on-demand backends, restarts the
  router, and withdraws the switch. A step that fails is reported the same
  way as a promotion's: named, with the command that finishes it, never as
  a rollback that completed.
- **`--adopt` is terminal-only too.** It downloads only the reviewed pin, so
  F10's trust is not at stake, but it changes what every server runs on a
  hand-installed machine, so it refuses without a tty like `--upgrade`, and
  runs under the same held switch as a promotion.

### 10.4 Choosing a model

Step 6b's `Recommendation` row gains its source, so the list setup shows is
the list it can fetch. The type, so 6b and 7a cannot read it differently:

```rust
/// Where a pinned sidecar or model comes from. Every variant carries a hash
/// or commit a reviewer committed (§10.2 item 1); F10's upgrade path is the
/// one exception, and it never constructs one of these.
pub enum Source {
    /// A release asset, per platform. A bump carries one sha256 per asset it
    /// pins — one per platform asset §10.3 lists for an engine release.
    ReleaseAsset { repo: &'static str, tag: &'static str, asset: &'static str, sha256: &'static str, bytes: u64 },
    /// A clone; `bytes` is the reviewed size of the checkout, for the plan.
    GitCommit { url: &'static str, commit: &'static str, bytes: u64 },
    /// One repository at one revision, and every file the row needs from it —
    /// a GGUF and its projector, or a diffusion model's three files. A row is
    /// never a projector-less model.
    HuggingFace { repo: &'static str, revision: &'static str, files: &'static [HubFile] },
    /// A `--require-hashes` lock, shipped in the binary; `bytes` is the
    /// reviewed download size of the packages it pins.
    PythonLock { lock: &'static str, bytes: u64 },
}

pub struct HubFile { pub path: &'static str, pub sha256: &'static str, pub bytes: u64 }
```

Every variant carries a size — `HuggingFace`'s is the sum of its files' —
so the plan's download total is a sum, never an estimate.
For each feature the plan shows the rows for this machine's tier and memory
shape, **the default preselected**, each with its evidence and whether it
fits beside what is already enabled (the sum, §6), and rows that do not fit
stay visible with the reason. *Bring your own* is always a choice: a path to
a GGUF, or a model already in the cache. After the download, the setting is
read back from the running server (`mecha setup --write`), never written
from the row.

### 10.5 Out of scope, still

macOS and Windows service managers (§8: the units are Linux user units; on
macOS the engine and models install, and starting them stays manual until
launchd is designed); system packages (`apt`, CUDA drivers, Docker) — setup
names them and stops; installing anything from the web app; and installing
inside a trial. An environment may switch a feature on (§5.1), but a trial
home never runs an installer: a switch that is on over an absent sidecar is
what §4.2 already shows — configured, not answering — and the trial reads
it as that, never as a reason to fetch.

### 10.6 Build order

Step 7 becomes these, each a PR that leaves every feature working:

- **7a.** The `Sidecar` registry, the manifest, provided-detection, the
  plan printed by `features enable` and `setup` (installing nothing yet),
  and the native downloader with its resume and hash tests. Layout's
  installer moves in as the first entry, since it is already pinned.
  Built in three PRs: **7a-1** the hub resolver and the downloader;
  **7a-2** the registry, the manifest, provided-detection and the plan;
  **7a-3** layout installed through them.
- **7b.** The engine: release-asset fetch, the build fallback, side-by-side
  directories, `--upgrade` with its measurement, `--rollback`, and `--adopt` for a provided engine.
  Built in three PRs: **7b-1** the pinned release, side by side, offered by
  `features enable`; **7b-2** the build fallback; **7b-3** `setup engine`'s
  `--upgrade`, `--rollback` and `--adopt`, with the gate and its ledger —
  itself two: **7b-3a** the gate, the ledger, `--adopt` and its
  `--rollback`; **7b-3b** `--upgrade` (F10) and an upgrade's rollback. The
  gate measures each server **as its unit runs it**: the unit's own
  launcher and environment (read from `systemctl --user show`), with
  `LLAMA_SERVER` and the launcher's port variable overridden, so the
  measurement carries the router's real presets and flags without mecha
  re-deriving them. Measured on the GB10 (2026-10-04): the embeddings and
  OCR legs pass on both this box's engine and `b11391`, the embedding
  agreeing at cosine 1.000000; the router leg is the owner's first
  `--adopt`, because it stops the live router.
- **7c.** The router unit and chat model choice; the embeddings and OCR
  servers on demand from nothing.
- **7d.** `uv` and the voice venv, Parakeet and the voice worker.
- **7e.** ComfyUI, ComfyUI-GGUF and the image models.
- **7f.** The speech server — Breeze (qwentts.cpp and its adapter), the
  default since 2026-10-03, built from a pinned commit with its model
  converted at install. (This step was Chatterbox, with a venv lock and a
  Docker recipe written down as a file, until Breeze replaced it.)

**How to know it works:** a clean container (no `~/.mecha`, no `scripts/`)
runs `cargo install` then `mecha features enable documents`, answers yes to
the OCR server, and gets a `document_read` that reads a scanned page through
OCR (`ocr` is a part, so `enable` refuses it by name — its install rides
`documents`' plan, item 3). Beside it, each of these fails on the behaviour
it guards against:

- the same run with the sidecar already provided — running, or **installed
  and idle-stopped** — installs nothing and wakes nothing;
- an install interrupted mid-download is offered as *resumable* on the next
  run, and resuming completes it — mecha's own partial tree is never read as
  provided;
- a hash mismatch on any source fails the install with nothing written and
  the switch untouched;
- `--no-install` writes the switch and fetches nothing;
- `--remove` deletes exactly the manifest's files and leaves a file mecha
  did not write in the same directory;
- `--upgrade` against a slower engine (a stub that answers slowly) declines
  to promote and says so with both numbers;
- without a tty, `features enable <id>` installs nothing and `setup engine
  --upgrade` refuses — with and without `--to`. (Plain `mecha setup` is
  covered already: without a terminal it prints the outstanding list and
  exits 1 before offering anything.) The install tests themselves drive a
  pty with a size, since the tty rule is what they cross;
- bare `--upgrade` with the confirmation declined downloads no byte, and a
  confirmation of tag *A* never fetches tag *B*: the tag resolved at fetch
  time is re-checked against the tag confirmed, and a mismatch stops;
- `--upgrade` on a provided engine refuses and names `--adopt`, writing no
  ledger row; `--adopt` without a tty refuses;
- a run that tries to start mid-promotion waits for the switch and is
  answered by the winning engine, never by the old one through a link that
  already names the new.
