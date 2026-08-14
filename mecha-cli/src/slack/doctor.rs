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

use mecha_core::doctor::{Finding, Remedy, Severity};
use mecha_slack::blocks;

use super::actions::Action;

/// Doorway verbs for the terminal-surface remedies, which **translate rather
/// than spawn** (design §6): `mecha outbox review` and `mecha frontdoor list`
/// are review surfaces, and spawning either from Slack is meaningless — there
/// is no terminal. The button posts the pending items into the thread as the
/// cards the connector already knows how to make; the remedy's *intent* — put
/// the stuck thing in front of the human — is honoured, and the argv is never
/// executed, because it was never an action, only a doorway. Not in
/// `actions::ids` for exactly that reason: `from_payload` must not know them.
pub const REVIEW_HERE_OUTBOX: &str = "slack_review_here_outbox";
pub const REVIEW_HERE_FRONTDOOR: &str = "slack_review_here_frontdoor";

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
        } else if let Some(doorway) = finding.remedy.as_ref().and_then(review_here_block) {
            out.push(doorway);
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
        blocks::button(action.action_id(), label, &action.value(), style)
    };
    match action {
        Action::TriggerRun { name } if running.contains(name) => {
            let cancel = Action::TriggerCancel { name: name.clone() };
            let disable = Action::TriggerDisable { name: name.clone() };
            vec![
                blocks::context(&format!(
                    "trigger `{name}` is running right now — Cancel stops it at its next \
                     safe point, partial turn kept"
                )),
                blocks::actions(vec![
                    button(&cancel, "Cancel run", Some("danger")),
                    button(&disable, "Disable", None),
                ]),
            ]
        }
        // Doctor only reports enabled triggers (a disabled one is nobody's
        // emergency), so the pair a finding can offer is the probe and the
        // silence; the way back — Enable — lives on the `triggers` listing,
        // the one surface that shows disabled triggers at all.
        Action::TriggerRun { name } => {
            let disable = Action::TriggerDisable { name: name.clone() };
            vec![blocks::actions(vec![
                button(action, "Run now", None),
                button(&disable, "Disable", None),
            ])]
        }
        Action::RestartUnit { .. } => {
            vec![blocks::actions(vec![button(action, "Restart", Some("primary"))])]
        }
        Action::MailImport { .. } => {
            vec![blocks::actions(vec![button(action, "Import", Some("primary"))])]
        }
        // No other variant is constructible from a remedy; if one becomes so,
        // rendering nothing keeps it display-only until this match learns it.
        _ => Vec::new(),
    }
}

