//! Document extraction: a PDF's own text layer and a local OCR model's
//! transcript, per page, kept side by side.
//!
//! `docs/DOCUMENT-EXTRACTION-DESIGN.md` is the design; this is the one
//! implementation every door shares — the `document_read` tool
//! ([`crate::tool::document`]), `mecha document extract`, and whatever reads
//! documents next. Three rules carry it:
//!
//! **Two outputs, never merged.** The text layer is the file's own words,
//! exact, in reading order — what a citation check ([`crate::grounding::holds`])
//! can hold a quote against. The OCR transcript is a model's reading of the
//! rendered page: structure (LaTeX, a table) and the only text a scan has, but
//! a *reading*, never evidence of what the file says. Each page carries both
//! under its own label and its page number, and nothing here ever substitutes
//! one for the other silently.
//!
//! **The parser is confined, and never runs in this process.** A PDF is a
//! program for a page-description interpreter, and poppler has a long CVE
//! history. Every poppler call runs under [`crate::sandbox`] (bwrap by
//! default: no network, a private `/tmp`, the system read-only, one scratch
//! directory holding a copy of the file) with a memory and CPU rlimit and a
//! wall-clock timeout. What comes back is text and PNGs; the PNGs are decoded
//! and **re-encoded** here by the memory-safe `image` crate before any byte
//! reaches the OCR server, so a renderer compromised by the file cannot hand
//! the model server a crafted image. There is no unconfined fallback: a
//! configured confinement that cannot run fails the extraction, by name.
//!
//! **A configured OCR server that does not answer is an error, not an empty
//! page.** The server is on demand (`llama-ocr.socket`), so the first request
//! waits through a cold start for `/health` to say `ok` — a listening server
//! is not a ready one. Then each page's envelope is checked before its
//! content: HTTP status, an `error` body, `finish_reason`, and content that is
//! not empty (llama-server can answer HTTP 200 with nothing in it).
//!
//! **Extraction is paid once per file.** Results are cached by the sha256 of
//! the bytes that were actually rendered (the copy, not the path — a file that
//! changes under the read cannot poison the entry), under
//! `~/.mecha/documents/<sha256>/`, with the OCR transcripts keyed by model and
//! pipeline so a model change never serves a stale reading.

use crate::sandbox::{Backend, Sandbox, SandboxConfig};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Bumped whenever the whole-page OCR recipe changes (prompt, render size,
/// post-processing), so a cached transcript from an older recipe is not
/// served as this one's. Part of every OCR cache key.
pub const OCR_PIPELINE: &str = "wp1";

/// The prompt PaddleOCR-VL is trained on for plain recognition. The model
/// has six (`OCR:`, `Table Recognition:`, `Formula Recognition:`, `Chart
/// Recognition:`, `Seal Recognition:`, `Spotting:`) and answers nothing else
/// reliably; the full pipeline sends the element prompts on regions a layout
/// model cropped, which this build does not have (design §5).
pub const OCR_PROMPT: &str = "OCR:";

/// The projector's pixel budget (`clip.vision.image_max_pixels` in the
/// mmproj header, 1,003,520). A page is rendered to fit it, because anything
/// larger is resized down by the server anyway — rendering bigger only pays
/// for pixels that are thrown away.
pub const OCR_MAX_PIXELS: f64 = 1_003_520.0;

/// Fewer non-whitespace characters than this on a page, and the page is
/// treated as having no text layer — a scan, or a figure-only page — so
/// `auto` sends it to OCR.
pub const MIN_TEXT_LAYER_CHARS: usize = 30;

/// `[documents]`: where the OCR server is and the caps at the door.
///
/// Present means the `document_read` tool is registered. **Global file
/// only** (`Config::merge_file` strips it from a project layer): `ocr_url`
/// is where page images of the owner's documents go, and `confine` is the
/// confinement around the parser — a cloned repository choosing either would
/// choose where private pages are sent, or turn the confinement off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DocumentsConfig {
    /// Whether to OCR at all. Off, extraction is the text layer only and a
    /// call that needs OCR says so rather than returning an empty page.
    pub ocr: bool,
    /// The OCR llama-server's base URL. Must be loopback — see
    /// [`crate::imagegen::loopback_url`]'s argument, which applies here with
    /// page images in place of prompts.
    pub ocr_url: String,
    /// The model name sent in each request, and part of the cache key.
    pub ocr_model: String,
    /// How long the first request may wait for an idle-stopped server to
    /// load and answer `/health` (measured cold: ~3 s; a page cache miss
    /// and a busy GPU make it longer).
    pub ocr_ready_secs: u64,
    /// One page's OCR may take this long before it is abandoned. Also the
    /// wall clock for each confined poppler call.
    pub page_timeout_secs: u64,
    /// Output token cap per page. The densest of 48 measured pages wrote
    /// 1,678 (design §8); a page cut off at the cap is an error, not a page.
    pub ocr_max_tokens: u32,
    /// Files larger than this are refused before anything parses them.
    pub max_file_mb: u64,
    /// Documents with more pages than this are refused.
    pub max_pages: u32,
    /// At most this many pages are sent to OCR in one call; the answer names
    /// the pages it left for the next call.
    pub max_ocr_pages: u32,
    /// How poppler is confined: `bwrap` (default), `landlock`, or `none` —
    /// which must be written out, and is printed on every extraction.
    /// `docker` is refused: the sandbox image carries no poppler.
    pub confine: Backend,
    /// Address-space ceiling for each poppler process, in MB.
    pub memory_mb: u64,
    /// Cache extractions under `~/.mecha/documents/`.
    pub cache: bool,
    /// Cached extractions not read for this many days are removed whenever a
    /// new one is written. `0` keeps them until `mecha document prune`.
    pub cache_days: u32,
}

impl Default for DocumentsConfig {
    fn default() -> Self {
        DocumentsConfig {
            ocr: true,
            ocr_url: "http://127.0.0.1:8085".into(),
            ocr_model: "paddleocr-vl-1.6".into(),
            ocr_ready_secs: 120,
            page_timeout_secs: 180,
            ocr_max_tokens: 8192,
            max_file_mb: 100,
            max_pages: 2000,
            max_ocr_pages: 30,
            confine: Backend::Bwrap,
            memory_mb: 2048,
            cache: true,
            cache_days: 30,
        }
    }
}

impl DocumentsConfig {
    /// Refuse a configuration whose promises cannot be kept, at registration
    /// — never at the first page.
    pub fn validate(&self) -> Result<()> {
        if self.ocr {
            ocr_url(&self.ocr_url)?;
            if self.ocr_model.trim().is_empty() {
                bail!("[documents] ocr_model is empty");
            }
            if self.max_ocr_pages == 0 {
                bail!(
                    "[documents] max_ocr_pages is 0 with ocr = true — every page would be \
                     deferred; set it above zero, or ocr = false"
                );
            }
        }
        if self.confine == Backend::Docker {
            bail!(
                "[documents] confine = \"docker\" is not supported — the sandbox image has no \
                 poppler; use \"bwrap\" (default) or \"landlock\""
            );
        }
        if self.max_file_mb == 0 || self.max_pages == 0 || self.memory_mb == 0 {
            bail!("[documents] max_file_mb, max_pages and memory_mb must be greater than zero");
        }
        Ok(())
    }

    pub fn max_file_bytes(&self) -> u64 {
        self.max_file_mb.saturating_mul(1024 * 1024)
    }
}

