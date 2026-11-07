//! Read-only mail-rule diagnostics. OAuth's address metadata is read, never tokens used.
use super::{Finding, MailAccounts, MailStores, Severity};
use crate::mail_rules::{self as rules, ApprovalReason, FailureKind, Mode, Outcome, RuleStore};
use chrono::{DateTime, Utc};
use std::{collections::BTreeMap, path::Path};

fn attention(summary: impl Into<String>, detail: impl Into<String>) -> Finding {
    Finding {
        component: "mail rules".into(),
        severity: Severity::Attention,
        summary: summary.into(),
        detail: detail.into(),
        remedy: None,
    }
}

fn accounts(mail: &MailStores, out: &mut Vec<Finding>) -> Vec<rules::RuleAccount> {
    let Some(root) = &mail.registry else {
        return Vec::new();
    };
    let text = match std::fs::read_to_string(root.join("accounts.toml")) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            out.push(Finding::unreadable("mail rules", "account registry", error));
            return Vec::new();
        }
    };
    let file: MailAccounts = match toml::from_str(&text) {
        Ok(file) => file,
        Err(error) => {
            out.push(Finding::unreadable("mail rules", "account registry", error));
            return Vec::new();
        }
    };
    #[derive(serde::Deserialize)]
    struct Address {
        account: Option<String>,
    }
    let mut result = Vec::new();
    for account in file.accounts {
        // The account registry names a directory, never an arbitrary path.
        if account.name.is_empty()
            || !account
                .name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
        {
            out.push(Finding::unreadable(
                "mail rules",
                "account registry",
                "invalid account name",
            ));
            continue;
        }
        let address = std::fs::read_to_string(root.join(&account.name).join("oauth.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Address>(&text).ok())
            .and_then(|row| row.account);
        if address.as_deref().is_none_or(|a| a.trim().is_empty()) {
            out.push(attention(format!("{}: no stored mailbox address", account.name),
                "Owner addresses are unknown: this holds every mail rule on this install, including rules for other accounts. Re-authorize this account with `mecha-mail auth` to record its address."));
        }
        result.push(rules::RuleAccount {
            name: account.name,
            proves_header_absence: account.provider == "google",
        });
    }
    result
}

pub fn check_mail_rules(home: &Path, mail: &MailStores, now: DateTime<Utc>) -> Vec<Finding> {
    let mut out = Vec::new();
    let accounts = accounts(mail, &mut out);
    let root = RuleStore::root_under(home);
    match root.try_exists() {
        Ok(false) => return out,
        Err(error) => {
            out.push(Finding::unreadable("mail rules", "rule store", error));
            return out;
        }
        Ok(true) => {}
    }
    let store = match RuleStore::open_existing(root) {
        Ok(store) => store,
        Err(error) => {
            out.push(Finding::unreadable("mail rules", "rule store", error));
            return out;
        }
    };
    if let Err(error) = store.approvals() {
        out.push(attention("corrupt or unreadable approvals history", format!(
            "{error:#}. The complete history is required: `off` cannot be written for a rule while approvals.jsonl is unreadable. Restore the history; disable mail_rules to stop unattended passes meanwhile.")));
    }
    // Each read is independent: damaged approvals must not conceal unknown ledger outcomes.
    match store.ledger() {
        Err(error) => out.push(Finding::unreadable("mail rules", "ledger", error)),
        Ok(rows) => {
            let timeout = rules::engine::EngineOptions::default().pending_timeout;
            let mut settlements: BTreeMap<&str, Vec<&rules::LedgerRow>> = BTreeMap::new();
            for row in &rows {
                if let Some(id) = row.settlement.as_deref() {
                    settlements.entry(id).or_default().push(row);
                }
            }
            for row in &rows {
                if row.outcome == Outcome::Unknown {
                    out.push(attention(format!("{}: unknown ledger outcome ({})", row.rule, row.id),
                        "This build cannot read the outcome. The action key remains closed; inspect with `mecha mail rule ledger` or use the build that wrote it."));
                }
                if matches!(row.outcome, Outcome::Pending | Outcome::UndoPending) {
                    let settlement = settlements
                        .get(row.id.as_str())
                        .into_iter()
                        .flatten()
                        .find(|other| other.settles(row));
                    match settlement {
                        None if row.at.is_none_or(|at| now - at > timeout) => out.push(attention(
                            format!("{}: stranded pending action ({})", row.rule, row.id),
                            "Its outcome is not recorded. Inspect the ledger; uncertain sends and tasks must not be retried as new actions. An unfinished undo resumes only through its original row.")),
                        Some(settled) if settled.failure_kind == FailureKind::Interrupted
                            || (settled.outcome == Outcome::Held && settled.reason == rules::FATE_UNKNOWN_REASON) => out.push(attention(
                            format!("{}: recovered pending action ({})", row.rule, row.id),
                            format!("Recovery recorded {:?}: {}. Inspect the ledger and provider state; a recovered interruption is not evidence of a completed action.", settled.outcome, settled.reason))),
                        _ => {}
                    }
                }
            }
        }
    }
    for (name, result) in [
        ("aliases", store.aliases().map(|_| ())),
        ("limits", store.limits().map(|_| ())),
        ("deferred queue", store.deferred().map(|_| ())),
    ] {
        if let Err(error) = result {
            out.push(Finding::unreadable("mail rules", name, error));
        }
    }
    match rules::rule_views(&store) {
        Ok(mut views) => {
            let _ = rules::account_holds(&store, &mut views, &accounts);
            let approvals = store.approvals().unwrap_or_default();
            for view in views {
                if let Some(error) = view.load_error {
                    out.push(Finding::unreadable(
                        "mail rules",
                        &format!("rule {}", view.name),
                        error,
                    ));
                }
                for hold in view.holds {
                    out.push(attention(format!("{}: {hold}", view.name), "This is a standing evaluation hold, not a zero match rate. Inspect `mecha mail rule show`."));
                }
                for group in view.groups {
                    let latest = approvals
                        .iter()
                        .rev()
                        .find(|row| row.rule == view.name && row.group == group.group);
                    let persisted_lapse = latest.is_some_and(|row| {
                        row.reason == ApprovalReason::Lapse && row.to_mode != Mode::Off
                    });
                    if persisted_lapse || group.holds.iter().any(|h| h.contains("approval lapsed"))
                    {
                        out.push(attention(format!("{} / {:?}: approval lapsed", view.name, group.group),
                            "Inputs changed; automatic permission is lowered until the owner reviews the current files and re-approves this group with `mecha mail rule mode`."));
                    }
                    for hold in group
                        .holds
                        .iter()
                        .filter(|h| h.contains("approval history"))
                    {
                        out.push(attention(
                            format!("{} / {:?}: {hold}", view.name, group.group),
                            "Review the approval history before changing this group's mode.",
                        ));
                    }
                }
            }
        }
        Err(error) => out.push(Finding::unreadable("mail rules", "rule views", error)),
    }
    out
}

/// Recorded auto permissions matter even when a provider/P1 ceiling currently
/// holds them. Every group has the same unconfined-shell self-grant residual.
pub fn check_mail_rule_shell(home: &Path, config: &crate::config::Config) -> Vec<Finding> {
    // Doctor does not probe backend readiness: Sandbox::preflight stops a run
    // whose configured sandbox fails, so shell cannot expose the store that way.
    // Exposure includes store writes and the network path through serve.
    let root = RuleStore::root_under(home);
    let exposed = config.sandbox.kind == crate::sandbox::Backend::None
        || config
            .sandbox
            .writable
            .iter()
            .any(|path| super::mount_reaches(path, &root));
    let network = crate::sandbox::Sandbox::new(config.sandbox.clone()).can_reach_network();
    if (!exposed && !network) || !config.tools.registers("shell") {
        return Vec::new();
    }
    let Ok(store) = RuleStore::open_existing(RuleStore::root_under(home)) else {
        return Vec::new();
    };
    let Ok(approvals) = store.approvals() else {
        return Vec::new();
    };
    rules::recorded_auto_groups(&approvals).into_iter().map(|(name, group)| {
        let mut routes = Vec::new();
        let mut details = Vec::new();
        if exposed {
            routes.push("shell can write the rule store");
            details.push("The actor gates are advisory when shell can write the rule store, including through a sandbox mount or its ancestor/alias. Configure a working sandbox with no writable exposure of the mecha home or rule store.");
        }
        if network {
            routes.push("shell can reach serve's loopback port");
            details.push("Shell network access can reach serve's loopback port and launch CLI children attributed as owner. Use a sandbox with network off and a backend that closes network access.");
        }
        details.push("This applies to Mailbox, Board and outbound groups alike; doctor checks global configuration only.");
        attention(format!("{name} / {group:?}: auto while {}", routes.join("; ")), details.join(" "))
    }).collect()
}

/// Pure reading of systemd's properties, supplied by the CLI (no network).
/// None means the query was unavailable, not that the unit is missing.
/// A missing unit is an explicit LoadState=not-found response.
pub fn check_mail_rules_timer(
    enabled: bool,
    timer: Option<&str>,
    service: Option<&str>,
) -> Vec<Finding> {
    if !enabled {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (name, properties, is_timer) in [
        ("mecha-mail-rules.timer", timer, true),
        ("mecha-mail-rules.service", service, false),
    ] {
        let problem = match properties {
            None => None,
            Some(text)
                if text.lines().any(|l| {
                    l == "LoadState=not-found"
                        || l == "LoadState=error"
                        || l == "LoadState=bad-setting"
                }) =>
            {
                Some("unit missing or unreadable")
            }
            Some(text) if text.lines().any(|l| l == "ActiveState=failed") => Some("unit failed"),
            Some(text) if !text.lines().any(|l| l == "LoadState=loaded") => {
                Some("unit load state unknown")
            }
            Some(text) if is_timer && !text.lines().any(|l| l == "ActiveState=active") => {
                Some("timer is not active")
            }
            Some(text)
                if !is_timer
                    && text
                        .lines()
                        .any(|l| l.starts_with("Result=") && l != "Result=success") =>
            {
                Some("last pass failed")
            }
            Some(text)
                if !is_timer
                    && (!text.lines().any(|l| l.starts_with("ActiveState="))
                        || !text.lines().any(|l| l == "Result=success")) =>
            {
                Some("service status unavailable")
            }
            _ => None,
        };
        if let Some(problem) = problem {
            out.push(attention(format!("{name}: {problem}"),
                "mail_rules is enabled. Install scripts/mecha-mail-rules.{service,timer} in ~/.config/systemd/user, run systemctl --user daemon-reload, then systemctl --user enable --now mecha-mail-rules.timer. Inspect the service journal before restarting a failed pass."));
        }
    }
    out
}
