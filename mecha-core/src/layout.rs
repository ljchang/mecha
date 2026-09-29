//! The layout stage of document OCR: which regions a page has — a table, a
//! formula, a heading, a paragraph, a figure — where they are, and in what
//! order they are read, so each region can go to the OCR model with the
//! prompt that model was trained for (`docs/DOCUMENT-EXTRACTION-DESIGN.md` §5).
//!
//! **Why.** Whole-page prompting of PaddleOCR-VL reads prose and equations
//! well and tables badly — a dropped column, a row label swapped for its
//! neighbour's, measured on the Transformer and BERT papers — while the same
//! table *cropped* and sent with `Table Recognition:` came back whole. The
//! model's own pipeline is a layout model ahead of the VLM; this is that
//! layout model, PP-DocLayoutV3, in the ONNX export its publisher ships.
//!
//! **Confined, like the parser, and for the same reason.** The layout model
//! reads pixels a document's author chose. It runs in a child process under
//! [`crate::sandbox`] with the `[documents] confine` backend — no network, a
//! private `/tmp`, the system and exactly three extra paths read-only (the
//! interpreter's environment, its prefix, the model file), an rlimited
//! address space — and it never sees an image file: the page is decoded here
//! by the memory-safe `image` crate and written to the child as a fixed-size
//! float tensor. A child that cannot start under the confinement makes the
//! stage *unavailable*, and the extraction says so on every page it affects;
//! the model is never run unconfined unless `confine = "none"` is written out.
//!
//! **Nothing stays resident.** One child per extraction call, started on the
//! first page that needs it and killed when the call ends.
//!
//! The post-processing is PaddleX's for this model (`DetPostProcess.apply`
//! with the `PaddleOCR-VL-1.6.yaml` settings, then the pipeline's
//! `filter_overlap_boxes`), ported so the regions match what the model's
//! authors evaluate. `merge_blocks` — stitching neighbouring text regions
//! into one image to batch them — is a throughput optimisation and is not.

use crate::sandbox::{Backend, Sandbox, SandboxConfig};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The worker, compiled in: `python -I -B -c WORKER` runs exactly this text.
pub const WORKER: &str = include_str!("layout_worker.py");

/// The model's input side (`inference.yml`: `Resize` to 800 × 800, aspect
/// not kept).
pub const SIDE: u32 = 800;

/// One page's input: 3 × 800 × 800 float32.
pub const TENSOR_BYTES: usize = 3 * (SIDE as usize) * (SIDE as usize) * 4;

/// The pipeline's detection threshold (`PaddleOCR-VL-1.6.yaml`).
pub const THRESHOLD: f32 = 0.3;

/// More rows than this from the worker is refused as a protocol error — the
/// model's own cap is 300.
pub const MAX_DETECTIONS: u32 = 1000;

/// At most this many regions of one page go to the OCR model; the rest are
/// named on the page, not dropped silently. A page is rarely past 60.
pub const MAX_REGIONS: usize = 120;

/// The page is rendered at this resolution for the layout stage, and each
/// region is cropped from that same render (PaddleX renders PDFs at zoom 2,
/// 144 dpi). A small region is upscaled by the OCR server to its minimum
/// pixel budget either way; this is enough that it is not upscaled from mush.
pub const LAYOUT_DPI: f64 = 144.0;

/// A page render larger than this is rendered at a lower resolution.
pub const LAYOUT_MAX_PIXELS: f64 = 4_000_000.0;

/// The ready marker the worker writes once the model has loaded.
const READY: &[u8; 4] = b"MLY1";

pub const TABLE_PROMPT: &str = "Table Recognition:";
pub const FORMULA_PROMPT: &str = "Formula Recognition:";

/// PP-DocLayoutV3's classes, by id (`inference.yml` `label_list`).
pub const LABELS: [&str; 25] = [
    "abstract",
    "algorithm",
    "aside_text",
    "chart",
    "content",
    "display_formula",
    "doc_title",
    "figure_title",
    "footer",
    "footer_image",
    "footnote",
    "formula_number",
    "header",
    "header_image",
    "image",
    "inline_formula",
    "number",
    "paragraph_title",
    "reference",
    "reference_content",
    "seal",
    "table",
    "text",
    "vertical_text",
    "vision_footnote",
];

