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
        // Exhaustive: a tool in none of the three lists is either a private
        // write, whose schema is then inspected, or a mistake. Before this, a
        // verb listed nowhere was checked only for its schema type and the
        // length of its description — and executed unstaged if it carried no
        // `openWorldHint` (found in review of #274).
        let listed = reads.contains(&name) || writes.contains(&name) || triage.contains(&name);
        assert!(
            listed || is_private_write_claim(tool),
            "{name} is in no quadrant: list it as a read, a write or a triage verb, or \
             declare it a private write with openWorldHint: false"
        );
    }
    let claimants: Vec<&str> = tools
        .iter()
        .filter(|t| is_private_write_claim(t))
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    check_private_write_schemas(tools, &claimants);
}

/// Does this tool claim the fourth quadrant? It does when it would execute
/// unstaged and is not a read or a destructive triage verb. mecha-core gives
/// `Egress::None` to an *absent* `openWorldHint` as much as to `false`
/// (`hint()` is `unwrap_or(false)`), so both spellings claim it, and both
/// are inspected (found in review of #274).
#[cfg(test)]
fn is_private_write_claim(tool: &Value) -> bool {
    let a = &tool["annotations"];
    a["openWorldHint"] != Value::Bool(true)
        && a["readOnlyHint"] != Value::Bool(true)
        && a["destructiveHint"] != Value::Bool(true)
}

/// Assert the fourth quadrant: a **private write**, which creates something
/// only the owner can read.
///
/// These tools execute instead of staging, because nothing marks them
/// `openWorldHint` (`docs/PROVENANCE-DESIGN.md` §2). The input schema is
/// therefore the whole guard: the outbox's "exact arguments, one click away"
/// review no longer applies. The schema must name nobody, and must not point
/// at anything that already exists — a `file_id` could name a document
/// someone else can already read, and a `folder_id` could put the new one in
/// a shared folder. Writing into either is a publish. A private write says
/// `openWorldHint: false` outright, so the label reads as a decision.
///
/// Two things make this a guard rather than a checklist:
///
/// - **Membership is derived, not listed.** Every tool that would execute
///   unstaged and is neither a read nor a destructive triage verb claims this
///   quadrant, whether `openWorldHint` is `false` or absent, and every
///   claimant is inspected — here, and by [`assert_tool_surface`] on every
///   surface that calls it. `expected` pins the claimants, so a new verb
///   cannot take the exemption unseen, and a listed one cannot quietly
///   leave it.
/// - **The schema is judged by an allowlist.** A property is allowed only if
///   it is one of *that tool's* content fields in [`check_private_write_schemas`]. A
///   name nobody anticipated — `folder_id`, `parents`, `documentId` — fails
///   until someone reads it and adds it there in a diff. A denylist of bad
///   names failed open on exactly those (found in review of #274).
#[cfg(test)]
pub(crate) fn assert_private_writes(tools: &[Value], expected: &[&str]) {
    let mut have: Vec<&str> = tools
        .iter()
        .filter(|t| is_private_write_claim(t))
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let mut want: Vec<&str> = expected.to_vec();
    want.sort_unstable();
    have.sort_unstable();
    assert_eq!(
        have, want,
        "the tools that would execute unstaged without being a read or a triage verb \
         must be exactly the private writes this test inspects"
    );
    for name in &have {
        let a = &tools.iter().find(|t| t["name"] == *name).unwrap()["annotations"];
        assert_eq!(
            a["openWorldHint"],
            Value::Bool(false),
            "{name} is a private write; it must say openWorldHint: false outright"
        );
    }
    check_private_write_schemas(tools, &have);
}

