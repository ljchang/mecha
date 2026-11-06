//! Owner CLI over the core store. Provider commands cross an argv-only child boundary.
use super::actor::acting_actor_explained;
use anyhow::{bail, ensure, Context, Result};
use mecha_core::mail_rules::{self as rules, ActionGroup, Mode, RuleStore};
use std::{
    io::{IsTerminal, Write},
    path::PathBuf,
};

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Install owner-named files in shadow, never auto.
    Add {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
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
    },
    /// Match one header rule against 30 days of mail, including archived mail. Never writes.
    Test {
        rule: String,
        #[arg(long)]
        account: Option<String>,
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
    /// Lower every group to shadow, preserving groups already off.
    PauseAll,
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

pub async fn run(cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Add { path, name } => {
            let store = RuleStore::open_default()?;
            let rule = rules::install_path(&store, &path, name.as_deref())?;
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
        Cmd::List { json } => {
            let views = if root()?.exists() {
                views(&existing()?)?
            } else {
                Vec::new()
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&views)?);
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
                println!("{}", serde_json::to_string_pretty(&value)?);
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
        } => {
            let store = existing()?;
            let (actor, why) = acting_actor_explained();
            let shown = if mode == Mode::Auto {
                let review = store.review_inputs(&rule)?;
                let loaded = store.get(&rule)?;
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
                println!("Approve {} / {} in auto", rule, spelling(&group));
                println!(
                    "Approval digest: {}",
                    serde_json::to_string_pretty(&review.digest)?
                );
                for (file, text) in &review.files {
                    println!("\n--- {file} ---\n{text}");
                }
                println!("Guards: exact message, activation cutoff, approval lapse, provider holds and automatic brakes.");
                if group == ActionGroup::Board {
                    println!("Tasks reach a task run's prompt and consume the rule/global daily caps. Propose is recommended.");
                }
                for hold in current.holds {
                    println!("Held: {}", hold.reason);
                }
                println!("Shadow rows under this group's current match digest (verdict_input kept separate):");
                let digest = loaded.match_digest(group, rules::GateStatus::Passing)?;
                for row in rules::recent_ledger(&store, Some(&rule), usize::MAX)?
                    .into_iter()
                    .filter(|r| {
                        r.group == group
                            && r.match_digest == digest
                            && r.outcome == rules::Outcome::WouldHave
                    })
                {
                    println!("{}", serde_json::to_string(&row)?);
                }
                if !yes {
                    ensure!(
                        std::io::stdin().is_terminal(),
                        "auto requires --yes or an interactive yes after reviewing the files"
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
                    .get(&rule)
                    .map(|r| r.approval_digest)
                    .unwrap_or_default()
            };
            let decision = store
                .change_mode(
                    &rule,
                    group,
                    mode,
                    Some(actor),
                    &shown,
                    &rules::owner_file_holds(),
                )
                .with_context(|| {
                    why.clone()
                        .unwrap_or_else(|| format!("actor: {}", actor.as_str()))
                })?;
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
            ensure!(
                !decision.refused,
                "mode refused by the store's permanent ceiling"
            );
            println!(
                "{} / {}: {} (actor: {})",
                rule,
                spelling(&group),
                spelling(&decision.target),
                actor.as_str()
            );
        }
        Cmd::Ledger { rule, limit, json } => {
            let rows = if root()?.exists() {
                rules::recent_ledger(&existing()?, rule.as_deref(), limit)?
            } else {
                Vec::new()
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
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
        Cmd::PauseAll => {
            let (actor, why) = acting_actor_explained();
            let count = if root()?.exists() {
                rules::pause_all(&existing()?, actor)?
            } else {
                0
            };
            println!(
                "Paused {count} groups to shadow (actor: {})",
                actor.as_str()
            );
            if let Some(why) = why {
                println!("Actor reading: {why}");
            }
        }
        Cmd::Test { rule, account } => {
            let mut args = vec!["rules".into(), "test".into(), rule];
            if let Some(account) = account {
                args.extend(["--account".into(), account]);
            }
            launch(&args).await?;
        }
        Cmd::Undo { row_id } => launch(&["rules".into(), "undo".into(), row_id]).await?,
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
    let executable = std::env::current_exe()?;
    let sibling = executable
        .parent()
        .context("executable has no parent")?
        .canonicalize()
        .context("resolving the executable directory")?
        .join("mecha-mail");
    let status = match tokio::process::Command::new(&sibling)
        .args(args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .await
    {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!("mecha-mail binary is missing; install it with `cargo install --path mecha-mail`")
        }
        Err(error) => return Err(error).context("launching mecha-mail"),
    };
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
    let directory = std::env::var_os("MECHA_MAIL_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".mecha/mail")));
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
