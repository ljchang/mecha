//! Page measurements are partitioned by group, current match digest and input.
use super::*;
use crate::closure::Actor;
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Serialize)]
pub struct RuleStatistics {
    pub verdict_input: VerdictInput,
    pub matched_actions: usize,
    pub matching_threads: usize,
    pub automatic_mailbox_actions: usize,
    pub owner_undos: usize,
    pub owner_wrong_matches: usize,
    pub owner_corrected_actions: usize,
    pub undo_rate: Option<f64>,
    pub wrong_match_rate: Option<f64>,
    pub correction_rate: Option<f64>,
    pub shadow_upper_bound: bool,
}

fn rate(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

pub(super) fn statistics(
    rule: &LoadedRule,
    group: ActionGroup,
    rows: &[LedgerRow],
) -> Result<Vec<RuleStatistics>> {
    let digest = rule.match_digest(group, GateStatus::Passing)?;
    let mut result = Vec::new();
    for verdict_input in [
        VerdictInput::NotApplicable,
        VerdictInput::Snippet,
        VerdictInput::FullBody,
        VerdictInput::Unknown,
    ] {
        let bucket: Vec<_> = rows
            .iter()
            .filter(|r| {
                r.rule == rule.name
                    && r.group == group
                    && r.match_digest == digest
                    && r.verdict_input == verdict_input
            })
            .collect();
        let mut actions = BTreeSet::new();
        let mut threads = BTreeSet::new();
        let mut automatic = BTreeSet::new();
        let mut undone = BTreeSet::new();
        let mut wrong = BTreeSet::new();
        let mut corrected = BTreeSet::new();
        for row in &bucket {
            let action = (
                row.message.account.clone(),
                row.message.provider_message_id.clone(),
                row.action.digest.clone(),
            );
            let thread = (row.message.account.clone(), row.message.thread_id.clone());
            if row.correction_of.is_none()
                && !row.matched_because.is_empty()
                && !matches!(
                    row.outcome,
                    Outcome::Pending | Outcome::ApprovalLapsed | Outcome::Unknown
                )
            {
                actions.insert(action.clone());
                threads.insert(thread.clone());
            }
            if row.outcome == Outcome::Done
                && row.group == ActionGroup::Mailbox
                && row.mode == Mode::Auto
            {
                automatic.insert(action.clone());
            }
            if row.actor == Some(Actor::Owner) {
                if row.counts_as_undo(rows) {
                    undone.insert(action.clone());
                    corrected.insert(action.clone());
                }
                if row.wrong_match {
                    wrong.insert(thread);
                    corrected.insert(action);
                }
            }
        }
        let owner_undos = automatic.intersection(&undone).count();
        let owner_wrong_matches = threads.intersection(&wrong).count();
        let owner_corrected_actions = actions.intersection(&corrected).count();
        result.push(RuleStatistics {
            verdict_input,
            matched_actions: actions.len(),
            matching_threads: threads.len(),
            automatic_mailbox_actions: automatic.len(),
            owner_undos,
            owner_wrong_matches,
            owner_corrected_actions,
            undo_rate: rate(owner_undos, automatic.len()),
            wrong_match_rate: rate(owner_wrong_matches, threads.len()),
            correction_rate: rate(owner_corrected_actions, actions.len()),
            shadow_upper_bound: rule.definition.is_description() || group.once_per_thread(),
        });
    }
    Ok(result)
}

pub(super) fn activity(
    name: &str,
    rows: &[LedgerRow],
    now: DateTime<Utc>,
) -> (usize, Option<DateTime<Utc>>, usize) {
    let fires: Vec<_> = rows
        .iter()
        .filter(|r| r.rule == name && matches!(r.outcome, Outcome::Done | Outcome::Sent))
        .filter_map(|r| r.at)
        .collect();
    (
        fires
            .iter()
            .filter(|at| **at >= now - Duration::days(7) && **at <= now)
            .count(),
        fires.into_iter().max(),
        rows.iter()
            .filter(|r| r.rule == name && r.outcome == Outcome::Failed)
            .count(),
    )
}

/// Preserve the ledger's wire fields, adding affordances without fetching mail.
/// Historical missing envelope text is null, never a provider lookup.
pub fn ledger_views(
    store: &RuleStore,
    rule: Option<&str>,
    limit: usize,
) -> Result<Vec<serde_json::Value>> {
    let all = store.ledger()?;
    recent_ledger(store, rule, limit)?
        .into_iter()
        .map(|row| {
            let undoable = prepare_undo(store, &row.id).is_ok();
            let wrong_match_allowed = wrong_match_original(store, &all, &row.id).is_ok()
                && !all
                    .iter()
                    .any(|other| other.outcome == Outcome::WrongMatch && other.same_key(&row));
            let mut value = serde_json::to_value(&row)?;
            for field in ["sender", "subject"] {
                if value[field].as_str().is_none_or(str::is_empty) {
                    value[field] = serde_json::Value::Null;
                }
            }
            value["undoable"] = serde_json::json!(undoable);
            value["wrong_match_allowed"] = serde_json::json!(wrong_match_allowed);
            Ok(value)
        })
        .collect()
}
