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
//! **Tables go through a layout stage.** A transcript is read region by
//! region: [`crate::layout`] finds the page's tables, formulas, headings and
//! paragraphs (PP-DocLayoutV3, confined like the parser), each region is
//! cropped from this process's own decode of the page and sent with the
//! prompt the OCR model was trained for, and the page is assembled in the
//! layout model's reading order. A layout stage that is configured but
//! cannot run is named on every page it affects, and those pages are read
//! whole — labelled so, never passed off as the layout reading.
//!
//! **Extraction is paid once per file.** Results are cached by the sha256 of
//! the bytes that were actually rendered (the copy, not the path — a file that
//! changes under the read cannot poison the entry), under
//! `~/.mecha/documents/<sha256>/`, with the OCR transcripts keyed by model and
//! pipeline so a model change never serves a stale reading.

use crate::layout::{LayoutChild, LayoutModel, LayoutRegion, Task};
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
/// served as this one's. Part of every whole-page OCR cache key.
pub const OCR_PIPELINE: &str = "wp1";

/// The layout recipe's version (render size, post-processing, prompts,
/// assembly) — part of every layout-read cache key, beside the layout
/// model's own hash, so a whole-page reading is never served as a layout
/// one and a changed layout model never serves the old one's regions.
pub const LAYOUT_PIPELINE: &str = "ly1";

/// Region reads in flight at once: the OCR server's slot count (`-np 2`,
/// `scripts/llama/mecha-ocr-server`). More would only queue there.
pub const REGION_CONCURRENCY: usize = 2;

/// The prompt PaddleOCR-VL is trained on for plain recognition. The model
/// has six (`OCR:`, `Table Recognition:`, `Formula Recognition:`, `Chart
/// Recognition:`, `Seal Recognition:`, `Spotting:`) and answers nothing else
/// reliably. With the layout stage (`layout.rs`) each cropped region gets
/// its element's prompt; this one reads a page whole — the fallback when the
/// stage is off, unavailable, or finds no regions (design §5).
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
    /// One page's OCR may take this long before it is abandoned — end to
    /// end: its layout pass and every region it is read in, not each region
    /// on its own. Also the wall clock for each confined poppler call.
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
    /// Read OCR pages region by region through the layout model (design
    /// §5). Off, every page is read whole — labelled so.
    pub layout: bool,
    /// The Python interpreter whose environment has `onnxruntime` and
    /// `numpy`. Default `~/.mecha/layout/venv/bin/python`, which
    /// `scripts/layout/install.sh` creates.
    pub layout_python: Option<PathBuf>,
    /// PP-DocLayoutV3's ONNX export. Default
    /// `~/.mecha/layout/PP-DocLayoutV3.onnx`.
    pub layout_model: Option<PathBuf>,
    /// CPU threads for the layout model (it never touches the GPU).
    pub layout_threads: u32,
    /// Address-space ceiling for the layout worker, in MB.
    pub layout_memory_mb: u64,
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
            layout: true,
            layout_python: None,
            layout_model: None,
            layout_threads: 4,
            layout_memory_mb: 4096,
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
        if self.layout && !(1..=64).contains(&self.layout_threads) {
            bail!("[documents] layout_threads must be between 1 and 64");
        }
        if self.layout && self.layout_memory_mb == 0 {
            bail!("[documents] layout_memory_mb must be greater than zero");
        }
        Ok(())
    }

    pub fn max_file_bytes(&self) -> u64 {
        self.max_file_mb.saturating_mul(1024 * 1024)
    }

    /// The layout worker's interpreter, configured or defaulted.
    pub fn layout_python_path(&self) -> Result<PathBuf> {
        match &self.layout_python {
            Some(p) => Ok(p.clone()),
            None => Ok(crate::work::mecha_home()?.join("layout/venv/bin/python")),
        }
    }

    /// The layout model file, configured or defaulted.
    pub fn layout_model_path(&self) -> Result<PathBuf> {
        match &self.layout_model {
            Some(p) => Ok(p.clone()),
            None => Ok(crate::work::mecha_home()?.join("layout/PP-DocLayoutV3.onnx")),
        }
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

/// Address space, CPU seconds and the largest file a confined child may
/// write, set between fork and exec and inherited through bwrap's exec by
/// the program it runs — poppler, or the layout worker.
pub(crate) fn limit(cmd: &mut tokio::process::Command, memory: u64, cpu: u64, fsize: u64) {
    #[cfg(unix)]
    unsafe {
        // Raw syscalls only: this runs between fork and exec.
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
            set(libc::RLIMIT_FSIZE, fsize)?;
            Ok(())
        });
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
        let cpu = self.timeout.as_secs().saturating_add(5);
        // FSIZE bounds any one file the renderer writes (a page PNG).
        limit(&mut cmd, self.memory_bytes, cpu, 256 * 1024 * 1024);
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
            let msg = format!("`{program}` failed ({}): {}", out.status, tail.trim());
            // Poppler's words about *this document* — it interpolates
            // content-stream strings into its diagnostics — travel as
            // third-party text (found on review of #404). The sandbox's own
            // refusal (bwrap's "bwrap: …", before poppler ever ran) is ours,
            // and marking it would arm untrusted taint over nothing a
            // document said.
            return Err(if stderr_is_the_sandboxs(&stderr) {
                anyhow!(msg)
            } else {
                anyhow::Error::new(ParserSaid(msg))
            });
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
        // No document here — an empty scratch — so whatever failed, it is not
        // a document's words: the chain is flattened to plain text, dropping
        // any `ParserSaid`, before it can be marked external (found on
        // review).
        self.run(&scratch, "pdfinfo", &["-v".to_string()])
            .await
            .map(|_| ())
            .map_err(|e| anyhow!("{e:#}"))
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
        let img = self
            .page_image(scratch, page, ocr_dpi(size.0, size.1))
            .await?;
        tokio::task::spawn_blocking(move || encode_png(&img)).await?
    }

    /// One page rendered at `dpi` and decoded here, under limits: the image
    /// every later step — the layout model's tensor, each region's crop —
    /// is made from, so none of them is the renderer's own bytes.
    async fn page_image(&self, scratch: &Scratch, page: u32, dpi: u32) -> Result<image::RgbImage> {
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
        tokio::task::spawn_blocking(move || decode_png(&raw)).await?
    }
}

