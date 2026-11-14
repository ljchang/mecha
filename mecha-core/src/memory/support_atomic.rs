//! Opaque transition from independently checked observations to a derived appraisal.
use super::*;

pub const ATOMIC_SUPPORT_PROTOCOL: &str = "situated-atomic-appraisal-v4";
pub const ATOMIC_SUPPORT_PRODUCER: &str = "situated-interpretation-v4";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtomicCriterionState {
    Met,
    NotMet,
    Unknown,
}
impl AtomicCriterionState {
    fn bearing(self) -> InterpretationBearing {
        match self {
            Self::Met => InterpretationBearing::Good,
            Self::NotMet => InterpretationBearing::Bad,
            Self::Unknown => InterpretationBearing::Unknown,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomicAppraisal {
    pub state: AtomicCriterionState,
    pub because: Vec<String>,
    pub reason: String,
    pub questions: Vec<String>,
}
/// Reader-visible provenance identifies the judgment as derived, never independently checked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomicProvenance {
    pub extraction_binding: String,
    pub checking_binding: String,
    pub appraisal_binding: String,
    pub appraiser_model: String,
}

#[derive(Debug)]
pub struct AppraisalRequest {
    checked: EvaluatedInterpretation,
}
impl AppraisalRequest {
    pub(crate) fn new(checked: EvaluatedInterpretation) -> Result<Self> {
        ensure!(
            checked.request.proposal.judgments.is_empty() && checked.appraisal.is_none(),
            "appraisal requires only original checked observations"
        );
        for (index, unit) in checked.request.proposal.units.iter().enumerate() {
            ensure!(
                unit.id == format!("o{}", index + 1)
                    && unit.kind == UnitKind::RecordedEvent
                    && unit.citations.len() == 1,
                "appraisal requires host-numbered single-citation atomic observations"
            );
            let check = check_for(&checked.reply, &format!("unit:{}", unit.id));
            ensure!(
                check.status != SupportStatus::Supported || check.citations == unit.citations,
                "supported appraisal premise must preserve its original citation"
            );
        }
        let evaluator = checked
            .draft
            .support
            .as_ref()
            .context("missing checks")?
            .evaluator_model
            .clone();
        let rebuilt = finish_support(checked.request, checked.reply, &evaluator)?;
        ensure!(
            serde_json::to_vec(&rebuilt.draft)? == serde_json::to_vec(&checked.draft)?,
            "noncanonical checked observations"
        );
        Ok(Self { checked: rebuilt })
    }
    pub fn prepared(&self) -> &PreparedInterpretation {
        self.checked.request.prepared()
    }
    pub fn proposal(&self) -> &SupportProposal {
        self.checked.proposal()
    }
    pub fn reply(&self) -> &SupportReply {
        self.checked.reply()
    }
    pub fn draft(&self) -> &InterpretationDraft {
        self.checked.draft()
    }
    pub fn support_request(&self) -> &SupportRequest {
        &self.checked.request
    }
    pub(crate) fn provenance(&self, model: &str) -> Result<AtomicProvenance> {
        Ok(AtomicProvenance {
            extraction_binding: atomic_request_binding(
                self.prepared(),
                "atomic-v4-extraction",
                self.prepared().generation_input(),
            )?,
            checking_binding: atomic_request_binding(
                self.prepared(),
                "atomic-v4-checking",
                self.support_request().generation_input(),
            )?,
            appraisal_binding: atomic_request_binding(
                self.prepared(),
                "atomic-v4-appraisal",
                &serde_json::to_string(&(self.proposal(), self.reply()))?,
            )?,
            appraiser_model: model.into(),
        })
    }
    pub fn supported_ids(&self) -> Vec<String> {
        self.checked
            .draft
            .support
            .as_ref()
            .unwrap()
            .units
            .iter()
            .filter_map(|u| match u {
                UnitDisposition::Accepted {
                    unit,
                    status: SupportStatus::Supported,
                } if unit.kind == UnitKind::RecordedEvent => Some(unit.id.clone()),
                _ => None,
            })
            .collect()
    }
}

pub fn finish_appraisal(
    request: AppraisalRequest,
    appraisal: AtomicAppraisal,
    model: &str,
) -> Result<EvaluatedInterpretation> {
    ensure!(identifier(model, 256), "invalid appraiser model");
    validate_atomic_appraisal(&request, &appraisal)?;
    let provenance = request.provenance(model)?;
    let mut checked = request.checked;
    let mut report = *checked
        .draft
        .support
        .clone()
        .context("missing support report")?;
    let bearing = appraisal.state.bearing();
    report.protocol = ATOMIC_SUPPORT_PROTOCOL.into();
    report.atomic = Some(provenance);
    report.judgments = vec![JudgmentDisposition {
        index: 0,
        goal: checked.request.prepared.context.criterion.goal.clone(),
        offered_bearing: bearing,
        bearing,
        because: appraisal.because.clone(),
        status: if bearing == InterpretationBearing::Unknown {
            SupportStatus::Unknown
        } else {
            SupportStatus::Supported
        },
        reason: if bearing == InterpretationBearing::Unknown {
            JudgmentReason::ExplicitUnknown
        } else {
            JudgmentReason::Retained
        },
        unsupported_factual_premise: None,
    }];
    checked.draft = render_report(
        &checked.request.prepared.context,
        &checked.request.prepared.sources,
        report,
    )?;
    checked.appraisal = Some((appraisal, model.into()));
    Ok(checked)
}
pub fn validate_atomic_appraisal(
    request: &AppraisalRequest,
    appraisal: &AtomicAppraisal,
) -> Result<()> {
    let supported = request.supported_ids();
    ensure!(
        appraisal.because.len() <= 8
            && appraisal.because.iter().collect::<BTreeSet<_>>().len() == appraisal.because.len()
            && appraisal.because.iter().all(|id| supported.contains(id)),
        "foreign, duplicate, speculative or rejected appraisal premise"
    );
    ensure!(
        !appraisal.reason.trim().is_empty()
            && appraisal.reason.len() <= 512
            && appraisal.reason.chars().count() <= 128
            && appraisal.questions.len() <= 4
            && appraisal
                .questions
                .iter()
                .all(|q| !q.trim().is_empty() && q.len() <= 256 && q.chars().count() <= 64),
        "invalid appraisal diagnostics"
    );
    if appraisal.state == AtomicCriterionState::Unknown {
        ensure!(
            !appraisal.questions.is_empty(),
            "unknown appraisal needs a question"
        );
    } else {
        ensure!(
            !appraisal.because.is_empty() && appraisal.questions.is_empty(),
            "known appraisal requires supported premises and no questions"
        );
    }
    Ok(())
}

pub(super) fn validate_atomic_report(report: &SupportReport) -> Result<()> {
    let provenance = report
        .atomic
        .as_ref()
        .context("missing derived appraisal provenance")?;
    ensure!(
        digest(&provenance.extraction_binding)
            && digest(&provenance.checking_binding)
            && digest(&provenance.appraisal_binding)
            && identifier(&provenance.appraiser_model, 256)
            && report.judgments.len() == 1,
        "invalid derived appraisal provenance"
    );
    let j = &report.judgments[0];
    ensure!(
        j.offered_bearing == j.bearing
            && j.unsupported_factual_premise.is_none()
            && if j.bearing == InterpretationBearing::Unknown {
                j.status == SupportStatus::Unknown && j.reason == JudgmentReason::ExplicitUnknown
            } else {
                j.status == SupportStatus::Supported
                    && j.reason == JudgmentReason::Retained
                    && !j.because.is_empty()
            },
        "noncanonical derived appraisal judgment"
    );
    ensure!(j.because.iter().all(|id| report.units.iter().any(|u| matches!(u, UnitDisposition::Accepted { unit, status: SupportStatus::Supported } if unit.id == *id && unit.kind == UnitKind::RecordedEvent))), "invalid derived appraisal premises");
    Ok(())
}

/// Core-owned phase binding. The wire may select passages but cannot redefine authority.
pub(crate) fn atomic_request_binding(
    prepared: &PreparedInterpretation,
    phase: &str,
    input: &str,
) -> Result<String> {
    hash(&(
        "mecha-atomic-support-v4",
        &prepared.request_id,
        &prepared.evidence_metadata,
        phase,
        input,
    ))
}
impl Memory {
    /// Reauthorize every consulted source before creating the next inference receipt.
    pub fn prepare_appraisal(
        &self,
        checked: EvaluatedInterpretation,
        access: &Access,
    ) -> Result<AppraisalRequest> {
        self.recheck_support_prepared(&checked.request.prepared, access)?;
        AppraisalRequest::new(checked)
    }
}
