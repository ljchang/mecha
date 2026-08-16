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
    /// Retention for `~/.mecha/work/`. See [`crate::work`].
    pub work: WorkConfig,
    /// Tunables for `mecha slack`. Global-file only; see [`SlackConfig`].
    pub slack: SlackConfig,
    /// Inter-agent messages between mecha sessions on this machine. See
    /// [`crate::mailbox`].
    pub messages: MessagesConfig,
}

/// Messaging between this machine's own mecha sessions.
///
/// Receiver-side policy, so it loads from the global file only, never a
/// project's `mecha.toml`: a cloned repository must not be able to set
/// `inbound = "accept"` on someone's session. Enforced structurally:
/// `merge_file` strips the section from project layers, loudly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MessagesConfig {
    /// Off by default, like outbox routing: a mailbox is a policy decision.
    pub enabled: bool,
    /// Where messages live. Defaults to `~/.mecha/messages`
    /// (or `$MECHA_MESSAGES_DIR`).
    pub dir: Option<PathBuf>,
    /// What a run does with inbound messages: `accept` folds them in at turn
    /// boundaries, `hold` leaves them for `mecha msg`. Unset — the default —
    /// resolves per surface: attended front-ends hold, unattended runs
    /// accept. See [`crate::mailbox::InboundPolicy`].
    pub inbound: Option<crate::mailbox::InboundPolicy>,
    /// Pending messages one recipient may hold before senders are refused.
    pub pending_cap: usize,
    /// Largest message body, in bytes.
    pub max_body_bytes: usize,
    /// Resolved (delivered/dismissed) messages kept per recipient before the
    /// oldest are pruned. Retention, so the per-turn claim scan stays bounded.
    pub keep: usize,
}

impl Default for MessagesConfig {
    fn default() -> Self {
        MessagesConfig {
            enabled: false,
            dir: None,
            inbound: None,
            pending_cap: crate::mailbox::DEFAULT_PENDING_CAP,
            max_body_bytes: crate::mailbox::DEFAULT_MAX_BODY_BYTES,
            keep: crate::mailbox::DEFAULT_KEEP_RESOLVED,
        }
    }
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
    /// Which of the routed names are *publications* rather than messages
    /// (`factory__bundle_publish`, `factory__bundle_alias`). They stage
    /// identically; they are **reviewed** differently — the reviewable object
    /// is the rendered page, `edit` is refused, and the writing-reflection
    /// miner skips them so a changed directory path never becomes a voice
    /// rule. See [`crate::outbox::OutboxKind`].
    ///
    /// Config's to declare, not the tool's: the loop must not learn what a
    /// publish is, and a third-party MCP server cannot be trusted to say.
    pub publish_tools: Vec<String>,
}

/// How much of a producer's generated output survives a `mecha work clean`.
///
/// A policy rather than an intention: the lesson of this project is that
/// anything without one becomes a pile nobody opens. The number is small on
/// purpose — the directory is scratch, and what matters is published.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkConfig {
    /// Entries kept per producer, newest first.
    pub keep: usize,
}

/// `[slack]` — tunables for the Slack remote control. **Nothing here grants
/// anything.** Who may drive the agent lives in `~/.mecha/slack/binding.json`,
/// a store rather than config, for the reason `[messages]` is global-only and
/// then some: a project file arrives with a cloned repository, and a repo that
/// could name a Slack owner would have been handed the remote control.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SlackConfig {
    /// Threads that may have a run in flight at once. At the cap the connector
    /// refuses and says so, rather than queueing: a run that starts twenty
    /// minutes later against a workspace that has moved is worse than an
    /// honest refusal.
    pub max_concurrent: usize,
    /// How long an approval card waits before the call is refused as
    /// unanswered. Never a denial by the user — see `Decision::Blocked`.
    pub approval_timeout_secs: u64,
    /// `ask` (the default), `allow`, or `read-only`, for a thread nobody has
    /// set a mode on.
    pub default_mode: String,
    pub max_turns: u32,
    pub max_cost_usd: Option<f64>,
    /// Flush a streamed chunk once this much text has accumulated, or this
    /// long has passed — whichever comes first. Size first is Slack's own
    /// guidance; the timer is so a slow model still shows progress.
    pub stream_flush_chars: usize,
    pub stream_flush_ms: u64,
    /// Largest attachment fetched into a run's workspace. Slack allows 1 GB;
    /// a remote control does not need to.
    pub max_upload_mb: u64,
    /// Narrow the tool surface for Slack-driven runs.
    ///
    /// Empty means "everything configured", which is the default and is
    /// usually too much: measured on the first live run, the schemas of every
    /// wired MCP server cost ~7–8k input tokens *per turn* before any work
    /// happened — against a 32k window whose compaction threshold is 21,845,
    /// a run starts a third of the way there. A phone rarely needs the mail
    /// and the calendar and the factory at once, and naming what it does need
    /// is the cheapest context this system has to give.
    pub tools: Vec<String>,
}

