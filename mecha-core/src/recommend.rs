//! What each model mecha runs costs, where it runs, and how it holds memory —
//! the rows `hardware.md`'s *Beside the chat model* table is generated from,
//! and the sum `mecha features --probe` reports (FEATURES-DESIGN.md §6,
//! §10.4).
//!
//! **One slot per model, not per feature.** A model is often needed by more
//! than one feature — the embedder by `graph`, `documents` and `personas` —
//! and is loaded once however many of them are on, so a row keyed by one
//! feature either under-counts (the feature that charges it is off while
//! another that uses it is on) or double-counts. A [`Slot`] names every
//! feature that needs it, and is counted once if any of them is shown.
//!
//! **Evidence on every figure** ([`Peak`]): a number without a source or a
//! source without a number cannot be written, and an unmeasured figure is
//! `null`, never zero. A figure measured on one machine is *arithmetic* on
//! any other.
//!
//! **No server is asked.** The probe reads `/proc/meminfo` (or `sysctl` on
//! macOS) and runs `nvidia-smi` for the card's memory — a process, not a
//! socket — so it can never start a socket-activated server or load a model
//! (§4.3).

use crate::feature::Feature;
use serde::Serialize;

/// One figure's evidence. `mb` is MiB throughout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "evidence", rename_all = "snake_case")]
pub enum Peak {
    Measured {
        mb: u32,
        machine: &'static str,
        date: &'static str,
    },
    /// From `hardware.md`'s formula, or measured figures added up or carried
    /// to a machine they were not measured on.
    Arithmetic { mb: u32 },
    /// No number at all — reported as null, never 0.
    Unmeasured,
}

impl Peak {
    pub fn mb(&self) -> Option<u32> {
        match self {
            Peak::Measured { mb, .. } | Peak::Arithmetic { mb } => Some(*mb),
            Peak::Unmeasured => None,
        }
    }

    /// The same number, as evidence on a machine it was not measured on.
    fn carried(&self) -> Peak {
        match self {
            Peak::Measured { mb, .. } => Peak::Arithmetic { mb: *mb },
            other => *other,
        }
    }
}

/// F5's two columns. `Unified`: one pool holds everything and the tier is
/// that pool. `Discrete`: the tier is the card's memory, and the cost is two
/// numbers, one per pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Memory {
    Unified { peak: Peak },
    Discrete { gpu: Peak, host: Peak },
}

/// How a slot holds its memory, which decides whether it is in the resident
/// sum or only the everything-loaded one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Residency {
    /// From start to stop.
    Resident,
    /// Nothing until the first request; freed after ten idle minutes.
    OnDemand,
    /// Loaded on use and released by an idle timer beside the server — image
    /// generation with `mecha-comfyui-idle-reset` installed.
    ReleasedOnIdle,
    /// Only while working.
    PerRequest,
}

impl Residency {
    pub fn word(self) -> &'static str {
        match self {
            Residency::Resident => "Resident",
            Residency::OnDemand => "On demand",
            Residency::ReleasedOnIdle => "Released on idle",
            Residency::PerRequest => "Per request",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunsOn {
    Gpu,
    Cpu,
}

impl RunsOn {
    pub fn word(self) -> &'static str {
        match self {
            RunsOn::Gpu => "GPU",
            RunsOn::Cpu => "CPU",
        }
    }
}

/// Where a pinned model comes from (§10.4). Every variant carries a hash or
/// commit a reviewer committed — GitHub's per-asset `digest` and Hugging
/// Face's `X-Linked-Etag` are how a pin is *authored*, never what is trusted
/// at install time — and a size, so the plan's download total is a sum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A release asset, per platform.
    ReleaseAsset {
        repo: &'static str,
        tag: &'static str,
        asset: &'static str,
        sha256: &'static str,
        bytes: u64,
    },
    /// A clone; `bytes` is the reviewed size of the checkout.
    GitCommit {
        url: &'static str,
        commit: &'static str,
        bytes: u64,
    },
    /// One repository at one revision, and every file the row needs from it.
    /// A row is never a projector-less model.
    HuggingFace {
        repo: &'static str,
        revision: &'static str,
        files: &'static [HubFile],
    },
    /// A `--require-hashes` lock, shipped in the binary.
    PythonLock { lock: &'static str, bytes: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HubFile {
    pub path: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

impl Source {
    pub fn bytes(&self) -> u64 {
        match self {
            Source::ReleaseAsset { bytes, .. }
            | Source::GitCommit { bytes, .. }
            | Source::PythonLock { bytes, .. } => *bytes,
            Source::HuggingFace { files, .. } => files.iter().map(|f| f.bytes).sum(),
        }
    }
}

/// One model at one tier and memory shape.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Recommendation {
    /// 16, 32, 64 or 128 — `hardware.md`'s tiers (F5).
    pub tier_gb: u32,
    pub memory: Memory,
    pub model: &'static str,
    /// What the figure counts, in the page's words.
    pub counts: &'static str,
    /// Empty when the model comes with its sidecar's own install (a Python
    /// package that fetches it, a model inside a wheel) — step 7d/7f's.
    pub sources: &'static [Source],
    /// Memory the figure leaves out and nothing measured, named so the probe
    /// can say what its sum is missing rather than present a partial sum as
    /// whole.
    pub excludes: Option<&'static str>,
}

/// A model mecha runs, and every feature that needs it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Slot {
    pub id: &'static str,
    pub label: &'static str,
    /// Empty for the chat model: every feature needs it.
    pub needed_by: &'static [Feature],
    pub runs_on: RunsOn,
    pub residency: Residency,
    pub rows: &'static [Recommendation],
}