/// Classes whose box swallows what it contains (`layout_merge_bboxes_mode:
/// large`): a box ≥ 90% inside one of these is dropped.
const LARGE: [&str; 5] = [
    "chart",
    "display_formula",
    "doc_title",
    "inline_formula",
    "paragraph_title",
];

/// What the OCR model is asked of a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    /// `OCR:` — prose, titles, captions, footnotes.
    Text,
    /// `Table Recognition:` — OTSL, turned into a Markdown table.
    Table,
    /// `Formula Recognition:` — LaTeX.
    Formula,
    /// Not sent: images, charts and seals (PaddleOCR-VL 1.6's defaults
    /// leave chart and seal recognition off). Marked in place.
    Figure,
}

impl Task {
    pub fn of(label: &str) -> Task {
        match label {
            "table" => Task::Table,
            "display_formula" | "inline_formula" => Task::Formula,
            "image" | "header_image" | "footer_image" | "chart" | "seal" => Task::Figure,
            _ => Task::Text,
        }
    }

    pub fn prompt(self) -> Option<&'static str> {
        match self {
            Task::Text => Some(crate::document::OCR_PROMPT),
            Task::Table => Some(TABLE_PROMPT),
            Task::Formula => Some(FORMULA_PROMPT),
            Task::Figure => None,
        }
    }
}

/// One detection, in the pixels of the image it was scaled to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detection {
    /// Index into [`LABELS`].
    pub class: usize,
    pub score: f32,
    /// `[x0, y0, x1, y1]`.
    pub bbox: [f32; 4],
    /// The model's reading-order score: lower is read first.
    pub order: f32,
}

impl Detection {
    pub fn label(&self) -> &'static str {
        LABELS[self.class]
    }

    fn area(&self) -> f32 {
        (self.bbox[2] - self.bbox[0]).abs() * (self.bbox[3] - self.bbox[1]).abs()
    }
}

/// One region of a transcribed page: what the layout model called it, where
/// it is, and what the OCR model read there. Stored with the transcript so a
/// citation can later open the region it came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutRegion {
    pub label: String,
    pub score: f32,
    /// `[x_min, y_min, x_max, y_max]` in PDF points from the page's top-left
    /// — the frame [`crate::document::Region`] uses.
    pub bbox: [f32; 4],
    /// The OCR model's reading of the region; `None` for a figure, which is
    /// not sent, and for a region that failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    /// Why this region has no reading although it was sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The resolution a `w × h` point page is rendered at for the layout stage.
pub fn layout_dpi(width_pt: f32, height_pt: f32) -> u32 {
    let area = (width_pt as f64) * (height_pt as f64);
    if !area.is_finite() || area <= 0.0 {
        return LAYOUT_DPI as u32;
    }
    let fit = 72.0 * (LAYOUT_MAX_PIXELS / area).sqrt();
    LAYOUT_DPI.min(fit).clamp(50.0, 300.0) as u32
}

/// The model's input for one page: resized to 800 × 800 (bicubic, aspect not
/// kept — the model was trained so), scaled to `[0, 1]`, RGB, channel-first,
/// little-endian float32.
pub fn tensor(img: &image::RgbImage) -> Vec<u8> {
    let small = image::imageops::resize(img, SIDE, SIDE, image::imageops::FilterType::CatmullRom);
    let mut out = Vec::with_capacity(TENSOR_BYTES);
    for c in 0..3 {
        for p in small.pixels() {
            out.extend_from_slice(&(f32::from(p[c]) / 255.0).to_le_bytes());
        }
    }
    out
}

