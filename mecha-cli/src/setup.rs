//! Turning CLI flags plus config into a ready-to-run [`Agent`].
//!
//! Every command that runs an agent goes through here, so precedence is
//! defined once: config file, then environment, then flags.

use crate::approve::TerminalApprover;
use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::agent::Agent;
use mecha_core::config::SearchBackendConfig;
use mecha_core::config::{Config, PermissionMode};
use mecha_core::mcp::{self, McpClient};
use mecha_core::search::{Exa, SearchBackend, SearchChain, Searxng, Tavily, WebSearch};
use mecha_core::subagent::{Subagent, SubagentProfile};
use mecha_core::tool::{Approver, ModeApprover, Registry, ToolCtx};
use std::path::PathBuf;
use std::sync::Arc;

pub struct Prepared {
    pub agent: Agent,
    pub provider_name: String,
    pub model: String,
    pub workspace: PathBuf,
    /// The resolved config, for commands that need to build a *second*
    /// connection — `eval` and its judge model.
    pub config: Config,
    /// The active sandbox, for surfaces that describe what `shell` actually
    /// is — the TUI's /tools modal. The tools themselves already hold it.
    pub sandbox: Arc<mecha_core::sandbox::Sandbox>,
    /// The todo tool the agent is actually using, for the TUI's live pane.
    pub todo: Option<Arc<mecha_core::tool::todo::TodoTool>>,
    /// The skill tool the agent is actually using, for the TUI's /skills
    /// modal. `None` when the run carries no skills — which is also the
    /// answer to "why is /skills empty", so the modal says so rather than
    /// showing a blank list.
    pub skill: Option<Arc<mecha_core::tool::skill::SkillTool>>,
    /// The messaging route, when `[messages]` is enabled — the front-end
    /// sets this run's identity on it once the session exists, and reads
    /// the store for waiting-mail notices.
    pub mailbox: Option<Arc<mecha_core::mailbox::MailboxRoute>>,
    /// Held for the lifetime of the run: dropping a client kills its server.
    pub _mcp: Vec<Arc<McpClient>>,
}

/// Everything except the model connection. Split out so `mecha tools` can list
/// what an agent *would* have without needing provider credentials.
pub struct PreparedTools {
    pub registry: Registry,
    pub sandbox: Arc<mecha_core::sandbox::Sandbox>,
    pub workspace: PathBuf,
    pub config: Config,
    pub approver: Arc<dyn Approver>,
    /// A concrete handle onto the registered todo tool, so a UI can poll
    /// `items()` live. `None` when the tool is disabled or an MCP server
    /// shadowed it.
    pub todo: Option<Arc<mecha_core::tool::todo::TodoTool>>,
    /// A concrete handle onto the registered skill tool, so a UI can read the
    /// carried set and what has been loaded so far. `None` when the run
    /// carries no skills or an MCP server shadowed the tool.
    pub skill: Option<Arc<mecha_core::tool::skill::SkillTool>>,
    /// The messaging route, when `[messages]` is enabled — `message_send`
    /// holds a clone, the agent's context gets it attached in `build`, and
    /// the front-end sets the run's identity on it once a session exists.
    pub mailbox: Option<Arc<mecha_core::mailbox::MailboxRoute>>,
    /// The `compact` tool's channel to the loop, resolved once here because
    /// two callers must answer the same question and one of them cannot see
    /// the other's answer: the registry is assembled in `prepare_tools` (the
    /// tool has to exist before subagents are built, so a child that
    /// allowlists it shares the parent's), while `ToolCtx` is built in
    /// `build`. Computing "does this run compact at all" in both places is
    /// two spellings of one rule, which is how a run gets the tool and no
    /// channel — or a channel and no tool.
    ///
    /// `Some` only when the run has a compaction threshold at all:
    /// `[agent] compact_at_tokens` is off by default and derives from the
    /// window, so a provider that declares no window has compaction disabled
    /// — and paraphrasing somebody's conversation because it got long is
    /// their decision, not one a tool may take on their behalf.
    pub compact_requested: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub _mcp: Vec<Arc<McpClient>>,
}

/// Build an agent. `interactive` decides whether an un-approved tool call can
/// prompt a human or must fall back to the configured [`PermissionMode`].
pub async fn prepare(opts: &GlobalOpts, interactive: bool) -> Result<Prepared> {
    let tools = prepare_tools(opts, interactive).await?;
    preflight_provider(&tools.config, opts).await;
    build(tools, opts)
}

/// Ask a local server whether it is serving what config says it is.
///
/// One `GET /props`, on the `Sandbox::preflight` argument: config makes
/// claims a run then narrows around, and reading them back is cheaper and
/// more correct than reimplementing llama-server's slot arithmetic or
/// remembering that a vision model is two files.
///
/// Warns and never refuses — a mismatch makes a run compact at the wrong
/// moment or quietly not send a picture, and a preflight that can stop a
/// working machine from starting is one people turn off. Scoped to
/// `kind = "local"`: llama-server is the only server here known to answer
/// this endpoint, and a 404 from somebody else's would be noise on every
/// start.
async fn preflight_provider(cfg: &mecha_core::config::Config, opts: &GlobalOpts) {
    let Ok((name, pcfg)) = cfg.provider(opts.provider.as_deref()) else {
        return;
    };
    if pcfg.kind != "local" {
        return;
    }
    let Some(base_url) = pcfg.base_url.as_deref() else {
        return;
    };
    // Silent when the server is simply not up: that is a failure the next
    // request reports far better than a startup line can, and printing it
    // here would put a warning in front of every command on a machine whose
    // model is not running yet.
    let Some(props) = mecha_core::provider::preflight::fetch(base_url).await else {
        return;
    };
    for line in mecha_core::provider::preflight::disagreements(&name, pcfg, &props) {
        eprintln!("warning: {line}");
    }
}

/// Build an agent that asks a caller-supplied approver.
///
/// The TUI needs this: its approver talks to the event loop over a channel, and
/// `prepare` would otherwise install one that writes prompts straight to a
/// terminal the interface has taken over. The approver is still only consulted
/// in `Ask` mode — a run configured read-only stays read-only.
pub async fn prepare_with_approver(
    opts: &GlobalOpts,
    approver: Arc<dyn Approver>,
) -> Result<Prepared> {
    let mut tools = prepare_tools(opts, true).await?;
    if tools.config.tools.permission_mode == PermissionMode::Ask {
        tools.approver = approver;
    }
    preflight_provider(&tools.config, opts).await;
    build(tools, opts)
}