/// The terminal-surface remedies, translated (§6). Recognised by exact argv
/// shape like `from_remedy`, and anything else — including these argvs with
/// extra arguments — stays copyable text. The button's value is a constant
/// tag: it authorises nothing and resolves against nothing, because the
/// doorway has no object; the connector keys on the verb alone.
fn review_here_block(remedy: &Remedy) -> Option<serde_json::Value> {
    let argv: Vec<&str> = remedy.argv.iter().map(String::as_str).collect();
    match argv.as_slice() {
        ["mecha", "outbox", "review"] => Some(blocks::actions(vec![blocks::button(
            REVIEW_HERE_OUTBOX,
            "Review here",
            "outbox",
            Some("primary"),
        )])),
        ["mecha", "frontdoor", "list"] => Some(blocks::actions(vec![blocks::button(
            REVIEW_HERE_FRONTDOOR,
            "Review here",
            "frontdoor",
            Some("primary"),
        )])),
        _ => None,
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
            // A terminal-surface doorway: never executed, translated into a
            // Review-here button (§6) with its copyable code intact.
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

        // The restart's one button, the trigger finding's probe-and-silence
        // pair, and the outbox doorway (§6, translated not spawned): four.
        assert_eq!(text.matches("\"button\"").count(), 4, "{text}");
        assert!(text.contains("\"slack_action_restart_unit\""), "{text}");
        assert!(text.contains("\"slack_action_trigger_run\""), "{text}");
        assert!(text.contains("\"slack_action_trigger_disable\""), "{text}");
        assert!(text.contains(REVIEW_HERE_OUTBOX), "{text}");
        assert!(!text.contains("\"slack_action_trigger_cancel\""), "{text}");

        // The payload is the object id only — never a command fragment. The
        // doorway's value is a constant tag that authorises nothing.
        for block in &rendered {
            let Some(elements) = block.get("elements").and_then(|e| e.as_array()) else {
                continue;
            };
            for button in elements.iter().filter(|b| b["type"] == "button") {
                let value = button["value"].as_str().unwrap_or_default();
                assert!(
                    value == "mecha-triggers.service" || value == "briefing" || value == "outbox",
                    "a button value must be a store id, got {value:?}"
                );
                assert!(!value.contains(' '), "no argv in a payload: {value:?}");
            }
        }

        // The unrecognised remedies keep their copyable code, and the OAuth
        // flow gains nothing.
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

    /// Phase 2, and the fails-on-old-behaviour half: the outbox and
    /// frontdoor findings were button-less — their remedies are terminal
    /// surfaces, and `from_remedy` rightly refuses them forever. The doorway
    /// translates instead of spawning (§6): the button posts the pending
    /// items into the thread as cards; the argv is never executed.
    #[test]
    fn outbox_and_frontdoor_findings_grow_review_here_doorways() {
        let findings = vec![
            finding(
                "outbox",
                Severity::Broken,
                Some(Remedy {
                    description: "open the outbox review surface — doctor never releases a draft"
                        .into(),
                    argv: vec!["mecha".into(), "outbox".into(), "review".into()],
                    needs_terminal: true,
                }),
            ),
            finding(
                "frontdoor",
                Severity::Attention,
                Some(Remedy {
                    description: "list the frontdoor queue".into(),
                    argv: vec!["mecha".into(), "frontdoor".into(), "list".into()],
                    needs_terminal: false,
                }),
            ),
        ];
        let text = blocks_text(&report_blocks(&findings, &no_running()));
        assert!(text.contains(REVIEW_HERE_OUTBOX), "{text}");
        assert!(text.contains(REVIEW_HERE_FRONTDOOR), "{text}");
        // Doorways, not verbs: neither id is parseable into an executable
        // action, so a replayed press can at most re-post cards.
        assert_eq!(Action::from_payload(REVIEW_HERE_OUTBOX, "outbox"), None);
        assert_eq!(Action::from_payload(REVIEW_HERE_FRONTDOOR, "frontdoor"), None);
        // The copyable command survives beside the button.
        assert!(text.contains("`mecha outbox review`"), "{text}");
        assert!(text.contains("`mecha frontdoor list`"), "{text}");
    }

    /// A shape that merely resembles the doorway stays text — the same
    /// fail-closed direction as `from_remedy`.
    #[test]
    fn a_near_miss_of_the_doorway_shape_stays_copyable_text() {
        for argv in [
            vec!["mecha", "outbox", "review", "--all"],
            vec!["mecha", "outbox", "send"],
            vec!["mecha", "frontdoor", "list", "--state", "closed"],
            vec!["mecha", "frontdoor", "triage"],
        ] {
            let f = finding(
                "outbox",
                Severity::Attention,
                Some(Remedy {
                    description: "d".into(),
                    argv: argv.iter().map(|s| s.to_string()).collect(),
                    needs_terminal: false,
                }),
            );
            let text = blocks_text(&report_blocks(&[f], &no_running()));
            assert!(!text.contains("\"button\""), "{argv:?} must stay text: {text}");
        }
    }

    /// Phase 2: a trigger finding offers the silence beside the probe. Doctor
    /// only surfaces enabled triggers, so Disable is the half of the pair
    /// that makes sense here; Enable lives on the `triggers` listing, the one
    /// surface that shows a disabled trigger at all.
    #[test]
    fn a_trigger_finding_offers_disable_beside_the_probe_and_beside_cancel() {
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
        let idle = blocks_text(&report_blocks(&findings, &no_running()));
        assert!(idle.contains("\"slack_action_trigger_run\""), "{idle}");
        assert!(idle.contains("\"slack_action_trigger_disable\""), "{idle}");

        let running: BTreeSet<String> = ["briefing".to_string()].into();
        let mid_run = blocks_text(&report_blocks(&findings, &running));
        assert!(mid_run.contains("\"slack_action_trigger_cancel\""), "{mid_run}");
        assert!(mid_run.contains("\"slack_action_trigger_disable\""), "{mid_run}");
    }

    /// Phase 2: the legacy-store finding's remedy is a one-tap import — the
    /// fails-on-old direction is that this argv shape used to render as
    /// copyable text with no button at all.
    #[test]
    fn a_legacy_import_finding_grows_a_one_tap_import_button() {
        let findings = vec![finding(
            "mail",
            Severity::Broken,
            Some(Remedy {
                description: "bring the legacy google login into the unified registry".into(),
                argv: vec![
                    "mecha-mail".into(),
                    "import".into(),
                    "google".into(),
                    "--provider".into(),
                    "google".into(),
                ],
                needs_terminal: false,
            }),
        )];
        let rendered = report_blocks(&findings, &no_running());
        let text = blocks_text(&rendered);
        assert!(text.contains("\"slack_action_mail_import\""), "{text}");
        // The value is the provider, from the closed set — never the argv.
        for block in &rendered {
            let Some(elements) = block.get("elements").and_then(|e| e.as_array()) else {
                continue;
            };
            for button in elements.iter().filter(|b| b["type"] == "button") {
                assert_eq!(button["value"], "google", "{text}");
            }
        }
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
