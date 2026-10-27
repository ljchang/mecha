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

use crate::comfy::{clip, Failure, SERVER_WAIT};
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
    /// The turbo distillation a persona's owner may choose instead
    /// (`sampling::Model::Turbo`): fewer steps, no guidance, the same text
    /// encoder and VAE.
    pub turbo_model: String,
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
    /// the default covers that with a little room. A Mac's default is the
    /// bf16 file's (`min_available_mb_on`).
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
    /// The folders the server loads LoRAs from (ComfyUI's `models/loras`,
    /// and any `extra_model_paths.yaml` adds), looked in for each picture
    /// entry of the owner's LoRA registry, whose file is checked against
    /// them (`loras`). Unset, a ComfyUI that mecha installed means its own
    /// `models/loras`; any other server's picture LoRAs are left out until
    /// it is set.
    pub loras_dir: Option<Vec<std::path::PathBuf>>,
}

impl ImageConfig {
    /// The diffusion model file a picture sampled as `model` loads.
    pub fn model_file(&self, model: crate::sampling::Model) -> &str {
        match model {
            crate::sampling::Model::Standard => &self.diffusion_model,
            crate::sampling::Model::Turbo => &self.turbo_model,
        }
    }
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
            // runs on native int8 kernels. bf16 on a Mac, which has none
            // (`sampling::standard_model`).
            diffusion_model: crate::sampling::standard_model().into(),
            turbo_model: crate::sampling::turbo_model().into(),
            text_encoder: "qwen3vl_8b_w4a8.safetensors".into(),
            vae: "qwen_image_2.1_vae_bf16.safetensors".into(),
            steps: 40,
            timeout_secs: 600,
            // The load cost is derived from this figure (`load_cost_mb_on`).
            min_available_mb: min_available_mb_on(std::env::consts::OS),
            // Fifteen minutes on Linux, matching `comfyui-idle-reset`'s
            // window: a picture inside a quarter hour of the last is drawn
            // warm. A cold one took 145–180 s against 36–63 s warm,
            // 2026-10-08, the restart wait included (the owner's ruling, that
            // day). Not armed behind a Mac's on-demand proxy
            // (`behind_mecha_s_proxy`).
            unload_after_secs: 900,
            server_temp_dir: None,
            loras_dir: None,
        }
    }
}

pub use crate::comfy::is_loopback;

/// The image server's URL, if it is on this machine
/// ([`crate::comfy::loopback_url`]): the tool declares no egress, and this is
/// what makes that true.
pub fn loopback_url(raw: &str) -> Result<reqwest::Url> {
    crate::comfy::loopback_url("[image] url", "the image tool", raw)
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
    /// Width and height, or `None` to follow the first reference's shape —
    /// which is only ever `None` when there is a reference to follow.
    pub size: Option<(u32, u32)>,
    /// How the sampler runs: the model, steps, guidance, sampler and
    /// scheduler, and the negative prompt (`sampling::Tuning::resolve`).
    pub sampling: crate::sampling::Sampling,
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
    /// The owner's picture LoRAs from settings (`loras::for_picture`),
    /// chained between the model's loader and the sampler in this order.
    /// Never the model's: the tool's schema has no such field.
    pub loras: Vec<crate::loras::PictureLora>,
}

/// The reference size for a masked edit: the canvas at full detail, since
/// the result is laid back over the original pixel for pixel.
pub const EDIT_REFERENCE_SIZE: u32 = 1024;

/// What [`ImageGenerate::upscale`] draws at, in pixels: the encoder's canvas
/// at [`EDIT_REFERENCE_SIZE`]. A picture with this many already gains
/// nothing. A video reference is scaled down to the clip's area, about the
/// same, and never up, so a smaller portrait reaches a clip as fewer
/// tokens: a 401 px portrait redrawn here matched its character at 0.53 in
/// a clip, against 0.45 as it was (2026-10-24).
pub const UPSCALE_PIXELS: u32 = EDIT_REFERENCE_SIZE * EDIT_REFERENCE_SIZE;

/// The redraw's words: the same photograph, larger, and nothing else.
/// Glasses and jewellery are "any", so a face without them is not asked
/// for them.
const UPSCALE_PROMPT: &str = "Recreate this exact photograph at high resolution. The same person \
    with the identical face, facial structure, eyes, nose, mouth, hair, expression, any glasses or \
    jewellery, skin tone, clothing, background, lighting and framing as <image1>. Sharp focus, fine \
    natural skin texture with visible pores, crisp detail in the eyes and hair. Change nothing \
    else.";

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
        .wire
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

/// The segmentation checkpoint, as the image server lists it: SAM 3.1 in
/// ComfyUI's own fp16 repackaging (`docs/SEGMENTATION-DESIGN.md` §3).
pub const SEGMENT_MODEL: &str = "sam3.1_multiplex_fp16.safetensors";

/// The longest words a mask is asked for by: as long as any other short
/// prose field the tool takes, and far past "the blue scarf on the chair".
pub const MASK_WORDS_CAP: usize = 120;

/// ComfyUI's graph for one SAM 3.1 segmentation (`docs/SEGMENTATION-DESIGN.md`):
/// the picture uploaded as `uploaded`, `words` as the text prompt, and the
/// union of every match out as a white-on-black picture. ComfyUI's own nodes
/// (`comfy_extras/nodes_sam3.py`), as its SAM3 blueprint wires them; the
/// threshold and refinement are the blueprint's.
pub fn sam_graph(uploaded: &str, words: &str) -> Value {
    json!({
        "ckpt": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": SEGMENT_MODEL}},
        "text": {"class_type": "CLIPTextEncode", "inputs": {"clip": ["ckpt", 1], "text": words}},
        "img": {"class_type": "LoadImage", "inputs": {"image": format!("{uploaded} [temp]")}},
        "sam": {"class_type": "SAM3_Detect", "inputs": {
            "model": ["ckpt", 0], "image": ["img", 0], "conditioning": ["text", 0],
            "threshold": 0.5, "refine_iterations": 2, "individual_masks": false}},
        "toimg": {"class_type": "MaskToImage", "inputs": {"mask": ["sam", 0]}},
        "out": {"class_type": "PreviewImage", "inputs": {"images": ["toimg", 0]}},
    })
}

/// How many separate areas a segmentation found: the 4-connected white
/// parts of `mask`, leaving out specks under a thousandth of the picture,
/// which a refinement pass can strand at an edge. Said beside the edit,
/// because a mask the model named is a guess (`docs/SEGMENTATION-DESIGN.md` S1).
pub(crate) fn found_areas(mask: &image::GrayImage) -> usize {
    let (w, h) = mask.dimensions();
    let speck = (w as usize * h as usize) / 1000;
    let mut seen = vec![false; w as usize * h as usize];
    let mut areas = 0;
    let mut stack = Vec::new();
    for start in 0..seen.len() {
        let (sx, sy) = ((start % w as usize) as u32, (start / w as usize) as u32);
        if seen[start] || mask.get_pixel(sx, sy).0[0] <= 127 {
            continue;
        }
        seen[start] = true;
        stack.push((sx, sy));
        let mut size = 0;
        while let Some((x, y)) = stack.pop() {
            size += 1;
            let near = [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ];
            for (nx, ny) in near {
                if nx >= w || ny >= h {
                    continue;
                }
                let i = ny as usize * w as usize + nx as usize;
                if !seen[i] && mask.get_pixel(nx, ny).0[0] > 127 {
                    seen[i] = true;
                    stack.push((nx, ny));
                }
            }
        }
        if size > speck {
            areas += 1;
        }
    }
    areas
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
        "clip": ["clip", 0], "prompt": with_triggers(&req.prompt, &req.loras),
        "negative_prompt": req.sampling.negative, "resolution": req.reference_size});
    let model = cfg.model_file(req.sampling.model);
    let (loader, mut unet) = unet_loader(model);
    unet["unet_name"] = json!(model);
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
    // One loader per LoRA, each patching the model the last one gave
    // (`docs/LORA-DESIGN.md` §5), as the clip graph chains them.
    let mut model = json!(["unet", 0]);
    for (i, l) in req.loras.iter().enumerate() {
        let node = format!("lora{}", i + 1);
        graph[&node] = json!({"class_type": "LoraLoaderModelOnly", "inputs": {
            "model": model, "lora_name": l.file, "strength_model": l.strength}});
        model = json!([node, 0]);
    }
    graph["sample"] = json!({"class_type": "KSampler", "inputs": {
        "model": model, "positive": ["encode", 0], "negative": ["encode", 1],
        "latent_image": canvas, "seed": req.seed, "steps": req.sampling.steps,
        // Guidance off (cfg 1) is the reference setting; above 1 doubles
        // every step. Each value is from a closed range or list
        // (`sampling::Tuning::checked`), never the model's.
        "cfg": req.sampling.cfg, "sampler_name": req.sampling.sampler,
        "scheduler": req.sampling.scheduler, "denoise": 1.0}});
    graph
}

/// Whether the owner's picture LoRAs apply to a picture that loads
/// `diffusion_model`, and why not when they do not (`docs/LORA-DESIGN.md`
/// §5). Checked per picture on the file it actually loads, which is the
/// turbo file for a turbo picture, never on one config key. M1 and M2
/// (2026-10-27) measured both int8 files, standard and turbo: every key
/// patched, the trained likeness drawn on both, about a second more a
/// picture. A `.gguf` file patches through its own loader, which was not
/// measured, so nothing applies to it.
pub fn model_lora(diffusion_model: &str) -> std::result::Result<(), &'static str> {
    if unet_loader(diffusion_model).0 == "UnetLoaderGGUF" {
        return Err("LoRAs are not applied to a .gguf model file");
    }
    Ok(())
}

/// What a picture's result says of the owner's LoRAs: counts and reasons,
/// never a name, since the model has no say in them (`docs/LORA-DESIGN.md`
/// §4). Empty when the settings list none.
pub fn loras_said(p: &crate::loras::Picked, off: Option<&str>, triggers: bool) -> String {
    let n = |k: usize| {
        if k == 1 {
            "1 LoRA".to_string()
        } else {
            format!("{k} LoRAs")
        }
    };
    let mut out = String::new();
    if !p.applied.is_empty() {
        out.push_str(&format!(
            " Picture settings applied {}.",
            n(p.applied.len())
        ));
        if triggers {
            out.push_str(" Settings added trigger words to the prompt.");
        }
    }
    if let Some(why) = off {
        out.push_str(&format!(
            " The picture settings' LoRAs were left out: {why}."
        ));
    }
    if p.refused > 0 {
        out.push_str(&format!(
            " {} in picture settings {} left out because {} file did not check.",
            n(p.refused),
            if p.refused == 1 { "was" } else { "were" },
            if p.refused == 1 { "its" } else { "their" },
        ));
    }
    if p.gone > 0 {
        out.push_str(&format!(
            " {} in picture settings {} no longer in the registry.",
            n(p.gone),
            if p.gone == 1 { "is" } else { "are" },
        ));
    }
    if p.damaged {
        out.push_str(" The LoRA list in picture settings could not be read, so none applied.");
    }
    if p.unreadable {
        out.push_str(
            " The LoRA registry could not be read, so no LoRA in picture settings applied.",
        );
    }
    out
}

/// The manifest's `loras`: the same counts, or null when settings list none.
fn loras_counted(p: &crate::loras::Picked, off: Option<&str>) -> Value {
    if p.applied.is_empty()
        && off.is_none()
        && p.refused == 0
        && p.gone == 0
        && !p.damaged
        && !p.unreadable
    {
        return Value::Null;
    }
    json!({
        "applied": p.applied.len(),
        "left_out": off,
        "refused": p.refused,
        "gone": p.gone,
        "damaged": p.damaged,
        "registry_unreadable": p.unreadable,
    })
}

/// The prompt with each LoRA's trigger words where it does not already say
/// them (any case), ahead of it: a picture prompt has no field to insert
/// into, so this is `videogen::loras::with_triggers`'s prefix branch.
pub fn with_triggers(prompt: &str, loras: &[crate::loras::PictureLora]) -> String {
    let lower = prompt.to_lowercase();
    let mut missing: Vec<&str> = Vec::new();
    for t in loras.iter().filter_map(|l| l.trigger.as_deref()) {
        if !lower.contains(&t.to_lowercase()) && !missing.contains(&t) {
            missing.push(t);
        }
    }
    if missing.is_empty() {
        return prompt.to_string();
    }
    format!("{}. {prompt}", missing.join(", "))
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
pub fn memory_need_mb(loaded: Option<bool>, min_mb: u64, may_segment: bool) -> u64 {
    // A job that may segment asks for SAM too, warm or not: the gauge cannot
    // tell it apart from the image model once either has loaded (§3).
    let sam = if may_segment { SEGMENT_NEED_MB } else { 0 };
    sam + match loaded {
        // What a load costs is what a loaded server has already paid; the
        // rest of the operator's figure is their margin, theirs to keep
        // (review of #515). A need computed down to 0 is "the reserve
        // alone", not "check off": `memory_guard` reads the operator's 0 off
        // the config, and the floor refuses an unreadable gauge whatever the
        // need.
        Some(true) => min_mb.saturating_sub(load_cost_mb()),
        Some(false) | None => min_mb,
    }
}

/// Whether this Mac's ComfyUI is mecha's own, run behind its on-demand
/// proxy, which stops the whole process after fifteen idle minutes
/// (`comfyui_unit::for_mac`). There `[image] unload_after_secs` is not
/// armed: a `/free` would reach the proxy and either reset its idle clock —
/// ComfyUI stopped half an hour after the last picture, not fifteen
/// minutes — or cold-start ComfyUI only to tell it to free (review of #689).
/// Keyed on that agent, not on the OS: a ComfyUI a Mac's owner runs by hand
/// has no proxy, and keeps the timer (review of #689, pass 2).
fn behind_mecha_s_proxy(url: &str) -> bool {
    cfg!(target_os = "macos") && dirs::home_dir().is_some_and(|h| behind_proxy_at(&h, url))
}

/// [`behind_mecha_s_proxy`] under `home`: the configured URL names the
/// shipped port, and the agent there runs the on-demand proxy — asked of the
/// plist, not taken from its existing. An agent installed before the proxy
/// ran ComfyUI on that port itself, and the timer is all that ever freed it;
/// a URL elsewhere is a ComfyUI the owner runs (review of #689).
pub(crate) fn behind_proxy_at(home: &std::path::Path, url: &str) -> bool {
    let shipped = crate::comfyui_unit::Naming::shipped();
    let names_port = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.port_or_known_default())
        == Some(shipped.port);
    let plist = crate::launchd::agents_dir(home).join(crate::launchd::plist_name(&shipped.unit));
    names_port
        && std::fs::read_to_string(plist).is_ok_and(|t| t.contains("<string>on-demand</string>"))
}

/// The default `[image] min_available_mb` on `os`: what a picture from cold
/// needs free.
///
/// - **Linux**: 20 GiB since the int8 default — its ~19.2 GiB load no longer
///   fit under the Q4's 19 (2026-10-04).
/// - **macOS**: 34 GiB, the bf16 file's (FEATURES-DESIGN §12.3): a load from
///   cold peaked at 32.7 GiB of process RSS on an M5 Max (2026-10-13; the
///   image slot's Mac row records it as 33,485 MB), rounded up to a whole
///   GiB as Linux's figure is. A 36-48 GB Mac holding its chat model does
///   not have it, and the gate says so rather than letting the load swap.
pub fn min_available_mb_on(os: &str) -> u64 {
    if os == "macos" {
        34_816
    } else {
        20_480
    }
}

/// What every segmentation in one picture job shares, apart from the render's
/// own timeout: a segmentation takes 1-5 s (`docs/SEGMENTATION-DESIGN.md`
/// §3), so this is margin for a cold load, never a wait the owner sees.
const SEGMENT_BUDGET: Duration = Duration::from_secs(120);

/// Why [`ComfyUi::can_segment`] has no answer: the server could not be
/// asked, which admission reads as unknown, never as "no SAM".
const SEGMENT_UNASKED: &str = "the image server could not be asked about segmentation";

/// What a segmentation adds to a picture job's need, in MB: SAM 3.1 loaded
/// beside the image model (`docs/SEGMENTATION-DESIGN.md` §3). Its cold load
/// peaked at 2,528 MB of GPU memory above a freed server on the GB10
/// workstation (2026-10-16), where the GPU's memory is the system's, rounded
/// up to a whole GiB as the other figures are. Not measured on a Mac, where
/// the same figure is asked as a floor (`recommend`'s Mac image row says so).
pub const SEGMENT_NEED_MB: u64 = 3_072;

/// What a reload from a `/free`d state still needs on `os` — the worse of
/// the two states `/system_stats` cannot tell apart. Linux: ~12 GiB,
/// measured with the Q4 GGUF on 2026-10-02, not re-measured for the int8
/// file. macOS: 14 GiB, the 13,571 MB ComfyUI's log says the bf16 model
/// loads to on MPS (2026-10-13) rounded up — a reload's own cost is not
/// measured there, and what `/free` gives back on MPS is not either. A Mac's
/// server never reads as loaded (`loaded_from_stats` is `None` on MPS), so
/// its figure is kept only so the arithmetic stays whole, not one a Mac asks.
fn reload_mb_on(os: &str) -> u64 {
    if os == "macos" {
        14_336
    } else {
        12_288
    }
}

/// What loading the model costs on `os`, which a server that has loaded it
/// has already paid: the default cold figure less a reload's need.
pub fn load_cost_mb_on(os: &str) -> u64 {
    min_available_mb_on(os).saturating_sub(reload_mb_on(os))
}

/// [`load_cost_mb_on`] on this system.
pub fn load_cost_mb() -> u64 {
    load_cost_mb_on(std::env::consts::OS)
}

/// Whether ComfyUI's `/system_stats` says it has loaded a model since it
/// started. On CUDA its `torch_vram_total` is 0 in a fresh process and stays
/// above 0 once anything has loaded - through a `/free`, too, so it answers
/// "has loaded", never "holds now". `None` for a reply this cannot read.
///
/// **Read on CUDA only; any other device is `None`, whatever the figure.**
/// ComfyUI's `get_total_memory` answers the machine's whole RAM for MPS and
/// the CPU, so the field is above 0 from the first request and says nothing
/// about a load — read as warm, a Mac's cold 34 GiB would never be asked
/// (review of #681). An allowlist, not a denylist: a device type this has
/// not seen, or a reply naming none, is unknown, and unknown is never warm
/// (review of #681, pass 2).
pub fn loaded_from_stats(stats: &Value) -> Option<bool> {
    let device = stats.pointer("/devices/0")?;
    if device.get("type").and_then(Value::as_str) != Some("cuda") {
        return None;
    }
    let total = device.get("torch_vram_total")?.as_f64()?;
    Some(total > 0.0)
}