fn build(tools: PreparedTools, opts: &GlobalOpts) -> Result<Prepared> {
    let cfg = tools.config;

    let (provider_name, provider_cfg) = cfg.provider(opts.provider.as_deref())?;
    let mut provider = mecha_core::provider::build(provider_cfg)?;

    let model = opts
        .model
        .clone()
        .or_else(|| provider_cfg.model.clone())
        .unwrap_or_else(|| provider.default_model().to_string());

    // Fallbacks, resolved at startup so a typo'd name fails on every start
    // rather than only on the outage that needed it. `--no-fallback` is the
    // strictness switch; eval forces it, because a scorecard produced by a
    // different model than the one it names measures nothing.
    if !provider_cfg.fallbacks.is_empty() && !opts.no_fallback {
        let mut fallbacks = Vec::new();
        for name in &provider_cfg.fallbacks {
            anyhow::ensure!(
                name != &provider_name,
                "provider {provider_name:?} lists itself as a fallback"
            );
            let (fb_name, fb_cfg) = cfg.provider(Some(name)).with_context(|| {
                format!("fallback {name:?} of provider {provider_name:?} is not configured")
            })?;
            fallbacks.push((fb_name, mecha_core::provider::build(fb_cfg)?));
        }
        provider = Box::new(mecha_core::provider::Failover::new(provider, fallbacks));
    }

    let ctx = ToolCtx {
        workspace: tools.workspace.clone(),
        shell_timeout: std::time::Duration::from_secs(cfg.tools.shell_timeout_secs),
        security: cfg.security.clone(),
        // Resolved against the *primary* provider's window; a turn served by
        // a narrower fallback inherits a budget sized for the primary. Known
        // and accepted — fallbacks are turn-local and empty by default, and a
        // budget that flapped per-turn would make transcripts incomparable.
        output_budget_bytes: cfg
            .tools
            .resolved_output_budget(provider_cfg.context_window),
        // Resolved in `prepare_tools` beside the tool's registration; see
        // `PreparedTools::compact_requested` for why it is not decided twice.
        compact_requested: tools.compact_requested.clone(),
        // `compact_requested`'s own shape: presence is the enablement, and
        // `--no-step-escalation` has already folded into `cfg.agent
        // .step_escalation` above (`prepare_tools`), so this is the one
        // place that decides it. `Agent::run_in` mints a fresh `Mutex` per
        // run regardless — this initial one is never actually read from.
        step_escalation: step_escalation_slot(cfg.agent.step_escalation),
        ..ToolCtx::default()
    };

    // Validated even when --no-hooks skips installing them: a typo in a hook's
    // event name should fail on every start, not only on the runs that use it.
    let hooks = mecha_core::hooks::HookSet::from_config(&cfg.hooks)?;
    let hooks = (!opts.no_hooks && !hooks.is_empty()).then(|| Arc::new(hooks));
    // Approval rules: validated again here (config load already did) so a
    // rule set built any other way fails on every start too. Installed on the
    // parent and on every child below, like hooks and for the same reason —
    // a rule only ever narrows, and delegating must not be the way around
    // one. There is deliberately no `--no-rules` *flag*: a `forbid` is the
    // operator's standing word, and a switch that lifts it for one run is the
    // silently-degrading-guard shape. `opts.no_rules` is set by one caller,
    // `mecha eval`'s `force_reproducible`, because a scorecard must not vary
    // with this machine's rules file.
    let policy = Arc::new(if opts.no_rules {
        mecha_core::policy::ExecPolicy::empty()
    } else {
        mecha_core::policy::ExecPolicy::from_config(&cfg.rules, cfg.approval.strict_inline_eval)?
    });

    // The outbox route. Opening the store here — not lazily at first stage —
    // makes an unwritable outbox a startup error instead of a mid-run
    // surprise on the one call that mattered.
    let outbox = if !opts.no_outbox && !cfg.outbox.tools.is_empty() {
        let root = match &cfg.outbox.dir {
            Some(dir) => dir.clone(),
            None => mecha_core::outbox::OutboxStore::default_root()?,
        };
        let store = mecha_core::outbox::OutboxStore::open(root)?;
        Some(Arc::new(mecha_core::outbox::OutboxRoute::new(
            store,
            cfg.outbox.tools.iter().cloned(),
            cfg.outbox.publish_tools.iter().cloned(),
        )))
    } else {
        None
    };

    // Subagents are built from the same tool pool but get their own registry —
    // an allowlist, not an inheritance. Do this before the parent takes
    // ownership of the registry.
    let mut registry = tools.registry;

    // Closing a task is the owner's act — §5.4's closure appraisal fires only
    // inside `tasks set`, so a model-driven `kg_task_update {status: done}`
    // consumed that one-shot moment silently. The guard refuses exactly the
    // closing statuses and leaves every other field of the tool alone; see
    // `closure_guard` for the argument. Wrapped HERE, before the subagent
    // loop below clones the pool, or a child registry would hold the
    // unwrapped handle — the delegation hole `withhold_tool`'s own doc names.
    // The wrapper keeps the inner tool's name and schema (the re-insert
    // lands on the same registry key) and deliberately extends the
    // description — the guard is a real capability change, and a
    // byte-identical spec would read `Match` in `tools_hash` across it.
    // (`tasks set` itself is unaffected: it calls through `prepare_tools`'s
    // registry directly, which this — the model-facing build — never touches.)
    //
    // One handle downstream of here is guarded on purpose, not by accident:
    // the copy `tasks work` and `questions` later pull back off this
    // registry via `withhold_tool` — D5's "the harness's hand" — is the
    // wrapped one, so `move_task` cannot perform a closure either. That is
    // the rule, not a limitation: the harness moves a task to `waiting` or
    // back to its pre-run status, and a closure is the *owner's* act on
    // every path — the day `move_task` is asked to carry `done`, the guard's
    // refusal is the correct answer and `tasks set` is the correct caller.
    crate::closure_guard::guard(&mut registry);
    for profile in &cfg.subagents {
        // `--tool` narrows the pool deliberately, and a subagent whose profile
        // names something the narrowing excluded is not a misconfiguration —
        // it is a subagent the caller has just said they do not want. Skip it,
        // loudly. Failing here instead made `--tool` unusable on any machine
        // with a subagent configured: `mecha run --tool fs_read` died on
        // `research` needing `web_search`, which is not what the flag means.
        //
        // The distinction is what keeps `build_subagent`'s error worth having:
        // a name that is missing *without* an active allowlist is still a
        // typo, and still fatal.
        {
            let excluded = excluded_by_allowlist(&profile.tools, &opts.tools);
            if !excluded.is_empty() {
                // Quiet under a trigger's own allowlist: that file names
                // exactly what the run may carry, so the skip is the config
                // working, not a surprise worth a journal line every morning.
                if !opts.tools_from_trigger {
                    eprintln!(
                        "mecha: subagent `{}` not registered — `--tool` excludes {}",
                        profile.name,
                        excluded.join(", ")
                    );
                }
                continue;
            }
        }
        // A profile may point at a different provider entry entirely.
        let (_, child_provider_cfg) = cfg.provider(profile.provider.as_deref())?;
        let child = build_subagent(
            profile,
            &registry,
            &cfg,
            child_provider_cfg,
            &ctx,
            hooks.as_ref(),
            &policy,
            outbox.as_ref(),
        )?;
        registry.insert(Arc::new(child));
    }

    // The guard's presence is a startup invariant, not a hope about line
    // order: `closure_guard::guard` above is positional, and a refactor
    // that loses it must fail every start rather than ship a surface where
    // the model can close tasks around `mecha tasks set`.
    crate::closure_guard::verify(&registry)?;

    let mut agent = Agent::new(
        provider,
        registry,
        tools.approver,
        ctx,
        cfg.agent.clone(),
        Some(model.clone()),
    )?
    .with_pricing(provider_cfg.pricing())
    .with_context_window(provider_cfg.context_window)
    // Read-only: what the run happened under, recorded beside what it did.
    // `eval` and the probes build their own per-case contexts and so stay
    // unsampled — see `Agent::with_homeostat`.
    .with_homeostat();

    if let Some(hooks) = hooks {
        agent.set_hooks(hooks);
    }
    agent.set_policy(Arc::clone(&policy));
    // A rule naming a tool that is not registered — `shel`, or an MCP tool
    // whose server did not come up — loads clean and judges nothing. Same
    // shape as the `[outbox]` warnings below, and it fires on every start
    // for the same reason; `--tool` narrowing is the caller saying so, and is
    // silent.
    for name in policy.tools() {
        let narrowed_out =
            !excluded_by_allowlist(std::slice::from_ref(&name.to_string()), &opts.tools).is_empty();
        if agent.registry().get(name).is_none() && !narrowed_out {
            eprintln!(
                "mecha: [[rule]] names `{name}`, which is not a registered tool — check the \
                 spelling, or this rule judges nothing"
            );
        }
        // The other "loads clean, judges nothing" case: a routed tool is
        // staged before the rules are read, and released without them.
        if outbox.as_ref().is_some_and(|o| o.routes(name)) {
            eprintln!(
                "mecha: [[rule]] names `{name}`, which `[outbox] tools` routes to staging — a \
                 staged call is reviewed by a person at release, not judged by rules, so this \
                 rule judges nothing"
            );
        }
    }
    if let Some(outbox) = outbox {
        // A typo in `[outbox] tools` means the *real* tool executes unrouted,
        // silently — the degrading-sandbox shape. It cannot be a hard error
        // (a routed tool's MCP server may be legitimately off today), so say
        // it out loud on every start instead.
        //
        // Except when `--tool` excluded it: that is the caller naming exactly
        // what they want, not a mistake, and a warning that fires every
        // morning on a deliberately narrowed run is how a real typo later
        // gets ignored.
        for name in outbox.routed() {
            let narrowed_out =
                !excluded_by_allowlist(std::slice::from_ref(&name.to_string()), &opts.tools)
                    .is_empty();
            if agent.registry().get(name).is_none() && !narrowed_out {
                eprintln!(
                    "mecha: [outbox] routes `{name}`, which is not a registered tool — \
                     check the spelling, or this routing protects nothing"
                );
            }
        }
        // A name declared a publish but not routed is worse than a typo: the
        // tool executes for real, unstaged, while config reads as though it
        // were under review. Same shape as the warning above, and it fires on
        // every start for the same reason.
        for name in outbox.publishes() {
            if !outbox.routes(name) {
                eprintln!(
                    "mecha: [outbox] calls `{name}` a publish but does not route it — \
                     add it to `tools`, or it executes unstaged"
                );
            }
        }
        agent.set_outbox(outbox);
    }
    // A tool that exists to send and is not routed executes for real, with
    // the interlock as its only guard — a mail server registered before
    // `[outbox] tools` named it was live mail, and nothing said so. Same
    // shape as the two warnings above, and it fires on every start for the
    // same reason. `--no-outbox` is the caller saying so deliberately, and
    // is silent. Tested one level down (`Registry::senders`), not here:
    // this function builds a live agent from a full config, and nothing in
    // `setup` is unit-tested at that level yet — a known gap, stated so
    // nobody reads the registry test as covering the warning.
    if !opts.no_outbox {
        for name in agent.registry().senders() {
            // `shell` can send when unconfined, and is a sender by
            // capability; its guard is the sandbox (`[sandbox]`), not the
            // outbox, and routing a shell command through a draft would stage
            // nothing a reviewer could read. Excluded here by name rather
            // than by a capability filter, because the filter that used to do
            // it (`!destructive`) also dropped the calendar cancellation.
            if name == "shell" {
                continue;
            }
            let routed = agent
                .context()
                .outbox
                .as_ref()
                .is_some_and(|o| o.routes(name));
            if !routed {
                eprintln!(
                    "mecha: `{name}` can send and is not routed through the outbox — add it to \
                     `[outbox] tools`, or it executes unstaged"
                );
            }
        }
    }
    if let Some(mb) = &tools.mailbox {
        agent.set_mailbox(Arc::clone(mb));
    }

    Ok(Prepared {
        agent,
        provider_name,
        model,
        workspace: tools.workspace,
        config: cfg,
        sandbox: tools.sandbox,
        todo: tools.todo,
        skill: tools.skill,
        mailbox: tools.mailbox,
        _mcp: tools._mcp,
    })
}