/// The OCR server's URL, if it is on this machine: page images of private
/// documents go there, so a remote host would make the tool's no-egress
/// declaration a lie. The rule is [`crate::imagegen::is_loopback`]'s.
pub fn ocr_url(raw: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw).with_context(|| format!("[documents] ocr_url `{raw}`"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("[documents] ocr_url `{raw}` must be http or https");
    }
    if !crate::imagegen::is_loopback(&url) {
        bail!(
            "[documents] ocr_url `{raw}` is not on this machine — page images of your \
             documents are sent to it, so it must be loopback (127.0.0.1, ::1 or localhost)"
        );
    }
    Ok(url)
}

/// What a call asks for, per page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The text layer where a page has one; OCR where it does not.
    Auto,
    /// The text layer only — exact, fast, never a model.
    Text,
    /// The OCR transcript only.
    Ocr,
    /// Both, side by side.
    Both,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "auto" => Some(Mode::Auto),
            "text" => Some(Mode::Text),
            "ocr" => Some(Mode::Ocr),
            "both" => Some(Mode::Both),
            _ => None,
        }
    }
}

/// Which pages: `all`, `4`, `2-5`, `1,3,7-9`. One-based and inclusive, like
/// every page number a person reads. Returns sorted, deduplicated page
/// numbers, all within `1..=total`.
pub fn parse_pages(spec: &str, total: u32) -> Result<Vec<u32>> {
    let spec = spec.trim();
    if total == 0 {
        bail!("the document has no pages");
    }
    if spec.is_empty() || spec.eq_ignore_ascii_case("all") {
        return Ok((1..=total).collect());
    }
    let mut out = std::collections::BTreeSet::new();
    for part in spec.split(',') {
        let part = part.trim();
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim(), b.trim()),
            None => (part, part),
        };
        let a: u32 = a
            .parse()
            .map_err(|_| anyhow!("`{part}` is not a page or a range like 2-5"))?;
        let b: u32 = if b.is_empty() {
            total
        } else {
            b.parse()
                .map_err(|_| anyhow!("`{part}` is not a page or a range like 2-5"))?
        };
        if a == 0 || b < a {
            bail!("`{part}` is not a page range (pages are numbered from 1)");
        }
        if a > total {
            bail!("page {a} is past the end — the document has {total} page(s)");
        }
        out.extend(a..=b.min(total));
    }
    Ok(out.into_iter().collect())
}

/// Split `pdftotext` output into pages. poppler ends every page with a form
/// feed, including the last, so `n` pages are `n` form feeds; a page with no
/// text is an empty string, never a missing one — page numbers stay aligned.
pub fn split_pages(text: &str, expected: usize) -> Result<Vec<String>> {
    let mut pages: Vec<String> = text.split('\u{c}').map(str::to_string).collect();
    // The text after the last form feed is empty (or whitespace).
    if pages.last().is_some_and(|p| p.trim().is_empty()) {
        pages.pop();
    }
    // Trailing pages with no text at all leave no content between their form
    // feeds either, but still emit them — so a short count means the output
    // was cut, not that pages were blank.
    if pages.len() != expected {
        bail!(
            "the text layer split into {} page(s) where the document has {expected} — \
             refusing to guess which page is which",
            pages.len()
        );
    }
    Ok(pages)
}

/// A rectangle on a page, in PDF points from the top-left, with the text
/// poppler placed inside it: the unit a citation can later point at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    /// `[x_min, y_min, x_max, y_max]`.
    pub bbox: [f32; 4],
    pub text: String,
}

/// Parse `pdftotext -bbox-layout` into each page's blocks. The input is the
/// confined renderer's output and treated as hostile: a malformed document
/// yields fewer regions, never a panic, and nothing but `<page>`, `<block>`
/// and `<word>` is read.
pub fn parse_bbox_layout(xhtml: &str) -> Vec<(f32, f32, Vec<Region>)> {
    let mut pages = Vec::new();
    for page_chunk in xhtml.split("<page ").skip(1) {
        let head = page_chunk.split('>').next().unwrap_or("");
        let width = attr(head, "width").unwrap_or(0.0);
        let height = attr(head, "height").unwrap_or(0.0);
        let body = page_chunk.split("</page>").next().unwrap_or("");
        let mut regions = Vec::new();
        for block in body.split("<block ").skip(1) {
            let bhead = block.split('>').next().unwrap_or("");
            let bbox = [
                attr(bhead, "xMin").unwrap_or(0.0),
                attr(bhead, "yMin").unwrap_or(0.0),
                attr(bhead, "xMax").unwrap_or(0.0),
                attr(bhead, "yMax").unwrap_or(0.0),
            ];
            let body = block.split("</block>").next().unwrap_or("");
            let mut lines = Vec::new();
            for line in body.split("<line").skip(1) {
                let words: Vec<String> = line
                    .split("<word")
                    .skip(1)
                    .filter_map(|w| {
                        let inner = w.split_once('>')?.1;
                        Some(unescape(inner.split("</word>").next()?))
                    })
                    .collect();
                if !words.is_empty() {
                    lines.push(words.join(" "));
                }
            }
            if !lines.is_empty() {
                regions.push(Region {
                    bbox,
                    text: lines.join("\n"),
                });
            }
        }
        pages.push((width, height, regions));
    }
    pages
}