/// Parse the worker's answer for one page: `n` rows of seven float32. The
/// worker is confined but still reads hostile pixels, so its answer is
/// checked like any other untrusted input: a row with an unknown class or a
/// value that is not finite is dropped, never trusted.
pub fn parse_rows(n: u32, bytes: &[u8]) -> Result<Vec<Detection>> {
    if n > MAX_DETECTIONS {
        bail!("the layout worker reported {n} regions (more than {MAX_DETECTIONS})");
    }
    if bytes.len() != n as usize * 28 {
        bail!(
            "the layout worker sent {} bytes for {n} regions",
            bytes.len()
        );
    }
    let mut out = Vec::with_capacity(n as usize);
    // The length check above makes the remainder empty.
    let (rows, _) = bytes.as_chunks::<28>();
    for row in rows {
        let v: Vec<f32> = row
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        if v.iter().any(|x| !x.is_finite()) {
            continue;
        }
        let class = v[0];
        if class < 0.0 || class.fract() != 0.0 || class as usize >= LABELS.len() {
            continue;
        }
        out.push(Detection {
            class: class as usize,
            score: v[1],
            bbox: [v[2], v[3], v[4], v[5]],
            order: v[6],
        });
    }
    Ok(out)
}

/// PaddleX's IoU: pixel-inclusive (`+ 1`), as its NMS computes it.
fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0]) + 1.0).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1]) + 1.0).max(0.0);
    let inter = iw * ih;
    let area = |r: &[f32; 4]| (r[2] - r[0] + 1.0) * (r[3] - r[1] + 1.0);
    let union = area(a) + area(b) - inter;
    if union > 0.0 {
        inter / union
    } else {
        0.0
    }
}

/// The share of the smaller box inside the other (`mode="small"`).
fn overlap_small(a: &Detection, b: &Detection) -> f32 {
    let iw = (a.bbox[2].min(b.bbox[2]) - a.bbox[0].max(b.bbox[0])).max(0.0);
    let ih = (a.bbox[3].min(b.bbox[3]) - a.bbox[1].max(b.bbox[1])).max(0.0);
    let small = a.area().min(b.area());
    if small > 0.0 {
        iw * ih / small
    } else {
        0.0
    }
}

/// Is `a` at least 90% inside `b`?
fn contained(a: &Detection, b: &Detection) -> bool {
    let iw = (a.bbox[2].min(b.bbox[2]) - a.bbox[0].max(b.bbox[0])).max(0.0);
    let ih = (a.bbox[3].min(b.bbox[3]) - a.bbox[1].max(b.bbox[1])).max(0.0);
    let area = a.area();
    area > 0.0 && iw * ih / area >= 0.9
}

