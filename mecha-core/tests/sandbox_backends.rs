//! The sandbox, executed rather than described.
//!
//! Every test in `sandbox.rs` asserts on the shape of an argv. That catches a
//! policy written wrong and cannot catch a policy that is right and does not
//! work — which is the case this project actually hit: `bwrap` installed,
//! `unprivileged_userns_clone=1`, and still refused. So these run something
//! through the real backend and look at what came back.
//!
//! Set `MECHA_TEST_REQUIRE_BACKENDS=1` to make a missing backend a failure.

mod support;

use mecha_core::sandbox::{Backend, Sandbox, SandboxConfig};
use support::*;

const IMAGE: &str = "debian:stable-slim";

/// Note the ordering: `kind` is forced, everything else comes from the caller.
/// Naming `image` here too would silently override a caller that set it, which
/// is exactly how the broken-image test first passed against a working image.
fn docker(cfg: SandboxConfig) -> Sandbox {
    Sandbox::new(SandboxConfig {
        kind: Backend::Docker,
        ..cfg
    })
}

/// A policy pointed at the image these tests expect to be local.
fn working() -> SandboxConfig {
    SandboxConfig {
        image: IMAGE.into(),
        ..Default::default()
    }
}

async fn run(sandbox: &Sandbox, script: &str, workspace: &std::path::Path) -> (bool, String) {
    let out = sandbox
        .command(script, workspace, workspace)
        .expect("building the confined command")
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .expect("running the confined command");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    )
}

#[tokio::test]
async fn a_disabled_sandbox_preflights_without_running_anything() {
    let dir = tmpdir("preflight-none");
    Sandbox::new(SandboxConfig::default())
        .preflight(&dir)
        .await
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn docker_preflight_proves_the_backend_actually_works() {
    if unavailable("docker", docker_available()) || unavailable(IMAGE, docker_image_present(IMAGE))
    {
        return;
    }
    let dir = tmpdir("preflight-docker");

    docker(working())
        .preflight(&dir)
        .await
        .expect("docker preflight failed on a machine where docker works");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_backend_that_cannot_run_fails_preflight_rather_than_degrading_to_unconfined() {
    if unavailable("docker", docker_available()) {
        return;
    }
    let dir = tmpdir("preflight-broken");

    // The failure mode that matters: `shell` declares *narrower* capabilities
    // when confined, and the trifecta interlock believes them. Falling back to
    // running unconfined would leave the interlock trusting a claim that
    // nothing is enforcing — worse than never configuring a sandbox at all.
    let err = docker(SandboxConfig {
        image: "mecha-nonexistent-image:no-such-tag".into(),
        ..Default::default()
    })
    .preflight(&dir)
    .await
    .expect_err("a broken backend preflighted clean")
    .to_string();

    assert!(
        err.contains("docker"),
        "the error should name the backend: {err}"
    );
    assert!(
        err.contains("image") || err.contains("daemon"),
        "the error should say what to check: {err}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn bwrap_preflight_explains_itself_when_user_namespaces_are_blocked() {
    if unavailable("bwrap", bwrap_present()) {
        return;
    }
    let dir = tmpdir("preflight-bwrap");
    let sandbox = Sandbox::new(SandboxConfig {
        kind: Backend::Bwrap,
        ..Default::default()
    });

    match sandbox.preflight(&dir).await {
        // Where user namespaces are permitted, working is the correct outcome.
        Ok(()) => {}
        Err(e) => {
            // And where they are not — Ubuntu 23.10+ with the AppArmor switch —
            // the message has to name the sysctl. "Permission denied" sends you
            // looking at file modes for an afternoon.
            let msg = e.to_string().to_lowercase();
            assert!(
                msg.contains("apparmor")
                    || msg.contains("user namespace")
                    || msg.contains("loopback"),
                "an unusable bwrap must say what to do about it: {msg}"
            );
        }
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_confined_command_loses_the_network_your_home_and_your_environment() {
    if unavailable("docker", docker_available()) || unavailable(IMAGE, docker_image_present(IMAGE))
    {
        return;
    }
    let dir = tmpdir("confined");
    let sandbox = docker(working());

    let (ok, uid) = run(&sandbox, "id -u", &dir).await;
    assert!(ok, "could not run inside the container");
    // Root inside the container writing to a bind mount leaves root-owned
    // files on the host that the user cannot delete. `--user` is what prevents
    // it, and it is invisible in any argv-shaped test that never runs.
    assert_ne!(uid, "0", "the confined command ran as root");
    #[cfg(unix)]
    assert_eq!(uid, unsafe { libc::getuid() }.to_string());

    let (_, home) = run(
        &sandbox,
        r#"[ -d "$HOME/.ssh" ] && echo YES || echo NO"#,
        &dir,
    )
    .await;
    assert_eq!(home, "NO", "the confined command can see your ssh keys");

    // No network is the single line that lets the interlock *relax*: with no
    // way out, a confined shell stops being an exfiltration sink.
    let (_, net) = run(
        &sandbox,
        "timeout 3 bash -c 'echo > /dev/tcp/1.1.1.1/53' 2>/dev/null && echo YES || echo NO",
        &dir,
    )
    .await;
    assert_eq!(net, "NO", "a confined command reached the network");

    let (_, count) = run(&sandbox, "env | wc -l", &dir).await;
    let count: usize = count.parse().expect("a count");
    assert!(
        count < 12,
        "the container inherited {count} environment variables"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn network_is_available_when_the_policy_asks_for_it() {
    if unavailable("docker", docker_available()) || unavailable(IMAGE, docker_image_present(IMAGE))
    {
        return;
    }
    let dir = tmpdir("confined-net");

    // The other half of the previous test: if `network = true` did not actually
    // open it, the "no network" assertion above would pass for the wrong reason
    // and prove nothing.
    let sandbox = docker(SandboxConfig {
        network: true,
        ..working()
    });
    let (_, net) = run(
        &sandbox,
        "timeout 5 bash -c 'echo > /dev/tcp/1.1.1.1/53' 2>/dev/null && echo YES || echo NO",
        &dir,
    )
    .await;
    assert_eq!(net, "YES", "`network = true` did not reach the network");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn files_written_inside_the_sandbox_stay_yours_on_the_host() {
    if unavailable("docker", docker_available()) || unavailable(IMAGE, docker_image_present(IMAGE))
    {
        return;
    }
    let dir = tmpdir("confined-write");
    let sandbox = docker(working());

    let (ok, _) = run(&sandbox, "echo hello > written.txt", &dir).await;
    assert!(ok, "the confined write failed");

    let written = dir.join("written.txt");
    assert_eq!(std::fs::read_to_string(&written).unwrap().trim(), "hello");

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(&written).unwrap().uid(),
            unsafe { libc::getuid() },
            "the sandbox left a file you do not own"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}
