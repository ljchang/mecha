//! A real MCP server, spawned and spoken to.
//!
//! The unit tests in `mcp.rs` inspect the `Command` before it is spawned, which
//! proves what we *intend* to hand a server. This proves what a server actually
//! receives, by asking one that reports everything it can see. The measurement
//! that motivated `env_clear()` was made exactly this way, by hand, once.
//!
//! Set `MECHA_TEST_REQUIRE_BACKENDS=1` to make a missing backend a failure.

mod support;

use mecha_core::config::McpServerConfig;
use mecha_core::mcp::McpClient;
use mecha_core::sandbox::{Backend, Sandbox, SandboxConfig};
use mecha_core::tool::{Tool, ToolCtx};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use support::*;

const IMAGE: &str = "python:3-slim";

/// Always passed through: most runtimes cannot start without them.
const BASE: [&str; 5] = ["PATH", "HOME", "LANG", "LC_ALL", "TZ"];

fn unconfined() -> Sandbox {
    Sandbox::new(SandboxConfig::default())
}

fn server(command: &str, script: &Path) -> McpServerConfig {
    McpServerConfig {
        name: "nosy".into(),
        command: command.into(),
        args: vec![script.display().to_string()],
        ..Default::default()
    }
}

async fn tool_named(tools: &[Arc<dyn Tool>], name: &str) -> Arc<dyn Tool> {
    tools
        .iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("no tool named {name}"))
        .clone()
}

async fn call(tool: &Arc<dyn Tool>, input: Value, workspace: &Path) -> String {
    let ctx = ToolCtx { workspace: workspace.to_path_buf(), ..Default::default() };
    let out = tool.call(input, &ctx).await.expect("the call itself failed");
    assert!(!out.is_error, "the server reported an error: {}", out.content);
    // Provenance, not capability: taint keys off where a result actually came
    // from, and everything an MCP server returns came from outside.
    assert!(out.external, "an MCP result was not marked as coming from outside");
    out.content
}