/// From the worker's raw rows (in the 800 × 800 input's pixels) to the
/// regions of a `width × height` page image, in reading order.
pub fn postprocess(raw: &[Detection], width: f32, height: f32) -> Vec<Detection> {
    let (sx, sy) = (width / SIDE as f32, height / SIDE as f32);
    // Threshold, in the page's pixels (the size filters below are absolute).
    let mut boxes: Vec<Detection> = raw
        .iter()
        .filter(|d| d.score > THRESHOLD)
        .map(|d| Detection {
            bbox: [
                d.bbox[0] * sx,
                d.bbox[1] * sy,
                d.bbox[2] * sx,
                d.bbox[3] * sy,
            ],
            ..*d
        })
        .collect();

    // NMS: IoU 0.6 within a class, 0.98 across classes, highest score first.
    boxes.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Detection> = Vec::new();
    for d in boxes {
        let dup = kept.iter().any(|k| {
            let limit = if k.class == d.class { 0.6 } else { 0.98 };
            iou(&k.bbox, &d.bbox) >= limit
        });
        if !dup {
            kept.push(d);
        }
    }

    // An `image` covering nearly the whole page is the page, not a figure.
    if kept.len() > 1 {
        let limit = if width > height { 0.82 } else { 0.93 } * width * height;
        let filtered: Vec<Detection> = kept
            .iter()
            .filter(|d| {
                if d.label() != "image" {
                    return true;
                }
                let w = d.bbox[2].min(width) - d.bbox[0].max(0.0);
                let h = d.bbox[3].min(height) - d.bbox[1].max(0.0);
                w * h <= limit
            })
            .copied()
            .collect();
        if !filtered.is_empty() {
            kept = filtered;
        }
    }

    // `large`: whatever sits inside a title, a formula or a chart is part of it.
    let swallowed: Vec<bool> = (0..kept.len())
        .map(|i| {
            (0..kept.len()).any(|j| {
                j != i && LARGE.contains(&kept[j].label()) && contained(&kept[i], &kept[j])
            })
        })
        .collect();
    let mut kept: Vec<Detection> = kept
        .into_iter()
        .zip(swallowed)
        .filter_map(|(d, s)| (!s).then_some(d))
        .collect();

    // Reading order, then clip to the image.
    kept.sort_by(|a, b| a.order.total_cmp(&b.order));
    let mut boxes: Vec<Detection> = kept
        .into_iter()
        .filter_map(|mut d| {
            d.bbox = [
                d.bbox[0].max(0.0),
                d.bbox[1].max(0.0),
                d.bbox[2].min(width),
                d.bbox[3].min(height),
            ];
            (d.bbox[2] > d.bbox[0] && d.bbox[3] > d.bbox[1]).then_some(d)
        })
        .filter(|d| d.label() != "reference")
        .collect();

    // The pipeline's `filter_overlap_boxes`: slivers go; an inline formula
    // half inside anything else is read as part of it; of two boxes mostly
    // overlapping, the larger stays — unless a table overlaps a figure, where
    // both are kept.
    let mut dropped = vec![false; boxes.len()];
    for i in 0..boxes.len() {
        let (w, h) = (
            boxes[i].bbox[2] - boxes[i].bbox[0],
            boxes[i].bbox[3] - boxes[i].bbox[1],
        );
        if w < 6.0 || h < 6.0 {
            dropped[i] = true;
        }
        for j in i + 1..boxes.len() {
            if dropped[i] || dropped[j] {
                continue;
            }
            let ratio = overlap_small(&boxes[i], &boxes[j]);
            let (li, lj) = (boxes[i].label(), boxes[j].label());
            if li == "inline_formula" || lj == "inline_formula" {
                if ratio > 0.5 {
                    dropped[i] |= li == "inline_formula";
                    dropped[j] |= lj == "inline_formula";
                }
                continue;
            }
            if ratio > 0.7 {
                const VISUAL: [&str; 4] = ["image", "table", "seal", "chart"];
                if li != lj && (VISUAL.contains(&li) || VISUAL.contains(&lj)) {
                    let has_table = li == "table" || lj == "table";
                    if !has_table || (VISUAL.contains(&li) && VISUAL.contains(&lj)) {
                        continue;
                    }
                }
                if boxes[i].area() >= boxes[j].area() {
                    dropped[j] = true;
                } else {
                    dropped[i] = true;
                }
            }
        }
    }
    let mut i = 0;
    boxes.retain(|_| {
        let keep = !dropped[i];
        i += 1;
        keep
    });
    boxes
}

/// A section heading's depth from its numbering — `3 Model` is 1, `3.2.1
/// Scaled` is 3, `A.1 Proofs` is 2, unnumbered is 1 — as PaddleX's
/// `format_title` counts it.
pub fn heading_depth(title: &str) -> usize {
    let first = title.split_whitespace().next().unwrap_or("");
    let token = first.trim_end_matches('.');
    let numbered = !token.is_empty()
        && token.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 3
                && (part.chars().all(|c| c.is_ascii_digit())
                    || (part.len() == 1 && part.chars().all(|c| c.is_ascii_uppercase())))
        })
        && (token.contains('.') || token.chars().all(|c| c.is_ascii_digit()));
    if numbered {
        (token.matches('.').count() + 1).min(5)
    } else {
        1
    }
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A formula reading without the delimiters the model sometimes adds.
fn bare_latex(s: &str) -> &str {
    let s = s.trim();
    for (open, close) in [("$$", "$$"), ("\\[", "\\]"), ("\\(", "\\)"), ("$", "$")] {
        if let Some(inner) = s.strip_prefix(open).and_then(|r| r.strip_suffix(close)) {
            return inner.trim();
        }
    }
    s
}

/// The page's transcript, from its regions in reading order: headings
/// marked, tables as Markdown, formulas as LaTeX, figures and failures named
/// in place.
pub fn assemble(regions: &[LayoutRegion]) -> String {
    let mut parts = Vec::with_capacity(regions.len());
    for r in regions {
        let label = r.label.as_str();
        let piece = match (&r.markdown, &r.error) {
            (_, Some(err)) => format!("*[{label} not transcribed: {err}]*"),
            (None, None) => format!("*[{label}]*"),
            (Some(text), None) => match label {
                "doc_title" => format!("# {}", one_line(text)),
                "paragraph_title" => {
                    let t = one_line(text);
                    format!("{} {t}", "#".repeat(heading_depth(&t) + 1))
                }
                "display_formula" => format!("$$\n{}\n$$", bare_latex(text)),
                "inline_formula" => format!("${}$", bare_latex(text)),
                _ => text.trim().to_string(),
            },
        };
        if !piece.trim().is_empty() {
            parts.push(piece);
        }
    }
    parts.join("\n\n")
}