const GB10: &str = "DGX Spark (GB10)";

/// GiB → MiB, for the arithmetic rows written from `hardware.md`'s GB figures.
const fn gb(tenths: u32) -> u32 {
    // tenths of a decimal GB, as MiB: 10^9 / 2^20 ≈ 953.674
    ((tenths as u64 * 100_000_000) / 1_048_576) as u32
}

const QWEN36_FILES: &[HubFile] = &[
    HubFile {
        path: "Qwen3.6-35B-A3B-UD-Q4_K_M.gguf",
        sha256: "0b21525e972670ed59e1812e170b27c26355381f0656ecc4e25617ece7dac58b",
        bytes: 22_663_387_424,
    },
    HubFile {
        path: "mmproj-BF16.gguf",
        sha256: "da63cb47a76763c712393f8a017070188a304fa39f8aeea6edc629ed7b975cfa",
        bytes: 902_822_528,
    },
];
const QWEN36: &[Source] = &[Source::HuggingFace {
    repo: "unsloth/Qwen3.6-35B-A3B-MTP-GGUF",
    revision: "5bc3e238d916f48a861bac2f8a1990a0e9b7e98d",
    files: QWEN36_FILES,
}];
const QWEN36_MODEL: &str = "Qwen3.6-35B-A3B Q4_K_M, with its vision projector";

