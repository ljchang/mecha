//! Native paired atomic-evidence comparison; synthetic and explicit only.
//! One authenticated source preparation feeds both arms. Direct answers are not
//! retained or described as independently checked facts.
use crate::{
    memory::interpretation::InterpretationContext,
    memory_bridge_trial::recorded_provider,
    memory_factual_support::{GenerationBudget, PassOutcome},
    memory_situated_needs::new_context_id,
    memory_support_atomic as atomic,
    memory_support_workflow::{
        checkpoint, criterion, descriptor, digest, packet_bytes, prepare_native, read_case,
        read_memory, write_audit_exports, write_value, PreparedNative, StageProviders, TrialCase,
    },
    message::CompletionRequest,
    provider::Provider,
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub const PROTOCOL: &str = "memory-atomic-comparison-v4";
const ARM_TOKENS: u32 = 6144;
const ARM_SECONDS: u64 = 540;

/// Probe labels are accepted for compatibility with frozen native cases, but are
/// never read, exported as evidence, evaluated, or included in policy inference.
pub fn load_case(path: &Path) -> Result<TrialCase> {
    let case = read_case(path)?;
    case.validate_native()?;
    Ok(case)
}

fn stage() -> Value {
    json!({"status":"pending","error":null})
}
fn arm(pipeline: bool) -> Value {
    let mut stages = serde_json::Map::new();
    for name in if pipeline {
        &[
            "generate",
            "check",
            "appraise",
            "retain",
            "stored_export",
            "recall",
            "changed_goal",
            "historical",
        ][..]
    } else {
        &["direct"][..]
    } {
        stages.insert((*name).into(), stage());
    }
    json!({"status":"pending","error":null,"stages":stages,"artifacts":{},
        "output_token_allowance":ARM_TOKENS,"deadline_seconds":ARM_SECONDS,
        "independently_checked_retention":false})
}
fn activate(arm: &mut Value, name: &str) {
    arm["active_stage"] = json!(name);
}
fn completed(arm: &mut Value, name: &str) {
    arm["stages"][name]["status"] = json!("completed");
}
fn artifact(
    root: &Path,
    arm: &mut Value,
    key: &str,
    file: &str,
    value: &impl serde::Serialize,
) -> Result<()> {
    let path = root.join(file);
    write_value(&path, value)?;
    arm["artifacts"][key] = descriptor(root, &path)?;
    Ok(())
}
fn remaining(started: Instant, tokens: u32) -> Result<GenerationBudget> {
    let left = Duration::from_secs(ARM_SECONDS)
        .checked_sub(started.elapsed())
        .context("arm deadline exhausted")?;
    GenerationBudget::new(tokens, left)
}
async fn provider(
    root: &Path,
    name: &str,
    providers: &dyn StageProviders,
    cancel: &CancellationToken,
    started: Instant,
    tokens: u32,
) -> Result<Box<dyn Provider>> {
    let budget = remaining(started, tokens)?;
    let inner = tokio::select! { biased;
        _ = cancel.cancelled() => anyhow::bail!("arm cancelled"),
        value = tokio::time::timeout(budget.deadline(), providers.create(name, cancel)) => {
            value.context("arm deadline during provider setup")??
        }
    };
    recorded_provider(
        inner,
        &root.join(format!("{name}-provider.jsonl")),
        u64::from(tokens),
        cancel.clone(),
    )
}
fn planned(arm: &mut Value, name: &str, request: &CompletionRequest) {
    arm["stages"][name]["packet_bytes"] = json!(packet_bytes(request));
    arm["stages"][name]["max_output_tokens"] = json!(request.max_tokens);
}
fn finish_pass<T>(
    root: &Path,
    arm: &mut Value,
    name: &str,
    audit_name: &str,
    outcome: PassOutcome<T>,
) -> Result<T> {
    let mut export = json!({"stages":{audit_name:arm["stages"][name].clone()}});
    write_audit_exports(root, &mut export, audit_name, &outcome)?;
    arm["stages"][name] = export["stages"][audit_name].take();
    if let Some(usage) = &outcome.usage {
        ensure!(
            usage.output_tokens
                <= arm["stages"][name]["max_output_tokens"]
                    .as_u64()
                    .unwrap_or(0),
            "provider exceeded stage output allowance"
        );
    }
    let result = outcome
        .result
        .map_err(|e| anyhow::anyhow!("{audit_name} refused: {e:?}"))?;
    completed(arm, name);
    Ok(result)
}
fn finish_arm(arm: &mut Value, outcome: Result<()>, started: Instant, cancel: &CancellationToken) {
    match outcome {
        Ok(()) => arm["status"] = json!("completed"),
        Err(error) => {
            let active = arm["active_stage"].as_str().unwrap_or("direct").to_string();
            arm["stages"][&active]["status"] = json!("failed");
            arm["stages"][&active]["error"] = json!(format!("{error:#}"));
            arm["status"] = json!("failed");
            arm["error"] = json!(format!("{error:#}"));
        }
    }
    for stage in arm["stages"]
        .as_object_mut()
        .expect("arm stages")
        .values_mut()
    {
        if stage["status"] == "pending" {
            stage["status"] = json!("withheld");
            stage["error"] = json!("upstream stage unavailable");
        }
    }
    let calls: Vec<_> = arm["stages"]
        .as_object()
        .expect("arm stages")
        .values()
        .filter(|s| s["provider_audit"].is_object())
        .collect();
    let started_calls: Vec<_> = calls.iter().filter(|s| s["request"].is_object()).collect();
    let count = started_calls.len();
    let packet_bytes: u64 = started_calls
        .iter()
        .filter_map(|s| s["packet_bytes"].as_u64())
        .sum();
    let mut totals = vec![];
    for key in ["input_tokens", "output_tokens"] {
        let values: Vec<_> = started_calls.iter().map(|s| s[key].as_u64()).collect();
        let known: u64 = values.iter().flatten().sum();
        let complete = values.iter().all(Option::is_some);
        totals.push((key, known, complete));
    }
    arm["policy_calls_started"] = json!(count);
    arm["input_packet_bytes"] = json!(packet_bytes);
    for (key, known, complete) in totals {
        arm[format!("observed_{key}")] = json!(known);
        arm[key] = if complete { json!(known) } else { Value::Null };
    }
    arm["elapsed_ms"] = json!(started.elapsed().as_millis());
    arm["cancelled"] = json!(cancel.is_cancelled());
    arm.as_object_mut()
        .expect("arm object")
        .remove("active_stage");
}

#[derive(Clone, Copy)]
struct ArmRun<'a> {
    root: &'a Path,
    model: &'a str,
    providers: &'a dyn StageProviders,
    cancel: &'a CancellationToken,
    started: Instant,
}
async fn pipeline(
    run: ArmRun<'_>,
    prepared: &mut PreparedNative,
    case: &TrialCase,
    arm: &mut Value,
) -> Result<()> {
    let ArmRun {
        root,
        model,
        providers,
        cancel,
        started,
    } = run;
    activate(arm, "generate");
    let budget = remaining(started, 1536)?;
    let request = atomic::generation_request(&prepared.prepared, model, budget)?;
    planned(arm, "generate", &request);
    let p = provider(root, "pipeline-generate", providers, cancel, started, 1536).await?;
    let outcome = atomic::generate(
        p.as_ref(),
        model,
        &prepared.prepared,
        remaining(started, 1536)?,
        cancel,
    )
    .await;
    let served = outcome.served_model.clone();
    let proposal = finish_pass(root, arm, "generate", "pipeline-generate", outcome)?;
    artifact(
        root,
        arm,
        "numbered_observations",
        "numbered-observations.json",
        &proposal,
    )?;

    activate(arm, "check");
    let request = prepared.memory.prepare_support_with_model(
        &prepared.prepared,
        proposal,
        &served.context("generation model unavailable")?,
        &prepared.source.access,
    )?;
    artifact(
        root,
        arm,
        "checking_binding_input",
        "checking-binding-input.json",
        &request.generation_input(),
    )?;
    let budget = remaining(started, 3072)?;
    planned(
        arm,
        "check",
        &atomic::evaluation_request(&request, model, budget)?,
    );
    let p = provider(root, "pipeline-check", providers, cancel, started, 3072).await?;
    let outcome = atomic::evaluate(
        p.as_ref(),
        model,
        request,
        remaining(started, 3072)?,
        cancel,
    )
    .await;
    let checked = finish_pass(root, arm, "check", "pipeline-check", outcome)?;
    artifact(
        root,
        arm,
        "fact_checks",
        "fact-checks.json",
        checked.reply(),
    )?;

    activate(arm, "appraise");
    let request = prepared
        .memory
        .prepare_appraisal(checked, &prepared.source.access)?;
    let appraisal_binding = serde_json::to_string(&(request.proposal(), request.reply()))?;
    artifact(
        root,
        arm,
        "appraisal_binding_input",
        "appraisal-binding-input.json",
        &appraisal_binding,
    )?;
    let budget = remaining(started, 1536)?;
    planned(
        arm,
        "appraise",
        &atomic::appraisal_request(&request, model, budget)?,
    );
    let p = provider(root, "pipeline-appraise", providers, cancel, started, 1536).await?;
    let outcome = atomic::appraise(
        p.as_ref(),
        model,
        request,
        remaining(started, 1536)?,
        cancel,
    )
    .await;
    let raw = outcome.raw_reply.clone();
    let appraised = finish_pass(root, arm, "appraise", "pipeline-appraise", outcome)?;
    let appraisal: Value = serde_json::from_str(&raw.context("appraisal reply unavailable")?)?;
    artifact(root, arm, "appraisal", "appraisal.json", &appraisal)?;

    activate(arm, "retain");
    ensure!(!cancel.is_cancelled(), "cancelled before retention");
    remaining(started, 1)?;
    let receipt = prepared.memory.retain_evaluated_interpretation(
        prepared.prepared.clone(),
        appraised,
        &prepared.source.access,
    )?;
    // A successful write remains successful if export or bounded recall fails.
    completed(arm, "retain");
    activate(arm, "stored_export");
    artifact(root, arm, "receipt", "pipeline-receipt.json", &receipt)?;
    // Reopen and authenticate the full stored record and its complete lineage.
    // This host audit is independent of contextual selection and packet budgets.
    let reopened = prepared.source.memory()?;
    let record =
        reopened.stored_interpretation(&receipt, &prepared.source.access, &prepared.current)?;
    // Memory uses DELETE journaling and completed transactions; no WAL sidecar
    // is needed for this immutable audit snapshot.
    let database = root.join("retained.memory-prototype.sqlite");
    fs::copy(&prepared.source.database, &database)?;
    arm["artifacts"]["database"] = descriptor(root, &database)?;
    artifact(root, arm, "retained", "retained-pipeline.json", &record)?;
    arm["independently_checked_retention"] = json!(true);
    completed(arm, "stored_export");

    activate(arm, "recall");
    let recall = read_memory(&prepared.source, &prepared.current, true).await?;
    artifact(root, arm, "recall", "recall.json", &recall)?;
    let selected = recall["memory"]["interpretation_roles"]
        .as_array()
        .is_some_and(|roles| {
            roles.iter().any(|r| {
                r["reference"] == json!(receipt.reference) && r["role"] == "SelectedCurrent"
            })
        });
    arm["stages"]["recall"]["packet_bytes"] = json!(serde_json::to_vec(&recall["memory"])?.len());
    arm["stages"]["recall"]["selected_current"] = json!(selected);
    arm["stages"]["recall"]["budget_omitted"] = recall["memory"]["budget_omitted"].clone();
    ensure!(
        selected,
        "ordinary GoalMemory did not select the retained interpretation"
    );
    completed(arm, "recall");

    activate(arm, "historical");
    let historical = InterpretationContext::new(criterion(&case.native, false), new_context_id())?;
    let packet = read_memory(&prepared.source, &historical, true).await?;
    ensure!(
        !has_current(&packet),
        "current interpretation promoted into historical context"
    );
    artifact(root, arm, "historical", "historical-control.json", &packet)?;
    completed(arm, "historical");

    activate(arm, "changed_goal");
    let mut followup = prepared.current.criterion().clone();
    followup.goal = "task:synthetic-followup".into();
    followup.goal_text = "Verify a prior artifact for a distinct synthetic follow-up.".into();
    followup.criterion_text =
        "Request fresh verification before relying on any historical artifact.".into();
    followup.criterion_revision = digest(followup.criterion_text.as_bytes());
    followup.confirmation_digest = digest(b"owner-confirmed-synthetic-followup");
    let changed = InterpretationContext::new(followup, new_context_id())?;
    let packet = read_memory(&prepared.source, &changed, true).await?;
    ensure!(
        !has_current(&packet),
        "historical interpretation promoted across changed goal"
    );
    artifact(root, arm, "changed_goal", "changed-goal.json", &packet)?;
    arm["stages"]["changed_goal"]["withheld_previous_current"] = json!(true);
    completed(arm, "changed_goal");
    ensure!(!cancel.is_cancelled(), "arm cancelled before completion");
    remaining(started, 1)?;
    Ok(())
}
fn has_current(packet: &Value) -> bool {
    packet["memory"]["interpretation_roles"]
        .as_array()
        .is_some_and(|roles| roles.iter().any(|r| r["role"] == "SelectedCurrent"))
}
async fn direct(run: ArmRun<'_>, prepared: &PreparedNative, arm: &mut Value) -> Result<()> {
    let ArmRun {
        root,
        model,
        providers,
        cancel,
        started,
    } = run;
    activate(arm, "direct");
    let budget = remaining(started, ARM_TOKENS)?;
    planned(
        arm,
        "direct",
        &atomic::direct_request(&prepared.prepared, model, budget)?,
    );
    let p = provider(root, "direct", providers, cancel, started, ARM_TOKENS).await?;
    let outcome = atomic::direct(
        p.as_ref(),
        model,
        &prepared.prepared,
        remaining(started, ARM_TOKENS)?,
        cancel,
    )
    .await;
    let result = finish_pass(root, arm, "direct", "direct", outcome)?;
    artifact(root, arm, "result", "direct-result.json", &result)?;
    ensure!(!cancel.is_cancelled(), "arm cancelled before completion");
    remaining(started, 1)?;
    Ok(())
}

