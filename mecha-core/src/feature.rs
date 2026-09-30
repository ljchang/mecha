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
//! **Step 0 reports the switches that exist today**, not the ones the design
//! proposes. Where a feature has no switch yet — personas, voice — it says
//! so in its detail rather than inventing one.

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
    /// Configured, and a fact on disk says it cannot work yet.
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
            config: global.clone(),
        }
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
}

/// Every feature, in [`Feature::ALL`] order.
pub fn all(facts: &Facts) -> Vec<Row> {
    Feature::ALL
        .iter()
        .map(|&f| Row {
            id: f,
            label: f.label(),
            part_of: f.part_of(),
            requires: f.requires(),
            state: state(facts, f),
        })
        .collect()
}

/// Is `f` on? The only place that question is answered.
///
/// Every row reads [`Facts::config`], the global configuration, and
/// nothing else configures it.
///
/// A feature whose parent or requirement is off is `Blocked` on the first
/// one found, whatever its own switch says: configured-but-unreachable is
/// different from off, and this says which dependency to fix first.
pub fn state(facts: &Facts, f: Feature) -> State {
    for need in f.needs() {
        if !state(facts, need).satisfies() {
            return State::Blocked { on: need };
        }
    }
    own_state(facts, f)
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
        // Reached only when `[documents]` is present: `state` checked it.
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
        Feature::Personas => on("no switch yet — always on (FEATURES-DESIGN.md F1)"),
        Feature::Voice => {
            on("no switch yet — `mecha voice-serve`, and `mecha serve`'s voice flags")
        }
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

    /// `facts` read against `cfg` — the tests' way of saying "this global
    /// configuration on this machine".
    fn at(cfg: &Config, facts: &Facts) -> Facts {
        Facts {
            config: cfg.clone(),
            ..facts.clone()
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
    /// nothing but what has no switch yet — and nothing reads as broken.
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
            Feature::Messages,
        ] {
            assert_eq!(get(&rows, f).word(), "off", "{}", f.id());
        }
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
        // Voice has a surface without the web app (`mecha voice-serve`);
        // the parts that live in `mecha serve` wait on it.
        assert_eq!(get(&rows, Feature::Voice).word(), "on");
        assert_eq!(
            get(&rows, Feature::Dictate),
            &State::Blocked { on: Feature::Web }
        );
        assert_eq!(
            get(&rows, Feature::Cloning),
            &State::Blocked { on: Feature::Web }
        );
        assert_eq!(
            get(&rows, Feature::Library),
            &State::Blocked { on: Feature::Image }
        );
    }

    /// An installed binary with no `[[mcp]]` entry is off: the entry is what
    /// puts the tools on the surface, and setup's `mail` step checks only
    /// the binary (FEATURES-DESIGN.md §1.1).
    #[test]
    fn a_server_is_on_only_through_an_enabled_mcp_entry() {
        let mut cfg = Config::default();
        let mut facts = Facts {
            has_mail_binary: true,
            mail_accounts: Some(2),
            ..Facts::default()
        };
        let State::Off { reason, .. } = state(&at(&cfg, &facts), Feature::Mail) else {
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
        assert_eq!(state(&at(&cfg, &facts), Feature::Mail).word(), "off");
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
        let empty = Facts::read(Path::new("/nonexistent"), &Config::default());
        assert_eq!(state(&empty, Feature::Graph).word(), "off");
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
        facts.has_factory_binary = false;
        assert_eq!(
            state(&at(&cfg, &facts), Feature::Publishing),
            State::Blocked {
                on: Feature::Frontdoor
            }
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
        assert_eq!(state(&at(&cfg, &facts), Feature::Documents).word(), "off");

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
