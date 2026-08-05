//! The stdio MCP transport, provider-agnostic: the newline-delimited
//! JSON-RPC dialect mecha's client speaks. Each provider supplies its tool
//! definitions and a dispatcher; everything else — framing, initialize,
//! tools/list, error shapes — lives here once.

use serde_json::{json, Value};

/// What a provider must supply to be served over MCP.
#[async_trait::async_trait]
pub trait ToolProvider: Send + Sync {
    fn server_name(&self) -> &'static str;

    fn tools(&self) -> Vec<Value>;

    /// `None` means "no such tool"; `Some((text, is_error))` is the result.
    async fn call(&self, name: &str, args: &Value) -> Option<(String, bool)>;
}

/// Serve until stdin closes.
pub async fn serve(provider: impl ToolProvider) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut lines = stdin.lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(id) = message.get("id").cloned().filter(|v| !v.is_null()) else {
            continue; // a notification; nothing to answer
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();

        let reply = match method {
            "initialize" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {
                        "name": provider.server_name(),
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": {"tools": provider.tools()}
            }),
            "tools/call" => {
                let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match provider.call(name, &args).await {
                    Some((text, is_error)) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "content": [{"type": "text", "text": text}],
                            "isError": is_error
                        }
                    }),
                    None => json!({
                        "jsonrpc": "2.0", "id": id,
                        "error": {"code": -32601, "message": format!("no such tool: {name}")}
                    }),
                }
            }
            other => json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("unsupported method: {other}")}
            }),
        };

        stdout.write_all(reply.to_string().as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }
    Ok(())
}

/// Assertions every provider's tool list must satisfy. Shared so a new
/// provider cannot ship a mislabelled surface — the annotations are the
/// security contract the connecting client reads.
#[cfg(test)]
pub(crate) fn assert_tool_surface(tools: &[Value], reads: &[&str], writes: &[&str]) {
    let annotation = |name: &str, key: &str| -> bool {
        tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("no tool {name}"))["annotations"][key]
            .as_bool()
            .unwrap_or(false)
    };
    for read in reads {
        assert!(
            annotation(read, "readOnlyHint"),
            "{read} must be readOnlyHint"
        );
        assert!(
            !annotation(read, "openWorldHint"),
            "{read} reaches only the provider that already custodies this data — not a send sink"
        );
    }
    for write in writes {
        assert!(
            annotation(write, "openWorldHint"),
            "{write} reaches third parties"
        );
        assert!(!annotation(write, "readOnlyHint"), "{write} is a write");
    }
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        assert!(tool["description"].as_str().unwrap().len() > 20, "{name}");
    }
}