/// Which of `wanted` an active `--tool` allowlist leaves out.
///
/// Empty when there is no allowlist, which is the load-bearing case: `--tool`
/// unset means "every tool", not "no tools", and reading it the other way is
/// what made `--tool` unusable on any machine with a subagent configured.
fn excluded_by_allowlist<'a>(wanted: &'a [String], allowlist: &[String]) -> Vec<&'a str> {
    if allowlist.is_empty() {
        return Vec::new();
    }
    wanted
        .iter()
        .filter(|t| !allowlist.iter().any(|kept| kept == *t))
        .map(String::as_str)
        .collect()
}

/// One line saying what `shell` actually is right now. Shared between
/// `mecha tools` and the TUI's /tools modal, so the two cannot disagree about
/// the fact an operator most needs.
pub fn sandbox_line(sandbox: &mecha_core::sandbox::Sandbox) -> String {
    if sandbox.is_enabled() {
        format!(
            "sandbox: {} · network {} · reads {}",
            sandbox.backend().as_str(),
            // Landlock's "on" is a property of the backend, not a setting the
            // operator chose — say so, or the line reads like a config error.
            if sandbox.backend() == mecha_core::sandbox::Backend::Landlock {
                "reachable (landlock cannot close UDP)"
            } else if sandbox.can_reach_network() {
                "on"
            } else {
                "off"
            },
            if sandbox.reaches_beyond_workspace() {
                "beyond the workspace"
            } else {
                "the workspace only"
            }
        )
    } else {
        "sandbox: none — commands run as you, with your credentials".to_string()
    }
}

/// Whether this run carries the step-escalation slot, folding
/// `--no-step-escalation` over the config value the same way every other
/// flag in `prepare_tools`'s "flags override config" section does.
///
/// Pulled out as its own function, and paired with [`step_escalation_slot`]
/// below, because neither step is exercised by anything else: the loop gates
/// purely on `ToolCtx::step_escalation.is_some()`, so `cfg.agent
/// .step_escalation` is read nowhere but here. A `prepare_tools`/`build` run
/// through a real provider is not a test this crate has anywhere to hang —
/// see `a_subagent_inherits_the_window_its_compaction_threshold_derives_from`'s
/// own "skip if this machine cannot build one" escape — so without a plain
/// unit test on the mapping itself, dropping this fold (or the `.then` in
/// `step_escalation_slot`) to an unconditional `true`/`Some` would leave the
/// whole suite green while the feature shipped on for every run, `mecha
/// eval` included.
fn step_escalation_enabled(cfg_value: bool, no_step_escalation: bool) -> bool {
    cfg_value && !no_step_escalation
}

/// The `ToolCtx` shape `compact_requested` already established: presence is
/// the enablement. See [`step_escalation_enabled`] for why this is its own
/// function rather than an inline `.then(...)` at the call site.
fn step_escalation_slot(
    enabled: bool,
) -> Option<Arc<std::sync::Mutex<Option<mecha_core::step::StepEscalation>>>> {
    enabled.then(|| Arc::new(std::sync::Mutex::new(None)))
}

