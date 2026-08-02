//! Minimal MCP client over stdio.
//!
//! Speaks JSON-RPC 2.0 line-by-line to a child process and exposes whatever
//! tools it advertises as ordinary [`Tool`] implementations. That's the whole
//! point: an MCP server's tools and mecha's built-ins are indistinguishable to
//! the agent loop.

use crate::config::McpServerConfig;
use crate::tool::{Capabilities, Tool, ToolCtx, ToolOutput};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::oneshot;

const PROTOCOL_VERSION: &str = "2025-06-18";
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// A live connection to one MCP server.
pub struct McpClient {
    name: String,
    stdin: tokio::sync::Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    next_id: AtomicU64,
    /// Held so the child is killed when the client drops.
    _child: Child,
}

impl McpClient {
    /// Spawn the server, perform the initialize handshake, and return a client.
    pub async fn connect(cfg: &McpServerConfig) -> Result<Arc<Self>> {
        let mut command = tokio::process::Command::new(&cfg.command);
        command
            .args(&cfg.args)
            .envs(&cfg.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            // The MCP convention is that stderr is the server's log, not part
            // of the protocol. Let it flow to ours.
            .stderr(std::process::Stdio::inherit());

        let mut child = command
            .spawn()
            .with_context(|| format!("spawning MCP server `{}` ({})", cfg.name, cfg.command))?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Reader task: route each response to whoever is awaiting that id.
        // Server-initiated notifications have no id and are ignored.
        {
            let pending = Arc::clone(&pending);
            let server = cfg.name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let Ok(msg) = serde_json::from_str::<Value>(line) else {
                        tracing::warn!(server, line, "MCP server sent non-JSON on stdout");
                        continue;
                    };
                    let Some(id) = msg.get("id").and_then(Value::as_u64) else { continue };
                    if let Some(tx) = pending.lock().unwrap().remove(&id) {
                        let _ = tx.send(msg);
                    }
                }
                // Stdout closed: the server exited. Wake everyone still waiting
                // rather than leaving them to time out one by one.
                pending.lock().unwrap().clear();
            });
        }

        let client = Arc::new(McpClient {
            name: cfg.name.clone(),
            stdin: tokio::sync::Mutex::new(stdin),
            pending,
            next_id: AtomicU64::new(1),
            _child: child,
        });

        client
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "mecha", "version": env!("CARGO_PKG_VERSION")},
                }),
            )
            .await
            .with_context(|| format!("MCP handshake with `{}` failed", cfg.name))?;

        client.notify("notifications/initialized", json!({})).await?;
        Ok(client)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    async fn send_line(&self, msg: &Value) -> Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send_line(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);

        self.send_line(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;

        let response = match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                bail!("MCP server `{}` did not answer {method} in time", self.name);
            }
            Ok(Err(_)) => bail!("MCP server `{}` exited during {method}", self.name),
            Ok(Ok(v)) => v,
        };

        if let Some(err) = response.get("error") {
            bail!(
                "MCP server `{}` returned an error for {method}: {}",
                self.name,
                err.get("message").and_then(Value::as_str).unwrap_or(&err.to_string())
            );
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Ask the server what it can do, and wrap each answer as a [`Tool`].
    pub async fn list_tools(self: &Arc<Self>) -> Result<Vec<Arc<dyn Tool>>> {
        let result = self.request("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        Ok(tools
            .into_iter()
            .filter_map(|t| {
                let remote_name = t.get("name")?.as_str()?.to_string();
                let hints = t.get("annotations").cloned().unwrap_or(Value::Null);
                let hint = |k: &str| hints.get(k).and_then(Value::as_bool).unwrap_or(false);

                Some(Arc::new(McpTool {
                    read_only: hint("readOnlyHint"),
                    // `openWorldHint` means the tool talks to the wider world:
                    // that makes it both a source of attacker-influenced content
                    // and a way for data to leave.
                    capabilities: Capabilities {
                        private_data: true,
                        untrusted_input: hint("openWorldHint"),
                        external_send: hint("openWorldHint"),
                        destructive: hint("destructiveHint"),
                    },
                    // Namespaced so two servers can each expose a `search`.
                    local_name: format!("{}__{}", self.name, remote_name),
                    remote_name,
                    description: t
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    schema: t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object"})),
                    client: Arc::clone(self),
                }) as Arc<dyn Tool>)
            })
            .collect())
    }

    async fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolOutput> {
        let result = self
            .request("tools/call", json!({"name": name, "arguments": arguments}))
            .await?;

        // Content is a list of typed parts; we flatten the text ones and note
        // anything else rather than silently dropping it.
        let mut text = Vec::new();
        for part in result.get("content").and_then(Value::as_array).unwrap_or(&vec![]) {
            match part.get("type").and_then(Value::as_str) {
                Some("text") => {
                    text.push(part.get("text").and_then(Value::as_str).unwrap_or("").to_string())
                }
                Some(other) => text.push(format!("[{other} content omitted]")),
                None => {}
            }
        }

        Ok(ToolOutput {
            content: if text.is_empty() { "(no content)".into() } else { text.join("\n") },
            is_error: result.get("isError").and_then(Value::as_bool).unwrap_or(false),
            external: true,
        })
    }
}

struct McpTool {
    read_only: bool,
    capabilities: Capabilities,
    local_name: String,
    remote_name: String,
    description: String,
    schema: Value,
    client: Arc<McpClient>,
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.local_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
    }

    fn read_only(&self) -> bool {
        // `readOnlyHint` is advisory and often omitted, so an unannotated tool
        // is assumed to change things.
        self.read_only
    }

    fn capabilities(&self) -> Capabilities {
        // An unannotated server tool is assumed to return private data — that
        // is what most of them exist to do — but not to reach the open world,
        // because assuming otherwise would arm the interlock on every call.
        self.capabilities
    }

    async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        match self.client.call_tool(&self.remote_name, input).await {
            Ok(out) => Ok(out),
            // A transport failure is the agent's problem to route around, not a
            // reason to abort the run.
            Err(e) => Ok(ToolOutput::err(format!("MCP call failed: {e}"))),
        }
    }
}

/// Connect every enabled server in config. A server that fails to start is
/// reported and skipped — one broken entry shouldn't sink the whole session.
pub async fn connect_all(
    configs: &[McpServerConfig],
) -> (Vec<Arc<dyn Tool>>, Vec<Arc<McpClient>>, Vec<String>) {
    let mut tools = Vec::new();
    let mut clients = Vec::new();
    let mut errors = Vec::new();

    for cfg in configs.iter().filter(|c| !c.disabled) {
        match McpClient::connect(cfg).await {
            Ok(client) => match client.list_tools().await {
                Ok(mut t) => {
                    tools.append(&mut t);
                    clients.push(client);
                }
                Err(e) => errors.push(format!("{}: {e}", cfg.name)),
            },
            Err(e) => errors.push(format!("{}: {e}", cfg.name)),
        }
    }

    (tools, clients, errors)
}
