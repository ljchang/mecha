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

/// The verbs that start the graph server themselves — `distill`, `gossip`,
/// `vet`, `corroborate` — ask the switch too, so `graph = false` is not
/// answered one way by `mecha tasks` and the other by `mecha vet` (review of
/// #445). `vet` needs no arguments, so it stands for the four; the refusal
/// comes before any server is spawned or any provider built.
#[test]
fn a_verb_that_starts_the_graph_itself_refuses_when_it_is_off() {
    let home = Home::new("vet");
    let wrapper = graph_wrapper(&home.0);
    for features in ["", "[features]\ngraph = false"] {
        std::fs::write(
            home.0.join("home/config.toml"),
            format!(
                "[sandbox]\nkind = \"none\"\n[[mcp]]\nname = \"graph\"\ncommand = {wrapper:?}\nsandbox = false\n{features}\n"
            ),
        )
        .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(["vet"])
            .current_dir(home.0.join("work"))
            .env("MECHA_HOME", home.0.join("home"))
            .env("HOME", home.0.join("home"))
            .env_remove("ANTHROPIC_API_KEY")
            .output()
            .expect("running mecha vet");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{features:?}: {err}");
        assert!(
            err.contains("belongs to `graph`, which is not switched on"),
            "{features:?}: {err}"
        );
    }
}

/// Step 3a: every verb that belongs to a switched-off feature refuses first,
/// with the one sentence the web's `feature_off` carries — label, why, and
/// the command — whatever else its config holds. The negative is not
/// vacuous: with the switches on, none of them says it (they may fail for
/// other reasons here — no server — which is the point: the refusal is the
/// switch's alone). And the verbs deliberately left open never refuse:
/// reading the library, pruning a cache, the cross-feature queue list.
#[test]
fn a_switched_off_feature_s_verbs_refuse_with_one_sentence() {
    let home = Home::new("verbs");
    let run = |features: &str, argv: &[&str]| {
        std::fs::write(
            home.0.join("home/config.toml"),
            format!("[sandbox]\nkind = \"none\"\n{features}\n"),
        )
        .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(argv)
            .current_dir(home.0.join("work"))
            .env("MECHA_HOME", home.0.join("home"))
            .env("HOME", home.0.join("home"))
            .env_remove("MECHA_MAIL_DIR")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .output()
            .expect("running mecha");
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let off = "[features]\nmail = false\ngraph = false\ndocuments = false\nimage = false\n\
               slack = false\npersonas = false\nfrontdoor = false\nmessages = false\nvoice = false";
    let on = "[features]\nmail = true\ngraph = true\ndocuments = true\nimage = true\n\
              slack = true\npersonas = true\nfrontdoor = true\nmessages = true\nvoice = true";
    for (argv, sentence) in [
        (
            &["mail", "list"][..],
            "Mail and calendar is off (turned off in [features]) — `mecha features enable mail`",
        ),
        (
            &["tasks", "list"][..],
            "Task board is off (needs knowledge graph) — `mecha features enable graph`",
        ),
        (
            &["kg", "notes"][..],
            "Knowledge graph is off (turned off in [features]) — `mecha features enable graph`",
        ),
        (
            &["review", "list"][..],
            "Knowledge graph is off (turned off in [features]) — `mecha features enable graph`",
        ),
        (
            &["document", "extract", "missing.pdf"][..],
            "PDF extraction is off (turned off in [features]) — `mecha features enable documents`",
        ),
        (
            &["imagelib", "remove", "nobody"][..],
            "Character and style library is off (needs image generation) — `mecha features enable image`",
        ),
        // Step 3b.
        (
            &["slack", "send", "/nonexistent.png"][..],
            "Slack remote control is off (turned off in [features]) — `mecha features enable slack`",
        ),
        (
            &["slack", "remote", "--sweep"][..],
            "Slack remote control is off (turned off in [features]) — `mecha features enable slack`",
        ),
        (
            &["persona", "new", "ada"][..],
            "Personas is off (turned off in [features]) — `mecha features enable personas`",
        ),
        (
            &["frontdoor", "list"][..],
            "Front door (inbound requests) is off (turned off in [features]) — `mecha features enable frontdoor`",
        ),
        (
            &["polls", "list"][..],
            "Front door (inbound requests) is off (turned off in [features]) — `mecha features enable frontdoor`",
        ),
        // A `false` like every other switch's: once it read as unanswered
        // ("not enabled"), because `[messages] enabled` was a plain bool.
        (
            &["msg", "list"][..],
            "Messages between sessions is off (turned off in [features]) — `mecha features enable messages`",
        ),
    ] {
        let err = run(off, argv);
        assert!(err.contains(sentence), "{argv:?} off:\n{err}");
        // What only a refusal prints: the enable command. (" is off (" is
        // also in the unconfigured-documents message, a different finding.)
        let err = run(on, argv);
        assert!(
            !err.contains("`mecha features enable"),
            "{argv:?} on:\n{err}"
        );
    }
    // Off only: switched on, `voice-serve` binds and serves until stopped.
    let err = run(off, &["voice-serve"]);
    assert!(
        err.contains("Voice is off (turned off in [features]) — `mecha features enable voice`"),
        "{err}"
    );
    for argv in [
        &["imagelib", "list"][..],
        &["document", "prune"][..],
        &["review", "queues"][..],
        // Step 3b: reading what is here, and the way Slack is set up.
        &["slack", "status"][..],
        // "What is this machine mirroring", answerable when Slack is the
        // thing that is wrong (review of #452).
        &["slack", "remote"][..],
        &["persona", "list"][..],
    ] {
        let err = run(off, argv);
        assert!(
            !err.contains("`mecha features enable"),
            "{argv:?} must stay open:\n{err}"
        );
    }
}

/// A guard that cannot read the configuration refuses rather than letting
/// the verb through (review of #451): `imagelib` loads no config of its own,
/// so a malformed file would otherwise have let `imagelib remove` run with
/// `image = false` written in it.
#[test]
fn an_unreadable_configuration_refuses_a_feature_s_verb() {
    let home = Home::new("badcfg");
    std::fs::write(
        home.0.join("home/config.toml"),
        "[features]\nimage = false\nthis is not toml\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mecha"))
        .args(["imagelib", "remove", "nobody"])
        .current_dir(home.0.join("work"))
        .env("MECHA_HOME", home.0.join("home"))
        .env("HOME", home.0.join("home"))
        .output()
        .expect("running mecha imagelib");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{err}");
    assert!(
        err.contains("cannot tell whether `image` is switched on"),
        "{err}"
    );
}

