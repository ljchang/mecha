//! Global mail-rule storage, without matching or mail I/O. Approvals resolve
//! latest-per-group and can only be lowered by changed inputs or computed holds;
//! ledger keys close by the union of evidence after exact-attempt settlement.
//! Rewrites use sibling rename under flock; audit histories are append-only.

mod approvals;
mod definition;
mod ledger;
mod store;

pub use approvals::*;
pub use definition::*;
pub use ledger::*;
pub use store::*;

#[cfg(test)]
mod tests;