/// The registry. Order is the page's row order.
pub const SLOTS: &[Slot] = &[
    Slot {
        id: "chat",
        label: "chat",
        needed_by: &[],
        runs_on: RunsOn::Gpu,
        residency: Residency::Resident,
        rows: &[
            // Measured on the router's uncensored arm of the same model and
            // quant (same architecture, same geometry) — what was resident.
            Recommendation {
                tier_gb: 128,
                memory: Memory::Unified {
                    peak: Peak::Measured { mb: 42_461, machine: GB10, date: "2026-10-02" },
                },
                model: QWEN36_MODEL,
                counts: "the server's GPU memory at four 262k slots",
                sources: QWEN36,
                excludes: Some(
                    "the chat server's process memory, and the router's prompt cache (`cache-ram`, up to 16 GiB)",
                ),
            },
            Recommendation {
                tier_gb: 128,
                memory: Memory::Discrete {
                    gpu: Peak::Arithmetic { mb: 42_461 },
                    host: Peak::Unmeasured,
                },
                model: QWEN36_MODEL,
                counts: "the GB10's measured GPU figure, carried to a card",
                sources: QWEN36,
                excludes: Some("the router's prompt cache (`cache-ram`, up to 16 GiB)"),
            },
            // 20.7 weights + 5.5 cache (one 256k slot) + 0.9 projector.
            Recommendation {
                tier_gb: 64,
                memory: Memory::Unified { peak: Peak::Arithmetic { mb: gb(271) } },
                model: QWEN36_MODEL,
                counts: "weights, one 256k slot's cache and the projector",
                sources: QWEN36,
                excludes: Some("the router's prompt cache (`cache-ram`)"),
            },
            Recommendation {
                tier_gb: 64,
                memory: Memory::Discrete {
                    gpu: Peak::Arithmetic { mb: gb(271) },
                    host: Peak::Unmeasured,
                },
                model: QWEN36_MODEL,
                counts: "weights, one 256k slot's cache and the projector",
                sources: QWEN36,
                excludes: Some("the router's prompt cache (`cache-ram`)"),
            },
            // 20.7 weights + 2.7 cache (128k) + 0.9 projector.
            Recommendation {
                tier_gb: 32,
                memory: Memory::Unified { peak: Peak::Arithmetic { mb: gb(243) } },
                model: QWEN36_MODEL,
                counts: "weights, a 128k cache and the projector",
                sources: QWEN36,
                excludes: Some("the router's prompt cache (`cache-ram`)"),
            },
            Recommendation {
                tier_gb: 32,
                memory: Memory::Discrete {
                    gpu: Peak::Arithmetic { mb: gb(243) },
                    host: Peak::Unmeasured,
                },
                model: QWEN36_MODEL,
                counts: "weights, a 128k cache and the projector",
                sources: QWEN36,
                excludes: Some("the router's prompt cache (`cache-ram`)"),
            },
            // No 16 GB row: the page names a class of model there (an 8B or a
            // 14B at Q4), not one anybody here has pinned or run.
        ],
    },
    Slot {
        id: "embeddings",
        label: "embeddings",
        needed_by: &[Feature::Graph, Feature::Documents, Feature::Personas],
        runs_on: RunsOn::Gpu,
        residency: Residency::OnDemand,
        rows: &[Recommendation {
            tier_gb: 128,
            memory: Memory::Unified {
                // 5,243 MiB GPU + 702 MiB process, read together.
                peak: Peak::Measured { mb: 5_945, machine: GB10, date: "2026-10-02" },
            },
            model: "harrier-oss-v1-0.6b f16, 32k context",
            counts: "loaded: GPU and process memory",
            sources: &[Source::HuggingFace {
                repo: "mradermacher/harrier-oss-v1-0.6b-GGUF",
                revision: "d79decec1ab9442e969e79804515b9c31683d30e",
                files: &[HubFile {
                    path: "harrier-oss-v1-0.6b.f16.gguf",
                    sha256: "f3af313d8f58b59f282bdf7299a6ad738efdcb0644e7a98ee6deb106336e717a",
                    bytes: 1_198_183_680,
                }],
            }],
            excludes: None,
        }],
    },
    Slot {
        id: "ocr",
        label: "OCR",
        needed_by: &[Feature::Ocr],
        runs_on: RunsOn::Gpu,
        residency: Residency::OnDemand,
        rows: &[Recommendation {
            tier_gb: 128,
            memory: Memory::Unified {
                // 2,641 MiB GPU + 888 MiB process, read together.
                peak: Peak::Measured { mb: 3_529, machine: GB10, date: "2026-10-02" },
            },
            model: "PaddleOCR-VL 1.6 (GGUF and projector)",
            counts: "loaded: GPU and process memory",
            sources: &[Source::HuggingFace {
                repo: "PaddlePaddle/PaddleOCR-VL-1.6-GGUF",
                revision: "511b09642bb324401f15f97cc23bc67e8f0a291d",
                files: &[
                    HubFile {
                        path: "PaddleOCR-VL-1.6-GGUF.gguf",
                        sha256: "f3ae46ec885050acf4b3d31944431e1fd90d50664fb09126af4a3c050ba14ee8",
                        bytes: 935_769_056,
                    },
                    HubFile {
                        path: "PaddleOCR-VL-1.6-GGUF-mmproj.gguf",
                        sha256: "204d757d7610d9b3faab10d506d69e5b244e32bf765e2bab2d0167e65e0a058a",
                        bytes: 881_770_560,
                    },
                ],
            }],
            excludes: None,
        }],
    },
    Slot {
        id: "layout",
        label: "layout",
        needed_by: &[Feature::Layout],
        runs_on: RunsOn::Cpu,
        residency: Residency::PerRequest,
        rows: &[Recommendation {
            tier_gb: 128,
            memory: Memory::Unified {
                // Recorded as "1.1 GB" peak RSS, base unstated; read as GiB.
                peak: Peak::Measured { mb: 1_126, machine: GB10, date: "2026-09-29" },
            },
            model: "PP-DocLayoutV3 (ONNX)",
            counts: "peak process memory",
            sources: &[Source::HuggingFace {
                repo: "PaddlePaddle/PP-DocLayoutV3_onnx",
                revision: "46bbdf188bb0a772c08aed74882ce7e51a8f1ea6",
                files: &[HubFile {
                    path: "inference.onnx",
                    sha256: "45bf71750b00739a41fc209f132eb104a4d6b5bb29483c9078164d8b87cf28ba",
                    bytes: 130_502_049,
                }],
            }],
            excludes: None,
        }],
    },
    Slot {
        id: "image",
        label: "image generation",
        needed_by: &[Feature::Image],
        runs_on: RunsOn::Gpu,
        residency: Residency::ReleasedOnIdle,
        rows: &[Recommendation {
            tier_gb: 128,
            memory: Memory::Unified {
                // ~1.1 GiB after the idle reset + ~18.5 GiB for a load from
                // cold (2026-10-02); ~15 warm, ~13.6 loaded and idle.
                peak: Peak::Arithmetic { mb: 20_070 },
            },
            model: "Qwen-Image 2.1 Q4, in ComfyUI",
            counts: "peak, a picture from cold: ~1.1 idle after the reset and ~18.5 to load; ~15 warm, ~13.6 loaded and idle",
            sources: &[
                Source::HuggingFace {
                    repo: "realrebelai/Qwen-Image-2.1_GGUFs",
                    revision: "8d393b750593a72ee040fe7d40611f479ccee679",
                    files: &[HubFile {
                        path: "Qwen-Image-2.1-Q4.gguf",
                        sha256: "51998ad7c068ce7d68e233237537900ffe874ab4d5c72e20758f5f18ceb15b8a",
                        bytes: 5_959_127_264,
                    }],
                },
                Source::HuggingFace {
                    repo: "Comfy-Org/Qwen-Image-2.1",
                    revision: "cb504a4090723e43f17ad01cec0359490e2de613",
                    files: &[
                        HubFile {
                            path: "text_encoders/qwen3vl_8b_w4a8.safetensors",
                            sha256: "7754425e55e7bea2bfde4dde59a4cc236cb44e5ee9c215ea66ef8d47012824eb",
                            bytes: 6_312_105_364,
                        },
                        HubFile {
                            path: "vae/qwen_image_2.1_vae_bf16.safetensors",
                            sha256: "bb21f7473051e1ac368515dd3f2e15cd44d7a11748ee8823e1ddca3e4876b7c9",
                            bytes: 675_509_688,
                        },
                    ],
                },
            ],
            excludes: None,
        }],
    },
    Slot {
        id: "stt",
        label: "speech to text",
        needed_by: &[Feature::Dictate, Feature::Calls],
        runs_on: RunsOn::Cpu,
        residency: Residency::Resident,
        rows: &[Recommendation {
            tier_gb: 128,
            memory: Memory::Unified {
                peak: Peak::Measured { mb: 685, machine: GB10, date: "2026-10-02" },
            },
            model: "Parakeet TDT 0.6B v3 int8",
            counts: "process memory",
            sources: &[Source::ReleaseAsset {
                repo: "k2-fsa/sherpa-onnx",
                tag: "asr-models",
                asset: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2",
                sha256: "5793d0fd397c5778d2cf2126994d58e9d56b1be7c04d13c7a15bb1b4eafb16bf",
                bytes: 487_170_055,
            }],
            excludes: None,
        }],
    },
    Slot {
        id: "tts",
        label: "speech",
        needed_by: &[Feature::Calls],
        runs_on: RunsOn::Gpu,
        residency: Residency::Resident,
        rows: &[Recommendation {
            tier_gb: 128,
            memory: Memory::Unified {
                // 5,512 MiB GPU + 2,506 MiB process, read together.
                peak: Peak::Measured { mb: 8_018, machine: GB10, date: "2026-10-02" },
            },
            model: "Chatterbox Turbo",
            counts: "GPU and process memory",
            // The chatterbox package fetches its own snapshot (step 7f).
            sources: &[],
            excludes: None,
        }],
    },
    Slot {
        id: "turn",
        label: "turn detection",
        needed_by: &[Feature::Calls],
        runs_on: RunsOn::Cpu,
        residency: Residency::Resident,
        rows: &[Recommendation {
            tier_gb: 128,
            memory: Memory::Unified {
                peak: Peak::Measured { mb: 465, machine: GB10, date: "2026-10-02" },
            },
            model: "Silero VAD and smart-turn v3, in the voice worker",
            counts: "the worker's process memory",
            // Both models ship inside the pipecat wheel (step 7d's lock).
            sources: &[],
            excludes: None,
        }],
    },
];