// ── The confined worker ─────────────────────────────────────────────────

/// A private 0700 directory, deleted on drop: the worker's only writable
/// path, and its `$HOME`.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<Self> {
        let dir =
            std::env::temp_dir().join(format!("mecha-layout-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(TempDir(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Where the layout model and its interpreter are, and how to run them.
#[derive(Debug, Clone)]
pub struct LayoutModel {
    python: PathBuf,
    model: PathBuf,
    confine: Backend,
    memory_bytes: u64,
    threads: u32,
    page_timeout: Duration,
    ready_timeout: Duration,
}

/// What [`LayoutModel::locate`] found: the files, resolved.
#[derive(Debug, Clone)]
pub struct Located {
    pub python: PathBuf,
    /// The model file's canonical path — what the worker opens and what the
    /// confinement binds.
    pub model: PathBuf,
    /// Paths the confinement makes readable, beyond the system.
    pub readable: Vec<PathBuf>,
}

impl LayoutModel {
    pub fn new(
        python: PathBuf,
        model: PathBuf,
        confine: Backend,
        memory_mb: u64,
        threads: u32,
        page_timeout: Duration,
    ) -> Self {
        LayoutModel {
            python,
            model,
            confine,
            memory_bytes: memory_mb.saturating_mul(1024 * 1024),
            threads: threads.max(1),
            page_timeout,
            ready_timeout: Duration::from_secs(60),
        }
    }

    pub fn model_path(&self) -> &Path {
        &self.model
    }

    /// Find the interpreter and the model, or say which is missing and what
    /// installs it. Nothing runs here.
    pub fn locate(&self) -> Result<Located> {
        let install = "scripts/layout/install.sh installs both";
        let model = std::fs::canonicalize(&self.model).map_err(|e| {
            anyhow!(
                "the layout model {} is not there ({e}) — {install}",
                self.model.display()
            )
        })?;
        if !model.is_file() {
            bail!("the layout model {} is not a file", model.display());
        }
        let real_python = std::fs::canonicalize(&self.python).map_err(|e| {
            anyhow!(
                "the layout interpreter {} is not there ({e}) — {install}",
                self.python.display()
            )
        })?;
        // The environment the interpreter belongs to (`venv/bin/python` →
        // `venv`), as configured and as resolved: a venv's `python` is a
        // symlink to an interpreter whose own prefix must be readable too.
        let mut readable = Vec::new();
        for p in [&self.python, &real_python] {
            if let Some(root) = p.parent().and_then(Path::parent) {
                if !readable.iter().any(|r: &PathBuf| r == root) {
                    readable.push(root.to_path_buf());
                }
            }
        }
        readable.push(model.clone());
        Ok(Located {
            python: self.python.clone(),
            model,
            readable,
        })
    }

    /// Start the worker under the confinement and wait for it to load the
    /// model. Any failure — the confinement cannot start, the interpreter
    /// lacks onnxruntime, the model does not load — is an error naming it,
    /// and nothing runs unconfined.
    pub async fn start(&self) -> Result<LayoutChild> {
        let found = self.locate()?;
        let dir = TempDir::new()?;
        let sandbox = Sandbox::new(SandboxConfig {
            kind: self.confine,
            network: false,
            readable: found.readable.clone(),
            ..SandboxConfig::default()
        });
        let args = vec![
            "-I".to_string(),
            "-B".to_string(),
            "-c".to_string(),
            WORKER.to_string(),
            found.model.display().to_string(),
            self.threads.to_string(),
        ];
        let python = found.python.display().to_string();
        let mut cmd = sandbox
            .wrap_argv(&python, &args, &dir.0, &dir.0)
            .context("building the confined layout command")?;
        if !sandbox.is_enabled() {
            // Unconfined by the operator's explicit choice: still no
            // inherited environment.
            cmd.env_clear();
            cmd.env("PATH", "/usr/local/bin:/usr/bin:/bin");
            cmd.env("HOME", &dir.0);
        }
        // CPU: a backstop, generous — the wall clock per page is the bound
        // that matters, and ONNX Runtime spends CPU on several threads.
        let cpu = self
            .page_timeout
            .as_secs()
            .saturating_add(5)
            .saturating_mul(u64::from(self.threads))
            .saturating_mul(64);
        crate::document::limit(&mut cmd, self.memory_bytes, cpu, 16 * 1024 * 1024);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().with_context(|| {
            format!(
                "cannot start the layout worker confined by {}",
                sandbox.backend().as_str()
            )
        })?;
        let stdin = child.stdin.take().context("the layout worker's stdin")?;
        let stdout = child.stdout.take().context("the layout worker's stdout")?;
        let stderr = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut err) = child.stderr.take() {
            let tail = stderr.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = err.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let mut t = tail.lock().unwrap_or_else(|p| p.into_inner());
                    t.extend_from_slice(&buf[..n]);
                    let over = t.len().saturating_sub(8192);
                    t.drain(..over);
                }
            });
        }
        let mut worker = LayoutChild {
            child,
            stdin,
            stdout,
            stderr,
            timeout: self.page_timeout,
            backend: sandbox.backend().as_str().to_string(),
            _dir: dir,
        };
        let mut marker = [0u8; 4];
        let started =
            tokio::time::timeout(self.ready_timeout, worker.stdout.read_exact(&mut marker)).await;
        match started {
            Ok(Ok(_)) if &marker == READY => Ok(worker),
            Ok(Ok(_)) => {
                bail!("the layout worker answered with something other than its ready marker")
            }
            Ok(Err(e)) => Err(worker.failure(&format!("did not start ({e})")).await),
            Err(_) => Err(worker
                .failure(&format!("did not load within {:?}", self.ready_timeout))
                .await),
        }
    }
}

