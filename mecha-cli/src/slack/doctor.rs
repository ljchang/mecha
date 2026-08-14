//! The `doctor` command word: the health picture from the screen in hand.
//!
//! An owner-tier message that is the bare word `doctor` runs
//! `mecha doctor --json` as a child process and posts the findings as Block
//! Kit — the same report the terminal prints, because two surfaces that
//! disagree about what is wrong would each be read as the whole truth.
//!
//! Two boundaries hold this in shape:
//!
//! - **Owner-tier only.** The connector's gate runs before the word is even
//!   looked at, so the matcher never sees a stranger's message. That order is
//!   load-bearing: the findings name stores, accounts and stuck items — the
//!   user's private surface — and a report posted into a non-owner channel
//!   would hand all of it to whoever is standing there.
//! - **Never on the ack path.** The examination reads every store and shells
//!   out to `systemctl`; it runs in spawned work, because the three-second
//!   ack budget is Slack's and this report is nobody's emergency.

use std::collections::BTreeSet;

use mecha_core::doctor::{Finding, Severity};
use mecha_slack::blocks;

use super::actions::Action;

/// A message that is the word `doctor` and nothing else, however cased or
/// padded. Anything more is a prompt for the model, not a command — "run
/// doctor on this file" must not be swallowed by the front-end.
pub fn is_doctor_command(text: &str) -> bool {
    text.trim().eq_ignore_ascii_case("doctor")
}

/// Run the examination and shape the answer for a thread: the notification
/// fallback text, and the blocks when doctor answered.
///
/// Exit 1 is findings, not failure — the JSON on stdout is the answer for
/// both documented codes, and only unparseable output is reported as doctor
/// itself being unwell.
pub async fn report() -> (String, Option<Vec<serde_json::Value>>) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return (format!("doctor could not run: {e}"), None),
    };
    let out = match tokio::process::Command::new(exe)
        .args(["doctor", "--json"])
        .stdin(std::process::Stdio::null())
        .output()
        .await
    {
        Ok(out) => out,
        Err(e) => return (format!("doctor could not run: {e}"), None),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    match serde_json::from_str::<Vec<Finding>>(stdout.trim()) {
        Ok(findings) => {
            let running = running_triggers(&findings);
            (
                summary_line(&findings),
                Some(report_blocks(&findings, &running)),
            )
        }
        Err(_) => {
            let err = String::from_utf8_lossy(&out.stderr);
            (
                format!(
                    "doctor did not answer with findings: {}",
                    err.trim().lines().next().unwrap_or("no output")
                ),
                None,
            )
        }
    }
}

/// One line for the notification banner, where blocks do not render.
fn summary_line(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return "Doctor: nothing wrong that this doctor can see.".into();
    }
    let broken = findings
        .iter()
        .filter(|f| f.severity == Severity::Broken)
        .count();
    format!(
        "Doctor: {} finding(s) — {} broken, {} for attention.",
        findings.len(),
        broken,
        findings.len() - broken
    )
}

/// The trigger names, among these findings' recognised remedies, whose run is
/// in flight right now — read from the trigger store's running marker,
/// best-effort. This is the one place the connector reports a running trigger
/// context, so it is where a Cancel button can honestly appear (the design
/// pass's phase 1 item 4); a running trigger doctor has no finding about gets
/// no button, because there is no card to hang it on.
fn running_triggers(findings: &[Finding]) -> BTreeSet<String> {
    let Some(store) = mecha_core::trigger::TriggerStore::open_existing_default() else {
        return BTreeSet::new();
    };
    findings
        .iter()
        .filter_map(|f| f.remedy.as_ref())
        .filter_map(Action::from_remedy)
        .filter_map(|action| match action {
            Action::TriggerRun { name } if store.running(&name).is_some() => Some(name),
            _ => None,
        })
        .collect()
}

