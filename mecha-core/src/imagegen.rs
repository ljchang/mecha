//! Local image generation: the `image_generate` tool, and the one backend it
//! speaks today (ComfyUI running Qwen-Image 2.1).
//!
//! **The model never authors the workflow.** ComfyUI's `/prompt` executes any
//! node graph it is handed — nodes that write files, fetch URLs, or run a
//! custom node's code. So the graph is fixed here, in code, and the model
//! supplies typed values only: a scene (or the fields of one that change), a
//! retouch in words, a size from a closed set, and workspace paths of the
//! picture, its mask and a room photo. This module writes the prompt from
//! them (`picture::plan` picks the render; IMAGE-DESIGN.md §5), and the
//! prompt reaches the graph as a JSON string value, so nothing in it can
//! become a node; a reference reaches it as a name the server chose.
//!
//! **No egress, and the declaration is earned rather than asserted.** The
//! schema has no destination field and [`loopback_url`] refuses any server not
//! on this machine, so a prompt — model-written, from a conversation that may
//! hold private data — goes nowhere but a local process. `[image]` also loads
//! from the global config only (`merge_file` strips it from a project layer):
//! a cloned repository must not be able to choose the address.
//!
//! **The result lands in the run's own workspace**, under `images/`, which is
//! the reason this is a builtin and not an MCP server: a server is spawned
//! once in one directory (`fixed_workspace`), while `mecha serve` jails each
//! chat session separately and serves downloads from that jail only.
//!
//! **The model sees what it made on request, not by default**: the result is a
//! path and a seed, and `image_view` puts the picture in front of it when the
//! task needs a look (`tool::image_view`). Returning the pixels every time
//! would spend ~1000 tokens of context per picture for the rest of the
//! conversation, most of them on pictures nobody asked the model to check.
//! The seed comes back too, for the record: the model never sends one in a
//! chat. The harness reuses a picture's seed where it helps (a restage on a
//! words setting keeps its room), every edit samples fresh (#306), and a
//! redraw takes a seed the picture was not drawn at.
//!
//! **Each picture's scene is recorded** outside the jail
//! ([`crate::scene::SceneSlot`]), keyed by the picture's bytes, so a change
//! to it is read against what it was drawn as. The manifest beside the PNG
//! keeps only what owner-facing doors read: nothing reads a scene back out of
//! the jail.
//!
//! **What the server keeps, it is asked to drop.** Every job's history entry
//! (the prompt, the file names) is deleted however the job ends; with
//! `[image] server_temp_dir` set, so are the uploaded references and the
//! preview it wrote, by the names it returned. A run that must survive this
//! process dying — an incognito chat's — also gets a trail
//! (`ToolCtx::image_trail`): the job's id and each file's name, written
//! *before* the server has them, for [`forget_trail`] to act on later.
//!
//! The request is shaped like stable-diffusion.cpp's native API (prompt, size,
//! steps, seed in; PNG bytes out; a job that can be cancelled) rather than
//! like ComfyUI's graphs, because that shape is model-agnostic and a second
//! backend should be an adapter, not a new tool.

use crate::tool::{Capabilities, Tool, ToolCtx, ToolOutput};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Which server `[image]` talks to. One variant today; a closed set because
/// each is a hand-written adapter, never something a file can extend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageBackend {
    #[default]
    Comfyui,
}

/// `[image]`: where the local image server is and what it should load.
///
/// Present means the `image_generate` tool is registered; absent means it is
/// not. Global-file only (see the module doc for why the address matters).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImageConfig {
    pub backend: ImageBackend,
    /// The server's base URL. Must be loopback: see [`loopback_url`].
    pub url: String,
    /// File names as the server lists them (ComfyUI: under `models/`). A
    /// `.gguf` diffusion model loads through the ComfyUI-GGUF custom node,
    /// anything else through ComfyUI's own `UNETLoader` ([`unet_loader`]).
    pub diffusion_model: String,
    pub text_encoder: String,
    pub vae: String,
    /// Denoising steps. The reference setting for Qwen-Image 2.1 is 40.
    pub steps: u32,
    /// How long one generation may take before it is abandoned.
    pub timeout_secs: u64,
    /// Refuse to start below this much available memory, in MB. `0` skips
    /// the check. On unified memory (GB10) the GPU's allocations come out of
    /// the same pool as everything else, and the first generation on this
    /// machine took it down alongside `llama-server` and a parallel link. A
    /// generation from a server holding no model needed ~18.5 GB above its
    /// idle footprint with the Q4 GGUF (GPU + RSS, measured 2026-10-02; the
    /// earlier "15 GB peak" counted the GPU side only), and ~0.7 GB more with
    /// the int8 file (2026-10-04); an idle-reset server holds no model, so
    /// the default covers that with a little room.
    pub min_available_mb: u64,
    /// Ask the server to unload its models this long after the last
    /// generation. `0` keeps them. Best effort only: the timer lives in this
    /// process and dies with it (a `serve` restarted after the last picture
    /// left 12.2 GB held for 10 h on 2026-10-02), and on unified memory
    /// `/free` moves the weights into the server's RSS rather than releasing
    /// them. The durable release is `scripts/comfyui/comfyui-idle-reset`,
    /// beside the server.
    pub unload_after_secs: u64,
    /// The directory the server writes its temp files into — for ComfyUI,
    /// the `--temp-directory` path with `temp` appended. When set, each job's
    /// uploaded references and preview are deleted there, by the names the
    /// server returned, however the job ends. Unset, they stay until the
    /// server restarts, and an incognito chat withholds the tool: a promise
    /// that cannot be kept is not made (`INCOGNITO-DESIGN.md` §6.3).
    pub server_temp_dir: Option<std::path::PathBuf>,
}

impl Default for ImageConfig {
    fn default() -> Self {
        ImageConfig {
            backend: ImageBackend::Comfyui,
            url: "http://127.0.0.1:8188".into(),
            // int8 ConvRot, not the Q4 GGUF: 28 s against 64 s a picture at
            // 40 steps on the GB10, and 37 s against ~80 s an edit, for
            // about 0.7 GB more at peak and the same pictures seed for seed
            // (measured 2026-10-04; ARCHITECTURE §Image generation). A GGUF
            // re-quantises every weight on every step; the ConvRot file
            // runs on native int8 kernels.
            diffusion_model: "qwen_image_2.1_int8_convrot.safetensors".into(),
            text_encoder: "qwen3vl_8b_w4a8.safetensors".into(),
            vae: "qwen_image_2.1_vae_bf16.safetensors".into(),
            steps: 40,
            timeout_secs: 600,
            // `LOAD_COST_MB` is derived from this figure; change both.
            // 20 GiB since the int8 default: its ~19.2 GiB load no longer
            // fit under the Q4's 19 (2026-10-04).
            min_available_mb: 20_480,
            // Fifteen minutes, matching `comfyui-idle-reset`'s window: a
            // picture inside a quarter hour of the last is drawn warm. A cold
            // one took 145–180 s against 36–63 s warm, 2026-10-08, the
            // restart wait included (the owner's ruling, that day).
            unload_after_secs: 900,
            server_temp_dir: None,
        }
    }
}

/// Whether a URL's host is this machine: a loopback IP, or `localhost`.
/// The one statement of that rule — [`loopback_url`] and the point-wise
/// pass's R29 check (`pointwise::on_this_machine`) both ask it.
pub fn is_loopback(url: &reqwest::Url) -> bool {
    // `host_str` brackets an IPv6 literal; an IP parses, anything else is a
    // name, and the only name that is this machine by definition is localhost.
    match url
        .host_str()
        .map(|h| h.trim_start_matches('[').trim_end_matches(']'))
    {
        Some(host) => match host.parse::<std::net::IpAddr>() {
            Ok(ip) => ip.is_loopback(),
            Err(_) => host.eq_ignore_ascii_case("localhost"),
        },
        None => false,
    }
}

/// The image server's URL, if it is on this machine.
///
/// The tool declares no egress, and this is what makes that true: a host that
/// is not loopback is refused, so the declaration cannot be configured into a
/// lie. A tunnel listening on loopback (`ssh -L`) still passes — the operator
/// built that on purpose, and no URL can reveal it.
pub fn loopback_url(raw: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw).with_context(|| format!("[image] url `{raw}`"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("[image] url `{raw}` must be http or https");
    }
    if !is_loopback(&url) {
        bail!(
            "[image] url `{raw}` is not on this machine — the image tool sends model-written \
             prompts to it, so it must be loopback (127.0.0.1, ::1 or localhost)"
        );
    }
    Ok(url)
}

/// The sizes the tool offers. Closed, because every size is a memory and time
/// cost measured on this machine; 2048² is not offered until it has been.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Square,
    Landscape,
    Portrait,
}

impl Size {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "square" => Some(Size::Square),
            "landscape" => Some(Size::Landscape),
            "portrait" => Some(Size::Portrait),
            _ => None,
        }
    }

    /// The size whose shape is nearest a picture's, read from its header:
    /// a picture this tool drew is exactly one of them. `None` when the
    /// bytes do not decode.
    pub(crate) fn nearest(bytes: &[u8]) -> Option<Self> {
        let (w, h) = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()?;
        if w == 0 || h == 0 {
            return None;
        }
        let aspect = (w as f64 / h as f64).ln();
        [Size::Square, Size::Landscape, Size::Portrait]
            .into_iter()
            .min_by(|a, b| {
                let d = |s: Size| {
                    let (sw, sh) = s.dims();
                    ((sw as f64 / sh as f64).ln() - aspect).abs()
                };
                d(*a).total_cmp(&d(*b))
            })
    }

    /// Width and height; both multiples of 32, which the model requires.
    pub fn dims(self) -> (u32, u32) {
        match self {
            Size::Square => (1024, 1024),
            Size::Landscape => (1344, 768),
            Size::Portrait => (768, 1344),
        }
    }
}

/// One generation, validated. Backend-neutral on purpose.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub prompt: String,
    pub negative: String,
    /// Width and height, or `None` to follow the first reference's shape —
    /// which is only ever `None` when there is a reference to follow.
    pub size: Option<(u32, u32)>,
    pub steps: u32,
    pub seed: u64,
    /// Images to edit or draw from, in order: `<image1>` is the first.
    pub references: Vec<Reference>,
    /// The size each reference is scaled to, in pixels a side, before it
    /// reaches the model. An edit keeps 1024 — its canvas is the picture;
    /// library portraits go at [`crate::imagelib::REFERENCE_SIZE`], because
    /// four at 1024² doubled the time and 512² held identity (research E2, E10).
    pub reference_size: u32,
    /// Where an edit may change the picture: white is redrawn, black is kept
    /// — the owner painted it, and it arrives here already sized to the
    /// canvas and softened (`prepare_mask`). `None` redraws the whole frame.
    pub mask: Option<Reference>,
}

/// The reference size for a masked edit: the canvas at full detail, since
/// the result is laid back over the original pixel for pixel.
pub const EDIT_REFERENCE_SIZE: u32 = 1024;

/// The reference size for every other edit-shaped render (placed, restaged
/// on a photo, edited with a head crop for everyone in the picture; an edit
/// of the picture where anyone has no crop keeps [`EDIT_REFERENCE_SIZE`],
/// since the canvas is their only identity source): the owner's trial of 2026-10-08. mecha-a3 measured
/// one person placed on the owner's photo at 1024, 768 and 512 against her
/// portrait (ArcFace .88/.88/.87 over three seeds, within seed noise, and
/// near-identical by eye at one seed); the references are most of an edit's
/// cost, so they go at the size a library portrait already does. The output
/// size is named explicitly ([`canvas_dims`]), so the picture stays full size.
pub const UNMASKED_EDIT_REFERENCE_SIZE: u32 = 512;

/// The size an edit is drawn at for the canvas `bytes`: [`edit_canvas`] at
/// [`EDIT_REFERENCE_SIZE`] over its upright shape, which is what the encoder
/// drew when no size was named and references went at 1024 (1184×896 for a
/// 4:3 photo). Named explicitly now that the references go smaller, or the
/// picture would come out at their size. `None` when the shape cannot be
/// read; the edit then keeps 1024 references and names no size.
pub(crate) fn canvas_dims(bytes: &[u8]) -> Option<(u32, u32)> {
    // Upright, as every other reader of a reference: a phone photo stored
    // sideways under the fit threshold keeps its tag, and the server turns it
    // before encoding (review of #600).
    let img = crate::image::decode_upright(bytes, "the canvas").ok()?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return None;
    }
    // The encoder's own arithmetic, so a very wide room is not clamped to
    // another shape (review of #600).
    Some(edit_canvas(w, h, EDIT_REFERENCE_SIZE))
}

/// A reference image, read out of the run's workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    /// The workspace-relative path it was named by, for the result text.
    pub path: String,
    pub bytes: Vec<u8>,
    /// File extension for the upload, from the sniffed type.
    pub ext: &'static str,
}

/// A reference larger than this is refused rather than read. A phone photo
/// is well under it; the node resizes to about 1024² anyway.
const MAX_REFERENCE_BYTES: u64 = 25 * 1024 * 1024;

/// A reference with more pixels than this goes up scaled down to it. The
/// encoder resizes every reference to about `reference_size`² (1 Mpx) anyway,
/// so the rest of a phone photo's 24.5 Mpx was decoded and resized by the
/// server for nothing: ~2 s an edit (measured 2026-10-04, a 24.5 Mpx
/// reference against the same picture at 1 Mpx). 4 Mpx is twice the
/// encoder's side, so its own resize still starts from more than it keeps.
pub const MAX_REFERENCE_PIXELS: u64 = 2048 * 2048;

/// `bytes` scaled to at most `max_pixels`, upright, as a PNG — or `None` to
/// send it as it is: small enough already, or not a picture this can decode,
/// which the server then judges as it always did. Turned by its EXIF
/// orientation first, because the PNG carries no tag and a phone photo is
/// stored sideways; one sent as it is keeps its tag, which the server reads.
/// Dropping the tag also drops the rest of the EXIF block, location included.
pub fn fit_reference(bytes: &[u8], max_pixels: u64) -> Option<Vec<u8>> {
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    let pixels = u64::from(w) * u64::from(h);
    if pixels <= max_pixels {
        return None;
    }
    let img = crate::image::decode_upright(bytes, "the reference").ok()?;
    let scale = (max_pixels as f64 / pixels as f64).sqrt();
    let fit = |side: u32| ((f64::from(side) * scale).floor() as u32).max(1);
    // `thumbnail` keeps the aspect ratio inside the box, and is the fast
    // filter for a reduction this large.
    let small = img.thumbnail(fit(img.width()), fit(img.height()));
    png_bytes(&small.to_rgb8()).ok()
}

/// The image type of `bytes`, by magic number, as an upload extension.
pub(crate) fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

pub(crate) const PROMPT_CAP: usize = 4_000;

/// Consecutive failed status polls before a job is abandoned.
const POLL_FAILURES: u32 = 3;

/// How many polls to wait for an interrupted job to reach the server's
/// history before deleting it from there.
const FORGET_WAIT_POLLS: u32 = 10;

/// One line of an image trail (`ToolCtx::image_trail`): a job the tool is
/// about to submit, or a file the server is about to hold or has said it
/// holds. Written before the thing exists on the server, so a process that
/// dies at any point leaves a trail that names everything it left there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrailEntry {
    Job(String),
    File(String),
}

impl TrailEntry {
    fn line(&self) -> String {
        match self {
            TrailEntry::Job(id) => format!("job {id}"),
            TrailEntry::File(name) => format!("file {name}"),
        }
    }

    /// A line back, if it is one this module could have written.
    fn parse(line: &str) -> Option<Self> {
        match line.split_once(' ')? {
            ("job", id) if job_id(id) => Some(TrailEntry::Job(id.to_string())),
            ("file", name) if plain_name(name) => Some(TrailEntry::File(name.to_string())),
            _ => None,
        }
    }
}

/// Whether `name` is one plain path component — the only kind of name the
/// tool deletes in `server_temp_dir`, whoever supplied it.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0'])
        // A newline would forge a second trail line for the next sweep to
        // act on (found on review of #331).
        && !name.chars().any(char::is_control)
}

/// Whether `id` looks like a job id: what the tool mints (a UUID) or what an
/// older server answers with, never anything longer or stranger.
fn job_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Append `entry` to the trail, if the run keeps one. The file must be where
/// the caller said — it is never created in a directory that is gone, since
/// a room that has been removed is a chat that has closed.
fn note(trail: Option<&std::path::Path>, entry: &TrailEntry) -> Result<()> {
    use std::io::Write;
    let Some(path) = trail else {
        return Ok(());
    };
    // Only a line that reads back as what was meant: the sweep acts on
    // whatever parses, so a name that would forge another entry is refused
    // here rather than trusted there.
    let line = entry.line();
    if TrailEntry::parse(&line).as_ref() != Some(entry) {
        bail!(
            "refusing to write {:?} to the image trail: it would not read back as written",
            line
        );
    }
    let mut options = std::fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("opening the image trail {}", path.display()))?;
    writeln!(file, "{line}").with_context(|| format!("writing the image trail {}", path.display()))
}

/// What a trail records, skipping any line this module would not write.
pub fn read_trail(path: &std::path::Path) -> Vec<TrailEntry> {
    match std::fs::read_to_string(path) {
        Ok(text) => text.lines().filter_map(TrailEntry::parse).collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        // There and unreadable is a finding, not an empty trail: what it
        // named stays on the image server (found on review of #331).
        Err(e) => {
            tracing::warn!(
                "an image trail could not be read ({}): {e}; what it named stays on the image \
                 server until it restarts",
                path.display()
            );
            Vec::new()
        }
    }
}

/// What a job's history entry says it wrote.
#[derive(Debug, Default)]
struct Wrote {
    /// In the temp directory, by name: what the tool deletes after a job.
    temp: Vec<String>,
    /// Anywhere else — a type other than `temp`, or a subfolder — which
    /// nothing here clears, described so it is said rather than skipped: a
    /// filter that quietly matched nothing would make the whole cleanup a
    /// no-op (found on review of #331; `upload` refuses the same case).
    elsewhere: Vec<String>,
}

fn outputs(entry: &Value) -> Wrote {
    let mut wrote = Wrote::default();
    let images = entry
        .get("outputs")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|outputs| outputs.values())
        .filter_map(|out| out.get("images")?.as_array())
        .flatten();
    for image in images {
        let field = |k: &str| image.get(k).and_then(Value::as_str).unwrap_or("");
        let (name, kind, sub) = (field("filename"), field("type"), field("subfolder"));
        if kind == "temp" && sub.is_empty() {
            if !name.is_empty() {
                wrote.temp.push(name.to_string());
            }
        } else {
            let under = if sub.is_empty() {
                String::new()
            } else {
                format!(" under `{sub}`")
            };
            wrote.elsewhere.push(format!(
                "`{name}` (filed as `{kind}`{under}, where nothing here clears it)"
            ));
        }
    }
    wrote
}

/// Delete each of `names` from `dir`, and say which could not be. A name that
/// is not one plain component is never touched. `gone_is_fine` is for a
/// trail, whose writer may already have removed a file before it died; after
/// a job, a copy the server confirmed and the directory does not hold means
/// the directory is the wrong one, which is worth saying.
fn discard(dir: Option<&std::path::Path>, names: &[String], gone_is_fine: bool) -> Option<String> {
    let dir = dir?;
    let mut failed = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in names.iter().filter(|n| seen.insert(n.as_str())) {
        if !plain_name(name) {
            failed.push(format!(
                "`{name}` (not one plain file name, so not touched)"
            ));
            continue;
        }
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => {}
            Err(e) if gone_is_fine && e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => failed.push(format!("`{name}` ({e})")),
        }
    }
    (!failed.is_empty()).then(|| format!("{} in {}", failed.join(", "), dir.display()))
}

/// Take everything a trail records off the image server: each job out of the
/// queue (interrupted if it is running) and out of the history, with any
/// file it wrote, and each recorded file deleted from `server_temp_dir`. For
/// a trail whose writer is gone — an incognito room a `serve` that died left
/// behind, swept at the next one's start (`INCOGNITO-DESIGN.md` §4.2).
pub async fn forget_trail(cfg: &ImageConfig, entries: &[TrailEntry]) -> Result<()> {
    let server = ComfyUi::for_config(cfg)?;
    // Every step below is best effort, so ask first whether anyone is there:
    // otherwise a server that is down reads as a trail that was cleared.
    server
        .get_json("queue")
        .await
        .context("the image server did not answer")?;
    let mut files = Vec::new();
    let mut elsewhere = Vec::new();
    for entry in entries {
        match entry {
            // No trail to write to: the delete below follows at once.
            TrailEntry::Job(id) => {
                let wrote = server.abandon(id, None).await;
                files.extend(wrote.temp);
                elsewhere.extend(wrote.elsewhere);
            }
            TrailEntry::File(name) => files.push(name.clone()),
        }
    }
    let mut unkept = Vec::new();
    if !files.is_empty() {
        match cfg.server_temp_dir.as_deref() {
            None => unkept.push(
                "[image] server_temp_dir is not set, so the files stay in the server's temp \
                 directory until it restarts"
                    .to_string(),
            ),
            Some(dir) => {
                if let Some(left) = discard(Some(dir), &files, true) {
                    unkept.push(format!("could not remove {left}"));
                }
            }
        }
    }
    if !elsewhere.is_empty() {
        unkept.push(elsewhere.join(", "));
    }
    if unkept.is_empty() {
        Ok(())
    } else {
        bail!("{}", unkept.join("; "))
    }
}

/// The node that loads `diffusion_model`, and its inputs other than the file
/// name. A `.gguf` file goes through the ComfyUI-GGUF custom node; any other
/// (the default int8 ConvRot `.safetensors`) through ComfyUI's own
/// `UNETLoader`, which reads the file's quantisation metadata itself, so
/// `weight_dtype` stays `default`. Chosen by file name so an operator's
/// `[image] diffusion_model` alone switches formats, with nothing else to keep
/// in step; the preflight asks the server for the same node.
pub fn unet_loader(diffusion_model: &str) -> (&'static str, Value) {
    let gguf = std::path::Path::new(diffusion_model)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gguf"));
    if gguf {
        ("UnetLoaderGGUF", json!({}))
    } else {
        ("UNETLoader", json!({"weight_dtype": "default"}))
    }
}

/// ComfyUI's graph for one generation with Qwen-Image 2.1 — text to image,
/// or an edit when `uploaded` names reference images already on the server.
///
/// `PreviewImage` rather than `SaveImage`: the server writes to its temp
/// directory instead of keeping a second, permanent copy in `output/` — the
/// copy that matters is the one in the run's workspace. References are read
/// from the temp directory too (`[temp]`), which the server empties when it
/// starts, so a private photo is not left in its `input/` folder.
///
/// With references the encoder takes the VAE (it splices each reference into
/// the sequence as latents) and, unless a size was asked for, its own latent
/// output is the canvas: sized to the first reference, because sampling at
/// any other size shifts the edit (the node's own guidance).
///
/// With a `mask` (an uploaded mask's server name) the canvas is the first
/// reference itself, VAE-encoded, and the sampler redraws only where the mask
/// is white (`SetLatentNoiseMask`); the result is composited back over the
/// original in mecha's own code, so nothing outside the mask moves
/// (`IMAGE-REGION-EDIT-RESEARCH.md`, approach C).
pub fn comfy_graph(
    cfg: &ImageConfig,
    req: &Request,
    uploaded: &[String],
    mask: Option<&str>,
) -> Value {
    let mut encode = json!({
        "clip": ["clip", 0], "prompt": req.prompt, "negative_prompt": req.negative,
        "resolution": req.reference_size});
    let (loader, mut unet) = unet_loader(&cfg.diffusion_model);
    unet["unet_name"] = json!(cfg.diffusion_model);
    let mut graph = json!({
        "unet": {"class_type": loader, "inputs": unet},
        "clip": {"class_type": "CLIPLoader", "inputs": {
            "clip_name": cfg.text_encoder, "type": "qwen_image", "device": "default"}},
        "vae": {"class_type": "VAELoader", "inputs": {"vae_name": cfg.vae}},
        "decode": {"class_type": "VAEDecode", "inputs": {"samples": ["sample", 0], "vae": ["vae", 0]}},
        "out": {"class_type": "PreviewImage", "inputs": {"images": ["decode", 0]}},
    });
    for (i, name) in uploaded.iter().enumerate() {
        let node = format!("ref{}", i + 1);
        graph[&node] =
            json!({"class_type": "LoadImage", "inputs": {"image": format!("{name} [temp]")}});
        encode[format!("images.image_{}", i + 1)] = json!([node, 0]);
    }
    if !uploaded.is_empty() {
        encode["vae"] = json!(["vae", 0]);
    }
    let canvas = match (mask, uploaded.first(), req.size) {
        (Some(mask), Some(_), _) => {
            graph["maskimg"] =
                json!({"class_type": "LoadImage", "inputs": {"image": format!("{mask} [temp]")}});
            graph["mask"] = json!({"class_type": "ImageToMask", "inputs": {
                "image": ["maskimg", 0], "channel": "red"}});
            graph["source"] = json!({"class_type": "VAEEncode", "inputs": {
                "pixels": ["ref1", 0], "vae": ["vae", 0]}});
            graph["masked"] = json!({"class_type": "SetLatentNoiseMask", "inputs": {
                "samples": ["source", 0], "mask": ["mask", 0]}});
            json!(["masked", 0])
        }
        (_, _, Some((width, height))) => {
            graph["latent"] = json!({"class_type": "EmptyLatentImage", "inputs": {
                "width": width, "height": height, "batch_size": 1}});
            json!(["latent", 0])
        }
        _ => json!(["encode", 2]),
    };
    graph["encode"] = json!({"class_type": "TextEncodeQwenImage21", "inputs": encode});
    graph["sample"] = json!({"class_type": "KSampler", "inputs": {
        "model": ["unet", 0], "positive": ["encode", 0], "negative": ["encode", 1],
        "latent_image": canvas, "seed": req.seed, "steps": req.steps,
        // Guidance off: the reference setting. Above 1 doubles every step.
        "cfg": 1.0, "sampler_name": "euler", "scheduler": "simple", "denoise": 1.0}});
    graph
}

/// How much free memory a job needs before it starts, in MB, given what the
/// server already holds. Memory the server holds is already gone from the
/// available pool, so asking for a generation's whole peak again would refuse
/// a warm picture that needs little more.
///
/// `loaded` is [`loaded_from_stats`]: `Some(false)` is a server that has
/// loaded nothing since it started (the idle reset leaves it so), which pays
/// the whole cold cost, `min_mb`. `Some(true)` has loaded the model at some
/// point, but ComfyUI's stats cannot tell a server still holding it (~2 GB
/// more needed) from one that was `/free`d (its weights moved to RSS, and
/// reloading needs ~9-12 GB more, measured 2026-10-02), so it is asked for
/// the larger of the two. `None` - the stats could not be read - pays cold:
/// unknown is never warm.
pub fn memory_need_mb(loaded: Option<bool>, min_mb: u64) -> u64 {
    match loaded {
        // What a load costs is what a loaded server has already paid; the
        // rest of the operator's figure is their margin, theirs to keep
        // (review of #515).
        // Never computed down to 0: `memory_verdict` reads 0 as "check off",
        // which would also drop its refusal on an unreadable gauge. Only the
        // operator's own 0 switches the check off.
        Some(true) if min_mb == 0 => 0,
        Some(true) => min_mb.saturating_sub(LOAD_COST_MB).max(1),
        Some(false) | None => min_mb,
    }
}

/// What loading the model costs, which a server that has loaded it has
/// already paid: the default cold figure (20 GiB) less what a reload from a
/// `/free`d state still needs (~12 GiB, the worse of the two states
/// `/system_stats` cannot tell apart; measured with the Q4 GGUF on
/// 2026-10-02, not re-measured for the int8 file). **The 20_480 is
/// `ImageConfig`'s default `min_available_mb`**, written out because a
/// `Default` impl is not const: change one, change both
/// (`the_load_cost_is_the_default_less_a_reload` holds them together).
pub const LOAD_COST_MB: u64 = 20_480 - 12_288;

/// Whether ComfyUI's `/system_stats` says it has loaded a model since it
/// started. Its `torch_vram_total` is 0 in a fresh process and stays above 0
/// once anything has loaded - through a `/free`, too, so it answers "has
/// loaded", never "holds now". `None` for a reply this cannot read.
pub fn loaded_from_stats(stats: &Value) -> Option<bool> {
    let total = stats.pointer("/devices/0/torch_vram_total")?.as_f64()?;
    Some(total > 0.0)
}

/// Whether there is room to start, as a sentence for the model if not.
/// `available` is `None` when it could not be read, which is a refusal rather
/// than a pass: an unreadable gauge is not an empty tank.
pub fn memory_verdict(available_mb: Option<u64>, min_mb: u64) -> std::result::Result<(), String> {
    if min_mb == 0 {
        return Ok(());
    }
    match available_mb {
        None => Err(
            "Could not read this machine's available memory, so the generation was not \
             started. The operator can set `[image] min_available_mb = 0` to skip this check."
                .into(),
        ),
        Some(mb) if mb < min_mb => Err(format!(
            "Only {:.1} GB of memory is available and a generation needs about {:.1} GB, so it \
             was not started — the machine shares one memory pool between the GPU and \
             everything else. Try again once something large (a build, another model) is done.",
            mb as f64 / 1024.0,
            min_mb as f64 / 1024.0
        )),
        Some(_) => Ok(()),
    }
}

/// Available memory in MB, from wherever this platform reports it. `None`
/// where it cannot be read, which [`memory_verdict`] refuses on.
///
/// macOS has no `/proc`; it is also unified memory, the machine class the
/// check exists for, so it gets a reader rather than a dead tool (found on
/// review of #303 — every call refused there, and CI stayed green because
/// the tests switch the check off).
#[cfg(target_os = "macos")]
fn mem_available_mb() -> Option<u64> {
    let out = std::process::Command::new("vm_stat").output().ok()?;
    parse_vm_stat(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(target_os = "macos"))]
fn mem_available_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// `vm_stat`'s pages that can be handed to a new allocation without swapping
/// — free, inactive, speculative and purgeable — times its page size, in MB.
/// The closest macOS analogue of Linux's `MemAvailable`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_vm_stat(text: &str) -> Option<u64> {
    let page: u64 = text
        .lines()
        .next()?
        .split("page size of ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let pages = |key: &str| -> Option<u64> {
        let line = text.lines().find(|l| l.starts_with(key))?;
        line.rsplit(':')
            .next()?
            .trim()
            .trim_end_matches('.')
            .parse()
            .ok()
    };
    let total = pages("Pages free")?
        + pages("Pages inactive")?
        + pages("Pages speculative").unwrap_or(0)
        + pages("Pages purgeable").unwrap_or(0);
    Some(total * page / (1024 * 1024))
}

/// A seed nobody chose: process-random SipHash keys over the clock. Kept
/// under 2³² so it reads back exactly anywhere, JavaScript included.
fn fresh_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
    );
    h.finish() & 0xFFFF_FFFF
}

/// The ComfyUI adapter.
struct ComfyUi {
    base: reqwest::Url,
    http: reqwest::Client,
    poll: Duration,
    /// Whether the server has answered this process at least once. One that
    /// has, and now does not, is restarting and is waited for in full
    /// ([`SERVER_WAIT`]); one never seen may simply not be running, and gets
    /// only [`UNSEEN_WAIT`] - enough for a restart already under way, not a
    /// minute and a half of every picture against a stopped service.
    answered: std::sync::atomic::AtomicBool,
}

/// Why a generation did not produce an image.
enum Failure {
    Cancelled,
    Other(anyhow::Error),
}

/// How long a job waits for an image server that is not answering yet
/// (`ComfyUi::await_server`), and how often it asks meanwhile. Tests wait
/// two seconds, so the ones aimed at a closed port stay fast.
#[cfg(not(test))]
const SERVER_WAIT: Duration = Duration::from_secs(90);
#[cfg(test)]
const SERVER_WAIT: Duration = Duration::from_secs(2);
const SERVER_WAIT_STEP: Duration = Duration::from_millis(500);
/// The wait for a server this process has never had an answer from. The
/// processes that meet a restart are the unseen ones - a one-shot `mecha
/// run`, the first picture after a `serve` restart - so it covers the slowest
/// start measured (40 s once; 4-10 s usually), while a stopped service is
/// still reported in half of [`SERVER_WAIT`] (review of #515).
#[cfg(not(test))]
const UNSEEN_WAIT: Duration = Duration::from_secs(45);
#[cfg(test)]
const UNSEEN_WAIT: Duration = Duration::from_secs(1);

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Self {
        Failure::Other(e)
    }
}

/// The first stretch of a server's error body — enough to name the problem,
/// never a page of it in the model's context.
fn clip(s: &str) -> String {
    const CAP: usize = 600;
    if s.chars().count() <= CAP {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(CAP).collect::<String>())
    }
}

/// What the server holds of one job, as far as this call knows: the files it
/// named, and the ones it may have written before an answer was lost. Only a
/// confirmed name that is not there is worth saying.
#[derive(Default)]
struct Held {
    confirmed: Vec<String>,
    possible: Vec<String>,
    /// Outputs filed where nothing here clears them: said, never skipped.
    elsewhere: Vec<String>,
}

impl Held {
    fn take(&mut self, wrote: Wrote) {
        self.confirmed.extend(wrote.temp);
        self.elsewhere.extend(wrote.elsewhere);
    }
}

