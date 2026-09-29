//! Document extraction against the real backends: poppler under bwrap, and
//! the on-demand OCR server.
//!
//! The PDFs are built here, byte by byte — no document is committed, and
//! the only names in them are the fictional cast's. One has a text layer;
//! the other is that page rendered to pixels and wrapped as an image-only
//! PDF, which is what a scan is to a parser.
//!
//! Skips when poppler or bwrap is missing, when nothing answers at the
//! OCR URL (`MECHA_TEST_OCR_URL`, default `http://127.0.0.1:8085`), or when
//! the layout stage is not installed (`MECHA_TEST_LAYOUT_DIR`, default
//! `~/.mecha/layout`, holding `venv/bin/python` and `PP-DocLayoutV3.onnx` —
//! `scripts/layout/install.sh`) — and `MECHA_TEST_REQUIRE_BACKENDS=1` turns
//! each skip into a failure.

mod support;

use mecha_core::document::{Cache, DocumentsConfig, Extractor, Mode, Pipeline};
use mecha_core::layout::LayoutModel;
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

/// A page with a title, a paragraph, a numbered heading and a ruled table
/// of the fictional cast's ledger — the shape whole-page OCR got wrong.
fn table_pdf() -> Vec<u8> {
    let mut c = String::new();
    let mut text = |x: f32, y: f32, font: &str, size: f32, s: &str| {
        c.push_str(&format!("BT /{font} {size} Tf {x} {y} Td ({s}) Tj ET\n"));
    };
    text(72.0, 720.0, "F2", 22.0, "Guild Ledger Review");
    let para = [
        "Priya and Ada reconciled the guild ledger for the year. Each clerk kept",
        "a separate book, and the totals below are the sums of their entries,",
        "checked twice against the receipts held by Mara at the counting house.",
    ];
    for (i, line) in para.iter().enumerate() {
        text(72.0, 680.0 - 14.0 * i as f32, "F1", 11.0, line);
    }
    text(72.0, 620.0, "F2", 14.0, "2 Results");
    text(72.0, 590.0, "F1", 10.0, "Table 1: Ledger totals by clerk.");
    let cols = [110.0, 220.0, 320.0, 420.0];
    let rows: [[&str; 4]; 4] = [
        ["Clerk", "Quarter", "Entries", "Total"],
        ["Priya", "Q1", "42", "318.50"],
        ["Ada", "Q2", "37", "290.25"],
        ["Mara", "Q3", "51", "402.75"],
    ];
    for (r, row) in rows.iter().enumerate() {
        let y = 560.0 - 20.0 * r as f32;
        for (x, cell) in cols.iter().zip(row) {
            text(*x, y, if r == 0 { "F2" } else { "F1" }, 11.0, cell);
        }
    }
    // Booktabs rules: above the header, below it, below the last row.
    for y in [575.0, 553.0, 494.0] {
        c.push_str(&format!("0.8 w 100 {y} m 480 {y} l S\n"));
    }
    pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_vec(),
        stream("", c.as_bytes()),
    ])
}

/// Where the layout stage is installed, if it is.
fn layout_dir() -> std::path::PathBuf {
    match std::env::var("MECHA_TEST_LAYOUT_DIR") {
        Ok(d) if !d.is_empty() => d.into(),
        _ => mecha_core::work::mecha_home().unwrap().join("layout"),
    }
}

fn layout_installed() -> bool {
    let d = layout_dir();
    d.join("venv/bin/python").exists() && d.join("PP-DocLayoutV3.onnx").exists()
}

fn layout_config(ocr_url: String) -> DocumentsConfig {
    let d = layout_dir();
    DocumentsConfig {
        ocr_url,
        confine: Backend::Bwrap,
        layout_python: Some(d.join("venv/bin/python")),
        layout_model: Some(d.join("PP-DocLayoutV3.onnx")),
        ..DocumentsConfig::default()
    }
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
        // The whole-page recipe; the layout stage has its own tests below.
        layout: false,
        ..DocumentsConfig::default()
    };
    let cache = Cache::new(dir.join("cache"));
    let ex = Extractor::new(cfg, Some(cache)).unwrap();
    let out = ex
        .extract(&scan, "all", Mode::Auto, false, None)
        .await
        .unwrap();
    assert_eq!(out.layout_unavailable, None);
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