/// Decode a PNG under explicit limits (8000 px a side, 512 MB) with the
/// memory-safe `image` crate.
pub fn decode_png(raw: &[u8]) -> Result<image::RgbImage> {
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
    Ok(img.to_rgb8())
}

/// A fresh PNG of an image this process holds.
pub fn encode_png(img: &image::RgbImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

/// Decode a PNG under explicit limits and write a fresh one: the page image
/// the OCR server sees is always this process's own encoding.
pub fn reencode_png(raw: &[u8]) -> Result<Vec<u8>> {
    encode_png(&decode_png(raw)?)
}

/// One region of a decoded page, as a fresh PNG no larger than the OCR
/// projector's budget. `bbox` is in the image's pixels.
pub fn crop_png(img: &image::RgbImage, bbox: [f32; 4]) -> Result<Vec<u8>> {
    let (w, h) = (img.width(), img.height());
    let x0 = (bbox[0].max(0.0).floor() as u32).min(w.saturating_sub(1));
    let y0 = (bbox[1].max(0.0).floor() as u32).min(h.saturating_sub(1));
    let x1 = (bbox[2].ceil().max(0.0) as u32).clamp(x0 + 1, w);
    let y1 = (bbox[3].ceil().max(0.0) as u32).clamp(y0 + 1, h);
    let mut crop = image::imageops::crop_imm(img, x0, y0, x1 - x0, y1 - y0).to_image();
    let px = f64::from(crop.width()) * f64::from(crop.height());
    if px > OCR_MAX_PIXELS {
        let k = (OCR_MAX_PIXELS / px).sqrt();
        let nw = ((f64::from(crop.width()) * k) as u32).max(1);
        let nh = ((f64::from(crop.height()) * k) as u32).max(1);
        crop = image::imageops::resize(&crop, nw, nh, image::imageops::FilterType::CatmullRom);
    }
    encode_png(&crop)
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

/// How a page's transcript was read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pipeline {
    /// The whole page, one `OCR:` prompt — the only recipe before the
    /// layout stage, so an entry without the field is this one.
    #[default]
    WholePage,
    /// Region by region, through the layout model.
    Layout,
}

/// One page's transcript, and what it cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrPage {
    pub markdown: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Wall clock for the page: rendering is not counted, the layout model
    /// and every region read are.
    pub secs: f64,
    #[serde(default)]
    pub pipeline: Pipeline,
    /// The layout model's regions, in reading order, each with what was
    /// read there — boxes in PDF points, for a citation to open.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<LayoutRegion>,
    /// The layout model's share of `secs`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub layout_secs: f64,
    /// Why this page was read whole although the layout stage is
    /// configured. Set when served, never cached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

impl OcrPage {
    /// Regions that were meant to be read and were not.
    pub fn failed_regions(&self) -> usize {
        self.regions.iter().filter(|r| r.error.is_some()).count()
    }
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
        self.recognize(png, OCR_PROMPT).await
    }

    /// Read one image — a page or a region of one — with one of the model's
    /// task prompts. The same envelope checks as [`OcrClient::page`].
    pub async fn recognize(&self, png: &[u8], prompt: &str) -> Result<OcrPage> {
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
                    {"type": "text", "text": prompt},
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
        pipeline: Pipeline::WholePage,
        regions: Vec::new(),
        layout_secs: 0.0,
        fallback: None,
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

    /// The OCR sub-key: model and pipeline ([`OCR_PIPELINE`], or
    /// [`LAYOUT_PIPELINE`] with the layout model's hash), reduced to a safe
    /// file name.
    pub fn ocr_key(model: &str, pipeline: &str) -> String {
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
        let pipeline: String = pipeline
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{clean}-{pipeline}")
    }

    fn ocr_path(&self, sha: &str, model: &str, pipeline: &str, page: u32) -> PathBuf {
        self.entry(sha)
            .join("ocr")
            .join(Self::ocr_key(model, pipeline))
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
        self.write_private(
            &self.entry(&layer.sha256).join("layer.json"),
            &serde_json::to_vec(layer)?,
        )
    }

    pub fn load_ocr(&self, sha: &str, model: &str, pipeline: &str, page: u32) -> Option<OcrPage> {
        let p = self.ocr_path(sha, model, pipeline, page);
        let page: OcrPage = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
        (!page.markdown.trim().is_empty() && page.failed_regions() == 0).then_some(page)
    }

    pub fn store_ocr(
        &self,
        sha: &str,
        model: &str,
        pipeline: &str,
        page: u32,
        ocr: &OcrPage,
    ) -> Result<()> {
        self.write_private(
            &self.ocr_path(sha, model, pipeline, page),
            &serde_json::to_vec(ocr)?,
        )
    }

    /// Remove entries whose `layer.json` was last read or written more than
    /// `max_age` ago. Returns the sha256s removed.
    pub fn prune(&self, max_age: Duration) -> Result<Vec<String>> {
        let mut removed = Vec::new();
        // A missing cache is an empty one; an unreadable one is not — `mecha
        // document prune` is where `sessions forget`'s residue sends the
        // owner, and "removed 0" over a store it could not read would be a
        // clean bill it had not earned (#410).
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(removed),
            Err(e) => return Err(e).with_context(|| format!("reading {}", self.root.display())),
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

    /// Write `bytes` to `path` atomically, with every directory from the
    /// cache root down to the file owner-only (0700).
    ///
    /// Not just the file's own directory: the root lists one entry per PDF,
    /// named by the file's hash, so a root left at the umask default (0755,
    /// or 0775 under a group umask) lets any local user confirm which
    /// documents the owner read (#410). A permission that cannot be set is an
    /// error, not a warning — the callers already treat a failed store as a
    /// skipped cache write, which is the right outcome for a cache that
    /// cannot be kept private.
    fn write_private(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let dir = path.parent().context("a cache path has a parent")?;
        let below = dir
            .strip_prefix(&self.root)
            .context("a cache path is under the cache root")?;
        let mut at = self.root.clone();
        crate::create_private_dir(&at).with_context(|| format!("securing {}", at.display()))?;
        for part in below.components() {
            at.push(part);
            crate::create_private_dir(&at).with_context(|| format!("securing {}", at.display()))?;
        }
        write_atomic(path, bytes)
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

/// Write `bytes` beside `path` and rename into place. The directories are
/// the caller's to create — [`Cache::write_private`] makes them owner-only.
///
/// The file is created 0600, never the umask default: it holds a document's
/// text or its transcript, which is more private than the hash-named listing
/// #410 was about, and a private store's files are owner-only as well as its
/// directories (found on review). `create_new` refuses to follow or reuse a
/// file already at the temporary name.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let dir = path.parent().context("a cache path has a parent")?;
    let tmp = dir.join(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .with_context(|| format!("writing {}", tmp.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("writing {}", tmp.display()))?;
    drop(file);
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

/// An error carrying what a parser printed about the document. Poppler
/// interpolates strings from a PDF's content streams into its diagnostics, so
/// this text is the document author's as much as the page text is: a caller
/// returning it to a model marks it external, exactly as it marks a page
/// ([`carries_document_text`]). Our own refusals ("not a PDF", a size cap)
/// are not this type, so they are not mislabelled third-party content.
#[derive(Debug)]
pub struct ParserSaid(pub String);

impl std::fmt::Display for ParserSaid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParserSaid {}

/// Whether a failed confined command's stderr is only the sandbox's own —
/// bwrap refusing to start (no user namespaces, a missing path) — so poppler
/// never ran and nothing in it came from a document.
fn stderr_is_the_sandboxs(stderr: &str) -> bool {
    let mut lines = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .peekable();
    lines.peek().is_some() && lines.all(|l| l.starts_with("bwrap:"))
}

/// Whether `e`, anywhere in its chain, carries a parser's words about the
/// document ([`ParserSaid`]).
pub fn carries_document_text(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.is::<ParserSaid>())
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
    /// Why the layout stage could not be used in this call although it is
    /// configured — the pages it affected say so too, and were read whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_unavailable: Option<String>,
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
        if let Some(why) = &self.layout_unavailable {
            out.push_str(&format!(
                "(the layout stage is unavailable, so OCR pages below were read whole — tables and \
                 headings in them are not reliable: {why})\n"
            ));
        }
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
                let how = match (ocr.pipeline, &ocr.fallback) {
                    (Pipeline::Layout, _) => format!("read by region: {}", ocr.regions.len()),
                    (Pipeline::WholePage, None) => "read whole".to_string(),
                    (Pipeline::WholePage, Some(why)) => format!("read whole — {why}"),
                };
                out.push_str(&format!(
                    "\n=== page {} of {} · OCR transcript ({}, {how}; a model's reading, not the file's text) ===\n",
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

    /// Pages that were to be transcribed and were not wholly: a page that
    /// failed, or one with a region that could not be read.
    pub fn incomplete_pages(&self) -> usize {
        self.pages
            .iter()
            .filter(|p| {
                p.ocr_error.is_some() || p.ocr.as_ref().is_some_and(|o| o.failed_regions() > 0)
            })
            .count()
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

/// How one call's OCR pages are read, decided at its first OCR page.
enum Stage {
    /// `layout = false`.
    WholePage,
    /// The layout model is there; `key` is the cache's pipeline key and the
    /// worker starts on the first page that is not cached.
    Layout {
        key: String,
        worker: Option<Box<LayoutChild>>,
    },
    /// Configured and not usable, and why — every OCR page is read whole and
    /// says so.
    Unavailable(String),
}

/// The layout model's hash, remembered while the file is unchanged.
type HashMemo = std::sync::Mutex<Option<(PathBuf, u64, std::time::SystemTime, String)>>;

/// The extractor: config, the confined renderer, the OCR client if any, the
/// cache if any. Cheap to build; holds no connection until used.
pub struct Extractor {
    cfg: DocumentsConfig,
    renderer: Renderer,
    ocr: Option<OcrClient>,
    cache: Option<Cache>,
    /// `None` when `layout = false`, or its paths could not be resolved.
    layout: Option<LayoutModel>,
    layout_hash: HashMemo,
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
        let layout = if cfg.ocr && cfg.layout {
            Some(LayoutModel::new(
                cfg.layout_python_path()?,
                cfg.layout_model_path()?,
                cfg.confine,
                cfg.layout_memory_mb,
                cfg.layout_threads,
                Duration::from_secs(cfg.page_timeout_secs.max(1)),
            ))
        } else {
            None
        };
        Ok(Extractor {
            renderer: Renderer::new(&cfg),
            ocr,
            cache,
            layout,
            layout_hash: std::sync::Mutex::new(None),
            cfg,
        })
    }

    pub fn config(&self) -> &DocumentsConfig {
        &self.cfg
    }

    /// The sha256 of the layout model file, read once per change of the file
    /// — it is part of every layout cache key.
    async fn layout_model_hash(&self, path: &Path) -> Result<String> {
        let meta = std::fs::metadata(path)?;
        let stamp = (path.to_path_buf(), meta.len(), meta.modified()?);
        {
            let memo = self.layout_hash.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((p, len, mtime, hash)) = memo.as_ref() {
                if (p, *len, *mtime) == (&stamp.0, stamp.1, stamp.2) {
                    return Ok(hash.clone());
                }
            }
        }
        let owned = path.to_path_buf();
        let hash = tokio::task::spawn_blocking(move || -> Result<String> {
            use std::io::Read;
            let mut f = std::fs::File::open(&owned)?;
            let mut h = Sha256::new();
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
        })
        .await??;
        *self.layout_hash.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((stamp.0, stamp.1, stamp.2, hash.clone()));
        Ok(hash)
    }

    /// Decide how this call reads its OCR pages.
    async fn stage(&self) -> Stage {
        let Some(model) = &self.layout else {
            return Stage::WholePage;
        };
        let found = match model.locate() {
            Ok(f) => f,
            Err(e) => return Stage::Unavailable(format!("{e:#}")),
        };
        match self.layout_model_hash(&found.model).await {
            Ok(hash) => Stage::Layout {
                key: format!("{LAYOUT_PIPELINE}-{}", &hash[..16]),
                worker: None,
            },
            Err(e) => Stage::Unavailable(format!(
                "cannot read the layout model {}: {e:#}",
                found.model.display()
            )),
        }
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
        let mut stage: Option<Stage> = None;
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
                        // Show the text layer — thin as it may be — so the
                        // page is not silent: a few words of the file's own
                        // are still its words (the OCR-failure branch's rule).
                        page.text
                            .get_or_insert_with(|| layer.text[n as usize - 1].clone());
                    }
                    Some(client) => {
                        if stage.is_none() {
                            stage = Some(self.stage().await);
                        }
                        let st = stage.as_mut().expect("decided above");
                        let size = layer
                            .sizes
                            .get(n as usize - 1)
                            .copied()
                            .unwrap_or((612.0, 792.0));
                        if let Some(hit) = self.cached_ocr(&sha, client, st, n) {
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
                            // One bound for the whole page: each region read
                            // carries its own timeout, and a page of many
                            // regions against a wedged server would otherwise
                            // run for page_timeout × regions (found on review).
                            let limit = Duration::from_secs(self.cfg.page_timeout_secs.max(1));
                            let work = async {
                                match tokio::time::timeout(
                                    limit,
                                    self.read_page(client, st, s, &sha, n, size),
                                )
                                .await
                                {
                                    Ok(r) => r,
                                    Err(_) => {
                                        // The read was dropped mid-page, and with
                                        // it `read_page`'s own reset: a layout
                                        // worker whose reply is still in the pipe
                                        // would hand the next page this page's
                                        // boxes — wrong crops, parsed as valid
                                        // and cached (found on review). So the
                                        // worker goes; the next page starts fresh.
                                        if let Stage::Layout { worker, .. } = &mut *st {
                                            *worker = None;
                                        }
                                        Err(anyhow!("the page did not finish within {limit:?}"))
                                    }
                                }
                            };
                            // Raced against the token: a cancel drops the page
                            // mid-read — the region requests in flight and the
                            // layout exchange with it — rather than waiting it out.
                            let Some(result) = until_cancelled(cancel, work).await else {
                                cancelled = true;
                                deferred.push(n);
                                out.push(page);
                                continue;
                            };
                            match result {
                                Ok(ocr) => page.ocr = Some(ocr),
                                Err(e) => {
                                    page.ocr_error = Some(format!("{e:#}"));
                                    // A thin text layer is still the file's
                                    // own words: shown when OCR failed, not
                                    // dropped behind the error (found on
                                    // review) — the ocr = false branch's rule.
                                    page.text
                                        .get_or_insert_with(|| layer.text[n as usize - 1].clone());
                                }
                            }
                        }
                    }
                }
            }
            out.push(page);
        }
        let layout_unavailable = match stage {
            Some(Stage::Unavailable(why)) => Some(why),
            _ => None,
        };
        Ok(Extraction {
            sha256: sha,
            pages_total: layer.pages,
            mode,
            pages: out,
            ocr_deferred: deferred,
            cancelled,
            confinement,
            layer_cached,
            layout_unavailable,
        })
    }

    /// A cached transcript for this page under this call's recipe, labelled
    /// as it is served.
    fn cached_ocr(&self, sha: &str, client: &OcrClient, stage: &Stage, n: u32) -> Option<OcrPage> {
        let cache = self.cache.as_ref()?;
        match stage {
            Stage::Layout { key, .. } => cache.load_ocr(sha, client.model(), key, n),
            Stage::WholePage => cache.load_ocr(sha, client.model(), OCR_PIPELINE, n),
            Stage::Unavailable(_) => {
                let mut hit = cache.load_ocr(sha, client.model(), OCR_PIPELINE, n)?;
                hit.fallback = Some("the layout stage is unavailable".into());
                Some(hit)
            }
        }
    }

    /// Read one page that is not cached, by this call's recipe, and cache it
    /// if it was read whole.
    async fn read_page(
        &self,
        client: &OcrClient,
        stage: &mut Stage,
        s: &Scratch,
        sha: &str,
        n: u32,
        size: (f32, f32),
    ) -> Result<OcrPage> {
        if let Stage::Layout { worker, .. } = stage {
            if worker.is_none() {
                let model = self.layout.as_ref().expect("a layout stage has a model");
                match model.start().await {
                    Ok(w) => *worker = Some(Box::new(w)),
                    Err(e) => *stage = Stage::Unavailable(format!("{e:#}")),
                }
            }
        }
        let fallback = match stage {
            Stage::Layout { key, worker } => {
                let w = worker.as_mut().expect("started above");
                let read = self.read_by_layout(client, w, s, n, size).await;
                if read.is_err() {
                    // A worker that failed mid-page is not trusted with the
                    // next one: the next page starts a fresh one.
                    *worker = None;
                }
                match read? {
                    Some(ocr) => {
                        if let (Some(cache), 0) = (&self.cache, ocr.failed_regions()) {
                            // A cache is a saving, not a precondition.
                            if let Err(e) = cache.store_ocr(sha, client.model(), key, n, &ocr) {
                                tracing::warn!("document cache: page {n} not stored: {e:#}");
                            }
                        }
                        return Ok(ocr);
                    }
                    None => Some("the layout model found nothing to read on this page".to_string()),
                }
            }
            Stage::Unavailable(_) => Some("the layout stage is unavailable".to_string()),
            Stage::WholePage => None,
        };
        let png = self.renderer.page_png(s, n, size).await?;
        let mut ocr = client.page(&png).await?;
        ocr.fallback = fallback;
        if let Some(cache) = &self.cache {
            // A page the layout stage read whole is that pipeline's answer for
            // this page, so it is stored under the key `cached_ocr` reads for
            // it, note and all — under OCR_PIPELINE alone the stage never found
            // it again, and every call re-paid the layout pass and a
            // whole-page read (found on review). The whole-page recipe's own
            // entry keeps no note: a later `layout = false` read must not
            // inherit "the layout stage is unavailable".
            let stored = match stage {
                Stage::Layout { key, .. } => cache.store_ocr(sha, client.model(), key, n, &ocr),
                _ => {
                    let plain = OcrPage {
                        fallback: None,
                        ..ocr.clone()
                    };
                    cache.store_ocr(sha, client.model(), OCR_PIPELINE, n, &plain)
                }
            };
            if let Err(e) = stored {
                tracing::warn!("document cache: page {n} not stored: {e:#}");
            }
        }
        Ok(ocr)
    }

    /// The layout reading of one page: regions found, each read with its own
    /// prompt, assembled in reading order. `None` when the layout model finds
    /// nothing to read, so the caller can read the page whole instead of
    /// recording a blank one.
    async fn read_by_layout(
        &self,
        client: &OcrClient,
        worker: &mut LayoutChild,
        s: &Scratch,
        n: u32,
        size: (f32, f32),
    ) -> Result<Option<OcrPage>> {
        use futures::StreamExt;
        let dpi = crate::layout::layout_dpi(size.0, size.1);
        let img = self.renderer.page_image(s, n, dpi).await?;
        let started = Instant::now();
        let found = worker.detect(&img).await.context("the layout stage")?;
        let layout_secs = started.elapsed().as_secs_f64();
        // Nothing *readable* is the same as nothing found: a page whose only
        // regions are figures (no region with a prompt) would otherwise be
        // transcribed as `*[image]*`, cached as a complete reading and served
        // from then on — where design §5 reads such a page whole (found on
        // review).
        if !found.iter().any(|d| Task::of(d.label()).prompt().is_some()) {
            return Ok(None);
        }
        // Pixels of the render → PDF points, the frame the text layer's own
        // regions use.
        let pt = 72.0 / dpi as f32;
        let mut regions: Vec<LayoutRegion> = found
            .iter()
            .map(|d| LayoutRegion {
                label: d.label().to_string(),
                score: d.score,
                bbox: d.bbox.map(|v| v * pt),
                markdown: None,
                error: None,
            })
            .collect();
        let mut jobs = Vec::new();
        for (i, d) in found.iter().enumerate() {
            let Some(prompt) = Task::of(d.label()).prompt() else {
                continue;
            };
            if i >= crate::layout::MAX_REGIONS {
                regions[i].error = Some(format!(
                    "past the per-page limit of {} regions",
                    crate::layout::MAX_REGIONS
                ));
                continue;
            }
            match crop_png(&img, d.bbox) {
                Ok(png) => jobs.push((i, png, prompt)),
                Err(e) => regions[i].error = Some(format!("{e:#}")),
            }
        }
        // Boxed so the future's type names no closure lifetime — the tool's
        // `async_trait` future must be `Send` for every one.
        type Read<'a> = std::pin::Pin<
            Box<dyn std::future::Future<Output = (usize, Result<OcrPage>)> + Send + 'a>,
        >;
        let reads: Vec<Read<'_>> = jobs
            .into_iter()
            .map(|(i, png, prompt)| -> Read<'_> {
                Box::pin(async move { (i, client.recognize(&png, prompt).await) })
            })
            .collect();
        let read: Vec<(usize, Result<OcrPage>)> = futures::stream::iter(reads)
            .buffered(REGION_CONCURRENCY)
            .collect()
            .await;
        let (mut prompt_tokens, mut completion_tokens) = (0, 0);
        for (i, result) in read {
            match result {
                Ok(r) => {
                    prompt_tokens += r.prompt_tokens;
                    completion_tokens += r.completion_tokens;
                    regions[i].markdown = Some(r.markdown);
                }
                Err(e) => regions[i].error = Some(format!("{e:#}")),
            }
        }
        let markdown = crate::layout::assemble(&regions);
        if markdown.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(OcrPage {
            markdown,
            model: client.model().to_string(),
            prompt_tokens,
            completion_tokens,
            secs: started.elapsed().as_secs_f64(),
            pipeline: Pipeline::Layout,
            regions,
            layout_secs,
            fallback: None,
        }))
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

    /// The sandbox's own refusal is ours; anything poppler said is the
    /// document's.
    #[test]
    fn a_sandbox_refusal_is_not_document_text() {
        assert!(stderr_is_the_sandboxs(
            "bwrap: No permissions to create new namespace\n"
        ));
        assert!(!stderr_is_the_sandboxs(
            "Syntax Error: Couldn't find trailer dictionary\n"
        ));
        assert!(!stderr_is_the_sandboxs(
            "bwrap: warning\nSyntax Error (12): Illegal character <2f> in (hi)\n"
        ));
        assert!(!stderr_is_the_sandboxs(""));
    }

    /// A parser's words survive context-wrapping as document text; our own
    /// refusals never read as it.
    #[test]
    fn parser_output_is_document_text_and_our_refusals_are_not() {
        let e = anyhow::Error::new(ParserSaid("pdftotext failed: Syntax Error: (hi)".into()))
            .context("extracting the text layer");
        assert!(carries_document_text(&e));
        assert!(!carries_document_text(&anyhow!(
            "not a PDF (no %PDF- header in the first kilobyte)"
        )));
    }

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
            Cache::ocr_key("paddleocr-vl-1.6", OCR_PIPELINE),
            format!("paddleocr-vl-1.6-{OCR_PIPELINE}")
        );
        assert_eq!(
            Cache::ocr_key("../x y", OCR_PIPELINE),
            format!(".._x_y-{OCR_PIPELINE}")
        );
        // A layout reading is another key from a whole-page one, and the
        // pipeline part cannot climb out of the directory either.
        assert_ne!(
            Cache::ocr_key("m", &format!("{LAYOUT_PIPELINE}-0123abcd")),
            Cache::ocr_key("m", OCR_PIPELINE)
        );
        assert_eq!(Cache::ocr_key("m", "../ly1"), "m-___ly1");
    }

    /// Every directory from the cache root to a stored file is owner-only,
    /// and so is the file, even under a root that already existed at the
    /// umask default: the root's listing names every PDF the owner read, by
    /// hash (#410), and the file holds its text.
    #[cfg(unix)]
    #[test]
    fn the_whole_cache_path_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mecha-doc-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o775)).unwrap();
        let cache = Cache::new(dir.clone());
        let sha = sha256_hex(b"a private document");
        let ocr = OcrPage {
            markdown: "# Title".into(),
            model: "m".into(),
            prompt_tokens: 1,
            completion_tokens: 1,
            secs: 0.1,
            pipeline: Pipeline::WholePage,
            regions: Vec::new(),
            layout_secs: 0.0,
            fallback: None,
        };
        cache.store_ocr(&sha, "m", OCR_PIPELINE, 1, &ocr).unwrap();
        let file = cache.ocr_path(&sha, "m", OCR_PIPELINE, 1);
        // The file too: it holds the document's text.
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{} is {mode:o}", file.display());
        let mut at = file.parent().unwrap().to_path_buf();
        loop {
            let mode = std::fs::metadata(&at).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{} is {mode:o}", at.display());
            if at == dir {
                break;
            }
            at.pop();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A cache that is missing prunes nothing; one that cannot be read is an
    /// error, never "removed 0" (#410).
    #[cfg(unix)]
    #[test]
    fn prune_refuses_to_report_an_unreadable_cache_clean() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mecha-doc-unreadable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let missing = Cache::new(dir.join("absent"));
        assert!(missing.prune(Duration::ZERO).unwrap().is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
        let unreadable = Cache::new(dir.clone());
        let result = unreadable.prune(Duration::ZERO);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        // Root can read anything; the test means nothing there.
        if unsafe { libc::geteuid() } != 0 {
            assert!(result.is_err(), "{result:?}");
        }
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
            pipeline: Pipeline::WholePage,
            regions: Vec::new(),
            layout_secs: 0.0,
            fallback: None,
        };
        let m = "paddleocr-vl-1.6";
        cache.store_ocr(&sha, m, OCR_PIPELINE, 1, &ocr).unwrap();
        assert_eq!(cache.load_ocr(&sha, m, OCR_PIPELINE, 1), Some(ocr.clone()));
        // Another model's transcript is another key; so is the layout recipe's.
        assert!(cache
            .load_ocr(&sha, "other-model", OCR_PIPELINE, 1)
            .is_none());
        assert!(cache.load_ocr(&sha, m, LAYOUT_PIPELINE, 1).is_none());
        // An entry written before the pipeline field existed reads as the
        // whole-page recipe it was.
        let old = r##"{"markdown":"# Old","model":"m","prompt_tokens":1,"completion_tokens":1,"secs":1.0}"##;
        let old: OcrPage = serde_json::from_str(old).unwrap();
        assert_eq!(old.pipeline, Pipeline::WholePage);
        // A layout page with a region that failed is not served from the cache.
        let partial = OcrPage {
            pipeline: Pipeline::Layout,
            regions: vec![LayoutRegion {
                label: "table".into(),
                score: 0.9,
                bbox: [0.0; 4],
                markdown: None,
                error: Some("cut off".into()),
            }],
            ..ocr
        };
        cache
            .store_ocr(&sha, m, LAYOUT_PIPELINE, 2, &partial)
            .unwrap();
        assert!(cache.load_ocr(&sha, m, LAYOUT_PIPELINE, 2).is_none());

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
            layout_unavailable: None,
        };
        let s = ex.render("papers/a.pdf");
        assert!(s.contains("page 1 of 3 · text layer"));
        assert!(s.contains("page 2 of 3 · OCR failed"));
        assert!(s.contains("pages not yet transcribed: 3"));
        assert!(!s.contains("layout stage"));
        assert_eq!(ex.incomplete_pages(), 1);
    }

    /// A layout reading, a whole-page one by choice, and a whole-page one
    /// because the layout stage could not run are three different labels —
    /// the last never passes for the first.
    #[test]
    fn a_transcript_says_how_it_was_read_and_a_fallback_says_why() {
        let ocr = |pipeline, fallback: Option<&str>, regions| OcrPage {
            markdown: "Ada's table".into(),
            model: "paddleocr-vl-1.6".into(),
            prompt_tokens: 0,
            completion_tokens: 0,
            secs: 0.0,
            pipeline,
            regions,
            layout_secs: 0.0,
            fallback: fallback.map(str::to_string),
        };
        let page = |number, o| Page {
            number,
            text: None,
            has_text_layer: false,
            ocr: Some(o),
            ocr_error: None,
            ocr_cached: false,
            regions: Vec::new(),
        };
        let region = |error: Option<&str>| LayoutRegion {
            label: "table".into(),
            score: 0.9,
            bbox: [0.0; 4],
            markdown: None,
            error: error.map(str::to_string),
        };
        let mut ex = Extraction {
            sha256: sha256_hex(b"x"),
            pages_total: 3,
            mode: Mode::Ocr,
            pages: vec![
                page(
                    1,
                    ocr(Pipeline::Layout, None, vec![region(None), region(None)]),
                ),
                page(2, ocr(Pipeline::WholePage, None, Vec::new())),
                page(
                    3,
                    ocr(Pipeline::Layout, None, vec![region(Some("cut off"))]),
                ),
            ],
            ocr_deferred: Vec::new(),
            cancelled: false,
            confinement: "bwrap".into(),
            layer_cached: false,
            layout_unavailable: None,
        };
        let s = ex.render("a.pdf");
        assert!(
            s.contains("page 1 of 3 · OCR transcript (paddleocr-vl-1.6, read by region: 2;"),
            "{s}"
        );
        assert!(
            s.contains("page 2 of 3 · OCR transcript (paddleocr-vl-1.6, read whole;"),
            "{s}"
        );
        assert_eq!(
            ex.incomplete_pages(),
            1,
            "a failed region makes a page incomplete"
        );

        ex.pages[1].ocr.as_mut().unwrap().fallback = Some("the layout stage is unavailable".into());
        ex.layout_unavailable = Some("the layout model /m.onnx is not there".into());
        let s = ex.render("a.pdf");
        assert!(
            s.contains("read whole — the layout stage is unavailable;"),
            "{s}"
        );
        assert!(s.contains("the layout stage is unavailable, so OCR pages below were read whole"));
        assert!(s.contains("/m.onnx is not there"));
    }
}