/// A finished call on the server: the image or why not, and — when some of
/// the server's temp copies could not be removed — which, and why.
struct Outcome {
    image: std::result::Result<Vec<u8>, Failure>,
    left: Option<String>,
}

impl ComfyUi {
    /// The client for `cfg`'s server, refusing one not on this machine — and
    /// a `server_temp_dir` that is not absolute, which would delete the
    /// server's file names relative to whatever directory this process runs
    /// in (found on review of #331; the incognito check alone came too late).
    fn for_config(cfg: &ImageConfig) -> Result<Self> {
        if let Some(dir) = &cfg.server_temp_dir {
            if !dir.is_absolute() {
                bail!(
                    "[image] server_temp_dir `{}` must be an absolute path — the tool deletes \
                     files there by the names the image server returns",
                    dir.display()
                );
            }
        }
        let mut base = loopback_url(&cfg.url)?;
        // `join` replaces the last path segment unless the base ends in `/`.
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        // The vetted address is the only one this client may reach. A
        // redirect would walk it elsewhere — a 307/308 on `/prompt` re-sends
        // the body, prompt and all — while the tool goes on declaring no
        // egress; an inherited proxy setting would route the loopback call
        // through someone else. `fetch_vetted` treats a redirect as fatal for
        // the same reason (found on review of #303).
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .context("building the image server's HTTP client")?;
        Ok(ComfyUi {
            base,
            http,
            poll: Duration::from_secs(1),
            answered: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn endpoint(&self, path: &str) -> Result<reqwest::Url> {
        Ok(self.base.join(path)?)
    }

    async fn get_json(&self, path: &str) -> Result<Value> {
        let res = self
            .http
            .get(self.endpoint(path)?)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        let status = res.status();
        let body = res.text().await?;
        if !status.is_success() {
            bail!("{path} answered {status}: {}", clip(&body));
        }
        Ok(serde_json::from_str(&body)?)
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<(reqwest::StatusCode, String)> {
        let res = self
            .http
            .post(self.endpoint(path)?)
            .timeout(Duration::from_secs(30))
            .json(body)
            .send()
            .await?;
        let status = res.status();
        Ok((status, res.text().await?))
    }

    /// Wait out a server that is restarting: the first request after one
    /// meets a closed port or a slow answer, and that is a few seconds of
    /// start, not a server that is gone. ComfyUI is restarted on purpose when
    /// it sits idle holding a model (`scripts/comfyui/comfyui-idle-reset`,
    /// measured 4-10 s, once 40 s), and by every deploy; failing the picture
    /// at the first refused connect told the model the server was down.
    /// Only a connect or timeout failure is waited on - a server that
    /// answers, even with an error, is answered - and a cancel ends the wait.
    async fn await_server(
        &self,
        cancel: Option<&CancellationToken>,
        wait: Duration,
    ) -> std::result::Result<(), Failure> {
        let wait = if self.answered.load(Ordering::Relaxed) {
            wait
        } else {
            wait.min(UNSEEN_WAIT)
        };
        let deadline = Instant::now() + wait;
        loop {
            let attempt = self
                .http
                .get(self.endpoint("queue")?)
                .timeout(Duration::from_secs(5))
                .send();
            let err = match attempt.await {
                Ok(_) => {
                    self.answered.store(true, Ordering::Relaxed);
                    return Ok(());
                }
                Err(e) if (e.is_connect() || e.is_timeout()) && Instant::now() < deadline => e,
                Err(e) => return Err(Failure::Other(e.into())),
            };
            tracing::debug!("image server not answering yet, waiting: {err}");
            let pause = tokio::time::sleep(SERVER_WAIT_STEP);
            match cancel {
                Some(token) => tokio::select! {
                    _ = token.cancelled() => return Err(Failure::Cancelled),
                    _ = pause => {}
                },
                None => pause.await,
            }
        }
    }

    /// Every file the graph names is one the server can load, checked before
    /// submitting: a missing file otherwise fails deep in a run as a node
    /// validation error the model cannot act on.
    async fn preflight(&self, cfg: &ImageConfig) -> Result<()> {
        let checks = [
            (
                unet_loader(&cfg.diffusion_model).0,
                "unet_name",
                &cfg.diffusion_model,
            ),
            ("CLIPLoader", "clip_name", &cfg.text_encoder),
            ("VAELoader", "vae_name", &cfg.vae),
        ];
        for (node, input, want) in checks {
            let info = self.get_json(&format!("object_info/{node}")).await?;
            let Some(choices) = info
                .pointer(&format!("/{node}/input/required/{input}/0"))
                .and_then(Value::as_array)
            else {
                // Only the GGUF loader comes from a custom node; the others
                // are ComfyUI's own, so a server missing them is too old.
                if node == "UnetLoaderGGUF" {
                    bail!(
                        "the image server has no `{node}` node — for GGUF models ComfyUI \
                         needs the ComfyUI-GGUF custom node"
                    );
                }
                bail!("the image server has no `{node}` node; update ComfyUI");
            };
            if !choices.iter().any(|c| c.as_str() == Some(want.as_str())) {
                let have: Vec<&str> = choices.iter().filter_map(Value::as_str).collect();
                bail!(
                    "the image server has no `{want}` for {node} (it has: {})",
                    if have.is_empty() {
                        "nothing".into()
                    } else {
                        have.join(", ")
                    }
                );
            }
        }
        let info = self.get_json("object_info/TextEncodeQwenImage21").await?;
        if info.get("TextEncodeQwenImage21").is_none() {
            bail!("the image server's ComfyUI predates Qwen-Image 2.1 support; update it");
        }
        Ok(())
    }

    /// Stop a job wherever it is: out of the queue if it has not started,
    /// interrupted if it has. Best effort — the caller is already failing.
    ///
    /// `/interrupt` is sent only when *this* job is the one running. Current
    /// ComfyUI honours the `prompt_id` in the body, but older servers ignore
    /// it and stop whatever is executing — which would let cancelling a
    /// queued image kill another call's running one. Asking the queue first
    /// makes it right on either (found on review of #303).
    async fn abandon(&self, id: &str, trail: Option<&std::path::Path>) -> Wrote {
        let _ = self.post_json("queue", &json!({"delete": [id]})).await;
        let running = self
            .get_json("queue")
            .await
            .ok()
            .and_then(|q| q.get("queue_running")?.as_array().cloned())
            .unwrap_or_default();
        // Each entry is `[number, prompt_id, prompt, extra, outputs]`.
        if running
            .iter()
            .any(|job| job.get(1).and_then(Value::as_str) == Some(id))
        {
            let _ = self.post_json("interrupt", &json!({"prompt_id": id})).await;
            // `/interrupt` answers when the flag is set, not when the job has
            // stopped; the server writes the interrupted job into its history
            // as it unwinds, after that. A delete sent now arrives before the
            // record exists and deletes nothing, leaving the prompt behind
            // (found on review of #306). So wait, briefly, for the record.
            for _ in 0..FORGET_WAIT_POLLS {
                let recorded = self
                    .get_json(&format!("history/{id}"))
                    .await
                    .ok()
                    .is_some_and(|h| h.get(id).is_some());
                if recorded {
                    break;
                }
                tokio::time::sleep(self.poll).await;
            }
        }
        // A cancelled or interrupted job is still recorded, prompt and all. A
        // job taken off the queue before it ran never reaches the history.
        // One that finished as it was stopped also wrote its preview, which
        // only the record names — so read it before forgetting it.
        let wrote = self
            .get_json(&format!("history/{id}"))
            .await
            .ok()
            .and_then(|h| h.get(id).map(outputs))
            .unwrap_or_default();
        // Onto the trail before the record naming them is forgotten, as the
        // polling path does: a death between here and the caller's delete
        // otherwise leaves a preview nothing names (found on review of #331).
        for name in &wrote.temp {
            if let Err(e) = note(trail, &TrailEntry::File(name.clone())) {
                tracing::debug!("a preview's name did not reach the image trail: {e:#}");
            }
        }
        self.forget(id).await;
        wrote
    }

    /// Put one reference in the server's temp directory as `filename` (a
    /// random name the caller has already written to the trail), and return
    /// the name the server filed it under. Multipart by hand: one file and one
    /// field do not justify a crate feature.
    async fn upload(&self, reference: &Reference, filename: &str) -> Result<String> {
        let boundary = format!("mecha-{:08x}{:08x}", fresh_seed(), fresh_seed());
        let mut body = Vec::with_capacity(reference.bytes.len() + 512);
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; \
                 filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&reference.bytes);
        body.extend_from_slice(
            format!(
                "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"type\"\r\n\r\n\
                 temp\r\n--{boundary}--\r\n"
            )
            .as_bytes(),
        );
        let res = self
            .http
            .post(self.endpoint("upload/image")?)
            .timeout(Duration::from_secs(60))
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await?;
        let status = res.status();
        let text = res.text().await?;
        if !status.is_success() {
            bail!(
                "uploading {} failed ({status}): {}",
                reference.path,
                clip(&text)
            );
        }
        let answer: Value = serde_json::from_str(&text)?;
        // The server says which directory it used. Only temp is emptied when
        // it starts; anywhere else, a private photo would outlive this call
        // with nothing to clear it (found on review of #306).
        let kind = answer.get("type").and_then(Value::as_str).unwrap_or("");
        if kind != "temp" {
            bail!(
                "the image server filed {} as `{kind}` rather than temp, where nothing clears it",
                reference.path
            );
        }
        if answer
            .get("subfolder")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
        {
            bail!(
                "the image server filed {} under a subfolder",
                reference.path
            );
        }
        answer
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                anyhow!(
                    "the image server did not name the upload of {}",
                    reference.path
                )
            })
    }

    /// One job, and then its temp files deleted however it ended.
    async fn generate(
        &self,
        cfg: &ImageConfig,
        req: &Request,
        cancel: Option<&CancellationToken>,
        timeout: Duration,
        trail: Option<&std::path::Path>,
    ) -> Outcome {
        let mut held = Held::default();
        let image = self.run(cfg, req, cancel, timeout, trail, &mut held).await;
        let dir = cfg.server_temp_dir.as_deref();
        let mut left: Vec<String> = [
            discard(dir, &held.confirmed, false),
            discard(dir, &held.possible, true),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !held.elsewhere.is_empty() {
            left.push(held.elsewhere.join(", "));
        }
        Outcome {
            image,
            left: (!left.is_empty()).then(|| left.join("; ")),
        }
    }

    async fn run(
        &self,
        cfg: &ImageConfig,
        req: &Request,
        cancel: Option<&CancellationToken>,
        timeout: Duration,
        trail: Option<&std::path::Path>,
        held: &mut Held,
    ) -> std::result::Result<Vec<u8>, Failure> {
        // A trail that cannot be written stops the job before the server has
        // anything of it: the trail is what cleans up after a process that
        // dies, and a protection that cannot run must stop the run.
        let record = |entry: TrailEntry| {
            note(trail, &entry).map_err(|e| {
                Failure::Other(e.context("the image job could not be recorded for cleanup"))
            })
        };
        self.await_server(cancel, SERVER_WAIT).await?;
        self.preflight(cfg).await?;
        // `comfy_graph`'s masked arm needs the picture it masks; without one
        // the mask would drop out of the graph and the whole frame be redrawn.
        // `request` refuses the pair, and this stops it here too, since
        // `Request` is public (review of #429).
        if req.mask.is_some() && req.references.is_empty() {
            return Err(Failure::Other(anyhow!(
                "a mask needs the picture it masks; nothing was drawn"
            )));
        }
        let mut uploaded = Vec::with_capacity(req.references.len() + 1);
        for reference in req.references.iter().chain(req.mask.iter()) {
            let asked = format!(
                "mecha-{:08x}{:08x}.{}",
                fresh_seed(),
                fresh_seed(),
                reference.ext
            );
            record(TrailEntry::File(asked.clone()))?;
            let filed = match self.upload(reference, &asked).await {
                Ok(filed) => filed,
                Err(e) => {
                    // The server may have written it before the answer was
                    // lost (found on review of #331).
                    held.possible.push(asked);
                    return Err(e.into());
                }
            };
            held.confirmed.push(filed.clone());
            if filed != asked {
                record(TrailEntry::File(filed.clone()))?;
            }
            uploaded.push(filed);
        }
        // The id is minted here and written down before the server sees the
        // job; ComfyUI takes a client's id in canonical UUID form. An older
        // server mints its own, which is written down as soon as it answers.
        let asked = uuid::Uuid::new_v4().to_string();
        record(TrailEntry::Job(asked.clone()))?;
        // The mask is uploaded last, and is no reference.
        let (refs, mask) = match req.mask {
            Some(_) => (&uploaded[..uploaded.len() - 1], uploaded.last()),
            None => (&uploaded[..], None),
        };
        let graph = comfy_graph(cfg, req, refs, mask.map(String::as_str));
        let submitted = self
            .post_json(
                "prompt",
                &json!({"prompt": graph, "client_id": "mecha", "prompt_id": asked}),
            )
            .await;
        let (status, body) = match submitted {
            Ok(answer) => answer,
            Err(e) => {
                // Queued under the id sent, perhaps, before the answer was
                // lost — and that id is in hand (found on review of #331).
                held.take(self.abandon(&asked, trail).await);
                return Err(e.into());
            }
        };
        if !status.is_success() {
            // Refused, so never queued: nothing to take back.
            return Err(anyhow!(
                "the image server rejected the job ({status}): {}",
                clip(&body)
            )
            .into());
        }
        let Some(id) = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("prompt_id")?.as_str().map(str::to_string))
        else {
            // Accepted, so queued — under the id sent.
            held.take(self.abandon(&asked, trail).await);
            return Err(anyhow!("the image server accepted the job but named no prompt_id").into());
        };
        if id != asked {
            if let Err(e) = record(TrailEntry::Job(id.clone())) {
                held.take(self.abandon(&id, trail).await);
                return Err(e);
            }
        }

        let started = Instant::now();
        let mut failed_polls = 0u32;
        let image = loop {
            let tick = tokio::time::sleep(self.poll);
            match cancel {
                Some(token) => tokio::select! {
                    _ = token.cancelled() => {
                        held.take(self.abandon(&id, trail).await);
                        return Err(Failure::Cancelled);
                    }
                    _ = tick => {}
                },
                None => tick.await,
            }
            if started.elapsed() > timeout {
                held.take(self.abandon(&id, trail).await);
                return Err(anyhow!(
                    "the image took longer than {} s and was abandoned",
                    timeout.as_secs()
                )
                .into());
            }
            // A failed poll is not a failed job: the server may be slow to
            // answer while it loads the models. Keep polling through a brief
            // outage; after several in a row, take the job off the server
            // before giving up — returning with it still queued would leave a
            // generation holding the GPU with no client to reap it (found on
            // review of #303).
            let history = match self.get_json(&format!("history/{id}")).await {
                Ok(history) => {
                    failed_polls = 0;
                    history
                }
                Err(e) => {
                    failed_polls += 1;
                    if failed_polls < POLL_FAILURES {
                        continue;
                    }
                    held.take(self.abandon(&id, trail).await);
                    return Err(e
                        .context(format!(
                            "the image server stopped answering ({POLL_FAILURES} polls in a row)"
                        ))
                        .into());
                }
            };
            let Some(entry) = history.get(&id) else {
                continue; // queued or running
            };
            // Everything the job wrote goes onto the trail and the list to
            // delete *before* the record naming it is forgotten — a job that
            // failed after its preview was written included (found on review
            // of #331: forgotten first, a death here left files nothing named).
            let wrote = outputs(entry);
            for name in &wrote.temp {
                if let Err(e) = note(trail, &TrailEntry::File(name.clone())) {
                    // No job left to stop by now; the usual cause is a room
                    // that has just closed. Said at debug, which is not a
                    // count at the default level (R1).
                    tracing::debug!("a preview's name did not reach the image trail: {e:#}");
                }
            }
            held.take(wrote);
            if entry.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
                self.forget(&id).await;
                let why = entry
                    .pointer("/status/messages")
                    .map(|m| clip(&m.to_string()))
                    .unwrap_or_default();
                return Err(anyhow!("the image server failed the job: {why}").into());
            }
            let found = entry
                .get("outputs")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|o| o.values())
                .filter_map(|out| out.get("images")?.as_array()?.first().cloned())
                .next();
            match found {
                Some(image) => break image,
                // Recorded without an image: finished with nothing to show.
                None if entry.pointer("/status/completed").and_then(Value::as_bool)
                    == Some(true) =>
                {
                    self.forget(&id).await;
                    return Err(
                        anyhow!("the image server finished the job without an image").into(),
                    );
                }
                None => continue,
            }
        };

        // Forgotten whether or not the fetch works: the server keeps every
        // job's prompt and file names in memory until it restarts, and the
        // copy that matters is the one in the workspace (or none at all).
        let fetched = self.fetch(&image).await;
        self.forget(&id).await;
        Ok(fetched?)
    }

    /// The finished image's bytes, checked to be a PNG.
    async fn fetch(&self, image: &Value) -> Result<Vec<u8>> {
        let field = |k: &str| {
            image
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let mut url = self.endpoint("view")?;
        url.query_pairs_mut()
            .append_pair("filename", &field("filename"))
            .append_pair("subfolder", &field("subfolder"))
            .append_pair("type", &field("type"));
        let res = self
            .http
            .get(url)
            .timeout(Duration::from_secs(60))
            .send()
            .await?;
        if !res.status().is_success() {
            bail!("fetching the finished image failed ({})", res.status());
        }
        let bytes = res.bytes().await?.to_vec();
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            bail!("the image server returned something that is not a PNG");
        }
        Ok(bytes)
    }

    /// Drop a job from the server's history. Best effort.
    async fn forget(&self, id: &str) {
        let _ = self.post_json("history", &json!({"delete": [id]})).await;
    }

    /// Whether the server has loaded a model since it started
    /// ([`loaded_from_stats`]); `None` if it cannot say - including a server
    /// that is restarting, which is then asked for the cold cost.
    async fn loaded(&self) -> Option<bool> {
        let res = self
            .http
            .get(self.endpoint("system_stats").ok()?)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?;
        // Any answer, even an error, is a server that is up.
        self.answered.store(true, Ordering::Relaxed);
        if !res.status().is_success() {
            return None;
        }
        let stats: Value = res.json().await.ok()?;
        loaded_from_stats(&stats)
    }

    /// Ask the server to drop its models and free the memory they held.
    async fn release(&self) {
        let _ = self
            .post_json("free", &json!({"unload_models": true, "free_memory": true}))
            .await;
    }
}

/// Write `bytes` to a new file under `images/` in the run's workspace and
/// return the workspace-relative path. Never overwrites: the name is new or
/// the write fails, so a generation cannot replace an earlier one or follow a
/// link planted where its file will go.
async fn save(ctx: &ToolCtx, stamp: &str, seed: u64, bytes: &[u8]) -> Result<String> {
    use tokio::io::AsyncWriteExt;
    for n in 0..100u32 {
        let name = if n == 0 {
            format!("images/{stamp}-{seed}.png")
        } else {
            format!("images/{stamp}-{seed}-{n}.png")
        };
        let path = ctx.resolve(&name)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        match options.open(&path).await {
            Ok(mut file) => {
                file.write_all(bytes).await?;
                file.flush().await?;
                return Ok(name);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(anyhow!("cannot write {name}: {e}")),
        }
    }
    bail!("no free file name under images/ for this second")
}

/// Read each reference out of the run's workspace, through the path jail.
///
/// The pixels never enter the conversation, so nothing here arms taint; the
/// result names paths only. `image_generate` sends them to the loopback
/// server; `image_library_propose` keeps one as a candidate's portrait in the
/// owner's library, bounded by `imagelib::MAX_PROPOSED_PORTRAIT_BYTES`.
pub(crate) async fn read_references(
    ctx: &ToolCtx,
    paths: &[String],
) -> std::result::Result<Vec<Reference>, String> {
    use tokio::io::AsyncReadExt;
    let mut out = Vec::with_capacity(paths.len());
    for raw in paths {
        let path = ctx
            .resolve(raw)
            .map_err(|e| format!("`{raw}` is not a file in the workspace: {e:#}"))?;
        let mut options = tokio::fs::OpenOptions::new();
        options.read(true);
        // A workspace can hold a FIFO (`shell: mkfifo`), and opening one
        // waits for a writer forever — before the `is_file` refusal below can
        // run. Open without waiting, then refuse anything that is not a
        // regular file, as `read_file_window` does (found on review of #306).
        #[cfg(unix)]
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
        let file = options
            .open(&path)
            .await
            .map_err(|e| format!("cannot open `{raw}`: {e}"))?;
        let meta = file
            .metadata()
            .await
            .map_err(|e| format!("cannot read `{raw}`: {e}"))?;
        if !meta.is_file() {
            return Err(format!("`{raw}` is not a file."));
        }
        if meta.len() > MAX_REFERENCE_BYTES {
            return Err(format!(
                "`{raw}` is {} MB; references are capped at {} MB.",
                meta.len() / (1024 * 1024),
                MAX_REFERENCE_BYTES / (1024 * 1024)
            ));
        }
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        file.take(MAX_REFERENCE_BYTES)
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| format!("cannot read `{raw}`: {e}"))?;
        let ext = sniff_image(&bytes)
            .ok_or_else(|| format!("`{raw}` is not a PNG, JPEG or WebP image."))?;
        out.push(Reference {
            path: raw.clone(),
            bytes,
            ext,
        });
    }
    Ok(out)
}

/// An edit whose result's layout is at least this alike its first
/// reference's came back a near-copy. Measured on 2026-09-29 with
/// [`layout_similarity`] over some 70 edits of one scene, each also judged by
/// eye: every copy, and every edit that left her sitting, scored 0.80 or
/// more but one kneeling partial (0.754); every edit that stood her up, 0.761
/// or less — the top of that range a small standing figure beside a man who
/// fills the frame, which 0.75 flagged and a live run then retried for
/// nothing. Edits of colour or a small detail keep the layout on purpose and
/// scored 0.78 to 1.00, so the notice says "the layout did not change",
/// never "the edit failed": only the model knows which it asked for.
pub const NEAR_COPY_LAYOUT: f64 = 0.78;

/// The path a face anchor's crop is listed under in `Request::references`:
/// never a workspace file, and left out of what the result says was edited.
const FACE_REFERENCE: &str = "library:face:";

/// What an edit with a head crop says about its canvas (IMAGE-SCENE-DESIGN.md
/// §4). Keeping the camera is what lets a crop ride along without dragging the
/// framing to its own close-up (2026-10-05, chain B: without it the anchored
/// edits zoomed in); the people's sentences follow it.
const CANVAS_KEEPS_CAMERA: &str = " <image1> is the canvas: keep its camera position, angle, \
     framing and composition.";

/// The same for an edit that moves the camera (`edit.camera`): the crop rides
/// along anyway, and the canvas keeps only its room. Measured as M2's R, which
/// moved the camera on the low-angle step with the crop on (2026-10-07).
const CANVAS_CAMERA_MOVES: &str = " <image1> is the canvas: keep its room and furniture; the \
     camera may move as described.";

/// How many references fit one edit, the picture being edited included
/// (IMAGE-DESIGN.md §6, C5, provisional): the canvas and a head crop for each
/// of up to five faces. Every reference in an unmasked edit is encoded at
/// [`UNMASKED_EDIT_REFERENCE_SIZE`] (a masked one at [`EDIT_REFERENCE_SIZE`])
/// whatever its own size. Three at 1024² was the measured fast shape (66 s);
/// six is the owner's provisional ceiling, slower, and the planner refuses a
/// sixth face rather than drop one.
const EDIT_REFERENCE_BUDGET: usize = 1 + crate::picture::MAX_FACES;

/// A library name as the edit prompt says it, each word capitalised: "maya" →
/// "Maya", "mara quinn" → "Mara Quinn" — the name is what binds a face to a
/// person in the prompt (review of #586).
pub(crate) fn capitalized(name: &str) -> String {
    name.split(' ')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// How alike two pictures' layouts are, from -1 to 1: the correlation of
/// their 32×32 grayscale thumbnails, so light and dark in the same places
/// score high whatever the colours. `None` when either does not decode or is
/// one flat tone — no reading, not a low one.
pub fn layout_similarity(a: &[u8], b: &[u8]) -> Option<f64> {
    fn thumb(bytes: &[u8]) -> Option<Vec<f64>> {
        // Upright, as the server and the page see it: a phone photo is
        // stored sideways with a tag, and the result never is.
        let picture = crate::image::decode_upright(bytes, "the picture").ok()?;
        let small = image::imageops::resize(
            &picture.to_luma8(),
            32,
            32,
            image::imageops::FilterType::Triangle,
        );
        Some(small.pixels().map(|p| f64::from(p.0[0])).collect())
    }
    let (x, y) = (thumb(a)?, thumb(b)?);
    let n = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let (mut cov, mut vx, mut vy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(&y) {
        cov += (a - mx) * (b - my);
        vx += (a - mx) * (a - mx);
        vy += (b - my) * (b - my);
    }
    (vx > 0.0 && vy > 0.0).then(|| cov / (vx * vy).sqrt())
}

/// The canvas an edit samples on for a picture `w`×`h` at `resolution`: the
/// encoder's own sizing (`TextEncodeQwenImage21`, about `resolution`² pixels,
/// aspect kept, multiples of 32, Python's round-half-even). A masked edit
/// resizes the picture to exactly this before it is uploaded, so the encoded
/// canvas, the reference the encoder sees and the composite all line up.
pub fn edit_canvas(w: u32, h: u32, resolution: u32) -> (u32, u32) {
    fn round_half_even(x: f64) -> f64 {
        let r = x.round();
        if (x - x.trunc()).abs() == 0.5 {
            2.0 * (x / 2.0).round()
        } else {
            r
        }
    }
    let (res, ratio) = (f64::from(resolution), f64::from(w) / f64::from(h));
    let side = |v: f64| ((round_half_even(v / 32.0) * 32.0) as u32).max(32);
    (
        side((res * res * ratio).sqrt()),
        side((res * res / ratio).sqrt()),
    )
}

/// Grow the painted area by about 21 px and feather its edge by about 16,
/// at the canvas's scale — the setting measured seamless
/// (`IMAGE-REGION-EDIT-RESEARCH.md` §4).
const MASK_GROW_SIGMA: f32 = 12.0;
const MASK_FEATHER_SIGMA: f32 = 8.0;

/// What a masked edit is built from: the picture at canvas size, the
/// softened mask for the composite, and the painted area's bounds, which is
/// where a near-copy is looked for.
#[derive(Debug, Clone)]
pub struct MaskPlan {
    /// The picture at its edit canvas: the original itself, not a resample,
    /// when the two sizes agree — as they do for every picture this tool made.
    pub picture: image::RgbImage,
    /// The picture's own size, before any resize to the canvas.
    pub source: (u32, u32),
    pub soft: image::GrayImage,
    pub bounds: (u32, u32, u32, u32),
}

/// Size the picture to its edit canvas and turn the owner's painted mask into
/// the one the sampler and the composite use. Refused, with a sentence for
/// the model, when the mask marks nothing, or its shape differs from the
/// picture's by more than 2% — the one check a file can bear out; that it was
/// painted over *this* picture is the page's pairing of the two names.
pub fn prepare_mask(
    picture: &[u8],
    mask: &[u8],
    resolution: u32,
) -> std::result::Result<MaskPlan, String> {
    use image::imageops::{fast_blur, resize, FilterType};
    // Upright, as the page painted over it: a phone photo stored sideways
    // with a tag would otherwise read as the other shape from its mask.
    let picture =
        crate::image::decode_upright(picture, "the picture").map_err(|e| format!("{e:#}"))?;
    let mask = crate::image::decode(mask, "the mask").map_err(|e| format!("{e:#}"))?;
    let (pw, ph) = (picture.width(), picture.height());
    let (mw, mh) = (mask.width(), mask.height());
    let (pr, mr) = (f64::from(pw) / f64::from(ph), f64::from(mw) / f64::from(mh));
    if (pr / mr - 1.0).abs() > 0.02 {
        return Err(format!(
            "The mask is {mw}×{mh} but the picture is {pw}×{ph}: a mask has to be painted \
             over the picture it edits."
        ));
    }
    let (cw, ch) = edit_canvas(pw, ph, resolution);
    // Resampled only when the sizes differ. (The `image` crate's `resize`
    // already returns an exact copy at the same size; the skip says so here
    // rather than leaning on that.)
    let picture = if (cw, ch) == (pw, ph) {
        picture.to_rgb8()
    } else {
        resize(&picture.to_rgb8(), cw, ch, FilterType::Lanczos3)
    };
    let hard = image::GrayImage::from_fn(cw, ch, {
        let luma = resize(&mask.to_luma8(), cw, ch, FilterType::Triangle);
        // Any real coverage counts: a thin stroke on a large photo is
        // averaged down by the resize, and 127 would erase it before the
        // owner is told it is too small (review of #429).
        move |x, y| {
            image::Luma([if luma.get_pixel(x, y).0[0] > 16 {
                255
            } else {
                0
            }])
        }
    });
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for (x, y, p) in hard.enumerate_pixels() {
        if p.0[0] > 0 {
            let b = bounds.get_or_insert((x, y, x + 1, y + 1));
            *b = (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1));
        }
    }
    let Some(bounds) = bounds else {
        return Err(
            "The mask marks nothing: its painted (white) area is empty. Ask the user to \
             paint the part to change."
                .into(),
        );
    };
    // The grow is a blur thresholded low: with σ = 12, "> 10" reaches about
    // 21 px past the painted edge. The threshold moves with the sigma; change
    // them together. (The 16 in `hard` above is coverage after a resize, not
    // this.)
    let grown = fast_blur(&hard, MASK_GROW_SIGMA);
    let grown = image::GrayImage::from_fn(cw, ch, |x, y| {
        image::Luma([if grown.get_pixel(x, y).0[0] > 10 {
            255
        } else {
            0
        }])
    });
    // A dab too small to survive the grow step would leave the sampler
    // nothing to redraw and the composite nothing to change, while the result
    // said the painted area was redrawn: refused, like an empty mask (review
    // of #429).
    if !grown.pixels().any(|p| p.0[0] > 0) {
        return Err(
            "The painted area is too small to edit: ask the user to paint over more of \
             what should change."
                .into(),
        );
    }
    let soft = fast_blur(&grown, MASK_FEATHER_SIGMA);
    // Painted pixels stay fully redrawn, whatever the blur did to them.
    let soft = image::GrayImage::from_fn(cw, ch, |x, y| {
        image::Luma([soft.get_pixel(x, y).0[0].max(hard.get_pixel(x, y).0[0])])
    });
    Ok(MaskPlan {
        picture,
        source: (pw, ph),
        soft,
        bounds,
    })
}

/// A PNG of `image`, for an upload or the saved result.
fn png_bytes<P, C>(image: &image::ImageBuffer<P, C>) -> std::result::Result<Vec<u8>, String>
where
    P: image::Pixel + image::PixelWithColorType,
    [P::Subpixel]: image::EncodableLayout,
    C: std::ops::Deref<Target = [P::Subpixel]>,
{
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| format!("could not encode a PNG: {e}"))?;
    Ok(out.into_inner())
}

/// Lay the server's result over the original through the soft mask: every
/// pixel the owner did not paint (outside the grown, feathered edge) comes
/// back exactly as it was. The result is sized to the canvas first, should
/// the server have answered at another size.
pub fn composite_masked(result: &[u8], plan: &MaskPlan) -> std::result::Result<Vec<u8>, String> {
    let (w, h) = plan.picture.dimensions();
    let result = crate::image::decode(result, "the result").map_err(|e| format!("{e:#}"))?;
    let result = if (result.width(), result.height()) == (w, h) {
        result.to_rgb8()
    } else {
        image::imageops::resize(
            &result.to_rgb8(),
            w,
            h,
            image::imageops::FilterType::Lanczos3,
        )
    };
    let out = image::RgbImage::from_fn(w, h, |x, y| {
        let m = u16::from(plan.soft.get_pixel(x, y).0[0]);
        let (o, r) = (plan.picture.get_pixel(x, y).0, result.get_pixel(x, y).0);
        image::Rgb(std::array::from_fn(|c| {
            ((u16::from(o[c]) * (255 - m) + u16::from(r[c]) * m + 127) / 255) as u8
        }))
    });
    png_bytes(&out)
}

