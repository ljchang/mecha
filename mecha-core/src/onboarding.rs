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
use std::path::{Path, PathBuf};

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
    /// The owner said they do not want this one.
    ///
    /// **Not a fifth shade of "not done" — its opposite.** Every other status
    /// here describes the machine; this one describes a decision, and the two
    /// were indistinguishable until it existed: somebody who does not use
    /// Slack read `not set up` on every `mecha setup` forever, and every
    /// scripted run exited non-zero over a choice they had already made. The
    /// same "a dash is never zero" rule [`crate::backlog`] states, one noun
    /// over — an absence of the thing and a decision against the thing are
    /// different findings, and a reader that cannot tell them apart turns a
    /// finished install into a permanent defect list.
    Declined,
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
    /// May the owner say they never want this?
    ///
    /// **A property of the step, not of its status**, and the distinction is
    /// load-bearing: inferring it from `Missing` made *"a provider that can
    /// answer"* declinable, so a person could decline the one thing without
    /// which nothing runs and be told `Nothing outstanding.` on an install
    /// that could not answer a single prompt. Found by running the flow
    /// rather than by reading it.
    ///
    /// True only where "I don't want this" is a coherent sentence — the
    /// integrations, and the charter. Never for a credential, a server that
    /// disagrees with its config, or anything else that is the machine being
    /// wrong rather than a feature going unused.
    pub optional: bool,
    /// For a `Declined` step, the command that takes the decision back —
    /// which differs by where it was made: `mecha setup --undecline <id>` for
    /// an answer given here, `mecha features enable <id>` for a switch
    /// written `false`. Without it the closing hint named `--undecline` for
    /// both, and for a switch it answered "nothing to restore" (found on
    /// review of #460).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<Vec<String>>,
}

impl Step {
    fn new(id: &str, title: &str, status: Status, detail: impl Into<String>) -> Self {
        Step {
            id: id.into(),
            title: title.into(),
            status,
            detail: detail.into(),
            remedy: None,
            optional: false,
            undo: None,
        }
    }
    /// Mark a step as one the owner may decline outright.
    fn optional(mut self) -> Self {
        self.optional = true;
        self
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
    /// The registry's readout, one row per feature and part
    /// (`feature::all`), read against the **global** configuration as
    /// `mecha features` reads it. The optional features' steps are built from
    /// it (FEATURES-DESIGN.md §9 step 4), so setup asks what registration
    /// asks — an enabled `[[mcp]]` entry, never a binary on `PATH`, which is
    /// what the four hand-written steps this replaced checked (§1.1).
    pub features: Vec<crate::feature::Row>,
    /// What the default provider's server said about itself, when it is
    /// local and answered.
    pub props: Option<crate::provider::preflight::Props>,
    /// Whether the configured provider has a usable credential.
    pub provider_credential: bool,
    /// Whether a global config file exists at all.
    ///
    /// `Config::load_global` tolerates its absence and returns defaults,
    /// which is right — mecha must work before anybody has written one — but
    /// it meant a new install was never told the file exists or where it
    /// lives, and the first thing anyone needs to change lives in it.
    pub config_file: bool,
    /// What the loopback probe found — **including whether it ran at all**.
    ///
    /// See [`LocalProbe`]. The point of the probe is to turn *"`anthropic` has
    /// no usable credential"* into *"there is a server running right here,
    /// shall I write it down"*, which is the difference between a remedy and
    /// a diagnosis.
    pub local_probe: LocalProbe,
    /// Whether a trigger scheduler is running or installed.
    pub scheduler_installed: bool,
    pub trigger_count: usize,
    /// What the owner's charter is doing, read through the ordinary loader.
    pub charter: CharterState,
    /// Step ids the owner has said they do not want, from
    /// [`read_declined`].
    ///
    /// An unreadable store yields an **empty** set rather than a failure, and
    /// the direction is deliberate: showing a step somebody declined is a
    /// nuisance, hiding one they never declined is a silently incomplete
    /// install. This is the one place in this module where unknown resolves
    /// towards *more* noise, because here noise is the safe side. `setup`
    /// says out loud that the store could not be read — **on stderr, and
    /// before the `--json` return**, so the scriptable surface carries it
    /// too and stdout stays a parseable array.
    pub declined: std::collections::BTreeSet<String>,
}

/// A local server nobody has configured yet: where it is, and what it says
/// about itself.
#[derive(Debug, Clone)]
pub struct LocalServer {
    pub base_url: String,
    pub props: crate::provider::preflight::Props,
}

/// What the loopback probe found — **three-valued, because two of these were
/// being reported as the third.**
///
/// An `Option` here collapsed *"asked, and nothing answered"* into *"never
/// asked"*, and the step then printed the first sentence for both: somebody
/// with a configured-but-unselected `[providers.local]` and a llama-server
/// happily running on it was told **"Nothing was answering at
/// http://127.0.0.1:8080 when this ran"** — a fact asserted with no
/// observation behind it, which is this module's own header rule
/// (*never write down a number the user merely believes*) inverted. The
/// distinction is the same one [`Status::Unknown`] and [`Facts::declined`]
/// both keep: an absence and an unasked question are different findings.
#[derive(Debug, Clone, Default)]
pub enum LocalProbe {
    /// No probe was attempted — something can already answer, or a local
    /// provider is configured and it is not this module's place to go looking
    /// for a second one.
    #[default]
    NotAttempted,
    /// Asked, and nothing that looks like a model server answered.
    NothingAnswered,
    /// Asked, and found a server no provider names.
    Found(LocalServer),
}

/// Does this `/props` answer come from something that is actually a model
/// server?
///
/// **`preflight::Props` defaults every field on purpose**, so a llama-server
/// version bump costs a check rather than a parse failure — which means `{}`
/// with a 200 deserializes perfectly, and *any* JSON service answering on
/// :8080 (a catch-all API, a proxy) parses as a `Props`. That tolerance is
/// right where it is used for a server the owner has already told us about,
/// and wrong here, where the whole question is whether this is a model server
/// at all: without a check, an unrelated service gets announced as "already
/// serving (an unnamed model)" and one `y` repoints `default_provider` at it,
/// with no `model` and no `context_window` — the two settings this module's
/// header is about, both of which degrade quietly rather than failing.
///
/// So the discovery site asks for one thing only llama-server supplies.
/// Deliberately a *disjunction* rather than a required pair: either field
/// alone is enough to say "a model server answered", and demanding both would
/// reject a real server whose build reports one of them differently — which
/// is the forward-compatibility `Props` was made tolerant for.
pub fn answers_like_a_model_server(props: &crate::provider::preflight::Props) -> bool {
    props.model_alias.is_some() || props.default_generation_settings.n_ctx.is_some()
}

/// Where to look for a local server when the configured provider cannot
/// answer.
///
/// **Loopback only, and only on an install that has nothing working.** Every
/// other network call in this module is to a server the config already names;
/// this one is a guess, so it is confined to the address the documentation
/// tells people to serve on and to a machine that is otherwise stuck. Nothing
/// leaves the box, and a `mecha setup` on a working install makes no extra
/// call at all.
///
/// One address rather than a scan: probing a range would be a port scanner in
/// a setup tool, and the payoff — finding a server on a port nobody
/// documented — is not worth a command that behaves like one.
pub fn local_probe_candidates() -> &'static [&'static str] {
    &["http://127.0.0.1:8080"]
}

