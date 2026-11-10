//! Authored selected-graph boundary; never opens a real graph store.
mod support;
use mecha_core::config::{CapabilityOverride, McpServerConfig};
use mecha_core::mcp::McpClient;
use mecha_core::sandbox::Sandbox;
use mecha_core::tool::{Tool, ToolCtx};
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

struct Fixture {
    root: std::path::PathBuf,
    cfg: McpServerConfig,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        Self::in_workspace(support::tmpdir("selected-graph"), mode)
    }

    fn in_workspace(root: std::path::PathBuf, mode: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let program = root.as_path().join("mecha-graph-mcp");
        std::fs::write(&program, include_str!("fixtures/selected_graph_mcp.py")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let cfg = McpServerConfig {
            name: "chosen".into(),
            command: program.display().to_string(),
            from_operator: true,
            capabilities: CapabilityOverride {
                untrusted_input: true,
                ..Default::default()
            },
            env: [("GRAPH_FIXTURE_MODE".into(), mode.into())].into(),
            ..Default::default()
        };
        Self { root, cfg }
    }
}

async fn selected(
    cfg: &McpServerConfig,
    workspace: &Path,
) -> anyhow::Result<(Arc<McpClient>, Vec<Arc<dyn Tool>>)> {
    let selected = mecha_core::mcp::connect_selected_graph(
        cfg,
        &Sandbox::default(),
        workspace,
        &["kg_search".to_string()].into(),
    )
    .await?;
    Ok((
        Arc::clone(selected.client()),
        selected
            .tools()
            .iter()
            .map(|tool| Arc::clone(tool.tool()))
            .collect(),
    ))
}

#[tokio::test]
async fn selected_graph_refuses_ambiguous_or_incomplete_inventory() {
    for mode in [
        "duplicate",
        "duplicate_page",
        "forever",
        "bad_cursor",
        "schema",
        "missing",
    ] {
        let f = Fixture::new(mode);
        assert!(
            selected(&f.cfg, f.root.as_path()).await.is_err(),
            "accepted {mode}"
        );
    }
}

#[tokio::test]
async fn selected_graph_refuses_unsafe_effects_and_capabilities() {
    for mode in ["open", "destructive", "write", "deferred"] {
        let f = Fixture::new(mode);
        assert!(
            selected(&f.cfg, f.root.as_path()).await.is_err(),
            "accepted {mode}"
        );
    }
    let mut f = Fixture::new("ok");
    f.cfg.effects.insert(
        "kg_search".into(),
        mecha_core::tool::effect::Effect::StoreWrite,
    );
    assert!(selected(&f.cfg, f.root.as_path()).await.is_err());
}

#[tokio::test]
async fn selected_graph_requires_enabled_operator_graph_origin_before_spawn() {
    for mode in ["project", "disabled", "wrong_server"] {
        let mut f = Fixture::new("ok");
        let marker = f.root.as_path().join("spawned");
        f.cfg
            .env
            .insert("GRAPH_FIXTURE_MARKER".into(), marker.display().to_string());
        match mode {
            "project" => f.cfg.from_operator = false,
            "disabled" => f.cfg.disabled = true,
            _ => {
                let other = f.root.as_path().join("other-server");
                std::fs::copy(&f.cfg.command, &other).unwrap();
                f.cfg.command = other.display().to_string();
            }
        }
        assert!(
            selected(&f.cfg, f.root.as_path()).await.is_err(),
            "accepted {mode}"
        );
        assert!(!marker.exists(), "spawned {mode}");
    }
}

#[tokio::test]
async fn selected_graph_nontext_empty_and_malformed_results_are_external_errors() {
    let f = Fixture::new("ok");
    let (client, tools) = selected(&f.cfg, f.root.as_path()).await.unwrap();
    let ctx = ToolCtx {
        workspace: f.root.as_path().to_path_buf(),
        ..Default::default()
    };
    for mode in ["image", "invalid", "empty"] {
        let out = tools[0].call(json!({"mode":mode}), &ctx).await.unwrap();
        assert!(
            out.is_error && out.external,
            "accepted {mode}: {}",
            out.content
        );
        assert!(out.content.len() < 512);
    }
    client.close().await.unwrap();
}

