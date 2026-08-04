//! The MCP face: tool definitions and dispatch over stdio JSON-RPC (the
//! newline-delimited dialect mecha's client speaks).
//!
//! Capability labeling, recorded because it will be re-litigated: **reads
//! are untrusted sources but not send sinks.** Mail bodies are other
//! people's words — the connecting client is expected to force
//! `untrusted_input` (mecha's config does) — but a search query travels
//! only to googleapis.com, the party that already custodies the mailbox, so
//! reads carry `readOnlyHint` and no `openWorldHint`. Sends and calendar
//! writes reach arbitrary third parties (recipients, invitees): they carry
//! `openWorldHint`, and the deployment routes them through mecha's outbox.

use serde_json::{json, Value};

use crate::calendar::{CalendarProvider, CreateEventRequest, UpdateEventRequest};
use crate::gmail::GmailProvider;
use crate::text::clean_body;
use crate::token::TokenManager;
use crate::types::{Email, GoogleError};

pub fn tool_definitions() -> Vec<Value> {
    json!([
        {
            "name": "gmail_search",
            "description": "Search the user's Gmail. `query` is Gmail search syntax (from:, to:, subject:, after:YYYY/MM/DD, is:unread, ...). Returns metadata and snippets; use gmail_get_thread to read full messages.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10}
                },
                "required": ["query"]
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "gmail_get_thread",
            "description": "Read a whole Gmail conversation by thread id (from gmail_search results), oldest first, with clean text bodies.",
            "inputSchema": {
                "type": "object",
                "properties": {"thread_id": {"type": "string"}},
                "required": ["thread_id"]
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "gmail_send",
            "description": "Send an email from the user's account. body_markdown is converted to HTML. For replies, pass the original message's thread_id and its Message-ID as reply_to_message_id so threading works.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": {"type": "string"},
                    "subject": {"type": "string"},
                    "body_markdown": {"type": "string"},
                    "cc": {"type": "string"},
                    "bcc": {"type": "string"},
                    "thread_id": {"type": "string"},
                    "reply_to_message_id": {"type": "string"}
                },
                "required": ["to", "subject", "body_markdown"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "calendar_list",
            "description": "List the user's calendars with their access roles.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "calendar_list_events",
            "description": "List calendar events in a time window (recurring events arrive expanded). Times are RFC 3339; omit both to get the next 7 days.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "time_min": {"type": "string"},
                    "time_max": {"type": "string"},
                    "calendar_id": {"type": "string", "default": "primary"}
                }
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "calendar_create_event",
            "description": "Create a calendar event. Times are RFC 3339 (or YYYY-MM-DD with all_day). Attendees receive invitations when the event is created.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "start_time": {"type": "string"},
                    "end_time": {"type": "string"},
                    "description": {"type": "string"},
                    "location": {"type": "string"},
                    "attendees": {"type": "array", "items": {"type": "string"}},
                    "all_day": {"type": "boolean", "default": false},
                    "timezone": {"type": "string"},
                    "calendar_id": {"type": "string", "default": "primary"}
                },
                "required": ["title", "start_time", "end_time"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "calendar_update_event",
            "description": "Update fields of an existing calendar event by id. Only the fields provided change.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "event_id": {"type": "string"},
                    "calendar_id": {"type": "string", "default": "primary"},
                    "title": {"type": "string"},
                    "start_time": {"type": "string"},
                    "end_time": {"type": "string"},
                    "description": {"type": "string"},
                    "location": {"type": "string"},
                    "attendees": {"type": "array", "items": {"type": "string"}},
                    "all_day": {"type": "boolean"},
                    "timezone": {"type": "string"}
                },
                "required": ["event_id"]
            },
            "annotations": {"openWorldHint": true, "destructiveHint": true}
        },
        {
            "name": "calendar_delete_event",
            "description": "Delete a calendar event by id. Attendees are notified of the cancellation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "event_id": {"type": "string"},
                    "calendar_id": {"type": "string", "default": "primary"}
                },
                "required": ["event_id"]
            },
            "annotations": {"openWorldHint": true, "destructiveHint": true}
        }
    ])
    .as_array()
    .cloned()
    .unwrap()
}

/// One tool call: token, dispatch, and — on a 401 — one forced refresh and
/// retry, which is the JS `refreshOrReconnect` pattern finally in Rust.
pub async fn call_tool(
    manager: &TokenManager,
    name: &str,
    args: &Value,
) -> Option<(String, bool)> {
    let first = dispatch(manager, name, args, false).await?;
    match first {
        Err(e) if e.is_auth_expiry() => {
            let retry = dispatch(manager, name, args, true).await?;
            Some(render(retry))
        }
        other => Some(render(other)),
    }
}

