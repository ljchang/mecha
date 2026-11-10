//! Host operations over an explicitly selected ordinary-run memory authority.

use anyhow::Result;
use mecha_core::memory_session::{SessionMemory, SESSION_MEMORY_PROTOCOL};
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
}

pub fn execute(args: Args) -> Result<()> {
    let request = SessionMemory::load_request(&std::path::absolute(args.request)?)?;
    let report = match args.cmd {
        Cmd::Init => {
            SessionMemory::initialize(&request)?;
            serde_json::json!({"protocol": SESSION_MEMORY_PROTOCOL, "status": "initialized"})
        }
        Cmd::Retry { run_id } => {
            let memory = SessionMemory::open_for_recovery(&request)?;
            serde_json::to_value(memory.retry(&run_id))?
        }
        Cmd::Forget { run_id } => {
            let memory = SessionMemory::open_for_recovery(&request)?;
            serde_json::to_value(memory.forget(&run_id)?)?
        }
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
