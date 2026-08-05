//! Layered configuration.
//!
//! Later layers win, field by field:
//!   1. built-in defaults
//!   2. `~/.mecha/config.toml`
//!   3. `./mecha.toml` in the working directory (project-local)
//!   4. environment variables
//!   5. CLI flags (applied by the caller, not here)

use crate::message::Effort;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Which entry in `providers` to use when `--provider` isn't given.
    pub default_provider: String,
    pub providers: BTreeMap<String, ProviderConfig>,
    pub agent: AgentConfig,
    pub tools: ToolsConfig,
    pub security: SecurityConfig,
    /// How `shell` is confined. See [`crate::sandbox`].
    pub sandbox: crate::sandbox::SandboxConfig,
    /// MCP servers to connect to at startup.
    #[serde(rename = "mcp")]
    pub mcp: Vec<McpServerConfig>,
    /// Subagents the parent may delegate to, each exposed as one tool.
    #[serde(rename = "subagent")]
    pub subagents: Vec<crate::subagent::SubagentProfile>,
    /// Search backends, in preference order. The chain falls through on
    /// failure, which is what makes stacking two free tiers viable.
    #[serde(rename = "search")]
    pub search: Vec<SearchBackendConfig>,
    /// User commands run at loop lifecycle points. See [`crate::hooks`].
    #[serde(rename = "hook")]
    pub hooks: Vec<HookConfig>,
    /// Outbound tools staged for user review instead of executed. See
    /// [`crate::outbox`].
    pub outbox: OutboxConfig,
}

/// Which tools are outbox-routed, and where staged items live.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutboxConfig {
    /// Registry names (`email__send`, `web__fetch`). A call to one of these
    /// is staged as a draft the user reviews with `mecha outbox`; the tool
    /// itself never runs until they release it. Empty means the outbox is
    /// off, which is the default — routing a tool is a policy decision.
    pub tools: Vec<String>,
    /// Where items are staged. Defaults to `~/.mecha/outbox`
    /// (or `$MECHA_OUTBOX_DIR`).
    pub dir: Option<PathBuf>,
}

/// One hook: a command run at a lifecycle point, with the event payload as
/// JSON on stdin.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HookConfig {
    /// `pre_tool` | `post_tool` | `session_end`. An unknown event is a startup
    /// error, not a warning — a policy hook that never fires because its event
    /// name has a typo is the silently-degrading-sandbox mistake again.
    pub event: String,
    /// Run via `sh -c`, as the user, in the workspace.
    pub command: String,
    /// Only fire for these tools (`pre_tool`/`post_tool`). Empty means all.
    pub tools: Vec<String>,
    /// Kill the hook after this long. The default is deliberately short: a
    /// `pre_tool` hook is on the critical path of every call it matches.
    pub timeout_secs: Option<u64>,
}

