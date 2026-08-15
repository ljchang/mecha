//! `mecha doctor` — read every store, report what is wrong, offer the way out.
//!
//! The human half of [`mecha_core::doctor`]. The core module does the reading;
//! this side renders the findings grouped by component (broken first), adds
//! the one check that is not a store — failed `mecha-*` systemd units — and,
//! when a person is actually at the terminal, offers each remedy as a y/N
//! question.
//!
//! **There is deliberately no `--yes` flag.** A doctor that applies fixes with
//! nobody watching is the silently-degrading-sandbox shape this codebase keeps
//! cataloguing: the whole value of the report is that a person read it before
//! anything changed. Unattended (non-TTY) invocations report and exit — never
//! prompt, never fix — which also makes `mecha doctor --json` safe to put on
//! a timer.
//!
//! A confirmed remedy is spawned inheriting the real terminal — never
//! `.output()`, which would hand an OAuth flow a pipe for a screen and a
//! closed stdin for a keyboard (the `self_cli_interactive` lesson).

use anyhow::{Context, Result};
use mecha_core::doctor::{self, Finding, Remedy, Severity};

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Machine output: the findings as JSON. Never prompts, even on a TTY.
    #[arg(long)]
    pub json: bool,
}

pub async fn execute(args: Args) -> Result<()> {
    let home = mecha_core::work::mecha_home()?;
    let mut findings = doctor::examine(&home, chrono::Utc::now());

    // The systemd check lives here rather than in core: it shells out, and
    // `examine` stays a pure function over store roots. A dead-auth finding
    // reorders the advice — restarting a unit that will refail teaches
    // nothing — so the unit findings are built with that knowledge.
    let dead_auth = findings
        .iter()
        .any(|f| f.component == "mail" && f.severity == Severity::Broken);
    findings.extend(failed_units(dead_auth));
    doctor::sort(&mut findings);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&findings)?);
    } else {
        render(&findings);
        if interactive() {
            offer_remedies(&findings)?;
        }
    }

    if findings.is_empty() {
        Ok(())
    } else {
        // Findings are the diagnosis, not a malfunction of doctor itself, so
        // the exit code carries them without an error message on stderr.
        std::process::exit(1);
    }
}

/// The findings regrouped for display: every finding of a component together,
/// components ordered by their worst finding, broken before attention within
/// each — the shape `render` prints. Public because the TUI's `/doctor` modal
/// and the Slack `doctor` command render the same report, and three surfaces
/// that disagree about what comes first would each be read as the whole truth.
pub fn grouped(mut findings: Vec<Finding>) -> Vec<Finding> {
    doctor::sort(&mut findings);
    let mut components: Vec<String> = Vec::new();
    for f in &findings {
        if !components.contains(&f.component) {
            components.push(f.component.clone());
        }
    }
    let mut out = Vec::with_capacity(findings.len());
    for component in components {
        // Stable within a component: the sorted order already put broken
        // before attention, and insertion order after that.
        out.extend(
            findings
                .iter()
                .filter(|f| f.component == component)
                .cloned(),
        );
    }
    out
}

/// Only a human at both ends of the terminal gets offered anything. A piped
/// doctor — cron, a script, `| tee` — reports and stops.
fn interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Grouped by component, components ordered by their worst finding, broken
/// before attention within each — the order is [`grouped`]'s, not a private
/// copy of it: three surfaces render this report, and a walk re-implemented
/// here is exactly how they drift.
fn render(findings: &[Finding]) {
    if findings.is_empty() {
        println!("nothing wrong that this doctor can see");
        return;
    }
    let ordered = grouped(findings.to_vec());
    let mut component: Option<&str> = None;
    for f in &ordered {
        if component != Some(f.component.as_str()) {
            component = Some(f.component.as_str());
            println!("{}:", f.component);
        }
        println!("  [{}] {}", f.severity.as_str(), f.summary);
        for line in f.detail.lines() {
            println!("      {line}");
        }
        if let Some(remedy) = &f.remedy {
            println!("      remedy: {}", shell_words(&remedy.argv));
        }
    }
    let broken = ordered
        .iter()
        .filter(|f| f.severity == Severity::Broken)
        .count();
    println!(
        "\n{} finding{}: {} broken, {} for attention",
        ordered.len(),
        if ordered.len() == 1 { "" } else { "s" },
        broken,
        ordered.len() - broken
    );
}

