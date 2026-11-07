//! Opt-in, synthetic-data memory prototype. No live connectors or default tool registration.
//! Ordinary SQLite here is not qualified for private production data.
pub mod concept;
pub mod context;
pub mod decision;
pub mod durable;
pub mod durable_action;
pub mod durable_decision;
pub mod evaluation;
pub mod experience;
pub mod fixture;
mod forget;
pub mod guard_coverage;
pub mod host_capture;
mod model;
pub mod model_evaluation;
pub mod plan_assessment;
pub mod plan_extraction;
mod recall;
mod sql;
mod store;
mod transcript;
pub use model::*;
pub use recall::{RecallHit, RecallQuery};
pub use store::Memory;
pub use transcript::transcript_source;

#[cfg(test)]
mod evaluation_tests;
#[cfg(test)]
mod recall_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transcript_tests;

#[cfg(test)]
mod model_evaluation_tests;

#[cfg(test)]
mod context_tests;

#[cfg(test)]
mod concept_tests;

#[cfg(test)]
mod plan_assessment_tests;

#[cfg(test)]
mod guard_coverage_tests;

#[cfg(test)]
mod plan_extraction_tests;

#[cfg(test)]
mod durable_tests;

#[cfg(test)]
mod durable_decision_tests;

#[cfg(test)]
mod decision_tests;

#[cfg(test)]
mod durable_action_tests;

#[cfg(test)]
mod host_capture_tests;