/// [`layout_similarity`] over what was painted only: both pictures and the
/// soft mask are cropped to the painted bounds and reduced to the same
/// 32×32 thumbnail grid, and only the cells the mask mostly covers are
/// correlated. The bounds alone would not do: the composite puts the
/// original back over most of a stroke's bounding box, and those identical
/// cells would call a masked edit that worked a near-copy (review of #429).
/// `None` when fewer than 16 cells are painted enough to read, which is no
/// reading, not a low one.
pub fn layout_similarity_painted(
    a: &[u8],
    b: &[u8],
    soft: &image::GrayImage,
    bounds: (u32, u32, u32, u32),
) -> Option<f64> {
    use image::imageops::{resize, FilterType};
    let (x0, y0, x1, y1) = bounds;
    let (x1, y1) = (x1.min(soft.width()), y1.min(soft.height()));
    (x1 > x0 && y1 > y0).then_some(())?;
    let (w, h) = (x1 - x0, y1 - y0);
    let thumb = |bytes: &[u8]| -> Option<Vec<f64>> {
        // Upright, as the server and the page see it: a phone photo is
        // stored sideways with a tag, and the result never is.
        let picture = crate::image::decode_upright(bytes, "the picture").ok()?;
        if (picture.width(), picture.height()) != soft.dimensions() {
            return None;
        }
        let cell = resize(
            &picture.crop_imm(x0, y0, w, h).to_luma8(),
            32,
            32,
            FilterType::Triangle,
        );
        Some(cell.pixels().map(|p| f64::from(p.0[0])).collect())
    };
    let cover = resize(
        &image::imageops::crop_imm(soft, x0, y0, w, h).to_image(),
        32,
        32,
        FilterType::Triangle,
    );
    let (ta, tb) = (thumb(a)?, thumb(b)?);
    let (x, y): (Vec<f64>, Vec<f64>) = cover
        .pixels()
        .zip(ta.iter().zip(&tb))
        .filter(|(c, _)| c.0[0] >= 200)
        .map(|(_, (a, b))| (*a, *b))
        .unzip();
    if x.len() < 16 {
        return None;
    }
    let n = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let (mut cov, mut vx, mut vy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(&y) {
        cov += (a - mx) * (b - my);
        vx += (a - mx) * (a - mx);
        vy += (b - my) * (b - my);
    }
    (vx > 0.0 && vy > 0.0).then(|| cov / (vx * vy).sqrt())
}

/// Cheap to clone: every piece of state a call shares with the next is
/// behind an `Arc`, so a clone is the same tool — which is how a deferred
/// render owns what it uses while it outlives the call that started it
/// (`docs/BACKGROUND-JOBS-DESIGN.md` §2.1).
#[derive(Clone)]
pub struct ImageGenerate {
    cfg: ImageConfig,
    backend: Arc<ComfyUi>,
    /// Bumped per finished generation; an idle-unload timer fires only if it
    /// still holds the value it was armed with.
    generation: Arc<AtomicU64>,
    /// The image library a `cast` or `style` resolves against, read afresh on
    /// every call so an entry the owner just approved is usable at once.
    /// `None` when the mecha home cannot be resolved.
    library_dir: Option<std::path::PathBuf>,
    /// Who "self" is in this persona's chat (`for_persona_as`, §8.6): its
    /// names and the library character it looks like. `None` outside a
    /// persona chat.
    self_as: Option<crate::tool::PersonaSelf>,
    /// Whether a new picture's `seed` is in the schema: the CLI and evals
    /// only ([`ImageGenerate::with_seeds`]). A chat never has it — 177 of
    /// 211 model-sent seeds copied an earlier result's (IMAGE-DESIGN.md §5.1).
    seeds: bool,
    /// A reference with more pixels than this is fitted before upload:
    /// [`MAX_REFERENCE_PIXELS`], lowered only by tests, which cannot afford
    /// a 4 Mpx picture in an unoptimised build.
    reference_pixels: u64,
    /// Where a persona edit's face anchor comes from ([`crate::face`]): the
    /// cached crop of the character's portrait, else the detector on it.
    /// Stood in for by tests, which have no 88 MB of weights.
    faces: Arc<dyn crate::face::FaceAnchors>,
    /// [`DESCRIPTION`] with the library's style names after it, read once
    /// when this form is built and never per request, so the tool list a
    /// chat caches stays byte-stable. A new style reaches a running serve's
    /// forms when they are next built: a restart, or a persona's agent rebuilt
    /// on an edit or a rebinding (serve caches it per persona version).
    description: String,
}

/// The description a form is built with: [`DESCRIPTION`], then the styles
/// to name at the very end, so a new style moves no earlier byte.
fn described(library_dir: Option<&std::path::Path>) -> String {
    let styles = library_dir
        .map(|d| crate::imagelib::Library::load(d).0)
        .as_ref()
        .and_then(crate::imagelib::styles_to_name);
    match styles {
        Some(styles) => format!("{DESCRIPTION} {styles}."),
        None => DESCRIPTION.to_string(),
    }
}

impl ImageGenerate {
    /// Whether this machine has the memory a generation needs, asked before
    /// any reference is read: reading them is itself a cost on that pool.
    async fn memory_guard(&self) -> std::result::Result<(), String> {
        let need = if self.cfg.min_available_mb == 0 {
            0
        } else {
            memory_need_mb(self.backend.loaded().await, self.cfg.min_available_mb)
        };
        memory_verdict(mem_available_mb(), need)
    }

    /// Refuses a configuration whose server is not on this machine.
    pub fn new(cfg: ImageConfig) -> Result<Self> {
        let library_dir = crate::imagelib::Library::default_dir().ok();
        Ok(ImageGenerate {
            backend: Arc::new(ComfyUi::for_config(&cfg)?),
            cfg,
            generation: Arc::new(AtomicU64::new(0)),
            description: described(library_dir.as_deref()),
            library_dir,
            self_as: None,
            seeds: false,
            reference_pixels: MAX_REFERENCE_PIXELS,
            faces: Arc::new(crate::face::CachedRetinaFace),
        })
    }

    /// Resolve `cast` and `style` against this library instead of the one in
    /// the mecha home.
    pub fn with_library_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.description = described(Some(&dir));
        self.library_dir = Some(dir);
        self
    }

    /// The CLI and eval form: a new picture may name its `seed`, so a run
    /// can be reproduced. Never a chat's (IMAGE-DESIGN.md §5.1).
    pub fn with_seeds(mut self) -> Self {
        self.seeds = true;
        self
    }

    #[cfg(test)]
    fn with_reference_pixels(mut self, pixels: u64) -> Self {
        self.reference_pixels = pixels;
        self
    }

    #[cfg(test)]
    fn with_faces(mut self, faces: Arc<dyn crate::face::FaceAnchors>) -> Self {
        self.faces = faces;
        self
    }

    /// The form a persona chat gets: strict about unknown cast names, and
    /// knowing who "self" is when `who` says.
    fn persona_form(&self, who: Option<crate::tool::PersonaSelf>) -> ImageGenerate {
        ImageGenerate {
            cfg: self.cfg.clone(),
            backend: Arc::clone(&self.backend),
            generation: Arc::clone(&self.generation),
            library_dir: self.library_dir.clone(),
            self_as: who,
            seeds: false,
            reference_pixels: self.reference_pixels,
            faces: Arc::clone(&self.faces),
            // Read afresh whenever a persona's agent is built (serve caches
            // one per persona version and binding, not per chat).
            description: described(self.library_dir.as_deref()),
        }
    }

    #[cfg(test)]
    fn polling_every(mut self, poll: Duration) -> Self {
        Arc::get_mut(&mut self.backend)
            .expect("unshared in tests")
            .poll = poll;
        self
    }

    fn arm_unload(&self) {
        let after = self.cfg.unload_after_secs;
        if after == 0 {
            return;
        }
        let armed = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = Arc::clone(&self.generation);
        let backend = Arc::clone(&self.backend);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(after)).await;
            if generation.load(Ordering::SeqCst) == armed {
                backend.release().await;
            }
        });
    }
}

/// What the model is told the tool does (`IMAGE-DESIGN.md` §5.1). A named
/// constant, so the acceptance gates read the same words the model does.
pub const DESCRIPTION: &str = "Draw a picture with the local image model, or change one, and save \
     it as a PNG in the workspace. Takes about a minute. Describe the picture as a `scene`; the \
     tool writes the image model's prompt. For a new picture, give the whole scene: `setting` \
     (everything but the people and the words: the place and its objects, or the whole subject \
     of a picture without people), `light` (light, mood, time of day), `camera` (shot size, \
     angle, framing), `style` (a library style's name, only when the user asks for a look), \
     `people`, `together` (what people do \
     with each other) and `text` (words to render exactly). Each person has `who` (a library \
     character's name, \"self\" for you, or a description of someone not in the library), \
     `where` (left, centre, right or background), `wearing`, `doing` and `expression`. To \
     change a picture, name it in `picture` and send only what changes in `scene`: a new pose \
     or camera redraws the scene, new clothes, an expression or someone added edit the \
     picture, and restating what is already true changes nothing (a call that changes \
     nothing draws the scene again). Take someone out with `remove`. One small change to the \
     picture itself (an object, a colour, a detail) goes in `retouch`, in words: never \
     clothes, a pose, an expression or a person, which are `scene.people`. With a painted \
     area, also pass its `mask`, and never make one up. When the user attaches a photo of a \
     place and wants someone in it, that photo is the setting: `scene.setting` is \
     {\"photo\": <its path>}, not the room described in words; a photo attached earlier in \
     the chat is still a setting you can name by its path. Nobody is drawn twice, and at most \
     five people with faces fit one picture. The library supplies how its characters look \
     (their portraits), so do not describe their faces. The image model renders \
     text well: put the exact words in `scene.text`. The first line of a result is the new \
     picture's file path: name it in `picture` to change that picture, and leave the path out \
     of replies, since the owner is shown the picture. In a chat the result can come at once as \
     `being made: <path>`: the picture is still being drawn, reaches the owner's screen when it \
     is done, and you are told then; it is not a failure, so do not draw it again, and change \
     it only after you are told it is done. A line under it beginning `Queued:` means it waits \
     behind other pictures and starts when they are done. Results are not shown to you, so never say what a \
     picture shows. A change always makes a new file and leaves the original as it was: never \
     write or copy a result over the picture it was edited from. If image_view is among your \
     tools, look at a picture only when the task needs you to see it (the user asked you to \
     check, compare or describe it), not to confirm that it worked.";

/// `scene.setting`'s description in the schema.
pub const SETTING_DESC: &str = "Everything in the picture except its people and its words: the \
     place and its objects, or the whole subject of a picture without people. Words, or \
     {\"photo\": <path>} to put the people in that room.";
/// `scene.people`'s description.
pub const PEOPLE_DESC: &str = "Who is in the picture, each once. For a change, only the people \
     who change, with only what changes: for new clothes alone send only `wearing`, since a \
     `doing` you send, even reworded, is a new pose and redraws the scene. Someone new needs \
     `wearing`, and `doing` unless `together` says what they do.";
/// `retouch`'s description.
pub const RETOUCH_DESC: &str = "One small change to the picture itself, in words: an object, a \
     colour, a detail (\"Give the man a red umbrella.\"). Never clothes, a pose, an expression \
     or a person: those are `scene.people`.";
/// `picture`'s description.
pub const PICTURE_DESC: &str = "The picture being changed: one the user attached (inbox/...) or \
     an earlier result (images/...). Leave it out for a new picture.";

impl ImageGenerate {
    /// The persona's names, which `who` resolves to its approved character.
    fn self_names(&self, lib: &crate::imagelib::Library) -> crate::picture::SelfNames {
        let Some(who) = &self.self_as else {
            return crate::picture::SelfNames::default();
        };
        let character = who
            .character
            .as_deref()
            .map(|c| c.trim().to_lowercase())
            .filter(|c| {
                lib.get(crate::imagelib::Kind::Character, c)
                    .is_some_and(|e| e.status == crate::imagelib::Status::Approved)
            });
        crate::picture::SelfNames {
            character,
            names: vec![who.name.clone(), who.display.clone()],
        }
    }
}

/// The prose a recorded scene carries into a prompt: its setting's words,
/// light, camera and relation, and each person's description, clothes, pose
/// and expression. Never paths or hashes.
fn record_prose(s: &crate::scene::Scene) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(crate::scene::Setting::Words { text }) = s.setting.as_ref().map(|f| &f.value) {
        out.push(text.clone());
    }
    for f in [&s.light, &s.camera, &s.together].into_iter().flatten() {
        out.push(f.value.clone());
    }
    for p in &s.people {
        if let crate::scene::Who::Described(d) = &p.who {
            out.push(d.clone());
        }
        out.extend([p.wearing.clone(), p.doing.clone(), p.expression.clone()]);
    }
    out
}

/// Every string in a JSON value, for a scan over all of a call's words.
fn collect_strings<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::String(t) => out.push(t),
        Value::Array(items) => items.iter().for_each(|i| collect_strings(i, out)),
        Value::Object(map) => map.values().for_each(|i| collect_strings(i, out)),
        _ => {}
    }
}

/// A picture's line in its chat's queue (`jobs::DeferredJob::with_label`):
/// who is in it, what the first of them is doing, and where — a handle the
/// owner reads to tell one queued picture from another, never the prompt.
/// Capped, since a phone shows it on one line.
fn queue_label(scene: &crate::scene::Scene) -> String {
    const MAX: usize = 80;
    let who: Vec<String> = scene
        .people
        .iter()
        .map(|p| crate::picture::shown(&p.who))
        .collect();
    let mut parts = Vec::new();
    if !who.is_empty() {
        // What the first is doing goes with the first, never after everyone
        // (review of #607: "Maya, a waiter reading" read as both reading).
        let mut names = who.clone();
        if let Some(d) = scene
            .people
            .first()
            .map(|p| p.doing.trim())
            .filter(|d| !d.is_empty())
        {
            names[0] = format!("{} {d}", names[0]);
        }
        parts.push(names.join(", "));
    }
    match scene.setting.as_ref().map(|f| &f.value) {
        Some(crate::scene::Setting::Words { text }) if !text.trim().is_empty() => {
            parts.push(text.trim().to_string())
        }
        Some(crate::scene::Setting::Photo { .. }) => parts.push("in your photo".to_string()),
        _ => {}
    }
    let mut label = if parts.is_empty() {
        "a picture".to_string()
    } else {
        parts.join(" — ")
    };
    if label.chars().count() > MAX {
        label = label.chars().take(MAX - 1).collect::<String>() + "…";
    }
    label
}

/// The scene's words for a new picture: the setting, light, camera, what the
/// people do together, and the words it renders, quoted exactly.
fn scene_words(scene: &crate::scene::Scene) -> String {
    let close = |s: &str| {
        let s = s.trim();
        if s.ends_with(['.', '!', '?', '"']) {
            s.to_string()
        } else {
            format!("{s}.")
        }
    };
    let mut out: Vec<String> = Vec::new();
    if let Some(crate::scene::Setting::Words { text }) = scene.setting.as_ref().map(|f| &f.value) {
        out.push(close(text));
    }
    for f in [&scene.light, &scene.camera, &scene.together]
        .into_iter()
        .flatten()
    {
        out.push(close(&f.value));
    }
    for w in scene.text.iter().flat_map(|t| t.value.iter()) {
        let at = if w.at.is_empty() {
            String::new()
        } else {
            format!(" {}", w.at)
        };
        let look = if w.look.is_empty() {
            String::new()
        } else {
            format!(", in {}", w.look)
        };
        out.push(format!(
            "{}\"{}\" appear{at}{look}.",
            crate::picture::RENDERED,
            w.words
        ));
    }
    out.join(" ")
}

/// The reader's change merged into a persona's call where the call is
/// silent: a `together` when it has none, and each person's `doing` and
/// `wearing` when theirs has none, for people the call already names (mecha-
/// a3's rule b). Nothing else: setting, light, camera and look stay the
/// call's. What was merged, for the manifest.
fn merge_read(
    change: &mut crate::scene::SceneChange,
    read: &serde_json::Map<String, Value>,
) -> Vec<String> {
    let mut merged = Vec::new();
    if change.together.is_none() {
        if let Some(t) = read
            .get("together")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            // Bounded as the call's own `together` is at the door.
            let cap = crate::imagelib::MAX_CAST_FIELD;
            change.together = Some(if t.chars().count() > cap {
                crate::picture::clip_at_sentence(t, cap)
            } else {
                t.to_string()
            });
            merged.push("together".to_string());
        }
    }
    let said = read
        .get("people")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (mut doings, mut wearings) = (0, 0);
    for p in change.people.iter_mut().filter(|p| !p.remove) {
        let names = [p.who.key(), crate::picture::shown(&p.who).to_lowercase()];
        let Some(r) = said.iter().find(|r| {
            r["who"]
                .as_str()
                .is_some_and(|w| names.iter().any(|n| n.eq_ignore_ascii_case(w.trim())))
        }) else {
            continue;
        };
        // Past what the call's own may carry, a part is not merged.
        let text = |k: &str| {
            r[k].as_str()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .filter(|t| t.chars().count() <= crate::imagelib::MAX_CAST_FIELD)
                .map(str::to_string)
        };
        if p.doing.is_none() {
            if let Some(d) = text("doing") {
                p.doing = Some(d);
                doings += 1;
            }
        }
        if p.wearing.is_none() {
            if let Some(w) = text("wearing") {
                p.wearing = Some(w);
                wearings += 1;
            }
        }
    }
    if doings > 0 {
        merged.push(format!("doing×{doings}"));
    }
    if wearings > 0 {
        merged.push(format!("wearing×{wearings}"));
    }
    merged
}

/// How long a picture waits for its people's parts (`roles`) before it is
/// drawn as the call said it.
const ROLE_SPLIT_TIMEOUT: Duration = Duration::from_secs(20);

/// What someone does, said with their place in the frame and their face.
fn doing_words(p: &crate::scene::Person, together: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(crate::scene::Where::Background) = p.at {
        parts.push("in the background".into());
    }
    if !p.doing.trim().is_empty() {
        parts.push(p.doing.trim().trim_end_matches('.').to_string());
    } else if !together {
        // No pose given and nothing said of what the people do together: a
        // plain one, never the reference's own (E3, E8). Beside a `together`
        // a pose drew a person twice, 3 of 12 (mecha-a3, 2026-10-08).
        parts.push("standing naturally".into());
    }
    if !p.expression.trim().is_empty() {
        parts.push(p.expression.trim().trim_end_matches('.').to_string());
    }
    parts.join(", ")
}

/// A person as an edit prompt states them: name, library description,
/// clothes, what they do and show.
fn edit_person(name: &str, text: &str, p: &crate::scene::Person, together: bool) -> String {
    let mut out = format!(" {name}");
    if !text.is_empty() {
        out.push_str(&format!(" ({text})"));
    }
    if !p.wearing.trim().is_empty() {
        out.push_str(&format!(
            ", wearing {}",
            p.wearing.trim().trim_end_matches('.')
        ));
    }
    let doing = doing_words(p, together);
    if !doing.is_empty() {
        out.push_str(&format!(", {doing}"));
    }
    if let Some(at) = p.at.filter(|a| *a != crate::scene::Where::Background) {
        out.push_str(&format!(", {}", at.name()));
    }
    out.push('.');
    out
}

#[async_trait]
impl Tool for ImageGenerate {
    fn name(&self) -> &str {
        "image_generate"
    }

