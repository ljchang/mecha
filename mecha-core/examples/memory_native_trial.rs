//! Explicit synthetic-only paired comparison; no ambient config or model loading.
#[cfg(feature = "memory-prototype")]
mod enabled {
    use anyhow::{ensure, Context, Result};
    use mecha_core::{
        memory_native_workflow::{load_case, run_case, run_checked_case, run_verdict_controls},
        memory_support_workflow::StageProviders,
        memory_trial_provider::{LocalTrialConfig, LocalTrialProvider},
        provider::Provider,
    };
    use std::{path::PathBuf, time::Duration};
    use tokio_util::sync::CancellationToken;

    const USAGE: &str = "memory_native_trial --synthetic-only NEW_ROOT ENDPOINT MODEL HOLDS_DIRECTORY 70510 CASE_FILE CASE_INDEX [--checks-first|--verdict-controls]";
    struct Resident {
        endpoint: String,
        model: String,
        holds: PathBuf,
        seed: u64,
    }
    #[async_trait::async_trait]
    impl StageProviders for Resident {
        async fn create(&self, _: &str, cancel: &CancellationToken) -> Result<Box<dyn Provider>> {
            Ok(Box::new(
                LocalTrialProvider::new_structured_with_timeout(
                    LocalTrialConfig {
                        endpoint: self.endpoint.clone(),
                        model: self.model.clone(),
                        holds_dir: self.holds.clone(),
                        seed: self.seed,
                    },
                    cancel.clone(),
                    Duration::from_secs(540),
                )
                .await?,
            ))
        }
    }
    pub async fn run() -> Result<()> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        ensure!(
            (args.len() == 8 || args.len() == 9) && args[0] == "--synthetic-only",
            USAGE
        );
        let mode = args.get(8).map(String::as_str);
        ensure!(
            matches!(mode, None | Some("--checks-first" | "--verdict-controls")),
            USAGE
        );
        let root = PathBuf::from(&args[1]);
        let holds = PathBuf::from(&args[4]);
        ensure!(
            root.is_absolute() && !root.exists() && holds.is_absolute(),
            USAGE
        );
        let seed: u64 = args[5].parse().context(USAGE)?;
        ensure!(seed == 70510, "frozen comparison requires seed70510");
        let case = load_case(&PathBuf::from(&args[6]))?;
        let index: usize = args[7].parse().context(USAGE)?;
        let cancel = CancellationToken::new();
        let signal_cancel = cancel.clone();
        let signal = tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                signal_cancel.cancel();
            }
        });
        let resident = Resident {
            endpoint: args[2].clone(),
            model: args[3].clone(),
            holds,
            seed,
        };
        let result = match mode {
            Some("--checks-first") => {
                run_checked_case(&root, &case, &resident.model, &resident, &cancel, index).await
            }
            Some("--verdict-controls") => {
                run_verdict_controls(&root, &case, &resident.model, &resident, &cancel, index).await
            }
            _ => run_case(&root, &case, &resident.model, &resident, &cancel, index).await,
        };
        signal.abort();
        let report = result?;
        println!("{}", serde_json::to_string(&report)?);
        ensure!(
            report["status"] == "completed",
            "native evidence comparison failed; inspect report.json"
        );
        Ok(())
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
