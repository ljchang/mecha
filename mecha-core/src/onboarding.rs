//! What a new install still needs, and the one command that fixes each.
//!
//! Two halves, split the way `compact.rs` and `candidate.rs` are split, and
//! for the same reason: getting onboarding wrong is *silent*. A missing
//! `context_window` does not error, it makes a long run die at a threshold
//! nobody set; a missing `vision` does not error, it makes every screenshot
//! arrive as a line of text. So the deciding is a pure function over
//! already-gathered facts and is unit-tested without a machine, and the
//! part that touches the world is thin enough to read.
//!
//! **The rule that makes this worth having: never write down a number the
//! user believes.** `GET /props` reports the served alias, the per-slot
//! `n_ctx` and whether a projector is loaded. Writing config from *that*
//! retires a whole class of bug — `context_window` naming `-c` instead of
//! `-c / -np`, `vision` unset against a multimodal model, `model` naming
//! weights the server is not serving — none of which anything can detect
//! later, because each one degrades quietly rather than failing.
//!
//! It is the same argument `Sandbox::preflight` makes, one step earlier:
//! ask the thing itself rather than trusting what was configured about it.

use crate::config::Config;
use crate::doctor::Remedy;
use serde::Serialize;
use std::path::Path;

/// Where a step stands. Deliberately three-valued: "cannot tell from here"
/// is a real answer and must not be printed as "not done" — a person told
/// their mail is unconfigured, when it is merely unreadable from this
/// process, goes and re-runs an OAuth flow they did not need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Done,
    Missing,
    /// Configured, but something about it disagrees with reality.
    Wrong,
    Unknown,
}

/// One thing a new install might still need.
#[derive(Debug, Clone, Serialize)]
pub struct Step {
    /// Stable slug, so `--json` output can be matched on.
    pub id: String,
    pub title: String,
    pub status: Status,
    pub detail: String,
    /// Optional because plenty of steps are somebody else's to do — picking
    /// a model, deciding whether they want mail at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remedy: Option<Remedy>,
}

impl Step {
    fn new(id: &str, title: &str, status: Status, detail: impl Into<String>) -> Self {
        Step {
            id: id.into(),
            title: title.into(),
            status,
            detail: detail.into(),
            remedy: None,
        }
    }
    fn with(mut self, description: &str, argv: &[&str], needs_terminal: bool) -> Self {
        self.remedy = Some(Remedy {
            description: description.into(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            needs_terminal,
        });
        self
    }
}

/// Everything the impure half gathered, so [`plan`] can stay a function.
///
/// A struct rather than a pile of arguments because it is going to grow, and
/// because a caller that has to remember the order of six booleans will
/// eventually get one wrong in a way that reads as a working install.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// Which helper binaries are on `PATH`. A crates.io install gets each
    /// from its own `cargo install`, so "is it in the workspace" is the
    /// wrong question — presence on `PATH` is the one that matters.
    pub has_mail_binary: bool,
    pub has_docs_binary: bool,
    pub has_graph_binary: bool,
    /// Whether any account/credential store has something in it. `None`
    /// where the directory could not be read — see [`Status::Unknown`].
    pub mail_accounts: Option<usize>,
    pub docs_accounts: Option<usize>,
    pub slack_linked: Option<bool>,
    /// What the default provider's server said about itself, when it is
    /// local and answered.
    pub props: Option<crate::provider::preflight::Props>,
    /// Whether the configured provider has a usable credential.
    pub provider_credential: bool,
    /// Whether a trigger scheduler is running or installed.
    pub scheduler_installed: bool,
    pub trigger_count: usize,
}