    /// Eligible for a persona (`docs/PERSONA-DESIGN.md` §3.3): its request goes only
    /// to the loopback image server `[image]` names, and it reads no owner store.
    fn for_persona(self: Arc<Self>) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(self.persona_form(None)))
    }

    /// The persona form, knowing who "self" is (`docs/PERSONA-DESIGN.md` §8.6).
    fn for_persona_as(self: Arc<Self>, who: &crate::tool::PersonaSelf) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(self.persona_form(Some(who.clone()))))
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        let person = json!({
            "type": "object",
            "properties": {
                "who": {"type": "string", "description": "A library character's name, \"self\" for you, or a description of someone not in the library."},
                "where": {"type": "string", "enum": ["left", "centre", "right", "background"]},
                "wearing": {"type": "string"},
                "doing": {"type": "string", "description": "Their pose and what they do, in words."},
                "expression": {"type": "string"},
                "remove": {"type": "boolean", "description": "true takes them out of the picture."}
            },
            "required": ["who"]
        });
        let mut schema = json!({
            "type": "object",
            "properties": {
                "picture": {"type": "string", "description": PICTURE_DESC},
                "scene": {
                    "type": "object",
                    "properties": {
                        "setting": {"description": SETTING_DESC},
                        "light": {"type": "string", "description": "Light, mood, time of day, colour tone."},
                        "camera": {"type": "string", "description": "Shot size, angle, framing."},
                        "style": {"type": "string", "description": "A library style's name, only when the user asks for a look; leave it out for the usual photograph. A look the library has no name for goes in words in `setting`."},
                        "people": {"type": "array", "items": person, "maxItems": crate::scene::MAX_PEOPLE, "description": PEOPLE_DESC},
                        "together": {"type": "string", "description": "What the people do with each other, once, by name."},
                        "text": {
                            "type": "array",
                            "items": {"type": "object", "properties": {
                                "words": {"type": "string"},
                                "where": {"type": "string"},
                                "look": {"type": "string"}
                            }, "required": ["words"]},
                            "description": "Words to render exactly."
                        }
                    }
                },
                "retouch": {"type": "string", "description": RETOUCH_DESC},
                "mask": {"type": "string", "description": "The painted area the user's message names, beside a retouch. Never make one up."},
                "size": {"type": "string", "enum": ["square", "landscape", "portrait"]}
            }
        });
        if self.seeds {
            schema["properties"]["seed"] = json!({
                "type": "integer",
                "minimum": 0,
                "description": "A new picture's seed, to reproduce it."
            });
        }
        schema
    }

    /// Read-only in the sense the approval gate means — it changes nothing of
    /// yours — which is the owner's ruling (2026-09-25): a picture should be
    /// one request in any chat, and web chats start read-only. What it writes
    /// is a *new* file under `images/` in the run's own workspace, never an
    /// existing one (`save` opens with `create_new`), the same footing as
    /// `todo` keeping its own list. Parallel calls are safe: the server
    /// queues them, and each polls its own job.
    fn read_only(&self) -> bool {
        true
    }

    /// Nothing private comes back (a path and a seed), nothing external, and
    /// nothing leaves: the destination is a loopback server the operator
    /// configured and the schema cannot name another. It creates only new
    /// files, so it is not destructive.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let lib = self
            .library_dir
            .as_ref()
            .map(|d| crate::imagelib::Library::load(d).0)
            .unwrap_or_default();
        let approved = |n: &str| {
            lib.get(crate::imagelib::Kind::Character, n)
                .is_some_and(|e| e.status == crate::imagelib::Status::Approved)
        };
        let me = self.self_names(&lib);
        let mut call = match crate::picture::parse(&input, &approved, &me, self.seeds) {
            Ok(c) => c,
            Err(why) => return Ok(refused(why)),
        };
        // A `picture` no file in this chat answers to: a name the model made
        // up. Beside a scene it is left out and said, and the scene is drawn
        // without it; on its own it is refused saying where pictures
        // come from. A bare "cannot open" was retried eleven times in one run
        // on 2026-10-08. Only a path inside the jail that does not exist:
        // anything the jail refuses is still refused as before.
        if let Some(p) = call.picture.clone() {
            let missing = ctx.resolve(&p).is_ok_and(|path| {
                // Only absence: a picture present but unreadable is
                // refused by the read below, as before (review of #600).
                std::fs::symlink_metadata(path)
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            });
            if missing {
                // Only a scene that describes a picture of its own (people or
                // a setting) draws without it: a style, light or camera alone
                // is a change to the picture named, and drawn from nothing it
                // was a stranger in an empty room (mecha-a3, review of #603).
                let describes = !call.change.people.is_empty()
                    || call.change.setting.is_some()
                    || call.setting_photo.is_some();
                if !describes {
                    return Ok(refused(format!(
                        "There is no picture `{p}` in this chat. Name a picture as a result \
                         gave it (images/…) or as the owner attached it (inbox/…), or leave \
                         `picture` out and describe a new picture in `scene`: its setting and \
                         people."
                    )));
                }
                call.picture = None;
                call.mask = None;
                // Beside a room photo the render is a placement, not a new
                // picture, so the note says only what was left out.
                call.notes.push(if call.setting_photo.is_some() {
                    format!(
                        "There is no picture `{p}` in this chat, so it was left out and the \
                         scene was drawn on the room photo."
                    )
                } else {
                    format!(
                        "There is no picture `{p}` in this chat, so it was left out and the \
                         scene was drawn as a new picture."
                    )
                });
            }
        }
        // A `style` the library does not hold: the field takes a library
        // style's name, and a model fills it with words ("hyperreal
        // render"). Left out and said, naming the unlocked styles, so the
        // picture still draws; refused only when nothing else is asked. A
        // refusal pointing at image_library, which a persona chat does not
        // have, was retried ten times in one run (2026-10-08). A style
        // waiting on the owner, or one whose entry did not load, is still
        // refused by name: those are findings, not over-fill (review of #603).
        if let Some(name) = call.change.style.clone() {
            // Spelled as a library name: a name is lowercase letters, digits
            // and hyphens, so "Digital painting" can only mean
            // `digital-painting` (mecha-a3, 2026-10-08: the live chat wrote
            // the words eight times beside a library that now holds the name).
            let key = crate::imagelib::spelled_as_name(&name);
            let kind = crate::imagelib::Kind::Style;
            let held = lib
                .get(kind, &key)
                .is_some_and(|e| e.status == crate::imagelib::Status::Approved);
            if !held && !crate::imagelib::absent(&lib, kind, &key) {
                return Ok(refused(crate::imagelib::missing(&lib, kind, &key)));
            }
            if held {
                call.change.style = Some(key.clone());
            }
            if !held {
                call.change.style = None;
                // Said back at most a name's length: the field is read at its
                // own cap, and a result is no place to echo a paragraph.
                let name = if name.chars().count() > crate::imagelib::MAX_NAME {
                    let cut: String = name.chars().take(crate::imagelib::MAX_NAME).collect();
                    format!("{}…", cut.trim_end())
                } else {
                    name
                };
                // The styles there are may be named back (not private, the
                // owner's ruling of 2026-10-08; locked ones left out), never a
                // character. A look with no name goes in words in the setting.
                let named = crate::imagelib::styles_to_name(&lib);
                let styles = named.as_ref().map(|s| format!(" {s}.")).unwrap_or_default();
                if !call.has_scene() && call.retouch.is_none() {
                    let ask = if named.is_some() {
                        "Name one of those, or leave"
                    } else {
                        "Leave"
                    };
                    return Ok(refused(format!(
                        "There is no approved style `{name}`.{styles} {ask} `style` out and \
                         say the look in words in `scene.setting`."
                    )));
                }
                call.notes.push(format!(
                    "There is no approved style `{name}`, so it was left out.{styles} To ask \
                     for a look, name a style or say it in words in `scene.setting`."
                ));
            }
        }
        // A library name that is not drawable is never drawn as a stranger
        // by that name: a candidate waits on the owner, and an entry that
        // did not load is not read as absent.
        for p in &call.change.people {
            let crate::scene::Who::Described(d) = &p.who else {
                continue;
            };
            let key = d.trim().to_lowercase();
            if lib.get(crate::imagelib::Kind::Character, &key).is_some() {
                return Ok(refused(format!(
                    "{} is in the image library but waiting for the owner's approval, so it \
                     cannot be drawn yet.",
                    capitalized(&key)
                )));
            }
        }
        // A character whose entry did not load is invisible to the planner's
        // name check (`named_in` sees only approved entries), so every word of
        // the call is read for one here: "Maya at a diner" with a corrupt
        // `maya` would otherwise reach the GPU and draw a stranger (review of
        // #383; restored on review of #597, pass 4).
        let mut prose: Vec<&str> = Vec::new();
        collect_strings(&input["scene"], &mut prose);
        collect_strings(&input["retouch"], &mut prose);
        if let Some(name) = prose
            .iter()
            .flat_map(|t| crate::imagelib::broken_named_in(&lib, t))
            .next()
        {
            return Ok(refused(format!(
                "{}'s library entry could not be read, so it cannot be drawn; the owner can \
                 check it with `mecha imagelib list`.",
                capitalized(&name)
            )));
        }
        // Before reading anything: references are a cost on the pool this
        // guards.
        if let Err(why) = self.memory_guard().await {
            return Ok(refused(why));
        }
        // The picture being changed, read once: its bytes key its record and
        // are the canvas.
        let picture = match &call.picture {
            Some(p) => match read_references(ctx, std::slice::from_ref(p)).await {
                Ok(mut r) => Some(r.remove(0)),
                Err(why) => return Ok(refused(why)),
            },
            None => None,
        };
        let base = match (&ctx.scene, &picture) {
            (Some(slot), Some(r)) => slot.lookup(&r.bytes),
            _ => None,
        };
        // The record's words reach the prompt too, so they are read for a
        // character whose entry has since broken, as the call's were above
        // (review of #597, pass 10).
        if let Some(name) = base
            .as_ref()
            .map(record_prose)
            .unwrap_or_default()
            .iter()
            .flat_map(|t| crate::imagelib::broken_named_in(&lib, t))
            .next()
        {
            return Ok(refused(format!(
                "{}'s library entry could not be read, so it cannot be drawn; the owner can \
                 check it with `mecha imagelib list`.",
                capitalized(&name)
            )));
        }
        let photo = match &call.setting_photo {
            Some(p) => match read_references(ctx, std::slice::from_ref(p)).await {
                Ok(mut r) => Some(r.remove(0)),
                Err(why) => return Ok(refused(why)),
            },
            None => None,
        };
        let photo_hash = photo.as_ref().map(|r| crate::scene::hash(&r.bytes));
        // What the persona's own call left out of the owner's ask, read on
        // the host's reader and merged in: only a `together`, and each
        // person's `doing` and `wearing`, where the call has none, for people
        // the call names. The merged words are stamped like the call's own
        // (`by`, from the conversation's taint), never clean for being the
        // harness's pass (mecha-05). Before `plan`, so a merged `together`
        // then splits into parts (`roles`).
        // The record the reader read, when it merged anything: its words may
        // come back restated, so its origin joins the call's (review of #610).
        let mut read_from: Option<crate::scene::Origin> = None;
        let reader_said = match (&ctx.scene_reader, call.change.people.is_empty()) {
            (Some(reader), false) => {
                let record = base
                    .clone()
                    .or_else(|| ctx.scene.as_ref().and_then(|s| s.current()));
                let read = tokio::time::timeout(ROLE_SPLIT_TIMEOUT, reader.read(record.as_ref()))
                    .await
                    .unwrap_or_else(|_| Err("the reader took too long".into()));
                Some(match read {
                    Ok(extracted) => {
                        let before = call.change.clone();
                        let merged = merge_read(&mut call.change, &extracted.scene);
                        // The reader's words pass the door the call's did: a
                        // library character whose entry did not load, named
                        // in them, would be drawn as a stranger (review of
                        // #610; the #383 shape). The merge is dropped.
                        let mut words: Vec<&str> =
                            call.change.together.iter().map(String::as_str).collect();
                        for p in &call.change.people {
                            words.extend(p.doing.as_deref());
                            words.extend(p.wearing.as_deref());
                        }
                        if words
                            .iter()
                            .any(|t| !crate::imagelib::broken_named_in(&lib, t).is_empty())
                        {
                            call.change = before;
                            "fell back: the reader named a library entry that could not be read"
                                .to_string()
                        } else if merged.is_empty() {
                            "nothing to merge".to_string()
                        } else {
                            read_from = record.as_ref().map(crate::scene::Scene::origin);
                            format!("merged: {}", merged.join(", "))
                        }
                    }
                    Err(why) => {
                        tracing::warn!("image_generate: scene reader: {why}");
                        format!("fell back: {why}")
                    }
                })
            }
            _ => None,
        };
        // A merge that read an untrusted record lands untrusted, whatever the
        // conversation's own label: the reader may restate what it read, and
        // a scene's origin is what arms the next turn's note.
        let by = match read_from {
            Some(o) => crate::scene::Origin::of(ctx.taint.as_ref()).union(o),
            None => crate::scene::Origin::of(ctx.taint.as_ref()),
        };
        let named = |t: &str| crate::imagelib::named_in(&lib, t);
        // What the chat last drew each person in: clothes a newcomer's call
        // left out come from it (mecha-a3's G1b).
        let current = ctx.scene.as_ref().and_then(|slot| slot.current());
        let worn = |key: &str| {
            current.as_ref().and_then(|s| {
                s.people
                    .iter()
                    .find(|p| p.who.key() == key)
                    .filter(|p| !p.wearing.trim().is_empty())
                    .map(|p| (p.wearing.clone(), p.origin))
            })
        };
        let plan = match crate::picture::plan(
            &call,
            base.as_ref(),
            photo_hash,
            by,
            &approved,
            &named,
            &worn,
        ) {
            Ok(p) => p,
            Err(why) => return Ok(refused(why)),
        };
        let mut req = Request {
            prompt: String::new(),
            negative: String::new(),
            size: None,
            steps: self.cfg.steps,
            seed: match plan.seed {
                crate::picture::Seed::Fresh => fresh_seed(),
                crate::picture::Seed::Base(s) | crate::picture::Seed::Given(s) => s,
            },
            references: Vec::new(),
            reference_size: EDIT_REFERENCE_SIZE,
            mask: None,
        };
        // A redraw is drawn at a seed the picture was not (§5.1).
        if plan.route == "redrawn" {
            while Some(req.seed) == base.as_ref().and_then(|b| b.seed) {
                req.seed = fresh_seed();
            }
        }
        let mut used: Vec<crate::imagelib::Used> = Vec::new();
        let mut mask_plan: Option<MaskPlan> = None;
        let mut crops_said: Vec<String> = Vec::new();
        // Whether the people's parts were split (`roles`), for the manifest:
        // the split is prompt-only and its fallback silent, so this is the
        // one place a picture says what happened (mecha-a3's G5). The
        // prompt itself stays out of the manifest (ARCHITECTURE §images).
        let mut roles_said: Option<String> = None;
        let mut reseeded: Option<u64> = None;
        let mut dropped: Vec<String> = Vec::new();
        let is_edit = matches!(plan.render, crate::picture::Render::Edit { .. });
        // Whether the scene says what its people do together: then nobody is
        // given a plain pose of their own (`doing_words`).
        let together = plan
            .next
            .together
            .as_ref()
            .is_some_and(|f| !f.value.trim().is_empty());
        match &plan.render {
            crate::picture::Render::New => {
                // Each person's own part, when the call put the whole act in
                // `together` and gave nobody a pose: read as one sentence it
                // drew a lineup with the act between neighbours and, on real
                // calls, a person twice in 6 of 12; split into parts, 1 of 12
                // (mecha-a3, 2026-10-08). For the prompt only: the record
                // keeps the call as sent. A split that fails draws the scene
                // as the call said it.
                let mut people = plan.people.clone();
                let mut words_scene = std::borrow::Cow::Borrowed(&plan.next);
                let as_called = together;
                let mut together = together;
                // Whenever the act is one sentence naming two or more people,
                // posed or not: a `together` left in the words beside the
                // people's own poses drew a person twice (mecha-a3's gate of
                // #610, 2 of 3). A pose given is kept verbatim (`roles`).
                if together && people.len() >= 2 {
                    // A door with no splitter says so, so a picture drawn
                    // unsplit where one was due is never a silent null
                    // (review of #609).
                    if ctx.role_split.is_none() {
                        roles_said = Some("not split: no splitter here".into());
                    }
                    if let (Some(splitter), Some(sentence)) =
                        (&ctx.role_split, plan.next.together.as_ref())
                    {
                        let names: Vec<String> = people
                            .iter()
                            .map(|p| crate::picture::shown(&p.who))
                            .collect();
                        let asked: Vec<crate::roles::Asked> = people
                            .iter()
                            .zip(&names)
                            .map(|(p, n)| crate::roles::Asked {
                                who: n.clone(),
                                doing: (!p.doing.trim().is_empty()).then(|| p.doing.clone()),
                            })
                            .collect();
                        // ~1-3 s on the router, 9 s once beside two live
                        // turns (mecha-a3): bounded, and a late split draws
                        // the call as sent.
                        let answer = tokio::time::timeout(
                            ROLE_SPLIT_TIMEOUT,
                            splitter.split(&asked, &sentence.value),
                        )
                        .await
                        .unwrap_or_else(|_| Err("the splitter took too long".into()));
                        match answer {
                            Ok(split) => {
                                // The split's places win (mecha-a3: the persona's
                                // are copies, and at three people the split's are
                                // what put the hands-off side by side), except
                                // `background`, which is a choice and is kept.
                                for (p, name) in people.iter_mut().zip(&names) {
                                    if let Some(r) = split.roles.iter().find(|r| &r.who == name) {
                                        p.doing = r.doing.clone();
                                        if p.at != Some(crate::scene::Where::Background) {
                                            p.at = Some(r.at);
                                        }
                                    }
                                }
                                people.sort_by_key(|p| crate::picture::rank(p.at));
                                let mut scene = plan.next.clone();
                                scene.together = (!split.together.trim().is_empty()).then(|| {
                                    crate::scene::Field {
                                        value: split.together.trim().to_string(),
                                        origin: sentence.origin,
                                    }
                                });
                                together = scene.together.is_some();
                                words_scene = std::borrow::Cow::Owned(scene);
                                roles_said = Some("applied".into());
                            }
                            Err(why) => {
                                tracing::warn!("image_generate: role split: {why}");
                                roles_said = Some(format!("fell back: {why}"));
                            }
                        }
                    }
                }
                let style = plan.next.style.as_ref().map(|s| s.value.clone());
                let compile_from = |people: &[crate::scene::Person],
                                    words: &crate::scene::Scene,
                                    together: bool| {
                    let mut cast = Vec::new();
                    let mut extras = Vec::new();
                    for p in people {
                        match &p.who {
                            crate::scene::Who::Library(n) => {
                                cast.push(crate::imagelib::CastMember {
                                    name: n.clone(),
                                    wearing: p.wearing.clone(),
                                    doing: doing_words(p, together),
                                })
                            }
                            crate::scene::Who::Described(d) => {
                                let mut e = d.trim().trim_end_matches('.').to_string();
                                if !p.wearing.trim().is_empty() {
                                    e.push_str(&format!(", wearing {}", p.wearing.trim()));
                                }
                                let doing = doing_words(p, together);
                                if !doing.is_empty() {
                                    e.push_str(&format!(", {doing}"));
                                }
                                extras.push(e);
                            }
                        }
                    }
                    crate::imagelib::compile(
                        &lib,
                        &match &plan.also {
                            // A retouch given for a new picture, or beside a
                            // restage: one more sentence of the scene.
                            Some(also) => format!("{} {also}", scene_words(words)),
                            None => scene_words(words),
                        },
                        &cast,
                        &extras,
                        style.as_deref(),
                    )
                };
                // A split is prompt-only and must never turn a drawable call
                // into a refusal: parts the compiler will not take (a
                // described person's part naming a library character, say)
                // fall back to the call as sent (review of #609).
                let compiled = match compile_from(&people, &words_scene, together) {
                    Ok(c) => c,
                    Err(_) if roles_said.as_deref() == Some("applied") => {
                        roles_said = Some("fell back: the split's parts did not compile".into());
                        match compile_from(&plan.people, &plan.next, as_called) {
                            Ok(c) => c,
                            Err(why) => return Ok(refused(why)),
                        }
                    }
                    Err(why) => return Ok(refused(why)),
                };
                req.prompt = compiled.prompt;
                req.references = compiled
                    .references
                    .into_iter()
                    .map(|(name, bytes, ext)| Reference {
                        path: format!("library:{name}"),
                        bytes,
                        ext,
                    })
                    .collect();
                req.reference_size = crate::imagelib::REFERENCE_SIZE;
                used = compiled.used;
                // Never the seed that drew a portrait: sampling there redraws it.
                let asked = req.seed;
                while compiled.source_seeds.contains(&req.seed) {
                    req.seed = fresh_seed();
                }
                if asked != req.seed && plan.seed == crate::picture::Seed::Given(asked) {
                    reseeded = Some(asked);
                }
                // A restage or redraw drawn new from the setting's words
                // keeps the picture's shape: the base seed holds the room only
                // at the same latent size (§2.6; mecha-a3's G4 on #597, as
                // #591 pass 6 had it).
                let shape = picture.as_ref().and_then(|r| Size::nearest(&r.bytes));
                req.size = Some(call.size.or(shape).unwrap_or(Size::Square).dims());
            }
            crate::picture::Render::Edit {
                canvas,
                camera_moves,
            } => {
                let canvas_ref = match canvas {
                    crate::picture::Canvas::Picture(_) => {
                        picture.clone().expect("an edit of a picture read it")
                    }
                    crate::picture::Canvas::Setting(path) => {
                        if call.setting_photo.as_deref() == Some(path.as_str()) {
                            photo
                                .clone()
                                .expect("a setting photo the call named was read")
                        } else {
                            // The record's setting photo: a label, checked by
                            // its hash, so a different file under that name
                            // is never drawn on.
                            let want = plan.next.setting.as_ref().and_then(|f| match &f.value {
                                crate::scene::Setting::Photo { hash, .. } => Some(hash.clone()),
                                _ => None,
                            });
                            match read_references(ctx, std::slice::from_ref(path)).await {
                                Ok(mut r)
                                    if want.as_deref()
                                        == Some(crate::scene::hash(&r[0].bytes).as_str()) =>
                                {
                                    r.remove(0)
                                }
                                _ => {
                                    return Ok(refused(format!(
                                        "The scene's room photo, {path}, is not in this chat as \
                                         it was, so the people cannot be redrawn in it. Attach \
                                         it again, or change the picture as it is."
                                    )))
                                }
                            }
                        }
                    }
                };
                req.references.push(canvas_ref);
                // A phone photo goes up at the encoder's scale and upright.
                let read = std::mem::take(&mut req.references);
                let budget = self.reference_pixels;
                req.references = match tokio::task::spawn_blocking(move || {
                    read.into_iter()
                        .map(|mut r| {
                            if let Some(fitted) = fit_reference(&r.bytes, budget) {
                                r.bytes = fitted;
                                r.ext = "png";
                            }
                            r
                        })
                        .collect()
                })
                .await
                {
                    Ok(fitted) => fitted,
                    Err(e) => {
                        return Ok(refused(format!("The picture could not be prepared: {e}")))
                    }
                };
                // The owner's painted mask, sized with the picture.
                // A mask that is not a picture in this chat is left out and
                // said, never a refusal (mecha-a3's G1b); one that reads but
                // marks nothing is still refused by `prepare_mask`.
                let mask_read = match &plan.mask {
                    Some(raw) => match read_references(ctx, std::slice::from_ref(raw)).await {
                        Ok(mut read) => Some((raw, read.remove(0))),
                        Err(why) => {
                            dropped.push(format!(
                                "The mask {raw} was left out ({why}), so the change was made \
                                 to the whole picture."
                            ));
                            None
                        }
                    },
                    None => None,
                };
                if let Some((raw, mask)) = mask_read {
                    let pic = req.references[0].bytes.clone();
                    let resolution = req.reference_size;
                    let prepared = tokio::task::spawn_blocking(move || {
                        let plan = prepare_mask(&pic, &mask.bytes, resolution)?;
                        let picture = png_bytes(&plan.picture)?;
                        let soft = png_bytes(
                            &image::DynamicImage::ImageLuma8(plan.soft.clone()).to_rgb8(),
                        )?;
                        Ok::<_, String>((plan, picture, soft))
                    })
                    .await;
                    let (mp, pic, soft) = match prepared {
                        Ok(Ok(p)) => p,
                        Ok(Err(why)) => return Ok(refused(why)),
                        Err(e) => {
                            return Ok(refused(format!("The mask could not be prepared: {e}")))
                        }
                    };
                    req.references[0].bytes = pic;
                    req.references[0].ext = "png";
                    req.mask = Some(Reference {
                        path: raw.clone(),
                        bytes: soft,
                        ext: "png",
                    });
                    mask_plan = Some(mp);
                }
                // Who the prompt describes: everyone, when they are placed
                // afresh on a room; otherwise the people the edit changes.
                let placed = matches!(canvas, crate::picture::Canvas::Setting(_));
                let d = &plan.delta;
                let described: Vec<&crate::scene::Person> = plan
                    .people
                    .iter()
                    .filter(|p| {
                        let k = p.who.key();
                        placed
                            || d.added.contains(&k)
                            || d.dressed.contains(&k)
                            || d.expressed.contains(&k)
                            || d.posed.contains(&k)
                    })
                    .collect();
                // Identity: a head crop of each face this render needs, with
                // the library description (IMAGE-DESIGN.md §2.1). Not on a
                // masked edit, where nothing outside the mask moves.
                let masked = req.mask.is_some();
                let entries: Vec<Option<crate::imagelib::Entry>> = described
                    .iter()
                    .map(|p| match &p.who {
                        crate::scene::Who::Library(n) if plan.faces.contains(&p.who.key()) => lib
                            .get(crate::imagelib::Kind::Character, n)
                            .filter(|e| e.status == crate::imagelib::Status::Approved)
                            .cloned(),
                        _ => None,
                    })
                    .collect();
                let faces = Arc::clone(&self.faces);
                let for_faces = lib.clone();
                let wanted = entries.clone();
                let anchors = tokio::task::spawn_blocking(move || {
                    wanted
                        .iter()
                        .map(|e| match e {
                            Some(_) if masked => None,
                            Some(e) => Some(faces.anchor(&for_faces, e)),
                            None => None,
                        })
                        .collect::<Vec<_>>()
                })
                .await
                // A detector that panicked costs the crops, and is said like
                // any other reason a face could not be had (`face.rs`: "said,
                // never silently drawn without"; review of #597, pass 3).
                .unwrap_or_else(|_| {
                    entries
                        .iter()
                        .map(|e| {
                            e.as_ref().filter(|_| !masked).map(|_| {
                                crate::face::Anchor::Unavailable("the face detector failed".into())
                            })
                        })
                        .collect()
                });
                let mut said = String::new();
                let mut crops = 0usize;
                for ((p, entry), anchor) in described.iter().zip(&entries).zip(anchors) {
                    let name = crate::picture::shown(&p.who);
                    let text = entry
                        .as_ref()
                        .map(|e| e.text.trim().trim_end_matches('.').to_string())
                        .unwrap_or_default();
                    said.push_str(&edit_person(&name, &text, p, together));
                    match anchor {
                        Some(crate::face::Anchor::Crop(bytes)) => {
                            req.references.push(Reference {
                                path: format!("{FACE_REFERENCE}{}", p.who.key()),
                                bytes,
                                ext: "png",
                            });
                            crops += 1;
                            let k = req.references.len();
                            said.push_str(&format!(
                                " Take only {name}'s facial identity from <image{k}>, nothing \
                                 else."
                            ));
                            crops_said.push(name.clone());
                        }
                        Some(crate::face::Anchor::NoFace) => dropped.push(format!(
                            "No face was found in {name}'s library portrait, so {name} was \
                             drawn from the description alone."
                        )),
                        Some(crate::face::Anchor::Unavailable(why)) => dropped.push(format!(
                            "{name}'s face could not be taken from the library ({why}), so \
                             {name} was drawn from the description alone."
                        )),
                        None => {}
                    }
                    if let Some(e) = entry {
                        used.push(crate::imagelib::Used {
                            kind: crate::imagelib::Kind::Character,
                            name: p.who.key(),
                            version: e.version,
                            portrait: e.portrait.clone(),
                        });
                    }
                }
                let close = |s: &str| {
                    let s = s.trim();
                    if s.is_empty() || s.ends_with(['.', '!', '?', '"']) {
                        s.to_string()
                    } else {
                        format!("{s}.")
                    }
                };
                let mut prompt = String::new();
                if !plan.keep.is_empty() && !masked {
                    prompt.push_str(&format!("Keep {} unchanged. ", plan.keep));
                }
                prompt.push_str(&close(&plan.instruction));
                // A retouch given beside the scene change: one more line.
                if let Some(also) = &plan.also {
                    prompt.push(' ');
                    prompt.push_str(&close(also));
                }
                // A style the edit changes to: its own words, as a new
                // picture's compile pastes them (review of #597).
                if plan.delta.style {
                    let Some(name) = plan
                        .next
                        .style
                        .as_ref()
                        .map(|f| f.value.trim().to_lowercase())
                    else {
                        return Ok(refused("The style to redraw in was empty."));
                    };
                    match lib
                        .get(crate::imagelib::Kind::Style, &name)
                        .filter(|e| e.status == crate::imagelib::Status::Approved)
                    {
                        Some(entry) => {
                            prompt.push(' ');
                            prompt.push_str(&close(&entry.text));
                            used.push(crate::imagelib::Used {
                                kind: crate::imagelib::Kind::Style,
                                name,
                                version: entry.version,
                                portrait: None,
                            });
                        }
                        None => {
                            // Defence only: the call's style was proved
                            // approved after parse, so this answers what the
                            // library would, never naming a tool.
                            return Ok(refused(crate::imagelib::missing(
                                &lib,
                                crate::imagelib::Kind::Style,
                                &name,
                            )));
                        }
                    }
                }
                // Placed afresh on a room photo, the people are drawn into
                // the whole scene: its camera, light, what they do together
                // and its words. The photo is the setting, so it says none.
                if placed {
                    let scene = scene_words(&plan.next);
                    if !scene.is_empty() {
                        prompt.push(' ');
                        prompt.push_str(&scene);
                    }
                }
                if crops > 0 {
                    prompt.push_str(if *camera_moves {
                        CANVAS_CAMERA_MOVES
                    } else {
                        CANVAS_KEEPS_CAMERA
                    });
                }
                prompt.push_str(&said);
                if crops > 1 || (placed && described.len() > 1) {
                    prompt.push_str(" Each of them appears exactly once.");
                }
                req.prompt = prompt.trim().to_string();
                // A masked edit keeps the picture's own shape and full detail.
                // Every other edit names its output size, and encodes its
                // references smaller (`UNMASKED_EDIT_REFERENCE_SIZE`) unless
                // the canvas is somebody's only identity source (below). On a
                // room photo the room's own shape is kept whatever size was
                // asked: a portrait from a landscape room drew a slice of
                // table (owner, 2026-10-08).
                // A canvas whose shape cannot be read (an animated WebP, one
                // past the decode cap) keeps 1024 references and no named
                // size, so the encoder still sizes the picture from it rather
                // than drawing at 512 (review of #600).
                req.size = if masked {
                    None
                } else {
                    let own = canvas_dims(&req.references[0].bytes);
                    let size = match canvas {
                        crate::picture::Canvas::Setting(_) => {
                            if call.size.is_some() {
                                dropped.push(
                                    "The room photo's own shape was kept: a picture placed in \
                                     a room is drawn in the room's shape."
                                        .into(),
                                );
                            }
                            own
                        }
                        crate::picture::Canvas::Picture(_) => call.size.map(Size::dims).or(own),
                    };
                    // On an edit of the picture, anyone in it without a head
                    // crop (everyone, on a retouch; a co-subject the edit
                    // leaves alone; someone not in the library) has the
                    // canvas as their only identity source, and at 512 that
                    // is too thin: mecha-a3 measured a retouch's ArcFace fall
                    // from .66–.70 to .42–.53 over three seeds (2026-10-08).
                    // Only an edit whose every person has a crop goes small;
                    // that is the case measured to hold at 512 (review of
                    // #601).
                    let canvas_only = matches!(canvas, crate::picture::Canvas::Picture(_))
                        && crops < plan.people.len().max(1);
                    if size.is_some() && !canvas_only {
                        req.reference_size = UNMASKED_EDIT_REFERENCE_SIZE;
                    }
                    size
                };
            }
        }
        // The planner caps faces, so this never fires; it is the budget's
        // own check, so a planner change cannot quietly pass it.
        if is_edit && req.references.len() > EDIT_REFERENCE_BUDGET {
            return Ok(refused(format!(
                "This change needs {} pictures at once; one edit holds {EDIT_REFERENCE_BUDGET}. \
                 Change fewer people at a time.",
                req.references.len()
            )));
        }
        // Someone named but not in the picture is never drawn: the image
        // model reads "the viewer" (the owner's ruling, 2026-10-08).
        let rendered: Vec<&str> = plan
            .next
            .text
            .iter()
            .flat_map(|t| t.value.iter().map(|w| w.words.as_str()))
            .collect();
        req.prompt = crate::picture::as_viewer(&req.prompt, &plan.offstage, &rendered);
        if req.prompt.chars().count() > crate::imagelib::MAX_COMPILED_PROMPT {
            return Ok(refused(format!(
                "The picture's description came to over {} characters; say less in the scene.",
                crate::imagelib::MAX_COMPILED_PROMPT
            )));
        }
        // The render, the save and the record are the job
        // (`docs/BACKGROUND-JOBS-DESIGN.md` §2.1). Its name is reserved here,
        // so "being made" can name it.
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
        let reserved = format!("images/{stamp}-{}.png", req.seed);
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut job_ctx = ctx.clone();
        job_ctx.cancel = Some(cancel.clone());
        // Never the run's event sender: a job holding one would hold the
        // whole chat until the picture is done (review of #583, pass 3).
        job_ctx.events = None;
        let me = self.clone();
        let picture_path = call.picture.clone();
        let canvas_path = match &plan.render {
            crate::picture::Render::New => None,
            crate::picture::Render::Edit { canvas, .. } => Some(match canvas {
                crate::picture::Canvas::Picture(p) | crate::picture::Canvas::Setting(p) => {
                    p.clone()
                }
            }),
        };
        let size_asked = call.size.is_some();
        // Its line in the chat's queue, taken before the job owns the plan.
        let label = queue_label(&plan.next);
        let job = async move {
            let (me, ctx) = (&me, &job_ctx);
            me.generation.fetch_add(1, Ordering::SeqCst);
            let started = Instant::now();
            let timeout = Duration::from_secs(me.cfg.timeout_secs);
            let Outcome { image, left } = me
                .backend
                .generate(
                    &me.cfg,
                    &req,
                    ctx.cancel.as_ref(),
                    timeout,
                    ctx.image_trail.as_deref(),
                )
                .await;
            me.arm_unload();
            let left = left
                .map(|left| {
                    if ctx.image_trail.is_some() {
                        tracing::debug!("image server temp copies not removed: {left}");
                    } else {
                        tracing::warn!("image server temp copies not removed: {left}");
                    }
                    format!(
                        " The image server's temp copies could not all be removed: {left}. Check \
                         [image] server_temp_dir — for ComfyUI, the --temp-directory path with \
                         `temp` appended."
                    )
                })
                .unwrap_or_default();
            let bytes = match image {
                Ok(bytes) => bytes,
                Err(Failure::Cancelled) => {
                    return ToolOutput::err(format!(
                        "Cancelled — the generation was stopped and nothing was saved.{left}"
                    ))
                }
                Err(Failure::Other(e)) => {
                    let reach = if e.chain().any(|c| c.is::<reqwest::Error>()) {
                        format!(
                            " Is the image server running at {}? The operator starts it; \
                             you cannot.",
                            me.cfg.url
                        )
                    } else {
                        String::new()
                    };
                    return ToolOutput::err(format!(
                        "Image generation failed: {e:#}.{reach}{left}"
                    ));
                }
            };
            // A masked edit keeps everything the owner did not paint.
            let bytes = match &mask_plan {
                None => bytes,
                Some(mp) => {
                    let mp = mp.clone();
                    match tokio::task::spawn_blocking(move || composite_masked(&bytes, &mp)).await {
                        Ok(Ok(bytes)) => bytes,
                        Ok(Err(why)) => {
                            return ToolOutput::err(format!(
                                "The image was made but could not be laid over the original: \
                                 {why}. Nothing was saved.{left}"
                            ))
                        }
                        Err(e) => {
                            return ToolOutput::err(format!(
                                "The image was made but could not be laid over the original: \
                                 {e}. Nothing was saved.{left}"
                            ))
                        }
                    }
                }
            };
            let path = match save(ctx, &stamp, req.seed, &bytes).await {
                Ok(path) => path,
                Err(e) => {
                    return ToolOutput::err(format!(
                        "The image was made but not saved: {e:#}{left}"
                    ))
                }
            };
            let secs = started.elapsed().as_secs();
            // An edit whose layout came back nearly the same is said as a
            // fact, never as advice (IMAGE-DESIGN.md §8, review Q1).
            // Read only against the picture it edited: a render on the room
            // photo (placed, restaged or redrawn there) is meant to move
            // everything, and its canvas is not `picture` (review of #597,
            // pass 8).
            let on_picture = matches!(
                plan.render,
                crate::picture::Render::Edit {
                    canvas: crate::picture::Canvas::Picture(_),
                    ..
                }
            );
            let similarity = if on_picture {
                let (was, now) = (req.references[0].bytes.clone(), bytes.clone());
                let painted = mask_plan.as_ref().map(|p| (p.soft.clone(), p.bounds));
                tokio::task::spawn_blocking(move || match painted {
                    Some((soft, bounds)) => layout_similarity_painted(&was, &now, &soft, bounds),
                    None => layout_similarity(&was, &now),
                })
                .await
                .ok()
                .flatten()
            } else {
                None
            };
            // The scene this render lands as, after the picture is saved,
            // so a cancelled or failed render never advances it.
            let picture_hash = crate::scene::hash(&bytes);
            let landed = ctx.scene.as_ref().map(|slot| {
                let mut next = plan.next.clone();
                next.picture = Some(picture_hash.clone());
                // The record keeps the seed that drew its room: a new picture's
                // or a restage's own. An edit samples fresh (#306) over a room
                // it did not draw, so it keeps the base's, or a later restage
                // would redraw a different room (review of #597, pass 7).
                if !is_edit {
                    next.seed = Some(req.seed);
                }
                next.chat = Some(slot.chat.clone());
                (slot.clone(), next)
            });
            let size = match req.size {
                Some((w, h)) => format!("{w}×{h}"),
                None => "picture-shaped".to_string(),
            };
            let manifest = json!({
                "image": path,
                "tool_use_id": ctx.call_id,
                "created": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                "route": plan.route,
                "picture": picture_path,
                "mask": plan.mask,
                "seed": req.seed,
                "steps": req.steps,
                "size": req.size,
                // Who was drawn, by name and entry version: what save-to-library
                // reads. Never the scene itself, which lives outside the jail.
                "cast": used.iter().filter(|u| u.kind == crate::imagelib::Kind::Character)
                    .map(|u| json!({"name": u.name, "version": u.version, "portrait": u.portrait}))
                    .collect::<Vec<_>>(),
                "style": used.iter().find(|u| u.kind == crate::imagelib::Kind::Style)
                    .map(|u| json!({"name": u.name, "version": u.version})),
                "crops": crops_said,
                "roles": roles_said,
                "reader": reader_said,
                "layout_similarity": similarity.map(|r| (r * 1000.0).round() / 1000.0),
                "scene": landed.as_ref().map(|(_, s)| json!({"picture": s.picture})),
                "model": {
                    "backend": "comfyui",
                    "diffusion_model": me.cfg.diffusion_model,
                    "text_encoder": me.cfg.text_encoder,
                    "vae": me.cfg.vae,
                },
            });
            // The prompt, for the owner only, outside the jail (the manifest
            // never carries it): a line that cannot be written costs the
            // picture nothing.
            if let Some(log) = &ctx.prompt_log {
                let line = json!({
                    "created": manifest["created"],
                    "image": path,
                    "route": plan.route,
                    "seed": req.seed,
                    "roles": manifest["roles"],
                    "reader": manifest["reader"],
                    "prompt": req.prompt,
                });
                if let Err(e) = append_prompt(log, &line) {
                    tracing::warn!("image_generate: prompt log: {e:#}");
                }
            }
            let manifest_note = match write_manifest(ctx, &path, &manifest).await {
                Ok(()) => String::new(),
                Err(e) => format!(" (The new picture's manifest was not written: {e:#}.)"),
            };
            if let Some((slot, scene)) = landed {
                let p = path.clone();
                let wrote = tokio::task::spawn_blocking(move || slot.land(&scene)).await;
                if !matches!(wrote, Ok(Ok(()))) {
                    tracing::warn!("the scene for {p} was not recorded: {wrote:?}");
                }
            }
            let of = picture_path.as_deref().unwrap_or("the picture");
            let with = used
                .iter()
                .map(|u| {
                    let kind = match u.kind {
                        crate::imagelib::Kind::Character => "character",
                        crate::imagelib::Kind::Style => "style",
                    };
                    format!("{kind} {} (v{})", u.name, u.version)
                })
                .collect::<Vec<_>>();
            let with = if with.is_empty() {
                String::new()
            } else {
                format!(" with {}", with.join(", "))
            };
            // Facts only (PERSONA-CONTEXT-DESIGN.md §5.2): how to use the
            // result lives once, in the description. The new picture's
            // status comes first, so "it" never reads as the original
            // (review of #581).
            let drawn = format!("drawn in {secs} s (seed {}, {} steps)", req.seed, req.steps);
            let mut text = match &canvas_path {
                None => {
                    let mut t = format!(
                        "image: {path}\nA new {size} picture{with}, {drawn}. It is on the \
                         owner's screen; you have not seen it."
                    );
                    match plan.route {
                        "restaged" => t.push_str(&format!(
                            " It was restaged from the scene's setting in words, not edited \
                             from {of}."
                        )),
                        "redrawn" => t.push_str(" It is the scene drawn again at a new seed."),
                        _ => {}
                    }
                    t
                }
                Some(canvas) => {
                    let how = match plan.route {
                        "placed" => "The people placed on",
                        "restaged" => {
                            "Restaged, not edited from the last picture, on the \
                                       scene's setting photo"
                        }
                        "retouched" => "A retouch of",
                        _ => "An edit of",
                    };
                    format!(
                        "image: {path}\n{how} {canvas}{with}: a {size} picture, {drawn}. The \
                         new picture is on the owner's screen; you have not seen it. {canvas} \
                         is unchanged."
                    )
                }
            };
            if let Some(mp) = &mask_plan {
                let mask = plan.mask.as_deref().unwrap_or("the mask");
                if size_asked {
                    text.push_str(
                        " (The size asked for was not used: a masked edit keeps the picture's own \
                         shape.)",
                    );
                }
                let (cw, ch) = mp.picture.dimensions();
                text.push_str(&if mp.source == (cw, ch) {
                    format!(
                        " Only the area painted in {mask} was redrawn, blended over a narrow \
                         edge around it; everything beyond that edge is the original, pixel for \
                         pixel."
                    )
                } else {
                    let (pw, ph) = mp.source;
                    format!(
                        " Only the area painted in {mask} was redrawn. The picture is \
                         {pw}×{ph} and was edited at {cw}×{ch}, its edit size, as any edit of it \
                         is; outside the painted area the result is the original at that size."
                    )
                });
            }
            if let Some(asked) = reseeded {
                text.push_str(&format!(
                    " Seed {asked} was not used: it drew a portrait in the picture, and \
                     sampling there redraws the portrait."
                ));
            }
            if let Some(said) = &plan.said {
                text.push(' ');
                text.push_str(said);
            }
            for d in &dropped {
                text.push(' ');
                text.push_str(d);
            }
            if let Some(r) = similarity.filter(|r| *r >= NEAR_COPY_LAYOUT) {
                text.push_str(&format!(
                    " The new picture's layout came back nearly the same as {of}'s (similarity \
                     {r:.2})."
                ));
            }
            text.push_str(&manifest_note);
            text.push_str(&left);
            ToolOutput::ok(text)
        };
        Ok(ToolOutput::deferred(
            format!("{BEING_MADE}{reserved}"),
            crate::jobs::DeferredJob::new(job, cancel, BUSY).with_label(label),
        ))
    }
}

/// What a second picture in the same conversation is told while one is being
/// made (`jobs::DeferredJob::busy`): a refusal before any GPU time, with
/// `refused`'s lead.
const BUSY: &str = "Nothing was drawn. This conversation already has a picture being made and \
                    the queue behind it is full; each appears on the owner's screen when it is \
                    done.";

/// The tests call the tool directly and read the finished picture: an
/// inherent `call`, which the concrete type resolves before the trait's,
/// awaits a deferred render inline as a host-less run does
/// (`jobs::run_inline`). Production calls go through `dyn Tool` and get the
/// deferral — so a new test written here exercises the inline path, and one
/// that means the deferred path must call through `<dyn Tool>` or a host
/// (`the_manifest_names_the_call_it_answers` is the one that carries the
/// deferred render to its manifest; review of #583).
#[cfg(test)]
impl ImageGenerate {
    pub(crate) async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let out = <Self as Tool>::call(self, input, ctx).await?;
        Ok(match out.deferred.clone() {
            Some(job) => crate::jobs::run_inline(&job, ctx.cancel.as_ref()).await,
            None => out,
        })
    }
}

/// The first line of a result whose picture is still being drawn (§2.1 of
/// `docs/BACKGROUND-JOBS-DESIGN.md`): what the page reads as still out, and
/// what [`repair_orphan`] looks for.
pub const BEING_MADE: &str = "being made: ";

/// A "being made" result with no job behind it — the server restarted while
/// it drew — settled by **asking the artifact, not the absent job**
/// (`docs/BACKGROUND-JOBS-DESIGN.md` §2.3). A picture counts only if a
/// manifest in the chat's `images/` names this call: the reserved name was
/// told to the model before the bytes existed, so a file at that path alone
/// is not even this call's. The manifest is the job's last write, after the
/// bytes and the near-copy check, so a picture without one is a picture
/// nobody was shown, and reads `not made`.
///
/// **What this does not prove.** The manifest sits in the same jail as the
/// picture, and the call's id is the model's own, so a run that writes both
/// files with `fs_write` can plant a "finished" picture for a render a
/// restart cut short (review of #583). The cost is bounded — a picture in
/// the chat's gallery the image server did not draw, no reach anywhere — and
/// a model that wants to show a fake picture has plainer routes; a manifest
/// out of the jail's reach would be the fix if that ever matters.
///
/// `workspace` is the chat's own directory, never a model-supplied path,
/// and every name read is one the directory listing returned. `None` when
/// `content` is not a "being made" result.
pub fn repair_orphan(
    workspace: &std::path::Path,
    tool_use_id: &str,
    content: &str,
) -> Option<(String, bool)> {
    content.lines().next()?.strip_prefix(BEING_MADE)?;
    let made = std::fs::read_dir(workspace.join("images"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter(|m| m["tool_use_id"].as_str() == Some(tool_use_id))
        .filter_map(|m| m["image"].as_str().map(str::to_string))
        .find(|image| {
            image
                .strip_prefix("images/")
                .and_then(|name| name.strip_suffix(".png"))
                .is_some_and(|stem| {
                    !stem.is_empty()
                        && stem
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
                        && !stem.starts_with('.')
                })
                && workspace.join(image).is_file()
        });
    Some(match made {
        Some(image) => (
            format!(
                "image: {image}\nThe picture was finished before the server restarted. It is on \
                 the owner's screen; you have not seen it."
            ),
            false,
        ),
        None => ("not made: the server restarted".to_string(), true),
    })
}

/// A refusal before any GPU time, saying so first. The first live run read a
/// refusal that opened with the characters' names as a finished picture,
/// never retried, and told the owner it existed (2026-09-28); one exit for
/// every pre-GPU refusal in `call` means the next one added cannot ship
/// without the lead (review of #384).
fn refused(why: impl std::fmt::Display) -> ToolOutput {
    ToolOutput::err(format!("Nothing was drawn. {why}"))
}

/// One line to the owner's prompt log, owner-only (0600) as the transcript
/// beside it is.
fn append_prompt(log: &std::path::Path, line: &Value) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log)?;
    writeln!(f, "{line}")
}

