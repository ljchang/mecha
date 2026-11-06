//! Shared gates and executable location for unattended and owner-driven passes.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Total hook allowance reserved beyond classify's original service budget.
/// Reserve grace and reap inside this allowance, including on a terminal.
pub const CLASSIFY_HOOK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
pub const CLASSIFY_HOOK_GRACE: std::time::Duration = std::time::Duration::from_secs(10);
pub const CLASSIFY_HOOK_CLEANUP: std::time::Duration = std::time::Duration::from_secs(2);

pub fn disabled_reason(config: &crate::config::Config) -> Option<String> {
    use crate::feature::{self, Feature};
    (!feature::switched_on(config, Feature::MailRules)).then(|| {
        format!(
            "mail_rules is off or requires mail; enable with `{}`",
            feature::enable_command(config, Feature::MailRules)
        )
    })
}

/// Exactly the launcher's rule: no PATH, project configuration or working directory.
pub fn child_path(executable: &Path) -> Result<PathBuf> {
    Ok(executable
        .parent()
        .context("executable has no parent")?
        .canonicalize()
        .context("resolving the executable directory")?
        .join("mecha-mail"))
}

pub fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub fn store_has_rules(home: &Path) -> Option<bool> {
    let root = super::RuleStore::root_under(home);
    match root.try_exists() {
        Ok(false) => Some(false),
        Ok(true) => super::RuleStore::open_existing(root)
            .and_then(|s| s.rule_names())
            .ok()
            .map(|names| !names.is_empty()),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mail_rules_needs_both_global_switches() {
        use crate::feature::{self, Feature};
        let mut cfg = crate::config::Config::default();
        assert!(disabled_reason(&cfg).is_some());
        feature::set_switch(&mut cfg, Feature::MailRules, true);
        assert!(disabled_reason(&cfg).is_some());
        feature::set_switch(&mut cfg, Feature::Mail, true);
        assert!(disabled_reason(&cfg).is_none());
        feature::set_switch(&mut cfg, Feature::MailRules, false);
        assert!(disabled_reason(&cfg).is_some());
    }
    #[test]
    fn the_mail_rules_units_are_the_scripts_copies() {
        assert_eq!(
            include_str!("../../units/mecha-mail-rules.service"),
            include_str!("../../../scripts/mecha-mail-rules.service")
        );
        assert_eq!(
            include_str!("../../units/mecha-mail-rules.timer"),
            include_str!("../../../scripts/mecha-mail-rules.timer")
        );
        let service = include_str!("../../units/mecha-mail-rules.service");
        assert!(service.contains("Environment=PATH=%h/.cargo/bin:"));
        assert!(service.contains("ExecStart=%h/.cargo/bin/mecha-mail rules apply --headers"));
        assert!(!service
            .lines()
            .filter(|line| !line.starts_with('#'))
            .any(|line| line.contains("model-idle")));
        assert!(
            !service.contains("SuccessExitStatus="),
            "only typed contention is a skip"
        );
        assert!(include_str!("../../units/mecha-mail-rules.timer").contains("OnCalendar=*:0/5"));
    }

    #[test]
    fn classify_services_reserve_ten_minutes_beyond_the_original_sweep_budgets() {
        assert!(
            include_str!("../../../scripts/mecha-mail-classify-day.service")
                .contains("TimeoutStartSec=40min")
        );
        assert!(include_str!("../../../scripts/mecha-mail-classify.service")
            .contains("TimeoutStartSec=55min"));
        assert_eq!(CLASSIFY_HOOK_TIMEOUT, std::time::Duration::from_secs(600));
        assert_eq!(CLASSIFY_HOOK_GRACE, std::time::Duration::from_secs(10));
        assert!(CLASSIFY_HOOK_GRACE + CLASSIFY_HOOK_CLEANUP < CLASSIFY_HOOK_TIMEOUT);
        assert_eq!(
            include_str!("../../units/mecha-mail-classify-day.service"),
            include_str!("../../../scripts/mecha-mail-classify-day.service")
        );
        assert_eq!(
            include_str!("../../units/mecha-mail-classify.service"),
            include_str!("../../../scripts/mecha-mail-classify.service")
        );
    }
}
