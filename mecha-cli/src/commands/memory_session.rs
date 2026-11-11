//! Host operations over a memory store: the mecha home's
//! (`mecha_core::memory_home`) unless `--request` names a manifest.

use anyhow::{ensure, Result};
use mecha_core::memory_session::{
    learning::LEARNING_PROTOCOL, SessionMemory, SESSION_MEMORY_PROTOCOL,
};
use std::path::PathBuf;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// A request manifest naming another store. Without it, the store under
    /// the mecha home (`~/.mecha/memory`).
    #[arg(long, value_name = "REQUEST_JSON")]
    pub request: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: Cmd,
}
#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Create a new encrypted store, private authority and key. Refuses a
    /// store that already exists, whole or partial.
    Init,
    /// Explicitly upgrade live encrypted ordinary storage to Task/Plan schema v10.
    EnablePlanning,
    /// Complete only this exact reserved task startup; never execute a model or tool.
    RetryTaskStart {
        session_id: String,
        #[arg(long, value_name = "TASK_JSON")]
        task: PathBuf,
    },
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
    // A manifest is a file a task could have written, so it is held to the
    // workspace rule below; the home store's request is built in code and has
    // no file to check.
    let request_path = args.request.map(std::path::absolute).transpose()?;
    // The home is asked for only when it is the store: a manifest needs none.
    let request = match &request_path {
        Some(path) => SessionMemory::load_request(path)?,
        None => mecha_core::memory_home::request(&mecha_core::work::mecha_home()?)?,
    };
    let manifest_outside = |workspace: &std::path::Path| match &request_path {
        Some(path) => SessionMemory::validate_request_path(path, workspace),
        None => Ok(()),
    };
    let mut failed = false;
    let report = match args.cmd {
        Cmd::Init => {
            // The manifest report is a wire format its callers compare whole;
            // the home store's adds where the store is, since no manifest said.
            match &request_path {
                Some(_) => {
                    SessionMemory::initialize(&request)?;
                    serde_json::json!({"protocol":SESSION_MEMORY_PROTOCOL,"status":"initialized"})
                }
                None => {
                    let made =
                        mecha_core::memory_home::initialize(&mecha_core::work::mecha_home()?)?;
                    serde_json::json!({"protocol":SESSION_MEMORY_PROTOCOL,"status":"initialized","store":made.authority.parent()})
                }
            }
        }
        Cmd::EnablePlanning => {
            let memory = SessionMemory::open_for_recovery(&request)?;
            let version = memory.enable_planning()?;
            serde_json::json!({"protocol":SESSION_MEMORY_PROTOCOL,"status":"planning_enabled","schema_version":version})
        }
        Cmd::RetryTaskStart { session_id, task } => {
            let memory = SessionMemory::open_for_recovery(&request)?;
            memory.retry_task_start_file(&session_id, &std::path::absolute(task)?)?;
            serde_json::json!({"protocol":mecha_core::memory_session::chat::CHAT_TASK_MEMORY_PROTOCOL,"status":"attached","conversation_id":session_id})
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
            manifest_outside(&workspace)?;
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
            manifest_outside(&workspace)?;
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
