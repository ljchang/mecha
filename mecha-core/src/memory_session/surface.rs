//! Runtime-independent graph surface values. They become authority only when
//! admitted into a private chat attachment; transcript JSON cannot select them.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

pub const CHAT_GRAPH_SURFACE_VERSION: &str = "ordinary-chat-graph-surface-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphToolBinding {
    pub local_name: String,
    pub remote_name: String,
    pub schema_digest: String,
    pub description_digest: String,
    pub read_only: bool,
    pub private_data: bool,
    pub untrusted_input: bool,
    pub egress: String,
    pub destructive: bool,
    pub effect: String,
    pub deferred: bool,
    pub fixed_workspace: PathBuf,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatGraphSurface {
    pub version: String,
    pub server_name: String,
    pub launch_digest: String,
    pub local_tools: Vec<String>,
    pub graph_tools: Vec<GraphToolBinding>,
}

fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl ChatGraphSurface {
    /// Validate stored authority without consulting a server or requiring that
    /// the historical workspace still exists. Attachment creation supplies the
    /// canonical execution workspace; recovery uses that same bound path.
    pub fn validate(&self, workspace: &Path) -> Result<()> {
        ensure!(
            self.version == CHAT_GRAPH_SURFACE_VERSION
                && name(&self.server_name)
                && hash(&self.launch_digest),
            "invalid chat graph surface identity"
        );
        ensure!(
            workspace.is_absolute()
                && workspace.as_os_str().len() <= 4096
                && workspace
                    .components()
                    .all(|c| matches!(c, Component::RootDir | Component::Normal(_))),
            "invalid chat graph workspace"
        );
        ensure!(
            !self.local_tools.is_empty()
                && self.local_tools.len() <= 4
                && self.local_tools.windows(2).all(|p| p[0] < p[1]),
            "invalid chat local tool order"
        );
        ensure!(
            self.local_tools.iter().any(|n| n == "goal_context")
                && self.local_tools.iter().all(|n| matches!(
                    n.as_str(),
                    "fs_read" | "fs_write" | "fs_list" | "goal_context"
                )),
            "invalid chat local tool surface"
        );
        ensure!(
            !self.graph_tools.is_empty()
                && self.graph_tools.len() <= 3
                && self
                    .graph_tools
                    .windows(2)
                    .all(|p| p[0].local_name < p[1].local_name),
            "invalid chat graph tool order"
        );
        let mut remotes = BTreeSet::new();
        for tool in &self.graph_tools {
            ensure!(
                name(&tool.local_name)
                    && !self.local_tools.contains(&tool.local_name)
                    && !matches!(
                        tool.local_name.as_str(),
                        "fs_read" | "fs_write" | "fs_list" | "goal_context"
                    )
                    && matches!(
                        tool.remote_name.as_str(),
                        "kg_search" | "kg_entity" | "kg_task_list"
                    )
                    && remotes.insert(&tool.remote_name)
                    && hash(&tool.schema_digest)
                    && hash(&tool.description_digest)
                    && tool.read_only
                    && tool.private_data
                    && tool.untrusted_input
                    && tool.egress == "none"
                    && !tool.destructive
                    && tool.effect == "read_that_records"
                    && !tool.deferred
                    && tool.fixed_workspace == workspace,
                "invalid chat graph tool binding"
            );
        }
        ensure!(
            serde_json::to_vec(self)?.len() <= 16384,
            "chat graph surface oversized"
        );
        Ok(())
    }
    pub fn tool_names(&self) -> Vec<String> {
        let mut names = self.local_tools.clone();
        names.extend(self.graph_tools.iter().map(|t| t.local_name.clone()));
        names.sort();
        names
    }
    pub fn contains_tool(&self, name: &str) -> bool {
        self.local_tools.iter().any(|n| n == name)
            || self.graph_tools.iter().any(|t| t.local_name == name)
    }
    pub fn digest(&self) -> Result<String> {
        let workspace = &self
            .graph_tools
            .first()
            .ok_or_else(|| anyhow::anyhow!("chat graph surface has no tools"))?
            .fixed_workspace;
        self.validate(workspace)?;
        Ok(super::authority::digest(&serde_json::to_vec(self)?))
    }
    pub(super) fn validate_tools(&self, names: &[String]) -> Result<()> {
        let mut actual = names.to_vec();
        actual.sort();
        ensure!(
            actual == self.tool_names(),
            "chat graph tool surface changed"
        );
        Ok(())
    }
}
