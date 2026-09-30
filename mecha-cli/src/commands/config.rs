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
            // `[features]` in the global file only: a project layer is
            // stripped of it, so written there it would be a warning on every
            // load and a switch that does nothing.
            let text = if project {
                STARTER.to_string()
            } else {
                format!("{STARTER}{FEATURES_STARTER}")
            };
            std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
            println!("wrote {}", path.display());
        }
    }

    Ok(())
}

/// Every optional feature, listed and off — the light install, and the list
/// a new user reads to learn what exists (FEATURES-DESIGN.md §5, ruling F1).
/// `mecha features enable <id>` flips one in place; `mecha features` says
/// what each needs besides its switch.
pub const FEATURES_STARTER: &str = r#"
# Optional features. Every one is off until switched on here or with
# `mecha features enable <id>`; `mecha features` shows what each still needs.
# Global file only: a project's mecha.toml cannot switch a feature on.
[features]
web = false        # the web app (`mecha serve`)
slack = false      # Slack remote control
mail = false       # mail and calendar
docs = false       # Google Docs, Sheets and Slides
graph = false      # the knowledge graph and the task board
search = false     # web search and open
documents = false  # PDF extraction (OCR and layout are [documents] settings)
image = false      # image generation and the character library
personas = false   # characters you write and talk to
voice = false      # dictation and voice calls
incognito = false  # a web chat that leaves no trace
frontdoor = false  # inbound requests, polls and publishing
messages = false   # messages between sessions on this machine
"#;

/// A commented starting point rather than a dump of defaults — the point of the
/// file is to show what's adjustable.
pub const STARTER: &str = r#"# mecha configuration.
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
#
# "A way to send" means a destination the MODEL can name: http_fetch's url,
# mail_send's to, a Slack channel, an unconfined shell. web_search is not one
# — its input schema has no destination field, so the query reaches the
# [[search]] backends below and nobody else, and an injection that fills it
# has no way to read it back. Searching therefore keeps working in a
# conversation that holds mail, calendar or graph data. See docs/TRIFECTA.md.
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
# exfiltration, this one stops private data leaving at all — web_search
# included, which `trifecta` deliberately leaves alone. Turn it on when no
# private data may reach a third party even at your own request. Off by
# default: it breaks "read my notes, then look something up".
# block_sends_after_private = false

# PDFs: the `document_read` tool and `mecha document extract`. Each page's
# own text layer (exact — what a quote is checked against) and, for scans or
# when asked, a local OCR model's Markdown transcript. poppler runs confined
# (bwrap by default; a confinement that cannot run fails the extraction, it
# never falls back). The OCR server must be on this machine — page images of
# your documents go to it. Global file only. See
# docs/DOCUMENT-EXTRACTION-DESIGN.md; scripts/llama/install.sh installs the
# on-demand OCR server this points at.
# [documents]
# ocr = true
# ocr_url = "http://127.0.0.1:8085"    # llama-ocr.socket; starts on first use
# ocr_model = "paddleocr-vl-1.6"
# confine = "bwrap"                     # bwrap | landlock | none
# max_file_mb = 100
# max_pages = 2000
# max_ocr_pages = 30                    # per call; the rest are named, not lost
# cache_days = 30                       # ~/.mecha/documents, by content hash
# layout = true                         # read OCR pages region by region (tables);
#                                       # scripts/layout/install.sh installs it
# layout_python = "~/.mecha/layout/venv/bin/python"
# layout_model = "~/.mecha/layout/PP-DocLayoutV3.onnx"

# Search backends, in preference order; the chain falls through on failure.
# A conversation holding private data and third-party content is served only
# by backends whose destination your config fixes and whose query is search
# terms and nothing else — searxng and tavily at any depth, exa at quick
# depth (its deep mode is agentic research that fetches pages the query can
# steer it towards). Configure at least one, or web_search is refused there.
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
# command = "mecha-graph-mcp"   # or an absolute path: a service unit without
#                               # ~/.cargo/bin on its PATH will not find the
#                               # bare name; front-ends then skip the server
#                               # with one stderr line (the tools just vanish),
#                               # distill/corroborate/gossip/vet exit non-zero.
#                               # Not confined: `sandbox = true` replaces PATH
#                               # with the system dirs and binds nothing under ~
#                               # unless listed, and this server's store lives
#                               # there, read-write.
# args = []
# # Its kg_* tools carry their own namespace; skip the graph__ prefix.
# prefix_tools = false
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::feature::Feature;

    /// The starter lists every feature with a switch, all off, and nothing
    /// else — so a new feature does not ship missing from the list a new user
    /// reads to learn what exists, and the file still loads.
    #[test]
    fn the_global_starter_lists_every_switch_off() {
        let text = format!("{STARTER}{FEATURES_STARTER}");
        let cfg: Config = toml::from_str(&text).expect("the starter loads");
        let table: toml::Table = toml::from_str(&text).unwrap();
        let listed: Vec<&str> = table["features"]
            .as_table()
            .unwrap()
            .iter()
            .map(|(k, v)| {
                assert_eq!(v.as_bool(), Some(false), "`{k}` ships off");
                k.as_str()
            })
            .collect();
        let mut switches: Vec<&str> = Feature::ALL
            .iter()
            .filter(|f| f.has_switch())
            .map(|f| f.id())
            .collect();
        let mut listed_sorted = listed.clone();
        listed_sorted.sort();
        switches.sort();
        assert_eq!(listed_sorted, switches);
        assert!(!cfg.messages.enabled);
        assert!(cfg.features.0.values().all(|on| !on));
    }
}
