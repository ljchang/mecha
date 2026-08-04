//! The Outlook MCP surface. Same capability argument as the Google server:
//! reads are `readOnlyHint` and deliberately not `openWorldHint` (a query
//! reaches only graph.microsoft.com, which already custodies the mailbox);
//! sends, replies, and calendar writes reach third parties, carry
//! `openWorldHint`, and are meant to be outbox-routed.

use serde_json::{json, Value};

use crate::microsoft::graph_calendar::{
    CreateEventRequest, OutlookCalendarProvider, UpdateEventRequest,
};
use crate::microsoft::graph_mail::OutlookProvider;
use crate::text::clean_body;
use crate::token::TokenManager;
use crate::types::{Email, MailError};

pub fn tool_definitions() -> Vec<Value> {
    json!([
        {
            "name": "outlook_search",
            "description": "Search the user's Outlook mail. `query` is free text, optionally with KQL fields: from:alice@x.edu, subject:invoice, received>=2026-08-01. Returns metadata and previews; use outlook_get_thread to read full messages.",
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
            "name": "outlook_recent",
            "description": "The most recent Outlook messages, newest first. Use when the user asks what just came in rather than for a specific search.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10}
                }
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "outlook_get_thread",
            "description": "Read a whole Outlook conversation by its thread id (the conversation id from a search result), oldest first, with clean text bodies.",
            "inputSchema": {
                "type": "object",
                "properties": {"thread_id": {"type": "string"}},
                "required": ["thread_id"]
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "outlook_send",
            "description": "Send a NEW email from the user's Outlook account. body_markdown is converted to HTML. To answer an existing message use outlook_reply instead, so it threads.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": {"type": "string"},
                    "subject": {"type": "string"},
                    "body_markdown": {"type": "string"},
                    "cc": {"type": "string"},
                    "bcc": {"type": "string"}
                },
                "required": ["to", "subject", "body_markdown"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "outlook_reply",
            "description": "Reply to an Outlook message by its message id (not the thread id). The reply stays in the conversation and quotes the original automatically. Set reply_all to include everyone.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message_id": {"type": "string"},
                    "body_markdown": {"type": "string"},
                    "reply_all": {"type": "boolean", "default": false}
                },
                "required": ["message_id", "body_markdown"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "calendar_list",
            "description": "List the user's Outlook calendars and whether each is writable.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "calendar_list_events",
            "description": "List Outlook calendar events in a time window, with recurring series expanded into their occurrences. Times are RFC 3339; omit both to get the next 7 days.",
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
            "description": "Create an Outlook calendar event. Times are RFC 3339 (or YYYY-MM-DD with all_day). Attendees receive invitations.",
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
            "description": "Update fields of an existing Outlook event by id. Only the fields provided change; attendees are notified.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "event_id": {"type": "string"},
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
            "description": "Delete an Outlook calendar event by id. Attendees are notified of the cancellation.",
            "inputSchema": {
                "type": "object",
                "properties": {"event_id": {"type": "string"}},
                "required": ["event_id"]
            },
            "annotations": {"openWorldHint": true, "destructiveHint": true}
        }
    ])
    .as_array()
    .cloned()
    .unwrap()
}

pub struct OutlookTools {
    pub manager: TokenManager,
}

#[async_trait::async_trait]
impl crate::mcp::ToolProvider for OutlookTools {
    fn server_name(&self) -> &'static str {
        "mecha-outlook"
    }

    fn tools(&self) -> Vec<Value> {
        tool_definitions()
    }

    async fn call(&self, name: &str, args: &Value) -> Option<(String, bool)> {
        let first = dispatch(&self.manager, name, args, false).await?;
        let result = match first {
            // One forced refresh and retry — the JS `refreshOrReconnect`
            // pattern, in Rust.
            Err(e) if e.is_auth_expiry() => dispatch(&self.manager, name, args, true).await?,
            other => other,
        };
        Some(match result {
            Ok(text) => (text, false),
            Err(e) => (format!("{e}"), true),
        })
    }
}

