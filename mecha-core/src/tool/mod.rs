//! Tools: the things an agent can actually do.
//!
//! A tool is a name, a description, a JSON Schema, and an async function. The
//! registry holds them; MCP servers and native Rust functions both land here as
//! the same trait object, so the agent loop never learns the difference.

pub mod builtin;

use crate::config::{PermissionMode, ToolsConfig};
use crate::message::ToolSpec;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    /// Returned to the model as `is_error: true` so it can recover rather than
    /// treating the failure as a result.
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        ToolOutput { content: content.into(), is_error: false }
    }

    pub fn err(content: impl Into<String>) -> Self {
        ToolOutput { content: content.into(), is_error: true }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;

    /// Read-only tools skip the approval gate and are safe to run in parallel.
    fn read_only(&self) -> bool {
        false
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput>;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: self.input_schema(),
        }
    }
}

/// What a tool is allowed to touch, and who decides.
pub struct ToolCtx {
    /// Filesystem tools refuse paths outside this root.
    pub workspace: PathBuf,
    pub shell_timeout: std::time::Duration,
}

impl ToolCtx {
    /// Resolve a model-supplied path against the workspace and prove it stays
    /// inside. The path is untrusted input: `..`, symlinks, and absolute paths
    /// all have to be checked after canonicalization, not before.
    pub fn resolve(&self, raw: &str) -> Result<PathBuf> {
        let candidate = {
            let p = Path::new(raw);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                self.workspace.join(p)
            }
        };

        // The file may not exist yet (a write), so canonicalize the nearest
        // existing ancestor and re-append the rest.
        let mut existing = candidate.as_path();
        let mut trailing = Vec::new();
        let canonical_root = loop {
            match existing.canonicalize() {
                Ok(c) => break c,
                Err(_) => match existing.parent() {
                    Some(parent) => {
                        if let Some(name) = existing.file_name() {
                            trailing.push(name.to_owned());
                        }
                        existing = parent;
                    }
                    None => anyhow::bail!("cannot resolve path {raw:?}"),
                },
            }
        };
        let mut resolved = canonical_root;
        for part in trailing.iter().rev() {
            resolved.push(part);
        }

        let root = self.workspace.canonicalize().unwrap_or_else(|_| self.workspace.clone());
        if !resolved.starts_with(&root) {
            anyhow::bail!(
                "path {raw:?} resolves outside the workspace ({})",
                root.display()
            );
        }
        Ok(resolved)
    }
}

/// The decision an approver hands back for one pending call.
#[derive(Debug, Clone)]
pub enum Decision {
    Allow,
    /// The reason is passed to the model so it can pick another approach.
    Deny(String),
}

/// Gates tool calls that aren't read-only. The CLI implements this with a
/// terminal prompt; a headless caller can auto-allow or auto-deny.
#[async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, tool: &dyn Tool, input: &Value) -> Decision;
}

/// Answers from the configured [`PermissionMode`] without asking anyone.
pub struct ModeApprover {
    pub mode: PermissionMode,
}

#[async_trait]
impl Approver for ModeApprover {
    async fn approve(&self, tool: &dyn Tool, _input: &Value) -> Decision {
        match self.mode {
            PermissionMode::Allow => Decision::Allow,
            PermissionMode::ReadOnly if tool.read_only() => Decision::Allow,
            PermissionMode::ReadOnly => Decision::Deny(format!(
                "`{}` modifies state and this run is read-only",
                tool.name()
            )),
            // Nothing is watching to answer, so the safe reading of "ask" is no.
            PermissionMode::Ask => Decision::Deny(format!(
                "`{}` needs approval and this run is non-interactive (use --yes to allow)",
                tool.name()
            )),
        }
    }
}

#[derive(Default)]
pub struct Registry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool. A later registration with the same name replaces the
    /// earlier one, so MCP servers can shadow built-ins deliberately.
    pub fn insert(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Tool>> {
        self.tools.values()
    }

    /// Specs in a stable order — the tool list is the very front of the prompt
    /// prefix, so reordering it would invalidate the cache on every request.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|t| t.spec()).collect()
    }

    /// Register the built-ins permitted by config.
    pub fn with_builtins(mut self, cfg: &ToolsConfig) -> Self {
        for tool in builtin::all() {
            let name = tool.name();
            let allowed = cfg.enabled.is_empty() || cfg.enabled.iter().any(|e| e == name);
            let blocked = cfg.disabled.iter().any(|d| d == name);
            if allowed && !blocked {
                self.insert(tool);
            }
        }
        self
    }
}