/// What still needs doing. Empty means a complete install.
///
/// Ordered by what blocks what: a provider that cannot answer makes every
/// step below it untestable, so it comes first, and integrations come before
/// scheduling because an unattended run with nothing wired to it is a cron
/// slot that prints "no mail configured" every morning.
pub fn plan(cfg: &Config, provider_name: &str, facts: &Facts) -> Vec<Step> {
    let mut steps = Vec::new();
    let local = cfg
        .providers
        .get(provider_name)
        .filter(|p| p.kind == "local");

    // --- 1. can anything answer at all
    if !facts.provider_credential && local.is_none() {
        steps.push(
            Step::new(
                "provider-credential",
                "A provider that can answer",
                Status::Missing,
                format!(
                    "`{provider_name}` has no usable credential. Set the environment variable \
                     its `api_key_env` names, or configure a local server instead."
                ),
            )
            .with(
                "Show which providers are configured and what each is missing.",
                &["mecha", "config", "show"],
                false,
            ),
        );
    }

    // --- 2. a local server, checked against itself
    if let Some(pcfg) = local {
        match &facts.props {
            None => steps.push(Step::new(
                "local-server",
                "The local server is reachable",
                Status::Missing,
                format!(
                    "Nothing answered at {}. Start the server before the rest of this can be \
                     checked — every value below is read back from it rather than guessed.",
                    pcfg.base_url.as_deref().unwrap_or("(no base_url)")
                ),
            )),
            Some(props) => {
                let mismatches =
                    crate::provider::preflight::disagreements(provider_name, pcfg, props);
                if mismatches.is_empty() {
                    steps.push(Step::new(
                        "local-server",
                        "The local server agrees with the config",
                        Status::Done,
                        format!(
                            "serving {}, {} tokens per slot, vision {}",
                            props.model_alias.as_deref().unwrap_or("(unnamed)"),
                            props
                                .default_generation_settings
                                .n_ctx
                                .map(|n| n.to_string())
                                .unwrap_or_else(|| "?".into()),
                            if props.modalities.vision { "on" } else { "off" },
                        ),
                    ));
                } else {
                    steps.push(
                        Step::new(
                            "local-server",
                            "The config disagrees with what is served",
                            Status::Wrong,
                            mismatches.join("\n\n"),
                        )
                        .with(
                            "Rewrite these from what the server reports, rather than editing \
                             them by hand.",
                            &["mecha", "setup", "--write"],
                            false,
                        ),
                    );
                }
            }
        }
    }

    steps.extend(integration_steps(facts));

    // --- 4. scheduling, and nothing is turned on for anyone
    //
    // A scheduled unattended agent run on a machine holding your mail is
    // never a default — the same argument that keeps `[[trigger]]` out of a
    // project's `mecha.toml`, where a cloned repository would be handing
    // itself a cron slot. What is offered is the *scheduler*, not a schedule.
    if !facts.scheduler_installed && facts.trigger_count > 0 {
        steps.push(
            Step::new(
                "scheduler",
                "Something to fire the triggers",
                Status::Missing,
                format!(
                    "{} trigger(s) are defined and nothing is running them. Being due is a \
                     function of the ledger and the clock, so any of a systemd timer, a \
                     crontab line running `mecha trigger tick`, or `mecha trigger daemon` \
                     will do.",
                    facts.trigger_count
                ),
            )
            .with(
                "Print a systemd user unit for the daemon, to review before installing.",
                &["mecha", "trigger", "daemon", "--print-unit"],
                false,
            ),
        );
    }

    steps
}