pub fn slot(id: &str) -> Option<&'static Slot> {
    SLOTS.iter().find(|s| s.id == id)
}

const TIERS: [u32; 4] = [16, 32, 64, 128];

fn gib(mb: u32) -> String {
    format!("{:.1}", mb as f64 / 1024.0)
}

/// `hardware.md`'s *Beside the chat model* table, generated from the 128 GB
/// unified rows — the page holds this text verbatim, and a test fails when
/// it does not.
pub fn beside_table() -> String {
    let mut out = String::from(
        "| Feature | Model | Runs on | How it holds memory | Cost on the GB10 (GiB) | What it counts | Evidence |\n\
         |---|---|---|---|---|---|---|\n",
    );
    for s in SLOTS {
        let Some(r) = s
            .rows
            .iter()
            .find(|r| r.tier_gb == 128 && matches!(r.memory, Memory::Unified { .. }))
        else {
            continue;
        };
        let Memory::Unified { peak } = r.memory else {
            unreachable!()
        };
        let who = if s.needed_by.is_empty() {
            "every feature".to_string()
        } else {
            s.needed_by
                .iter()
                .map(|f| format!("`{}`", f.id()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let (cost, evidence) = match peak {
            Peak::Measured { mb, date, .. } => (gib(mb), format!("Measured {date}")),
            Peak::Arithmetic { mb } => (format!("~{}", gib(mb)), "Arithmetic".to_string()),
            Peak::Unmeasured => ("—".to_string(), "Unmeasured".to_string()),
        };
        let counts = match r.excludes {
            Some(x) => format!("{}; not counted: {}", r.counts, x),
            None => r.counts.to_string(),
        };
        out.push_str(&format!(
            "| {} — {who} | {} | {} | {} | {cost} | {counts} | {evidence} |\n",
            s.label,
            r.model,
            s.runs_on.word(),
            s.residency.word(),
        ));
    }
    out
}

/// The machine's memory, as the probe reads it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Machine {
    /// One pool (a GB10, a Mac, or a computer with no card the probe can
    /// read). `gpu_unread` says the last case: a card may exist that only
    /// `nvidia-smi` could have described.
    Unified {
        total_mb: u64,
        gpu_unread: bool,
    },
    Discrete {
        gpu_mb: u64,
        host_mb: u64,
    },
}

impl Machine {
    /// The memory the tier is read from: the pool, or the card.
    fn tier_mb(&self) -> u64 {
        match self {
            Machine::Unified { total_mb, .. } => *total_mb,
            Machine::Discrete { gpu_mb, .. } => *gpu_mb,
        }
    }

    /// The largest tier the memory reaches, in decimal GB as sold — a GB10's
    /// 121.7 GiB is 130.7 GB, the 128 GB tier; a 24 GiB card is the 16 GB
    /// tier (a card between tiers takes the row at or below it).
    pub fn tier_gb(&self) -> Option<u32> {
        let bytes = self.tier_mb() * 1_048_576;
        TIERS
            .iter()
            .rev()
            .copied()
            .find(|t| bytes >= *t as u64 * 1_000_000_000)
    }

    /// Read without starting anything: `/proc/meminfo` or `sysctl`, and
    /// `nvidia-smi`'s total per card (`[N/A]` on a unified GB10).
    pub fn read() -> anyhow::Result<Machine> {
        let host_mb = host_total_mb()?;
        let gpu = nvidia_total_mb();
        Ok(match gpu {
            GpuRead::Cards(mb) => Machine::Discrete {
                gpu_mb: mb,
                host_mb,
            },
            GpuRead::Unified => Machine::Unified {
                total_mb: host_mb,
                gpu_unread: false,
            },
            GpuRead::None => Machine::Unified {
                total_mb: host_mb,
                gpu_unread: !cfg!(target_os = "macos"),
            },
        })
    }
}

enum GpuRead {
    Cards(u64),
    Unified,
    None,
}

fn nvidia_total_mb() -> GpuRead {
    let Ok(out) = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
        .output()
    else {
        return GpuRead::None;
    };
    if !out.status.success() {
        return GpuRead::None;
    }
    parse_nvidia_total(&String::from_utf8_lossy(&out.stdout))
}

fn parse_nvidia_total(text: &str) -> GpuRead {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return GpuRead::None;
    }
    let mut sum = 0u64;
    for l in &lines {
        match l.parse::<u64>() {
            Ok(mb) => sum += mb,
            // The GB10 answers `[N/A]`: its GPU has no memory of its own.
            Err(_) => return GpuRead::Unified,
        }
    }
    GpuRead::Cards(sum)
}

fn host_total_mb() -> anyhow::Result<u64> {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()?;
        let bytes: u64 = String::from_utf8_lossy(&out.stdout).trim().parse()?;
        return Ok(bytes / 1_048_576);
    }
    let text = std::fs::read_to_string("/proc/meminfo")?;
    parse_meminfo_total(&text).ok_or_else(|| anyhow::anyhow!("/proc/meminfo has no MemTotal"))
}