/// The layout model runs under bwrap and finds the page's table and title —
/// no OCR server needed for this part.
#[tokio::test]
async fn the_layout_model_runs_confined_and_finds_the_table() {
    if support::unavailable("poppler + bwrap", poppler_and_bwrap()) {
        return;
    }
    if support::unavailable(
        "the layout stage (scripts/layout/install.sh)",
        layout_installed(),
    ) {
        return;
    }
    let dir = scratch("layout");
    std::fs::write(dir.join("t.pdf"), table_pdf()).unwrap();
    let ok = Command::new("pdftoppm")
        .args(["-r", "144", "-singlefile", "-png"])
        .arg(dir.join("t.pdf"))
        .arg(dir.join("t"))
        .status()
        .unwrap();
    assert!(ok.success());
    let img = image::open(dir.join("t.png")).unwrap().to_rgb8();
    let d = layout_dir();
    let model = LayoutModel::new(
        d.join("venv/bin/python"),
        d.join("PP-DocLayoutV3.onnx"),
        Backend::Bwrap,
        4096,
        4,
        std::time::Duration::from_secs(120),
    );
    let mut worker = model.start().await.unwrap();
    assert_eq!(worker.backend(), "bwrap");
    let found = worker.detect(&img).await.unwrap();
    let labels: Vec<&str> = found.iter().map(|d| d.label()).collect();
    assert!(labels.contains(&"table"), "{labels:?}");
    assert!(
        labels.contains(&"doc_title") || labels.contains(&"paragraph_title"),
        "{labels:?}"
    );
    // The same worker takes a second page: one process per extraction.
    assert_eq!(worker.detect(&img).await.unwrap().len(), found.len());
    std::fs::remove_dir_all(dir).ok();
}

