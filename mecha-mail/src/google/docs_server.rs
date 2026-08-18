//! The MCP face for documents: tool definitions and dispatch.
//!
//! **The capability labeling is the part worth not re-litigating**, and it
//! lands in the same three quadrants as the mail surface — but the middle one
//! is here for a different reason, and the reason is easy to miss.
//!
//! - **Reads** carry `readOnlyHint` and *not* `openWorldHint`. A document
//!   fetch travels only to googleapis.com, which already custodies the file.
//!   The bodies are still other people's words — a shared document is
//!   third-party text, and a document *comment* is a better injection vector
//!   than an email body because it is invisible in the rendered page — so the
//!   connecting client is expected to force `untrusted_input`, exactly as
//!   mecha's config already does for mail and pkg.
//! - **Writes** carry `openWorldHint`, and this is the leg people miss:
//!   **writing into a document a third party can read is exfiltration.** It
//!   looks like a local edit and it is a publish, with far more bandwidth
//!   than `http_fetch`'s query string. So every write belongs in
//!   `[outbox] tools` and stages rather than executing.
//! - **`docs_trash` is neither.** It moves the user's own file to their own
//!   trash and reaches nobody, so `openWorldHint` would be wrong — it would
//!   route trashing through the outbox and make review circular. But
//!   `readOnlyHint` would be worse: a read-only unattended run could then
//!   empty a folder at seven in the morning. It carries `destructiveHint`
//!   alone and sits with the approver. This is the `mail_triage` quadrant,
//!   arrived at independently.
//!
//! Deliberately absent: any verb that changes sharing or permissions.
//! `drive.file` would happily permit it, so the boundary here has to be the
//! tool surface — and it is the one action where a successful injection costs
//! the whole corpus rather than one file.

use serde_json::{json, Value};

use crate::google::docs::{kind_of, DocsClient};
use crate::token::TokenManager;
use crate::types::MailError;

pub struct DocsTools {
    pub client: DocsClient,
}