/// The findings as Block Kit: grouped by component, broken before attention,
/// each finding's summary, detail and remedy.
///
/// **A remedy [`Action::from_remedy`] recognises grows a one-tap button**;
/// everything else — every `needs_terminal` remedy, every unrecognised argv
/// shape — stays copyable code exactly as before, so a new remedy shape in
/// core is display-only here until someone deliberately adds a variant to the
/// closed enum. The button carries the fixed verb and the object id, never an
/// argv: the command line is re-derived from typed state at tap time. The
/// copyable code stays beside the button on purpose — the tap should never
/// run something the reader could not have read.
///
/// `running` names the trigger findings whose run is in flight: those offer
/// **Cancel** instead of a probe run, because re-running a trigger that is
/// mid-run is an overlap-skip and what the owner plausibly wants at that
/// moment is the stop.
pub fn report_blocks(
    findings: &[Finding],
    running: &BTreeSet<String>,
) -> Vec<serde_json::Value> {
    if findings.is_empty() {
        return vec![blocks::section(
            "*Doctor:* nothing wrong that this doctor can see.",
        )];
    }
    let grouped = crate::commands::doctor::grouped(findings.to_vec());
    let mut out = vec![blocks::section(&format!("*{}*", summary_line(&grouped)))];
    let mut component: Option<&str> = None;
    for finding in &grouped {
        if component != Some(finding.component.as_str()) {
            component = Some(finding.component.as_str());
            out.push(blocks::section(&format!("*{}*", finding.component)));
        }
        out.push(blocks::section(&finding_text(finding)));
        if let Some(action) = finding.remedy.as_ref().and_then(Action::from_remedy) {
            out.extend(action_blocks(&action, running));
        }
    }
    // The block ceiling, made visible: Slack silently discards blocks past
    // its cap, and a health report missing its tail is exactly the silence
    // doctor exists to end. `cap_blocks` cuts and says so.
    blocks::cap_blocks(out)
}

/// The button(s) for one recognised remedy. The payload is (fixed verb,
/// object id) — the executor re-resolves the id against its store at tap
/// time, and the restart re-examines the unit before running anything.
fn action_blocks(action: &Action, running: &BTreeSet<String>) -> Vec<serde_json::Value> {
    // Composed from the typed action's own verb and value, so the pair the
    // payload carries cannot drift from what `from_payload` parses.
    let button = |action: &Action, label: &str, style: Option<&str>| {
        blocks::button(action.action_id(), label, action.value(), style)
    };
    match action {
        Action::TriggerRun { name } if running.contains(name) => {
            let cancel = Action::TriggerCancel { name: name.clone() };
            vec![
                blocks::context(&format!(
                    "trigger `{name}` is running right now — Cancel stops it at its next \
                     safe point, partial turn kept"
                )),
                blocks::actions(vec![button(&cancel, "Cancel run", Some("danger"))]),
            ]
        }
        Action::TriggerRun { .. } => {
            vec![blocks::actions(vec![button(action, "Run now", None)])]
        }
        Action::RestartUnit { .. } => {
            vec![blocks::actions(vec![button(action, "Restart", Some("primary"))])]
        }
        // No other variant is constructible from a remedy; if one becomes so,
        // rendering nothing keeps it display-only until this match learns it.
        _ => Vec::new(),
    }
}