#[tokio::test]
async fn a_real_handshake_yields_the_servers_tools_namespaced_and_annotated() {
    if unavailable("python3", python3_available()) {
        return;
    }
    let dir = tmpdir("mcp-handshake");
    let cfg = server("python3", &fixture_server());

    let client = McpClient::connect(&cfg, &unconfined(), &dir).await.expect("handshake failed");
    let tools = client.list_tools().await.expect("tools/list failed");

    // Namespaced, so two servers can each expose a `search`.
    let names: BTreeSet<&str> = tools.iter().map(|t| t.name()).collect();
    assert_eq!(
        names,
        BTreeSet::from(["nosy__environ", "nosy__probe", "nosy__touch"]),
        "the advertised tools did not survive the handshake"
    );

    // The annotations feed the interlock, so their mapping is worth pinning.
    let environ = tool_named(&tools, "nosy__environ").await;
    assert!(environ.read_only(), "readOnlyHint was dropped");
    assert!(!environ.capabilities().external_send, "an unannotated tool became a send sink");
    assert!(
        !environ.capabilities().untrusted_input,
        "an unannotated tool would arm the interlock on every call"
    );

    let touch = tool_named(&tools, "nosy__touch").await;
    assert!(touch.capabilities().destructive, "destructiveHint was dropped");
    assert!(!touch.read_only());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_environment_a_server_actually_sees_is_the_allowlist() {
    if unavailable("python3", python3_available()) {
        return;
    }
    let dir = tmpdir("mcp-environ");
    let ours: BTreeSet<String> = std::env::vars().map(|(k, _)| k).collect();

    let Some(passthrough) = ours.iter().find(|k| !BASE.contains(&k.as_str())).cloned() else {
        return; // An environment this bare has nothing to leak.
    };

    let cfg = McpServerConfig {
        env: [("MECHA_EXPLICIT_TOKEN".to_string(), "granted".to_string())].into_iter().collect(),
        env_passthrough: vec![passthrough.clone()],
        ..server("python3", &fixture_server())
    };

    let client = McpClient::connect(&cfg, &unconfined(), &dir).await.expect("handshake failed");
    let tools = client.list_tools().await.unwrap();
    let reported = call(&tool_named(&tools, "nosy__environ").await, json!({}), &dir).await;

    let seen: BTreeSet<String> = reported
        .lines()
        .filter_map(|l| l.split_once('=').map(|(k, _)| k.to_string()))
        .collect();

    let allowed: BTreeSet<String> = BASE
        .iter()
        .map(|s| s.to_string())
        .chain([passthrough.clone(), "MECHA_EXPLICIT_TOKEN".to_string()])
        .collect();

    // Asserted as a subset rather than against a list of known secrets: the
    // bug was never about one variable. `envs()` layers onto the inherited
    // environment, so *everything* crossed, provider keys included.
    let leaked: Vec<_> = seen.difference(&allowed).collect();
    assert!(leaked.is_empty(), "the server was handed variables nobody named: {leaked:?}");

    assert!(seen.contains(&passthrough), "a named passthrough never arrived");
    assert!(seen.contains("MECHA_EXPLICIT_TOKEN"), "an explicit value never arrived");
    assert!(
        seen.len() < ours.len(),
        "the server holds as much as we do ({} vs {})",
        seen.len(),
        ours.len()
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_confined_server_loses_the_network_and_your_home_but_keeps_the_workspace() {
    if unavailable("docker", docker_available())
        || unavailable(IMAGE, docker_image_present(IMAGE))
    {
        return;
    }
    let dir = tmpdir("mcp-confined");

    // The server has to live inside the workspace: that is the only thing
    // mounted, which is the point being tested.
    let script = dir.join("nosy_mcp_server.py");
    std::fs::copy(fixture_server(), &script).unwrap();

    let cfg = McpServerConfig {
        sandbox: true,
        ..server("python3", &script)
    };
    let sandbox = Sandbox::new(SandboxConfig {
        kind: Backend::Docker,
        image: IMAGE.into(),
        ..Default::default()
    });

    let client = McpClient::connect(&cfg, &sandbox, &dir).await.expect("confined handshake failed");
    let tools = client.list_tools().await.unwrap();
    let probe = call(&tool_named(&tools, "nosy__probe").await, json!({}), &dir).await;
    let probe: Value = serde_json::from_str(&probe).expect("probe returned non-JSON");

    assert_eq!(probe["network"], json!(false), "a confined server reached the network");
    assert_eq!(probe["home_ssh_exists"], json!(false), "a confined server can see your ssh keys");
    assert_ne!(probe["uid"], json!(0), "a confined server runs as root");

    // These negatives only mean something on a machine where the positives
    // hold unconfined, so pin the one fact that is unambiguous either way: a
    // confined server does not share the host's UTS namespace.
    let host = std::process::Command::new("hostname").output().unwrap();
    let host = String::from_utf8_lossy(&host.stdout).trim().to_string();
    assert_ne!(probe["hostname"], json!(host), "the server ran outside the sandbox");

    // A confined server sees the *workspace*, which is the documented trade:
    // confined against your home directory, not against your project.
    assert_eq!(probe["cwd"], json!(dir.display().to_string()));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_confined_server_leaves_files_you_still_own() {
    if unavailable("docker", docker_available())
        || unavailable(IMAGE, docker_image_present(IMAGE))
    {
        return;
    }
    let dir = tmpdir("mcp-confined-write");
    let script = dir.join("nosy_mcp_server.py");
    std::fs::copy(fixture_server(), &script).unwrap();

    let cfg = McpServerConfig { sandbox: true, ..server("python3", &script) };
    let sandbox = Sandbox::new(SandboxConfig {
        kind: Backend::Docker,
        image: IMAGE.into(),
        ..Default::default()
    });

    let client = McpClient::connect(&cfg, &sandbox, &dir).await.expect("confined handshake failed");
    let tools = client.list_tools().await.unwrap();
    let touch = tool_named(&tools, "nosy__touch").await;
    call(&touch, json!({"name": "from-the-server.txt"}), &dir).await;

    let written = dir.join("from-the-server.txt");
    assert!(written.exists(), "the confined server's write never reached the workspace");

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Without `--user` the container writes as root and leaves files on
        // your disk you cannot delete.
        assert_eq!(
            std::fs::metadata(&written).unwrap().uid(),
            unsafe { libc::getuid() },
            "the confined server left a file you do not own"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_server_asking_for_confinement_with_no_backend_never_starts() {
    if unavailable("python3", python3_available()) {
        return;
    }
    let dir = tmpdir("mcp-unconfinable");
    let cfg = McpServerConfig { sandbox: true, ..server("python3", &fixture_server()) };

    // Through the real entry point, not just the builder: a server that asked
    // to be confined and quietly was not is the failure this refuses.
    let err = match McpClient::connect(&cfg, &unconfined(), &dir).await {
        Ok(_) => panic!("a server that asked to be confined was started unconfined"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("no sandbox backend is set"), "unexpected error: {err}");

    std::fs::remove_dir_all(&dir).ok();
}