/// A running worker. Dropping it kills the process.
pub struct LayoutChild {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    stderr: Arc<Mutex<Vec<u8>>>,
    timeout: Duration,
    backend: String,
    _dir: TempDir,
}

impl LayoutChild {
    /// The confinement the worker runs under, for the record.
    pub fn backend(&self) -> &str {
        &self.backend
    }

    /// One page's regions, in reading order, in the pixels of `img`.
    pub async fn detect(&mut self, img: &image::RgbImage) -> Result<Vec<Detection>> {
        let input = {
            let img = img.clone();
            tokio::task::spawn_blocking(move || tensor(&img)).await?
        };
        let exchange = async {
            self.stdin.write_all(&input).await?;
            self.stdin.flush().await?;
            let n = self.stdout.read_u32_le().await?;
            if n > MAX_DETECTIONS {
                bail!("the layout worker reported {n} regions (more than {MAX_DETECTIONS})");
            }
            let mut rows = vec![0u8; n as usize * 28];
            self.stdout.read_exact(&mut rows).await?;
            parse_rows(n, &rows)
        };
        let raw = match tokio::time::timeout(self.timeout, exchange).await {
            Ok(Ok(rows)) => rows,
            Ok(Err(e)) => return Err(self.failure(&format!("failed ({e:#})")).await),
            Err(_) => {
                return Err(self
                    .failure(&format!("did not answer within {:?}", self.timeout))
                    .await)
            }
        };
        Ok(postprocess(&raw, img.width() as f32, img.height() as f32))
    }