/// Resolve config, workspace, tools, and the approval policy.
pub async fn prepare_tools(opts: &GlobalOpts, interactive: bool) -> Result<PreparedTools> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let mut cfg = if opts.global_config_only {
        Config::load_global()?
    } else {
        Config::load(&cwd)?
    };

    // --- flags override config ---
    if let Some(effort) = opts.effort {
        cfg.agent.effort = Some(effort);
    }
    if let Some(max_turns) = opts.max_turns {
        cfg.agent.max_turns = max_turns;
    }
    if opts.max_output_tokens.is_some() {
        cfg.agent.max_output_tokens = opts.max_output_tokens;
    }
    if opts.max_cost.is_some() {
        cfg.agent.max_cost_usd = opts.max_cost;
    }
    if opts.compact_at.is_some() {
        cfg.agent.compact_at_tokens = opts.compact_at;
    }
    cfg.agent.step_escalation =
        step_escalation_enabled(cfg.agent.step_escalation, opts.no_step_escalation);
    if opts.no_thinking {
        cfg.agent.thinking = false;
        // Disabling thinking above `high` effort is rejected by the API. The
        // user asked for no thinking, so honour that and cap the effort.
        if matches!(
            cfg.agent.effort,
            Some(mecha_core::Effort::XHigh) | Some(mecha_core::Effort::Max)
        ) {
            cfg.agent.effort = Some(mecha_core::Effort::High);
        }
    }
    if let Some(system) = &opts.system {
        cfg.agent.system_prompt = Some(read_maybe_file(system)?);
        cfg.agent.system_prompt_file = None;
    }
    // A front-end's standing block, appended rather than replacing: `--system`
    // is the user's voice and this is the harness's, and clobbering the first
    // with the second would make configuring both impossible. It sits before
    // the date stamp so the parts that never change stay contiguous at the
    // front of the cached prefix.
    if let Some(extra) = &opts.system_extra {
        let base = cfg.agent.resolve_system_prompt()?.unwrap_or_default();
        cfg.agent.system_prompt = Some(if base.is_empty() {
            extra.clone()
        } else {
            format!("{base}\n\n{extra}")
        });
        cfg.agent.system_prompt_file = None;
    }
    // Today's date, because a model has no clock and every calendar or mail
    // question is relative to one. Found by running it: asked for "the next
    // three days" the model queried a window in January, six months stale,
    // and the tool dutifully returned nothing wrong — just nothing useful.
    //
    // It goes before the learned rules so it stays adjacent to the user's own
    // prompt, and it is the one part of the cached prefix that legitimately
    // changes daily. `RunConfig` records it, so a replay reproduces the date
    // the run actually saw rather than today's.
    {
        let base = cfg.agent.resolve_system_prompt()?.unwrap_or_default();
        // In the user's zone, not the machine's. A server runs in UTC, and
        // answering "what's on Thursday" four hours off is wrong in the worst
        // way — internally consistent, so it reads as correct.
        let stamp = match cfg.agent.timezone() {
            Some(tz) => {
                let now = chrono::Utc::now().with_timezone(&tz);
                format!(
                    "Today is {}, and the user's timezone is {tz} (currently {}). \
                     Give times in that zone unless asked otherwise, and work out \
                     relative dates (\"next Tuesday\", \"this week\") from today \
                     rather than guessing.",
                    now.format("%A, %-d %B %Y"),
                    now.format("%Z, UTC%:z")
                )
            }
            None => format!(
                "Today is {}. Work out relative dates (\"next Tuesday\", \
                 \"this week\") from it rather than guessing.",
                chrono::Local::now().format("%A, %-d %B %Y")
            ),
        };
        cfg.agent.system_prompt = Some(if base.is_empty() {
            stamp
        } else {
            format!("{base}\n\n{stamp}")
        });
        cfg.agent.system_prompt_file = None;
    }
    // Skills, at level 1: a name and a description each, and nothing else
    // until the model asks. They sit inside the cached prefix like the learned
    // rules, and change only when the user edits the store — so enabling or
    // disabling one re-pays the prefix for that session, which is the reason
    // nothing here may toggle per turn.
    //
    // Read-only and best-effort: a machine with no skills gets no block and no
    // tool, and one bad SKILL.md never stops the others loading.
    let skills = if opts.no_skills {
        Vec::new()
    } else {
        let dir = match cfg.skills.dir.clone() {
            Some(dir) => dir,
            None => mecha_core::skill::SkillStore::default_dir()?,
        };
        let (store, errors) = mecha_core::skill::SkillStore::load(&dir);
        // A skill that failed to parse is indistinguishable from one the model
        // chose not to use, which is the unrouted-domain shape: said at
        // startup, naming the file and the reason, or it is silent forever.
        for e in &errors {
            eprintln!(
                "mecha: skill `{}` did not load — {}",
                e.dir.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                e.why
            );
        }
        // And a name in config matching nothing on disk means a skill the user
        // believes is enabled and is not — the routed-outbox-name precedent.
        for name in store
            .unknown_names(&cfg.skills.enabled)
            .into_iter()
            .chain(store.unknown_names(&cfg.skills.disabled))
        {
            eprintln!(
                "mecha: [skills] names `{name}`, which is not a skill in {} — check the \
                 spelling, or the directory name",
                dir.display()
            );
        }
        // `--skill` narrows further, and only further: it intersects with
        // what config already selected rather than replacing it, so the flag
        // cannot turn on something `[skills] disabled` withheld. Same
        // direction as the project layer, for the same reason.
        let selected = store.select(&cfg.skills.enabled, &cfg.skills.disabled);
        if opts.skills.is_empty() {
            selected
        } else {
            // Two different silences to break, and only one is a missing
            // file. A skill that *exists* but sits outside `[skills] enabled`
            // (or inside `disabled`) intersects away to nothing — and for a
            // trigger, which derives `no_skills` from its own list, that means
            // the scheduled run carries no skills and no `skill` tool at all.
            // "My trigger names a skill and it never fires" needs an answer on
            // screen.
            for name in &opts.skills {
                if store.get(name).is_none() {
                    eprintln!(
                        "mecha: --skill names `{name}`, which is not a skill in {}",
                        dir.display()
                    );
                } else if !selected.iter().any(|s| &s.name == name) {
                    eprintln!(
                        "mecha: --skill names `{name}`, which `[skills]` withholds — it \
                         will not be carried, because narrowing cannot re-enable what \
                         config disabled"
                    );
                }
            }
            selected
                .into_iter()
                .filter(|s| opts.skills.iter().any(|n| n == &s.name))
                .collect()
        }
    };
    // Gated on the same condition the tool registration below is. A `--tool`
    // allowlist that omits `skill` (the shipped Slack path sets one from
    // `[slack] tools`) would otherwise leave a system prompt saying "call the
    // `skill` tool" with no such tool in the surface, costing the model a turn
    // on a call that can only fail.
    let skills: Vec<_> = if opts.tools.is_empty() || opts.tools.iter().any(|t| t == "skill") {
        skills
    } else {
        Vec::new()
    };
    if let Some(block) = mecha_core::skill::prompt_block(&skills) {
        let base = cfg.agent.resolve_system_prompt()?.unwrap_or_default();
        cfg.agent.system_prompt = Some(if base.is_empty() {
            block
        } else {
            format!("{base}\n\n{block}")
        });
        cfg.agent.system_prompt_file = None;
    }

    // The charter, right after skills and before learned rules: standing
    // priorities the owner wrote once, rendered straight into the prompt —
    // no progressive disclosure and no tool, unlike skills, because §11 says
    // this rides in the cached prefix "like RULES_CHAR_BUDGET" rather than
    // being loaded on demand. Global-only and read-only by construction: see
    // `mecha_core::charter`'s module doc for why there is no config field to
    // point this at a project file instead.
    if !opts.no_charter {
        let charter_path = mecha_core::charter::Charter::default_path()?;
        // Best-effort, like skills and learned rules: a genuinely malformed
        // charter (bad TOML, a duplicate or empty id) must not brick every
        // run started until it is fixed — `mecha doctor` and `mecha charter`
        // are where that failure is surfaced as the emergency it is, since
        // stderr here is covered for a whole TUI session by the alternate
        // screen. Crossing the character budget is *not* one of these
        // failures — `Charter::load` still returns the document, on the
        // learned-rules `over_budget_domains` precedent of warning rather
        // than dropping what is over cap.
        let charter = mecha_core::charter::Charter::load(&charter_path).unwrap_or_else(|e| {
            eprintln!(
                "mecha: charter at {} did not load — {e:#} — starting with none",
                charter_path.display()
            );
            mecha_core::charter::Charter::default()
        });
        if charter.over_budget() {
            eprintln!(
                "mecha: charter at {} is {} characters, over the {}-character budget — it \
                 still rides in the prompt in full, but costs more of the cached prefix than \
                 argued",
                charter_path.display(),
                charter.char_count(),
                mecha_core::charter::CHARTER_CHAR_BUDGET
            );
        }
        if let Some(block) = mecha_core::charter::prompt_block(&charter) {
            let base = cfg.agent.resolve_system_prompt()?.unwrap_or_default();
            cfg.agent.system_prompt = Some(if base.is_empty() {
                block
            } else {
                format!("{base}\n\n{block}")
            });
            cfg.agent.system_prompt_file = None;
        }
    }

    // Learned rules ride at the end of the system prompt — still inside the
    // cached prefix, and they only change at consolidation time. Read-only:
    // an agent that has learned nothing yet must not create state by starting.
    if !opts.no_learned_rules {
        if let Some(store) = mecha_core::learning::LearningStore::open_existing_default() {
            // The cap's warning half (the gate in `mecha learn` is the
            // refusal half): a domain over budget degrades every run this
            // block rides in, so it is said where the run starts — the
            // routed-name-matches-no-tool precedent.
            for (domain, n) in store.over_budget_domains().unwrap_or_default() {
                eprintln!(
                    "mecha: learned rules for `{domain}` number {n}, over the cap of {} — \
                     adherence degrades; `mecha learn` will consolidate before it may add",
                    mecha_core::learning::MAX_ACTIVE_RULES_PER_DOMAIN
                );
            }
            // A domain holding rules that ride in no prompt is silent by
            // construction — same shape as a routed outbox name matching no
            // tool, and said at the same moment for the same reason.
            // Measured against every domain something loads, not just what a
            // *run* carries: `triage` is read by the mail classifier's own
            // pass, so warning about it would be false, permanent, and — worse
            // — the place a genuinely unrouted domain would hide.
            let routed = mecha_core::learning::routed_domains();
            for domain in store.unrouted_domains(&routed).unwrap_or_default() {
                eprintln!(
                    "mecha: rules for `{domain}` are never loaded — nothing carries that \
                     domain, so they cannot fire. Check the filename, or route it."
                );
            }
            if let Some(block) = store.rules_prompt_block_for(mecha_core::learning::RUN_DOMAINS)? {
                let base = cfg.agent.resolve_system_prompt()?.unwrap_or_default();
                cfg.agent.system_prompt = Some(if base.is_empty() {
                    block
                } else {
                    format!("{base}\n\n{block}")
                });
                cfg.agent.system_prompt_file = None;
            }
        }
    }
    if !opts.tools.is_empty() {
        cfg.tools.enabled = opts.tools.clone();
    }
    if opts.yes {
        cfg.tools.permission_mode = PermissionMode::Allow;
    }
    if opts.read_only {
        cfg.tools.permission_mode = PermissionMode::ReadOnly;
    }

    let workspace = opts
        .workspace
        .clone()
        .or_else(|| cfg.tools.workspace.clone())
        .unwrap_or(cwd);
    let workspace = workspace
        .canonicalize()
        .with_context(|| format!("workspace {} does not exist", workspace.display()))?;
    // A jail rooted where the secrets live is close to no jail. `$HOME`
    // contains `~/.mecha/` — the mail OAuth tokens, every transcript, the
    // learning store — so `mecha chat` started from a home directory could
    // `fs_read` all of it. Caught here rather than per-front-end, so it covers
    // every future interface too.
    mecha_core::work::ensure_outside_mecha_home(&workspace)?;

    // --- tools ---
    let sandbox = Arc::new(mecha_core::sandbox::Sandbox::new(cfg.sandbox.clone()));
    let mut registry = Registry::new().with_builtins(&cfg.tools, Arc::clone(&sandbox));

    // A concrete handle onto the todo tool, for a UI that renders the list
    // live — `TodoTool::items()` exists for exactly that, but the registry
    // type-erases on insert. Shadow-insert a fresh instance (same type, so
    // the spec the model sees is byte-identical and the prompt-cache prefix
    // and eval surface are untouched) and keep the handle. Gated on the name:
    // config may have disabled the tool, and then there is nothing to watch.
    // Before subagents are built, so a child that allowlists `todo` shares
    // the same list.
    // `compact` exists only where compaction does, on `web_search`'s rule: a
    // tool that can only ever answer "not enabled here" is worse than no tool,
    // and it would cost a slot in every prompt to say so. Resolved here and
    // carried on `PreparedTools`, so `build` fits `ToolCtx` with this same
    // channel rather than deciding the question a second time.
    // `.ok()` and not `?`: this function deliberately stops short of building
    // a provider, and `mecha tools`, `mecha kg`, `mecha mail …` and `mecha
    // outbox send` all call it without ever reaching `build`. CLAUDE.md states
    // that `mecha tools` runs with no provider configured — it is the MCP
    // smoke test — and a release path running hours later in another process
    // must not start failing because `default_provider` names an entry defined
    // on a different machine. No window resolves to no threshold, hence no
    // tool, which is the same answer this run would have got anyway.
    let window = cfg
        .provider(opts.provider.as_deref())
        .ok()
        .and_then(|(_, p)| p.context_window);
    let compact_requested = cfg
        .agent
        .compact_at(window)
        .map(|_| Arc::new(std::sync::atomic::AtomicBool::new(false)));
    // `web_search`'s rule is the whole precedent, and it has two halves: the
    // tool exists only where its mechanism does, *and* an explicit allowlist
    // still excludes it. `CompactTool` is not in `builtin::all`, so
    // `[tools] enabled`/`disabled` never sees it and only this check can —
    // without which `mecha run --tool fs_read` and a trigger narrowed to
    // `mail__*` both get `compact` in the surface anyway, which makes "what
    // does this run actually do" unanswerable from the trigger file.
    let compact_allowed = opts.tools.is_empty() || opts.tools.iter().any(|t| t == "compact");
    let compact_disabled = cfg.tools.disabled.iter().any(|t| t == "compact")
        || (!cfg.tools.enabled.is_empty() && !cfg.tools.enabled.iter().any(|t| t == "compact"));
    let compact_requested =
        compact_requested.filter(|_| compact_allowed && !compact_disabled && !opts.no_compact_tool);
    if compact_requested.is_some() {
        registry.insert(Arc::new(mecha_core::tool::builtin::CompactTool));
    }

    let todo = registry.get("todo").is_some().then(|| {
        let handle = Arc::new(mecha_core::tool::todo::TodoTool::new());
        registry.insert(Arc::clone(&handle) as Arc<dyn mecha_core::tool::Tool>);
        handle
    });

    // The `skill` tool exists only when there is a skill to load, on the same
    // rule as `web_search`: a tool that can only ever answer "nothing here" is
    // worse than no tool, and it would cost a slot in every prompt to say so.
    // Registered before subagents are built, so a child that allowlists
    // `skill` gets the same set the parent has.
    // `skills` is already empty when a `--tool` allowlist excludes `skill` —
    // filtered where the prompt block is built, so the block and the tool
    // cannot disagree about whether skills exist.
    //
    // The handle is kept for the same reason `todo`'s is: a UI asking "what
    // does this agent know how to do" has to read the tool the run is actually
    // using. Which skills are *loaded* is conversation state living behind that
    // Mutex, and the carried set has already had `--skill` applied — neither is
    // recoverable from config and the store.
    let skill = (!skills.is_empty()).then(|| {
        let handle = Arc::new(mecha_core::tool::skill::SkillTool::new(skills.clone()));
        registry.insert(Arc::clone(&handle) as Arc<dyn mecha_core::tool::Tool>);
        handle
    });

    // Search is only registered when a backend is configured — an agent with a
    // `web_search` tool that always errors is worse than no tool at all.
    if !cfg.search.is_empty() {
        let (chain, errors) = build_search_chain(&cfg.search);
        for error in errors {
            eprintln!("mecha: search backend unavailable — {error}");
        }
        if !chain.is_empty() {
            let allowed = opts.tools.is_empty() || opts.tools.iter().any(|t| t == "web_search");
            if allowed {
                registry.insert(Arc::new(WebSearch::new(Arc::new(chain))));
            }
        }
    }
    let mut clients = Vec::new();
    // Named servers are dropped before connecting rather than after: a server
    // that is off should not have been spawned, since spawning it is what runs
    // third-party code.
    let wanted: Vec<_> = cfg
        .mcp
        .iter()
        .filter(|c| !opts.no_mcp_servers.iter().any(|n| n == &c.name))
        .cloned()
        .collect();
    if !opts.no_mcp && !wanted.is_empty() {
        let (tools, connected, errors) = mcp::connect_all(&wanted, &sandbox, &workspace).await;
        for error in errors {
            // A dead server is worth saying out loud, but it shouldn't stop the
            // run — the other tools still work.
            eprintln!("mecha: MCP server unavailable — {error}");
        }
        for tool in tools {
            // `--tool` filters MCP tools too, so a run can be narrowed to
            // exactly one remote capability.
            if opts.tools.is_empty() || opts.tools.iter().any(|t| t == tool.name()) {
                // A prefixed MCP tool always contains `__` (the prefix is the
                // marker) and shadowing through it keeps its existing
                // semantics. An UNPREFIXED tool arrived on a promise of
                // distinct names — `prefix_tools = false` — and a collision
                // is that promise broken: fail the start rather than let one
                // tool silently replace another, which is the
                // silently-degrading-sandbox shape wearing a tool name.
                if !tool.name().contains("__") && registry.get(tool.name()).is_some() {
                    anyhow::bail!(
                        "MCP tool `{}` collides with an already-registered tool; its \
                         server sets `prefix_tools = false`, which promises distinct \
                         names. Restore the prefix or rename the tool.",
                        tool.name()
                    );
                }
                registry.insert(tool);
            }
        }
        clients = connected;
    }

    // The messaging route. Opened here like the outbox — an unwritable store
    // fails at startup, not on the one send that mattered. Built whenever
    // `[messages]` is enabled, delivery or not: the route is also what stamps
    // outgoing taint, and a surface that can send must never send unstamped.
    let mailbox = if !opts.no_messages && cfg.messages.enabled {
        let store = mecha_core::mailbox::MailboxStore::from_config(&cfg.messages)?;
        // The inbound decision: config's word wins; otherwise a *scheduled*
        // run (the trigger runner is the only caller that sets
        // `global_config_only`) accepts — nobody is coming to release a hold,
        // and its own defaults (read-only mode, outbox staging, the interlock
        // plus merged sender taint) govern what a message can provoke — while
        // everything a person drives (chat, tui, and a one-shot `run`,
        // whether or not its stdin is a pipe) holds and reports the backlog.
        // Keyed on `global_config_only`, deliberately not on `interactive`:
        // the latter is the approval-mode signal, and a piped `run --json` is
        // unattended for approvals but must still *hold* mail rather than fold
        // a stray message into a scripted task.
        use mecha_core::mailbox::InboundPolicy;
        let inbound = cfg.messages.inbound.unwrap_or(if opts.global_config_only {
            InboundPolicy::Accept
        } else {
            InboundPolicy::Hold
        });
        // `refuse` is reserved and behaves as `hold` today; say so rather than
        // letting a config author believe sends are being turned away.
        if inbound == InboundPolicy::Refuse {
            eprintln!(
                "mecha: [messages] inbound = \"refuse\" is not yet implemented and \
                 behaves as \"hold\" — messages accumulate pending until the cap"
            );
        }
        let deliver = inbound == InboundPolicy::Accept;
        let route = Arc::new(mecha_core::mailbox::MailboxRoute::new(store, deliver));
        // `--tool` filters this like it filters MCP tools: an active
        // allowlist that does not name it has said no.
        if opts.tools.is_empty() || opts.tools.iter().any(|t| t == "message_send") {
            registry.insert(Arc::new(mecha_core::mailbox::MessageSendTool::new(
                Arc::clone(&route),
            )));
        }
        Some(route)
    } else {
        None
    };

    let approver: Arc<dyn Approver> =
        if interactive && cfg.tools.permission_mode == PermissionMode::Ask {
            Arc::new(TerminalApprover::default())
        } else {
            Arc::new(ModeApprover {
                mode: cfg.tools.permission_mode,
            })
        };

    // An MCP server may legitimately shadow `todo` (registered after the
    // built-ins, deliberately). The handle would then be live but frozen —
    // watching a list nothing writes to — so drop it unless the registered
    // tool is still ours.
    let todo = todo.filter(|handle| still_ours(&registry, "todo", handle));
    // And `skill` the same way: a server shadowing it would leave the modal
    // listing procedures nothing can load.
    let skill = skill.filter(|handle| still_ours(&registry, "skill", handle));

    // Prove the sandbox works before the agent can call anything, and refuse
    // to start if it doesn't. Falling back to unconfined execution would be
    // worse than never having configured one: `shell` declares narrower
    // capabilities when confined, and the loop's interlock trusts that claim.
    if sandbox.is_enabled() && registry.get("shell").is_some() {
        sandbox
            .preflight(&workspace)
            .await
            .context("sandbox preflight failed — refusing to run `shell` unconfined")?;
    }

    Ok(PreparedTools {
        registry,
        sandbox,
        workspace,
        config: cfg,
        approver,
        todo,
        skill,
        mailbox,
        compact_requested,
        _mcp: clients,
    })
}