/// A picture's fresh seed: the next from the stream a replay stamped
/// (`ToolCtx::image_seeds`), else at random. In the range `fresh_seed` draws.
fn draw_seed(ctx: &ToolCtx) -> u64 {
    match &ctx.image_seeds {
        Some(stream) => stream.next() & 0xFFFF_FFFF,
        None => fresh_seed(),
    }
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

/// The ComfyUI adapter: the shared transport ([`crate::comfy::Client`])
/// and what this tool keeps about the server beyond it.
struct ComfyUi {
    wire: crate::comfy::Client,
    /// Where the model each server holds is written for every process that
    /// draws (`ImageGenerate::with_model_record`): a serve restarted after a
    /// turbo picture, or a replay beside serve, otherwise read an unused
    /// server as holding the standard model and loaded it beside turbo
    /// (mecha-a3's gate of #648). Unset (tests), this process's memory alone.
    model_record: std::sync::OnceLock<std::path::PathBuf>,
    /// The server has been seen able to segment ([`ComfyUi::segments`]).
    /// Only a yes is kept: an install that adds SAM is seen on the next ask.
    segments: std::sync::atomic::AtomicBool,
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

/// The diffusion model file this process last sent each server, by its
/// address: a picture that needs the other one unloads it first (the owner's
/// ruling, 2026-10-11: "switching to turbo mode should unload the standard
/// model"), since ComfyUI otherwise keeps both resident, ~7 GB more against
/// the memory floor. Process-wide, because each persona's form has its own
/// client and one server draws for them all. Another process's pictures are
/// unseen here, so a server this process has not used is read as holding the
/// standard model: the common case pays no extra unload.
static LAST_MODEL: std::sync::Mutex<std::collections::BTreeMap<String, String>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// The shared held-model record, by server address; unreadable reads empty,
/// so a server reads as holding the standard model, as an unused one does.
fn read_held(path: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Written whole beside itself and renamed over, so a reader never sees half.
fn write_held(
    path: &std::path::Path,
    held: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Unique per write: two chats in one serve can draw at once (review of
    // #648, pass 3).
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let tmp = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, serde_json::to_vec(held)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// How long a job waits after asking the server to unload, so the unload
/// runs before the job: ComfyUI's worker wakes for the flag and acts on it
/// only once its queue is empty, and a prompt that lands first is drawn
/// with both models resident.
#[cfg(not(test))]
const UNLOAD_SETTLE: Duration = Duration::from_secs(2);
#[cfg(test)]
const UNLOAD_SETTLE: Duration = Duration::from_millis(10);

impl ComfyUi {
    /// Whether a picture drawn with `file` changes the model the server
    /// holds (`LAST_MODEL`); `standard` is what an unused server is read as.
    fn switching(&self, file: &str, standard: &str) -> bool {
        match self.held().as_deref() {
            // Released since its last picture: it holds nothing to unload.
            Some("") => false,
            Some(held) => held != file,
            None => standard != file,
        }
    }

    /// Note that this server now holds no model: an idle release unloaded
    /// it, so the next picture switches nothing, whichever model it asks for
    /// (review of #648).
    fn released(&self) {
        self.drew_with("");
    }

    /// The model this server last drew with, as the shared record says when
    /// there is one, else as this process saw it. `""` is a released server.
    fn held(&self) -> Option<String> {
        match self.model_record.get() {
            // The record wins where it names this server: another process may
            // have drawn since. Where it cannot be read or has no entry (a
            // write that failed), what this process sent last, never the
            // standard model by default (review of #648, pass 5).
            Some(path) => read_held(path)
                .remove(self.wire.base().as_str())
                .or_else(|| self.held_here()),
            None => self.held_here(),
        }
    }

    /// What this process last sent this server, or released it to.
    fn held_here(&self) -> Option<String> {
        LAST_MODEL
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(self.wire.base().as_str())
            .cloned()
    }

    /// Note `file` as the model this server now draws with. A record that
    /// cannot be written costs only the next switch's unload.
    fn drew_with(&self, file: &str) {
        LAST_MODEL
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.wire.base().to_string(), file.to_string());
        if let Some(path) = self.model_record.get() {
            let mut held = read_held(path);
            held.insert(self.wire.base().to_string(), file.to_string());
            if let Err(e) = write_held(path, &held) {
                tracing::warn!("image_generate: the held-model record: {e:#}");
            }
        }
    }

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
        Ok(ComfyUi {
            wire: crate::comfy::Client::new(loopback_url(&cfg.url)?, "the image server")?,
            model_record: std::sync::OnceLock::new(),
            segments: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Every file the graph names is one the server can load, checked before
    /// submitting: a missing file otherwise fails deep in a run as a node
    /// validation error the model cannot act on.
    ///
    /// A persona's picture settings are checked the same way, and a value the
    /// server lacks is refused naming the setting, since it stays chosen until
    /// the owner changes it (review of #648): the turbo file, and a sampler
    /// or scheduler other than the reference ones, which every ComfyUI has.
    async fn preflight(&self, cfg: &ImageConfig, s: &crate::sampling::Sampling) -> Result<()> {
        let model = cfg.model_file(s.model).to_string();
        let reference = crate::sampling::Sampling::reference(s.steps);
        let turbo = s.model == crate::sampling::Model::Turbo;
        let mut checks = vec![
            (unet_loader(&model).0, "unet_name", model.clone(), turbo),
            ("CLIPLoader", "clip_name", cfg.text_encoder.clone(), false),
            ("VAELoader", "vae_name", cfg.vae.clone(), false),
        ];
        if s.sampler != reference.sampler {
            checks.push(("KSampler", "sampler_name", s.sampler.clone(), true));
        }
        if s.scheduler != reference.scheduler {
            checks.push(("KSampler", "scheduler", s.scheduler.clone(), true));
        }
        // One request per node: the guided preset checks two KSampler inputs.
        let mut fetched: std::collections::BTreeMap<&str, Value> = Default::default();
        for (node, input, want, chosen) in checks {
            let info = match fetched.get(node) {
                Some(info) => info.clone(),
                None => {
                    let info = self.wire.get_json(&format!("object_info/{node}")).await?;
                    fetched.insert(node, info.clone());
                    info
                }
            };
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
                // The turbo model is fetched on demand, so its absence has
                // a command that ends it (the owner, 2026-10-11).
                // Only the pinned file: `--turbo` fetches nothing else, and a
                // configured `turbo_model` it cannot supply keeps the
                // settings' own refusal (review of #650).
                if turbo && input == "unet_name" && want == crate::sampling::turbo_model() {
                    bail!(
                        "the image server has no `{want}`, the turbo model this persona's \
                         picture settings choose — `mecha features enable image --turbo` \
                         fetches it into the image server mecha installs; or choose the \
                         standard model on the persona's page"
                    );
                }
                if chosen {
                    bail!(
                        "the image server has no `{want}` for {node}, which this persona's \
                         picture settings choose; change them on the persona's page, or add it \
                         to the image server"
                    );
                }
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
        let info = self
            .wire
            .get_json("object_info/TextEncodeQwenImage21")
            .await?;
        if info.get("TextEncodeQwenImage21").is_none() {
            bail!("the image server's ComfyUI predates Qwen-Image 2.1 support; update it");
        }
        Ok(())
    }

    /// Stop a job wherever it is ([`crate::comfy::Client::take_off`]), and
    /// say what it wrote: a job that finished as it was stopped also wrote
    /// its preview, which only its record names — so it is read before the
    /// record is forgotten.
    async fn abandon(&self, id: &str, trail: Option<&std::path::Path>) -> Wrote {
        let wrote = self
            .wire
            .take_off(id)
            .await
            .map(|entry| outputs(&entry))
            .unwrap_or_default();
        // Onto the trail before the record naming them is forgotten, as the
        // polling path does: a death between here and the caller's delete
        // otherwise leaves a preview nothing names (found on review of #331).
        for name in &wrote.temp {
            if let Err(e) = note(trail, &TrailEntry::File(name.clone())) {
                tracing::debug!("a preview's name did not reach the image trail: {e:#}");
            }
        }
        self.wire.forget(id).await;
        wrote
    }

    /// Put one reference in the server's temp directory as `filename` (a
    /// random name the caller has already written to the trail), and return
    /// the name the server filed it under.
    async fn upload(&self, reference: &Reference, filename: &str) -> Result<String> {
        self.wire
            .upload_temp(&reference.bytes, filename, &reference.path)
            .await
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

    /// [`ComfyUi::can_segment`] as a yes or no for admission, a yes kept for
    /// the process: a call is charged SAM's memory only where SAM can load
    /// (review of #710, pass 2).
    /// A server that could not be asked is charged, never admitted light:
    /// unknown is never "no SAM" (review of #710, pass 3).
    async fn segments(&self) -> bool {
        if self.segments.load(Ordering::SeqCst) {
            return true;
        }
        match self.can_segment().await {
            Ok(()) => {
                self.segments.store(true, Ordering::SeqCst);
                true
            }
            Err(why) => why == SEGMENT_UNASKED,
        }
    }

    /// Ready to segment, inside a job: the server waited for as a render
    /// waits for it (a restart under the job, the idle reset), then asked.
    /// `Err(None)` is a stop; `Err(Some(why))` is said, and the edit drawn
    /// without the segmentation (review of #710, pass 3).
    async fn ready_to_segment(
        &self,
        cancel: Option<&CancellationToken>,
    ) -> std::result::Result<(), Option<String>> {
        match self.wire.await_server(cancel, SERVER_WAIT).await {
            Err(Failure::Cancelled) => Err(None),
            Err(Failure::Other(_)) => Err(Some("the image server is not answering".into())),
            Ok(()) => self.can_segment().await.map_err(Some),
        }
    }

    /// Whether the server can segment: ComfyUI's `SAM3_Detect` node, and
    /// [`SEGMENT_MODEL`] among its checkpoints. A short reason when not,
    /// which the result says (`docs/SEGMENTATION-DESIGN.md` §5).
    async fn can_segment(&self) -> std::result::Result<(), String> {
        let sam = self
            .wire
            .get_json("object_info/SAM3_Detect")
            .await
            .map_err(|_| SEGMENT_UNASKED.to_string())?;
        if sam.get("SAM3_Detect").is_none() {
            return Err("the image server's ComfyUI has no SAM 3 node".into());
        }
        let ckpt = self
            .wire
            .get_json("object_info/CheckpointLoaderSimple")
            .await
            .map_err(|_| SEGMENT_UNASKED.to_string())?;
        let listed = ckpt
            .pointer("/CheckpointLoaderSimple/input/required/ckpt_name/0")
            .and_then(Value::as_array)
            .is_some_and(|c| c.iter().any(|n| n.as_str() == Some(SEGMENT_MODEL)));
        if !listed {
            return Err("SAM 3.1 segmentation is not installed on the image server".into());
        }
        Ok(())
    }

    /// SAM 3.1's mask of what `words` name in `picture`, as a white-on-black
    /// picture at the server's size: `Ok(None)` when nothing matched. Run
    /// inside an admitted picture job, before its render, never on its own
    /// (`docs/SEGMENTATION-DESIGN.md` §3: the admission gauge cannot tell
    /// which model a process holds). Its temp copies are deleted as a
    /// render's are; what could not be is the second value, said.
    async fn segment(
        &self,
        cfg: &ImageConfig,
        picture: &Reference,
        words: &str,
        cancel: Option<&CancellationToken>,
        timeout: Duration,
        trail: Option<&std::path::Path>,
    ) -> (
        std::result::Result<Option<image::GrayImage>, Failure>,
        Option<String>,
    ) {
        // The job's segmentation budget is spent: nothing is uploaded.
        if timeout.is_zero() {
            return (
                Err(Failure::Other(anyhow!(
                    "segmentation ran past its {} s",
                    SEGMENT_BUDGET.as_secs()
                ))),
                None,
            );
        }
        let mut held = Held::default();
        let got = async {
            self.wire.await_server(cancel, SERVER_WAIT).await?;
            let asked = format!(
                "mecha-{:08x}{:08x}.{}",
                fresh_seed(),
                fresh_seed(),
                picture.ext
            );
            note(trail, &TrailEntry::File(asked.clone())).map_err(|e| {
                Failure::Other(e.context("the segmentation could not be recorded for cleanup"))
            })?;
            let filed = match self.upload(picture, &asked).await {
                Ok(filed) => filed,
                Err(e) => {
                    held.possible.push(asked);
                    return Err(Failure::Other(e));
                }
            };
            held.confirmed.push(filed.clone());
            // A copy the server renamed is trailed too, as `run` does: the
            // trail is the record that outlives this process (review of #331).
            if filed != asked {
                note(trail, &TrailEntry::File(filed.clone())).map_err(|e| {
                    Failure::Other(e.context("the segmentation could not be recorded for cleanup"))
                })?;
            }
            let png = self
                .submit_and_wait(sam_graph(&filed, words), cancel, timeout, trail, &mut held)
                .await?;
            let mask = image::load_from_memory(&png)
                .map_err(|e| Failure::Other(anyhow!("the segmentation did not decode: {e}")))?
                .to_luma8();
            Ok(mask.pixels().any(|p| p.0[0] > 127).then_some(mask))
        }
        .await;
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
        (got, (!left.is_empty()).then(|| left.join("; ")))
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
        self.wire.await_server(cancel, SERVER_WAIT).await?;
        let model = cfg.model_file(req.sampling.model);
        self.preflight(cfg, &req.sampling).await?;
        // The other model out first, so the two are never resident at once.
        // Recorded before the unload, so a picture starting meanwhile sees the
        // switch too and unloads rather than loading beside it (review of
        // #648, pass 3). Best effort: ComfyUI acts on /free only once its
        // queue is empty, so pictures already queued can still overlap.
        let switching = self.switching(model, &cfg.diffusion_model);
        self.drew_with(model);
        if switching {
            let _ = self.release().await;
            let settle = tokio::time::sleep(UNLOAD_SETTLE);
            match cancel {
                Some(token) => tokio::select! {
                    _ = token.cancelled() => return Err(Failure::Cancelled),
                    _ = settle => {}
                },
                None => settle.await,
            }
        }
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
        // The mask is uploaded last, and is no reference.
        let (refs, mask) = match req.mask {
            Some(_) => (&uploaded[..uploaded.len() - 1], uploaded.last()),
            None => (&uploaded[..], None),
        };
        let graph = comfy_graph(cfg, req, refs, mask.map(String::as_str));
        self.submit_and_wait(graph, cancel, timeout, trail, held)
            .await
    }

    /// One graph through the server: submitted under an id written down
    /// first, polled until it finishes or fails, the image it made fetched
    /// and checked, and everything it wrote put on the trail and `held` for
    /// deleting. Cancel, the timeout and a server that stops answering all
    /// take the job back off the server. A picture's render and a
    /// segmentation (`segment`) both go through it.
    async fn submit_and_wait(
        &self,
        graph: Value,
        cancel: Option<&CancellationToken>,
        timeout: Duration,
        trail: Option<&std::path::Path>,
        held: &mut Held,
    ) -> std::result::Result<Vec<u8>, Failure> {
        let record = |entry: TrailEntry| {
            note(trail, &entry).map_err(|e| {
                Failure::Other(e.context("the image job could not be recorded for cleanup"))
            })
        };
        // The id is minted here and written down before the server sees the
        // job; ComfyUI takes a client's id in canonical UUID form. An older
        // server mints its own, which is written down as soon as it answers.
        let asked = uuid::Uuid::new_v4().to_string();
        record(TrailEntry::Job(asked.clone()))?;
        let submitted = self
            .wire
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
            let tick = tokio::time::sleep(self.wire.poll);
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
            let history = match self.wire.get_json(&format!("history/{id}")).await {
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
                self.wire.forget(&id).await;
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
                    self.wire.forget(&id).await;
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
        self.wire.forget(&id).await;
        Ok(fetched?)
    }

    /// The finished image's bytes, checked to be a PNG.
    async fn fetch(&self, image: &Value) -> Result<Vec<u8>> {
        let bytes = self.wire.view(image, "the finished image").await?;
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            bail!("the image server returned something that is not a PNG");
        }
        Ok(bytes)
    }

    /// Whether the server has loaded a model since it started
    /// ([`loaded_from_stats`]); `None` if it cannot say - including a server
    /// that is restarting, which is then asked for the cold cost.
    async fn loaded(&self) -> Option<bool> {
        loaded_from_stats(&self.wire.stats().await?)
    }

    /// Ask the server to drop its models and free the memory they held.
    async fn release(&self) -> bool {
        self.wire.release().await
    }

    /// An idle release: the server is marked empty only when it took the
    /// unload, so a swallowed failure never tells every process that reads
    /// the record that the next switch has nothing to unload.
    async fn release_idle(&self) {
        if self.release().await {
            self.released();
        }
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
/// at the canvas's scale — the setting measured seamless on one textured
/// picture (`IMAGE-REGION-EDIT-RESEARCH.md` §4). §7.7 later saw a faint
/// rectangular seam at this edge where it crossed a smooth background, on
/// one scene at n = 4; still open.
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

/// A painted mask at the canvas's size, hard-edged. Any real coverage
/// counts: a thin stroke on a large photo is averaged down by the resize, and
/// 127 would erase it before the owner is told it is too small (review of
/// #429).
fn hard_at(mask: &image::GrayImage, cw: u32, ch: u32) -> image::GrayImage {
    let luma = image::imageops::resize(mask, cw, ch, image::imageops::FilterType::Triangle);
    image::GrayImage::from_fn(cw, ch, |x, y| {
        image::Luma([if luma.get_pixel(x, y).0[0] > 16 {
            255
        } else {
            0
        }])
    })
}

/// The painted area grown past its edge. The grow is a blur thresholded
/// low: with σ = 12, "> 10" reaches about 21 px past the painted edge. The
/// threshold moves with the sigma; change them together. (The 16 in
/// [`hard_at`] is coverage after a resize, not this.)
fn grow_mask(hard: &image::GrayImage) -> image::GrayImage {
    let grown = image::imageops::fast_blur(hard, MASK_GROW_SIGMA);
    image::GrayImage::from_fn(hard.width(), hard.height(), |x, y| {
        image::Luma([if grown.get_pixel(x, y).0[0] > 10 {
            255
        } else {
            0
        }])
    })
}

/// The grown area feathered for the composite. Painted pixels stay fully
/// redrawn, whatever the blur did to them.
fn feather_mask(grown: &image::GrayImage, hard: &image::GrayImage) -> image::GrayImage {
    let soft = image::imageops::fast_blur(grown, MASK_FEATHER_SIGMA);
    image::GrayImage::from_fn(hard.width(), hard.height(), |x, y| {
        image::Luma([soft.get_pixel(x, y).0[0].max(hard.get_pixel(x, y).0[0])])
    })
}

/// How far a segmented face is grown before it is feathered, in canvas
/// pixels, and the feather's sigma: enough to carry the jaw's shadow and the
/// hairline's first strands, measured by eye on the 2026-10-16 composites
/// (`docs/SEGMENTATION-DESIGN.md` S2).
const FACE_EDGE_GROW: u32 = 5;
const FACE_EDGE_FEATHER: f32 = 3.0;

/// The composite's mask for an edit whose painted regions take a face
/// (`docs/SEGMENTATION-DESIGN.md` S2): inside the face regions, the paint ∩
/// the face SAM found (grown [`FACE_EDGE_GROW`], feathered), so the new face
/// is laid back along the face's own edges and the photo keeps its hairline,
/// ears and earrings; every other region keeps its own softened paint
/// (`others`, the hard paint of the regions with no face). Never past
/// `soft`, so nothing outside the paint's own edge can change. `None` when
/// the face found is not inside the face regions' paint (another face in the
/// picture, or none): the paint is kept then, and said.
pub(crate) fn edge_faces(
    soft: &image::GrayImage,
    others: Option<&image::GrayImage>,
    found: &image::GrayImage,
) -> Option<image::GrayImage> {
    let (w, h) = soft.dimensions();
    let found = image::imageops::resize(found, w, h, image::imageops::FilterType::Triangle);
    // A square grow, separable: rows, then columns.
    let r = FACE_EDGE_GROW as i64;
    let on = |img: &image::GrayImage, x: i64, y: i64| {
        x >= 0
            && y >= 0
            && x < w as i64
            && y < h as i64
            && img.get_pixel(x as u32, y as u32).0[0] > 127
    };
    let rows = image::GrayImage::from_fn(w, h, |x, y| {
        let hit = (-r..=r).any(|d| on(&found, x as i64 + d, y as i64));
        image::Luma([if hit { 255 } else { 0 }])
    });
    let grown = image::GrayImage::from_fn(w, h, |x, y| {
        let hit = (-r..=r).any(|d| on(&rows, x as i64, y as i64 + d));
        image::Luma([if hit { 255 } else { 0 }])
    });
    let face = image::imageops::fast_blur(&grown, FACE_EDGE_FEATHER);
    let kept = others.map(|hard| feather_mask(&grow_mask(hard), hard));
    let (mut painted, mut faced) = (0u64, 0u64);
    let out = image::GrayImage::from_fn(w, h, |x, y| {
        let s = soft.get_pixel(x, y).0[0];
        let k = kept.as_ref().map_or(0, |k| k.get_pixel(x, y).0[0]);
        let f = s.min(face.get_pixel(x, y).0[0]);
        if k == 0 {
            painted += u64::from(s);
            faced += u64::from(f);
        }
        image::Luma([k.max(f)])
    });
    // The face found must fill a real part of the face regions' paint: a
    // sliver at its edge is a neighbour's face, not this one.
    (painted > 0 && faced * 10 >= painted).then_some(out)
}

/// What an edit whose painted regions take a face needs, in its job, to lay
/// the new face back along the face's own edges ([`edge_faces`]).
struct FaceEdges {
    /// The hard paint of the regions with no face, which keep their own.
    others: Option<image::GrayImage>,
    /// A face region's words name the hair too: SAM's "hair" is added.
    hair: bool,
    colours: Vec<String>,
}

/// What a face region's note adds when its character's face could not be
/// had on a picture whose people are known: the name is read as the viewer
/// there, so "its words alone" are not the words as given (review of #706).
fn viewer_said(names_checked: bool, shown: &str) -> String {
    if names_checked {
        format!(", with {shown} read as the viewer, since {shown} is not in the picture")
    } else {
        String::new()
    }
}

/// Whether a painted region's words name the hair, which a face region then
/// takes with the face (`docs/SEGMENTATION-DESIGN.md` S2).
fn about_hair(words: &str) -> bool {
    words
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| matches!(w.to_lowercase().as_str(), "hair" | "hairline" | "hairstyle"))
}

/// The hard paint of `regions` in a region index, at the canvas's size, as
/// [`prepare_mask`] reads the union. `None` for no regions.
pub(crate) fn regions_hard(
    index: &[u8],
    regions: &[&crate::picture::Region],
    cw: u32,
    ch: u32,
) -> std::result::Result<Option<image::GrayImage>, String> {
    if regions.is_empty() {
        return Ok(None);
    }
    let index = crate::image::decode(index, "the regions")
        .map_err(|e| format!("{e:#}"))?
        .to_rgb8();
    let union = image::GrayImage::from_fn(index.width(), index.height(), |x, y| {
        let p = index.get_pixel(x, y).0;
        image::Luma([if regions.iter().any(|r| r.rgb == p) {
            255
        } else {
            0
        }])
    });
    Ok(Some(hard_at(&union, cw, ch)))
}

/// [`prepare_mask`]'s two refusals of a mask whose marks are too few: the
/// owner's paint is refused with them, and a mask segmentation found is
/// dropped and said instead (S1), told apart by these, not by an error's
/// other wording.
const MASK_MARKS_NOTHING: &str = "The mask marks nothing: its painted (white) area is empty. \
                                  Ask the user to paint the part to change.";
const MASK_TOO_SMALL: &str = "The painted area is too small to edit: ask the user to paint \
                              over more of what should change.";

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
    use image::imageops::{resize, FilterType};
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
    let hard = hard_at(&mask.to_luma8(), cw, ch);
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for (x, y, p) in hard.enumerate_pixels() {
        if p.0[0] > 0 {
            let b = bounds.get_or_insert((x, y, x + 1, y + 1));
            *b = (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1));
        }
    }
    let Some(bounds) = bounds else {
        return Err(MASK_MARKS_NOTHING.into());
    };
    let grown = grow_mask(&hard);
    // A dab too small to survive the grow step would leave the sampler
    // nothing to redraw and the composite nothing to change, while the result
    // said the painted area was redrawn: refused, like an empty mask (review
    // of #429).
    if !grown.pixels().any(|p| p.0[0] > 0) {
        return Err(MASK_TOO_SMALL.into());
    }
    let soft = feather_mask(&grown, &hard);
    Ok(MaskPlan {
        picture,
        source: (pw, ph),
        soft,
        bounds,
    })
}

/// A region index (IMAGE-REGION-EDIT-RESEARCH.md §7.4, M2) read into the
/// mask the composite keeps everything else by, and the picture the model
/// reads as `<image2>`: the canvas with each region outlined in its colour.
///
/// The index is read per colour, by exact pixel value, before anything
/// reaches `prepare_mask`. That gives each region its identity, which the
/// empty check and the outline both need. It also masks only the colours
/// the call names: `prepare_mask`'s luma threshold alone would mask any
/// pixel brighter than near-black, a stray or a palette colour with no
/// words among them. A region with no pixels is refused rather than sent as
/// words with no place.
pub fn prepare_regions(
    picture: &[u8],
    index: &[u8],
    regions: &[crate::picture::Region],
    resolution: u32,
) -> std::result::Result<(MaskPlan, image::RgbImage), String> {
    let index = crate::image::decode(index, "the regions")
        .map_err(|e| format!("{e:#}"))?
        .to_rgb8();
    let (w, h) = index.dimensions();
    let mut union = image::GrayImage::new(w, h);
    for r in regions {
        let mut any = false;
        for (x, y, p) in index.enumerate_pixels() {
            if p.0 == r.rgb {
                union.put_pixel(x, y, image::Luma([255]));
                any = true;
            }
        }
        if !any {
            return Err(format!(
                "The {} region is empty: no pixel of the index is in that colour.",
                r.colour
            ));
        }
    }
    let union = png_bytes(&image::DynamicImage::ImageLuma8(union).to_rgb8())?;
    let plan = prepare_mask(picture, &union, resolution)?;
    let (cw, ch) = plan.picture.dimensions();
    let mut outlined = plan.picture.clone();
    // About 6 px on a 1024 canvas, as measured.
    let radius = (cw.max(ch) / 340).max(2) as i64;
    // Each region's coverage at the canvas's size, averaged rather than
    // sampled: on a photo larger than the canvas, a stroke narrower than the
    // downscale's step fell between nearest samples and lost its outline
    // while its words still went (review of #623). Shrinking, any coverage
    // counts; growing, half does, which is where nearest put the edge.
    let floor = if w > cw || h > ch { 1 } else { 128 };
    for r in regions {
        let mut own = image::GrayImage::new(w, h);
        for (x, y, p) in index.enumerate_pixels() {
            if p.0 == r.rgb {
                own.put_pixel(x, y, image::Luma([255]));
            }
        }
        let cover = image::imageops::resize(&own, cw, ch, image::imageops::FilterType::Triangle);
        let inside = |x: i64, y: i64| {
            x >= 0
                && y >= 0
                && (x as u32) < cw
                && (y as u32) < ch
                && cover.get_pixel(x as u32, y as u32).0[0] >= floor
        };
        for y in 0..ch as i64 {
            for x in 0..cw as i64 {
                // On the region's rim: inside it, with a neighbour outside.
                let edge = inside(x, y)
                    && !(inside(x - 1, y)
                        && inside(x + 1, y)
                        && inside(x, y - 1)
                        && inside(x, y + 1));
                if !edge {
                    continue;
                }
                for oy in -radius..=radius {
                    for ox in -radius..=radius {
                        let (px, py) = (x + ox, y + oy);
                        if ox * ox + oy * oy <= radius * radius
                            && px >= 0
                            && py >= 0
                            && (px as u32) < cw
                            && (py as u32) < ch
                        {
                            outlined.put_pixel(px as u32, py as u32, image::Rgb(r.rgb));
                        }
                    }
                }
            }
        }
    }
    Ok((plan, outlined))
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
    /// The memory floor a generation is admitted against (`[system]`;
    /// `docs/SYSTEM-STATE-DESIGN.md` §3.7). R5's defaults until the caller
    /// passes the operator's (`with_floor`).
    floor: crate::system::floor::SystemConfig,
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
    /// The chat's picture settings (`sampling::chat_path`), read on every
    /// call so a change on the settings page is the next picture's: the
    /// layer a persona's own settings lie over. Unset — replays, evals,
    /// tests — there is no chat layer, so an arm measures only what it
    /// names.
    chat_settings: Option<std::path::PathBuf>,
    /// The mecha home whose LoRA registry a picture's list is resolved
    /// against (`loras::Loaded::read`), read on every call that has a list.
    /// Unset — replays, evals, tests — no list applies.
    loras_home: Option<std::path::PathBuf>,
    /// The registry as last read whole: a re-read that fails keeps it, so
    /// one transient read error never draws a picture without its LoRAs
    /// (`docs/LORA-DESIGN.md` §3).
    loras_last: Arc<std::sync::Mutex<Option<crate::loras::Loaded>>>,
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
    /// The shared memory floor (`system::floor::admit_now`) decides: the
    /// need below plus the reserve, refused under memory pressure, and
    /// refused when the gauge cannot be read. Only the operator's own
    /// `min_available_mb = 0` switches it off.
    async fn memory_guard(
        &self,
        model: crate::sampling::Model,
        may_segment: bool,
    ) -> std::result::Result<(), String> {
        if self.cfg.min_available_mb == 0 {
            return Ok(());
        }
        // A switch of model unloads the one held and loads the other: cold.
        // Not asked on a Mac: `loaded_from_stats` reads any device but CUDA
        // as unknown, so the answer is always cold there, and asking it
        // through the on-demand proxy started ComfyUI even for a picture the
        // gate then refused (review of #689).
        let loaded = if cfg!(target_os = "macos") {
            None
        } else if self
            .backend
            .switching(self.cfg.model_file(model), &self.cfg.diffusion_model)
        {
            Some(false)
        } else {
            self.backend.loaded().await
        };
        // SAM is charged where it can load. Asked on Linux only, as `loaded`
        // is: on a Mac the ask goes through the on-demand proxy and starts
        // ComfyUI, so a Mac is charged whenever the call may segment.
        let may_segment =
            may_segment && (cfg!(target_os = "macos") || self.backend.segments().await);
        let need_mb = memory_need_mb(loaded, self.cfg.min_available_mb, may_segment);
        let series = crate::system::db_path().ok();
        let verdict = crate::system::floor::admit_now(
            need_mb.saturating_mul(1024 * 1024),
            &self.floor,
            series.as_deref(),
        );
        match verdict.sentence("a generation") {
            None => Ok(()),
            Some(why) => {
                // The off switch is the operator's, said in the operator's
                // log and never to the model: told it, a model retried six
                // times in twenty seconds and then recommended the owner
                // switch the guard off (owner's Mac, 2026-10-13).
                tracing::warn!(
                    "image generation refused for memory: {why} — `[image] min_available_mb` \
                     sets the need, and 0 switches the check off"
                );
                Err(format!(
                    "{why} This will not change by retrying within this turn: say so to the \
                     owner and stop — it can be tried again later, once something large (a \
                     build, another model) has finished."
                ))
            }
        }
    }

    /// Admit generations against the operator's `[system]` floor.
    pub fn with_floor(mut self, floor: crate::system::floor::SystemConfig) -> Self {
        self.floor = floor;
        self
    }

    /// Share which model each server holds with every other process that
    /// draws, through the file at `path` (`ComfyUi::model_record`).
    pub fn with_model_record(self, path: std::path::PathBuf) -> Self {
        let _ = self.backend.model_record.set(path);
        self
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
            floor: crate::system::floor::SystemConfig::default(),
            seeds: false,
            reference_pixels: MAX_REFERENCE_PIXELS,
            faces: Arc::new(crate::face::CachedRetinaFace),
            chat_settings: None,
            loras_home: None,
            loras_last: Arc::new(std::sync::Mutex::new(None)),
        })
    }

    /// Resolve the owner's picture LoRA lists against the registry in
    /// `mecha_home` on every call.
    pub fn with_loras(mut self, mecha_home: std::path::PathBuf) -> Self {
        self.loras_home = Some(mecha_home);
        self
    }

    /// What this picture's LoRA list comes to: the turn's layer over the
    /// chat's, each name resolved against the registry as it is now, and
    /// none at all on a model file LoRAs are not measured on (`model_lora`).
    fn picture_loras(
        &self,
        ctx: &ToolCtx,
        model: crate::sampling::Model,
    ) -> (crate::loras::Picked, Option<&'static str>) {
        let chat = self
            .chat_settings
            .as_deref()
            .map(crate::sampling::chat_loras)
            .unwrap_or_default();
        let (names, damaged) = ctx.image_loras.over(&chat);
        if names.is_empty() {
            return (
                crate::loras::Picked {
                    damaged,
                    ..Default::default()
                },
                None,
            );
        }
        let Some(home) = &self.loras_home else {
            return (
                crate::loras::Picked {
                    gone: names.len(),
                    damaged,
                    ..Default::default()
                },
                None,
            );
        };
        let folders = crate::loras::Folders::of(home, Some(&self.cfg), None);
        let fresh = crate::loras::Loaded::read(home, None, &folders);
        let loaded = {
            let mut last = self.loras_last.lock().unwrap_or_else(|e| e.into_inner());
            match (&fresh.unreadable, last.as_ref()) {
                (Some(why), Some(kept)) => {
                    tracing::warn!("the LoRA registry {why}; pictures keep the last one read");
                    kept.clone()
                }
                // Nothing read before: every name would look deleted, so
                // the result says the registry did not read instead (review
                // of #751).
                (Some(_), None) => {
                    return (
                        crate::loras::Picked {
                            damaged,
                            unreadable: true,
                            ..Default::default()
                        },
                        None,
                    )
                }
                (None, _) => {
                    *last = Some(fresh.clone());
                    fresh
                }
            }
        };
        let picked = crate::loras::for_picture(&names, damaged, &loaded);
        match model_lora(self.cfg.model_file(model)) {
            Ok(()) => (picked, None),
            Err(why) if !picked.applied.is_empty() => (
                crate::loras::Picked {
                    applied: Vec::new(),
                    ..picked
                },
                Some(why),
            ),
            Err(_) => (picked, None),
        }
    }

    /// Lay the turn's settings over the chat's, read from `path` on every
    /// call (`ImageGenerate::chat_settings`).
    pub fn with_chat_settings(mut self, path: std::path::PathBuf) -> Self {
        self.chat_settings = Some(path);
        self
    }

    /// The settings this picture is drawn with: the turn's — a persona's,
    /// or none for the assistant's chat — over the chat's.
    fn tuning(&self, ctx: &ToolCtx) -> crate::sampling::Tuning {
        match &self.chat_settings {
            Some(path) => ctx.image_tuning.over(&crate::sampling::chat(path)),
            None => ctx.image_tuning.clone(),
        }
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
            // The persona form is where most pictures are made: it keeps the
            // operator's floor, never falls back to the defaults.
            floor: self.floor.clone(),
            seeds: false,
            reference_pixels: self.reference_pixels,
            faces: Arc::clone(&self.faces),
            // Read afresh whenever a persona's agent is built (serve caches
            // one per persona version and binding, not per chat).
            description: described(self.library_dir.as_deref()),
            // The layer a persona's unset settings fall to (the owner,
            // 2026-10-11): a persona form without it would draw an unset
            // field at the model's default, not the chat's.
            chat_settings: self.chat_settings.clone(),
            loras_home: self.loras_home.clone(),
            loras_last: Arc::clone(&self.loras_last),
        }
    }

    #[cfg(test)]
    fn polling_every(mut self, poll: Duration) -> Self {
        Arc::get_mut(&mut self.backend)
            .expect("unshared in tests")
            .wire
            .poll = poll;
        self
    }

    /// Build a touching scene in layers (IMAGE-DESIGN.md §15.3): the plate,
    /// one cutout per person flattened on grey, the placing pass and the
    /// finish, in one job, with the job's Stop read between steps and handed
    /// to each render. The plate and each cutout are kept beside the
    /// picture, and each pass is a line of the prompt log. `left` gathers
    /// the image server's copies no pass could remove, and stays the
    /// caller's on every exit, so a build that fails part-way still says
    /// them beside its fallback (review of #626).
    async fn build_layers(
        &self,
        ctx: &ToolCtx,
        lp: &crate::layers::Plan,
        base: &Request,
        timeout: Duration,
        stamp: &str,
        left: &mut Vec<String>,
    ) -> std::result::Result<Built, LayersFailed> {
        let mut steps: Vec<Value> = Vec::new();
        let stopped = || ctx.cancel.as_ref().is_some_and(|c| c.is_cancelled());
        let joined = |left: &[String]| (!left.is_empty()).then(|| left.join("; "));
        // `[image] timeout_secs` bounds the build's passes together, not
        // each of its n + 3: per pass, a wedged build could hold the chat's
        // one job slot for close to an hour (review of #624). Each pass gets
        // what is left, and none starts with nothing left. A build that
        // fails then draws the single pass with a timeout of its own, so a
        // call can take up to twice it, and still delivers a picture.
        let deadline = Instant::now() + timeout;
        let pass = |step: &str, mut req: Request| {
            let step = step.to_string();
            // Someone named but not in the picture reaches the image model
            // as "the viewer", on every pass as on the single one (the
            // owner's ruling, 2026-10-08; review of #626).
            req.prompt = crate::picture::as_viewer(&req.prompt, &lp.offstage, &[]);
            async move {
                let started = Instant::now();
                let remaining = deadline.saturating_duration_since(started);
                if remaining.is_zero() {
                    let out = Outcome {
                        image: Err(Failure::Other(anyhow::anyhow!(
                            "the build ran past its {} s",
                            timeout.as_secs()
                        ))),
                        left: None,
                    };
                    return (step, req, 0, out);
                }
                let out = self
                    .backend
                    .generate(
                        &self.cfg,
                        &req,
                        ctx.cancel.as_ref(),
                        remaining,
                        ctx.image_trail.as_deref(),
                    )
                    .await;
                (step, req, started.elapsed().as_secs(), out)
            }
        };
        let run = |step: String,
                   req: Request,
                   secs: u64,
                   out: Outcome,
                   left: &mut Vec<String>,
                   steps: &mut Vec<Value>| {
            left.extend(out.left);
            steps.push(json!({"step": step, "seed": req.seed, "secs": secs}));
            if let Some(log) = &ctx.prompt_log {
                let line = json!({
                    "created": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    "route": "layered",
                    "step": step,
                    "seed": req.seed,
                    // As sent: the settings' trigger words included.
                    "prompt": with_triggers(&req.prompt, &req.loras),
                    "words": lp.words,
                });
                if let Err(e) = append_prompt(log, &line) {
                    tracing::warn!("image_generate: prompt log: {e:#}");
                }
            }
            match out.image {
                Ok(bytes) => Ok(bytes),
                Err(Failure::Cancelled) => Err(LayersFailed::Cancelled(None)),
                Err(Failure::Other(e)) => Err(LayersFailed::At(step, format!("{e:#}"))),
            }
        };
        let fail = |e: LayersFailed, left: &[String]| match e {
            LayersFailed::Cancelled(_) => LayersFailed::Cancelled(joined(left)),
            other => other,
        };
        // 1. The plate: the room alone, in the scene's shape.
        let (step, req, secs, out) = pass(
            "plate",
            Request {
                prompt: lp.plate.clone(),
                sampling: base.sampling.clone(),
                size: base.size,
                seed: base.seed,
                references: Vec::new(),
                reference_size: base.reference_size,
                mask: None,
                loras: base.loras.clone(),
            },
        )
        .await;
        let mut plate = run(step, req, secs, out, left, &mut steps).map_err(|e| fail(e, left))?;
        // The room must be empty: a face in it is a stranger the placing
        // pass would keep (mecha-a3, 2026-10-10). One more draw at a new
        // seed, then the single pass. A detector that cannot run is said in
        // the manifest; it does not stop the build, since it guards the
        // picture's quality, not the owner's data.
        // The detector runs off the async workers, as the crops' does: it
        // can take seconds, and a panic there must cost the check, never
        // the job, whose slot is freed only when its future ends (review of
        // #626).
        let detector = Arc::clone(&self.faces);
        let faces = |png: &[u8]| {
            let detector = Arc::clone(&detector);
            let png = png.to_vec();
            async move {
                tokio::task::spawn_blocking(move || detector.faces_in(&png))
                    .await
                    .unwrap_or_else(|_| Err("the face detector panicked".into()))
            }
        };
        match faces(&plate).await {
            Ok(0) => steps.push(json!({"step": "plate check", "faces": 0})),
            Ok(n) => {
                steps.push(json!({"step": "plate check", "faces": n}));
                if stopped() {
                    return Err(LayersFailed::Cancelled(joined(left)));
                }
                let (step, req, secs, out) = pass(
                    "plate again",
                    Request {
                        prompt: lp.plate.clone(),
                        sampling: base.sampling.clone(),
                        size: base.size,
                        seed: draw_seed(ctx),
                        references: Vec::new(),
                        reference_size: base.reference_size,
                        mask: None,
                        loras: base.loras.clone(),
                    },
                )
                .await;
                plate = run(step, req, secs, out, left, &mut steps).map_err(|e| fail(e, left))?;
                match faces(&plate).await {
                    Ok(0) => steps.push(json!({"step": "plate check", "faces": 0})),
                    Ok(n) => {
                        return Err(LayersFailed::At(
                            "plate".into(),
                            format!("the room came back with {n} face(s) in it, twice"),
                        ))
                    }
                    Err(why) => steps.push(json!({"step": "plate check", "unchecked": why})),
                }
            }
            Err(why) => steps.push(json!({"step": "plate check", "unchecked": why})),
        }
        save_named(ctx, &format!("images/{stamp}-plate.png"), &plate)
            .await
            .map_err(|e| LayersFailed::At("plate".into(), format!("{e:#}")))?;
        // 2. A cutout per person, from their portrait, in a neutral pose,
        // flattened on grey before it is a reference.
        let mut flats: Vec<Reference> = Vec::new();
        let mut people: Vec<crate::scene::LayerPerson> = Vec::new();
        let named: Vec<String> = lp
            .people
            .iter()
            .map(|p| p.shown.clone())
            .chain(lp.offstage.iter().cloned())
            .collect();
        for (i, p) in lp.people.iter().enumerate() {
            if stopped() {
                return Err(LayersFailed::Cancelled(joined(left)));
            }
            let name = format!("cutout {} of {}", i + 1, lp.people.len());
            let (step, req, secs, out) = pass(
                &name,
                Request {
                    // A replay's posed arm puts the part into the cutout
                    // (`ToolCtx::layers_posed`); a chat never does.
                    // A solo cutout names no one: clothes said by
                    // someone's name ("John's jacket") read as the
                    // person's own, as the light names nobody (review of
                    // #626).
                    prompt: if ctx.layers_posed && !p.part.trim().is_empty() {
                        crate::layers::cutout_prompt_posed(
                            &crate::layers::unnamed(&p.wearing, &named),
                            &p.part,
                            &lp.light,
                        )
                    } else {
                        crate::layers::cutout_prompt(
                            &crate::layers::unnamed(&p.wearing, &named),
                            &lp.light,
                        )
                    },
                    sampling: base.sampling.clone(),
                    size: Some((1024, 1024)),
                    seed: draw_seed(ctx),
                    references: vec![Reference {
                        path: format!("library:{}", p.key),
                        bytes: p.portrait.clone(),
                        ext: p.ext,
                    }],
                    reference_size: EDIT_REFERENCE_SIZE,
                    mask: None,
                    loras: base.loras.clone(),
                },
            )
            .await;
            let seed = req.seed;
            let cutout = run(step, req, secs, out, left, &mut steps).map_err(|e| fail(e, left))?;
            let flat = crate::layers::flatten_on_grey(&cutout)
                .map_err(|why| LayersFailed::At(name.clone(), why))?;
            save_named(
                ctx,
                &format!("images/{stamp}-cutout-{}.png", p.key),
                &cutout,
            )
            .await
            .map_err(|e| LayersFailed::At(name.clone(), format!("{e:#}")))?;
            people.push(crate::scene::LayerPerson {
                who: p.key.clone(),
                cutout: crate::scene::hash(&cutout),
                part: p.part.clone(),
                seed,
            });
            flats.push(Reference {
                path: format!("layers:{}", p.key),
                bytes: flat,
                ext: "png",
            });
        }
        if stopped() {
            return Err(LayersFailed::Cancelled(joined(left)));
        }
        // 3. The placing pass: the plate as the canvas, each person by tag.
        let mut references = vec![Reference {
            path: "layers:plate".into(),
            bytes: plate.clone(),
            ext: "png",
        }];
        references.extend(flats);
        let (step, req, secs, out) = pass(
            "placing",
            Request {
                prompt: crate::layers::placing_prompt(&lp.people, lp.leftover.as_deref()),
                sampling: base.sampling.clone(),
                size: None,
                seed: draw_seed(ctx),
                references,
                reference_size: EDIT_REFERENCE_SIZE,
                mask: None,
                loras: base.loras.clone(),
            },
        )
        .await;
        let placing_seed = req.seed;
        let placed = run(step, req, secs, out, left, &mut steps).map_err(|e| fail(e, left))?;
        if stopped() {
            return Err(LayersFailed::Cancelled(joined(left)));
        }
        // 4. The finish: light, depth of field and detail, nothing else.
        let (step, req, secs, out) = pass(
            "finish",
            Request {
                prompt: crate::layers::finish_prompt(lp.style.as_deref()),
                sampling: base.sampling.clone(),
                size: None,
                seed: draw_seed(ctx),
                references: vec![Reference {
                    path: "layers:placed".into(),
                    bytes: placed,
                    ext: "png",
                }],
                reference_size: EDIT_REFERENCE_SIZE,
                mask: None,
                loras: base.loras.clone(),
            },
        )
        .await;
        let finish_seed = req.seed;
        let bytes = run(step, req, secs, out, left, &mut steps).map_err(|e| fail(e, left))?;
        Ok(Built {
            bytes,
            record: crate::scene::Layers {
                plate: crate::scene::hash(&plate),
                people,
                placing_seed,
                finish_seed,
                origin: lp.origin,
            },
            steps,
            left: joined(left),
        })
    }

    /// `picture` drawn again as itself at about a megapixel
    /// ([`UPSCALE_PIXELS`]): `mecha imagelib upscale`. A one-reference edit
    /// on the standard model, the canvas the encoder's own at
    /// [`EDIT_REFERENCE_SIZE`], so the picture keeps its shape. Admitted
    /// against memory as any picture is; the caller judges the face.
    pub async fn upscale(&self, picture: &[u8], seed: u64) -> std::result::Result<Vec<u8>, String> {
        let ext = sniff_image(picture).ok_or("not a PNG, JPEG or WebP image")?;
        let img =
            crate::image::decode_upright(picture, "the picture").map_err(|e| format!("{e:#}"))?;
        let (w, h) = (img.width(), img.height());
        // Nine tenths, so a 1024² picture the encoder rounded is not redrawn.
        if u64::from(w) * u64::from(h) >= u64::from(UPSCALE_PIXELS) * 9 / 10 {
            return Err(format!(
                "it is already {w}×{h}; a redraw at about a megapixel adds nothing"
            ));
        }
        self.memory_guard(crate::sampling::Model::Standard, false)
            .await?;
        let req = Request {
            prompt: UPSCALE_PROMPT.to_string(),
            size: None,
            sampling: crate::sampling::Sampling::reference(self.cfg.steps),
            seed,
            references: vec![Reference {
                path: "portrait".into(),
                bytes: picture.to_vec(),
                ext,
            }],
            reference_size: EDIT_REFERENCE_SIZE,
            mask: None,
            // A redraw for a clip's reference: the owner's picture LoRAs
            // are for the pictures a chat draws, not for this.
            loras: Vec::new(),
        };
        let timeout = Duration::from_secs(self.cfg.timeout_secs);
        let out = self
            .backend
            .generate(&self.cfg, &req, None, timeout, None)
            .await;
        self.arm_unload();
        if let Some(left) = out.left {
            tracing::warn!("image server temp copies not removed: {left}");
        }
        match out.image {
            Ok(bytes) => Ok(bytes),
            Err(Failure::Cancelled) => Err("cancelled".into()),
            Err(Failure::Other(e)) => Err(format!("{e:#}")),
        }
    }

    fn arm_unload(&self) {
        let after = self.cfg.unload_after_secs;
        if after == 0 {
            return;
        }
        // No `/free` behind a Mac's proxy, and no claim that the server was
        // released either: the proxy stops it on its own window, which an open
        // ComfyUI tab can hold off, so "holds nothing" would be asserted
        // unconfirmed (`release_idle`'s rule, #648). The cost is one needless
        // `/free` before a picture with the other model (review of #689).
        if behind_mecha_s_proxy(&self.cfg.url) {
            return;
        }
        let armed = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = Arc::clone(&self.generation);
        let backend = Arc::clone(&self.backend);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(after)).await;
            if generation.load(Ordering::SeqCst) == armed {
                backend.release_idle().await;
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
     with each other) and `text` (words to render exactly), all from the user's request: a \
     new picture keeps nothing from earlier ones, so never bring back an earlier picture's \
     place, clothes or people the request does not ask for. Each person has `who` (a library \
     character's name, \"self\" for you, or a description of someone not in the library), \
     `where` (left, centre, right or background), `wearing`, `doing` and `expression`. Only \
     when the user asks to continue, build on, edit or change a picture, name it in `picture` \
     and send only what changes in `scene`: a new pose \
     or camera redraws the scene, new clothes, an expression or someone added edit the \
     picture, and restating what is already true changes nothing (a call that changes \
     nothing draws the scene again). Take someone out with `remove`. One small change to the \
     picture itself (an object, a colour, a detail) goes in `retouch`, in words: never \
     clothes, a pose, an expression or a person, which are `scene.people`. To change only \
     one thing in the picture, name it in `mask` as what it is, in a few words (never a \
     name or a position); with a painted area, `mask` is its path, never one made up. When \
     the user attaches a photo of a place and wants someone in it, that photo is the \
     setting: `scene.setting` is \
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
     picture shows, and never judge whether one came out right, or draw it again, on your own: \
     the owner sees it and says what to change. A change always makes a new file and leaves the original as it was: never \
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
     an earlier result (images/...), only when the user asks to continue, build on, edit or \
     change it. Leave it out for a new picture, which keeps nothing from earlier ones.";

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
    // A pose is read only for a call that gave none and leaves no `together`
    // (mecha-a3, 2026-10-09): asked for a part per person on a one-way act,
    // the reader mirrored the verb onto the receiver 6 of 10 times, where the
    // splitter, given the `together`, inverted 0 of 36.
    let gave_a_pose = change.people.iter().any(|p| p.doing.is_some());
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
        if p.doing.is_none() && !gave_a_pose && change.together.is_none() {
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

/// How long a persona's picture waits for the scene reader before it is drawn
/// as the call said it. Before the split, so a turn can spend both.
const SCENE_READ_TIMEOUT: Duration = Duration::from_secs(20);

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

/// The words that say what someone has on. Never a word that names an
/// occasion or a task with clothes in it (a gala, a suitcase being packed),
/// which beside the outfit given is one outfit, not two (review of #647,
/// passes 3 and 4). Matched as whole words, so a clothesline is not one.
const WEAR_WORDS: &[&str] = &["wearing", "wears", "wore"];

/// The words that say someone has nothing on: always a clothing state,
/// whatever else the clause names.
const UNDRESS_WORDS: &[&str] = &[
    "undressed",
    "naked",
    "nude",
    "topless",
    "shirtless",
    "bottomless",
    "unclothed",
];

/// Things worn that are not an outfit: a clause that names only these
/// ("wearing a bike helmet") leaves the person's clothes to the prompt,
/// where dropping them left nothing saying the person was dressed at all
/// (review of #647, pass 8).
const ACCESSORIES: &[&str] = &[
    "headphones",
    "earbuds",
    "earphones",
    "headset",
    "glasses",
    "sunglasses",
    "spectacles",
    "goggles",
    "monocle",
    "hat",
    "hats",
    "cap",
    "caps",
    "beanie",
    "helmet",
    "crown",
    "crowns",
    "tiara",
    "headband",
    "wig",
    "backpack",
    "bag",
    "handbag",
    "purse",
    "scarf",
    "scarves",
    "gloves",
    "mittens",
    "watch",
    "necklace",
    "bracelet",
    "earrings",
    "ring",
    "rings",
    "jewelry",
    "jewellery",
    "mask",
    "badge",
    "lanyard",
    "belt",
    "tie",
    "bowtie",
    "boots",
    "shoes",
    "sneakers",
    "sandals",
    "heels",
    "socks",
    "slippers",
    "ribbon",
    "bow",
    "brooch",
    "pin",
    "makeup",
    "lipstick",
    "perfume",
    "flower",
    "flowers",
];

/// What else a person is said to be "wearing" that is no outfit: an
/// expression, or their hair (review of #647, pass 9).
const NOT_OUTFIT: &[&str] = &[
    "smile",
    "smiles",
    "grin",
    "grins",
    "smirk",
    "frown",
    "scowl",
    "pout",
    "expression",
    "look",
    "hair",
    "braid",
    "braids",
    "ponytail",
    "bun",
    "curls",
];

/// Garments, which make a clause an outfit even beside an accessory
/// ("wearing a parka and a beanie").
const GARMENTS: &[&str] = &[
    "dress",
    "dresses",
    "gown",
    "shirt",
    "tshirt",
    "blouse",
    "top",
    "tank",
    "sweater",
    "jumper",
    "hoodie",
    "cardigan",
    "sweatshirt",
    "jacket",
    "coat",
    "parka",
    "raincoat",
    "overcoat",
    "blazer",
    "vest",
    "suit",
    "tuxedo",
    "uniform",
    "costume",
    "robe",
    "bathrobe",
    "kimono",
    "towel",
    "pajamas",
    "pyjamas",
    "nightgown",
    "nightie",
    "onesie",
    "jeans",
    "pants",
    "trousers",
    "shorts",
    "skirt",
    "leggings",
    "tights",
    "overalls",
    "jumpsuit",
    "sweatpants",
    "joggers",
    "bra",
    "underwear",
    "panties",
    "boxers",
    "briefs",
    "lingerie",
    "corset",
    "camisole",
    "slip",
    "swimsuit",
    "bikini",
    "trunks",
    "leotard",
    "apron",
    "sari",
    "poncho",
    "cloak",
    "cape",
    "polo",
    "clothes",
    "clothing",
    "outfit",
];

/// Whether `text` states a clothing state. An undress word always does. A
/// clause with a wear word ([`WEAR_WORDS`], or "dressed in" / "clothed in")
/// does unless everything it names that is worn is an accessory, an
/// expression or hair, with no garment and no "only" or "nothing" to make
/// that the whole outfit. An item none of the lists knows counts as
/// clothing: that keeps an outfit in a state ("a wetsuit peeled to the
/// waist"), the shape mecha-a3 measured, and a garment missing from the list
/// errs toward leaving the record's outfit out.
fn states_clothing(text: &str) -> bool {
    text.to_lowercase()
        .split([',', ';', '.', ':'])
        .any(|clause| {
            let words: Vec<&str> = clause
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect();
            if words.iter().any(|w| UNDRESS_WORDS.contains(w)) {
                return true;
            }
            let wears = words.iter().any(|w| WEAR_WORDS.contains(w))
                || words
                    .windows(2)
                    .any(|w| matches!(w[0], "dressed" | "clothed") && w[1] == "in");
            let no_outfit = words
                .iter()
                .any(|w| ACCESSORIES.contains(w) || NOT_OUTFIT.contains(w))
                && !words.iter().any(|w| GARMENTS.contains(w))
                && !words.iter().any(|w| matches!(*w, "only" | "nothing"));
            wears && !no_outfit
        })
}

/// Whether a person's act already says what they have on. That is their own
/// words (`doing`, a split part included, never their expression), or a
/// shared text when it names them or names nobody in the picture: the
/// `together`, and a retouch pasted into the same prompt. Then the prompt
/// states no outfit beside it.
///
/// Whatever filled `wearing` (the call, the reader, the record, or a
/// newcomer's default), a second outfit beside "wearing a wetsuit peeled to the waist"
/// is a dressed and an undressed person in one sentence, which the image
/// model drew as two (mecha-a3, 2026-10-11, on a94a0a076). The record keeps
/// the clothes; only the prompt leaves them out.
///
/// Except an outfit the call itself gave this turn (`asked`): the call's
/// own words are kept, so only a field refilled by the reader, the record
/// or a default yields to the act (review of #647, pass 3). A shared act
/// naming two people holds back both: which person a clause dresses is not
/// attempted, and the lost outfit is the cheaper mistake than two figures.
fn clothes_in_act(
    p: &crate::scene::Person,
    own: &str,
    shared: &[Option<&str>],
    everyone: &[&crate::scene::Person],
    asked: &std::collections::BTreeSet<String>,
) -> bool {
    if asked.contains(&p.who.key()) {
        return false;
    }
    let named = |t: &str, q: &crate::scene::Person| {
        [q.who.key(), crate::picture::shown(&q.who)]
            .iter()
            .any(|n| !crate::layers::word_spans(t, n).is_empty())
    };
    states_clothing(own)
        || shared
            .iter()
            .flatten()
            .any(|t| states_clothing(t) && (named(t, p) || everyone.iter().all(|q| !named(t, q))))
}

/// A person as an edit prompt states them: name, library description,
/// clothes unless the act already gives them (`clothes_in_act`), what they
/// do and show.
fn edit_person(
    name: &str,
    text: &str,
    p: &crate::scene::Person,
    together: bool,
    in_act: bool,
) -> String {
    let mut out = format!(" {name}");
    if !text.is_empty() {
        out.push_str(&format!(" ({text})"));
    }
    if !p.wearing.trim().is_empty() && !in_act {
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

    /// Its calls answer `being made: <path>` and draw as the chat's job
    /// (BACKGROUND-JOBS-DESIGN.md §2.1).
    fn defers(&self) -> bool {
        true
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
                "mask": {"type": "string", "description": "Beside a retouch: the one thing in the picture to change, named as what it is in a few words ('the green jacket', 'the sky'), never by a person's name or a position in the picture; or the path of the painted area the user's message names. Never make up a path."},
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
                // Regions mark places in the picture named, and their legend
                // is already the retouch: drawn as a new picture they would
                // describe outlines that are not there (review of #623).
                if !call.regions.is_empty() {
                    return Ok(refused(format!(
                        "There is no picture `{p}` in this chat, and the painted regions mark \
                         places in it. Name the picture as a result gave \
                         it (images/…) or as the owner attached it (inbox/…)."
                    )));
                }
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
        // A region's words, not its colour: a character keyed `red` would
        // otherwise refuse every edit painted red (review of #623).
        if let Some(list) = input["regions"].as_array() {
            for r in list {
                collect_strings(&r["words"], &mut prose);
            }
        }
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
        // Read once, so the guard and the request agree on the model.
        let tuning = self.tuning(ctx);
        // The owner's LoRAs for it, from the same read of the settings.
        let (picked, lora_off) = self.picture_loras(ctx, tuning.model.unwrap_or_default());
        let may_segment = call.may_segment();
        if let Err(why) = self
            .memory_guard(tuning.model.unwrap_or_default(), may_segment)
            .await
        {
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
        // What the call may carry from before: the record of the picture it
        // builds on, and for a new picture nothing (the owner, 2026-10-27). A
        // picture builds on another only when its call names it, which the
        // owner asks for; a new one is drawn from its own words, never the
        // chat's last clothes, place or people. The chat's copy stands in only
        // for the picture it records, when that one's index entry went
        // missing: never for another, so an upload named in `picture` (never
        // rendered, so never indexed) borrows nothing (review of #753, pass 2).
        // Belt and braces: `land` writes the index entry before the copy, so
        // a copy naming a picture implies its entry, and `base` is already
        // that record; this arm covers an entry lost from the index by hand.
        let carried = match &picture {
            Some(r) => base.clone().or_else(|| {
                let named = crate::scene::hash(&r.bytes);
                ctx.scene
                    .as_ref()
                    .and_then(|s| s.current())
                    .filter(|s| s.picture.as_deref() == Some(named.as_str()))
            }),
            None => None,
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
        // Named characters on a new picture that lists nobody are drawn
        // (`picture::draw_named`), and added here, ahead of the reader, so
        // the owner's words for them are merged too (review of #613).
        if call.picture.is_none() {
            if let Some(note) = crate::picture::draw_named(
                &mut call.change,
                call.retouch.as_deref(),
                &approved,
                &|t: &str| crate::imagelib::named_in(&lib, t),
            ) {
                call.notes.push(note);
            }
        }
        // Whose outfit the call itself gave this turn, before the reader
        // fills any: those are never left out of the prompt
        // (`clothes_in_act`).
        let asked_wearing: std::collections::BTreeSet<String> = call
            .change
            .people
            .iter()
            .filter(|c| c.wearing.as_deref().is_some_and(|w| !w.trim().is_empty()))
            .map(|c| c.who.key())
            .collect();
        let reader_said = match (&ctx.scene_reader, call.change.people.is_empty()) {
            (Some(reader), false) => {
                let record = carried.clone();
                let read = tokio::time::timeout(SCENE_READ_TIMEOUT, reader.read(record.as_ref()))
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
            // A scene call where no reader was stamped says so, so a merge
            // that was due and did not happen is never a silent null
            // (review of #610).
            (None, false) => Some("not read: no reader here".into()),
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
        // left out come from it (mecha-a3's G1b), on a picture that builds
        // on another only. The chat's copy, not `carried`: a newcomer is by
        // definition not in the picture built on, so only the chat's last
        // picture can have drawn them (review of #753, pass 3). A new
        // picture asks nothing of it.
        let chat_last = picture
            .is_some()
            .then(|| ctx.scene.as_ref().and_then(|s| s.current()))
            .flatten();
        let worn = |key: &str| {
            chat_last.as_ref().and_then(|s| {
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
            size: None,
            sampling: tuning.resolve(self.cfg.steps),
            seed: match plan.seed {
                crate::picture::Seed::Fresh => draw_seed(ctx),
                crate::picture::Seed::Base(s) | crate::picture::Seed::Given(s) => s,
            },
            references: Vec::new(),
            reference_size: EDIT_REFERENCE_SIZE,
            mask: None,
            loras: picked.applied.clone(),
        };
        // A redraw is drawn at a seed the picture was not (§5.1).
        if plan.route == "redrawn" {
            while Some(req.seed) == base.as_ref().and_then(|b| b.seed) {
                req.seed = draw_seed(ctx);
            }
        }
        let mut used: Vec<crate::imagelib::Used> = Vec::new();
        let mut mask_plan: Option<MaskPlan> = None;
        // The painted regions' index, kept for the face regions' edges.
        let mut region_index: Option<Vec<u8>> = None;
        let mut face_edges: Option<FaceEdges> = None;
        let mut crops_said: Vec<String> = Vec::new();
        // Whether the people's parts were split (`roles`), for the manifest:
        // the split is prompt-only and its fallback silent, so this is the
        // one place a picture says what happened (mecha-a3's G5). The
        // prompt itself stays out of the manifest (ARCHITECTURE §images).
        let mut roles_said: Option<String> = None;
        // Whose outfit the prompt left out because their act already says
        // what they have on (`clothes_in_act`), by the name the result uses,
        // and whether they are a library character. Library names go in the
        // manifest, so a person drawn without the record's clothes is never a
        // silent null; a described person's name is their description, so
        // it only silences their clothes note (review of #647, pass 5).
        let mut clothes_said: Vec<(String, bool)> = Vec::new();
        // The words a split put in the prompt, for the owner's prompt log
        // (`words`), which the privacy guard reads.
        let mut split_words: Vec<String> = Vec::new();
        // A touching scene the persona's switch builds in layers (§15).
        let mut layers_plan: Option<crate::layers::Plan> = None;
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
                        // A split for a picture that may be built in
                        // layers is asked to name whom each part acts on
                        // (`roles::NAME_WHOM`); one for a single pass is
                        // asked as before, since there it emptied parts and
                        // kept no more acts (mecha-a3, 2026-10-10). These
                        // are the layering checks known before the split;
                        // the rest (portraits, places, the setting's words)
                        // follow it, and a picture they send to one pass
                        // keeps the named split.
                        let may_layer = ctx.layers
                            && plan.route == "new"
                            && as_called
                            && people.len() == crate::layers::MEASURED_PEOPLE
                            && people
                                .iter()
                                .all(|p| matches!(p.who, crate::scene::Who::Library(_)))
                            && matches!(
                                plan.next.setting.as_ref().map(|f| &f.value),
                                Some(crate::scene::Setting::Words { .. })
                            )
                            && plan.also.is_none()
                            && plan.next.text.as_ref().is_none_or(|t| t.value.is_empty());
                        // ~1-3 s on the router, 9 s once beside two live
                        // turns (mecha-a3): bounded, and a late split draws
                        // the call as sent.
                        let answer = tokio::time::timeout(
                            ROLE_SPLIT_TIMEOUT,
                            splitter.split(&asked, &sentence.value, may_layer),
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
                                split_words.extend(split.roles.iter().map(|r| r.doing.clone()));
                                split_words.push(split.together.clone());
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
                // A picture layers may draw is compiled from portraits alone,
                // so a layered build's fallback single pass draws what its
                // cutouts would have (§9.4). Should layers then decline, the
                // single pass is the picture, and it is compiled again with
                // the bodies (review of #685).
                let layers_wanted =
                    ctx.layers && plan.route == "new" && people.len() >= 2 && as_called;
                let compile_from = |people: &[crate::scene::Person],
                                    words: &crate::scene::Scene,
                                    together: bool,
                                    bodies: bool| {
                    let mut cast = Vec::new();
                    let mut extras = Vec::new();
                    let mut in_act = Vec::new();
                    let everyone: Vec<&crate::scene::Person> = people.iter().collect();
                    let shared = words.together.as_ref().map(|f| f.value.as_str());
                    for p in people {
                        let doing = doing_words(p, together);
                        let kept = clothes_in_act(
                            p,
                            &p.doing,
                            &[shared, plan.also.as_deref()],
                            &everyone,
                            &asked_wearing,
                        );
                        if kept {
                            in_act.push((
                                crate::picture::shown(&p.who),
                                matches!(p.who, crate::scene::Who::Library(_)),
                            ));
                        }
                        match &p.who {
                            crate::scene::Who::Library(n) => {
                                cast.push(crate::imagelib::CastMember {
                                    name: n.clone(),
                                    wearing: p.wearing.clone(),
                                    doing,
                                    clothes_in_act: kept,
                                })
                            }
                            crate::scene::Who::Described(d) => {
                                let mut e = d.trim().trim_end_matches('.').to_string();
                                if !p.wearing.trim().is_empty() && !kept {
                                    e.push_str(&format!(", wearing {}", p.wearing.trim()));
                                }
                                if !doing.is_empty() {
                                    e.push_str(&format!(", {doing}"));
                                }
                                extras.push(e);
                            }
                        }
                    }
                    crate::imagelib::compile_with(
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
                        bodies,
                    )
                    .map(|c| (c, in_act))
                };
                // A split is prompt-only and must never turn a drawable call
                // into a refusal: parts the compiler will not take (a
                // described person's part naming a library character, say)
                // fall back to the call as sent (review of #609).
                let mut from_split = true;
                let (compiled, in_act) =
                    match compile_from(&people, &words_scene, together, !layers_wanted) {
                        Ok(c) => c,
                        Err(_) if roles_said.as_deref() == Some("applied") => {
                            roles_said =
                                Some("fell back: the split's parts did not compile".into());
                            from_split = false;
                            match compile_from(&plan.people, &plan.next, as_called, !layers_wanted)
                            {
                                Ok(c) => c,
                                Err(why) => return Ok(refused(why)),
                            }
                        }
                        Err(why) => return Ok(refused(why)),
                    };
                req.prompt = compiled.prompt;
                clothes_said = in_act;
                // Said once layers have decided, since a decline recompiles.
                let mut body_notes = compiled.notes;
                let library_refs = |refs: Vec<(String, Vec<u8>, &'static str)>| {
                    refs.into_iter()
                        .map(|(name, bytes, ext)| Reference {
                            path: format!("library:{name}"),
                            bytes,
                            ext,
                        })
                        .collect::<Vec<_>>()
                };
                req.references = library_refs(compiled.references);
                req.reference_size = crate::imagelib::REFERENCE_SIZE;
                used = compiled.used;
                // Never the seed that drew a portrait: sampling there redraws it.
                let asked = req.seed;
                while compiled.source_seeds.contains(&req.seed) {
                    req.seed = draw_seed(ctx);
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
                // Layers (IMAGE-DESIGN.md §15): the owner's switch, a new
                // picture, two or more people sharing an act. Decided here,
                // before anything is drawn, so the layers are kept.
                if layers_wanted {
                    let portraits: Vec<Option<(Vec<u8>, &'static str)>> = people
                        .iter()
                        .map(|p| match &p.who {
                            crate::scene::Who::Library(n) => req
                                .references
                                .iter()
                                .find(|r| r.path == format!("library:{n}"))
                                .map(|r| (r.bytes.clone(), r.ext)),
                            crate::scene::Who::Described(_) => None,
                        })
                        .collect();
                    let library = people
                        .iter()
                        .filter(|p| matches!(p.who, crate::scene::Who::Library(_)))
                        .count();
                    let setting = match plan.next.setting.as_ref().map(|f| &f.value) {
                        Some(crate::scene::Setting::Words { text }) => Some(text.clone()),
                        _ => None,
                    };
                    // A split that applied places everyone; one that fell
                    // back leaves only the places the call gave.
                    let placed = roles_said.as_deref() == Some("applied")
                        || people.iter().all(|p| p.at.is_some());
                    match crate::layers::not_layered(
                        people.len(),
                        library,
                        portraits.iter().flatten().count(),
                        setting.is_some(),
                        placed,
                        people.len() < EDIT_REFERENCE_BUDGET,
                    ) {
                        Some(why) => {
                            dropped.push(format!("Drawn in one pass, not in layers: {why}."))
                        }
                        None => {
                            let value = |f: &Option<crate::scene::Field<String>>| {
                                f.as_ref()
                                    .map(|f| f.value.trim().to_string())
                                    .filter(|v| !v.is_empty())
                            };
                            let applied = roles_said.as_deref() == Some("applied");
                            // The style's own words, as the compile pasted
                            // them for a single pass (mecha-a3, 2026-10-10:
                            // 11 of 15 real touching calls named one).
                            let style_words = style.as_deref().and_then(|name| {
                                lib.get(crate::imagelib::Kind::Style, &name.trim().to_lowercase())
                                    .filter(|e| e.status == crate::imagelib::Status::Approved)
                                    .map(|e| e.text.clone())
                            });
                            // The plate never sees a person (mecha-a3: a
                            // camera dwelling on someone drew a stranger
                            // into the empty room): framing words from a
                            // closed set, and light that names nobody.
                            // The plate names no one at all: not the
                            // people, and not anyone named offstage.
                            let names: Vec<String> = people
                                .iter()
                                .map(|p| crate::picture::shown(&p.who))
                                .chain(plan.offstage.iter().cloned())
                                .collect();
                            let room = setting
                                .as_deref()
                                .and_then(|s| crate::layers::room_words(s, &names));
                            let plate_light = value(&plan.next.light)
                                .filter(|l| crate::layers::names_no_one(l, &names));
                            let plate_framing = value(&plan.next.camera)
                                .as_deref()
                                .and_then(crate::layers::framing);
                            // Each person's clothes, the fuller of the
                            // call's and the record it builds on (mecha-a3:
                            // the shorter lost half an outfit).
                            let recorded = carried.clone();
                            let fuller = |p: &crate::scene::Person| {
                                // The call's own clothes, when it names any:
                                // a fuller record's outfit replaced the one
                                // asked for (mecha-a3, 2026-10-10).
                                if let Some(asked) = call
                                    .change
                                    .people
                                    .iter()
                                    .find(|c| c.who.key() == p.who.key())
                                    .and_then(|c| c.wearing.clone())
                                    .filter(|w| !w.trim().is_empty())
                                {
                                    return asked;
                                }
                                let kept = recorded
                                    .as_ref()
                                    .and_then(|r| {
                                        r.people.iter().find(|q| q.who.key() == p.who.key())
                                    })
                                    .map(|q| q.wearing.clone())
                                    .unwrap_or_default();
                                if kept.trim().chars().count() > p.wearing.trim().chars().count() {
                                    kept
                                } else {
                                    p.wearing.clone()
                                }
                            };
                            // The chat's words every pass is built from,
                            // for the prompt log, as the single pass keeps
                            // them, with the clothes each cutout wears.
                            let mut words = record_prose(&plan.next);
                            words.extend(plan.also.iter().cloned());
                            words.extend(split_words.iter().cloned());
                            words.extend(people.iter().map(fuller));
                            words.retain(|w| !w.trim().is_empty());
                            layers_plan = Some(crate::layers::Plan {
                                plate: crate::layers::plate_prompt(
                                    room.as_deref().unwrap_or_default(),
                                    plate_light.as_deref(),
                                    plate_framing.as_deref(),
                                ),
                                style: style_words,
                                // The cutout is lit by light that names
                                // nobody, as the plate is: a solo cutout
                                // naming someone absent invites them in.
                                light: plate_light
                                    .clone()
                                    .unwrap_or_else(|| "soft, even light".into()),
                                people: people
                                    .iter()
                                    .zip(portraits)
                                    .filter_map(|(p, portrait)| {
                                        let (bytes, ext) = portrait?;
                                        Some(crate::layers::Person {
                                            key: p.who.key(),
                                            shown: crate::picture::shown(&p.who),
                                            wearing: fuller(p),
                                            at: p.at.map(|w| w.name().to_string()),
                                            part: p.doing.clone(),
                                            expression: p.expression.clone(),
                                            portrait: bytes,
                                            ext,
                                        })
                                    })
                                    .collect(),
                                // The split's leftover; the call's own act
                                // when the split fell back (§15.3).
                                leftover: if applied {
                                    value(&words_scene.together)
                                } else {
                                    value(&plan.next.together)
                                },
                                // The build draws on the whole scene: the
                                // plate on setting, light and camera, each
                                // cutout on clothes, the parts on the
                                // `together` and the people's own words. So
                                // the record takes the scene's own origin,
                                // which names every field and so makes the
                                // compiler ask about any field added later
                                // (review of #624).
                                origin: plan.next.origin(),
                                words,
                                offstage: plan.offstage.clone(),
                            });
                            // Words the picture renders, or a retouch beside
                            // the scene: the single pass draws them, and no
                            // layered pass has been measured with them, so
                            // the picture is one render rather than one that
                            // quietly leaves them out (review of #626).
                            let renders_words =
                                plan.next.text.as_ref().is_some_and(|t| !t.value.is_empty());
                            if renders_words || plan.also.is_some() {
                                layers_plan = None;
                                dropped.push(
                                    if renders_words {
                                        "Drawn in one pass, not in layers: the picture has words \
                                         to render, which only the single pass draws."
                                    } else {
                                        "Drawn in one pass, not in layers: a retouch beside the \
                                         scene is drawn only by the single pass."
                                    }
                                    .into(),
                                );
                            // Every clause of the setting names someone, so
                            // the plate would have no room in words.
                            } else if room.is_none() {
                                layers_plan = None;
                                dropped.push(
                                    "Drawn in one pass, not in layers: every word of the \
                                     setting names someone, and the room is drawn with no one \
                                     in it."
                                        .into(),
                                );
                            // Two people set on one side of the frame would
                            // share a place tag (review of #624).
                            } else if layers_plan
                                .as_ref()
                                .is_some_and(|lp| crate::layers::shares_a_place(&lp.people))
                            {
                                layers_plan = None;
                                dropped.push(
                                    "Drawn in one pass, not in layers: two people would share \
                                     one place."
                                        .into(),
                                );
                            }
                        }
                    }
                }
                // Layers considered and declined: the single pass is the
                // picture, so it is drawn with the bodies after all, on the
                // compile route that succeeded (review of #685).
                if layers_wanted && layers_plan.is_none() {
                    let again = if from_split {
                        compile_from(&people, &words_scene, together, true)
                    } else {
                        compile_from(&plan.people, &plan.next, as_called, true)
                    };
                    match again {
                        Ok((c, _)) => {
                            req.prompt = c.prompt;
                            req.references = library_refs(c.references);
                            used = c.used;
                            body_notes = c.notes;
                        }
                        // The faces-only compile stands. `compile_with`
                        // keeps the prompt cap to itself, so a failure here
                        // is one the bodies did not cause: said as it is,
                        // since the layers note would no longer be true.
                        Err(why) => {
                            tracing::warn!("image_generate: recompile with bodies: {why}");
                            body_notes =
                                vec![format!("Full-figure references were not sent: {why}")]
                        }
                    }
                }
                dropped.extend(body_notes);
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
                // Regions carry no words of the owner's own beside them, so
                // there is no whole-picture edit to fall back to: a regions
                // edit whose index is gone draws nothing and says so, never
                // a redraw of the picture the owner painted (review of #623).
                if mask_read.is_none() && !call.regions.is_empty() {
                    return Ok(refused(
                        "The painted regions could not be read, so nothing was drawn. Ask the \
                         owner to paint them again.",
                    ));
                }
                if let Some((raw, mask)) = mask_read {
                    if !call.regions.is_empty() {
                        region_index = Some(mask.bytes.clone());
                    }
                    let pic = req.references[0].bytes.clone();
                    let resolution = req.reference_size;
                    let regions = call.regions.clone();
                    let prepared = tokio::task::spawn_blocking(move || {
                        // With regions the mask is their index, and their
                        // outlines ride as `<image2>` (M2).
                        let (plan, outlined) = if regions.is_empty() {
                            (prepare_mask(&pic, &mask.bytes, resolution)?, None)
                        } else {
                            let (plan, outlined) =
                                prepare_regions(&pic, &mask.bytes, &regions, resolution)?;
                            (plan, Some(png_bytes(&outlined)?))
                        };
                        let picture = png_bytes(&plan.picture)?;
                        let soft = png_bytes(
                            &image::DynamicImage::ImageLuma8(plan.soft.clone()).to_rgb8(),
                        )?;
                        Ok::<_, String>((plan, picture, soft, outlined))
                    })
                    .await;
                    let (mp, pic, soft, outlined) = match prepared {
                        Ok(Ok(p)) => p,
                        Ok(Err(why)) => return Ok(refused(why)),
                        Err(e) => {
                            return Ok(refused(format!("The mask could not be prepared: {e}")))
                        }
                    };
                    req.references[0].bytes = pic;
                    req.references[0].ext = "png";
                    if let Some(bytes) = outlined {
                        req.references.insert(
                            1,
                            Reference {
                                path: "the regions, outlined".into(),
                                bytes,
                                ext: "png",
                            },
                        );
                    }
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
                let everyone: Vec<&crate::scene::Person> = plan.people.iter().collect();
                // The `together` reaches an edit prompt only on a placement
                // or when the call changed it; otherwise it is the record's,
                // not in the prompt, and says nothing the model reads
                // (review of #647, pass 5).
                let together_sent = (placed || plan.delta.together)
                    .then(|| plan.next.together.as_ref().map(|f| f.value.as_str()))
                    .flatten();
                for ((p, entry), anchor) in described.iter().zip(&entries).zip(anchors) {
                    let name = crate::picture::shown(&p.who);
                    let text = entry
                        .as_ref()
                        .map(|e| e.text.trim().trim_end_matches('.').to_string())
                        .unwrap_or_default();
                    // Someone this edit dresses has the new outfit in the
                    // instruction ("Dress Maya in …"), whatever the person
                    // line says, so it is neither left out here nor claimed
                    // as left out (review of #647, pass 7).
                    let in_act = !plan.delta.dressed.contains(&p.who.key())
                        && clothes_in_act(
                            p,
                            &p.doing,
                            &[together_sent, plan.also.as_deref()],
                            &everyone,
                            &asked_wearing,
                        );
                    if in_act {
                        clothes_said
                            .push((name.clone(), matches!(p.who, crate::scene::Who::Library(_))));
                    }
                    said.push_str(&edit_person(&name, &text, p, together, in_act));
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
                            full_body: None,
                            details: Vec::new(),
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
                                full_body: None,
                                details: Vec::new(),
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
        // A face on a painted region (owner, 2026-10-16). A region carrying a
        // face (a picture the owner dropped on it, or the one library
        // character its words name) takes that face's tight crop as a
        // reference, and the picture is sampled unmasked, then laid back
        // through the painted regions only. Measured on the warm server the
        // same day, the owner's photo and a crop of the face wanted, two
        // seeds: unmasked with the crop held that face at ArcFace .79–.85
        // with every pixel outside the paint the original's; sampled inside
        // the mask it reached .40–.52, the original face pulling back through
        // the noised source; the edits before this, with no face sent at
        // all, .10–.25. Nothing outside the paint is drawn, so the
        // references go at the unmasked size even for a person without a
        // crop: the canvas is nobody's identity source there.
        // Who the prompt reads as "the viewer": the plan's offstage people,
        // and anyone a face region named whose face could not be sent, but
        // only on a picture whose people are known: on one with no record a
        // name may be someone already in it, and the edit keeps their face
        // from the canvas (`picture::plan`'s `checks_names`, mecha-a3's G1;
        // review of #706, pass 5).
        let mut viewers = plan.offstage.clone();
        let names_checked = plan.next.people_known;
        if mask_plan.is_some() && !call.regions.is_empty() {
            let mut lines: Vec<String> = Vec::new();
            let mut face_regions: Vec<crate::picture::Region> = Vec::new();
            for r in &call.regions {
                let found = match &r.reference {
                    Some(path) => {
                        let read = match read_references(ctx, std::slice::from_ref(path)).await {
                            Ok(mut read) => read.remove(0),
                            Err(why) => {
                                return Ok(refused(format!(
                                    "The picture on the {} region could not be read ({why}), \
                                     so nothing was drawn. Ask the owner to drop it again.",
                                    r.colour
                                )))
                            }
                        };
                        // Its face only when the region's words are about one:
                        // "a red dress like this" over a model in it keeps the
                        // dress, never her face (review of #706, pass 3).
                        let about_face = crate::picture::about_a_face(&r.words);
                        let anchor = if about_face {
                            let faces = Arc::clone(&self.faces);
                            let bytes = read.bytes.clone();
                            tokio::task::spawn_blocking(move || faces.crop(&bytes))
                                .await
                                .unwrap_or_else(|_| {
                                    crate::face::Anchor::Unavailable(
                                        "the face detector failed".into(),
                                    )
                                })
                        } else {
                            crate::face::Anchor::NoFace
                        };
                        match anchor {
                            crate::face::Anchor::Crop(crop) => {
                                Some((crop, "png", true, path.clone()))
                            }
                            other => {
                                // The whole picture, then: a look, not a
                                // face, said either way. Fitted off the
                                // executor, as the canvas is: a phone photo
                                // decodes, shrinks and re-encodes here.
                                match other {
                                    crate::face::Anchor::Unavailable(why) => dropped.push(format!(
                                        "No face could be taken from {path} ({why})."
                                    )),
                                    _ if about_face => {
                                        dropped.push(format!("No face was found in {path}."))
                                    }
                                    _ => {}
                                }
                                let budget = self.reference_pixels;
                                let bytes = read.bytes;
                                let fitted = tokio::task::spawn_blocking(move || {
                                    let fitted = fit_reference(&bytes, budget);
                                    (fitted, bytes)
                                })
                                .await;
                                match fitted {
                                    Ok((Some(fitted), _)) => {
                                        Some((fitted, "png", false, path.clone()))
                                    }
                                    Ok((None, bytes)) => {
                                        Some((bytes, read.ext, false, path.clone()))
                                    }
                                    Err(e) => {
                                        return Ok(refused(format!(
                                            "The picture on the {} region could not be prepared: \
                                             {e}",
                                            r.colour
                                        )))
                                    }
                                }
                            }
                        }
                    }
                    // A name in a region's words takes that character's face
                    // only when the words are about a face: "Maya's shirt in
                    // red" is a shirt (review of #706).
                    None => match crate::imagelib::named_in(&lib, &r.words).as_slice() {
                        [name] if crate::picture::about_a_face(&r.words) => {
                            let entry = lib
                                .get(crate::imagelib::Kind::Character, name)
                                .filter(|e| e.status == crate::imagelib::Status::Approved)
                                .cloned();
                            match entry {
                                Some(entry) => {
                                    let (entry_version, entry_portrait) =
                                        (entry.version, entry.portrait.clone());
                                    let faces = Arc::clone(&self.faces);
                                    let for_faces = lib.clone();
                                    let anchor = tokio::task::spawn_blocking(move || {
                                        faces.anchor(&for_faces, &entry)
                                    })
                                    .await
                                    .unwrap_or_else(|_| {
                                        crate::face::Anchor::Unavailable(
                                            "the face detector failed".into(),
                                        )
                                    });
                                    let shown = capitalized(name);
                                    match anchor {
                                        crate::face::Anchor::Crop(crop) => {
                                            used.push(crate::imagelib::Used {
                                                kind: crate::imagelib::Kind::Character,
                                                name: name.clone(),
                                                version: entry_version,
                                                portrait: entry_portrait.clone(),
                                                full_body: None,
                                                details: Vec::new(),
                                            });
                                            Some((crop, "png", true, format!("{shown}'s face")))
                                        }
                                        crate::face::Anchor::NoFace => {
                                            if names_checked {
                                                viewers.push(name.clone());
                                            }
                                            dropped.push(format!(
                                                "No face was found in {shown}'s library \
                                                 picture, so the {} region was drawn from its \
                                                 words alone{}.",
                                                r.colour,
                                                viewer_said(names_checked, &shown)
                                            ));
                                            None
                                        }
                                        crate::face::Anchor::Unavailable(why) => {
                                            if names_checked {
                                                viewers.push(name.clone());
                                            }
                                            dropped.push(format!(
                                                "{shown}'s face could not be taken from the \
                                                 library ({why}), so the {} region was drawn \
                                                 from its words alone{}.",
                                                r.colour,
                                                viewer_said(names_checked, &shown)
                                            ));
                                            None
                                        }
                                    }
                                }
                                None => None,
                            }
                        }
                        // None named, or several: the words alone say whose.
                        _ => None,
                    },
                };
                if let Some((bytes, ext, is_face, from)) = found {
                    if is_face {
                        face_regions.push(r.clone());
                    }
                    req.references.push(Reference {
                        path: format!("the {} region's reference", r.colour),
                        bytes,
                        ext,
                    });
                    let k = req.references.len();
                    lines.push(if is_face {
                        format!(
                            "In the {} outline, take only the facial identity from <image{k}>, \
                             nothing else.",
                            r.colour
                        )
                    } else {
                        format!(
                            "In the {} outline, take the look from <image{k}>.",
                            r.colour
                        )
                    });
                    dropped.push(format!(
                        "The {} region {} {from}.",
                        r.colour,
                        if is_face {
                            "was given the face from"
                        } else {
                            "was given the look of"
                        }
                    ));
                }
            }
            if !lines.is_empty() {
                req.prompt = format!("{} {}", req.prompt.trim(), lines.join(" "));
                // Sampled whole; `mask_plan` still lays the result back
                // through the paint, and reads the change inside it.
                req.mask = None;
                req.size = canvas_dims(&req.references[0].bytes);
                // Small references only when every painted region takes
                // its change from a reference of its own, as measured: a
                // region with none is drawn from the canvas, which at 512
                // lost identity (.66–.70 to .42–.53, #601), so it keeps the
                // full canvas (review of #706).
                if req.size.is_some() && lines.len() == call.regions.len() {
                    req.reference_size = UNMASKED_EDIT_REFERENCE_SIZE;
                }
            }
            // A face laid back through a brush stroke takes the stroke's edge,
            // not the jaw's or the hairline's: the job finds the face, and the
            // composite follows it inside the face regions (S2).
            if let (false, Some(index), Some(mp)) =
                (face_regions.is_empty(), &region_index, &mask_plan)
            {
                let others: Vec<crate::picture::Region> = call
                    .regions
                    .iter()
                    .filter(|r| !face_regions.iter().any(|f| f.rgb == r.rgb))
                    .cloned()
                    .collect();
                let (cw, ch) = mp.picture.dimensions();
                let index = index.clone();
                let hard = tokio::task::spawn_blocking(move || {
                    regions_hard(&index, &others.iter().collect::<Vec<_>>(), cw, ch)
                })
                .await;
                match hard {
                    Ok(Ok(others)) => {
                        face_edges = Some(FaceEdges {
                            others,
                            hair: face_regions.iter().any(|r| about_hair(&r.words)),
                            colours: face_regions.iter().map(|r| r.colour.clone()).collect(),
                        })
                    }
                    Ok(Err(e)) => tracing::warn!("image_generate: region paint unread: {e}"),
                    Err(e) => tracing::warn!("image_generate: region paint unread: {e}"),
                }
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
        req.prompt = crate::picture::as_viewer(&req.prompt, &viewers, &rendered);
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
        // An area named in words, on an edit of the picture with no painted
        // mask: found in the job, after admission, before the render
        // (`docs/SEGMENTATION-DESIGN.md` S1 and §3).
        let segment_words = plan.mask_words.clone().filter(|_| {
            mask_plan.is_none()
                && !req.references.is_empty()
                && matches!(
                    plan.render,
                    crate::picture::Render::Edit {
                        canvas: crate::picture::Canvas::Picture(_),
                        ..
                    }
                )
        });
        // Its line in the chat's queue, taken before the job owns the plan.
        let label = queue_label(&plan.next);
        let job = async move {
            let (me, ctx) = (&me, &job_ctx);
            me.generation.fetch_add(1, Ordering::SeqCst);
            let started = Instant::now();
            let timeout = Duration::from_secs(me.cfg.timeout_secs);
            // Every segmentation in the job shares one budget, apart from the
            // render's own timeout, so a wedged one holds the chat's job slot
            // for at most that much more, never a second or third timeout
            // (review of #710, pass 5; #624's shared deadline).
            let seg_deadline = Instant::now() + SEGMENT_BUDGET.min(timeout);
            // The named area becomes the painted mask it names; anything that
            // keeps it from being one draws the whole picture, as the words
            // were before, and says so. Never a refusal, so every
            // segmentation is followed by this job's render (§3).
            let mut seg_left: Option<String> = None;
            let mut seg_cancelled = false;
            let unusable = |words: &str| {
                format!(
                    "What matched '{words}' could not be used as a mask, so the change was made \
                     to the whole picture."
                )
            };
            // The words whose area became the mask, when one was found, and
            // how many separate areas it is.
            let mut masked_by: Option<(String, usize)> = None;
            if let Some(words) = &segment_words {
                let found = match me.backend.ready_to_segment(ctx.cancel.as_ref()).await {
                    Err(None) => {
                        seg_cancelled = true;
                        Err("it was stopped".to_string())
                    }
                    Err(Some(why)) => Err(why),
                    Ok(()) => {
                        let (got, left) = me
                            .backend
                            .segment(
                                &me.cfg,
                                &req.references[0],
                                words,
                                ctx.cancel.as_ref(),
                                seg_deadline.saturating_duration_since(Instant::now()),
                                ctx.image_trail.as_deref(),
                            )
                            .await;
                        seg_left = left;
                        match got {
                            Err(Failure::Cancelled) => {
                                seg_cancelled = true;
                                Err("it was stopped".to_string())
                            }
                            Err(Failure::Other(e)) => {
                                tracing::warn!("image_generate: segmentation failed: {e:#}");
                                Err("the segmentation failed".to_string())
                            }
                            Ok(found) => Ok(found),
                        }
                    }
                };
                match found {
                    Err(_) if seg_cancelled => {}
                    Err(why) => dropped.push(format!(
                        "The area '{words}' could not be found ({why}), so the change was made \
                         to the whole picture."
                    )),
                    Ok(None) => dropped.push(format!(
                        "Nothing in the picture matched '{words}', so the change was made to the \
                         whole picture."
                    )),
                    Ok(Some(mask)) => {
                        let canvas = req.references[0].bytes.clone();
                        let prepared = tokio::task::spawn_blocking(move || {
                            let mask = png_bytes(&image::DynamicImage::ImageLuma8(mask).to_rgb8())?;
                            let plan = prepare_mask(&canvas, &mask, EDIT_REFERENCE_SIZE)?;
                            // Counted on the mask the edit uses, after the
                            // grow: two finds the grow joins are one area.
                            let areas = found_areas(&plan.soft).max(1);
                            let picture = png_bytes(&plan.picture)?;
                            let soft = png_bytes(
                                &image::DynamicImage::ImageLuma8(plan.soft.clone()).to_rgb8(),
                            )?;
                            Ok::<_, String>((plan, picture, soft, areas))
                        })
                        .await;
                        match prepared {
                            Ok(Ok((mp, picture, soft, areas))) => {
                                req.references[0].bytes = picture;
                                req.references[0].ext = "png";
                                req.mask = Some(Reference {
                                    path: format!("the area '{words}'"),
                                    bytes: soft,
                                    ext: "png",
                                });
                                req.size = None;
                                req.reference_size = EDIT_REFERENCE_SIZE;
                                mask_plan = Some(mp);
                                masked_by = Some((words.clone(), areas));
                            }
                            // `prepare_mask`'s two refusals of a mark too
                            // small; anything else is an error, logged.
                            Ok(Err(why)) if why == MASK_MARKS_NOTHING || why == MASK_TOO_SMALL => {
                                dropped.push(format!(
                                    "What matched '{words}' was too small to edit alone, so \
                                     the change was made to the whole picture."
                                ))
                            }
                            Ok(Err(why)) => {
                                tracing::warn!("image_generate: a found mask was unusable: {why}");
                                dropped.push(unusable(words));
                            }
                            Err(e) => {
                                tracing::warn!("image_generate: a found mask was unusable: {e}");
                                dropped.push(unusable(words));
                            }
                        }
                    }
                }
            }
            // A face region's composite follows the face SAM finds on the
            // canvas, inside the paint (S2). Anything that keeps it from
            // doing so lays the face back along the paint, as before, and
            // says so; never a refusal.
            if let (Some(fe), Some(mp)) = (&face_edges, mask_plan.as_mut()) {
                let which = match fe.colours.as_slice() {
                    [one] => format!("the {one} region"),
                    many => format!("the {} regions", many.join(" and ")),
                };
                let mut found: std::result::Result<Option<image::GrayImage>, String> = Ok(None);
                let mut took_hair = false;
                let mut hair_failed = false;
                match me.backend.ready_to_segment(ctx.cancel.as_ref()).await {
                    Err(None) => seg_cancelled = true,
                    Err(Some(why)) => found = Err(why),
                    Ok(()) => {
                        let asked: &[&str] = if fe.hair {
                            &["face", "hair"]
                        } else {
                            &["face"]
                        };
                        for words in asked {
                            let (got, left) = me
                                .backend
                                .segment(
                                    &me.cfg,
                                    &req.references[0],
                                    words,
                                    ctx.cancel.as_ref(),
                                    seg_deadline.saturating_duration_since(Instant::now()),
                                    ctx.image_trail.as_deref(),
                                )
                                .await;
                            if let Some(left) = left {
                                seg_left = Some(match seg_left.take() {
                                    Some(before) => format!("{before}; {left}"),
                                    None => left,
                                });
                            }
                            match got {
                                Err(Failure::Cancelled) => {
                                    seg_cancelled = true;
                                    break;
                                }
                                // A face already found is kept when only
                                // the hair fails: the face edge is the
                                // better composite, and the hair is said
                                // to be the original's (review of #710).
                                Err(Failure::Other(e))
                                    if *words == "hair" && matches!(found, Ok(Some(_))) =>
                                {
                                    tracing::warn!("image_generate: segmentation failed: {e:#}");
                                    hair_failed = true;
                                    break;
                                }
                                Err(Failure::Other(e)) => {
                                    tracing::warn!("image_generate: segmentation failed: {e:#}");
                                    found = Err("the segmentation failed".into());
                                    break;
                                }
                                // No face, no face edge: the hair alone
                                // would lay the new face into the hair and
                                // say both followed their edges (review of
                                // #710). The paint, said, instead.
                                Ok(None) if *words == "face" => break,
                                Ok(None) => {}
                                Ok(Some(m)) => {
                                    took_hair |= *words == "hair";
                                    found = Ok(Some(match found {
                                        Ok(Some(mut acc)) if acc.dimensions() == m.dimensions() => {
                                            for (a, b) in acc.pixels_mut().zip(m.pixels()) {
                                                a.0[0] = a.0[0].max(b.0[0]);
                                            }
                                            acc
                                        }
                                        _ => m,
                                    }));
                                }
                            }
                        }
                    }
                }
                if !seg_cancelled {
                    let laid = match found {
                        Err(why) => Err(format!(
                            "The new face in {which} was laid back along the paint: the face's \
                             own edges could not be found ({why})."
                        )),
                        Ok(None) => Err(format!(
                            "No face was found in the picture, so the new face in {which} was \
                             laid back along the paint."
                        )),
                        Ok(Some(found)) => {
                            let (soft, others) = (mp.soft.clone(), fe.others.clone());
                            match tokio::task::spawn_blocking(move || {
                                edge_faces(&soft, others.as_ref(), &found)
                            })
                            .await
                            {
                                Ok(Some(edged)) => Ok(edged),
                                _ => Err(format!(
                                    "The face found in the picture is not inside {which}, so the \
                                     new face was laid back along the paint."
                                )),
                            }
                        }
                    };
                    match laid {
                        Ok(edged) => {
                            mp.soft = edged;
                            dropped.push(if took_hair {
                                format!(
                                    "The new face and hair in {which} were laid back along \
                                     their own edges rather than the paint's."
                                )
                            } else {
                                let hair = match (fe.hair, hair_failed) {
                                    (false, _) => "",
                                    (true, false) => {
                                        " No hair was found, so the hair is the original's."
                                    }
                                    (true, true) => {
                                        " The hair could not be found (the segmentation failed), \
                                         so the hair is the original's."
                                    }
                                };
                                format!(
                                    "The new face in {which} was laid back along the face's own \
                                     edges rather than the paint's, so the hair and everything \
                                     around the face are the original's.{hair}"
                                )
                            });
                        }
                        Err(said) => dropped.push(said),
                    }
                }
            }
            // Built in layers when the switch decided so; any step that
            // fails draws the single pass and says so (§15.5).
            let mut layered: Option<(crate::scene::Layers, Vec<Value>)> = None;
            let mut layers_note: Option<String> = None;
            let mut layer_left: Vec<String> = Vec::new();
            let Outcome { image, left } = if seg_cancelled {
                Outcome {
                    image: Err(Failure::Cancelled),
                    left: seg_left.take(),
                }
            } else {
                match &layers_plan {
                    Some(lp) => match me
                        .build_layers(ctx, lp, &req, timeout, &stamp, &mut layer_left)
                        .await
                    {
                        Ok(built) => {
                            layered = Some((built.record, built.steps));
                            Outcome {
                                image: Ok(built.bytes),
                                left: built.left,
                            }
                        }
                        Err(LayersFailed::Cancelled(left)) => Outcome {
                            image: Err(Failure::Cancelled),
                            left,
                        },
                        Err(LayersFailed::At(step, why)) => {
                            tracing::warn!("image_generate: layers failed at {step}: {why}");
                            layers_note = Some(format!(
                                "Layers failed at {step}: {why}; drawn in one pass."
                            ));
                            let mut out = me
                                .backend
                                .generate(
                                    &me.cfg,
                                    &req,
                                    ctx.cancel.as_ref(),
                                    timeout,
                                    ctx.image_trail.as_deref(),
                                )
                                .await;
                            // The failed build's own copies the server kept,
                            // said beside the single pass's.
                            layer_left.extend(out.left.take());
                            out.left = (!layer_left.is_empty()).then(|| layer_left.join("; "));
                            out
                        }
                    },
                    None => {
                        me.backend
                            .generate(
                                &me.cfg,
                                &req,
                                ctx.cancel.as_ref(),
                                timeout,
                                ctx.image_trail.as_deref(),
                            )
                            .await
                    }
                }
            };
            me.arm_unload();
            // The segmentation's own temp copies, said beside the render's.
            let left = match (left, seg_left) {
                (Some(a), Some(b)) => Some(format!("{a}; {b}")),
                (a, b) => a.or(b),
            };
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
                // A picture drawn in one pass has no layers, whatever the
                // picture it came from had.
                next.layers = layered.as_ref().map(|(r, _)| r.clone());
                (slot.clone(), next)
            });
            let size = match req.size {
                Some((w, h)) => format!("{w}×{h}"),
                None => "picture-shaped".to_string(),
            };
            // Library names only: a described person's name is their
            // description, and the manifest carries no scene words.
            let clothes_named: Option<Vec<&String>> = {
                let named: Vec<&String> = clothes_said
                    .iter()
                    .filter(|(_, library)| *library)
                    .map(|(n, _)| n)
                    .collect();
                (!named.is_empty() && layered.is_none()).then_some(named)
            };
            // Described people by count only: their name is their
            // description, and the manifest carries no scene words, but the
            // omission is never a silent null (review of #647, pass 8).
            let clothes_others = clothes_said.iter().filter(|(_, library)| !*library).count();
            let clothes_others =
                (clothes_others > 0 && layered.is_none()).then_some(clothes_others);
            let manifest = json!({
                "image": path,
                "tool_use_id": ctx.call_id,
                "created": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                "route": if layered.is_some() { "layered" } else { plan.route },
                "layers": layered.as_ref().map(|(_, steps)| json!(steps)),
                "layers_fell_back": layers_note,
                "picture": picture_path,
                "mask": plan.mask,
                "mask_words": masked_by.as_ref().map(|(w, _)| w),
                "seed": req.seed,
                "steps": req.sampling.steps,
                "size": req.size,
                // Who was drawn, by name and entry version: what save-to-library
                // reads. Never the scene itself, which lives outside the jail.
                "cast": used.iter().filter(|u| u.kind == crate::imagelib::Kind::Character)
                    .map(|u| {
                        let mut row =
                            json!({"name": u.name, "version": u.version, "portrait": u.portrait});
                        // The figure that went beside the portrait (§9.4),
                        // by hash like the portrait: the version says only
                        // that one was attached.
                        if let Some(blob) = &u.full_body {
                            row["full_body"] = json!(blob);
                        }
                        // The close-ups that went, by hash, never by label.
                        if !u.details.is_empty() {
                            row["details"] = json!(u.details);
                        }
                        row
                    })
                    .collect::<Vec<_>>(),
                "style": used.iter().find(|u| u.kind == crate::imagelib::Kind::Style)
                    .map(|u| json!({"name": u.name, "version": u.version})),
                "crops": crops_said,
                "roles": roles_said,
                // Only for the prompt that was sent: a layered picture's
                // passes carry each person's outfit on the cutout, and the
                // one-pass prompt this names was never drawn (review of #647).
                "clothes_in_act": clothes_named,
                "clothes_in_act_others": clothes_others,
                "reader": reader_said,
                "layout_similarity": similarity.map(|r| (r * 1000.0).round() / 1000.0),
                "scene": landed.as_ref().map(|(_, s)| json!({"picture": s.picture})),
                "model": {
                    "backend": "comfyui",
                    "diffusion_model": me.cfg.model_file(req.sampling.model),
                    "text_encoder": me.cfg.text_encoder,
                    "vae": me.cfg.vae,
                },
                // How it was sampled, beside the steps above: the owner's
                // persona settings, resolved. Whether the negative was sent,
                // never its text, which is fixed in code.
                "sampling": {
                    "cfg": req.sampling.cfg,
                    "sampler": req.sampling.sampler,
                    "scheduler": req.sampling.scheduler,
                    "negative": !req.sampling.negative.is_empty(),
                },
                // The owner's LoRAs, by count: the model had no say in them,
                // and the manifest names none (`docs/LORA-DESIGN.md` §4).
                "loras": loras_counted(&picked, lora_off),
            });
            // The prompt, for the owner only, outside the jail (the manifest
            // never carries it): a line that cannot be written costs the
            // picture nothing. A layered picture's passes wrote their own
            // lines; this compiled prompt was never sent (review of #626).
            if let Some(log) = ctx.prompt_log.as_ref().filter(|_| layered.is_none()) {
                let line = json!({
                    "created": manifest["created"],
                    "image": path,
                    "route": plan.route,
                    "seed": req.seed,
                    "sampling": manifest["sampling"],
                    "loras": manifest["loras"],
                    "roles": manifest["roles"],
                    "reader": manifest["reader"],
                    "prompt": with_triggers(&req.prompt, &req.loras),
                    // The chat-derived words in it, apart from the compiler's
                    // own sentences: what `check-private` reads, so a prompt
                    // studied here is never pasted into the repository.
                    "words": record_prose(&plan.next)
                        .into_iter()
                        .chain(plan.also.clone())
                        .chain(split_words.iter().cloned())
                        .filter(|w| !w.trim().is_empty())
                        .collect::<Vec<_>>(),
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
                .map(|u| format!("{} {} (v{})", u.kind.label(), u.name, u.version))
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
            let drawn = format!(
                "drawn in {secs} s (seed {}, {} steps)",
                req.seed, req.sampling.steps
            );
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
                let area = match (&plan.mask, &masked_by) {
                    (Some(m), _) => format!("the area painted in {m}"),
                    (None, Some((w, 1))) => format!("what was found for '{w}' (1 area)"),
                    (None, Some((w, n))) => format!("what was found for '{w}' ({n} areas)"),
                    (None, None) => "the painted area".to_string(),
                };
                if size_asked {
                    text.push_str(
                        " (The size asked for was not used: a masked edit keeps the picture's own \
                         shape.)",
                    );
                }
                let (cw, ch) = mp.picture.dimensions();
                text.push_str(&if mp.source == (cw, ch) {
                    format!(
                        " Only {area} was redrawn, blended over a narrow \
                         edge around it; everything beyond that edge is the original, pixel for \
                         pixel."
                    )
                } else {
                    let (pw, ph) = mp.source;
                    format!(
                        " Only {area} was redrawn. The picture is \
                         {pw}×{ph} and was edited at {cw}×{ch}, its edit size, as any edit of it \
                         is; outside that area the result is the original at that size."
                    )
                });
            }
            if layered.is_some() {
                text.push_str(
                    " It was built in layers: the room, each person on their own, then placed \
                     together and finished.",
                );
            }
            text.push_str(&loras_said(
                &picked,
                lora_off,
                with_triggers(&req.prompt, &req.loras) != req.prompt,
            ));
            if let Some(note) = &layers_note {
                text.push(' ');
                text.push_str(note);
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
            // Where a newcomer's clothes came from, unless the prompt left
            // them out: then the sentence would tell the model something the
            // picture does not show (review of #647, pass 4).
            for (who, note) in &plan.clothes_said {
                if layered.is_some() || !clothes_said.iter().any(|(n, _)| n == who) {
                    text.push(' ');
                    text.push_str(note);
                }
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

/// A layered build that finished: the picture, its record, each step's seed
/// and seconds, and any server temp copies left.
struct Built {
    bytes: Vec<u8>,
    record: crate::scene::Layers,
    steps: Vec<Value>,
    left: Option<String>,
}

/// Why a layered build ended early: a Stop, or the step that failed and
/// why, which draws the single pass instead (IMAGE-DESIGN.md §15.5).
enum LayersFailed {
    Cancelled(Option<String>),
    At(String, String),
}

/// Write `bytes` to the workspace path `name`, created new and never through
/// a link, as `save` writes the picture itself.
async fn save_named(ctx: &ToolCtx, name: &str, bytes: &[u8]) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let path = ctx.resolve(name)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options
        .open(&path)
        .await
        .map_err(|e| anyhow!("cannot write {name}: {e}"))?;
    file.write_all(bytes).await?;
    file.flush().await?;
    Ok(())
}

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
    // `mode` holds only when the file is made: an older one is narrowed too.
    use std::os::unix::fs::PermissionsExt;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
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
    /// The default asks for this system's cold figure, and on each system
    /// the load cost is that figure less a reload's need.
    #[test]
    fn the_load_cost_is_the_default_less_a_reload() {
        assert_eq!(
            ImageConfig::default().min_available_mb,
            min_available_mb_on(std::env::consts::OS)
        );
        for os in ["linux", "macos"] {
            assert_eq!(
                load_cost_mb_on(os) + reload_mb_on(os),
                min_available_mb_on(os),
                "{os}"
            );
        }
        assert_eq!(min_available_mb_on("linux"), 20_480);
        assert_eq!(load_cost_mb_on("linux"), 8_192);
        // A Mac's covers the bf16 file's measured cold peak (§12.3).
        let mac_row = crate::recommend::SLOTS
            .iter()
            .find(|s| s.id == "image")
            .and_then(|s| s.mac_rows.first())
            .unwrap();
        let crate::recommend::Memory::Unified { peak } = mac_row.memory else {
            panic!("the Mac image row is unified");
        };
        assert!(min_available_mb_on("macos") >= peak.mb().unwrap() as u64);
    }

    #[test]
    fn the_memory_asked_for_follows_what_the_server_holds() {
        let cold = min_available_mb_on(std::env::consts::OS);
        assert_eq!(memory_need_mb(Some(false), cold, false), cold);
        assert_eq!(
            memory_need_mb(None, cold, false),
            cold,
            "unknown is never warm"
        );
        assert_eq!(
            memory_need_mb(Some(true), cold, false),
            reload_mb_on(std::env::consts::OS)
        );
        // An operator's raised margin is kept, not capped at the default's.
        assert_eq!(
            memory_need_mb(Some(true), 50_000, false),
            50_000 - load_cost_mb()
        );
        // A small figure is not computed into the "check off" sentinel: the
        // unreadable-gauge refusal must survive it.
        assert_eq!(memory_need_mb(Some(true), 4_000, false), 0);
        // A job that may segment asks for SAM on top, warm or cold: the
        // gauge cannot tell SAM from the image model once either has loaded.
        for loaded in [Some(true), Some(false), None] {
            assert_eq!(
                memory_need_mb(loaded, cold, true),
                memory_need_mb(loaded, cold, false) + SEGMENT_NEED_MB
            );
        }
        // A need of 0 is the reserve alone, and the floor still refuses an
        // unreadable gauge for it: only the operator's own 0 is "off".
        assert!(!crate::system::floor::admit(
            &crate::system::Reading::Unread { why: "x".into() },
            &crate::system::Reading::NotHere,
            0,
            &crate::system::floor::SystemConfig::default(),
        )
        .admitted());
        // Only the operator's own 0 switches it off.
        assert_eq!(memory_need_mb(Some(true), 0, false), 0);
    }

    /// The stats as ComfyUI 0.37 answered them on 2026-10-02, fresh, loaded
    /// and after `/free`; anything else reads as unknown.
    #[test]
    fn loaded_is_read_from_the_servers_stats() {
        let stats = |total: Value| json!({"devices": [{"name": "cuda:0", "type": "cuda", "torch_vram_total": total}]});
        assert_eq!(loaded_from_stats(&stats(json!(0))), Some(false));
        assert_eq!(
            loaded_from_stats(&stats(json!(6_000_000_000u64))),
            Some(true)
        );
        assert_eq!(loaded_from_stats(&stats(json!("5 GB"))), None);
        assert_eq!(loaded_from_stats(&json!({"devices": []})), None);
        assert_eq!(loaded_from_stats(&json!({"system": {}})), None);
        // MPS and the CPU report the machine's RAM there from the start
        // (`get_total_memory` at `comfyui_unit::COMMIT`): never a load's
        // sign, so never warm.
        for kind in ["mps", "cpu"] {
            let fresh = json!({"devices": [{"name": kind, "type": kind,
                "torch_vram_total": 137_438_953_472u64}]});
            assert_eq!(loaded_from_stats(&fresh), None, "{kind}");
        }
        let cuda = json!({"devices": [{"name": "cuda:0", "type": "cuda",
            "torch_vram_total": 0}]});
        assert_eq!(loaded_from_stats(&cuda), Some(false));
        // A device that names no type, or one this has not seen, is unknown
        // — never warm (review of #681, pass 2).
        for device in [
            json!({"name": "cuda:0", "torch_vram_total": 6_000_000_000u64}),
            json!({"name": "xpu:0", "type": "xpu", "torch_vram_total": 6_000_000_000u64}),
        ] {
            assert_eq!(loaded_from_stats(&json!({"devices": [device]})), None);
        }
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

    /// Segmenting inside a job waits out a restart as a render does: a
    /// word mask asked during the idle reset keeps its mask rather than
    /// drawing the whole picture (review of #710, pass 3).
    #[tokio::test]
    async fn segmentation_waits_for_a_restarting_server() {
        let addr = closed_port();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    let body = if head.contains("/object_info/CheckpointLoaderSimple") {
                        json!({"CheckpointLoaderSimple": {"input": {"required": {
                            "ckpt_name": [[SEGMENT_MODEL], {}]}}}})
                    } else if head.contains("/object_info/SAM3_Detect") {
                        json!({"SAM3_Detect": {"input": {}}})
                    } else {
                        json!({})
                    }
                    .to_string();
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(reply.as_bytes()).await;
                });
            }
        });
        let ready = server_at(addr).ready_to_segment(None).await;
        assert!(ready.is_ok(), "{ready:?}");
    }

    /// A job whose segmentation budget is spent segments nothing: no upload,
    /// no graph, a failure the edit draws past (review of #710, pass 5).
    #[tokio::test]
    async fn a_spent_segmentation_budget_sends_nothing() {
        let (url, seen) = fake_with(Fake {
            sam: true,
            ..Fake::default()
        })
        .await;
        let t = tool(&url);
        let picture = Reference {
            path: "images/a.png".into(),
            bytes: picture(8, [1, 2, 3]),
            ext: "png",
        };
        let (got, _) = t
            .backend
            .segment(&t.cfg, &picture, "the sky", None, Duration::ZERO, None)
            .await;
        assert!(matches!(got, Err(Failure::Other(_))));
        assert!(
            seen.lock().unwrap().is_empty(),
            "{:?}",
            seen.lock().unwrap()
        );
    }

    /// Admission charges SAM for a server it could not ask: unknown is
    /// never "no SAM" (review of #710, pass 3).
    #[tokio::test]
    async fn a_server_that_cannot_be_asked_is_charged_for_segmentation() {
        assert!(server_at(closed_port()).segments().await);
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
        /// The unet loader lists no turbo model: one never fetched.
        no_turbo: bool,
        /// The checkpoint loader lists [`SEGMENT_MODEL`]: segmentation is
        /// installed. `SAM3_Detect` is answered either way, as every node is.
        sam: bool,
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
                no_turbo: false,
                sam: false,
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
            no_turbo,
            sam,
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
                                let mut unets = vec![crate::sampling::standard_model()];
                                if !no_turbo {
                                    unets.push(crate::sampling::turbo_model());
                                }
                                json!({node: {"input": {"required": {"unet_name": [unets, {}]}}}})
                            }
                            "UnetLoaderGGUF" => files("unet_name", "Qwen-Image-2.1-Q4.gguf"),
                            // A server with only some of KSampler's names.
                            "KSampler" => json!({node: {"input": {"required": {
                                "sampler_name": [["euler", "res_multistep"], {}],
                                "scheduler": [["simple", "beta"], {}]}}}}),
                            "CLIPLoader" => files("clip_name", "qwen3vl_8b_w4a8.safetensors"),
                            "VAELoader" => files("vae_name", "qwen_image_2.1_vae_bf16.safetensors"),
                            "CheckpointLoaderSimple" if sam => files("ckpt_name", SEGMENT_MODEL),
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
    fn a_trigger_is_added_once_and_only_where_the_prompt_lacks_it() {
        let l = |t: Option<&str>| crate::loras::PictureLora {
            file: "x.safetensors".into(),
            strength: 1.0,
            trigger: t.map(String::from),
        };
        assert_eq!(with_triggers("a lighthouse", &[]), "a lighthouse");
        assert_eq!(
            with_triggers(
                "a lighthouse",
                &[l(Some("oil paint")), l(None), l(Some("oil paint"))]
            ),
            "oil paint. a lighthouse"
        );
        assert_eq!(
            with_triggers("An Oil Paint lighthouse", &[l(Some("oil paint"))]),
            "An Oil Paint lighthouse"
        );
        assert!(model_lora("Qwen-Image-2.1-Q4.GGUF").is_err());
        assert!(model_lora(crate::sampling::turbo_model()).is_ok());
    }

    #[test]
    fn the_graph_is_fixed_and_the_prompt_is_only_ever_a_value() {
        let cfg = ImageConfig::default();
        // A prompt that is itself a graph fragment must stay a string.
        let hostile = r#"", "class_type": "SaveImage", "evil": {"#;
        let req = Request {
            prompt: hostile.into(),
            sampling: crate::sampling::Sampling::reference(40),
            size: Some((1024, 1024)),
            seed: 7,
            references: Vec::new(),
            reference_size: EDIT_REFERENCE_SIZE,
            mask: None,
            loras: Vec::new(),
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
        assert_eq!(g["sample"]["inputs"]["sampler_name"], "euler");
        assert_eq!(g["sample"]["inputs"]["scheduler"], "simple");
        assert_eq!(g["encode"]["inputs"]["negative_prompt"], "");
    }

    /// A persona's picture settings reach the sampler, the negative and the
    /// loader, and nothing else of the graph moves.
    #[test]
    fn the_owners_settings_reach_the_sampler_and_the_loader() {
        let cfg = ImageConfig::default();
        let guided = crate::sampling::Tuning {
            cfg: Some(3.0),
            steps: Some(30),
            sampler: Some("res_multistep".into()),
            scheduler: Some("beta".into()),
            negative: Some(true),
            ..Default::default()
        };
        let req = |sampling| Request {
            prompt: "a lighthouse at dusk".into(),
            sampling,
            size: Some((1024, 1024)),
            seed: 7,
            references: Vec::new(),
            reference_size: EDIT_REFERENCE_SIZE,
            mask: None,
            loras: Vec::new(),
        };
        let g = comfy_graph(&cfg, &req(guided.resolve(cfg.steps)), &[], None);
        let k = &g["sample"]["inputs"];
        assert_eq!(
            (k["cfg"].as_f64(), k["steps"].as_u64()),
            (Some(3.0), Some(30))
        );
        assert_eq!(
            (k["sampler_name"].as_str(), k["scheduler"].as_str()),
            (Some("res_multistep"), Some("beta"))
        );
        assert_eq!(
            g["encode"]["inputs"]["negative_prompt"],
            crate::sampling::NEGATIVE
        );
        assert_eq!(g["unet"]["inputs"]["unet_name"], cfg.diffusion_model);
        let turbo = crate::sampling::Tuning {
            model: Some(crate::sampling::Model::Turbo),
            ..Default::default()
        };
        let g = comfy_graph(&cfg, &req(turbo.resolve(cfg.steps)), &[], None);
        assert_eq!(
            g["unet"]["inputs"]["unet_name"],
            crate::sampling::turbo_model()
        );
        assert_eq!(g["unet"]["class_type"], "UNETLoader");
        assert_eq!(g["sample"]["inputs"]["steps"], crate::sampling::TURBO_STEPS);
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
                sampling: crate::sampling::Sampling::reference(40),
                size: Some((1024, 1024)),
                seed: 7,
                references: Vec::new(),
                reference_size: EDIT_REFERENCE_SIZE,
                mask: None,
                loras: Vec::new(),
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

    /// An upscale is the picture as the one reference of a standard-model
    /// edit at the edit canvas's size, with nothing else asked; the server's
    /// picture comes back as it is.
    #[tokio::test]
    async fn an_upscale_redraws_the_picture_as_its_only_reference() {
        let red = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 20, 20]));
        let mut result = std::io::Cursor::new(Vec::new());
        red.write_to(&mut result, image::ImageFormat::Png).unwrap();
        let result = result.into_inner();
        let (url, log) = fake_with(Fake {
            history: vec![done()],
            views: vec![result.clone()],
            ..Fake::default()
        })
        .await;
        let out = tool(&url)
            .upscale(&picture(8, [240, 220, 40]), 7)
            .await
            .unwrap();
        assert_eq!(out, result);
        let seen = log.lock().unwrap().clone();
        let uploads = seen
            .iter()
            .filter(|l| l.starts_with("POST /upload/image"))
            .count();
        assert_eq!(uploads, 1, "the picture alone");
        let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
        assert!(
            submitted.contains("Recreate this exact photograph"),
            "{submitted}"
        );
        assert!(submitted.contains("\"resolution\":1024"), "{submitted}");
        assert!(!submitted.contains("images.image_2"), "{submitted}");
        assert!(
            !submitted.contains("EmptyLatentImage"),
            "the encoder's canvas: {submitted}"
        );
        let before = seen.len();
        assert!(
            tool(&url).upscale(b"not a picture", 7).await.is_err(),
            "refused before the server"
        );
        let big = image::RgbImage::from_pixel(1024, 1024, image::Rgb([9, 9, 9]));
        let mut png = std::io::Cursor::new(Vec::new());
        big.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let why = tool(&url).upscale(&png.into_inner(), 7).await.unwrap_err();
        assert!(why.contains("already 1024×1024"), "{why}");
        assert_eq!(
            log.lock().unwrap().len(),
            before,
            "neither reached the server"
        );
    }

    /// The operator's `[system]` floor reaches the persona form, which is
    /// where most pictures are made — never R5's defaults instead.
    #[test]
    fn the_persona_form_keeps_the_operators_memory_floor() {
        let floor = crate::system::floor::SystemConfig {
            memory_reserve_mb: 1234,
            memory_pressure_ceiling: 42.0,
        };
        let tool = ImageGenerate::new(ImageConfig {
            url: "http://127.0.0.1:8188".into(),
            ..Default::default()
        })
        .unwrap()
        .with_floor(floor.clone());
        assert_eq!(tool.floor, floor);
        assert_eq!(tool.persona_form(None).floor, floor);
    }

    /// The unload timer stands down only for mecha's proxy-fronted agent on
    /// the shipped port: never for an older agent that runs ComfyUI itself,
    /// a URL naming another port, or no agent (review of #689).
    #[test]
    fn the_unload_timer_is_left_to_mecha_s_proxy_only() {
        assert_eq!(ImageConfig::default().unload_after_secs, 900);
        if !cfg!(target_os = "macos") {
            assert!(!behind_mecha_s_proxy("http://127.0.0.1:8188"));
        }
        let home = std::env::temp_dir().join(format!("mecha-proxy-{}", uuid::Uuid::new_v4()));
        let plist = crate::launchd::agents_dir(&home).join(crate::launchd::plist_name(
            &crate::comfyui_unit::Naming::shipped().unit,
        ));
        let url = "http://127.0.0.1:8188";
        assert!(!behind_proxy_at(&home, url), "no agent");
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        std::fs::write(&plist, "<string>/v/python</string><string>main.py</string>").unwrap();
        assert!(
            !behind_proxy_at(&home, url),
            "an agent from before the proxy"
        );
        std::fs::write(
            &plist,
            "<string>/m/mecha</string><string>features</string><string>on-demand</string>",
        )
        .unwrap();
        assert!(behind_proxy_at(&home, url));
        assert!(
            !behind_proxy_at(&home, "http://127.0.0.1:9188"),
            "another port"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A Mac's available memory is macOS's own level (the owner's ruling,
    /// 2026-10-13): a percentage, and anything else unread.
    #[test]
    fn macos_available_memory_is_its_memorystatus_level() {
        let parse = crate::system::source::parse_memorystatus_level;
        assert_eq!(parse("37\n"), Some(37));
        assert_eq!(parse("0"), Some(0));
        assert_eq!(parse("100"), Some(100));
        assert_eq!(parse("101"), None);
        assert_eq!(parse("-3"), None);
        assert_eq!(parse("sysctl: unknown oid"), None);
        assert_eq!(parse(""), None);
    }

    /// §5.2 moved the result's guidance into the description; these
    /// prohibitions answer measured failures and must not be dropped on the
    /// way: a model copied the result over the original in 3 of 4 live chats
    /// (#429), described pictures it had not seen (review of #581), and
    /// judged two pictures it had not seen as wrong and redrew them, one of
    /// them right after all (owner's Mac, 2026-10-14; the owner's ruling: the
    /// owner says what to change).
    #[test]
    fn the_description_keeps_the_prohibitions() {
        let d = tool("http://127.0.0.1:1").description().to_string();
        assert!(
            d.contains("never write or copy a result over the picture it was edited from"),
            "{d}"
        );
        assert!(d.contains("never say what a picture shows"), "{d}");
        assert!(
            d.contains("never judge whether one came out right, or draw it again, on your own"),
            "{d}"
        );
    }

    #[test]
    fn an_edit_graph_loads_its_references_from_temp_and_samples_on_their_shape() {
        let cfg = ImageConfig::default();
        let req = Request {
            prompt: "Keep <image1> unchanged except the jacket".into(),
            sampling: crate::sampling::Sampling::reference(40),
            size: None,
            seed: 9,
            references: Vec::new(),
            reference_size: EDIT_REFERENCE_SIZE,
            mask: None,
            loras: Vec::new(),
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
            .with_floor(crate::system::floor::SystemConfig {
                memory_reserve_mb: 0,
                memory_pressure_ceiling: f64::INFINITY,
            })
            .polling_every(Duration::from_millis(10))
            .call(json!({"scene": {"setting": "a fox"}}), &ctx(&dir))
            .await
            .unwrap();
            let seen = seen.lock().unwrap();
            let asked = seen.iter().any(|l| l.starts_with("GET /system_stats"));
            let submitted = seen.iter().any(|l| l.starts_with("POST /prompt"));
            (asked, submitted)
        };
        // A floor of 1 MB with no reserve and no pressure ceiling passes on
        // any machine, so the admitted path through the real gate runs to a
        // submitted generation (a test runner may have under 8 GiB free,
        // which R5's reserve would refuse).
        let (asked, submitted) = ask(1).await;
        // A Mac does not ask: the answer is always unknown there, and asking
        // started ComfyUI behind its proxy (review of #689).
        assert_eq!(
            asked,
            !cfg!(target_os = "macos"),
            "the check asked the server: {asked}"
        );
        assert!(submitted, "an admitted generation was never submitted");
        assert!(!ask(0).await.0, "a switched-off check still asked");
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
        // `the_description_keeps_the_prohibitions`) are the guard against
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
            submitted.contains(
                "Exactly two people are in the foreground, Maya and John, each exactly once; \
                 anyone in the background is a stranger, not from any image."
            ),
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

    fn attach_body(lib: &std::path::Path, name: &str, rgb: [u8; 3]) {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb(rgb));
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        crate::imagelib::add_reference(
            lib,
            name,
            crate::imagelib::RefKind::FullBody,
            &png.into_inner(),
            None,
        )
        .unwrap();
    }

    /// IMAGE-COMPILER-DESIGN.md §9.4: a cast of one with a full body sends
    /// two images, bound to that person, and the manifest says which figure
    /// went; a cast of two with a body each sends portraits only (E14).
    #[tokio::test]
    async fn a_body_goes_to_a_picture_of_its_character_alone() {
        let (url, seen) = fake(vec![done(); 2], "200 OK").await;
        let dir = tempdir();
        let lib = library_with(&["maya", "john"]);
        attach_body(&lib, "maya", [70, 1, 1]);
        attach_body(&lib, "john", [70, 2, 2]);
        let t = tool(&url).with_library_dir(lib.clone());
        let uploads = |seen: &[String]| {
            seen.iter()
                .filter(|l| l.starts_with("POST /upload/image"))
                .count()
        };

        let out = t
            .call(
                json!({"scene": {"setting": "a studio", "people": two_people()}, "seed": 5}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains(
                "Full-figure references were not sent: they go only to a picture of one person, \
                 and this one has two."
            ),
            "{}",
            out.content
        );
        assert_eq!(uploads(&seen.lock().unwrap()), 2, "a portrait per person");

        let john = json!([{"who": "john", "wearing": "a flannel shirt", "doing": "smiling"}]);
        let out = t
            .call(
                json!({"scene": {"setting": "a studio", "people": john}, "seed": 5}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(!out.content.contains("Full-figure"), "{}", out.content);
        let seen = seen.lock().unwrap().clone();
        assert_eq!(uploads(&seen) - 2, 2, "a portrait and a full figure");
        let submitted = seen
            .iter()
            .rfind(|l| l.starts_with("POST /prompt"))
            .unwrap();
        assert!(
            submitted.contains(
                "<image1> and <image2> show one person, John: <image1> is John's portrait, \
                 <image2> John's full figure."
            ),
            "{submitted}"
        );
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
        assert!(manifest["cast"][0]["full_body"]
            .as_str()
            .unwrap()
            .starts_with("sha256-"));
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// §9.4, 2026-10-26: a close-up goes after the faces, bound to its person
    /// in the prompt, and the manifest keeps its hash. Its label is prompt
    /// words only and never reaches the result.
    #[tokio::test]
    async fn a_close_up_goes_after_the_faces_and_into_the_manifest() {
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let dir = tempdir();
        let lib = library_with(&["maya", "john"]);
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([71, 1, 1]));
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        crate::imagelib::add_reference(
            &lib,
            "maya",
            crate::imagelib::RefKind::Detail,
            &png.into_inner(),
            Some("left-forearm tattoo".into()),
        )
        .unwrap();
        let t = tool(&url).with_library_dir(lib.clone());
        let out = t
            .call(
                json!({"scene": {"setting": "a studio", "people": two_people()}, "seed": 5}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(!out.content.contains("tattoo"), "{}", out.content);
        let seen = seen.lock().unwrap().clone();
        let uploads = seen
            .iter()
            .filter(|l| l.starts_with("POST /upload/image"))
            .count();
        assert_eq!(uploads, 3, "two portraits and a close-up");
        let submitted = seen
            .iter()
            .rfind(|l| l.starts_with("POST /prompt"))
            .unwrap();
        for words in [
            "Maya's left-forearm tattoo is shown close up in <image3>",
            "<image1> and <image2> serve as identity sources only.",
        ] {
            assert!(submitted.contains(words), "{words}\n{submitted}");
        }
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
        assert!(manifest["cast"][0]["details"][0]
            .as_str()
            .unwrap()
            .starts_with("sha256-"));
        assert!(manifest["cast"][1].get("details").is_none());
        assert!(!manifest.to_string().contains("tattoo"));
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A body in a cast of two is not sent, and the model is told in a count,
    /// never which person or what the picture is labelled.
    #[tokio::test]
    async fn a_body_in_a_cast_of_two_is_not_sent_and_the_result_says_so_in_counts() {
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let dir = tempdir();
        let lib = library_with(&["maya", "john"]);
        attach_body(&lib, "maya", [70, 3, 3]);
        let t = tool(&url).with_library_dir(lib.clone());
        let out = t
            .call(
                json!({"scene": {"setting": "a studio", "people": two_people()}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains(
                "Full-figure references were not sent: they go only to a picture of one person, \
                 and this one has two."
            ),
            "{}",
            out.content
        );
        let seen = seen.lock().unwrap().clone();
        let uploads = seen
            .iter()
            .filter(|l| l.starts_with("POST /upload/image"))
            .count();
        assert_eq!(uploads, 2);
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A picture layers may draw is compiled from portraits, so a layered
    /// build's fallback single pass draws no figure the cutouts did not.
    #[tokio::test]
    async fn a_picture_planned_in_layers_sends_no_bodies() {
        let (url, seen) = fake(vec![done(); 8], "200 OK").await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        attach_body(&lib, "maya", [70, 4, 4]);
        attach_body(&lib, "john", [70, 5, 5]);
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let out = t.call(touching_call(), &cx).await.unwrap();
        assert!(
            out.content.contains(
                "Full-figure references were not sent: a picture planned in layers is drawn \
                 from portraits."
            ),
            "{}",
            out.content
        );
        let seen = seen.lock().unwrap().clone();
        assert!(
            seen.iter()
                .filter(|l| l.starts_with("POST /prompt"))
                .all(|l| !l.contains("full figure")),
            "{seen:?}"
        );
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// Layers considered and then declined (a described person here): the
    /// single pass is the picture, so it carries the bodies, and the result
    /// says only why layers declined (review of #685).
    #[tokio::test]
    async fn layers_that_decline_leave_the_bodies_in_the_single_pass() {
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya"]));
        attach_body(&lib, "maya", [70, 6, 6]);
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let call = json!({"scene": {
            "setting": "a studio",
            "people": [
                {"who": "maya", "wearing": "a yellow raincoat", "doing": "laughing"},
                {"who": "a waiter with a tray", "wearing": "a white apron", "doing": "smiling"}
            ],
            "together": "the waiter hands Maya a cup"
        }});
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("not in layers"), "{}", out.content);
        assert!(!out.content.contains("Full-figure"), "{}", out.content);
        let seen = seen.lock().unwrap().clone();
        let uploads = seen
            .iter()
            .filter(|l| l.starts_with("POST /upload/image"))
            .count();
        assert_eq!(uploads, 2, "the portrait and the full figure");
        let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
        assert!(submitted.contains("Maya's full figure"), "{submitted}");
        for d in [dir, store, lib] {
            std::fs::remove_dir_all(d).ok();
        }
    }

    /// A replay's seed stream (`ToolCtx::image_seeds`): two arms stamped
    /// with the same stream draw a fresh picture at the same seed, and a
    /// collision redraws from the stream, never at random.
    #[tokio::test]
    async fn a_stamped_seed_stream_pairs_fresh_seeds_and_feeds_the_redraw() {
        let (url, _) = fake(vec![done(), done(), done()], "200 OK").await;
        let dir = tempdir();
        let lib = library_with(&["maya", "john"]);
        let t = tool(&url).with_library_dir(lib.clone());
        let seeded = |n: u64| ToolCtx {
            image_seeds: Some(std::sync::Arc::new(crate::sample::SeedStream::new(n))),
            ..ctx(&dir)
        };
        let seed_of = |content: &str| -> u64 {
            let at = content.find("(seed ").expect("a seed is said") + 6;
            content[at..].split(',').next().unwrap().parse().unwrap()
        };
        let first_draw = crate::sample::SeedStream::new(3).next() & 0xFFFF_FFFF;
        let call = json!({"scene": {"setting": "a park", "people": two_people()}});
        let a = t.call(call.clone(), &seeded(3)).await.unwrap();
        let b = t.call(call, &seeded(3)).await.unwrap();
        assert!(!a.is_error && !b.is_error, "{} {}", a.content, b.content);
        assert_eq!(seed_of(&a.content), first_draw);
        assert_eq!(seed_of(&b.content), first_draw);
        // 901 drew john's portrait: the redraw away from it is the stream's.
        let c = t
            .call(
                json!({"scene": {"setting": "a park", "people": two_people()}, "seed": 901}),
                &seeded(3),
            )
            .await
            .unwrap();
        assert_eq!(seed_of(&c.content), first_draw, "{}", c.content);
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
    /// A photo's first call naming its people sends only what the call says
    /// about them: no library description, no default pose and no crop
    /// pulling them toward a portrait (review of #707, pass 2: the plan said
    /// so, and the request did not).
    #[tokio::test]
    async fn people_named_on_a_photo_are_drawn_from_it_with_only_what_was_asked() {
        let (url, seen) = fake_with(Fake {
            history: vec![done(); 2],
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        let lib = library_with(&["maya"]);
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("inbox/her.png"), PNG).unwrap();
        let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [1, 2, 3])));
        let out = tool(&url)
            .with_library_dir(lib.clone())
            .with_faces(faces.clone())
            .call(
                json!({"picture": "inbox/her.png", "scene": {
                    "setting": "a narrow greenhouse",
                    "people": [{"who": "maya", "wearing": "a mustard raincoat"}]}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let first = seen.lock().unwrap().clone();
        let submitted = first
            .iter()
            .find(|l| l.starts_with("POST /prompt"))
            .unwrap();
        assert!(
            submitted.contains("Dress Maya in a mustard raincoat."),
            "{submitted}"
        );
        assert!(!submitted.contains("narrow greenhouse"), "{submitted}");
        assert!(!submitted.contains("standing naturally"), "{submitted}");
        assert!(!submitted.contains("a memorable face"), "{submitted}");
        assert!(!submitted.contains("Add Maya"), "{submitted}");
        assert_eq!(faces.asked.load(Ordering::SeqCst), 0, "no portrait crop");
        // Naming her alone redraws the photo as it is, with no pose for her
        // (review of #707, pass 3: the redraw route still sent one).
        let alone = tool(&url)
            .with_library_dir(lib.clone())
            .with_faces(faces.clone())
            .call(
                json!({"picture": "inbox/her.png", "scene": {"people": [{"who": "maya"}]}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!alone.is_error, "{}", alone.content);
        let sent = seen
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|l| l.starts_with("POST /prompt"))
            .unwrap()
            .clone();
        assert!(sent.contains("Draw this picture again"), "{sent}");
        assert!(!sent.contains("standing naturally"), "{sent}");
        assert!(!sent.contains("a memorable face"), "{sent}");
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(lib).ok();
    }

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
        let (url, _) = distinct(2).await;
        let dir = tempdir();
        let store = tempdir();
        let lib = library_with(&["maya"]);
        let t = tool(&url)
            .with_library_dir(lib.clone())
            .with_faces(Arc::new(Panics));
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("inbox/room.png"), PNG).unwrap();
        let c = scene_ctx(&dir, &store, "chat-a");
        // Naming her on the photo draws her from it: no crop is asked for,
        // so the detector is not reached (review of #707, pass 2).
        let first = t
            .call(
                json!({"picture": "inbox/room.png",
                       "scene": {"people": [{"who": "maya", "wearing": "a coat", "doing": "sitting on the bench"}]}}),
                &c,
            )
            .await
            .unwrap();
        assert!(!first.is_error, "{}", first.content);
        // Dressing her on the recorded picture anchors her face, and the
        // detector panics there.
        let out = t
            .call(
                json!({"picture": picture_of(&first.content),
                       "scene": {"people": [{"who": "maya", "wearing": "a red scarf"}]}}),
                &c,
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
        std::fs::remove_dir_all(store).ok();
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
            sampling: crate::sampling::Sampling::reference(40),
            size: None,
            seed: 9,
            references: Vec::new(),
            reference_size: EDIT_REFERENCE_SIZE,
            mask: None,
            loras: Vec::new(),
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

    /// A colour and the rectangle painted in it, `(x0, y0, x1, y1)`.
    type Painted = ([u8; 3], (u32, u32, u32, u32));

    /// A region index: black, with each `(rgb, rect)` painted in.
    fn index_png(w: u32, h: u32, painted: &[Painted]) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| {
            painted
                .iter()
                .find(|(_, (x0, y0, x1, y1))| (*x0..*x1).contains(&x) && (*y0..*y1).contains(&y))
                .map_or(image::Rgb([0, 0, 0]), |(c, _)| image::Rgb(*c))
        });
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        png.into_inner()
    }

    fn region(colour: &str, words: &str) -> crate::picture::Region {
        let (name, rgb) = crate::picture::REGION_COLOURS
            .iter()
            .find(|(n, _)| *n == colour)
            .unwrap();
        crate::picture::Region {
            colour: name.to_string(),
            rgb: *rgb,
            words: words.into(),
            reference: None,
        }
    }

    /// The index is read per colour: each named region is masked, and a
    /// pixel in no named colour is not, which `prepare_mask`'s luma
    /// threshold alone would mask. Each region is outlined in its own colour
    /// on the canvas copy, and one with nothing painted is refused.
    #[test]
    fn regions_are_read_by_colour_and_outlined() {
        let original = picture(8, [240, 220, 40]);
        let regions = [
            region("magenta", "make it red"),
            region("blue", "remove it"),
        ];
        let index = index_png(
            64,
            64,
            &[
                ([255, 0, 255], (4, 4, 20, 20)),
                ([0, 64, 255], (40, 40, 60, 60)),
                // A stray grey and a palette colour with no words.
                ([128, 128, 128], (40, 4, 56, 20)),
                ([0, 255, 255], (4, 40, 20, 56)),
            ],
        );
        let (plan, outlined) = prepare_regions(&original, &index, &regions, 1024).unwrap();
        let (w, h) = plan.picture.dimensions();
        // Both regions are in the mask, the blue one included.
        let at = |fx: f32, fy: f32| ((w as f32 * fx) as u32, (h as f32 * fy) as u32);
        let (bx, by) = at(50.0 / 64.0, 50.0 / 64.0);
        assert_eq!(
            plan.soft.get_pixel(bx, by).0[0],
            255,
            "the blue region is masked"
        );
        let (mx, my) = at(12.0 / 64.0, 12.0 / 64.0);
        assert_eq!(plan.soft.get_pixel(mx, my).0[0], 255);
        // Neither the stray grey nor the unnamed cyan is.
        for (fx, fy) in [(48.0, 12.0), (12.0, 48.0)] {
            let (x, y) = at(fx / 64.0, fy / 64.0);
            assert_eq!(
                plan.soft.get_pixel(x, y).0[0],
                0,
                "an unnamed colour is not masked"
            );
        }
        // Outlined in each colour, the inside left as the picture.
        let colours: std::collections::HashSet<[u8; 3]> = outlined.pixels().map(|p| p.0).collect();
        assert!(colours.contains(&[255, 0, 255]) && colours.contains(&[0, 64, 255]));
        assert_eq!(outlined.get_pixel(mx, my), plan.picture.get_pixel(mx, my));
        // A region with nothing painted is refused by name.
        let none = prepare_regions(
            &original,
            &index,
            &[region("magenta", "a"), region("green", "b")],
            1024,
        )
        .unwrap_err();
        assert!(none.contains("The green region is empty"), "{none}");
    }

    /// On a photo larger than the edit canvas, a region a few pixels wide
    /// still gets its outline: sampled nearest, it fell between samples and
    /// went as words with no place (review of #623).
    #[test]
    fn a_thin_region_on_a_large_photo_keeps_its_outline() {
        let mut index = image::RgbImage::new(4032, 3024);
        for y in 1000..2000 {
            for x in 2001..2004 {
                index.put_pixel(x, y, image::Rgb([255, 0, 255]));
            }
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        index.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        // The picture at the index's size, so the canvas is a downscale.
        let big = image::RgbImage::from_pixel(4032, 3024, image::Rgb([240, 220, 40]));
        let mut big_bytes = std::io::Cursor::new(Vec::new());
        big.write_to(&mut big_bytes, image::ImageFormat::Png)
            .unwrap();
        let (plan, outlined) = prepare_regions(
            &big_bytes.into_inner(),
            &bytes.into_inner(),
            &[region("magenta", "a thin scarf")],
            1024,
        )
        .unwrap();
        assert!(plan.picture.width() < 4032, "the canvas is a downscale");
        assert!(
            outlined.pixels().any(|p| p.0 == [255, 0, 255]),
            "the thin region is outlined"
        );
    }

    /// A regions edit whose index cannot be read draws nothing: there are no
    /// owner's words for a whole-picture edit to run on (review of #623).
    #[tokio::test]
    async fn a_regions_edit_without_its_index_draws_nothing() {
        let (url, seen) = distinct(1).await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::write(dir.join("images/orig.png"), picture(8, [240, 220, 40])).unwrap();
        let out = tool(&url)
            .call(
                json!({"picture": "images/orig.png", "mask": "inbox/missing.png",
                       "regions": [{"colour": "magenta", "words": "make it red"}]}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(
            out.content.contains("painted regions could not be read"),
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

    /// A regions edit sends the clean picture as the canvas, the outlined
    /// copy as `<image2>`, the union as the mask, and the legend as the
    /// words; the rest of the picture comes back as it was.
    #[tokio::test]
    async fn a_regions_edit_sends_the_outlines_as_the_second_picture() {
        let red = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 20, 20]));
        let mut result = std::io::Cursor::new(Vec::new());
        red.write_to(&mut result, image::ImageFormat::Png).unwrap();
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
            dir.join("inbox/regions.png"),
            index_png(
                64,
                64,
                &[
                    ([255, 0, 255], (0, 16, 16, 56)),
                    ([0, 255, 255], (40, 16, 56, 56)),
                ],
            ),
        )
        .unwrap();
        let out = tool(&url)
            .call(
                json!({"picture": "images/orig.png", "mask": "inbox/regions.png",
                       "regions": [{"colour": "magenta", "words": "make her top red"},
                                   {"colour": "cyan", "words": "make his shirt green"}]}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let seen = seen.lock().unwrap().clone();
        let uploads = seen
            .iter()
            .filter(|l| l.starts_with("POST /upload/image"))
            .count();
        assert_eq!(uploads, 3, "the picture, its outlined copy, and the mask");
        let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
        assert!(submitted.contains("images.image_2"), "{submitted}");
        assert!(submitted.contains("SetLatentNoiseMask"), "{submitted}");
        assert!(
            submitted.contains("in the magenta outline, make her top red; in the cyan outline, make his shirt green"),
            "{submitted}"
        );
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
        // Between the two regions, untouched.
        let (w, h) = saved.dimensions();
        let canvas = prepare_mask(&original, &mask_png(64, 64, (0, 16, 16, 56)), 1024).unwrap();
        assert_eq!(
            saved.get_pixel(w / 2, h / 20),
            canvas.picture.get_pixel(w / 2, h / 20)
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// A face dropped on a painted region (owner, 2026-10-16): its tight
    /// crop rides as the picture after the outlines, the edit is sampled
    /// unmasked at the canvas's own shape, the prompt says which region
    /// takes the face, and the result is still laid back only inside the
    /// paint. Sampled inside the mask the face reached ArcFace .40–.52;
    /// unmasked and laid back, .79–.85.
    #[tokio::test]
    async fn a_face_dropped_on_a_region_is_sent_and_sampled_unmasked() {
        let red = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 20, 20]));
        let mut result = std::io::Cursor::new(Vec::new());
        red.write_to(&mut result, image::ImageFormat::Png).unwrap();
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
        std::fs::write(dir.join("inbox/face.png"), picture(8, [90, 60, 40])).unwrap();
        std::fs::write(
            dir.join("inbox/regions.png"),
            index_png(
                64,
                64,
                &[
                    ([255, 0, 255], (0, 16, 16, 56)),
                    ([0, 255, 255], (40, 16, 56, 56)),
                ],
            ),
        )
        .unwrap();
        let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [1, 2, 3])));
        let out = tool(&url)
            .with_faces(faces.clone())
            .call(
                json!({"picture": "images/orig.png", "mask": "inbox/regions.png",
                       "regions": [{"colour": "magenta", "words": "her face",
                                    "reference": "inbox/face.png"},
                                   {"colour": "cyan", "words": "make his shirt green"}]}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            faces.asked.load(Ordering::SeqCst),
            1,
            "the dropped picture was cropped"
        );
        assert!(
            out.content
                .contains("The magenta region was given the face from inbox/face.png."),
            "{}",
            out.content
        );
        let seen = seen.lock().unwrap().clone();
        let uploads = seen
            .iter()
            .filter(|l| l.starts_with("POST /upload/image"))
            .count();
        assert_eq!(
            uploads, 3,
            "the picture, its outlined copy, and the face crop"
        );
        let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
        assert!(submitted.contains("images.image_3"), "{submitted}");
        assert!(!submitted.contains("SetLatentNoiseMask"), "{submitted}");
        assert!(submitted.contains("EmptyLatentImage"), "{submitted}");
        assert!(
            submitted.contains(
                "In the magenta outline, take only the facial identity from <image3>, nothing else."
            ),
            "{submitted}"
        );
        // The cyan region has no reference of its own: the canvas is its
        // source, so it stays at full size (review of #706).
        assert!(submitted.contains("\"resolution\":1024"), "{submitted}");
        let png = picture_of(&out.content);
        let saved = image::load_from_memory(&std::fs::read(dir.join(png)).unwrap())
            .unwrap()
            .to_rgb8();
        // Between the two regions, untouched.
        let (w, h) = saved.dimensions();
        let canvas = prepare_mask(&original, &mask_png(64, 64, (0, 16, 16, 56)), 1024).unwrap();
        assert_eq!(
            saved.get_pixel(w / 2, h / 20),
            canvas.picture.get_pixel(w / 2, h / 20)
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// With segmentation installed, a face region's result is laid back
    /// along the face found on the canvas, inside the paint: the rest of the
    /// stroke keeps the original (the hairline, the ears), and a region with
    /// no face keeps its whole paint (S2).
    #[tokio::test]
    async fn a_face_region_follows_the_face_found_rather_than_the_stroke() {
        let red = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 20, 20]));
        let mut result = std::io::Cursor::new(Vec::new());
        red.write_to(&mut result, image::ImageFormat::Png).unwrap();
        let (url, seen) = fake_with(Fake {
            history: vec![done(), done()],
            views: vec![mask_png(64, 64, (2, 24, 10, 40)), result.into_inner()],
            sam: true,
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        let original = picture(8, [240, 220, 40]);
        std::fs::write(dir.join("images/orig.png"), &original).unwrap();
        std::fs::write(dir.join("inbox/face.png"), picture(8, [90, 60, 40])).unwrap();
        std::fs::write(
            dir.join("inbox/regions.png"),
            index_png(
                64,
                64,
                &[
                    ([255, 0, 255], (0, 16, 16, 56)),
                    ([0, 255, 255], (40, 16, 56, 56)),
                ],
            ),
        )
        .unwrap();
        let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [1, 2, 3])));
        let out = tool(&url)
            .with_faces(faces)
            .call(
                json!({"picture": "images/orig.png", "mask": "inbox/regions.png",
                       "regions": [{"colour": "magenta", "words": "her face",
                                    "reference": "inbox/face.png"},
                                   {"colour": "cyan", "words": "make his shirt green"}]}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains(
                "The new face in the magenta region was laid back along the face's own edges"
            ),
            "{}",
            out.content
        );
        let seen = seen.lock().unwrap().clone();
        let prompts: Vec<&String> = seen
            .iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .collect();
        assert_eq!(prompts.len(), 2, "the face, then the render");
        assert!(
            prompts[0].contains("SAM3_Detect") && prompts[0].contains("\"text\":\"face\""),
            "{}",
            prompts[0]
        );
        let saved =
            image::load_from_memory(&std::fs::read(dir.join(picture_of(&out.content))).unwrap())
                .unwrap()
                .to_rgb8();
        let canvas = prepare_mask(&original, &mask_png(64, 64, (0, 16, 16, 56)), 1024).unwrap();
        // The canvas is 1024 square: one cell of the 64 grid is 16 px.
        assert_eq!(saved.get_pixel(96, 512).0, [200, 20, 20], "the face");
        assert_eq!(
            saved.get_pixel(96, 300),
            canvas.picture.get_pixel(96, 300),
            "the stroke above the face is the original"
        );
        assert_eq!(
            saved.get_pixel(768, 640).0,
            [200, 20, 20],
            "the shirt region"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// A face region asking for hair, where no face is found: the hair is
    /// never asked for alone, which would lay the new face into the hair and
    /// say both followed their edges. The paint, said (review of #710).
    #[tokio::test]
    async fn a_face_region_with_no_face_found_never_takes_the_hair_alone() {
        let red = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 20, 20]));
        let mut result = std::io::Cursor::new(Vec::new());
        red.write_to(&mut result, image::ImageFormat::Png).unwrap();
        let (url, seen) = fake_with(Fake {
            history: vec![done(), done()],
            views: vec![mask_png(64, 64, (0, 0, 0, 0)), result.into_inner()],
            sam: true,
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("images/orig.png"), picture(8, [240, 220, 40])).unwrap();
        std::fs::write(dir.join("inbox/face.png"), picture(8, [90, 60, 40])).unwrap();
        std::fs::write(
            dir.join("inbox/regions.png"),
            index_png(64, 64, &[([255, 0, 255], (0, 16, 16, 56))]),
        )
        .unwrap();
        let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [1, 2, 3])));
        let out = tool(&url)
            .with_faces(faces)
            .call(
                json!({"picture": "images/orig.png", "mask": "inbox/regions.png",
                       "regions": [{"colour": "magenta", "words": "her face and hair",
                                    "reference": "inbox/face.png"}]}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let seen = seen.lock().unwrap().clone();
        let sam: Vec<&String> = seen
            .iter()
            .filter(|l| l.starts_with("POST /prompt") && l.contains("SAM3_Detect"))
            .collect();
        assert_eq!(sam.len(), 1, "the face only: {sam:?}");
        assert!(
            out.content.contains(
                "No face was found in the picture, so the new face in the magenta region was \
                 laid back along the paint."
            ),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// A face region whose hair segmentation fails after the face was found
    /// keeps the face's own edge and says the hair is the original's, rather
    /// than throwing the face away (review of #710).
    #[tokio::test]
    async fn a_face_region_whose_hair_fails_keeps_the_face() {
        let red = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 20, 20]));
        let mut result = std::io::Cursor::new(Vec::new());
        red.write_to(&mut result, image::ImageFormat::Png).unwrap();
        let (url, _) = fake_with(Fake {
            history: vec![done(), done(), done()],
            // The hair's answer does not decode: a failed segmentation.
            views: vec![
                mask_png(64, 64, (2, 24, 10, 40)),
                PNG.to_vec(),
                result.into_inner(),
            ],
            sam: true,
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("images/orig.png"), picture(8, [240, 220, 40])).unwrap();
        std::fs::write(dir.join("inbox/face.png"), picture(8, [90, 60, 40])).unwrap();
        std::fs::write(
            dir.join("inbox/regions.png"),
            index_png(64, 64, &[([255, 0, 255], (0, 16, 16, 56))]),
        )
        .unwrap();
        let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [1, 2, 3])));
        let out = tool(&url)
            .with_faces(faces)
            .call(
                json!({"picture": "images/orig.png", "mask": "inbox/regions.png",
                       "regions": [{"colour": "magenta", "words": "her face and hair",
                                    "reference": "inbox/face.png"}]}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("laid back along the face's own edges")
                && out
                    .content
                    .contains("The hair could not be found (the segmentation failed)"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// A face region asking for hair, where the face is found and the hair
    /// is not: the face follows its own edges, and the result says the hair
    /// is the original's, never that both were taken (review of #710).
    #[tokio::test]
    async fn a_face_region_whose_hair_is_not_found_says_so() {
        let red = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 20, 20]));
        let mut result = std::io::Cursor::new(Vec::new());
        red.write_to(&mut result, image::ImageFormat::Png).unwrap();
        let (url, _) = fake_with(Fake {
            history: vec![done(), done(), done()],
            views: vec![
                mask_png(64, 64, (2, 24, 10, 40)),
                mask_png(64, 64, (0, 0, 0, 0)),
                result.into_inner(),
            ],
            sam: true,
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("images/orig.png"), picture(8, [240, 220, 40])).unwrap();
        std::fs::write(dir.join("inbox/face.png"), picture(8, [90, 60, 40])).unwrap();
        std::fs::write(
            dir.join("inbox/regions.png"),
            index_png(64, 64, &[([255, 0, 255], (0, 16, 16, 56))]),
        )
        .unwrap();
        let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [1, 2, 3])));
        let out = tool(&url)
            .with_faces(faces)
            .call(
                json!({"picture": "images/orig.png", "mask": "inbox/regions.png",
                       "regions": [{"colour": "magenta", "words": "her face and hair",
                                    "reference": "inbox/face.png"}]}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("laid back along the face's own edges")
                && out.content.contains("No hair was found")
                && !out.content.contains("face and hair in"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// Without segmentation, a face region is laid back along the paint, as
    /// before S2, and the result says why.
    #[tokio::test]
    async fn a_face_region_without_segmentation_keeps_the_stroke_and_says_so() {
        let red = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 20, 20]));
        let mut result = std::io::Cursor::new(Vec::new());
        red.write_to(&mut result, image::ImageFormat::Png).unwrap();
        let (url, _seen) = fake_with(Fake {
            history: vec![done()],
            views: vec![result.into_inner()],
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("images/orig.png"), picture(8, [240, 220, 40])).unwrap();
        std::fs::write(dir.join("inbox/face.png"), picture(8, [90, 60, 40])).unwrap();
        std::fs::write(
            dir.join("inbox/regions.png"),
            index_png(64, 64, &[([255, 0, 255], (0, 16, 16, 56))]),
        )
        .unwrap();
        let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [1, 2, 3])));
        let out = tool(&url)
            .with_faces(faces)
            .call(
                json!({"picture": "images/orig.png", "mask": "inbox/regions.png",
                       "regions": [{"colour": "magenta", "words": "her face",
                                    "reference": "inbox/face.png"}]}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains(
                "The new face in the magenta region was laid back along the paint: the face's \
                 own edges could not be found (SAM 3.1 segmentation is not installed"
            ),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// A face region whose character's face cannot be had puts no stranger
    /// under the name on a picture whose people are known: the prompt reads
    /// "the viewer". On a photo with no record the name may be someone
    /// already in it, so it stays (G1; review of #706, passes 4 and 5).
    #[tokio::test]
    async fn a_named_face_that_cannot_be_had_is_the_viewer_only_where_names_are_checked() {
        let red = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 20, 20]));
        let mut result = std::io::Cursor::new(Vec::new());
        red.write_to(&mut result, image::ImageFormat::Png).unwrap();
        let (url, seen) = fake_with(Fake {
            history: vec![done(); 3],
            views: vec![result.into_inner(); 3],
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        let store = tempdir();
        let lib = library_with(&["maya", "john"]);
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("images/orig.png"), picture(8, [240, 220, 40])).unwrap();
        std::fs::write(
            dir.join("inbox/regions.png"),
            index_png(64, 64, &[([255, 0, 255], (0, 16, 16, 56))]),
        )
        .unwrap();
        let faces = stub_faces(crate::face::Anchor::Unavailable("not installed".into()));
        let t = tool(&url)
            .with_library_dir(lib.clone())
            .with_faces(faces.clone());
        let last_prompt = || {
            seen.lock()
                .unwrap()
                .iter()
                .rev()
                .find(|l| l.starts_with("POST /prompt"))
                .unwrap()
                .clone()
        };
        let region = |picture: &str| {
            json!({"picture": picture, "mask": "inbox/regions.png",
                   "regions": [{"colour": "magenta", "words": "Maya's face"}]})
        };
        // No record: the name stays.
        let out = t.call(region("images/orig.png"), &ctx(&dir)).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        let sent = last_prompt();
        assert!(sent.contains("Maya"), "{sent}");
        assert!(
            out.content.contains("could not be taken from the library"),
            "{}",
            out.content
        );
        // A recorded picture of John: Maya is not in it, so she is the viewer.
        let c = scene_ctx(&dir, &store, "chat-a");
        let john = t
            .call(
                json!({"scene": {"setting": "a study", "people": [
                    {"who": "john", "wearing": "a coat", "doing": "reading"}]}}),
                &c,
            )
            .await
            .unwrap();
        assert!(!john.is_error, "{}", john.content);
        std::fs::write(
            dir.join("inbox/regions.png"),
            index_png(64, 64, &[([255, 0, 255], (0, 16, 16, 56))]),
        )
        .unwrap();
        let out = t
            .call(region(&picture_of(&john.content)), &c)
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let sent = last_prompt();
        assert!(!sent.contains("Maya"), "{sent}");
        assert!(sent.contains("the viewer"), "{sent}");
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A picture dropped on a region whose words are not about a face is a
    /// look, whoever is in it: the detector is not asked and the prompt takes
    /// the look (review of #706, pass 3: a dress worn by a model lost the
    /// dress to her face).
    #[tokio::test]
    async fn a_dropped_picture_is_a_look_unless_the_words_are_about_a_face() {
        let red = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 20, 20]));
        let mut result = std::io::Cursor::new(Vec::new());
        red.write_to(&mut result, image::ImageFormat::Png).unwrap();
        let (url, seen) = fake_with(Fake {
            history: vec![done()],
            views: vec![result.into_inner()],
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("images/orig.png"), picture(8, [240, 220, 40])).unwrap();
        std::fs::write(dir.join("inbox/dress.png"), picture(8, [150, 20, 30])).unwrap();
        std::fs::write(
            dir.join("inbox/regions.png"),
            index_png(64, 64, &[([255, 0, 255], (0, 16, 16, 56))]),
        )
        .unwrap();
        let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [1, 2, 3])));
        let out = tool(&url)
            .with_faces(faces.clone())
            .call(
                json!({"picture": "images/orig.png", "mask": "inbox/regions.png",
                       "regions": [{"colour": "magenta", "words": "a red dress like this",
                                    "reference": "inbox/dress.png"}]}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(faces.asked.load(Ordering::SeqCst), 0, "no face looked for");
        let seen = seen.lock().unwrap().clone();
        let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
        assert!(
            submitted.contains("In the magenta outline, take the look from <image3>."),
            "{submitted}"
        );
        assert!(!submitted.contains("facial identity"), "{submitted}");
        assert!(
            out.content.contains("given the look of inbox/dress.png"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("No face"), "{}", out.content);
        std::fs::remove_dir_all(dir).ok();
    }

    /// A library name in a region's words takes that character's face only
    /// when the words are about a face, and the character is recorded as
    /// used; with every painted region holding a face, the references go
    /// small (review of #706).
    #[tokio::test]
    async fn a_name_in_a_region_takes_a_face_only_when_the_words_are_about_one() {
        for (words, wants_face) in [("Maya's shirt in red", false), ("Maya's face", true)] {
            let red = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 20, 20]));
            let mut result = std::io::Cursor::new(Vec::new());
            red.write_to(&mut result, image::ImageFormat::Png).unwrap();
            let (url, seen) = fake_with(Fake {
                history: vec![done()],
                views: vec![result.into_inner()],
                ..Fake::default()
            })
            .await;
            let dir = tempdir();
            let lib = library_with(&["maya"]);
            std::fs::create_dir_all(dir.join("images")).unwrap();
            std::fs::create_dir_all(dir.join("inbox")).unwrap();
            std::fs::write(dir.join("images/orig.png"), picture(8, [240, 220, 40])).unwrap();
            std::fs::write(
                dir.join("inbox/regions.png"),
                index_png(64, 64, &[([255, 0, 255], (0, 16, 16, 56))]),
            )
            .unwrap();
            let faces = stub_faces(crate::face::Anchor::Crop(picture(4, [1, 2, 3])));
            let out = tool(&url)
                .with_library_dir(lib.clone())
                .with_faces(faces.clone())
                .call(
                    json!({"picture": "images/orig.png", "mask": "inbox/regions.png",
                           "regions": [{"colour": "magenta", "words": words}]}),
                    &ctx(&dir),
                )
                .await
                .unwrap();
            assert!(!out.is_error, "{words}: {}", out.content);
            let asked = faces.asked.load(Ordering::SeqCst);
            let seen = seen.lock().unwrap().clone();
            let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
            if wants_face {
                assert_eq!(asked, 1, "{words}");
                assert!(
                    submitted.contains("facial identity from <image3>"),
                    "{submitted}"
                );
                assert!(submitted.contains("\"resolution\":512"), "{submitted}");
                assert!(
                    out.content.contains("given the face from Maya's face"),
                    "{}",
                    out.content
                );
                let m = manifest_of(&dir, &out.content);
                assert_eq!(m["cast"][0]["name"], "maya", "{m}");
            } else {
                assert_eq!(asked, 0, "{words}: a shirt is not a face");
                assert!(!submitted.contains("facial identity"), "{submitted}");
                assert!(submitted.contains("SetLatentNoiseMask"), "{submitted}");
            }
            std::fs::remove_dir_all(dir).ok();
            std::fs::remove_dir_all(lib).ok();
        }
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

    /// A masked edit whose mask is named in words, on a server that can
    /// segment: what SAM answers for each case is `views[0]`.
    async fn edit_with_mask_words(
        sam: bool,
        found: Vec<u8>,
    ) -> (ToolOutput, Vec<String>, std::path::PathBuf, Vec<u8>) {
        let green = image::RgbImage::from_pixel(64, 64, image::Rgb([10, 200, 30]));
        let mut result = std::io::Cursor::new(Vec::new());
        green
            .write_to(&mut result, image::ImageFormat::Png)
            .unwrap();
        let (url, seen) = fake_with(Fake {
            history: vec![done(), done()],
            views: vec![found, result.into_inner()],
            sam,
            ..Fake::default()
        })
        .await;
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        let original = picture(8, [240, 220, 40]);
        std::fs::write(dir.join("images/orig.png"), &original).unwrap();
        let out = tool(&url)
            .call(
                json!({"picture": "images/orig.png", "retouch": "Make the sky orange.",
                       "mask": "the sky"}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        let seen = seen.lock().unwrap().clone();
        (out, seen, dir, original)
    }

    fn prompts(seen: &[String]) -> Vec<&String> {
        seen.iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .collect()
    }

    #[tokio::test]
    async fn an_area_named_in_words_is_found_and_only_it_is_redrawn() {
        let mask = mask_png(64, 64, (0, 16, 16, 56));
        let (out, seen, dir, original) = edit_with_mask_words(true, mask.clone()).await;
        assert!(!out.is_error, "{}", out.content);
        let prompts = prompts(&seen);
        assert_eq!(prompts.len(), 2, "the segmentation, then the render");
        assert!(
            prompts[0].contains("SAM3_Detect") && prompts[0].contains("the sky"),
            "{}",
            prompts[0]
        );
        assert!(!prompts[0].contains("KSampler"), "{}", prompts[0]);
        assert!(prompts[1].contains("SetLatentNoiseMask"), "{}", prompts[1]);
        assert!(
            out.content
                .contains("Only what was found for 'the sky' (1 area) was redrawn"),
            "{}",
            out.content
        );
        // Outside the area found, the original; inside, the render.
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
        let canvas = prepare_mask(&original, &mask, EDIT_REFERENCE_SIZE).unwrap();
        assert_eq!(
            saved.get_pixel(800, 576),
            canvas.picture.get_pixel(800, 576)
        );
        assert_eq!(saved.get_pixel(128, 576).0, [10, 200, 30]);
        let m = manifest_of(&dir, &out.content);
        assert_eq!(m["mask_words"], "the sky", "{m}");
        assert!(m["mask"].is_null(), "{m}");
        std::fs::remove_dir_all(dir).ok();
    }

    fn square(w: u32, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> image::GrayImage {
        image::GrayImage::from_fn(w, w, |x, y| {
            image::Luma([if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
                255
            } else {
                0
            }])
        })
    }

    #[test]
    fn a_face_region_is_laid_back_along_the_face_found_inside_its_paint() {
        let soft = square(100, (20, 20, 96, 96));
        let face = square(100, (40, 40, 60, 60));
        let out = edge_faces(&soft, None, &face).unwrap();
        assert_eq!(out.get_pixel(50, 50).0[0], 255, "the face is redrawn");
        assert_eq!(out.get_pixel(25, 25).0[0], 0, "the paint around it is not");
        assert_eq!(out.get_pixel(5, 5).0[0], 0, "nor anything past the paint");
        assert!(out.get_pixel(62, 50).0[0] > 0, "the face's edge, grown");
        // As the job builds it: the composite's soft mask is the union of
        // every region's paint, grown and feathered, and the no-face region
        // is part of that union. The result is never past it, at any pixel:
        // not a restatement of the formula, since `kept` and `soft` are
        // built separately here, as there.
        let face_paint = square(100, (20, 20, 70, 70));
        let others = square(100, (82, 82, 92, 92));
        let union = image::GrayImage::from_fn(100, 100, |x, y| {
            image::Luma([face_paint.get_pixel(x, y).0[0].max(others.get_pixel(x, y).0[0])])
        });
        let soft = feather_mask(&grow_mask(&union), &union);
        let out = edge_faces(&soft, Some(&others), &face).unwrap();
        assert_eq!(
            out.get_pixel(87, 87).0[0],
            255,
            "the no-face region keeps its paint"
        );
        assert_eq!(
            out.get_pixel(25, 25).0[0],
            0,
            "the face region's paint past the face"
        );
        for (x, y, o) in out.enumerate_pixels() {
            assert!(
                o.0[0] <= soft.get_pixel(x, y).0[0],
                "({x}, {y}) is past the paint"
            );
        }
    }

    #[test]
    fn a_face_outside_the_face_region_keeps_the_paint() {
        let soft = square(100, (20, 20, 80, 80));
        // A face elsewhere in the picture, and one at the paint's edge only.
        for face in [square(100, (0, 0, 8, 8)), square(100, (85, 20, 100, 80))] {
            assert!(edge_faces(&soft, None, &face).is_none());
        }
        assert!(about_hair("her face and hair") && !about_hair("her face, chair behind"));
    }

    #[test]
    fn found_areas_counts_separate_parts_and_leaves_out_specks() {
        let mut m = image::GrayImage::new(100, 100);
        assert_eq!(found_areas(&m), 0);
        for (x, y) in (10..30).flat_map(|x| (10..30).map(move |y| (x, y))) {
            m.put_pixel(x, y, image::Luma([255]));
        }
        assert_eq!(found_areas(&m), 1);
        // A second part, touching the first only at a corner: separate.
        for (x, y) in (30..50).flat_map(|x| (30..50).map(move |y| (x, y))) {
            m.put_pixel(x, y, image::Luma([255]));
        }
        assert_eq!(found_areas(&m), 2);
        // A speck of ten pixels, a thousandth of the picture: not an area.
        for x in 80..90 {
            m.put_pixel(x, 90, image::Luma([255]));
        }
        assert_eq!(found_areas(&m), 2);
    }

    /// Admission charges SAM only where the server can segment, and keeps
    /// a yes for the process (review of #710, pass 2).
    #[tokio::test]
    async fn segmentation_is_charged_only_where_the_server_can_segment() {
        let (url, seen) = fake_with(Fake {
            sam: true,
            ..Fake::default()
        })
        .await;
        let t = tool(&url);
        assert!(t.backend.segments().await);
        assert!(t.backend.segments().await);
        let asks = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("/object_info/SAM3_Detect"))
            .count();
        assert_eq!(asks, 1, "a yes is kept");
        let (url, _) = fake_with(Fake::default()).await;
        assert!(!tool(&url).backend.segments().await);
    }

    /// What SAM found but `prepare_mask` refuses: a speck is too small to
    /// edit alone; a mask of another shape could not be used. Both draw the
    /// whole picture and say which.
    #[tokio::test]
    async fn a_found_mask_too_small_or_unusable_redraws_the_whole_picture() {
        // One white pixel at 4096 square averages under the canvas's floor.
        let (out, seen, dir, _) =
            edit_with_mask_words(true, mask_png(4096, 4096, (0, 0, 1, 1))).await;
        assert!(
            out.content
                .contains("What matched 'the sky' was too small to edit alone"),
            "{}",
            out.content
        );
        assert!(!prompts(&seen)[1].contains("SetLatentNoiseMask"));
        std::fs::remove_dir_all(dir).ok();
        let (out, seen, dir, _) = edit_with_mask_words(true, mask_png(64, 16, (0, 0, 32, 8))).await;
        assert!(
            out.content
                .contains("What matched 'the sky' could not be used as a mask"),
            "{}",
            out.content
        );
        assert!(!prompts(&seen)[1].contains("SetLatentNoiseMask"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn an_area_named_in_words_without_segmentation_redraws_the_whole_picture() {
        let (out, seen, dir, _) = edit_with_mask_words(false, PNG.to_vec()).await;
        assert!(!out.is_error, "{}", out.content);
        let prompts = prompts(&seen);
        assert_eq!(prompts.len(), 1, "{seen:?}");
        assert!(!prompts[0].contains("SAM3_Detect"), "{}", prompts[0]);
        assert!(!prompts[0].contains("SetLatentNoiseMask"), "{}", prompts[0]);
        assert!(
            out.content.contains(
                "The area 'the sky' could not be found (SAM 3.1 segmentation is not installed"
            ) && out.content.contains("the whole picture"),
            "{}",
            out.content
        );
        assert!(manifest_of(&dir, &out.content)["mask_words"].is_null());
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn an_area_named_in_words_that_matches_nothing_redraws_the_whole_picture() {
        let (out, seen, dir, _) = edit_with_mask_words(true, mask_png(64, 64, (0, 0, 0, 0))).await;
        assert!(!out.is_error, "{}", out.content);
        let prompts = prompts(&seen);
        assert_eq!(prompts.len(), 2, "{seen:?}");
        assert!(!prompts[1].contains("SetLatentNoiseMask"), "{}", prompts[1]);
        assert!(
            out.content
                .contains("Nothing in the picture matched 'the sky'"),
            "{}",
            out.content
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

        fn crop(&self, _: &[u8]) -> crate::face::Anchor {
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
            voice: None,
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

    /// A cutout as the server returns one: RGBA, a figure on transparency,
    /// or nothing at all.
    fn cutout_png(figure: bool) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(64, 64, |x, y| {
            if figure && (24..40).contains(&x) && (8..60).contains(&y) {
                image::Rgba([180, 60, 60, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        });
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        png.into_inner()
    }

    /// The tests' context has no splitter, so the call gives the places a
    /// split would have.
    fn touching_call() -> Value {
        json!({"scene": {"setting": "a sunlit kitchen with a long oak table",
            "people": [{"who": "maya", "wearing": "a yellow raincoat", "where": "left"},
                       {"who": "john", "wearing": "a flannel shirt", "where": "right"}],
            "together": "Maya lifts John off the ground"}})
    }

    /// A layered picture never claims `clothes_in_act`: its passes carry the
    /// outfit on each cutout, and the one-pass prompt the claim is about was
    /// never sent (review of #647). Fails where the manifest took it from
    /// the compiled prompt whichever route drew.
    #[tokio::test]
    async fn a_layered_picture_never_claims_its_clothes_were_left_out() {
        let (url, _) = fake_with(Fake {
            history: vec![done(); 6],
            views: vec![
                picture(8, [90, 90, 90]),
                cutout_png(true),
                cutout_png(true),
                picture(20, [200, 40, 40]),
                picture(22, [210, 50, 50]),
                picture(24, [60, 160, 60]),
            ],
            ..Fake::default()
        })
        .await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        // Her outfit left to the default, and a shared act that undresses
        // her: the one-pass prompt would leave it out.
        let mut call = touching_call();
        call["scene"]["people"][0]
            .as_object_mut()
            .unwrap()
            .remove("wearing");
        call["scene"]["together"] = json!("Maya, wearing only a towel, lifts John off the ground");
        let out = t.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        let manifest = manifest_of(&dir, &out.content);
        assert_eq!(manifest["route"], "layered", "{}", out.content);
        assert!(manifest["clothes_in_act"].is_null(), "{manifest}");
        // The same call in one pass does claim it, so the null above is the
        // gate's and not a rule that never fired.
        cx.layers = false;
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            manifest_of(&dir, &out.content)["clothes_in_act"],
            json!(["Maya"])
        );
    }

    /// Someone added to a picture the owner builds on wears what this chat
    /// last drew them in (mecha-a3's G1b), from the chat's copy, since the
    /// picture built on does not hold them; a new picture asks nothing of
    /// it. Fails where `worn` read only the record built on (review of
    /// #753, pass 3: it could never fire).
    #[tokio::test]
    async fn a_newcomer_to_a_picture_built_on_wears_what_the_chat_last_drew() {
        let (url, seen) = distinct(6).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let draw = |v: Value| {
            let (t, cx) = (&t, &cx);
            async move {
                let out = t.call(v, cx).await.unwrap();
                assert!(!out.is_error, "{}", out.content);
                out
            }
        };
        let first = draw(json!({"scene": {"setting": "a quiet reading room",
            "people": [{"who": "maya", "wearing": "a green coat", "doing": "reading a book"}]}}))
        .await;
        let p1 = picture_of(&first.content);
        draw(json!({"scene": {"setting": "a bakery at dawn",
            "people": [{"who": "john", "wearing": "a striped baker's apron", "doing": "kneading dough"}]}}))
        .await;
        // A new picture of John alone takes nothing from the bakery.
        draw(json!({"scene": {"setting": "a harbour wall",
            "people": [{"who": "john", "doing": "watching the boats"}]}}))
        .await;
        assert!(
            !last_prompt(&seen).contains("striped baker's apron"),
            "{}",
            last_prompt(&seen)
        );
        // Built on the first picture, John is a newcomer, and wears what the
        // chat last drew him in: the harbour picture, which gave no clothes,
        // so the default. Draw him once more in the apron, then add him.
        draw(json!({"scene": {"setting": "a bakery at dawn",
            "people": [{"who": "john", "wearing": "a striped baker's apron", "doing": "kneading dough"}]}}))
        .await;
        let added = draw(json!({"picture": p1, "scene": {"people": [
            {"who": "john", "where": "right", "doing": "handing Maya a cup of tea"}]}}))
        .await;
        assert!(
            added
                .content
                .contains("John is wearing what this chat last drew them in."),
            "{}",
            added.content
        );
        assert!(
            last_prompt(&seen).contains("striped baker's apron"),
            "{}",
            last_prompt(&seen)
        );
    }

    /// A layered new picture takes no outfit from the chat's last picture:
    /// the cutouts' clothes are the call's, or the record a call builds on,
    /// and a new picture builds on none (the owner, 2026-10-27). Fails where
    /// the layered build read the chat's copy (`recorded`).
    #[tokio::test]
    async fn a_layered_new_picture_takes_no_outfit_from_the_chats_last() {
        let (url, seen) = fake_with(Fake {
            history: vec![done(); 7],
            views: vec![
                picture(6, [30, 30, 30]),
                picture(8, [90, 90, 90]),
                cutout_png(true),
                cutout_png(true),
                picture(20, [200, 40, 40]),
                picture(22, [210, 50, 50]),
                picture(24, [60, 160, 60]),
            ],
            ..Fake::default()
        })
        .await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        let first = t
            .call(
                json!({"scene": {"setting": "a quiet reading room",
                       "people": [{"who": "maya", "wearing": "a long green wool coat with brass buttons",
                                   "doing": "reading a book"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!first.is_error, "{}", first.content);
        let coat = |seen: &Arc<Mutex<Vec<String>>>| {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|l| l.contains("green wool coat"))
                .count()
        };
        let before = coat(&seen);
        assert!(before > 0, "the first picture's prompt names the coat");

        cx.layers = true;
        let mut call = touching_call();
        call["scene"]["people"][0]
            .as_object_mut()
            .unwrap()
            .remove("wearing");
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            manifest_of(&dir, &out.content)["route"],
            "layered",
            "{}",
            out.content
        );
        assert_eq!(
            coat(&seen),
            before,
            "a layered pass wore the last picture's coat"
        );
    }

    /// `[image] timeout_secs` bounds the whole layered build: a pass that
    /// would start past it is never sent, rather than each of n + 3 passes
    /// having the full time (review of #624).
    #[tokio::test]
    async fn a_layered_build_past_its_time_sends_no_more_passes() {
        let (url, seen) = fake_with(Fake {
            history: vec![done(); 2],
            views: vec![picture(20, [200, 40, 40])],
            ..Fake::default()
        })
        .await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = ImageGenerate::new(ImageConfig {
            url: url.clone(),
            min_available_mb: 0,
            unload_after_secs: 0,
            timeout_secs: 0,
            ..Default::default()
        })
        .unwrap()
        .polling_every(Duration::from_millis(10))
        .with_seeds()
        .with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        // With no time at all the single pass it falls back to times out too.
        let out = t.call(touching_call(), &cx).await.unwrap();
        assert!(out.content.contains("longer than 0 s"), "{}", out.content);
        let plates = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("POST /prompt") && l.contains("No people."))
            .count();
        assert_eq!(plates, 0, "no pass is sent with no time left");
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A layered picture's `layers` entry describes that picture only: an
    /// edit of it lands with none, or a later re-place would be built on the
    /// previous picture's plate and cutouts (review of #624).
    #[tokio::test]
    async fn an_edit_of_a_layered_picture_lands_without_its_layers() {
        let (url, _seen) = fake_with(Fake {
            history: vec![done(); 6],
            views: vec![
                picture(8, [90, 90, 90]),
                cutout_png(true),
                cutout_png(true),
                picture(20, [200, 40, 40]),
                picture(22, [210, 50, 50]),
                picture(24, [30, 60, 200]),
            ],
            ..Fake::default()
        })
        .await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let first = t.call(touching_call(), &cx).await.unwrap();
        assert!(
            first.content.contains("built in layers"),
            "{}",
            first.content
        );
        let slot = cx.scene.as_ref().unwrap();
        let layered = picture_of(&first.content);
        assert!(slot
            .lookup(&std::fs::read(dir.join(&layered)).unwrap())
            .unwrap()
            .layers
            .is_some());
        let edit = t
            .call(json!({"picture": layered, "retouch": "a red scarf"}), &cx)
            .await
            .unwrap();
        assert!(!edit.is_error, "{}", edit.content);
        let landed = slot
            .lookup(&std::fs::read(dir.join(picture_of(&edit.content))).unwrap())
            .unwrap();
        assert!(landed.layers.is_none(), "{:?}", landed.layers);
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// Two people given the same side, neither acting on the other, would
    /// share a place tag in the placing prompt: drawn in one pass, and said
    /// (review of #624).
    #[tokio::test]
    async fn a_shared_place_is_one_render_and_says_why() {
        let (url, seen) = distinct(1).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let mut call = touching_call();
        call["scene"]["people"][1]["where"] = json!("left");
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("two people would share one place"),
            "{}",
            out.content
        );
        let prompts = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .count();
        assert_eq!(prompts, 1);
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A split that fell back leaves only the call's places, and a placing
    /// pass with no places dropped a person 2 of 2 runs (mecha-a3): with
    /// nobody placed, the switch draws one pass and says why (review of
    /// #624).
    #[tokio::test]
    async fn an_unplaced_touching_scene_is_one_render_and_says_why() {
        let (url, seen) = distinct(1).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let mut call = touching_call();
        for p in call["scene"]["people"].as_array_mut().unwrap() {
            p.as_object_mut().unwrap().remove("where");
        }
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("not everyone has a place"),
            "{}",
            out.content
        );
        let prompts = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .count();
        assert_eq!(prompts, 1);
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// The persona's switch builds a touching scene in layers (IMAGE-DESIGN.md
    /// §15): a plate, a cutout per person from their portrait, the placing
    /// pass with the plate first and names as tags, and the finish; the
    /// plate and cutouts kept, the record carrying the build, and the result
    /// and manifest saying so. Fails on a tool with no layers, which draws
    /// one render.
    #[tokio::test]
    async fn a_touching_scene_is_built_in_layers_when_the_switch_is_on() {
        let (url, seen) = fake_with(Fake {
            history: vec![done(); 5],
            views: vec![
                picture(8, [90, 90, 90]),
                cutout_png(true),
                cutout_png(true),
                picture(20, [200, 40, 40]),
                picture(22, [210, 50, 50]),
            ],
            ..Fake::default()
        })
        .await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let out = t.call(touching_call(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("built in layers"), "{}", out.content);
        let seen = seen.lock().unwrap().clone();
        let prompts: Vec<&String> = seen
            .iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .collect();
        assert_eq!(prompts.len(), 5, "plate, two cutouts, placing, finish");
        assert!(prompts[0].contains("No people."), "{}", prompts[0]);
        assert!(
            prompts[1].contains("RGBA image with transparency"),
            "{}",
            prompts[1]
        );
        assert!(
            prompts[3].contains("Place the person from <image2>"),
            "{}",
            prompts[3]
        );
        assert!(
            !prompts[3].contains("Maya") && !prompts[3].contains("John"),
            "{}",
            prompts[3]
        );
        assert!(prompts[4].contains("85 mm"), "{}", prompts[4]);
        let manifest = manifest_of(&dir, &out.content);
        assert_eq!(manifest["route"], "layered");
        let renders = manifest["layers"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s.get("seed").is_some())
            .count();
        assert_eq!(renders, 5);
        let kept: Vec<String> = std::fs::read_dir(dir.join("images"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(kept.iter().any(|n| n.ends_with("-plate.png")), "{kept:?}");
        assert!(
            kept.iter().any(|n| n.ends_with("-cutout-maya.png")),
            "{kept:?}"
        );
        let landed = cx
            .scene
            .as_ref()
            .unwrap()
            .lookup(&std::fs::read(dir.join(picture_of(&out.content))).unwrap())
            .unwrap();
        let layers = landed.layers.expect("the build is recorded");
        assert_eq!(layers.people.len(), 2);
        assert_eq!(layers.people[0].who, "maya");
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A layered build draws what the single pass would: each person's
    /// expression rides with their part, the plate keeps only the setting's
    /// clauses that name nobody, and the prompt log's `words` hold every
    /// chat word its passes were built from, clothes included (review of
    /// #626).
    #[tokio::test]
    async fn a_layered_build_keeps_expressions_and_logs_every_chat_word() {
        let (url, seen) = fake_with(Fake {
            history: vec![done(); 5],
            views: vec![
                picture(8, [90, 90, 90]),
                cutout_png(true),
                cutout_png(true),
                picture(20, [200, 40, 40]),
                picture(22, [210, 50, 50]),
            ],
            ..Fake::default()
        })
        .await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let log = store.join("sessions/chat-a.prompts.log");
        cx.prompt_log = Some(log.clone());
        let mut call = touching_call();
        call["scene"]["setting"] =
            json!("a sunlit kitchen with a long oak table, Maya's scarf over a chair");
        call["scene"]["people"][1]["expression"] = json!("grinning");
        call["scene"]["light"] = json!("lamplight falling on John's shoulders");
        call["scene"]["people"][0]["wearing"] = json!("John's old flannel shirt");
        let out = t.call(call, &cx).await.unwrap();
        assert!(out.content.contains("built in layers"), "{}", out.content);
        let seen = seen.lock().unwrap().clone();
        let prompts: Vec<&String> = seen
            .iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .collect();
        assert!(prompts[0].contains("a sunlit kitchen"), "{}", prompts[0]);
        assert!(
            !prompts[0].contains("scarf") && !prompts[0].contains("Maya"),
            "{}",
            prompts[0]
        );
        assert!(prompts[3].contains("grinning"), "{}", prompts[3]);
        // A cutout is lit by light that names nobody, as the plate is, and
        // wears clothes said by no one's name.
        for cutout in &prompts[1..3] {
            assert!(cutout.contains("soft, even light"), "{cutout}");
            assert!(!cutout.contains("lamplight"), "{cutout}");
            assert!(!cutout.contains("John"), "{cutout}");
        }
        assert!(
            prompts[1].contains("the person's old flannel shirt"),
            "{}",
            prompts[1]
        );
        // One line per pass sent, and none for the single pass never sent.
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 5, "{text}");
        assert!(lines.iter().all(|l| l["route"] == "layered"), "{text}");
        for line in &lines {
            let words: Vec<&str> = line["words"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect();
            for want in ["John's old flannel shirt", "a flannel shirt", "grinning"] {
                assert!(words.contains(&want), "{want}: {words:?}");
            }
            assert!(!words.iter().any(|w| w.contains("<image")), "{words:?}");
        }
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// Someone named but not in the picture reaches every layered pass as
    /// "the viewer", and the plate as nothing at all, as the single pass
    /// reads them (the owner's ruling, 2026-10-08; review of #626).
    #[tokio::test]
    async fn a_layered_build_reads_someone_offstage_as_the_viewer() {
        let (url, seen) = fake_with(Fake {
            history: vec![done(); 5],
            views: vec![
                picture(8, [90, 90, 90]),
                cutout_png(true),
                cutout_png(true),
                picture(20, [200, 40, 40]),
                picture(22, [210, 50, 50]),
            ],
            ..Fake::default()
        })
        .await;
        let (dir, store, lib) = (
            tempdir(),
            tempdir(),
            library_with(&["maya", "john", "wren"]),
        );
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let mut call = touching_call();
        call["scene"]["setting"] =
            json!("a sunlit kitchen with a long oak table, Wren's bicycle by the door");
        call["scene"]["together"] = json!("Maya lifts John off the ground while Wren laughs");
        let out = t.call(call, &cx).await.unwrap();
        assert!(out.content.contains("built in layers"), "{}", out.content);
        let seen = seen.lock().unwrap().clone();
        let prompts: Vec<&String> = seen
            .iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .collect();
        assert_eq!(prompts.len(), 5);
        assert!(!prompts.iter().any(|p| p.contains("Wren")), "{prompts:?}");
        assert!(!prompts[0].contains("bicycle"), "{}", prompts[0]);
        assert!(prompts[3].contains("the viewer laughs"), "{}", prompts[3]);
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// Words the picture renders are drawn by the single pass only, so a
    /// scene with them is one render and says why, rather than a layered
    /// picture that quietly leaves them out (review of #626).
    #[tokio::test]
    async fn a_scene_with_words_to_render_is_one_render_and_says_why() {
        let (url, seen) = distinct(1).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let mut call = touching_call();
        call["scene"]["text"] = json!([{"words": "OPEN LATE"}]);
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("words to render"), "{}", out.content);
        let posts = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .count();
        assert_eq!(posts, 1);
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A build that fails part-way still says the server copies its
    /// finished passes left, beside the single pass it falls back to
    /// (review of #626).
    #[tokio::test]
    async fn a_failed_build_says_the_copies_its_passes_left() {
        let dir = tempdir();
        let temp = dir.join("server-temp");
        std::fs::create_dir_all(&temp).unwrap();
        for f in ["cut_00001_.png", "one_00001_.png"] {
            std::fs::write(temp.join(f), "x").unwrap();
        }
        let (url, _seen) = fake_with(Fake {
            history: vec![
                done_as("plate_00001_.png"),
                done_as("cut_00001_.png"),
                done_as("one_00001_.png"),
            ],
            views: vec![
                picture(8, [90, 90, 90]),
                cutout_png(false),
                picture(20, [200, 40, 40]),
            ],
            temp: Some(temp.clone()),
            ..Fake::default()
        })
        .await;
        let (store, lib) = (tempdir(), library_with(&["maya", "john"]));
        let t = tool_in(&url, &temp).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let out = t.call(touching_call(), &cx).await.unwrap();
        assert!(
            out.content.contains("Layers failed at cutout 1 of 2"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("could not all be removed")
                && out.content.contains("plate_00001_.png"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A setting every clause of which names someone leaves the plate no
    /// room in words, so the picture is one render and says why (review of
    /// #626).
    #[tokio::test]
    async fn a_setting_that_only_names_people_is_one_render_and_says_why() {
        let (url, seen) = distinct(1).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let mut call = touching_call();
        call["scene"]["setting"] = json!("Maya's flat");
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content
                .contains("every word of the setting names someone"),
            "{}",
            out.content
        );
        let posts = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .count();
        assert_eq!(posts, 1);
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A scene with a library style is layered too, the style's words on
    /// the finish only (mecha-a3: 11 of 15 real touching calls named one,
    /// and a portrait-recipe style on the plate drew a person there); a
    /// replay's posed arm puts each part into its cutout.
    #[tokio::test]
    async fn a_styled_scene_is_layered_and_the_posed_arm_poses_cutouts() {
        let (url, seen) = fake_with(Fake {
            history: vec![done(); 5],
            views: vec![
                picture(8, [90, 90, 90]),
                cutout_png(true),
                cutout_png(true),
                picture(20, [200, 40, 40]),
                picture(22, [210, 50, 50]),
            ],
            ..Fake::default()
        })
        .await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        crate::imagelib::create(
            &lib,
            crate::imagelib::NewEntry {
                kind: crate::imagelib::Kind::Style,
                name: "inkwash".into(),
                text: "loose ink and wash, soft paper grain".into(),
                portrait: None,
                source_seed: None,
                origin: crate::imagelib::Origin::Owner,
                locked: false,
            },
        )
        .unwrap();
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        cx.layers_posed = true;
        let mut call = touching_call();
        call["scene"]["style"] = json!("inkwash");
        call["scene"]["people"][0]["doing"] = json!("lifting John off the ground");
        let out = t.call(call, &cx).await.unwrap();
        assert!(out.content.contains("built in layers"), "{}", out.content);
        let seen = seen.lock().unwrap().clone();
        let prompts: Vec<&String> = seen
            .iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .collect();
        assert!(
            !prompts[0].contains("ink and wash"),
            "never the plate: {}",
            prompts[0]
        );
        assert!(
            prompts[4].contains("loose ink and wash"),
            "the finish: {}",
            prompts[4]
        );
        assert!(
            !prompts[1].contains("ink and wash"),
            "never a cutout: {}",
            prompts[1]
        );
        assert!(
            !prompts[3].contains("ink and wash"),
            "never the placing: {}",
            prompts[3]
        );
        assert!(
            prompts[1].contains("lifting"),
            "the posed cutout: {}",
            prompts[1]
        );
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A plate check whose detector panics costs the check, never the job:
    /// the build goes on and the manifest says the plate went unchecked
    /// (review of #626).
    #[tokio::test]
    async fn a_panicking_plate_check_costs_the_check_not_the_build() {
        struct PanicsOnPlates;
        impl crate::face::FaceAnchors for PanicsOnPlates {
            fn anchor(
                &self,
                _library: &crate::imagelib::Library,
                _entry: &crate::imagelib::Entry,
            ) -> crate::face::Anchor {
                crate::face::Anchor::NoFace
            }
            fn faces_in(&self, _png: &[u8]) -> std::result::Result<usize, String> {
                panic!("detector down")
            }
        }
        let (url, _seen) = fake_with(Fake {
            history: vec![done(); 5],
            views: vec![
                picture(8, [90, 90, 90]),
                cutout_png(true),
                cutout_png(true),
                picture(20, [200, 40, 40]),
                picture(22, [210, 50, 50]),
            ],
            ..Fake::default()
        })
        .await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url)
            .with_library_dir(lib.clone())
            .with_faces(Arc::new(PanicsOnPlates));
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let out = t.call(touching_call(), &cx).await.unwrap();
        assert!(out.content.contains("built in layers"), "{}", out.content);
        let manifest = manifest_of(&dir, &out.content);
        let check = manifest["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["step"] == "plate check")
            .cloned()
            .unwrap();
        assert_eq!(check["unchecked"], "the face detector panicked");
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// Reports a face in every picture: the plate is never empty.
    struct SeesFaces;

    impl crate::face::FaceAnchors for SeesFaces {
        fn anchor(
            &self,
            _library: &crate::imagelib::Library,
            _entry: &crate::imagelib::Entry,
        ) -> crate::face::Anchor {
            crate::face::Anchor::NoFace
        }
        fn faces_in(&self, _png: &[u8]) -> std::result::Result<usize, String> {
            Ok(1)
        }
    }

    /// The plate must come back empty: a face in it is drawn once more at a
    /// new seed, and twice is the single pass, said (mecha-a3's blocker: a
    /// stranger in the plate was kept and one person never placed). The
    /// plate prompt names nobody, whatever the camera said.
    #[tokio::test]
    async fn a_plate_with_someone_in_it_is_redrawn_then_given_up() {
        let (url, seen) = fake_with(Fake {
            history: vec![done(); 3],
            views: vec![
                picture(8, [90, 90, 90]),
                picture(9, [91, 90, 90]),
                picture(20, [200, 40, 40]),
            ],
            ..Fake::default()
        })
        .await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url)
            .with_library_dir(lib.clone())
            .with_faces(Arc::new(SeesFaces));
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let mut call = touching_call();
        call["scene"]["camera"] = json!("Medium shot, low angle, lingering on Maya's scarf");
        call["scene"]["light"] = json!("a lamp warming his collar");
        let out = t.call(call, &cx).await.unwrap();
        assert!(
            out.content.contains(
                "Layers failed at plate: the room came back with 1 face(s) in it, twice; drawn \
                 in one pass."
            ),
            "{}",
            out.content
        );
        let seen = seen.lock().unwrap().clone();
        let prompts: Vec<&String> = seen
            .iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .collect();
        assert_eq!(
            prompts.len(),
            3,
            "the plate, once more, then the single pass"
        );
        let plate = prompts[0];
        assert!(
            plate.contains("medium shot") && plate.contains("No people."),
            "{plate}"
        );
        assert!(
            !plate.contains("Maya") && !plate.contains("behind") && !plate.contains("her skin"),
            "{plate}"
        );
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// A step that fails draws the single pass and says where, never a
    /// different picture quietly (§15.5); the record carries no layers.
    #[tokio::test]
    async fn a_failed_layer_draws_the_single_pass_and_says_so() {
        let (url, seen) = fake_with(Fake {
            history: vec![done(); 3],
            views: vec![
                picture(8, [90, 90, 90]),
                cutout_png(false),
                picture(20, [200, 40, 40]),
            ],
            ..Fake::default()
        })
        .await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.layers = true;
        let out = t.call(touching_call(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains(
                "Layers failed at cutout 1 of 2: the cutout came back empty; drawn in one pass."
            ),
            "{}",
            out.content
        );
        let prompts = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .count();
        assert_eq!(prompts, 3, "plate, the empty cutout, then the single pass");
        let manifest = manifest_of(&dir, &out.content);
        assert_eq!(manifest["route"], "new");
        let landed = cx
            .scene
            .as_ref()
            .unwrap()
            .lookup(&std::fs::read(dir.join(picture_of(&out.content))).unwrap())
            .unwrap();
        assert!(landed.layers.is_none());
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
    }

    /// With the switch off, or for a scene that does not qualify, the
    /// picture is one render as today.
    #[tokio::test]
    async fn without_the_switch_a_touching_scene_is_one_render() {
        let (url, seen) = distinct(1).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let cx = scene_ctx(&dir, &store, "chat-a");
        let out = t.call(touching_call(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(!out.content.contains("layers"), "{}", out.content);
        let prompts = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("POST /prompt"))
            .count();
        assert_eq!(prompts, 1);
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
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
        // A call that gave a pose, or a scene with a `together`, takes no
        // pose from the reader: the splitter assigns the parts, and the
        // reader mirrored one-way acts (mecha-a3, 2026-10-09).
        assert_eq!(change.people[1].doing, None);
        assert_eq!(change.people.len(), 2, "nobody added");
        assert!(change.setting.is_none(), "the setting stays the call's");
        assert_eq!(merged, ["together", "wearing×1"]);
        // With neither, the reader's poses are taken.
        let bare = |who: &str| crate::scene::PersonChange {
            who: crate::scene::Who::Library(who.into()),
            at: None,
            wearing: None,
            doing: None,
            expression: None,
            remove: false,
        };
        let mut change = crate::scene::SceneChange {
            people: vec![bare("maya"), bare("john")],
            ..Default::default()
        };
        let read = json!({"people": [{"who": "Maya", "doing": "spinning"},
            {"who": "John", "doing": "clapping"}]});
        let merged = merge_read(&mut change, read.as_object().unwrap());
        assert_eq!(change.people[1].doing.as_deref(), Some("clapping"));
        assert_eq!(merged, ["doing×2"]);
        // Each condition alone holds the reader's poses back: a pose the
        // call gave, with no `together` anywhere...
        let mut posed = crate::scene::SceneChange {
            people: vec![bare("maya"), bare("john")],
            ..Default::default()
        };
        posed.people[0].doing = Some("waving".into());
        merge_read(&mut posed, read.as_object().unwrap());
        assert_eq!(posed.people[1].doing, None, "a call-given pose");
        // ...and a `together` the reader supplied, with no pose given.
        let mut joined = crate::scene::SceneChange {
            people: vec![bare("maya"), bare("john")],
            ..Default::default()
        };
        let read = json!({"together": "Maya and John dance",
            "people": [{"who": "Maya", "doing": "spinning"}, {"who": "John", "doing": "clapping"}]});
        merge_read(&mut joined, read.as_object().unwrap());
        assert_eq!(joined.together.as_deref(), Some("Maya and John dance"));
        assert_eq!(joined.people[1].doing, None, "a together from the reader");
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
        let read = json!({"together": long});
        merge_read(&mut change, read.as_object().unwrap());
        assert!(
            change.together.as_ref().unwrap().chars().count() <= crate::imagelib::MAX_CAST_FIELD
        );
        // A part too long to merge is left, with no `together` in the way
        // that would leave it anyway (review of #614, pass 2).
        let mut change = crate::scene::SceneChange {
            people: vec![bare("maya")],
            ..Default::default()
        };
        let read = json!({"people": [{"who": "Maya", "doing": long, "wearing": long}]});
        merge_read(&mut change, read.as_object().unwrap());
        assert!(change.people[0].doing.is_none(), "too long to merge");
        assert!(change.people[0].wearing.is_none(), "too long to merge");
    }

    /// An act that says what someone has on is their clothes: their own
    /// words, or a shared act naming them or nobody; a word is matched whole.
    #[test]
    fn an_act_that_dresses_someone_is_their_clothes() {
        let person = |who: &str| crate::scene::Person {
            who: crate::scene::Who::Library(who.into()),
            at: None,
            wearing: String::new(),
            doing: String::new(),
            expression: String::new(),
            origin: crate::scene::Origin::Clean,
        };
        let (maya, john) = (person("maya"), person("john"));
        let both = [&maya, &john];
        let none = std::collections::BTreeSet::new();
        let act =
            |p, doing, together: Option<&str>| clothes_in_act(p, doing, &[together], &both, &none);
        assert!(act(&maya, "wearing her raincoat unbuttoned", None));
        assert!(
            act(&maya, "dressed in a parka, waving", None),
            "her own act"
        );
        assert!(!act(&maya, "pegging towels on the clothesline", None));
        assert!(!act(&maya, "holding the door", None));
        let named = Some("Maya, wearing only a towel, waves to John");
        assert!(act(&maya, "", named));
        assert!(act(&john, "", named), "John is named too");
        let hers = Some("Maya, wearing only a towel, waves");
        assert!(!act(&john, "", hers), "not his act");
        let nobody = Some("both of them naked under the blanket, laughing");
        assert!(act(&john, "", nobody));
        assert!(!act(&john, "", Some("Maya waves to John")));
        // A shared act holds to clothing states: "dressed for the gala"
        // beside a red dress is one outfit, not two (review of #647, pass 3).
        assert!(!act(
            &maya,
            "",
            Some("they are dressed for the gala, dancing")
        ));
        assert!(!act(&maya, "", Some("both soaked through their clothes")));
        // The same in a person's own act: a task or an occasion is not a
        // clothing state (review of #647, pass 4).
        assert!(!act(&maya, "folding clothes into a suitcase", None));
        assert!(!act(&maya, "leaning on a worn fence", None), "an adjective");
        // An accessory alone is not an outfit: dropping the record's left the
        // prompt saying nothing of clothes (review of #647, pass 8).
        assert!(!act(&maya, "wearing a bike helmet, oiling a chain", None));
        assert!(!act(&maya, "wearing mirrored goggles", None));
        assert!(
            act(&maya, "wearing a parka and a beanie", None),
            "a garment too"
        );
        assert!(
            act(&maya, "wearing only wellington boots", None),
            "only makes the boots everything worn"
        );
        assert!(
            act(&maya, "wearing a wetsuit peeled to the waist", None),
            "an unknown item with a state"
        );
        assert!(act(&maya, "wearing a dirndl", None), "unknown items count");
        // Nor an expression or hair (review of #647, pass 9).
        assert!(!act(
            &maya,
            "wearing a wide smile, leaning on the rail",
            None
        ));
        assert!(!act(&maya, "wearing her hair in a braid", None));
        assert!(!act(
            &maya,
            "",
            Some("both of them wearing big grins, laughing")
        ));
        assert!(!act(&maya, "dressed for the gala, dancing", None));
        assert!(!act(&maya, "showing off her new outfit", None));
        // A retouch pasted into the same prompt is a shared text too.
        assert!(clothes_in_act(
            &maya,
            "",
            &[None, Some("she is topless now")],
            &both,
            &none
        ));
        // An outfit the call itself gave this turn is never left out.
        let asked = std::collections::BTreeSet::from(["maya".to_string()]);
        assert!(!clothes_in_act(
            &maya,
            "wearing her raincoat unbuttoned",
            &[None],
            &both,
            &asked
        ));
    }

    /// Whoever filled `wearing` (here a newcomer's default, and on a restage
    /// the record), a person whose act already says what they have on is
    /// stated without a second outfit; the record keeps the clothes, and the
    /// manifest says whose were left out. Fails where the prompt stated
    /// `wearing` whatever the act said (review of #647).
    #[tokio::test]
    async fn an_act_that_dresses_its_person_draws_one_outfit_and_says_so() {
        let (url, seen) = distinct(6).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let cx = clean(scene_ctx(&dir, &store, "chat-a"));
        // A newcomer with no clothes given: the default fills the field.
        let out = t
            .call(
                json!({"scene": {"setting": "a porch", "people": [
                    {"who": "maya", "doing": "wearing a green parka, shaking off the rain"},
                    {"who": "john", "wearing": "a flannel shirt", "doing": "holding the door"}
                ]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let prompt = last_prompt(&seen);
        assert!(prompt.contains("green parka"), "{prompt}");
        assert!(!prompt.contains("clothes that suit the scene"), "{prompt}");
        assert!(prompt.contains("flannel shirt"), "{prompt}");
        assert_eq!(
            prompt.matches("wearing").count(),
            2,
            "one outfit each: {prompt}"
        );
        assert_eq!(
            manifest_of(&dir, &out.content)["clothes_in_act"],
            json!(["Maya"])
        );
        // The result never tells the model she wears the default the prompt
        // left out (review of #647, pass 4).
        assert!(
            !out.content.contains("Maya had no clothes given"),
            "{}",
            out.content
        );

        // On the record in a raincoat, then an act that dresses her: the
        // restage draws from the record, and leaves its outfit out.
        let first = t
            .call(
                json!({"scene": {"setting": "a quiet reading room",
                       "people": [{"who": "maya", "wearing": "a yellow raincoat", "doing": "reading a book"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!first.is_error, "{}", first.content);
        assert!(manifest_of(&dir, &first.content)["clothes_in_act"].is_null());
        let p1 = picture_of(&first.content);
        let then = t
            .call(
                json!({"picture": p1, "scene": {"people": [
                    {"who": "maya", "doing": "wearing a green parka, shaking off the rain"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!then.is_error, "{}", then.content);
        let prompt = last_prompt(&seen);
        assert!(prompt.contains("green parka"), "{prompt}");
        assert!(!prompt.contains("yellow raincoat"), "{prompt}");
        assert_eq!(
            manifest_of(&dir, &then.content)["clothes_in_act"],
            json!(["Maya"])
        );
        let landed = cx
            .scene
            .as_ref()
            .unwrap()
            .lookup(&std::fs::read(dir.join(picture_of(&then.content))).unwrap())
            .unwrap();
        assert_eq!(
            landed.people[0].wearing, "a yellow raincoat",
            "the record keeps it"
        );

        // A described person on record in a grey hoodie, then in an act
        // that dresses them: the outfit is left out the same way, the result
        // never says they wear the record's (review of #647, pass 5), and
        // their description never reaches the manifest.
        let before = t
            .call(
                json!({"scene": {"setting": "a bus shelter", "people": [
                    {"who": "a tall stranger with a scar across one cheek",
                     "wearing": "a grey hoodie", "doing": "waiting"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!before.is_error, "{}", before.content);
        // A described person's outfit is left out the same way, but their
        // description never reaches the manifest, which carries no scene
        // words (review of #647, pass 3).
        let out = t
            .call(
                json!({"scene": {"setting": "a bus shelter", "people": [
                    {"who": "a tall stranger with a scar across one cheek",
                     "doing": "wearing a soaked overcoat, shivering"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let prompt = last_prompt(&seen);
        assert!(prompt.contains("soaked overcoat"), "{prompt}");
        assert!(!prompt.contains("grey hoodie"), "{prompt}");
        assert!(!prompt.contains("clothes that suit the scene"), "{prompt}");
        assert!(
            !out.content.contains("what this chat last drew them in"),
            "{}",
            out.content
        );
        let manifest = manifest_of(&dir, &out.content);
        assert!(manifest["clothes_in_act"].is_null(), "no description");
        assert_eq!(manifest["clothes_in_act_others"], 1, "but counted");

        // A retouch in the same prompt that states a clothing state is a
        // shared act too (review of #647, pass 4).
        let out = t
            .call(
                json!({"scene": {"setting": "a sunny terrace", "people": [
                    {"who": "maya", "doing": "stretching"}]},
                       "retouch": "she is topless now"}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let prompt = last_prompt(&seen);
        assert!(!prompt.contains("clothes that suit the scene"), "{prompt}");
        assert_eq!(
            manifest_of(&dir, &out.content)["clothes_in_act"],
            json!(["Maya"])
        );
    }

    /// On an edit the recorded `together` is not in the prompt unless the
    /// edit places people on a photo or changes it, so it never holds an
    /// outfit back; a newcomer's own act still does (review of #647, pass 5).
    #[tokio::test]
    async fn an_edit_reads_only_the_acts_its_prompt_carries() {
        let (url, seen) = distinct(3).await;
        let (dir, store, lib) = (
            tempdir(),
            tempdir(),
            library_with(&["maya", "john", "wren"]),
        );
        let t = tool(&url).with_library_dir(lib.clone());
        let cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let first = t
            .call(
                json!({"scene": {"setting": "a cabin by a lake",
                    "people": [{"who": "maya", "wearing": "a wool sweater", "where": "left"},
                               {"who": "john", "wearing": "a flannel shirt", "where": "right"}],
                    "together": "both of them wearing only bath towels, laughing"}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!first.is_error, "{}", first.content);
        // Only Maya's expression changes: the recorded act stays on the
        // record but is not in the edit prompt (an added or moved person
        // clears it, `Scene::apply`), so her recorded sweater is stated.
        let second = t
            .call(
                json!({"picture": picture_of(&first.content), "scene": {"people": [
                    {"who": "maya", "expression": "grinning"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!second.is_error, "{}", second.content);
        assert_eq!(manifest_of(&dir, &second.content)["route"], "edited");
        let prompt = last_prompt(&seen);
        assert!(prompt.contains("wool sweater"), "{prompt}");
        assert!(
            !prompt.contains("bath towels"),
            "not in this prompt: {prompt}"
        );
        assert!(manifest_of(&dir, &second.content)["clothes_in_act"].is_null());
        // Wren joins in an act that dresses her: her outfit is left out.
        let third = t
            .call(
                json!({"picture": picture_of(&second.content), "scene": {"people": [
                    {"who": "wren", "doing": "wearing a green parka, waving"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!third.is_error, "{}", third.content);
        let prompt = last_prompt(&seen);
        assert!(!prompt.contains("clothes that suit the scene"), "{prompt}");
        assert_eq!(
            manifest_of(&dir, &third.content)["clothes_in_act"],
            json!(["Wren"])
        );
        assert!(
            !third.content.contains("Wren had no clothes given"),
            "{}",
            third.content
        );
    }

    /// Switching to the turbo model unloads the standard one before the job
    /// is sent, and switching back unloads turbo; a picture on the model
    /// already held unloads nothing (the owner's ruling, 2026-10-11). Each
    /// picture's manifest says how it was sampled.
    #[tokio::test]
    async fn a_switch_of_model_unloads_the_other_first() {
        let (url, seen) = distinct(3).await;
        let (dir, store) = (tempdir(), tempdir());
        let t = tool(&url);
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        let call = json!({"scene": {"setting": "a lighthouse at dusk"}});
        let sent = |seen: &Arc<Mutex<Vec<String>>>| {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|l| l.starts_with("POST /free") || l.starts_with("POST /prompt"))
                .map(|l| {
                    if l.starts_with("POST /free") {
                        "free"
                    } else {
                        "prompt"
                    }
                })
                .collect::<Vec<_>>()
        };
        cx.image_tuning = crate::sampling::Tuning {
            model: Some(crate::sampling::Model::Turbo),
            ..Default::default()
        };
        let out = t.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(sent(&seen), ["free", "prompt"]);
        assert!(last_prompt(&seen).contains(crate::sampling::turbo_model()));
        let m = manifest_of(&dir, &out.content);
        assert_eq!(
            m["model"]["diffusion_model"],
            crate::sampling::turbo_model()
        );
        assert_eq!(m["steps"], crate::sampling::TURBO_STEPS);
        assert_eq!(m["sampling"]["negative"], false);
        // Back to standard: turbo out first.
        cx.image_tuning = Default::default();
        let out = t.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(sent(&seen), ["free", "prompt", "free", "prompt"]);
        // The same model again: nothing unloaded.
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(sent(&seen), ["free", "prompt", "free", "prompt", "prompt"]);
    }

    /// The chat's settings, read from their file on each call, are what a
    /// picture with no settings of its own is drawn with — through the
    /// persona form too — and a persona's own fields win (the owner,
    /// 2026-10-11). Without the layer (replay, eval), the turn's alone.
    #[tokio::test]
    async fn the_chat_settings_lie_under_every_picture() {
        let (url, seen) = distinct(4).await;
        let (dir, store) = (tempdir(), tempdir());
        let path = crate::sampling::chat_path(&tempdir());
        crate::sampling::set_chat(
            &path,
            crate::sampling::Tuning {
                cfg: Some(3.0),
                steps: Some(30),
                ..Default::default()
            },
        )
        .unwrap();
        let sampled = |seen: &Arc<Mutex<Vec<String>>>| {
            let line = last_prompt(seen);
            let body: Value = serde_json::from_str(&line[line.find('{').unwrap()..]).unwrap();
            let k = &body["prompt"]["sample"]["inputs"];
            (k["cfg"].as_f64().unwrap(), k["steps"].as_u64().unwrap())
        };
        let call = json!({"scene": {"setting": "a lighthouse at dusk"}});
        let t = tool(&url).with_chat_settings(path.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        let out = t.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(sampled(&seen), (3.0, 30), "the chat's");
        // A persona's own field wins; the rest are the chat's.
        let persona = t.persona_form(None);
        cx.image_tuning = crate::sampling::Tuning {
            cfg: Some(2.0),
            ..Default::default()
        };
        let out = persona.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            sampled(&seen),
            (2.0, 30),
            "the persona's cfg over the chat's steps"
        );
        // A change on the page is the next picture's: read per call.
        crate::sampling::set_chat(&path, Default::default()).unwrap();
        cx.image_tuning = Default::default();
        let out = t.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(sampled(&seen), (1.0, ImageConfig::default().steps as u64));
        // No layer: the turn's alone.
        crate::sampling::set_chat(
            &path,
            crate::sampling::Tuning {
                cfg: Some(3.0),
                ..Default::default()
            },
        )
        .unwrap();
        let out = tool(&url).call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(sampled(&seen).0, 1.0, "no chat layer");
    }

    /// The owner's picture LoRAs (LORA-DESIGN.md §5): the chat's list rides
    /// the graph between the loader and the sampler, in list order, with its
    /// triggers ahead of the prompt, and the result counts them without a
    /// name. A persona's empty list draws none; a name the registry lost is
    /// counted; a `.gguf` model leaves them out and says so.
    #[tokio::test]
    async fn the_owners_loras_ride_the_graph_and_the_result_counts_them() {
        let (url, seen) = distinct(6).await;
        let (dir, store, home) = (tempdir(), tempdir(), tempdir());
        for f in ["paint", "grain"] {
            crate::loras::write_test_lora(
                &home.join("pics").join(format!("{f}.safetensors")),
                crate::loras::QWEN_KEYS,
            );
        }
        std::fs::write(
            home.join(crate::loras::FILE),
            "[[lora]]\nname = \"paint\"\nabout = \"a made-up paint look\"\nmodel = \"qwen-image\"\n\
             file = \"paint.safetensors\"\nstrength = 0.8\ntrigger = \"oil paint\"\n\n\
             [[lora]]\nname = \"grain\"\nabout = \"a made-up film grain\"\nmodel = \"qwen-image\"\n\
             file = \"grain.safetensors\"\n",
        )
        .unwrap();
        let chat = crate::sampling::chat_path(&home);
        crate::sampling::set_chat_loras(
            &chat,
            &crate::loras::Layer::Set(vec!["paint".into(), "grain".into()]),
        )
        .unwrap();
        let cfg = |model: &str| ImageConfig {
            url: url.clone(),
            min_available_mb: 0,
            unload_after_secs: 0,
            loras_dir: Some(vec![home.join("pics")]),
            diffusion_model: model.into(),
            ..Default::default()
        };
        let t = ImageGenerate::new(cfg(&ImageConfig::default().diffusion_model))
            .unwrap()
            .with_chat_settings(chat.clone())
            .with_loras(home.clone());
        let graph = |seen: &Arc<Mutex<Vec<String>>>| {
            let line = last_prompt(seen);
            let body: Value = serde_json::from_str(&line[line.find('{').unwrap()..]).unwrap();
            body["prompt"].clone()
        };
        let call = json!({"scene": {"setting": "a lighthouse at dusk"}});
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        let out = t.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        let g = graph(&seen);
        assert_eq!(g["lora1"]["inputs"]["lora_name"], "paint.safetensors");
        assert_eq!(g["lora1"]["inputs"]["strength_model"], 0.8);
        assert_eq!(g["lora1"]["inputs"]["model"], json!(["unet", 0]));
        assert_eq!(g["lora2"]["inputs"]["lora_name"], "grain.safetensors");
        assert_eq!(g["lora2"]["inputs"]["model"], json!(["lora1", 0]));
        assert_eq!(g["sample"]["inputs"]["model"], json!(["lora2", 0]));
        let prompt = g["encode"]["inputs"]["prompt"].as_str().unwrap();
        assert!(prompt.starts_with("oil paint. "), "{prompt}");
        assert!(out.content.contains("applied 2 LoRAs"), "{}", out.content);
        assert!(out.content.contains("trigger words"), "{}", out.content);
        for name in ["paint", "grain"] {
            assert!(!out.content.contains(name), "named: {}", out.content);
        }
        assert_eq!(manifest_of(&dir, &out.content)["loras"]["applied"], 2);
        // Through the persona form, a persona set to none draws none.
        let persona = t.persona_form(None);
        cx.image_loras = crate::loras::Layer::Set(vec![]);
        let out = persona.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(graph(&seen).get("lora1").is_none());
        assert!(!out.content.contains("LoRA"), "{}", out.content);
        assert!(manifest_of(&dir, &out.content)["loras"].is_null());
        // A name the registry no longer has is counted, never named.
        cx.image_loras = crate::loras::Layer::Set(vec!["grain".into(), "gone".into()]);
        let out = persona.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            graph(&seen)["lora1"]["inputs"]["lora_name"],
            "grain.safetensors"
        );
        assert!(
            out.content
                .contains("1 LoRA in picture settings is no longer in the registry"),
            "{}",
            out.content
        );
        // A registry that does not read, with none read before: said as
        // that, never as names deleted (review of #751).
        let cold = ImageGenerate::new(cfg(&ImageConfig::default().diffusion_model))
            .unwrap()
            .with_chat_settings(chat.clone())
            .with_loras(home.clone());
        let registry = std::fs::read_to_string(home.join(crate::loras::FILE)).unwrap();
        std::fs::write(home.join(crate::loras::FILE), "[[lora]\n").unwrap();
        cx.image_loras = crate::loras::Layer::Inherit;
        let out = cold.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("registry could not be read"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("no longer"), "{}", out.content);
        // The tool that read it whole before keeps that read.
        let out = t.call(call.clone(), &cx).await.unwrap();
        assert!(out.content.contains("applied 2 LoRAs"), "{}", out.content);
        std::fs::write(home.join(crate::loras::FILE), registry).unwrap();
        // A .gguf model file: left out, and said.
        cx.image_loras = crate::loras::Layer::Inherit;
        let gguf = ImageGenerate::new(cfg("Qwen-Image-2.1-Q4.gguf"))
            .unwrap()
            .with_chat_settings(chat)
            .with_loras(home);
        let out = gguf.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(graph(&seen).get("lora1").is_none());
        assert!(out.content.contains(".gguf"), "{}", out.content);
    }

    /// The pinned turbo model missing from the server is refused naming the
    /// command that fetches it — turbo is fetched on demand (the owner,
    /// 2026-10-11) — before anything is sent. A configured `turbo_model`
    /// that command cannot supply is refused naming the settings instead,
    /// so the refusal never names a command that would not end it (review
    /// of #650).
    #[tokio::test]
    async fn a_missing_turbo_model_is_refused_naming_the_fetch() {
        let turbo = crate::sampling::Tuning {
            model: Some(crate::sampling::Model::Turbo),
            ..Default::default()
        };
        let call = json!({"scene": {"setting": "a lighthouse at dusk"}});
        let (url, seen) = fake_with(Fake {
            no_turbo: true,
            ..Fake::default()
        })
        .await;
        let (dir, store) = (tempdir(), tempdir());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.image_tuning = turbo.clone();
        let out = tool(&url).call(call.clone(), &cx).await.unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(
            out.content.contains("features enable image --turbo"),
            "{}",
            out.content
        );
        // Not the pinned file: the settings' refusal, no command.
        let own = ImageGenerate::new(ImageConfig {
            url: url.clone(),
            min_available_mb: 0,
            unload_after_secs: 0,
            // Through the unet loader, where the pinned-file check lives.
            turbo_model: "my-turbo.safetensors".into(),
            ..Default::default()
        })
        .unwrap()
        .polling_every(Duration::from_millis(10))
        .with_library_dir(tempdir())
        .with_seeds();
        let out = own.call(call, &cx).await.unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(!out.content.contains("--turbo"), "{}", out.content);
        assert!(out.content.contains("picture settings"), "{}", out.content);
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.starts_with("POST /prompt")),
            "nothing was sent"
        );
    }

    /// A sampler the persona's settings choose and the server lacks is
    /// refused before anything is sent, naming the setting: it stays chosen
    /// until the owner changes it, so a bare node error would recur on every
    /// picture (review of #648). One the server has draws.
    #[tokio::test]
    async fn a_chosen_sampler_the_server_lacks_is_refused_naming_the_setting() {
        let (url, seen) = distinct(1).await;
        let (dir, store) = (tempdir(), tempdir());
        let t = tool(&url);
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        let call = json!({"scene": {"setting": "a lighthouse at dusk"}});
        cx.image_tuning = crate::sampling::Tuning {
            sampler: Some("dpmpp_2m".into()),
            ..Default::default()
        };
        let out = t.call(call.clone(), &cx).await.unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(out.content.contains("picture settings"), "{}", out.content);
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.starts_with("POST /prompt")),
            "nothing was sent"
        );
        cx.image_tuning.sampler = Some("res_multistep".into());
        cx.image_tuning.scheduler = Some("beta".into());
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        // Both inputs read from one request (review of #648, pass 3): one
        // for the refused picture, one for this.
        let asked = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("GET /object_info/KSampler"))
            .count();
        assert_eq!(asked, 2);
    }

    /// After an idle release the server holds nothing, so the next picture
    /// unloads nothing, whichever model it asks for (review of #648); a
    /// server this process never used reads as holding the standard model.
    #[test]
    fn a_released_server_has_nothing_to_unload() {
        let addr: std::net::SocketAddr = "127.0.0.1:9".parse().unwrap();
        let server = server_at(addr);
        let (standard, turbo) = ("standard.safetensors", crate::sampling::turbo_model());
        assert!(!server.switching(standard, standard));
        assert!(
            server.switching(turbo, standard),
            "unused reads as standard"
        );
        server.drew_with(turbo);
        assert!(server.switching(standard, standard));
        server.released();
        assert!(!server.switching(standard, standard));
        assert!(!server.switching(turbo, standard));
    }

    /// A fresh process reads which model the server holds from the shared
    /// record: after another drew with turbo, a standard picture unloads it
    /// (mecha-a3's gate of #648). Fails where each process assumed standard.
    #[test]
    fn a_fresh_process_reads_the_held_model_from_the_record() {
        let record = tempdir().join("held-model.json");
        let addr: std::net::SocketAddr = "127.0.0.1:11".parse().unwrap();
        let (standard, turbo) = ("standard.safetensors", crate::sampling::turbo_model());
        let earlier = server_at(addr);
        earlier.model_record.set(record.clone()).unwrap();
        earlier.drew_with(turbo);
        // Another process: its own client, the same record.
        let fresh = server_at(addr);
        fresh.model_record.set(record.clone()).unwrap();
        assert!(fresh.switching(standard, standard), "turbo is held");
        fresh.released();
        assert!(
            !earlier.switching(standard, standard),
            "released, as the record says"
        );
        // An unreadable record falls back to what this process last did:
        // here, released (review of #648, pass 5).
        std::fs::write(&record, "not json").unwrap();
        assert!(!fresh.switching(standard, standard));
        assert!(!fresh.switching(turbo, standard));
    }

    /// A record that cannot be written never reads as the standard model:
    /// this process's own last picture stands, so the next switch still
    /// unloads (review of #648, pass 5). Fails where an unwritten record
    /// read as an unused server.
    #[test]
    fn an_unwritten_record_falls_back_to_what_this_process_drew() {
        let addr: std::net::SocketAddr = "127.0.0.1:13".parse().unwrap();
        let (standard, turbo) = ("standard.safetensors", crate::sampling::turbo_model());
        let server = server_at(addr);
        server
            .model_record
            .set(std::path::PathBuf::from(
                "/proc/mecha-no-such-dir/held-model.json",
            ))
            .unwrap();
        server.drew_with(turbo);
        assert!(
            server.switching(standard, standard),
            "turbo is what it sent"
        );
    }

    /// An idle unload the server did not take is not recorded as one: the
    /// record keeps the model it holds, so the next switch still unloads it
    /// (review of #648, pass 5).
    #[tokio::test]
    async fn an_idle_unload_the_server_refused_leaves_the_record() {
        let record = tempdir().join("held-model.json");
        // Nothing listens on port 1: the /free fails.
        let addr: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
        let (standard, turbo) = ("standard.safetensors", crate::sampling::turbo_model());
        let server = server_at(addr);
        server.model_record.set(record.clone()).unwrap();
        server.drew_with(turbo);
        server.release_idle().await;
        assert!(server.switching(standard, standard), "still turbo");
        assert_eq!(
            read_held(&record)
                .get(server.wire.base().as_str())
                .map(String::as_str),
            Some(turbo)
        );
    }

    /// An edit that dresses someone states the new outfit in its instruction,
    /// so the manifest never claims it was left out, even where their act
    /// states a clothing state: a reader-merged outfit on a bare follow-up
    /// is the reachable shape (review of #647, pass 7). Fails where the edit
    /// path suppressed and claimed whatever the instruction said.
    #[tokio::test]
    async fn an_edit_that_dresses_someone_never_claims_their_outfit_left_out() {
        let (url, seen) = distinct(2).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let first = t
            .call(
                json!({"scene": {"setting": "a porch", "people": [
                    {"who": "maya", "wearing": "a wool sweater",
                     "doing": "wearing a green parka, shaking off the rain"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!first.is_error, "{}", first.content);
        cx.scene_reader = Some(Arc::new(FakeReader(Ok(json!({"people": [
            {"who": "Maya", "wearing": "a red dress"}
        ]})))));
        let then = t
            .call(
                json!({"picture": picture_of(&first.content),
                       "scene": {"people": [{"who": "maya"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!then.is_error, "{}", then.content);
        let manifest = manifest_of(&dir, &then.content);
        assert_eq!(manifest["route"], "edited", "{manifest}");
        assert!(last_prompt(&seen).contains("Dress Maya in a red dress"));
        assert!(manifest["clothes_in_act"].is_null(), "{manifest}");
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

    /// A reader that notes whether it was handed a record, and finds nothing.
    #[derive(Debug)]
    struct RecordSeen(Arc<std::sync::Mutex<Vec<bool>>>);

    #[async_trait]
    impl crate::persona::edit::SceneReader for RecordSeen {
        async fn read(
            &self,
            record: Option<&crate::scene::Scene>,
        ) -> std::result::Result<crate::persona::edit::Extracted, String> {
            self.0.lock().unwrap().push(record.is_some());
            Ok(crate::persona::edit::Extracted {
                scene: Default::default(),
                retouch: None,
                left_out: None,
            })
        }
    }

    /// A new picture starts from nothing (the owner, 2026-10-27): the chat's
    /// last picture lends it no clothes or place, and the reader is handed
    /// no record. One that names the picture it builds on carries both.
    /// Fails where a new picture's clothes and reader record came from the
    /// chat's copy.
    #[tokio::test]
    async fn a_new_picture_carries_nothing_from_the_chats_last() {
        let (url, seen) = distinct(4).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = clean(scene_ctx(&dir, &store, "chat-a"));
        let first = t
            .call(
                json!({"scene": {"setting": "a quiet reading room",
                       "people": [{"who": "maya", "wearing": "a green coat", "doing": "reading a book"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!first.is_error, "{}", first.content);
        let p1 = picture_of(&first.content);
        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        cx.scene_reader = Some(Arc::new(RecordSeen(Arc::clone(&records))));

        let second = t
            .call(
                json!({"scene": {"setting": "a beach at dawn",
                       "people": [{"who": "maya", "doing": "flying a kite"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!second.is_error, "{}", second.content);
        let prompt = last_prompt(&seen);
        assert!(prompt.contains("a beach at dawn"), "{prompt}");
        assert!(
            !prompt.contains("green coat"),
            "the last clothes rode along: {prompt}"
        );
        // No code path has carried a setting into a new picture (`plan` takes
        // `base`, and the reader merges only acts and clothes): a place came
        // back only when the model restated it from the note, which the note
        // test and the description checks below guard. Kept against a new
        // channel.
        assert!(
            !prompt.contains("reading room"),
            "the last place rode along: {prompt}"
        );
        assert_eq!(
            *records.lock().unwrap(),
            vec![false],
            "the reader read the chat's record"
        );

        let third = t
            .call(
                json!({"picture": p1, "scene": {"people": [{"who": "maya", "doing": "standing by the window"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!third.is_error, "{}", third.content);
        let prompt = last_prompt(&seen);
        assert!(
            prompt.contains("a green coat"),
            "building on it lost the clothes: {prompt}"
        );
        assert_eq!(records.lock().unwrap().last(), Some(&true));

        // An upload named in `picture` has no record of its own, and borrows
        // none from the chat's last picture (review of #753, pass 2).
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("inbox/beach.png"), picture(30, [10, 120, 200])).unwrap();
        let upload = t
            .call(
                json!({"picture": "inbox/beach.png", "scene": {"people": [{"who": "maya", "doing": "standing on the sand"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!upload.is_error, "{}", upload.content);
        let prompt = last_prompt(&seen);
        assert!(
            !prompt.contains("green coat"),
            "an upload wore the last picture's clothes: {prompt}"
        );
        assert_eq!(
            records.lock().unwrap().last(),
            Some(&false),
            "the reader was handed another picture's record"
        );
        // What the model is told, where it decides: a new picture keeps
        // nothing, and `picture` is for when the owner asks to build on one.
        assert!(DESCRIPTION.contains("a new picture keeps nothing from earlier ones"));
        assert!(DESCRIPTION
            .contains("Only when the user asks to continue, build on, edit or change a picture"));
        assert!(PICTURE_DESC
            .contains("only when the user asks to continue, build on, edit or change it"));
    }

    /// A persona's bare call is drawn with what the reader found it left
    /// out, stamped with the conversation's own origin (an untrusted turn's
    /// merged words land untrusted), and said in the manifest; a reader that
    /// fails leaves the call as sent.
    /// A new picture that lists nobody but names two characters draws them,
    /// and draws them ahead of the reader, so the clothes the owner gave
    /// them are merged rather than "clothes that suit the scene" (review of
    /// #613). Fails where the reader ran only on a call that listed people.
    #[tokio::test]
    async fn named_characters_drawn_from_the_words_still_get_the_readers_clothes() {
        let (url, seen) = distinct(1).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        cx.scene_reader = Some(Arc::new(FakeReader(Ok(json!({"people": [
            {"who": "Maya", "wearing": "a yellow raincoat"},
            {"who": "John", "wearing": "a flannel shirt"}
        ]})))));
        let out = t
            .call(
                json!({"scene": {"setting": "a café", "together": "Maya and John share a pot of tea"}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("Maya, John named in the words"),
            "{}",
            out.content
        );
        assert_eq!(
            manifest_of(&dir, &out.content)["reader"],
            "merged: wearing×2"
        );
        assert!(last_prompt(&seen).contains("yellow raincoat"));
    }

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
        let (url, _seen) = distinct(2).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = clean(scene_ctx(&dir, &store, "chat-a"));
        // A picture whose record is clean but for its `together`, which the
        // next call clears by posing someone: without the union, nothing
        // untrusted would be left in the scene that lands.
        std::fs::create_dir_all(dir.join("images")).unwrap();
        let bytes = picture(8, [120, 90, 60]);
        std::fs::write(dir.join("images/base.png"), &bytes).unwrap();
        let clean_field = |v: &str| {
            Some(crate::scene::Field {
                value: v.to_string(),
                origin: crate::scene::Origin::Clean,
            })
        };
        let person = |key: &str, wearing: &str| crate::scene::Person {
            who: crate::scene::Who::Library(key.into()),
            at: None,
            wearing: wearing.into(),
            doing: String::new(),
            expression: String::new(),
            origin: crate::scene::Origin::Clean,
        };
        let base = crate::scene::Scene {
            setting: Some(crate::scene::Field {
                value: crate::scene::Setting::Words {
                    text: "a park".into(),
                },
                origin: crate::scene::Origin::Clean,
            }),
            light: clean_field("a flat grey overcast"),
            together: Some(crate::scene::Field {
                value: "Maya hands John a cup".into(),
                origin: crate::scene::Origin::Untrusted,
            }),
            people: vec![person("maya", "a coat"), person("john", "a suit")],
            people_known: true,
            picture: Some(crate::scene::hash(&bytes)),
            chat: Some("chat-a".into()),
            ..Default::default()
        };
        cx.scene.as_ref().unwrap().land(&base).unwrap();
        cx.scene_reader = Some(Arc::new(FakeReader(Ok(json!({
            "people": [{"who": "Maya", "doing": "tying a shoelace"}]
        })))));
        let out = t
            .call(
                json!({"picture": "images/base.png", "scene": {"people": [{"who": "maya"}]}}),
                &cx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(manifest_of(&dir, &out.content)["reader"], "merged: doing×1");
        let landed = cx
            .scene
            .as_ref()
            .unwrap()
            .lookup(&std::fs::read(dir.join(picture_of(&out.content))).unwrap())
            .unwrap();
        assert!(landed.together.is_none(), "the pose change cleared it");
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
        // The chat's own words, apart, for the privacy guard.
        let words: Vec<&str> = line["words"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(words.contains(&"a quiet harbour"), "{words:?}");
        assert!(words.contains(&"waving"), "{words:?}");
        assert!(!words.iter().any(|w| w.contains("<image")), "{words:?}");
        // No reader stamped on a scene call: said, never a silent null.
        assert_eq!(line["reader"], "not read: no reader here");
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
            _layered: bool,
        ) -> std::result::Result<crate::roles::Split, String> {
            self.0.clone()
        }
    }

    /// A splitter that answers nothing and keeps whether each ask was for
    /// a layered build.
    #[derive(Debug, Default)]
    struct AskedSplit(std::sync::Mutex<Vec<bool>>);

    #[async_trait]
    impl crate::roles::RoleSplit for AskedSplit {
        async fn split(
            &self,
            _people: &[crate::roles::Asked],
            _together: &str,
            layered: bool,
        ) -> std::result::Result<crate::roles::Split, String> {
            self.0.lock().unwrap().push(layered);
            Err("no answer".into())
        }
    }

    /// The split asks for named parts only where the picture may be built in
    /// layers: on the single pass the naming sentence emptied parts and kept
    /// no more acts (mecha-a3, 2026-10-10; review of #626).
    #[tokio::test]
    async fn only_a_picture_that_may_layer_asks_for_named_parts() {
        let (url, _seen) = distinct(2).await;
        let (dir, store, lib) = (tempdir(), tempdir(), library_with(&["maya", "john"]));
        let t = tool(&url).with_library_dir(lib.clone());
        let mut cx = scene_ctx(&dir, &store, "chat-a");
        let asked = Arc::new(AskedSplit::default());
        cx.role_split = Some(asked.clone());
        // No places given, so the failed split leaves the picture one pass.
        let mut call = touching_call();
        for p in call["scene"]["people"].as_array_mut().unwrap() {
            p.as_object_mut().unwrap().remove("where");
        }
        cx.layers = true;
        let out = t.call(call.clone(), &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        cx.layers = false;
        let out = t.call(call, &cx).await.unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(*asked.0.lock().unwrap(), [true, false]);
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(store).ok();
        std::fs::remove_dir_all(lib).ok();
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

    /// A region's words are read for a broken entry too: the panel sends
    /// them as `regions`, not `retouch` (review of #623).
    #[tokio::test]
    async fn a_broken_entry_named_in_a_region_is_refused_before_the_gpu() {
        let lib = library_with(&["maya", "john"]);
        std::fs::write(lib.join("characters/john/entry.toml"), "not = [toml").unwrap();
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::write(dir.join("images/a.png"), picture(20, [200, 30, 30])).unwrap();
        let out = tool("http://127.0.0.1:1")
            .with_library_dir(lib.clone())
            .call(
                json!({"picture": "images/a.png", "mask": "inbox/r.png",
                       "regions": [{"colour": "magenta", "words": "John in a red coat"}]}),
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

    /// Regions on a picture that is not in the chat are refused even beside
    /// a scene: left out, their legend would ride into a new picture's
    /// prompt describing outlines that do not exist (review of #623).
    #[tokio::test]
    async fn regions_on_a_made_up_picture_draw_nothing() {
        let (url, seen) = distinct(1).await;
        let dir = tempdir();
        let out = tool(&url)
            .call(
                json!({"picture": "images/nope.png", "mask": "inbox/r.png",
                       "regions": [{"colour": "magenta", "words": "a red coat"}],
                       "scene": {"people": [{"who": "a woman in a grey coat"}]}}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(
            out.content.contains("painted regions mark places in it"),
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
