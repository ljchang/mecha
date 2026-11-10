//! Host operations over an explicitly selected ordinary-run memory authority.

use anyhow::{ensure, Result};
use mecha_core::memory_session::{
    learning::LEARNING_PROTOCOL, SessionMemory, SESSION_MEMORY_PROTOCOL,
};
use std::path::PathBuf;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Owner-selected request manifest.
    #[arg(long, value_name = "REQUEST_JSON")]
    pub request: PathBuf,
    #[command(subcommand)]
    pub cmd: Cmd,
}
#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Create a new encrypted store, private authority and key.
    Init,
    /// Retry capture from an existing finalized recording.
    Retry { run_id: String },
    /// Withdraw a capture and its retained dependents.
    Forget { run_id: String },
    /// Inspect bounded authenticated originals for host source selection.
    Sources {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long, required_unless_present = "run_id", conflicts_with = "run_id")]
        input: Option<PathBuf>,
        #[arg(long, required_unless_present = "input")]
        run_id: Option<String>,
    },
    /// Perform one explicit native induction or rumination call.
    Learn {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        holds_dir: PathBuf,
        #[arg(long)]
        seed: u64,
    },
}
pub async fn execute(args: Args) -> Result<()> {
    let request_path = std::path::absolute(args.request)?;
    let request = SessionMemory::load_request(&request_path)?;
    let mut failed = false;
    let report = match args.cmd {
        Cmd::Init => {
            SessionMemory::initialize(&request)?;
            serde_json::json!({"protocol":SESSION_MEMORY_PROTOCOL,"status":"initialized"})
        }
        Cmd::Retry { run_id } => {
            let memory = SessionMemory::open_for_recovery(&request)?;
            if memory.is_chat_capture(&run_id)? {
                serde_json::to_value(memory.retry_chat(&run_id))?
            } else {
                serde_json::to_value(memory.retry(&run_id))?
            }
        }
        Cmd::Forget { run_id } => {
            let memory = SessionMemory::open_for_recovery(&request)?;
            if memory.is_chat_capture(&run_id)? {
                serde_json::json!({"protocol":mecha_core::memory_session::chat::CHAT_MEMORY_PROTOCOL,"run_id":run_id,"status":"forgotten","report":memory.forget_chat(&run_id)?})
            } else {
                serde_json::to_value(memory.forget(&run_id)?)?
            }
        }
        Cmd::Sources {
            workspace,
            input,
            run_id,
        } => {
            let workspace = workspace.canonicalize()?;
            SessionMemory::validate_request_path(&request_path, &workspace)?;
            let memory = SessionMemory::open(&request, &workspace)?;
            let sources = if let Some(input) = input {
                memory.learning_sources(&SessionMemory::load_learning_selections(
                    &std::path::absolute(input)?,
                    &workspace,
                )?)?
            } else {
                memory.learning_run_sources(
                    run_id
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("source run required"))?,
                )?
            };
            serde_json::json!({"protocol":LEARNING_PROTOCOL,"sources":sources})
        }
        Cmd::Learn {
            workspace,
            input,
            endpoint,
            model,
            holds_dir,
            seed,
        } => {
            let workspace = workspace.canonicalize()?;
            SessionMemory::validate_request_path(&request_path, &workspace)?;
            let memory = SessionMemory::open(&request, &workspace)?;
            let input =
                SessionMemory::load_learning_input(&std::path::absolute(input)?, &workspace)?;
            // Constructing the shared transport does not discover/load a model.
            let client =
                super::memory::LocalMemoryModel::new(&endpoint, &model, holds_dir, seed, true)?;
            let report = memory
                .learn(input, |generation, guard| {
                    client.complete_learning(generation, guard)
                })
                .await;
            failed = report.status == "failed";
            let mut report = serde_json::to_value(report)?;
            report["endpoint"] = serde_json::json!(endpoint);
            report["model"] = serde_json::json!(model);
            report["seed"] = serde_json::json!(seed);
            report["usage_policy"] = serde_json::json!("positive_decoded_counts_only; zero_or_absent_is_unknown; use_recorded_wire_for_exact_presence");
            report["model_controls"] = serde_json::json!({"temperature":0.6,"max_tokens":8192,"thinking":false,"max_retries":0,"local_only":true,"structured_output":"llama_json","timeout_seconds":180});
            report
        }
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    ensure!(!failed, "ordinary learning failed; JSON report preserved");
    Ok(())
}