fn finding_text(finding: &Finding) -> String {
    let mut text = format!("[{}] {}", finding.severity.as_str(), finding.summary);
    if !finding.detail.is_empty() {
        text.push('\n');
        text.push_str(&finding.detail);
    }
    if let Some(remedy) = &finding.remedy {
        text.push_str(&format!(
            "\n{}\n`{}`",
            remedy.description,
            remedy.argv.join(" ")
        ));
        if remedy.needs_terminal {
            text.push_str("\n_needs a terminal — run it where there is one_");
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::doctor::Remedy;
    use mecha_slack::binding::{self, Binding};

    fn finding(component: &str, severity: Severity, remedy: Option<Remedy>) -> Finding {
        Finding {
            component: component.into(),
            severity,
            summary: format!("{component} is unwell"),
            detail: "the longer story".into(),
            remedy,
        }
    }

    fn blocks_text(blocks: &[serde_json::Value]) -> String {
        serde_json::to_string(blocks).unwrap()
    }

    #[test]
    fn the_word_matches_trimmed_and_case_insensitive_and_nothing_longer() {
        for word in ["doctor", "Doctor", "DOCTOR", "  doctor  ", "\ndoctor\n"] {
            assert!(is_doctor_command(word), "{word:?} should fire");
        }
        for text in [
            "doctors",
            "run doctor",
            "doctor please",
            "the doctor is in",
            "",
            "   ",
        ] {
            assert!(!is_doctor_command(text), "{text:?} should not fire");
        }
    }

    /// The connector's order is the guarantee: the gate runs before the word
    /// is looked at, so a non-owner's `doctor` never reaches the matcher —
    /// this pins the composition the handler relies on.
    #[test]
    fn a_non_owner_is_gated_before_the_word_is_ever_matched() {
        let bound = Binding {
            team_id: "T1".into(),
            enterprise_id: None,
            owners: vec!["U_OWNER".into()],
            bound_at: chrono::Utc::now(),
        };
        let owner = binding::check(Some(&bound), Some("U_OWNER"), Some("T1"));
        assert!(owner.is_allowed() && is_doctor_command("doctor"));

        let stranger = binding::check(Some(&bound), Some("U_STRANGER"), Some("T1"));
        assert!(
            !stranger.is_allowed(),
            "a stranger's message is refused before any command word is read"
        );
    }

    fn no_running() -> BTreeSet<String> {
        BTreeSet::new()
    }

    #[test]
    fn the_report_groups_by_component_with_broken_first_and_shows_the_remedy() {
        let findings = vec![
            finding("outbox", Severity::Attention, None),
            finding(
                "mail",
                Severity::Broken,
                Some(Remedy {
                    description: "re-authenticate the `personal` account".into(),
                    argv: vec!["mecha-mail".into(), "auth".into(), "personal".into()],
                    needs_terminal: true,
                }),
            ),
            finding(
                "systemd",
                Severity::Broken,
                Some(Remedy {
                    description: "restart it".into(),
                    argv: vec![
                        "systemctl".into(),
                        "--user".into(),
                        "restart".into(),
                        "mecha-triggers.service".into(),
                    ],
                    needs_terminal: false,
                }),
            ),
        ];
        let text = blocks_text(&report_blocks(&findings, &no_running()));

        // Broken components lead; the attention-only one trails.
        let mail = text.find("*mail*").expect("mail header");
        let outbox = text.find("*outbox*").expect("outbox header");
        assert!(mail < outbox, "broken before attention: {text}");

        // Summary, detail and remedy all ride along.
        assert!(text.contains("mail is unwell"), "{text}");
        assert!(text.contains("the longer story"), "{text}");
        assert!(text.contains("re-authenticate the `personal` account"), "{text}");
        // The remedy's command line stays copyable code beside any button —
        // the tap must never run something the reader could not have read.
        assert!(text.contains("`mecha-mail auth personal`"), "{text}");
        // Exactly the terminal-bound remedy is labelled as such — not the
        // systemctl one beside it.
        assert_eq!(
            text.matches("needs a terminal").count(),
            1,
            "only mecha-mail auth is terminal-bound: {text}"
        );

        let healthy = blocks_text(&report_blocks(&[], &no_running()));
        assert!(healthy.contains("nothing wrong"), "{healthy}");
    }

    /// The contract that replaced display-only: a remedy the closed enum
    /// recognises gets a one-tap button carrying (fixed verb, object id) —
    /// never an argv — and everything unrecognised stays copyable code with
    /// no button at all. Exactly the two shapes, nothing more.
    #[test]
    fn recognised_remedies_grow_buttons_and_everything_else_stays_copyable() {
        let findings = vec![
            finding(
                "systemd",
                Severity::Broken,
                Some(Remedy {
                    description: "restart it".into(),
                    argv: vec![
                        "systemctl".into(),
                        "--user".into(),
                        "restart".into(),
                        "mecha-triggers.service".into(),
                    ],
                    needs_terminal: false,
                }),
            ),
            finding(
                "triggers",
                Severity::Attention,
                Some(Remedy {
                    description: "run `briefing` by hand".into(),
                    argv: vec![
                        "mecha".into(),
                        "trigger".into(),
                        "run".into(),
                        "briefing".into(),
                    ],
                    needs_terminal: false,
                }),
            ),
            // Terminal-bound: an OAuth flow never becomes a button.
            finding(
                "mail",
                Severity::Broken,
                Some(Remedy {
                    description: "re-authenticate".into(),
                    argv: vec!["mecha-mail".into(), "auth".into(), "personal".into()],
                    needs_terminal: true,
                }),
            ),
            // Recognised by nobody: a terminal-surface doorway stays text.
            finding(
                "outbox",
                Severity::Attention,
                Some(Remedy {
                    description: "open the review surface".into(),
                    argv: vec!["mecha".into(), "outbox".into(), "review".into()],
                    needs_terminal: true,
                }),
            ),
        ];
        let rendered = report_blocks(&findings, &no_running());
        let text = blocks_text(&rendered);

        // Exactly two buttons, one per recognised shape.
        assert_eq!(text.matches("\"button\"").count(), 2, "{text}");
        assert!(text.contains("\"slack_action_restart_unit\""), "{text}");
        assert!(text.contains("\"slack_action_trigger_run\""), "{text}");
        assert!(!text.contains("\"slack_action_trigger_cancel\""), "{text}");

        // The payload is the object id only — never a command fragment.
        for block in &rendered {
            let Some(elements) = block.get("elements").and_then(|e| e.as_array()) else {
                continue;
            };
            for button in elements {
                let value = button["value"].as_str().unwrap_or_default();
                assert!(
                    value == "mecha-triggers.service" || value == "briefing",
                    "a button value must be a store id, got {value:?}"
                );
                assert!(!value.contains(' '), "no argv in a payload: {value:?}");
            }
        }

        // The unrecognised remedies keep their copyable code and gain nothing.
        assert!(text.contains("`mecha-mail auth personal`"), "{text}");
        assert!(text.contains("`mecha outbox review`"), "{text}");
    }

    /// Item 4 of the phase plan: where the report already carries a trigger
    /// finding, a run in flight turns the probe button into Cancel — the one
    /// action a phone most plausibly needs at an inconvenient hour. This is
    /// deliberately the only site trigger cancel surfaces on: the connector
    /// reports a running trigger context nowhere else, so a card anywhere
    /// else would be composed from state no store showed the reader.
    #[test]
    fn a_trigger_finding_whose_run_is_in_flight_offers_cancel_instead_of_run() {
        let findings = vec![finding(
            "triggers",
            Severity::Attention,
            Some(Remedy {
                description: "run `briefing` by hand".into(),
                argv: vec![
                    "mecha".into(),
                    "trigger".into(),
                    "run".into(),
                    "briefing".into(),
                ],
                needs_terminal: false,
            }),
        )];
        let running: BTreeSet<String> = ["briefing".to_string()].into();
        let text = blocks_text(&report_blocks(&findings, &running));

        assert!(text.contains("\"slack_action_trigger_cancel\""), "{text}");
        assert!(
            !text.contains("\"slack_action_trigger_run\""),
            "re-running a mid-flight trigger is an overlap-skip, not an offer: {text}"
        );
        assert!(text.contains("next safe point"), "{text}");

        // The same finding with nothing running offers the probe.
        let idle = blocks_text(&report_blocks(&findings, &no_running()));
        assert!(idle.contains("\"slack_action_trigger_run\""), "{idle}");
        assert!(!idle.contains("\"slack_action_trigger_cancel\""), "{idle}");
    }

    /// Slack drops blocks past its cap of 50 with a warning nobody reads; a
    /// cut report has to say so itself.
    #[test]
    fn a_report_past_the_block_cap_is_truncated_visibly() {
        let many: Vec<Finding> = (0..60)
            .map(|i| finding(&format!("component-{i:02}"), Severity::Broken, None))
            .collect();
        let blocks = report_blocks(&many, &no_running());
        assert!(
            blocks.len() <= mecha_slack::blocks::limits::BLOCKS_PER_MESSAGE,
            "{} blocks",
            blocks.len()
        );
        let text = blocks_text(&blocks);
        assert!(
            text.contains("more blocks not shown"),
            "the cut says so: {text}"
        );
    }
}
