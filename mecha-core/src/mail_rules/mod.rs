//! Global mail-rule storage, and deterministic execution without provider I/O. Approvals resolve
//! latest-per-group and can only be lowered by changed inputs or computed holds;
//! ledger keys close by the union of evidence after exact-attempt settlement.
//! Rewrites use sibling rename under flock; audit histories are append-only.

mod approvals;
mod definition;
pub mod engine;
mod headers;
mod ledger;
mod store;

pub use approvals::*;
pub use definition::*;
pub use ledger::*;
pub use store::*;

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