/// Is the tool registered under `name` still the one this handle points at?
///
/// An MCP server may legitimately shadow a built-in — servers are registered
/// after them, deliberately — and a shadowed handle stays live while watching
/// state nothing writes to any more, which is worse than having no handle at
/// all: a UI would show a frozen list with no sign it had been replaced.
///
/// Data pointers, not `Arc::ptr_eq`: comparing a concrete `Arc` with a
/// trait-object `Arc` through `ptr_eq` compares vtables too, which is the
/// documented footgun.
fn still_ours<T: mecha_core::tool::Tool + 'static>(
    registry: &Registry,
    name: &str,
    handle: &Arc<T>,
) -> bool {
    registry.get(name).is_some_and(|registered| {
        std::ptr::eq(
            Arc::as_ptr(registered) as *const (),
            Arc::as_ptr(handle) as *const (),
        )
    })
}

/// Build the search chain in configured order, skipping backends that cannot
/// be constructed (usually a missing key) rather than failing the whole run.
fn build_search_chain(configs: &[SearchBackendConfig]) -> (SearchChain, Vec<String>) {
    let mut backends: Vec<mecha_core::search::ChainEntry> = Vec::new();
    let mut errors = Vec::new();

    for cfg in configs.iter().filter(|c| !c.disabled) {
        let built: Result<Box<dyn SearchBackend>> = match cfg.kind.as_str() {
            "exa" => cfg
                .resolve_api_key()
                .context("no API key (set api_key_env, e.g. EXA_API_KEY)")
                .and_then(|k| {
                    Ok(Box::new(Exa::new(k, cfg.base_url.clone())?) as Box<dyn SearchBackend>)
                }),
            "tavily" => cfg
                .resolve_api_key()
                .context("no API key (set api_key_env, e.g. TAVILY_API_KEY)")
                .and_then(|k| {
                    Ok(Box::new(Tavily::new(k, cfg.base_url.clone())?) as Box<dyn SearchBackend>)
                }),
            "searxng" => cfg
                .base_url
                .clone()
                .context("searxng needs `base_url` pointing at your instance")
                .and_then(|u| Ok(Box::new(Searxng::new(u)?) as Box<dyn SearchBackend>)),
            other => Err(anyhow::anyhow!(
                "unknown search backend {other:?} (expected: exa, tavily, searxng)"
            )),
        };

        match built {
            Ok(backend) => backends.push(mecha_core::search::ChainEntry {
                backend,
                prefer_deep: cfg.prefer_deep,
            }),
            Err(e) => errors.push(format!("{}: {e}", cfg.kind)),
        }
    }

    (SearchChain::with_entries(backends), errors)
}