/// What `~/.mecha/charter.toml` is doing — the five answers a reader has to
/// tell apart.
///
/// `Absent` and `Empty` are **not** folded together, for the reason
/// [`crate::doctor`]'s own charter check keeps them apart: a file that parses
/// cleanly to zero lines is an authoring mistake by construction (nobody
/// writes an empty charter on purpose), where no file at all is the ordinary
/// state of a fresh install and is the one this module exists to offer
/// something about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CharterState {
    /// No file. A new install, and the case worth prompting.
    #[default]
    Absent,
    /// A file with no `[[line]]` entries — a template nobody filled in, or an
    /// edit that removed the last line.
    Empty,
    /// `n` lines, loading cleanly.
    Lines(usize),
    /// It exists and does not load: every run is starting un-chartered.
    Broken(String),
    /// Could not be established from here.
    Unknown,
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

    // --- 0. somewhere to put settings at all
    //
    // `Config::load_global` returns defaults when there is no file, which is
    // right — mecha must work before anybody has written one — but it also
    // meant nothing ever told a new install that the file exists or where it
    // lives, while every remaining step here is fixed by editing it.
    if !facts.config_file {
        steps.push(
            Step::new(
                "config-file",
                "A config file to change things in",
                Status::Missing,
                concat!(
                    "Everything runs on defaults until there is one, and defaults are ",
                    "fine — but the model, the server and the budgets are all set here, ",
                    "so the first thing anyone needs is a file to put them in. It is ",
                    "written commented, so it doubles as the list of what is adjustable."
                ),
            )
            .with(
                "Write a commented starter config to ~/.mecha/config.toml.",
                &["mecha", "config", "init"],
                false,
            ),
        );
    }

    // --- 1. can anything answer at all
    if !facts.provider_credential && local.is_none() {
        steps.push(provider_step(provider_name, cfg, facts));
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

    steps.extend(feature_steps(facts));
    steps.extend(timezone_step(cfg));
    steps.push(charter_step(&facts.charter));

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

    // Applied last, over the finished list, so a decline can never change
    // what a step *says* — only whether it is still being asked for. A
    // declined step keeps its detail and loses its remedy, because a remedy
    // is an offer and this one has been answered.
    //
    // **A `Done` step is never overwritten.** Declining Slack and then
    // linking it anyway (from the phone, from `mecha slack auth` directly)
    // must read as done rather than as refused — the machine's state is a
    // fact and the decision is a preference, and where they disagree the
    // fact wins. Otherwise a stale decline would hide a working integration
    // from its own owner. The same reason `Wrong` and `Broken` survive it:
    // "I don't want mail" is not "I don't want to be told my mail is
    // broken", and a decline that could suppress a failure would be a
    // silently-degrading guard.
    //
    // Gated on `optional` as well as on the status, so the guarantee holds
    // against a **hand-edited** store too: `setup-declined.json` is a plain
    // file, and a decline that only the prompt refused to record would be
    // one anybody could add with a text editor.
    for step in &mut steps {
        // A feature step's id is the feature's; step 1a offered the same
        // switch as `feature-<id>`, and an answer given there still counts.
        let declined = decline_keys(&step.id).any(|k| facts.declined.contains(&k));
        if step.optional && matches!(step.status, Status::Missing) && declined {
            step.status = Status::Declined;
            step.remedy = None;
            step.undo = Some(
                ["mecha", "setup", "--undecline", step.id.as_str()]
                    .map(String::from)
                    .to_vec(),
            );
        }
    }

    steps
}

/// Is the owner's zone configured — once, in `[agent] timezone`?
///
/// Every consumer derives it from there: the date the model is shown, and
/// every `[[mcp]]` server as `MECHA_TZ` (`McpServerConfig::owner_zone`). The
/// mail servers resolve `today` in that and **never** the machine's `TZ`,
/// which is UTC on a server and made the UTC day stand in for the owner's
/// (#243) — so without it every relative calendar window is refused, and the
/// model finds out per call rather than the owner finding out once, here.
///
/// Asked only once a server is wired: with none, the machine's zone is the
/// documented default for the date and costs nothing. Not optional once
/// asked — an unset zone there is not a feature going unused but a server
/// that cannot say which day it is. The fix is an edit, not a command, and
/// the one thing this step must not do is write down a zone for somebody.
fn timezone_step(cfg: &Config) -> Option<Step> {
    const TITLE: &str = "The owner's timezone";
    if cfg.mcp.iter().all(|m| m.disabled) {
        return None;
    }
    Some(match cfg.agent.timezone.as_deref() {
        Some(zone) => Step::new(
            "timezone",
            TITLE,
            Status::Done,
            format!("{zone}, handed to every MCP server as MECHA_TZ"),
        ),
        None => Step::new(
            "timezone",
            TITLE,
            Status::Missing,
            "No [agent] timezone, so an MCP server is handed no MECHA_TZ, and the mail \
             servers refuse `today`, `tomorrow` and the other relative windows rather than \
             guess a day. Set it once, as an IANA name, under [agent] in \
             ~/.mecha/config.toml: `timezone = \"<your IANA zone, e.g. Europe/Berlin>\"`. \
             Every MCP server is handed it as MECHA_TZ.",
        ),
    })
}

/// The one step that blocks every other, and the only one whose remedy used
/// to be a *viewer*.
///
/// It said `anthropic has no usable credential` and offered
/// `mecha config show` — which displays a file and fixes nothing. The step
/// that makes all the others untestable was the step with no path forward,
/// which is precisely backwards.
///
/// There are exactly two ways out and they are not symmetric, so the step
/// says which one this machine is actually in:
///
/// - **A local server is already running.** The common case for anybody who
///   followed the hardware pages first, and a real remedy: every value that
///   would be written is read back off `/props`, so the *existence* of the
///   provider is as much a measured fact as its context window.
/// - **No server.** Then the fix is a secret, and a secret is the one thing
///   this tool must not write — mecha stores the **name of an environment
///   variable**, never a key, so a config file can be read, copied or
///   committed without leaking one. Naming the exact variable and the exact
///   line is the most a remedy can honestly be here, so the detail carries
///   both rather than pointing at a command that would only show what is
///   already known.
fn provider_step(provider_name: &str, cfg: &Config, facts: &Facts) -> Step {
    let env_var = cfg
        .providers
        .get(provider_name)
        .and_then(|p| p.api_key_env.clone());

    let step = |detail: String| {
        Step::new(
            "provider-credential",
            "A provider that can answer",
            Status::Missing,
            detail,
        )
    };

    // 1. Something is serving, and no provider names it. The only branch with
    //    a command behind it, because it is the only one where the fix is a
    //    fact mecha can read rather than a secret only the owner has.
    if let LocalProbe::Found(found) = &facts.local_probe {
        let serving = found
            .props
            .model_alias
            .as_deref()
            .unwrap_or("(an unnamed model)");
        return step(format!(
            concat!(
                "`{provider_name}` has no usable credential — but something is ",
                "already serving {serving} at {url}, and nothing in the config names ",
                "it. Writing it down reads every value back off the server rather ",
                "than asking you for any of them, which is the only way ",
                "`context_window` ever gets to be the per-slot figure rather than ",
                "`-c`."
            ),
            provider_name = provider_name,
            serving = serving,
            url = found.base_url
        ))
        .with(
            "Write the local server down as a provider, from what it reports about itself.",
            &["mecha", "setup", "--write"],
            false,
        );
    }

    // 2. A local provider is configured and simply is not the selected one.
    //
    //    **This branch is why the probe's absence had to become visible.**
    //    Nothing probes when a local provider exists, so the "nothing
    //    answered" sentence below would be printed about an address never
    //    asked — and this is exactly the config that hits it: a
    //    `[providers.local]` on :8080 with a server running on it, and
    //    `default_provider` still pointing at a cloud provider whose key was
    //    never exported. Before this, that person got one step telling them
    //    to do the thing they had already done, and was never told the actual
    //    one-line fix.
    if let Some((name, pcfg)) = cfg
        .providers
        .iter()
        .find(|(name, p)| p.kind == "local" && *name != provider_name)
    {
        let where_it_points = pcfg
            .base_url
            .as_deref()
            .map(|u| format!(" ({u})"))
            .unwrap_or_default();
        return step(format!(
            concat!(
                "`{provider_name}` has no usable credential — but you already have a ",
                "local provider configured, `{name}`{where_it_points}, and it is not ",
                "the default. Point `default_provider` at it in the config, or select ",
                "it for one run with `-p {name}`. Nothing here probed {name}: whether ",
                "it is up is what `mecha setup` reports once it is the one being used."
            ),
            provider_name = provider_name,
            name = name,
            where_it_points = where_it_points
        ));
    }

    // 3. Nothing configured can answer and nothing was found. The fix is a
    //    secret, and a secret is the one thing this tool must not write —
    //    mecha stores the **name of an environment variable**, never a key, so
    //    a config file can be read, copied or committed without leaking one.
    //    Naming the exact variable is the most a remedy can honestly be, so
    //    the detail carries it rather than pointing at a command that would
    //    only show what is already known.
    let key_line = match &env_var {
        Some(var) => format!(
            concat!(
                "Set `{var}` in your shell (`export {var}=…`) and start a new one — ",
                "mecha stores the variable's *name* in the config and never the key ",
                "itself, so nothing here has to hold a secret."
            ),
            var = var
        ),
        // A provider configured with no `api_key_env` at all cannot be fixed
        // by exporting anything, and saying "set the variable it names" about
        // a provider that names none is the kind of instruction that sends
        // somebody looking for a typo they did not make.
        None => format!(
            concat!(
                "`{provider_name}` names no `api_key_env`, so there is no variable to ",
                "set — give it one, or point `default_provider` at a local server."
            ),
            provider_name = provider_name
        ),
    };
    // **Only said when a probe actually ran.** Reporting "nothing was
    // answering at X" after not asking is the same class of claim as writing
    // down a `context_window` nobody read off the wire.
    let local_line = match &facts.local_probe {
        LocalProbe::NothingAnswered => format!(
            concat!(
                "2. Run a model locally — the target rather than the fallback. Serve ",
                "it, then `mecha setup --write` reads the settings off it. Nothing ",
                "was answering at {tried} when this ran."
            ),
            tried = local_probe_candidates().join(", ")
        ),
        _ => concat!(
            "2. Run a model locally — the target rather than the fallback. Serve it, ",
            "then `mecha setup --write` reads the settings off it."
        )
        .to_string(),
    };
    step(format!(
        "Nothing can answer a prompt yet, so nothing below this can be tested. \
         Two ways out.\n\n1. {key_line}\n\n{local_line}"
    ))
}

