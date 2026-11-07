//! Owner CLI over the core store. Provider commands cross an argv-only child boundary.
use super::actor::acting_actor_explained;
use anyhow::{bail, ensure, Context, Result};
use mecha_core::mail_rules::{self as rules, ActionGroup, Mode, RuleStore};
use std::{
    io::{IsTerminal, Read, Write},
    path::PathBuf,
};

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Install owner-named files or stdin content in shadow, never auto.
    Add {
        #[arg(required_unless_present = "stdin", conflicts_with = "stdin")]
        path: Option<PathBuf>,
        /// Read rule.toml from stdin; requires `--name=NAME`.
        #[arg(long, requires = "name")]
        stdin: bool,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Replace rule.toml; changed approvals lapse. Bundled files are preserved.
    Edit {
        rule: String,
        #[arg(long, required_unless_present = "stdin", conflicts_with = "stdin")]
        file: Option<PathBuf>,
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        json: bool,
    },
    /// Read recorded/effective modes and holds.
    List {
        #[arg(long)]
        json: bool,
    },
    Show {
        rule: String,
        #[arg(long)]
        json: bool,
    },
    /// Set one action group's mode; auto binds the files shown here.
    Mode {
        rule: String,
        #[arg(value_parser = parse_group)]
        group: ActionGroup,
        #[arg(value_parser = parse_mode)]
        mode: Mode,
        #[arg(long)]
        yes: bool,
        /// Approval digest JSON from show; refuses if the inputs changed.
        #[arg(long)]
        digest: Option<String>,
        #[arg(long)]
        json: bool,
        /// Require a supplied digest on raises (used by the web review).
        #[arg(long, hide = true)]
        require_digest: bool,
    },
    /// Match one header rule against 30 days of mail, including archived mail. Never writes.
    Test {
        rule: String,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Newest ledger rows, with pointers and reasons; never message bodies.
    Ledger {
        rule: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Restore the exact message of a completed mailbox action.
    Undo { row_id: String },
    /// Mark a header matcher wrong; edit the rule to fix it. Does not undo the action.
    WrongMatch {
        row_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Lower this rule to shadow, preserving off groups. Resume through mode and its approval screen; there is no unpause.
    Pause {
        rule: String,
        #[arg(long)]
        json: bool,
    },
    /// Lower every group to shadow, preserving groups already off. Resume through mode.
    PauseAll {
        #[arg(long)]
        json: bool,
    },
}
fn parse_mode(value: &str) -> Result<Mode> {
    let mode = serde_json::from_value(serde_json::json!(value))?;
    ensure!(
        mode != Mode::Unknown,
        "expected off, shadow, propose or auto"
    );
    Ok(mode)
}
fn parse_group(value: &str) -> Result<ActionGroup> {
    let group = serde_json::from_value(serde_json::json!(value.replace('-', "_")))?;
    ensure!(group != ActionGroup::Unknown, "unknown action group; use mailbox, private_record, report or board (send groups are not supported in P1)");
    Ok(group)
}
fn spelling(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}
fn root() -> Result<PathBuf> {
    Ok(RuleStore::root_under(&mecha_core::work::mecha_home()?))
}
fn existing() -> Result<RuleStore> {
    RuleStore::open_existing(root()?)
        .context("opening mail rules; install a rule with `mecha mail rule add PATH`")
}
fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
fn input(file: Option<&std::path::Path>, stdin: bool) -> Result<String> {
    if stdin || file.is_some_and(|p| p == std::path::Path::new("-")) {
        let mut text = String::new();
        std::io::stdin()
            .take(1024 * 1024 + 1)
            .read_to_string(&mut text)?;
        ensure!(text.len() <= 1024 * 1024, "rule text exceeds 1 MiB");
        Ok(text)
    } else {
        std::fs::read_to_string(file.context("--file or --stdin required")?)
            .context("reading rule file")
    }
}

pub async fn run(cmd: Cmd) -> Result<()> {
    let json = matches!(
        &cmd,
        Cmd::Add { json: true, .. }
            | Cmd::Edit { json: true, .. }
            | Cmd::Mode { json: true, .. }
            | Cmd::Pause { json: true, .. }
            | Cmd::PauseAll { json: true }
            | Cmd::WrongMatch { json: true, .. }
    );
    let result = run_inner(cmd).await;
    if json {
        if let Err(error) = &result {
            let code = error
                .downcast_ref::<rules::ModeError>()
                .map(|e| e.code())
                .unwrap_or("invalid_request");
            print_json(
                &serde_json::json!({"ok": false, "error": {"code": code, "message": format!("{error:#}")}}),
            )?;
        }
    }
    result
}
async fn run_inner(cmd: Cmd) -> Result<()> {
    if matches!(
        &cmd,
        Cmd::Add { .. }
            | Cmd::Edit { .. }
            | Cmd::Test { .. }
            | Cmd::Undo { .. }
            | Cmd::Mode {
                mode: Mode::Propose | Mode::Auto,
                ..
            }
    ) {
        super::features::require(mecha_core::feature::Feature::MailRules)?;
    }
    match cmd {
        Cmd::Add {
            path,
            stdin,
            name,
            json,
        } => {
            // Parse content before opening/creating the store.
            let text = if stdin {
                Some(input(None, true)?)
            } else {
                None
            };
            if let Some(text) = &text {
                rules::RuleDefinition::parse(text)?;
            }
            let store = RuleStore::open_default()?;
            let rule = if let Some(text) = text {
                let name = name
                    .as_deref()
                    .filter(|n| !n.is_empty())
                    .context("stdin requires --name=NAME")?;
                store.install(name, &text, &Default::default())?
            } else {
                rules::install_path(
                    &store,
                    path.as_deref().context("rule source required")?,
                    name.as_deref(),
                )?
            };
            if json {
                print_json(
                    &serde_json::json!({"ok": true, "name": rule.name, "mode": "shadow", "approval_digest": rule.approval_digest}),
                )?;
            } else {
                println!(
                    "Installed {} in shadow: {}",
                    rule.name,
                    rule.directory.display()
                );
                println!(
                    "Approval digest: {}",
                    serde_json::to_string(&rule.approval_digest)?
                );
                for action in &rule.definition.actions {
                    println!(
                        "  {} / {}",
                        spelling(&action.group()),
                        spelling(&action.verb)
                    );
                }
            }
        }
        Cmd::Edit {
            rule,
            file,
            stdin,
            json,
        } => {
            let text = input(file.as_deref(), stdin)?;
            rules::RuleDefinition::parse(&text)?;
            let report = existing()?.edit(&rule, &text)?;
            if json {
                print_json(&report)?;
            } else {
                println!(
                    "{}: changed {}",
                    report.name,
                    if report.changed_files.is_empty() {
                        "nothing".into()
                    } else {
                        report.changed_files.join(", ")
                    }
                );
                println!(
                    "Approval digest: {}",
                    serde_json::to_string(&report.approval_digest)?
                );
                for notice in report.notices {
                    println!("{notice}: {:?}", report.lapsed_groups);
                }
            }
        }
        Cmd::List { json } => {
            let views = if root()?.exists() {
                views(&existing()?)?
            } else {
                Vec::new()
            };
            if json {
                print_json(&views)?;
            } else if views.is_empty() {
                println!("no mail rules installed");
            } else {
                for view in &views {
                    print_view(view);
                }
            }
        }
        Cmd::Show { rule, json } => {
            let store = existing()?;
            let view = views(&store)?
                .into_iter()
                .find(|v| v.name == rule)
                .context("unknown rule")?;
            if json {
                let mut value = serde_json::to_value(&view)?;
                value["inputs"] = if view.load_error.is_none() {
                    serde_json::to_value(store.review_inputs(&rule)?)?
                } else {
                    serde_json::Value::Null
                };
                value["rule_toml"] = serde_json::to_value(store.rule_text(&rule).ok())?;
                print_json(&value)?;
            } else {
                print_view(&view);
                if view.load_error.is_none() {
                    for (file, text) in store.review_inputs(&rule)?.files {
                        println!("\n--- {file} ---\n{text}");
                    }
                }
            }
        }
        Cmd::Mode {
            rule,
            group,
            mode,
            yes,
            digest,
            json,
            require_digest,
        } => {
            let supplied = digest
                .as_deref()
                .map(serde_json::from_str::<rules::ApprovalDigest>)
                .transpose()
                .context("invalid approval digest JSON")?;
            change_mode(
                &rule,
                group,
                mode,
                yes,
                supplied.as_ref(),
                json,
                require_digest,
            )?;
        }
        Cmd::Ledger { rule, limit, json } => {
            if json {
                let rows = if root()?.exists() {
                    rules::ledger_views(&existing()?, rule.as_deref(), limit)?
                } else {
                    Vec::new()
                };
                print_json(&rows)?;
            } else {
                let rows = if root()?.exists() {
                    rules::recent_ledger(&existing()?, rule.as_deref(), limit)?
                } else {
                    Vec::new()
                };
                if rows.is_empty() {
                    println!("no mail rule ledger entries");
                }
                for row in rows {
                    println!(
                        "{} {} {} {} {}/{}/{} — {}",
                        row.id,
                        row.at
                            .map(|t| t.to_rfc3339())
                            .unwrap_or_else(|| "unknown time".into()),
                        spelling(&row.outcome),
                        spelling(&row.action.verb),
                        row.message.account,
                        row.message.thread_id,
                        row.message.provider_message_id,
                        row.reason
                    );
                }
            }
        }
        Cmd::Pause { rule, json } => {
            let (actor, why) = acting_actor_explained();
            let report = rules::pause_rule(&existing()?, &rule, actor)?;
            pause_output(report, actor, why, json)?;
        }
        Cmd::PauseAll { json } => {
            let (actor, why) = acting_actor_explained();
            let report = if root()?.exists() {
                rules::pause_all(&existing()?, actor)?
            } else {
                rules::PauseReport::default()
            };
            pause_output(report, actor, why, json)?;
        }
        Cmd::WrongMatch { row_id, json } => {
            let (actor, why) = acting_actor_explained();
            let report = existing()?.wrong_match(&row_id, actor)?;
            if json {
                print_json(&report)?;
            } else {
                println!("{}", report.message);
                if let Some(why) = why {
                    println!("Actor reading: {why}");
                }
            }
        }
        Cmd::Test {
            rule,
            account,
            json,
        } => {
            let mut args = vec!["rules".into(), "test".into()];
            if json {
                args.push("--json".into());
            }
            if let Some(account) = account {
                // `=` binds a value that begins with a hyphen.
                args.push(format!("--account={account}"));
            }
            args.extend(["--".into(), rule]);
            launch(&args).await?;
        }
        Cmd::Undo { row_id } => {
            launch(&["rules".into(), "undo".into(), "--".into(), row_id]).await?
        }
    }
    Ok(())
}
fn pause_output(
    report: rules::PauseReport,
    actor: mecha_core::closure::Actor,
    why: Option<String>,
    json: bool,
) -> Result<()> {
    if json {
        print_json(
            &serde_json::json!({"ok": true, "groups": report.groups, "notices": report.notices, "actor": actor, "actor_reason": why}),
        )?;
    } else {
        for notice in report.notices {
            eprintln!("{notice}");
        }
        println!(
            "Paused {} groups to shadow (actor: {})",
            report.groups,
            actor.as_str()
        );
        if let Some(why) = why {
            println!("Actor reading: {why}");
        }
    }
    Ok(())
}

fn change_mode(
    rule: &str,
    group: ActionGroup,
    mode: Mode,
    yes: bool,
    supplied: Option<&rules::ApprovalDigest>,
    json: bool,
    require_digest: bool,
) -> Result<()> {
    let store = existing()?;
    let (actor, why) = acting_actor_explained();
    let shown = if mode == Mode::Auto {
        let review = store.review_inputs(rule)?;
        if let Some(expected) = supplied {
            ensure!(expected == &review.digest, rules::ModeError::StaleDigest);
        }
        let loaded = store.get(rule)?;
        let current = rules::resolve_mode(
            &loaded,
            group,
            &store.approvals()?,
            &rules::owner_file_holds(),
        );
        rules::gate_mode(current.effective, mode, Some(actor)).with_context(|| {
            why.clone()
                .unwrap_or_else(|| format!("actor: {}", actor.as_str()))
        })?;
        ensure!(
            mode.rank() <= store.mode_ceiling(rule, group)?.rank(),
            rules::ModeError::Ceiling
        );
        let mut screen = format!(
            "Approve {rule} / {} in auto\nApproval digest: {}\n",
            spelling(&group),
            serde_json::to_string_pretty(&review.digest)?
        );
        for (file, text) in &review.files {
            screen.push_str(&format!("\n--- {file} ---\n{text}\n"));
        }
        screen.push_str("Guards: exact message, activation cutoff, approval lapse, provider holds and automatic brakes.\n");
        if group == ActionGroup::Board {
            screen.push_str("Tasks reach a task run's prompt and consume the rule/global daily caps. Propose is recommended.\n");
        }
        for hold in current.holds {
            screen.push_str(&format!("Held: {}\n", hold.reason));
        }
        screen.push_str(
            "Shadow rows under this group's current match digest (verdict_input kept separate):\n",
        );
        let digest = loaded.match_digest(group, rules::GateStatus::Passing)?;
        for row in rules::recent_ledger(&store, Some(rule), usize::MAX)?
            .into_iter()
            .filter(|r| {
                r.group == group
                    && r.match_digest == digest
                    && r.outcome == rules::Outcome::WouldHave
            })
        {
            screen.push_str(&format!("{}\n", serde_json::to_string(&row)?));
        }
        if json {
            eprint!("{screen}");
        } else {
            print!("{screen}");
        }
        if !yes {
            ensure!(
                !json && std::io::stdin().is_terminal(),
                rules::ModeError::ConfirmationRequired
            );
            print!("Approve these inputs? Type yes: ");
            std::io::stdout().flush()?;
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;
            ensure!(
                answer.trim().eq_ignore_ascii_case("yes"),
                "approval cancelled"
            );
        }
        review.digest
    } else {
        store
            .get(rule)
            .map(|r| r.approval_digest)
            .unwrap_or_default()
    };
    let decision = store
        .change_mode_checked(
            rule,
            group,
            mode,
            Some(actor),
            supplied.unwrap_or(&shown),
            &rules::owner_file_holds(),
            supplied,
            require_digest,
        )
        .with_context(|| {
            why.clone()
                .unwrap_or_else(|| format!("actor: {}", actor.as_str()))
        })?;
    ensure!(!decision.refused, rules::ModeError::Ceiling);
    if json {
        let view = views(&store)?
            .into_iter()
            .find(|v| v.name == rule)
            .context("unknown rule")?;
        let group_view = view
            .groups
            .into_iter()
            .find(|g| g.group == group)
            .context("unknown group")?;
        print_json(
            &serde_json::json!({"ok": true, "name": rule, "group": group, "requested_mode": mode, "recorded_mode": group_view.recorded_mode, "effective_mode": group_view.effective_mode, "clamped": decision.clamped, "holds": group_view.holds, "actor": actor, "actor_reason": why}),
        )?;
    } else {
        if decision.clamped {
            println!(
                "Requested {} clamped to {}: {}",
                spelling(&mode),
                spelling(&decision.target),
                why.as_deref().unwrap_or("store safety gate")
            );
        }
        for hold in &decision.holds {
            println!("Held: {}", hold.reason);
        }
        println!(
            "{} / {}: {} (actor: {})",
            rule,
            spelling(&group),
            spelling(&decision.target),
            actor.as_str()
        );
    }
    Ok(())
}

fn print_view(view: &rules::RuleView) {
    println!(
        "{} — {}",
        view.name,
        view.description
            .as_deref()
            .unwrap_or("unreadable definition")
    );
    for group in &view.groups {
        println!(
            "  {}: recorded={} effective={}",
            spelling(&group.group),
            spelling(&group.recorded_mode),
            spelling(&group.effective_mode)
        );
        for hold in &group.holds {
            println!("    held: {hold}");
        }
    }
    for hold in &view.holds {
        println!("  {hold}");
    }
    if let Some(error) = &view.load_error {
        println!("  {error}");
    }
    println!(
        "  last activity: {}",
        view.last_activity
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "none".into())
    );
}

async fn launch(args: &[String]) -> Result<()> {
    use std::os::unix::process::ExitStatusExt;
    let status = child(&std::env::current_exe()?, args)?
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .await
        .context("launching mecha-mail")?;
    if !status.success() {
        crate::exit_with(
            status
                .code()
                .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)),
        );
    }
    Ok(())
}

