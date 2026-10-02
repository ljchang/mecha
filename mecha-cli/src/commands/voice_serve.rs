//! `mecha voice-serve` — thin over `crate::voice`, on the slack/tui rule:
//! the command module carries the args, the sibling module the logic.

use crate::GlobalOpts;
use anyhow::Result;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Port on 127.0.0.1. The bind address is deliberately not a flag:
    /// this surface is loopback-only (docs/VOICE-RESEARCH.md, D2).
    #[arg(long, default_value_t = mecha_core::config::VoiceConfig::DEFAULT_VOICE_PORT)]
    pub port: u16,

    /// Require this bearer token on every request. The loopback bind is
    /// the boundary; this is one header of defence against other local
    /// processes.
    #[arg(long)]
    pub token: Option<String>,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    super::features::require(mecha_core::feature::Feature::Voice)?;
    crate::voice::run(global, args).await
}
