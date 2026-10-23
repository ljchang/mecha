//! Which optional parts of mecha are on — one closed registry, answered from
//! the configuration and the disk (`docs/FEATURES-DESIGN.md`).
//!
//! **Closed on purpose.** The binary knows every feature and config only
//! chooses among them, which is how Codex's `FEATURES` table works and why a
//! repository, a skill or an MCP server cannot add one: the same argument
//! that keeps triggers and skills out of layered config.
//!
//! **No network, ever, in this module.** Several servers here start by being
//! asked — OCR and embeddings are socket-activated, the router loads any
//! model a request names — so a reading that probed would turn a page load
//! into a way to fill memory (§4.3). Everything below is a function of
//! [`Config`] and a [`Facts`] read from the disk. Liveness is a later,
//! separate reading, and `Unready` here only ever means a fact on disk says
//! the feature cannot work yet (a mail server with no mailbox authorised).
//!
//! **The owner's switch comes first.** Every feature with a switch reads its
//! bool in `[features]` before anything else ([`switch`]); a part rides its
//! parent. An install that predates `[features]` has every switch absent, so
//! every feature reads off — and [`announcements`] names the ones it had set
//! up, which the upgrade notice prints and `mecha setup` offers. Tool
//! registration and server connection ask [`switched_on`] — the switch,
//! never the readout (FEATURES-DESIGN.md §4.2 item 6) — and `mecha serve`
//! refuses without `web`.

use crate::config::{Config, McpServerConfig};
use serde::Serialize;
use std::path::Path;

/// Every optional part of mecha, parts after the feature they belong to.
///
/// A closed enum written to `--json` and, later, `/api/features`: a wire
/// format, so an id never changes once shipped. Order is the order of
/// [`Feature::ALL`], which is the order a person reads them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Web,
    Incognito,
    Slack,
    Mail,
    Docs,
    Graph,
    Tasks,
    Search,
    Documents,
    Ocr,
    Layout,
    Image,
    Library,
    /// `video_generate`: clips with sound from a local video server.
    Video,
    Personas,
    Voice,
    Dictate,
    Calls,
    Cloning,
    Frontdoor,
    Publishing,
    Messages,
    /// `system_read`: the machine's state for the assistant
    /// (`SYSTEM-STATE-DESIGN.md` §5, R3).
    System,
}

impl Feature {
    pub const ALL: &'static [Feature] = &[
        Feature::Web,
        Feature::Incognito,
        Feature::Slack,
        Feature::Mail,
        Feature::Docs,
        Feature::Graph,
        Feature::Tasks,
        Feature::Search,
        Feature::Documents,
        Feature::Ocr,
        Feature::Layout,
        Feature::Image,
        Feature::Library,
        Feature::Video,
        Feature::Personas,
        Feature::Voice,
        Feature::Dictate,
        Feature::Calls,
        Feature::Cloning,
        Feature::Frontdoor,
        Feature::Publishing,
        Feature::Messages,
        Feature::System,
    ];

    /// The name on the command line and on the wire.
    pub fn id(self) -> &'static str {
        match self {
            Feature::Web => "web",
            Feature::Slack => "slack",
            Feature::Mail => "mail",
            Feature::Docs => "docs",
            Feature::Graph => "graph",
            Feature::Tasks => "tasks",
            Feature::Search => "search",
            Feature::Documents => "documents",
            Feature::Ocr => "ocr",
            Feature::Layout => "layout",
            Feature::Image => "image",
            Feature::Library => "library",
            Feature::Video => "video",
            Feature::Personas => "personas",
            Feature::Voice => "voice",
            Feature::Dictate => "dictate",
            Feature::Calls => "calls",
            Feature::Cloning => "cloning",
            Feature::Incognito => "incognito",
            Feature::Frontdoor => "frontdoor",
            Feature::Publishing => "publishing",
            Feature::Messages => "messages",
            Feature::System => "system",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Feature::Web => "The web app",
            Feature::Slack => "Slack remote control",
            Feature::Mail => "Mail and calendar",
            Feature::Docs => "Google Docs, Sheets and Slides",
            Feature::Graph => "Knowledge graph",
            Feature::Tasks => "Task board",
            Feature::Search => "Web search",
            Feature::Documents => "PDF extraction",
            Feature::Ocr => "OCR",
            Feature::Layout => "Page layout",
            Feature::Image => "Image generation",
            Feature::Library => "Character and style library",
            Feature::Video => "Video generation",
            Feature::Personas => "Personas",
            Feature::Voice => "Voice",
            Feature::Dictate => "Dictation",
            Feature::Calls => "Voice calls",
            Feature::Cloning => "Voice cloning",
            Feature::Incognito => "Incognito chat",
            Feature::Frontdoor => "Front door (inbound requests)",
            Feature::Publishing => "Publishing tools",
            Feature::Messages => "Messages between sessions",
            Feature::System => "Machine state for the assistant",
        }
    }

    pub fn parse(id: &str) -> Option<Feature> {
        Feature::ALL.iter().copied().find(|f| f.id() == id)
    }

    /// The feature this one is a part of, if any. A part is shown under its
    /// parent and is never on while the parent is off — so a half-configured
    /// feature has a row that says which half.
    pub fn part_of(self) -> Option<Feature> {
        match self {
            Feature::Tasks => Some(Feature::Graph),
            Feature::Ocr => Some(Feature::Documents),
            Feature::Layout => Some(Feature::Ocr),
            Feature::Library => Some(Feature::Image),
            Feature::Dictate | Feature::Calls | Feature::Cloning => Some(Feature::Voice),
            Feature::Publishing => Some(Feature::Frontdoor),
            // The owner, 2026-10-14: no separate approval — a part of the web
            // app, on wherever the chat model is local.
            Feature::Incognito => Some(Feature::Web),
            _ => None,
        }
    }

    /// What must be on for this to be on, beyond [`Feature::part_of`].
    pub fn requires(self) -> &'static [Feature] {
        match self {
            // Not the front door on mail: requests arrive by `factory-publish
            // drain` and publishing needs no mailbox; only booking settlement
            // reads the mail ledger, and that side fails closed on its own
            // (found on review of #428).
            // Voice itself is not the web app's: `mecha voice-serve` is a
            // standalone loopback surface that reads nothing from `[web]`.
            // Its parts that live in `mecha serve` — the browser's dictation,
            // voice calls, cloning — are (found on review of #428).
            Feature::Dictate | Feature::Calls | Feature::Cloning => &[Feature::Web],
            _ => &[],
        }
    }

    /// Whether this feature has its own bool in `[features]`. A part rides
    /// its parent and has none, so `enable` refuses a part id by name.
    pub fn has_switch(self) -> bool {
        self.part_of().is_none()
    }

    /// The feature whose bool decides this one: itself, or for a part the
    /// top of its `part_of` chain (`layout` → `ocr` → `documents`).
    pub fn switch_owner(self) -> Feature {
        std::iter::successors(Some(self), |g| g.part_of())
            .last()
            .unwrap_or(self)
    }

    /// Whether switching this off turns something off *today* — its tools
    /// are unregistered, its server is not started, its web routes answer
    /// `feature_off` and its verbs refuse ([`refusal`]). Every feature is
    /// guarded since step 3b; 3a guarded the first seven, and 3b Slack,
    /// personas, voice, incognito, the front door (its queue and its
    /// publishing server together) and messages, each whole. The upgrade
    /// notice calls off only what this says is off, so a feature added with
    /// its guards still to come answers `false` until they land (found on
    /// review of #445). Exhaustive: a new feature decides.
    pub fn gated(self) -> bool {
        match self {
            Feature::Web
            | Feature::Mail
            | Feature::Docs
            | Feature::Graph
            | Feature::Search
            | Feature::Documents
            | Feature::Image
            | Feature::Video
            | Feature::Slack
            | Feature::Personas
            | Feature::Voice
            | Feature::Frontdoor
            | Feature::Messages
            | Feature::System => true,
            Feature::Incognito
            | Feature::Tasks
            | Feature::Ocr
            | Feature::Layout
            | Feature::Library
            | Feature::Dictate
            | Feature::Calls
            | Feature::Cloning
            | Feature::Publishing => self.switch_owner().gated(),
        }
    }

    /// Whether an experiment environment's `config.toml` may switch this on.
    ///
    /// **An environment may switch a feature on only if it could configure
    /// it**, judged by where the feature's settings and credentials live, not
    /// by who supplies them (FEATURES-DESIGN.md §5.1). An environment arrives
    /// with a checkout; `trial_env::config_at` refuses any `[features]` key
    /// set `true` for which this is `false`. Exhaustive, so a new variant does
    /// not compile until it decides — the list is a function, not a fourth
    /// hand-kept list.
    pub fn switchable_from_environment(self) -> bool {
        match self {
            // A binary on PATH, and the trial's own `requests` store under
            // `$MECHA_HOME`.
            Feature::Frontdoor => true,
            // Operator-only tables: the surfaces, the image server, the OCR
            // server and parser sandbox, the mailbox.
            Feature::Web
            | Feature::Slack
            | Feature::Image
            | Feature::Video
            | Feature::Documents
            | Feature::Messages
            | Feature::Voice
            | Feature::Personas
            // The operator's credentials: the mail crate's stores live under
            // the real home whatever `$MECHA_HOME` says.
            | Feature::Mail
            | Feature::Docs
            // The operator's backends and keys (`MACHINE_TABLES`).
            | Feature::Search
            // The operator's machine, read through its own series.
            | Feature::System
            // A manifest's `live_servers` can carry the operator's graph in,
            // and the row cannot tell that entry from a declared one;
            // `config_at` defaults this bool instead (FEATURES-DESIGN §5.1).
            | Feature::Graph => false,
            // Parts have no switch to set.
            Feature::Incognito
            | Feature::Tasks
            | Feature::Ocr
            | Feature::Layout
            | Feature::Library
            | Feature::Dictate
            | Feature::Calls
            | Feature::Cloning
            | Feature::Publishing => false,
        }
    }

    fn needs(self) -> impl Iterator<Item = Feature> {
        self.part_of()
            .into_iter()
            .chain(self.requires().iter().copied())
    }
}

/// What a feature is doing, kept apart where a bool would merge them.
///
/// `Blocked` and `Off` both hide a feature; `Unready` and `Unknown` never
/// do, because hiding something the owner turned on reads as "not
/// configured" — a silently-degrading guard (design §4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum State {
    On {
        detail: String,
    },
    Off {
        reason: String,
        fix: Option<String>,
    },
    /// Configured, but something it needs is off.
    Blocked {
        on: Feature,
    },
    /// Switched on, and its settings are missing or a fact on disk says it
    /// cannot work yet.
    Unready {
        reason: String,
        fix: Option<String>,
    },
    /// A store or credential file could not be read. Never clean.
    Unknown {
        reason: String,
    },
}

impl State {
    /// Whether whatever depends on this may be on. `Unknown` does not block:
    /// it is shown with a warning, and blocking on it would hide the
    /// dependents of something that may well be working.
    fn satisfies(&self) -> bool {
        !matches!(self, State::Off { .. } | State::Blocked { .. })
    }

    /// Whether a surface shows this feature — the web app's nav, cards and
    /// buttons (FEATURES-DESIGN.md §4.2 item 3). The same test as
    /// `satisfies`, and deliberately so: what cannot carry its dependents
    /// is what is hidden, and nothing the owner switched on is.
    pub fn shown(&self) -> bool {
        self.satisfies()
    }

    pub fn word(&self) -> &'static str {
        match self {
            State::On { .. } => "on",
            State::Off { .. } => "off",
            State::Blocked { .. } => "blocked",
            State::Unready { .. } => "unready",
            State::Unknown { .. } => "unknown",
        }
    }
}

/// The owner's answer in `[features]` for one feature with a switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Switch {
    On,
    Off,
    /// No key: a question not yet answered, which is what an install that
    /// predates `[features]` has for every feature.
    Absent,
}

/// `f`'s switch, or `None` for a part, which has none.
///
/// `messages` is read from `[messages] enabled`, where `apply` puts a
/// `[features] messages` answer (`config::FeaturesConfig`), and reads like
/// every other switch: unset is `Absent`, `false` is `Off`.
pub fn switch(cfg: &Config, f: Feature) -> Option<Switch> {
    if !f.has_switch() {
        return None;
    }
    let answer = if f == Feature::Messages {
        cfg.messages.enabled
    } else {
        cfg.features.get(f.id())
    };
    Some(match answer {
        Some(true) => Switch::On,
        Some(false) => Switch::Off,
        None => Switch::Absent,
    })
}

/// The top-level features `cfg` has switched on, in `Feature::ALL` order:
/// what a run records (`RunConfig::features_on`) and what an experiment row's
/// condition hash carries, from this one definition so the two cannot drift.
/// Parts are left out — they have no switch, and are recomputable from the
/// config.
pub fn switches_on(cfg: &Config) -> Vec<Feature> {
    Feature::ALL
        .iter()
        .copied()
        .filter(|f| f.has_switch() && switched_on(cfg, *f))
        .collect()
}

/// Answer `f`'s switch in memory, where [`switch`] reads it: the
/// `[features]` map, or `[messages] enabled` for `messages`. A part has no
/// switch, so nothing is written for one.
pub fn set_switch(cfg: &mut Config, f: Feature, on: bool) {
    if !f.has_switch() {
        return;
    }
    if f == Feature::Messages {
        cfg.messages.enabled = Some(on);
    } else {
        cfg.features.0.insert(f.id().to_string(), on);
    }
}

/// Whether `f` is switched on — its own bool, or for a part its parent's.
///
/// What tool registration and server connection ask (FEATURES-DESIGN.md §4.2
/// item 6): **the switch, never the readout**. Registration builds from the
/// session's settings as it always has, and a switched-on feature whose
/// settings are missing or whose server is down still registers what it can,
/// so the tool list — the front of the cached prefix — does not move with a
/// server's uptime. `cfg` may be a project-layered config: `[features]` is
/// stripped from project layers, so its switches are the global file's.
pub fn switched_on(cfg: &Config, f: Feature) -> bool {
    // Its own bool (a part has none), and every need's — a parent and each
    // `requires` edge — so it answers as `state` does for the switch half,
    // config-only, and cannot register a tool `mecha features` shows
    // `Blocked` (found on review of #445).
    let own = match switch(cfg, f) {
        Some(s) => s == Switch::On,
        None => true,
    };
    own && f.needs().all(|n| switched_on(cfg, n))
}