    /// Kill the worker and name what it said on the way down.
    async fn failure(&mut self, what: &str) -> anyhow::Error {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
        // Let the stderr reader catch the last lines.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let tail = {
            let t = self.stderr.lock().unwrap_or_else(|p| p.into_inner());
            String::from_utf8_lossy(&t).into_owned()
        };
        let tail: Vec<&str> = tail.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = tail[tail.len().saturating_sub(3)..].join(" / ");
        if tail.is_empty() {
            anyhow!("the layout worker ({} confinement) {what}", self.backend)
        } else {
            anyhow!(
                "the layout worker ({} confinement) {what}: {}",
                self.backend,
                tail.trim()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(label: &str, score: f32, bbox: [f32; 4], order: f32) -> Detection {
        Detection {
            class: LABELS.iter().position(|l| *l == label).unwrap(),
            score,
            bbox,
            order,
        }
    }

    /// In the 800 × 800 input's pixels, scaled to an 800 × 800 page so the
    /// numbers read as they are.
    fn run(raw: &[Detection]) -> Vec<(&'static str, [f32; 4])> {
        postprocess(raw, 800.0, 800.0)
            .into_iter()
            .map(|d| (d.label(), d.bbox))
            .collect()
    }

    #[test]
    fn regions_come_back_thresholded_deduplicated_and_in_reading_order() {
        let raw = [
            det("text", 0.9, [10.0, 300.0, 400.0, 400.0], 3.0),
            det("paragraph_title", 0.8, [10.0, 250.0, 200.0, 270.0], 2.0),
            det("doc_title", 0.95, [10.0, 10.0, 700.0, 60.0], 1.0),
            // Below the threshold.
            det("table", 0.29, [10.0, 500.0, 700.0, 700.0], 0.0),
            // A near-duplicate of the text, same class, lower score.
            det("text", 0.5, [11.0, 301.0, 401.0, 399.0], 9.0),
        ];
        let got: Vec<_> = run(&raw).into_iter().map(|(l, _)| l).collect();
        assert_eq!(got, vec!["doc_title", "paragraph_title", "text"]);
    }

    #[test]
    fn what_sits_inside_a_title_or_a_formula_is_part_of_it_and_inline_math_joins_its_paragraph() {
        let raw = [
            det("text", 0.9, [10.0, 100.0, 700.0, 300.0], 1.0),
            // Inside the paragraph: read as part of it.
            det("inline_formula", 0.8, [50.0, 120.0, 90.0, 140.0], 2.0),
            det("display_formula", 0.9, [100.0, 400.0, 600.0, 460.0], 3.0),
            // Inside the display formula (`large`): swallowed.
            det("text", 0.7, [120.0, 410.0, 200.0, 450.0], 4.0),
            // A standalone inline formula stays.
            det("inline_formula", 0.8, [100.0, 600.0, 160.0, 620.0], 5.0),
            // `reference` is a container; its entries are the content.
            det("reference", 0.9, [10.0, 650.0, 700.0, 790.0], 6.0),
            det("reference_content", 0.9, [12.0, 660.0, 690.0, 700.0], 7.0),
            // A sliver.
            det("text", 0.9, [10.0, 720.0, 700.0, 724.0], 8.0),
        ];
        let got: Vec<_> = run(&raw).into_iter().map(|(l, _)| l).collect();
        assert_eq!(
            got,
            vec![
                "text",
                "display_formula",
                "inline_formula",
                "reference_content"
            ]
        );
    }

    #[test]
    fn a_table_over_a_figure_keeps_both_and_mostly_overlapping_text_keeps_the_larger() {
        let raw = [
            det("table", 0.9, [10.0, 10.0, 400.0, 300.0], 1.0),
            det("image", 0.9, [20.0, 20.0, 390.0, 290.0], 2.0),
            det("text", 0.9, [10.0, 400.0, 700.0, 500.0], 3.0),
            det("abstract", 0.8, [15.0, 405.0, 690.0, 495.0], 4.0),
        ];
        let got: Vec<_> = run(&raw).into_iter().map(|(l, _)| l).collect();
        assert_eq!(got, vec!["table", "image", "text"]);
        // A whole-page `image` is the page, not a figure — dropped when there
        // is anything else.
        let raw = [
            det("image", 0.9, [0.0, 0.0, 800.0, 800.0], 1.0),
            det("text", 0.9, [10.0, 400.0, 700.0, 500.0], 2.0),
        ];
        assert_eq!(run(&raw).len(), 1);
    }

    #[test]
    fn boxes_scale_to_the_page_and_are_clipped_to_it() {
        let raw = [det("text", 0.9, [-10.0, 400.0, 900.0, 800.0], 1.0)];
        let got = postprocess(&raw, 1600.0, 400.0);
        assert_eq!(got[0].bbox, [0.0, 200.0, 1600.0, 400.0]);
    }

    #[test]
    fn the_workers_rows_are_checked_like_untrusted_input() {
        let row = |v: [f32; 7]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let mut bytes = row([22.0, 0.9, 1.0, 2.0, 3.0, 4.0, 0.0]);
        bytes.extend(row([99.0, 0.9, 1.0, 2.0, 3.0, 4.0, 0.0])); // no such class
        bytes.extend(row([21.5, 0.9, 1.0, 2.0, 3.0, 4.0, 0.0])); // not an id
        bytes.extend(row([21.0, f32::NAN, 1.0, 2.0, 3.0, 4.0, 0.0]));
        let got = parse_rows(4, &bytes).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].label(), "text");
        assert!(
            parse_rows(3, &bytes).is_err(),
            "length must match the count"
        );
        assert!(parse_rows(MAX_DETECTIONS + 1, &[]).is_err());
    }