#[tokio::test]
async fn selected_graph_keeps_exact_text_and_external_failures() {
    let mut f = Fixture::new("ok");
    f.cfg.prefix_tools = Some(false);
    let (client, tools) = selected(&f.cfg, f.root.as_path()).await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name(), "kg_search");
    let ctx = ToolCtx {
        workspace: f.root.as_path().to_path_buf(),
        ..Default::default()
    };
    let out = tools[0].call(json!({}), &ctx).await.unwrap();
    assert!(!out.is_error && out.external);
    assert_eq!(out.content, "Authored snapshot: project Cedar is active.");
    let out = tools[0].call(json!({"mode":"large"}), &ctx).await.unwrap();
    assert_eq!(out.content.len(), 20000);
    for mode in ["wire_error", "exit"] {
        let out = tools[0].call(json!({"mode":mode}), &ctx).await.unwrap();
        assert!(out.is_error && out.external);
    }
    client.close().await.unwrap();
}

#[tokio::test]
async fn selected_graph_inventory_binds_actual_origin_mapping_schema_and_description() {
    let mut f = Fixture::new("ok");
    f.cfg.capabilities.untrusted_input = false;
    let selected_names = ["kg_search".to_string(), "kg_entity".to_string()].into();
    let graph = mecha_core::mcp::connect_selected_graph(
        &f.cfg,
        &Sandbox::default(),
        &f.root,
        &selected_names,
    )
    .await
    .unwrap();
    assert_eq!(graph.tools().len(), 2);
    let descriptor = graph.tools()[1].descriptor().clone();
    assert_eq!(descriptor.server_name, "chosen");
    assert_eq!(descriptor.local_name, "chosen__kg_search");
    assert_eq!(descriptor.remote_name, "kg_search");
    assert!(descriptor.capabilities.untrusted_input && descriptor.capabilities.private_data);
    assert_eq!(descriptor.fixed_workspace, f.root.canonicalize().unwrap());
    assert_eq!(descriptor.schema_digest.len(), 64);
    graph.client().close().await.unwrap();
    f.cfg
        .env
        .insert("GRAPH_FIXTURE_MODE".into(), "description".into());
    let changed = mecha_core::mcp::connect_selected_graph(
        &f.cfg,
        &Sandbox::default(),
        &f.root,
        &selected_names,
    )
    .await
    .unwrap();
    assert_eq!(
        changed.tools()[1].descriptor().schema_digest,
        descriptor.schema_digest
    );
    assert_ne!(
        changed.tools()[1].descriptor().description_digest,
        descriptor.description_digest
    );
    changed.client().close().await.unwrap();
}

#[tokio::test]
async fn selected_graph_launch_digest_binds_config_and_freezes_the_spawn() {
    let mut f = Fixture::new("ok");
    let marker = f.root.join("spawned");
    f.cfg
        .env
        .insert("GRAPH_FIXTURE_MARKER".into(), marker.display().to_string());
    f.cfg
        .env
        .insert("GRAPH_TEST_EXPLICIT".into(), "frozen".into());
    f.cfg.owner_zone = Some("UTC".into());
    let names = ["kg_search".to_string()].into();
    let prepared =
        mecha_core::mcp::prepare_selected_graph(&f.cfg, &Sandbox::default(), &f.root, &names)
            .unwrap();
    let digest = prepared.launch_digest().to_string();
    assert!(!marker.exists(), "preparing a launch spawned the child");
    let again =
        mecha_core::mcp::prepare_selected_graph(&f.cfg, &Sandbox::default(), &f.root, &names)
            .unwrap();
    assert_eq!(again.launch_digest(), digest);
    f.cfg
        .env
        .insert("GRAPH_TEST_EXPLICIT".into(), "changed".into());
    let changed =
        mecha_core::mcp::prepare_selected_graph(&f.cfg, &Sandbox::default(), &f.root, &names)
            .unwrap();
    assert_ne!(changed.launch_digest(), digest);
    let graph = prepared.connect().await.unwrap();
    assert_eq!(graph.launch_digest(), digest);
    let ctx = ToolCtx {
        workspace: f.root.clone(),
        ..Default::default()
    };
    let out = graph.tools()[0]
        .tool()
        .call(json!({"mode":"env"}), &ctx)
        .await
        .unwrap();
    let env: serde_json::Value = serde_json::from_str(&out.content).unwrap();
    assert_eq!(env["GRAPH_TEST_EXPLICIT"], "frozen");
    assert_eq!(env["MECHA_TZ"], "UTC");
    assert_eq!(env["ANTHROPIC_API_KEY"], serde_json::Value::Null);
    assert!(marker.exists());
    graph.client().close().await.unwrap();
}