/// The command that switches `f` on: its owner, and every `requires` edge of
/// the owner that is not on, first — the one spelling every surface prints,
/// so none prints a command `plan_enable` refuses (#443, #445).
pub fn enable_command(cfg: &Config, f: Feature) -> String {
    // Every switch `f` hangs on — its owner's and, through `needs`, each
    // parent's and requirement's owner — that is not on, in `Feature::ALL`
    // order, which lists what a feature needs before it. `dictate` needs
    // `voice` (its parent) and `web` (its requirement); the owner's own
    // switch is always named, so the command is never empty.
    // Visited-guarded, so it terminates by construction rather than because
    // the graph happens to be a shallow DAG (review of #445).
    fn collect(f: Feature, seen: &mut Vec<Feature>, into: &mut Vec<Feature>) {
        if seen.contains(&f) {
            return;
        }
        seen.push(f);
        let owner = f.switch_owner();
        if !into.contains(&owner) {
            into.push(owner);
        }
        for n in f.needs().chain(owner.needs()) {
            collect(n, seen, into);
        }
    }
    let mut hung = Vec::new();
    collect(f, &mut Vec::new(), &mut hung);
    let owner = f.switch_owner();
    let ids: Vec<&str> = Feature::ALL
        .iter()
        .filter(|g| hung.contains(g))
        .filter(|g| **g == owner || switch(cfg, **g) != Some(Switch::On))
        .map(|g| g.id())
        .collect();
    format!("mecha features enable {}", ids.join(" "))
}

/// Why a surface that belongs to `f` refuses: `f` is switched off today
/// ([`Feature::gated`]) and its row is hidden ([`State::shown`]). `None` lets
/// it through — on, unready (shown with its fix, and the handler says what is
/// missing), unknown, or a feature whose guard has not landed yet.
///
/// The one predicate behind every route's `feature_off` and every CLI verb's
/// refusal (FEATURES-DESIGN.md §4.2 items 4–5), so the page, the API and the
/// terminal say the same thing the same way.
pub fn refusal(facts: &Facts, f: Feature) -> Option<Refusal> {
    if !f.gated() {
        return None;
    }
    let st = state(facts, f);
    if st.shown() {
        return None;
    }
    // Only off and blocked hide today; a state `shown` learns to hide later
    // refuses with its own reason rather than panicking a start or a verb
    // (review of #451).
    let why = match &st {
        State::Blocked { on } => format!("needs {}", on.label().to_lowercase()),
        State::Off { reason, .. } | State::Unready { reason, .. } | State::Unknown { reason } => {
            reason.clone()
        }
        State::On { detail } => detail.clone(),
    };
    Some(Refusal {
        feature: f,
        why,
        fix: fix(facts, f).unwrap_or_else(|| enable_command(&facts.config, f)),
    })
}

/// A surface's refusal: which feature, why, and what brings it on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Refusal {
    pub feature: Feature,
    pub why: String,
    pub fix: String,
}

impl Refusal {
    /// The one sentence a verb prints: *"Image generation is off (not
    /// enabled in [features]) — `mecha features enable image`"*.
    pub fn sentence(&self) -> String {
        format!(
            "{} is off ({}) — `{}`",
            self.feature.label(),
            self.why,
            self.fix
        )
    }
}

/// Why `server` must not be started: `Some` when it belongs to a feature whose
/// switch is not on. Every door that connects a server itself rather than
/// through `prepare_tools` asks this first — `distill`, `gossip`, `vet` and
/// `corroborate` find the graph server by name and spawn it directly, and
/// with `graph = false` they went on writing the owner's graph while
/// `mecha tasks`, gated, had no board (found on review of #445).
pub fn server_refusal(cfg: &Config, server: &McpServerConfig) -> Option<String> {
    let f = server_feature(server)?;
    (!switched_on(cfg, f)).then(|| {
        format!(
            "[[mcp]] `{}` belongs to `{}`, which is not switched on in [features] — `{}`",
            server.name,
            f.switch_owner().id(),
            enable_command(cfg, f)
        )
    })
}

/// The feature an `[[mcp]]` server belongs to, by the program its `command`
/// runs — the same match the rows use (`mcp_entry`). `None` for a server that
/// belongs to no feature: it connects as it always has.
pub fn server_feature(server: &McpServerConfig) -> Option<Feature> {
    let program = Path::new(&server.command).file_name()?.to_str()?;
    match program {
        "mecha-mail" => Some(Feature::Mail),
        "mecha-docs" => Some(Feature::Docs),
        "mecha-graph-mcp" => Some(Feature::Graph),
        // The front door's other half, gated with its queue in step 3b — never
        // one half first, or the notice called the front door "still working"
        // with half of it gone (found on review of #445).
        "factory-publish" => Some(Feature::Publishing),
        _ => None,
    }
}

/// Switches this binary once had and folded into another: written by an
/// older `enable`, read by nothing, and never reported as unknown — the
/// owner did nothing wrong. `incognito` became part of `web` (the owner,
/// 2026-10-14).
pub const RETIRED_SWITCHES: &[&str] = &["incognito"];

/// Keys in `[features]` this build does not know — a newer build's feature,
/// or a typo. Ignored, never a load failure (see `config::FeaturesConfig`),
/// and reported so a typo still shows as the feature it meant being off.
pub fn unknown_switches(cfg: &Config) -> Vec<String> {
    cfg.features
        .0
        .keys()
        .filter(|k| !RETIRED_SWITCHES.contains(&k.as_str()))
        .filter(|k| Feature::parse(k).is_none_or(|f| !f.has_switch()))
        .cloned()
        .collect()
}

/// What the impure half read from the disk, so [`state`] stays a function.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub has_mail_binary: bool,
    pub has_docs_binary: bool,
    pub has_graph_binary: bool,
    pub has_factory_binary: bool,
    /// `None` where the store could not be read.
    pub mail_accounts: Option<usize>,
    pub docs_accounts: Option<usize>,
    pub slack_linked: Option<bool>,
    /// Whether the persona store holds at least one persona (a directory
    /// with a `persona.toml`); `None` where it could not be read.
    pub personas_stored: Option<bool>,
    /// Whether a voice unit file is installed for this user
    /// (`mecha-voice-serve.service` or `mecha-voice-worker.service`, or the
    /// worker's launchd agent on a Mac). Installed, not running: a
    /// socket-activated unit is idle until asked.
    pub voice_unit_installed: bool,
    /// The **global** configuration — the only one any row reads.
    ///
    /// Carried here, rather than taken beside `Facts`, so the rule is the
    /// type's and not a doc comment's: [`state`] has no `Config` parameter a
    /// caller could hand a project-layered value to. A project's `mecha.toml`
    /// keeps its `[[mcp]]`, `[[search]]`, `[tools]` and `default_provider`
    /// through `merge_file`, and every one of them decides a row — so a
    /// layered config would let a cloned repository say what the owner's
    /// install has on (found on review of #427 and #428).
    pub config: Config,
}

impl Facts {
    /// Read everything [`state`] needs from `home`, `PATH` and the global
    /// configuration. No network. Each account store is found by its owner's
    /// rule (`onboarding::mail_store_dir`), never `home.join(..)`.
    pub fn read(home: &Path, global: &Config) -> Facts {
        use crate::onboarding::{
            count_accounts, docs_store_dir, find_program, mail_store_dir, slack_linked,
        };
        let current_on_path = |program: &str| {
            find_program(program).is_some_and(|p| !crate::sidecar::stale_copy(program, &p))
        };
        Facts {
            // On the PATH — unless a copy from another build, which is not
            // the feature's server (§15; review of #699) — or mecha's own
            // build in its tree.
            has_mail_binary: current_on_path("mecha-mail") || crate::mail_mcp::installed(home),
            has_docs_binary: current_on_path("mecha-docs") || crate::mail_mcp::installed(home),
            has_graph_binary: current_on_path("mecha-graph-mcp")
                || crate::graph_mcp::installed(home),
            has_factory_binary: find_program("factory-publish").is_some(),
            mail_accounts: mail_store_dir().and_then(|d| count_accounts(&d)),
            docs_accounts: docs_store_dir().and_then(|d| count_accounts(&d)),
            slack_linked: slack_linked(home),
            personas_stored: personas_stored(&home.join("personas")),
            voice_unit_installed: dirs::config_dir().is_some_and(|d| {
                let units = d.join("systemd/user");
                ["mecha-voice-serve.service", "mecha-voice-worker.service"]
                    .iter()
                    .any(|u| units.join(u).is_file())
            }) || dirs::home_dir().is_some_and(|h| {
                crate::launchd::agents_dir(&h)
                    .join(crate::launchd::plist_name("mecha-voice-worker"))
                    .is_file()
            }),
            config: global.clone(),
        }
    }
}