/// Offer each distinct remedy once. Seven stuck drafts all pointing at
/// `mecha outbox review` are one question, not seven.
fn offer_remedies(findings: &[Finding]) -> Result<()> {
    use std::io::Write;
    let mut offered: Vec<&[String]> = Vec::new();
    let stdin = std::io::stdin();
    for finding in findings {
        let Some(remedy) = &finding.remedy else {
            continue;
        };
        if offered.contains(&remedy.argv.as_slice()) {
            continue;
        }
        offered.push(&remedy.argv);

        println!("\n{}", remedy.description);
        print!("run `{}`? [y/N] ", shell_words(&remedy.argv));
        std::io::stdout().flush()?;
        if !affirmed(&mut stdin.lock()) {
            println!("skipped");
            continue;
        }
        run_remedy(remedy)?;
    }
    Ok(())
}

/// One y/N answer, where EOF is "no" — the outbox `send` convention: silence
/// is not consent. Factored over any reader so the decision is unit-testable.
fn affirmed(reader: &mut impl std::io::BufRead) -> bool {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => {
            println!();
            false
        }
        Ok(_) => line.trim().eq_ignore_ascii_case("y"),
        Err(_) => false,
    }
}

/// Spawn the remedy inheriting the real terminal. `.status()` leaves stdin,
/// stdout and stderr connected, which is what an OAuth flow or an `$EDITOR`
/// inside `outbox review` needs — capturing them is the bug this comment
/// exists to prevent.
fn run_remedy(remedy: &Remedy) -> Result<()> {
    let (program, rest) = remedy
        .argv
        .split_first()
        .context("a remedy with an empty argv")?;
    let status = std::process::Command::new(program)
        .args(rest)
        .status()
        .with_context(|| format!("running `{}`", shell_words(&remedy.argv)))?;
    if status.success() {
        println!("`{}` finished", shell_words(&remedy.argv));
    } else {
        println!("`{}` exited with {status}", shell_words(&remedy.argv));
    }
    Ok(())
}

fn shell_words(argv: &[String]) -> String {
    argv.join(" ")
}

/// Failed `mecha-*` user units, best-effort. A machine without `systemctl` —
/// or without a user session bus — simply contributes nothing: doctor is an
/// observer, and an absent init system is not a finding.
fn failed_units(dead_auth: bool) -> Vec<Finding> {
    let output = match std::process::Command::new("systemctl")
        .args([
            "--user",
            "list-units",
            "--state=failed",
            "--no-legend",
            "--plain",
            "mecha-*",
        ])
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        // A failure to ask (no systemd, no session bus) is a silent skip by
        // design, not a finding about a component that may not exist.
        _ => return Vec::new(),
    };
    parse_failed_units(&output)
        .into_iter()
        .map(|unit| unit_finding(&unit, dead_auth))
        .collect()
}

/// Unit names from `--no-legend --plain` output: first column per line.
fn parse_failed_units(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let unit = line
                .trim_start_matches(['●', '×', '*', ' '])
                .split_whitespace()
                .next()?;
            unit.starts_with("mecha-").then(|| unit.to_string())
        })
        .collect()
}