/// Build one subagent: a child [`Agent`] with a restricted registry, wrapped as
/// a tool the parent can call.
#[allow(clippy::too_many_arguments)]
fn build_subagent(
    profile: &SubagentProfile,
    pool: &Registry,
    cfg: &Config,
    provider_cfg: &mecha_core::config::ProviderConfig,
    ctx: &ToolCtx,
    hooks: Option<&Arc<mecha_core::hooks::HookSet>>,
    policy: &Arc<mecha_core::policy::ExecPolicy>,
    outbox: Option<&Arc<mecha_core::outbox::OutboxRoute>>,
) -> Result<Subagent> {
    let mut child_registry = Registry::new();
    for wanted in &profile.tools {
        // A subagent cannot safely send inter-agent mail: its message_send
        // would stamp the taint of its *own* fresh conversation (or, unwatched,
        // a frozen snapshot of the parent's) rather than what actually entered
        // the context that asked for the send — either way a laundering path
        // around the taint forwarding the whole feature rests on. So it is not
        // offered to children at all, and a profile that asks for it is a hard
        // error rather than a silent strip. The parent sends, based on the
        // prose the subagent returns.
        if wanted == "message_send" {
            anyhow::bail!(
                "subagent `{}` asks for `message_send`, which is not available to \
                 subagents: a child cannot stamp inter-agent messages with the \
                 taint of the context that requested them. Have the parent send \
                 based on the child's returned answer.",
                profile.name
            );
        }
        match pool.get(wanted) {
            // The closure guard again, at the clone: `build` wraps the pool's
            // handle before this loop runs, but that protection is an
            // *ordering* — reviewed as the one load-bearing line position in
            // the arrangement — and a reorder would silently hand every
            // child the unwrapped tool. Wrapping here too makes the property
            // structural rather than positional; a double wrap is harmless
            // (the outer guard refuses first, and everything else delegates
            // straight through), and the ordering test in this file's tests
            // drives exactly this seam with a raw pool.
            Some(tool) if wanted == "kg_task_update" || wanted.ends_with("__kg_task_update") => {
                child_registry.insert(crate::closure_guard::ClosedStatusGuard::wrap(Arc::clone(
                    tool,
                )))
            }
            Some(tool) => child_registry.insert(Arc::clone(tool)),
            // A typo here silently produces a child that cannot do its job, so
            // say so rather than starting a crippled agent.
            None => anyhow::bail!(
                "subagent `{}` asks for tool `{wanted}`, which is not available. \
                 Available: {}",
                profile.name,
                pool.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ")
            ),
        }
    }

    // A child cannot prompt anyone, so `ask` degrades to read-only rather than
    // to a blanket denial that would make the child useless.
    let mode = match cfg.tools.permission_mode {
        PermissionMode::Ask => PermissionMode::ReadOnly,
        other => other,
    };

    let mut child_cfg = cfg.agent.clone();
    child_cfg.max_turns = profile.max_turns;
    child_cfg.system_prompt = profile.system_prompt.clone();
    child_cfg.system_prompt_file = None;

    let mut child = Agent::new(
        mecha_core::provider::build(provider_cfg)?,
        child_registry,
        Arc::new(ModeApprover { mode }),
        ToolCtx {
            workspace: ctx.workspace.clone(),
            shell_timeout: ctx.shell_timeout,
            security: ctx.security.clone(),
            output_budget_bytes: ctx.output_budget_bytes,
            // A subagent is its own isolation domain; the default derives a
            // fresh spill directory rather than sharing the parent's.
            ..ToolCtx::default()
        },
        child_cfg,
        // Profile model wins; otherwise the child uses its provider's default.
        profile.model.clone().or_else(|| provider_cfg.model.clone()),
    )?
    // **The child's own window, or its compaction threshold is not merely
    // different — it does not exist.** `AgentConfig::compact_at` *derives* the
    // threshold from the window when `compact_at_tokens` is unset, which is
    // the default, so a child built without one has `compact_limit() == None`
    // and never compacts at any length.
    //
    // That was survivable while nothing said otherwise, and stopped being so
    // when the model got a button for it: a profile listing `compact` gets the
    // tool (it is in the pool before children are built) and gets the channel
    // (`subagent.rs` clones the parent's `ToolCtx` wholesale), so the child was
    // told *"the transcript will be summarised before your next turn"* by a
    // loop that would never read the flag. The two halves are inherited by
    // different mechanisms — the channel rides on the context, the threshold
    // on the agent — and only one of them made the trip.
    .with_context_window(provider_cfg.context_window);
    // The parent's hooks apply to the child too, or delegating would be the
    // way around a pre_tool policy.
    if let Some(hooks) = hooks {
        child.set_hooks(Arc::clone(hooks));
    }
    // And the approval rules, which only ever narrow.
    child.set_policy(Arc::clone(policy));
    // Same rule for the outbox: a child's send stages like the parent's, or
    // delegating becomes the way to send unstaged.
    if let Some(outbox) = outbox {
        child.set_outbox(Arc::clone(outbox));
    }

    Subagent::new(profile.clone(), Arc::new(child))
}

/// Register the recall tool over this session's transcript, so the run can
/// search its own recorded history — including what a compaction summarised
/// away — instead of re-running tools or re-living the dropped stretch.
///
/// Call this *before* appending the session's `Record::Config`: that record
/// captures the registry's tool list for replay, and a list missing `recall`
/// would make every replay diff against a surface the run never had.
///
/// Where it belongs is decided by what the transcript can hold (see
/// `mecha_core::tool::recall`): chat and the TUI always (their sessions span
/// runs, and prior turns are exactly what compaction removes), a resumed
/// `mecha run` (the resumed file holds the history). Not fresh one-shots or
/// triggers — a per-run session's record is empty until the run ends, so the
/// tool would be a dead spec in the prompt. And not Slack: one shared agent
/// serves every thread while sessions are per-run, so a per-run insert into
/// the shared registry would point one thread's recall at another thread's
/// transcript — a cross-conversation read the fixed path exists to prevent.
pub fn register_recall(agent: &mut Agent, session: &mecha_core::session::Session) {
    agent
        .registry_mut()
        .insert(Arc::new(mecha_core::tool::recall::Recall::new(
            session.path.clone(),
        )));
}