#[test]
fn selected_graph_launch_binds_every_policy_axis_and_refuses_disabled_tools() {
    let f = Fixture::new("ok");
    let names = ["kg_search".to_string()].into();
    let digest = |cfg: &McpServerConfig| {
        mecha_core::mcp::prepare_selected_graph(cfg, &Sandbox::default(), &f.root, &names)
            .unwrap()
            .launch_digest()
            .to_string()
    };
    let base = digest(&f.cfg);
    for change in 0..7 {
        let mut config = f.cfg.clone();
        match change {
            0 => config.args.push("--authored-argument".into()),
            1 => config.owner_zone = Some("UTC".into()),
            2 => config.prefix_tools = Some(false),
            3 => config.trust_result_claims = true,
            4 => config.disabled_tools.push("kg_entity".into()),
            5 => {
                config.effects.insert(
                    "kg_search".into(),
                    mecha_core::tool::effect::Effect::StoreWrite,
                );
            }
            _ => config.capabilities.destructive = true,
        }
        assert_ne!(digest(&config), base, "unbound configuration axis {change}");
    }
    let mut config = f.cfg.clone();
    config.disabled_tools.push("kg_search".into());
    assert!(
        mecha_core::mcp::prepare_selected_graph(&config, &Sandbox::default(), &f.root, &names)
            .is_err()
    );
    config = f.cfg.clone();
    config.sandbox = true;
    assert!(
        mecha_core::mcp::prepare_selected_graph(&config, &Sandbox::default(), &f.root, &names)
            .is_err()
    );
    let sandbox = Sandbox::new(mecha_core::sandbox::SandboxConfig {
        network: true,
        ..Default::default()
    });
    let other = mecha_core::mcp::prepare_selected_graph(&f.cfg, &sandbox, &f.root, &names).unwrap();
    assert_ne!(other.launch_digest(), base);
}

#[tokio::test]
async fn ordinary_mcp_retains_its_compatible_nontext_conversion() {
    let f = Fixture::new("ok");
    let client = McpClient::connect(&f.cfg, &Sandbox::default(), &f.root)
        .await
        .unwrap();
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 4);
    let ctx = ToolCtx {
        workspace: f.root.clone(),
        ..Default::default()
    };
    let out = tools[0].call(json!({"mode":"image"}), &ctx).await.unwrap();
    assert!(!out.is_error && out.external);
    assert_eq!(out.content, "[image content omitted]");
    client.close().await.unwrap();
}

#[tokio::test]
async fn selected_graph_refuses_malformed_nested_schema_and_annotation_types() {
    for mode in ["nested_schema", "bad_annotations", "bad_background"] {
        let f = Fixture::new(mode);
        assert!(selected(&f.cfg, &f.root).await.is_err(), "accepted {mode}");
    }
}

