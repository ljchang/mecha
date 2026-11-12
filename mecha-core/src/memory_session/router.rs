//! One recall tool for a registry that many conversations share.
//!
//! A terminal chat owns its registry and swaps the tracked reader in each
//! turn. A server builds one agent for every conversation, and its tool
//! list is the front of a cached prefix, so the list cannot change per
//! conversation. This tool stands in the registry under the recall tool's
//! name and schema, and each call is answered by the reader bound to the
//! conversation the call was made in (`ToolCtx::conversation`, stamped by
//! the front end per turn). Never by its workspace: a resumed conversation
//! keeps its recorded workspace under a new key, so two live conversations
//! can share one directory.
use crate::tool::{Capabilities, Tool, ToolCtx, ToolOutput};
use anyhow::{ensure, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub struct RecallRouter {
    name: String,
    description: String,
    input_schema: Value,
    capabilities: Capabilities,
    effect: Option<crate::tool::effect::Effect>,
    read_only: bool,
    bound: Mutex<BTreeMap<String, Arc<dyn Tool>>>,
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
    /// Answer calls made in `conversation` with `reader` until it is
    /// released.
    /// The reader must be the tool this one announces: a model was shown
    /// one schema and one set of capabilities, and the interlock believed
    /// them.
    pub fn bind(&self, conversation: &str, reader: Arc<dyn Tool>) -> Result<()> {
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
            .insert(conversation.into(), reader);
        Ok(())
    }
    pub fn release(&self, conversation: &str) {
        if let Ok(mut bound) = self.bound.lock() {
            bound.remove(conversation);
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
        let reader = ctx.conversation.as_deref().and_then(|conversation| {
            self.bound
                .lock()
                .ok()
                .and_then(|bound| bound.get(conversation).cloned())
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Reader {
        says: &'static str,
        description: &'static str,
        private: bool,
    }
    #[async_trait]
    impl Tool for Reader {
        fn name(&self) -> &str {
            "goal_context"
        }
        fn description(&self) -> &str {
            self.description
        }
        fn input_schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                private_data: self.private,
                ..Capabilities::default()
            }
        }
        async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
            Ok(ToolOutput::ok(self.says))
        }
    }
    fn reader(says: &'static str) -> Arc<dyn Tool> {
        Arc::new(Reader {
            says,
            description: "recall",
            private: true,
        })
    }
    async fn asked(router: &RecallRouter, conversation: Option<&str>, workspace: &Path) -> String {
        let mut ctx = ToolCtx {
            workspace: workspace.to_path_buf(),
            ..Default::default()
        };
        ctx.conversation = conversation.map(str::to_string);
        router
            .call(serde_json::json!({}), &ctx)
            .await
            .unwrap()
            .content
    }
    use std::path::Path;

    /// Two conversations that share one workspace, as a resumed chat and
    /// the chat first made in its directory do, each get their own reader;
    /// releasing one leaves the other bound.
    #[tokio::test]
    async fn a_call_is_answered_by_its_conversation_s_reader_not_its_workspace_s() {
        // One directory for both, never touched: routing does not read it.
        let dir = std::env::temp_dir();
        let router = RecallRouter::of(reader("schema").as_ref());
        router.bind("first", reader("first's memory")).unwrap();
        router.bind("r-second", reader("second's memory")).unwrap();
        assert_eq!(asked(&router, Some("first"), &dir).await, "first's memory");
        assert_eq!(
            asked(&router, Some("r-second"), &dir).await,
            "second's memory"
        );
        router.release("first");
        assert_eq!(
            asked(&router, Some("r-second"), &dir).await,
            "second's memory"
        );
        for unbound in [Some("first"), Some("third"), None] {
            assert!(asked(&router, unbound, &dir)
                .await
                .contains("No memory is attached"));
        }
    }

    /// A model was shown one description and one set of capabilities, and
    /// the interlock believed them: a reader that differs is not bound.
    #[test]
    fn a_reader_that_differs_from_what_the_registry_announces_is_refused() {
        let router = RecallRouter::of(reader("schema").as_ref());
        assert!(router
            .bind(
                "c",
                Arc::new(Reader {
                    says: "x",
                    description: "something else",
                    private: true
                })
            )
            .is_err());
        assert!(router
            .bind(
                "c",
                Arc::new(Reader {
                    says: "x",
                    description: "recall",
                    private: false
                })
            )
            .is_err());
        router.bind("c", reader("x")).unwrap();
    }
}
