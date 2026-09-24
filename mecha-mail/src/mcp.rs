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
/// Assert the three capability quadrants this crate's tools fall into.
///
/// - `reads` — `readOnlyHint`, never `openWorldHint`. A search query reaches
///   only the provider that already custodies the mailbox.
/// - `writes` — `openWorldHint`. These reach third parties (recipients,
///   invitees) and are what `[outbox] tools` names so they stage.
/// - `triage` — **neither**, plus `destructiveHint`. Archive, read-state,
///   spam and trash mutate the user's own mailbox and reach nobody. Marking
///   them `openWorldHint` would put them in the outbox's path and make the
///   triage loop review a queue in order to fill another queue; marking them
///   `readOnlyHint` would let a read-only unattended run empty the inbox at
///   seven in the morning. The quadrant exists so neither mistake is silent.
pub(crate) fn assert_tool_surface(
    tools: &[Value],
    reads: &[&str],
    writes: &[&str],
    triage: &[&str],
) {
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
    for t in triage {
        assert!(
            annotation(t, "destructiveHint"),
            "{t} mutates the mailbox and must say so"
        );
        assert!(
            !annotation(t, "readOnlyHint"),
            "{t} is a write — a read-only run must not reach it"
        );
        assert!(
            !annotation(t, "openWorldHint"),
            "{t} reaches no third party; openWorldHint would route it through the outbox"
        );
    }
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        assert!(tool["description"].as_str().unwrap().len() > 20, "{name}");
    }
}

/// Assert the fourth quadrant: a **private write**, which creates something
/// only the owner can read.
///
/// These tools carry no `openWorldHint`, so they execute instead of staging
/// (`docs/PROVENANCE-DESIGN.md` §2). The input schema is therefore the whole
/// guard: the outbox's "exact arguments, one click away" review no longer
/// applies. The schema must name nobody, and must not point at anything that
/// already exists — a `file_id` could name a document someone else can
/// already read, and a `folder_id` could put the new one in a shared folder.
/// Writing into either is a publish.
///
/// Two things make this a guard rather than a checklist:
///
/// - **Membership is derived, not listed.** Every tool that says
///   `openWorldHint: false` outright and is not a read is *claiming* this
///   quadrant, and every claimant is inspected. `expected` is checked against
///   the claimants, so a new verb cannot take the exemption without its
///   schema being read, and a listed one cannot quietly leave it.
/// - **The schema is judged by an allowlist.** A property is allowed only if
///   it is one of the content fields below. A name nobody anticipated —
///   `folder_id`, `parents`, `documentId` — fails until someone reads it and
///   adds it here in a diff. A denylist of bad names failed open on exactly
///   those (found in review of #274).
#[cfg(test)]
pub(crate) fn assert_private_writes(tools: &[Value], expected: &[&str]) {
    // What a private write may say: the content of the new thing, and when.
    // Nothing here names a party or an existing object. `account` picks which
    // of the owner's own accounts holds the new thing, among the ones the
    // server was configured with, so it names nobody else.
    const CONTENT: &[&str] = &[
        "title",
        "body",
        "description",
        "location",
        "start_time",
        "end_time",
        "all_day",
        "timezone",
        "account",
    ];
    let claimants: Vec<&str> = tools
        .iter()
        .filter(|t| {
            let a = &t["annotations"];
            a["openWorldHint"] == Value::Bool(false) && a["readOnlyHint"] != Value::Bool(true)
        })
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let mut want: Vec<&str> = expected.to_vec();
    let mut have = claimants.clone();
    want.sort_unstable();
    have.sort_unstable();
    assert_eq!(
        have, want,
        "the tools claiming the private-write quadrant (openWorldHint: false, not a read) \
         must be exactly the ones this test inspects"
    );
    for name in claimants {
        let tool = tools.iter().find(|t| t["name"] == name).unwrap();
        assert_ne!(
            tool["annotations"]["destructiveHint"],
            Value::Bool(true),
            "{name} creates; it must not destroy"
        );
        let props = tool["inputSchema"]["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{name} has no properties"));
        for prop in props.keys() {
            assert!(
                CONTENT.contains(&prop.as_str()),
                "{name}.{prop} is not a content field. A private write may name no party \
                 and no existing object; if this one names neither, add it to CONTENT \
                 with the reason"
            );
        }
    }
}