#[tokio::test]
async fn selected_graph_bwrap_delivers_frozen_environment_and_workspace() {
    if support::unavailable("bwrap", support::bwrap_present())
        || support::unavailable("python3", support::python3_available())
    {
        return;
    }
    // Give this test a known inherited nonsecret and a known forbidden sentinel
    // without mutating the environment of concurrently running Rust tests.
    const CHILD: &str = "MECHA_GRAPH_BWRAP_TEST_CHILD";
    if std::env::var(CHILD).as_deref() != Ok("1") {
        let fixture = Fixture::new("ok");
        let mut digests = Vec::new();
        for override_value in ["authored-sandbox-first", "authored-sandbox-second"] {
            let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "selected_graph_bwrap_delivers_frozen_environment_and_workspace",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("MECHA_GRAPH_BWRAP_WORKSPACE", &fixture.root)
                .env("MECHA_GRAPH_ALLOWED", "authored-passthrough")
                .env("MECHA_GRAPH_OVERRIDE", override_value)
                .env("MECHA_RUN_POSTURE", "authored-forged-posture")
                .env("ANTHROPIC_API_KEY", "authored-forbidden-sentinel")
                .kill_on_drop(true);
            let output = tokio::time::timeout(std::time::Duration::from_secs(45), command.output())
                .await
                .expect("selected bwrap probe exceeded its deadline")
                .unwrap();
            assert!(
                output.status.success(),
                "isolated selected bwrap probe failed: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8(output.stdout).unwrap();
            let digest = stdout
                .lines()
                .find_map(|line| line.strip_prefix("GRAPH_LAUNCH_DIGEST="))
                .expect("selected launch digest was not reported")
                .to_string();
            digests.push(digest);
        }
        assert_ne!(
            digests[0], digests[1],
            "sandbox environment values must bind the launch digest"
        );
        std::fs::remove_dir_all(fixture.root).unwrap();
        return;
    }
    assert_eq!(
        std::env::var("ANTHROPIC_API_KEY").unwrap(),
        "authored-forbidden-sentinel"
    );
    let mut f = Fixture::in_workspace(
        std::env::var_os("MECHA_GRAPH_BWRAP_WORKSPACE")
            .unwrap()
            .into(),
        "ok",
    );
    // Extend only this copy of the authored fixture; the shared server fixture
    // and ordinary MCP tests retain their existing output contract.
    let script = std::fs::read_to_string(&f.cfg.command).unwrap();
    let script = script.replace(
        r#"json.dumps({key: os.environ.get(key) for key in ["GRAPH_TEST_EXPLICIT", "MECHA_TZ", "ANTHROPIC_API_KEY"]})"#,
        r#"json.dumps({"cwd": os.getcwd(), "explicit": os.environ.get("GRAPH_TEST_EXPLICIT"), "passthrough": os.environ.get("MECHA_GRAPH_ALLOWED"), "secret": os.environ.get("ANTHROPIC_API_KEY"), "posture": os.environ.get("MECHA_RUN_POSTURE"), "zone": os.environ.get("MECHA_TZ"), "home": os.environ.get("HOME"), "path": os.environ.get("PATH"), "override": os.environ.get("MECHA_GRAPH_OVERRIDE")})"#,
    );
    std::fs::write(&f.cfg.command, script).unwrap();
    f.cfg.sandbox = true;
    f.cfg.env_passthrough = vec!["MECHA_GRAPH_ALLOWED".into()];
    f.cfg
        .env
        .insert("GRAPH_TEST_EXPLICIT".into(), "authored-explicit".into());
    f.cfg.owner_zone = Some("UTC".into());
    f.cfg
        .env
        .insert("MECHA_GRAPH_OVERRIDE".into(), "authored-overridden".into());
    let sandbox = Sandbox::new(mecha_core::sandbox::SandboxConfig {
        kind: mecha_core::sandbox::Backend::Bwrap,
        env: vec!["MECHA_GRAPH_OVERRIDE".into(), "MECHA_RUN_POSTURE".into()],
        ..Default::default()
    });
    let prepared = mecha_core::mcp::prepare_selected_graph(
        &f.cfg,
        &sandbox,
        &f.root,
        &["kg_search".to_string()].into(),
    )
    .expect("selected graph bwrap preparation failed");
    println!("GRAPH_LAUNCH_DIGEST={}", prepared.launch_digest());
    let graph = prepared
        .connect()
        .await
        .expect("selected graph bwrap startup failed");
    let listed = &graph.tools()[0];
    assert_eq!(
        listed.descriptor().fixed_workspace,
        f.root.canonicalize().unwrap()
    );
    let out = listed
        .tool()
        .call(
            json!({"mode": "env"}),
            &ToolCtx {
                workspace: f.root.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        !out.is_error && out.external,
        "selected bwrap response failed"
    );
    let report: serde_json::Value = serde_json::from_str(&out.content).unwrap();
    assert_eq!(report["cwd"], f.root.display().to_string());
    assert_eq!(report["home"], f.root.display().to_string());
    assert_eq!(
        report["path"],
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    );
    assert_eq!(
        report["override"],
        std::env::var("MECHA_GRAPH_OVERRIDE").unwrap()
    );
    assert_eq!(report["explicit"], "authored-explicit");
    assert_eq!(report["passthrough"], "authored-passthrough");
    assert_eq!(report["zone"], "UTC");
    assert_eq!(report["posture"], mecha_core::closure::RunPosture::UNKNOWN);
    assert!(
        report["secret"].is_null(),
        "forbidden inherited sentinel reached the server"
    );
    graph.client().close().await.unwrap();
}
