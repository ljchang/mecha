//! `mecha config` — see what settings are actually in effect.

use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::config::Config;
use std::path::PathBuf;

#[derive(clap::Subcommand, Debug)]
pub enum Args {
    /// Print the merged configuration as TOML.
    Show,

    /// Print the files that are being read, and whether they exist.
    Path,

    /// Write a starter config file.
    Init {
        /// Write `./mecha.toml` instead of `~/.mecha/config.toml`.
        #[arg(long)]
        project: bool,

        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },
}

pub async fn execute(_global: &GlobalOpts, args: Args) -> Result<()> {
    let cwd = std::env::current_dir()?;

    match args {
        Args::Show => {
            let cfg = Config::load(&cwd)?;
            print!("{}", toml::to_string_pretty(&cfg)?);
        }

        Args::Path => {
            let project = cwd.join(Config::PROJECT_FILE);
            for path in [Config::global_path(), Some(project)].into_iter().flatten() {
                let state = if path.exists() { "found" } else { "absent" };
                println!("{state}  {}", path.display());
            }
        }

        Args::Init { project, force } => {
            let path: PathBuf = if project {
                cwd.join(Config::PROJECT_FILE)
            } else {
                Config::global_path().context("cannot determine the home directory")?
            };

            anyhow::ensure!(
                force || !path.exists(),
                "{} already exists (pass --force to overwrite)",
                path.display()
            );

            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, STARTER)
                .with_context(|| format!("writing {}", path.display()))?;
            println!("wrote {}", path.display());
        }
    }

    Ok(())
}

/// A commented starting point rather than a dump of defaults — the point of the
/// file is to show what's adjustable.
const STARTER: &str = r#"# mecha configuration.
#
# Layered: ~/.mecha/config.toml, then ./mecha.toml, then MECHA_* environment
# variables, then CLI flags. Each layer overrides only the fields it names.

default_provider = "anthropic"

[providers.anthropic]
kind = "anthropic"
model = "claude-opus-5"
api_key_env = "ANTHROPIC_API_KEY"
# Needed for cost budgets and reporting; omit for a local model.
# input_price_per_mtok = 5.0
# output_price_per_mtok = 25.0

# A local OpenAI-compatible server (llama-server, vLLM, Ollama).
#
# Do not type the last three by hand — `mecha setup` reads them off the
# server's own /props and `mecha setup --write` fills them in. Each is a
# setting nothing can check afterwards, because each degrades quietly rather
# than failing.
# [providers.local]
# kind = "local"
# base_url = "http://127.0.0.1:8080"
# model = "qwen3-14b"          # what /props reports as model_alias
# context_window = 32768       # the PER-SLOT window, not `-c`: llama-server
#                              # divides -c across slots and recent builds
#                              # default to more than one. Four things derive
#                              # from this and all four degrade in silence.
# vision = true                # only if a projector is loaded. A multimodal
#                              # model served without --mmproj reports
#                              # vision:false and simply says it cannot see.

[agent]
# system_prompt_file = "prompts/agent.md"
max_turns = 40
max_tokens = 64000
# Ceilings on size, not just round trips. Unset by default.
# max_output_tokens = 20000
# max_cost_usd = 0.50
effort = "high"     # low | medium | high | xhigh | max
thinking = true
cache_prompt = true

[tools]
# Empty `enabled` means every built-in: fs_read, fs_write, fs_edit, fs_list,
# shell, http_fetch.
enabled = []
disabled = []
permission_mode = "ask"   # ask | allow | read-only
shell_timeout_secs = 120

[security]
# The lethal trifecta: private data + untrusted content + a way to send.
# Once a conversation holds the first two, outbound tools are refused —
# text hidden in third-party content could be directing the exfiltration.
#   block     refuse the send (default)
#   ask       escalate to a human
#   allow     permit it (only when the "untrusted" source is actually trusted)
trifecta = "block"

# Refuse HTTP to loopback, private, link-local, and CGNAT addresses. Without
# this, http_fetch reaches your LAN and cloud metadata endpoints.
block_private_ips = true

# allowed_domains = ["docs.rs", "arxiv.org"]   # if set, nothing else is fetched
# blocked_domains = []

# Wrap third-party content so the model treats it as data, not instructions.
mark_untrusted_output = true

# Block EVERY outbound call once private data is in context, injection or not.
# A different control from `trifecta`: that one stops an injection driving
# exfiltration, this one stops private data leaving at all. Off by default —
# it breaks "read my notes, then look something up", and capability separation
# (search in a subagent with no filesystem access) is usually the better fix.
# block_sends_after_private = false

# Search backends, in preference order; the chain falls through on failure.
# [[search]]
# kind = "searxng"                    # self-hosted: no key, no quota
# base_url = "http://127.0.0.1:8888"
# [[search]]
# kind = "exa"                        # ~1,400 searches/mo free
# api_key_env = "EXA_API_KEY"
# [[search]]
# kind = "tavily"                     # 1,000 credits/mo free
# api_key_env = "TAVILY_API_KEY"

# Subagents. Each becomes one tool on the parent. `tools` is an allowlist, not
# an inheritance — this is where capability isolation is expressed.
# [[subagent]]
# name = "read_web"
# description = "Fetch a URL and return a factual summary. Use this rather than
#                fetching directly when private data is already in context."
# tools = ["http_fetch"]
# max_turns = 6
# model = "gemma-4-4b"     # optional: a cheap model for a narrow job
# provider = "local-small" # optional: a different server entirely

# MCP servers. Their tools appear as `<name>__<tool>`.
# [[mcp]]
# name = "graph"
# command = "/path/to/pkg-mcp"
# args = []
"#;