/// Step 3b: the front door is gated whole — its publishing server is not
/// connected with `frontdoor` off, as mail's, docs' and the graph's are not
/// with theirs. A fixture server whose command's file name is
/// `factory-publish` stands in for it, its tools prefixed `factory__`.
#[test]
fn the_publishing_server_connects_only_with_the_front_door_on() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new("publish");
    let wrapper = home.0.join("factory-publish");
    std::fs::write(
        &wrapper,
        format!("#!/bin/sh\nexec python3 {:?} \"$@\"\n", board_server()),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let board = home.0.join("board");
    std::fs::create_dir_all(&board).unwrap();
    let tools = |features: &str| -> Vec<String> {
        std::fs::write(
            home.0.join("home/config.toml"),
            format!(
                "[sandbox]\nkind = \"none\"\n{features}\n[[mcp]]\nname = \"factory\"\n\
                 command = {wrapper:?}\nsandbox = false\n[mcp.env]\nMECHA_FIXTURE_DIR = {board:?}\n"
            ),
        )
        .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(["tools", "--json"])
            .current_dir(home.0.join("work"))
            .env("MECHA_HOME", home.0.join("home"))
            .env("HOME", home.0.join("home"))
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .output()
            .expect("running mecha tools");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        v.as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    };
    let off = tools("[features]\nfrontdoor = false");
    assert!(
        !off.iter().any(|t| t.starts_with("factory__")),
        "the publishing server connected with the front door off: {off:?}"
    );
    let on = tools("[features]\nfrontdoor = true");
    assert!(
        on.iter().any(|t| t.starts_with("factory__")),
        "the publishing server did not connect with the front door on: {on:?}"
    );
}
