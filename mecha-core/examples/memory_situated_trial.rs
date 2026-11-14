//! One fixed synthetic situated-appraisal comparison pair. No ambient stores.
#[cfg(feature = "memory-prototype")]
mod enabled {
    use anyhow::{ensure, Context, Result};
    use mecha_core::{
        memory::{interpretation::InterpretationContext, Memory},
        memory_bridge_trial::recorded_provider,
        memory_experience_bridge::LocalAuthority,
        memory_situated_appraisal::{generate, GenerationBudget},
        memory_situated_needs::new_context_id,
        memory_situated_workflow::{
            criterion, prepare_fixture, run_planning_task, Fixture, Goal, ROUTES,
        },
        memory_trial_provider::{LocalTrialConfig, LocalTrialProvider},
    };
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::Arc,
        time::{Duration, Instant},
    };
    use tokio_util::sync::CancellationToken;

    const USAGE: &str = "memory_situated_trial --synthetic-only NEW_ROOT ENDPOINT MODEL HOLDS_DIRECTORY SEED fast|complete source-first|interpretation-first";

    fn digest(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
    fn write(path: &Path, value: &Value) -> Result<()> {
        fs::write(path, serde_json::to_vec_pretty(value)?)?;
        Ok(())
    }

    // The original source paths inside authority bindings remain immutable.
    // Each arm gets an independent DB and authority for its subsequent capture.
    fn clone_fixture(source: &Fixture, source_root: &Path, target: &Path) -> Result<Fixture> {
        fs::create_dir(target)?;
        let database = target.join("situated.memory-prototype.sqlite");
        fs::copy(&source.database, &database)?;
        let authority_path = target.join("authority");
        fs::create_dir(&authority_path)?;
        let mut total = 0usize;
        for entry in fs::read_dir(source_root.join("authority"))? {
            let entry = entry?;
            ensure!(entry.file_type()?.is_file(), "unexpected authority entry");
            let bytes = fs::read(entry.path())?;
            total += bytes.len();
            ensure!(total <= 32 * 1024 * 1024, "oversized synthetic authority");
            fs::write(authority_path.join(entry.file_name()), bytes)?;
        }
        Ok(Fixture {
            database,
            authority: LocalAuthority::open(&authority_path)?,
            receipt: source.receipt.clone(),
            access: source.access.clone(),
            source_transcript: source.source_transcript.clone(),
            source_check: source.source_check.clone(),
            source_confirmation: source.source_confirmation.clone(),
        })
    }

    pub async fn run() -> Result<()> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        ensure!(args.len() == 8 && args[0] == "--synthetic-only", USAGE);
        let root = PathBuf::from(&args[1]);
        let holds = PathBuf::from(&args[4]);
        ensure!(
            root.is_absolute() && !root.exists() && holds.is_absolute(),
            USAGE
        );
        let seed: u64 = args[5].parse().context(USAGE)?;
        let goal = match args[6].as_str() {
            "fast" => Goal::Fast,
            "complete" => Goal::Complete,
            _ => anyhow::bail!(USAGE),
        };
        let order = match args[7].as_str() {
            "source-first" => ["source", "interpretation"],
            "interpretation-first" => ["interpretation", "source"],
            _ => anyhow::bail!(USAGE),
        };
        fs::create_dir(&root)?;
        let root = root.canonicalize()?;
        let started = Instant::now();
        let cancel = CancellationToken::new();
        let timer_cancel = cancel.clone();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(600)).await;
            timer_cancel.cancel();
        });
        let signal_cancel = cancel.clone();
        let signal = tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                signal_cancel.cancel();
            }
        });
        let endpoint = args[2].clone();
        let model = args[3].clone();
        let result = async {
            let source_root = root.join("source-fixture");
            let source = prepare_fixture(&source_root).await?;
            let original = fs::read(&source.source_transcript)?;
            let original_db = fs::read(&source.database)?;
            let control = clone_fixture(&source, &source_root, &root.join("source-arm"))?;
            let treatment = clone_fixture(&source, &source_root, &root.join("interpretation-arm"))?;
            let context = InterpretationContext::new(criterion(goal), new_context_id())?;
            let mut memory = Memory::open(&treatment.database)?.with_source_guard(Arc::new(treatment.authority.clone()));
            let prepared = memory.prepare_interpretation(&treatment.receipt, &[], &context, &memory.snapshot()?, &treatment.access)?;
            write(&root.join("generation-input.json"), &serde_json::from_str(prepared.generation_input())?)?;
            let provider = LocalTrialProvider::new(LocalTrialConfig { endpoint: endpoint.clone(), model: model.clone(), holds_dir: holds.clone(), seed }, cancel.clone()).await;
            let generation = match provider {
                Ok(provider) => {
                    let provider = recorded_provider(Box::new(provider), &root.join("generation-provider.jsonl"), 4096, cancel.clone())?;
                    let generated = generate(provider.as_ref(), &model, &prepared, GenerationBudget::new(4096, Duration::from_secs(180))?, &cancel).await;
                    match generated.draft {
                        Ok(draft) => {
                            write(&root.join("draft.json"), &serde_json::to_value(&draft)?)?;
                            match memory.retain_interpretation(prepared, draft, &treatment.access) {
                                Ok(receipt) => json!({"status":"retained","receipt":receipt,"usage":generated.usage,"elapsed_ms":generated.elapsed.as_millis()}),
                                Err(error) => json!({"status":"admission_failed","error":error.to_string(),"usage":generated.usage}),
                            }
                        }
                        Err(error) => json!({"status":"generation_failed","error":format!("{error:?}"),"usage":generated.usage,"elapsed_ms":generated.elapsed.as_millis()}),
                    }
                }
                Err(error) => json!({"status":"provider_setup_failed","error":error.to_string()}),
            };
            drop(memory);
            let mut report = json!({"protocol":"situated-interpretation-pair-v1","goal":args[6],"seed":seed,"order":order,"routes":ROUTES,"criterion":context.criterion(),"generation":generation,"arms":{},"source_transcript_sha256":digest(&original),"source_database_sha256":digest(&original_db)});
            write(&root.join("report.json"), &report)?;
            for arm in order {
                // Every planned arm remains recorded even when generation failed.
                let fixture = if arm == "source" { &control } else { &treatment };
                let phase_started = Instant::now();
                let run = async {
                    let provider = LocalTrialProvider::new(LocalTrialConfig {endpoint:endpoint.clone(),model:model.clone(),holds_dir:holds.clone(),seed}, cancel.clone()).await?;
                    let provider = recorded_provider(Box::new(provider), &root.join(format!("{arm}-provider.jsonl")), 16384, cancel.clone())?;
                    run_planning_task(fixture, &context, arm == "interpretation", &root.join(format!("{arm}-task")), provider, &cancel).await
                }.await;
                report["arms"][arm] = match run {
                    Ok(task) => json!({"status":"recorded","task":task,"elapsed_ms":phase_started.elapsed().as_millis()}),
                    Err(error) => json!({"status":"setup_failed","error":error.to_string(),"elapsed_ms":phase_started.elapsed().as_millis()}),
                };
                write(&root.join("report.json"), &report)?;
            }
            report["source_unchanged"] = json!(fs::read(&source.source_transcript)? == original && fs::read(&source.database)? == original_db);
            report["elapsed_ms"] = json!(started.elapsed().as_millis());
            report["deadline_expired"] = json!(started.elapsed() >= Duration::from_secs(600));
            write(&root.join("report.json"), &report)?;
            println!("{}", serde_json::to_string(&report)?);
            Ok::<_, anyhow::Error>(())
        }.await;
        timer.abort();
        signal.abort();
        result
    }
}

#[cfg(feature = "memory-prototype")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    enabled::run().await
}

#[cfg(not(feature = "memory-prototype"))]
fn main() {
    eprintln!("requires --features memory-prototype and explicit --synthetic-only");
}