impl Default for SlackConfig {
    fn default() -> Self {
        SlackConfig {
            max_concurrent: 3,
            approval_timeout_secs: 600,
            default_mode: "ask".into(),
            max_turns: 40,
            max_cost_usd: None,
            stream_flush_chars: 800,
            stream_flush_ms: 1000,
            max_upload_mb: 25,
            tools: Vec::new(),
        }
    }
}

impl Default for WorkConfig {
    fn default() -> Self {
        WorkConfig {
            keep: crate::work::DEFAULT_KEEP,
        }
    }
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
                max_retries: None,
                retry_after_cap_secs: None,
                fallbacks: Vec::new(),
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
            work: WorkConfig::default(),
            slack: SlackConfig::default(),
            messages: MessagesConfig::default(),
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
    /// Retries per request on transient failures — 429, 5xx, transport. 0
    /// disables. Unset means 3. Auth, billing, invalid-request and
    /// context-overflow errors are never retried: the same payload fails the
    /// same way, and overflow belongs to the compaction path.
    pub max_retries: Option<u32>,
    /// A `Retry-After` above this many seconds is surfaced as a failure
    /// instead of slept through (default 60) — a provider can name a wait
    /// long enough that the process is simply asleep, and control never
    /// returns to a layer that could fall back instead.
    pub retry_after_cap_secs: Option<u64>,
    /// Provider entries to try, in order, when this one exhausts its retries
    /// on a *transient* failure. Turn-local: the next turn starts from this
    /// provider again. Each fallback answers with its own model. Empty —
    /// the default — means strict: fail rather than silently answer with a
    /// different model. `mecha eval` never falls back regardless: a
    /// scorecard grades the model it names.
    pub fallbacks: Vec<String>,
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
    /// Stop a run that repeats an identical tool call, with an identical
    /// result, right after a compaction (`StopCause::Loop`).
    ///
    /// On by default — the asymmetry is deliberate. A general repeated-call
    /// detector would need a measurement to justify watching all of ordinary
    /// work; this one exists to escape the specific loop that burns unbounded
    /// tokens at the largest prompts a run will ever send, and a no-config
    /// user should get that protection. Identical arguments with a *changing*
    /// result is polling and never trips it.
    pub loop_guard: bool,
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
            loop_guard: true,
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
        self.compact_at_tokens
            .or_else(|| context_window.map(|w| (w as f64 * Self::COMPACT_FRACTION) as u64))
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
    /// from. Unset means derive it from the provider's context window — see
    /// [`ToolsConfig::resolved_output_budget`].
    pub output_budget_bytes: Option<usize>,
}

impl ToolsConfig {
    /// Ceiling when nothing pins the budget: right for the wide-window
    /// frontier models the number was originally chosen against.
    const OUTPUT_BUDGET_MAX: usize = 24_000;
    /// Floor: below this, a single `cargo build` error listing stops fitting
    /// and every result arrives pre-truncated — a budget that starves the
    /// model of its own results is worse than a tight window.
    const OUTPUT_BUDGET_MIN: usize = 6_000;