fn attr(head: &str, name: &str) -> Option<f32> {
    let key = format!("{name}=\"");
    let start = head.find(&key)? + key.len();
    let rest = &head[start..];
    rest[..rest.find('"')?].parse().ok()
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// PaddleOCR-VL writes tables in OTSL (`<fcel>` filled cell, `<ecel>` empty,
/// `<lcel>`/`<ucel>`/`<xcel>` merged left/up/both, `<nl>` row end). Turn every
/// OTSL run in `text` into a Markdown table; merged cells become empty cells,
/// which loses the span but never a value. Text without OTSL is returned as
/// it came.
pub fn otsl_to_markdown(text: &str) -> String {
    if !text.contains("<fcel>") && !text.contains("<ecel>") {
        return text.to_string();
    }
    let mut out = String::new();
    for line in text.split('\n') {
        if line.contains("<fcel>") || line.contains("<ecel>") {
            out.push_str(&otsl_table(line));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out.pop();
    out
}

fn otsl_table(s: &str) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for row in s.split("<nl>") {
        if row.trim().is_empty() {
            continue;
        }
        let mut cells = Vec::new();
        let mut rest = row;
        while let Some(i) = rest.find('<') {
            let Some(j) = rest[i..].find('>') else { break };
            let tag = &rest[i + 1..i + j];
            let after = &rest[i + j + 1..];
            let end = after.find('<').unwrap_or(after.len());
            let content = after[..end].trim();
            match tag {
                "fcel" => cells.push(content.replace('|', "\\|")),
                "ecel" | "lcel" | "ucel" | "xcel" => cells.push(String::new()),
                _ => {}
            }
            rest = &after[end..];
        }
        if !cells.is_empty() {
            rows.push(cells);
        }
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    if width == 0 {
        return s.to_string();
    }
    let mut md = String::new();
    for (i, row) in rows.iter().enumerate() {
        let mut cells = row.clone();
        cells.resize(width, String::new());
        md.push_str("| ");
        md.push_str(&cells.join(" | "));
        md.push_str(" |\n");
        if i == 0 {
            md.push('|');
            md.push_str(&" --- |".repeat(width));
            md.push('\n');
        }
    }
    md.trim_end().to_string()
}

/// Lowercase hex sha256 — the cache key, and the name a result reports so a
/// reader can tell two extractions of one file from two files.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Is this a PDF by its bytes? The header may sit anywhere in the first
/// kilobyte (the spec's allowance for junk before it); the extension is a
/// claim and is not read.
pub fn looks_like_pdf(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(1024)]
        .windows(5)
        .any(|w| w == b"%PDF-")
}

/// The render resolution that makes a `w × h` point page fit the OCR
/// projector's pixel budget, clamped to 50–300 dpi.
pub fn ocr_dpi(width_pt: f32, height_pt: f32) -> u32 {
    let area = (width_pt as f64) * (height_pt as f64);
    if !area.is_finite() || area <= 0.0 {
        return 100;
    }
    let dpi = 72.0 * (OCR_MAX_PIXELS / area).sqrt();
    dpi.clamp(50.0, 300.0) as u32
}

// ── The confined renderer ───────────────────────────────────────────────

/// A private scratch directory holding a copy of the document, deleted on
/// drop. The confined process sees this directory and nothing else writable.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(bytes: &[u8]) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("mecha-doc-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let scratch = Scratch { dir };
        std::fs::write(scratch.dir.join("in.pdf"), bytes)?;
        Ok(scratch)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// poppler, confined. One per extraction; every call is a fresh process.
pub struct Renderer {
    sandbox: Sandbox,
    timeout: Duration,
    memory_bytes: u64,
}

impl Renderer {
    pub fn new(cfg: &DocumentsConfig) -> Self {
        Renderer {
            sandbox: Sandbox::new(SandboxConfig {
                kind: cfg.confine,
                network: false,
                ..SandboxConfig::default()
            }),
            timeout: Duration::from_secs(cfg.page_timeout_secs.max(1)),
            memory_bytes: cfg.memory_mb.saturating_mul(1024 * 1024),
        }
    }

    /// Run one poppler program in the scratch directory and return its stdout.
    /// Confined, rlimited, time-bounded; a non-zero exit is an error carrying
    /// the tail of stderr.
    async fn run(&self, scratch: &Scratch, program: &str, args: &[String]) -> Result<Vec<u8>> {
        let mut cmd = self
            .sandbox
            .wrap_argv(program, args, &scratch.dir, &scratch.dir)
            .context("building the confined poppler command")?;
        if !self.sandbox.is_enabled() {
            // Unconfined by the operator's explicit choice: still no inherited
            // environment — a PDF renderer has no business holding API keys.
            cmd.env_clear();
            cmd.env("PATH", "/usr/local/bin:/usr/bin:/bin");
            cmd.env("HOME", &scratch.dir);
        }
        let memory = self.memory_bytes;
        let cpu = self.timeout.as_secs().saturating_add(5);
        #[cfg(unix)]
        unsafe {
            // Inherited through bwrap's exec by the poppler process. Raw
            // syscalls only: this runs between fork and exec.
            cmd.pre_exec(move || {
                let set = |res, v: u64| {
                    let lim = libc::rlimit {
                        rlim_cur: v as libc::rlim_t,
                        rlim_max: v as libc::rlim_t,
                    };
                    if libc::setrlimit(res, &lim) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                };
                set(libc::RLIMIT_AS, memory)?;
                set(libc::RLIMIT_CPU, cpu)?;
                // Bounds any one file the renderer writes (a page PNG).
                set(libc::RLIMIT_FSIZE, 256 * 1024 * 1024)?;
                Ok(())
            });
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let child = cmd.spawn().with_context(|| {
            format!(
                "cannot start `{program}` confined by {}",
                self.sandbox.backend().as_str()
            )
        })?;
        let out = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| anyhow!("`{program}` did not finish within {:?}", self.timeout))??;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let tail: String = stderr
                .lines()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join(" / ");
            bail!("`{program}` failed ({}): {}", out.status, tail.trim());
        }
        Ok(out.stdout)
    }

    /// Prove the confinement works before any document meets it — the
    /// sandbox's own preflight rule: a confinement that cannot run stops the
    /// extraction, it never degrades to an unconfined parse.
    pub async fn preflight(&self) -> Result<()> {
        let scratch = Scratch::new(b"")?;
        // `pdfinfo -v` exits 0 on the poppler shipped here (24.02); what it
        // proves is that the confinement starts *and* poppler is inside it.
        self.run(&scratch, "pdfinfo", &["-v".to_string()])
            .await
            .map(|_| ())
            .with_context(|| {
                format!(
                    "the PDF renderer cannot run under {} confinement — install poppler-utils, \
                     fix the confinement, or set [documents] confine explicitly; there is no \
                     silent fallback to an unconfined parse",
                    self.sandbox.backend().as_str()
                )
            })
    }

    /// Page count and each page's size in points.
    async fn info(&self, scratch: &Scratch, max_pages: u32) -> Result<Vec<(f32, f32)>> {
        let head = self.run(scratch, "pdfinfo", &["in.pdf".into()]).await?;
        let head = String::from_utf8_lossy(&head);
        let pages: u32 = head
            .lines()
            .find_map(|l| l.strip_prefix("Pages:").map(|v| v.trim().parse().ok()))
            .flatten()
            .ok_or_else(|| anyhow!("pdfinfo reported no page count — not a readable PDF"))?;
        if pages == 0 {
            bail!("the document has no pages");
        }
        if pages > max_pages {
            bail!("the document has {pages} pages; [documents] max_pages is {max_pages}");
        }
        let sizes = self
            .run(
                scratch,
                "pdfinfo",
                &[
                    "-f".into(),
                    "1".into(),
                    "-l".into(),
                    pages.to_string(),
                    "in.pdf".into(),
                ],
            )
            .await?;
        Ok(parse_page_sizes(&String::from_utf8_lossy(&sizes), pages))
    }

    async fn text_layer(&self, scratch: &Scratch, pages: usize) -> Result<Vec<String>> {
        // Reading order, not `-layout`: `-layout` sets a two-column page's
        // columns side by side, so a sentence broken across lines in one
        // column is interrupted by the other column's words — and a quote no
        // longer holds as one run (design §8: over 48 pages of ten papers,
        // `-layout` held a median of 75%, mean 62%, of the sentences reading
        // order has).
        let out = self
            .run(
                scratch,
                "pdftotext",
                &["-enc".into(), "UTF-8".into(), "in.pdf".into(), "-".into()],
            )
            .await?;
        split_pages(&String::from_utf8_lossy(&out), pages)
    }

    async fn regions(&self, scratch: &Scratch) -> Result<Vec<Vec<Region>>> {
        let out = self
            .run(
                scratch,
                "pdftotext",
                &[
                    "-bbox-layout".into(),
                    "-enc".into(),
                    "UTF-8".into(),
                    "in.pdf".into(),
                    "-".into(),
                ],
            )
            .await?;
        Ok(parse_bbox_layout(&String::from_utf8_lossy(&out))
            .into_iter()
            .map(|(_, _, r)| r)
            .collect())
    }

    /// One page as a PNG sized for the OCR projector, decoded and re-encoded
    /// here so nothing the renderer wrote reaches the model server verbatim.
    async fn page_png(&self, scratch: &Scratch, page: u32, size: (f32, f32)) -> Result<Vec<u8>> {
        let dpi = ocr_dpi(size.0, size.1);
        let name = format!("p{page}");
        self.run(
            scratch,
            "pdftoppm",
            &[
                "-r".into(),
                dpi.to_string(),
                "-f".into(),
                page.to_string(),
                "-l".into(),
                page.to_string(),
                "-singlefile".into(),
                "-png".into(),
                "in.pdf".into(),
                name.clone(),
            ],
        )
        .await?;
        let path = scratch.dir.join(format!("{name}.png"));
        let raw = tokio::fs::read(&path)
            .await
            .with_context(|| format!("the renderer wrote no image for page {page}"))?;
        let _ = tokio::fs::remove_file(&path).await;
        tokio::task::spawn_blocking(move || reencode_png(&raw)).await?
    }
}

/// Decode a PNG under explicit limits and write a fresh one: the page image
/// the OCR server sees is always this process's own encoding.
pub fn reencode_png(raw: &[u8]) -> Result<Vec<u8>> {
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(raw), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8000);
    limits.max_image_height = Some(8000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader
        .decode()
        .context("the rendered page is not a valid PNG")?;
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    rgb.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

/// `pdfinfo -f 1 -l N` prints `Page    4 size: 612 x 792 pts` per page.
/// A page it did not describe gets US Letter, which only sets a render size.
pub fn parse_page_sizes(text: &str, pages: u32) -> Vec<(f32, f32)> {
    let mut sizes = vec![(612.0, 792.0); pages as usize];
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("Page") else {
            continue;
        };
        let Some((num, dims)) = rest.split_once("size:") else {
            continue;
        };
        let Ok(n) = num.trim().parse::<usize>() else {
            continue;
        };
        let mut it = dims.split_whitespace();
        let (Some(w), Some("x"), Some(h)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        if let (Ok(w), Ok(h), true) = (w.parse(), h.parse(), (1..=pages as usize).contains(&n)) {
            sizes[n - 1] = (w, h);
        }
    }
    sizes
}

// ── The OCR server ──────────────────────────────────────────────────────

/// One page's transcript, and what it cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrPage {
    pub markdown: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub secs: f64,
}

pub struct OcrClient {
    base: reqwest::Url,
    model: String,
    http: reqwest::Client,
    ready: Duration,
    page: Duration,
    max_tokens: u32,
}

impl OcrClient {
    pub fn new(cfg: &DocumentsConfig) -> Result<Self> {
        Ok(OcrClient {
            base: ocr_url(&cfg.ocr_url)?,
            model: cfg.ocr_model.clone(),
            http: reqwest::Client::builder()
                // The vetted address is the only one this client may reach —
                // `ComfyUi::new`'s rule, with a page image of the owner's
                // document in place of a prompt: a 307/308 would re-send the
                // body elsewhere while the tool goes on declaring no egress,
                // and an inherited proxy setting would route the loopback
                // call through someone else (found on review).
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                // Short: pooled connections to the socket proxy count as
                // activity and would hold the model up past its idle stop.
                .pool_idle_timeout(Duration::from_secs(5))
                .build()?,
            ready: Duration::from_secs(cfg.ocr_ready_secs.max(1)),
            page: Duration::from_secs(cfg.page_timeout_secs.max(1)),
            max_tokens: cfg.ocr_max_tokens,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base.as_str().trim_end_matches('/'))
    }

    /// Wait for the server to be ready. On demand, the first connection is
    /// what starts it: systemd holds the socket and the request waits through
    /// the load. A refused connection means nothing is listening at all —
    /// the socket unit is not installed or not running — and is named so.
    pub async fn ready(&self) -> Result<()> {
        let url = self.url("/health");
        let resp = self
            .http
            .get(&url)
            .timeout(self.ready)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    anyhow!(
                        "the OCR server at {} is not reachable ({e}) — is its socket listening? \
                         `systemctl --user status llama-ocr.socket` (install: scripts/llama/install.sh)",
                        self.base
                    )
                } else if e.is_timeout() {
                    anyhow!(
                        "the OCR server at {} did not become ready within {:?} — \
                         `journalctl --user -u llama-ocr.service`",
                        self.base,
                        self.ready
                    )
                } else {
                    anyhow!("the OCR server at {} did not answer: {e}", self.base)
                }
            })?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        check_health(status.as_u16(), &body)
            .map_err(|e| anyhow!("OCR server at {}: {e}", self.base))
    }

    /// OCR one page image. Every way the answer can be empty or partial is an
    /// error here, so a caller never records a blank page as a transcript.
    pub async fn page(&self, png: &[u8]) -> Result<OcrPage> {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(png);
        let body = json!({
            "model": self.model,
            "temperature": 0,
            "max_tokens": self.max_tokens,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{data}")}},
                    {"type": "text", "text": OCR_PROMPT},
                ],
            }],
        });
        let started = Instant::now();
        let resp = self
            .http
            .post(self.url("/v1/chat/completions"))
            .timeout(self.page)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    anyhow!("OCR did not finish within {:?}", self.page)
                } else {
                    anyhow!("the OCR server at {} did not answer: {e}", self.base)
                }
            })?;
        let status = resp.status().as_u16();
        let text = resp.text().await.context("reading the OCR answer")?;
        let mut page = parse_completion(status, &text)?;
        page.secs = started.elapsed().as_secs_f64();
        if page.model.is_empty() {
            page.model = self.model.clone();
        }
        Ok(page)
    }
}

