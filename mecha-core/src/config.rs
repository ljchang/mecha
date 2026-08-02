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
    /// MCP servers to connect to at startup.
    #[serde(rename = "mcp")]
    pub mcp: Vec<McpServerConfig>,
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
            },
        );
        Config {
            default_provider: "anthropic".to_string(),
            providers,
            agent: AgentConfig::default(),
            tools: ToolsConfig::default(),
            mcp: Vec::new(),
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
}

impl ProviderConfig {
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
        }
    }
}

impl AgentConfig {
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
}

impl Default for ToolsConfig {
    fn default() -> Self {
        ToolsConfig {
            enabled: Vec::new(),
            disabled: Vec::new(),
            workspace: None,
            permission_mode: PermissionMode::Ask,
            shell_timeout_secs: 120,
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
pub struct McpServerConfig {
    /// Prefixed onto every tool the server exposes, so two servers can both
    /// have a `search` without colliding.
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
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
    #[serde(rename = "mcp")]
    mcp: Option<Vec<McpServerConfig>>,
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
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolsLayer {
    enabled: Option<Vec<String>>,
    disabled: Option<Vec<String>>,
    workspace: Option<PathBuf>,
    permission_mode: Option<PermissionMode>,
    shell_timeout_secs: Option<u64>,
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
        }
        // MCP servers replace wholesale — merging lists by name would make it
        // impossible for a project to turn a global server off.
        if let Some(v) = self.mcp {
            cfg.mcp = v;
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
}