fn views(store: &RuleStore) -> Result<Vec<rules::RuleView>> {
    let mut views = rules::rule_views(store)?;
    // Registry metadata only. OAuth credentials and provider clients remain in
    // mecha-mail; the account file uses its established MECHA_MAIL_DIR override.
    let directory = mecha_core::onboarding::mail_store_dir();
    let registry = directory
        .map(|dir| std::fs::read_to_string(dir.join("accounts.toml")))
        .transpose()
        .ok()
        .flatten()
        .and_then(|text| toml::from_str::<toml::Value>(&text).ok());
    if let Some(entries) = registry
        .as_ref()
        .and_then(|v| v.get("account"))
        .and_then(|v| v.as_array())
    {
        let accounts: Vec<_> = entries
            .iter()
            .filter_map(|entry| {
                Some(rules::RuleAccount {
                    name: entry.get("name")?.as_str()?.into(),
                    proves_header_absence: entry.get("provider").and_then(|v| v.as_str())
                        == Some("google"),
                })
            })
            .collect();
        rules::account_holds(store, &mut views, &accounts)?;
    } else {
        for view in &mut views {
            view.holds
                .push("provider conditions unknown: account registry unavailable".into());
        }
    }
    Ok(views)
}

fn child(executable: &std::path::Path, args: &[String]) -> Result<tokio::process::Command> {
    let sibling = rules::child_path(executable)?;
    if !rules::is_executable_file(&sibling) {
        bail!("mecha-mail binary is missing or not executable; install it with `cargo install --path mecha-mail`");
    }
    let mut command = tokio::process::Command::new(sibling);
    command.args(args).kill_on_drop(true);
    Ok(command)
}