/// The restart re-examination both tap surfaces share (SLACK-ACTIONS-DESIGN
/// §5, and F4): a restart is naturally idempotent against a *failed* unit but
/// disruptive against a *running* one — `mecha-triggers` restarted mid-run
/// cancels whatever it was doing — so the finding must still be true when the
/// press lands, not just when the card (or modal) was composed. Given the
/// unit's current failed-ness, `Some(line)` means skip the restart and say
/// this; `None` means run it. Lives here, beside the finding that composes
/// the remedy, because slack/ and tui/ must not import each other.
pub fn recovered_before_restart(unit: &str, is_failed: bool) -> Option<String> {
    (!is_failed).then(|| format!("{unit} already recovered — nothing was run"))
}

/// The recogniser the TUI's guard uses: the unit of a restart-shaped remedy
/// argv, or `None` for anything else — a non-restart remedy has no
/// re-examination to run.
pub fn restart_unit_of(argv: &[String]) -> Option<&str> {
    match argv {
        [systemctl, user, restart, unit]
            if systemctl == "systemctl" && user == "--user" && restart == "restart" =>
        {
            Some(unit)
        }
        _ => None,
    }
}

/// `systemctl --user is-failed <unit>` exits 0 exactly when the unit is
/// failed. A machine without systemd answers "not failed", which makes the
/// guard skip the restart and report "already recovered" — doctor would never
/// have composed the remedy there in the first place. The one probe both
/// surfaces run (the Slack executor wraps it in `spawn_blocking`).
pub fn unit_is_failed(unit: &str) -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "is-failed", unit])
        .stdin(std::process::Stdio::null())
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn unit_finding(unit: &str, dead_auth: bool) -> Finding {
    // Ordering matters: a unit that failed because a login is dead will fail
    // again on restart, and the retry teaches nothing. The remedy still
    // names the restart — it is the fix — but its description says what
    // comes first.
    let description = if dead_auth {
        format!(
            "re-authenticate the dead mail account first, then restart {unit} — \
             restarting a unit that will refail teaches nothing"
        )
    } else {
        format!("restart {unit}")
    };
    Finding {
        component: "systemd".to_string(),
        severity: Severity::Broken,
        summary: format!("unit {unit} has failed"),
        detail: format!("recent log: journalctl --user -u {unit} -n 20"),
        remedy: Some(Remedy {
            description,
            argv: vec![
                "systemctl".into(),
                "--user".into(),
                "restart".into(),
                unit.to_string(),
            ],
            needs_terminal: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_not_consent_and_only_yes_is_yes() {
        assert!(affirmed(&mut "y\n".as_bytes()));
        assert!(affirmed(&mut "Y\n".as_bytes()));
        assert!(!affirmed(&mut "n\n".as_bytes()));
        assert!(
            !affirmed(&mut "yes\n".as_bytes()),
            "only a bare y, like the outbox"
        );
        assert!(
            !affirmed(&mut "\n".as_bytes()),
            "enter alone is the default: no"
        );
        assert!(
            !affirmed(&mut "".as_bytes()),
            "EOF is no — a piped doctor releases nothing"
        );
    }

    #[test]
    fn json_output_round_trips() {
        let findings = vec![
            Finding {
                component: "mail".into(),
                severity: Severity::Broken,
                summary: "mail auth for `personal` is dead".into(),
                detail: "permanent refresh failure since 2026-08-11".into(),
                remedy: Some(Remedy {
                    description: "re-authenticate".into(),
                    argv: vec!["mecha-mail".into(), "auth".into(), "personal".into()],
                    needs_terminal: true,
                }),
            },
            Finding {
                component: "outbox".into(),
                severity: Severity::Attention,
                summary: "2 drafts pending for more than 48h".into(),
                detail: String::new(),
                remedy: None,
            },
        ];
        let text = serde_json::to_string_pretty(&findings).unwrap();
        let back: Vec<Finding> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, findings);
        // The severity is a word a script can match, not an index.
        assert!(text.contains("\"broken\""), "{text}");
        assert!(text.contains("\"attention\""), "{text}");
    }

    /// The display grouping every front-end shares: a component's findings
    /// stay together, the component with the broken finding leads, and broken
    /// outranks attention within a component.
    #[test]
    fn grouping_keeps_a_component_together_and_leads_with_what_is_broken() {
        let finding = |component: &str, severity: Severity, summary: &str| Finding {
            component: component.into(),
            severity,
            summary: summary.into(),
            detail: String::new(),
            remedy: None,
        };
        let scattered = vec![
            finding("outbox", Severity::Attention, "stuck drafts"),
            finding("mail", Severity::Broken, "dead auth"),
            finding("mail", Severity::Attention, "legacy login"),
            finding("frontdoor", Severity::Attention, "stale request"),
        ];
        let order: Vec<(String, Severity)> = grouped(scattered)
            .into_iter()
            .map(|f| (f.component, f.severity))
            .collect();
        assert_eq!(
            order,
            vec![
                ("mail".to_string(), Severity::Broken),
                ("mail".to_string(), Severity::Attention),
                ("frontdoor".to_string(), Severity::Attention),
                ("outbox".to_string(), Severity::Attention),
            ]
        );
    }

    #[test]
    fn failed_units_parse_from_plain_output_and_only_mecha_ones_count() {
        let output = "mecha-triggers.service loaded failed failed Mecha trigger daemon\n\
                      some-other.service loaded failed failed Unrelated\n\
                      ● mecha-frontdoor.service loaded failed failed Frontdoor drain\n";
        assert_eq!(
            parse_failed_units(output),
            vec!["mecha-triggers.service", "mecha-frontdoor.service"]
        );
        assert!(parse_failed_units("").is_empty());
    }

    /// F4's shared guard, failing on the old TUI behaviour: the /doctor
    /// modal used to spawn `systemctl restart` unconditionally from a stale
    /// confirm, killing a unit that had recovered mid-run. Both surfaces now
    /// route through this decision.
    #[test]
    fn a_recovered_unit_is_not_restarted_and_a_failed_one_is() {
        let skip = recovered_before_restart("mecha-triggers.service", false)
            .expect("a recovered unit skips the restart");
        assert!(skip.contains("already recovered"), "{skip}");
        assert!(skip.contains("mecha-triggers.service"), "{skip}");

        assert_eq!(
            recovered_before_restart("mecha-triggers.service", true),
            None,
            "a unit that is still failed is restarted"
        );
    }

    #[test]
    fn only_a_restart_shaped_remedy_names_a_unit_to_re_examine() {
        let argv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            restart_unit_of(&argv(&[
                "systemctl",
                "--user",
                "restart",
                "mecha-x.service"
            ])),
            Some("mecha-x.service")
        );
        for other in [
            argv(&["systemctl", "restart", "mecha-x.service"]),
            argv(&["systemctl", "--user", "stop", "mecha-x.service"]),
            argv(&["mecha", "trigger", "run", "briefing"]),
            argv(&["systemctl", "--user", "restart", "mecha-x.service", "--now"]),
            argv(&[]),
        ] {
            assert_eq!(restart_unit_of(&other), None, "{other:?}");
        }
    }

    #[test]
    fn a_dead_auth_reorders_the_unit_advice() {
        let plain = unit_finding("mecha-triggers.service", false);
        assert!(!plain
            .remedy
            .as_ref()
            .unwrap()
            .description
            .contains("re-auth"));

        let with_auth = unit_finding("mecha-triggers.service", true);
        let remedy = with_auth.remedy.unwrap();
        assert!(
            remedy.description.contains("re-authenticate"),
            "{}",
            remedy.description
        );
        // The argv itself is unchanged: the restart is still the fix.
        assert_eq!(
            remedy.argv,
            vec!["systemctl", "--user", "restart", "mecha-triggers.service"]
        );
        assert!(with_auth
            .detail
            .contains("journalctl --user -u mecha-triggers.service -n 20"));
    }
}