/// `/health` must be HTTP 200 **and** say `ok`: llama-server answers 503
/// while it loads, and a proxy or a stranger on the port could answer 200
/// with anything.
pub fn check_health(status: u16, body: &str) -> Result<()> {
    if status != 200 {
        bail!("/health answered HTTP {status}: {}", snippet(body));
    }
    let ok = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("status").and_then(Value::as_str).map(|s| s == "ok"))
        .unwrap_or(false);
    if !ok {
        bail!("/health answered 200 without status ok: {}", snippet(body));
    }
    Ok(())
}

/// Check the envelope before the content (`CLAUDE.md` §Recurring shapes):
/// status, an `error` object, `finish_reason`, then a non-empty content.
pub fn parse_completion(status: u16, body: &str) -> Result<OcrPage> {
    let v: Value = serde_json::from_str(body).map_err(|_| {
        anyhow!(
            "OCR answered HTTP {status} with a body that is not JSON: {}",
            snippet(body)
        )
    })?;
    if let Some(err) = v.get("error") {
        bail!(
            "OCR server refused the page (HTTP {status}): {}",
            snippet(&err.to_string())
        );
    }
    if status != 200 {
        bail!("OCR answered HTTP {status}: {}", snippet(body));
    }
    let choice = v
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| anyhow!("OCR answered HTTP 200 with no choices"))?;
    let finish = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .unwrap_or("");
    let content = choice
        .pointer("/message/content")
        .and_then(Value::as_str)
        .unwrap_or("");
    match finish {
        "stop" => {}
        "length" => bail!(
            "the OCR transcript was cut off at the token limit ([documents] ocr_max_tokens) — \
             the page is not transcribed whole"
        ),
        other => bail!("OCR finished with `{other}` rather than `stop`"),
    }
    if content.trim().is_empty() {
        bail!("OCR answered HTTP 200 with an empty transcript — refusing to record a blank page");
    }
    let usage = |k: &str| {
        v.pointer(&format!("/usage/{k}"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    Ok(OcrPage {
        markdown: otsl_to_markdown(content.trim()),
        model: v
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        prompt_tokens: usage("prompt_tokens"),
        completion_tokens: usage("completion_tokens"),
        secs: 0.0,
    })
}

fn snippet(s: &str) -> String {
    let s = s.trim();
    let cut: String = s.chars().take(300).collect();
    if cut.len() < s.len() {
        format!("{cut}…")
    } else {
        cut
    }
}

// ── The cache ───────────────────────────────────────────────────────────

/// What the text layer of one file is, cached whole — it is one confined
/// `pdftotext` for every page, so there is no per-page saving to be had.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub sha256: String,
    pub pages: u32,
    /// `(width, height)` in points, per page.
    pub sizes: Vec<(f32, f32)>,
    /// The text layer per page, in reading order.
    pub text: Vec<String>,
    /// poppler's blocks per page, with their boxes. Empty where the page has
    /// no text layer.
    pub regions: Vec<Vec<Region>>,
}

impl Layer {
    /// Whether a page has a text layer worth trusting over OCR.
    pub fn has_text(&self, page: u32) -> bool {
        self.text.get(page as usize - 1).is_some_and(|t| {
            t.chars().filter(|c| !c.is_whitespace()).count() >= MIN_TEXT_LAYER_CHARS
        })
    }
}

/// `~/.mecha/documents/<sha256>/` — `layer.json`, and
/// `ocr/<model>-<pipeline>/<page>.json`. Content-addressed, so it holds no
/// path, no session and no name: forgetting a session does not reach it
/// (design §4), and retention is by age.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Cache { root }
    }

    /// `$MECHA_DOCUMENTS_DIR`, or `~/.mecha/documents`.
    pub fn default_dir() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("MECHA_DOCUMENTS_DIR") {
            if !dir.is_empty() {
                return Ok(PathBuf::from(dir));
            }
        }
        Ok(crate::work::mecha_home()?.join("documents"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn entry(&self, sha: &str) -> PathBuf {
        self.root.join(sha)
    }

    /// The OCR sub-key: model and pipeline, reduced to a safe file name.
    pub fn ocr_key(model: &str) -> String {
        let clean: String = model
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{clean}-{OCR_PIPELINE}")
    }

    fn ocr_path(&self, sha: &str, model: &str, page: u32) -> PathBuf {
        self.entry(sha)
            .join("ocr")
            .join(Self::ocr_key(model))
            .join(format!("{page:05}.json"))
    }

    pub fn load_layer(&self, sha: &str) -> Option<Layer> {
        let path = self.entry(sha).join("layer.json");
        let layer: Layer = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
        // A cached layer for another file under this name is corruption.
        (layer.sha256 == sha && layer.text.len() == layer.pages as usize).then(|| {
            touch(&path);
            layer
        })
    }

    pub fn store_layer(&self, layer: &Layer) -> Result<()> {
        write_atomic(
            &self.entry(&layer.sha256).join("layer.json"),
            &serde_json::to_vec(layer)?,
        )
    }

    pub fn load_ocr(&self, sha: &str, model: &str, page: u32) -> Option<OcrPage> {
        let p = self.ocr_path(sha, model, page);
        let page: OcrPage = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
        (!page.markdown.trim().is_empty()).then_some(page)
    }

    pub fn store_ocr(&self, sha: &str, model: &str, page: u32, ocr: &OcrPage) -> Result<()> {
        write_atomic(&self.ocr_path(sha, model, page), &serde_json::to_vec(ocr)?)
    }

    /// Remove entries whose `layer.json` was last read or written more than
    /// `max_age` ago. Returns the sha256s removed.
    pub fn prune(&self, max_age: Duration) -> Result<Vec<String>> {
        let mut removed = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Ok(removed);
        };
        let now = std::time::SystemTime::now();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.len() != 64 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let stamp = std::fs::metadata(entry.path().join("layer.json"))
                .and_then(|m| m.modified())
                .or_else(|_| entry.metadata().and_then(|m| m.modified()));
            let old = stamp
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age > max_age);
            if old {
                std::fs::remove_dir_all(entry.path())
                    .with_context(|| format!("removing {}", entry.path().display()))?;
                removed.push(name);
            }
        }
        Ok(removed)
    }

    /// Remove one file's entry. `true` when there was one.
    pub fn forget(&self, sha: &str) -> Result<bool> {
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("`{sha}` is not a sha256");
        }
        let dir = self.entry(sha);
        if !dir.exists() {
            return Ok(false);
        }
        std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        Ok(true)
    }
}