fn parse_meminfo_total(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// llmfit's endpoints (≤60 % Perfect, >98 % Too Tight); the band between is
/// mecha's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    Comfortable,
    Fits,
    Tight,
    TooTight,
}

impl Band {
    pub fn of(used_mb: u64, total_mb: u64) -> Band {
        let r = used_mb as f64 / total_mb.max(1) as f64;
        if r <= 0.60 {
            Band::Comfortable
        } else if r <= 0.85 {
            Band::Fits
        } else if r <= 0.98 {
            Band::Tight
        } else {
            Band::TooTight
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Band::Comfortable => "comfortable",
            Band::Fits => "fits",
            Band::Tight => "tight",
            Band::TooTight => "too tight",
        }
    }
}

/// One slot's line in the probe.
#[derive(Debug, Clone, Serialize)]
pub struct Line {
    pub slot: &'static str,
    pub label: &'static str,
    pub model: Option<&'static str>,
    pub residency: Residency,
    /// GPU pool (unified: the one pool).
    pub gpu: Peak,
    /// Host pool; `None` on a unified machine.
    pub host: Option<Peak>,
    /// The figure is the row for this tier and shape (`true`), or carried
    /// from another machine's row (`false`, and its evidence is arithmetic).
    pub own_row: bool,
    pub excludes: Option<&'static str>,
}

