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
/// fallback line, and the blocks when there is a store to read.
pub fn listing() -> (String, Option<Vec<Value>>) {
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
    let mut last: std::collections::BTreeMap<String, String> = Default::default();
    if let Ok(runs) = store.runs() {
        for run in runs {
            last.insert(run.trigger.clone(), run.status.as_str().to_string());
        }
    }
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