fn touch(path: &Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("a cache path has a parent")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The owner's documents: nobody else on the machine reads the cache.
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let tmp = dir.join(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

// ── Extraction ──────────────────────────────────────────────────────────

/// One page of an extraction. `text` and `ocr` are separate on purpose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    /// One-based.
    pub number: u32,
    /// The file's own words for this page, when asked for.
    pub text: Option<String>,
    /// Whether the page has a text layer at all.
    pub has_text_layer: bool,
    pub ocr: Option<OcrPage>,
    /// Why this page has no transcript although one was wanted — said, never
    /// left as a blank.
    pub ocr_error: Option<String>,
    /// Whether the transcript came from the cache.
    pub ocr_cached: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub regions: Vec<Region>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extraction {
    pub sha256: String,
    pub pages_total: u32,
    pub mode: Mode,
    pub pages: Vec<Page>,
    /// Pages this call needed OCR for and did not reach (`max_ocr_pages`,
    /// or a cancelled run).
    pub ocr_deferred: Vec<u32>,
    /// The run was cancelled mid-extraction; `pages` holds what was done.
    #[serde(default)]
    pub cancelled: bool,
    /// How poppler was confined, for the record.
    pub confinement: String,
    pub layer_cached: bool,
}

impl Extraction {
    /// The text a model or a terminal reads. Page by page; each part labelled
    /// with where it came from.
    pub fn render(&self, name: &str) -> String {
        let mut out = format!(
            "document: {name} · {} page(s) · sha256 {}\n",
            self.pages_total,
            &self.sha256[..12]
        );
        for p in &self.pages {
            if let Some(text) = &p.text {
                out.push_str(&format!(
                    "\n=== page {} of {} · text layer (the file's own words) ===\n",
                    p.number, self.pages_total
                ));
                if text.trim().is_empty() {
                    out.push_str("(no text layer on this page — a scan or a figure)\n");
                } else {
                    out.push_str(text.trim_end());
                    out.push('\n');
                }
            }
            if let Some(ocr) = &p.ocr {
                out.push_str(&format!(
                    "\n=== page {} of {} · OCR transcript ({}; a model's reading, not the file's text) ===\n",
                    p.number, self.pages_total, ocr.model
                ));
                out.push_str(ocr.markdown.trim_end());
                out.push('\n');
            }
            if let Some(err) = &p.ocr_error {
                out.push_str(&format!(
                    "\n=== page {} of {} · OCR failed ===\n{err}\n",
                    p.number, self.pages_total
                ));
            }
        }
        if !self.ocr_deferred.is_empty() {
            let why = if self.cancelled {
                "the run was cancelled"
            } else {
                "OCR stopped at the per-call limit"
            };
            out.push_str(&format!(
                "\n({why}; pages not yet transcribed: {} — ask for them next)\n",
                compact_ranges(&self.ocr_deferred)
            ));
        }
        out
    }
}

/// `[1,2,3,5,7,8]` → `1-3,5,7-8`.
pub fn compact_ranges(pages: &[u32]) -> String {
    let mut out = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let start = pages[i];
        let mut end = start;
        while i + 1 < pages.len() && pages[i + 1] == end + 1 {
            i += 1;
            end = pages[i];
        }
        out.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
        i += 1;
    }
    out.join(",")
}

/// The extractor: config, the confined renderer, the OCR client if any, the
/// cache if any. Cheap to build; holds no connection until used.
pub struct Extractor {
    cfg: DocumentsConfig,
    renderer: Renderer,
    ocr: Option<OcrClient>,
    cache: Option<Cache>,
}

impl Extractor {
    /// `cache` is `None` to write nothing to disk (a surface that promises no
    /// trace, or a test).
    pub fn new(cfg: DocumentsConfig, cache: Option<Cache>) -> Result<Self> {
        cfg.validate()?;
        let ocr = if cfg.ocr {
            Some(OcrClient::new(&cfg)?)
        } else {
            None
        };
        Ok(Extractor {
            renderer: Renderer::new(&cfg),
            ocr,
            cache,
            cfg,
        })
    }

    pub fn config(&self) -> &DocumentsConfig {
        &self.cfg
    }

    /// Extract `pages` (a [`parse_pages`] spec) of the PDF in `bytes`.
    /// `with_regions` keeps poppler's blocks on each page (the CLI's `--json`).
    ///
    /// `cancel` is the run's token: `run_tools` awaits a tool to completion,
    /// so a tool that waits on a model must watch it itself, as
    /// `image_generate` does (found on review). A cancelled extraction
    /// returns the pages already done, the rest listed as not transcribed.
    pub async fn extract(
        &self,
        bytes: &[u8],
        pages: &str,
        mode: Mode,
        with_regions: bool,
        cancel: Option<&CancellationToken>,
    ) -> Result<Extraction> {
        if bytes.len() as u64 > self.cfg.max_file_bytes() {
            bail!(
                "the file is {} MB; [documents] max_file_mb is {}",
                bytes.len() / (1024 * 1024),
                self.cfg.max_file_mb
            );
        }
        if !looks_like_pdf(bytes) {
            bail!("not a PDF (no %PDF- header in the first kilobyte)");
        }
        if matches!(mode, Mode::Ocr | Mode::Both) && self.ocr.is_none() {
            bail!("OCR is off ([documents] ocr = false) — ask for mode `text`");
        }
        let sha = sha256_hex(bytes);
        let mut scratch: Option<Scratch> = None;
        let confinement = self.renderer.sandbox.backend().as_str().to_string();

        let (layer, layer_cached) = match self.cache.as_ref().and_then(|c| c.load_layer(&sha)) {
            Some(layer) => (layer, true),
            None => {
                self.renderer.preflight().await?;
                let s = scratch.insert(Scratch::new(bytes)?);
                let sizes = self.renderer.info(s, self.cfg.max_pages).await?;
                let n = sizes.len();
                let text = self.renderer.text_layer(s, n).await?;
                // Regions are a convenience (a citation's box), not the
                // text: a document poppler cannot box still extracts.
                let mut regions = self.renderer.regions(s).await.unwrap_or_default();
                regions.resize(n, Vec::new());
                let layer = Layer {
                    sha256: sha.clone(),
                    pages: n as u32,
                    sizes,
                    text,
                    regions,
                };
                if let Some(cache) = &self.cache {
                    // A cache is a saving, not a precondition: a full disk
                    // must not turn a good extraction into an error.
                    if let Err(e) = cache.store_layer(&layer) {
                        tracing::warn!("document cache: text layer not stored: {e:#}");
                    }
                    if self.cfg.cache_days > 0 {
                        let _ = cache
                            .prune(Duration::from_secs(u64::from(self.cfg.cache_days) * 86_400));
                    }
                }
                (layer, false)
            }
        };

        let wanted = parse_pages(pages, layer.pages)?;
        let mut out = Vec::with_capacity(wanted.len());
        let mut deferred = Vec::new();
        let mut ocr_budget = self.cfg.max_ocr_pages;
        let mut ready = false;
        let mut cancelled = false;
        for n in wanted {
            if cancelled || cancel.is_some_and(|c| c.is_cancelled()) {
                cancelled = true;
                deferred.push(n);
                continue;
            }
            let has_text = layer.has_text(n);
            let want_text =
                matches!(mode, Mode::Text | Mode::Both) || (mode == Mode::Auto && has_text);
            let want_ocr =
                matches!(mode, Mode::Ocr | Mode::Both) || (mode == Mode::Auto && !has_text);
            let mut page = Page {
                number: n,
                text: want_text.then(|| layer.text[n as usize - 1].clone()),
                has_text_layer: has_text,
                ocr: None,
                ocr_error: None,
                ocr_cached: false,
                regions: if with_regions {
                    layer
                        .regions
                        .get(n as usize - 1)
                        .cloned()
                        .unwrap_or_default()
                } else {
                    Vec::new()
                },
            };
            if want_ocr {
                match &self.ocr {
                    None => {
                        page.ocr_error = Some(
                            "this page has no text layer and OCR is off ([documents] ocr = false)"
                                .into(),
                        );
                        // Show the (empty) text layer so the page is not silent.
                        page.text.get_or_insert_with(String::new);
                    }
                    Some(client) => {
                        if let Some(hit) = self
                            .cache
                            .as_ref()
                            .and_then(|c| c.load_ocr(&sha, client.model(), n))
                        {
                            page.ocr = Some(hit);
                            page.ocr_cached = true;
                        } else if ocr_budget == 0 {
                            deferred.push(n);
                        } else {
                            ocr_budget -= 1;
                            if !ready {
                                // Unreachable is the whole call's error, not a
                                // page's: nothing after it would succeed.
                                match until_cancelled(cancel, client.ready()).await {
                                    Some(r) => r?,
                                    None => {
                                        cancelled = true;
                                        deferred.push(n);
                                        out.push(page);
                                        continue;
                                    }
                                }
                                ready = true;
                            }
                            let s = match scratch.as_mut() {
                                Some(s) => s,
                                None => {
                                    self.renderer.preflight().await?;
                                    scratch.insert(Scratch::new(bytes)?)
                                }
                            };
                            let size = layer
                                .sizes
                                .get(n as usize - 1)
                                .copied()
                                .unwrap_or((612.0, 792.0));
                            let work = async {
                                match self.renderer.page_png(s, n, size).await {
                                    Ok(png) => client.page(&png).await,
                                    Err(e) => Err(e),
                                }
                            };
                            let Some(result) = until_cancelled(cancel, work).await else {
                                cancelled = true;
                                deferred.push(n);
                                out.push(page);
                                continue;
                            };
                            match result {
                                Ok(ocr) => {
                                    if let Some(cache) = &self.cache {
                                        if let Err(e) =
                                            cache.store_ocr(&sha, client.model(), n, &ocr)
                                        {
                                            tracing::warn!(
                                                "document cache: page {n} not stored: {e:#}"
                                            );
                                        }
                                    }
                                    page.ocr = Some(ocr);
                                }
                                Err(e) => page.ocr_error = Some(format!("{e:#}")),
                            }
                        }
                    }
                }
            }
            out.push(page);
        }
        Ok(Extraction {
            sha256: sha,
            pages_total: layer.pages,
            mode,
            pages: out,
            ocr_deferred: deferred,
            cancelled,
            confinement,
            layer_cached,
        })
    }
}

/// `fut`, or `None` if `cancel` fires first. With no token it is just `fut`.
async fn until_cancelled<F: std::future::Future>(
    cancel: Option<&CancellationToken>,
    fut: F,
) -> Option<F::Output> {
    match cancel {
        Some(token) => tokio::select! {
            out = fut => Some(out),
            _ = token.cancelled() => None,
        },
        None => Some(fut.await),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny HTTP server answering each connection with the next canned
    /// response, for the OCR client's transport tests.
    fn canned_server(responses: Vec<String>) -> u16 {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for r in responses {
                let Ok((mut conn, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 2048];
                let _ = conn.read(&mut buf);
                let _ = conn.write_all(r.as_bytes());
            }
        });
        port
    }

    fn http(status: &str, extra: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// The vetted loopback address is the only one the OCR client may reach:
    /// a redirect is an answer, not a hop, or a 307 would re-send a page
    /// image elsewhere while the tool declares no egress.
    #[test]
    fn the_ocr_client_does_not_follow_a_redirect() {
        let healthy = canned_server(vec![http("200 OK", "", r#"{"status":"ok"}"#)]);
        let redirector = canned_server(vec![http(
            "307 Temporary Redirect",
            &format!("Location: http://127.0.0.1:{healthy}/health\r\n"),
            "",
        )]);
        let cfg = DocumentsConfig {
            ocr_url: format!("http://127.0.0.1:{redirector}"),
            ..DocumentsConfig::default()
        };
        let client = OcrClient::new(&cfg).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt.block_on(client.ready()).unwrap_err();
        assert!(err.to_string().contains("307"), "{err:#}");
    }

    /// A cancelled run stops at the next page and says so, rather than
    /// holding the run until every page is transcribed.
    #[test]
    fn until_cancelled_yields_to_the_token() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let token = CancellationToken::new();
        token.cancel();
        let never = std::future::pending::<()>();
        assert!(rt.block_on(until_cancelled(Some(&token), never)).is_none());
        assert_eq!(rt.block_on(until_cancelled(None, async { 7 })), Some(7));
    }

    #[test]
    fn page_specs_are_one_based_inclusive_and_bounded() {
        assert_eq!(parse_pages("all", 3).unwrap(), vec![1, 2, 3]);
        assert_eq!(parse_pages("", 2).unwrap(), vec![1, 2]);
        assert_eq!(parse_pages("2", 5).unwrap(), vec![2]);
        assert_eq!(parse_pages("4-", 5).unwrap(), vec![4, 5]);
        assert_eq!(parse_pages("5,1-2,2", 9).unwrap(), vec![1, 2, 5]);
        // A range running past the end is clipped, a start past it refused.
        assert_eq!(parse_pages("3-99", 4).unwrap(), vec![3, 4]);
        for bad in ["0", "6", "3-2", "x", "1-y", "-3"] {
            assert!(parse_pages(bad, 5).is_err(), "{bad}");
        }
        assert!(parse_pages("1", 0).is_err());
    }

    #[test]
    fn pages_split_on_form_feeds_and_blank_pages_keep_their_place() {
        let text = "one\n\u{c}\u{c}three\n\u{c}";
        let pages = split_pages(text, 3).unwrap();
        assert_eq!(pages, vec!["one\n", "", "three\n"]);
        // A count that does not match is refused rather than guessed.
        assert!(split_pages(text, 4).is_err());
        assert!(split_pages("a\u{c}", 2).is_err());
    }

    #[test]
    fn bbox_layout_yields_blocks_with_boxes_and_unescaped_words() {
        let xhtml = r#"<html><body><doc>
<page width="612.000000" height="792.000000">
<flow><block xMin="72.0" yMin="90.5" xMax="300.0" yMax="110.0">
<line xMin="72" yMin="90" xMax="300" yMax="100"><word xMin="72" yMin="90" xMax="90" yMax="100">Priya</word><word xMin="92" yMin="90" xMax="120" yMax="100">&amp;</word><word xMin="1" yMin="1" xMax="2" yMax="2">Ada</word></line>
<line xMin="72" yMin="100" xMax="300" yMax="110"><word xMin="72" yMin="100" xMax="90" yMax="110">&lt;x&gt;</word></line>
</block></flow>
</page>
<page width="100" height="200"></page>
</doc></body></html>"#;
        let pages = parse_bbox_layout(xhtml);
        assert_eq!(pages.len(), 2);
        assert_eq!((pages[0].0, pages[0].1), (612.0, 792.0));
        assert_eq!(pages[0].2.len(), 1);
        assert_eq!(pages[0].2[0].bbox, [72.0, 90.5, 300.0, 110.0]);
        assert_eq!(pages[0].2[0].text, "Priya & Ada\n<x>");
        assert!(pages[1].2.is_empty());
        // Hostile input degrades, never panics.
        for junk in [
            "<page ",
            "<page width=\"x\"><block <line<word>",
            "<block xMin=\"",
        ] {
            let _ = parse_bbox_layout(junk);
        }
    }

    #[test]
    fn otsl_becomes_a_markdown_table_and_merged_cells_keep_their_column() {
        let s = "Table 1: Results\n<fcel>Model<fcel>BLEU<lcel><nl><ucel><fcel>EN-DE<fcel>EN-FR<nl><fcel>Base | small<fcel>27.3<ecel><nl>";
        let md = otsl_to_markdown(s);
        assert_eq!(
            md,
            "Table 1: Results\n| Model | BLEU |  |\n| --- | --- | --- |\n|  | EN-DE | EN-FR |\n| Base \\| small | 27.3 |  |"
        );
        assert_eq!(otsl_to_markdown("no table here"), "no table here");
    }

    #[test]
    fn health_needs_status_200_and_ok_in_the_body() {
        assert!(check_health(200, r#"{"status":"ok"}"#).is_ok());
        assert!(check_health(503, r#"{"error":{"code":503,"message":"Loading model"}}"#).is_err());
        assert!(check_health(200, r#"{"status":"loading"}"#).is_err());
        assert!(check_health(200, "hello").is_err());
    }

    #[test]
    fn a_completion_is_checked_envelope_first() {
        let ok = r#"{"model":"paddleocr-vl-1.6","choices":[{"finish_reason":"stop","message":{"content":"Hello Mara"}}],"usage":{"prompt_tokens":1253,"completion_tokens":4}}"#;
        let page = parse_completion(200, ok).unwrap();
        assert_eq!(page.markdown, "Hello Mara");
        assert_eq!((page.prompt_tokens, page.completion_tokens), (1253, 4));

        // HTTP 200 with an empty content — the llama-server trap.
        let empty = r#"{"choices":[{"finish_reason":"stop","message":{"content":"  "}}]}"#;
        assert!(parse_completion(200, empty)
            .unwrap_err()
            .to_string()
            .contains("empty"));
        // Cut off at the token limit: a partial page is not a page.
        let cut = r#"{"choices":[{"finish_reason":"length","message":{"content":"half"}}]}"#;
        assert!(parse_completion(200, cut)
            .unwrap_err()
            .to_string()
            .contains("cut off"));
        // An error body, whatever the status says.
        let err = r#"{"error":{"code":400,"message":"image too large"}}"#;
        assert!(parse_completion(400, err).is_err());
        assert!(parse_completion(200, err).is_err());
        assert!(parse_completion(502, "<html>bad gateway</html>").is_err());
        assert!(parse_completion(200, r#"{"choices":[]}"#).is_err());
    }

    #[test]
    fn a_remote_ocr_server_is_refused() {
        assert!(ocr_url("http://127.0.0.1:8085").is_ok());
        assert!(ocr_url("http://localhost:8085").is_ok());
        assert!(ocr_url("http://[::1]:8085").is_ok());
        assert!(ocr_url("http://10.0.0.5:8085").is_err());
        assert!(ocr_url("http://ocr.example.com").is_err());
        assert!(ocr_url("file:///tmp/x").is_err());
        let mut cfg = DocumentsConfig {
            ocr_url: "http://192.168.1.2:8085".into(),
            ..DocumentsConfig::default()
        };
        assert!(cfg.validate().is_err());
        // With OCR off, the URL is never used and not judged.
        cfg.ocr = false;
        assert!(cfg.validate().is_ok());
        cfg.confine = Backend::Docker;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn caps_and_type_are_checked_before_anything_parses() {
        let cfg = DocumentsConfig {
            max_file_mb: 1,
            ocr: false,
            cache: false,
            ..DocumentsConfig::default()
        };
        let ex = Extractor::new(cfg, None).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let big = vec![b' '; 2 * 1024 * 1024];
        let e = rt
            .block_on(ex.extract(&big, "all", Mode::Text, false, None))
            .unwrap_err();
        assert!(e.to_string().contains("max_file_mb"), "{e}");
        let e = rt
            .block_on(ex.extract(b"GIF89a not a pdf", "all", Mode::Text, false, None))
            .unwrap_err();
        assert!(e.to_string().contains("not a PDF"), "{e}");
        let e = rt
            .block_on(ex.extract(b"%PDF-1.4", "all", Mode::Ocr, false, None))
            .unwrap_err();
        assert!(e.to_string().contains("OCR is off"), "{e}");
    }

    #[test]
    fn pdf_is_known_by_its_header_not_its_name() {
        assert!(looks_like_pdf(b"%PDF-1.7\n..."));
        assert!(looks_like_pdf(b"\xef\xbb\xbfjunk%PDF-1.4"));
        assert!(!looks_like_pdf(b"\x89PNG\r\n"));
        let mut late = vec![b' '; 2000];
        late.extend_from_slice(b"%PDF-1.4");
        assert!(!looks_like_pdf(&late));
    }

    #[test]
    fn a_page_renders_to_the_projectors_budget() {
        // US Letter: 72·sqrt(1,003,520 / (612·792)) = 103.
        assert_eq!(ocr_dpi(612.0, 792.0), 103);
        assert_eq!(ocr_dpi(10.0, 10.0), 300);
        assert_eq!(ocr_dpi(10000.0, 10000.0), 50);
        assert_eq!(ocr_dpi(0.0, 792.0), 100);
    }

    #[test]
    fn page_sizes_come_from_pdfinfo_with_letter_for_the_undescribed() {
        let text = "Page    1 size: 612 x 792 pts (letter)\nPage    2 size: 595.276 x 841.89 pts (A4)\nPage    9 size: 1 x 1 pts\n";
        let sizes = parse_page_sizes(text, 3);
        assert_eq!(
            sizes,
            vec![(612.0, 792.0), (595.276, 841.89), (612.0, 792.0)]
        );
    }

    #[test]
    fn cache_keys_are_content_hashes_and_ocr_keys_carry_model_and_pipeline() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            Cache::ocr_key("paddleocr-vl-1.6"),
            format!("paddleocr-vl-1.6-{OCR_PIPELINE}")
        );
        assert_eq!(Cache::ocr_key("../x y"), format!(".._x_y-{OCR_PIPELINE}"));
    }

    #[test]
    fn cache_round_trips_refuses_mismatches_and_prunes_by_age() {
        let dir = std::env::temp_dir().join(format!("mecha-doc-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Cache::new(dir.clone());
        let sha = sha256_hex(b"a document");
        let layer = Layer {
            sha256: sha.clone(),
            pages: 1,
            sizes: vec![(612.0, 792.0)],
            text: vec!["Priya wrote this.".into()],
            regions: vec![Vec::new()],
        };
        cache.store_layer(&layer).unwrap();
        assert_eq!(cache.load_layer(&sha).unwrap().text, layer.text);
        // A layer filed under the wrong hash is not served.
        let other = sha256_hex(b"another");
        std::fs::create_dir_all(dir.join(&other)).unwrap();
        std::fs::copy(
            dir.join(&sha).join("layer.json"),
            dir.join(&other).join("layer.json"),
        )
        .unwrap();
        assert!(cache.load_layer(&other).is_none());

        let ocr = OcrPage {
            markdown: "# Title".into(),
            model: "paddleocr-vl-1.6".into(),
            prompt_tokens: 1,
            completion_tokens: 2,
            secs: 0.5,
        };
        cache.store_ocr(&sha, "paddleocr-vl-1.6", 1, &ocr).unwrap();
        assert_eq!(cache.load_ocr(&sha, "paddleocr-vl-1.6", 1), Some(ocr));
        // Another model's transcript is another key.
        assert!(cache.load_ocr(&sha, "other-model", 1).is_none());

        assert!(cache.prune(Duration::from_secs(3600)).unwrap().is_empty());
        let removed = cache.prune(Duration::ZERO).unwrap();
        assert_eq!(removed.len(), 2, "{removed:?}");
        assert!(cache.load_layer(&sha).is_none());
        assert!(cache.forget("not-a-hash").is_err());
        assert!(!cache.forget(&sha).unwrap());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn ranges_compact_for_the_deferred_note() {
        assert_eq!(compact_ranges(&[1, 2, 3, 5, 7, 8]), "1-3,5,7-8");
        assert_eq!(compact_ranges(&[4]), "4");
        assert_eq!(compact_ranges(&[]), "");
    }

    #[test]
    fn a_rendered_page_is_re_encoded_and_garbage_is_refused() {
        let mut png = Vec::new();
        image::RgbImage::from_pixel(20, 10, image::Rgb([1, 2, 3]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let out = reencode_png(&png).unwrap();
        let back = image::load_from_memory(&out).unwrap();
        assert_eq!((back.width(), back.height()), (20, 10));
        assert!(reencode_png(b"\x89PNG\r\n\x1a\nnot really").is_err());
    }

    #[test]
    fn the_render_labels_each_source_and_never_shows_a_blank_as_a_transcript() {
        let ex = Extraction {
            sha256: sha256_hex(b"x"),
            pages_total: 3,
            mode: Mode::Auto,
            pages: vec![
                Page {
                    number: 1,
                    text: Some("Ada's abstract.".into()),
                    has_text_layer: true,
                    ocr: None,
                    ocr_error: None,
                    ocr_cached: false,
                    regions: Vec::new(),
                },
                Page {
                    number: 2,
                    text: None,
                    has_text_layer: false,
                    ocr: None,
                    ocr_error: Some("OCR did not finish within 180s".into()),
                    ocr_cached: false,
                    regions: Vec::new(),
                },
            ],
            ocr_deferred: vec![3],
            cancelled: false,
            confinement: "bwrap".into(),
            layer_cached: false,
        };
        let s = ex.render("papers/a.pdf");
        assert!(s.contains("page 1 of 3 · text layer"));
        assert!(s.contains("page 2 of 3 · OCR failed"));
        assert!(s.contains("pages not yet transcribed: 3"));
    }
}