/// The integrations, each detected the same way: is the binary there, and has
/// anything been authorised through it.
///
/// **mecha's own integrations are offered as commands to run; the graph's are
/// only ever named.** mecha reaches the knowledge graph through the MCP tool
/// surface and nothing else — no dependency, no second reader of its store —
/// and a setup flow that drove `mecha-graph source add` would be exactly the
/// coupling that rule exists to prevent. So a graph source is a sentence
/// pointing at that project's own CLI, never a command this one spawns.
fn integration_steps(facts: &Facts) -> Vec<Step> {
    let mut steps = Vec::new();

    steps.push(match (facts.has_mail_binary, facts.mail_accounts) {
        (false, _) => Step::new(
            "mail",
            "Mail and calendar",
            Status::Missing,
            "`mecha-mail` is not on PATH. It is a separate crate, and optional — nothing else \
             needs it.",
        )
        .with(
            "Install the mail and calendar MCP servers.",
            &["cargo", "install", "mecha-mail", "--locked"],
            false,
        ),
        (true, Some(0)) => Step::new(
            "mail",
            "Mail and calendar",
            Status::Missing,
            "`mecha-mail` is installed with no accounts authorised. The model names an \
             *account*, never a provider, so add one per mailbox.",
        )
        .with(
            "Authorise a mailbox. Needs a browser, or `--paste` over SSH.",
            &["mecha-mail", "auth", "personal", "--provider", "google"],
            true,
        ),
        (true, Some(n)) => Step::new(
            "mail",
            "Mail and calendar",
            Status::Done,
            format!("{n} account(s) authorised"),
        ),
        (true, None) => Step::new(
            "mail",
            "Mail and calendar",
            Status::Unknown,
            "`mecha-mail` is installed; its credential store could not be read from here.",
        ),
    });

    steps.push(match (facts.has_docs_binary, facts.docs_accounts) {
        (false, _) => Step::new(
            "docs",
            "Google Docs, Sheets and Slides",
            Status::Missing,
            "`mecha-docs` ships with the mail crate. Under `drive.file` it reaches only files \
             it created or you handed it in Google's own picker — which is the reason to want \
             it, and no instruction inside a run can widen that.",
        )
        .with(
            "Install the documents server (same crate as mail).",
            &["cargo", "install", "mecha-mail", "--locked"],
            false,
        ),
        (true, Some(0)) => Step::new(
            "docs",
            "Google Docs, Sheets and Slides",
            Status::Missing,
            "`mecha-docs` is installed with no account authorised.",
        )
        .with(
            "Authorise Drive access. Use `--paste` if there is no browser here.",
            &["mecha-docs", "auth", "personal"],
            true,
        ),
        (true, Some(n)) => Step::new(
            "docs",
            "Google Docs, Sheets and Slides",
            Status::Done,
            format!("{n} account(s) authorised"),
        ),
        (true, None) => Step::new(
            "docs",
            "Google Docs, Sheets and Slides",
            Status::Unknown,
            "installed; the credential store could not be read from here.",
        ),
    });

    steps.push(match facts.slack_linked {
        Some(true) => Step::new(
            "slack",
            "Slack as a remote control",
            Status::Done,
            "linked to a workspace",
        ),
        Some(false) => Step::new(
            "slack",
            "Slack as a remote control",
            Status::Missing,
            "Watch a run from a phone, approve what it wants to send, and hand files in and \
             out. The owner is bound by a nonce printed on this machine, so proving shell \
             access here is what claims it.",
        )
        .with(
            "Start the Slack setup, which prints the binding nonce.",
            &["mecha", "slack", "auth"],
            true,
        ),
        None => Step::new(
            "slack",
            "Slack as a remote control",
            Status::Unknown,
            "the binding store could not be read from here.",
        ),
    });

    steps.push(if facts.has_graph_binary {
        Step::new(
            "graph",
            "The personal knowledge graph",
            Status::Done,
            "`mecha-graph-mcp` is on PATH. Its own sources — ambient conversations, a \
             calendar ICS feed, Slack, messages, mail — are configured with `mecha-graph \
             source`, in that project. mecha reaches the graph only through its MCP tools \
             and deliberately knows nothing else about it.",
        )
    } else {
        Step::new(
            "graph",
            "The personal knowledge graph",
            Status::Missing,
            "Memory: who people are, what happened when. A separate project, wired in as an \
             MCP server whose reads are marked untrusted — a graph fed by mail and messages \
             holds third-party text by construction.",
        )
        .with(
            "Install the graph's MCP server.",
            &["cargo", "install", "mecha-graph-mcp", "--locked"],
            false,
        )
    });

    steps
}