/// The charter: what mecha is for, in the owner's own words.
///
/// **Nothing anywhere used to mention this.** `doctor`'s charter check
/// returns early on a file that does not exist — correctly, because never
/// having written one is not a fault — so a fresh install had no surface at
/// all that named the feature, and a never-written charter was
/// indistinguishable from a deliberately-empty one. Discovery was scrolling
/// the TUI's `/help` or finding the gear on the web page. That is the wrong
/// way round for the one document that says what the machine is *for*.
///
/// The remedy hands over `$EDITOR`; it never composes a line. See
/// [`crate::charter`]'s module doc for why that distinction, rather than the
/// absence of a verb, is the actual invariant.
fn charter_step(state: &CharterState) -> Step {
    const WHY: &str = concat!(
        "A short ranked list of standing priorities, in your own words, that rides in ",
        "every run's prompt. Order is rank: when two conflict, the higher one wins ",
        "outright. mecha never writes a line of it."
    );
    match state {
        CharterState::Lines(n) => Step::new(
            "charter",
            "Your charter",
            Status::Done,
            format!(
                "{n} standing priorit{} in rank order",
                if *n == 1 { "y" } else { "ies" }
            ),
        ),
        // Distinguished from `Absent` in the *detail*, not the status: both
        // are "nothing rides in the prompt", and both are fixed by the same
        // command, but only one of them is a half-finished edit somebody
        // should be told about rather than a fresh install.
        CharterState::Empty => Step::new(
            "charter",
            "Your charter",
            Status::Missing,
            format!(
                "The file exists with no `[[line]]` entries yet, so nothing from it rides \
                 in any prompt. {WHY}"
            ),
        )
        .with(
            "Open the charter in $EDITOR.",
            &["mecha", "charter", "edit"],
            true,
        )
        .optional(),
        CharterState::Absent => Step::new(
            "charter",
            "Your charter",
            Status::Missing,
            format!("Nothing written yet — every run is proceeding un-chartered. {WHY}"),
        )
        .with(
            "Create it from a commented template and open it in $EDITOR.",
            &["mecha", "charter", "edit"],
            true,
        )
        .optional(),
        // `Wrong`, not `Missing`: there is a document and it disagrees with
        // what a run can load, which is a different thing to do about it —
        // and unlike the two above, this one is already a `doctor` finding,
        // because it is a fault rather than an absence.
        CharterState::Broken(e) => Step::new(
            "charter",
            "Your charter does not load",
            Status::Wrong,
            format!(
                "{e}

Every run is starting un-chartered until this parses."
            ),
        )
        .with(
            "Open the charter in $EDITOR and fix it.",
            &["mecha", "charter", "edit"],
            true,
        ),
        CharterState::Unknown => Step::new(
            "charter",
            "Your charter",
            Status::Unknown,
            "the charter could not be read from here.",
        ),
    }
}

/// One step per optional feature — every feature with a switch — in
/// `Feature::ALL` order, which lists what a feature needs before it, so a
/// dependency is offered first (FEATURES-DESIGN.md §4.2 item 2, §9 step 4).
///
/// Built from the registry's own readout, so a step asks exactly what
/// `mecha features`, registration and the guards ask. The four hand-written
/// steps this replaced (`mail`, `docs`, `slack`, `graph`) looked for a binary
/// on `PATH` and read an installed binary with no `[[mcp]]` entry as done
/// (§1.1); their ids are the features' ids, so a decline recorded against
/// them still holds.
///
/// | readout | step |
/// |---|---|
/// | on | done, with its parts |
/// | switch written `false` | declined: an answer, already given |
/// | switch unanswered | missing and declinable, the remedy the enable command |
/// | switched on, but blocked or not ready | wrong: a yes that does not work is not a preference |
/// | unknown | unknown, offering nothing |
fn feature_steps(facts: &Facts) -> Vec<Step> {
    use crate::feature::{State, Switch};
    facts
        .features
        .iter()
        .filter(|row| row.part_of.is_none())
        .map(|row| {
            let f = row.id;
            let (id, title) = (f.id(), f.label());
            match &row.state {
                State::On { detail } => {
                    let parts: Vec<String> = facts
                        .features
                        .iter()
                        // Every part the switch owns, nested ones too
                        // (`layout` is `ocr`'s, and `ocr` is `documents'`).
                        .filter(|p| p.id != f && p.id.switch_owner() == f)
                        .map(|p| format!("{} {}", p.id.id(), p.state.word()))
                        .collect();
                    let detail = if parts.is_empty() {
                        detail.clone()
                    } else {
                        format!("{detail} ({})", parts.join(", "))
                    };
                    Step::new(id, title, Status::Done, detail)
                }
                // `false` is the owner's answer, written down. Not outstanding,
                // and not a decline `--undecline` can take back: the switch is
                // in the config, and the way back is the command that sets it.
                State::Off { .. } if row.switch == Some(Switch::Off) => {
                    let mut step = Step::new(
                        id,
                        title,
                        Status::Declined,
                        format!(
                            "turned off in `[features]`; `{}` turns it on",
                            row.next.as_deref().unwrap_or("mecha features enable")
                        ),
                    );
                    step.undo = row.next.as_deref().and_then(runnable).map(|(argv, _)| argv);
                    step
                }
                State::Off { .. } => with_next(
                    Step::new(
                        id,
                        title,
                        Status::Missing,
                        if row.in_use {
                            "Set up here but not switched on in `[features]`, so it is off. \
                             Every optional feature has a switch, and one left unanswered is off."
                                .to_string()
                        } else {
                            format!("{} Off until it is switched on.", blurb(f))
                        },
                    ),
                    row.next.as_deref(),
                )
                .optional(),
                // Reached only with the switch written `true` (`state` stops
                // at an absent or `false` one), so a yes that does not work,
                // like `Unready` — never a "no thanks" (review of #460). The
                // chained command takes the dependency first.
                State::Blocked { on } => with_next(
                    Step::new(
                        id,
                        title,
                        Status::Wrong,
                        format!(
                            "Switched on, but it needs {} first. `mecha features disable {id}` \
                             turns it off instead.",
                            on.label().to_lowercase()
                        ),
                    ),
                    row.next.as_deref(),
                ),
                State::Unready { reason, .. } => with_next(
                    Step::new(
                        id,
                        title,
                        Status::Wrong,
                        format!(
                            "Switched on, but not ready: {reason}. `mecha features disable \
                             {id}` turns it off instead."
                        ),
                    ),
                    row.next.as_deref(),
                ),
                State::Unknown { reason } => Step::new(id, title, Status::Unknown, reason.clone()),
            }
        })
        .collect()
}

/// A step's remedy from the row's next command — when it is one. Setup only
/// ever *runs* a command; a config edit (`add an [image] table`, `set
/// [documents] ocr = true`) or an instruction (`install factory-publish from
/// …`) is said in the detail and never run, because the edit is the owner's
/// to make (the module rule: setup never writes down what it was not told).
fn with_next(mut step: Step, next: Option<&str>) -> Step {
    let Some(next) = next else { return step };
    match runnable(next) {
        Some((argv, needs_terminal)) => {
            let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
            step.with(&format!("Run `{}`.", argv.join(" ")), &argv, needs_terminal)
        }
        None => {
            step.detail = format!("{} Next: {next}.", step.detail);
            step
        }
    }
}

/// `next` as an argv, if it is a command of ours to run: one of the binaries
/// this project ships, with nothing in it a person has to fill in. A
/// `<name>` the mail server needs is filled with `personal`, as the step this
/// replaced offered. An `auth` needs a terminal — a browser or a pasted code.
fn runnable(next: &str) -> Option<(Vec<String>, bool)> {
    let argv: Vec<String> = next
        .split_whitespace()
        .map(|w| {
            if w == "<name>" {
                "personal".to_string()
            } else {
                w.to_string()
            }
        })
        .collect();
    let program = argv.first()?.as_str();
    if !matches!(program, "mecha" | "mecha-mail" | "mecha-docs" | "cargo") {
        return None;
    }
    if argv
        .iter()
        .any(|w| w.contains(['<', '>', '[', ']', '`', '"', '=']))
    {
        return None;
    }
    let needs_terminal = argv.iter().any(|w| w == "auth");
    Some((argv, needs_terminal))
}