/// Alternate order from the caller's frozen case index, never from model output.
/// Structural input errors return Err; execution failures remain report data.
pub async fn run_case(
    root: &Path,
    case: &TrialCase,
    model: &str,
    providers: &dyn StageProviders,
    cancel: &CancellationToken,
    case_index: usize,
) -> Result<Value> {
    case.validate_native()?;
    ensure!(
        root.is_absolute() && !root.exists(),
        "new absolute comparison root required"
    );
    fs::create_dir(root)?;
    let root = root.canonicalize()?;
    let order = if case_index.is_multiple_of(2) {
        ["pipeline", "direct"]
    } else {
        ["direct", "pipeline"]
    };
    let started = Instant::now();
    let mut report = json!({"protocol":PROTOCOL,"case_id":case.case_id,"case_sha256":case.case_sha256,
        "model":model,"seed":70510,"case_index":case_index,"arm_order":order,"status":"failed",
        "error":null,"stages":{"source":stage(),"capture":stage()},"artifacts":{},
        "arms":{"pipeline":arm(true),"direct":arm(false)},"diagnostic_policy_calls":0});
    checkpoint(&root, &report)?;
    let outcome = prepare_native(&root, case, cancel, &mut report, true).await;
    match outcome {
        Err(error) => {
            let active = report["active_stage"]
                .as_str()
                .unwrap_or("source")
                .to_string();
            report["stages"][&active]["status"] = json!("failed");
            report["stages"][&active]["error"] = json!(format!("{error:#}"));
            report["error"] = json!(format!("{error:#}"));
        }
        Ok(mut prepared) => {
            let initial = json!({"sources":prepared.prepared.sources(),
                "evidence_metadata":prepared.prepared.evidence_metadata(),"criterion":prepared.current.criterion()});
            let initial_path = root.join("initial-evidence.json");
            write_value(&initial_path, &initial)?;
            report["initial_evidence"] = descriptor(&root, &initial_path)?;
            report["initial_evidence_sha256"] = report["initial_evidence"]["sha256"].clone();
            // Host audit only: opaque random identity and exact canonical bytes
            // let an independent reader reproduce phase bindings. None of these
            // audit fields is added to a model packet.
            let binding_path = root.join("binding-context.json");
            write_value(
                &binding_path,
                &json!({
                    "prepared_request_id":prepared.prepared.request_identity(),
                    "generation_input":prepared.prepared.generation_input(),
                    "evidence_metadata_json":serde_json::to_string(prepared.prepared.evidence_metadata())?,
                }),
            )?;
            report["binding_context"] = descriptor(&root, &binding_path)?;
            for name in order {
                report["arms"][name]["initial_evidence_sha256"] =
                    report["initial_evidence"]["sha256"].clone();
                report["arms"][name]["model"] = json!(model);
                report["arms"][name]["seed"] = json!(70510);
            }
            checkpoint(&root, &report)?;
            for name in order {
                let arm_started = Instant::now();
                let arm_cancel = cancel.child_token();
                let timer_cancel = arm_cancel.clone();
                let timer = tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(ARM_SECONDS)).await;
                    timer_cancel.cancel();
                });
                let run = ArmRun {
                    root: &root,
                    model,
                    providers,
                    cancel: &arm_cancel,
                    started: arm_started,
                };
                let result = if name == "pipeline" {
                    pipeline(run, &mut prepared, case, &mut report["arms"][name]).await
                } else {
                    direct(run, &prepared, &mut report["arms"][name]).await
                };
                timer.abort();
                finish_arm(&mut report["arms"][name], result, arm_started, &arm_cancel);
                checkpoint(&root, &report)?;
            }
            let intact = descriptor(&root, &prepared.source_control)?
                == report["source_only_database"]
                && descriptor(&root, &prepared.source.input)?["sha256"]
                    == report["initial_hashes"]["initial_input"]
                && descriptor(&root, &prepared.source.transcript)?["sha256"]
                    == report["initial_hashes"]["native_transcript"]
                && match &case.native.observed_output {
                    Some(_) => {
                        descriptor(&root, &prepared.source.artifact)?["sha256"]
                            == report["initial_hashes"]["observed_output"]
                    }
                    None => !prepared.source.artifact.exists(),
                };
            report["source_integrity_verified"] = json!(intact);
            if !intact {
                report["error"] =
                    json!("source evidence or source-only control changed after capture");
            }
            if intact
                && order
                    .iter()
                    .all(|n| report["arms"][n]["status"] == "completed")
                && !cancel.is_cancelled()
            {
                report["status"] = json!("completed");
            }
        }
    }
    for stage in report["stages"]
        .as_object_mut()
        .expect("source stages")
        .values_mut()
    {
        if stage["status"] == "pending" {
            stage["status"] = json!("withheld");
            stage["error"] = json!("upstream source stage unavailable");
        }
    }
    for arm_name in order {
        if report["arms"][arm_name]["status"] == "pending" {
            report["arms"][arm_name]["status"] = json!("withheld");
            report["arms"][arm_name]["error"] = json!("source capture unavailable");
            for stage in report["arms"][arm_name]["stages"]
                .as_object_mut()
                .expect("arm stages")
                .values_mut()
            {
                stage["status"] = json!("withheld");
                stage["error"] = json!("source capture unavailable");
            }
        }
    }
    report
        .as_object_mut()
        .expect("report object")
        .remove("active_stage");
    report["elapsed_ms"] = json!(started.elapsed().as_millis());
    report["cancelled"] = json!(cancel.is_cancelled());
    checkpoint(&root, &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        memory_support_workflow::{NativeCase, ProbeCase},
        provider::Provider,
    };
    struct Refuse;
    #[async_trait::async_trait]
    impl StageProviders for Refuse {
        async fn create(&self, _: &str, _: &CancellationToken) -> Result<Box<dyn Provider>> {
            anyhow::bail!("authored setup failure")
        }
    }
    fn case() -> TrialCase {
        TrialCase {
            version: 1,
            case_id: "atomic-authored".into(),
            case_sha256: "a".repeat(64),
            native: NativeCase {
                input_text: "volume,page,approved\na3,14,yes\na3,15,no\n".into(),
                requested_output: r#"[{"volume":"a3","page":"14"}]"#.into(),
                observed_output: Some(r#"[{"volume":"a3","page":"15"}]"#.into()),
                historical_criterion: "Include exactly approved=yes pages.".into(),
                current_criterion: "Include exactly approved=yes pages.".into(),
                goal_text: "Preserve requested archive pages.".into(),
            },
            probe: ProbeCase {
                units: vec![],
                judgments: vec![],
            },
        }
    }
    #[tokio::test]
    async fn failed_provider_preserves_both_arms_and_shared_source_evidence() {
        let temporary = std::env::temp_dir().join(format!("atomic-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temporary).unwrap();
        let root = temporary.canonicalize().unwrap().join("run");
        let report = run_case(
            &root,
            &case(),
            "authored-atomic",
            &Refuse,
            &CancellationToken::new(),
            1,
        )
        .await
        .unwrap();
        assert_eq!(report["status"], "failed");
        assert_eq!(report["arm_order"], json!(["direct", "pipeline"]));
        assert_eq!(report["arms"]["direct"]["status"], "failed");
        assert_eq!(report["arms"]["pipeline"]["status"], "failed");
        assert!(root.join("source-capture.json").is_file());
        assert!(root.join("initial-evidence.json").is_file());
        assert_eq!(
            report["arms"]["direct"]["initial_evidence_sha256"],
            report["initial_evidence"]["sha256"]
        );
        assert_eq!(
            report["arms"]["pipeline"]["initial_evidence_sha256"],
            report["initial_evidence"]["sha256"]
        );
        fs::remove_dir_all(temporary).unwrap();
    }
    #[derive(Clone, Copy)]
    enum Mode {
        Good,
        Malformed,
        Empty,
        Cancel,
        Truncated,
    }
    struct Authored {
        mode: Mode,
        blocked_export: Option<std::path::PathBuf>,
        crowd_database: Option<std::path::PathBuf>,
        seen: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    }
    struct Reply {
        stage: String,
        mode: Mode,
        cancel: CancellationToken,
        seen: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    }
    #[async_trait::async_trait]
    impl StageProviders for Authored {
        async fn create(
            &self,
            stage: &str,
            cancel: &CancellationToken,
        ) -> Result<Box<dyn Provider>> {
            if stage == "pipeline-appraise" {
                if let Some(path) = &self.blocked_export {
                    fs::create_dir(path)?;
                }
                if let Some(path) = &self.crowd_database {
                    let mut memory = crate::memory::Memory::open(path)?;
                    for index in 0..8 {
                        memory.capture(crate::memory::Capture {
                            source: crate::memory::SourceKey {
                                connector: "authored".into(),
                                account: "synthetic".into(),
                                id: format!("crowded-output-{index}"),
                            },
                            text: "output ".repeat(500),
                            occurred_at: None,
                            scopes: ["synthetic:support".into()].into(),
                            transcript: None,
                        })?;
                    }
                }
            }
            Ok(Box::new(Reply {
                stage: stage.into(),
                mode: self.mode,
                cancel: cancel.clone(),
                seen: self.seen.clone(),
            }))
        }
    }
    #[async_trait::async_trait]
    impl Provider for Reply {
        fn id(&self) -> &str {
            "authored-atomic"
        }
        fn default_model(&self) -> &str {
            "authored-atomic"
        }
        fn local(&self) -> bool {
            true
        }
        fn structured_output(&self) -> bool {
            true
        }
        async fn complete(
            &self,
            request: &CompletionRequest,
            _: Option<&crate::provider::StreamSink>,
        ) -> Result<crate::message::CompletionResponse> {
            let input = request.messages[0].text();
            let packet: Value = serde_json::from_str(
                input
                    .strip_prefix("<support-input>\n")
                    .unwrap()
                    .strip_suffix("\n</support-input>")
                    .unwrap(),
            )?;
            self.seen
                .lock()
                .unwrap()
                .push((self.stage.clone(), packet.clone()));
            let failed = self.stage == "pipeline-generate";
            if failed && matches!(self.mode, Mode::Cancel) {
                self.cancel.cancel();
                std::future::pending::<()>().await;
            }
            let observation = || {
                let handle = packet["catalog"]["citations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|entry| entry["kind"] == "ArtifactObservation")
                    .unwrap()["handle"]
                    .clone();
                json!({"text":"The later artifact was observed by the host.","evidence":handle})
            };
            let reply = match self.stage.as_str() {
                "pipeline-generate" => {
                    json!({"request_key":packet["request_key"],"observations":[observation()]})
                }
                "pipeline-check" => json!({"request_key":packet["request_key"],"checks":[{
                    "target":"unit:o1","status":"Supported","original_citation":"DirectSupport",
                    "reason":"Authored protocol fixture.","questions":[]}]}),
                "pipeline-appraise" => {
                    json!({"request_key":packet["request_key"],"state":"not_met","because":["o1"],"reason":"Authored current criterion fixture.","questions":[]})
                }
                "direct" => {
                    json!({"request_key":packet["request_key"],"observations":[observation()],
                    "criterion":{"state":"not_met","because":[0],"reason":"Authored direct fixture.","questions":[]}})
                }
                _ => anyhow::bail!("unexpected policy stage"),
            };
            let text = if failed && matches!(self.mode, Mode::Malformed) {
                "{}".into()
            } else if failed && matches!(self.mode, Mode::Empty) {
                String::new()
            } else {
                reply.to_string()
            };
            Ok(crate::message::CompletionResponse {
                message: crate::message::Message::assistant(vec![crate::message::Block::text(
                    text,
                )]),
                model: "authored-atomic".into(),
                refusal: None,
                malformed_tool_args: 0,
                stop_reason: if failed && matches!(self.mode, Mode::Truncated) {
                    crate::message::StopReason::MaxTokens
                } else {
                    crate::message::StopReason::EndTurn
                },
                usage: crate::message::Usage {
                    input_tokens: 11,
                    output_tokens: 7,
                    ..Default::default()
                },
            })
        }
    }
    fn temporary() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("atomic-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        path.canonicalize().unwrap()
    }
    fn authored(mode: Mode) -> Authored {
        Authored {
            mode,
            blocked_export: None,
            crowd_database: None,
            seen: Default::default(),
        }
    }
    fn read_json(root: &Path, file: &str) -> Value {
        serde_json::from_slice(&fs::read(root.join(file)).unwrap()).unwrap()
    }
    #[tokio::test]
    async fn paired_success_reopens_memory_and_sends_equal_initial_packets_with_equal_arm_caps() {
        let temporary = temporary();
        let root = temporary.join("run");
        let factory = authored(Mode::Good);
        let report = run_case(
            &root,
            &case(),
            "authored-atomic",
            &factory,
            &CancellationToken::new(),
            0,
        )
        .await
        .unwrap();
        assert_eq!(report["status"], "completed", "{report:#}");
        assert_eq!(report["arms"]["pipeline"]["policy_calls_started"], 3);
        assert_eq!(report["arms"]["direct"]["policy_calls_started"], 1);
        assert_eq!(report["arms"]["pipeline"]["input_tokens"], 33);
        assert_eq!(report["arms"]["pipeline"]["output_tokens"], 21);
        assert_eq!(report["arms"]["direct"]["input_tokens"], 11);
        assert_eq!(report["arms"]["direct"]["output_tokens"], 7);
        assert_eq!(
            report["arms"]["pipeline"]["independently_checked_retention"],
            true
        );
        assert_eq!(
            report["arms"]["direct"]["independently_checked_retention"],
            false
        );
        assert_eq!(
            report["arms"]["pipeline"]["stages"]["recall"]["selected_current"],
            true
        );
        let calls = factory.seen.lock().unwrap();
        assert_eq!(
            calls.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            [
                "pipeline-generate",
                "pipeline-check",
                "pipeline-appraise",
                "direct"
            ]
        );
        assert_eq!(
            calls[0].1, calls[3].1,
            "direct must receive precisely initial evidence, never retained output"
        );
        assert!(calls[3].1.get("proposal").is_none());
        let mut pipeline_cap = 0;
        for (name, want) in [
            ("pipeline-generate", 1536),
            ("pipeline-check", 3072),
            ("pipeline-appraise", 1536),
            ("direct", 6144),
        ] {
            let request = read_json(&root, &format!("{name}-request.json"));
            assert_eq!(request["request"]["max_tokens"], want);
            if name != "direct" {
                pipeline_cap += want;
            }
            assert_eq!(request["request"]["local_only"], true);
            assert!(request["request"]["tools"].as_array().unwrap().is_empty());
        }
        assert_eq!(pipeline_cap, 6144);
        let retained = read_json(&root, "retained-pipeline.json");
        assert_eq!(
            retained["draft"]["producer_version"],
            "situated-interpretation-v4"
        );
        assert!(retained["draft"]["support"]["atomic"].is_object());
        assert_eq!(report["source_integrity_verified"], true);
        drop(calls);
        fs::remove_dir_all(temporary).unwrap();
    }
    #[tokio::test]
    async fn failed_canonical_export_preserves_write_receipt_without_claiming_verification() {
        let temporary = temporary();
        let root = temporary.join("run");
        let mut factory = authored(Mode::Good);
        factory.blocked_export = Some(root.join("retained-pipeline.json"));
        let report = run_case(
            &root,
            &case(),
            "authored-atomic",
            &factory,
            &CancellationToken::new(),
            0,
        )
        .await
        .unwrap();
        let pipeline = &report["arms"]["pipeline"];
        assert_eq!(
            pipeline["stages"]["retain"]["status"], "completed",
            "{report:#}"
        );
        assert_eq!(pipeline["stages"]["stored_export"]["status"], "failed");
        assert_eq!(pipeline["stages"]["recall"]["status"], "withheld");
        assert_eq!(pipeline["independently_checked_retention"], false);
        assert!(pipeline["artifacts"]["receipt"].is_object());
        assert!(pipeline["artifacts"]["database"].is_object());
        assert!(pipeline["artifacts"]["retained"].is_null());
        assert_eq!(report["arms"]["direct"]["status"], "completed");
        fs::remove_dir_all(temporary).unwrap();
    }
    #[tokio::test]
    async fn crowded_recall_preserves_successful_retention_and_audit_artifacts() {
        let temporary = temporary();
        let root = temporary.join("run");
        let mut factory = authored(Mode::Good);
        factory.crowd_database = Some(root.join("native/support.memory-prototype.sqlite"));
        let report = run_case(
            &root,
            &case(),
            "authored-atomic",
            &factory,
            &CancellationToken::new(),
            0,
        )
        .await
        .unwrap();
        let pipeline = &report["arms"]["pipeline"];
        assert_eq!(
            pipeline["stages"]["retain"]["status"], "completed",
            "{report:#}"
        );
        assert_eq!(
            pipeline["stages"]["recall"]["status"], "failed",
            "{report:#}"
        );
        assert_eq!(pipeline["stages"]["recall"]["selected_current"], false);
        assert_eq!(pipeline["stages"]["recall"]["budget_omitted"], true);
        assert!(
            pipeline["stages"]["recall"]["packet_bytes"]
                .as_u64()
                .unwrap()
                <= 20_000
        );
        assert_eq!(pipeline["independently_checked_retention"], true);
        for name in ["receipt", "retained", "database", "recall"] {
            assert!(root
                .join(pipeline["artifacts"][name]["path"].as_str().unwrap())
                .is_file());
        }
        assert_eq!(report["arms"]["direct"]["status"], "completed");
        fs::remove_dir_all(temporary).unwrap();
    }
    #[tokio::test]
    async fn malformed_empty_and_truncated_generation_preserve_costs_and_leave_direct_independent()
    {
        for mode in [Mode::Malformed, Mode::Empty, Mode::Truncated] {
            let temporary = temporary();
            let root = temporary.join("run");
            let factory = authored(mode);
            let report = run_case(
                &root,
                &case(),
                "authored-atomic",
                &factory,
                &CancellationToken::new(),
                0,
            )
            .await
            .unwrap();
            assert_eq!(report["status"], "failed");
            assert_eq!(
                report["arms"]["pipeline"]["stages"]["generate"]["status"],
                "failed"
            );
            assert_eq!(
                report["arms"]["pipeline"]["stages"]["check"]["status"],
                "withheld"
            );
            assert_eq!(report["arms"]["pipeline"]["input_tokens"], 11);
            assert_eq!(report["arms"]["pipeline"]["output_tokens"], 7);
            assert_eq!(
                report["arms"]["direct"]["status"], "completed",
                "{report:#}"
            );
            assert!(root.join("pipeline-generate-reply.json").is_file());
            assert!(root.join("pipeline-generate-raw.txt").is_file());
            assert!(!root.join("retained-pipeline.json").exists());
            fs::remove_dir_all(temporary).unwrap();
        }
    }
    #[tokio::test]
    async fn cancelled_pipeline_preserves_started_request_without_fabricated_usage() {
        let temporary = temporary();
        let root = temporary.join("run");
        let factory = authored(Mode::Cancel);
        let report = run_case(
            &root,
            &case(),
            "authored-atomic",
            &factory,
            &CancellationToken::new(),
            0,
        )
        .await
        .unwrap();
        assert_eq!(report["status"], "failed");
        assert_eq!(report["arms"]["pipeline"]["cancelled"], true);
        assert!(report["arms"]["pipeline"]["input_tokens"].is_null());
        assert_eq!(report["arms"]["pipeline"]["policy_calls_started"], 1);
        assert!(root.join("pipeline-generate-request.json").is_file());
        assert!(!root.join("pipeline-generate-reply.json").exists());
        assert_eq!(report["arms"]["direct"]["status"], "completed");
        fs::remove_dir_all(temporary).unwrap();
    }
    #[tokio::test]
    async fn missing_artifact_remains_typed_source_evidence_in_both_arms() {
        let temporary = temporary();
        let root = temporary.join("run");
        let factory = authored(Mode::Good);
        let mut missing = case();
        missing.native.observed_output = None;
        let report = run_case(
            &root,
            &missing,
            "authored-atomic",
            &factory,
            &CancellationToken::new(),
            1,
        )
        .await
        .unwrap();
        assert_eq!(report["status"], "completed", "{report:#}");
        assert!(report["initial_hashes"]["observed_output"].is_null());
        assert!(!root.join("native/workspace/output.json").exists());
        let calls = factory.seen.lock().unwrap();
        assert_eq!(calls[0].0, "direct");
        assert_eq!(calls[0].1, calls[1].1);
        assert!(calls[0].1["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "ArtifactObservation"
                && s["text"].as_str().unwrap().contains("Missing")));
        drop(calls);
        fs::remove_dir_all(temporary).unwrap();
    }
    #[tokio::test]
    async fn arm_deadline_is_shared_and_expiry_refuses_provider_setup() {
        let temporary = temporary();
        let factory = authored(Mode::Good);
        let started = Instant::now() - Duration::from_secs(541);
        assert!(provider(
            &temporary,
            "direct",
            &factory,
            &CancellationToken::new(),
            started,
            6144
        )
        .await
        .is_err());
        assert!(factory.seen.lock().unwrap().is_empty());
        let started = Instant::now() - Duration::from_secs(500);
        assert!(remaining(started, 1536).unwrap().deadline() <= Duration::from_secs(40));
        fs::remove_dir_all(temporary).unwrap();
    }
}