pub fn tool_definitions() -> Vec<Value> {
    json!([
        {
            "name": "docs_list",
            "description": "List every Google Doc, Sheet, Slides deck and folder mecha can reach. This is the whole of what it can touch: files it created, plus files the user added with `mecha-docs pick`. A document not listed here is not reachable and cannot be made reachable from inside a run.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "docs_read",
            "description": "Read a Google Doc's text by file id (from docs_list). Returns the title and the body as plain text, with tables flattened to pipe-separated rows.",
            "inputSchema": {
                "type": "object",
                "properties": {"file_id": {"type": "string"}},
                "required": ["file_id"]
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "sheets_read",
            "description": "Read a range from a Google Sheet by file id. `range` is A1 notation, optionally with a sheet name: 'Sheet1!A1:D50', or 'A:C' for whole columns.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_id": {"type": "string"},
                    "range": {"type": "string", "default": "A1:Z1000"}
                },
                "required": ["file_id"]
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "slides_read",
            "description": "Read a Google Slides presentation's text by file id, slide by slide.",
            "inputSchema": {
                "type": "object",
                "properties": {"file_id": {"type": "string"}},
                "required": ["file_id"]
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "docs_create",
            "description": "Create a new Google Doc with a title, and optionally an initial body. Returns its file id. Anything mecha creates is reachable from then on with no further permission step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "body": {"type": "string"}
                },
                "required": ["title"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "docs_append",
            "description": "Append text to the end of a Google Doc. Use for adding a section or a note; use docs_replace to change text that is already there.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_id": {"type": "string"},
                    "text": {"type": "string"}
                },
                "required": ["file_id", "text"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "docs_replace",
            "description": "Replace every occurrence of some text in a Google Doc. This is the surgical edit: quote enough of the surrounding wording in `find` to be unambiguous. Reports how many occurrences changed, and zero means the anchor text was not found and nothing was edited.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_id": {"type": "string"},
                    "find": {"type": "string"},
                    "replace": {"type": "string"},
                    "match_case": {"type": "boolean", "default": true}
                },
                "required": ["file_id", "find", "replace"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "sheets_create",
            "description": "Create a new Google Sheet with a title. Returns its file id.",
            "inputSchema": {
                "type": "object",
                "properties": {"title": {"type": "string"}},
                "required": ["title"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "sheets_write",
            "description": "Write rows into a Google Sheet range. `values` is an array of row arrays. Values are interpreted as a person typing them would expect, so '=SUM(A1:A9)' becomes a formula. Overwrites whatever is in the range.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_id": {"type": "string"},
                    "range": {"type": "string"},
                    "values": {
                        "type": "array",
                        "items": {"type": "array", "items": {"type": "string"}}
                    }
                },
                "required": ["file_id", "range", "values"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "slides_create",
            "description": "Create a new Google Slides presentation with a title. Returns its file id. Editing slide content is not yet supported.",
            "inputSchema": {
                "type": "object",
                "properties": {"title": {"type": "string"}},
                "required": ["title"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "docs_trash",
            "description": "Move a file mecha can reach to the user's Drive trash, where they can restore it. There is deliberately no permanent-delete verb.",
            "inputSchema": {
                "type": "object",
                "properties": {"file_id": {"type": "string"}},
                "required": ["file_id"]
            },
            "annotations": {"destructiveHint": true}
        }
    ])
    .as_array()
    .unwrap()
    .clone()
}

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, MailError> {
    args[key]
        .as_str()
        .ok_or_else(|| MailError::InvalidInput(format!("`{key}` is required")))
}

async fn dispatch(
    client: &DocsClient,
    name: &str,
    args: &Value,
) -> Option<Result<String, MailError>> {
    let out = match name {
        "docs_list" => client.list_scope().await.map(|files| {
            if files.is_empty() {
                return "Nothing is in scope yet. Documents you create here become \
                        reachable automatically; existing ones must be added by the \
                        user with `mecha-docs pick`."
                    .to_string();
            }
            files
                .iter()
                .map(|f| format!("{:7} {}  [{}]", kind_of(&f.mime_type), f.name, f.id))
                .collect::<Vec<_>>()
                .join("\n")
        }),
        "docs_read" => match arg(args, "file_id") {
            Ok(id) => client
                .read_document(id)
                .await
                .map(|(title, body)| format!("# {title}\n\n{body}")),
            Err(e) => Err(e),
        },
        "sheets_read" => match arg(args, "file_id") {
            Ok(id) => {
                let range = args["range"].as_str().unwrap_or("A1:Z1000");
                client.read_sheet(id, range).await.map(|rows| {
                    if rows.is_empty() {
                        format!("{range} is empty")
                    } else {
                        rows.iter()
                            .map(|r| r.join("\t"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                })
            }
            Err(e) => Err(e),
        },
        "slides_read" => match arg(args, "file_id") {
            Ok(id) => client
                .read_presentation(id)
                .await
                .map(|(title, body)| format!("# {title}\n\n{body}")),
            Err(e) => Err(e),
        },
        "docs_create" => match arg(args, "title") {
            Ok(title) => match client.create_document(title).await {
                Ok(id) => {
                    // A create followed by a failed body write leaves an empty
                    // document, which is recoverable and visible. Reporting the
                    // id regardless is what makes it recoverable.
                    if let Some(body) = args["body"].as_str().filter(|b| !b.is_empty()) {
                        match client.append_text(&id, body).await {
                            Ok(()) => Ok(format!("created {title:?} [{id}] with its body")),
                            Err(e) => Ok(format!(
                                "created {title:?} [{id}], but writing the body failed: {e}"
                            )),
                        }
                    } else {
                        Ok(format!("created {title:?} [{id}]"))
                    }
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        },
        "docs_append" => match (arg(args, "file_id"), arg(args, "text")) {
            (Ok(id), Ok(text)) => client
                .append_text(id, text)
                .await
                .map(|()| format!("appended {} characters", text.len())),
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "docs_replace" => match (
            arg(args, "file_id"),
            arg(args, "find"),
            arg(args, "replace"),
        ) {
            (Ok(id), Ok(find), Ok(replace)) => {
                let match_case = args["match_case"].as_bool().unwrap_or(true);
                match client.replace_text(id, find, replace, match_case).await {
                    // Zero is not success. A model told "ok" here goes on to
                    // report an edit that never happened.
                    Ok(0) => Ok(format!(
                        "no occurrences of {find:?} found — nothing was changed. \
                         Read the document and quote its exact wording."
                    )),
                    Ok(n) => Ok(format!("replaced {n} occurrence(s)")),
                    Err(e) => Err(e),
                }
            }
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
        },
        "sheets_create" => match arg(args, "title") {
            Ok(title) => client
                .create_sheet(title)
                .await
                .map(|id| format!("created sheet {title:?} [{id}]")),
            Err(e) => Err(e),
        },
        "sheets_write" => match (arg(args, "file_id"), arg(args, "range")) {
            (Ok(id), Ok(range)) => {
                let values = args["values"].clone();
                if !values.is_array() {
                    Err(MailError::InvalidInput(
                        "`values` must be an array of row arrays".into(),
                    ))
                } else {
                    client
                        .write_sheet(id, range, values)
                        .await
                        .map(|n| format!("wrote {n} cell(s) to {range}"))
                }
            }
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "slides_create" => match arg(args, "title") {
            Ok(title) => client
                .create_presentation(title)
                .await
                .map(|id| format!("created presentation {title:?} [{id}]")),
            Err(e) => Err(e),
        },
        "docs_trash" => match arg(args, "file_id") {
            Ok(id) => client
                .trash(id)
                .await
                .map(|()| format!("moved {id} to the Drive trash; it can be restored there")),
            Err(e) => Err(e),
        },
        _ => return None,
    };
    Some(out)
}

pub async fn call_tool(client: &DocsClient, name: &str, args: &Value) -> Option<(String, bool)> {
    // Expected failures come back as `is_error` results rather than protocol
    // errors, so the model can recover — the crate-wide convention.
    match dispatch(client, name, args).await? {
        Ok(text) => Some((text, false)),
        Err(e) => Some((e.to_string(), true)),
    }
}

#[async_trait::async_trait]
impl crate::mcp::ToolProvider for DocsTools {
    fn server_name(&self) -> &'static str {
        "mecha-docs"
    }

    fn tools(&self) -> Vec<Value> {
        tool_definitions()
    }

    async fn call(&self, name: &str, args: &Value) -> Option<(String, bool)> {
        call_tool(&self.client, name, args).await
    }
}

impl DocsTools {
    pub fn new(manager: TokenManager) -> Self {
        Self {
            client: DocsClient::new(manager),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_surface_is_labelled_correctly() {
        crate::mcp::assert_tool_surface(
            &tool_definitions(),
            &["docs_list", "docs_read", "sheets_read", "slides_read"],
            &[
                "docs_create",
                "docs_append",
                "docs_replace",
                "sheets_create",
                "sheets_write",
                "slides_create",
            ],
            &["docs_trash"],
        );
    }

    /// Sharing is the one action where a successful injection costs the whole
    /// corpus rather than one file, and `drive.file` would permit it — so the
    /// boundary has to be the tool surface, and an absence needs a test or it
    /// is only a habit.
    #[test]
    fn there_is_no_sharing_or_permissions_verb() {
        let names: Vec<String> = tool_definitions()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        for forbidden in ["share", "permission", "publish", "anyone", "delete"] {
            assert!(
                !names.iter().any(|n| n.contains(forbidden)),
                "the documents surface must offer no {forbidden} verb; got {names:?}"
            );
        }
    }

    /// Trash rather than destroy, and the absence of a permanent-delete verb
    /// is the point — the same rule that keeps `gmail.modify` short of
    /// `https://mail.google.com/`.
    #[test]
    fn removal_is_reversible_and_says_so() {
        let trash = tool_definitions()
            .into_iter()
            .find(|t| t["name"] == "docs_trash")
            .expect("docs_trash exists");
        let description = trash["description"].as_str().unwrap();
        assert!(description.contains("restore"));
        assert!(description.contains("no permanent-delete"));
    }

    /// Every write is named in the deployment's `[outbox] tools`, so this
    /// list is what a config has to cover. A write added without an
    /// annotation would execute unstaged, which is the silently-degrading
    /// shape.
    #[test]
    fn every_non_read_is_either_a_staged_write_or_the_trash_verb() {
        for tool in tool_definitions() {
            let name = tool["name"].as_str().unwrap().to_string();
            let a = &tool["annotations"];
            let read = a["readOnlyHint"].as_bool().unwrap_or(false);
            let world = a["openWorldHint"].as_bool().unwrap_or(false);
            let destructive = a["destructiveHint"].as_bool().unwrap_or(false);
            assert!(
                read || world || destructive,
                "{name} carries no capability annotation at all"
            );
            assert!(!(read && world), "{name} cannot be both a read and a sink");
        }
    }
}
