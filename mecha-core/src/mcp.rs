//! Minimal MCP client over stdio.
//!
//! Speaks JSON-RPC 2.0 line-by-line to a child process and exposes whatever
//! tools it advertises as ordinary [`Tool`] implementations. That's the whole
//! point: an MCP server's tools and mecha's built-ins are indistinguishable to
//! the agent loop.

use crate::config::McpServerConfig;
use crate::sandbox::Sandbox;
use crate::tool::{Capabilities, Tool, ToolCtx, ToolOutput};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::oneshot;

const PROTOCOL_VERSION: &str = "2025-06-18";
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// The id of a response, however the server spelled it. We always send
/// numeric ids, but JSON-RPC allows string ids and real servers echo numbers
/// back as strings — refusing those would leave every call to time out
/// against a server that is answering.
fn response_id(msg: &Value) -> Option<u64> {
    match msg.get("id")? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// A live connection to one MCP server.
pub struct McpClient {
    name: String,
    /// Whether tools register as `<name>__<tool>` (the collision-proof
    /// default) or under their raw names. See [`McpServerConfig::prefix_tools`].
    prefix_tools: bool,
    /// Capabilities forced onto every tool from this server, unioned with what
    /// it declares. See [`McpServerConfig::capabilities`].
    forced: Capabilities,
    stdin: tokio::sync::Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    next_id: AtomicU64,
    /// The directory the server was spawned in — where its relative paths
    /// resolve, however the per-run workspace moves (see
    /// [`Tool::fixed_workspace`]).
    workspace: PathBuf,
    /// Held so the child is killed when the client drops.
    _child: Child,
}

impl McpClient {
    /// The child process as it will be spawned: confinement decided, and the
    /// environment built.
    ///
    /// Split out from [`McpClient::connect`] so the environment policy can be
    /// asserted on. It is worth asserting on: `envs()` *adds to* the inherited
    /// environment rather than replacing it, so the bug this prevents looks
    /// entirely correct at the call site while every server on the machine
    /// quietly holds your provider keys.
    fn build_command(
        cfg: &McpServerConfig,
        sandbox: &Sandbox,
        workspace: &Path,
    ) -> Result<tokio::process::Command> {
        // Same rule as `shell`: asking to be confined and silently not being
        // confined is the worst outcome, because the decision was made on the
        // belief that it held.
        if cfg.sandbox && !sandbox.is_enabled() {
            bail!(
                "MCP server `{}` is configured with `sandbox = true`, but no sandbox \
                 backend is set. Set [sandbox] kind = \"bwrap\" or \"docker\", or drop \
                 `sandbox = true` to accept that it runs unconfined.",
                cfg.name
            );
        }

        let mut command = if cfg.sandbox {
            let confined = match cfg.network {
                Some(network) => sandbox.with_network(network),
                None => sandbox.clone(),
            };
            confined
                .wrap_argv(&cfg.command, &cfg.args, workspace, workspace)
                .with_context(|| format!("confining MCP server `{}`", cfg.name))?
        } else {
            let mut c = tokio::process::Command::new(&cfg.command);
            c.args(&cfg.args);
            // The workspace, whether or not we confine. A confined server gets
            // it as the only writable mount and `--chdir`s there; an
            // unconfined one used to inherit *mecha's* working directory,
            // which is wherever the user happened to launch it. That is not a
            // containment hole — an unconfined server can reach anything
            // regardless — but it silently breaks every server that resolves a
            // relative path, because the model's paths are relative to the run
            // workspace and the server's are not. `mecha-factory-publish`
            // documents `--root` as defaulting to the working directory on
            // exactly this assumption.
            c.current_dir(workspace);
            c
        };

        // Clear first, then add back. `envs()` alone layers on top of the
        // inherited environment, which is how a server ends up holding your
        // provider keys without anyone deciding it should.
        command.env_clear();
        command.envs(Sandbox::child_env(&cfg.env_passthrough));
        command.envs(&cfg.env);

        Ok(command)
    }

    /// Spawn the server, perform the initialize handshake, and return a client.
    ///
    /// An MCP server is third-party code running on your machine, which makes
    /// it a larger hole than `shell` ever was: `shell` at least runs commands a
    /// model asked for out loud, where a server runs whatever its author wrote.
    /// So it gets the same treatment — a named environment rather than an
    /// inherited one, and optional confinement.
    pub async fn connect(
        cfg: &McpServerConfig,
        sandbox: &Sandbox,
        workspace: &Path,
    ) -> Result<Arc<Self>> {
        let mut command = Self::build_command(cfg, sandbox, workspace)?;

        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            // The MCP convention is that stderr is the server's log, not part
            // of the protocol. It used to inherit ours, but a raw share of
            // the terminal garbles a full-screen front-end mid-frame — so it
            // flows through tracing instead, tagged with the server's name
            // and visible under MECHA_LOG.
            .stderr(std::process::Stdio::piped());

        let mut child = command
            .spawn()
            .with_context(|| format!("spawning MCP server `{}` ({})", cfg.name, cfg.command))?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        if let Some(stderr) = child.stderr.take() {
            let server = cfg.name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(server = %server, "{line}");
                }
            });
        }

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
                    let Some(id) = response_id(&msg) else {
                        continue;
                    };
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
            prefix_tools: cfg.prefix_tools.unwrap_or(true),
            forced: cfg.capabilities.into(),
            stdin: tokio::sync::Mutex::new(stdin),
            pending,
            next_id: AtomicU64::new(1),
            workspace: workspace.to_path_buf(),
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

        client
            .notify("notifications/initialized", json!({}))
            .await?;
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
                err.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or(&err.to_string())
            );
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Ask the server what it can do, and wrap each answer as a [`Tool`].
    pub async fn list_tools(self: &Arc<Self>) -> Result<Vec<Arc<dyn Tool>>> {
        // Paged: a server with more tools than one page returns a
        // `nextCursor`, and stopping at page one silently shrinks its
        // surface — tools the config counted on simply would not exist.
        // Bounded, so a server that hands out cursors forever cannot wedge
        // startup.
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            let params = match cursor.take() {
                Some(c) => json!({"cursor": c}),
                None => json!({}),
            };
            let result = self.request("tools/list", params).await?;
            tools.extend(
                result
                    .get("tools")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
            match result.get("nextCursor").and_then(Value::as_str) {
                Some(c) if !c.is_empty() => cursor = Some(c.to_string()),
                _ => break,
            }
        }
        if cursor.is_some() {
            tracing::warn!(
                server = %self.name,
                "tools/list still paging after 100 pages; taking what arrived"
            );
        }

        Ok(tools
            .into_iter()
            .filter_map(|t| {
                let remote_name = t.get("name")?.as_str()?.to_string();
                let hints = t.get("annotations").cloned().unwrap_or(Value::Null);
                let hint = |k: &str| hints.get(k).and_then(Value::as_bool).unwrap_or(false);

                Some(Arc::new(McpTool {
                    // Only a forced `destructive` contradicts a read-only
                    // claim; the others are orthogonal to it and dropping the
                    // exemption for them was wrong. `untrusted_input` says the
                    // content coming *out* may be attacker-influenced, and
                    // `external_send` says data can leave — neither implies the
                    // tool changes anything, and `http_fetch` is read-only
                    // while being a send sink for exactly that reason. Blanket
                    // narrowing here made every pkg retrieval prompt for
                    // approval, which is unusable for memory read at turn start.
                    read_only: hint("readOnlyHint") && !self.forced.destructive,
                    // `openWorldHint` means the tool talks to the wider world:
                    // that makes it both a source of attacker-influenced content
                    // and a way for data to leave.
                    capabilities: Capabilities {
                        private_data: true,
                        untrusted_input: hint("openWorldHint"),
                        external_send: hint("openWorldHint"),
                        destructive: hint("destructiveHint"),
                    }
                    .union(self.forced),
                    // Namespaced so two servers can each expose a `search` —
                    // unless the config says this server's tools carry their
                    // own namespace, in which case the raw name is the name.
                    local_name: if self.prefix_tools {
                        format!("{}__{}", self.name, remote_name)
                    } else {
                        remote_name.clone()
                    },
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

    // pub(crate) for `distill`, which pushes episodes through a graph
    // server's `kg_upsert` without a run (and so without a `ToolCtx`).
    pub(crate) async fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolOutput> {
        let result = self
            .request("tools/call", json!({"name": name, "arguments": arguments}))
            .await?;

        // Content is a list of typed parts; we flatten the text ones and note
        // anything else rather than silently dropping it.
        let mut text = Vec::new();
        for part in result
            .get("content")
            .and_then(Value::as_array)
            .unwrap_or(&vec![])
        {
            match part.get("type").and_then(Value::as_str) {
                Some("text") => text.push(
                    part.get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                ),
                Some(other) => text.push(format!("[{other} content omitted]")),
                None => {}
            }
        }

        Ok(ToolOutput {
            content: if text.is_empty() {
                "(no content)".into()
            } else {
                text.join("\n")
            },
            is_error: result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            external: true,
            refusal: false,
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

    fn fixed_workspace(&self) -> Option<PathBuf> {
        // The server was spawned once, in one directory; its relative paths
        // resolve there no matter which per-run workspace the call carries.
        Some(self.client.workspace.clone())
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
    sandbox: &Sandbox,
    workspace: &Path,
) -> (Vec<Arc<dyn Tool>>, Vec<Arc<McpClient>>, Vec<String>) {
    let mut tools = Vec::new();
    let mut clients = Vec::new();
    let mut errors = Vec::new();

    for cfg in configs.iter().filter(|c| !c.disabled) {
        match McpClient::connect(cfg, sandbox, workspace).await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::SandboxConfig;

    /// Always passed through, because most runtimes cannot start without them.
    const BASE: [&str; 5] = ["PATH", "HOME", "LANG", "LC_ALL", "TZ"];

    fn unconfined() -> Sandbox {
        Sandbox::new(SandboxConfig::default())
    }

    #[test]
    fn a_response_id_is_accepted_however_the_server_spelled_it() {
        use serde_json::json;
        // We send numbers; a compliant server echoes numbers, a common
        // dialect echoes them as strings. Both must route, or every call
        // waits out the full timeout against a server that answered.
        assert_eq!(response_id(&json!({"id": 7})), Some(7));
        assert_eq!(response_id(&json!({"id": "7"})), Some(7));
        assert_eq!(response_id(&json!({"id": "not-ours"})), None);
        assert_eq!(response_id(&json!({"id": null})), None);
        assert_eq!(response_id(&json!({})), None);
    }

    #[test]
    fn asking_for_confinement_with_no_backend_is_an_error_not_a_warning() {
        let cfg = McpServerConfig {
            name: "nosy".into(),
            command: "/usr/bin/env".into(),
            sandbox: true,
            ..Default::default()
        };

        // Same rule as `shell`: running unconfined after being told to confine
        // would have every downstream decision resting on a belief nothing is
        // enforcing.
        let err = McpClient::build_command(&cfg, &unconfined(), Path::new("/tmp"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("no sandbox backend is set"),
            "unexpected error: {err}"
        );
        assert!(
            err.contains("nosy"),
            "the error should name the server: {err}"
        );
    }

    #[test]
    fn an_unconfined_server_is_spawned_directly_rather_than_wrapped() {
        let cfg = McpServerConfig {
            name: "plain".into(),
            command: "/usr/bin/env".into(),
            args: vec!["-0".into()],
            ..Default::default()
        };

        let cmd = McpClient::build_command(&cfg, &unconfined(), Path::new("/tmp")).unwrap();
        let std = cmd.as_std();

        assert_eq!(std.get_program(), "/usr/bin/env");
        let args: Vec<_> = std
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(args, vec!["-0"]);
    }

    /// Confined or not, a server starts in the run's workspace.
    ///
    /// The confined branch has always done this — the workspace is its only
    /// writable mount, and `wrap_argv` `--chdir`s into it. The unconfined
    /// branch inherited mecha's own working directory, so a server that
    /// resolves relative paths resolved them against wherever the user
    /// launched mecha. Nothing about confinement changes: an unconfined server
    /// could always reach the whole filesystem. What changes is that the two
    /// branches now agree about where the model's paths point.
    #[test]
    fn an_unconfined_server_still_starts_in_the_workspace() {
        let cfg = McpServerConfig {
            name: "plain".into(),
            command: "/usr/bin/env".into(),
            ..Default::default()
        };

        let workspace = Path::new("/tmp");
        let cmd = McpClient::build_command(&cfg, &unconfined(), workspace).unwrap();

        assert_eq!(
            cmd.as_std().get_current_dir(),
            Some(workspace),
            "an unconfined server must start in the workspace, not in mecha's cwd"
        );
    }

    /// The measurement that motivated `env_clear()`, as a test: spawn a server
    /// that reports its own environment and check what actually crossed.
    ///
    /// Asserted as a subset rather than against a hand-listed set of secrets,
    /// because the leak was never about one variable — `envs()` layered onto
    /// the inherited environment, so *everything* crossed, provider keys
    /// included, and the call site looked right.
    #[tokio::test]
    async fn the_child_environment_is_an_allowlist_not_an_inheritance() {
        let ours: std::collections::BTreeSet<String> = std::env::vars().map(|(k, _)| k).collect();

        // Something we hold that is not in the base set — under `cargo test`
        // there are many. Naming it makes it cross; its neighbours must not.
        let Some(passthrough) = ours.iter().find(|k| !BASE.contains(&k.as_str())).cloned() else {
            return; // An environment this bare has nothing to leak.
        };

        let cfg = McpServerConfig {
            name: "nosy".into(),
            command: "/usr/bin/env".into(),
            // NUL-separated: a value containing a newline cannot be mistaken
            // for another variable.
            args: vec!["-0".into()],
            env: [("MECHA_EXPLICIT_TOKEN".to_string(), "granted".to_string())]
                .into_iter()
                .collect(),
            env_passthrough: vec![passthrough.clone()],
            ..Default::default()
        };

        let mut cmd = McpClient::build_command(&cfg, &unconfined(), Path::new("/tmp")).unwrap();
        let out = cmd
            .stdout(std::process::Stdio::piped())
            .output()
            .await
            .unwrap();
        assert!(out.status.success(), "env did not run");

        let child: std::collections::BTreeSet<String> = String::from_utf8_lossy(&out.stdout)
            .split('\0')
            .filter(|s| !s.is_empty())
            .filter_map(|entry| entry.split_once('=').map(|(k, _)| k.to_string()))
            .collect();

        let allowed: std::collections::BTreeSet<String> = BASE
            .iter()
            .map(|s| s.to_string())
            .chain([passthrough.clone(), "MECHA_EXPLICIT_TOKEN".to_string()])
            .collect();

        let leaked: Vec<_> = child.difference(&allowed).collect();
        assert!(
            leaked.is_empty(),
            "these crossed without being named: {leaked:?}"
        );

        assert!(
            child.contains(&passthrough),
            "a named passthrough did not cross"
        );
        assert!(
            child.contains("MECHA_EXPLICIT_TOKEN"),
            "an explicit value did not cross"
        );
        assert!(
            child.len() < ours.len(),
            "the child holds as much as we do ({} vs {}) — the environment was inherited",
            child.len(),
            ours.len()
        );
    }
}
