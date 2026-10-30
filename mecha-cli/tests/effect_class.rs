//! The closed effect class per tool (`mecha_core::tool::effect`,
//! `docs/ROLLOUT-DESIGN.md` §3.2), asked of the artifact: `mecha tools
//! --json` against an isolated home, so the registry, the config layers, the
//! MCP client and the resolver are the ones a real run builds.
//!
//! The first-party servers are stood in for by a lister fixture serving the
//! **real** definitions (`mecha-mail`'s and `mecha-docs`'s own
//! `tool_definitions`, names and annotations) under a program of the real
//! name, so a tool either server adds that mecha's table does not name shows
//! up here as `undeclared` rather than shipping that way. The graph's are
//! copied from the revision `graph_mcp::REV` pins, whose source is not in
//! this repository.
//!
//! Every assertion here fails on the naive build that derives the class
//! from `read_only()`: `message_send`, `image_generate`, `http_fetch`,
//! `persona_propose`, the readOnlyHint'd third-party tool and every graph
//! read are `read_only()`, and none of them may be a `read`.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Home {
        let dir = std::env::temp_dir().join(format!("mecha-effect-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        std::fs::create_dir_all(dir.join("work")).unwrap();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        Home(dir)
    }

    /// An executable named `program` that serves `tools` from the lister.
    fn server(&self, program: &str, tools: &Value) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let list = self.0.join(format!("{program}.json"));
        std::fs::write(&list, serde_json::to_vec(tools).unwrap()).unwrap();
        let lister = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("lister_server.py");
        let path = self.0.join("bin").join(program);
        std::fs::write(
            &path,
            format!("#!/bin/sh\nexec python3 {lister:?} {list:?}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Each tool's `effect` and `capabilities.egress`, by name.
fn effects(home: &Home) -> BTreeMap<String, (String, String)> {
    let out = Command::new(env!("CARGO_BIN_EXE_mecha"))
        .args(["tools", "--json"])
        .current_dir(home.0.join("work"))
        .env("MECHA_HOME", home.0.join("home"))
        .env("HOME", home.0.join("home"))
        .env("MECHA_SESSION_KIND", "test")
        .env_remove("MECHA_MAIL_DIR")
        .env_remove("MECHA_GOOGLE_DIR")
        .env_remove("MECHA_OUTLOOK_DIR")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .output()
        .expect("running mecha tools");
    assert!(
        out.status.success(),
        "mecha tools failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("--json is JSON");
    v.as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_string(),
                (
                    t["effect"]
                        .as_str()
                        .expect("every tool carries an effect")
                        .to_string(),
                    t["capabilities"]["egress"].as_str().unwrap().to_string(),
                ),
            )
        })
        .collect()
}

/// The graph's surface at the pinned revision: names and annotations as its
/// `tools/list` sends them, plus one tool the pin does not have.
fn graph_at_the_pin() -> Value {
    let read = json!({"readOnlyHint": true, "openWorldHint": false});
    let write = json!({"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false});
    let reads = [
        "kg_search",
        "kg_entity",
        "kg_verify",
        "kg_pending",
        "kg_shadow_queue",
        "kg_timeline",
        "kg_notes",
        "kg_related",
        "kg_task_list",
    ];
    let writes = [
        "kg_verdict",
        "kg_upsert",
        "kg_task_create",
        "kg_task_update",
    ];
    let tool = |name: &str, a: &Value| json!({"name": name, "description": "", "inputSchema": {"type": "object"}, "annotations": a});
    let mut tools: Vec<Value> = reads.iter().map(|n| tool(n, &read)).collect();
    tools.extend(writes.iter().map(|n| tool(n, &write)));
    // A newer graph's read: first-party server, unlisted tool.
    tools.push(tool("kg_dream", &read));
    Value::Array(tools)
}

/// A third-party server, with one tool per rule the operator and the
/// annotations can meet.
fn sketch() -> Value {
    let tool = |name: &str, a: Value| json!({"name": name, "description": "", "inputSchema": {"type": "object"}, "annotations": a});
    json!([
        tool("plain", json!({})),
        tool("peek", json!({"readOnlyHint": true})),
        tool("look", json!({"readOnlyHint": true})),
        tool("draw", json!({})),
        tool("wipe", json!({"destructiveHint": true})),
        tool("post", json!({"openWorldHint": true})),
        tool("queue", json!({})),
    ])
}

/// The global config: every first-party server under its real program
/// name, the sketch server with the operator's classes, and the features
/// that register the built-ins a design names.
fn write_global(home: &Home) {
    let mail = home.server(
        "mecha-mail",
        &Value::Array(mecha_mail::unified::tool_definitions(
            &["campus".to_string()],
            &Default::default(),
        )),
    );
    let docs = home.server(
        "mecha-docs",
        &Value::Array(mecha_mail::google::docs_server::tool_definitions()),
    );
    let graph = home.server("mecha-graph-mcp", &graph_at_the_pin());
    let sketch = home.server("sketch-mcp", &sketch());
    std::fs::write(
        home.0.join("home/config.toml"),
        format!(
            r#"
[sandbox]
kind = "none"
[[search]]
kind = "searxng"
base_url = "http://127.0.0.1:9"
[image]
url = "http://127.0.0.1:9"
[outbox]
tools = ["mail__mail_send", "sketch__queue"]
[features]
search = true
image = true
library = true
system = true
personas = true
messages = true
mail = true
docs = true
graph = true

[[mcp]]
name = "mail"
command = {mail:?}
[[mcp]]
name = "docs"
command = {docs:?}
[[mcp]]
name = "graph"
command = {graph:?}
prefix_tools = false
[[mcp]]
name = "sketch"
command = {sketch:?}
[mcp.effects]
look = "read"
draw = "render"
wipe = "read"
post = "read"
"#
        ),
    )
    .unwrap();
}

fn class<'a>(got: &'a BTreeMap<String, (String, String)>, name: &str) -> &'a str {
    &got.get(name)
        .unwrap_or_else(|| panic!("{name} not registered: {:?}", got.keys()))
        .0
}

#[test]
fn every_registered_tool_resolves_to_its_class() {
    let home = Home::new("all");
    write_global(&home);
    let got = effects(&home);

    // The cases the two designs name, row by row.
    let expected: &[(&str, &str)] = &[
        // Store writes that are all `read_only()`.
        ("message_send", "store_write"),
        ("persona_propose", "store_write"),
        ("image_library_propose", "store_write"),
        // Writes the library store and declares `destructive`: §3.2's
        // numbered order puts the destructive row first.
        ("image_library_reference", "irreversible"),
        ("image_generate", "render"),
        ("fs_read", "read"),
        ("fs_list", "read"),
        ("mail__mail_search", "read"),
        ("mail__mail_get_thread", "read"),
        ("mail__calendar_list_events", "read"),
        ("docs__docs_read", "read"),
        ("web_search", "read"),
        ("kg_search", "read_that_records"),
        ("kg_entity", "read_that_records"),
        ("kg_task_list", "read_that_records"),
        ("http_fetch", "irreversible"),
        ("fs_write", "workspace"),
        ("fs_edit", "workspace"),
        // `[sandbox] kind = "none"`: an unconfined shell is `Chosen`.
        ("shell", "irreversible"),
        ("mail__calendar_hold", "irreversible"),
        ("mail__mail_triage", "irreversible"),
        ("docs__docs_create", "irreversible"),
        ("docs__docs_trash", "irreversible"),
        ("kg_upsert", "irreversible"),
        ("kg_verdict", "irreversible"),
        ("kg_task_create", "irreversible"),
        ("kg_task_update", "irreversible"),
        // Routed by the operator's outbox, though `Chosen`.
        ("mail__mail_send", "staged"),
        // The same kind of send, unrouted.
        ("mail__mail_reply", "irreversible"),
        // Third party, nobody classified it.
        ("sketch__plain", "undeclared"),
        // `readOnlyHint` never promotes.
        ("sketch__peek", "undeclared"),
        // The operator's word.
        ("sketch__look", "read"),
        ("sketch__draw", "render"),
        // The operator's word, moved toward caution by the server's hints.
        ("sketch__wipe", "irreversible"),
        ("sketch__post", "irreversible"),
        // Routed, though nobody classified it: a routed call never runs.
        ("sketch__queue", "staged"),
        // A first-party server's tool mecha's table does not list.
        ("kg_dream", "undeclared"),
    ];
    for (name, want) in expected {
        assert_eq!(class(&got, name), *want, "{name}");
    }
    // `web_search` is a read whose query still leaves blind.
    assert_eq!(got["web_search"].1, "blind");

    // No first-party tool is undeclared but the one the pin lacks: a tool
    // `mecha-mail` or `mecha-docs` adds without a row here fails this.
    for (name, (effect, _)) in &got {
        let first_party = name.starts_with("mail__")
            || name.starts_with("docs__")
            || (name.starts_with("kg_") && name != "kg_dream");
        if first_party {
            assert_ne!(effect, "undeclared", "{name} has no row in mecha's table");
        }
    }

    // Every built-in registered here has a class pinned, so a new one cannot
    // land unclassified by omission: it fails until someone decides.
    let builtins: BTreeMap<&str, &str> = [
        ("fs_read", "read"),
        ("fs_list", "read"),
        ("fs_write", "workspace"),
        ("fs_edit", "workspace"),
        ("shell", "irreversible"),
        ("http_fetch", "irreversible"),
        ("goal_context", "read"),
        ("web_search", "read"),
        ("web_open", "read"),
        ("image_generate", "render"),
        ("image_library", "read"),
        ("image_library_propose", "store_write"),
        ("image_library_reference", "irreversible"),
        ("message_send", "store_write"),
        ("persona_propose", "store_write"),
        ("system_read", "read"),
        // Acts on the run's own plan, which no class in §3.2 names: owed a
        // ruling, and undeclared until it has one.
        ("todo", "undeclared"),
    ]
    .into_iter()
    .collect();
    for (name, (effect, _)) in &got {
        let from_a_server = name.contains("__") || name.starts_with("kg_");
        if from_a_server {
            continue;
        }
        let want = builtins
            .get(name.as_str())
            .unwrap_or_else(|| panic!("built-in `{name}` ({effect}) has no row here"));
        assert_eq!(effect, want, "{name}");
    }
}

/// A project's `mecha.toml` arrives with a cloned repository. It may declare
/// servers, and its list replaces the operator's, but it can neither name a
/// class nor be taken for a first-party server by its `command`.
#[test]
fn a_project_layer_buys_no_class() {
    let home = Home::new("project");
    write_global(&home);
    // The project's own `mecha-mail`, and its own word about `sketch`.
    let mail = home.server(
        "mecha-mail",
        &Value::Array(mecha_mail::unified::tool_definitions(
            &["campus".to_string()],
            &Default::default(),
        )),
    );
    let sketch = home.0.join("bin").join("sketch-mcp");
    std::fs::write(
        home.0.join("work").join("mecha.toml"),
        format!(
            "[[mcp]]\nname = \"mail\"\ncommand = {mail:?}\n\
             [[mcp]]\nname = \"sketch\"\ncommand = {sketch:?}\n[mcp.effects]\nlook = \"read\"\n"
        ),
    )
    .unwrap();
    let got = effects(&home);
    for name in ["mail__mail_search", "mail__calendar_hold", "sketch__look"] {
        assert_eq!(class(&got, name), "undeclared", "{name}");
    }
    // Routing is the outbox's, and the global `[outbox]` still applies.
    assert_eq!(class(&got, "mail__mail_send"), "staged");
    // Built-ins are mecha's code, whichever layer is in front.
    assert_eq!(class(&got, "fs_read"), "read");
}
