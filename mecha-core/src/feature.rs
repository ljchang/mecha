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
    Personas,
    Voice,
    Dictate,
    Calls,
    Cloning,
    Incognito,
    Frontdoor,
    Publishing,
    Messages,
}

impl Feature {
    pub const ALL: &'static [Feature] = &[
        Feature::Web,
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
        Feature::Personas,
        Feature::Voice,
        Feature::Dictate,
        Feature::Calls,
        Feature::Cloning,
        Feature::Incognito,
        Feature::Frontdoor,
        Feature::Publishing,
        Feature::Messages,
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
            Feature::Personas => "personas",
            Feature::Voice => "voice",
            Feature::Dictate => "dictate",
            Feature::Calls => "calls",
            Feature::Cloning => "cloning",
            Feature::Incognito => "incognito",
            Feature::Frontdoor => "frontdoor",
            Feature::Publishing => "publishing",
            Feature::Messages => "messages",
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
            Feature::Personas => "Personas",
            Feature::Voice => "Voice",
            Feature::Dictate => "Dictation",
            Feature::Calls => "Voice calls",
            Feature::Cloning => "Voice cloning",
            Feature::Incognito => "Incognito chat",
            Feature::Frontdoor => "Front door (inbound requests)",
            Feature::Publishing => "Publishing tools",
            Feature::Messages => "Messages between sessions",
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
            Feature::Incognito | Feature::Dictate | Feature::Calls | Feature::Cloning => {
                &[Feature::Web]
            }
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
    /// are unregistered, its server is not started, or its surface refuses.
    /// The rest (Slack, personas, voice, incognito, the front door — its queue
    /// and its publishing server together) wait for the route and verb guards
    /// of FEATURES-DESIGN.md §9 step 3, and
    /// the upgrade notice must not call them off while they work (found on
    /// review of #445). Exhaustive: a step that gates one flips its arm.
    pub fn gated(self) -> bool {
        match self {
            Feature::Web
            | Feature::Mail
            | Feature::Docs
            | Feature::Graph
            | Feature::Search
            | Feature::Documents
            | Feature::Image => true,
            Feature::Slack
            | Feature::Personas
            | Feature::Voice
            | Feature::Incognito
            | Feature::Frontdoor
            | Feature::Messages => false,
            Feature::Tasks
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
            // `default_provider` and `providers` are machine tables.
            | Feature::Incognito
            // A manifest's `live_servers` can carry the operator's graph in,
            // and the row cannot tell that entry from a declared one;
            // `config_at` defaults this bool instead (FEATURES-DESIGN §5.1).
            | Feature::Graph => false,
            // Parts have no switch to set.
            Feature::Tasks
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
/// `[features] messages` answer (`config::FeaturesConfig`); its default is
/// `false`, which reads as `Absent` — nothing distinguishes it from an
/// unanswered key, and messaging is never announced anyway.
pub fn switch(cfg: &Config, f: Feature) -> Option<Switch> {
    if !f.has_switch() {
        return None;
    }
    if f == Feature::Messages {
        return Some(if cfg.messages.enabled {
            Switch::On
        } else {
            Switch::Absent
        });
    }
    Some(match cfg.features.get(f.id()) {
        Some(true) => Switch::On,
        Some(false) => Switch::Off,
        None => Switch::Absent,
    })
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
        // Not `factory-publish` yet: gating the publishing server while the
        // front door's queue stays ungated made the notice call the front
        // door "still working" with half of it gone. Both halves follow the
        // switch together in §9 step 3 (found on review of #445).
        _ => None,
    }
}

/// Keys in `[features]` this build does not know — a newer build's feature,
/// or a typo. Ignored, never a load failure (see `config::FeaturesConfig`),
/// and reported so a typo still shows as the feature it meant being off.
pub fn unknown_switches(cfg: &Config) -> Vec<String> {
    cfg.features
        .0
        .keys()
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
    /// (`mecha-voice-serve.service` or `mecha-voice-worker.service`).
    /// Installed, not running: a socket-activated unit is idle until asked.
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
            count_accounts, docs_store_dir, mail_store_dir, on_path, slack_linked,
        };
        Facts {
            has_mail_binary: on_path("mecha-mail"),
            has_docs_binary: on_path("mecha-docs"),
            has_graph_binary: on_path("mecha-graph-mcp"),
            has_factory_binary: on_path("factory-publish"),
            mail_accounts: mail_store_dir().and_then(|d| count_accounts(&d)),
            docs_accounts: docs_store_dir().and_then(|d| count_accounts(&d)),
            slack_linked: slack_linked(home),
            personas_stored: personas_stored(&home.join("personas")),
            voice_unit_installed: dirs::config_dir().is_some_and(|d| {
                let units = d.join("systemd/user");
                ["mecha-voice-serve.service", "mecha-voice-worker.service"]
                    .iter()
                    .any(|u| units.join(u).is_file())
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
    for need in f.needs() {
        if !state(facts, need).satisfies() {
            return State::Blocked { on: need };
        }
    }
    match own_state(facts, f) {
        // Past the switch, a feature with one was switched on, so its own
        // `off` means "said yes, not set up yet": `Unready`, shown with the
        // fix, never hidden (FEATURES-DESIGN.md §5). A part's `off` stays —
        // `[documents] ocr = false` is the owner's own no. Harmless while
        // nothing hid a row; the web app hiding `Off` is what made it matter.
        State::Off { reason, fix } if f.has_switch() => State::Unready { reason, fix },
        own => own,
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
    match state(facts, f) {
        State::Off { fix, .. } | State::Unready { fix, .. } => fix,
        State::Blocked { .. } if !switched_on(&facts.config, f) => {
            Some(enable_command(&facts.config, f))
        }
        State::Blocked { on } => fix(facts, on),
        State::On { .. } | State::Unknown { .. } => None,
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
        Feature::Personas => facts.personas_stored == Some(true),
        Feature::Voice => facts.voice_unit_installed,
        Feature::Frontdoor => facts.has_factory_binary,
        Feature::Incognito | Feature::Messages => false,
        Feature::Tasks
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
        if switch(&facts.config, g) == Some(Switch::Absent) && g != Feature::Messages {
            assumed.config.features.0.insert(g.id().to_string(), true);
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
/// disable that would break another plugin — `enable incognito` alone would
/// land in `Blocked`.
pub fn plan_enable(cfg: &Config, ids: &[String]) -> Result<Vec<Feature>, String> {
    let mut wanted = Vec::new();
    for id in ids {
        let f = Feature::parse(id)
            .ok_or_else(|| format!("`{id}` is not a feature — `mecha features` lists them"))?;
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
    let table = doc
        .entry("features")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
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

/// The shared shape of the three features that are an MCP server.
fn server_state(
    facts: &Facts,
    program: &str,
    installed: bool,
    install: &str,
    accounts: Option<Option<usize>>,
    authorise: &str,
) -> State {
    let Some(entry) = mcp_entry(facts, program) else {
        return if installed {
            off(
                format!("`{program}` is installed, but no [[mcp]] entry runs it"),
                format!("add an [[mcp]] entry with command = \"{program}\""),
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
            facts,
            "mecha-mail",
            facts.has_mail_binary,
            "cargo install mecha-mail --locked",
            Some(facts.mail_accounts),
            "mecha-mail auth <name> --provider google",
        ),
        Feature::Docs => server_state(
            facts,
            "mecha-docs",
            facts.has_docs_binary,
            "cargo install mecha-mail --locked",
            Some(facts.docs_accounts),
            "mecha-docs auth",
        ),
        Feature::Graph => server_state(
            facts,
            "mecha-graph-mcp",
            facts.has_graph_binary,
            "cargo install mecha-graph-mcp --locked",
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
        // parent is `Unready`, which blocks nothing — so the part says the
        // table is missing, never a setting that is not there to be false.
        Feature::Ocr if cfg.documents.is_none() => {
            off("no [documents] table", "add a [documents] table")
        }
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
        Feature::Library if !cfg.tools.registers("image_library") => tools_off("image_library"),
        Feature::Library => match crate::imagelib::Library::default_dir() {
            Ok(dir) => on(dir.display().to_string()),
            Err(e) => State::Unknown {
                reason: format!("the library's directory: {e:#}"),
            },
        },
        Feature::Personas => on("~/.mecha/personas"),
        Feature::Voice => on("`mecha voice-serve`, and `mecha serve`'s voice flags"),
        Feature::Dictate => {
            on("the web app's speech to text, at a fixed address — not yet configurable")
        }
        Feature::Calls => on("`mecha serve`'s --offer-target"),
        Feature::Cloning => match &cfg.web.voices_dir {
            Some(dir) => on(dir.display().to_string()),
            None => off(
                "no [web] voices_dir",
                "set [web] voices_dir to the directory the TTS container mounts as /voices",
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
                "install factory-publish from the mecha-factory repository",
            ),
        },
        // The model's half: publishing tools, from an `[[mcp]]` entry.
        Feature::Publishing => server_state(
            facts,
            "factory-publish",
            facts.has_factory_binary,
            "install factory-publish from the mecha-factory repository",
            None,
            "",
        ),
        Feature::Messages => match cfg.messages.enabled {
            true => on("[messages] enabled"),
            false => off(
                "[messages] enabled = false",
                "set [messages] enabled = true",
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            if f.has_switch() && f != Feature::Messages {
                cfg.features.0.insert(f.id().to_string(), on);
            }
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
                | Feature::Personas
                | Feature::Voice
                | Feature::Dictate
                | Feature::Calls
                | Feature::Cloning
                | Feature::Incognito
                | Feature::Frontdoor
                | Feature::Publishing
                | Feature::Messages => {}
            }
        }
        // The match above lists 21 — raise this with it, and a variant
        // added to the match but not to `ALL` fails here.
        assert_eq!(Feature::ALL.len(), 21);
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
            Feature::Incognito,
            Feature::Frontdoor,
            Feature::Messages,
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
        // whose own address is fine, is still reported rather than hidden.
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
        assert_eq!(state(&at(&cfg, &facts), Feature::Ocr).word(), "on");
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
        // the install command — and does not block its part, whose own
        // entry answers for it.
        facts.has_factory_binary = false;
        assert_eq!(
            state(&at(&cfg, &facts), Feature::Frontdoor).word(),
            "unready"
        );
        assert_eq!(state(&at(&cfg, &facts), Feature::Publishing).word(), "on");
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
        // Its part is not blocked by an unready parent.
        assert!(row(Feature::Library).shown);
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
        // A part's own off is the owner's no, and stays off.
        cfg.tools.disabled = vec!["image_library".into()];
        assert_eq!(state(&at(&cfg, &facts), Feature::Library).word(), "off");
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
        // `messages` reads `[messages] enabled`, where `apply` puts the answer.
        cfg.messages.enabled = true;
        assert_eq!(switch(&cfg, Feature::Messages), Some(Switch::On));
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
        let err = plan_enable(&cfg, &ids(&["incognito"])).unwrap_err();
        assert!(err.contains("mecha features enable web incognito"), "{err}");
        assert_eq!(
            plan_enable(&cfg, &ids(&["web", "incognito"])).unwrap(),
            vec![Feature::Web, Feature::Incognito]
        );
        let mut web_on = cfg.clone();
        web_on.features.0.insert("web".into(), true);
        assert_eq!(
            plan_enable(&web_on, &ids(&["incognito"])).unwrap(),
            vec![Feature::Incognito]
        );
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
        cfg.features.0.insert("incognito".into(), true);
        assert!(!switched_on(&cfg, Feature::Incognito), "web is not on");
        cfg.features.0.insert("web".into(), true);
        assert!(switched_on(&cfg, Feature::Incognito));
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
        // The publishing server is not gated yet: the front door is gated
        // whole in step 3, never one half before the other.
        let publish = mcp("factory", "factory-publish");
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
