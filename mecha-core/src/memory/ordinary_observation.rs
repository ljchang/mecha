//! Host-recorded ordinary episodes carry observations without a task verdict.
use super::chat_observation::{
    ChatObservationContext, CHAT_OBSERVATION_PROTOCOL, CHAT_RECORDER_VERSION,
    CHAT_TASK_OBSERVATION_PROTOCOL, CHAT_TASK_RECORDER_VERSION, CHAT_WEB_OBSERVATION_PROTOCOL,
    CHAT_WEB_RECORDER_VERSION,
};
use super::host_capture::ConfirmedCriterion;
use super::host_capture::{
    validate, validate_action, ArtifactCheck, CaptureIdentity, CheckVerdict, HostCapture,
    PassageKind, ReceiptStatus,
};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const ORDINARY_OBSERVATION_PROTOCOL: &str = "ordinary-session-observation-v1";
pub const ORDINARY_RECORDER_VERSION: &str = "ordinary-session-recorder-v1";

/// The assessment authorized by a canonical host capture receipt.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticAssessment {
    NotEvaluated,
    ArtifactCheck(ArtifactCheck),
}

/// Ordinary admission takes only this validated, host-owned payload.
pub(crate) struct OrdinaryObservationCapture(HostCapture);

impl OrdinaryObservationCapture {
    pub(crate) fn try_from_host(input: HostCapture) -> Result<Self> {
        validate_ordinary(&input)?;
        Ok(Self(input))
    }

    pub(super) fn into_host(self) -> HostCapture {
        self.0
    }
}

pub(super) fn claims_ordinary_protocol(identity: &CaptureIdentity) -> bool {
    identity.check_version == ORDINARY_OBSERVATION_PROTOCOL
        || identity.evaluator_version == ORDINARY_RECORDER_VERSION
        || identity.check_version == CHAT_OBSERVATION_PROTOCOL
        || identity.evaluator_version == CHAT_RECORDER_VERSION
        || identity.check_version == CHAT_WEB_OBSERVATION_PROTOCOL
        || identity.evaluator_version == CHAT_WEB_RECORDER_VERSION
        || identity.check_version == CHAT_TASK_OBSERVATION_PROTOCOL
        || identity.evaluator_version == CHAT_TASK_RECORDER_VERSION
}

pub(super) fn validate_ordinary(input: &HostCapture) -> Result<()> {
    ensure!(
        (input.identity.check_version == ORDINARY_OBSERVATION_PROTOCOL
            && input.identity.evaluator_version == ORDINARY_RECORDER_VERSION)
            || (input.identity.check_version == CHAT_OBSERVATION_PROTOCOL
                && input.identity.evaluator_version == CHAT_RECORDER_VERSION)
            || (input.identity.check_version == CHAT_WEB_OBSERVATION_PROTOCOL
                && input.identity.evaluator_version == CHAT_WEB_RECORDER_VERSION)
            || (input.identity.check_version == CHAT_TASK_OBSERVATION_PROTOCOL
                && input.identity.evaluator_version == CHAT_TASK_RECORDER_VERSION),
        "invalid ordinary observation protocol"
    );
    ensure!(
        input.artifact.verdict == CheckVerdict::Unavailable
            && input.artifact.artifact_digest.is_none(),
        "ordinary observation cannot supply an artifact check"
    );
    validate(input, super::host_capture::MAX_CONSULTED, 0)?;

    let mut receipts = BTreeMap::new();
    for receipt in &input.receipts {
        ensure!(
            matches!(
                receipt.status,
                ReceiptStatus::Succeeded | ReceiptStatus::Failed | ReceiptStatus::Unavailable
            ) && receipts
                .insert(receipt.call_id.as_str(), receipt.tool_name.as_str())
                .is_none(),
            "invalid ordinary tool receipt"
        );
    }
    for passage in &input.originals {
        match passage.kind {
            PassageKind::ActionInvocation => {
                let action = validate_action(passage)?;
                ensure!(
                    receipts.remove(action.call_id.as_str()) == Some(action.tool_name.as_str()),
                    "ordinary invocation lacks a matching receipt"
                );
            }
            PassageKind::ArtifactObservation => {
                anyhow::bail!("ordinary observation cannot supply an artifact passage");
            }
            PassageKind::UserRequest | PassageKind::AssistantText | PassageKind::ToolResult => {}
        }
    }
    ensure!(receipts.is_empty(), "ordinary receipt lacks an invocation");
    Ok(())
}

/// One context form only; reserved labels cannot be mixed or partially spoofed.
pub(super) fn validate_context(
    identity: &CaptureIdentity,
    criterion: Option<&ConfirmedCriterion>,
    chat: Option<&ChatObservationContext>,
) -> Result<()> {
    if (identity.check_version == CHAT_OBSERVATION_PROTOCOL
        && identity.evaluator_version == CHAT_RECORDER_VERSION)
        || (identity.check_version == CHAT_WEB_OBSERVATION_PROTOCOL
            && identity.evaluator_version == CHAT_WEB_RECORDER_VERSION)
        || (identity.check_version == CHAT_TASK_OBSERVATION_PROTOCOL
            && identity.evaluator_version == CHAT_TASK_RECORDER_VERSION)
    {
        ensure!(
            criterion.is_none(),
            "chat observation cannot supply a confirmed criterion"
        );
        let context = chat.ok_or_else(|| anyhow::anyhow!("chat observation requires context"))?;
        context.validate(identity)?;
    } else {
        ensure!(
            criterion.is_some() && chat.is_none(),
            "legacy capture requires only a confirmed criterion"
        );
        ensure!(
            !claims_ordinary_protocol(identity)
                || (identity.check_version == ORDINARY_OBSERVATION_PROTOCOL
                    && identity.evaluator_version == ORDINARY_RECORDER_VERSION),
            "invalid ordinary observation protocol"
        );
    }
    Ok(())
}
