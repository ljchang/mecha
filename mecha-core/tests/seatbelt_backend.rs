//! The macOS Seatbelt confinement, measured: a process under the profile
//! `Sandbox` builds cannot read a file outside its inputs, cannot write
//! outside its scratch directory, and cannot reach the network — and poppler
//! still renders a page under it, through the same `Extractor` a document
//! takes.
//!
//! Every negative is checked against its positive first: the file outside
//! is readable unconfined, the address answers unconfined, and the network
//! denial is shown to be the network rule by the same call succeeding with
//! `network = true`. A refusal that would have happened anyway proves
//! nothing.
//!
//! macOS only. Skips when poppler or the network is missing;
//! `MECHA_TEST_REQUIRE_BACKENDS=1` turns each skip into a failure.

#![cfg(target_os = "macos")]

mod support;

use mecha_core::document::{DocumentsConfig, Extractor, Mode, Renderer};
use mecha_core::sandbox::{Backend, Sandbox, SandboxConfig};
use std::path::Path;
use std::process::Output;

const LINE: &str = "Priya reviewed the ledger with Ada on Tuesday.";

fn seatbelt(network: bool) -> Sandbox {
    Sandbox::new(SandboxConfig {
        kind: Backend::Seatbelt,
        network,
        ..SandboxConfig::default()
    })
}

async fn run(sandbox: &Sandbox, program: &str, args: &[&str], ws: &Path) -> Output {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    sandbox
        .wrap_argv(program, &args, ws, ws)
        .unwrap()
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .unwrap()
}

/// The scratch directory is readable and writable; a file beside it is
/// neither, though it is readable unconfined.
#[tokio::test]
async fn the_profile_reaches_its_scratch_and_nothing_else_on_disk() {
    let ws = support::tmpdir("sb-ws");
    let outside = support::tmpdir("sb-outside");
    let secret = outside.join("id_ed25519");
    std::fs::write(&secret, "SECRET-KEY-MATERIAL").unwrap();
    std::fs::write(ws.join("in.txt"), "THE-INPUT").unwrap();

    // Not vacuous: the file is there and readable without the profile.
    let plain = std::process::Command::new("/bin/cat")
        .arg(&secret)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&plain.stdout),
        "SECRET-KEY-MATERIAL"
    );

    let sb = seatbelt(false);
    let inside = run(&sb, "/bin/cat", &[ws.join("in.txt").to_str().unwrap()], &ws).await;
    assert!(
        inside.status.success(),
        "the scratch is readable: {}",
        String::from_utf8_lossy(&inside.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&inside.stdout), "THE-INPUT");

    let read = run(&sb, "/bin/cat", &[secret.to_str().unwrap()], &ws).await;
    assert!(!read.status.success(), "a file outside its inputs was read");
    assert!(!String::from_utf8_lossy(&read.stdout).contains("SECRET"));

    let beside = outside.join("planted");
    let write = run(&sb, "/usr/bin/touch", &[beside.to_str().unwrap()], &ws).await;
    assert!(
        !write.status.success() && !beside.exists(),
        "a write landed outside the scratch"
    );

    let ok = ws.join("made-here");
    let write = run(&sb, "/usr/bin/touch", &[ok.to_str().unwrap()], &ws).await;
    assert!(
        write.status.success() && ok.exists(),
        "the scratch is writable: {}",
        String::from_utf8_lossy(&write.stderr)
    );

    std::fs::remove_dir_all(ws).ok();
    std::fs::remove_dir_all(outside).ok();
}

/// No network unless asked: the same request answers unconfined and with
/// `network = true`, and fails under the default profile. An address, not a
/// name, so the denial measured is the connection's, not the resolver's.
#[tokio::test]
async fn the_network_is_closed_unless_the_call_opens_it() {
    let url = "http://1.1.1.1/";
    let curl = |extra: &[&'static str]| {
        let mut a: Vec<&str> = vec!["-s", "-o", "/dev/null", "-m", "10"];
        a.extend(extra);
        a.push(url);
        a
    };
    let plain = std::process::Command::new("/usr/bin/curl")
        .args(curl(&[]))
        .status()
        .unwrap();
    if support::unavailable("network to 1.1.1.1", plain.success()) {
        return;
    }
    let ws = support::tmpdir("sb-net");
    let closed = run(&seatbelt(false), "/usr/bin/curl", &curl(&[]), &ws).await;
    assert!(
        !closed.status.success(),
        "the network answered under the profile"
    );
    let open = run(&seatbelt(true), "/usr/bin/curl", &curl(&[]), &ws).await;
    assert!(
        open.status.success(),
        "with network = true the same call answers, so the refusal above is the network \
         rule: {}",
        String::from_utf8_lossy(&open.stderr)
    );
    std::fs::remove_dir_all(ws).ok();
}

/// A `[sandbox]` (shell) command is refused under Seatbelt, never run.
#[tokio::test]
async fn a_shell_command_is_refused_under_seatbelt() {
    let ws = support::tmpdir("sb-shell");
    let sb = seatbelt(false);
    assert!(sb.command("echo hi", &ws, &ws).is_err());
    assert!(sb.preflight(&ws).await.is_err());
    std::fs::remove_dir_all(ws).ok();
}

fn poppler() -> bool {
    mecha_core::sandbox::seatbelt::resolve_program("pdftoppm", std::env::var_os("PATH").as_deref())
        .is_ok()
}

/// A PDF whose objects are given in order; the xref is computed.
fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn text_pdf() -> Vec<u8> {
    let content = format!("BT /F1 24 Tf 60 700 Td ({LINE}) Tj ET");
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content.as_bytes());
    stream.extend_from_slice(b"\nendstream");
    pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        stream,
    ])
}

/// poppler under the profile: the renderer's own preflight passes, a page
/// renders to a PNG in the scratch, and the text layer comes back exact
/// through the `Extractor` a document takes — confined by Seatbelt, the
/// default here.
#[tokio::test]
async fn poppler_renders_and_reads_a_page_under_seatbelt() {
    if support::unavailable("poppler (`brew install poppler`)", poppler()) {
        return;
    }
    let cfg = DocumentsConfig {
        ocr: false,
        cache: false,
        layout: false,
        ..DocumentsConfig::default()
    };
    assert_eq!(cfg.confine, Backend::Seatbelt, "the default on a Mac");
    Renderer::new(&cfg)
        .preflight()
        .await
        .expect("pdfinfo runs under the profile");

    let ws = support::tmpdir("sb-pdf");
    std::fs::write(ws.join("in.pdf"), text_pdf()).unwrap();
    let out = run(
        &seatbelt(false),
        "pdftoppm",
        &["-r", "50", "-png", "in.pdf", "page"],
        &ws,
    )
    .await;
    assert!(
        out.status.success(),
        "pdftoppm under the profile: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pngs: Vec<_> = std::fs::read_dir(&ws)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".png"))
        .collect();
    assert_eq!(pngs.len(), 1, "one page, one PNG");

    let ex = Extractor::new(cfg, None).unwrap();
    let got = ex
        .extract(&text_pdf(), "all", Mode::Text, true, None)
        .await
        .unwrap();
    assert_eq!(got.confinement, "seatbelt");
    let text = got.pages[0].text.as_deref().unwrap_or_default();
    assert!(mecha_core::grounding::holds(text, LINE), "{text:?}");
    std::fs::remove_dir_all(ws).ok();
}
