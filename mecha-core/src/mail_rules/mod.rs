//! Global mail-rule storage, and deterministic execution without provider I/O. Approvals resolve
//! latest-per-group and can only be lowered by changed inputs or computed holds;
//! ledger keys close by the union of evidence after exact-attempt settlement.
//! Rewrites use sibling rename under flock; audit histories are append-only.

mod approvals;
mod definition;
pub mod engine;
mod headers;
mod ledger;
mod management;
mod presentation;
mod runtime;
mod store;
mod undo;
mod waiting;

pub use approvals::*;
pub use definition::*;
pub use ledger::*;
pub use management::*;
pub use presentation::*;
pub use runtime::*;
pub use store::*;
pub use undo::*;
pub use waiting::waiting;

#[cfg(test)]
mod tests;

/// Stable ledger spelling for uncertain dispatch fate, including historical rows.
pub const FATE_UNKNOWN_REASON: &str = "fate unknown";

/// A hold's dispatch status is distinct from its diagnostic text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeldReason {
    FateUnknown,
    /// The host guarantees no effect ran and no background job was started.
    /// Release the pending claim; report a non-closing observation hold.
    NotDispatched(String),
    Other(String),
}
impl HeldReason {
    pub fn from_ledger(reason: &str) -> Self {
        if reason == FATE_UNKNOWN_REASON {
            Self::FateUnknown
        } else {
            Self::Other(reason.into())
        }
    }
}
impl From<String> for HeldReason {
    fn from(reason: String) -> Self {
        Self::Other(reason)
    }
}
impl From<&str> for HeldReason {
    fn from(reason: &str) -> Self {
        Self::Other(reason.into())
    }
}
impl std::fmt::Display for HeldReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::FateUnknown => FATE_UNKNOWN_REASON,
            Self::NotDispatched(reason) | Self::Other(reason) => reason,
        })
    }
}

/// Validated recorded permissions, before provider or phase ceilings. A corrupt
/// transition must not make an observer report a permission the engine rejected.
pub(crate) fn recorded_auto_groups(rows: &[ApprovalRow]) -> Vec<(String, ActionGroup)> {
    rows.iter()
        .map(|row| (row.rule.clone(), row.group))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|(rule, group)| approvals::approval_state(rule, *group, rows).mode == Mode::Auto)
        .collect()
}