    /// The per-turn tool-output budget, window-proportional when unpinned.
    ///
    /// An eighth of the window in tokens, ~3 bytes per token. The constraint
    /// it serves: the between-turns compaction check reads the *previous*
    /// turn's prompt size, so one turn's results must not leap the gap
    /// between the threshold (two thirds of the window) and the window
    /// itself — a third of the window, shared with the model's own output.
    /// The old flat 24 KB is ~8–12k tokens of numeric data, *larger* than
    /// that gap at a 32k window: on the 2026-08-07 Terminal-Bench subset a
    /// trial jumped from under the threshold to 45k tokens in one turn and
    /// died on the overflow. An eighth of the window (12,288 bytes at 32k)
    /// keeps even token-dense results inside the gap with room for output.
    pub fn resolved_output_budget(&self, context_window: Option<u64>) -> usize {
        if let Some(pinned) = self.output_budget_bytes {
            return pinned;
        }
        match context_window {
            Some(window) => {
                ((window as usize / 8) * 3).clamp(Self::OUTPUT_BUDGET_MIN, Self::OUTPUT_BUDGET_MAX)
            }
            None => Self::OUTPUT_BUDGET_MAX,
        }
    }
}

impl Default for ToolsConfig {
    fn default() -> Self {
        ToolsConfig {
            enabled: Vec::new(),
            disabled: Vec::new(),
            workspace: None,
            permission_mode: PermissionMode::Ask,
            shell_timeout_secs: 120,
            output_budget_bytes: None,
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
    /// Register this server's tools under their own names, without the
    /// `<name>__` prefix. Unset means prefixed — the default that lets two
    /// servers both expose a `search`. Turn it off for a server whose tools
    /// already carry their own namespace (`kg_*`), where the prefix is pure
    /// stutter the model types in every call. The setting is a promise of
    /// distinct names: an unprefixed tool that collides with anything
    /// already registered fails startup loudly rather than shadowing it.
    pub prefix_tools: Option<bool>,
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
        crate::work::mecha_home()
            .ok()
            .map(|h| h.join("config.toml"))
    }

    pub const PROJECT_FILE: &'static str = "mecha.toml";

    /// Load defaults, then the global file, then the project file, then env.
    pub fn load(project_dir: &Path) -> Result<Self> {
        let mut cfg = Config::default();
        if let Some(path) = Self::global_path() {
            if path.exists() {
                cfg.merge_file(&path, LayerTrust::Global)?;
            }
        }
        let project = project_dir.join(Self::PROJECT_FILE);
        if project.exists() {
            cfg.merge_file(&project, LayerTrust::Project)?;
        }
        cfg.merge_env();
        Ok(cfg)
    }

    /// Defaults plus `~/.mecha/config.toml` plus env — no project layer.
    ///
    /// For runs that must not be configurable by whatever directory they happen
    /// to start in. A `mecha.toml` arrives with a cloned repository, and it can
    /// name MCP servers to spawn, hooks to execute and tools to enable; that is
    /// a reasonable bargain when a person is sitting there having just decided
    /// to work in that repository, and not one at all for a
    /// [`crate::trigger`] firing at 03:00 with nobody watching.
    pub fn load_global() -> Result<Self> {
        let mut cfg = Config::default();
        if let Some(path) = Self::global_path() {
            if path.exists() {
                cfg.merge_file(&path, LayerTrust::Global)?;
            }
        }
        cfg.merge_env();
        Ok(cfg)
    }

    fn merge_file(&mut self, path: &Path, trust: LayerTrust) -> Result<()> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut layer: ConfigLayer =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        // `[messages]` is receiver-side admission policy, and a project file
        // arrives with a cloned repository — it must not be able to switch a
        // session's inbound handling to `accept`. Dropped loudly rather than
        // silently: an ignored section that looks applied is the
        // silently-degrading-sandbox shape.
        if trust == LayerTrust::Project && layer.messages.take().is_some() {
            tracing::warn!(
                "[messages] in {} is ignored — messaging policy loads from the \
                 global config only",
                path.display()
            );
        }
        // `[slack]` for the same reason, and a stronger one: a project file
        // arrives with a cloned repository, and Slack is the remote control.
        if trust == LayerTrust::Project && layer.slack.take().is_some() {
            tracing::warn!(
                "[slack] in {} is ignored — the Slack surface loads from the \
                 global config only",
                path.display()
            );
        }
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
                self.providers
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
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

/// Which file a layer came from, deciding what it may set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerTrust {
    Global,
    Project,
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
    work: Option<WorkLayer>,
    slack: Option<SlackLayer>,
    messages: Option<MessagesLayer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct MessagesLayer {
    enabled: Option<bool>,
    dir: Option<PathBuf>,
    inbound: Option<crate::mailbox::InboundPolicy>,
    pending_cap: Option<usize>,
    max_body_bytes: Option<usize>,
    keep: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkLayer {
    keep: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlackLayer {
    max_concurrent: Option<usize>,
    approval_timeout_secs: Option<u64>,
    default_mode: Option<String>,
    max_turns: Option<u32>,
    max_cost_usd: Option<f64>,
    stream_flush_chars: Option<usize>,
    stream_flush_ms: Option<u64>,
    max_upload_mb: Option<u64>,
    tools: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutboxLayer {
    tools: Option<Vec<String>>,
    dir: Option<PathBuf>,
    publish_tools: Option<Vec<String>>,
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
    loop_guard: Option<bool>,
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
            if let Some(v) = a.loop_guard {
                t.loop_guard = v;
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
                t.output_budget_bytes = Some(v);
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
            if let Some(v) = x.publish_tools {
                t.publish_tools = v;
            }
        }
        if let Some(x) = self.work {
            if let Some(v) = x.keep {
                cfg.work.keep = v;
            }
        }
        // Only ever reached from the global layer, like `[messages]`.
        if let Some(x) = self.slack {
            let t = &mut cfg.slack;
            if let Some(v) = x.max_concurrent {
                t.max_concurrent = v;
            }
            if let Some(v) = x.approval_timeout_secs {
                t.approval_timeout_secs = v;
            }
            if let Some(v) = x.default_mode {
                t.default_mode = v;
            }
            if let Some(v) = x.max_turns {
                t.max_turns = v;
            }
            if let Some(v) = x.max_cost_usd {
                t.max_cost_usd = Some(v);
            }
            if let Some(v) = x.stream_flush_chars {
                t.stream_flush_chars = v;
            }
            if let Some(v) = x.stream_flush_ms {
                t.stream_flush_ms = v;
            }
            if let Some(v) = x.max_upload_mb {
                t.max_upload_mb = v;
            }
            if let Some(v) = x.tools {
                t.tools = v;
            }
        }
        // Only ever reached from the global layer: `merge_file` strips this
        // section from a project file before applying, with a warning.
        if let Some(x) = self.messages {
            let t = &mut cfg.messages;
            if let Some(v) = x.enabled {
                t.enabled = v;
            }
            if x.dir.is_some() {
                t.dir = x.dir;
            }
            if x.inbound.is_some() {
                t.inbound = x.inbound;
            }
            if let Some(v) = x.pending_cap {
                t.pending_cap = v;
            }
            if let Some(v) = x.max_body_bytes {
                t.max_body_bytes = v;
            }
            if let Some(v) = x.keep {
                t.keep = v;
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

    /// One turn's tool results must not leap the gap between the compaction
    /// threshold and the window — the flat 24 KB budget was ~8–12k tokens of
    /// numeric data against a 10.9k-token gap at 32k, and a 2026-08-07
    /// Terminal-Bench trial died on exactly that jump.
    #[test]
    fn the_output_budget_derives_from_a_known_context_window() {
        let mut cfg = ToolsConfig::default();

        // Unknowable window: the ceiling, which is the old flat default.
        assert_eq!(cfg.resolved_output_budget(None), 24_000);

        // The DGX's llama-server runs -c 32768: an eighth of the window in
        // tokens, ~3 bytes each — and comfortably inside the threshold gap
        // even at one byte per token.
        let derived = cfg.resolved_output_budget(Some(32768));
        assert_eq!(derived, 12_288);

        // Wide windows keep the old number; tiny ones keep results usable.
        assert_eq!(cfg.resolved_output_budget(Some(200_000)), 24_000);
        assert_eq!(cfg.resolved_output_budget(Some(8_192)), 6_000);

        cfg.output_budget_bytes = Some(1_000);
        assert_eq!(
            cfg.resolved_output_budget(Some(32768)),
            1_000,
            "explicit wins"
        );
    }

    /// A `mecha.toml` arrives with a cloned repository, and it can name MCP
    /// servers to spawn, hooks to run and tools to enable. That is a reasonable
    /// bargain for someone who just decided to work in that repository, and no
    /// bargain at all for a trigger firing at 03:00 — so the scheduled path
    /// loads the global layer only. Verified as a *difference*, because the
    /// same call on a machine with no project file proves nothing.
    #[test]
    fn the_project_layer_is_reachable_from_load_and_not_from_load_global() {
        let dir = std::env::temp_dir().join(format!("mecha-config-scope-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(Config::PROJECT_FILE),
            "default_provider = \"contributed-by-the-repository\"\n",
        )
        .unwrap();

        let with_project = Config::load(&dir).unwrap();
        assert_eq!(
            with_project.default_provider,
            "contributed-by-the-repository"
        );

        let global_only = Config::load_global().unwrap();
        assert_ne!(
            global_only.default_provider, "contributed-by-the-repository",
            "a scheduled unattended run must not take its configuration from \
             whatever directory it happens to start in"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_project_layer_slack_section_is_stripped_but_a_global_one_is_kept() {
        // The same boundary as `[messages]`, and a sharper one: Slack is the
        // remote control, and a mecha.toml arrives with a cloned repository.
        // Nothing in `[slack]` grants access — who may drive lives in the
        // binding store — but a repo must not get to widen the default mode or
        // the budget of runs someone drives from their phone.
        let dir = std::env::temp_dir().join(format!("mecha-slack-scope-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("layer.toml");
        std::fs::write(
            &path,
            "[slack]\ndefault_mode = \"allow\"\nmax_turns = 999\n",
        )
        .unwrap();

        let mut from_project = Config::default();
        from_project.merge_file(&path, LayerTrust::Project).unwrap();
        assert_eq!(
            from_project.slack.default_mode, "ask",
            "a project file must not widen the default mode"
        );
        assert_eq!(from_project.slack.max_turns, 40);

        let mut from_global = Config::default();
        from_global.merge_file(&path, LayerTrust::Global).unwrap();
        assert_eq!(
            from_global.slack.default_mode, "allow",
            "the global file is authoritative"
        );
        assert_eq!(from_global.slack.max_turns, 999);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_project_layer_messages_section_is_stripped_but_a_global_one_is_kept() {
        // The security boundary: a cloned repo's mecha.toml must not be able to
        // set `inbound = "accept"` (or enable messaging at all) on someone's
        // session. `merge_file` strips the section on a project layer and keeps
        // it on a global one — this pins both halves, and that the strip is a
        // strip rather than a broken apply.
        let dir = std::env::temp_dir().join(format!("mecha-msg-scope-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("layer.toml");
        std::fs::write(&path, "[messages]\nenabled = true\ninbound = \"accept\"\n").unwrap();

        let mut from_project = Config::default();
        from_project.merge_file(&path, LayerTrust::Project).unwrap();
        assert!(
            !from_project.messages.enabled,
            "a project file must not enable messaging"
        );
        assert!(
            from_project.messages.inbound.is_none(),
            "a project file must not set inbound policy"
        );

        let mut from_global = Config::default();
        from_global.merge_file(&path, LayerTrust::Global).unwrap();
        assert!(
            from_global.messages.enabled,
            "the global file is authoritative"
        );
        assert_eq!(
            from_global.messages.inbound,
            Some(crate::mailbox::InboundPolicy::Accept)
        );

        let _ = std::fs::remove_dir_all(&dir);
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
        assert!(
            parsed.is_ok(),
            "Config has a field ConfigLayer cannot read: {parsed:?}"
        );
    }
}