/// The values a local server reports about itself, ready to be written down.
///
/// Returned rather than applied, so the caller can show them before changing
/// anything: this rewrites settings a person may have had reasons for, and
/// "here is what I would write" is a different act from writing it.
pub fn verified_settings(props: &crate::provider::preflight::Props) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(alias) = &props.model_alias {
        out.push(("model", format!("{alias:?}")));
    }
    // The per-slot figure, which is the one a request actually gets. Reading
    // it back is what makes `-c` versus `-c / -np` a non-question.
    if let Some(n) = props.default_generation_settings.n_ctx {
        out.push(("context_window", n.to_string()));
    }
    out.push(("vision", props.modalities.vision.to_string()));
    out
}

/// Count the per-account directories under a credential root.
///
/// `None` when the root cannot be read *and does not simply not exist* — an
/// absent directory is a confident zero, where an unreadable one is genuinely
/// unknown and must not be reported as "no accounts".
pub fn count_accounts(root: &Path) -> Option<usize> {
    match std::fs::read_dir(root) {
        Ok(entries) => Some(
            entries
                .flatten()
                .filter(|e| e.path().join("oauth.json").is_file())
                .count(),
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(0),
        Err(_) => None,
    }
}

/// Is `name` runnable from `PATH`?
pub fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::preflight::{GenerationSettings, Modalities, Props};

    fn cfg_with_local(context_window: u64, vision: Option<bool>) -> Config {
        let mut cfg = Config::default();
        let mut p = cfg.providers.get("anthropic").cloned().unwrap();
        p.kind = "local".into();
        p.model = Some("qwen3.6-35b-a3b".into());
        p.base_url = Some("http://127.0.0.1:8080".into());
        p.api_key_env = None;
        p.context_window = Some(context_window);
        p.vision = vision;
        cfg.providers.insert("local".into(), p);
        cfg
    }

    fn props(n_ctx: u64, slots: u64, vision: bool) -> Props {
        Props {
            model_alias: Some("qwen3.6-35b-a3b".into()),
            total_slots: Some(slots),
            modalities: Modalities { vision },
            default_generation_settings: GenerationSettings { n_ctx: Some(n_ctx) },
        }
    }

    fn facts(props: Option<Props>) -> Facts {
        Facts {
            provider_credential: true,
            props,
            mail_accounts: Some(1),
            docs_accounts: Some(1),
            slack_linked: Some(true),
            has_mail_binary: true,
            has_docs_binary: true,
            has_graph_binary: true,
            scheduler_installed: true,
            trigger_count: 0,
        }
    }

    fn step<'a>(steps: &'a [Step], id: &str) -> &'a Step {
        steps.iter().find(|s| s.id == id).expect("step missing")
    }

    /// A complete install has nothing to say about itself.
    #[test]
    fn everything_configured_and_agreeing_reports_no_work() {
        let cfg = cfg_with_local(262144, Some(true));
        let steps = plan(&cfg, "local", &facts(Some(props(262144, 4, true))));
        assert!(
            steps.iter().all(|s| s.status == Status::Done),
            "unexpected work: {:?}",
            steps
                .iter()
                .filter(|s| s.status != Status::Done)
                .map(|s| &s.id)
                .collect::<Vec<_>>()
        );
    }

    /// The trap this exists to retire: `context_window` naming `-c` rather
    /// than `-c / -np`. The two are the same number until `-np` moves off 1,
    /// which is what makes it easy to write down wrong and impossible to
    /// notice afterwards.
    #[test]
    fn a_context_window_that_names_c_rather_than_c_over_np_is_reported_wrong() {
        let cfg = cfg_with_local(1048576, Some(true));
        let steps = plan(&cfg, "local", &facts(Some(props(262144, 4, true))));
        let s = step(&steps, "local-server");
        assert_eq!(s.status, Status::Wrong);
        assert!(s.detail.contains("262144"), "{}", s.detail);
        assert!(s.remedy.is_some(), "and it is fixable without hand-editing");
    }

    /// The bug that hid for months, in the direction nobody looks.
    #[test]
    fn a_vision_model_nobody_configured_to_use_is_reported_wrong() {
        let cfg = cfg_with_local(262144, None); // vision unset → false for local
        let steps = plan(&cfg, "local", &facts(Some(props(262144, 4, true))));
        assert_eq!(step(&steps, "local-server").status, Status::Wrong);
    }

    /// A server that is simply not running must not be reported as a
    /// misconfiguration — there is nothing to compare against yet, and
    /// telling someone their config is wrong when it may be fine sends them
    /// editing a correct file.
    #[test]
    fn a_server_that_is_not_up_is_missing_rather_than_wrong() {
        let cfg = cfg_with_local(262144, Some(true));
        let steps = plan(&cfg, "local", &facts(None));
        assert_eq!(step(&steps, "local-server").status, Status::Missing);
        assert!(step(&steps, "local-server").remedy.is_none());
    }

    /// "Cannot tell from here" is not "not done". A person told their mail is
    /// unconfigured, when the store is merely unreadable, re-runs an OAuth
    /// flow they did not need.
    #[test]
    fn an_unreadable_store_is_unknown_and_offers_nothing() {
        let mut f = facts(Some(props(262144, 4, true)));
        f.mail_accounts = None;
        let steps = plan(&cfg_with_local(262144, Some(true)), "local", &f);
        let s = step(&steps, "mail");
        assert_eq!(s.status, Status::Unknown);
        assert!(s.remedy.is_none(), "unknown must not propose a fix");
    }

    /// The boundary that keeps mecha from growing a second way to reach the
    /// graph. Its sources belong to that project's CLI, and this one must
    /// never spawn them.
    #[test]
    fn a_graph_step_never_offers_to_run_a_graph_source_command() {
        let steps = plan(
            &cfg_with_local(262144, Some(true)),
            "local",
            &facts(Some(props(262144, 4, true))),
        );
        for s in &steps {
            if let Some(r) = &s.remedy {
                assert!(
                    !r.argv.iter().any(|a| a == "source"),
                    "{} would drive the graph's own source CLI: {:?}",
                    s.id,
                    r.argv
                );
            }
        }
    }

    /// Nothing schedules anything for anyone. A cron slot on a machine
    /// holding your mail is never a default, and the offer is the *runner*,
    /// never a schedule.
    #[test]
    fn a_scheduler_is_only_offered_once_a_trigger_exists() {
        let cfg = cfg_with_local(262144, Some(true));
        let mut f = facts(Some(props(262144, 4, true)));
        f.scheduler_installed = false;

        f.trigger_count = 0;
        assert!(
            !plan(&cfg, "local", &f).iter().any(|s| s.id == "scheduler"),
            "no triggers means nothing to run; do not offer a runner"
        );

        f.trigger_count = 2;
        let steps = plan(&cfg, "local", &f);
        let s = step(&steps, "scheduler");
        assert_eq!(s.status, Status::Missing);
        assert!(
            !s.remedy.as_ref().unwrap().argv.contains(&"add".to_string()),
            "offer the runner, never a schedule"
        );
    }

    /// What gets written comes off the wire, not out of the config.
    #[test]
    fn verified_settings_are_read_back_from_the_server() {
        let got = verified_settings(&props(65536, 4, true));
        assert!(got.contains(&("context_window", "65536".into())), "{got:?}");
        assert!(got.contains(&("vision", "true".into())), "{got:?}");
        assert!(
            got.iter().any(|(k, v)| *k == "model" && v.contains("qwen")),
            "{got:?}"
        );
    }

    /// An absent directory is a confident zero; an unreadable one is not.
    #[test]
    fn a_missing_credential_root_counts_zero_rather_than_unknown() {
        assert_eq!(count_accounts(Path::new("/no/such/root")), Some(0));
    }
}