/// What a feature is for, in one line — what a step says when it is off.
/// Exhaustive, so a new feature says what it is before setup can offer it.
fn blurb(f: crate::feature::Feature) -> &'static str {
    use crate::feature::Feature;
    match f {
        Feature::Web => {
            "The web app on your tailnet: chat, mail, the graph and the review queues from a phone."
        }
        Feature::Slack => {
            "Watch a run from Slack, approve what it wants to send, and hand files in and out."
        }
        Feature::Mail => {
            "Mail and calendar, behind one server that names an account, never a provider."
        }
        Feature::Docs => {
            "Google Docs, Sheets and Slides — only files it made or you hand it in Google's picker."
        }
        Feature::Graph => "Memory: who people are and what happened when, read as untrusted.",
        Feature::Search => "Searching and opening the web, through the backends you list.",
        Feature::Documents => "Reading PDFs, with optional OCR and page layout.",
        Feature::Image => {
            "Making pictures on a local image server, with a character and style library."
        }
        Feature::Personas => "Characters you write and talk to, apart from the assistant.",
        Feature::Voice => "Talking to mecha: dictation, voice calls and voice cloning.",
        Feature::Incognito => "A web chat that leaves no trace, on a local model.",
        Feature::Frontdoor => {
            "Inbound requests from strangers, polls and publishing, behind a quarantine."
        }
        Feature::Messages => "Messages between this machine's own sessions.",
        // Parts are summarised under their parent and never stepped alone.
        Feature::Tasks
        | Feature::Ocr
        | Feature::Layout
        | Feature::Library
        | Feature::Dictate
        | Feature::Calls
        | Feature::Cloning
        | Feature::Publishing => "",
    }
}

/// The values a local server reports about itself, ready to be written down.
///
/// Returned rather than applied, so the caller can show them before changing
/// anything: this rewrites settings a person may have had reasons for, and
/// "here is what I would write" is a different act from writing it.
pub fn verified_settings(props: &crate::provider::preflight::Props) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(alias) = &props.model_alias {
        out.push(("model", toml_string(alias)));
    }
    // The per-slot figure, which is the one a request actually gets. Reading
    // it back is what makes `-c` versus `-c / -np` a non-question.
    if let Some(n) = props.default_generation_settings.n_ctx {
        out.push(("context_window", n.to_string()));
    }
    out.push(("vision", props.modalities.vision.to_string()));
    out
}

/// A TOML string literal, escaped the way **TOML** escapes.
///
/// **Not `format!("{s:?}")`, which is Rust escaping.** `str`'s `Debug`
/// renders a control character as `\u{1b}`; TOML's `\u` takes exactly four
/// hex digits and no braces, so that value is written into a config file that
/// then fails to parse. Quotes, backslashes, tabs and newlines happen to
/// escape compatibly, which is why this survived unnoticed — only
/// non-printables bite.
///
/// It matters here more than it looks because of where these bytes come from:
/// a server's own `/props`, on the discovery path, where the server is one
/// nobody has named. [`answers_like_a_model_server`] establishes that
/// something answering `:8080` may be a stranger; this makes sure a stranger's
/// answer cannot produce a config that no later `mecha` command can load.
pub fn toml_string(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
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

/// Where declines are recorded.
///
/// **In `~/.mecha/`, never in layered config**, on the rule triggers, skills
/// and the charter all keep: a project's `mecha.toml` arrives with a cloned
/// repository, and a repo that could decline your integrations would be
/// deciding what your machine offers you. There is deliberately no config
/// field pointing anywhere else, which is the same way the other three keep
/// the guarantee — by having no configurable path at all rather than by
/// asking callers to choose the global loader.
pub fn declined_path(home: &Path) -> PathBuf {
    home.join("setup-declined.json")
}

/// Step ids the owner has said they do not want.
///
/// An absent file is a confident empty set; an unreadable or malformed one
/// is `None`, so the caller can *say so* rather than quietly proceeding as
/// though nothing had been declined. See [`Facts::declined`] for why the
/// resolution of `None` is nonetheless "offer everything".
pub fn read_declined(home: &Path) -> Option<std::collections::BTreeSet<String>> {
    let path = declined_path(home);
    match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Some(std::collections::BTreeSet::new())
        }
        Err(_) => None,
        Ok(text) => serde_json::from_str::<Declined>(&text)
            .ok()
            .map(|d| d.declined),
    }
}

/// Record that the owner does not want `id`. Idempotent.
///
/// Read-modify-write rather than append, because this is a *set* and the
/// file is small enough that the whole document is the unit. Written to a
/// temp sibling and renamed, so a crash mid-write leaves the previous
/// answer whole rather than a truncated file that reads as "nothing
/// declined" — which would silently re-offer everything.
///
/// `Ok(Some(path))` says an unreadable store was **set aside** at `path`
/// rather than overwritten — see [`salvage_unreadable`]. The caller is
/// expected to say so; losing somebody's recorded answers is not a thing to
/// do quietly.
pub fn decline(home: &Path, id: &str) -> std::io::Result<DeclineWrite> {
    let (mut set, salvaged) = read_for_write(home);
    let changed = set.insert(id.to_string());
    write_declined(home, &set)?;
    Ok(DeclineWrite { salvaged, changed })
}

/// Take one back out — the undo, so a decline is a preference rather than a
/// door that locks behind you. `id` of `None` clears every one.
/// The keys that record a decline of step `id`: the id itself and, for a
/// feature step, the `feature-<id>` step 1a offered the same switch under.
/// One function for both readers — `plan` honouring a decline and
/// `undecline` taking it back — so the two cannot drift: a decline the plan
/// honoured and the undo did not remove read as "nothing to restore" while
/// the step still said no thanks (found on review of #460).
pub fn decline_keys(id: &str) -> impl Iterator<Item = String> {
    [id.to_string(), format!("feature-{id}")].into_iter()
}

pub fn undecline(home: &Path, id: Option<&str>) -> std::io::Result<DeclineWrite> {
    let (mut set, salvaged) = read_for_write(home);
    let changed = match id {
        // Every key the plan reads as this step's decline, so the undo
        // cannot report "nothing to restore" over one it still honours.
        Some(id) => decline_keys(id).fold(false, |changed, k| set.remove(&k) | changed),
        None => {
            let had = !set.is_empty();
            set.clear();
            had
        }
    };
    write_declined(home, &set)?;
    Ok(DeclineWrite { salvaged, changed })
}

/// What a write to the decline store actually did.
///
/// **`changed` is graded off the set, never off the argument.** `undecline`
/// used to discard `BTreeSet::remove`'s answer, so a typo'd id wrote the set
/// back untouched and the caller still announced *"`slak` will be offered
/// again"* and exited 0 — the person believed the way back had been taken,
/// ran `mecha setup`, and saw `you said no thanks` on the step they had just
/// "restored", with nothing anywhere saying the two disagreed. A claim about
/// a local write is worth checking against the write, which is the same rule
/// [`salvage_unreadable`] one line up exists for and the same rule the
/// harness applies to everything a *model* says about its own work.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DeclineWrite {
    /// Where an unreadable store was set aside, when it had to be.
    pub salvaged: Option<PathBuf>,
    /// Whether the recorded set actually changed.
    pub changed: bool,
}

/// The set to modify, and where the previous file went if it could not be
/// read.
///
/// **`unwrap_or_default()` here was this module's own rule inverted.**
/// [`read_declined`] answers `None` for an unreadable or malformed store
/// precisely so a caller can tell *unknown* from *empty* — and collapsing it
/// to empty on the write path then **persisted** the collapse: somebody with
/// a typo in `setup-declined.json` saw `setup`'s honest "could not be read"
/// warning, answered `never` to one step, and had the file rewritten with
/// exactly that one id, every previously recorded answer gone with no further
/// word. [`write_declined`]'s own comment argues that a partial write "would
/// silently re-offer everything"; this path was doing it on purpose.
///
/// So the bytes are kept. Same move `mecha setup --write` makes before
/// editing a config it did not author: a file somebody may have meant
/// something by is moved aside, never overwritten.
fn read_for_write(home: &Path) -> (std::collections::BTreeSet<String>, Option<PathBuf>) {
    match read_declined(home) {
        Some(set) => (set, None),
        None => (Default::default(), salvage_unreadable(home)),
    }
}

