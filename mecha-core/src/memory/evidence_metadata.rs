//! Descriptive host evidence context, derived only from the authorized closure.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceMetadata {
    pub source_id: String,
    pub role: EvidenceRole,
    pub interpretation_influenced: bool,
    pub captures: Vec<CaptureAssociation>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceRole {
    /// No authenticated capture projection for this source is in the request closure.
    Unclassified,
    HistoricalCriterion,
    /// Owner evidence and rendered charter, without a confirmed task criterion.
    HistoricalChatContext,
    ExecutionReceipt,
    ArtifactCheck,
    UserStatement,
    AssistantStatement,
    ToolResult,
    ActionInvocation,
    ArtifactObservation,
    PriorInterpretation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureRelation {
    SelectedCapture,
    Dependency,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureAssociation {
    pub capture: RevisionRef,
    pub relation: CaptureRelation,
    pub capture_time: Option<String>,
    pub passage_time: Option<String>,
    pub transcript_line: Option<u64>,
}
fn passage_role(kind: &PassageKind) -> EvidenceRole {
    match kind {
        PassageKind::UserRequest => EvidenceRole::UserStatement,
        PassageKind::AssistantText => EvidenceRole::AssistantStatement,
        PassageKind::ToolResult => EvidenceRole::ToolResult,
        PassageKind::ActionInvocation => EvidenceRole::ActionInvocation,
        PassageKind::ArtifactObservation => EvidenceRole::ArtifactObservation,
    }
}
fn canonical_time(value: &Option<String>) -> Result<Option<String>> {
    value.as_deref().map(normalize_time).transpose()
}
pub(super) fn evidence_metadata(
    conn: &Connection,
    refs: &[RevisionRef],
    selected: &RevisionRef,
    sources: &[SourceField],
) -> Result<Vec<EvidenceMetadata>> {
    let mut projections = Vec::new();
    for reference in refs {
        let Some(Record::Evidence(e)) = lookup(conn, reference)? else {
            continue;
        };
        if matches!(
            e.source.connector.as_str(),
            "host-capture-projection-v1" | "host-capture-projection-v2"
        ) {
            let input = projection_input(conn, &e)?;
            let originals: String = conn.query_row(
                "SELECT originals FROM host_capture_receipt WHERE identity_hash=?1 AND state='active'",
                [&e.source.id], |row| row.get(0),
            )?;
            let originals: Vec<RevisionRef> = serde_json::from_str(&originals)?;
            ensure!(
                originals.len() == input.originals.len(),
                "invalid capture association count"
            );
            projections.push((reference, input, originals));
        }
    }
    projections.sort_by(|a, b| (&a.0.object, &a.0.revision).cmp(&(&b.0.object, &b.0.revision)));
    let mut metadata = Vec::new();
    for source in sources {
        let mut role = None;
        let mut captures = Vec::new();
        if source.kind == SourceKind::PriorInterpretation {
            role = Some(EvidenceRole::PriorInterpretation);
        } else {
            for (capture, input, originals) in &projections {
                let relation = if *capture == selected {
                    CaptureRelation::SelectedCapture
                } else {
                    CaptureRelation::Dependency
                };
                let mut associations = Vec::new();
                if **capture == source.source {
                    let r = match source.field.as_str() {
                        "/criterion" => EvidenceRole::HistoricalCriterion,
                        "/chat_context" => EvidenceRole::HistoricalChatContext,
                        "/receipts" => EvidenceRole::ExecutionReceipt,
                        "/artifact" => EvidenceRole::ArtifactCheck,
                        _ => anyhow::bail!("unknown capture source field"),
                    };
                    associations.push((r, None, None));
                } else {
                    for (reference, passage) in originals.iter().zip(&input.originals) {
                        if reference == &source.source {
                            associations.push((
                                passage_role(&passage.kind),
                                canonical_time(&passage.occurred_at)?,
                                Some(passage.line),
                            ));
                        }
                    }
                }
                for (r, passage_time, transcript_line) in associations {
                    ensure!(
                        role.is_none_or(|old| old == r),
                        "conflicting authenticated evidence roles"
                    );
                    role = Some(r);
                    captures.push(CaptureAssociation {
                        capture: (*capture).clone(),
                        relation,
                        capture_time: canonical_time(&input.occurred_at)?,
                        passage_time,
                        transcript_line,
                    });
                }
            }
        }
        metadata.push(EvidenceMetadata {
            source_id: source.id.clone(),
            role: role.unwrap_or(EvidenceRole::Unclassified),
            interpretation_influenced: source.kind == SourceKind::InterpretationInfluenced,
            captures,
        });
    }
    Ok(metadata)
}