#[derive(Debug, Default)]
pub(super) struct HookReport {
    pub attempted: usize,
    pub failed: usize,
    pub diagnostic: String,
}

/// Called only after classify has made its verdicts durable. Its result never
/// decides classify's exit status, including a missing sibling or unreadable config.
pub(super) async fn after_classify(account: Option<&str>) {
    let result = match (
        mecha_core::config::Config::load_global(),
        std::env::current_exe(),
    ) {
        (Ok(config), Ok(executable)) => hook(&config, &executable, account).await,
        (Err(error), _) => HookReport {
            failed: 1,
            diagnostic: format!("{error:#}"),
            ..Default::default()
        },
        (_, Err(error)) => HookReport {
            failed: 1,
            diagnostic: error.to_string(),
            ..Default::default()
        },
    };
    if !result.diagnostic.is_empty() {
        eprintln!("{}", result.diagnostic.trim_end());
    }
    if result.attempted > 0 || result.failed > 0 {
        eprintln!(
            "mail rules after classify: {} attempted, {} failed",
            result.attempted, result.failed
        );
    }
}

async fn hook(
    config: &mecha_core::config::Config,
    executable: &std::path::Path,
    account: Option<&str>,
) -> HookReport {
    hook_with_budget(
        config,
        executable,
        account,
        HookBudget {
            total: rules::CLASSIFY_HOOK_TIMEOUT,
            grace: rules::CLASSIFY_HOOK_GRACE,
            reap: rules::CLASSIFY_HOOK_CLEANUP,
        },
    )
    .await
}