/// Does `dir` hold a persona — a directory with a `persona.toml`, which is
/// what `persona::Store::load` counts? `None` when it could not be read.
fn personas_stored(dir: &Path) -> Option<bool> {
    match std::fs::read_dir(dir) {
        Ok(entries) => Some(entries.flatten().any(|e| {
            let p = e.path();
            !e.file_name().to_string_lossy().starts_with('.') && p.join("persona.toml").is_file()
        })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

/// One row of `mecha features`.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub id: Feature,
    pub label: &'static str,
    pub part_of: Option<Feature>,
    pub requires: &'static [Feature],
    #[serde(flatten)]
    pub state: State,
    /// The owner's answer in `[features]`; `None` for a part.
    pub switch: Option<Switch>,
    /// Switch absent, but this install has it set up and it would work —
    /// the upgrade notice's case, shown as its own row rather than a bare
    /// `off` (FEATURES-DESIGN.md §4.2).
    pub in_use: bool,
    /// Whether a surface shows it ([`State::shown`]), so the web app reads
    /// the rule rather than restating it.
    pub shown: bool,
    /// What to run to bring it on ([`fix`]), a `Blocked` row's included —
    /// the "next command" of FEATURES-DESIGN.md §4.2 item 1. Not `fix`: the
    /// flattened state already carries one for `Off` and `Unready`.
    pub next: Option<String>,
    /// Whether switching it off refuses today ([`Feature::gated`]): its
    /// routes answer `feature_off` and its verbs refuse. The web keys what a
    /// refusal forces — a flat card, a pane sent home — on this, not on
    /// `shown`, so a feature hidden before its guard lands keeps its door
    /// (review of #451: the front door's queue had none).
    pub gated: bool,
}

/// Every feature, in [`Feature::ALL`] order.
pub fn all(facts: &Facts) -> Vec<Row> {
    Feature::ALL
        .iter()
        .map(|&f| {
            let state = state(facts, f);
            Row {
                id: f,
                label: f.label(),
                part_of: f.part_of(),
                requires: f.requires(),
                shown: state.shown(),
                next: fix(facts, f),
                gated: f.gated(),
                state,
                switch: switch(&facts.config, f),
                in_use: announcement(facts, f).is_some(),
            }
        })
        .collect()
}

/// Is `f` on? The only place that question is answered.
///
/// Every row reads [`Facts::config`], the global configuration, and
/// nothing else configures it.
///
/// The owner's switch is read first: absent or `false` is `Off`, whatever the
/// settings or the dependencies say. With it on (or for a part, which has
/// none), a feature whose parent or requirement is off is `Blocked` on the
/// first one found: configured-but-unreachable is different from off, and
/// this says which dependency to fix first.
pub fn state(facts: &Facts, f: Feature) -> State {
    // The fix names every dependency that is not on either, because `enable`
    // refuses a feature alone when its requirement is off — the row must
    // not print a command the same binary rejects (found on review of #443).
    let fix = || enable_command(&facts.config, f);
    match switch(&facts.config, f) {
        Some(Switch::Off) => return off("turned off in [features]", fix()),
        Some(Switch::Absent) => return off("not enabled in [features]", fix()),
        Some(Switch::On) | None => {}
    }
    // The first need that is switched on but not ready. It blocks nothing —
    // an unready feature is shown — but nothing standing on it is `On`
    // either: `ocr` under a `document_read` that `[tools]` withholds, the
    // library under an `image = true` with no `[image]` table, the board
    // under a graph with no `[[mcp]]` entry — each has no tool to reach
    // (found on review of #449).
    let mut waits_on = None;
    for need in f.needs() {
        match state(facts, need) {
            s if !s.satisfies() => return State::Blocked { on: need },
            State::Unready { reason, fix } if waits_on.is_none() => {
                waits_on = Some((need, reason, fix));
            }
            _ => {}
        }
    }
    let own = match own_state(facts, f) {
        // Past the switch, a feature with one was switched on, so its own
        // `off` means "said yes, not set up yet": `Unready`, shown with the
        // fix, never hidden (FEATURES-DESIGN.md §5). A part's `off` stays —
        // `[documents] ocr = false` is the owner's own no. Harmless while
        // nothing hid a row; the web app hiding `Off` is what made it matter.
        State::Off { reason, fix } if f.has_switch() => State::Unready { reason, fix },
        own => own,
    };
    match (own, waits_on) {
        (State::On { .. }, Some((need, reason, fix))) => State::Unready {
            reason: format!("`{}` is not ready: {reason}", need.id()),
            fix,
        },
        (own, _) => own,
    }
}

/// What to run to bring `f` on. A `Blocked` row hanging on a switch that is
/// not on answers [`enable_command`], which names every such switch at once —
/// `dictate` with neither `voice` nor `web` on is one command, not two
/// attempts; one blocked only by a setting follows the need to its fix —
/// `layout` blocked on `ocr = false` answers the setting, never a switch
/// that is already on. `None` for `On` and `Unknown`, which have nothing to
/// run.
pub fn fix(facts: &Facts, f: Feature) -> Option<String> {
    // Visited-guarded, as `enable_command` is: terminates by construction,
    // not because `Blocked { on }` happens to point down a shallow DAG.
    let mut seen = Vec::new();
    let mut at = f;
    loop {
        if seen.contains(&at) {
            return None;
        }
        seen.push(at);
        match state(facts, at) {
            State::Off { fix, .. } | State::Unready { fix, .. } => return fix,
            State::Blocked { .. } if !switched_on(&facts.config, at) => {
                return Some(enable_command(&facts.config, at));
            }
            State::Blocked { on } => at = on,
            State::On { .. } | State::Unknown { .. } => return None,
        }
    }
}

/// Why a feature is announced on an install that has not answered its
/// switch: it is set up here and would work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Announcement {
    pub id: Feature,
    /// Present when it would be `Unready`: what is still missing.
    pub caveat: Option<String>,
    /// The command that answers it, naming any dependency whose switch is
    /// also absent first — `enable` alone would land in `Blocked`.
    pub fix: String,
}

impl Announcement {
    /// The one line printed on a start.
    pub fn line(&self) -> String {
        match &self.caveat {
            Some(why) => format!(
                "`{}`: configured but not enabled ({why}) — `{}`",
                self.id.id(),
                self.fix
            ),
            None => format!(
                "`{}`: configured but not enabled — `{}`",
                self.id.id(),
                self.fix
            ),
        }
    }
}

/// Evidence that the owner set `f` up on this install — not that it would
/// work, which `state` answers, but that there is something to lose if it
/// stays off. Exhaustive, so a new feature decides. Features with no
/// evidence (incognito, messages, parts) are never announced: they have
/// nothing an upgrade could take away unannounced.
fn evidence(facts: &Facts, f: Feature) -> bool {
    let cfg = &facts.config;
    match f {
        Feature::Web => cfg
            .web
            .owner_login
            .as_deref()
            .is_some_and(|l| !l.is_empty()),
        Feature::Slack => facts.slack_linked == Some(true),
        Feature::Mail => mcp_entry(facts, "mecha-mail").is_some_and(|m| !m.disabled),
        Feature::Docs => mcp_entry(facts, "mecha-docs").is_some_and(|m| !m.disabled),
        Feature::Graph => mcp_entry(facts, "mecha-graph-mcp").is_some_and(|m| !m.disabled),
        Feature::Search => cfg.search.iter().any(|b| !b.disabled),
        Feature::Documents => cfg.documents.is_some(),
        Feature::Image => cfg.image.is_some(),
        Feature::Video => cfg.video.is_some(),
        Feature::Personas => facts.personas_stored == Some(true),
        Feature::Voice => facts.voice_unit_installed,
        Feature::Frontdoor => facts.has_factory_binary,
        Feature::Messages | Feature::System => false,
        Feature::Incognito
        | Feature::Tasks
        | Feature::Ocr
        | Feature::Layout
        | Feature::Library
        | Feature::Dictate
        | Feature::Calls
        | Feature::Cloning
        | Feature::Publishing => false,
    }
}

/// `Some` when `f`'s switch is absent, the owner set it up here, and it would
/// be usable with the unanswered switches treated as on.
///
/// The substitution covers **every absent** switch, not only `f`'s, because on
/// an install that predates `[features]` its dependencies are absent too. An
/// explicit `false` is an answer and is never substituted, so an owner who
/// wrote `web = false` is not told to enable what needs it. "Usable" is `On`
/// or `Unready` — the owner's ruling of 2026-09-30, with `Unready`'s reason
/// carried — and never `Unknown`, which would offer a switch off a store that
/// could not be read (FEATURES-DESIGN.md §4.2).
pub fn announcement(facts: &Facts, f: Feature) -> Option<Announcement> {
    if switch(&facts.config, f) != Some(Switch::Absent) || !evidence(facts, f) {
        return None;
    }
    let mut assumed = facts.clone();
    for &g in Feature::ALL {
        if switch(&facts.config, g) == Some(Switch::Absent) {
            set_switch(&mut assumed.config, g, true);
        }
    }
    let caveat = match state(&assumed, f) {
        State::On { .. } => None,
        State::Unready { reason, .. } => Some(reason),
        State::Off { .. } | State::Blocked { .. } | State::Unknown { .. } => return None,
    };
    let mut ids: Vec<&str> = f
        .requires()
        .iter()
        .filter(|d| switch(&facts.config, **d) == Some(Switch::Absent))
        .map(|d| d.id())
        .collect();
    ids.push(f.id());
    Some(Announcement {
        id: f,
        caveat,
        fix: format!("mecha features enable {}", ids.join(" ")),
    })
}

/// Every announcement, in `Feature::ALL` order — the upgrade notice's lines
/// and `mecha setup`'s offers, from one function so the two cannot disagree.
pub fn announcements(facts: &Facts) -> Vec<Announcement> {
    Feature::ALL
        .iter()
        .filter_map(|&f| announcement(facts, f))
        .collect()
}

/// Check a request to switch features on, before anything is written.
///
/// A part id is refused by name, pointing at its parent and the setting that
/// turns it on. A feature whose `requires` is not already on and not in the
/// same request is refused with the chained command, as Claude Code refuses a
/// disable that would break another plugin, rather than land in `Blocked`.
/// (No feature with a switch has a `requires` edge since F15; the branch
/// waits for the next one.)
pub fn plan_enable(cfg: &Config, ids: &[String]) -> Result<Vec<Feature>, String> {
    let mut wanted = Vec::new();
    for id in ids {
        let f = Feature::parse(id)
            .ok_or_else(|| format!("`{id}` is not a feature — `mecha features` lists them"))?;
        // Incognito had a switch until the owner folded it into the web app
        // (2026-10-14); it has no setting to turn on, only the local model.
        if f == Feature::Incognito {
            return Err(
                "`incognito` is part of `web` and has no switch of its own — enable `web`; an \
                 incognito chat is offered wherever the chat model runs on this machine"
                    .into(),
            );
        }
        if let Some(parent) = f.part_of() {
            return Err(format!(
                "`{id}` is part of `{}` and has no switch of its own — enable `{}`, and \
                 turn `{id}` on in its settings",
                parent.id(),
                parent.id()
            ));
        }
        if !wanted.contains(&f) {
            wanted.push(f);
        }
    }
    let missing: Vec<Feature> = wanted
        .iter()
        .flat_map(|f| f.requires().iter().copied())
        .filter(|d| !wanted.contains(d) && switch(cfg, *d) != Some(Switch::On))
        .collect();
    if !missing.is_empty() {
        let mut chain: Vec<&str> = missing.iter().map(|f| f.id()).collect();
        chain.dedup();
        chain.extend(wanted.iter().map(|f| f.id()));
        return Err(format!(
            "{} needs {} on as well — `mecha features enable {}`",
            wanted
                .iter()
                .map(|f| format!("`{}`", f.id()))
                .collect::<Vec<_>>()
                .join(", "),
            missing
                .iter()
                .map(|f| format!("`{}`", f.id()))
                .collect::<Vec<_>>()
                .join(", "),
            chain.join(" ")
        ));
    }
    Ok(wanted)
}

/// What a feature that is an MCP server needs written beside its switch: the
/// program its `[[mcp]]` entry runs, whether its tools keep the server's
/// prefix, and — mail only — the calls staged in the outbox rather than run.
pub struct McpWiring {
    pub name: &'static str,
    pub program: &'static str,
    /// What installs the program — the one answer `server_state` gives too.
    pub install: &'static str,
    pub prefix_tools: bool,
    pub outbox: &'static [&'static str],
    /// Environment a new entry carries: what the server must be told about
    /// mecha's own servers, which its defaults do not know.
    pub env: &'static [(&'static str, &'static str)],
}

/// The graph server's embedder: mecha's on-demand embeddings server and the
/// model it serves. Without them the pinned server falls back to 768
/// dimensions and refuses the 1024 harrier returns — every `kg_search` on a
/// fresh install failed so (the owner's fresh Mac, 2026-10-15).
/// `graph_embedder_is_mecha_s` holds each value to its source.
pub const GRAPH_EMBED_ENV: &[(&str, &str)] = &[
    ("MECHA_GRAPH_EMBED_URL", "http://127.0.0.1:8081"),
    ("MECHA_GRAPH_EMBED_MODEL", crate::recommend::EMBED_ALIAS),
    ("MECHA_GRAPH_EMBED_DIMS", crate::recommend::EMBED_DIMS),
];

impl McpWiring {
    /// The staged calls as an entry called `name` registers them: `outbox` is
    /// spelled for the entry this wiring writes (`<name>__<tool>`), and an
    /// owner's entry may be called otherwise or unprefixed (review of #669,
    /// pass 4).
    pub fn routes_as(&self, name: &str, prefix: bool) -> Vec<String> {
        let own = format!("{}__", self.name);
        self.outbox
            .iter()
            .map(|t| {
                let tool = t.strip_prefix(own.as_str()).unwrap_or(t);
                match prefix {
                    true => format!("{name}__{tool}"),
                    false => tool.to_string(),
                }
            })
            .collect()
    }
}

/// The wiring `features enable` writes for a feature, where it has one: the
/// shape the docs have always given (`mail.md`, `documents.md`, the graph
/// page), so a yes in `mecha setup` ends at a sign-in page rather than at "add
/// an [[mcp]] entry" (the owner, 2026-10-12: sends staged).
pub fn mcp_wiring(f: Feature) -> Option<McpWiring> {
    Some(match f {
        Feature::Mail => McpWiring {
            name: "mail",
            program: "mecha-mail",
            // `enable` builds the matching servers (§15).
            install: "mecha features enable mail",
            prefix_tools: true,
            env: &[],
            outbox: &[
                "mail__mail_send",
                "mail__mail_reply",
                "mail__calendar_create_event",
                "mail__calendar_update_event",
                "mail__calendar_delete_event",
            ],
        },
        Feature::Docs => McpWiring {
            name: "docs",
            program: "mecha-docs",
            install: "mecha features enable docs",
            prefix_tools: true,
            env: &[],
            // Writing into a document a third party can read is a send
            // (`documents.md`): the three verbs that write into an existing
            // file stage for review (review of #669).
            outbox: &[
                "docs__docs_append",
                "docs__docs_replace",
                "docs__sheets_write",
            ],
        },
        // `enable graph` builds the pinned server when none is on the PATH
        // (§13, the owner's ruling of 2026-10-13).
        Feature::Graph => McpWiring {
            name: "graph",
            program: "mecha-graph-mcp",
            install: "mecha features enable graph",
            prefix_tools: false,
            outbox: &[],
            env: GRAPH_EMBED_ENV,
        },
        _ => return None,
    })
}

/// What installs a server feature's program: one answer for `server_state`'s
/// next step and `features enable`'s (review of #669).
fn installs(f: Feature) -> &'static str {
    mcp_wiring(f).map(|w| w.install).unwrap_or_default()
}

/// What [`write_mcp_entry`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpWrite {
    /// Whether the `[[mcp]]` entry was written; false when one already runs
    /// the program or carries the feature's name (enabled or not), which is
    /// left as it is.
    pub wrote_entry: bool,
    /// The feature's staged calls as the registry will name them: off the
    /// entry that serves it — the owner's own `name` and `prefix_tools` where
    /// one exists, never the docs' spelling — so the routing written or
    /// advised is routing that routes (review of #669, pass 4).
    pub routes: Vec<String>,
    /// Whether an `[outbox]` table was written, where there was none.
    pub wrote_outbox: bool,
    /// The routing the owner's own `[outbox]` lacks — never edited, so the
    /// caller says the lines to add. Checked whether or not the entry was
    /// written: a hand-wired install is the one most likely to lack it
    /// (review of #669).
    pub outbox_missing: Vec<String>,
    /// Routes an operator rule names, which kept the `[outbox]` unwritten:
    /// for the caller to name with the rule.
    pub ruled: Vec<String>,
    /// Routes an operator rule names that the owner's `[outbox]` lacks: left
    /// out of `outbox_missing`, whose advice would otherwise have the owner
    /// route a call their rule judges — every start refuses an `allow` or
    /// `prompt` beside a route, and a `forbid` becomes a releasable draft
    /// (review of #669, pass 7).
    pub ruled_unrouted: Vec<String>,
    /// An existing entry that does not force `untrusted_input = true`: never
    /// edited, named by the caller (review of #669).
    pub untrusted_missing: bool,
    /// The variables of `McpWiring::env` an existing entry running the
    /// program does not set: never edited — a graph store built at another
    /// width would break under new ones — and said by the caller with the
    /// lines to add (review of #698).
    pub env_missing: Vec<(&'static str, &'static str)>,
    /// The command of an entry carrying the feature's name that runs some
    /// other program: left as it is, and no entry added beside it, so the
    /// caller names it (review of #669, pass 5).
    pub runs_other: Option<String>,
    /// The name of the entry running the program, where the only one is
    /// `disabled = true`: left as it is, but the feature reads off until it
    /// is enabled, so the caller says so beside whatever was staged (review
    /// of #669, pass 9).
    pub disabled: Option<String>,
    /// The command an existing entry ran — a copy of one of mecha's own
    /// servers from another build — now pointed at mecha's matching one
    /// (§15, the owner's ruling of 2026-10-15). Only `command` changes.
    pub repointed: Option<String>,
}

/// Write one feature's wiring — [`write_mcp_entries`] with a single item and
/// no operator rules. Tests only: "no rule exists" is no answer for a real
/// config, where a route beside an operator rule stops every start (review of
/// #669, pass 9).
#[cfg(test)]
fn write_mcp_entry(path: &Path, wiring: &McpWiring, command: &Path) -> anyhow::Result<McpWrite> {
    Ok(write_mcp_entries(path, &[(wiring, command)], &|_| false)?.remove(0))
}

