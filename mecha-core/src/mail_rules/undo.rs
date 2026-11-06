//! A correction restores the authorized message, never reopens its action key.
use super::*;
use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreAction {
    Unarchive,
    Untrash,
    Read,
    Unread,
}

#[async_trait]
pub trait Restorer: Send + Sync {
    async fn restore(&self, message: &MessageRef, action: RestoreAction) -> Result<()>;
}

pub async fn undo(
    store: &RuleStore,
    row_id: &str,
    restorer: &dyn Restorer,
    now: DateTime<Utc>,
) -> Result<LedgerRow> {
    let _lease = store.engine_lease()?;
    let (original, action, pending) = undo_plan(store, row_id)?;
    let intent = if let Some(pending) = pending {
        pending
    } else {
        let mut intent = original.clone();
        intent.id = uuid::Uuid::new_v4().to_string();
        intent.at = Some(now);
        intent.outcome = Outcome::UndoPending;
        intent.closes = true;
        intent.settlement = None;
        intent.correction_of = Some(original.id.clone());
        intent.wrong_match = false;
        intent.reason = format!("undo intent for {}", original.id);
        store
            .append_ledger(&intent)
            .context("recording undo intent before provider I/O")?;
        intent
    };
    // Persist a safety brake before provider I/O, including when recovery resumes
    // an existing intent. No network await holds the short store writer lock.
    apply_undo_brake(store, &original.rule)?;
    let restored = restorer.restore(&original.message, action).await;
    let correction = intent.settled(
        if restored.is_ok() {
            Outcome::Undone
        } else {
            Outcome::Failed
        },
        match &restored {
            Ok(()) => format!("undo of {}", original.id),
            Err(error) => format!("undo of {} failed: {error:#}", original.id),
        },
        now,
    )?;
    store.append_ledger(&correction).with_context(|| {
        format!(
            "settling undo intent {}; retry undo {} to recover its uncertain fate",
            intent.id, original.id
        )
    })?;
    restored.context("restoring mailbox action; undo intent settled as failed")?;
    Ok(correction)
}

fn apply_undo_brake(store: &RuleStore, rule_name: &str) -> Result<()> {
    let targets = match store.get(rule_name) {
        Ok(rule) => engine::unacknowledged_brakes(&rule, &store.ledger()?, &store.approvals()?),
        // lower_modes invokes lapse_unreadable_locked: by the time it returns,
        // every historical group is durably clamped to shadow (off stays off).
        // Explicit targets retain that clamp if the files are restored between
        // this failed read and the writer transaction. A failed clamp must stop
        // the restore, never silently leave the old permission in place.
        Err(_) => store
            .approvals()?
            .into_iter()
            .filter(|row| row.rule == rule_name && row.group != ActionGroup::Unknown)
            .map(|row| (row.group, Mode::Shadow))
            .collect(),
    };
    store
        .lower_modes(
            rule_name,
            &targets,
            None,
            &owner_file_holds(),
            ApprovalReason::Brake,
        )
        .context(
            "undo intent recorded, but applying its safety brake failed; restore not attempted",
        )?;
    Ok(())
}

/// Read-only preflight for a host, repeated under the engine lease by undo.
pub fn prepare_undo(store: &RuleStore, row_id: &str) -> Result<(LedgerRow, RestoreAction)> {
    let (original, action, _) = undo_plan(store, row_id)?;
    Ok((original, action))
}

fn undo_plan(
    store: &RuleStore,
    row_id: &str,
) -> Result<(LedgerRow, RestoreAction, Option<LedgerRow>)> {
    let rows = store.ledger()?;
    let mut matches = rows.iter().filter(|row| row.id == row_id);
    let original = matches.next().context("unknown ledger row id")?;
    ensure!(matches.next().is_none(), "ledger row id is ambiguous");
    original.validate_key()?;
    ensure!(
        original.outcome == Outcome::Done,
        "undo requires a completed mailbox action; this row is {:?}",
        original.outcome
    );
    ensure!(original.group == ActionGroup::Mailbox, "no restore for this action group (spam, sends, tasks and private records cannot be undone here)");
    let action = match original.action.verb {
        ActionVerb::Archive => RestoreAction::Unarchive,
        ActionVerb::Trash => RestoreAction::Untrash,
        ActionVerb::Read => RestoreAction::Unread,
        ActionVerb::Unread => RestoreAction::Read,
        _ => anyhow::bail!("this action has no restore"),
    };
    ensure!(
        !rows
            .iter()
            .any(|row| row.outcome == Outcome::Undone && row.same_key(original)),
        "this mailbox action is already undone"
    );
    let corrections: Vec<_> = rows
        .iter()
        .filter(|row| row.same_key(original) && row.correction_of.is_some())
        .collect();
    let mut pending = corrections
        .iter()
        .filter(|row| row.outcome == Outcome::UndoPending);
    let intent = pending.next().copied();
    ensure!(
        pending.next().is_none(),
        "ambiguous undo intents; refusing another restore"
    );
    if let Some(intent) = intent {
        ensure!(
            intent.correction_of.as_deref() == Some(&original.id),
            "undo intent belongs to another completed row"
        );
        ensure!(
            !rows.iter().any(|row| row.settles(intent)),
            "this mailbox action's undo is already settled"
        );
        ensure!(
            corrections.len() == 1,
            "undo has unknown correction history; refusing another restore"
        );
    } else {
        ensure!(
            corrections.is_empty(),
            "undo has prior correction history; refusing another restore"
        );
    }
    Ok((original.clone(), action, intent.cloned()))
}
