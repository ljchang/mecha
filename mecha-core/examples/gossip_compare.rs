//! Native gossip experiment actor. Runs one isolated trial against frozen MCP
//! sources; the Python runner registers order, inputs and scoring before use.
use anyhow::{Context, Result};
use async_trait::async_trait;
use mecha_core::{
    agent::{Agent, RunContext},
    config::{AgentConfig, McpServerConfig, PermissionMode, ProviderConfig},
    gossip::{self, EntityIdentity, FollowupMode, ReaderSetup, Vantage},
    mcp::McpClient,
    message::{CompletionRequest, CompletionResponse},
    provider::{self, Provider, StreamSink},
    sandbox::Sandbox,
    tool::{ModeApprover, ToolCtx},
};
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};

struct RecordedProvider {
    inner: Box<dyn Provider>,
    role: String,
    log: Arc<Mutex<std::fs::File>>,
}
impl RecordedProvider {
    fn record(&self, value: Value) -> Result<()> {
        let mut file = self.log.lock().unwrap();
        writeln!(file, "{value}")?;
        file.flush()?;
        Ok(())
    }
}
#[async_trait]
impl Provider for RecordedProvider {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn default_model(&self) -> &str {
        self.inner.default_model()
    }
    async fn complete(
        &self,
        req: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        self.record(
            json!({"event": "request", "role": self.role, "model": req.model,
            "system": req.system, "messages": req.messages, "tools": req.tools,
            "max_tokens": req.max_tokens, "thinking": req.thinking, "effort": req.effort}),
        )?;
        let response = self.inner.complete(req, sink).await;
        match &response {
            Ok(r) => self.record(json!({"event": "response", "role": self.role,
                "message": r.message, "usage": r.usage, "model": r.model,
                "stop_reason": r.stop_reason, "malformed_tool_args": r.malformed_tool_args}))?,
            Err(e) => self
                .record(json!({"event": "error", "role": self.role, "error": format!("{e:#}")}))?,
        }
        response
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 6,
        "gossip_compare CASES SERVER CASE MODE SEED OUT"
    );
    let cases: Vec<Value> = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let case = cases
        .iter()
        .find(|c| c["id"] == args[2])
        .context("unknown case")?;
    let mode: FollowupMode = serde_json::from_value(json!(args[3]))?;
    let seed: u64 = args[4].parse()?;
    let out = PathBuf::from(&args[5]);
    std::fs::create_dir(&out).context("trial output must be new")?;
    let log = Arc::new(Mutex::new(
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(out.join("provider.jsonl"))?,
    ));
    let provider_cfg: ProviderConfig = toml::from_str(&format!(
        r#"
kind = "local"
model = "qwen3.6-35b-a3b"
base_url = "http://127.0.0.1:8080"
context_window = 262144
max_retries = 0
seed = {seed}
"#
    ))?;
    let agent_cfg = AgentConfig {
        max_turns: 4,
        max_tokens: provider::LOCAL_MAX_TOKENS,
        force_final_answer: false,
        boredom: false,
        step_checks: false,
        goal_guidance: false,
        compact_validate: false,
        ..AgentConfig::default()
    };
    std::fs::write(
        out.join("config.json"),
        serde_json::to_vec_pretty(&json!({
            "provider": provider_cfg, "agent": agent_cfg, "mode": mode,
            "rounds": 3, "answer_max_turns": 4, "ask_max_turns": 1,
            "ask_retry_limit": 1, "maximum_provider_requests": 32,
        }))?,
    )?;
    let server = McpServerConfig {
        name: "gossip-fixture".into(),
        command: "python3".into(),
        args: vec![
            args[1].clone(),
            "--cases".into(),
            args[0].clone(),
            "--case".into(),
            args[2].clone(),
            "--log".into(),
            out.join("search.jsonl").to_string_lossy().into(),
        ],
        ..McpServerConfig::default()
    };
    let client = McpClient::connect(&server, &Sandbox::new(Default::default()), &out).await?;
    let identity: EntityIdentity = serde_json::from_value(case["entity"].clone())?;
    let tool_ctx = ToolCtx {
        workspace: out.clone(),
        ..ToolCtx::default()
    };
    let mut answerers: Vec<(Vantage, Agent)> = Vec::new();
    let mut askers: Vec<(Vantage, Agent)> = Vec::new();
    for source in ["slack", "bee"] {
        let v = Vantage {
            label: source.into(),
            sources: vec![source.into()],
        };
        let recorded = |role: &str| -> Result<Box<dyn Provider>> {
            Ok(Box::new(RecordedProvider {
                inner: provider::build(&provider_cfg)?,
                role: format!("{source}_{role}"),
                log: log.clone(),
            }))
        };
        answerers.push((
            v.clone(),
            gossip::reader_with_identity(
                recorded("answer")?,
                ReaderSetup {
                    client: client.clone(),
                    vantage: v.clone(),
                    since: "2026-01-01".into(),
                    until: "2026-09-10".into(),
                    tool_ctx: tool_ctx.clone(),
                    agent_cfg: agent_cfg.clone(),
                    model: provider_cfg.model.clone(),
                    system_prompt: gossip::ANSWER_SYS.into(),
                },
                Some(identity.clone()),
            )?,
        ));
        let ask_cfg = AgentConfig {
            max_turns: 1,
            ..agent_cfg.clone()
        };
        askers.push((
            v,
            gossip::asker_with_followup(
                recorded("ask")?,
                tool_ctx.clone(),
                ask_cfg,
                provider_cfg.model.clone(),
                mode,
            )?,
        ));
    }
    let cx = RunContext::new(
        tool_ctx,
        Arc::new(ModeApprover {
            mode: PermissionMode::ReadOnly,
        }),
    );
    let start = std::time::Instant::now();
    let result = gossip::exchange_with_followup(
        &answerers,
        &askers,
        &cx,
        &identity.name,
        case["question"].as_str().context("question")?,
        3,
        mode,
    )
    .await;
    let record = match &result {
        Ok(exchange) => json!({"status": "complete", "exchange": exchange,
            "round_yield": gossip::round_yield(exchange), "elapsed_seconds": start.elapsed().as_secs_f64()}),
        Err(e) => {
            json!({"status": "error", "error": format!("{e:#}"), "elapsed_seconds": start.elapsed().as_secs_f64()})
        }
    };
    std::fs::write(out.join("result.json"), serde_json::to_vec_pretty(&record)?)?;
    result?;
    Ok(())
}