#[derive(Clone, Copy)]
struct HookBudget {
    total: std::time::Duration,
    grace: std::time::Duration,
    reap: std::time::Duration,
}

async fn hook_with_budget(
    config: &mecha_core::config::Config,
    executable: &std::path::Path,
    account: Option<&str>,
    budget: HookBudget,
) -> HookReport {
    if let Some(reason) = rules::disabled_reason(config) {
        return HookReport {
            diagnostic: format!("{reason}; skipping after-classify hook"),
            ..Default::default()
        };
    }
    let mut args = vec!["rules".into(), "apply".into(), "--after-classify".into()];
    if let Some(account) = account {
        args.extend(["--account".into(), account.into()]);
    }
    let result = async {
        use tokio::io::AsyncReadExt;
        let mut command = child(executable, &args)?;
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .process_group(0);
        let mut process = command.spawn().context("launching mail rules hook")?;
        let group = process.id().context("hook process id unavailable")? as i32;
        let mut stdout = process.stdout.take().context("hook stdout unavailable")?;
        let mut stderr = process.stderr.take().context("hook stderr unavailable")?;
        // The total allowance includes termination. Never spend the service's
        // cleanup reserve on provider work or on descendants holding pipes open.
        let running = budget.total.saturating_sub(budget.grace + budget.reap);
        let needs_kill = {
            let output = async {
                let mut out = Vec::new();
                let mut err = Vec::new();
                let (status, _, _) = tokio::try_join!(
                    process.wait(),
                    stdout.read_to_end(&mut out),
                    stderr.read_to_end(&mut err)
                )?;
                Ok::<_, std::io::Error>(std::process::Output {
                    status,
                    stdout: out,
                    stderr: err,
                })
            };
            tokio::pin!(output);
            if let Ok(result) = tokio::time::timeout(running, &mut output).await {
                return result.context("waiting for mail rules hook");
            }
            // SAFETY: the child starts its own process group; only this hook and
            // its descendants receive the signal, never the classifier.
            unsafe {
                libc::kill(-group, libc::SIGTERM);
            }
            // Keep draining output during grace so a cooperative child can
            // finish cleanup without blocking on a full pipe.
            tokio::time::timeout(budget.grace, &mut output)
                .await
                .is_err()
        };
        if needs_kill {
            // SAFETY: this is the same owned child process group as above.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
            tokio::time::timeout(budget.reap, process.wait())
                .await
                .context("mail rules hook timed out; reap also timed out")?
                .context("mail rules hook timed out; reaping failed")?;
        }
        // Even exit(0) in response to TERM is an incomplete, timed-out pass.
        bail!(
            "mail rules hook timed out within its {} second total allowance",
            budget.total.as_secs()
        );
    }
    .await;
    match result {
        Ok(output) => HookReport {
            attempted: 1,
            failed: usize::from(!output.status.success()),
            diagnostic: format!(
                "{}{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
                if output.status.success() {
                    String::new()
                } else {
                    format!("\nmail rules child: {}", output.status)
                }
            ),
        },
        Err(error) => HookReport {
            attempted: 1,
            failed: 1,
            diagnostic: format!("mail rules hook failed: {error:#}"),
        },
    }
}