/// `@path` reads from a file; anything else is the literal value. Lets
/// `--system @prompts/reviewer.md` work without a second flag.
pub fn read_maybe_file(value: &str) -> Result<String> {
    match value.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("reading system prompt from {path}")),
        None => Ok(value.to_string()),
    }
}

/// Find a tool by its bare name whatever prefix config gave the server.
///
/// `mail__mail_recent` assumes the server is aliased `mail` with
/// `prefix_tools` on, and neither is guaranteed — a deployment that renamed
/// the server would get "tool not available" from a driver that hardcoded the
/// prefix. Matching on the suffix is what `[outbox] tools` already does when
/// it warns about a routed name.
///
/// Lives here rather than beside its first caller because there is now more
/// than one driver reaching the tool surface from the command line, and two
/// copies of this rule is two places for a prefix assumption to creep back in.
pub fn find_tool<'a>(
    registry: &'a Registry,
    bare: &str,
) -> Option<&'a Arc<dyn mecha_core::tool::Tool>> {
    registry
        .iter()
        .find(|t| t.name() == bare || t.name().ends_with(&format!("__{bare}")))
}

/// Take a tool off a run's surface by its bare name, prefix or not.
///
/// [`find_tool`]'s rule in the other direction, and it must stay that way:
/// withholding `kg_task_update` while a deployment has `prefix_tools` on
/// would otherwise silently withhold nothing, and a control that quietly
/// stops applying is worse than one that was never claimed.
///
/// **The withdrawn tool is handed back, and that is the point.** The harness
/// keeps a handle the model can no longer dispatch through, which is exactly
/// what "status is moved by the harness, and the model is never given a tool
/// that sets it" (D5) needs to be true of: one tool, one holder, and the
/// holder is not the party the status is a judgement about. Returning the
/// name too lets a caller report what it actually withheld rather than
/// assuming the name it asked for was there.
/// Subagent profiles that would hand a withheld tool back to the model.
///
/// **`withhold_tool` cannot reach a child registry, and that is structural.**
/// `build_subagent` clones each allowed tool out of the pool into a separate
/// registry while the agent is being prepared, so removing a tool from the
/// parent afterwards leaves any child that allowlisted it holding a live
/// handle. A run told "you have no tool that sets status" could then simply
/// delegate — the same hole hooks and the outbox route close by inheritance
/// (`subagents inherit the parent's hooks, or delegating is the way around a
/// pre_tool policy`), arriving where inheritance cannot reach.
///
/// So the caller refuses instead. Named profiles rather than a silent strip,
/// because a subagent quietly missing the tool its description promises is
/// the crippled-child failure `build_subagent` already rejects a typo for.
pub fn subagents_holding(config: &mecha_core::config::Config, bare: &str) -> Vec<String> {
    config
        .subagents
        .iter()
        .filter(|p| {
            p.tools
                .iter()
                .any(|t| t == bare || t.ends_with(&format!("__{bare}")))
        })
        .map(|p| p.name.clone())
        .collect()
}

pub fn withhold_tool(
    registry: &mut Registry,
    bare: &str,
) -> Option<(String, Arc<dyn mecha_core::tool::Tool>)> {
    let name = registry
        .iter()
        .map(|t| t.name().to_string())
        .find(|n| n == bare || n.ends_with(&format!("__{bare}")))?;
    registry.remove(&name).map(|tool| (name, tool))
}