    #[test]
    fn the_tensor_is_channel_first_rgb_in_unit_range() {
        let img = image::RgbImage::from_pixel(40, 20, image::Rgb([255, 0, 51]));
        let t = tensor(&img);
        assert_eq!(t.len(), TENSOR_BYTES);
        let at =
            |i: usize| f32::from_le_bytes([t[i * 4], t[i * 4 + 1], t[i * 4 + 2], t[i * 4 + 3]]);
        let plane = (SIDE * SIDE) as usize;
        assert!((at(0) - 1.0).abs() < 1e-6);
        assert!(at(plane).abs() < 1e-6);
        assert!((at(2 * plane) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn headings_are_marked_by_their_numbering() {
        assert_eq!(heading_depth("Introduction"), 1);
        assert_eq!(heading_depth("3 Model Architecture"), 1);
        assert_eq!(heading_depth("3.2 Attention"), 2);
        assert_eq!(heading_depth("3.2.1 Scaled Dot-Product Attention"), 3);
        assert_eq!(heading_depth("A.1 Proofs"), 2);
        assert_eq!(heading_depth("e.g. something"), 1);
        assert_eq!(heading_depth("1.2.3.4.5.6.7 Deep"), 5);
    }

    #[test]
    fn a_page_assembles_in_order_with_headings_tables_formulas_and_failures_named() {
        let r = |label: &str, md: Option<&str>, err: Option<&str>| LayoutRegion {
            label: label.into(),
            score: 0.9,
            bbox: [0.0; 4],
            markdown: md.map(str::to_string),
            error: err.map(str::to_string),
        };
        let page = assemble(&[
            r("doc_title", Some("Ledgers of\nthe Guild"), None),
            r("paragraph_title", Some("3.2 Results"), None),
            r("text", Some("Priya checked the totals. "), None),
            r("display_formula", Some("\\[ a + b \\]"), None),
            r("inline_formula", Some("$x$"), None),
            r("table", Some("| A | B |\n| --- | --- |\n| 1 | 2 |"), None),
            r("image", None, None),
            r("table", None, Some("cut off at the token limit")),
        ]);
        assert_eq!(
            page,
            "# Ledgers of the Guild\n\n### 3.2 Results\n\nPriya checked the totals.\n\n$$\na + b\n$$\n\n$x$\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\n*[image]*\n\n*[table not transcribed: cut off at the token limit]*"
        );
    }

    #[test]
    fn each_label_goes_to_the_prompt_its_model_was_trained_for() {
        assert_eq!(Task::of("table").prompt(), Some(TABLE_PROMPT));
        assert_eq!(Task::of("display_formula").prompt(), Some(FORMULA_PROMPT));
        assert_eq!(Task::of("paragraph_title").prompt(), Some("OCR:"));
        assert_eq!(Task::of("chart").prompt(), None);
        assert_eq!(Task::of("image").prompt(), None);
        // Every label the model has is routed somewhere.
        for l in LABELS {
            let _ = Task::of(l);
        }
    }

    #[test]
    fn a_large_page_renders_at_a_lower_resolution() {
        assert_eq!(layout_dpi(612.0, 792.0), 144);
        assert!(layout_dpi(2000.0, 3000.0) < 144);
        assert_eq!(layout_dpi(0.0, 1.0), 144);
    }
}
