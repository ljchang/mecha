//! Read models and owner operations shared by command and future web surfaces.
use super::*;
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path};

pub fn owner_file_holds() -> P1Holds {
    // P1 installs only owner-named files. Proposal provenance arrives in P7.
    P1Holds {
        origin_gate: GateStatus::Passing,
    }
}

#[derive(Debug, Serialize)]
pub struct GroupView {
    pub group: ActionGroup,
    pub recorded_mode: Mode,
    pub effective_mode: Mode,
    pub activated_at: Option<DateTime<Utc>>,
    pub holds: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RuleView {
    pub name: String,
    pub description: Option<String>,
    pub approval_digest: Option<ApprovalDigest>,
    pub groups: Vec<GroupView>,
    pub holds: Vec<String>,
    pub last_activity: Option<DateTime<Utc>>,
    pub load_error: Option<String>,
}

/// Uses append order to break timestamp ties, and keeps unknown outcomes intact.
pub fn recent_ledger(
    store: &RuleStore,
    rule: Option<&str>,
    limit: usize,
) -> Result<Vec<LedgerRow>> {
    let mut rows: Vec<_> = store
        .ledger()?
        .into_iter()
        .rev()
        .filter(|row| rule.is_none_or(|name| row.rule == name))
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.at));
    rows.truncate(limit);
    Ok(rows)
}

pub fn rule_views(store: &RuleStore) -> Result<Vec<RuleView>> {
    let approvals = store.approvals()?;
    let ledger = store.ledger()?;
    let mut views = Vec::new();
    let mut rules = Vec::new();
    let mut all_modes = BTreeMap::new();
    for name in store.rule_names()? {
        let loaded = store.get(&name);
        let mut view = RuleView {
            name: name.clone(),
            description: None,
            approval_digest: None,
            groups: Vec::new(),
            holds: Vec::new(),
            load_error: None,
            last_activity: ledger
                .iter()
                .filter(|row| row.rule == name)
                .filter_map(|row| row.at)
                .max(),
        };
        match loaded {
            Ok(rule) => {
                view.description = Some(rule.definition.description.clone());
                view.approval_digest = Some(rule.approval_digest.clone());
                let targets = engine::unacknowledged_brakes(&rule, &ledger, &approvals);
                let mut modes = BTreeMap::new();
                for group in rule.definition.groups() {
                    let mut mode = resolve_mode(&rule, group, &approvals, &owner_file_holds());
                    let mut holds: Vec<_> = mode.holds.iter().map(|h| h.reason.clone()).collect();
                    if mode.lapsed {
                        holds.push("approval lapsed: files changed".into());
                    }
                    if let Some(target) = targets.get(&group) {
                        mode.effective = mode.effective.lower(*target);
                        holds.push("automatic brake".into());
                    }
                    if approvals
                        .iter()
                        .rev()
                        .find(|r| r.rule == name && r.group == group)
                        .is_some_and(|r| r.reason == ApprovalReason::Brake)
                    {
                        holds.push("automatic brake recorded; owner re-approval required".into());
                    }
                    if mode.effective.rank() >= Mode::Propose.rank() && mode.activated_at.is_none()
                    {
                        holds.push("activation unknown".into());
                    }
                    let recorded = approvals
                        .iter()
                        .rev()
                        .find(|row| row.rule == rule.name && row.group == group)
                        .filter(|row| row.to_mode == Mode::Unknown)
                        .map(|_| Mode::Unknown)
                        .unwrap_or(mode.recorded);
                    view.groups.push(GroupView {
                        group,
                        recorded_mode: recorded,
                        effective_mode: mode.effective,
                        activated_at: mode.activated_at,
                        holds,
                    });
                    modes.insert(group, mode);
                }
                all_modes.insert(name, modes);
                rules.push(rule);
            }
            Err(error) => {
                view.load_error = Some(format!("{error:#}"));
                view.holds.push("rule definition unreadable".into());
                for group in approvals
                    .iter()
                    .filter(|r| r.rule == name)
                    .map(|r| r.group)
                    .collect::<std::collections::BTreeSet<_>>()
                {
                    let state = super::approvals::approval_state(&name, group, &approvals);
                    view.groups.push(GroupView {
                        group,
                        recorded_mode: state.mode,
                        effective_mode: state.mode.lower(Mode::Shadow),
                        activated_at: state.activated_at,
                        holds: vec!["rule definition unreadable".into()],
                    });
                }
            }
        }
        views.push(view);
    }
    let omitted = engine::list_cap(&rules, &all_modes);
    for view in &mut views {
        if omitted.contains(&view.name) {
            view.holds
                .push("held: list cap (12 entries); lower or disable another rule".into());
        }
    }
    Ok(views)
}

