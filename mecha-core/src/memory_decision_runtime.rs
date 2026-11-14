//! Explicit, bounded model decisions over retained synthetic read observations.
//! The host supplies inference; this layer never discovers a provider or executes a choice.

use crate::memory::{
    decision::{self, Contract, EvidenceView, Prepared, ReadBinding},
    durable::{Phase as ReadPhase, Receipt},
    durable_decision::{
        AttemptData, AttemptKind, AttemptPhase, HostModelResult, ModelGuard, ModelPhase,
        ModelReceipt, ModelStatus,
    },
    model_evaluation::ModelReply,
    Access, Memory,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{future::Future, time::Instant};

const CHECKER_VERSION: &str = "synthetic-measurements-v1";

/// Host-selected task and model settings, sealed before the first inference.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec {
    pub contract: Contract,
    pub model: String,
    pub seed: u64,
    pub view: EvidenceView,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedSpec {
    checker_version: String,
    spec: ModelSpec,
}

/// Bind exactly one model workflow to an already successful guarded read.
/// Validating received synthetic measurements does not certify their real-world truth.
pub fn bind(
    db: &mut Memory,
    read: &Receipt,
    spec: ModelSpec,
    access: &Access,
) -> Result<ModelReceipt> {
    validate_spec(&spec)?;
    let status = db.durable_status(read, access)?;
    ensure!(
        status.phase == ReadPhase::Observed && !status.cancelled,
        "successful active read required"
    );
    let value = db
        .durable_observation(read, access)?
        .context("missing read observation")?;
    ensure!(
        value["kind"] == "host_dispatch_observation"
            && value["observation"]["status"] == "successful",
        "successful host-dispatched observation required"
    );
    let content = value["observation"]["content"]
        .as_str()
        .context("missing observed read bytes")?;
    decision::prepare(
        &spec.contract,
        ReadBinding {
            task: status.task.context("missing read task")?,
            observation: status
                .observation
                .context("missing observation reference")?,
            read_sha256: hash(content.as_bytes()),
        },
        content,
        spec.view,
    )?;
    db.durable_bind_model(
        read,
        serde_json::to_value(SealedSpec {
            checker_version: CHECKER_VERSION.into(),
            spec,
        })?,
        access,
    )
}

/// Read the immutable settings under current access, without provider discovery.
pub fn spec(db: &Memory, receipt: &ModelReceipt, access: &Access) -> Result<ModelSpec> {
    let sealed: SealedSpec = serde_json::from_value(db.durable_model_spec(receipt, access)?)?;
    ensure!(
        sealed.checker_version == CHECKER_VERSION,
        "unsupported decision checker version"
    );
    validate_spec(&sealed.spec)?;
    Ok(sealed.spec)
}
fn validate_spec(spec: &ModelSpec) -> Result<()> {
    ensure!(
        !spec.model.trim().is_empty() && spec.model.len() <= 256,
        "explicit bounded model identifier required"
    );
    Ok(())
}
fn prepare(
    db: &Memory,
    receipt: &ModelReceipt,
    spec: &ModelSpec,
    access: &Access,
) -> Result<Prepared> {
    let read = db.durable_model_read(receipt, access)?;
    decision::prepare(
        &spec.contract,
        ReadBinding {
            task: read.task,
            observation: read.observation,
            read_sha256: read.sha256,
        },
        &read.content,
        spec.view,
    )
}

/// Run to a terminal state or a spent, unresolved attempt. Never replay inference.
pub async fn decide<F, Fut>(
    db: &mut Memory,
    receipt: &ModelReceipt,
    access: &Access,
    mut infer: F,
) -> Result<ModelStatus>
where
    F: FnMut(String, Option<Value>, ModelGuard) -> Fut,
    Fut: Future<Output = Result<ModelReply>>,
{
    // Initial call, assessment, optional retry, assessment, finalization: five steps.
    for _ in 0..5 {
        let status = advance_once(db, receipt, access, &mut infer).await?;
        if status.phase != ModelPhase::Active
            || status.initial.phase == AttemptPhase::Indeterminate
            || status.retry.phase == AttemptPhase::Indeterminate
        {
            return Ok(status);
        }
    }
    db.durable_model_status(receipt, access)
}

/// Perform one resumable step: inference+raw settlement, pure assessment, or finalization.
/// A caller can stop between steps; only settled responses can continue after restart.
pub async fn advance<F, Fut>(
    db: &mut Memory,
    receipt: &ModelReceipt,
    access: &Access,
    mut infer: F,
) -> Result<ModelStatus>
where
    F: FnMut(String, Option<Value>, ModelGuard) -> Fut,
    Fut: Future<Output = Result<ModelReply>>,
{
    advance_once(db, receipt, access, &mut infer).await
}

async fn advance_once<F, Fut>(
    db: &mut Memory,
    receipt: &ModelReceipt,
    access: &Access,
    infer: &mut F,
) -> Result<ModelStatus>
where
    F: FnMut(String, Option<Value>, ModelGuard) -> Fut,
    Fut: Future<Output = Result<ModelReply>>,
{
    let status = db.durable_model_status(receipt, access)?;
    match status.phase {
        ModelPhase::Forgotten | ModelPhase::Cancelled => return Ok(status),
        ModelPhase::Finalized => {
            db.durable_model_decision(receipt, access)?
                .context("missing final decision")?;
            return Ok(status);
        }
        ModelPhase::Active => {}
    }
    // Both reads recheck source closures; reservation/settlement/finalization recheck atomically.
    let spec = spec(db, receipt, access)?;
    let prepared = prepare(db, receipt, &spec, access)?;
    let initial = db.durable_model_attempt(receipt, AttemptKind::Initial, access)?;
    let (kind, phase, request) = match status.initial.phase {
        AttemptPhase::Unused => (
            AttemptKind::Initial,
            AttemptPhase::Unused,
            prepared.request().clone(),
        ),
        AttemptPhase::Indeterminate => return Ok(status),
        AttemptPhase::Settled => (
            AttemptKind::Initial,
            AttemptPhase::Settled,
            prepared.request().clone(),
        ),
        AttemptPhase::Assessed => {
            let attempt = initial.as_ref().context("missing initial attempt")?;
            verify_request(attempt, &request_record(&spec, prepared.request()))?;
            let checked: Checked = serde_json::from_value(
                attempt
                    .assessment
                    .clone()
                    .context("missing initial assessment")?,
            )?;
            if !checked.retry_requested {
                return finalize(
                    db,
                    receipt,
                    access,
                    &prepared,
                    &spec,
                    attempt,
                    AttemptKind::Initial,
                );
            }
            let text = response_text(attempt, &spec.model)?;
            let request = prepared
                .reconsideration(&text)?
                .context("stored retry finding no longer reproducible")?;
            (AttemptKind::Retry, status.retry.phase, request)
        }
    };
    let exact = request_record(&spec, &request);
    match phase {
        AttemptPhase::Unused => {
            let permit = db.reserve_model_attempt(receipt, kind, exact, access)?;
            let guard = db.model_attempt_guard(&permit, access)?;
            let start = Instant::now();
            let result = match guard.check() {
                Ok(()) => infer(request.prompt, Some(request.response_schema), guard).await,
                Err(e) => Err(e),
            };
            let call = bounded_call(result, start.elapsed().as_millis());
            db.settle_model_attempt(
                permit,
                HostModelResult::new(serde_json::to_value(call)?),
                access,
            )
        }
        AttemptPhase::Indeterminate => Ok(status),
        AttemptPhase::Settled => {
            let attempt = db
                .durable_model_attempt(receipt, kind, access)?
                .context("missing settled attempt")?;
            verify_request(&attempt, &exact)?;
            let result =
                response_text(&attempt, &spec.model).and_then(|text| prepared.assess(&text));
            let checked = match result {
                Ok(assessment) => Checked {
                    valid: true,
                    error: None,
                    retry_requested: kind == AttemptKind::Initial
                        && !assessment.findings.is_empty(),
                    evaluation: Some(serde_json::to_value(assessment)?),
                },
                Err(e) => Checked {
                    valid: false,
                    error: Some(bounded_error(&e)),
                    retry_requested: false,
                    evaluation: None,
                },
            };
            db.assess_model_attempt(
                receipt,
                kind,
                serde_json::to_value(&checked)?,
                checked.retry_requested,
                access,
            )
        }
        AttemptPhase::Assessed => {
            let attempt = db
                .durable_model_attempt(receipt, kind, access)?
                .context("missing assessed attempt")?;
            verify_request(&attempt, &exact)?;
            finalize(db, receipt, access, &prepared, &spec, &attempt, kind)
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checked {
    valid: bool,
    error: Option<String>,
    evaluation: Option<Value>,
    retry_requested: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReply {
    text: String,
    model: String,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    error: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCall {
    reply: Option<StoredReply>,
    error: Option<String>,
    millis: u128,
    discarded: Option<Value>,
}
fn bounded_call(result: Result<ModelReply>, millis: u128) -> StoredCall {
    let mut call = match result {
        Ok(r) => StoredCall {
            reply: Some(StoredReply {
                text: r.text,
                model: r.model,
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                error: r.error,
            }),
            error: None,
            millis,
            discarded: None,
        },
        Err(e) => StoredCall {
            reply: None,
            error: Some(bounded_error(&e)),
            millis,
            discarded: None,
        },
    };
    let bytes = serde_json::to_vec(&call).expect("primitive call fields serialize");
    if bytes.len() > 64_000 {
        let reply = call.reply.take();
        call.discarded = Some(
            json!({"sha256":hash(&bytes),"bytes":bytes.len(),"input_tokens":reply.as_ref().and_then(|r|r.input_tokens),"output_tokens":reply.as_ref().and_then(|r|r.output_tokens)}),
        );
        call.error = Some("Oversized model response omitted; attempt failed.".into());
    }
    call
}
fn response_text(attempt: &AttemptData, model: &str) -> Result<String> {
    let call: StoredCall =
        serde_json::from_value(attempt.result.clone().context("missing raw result")?)?;
    ensure!(call.error.is_none(), "inference failed: {:?}", call.error);
    let reply = call.reply.context("missing model response")?;
    ensure!(
        reply.model == model,
        "response model differs from sealed model"
    );
    ensure!(
        reply.error.is_none(),
        "generation failed: {:?}",
        reply.error
    );
    ensure!(!reply.text.trim().is_empty(), "empty model response");
    Ok(reply.text)
}
fn request_record(spec: &ModelSpec, request: &crate::memory::experience::Request) -> Value {
    json!({"checker_version":CHECKER_VERSION,"model":spec.model,"seed":spec.seed,"prompt":request.prompt,"response_schema":request.response_schema})
}
fn verify_request(attempt: &AttemptData, expected: &Value) -> Result<()> {
    ensure!(
        &attempt.request == expected,
        "stored request differs from sealed evidence/contract"
    );
    Ok(())
}
fn finalize(
    db: &mut Memory,
    receipt: &ModelReceipt,
    access: &Access,
    prepared: &Prepared,
    spec: &ModelSpec,
    attempt: &AttemptData,
    kind: AttemptKind,
) -> Result<ModelStatus> {
    let checked: Checked = serde_json::from_value(
        attempt
            .assessment
            .clone()
            .context("missing host assessment")?,
    )?;
    let decision = if checked.valid {
        let text = response_text(attempt, &spec.model)?;
        let current = prepared.assess(&text)?;
        ensure!(
            checked.evaluation == Some(serde_json::to_value(current)?),
            "stored assessment differs from deterministic check"
        );
        serde_json::to_value(prepared.finish(&text)?)?
    } else {
        // An invalid response must still fail on deterministic replay; a stored label alone is insufficient.
        let failure = response_text(attempt, &spec.model).and_then(|text| prepared.assess(&text));
        ensure!(failure.is_err(), "stored failure no longer reproducible");
        json!({"disposition":"failed","error":checked.error,"executed":false})
    };
    let mut decision = decision;
    decorate_final(&mut decision, kind)?;
    db.finalize_model(receipt, decision, access)
}
/// Internal authenticated handoff to the separate host action runtime. Stored
/// advisory JSON alone cannot supply this authority.
pub(crate) struct AuthenticatedRecommendation {
    pub contract: Contract,
    pub choice: String,
    pub expected_from_evidence: Option<Value>,
}
pub(crate) fn authenticated_recommendation(
    db: &Memory,
    receipt: &ModelReceipt,
    access: &Access,
) -> Result<AuthenticatedRecommendation> {
    let status = db.durable_model_status(receipt, access)?;
    ensure!(
        status.phase == ModelPhase::Finalized && !status.cancelled,
        "active finalized model workflow required"
    );
    let spec = spec(db, receipt, access)?;
    let prepared = prepare(db, receipt, &spec, access)?;
    ensure!(
        status.initial.phase == AttemptPhase::Assessed,
        "initial attempt must have a retained assessment"
    );
    let initial = db
        .durable_model_attempt(receipt, AttemptKind::Initial, access)?
        .context("missing initial attempt")?;
    verify_request(&initial, &request_record(&spec, prepared.request()))?;
    let text = response_text(&initial, &spec.model)?;
    let assessment = prepared.assess(&text)?;
    let retry_requested = !assessment.findings.is_empty();
    verify_action_assessment(&initial, &assessment, retry_requested)?;
    let (kind, text) = if retry_requested {
        ensure!(
            status.retry.phase == AttemptPhase::Assessed,
            "requested retry must have a retained assessment"
        );
        let request = prepared
            .reconsideration(&text)?
            .context("retry finding no longer reproducible")?;
        let retry = db
            .durable_model_attempt(receipt, AttemptKind::Retry, access)?
            .context("missing retry attempt")?;
        verify_request(&retry, &request_record(&spec, &request))?;
        let text = response_text(&retry, &spec.model)?;
        let assessment = prepared.assess(&text)?;
        verify_action_assessment(&retry, &assessment, false)?;
        (AttemptKind::Retry, text)
    } else {
        ensure!(
            status.retry.phase == AttemptPhase::Unused,
            "unexpected retry attempt"
        );
        (AttemptKind::Initial, text)
    };
    let decision = prepared.finish(&text)?;
    ensure!(
        decision.disposition == decision::Disposition::Recommend,
        "only an authenticated recommendation can bind an action"
    );
    let choice = decision.proposed_choice.clone();
    let mut expected = serde_json::to_value(decision)?;
    decorate_final(&mut expected, kind)?;
    ensure!(
        db.durable_model_decision(receipt, access)? == Some(expected),
        "final advisory record differs from retained response and sealed evidence"
    );
    let expected_from_evidence = prepared
        .request()
        .passages
        .iter()
        .filter_map(|p| p.projection.as_ref())
        .map(|p| serde_json::from_str::<Value>(&p.text))
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .find_map(|p| {
            let m = &p["outcome"];
            (m["choice"] == choice).then(|| {
                json!({
                "observed_minutes":m["observed_minutes"],
                "energy":m["energy"],"quality":m["quality"]})
            })
        });
    Ok(AuthenticatedRecommendation {
        contract: spec.contract,
        choice,
        expected_from_evidence,
    })
}
fn verify_action_assessment(
    attempt: &AttemptData,
    assessment: &crate::memory::experience::Assessment,
    retry_requested: bool,
) -> Result<()> {
    let expected = Checked {
        valid: true,
        error: None,
        evaluation: Some(serde_json::to_value(assessment)?),
        retry_requested,
    };
    ensure!(
        attempt.assessment.as_ref() == Some(&serde_json::to_value(expected)?),
        "stored assessment differs from deterministic check"
    );
    Ok(())
}
fn decorate_final(decision: &mut Value, kind: AttemptKind) -> Result<()> {
    decision["attempt_kind"] = serde_json::to_value(kind)?;
    decision["checker_version"] = json!(CHECKER_VERSION);
    decision["executed"] = json!(false);
    decision["private_data"] = json!(true);
    decision["untrusted_input"] = json!(true);
    Ok(())
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn bounded_error(e: &anyhow::Error) -> String {
    format!("{e:#}").chars().take(2000).collect()
}

#[cfg(test)]
#[path = "memory_decision_runtime_tests.rs"]
mod tests;