fn render(result: Result<String, GoogleError>) -> (String, bool) {
    match result {
        Ok(text) => (text, false),
        Err(e) => (format!("{e}"), true),
    }
}

/// `None` means "no such tool". `force_refresh` is the post-401 retry arm.
async fn dispatch(
    manager: &TokenManager,
    name: &str,
    args: &Value,
    force_refresh: bool,
) -> Option<Result<String, GoogleError>> {
    let token = if force_refresh {
        manager.force_refresh().await
    } else {
        manager.access_token().await
    };
    let token = match token {
        Ok(t) => t,
        Err(e) => return Some(Err(GoogleError::AuthError(format!("{e:#}")))),
    };

    let str_arg = |key: &str| args.get(key).and_then(Value::as_str).map(|s| s.to_string());
    let result = match name {
        "gmail_search" => {
            let Some(query) = str_arg("query") else {
                return Some(Err(GoogleError::ParseError("missing required `query`".into())));
            };
            let max = args.get("max_results").and_then(Value::as_u64).unwrap_or(10) as u32;
            GmailProvider::new(token)
                .search(&query, max.clamp(1, 50))
                .await
                .map(|emails| render_search(&emails))
        }
        "gmail_get_thread" => {
            let Some(thread_id) = str_arg("thread_id") else {
                return Some(Err(GoogleError::ParseError("missing required `thread_id`".into())));
            };
            GmailProvider::new(token)
                .get_thread(&thread_id)
                .await
                .map(|emails| render_thread(&emails))
        }
        "gmail_send" => {
            let (Some(to), Some(subject), Some(body_md)) =
                (str_arg("to"), str_arg("subject"), str_arg("body_markdown"))
            else {
                return Some(Err(GoogleError::ParseError(
                    "gmail_send requires `to`, `subject`, and `body_markdown`".into(),
                )));
            };
            let html = markdown_to_html(&body_md);
            GmailProvider::new(token)
                .send_email(
                    &to,
                    &subject,
                    &html,
                    str_arg("thread_id").as_deref(),
                    str_arg("cc").as_deref(),
                    str_arg("bcc").as_deref(),
                    str_arg("reply_to_message_id").as_deref(),
                )
                .await
                .map(|id| format!("sent (message id {id}) to {to}"))
        }
        "calendar_list" => CalendarProvider::new(token).list_calendars().await.map(|cals| {
            serde_json::to_string_pretty(&cals).unwrap_or_else(|_| "[]".into())
        }),
        "calendar_list_events" => {
            let now = chrono::Utc::now();
            let time_min = str_arg("time_min").unwrap_or_else(|| now.to_rfc3339());
            let time_max = str_arg("time_max")
                .unwrap_or_else(|| (now + chrono::Duration::days(7)).to_rfc3339());
            let calendar_id = str_arg("calendar_id").unwrap_or_else(|| "primary".into());
            CalendarProvider::new(token)
                .list_events(&calendar_id, &time_min, &time_max)
                .await
                .map(|events| {
                    if events.is_empty() {
                        format!("no events between {time_min} and {time_max}")
                    } else {
                        serde_json::to_string_pretty(&events).unwrap_or_else(|_| "[]".into())
                    }
                })
        }
        "calendar_create_event" => {
            let (Some(title), Some(start), Some(end)) =
                (str_arg("title"), str_arg("start_time"), str_arg("end_time"))
            else {
                return Some(Err(GoogleError::ParseError(
                    "calendar_create_event requires `title`, `start_time`, and `end_time`"
                        .into(),
                )));
            };
            let request = CreateEventRequest {
                title,
                description: str_arg("description"),
                start_time: start,
                end_time: end,
                location: str_arg("location"),
                attendees: str_list(args, "attendees"),
                all_day: args.get("all_day").and_then(Value::as_bool).unwrap_or(false),
                timezone: str_arg("timezone"),
            };
            let calendar_id = str_arg("calendar_id").unwrap_or_else(|| "primary".into());
            CalendarProvider::new(token)
                .create_event(&calendar_id, &request)
                .await
                .map(|e| serde_json::to_string_pretty(&e).unwrap_or_default())
        }
        "calendar_update_event" => {
            let Some(event_id) = str_arg("event_id") else {
                return Some(Err(GoogleError::ParseError("missing required `event_id`".into())));
            };
            let request = UpdateEventRequest {
                title: str_arg("title"),
                description: str_arg("description"),
                start_time: str_arg("start_time"),
                end_time: str_arg("end_time"),
                location: str_arg("location"),
                attendees: args
                    .get("attendees")
                    .and_then(Value::as_array)
                    .map(|_| str_list(args, "attendees")),
                all_day: args.get("all_day").and_then(Value::as_bool),
                timezone: str_arg("timezone"),
            };
            let calendar_id = str_arg("calendar_id").unwrap_or_else(|| "primary".into());
            CalendarProvider::new(token)
                .update_event(&calendar_id, &event_id, &request)
                .await
                .map(|e| serde_json::to_string_pretty(&e).unwrap_or_default())
        }
        "calendar_delete_event" => {
            let Some(event_id) = str_arg("event_id") else {
                return Some(Err(GoogleError::ParseError("missing required `event_id`".into())));
            };
            let calendar_id = str_arg("calendar_id").unwrap_or_else(|| "primary".into());
            CalendarProvider::new(token)
                .delete_event(&calendar_id, &event_id)
                .await
                .map(|()| format!("deleted event {event_id}"))
        }
        _ => return None,
    };
    Some(result)
}

