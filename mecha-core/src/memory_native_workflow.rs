//! Explicit paired synthetic qualification of complete native evidence.
//! Both arms share authenticated evidence and a response contract. Results remain
//! provisional host audit artifacts; this workflow never retains a judgment.
use crate::{
    memory::interpretation::{NativeEvidenceFormat, NativeEvidenceRequest},
    memory_bridge_trial::recorded_provider,
    memory_factual_support::{pass, GenerationBudget},
    memory_support_workflow::{
        checkpoint, descriptor, packet_bytes, prepare_native, read_case, write_audit_exports,
        write_value, PreparedNative, StageProviders, TrialCase,
    },
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

pub const PROTOCOL: &str = "memory-native-comparison-v1";
const TOKENS: u32 = 6144;
const SECONDS: u64 = 540;
const STAGES: [&str; 6] = [
    "prepare",
    "provider",
    "recheck",
    "inference",
    "postcheck",
    "finalize",
];

/// Explicit owner-selected synthetic fixture; probe labels are never consulted.
pub fn load_case(path: &Path) -> Result<TrialCase> {
    let case = read_case(path)?;
    case.validate_native()?;
    Ok(case)
}
fn stage() -> Value {
    json!({"status":"pending","error":null})
}
fn arm() -> Value {
    let stages: serde_json::Map<String, Value> =
        STAGES.into_iter().map(|s| (s.into(), stage())).collect();
    json!({"status":"pending","error":null,"stages":stages,"policy_calls_started":0,
        "output_token_allowance":TOKENS,"deadline_seconds":SECONDS,"durable_judgment":false})
}
fn activate(arm: &mut Value, name: &str) {
    arm["active_stage"] = json!(name);
}
fn completed(arm: &mut Value, name: &str) {
    arm["stages"][name]["status"] = json!("completed");
}
fn remaining(started: Instant) -> Result<GenerationBudget> {
    GenerationBudget::new(
        TOKENS,
        Duration::from_secs(SECONDS)
            .checked_sub(started.elapsed())
            .context("arm deadline exhausted")?,
    )
}
async fn discover(
    root: &Path,
    name: &str,
    providers: &dyn StageProviders,
    cancel: &CancellationToken,
    started: Instant,
) -> Result<Box<dyn Provider>> {
    let budget = remaining(started)?;
    let inner = tokio::select! { biased;
        _=cancel.cancelled()=>anyhow::bail!("arm cancelled before provider discovery"),
        result=tokio::time::timeout(budget.deadline(),providers.create(name,cancel))=>result.context("provider discovery timed out")??
    };
    recorded_provider(
        inner,
        &root.join(format!("{name}-provider.jsonl")),
        u64::from(TOKENS),
        cancel.clone(),
    )
}
struct ArmRun<'a> {
    root: &'a Path,
    name: &'a str,
    model: &'a str,
    providers: &'a dyn StageProviders,
    cancel: &'a CancellationToken,
    started: Instant,
}
async fn run_arm(
    run: ArmRun<'_>,
    native: &PreparedNative,
    request: &NativeEvidenceRequest,
    arm: &mut Value,
) -> Result<()> {
    let ArmRun {
        root,
        name,
        model,
        providers,
        cancel,
        started,
    } = run;
    activate(arm, "prepare");
    ensure!(!cancel.is_cancelled(), "arm cancelled");
    let format = if name == "raw" {
        NativeEvidenceFormat::Raw
    } else {
        NativeEvidenceFormat::Typed
    };
    let (input, schema) = request.input(format)?;
    write_value(
        &root.join(format!("{name}-input.json")),
        &json!({"request_key":request.view().request_key,"input":input,"schema":schema}),
    )?;
    arm["input"] = descriptor(root, &root.join(format!("{name}-input.json")))?;
    let planned = request.generation_request(model, remaining(started)?, format)?;
    arm["stages"]["inference"]["packet_bytes"] = json!(packet_bytes(&planned));
    arm["stages"]["inference"]["max_output_tokens"] = json!(planned.max_tokens);
    completed(arm, "prepare");
    activate(arm, "provider");
    let provider = discover(root, name, providers, cancel, started).await?;
    completed(arm, "provider");
    activate(arm, "recheck");
    native
        .memory
        .recheck_native_evidence(request, &native.source.access)?;
    ensure!(!cancel.is_cancelled(), "arm cancelled before dispatch");
    completed(arm, "recheck");
    activate(arm, "inference");
    let outcome = pass(
        provider.as_ref(),
        model,
        Ok(planned),
        remaining(started)?,
        cancel,
        |text, _| Ok(text.to_string()),
    )
    .await;
    let mut exports = json!({"stages":{name:arm["stages"]["inference"].clone()}});
    write_audit_exports(root, &mut exports, name, &outcome)?;
    arm["stages"]["inference"] = exports["stages"][name].take();
    let inference = outcome
        .result
        .map_err(|e| anyhow::anyhow!("native pass refused: {e:?}"));
    // This fresh check runs even when the provider returned malformed output,
    // refusal, model mismatch, or cancellation. Never cache discovery authority.
    let postcheck = native
        .memory
        .recheck_native_evidence(request, &native.source.access);
    if let Err(error) = &inference {
        arm["stages"]["inference"]["status"] = json!("failed");
        arm["stages"]["inference"]["error"] = json!(format!("{error:#}"));
    } else {
        completed(arm, "inference");
    }
    activate(arm, "postcheck");
    postcheck?;
    completed(arm, "postcheck");
    activate(arm, "inference");
    let response = inference?;
    ensure!(
        outcome
            .usage
            .is_none_or(|u| u.output_tokens <= u64::from(TOKENS)),
        "provider exceeded output allowance"
    );
    activate(arm, "finalize");
    let result =
        native
            .memory
            .finish_native_appraisal(request, &response, &native.source.access)?;
    let path = root.join(format!("{name}-result.json"));
    write_value(&path, &result)?;
    arm["result"] = descriptor(root, &path)?;
    arm["appraisal"] = serde_json::to_value(result)?;
    completed(arm, "finalize");
    Ok(())
}
fn finish_arm(arm: &mut Value, result: Result<()>, started: Instant, cancelled: bool) {
    match result {
        Ok(()) => arm["status"] = json!("completed"),
        Err(error) => {
            let active = arm["active_stage"]
                .as_str()
                .unwrap_or("prepare")
                .to_string();
            arm["stages"][&active]["status"] = json!("failed");
            arm["stages"][&active]["error"] = json!(format!("{error:#}"));
            arm["status"] = json!("failed");
            arm["error"] = json!(format!("{error:#}"));
        }
    }
    for entry in arm["stages"]
        .as_object_mut()
        .expect("arm stages")
        .values_mut()
    {
        if entry["status"] == "pending" {
            entry["status"] = json!("withheld");
            entry["error"] = json!("upstream stage unavailable");
        }
    }
    arm["policy_calls_started"] =
        json!(u8::from(arm["stages"]["inference"]["request"].is_object()));
    for key in ["input_tokens", "output_tokens"] {
        arm[key] = arm["stages"]["inference"][key].clone();
    }
    arm["elapsed_ms"] = json!(started.elapsed().as_millis());
    arm["cancelled"] = json!(cancelled);
    arm.as_object_mut().expect("arm").remove("active_stage");
}
async fn paired(
    root: &Path,
    case: &TrialCase,
    model: &str,
    providers: &dyn StageProviders,
    cancel: &CancellationToken,
    order: [&str; 2],
    report: &mut Value,
) -> Result<()> {
    let native = prepare_native(root, case, cancel, report, true).await?;
    report["active_stage"] = json!("native_prepare");
    let database = descriptor(root, &native.source.database)?;
    report["postcapture_database"] = database.clone();
    let request = native
        .memory
        .prepare_native_evidence(&native.prepared, &native.source.access)?;
    let facts = root.join("native-evidence.json");
    write_value(&facts, request.view())?;
    report["normalized_facts"] = descriptor(root, &facts)?;
    report["request_key"] = json!(request.view().request_key);
    report["stages"]["native_prepare"]["status"] = json!("completed");
    checkpoint(root, report)?;
    let mut any_cancelled = cancel.is_cancelled();
    for name in order {
        let arm_started = Instant::now();
        // Each arm owns its deadline; only root cancellation reaches siblings.
        let arm_cancel = cancel.child_token();
        let timer_cancel = arm_cancel.clone();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(SECONDS)).await;
            timer_cancel.cancel();
        });
        let run = ArmRun {
            root,
            name,
            model,
            providers,
            cancel: &arm_cancel,
            started: arm_started,
        };
        let result = run_arm(run, &native, &request, &mut report["arms"][name]).await;
        timer.abort();
        any_cancelled |= arm_cancel.is_cancelled();
        let unchanged = descriptor(root, &native.source.database)? == database;
        report["arms"][name]["database_unchanged"] = json!(unchanged);
        let result = result.and_then(|()| {
            ensure!(unchanged, "database changed after read-only arm");
            Ok(())
        });
        finish_arm(
            &mut report["arms"][name],
            result,
            arm_started,
            arm_cancel.is_cancelled(),
        );
        checkpoint(root, report)?;
    }
    let intact = descriptor(root, &native.source_control)? == report["source_only_database"]
        && descriptor(root, &native.source.input)?["sha256"]
            == report["initial_hashes"]["initial_input"]
        && descriptor(root, &native.source.transcript)?["sha256"]
            == report["initial_hashes"]["native_transcript"]
        && match case.native.observed_output {
            Some(_) => {
                descriptor(root, &native.source.artifact)?["sha256"]
                    == report["initial_hashes"]["observed_output"]
            }
            None => !native.source.artifact.exists(),
        };
    report["source_integrity_verified"] = json!(intact);
    report["cancelled"] = json!(any_cancelled);
    ensure!(intact, "captured source or source-only control changed");
    if order
        .iter()
        .all(|name| report["arms"][name]["status"] == "completed")
        && !any_cancelled
    {
        report["status"] = json!("completed");
    }
    Ok(())
}
/// One source capture, two resident-only one-call arms, counterbalanced by index.
/// Execution failures remain in report.json; invalid explicit inputs return Err.
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
        ["raw", "typed"]
    } else {
        ["typed", "raw"]
    };
    let started = Instant::now();
    let mut report = json!({"protocol":PROTOCOL,"case_id":case.case_id,"case_sha256":case.case_sha256,"case_index":case_index,"model":model,"seed":70510,"arm_order":order,"status":"failed","error":null,"cancelled":false,"artifacts":{},"diagnostic_policy_calls":0,"stages":{"source":stage(),"capture":stage(),"native_prepare":stage()},"arms":{"raw":arm(),"typed":arm()}});
    checkpoint(&root, &report)?;
    if let Err(error) = paired(&root, case, model, providers, cancel, order, &mut report).await {
        let active = report["active_stage"]
            .as_str()
            .unwrap_or("source")
            .to_string();
        report["stages"][&active]["status"] = json!("failed");
        report["stages"][&active]["error"] = json!(format!("{error:#}"));
        report["error"] = json!(format!("{error:#}"));
    }
    for stage in report["stages"]
        .as_object_mut()
        .expect("stages")
        .values_mut()
    {
        if stage["status"] == "pending" {
            stage["status"] = json!("withheld");
            stage["error"] = json!("upstream stage unavailable");
        }
    }
    for name in order {
        if report["arms"][name]["status"] == "pending" {
            finish_arm(
                &mut report["arms"][name],
                Err(anyhow::anyhow!("native preparation unavailable")),
                Instant::now(),
                cancel.is_cancelled(),
            );
        }
    }
    report
        .as_object_mut()
        .expect("report")
        .remove("active_stage");
    report["elapsed_ms"] = json!(started.elapsed().as_millis());
    report["cancelled"] = json!(report["cancelled"] == true || cancel.is_cancelled());
    checkpoint(&root, &report)?;
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        memory_support_workflow::{NativeCase, ProbeCase},
        message::{Block, CompletionRequest, CompletionResponse, Message, StopReason, Usage},
        provider::StreamSink,
    };
    use std::sync::{Arc, Mutex};
    struct Scripted {
        root: std::path::PathBuf,
        seen: Arc<Mutex<Vec<String>>>,
        root_cancel: CancellationToken,
        mode: u8,
    }
    struct Reply {
        cancel: CancellationToken,
        root_cancel: CancellationToken,
        root: std::path::PathBuf,
        mode: u8,
    }
    fn withdraw(root: &Path) -> Result<()> {
        let authority =
            crate::memory_experience_bridge::LocalAuthority::open(&root.join("native/authority"))?;
        let binding = fs::read_dir(root.join("native/authority"))?
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "binding"))
            .unwrap();
        let binding: crate::memory::host_capture::ExternalBinding =
            serde_json::from_slice(&fs::read(binding)?)?;
        authority.withdraw(&binding.locator)
    }
    #[async_trait::async_trait]
    impl StageProviders for Scripted {
        async fn create(
            &self,
            name: &str,
            cancel: &CancellationToken,
        ) -> Result<Box<dyn Provider>> {
            self.seen.lock().unwrap().push(name.into());
            if self.mode == 2 && name == "raw" {
                withdraw(&self.root)?;
            }
            if self.mode == 3 {
                anyhow::bail!("scripted discovery failure");
            }
            Ok(Box::new(Reply {
                cancel: cancel.clone(),
                root_cancel: self.root_cancel.clone(),
                root: self.root.clone(),
                mode: self.mode,
            }))
        }
    }
    #[async_trait::async_trait]
    impl Provider for Reply {
        fn id(&self) -> &str {
            "scripted"
        }
        fn default_model(&self) -> &str {
            "scripted-native"
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
            _: Option<&StreamSink>,
        ) -> Result<CompletionResponse> {
            assert_eq!(request.max_tokens, 6144);
            assert!(request.local_only && request.tools.is_empty());
            assert_eq!(request.think, Some(false));
            if self.mode == 1 && !self.root.join("typed-provider.jsonl").is_file() {
                self.cancel.cancel();
                std::future::pending::<()>().await;
            }
            if self.mode == 6 {
                self.root_cancel.cancel();
                std::future::pending::<()>().await;
            }
            let text = request.messages[0].text();
            let packet: Value =
                serde_json::from_str(&text[text.find('{').unwrap()..=text.rfind('}').unwrap()])?;
            let response = json!({"request_key":packet["request_key"],"state":"unknown","because":[],"reason":"The scripted provider measures lifecycle only.","questions":["Does the artifact meet the current criterion?"]});
            if self.mode == 4
                && self.root.join("raw-provider.jsonl").is_file()
                && !self.root.join("typed-provider.jsonl").is_file()
            {
                withdraw(&self.root)?;
            }
            let text = if self.mode == 5 {
                format!("{{\"state\":\"unknown\",{}", &response.to_string()[1..])
            } else {
                response.to_string()
            };
            Ok(CompletionResponse {
                message: Message::assistant(vec![Block::text(text)]),
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 30,
                    output_tokens: 20,
                    ..Default::default()
                },
                refusal: None,
                model: "scripted-native".into(),
                malformed_tool_args: 0,
            })
        }
    }
    fn case() -> TrialCase {
        TrialCase {
            version: 1,
            case_id: "native-authored".into(),
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
    async fn run(mode: u8, index: usize) -> (std::path::PathBuf, Value, Vec<String>) {
        let temporary =
            std::env::temp_dir().join(format!("native-trial-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&temporary).unwrap();
        let root = temporary.canonicalize().unwrap().join("run");
        let seen = Arc::new(Mutex::new(vec![]));
        let cancel = CancellationToken::new();
        let factory = Scripted {
            root: root.clone(),
            root_cancel: cancel.clone(),
            seen: seen.clone(),
            mode,
        };
        let report = run_case(&root, &case(), "scripted-native", &factory, &cancel, index)
            .await
            .unwrap();
        let seen = seen.lock().unwrap().clone();
        (temporary, report, seen)
    }
    #[tokio::test]
    async fn paired_calls_share_binding_schema_and_preserve_database() {
        let (tmp, report, seen) = run(0, 1).await;
        assert_eq!(report["status"], "completed", "{report:#}");
        assert_eq!(seen, vec!["typed", "raw"]);
        let root = tmp.join("run");
        let mut requests = vec![];
        for name in ["raw", "typed"] {
            let arm = &report["arms"][name];
            assert_eq!(arm["policy_calls_started"], 1);
            assert_eq!(arm["database_unchanged"], true);
            assert_eq!(arm["stages"]["inference"]["output_tokens"], 20);
            let audit = fs::read_to_string(root.join(format!("{name}-provider.jsonl"))).unwrap();
            let events: Vec<Value> = audit
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            let calls: Vec<_> = events
                .iter()
                .filter(|e| e["event"] == "request_started")
                .collect();
            assert_eq!(calls.len(), 1);
            requests.push(
                serde_json::from_slice::<Value>(
                    &fs::read(root.join(format!("{name}-input.json"))).unwrap(),
                )
                .unwrap(),
            );
        }
        assert_eq!(requests[0]["schema"], requests[1]["schema"]);
        assert_eq!(requests[0]["request_key"], requests[1]["request_key"]);
        assert_ne!(requests[0]["input"], requests[1]["input"]);
        fs::remove_dir_all(tmp).unwrap();
    }
    #[tokio::test]
    async fn arm_cancellation_keeps_attempt_and_preserves_sibling_budget() {
        let (tmp, report, seen) = run(1, 0).await;
        assert_eq!(report["status"], "failed");
        assert_eq!(report["cancelled"], true);
        assert_eq!(seen, vec!["raw", "typed"]);
        assert_eq!(report["arms"]["raw"]["policy_calls_started"], 1);
        assert_eq!(report["arms"]["raw"]["cancelled"], true);
        assert_eq!(report["arms"]["typed"]["policy_calls_started"], 1);
        assert_eq!(report["arms"]["typed"]["status"], "completed");
        assert_eq!(report["arms"]["typed"]["cancelled"], false);
        assert_eq!(report["arms"]["raw"]["database_unchanged"], true);
        fs::remove_dir_all(tmp).unwrap();
    }
    #[tokio::test]
    async fn root_cancellation_keeps_attempt_and_withholds_sibling_call() {
        let (tmp, report, seen) = run(6, 0).await;
        assert_eq!(report["status"], "failed");
        assert_eq!(report["cancelled"], true);
        assert_eq!(seen, vec!["raw"]);
        assert_eq!(report["arms"]["raw"]["policy_calls_started"], 1);
        assert_eq!(report["arms"]["typed"]["policy_calls_started"], 0);
        assert_eq!(report["arms"]["typed"]["cancelled"], true);
        fs::remove_dir_all(tmp).unwrap();
    }
    #[tokio::test]
    async fn revocation_during_discovery_dispatches_nothing_and_preserves_stages() {
        let (tmp, report, seen) = run(2, 0).await;
        assert_eq!(report["status"], "failed");
        assert_eq!(seen, vec!["raw", "typed"]);
        for name in ["raw", "typed"] {
            assert_eq!(report["arms"][name]["policy_calls_started"], 0);
            assert_eq!(
                report["arms"][name]["stages"]["recheck"]["status"],
                "failed"
            );
            assert_eq!(
                report["arms"][name]["stages"]["inference"]["status"],
                "withheld"
            );
        }
        fs::remove_dir_all(tmp).unwrap();
    }
    #[tokio::test]
    async fn provider_setup_failure_keeps_both_arms_and_failure_slots() {
        let (tmp, report, seen) = run(3, 0).await;
        assert_eq!(seen, vec!["raw", "typed"]);
        for name in ["raw", "typed"] {
            assert_eq!(
                report["arms"][name]["stages"]["provider"]["status"],
                "failed"
            );
            assert_eq!(report["arms"][name]["policy_calls_started"], 0);
            assert_eq!(report["arms"][name]["database_unchanged"], true);
        }
        fs::remove_dir_all(tmp).unwrap();
    }
    #[tokio::test]
    async fn revocation_after_response_is_recorded_before_result_is_withheld() {
        let (tmp, report, _) = run(4, 0).await;
        assert_eq!(report["arms"]["raw"]["policy_calls_started"], 1);
        assert_eq!(
            report["arms"]["raw"]["stages"]["inference"]["status"],
            "completed"
        );
        assert_eq!(
            report["arms"]["raw"]["stages"]["postcheck"]["status"],
            "failed"
        );
        assert_eq!(
            report["arms"]["raw"]["stages"]["finalize"]["status"],
            "withheld"
        );
        assert_eq!(report["arms"]["typed"]["policy_calls_started"], 0);
        assert!(tmp.join("run/raw-reply.json").is_file());
        fs::remove_dir_all(tmp).unwrap();
    }
    #[tokio::test]
    async fn duplicate_reply_keys_reach_strict_finalizer_unchanged() {
        let (tmp, report, _) = run(5, 0).await;
        for name in ["raw", "typed"] {
            assert_eq!(report["arms"][name]["policy_calls_started"], 1);
            assert_eq!(
                report["arms"][name]["stages"]["finalize"]["status"],
                "failed"
            );
            assert!(report["arms"][name]["error"]
                .as_str()
                .unwrap()
                .contains("duplicate field"));
            assert_eq!(report["arms"][name]["database_unchanged"], true);
        }
        fs::remove_dir_all(tmp).unwrap();
    }
}