/// Owner-supplied filesystem paths; never a tool-input entrypoint. The store's
/// installer validates names, containment and definitions and always writes shadow.
pub fn install_path(store: &RuleStore, source: &Path, name: Option<&str>) -> Result<LoadedRule> {
    let source = source.canonicalize().context("reading rule source")?;
    let directory = if source.is_dir() {
        source.as_path()
    } else {
        ensure!(
            source.file_name().is_some_and(|n| n == "rule.toml"),
            "expected a rule directory or rule.toml"
        );
        source.parent().context("rule source has no directory")?
    };
    let name = name
        .or_else(|| directory.file_name().and_then(|n| n.to_str()))
        .context("rule source needs a name")?;
    let mut files = BTreeMap::new();
    let mut text = None;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let resolved = path.canonicalize()?;
        ensure!(
            resolved.starts_with(directory) && resolved.is_file(),
            "rule source must contain only regular files inside its directory"
        );
        let file = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("non-UTF8 rule file name"))?;
        let content = std::fs::read_to_string(&resolved)?;
        if file == "rule.toml" {
            text = Some(content);
        } else {
            files.insert(file, content);
        }
    }
    store.install(name, &text.context("rule.toml is missing")?, &files)
}

#[derive(Debug, Default)]
pub struct PauseReport {
    pub groups: usize,
    pub notices: Vec<String>,
}

pub fn pause_all(store: &RuleStore, actor: crate::closure::Actor) -> Result<PauseReport> {
    let mut count = 0;
    let mut errors = Vec::new();
    let targets = [
        ActionGroup::Mailbox,
        ActionGroup::PrivateRecord,
        ActionGroup::Report,
        ActionGroup::Board,
        ActionGroup::FixedWords,
        ActionGroup::VerbatimForward,
        ActionGroup::TypedFields,
        ActionGroup::DraftedProse,
    ]
    .into_iter()
    .map(|g| (g, Mode::Shadow))
    .collect();
    let (names, notices) = store.pause_names()?;
    for name in names {
        match store.lower_modes(
            &name,
            &targets,
            Some(actor),
            &owner_file_holds(),
            ApprovalReason::StepDown,
        ) {
            Ok(decisions) => count += decisions.iter().filter(|d| d.write).count(),
            Err(error) => errors.push(format!("{name}: {error:#}")),
        }
    }
    ensure!(
        errors.is_empty(),
        "pause-all incomplete: {}",
        errors.join("; ")
    );
    Ok(PauseReport {
        groups: count,
        notices,
    })
}

#[derive(Debug, Serialize)]
pub struct ReviewInputs {
    pub digest: ApprovalDigest,
    pub files: BTreeMap<String, String>,
}

/// Provider capabilities supplied by a host's account registry, never credentials.
pub struct RuleAccount {
    pub name: String,
    pub proves_header_absence: bool,
}

pub fn account_holds(
    store: &RuleStore,
    views: &mut [RuleView],
    accounts: &[RuleAccount],
) -> Result<()> {
    for view in views {
        let Ok(rule) = store.get(&view.name) else {
            continue;
        };
        if view
            .groups
            .iter()
            .all(|group| group.effective_mode == Mode::Off)
        {
            continue;
        }
        if rule.definition.matcher.list_id.is_empty() && rule.definition.matcher.header.is_empty() {
            continue;
        }
        for account in accounts {
            if !account.proves_header_absence
                && (rule.definition.matcher.account.is_empty()
                    || rule
                        .definition
                        .matcher
                        .account
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(&account.name)))
            {
                view.holds.push(format!("{}: provider cannot prove header absence for list_id/header matchers; evaluation held", account.name));
            }
        }
    }
    Ok(())
}

/// Retrospective matching intentionally ignores activation and historical action
/// keys. No store or effect interface exists on this path.
pub fn test_match(
    rule: &LoadedRule,
    candidate: &engine::Candidate,
    in_inbox: Option<bool>,
    now: DateTime<Utc>,
) -> Result<Vec<LedgerRow>> {
    ensure!(
        !rule.definition.is_description(),
        "description rule testing requires P4; only header rules can be tested"
    );
    if engine::header_match(&rule.definition.matcher, candidate) == engine::HeaderMatch::NoMatch {
        return Ok(Vec::new());
    }
    let unknown =
        engine::header_match(&rule.definition.matcher, candidate) == engine::HeaderMatch::Unknown;
    rule.definition
        .actions
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let mut row = LedgerRow::header(
                rule,
                index,
                candidate.message.clone(),
                Mode::Shadow,
                GateStatus::Passing,
                if unknown {
                    Outcome::Held
                } else {
                    Outcome::WouldHave
                },
                now,
            )?;
            row.closes = false;
            row.in_inbox = in_inbox;
            row.matched_because = serde_json::to_string(&rule.definition.matcher)?;
            row.reason = format!(
                "{}; {}",
                if unknown {
                    "test: matching facts unknown"
                } else {
                    "test: header match only; no action planned"
                },
                match in_inbox {
                    Some(false) => "message is no longer in the inbox",
                    Some(true) => "message is in the inbox",
                    None => "inbox membership unknown",
                }
            );
            Ok(row)
        })
        .collect()
}