impl Default for Config {
    fn default() -> Self {
        let mut providers = BTreeMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig {
                kind: "anthropic".to_string(),
                model: Some(crate::provider::anthropic::DEFAULT_MODEL.to_string()),
                api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
                api_key: None,
                base_url: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
                temperature: None,
                seed: None,
                context_window: None,
            },
        );
        Config {
            default_provider: "anthropic".to_string(),
            providers,
            agent: AgentConfig::default(),
            tools: ToolsConfig::default(),
            security: SecurityConfig::default(),
            sandbox: crate::sandbox::SandboxConfig::default(),
            mcp: Vec::new(),
            subagents: Vec::new(),
            search: Vec::new(),
            hooks: Vec::new(),
            outbox: OutboxConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderConfig {
    /// `anthropic` | `openai` | `local`
    pub kind: String,
    pub model: Option<String>,
    /// Environment variable holding the key. Preferred over `api_key`.
    pub api_key_env: Option<String>,
    /// Inline key. Convenient, but it lands in a file on disk — prefer the env var.
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    /// Per-million-token prices, so budgets and reporting can be in dollars.
    /// Leave unset for a local model — the marginal cost really is zero.
    pub input_price_per_mtok: Option<f64>,
    pub output_price_per_mtok: Option<f64>,
    /// Sampling temperature, sent verbatim by providers that accept one. Unset
    /// means the server's default. Do not reach for 0.0 to get repeatability:
    /// measured on qwen3.6, greedy decoding walks into verbatim repetition
    /// loops that sampling noise would have broken. Pin the server's own
    /// default value and set `seed` instead — same distribution, repeatable
    /// draws. The Anthropic API rejects the parameter, so setting this on an
    /// `anthropic` provider is a startup error rather than a silent no-op.
    pub temperature: Option<f64>,
    /// Sampling seed, for repeatable draws at a nonzero temperature. Only as
    /// deterministic as the backend: llama-server repeats exactly when requests
    /// run one at a time, and does not once concurrent requests share a batch.
    /// Rejected on `anthropic` for the same reason as `temperature`.
    pub seed: Option<u64>,
    /// How many tokens this model's context holds — for a local server, the
    /// `-c` it was started with.
    ///
    /// Nothing here can discover this: a provider reports how many tokens a
    /// prompt *used*, never how many are left. Without it the compaction
    /// threshold has to be an absolute number somebody remembers to set, and
    /// when nobody does, a long session dies on a raw
    /// `exceed_context_size_error` from the server with the whole run lost.
    /// With it, [`AgentConfig::compact_at`] derives a threshold and the CLI
    /// can show how much room is left.
    pub context_window: Option<u64>,
}

impl ProviderConfig {
    /// Prices, if configured. Both halves are required: knowing one is worse
    /// than knowing neither, because it silently under-reports.
    pub fn pricing(&self) -> Option<crate::message::Pricing> {
        match (self.input_price_per_mtok, self.output_price_per_mtok) {
            (Some(input), Some(output)) => Some(crate::message::Pricing {
                input_per_mtok: input,
                output_per_mtok: output,
                ..Default::default()
            }),
            _ => None,
        }
    }

    pub fn resolve_api_key(&self) -> Option<String> {
        if let Some(var) = &self.api_key_env {
            if let Ok(v) = std::env::var(var) {
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
        self.api_key.clone().filter(|k| !k.is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    pub system_prompt: Option<String>,
    /// Read the system prompt from a file. Wins over `system_prompt`.
    pub system_prompt_file: Option<PathBuf>,
    /// Hard stop on runaway loops: how many model turns one run may take.
    pub max_turns: u32,
    pub max_tokens: u32,
    pub effort: Option<Effort>,
    pub thinking: bool,
    /// Mark the tools + system prefix as cacheable.
    pub cache_prompt: bool,
    /// When the turn budget runs out, spend one more turn with the tools
    /// removed so the model has to answer with what it has. Without this a
    /// model that never stops searching returns nothing at all.
    pub force_final_answer: bool,
    /// Stop once this many output tokens have been generated in one run.
    /// `max_turns` bounds the number of round trips; this bounds their size,
    /// which is what actually runs up a bill.
    pub max_output_tokens: Option<u64>,
    /// Stop once one run has cost this much. Requires prices on the provider.
    pub max_cost_usd: Option<f64>,
    /// Summarise the middle of the conversation once the prompt passes this
    /// many tokens.
    ///
    /// Measured against what the provider *reported* for the last turn rather
    /// than an estimate, so it tracks the real prompt including cached tokens.
    /// Unset by default: compaction is lossy, and silently paraphrasing
    /// someone's conversation because it got long is a decision they should
    /// make. Set it to roughly two thirds of the model's context window — or
    /// set `context_window` on the provider and let
    /// [`AgentConfig::compact_at`] work it out.
    pub compact_at_tokens: Option<u64>,
    /// IANA timezone name for the user, e.g. `America/New_York`. Unset means
    /// the machine's. See [`AgentConfig::timezone`].
    pub timezone: Option<String>,
    /// Turns kept verbatim after a compaction. The recent ones are where the
    /// work is; a summary of the last two turns is worse than the turns.
    pub compact_keep_recent: usize,
    /// Check each summary against the transcript it replaces before
    /// installing it, and regenerate once with the omissions named.
    ///
    /// Summaries fail by *omission* — they preserve what is true and drop
    /// task-critical specifics — and the producer cannot see its own gaps.
    /// A separate grounded comparison can: it reads both texts side by side,
    /// which is a different task from generating either. Measured elsewhere
    /// (Slipstream) at +6.4–8.8 points on SWE-bench Verified for under 1%
    /// latency, with ~90% of catches being omissions. Costs one extra
    /// request per compaction, two when a regeneration is needed.
    pub compact_validate: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            system_prompt: None,
            system_prompt_file: None,
            max_turns: 40,
            // Streaming is the default, so there's no HTTP-timeout reason to
            // keep this small; leave room for thinking plus the answer.
            max_tokens: 64_000,
            effort: Some(Effort::High),
            thinking: true,
            cache_prompt: true,
            force_final_answer: true,
            // Unset by default: a ceiling that surprises you mid-task is worse
            // than no ceiling. Set them once you run things unattended.
            max_output_tokens: None,
            max_cost_usd: None,
            compact_at_tokens: None,
            timezone: None,
            compact_keep_recent: 6,
            compact_validate: true,
        }
    }
}

impl AgentConfig {
    /// The user's IANA timezone (`America/New_York`), when it is not the
    /// machine's.
    ///
    /// A server runs in UTC and the model has no clock, so without this every
    /// "what's on Thursday" is answered in the wrong zone — and wrongly in a
    /// way that looks right, since the times are internally consistent. An
    /// IANA name rather than an offset, because an offset is wrong twice a
    /// year.
    pub fn timezone(&self) -> Option<chrono_tz::Tz> {
        let name = self.timezone.as_deref()?;
        match name.parse::<chrono_tz::Tz>() {
            Ok(tz) => Some(tz),
            Err(_) => {
                tracing::warn!("unknown [agent] timezone `{name}`; using the machine's");
                None
            }
        }
    }

    /// Fraction of a known context window at which to start compacting.
    ///
    /// Two thirds, because the threshold is checked *between* turns against
    /// what the last one reported: the next turn still has to fit the model's
    /// reply, and a burst of parallel tool results can add several thousand
    /// tokens before anything gets to look again. Leaving a third of the
    /// window is what makes the reactive check safe.
    pub const COMPACT_FRACTION: f64 = 0.66;

    /// Where compaction kicks in for a run: the explicit setting if there is
    /// one, otherwise derived from the provider's context window.
    ///
    /// Deriving it is what turns compaction from something you must remember
    /// to configure into something that just works — and the failure it
    /// prevents is total, not gradual: one turn over the window and the
    /// server refuses the request outright.
    pub fn compact_at(&self, context_window: Option<u64>) -> Option<u64> {
        self.compact_at_tokens.or_else(|| {
            context_window.map(|w| (w as f64 * Self::COMPACT_FRACTION) as u64)
        })
    }

    pub fn resolve_system_prompt(&self) -> Result<Option<String>> {
        if let Some(path) = &self.system_prompt_file {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading system_prompt_file {}", path.display()))?;
            return Ok(Some(text));
        }
        Ok(self.system_prompt.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolsConfig {
    /// Built-in tools to register. Empty means "all of them".
    pub enabled: Vec<String>,
    /// Built-in tools to withhold, applied after `enabled`.
    pub disabled: Vec<String>,
    /// Filesystem tools refuse to touch anything outside this root.
    pub workspace: Option<PathBuf>,
    /// Default answer when nothing is watching to approve a call.
    pub permission_mode: PermissionMode,
    pub shell_timeout_secs: u64,
    /// The byte budget one turn's tool results share, divided across the
    /// batch. Oversized results are spilled to a file in full and cut in the
    /// transcript, with the marker naming the path and the line to resume
    /// from.
    pub output_budget_bytes: usize,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        ToolsConfig {
            enabled: Vec::new(),
            disabled: Vec::new(),
            workspace: None,
            permission_mode: PermissionMode::Ask,
            shell_timeout_secs: 120,
            output_budget_bytes: 24_000,
        }
    }
}

/// Defenses against the *lethal trifecta*: private data, untrusted content, and
/// a way to send data out. An agent holding all three can be turned into an
/// exfiltration tool by instructions hidden in the content it reads — a
/// calendar invite title, an email footer, a web page.
///
/// The mitigation is structural, not a filter: once both private data and
/// untrusted content have entered a conversation, refuse to let it send.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecurityConfig {
    pub trifecta: TrifectaPolicy,
    /// Refuse HTTP requests to loopback, private, and link-local addresses.
    /// Without this, `http_fetch` reaches your LAN and cloud metadata endpoints.
    pub block_private_ips: bool,
    /// If non-empty, HTTP requests may only go to these hosts (suffix match).
    pub allowed_domains: Vec<String>,
    /// Hosts that are always refused, checked before `allowed_domains`.
    pub blocked_domains: Vec<String>,
    /// Wrap third-party content in a marker telling the model to treat it as
    /// data rather than instructions. Weak on its own — defense in depth.
    pub mark_untrusted_output: bool,
    /// Block *every* outbound call once private data is in context, whether or
    /// not untrusted content has arrived.
    ///
    /// This is a different control from `trifecta`, guarding a different
    /// threat. The trifecta interlock stops an *injection* turning the agent
    /// into an exfiltration tool; it deliberately allows sends that happen
    /// before any third-party content exists, because nothing could have
    /// influenced them yet. That still lets the agent put your private data
    /// into a search query because you asked it to, or because it judged that
    /// helpful — an ordinary privacy leak rather than an attack.
    ///
    /// Turn this on when private data must not leave at all. It is
    /// restrictive: it makes "read my notes, then look something up" fail.
    pub block_sends_after_private: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        SecurityConfig {
            trifecta: TrifectaPolicy::Block,
            block_private_ips: true,
            allowed_domains: Vec::new(),
            blocked_domains: Vec::new(),
            mark_untrusted_output: true,
            // Off by default: it breaks common, legitimate workflows, and the
            // right answer for most people is capability separation (put
            // search in a subagent with no filesystem access) rather than a
            // blanket ban.
            block_sends_after_private: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrifectaPolicy {
    /// Refuse the send outright. The default.
    Block,
    /// Ask a human. Only meaningful when someone is watching.
    Ask,
    /// Allow it. Appropriate only when the "untrusted" content is in fact
    /// trusted — e.g. an allowlist of internal hosts.
    Allow,
}

/// Capabilities to force on a server's tools. Absent flags leave the server's
/// own declaration alone; there is deliberately no way to switch one off.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CapabilityOverride {
    pub private_data: bool,
    pub untrusted_input: bool,
    pub external_send: bool,
    pub destructive: bool,
}

impl From<CapabilityOverride> for crate::tool::Capabilities {
    fn from(o: CapabilityOverride) -> Self {
        crate::tool::Capabilities {
            private_data: o.private_data,
            untrusted_input: o.untrusted_input,
            external_send: o.external_send,
            destructive: o.destructive,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    /// Prompt before anything that isn't read-only.
    Ask,
    /// Run everything without asking. For trusted, headless work.
    Allow,
    /// Read-only tools run; everything else is refused.
    ReadOnly,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchBackendConfig {
    /// `exa` | `tavily` | `searxng`
    pub kind: String,
    /// Environment variable holding the key. Preferred over `api_key`.
    pub api_key_env: Option<String>,
    pub api_key: Option<String>,
    /// Required for `searxng` (your instance); optional override elsewhere.
    pub base_url: Option<String>,
    pub disabled: bool,
}

impl SearchBackendConfig {
    pub fn resolve_api_key(&self) -> Option<String> {
        if let Some(var) = &self.api_key_env {
            if let Ok(v) = std::env::var(var) {
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
        self.api_key.clone().filter(|k| !k.is_empty())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpServerConfig {
    /// Prefixed onto every tool the server exposes, so two servers can both
    /// have a `search` without colliding.
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// Values handed to the server explicitly. Use this for a token the server
    /// needs, so granting it is a decision written down rather than a
    /// side-effect of what happened to be exported.
    pub env: BTreeMap<String, String>,
    /// Variables inherited from mecha's own environment, by name.
    ///
    /// Empty by default, and that default is the point: an MCP server is
    /// third-party code, and a process that inherits your whole environment
    /// inherits every provider key in it. `PATH`, `HOME`, `LANG`, `LC_ALL` and
    /// `TZ` always pass through — without them most runtimes cannot start.
    pub env_passthrough: Vec<String>,
    /// Confine this server with the configured `[sandbox]` backend.
    ///
    /// Off by default because a confined server sees only the workspace and,
    /// unless allowed, no network — which is wrong for most of the servers
    /// people actually run. Worth turning on for anything you did not write.
    pub sandbox: bool,
    /// Network for this server alone, overriding `[sandbox] network`.
    ///
    /// The case this exists for: a third-party server that has to reach its own
    /// API, confined, while `shell` still has no way off the machine. With one
    /// shared switch you would have to open `shell` to satisfy the server.
    pub network: Option<bool>,
    /// Capabilities forced onto every tool this server exposes, on top of
    /// whatever it declares for itself.
    ///
    /// MCP capability flags come from the server's own `annotations`, which
    /// means a third-party server decides how much the interlock distrusts it.
    /// An unannotated tool is treated as private-but-trusted — wrong in the
    /// dangerous direction for anything that reaches the open world. A Google
    /// Docs server is the worked example: a document someone shared with you is
    /// third-party text, and writing into a document an attacker can read is an
    /// exfiltration channel, so it is all three legs at once and says none of
    /// them.
    ///
    /// Only ever widens — see [`crate::tool::Capabilities::union`].
    pub capabilities: CapabilityOverride,
    /// Skip this server without deleting its config.
    pub disabled: bool,
}

impl Config {
    pub fn global_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".mecha").join("config.toml"))
    }

    pub const PROJECT_FILE: &'static str = "mecha.toml";

    /// Load defaults, then the global file, then the project file, then env.
    pub fn load(project_dir: &Path) -> Result<Self> {
        let mut cfg = Config::default();

        if let Some(path) = Self::global_path() {
            if path.exists() {
                cfg.merge_file(&path)?;
            }
        }
        let project = project_dir.join(Self::PROJECT_FILE);
        if project.exists() {
            cfg.merge_file(&project)?;
        }
        cfg.merge_env();
        Ok(cfg)
    }

    fn merge_file(&mut self, path: &Path) -> Result<()> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let layer: ConfigLayer = toml::from_str(&text)
            .with_context(|| format!("parsing {}", path.display()))?;
        layer.apply(self);
        Ok(())
    }

    fn merge_env(&mut self) {
        if let Ok(v) = std::env::var("MECHA_PROVIDER") {
            self.default_provider = v;
        }
        if let Ok(v) = std::env::var("MECHA_MODEL") {
            let name = self.default_provider.clone();
            if let Some(p) = self.providers.get_mut(&name) {
                p.model = Some(v);
            }
        }
        if let Ok(v) = std::env::var("MECHA_EFFORT") {
            if let Ok(e) = v.parse() {
                self.agent.effort = Some(e);
            }
        }
    }

    pub fn provider(&self, name: Option<&str>) -> Result<(String, &ProviderConfig)> {
        let name = name.unwrap_or(&self.default_provider).to_string();
        let cfg = self.providers.get(&name).with_context(|| {
            format!(
                "no provider named {name:?}. Configured: {}",
                self.providers.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        Ok((name, cfg))
    }

    /// Write this config to `path`, creating parent directories.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

/// A partially-specified config file. Every field is optional so a project file
/// can override one setting without restating the rest.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigLayer {
    default_provider: Option<String>,
    providers: Option<BTreeMap<String, ProviderConfig>>,
    agent: Option<AgentLayer>,
    tools: Option<ToolsLayer>,
    security: Option<SecurityLayer>,
    #[serde(rename = "mcp")]
    mcp: Option<Vec<McpServerConfig>>,
    #[serde(rename = "subagent")]
    subagents: Option<Vec<crate::subagent::SubagentProfile>>,
    #[serde(rename = "search")]
    search: Option<Vec<SearchBackendConfig>>,
    #[serde(rename = "hook")]
    hooks: Option<Vec<HookConfig>>,
    sandbox: Option<SandboxLayer>,
    outbox: Option<OutboxLayer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutboxLayer {
    tools: Option<Vec<String>>,
    dir: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentLayer {
    system_prompt: Option<String>,
    system_prompt_file: Option<PathBuf>,
    max_turns: Option<u32>,
    max_tokens: Option<u32>,
    effort: Option<Effort>,
    thinking: Option<bool>,
    cache_prompt: Option<bool>,
    force_final_answer: Option<bool>,
    max_output_tokens: Option<u64>,
    max_cost_usd: Option<f64>,
    compact_at_tokens: Option<u64>,
    compact_keep_recent: Option<usize>,
    compact_validate: Option<bool>,
    timezone: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecurityLayer {
    trifecta: Option<TrifectaPolicy>,
    block_private_ips: Option<bool>,
    allowed_domains: Option<Vec<String>>,
    blocked_domains: Option<Vec<String>>,
    mark_untrusted_output: Option<bool>,
    block_sends_after_private: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SandboxLayer {
    kind: Option<crate::sandbox::Backend>,
    network: Option<bool>,
    writable: Option<Vec<PathBuf>>,
    readable: Option<Vec<PathBuf>>,
    env: Option<Vec<String>>,
    image: Option<String>,
    memory_mb: Option<u64>,
    cpus: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolsLayer {
    enabled: Option<Vec<String>>,
    disabled: Option<Vec<String>>,
    workspace: Option<PathBuf>,
    permission_mode: Option<PermissionMode>,
    shell_timeout_secs: Option<u64>,
    output_budget_bytes: Option<usize>,
}

impl ConfigLayer {
    fn apply(self, cfg: &mut Config) {
        if let Some(v) = self.default_provider {
            cfg.default_provider = v;
        }
        // Providers merge by key so a project file can add a local endpoint
        // without redeclaring the Anthropic one.
        if let Some(providers) = self.providers {
            cfg.providers.extend(providers);
        }
        if let Some(a) = self.agent {
            let t = &mut cfg.agent;
            if a.system_prompt.is_some() {
                t.system_prompt = a.system_prompt;
            }
            if a.system_prompt_file.is_some() {
                t.system_prompt_file = a.system_prompt_file;
            }
            if let Some(v) = a.max_turns {
                t.max_turns = v;
            }
            if let Some(v) = a.max_tokens {
                t.max_tokens = v;
            }
            if a.effort.is_some() {
                t.effort = a.effort;
            }
            if let Some(v) = a.thinking {
                t.thinking = v;
            }
            if let Some(v) = a.cache_prompt {
                t.cache_prompt = v;
            }
            if let Some(v) = a.force_final_answer {
                t.force_final_answer = v;
            }
            if a.max_output_tokens.is_some() {
                t.max_output_tokens = a.max_output_tokens;
            }
            if a.max_cost_usd.is_some() {
                t.max_cost_usd = a.max_cost_usd;
            }
            if a.compact_at_tokens.is_some() {
                t.compact_at_tokens = a.compact_at_tokens;
            }
            if let Some(v) = a.compact_keep_recent {
                t.compact_keep_recent = v;
            }
            if let Some(v) = a.compact_validate {
                t.compact_validate = v;
            }
            if a.timezone.is_some() {
                t.timezone = a.timezone;
            }
        }
        if let Some(x) = self.tools {
            let t = &mut cfg.tools;
            if let Some(v) = x.enabled {
                t.enabled = v;
            }
            if let Some(v) = x.disabled {
                t.disabled = v;
            }
            if x.workspace.is_some() {
                t.workspace = x.workspace;
            }
            if let Some(v) = x.permission_mode {
                t.permission_mode = v;
            }
            if let Some(v) = x.shell_timeout_secs {
                t.shell_timeout_secs = v;
            }
            if let Some(v) = x.output_budget_bytes {
                t.output_budget_bytes = v;
            }
        }
        if let Some(x) = self.security {
            let t = &mut cfg.security;
            if let Some(v) = x.trifecta {
                t.trifecta = v;
            }
            if let Some(v) = x.block_private_ips {
                t.block_private_ips = v;
            }
            if let Some(v) = x.allowed_domains {
                t.allowed_domains = v;
            }
            if let Some(v) = x.blocked_domains {
                t.blocked_domains = v;
            }
            if let Some(v) = x.mark_untrusted_output {
                t.mark_untrusted_output = v;
            }
            if let Some(v) = x.block_sends_after_private {
                t.block_sends_after_private = v;
            }
        }
        if let Some(x) = self.sandbox {
            let t = &mut cfg.sandbox;
            if let Some(v) = x.kind {
                t.kind = v;
            }
            if let Some(v) = x.network {
                t.network = v;
            }
            if let Some(v) = x.writable {
                t.writable = v;
            }
            if let Some(v) = x.readable {
                t.readable = v;
            }
            if let Some(v) = x.env {
                t.env = v;
            }
            if let Some(v) = x.image {
                t.image = v;
            }
            if x.memory_mb.is_some() {
                t.memory_mb = x.memory_mb;
            }
            if x.cpus.is_some() {
                t.cpus = x.cpus;
            }
        }
        // MCP servers replace wholesale — merging lists by name would make it
        // impossible for a project to turn a global server off.
        if let Some(v) = self.mcp {
            cfg.mcp = v;
        }
        if let Some(v) = self.subagents {
            cfg.subagents = v;
        }
        if let Some(v) = self.search {
            cfg.search = v;
        }
        // Wholesale, like MCP servers and for the same reason: a project that
        // cannot turn a global hook off cannot be trusted to run anything.
        if let Some(v) = self.hooks {
            cfg.hooks = v;
        }
        if let Some(x) = self.outbox {
            let t = &mut cfg.outbox;
            // Wholesale: a project must be able to un-route a tool the global
            // config routes, and vice versa.
            if let Some(v) = x.tools {
                t.tools = v;
            }
            if x.dir.is_some() {
                t.dir = x.dir;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_overrides_only_named_fields() {
        let mut cfg = Config::default();
        let layer: ConfigLayer = toml::from_str(
            r#"
            [agent]
            max_turns = 5
            "#,
        )
        .unwrap();
        layer.apply(&mut cfg);
        assert_eq!(cfg.agent.max_turns, 5);
        // Untouched fields keep their defaults.
        assert_eq!(cfg.agent.max_tokens, 64_000);
        assert_eq!(cfg.default_provider, "anthropic");
    }

    #[test]
    fn providers_merge_by_key() {
        let mut cfg = Config::default();
        let layer: ConfigLayer = toml::from_str(
            r#"
            [providers.local]
            kind = "local"
            base_url = "http://127.0.0.1:8080"
            "#,
        )
        .unwrap();
        layer.apply(&mut cfg);
        assert!(cfg.providers.contains_key("anthropic"));
        assert!(cfg.providers.contains_key("local"));
    }

    #[test]
    fn hooks_configure_from_a_file() {
        let mut cfg = Config::default();
        let layer: ConfigLayer = toml::from_str(
            r#"
            [[hook]]
            event = "pre_tool"
            tools = ["shell"]
            command = "policy.sh"
            "#,
        )
        .unwrap();
        layer.apply(&mut cfg);
        assert_eq!(cfg.hooks.len(), 1);
        assert_eq!(cfg.hooks[0].event, "pre_tool");
        assert_eq!(cfg.hooks[0].tools, ["shell"]);
    }

    /// An explicit threshold always wins; otherwise a known window derives
    /// one. The derived value must leave real headroom — the check happens
    /// *between* turns, so the next request has to fit the reply and whatever
    /// a burst of parallel tool results adds.
    #[test]
    fn the_compaction_threshold_derives_from_a_known_context_window() {
        let mut cfg = AgentConfig::default();
        assert_eq!(cfg.compact_at(None), None, "unknowable stays unset");

        // The DGX's llama-server runs -c 32768; two thirds of that.
        let derived = cfg.compact_at(Some(32768)).unwrap();
        assert_eq!(derived, 21626);
        assert!(
            derived < 32768 - 8192,
            "must leave room for a reply and a burst of tool results: {derived}"
        );

        cfg.compact_at_tokens = Some(9000);
        assert_eq!(cfg.compact_at(Some(32768)), Some(9000), "explicit wins");
    }

    #[test]
    fn every_field_of_config_is_reachable_from_a_file() {
        // The bug this exists for: `hooks` was added to `Config` and not to
        // `ConfigLayer`, so `[[hook]]` in any config file was a hard parse
        // error and the whole feature was unreachable — while every unit test
        // passed, because they all built the type directly.
        //
        // Serialising the default config produces one entry per top-level
        // field; `ConfigLayer` denies unknown fields, so parsing it back is a
        // standing check that the two structs still agree. Any field added to
        // one and not the other fails here rather than in someone's config.
        let rendered = toml::to_string(&Config::default()).unwrap();
        let parsed = toml::from_str::<ConfigLayer>(&rendered);
        assert!(parsed.is_ok(), "Config has a field ConfigLayer cannot read: {parsed:?}");
    }
}
