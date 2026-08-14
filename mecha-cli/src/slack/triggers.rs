//! The `triggers` command word: the schedule, from the screen in hand.
//!
//! Same shape as `doctor`, and the same two boundaries hold it: the
//! connector's gate runs before the word is looked at, so a stranger's
//! `triggers` never reaches the matcher (the listing names every scheduled
//! prompt on the machine — the private surface); and the store read runs in
//! spawned work, off the ack path.
//!
//! Every button is composed from a typed [`Action`], so the payload is the
//! fixed verb and the trigger name — the argv is re-derived at tap time and
//! the name re-resolves against the trigger store. What a row offers follows
//! its state: a running trigger offers **Cancel** (re-running it is an
//! overlap-skip), an idle enabled one offers **Run** and **Disable**, and a
//! disabled one offers **Enable** and nothing else — the doctor report never
//! shows a disabled trigger (it is nobody's emergency), so this listing is
//! where a silenced trigger comes back.

use mecha_slack::blocks;
use serde_json::Value;

use super::actions::Action;

/// A message that is the word `triggers` and nothing else, however cased or
/// padded — "list my triggers please" is a prompt for the model, not a
/// command.
pub fn is_triggers_command(text: &str) -> bool {
    text.trim().eq_ignore_ascii_case("triggers")
}

/// One trigger, as the listing shows it. A plain struct so the rendering is a
/// pure function of it and testable without a store.
pub struct Row {
    pub name: String,
    pub enabled: bool,
    pub schedule: String,
    pub running: bool,
    /// The newest ledger row's status, if any.
    pub last: Option<String>,
}

/// Read the store and shape the answer for a thread: the notification
/// fallback line, and the blocks when there is a store to read. The reads
/// run on the blocking pool — this is called from a spawned task on the
/// connector's runtime, and a "plain spawned task" still runs on the very
/// threads every other thread's events dispatch on.
pub async fn listing() -> (String, Option<Vec<Value>>) {
    tokio::task::spawn_blocking(read_listing)
        .await
        .unwrap_or_else(|e| (format!("The trigger listing was lost: {e}"), None))
}

/// The blocking half: every store read, and the last-status words from the
/// ledger's tail.
fn read_listing() -> (String, Option<Vec<Value>>) {
    let Some(store) = mecha_core::trigger::TriggerStore::open_existing_default() else {
        return (
            "No triggers — `mecha trigger add` creates one.".to_string(),
            None,
        );
    };
    let (triggers, problems) = match store.list() {
        Ok(listed) => listed,
        Err(e) => return (format!("The trigger store could not be read: {e}"), None),
    };
    if triggers.is_empty() {
        return (
            "No triggers — `mecha trigger add` creates one.".to_string(),
            None,
        );
    }
    let names: std::collections::BTreeSet<&str> =
        triggers.iter().map(|t| t.name.as_str()).collect();
    let last = last_statuses(&store, &names);
    let rows: Vec<Row> = triggers
        .iter()
        .map(|t| Row {
            name: t.name.clone(),
            enabled: t.enabled,
            schedule: t.schedule.source().to_string(),
            running: store.running(&t.name).is_some(),
            last: last.get(&t.name).cloned(),
        })
        .collect();
    let enabled = rows.iter().filter(|r| r.enabled).count();
    let mut blocks_out = vec![blocks::section(&format!(
        "*Triggers:* {} known, {} enabled.",
        rows.len(),
        enabled
    ))];
    blocks_out.extend(listing_blocks(&rows));
    for p in &problems {
        blocks_out.push(blocks::context(&format!("unreadable trigger — {p}")));
    }
    (
        format!("Triggers: {} known, {} enabled.", rows.len(), enabled),
        Some(blocks::cap_blocks(blocks_out)),
    )
}