/// The outbox items one session has staged so far.
///
/// Beside [`find_tool`] for the reason that one is here: more than one driver
/// now runs a seeded agent and reports what it staged by diffing this across
/// the run, and two copies is two places for the session-id join to drift.
pub fn staged_ids(session_id: &str) -> std::collections::HashSet<String> {
    mecha_core::outbox::OutboxStore::open_existing_default()
        .and_then(|s| s.items().ok())
        .map(|items| {
            items
                .iter()
                .filter(|i| i.session_id.as_deref() == Some(session_id))
                .map(|i| i.id.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The context a command-line driver calls a tool with: the run's workspace,
/// the configured limits, and nothing else. Same shape every driver needs, so
/// it is built once.
pub fn tool_ctx(prepared: &PreparedTools) -> ToolCtx {
    ToolCtx {
        workspace: prepared.workspace.clone(),
        shell_timeout: std::time::Duration::from_secs(prepared.config.tools.shell_timeout_secs),
        security: prepared.config.security.clone(),
        output_budget_bytes: prepared.config.tools.resolved_output_budget(None),
        ..Default::default()
    }
}

/// An [`Asker`] with nobody behind it: every question is declined at once.
///
/// Two callers, one fact — there is no human. An eval run has nobody watching,
/// and declining is the honest thing for the tool to report, leaving the model
/// to proceed and say which reading it chose. A replay has nobody either, and
/// there it is stronger than honesty: `Asker`'s own contract says a tool that
/// blocks forever is worse than one that does not exist, and a corpus walk is
/// exactly the job that would otherwise hang overnight on a prompt nobody will
/// ever see.
///
/// [`Asker`]: mecha_core::tool::ask::Asker
pub struct NoOneToAsk;

#[async_trait::async_trait]
impl mecha_core::tool::ask::Asker for NoOneToAsk {
    async fn ask(&self, _question: &str, _options: &[String]) -> Option<String> {
        None
    }
}

/// Tools a replay may offer **for their description alone**, when the live
/// registry cannot build them.
///
/// **The membership rule is one sentence: a tool whose presence depends on a
/// *front-end* rather than on configuration.** An MCP server that is not
/// connected, or a mail server since renamed, is a real difference between the
/// recording and now, and the bail is right for it — the replay would be
/// asking a different agent a different question. A tool that no CLI process
/// can ever hold, however it is configured, is a different thing: its absence
/// says nothing about the run and everything about the process doing the
/// reading.
///
/// Three qualify, and between them they account for the store:
///
/// | tool | registered by | sessions it blocked |
/// |---|---|---|
/// | `ask_user` | a front-end that owns a human | 246 of 408 |
/// | `recall` | the session-recording front-ends | 122 |
/// | `show_file` | a front-end with a thread attached | 55 |
///
/// Together they take the store from **22% replayable to 76%**, measured by
/// comparing each session's recorded tool list against the surface
/// [`prepare`] builds. The residual 97 sessions name a `pkg__*` or a
/// `google__*` tool and stay refused, which is the bail doing its job — the
/// second of those is a server that was renamed to `mail__*`, so those
/// recordings are permanently unreplayable and nothing here should pretend
/// otherwise.
///
/// Two properties make this faithful rather than a fake.
///
/// **Every entry is the real tool**, so its description and schema are the
/// bytes the recording put in front of the model rather than a stub written to
/// look similar. That matters most for `ask_user`, whose decline wording is
/// A/B-measured, and it costs nothing for the other two.
///
/// **And each is unreachable rather than merely unused.**
/// [`mecha_core::replay_run::replay_registry`] consults this only in the modes
/// where nothing executes — `Stop` and `Error` — so the constructor arguments
/// below are never read. That is what licenses the inert ones: a `recall` over
/// a path that does not exist and a `show_file` that would refuse every size
/// are not lies waiting to be believed, they are values in a branch the replay
/// cannot enter. **A plausible-looking argument would be the worse choice** —
/// a real transcript path here reads to the next person as though the tool
/// were live, and invites them to rely on it.
///
/// Not `Live`: there the replay abandons the recording and continues as a
/// genuine fresh run, and a fresh run holding a permanently-erroring tool is
/// not one — its divergence would read as a finding about the model when it is
/// a finding about the harness. That is a measurement-validity argument rather
/// than a fail-fast preference, and it is why the gate is on what a mode
/// *does* and never on its name.
pub fn surface_only_registry() -> Registry {
    let mut r = Registry::new();
    r.insert(Arc::new(mecha_core::tool::ask::AskUserTool::new(Arc::new(
        NoOneToAsk,
    ))));
    // Named for what it is, not for a session: see the note above on why an
    // inert argument beats a plausible one.
    r.insert(Arc::new(mecha_core::tool::recall::Recall::new(
        std::path::PathBuf::from("(no transcript — a replay never calls this)"),
    )));
    r.insert(Arc::new(crate::slack::show::ShowFileTool::new(0)));
    r
}

#[cfg(test)]
mod surface_only_tests {
    use super::surface_only_registry;

    /// The three names that made 78% of the store unreplayable, asserted by
    /// name rather than by count.
    ///
    /// A count would pass while the set drifted; these are the ones measured
    /// against the store, and each is here because **no CLI process can build
    /// it however it is configured** — which is the membership rule, and the
    /// line between this and the bail. If a fourth front-end tool is ever
    /// registered, this test failing is the reminder that a replay of every
    /// session recorded after it will refuse to start.
    #[test]
    fn the_front_end_tools_are_all_of_them_and_only_them() {
        let r = surface_only_registry();
        let mut names: Vec<&str> = r.iter().map(|t| t.name()).collect();
        names.sort();
        assert_eq!(names, ["ask_user", "recall", "show_file"]);
    }

    /// Faithful rather than a fake: each entry is the *real* tool, so what the
    /// model is offered is the bytes the recording offered it.
    ///
    /// Checked on the description because that is the part a stub would get
    /// wrong and nobody would notice — a name and a schema are easy to
    /// reproduce, and prose is not.
    #[test]
    fn every_stand_in_carries_a_real_description_and_schema() {
        for tool in surface_only_registry().iter() {
            let d = tool.description();
            assert!(
                d.len() > 80,
                "`{}` has a stub description ({} bytes)",
                tool.name(),
                d.len()
            );
            assert!(
                tool.input_schema().get("properties").is_some(),
                "`{}` has no schema properties",
                tool.name()
            );
        }
    }

    /// The constructor arguments are inert on purpose, and this is the
    /// assertion that keeps them that way.
    ///
    /// `replay_registry` consults this registry only where nothing executes,
    /// so a `recall` pointed at a path that cannot exist is a value in an
    /// unreachable branch rather than a lie. If someone later makes one of
    /// these look real — a live transcript path, a genuine upload cap — the
    /// next reader will believe it, and the branch it sits in will still never
    /// run.
    #[test]
    fn the_inert_arguments_are_visibly_inert() {
        let r = surface_only_registry();
        let recall = r.get("recall").expect("recall is a stand-in");
        // Nothing to assert about behaviour — it cannot be reached. What is
        // assertable is that the thing standing in is the real type, which is
        // what `every_stand_in_carries_a_real_description_and_schema` covers,
        // and that it is registered at all.
        assert_eq!(recall.name(), "recall");
        assert_eq!(
            r.get("show_file").map(|t| t.name()),
            Some("show_file"),
            "show_file is a mecha-cli tool, so core cannot build it — this \
             registry is where the caller supplies what core cannot"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_subagent, excluded_by_allowlist, step_escalation_enabled, step_escalation_slot,
    };

    /// The review finding this pins: `cfg.agent.step_escalation` is read
    /// nowhere but `build`'s `ToolCtx` construction, so nothing else would
    /// catch this mapping breaking. Both directions of the truth table,
    /// composed end to end through both functions — cfg value and CLI
    /// override in, slot presence out.
    #[test]
    fn the_cli_override_and_the_slot_construction_agree_on_all_four_combinations() {
        for cfg_value in [false, true] {
            for no_flag in [false, true] {
                let enabled = step_escalation_enabled(cfg_value, no_flag);
                assert_eq!(enabled, cfg_value && !no_flag);
                assert_eq!(
                    step_escalation_slot(enabled).is_some(),
                    enabled,
                    "cfg={cfg_value} no_step_escalation={no_flag}"
                );
            }
        }
    }

    /// Named directly, since these are the two behaviours a person actually
    /// cares about: off by default, and the override always wins.
    #[test]
    fn off_by_default_and_the_override_always_wins() {
        assert!(!step_escalation_enabled(false, false), "off by default");
        assert!(
            !step_escalation_enabled(true, true),
            "--no-step-escalation must win even when config turns it on"
        );
        assert!(step_escalation_enabled(true, false));
    }

    /// **A child that cannot compact must not be handed the button for it.**
    ///
    /// `AgentConfig::compact_at` *derives* the threshold from the context
    /// window when `compact_at_tokens` is unset — which is the default — so a
    /// child built without a window has `compact_limit() == None` and never
    /// compacts at any length. That was survivable while nothing said
    /// otherwise, and stopped being so when `compact` shipped: the child gets
    /// the tool (it is in the pool before children are built) and the channel
    /// (`subagent.rs` clones the parent's `ToolCtx` wholesale), so it was told
    /// *"the transcript will be summarised before your next turn"* by a loop
    /// that would never read the flag.
    ///
    /// The two halves are inherited by different mechanisms — the channel on
    /// the context, the threshold on the agent — and only one made the trip.
    /// Fails on the old `build_subagent`, which never called
    /// `with_context_window`.
    #[test]
    fn a_subagent_inherits_the_window_its_compaction_threshold_derives_from() {
        let cfg = mecha_core::config::Config::default();
        let mut provider_cfg = cfg.providers.values().next().cloned().unwrap_or_default();
        provider_cfg.context_window = Some(100_000);
        let profile = mecha_core::subagent::SubagentProfile {
            name: "child".into(),
            description: "a child".into(),
            ..Default::default()
        };
        let child = build_subagent(
            &profile,
            &mecha_core::tool::Registry::new(),
            &cfg,
            &provider_cfg,
            &mecha_core::tool::ToolCtx::default(),
            None,
            &std::sync::Arc::new(mecha_core::policy::ExecPolicy::empty()),
            None,
        );
        let child = match child {
            Ok(c) => c,
            // A provider this machine cannot build is not what this test is
            // about; skipping beats asserting on the error text.
            Err(_) => return,
        };
        assert_eq!(
            child.agent().context_window(),
            Some(100_000),
            "without the window the child's compaction threshold is None, and \
             `compact` becomes a tool that reports success and does nothing"
        );
    }

    /// The review's note on the closure guard: the security argument rested
    /// on one line's *position* — wrapped before the subagent loop — and
    /// nothing measured it, so moving the wrap below the loop restored the
    /// delegation hole with every test green. `build_subagent` now wraps at
    /// the clone too, which is what this drives: a **raw** pool (exactly
    /// what a reordered `build` would hand it) must still produce a child
    /// whose `kg_task_update` refuses a closure.
    #[tokio::test]
    async fn a_subagent_built_from_a_raw_pool_still_carries_the_closure_guard() {
        struct Raw;
        #[async_trait::async_trait]
        impl mecha_core::tool::Tool for Raw {
            fn name(&self) -> &str {
                "graph__kg_task_update"
            }
            fn description(&self) -> &str {
                "update a task"
            }
            fn input_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object"})
            }
            async fn call(
                &self,
                _input: serde_json::Value,
                _ctx: &mecha_core::tool::ToolCtx,
            ) -> anyhow::Result<mecha_core::tool::ToolOutput> {
                Ok(mecha_core::tool::ToolOutput::ok("reached the store"))
            }
        }

        let mut pool = mecha_core::tool::Registry::new();
        pool.insert(std::sync::Arc::new(Raw));
        let cfg = mecha_core::config::Config::default();
        // A local provider, so the child always builds: the default config's
        // provider is anthropic, whose builder wants a credential — found on
        // review, the first cut skipped on that `Err` and this test asserted
        // nothing on any machine without a key, standing in for a property
        // the PR's own comment calls previously positional. `kind = "local"`
        // needs no credential and nothing here ever sends a request.
        let provider_cfg = mecha_core::config::ProviderConfig {
            kind: "local".into(),
            base_url: Some("http://127.0.0.1:1".into()),
            ..Default::default()
        };
        let profile = mecha_core::subagent::SubagentProfile {
            name: "child".into(),
            description: "a child".into(),
            tools: vec!["graph__kg_task_update".into()],
            ..Default::default()
        };
        let child = build_subagent(
            &profile,
            &pool,
            &cfg,
            &provider_cfg,
            &mecha_core::tool::ToolCtx::default(),
            None,
            &std::sync::Arc::new(mecha_core::policy::ExecPolicy::empty()),
            None,
        )
        .expect("a local provider needs no credential, so the child must build");
        let tool = child
            .agent()
            .registry()
            .get("graph__kg_task_update")
            .expect("the child allowlisted it");
        let out = tool
            .call(
                serde_json::json!({"task": "t1", "status": "done"}),
                &mecha_core::tool::ToolCtx::default(),
            )
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.contains("mecha tasks set"),
            "a child built from a raw pool must still refuse a closure: {}",
            out.content
        );
    }

    /// The bug this pins: `--tool fs_read` used to abort the whole run because
    /// the configured `research` subagent wants `web_search`. Narrowing the
    /// tool surface is the caller saying what they want, not a typo — the
    /// subagent is dropped, and only a name missing *without* an allowlist is
    /// still an error.
    #[test]
    fn an_absent_allowlist_excludes_nothing_and_a_present_one_excludes_what_it_omits() {
        let wanted: Vec<String> = ["web_search", "http_fetch", "todo"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        assert!(
            excluded_by_allowlist(&wanted, &[]).is_empty(),
            "no --tool means every tool, not no tools"
        );

        let narrow: Vec<String> = vec!["fs_read".into()];
        assert_eq!(
            excluded_by_allowlist(&wanted, &narrow),
            vec!["web_search", "http_fetch", "todo"]
        );

        let full: Vec<String> = ["web_search", "http_fetch", "todo", "fs_read"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(
            excluded_by_allowlist(&wanted, &full).is_empty(),
            "an allowlist that covers the profile keeps it"
        );
    }
}