/// Move an unreadable decline store aside so the write about to happen
/// cannot destroy it. Best-effort: a salvage that fails must not stop the
/// answer being recorded, or a read-only directory would make `never`
/// permanently unavailable.
fn salvage_unreadable(home: &Path) -> Option<PathBuf> {
    let path = declined_path(home);
    // Stamped, so a second corruption later does not overwrite the first
    // salvage — the whole point here is that nothing is lost quietly.
    let aside = path.with_extension(format!(
        "json.unreadable.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ));
    std::fs::rename(&path, &aside).ok().map(|()| aside)
}

fn write_declined(home: &Path, set: &std::collections::BTreeSet<String>) -> std::io::Result<()> {
    let path = declined_path(home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(&Declined {
        declined: set.clone(),
    })
    .map_err(std::io::Error::other)?;
    // Same directory, so the rename cannot cross a filesystem — the shape
    // `serve::settings`' charter save already uses, and for the same reason.
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &path)
}

/// The file's own shape. A named field rather than a bare array so the
/// document can grow a sibling (a timestamp, a reason) without the next
/// version having to guess what a top-level list meant.
#[derive(Debug, Default, Serialize, serde::Deserialize)]
struct Declined {
    #[serde(default)]
    declined: std::collections::BTreeSet<String>,
}

/// Read the charter the way a run does, for [`Facts`].
///
/// Through [`crate::charter::Charter::load`] rather than by looking at the
/// file, because the question is not "is there a file" but "would a run get
/// anything from it" — and those differ for a template nobody filled in and
/// for a document with a typo'd table name.
pub fn charter_state(path: &Path) -> CharterState {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return CharterState::Absent,
        Err(_) => return CharterState::Unknown,
        Ok(_) => {}
    }
    match crate::charter::Charter::load(path) {
        Err(e) => CharterState::Broken(format!("{e:#}")),
        Ok(c) if c.is_empty() => CharterState::Empty,
        Ok(c) => CharterState::Lines(c.lines().len()),
    }
}

/// The mail crate's account store, found **by the mail crate's rule**:
/// `$MECHA_MAIL_DIR`, else `~/.mecha/mail` under the real home — never
/// `$MECHA_HOME/mail`. The mail crate honours the first variable and ignores
/// the second, so a reader of its store that used mecha's own rule looked in
/// the wrong place whenever either was set, and counted an authorised install
/// as having no account (found on review of #428; `frontdoor::mail_dir` had
/// paid for the same mistake first).
pub fn mail_store_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("MECHA_MAIL_DIR") {
        return Some(PathBuf::from(dir));
    }
    Some(dirs::home_dir()?.join(".mecha").join("mail"))
}

/// A legacy per-provider store (`mecha-google`, `mecha-outlook`) by
/// `token::provider_path`'s rule: `$<env_var>`, else `~/.mecha/<provider>`
/// under the real home — never `$MECHA_HOME/<provider>`.
pub fn legacy_store_dir(provider: &str, env_var: &str) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var(env_var) {
        return Some(PathBuf::from(dir));
    }
    Some(dirs::home_dir()?.join(".mecha").join(provider))
}

/// `mecha-docs`' account store: always `~/.mecha/docs` under the real home
/// (`google::docs::docs_home`), which honours neither variable.
pub fn docs_store_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".mecha").join("docs"))
}