async fn dispatch(
    manager: &TokenManager,
    name: &str,
    args: &Value,
    force_refresh: bool,
) -> Option<Result<String, MailError>> {
    let token = if force_refresh {
        manager.force_refresh().await
    } else {
        manager.access_token().await
    };
    let token = match token {
        Ok(t) => t,
        Err(e) => return Some(Err(MailError::AuthError(format!("{e:#}")))),
    };

    let str_arg = |key: &str| args.get(key).and_then(Value::as_str).map(|s| s.to_string());
    let missing = |what: &str| Some(Err(MailError::ParseError(format!("missing required `{what}`"))));

    let result = match name {
        "outlook_search" => {
            let Some(query) = str_arg("query") else { return missing("query") };
            let max = args.get("max_results").and_then(Value::as_u64).unwrap_or(10) as u32;
            OutlookProvider::new(token)
                .search(&query, max.clamp(1, 50))
                .await
                .map(|e| render_search(&e))
        }
        "outlook_recent" => {
            let max = args.get("max_results").and_then(Value::as_u64).unwrap_or(10) as u32;
            OutlookProvider::new(token)
                .recent(max.clamp(1, 50))
                .await
                .map(|e| render_search(&e))
        }
        "outlook_get_thread" => {
            let Some(thread_id) = str_arg("thread_id") else { return missing("thread_id") };
            OutlookProvider::new(token)
                .get_thread(&thread_id)
                .await
                .map(|e| render_thread(&e))
        }
        "outlook_send" => {
            let (Some(to), Some(subject), Some(body_md)) =
                (str_arg("to"), str_arg("subject"), str_arg("body_markdown"))
            else {
                return missing("to, subject, and body_markdown");
            };
            let html = crate::google::server::markdown_to_html(&body_md);
            OutlookProvider::new(token)
                .send_email(&to, &subject, &html, str_arg("cc").as_deref(), str_arg("bcc").as_deref())
                .await
                .map(|()| format!("sent to {to}"))
        }
        "outlook_reply" => {
            let (Some(message_id), Some(body_md)) =
                (str_arg("message_id"), str_arg("body_markdown"))
            else {
                return missing("message_id and body_markdown");
            };
            let reply_all = args.get("reply_all").and_then(Value::as_bool).unwrap_or(false);
            let html = crate::google::server::markdown_to_html(&body_md);
            OutlookProvider::new(token)
                .reply(&message_id, &html, reply_all)
                .await
                .map(|()| "replied in the original conversation".to_string())
        }
        "calendar_list" => OutlookCalendarProvider::new(token)
            .list_calendars()
            .await
            .map(|c| serde_json::to_string_pretty(&c).unwrap_or_else(|_| "[]".into())),
        "calendar_list_events" => {
            let now = chrono::Utc::now();
            let time_min = str_arg("time_min").unwrap_or_else(|| now.to_rfc3339());
            let time_max = str_arg("time_max")
                .unwrap_or_else(|| (now + chrono::Duration::days(7)).to_rfc3339());
            let calendar_id = str_arg("calendar_id").unwrap_or_else(|| "primary".into());
            OutlookCalendarProvider::new(token)
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
                return missing("title, start_time, and end_time");
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
            OutlookCalendarProvider::new(token)
                .create_event(&calendar_id, &request)
                .await
                .map(|e| serde_json::to_string_pretty(&e).unwrap_or_default())
        }
        "calendar_update_event" => {
            let Some(event_id) = str_arg("event_id") else { return missing("event_id") };
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
            OutlookCalendarProvider::new(token)
                .update_event(&event_id, &request)
                .await
                .map(|e| serde_json::to_string_pretty(&e).unwrap_or_default())
        }
        "calendar_delete_event" => {
            let Some(event_id) = str_arg("event_id") else { return missing("event_id") };
            OutlookCalendarProvider::new(token)
                .delete_event(&event_id)
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

fn render_search(emails: &[Email]) -> String {
    if emails.is_empty() {
        return "no matching messages".into();
    }
    let rows: Vec<Value> = emails
        .iter()
        .map(|e| {
            json!({
                "message_id": e.provider_id,
                "thread_id": e.thread_id,
                "from": format!("{} <{}>", e.from_name, e.from_address),
                "subject": e.subject,
                "date": e.date_received,
                "preview": e.snippet,
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
                "--- From: {} <{}> · {}\nSubject: {}\nMessage id (for outlook_reply): {}\n\n{}",
                e.from_name,
                e.from_address,
                e.date_received,
                e.subject,
                e.provider_id,
                clean_body(e)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_surface_is_labelled_correctly() {
        crate::mcp::assert_tool_surface(
            &tool_definitions(),
            &[
                "outlook_search",
                "outlook_recent",
                "outlook_get_thread",
                "calendar_list",
                "calendar_list_events",
            ],
            &[
                "outlook_send",
                "outlook_reply",
                "calendar_create_event",
                "calendar_update_event",
                "calendar_delete_event",
            ],
        );
    }

    /// A reply must be reachable as its own tool: Graph threads by replying
    /// to a message, and a send-with-headers does not thread.
    #[test]
    fn reply_is_a_distinct_tool_keyed_on_message_id() {
        let tools = tool_definitions();
        let reply = tools.iter().find(|t| t["name"] == "outlook_reply").unwrap();
        assert_eq!(reply["inputSchema"]["required"][0], "message_id");
        assert!(reply["description"].as_str().unwrap().contains("conversation"));
    }
}
