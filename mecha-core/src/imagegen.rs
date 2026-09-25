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
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub seed: u64,
}

const PROMPT_CAP: usize = 4_000;

/// Consecutive failed status polls before a job is abandoned.
const POLL_FAILURES: u32 = 3;

/// ComfyUI's graph for one text-to-image generation with Qwen-Image 2.1.
///
/// `PreviewImage` rather than `SaveImage`: the server writes to its temp
/// directory instead of keeping a second, permanent copy in `output/` — the
/// copy that matters is the one in the run's workspace.
pub fn comfy_graph(cfg: &ImageConfig, req: &Request) -> Value {
    json!({
        "unet": {"class_type": "UnetLoaderGGUF", "inputs": {"unet_name": cfg.diffusion_model}},
        "clip": {"class_type": "CLIPLoader", "inputs": {
            "clip_name": cfg.text_encoder, "type": "qwen_image", "device": "default"}},
        "vae": {"class_type": "VAELoader", "inputs": {"vae_name": cfg.vae}},
        "encode": {"class_type": "TextEncodeQwenImage21", "inputs": {
            "clip": ["clip", 0], "prompt": req.prompt, "negative_prompt": req.negative,
            "resolution": 1024}},
        "latent": {"class_type": "EmptyLatentImage", "inputs": {
            "width": req.width, "height": req.height, "batch_size": 1}},
        "sample": {"class_type": "KSampler", "inputs": {
            "model": ["unet", 0], "positive": ["encode", 0], "negative": ["encode", 1],
            "latent_image": ["latent", 0], "seed": req.seed, "steps": req.steps,
            // Guidance off: the reference setting. Above 1 doubles every step.
            "cfg": 1.0, "sampler_name": "euler", "scheduler": "simple", "denoise": 1.0}},
        "decode": {"class_type": "VAEDecode", "inputs": {"samples": ["sample", 0], "vae": ["vae", 0]}},
        "out": {"class_type": "PreviewImage", "inputs": {"images": ["decode", 0]}},
    })
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
            "Could not read available memory (/proc/meminfo), so the generation was not \
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

fn mem_available_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
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
    }

    async fn generate(
        &self,
        cfg: &ImageConfig,
        req: &Request,
        cancel: Option<&CancellationToken>,
        timeout: Duration,
    ) -> std::result::Result<Vec<u8>, Failure> {
        self.preflight(cfg).await?;
        let (status, body) = self
            .post_json(
                "prompt",
                &json!({"prompt": comfy_graph(cfg, req), "client_id": "mecha"}),
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
                    return Err(
                        anyhow!("the image server finished the job without an image").into(),
                    )
                }
                None => continue,
            }
        };

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
            .await
            .map_err(anyhow::Error::from)?;
        if !res.status().is_success() {
            return Err(anyhow!("fetching the finished image failed ({})", res.status()).into());
        }
        let bytes = res.bytes().await.map_err(anyhow::Error::from)?.to_vec();
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(anyhow!("the image server returned something that is not a PNG").into());
        }
        Ok(bytes)
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
        Ok(ImageGenerate {
            backend: Arc::new(ComfyUi {
                base,
                http: reqwest::Client::new(),
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

    fn request(&self, input: &Value) -> std::result::Result<Request, String> {
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
        let size = match input.get("size").and_then(Value::as_str) {
            None => Size::Square,
            Some(s) => Size::parse(s).ok_or_else(|| {
                format!("`size` must be square, landscape or portrait, not `{s}`.")
            })?,
        };
        let seed = match input.get("seed") {
            None | Some(Value::Null) => fresh_seed(),
            Some(v) => v
                .as_u64()
                .ok_or_else(|| "`seed` must be a whole number, zero or more.".to_string())?,
        };
        let (width, height) = size.dims();
        Ok(Request {
            prompt: prompt.to_string(),
            negative: negative.to_string(),
            width,
            height,
            steps: self.cfg.steps,
            seed,
        })
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
        "Generate an image from a text description with the local image model and save it \
         as a PNG in the workspace. Takes about a minute. It renders text inside images \
         well — put the exact words in quotes. You will not see the result; the user will. \
         To revise one, call again with an edited prompt and the same seed."
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
                    "description": "Default \"square\" (1024×1024). landscape is 1344×768, portrait 768×1344."
                },
                "seed": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "A seed from an earlier result keeps its composition while the prompt changes. Omit for a new image."
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
        let req = match self.request(&input) {
            Ok(req) => req,
            Err(why) => return Ok(ToolOutput::err(why)),
        };
        if let Err(why) = memory_verdict(mem_available_mb(), self.cfg.min_available_mb) {
            return Ok(ToolOutput::err(why));
        }
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
        Ok(ToolOutput::ok(format!(
            "image: {path}\n\
             Generated a {}×{} image in {} s (seed {}, {} steps) and saved it to {path} in the \
             workspace. You cannot see it; the user can, so do not describe what it shows. To \
             revise it, call image_generate again with an edited prompt and seed {} to keep \
             the composition.",
            req.width,
            req.height,
            started.elapsed().as_secs(),
            req.seed,
            req.steps,
            req.seed
        )))
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
        // And the schema has nowhere to put a destination.
        let schema = t.input_schema();
        let props = schema["properties"].as_object().unwrap();
        let mut keys: Vec<_> = props.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["negative_prompt", "prompt", "seed", "size"]);
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
            width: 1024,
            height: 1024,
            steps: 40,
            seed: 7,
        };
        let g = comfy_graph(&cfg, &req);
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
    fn bad_input_is_named_back_to_the_model() {
        let t = tool("http://127.0.0.1:1");
        assert!(t.request(&json!({})).is_err());
        assert!(t.request(&json!({"prompt": "   "})).is_err());
        assert!(t.request(&json!({"prompt": "x", "size": "huge"})).is_err());
        assert!(t.request(&json!({"prompt": "x", "seed": -1})).is_err());
        assert!(t
            .request(&json!({"prompt": "x".repeat(PROMPT_CAP + 1)}))
            .is_err());
        let r = t
            .request(&json!({"prompt": " a fox ", "size": "portrait", "seed": 3}))
            .unwrap();
        assert_eq!(
            (r.prompt.as_str(), r.width, r.height, r.seed),
            ("a fox", 768, 1344, 3)
        );
        assert_eq!(r.steps, 40);
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
