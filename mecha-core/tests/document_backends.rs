//! Document extraction against the real backends: poppler under bwrap, and
//! the on-demand OCR server.
//!
//! The PDFs are built here, byte by byte — no document is committed, and
//! the only names in them are the fictional cast's. One has a text layer;
//! the other is that page rendered to pixels and wrapped as an image-only
//! PDF, which is what a scan is to a parser.
//!
//! Skips when poppler or bwrap is missing, or when nothing answers at the
//! OCR URL (`MECHA_TEST_OCR_URL`, default `http://127.0.0.1:8085`) — and
//! `MECHA_TEST_REQUIRE_BACKENDS=1` turns each skip into a failure.

mod support;

use mecha_core::document::{Cache, DocumentsConfig, Extractor, Mode};
use mecha_core::sandbox::Backend;
use mecha_core::tool::document::DocumentRead;
use mecha_core::tool::{Tool, ToolCtx};
use serde_json::json;
use std::process::Command;

const LINE: &str = "Priya reviewed the ledger with Ada on Tuesday.";

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

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    v.extend_from_slice(data);
    v.extend_from_slice(b"\nendstream");
    v
}

fn text_pdf() -> Vec<u8> {
    let content = format!("BT /F1 24 Tf 60 700 Td ({LINE}) Tj ET");
    pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        stream("", content.as_bytes()),
    ])
}

/// The text PDF, rendered by poppler and wrapped as raw RGB: a scan.
fn scanned_pdf(dir: &std::path::Path) -> Vec<u8> {
    std::fs::write(dir.join("t.pdf"), text_pdf()).unwrap();
    let ok = Command::new("pdftoppm")
        .args(["-r", "100", "-singlefile", "-png"])
        .arg(dir.join("t.pdf"))
        .arg(dir.join("t"))
        .status()
        .unwrap();
    assert!(ok.success());
    let img = image::open(dir.join("t.png")).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        stream(
            &format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8"),
            img.as_raw(),
        ),
        stream("", b"q 612 0 0 792 0 0 cm /Im1 Do Q"),
    ])
}

fn have(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mecha-docint-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ocr_url() -> String {
    std::env::var("MECHA_TEST_OCR_URL").unwrap_or_else(|_| "http://127.0.0.1:8085".into())
}

async fn ocr_up() -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
    else {
        return false;
    };
    client
        .get(format!("{}/health", ocr_url()))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

fn poppler_and_bwrap() -> bool {
    have("pdfinfo", "-v") && have("bwrap", "--version")
}

/// The text layer, through bwrap, exactly — and the tool marks it as the
/// document's words (external) while its own refusals are not.
#[tokio::test]
async fn the_text_layer_comes_back_exact_from_a_confined_parser() {
    if support::unavailable("poppler + bwrap", poppler_and_bwrap()) {
        return;
    }
    let dir = scratch("text");
    std::fs::write(dir.join("ledger.pdf"), text_pdf()).unwrap();
    let cfg = DocumentsConfig {
        ocr: false,
        confine: Backend::Bwrap,
        ..DocumentsConfig::default()
    };
    let cache = Cache::new(dir.join("cache"));
    let ex = Extractor::new(cfg.clone(), Some(cache.clone())).unwrap();
    let out = ex
        .extract(&text_pdf(), "all", Mode::Text, true, None)
        .await
        .unwrap();
    assert_eq!(out.pages_total, 1);
    assert_eq!(out.confinement, "bwrap");
    let text = out.pages[0].text.as_deref().unwrap();
    assert!(mecha_core::grounding::holds(text, LINE), "{text:?}");
    assert!(
        out.pages[0]
            .regions
            .iter()
            .any(|r| r.text.contains("Priya")),
        "{:?}",
        out.pages[0].regions
    );
    // Second read is the cache's, and says so.
    let again = ex
        .extract(&text_pdf(), "1", Mode::Text, false, None)
        .await
        .unwrap();
    assert!(again.layer_cached);

    let tool = DocumentRead::new(Extractor::new(cfg, None).unwrap());
    let ctx = ToolCtx {
        workspace: dir.clone(),
        ..Default::default()
    };
    let ok = tool
        .call(json!({"path": "ledger.pdf", "mode": "text"}), &ctx)
        .await
        .unwrap();
    assert!(!ok.is_error && ok.external, "{}", ok.content);
    assert!(
        ok.content.contains("page 1 of 1 · text layer"),
        "{}",
        ok.content
    );
    std::fs::remove_dir_all(dir).ok();
}

/// A garbage file that claims to be a PDF fails inside the confinement, as
/// an error — never a panic, never an empty success.
#[tokio::test]
async fn a_broken_pdf_is_an_error_from_the_confined_parser() {
    if support::unavailable("poppler + bwrap", poppler_and_bwrap()) {
        return;
    }
    let ex = Extractor::new(
        DocumentsConfig {
            ocr: false,
            ..DocumentsConfig::default()
        },
        None,
    )
    .unwrap();
    let err = ex
        .extract(
            b"%PDF-1.4\nthis is not a document\n",
            "all",
            Mode::Auto,
            false,
            None,
        )
        .await
        .unwrap_err();
    assert!(!err.to_string().is_empty());
    // Real poppler, real failure: its stderr is the document's words, and
    // the error says so, so `document_read` marks it external.
    assert!(mecha_core::document::carries_document_text(&err), "{err:#}");
}

/// A scan has no text layer, so `auto` sends it to the model — and the
/// model reads the line back. Also: the OCR is cached by content hash.
#[tokio::test]
async fn a_scan_is_read_by_the_ocr_model() {
    if support::unavailable("poppler + bwrap", poppler_and_bwrap()) {
        return;
    }
    if support::unavailable("the OCR server", ocr_up().await) {
        return;
    }
    let dir = scratch("scan");
    let scan = scanned_pdf(&dir);
    let cfg = DocumentsConfig {
        ocr_url: ocr_url(),
        ..DocumentsConfig::default()
    };
    let cache = Cache::new(dir.join("cache"));
    let ex = Extractor::new(cfg, Some(cache)).unwrap();
    let out = ex
        .extract(&scan, "all", Mode::Auto, false, None)
        .await
        .unwrap();
    let page = &out.pages[0];
    assert!(!page.has_text_layer, "the scan should have no text layer");
    let ocr = page
        .ocr
        .as_ref()
        .unwrap_or_else(|| panic!("{:?}", page.ocr_error));
    assert!(
        mecha_core::grounding::holds(&ocr.markdown, "Priya reviewed the ledger"),
        "{:?}",
        ocr.markdown
    );
    let again = ex
        .extract(&scan, "all", Mode::Auto, false, None)
        .await
        .unwrap();
    assert!(again.pages[0].ocr_cached);
    std::fs::remove_dir_all(dir).ok();
}

/// Configured but not listening is a named error, never an empty page.
#[tokio::test]
async fn an_unreachable_ocr_server_is_named() {
    if support::unavailable("poppler + bwrap", poppler_and_bwrap()) {
        return;
    }
    let dir = scratch("down");
    // A port nothing listens on: bind one, learn it, drop it.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let ex = Extractor::new(
        DocumentsConfig {
            ocr_url: format!("http://127.0.0.1:{port}"),
            ocr_ready_secs: 5,
            ..DocumentsConfig::default()
        },
        None,
    )
    .unwrap();
    let err = ex
        .extract(&scanned_pdf(&dir), "all", Mode::Auto, false, None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not reachable"), "{err:#}");
    std::fs::remove_dir_all(dir).ok();
}