#[cfg(test)]
mod classify_hook_tests {
    use super::*;
    use mecha_core::feature::{self, Feature};
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn mail_rules_hung_hook_exits_within_the_reserved_service_budget_and_is_killed() {
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!(
            "rule-hook-timeout-{}",
            mecha_core::session::Session::new_id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let fixture = dir.join("mecha-mail");
        let mut config = mecha_core::config::Config::default();
        feature::set_switch(&mut config, Feature::Mail, true);
        feature::set_switch(&mut config, Feature::MailRules, true);
        let budget = HookBudget {
            total: Duration::from_secs(3),
            grace: Duration::from_millis(500),
            reap: Duration::from_millis(200),
        };
        // Both a cooperative child and one that ignores TERM must be reaped.
        for cooperative in [true, false] {
            std::fs::write(
                &fixture,
                format!(
                    r#"#!/usr/bin/env python3
import os, pathlib, signal, time
root = pathlib.Path(__file__).parent
(root / 'pid').write_text(str(os.getpid()))
def terminate(signum, frame):
    (root / 'term').write_text('received')
    if {cooperative}:
        time.sleep(0.2)
        (root / 'cleaned').write_text('finished')
        raise SystemExit(0)
signal.signal(signal.SIGTERM, terminate)
while True:
    time.sleep(1)
"#,
                    cooperative = if cooperative { "True" } else { "False" }
                ),
            )
            .unwrap();
            std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o700)).unwrap();
            let started = Instant::now();
            let report = hook_with_budget(&config, &dir.join("mecha"), None, budget).await;
            assert!(
                started.elapsed() < budget.total + Duration::from_millis(500),
                "hook consumed the reserved service budget"
            );
            assert_eq!(report.attempted, 1);
            assert_eq!(report.failed, 1);
            assert!(
                report.diagnostic.contains("timed out"),
                "{}",
                report.diagnostic
            );
            assert!(
                dir.join("term").exists(),
                "deadline must send SIGTERM before SIGKILL"
            );
            assert_eq!(dir.join("cleaned").exists(), cooperative);
            if cooperative {
                std::fs::remove_file(dir.join("cleaned")).unwrap();
            } else {
                assert!(started.elapsed() >= budget.total - budget.reap);
            }
            let pid: i32 = std::fs::read_to_string(dir.join("pid"))
                .unwrap()
                .parse()
                .unwrap();
            // SAFETY: signal zero only checks the fixture process's existence.
            assert_ne!(
                unsafe { libc::kill(pid, 0) },
                0,
                "timed-out hook was not reaped"
            );
            std::fs::remove_file(dir.join("term")).unwrap();
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn mail_rules_classify_hook_uses_the_sibling_and_counts_failure_without_exiting() {
        let dir = std::env::temp_dir().join(format!(
            "rule-hook-{}",
            mecha_core::session::Session::new_id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let fixture = dir.join("mecha-mail");
        std::fs::write(
            &fixture,
            "#!/bin/sh\nprintf '%s\\n' \"$@\"\nprintf 'fixture error' >&2\nexit 19\n",
        )
        .unwrap();
        std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = mecha_core::config::Config::default();
        let skipped = hook(&config, &dir.join("mecha"), None).await;
        assert_eq!(skipped.attempted, 0);
        assert_eq!(skipped.failed, 0);
        assert_eq!(skipped.diagnostic.lines().count(), 1);
        feature::set_switch(&mut config, Feature::Mail, true);
        feature::set_switch(&mut config, Feature::MailRules, true);
        let report = hook(
            &config,
            &dir.join("mecha"),
            Some("fictional account; echo unsafe"),
        )
        .await;
        assert_eq!(report.attempted, 1);
        assert_eq!(report.failed, 1);
        assert!(report.diagnostic.starts_with(
            "rules\napply\n--after-classify\n--account\nfictional account; echo unsafe\n"
        ));
        assert!(report.diagnostic.contains("fixture error"));
        std::fs::write(&fixture, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        let all = hook(&config, &dir.join("mecha"), None).await;
        assert_eq!(all.failed, 0);
        assert_eq!(all.diagnostic, "rules\napply\n--after-classify\n");
        std::fs::remove_file(&fixture).unwrap();
        let missing = hook(&config, &dir.join("mecha"), None).await;
        assert_eq!(missing.failed, 1);
        assert!(missing
            .diagnostic
            .contains("cargo install --path mecha-mail"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