/// Write a generation's manifest beside its PNG — `images/<stem>.json`, new
/// or not at all, like the picture. It is what makes the image reproducible,
/// and what "save to library" and lineage read.
async fn write_manifest(ctx: &ToolCtx, png: &str, manifest: &Value) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let stem = png
        .strip_suffix(".png")
        .ok_or_else(|| anyhow!("`{png}` is not a PNG path"))?;
    let path = ctx.resolve(&format!("{stem}.json"))?;
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options.open(&path).await?;
    file.write_all(serde_json::to_string_pretty(manifest)?.as_bytes())
        .await?;
    file.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::jpeg_with_orientation;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake-pixels";

    /// What a job asks for follows what the server holds: cold after a
    /// restart or when unknown, and less by the load's cost once it has loaded
    /// the model, keeping whatever margin the operator added.
    /// `LOAD_COST_MB` is written out from the default because a `Default`
    /// impl is not const; this is what makes "change both" a failure rather
    /// than a comment when only one moves.
    #[test]
    fn the_load_cost_is_the_default_less_a_reload() {
        assert_eq!(
            LOAD_COST_MB + 12_288,
            ImageConfig::default().min_available_mb
        );
    }

    #[test]
    fn the_memory_asked_for_follows_what_the_server_holds() {
        assert_eq!(memory_need_mb(Some(false), 20_480), 20_480);
        assert_eq!(
            memory_need_mb(None, 20_480),
            20_480,
            "unknown is never warm"
        );
        assert_eq!(memory_need_mb(Some(true), 20_480), 12_288);
        // An operator's raised margin is kept, not capped at the default's.
        assert_eq!(memory_need_mb(Some(true), 30_000), 30_000 - LOAD_COST_MB);
        // A small figure is not computed into the "check off" sentinel: the
        // unreadable-gauge refusal must survive it.
        assert_eq!(memory_need_mb(Some(true), 4_000), 1);
        assert!(memory_verdict(None, memory_need_mb(Some(true), 4_000)).is_err());
        // Only the operator's own 0 switches it off.
        assert_eq!(memory_need_mb(Some(true), 0), 0);
    }

    /// The stats as ComfyUI 0.37 answered them on 2026-10-02, fresh, loaded
    /// and after `/free`; anything else reads as unknown.
    #[test]
    fn loaded_is_read_from_the_servers_stats() {
        let stats =
            |total: Value| json!({"devices": [{"name": "cuda:0", "torch_vram_total": total}]});
        assert_eq!(loaded_from_stats(&stats(json!(0))), Some(false));
        assert_eq!(
            loaded_from_stats(&stats(json!(6_000_000_000u64))),
            Some(true)
        );
        assert_eq!(loaded_from_stats(&stats(json!("5 GB"))), None);
        assert_eq!(loaded_from_stats(&json!({"devices": []})), None);
        assert_eq!(loaded_from_stats(&json!({"system": {}})), None);
    }

    /// A loopback address nothing listens on yet.
    fn closed_port() -> std::net::SocketAddr {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        addr
    }

    fn server_at(addr: std::net::SocketAddr) -> ComfyUi {
        ComfyUi::for_config(&ImageConfig {
            url: format!("http://{addr}"),
            ..ImageConfig::default()
        })
        .unwrap()
    }

    /// A server restarting under a job (the idle reset, a deploy) is waited
    /// for: the job asks again until the port answers, rather than telling
    /// the model the image server is not running. Never seen before, it is
    /// met within the short wait an idle reset fits in.
    #[tokio::test]
    async fn a_restarting_server_is_waited_for_not_reported_down() {
        let addr = closed_port();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = sock.read(&mut buf).await;
                    let _ = sock
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                        )
                        .await;
                });
            }
        });
        let started = Instant::now();
        let waited = server_at(addr).await_server(None, SERVER_WAIT).await;
        assert!(waited.is_ok(), "a server that came up was reported down");
        assert!(
            started.elapsed() >= Duration::from_millis(250),
            "answered before the server existed"
        );
    }

    /// The wait is bounded: a server that never comes is reported, with the
    /// connection error the tool turns into "Is the image server running?".
    #[tokio::test]
    async fn a_server_that_never_comes_is_reported_after_the_wait() {
        let started = Instant::now();
        let waited = server_at(closed_port())
            .await_server(None, Duration::from_millis(800))
            .await;
        match waited {
            Err(Failure::Other(e)) => assert!(
                e.chain().any(|c| c.is::<reqwest::Error>()),
                "not a connection error: {e:#}"
            ),
            Err(Failure::Cancelled) => panic!("reported as cancelled"),
            Ok(()) => panic!("a closed port answered"),
        }
        assert!(started.elapsed() >= Duration::from_millis(700));
    }

    /// A server never seen by this process gets the short wait, not the full
    /// one: a stopped service is reported in seconds. One that has answered
    /// before is waited for in full, since it is restarting.
    #[tokio::test]
    async fn only_a_server_seen_before_is_waited_for_in_full() {
        let unseen = server_at(closed_port());
        let started = Instant::now();
        assert!(unseen
            .await_server(None, Duration::from_secs(60))
            .await
            .is_err());
        assert!(
            started.elapsed() < UNSEEN_WAIT + Duration::from_secs(2),
            "a never-seen server was waited on for {:?}",
            started.elapsed()
        );

        let seen = server_at(closed_port());
        seen.answered.store(true, Ordering::Relaxed);
        let started = Instant::now();
        assert!(seen.await_server(None, UNSEEN_WAIT * 3).await.is_err());
        assert!(
            started.elapsed() >= UNSEEN_WAIT * 2,
            "a server seen before was given up on after {:?}",
            started.elapsed()
        );
    }

    /// A cancel ends the wait at once: a stopped picture does not sit out
    /// the rest of a 90 s window.
    #[tokio::test]
    async fn a_cancel_ends_the_wait() {
        let token = CancellationToken::new();
        let stop = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            stop.cancel();
        });
        let started = Instant::now();
        let waited = server_at(closed_port())
            .await_server(Some(&token), Duration::from_secs(60))
            .await;
        assert!(matches!(waited, Err(Failure::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    fn ctx(dir: &std::path::Path) -> ToolCtx {
        ToolCtx {
            workspace: dir.to_path_buf(),
            ..Default::default()
        }
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mecha-imagegen-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn reply(status: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn json_reply(body: Value) -> Vec<u8> {
        reply("200 OK", "application/json", body.to_string().as_bytes())
    }

    /// A ComfyUI stand-in answering each request line with a canned response,
    /// chosen by path, logging every request line and body it saw. `history`
    /// answers come off a queue, so a test can say "running, running, done".
    async fn fake(
        history: Vec<Value>,
        prompt_status: &'static str,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        fake_running(history, prompt_status, true, "temp").await
    }

    /// As [`fake`], with `GET /queue` reporting `job-1` as running or not —
    /// queued behind someone else's job when `running` is false.
    async fn fake_running(
        history: Vec<Value>,
        prompt_status: &'static str,
        running: bool,
        upload_type: &'static str,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        fake_with(Fake {
            history,
            prompt_status,
            running,
            upload_type,
            ..Fake::default()
        })
        .await
    }

    /// What [`fake_with`]'s server does beyond the defaults.
    struct Fake {
        history: Vec<Value>,
        prompt_status: &'static str,
        running: bool,
        upload_type: &'static str,
        /// `/prompt`'s answer body, in place of `{"prompt_id": "job-1", …}`.
        prompt_body: Option<Value>,
        /// A path answered by hanging up, after doing its work — an answer
        /// lost on the way back.
        hang_up: Option<&'static str>,
        /// Where uploads are written under the name they were sent with, as
        /// the real server does; otherwise each is answered as `up.png`.
        temp: Option<std::path::PathBuf>,
        /// The record an interrupted job reads back as, in place of an error
        /// with no outputs — one that finished as it was stopped.
        interrupted_record: Option<Value>,
        /// What `/view` answers, in turn, in place of [`PNG`]'s undecodable
        /// bytes; the last one answers every call after.
        views: Vec<Vec<u8>>,
    }

    impl Default for Fake {
        fn default() -> Self {
            Fake {
                history: Vec::new(),
                prompt_status: "200 OK",
                running: true,
                upload_type: "temp",
                prompt_body: None,
                hang_up: None,
                temp: None,
                interrupted_record: None,
                views: Vec::new(),
            }
        }
    }

    async fn fake_with(opts: Fake) -> (String, Arc<Mutex<Vec<String>>>) {
        let Fake {
            history,
            prompt_status,
            running,
            upload_type,
            prompt_body,
            hang_up,
            temp,
            interrupted_record,
            views,
        } = opts;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let history = Arc::new(Mutex::new(std::collections::VecDeque::from(history)));
        // Like the real server: an interrupted job is written to the history
        // only after `/interrupt` has answered.
        let interrupted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let views = Arc::new(Mutex::new(std::collections::VecDeque::from(views)));
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let log = Arc::clone(&log);
                let history = Arc::clone(&history);
                let interrupted = Arc::clone(&interrupted);
                let prompt_body = prompt_body.clone();
                let temp = temp.clone();
                let interrupted_record = interrupted_record.clone();
                let views = Arc::clone(&views);
                tokio::spawn(async move {
                    let mut req = Vec::new();
                    let mut tmp = [0u8; 8192];
                    let mut head_end = None;
                    loop {
                        if let Some(end) = head_end {
                            let head = String::from_utf8_lossy(&req[..end]).to_lowercase();
                            let len = head
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length:"))
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            if req.len() >= end + 4 + len {
                                break;
                            }
                        }
                        match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => req.extend_from_slice(&tmp[..n]),
                        }
                        if head_end.is_none() {
                            head_end = req.windows(4).position(|w| w == b"\r\n\r\n");
                        }
                    }
                    let text = String::from_utf8_lossy(&req).to_string();
                    let line = text.lines().next().unwrap_or_default().to_string();
                    let body = head_end
                        .map(|e| String::from_utf8_lossy(&req[e + 4..]).to_string())
                        .unwrap_or_default();
                    log.lock().unwrap().push(format!("{line} {body}"));
                    let path = line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or_default()
                        .to_string();
                    let out = if let Some(node) = path.strip_prefix("/object_info/") {
                        let files = |input: &str, name: &str| json!({node: {"input": {"required": {input: [[name], {}]}}}});
                        json_reply(match node {
                            "UNETLoader" => {
                                files("unet_name", "qwen_image_2.1_int8_convrot.safetensors")
                            }
                            "UnetLoaderGGUF" => files("unet_name", "Qwen-Image-2.1-Q4.gguf"),
                            "CLIPLoader" => files("clip_name", "qwen3vl_8b_w4a8.safetensors"),
                            "VAELoader" => files("vae_name", "qwen_image_2.1_vae_bf16.safetensors"),
                            _ => json!({node: {"input": {}}}),
                        })
                    } else if path == "/prompt" {
                        let answer = prompt_body.unwrap_or_else(
                            || json!({"prompt_id": "job-1", "error": {"message": "bad node"}}),
                        );
                        reply(
                            prompt_status,
                            "application/json",
                            answer.to_string().as_bytes(),
                        )
                    } else if path.starts_with("/history/") && interrupted.load(Ordering::SeqCst) {
                        json_reply(interrupted_record.unwrap_or_else(|| {
                            json!({"job-1": {"status": {"status_str": "error",
                                "completed": false}, "outputs": {}}})
                        }))
                    } else if path.starts_with("/history/") {
                        let next = history.lock().unwrap().pop_front().unwrap_or(json!({}));
                        if next == json!("fail") {
                            reply("500 Internal Server Error", "text/plain", b"stalled")
                        } else {
                            json_reply(next)
                        }
                    } else if path.starts_with("/view?") {
                        let mut views = views.lock().unwrap();
                        let view = if views.len() > 1 {
                            views.pop_front()
                        } else {
                            views.front().cloned()
                        };
                        reply("200 OK", "image/png", view.as_deref().unwrap_or(PNG))
                    } else if path == "/upload/image" {
                        let sent = body
                            .split("filename=\"")
                            .nth(1)
                            .and_then(|rest| rest.split('"').next())
                            .unwrap_or("up.png")
                            .to_string();
                        let name = match &temp {
                            Some(dir) => {
                                std::fs::write(dir.join(&sent), "x").unwrap();
                                log.lock().unwrap().push(format!("wrote {sent}"));
                                sent
                            }
                            None => "up.png".to_string(),
                        };
                        json_reply(json!({"name": name, "subfolder": "", "type": upload_type}))
                    } else if path == "/interrupt" {
                        interrupted.store(true, Ordering::SeqCst);
                        json_reply(json!({}))
                    } else if line.starts_with("GET /queue") {
                        let now = if running { "job-1" } else { "someone-else" };
                        json_reply(
                            json!({"queue_running": [[0, now, {}, {}, []]], "queue_pending": []}),
                        )
                    } else {
                        json_reply(json!({}))
                    };
                    if hang_up != Some(path.as_str()) {
                        let _ = sock.write_all(&out).await;
                    }
                    let _ = sock.shutdown().await;
                });
            }
        });
        (format!("http://{addr}"), seen)
    }

    fn done() -> Value {
        json!({"job-1": {
            "status": {"status_str": "success", "completed": true},
            "outputs": {"out": {"images": [
                {"filename": "a_temp_00001_.png", "subfolder": "", "type": "temp"}
            ]}}
        }})
    }

    /// A tool on a fake server with an empty scratch library: `new` points at
    /// the operator's real one, which a test must never read — an edit naming
    /// "maya" found the real `maya` there once edits checked names (2026-10-07).
    fn tool(url: &str) -> ImageGenerate {
        ImageGenerate::new(ImageConfig {
            url: url.into(),
            min_available_mb: 0,
            unload_after_secs: 0,
            ..Default::default()
        })
        .unwrap()
        .polling_every(Duration::from_millis(10))
        .with_library_dir(tempdir())
        .with_seeds()
    }

    #[test]
    fn only_a_server_on_this_machine_is_accepted() {
        for ok in [
            "http://127.0.0.1:8188",
            "http://localhost:8188/",
            "http://[::1]:8188",
            "https://LOCALHOST",
        ] {
            assert!(loopback_url(ok).is_ok(), "{ok} should pass");
        }
        for bad in [
            "http://192.168.1.5:8188",
            "http://0.0.0.0:8188",
            "http://example.com",
            "http://localhost.example.com",
            "file:///tmp/x",
            "not a url",
        ] {
            assert!(loopback_url(bad).is_err(), "{bad} should be refused");
        }
        // And the tool itself cannot be built around one.
        let remote = ImageConfig {
            url: "http://10.0.0.2:8188".into(),
            ..Default::default()
        };
        assert!(ImageGenerate::new(remote).is_err());
    }

    #[test]
    fn declares_nothing_that_arms_the_interlock() {
        let t = tool("http://127.0.0.1:1");
        let caps = t.capabilities();
        assert!(!caps.private_data && !caps.untrusted_input && !caps.destructive);
        assert_eq!(caps.egress, crate::tool::Egress::None);
        // Runs in a read-only chat without an approval (the owner's ruling).
        assert!(t.read_only());
        // And the schema has nowhere to put a destination. `picture`, `mask`
        // and `scene.setting.photo` name *sources*, and each goes through
        // the path jail: reading a workspace file into a loopback server
        // sends nothing anywhere. `who` and `style` name library entries,
        // resolved by this code in the owner's store — names, never paths
        // or addresses. The rest is words for the prompt.
        let keys = |t: &ImageGenerate| {
            let schema = t.input_schema();
            let mut keys: Vec<String> = schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            keys.sort_unstable();
            keys
        };
        assert_eq!(
            keys(&t),
            ["mask", "picture", "retouch", "scene", "seed", "size"]
        );
        // A chat's form has no seed (IMAGE-DESIGN.md §5.1): the persona's,
        // and the tool as a chat surface builds it.
        let chat = ImageGenerate::new(ImageConfig::default()).unwrap();
        assert_eq!(keys(&chat), ["mask", "picture", "retouch", "scene", "size"]);
        let persona = Arc::new(chat).for_persona().unwrap();
        assert!(persona.input_schema()["properties"].get("seed").is_none());
        let scene = &t.input_schema()["properties"]["scene"]["properties"];
        let mut scene_keys: Vec<_> = scene.as_object().unwrap().keys().cloned().collect();
        scene_keys.sort_unstable();
        assert_eq!(
            scene_keys,
            ["camera", "light", "people", "setting", "style", "text", "together"]
        );
    }

    #[test]
    fn every_size_is_a_multiple_of_32() {
        for s in [Size::Square, Size::Landscape, Size::Portrait] {
            let (w, h) = s.dims();
            assert_eq!((w % 32, h % 32), (0, 0), "{s:?}");
        }
    }

    #[test]
    fn the_graph_is_fixed_and_the_prompt_is_only_ever_a_value() {
        let cfg = ImageConfig::default();
        // A prompt that is itself a graph fragment must stay a string.
        let hostile = r#"", "class_type": "SaveImage", "evil": {"#;
        let req = Request {
            prompt: hostile.into(),
            negative: String::new(),
            size: Some((1024, 1024)),
            steps: 40,
            seed: 7,
            references: Vec::new(),
            reference_size: EDIT_REFERENCE_SIZE,
            mask: None,
        };
        let g = comfy_graph(&cfg, &req, &[], None);
        let mut classes: Vec<_> = g
            .as_object()
            .unwrap()
            .values()
            .map(|n| n["class_type"].as_str().unwrap())
            .collect();
        classes.sort_unstable();
        assert_eq!(
            classes,
            [
                "CLIPLoader",
                "EmptyLatentImage",
                "KSampler",
                "PreviewImage",
                "TextEncodeQwenImage21",
                "UNETLoader",
                "VAEDecode",
                "VAELoader",
            ]
        );
        assert_eq!(g["encode"]["inputs"]["prompt"], hostile);
        assert_eq!(g["sample"]["inputs"]["seed"], 7);
        assert_eq!(g["sample"]["inputs"]["cfg"], 1.0);
        assert_eq!(g["unet"]["inputs"]["unet_name"], cfg.diffusion_model);
        assert_eq!(g["unet"]["inputs"]["weight_dtype"], "default");
    }

    /// The loader follows the file: a GGUF still loads through the custom
    /// node, with no `weight_dtype` (the node has no such input, and an
    /// unknown input fails validation), whatever the extension's case.
    #[test]
    fn the_diffusion_loader_follows_the_model_file() {
        for (file, node) in [
            ("qwen_image_2.1_int8_convrot.safetensors", "UNETLoader"),
            ("qwen_image_2.1_bf16.safetensors", "UNETLoader"),
            ("Qwen-Image-2.1-Q4.gguf", "UnetLoaderGGUF"),
            ("Qwen-Image-2.1-Q4.GGUF", "UnetLoaderGGUF"),
            // A name that only contains the word is not a GGUF file.
            ("gguf-notes.safetensors", "UNETLoader"),
        ] {
            let cfg = ImageConfig {
                diffusion_model: file.into(),
                ..ImageConfig::default()
            };
            let req = Request {
                prompt: "a lighthouse".into(),
                negative: String::new(),
                size: Some((1024, 1024)),
                steps: 40,
                seed: 7,
                references: Vec::new(),
                reference_size: EDIT_REFERENCE_SIZE,
                mask: None,
            };
            let g = comfy_graph(&cfg, &req, &[], None);
            assert_eq!(g["unet"]["class_type"], node, "{file}");
            assert_eq!(g["unet"]["inputs"]["unet_name"], file, "{file}");
            assert_eq!(
                g["unet"]["inputs"].get("weight_dtype").is_some(),
                node == "UNETLoader",
                "{file}"
            );
            assert_eq!(unet_loader(file).0, node, "{file}");
        }
    }

    #[test]
    fn an_unreadable_gauge_refuses_and_zero_disables_the_check() {
        assert!(memory_verdict(None, 16_384).is_err());
        assert!(memory_verdict(Some(8_000), 16_384).is_err());
        assert!(memory_verdict(Some(20_000), 16_384).is_ok());
        assert!(memory_verdict(None, 0).is_ok());
        let why = memory_verdict(Some(8_192), 16_384).unwrap_err();
        assert!(why.contains("8.0 GB") && why.contains("16.0 GB"), "{why}");
    }

    #[test]
    fn macos_available_memory_is_read_from_vm_stat() {
        let sample = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
            Pages free:                               65536.\n\
            Pages active:                            900000.\n\
            Pages inactive:                          131072.\n\
            Pages speculative:                        32768.\n\
            Pages throttled:                              0.\n\
            Pages wired down:                        200000.\n\
            Pages purgeable:                          16384.\n";
        // (65536 + 131072 + 32768 + 16384) pages × 16 KiB = 3840 MB.
        assert_eq!(parse_vm_stat(sample), Some(3_840));
        assert_eq!(parse_vm_stat("not vm_stat output"), None);
    }

    /// §5.2 moved the result's guidance into the description; these two
    /// prohibitions answer measured failures and must not be dropped on the
    /// way: a model copied the result over the original in 3 of 4 live chats
    /// (#429), and described pictures it had not seen (review of #581).
    #[test]
    fn the_description_keeps_the_two_prohibitions() {
        let d = tool("http://127.0.0.1:1").description().to_string();
        assert!(
            d.contains("never write or copy a result over the picture it was edited from"),
            "{d}"
        );
        assert!(d.contains("never say what a picture shows"), "{d}");
    }

    #[test]
    fn an_edit_graph_loads_its_references_from_temp_and_samples_on_their_shape() {
        let cfg = ImageConfig::default();
        let req = Request {
            prompt: "Keep <image1> unchanged except the jacket".into(),
            negative: String::new(),
            size: None,
            steps: 40,
            seed: 9,
            references: Vec::new(),
            reference_size: EDIT_REFERENCE_SIZE,
            mask: None,
        };
        let g = comfy_graph(&cfg, &req, &["a.png".into(), "b.jpg".into()], None);
        assert_eq!(g["ref1"]["class_type"], "LoadImage");
        assert_eq!(g["ref1"]["inputs"]["image"], "a.png [temp]");
        assert_eq!(g["ref2"]["inputs"]["image"], "b.jpg [temp]");
        let enc = &g["encode"]["inputs"];
        assert_eq!(enc["images.image_1"], json!(["ref1", 0]));
        assert_eq!(enc["images.image_2"], json!(["ref2", 0]));
        assert_eq!(enc["vae"], json!(["vae", 0]));
        assert_eq!(g["sample"]["inputs"]["latent_image"], json!(["encode", 2]));
        assert!(
            g.get("latent").is_none(),
            "no empty canvas when following the reference"
        );
        // A size asked for gets its own canvas even with references.
        let sized = Request {
            size: Some((1344, 768)),
            ..req
        };
        let g = comfy_graph(&cfg, &sized, &["a.png".into()], None);
        assert_eq!(g["sample"]["inputs"]["latent_image"], json!(["latent", 0]));
        assert_eq!(g["latent"]["inputs"]["width"], 1344);
        // And text-to-image has no references, no VAE on the encoder.
        let plain = Request {
            size: Some((1024, 1024)),
            ..sized
        };
        let g = comfy_graph(&cfg, &plain, &[], None);
        assert!(g.get("ref1").is_none() && g["encode"]["inputs"].get("vae").is_none());
    }

    #[test]
    fn only_real_images_are_sniffed_as_images() {
        assert_eq!(sniff_image(PNG), Some("png"));
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), Some("jpg"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff_image(b"<svg xmlns=..."), None);
        assert_eq!(sniff_image(b"#!/bin/sh"), None);
    }

    /// The memory check asks the server what it holds when the check is on,
    /// and not when it is off. Every other tool test switches the check off
    /// (`min_available_mb: 0`), which is the blind spot review of #303 found:
    /// a check reachable from no test can be miswired with the suite green.
    #[tokio::test]
    async fn the_memory_check_asks_the_server_what_it_holds() {
        let ask = |min_available_mb: u64| async move {
            let (url, seen) = fake(vec![json!({}), done()], "200 OK").await;
            let dir = tempdir();
            ImageGenerate::new(ImageConfig {
                url,
                min_available_mb,
                unload_after_secs: 0,
                ..Default::default()
            })
            .unwrap()
            .polling_every(Duration::from_millis(10))
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
            let asked = seen
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.starts_with("GET /system_stats"));
            asked
        };
        // 1 MB is available on any machine, so the verdict passes and only
        // the question is measured.
        assert!(ask(1).await, "the check never asked the server");
        assert!(!ask(0).await, "a switched-off check still asked");
    }

    #[tokio::test]
    async fn a_finished_job_lands_in_the_run_workspace() {
        let (url, seen) = fake(vec![json!({}), done()], "200 OK").await;
        let dir = tempdir();
        let out = tool(&url)
            .call(
                json!({"scene": {"setting": "a fox"}, "seed": 7}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let path = out
            .content
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("image: "))
            .expect("first line names the file");
        assert!(
            path.starts_with("images/") && path.ends_with("-7.png"),
            "{path}"
        );
        assert_eq!(std::fs::read(dir.join(path)).unwrap(), PNG);
        assert!(!out.external, "our own output is not third-party content");
        // For a run with no `image_view` — a blind provider, `--tool`, or
        // `[tools] disabled` — this fact and the description's rule (pinned in
        // `the_description_keeps_the_two_prohibitions`) are the guard against
        // it describing a picture it never saw.
        assert!(
            out.content.contains("you have not seen it."),
            "{}",
            out.content
        );
        assert!(out.image.is_none(), "a generation returns no pixels");

        let seen = seen.lock().unwrap().clone();
        let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
        assert!(submitted.contains("\"prompt\":\"a fox.\""), "{submitted}");
        let view = seen.iter().find(|l| l.starts_with("GET /view?")).unwrap();
        assert!(view.contains("type=temp"), "{view}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn two_images_in_one_second_do_not_overwrite_each_other() {
        let (url, _) = fake(vec![done(), done()], "200 OK").await;
        let dir = tempdir();
        let t = tool(&url);
        let a = t
            .call(json!({"scene": {"setting": "a"}, "seed": 1}), &ctx(&dir))
            .await
            .unwrap();
        let b = t
            .call(json!({"scene": {"setting": "b"}, "seed": 1}), &ctx(&dir))
            .await
            .unwrap();
        let first = |o: &ToolOutput| o.content.lines().next().unwrap().to_string();
        assert!(!a.is_error && !b.is_error);
        assert_ne!(first(&a), first(&b));
        let names: Vec<String> = std::fs::read_dir(dir.join("images"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let pngs: Vec<&String> = names.iter().filter(|n| n.ends_with(".png")).collect();
        assert_eq!(pngs.len(), 2);
        // Each picture has its manifest beside it.
        for png in pngs {
            let json = png.replace(".png", ".json");
            assert!(names.contains(&json), "{json} missing from {names:?}");
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// A queued picture is told apart by who, doing what, where — the
    /// scene's own words, capped for one line on a phone.
    #[test]
    fn a_queued_picture_is_labelled_by_its_scene() {
        use crate::scene::{Field, Origin, Person, Scene, Setting, Who};
        let person = |who: Who, doing: &str| Person {
            who,
            at: None,
            wearing: String::new(),
            doing: doing.into(),
            expression: String::new(),
            origin: Origin::default(),
        };
        let mut scene = Scene {
            setting: Some(Field {
                value: Setting::Words {
                    text: "a park bench in the rain".into(),
                },
                origin: Origin::default(),
            }),
            ..Scene::default()
        };
        scene.people = vec![
            person(Who::Library("maya".into()), "reading"),
            person(Who::Described("a waiter".into()), ""),
        ];
        assert_eq!(
            queue_label(&scene),
            "Maya reading, a waiter — a park bench in the rain"
        );
        assert_eq!(queue_label(&Scene::default()), "a picture");
        scene.setting = Some(Field {
            value: Setting::Words {
                text: "x".repeat(200),
            },
            origin: Origin::default(),
        });
        let long = queue_label(&scene);
        assert_eq!(long.chars().count(), 80);
        assert!(long.ends_with('…'));
    }

    /// A deferred render holds nothing of the run that started it: once the
    /// caller lets go of the run's event sender, the stream closes while the
    /// job is still waiting to run — which is when a host hands its
    /// conversation back. Fails while the job holds a clone of it.
    #[tokio::test]
    async fn a_deferred_render_does_not_hold_the_runs_events() {
        let (url, _) = fake(vec![], "200 OK").await;
        let dir = tempdir();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut c = ctx(&dir);
        c.events = Some(tx);
        let out =
            <ImageGenerate as Tool>::call(&tool(&url), json!({"scene": {"setting": "a fox"}}), &c)
                .await
                .unwrap();
        let job = out.deferred.clone().expect("deferred");
        drop((out, c));
        assert!(
            matches!(
                rx.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
            ),
            "the job still holds the run's event sender"
        );
        drop(job);
        std::fs::remove_dir_all(dir).ok();
    }

    /// The manifest names the call its picture answers, which is what the
    /// restart repair matches on.
    #[tokio::test]
    async fn the_manifest_names_the_call_it_answers() {
        let (url, _) = fake(vec![done()], "200 OK").await;
        let dir = tempdir();
        let mut c = ctx(&dir);
        c.call_id = Some("c7".into());
        let out = tool(&url)
            .call(json!({"scene": {"setting": "a fox"}, "seed": 3}), &c)
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let png = out
            .content
            .lines()
            .next()
            .unwrap()
            .strip_prefix("image: ")
            .unwrap();
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.join(png.replace(".png", ".json"))).unwrap())
                .unwrap();
        assert_eq!(manifest["tool_use_id"], "c7");
        std::fs::remove_dir_all(dir).ok();
    }

    /// A "being made" result with no job behind it is settled by the
    /// artifact: a picture counts only when a manifest names this call —
    /// wherever `save` put it — and anything else is "not made".
    #[test]
    fn a_restart_repair_trusts_only_the_jobs_own_manifest() {
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        let made = "being made: images/a.png";
        let not_made = Some(("not made: the server restarted".to_string(), true));
        assert_eq!(repair_orphan(&dir, "c1", "image: images/a.png"), None);
        // Bytes at the reserved path and no manifest: nobody was shown them.
        std::fs::write(dir.join("images/a.png"), PNG).unwrap();
        assert_eq!(repair_orphan(&dir, "c1", made), not_made);
        // A manifest naming another call, or pointing out of `images/`.
        let manifest = |name: &str, image: &str, id: &str| {
            std::fs::write(
                dir.join(format!("images/{name}.json")),
                json!({"image": image, "tool_use_id": id}).to_string(),
            )
            .unwrap()
        };
        manifest("a", "images/a.png", "c2");
        assert_eq!(repair_orphan(&dir, "c1", made), not_made);
        manifest("x", "../secret.png", "c1");
        assert_eq!(repair_orphan(&dir, "c1", made), not_made);
        // Its own, under the numbered name `save` took beside the reserved one.
        std::fs::write(dir.join("images/a-1.png"), PNG).unwrap();
        manifest("a-1", "images/a-1.png", "c1");
        let (text, is_error) = repair_orphan(&dir, "c1", made).unwrap();
        assert!(!is_error);
        assert!(text.starts_with("image: images/a-1.png\n"), "{text}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn cancelling_takes_the_job_off_the_server() {
        // History never shows the job, so only the cancel can end the call.
        let (url, seen) = fake(vec![], "200 OK").await;
        let dir = tempdir();
        let token = CancellationToken::new();
        let mut c = ctx(&dir);
        c.cancel = Some(token.clone());
        let t = tool(&url);
        let call =
            tokio::spawn(async move { t.call(json!({"scene": {"setting": "a fox"}}), &c).await });
        tokio::time::sleep(Duration::from_millis(80)).await;
        token.cancel();
        let out = call.await.unwrap().unwrap();
        assert!(
            out.is_error && out.content.starts_with("Cancelled"),
            "{}",
            out.content
        );
        let seen = seen.lock().unwrap().clone();
        assert!(seen
            .iter()
            .any(|l| l.starts_with("POST /queue") && l.contains("job-1")));
        assert!(seen
            .iter()
            .any(|l| l.starts_with("POST /interrupt") && l.contains("job-1")));
        assert!(
            seen.iter()
                .any(|l| l.starts_with("POST /history") && l.contains("job-1")),
            "a cancelled job left its prompt in the server's history"
        );
        // And the delete waited for the record: a history read sits between
        // the interrupt and the delete, or the delete raced the record.
        let at = |p: &str| seen.iter().position(|l| l.starts_with(p)).unwrap();
        let (interrupt, delete) = (at("POST /interrupt"), at("POST /history"));
        assert!(
            seen[interrupt..delete]
                .iter()
                .any(|l| l.starts_with("GET /history/job-1")),
            "the delete was sent before the interrupted job reached the history: {seen:?}"
        );
        assert!(!dir.join("images").exists(), "nothing saved");
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn cancelling_a_queued_job_never_interrupts_the_running_one() {
        // `job-1` is queued behind another call's job. Cancelling it must take
        // it off the queue and leave the running job alone — an older server
        // ignores `/interrupt`'s body and would stop whatever is executing.
        let (url, seen) = fake_running(vec![], "200 OK", false, "temp").await;
        let dir = tempdir();
        let token = CancellationToken::new();
        let mut c = ctx(&dir);
        c.cancel = Some(token.clone());
        let t = tool(&url);
        let call =
            tokio::spawn(async move { t.call(json!({"scene": {"setting": "a fox"}}), &c).await });
        tokio::time::sleep(Duration::from_millis(80)).await;
        token.cancel();
        let out = call.await.unwrap().unwrap();
        assert!(
            out.is_error && out.content.starts_with("Cancelled"),
            "{}",
            out.content
        );
        let seen = seen.lock().unwrap().clone();
        assert!(seen
            .iter()
            .any(|l| l.starts_with("POST /queue") && l.contains("job-1")));
        assert!(
            !seen.iter().any(|l| l.starts_with("POST /interrupt")),
            "a queued job's cancel interrupted a running one: {seen:?}"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_job_that_fails_still_releases_the_models() {
        // The server loaded the models before the graph failed; the memory
        // they hold is what the unload exists for.
        let failed = json!({"job-1": {"status": {"status_str": "error", "completed": false,
            "messages": [["execution_error", {"exception_message": "boom"}]]}, "outputs": {}}});
        let (url, seen) = fake(vec![failed], "200 OK").await;
        let dir = tempdir();
        let t = ImageGenerate::new(ImageConfig {
            url,
            min_available_mb: 0,
            unload_after_secs: 1,
            ..Default::default()
        })
        .unwrap()
        .polling_every(Duration::from_millis(10));
        let out = t
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.contains("boom"),
            "{}",
            out.content
        );
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|l| l.starts_with("POST /free")),
            "a failed job left the models loaded"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_brief_polling_outage_does_not_fail_the_job() {
        let (url, _) = fake(vec![json!("fail"), json!("fail"), done()], "200 OK").await;
        let dir = tempdir();
        let out = tool(&url)
            .call(
                json!({"scene": {"setting": "a fox"}, "seed": 7}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_server_that_stops_answering_polls_gets_its_job_taken_back() {
        // Returning with the job still queued would leave a generation on the
        // GPU that no client will ever collect.
        let fails = vec![json!("fail"); POLL_FAILURES as usize];
        let (url, seen) = fake(fails, "200 OK").await;
        let dir = tempdir();
        let out = tool(&url)
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.contains("stopped answering"),
            "{}",
            out.content
        );
        let seen = seen.lock().unwrap().clone();
        assert!(
            seen.iter()
                .any(|l| l.starts_with("POST /queue") && l.contains("job-1")),
            "the job was left on the server: {seen:?}"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_redirect_never_takes_a_prompt_off_the_vetted_address() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // Elsewhere: any connection here is the leak.
        let elsewhere = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let away = elsewhere.local_addr().unwrap();
        let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&reached);
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = elsewhere.accept().await {
                flag.store(true, Ordering::SeqCst);
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}")
                    .await;
            }
        });
        // The configured server answers every request with a 307 there.
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let here = server.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = server.accept().await {
                let mut buf = [0u8; 8192];
                let _ = sock.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 307 Temporary Redirect\r\nlocation: http://{away}/prompt\r\n\
                     content-length: 0\r\nconnection: close\r\n\r\n"
                );
                let _ = sock.write_all(reply.as_bytes()).await;
            }
        });
        let dir = tempdir();
        let out = tool(&format!("http://{here}"))
            .call(json!({"scene": {"setting": "private words"}}), &ctx(&dir))
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !reached.load(Ordering::SeqCst),
            "the client followed a redirect off the loopback address it was vetted for"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn an_attached_photo_is_edited_through_a_temp_upload() {
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("inbox/me.jpg"), [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3]).unwrap();
        let out = tool(&url)
            .call(
                json!({"picture": "inbox/me.jpg", "retouch": "a sunset sky"}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("A retouch of inbox/me.jpg")
                && out.content.contains("inbox/me.jpg is unchanged.")
                && out.content.contains("you have not seen it."),
            "{}",
            out.content
        );
        // The new picture's status comes first, so "it" never reads as the
        // original (review of #581).
        let at = |needle: &str| {
            out.content
                .find(needle)
                .unwrap_or_else(|| panic!("{needle:?} not in: {}", out.content))
        };
        assert!(
            at("The new picture is on the owner's screen") < at("inbox/me.jpg is unchanged."),
            "{}",
            out.content
        );
        let seen = seen.lock().unwrap().clone();
        let upload = seen
            .iter()
            .find(|l| l.starts_with("POST /upload/image"))
            .unwrap();
        assert!(
            upload.contains("name=\"type\"") && upload.contains("temp"),
            "{upload}"
        );
        let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
        assert!(submitted.contains("up.png [temp]"), "{submitted}");
        assert!(
            std::fs::read(dir.join("inbox/me.jpg"))
                .unwrap()
                .starts_with(&[0xFF, 0xD8]),
            "the original is untouched"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// The fit is wired where its comment says: a photo over the budget goes
    /// up as the fitted PNG — no EXIF block, so no location — and the repeat
    /// guard and the near-copy check run over those bytes, not the file's.
    #[tokio::test]
    async fn a_large_photo_goes_up_fitted_and_upright() {
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        let photo = jpeg_with_orientation(300, 100, 6);
        assert!(String::from_utf8_lossy(&photo).contains("Exif"));
        std::fs::write(dir.join("inbox/big.jpg"), &photo).unwrap();
        let out = tool(&url)
            .with_reference_pixels(10_000)
            .call(
                json!({"picture": "inbox/big.jpg", "retouch": "a sunset sky"}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let seen = seen.lock().unwrap().clone();
        let upload = seen
            .iter()
            .find(|l| l.starts_with("POST /upload/image"))
            .unwrap();
        assert!(upload.contains("PNG\r\n"), "sent as a PNG: {upload:.200}");
        assert!(!upload.contains("Exif"), "the EXIF block stays behind");
        assert!(!upload.contains("JFIF"), "not the original JPEG");
        assert_eq!(
            std::fs::read(dir.join("inbox/big.jpg")).unwrap(),
            photo,
            "the original is untouched"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_reference_outside_the_workspace_or_not_an_image_is_refused_before_any_upload() {
        let (url, seen) = fake(vec![], "200 OK").await;
        let dir = tempdir();
        std::fs::write(dir.join("notes.txt"), "just text").unwrap();
        let t = tool(&url);
        for bad in [
            "../outside.png",
            "/etc/hostname",
            "notes.txt",
            "missing.png",
        ] {
            let out = t
                .call(json!({"picture": bad, "retouch": "x"}), &ctx(&dir))
                .await
                .unwrap();
            assert!(
                out.is_error && out.content.contains(bad),
                "{bad}: {}",
                out.content
            );
        }
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("/upload/image") || l.contains("/prompt")),
            "nothing reached the server"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// A seed draws a new picture only; a change to one samples afresh
    /// (#306), and a chat's schema has no seed at all (IMAGE-DESIGN.md §5.1).
    #[tokio::test]
    async fn a_seed_beside_a_picture_is_refused_before_the_gpu() {
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::write(dir.join("images/20260925-142604-7.png"), PNG).unwrap();
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let out = tool(&url)
            .call(
                json!({"picture": "images/20260925-142604-7.png", "retouch": "a yellow raincoat",
                       "seed": 7}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.starts_with("Nothing was drawn. "),
            "{}",
            out.content
        );
        assert!(!seen
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("POST /prompt")));
        // And the chat's form refuses a seed outright.
        let chat = ImageGenerate::new(ImageConfig {
            url: url.clone(),
            min_available_mb: 0,
            ..Default::default()
        })
        .unwrap()
        .with_library_dir(tempdir());
        let out = chat
            .call(
                json!({"scene": {"setting": "a fox"}, "seed": 7}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.contains("`seed` is not part of this tool"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// The tool, deleting the server's temp copies in `temp`.
    fn tool_in(url: &str, temp: &std::path::Path) -> ImageGenerate {
        ImageGenerate::new(ImageConfig {
            url: url.into(),
            min_available_mb: 0,
            unload_after_secs: 0,
            server_temp_dir: Some(temp.to_path_buf()),
            ..Default::default()
        })
        .unwrap()
        .polling_every(Duration::from_millis(10))
    }

    /// A finished job whose one preview is `filename`.
    fn done_as(filename: &str) -> Value {
        json!({"job-1": {
            "status": {"status_str": "success", "completed": true},
            "outputs": {"out": {"images": [
                {"filename": filename, "subfolder": "", "type": "temp"}
            ]}}
        }})
    }

    /// A workspace holding one photo to edit, and a temp directory holding
    /// what the fake server says it wrote — plus a file nobody named.
    fn edit_scene() -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("inbox/me.jpg"), [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3]).unwrap();
        let temp = dir.join("server-temp");
        std::fs::create_dir_all(&temp).unwrap();
        for f in ["up.png", "a_temp_00001_.png", "someone-elses.png"] {
            std::fs::write(temp.join(f), "x").unwrap();
        }
        (dir, temp)
    }

    /// The id this call minted, as sent in `POST /prompt`.
    fn minted(seen: &[String]) -> String {
        let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
        let body: Value = serde_json::from_str(
            submitted
                .split_once(' ')
                .and_then(|(_, rest)| rest.split_once(' '))
                .and_then(|(_, rest)| rest.split_once(' '))
                .map(|(_, body)| body)
                .unwrap_or_default(),
        )
        .unwrap();
        body["prompt_id"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn a_preview_filed_anywhere_but_temp_is_said_rather_than_skipped() {
        let elsewhere = json!({"job-1": {
            "status": {"status_str": "success", "completed": true},
            "outputs": {"out": {"images": [
                {"filename": "o_00001_.png", "subfolder": "", "type": "output"}
            ]}}
        }});
        let (url, _) = fake(vec![elsewhere], "200 OK").await;
        let dir = tempdir();
        let temp = dir.join("server-temp");
        std::fs::create_dir_all(&temp).unwrap();
        let out = tool_in(&url, &temp)
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
        assert!(
            out.content.contains("o_00001_.png")
                && out.content.contains("filed as `output`")
                && out.content.contains("could not all be removed"),
            "a copy filed where nothing clears it went unsaid: {}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_cancelled_jobs_preview_is_written_down_before_its_record_goes() {
        // Cancelled just as the server finished: the preview exists, and only
        // the record names it. It must reach the trail before the record is
        // forgotten, and be deleted.
        let (url, _) = fake_with(Fake {
            interrupted_record: Some(done()),
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        let temp = dir.join("server-temp");
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(temp.join("a_temp_00001_.png"), "x").unwrap();
        let trail = dir.join("image-trail");
        let token = CancellationToken::new();
        let mut c = ctx(&dir);
        c.cancel = Some(token.clone());
        c.image_trail = Some(trail.clone());
        let t = tool_in(&url, &temp);
        let call =
            tokio::spawn(async move { t.call(json!({"scene": {"setting": "a fox"}}), &c).await });
        tokio::time::sleep(Duration::from_millis(80)).await;
        token.cancel();
        let out = call.await.unwrap().unwrap();
        assert!(out.content.starts_with("Cancelled"), "{}", out.content);
        assert!(
            read_trail(&trail).contains(&TrailEntry::File("a_temp_00001_.png".into())),
            "a cancelled job's preview was never written down: {:?}",
            read_trail(&trail)
        );
        assert!(
            !temp.join("a_temp_00001_.png").exists(),
            "the preview stayed"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_job_accepted_without_an_id_is_still_taken_back() {
        let (url, seen) = fake_with(Fake {
            prompt_body: Some(json!({})),
            running: false,
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        let out = tool(&url)
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
        assert!(
            out.content.contains("named no prompt_id"),
            "{}",
            out.content
        );
        let seen = seen.lock().unwrap().clone();
        let id = minted(&seen);
        assert!(
            seen.iter()
                .any(|l| l.starts_with("POST /queue") && l.contains(&id)),
            "a job the server accepted was left queued: {seen:?}"
        );
        assert!(
            seen.iter()
                .any(|l| l.starts_with("POST /history") && l.contains(&id)),
            "a job the server accepted kept its record: {seen:?}"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_job_whose_answer_was_lost_is_still_taken_back() {
        let (url, seen) = fake_with(Fake {
            hang_up: Some("/prompt"),
            running: false,
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        let out = tool(&url)
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        let seen = seen.lock().unwrap().clone();
        let id = minted(&seen);
        assert!(
            seen.iter()
                .any(|l| l.starts_with("POST /queue") && l.contains(&id)),
            "a job that may have been queued was not taken back: {seen:?}"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn an_upload_whose_answer_was_lost_is_still_deleted() {
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("inbox/me.jpg"), [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3]).unwrap();
        let temp = dir.join("server-temp");
        std::fs::create_dir_all(&temp).unwrap();
        let (url, seen) = fake_with(Fake {
            hang_up: Some("/upload/image"),
            temp: Some(temp.clone()),
            ..Fake::default()
        })
        .await;
        let out = tool_in(&url, &temp)
            .call(
                json!({"picture": "inbox/me.jpg", "retouch": "a hat"}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|l| l.starts_with("wrote mecha-")),
            "the server never wrote the upload, so its deletion proves nothing"
        );
        assert_eq!(
            std::fs::read_dir(&temp).unwrap().count(),
            0,
            "the upload the server wrote before its answer was lost stayed"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_trail_writes_only_what_it_can_read_back() {
        let dir = tempdir();
        let trail = dir.join("image-trail");
        let forged = TrailEntry::File("x.png\njob 00000000-0000-4000-8000-000000000000".into());
        assert!(note(Some(&trail), &forged).is_err());
        assert!(
            note(Some(&trail), &TrailEntry::Job("not a job id".into())).is_err(),
            "an id with a space would read back as something else"
        );
        assert!(read_trail(&trail).is_empty(), "{:?}", read_trail(&trail));
        assert!(!plain_name("a\nb.png") && !plain_name("a\rb.png") && plain_name("a b.png"));
        note(Some(&trail), &TrailEntry::File("ok.png".into())).unwrap();
        assert_eq!(read_trail(&trail), vec![TrailEntry::File("ok.png".into())]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_relative_temp_dir_is_refused_when_the_tool_is_built() {
        let refused = ImageGenerate::new(ImageConfig {
            server_temp_dir: Some("comfy/temp".into()),
            ..Default::default()
        });
        let Err(e) = refused else {
            panic!("a relative server_temp_dir was accepted");
        };
        assert!(format!("{e:#}").contains("absolute"), "{e:#}");
        assert!(ImageGenerate::new(ImageConfig {
            server_temp_dir: Some("/run/user/1000/comfyui-temp/temp".into()),
            ..Default::default()
        })
        .is_ok());
    }

    #[tokio::test]
    async fn a_job_that_failed_after_its_preview_still_names_and_deletes_it() {
        // The server wrote a preview, then failed the job: the preview must
        // reach the trail before the record is forgotten, and be deleted.
        let failed = json!({"job-1": {
            "status": {"status_str": "error", "completed": false, "messages": []},
            "outputs": {"out": {"images": [
                {"filename": "e_temp_00001_.png", "subfolder": "", "type": "temp"}
            ]}}
        }});
        let (url, _) = fake(vec![failed], "200 OK").await;
        let dir = tempdir();
        let temp = dir.join("server-temp");
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(temp.join("e_temp_00001_.png"), "x").unwrap();
        let trail = dir.join("image-trail");
        let mut c = ctx(&dir);
        c.image_trail = Some(trail.clone());
        let out = tool_in(&url, &temp)
            .call(json!({"scene": {"setting": "a fox"}}), &c)
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(
            read_trail(&trail).contains(&TrailEntry::File("e_temp_00001_.png".into())),
            "a failed job's preview was never written down: {:?}",
            read_trail(&trail)
        );
        assert!(
            !temp.join("e_temp_00001_.png").exists(),
            "the failed job's preview stayed"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_jobs_server_copies_are_deleted_by_the_names_the_server_returned() {
        let (url, _) = fake(vec![done()], "200 OK").await;
        let (dir, temp) = edit_scene();
        let out = tool_in(&url, &temp)
            .call(
                json!({"picture": "inbox/me.jpg", "retouch": "a hat"}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            !temp.join("up.png").exists(),
            "the uploaded reference stayed"
        );
        assert!(
            !temp.join("a_temp_00001_.png").exists(),
            "the preview stayed"
        );
        assert!(
            temp.join("someone-elses.png").exists(),
            "a file nobody named was touched"
        );
        assert!(!out.content.contains("could not"), "{}", out.content);
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_name_that_is_not_one_plain_component_is_never_deleted() {
        let (url, _) = fake(vec![done_as("../escape.png")], "200 OK").await;
        let dir = tempdir();
        let temp = dir.join("server-temp");
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(dir.join("escape.png"), "keep me").unwrap();
        let out = tool_in(&url, &temp)
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
        assert!(
            dir.join("escape.png").exists(),
            "a server's name walked out of the directory"
        );
        assert!(
            out.content.contains("not one plain file name"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_cancelled_edit_still_deletes_its_upload() {
        // History never shows the job, so only the cancel ends the call.
        let (url, _) = fake(vec![], "200 OK").await;
        let (dir, temp) = edit_scene();
        let token = CancellationToken::new();
        let mut c = ctx(&dir);
        c.cancel = Some(token.clone());
        let t = tool_in(&url, &temp);
        let call = tokio::spawn(async move {
            t.call(json!({"picture": "inbox/me.jpg", "retouch": "a hat"}), &c)
                .await
        });
        tokio::time::sleep(Duration::from_millis(80)).await;
        token.cancel();
        let out = call.await.unwrap().unwrap();
        assert!(out.content.starts_with("Cancelled"), "{}", out.content);
        assert!(
            !temp.join("up.png").exists(),
            "a cancelled edit left its upload"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_copy_the_directory_does_not_hold_is_said_rather_than_skipped() {
        // The server says it wrote a preview; the configured directory has
        // none — the wrong directory, and a deletion that would quietly never
        // happen.
        let (url, _) = fake(vec![done()], "200 OK").await;
        let dir = tempdir();
        let temp = dir.join("not-the-servers");
        std::fs::create_dir_all(&temp).unwrap();
        let out = tool_in(&url, &temp)
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
        assert!(!out.is_error, "the image itself was made: {}", out.content);
        assert!(
            out.content.contains("could not all be removed")
                && out.content.contains("a_temp_00001_.png")
                && out.content.contains("server_temp_dir"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn the_trail_names_the_job_and_every_file_and_the_server_gets_that_id() {
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let (dir, temp) = edit_scene();
        let trail = dir.join("image-trail");
        let mut c = ctx(&dir);
        c.image_trail = Some(trail.clone());
        let out = tool_in(&url, &temp)
            .call(json!({"picture": "inbox/me.jpg", "retouch": "a hat"}), &c)
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let entries = read_trail(&trail);
        let jobs: Vec<&String> = entries
            .iter()
            .filter_map(|e| match e {
                TrailEntry::Job(id) => Some(id),
                _ => None,
            })
            .collect();
        let files: Vec<&String> = entries
            .iter()
            .filter_map(|e| match e {
                TrailEntry::File(name) => Some(name),
                _ => None,
            })
            .collect();
        // Minted here, and sent: the server was told the id already written.
        let minted = jobs.first().expect("a job was recorded");
        assert!(uuid::Uuid::parse_str(minted).is_ok(), "{minted}");
        let submitted = seen
            .lock()
            .unwrap()
            .iter()
            .find(|l| l.starts_with("POST /prompt"))
            .cloned()
            .unwrap();
        assert!(
            submitted.contains(&format!("\"prompt_id\":\"{minted}\"")),
            "{submitted}"
        );
        // This fake answers with its own id, as an older server does; that
        // one is recorded too.
        assert!(jobs.iter().any(|j| j.as_str() == "job-1"), "{entries:?}");
        // The name asked for, the name filed under, and the preview.
        assert!(
            files
                .iter()
                .any(|f| f.starts_with("mecha-") && f.ends_with(".jpg")),
            "{entries:?}"
        );
        assert!(files.iter().any(|f| f.as_str() == "up.png"), "{entries:?}");
        assert!(
            files.iter().any(|f| f.as_str() == "a_temp_00001_.png"),
            "{entries:?}"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_trail_that_cannot_be_written_stops_the_job_before_the_server_has_it() {
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let (dir, temp) = edit_scene();
        let mut c = ctx(&dir);
        // A room that has gone: the chat closed, so the job must not start.
        c.image_trail = Some(dir.join("gone-room").join("image-trail"));
        let out = tool_in(&url, &temp)
            .call(json!({"picture": "inbox/me.jpg", "retouch": "a hat"}), &c)
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.contains("could not be recorded for cleanup"),
            "{}",
            out.content
        );
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.starts_with("POST /upload") || l.starts_with("POST /prompt")),
            "the server received something no trail named"
        );
        assert!(
            !dir.join("gone-room").exists(),
            "the trail recreated a closed room"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_dead_writers_trail_is_taken_back_off_the_server() {
        // The job is not running (a serve died after it finished), so what it
        // wrote is found in its record, deleted, and the record forgotten.
        let (url, seen) = fake_running(vec![done()], "200 OK", false, "temp").await;
        let (dir, temp) = edit_scene();
        let cfg = ImageConfig {
            url,
            server_temp_dir: Some(temp.clone()),
            ..Default::default()
        };
        forget_trail(
            &cfg,
            &[
                TrailEntry::Job("job-1".into()),
                TrailEntry::File("up.png".into()),
                // Recorded before an upload that never happened: not a failure.
                TrailEntry::File("mecha-00000000deadbeef.jpg".into()),
            ],
        )
        .await
        .unwrap();
        assert!(!temp.join("up.png").exists());
        assert!(
            !temp.join("a_temp_00001_.png").exists(),
            "the job's preview stayed"
        );
        assert!(temp.join("someone-elses.png").exists());
        let seen = seen.lock().unwrap().clone();
        assert!(seen
            .iter()
            .any(|l| l.starts_with("POST /queue") && l.contains("job-1")));
        assert!(
            seen.iter()
                .any(|l| l.starts_with("POST /history") && l.contains("job-1")),
            "the record, prompt and all, stayed on the server"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_trail_against_a_server_that_is_down_says_so() {
        // A port nobody is listening on.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let cfg = ImageConfig {
            url: format!("http://127.0.0.1:{port}"),
            ..Default::default()
        };
        let err = forget_trail(&cfg, &[TrailEntry::Job("job-1".into())])
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("did not answer"), "{err:#}");
    }

    #[test]
    fn a_trail_reads_back_only_what_this_module_writes() {
        let dir = tempdir();
        let trail = dir.join("image-trail");
        std::fs::write(
            &trail,
            "job 0b9e1c2a-4f3d-4a7e-9d5c-2f1e0a9b8c7d\nfile up.png\nfile ../../etc/passwd\n\
             file a/b.png\njob ;rm -rf\nsomething else\nfile \n",
        )
        .unwrap();
        assert_eq!(
            read_trail(&trail),
            vec![
                TrailEntry::Job("0b9e1c2a-4f3d-4a7e-9d5c-2f1e0a9b8c7d".into()),
                TrailEntry::File("up.png".into()),
            ]
        );
        assert!(read_trail(&dir.join("absent")).is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_finished_job_is_dropped_from_the_servers_history() {
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let dir = tempdir();
        let out = tool(&url)
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|l| l.starts_with("POST /history") && l.contains("job-1")),
            "the prompt was left in the server's history"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_fifo_named_as_a_reference_is_refused_rather_than_waited_on() {
        use std::os::unix::fs::OpenOptionsExt;
        let (url, _) = fake(vec![], "200 OK").await;
        let dir = tempdir();
        let fifo = dir.join("pipe.png");
        let cpath = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path; mkfifo only creates the node.
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
        let t = tool(&url);
        let c = ctx(&dir);
        let call = t.call(json!({"picture": "pipe.png", "retouch": "edit"}), &c);
        let out = match tokio::time::timeout(Duration::from_secs(5), call).await {
            Ok(out) => out.unwrap(),
            Err(_) => {
                // Release the reader stuck in `open` so the runtime can shut
                // down, then fail rather than hang the suite.
                let _ = std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&fifo);
                panic!("opening a FIFO blocked the call");
            }
        };
        assert!(
            out.is_error && out.content.contains("pipe.png"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn an_upload_filed_anywhere_but_temp_stops_the_job() {
        let (url, seen) = fake_running(vec![done()], "200 OK", true, "input").await;
        let dir = tempdir();
        std::fs::write(dir.join("me.png"), PNG).unwrap();
        let out = tool(&url)
            .call(json!({"picture": "me.png", "retouch": "edit"}), &ctx(&dir))
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.contains("rather than temp"),
            "{}",
            out.content
        );
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.starts_with("POST /prompt")),
            "a job ran on a reference the server kept"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_rejected_job_says_why() {
        let (url, _) = fake(vec![], "400 Bad Request").await;
        let dir = tempdir();
        let out = tool(&url)
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.contains("bad node"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_missing_model_file_is_named_before_anything_is_submitted() {
        let (url, seen) = fake(vec![], "200 OK").await;
        let dir = tempdir();
        let t = ImageGenerate::new(ImageConfig {
            url,
            diffusion_model: "nope.gguf".into(),
            min_available_mb: 0,
            unload_after_secs: 0,
            ..Default::default()
        })
        .unwrap();
        let out = t
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
        assert!(
            out.is_error
                && out.content.contains("nope.gguf")
                && out.content.contains("Qwen-Image-2.1-Q4.gguf"),
            "{}",
            out.content
        );
        assert!(!seen
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("POST /prompt")));
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_server_that_is_down_is_named_as_such() {
        // Bind then drop, so the port is closed.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let dir = tempdir();
        let out = tool(&format!("http://127.0.0.1:{port}"))
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.contains("Is the image server running"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// A library holding approved characters, each with a real (decodable)
    /// portrait and a known source seed, 900 upward.
    fn library_with(names: &[&str]) -> std::path::PathBuf {
        let dir = tempdir();
        for (i, name) in names.iter().enumerate() {
            let img = image::RgbImage::from_pixel(2, 2, image::Rgb([40 * i as u8, 10, 10]));
            let mut png = std::io::Cursor::new(Vec::new());
            img.write_to(&mut png, image::ImageFormat::Png).unwrap();
            crate::imagelib::create(
                &dir,
                crate::imagelib::NewEntry {
                    kind: crate::imagelib::Kind::Character,
                    name: name.to_string(),
                    text: format!("{name}, a memorable face"),
                    portrait: Some(png.into_inner()),
                    source_seed: Some(900 + i as u64),
                    origin: crate::imagelib::Origin::Owner,
                    locked: false,
                },
            )
            .unwrap();
        }
        dir
    }

    fn two_people() -> Value {
        json!([
            {"who": "maya", "wearing": "a yellow raincoat", "doing": "laughing"},
            {"who": "john", "wearing": "a flannel shirt", "doing": "smiling"}
        ])
    }

    #[tokio::test]
    async fn a_cast_compiles_into_portraits_at_512_and_a_manifest() {
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let dir = tempdir();
        let lib = library_with(&["maya", "john"]);
        let t = tool(&url).with_library_dir(lib.clone());
        let out = t
            .call(
                json!({"scene": {"setting": "a diner booth at night", "people": two_people()}, "seed": 5}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content
                .contains("with character maya (v1), character john (v1)"),
            "{}",
            out.content
        );
        // Not an edit: the seed the model chose is kept, so a scene can be
        // revised with its composition.
        assert!(out.content.contains("(seed 5,"), "{}", out.content);

        let seen = seen.lock().unwrap().clone();
        let uploads = seen
            .iter()
            .filter(|l| l.starts_with("POST /upload/image"))
            .count();
        assert_eq!(uploads, 2, "one portrait per person");
        let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
        assert!(submitted.contains("\"resolution\":512"), "{submitted}");
        assert!(
            submitted.contains("<image1> (maya, a memorable face)"),
            "{submitted}"
        );
        assert!(
            submitted.contains("wearing a flannel shirt, smiling"),
            "{submitted}"
        );
        assert!(
            submitted.contains("Each of the two people from the images appears exactly once"),
            "{submitted}"
        );
        // A portrait is not a canvas: the default is square, not its shape.
        assert!(submitted.contains("\"width\":1024"), "{submitted}");

        let png = out
            .content
            .lines()
            .next()
            .unwrap()
            .strip_prefix("image: ")
            .unwrap();
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.join(png.replace(".png", ".json"))).unwrap())
                .unwrap();
        assert!(submitted.contains("<image2> (john"), "{submitted}");
        assert!(submitted.contains("a diner booth at night."), "{submitted}");
        // The manifest keeps what save-to-library reads, and no prompt.
        assert_eq!(manifest["seed"], 5);
        assert_eq!(manifest["cast"][0]["name"], "maya");
        assert_eq!(manifest["cast"][0]["version"], 1);
        assert!(manifest["cast"][0]["portrait"]
            .as_str()
            .unwrap()
            .starts_with("sha256-"));
        assert_eq!(manifest["route"], "new");
        assert!(manifest.get("prompt").is_none(), "{manifest}");
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    #[tokio::test]
    async fn the_seed_that_drew_a_portrait_is_never_sampled_for_its_scene() {
        let (url, _) = fake(vec![done(), done()], "200 OK").await;
        let dir = tempdir();
        let lib = library_with(&["maya", "john"]);
        let t = tool(&url).with_library_dir(lib.clone());
        // 901 drew john's portrait.
        let out = t
            .call(
                json!({"scene": {"setting": "a park", "people": two_people()}, "seed": 901}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("Seed 901 was not used"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("(seed 901,"), "{}", out.content);
        // Reseeded each time, so the same call again is another picture.
        let again = t
            .call(
                json!({"scene": {"setting": "a park", "people": two_people()}, "seed": 901}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(again.content.starts_with("image: "), "{}", again.content);
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    #[tokio::test]
    async fn a_candidate_never_reaches_the_server() {
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let dir = tempdir();
        let lib = library_with(&["maya"]);
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]));
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        crate::imagelib::create(
            &lib,
            crate::imagelib::NewEntry {
                kind: crate::imagelib::Kind::Character,
                name: "john".into(),
                text: "john, proposed by a model".into(),
                portrait: Some(png.into_inner()),
                source_seed: None,
                origin: crate::imagelib::Origin::ModelClean,
                locked: false,
            },
        )
        .unwrap();
        let t = tool(&url).with_library_dir(lib.clone());
        let out = t
            .call(
                json!({"scene": {"setting": "a park", "people": two_people()}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(out.is_error && out.content.starts_with("Nothing was drawn. "));
        assert!(
            out.content
                .contains("John is in the image library but waiting for the owner's approval"),
            "{}",
            out.content
        );
        assert!(!seen
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("POST /prompt")));
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A face detector that panics costs the crops and nothing else: the
    /// person is still drawn and kept in the manifest's `cast` (review of
    /// #586), and the result says why there was no crop (review of #597).
    #[tokio::test]
    async fn a_panicking_detector_keeps_the_people_on_record() {
        struct Panics;
        impl crate::face::FaceAnchors for Panics {
            fn anchor(
                &self,
                _: &crate::imagelib::Library,
                _: &crate::imagelib::Entry,
            ) -> crate::face::Anchor {
                panic!("detector down")
            }
        }
        let (url, _) = fake(vec![done()], "200 OK").await;
        let dir = tempdir();
        let lib = library_with(&["maya"]);
        let t = tool(&url)
            .with_library_dir(lib.clone())
            .with_faces(Arc::new(Panics));
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("inbox/room.png"), PNG).unwrap();
        let out = t
            .call(
                json!({"picture": "inbox/room.png",
                       "scene": {"people": [{"who": "maya", "wearing": "a coat", "doing": "sitting on the bench"}]}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let m = manifest_of(&dir, &out.content);
        assert_eq!(m["cast"][0]["name"], "maya", "{m}");
        assert_eq!(m["crops"], json!([]), "{m}");
        // And the reason is said, never a silent draw without the face.
        assert!(
            out.content.contains(
                "Maya's face could not be taken from the library (the face detector failed)"
            ),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    #[test]
    fn every_word_of_a_name_is_capitalised() {
        assert_eq!(capitalized("mara quinn"), "Mara Quinn");
        assert_eq!(capitalized("maya"), "Maya");
    }

    /// A 64×64 picture: a `figure`-coloured block standing at `x0` on a
    /// nearly flat ground, so where the figure is carries the layout.
    fn picture(x0: u32, figure: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_fn(64, 64, |x, y| {
            if (x0..x0 + 16).contains(&x) && (16..56).contains(&y) {
                image::Rgb(figure)
            } else {
                let g = 60 + (y / 2) as u8;
                image::Rgb([g / 2, g, g / 2 + 30])
            }
        });
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        png.into_inner()
    }

    fn manifest_of(dir: &std::path::Path, content: &str) -> Value {
        let png = content
            .lines()
            .next()
            .unwrap()
            .strip_prefix("image: ")
            .unwrap();
        serde_json::from_slice(&std::fs::read(dir.join(png.replace(".png", ".json"))).unwrap())
            .unwrap()
    }

    #[test]
    fn layout_similarity_reads_where_things_are_not_their_colour() {
        let sat = picture(8, [240, 220, 40]);
        let same = layout_similarity(&sat, &sat).unwrap();
        assert!(same > 0.99, "{same}");
        // Recoloured in place keeps the layout, and scores as a near-copy —
        // which is why the notice says what did not change, not "failed".
        let recoloured = layout_similarity(&sat, &picture(8, [200, 240, 200])).unwrap();
        assert!(recoloured >= NEAR_COPY_LAYOUT, "{recoloured}");
        let moved = layout_similarity(&sat, &picture(40, [240, 220, 40])).unwrap();
        assert!(moved < NEAR_COPY_LAYOUT, "{moved}");
        // No reading is not a low reading.
        let mut flat = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(8, 8, image::Rgb([9, 9, 9]))
            .write_to(&mut flat, image::ImageFormat::Png)
            .unwrap();
        assert_eq!(layout_similarity(&sat, flat.get_ref()), None);
        assert_eq!(layout_similarity(&sat, PNG), None);
    }

    /// A `w`×`h` mask, white inside `(x0, y0, x1, y1)`.
    fn mask_png(w: u32, h: u32, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> Vec<u8> {
        let img = image::GrayImage::from_fn(w, h, |x, y| {
            image::Luma([if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
                255
            } else {
                0
            }])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        png.into_inner()
    }

    #[test]
    fn the_edit_canvas_is_the_encoders_own_sizing() {
        // `TextEncodeQwenImage21` at resolution 1024: about a megapixel, the
        // aspect kept, multiples of 32 — checked against the node's source.
        assert_eq!(edit_canvas(1344, 768, 1024), (1344, 768));
        assert_eq!(edit_canvas(1024, 1024, 1024), (1024, 1024));
        assert_eq!(edit_canvas(768, 1344, 1024), (768, 1344));
        assert_eq!(edit_canvas(4000, 3000, 1024), (1184, 896));
        assert_eq!(edit_canvas(64, 64, 1024), (1024, 1024));
    }

    #[test]
    fn a_masked_graph_resamples_only_under_the_mask() {
        let cfg = ImageConfig::default();
        let req = Request {
            prompt: "Make the dress green.".into(),
            negative: String::new(),
            size: None,
            steps: 40,
            seed: 9,
            references: Vec::new(),
            reference_size: EDIT_REFERENCE_SIZE,
            mask: None,
        };
        let g = comfy_graph(&cfg, &req, &["pic.png".into()], Some("m.png"));
        assert_eq!(g["maskimg"]["inputs"]["image"], "m.png [temp]");
        assert_eq!(g["mask"]["class_type"], "ImageToMask");
        assert_eq!(g["source"]["class_type"], "VAEEncode");
        assert_eq!(g["source"]["inputs"]["pixels"], json!(["ref1", 0]));
        assert_eq!(g["masked"]["class_type"], "SetLatentNoiseMask");
        assert_eq!(g["sample"]["inputs"]["latent_image"], json!(["masked", 0]));
        // The mask is no reference: the encoder sees the picture alone.
        let enc = &g["encode"]["inputs"];
        assert_eq!(enc["images.image_1"], json!(["ref1", 0]));
        assert!(enc.get("images.image_2").is_none(), "{enc}");
        // Without one, nothing of it is in the graph.
        let g = comfy_graph(&cfg, &req, &["pic.png".into()], None);
        assert!(g.get("masked").is_none() && g.get("maskimg").is_none());
    }

    #[test]
    fn a_prepared_mask_grows_feathers_and_refuses_nothing_or_a_misfit() {
        let picture = picture(8, [240, 220, 40]);
        let plan = prepare_mask(&picture, &mask_png(64, 64, (0, 16, 16, 56)), 1024).unwrap();
        assert_eq!(plan.picture.dimensions(), (1024, 1024));
        assert_eq!(plan.soft.dimensions(), (1024, 1024));
        // Painted pixels are fully redrawn; the edge is grown, then feathered;
        // far away nothing is. The box is 0..256 × 256..896 at canvas size;
        // upscaling the 64 px mask 16× leaves it a soft edge, which counts as
        // painted from 16 up, 7 px each side.
        assert_eq!(plan.bounds, (0, 249, 263, 903));
        assert_eq!(plan.soft.get_pixel(128, 576).0[0], 255);
        let just_outside = plan.soft.get_pixel(266, 576).0[0];
        assert!(just_outside > 0 && just_outside < 255, "{just_outside}");
        assert_eq!(plan.soft.get_pixel(800, 576).0[0], 0);
        assert_eq!(plan.soft.get_pixel(128, 100).0[0], 0);

        let err = prepare_mask(&picture, &mask_png(64, 64, (0, 0, 0, 0)), 1024).unwrap_err();
        assert!(err.contains("marks nothing"), "{err}");
        let err = prepare_mask(&picture, &mask_png(128, 64, (0, 0, 10, 10)), 1024).unwrap_err();
        assert!(err.contains("painted over the picture it edits"), "{err}");
    }

    #[test]
    fn the_composite_keeps_every_unpainted_pixel_exactly() {
        let picture = picture(8, [240, 220, 40]);
        let plan = prepare_mask(&picture, &mask_png(64, 64, (0, 16, 16, 56)), 1024).unwrap();
        let green = image::RgbImage::from_pixel(64, 64, image::Rgb([10, 200, 30]));
        let mut result = std::io::Cursor::new(Vec::new());
        green
            .write_to(&mut result, image::ImageFormat::Png)
            .unwrap();
        let out = composite_masked(result.get_ref(), &plan).unwrap();
        let out = image::load_from_memory(&out).unwrap().to_rgb8();
        assert_eq!(out.dimensions(), (1024, 1024));
        // Outside the grown, feathered edge: the original, to the byte.
        for (x, y) in [(800, 576), (512, 100), (1000, 1000), (300, 50)] {
            assert_eq!(
                out.get_pixel(x, y),
                plan.picture.get_pixel(x, y),
                "({x},{y})"
            );
        }
        // Inside what was painted: the result.
        assert_eq!(out.get_pixel(128, 576).0, [10, 200, 30]);
    }

    #[tokio::test]
    async fn a_masked_edit_uploads_the_mask_and_keeps_the_rest_of_the_picture() {
        let green = image::RgbImage::from_pixel(64, 64, image::Rgb([10, 200, 30]));
        let mut result = std::io::Cursor::new(Vec::new());
        green
            .write_to(&mut result, image::ImageFormat::Png)
            .unwrap();
        let (url, seen) = fake_with(Fake {
            history: vec![done()],
            views: vec![result.into_inner()],
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        let original = picture(8, [240, 220, 40]);
        std::fs::write(dir.join("images/orig.png"), &original).unwrap();
        std::fs::write(
            dir.join("inbox/mask.png"),
            mask_png(64, 64, (0, 16, 16, 56)),
        )
        .unwrap();
        let out = tool(&url)
            .call(
                json!({"picture": "images/orig.png", "retouch": "Make her dress green.",
                       "mask": "inbox/mask.png", "size": "landscape"}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content
                .contains("Only the area painted in inbox/mask.png was redrawn")
                && out.content.contains("The size asked for was not used"),
            "{}",
            out.content
        );
        let seen = seen.lock().unwrap().clone();
        let uploads = seen
            .iter()
            .filter(|l| l.starts_with("POST /upload/image"))
            .count();
        assert_eq!(uploads, 2, "the picture and its mask");
        let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
        assert!(submitted.contains("SetLatentNoiseMask"), "{submitted}");
        // The saved picture: the original outside the mask, the result inside.
        let png = out
            .content
            .lines()
            .next()
            .unwrap()
            .strip_prefix("image: ")
            .unwrap();
        let saved = image::load_from_memory(&std::fs::read(dir.join(png)).unwrap())
            .unwrap()
            .to_rgb8();
        let canvas = prepare_mask(&original, &mask_png(64, 64, (0, 16, 16, 56)), 1024).unwrap();
        assert_eq!(
            saved.get_pixel(800, 576),
            canvas.picture.get_pixel(800, 576)
        );
        assert_eq!(saved.get_pixel(128, 576).0, [10, 200, 30]);
        assert_eq!(manifest_of(&dir, &out.content)["mask"], "inbox/mask.png");
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn an_empty_mask_is_refused_before_the_gpu() {
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("images/orig.png"), picture(8, [240, 220, 40])).unwrap();
        std::fs::write(dir.join("inbox/mask.png"), mask_png(64, 64, (0, 0, 0, 0))).unwrap();
        let out = tool(&url)
            .call(
                json!({"picture": "images/orig.png", "retouch": "x",
                       "mask": "inbox/mask.png"}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.starts_with("Nothing was drawn."),
            "{}",
            out.content
        );
        assert!(out.content.contains("marks nothing"), "{}", out.content);
        let seen = seen.lock().unwrap().clone();
        assert!(
            !seen.iter().any(|l| l.starts_with("POST /prompt")),
            "{seen:?}"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_canvas_sized_picture_keeps_its_own_bytes_outside_the_mask() {
        // The claim is "the original, pixel for pixel": measured against the
        // source file itself, not the prepared canvas, which the other tests
        // compare against (review of #429). A generated picture is already
        // canvas-sized, and this is the case the claim is made for.
        let source = image::RgbImage::from_fn(1344, 768, |x, y| {
            image::Rgb([
                (x * 7 % 251) as u8,
                (y * 13 % 241) as u8,
                ((x ^ y) % 239) as u8,
            ])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        source.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let plan = prepare_mask(
            png.get_ref(),
            &mask_png(1344, 768, (300, 300, 500, 500)),
            1024,
        )
        .unwrap();
        assert_eq!(plan.source, (1344, 768));
        let green = image::RgbImage::from_pixel(1344, 768, image::Rgb([10, 200, 30]));
        let mut result = std::io::Cursor::new(Vec::new());
        green
            .write_to(&mut result, image::ImageFormat::Png)
            .unwrap();
        let out = composite_masked(result.get_ref(), &plan).unwrap();
        let out = image::load_from_memory(&out).unwrap().to_rgb8();
        let unchanged = out
            .enumerate_pixels()
            .filter(|(x, y, _)| plan.soft.get_pixel(*x, *y).0[0] == 0)
            .all(|(x, y, p)| p == source.get_pixel(x, y));
        assert!(unchanged, "an unpainted pixel differs from the source file");
    }

    #[test]
    fn a_dab_too_small_to_survive_the_grow_is_refused() {
        // One painted pixel, at canvas size (1344×768 is its own canvas):
        // the grow step's blur spreads it to well under its threshold, so
        // nothing would be left to redraw.
        let big = image::RgbImage::from_pixel(1344, 768, image::Rgb([90, 90, 90]));
        let mut png = std::io::Cursor::new(Vec::new());
        big.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let err =
            prepare_mask(png.get_ref(), &mask_png(1344, 768, (10, 10, 11, 11)), 1024).unwrap_err();
        assert!(err.contains("too small to edit"), "{err}");
    }

    #[test]
    fn a_thin_stroke_on_a_large_photo_survives_the_downscale() {
        // A 2 px stroke on a 2368×1776 photo is about 1 px at its 1184×896
        // canvas: averaged down, not erased, so the owner gets an edit and
        // not "marks nothing" (review of #429). At a threshold of 127 this
        // was refused.
        let photo = image::RgbImage::from_pixel(2368, 1776, image::Rgb([90, 90, 90]));
        let mut png = std::io::Cursor::new(Vec::new());
        photo.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let plan = prepare_mask(
            png.get_ref(),
            &mask_png(2368, 1776, (600, 600, 602, 1200)),
            1024,
        )
        .unwrap();
        assert_eq!(plan.picture.dimensions(), (1184, 896));
        assert_eq!(plan.source, (2368, 1776));
    }

    #[test]
    fn a_masked_reading_counts_only_painted_cells() {
        // Two painted squares in a 640² bounding box: 80% of the box is
        // restored by the composite, identical in both pictures. Inverted
        // inside the squares, the edit plainly worked — and a reading over
        // the whole box would be dominated by the restored cells.
        let source = image::RgbImage::from_fn(1344, 768, |x, y| {
            let v = ((x / 8 + y / 5) % 200) as u8 + 20;
            image::Rgb([v, v, v])
        });
        let mut hard = image::GrayImage::new(1344, 768);
        for (x, y, p) in hard.enumerate_pixels_mut() {
            let a = (300..500).contains(&x) && (64..264).contains(&y);
            let b = (740..940).contains(&x) && (504..704).contains(&y);
            p.0[0] = if a || b { 255 } else { 0 };
        }
        let edited = image::RgbImage::from_fn(1344, 768, |x, y| {
            let o = source.get_pixel(x, y).0;
            if hard.get_pixel(x, y).0[0] > 0 {
                image::Rgb([255 - o[0], 255 - o[1], 255 - o[2]])
            } else {
                image::Rgb(o)
            }
        });
        let png = |img: &image::RgbImage| {
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).unwrap();
            out.into_inner()
        };
        let bounds = (300, 64, 940, 704);
        let r = layout_similarity_painted(&png(&source), &png(&edited), &hard, bounds).unwrap();
        assert!(r < 0.0, "inverted where painted reads as changed: {r}");
        let same = layout_similarity_painted(&png(&source), &png(&source), &hard, bounds).unwrap();
        assert!(same > 0.99, "{same}");
        // A sliver paints too few cells to read at all.
        let mut sliver = image::GrayImage::new(1344, 768);
        for y in 64..704 {
            sliver.put_pixel(300, y, image::Luma([255]));
        }
        assert_eq!(
            layout_similarity_painted(&png(&source), &png(&edited), &sliver, bounds),
            None
        );
    }

    /// A reference within the budget goes up byte for byte, tag and all:
    /// the server reads the tag itself.
    #[test]
    fn a_small_reference_is_sent_as_it_is() {
        let jpg = jpeg_with_orientation(90, 30, 6);
        assert_eq!(fit_reference(&jpg, 90 * 30), None);
    }

    /// Over the budget: scaled to fit it, the shape kept, and turned upright
    /// first — a sideways-stored landscape comes back portrait, its left
    /// half on top, because the PNG it becomes has no tag to say so.
    #[test]
    fn a_large_sideways_photo_goes_up_scaled_and_upright() {
        let jpg = jpeg_with_orientation(300, 100, 6); // 6: turn 90° clockwise
        let fitted = fit_reference(&jpg, 10_000).expect("over the budget");
        assert_eq!(sniff_image(&fitted), Some("png"));
        let img = image::load_from_memory(&fitted).unwrap().to_rgb8();
        let (w, h) = img.dimensions();
        assert!(u64::from(w) * u64::from(h) <= 10_000, "{w}×{h}");
        assert!(h > w * 2, "upright is portrait: {w}×{h}");
        let (top, bottom) = (img.get_pixel(w / 2, h / 8), img.get_pixel(w / 2, h * 7 / 8));
        assert!(
            top[0] > 200 && top[2] < 60,
            "the left half is on top: {top:?}"
        );
        assert!(
            bottom[2] > 200 && bottom[0] < 60,
            "the right half below: {bottom:?}"
        );
    }

    /// No tag: the shape as stored, scaled within the budget.
    #[test]
    fn a_large_untagged_picture_keeps_its_shape() {
        let jpg = jpeg_with_orientation(300, 100, 1); // 1: as stored
        let fitted = fit_reference(&jpg, 10_000).unwrap();
        let img = image::load_from_memory(&fitted).unwrap();
        let (w, h) = (img.width(), img.height());
        assert!(u64::from(w) * u64::from(h) <= 10_000, "{w}×{h}");
        let ratio = f64::from(w) / f64::from(h);
        assert!((ratio - 3.0).abs() < 0.1, "{w}×{h}");
    }

    /// A file whose header reads but whose body does not is the server's to
    /// judge, as before this step existed: sent as it is, never dropped.
    #[test]
    fn a_reference_that_does_not_decode_is_sent_as_it_is() {
        // A PNG, whose decoder fails on a short body; a JPEG decoder may
        // fill in what is missing instead, which is not this case.
        let png = png_bytes(&image::RgbImage::from_pixel(
            300,
            100,
            image::Rgb([9, 9, 9]),
        ))
        .unwrap();
        let cut = &png[..png.len() / 2];
        assert!(image::ImageReader::new(std::io::Cursor::new(cut))
            .with_guessed_format()
            .unwrap()
            .into_dimensions()
            .is_ok());
        assert_eq!(fit_reference(cut, 10_000), None);
        assert_eq!(fit_reference(b"not a picture", 10_000), None);
    }

    /// A small photo stored sideways with a tag is not fitted, so the mask
    /// path meets it as stored. The page painted over it upright, so the
    /// mask is the other shape; read without the tag, this refused.
    #[test]
    fn a_mask_over_a_sideways_stored_photo_fits_it() {
        let jpg = jpeg_with_orientation(300, 100, 6);
        let mask = image::GrayImage::from_fn(100, 300, |_, y| {
            image::Luma([if y < 150 { 255 } else { 0 }])
        });
        let mask = png_bytes(&mask).unwrap();
        let plan = prepare_mask(&jpg, &mask, EDIT_REFERENCE_SIZE).expect("same shape upright");
        assert_eq!(plan.source, (100, 300));
        assert!(plan.picture.height() > plan.picture.width());
    }

    /// The near-copy check compares the reference as the server saw it,
    /// upright, with a result that is: a sideways-stored copy of a picture
    /// reads as the picture, where read as stored it did not.
    #[test]
    fn a_sideways_stored_reference_is_compared_upright() {
        let jpg = jpeg_with_orientation(300, 100, 6);
        let upright = image::RgbImage::from_fn(100, 300, |_, y| {
            image::Rgb(if y < 150 { [255, 0, 0] } else { [0, 0, 255] })
        });
        let upright = png_bytes(&upright).unwrap();
        let alike = layout_similarity(&jpg, &upright).unwrap();
        assert!(alike > 0.9, "{alike}");
    }

    /// A face anchor that never runs a detector: a fixed crop, or a fixed
    /// reason it cannot, counting how often it was asked.
    struct StubFaces {
        answer: crate::face::Anchor,
        asked: std::sync::atomic::AtomicUsize,
    }

    impl crate::face::FaceAnchors for StubFaces {
        fn anchor(
            &self,
            _: &crate::imagelib::Library,
            _: &crate::imagelib::Entry,
        ) -> crate::face::Anchor {
            self.asked.fetch_add(1, Ordering::SeqCst);
            self.answer.clone()
        }
    }

    fn stub_faces(answer: crate::face::Anchor) -> Arc<StubFaces> {
        Arc::new(StubFaces {
            answer,
            asked: Default::default(),
        })
    }

    fn persona_maya() -> crate::tool::PersonaSelf {
        crate::tool::PersonaSelf {
            name: "maya".into(),
            display: "Maya".into(),
            character: Some("maya".into()),
        }
    }

    fn picture_of(content: &str) -> String {
        content
            .lines()
            .next()
            .unwrap()
            .strip_prefix("image: ")
            .unwrap()
            .to_string()
    }

    /// The last prompt sent to the image server.
    fn last_prompt(seen: &Arc<Mutex<Vec<String>>>) -> String {
        seen.lock()
            .unwrap()
            .iter()
            .rev()
            .find(|l| l.starts_with("POST /prompt"))
            .unwrap()
            .clone()
    }

    /// A persona chat's scene slot over a scratch store, for chat `chat`.
    fn scene_ctx(dir: &std::path::Path, store: &std::path::Path, chat: &str) -> ToolCtx {
        ToolCtx {
            workspace: dir.to_path_buf(),
            scene: Some(crate::scene::SceneSlot {
                chat_copy: store.join("sessions").join(format!("{chat}.scene.json")),
                store: store.join("scene"),
                chat: chat.into(),
                from_latest: true,
            }),
            ..Default::default()
        }
    }

    /// A fake that draws `n` different pictures, so each landed render has
    /// its own hash in the scene index.
    async fn distinct(n: usize) -> (String, Arc<Mutex<Vec<String>>>) {
        fake_with(Fake {
            history: vec![done(); n],
            views: (0..n)
                .map(|i| {
                    picture(
                        8,
                        [(40 + 50 * i % 200) as u8, 90, (200 - 30 * i % 200) as u8],
                    )
                })
                .collect(),
            ..Fake::default()
        })
        .await
    }

    /// Someone is drawn only when listed in `people` (the owner's ruling,
    /// 2026-10-08): a library name in the words for someone not in the
    /// picture reaches the image model as "the viewer", no portrait of them
    /// is sent, and the result says so. The persona addressing the owner by
    /// a name the library also holds was every G1b refusal.
    #[tokio::test]
    async fn a_library_name_in_prose_for_someone_not_drawn_is_the_viewer() {
        let (url, seen) = distinct(1).await;
        let lib = library_with(&["maya", "john"]);
        let dir = tempdir();
        let out = tool(&url)
            .with_library_dir(lib.clone())
            .call(
                json!({"scene": {"setting": "a park bench",
                       "together": "Maya waves at John off to the side",
                       "people": [{"who": "maya", "wearing": "a coat", "doing": "reading"}]}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content
                .contains("John named in the words but not in the picture"),
            "{}",
            out.content
        );
        let prompt = last_prompt(&seen);
        assert!(
            prompt.contains("the viewer") && !prompt.contains("John"),
            "{prompt}"
        );
        assert_eq!(uploads(&seen), 1, "only Maya's portrait");
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A clean run's context over a scene slot, as the persona chat builds it.
    fn clean(mut ctx: ToolCtx) -> ToolCtx {
        ctx.taint = Some(crate::agent::Taint {
            private: false,
            untrusted: false,
        });
        ctx
    }

    fn seed_in(seen: &Arc<Mutex<Vec<String>>>) -> u64 {
        let p = last_prompt(seen);
        let body: Value = serde_json::from_str(&p[p.find('{').unwrap()..]).unwrap();
        body["prompt"]["sample"]["inputs"]["seed"].as_u64().unwrap()
    }

    fn uploads(seen: &Arc<Mutex<Vec<String>>>) -> usize {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("POST /upload/image"))
            .count()
    }

    /// A new picture lands as the chat's scene, keyed by its bytes; a pose
    /// change on it is a restage drawn from the setting's words at the
    /// picture's seed, with the old pose gone from the prompt
    /// (IMAGE-DESIGN.md §2.6, §5.2).
    #[tokio::test]
    async fn a_new_picture_lands_its_scene_and_a_pose_change_restages_at_its_seed() {
        let (url, seen) = distinct(2).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let first = t
            .call(
                json!({"scene": {"setting": "a quiet reading room", "light": "late afternoon sun",
                       "people": [{"who": "maya", "wearing": "a green coat", "doing": "reading a book"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!first.is_error, "{}", first.content);
        assert!(
            first
                .content
                .contains("A new 1024×1024 picture with character maya (v1)"),
            "{}",
            first.content
        );
        let p1 = picture_of(&first.content);
        let seed1 = seed_in(&seen);
        let slot = cx.scene.clone().unwrap();
        let landed = slot
            .lookup(&std::fs::read(dir.join(&p1)).unwrap())
            .expect("landed");
        assert_eq!(landed.seed, Some(seed1));
        assert_eq!(landed.people[0].doing, "reading a book");
        assert_eq!(landed.people[0].origin, crate::scene::Origin::Clean);
        assert_eq!(manifest_of(&dir, &first.content)["route"], "new");

        let second = t
            .call(
                json!({"picture": p1, "scene": {"people": [{"who": "maya", "doing": "standing by the window"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!second.is_error, "{}", second.content);
        assert!(
            second
                .content
                .contains("restaged from the scene's setting in words"),
            "{}",
            second.content
        );
        assert_eq!(seed_in(&seen), seed1, "a restage keeps the picture's seed");
        let prompt = last_prompt(&seen);
        assert!(
            prompt.contains("a quiet reading room.") && prompt.contains("standing by the window"),
            "{prompt}"
        );
        assert!(
            !prompt.contains("reading a book"),
            "the old pose rode along: {prompt}"
        );
        assert!(
            prompt.contains("a green coat"),
            "the clothes are kept: {prompt}"
        );
        let p2 = picture_of(&second.content);
        let now = slot.lookup(&std::fs::read(dir.join(&p2)).unwrap()).unwrap();
        assert_eq!(now.people[0].doing, "standing by the window");
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// A co-subject the edit leaves alone has no crop, so the canvas is her
    /// only identity source and the references stay at 1024; with one person
    /// and her crop they go at 512 (review of #601).
    #[tokio::test]
    async fn a_co_subject_without_a_crop_keeps_full_references() {
        let (url, seen) = distinct(3).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [200, 150, 120])));
        let t = tool(&url)
            .with_library_dir(lib.clone())
            .with_faces(Arc::clone(&faces) as Arc<dyn crate::face::FaceAnchors>);
        let cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let resolution = || {
            let p = last_prompt(&seen);
            let body: Value = serde_json::from_str(&p[p.find('{').unwrap()..]).unwrap();
            body["prompt"]["encode"]["inputs"]["resolution"].clone()
        };
        let two = t
            .call(
                json!({"scene": {"setting": "a park bench", "people": [
                    {"who": "maya", "wearing": "a green coat", "doing": "sitting"},
                    {"who": "john", "wearing": "a flannel shirt", "doing": "sitting"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!two.is_error, "{}", two.content);
        let dressed = t
            .call(
                json!({"picture": picture_of(&two.content),
                       "scene": {"people": [{"who": "maya", "wearing": "a red scarf"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!dressed.is_error, "{}", dressed.content);
        assert_eq!(
            manifest_of(&dir, &dressed.content)["crops"],
            json!(["Maya"]),
            "only the person changed gets a crop"
        );
        assert_eq!(resolution(), 1024, "John rides on the canvas alone");
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// The plain pose is said at the prompt, never stored: two people drawn
    /// with no pose stand naturally, and a `together` sent after them is
    /// drawn without a standing pose beside it, the shape that drew a person
    /// twice (mecha-a3; review of #608).
    #[tokio::test]
    async fn a_together_after_the_people_carries_no_standing_pose() {
        let (url, seen) = distinct(3).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [200, 150, 120])));
        let t = tool(&url)
            .with_library_dir(lib.clone())
            .with_faces(Arc::clone(&faces) as Arc<dyn crate::face::FaceAnchors>);
        let cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let first = t
            .call(
                json!({"scene": {"setting": "a clinic room", "people": [
                    {"who": "maya", "wearing": "a coat"},
                    {"who": "john", "wearing": "a suit"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!first.is_error, "{}", first.content);
        assert!(last_prompt(&seen).contains("standing naturally"));
        let then = t
            .call(
                json!({"picture": picture_of(&first.content),
                       "scene": {"together": "Maya kneels to tie John's shoelace"}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!then.is_error, "{}", then.content);
        let p = last_prompt(&seen);
        assert!(p.contains("kneels to tie"), "{p}");
        assert!(!p.contains("standing naturally"), "{p}");
        // A change of clothes under that `together` is an edit; its prompt
        // gives Maya no standing pose either (`edit_person`).
        let dressed = t
            .call(
                json!({"picture": picture_of(&then.content),
                       "scene": {"people": [{"who": "maya", "wearing": "a red coat"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!dressed.is_error, "{}", dressed.content);
        let p = last_prompt(&seen);
        assert!(p.contains("a red coat"), "{p}");
        assert!(!p.contains("standing naturally"), "{p}");
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// The reader's change fills only what the call left out: a `together`,
    /// and each named person's `doing` and `wearing`; never someone the call
    /// does not name, and never the setting.
    #[test]
    fn a_read_fills_only_what_the_call_left_out() {
        let mut change = crate::scene::SceneChange {
            together: None,
            people: vec![
                crate::scene::PersonChange {
                    who: crate::scene::Who::Library("maya".into()),
                    at: None,
                    wearing: None,
                    doing: Some("waving".into()),
                    expression: None,
                    remove: false,
                },
                crate::scene::PersonChange {
                    who: crate::scene::Who::Library("john".into()),
                    at: None,
                    wearing: None,
                    doing: None,
                    expression: None,
                    remove: false,
                },
            ],
            ..Default::default()
        };
        let read = json!({
            "together": "Maya and John dance",
            "setting": "a ballroom",
            "people": [
                {"who": "Maya", "doing": "spinning", "wearing": "a red dress"},
                {"who": "John", "doing": "leading the dance"},
                {"who": "Wren", "doing": "watching"}
            ]
        });
        let merged = merge_read(&mut change, read.as_object().unwrap());
        assert_eq!(change.together.as_deref(), Some("Maya and John dance"));
        assert_eq!(
            change.people[0].doing.as_deref(),
            Some("waving"),
            "hers is kept"
        );
        assert_eq!(change.people[0].wearing.as_deref(), Some("a red dress"));
        assert_eq!(change.people[1].doing.as_deref(), Some("leading the dance"));
        assert_eq!(change.people.len(), 2, "nobody added");
        assert!(change.setting.is_none(), "the setting stays the call's");
        assert_eq!(merged, ["together", "doing×1", "wearing×1"]);
        // Bounded as the call's own: a long act is clipped, a long part left.
        let long = format!("{}.", "Maya laughs ".repeat(40));
        let mut change = crate::scene::SceneChange {
            people: vec![crate::scene::PersonChange {
                who: crate::scene::Who::Library("maya".into()),
                at: None,
                wearing: None,
                doing: None,
                expression: None,
                remove: false,
            }],
            ..Default::default()
        };
        let read = json!({"together": long, "people": [{"who": "Maya", "doing": long}]});
        merge_read(&mut change, read.as_object().unwrap());
        assert!(
            change.together.as_ref().unwrap().chars().count() <= crate::imagelib::MAX_CAST_FIELD
        );
        assert!(change.people[0].doing.is_none(), "too long to merge");
    }

    /// A reader that answers as told, standing in for the persona host's.
    #[derive(Debug)]
    struct FakeReader(std::result::Result<serde_json::Value, String>);

    #[async_trait]
    impl crate::persona::edit::SceneReader for FakeReader {
        async fn read(
            &self,
            _record: Option<&crate::scene::Scene>,
        ) -> std::result::Result<crate::persona::edit::Extracted, String> {
            self.0.clone().map(|v| crate::persona::edit::Extracted {
                scene: v.as_object().cloned().unwrap_or_default(),
                retouch: None,
                left_out: None,
            })
        }
    }

    /// A persona's bare call is drawn with what the reader found it left
    /// out, stamped with the conversation's own origin (an untrusted turn's
    /// merged words land untrusted), and said in the manifest; a reader that
    /// fails leaves the call as sent.
    #[tokio::test]
    async fn a_bare_call_is_filled_from_the_turns_words() {
        let (url, seen) = distinct(4).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.taint = Some(crate::agent::Taint {
            private: false,
            untrusted: true,
        });
        cx.scene_reader = Some(Arc::new(FakeReader(Ok(json!({
            "together": "Maya hands John a bunch of tulips"
        })))));
        let call = json!({"scene": {"setting": "a park",
            "people": [{"who": "maya", "wearing": "a coat"}, {"who": "john", "wearing": "a suit"}]}});
        let out = t.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(last_prompt(&seen).contains("bunch of tulips"));
        let landed = cx
            .scene
            .as_ref()
            .unwrap()
            .lookup(&std::fs::read(dir.join(picture_of(&out.content))).unwrap())
            .unwrap();
        let together = landed.together.unwrap();
        assert_eq!(together.value, "Maya hands John a bunch of tulips");
        assert_eq!(together.origin, crate::scene::Origin::Untrusted);
        assert_eq!(
            manifest_of(&dir, &out.content)["reader"],
            "merged: together"
        );
        // A reader that fails leaves the call as sent.
        cx.scene_reader = Some(Arc::new(FakeReader(Err("no answer".into()))));
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(!last_prompt(&seen).contains("tulips"));
        assert_eq!(
            manifest_of(&dir, &out.content)["reader"],
            "fell back: no answer"
        );
        // A library character whose entry did not load, named only in the
        // reader's words, is never drawn as a stranger: the merge is dropped.
        let wren = lib.join("characters/wren");
        std::fs::create_dir_all(&wren).unwrap();
        std::fs::write(wren.join("entry.toml"), "not = [toml").unwrap();
        cx.scene_reader = Some(Arc::new(FakeReader(Ok(json!({
            "together": "Maya and John dance with Wren"
        })))));
        let out = t
            .call(
                json!({"scene": {"setting": "a park",
                    "people": [{"who": "maya", "wearing": "a coat"}, {"who": "john", "wearing": "a suit"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(!last_prompt(&seen).contains("Wren"));
        assert_eq!(
            manifest_of(&dir, &out.content)["reader"],
            "fell back: the reader named a library entry that could not be read"
        );
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// A merge that read an untrusted record lands untrusted, even in a clean
    /// turn: the reader may restate what it read, and a scene's origin is
    /// what arms the next turn's note (review of #610).
    #[tokio::test]
    async fn a_merge_from_an_untrusted_record_lands_untrusted() {
        let (url, _seen) = distinct(3).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.taint = Some(crate::agent::Taint {
            private: false,
            untrusted: true,
        });
        let first = t
            .call(
                json!({"scene": {"setting": "a park", "people": [
                    {"who": "maya", "wearing": "a coat", "doing": "reading"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!first.is_error, "{}", first.content);
        let mut cx = clean(cx);
        cx.scene_reader = Some(Arc::new(FakeReader(Ok(json!({
            "people": [{"who": "Maya", "doing": "waving at the camera"}]
        })))));
        let out = t
            .call(
                json!({"picture": picture_of(&first.content), "scene": {"people": [{"who": "maya"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let landed = cx
            .scene
            .as_ref()
            .unwrap()
            .lookup(&std::fs::read(dir.join(picture_of(&out.content))).unwrap())
            .unwrap();
        assert_eq!(landed.origin(), crate::scene::Origin::Untrusted);
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// Each picture's prompt is saved for the owner where the host stamps a
    /// log (owner-only, outside the jail), and nowhere when it does not.
    #[tokio::test]
    async fn a_pictures_prompt_is_saved_only_where_the_host_asks() {
        use std::os::unix::fs::PermissionsExt;
        let (url, _seen) = distinct(2).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let log = store.join("sessions/chat-a.prompts.log");
        cx.prompt_log = Some(log.clone());
        let call = json!({"scene": {"setting": "a quiet harbour",
            "people": [{"who": "maya", "wearing": "a coat", "doing": "waving"}]}});
        let out = t.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        let text = std::fs::read_to_string(&log).unwrap();
        let line: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert!(line["prompt"].as_str().unwrap().contains("a quiet harbour"));
        assert_eq!(line["image"], picture_of(&out.content));
        assert_eq!(
            std::fs::metadata(&log).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // Not stamped: nothing saved.
        cx.prompt_log = None;
        std::fs::remove_file(&log).unwrap();
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(!log.exists());
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// A splitter that answers as told, standing in for the persona host's.
    #[derive(Debug)]
    struct FakeSplit(std::result::Result<crate::roles::Split, String>);

    #[async_trait]
    impl crate::roles::RoleSplit for FakeSplit {
        async fn split(
            &self,
            _people: &[crate::roles::Asked],
            _together: &str,
        ) -> std::result::Result<crate::roles::Split, String> {
            self.0.clone()
        }
    }

    /// A call that puts the whole act in `together` and poses nobody is
    /// drawn from each person's own part, in the split's order, with only the
    /// leftover of `together` said; the record keeps the call as sent; and a
    /// split that fails draws the call as it was (mecha-a3, 2026-10-08).
    #[tokio::test]
    async fn a_together_is_drawn_as_each_persons_part() {
        let (url, seen) = distinct(4).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let call = json!({"scene": {"setting": "a park", "together": "Maya hands John a cup",
            "people": [{"who": "maya", "wearing": "a coat"}, {"who": "john", "wearing": "a suit"}]}});
        cx.role_split = Some(Arc::new(FakeSplit(Ok(crate::roles::Split {
            roles: vec![
                crate::roles::Role {
                    who: "Maya".into(),
                    doing: "holding out a paper cup to John".into(),
                    at: crate::scene::Where::Right,
                },
                crate::roles::Role {
                    who: "John".into(),
                    doing: "reaching to take the cup from Maya".into(),
                    at: crate::scene::Where::Left,
                },
            ],
            together: String::new(),
        }))));
        let out = t.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        let p = last_prompt(&seen);
        assert!(p.contains("holding out a paper cup to John"), "{p}");
        assert!(
            !p.contains("Maya hands John a cup"),
            "the leftover alone: {p}"
        );
        assert!(
            p.find("<image1> (john").unwrap() < p.find("<image2> (maya").unwrap(),
            "the split's order: {p}"
        );
        // The record keeps the call as the persona sent it.
        let landed = cx
            .scene
            .as_ref()
            .unwrap()
            .lookup(&std::fs::read(dir.join(picture_of(&out.content))).unwrap())
            .unwrap();
        assert!(landed.people.iter().all(|q| q.doing.is_empty()));
        assert!(landed.together.is_some());
        // The manifest says the split ran; never the prompt, which carries
        // the library's descriptions into a file a run can read.
        let m = manifest_of(&dir, &out.content);
        assert_eq!(m["roles"], "applied");
        assert!(m.get("prompt").is_none());
        // Everyone posed and a `together` too: still split, so the named
        // sentence leaves the words (mecha-a3: a second person in 2 of 3).
        let posed = json!({"scene": {"setting": "a park", "together": "Maya hands John a cup",
            "people": [{"who": "maya", "wearing": "a coat", "doing": "smiling"},
                       {"who": "john", "wearing": "a suit", "doing": "reaching out"}]}});
        let out = t.call(posed, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(manifest_of(&dir, &out.content)["roles"], "applied");
        assert!(!last_prompt(&seen).contains("Maya hands John a cup"));
        // A split that fails draws the call as it was.
        cx.role_split = Some(Arc::new(FakeSplit(Err("no answer".into()))));
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(last_prompt(&seen).contains("Maya hands John a cup"));
        assert_eq!(
            manifest_of(&dir, &out.content)["roles"],
            "fell back: no answer"
        );
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// A split is prompt-only and never turns a drawable call into a
    /// refusal: parts the compiler will not take (a described person's part
    /// naming a library character) draw the call as sent, and say so; and a
    /// place the call gave as `background` is kept (review of #609).
    #[tokio::test]
    async fn a_split_that_will_not_compile_draws_the_call_as_sent() {
        let (url, seen) = distinct(3).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = clean(scene_ctx(&dir, &store, "chat-a"));
        cx.role_split = Some(Arc::new(FakeSplit(Ok(crate::roles::Split {
            roles: vec![
                crate::roles::Role {
                    who: "Maya".into(),
                    doing: "taking a tray".into(),
                    at: crate::scene::Where::Left,
                },
                crate::roles::Role {
                    who: "a waiter".into(),
                    doing: "handing Maya a tray".into(),
                    at: crate::scene::Where::Right,
                },
            ],
            together: String::new(),
        }))));
        let out = t
            .call(
                json!({"scene": {"setting": "a cafe", "together": "a waiter hands Maya a tray",
                    "people": [{"who": "maya", "wearing": "a coat"},
                               {"who": "a waiter", "wearing": "an apron"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "drawn, not refused: {}", out.content);
        assert!(last_prompt(&seen).contains("a waiter hands Maya a tray"));
        assert_eq!(
            manifest_of(&dir, &out.content)["roles"],
            "fell back: the split's parts did not compile"
        );
        // A `background` the call chose is kept beside the split's parts.
        cx.role_split = Some(Arc::new(FakeSplit(Ok(crate::roles::Split {
            roles: vec![
                crate::roles::Role {
                    who: "Maya".into(),
                    doing: "pouring tea".into(),
                    at: crate::scene::Where::Left,
                },
                crate::roles::Role {
                    who: "John".into(),
                    doing: "reading by the window".into(),
                    at: crate::scene::Where::Right,
                },
            ],
            together: String::new(),
        }))));
        let out = t
            .call(
                json!({"scene": {"setting": "a cafe", "together": "Maya pours tea while John reads",
                    "people": [{"who": "maya", "wearing": "a coat"},
                               {"who": "john", "wearing": "a suit", "where": "background"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let p = last_prompt(&seen);
        assert!(
            p.contains("in the background, reading by the window"),
            "{p}"
        );
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// New clothes edit the picture itself, with the person's head crop and
    /// library description; a call that changes nothing is drawn again at a
    /// new seed (§5.1, review B4).
    #[tokio::test]
    async fn clothes_edit_with_a_crop_and_a_restatement_redraws() {
        let (url, seen) = distinct(3).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya"]));
        let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [200, 150, 120])));
        let t = tool(&url)
            .with_library_dir(lib.clone())
            .with_faces(Arc::clone(&faces) as Arc<dyn crate::face::FaceAnchors>);
        let cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let first = t
            .call(
                json!({"scene": {"setting": "a park bench",
                       "people": [{"who": "maya", "wearing": "a green coat", "doing": "sitting"}]}}),
                &cx,
            )
            .await
            .unwrap();
        let p1 = picture_of(&first.content);
        let seed1 = seed_in(&seen);
        let before = uploads(&seen);

        let dressed = t
            .call(
                json!({"picture": p1, "scene": {"people": [{"who": "maya", "wearing": "a red scarf"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!dressed.is_error, "{}", dressed.content);
        assert!(
            dressed
                .content
                .contains(&format!("An edit of {p1} with character maya (v1)")),
            "{}",
            dressed.content
        );
        assert!(
            dressed.content.contains(&format!("{p1} is unchanged.")),
            "{}",
            dressed.content
        );
        assert_eq!(uploads(&seen) - before, 2, "the canvas and one crop");
        assert_eq!(faces.asked.load(Ordering::SeqCst), 1);
        let prompt = last_prompt(&seen);
        assert!(
            prompt.contains("Take only Maya's facial identity from <image2>, nothing else."),
            "{prompt}"
        );
        assert!(prompt.contains("maya, a memorable face"), "{prompt}");
        assert!(prompt.contains("a red scarf"), "{prompt}");
        // The crop carries who she is, so the references go small.
        let body: Value = serde_json::from_str(&prompt[prompt.find('{').unwrap()..]).unwrap();
        assert_eq!(body["prompt"]["encode"]["inputs"]["resolution"], 512);
        assert_eq!(
            manifest_of(&dir, &dressed.content)["crops"],
            json!(["Maya"])
        );

        let p2 = picture_of(&dressed.content);
        let again = t
            .call(
                json!({"picture": p2, "scene": {"people": [{"who": "maya", "wearing": "a red scarf", "doing": "sitting"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(
            again.content.contains("drawn again at a new seed"),
            "{}",
            again.content
        );
        let landed2 = cx
            .scene
            .as_ref()
            .unwrap()
            .lookup(&std::fs::read(dir.join(&p2)).unwrap())
            .unwrap();
        assert_ne!(
            Some(seed_in(&seen)),
            landed2.seed,
            "a redraw differs from the picture it redraws"
        );
        let _ = seed1;
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// A render that fails lands nothing: the scene advances only with a
    /// saved picture.
    #[tokio::test]
    async fn a_failed_render_never_advances_the_scene() {
        let (dir, store) = (tempdir(), tempdir());
        let out = tool("http://127.0.0.1:1")
            .call(
                json!({"scene": {"setting": "a harbour at dawn"}}),
                &clean(scene_ctx(&dir, &store, "chat-a")),
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        let index = store.join("scene").join("index");
        assert!(
            std::fs::read_dir(&index).map_or(true, |mut d| d.next().is_none()),
            "a failed render was recorded"
        );
        for d in [dir, store] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// A tainted run's scene is recorded as untrusted, field by field, so a
    /// later clean run knows which words it did not write.
    #[tokio::test]
    async fn a_scenes_origin_is_its_runs_taint() {
        let (url, _) = distinct(1).await;
        let (dir, store) = (tempdir(), tempdir());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.taint = Some(crate::agent::Taint {
            private: false,
            untrusted: true,
        });
        let out = tool(&url)
            .call(json!({"scene": {"setting": "a rooftop garden", "people": [{"who": "a gardener", "wearing": "overalls", "doing": "watering"}]}}), &cx)
            .await
            .unwrap();
        let p = picture_of(&out.content);
        let s = cx
            .scene
            .as_ref()
            .unwrap()
            .lookup(&std::fs::read(dir.join(&p)).unwrap())
            .unwrap();
        assert_eq!(s.setting.unwrap().origin, crate::scene::Origin::Untrusted);
        assert_eq!(s.people[0].origin, crate::scene::Origin::Untrusted);
        for d in [dir, store] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// People placed on a room photo: an edit of the photo with each face's
    /// crop. A pose change after it redraws everyone on that photo, checked
    /// by its hash, so a different file under the same name is refused.
    #[tokio::test]
    async fn a_photo_setting_places_people_and_restages_on_the_same_photo() {
        let (url, seen) = distinct(3).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("inbox/room.png"), picture(30, [90, 90, 90])).unwrap();
        let t = tool(&url)
            .with_library_dir(lib.clone())
            .with_faces(stub_faces(crate::face::Anchor::Crop(picture(
                4,
                [200, 150, 120],
            ))));
        let cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let placed = t
            .call(
                json!({"scene": {"setting": {"photo": "inbox/room.png"}, "people": two_people()}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!placed.is_error, "{}", placed.content);
        assert!(
            placed
                .content
                .contains("The people placed on inbox/room.png"),
            "{}",
            placed.content
        );
        assert_eq!(uploads(&seen), 3, "the room and two crops");
        assert!(last_prompt(&seen).contains("Each of them appears exactly once."));

        let p1 = picture_of(&placed.content);
        let restaged = t
            .call(
                json!({"picture": p1, "scene": {"camera": "from above"}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!restaged.is_error, "{}", restaged.content);
        assert!(
            restaged
                .content
                .contains("on the scene's setting photo inbox/room.png"),
            "{}",
            restaged.content
        );
        assert!(
            last_prompt(&seen).contains("from above."),
            "the camera is said: {}",
            last_prompt(&seen)
        );

        std::fs::write(dir.join("inbox/room.png"), picture(50, [10, 200, 10])).unwrap();
        let p2 = picture_of(&restaged.content);
        let gone = t
            .call(
                json!({"picture": p2, "scene": {"camera": "at eye level"}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(
            gone.is_error && gone.content.contains("is not in this chat as it was"),
            "{}",
            gone.content
        );
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// A persona draws itself as its linked character by `self`, and by its
    /// own name.
    #[tokio::test]
    async fn a_persona_draws_itself_as_its_character() {
        let (url, seen) = distinct(2).await;
        let (dir, lib) = (tempdir(), library_with(&["maya"]));
        let t = tool(&url)
            .with_library_dir(lib.clone())
            .persona_form(Some(persona_maya()));
        for who in ["self", "Maya"] {
            let out = t
                .call(
                    json!({"scene": {"setting": "a train platform",
                           "people": [{"who": who, "wearing": "a coat", "doing": "waiting"}]}}),
                    &ctx(&dir),
                )
                .await
                .unwrap();
            assert!(
                out.content.contains("with character maya (v1)"),
                "{who}: {}",
                out.content
            );
        }
        assert_eq!(uploads(&seen), 2, "one portrait each time");
        for d in [dir, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// Six faces are more than one picture draws: refused before the GPU,
    /// never trimmed. A described person costs no face.
    #[tokio::test]
    async fn a_sixth_face_is_refused_and_a_described_person_is_free() {
        let names = ["maya", "john", "wren", "ivo", "tamsin", "orla"];
        let (url, seen) = distinct(1).await;
        let (dir, lib) = (tempdir(), library_with(&names));
        let t = tool(&url).with_library_dir(lib.clone());
        let six: Vec<Value> = names
            .iter()
            .map(|n| json!({"who": n, "wearing": "a coat", "doing": "standing"}))
            .collect();
        let out = t
            .call(
                json!({"scene": {"setting": "a pier", "people": six}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.starts_with("Nothing was drawn. "),
            "{}",
            out.content
        );
        assert!(!seen
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("POST /prompt")));
        let five_and_one: Vec<Value> = names[..5]
            .iter()
            .map(|n| json!({"who": n, "wearing": "a coat", "doing": "standing"}))
            .chain([json!({"who": "a fisherman", "wearing": "oilskins", "doing": "mending a net"})])
            .collect();
        let out = t
            .call(
                json!({"scene": {"setting": "a pier", "people": five_and_one}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(last_prompt(&seen).contains("a fisherman, wearing oilskins, mending a net"));
        assert_eq!(uploads(&seen), 5, "five portraits, none for the fisherman");
        for d in [dir, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// `style` is for a look the user asked for: with the styles listed, a
    /// description that said to fill it on every new picture had a persona
    /// pick an unasked-for style 4 times in 5 (mecha-a3, 2026-10-08).
    #[test]
    fn style_is_only_for_a_look_the_user_asked_for() {
        let t = tool("http://127.0.0.1:1");
        assert!(t
            .description()
            .contains("`style` (a library style's name, only when the user asks for a look)"));
        let schema = t.input_schema();
        let said = schema["properties"]["scene"]["properties"]["style"]["description"]
            .as_str()
            .unwrap();
        assert!(
            said.contains("leave it out for the usual photograph"),
            "{said}"
        );
    }

    /// The description names the approved, unlocked styles at its very end,
    /// read when the form is built, and never a character (the owner's
    /// ruling, 2026-10-08: style names are not private, character names are).
    #[test]
    fn the_description_ends_with_the_styles_to_name() {
        let lib = library_with(&["maya"]);
        for (name, locked) in [("ink-wash", false), ("secret-look", true)] {
            crate::imagelib::create(
                &lib,
                crate::imagelib::NewEntry {
                    kind: crate::imagelib::Kind::Style,
                    name: name.into(),
                    text: "a look".into(),
                    portrait: None,
                    source_seed: None,
                    origin: crate::imagelib::Origin::Owner,
                    locked,
                },
            )
            .unwrap();
        }
        let t = tool("http://127.0.0.1:1").with_library_dir(lib.clone());
        let d = t.description().to_string();
        assert!(d.starts_with(DESCRIPTION), "earlier bytes never move");
        assert!(d.ends_with(" Styles you can name: `ink-wash`."), "{d}");
        assert!(!d.contains("secret-look") && !d.contains("`maya`"), "{d}");
        // The persona form reads the library when it is built.
        let persona = Arc::new(t).for_persona().unwrap();
        assert!(persona.description().ends_with("`ink-wash`."));
        // No library, no list.
        let bare = tool("http://127.0.0.1:1");
        assert_eq!(bare.description(), DESCRIPTION);
        std::fs::remove_dir_all(lib).ok();
    }

    /// A `style` the library does not hold is left out and said, with the
    /// unlocked styles there are, and the picture still draws, a new one or
    /// one drawn over the chat's record; asked alone it is refused naming
    /// them. A persona chat retried "call image_library" ten times
    /// (2026-10-08).
    #[tokio::test]
    async fn an_unknown_style_is_left_out_and_the_picture_draws() {
        let (url, seen) = distinct(8).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya"]));
        crate::imagelib::create(
            &lib,
            crate::imagelib::NewEntry {
                kind: crate::imagelib::Kind::Style,
                name: "ink-wash".into(),
                text: "soft grey ink wash on rice paper".into(),
                portrait: None,
                source_seed: None,
                origin: crate::imagelib::Origin::Owner,
                locked: false,
            },
        )
        .unwrap();
        // A locked style is the owner's way to hide one: never named.
        crate::imagelib::create(
            &lib,
            crate::imagelib::NewEntry {
                kind: crate::imagelib::Kind::Style,
                name: "secret-look".into(),
                text: "a look the owner keeps out of sight".into(),
                portrait: None,
                source_seed: None,
                origin: crate::imagelib::Origin::Owner,
                locked: true,
            },
        )
        .unwrap();
        let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [200, 150, 120])));
        let t = tool(&url)
            .with_library_dir(lib.clone())
            .with_faces(Arc::clone(&faces) as Arc<dyn crate::face::FaceAnchors>);
        let cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let posts = || {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|l| l.starts_with("POST /prompt"))
                .count()
        };
        let first = t
            .call(
                json!({"scene": {"setting": "a park bench", "style": "hyperreal render",
                       "people": [{"who": "maya", "wearing": "a green coat", "doing": "sitting"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!first.is_error, "{}", first.content);
        // The note names only what was asked, and the way on in words: never
        // the library's other styles (the roster is the owner's).
        assert!(
            first
                .content
                .contains("There is no approved style `hyperreal render`")
                && first.content.contains("Styles you can name: `ink-wash`.")
                && first.content.contains("`scene.setting`")
                && !first.content.contains("secret-look"),
            "{}",
            first.content
        );
        assert_eq!(posts(), 1);
        // Words for a library style's name are that style: case, spaces and
        // underscores are spelling, not a different style.
        for words in ["Ink Wash", "ink_wash", " INK  wash "] {
            let out = t
                .call(
                    json!({"scene": {"setting": "a quiet harbour", "style": words,
                           "people": [{"who": "maya", "wearing": "a coat", "doing": "waving"}]}}),
                    &cx,
                )
                .await
                .unwrap();
            assert!(!out.is_error, "{}", out.content);
            assert!(!out.content.contains("There is no"), "{}", out.content);
            assert!(
                last_prompt(&seen).contains("soft grey ink wash on rice paper"),
                "{words}: the style's own words reach the prompt"
            );
        }
        let posts_before = posts();
        // Over the chat's record, where the persona's calls went.
        let again = t
            .call(
                json!({"scene": {"style": "oil on linen", "light": "a grey drizzle",
                       "people": [{"who": "maya"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!again.is_error, "{}", again.content);
        assert!(
            again
                .content
                .contains("There is no approved style `oil on linen`"),
            "{}",
            again.content
        );
        assert_eq!(posts(), posts_before + 1);
        let alone = t
            .call(
                json!({"picture": picture_of(&again.content),
                       "scene": {"style": "oil on linen"}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(alone.is_error, "{}", alone.content);
        assert!(
            alone.content.contains("Name one of those")
                && alone.content.contains("`ink-wash`")
                && !alone.content.contains("secret-look"),
            "{}",
            alone.content
        );
        assert!(
            !alone.content.contains("image_library"),
            "{}",
            alone.content
        );
        assert_eq!(posts(), posts_before + 1, "a refused call draws nothing");
        // A made-up picture beside a scene that describes no picture of its
        // own (a style alone) is refused, never drawn from nothing: it drew a
        // stranger in an empty room (mecha-a3, review of #603).
        let restyle = t
            .call(
                json!({"picture": "images/made_up_name.png", "scene": {"style": "Ink Wash"}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(
            restyle.is_error && restyle.content.contains("There is no picture"),
            "{}",
            restyle.content
        );
        assert_eq!(posts(), posts_before + 1, "nothing drawn from nothing");

        // A style waiting on the owner, or one whose entry did not load, is a
        // finding: refused by name, never dropped (review of #603).
        let mut proposed = crate::imagelib::NewEntry {
            kind: crate::imagelib::Kind::Style,
            name: "chalk-pastel".into(),
            text: "soft chalk pastel on toned paper".into(),
            portrait: None,
            source_seed: None,
            origin: crate::imagelib::Origin::ModelUntrusted,
            locked: false,
        };
        crate::imagelib::create(&lib, proposed.clone()).unwrap();
        proposed.name = "charcoal".into();
        crate::imagelib::create(&lib, proposed).unwrap();
        std::fs::write(lib.join("styles/charcoal/entry.toml"), "not = [valid").unwrap();
        for (style, says) in [
            ("chalk-pastel", "waiting for the owner's approval"),
            ("charcoal", "could not be read"),
        ] {
            let out = t
                .call(
                    json!({"scene": {"setting": "a quiet harbour", "style": style,
                           "people": [{"who": "maya", "wearing": "a coat", "doing": "waving"}]}}),
                    &cx,
                )
                .await
                .unwrap();
            assert!(
                out.is_error && out.content.contains(says),
                "{}",
                out.content
            );
        }
        assert_eq!(posts(), posts_before + 1);

        // A description longer than any library name is left out the same
        // way, never a shape refusal.
        let long = "a slow dreamy wash of muted teal and amber with heavy grain and soft edges";
        assert!(long.chars().count() > crate::imagelib::MAX_NAME);
        let out = t
            .call(
                json!({"scene": {"setting": "a quiet harbour", "style": long,
                       "people": [{"who": "maya", "wearing": "a coat", "doing": "waving"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("There is no approved style"),
            "{}",
            out.content
        );
        assert!(
            !out.content.contains(long),
            "the echo is clipped: {}",
            out.content
        );
        assert_eq!(posts(), posts_before + 2);

        // With no library at all, nothing is offered to choose from.
        let bare = tool(&url)
            .call(
                json!({"picture": picture_of(&again.content), "scene": {"style": "oil on linen"}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(bare.is_error, "{}", bare.content);
        assert!(
            bare.content
                .contains("There is no approved style `oil on linen`. Leave `style` out"),
            "{}",
            bare.content
        );
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// A style on a picture with no record edits it in that style: the
    /// library's own words for the style reach the prompt, and the result
    /// names it (review of #597).
    #[tokio::test]
    async fn a_style_edit_carries_the_styles_own_words() {
        let (url, seen) = distinct(1).await;
        let (dir, lib) = (tempdir(), library_with(&["maya"]));
        crate::imagelib::create(
            &lib,
            crate::imagelib::NewEntry {
                kind: crate::imagelib::Kind::Style,
                name: "ink-wash".into(),
                text: "soft grey ink wash on rice paper".into(),
                portrait: None,
                source_seed: None,
                origin: crate::imagelib::Origin::Owner,
                locked: false,
            },
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("inbox/her.png"), picture(20, [200, 30, 30])).unwrap();
        let out = tool(&url)
            .with_library_dir(lib.clone())
            .call(
                json!({"picture": "inbox/her.png", "scene": {"style": "ink-wash"}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("style ink-wash (v1)"),
            "{}",
            out.content
        );
        let prompt = last_prompt(&seen);
        assert!(
            prompt.contains("ink-wash style")
                && prompt.contains("soft grey ink wash on rice paper"),
            "{prompt}"
        );
        for d in [dir, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// A restage drawn new from the setting's words keeps the picture's
    /// shape: the base seed holds the room only at the same latent size
    /// (mecha-a3's G4 on #597: a landscape base came back square, its window
    /// gone). A size the call names still wins.
    #[tokio::test]
    async fn a_words_restage_keeps_the_pictures_shape() {
        let wide = |c: u8| {
            let img = image::RgbImage::from_fn(64, 36, |x, _| image::Rgb([c, (x * 3) as u8, 90]));
            let mut png = std::io::Cursor::new(Vec::new());
            img.write_to(&mut png, image::ImageFormat::Png).unwrap();
            png.into_inner()
        };
        let (url, seen) = fake_with(Fake {
            history: vec![done(); 3],
            views: vec![wide(10), wide(120), wide(200)],
            ..Fake::default()
        })
        .await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let width = |seen: &Arc<Mutex<Vec<String>>>| {
            let p = last_prompt(seen);
            let body: Value = serde_json::from_str(&p[p.find('{').unwrap()..]).unwrap();
            body["prompt"]["latent"]["inputs"]["width"].as_u64()
        };
        let first = t
            .call(
                json!({"scene": {"setting": "a study with a tall window",
                       "people": [{"who": "maya", "wearing": "a coat", "doing": "reading"}]},
                       "size": "landscape"}),
                &cx,
            )
            .await
            .unwrap();
        assert_eq!(width(&seen), Some(1344));
        let p1 = picture_of(&first.content);
        let second = t
            .call(
                json!({"picture": p1, "scene": {"people": [{"who": "maya", "doing": "standing"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(second.content.contains("restaged"), "{}", second.content);
        assert_eq!(width(&seen), Some(1344), "the restage kept the shape");
        let p2 = picture_of(&second.content);
        t.call(
            json!({"picture": p2, "scene": {"camera": "from above"}, "size": "square"}),
            &cx,
        )
        .await
        .unwrap();
        assert_eq!(width(&seen), Some(1024), "a size the call names wins");
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// A mask the chat does not hold is left out and said, and the retouch
    /// is made to the whole picture (mecha-a3's G1b: a model filled `mask`
    /// with prose or a name 3 times in 65).
    #[tokio::test]
    async fn a_mask_that_is_not_in_the_chat_is_left_out_and_said() {
        let (url, seen) = distinct(1).await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("inbox/her.png"), picture(20, [200, 30, 30])).unwrap();
        let out = tool(&url)
            .call(
                json!({"picture": "inbox/her.png", "retouch": "a red hat",
                       "mask": "inbox/never-painted.png"}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content
                .contains("The mask inbox/never-painted.png was left out"),
            "{}",
            out.content
        );
        assert_eq!(uploads(&seen), 1, "the picture, and no mask");
        std::fs::remove_dir_all(dir).ok();
    }

    /// A character whose library entry did not load, named anywhere in the
    /// call's words, is refused before the GPU: the planner's name check
    /// cannot see a broken entry, and the words alone would draw a stranger
    /// (review of #383; review of #597, pass 4).
    #[tokio::test]
    async fn a_broken_entry_named_in_prose_is_refused_before_the_gpu() {
        let lib = library_with(&["maya", "john"]);
        std::fs::write(lib.join("characters/john/entry.toml"), "not = [toml").unwrap();
        let dir = tempdir();
        let out = tool("http://127.0.0.1:1")
            .with_library_dir(lib.clone())
            .call(
                json!({"scene": {"setting": "a kitchen", "people": [
                    {"who": "maya", "wearing": "a coat", "doing": "handing a cup to John"}]}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(
            out.content
                .contains("John's library entry could not be read"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// The record keeps the seed that drew its room: an edit between a new
    /// picture and a restage samples fresh but leaves the base's seed, so
    /// the restage still keeps the room (review of #597, pass 7).
    #[tokio::test]
    async fn an_edit_between_keeps_the_rooms_seed_for_a_restage() {
        let (url, seen) = distinct(3).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let first = t
            .call(
                json!({"scene": {"setting": "a greenhouse", "people": [
                    {"who": "maya", "wearing": "a coat", "doing": "watering plants"}]}, "seed": 77}),
                &cx,
            )
            .await
            .unwrap();
        assert_eq!(seed_in(&seen), 77);
        let p1 = picture_of(&first.content);
        let edited = t
            .call(
                json!({"picture": p1, "scene": {"people": [{"who": "maya", "wearing": "a red scarf"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(edited.content.contains("An edit of"), "{}", edited.content);
        let p2 = picture_of(&edited.content);
        let restaged = t
            .call(
                json!({"picture": p2, "scene": {"people": [{"who": "maya", "doing": "sitting on a bench"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(
            restaged.content.contains("restaged"),
            "{}",
            restaged.content
        );
        assert_eq!(seed_in(&seen), 77, "the restage drew at the room's seed");
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// An unmasked edit encodes its references at 512 and names its output
    /// size in the canvas's own shape; on a room photo that shape is kept
    /// even when another was asked, and said (owner, 2026-10-08).
    #[tokio::test]
    async fn an_edit_encodes_small_and_a_room_keeps_its_shape() {
        let (url, seen) = distinct(3).await;
        let (dir, lib) = (tempdir(), library_with(&["maya"]));
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        // A landscape room, 4:3.
        let room = {
            let img = image::RgbImage::from_pixel(64, 48, image::Rgb([90, 120, 90]));
            let mut png = std::io::Cursor::new(Vec::new());
            img.write_to(&mut png, image::ImageFormat::Png).unwrap();
            png.into_inner()
        };
        std::fs::write(dir.join("inbox/room.png"), room).unwrap();
        let out = tool(&url)
            .with_library_dir(lib.clone())
            .call(
                json!({"size": "portrait", "scene": {"setting": {"photo": "inbox/room.png"},
                       "people": [{"who": "maya", "wearing": "a coat", "doing": "sitting"}]}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("The room photo's own shape was kept"),
            "{}",
            out.content
        );
        let p = last_prompt(&seen);
        let body: Value = serde_json::from_str(&p[p.find('{').unwrap()..]).unwrap();
        assert_eq!(body["prompt"]["encode"]["inputs"]["resolution"], 512);
        let (w, h) = (
            body["prompt"]["latent"]["inputs"]["width"]
                .as_u64()
                .unwrap(),
            body["prompt"]["latent"]["inputs"]["height"]
                .as_u64()
                .unwrap(),
        );
        assert!(w > h, "the landscape room kept its shape: {w}x{h}");
        // A photo stored sideways is sized as the page shows it, upright.
        assert_eq!(
            canvas_dims(&jpeg_with_orientation(64, 48, 6)),
            Some((896, 1184))
        );
        assert_eq!(
            canvas_dims(&std::fs::read(dir.join("inbox/room.png")).unwrap()),
            Some((1184, 896))
        );
        // A very wide room keeps its shape: the encoder's own arithmetic,
        // never a per-side clamp that narrows only the width.
        let wide = png_bytes(&image::RgbImage::new(160, 30)).unwrap();
        assert_eq!(canvas_dims(&wide), Some(edit_canvas(160, 30, 1024)));
        assert!(canvas_dims(&wide).unwrap().0 > 2048);

        // An unmasked retouch of a picture: named at the picture's shape,
        // references at 1024, since no crop carries who is in it and the
        // canvas alone at 512 lost them (mecha-a3, 2026-10-08).
        let png = |w, h| {
            png_bytes(&image::RgbImage::from_pixel(
                w,
                h,
                image::Rgb([200, 90, 40]),
            ))
            .unwrap()
        };
        std::fs::write(dir.join("inbox/tall.png"), png(48, 64)).unwrap();
        let latent = |p: &str| -> (Value, Value) {
            let body: Value = serde_json::from_str(&p[p.find('{').unwrap()..]).unwrap();
            (
                body["prompt"]["encode"]["inputs"]["resolution"].clone(),
                body["prompt"]["sample"]["inputs"]["latent_image"].clone(),
            )
        };
        let out = tool(&url)
            .call(
                json!({"picture": "inbox/tall.png", "retouch": "a red hat"}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let p = last_prompt(&seen);
        let body: Value = serde_json::from_str(&p[p.find('{').unwrap()..]).unwrap();
        assert_eq!(latent(&p).0, 1024);
        assert_eq!(body["prompt"]["latent"]["inputs"]["width"], 896);
        assert_eq!(body["prompt"]["latent"]["inputs"]["height"], 1184);

        // A canvas whose shape cannot be read keeps 1024 references and
        // names no size, so the encoder sizes the picture from it: never a
        // picture drawn at the references' 512.
        let mut broken = png(64, 48);
        broken.truncate(60);
        std::fs::write(dir.join("inbox/broken.png"), broken).unwrap();
        let out = tool(&url)
            .call(
                json!({"picture": "inbox/broken.png", "retouch": "a red hat"}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let p = last_prompt(&seen);
        assert_eq!(latent(&p), (json!(1024), json!(["encode", 2])));
        for d in [dir, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// A `picture` the model made up, beside a scene, is left out and said,
    /// and the scene draws as new; alone it is refused saying where pictures
    /// come from (a live run retried a bare "cannot open" eleven times,
    /// 2026-10-08). A path the jail refuses is still refused.
    #[tokio::test]
    async fn a_made_up_picture_is_left_out_beside_a_scene() {
        let (url, seen) = distinct(1).await;
        let dir = tempdir();
        let out = tool(&url)
            .call(
                json!({"picture": "images/made_up_name.jpg",
                       "scene": {"setting": "a sunny porch", "people": [
                           {"who": "a woman in a green raincoat", "wearing": "a green raincoat", "doing": "waving"}]}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("A new"), "{}", out.content);
        assert!(
            out.content
                .contains("There is no picture `images/made_up_name.jpg` in this chat"),
            "{}",
            out.content
        );
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("POST /prompt")));
        let alone = tool(&url)
            .call(
                json!({"picture": "images/made_up_name.jpg", "retouch": "a red hat"}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(
            alone.is_error && alone.content.contains("leave `picture` out"),
            "{}",
            alone.content
        );
        let outside = tool(&url)
            .call(
                json!({"picture": "/etc/passwd", "scene": {"setting": "a porch"}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(
            outside.is_error && outside.content.contains("is not a file in the workspace"),
            "the jail refuses it, never the absorb: {}",
            outside.content
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
