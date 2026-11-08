//! Fallible model support assessments; these types grant no factual or action authority.
use super::*;

#[path = "support_atomic.rs"]
mod atomic;
pub use atomic::*;

pub const SUPPORT_PROTOCOL: &str = "situated-factual-support-v1";
pub const SUPPORT_PRODUCER: &str = "situated-interpretation-v2";
pub const SUPPORT_REVISION_PROTOCOL: &str = "situated-factual-support-v2";
pub const SUPPORT_REVISION_PRODUCER: &str = "situated-interpretation-v3";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCitation {
    pub source_id: String,
    pub quote: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitKind {
    RecordedEvent,
    GoalAssessment,
    Hypothesis,
    Prediction,
}
impl UnitKind {
    fn factual(self) -> bool {
        matches!(self, Self::RecordedEvent | Self::GoalAssessment)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedUnit {
    pub id: String,
    pub kind: UnitKind,
    pub text: String,
    pub citations: Vec<SourceCitation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedJudgment {
    pub goal: String,
    pub bearing: InterpretationBearing,
    pub because: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportProposal {
    pub units: Vec<ProposedUnit>,
    pub judgments: Vec<ProposedJudgment>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportStatus {
    Supported,
    Contradicted,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportCheck {
    pub target: String,
    pub status: SupportStatus,
    pub unsupported_factual_premise: bool,
    pub citations: Vec<SourceCitation>,
    pub reason: String,
    pub questions: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportReply {
    pub checks: Vec<SupportCheck>,
}

/// No Deserialize: a public draft or JSON report is never an evaluated receipt.
#[derive(Debug)]
pub struct SupportRequest {
    prepared: PreparedInterpretation,
    proposal: SupportProposal,
    generation_input: String,
    generator_model: String,
}
impl SupportRequest {
    pub fn generation_input(&self) -> &str {
        &self.generation_input
    }
    pub fn proposal(&self) -> &SupportProposal {
        &self.proposal
    }
    pub fn prepared(&self) -> &PreparedInterpretation {
        &self.prepared
    }
    pub fn targets(&self) -> Vec<String> {
        targets(&self.proposal)
    }
}
#[derive(Debug)]
pub struct EvaluatedInterpretation {
    request: SupportRequest,
    reply: SupportReply,
    draft: InterpretationDraft,
    appraisal: Option<(AtomicAppraisal, String)>,
}
impl EvaluatedInterpretation {
    /// Diagnostics are explicitly outside the retained reader record.
    pub fn reply(&self) -> &SupportReply {
        &self.reply
    }
    pub fn proposal(&self) -> &SupportProposal {
        &self.request.proposal
    }
    pub fn draft(&self) -> &InterpretationDraft {
        &self.draft
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", deny_unknown_fields)]
pub enum UnitDisposition {
    Accepted {
        unit: ProposedUnit,
        status: SupportStatus,
    },
    Rejected {
        id: String,
        status: SupportStatus,
    },
}
impl UnitDisposition {
    pub fn id(&self) -> &str {
        match self {
            Self::Accepted { unit, .. } => &unit.id,
            Self::Rejected { id, .. } => id,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JudgmentReason {
    Retained,
    ExplicitUnknown,
    UnsupportedJudgment,
    UnsupportedPremises,
    DuplicateGoal,
    Superseded,
    Conflicting,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgmentDisposition {
    pub index: usize,
    pub goal: String,
    pub offered_bearing: InterpretationBearing,
    pub bearing: InterpretationBearing,
    pub because: Vec<String>,
    pub status: SupportStatus,
    pub reason: JudgmentReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsupported_factual_premise: Option<bool>,
}
/// Reader-safe report. Rejected text and evaluator diagnostics have no fields here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportReport {
    pub protocol: String,
    pub generator_model: String,
    pub evaluator_model: String,
    pub proposal_hash: String,
    pub request_hash: String,
    pub units: Vec<UnitDisposition>,
    pub judgments: Vec<JudgmentDisposition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atomic: Option<AtomicProvenance>,
}

pub fn parse_support_proposal(text: &str) -> Result<SupportProposal> {
    ensure!(text.len() <= 16384, "oversized support proposal");
    // Deserialize directly, never through Value: serde rejects repeated struct keys.
    Ok(serde_json::from_str(text)?)
}
pub fn parse_support_reply(text: &str) -> Result<SupportReply> {
    ensure!(text.len() <= 16384, "oversized support reply");
    Ok(serde_json::from_str(text)?)
}
fn unit_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn citations(sources: &[SourceField], selected: &[SourceCitation]) -> Result<()> {
    ensure!(selected.len() <= 2, "too many support citations");
    let mut seen = BTreeSet::new();
    for c in selected {
        ensure!(
            (12..=300).contains(&c.quote.chars().count()) && seen.insert((&c.source_id, &c.quote)),
            "invalid support citation"
        );
        let source = sources
            .iter()
            .find(|s| s.id == c.source_id)
            .context("unknown support source")?;
        ensure!(
            source.text.contains(&c.quote),
            "support quote is not an exact source substring"
        );
    }
    Ok(())
}
fn validate_proposal(
    context: &InterpretationContext,
    sources: &[SourceField],
    p: &SupportProposal,
) -> Result<()> {
    ensure!(
        p.units.len() <= 8 && p.judgments.len() <= 8 && serde_json::to_vec(p)?.len() <= 16384,
        "invalid support proposal bounds"
    );
    let mut ids = BTreeSet::new();
    for unit in &p.units {
        ensure!(
            unit_id(&unit.id)
                && ids.insert(&unit.id)
                && !unit.text.trim().is_empty()
                && unit.text.len() <= 512,
            "invalid support unit"
        );
        citations(sources, &unit.citations)?;
        ensure!(
            !unit.kind.factual() || !unit.citations.is_empty(),
            "factual unit without citation"
        );
    }
    for j in &p.judgments {
        ensure!(
            j.goal == context.criterion.goal
                && j.because.len() <= 8
                && j.because.iter().all(|id| ids.contains(id))
                && j.because.iter().collect::<BTreeSet<_>>().len() == j.because.len(),
            "invalid support judgment"
        );
    }
    Ok(())
}
pub fn validate_support_proposal(
    prepared: &PreparedInterpretation,
    proposal: &SupportProposal,
) -> Result<()> {
    validate_proposal(&prepared.context, &prepared.sources, proposal)
}
fn targets(p: &SupportProposal) -> Vec<String> {
    p.units
        .iter()
        .map(|u| format!("unit:{}", u.id))
        .chain(
            p.judgments
                .iter()
                .enumerate()
                .map(|(i, _)| format!("judgment:{i}")),
        )
        .collect()
}
fn validate_reply(request: &SupportRequest, reply: &SupportReply) -> Result<()> {
    ensure!(
        serde_json::to_vec(reply)?.len() <= 16384,
        "oversized support reply"
    );
    let expected = targets(&request.proposal)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for check in &reply.checks {
        ensure!(
            expected.contains(&check.target) && seen.insert(check.target.clone()),
            "duplicate or foreign support target"
        );
        citations(&request.prepared.sources, &check.citations)?;
        ensure!(
            !check.reason.trim().is_empty()
                && check.reason.len() <= 512
                && check.questions.len() <= 4
                && check
                    .questions
                    .iter()
                    .all(|q| !q.trim().is_empty() && q.len() <= 256),
            "invalid support diagnostics"
        );
        match check.status {
            SupportStatus::Supported => ensure!(
                !check.citations.is_empty() && check.questions.is_empty(),
                "invalid Supported check"
            ),
            SupportStatus::Unknown => ensure!(
                !check.questions.is_empty(),
                "Unknown check needs a question"
            ),
            SupportStatus::Contradicted => {}
        }
    }
    ensure!(seen == expected, "incomplete support checklist");
    Ok(())
}
fn check_for<'a>(reply: &'a SupportReply, target: &str) -> &'a SupportCheck {
    reply
        .checks
        .iter()
        .find(|c| c.target == target)
        .expect("validated complete checklist")
}

pub fn finish_support(
    request: SupportRequest,
    reply: SupportReply,
    evaluator_model: &str,
) -> Result<EvaluatedInterpretation> {
    finish_support_protocol(request, reply, evaluator_model, false)
}

pub fn finish_support_revised(
    request: SupportRequest,
    reply: SupportReply,
    evaluator_model: &str,
) -> Result<EvaluatedInterpretation> {
    finish_support_protocol(request, reply, evaluator_model, true)
}

fn finish_support_protocol(
    request: SupportRequest,
    reply: SupportReply,
    evaluator_model: &str,
    revised: bool,
) -> Result<EvaluatedInterpretation> {
    ensure!(
        identifier(evaluator_model, 256),
        "invalid support evaluator model"
    );
    validate_proposal(
        &request.prepared.context,
        &request.prepared.sources,
        &request.proposal,
    )?;
    validate_reply(&request, &reply)?;
    let units = request
        .proposal
        .units
        .iter()
        .map(|unit| {
            let check = check_for(&reply, &format!("unit:{}", unit.id));
            let accepted = !check.unsupported_factual_premise
                && match check.status {
                    SupportStatus::Supported => true,
                    SupportStatus::Unknown => !unit.kind.factual(),
                    SupportStatus::Contradicted => false,
                };
            if accepted {
                UnitDisposition::Accepted {
                    unit: unit.clone(),
                    status: check.status,
                }
            } else {
                UnitDisposition::Rejected {
                    id: unit.id.clone(),
                    status: check.status,
                }
            }
        })
        .collect::<Vec<_>>();
    let mut judgments: Vec<_> = request.proposal.judgments.iter().enumerate().map(|(index,j)| {
        let check = check_for(&reply, &format!("judgment:{index}"));
        let factual_support = !j.because.is_empty() && j.because.iter().all(|id| units.iter().any(|entry| matches!(entry, UnitDisposition::Accepted{unit,status:SupportStatus::Supported} if unit.id == *id && unit.kind.factual())));
        let reason = if index > 0 { JudgmentReason::DuplicateGoal }
            else if j.bearing == InterpretationBearing::Unknown { JudgmentReason::ExplicitUnknown }
            else if check.status != SupportStatus::Supported || check.unsupported_factual_premise { JudgmentReason::UnsupportedJudgment }
            else if !factual_support { JudgmentReason::UnsupportedPremises }
            else { JudgmentReason::Retained };
        JudgmentDisposition { index, goal: j.goal.clone(), offered_bearing: j.bearing,
            bearing: if reason == JudgmentReason::Retained {j.bearing} else {InterpretationBearing::Unknown},
            because: j.because.clone(), status: check.status, reason,
            unsupported_factual_premise: revised.then_some(check.unsupported_factual_premise) }
    }).collect();
    if revised {
        let decisions = revised_decisions(&units, &judgments)?;
        for (j, (reason, bearing)) in judgments.iter_mut().zip(decisions) {
            j.reason = reason;
            j.bearing = bearing;
        }
    }
    let report = SupportReport {
        protocol: if revised {
            SUPPORT_REVISION_PROTOCOL
        } else {
            SUPPORT_PROTOCOL
        }
        .into(),
        generator_model: request.generator_model.clone(),
        evaluator_model: evaluator_model.into(),
        proposal_hash: hash(&request.proposal)?,
        request_hash: request.prepared.request_id.clone(),
        units,
        judgments,
        atomic: None,
    };
    let draft = render_report(&request.prepared.context, &request.prepared.sources, report)?;
    Ok(EvaluatedInterpretation {
        request,
        reply,
        draft,
        appraisal: None,
    })
}

/// Reconstruct selection exclusively from preserved assessment determinants.
fn revised_decisions(
    units: &[UnitDisposition],
    judgments: &[JudgmentDisposition],
) -> Result<Vec<(JudgmentReason, InterpretationBearing)>> {
    let mut reasons = Vec::new();
    for j in judgments {
        let unsupported = j
            .unsupported_factual_premise
            .context("missing judgment premise flag")?;
        let factual = !j.because.is_empty()
            && j.because.iter().all(|id| {
                units.iter().any(|entry| {
                    matches!(entry,
            UnitDisposition::Accepted { unit, status: SupportStatus::Supported }
            if unit.id == *id && unit.kind.factual())
                })
            });
        reasons.push(if j.offered_bearing == InterpretationBearing::Unknown {
            JudgmentReason::ExplicitUnknown
        } else if j.status != SupportStatus::Supported || unsupported {
            JudgmentReason::UnsupportedJudgment
        } else if !factual {
            JudgmentReason::UnsupportedPremises
        } else {
            JudgmentReason::Retained
        });
    }
    let eligible: Vec<_> = reasons
        .iter()
        .enumerate()
        .filter_map(|(i, r)| (*r == JudgmentReason::Retained).then_some(i))
        .collect();
    let conflict = eligible.first().is_some_and(|first| {
        eligible
            .iter()
            .any(|i| judgments[*i].offered_bearing != judgments[*first].offered_bearing)
    });
    for (position, index) in eligible.iter().enumerate() {
        reasons[*index] = if conflict {
            JudgmentReason::Conflicting
        } else if position == 0 {
            JudgmentReason::Retained
        } else {
            JudgmentReason::Superseded
        };
    }
    Ok(reasons
        .into_iter()
        .zip(judgments)
        .map(|(reason, j)| {
            (
                reason,
                if reason == JudgmentReason::Retained {
                    j.offered_bearing
                } else {
                    InterpretationBearing::Unknown
                },
            )
        })
        .collect())
}

fn validate_report(
    context: &InterpretationContext,
    sources: &[SourceField],
    report: &SupportReport,
) -> Result<()> {
    ensure!(
        (report.protocol == SUPPORT_PROTOCOL
            || report.protocol == SUPPORT_REVISION_PROTOCOL
            || report.protocol == ATOMIC_SUPPORT_PROTOCOL)
            && identifier(&report.generator_model, 256)
            && identifier(&report.evaluator_model, 256)
            && digest(&report.proposal_hash)
            && digest(&report.request_hash)
            && report.units.len() <= 8
            && report.judgments.len() <= 8,
        "invalid support report"
    );
    if report.protocol == ATOMIC_SUPPORT_PROTOCOL {
        atomic::validate_atomic_report(report)?;
    } else {
        ensure!(
            report.atomic.is_none(),
            "atomic provenance in legacy report"
        );
    }
    let mut ids = BTreeSet::new();
    for entry in &report.units {
        ensure!(
            unit_id(entry.id()) && ids.insert(entry.id()),
            "invalid report unit identity"
        );
        if let UnitDisposition::Accepted { unit, status } = entry {
            ensure!(
                *status != SupportStatus::Contradicted
                    && (!unit.kind.factual() || *status == SupportStatus::Supported),
                "invalid accepted unit status"
            );
            validate_proposal(
                context,
                sources,
                &SupportProposal {
                    units: vec![unit.clone()],
                    judgments: vec![],
                },
            )?;
        }
    }
    let revised = report.protocol == SUPPORT_REVISION_PROTOCOL;
    let decisions = if revised {
        Some(revised_decisions(&report.units, &report.judgments)?)
    } else {
        None
    };
    for (index, j) in report.judgments.iter().enumerate() {
        ensure!(
            j.index == index
                && j.goal == context.criterion.goal
                && j.because.len() <= 8
                && j.because.iter().all(|id| ids.contains(id.as_str()))
                && j.because.iter().collect::<BTreeSet<_>>().len() == j.because.len(),
            "invalid report judgment"
        );
        if let Some(decisions) = &decisions {
            ensure!(
                (j.reason, j.bearing) == decisions[index],
                "noncanonical revised judgment"
            );
            continue;
        }
        ensure!(
            j.unsupported_factual_premise.is_none(),
            "legacy judgment premise flag"
        );
        let supported = !j.because.is_empty() && j.because.iter().all(|id| report.units.iter().any(|entry| matches!(entry,UnitDisposition::Accepted{unit,status:SupportStatus::Supported} if unit.id == *id && unit.kind.factual())));
        match j.reason {
            JudgmentReason::Superseded | JudgmentReason::Conflicting => {
                anyhow::bail!("new judgment reason in legacy report")
            }
            JudgmentReason::Retained => ensure!(
                index == 0
                    && j.offered_bearing != InterpretationBearing::Unknown
                    && j.bearing == j.offered_bearing
                    && j.status == SupportStatus::Supported
                    && supported,
                "invalid retained judgment"
            ),
            JudgmentReason::DuplicateGoal => ensure!(
                index > 0 && j.bearing == InterpretationBearing::Unknown,
                "invalid duplicate judgment"
            ),
            JudgmentReason::ExplicitUnknown => ensure!(
                index == 0
                    && j.offered_bearing == InterpretationBearing::Unknown
                    && j.bearing == InterpretationBearing::Unknown,
                "invalid explicit unknown"
            ),
            JudgmentReason::UnsupportedJudgment | JudgmentReason::UnsupportedPremises => ensure!(
                index == 0
                    && j.offered_bearing != InterpretationBearing::Unknown
                    && j.bearing == InterpretationBearing::Unknown,
                "invalid downgraded judgment"
            ),
        }
    }
    Ok(())
}
fn render_report(
    context: &InterpretationContext,
    sources: &[SourceField],
    report: SupportReport,
) -> Result<InterpretationDraft> {
    validate_report(context, sources, &report)?;
    let mut prose = Vec::new();
    let mut claims = Vec::new();
    let mut claim_map = BTreeMap::new();
    let mut dropped_by = BTreeMap::new();
    for entry in &report.units {
        match entry {
            UnitDisposition::Accepted { unit, .. } => {
                let label = match unit.kind {
                    UnitKind::RecordedEvent => "Recorded event",
                    UnitKind::GoalAssessment => "Goal assessment",
                    UnitKind::Hypothesis => "Speculative hypothesis",
                    UnitKind::Prediction => "Speculative prediction",
                };
                prose.push(format!("{label}: {}", unit.text));
                if unit.kind.factual() {
                    let mut indices = Vec::new();
                    for c in &unit.citations {
                        indices.push(claims.len());
                        claims.push(InterpretationClaim {
                            source_id: c.source_id.clone(),
                            statement: unit.text.clone(),
                            quote: c.quote.clone(),
                        });
                    }
                    claim_map.insert(unit.id.as_str(), indices);
                }
            }
            UnitDisposition::Rejected { status, .. } => {
                *dropped_by.entry(format!("{status:?}")).or_insert(0) += 1
            }
        }
    }
    if prose.is_empty() {
        prose.push("No supported statements retained.".into());
    }
    let selected = if report.protocol == SUPPORT_REVISION_PROTOCOL {
        report
            .judgments
            .iter()
            .find(|j| j.reason == JudgmentReason::Retained)
            .or_else(|| report.judgments.first())
    } else {
        report.judgments.first()
    };
    let judgments = selected
        .into_iter()
        .map(|j| InterpretationJudgment {
            goal: j.goal.clone(),
            bearing: j.bearing,
            because: if j.reason == JudgmentReason::Retained {
                j.because
                    .iter()
                    .flat_map(|id| claim_map[id.as_str()].clone())
                    .collect()
            } else {
                vec![]
            },
        })
        .collect();
    let offered = claims.len() + dropped_by.values().sum::<usize>();
    let draft = InterpretationDraft {
        interpretation: prose.join("\n"),
        claims,
        judgments,
        prediction: None,
        grounding: GroundingReport {
            offered,
            dropped: dropped_by.values().sum(),
            dropped_by,
            unresolved_goals: 0,
        },
        model: report.generator_model.clone(),
        producer_version: if report.protocol == ATOMIC_SUPPORT_PROTOCOL {
            ATOMIC_SUPPORT_PRODUCER
        } else if report.protocol == SUPPORT_REVISION_PROTOCOL {
            SUPPORT_REVISION_PRODUCER
        } else {
            SUPPORT_PRODUCER
        }
        .into(),
        support: Some(Box::new(report)),
    };
    ensure!(
        draft.interpretation.len() <= 8192 && serde_json::to_vec(&draft)?.len() <= 16384,
        "oversized evaluated interpretation"
    );
    Ok(draft)
}
pub(super) fn validate_evaluated_draft(
    context: &InterpretationContext,
    sources: &[SourceField],
    draft: &InterpretationDraft,
) -> Result<()> {
    let report = draft
        .support
        .clone()
        .context("missing evaluated support report")?;
    let expected = render_report(context, sources, *report)?;
    ensure!(
        serde_json::to_vec(&expected)? == serde_json::to_vec(draft)?,
        "noncanonical evaluated rendering"
    );
    Ok(())
}

impl Memory {
    fn recheck_support_prepared(&self, p: &PreparedInterpretation, access: &Access) -> Result<()> {
        self.check_access(access)?;
        self.live_identity()?;
        ensure!(
            self.conn
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?
                == 8,
            "evaluated interpretation requires version 8"
        );
        let previous = p
            .roots
            .iter()
            .filter(|r| *r != &p.receipt.projection)
            .cloned()
            .collect::<Vec<_>>();
        let fresh =
            self.prepare_interpretation(&p.receipt, &previous, &p.context, &p.snapshot, access)?;
        ensure!(
            fresh.instance == p.instance
                && fresh.access.domain == p.access.domain
                && fresh.access.scopes == p.access.scopes
                && fresh.request_id == p.request_id
                && fresh.generation_input == p.generation_input
                && fresh.evidence_metadata == p.evidence_metadata,
            "support preparation changed"
        );
        Ok(())
    }
    pub fn prepare_support(
        &self,
        prepared: &PreparedInterpretation,
        proposal: SupportProposal,
        access: &Access,
    ) -> Result<SupportRequest> {
        self.prepare_support_with_model(prepared, proposal, "authored", access)
    }
    /// Bind the observed generator identity supplied by the caller's host audit.
    pub fn prepare_support_with_model(
        &self,
        prepared: &PreparedInterpretation,
        proposal: SupportProposal,
        generator_model: &str,
        access: &Access,
    ) -> Result<SupportRequest> {
        self.recheck_support_prepared(prepared, access)?;
        ensure!(identifier(generator_model, 256), "invalid generator model");
        validate_proposal(&prepared.context, &prepared.sources, &proposal)?;
        let generation_input = serde_json::to_string(
            &serde_json::json!({"protocol":SUPPORT_PROTOCOL,"criterion":prepared.context.criterion,"sources":prepared.sources,"proposal":proposal,"targets":targets(&proposal)}),
        )?;
        ensure!(
            generation_input.len() <= 48 * 1024,
            "oversized support evaluation packet"
        );
        Ok(SupportRequest {
            prepared: prepared.clone(),
            proposal,
            generation_input,
            generator_model: generator_model.into(),
        })
    }
    pub fn retain_evaluated_interpretation(
        &mut self,
        p: PreparedInterpretation,
        evaluated: EvaluatedInterpretation,
        access: &Access,
    ) -> Result<InterpretationReceipt> {
        self.recheck_support_prepared(&p, access)?;
        ensure!(
            p.request_id == evaluated.request.prepared.request_id
                && p.instance == evaluated.request.prepared.instance
                && p.generation_input == evaluated.request.prepared.generation_input
                && p.evidence_metadata == evaluated.request.prepared.evidence_metadata,
            "foreign support evaluation"
        );
        // Re-run every structural assessment; the wrapper is not a success flag.
        let evaluator = evaluated
            .draft
            .support
            .as_ref()
            .context("missing support report")?
            .evaluator_model
            .clone();
        let protocol = &evaluated
            .draft
            .support
            .as_ref()
            .context("missing support report")?
            .protocol;
        let revised = match protocol.as_str() {
            SUPPORT_PROTOCOL | ATOMIC_SUPPORT_PROTOCOL => false,
            SUPPORT_REVISION_PROTOCOL => true,
            _ => anyhow::bail!("unknown support protocol"),
        };
        let mut rebuilt =
            finish_support_protocol(evaluated.request, evaluated.reply, &evaluator, revised)?;
        if let Some((appraisal, model)) = evaluated.appraisal {
            rebuilt = finish_appraisal(AppraisalRequest::new(rebuilt)?, appraisal, &model)?;
        }
        ensure!(
            serde_json::to_vec(&rebuilt.draft)? == serde_json::to_vec(&evaluated.draft)?,
            "mutated support evaluation"
        );
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        authenticate_receipt(&tx, &p.receipt)?;
        let refs = check_roots(
            &tx,
            &p.roots,
            &p.snapshot,
            access,
            self.source_guard.as_deref(),
        )?;
        ensure!(
            serde_json::to_vec(&source_fields(&tx, &refs)?)? == serde_json::to_vec(&p.sources)?,
            "prepared source fields changed"
        );
        ensure!(
            evidence_metadata(&tx, &refs, &p.receipt.projection, &p.sources)?
                == p.evidence_metadata,
            "prepared evidence metadata changed"
        );
        let draft = rebuilt.draft;
        let claim_sources = claim_sources_for(&p.sources, &draft);
        let record = Record::Interpretation(Interpretation {
            context_key: p.context.key.clone(),
            criterion: p.context.criterion.clone(),
            host_context_id: p.context.host_context_id.clone(),
            draft,
            claim_sources,
        });
        ensure!(
            serde_json::to_vec(&record)?.len() <= 16384,
            "oversized evaluated record"
        );
        let payload = hash(&record)?;
        let existing=tx.query_row("SELECT payload_hash,receipt_id,object_id,revision_id,state FROM interpretation_receipt WHERE request_id=?1",[&p.request_id],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,String>(4)?))).optional()?;
        let receipt = if let Some((old, id, object, revision, state)) = existing {
            ensure!(
                state == "active" && old.as_deref() == Some(&payload),
                "interpretation retry conflict or unavailable"
            );
            let reference = RevisionRef {
                object: object.context("missing interpretation object")?,
                revision: revision.context("missing interpretation revision")?,
            };
            current(&tx, &reference)?;
            InterpretationReceipt {
                id: id.context("missing interpretation receipt")?,
                reference,
            }
        } else {
            let mut scopes = BTreeSet::new();
            for r in &refs {
                scopes.extend(required_scopes(&tx, &r.revision)?);
            }
            let reference = append(&tx, None, &record, &scopes, &p.roots, "active")?;
            let receipt = InterpretationReceipt {
                id: uuid::Uuid::new_v4().to_string(),
                reference,
            };
            tx.execute(
                "INSERT INTO interpretation_receipt VALUES (?1,?2,?3,?4,?5,?6,'active')",
                params![
                    p.request_id,
                    payload,
                    receipt.id,
                    receipt.reference.object,
                    receipt.reference.revision,
                    p.context.key
                ],
            )?;
            receipt
        };
        check_roots(
            &tx,
            &p.roots,
            &p.snapshot,
            access,
            self.source_guard.as_deref(),
        )?;
        tx.commit()?;
        Ok(receipt)
    }
}

#[cfg(test)]
mod metadata_binding_tests {
    use super::*;
    use crate::memory::support_tests::{
        access, prepared_supported, proposal, reply, supported_fixture,
    };

    #[test]
    fn metadata_change_cannot_reuse_legacy_preparation_hash() {
        let (_, mut db, _) = supported_fixture();
        let mut prepared = prepared_supported(&mut db);
        let legacy_input = prepared.generation_input.clone();
        let legacy_id = prepared.request_id.clone();
        assert!(!prepared.evidence_metadata.is_empty());
        prepared.evidence_metadata.clear();
        assert_eq!(prepared.generation_input, legacy_input);
        assert_eq!(prepared.request_id, legacy_id);
        assert!(db
            .prepare_support(&prepared, proposal(&prepared), &access())
            .is_err());
    }

    #[test]
    fn evaluated_metadata_must_match_supplied_preparation() {
        let (_, mut db, _) = supported_fixture();
        let prepared = prepared_supported(&mut db);
        let request = db
            .prepare_support(&prepared, proposal(&prepared), &access())
            .unwrap();
        let assessed = reply(
            &prepared,
            SupportStatus::Supported,
            SupportStatus::Supported,
        );
        let mut evaluated = finish_support_revised(request, assessed, "authored").unwrap();
        assert!(!evaluated.request.prepared.evidence_metadata.is_empty());
        evaluated.request.prepared.evidence_metadata.clear();
        assert!(db
            .retain_evaluated_interpretation(prepared, evaluated, &access())
            .is_err());
    }
}

#[cfg(test)]
#[allow(unexpected_cfgs)]
mod atomic_appraisal_tests {
    use super::*;
    use crate::memory::support_tests::{
        access, prepared_supported, proposal as legacy_proposal, reply as legacy_reply,
        supported_fixture,
    };
    fn proposal(p: &PreparedInterpretation) -> SupportProposal {
        let mut proposed = legacy_proposal(p);
        proposed.units[0].id = "o1".into();
        proposed.judgments.clear();
        proposed
    }
    fn reply(
        p: &PreparedInterpretation,
        unit: SupportStatus,
        judgment: SupportStatus,
    ) -> SupportReply {
        let mut reply = legacy_reply(p, unit, judgment);
        reply.checks[0].target = "unit:o1".into();
        reply
    }
    #[test]
    fn atomic_appraisal_preserves_checks_and_derives_a_current_judgment() {
        let (_, mut db, _) = supported_fixture();
        let p = prepared_supported(&mut db);
        let mut units = proposal(&p);
        units.judgments.clear();
        let mut checks = reply(&p, SupportStatus::Supported, SupportStatus::Supported);
        checks.checks.retain(|c| c.target.starts_with("unit:"));
        let r = db.prepare_support(&p, units.clone(), &access()).unwrap();
        let e = finish_support(r, checks.clone(), "checker").unwrap();
        let request = AppraisalRequest::new(e).unwrap();
        let evaluated = finish_appraisal(
            request,
            AtomicAppraisal {
                state: AtomicCriterionState::Met,
                because: vec![units.units[0].id.clone()],
                reason: "The observations satisfy the criterion".into(),
                questions: vec![],
            },
            "appraiser",
        )
        .unwrap();
        assert_eq!(evaluated.proposal(), &units);
        assert_eq!(evaluated.reply(), &checks);
        assert_eq!(
            evaluated.draft().judgments[0].bearing,
            InterpretationBearing::Good
        );
        assert_eq!(
            evaluated.draft().support.as_ref().unwrap().protocol,
            "situated-atomic-appraisal-v4"
        );
        db.retain_evaluated_interpretation(p, evaluated, &access())
            .unwrap();
    }
    fn checked(
        db: &Memory,
        p: &PreparedInterpretation,
        kind: UnitKind,
        status: SupportStatus,
    ) -> EvaluatedInterpretation {
        let mut units = proposal(p);
        units.judgments.clear();
        units.units[0].kind = kind;
        let mut checks = reply(p, status, SupportStatus::Supported);
        checks.checks.retain(|c| c.target.starts_with("unit:"));
        finish_support(
            db.prepare_support(p, units, &access()).unwrap(),
            checks,
            "checker",
        )
        .unwrap()
    }
    fn appraisal(state: AtomicCriterionState, because: Vec<String>) -> AtomicAppraisal {
        AtomicAppraisal {
            state,
            because,
            reason: "Synthetic criterion assessment".into(),
            questions: if state == AtomicCriterionState::Unknown {
                vec!["What observation is missing?".into()]
            } else {
                vec![]
            },
        }
    }
    #[test]
    fn atomic_appraisal_rejects_foreign_duplicate_rejected_and_speculative_premises() {
        let (_, mut db, _) = supported_fixture();
        let p = prepared_supported(&mut db);
        for (kind, status, ids) in [
            (
                UnitKind::RecordedEvent,
                SupportStatus::Supported,
                vec!["foreign"],
            ),
            (
                UnitKind::RecordedEvent,
                SupportStatus::Supported,
                vec!["o1", "o1"],
            ),
            (
                UnitKind::RecordedEvent,
                SupportStatus::Contradicted,
                vec!["o1"],
            ),
            (UnitKind::RecordedEvent, SupportStatus::Unknown, vec!["o1"]),
            (UnitKind::Hypothesis, SupportStatus::Supported, vec!["o1"]),
        ] {
            let r = AppraisalRequest::new(checked(&db, &p, kind, status));
            assert!(r
                .and_then(|r| finish_appraisal(
                    r,
                    appraisal(
                        AtomicCriterionState::Met,
                        ids.into_iter().map(str::to_owned).collect()
                    ),
                    "appraiser"
                ))
                .is_err());
        }
        let r = AppraisalRequest::new(checked(
            &db,
            &p,
            UnitKind::RecordedEvent,
            SupportStatus::Supported,
        ))
        .unwrap();
        let mut a = appraisal(AtomicCriterionState::Unknown, vec![]);
        a.questions.clear();
        assert!(finish_appraisal(r, a, "appraiser").is_err());
    }
    #[test]
    fn atomic_appraisal_unknown_keeps_original_rejections_and_revocation_blocks_retention() {
        use std::sync::atomic::Ordering;
        let (_, mut db, guard) = supported_fixture();
        let p = prepared_supported(&mut db);
        let e = checked(
            &db,
            &p,
            UnitKind::RecordedEvent,
            SupportStatus::Contradicted,
        );
        let original = e.reply().clone();
        let r = AppraisalRequest::new(e).unwrap();
        let e = finish_appraisal(
            r,
            appraisal(AtomicCriterionState::Unknown, vec![]),
            "appraiser",
        )
        .unwrap();
        assert_eq!(e.reply(), &original);
        assert!(e.draft().claims.is_empty());
        assert_eq!(
            e.draft().judgments[0].bearing,
            InterpretationBearing::Unknown
        );
        guard.store(false, Ordering::SeqCst);
        assert!(db.retain_evaluated_interpretation(p, e, &access()).is_err());
    }
    #[test]
    #[cfg(feature = "memory-prototype")]
    fn atomic_generation_assigns_ids_and_rejects_model_ids_extra_judgments_and_ninth_atom() {
        use crate::memory_support_atomic as wire;
        use serde_json::json;
        let (_, mut db, _) = supported_fixture();
        let p = prepared_supported(&mut db);
        let (input, _) = wire::generation_input(&p).unwrap();
        let input: serde_json::Value = serde_json::from_str(&input).unwrap();
        let atom = json!({"text":"The source records a requested conversion", "evidence":input["catalog"]["citations"][0]["handle"]});
        let valid =
            json!({"request_key":input["request_key"],"observations":[atom.clone(),atom.clone()]});
        let result = wire::parse_generation(&p, &valid.to_string()).unwrap();
        assert_eq!(
            result
                .units
                .iter()
                .map(|u| u.id.as_str())
                .collect::<Vec<_>>(),
            vec!["o1", "o2"]
        );
        assert!(result.judgments.is_empty());
        assert_eq!(result.units[0].citations.len(), 1);
        let mut bad = valid.clone();
        bad["observations"][0]["id"] = json!("chosen");
        assert!(wire::parse_generation(&p, &bad.to_string()).is_err());
        let mut bad = valid.clone();
        bad["judgments"] = json!([]);
        assert!(wire::parse_generation(&p, &bad.to_string()).is_err());
        let mut bad = valid.clone();
        bad["observations"] = json!(vec![atom; 9]);
        assert!(wire::parse_generation(&p, &bad.to_string()).is_err());
        let mut bad = valid.clone();
        bad["observations"][0]["evidence"] = json!("foreign");
        assert!(wire::parse_generation(&p, &bad.to_string()).is_err());
        let duplicate = format!(
            "{{\"request_key\":{},\"observations\":[],\"observations\":[]}}",
            input["request_key"]
        );
        assert!(wire::parse_generation(&p, &duplicate).is_err());
    }
    #[test]
    #[cfg(feature = "memory-prototype")]
    fn atomic_checks_require_exact_coverage_and_original_citation_support() {
        use crate::memory_support_atomic as wire;
        use serde_json::json;
        let (_, mut db, _) = supported_fixture();
        let p = prepared_supported(&mut db);
        let mut proposed = proposal(&p);
        proposed.judgments.clear();
        let r = db.prepare_support(&p, proposed, &access()).unwrap();
        let (input, _) = wire::evaluation_input(&r).unwrap();
        let input: serde_json::Value = serde_json::from_str(&input).unwrap();
        let good = json!({"request_key":input["request_key"],"checks":[{"target":"unit:o1","status":"Supported","original_citation":"DirectSupport","reason":"Original source supports the complete claim","questions":[]}]});
        let parsed = wire::parse_evaluation(&r, &good.to_string()).unwrap();
        assert_eq!(parsed.checks[0].citations, r.proposal().units[0].citations);
        for relation in ["ContextOnly", "Contradiction", "Uncertain"] {
            let mut bad = good.clone();
            bad["checks"][0]["original_citation"] = json!(relation);
            assert!(wire::parse_evaluation(&r, &bad.to_string()).is_err());
        }
        let mut bad = good.clone();
        bad["checks"][0]["evidence"] = json!(["a different correct quote"]);
        assert!(wire::parse_evaluation(&r, &bad.to_string()).is_err());
        let mut bad = good.clone();
        bad["checks"] = json!([]);
        assert!(wire::parse_evaluation(&r, &bad.to_string()).is_err());
        let mut bad = good.clone();
        bad["checks"][0]["target"] = json!("unit:foreign");
        assert!(wire::parse_evaluation(&r, &bad.to_string()).is_err());
        let mut bad = good.clone();
        bad["checks"] = json!([good["checks"][0], good["checks"][0]]);
        assert!(wire::parse_evaluation(&r, &bad.to_string()).is_err());
        let mut unknown = good.clone();
        unknown["checks"][0]["status"] = json!("Unknown");
        unknown["checks"][0]["original_citation"] = json!("Uncertain");
        assert!(wire::parse_evaluation(&r, &unknown.to_string()).is_err());
        unknown["checks"][0]["questions"] = json!(["What evidence is missing?"]);
        assert!(wire::parse_evaluation(&r, &unknown.to_string()).is_ok());
    }
    #[test]
    #[cfg(feature = "memory-prototype")]
    fn atomic_zero_observations_can_abstain_and_direct_shares_input_without_independent_checks() {
        use crate::memory_support_atomic as wire;
        use serde_json::json;
        let (_, mut db, _) = supported_fixture();
        let p = prepared_supported(&mut db);
        let r = db
            .prepare_support(
                &p,
                SupportProposal {
                    units: vec![],
                    judgments: vec![],
                },
                &access(),
            )
            .unwrap();
        let e = finish_support(r, SupportReply { checks: vec![] }, "checker").unwrap();
        let r = AppraisalRequest::new(e).unwrap();
        let (input, schema) = wire::appraisal_input(&r).unwrap();
        assert!(!schema.to_string().contains("\"enum\":[]"));
        let input: serde_json::Value = serde_json::from_str(&input).unwrap();
        let response = json!({"request_key":input["request_key"],"state":"unknown","because":[],"reason":"No supported observations","questions":["What happened?"]});
        let a = wire::parse_appraisal(&r, &response.to_string()).unwrap();
        let e = finish_appraisal(r, a, "appraiser").unwrap();
        assert!(e.proposal().units.is_empty());
        assert!(e.reply().checks.is_empty());
        let (initial, _) = wire::generation_input(&p).unwrap();
        let (direct, _) = wire::direct_input(&p).unwrap();
        assert_eq!(initial, direct);
        let budget = crate::memory_factual_support::GenerationBudget::new(
            6144,
            std::time::Duration::from_secs(540),
        )
        .unwrap();
        assert_eq!(
            wire::direct_request(&p, "model", budget).unwrap().think,
            Some(false)
        );
        let stage = crate::memory_factual_support::GenerationBudget::new(
            512,
            std::time::Duration::from_secs(30),
        )
        .unwrap();
        assert_eq!(
            wire::generation_request(&p, "model", stage).unwrap().think,
            Some(false)
        );
        assert!(crate::memory_factual_support::generation_request(&p, "model", budget).is_err());
        let initial: serde_json::Value = serde_json::from_str(&initial).unwrap();
        let direct = json!({"request_key":initial["request_key"],"observations":[],"criterion":{"state":"unknown","because":[],"reason":"Missing observation","questions":["What happened?"]}});
        let result = wire::parse_direct(&p, &direct.to_string()).unwrap();
        assert!(result.proposal.judgments.is_empty());
        let mut bad = direct.clone();
        bad["criterion"]["because"] = json!([0]);
        assert!(wire::parse_direct(&p, &bad.to_string()).is_err());
    }
    #[test]
    fn atomic_appraisal_preparation_reauthorizes_sources() {
        let (_, mut db, guard) = supported_fixture();
        let p = prepared_supported(&mut db);
        let e = checked(&db, &p, UnitKind::RecordedEvent, SupportStatus::Supported);
        guard.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(db.prepare_appraisal(e, &access()).is_err());
    }
    #[test]
    fn atomic_appraisal_cannot_rescue_original_citation_with_a_replacement_check() {
        let (_, mut db, _) = supported_fixture();
        let p = prepared_supported(&mut db);
        let mut e = checked(&db, &p, UnitKind::RecordedEvent, SupportStatus::Supported);
        let original = &e.request.proposal.units[0].citations[0];
        let source = p
            .sources()
            .iter()
            .find(|s| s.id == original.source_id)
            .unwrap();
        let replacement = SourceCitation {
            source_id: source.id.clone(),
            quote: source.text.chars().take(20).collect(),
        };
        assert_ne!(&replacement, original);
        e.reply.checks[0].citations = vec![replacement];
        let e = finish_support(e.request, e.reply, "checker").unwrap();
        assert!(AppraisalRequest::new(e).is_err());
    }
    #[test]
    fn atomic_appraisal_refuses_legacy_nonatomic_shape() {
        let (_, mut db, _) = supported_fixture();
        let p = prepared_supported(&mut db);
        for mode in 0..3 {
            let mut e = checked(&db, &p, UnitKind::RecordedEvent, SupportStatus::Supported);
            if mode == 0 {
                e.request.proposal.units[0].id = "u1".into();
                e.reply.checks[0].target = "unit:u1".into();
            } else if mode == 1 {
                e.request.proposal.units[0].kind = UnitKind::GoalAssessment;
            } else {
                let original = e.request.proposal.units[0].citations[0].clone();
                let source = p
                    .sources()
                    .iter()
                    .find(|s| s.id == original.source_id)
                    .unwrap();
                let second = SourceCitation {
                    source_id: source.id.clone(),
                    quote: source.text.chars().take(20).collect(),
                };
                e.request.proposal.units[0].citations.push(second.clone());
                e.reply.checks[0].citations.push(second);
            }
            let e = finish_support(e.request, e.reply, "checker").unwrap();
            assert!(db.prepare_appraisal(e, &access()).is_err());
        }
    }
    #[test]
    #[cfg(feature = "memory-prototype")]
    fn atomic_wire_alternatives_are_complete_objects_for_pinned_grammar() {
        use crate::memory_support_atomic as wire;
        use serde_json::{json, Value};
        fn complete(branch: &Value, fields: &[&str]) {
            assert_eq!(branch["type"], "object");
            assert_eq!(branch["additionalProperties"], false);
            let required = branch["required"].as_array().unwrap();
            let properties = branch["properties"].as_object().unwrap();
            assert_eq!(required.len(), fields.len());
            assert_eq!(properties.len(), fields.len());
            for field in fields {
                assert!(required.contains(&json!(field)), "missing required {field}");
                assert!(properties.contains_key(*field), "missing property {field}");
            }
        }
        fn no_empty_enums(value: &Value) {
            match value {
                Value::Object(map) => {
                    if let Some(values) = map.get("enum") {
                        assert!(!values.as_array().unwrap().is_empty());
                    }
                    for v in map.values() {
                        no_empty_enums(v);
                    }
                }
                Value::Array(values) => {
                    for v in values {
                        no_empty_enums(v);
                    }
                }
                _ => {}
            }
        }
        let (_, mut db, _) = supported_fixture();
        let p = prepared_supported(&mut db);
        let request = db.prepare_support(&p, proposal(&p), &access()).unwrap();
        let (_, checks) = wire::evaluation_input(&request).unwrap();
        for branch in checks["properties"]["checks"]["items"]["oneOf"]
            .as_array()
            .unwrap()
        {
            complete(
                branch,
                &[
                    "target",
                    "status",
                    "original_citation",
                    "reason",
                    "questions",
                ],
            );
            assert_eq!(branch["properties"]["target"]["enum"], json!(["unit:o1"]));
            match branch["properties"]["status"]["enum"][0].as_str().unwrap() {
                "Supported" => {
                    assert_eq!(branch["properties"]["questions"]["maxItems"], 0);
                    assert_eq!(
                        branch["properties"]["original_citation"]["enum"],
                        json!(["DirectSupport"])
                    );
                }
                "Unknown" => assert_eq!(branch["properties"]["questions"]["minItems"], 1),
                "Contradicted" => {}
                _ => panic!("unknown status"),
            }
        }
        for status in [SupportStatus::Supported, SupportStatus::Contradicted] {
            let r = db
                .prepare_appraisal(checked(&db, &p, UnitKind::RecordedEvent, status), &access())
                .unwrap();
            let (input, schema) = wire::appraisal_input(&r).unwrap();
            let input: Value = serde_json::from_str(&input).unwrap();
            let branches = schema["oneOf"].as_array().unwrap();
            assert_eq!(
                branches.len(),
                if status == SupportStatus::Supported {
                    2
                } else {
                    1
                }
            );
            for branch in branches {
                complete(
                    branch,
                    &["request_key", "state", "because", "reason", "questions"],
                );
                assert_eq!(
                    branch["properties"]["request_key"]["enum"],
                    json!([input["request_key"]])
                );
                if branch["properties"]["state"]["enum"] == json!(["unknown"]) {
                    assert_eq!(branch["properties"]["questions"]["minItems"], 1);
                } else {
                    assert_eq!(branch["properties"]["because"]["minItems"], 1);
                    assert_eq!(branch["properties"]["questions"]["maxItems"], 0);
                }
            }
            no_empty_enums(&schema);
        }
        let (_, direct) = wire::direct_input(&p).unwrap();
        for branch in direct["properties"]["criterion"]["oneOf"]
            .as_array()
            .unwrap()
        {
            complete(branch, &["state", "because", "reason", "questions"]);
            assert_eq!(branch["properties"]["because"]["items"]["type"], "integer");
        }
        let empty = db
            .prepare_support(
                &p,
                SupportProposal {
                    units: vec![],
                    judgments: vec![],
                },
                &access(),
            )
            .unwrap();
        let (_, schema) = wire::evaluation_input(&empty).unwrap();
        assert_eq!(schema["properties"]["checks"]["maxItems"], 0);
        no_empty_enums(&schema);
    }
}