/// One pool's sum: `None` when any figure in it is unmeasured.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Sum {
    pub total_mb: u64,
    pub resident_mb: Option<u64>,
    pub loaded_mb: Option<u64>,
    pub band: Option<Band>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Budget {
    pub machine: Machine,
    pub tier_gb: Option<u32>,
    pub lines: Vec<Line>,
    /// Unified: the pool. Discrete: the card.
    pub gpu: Sum,
    /// Discrete only.
    pub host: Option<Sum>,
    /// Named, never folded into a sum.
    pub not_counted: Vec<String>,
}

/// The slots a set of shown features needs: the chat model always, and each
/// other slot once if any feature that needs it is shown.
pub fn needed(shown: &[Feature]) -> Vec<&'static Slot> {
    SLOTS
        .iter()
        .filter(|s| s.needed_by.is_empty() || s.needed_by.iter().any(|f| shown.contains(f)))
        .collect()
}

fn pick(
    slot: &'static Slot,
    machine: &Machine,
    tier: Option<u32>,
) -> Option<(&'static Recommendation, bool)> {
    let discrete = matches!(machine, Machine::Discrete { .. });
    let shape_ok = |r: &Recommendation| matches!(r.memory, Memory::Discrete { .. }) == discrete;
    if let Some(t) = tier {
        if let Some(r) = slot.rows.iter().find(|r| r.tier_gb == t && shape_ok(r)) {
            return Some((r, true));
        }
    }
    // No row of its own: the same model's nearest figure, carried. A chat
    // model has rows per tier and gets none below its smallest — the page
    // names no model at 16 GB.
    if slot.needed_by.is_empty() {
        return None;
    }
    slot.rows
        .iter()
        .max_by_key(|r| r.tier_gb)
        .map(|r| (r, false))
}

fn add(acc: Option<u64>, p: &Peak) -> Option<u64> {
    Some(acc? + p.mb()? as u64)
}

