//! One recall tool for a registry that many conversations share.
//!
//! A terminal chat owns its registry and swaps the tracked reader in each
//! turn. A server builds one agent for every conversation, and its tool
//! list is the front of a cached prefix, so the list cannot change per
//! conversation. This tool stands in the registry under the recall tool's
//! name and schema, and each call is answered by the reader bound to the
//! workspace the call runs in: a web conversation's workspace is its own.
use crate::tool::{Capabilities, Tool, ToolCtx, ToolOutput};
use anyhow::{ensure, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub struct RecallRouter {
    name: String,
    description: String,
    input_schema: Value,
    capabilities: Capabilities,
    effect: Option<crate::tool::effect::Effect>,
    read_only: bool,
    bound: Mutex<BTreeMap<PathBuf, Arc<dyn Tool>>>,
}
impl RecallRouter {
    /// With the metadata of `schema`, which is never called.
    pub fn of(schema: &dyn Tool) -> Arc<Self> {
        Arc::new(Self {
            name: schema.name().into(),
            description: schema.description().into(),
            input_schema: schema.input_schema(),
            capabilities: schema.capabilities(),
            effect: schema.effect(),
            read_only: schema.read_only(),
            bound: Mutex::new(BTreeMap::new()),
        })
    }
    /// Answer calls made in `workspace` with `reader` until it is released.
    /// The reader must be the tool this one announces: a model was shown
    /// one schema and one set of capabilities, and the interlock believed
    /// them.
    pub fn bind(&self, workspace: &Path, reader: Arc<dyn Tool>) -> Result<()> {
        ensure!(
            reader.name() == self.name
                && reader.description() == self.description
                && reader.input_schema() == self.input_schema
                && reader.capabilities() == self.capabilities
                && reader.effect() == self.effect
                && reader.read_only() == self.read_only,
            "recall reader differs from the tool the registry announces"
        );
        self.bound
            .lock()
            .map_err(|_| anyhow::anyhow!("recall routing unavailable"))?
            .insert(workspace.canonicalize()?, reader);
        Ok(())
    }
    pub fn release(&self, workspace: &Path) {
        if let (Ok(mut bound), Ok(workspace)) = (self.bound.lock(), workspace.canonicalize()) {
            bound.remove(&workspace);
        }
    }
}
#[async_trait]
impl Tool for RecallRouter {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }
    fn effect(&self) -> Option<crate::tool::effect::Effect> {
        self.effect
    }
    fn read_only(&self) -> bool {
        self.read_only
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let reader = ctx.workspace.canonicalize().ok().and_then(|workspace| {
            self.bound
                .lock()
                .ok()
                .and_then(|bound| bound.get(&workspace).cloned())
        });
        match reader {
            Some(reader) => reader.call(input, ctx).await,
            // A conversation with no memory attached, or a turn its memory
            // could not begin. Said to the model, which can go on without.
            None => Ok(ToolOutput::err(
                "No memory is attached to this turn; nothing can be recalled.",
            )),
        }
    }
}
