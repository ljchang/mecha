//! Step 1b of `docs/FEATURES-DESIGN.md`: tools and feature servers register
//! only when their `[features]` switch is on.
//!
//! Driven through the real binary (`mecha tools --json`, which builds the
//! registry without a provider) against an isolated home, because the gate
//! lives in `setup::prepare_tools`, which loads its own config. The negative
//! is not vacuous: the same config with the switches on must show the tools,
//! or a gate that refused everything would pass.

use std::path::{Path, PathBuf};
use std::process::Command;

fn board_server() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("eval")
        .join("fixtures")
        .join("board_server.py")
}

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Home {
        let dir = std::env::temp_dir().join(format!("mecha-gate-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        std::fs::create_dir_all(dir.join("work")).unwrap();
        Home(dir)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fixture MCP server whose command's file name is `mecha-graph-mcp`, so
/// the gate reads it as the knowledge graph (`feature::server_feature`).
fn graph_wrapper(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("mecha-graph-mcp");
    std::fs::write(
        &path,
        format!("#!/bin/sh\nexec python3 {:?} \"$@\"\n", board_server()),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn tools(home: &Home, features: &str) -> Vec<String> {
    tools_with(home, features, &[]).0
}

/// The registry's tool names, and what the build said on stderr.
fn tools_with(home: &Home, features: &str, extra: &[&str]) -> (Vec<String>, String) {
    let wrapper = graph_wrapper(&home.0);
    let board = home.0.join("board");
    std::fs::create_dir_all(&board).unwrap();
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
[[mcp]]
name = "graph"
command = {wrapper:?}
prefix_tools = false
sandbox = false
[mcp.env]
MECHA_FIXTURE_DIR = {board:?}
{features}
"#
        ),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mecha"))
        .args(["tools", "--json"])
        .args(extra)
        .current_dir(home.0.join("work"))
        .env("MECHA_HOME", home.0.join("home"))
        .env("HOME", home.0.join("home"))
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
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("--json is JSON");
    let names = v
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    (names, String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn a_feature_registers_only_when_its_switch_is_on() {
    let home = Home::new("gate");
    let off = tools(&home, "");
    for name in ["web_search", "image_generate"] {
        assert!(
            !off.iter().any(|t| t == name),
            "{name} registered with its switch absent: {off:?}"
        );
    }
    assert!(
        !off.iter().any(|t| t.starts_with("kg_task")),
        "the graph server connected with its switch absent: {off:?}"
    );
    // A tool that belongs to no feature is untouched by all of it.
    assert!(off.iter().any(|t| t == "fs_read"), "{off:?}");

    let on = tools(
        &home,
        "[features]\nsearch = true\nimage = true\ngraph = true",
    );
    for name in ["web_search", "image_generate"] {
        assert!(
            on.iter().any(|t| t == name),
            "{name} missing with its switch on: {on:?}"
        );
    }
    assert!(
        on.iter().any(|t| t.starts_with("kg_task")),
        "the graph server did not connect with its switch on: {on:?}"
    );

    // An explicit `false` is as off as an absent key.
    let no = tools(
        &home,
        "[features]\nsearch = false\nimage = true\ngraph = false",
    );
    assert!(!no.iter().any(|t| t == "web_search"), "{no:?}");
    assert!(no.iter().any(|t| t == "image_generate"), "{no:?}");
    assert!(!no.iter().any(|t| t.starts_with("kg_task")), "{no:?}");
}

/// A gated tool is said only when the run named it: every verb that builds a
/// registry passes through the gate, including the children `mecha serve`
/// spawns per request, so a line per build would repeat into the journal
/// (review of #445).
#[test]
fn a_switched_off_tool_is_named_only_when_asked_for() {
    let home = Home::new("quiet");
    let (_, quiet) = tools_with(&home, "", &[]);
    assert!(!quiet.contains("not switched on"), "{quiet}");
    let (names, said) = tools_with(&home, "", &["--tool", "web_search"]);
    assert!(!names.iter().any(|t| t == "web_search"));
    assert!(
        said.contains("web_search not registered") && said.contains("mecha features enable search"),
        "{said}"
    );
}

/// `mecha serve` refuses without `web`, and the two ways it can be
/// unanswered read differently — one command from working, or the owner's
/// choice (review of #445: the refusal was unmeasured).
#[test]
fn serve_refuses_without_web_and_says_which_way() {
    for (features, expect) in [
        ("", "predates the switch"),
        ("[features]\nweb = false", "turned off"),
    ] {
        let home = Home::new(if features.is_empty() {
            "serve-absent"
        } else {
            "serve-false"
        });
        std::fs::write(
            home.0.join("home/config.toml"),
            format!("[sandbox]\nkind = \"none\"\n[web]\nowner_login = \"someone\"\n{features}\n"),
        )
        .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(["serve", "--port", "0"])
            .current_dir(home.0.join("work"))
            .env("MECHA_HOME", home.0.join("home"))
            .env("HOME", home.0.join("home"))
            .env_remove("ANTHROPIC_API_KEY")
            .output()
            .expect("running mecha serve");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success(),
            "serve started with {features:?}: {err}"
        );
        assert!(err.contains(expect), "{features:?}: {err}");
    }
}