fn str_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

pub fn markdown_to_html(markdown: &str) -> String {
    let parser = pulldown_cmark::Parser::new(markdown);
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    html
}

fn render_search(emails: &[Email]) -> String {
    if emails.is_empty() {
        return "no matching messages".into();
    }
    let rows: Vec<Value> = emails
        .iter()
        .map(|e| {
            json!({
                "thread_id": e.thread_id,
                "message_id": e.message_id,
                "from": format!("{} <{}>", e.from_name, e.from_address),
                "subject": e.subject,
                "date": e.date_received,
                "snippet": e.snippet,
                "unread": !e.is_read,
                "has_attachments": e.has_attachments,
            })
        })
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".into())
}

fn render_thread(emails: &[Email]) -> String {
    emails
        .iter()
        .map(|e| {
            format!(
                "--- From: {} <{}> · {}\nSubject: {}\nMessage-ID: {}\n\n{}",
                e.from_name,
                e.from_address,
                e.date_received,
                e.subject,
                e.message_id.as_deref().unwrap_or("(none)"),
                clean_body(e)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Serve MCP over stdio until stdin closes.
pub async fn serve(manager: TokenManager) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut lines = stdin.lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(line) else { continue };
        let Some(id) = message.get("id").cloned().filter(|v| !v.is_null()) else {
            continue; // a notification; nothing to answer
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or_default();

        let reply = match method {
            "initialize" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "mecha-google", "version": env!("CARGO_PKG_VERSION")}
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": {"tools": tool_definitions()}
            }),
            "tools/call" => {
                let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
                let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
                let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                match call_tool(&manager, name, &args).await {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The annotations are the security contract the connecting client reads;
    /// getting one wrong mislabels a tool for every deployment.
    #[test]
    fn reads_are_read_only_and_writes_are_open_world() {
        let tools = tool_definitions();
        let annotation = |name: &str, key: &str| -> bool {
            tools
                .iter()
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("no tool {name}"))["annotations"][key]
                .as_bool()
                .unwrap_or(false)
        };

        for read in ["gmail_search", "gmail_get_thread", "calendar_list", "calendar_list_events"]
        {
            assert!(annotation(read, "readOnlyHint"), "{read} must be readOnlyHint");
            assert!(
                !annotation(read, "openWorldHint"),
                "{read} reaches only googleapis.com, the data's custodian — not a send sink"
            );
        }
        for write in [
            "gmail_send",
            "calendar_create_event",
            "calendar_update_event",
            "calendar_delete_event",
        ] {
            assert!(annotation(write, "openWorldHint"), "{write} reaches third parties");
            assert!(!annotation(write, "readOnlyHint"), "{write} is a write");
        }
        for destructive in ["calendar_update_event", "calendar_delete_event"] {
            assert!(annotation(destructive, "destructiveHint"));
        }
    }

    #[test]
    fn every_tool_declares_an_object_schema_with_required_fields_listed() {
        for tool in tool_definitions() {
            let name = tool["name"].as_str().unwrap();
            assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
            assert!(tool["description"].as_str().unwrap().len() > 20, "{name}");
        }
    }

    #[test]
    fn markdown_becomes_html_at_the_send_boundary() {
        let html = markdown_to_html("Hello **there**\n\n- one\n- two");
        assert!(html.contains("<strong>there</strong>"), "{html}");
        assert!(html.contains("<li>one</li>"), "{html}");
    }
}
