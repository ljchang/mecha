//! Waiting proposals and approvals, without creating stores or persisting lapses.
use super::*;
use crate::backlog::Waiter;
use anyhow::{ensure, Result};
use std::{collections::BTreeSet, path::Path};

pub fn waiting(root: &Path) -> Result<Vec<Waiter>> {
    if !root.try_exists()? {
        return Ok(Vec::new());
    }
    let store = RuleStore::open_existing(root)?;
    let approvals = store.approvals()?;
    let mut items = Vec::new();
    for name in store.rule_names()? {
        let rule = store.get(&name)?;
        let groups: BTreeSet<_> = rule
            .definition
            .groups()
            .into_iter()
            .chain(approvals.iter().filter(|r| r.rule == name).map(|r| r.group))
            .collect();
        for group in groups {
            let state = resolve_mode(&rule, group, &approvals, &owner_file_holds());
            let latest = approvals
                .iter()
                .rev()
                .find(|r| r.rule == name && r.group == group);
            if state.lapsed
                || latest
                    .is_some_and(|r| r.reason == ApprovalReason::Lapse && r.to_mode != Mode::Off)
            {
                items.push(Waiter::new(
                    format!("approval:{name}:{group:?}"),
                    latest
                        .and_then(|r| r.at)
                        .map(|at| at.to_rfc3339())
                        .unwrap_or_default(),
                ));
            }
        }
    }
    let proposals = store.root().join("proposals");
    if proposals.try_exists()? {
        let canonical = proposals.canonicalize()?;
        ensure!(
            canonical.starts_with(store.root()),
            "proposals escape the rule store"
        );
        for entry in std::fs::read_dir(&canonical)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 proposal name"))?;
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path().canonicalize()?;
            ensure!(path.starts_with(&canonical), "proposal escapes its store");
            ensure!(path.is_dir(), "proposal is not a directory");
            let definition = path.join("rule.toml").canonicalize()?;
            ensure!(
                definition.starts_with(&path),
                "proposal definition escapes its directory"
            );
            // P7 will own proposal creation/acceptance. The P1 reader counts
            // remaining proposal directories; filesystem age is labelled as such.
            let since: chrono::DateTime<chrono::Utc> =
                std::fs::metadata(definition)?.modified()?.into();
            items.push(Waiter::new(format!("proposal:{name}"), since.to_rfc3339()));
        }
    }
    Ok(items)
}