/// The confinement is real for the layout worker: an "interpreter" that
/// reads a file outside the paths it was given reports ready only when it
/// can — which it can unconfined (so the check is not vacuous) and cannot
/// under bwrap, where the stage then fails closed rather than running.
#[cfg(unix)]
#[tokio::test]
async fn the_layout_worker_cannot_reach_outside_its_confinement() {
    if support::unavailable("bwrap", have("bwrap", "--version")) {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("layout-escape");
    let secret = dir.join("secret.txt");
    std::fs::write(&secret, "Ada's private note").unwrap();
    let bin = dir.join("env/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let python = bin.join("python");
    std::fs::write(
        &python,
        format!(
            "#!/bin/sh\nif cat '{}' >/dev/null 2>&1; then printf MLY1; sleep 5; else echo 'no secret here' >&2; exit 3; fi\n",
            secret.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
    let model = dir.join("env/model.onnx");
    std::fs::write(&model, b"not a model").unwrap();
    let start = |confine| {
        LayoutModel::new(
            python.clone(),
            model.clone(),
            confine,
            512,
            1,
            std::time::Duration::from_secs(10),
        )
    };
    assert!(
        start(Backend::None).start().await.is_ok(),
        "unconfined, the fake worker reads the file — the negative below means something"
    );
    let err = match start(Backend::Bwrap).start().await {
        Ok(_) => panic!("under bwrap the worker read a file outside its confinement"),
        Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("bwrap"), "{err}");
    assert!(err.contains("no secret here"), "{err}");
    std::fs::remove_dir_all(dir).ok();
}

/// End to end: the table comes back whole, as a Markdown table, rows intact;
/// the title is marked; the regions are stored with boxes in points; and
/// the second read is the cache's, under the layout key.
#[tokio::test]
async fn a_table_comes_back_whole_through_the_layout_stage() {
    if support::unavailable("poppler + bwrap", poppler_and_bwrap()) {
        return;
    }
    if support::unavailable(
        "the layout stage (scripts/layout/install.sh)",
        layout_installed(),
    ) {
        return;
    }
    if support::unavailable("the OCR server", ocr_up().await) {
        return;
    }
    let dir = scratch("layout-e2e");
    let ex = Extractor::new(
        layout_config(ocr_url()),
        Some(Cache::new(dir.join("cache"))),
    )
    .unwrap();
    let out = ex
        .extract(&table_pdf(), "1", Mode::Ocr, false, None)
        .await
        .unwrap();
    assert_eq!(out.layout_unavailable, None);
    let page = &out.pages[0];
    let ocr = page
        .ocr
        .as_ref()
        .unwrap_or_else(|| panic!("{:?}", page.ocr_error));
    assert_eq!(ocr.pipeline, Pipeline::Layout, "{}", ocr.markdown);
    assert_eq!(ocr.failed_regions(), 0, "{:?}", ocr.regions);
    let table = ocr
        .regions
        .iter()
        .find(|r| r.label == "table")
        .unwrap_or_else(|| panic!("no table region: {:?}", ocr.regions));
    let md = table.markdown.as_deref().unwrap();
    // Every row keeps its own cells, in one Markdown table.
    for (who, total) in [("Priya", "318.50"), ("Ada", "290.25"), ("Mara", "402.75")] {
        assert!(
            md.lines()
                .any(|l| l.starts_with('|') && l.contains(who) && l.contains(total)),
            "{who}'s row lost {total}:\n{md}"
        );
    }
    // In PDF points, on the page: the table sits in the upper half.
    let [x0, y0, x1, y1] = table.bbox;
    assert!(0.0 <= x0 && x0 < x1 && x1 <= 612.0 && 0.0 <= y0 && y0 < y1 && y1 <= 792.0);
    assert!(y1 < 396.0, "{:?}", table.bbox);
    assert!(
        ocr.markdown
            .lines()
            .any(|l| l.starts_with('#') && l.contains("Guild Ledger")),
        "{}",
        ocr.markdown
    );
    let rendered = out.render("ledger.pdf");
    assert!(rendered.contains("read by region"), "{rendered}");

    let again = ex
        .extract(&table_pdf(), "1", Mode::Ocr, false, None)
        .await
        .unwrap();
    assert!(again.pages[0].ocr_cached);
    assert_eq!(
        again.pages[0].ocr.as_ref().unwrap().pipeline,
        Pipeline::Layout
    );
    std::fs::remove_dir_all(dir).ok();
}

/// Configured and missing: every OCR page is read whole, and the result says
/// so — at the top and on the page — rather than passing for a layout read.
#[tokio::test]
async fn a_missing_layout_model_is_named_and_the_page_is_read_whole() {
    if support::unavailable("poppler + bwrap", poppler_and_bwrap()) {
        return;
    }
    if support::unavailable("the OCR server", ocr_up().await) {
        return;
    }
    let dir = scratch("layout-missing");
    let cfg = DocumentsConfig {
        layout_model: Some(dir.join("nowhere/PP-DocLayoutV3.onnx")),
        ..layout_config(ocr_url())
    };
    let ex = Extractor::new(cfg, None).unwrap();
    let out = ex
        .extract(&table_pdf(), "1", Mode::Ocr, false, None)
        .await
        .unwrap();
    let why = out
        .layout_unavailable
        .as_deref()
        .expect("the stage is named unavailable");
    assert!(why.contains("nowhere/PP-DocLayoutV3.onnx"), "{why}");
    let ocr = out.pages[0].ocr.as_ref().unwrap();
    assert_eq!(ocr.pipeline, Pipeline::WholePage);
    assert!(ocr.fallback.is_some());
    assert!(ocr.regions.is_empty());
    let rendered = out.render("ledger.pdf");
    assert!(
        rendered.contains("layout stage is unavailable"),
        "{rendered}"
    );
    assert!(rendered.contains("read whole — "), "{rendered}");
    std::fs::remove_dir_all(dir).ok();
}

/// A cancelled run stops a layout read mid-page — the worker exchange and
/// the region requests in flight are dropped — and says which page it left.
#[tokio::test]
async fn a_cancelled_run_stops_a_layout_read_mid_page() {
    if support::unavailable("poppler + bwrap", poppler_and_bwrap()) {
        return;
    }
    if support::unavailable(
        "the layout stage (scripts/layout/install.sh)",
        layout_installed(),
    ) {
        return;
    }
    if support::unavailable("the OCR server", ocr_up().await) {
        return;
    }
    let ex = Extractor::new(layout_config(ocr_url()), None).unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let later = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        later.cancel();
    });
    let started = std::time::Instant::now();
    let out = ex
        .extract(&table_pdf(), "1", Mode::Ocr, false, Some(&token))
        .await
        .unwrap();
    assert!(out.cancelled);
    assert_eq!(out.ocr_deferred, vec![1]);
    assert!(out.pages[0].ocr.is_none());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

/// `page_timeout_secs` bounds a page end to end. A server that answers
/// `/health` and then never answers a completion must cost the page its one
/// budget — not a budget per region, which is what a page of many regions
/// read two at a time used to pay (found on review of #406).
#[tokio::test]
async fn a_wedged_server_costs_a_page_one_timeout_not_one_per_region() {
    if support::unavailable("poppler + bwrap", poppler_and_bwrap()) {
        return;
    }
    if support::unavailable(
        "the layout stage (scripts/layout/install.sh)",
        layout_installed(),
    ) {
        return;
    }
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { return };
            let mut buf = [0u8; 4096];
            let n = conn.read(&mut buf).unwrap_or(0);
            if String::from_utf8_lossy(&buf[..n]).starts_with("GET /health") {
                let body = r#"{"status":"ok"}"#;
                let _ = write!(
                    conn,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            } else {
                // A completion: accepted, never answered.
                held.push(conn);
            }
        }
    });
    let ex = Extractor::new(
        DocumentsConfig {
            page_timeout_secs: 3,
            ..layout_config(format!("http://127.0.0.1:{port}"))
        },
        None,
    )
    .unwrap();
    let started = std::time::Instant::now();
    let out = ex
        .extract(&table_pdf(), "1", Mode::Ocr, false, None)
        .await
        .unwrap();
    let err = out.pages[0].ocr_error.clone().unwrap_or_default();
    assert!(err.contains("the page did not finish within"), "{err}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}