/// What everything shown would hold, pool by pool (§6). A pool with an
/// unmeasured figure sums to `None` — per pool, never both.
pub fn budget(machine: Machine, shown: &[Feature]) -> Budget {
    let tier = machine.tier_gb();
    let mut lines = Vec::new();
    let mut not_counted = Vec::new();
    for slot in needed(shown) {
        let Some((r, own)) = pick(slot, &machine, tier) else {
            lines.push(Line {
                slot: slot.id,
                label: slot.label,
                model: None,
                residency: slot.residency,
                gpu: Peak::Unmeasured,
                host: matches!(machine, Machine::Discrete { .. }).then_some(Peak::Unmeasured),
                own_row: false,
                excludes: None,
            });
            continue;
        };
        let (gpu, host) = match (machine, r.memory) {
            (Machine::Unified { .. }, Memory::Unified { peak }) => (peak, None),
            (Machine::Unified { .. }, Memory::Discrete { gpu, host }) => {
                // Both pools are one here.
                let p = match (gpu.mb(), host.mb()) {
                    (Some(g), Some(h)) => Peak::Arithmetic { mb: g + h },
                    _ => Peak::Unmeasured,
                };
                (p, None)
            }
            (Machine::Discrete { .. }, Memory::Discrete { gpu, host }) => (gpu, Some(host)),
            // A unified figure on a card: placed whole in the pool it runs
            // on, which is arithmetic until somebody measures the split.
            (Machine::Discrete { .. }, Memory::Unified { peak }) => match slot.runs_on {
                RunsOn::Gpu => (peak.carried(), Some(Peak::Arithmetic { mb: 0 })),
                RunsOn::Cpu => (Peak::Arithmetic { mb: 0 }, Some(peak.carried())),
            },
        };
        let (gpu, host) = if own {
            (gpu, host)
        } else {
            (gpu.carried(), host.map(|h| h.carried()))
        };
        if let Some(x) = r.excludes {
            not_counted.push(format!("{}: {x}", slot.label));
        }
        lines.push(Line {
            slot: slot.id,
            label: slot.label,
            model: Some(r.model),
            residency: slot.residency,
            gpu,
            host,
            own_row: own,
            excludes: r.excludes,
        });
    }
    let sum = |total_mb: u64, pick: &dyn Fn(&Line) -> Option<Peak>| {
        let mut resident = Some(0u64);
        let mut loaded = Some(0u64);
        for l in &lines {
            let Some(p) = pick(l) else { continue };
            loaded = add(loaded, &p);
            if l.residency == Residency::Resident {
                resident = add(resident, &p);
            }
        }
        Sum {
            total_mb,
            resident_mb: resident,
            loaded_mb: loaded,
            band: loaded.map(|l| Band::of(l, total_mb)),
        }
    };
    let (gpu, host) = match machine {
        Machine::Unified { total_mb, .. } => (sum(total_mb, &|l: &Line| Some(l.gpu)), None),
        Machine::Discrete { gpu_mb, host_mb } => (
            sum(gpu_mb, &|l: &Line| Some(l.gpu)),
            Some(sum(host_mb, &|l: &Line| l.host)),
        ),
    };
    Budget {
        machine,
        tier_gb: tier,
        lines,
        gpu,
        host,
        not_counted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_hex(s: &str, len: usize) -> bool {
        s.len() == len
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }

    /// The page and the registry are one table (§6: "generated from the rows,
    /// or checked against them"). On failure, paste the printed table over
    /// the page's.
    #[test]
    fn the_hardware_page_holds_the_registry_table() {
        let page = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../website/docs/getting-started/hardware.md");
        let text = std::fs::read_to_string(&page).unwrap();
        let table = beside_table();
        assert!(
            text.contains(&table),
            "hardware.md's *Beside the chat model* table is not the registry's — replace it with:\n\n{table}"
        );
    }

    /// Every pin is a pin: a 64-hex sha256 (or a 40-hex revision), and a
    /// size, so §10.2's plan can sum a download and 7a can verify one.
    #[test]
    fn every_source_is_pinned_and_sized() {
        for s in SLOTS {
            for r in s.rows {
                for src in r.sources {
                    assert!(src.bytes() > 0, "{}: a source with no size", s.id);
                    match src {
                        Source::ReleaseAsset { sha256, .. } => {
                            assert!(is_hex(sha256, 64), "{}", s.id)
                        }
                        Source::GitCommit { commit, .. } => assert!(is_hex(commit, 40), "{}", s.id),
                        Source::HuggingFace {
                            revision, files, ..
                        } => {
                            assert!(is_hex(revision, 40), "{}: revision", s.id);
                            assert!(!files.is_empty(), "{}: a repository with no files", s.id);
                            for f in *files {
                                assert!(is_hex(f.sha256, 64), "{}: {}", s.id, f.path);
                                assert!(f.bytes > 0, "{}: {}", s.id, f.path);
                            }
                        }
                        Source::PythonLock { lock, .. } => assert!(!lock.is_empty(), "{}", s.id),
                    }
                }
            }
        }
    }

    /// Every slot has the GB10 row the page is generated from, and every
    /// feature a slot names is a registry feature, so a renamed id cannot
    /// leave a model charged to nothing.
    #[test]
    fn every_slot_has_its_gb10_row_and_real_features() {
        let mut ids = std::collections::HashSet::new();
        for s in SLOTS {
            assert!(ids.insert(s.id), "two slots named {}", s.id);
            assert!(
                s.rows
                    .iter()
                    .any(|r| r.tier_gb == 128 && matches!(r.memory, Memory::Unified { .. })),
                "{} has no 128 GB unified row",
                s.id
            );
            for f in s.needed_by {
                assert!(Feature::ALL.contains(f));
            }
            for r in s.rows {
                assert!(TIERS.contains(&r.tier_gb), "{}: tier {}", s.id, r.tier_gb);
            }
        }
    }

    #[test]
    fn the_tier_is_read_in_gb_as_sold() {
        // A GB10: 121.7 GiB of MemTotal is the 128 GB tier.
        let gb10 = Machine::Unified {
            total_mb: 124_610,
            gpu_unread: false,
        };
        assert_eq!(gb10.tier_gb(), Some(128));
        // A 24 GiB card takes the row below it.
        let card = Machine::Discrete {
            gpu_mb: 24_576,
            host_mb: 65_536,
        };
        assert_eq!(card.tier_gb(), Some(16));
        let small = Machine::Unified {
            total_mb: 8_192,
            gpu_unread: false,
        };
        assert_eq!(small.tier_gb(), None);
    }

    #[test]
    fn nvidia_smi_reads_cards_or_says_unified() {
        assert!(matches!(parse_nvidia_total("[N/A]\n"), GpuRead::Unified));
        assert!(matches!(
            parse_nvidia_total("24576\n24576\n"),
            GpuRead::Cards(49_152)
        ));
        assert!(matches!(parse_nvidia_total(""), GpuRead::None));
        assert_eq!(
            parse_meminfo_total("MemTotal:       127600524 kB\nMemFree: 1 kB\n"),
            Some(124_609)
        );
    }

    /// A model several features need is counted once — and is counted
    /// whichever of them is the one that is on.
    #[test]
    fn a_shared_model_is_counted_once_whoever_needs_it() {
        let m = Machine::Unified {
            total_mb: 124_610,
            gpu_unread: false,
        };
        for shown in [
            &[Feature::Graph][..],
            &[Feature::Personas],
            &[Feature::Graph, Feature::Documents, Feature::Personas],
        ] {
            let b = budget(m, shown);
            assert_eq!(
                b.lines.iter().filter(|l| l.slot == "embeddings").count(),
                1,
                "{shown:?}"
            );
        }
        let none = budget(m, &[]);
        assert!(none.lines.iter().all(|l| l.slot == "chat"));
    }

    /// The GB10 with everything on: the resident sum is the resident rows,
    /// the loaded sum all of them, and what the figures leave out is named.
    #[test]
    fn the_gb10_sums_are_the_rows_and_name_what_they_leave_out() {
        let m = Machine::Unified {
            total_mb: 124_610,
            gpu_unread: false,
        };
        let b = budget(m, Feature::ALL);
        let resident: u64 = SLOTS
            .iter()
            .filter(|s| s.residency == Residency::Resident)
            .map(|s| match s.rows[0].memory {
                Memory::Unified { peak } => peak.mb().unwrap() as u64,
                _ => unreachable!(),
            })
            .sum();
        assert_eq!(b.gpu.resident_mb, Some(resident));
        assert!(b.gpu.loaded_mb.unwrap() > resident);
        assert!(b.host.is_none());
        assert!(b.not_counted.iter().any(|n| n.contains("prompt cache")));
        assert!(b.lines.iter().all(|l| l.own_row || l.slot != "chat"));
    }

    /// §6: on a card, two sums against two totals, and a `null` is per pool —
    /// an unmeasured host figure nulls the host sum and leaves the GPU's.
    #[test]
    fn on_a_card_an_unmeasured_host_nulls_only_the_host_sum() {
        let m = Machine::Discrete {
            gpu_mb: 32_768,
            host_mb: 65_536,
        };
        let b = budget(m, &[Feature::Ocr]);
        assert_eq!(b.tier_gb, Some(32));
        assert!(b.gpu.loaded_mb.is_some(), "the GPU sum must stand");
        let host = b.host.unwrap();
        assert_eq!(
            host.loaded_mb, None,
            "the chat row's host figure is unmeasured"
        );
        assert_eq!(host.band, None);
        // OCR has no card row of its own: carried, so arithmetic.
        let ocr = b.lines.iter().find(|l| l.slot == "ocr").unwrap();
        assert!(!ocr.own_row);
        assert!(matches!(ocr.gpu, Peak::Arithmetic { .. }));
    }

    /// The page names no chat model at 16 GB, so neither does the probe: the
    /// line is there, unmeasured, and the sum says it does not know.
    #[test]
    fn sixteen_gb_has_no_chat_row_and_the_sum_is_null() {
        let m = Machine::Unified {
            total_mb: 16_384,
            gpu_unread: false,
        };
        let b = budget(m, &[]);
        let chat = b.lines.iter().find(|l| l.slot == "chat").unwrap();
        assert_eq!(chat.model, None);
        assert_eq!(b.gpu.loaded_mb, None);
        assert_eq!(b.gpu.band, None);
    }

    #[test]
    fn bands_follow_llmfits_endpoints() {
        assert_eq!(Band::of(60, 100), Band::Comfortable);
        assert_eq!(Band::of(61, 100), Band::Fits);
        assert_eq!(Band::of(98, 100), Band::Tight);
        assert_eq!(Band::of(99, 100), Band::TooTight);
    }
}