/// Write each feature's `[[mcp]]` entry, running its command, into the global
/// config where no entry runs that program or carries its name, with
/// `untrusted_input` forced on — every one of these servers carries other
/// people's words — and their staged calls in one `[outbox]` when there is
/// none yet. One call decides the outbox for every feature in it, so `enable
/// mail docs` (and `mecha setup`'s yeses, wired together after the last)
/// routes both rather than the second finding the first's fresh table and
/// leaving it (review of #669). An entry or `[outbox]` the owner wrote is
/// never edited; what an existing entry lacks of the forced capability, and
/// what an existing `[outbox]` lacks of the routing, is returned for the
/// caller to name. In place through `toml_edit`, comments kept, written
/// beside and renamed, and not written at all when nothing changed.
///
/// A name matches as well as a command: an entry called `mail` that runs a
/// wrapper is the owner's mail, and a second `mail` beside it would shadow
/// one server's tools with the other's — for the graph's unprefixed names,
/// fail every start (review of #669, pass 4).
///
/// `ruled` says whether an operator rule names a tool. Where one names a
/// route and no `[outbox]` exists, none is written: an `allow`/`prompt` there
/// `setup::live_rules` refuses at every start, and a `forbid` the route would
/// turn into a releasable draft (review of #669). The ruled routes are
/// returned and nothing is reported missing — there is no table to add to,
/// and the rule is what stands in the way.
pub fn write_mcp_entries(
    path: &Path,
    items: &[(&McpWiring, &Path)],
    ruled: &dyn Fn(&str) -> bool,
) -> anyhow::Result<Vec<McpWrite>> {
    use anyhow::Context;
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("{} is not valid TOML; nothing was changed", path.display()))?;
    let outbox_existed = doc.contains_key("outbox");
    let mut results = Vec::new();
    let mut changed = false;
    for (wiring, command) in items {
        let runs = |t: &toml_edit::Table| {
            t.get("command")
                .and_then(|c| c.as_str())
                .and_then(|c| Path::new(c).file_name().map(|n| n == wiring.program))
                .unwrap_or(false)
        };
        let named =
            |t: &toml_edit::Table| t.get("name").and_then(|n| n.as_str()) == Some(wiring.name);
        // An entry running the program first; one only carrying the name
        // after, which is left as it is and said (below). Enabled before
        // disabled in each, as `mcp_entry` reads them: a stale disabled
        // duplicate must not name the routing, which would then match no
        // registered tool while the live entry's sends ran unstaged (review
        // of #669, pass 6).
        let live = |t: &toml_edit::Table| t.get("disabled").and_then(|d| d.as_bool()) != Some(true);
        let servers = doc.get("mcp").and_then(|m| m.as_array_of_tables());
        let existing = servers.and_then(|s| {
            s.iter()
                .find(|t| runs(t) && live(t))
                .or_else(|| s.iter().find(|t| runs(t)))
                .or_else(|| s.iter().find(|t| named(t) && live(t)))
                .or_else(|| s.iter().find(|t| named(t)))
                .cloned()
        });
        // The owner's entry of this name runs something else: no second one
        // beside it (one name, two servers, shadowed tools), but the feature
        // reads as served only by an entry running the program
        // (`server_state`), so the caller says so rather than leaving a
        // silent "nothing written" (review of #669, pass 5).
        // An entry running a stale copy of one of mecha's own servers is
        // pointed at the matching build, `command` alone (§15): the owner
        // ruled that mecha's own servers follow mecha.
        let repointed = existing
            .as_ref()
            .filter(|t| runs(t))
            .and_then(|t| t.get("command").and_then(|c| c.as_str()))
            .filter(|old| {
                Path::new(old) != *command
                    && crate::sidecar::stale_copy(wiring.program, Path::new(old))
            })
            .map(str::to_string);
        // Every entry running that copy, a disabled duplicate included: the
        // copy is the stale thing, whichever entry names it.
        if let Some(old) = &repointed {
            if let Some(servers) = doc.get_mut("mcp").and_then(|m| m.as_array_of_tables_mut()) {
                for t in servers.iter_mut() {
                    if t.get("command").and_then(|c| c.as_str()) == Some(old.as_str()) {
                        t.insert("command", toml_edit::value(command.display().to_string()));
                        changed = true;
                    }
                }
            }
        }
        let disabled = existing.as_ref().filter(|t| runs(t) && !live(t)).map(|t| {
            t.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string()
        });
        let runs_other = existing.as_ref().filter(|t| !runs(t)).map(|t| {
            t.get("command")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string()
        });
        let env_missing: Vec<(&'static str, &'static str)> = match &existing {
            Some(t) if runs(t) => wiring
                .env
                .iter()
                .copied()
                .filter(|(k, _)| t.get("env").and_then(|e| e.get(k)).is_none())
                .collect(),
            _ => Vec::new(),
        };
        // An entry the owner wrote that does not force the capability: named,
        // never edited. It is the stronger gap — a mail read declares only
        // `readOnlyHint`, so without it a mail body never arms the untrusted
        // leg of the interlock (review of #669).
        let untrusted_missing = existing.as_ref().is_some_and(|t| {
            t.get("capabilities")
                .and_then(|c| c.get("untrusted_input"))
                .and_then(|v| v.as_bool())
                != Some(true)
        });
        // The registry prefixes a tool with its entry's name unless the entry
        // says `prefix_tools = false` (`mcp.rs`).
        let (name, prefix) = match &existing {
            Some(t) => (
                t.get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or(wiring.name)
                    .to_string(),
                t.get("prefix_tools").and_then(|v| v.as_bool()) != Some(false),
            ),
            None => (wiring.name.to_string(), wiring.prefix_tools),
        };
        let routes = wiring.routes_as(&name, prefix);
        if existing.is_none() {
            let mut entry = toml_edit::Table::new();
            entry.insert("name", toml_edit::value(wiring.name));
            entry.insert("command", toml_edit::value(command.display().to_string()));
            if !wiring.prefix_tools {
                entry.insert("prefix_tools", toml_edit::value(false));
            }
            if !wiring.env.is_empty() {
                let mut env = toml_edit::Table::new();
                for (k, v) in wiring.env {
                    env.insert(k, toml_edit::value(*v));
                }
                entry.insert("env", toml_edit::Item::Table(env));
            }
            let mut caps = toml_edit::Table::new();
            caps.insert("untrusted_input", toml_edit::value(true));
            entry.insert("capabilities", toml_edit::Item::Table(caps));
            match doc.get_mut("mcp") {
                Some(item) => item
                    .as_array_of_tables_mut()
                    .context(
                        "[mcp] in the config is not an array of [[mcp]] tables; nothing was changed",
                    )?
                    .push(entry),
                None => {
                    let mut servers = toml_edit::ArrayOfTables::new();
                    servers.push(entry);
                    crate::config::append_tables(&mut doc, "mcp", servers);
                }
            }
            changed = true;
        }
        results.push(McpWrite {
            wrote_entry: existing.is_none(),
            ruled: if outbox_existed {
                Vec::new()
            } else {
                routes.iter().filter(|t| ruled(t)).cloned().collect()
            },
            routes,
            ruled_unrouted: Vec::new(),
            wrote_outbox: false,
            outbox_missing: Vec::new(),
            untrusted_missing,
            env_missing,
            runs_other,
            disabled,
            repointed,
        });
    }
    if !outbox_existed {
        // A rule on any route: nothing written and nothing missing (see
        // above).
        let wanted: Vec<&String> = results.iter().flat_map(|r| &r.routes).collect();
        if !wanted.is_empty() && results.iter().all(|r| r.ruled.is_empty()) {
            let mut tools = toml_edit::Array::new();
            for t in wanted {
                tools.push(t.as_str());
            }
            let mut table = toml_edit::Table::new();
            table.insert("tools", toml_edit::value(tools));
            crate::config::append_table(&mut doc, "outbox", table);
            changed = true;
            for r in results.iter_mut() {
                r.wrote_outbox = !r.routes.is_empty();
            }
        }
    } else {
        let routed: Vec<String> = doc
            .get("outbox")
            .and_then(|o| o.get("tools"))
            .and_then(|t| t.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        for r in results.iter_mut() {
            let (judged, missing): (Vec<String>, Vec<String>) = r
                .routes
                .iter()
                .filter(|t| !routed.contains(t))
                .cloned()
                .partition(|t| ruled(t));
            r.outbox_missing = missing;
            r.ruled_unrouted = judged;
        }
    }
    if !changed {
        return Ok(results);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("toml.mcp-tmp");
    std::fs::write(&tmp, doc.to_string()).with_context(|| format!("writing {}", tmp.display()))?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(results)
}

/// Write switches into the `[features]` table of the config file at `path`,
/// editing in place.
///
/// **In place, never a rewrite**: comments, ordering and every other table
/// survive, and so does a `[features]` key this build does not know — the
/// newer build's feature that a rewrite through `Config` would drop. Creates
/// the file, and the table, when absent: the state every install that
/// predates `[features]` is in, where `setup::apply` would bail. Written to a
/// temporary file beside it and renamed, so a crash leaves the old file.
pub fn write_switches(path: &Path, changes: &[(Feature, bool)]) -> anyhow::Result<()> {
    use anyhow::Context;
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("{} is not valid TOML; nothing was changed", path.display()))?;
    // A new `[features]` goes after the file's closing comments, as every
    // other new table does (`config::append_table`; review of #681).
    if !doc.contains_key("features") {
        crate::config::append_table(&mut doc, "features", toml_edit::Table::new());
    }
    let table = doc
        .get_mut("features")
        .context("the [features] table just written is not there")?
        .as_table_mut()
        .with_context(|| format!("`features` in {} is not a table", path.display()))?;
    for (f, on) in changes {
        anyhow::ensure!(f.has_switch(), "`{}` has no switch of its own", f.id());
        table.insert(f.id(), toml_edit::value(*on));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("toml.features-tmp");
    std::fs::write(&tmp, doc.to_string()).with_context(|| format!("writing {}", tmp.display()))?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

fn on(detail: impl Into<String>) -> State {
    State::On {
        detail: detail.into(),
    }
}

fn off(reason: impl Into<String>, fix: impl Into<String>) -> State {
    State::Off {
        reason: reason.into(),
        fix: Some(fix.into()),
    }
}

/// The `[[mcp]]` entry that runs `program`, found by the command's file
/// name — the tools themselves are only visible by connecting, which this
/// module never does. Enabled entries first, so a disabled duplicate does
/// not hide a working one.
fn mcp_entry<'a>(facts: &'a Facts, program: &str) -> Option<&'a McpServerConfig> {
    let servers = &facts.config.mcp;
    let runs = |m: &&McpServerConfig| {
        Path::new(&m.command)
            .file_name()
            .is_some_and(|n| n == program)
    };
    servers
        .iter()
        .filter(runs)
        .find(|m| !m.disabled)
        .or_else(|| servers.iter().find(runs))
}

/// The shared shape of the features that are an MCP server.
fn server_state(
    f: Feature,
    facts: &Facts,
    program: &str,
    installed: bool,
    install: &str,
    accounts: Option<Option<usize>>,
    authorise: &str,
) -> State {
    let Some(entry) = mcp_entry(facts, program) else {
        return if installed {
            // Where `enable` writes the entry, the fix is that runnable
            // command, so setup can finish the feature it was asked for —
            // a binary installed after the first yes otherwise leaves an
            // edit no step can run (review of #669, pass 8).
            let fix = match mcp_wiring(f) {
                Some(_) => format!("mecha features enable {}", f.id()),
                None => format!("add an [[mcp]] entry with command = \"{program}\""),
            };
            off(
                format!("`{program}` is installed, but no [[mcp]] entry runs it"),
                fix,
            )
        } else {
            off(format!("`{program}` is not installed"), install)
        };
    };
    if entry.disabled {
        return off(
            format!("the [[mcp]] entry `{}` is disabled", entry.name),
            format!("remove `disabled = true` from [[mcp]] `{}`", entry.name),
        );
    }
    match accounts {
        None => on(format!("[[mcp]] `{}`", entry.name)),
        Some(None) => State::Unknown {
            reason: format!("`{program}`'s credential store could not be read"),
        },
        Some(Some(0)) => State::Unready {
            reason: "no account is authorised".into(),
            fix: Some(authorise.into()),
        },
        Some(Some(n)) => on(format!("[[mcp]] `{}`, {n} account(s)", entry.name)),
    }
}

/// Off because `[tools]` withholds the tool — the one predicate
/// (`ToolsConfig::registers`) that `prepare_tools` asks too.
fn tools_off(tool: &str) -> State {
    off(
        format!("`{tool}` is turned off in [tools]"),
        format!("remove `{tool}` from [tools] disabled, or add it to [tools] enabled"),
    )
}