/// The newest ledger status word per listed trigger, from the tail.
///
/// The ledger is append-only, so the first row seen per trigger scanning
/// newest-first is its last word, and the scan stops the moment every listed
/// trigger has one — one status word each must not deserialize every run
/// ever recorded (the old full parse also died wholesale on one torn old
/// line, taking every trigger's status with it).
fn last_statuses(
    store: &mecha_core::trigger::TriggerStore,
    names: &std::collections::BTreeSet<&str>,
) -> std::collections::BTreeMap<String, String> {
    let mut last = std::collections::BTreeMap::new();
    let _ = store.scan_runs_rev(|run| {
        if names.contains(run.trigger.as_str()) {
            let status = run.status.as_str().to_string();
            last.entry(run.trigger).or_insert(status);
        }
        last.len() < names.len()
    });
    last
}

/// The rows as Block Kit: one section and one actions block per trigger, the
/// buttons composed from typed actions so verb and value cannot drift from
/// what `from_payload` parses. Capped visibly by the caller like every other
/// report.
pub fn listing_blocks(rows: &[Row]) -> Vec<Value> {
    let button = |action: &Action, label: &str, style: Option<&str>| {
        blocks::button(action.action_id(), label, &action.value(), style)
    };
    let mut out = Vec::new();
    for row in rows {
        let state = match (row.running, row.enabled) {
            (true, _) => "running now",
            (false, true) => "enabled",
            (false, false) => "disabled",
        };
        out.push(blocks::section(&format!(
            "`{}` · `{}` · {}{}",
            row.name,
            row.schedule,
            state,
            row.last
                .as_ref()
                .map(|s| format!(" · last {s}"))
                .unwrap_or_default()
        )));
        let controls = if row.running {
            // Re-running a mid-flight trigger is an overlap-skip; the honest
            // offers are the stop, and the silence.
            vec![
                button(
                    &Action::TriggerCancel {
                        name: row.name.clone(),
                    },
                    "Cancel run",
                    Some("danger"),
                ),
                button(
                    &Action::TriggerDisable {
                        name: row.name.clone(),
                    },
                    "Disable",
                    None,
                ),
            ]
        } else if row.enabled {
            vec![
                button(
                    &Action::TriggerRun {
                        name: row.name.clone(),
                    },
                    "Run now",
                    None,
                ),
                button(
                    &Action::TriggerDisable {
                        name: row.name.clone(),
                    },
                    "Disable",
                    None,
                ),
            ]
        } else {
            // A disabled trigger cannot run and cannot be cancelled; the one
            // thing it can do is come back.
            vec![button(
                &Action::TriggerEnable {
                    name: row.name.clone(),
                },
                "Enable",
                Some("primary"),
            )]
        };
        out.push(blocks::actions(controls));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack::actions::ids;

    fn row(name: &str, enabled: bool, running: bool) -> Row {
        Row {
            name: name.into(),
            enabled,
            schedule: "0 7 * * *".into(),
            running,
            last: Some("ok".into()),
        }
    }

    fn text(blocks: &[Value]) -> String {
        serde_json::to_string(blocks).unwrap()
    }

    #[test]
    fn the_word_matches_exactly_and_nothing_longer() {
        for word in ["triggers", "Triggers", "  TRIGGERS  "] {
            assert!(is_triggers_command(word), "{word:?} should fire");
        }
        for prose in [
            "trigger",
            "list my triggers please",
            "triggers?",
            "run triggers",
            "",
        ] {
            assert!(!is_triggers_command(prose), "{prose:?} should not fire");
        }
    }

    /// The listing's contract: what a row offers follows its state, and a
    /// disabled trigger's only offer is the way back — the enable half of the
    /// pair whose disable half rides on doctor's findings, because doctor
    /// never shows a disabled trigger.
    #[test]
    fn a_disabled_row_offers_enable_and_only_enable() {
        let rendered = listing_blocks(&[row("morning", false, false)]);
        let t = text(&rendered);
        assert!(t.contains(ids::TRIGGER_ENABLE), "{t}");
        assert!(!t.contains(ids::TRIGGER_DISABLE), "{t}");
        assert!(!t.contains(ids::TRIGGER_RUN), "a disabled trigger cannot run: {t}");
        assert!(!t.contains(ids::TRIGGER_CANCEL), "{t}");
    }

    #[test]
    fn an_enabled_idle_row_offers_run_and_disable_and_a_running_row_offers_cancel() {
        let idle = text(&listing_blocks(&[row("morning", true, false)]));
        assert!(idle.contains(ids::TRIGGER_RUN), "{idle}");
        assert!(idle.contains(ids::TRIGGER_DISABLE), "{idle}");
        assert!(!idle.contains(ids::TRIGGER_ENABLE), "{idle}");
        assert!(!idle.contains(ids::TRIGGER_CANCEL), "{idle}");

        let running = text(&listing_blocks(&[row("morning", true, true)]));
        assert!(running.contains(ids::TRIGGER_CANCEL), "{running}");
        assert!(
            !running.contains(ids::TRIGGER_RUN),
            "re-running a mid-flight trigger is an overlap-skip: {running}"
        );
    }

    /// The listing's status words come from the ledger tail: the newest row
    /// per listed trigger wins, and the scan survives — and never reaches —
    /// a torn old line. Fails on the old shape twice over: the full
    /// `store.runs()` parse dies wholesale on invalid UTF-8 anywhere in the
    /// file (pinned below), so the old `if let Ok(runs)` silently dropped
    /// every trigger's status the moment one ancient line tore.
    #[test]
    fn the_last_status_comes_from_the_ledger_tail_past_a_torn_old_line() {
        use mecha_core::trigger::{RunRecord, RunStatus, TriggerStore};
        use std::io::Write;

        let dir = std::env::temp_dir().join(format!(
            "mecha-slack-triggers-tail-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = TriggerStore::open(&dir).unwrap();

        // A torn, invalid-UTF-8 row from long ago, at the head of the file.
        {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(store.ledger_path())
                .unwrap();
            file.write_all(b"{\"trigger\": \"morning\xff\xfe\n").unwrap();
        }
        // An older error and a newer ok for `morning`: the tail's first
        // sighting — the newest row — is the word shown.
        let mut old = RunRecord::started("morning", None, false);
        old.status = RunStatus::Error;
        store.append_run(&old).unwrap();
        let mut prep = RunRecord::started("prep", None, false);
        prep.status = RunStatus::SkippedOverlap;
        store.append_run(&prep).unwrap();
        let mut new = RunRecord::started("morning", None, false);
        new.status = RunStatus::Ok;
        store.append_run(&new).unwrap();

        let names: std::collections::BTreeSet<&str> =
            ["morning", "prep", "never-ran"].into_iter().collect();
        let last = last_statuses(&store, &names);
        assert_eq!(last.get("morning").map(String::as_str), Some("ok"));
        assert_eq!(
            last.get("prep").map(String::as_str),
            Some("skipped (overlap)")
        );
        // No row is no status — the listing shows nothing, not a guess.
        assert_eq!(last.get("never-ran"), None);

        // The contrast the fix rests on, pinned: the full parse cannot even
        // read this ledger.
        assert!(store.runs().is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_button_value_is_the_trigger_name_and_never_a_command_fragment() {
        let rendered = listing_blocks(&[
            row("morning", true, false),
            row("nightly", false, false),
            row("prep", true, true),
        ]);
        for block in &rendered {
            let Some(elements) = block.get("elements").and_then(Value::as_array) else {
                continue;
            };
            for button in elements.iter().filter(|b| b["type"] == "button") {
                let value = button["value"].as_str().unwrap_or_default();
                assert!(
                    ["morning", "nightly", "prep"].contains(&value),
                    "a button value must be a trigger name, got {value:?}"
                );
            }
        }
    }
}
