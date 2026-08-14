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

use mecha_core::doctor::{Finding, Severity};
use mecha_slack::blocks;

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
        Ok(findings) => (summary_line(&findings), Some(report_blocks(&findings))),
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

/// The findings as Block Kit: grouped by component, broken before attention,
/// each finding's summary, detail and remedy.
///
/// **Remedies are display-only on this surface, deliberately.** The command
/// line is rendered as copyable code and a `needs_terminal` remedy says so —
/// there is no button that executes an argv from a phone, because
/// tap-to-run-a-remedy needs its own design pass (approval semantics, which
/// tier may tap, idempotence of a re-tapped restart) and must not ride in as
/// a footnote on a rendering change.
pub fn report_blocks(findings: &[Finding]) -> Vec<serde_json::Value> {
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
    }
    // The block ceiling, made visible: Slack silently discards blocks past
    // its cap, and a health report missing its tail is exactly the silence
    // doctor exists to end. `cap_blocks` cuts and says so.
    blocks::cap_blocks(out)
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
        let text = blocks_text(&report_blocks(&findings));

        // Broken components lead; the attention-only one trails.
        let mail = text.find("*mail*").expect("mail header");
        let outbox = text.find("*outbox*").expect("outbox header");
        assert!(mail < outbox, "broken before attention: {text}");

        // Summary, detail and remedy all ride along.
        assert!(text.contains("mail is unwell"), "{text}");
        assert!(text.contains("the longer story"), "{text}");
        assert!(text.contains("re-authenticate the `personal` account"), "{text}");
        // The remedy's command line is copyable code, never a button — the
        // display-only rule.
        assert!(text.contains("`mecha-mail auth personal`"), "{text}");
        assert!(
            !text.contains("\"button\""),
            "no remedy executes from a tap: {text}"
        );
        // Exactly the terminal-bound remedy is labelled as such — not the
        // systemctl one beside it.
        assert_eq!(
            text.matches("needs a terminal").count(),
            1,
            "only mecha-mail auth is terminal-bound: {text}"
        );

        let healthy = blocks_text(&report_blocks(&[]));
        assert!(healthy.contains("nothing wrong"), "{healthy}");
    }

    /// Slack drops blocks past its cap of 50 with a warning nobody reads; a
    /// cut report has to say so itself.
    #[test]
    fn a_report_past_the_block_cap_is_truncated_visibly() {
        let many: Vec<Finding> = (0..60)
            .map(|i| finding(&format!("component-{i:02}"), Severity::Broken, None))
            .collect();
        let blocks = report_blocks(&many);
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