fn own_state(facts: &Facts, f: Feature) -> State {
    let cfg = &facts.config;
    match f {
        Feature::Web => match cfg.web.owner_login.as_deref() {
            Some(login) if !login.is_empty() => on(match &cfg.web.assets {
                Some(dir) => format!("serving {}", dir.display()),
                None => "the API only — no [web] assets".into(),
            }),
            _ => off(
                "no [web] owner_login — `mecha serve` needs one (or `--owner-login` each start)",
                "set [web] owner_login to your Tailscale login",
            ),
        },
        Feature::Slack => match facts.slack_linked {
            Some(true) => on("tokens stored"),
            Some(false) => off("no Slack tokens stored", "mecha slack auth"),
            None => State::Unknown {
                reason: "the Slack token store could not be read".into(),
            },
        },
        Feature::Mail => server_state(
            Feature::Mail,
            facts,
            "mecha-mail",
            facts.has_mail_binary,
            installs(Feature::Mail),
            Some(facts.mail_accounts),
            "mecha-mail auth",
        ),
        Feature::Docs => server_state(
            Feature::Docs,
            facts,
            "mecha-docs",
            facts.has_docs_binary,
            installs(Feature::Docs),
            Some(facts.docs_accounts),
            "mecha-docs auth",
        ),
        Feature::Graph => server_state(
            Feature::Graph,
            facts,
            "mecha-graph-mcp",
            facts.has_graph_binary,
            installs(Feature::Graph),
            None,
            "",
        ),
        Feature::Tasks => on("rides the graph's kg_task_* tools"),
        // Asked as `build_search_chain` asks it: a listed backend with no
        // key, no instance URL or an unknown kind is dropped there, and with
        // none left no `web_search` is registered (found on review of #428).
        Feature::Search => {
            let enabled: Vec<_> = cfg.search.iter().filter(|s| !s.disabled).collect();
            let live: Vec<&str> = enabled
                .iter()
                .filter(|s| s.problem().is_none())
                .map(|s| s.kind.as_str())
                .collect();
            let broken: Vec<String> = enabled
                .iter()
                .filter_map(|s| s.problem().map(|p| format!("{}: {p}", s.kind)))
                .collect();
            if !live.is_empty() {
                let mut detail = live.join(", ");
                if !broken.is_empty() {
                    detail.push_str(&format!(" (dropped — {})", broken.join("; ")));
                }
                on(detail)
            } else if !broken.is_empty() {
                State::Unready {
                    reason: broken.join("; "),
                    fix: None,
                }
            } else if cfg.search.is_empty() {
                off(
                    "no [[search]] backend",
                    "add a [[search]] backend (exa, tavily or searxng)",
                )
            } else {
                off(
                    "every [[search]] backend is disabled",
                    "remove `disabled = true` from a [[search]] backend",
                )
            }
        }
        // What `prepare_tools` checks before registering `document_read`:
        // `[tools]` first, then the config's own promises (a remote OCR
        // server, a docker confinement), which `Extractor::new` refuses.
        Feature::Documents => match &cfg.documents {
            None => off("no [documents] table", "add a [documents] table"),
            Some(_) if !cfg.tools.registers("document_read") => tools_off("document_read"),
            Some(d) => match d.validate() {
                Ok(()) => on("[documents]"),
                Err(e) => State::Unready {
                    reason: format!("{e:#}"),
                    fix: None,
                },
            },
        },
        // Reached with no `[documents]` too: switched on without it, the
        // parent is `Unready`, which blocks nothing. A missing table is "not
        // set up yet", not the owner's no to OCR, so the part is unready
        // beside its parent — shown, blocking nothing — as the library is
        // beside a table-less `image` (review of #449, pass 3). `off` here
        // hid OCR and blocked layout under an unready parent.
        Feature::Ocr if cfg.documents.is_none() => State::Unready {
            reason: "no [documents] table".into(),
            fix: Some("add a [documents] table".into()),
        },
        Feature::Ocr => match &cfg.documents {
            Some(d) if d.ocr => match crate::document::ocr_url(&d.ocr_url) {
                Ok(url) => on(url.to_string()),
                Err(e) => State::Unready {
                    reason: format!("{e:#}"),
                    fix: None,
                },
            },
            _ => off("[documents] ocr = false", "set [documents] ocr = true"),
        },
        // As `ocr`: no table is "not set up yet", never `layout = false`.
        Feature::Layout if cfg.documents.is_none() => State::Unready {
            reason: "no [documents] table".into(),
            fix: Some("add a [documents] table".into()),
        },
        Feature::Layout => match &cfg.documents {
            Some(d) if d.layout => on("region by region"),
            _ => off(
                "[documents] layout = false",
                "set [documents] layout = true",
            ),
        },
        // A server off this machine is refused at registration, not
        // registered (`imagegen::loopback_url`), so it is not on here either.
        Feature::Image => match &cfg.image {
            Some(image) => match crate::imagegen::loopback_url(&image.url) {
                Ok(url) => on(url.to_string()),
                Err(e) => State::Unready {
                    reason: format!("{e:#}"),
                    fix: None,
                },
            },
            None => off("no [image] table", "add an [image] table"),
        },
        // `[video] url` off this machine is refused at load
        // (`Config::validate`), so this reads an address already vetted;
        // asked again so the row says why rather than trusting the order.
        Feature::Video => match &cfg.video {
            Some(video) => match video.loopback_url() {
                Ok(url) => on(url.to_string()),
                Err(e) => State::Unready {
                    reason: format!("{e:#}"),
                    fix: None,
                },
            },
            None => off("no [video] table", "add a [video] table"),
        },
        // The library is the store and the owner's page, on with `image`
        // (the owner's ruling L1, 2026-09-30: there is no reason to want
        // images without it). `[tools]` narrows what the *model* reaches,
        // per tool, like any builtin — said in the detail, never an `off`
        // that would take the owner's page with it. Its tools register only
        // inside `[image]`, which `image`'s own row answers for.
        Feature::Library => match crate::imagelib::Library::default_dir() {
            Ok(dir) => {
                let withheld: Vec<&str> = [
                    "image_library",
                    "image_library_propose",
                    "image_library_reference",
                ]
                    .into_iter()
                    .filter(|t| !cfg.tools.registers(t))
                    .collect();
                on(match withheld.as_slice() {
                    [] => dir.display().to_string(),
                    names => format!(
                        "{} ([tools] withholds {} from the model)",
                        dir.display(),
                        names.join(", ")
                    ),
                })
            }
            Err(e) => State::Unknown {
                reason: format!("the library's directory: {e:#}"),
            },
        },
        // The store and the owner's page. The model's one door into it,
        // `persona_propose`, is narrowed by `[tools]` like the library's, and
        // said here rather than left to vanish (review of #493).
        Feature::Personas => on(if cfg.tools.registers("persona_propose") {
            "~/.mecha/personas".to_string()
        } else {
            "~/.mecha/personas ([tools] withholds persona_propose from the model)".to_string()
        }),
        // `voice` stays on at port 0: `mecha voice-serve` is its own surface,
        // with its own `--port`. The sentence says only what `mecha serve`
        // mounts (review of #503).
        Feature::Voice => on(match cfg.voice.voice_port() {
            0 => "`mecha voice-serve`; `mecha serve` mounts no facade ([voice] voice_port = 0)"
                .to_string(),
            port => format!(
                "`mecha voice-serve`, and the facade `mecha serve` mounts on [voice] voice_port \
                 {port}"
            ),
        }),
        Feature::Dictate => on(format!("speech to text at {}", cfg.voice.stt_url())),
        // Empty means `mecha serve` does not reach the voice worker at all:
        // `offer_target` is the worker's address for everything serve sends
        // it — the call relay, Listen, the voice previews and lists
        // (`settings::speak`, `library_voice_sample`, `library_voices`).
        // Calls stay on — a browser can reach the worker's own door, and the
        // facade the worker calls mounts on this row — and the row says what
        // goes (the owner's choice, 2026-10-02: say it, rather than split
        // the address in two).
        Feature::Calls => on(match cfg.voice.offer_target() {
            Some(target) => format!("offers to {target}"),
            None => "`mecha serve` does not reach the voice worker ([voice] offer_target is \
                     empty): no call relay, no Listen, no voice previews; calls only through \
                     the worker's own door"
                .to_string(),
        }),
        Feature::Cloning => match &cfg.voice.voices_dir {
            Some(dir) => on(dir.display().to_string()),
            None => off(
                "no [voice] voices_dir",
                "`mecha features enable voice` writes it for the speech server it installs, \
                 where there is no [voice] table; otherwise, or for one you run, set [voice] voices_dir to the directory it reads voices from \
                 (Chatterbox: the one it mounts as /voices)",
            ),
        },
        Feature::Incognito => match crate::config::provider_is_local(cfg, &cfg.default_provider) {
            Ok(()) => on(format!("provider `{}`", cfg.default_provider)),
            Err(why) => off(
                format!("{why} — an incognito chat needs a local model"),
                "point default_provider at a server on this machine, with no fallbacks",
            ),
        },
        // The queue half: `factory-publish drain` fills ~/.mecha/requests,
        // and the Review page and `mecha frontdoor` read it with no MCP
        // entry at all (found on review of #427).
        Feature::Frontdoor => match facts.has_factory_binary {
            true => on("`factory-publish` on PATH — its drain fills ~/.mecha/requests"),
            false => off(
                "`factory-publish` is not installed",
                "cargo install mecha-factory-publish --locked",
            ),
        },
        // The model's half: publishing tools, from an `[[mcp]]` entry.
        Feature::Publishing => server_state(
            Feature::Publishing,
            facts,
            "factory-publish",
            facts.has_factory_binary,
            "cargo install mecha-factory-publish --locked",
            None,
            "",
        ),
        // Nothing to set up: past the switch, it is on.
        Feature::Messages => on("[messages] enabled"),
        // The series `mecha-system-sample.timer` writes; a missing one is the
        // tool's to say per call ("is the timer running?"), never a reason
        // to withhold the tool and move the cached prefix.
        Feature::System => match cfg.tools.registers("system_read") {
            true => on("`system_read` over ~/.mecha/system/series.sqlite"),
            false => tools_off("system_read"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An entry running a copy of one of mecha's own servers from another
    /// build is pointed at mecha's matching one, `command` alone; the
    /// owner's own checkout build is left as it is (§15).
    #[test]
    fn a_stale_copy_s_entry_is_pointed_at_the_matching_build() {
        let root = std::env::temp_dir().join(format!("mecha-mcp-r-{}", uuid::Uuid::new_v4()));
        let cargo_root = root.join("cargo");
        std::fs::create_dir_all(cargo_root.join("bin")).unwrap();
        let old = cargo_root.join("bin/mecha-graph-mcp");
        std::fs::write(&old, "").unwrap();
        let record = |source: &str| {
            std::fs::write(
                cargo_root.join(".crates2.json"),
                serde_json::json!({"installs": {
                    format!("mecha-graph-mcp 0.1.5 ({source})"): {"bins": ["mecha-graph-mcp"]}
                }})
                .to_string(),
            )
            .unwrap();
        };
        let entry = format!(
            "[[mcp]]\nname = \"graph\"\ncommand = \"{}\"\nprefix_tools = false\n\
             [mcp.capabilities]\nuntrusted_input = true\n",
            old.display()
        );
        let graph = mcp_wiring(Feature::Graph).unwrap();
        let ours = root.join("sidecars/graph-mcp/bin/mecha-graph-mcp");
        let cfg = root.join("config.toml");
        // The published crate under the pin's commit: another build.
        record("registry+https://github.com/rust-lang/crates.io-index");
        std::fs::write(&cfg, &entry).unwrap();
        let done = write_mcp_entries(&cfg, &[(&graph, ours.as_path())], &|_| false).unwrap();
        assert_eq!(done[0].repointed.as_deref(), Some(old.to_str().unwrap()));
        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains(&ours.display().to_string()), "{text}");
        assert!(text.contains("prefix_tools = false") && text.contains("untrusted_input = true"));
        // The owner's checkout build: theirs, not touched.
        record("path+file:///home/u/mecha-graph/mecha-graph-mcp");
        std::fs::write(&cfg, &entry).unwrap();
        let done = write_mcp_entries(&cfg, &[(&graph, ours.as_path())], &|_| false).unwrap();
        assert!(done[0].repointed.is_none());
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), entry);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The graph's entry carries mecha's embedder; mail's carries no
    /// environment (the owner's fresh Mac, 2026-10-15).
    #[test]
    fn the_graph_entry_names_mecha_s_embedder() {
        let root = std::env::temp_dir().join(format!("mecha-mcp-g-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let p = root.join("config.toml");
        let graph = mcp_wiring(Feature::Graph).unwrap();
        let mail = mcp_wiring(Feature::Mail).unwrap();
        write_mcp_entries(
            &p,
            &[
                (&graph, Path::new("/b/mecha-graph-mcp")),
                (&mail, Path::new("/b/mecha-mail")),
            ],
            &|_| false,
        )
        .unwrap();
        let cfg: crate::config::Config =
            toml::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let entry = |n: &str| cfg.mcp.iter().find(|m| m.name == n).unwrap().clone();
        let env = entry("graph").env;
        assert_eq!(env["MECHA_GRAPH_EMBED_DIMS"], crate::recommend::EMBED_DIMS);
        assert_eq!(env.len(), GRAPH_EMBED_ENV.len());
        assert!(entry("mail").env.is_empty());
        // An entry written before this, with no embedder, is never edited
        // and is reported with what it lacks (review of #698).
        let old = root.join("old.toml");
        std::fs::write(
            &old,
            "[[mcp]]\nname = \"graph\"\ncommand = \"/b/mecha-graph-mcp\"\nprefix_tools = false\n\
             [mcp.capabilities]\nuntrusted_input = true\n",
        )
        .unwrap();
        let before = std::fs::read_to_string(&old).unwrap();
        let done = write_mcp_entries(&old, &[(&graph, Path::new("/b/mecha-graph-mcp"))], &|_| {
            false
        })
        .unwrap();
        assert!(!done[0].wrote_entry);
        assert_eq!(done[0].env_missing, GRAPH_EMBED_ENV);
        assert_eq!(std::fs::read_to_string(&old).unwrap(), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// One command decides the outbox once: `enable mail docs` stages both
    /// features' calls in the `[outbox]` it writes, in either order, rather
    /// than the second finding the first's fresh table (review of #669).
    #[test]
    fn one_enable_routes_every_feature_it_wires() {
        let root = std::env::temp_dir().join(format!("mecha-mcp-u-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mail = mcp_wiring(Feature::Mail).unwrap();
        let docs = mcp_wiring(Feature::Docs).unwrap();
        let mut seen = Vec::new();
        for (name, order) in [("md.toml", [&mail, &docs]), ("dm.toml", [&docs, &mail])] {
            let p = root.join(name);
            let items: Vec<(&McpWiring, &Path)> =
                order.iter().map(|w| (*w, Path::new("/b/x"))).collect();
            let done = write_mcp_entries(&p, &items, &|_| false).unwrap();
            assert!(done
                .iter()
                .all(|d| d.wrote_entry && d.wrote_outbox && d.outbox_missing.is_empty()));
            let v: toml::Value = toml::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            let mut tools: Vec<String> = v["outbox"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect();
            tools.sort();
            seen.push(tools);
        }
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(seen[0].len(), mail.outbox.len() + docs.outbox.len());
        assert_eq!(seen[0], seen[1], "order does not decide what is staged");

        // Not staged (an operator rule would conflict): no [outbox] written,
        // nothing reported missing of a table that does not exist (review of
        // #669, pass 4), the entry still written.
        let root = std::env::temp_dir().join(format!("mecha-mcp-n-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let p = root.join("ruled.toml");
        let done = write_mcp_entries(&p, &[(&mail, Path::new("/b/mecha-mail"))], &|t| {
            t == "mail__mail_send"
        })
        .unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert!(done[0].wrote_entry && !done[0].wrote_outbox);
        assert!(done[0].outbox_missing.is_empty(), "{:?}", done[0]);
        assert_eq!(done[0].ruled, ["mail__mail_send"]);
        assert!(!text.contains("[outbox]"), "{text}");

        // The owner's `[outbox]` exists: a ruled route is left out of the
        // advice to add, and returned apart (review of #669, pass 7).
        let p = root.join("owned-ruled.toml");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&p, "[outbox]\ntools = []\n").unwrap();
        let done = write_mcp_entries(&p, &[(&mail, Path::new("/b/mecha-mail"))], &|t| {
            t == "mail__mail_send"
        })
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(done[0].ruled_unrouted, ["mail__mail_send"]);
        assert!(done[0].ruled.is_empty());
        assert_eq!(done[0].outbox_missing.len(), mail.outbox.len() - 1);
        assert!(!done[0]
            .outbox_missing
            .iter()
            .any(|t| t == "mail__mail_send"));
    }

    /// The routing is spelled off the entry that serves the feature, not the
    /// docs' entry: an owner's `gmail` entry running `mecha-mail` is routed
    /// as `gmail__…`, an unprefixed one by the bare names, and an entry named
    /// `mail` that runs a wrapper is the owner's mail — no second `mail` is
    /// appended beside it (review of #669, pass 4) — and a disabled duplicate
    /// never names the routing over a live entry (pass 6). Each of these
    /// wrote `mail__…` names that route nothing, or a duplicate entry, before.
    #[test]
    fn the_routing_is_named_off_the_entry_that_serves_it() {
        let root = std::env::temp_dir().join(format!("mecha-mcp-r-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mail = mcp_wiring(Feature::Mail).unwrap();
        let write = |name: &str, text: &str| {
            let p = root.join(name);
            std::fs::write(&p, text).unwrap();
            let done = write_mcp_entry(&p, &mail, Path::new("/b/mecha-mail")).unwrap();
            let v: toml::Value = toml::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            (done, v)
        };
        let (renamed, renamed_v) = write(
            "renamed.toml",
            "[[mcp]]\nname = \"gmail\"\ncommand = \"~/.cargo/bin/mecha-mail\"\n",
        );
        let (bare, _) = write(
            "bare.toml",
            "[[mcp]]\nname = \"mail\"\ncommand = \"/b/mecha-mail\"\nprefix_tools = false\n\n\
             [outbox]\ntools = [\"mail_send\"]\n",
        );
        let (wrapped, wrapped_v) = write(
            "wrapped.toml",
            "[[mcp]]\nname = \"mail\"\ncommand = \"/usr/local/bin/mail-wrap\"\n",
        );
        let (off, off_v) = write(
            "off.toml",
            "[[mcp]]\nname = \"mail\"\ncommand = \"/b/mecha-mail\"\ndisabled = true\n",
        );
        let (stale, stale_v) = write(
            "stale.toml",
            "[[mcp]]\nname = \"mail\"\ncommand = \"/b/mecha-mail\"\ndisabled = true\n\n\
             [[mcp]]\nname = \"gmail\"\ncommand = \"/b/mecha-mail\"\n",
        );
        let _ = std::fs::remove_dir_all(&root);

        assert!(!renamed.wrote_entry && renamed.wrote_outbox);
        let tools: Vec<&str> = renamed_v["outbox"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.as_str())
            .collect();
        assert_eq!(tools.len(), mail.outbox.len());
        assert!(tools.iter().all(|t| t.starts_with("gmail__")), "{tools:?}");
        assert!(tools.contains(&"gmail__mail_send"));

        assert!(!bare.wrote_entry && !bare.wrote_outbox);
        assert_eq!(
            bare.outbox_missing,
            [
                "mail_reply",
                "calendar_create_event",
                "calendar_update_event",
                "calendar_delete_event"
            ]
        );

        // Only a disabled entry runs it: left as it is, and said.
        assert_eq!(off.disabled.as_deref(), Some("mail"));
        assert!(!off.wrote_entry && off_v["mcp"].as_array().unwrap().len() == 1);
        assert!(stale.disabled.is_none() && renamed.disabled.is_none());

        // A disabled duplicate first: the live `gmail` names the routing.
        assert!(!stale.wrote_entry && stale.wrote_outbox);
        assert_eq!(
            stale_v["outbox"]["tools"][0].as_str(),
            Some("gmail__mail_send")
        );

        assert!(!wrapped.wrote_entry);
        assert_eq!(
            wrapped.runs_other.as_deref(),
            Some("/usr/local/bin/mail-wrap")
        );
        assert!(renamed.runs_other.is_none() && bare.runs_other.is_none());
        assert_eq!(wrapped_v["mcp"].as_array().unwrap().len(), 1);
        assert_eq!(wrapped.routes, mail.outbox);
    }

    /// `enable mail` writes the entry the docs give, untrusted, with its sends
    /// staged where there is no `[outbox]`; a second write changes nothing; an
    /// owner's `[outbox]` is never edited and what it lacks is returned — on a
    /// hand-wired install too; docs stages its write verbs; the graph keeps
    /// its tools' own names; and the written file loads as a config (the
    /// owner, 2026-10-12; review of #669).
    #[test]
    fn enable_writes_a_server_feature_its_entry() {
        let root = std::env::temp_dir().join(format!("mecha-mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let bin = Path::new("/opt/bin/mecha-mail");
        let mail = mcp_wiring(Feature::Mail).unwrap();
        let lacking: Vec<String> = mail.outbox[1..].iter().map(|t| t.to_string()).collect();
        let write = |name: &str, text: &str, w: &McpWiring, cmd: &str| {
            let p = root.join(name);
            std::fs::write(&p, text).unwrap();
            let done = write_mcp_entry(&p, w, Path::new(cmd)).unwrap();
            (done, std::fs::read_to_string(&p).unwrap(), p)
        };
        let (fresh, text, fresh_path) = write(
            "fresh.toml",
            "# mine\n[features]\nmail = true\n",
            &mail,
            "/opt/bin/mecha-mail",
        );
        let again = write_mcp_entry(&fresh_path, &mail, bin).unwrap();
        let owned_before = "[outbox]\ntools = [\"mail__mail_send\"]\n";
        let (partial, owned_text, _) =
            write("owned.toml", owned_before, &mail, "/opt/bin/mecha-mail");
        let hand_before = "[[mcp]]\nname = \"mail\"\ncommand = \"~/.cargo/bin/mecha-mail\"\n\n\
                           [outbox]\ntools = [\"mail__mail_send\"]\n";
        let (hand, hand_text, _) = write("hand.toml", hand_before, &mail, "/opt/bin/mecha-mail");
        let (_, graph_text, _) = write(
            "graph.toml",
            "",
            &mcp_wiring(Feature::Graph).unwrap(),
            "/b/mecha-graph-mcp",
        );
        let (_, docs_text, _) = write(
            "docs.toml",
            "",
            &mcp_wiring(Feature::Docs).unwrap(),
            "/b/mecha-docs",
        );
        let cfg = crate::config::Config::load_layers(Some(&fresh_path), None);
        let _ = std::fs::remove_dir_all(&root);

        let tools = |v: &toml::Value| -> Vec<String> {
            v["outbox"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect()
        };
        // Fresh: entry and routing written, comments kept.
        assert!(fresh.wrote_entry && fresh.wrote_outbox && fresh.outbox_missing.is_empty());
        assert!(text.starts_with("# mine"), "{text}");
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        let entry = &parsed["mcp"][0];
        assert_eq!(entry["name"].as_str(), Some("mail"));
        assert_eq!(entry["command"].as_str(), Some("/opt/bin/mecha-mail"));
        assert_eq!(
            entry["capabilities"]["untrusted_input"].as_bool(),
            Some(true)
        );
        assert_eq!(tools(&parsed), mail.outbox);
        // Again: nothing.
        assert!(!again.wrote_entry && !again.wrote_outbox && again.outbox_missing.is_empty());
        // The present case: the entry it wrote carries the capability, so a
        // re-run never warns that it is missing (review of #669, pass 7).
        assert!(!again.untrusted_missing);
        // An owner's partial [outbox]: the entry, not the table; the gap said.
        assert!(partial.wrote_entry && !partial.wrote_outbox);
        assert_eq!(partial.outbox_missing, lacking);
        assert!(owned_text.starts_with(owned_before), "{owned_text}");
        // Hand-wired: nothing written, the gap still said.
        assert!(!hand.wrote_entry && !hand.wrote_outbox);
        assert_eq!(hand.outbox_missing, lacking);
        assert_eq!(hand_text, hand_before);
        // Graph keeps its names and stages nothing; docs stages its writes.
        let g: toml::Value = toml::from_str(&graph_text).unwrap();
        assert_eq!(g["mcp"][0]["prefix_tools"].as_bool(), Some(false));
        assert!(g.get("outbox").is_none());
        let d: toml::Value = toml::from_str(&docs_text).unwrap();
        assert_eq!(
            tools(&d),
            [
                "docs__docs_append",
                "docs__docs_replace",
                "docs__sheets_write"
            ]
        );
        // The written file loads.
        let cfg = cfg.expect("the written file loads");
        assert!(!fresh.untrusted_missing);
        // The hand-wired entry above forces nothing: that is said.
        assert!(hand.untrusted_missing);
        assert!(cfg.mcp[0].capabilities.untrusted_input);
        assert_eq!(cfg.outbox.tools, mail.outbox);
    }

    fn mcp(name: &str, command: &str) -> McpServerConfig {
        let mut m: McpServerConfig =
            toml::from_str(&format!("name = \"{name}\"\ncommand = \"{command}\"")).unwrap();
        m.disabled = false;
        m
    }

    /// A machine where every store was read and held nothing. Not
    /// `Facts::default()`, whose `None`s are *unreadable* — the safe default,
    /// and the wrong fixture for "nothing installed".
    fn empty_machine() -> Facts {
        Facts {
            slack_linked: Some(false),
            mail_accounts: Some(0),
            docs_accounts: Some(0),
            ..Facts::default()
        }
    }

    /// `facts` read against `cfg` with **every switch on** — the tests of a
    /// row's own settings, which the switch would otherwise hide. The switch
    /// itself has its own tests, which build their `Facts` as they are.
    fn at(cfg: &Config, facts: &Facts) -> Facts {
        let mut config = cfg.clone();
        switch_all(&mut config, true);
        Facts {
            config,
            ..facts.clone()
        }
    }

    fn switch_all(cfg: &mut Config, on: bool) {
        for &f in Feature::ALL {
            set_switch(cfg, f, on);
        }
    }

    fn get(rows: &[Row], f: Feature) -> &State {
        &rows.iter().find(|r| r.id == f).unwrap().state
    }

    /// `Feature::ALL` is a hand-kept list, and every other test iterates it
    /// — so a variant left out of it would vanish from `mecha features` and
    /// from every test at once. This `match` has no wildcard: a new variant
    /// does not compile until it is placed here, and placing it here is
    /// placing it in `ALL` (found on review of #428).
    #[test]
    fn every_variant_is_in_all() {
        for &f in Feature::ALL {
            match f {
                Feature::Web
                | Feature::Slack
                | Feature::Mail
                | Feature::Docs
                | Feature::Graph
                | Feature::Tasks
                | Feature::Search
                | Feature::Documents
                | Feature::Ocr
                | Feature::Layout
                | Feature::Image
                | Feature::Library
                | Feature::Video
                | Feature::Personas
                | Feature::Voice
                | Feature::Dictate
                | Feature::Calls
                | Feature::Cloning
                | Feature::Incognito
                | Feature::Frontdoor
                | Feature::Publishing
                | Feature::Messages
                | Feature::System => {}
            }
        }
        // The match above lists 23 — raise this with it, and a variant
        // added to the match but not to `ALL` fails here.
        assert_eq!(Feature::ALL.len(), 23);
    }

    #[test]
    fn ids_are_unique_parse_back_and_match_the_wire_name() {
        let mut seen = std::collections::BTreeSet::new();
        for &f in Feature::ALL {
            assert!(seen.insert(f.id()), "duplicate id {}", f.id());
            assert_eq!(Feature::parse(f.id()), Some(f));
            assert_eq!(
                serde_json::to_value(f).unwrap(),
                serde_json::Value::String(f.id().into())
            );
        }
        assert_eq!(seen.len(), Feature::ALL.len());
    }

    /// `state` recurses through `needs`; a cycle would never return, and a
    /// part listed before its parent would read in the wrong order.
    #[test]
    fn everything_a_feature_needs_comes_before_it() {
        for (i, &f) in Feature::ALL.iter().enumerate() {
            for need in f.needs() {
                let j = Feature::ALL.iter().position(|&g| g == need).unwrap();
                assert!(j < i, "{} needs {}, listed after it", f.id(), need.id());
            }
        }
    }

    /// The light install: a default config and an empty machine turn on
    /// nothing — every switch is absent — and nothing reads as broken.
    #[test]
    fn a_default_config_on_an_empty_machine_is_off_and_never_unknown() {
        let rows = all(&empty_machine());
        for row in &rows {
            assert!(
                !matches!(row.state, State::Unknown { .. } | State::Unready { .. }),
                "{} is {:?} on an empty machine",
                row.id.id(),
                row.state
            );
        }
        for f in [
            Feature::Web,
            Feature::Slack,
            Feature::Mail,
            Feature::Graph,
            Feature::Search,
            Feature::Documents,
            Feature::Image,
            Feature::Personas,
            Feature::Voice,
            Feature::Frontdoor,
            Feature::Messages,
            Feature::System,
        ] {
            assert_eq!(get(&rows, f).word(), "off", "{}", f.id());
        }
        // And none of it is announced: nothing was set up.
        assert!(announcements(&empty_machine()).is_empty());
        // Parts and dependents say what they wait on rather than repeating
        // their parent's reason.
        assert_eq!(
            get(&rows, Feature::Tasks),
            &State::Blocked { on: Feature::Graph }
        );
        assert_eq!(
            get(&rows, Feature::Layout),
            &State::Blocked { on: Feature::Ocr }
        );
        // A part waits on its parent first: voice, then web.
        assert_eq!(
            get(&rows, Feature::Dictate),
            &State::Blocked { on: Feature::Voice }
        );
        // Voice has a surface without the web app (`mecha voice-serve`);
        // with it on, the parts that live in `mecha serve` wait on web.
        let mut voice_on = Config::default();
        voice_on.features.0.insert("voice".into(), true);
        let facts = Facts {
            config: voice_on,
            ..empty_machine()
        };
        assert_eq!(state(&facts, Feature::Voice).word(), "on");
        assert_eq!(
            state(&facts, Feature::Cloning),
            State::Blocked { on: Feature::Web }
        );
        assert_eq!(
            get(&rows, Feature::Library),
            &State::Blocked { on: Feature::Image }
        );
    }

    /// An installed binary with no `[[mcp]]` entry is not on: the entry is
    /// what puts the tools on the surface, and setup's `mail` step checks
    /// only the binary (FEATURES-DESIGN.md §1.1). Switched on, that reads
    /// `unready` — the owner said yes and the fix is one entry away (§5).
    #[test]
    fn a_server_is_on_only_through_an_enabled_mcp_entry() {
        let mut cfg = Config::default();
        let mut facts = Facts {
            has_mail_binary: true,
            mail_accounts: Some(2),
            ..Facts::default()
        };
        let State::Unready { reason, .. } = state(&at(&cfg, &facts), Feature::Mail) else {
            panic!("an installed binary alone is not mail")
        };
        assert!(reason.contains("no [[mcp]] entry"), "{reason}");

        cfg.mcp
            .push(mcp("mail", "/home/someone/.cargo/bin/mecha-mail"));
        assert_eq!(state(&at(&cfg, &facts), Feature::Mail).word(), "on");

        facts.mail_accounts = Some(0);
        assert_eq!(state(&at(&cfg, &facts), Feature::Mail).word(), "unready");
        facts.mail_accounts = None;
        assert_eq!(state(&at(&cfg, &facts), Feature::Mail).word(), "unknown");

        cfg.mcp[0].disabled = true;
        assert_eq!(state(&at(&cfg, &facts), Feature::Mail).word(), "unready");
        // A disabled duplicate does not hide an enabled entry.
        cfg.mcp.push(mcp("mail2", "mecha-mail"));
        facts.mail_accounts = Some(1);
        assert_eq!(state(&at(&cfg, &facts), Feature::Mail).word(), "on");
    }

    /// `Facts::read` carries the configuration it was given, and `state`
    /// reads nothing else: the global-only rule is the signature's.
    #[test]
    fn a_row_reads_only_the_configuration_facts_carry() {
        // `Facts::read` reads the mail-store variables, which doctor's tests
        // move; take the crate's one environment lock.
        let _lock = crate::work::tests::lock();
        let with_graph = Config {
            mcp: vec![mcp("graph", "mecha-graph-mcp")],
            ..Config::default()
        };
        let mut with_graph = with_graph;
        with_graph.features.0.insert("graph".into(), true);
        let mut switched = Config::default();
        switched.features.0.insert("graph".into(), true);
        let empty = Facts::read(Path::new("/nonexistent"), &switched);
        assert_eq!(state(&empty, Feature::Graph).word(), "unready");
        let owners = Facts::read(Path::new("/nonexistent"), &with_graph);
        assert_eq!(state(&owners, Feature::Graph).word(), "on");
    }

    /// `Unready` and `Unknown` do not block what depends on them — both are
    /// shown, and hiding the dependents of a feature that may be working
    /// would be the silent kind of wrong.
    #[test]
    fn only_off_and_blocked_block_dependents() {
        assert!(State::Unknown {
            reason: String::new()
        }
        .satisfies());
        assert!(State::Unready {
            reason: String::new(),
            fix: None
        }
        .satisfies());
        // `[documents]` whose own promise fails is unready; its OCR part,
        // whose own address is fine, is still reported rather than hidden —
        // as unready too, since `document_read` is what reaches it.
        let cfg = Config {
            documents: Some(
                toml::from_str::<crate::document::DocumentsConfig>("ocr = true\nocr_model = \"\"")
                    .unwrap(),
            ),
            ..Config::default()
        };
        let facts = Facts::default();
        assert_eq!(
            state(&at(&cfg, &facts), Feature::Documents).word(),
            "unready"
        );
        let ocr = state(&at(&cfg, &facts), Feature::Ocr);
        assert!(ocr.shown(), "{ocr:?}");
        assert_eq!(ocr.word(), "unready");
    }

    /// The front door's queue needs no mailbox and no `[[mcp]]` entry: the
    /// drain fills it from the binary. Publishing is the part that needs the
    /// entry (found on review of #427 and #428).
    #[test]
    fn the_front_door_is_its_binary_and_publishing_its_entry() {
        let mut cfg = Config::default();
        let mut facts = Facts {
            has_factory_binary: true,
            ..Facts::default()
        };
        assert_eq!(state(&at(&cfg, &facts), Feature::Frontdoor).word(), "on");
        assert_eq!(state(&at(&cfg, &facts), Feature::Publishing).word(), "off");
        cfg.mcp.push(mcp("factory", "factory-publish"));
        assert_eq!(state(&at(&cfg, &facts), Feature::Publishing).word(), "on");
        // Switched on without its binary, the queue is unready — shown with
        // the install command — and does not block its part, which is
        // unready beside it rather than on.
        facts.has_factory_binary = false;
        assert_eq!(
            state(&at(&cfg, &facts), Feature::Frontdoor).word(),
            "unready"
        );
        assert_eq!(
            state(&at(&cfg, &facts), Feature::Publishing).word(),
            "unready"
        );
        // Switched off, the part is blocked on it.
        let mut off = at(&cfg, &facts);
        off.config.features.0.insert("frontdoor".into(), false);
        assert_eq!(
            state(&off, Feature::Publishing),
            State::Blocked {
                on: Feature::Frontdoor
            }
        );
    }

    /// The owner's yes is never hidden: switched on without its settings, a
    /// feature is `Unready`, shown, with the fix (FEATURES-DESIGN.md §5). A
    /// part has no switch, so its own `off` is the owner's no and hides. A
    /// `Blocked` row's `next` is the command of the need that blocks it,
    /// not the owner's switch that is already on.
    #[test]
    fn a_switched_on_feature_is_never_hidden_and_a_blocked_row_names_its_cause() {
        let mut cfg = Config::default();
        cfg.features.0.insert("image".into(), true);
        let facts = |cfg: &Config| Facts {
            config: cfg.clone(),
            ..empty_machine()
        };
        let rows = all(&facts(&cfg));
        let row = |f: Feature| rows.iter().find(|r| r.id == f).unwrap();
        let State::Unready { reason, fix } = &row(Feature::Image).state else {
            panic!("image = true with no [image] table is unready")
        };
        assert!(reason.contains("[image]"), "{reason}");
        assert!(fix.is_some());
        assert!(row(Feature::Image).shown);
        assert_eq!(row(Feature::Image).next, *fix);
        // Its part is not blocked by an unready parent, and not on either:
        // the library's tools register only inside `[image]`.
        assert!(row(Feature::Library).shown);
        assert_eq!(row(Feature::Library).state.word(), "unready");
        // Off and blocked hide; the switch's own fix is the next command.
        assert!(!row(Feature::Mail).shown);
        assert_eq!(
            row(Feature::Mail).next.as_deref(),
            Some("mecha features enable mail")
        );
        assert_eq!(
            row(Feature::Dictate).state,
            State::Blocked { on: Feature::Voice }
        );
        assert!(!row(Feature::Dictate).shown);
        assert_eq!(
            row(Feature::Dictate).next.as_deref(),
            Some("mecha features enable web voice")
        );

        // Switched on with no table: the parent and both parts are unready
        // and shown — a missing table blocks nothing, as with `image`.
        cfg.features.0.insert("documents".into(), true);
        let rows = all(&facts(&cfg));
        let row = |f: Feature| rows.iter().find(|r| r.id == f).unwrap();
        for f in [Feature::Documents, Feature::Ocr, Feature::Layout] {
            assert_eq!(row(f).state.word(), "unready", "{}", f.id());
            assert!(row(f).shown, "{}", f.id());
            assert_eq!(row(f).next.as_deref(), Some("add a [documents] table"));
        }

        // A part off by its own setting: hidden, and its dependent's next
        // command is that setting — not `enable documents`, already on.
        cfg.features.0.insert("documents".into(), true);
        cfg.documents =
            Some(toml::from_str::<crate::document::DocumentsConfig>("ocr = false").unwrap());
        let rows = all(&facts(&cfg));
        let row = |f: Feature| rows.iter().find(|r| r.id == f).unwrap();
        assert!(row(Feature::Documents).shown);
        assert_eq!(row(Feature::Ocr).state.word(), "off");
        assert!(!row(Feature::Ocr).shown);
        assert_eq!(
            row(Feature::Layout).state,
            State::Blocked { on: Feature::Ocr }
        );
        assert_eq!(
            row(Feature::Layout).next.as_deref(),
            Some("set [documents] ocr = true")
        );

        // Nothing standing on an unready need reads on (review of #449):
        // `[tools]` withholding `document_read` leaves OCR and layout with
        // no tool to reach them, and a graph with no entry leaves the board
        // with none — shown, unready, never on.
        cfg.documents = Some(
            toml::from_str::<crate::document::DocumentsConfig>("ocr = true\nlayout = true")
                .unwrap(),
        );
        cfg.tools.disabled = vec!["document_read".into()];
        cfg.features.0.insert("graph".into(), true);
        let rows = all(&facts(&cfg));
        let row = |f: Feature| rows.iter().find(|r| r.id == f).unwrap();
        for f in [
            Feature::Documents,
            Feature::Ocr,
            Feature::Layout,
            Feature::Tasks,
        ] {
            assert_eq!(row(f).state.word(), "unready", "{}", f.id());
            assert!(row(f).shown, "{}", f.id());
        }
        let State::Unready { reason, .. } = &row(Feature::Ocr).state else {
            unreachable!()
        };
        assert!(reason.contains("`documents`"), "{reason}");
    }

    /// Each row asks what `prepare_tools` asks before registering the tool,
    /// so the list never reads `on` for a feature the run does not have
    /// (found on review of #428).
    #[test]
    fn a_row_is_on_only_where_its_tool_would_register() {
        let facts = Facts::default();
        let docs =
            || Some(toml::from_str::<crate::document::DocumentsConfig>("ocr = false").unwrap());
        let mut cfg = Config {
            documents: docs(),
            ..Config::default()
        };
        assert_eq!(state(&at(&cfg, &facts), Feature::Documents).word(), "on");
        cfg.tools.disabled = vec!["document_read".into()];
        assert_eq!(
            state(&at(&cfg, &facts), Feature::Documents).word(),
            "unready"
        );

        // A server off this machine is refused at registration.
        let image = |url: &str| {
            let mut i: crate::imagegen::ImageConfig = toml::from_str("").unwrap();
            i.url = url.into();
            Some(i)
        };
        let mut cfg = Config {
            image: image("http://127.0.0.1:8188"),
            ..Config::default()
        };
        assert_eq!(state(&at(&cfg, &facts), Feature::Image).word(), "on");
        assert_eq!(state(&at(&cfg, &facts), Feature::Library).word(), "on");
        // `[tools]` withholding the library's tools narrows the model, not
        // the owner: the library stays on with `image` and says which tools
        // the model lacks (ruling L1).
        cfg.tools.disabled = vec!["image_library".into(), "image_library_propose".into()];
        let State::On { detail } = state(&at(&cfg, &facts), Feature::Library) else {
            panic!("the library follows image, whatever [tools] says")
        };
        assert!(
            detail.contains("withholds image_library, image_library_propose"),
            "{detail}"
        );
        // As the library's: the personas page stays on, and the row says the
        // model cannot propose.
        cfg.tools.disabled.push("persona_propose".into());
        let State::On { detail } = state(&at(&cfg, &facts), Feature::Personas) else {
            panic!("personas stay on whatever [tools] says")
        };
        assert!(detail.contains("withholds persona_propose"), "{detail}");
        cfg.image = image("http://10.0.0.5:8188");
        assert_eq!(state(&at(&cfg, &facts), Feature::Image).word(), "unready");

        // A backend the chain builder would drop does not make search on.
        let backend =
            |t: &str| -> crate::config::SearchBackendConfig { toml::from_str(t).unwrap() };
        let mut cfg = Config {
            search: vec![backend(
                "kind = \"exa\"\napi_key_env = \"MECHA_TEST_SURELY_UNSET_KEY\"",
            )],
            ..Config::default()
        };
        assert_eq!(state(&at(&cfg, &facts), Feature::Search).word(), "unready");
        cfg.search.push(backend("kind = \"brave\""));
        assert_eq!(state(&at(&cfg, &facts), Feature::Search).word(), "unready");
        cfg.search.push(backend(
            "kind = \"searxng\"\nbase_url = \"http://127.0.0.1:8888\"",
        ));
        let State::On { detail } = state(&at(&cfg, &facts), Feature::Search) else {
            panic!("one live backend is search")
        };
        assert!(detail.starts_with("searxng (dropped"), "{detail}");
    }

    #[test]
    fn a_documents_table_turns_on_its_parts_by_their_own_switches() {
        let cfg = Config {
            documents: Some(
                toml::from_str::<crate::document::DocumentsConfig>("ocr = true\nlayout = false")
                    .unwrap(),
            ),
            ..Config::default()
        };
        let facts = Facts::default();
        assert_eq!(state(&at(&cfg, &facts), Feature::Documents).word(), "on");
        assert_eq!(state(&at(&cfg, &facts), Feature::Ocr).word(), "on");
        assert_eq!(state(&at(&cfg, &facts), Feature::Layout).word(), "off");
    }

    /// Voice's parts read `[voice]` (step 5): dictation names the address it
    /// posts to, an empty `offer_target` reads calls on and names what serve
    /// no longer reaches,
    /// and cloning reads `[voice] voices_dir` — not `[web]`'s, which is
    /// applied into it at load.
    #[test]
    fn the_voice_parts_read_the_voice_table() {
        let mut cfg = Config::default();
        switch_all(&mut cfg, true);
        // The parts need `web` ready, which needs its owner.
        cfg.web.owner_login = Some("owner@example.com".into());
        let facts = |cfg: &Config| Facts {
            config: cfg.clone(),
            ..empty_machine()
        };
        let State::On { detail } = state(&facts(&cfg), Feature::Dictate) else {
            panic!("dictation is on with the default address")
        };
        assert!(
            detail.contains(crate::config::VoiceConfig::DEFAULT_STT_URL),
            "{detail}"
        );
        assert_eq!(state(&facts(&cfg), Feature::Calls).word(), "on");
        // Empty: calls stay on (the facade the worker calls mounts on this
        // row, review of #503 pass 3), and the row names everything serve no
        // longer reaches through the worker.
        cfg.voice.offer_target = Some(" ".into());
        let State::On { detail } = state(&facts(&cfg), Feature::Calls) else {
            panic!("an empty offer target is not calls off")
        };
        for gone in ["no call relay", "no Listen", "no voice previews"] {
            assert!(detail.contains(gone), "{gone}: {detail}");
        }
        let State::Off { reason, .. } = state(&facts(&cfg), Feature::Cloning) else {
            panic!("no voices_dir is cloning off")
        };
        assert!(reason.contains("[voice] voices_dir"), "{reason}");
        cfg.voice.voices_dir = Some("/srv/voices".into());
        assert_eq!(state(&facts(&cfg), Feature::Cloning).word(), "on");
        // Port 0 mounts no facade, and the row says so rather than naming
        // port 0 as a mount (review of #503).
        cfg.voice.voice_port = Some(0);
        let State::On { detail } = state(&facts(&cfg), Feature::Voice) else {
            panic!("voice stays on: `mecha voice-serve` is its own surface")
        };
        assert!(detail.contains("mounts no facade"), "{detail}");
    }

    /// The owner's switch comes first: absent and `false` are both off,
    /// whatever the settings say, and only `true` lets the row's own
    /// settings decide. A part has no switch.
    #[test]
    fn the_switch_decides_before_the_settings() {
        let mut cfg = Config::default();
        cfg.mcp.push(mcp("graph", "mecha-graph-mcp"));
        let facts = |cfg: &Config| Facts {
            config: cfg.clone(),
            ..empty_machine()
        };
        let State::Off { reason, fix } = state(&facts(&cfg), Feature::Graph) else {
            panic!("an absent switch is off")
        };
        assert!(reason.contains("not enabled"), "{reason}");
        assert_eq!(fix.as_deref(), Some("mecha features enable graph"));
        cfg.features.0.insert("graph".into(), false);
        let State::Off { reason, .. } = state(&facts(&cfg), Feature::Graph) else {
            panic!("a false switch is off")
        };
        assert!(reason.contains("turned off"), "{reason}");
        cfg.features.0.insert("graph".into(), true);
        assert_eq!(state(&facts(&cfg), Feature::Graph).word(), "on");
        assert_eq!(switch(&cfg, Feature::Tasks), None);
        assert_eq!(switch(&cfg, Feature::Graph), Some(Switch::On));
        // `messages` reads `[messages] enabled`, where `apply` puts the
        // answer, and like every other switch: unset is unanswered, and
        // `false` is a no, not an unanswered question that setup asks forever.
        assert_eq!(switch(&cfg, Feature::Messages), Some(Switch::Absent));
        cfg.messages.enabled = Some(false);
        assert_eq!(switch(&cfg, Feature::Messages), Some(Switch::Off));
        let State::Off { reason, fix } = state(&facts(&cfg), Feature::Messages) else {
            panic!("a false messages switch is off")
        };
        assert!(reason.contains("turned off"), "{reason}");
        assert_eq!(fix.as_deref(), Some("mecha features enable messages"));
        cfg.messages.enabled = Some(true);
        assert_eq!(switch(&cfg, Feature::Messages), Some(Switch::On));
        assert_eq!(state(&facts(&cfg), Feature::Messages).word(), "on");
    }

    #[test]
    fn unknown_switches_are_reported_not_fatal() {
        let mut cfg = Config::default();
        for k in ["mail", "from_a_newer_build", "ocr"] {
            cfg.features.0.insert(k.into(), true);
        }
        // A part id is not a switch either.
        assert_eq!(unknown_switches(&cfg), vec!["from_a_newer_build", "ocr"]);
    }

    /// The upgrade notice: an absent switch over something set up here that
    /// would work — `On` or `Unready` with its reason (owner, 2026-09-30),
    /// never `Unknown`, never an explicit `false`, never a feature with
    /// nothing set up (FEATURES-DESIGN.md §4.2).
    #[test]
    fn an_upgrade_announces_what_was_set_up_and_would_work() {
        let mut cfg = Config::default();
        cfg.mcp.push(mcp("mail", "mecha-mail"));
        let mut facts = Facts {
            config: cfg.clone(),
            mail_accounts: Some(2),
            ..empty_machine()
        };
        let a = announcement(&facts, Feature::Mail).expect("mail is set up and works");
        assert_eq!(a.caveat, None);
        assert_eq!(a.fix, "mecha features enable mail");
        assert!(
            a.line().contains("configured but not enabled"),
            "{}",
            a.line()
        );
        assert!(all(&facts)
            .iter()
            .any(|r| r.id == Feature::Mail && r.in_use));

        facts.mail_accounts = Some(0);
        let a = announcement(&facts, Feature::Mail).expect("unready is announced");
        assert!(a.caveat.as_deref().unwrap().contains("no account"), "{a:?}");

        facts.mail_accounts = None;
        assert_eq!(
            announcement(&facts, Feature::Mail),
            None,
            "unknown is never announced"
        );

        facts.mail_accounts = Some(1);
        facts.config.features.0.insert("mail".into(), false);
        assert_eq!(
            announcement(&facts, Feature::Mail),
            None,
            "false is an answer"
        );

        facts.config.features.0.insert("mail".into(), true);
        assert_eq!(announcement(&facts, Feature::Mail), None, "already on");

        // Personas and voice need evidence of use, not just "always usable",
        // or a light install is told about them on every start.
        let mut facts = empty_machine();
        assert_eq!(announcement(&facts, Feature::Personas), None);
        assert_eq!(announcement(&facts, Feature::Voice), None);
        facts.personas_stored = Some(true);
        facts.voice_unit_installed = true;
        assert!(announcement(&facts, Feature::Personas).is_some());
        assert!(announcement(&facts, Feature::Voice).is_some());
        // And a part is never announced, having no switch to answer.
        assert!(announcements(&facts).iter().all(|a| a.id.has_switch()));
    }

    #[test]
    fn enabling_refuses_parts_and_names_the_chain() {
        let cfg = Config::default();
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let err = plan_enable(&cfg, &ids(&["ocr"])).unwrap_err();
        assert!(err.contains("part of `documents`"), "{err}");
        assert!(plan_enable(&cfg, &ids(&["nonsense"]))
            .unwrap_err()
            .contains("not a feature"));
        // Incognito is part of the web app, with nothing of its own to enable.
        let err = plan_enable(&cfg, &ids(&["incognito"])).unwrap_err();
        assert!(
            err.contains("part of `web`") && err.contains("enable `web`"),
            "{err}"
        );
        let err = plan_enable(&cfg, &ids(&["calls"])).unwrap_err();
        assert!(err.contains("part of `voice`"), "{err}");
        // And every off row's own fix is a command `enable` accepts — the row
        // must not print one this binary refuses (review of #443).
        let facts = Facts {
            config: cfg.clone(),
            ..empty_machine()
        };
        for &f in Feature::ALL.iter().filter(|f| f.has_switch()) {
            let State::Off { fix: Some(fix), .. } = state(&facts, f) else {
                panic!("{} is off on an empty machine", f.id())
            };
            let args: Vec<String> = fix.split_whitespace().skip(3).map(String::from).collect();
            plan_enable(&cfg, &args).unwrap_or_else(|e| panic!("{fix}: {e}"));
        }
    }

    /// `switched_on` answers as `state` does for the switch half — its own
    /// bool and every need's — so registration cannot hand out a tool
    /// `mecha features` shows `Blocked`; and `enable_command` names every
    /// switch a feature hangs on (review of #445).
    #[test]
    fn switched_on_and_its_command_follow_every_need() {
        let mut cfg = Config::default();
        // An older `enable`'s `incognito = false` is read by nothing: the
        // web app's switch decides (the owner, 2026-10-14).
        cfg.features.0.insert("incognito".into(), false);
        assert!(!switched_on(&cfg, Feature::Incognito), "web is not on");
        cfg.features.0.insert("web".into(), true);
        assert!(switched_on(&cfg, Feature::Incognito));
        assert!(
            unknown_switches(&cfg).is_empty(),
            "a retired switch is not unknown"
        );
        // A part: its parent's switch and its requirement's.
        assert!(!switched_on(&cfg, Feature::Dictate), "voice is not on");
        assert_eq!(
            enable_command(&cfg, Feature::Dictate),
            "mecha features enable voice"
        );
        cfg.features.0.remove("web");
        assert_eq!(
            enable_command(&cfg, Feature::Dictate),
            "mecha features enable web voice"
        );
        cfg.features.0.insert("voice".into(), true);
        cfg.features.0.insert("web".into(), true);
        assert!(switched_on(&cfg, Feature::Dictate));
        // Every command it prints is one `enable` accepts.
        let empty = Config::default();
        for &f in Feature::ALL {
            let cmd = enable_command(&empty, f);
            let args: Vec<String> = cmd.split_whitespace().skip(3).map(String::from).collect();
            plan_enable(&empty, &args).unwrap_or_else(|e| panic!("{}: {cmd}: {e}", f.id()));
        }
    }

    #[test]
    fn a_server_in_a_switched_off_feature_is_refused_by_name() {
        let mut cfg = Config::default();
        let graph = mcp("graph", "/somewhere/mecha-graph-mcp");
        let other = mcp("notes", "python3");
        let why = server_refusal(&cfg, &graph).expect("graph is not switched on");
        assert!(why.contains("mecha features enable graph"), "{why}");
        assert_eq!(server_refusal(&cfg, &other), None, "a server in no feature");
        cfg.features.0.insert("graph".into(), true);
        assert_eq!(server_refusal(&cfg, &graph), None);
        // The publishing server is the front door's other half, gated with
        // its queue since step 3b and refused by the front door's switch.
        let publish = mcp("factory", "factory-publish");
        let why = server_refusal(&cfg, &publish).expect("frontdoor is not switched on");
        assert!(why.contains("mecha features enable frontdoor"), "{why}");
        cfg.features.0.insert("frontdoor".into(), true);
        assert_eq!(server_refusal(&cfg, &publish), None);
    }

    /// The writer edits in place: comments, other tables and a newer build's
    /// unknown key survive; a missing file and a missing table are created; a
    /// file that is not TOML is left alone.
    #[test]
    fn switches_are_written_in_place() {
        let dir = std::env::temp_dir().join(format!("mecha-switches-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "# the owner's note\ndefault_provider = \"local\"\n\n[features]\nfrom_a_newer_build = true\n",
        )
        .unwrap();
        write_switches(&path, &[(Feature::Mail, true), (Feature::Web, false)]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# the owner's note"), "{text}");
        assert!(text.contains("from_a_newer_build = true"), "{text}");
        let cfg: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg.features.get("mail"), Some(true));
        assert_eq!(cfg.features.get("web"), Some(false));
        assert_eq!(cfg.default_provider, "local");

        let fresh = dir.join("fresh.toml");
        write_switches(&fresh, &[(Feature::Graph, true)]).unwrap();
        let cfg: Config = toml::from_str(&std::fs::read_to_string(&fresh).unwrap()).unwrap();
        assert_eq!(cfg.features.get("graph"), Some(true));

        let broken = dir.join("broken.toml");
        std::fs::write(&broken, "this is [not toml").unwrap();
        assert!(write_switches(&broken, &[(Feature::Mail, true)]).is_err());
        assert_eq!(
            std::fs::read_to_string(&broken).unwrap(),
            "this is [not toml"
        );
        assert!(
            write_switches(&fresh, &[(Feature::Ocr, true)]).is_err(),
            "a part"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Only the front door may be switched on from an experiment environment
    /// (FEATURES-DESIGN.md §5.1): the rest have operator-only tables, the
    /// operator's credentials, machine tables, or — `graph` — a server a
    /// manifest can carry in from the operator.
    #[test]
    fn only_the_front_door_is_switchable_from_an_environment() {
        let on: Vec<_> = Feature::ALL
            .iter()
            .filter(|f| f.switchable_from_environment())
            .collect();
        assert_eq!(on, vec![&Feature::Frontdoor]);
    }

    #[test]
    fn the_json_row_is_flat_and_names_the_state() {
        let rows = all(&Facts::default());
        let v = serde_json::to_value(&rows[0]).unwrap();
        assert_eq!(v["id"], "web");
        assert_eq!(v["state"], "off");
        assert!(v["fix"].is_string());
        let tasks =
            serde_json::to_value(rows.iter().find(|r| r.id == Feature::Tasks).unwrap()).unwrap();
        assert_eq!(tasks["part_of"], "graph");
        assert_eq!(tasks["on"], "graph");
    }
}
