//! One explicit synthetic bridge trial per process; no ambient configuration.
#[cfg(feature = "memory-prototype")]
mod enabled {
    use anyhow::{ensure, Context, Result};
    use mecha_core::memory_bridge_trial::{run_trial, TrialConfig, TrialRequest};
    use mecha_core::memory_trial_provider::{LocalTrialConfig, LocalTrialProvider};
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use tokio_util::sync::CancellationToken;

    const USAGE: &str = "usage: memory_bridge_trial --synthetic-only REQUEST_JSON NEW_DIRECTORY --endpoint URL --model MODEL --holds-dir ABSOLUTE_DIRECTORY --seed U64";

    struct Arguments {
        request: PathBuf,
        root: PathBuf,
        provider: LocalTrialConfig,
    }

    fn parse(args: Vec<OsString>) -> Result<Arguments> {
        ensure!(args.len() == 11, USAGE);
        ensure!(
            args[0] == "--synthetic-only",
            "explicit --synthetic-only acknowledgement required; {USAGE}"
        );
        let request = PathBuf::from(&args[1]);
        let root = PathBuf::from(&args[2]);
        ensure!(root.is_absolute(), "trial directory must be absolute");
        let mut endpoint = None;
        let mut model = None;
        let mut holds_dir = None;
        let mut seed = None;
        for pair in args[3..].as_chunks::<2>().0 {
            match pair[0].to_str() {
                Some("--endpoint") if endpoint.is_none() => {
                    endpoint = Some(
                        pair[1]
                            .to_str()
                            .context("endpoint must be UTF-8")?
                            .to_owned(),
                    )
                }
                Some("--model") if model.is_none() => {
                    model = Some(pair[1].to_str().context("model must be UTF-8")?.to_owned())
                }
                Some("--holds-dir") if holds_dir.is_none() => {
                    holds_dir = Some(PathBuf::from(&pair[1]))
                }
                Some("--seed") if seed.is_none() => {
                    seed = Some(
                        pair[1]
                            .to_str()
                            .context("seed must be UTF-8")?
                            .parse::<u64>()
                            .context("seed must be U64")?,
                    )
                }
                _ => anyhow::bail!("unknown or duplicate argument; {USAGE}"),
            }
        }
        let holds_dir = holds_dir.context(USAGE)?;
        ensure!(holds_dir.is_absolute(), "holds directory must be absolute");
        Ok(Arguments {
            request,
            root,
            provider: LocalTrialConfig {
                endpoint: endpoint.context(USAGE)?,
                model: model.context(USAGE)?,
                holds_dir,
                seed: seed.context(USAGE)?,
            },
        })
    }

    // Starts before provider holds/resident checks, so setup cannot reset the
    // whole-trial budget. Drop aborts the timer on every exit, including errors.
    struct Deadline {
        started: Instant,
        expires: Instant,
        timer: tokio::task::JoinHandle<()>,
    }
    impl Deadline {
        fn start(duration: Duration, cancel: CancellationToken) -> Self {
            let started = Instant::now();
            let expires = started + duration;
            let timer = tokio::spawn(async move {
                tokio::time::sleep_until(tokio::time::Instant::from_std(expires)).await;
                cancel.cancel();
            });
            Self {
                started,
                expires,
                timer,
            }
        }
        fn expired(&self) -> bool {
            Instant::now() >= self.expires
        }
    }
    impl Drop for Deadline {
        fn drop(&mut self) {
            self.timer.abort();
        }
    }

    pub async fn run() -> Result<()> {
        let args = parse(std::env::args_os().skip(1).collect())?;
        ensure!(!args.root.exists(), "trial directory must be new");
        let request: TrialRequest = serde_json::from_slice(
            &std::fs::read(&args.request).context("reading synthetic request")?,
        )
        .context("parsing synthetic request")?;
        let cancel = CancellationToken::new();
        let signal_cancel = cancel.clone();
        let signal = tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                signal_cancel.cancel();
            }
        });
        let config = TrialConfig::default();
        let deadline = Deadline::start(Duration::from_secs(config.timeout_secs), cancel.clone());
        let result = async {
            let provider = LocalTrialProvider::new(args.provider, cancel.clone()).await?;
            run_trial(&args.root, request, Box::new(provider), config, cancel).await
        }
        .await;
        signal.abort();
        let mut report = result?;
        if deadline.expired() {
            report.status = "timeout".into();
            report.run_error =
                Some("whole trial deadline exceeded including provider setup".into());
        }
        report.elapsed_ms = deadline
            .started
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        // Host-only report, outside the tool jail; retain the whole-process
        // deadline classification and elapsed time alongside native records.
        std::fs::write(
            args.root.join("report.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        println!("{}", serde_json::to_string(&report)?);
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn args() -> Vec<OsString> {
            [
                "--synthetic-only",
                "/tmp/request.json",
                "/tmp/new-trial",
                "--endpoint",
                "http://127.0.0.1:8080",
                "--model",
                "trial-model",
                "--holds-dir",
                "/tmp/holds",
                "--seed",
                "42",
            ]
            .into_iter()
            .map(OsString::from)
            .collect()
        }
        #[tokio::test]
        async fn setup_and_execution_share_one_deadline() {
            let cancel = CancellationToken::new();
            let deadline = Deadline::start(Duration::from_millis(40), cancel.clone());
            tokio::time::sleep(Duration::from_millis(25)).await; // provider setup
            assert!(!cancel.is_cancelled());
            tokio::time::timeout(Duration::from_millis(30), cancel.cancelled())
                .await
                .unwrap();
            assert!(deadline.expired());
            assert!(deadline.started.elapsed() < Duration::from_millis(100));
        }
        #[test]
        fn explicit_arguments_are_required_and_preserved() {
            let parsed = parse(args()).unwrap();
            assert_eq!(parsed.provider.seed, 42);
            assert_eq!(parsed.provider.model, "trial-model");
            assert_eq!(parsed.provider.holds_dir, PathBuf::from("/tmp/holds"));
            for (index, bad) in [
                (0, "--production"),
                (2, "relative"),
                (3, "--unknown"),
                (7, "--model"),
                (8, "relative"),
                (10, "-1"),
            ] {
                let mut invalid = args();
                invalid[index] = bad.into();
                assert!(parse(invalid).is_err(), "accepted {index}: {bad}");
            }
            let mut invalid = args();
            invalid.push("--extra".into());
            assert!(parse(invalid).is_err());
        }
    }
}

#[cfg(feature = "memory-prototype")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    enabled::run().await
}

#[cfg(not(feature = "memory-prototype"))]
fn main() {
    eprintln!("This synthetic trial requires --features memory-prototype");
    std::process::exit(2);
}
