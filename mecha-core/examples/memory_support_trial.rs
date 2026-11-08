//! One frozen synthetic factual-support case; no ambient stores or model reload.
#[cfg(feature = "memory-prototype")]
mod enabled {
    use anyhow::{ensure, Context, Result};
    use mecha_core::{
        memory_support_workflow::{load_case, run_case, StageProviders},
        memory_trial_provider::{LocalTrialConfig, LocalTrialProvider},
        provider::Provider,
    };
    use std::{path::PathBuf, time::Duration};
    use tokio_util::sync::CancellationToken;

    const USAGE: &str = "memory_support_trial --synthetic-only NEW_ROOT ENDPOINT MODEL HOLDS_DIRECTORY 70510 CASE_FILE";
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
                LocalTrialProvider::new(
                    LocalTrialConfig {
                        endpoint: self.endpoint.clone(),
                        model: self.model.clone(),
                        holds_dir: self.holds.clone(),
                        seed: self.seed,
                    },
                    cancel.clone(),
                )
                .await?,
            ))
        }
    }
    pub async fn run() -> Result<()> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        ensure!(args.len() == 7 && args[0] == "--synthetic-only", USAGE);
        let root = PathBuf::from(&args[1]);
        let holds = PathBuf::from(&args[4]);
        ensure!(
            root.is_absolute() && !root.exists() && holds.is_absolute(),
            USAGE
        );
        let seed: u64 = args[5].parse().context(USAGE)?;
        ensure!(seed == 70510, "frozen qualification requires seed70510");
        let case = load_case(&PathBuf::from(&args[6]))?;
        let cancel = CancellationToken::new();
        let deadline_cancel = cancel.clone();
        let deadline = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(600)).await;
            deadline_cancel.cancel();
        });
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
        let report = run_case(&root, &case, &resident.model, &resident, &cancel).await;
        deadline.abort();
        signal.abort();
        let report = report?;
        println!("{}", serde_json::to_string(&report)?);
        ensure!(
            report["status"] == "completed",
            "native factual-support case failed; inspect report.json"
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
