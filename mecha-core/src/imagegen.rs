//! Local image generation: the `image_generate` tool, and the one backend it
//! speaks today (ComfyUI running Qwen-Image 2.1).
//!
//! **The model never authors the workflow.** ComfyUI's `/prompt` executes any
//! node graph it is handed — nodes that write files, fetch URLs, or run a
//! custom node's code. So the graph is fixed here, in code, and the model
//! supplies typed values only: a prompt, a negative prompt, a size from a
//! closed set, a seed. The prompt reaches the graph as a JSON string value, so
//! nothing in it can become a node.
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
//! **The model cannot see what it made** — images enter a conversation on user
//! turns only (`ARCHITECTURE.md` §Images) — so the result says so, and gives
//! the seed back: revising means editing the prompt and reusing the seed.
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
    /// File names as the server lists them (ComfyUI: under `models/`).
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
    /// machine took it down alongside `llama-server` and a parallel link; a
    /// cold 1024² generation measured a 15 GB peak.
    pub min_available_mb: u64,
    /// Ask the server to unload its models this long after the last
    /// generation, so ~15 GB is not held between requests. `0` keeps them.
    pub unload_after_secs: u64,
}

impl Default for ImageConfig {
    fn default() -> Self {
        ImageConfig {
            backend: ImageBackend::Comfyui,
            url: "http://127.0.0.1:8188".into(),
            diffusion_model: "Qwen-Image-2.1-Q4.gguf".into(),
            text_encoder: "qwen3vl_8b_w4a8.safetensors".into(),
            vae: "qwen_image_2.1_vae_bf16.safetensors".into(),
            steps: 40,
            timeout_secs: 600,
            min_available_mb: 16_384,
            unload_after_secs: 600,
        }
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
    // `host_str` brackets an IPv6 literal; an IP parses, anything else is a
    // name, and the only name that is this machine by definition is localhost.
    let local = match url
        .host_str()
        .map(|h| h.trim_start_matches('[').trim_end_matches(']'))
    {
        Some(host) => match host.parse::<std::net::IpAddr>() {
            Ok(ip) => ip.is_loopback(),
            Err(_) => host.eq_ignore_ascii_case("localhost"),
        },
        None => false,
    };
    if !local {
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
    fn parse(s: &str) -> Option<Self> {
        match s {
            "square" => Some(Size::Square),
            "landscape" => Some(Size::Landscape),
            "portrait" => Some(Size::Portrait),
            _ => None,
        }
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

/// At most this many references per call. The model takes ten; every one is
/// a VAE encode and a slice of the sequence on the shared memory pool, and
/// four covers "edit this, in the style of that".
const MAX_REFERENCES: usize = 4;

/// A reference larger than this is refused rather than read. A phone photo
/// is well under it; the node resizes to about 1024² anyway.
const MAX_REFERENCE_BYTES: u64 = 25 * 1024 * 1024;

/// The image type of `bytes`, by magic number, as an upload extension.
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
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

const PROMPT_CAP: usize = 4_000;

/// Consecutive failed status polls before a job is abandoned.
const POLL_FAILURES: u32 = 3;

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
pub fn comfy_graph(cfg: &ImageConfig, req: &Request, uploaded: &[String]) -> Value {
    let mut encode = json!({
        "clip": ["clip", 0], "prompt": req.prompt, "negative_prompt": req.negative,
        "resolution": 1024});
    let mut graph = json!({
        "unet": {"class_type": "UnetLoaderGGUF", "inputs": {"unet_name": cfg.diffusion_model}},
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
    let canvas = match req.size {
        Some((width, height)) => {
            graph["latent"] = json!({"class_type": "EmptyLatentImage", "inputs": {
                "width": width, "height": height, "batch_size": 1}});
            json!(["latent", 0])
        }
        None => json!(["encode", 2]),
    };
    graph["encode"] = json!({"class_type": "TextEncodeQwenImage21", "inputs": encode});
    graph["sample"] = json!({"class_type": "KSampler", "inputs": {
        "model": ["unet", 0], "positive": ["encode", 0], "negative": ["encode", 1],
        "latent_image": canvas, "seed": req.seed, "steps": req.steps,
        // Guidance off: the reference setting. Above 1 doubles every step.
        "cfg": 1.0, "sampler_name": "euler", "scheduler": "simple", "denoise": 1.0}});
    graph
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
}

/// Why a generation did not produce an image.
enum Failure {
    Cancelled,
    Other(anyhow::Error),
}

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

impl ComfyUi {
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

    /// Every file the graph names is one the server can load, checked before
    /// submitting: a missing file otherwise fails deep in a run as a node
    /// validation error the model cannot act on.
    async fn preflight(&self, cfg: &ImageConfig) -> Result<()> {
        let checks = [
            ("UnetLoaderGGUF", "unet_name", &cfg.diffusion_model),
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
    async fn abandon(&self, id: &str) {
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
        }
        // A cancelled or interrupted job is still recorded, prompt and all.
        self.forget(id).await;
    }

    /// Put one reference in the server's temp directory under a name nobody
    /// chose, and return the name the server filed it under. Multipart by
    /// hand: one file and one field do not justify a crate feature.
    async fn upload(&self, reference: &Reference) -> Result<String> {
        let boundary = format!("mecha-{:08x}{:08x}", fresh_seed(), fresh_seed());
        let filename = format!(
            "mecha-{:08x}{:08x}.{}",
            fresh_seed(),
            fresh_seed(),
            reference.ext
        );
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

    async fn generate(
        &self,
        cfg: &ImageConfig,
        req: &Request,
        cancel: Option<&CancellationToken>,
        timeout: Duration,
    ) -> std::result::Result<Vec<u8>, Failure> {
        self.preflight(cfg).await?;
        let mut uploaded = Vec::with_capacity(req.references.len());
        for reference in &req.references {
            uploaded.push(self.upload(reference).await?);
        }
        let (status, body) = self
            .post_json(
                "prompt",
                &json!({"prompt": comfy_graph(cfg, req, &uploaded), "client_id": "mecha"}),
            )
            .await?;
        if !status.is_success() {
            return Err(anyhow!(
                "the image server rejected the job ({status}): {}",
                clip(&body)
            )
            .into());
        }
        let id = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("prompt_id")?.as_str().map(str::to_string))
            .ok_or_else(|| anyhow!("the image server accepted the job but named no prompt_id"))?;

        let started = Instant::now();
        let mut failed_polls = 0u32;
        let image = loop {
            let tick = tokio::time::sleep(self.poll);
            match cancel {
                Some(token) => tokio::select! {
                    _ = token.cancelled() => {
                        self.abandon(&id).await;
                        return Err(Failure::Cancelled);
                    }
                    _ = tick => {}
                },
                None => tick.await,
            }
            if started.elapsed() > timeout {
                self.abandon(&id).await;
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
                    self.abandon(&id).await;
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
async fn save(ctx: &ToolCtx, seed: u64, bytes: &[u8]) -> Result<String> {
    use tokio::io::AsyncWriteExt;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
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
/// The pixels go to the loopback server and nowhere else — never into the
/// conversation, so nothing here arms taint; the result names paths only.
async fn read_references(
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
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
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

pub struct ImageGenerate {
    cfg: ImageConfig,
    backend: Arc<ComfyUi>,
    /// Bumped per finished generation; an idle-unload timer fires only if it
    /// still holds the value it was armed with.
    generation: Arc<AtomicU64>,
}

impl ImageGenerate {
    /// Refuses a configuration whose server is not on this machine.
    pub fn new(cfg: ImageConfig) -> Result<Self> {
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
        Ok(ImageGenerate {
            backend: Arc::new(ComfyUi {
                base,
                http,
                poll: Duration::from_secs(1),
            }),
            cfg,
            generation: Arc::new(AtomicU64::new(0)),
        })
    }

    #[cfg(test)]
    fn polling_every(mut self, poll: Duration) -> Self {
        Arc::get_mut(&mut self.backend)
            .expect("unshared in tests")
            .poll = poll;
        self
    }

    /// The call's input, validated, and the reference paths still to read —
    /// reading needs the run's workspace, which [`Self::call`] has.
    fn request(&self, input: &Value) -> std::result::Result<(Request, Vec<String>), String> {
        let prompt = input
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if prompt.is_empty() {
            return Err("`prompt` is required: describe the image.".into());
        }
        if prompt.chars().count() > PROMPT_CAP {
            return Err(format!(
                "`prompt` is over {PROMPT_CAP} characters; shorten it."
            ));
        }
        let negative = input
            .get("negative_prompt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if negative.chars().count() > PROMPT_CAP {
            return Err(format!(
                "`negative_prompt` is over {PROMPT_CAP} characters."
            ));
        }
        let references: Vec<String> = match input.get("reference_images") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|v| v.as_str().map(|s| s.trim().to_string()))
                .collect::<Option<_>>()
                .ok_or("`reference_images` must be a list of paths.")?,
            Some(_) => return Err("`reference_images` must be a list of paths.".into()),
        };
        if references.len() > MAX_REFERENCES {
            return Err(format!(
                "At most {MAX_REFERENCES} reference images per call, not {}.",
                references.len()
            ));
        }
        let size = match input.get("size").and_then(Value::as_str) {
            // An edit follows its first reference's shape unless asked not to.
            None if !references.is_empty() => None,
            None => Some(Size::Square),
            Some(s) => Some(Size::parse(s).ok_or_else(|| {
                format!("`size` must be square, landscape or portrait, not `{s}`.")
            })?),
        };
        let seed = match input.get("seed") {
            None | Some(Value::Null) => fresh_seed(),
            Some(v) => v
                .as_u64()
                .ok_or_else(|| "`seed` must be a whole number, zero or more.".to_string())?,
        };
        Ok((
            Request {
                prompt: prompt.to_string(),
                negative: negative.to_string(),
                size: size.map(Size::dims),
                steps: self.cfg.steps,
                seed,
                references: Vec::new(),
            },
            references,
        ))
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

#[async_trait]
impl Tool for ImageGenerate {
    fn name(&self) -> &str {
        "image_generate"
    }

    fn description(&self) -> &str {
        "Generate an image with the local image model, or edit one, and save the result as a \
         PNG in the workspace. Takes about a minute. It renders text inside images well — put \
         the exact words in quotes. To edit, pass the picture's path in reference_images (one \
         the user attached, or an earlier result) and say in the prompt what to change and \
         what to keep, e.g. \"Keep <image1> unchanged except: the jacket is now yellow\". You \
         will not see the result; the user will."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "What the image shows, as descriptive prose: subject, setting, style, lighting. Quote any text that should appear in it."
                },
                "negative_prompt": {
                    "type": "string",
                    "description": "What to keep out of the image. Usually leave it out."
                },
                "size": {
                    "type": "string",
                    "enum": ["square", "landscape", "portrait"],
                    "description": "Default \"square\" (1024×1024); an edit defaults to its first reference's shape. landscape is 1344×768, portrait 768×1344."
                },
                "reference_images": {
                    "type": "array",
                    "items": {"type": "string"},
                    "maxItems": MAX_REFERENCES,
                    "description": "Workspace paths of images to edit or draw from — an attached picture (inbox/...) or an earlier result (images/...). The first is the one being edited; refer to them as <image1>, <image2> in the prompt."
                },
                "seed": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Without reference_images, an earlier result's seed keeps its composition while the prompt changes. Omit it for a new image. An edit always uses a fresh seed, so it is ignored there."
                }
            },
            "required": ["prompt"]
        })
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
        let (mut req, paths) = match self.request(&input) {
            Ok(parsed) => parsed,
            Err(why) => return Ok(ToolOutput::err(why)),
        };
        // Before reading anything: up to a hundred megabytes of references is
        // itself a cost on the pool this check guards.
        if let Err(why) = memory_verdict(mem_available_mb(), self.cfg.min_available_mb) {
            return Ok(ToolOutput::err(why));
        }
        req.references = match read_references(ctx, &paths).await {
            Ok(references) => references,
            Err(why) => return Ok(ToolOutput::err(why)),
        };
        // An edit always samples at a fresh seed. The seed that drew a picture
        // starts from the noise that drew it, and the model redraws it rather
        // than editing it — measured on 2026-09-25: four edits sampled at the
        // reference's seed came back as near-copies on every model file
        // tried; the same edit at a fresh seed was clean. Keyed on "this is an
        // edit", not on recognising the file: a re-attached download or a
        // renamed copy carries the same seed and no name to read it from
        // (found on review of #306). Enforced here rather than asked for,
        // because a text-to-image result tells the model its seed keeps the
        // composition.
        let mut reseeded = None;
        if !req.references.is_empty() {
            if let Some(asked) = input.get("seed").and_then(Value::as_u64) {
                while req.seed == asked {
                    req.seed = fresh_seed();
                }
                reseeded = Some(asked);
            }
        }
        // A call that starts invalidates any idle timer already armed, so a
        // `/free` cannot land while this job is loading or running.
        self.generation.fetch_add(1, Ordering::SeqCst);
        let started = Instant::now();
        let timeout = Duration::from_secs(self.cfg.timeout_secs);
        let outcome = self
            .backend
            .generate(&self.cfg, &req, ctx.cancel.as_ref(), timeout)
            .await;
        // Armed whatever the outcome: a job that failed or was cancelled
        // mid-graph has already loaded the models, and the memory it holds is
        // the reason the timer exists (found on review of #303). The counter
        // makes a timer armed by an earlier call a no-op.
        self.arm_unload();
        let bytes = match outcome {
            Ok(bytes) => bytes,
            Err(Failure::Cancelled) => {
                return Ok(ToolOutput::err(
                    "Cancelled — the generation was stopped and nothing was saved.",
                ))
            }
            Err(Failure::Other(e)) => {
                let reach = if e.chain().any(|c| c.is::<reqwest::Error>()) {
                    format!(
                        " Is the image server running at {}? The operator starts it; \
                         you cannot.",
                        self.cfg.url
                    )
                } else {
                    String::new()
                };
                return Ok(ToolOutput::err(format!(
                    "Image generation failed: {e:#}.{reach}"
                )));
            }
        };
        let path = match save(ctx, req.seed, &bytes).await {
            Ok(path) => path,
            Err(e) => {
                return Ok(ToolOutput::err(format!(
                    "The image was made but not saved: {e:#}"
                )))
            }
        };
        let secs = started.elapsed().as_secs();
        let size = match req.size {
            Some((w, h)) => format!("{w}×{h}"),
            None => "reference-shaped".to_string(),
        };
        let mut text = format!("image: {path}\n");
        if req.references.is_empty() {
            text.push_str(&format!(
                "Generated a {size} image in {secs} s (seed {}, {} steps) and saved it to {path} \
                 in the workspace. You cannot see it; the user can, so do not describe what it \
                 shows. To revise it, call image_generate again with an edited prompt and seed {} \
                 to keep the composition, or edit it by passing {path} in reference_images.",
                req.seed, req.steps, req.seed
            ));
        } else {
            let sources: Vec<&str> = req.references.iter().map(|r| r.path.as_str()).collect();
            text.push_str(&format!(
                "Edited {} into a {size} image in {secs} s (seed {}, {} steps) and saved it to \
                 {path} in the workspace; the original is unchanged. You cannot see it; the user \
                 can, so do not describe what it shows. To change it further, edit {path} next.",
                sources.join(", "),
                req.seed,
                req.steps
            ));
        }
        if let Some(asked) = reseeded {
            text.push_str(&format!(
                " (Seed {asked} was not used: an edit always starts from a fresh seed, because \
                 the seed that drew a picture redraws it instead of editing it. Seed {} was \
                 used.)",
                req.seed
            ));
        }
        Ok(ToolOutput::ok(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake-pixels";

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
        fake_running(history, prompt_status, true).await
    }

    /// As [`fake`], with `GET /queue` reporting `job-1` as running or not —
    /// queued behind someone else's job when `running` is false.
    async fn fake_running(
        history: Vec<Value>,
        prompt_status: &'static str,
        running: bool,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let history = Arc::new(Mutex::new(std::collections::VecDeque::from(history)));
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let log = Arc::clone(&log);
                let history = Arc::clone(&history);
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
                            "UnetLoaderGGUF" => files("unet_name", "Qwen-Image-2.1-Q4.gguf"),
                            "CLIPLoader" => files("clip_name", "qwen3vl_8b_w4a8.safetensors"),
                            "VAELoader" => files("vae_name", "qwen_image_2.1_vae_bf16.safetensors"),
                            _ => json!({node: {"input": {}}}),
                        })
                    } else if path == "/prompt" {
                        reply(
                            prompt_status,
                            "application/json",
                            json!({"prompt_id": "job-1", "error": {"message": "bad node"}})
                                .to_string()
                                .as_bytes(),
                        )
                    } else if path.starts_with("/history/") {
                        let next = history.lock().unwrap().pop_front().unwrap_or(json!({}));
                        if next == json!("fail") {
                            reply("500 Internal Server Error", "text/plain", b"stalled")
                        } else {
                            json_reply(next)
                        }
                    } else if path.starts_with("/view?") {
                        reply("200 OK", "image/png", PNG)
                    } else if path == "/upload/image" {
                        json_reply(json!({"name": "up.png", "subfolder": "", "type": "temp"}))
                    } else if line.starts_with("GET /queue") {
                        let now = if running { "job-1" } else { "someone-else" };
                        json_reply(
                            json!({"queue_running": [[0, now, {}, {}, []]], "queue_pending": []}),
                        )
                    } else {
                        json_reply(json!({}))
                    };
                    let _ = sock.write_all(&out).await;
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

    fn tool(url: &str) -> ImageGenerate {
        ImageGenerate::new(ImageConfig {
            url: url.into(),
            min_available_mb: 0,
            unload_after_secs: 0,
            ..Default::default()
        })
        .unwrap()
        .polling_every(Duration::from_millis(10))
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
        // And the schema has nowhere to put a destination. `reference_images`
        // names *sources*, and each goes through the path jail: reading a
        // workspace file into a loopback server sends nothing anywhere.
        let schema = t.input_schema();
        let props = schema["properties"].as_object().unwrap();
        let mut keys: Vec<_> = props.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "negative_prompt",
                "prompt",
                "reference_images",
                "seed",
                "size"
            ]
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
        };
        let g = comfy_graph(&cfg, &req, &[]);
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
                "UnetLoaderGGUF",
                "VAEDecode",
                "VAELoader",
            ]
        );
        assert_eq!(g["encode"]["inputs"]["prompt"], hostile);
        assert_eq!(g["sample"]["inputs"]["seed"], 7);
        assert_eq!(g["sample"]["inputs"]["cfg"], 1.0);
        assert_eq!(g["unet"]["inputs"]["unet_name"], cfg.diffusion_model);
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

    #[test]
    fn bad_input_is_named_back_to_the_model() {
        let t = tool("http://127.0.0.1:1");
        assert!(t.request(&json!({})).is_err());
        assert!(t.request(&json!({"prompt": "   "})).is_err());
        assert!(t.request(&json!({"prompt": "x", "size": "huge"})).is_err());
        assert!(t.request(&json!({"prompt": "x", "seed": -1})).is_err());
        assert!(t
            .request(&json!({"prompt": "x".repeat(PROMPT_CAP + 1)}))
            .is_err());
        assert!(t
            .request(&json!({"prompt": "x", "reference_images": "images/a.png"}))
            .is_err());
        assert!(t
            .request(&json!({"prompt": "x", "reference_images": [1]}))
            .is_err());
        let five = vec!["images/a.png"; MAX_REFERENCES + 1];
        assert!(t
            .request(&json!({"prompt": "x", "reference_images": five}))
            .is_err());
        let (r, paths) = t
            .request(&json!({"prompt": " a fox ", "size": "portrait", "seed": 3}))
            .unwrap();
        assert_eq!(
            (r.prompt.as_str(), r.size, r.seed, paths.len()),
            ("a fox", Some((768, 1344)), 3, 0)
        );
        assert_eq!(r.steps, 40);
        // No size: square for a new image, the reference's shape for an edit.
        let (r, _) = t.request(&json!({"prompt": "x"})).unwrap();
        assert_eq!(r.size, Some((1024, 1024)));
        let (r, paths) = t
            .request(&json!({"prompt": "x", "reference_images": ["inbox/me.jpg"]}))
            .unwrap();
        assert_eq!((r.size, paths), (None, vec!["inbox/me.jpg".to_string()]));
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
        };
        let g = comfy_graph(&cfg, &req, &["a.png".into(), "b.jpg".into()]);
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
        let g = comfy_graph(&cfg, &sized, &["a.png".into()]);
        assert_eq!(g["sample"]["inputs"]["latent_image"], json!(["latent", 0]));
        assert_eq!(g["latent"]["inputs"]["width"], 1344);
        // And text-to-image has no references, no VAE on the encoder.
        let plain = Request {
            size: Some((1024, 1024)),
            ..sized
        };
        let g = comfy_graph(&cfg, &plain, &[]);
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

    #[tokio::test]
    async fn a_finished_job_lands_in_the_run_workspace() {
        let (url, seen) = fake(vec![json!({}), done()], "200 OK").await;
        let dir = tempdir();
        let out = tool(&url)
            .call(json!({"prompt": "a fox", "seed": 7}), &ctx(&dir))
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

        let seen = seen.lock().unwrap().clone();
        let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
        assert!(submitted.contains("\"prompt\":\"a fox\""), "{submitted}");
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
            .call(json!({"prompt": "a", "seed": 1}), &ctx(&dir))
            .await
            .unwrap();
        let b = t
            .call(json!({"prompt": "b", "seed": 1}), &ctx(&dir))
            .await
            .unwrap();
        let first = |o: &ToolOutput| o.content.lines().next().unwrap().to_string();
        assert!(!a.is_error && !b.is_error);
        assert_ne!(first(&a), first(&b));
        assert_eq!(std::fs::read_dir(dir.join("images")).unwrap().count(), 2);
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
        let call = tokio::spawn(async move { t.call(json!({"prompt": "a fox"}), &c).await });
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
        assert!(!dir.join("images").exists(), "nothing saved");
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn cancelling_a_queued_job_never_interrupts_the_running_one() {
        // `job-1` is queued behind another call's job. Cancelling it must take
        // it off the queue and leave the running job alone — an older server
        // ignores `/interrupt`'s body and would stop whatever is executing.
        let (url, seen) = fake_running(vec![], "200 OK", false).await;
        let dir = tempdir();
        let token = CancellationToken::new();
        let mut c = ctx(&dir);
        c.cancel = Some(token.clone());
        let t = tool(&url);
        let call = tokio::spawn(async move { t.call(json!({"prompt": "a fox"}), &c).await });
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
            .call(json!({"prompt": "a fox"}), &ctx(&dir))
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
            .call(json!({"prompt": "a fox", "seed": 7}), &ctx(&dir))
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
            .call(json!({"prompt": "a fox"}), &ctx(&dir))
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
            .call(json!({"prompt": "private words"}), &ctx(&dir))
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
                json!({"prompt": "Keep <image1> unchanged except: a sunset sky",
                       "reference_images": ["inbox/me.jpg"]}),
                &ctx(&dir),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("Edited inbox/me.jpg"),
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
                .call(
                    json!({"prompt": "x", "reference_images": [bad]}),
                    &ctx(&dir),
                )
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

    #[tokio::test]
    async fn an_edit_never_samples_at_the_seed_it_was_given() {
        // Wherever the reference came from: this tool's own result, or a
        // re-attached copy under a name that carries no seed at all.
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(dir.join("images/20260925-142604-7.png"), PNG).unwrap();
        std::fs::write(dir.join("inbox/download.png"), PNG).unwrap();
        for reference in ["images/20260925-142604-7.png", "inbox/download.png"] {
            let (url, seen) = fake(vec![done()], "200 OK").await;
            let out = tool(&url)
                .call(
                    json!({"prompt": "same fox, yellow raincoat", "seed": 7,
                           "reference_images": [reference]}),
                    &ctx(&dir),
                )
                .await
                .unwrap();
            assert!(!out.is_error, "{reference}: {}", out.content);
            assert!(
                out.content.contains("Seed 7 was not used"),
                "{}",
                out.content
            );
            let submitted = seen
                .lock()
                .unwrap()
                .iter()
                .find(|l| l.starts_with("POST /prompt"))
                .cloned()
                .unwrap();
            assert!(
                !submitted.contains("\"seed\":7,"),
                "{reference} sampled at the seed it was given: {submitted}"
            );
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_finished_job_is_dropped_from_the_servers_history() {
        let (url, seen) = fake(vec![done()], "200 OK").await;
        let dir = tempdir();
        let out = tool(&url)
            .call(json!({"prompt": "a fox"}), &ctx(&dir))
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

    #[tokio::test]
    async fn a_rejected_job_says_why() {
        let (url, _) = fake(vec![], "400 Bad Request").await;
        let dir = tempdir();
        let out = tool(&url)
            .call(json!({"prompt": "a fox"}), &ctx(&dir))
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
            .call(json!({"prompt": "a fox"}), &ctx(&dir))
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
            .call(json!({"prompt": "a fox"}), &ctx(&dir))
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.contains("Is the image server running"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