/// Whether Slack tokens are stored: `None` when `<home>/slack` could not be
/// read, which is a finding, not an absence.
pub fn slack_linked(home: &std::path::Path) -> Option<bool> {
    let dir = home.join("slack");
    match std::fs::read_dir(&dir) {
        Ok(entries) => Some(entries.flatten().any(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == "json" || x == "toml")
        })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false),
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
            role: None,
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
            // A complete install has answered every switch it uses; the
            // feature steps' own tests build rows with `rows`.
            features: Vec::new(),
            scheduler_installed: true,
            trigger_count: 0,
            // A complete install has a charter and a config file, so
            // `everything_configured…` keeps meaning what it says.
            charter: CharterState::Lines(3),
            config_file: true,
            local_probe: LocalProbe::NotAttempted,
            declined: Default::default(),
        }
    }

    /// The registry's readout for a machine where every store was read and
    /// held nothing, adjusted by `tweak` — real rows from `feature::all`, so
    /// a step is tested against what `mecha features` would say.
    fn rows(tweak: impl FnOnce(&mut crate::feature::Facts)) -> Vec<crate::feature::Row> {
        let mut ff = crate::feature::Facts {
            slack_linked: Some(false),
            mail_accounts: Some(0),
            docs_accounts: Some(0),
            personas_stored: Some(false),
            ..Default::default()
        };
        tweak(&mut ff);
        crate::feature::all(&ff)
    }

    fn switch_on(ff: &mut crate::feature::Facts, ids: &[&str]) {
        for id in ids {
            ff.config.features.0.insert(id.to_string(), true);
        }
    }

    fn mcp_entry(name: &str, command: &str) -> crate::config::McpServerConfig {
        crate::config::McpServerConfig {
            name: name.into(),
            command: command.into(),
            ..Default::default()
        }
    }

    fn step<'a>(steps: &'a [Step], id: &str) -> &'a Step {
        steps.iter().find(|s| s.id == id).expect("step missing")
    }

    /// #243: the zone is configured once and derived everywhere, so the only
    /// thing to check is that once. Unset is not declinable — and the step
    /// offers a placeholder, never a zone nobody chose.
    #[test]
    fn the_timezone_is_set_once_and_never_guessed() {
        let f = facts(Some(props(262144, 4, true)));
        let mut cfg = cfg_with_local(262144, Some(true));
        cfg.agent.timezone = None;
        assert!(
            plan(&cfg, "local", &f).iter().all(|s| s.id != "timezone"),
            "no server wired: the machine's zone is the default, nothing to ask"
        );

        cfg.mcp = vec![crate::config::McpServerConfig {
            name: "mail".into(),
            command: "anything".into(),
            ..Default::default()
        }];
        cfg.agent.timezone = Some("Europe/Berlin".into());
        assert_eq!(
            step(&plan(&cfg, "local", &f), "timezone").status,
            Status::Done
        );

        cfg.agent.timezone = None;
        let steps = plan(&cfg, "local", &f);
        let s = step(&steps, "timezone");
        assert_eq!(s.status, Status::Missing);
        assert!(
            !s.optional,
            "not knowing which day it is is not a preference"
        );
        assert!(s.detail.contains("<your IANA zone"), "{}", s.detail);
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

    /// F6: a feature this install set up with its switch unanswered is a
    /// declinable step whose remedy writes the bool — the row's `in_use`, from
    /// the same `announcement` the upgrade notice prints.
    #[test]
    fn a_feature_set_up_but_unanswered_is_offered_and_declinable() {
        let cfg = cfg_with_local(262144, Some(true));
        let mut f = facts(Some(props(262144, 4, true)));
        f.features = rows(|ff| {
            ff.has_mail_binary = true;
            ff.mail_accounts = Some(2);
            ff.config.mcp.push(mcp_entry("mail", "mecha-mail"));
        });
        let steps = plan(&cfg, "local", &f);
        let mail = step(&steps, "mail");
        assert_eq!(mail.status, Status::Missing);
        assert!(mail.optional, "declinable, as every unanswered switch is");
        assert!(mail.detail.contains("Set up here"), "{}", mail.detail);
        assert_eq!(
            mail.remedy.as_ref().unwrap().argv,
            ["mecha", "features", "enable", "mail"]
        );
        // An answer given to step 1a's offer, under its old id, still holds —
        // and `--undecline mail` takes it back, reporting that it did, since
        // `mail` is the only id any surface shows (review of #460).
        f.declined.insert("feature-mail".into());
        assert_eq!(
            step(&plan(&cfg, "local", &f), "mail").status,
            Status::Declined
        );
        let home = std::env::temp_dir().join(format!(
            "mecha-feature-decline-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        decline(&home, "feature-mail").unwrap();
        assert!(undecline(&home, Some("mail")).unwrap().changed);
        assert!(read_declined(&home).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The registry decides, so setup asks what registration asks (§1.1): an
    /// installed `mecha-mail` with no `[[mcp]]` entry is not mail. Switched
    /// on, it is a yes that does not work — `Wrong`, never declinable — and
    /// the fix is an edit, said rather than run. With the entry and no
    /// account, the remedy is the auth, which needs a terminal.
    #[test]
    fn a_feature_step_asks_what_registration_asks() {
        let cfg = cfg_with_local(262144, Some(true));
        let mut f = facts(Some(props(262144, 4, true)));
        f.features = rows(|ff| {
            switch_on(ff, &["mail"]);
            ff.has_mail_binary = true;
        });
        let steps = plan(&cfg, "local", &f);
        let mail = step(&steps, "mail");
        assert_eq!(mail.status, Status::Wrong);
        assert!(!mail.optional);
        assert!(mail.remedy.is_none(), "an edit is never run");
        assert!(mail.detail.contains("[[mcp]]"), "{}", mail.detail);

        f.features = rows(|ff| {
            switch_on(ff, &["mail"]);
            ff.has_mail_binary = true;
            ff.config.mcp.push(mcp_entry("mail", "mecha-mail"));
        });
        let steps = plan(&cfg, "local", &f);
        let remedy = step(&steps, "mail")
            .remedy
            .clone()
            .expect("the auth is runnable");
        assert_eq!(
            remedy.argv,
            ["mecha-mail", "auth", "personal", "--provider", "google"]
        );
        assert!(remedy.needs_terminal);
    }

    /// `false` is an answer written down: declined, not outstanding, and the
    /// way back is the enable command rather than `--undecline`. A feature
    /// switched on but blocked on an unanswered dependency is a yes that does
    /// not work, offered the chained command, dependency first.
    #[test]
    fn an_explicit_false_is_an_answer_and_a_dependency_comes_first() {
        let cfg = cfg_with_local(262144, Some(true));
        let mut f = facts(Some(props(262144, 4, true)));
        f.features = rows(|ff| {
            ff.config.features.0.insert("slack".into(), false);
            switch_on(ff, &["incognito"]);
        });
        let steps = plan(&cfg, "local", &f);
        let slack = step(&steps, "slack");
        assert_eq!(slack.status, Status::Declined);
        // Its way back is the switch, not `--undecline` (review of #460).
        assert_eq!(
            slack.undo.as_deref(),
            Some(&["mecha", "features", "enable", "slack"].map(String::from)[..])
        );
        assert!(
            slack.detail.contains("mecha features enable slack"),
            "{}",
            slack.detail
        );
        // Switched on and blocked is a yes that does not work: wrong, not
        // declinable, its remedy the chained command (review of #460).
        let incognito = step(&steps, "incognito");
        assert_eq!(incognito.status, Status::Wrong);
        assert!(!incognito.optional);
        assert!(
            incognito
                .detail
                .contains("mecha features disable incognito"),
            "{}",
            incognito.detail
        );
        assert_eq!(
            incognito.remedy.as_ref().unwrap().argv,
            ["mecha", "features", "enable", "web", "incognito"]
        );
        // The dependency's own step comes before it.
        let at = |id: &str| steps.iter().position(|s| s.id == id).unwrap();
        assert!(at("web") < at("incognito"));
    }

    /// A fresh install is *offered* a charter, and the offer never composes
    /// one.
    ///
    /// The gap this closes: `doctor::check_charter` returns early on a file
    /// that does not exist — right, because never having written one is not
    /// a fault — so before this step nothing on any surface named the
    /// feature to a new user at all.
    #[test]
    fn a_fresh_install_is_offered_a_charter_and_the_offer_authors_nothing() {
        let cfg = cfg_with_local(262144, Some(true));
        let mut f = facts(Some(props(262144, 4, true)));
        f.charter = CharterState::Absent;
        let steps = plan(&cfg, "local", &f);
        let charter = step(&steps, "charter");
        assert_eq!(charter.status, Status::Missing);
        let remedy = charter.remedy.as_ref().expect("a fresh charter is offered");
        assert_eq!(remedy.argv, ["mecha", "charter", "edit"]);
        assert!(
            remedy.needs_terminal,
            "handing over $EDITOR needs a keyboard"
        );
        // The offer must not put words in anyone's mouth: the *only* text
        // this module supplies about a charter is a description of what one
        // is. Nothing here may read as a suggested priority, because a
        // priority mecha proposed is a priority a model could later argue
        // from — see `charter.rs`'s module doc for the invariant.
        assert!(
            charter.detail.contains("in your own words"),
            "the detail should say whose words these are: {}",
            charter.detail
        );
    }

    /// A file with no lines and no file at all are both "nothing rides in
    /// the prompt" and are told apart in the detail, because only one of
    /// them is a half-finished edit.
    #[test]
    fn an_empty_charter_reads_differently_from_an_absent_one() {
        let cfg = cfg_with_local(262144, Some(true));
        let mut f = facts(Some(props(262144, 4, true)));

        f.charter = CharterState::Empty;
        let empty = step(&plan(&cfg, "local", &f), "charter").clone();
        f.charter = CharterState::Absent;
        let absent = step(&plan(&cfg, "local", &f), "charter").clone();

        assert_eq!(empty.status, Status::Missing);
        assert_eq!(absent.status, Status::Missing);
        assert_ne!(
            empty.detail, absent.detail,
            "a template nobody filled in is not the same finding as a fresh install"
        );
    }

    /// A charter that does not load is `Wrong`, not `Missing`: there *is* a
    /// document and it disagrees with what a run can load, which is a
    /// different thing to do about it — and, unlike the other two, already
    /// a `doctor` finding.
    #[test]
    fn a_charter_that_does_not_load_is_wrong_rather_than_missing() {
        let cfg = cfg_with_local(262144, Some(true));
        let mut f = facts(Some(props(262144, 4, true)));
        f.charter = CharterState::Broken("duplicate id `x`".into());
        let charter = step(&plan(&cfg, "local", &f), "charter").clone();
        assert_eq!(charter.status, Status::Wrong);
        assert!(charter.detail.contains("duplicate id"));
        assert!(
            charter.detail.contains("un-chartered"),
            "say what it costs, not just that it failed: {}",
            charter.detail
        );
    }

    /// A decline is remembered, and it is not a fifth shade of "not done".
    #[test]
    fn a_declined_step_reports_the_decision_rather_than_the_absence() {
        let cfg = cfg_with_local(262144, Some(true));
        let mut f = facts(Some(props(262144, 4, true)));
        f.features = rows(|_| {});

        // Without the decline it is ordinary outstanding work, with a remedy.
        let before = step(&plan(&cfg, "local", &f), "slack").clone();
        assert_eq!(before.status, Status::Missing);
        assert!(before.remedy.is_some());

        f.declined.insert("slack".into());
        let after = step(&plan(&cfg, "local", &f), "slack").clone();
        // An answer given here is taken back here.
        assert_eq!(
            after.undo.as_deref(),
            Some(&["mecha", "setup", "--undecline", "slack"].map(String::from)[..])
        );
        assert_eq!(after.status, Status::Declined);
        assert!(
            after.remedy.is_none(),
            "a remedy is an offer, and this one has been answered"
        );
        assert_eq!(
            after.detail, before.detail,
            "a decline changes whether a step is asked for, never what it says"
        );
    }

    /// The step that blocks every other one carries a **remedy**, not a
    /// viewer, when there is something to run.
    ///
    /// It used to say `anthropic has no usable credential` and offer
    /// `mecha config show`, which displays a file and fixes nothing — the one
    /// step that makes all the others untestable was the one with no path
    /// forward.
    #[test]
    fn a_running_local_server_turns_the_blocking_step_into_something_runnable() {
        let cfg = Config::default();
        let mut f = facts(None);
        f.provider_credential = false;
        f.local_probe = LocalProbe::Found(LocalServer {
            base_url: "http://127.0.0.1:8080".into(),
            props: props(32768, 4, false),
        });

        let s = plan(&cfg, "anthropic", &f);
        let step = step(&s, "provider-credential");
        assert_eq!(
            step.remedy.as_ref().map(|r| r.argv.clone()),
            Some(vec!["mecha".into(), "setup".into(), "--write".into()]),
            "a server is running: writing it down is a thing this tool can do"
        );
        assert!(
            step.detail.contains("127.0.0.1:8080") && step.detail.contains("qwen3.6-35b-a3b"),
            "name what was found, so the offer is checkable: {}",
            step.detail
        );
        // Never declinable, however it is phrased — a credential is not a
        // feature going unused.
        assert!(!step.optional);
    }

    /// With nothing answering, the fix is a secret — and a secret is the one
    /// thing this tool must not write. So the step names the exact variable
    /// and says the key never lands in a file, rather than pointing at a
    /// command that could not help.
    #[test]
    fn with_no_server_the_step_names_the_variable_and_promises_not_to_store_it() {
        let cfg = Config::default();
        let mut f = facts(None);
        f.provider_credential = false;
        f.local_probe = LocalProbe::NothingAnswered;

        let s = plan(&cfg, "anthropic", &f);
        let step = step(&s, "provider-credential");
        assert!(
            step.detail.contains("ANTHROPIC_API_KEY"),
            "name the variable rather than describing it: {}",
            step.detail
        );
        assert!(
            step.detail.contains("never the key itself"),
            "say where the secret does *not* go: {}",
            step.detail
        );
        // Both ways out are named, including the one this project is for.
        assert!(step.detail.contains("locally"), "{}", step.detail);
        assert!(
            step.remedy.is_none(),
            "there is no command that can set somebody's environment for them, \
             and offering one that only prints is what this replaced"
        );
    }

    /// A provider with no `api_key_env` cannot be fixed by exporting
    /// anything, and telling somebody to "set the variable it names" about a
    /// provider that names none sends them looking for a typo they did not
    /// make.
    #[test]
    fn a_provider_naming_no_key_variable_is_not_told_to_set_one() {
        let mut cfg = Config::default();
        cfg.providers.get_mut("anthropic").unwrap().api_key_env = None;
        let mut f = facts(None);
        f.provider_credential = false;

        f.local_probe = LocalProbe::NothingAnswered;
        let detail = step(&plan(&cfg, "anthropic", &f), "provider-credential")
            .detail
            .clone();
        assert!(detail.contains("names no `api_key_env`"), "{detail}");
        assert!(!detail.contains("export "), "nothing to export: {detail}");
    }

    /// **A probe that never ran must not be reported as one that found
    /// nothing.**
    ///
    /// The config that hits this: a `[providers.local]` on :8080 with a
    /// server running on it, and `default_provider` still pointing at a
    /// cloud provider whose key was never exported. Nothing probes (a local
    /// provider is configured), so an `Option<LocalServer>` made "never
    /// asked" indistinguishable from "asked and heard nothing" — and the
    /// person with a running server was told *"Nothing was answering at
    /// http://127.0.0.1:8080 when this ran"*. A fact with no observation
    /// behind it, which is this module's own header rule inverted.
    #[test]
    fn an_unattempted_probe_is_never_reported_as_a_failed_one() {
        let cfg = Config::default();
        let mut f = facts(None);
        f.provider_credential = false;

        f.local_probe = LocalProbe::NothingAnswered;
        let asked = step(&plan(&cfg, "anthropic", &f), "provider-credential")
            .detail
            .clone();
        assert!(
            asked.contains("Nothing was answering"),
            "a probe that ran may report what it found: {asked}"
        );

        f.local_probe = LocalProbe::NotAttempted;
        let never_asked = step(&plan(&cfg, "anthropic", &f), "provider-credential")
            .detail
            .clone();
        assert!(
            !never_asked.contains("Nothing was answering"),
            "a probe that never ran must claim nothing about what is there: {never_asked}"
        );
        // And the rest of the advice survives — this is about dropping one
        // unearned sentence, not the route it belongs to.
        assert!(never_asked.contains("Run a model locally"), "{never_asked}");
    }

    /// A configured local provider that simply is not selected gets named,
    /// with the one-line fix.
    ///
    /// Before this branch existed, that install produced a single step
    /// telling somebody to serve a model they were already serving, and
    /// never mentioned the provider sitting in their own config.
    #[test]
    fn a_configured_but_unselected_local_provider_is_named_as_the_way_out() {
        let mut cfg = Config::default();
        let mut local = cfg.providers.get("anthropic").cloned().unwrap();
        local.kind = "local".into();
        local.base_url = Some("http://127.0.0.1:8080".into());
        local.api_key_env = None;
        cfg.providers.insert("local".into(), local);

        let mut f = facts(None);
        f.provider_credential = false;
        // Nothing probed, because a local provider exists — which is exactly
        // the state that used to produce a false report.
        f.local_probe = LocalProbe::NotAttempted;

        let detail = step(&plan(&cfg, "anthropic", &f), "provider-credential")
            .detail
            .clone();
        assert!(
            detail.contains("`local`") && detail.contains("127.0.0.1:8080"),
            "name the provider they already have, and where it points: {detail}"
        );
        assert!(
            detail.contains("default_provider"),
            "and the one-line fix: {detail}"
        );
        assert!(
            !detail.contains("Nothing was answering"),
            "nothing probed it, so nothing may be claimed about it: {detail}"
        );
    }

    /// **Parsing is not identification.** `Props` defaults every field so a
    /// llama-server version bump costs a check rather than a parse failure —
    /// so `{}` from any JSON service on :8080 parses perfectly. Without this
    /// check it was announced as "already serving (an unnamed model)", and
    /// one `y` would repoint `default_provider` at it with no `model` and no
    /// `context_window`: the two settings this module exists to stop people
    /// getting wrong, both of which degrade quietly.
    #[test]
    fn a_stranger_answering_200_is_not_a_model_server() {
        use crate::provider::preflight::Props;

        // The premise, pinned rather than assumed: an empty object really
        // does parse. If `Props` ever stops being fully-defaulted this
        // assertion fails and the check below can be reconsidered, instead of
        // quietly guarding against something that can no longer happen.
        let stranger: Props =
            serde_json::from_str("{}").expect("Props defaults every field, so `{}` parses");
        assert!(
            !answers_like_a_model_server(&stranger),
            "any JSON service answering 200 would otherwise read as a model server"
        );
        // A plausible non-model service: valid JSON, unrelated keys, still a
        // clean parse because unknown fields are ignored.
        let proxy: Props = serde_json::from_str(r#"{"status":"ok","uptime":42}"#)
            .expect("unknown fields are ignored");
        assert!(!answers_like_a_model_server(&proxy));

        // Either field alone identifies a real one — a disjunction on
        // purpose, so a build that reports one of them differently is not
        // rejected, which is what the tolerance was for.
        let named = Props {
            model_alias: Some("qwen3-14b".into()),
            ..Props::default()
        };
        assert!(answers_like_a_model_server(&named));
        assert!(answers_like_a_model_server(&props(32768, 4, false)));
    }

    /// A new install is told the config file exists and where — nothing did.
    /// `Config::load_global` tolerating its absence is right, and is also why
    /// nobody ever learned about it.
    #[test]
    fn a_missing_config_file_is_offered_and_a_present_one_is_not_mentioned() {
        let cfg = Config::default();
        let mut f = facts(None);
        f.config_file = false;
        let s = plan(&cfg, "anthropic", &f);
        assert_eq!(
            step(&s, "config-file")
                .remedy
                .as_ref()
                .map(|r| r.argv.clone()),
            Some(vec!["mecha".into(), "config".into(), "init".into()])
        );

        f.config_file = true;
        assert!(
            !plan(&cfg, "anthropic", &f)
                .iter()
                .any(|s| s.id == "config-file"),
            "a file that exists is not a step"
        );
    }

    /// **The step that makes everything else work cannot be declined.**
    ///
    /// The bug this fails on was found by running the flow rather than by
    /// reading it: `declinable` was inferred from `Status::Missing`, and a
    /// provider with no credential is missing — so declining every "missing"
    /// step reported `Nothing outstanding.` on an install that could not
    /// answer a single prompt. Asserted against a *hand-edited* store,
    /// because the file is plain JSON and a guarantee that only the prompt
    /// enforced would be one anybody could edit around.
    #[test]
    fn a_step_that_is_not_optional_cannot_be_declined_even_by_editing_the_file() {
        let mut cfg = Config::default();
        // A provider with no credential and no local server: the one step
        // that blocks every other.
        let p = cfg.providers.get_mut("anthropic").unwrap();
        p.api_key_env = Some("MECHA_TEST_NO_SUCH_KEY".into());
        let mut f = facts(None);
        f.provider_credential = false;
        f.features = rows(|_| {});

        for id in [
            "provider-credential",
            "mail",
            "docs",
            "slack",
            "graph",
            "charter",
        ] {
            f.declined.insert(id.to_string());
        }
        let steps = plan(&cfg, "anthropic", &f);

        assert_eq!(
            step(&steps, "provider-credential").status,
            Status::Missing,
            "a credential is not a feature somebody can decline"
        );
        // The integrations, which genuinely are optional, still honour it —
        // otherwise this test would pass on a decline that never worked.
        assert_eq!(step(&steps, "slack").status, Status::Declined);
    }

    /// Every declinable step is one where "I don't want this" is a coherent
    /// sentence. Asserted over the whole plan rather than per step, so the
    /// next step added has to be decided about rather than defaulting into
    /// being refusable.
    #[test]
    fn only_genuinely_optional_things_are_declinable() {
        let cfg = Config::default();
        let mut f = facts(None);
        f.provider_credential = false;
        // Every feature unanswered: each is a coherent "I don't want this".
        f.features = rows(|_| {});
        f.charter = CharterState::Absent;
        f.trigger_count = 1;
        f.scheduler_installed = false;

        let steps = plan(&cfg, "anthropic", &f);
        let optional: Vec<&str> = steps
            .iter()
            .filter(|s| s.optional)
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(
            optional,
            [
                "web",
                "slack",
                "mail",
                "docs",
                "graph",
                "search",
                "documents",
                "image",
                "personas",
                "voice",
                "incognito",
                "frontdoor",
                "messages",
                "charter"
            ]
        );
    }

    /// The offer text is one paragraph, not a wrapped source literal.
    ///
    /// A `\`-continued string that loses its backslash keeps the source's
    /// indentation, and the result reads as a bug to the one person least
    /// able to tell it is cosmetic — somebody on their first five minutes.
    /// Caught by running the command; kept by this.
    #[test]
    fn no_step_detail_carries_its_source_indentation() {
        let cfg = Config::default();
        let mut f = facts(None);
        f.provider_credential = false;
        f.charter = CharterState::Empty;
        f.trigger_count = 1;
        f.scheduler_installed = false;
        for s in plan(&cfg, "anthropic", &f) {
            assert!(
                !s.detail.contains("   "),
                "`{}` carries a run of spaces from its source literal: {:?}",
                s.id,
                s.detail
            );
        }
    }

    /// The fact beats the preference. Declining Slack and then linking it
    /// anyway — from the phone, or by running `mecha slack auth` directly —
    /// must read as done, or a stale decline hides a working integration
    /// from its own owner.
    #[test]
    fn a_decline_never_overwrites_a_step_that_is_actually_done() {
        let cfg = cfg_with_local(262144, Some(true));
        let mut f = facts(Some(props(262144, 4, true)));
        f.features = rows(|ff| {
            switch_on(ff, &["slack"]);
            ff.slack_linked = Some(true);
        });
        f.declined.insert("slack".into());
        assert_eq!(step(&plan(&cfg, "local", &f), "slack").status, Status::Done);
    }

    /// And it never suppresses a failure. "I don't want mail" is not "I
    /// don't want to be told my mail is broken" — a decline that could hide
    /// a `Wrong` would be a silently-degrading guard.
    #[test]
    fn a_decline_cannot_suppress_a_broken_one() {
        let cfg = cfg_with_local(262144, Some(true));
        let mut f = facts(Some(props(262144, 4, true)));
        f.charter = CharterState::Broken("bad toml".into());
        f.declined.insert("charter".into());
        assert_eq!(
            step(&plan(&cfg, "local", &f), "charter").status,
            Status::Wrong,
            "a decline must not hide a document that stops every run being chartered"
        );
    }

    /// An unknown store is not silently declined either — `Unknown` is
    /// already "cannot tell from here" and must not become "answered".
    #[test]
    fn a_decline_does_not_apply_to_an_unknown_step() {
        let cfg = cfg_with_local(262144, Some(true));
        let mut f = facts(Some(props(262144, 4, true)));
        f.features = rows(|ff| {
            switch_on(ff, &["mail"]);
            ff.config.mcp.push(mcp_entry("mail", "mecha-mail"));
            ff.mail_accounts = None;
        });
        f.declined.insert("mail".into());
        assert_eq!(
            step(&plan(&cfg, "local", &f), "mail").status,
            Status::Unknown
        );
    }

    /// The store round-trips, is idempotent, and can be undone — a decline
    /// is a preference, not a door that locks behind you.
    #[test]
    fn declines_round_trip_and_can_be_taken_back() {
        let home = std::env::temp_dir().join(format!(
            "mecha-declined-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();

        // An absent file is a confident empty set, never `None` — the same
        // rule `count_accounts` keeps for a credential root.
        assert_eq!(read_declined(&home), Some(Default::default()));

        decline(&home, "slack").unwrap();
        decline(&home, "slack").unwrap();
        decline(&home, "docs").unwrap();
        let set = read_declined(&home).unwrap();
        assert_eq!(set.len(), 2, "declining twice declines once");
        assert!(set.contains("slack") && set.contains("docs"));

        undecline(&home, Some("slack")).unwrap();
        assert_eq!(
            read_declined(&home)
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            ["docs"]
        );
        undecline(&home, None).unwrap();
        assert!(read_declined(&home).unwrap().is_empty());

        // A file that is there but is not a decline store is `None`, so the
        // caller can say so — not an empty set, which would read as "you
        // have declined nothing" about a document nobody could parse.
        std::fs::write(declined_path(&home), "{not json").unwrap();
        assert_eq!(read_declined(&home), None);

        let _ = std::fs::remove_dir_all(&home);
    }

    /// **An unreadable store is kept, never overwritten.**
    ///
    /// The bug this fails on: `decline` collapsed `read_declined`'s `None`
    /// to an empty set and then *persisted* it, so somebody with a typo in
    /// `setup-declined.json` who answered `never` to one step had the file
    /// rewritten with exactly that one id and every earlier answer gone —
    /// with no message beyond the "could not be read" line they had already
    /// seen and reasonably read as "so nothing is recorded".
    #[test]
    fn declining_over_an_unreadable_store_keeps_the_old_bytes() {
        let home = std::env::temp_dir().join(format!(
            "mecha-declined-salvage-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();

        let damaged = r#"{"declined": ["slack", "docs"] "#; // truncated
        std::fs::write(declined_path(&home), damaged).unwrap();
        assert_eq!(
            read_declined(&home),
            None,
            "the fixture is genuinely unreadable"
        );

        let salvaged = decline(&home, "mail")
            .unwrap()
            .salvaged
            .expect("the old file is kept");
        assert_eq!(
            std::fs::read_to_string(&salvaged).unwrap(),
            damaged,
            "kept byte for byte — a salvage that rewrites is not a salvage"
        );

        // The new store is well-formed and holds the answer just given.
        let now = read_declined(&home).unwrap();
        assert_eq!(now.into_iter().collect::<Vec<_>>(), ["mail"]);

        // And the ordinary path reports no salvage, so a caller cannot print
        // the warning on every decline.
        assert_eq!(decline(&home, "slack").unwrap().salvaged, None);

        // `undecline` takes the same care: it also writes the whole document.
        std::fs::write(declined_path(&home), damaged).unwrap();
        assert!(
            undecline(&home, Some("slack")).unwrap().salvaged.is_some(),
            "the undo path overwrites the same file and must salvage too"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// **`changed` is read off the set, never off the argument.**
    ///
    /// `undecline` discarded `BTreeSet::remove`'s answer, so a typo'd id
    /// wrote the set back untouched while the caller announced the restore
    /// and exited 0 — the person then met `you said no thanks` on the step
    /// they thought they had just brought back.
    #[test]
    fn a_write_reports_what_changed_rather_than_what_was_asked() {
        let home = std::env::temp_dir().join(format!(
            "mecha-declined-changed-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();

        assert!(
            decline(&home, "slack").unwrap().changed,
            "a new decline changes it"
        );
        assert!(
            !decline(&home, "slack").unwrap().changed,
            "declining twice is idempotent, and the second one changed nothing"
        );

        // The finding: an id nobody declined.
        assert!(
            !undecline(&home, Some("slak")).unwrap().changed,
            "a typo restores nothing, and must not report that it did"
        );
        assert!(
            undecline(&home, Some("slack")).unwrap().changed,
            "and a real one does"
        );

        // `all` over an empty set is vacuous rather than false, but it is
        // still not a restore — saying so costs a word and saves a wrong
        // belief.
        assert!(!undecline(&home, None).unwrap().changed);
        decline(&home, "docs").unwrap();
        assert!(undecline(&home, None).unwrap().changed);

        let _ = std::fs::remove_dir_all(&home);
    }

    /// A value read off a stranger's `/props` cannot produce a config that
    /// will not parse.
    ///
    /// `format!("{alias:?}")` is *Rust* escaping: a control character renders
    /// as `\u{1b}`, and TOML's `\u` takes four hex digits with no braces, so
    /// the written file is a parse error — and every later `mecha` command
    /// then dies at `Config::load_global` with a message pointing at
    /// `mecha config init` rather than at what happened.
    #[test]
    fn a_model_alias_is_escaped_for_toml_rather_than_for_rust() {
        use crate::provider::preflight::Props;

        for alias in [
            "qwen3-14b",
            "has \"quotes\"",
            "has\\backslash",
            // The one that bites: `Debug` writes `\u{1b}`, TOML cannot read it.
            "esc\u{1b}ape",
            "new\nline",
            "tab\there",
        ] {
            let props = Props {
                model_alias: Some(alias.to_string()),
                ..Props::default()
            };
            let rendered = verified_settings(&props)
                .into_iter()
                .find(|(k, _)| *k == "model")
                .expect("a named model is written down")
                .1;

            // Round-tripped through the parser that will actually read it,
            // rather than eyeballed: the question is whether a *run* can load
            // the file, so ask the same reader.
            let doc: toml::Table = format!("model = {rendered}")
                .parse()
                .unwrap_or_else(|e| panic!("{alias:?} rendered as {rendered} — unparseable: {e}"));
            assert_eq!(
                doc["model"].as_str(),
                Some(alias),
                "value survived the trip"
            );
        }
    }

    /// `charter_state` answers the question a run would ask, not "is there
    /// a file" — a template nobody filled in loads fine and supplies
    /// nothing.
    #[test]
    fn charter_state_reads_what_a_run_would_get() {
        let dir = std::env::temp_dir().join(format!(
            "mecha-charter-state-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("charter.toml");

        assert_eq!(charter_state(&path), CharterState::Absent);

        std::fs::write(&path, crate::charter::TEMPLATE).unwrap();
        assert_eq!(
            charter_state(&path),
            CharterState::Empty,
            "the shipped template must parse to zero lines, or it is authoring priorities"
        );

        std::fs::write(&path, "[[line]]\nid = \"a\"\ntext = \"b\"\n").unwrap();
        assert_eq!(charter_state(&path), CharterState::Lines(1));

        std::fs::write(&path, "[[lines]]\nid = \"a\"\n").unwrap();
        assert!(matches!(charter_state(&path), CharterState::Broken(_)));

        let _ = std::fs::remove_dir_all(&dir);
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
        f.features = rows(|ff| {
            switch_on(ff, &["mail"]);
            ff.config.mcp.push(mcp_entry("mail", "mecha-mail"));
            ff.mail_accounts = None;
        });
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
        let mut f = facts(Some(props(262144, 4, true)));
        // Every shape a graph step can take: unanswered, switched on with
        // nothing installed, and switched on with the binary but no entry.
        let mut steps = Vec::new();
        for tweak in [(false, false), (true, false), (true, true)] {
            f.features = rows(|ff| {
                if tweak.0 {
                    switch_on(ff, &["graph"]);
                }
                ff.has_graph_binary = tweak.1;
            });
            steps.extend(plan(&cfg_with_local(262144, Some(true)), "local", &f));
        }
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