/// The allowlist half of [`assert_private_writes`], shared with
/// [`assert_tool_surface`] so every surface inspects its claimants.
#[cfg(test)]
fn check_private_write_schemas(tools: &[Value], names: &[&str]) {
    // What each private write may say. Per tool, because a name that is
    // innocent on one verb is the escape on another: `location` is a place
    // on a calendar event and could be a Drive folder on a document, and a
    // crate-wide list let one verb's vocabulary widen every other's (found
    // in review of #277). A verb not named here gets the narrow default, so a
    // new private write is read before it can declare anything else.
    fn content_fields(tool: &str) -> &'static [&'static str] {
        match tool {
            // Read in #277. `description` and `location` are the event's
            // text; the four time fields say when. `account` picks which of
            // the owner's own configured accounts holds the hold — an enum
            // over those names, so it names nobody else.
            "calendar_hold" => &[
                "title",
                "description",
                "location",
                "start_time",
                "end_time",
                "all_day",
                "timezone",
                "account",
            ],
            _ => &["title", "body"],
        }
    }
    for name in names {
        let tool = tools.iter().find(|t| t["name"] == *name).unwrap();
        let props = tool["inputSchema"]["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{name} has no properties"));
        for (prop, spec) in props {
            // Scalars only: an allowed name must not smuggle a structure in.
            // `body: {"type": "object", "properties": {"share_with": …}}`
            // passed a names-only check (found in review of #274).
            let scalar = matches!(
                spec["type"].as_str(),
                Some("string" | "boolean" | "integer" | "number")
            ) && spec.get("properties").is_none()
                && spec.get("items").is_none();
            assert!(
                scalar,
                "{name}.{prop} is not a flat scalar; a private write's fields carry content, \
                 never a structure that could hold a recipient"
            );
            assert!(
                content_fields(name).contains(&prop.as_str()),
                "{name}.{prop} is not a content field. A private write may name no party \
                 and no existing object; if this one names neither, add it to \
                 content_fields for {name} with the reason"
            );
        }
    }
}

#[cfg(test)]
mod guard_tests {
    //! The guard that replaced the outbox's review, tested on its own: every
    //! surface in this crate satisfies it, so without these a change that
    //! stopped it guarding would leave every test green.

    use super::*;

    fn create(props: Value, annotations: Value) -> Value {
        json!({
            "name": "thing_create",
            "description": "Create a new thing with a title, for the guard's tests.",
            "inputSchema": {"type": "object", "properties": props},
            "annotations": annotations,
        })
    }

    #[test]
    fn a_clean_private_write_passes() {
        let tools = vec![create(
            json!({"title": {"type": "string"}, "body": {"type": "string"}}),
            json!({"openWorldHint": false, "readOnlyHint": false}),
        )];
        assert_private_writes(&tools, &["thing_create"]);
        assert_tool_surface(&tools, &[], &[], &[]);
    }

    #[test]
    #[should_panic(expected = "not a content field")]
    fn a_property_outside_the_allowlist_fails() {
        let tools = vec![create(
            json!({"title": {"type": "string"}, "share_with": {"type": "string"}}),
            json!({"openWorldHint": false}),
        )];
        assert_private_writes(&tools, &["thing_create"]);
    }

    #[test]
    #[should_panic(expected = "exactly the private writes")]
    fn an_unlisted_claimant_fails() {
        let tools = vec![create(
            json!({"title": {"type": "string"}}),
            json!({"openWorldHint": false}),
        )];
        assert_private_writes(&tools, &[]);
    }

    /// An absent `openWorldHint` is `Egress::None` in mecha-core, so it
    /// claims the quadrant — and must spell the decision out.
    #[test]
    #[should_panic(expected = "must say openWorldHint: false outright")]
    fn an_unannotated_claimant_must_spell_the_decision() {
        let tools = vec![create(json!({"title": {"type": "string"}}), json!({}))];
        assert_private_writes(&tools, &["thing_create"]);
    }

    /// The surface check reaches claimants it was never told about.
    #[test]
    #[should_panic(expected = "not a content field")]
    fn the_surface_check_inspects_a_claimant_nobody_listed() {
        let tools = vec![create(
            json!({"title": {"type": "string"}, "folder_id": {"type": "string"}}),
            json!({"openWorldHint": false}),
        )];
        assert_tool_surface(&tools, &[], &[], &[]);
    }

    /// One verb's vocabulary must not widen another's: `location` is a place
    /// on a hold and could be a shared folder on a document.
    #[test]
    #[should_panic(expected = "not a content field")]
    fn a_calendar_field_does_not_pass_on_another_verb() {
        let tools = vec![create(
            json!({"title": {"type": "string"}, "location": {"type": "string"}}),
            json!({"openWorldHint": false}),
        )];
        assert_private_writes(&tools, &["thing_create"]);
    }

    /// An allowed name must not carry a structure a recipient could hide in.
    #[test]
    #[should_panic(expected = "not a flat scalar")]
    fn a_nested_shape_under_an_allowed_name_fails() {
        let tools = vec![create(
            json!({"title": {"type": "string"},
                   "body": {"type": "object", "properties": {"share_with": {"type": "string"}}}}),
            json!({"openWorldHint": false}),
        )];
        assert_private_writes(&tools, &["thing_create"]);
    }

    #[test]
    #[should_panic(expected = "is in no quadrant")]
    fn a_destructive_verb_listed_nowhere_fails() {
        let tools = vec![create(
            json!({"title": {"type": "string"}}),
            json!({"destructiveHint": true}),
        )];
        assert_tool_surface(&tools, &[], &[], &[]);
    }
}
