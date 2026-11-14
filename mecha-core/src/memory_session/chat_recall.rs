use super::super::authority::Attempt;
use super::*;
use crate::tool::{Capabilities, ToolCtx, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
pub(super) struct TrackedChat {
    pub inner: crate::tool::goal_memory::GoalMemory,
    pub authority: Arc<Authority>,
    pub id: String,
    pub state: Arc<Mutex<RunState>>,
    pub storage: StorageAccess,
    pub lease: Arc<Lease>,
}
#[async_trait]
impl Tool for TrackedChat {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn input_schema(&self) -> Value {
        self.inner.input_schema()
    }
    fn read_only(&self) -> bool {
        self.inner.read_only()
    }
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn effect(&self) -> Option<crate::tool::effect::Effect> {
        self.inner.effect()
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        self.lease.validate()?;
        let index = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("ordinary tracking unavailable"))?;
            ensure!(
                !state.sealed && !self.authority.file(&self.id, "chat-seal")?.try_exists()?,
                "ordinary run already finalized"
            );
            if state.next >= 32 {
                authority::write(&self.authority.file(&self.id, "overflow")?, b"overflow")?;
                anyhow::bail!("ordinary recall attempt limit reached");
            }
            let i = state.next;
            state.next += 1;
            let bytes = serde_json::to_vec(&input)?;
            ensure!(bytes.len() <= 8192, "ordinary recall input oversized");
            authority::write(
                &self.authority.file(&self.id, &format!("attempt-{i}"))?,
                &bytes,
            )?;
            i
        };
        let (frozen, attachment) = frozen(&self.authority, &self.id)?;
        ensure!(
            ctx.workspace.canonicalize()? == attachment.workspace,
            "chat recall execution surface changed"
        );
        if attachment.open() {
            // Checked on every call: a grant may be made mid-chat, and one
            // that reaches the store or this transcript ends the recall.
            authority::exclude_grants(
                &self.authority.request,
                Some(&attachment.source),
                &ctx.grants.all(),
            )?;
        } else {
            ensure!(
                ctx.grants.is_empty(),
                "chat recall execution surface changed"
            );
        }
        let memory =
            Memory::open_read_only_with_storage(&self.authority.request.db, self.storage.clone())?
                .with_source_guard(self.authority.clone());
        memory.validate_dependency_roots(
            &frozen.dependencies,
            &Access {
                domain: self.authority.request.domain.clone(),
                scopes: self.authority.request.scopes.clone(),
            },
        )?;
        let output = if input.get("serves").is_some() {
            ToolOutput::err("chat memory uses authenticated owner context; omit serves")
        } else if !ctx.goal_lessons.is_empty()
            || !ctx.goal_examples.is_empty()
            || ctx.success_examples.is_some()
            || ctx.goal_appraisals.is_some()
        {
            ToolOutput::err("chat memory cannot deliver legacy goal evidence")
        } else {
            self.inner.call(input.clone(), ctx).await?
        };
        let tracked = (|| -> Result<Attempt> {
            ensure!(
                output.content.len() <= authority::RECALL_OUTPUT_MAX && output.image.is_none(),
                "ordinary recall output unavailable"
            );
            let references = if output.is_error {
                ensure!(
                    !output.external,
                    "ordinary failed recall has external delivery"
                );
                vec![]
            } else if input.get("memory").is_some() {
                let memory = Memory::open_read_only_with_storage(
                    &self.authority.request.db,
                    self.storage.clone(),
                )?
                .with_source_guard(self.authority.clone());
                let references = crate::memory_experience_bridge::memory_packet_references_checked(
                    &output.content,
                    Some((
                        &memory,
                        &Access {
                            domain: self.authority.request.domain.clone(),
                            scopes: self.authority.request.scopes.clone(),
                        },
                    )),
                )?;
                memory.validate_delivered_context(
                    &references,
                    &[],
                    &Access {
                        domain: self.authority.request.domain.clone(),
                        scopes: self.authority.request.scopes.clone(),
                    },
                )?;
                references
            } else {
                vec![]
            };
            Ok(Attempt {
                input,
                output: output.content.clone(),
                is_error: output.is_error,
                external: output.external,
                references,
            })
        })();
        // A tracking failure never changes a tool result already obtained. The
        // durable pending attempt makes later retention unavailable.
        if let Ok(attempt) = tracked {
            let _ = authority::write(
                &self.authority.file(&self.id, &format!("result-{index}"))?,
                &serde_json::to_vec(&attempt)?,
            );
        }
        Ok(output)
    }
}

/// Only metadata survives construction; an unbound schema cannot read memory.
pub(super) struct ContextSchema {
    spec: crate::message::ToolSpec,
    capabilities: Capabilities,
    effect: Option<crate::tool::effect::Effect>,
    read_only: bool,
}
impl ContextSchema {
    pub(super) fn of(tool: &dyn Tool) -> Self {
        Self {
            spec: crate::message::ToolSpec {
                name: tool.name().into(),
                description: tool.description().into(),
                input_schema: tool.input_schema(),
            },
            capabilities: tool.capabilities(),
            effect: tool.effect(),
            read_only: tool.read_only(),
        }
    }
}
#[async_trait]
impl Tool for ContextSchema {
    fn name(&self) -> &str {
        &self.spec.name
    }
    fn description(&self) -> &str {
        &self.spec.description
    }
    fn input_schema(&self) -> Value {
        self.spec.input_schema.clone()
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
    async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        Ok(ToolOutput::err(
            "chat memory requires a host-bound tracked turn",
        ))
    }
}
