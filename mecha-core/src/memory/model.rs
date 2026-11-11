use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionRef {
    pub object: String,
    pub revision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub instance: String,
    pub sequence: i64,
}

/// Host-supplied scope, never inferred from fixture text.
#[derive(Clone, Debug)]
pub struct Access {
    pub domain: String,
    pub scopes: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceKey {
    pub connector: String,
    pub account: String,
    pub id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capture {
    pub source: SourceKey,
    pub text: String,
    pub occurred_at: Option<String>,
    pub scopes: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<TranscriptMeta>,
}

/// Parsed source fields, never model-authored extractions. The raw JSONL line is
/// retained in Capture::text; pointers address strings in that exact record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptMeta {
    pub session_id: String,
    pub line: usize,
    pub record_type: String,
    pub speaker: Option<String>,
    pub fields: Vec<TextField>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextField {
    pub pointer: String,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub session_id: String,
    pub inserted: usize,
    pub unchanged: usize,
    pub updated_scopes: usize,
    pub records: Vec<RevisionRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "at", rename_all = "snake_case")]
pub enum Bound {
    Known(String),
    Unknown,
    Unbounded,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Interval {
    pub from: Bound,
    pub to: Bound,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Span {
    pub evidence: RevisionRef,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Target {
    Entity(RevisionRef),
    Literal(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelationInput {
    pub predicate: String,
    pub subject: RevisionRef,
    pub object: Target,
    pub support: Span,
    pub consulted: Vec<RevisionRef>,
    pub text: String,
    pub valid: Interval,
    pub polarity: i8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Record {
    Evidence(Capture),
    Entity { label: String },
    Relation(RelationInput),
    Concept(super::concept::Concept),
    Interpretation(super::interpretation::Interpretation),
    Task(super::task::TaskRevision),
    Plan(super::plan::PlanRevision),
}

#[derive(Clone, Debug)]
pub struct Query {
    pub text: String,
    pub known_at: Snapshot,
    pub valid_at: Option<String>,
    pub include_possible: bool,
    pub limit: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Hit {
    pub reference: RevisionRef,
    pub sequence: i64,
    pub record: Record,
    pub possible_time: bool,
    pub required_scopes: BTreeSet<String>,
    /// Output labels, never instruction authority. Everything recalled is
    /// private; `untrusted_input` is derived from `origin` by the one rule in
    /// `Origin::untrusted`, so unknown reads as untrusted.
    pub private_data: bool,
    pub untrusted_input: bool,
    /// Whether an evidence passage's bytes came from outside the harness,
    /// when a live capture says: the one field of `origin` a reader of the
    /// packet is shown, absent when unknown. The passage's own source id
    /// already names its session, line and field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<bool>,
    /// The whole origin, for a caller in this process. Never on the wire: a
    /// packet is cut to a byte budget, and the capture's own record carries
    /// the same descriptors when it is delivered.
    #[serde(skip)]
    pub origin: Option<super::host_capture::Origin>,
}

#[derive(Debug, Serialize)]
pub struct ForgetReport {
    pub objects: usize,
    pub revisions: usize,
    pub security_epoch: i64,
    pub scope: &'static str,
}
