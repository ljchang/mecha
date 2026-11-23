//! The chat model catalog (`docs/MODEL-CATALOG-DESIGN.md`): a short list of
//! chat models compiled into mecha, each a whole serving decision — its
//! pinned files, its preset section, its provider fields and its memory —
//! installed onto mecha's router only when the owner approves the download.
//!
//! **Compiled in, never fetched (C1).** Every entry is a pin a reviewer
//! committed, in `recommend`'s shape: repository, revision, path, sha256 and
//! size per file. A new model is a pull request.
//!
//! **The record survives `mecha setup chat` (C5).** The ids installed are a
//! file beside the router's presets, one per line, and the presets are always
//! rendered as the base section followed by one section per recorded id
//! (`router_unit::install`), so changing the base model keeps the catalog.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::fetch::Cached;
use crate::install::Say;
use crate::recommend::{HubFile, Machine};

/// What a pinned file is to the preset that serves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// `model =`.
    Weights,
    /// `mmproj =`: the vision projector.
    Projector,
    /// `model-draft =`: a separate MTP draft file.
    Draft,
}

/// One pinned file and its role. Files of one entry may come from different
/// revisions — unsloth's Qwen3.8 projector was left at its first upload when
/// the weights were re-uploaded — so the revision is per file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Pin {
    pub repo: &'static str,
    pub revision: &'static str,
    pub file: HubFile,
    pub role: Role,
}

/// How the router is told to serve an entry, and so the tiers it is offered
/// at: one with no geometry is not offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Geometry {
    /// The production chat row's at each of `tiers`
    /// (`recommend::chat_geometry`), host prompt cache included: a variant
    /// of the production model is served exactly as it is.
    Production { tiers: &'static [u32] },
    /// `ctx-size` and `parallel` per tier: the GB10's at 128 GB, and a
    /// smaller window where the GB10's would ask more of the tier than the
    /// production model does there.
    PerTier(&'static [TierGeometry]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TierGeometry {
    pub tier_gb: u32,
    pub ctx: u32,
    pub slots: u32,
}

/// Qwen3.8-27B's: the GB10's 256k at 128 GB, and 128k at 64 GB, where 256k
/// (34 GiB) would ask more than production's 28 GiB. Dense at 27B, it is not
/// offered at 32 GB.
const QWEN38_GEOMETRY: &[TierGeometry] = &[
    TierGeometry {
        tier_gb: 64,
        ctx: 131_072,
        slots: 1,
    },
    TierGeometry {
        tier_gb: 128,
        ctx: 262_144,
        slots: 1,
    },
];
/// Gemma 4 26B-A4B's: production's per-slot window at every tier — 256k at
/// 64 and 128 GB, 128k at 32 GB — and the window it was trained to
/// (`gemma4.context_length` = 262144 in the GGUF). It used to be 32k
/// everywhere, a figure carried over from the retired standalone launcher and
/// never measured; in the driven pilot (2026-11-08) that window compacted on
/// every task and twice ended a run without one edit, re-reading files it had
/// already read. Only 5 of its 30 layers attend globally (the rest a
/// 1024-token sliding window), so the window is cheap: `kv_kib_per_token`.
const GEMMA_GEOMETRY: &[TierGeometry] = &[
    TierGeometry {
        tier_gb: 32,
        ctx: 131_072,
        slots: 1,
    },
    TierGeometry {
        tier_gb: 64,
        ctx: 262_144,
        slots: 1,
    },
    TierGeometry {
        tier_gb: 128,
        ctx: 262_144,
        slots: 1,
    },
];

/// Inkling 2 Mini's: the largest windows, in 8k steps, within the fit rule —
/// 240k at 128 GB, 96k at 64 GB, 72k at 32 GB. The window is dear for an MoE
/// this size: llama.cpp gives each of the 48 layer passes (24 layers, the 12
/// of `loop_config`'s core 6..18 run three times) a full-length cache, its
/// sliding-window layers included — `kv_kib_per_token` — so 256k at 128 GB
/// would ask 50,960 MiB against a budget of 49,783 (production's 45,258, to
/// a tenth). Trained to 1M; the window grows if llama.cpp windows those
/// layers.
const INKLING_GEOMETRY: &[TierGeometry] = &[
    TierGeometry {
        tier_gb: 32,
        ctx: 73_728,
        slots: 1,
    },
    TierGeometry {
        tier_gb: 64,
        ctx: 98_304,
        slots: 1,
    },
    TierGeometry {
        tier_gb: 128,
        ctx: 245_760,
        slots: 1,
    },
];

/// One catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Entry {
    /// The preset section, so the router's alias, so the provider's `model`.
    pub id: &'static str,
    /// The `[providers.<name>]` table `model add` writes for it.
    pub provider: &'static str,
    pub label: &'static str,
    /// What the owner should know before downloading it — said on the
    /// confirmation and beside the row.
    pub note: &'static str,
    pub pins: &'static [Pin],
    pub geometry: Geometry,
    /// The preset's own lines past model, projector, draft and geometry.
    pub lines: &'static [&'static str],
    /// The provider's `temperature`: the preset's `temp`, or none where the
    /// preset sets none — R4 refuses a disagreement (`router::sampling_mismatches`).
    pub temperature: Option<f64>,
    /// KV cache per token, KiB, for the memory arithmetic.
    pub kv_kib_per_token: u32,
    /// Constant state per slot, MiB (Qwen3.6's recurrent layers).
    pub state_mb_per_slot: u32,
    /// The oldest llama.cpp build that loads it, where that is newer than
    /// some engine mecha has shipped: `add` asks the router's engine before
    /// anything is fetched (`engine_refusal`). `None` is any engine mecha
    /// has pinned since the catalog began.
    pub min_engine_build: Option<u32>,
}

/// Qwen's thinking-mode sampling past `temp`, as the GB10 serves every Qwen
/// preset (`router_unit::presets_text`'s pinned lines).
macro_rules! qwen_sampling {
    () => {
        concat!(
            "top-p = 0.95\ntop-k = 20\nmin-p = 0.0\npresence-penalty = 0.0\n",
            "repeat-penalty = 1.0\nreasoning-budget = 4096\nreasoning-preserve = true"
        )
    };
}

const UNCENSORED_NOTE: &str =
    "an uncensored build: its refusals were trained out, so it answers what the base model would decline";

/// The registry, in the order the chip lists it. The first five were loaded,
/// answered and drafted on the owner's Mac on 2026-10-19 from these pins
/// (design §4), from the GB10 router's presets of the same day; later entries
/// say where they were measured.
pub const CATALOG: &[Entry] = &[
    Entry {
        id: "qwen3.6-35b-a3b-uncensored",
        provider: "local-uncensored",
        label: "Qwen3.6-35B-A3B uncensored (HauhauCS Aggressive), with the production model's MTP head",
        note: UNCENSORED_NOTE,
        pins: &[
            // HauhauCS's Q4_K_M (`bbef58c3…`) with unsloth's MTP head
            // (`0b21525e…`, the production row's weights) grafted on by
            // `scripts/mtp-graft.py` (#516). Built on the GB10 and on the
            // owner's Mac on 2026-10-19 to the same bytes, and hosted with
            // its provenance in the card, since HauhauCS ships no head.
            Pin {
                repo: "mecha-factory/Qwen3.6-35B-A3B-Uncensored-MTP-GGUF",
                revision: "caaaf5253c6b91d512b408df62afef354f03dc7f",
                file: HubFile {
                    path: "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q4_K_M-MTP.gguf",
                    sha256: "8290c5bae3c744a9654281358e45669e158a9af2f0a23ad2a939bcb84528204f",
                    bytes: 21_695_616_448,
                },
                role: Role::Weights,
            },
            Pin {
                repo: "HauhauCS/Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive",
                revision: "f12a584fecbeb5f20001130d8ecd66c9327ae685",
                file: HubFile {
                    path: "mmproj-Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-f16.gguf",
                    sha256: "c8e702344a81f8c226a914aa980ed6e1f604bce9374f1fed8e65c896908af414",
                    bytes: 899_283_072,
                },
                role: Role::Projector,
            },
        ],
        geometry: Geometry::Production { tiers: &[32, 64, 128] },
        // Two drafted tokens, not production's default: acceptance into this
        // model's prose was 0.58–0.66 at 2 against 0.47–0.52 at 3 (GB10).
        lines: &[
            "image-min-tokens = 1024",
            "spec-type = draft-mtp",
            "spec-draft-n-max = 2",
            "temp = 0.6",
            qwen_sampling!(),
        ],
        temperature: Some(0.6),
        kv_kib_per_token: 22,
        state_mb_per_slot: 64,
        min_engine_build: None,
    },
    Entry {
        id: "qwen3.8-27b",
        provider: "qwen38",
        label: "Qwen3.8-27B UD-Q4_K_XL (dense), with its vision projector",
        note: "dense, so every weight is read for every token: about a third of the production model's speed",
        pins: &[
            Pin {
                repo: "unsloth/Qwen3.8-27B-GGUF",
                revision: "4ca720788d1e01f1bff70c033e0d0028fd02e502",
                file: HubFile {
                    path: "Qwen3.8-27B-UD-Q4_K_XL.gguf",
                    sha256: "3f227079003add2511437e5b1e94812e363385225bf6a9b47b0054a72bc8b01e",
                    bytes: 17_559_178_144,
                },
                role: Role::Weights,
            },
            // The projector's own revision: unchanged when the weights were
            // re-uploaded, and not in the newer snapshot.
            Pin {
                repo: "unsloth/Qwen3.8-27B-GGUF",
                revision: "fe1e2a23d973adb629709749dc4f6756df66ef10",
                file: HubFile {
                    path: "mmproj-BF16.gguf",
                    sha256: "83ee4f4f205fa514161778c41df1ea14144faa0f713510893b63c2395f5c2d53",
                    bytes: 931_146_432,
                },
                role: Role::Projector,
            },
        ],
        geometry: Geometry::PerTier(QWEN38_GEOMETRY),
        lines: &[
            "image-min-tokens = 1024",
            "spec-type = draft-mtp",
            "spec-draft-n-max = 4",
            "temp = 1.0",
            qwen_sampling!(),
        ],
        temperature: Some(1.0),
        kv_kib_per_token: 64,
        state_mb_per_slot: 0,
        min_engine_build: None,
    },
    Entry {
        id: "qwen3.8-27b-uncensored",
        provider: "qwen38-uncensored",
        label: "Qwen3.8-27B uncensored (HauhauCS Aggressive Q4_K_P), MTP head in the file",
        note: "an uncensored build: its refusals were trained out, so it answers what the base model would decline; its reasoning tends to stay in English",
        pins: &[
            Pin {
                repo: "HauhauCS/Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-MTP-GGUF",
                revision: "993a5971fda8f30dd1b7eb2654792ba4415c7460",
                file: HubFile {
                    path: "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q4_K_P.gguf",
                    sha256: "ba36dc3c2b2ff5e0aa5d71092a8894546996a6a119ae391803dda07cdc08516d",
                    bytes: 17_923_393_664,
                },
                role: Role::Weights,
            },
            Pin {
                repo: "HauhauCS/Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-MTP-GGUF",
                revision: "993a5971fda8f30dd1b7eb2654792ba4415c7460",
                file: HubFile {
                    path: "mmproj-Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-BF16.gguf",
                    sha256: "5681b690bcb8eb10cd28d62d078cb4e01521a3ea4880a3fc7d54de72de2dd142",
                    bytes: 931_146_624,
                },
                role: Role::Projector,
            },
        ],
        geometry: Geometry::PerTier(QWEN38_GEOMETRY),
        lines: &[
            "image-min-tokens = 1024",
            "spec-type = draft-mtp",
            "spec-draft-n-max = 4",
            "temp = 1.0",
            qwen_sampling!(),
        ],
        temperature: Some(1.0),
        kv_kib_per_token: 64,
        state_mb_per_slot: 0,
        min_engine_build: None,
    },
    Entry {
        id: "qwen3.8-27b-abliterated",
        provider: "qwen38-abliterated",
        label: "Qwen3.8-27B abliterated (huihui-ai UD-Q4_K_XL), MTP head in the file",
        note: "an abliterated build: the refusal direction was removed from its weights, so it answers what the base model would decline",
        pins: &[
            Pin {
                repo: "huihui-ai/Huihui-Qwen3.8-27B-abliterated-GGUF",
                revision: "3f101cd22b7999228bbd5d79a33975414eb9758b",
                file: HubFile {
                    path: "Huihui-Qwen3.8-27B-abliterated-UD-Q4_K_XL.gguf",
                    sha256: "ebbc66b45cf36bf47dc052d560337ff047a8b4eef851c8919d83d623703b6aa4",
                    bytes: 17_378_626_464,
                },
                role: Role::Weights,
            },
            Pin {
                repo: "huihui-ai/Huihui-Qwen3.8-27B-abliterated-GGUF",
                revision: "3f101cd22b7999228bbd5d79a33975414eb9758b",
                file: HubFile {
                    path: "mmproj-model-bf16.gguf",
                    sha256: "c9a09064683620bea3d3bfed5d4462e1a97a7d2fff7e5045d6862a0a85eeb5b5",
                    bytes: 931_145_888,
                },
                role: Role::Projector,
            },
        ],
        geometry: Geometry::PerTier(QWEN38_GEOMETRY),
        lines: &[
            "image-min-tokens = 1024",
            "spec-type = draft-mtp",
            "spec-draft-n-max = 4",
            "temp = 1.0",
            qwen_sampling!(),
        ],
        temperature: Some(1.0),
        kv_kib_per_token: 64,
        state_mb_per_slot: 0,
        min_engine_build: None,
    },
    Entry {
        id: "gemma-4-26b-a4b",
        provider: "gemma26",
        label: "Gemma 4 26B-A4B UD-Q4_K_M, with its vision projector and MTP draft file",
        note: "production's own context window, and llama-server's own sampling",
        // One revision for all three: a draft from another revision
        // mis-drafts silently.
        pins: &[
            Pin {
                repo: "unsloth/gemma-4-26B-A4B-it-GGUF",
                revision: "c462057f7ed65ccdb7f7e0778fae67894d425d92",
                file: HubFile {
                    path: "gemma-4-26B-A4B-it-UD-Q4_K_M.gguf",
                    sha256: "34c746b1d50ab813e29cd46c4796e3f43c741901a582f93a67b55b9fc9687b35",
                    bytes: 16_947_539_744,
                },
                role: Role::Weights,
            },
            Pin {
                repo: "unsloth/gemma-4-26B-A4B-it-GGUF",
                revision: "c462057f7ed65ccdb7f7e0778fae67894d425d92",
                file: HubFile {
                    path: "mmproj-BF16.gguf",
                    sha256: "41926ed5f1403cf5add23b0684992805ea6f97253096132e769e65646b8cef9d",
                    bytes: 1_194_828_256,
                },
                role: Role::Projector,
            },
            Pin {
                repo: "unsloth/gemma-4-26B-A4B-it-GGUF",
                revision: "c462057f7ed65ccdb7f7e0778fae67894d425d92",
                file: HubFile {
                    path: "mtp-gemma-4-26B-A4B-it.gguf",
                    sha256: "6326fb9f5e487aa8dcdd313a091e3c67724cb2a666ec3b7d2895b5b26d93ed1b",
                    bytes: 461_766_816,
                },
                role: Role::Draft,
            },
        ],
        geometry: Geometry::PerTier(GEMMA_GEOMETRY),
        // No sampling lines and no image floor, deliberately: the GB10 serves
        // it on llama-server's defaults, so the provider has no temperature.
        lines: &["spec-type = draft-mtp", "n-gpu-layers-draft = 999"],
        temperature: None,
        // Measured on the GB10 (2026-11-08), its own preset's arguments, MTP
        // draft and projector loaded: 19,267 MiB of GPU at 32k, 24,195 MiB at
        // 256k — 4,928 MiB for 229,376 more tokens. Was 64, an estimate.
        kv_kib_per_token: 22,
        state_mb_per_slot: 0,
        min_engine_build: None,
    },
    // Measured on the GB10 only (2026-11-21/22), not on a Mac: the
    // `inkling2_loop` architecture is in llama.cpp from b12611, so the pinned
    // engine (`engine::PIN`) has to be that new for this entry to load.
    Entry {
        id: "inkling-2-mini",
        provider: "inkling2",
        label: "Inkling 2 Mini UD-Q4_K_XL (MoE, 3.4B active), with its vision projector",
        note: "Thinking Machines' MoE, with no draft: about half the production model's speed with its draft off, and a shorter window at each tier, since its context costs six times as much per token",
        // unsloth's mainline-layout file: the repository's root GGUFs are in
        // unsloth's own fork's layout, which mainline llama.cpp refuses.
        // `.gguf_file` is the upload's name; llama-server reads it as is.
        pins: &[
            Pin {
                repo: "unsloth/Inkling-2-Mini-GGUF",
                revision: "ae9327c5fbc7333116ba32eee78bb5d2bbefae4f",
                file: HubFile {
                    path: "Mainline_Support/Inkling-2-Mini-UD-Q4_K_XL.gguf_file",
                    sha256: "e64276656f072e9b2197a61723667cbdb71da66a716ffd966e572c21088f3cd3",
                    bytes: 17_629_760_736,
                },
                role: Role::Weights,
            },
            // BF16, the one `start-router.sh`'s `hub_mmproj` (`mmproj_in`)
            // picks when the revision ships BF16 and F16 side by side, as
            // this one does: pin the other and the router serves one
            // projector while `present()` checks another.
            Pin {
                repo: "unsloth/Inkling-2-Mini-GGUF",
                revision: "ae9327c5fbc7333116ba32eee78bb5d2bbefae4f",
                file: HubFile {
                    path: "mmproj-BF16.gguf",
                    sha256: "2c165c78790e78a3f45a2d1e9e931083e52ee69d45ff2a0dc9c281fdf1a666e7",
                    bytes: 641_805_760,
                },
                role: Role::Projector,
            },
        ],
        geometry: Geometry::PerTier(INKLING_GEOMETRY),
        // The model card's sampling — temperature 1.0, nothing truncated —
        // with one change: min-p 0.05. The card's is its evaluation setting,
        // and in long prose every tail token is kept and steers what
        // follows: a persona chat on it came back with wrong times, slipped
        // points of view and off-kilter idiom (2026-11-23). On that chat's
        // first turn, 4 seeds a setting, the tokens chosen that the model
        // gave under 1% were 47 in 1000 untruncated (35.8 on the base
        // weights, the rest the uncensored graft's), 0.4 at min-p 0.05, and
        // 1.3 under top-p 0.95 / top-k 20 (for scale, Qwen3.6 under its own
        // preset was 0.1); the prose followed the counts. min-p is the one
        // kept because it cuts relative to the top token, so a confident
        // step keeps every candidate it had.
        //
        // No draft: its own drafter (DSpark) is not in llama.cpp yet.
        // Through both open versions (#32218, and the rewrite at 8b4c3aae)
        // it decoded 5–14% faster on code and 5–15% slower on prose,
        // accepting 0.39 of three drafted tokens and 0.50 of two (GB10,
        // 2026-11-21 and -22). The template's own default effort (0.9,
        // "high") stands; a request sets `chat_template_kwargs.reasoning_effort`.
        //
        // `reasoning-preserve` is set, never left to the build, as for every
        // Qwen preset (`start-router.sh`'s `qwen_sampling`): the template
        // has no rule of its own and renders every `reasoning_content` it is
        // handed, so the flag alone decides whether earlier thinking stays in
        // the prompt — and llama.cpp flipped its default once already. Kept,
        // so each prompt is a prefix of the next. No `reasoning-budget`:
        // whether the build's forced end of thinking lands in Inkling's
        // `<|content_thinking|>` format is not measured. No
        // `image-min-tokens`: the floor is Qwen-VL's grounding fix.
        lines: &[
            "temp = 1.0",
            "top-p = 1.0",
            "top-k = 0",
            "min-p = 0.05",
            "reasoning-preserve = true",
        ],
        temperature: Some(1.0),
        // Measured on the GB10 (2026-11-21), weights and projector loaded:
        // 21,924 MiB of GPU at 32k, 51,244 MiB at 256k — 29,320 MiB for
        // 229,376 more tokens.
        kv_kib_per_token: 131,
        state_mb_per_slot: 0,
        // b12611: the first release with #32704.
        min_engine_build: Some(12_611),
    },
];

/// Why the router's engine here cannot load `entry`, or `None`. Asked of the
/// engine itself, by its `--version`, before anything is fetched: an entry
/// newer than the engine would otherwise download, restart the router, pass
/// its health check on the base model, and fail only when the owner picks it.
///
/// `running` is the engine the router's service names today
/// (`router_unit::running_engine`: the unit's drop-ins, or the launchd
/// plist), which is the one to ask: the service is written once, at `setup
/// chat`, and installing mecha's engine afterwards does not rewrite it —
/// that is `--adopt`'s move. Only where the service names none or cannot be
/// read — which `add` says — is the engine `Engine::resolve` would install
/// with asked instead.
///
/// A version that cannot be read is a refusal, never a pass — and a build
/// from source counts its own commits, not the release's number, so it reads
/// low and is sent to `--adopt` rather than trusted.
pub fn engine_refusal(
    entry: &Entry,
    m: &crate::sidecar::Machinery,
    running: Option<&Path>,
) -> Option<String> {
    let min = entry.min_engine_build?;
    let server = match running {
        Some(p) => p.to_path_buf(),
        None => match crate::llama_units::Engine::resolve(m) {
            Ok(e) => e.path().to_path_buf(),
            Err(e) => return Some(format!("{e:#}")),
        },
    };
    // Compared as files, as `engine_gate` asks "is this mecha's engine": a
    // unit may spell the same binary another way.
    let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    };
    let remedy = if same(&server, &crate::engine_gate::managed_binary(&m.mecha_home)) {
        "`mecha setup engine --upgrade` moves it to a newer release"
    } else {
        "`mecha setup engine --adopt` moves the router onto mecha's pinned engine"
    };
    let needs = format!("{} needs llama.cpp b{min} or newer", entry.id);
    let at = server.display();
    match crate::engine::build_of(&server) {
        Ok(Some(b)) if b >= min => None,
        Ok(Some(b)) => Some(format!(
            "{needs}, and the router's engine ({at}) is build {b} — {remedy}"
        )),
        Ok(None) => Some(format!(
            "{needs}, and the router's engine ({at}) names no build in its --version — {remedy}"
        )),
        Err(e) => Some(format!(
            "{needs}, and the router's engine could not be asked: {e:#} — {remedy}"
        )),
    }
}

/// The entry `id` names.
pub fn entry(id: &str) -> Option<&'static Entry> {
    CATALOG.iter().find(|e| e.id == id)
}

/// The tiers `entry` is offered at, smallest first.
pub fn tiers(entry: &Entry) -> Vec<u32> {
    let mut t: Vec<u32> = match entry.geometry {
        Geometry::Production { tiers } => tiers.to_vec(),
        Geometry::PerTier(rows) => rows.iter().map(|r| r.tier_gb).collect(),
    };
    t.sort_unstable();
    t
}

/// How the router serves `entry` at `tier`: `(ctx-size, parallel,
/// cache-ram)`. `None` where it is not offered.
pub fn geometry_at(entry: &Entry, tier: u32) -> Option<(u32, u32, Option<u32>)> {
    match entry.geometry {
        Geometry::Production { tiers } if tiers.contains(&tier) => {
            crate::recommend::chat_geometry(tier).map(|g| (g.ctx, g.slots, Some(g.cache_ram_mb)))
        }
        Geometry::Production { .. } => None,
        Geometry::PerTier(rows) => rows
            .iter()
            .find(|r| r.tier_gb == tier)
            .map(|r| (r.ctx, r.slots, None)),
    }
}

/// The bytes the router reads: the weights, the projector and any draft file.
fn served_bytes(entry: &Entry) -> u64 {
    entry.pins.iter().map(|p| p.file.bytes).sum()
}

/// Its memory at `tier`, MiB, by `recommend`'s arithmetic: the served files,
/// the KV cache over every slot's window, and each slot's constant state.
/// Arithmetic, never a measurement — what the plan calls it.
pub fn memory_mb(entry: &Entry, tier: u32) -> Option<u32> {
    let (ctx, slots, _) = geometry_at(entry, tier)?;
    let files = served_bytes(entry) / 1_048_576;
    let kv = u64::from(ctx) / 1024 * u64::from(entry.kv_kib_per_token);
    Some((files + kv + u64::from(slots) * u64::from(entry.state_mb_per_slot)) as u32)
}

/// The record of installed ids, beside the presets it renders into.
pub fn record_path(mecha_home: &Path) -> PathBuf {
    crate::router_unit::presets_path(mecha_home).with_file_name("catalog")
}

/// The ids installed, in the order they were added. A missing record is
/// none; an unreadable one is an error, not an empty catalog.
pub fn installed(mecha_home: &Path) -> Result<Vec<String>> {
    let path = record_path(mecha_home);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn record_text(ids: &[String]) -> String {
    let mut out = String::from(
        "# Chat models installed from mecha's catalog (`mecha model add`), one id per line.\n\
         # The router's presets are rendered from this: base section, then one per id.\n",
    );
    for id in ids {
        out.push_str(id);
        out.push('\n');
    }
    out
}

/// Add `id` to the record; already there, nothing changes.
pub fn record_add(mecha_home: &Path, id: &str) -> Result<()> {
    let mut ids = installed(mecha_home)?;
    if ids.iter().any(|i| i == id) {
        return Ok(());
    }
    ids.push(id.to_string());
    crate::llama_units::write_owned(
        mecha_home,
        crate::router_unit::ID,
        &record_path(mecha_home),
        &record_text(&ids),
        0o644,
    )
}

/// Take `id` out of the record. `false` when it was not there.
pub fn record_remove(mecha_home: &Path, id: &str) -> Result<bool> {
    let mut ids = installed(mecha_home)?;
    let before = ids.len();
    ids.retain(|i| i != id);
    if ids.len() == before {
        return Ok(false);
    }
    crate::llama_units::write_owned(
        mecha_home,
        crate::router_unit::ID,
        &record_path(mecha_home),
        &record_text(&ids),
        0o644,
    )?;
    Ok(true)
}

/// The files a preset names, where they are on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub model: PathBuf,
    pub mmproj: Option<PathBuf>,
    pub draft: Option<PathBuf>,
}

/// Where `entry`'s served files are, whether or not they are there yet: the
/// hub's snapshot paths.
pub fn paths(entry: &Entry, hub: &Path) -> Result<Paths> {
    let snap = |p: &Pin| crate::fetch::snapshot_path(hub, p.repo, p.revision, &p.file);
    let of = |role| entry.pins.iter().find(|p| p.role == role);
    Ok(Paths {
        model: snap(of(Role::Weights).context("a catalog entry with no weights")?),
        mmproj: of(Role::Projector).map(snap),
        draft: of(Role::Draft).map(snap),
    })
}

/// One preset section for `entry` at `tier`, its files at `paths`.
pub fn section_text(entry: &Entry, tier: u32, paths: &Paths) -> Result<String> {
    let (ctx, slots, cache_ram) = geometry_at(entry, tier)
        .with_context(|| format!("{} is not offered at the {tier} GB tier", entry.id))?;
    let mut out = format!("[{}]\nmodel = {}\n", entry.id, paths.model.display());
    if let Some(mm) = &paths.mmproj {
        out.push_str(&format!("mmproj = {}\n", mm.display()));
    }
    out.push_str(&format!("ctx-size = {ctx}\nparallel = {slots}\n"));
    if let Some(c) = cache_ram {
        out.push_str(&format!("cache-ram = {c}\n"));
    }
    if let Some(d) = &paths.draft {
        out.push_str(&format!("model-draft = {}\n", d.display()));
    }
    for line in entry.lines {
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

/// Whether every file `entry` serves is on disk at its pinned size — the
/// cheap check, which reads no bytes; the fetch verified them as they were
/// written. A plain file of the pinned size put in the hub by hand
/// (`Unverified`) counts: `fetch_hub_file` hash-checks one and keeps it where
/// it is rather than moving it into a blob, so demanding the blob here would
/// drop a section `model add` had just checked, on every add (review of #727).
/// From `setup chat` or `features enable --reinstall` nothing hashes it in
/// that run: a same-size file swapped into the owner's hub cache by hand
/// after the add would be served. The hub is the owner's, and hashing 20 GB
/// on every render is the cost the cheap check exists to avoid.
pub fn present(entry: &Entry, hub: &Path) -> Result<bool> {
    for p in entry.pins {
        if !on_disk(hub, p)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn on_disk(hub: &Path, p: &Pin) -> Result<bool> {
    Ok(matches!(
        crate::fetch::cached(hub, p.repo, p.revision, &p.file, false)?,
        Cached::Verified | Cached::Unverified
    ))
}

/// The sections for every recorded id, rendered for `tier`, and why any
/// recorded id was left out: unknown to this build, not offered at this
/// tier, or its files gone. A section naming a missing file would fail the
/// child's start at the first switch to it; left out, the record keeps it and
/// `mecha model add` again restores it.
///
/// A recorded id that is the base section's own name (`base_alias`, a brought
/// GGUF whose stem is a catalog id) is left out too: two sections of one
/// name, and renaming the owner's base would orphan the provider that names
/// it (review of #727).
pub fn sections(
    mecha_home: &Path,
    hub: &Path,
    tier: Option<u32>,
    base_alias: &str,
) -> Result<(String, Vec<String>)> {
    let mut text = String::new();
    let mut skipped = Vec::new();
    for id in installed(mecha_home)? {
        let Some(e) = entry(&id) else {
            skipped.push(format!("{id}: not in this build's catalog"));
            continue;
        };
        if id == base_alias {
            skipped.push(format!(
                "{id}: the router's base model already has that name"
            ));
            continue;
        }
        let Some(t) = tier else {
            skipped.push(format!(
                "{id}: this machine is below the smallest memory tier"
            ));
            continue;
        };
        if geometry_at(e, t).is_none() {
            skipped.push(format!("{id}: not offered at this machine's memory tier"));
            continue;
        }
        let tier = t;
        if !present(e, hub)? {
            skipped.push(format!(
                "{id}: its files are not all in place — `mecha model add {id}` fetches them"
            ));
            continue;
        }
        text.push('\n');
        text.push_str(&section_text(e, tier, &paths(e, hub)?)?);
    }
    Ok((text, skipped))
}

/// What the catalog offers this machine, one row per entry.
#[derive(Debug, Clone, Serialize)]
pub struct Offer {
    pub id: &'static str,
    pub provider: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    /// In the record: `model add` finished its download, or was interrupted
    /// after it (the router's own list says whether it serves it).
    pub installed: bool,
    /// Why it is not offered here, or `None` when it is.
    pub unfit: Option<String>,
    /// What is still to download, less what the cache holds.
    pub download_bytes: u64,
    /// Its memory at this machine's tier (`memory_mb`), MiB; `None` where it
    /// is not offered.
    pub memory_mb: Option<u32>,
}

/// Every entry as this machine sees it. Reads no file's bytes.
pub fn offers(mecha_home: &Path, hub: &Path, machine: &Machine) -> Result<Vec<Offer>> {
    let tier = machine.tier_gb();
    let recorded = installed(mecha_home)?;
    CATALOG
        .iter()
        .map(|e| {
            let unfit = match tier {
                None => Some("this machine is below the smallest memory tier".to_string()),
                Some(t) if geometry_at(e, t).is_none() => Some(format!(
                    "offered at the {} GB memory tiers; this machine is at {t} GB",
                    tiers(e)
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Some(_) => None,
            };
            let mut download = 0;
            // A hand-placed file is hashed at install, not fetched.
            for p in e.pins {
                if !on_disk(hub, p)? {
                    download += p.file.bytes;
                }
            }
            Ok(Offer {
                id: e.id,
                provider: e.provider,
                label: e.label,
                note: e.note,
                installed: recorded.iter().any(|r| r == e.id),
                memory_mb: tier.and_then(|t| memory_mb(e, t)),
                unfit,
                download_bytes: download,
            })
        })
        .collect()
}

/// An install in progress, for the chip to show while `model add` runs as
/// the web route's child. Written by the installer, read by `model list`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Progress {
    pub id: String,
    /// `fetching` or `restarting`.
    pub phase: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub pid: u32,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

fn progress_path(mecha_home: &Path) -> PathBuf {
    record_path(mecha_home).with_file_name("catalog-installing.json")
}

/// Write the progress record, replacing it in one rename.
pub fn progress_write(mecha_home: &Path, p: &Progress) -> Result<()> {
    let path = progress_path(mecha_home);
    std::fs::create_dir_all(path.parent().context("no parent")?)?;
    let tmp = path.with_extension("json.new");
    std::fs::write(&tmp, serde_json::to_vec(p)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// The install in progress, if its process is still alive. A record whose
/// writer is gone is an install that died: not reported as running, and
/// removed.
pub fn progress_read(mecha_home: &Path) -> Option<Progress> {
    let path = progress_path(mecha_home);
    let p: Progress = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    if crate::process_alive(p.pid) {
        Some(p)
    } else {
        let _ = std::fs::remove_file(&path);
        None
    }
}

pub fn progress_clear(mecha_home: &Path) {
    let _ = std::fs::remove_file(progress_path(mecha_home));
}

/// Fetch every file `entry` needs, verifying each against its pin.
/// `progress` sees the bytes fetched so far of the total still to fetch.
/// Nothing here touches the router: hours of download never hold a run (C6).
pub async fn fetch(
    entry: &Entry,
    hub: &Path,
    say: Say<'_>,
    progress: &mut dyn FnMut(&str, u64, u64),
) -> Result<()> {
    let mut todo = Vec::new();
    for p in entry.pins {
        match crate::fetch::cached(hub, p.repo, p.revision, &p.file, false)? {
            Cached::Verified => say(&format!("{}/{} is in the cache", p.repo, p.file.path)),
            Cached::Unverified => {
                // Put there by hand: hashed against the pin, not fetched —
                // and so not in the total the plan priced (`offers`). On a
                // hash miss `fetch_hub_file` fetches the pin after all; that
                // download is said and its bytes reported, not silent
                // (review of #727).
                say(&format!(
                    "checking {}/{} against its pin — if it is not the pinned file, the pinned \
                     one is downloaded ({:.1} GiB)",
                    p.repo,
                    p.file.path,
                    p.file.bytes as f64 / 1_073_741_824.0
                ));
                let bytes = p.file.bytes;
                crate::fetch::fetch_hub_file(hub, p.repo, p.revision, &p.file, &mut |n| {
                    progress("fetching", n, bytes)
                })
                .await
                .with_context(|| format!("checking {}/{}", p.repo, p.file.path))?;
            }
            _ => todo.push(p),
        }
    }
    let total: u64 = todo.iter().map(|p| p.file.bytes).sum();
    let mut before = 0u64;
    for p in todo {
        say(&format!(
            "fetching {}/{} ({:.1} GiB)",
            p.repo,
            p.file.path,
            p.file.bytes as f64 / 1_073_741_824.0
        ));
        progress("fetching", before, total);
        let base = before;
        crate::fetch::fetch_hub_file(hub, p.repo, p.revision, &p.file, &mut |n| {
            progress("fetching", base + n, total)
        })
        .await
        .with_context(|| format!("fetching {}/{}", p.repo, p.file.path))?;
        before += p.file.bytes;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(what: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mecha-catalog-{what}-{}", uuid::Uuid::new_v4()))
    }

    /// Ids are unique, are the router's aliases (so plain), and none is the
    /// production preset's, which the base section owns; provider names are
    /// unique too.
    #[test]
    fn every_entry_is_named_once_and_never_as_production() {
        let mut ids = std::collections::BTreeSet::new();
        let mut providers = std::collections::BTreeSet::new();
        for e in CATALOG {
            assert!(ids.insert(e.id), "{} twice", e.id);
            assert!(providers.insert(e.provider), "{} twice", e.provider);
            assert_ne!(e.id, crate::router_unit::PINNED_ALIAS);
            assert_ne!(e.provider, "local", "the default's own table");
            // A router alias and a TOML string as written: no quoting needed.
            assert!(
                e.id.bytes().all(|b| b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || b == b'.'
                    || b == b'-'),
                "{}",
                e.id
            );
        }
    }

    /// mecha's own engine loads every entry: an entry may ask more than an
    /// older install's engine, never more than the one mecha ships.
    #[test]
    fn the_pinned_engine_loads_every_entry() {
        for e in CATALOG {
            if let Some(min) = e.min_engine_build {
                assert!(crate::engine::PIN.build >= min, "{} needs b{min}", e.id);
            }
        }
        assert!(
            CATALOG.iter().any(|e| e.min_engine_build.is_some()),
            "the comparison is not vacuous"
        );
    }

    /// A machine whose router engine answers `--version` with `text`.
    #[cfg(unix)]
    fn machine_whose_engine_says(dir: &Path, text: &str) -> crate::sidecar::Machinery {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let server = bin.join("llama-server");
        std::fs::write(&server, format!("#!/bin/sh\nprintf '%s\\n' '{text}' >&2\n")).unwrap();
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
        crate::sidecar::Machinery {
            home: dir.join("home"),
            mecha_home: dir.join("home/.mecha"),
            unit_dirs: vec![],
            path: vec![bin],
            docker: Box::new(|_| crate::sidecar::Lookup::Absent),
        }
    }

    /// `add` refuses an entry newer than the router's engine before it
    /// fetches anything, and refuses one whose engine's build it cannot read.
    #[cfg(unix)]
    #[test]
    fn an_engine_older_than_the_entry_is_refused_before_the_download() {
        let ink = entry("inkling-2-mini").unwrap();
        let gemma = entry("gemma-4-26b-a4b").unwrap();
        for (said, refused) in [
            ("version: 1.0.0-dev (build 12627, commit 225a5345b)", None),
            ("version: 1.0.0-dev (build 12611, commit 0123456)", None),
            (
                "version: 1.0.0-dev (build 12608, commit 93cb37ea)",
                Some("is build 12608"),
            ),
            // A build from source counts its own commits: it reads low.
            (
                "version: 1.0.0-dev (build 2371, commit c56be864)",
                Some("is build 2371"),
            ),
            ("version: 11205 (95887577)", Some("names no build")),
            ("", Some("names no build")),
        ] {
            let dir = scratch("engine");
            let m = machine_whose_engine_says(&dir, said);
            let got = engine_refusal(ink, &m, None);
            match refused {
                None => assert_eq!(got, None, "{said}"),
                Some(why) => {
                    let got = got.unwrap_or_else(|| panic!("{said} passed"));
                    assert!(got.contains(why), "{said}: {got}");
                    assert!(got.contains("b12611"), "{got}");
                    // Found on the PATH, not mecha's: the remedy is --adopt.
                    assert!(got.contains("--adopt"), "{got}");
                }
            }
            // An entry with no minimum never asks the engine.
            assert_eq!(engine_refusal(gemma, &m, None), None);
            let _ = std::fs::remove_dir_all(&dir);
        }
        // No engine at all is said, not passed.
        let dir = scratch("engine");
        let mut m = machine_whose_engine_says(&dir, "");
        m.path.clear();
        assert!(engine_refusal(ink, &m, None).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The engine asked is the one the router's unit runs, not the one an
    /// install would pick: mecha's own engine installed after `setup chat`
    /// does not move the router onto it until `--adopt`.
    #[cfg(unix)]
    #[test]
    fn the_engine_asked_is_the_one_the_router_runs() {
        use std::os::unix::fs::PermissionsExt;
        let ink = entry("inkling-2-mini").unwrap();
        let dir = scratch("engine");
        // The PATH's engine — and the router's — is old.
        let m = machine_whose_engine_says(&dir, "version: 1.0.0-dev (build 12000, commit 0123456)");
        let old = dir.join("bin/llama-server");
        // mecha's own is new, and installed.
        let managed = crate::engine_gate::managed_binary(&m.mecha_home);
        std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
        std::fs::write(
            &managed,
            "#!/bin/sh\nprintf '%s\\n' 'version: 1.0.0-dev (build 12627, commit 225a5345b)' >&2\n",
        )
        .unwrap();
        std::fs::set_permissions(&managed, std::fs::Permissions::from_mode(0o755)).unwrap();
        // With the unit unreadable, the install's choice — mecha's — passes.
        assert_eq!(engine_refusal(ink, &m, None), None);
        // The router's unit still names the old one: that is what is asked.
        let got = engine_refusal(ink, &m, Some(&old)).expect("the router's own engine is old");
        assert!(
            got.contains("is build 12000") && got.contains("--adopt"),
            "{got}"
        );
        // Named by the unit as mecha's own, it passes, and its remedy would
        // have been --upgrade.
        assert_eq!(engine_refusal(ink, &m, Some(&managed)), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Exactly one weights file per entry, at most one projector and draft;
    /// every pin a full sha256 and a size.
    #[test]
    fn every_pin_is_whole() {
        for e in CATALOG {
            let n = |r| e.pins.iter().filter(|p| p.role == r).count();
            assert_eq!(n(Role::Weights), 1, "{}", e.id);
            assert!(n(Role::Projector) <= 1 && n(Role::Draft) <= 1, "{}", e.id);
            for p in e.pins {
                assert_eq!(p.file.sha256.len(), 64, "{}", p.file.path);
                assert!(p.file.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
                assert_eq!(p.revision.len(), 40, "{}", p.file.path);
                assert!(p.file.bytes > 0);
            }
        }
    }

    /// A provider's temperature is its preset's `temp`, and none where the
    /// preset sets none — the disagreement R4 refuses a switch over.
    #[test]
    fn the_provider_temperature_is_the_preset_s() {
        for e in CATALOG {
            let temp = e
                .lines
                .iter()
                .flat_map(|l| l.lines())
                .find_map(|l| l.strip_prefix("temp = "))
                .map(|v| v.parse::<f64>().unwrap());
            assert_eq!(temp, e.temperature, "{}", e.id);
        }
    }

    /// Every offered tier has a geometry, and no entry asks more of a tier
    /// than the production chat model already does there (a tenth over, for
    /// a projector or draft file): the router holds one chat model, so a
    /// catalog model takes production's place, never more than its room.
    #[test]
    fn every_offered_tier_has_a_geometry_that_fits() {
        let production = |t: u32| {
            let g = crate::recommend::chat_geometry(t).unwrap();
            // The production row's weights and projector, from its pins.
            let files = u64::from(crate::recommend::QWEN36_FILES_MB);
            files + u64::from(g.ctx) / 1024 * 22 + 64 * u64::from(g.slots)
        };
        for e in CATALOG {
            for t in tiers(e) {
                assert!(geometry_at(e, t).is_some(), "{} at {t}", e.id);
                let mb = u64::from(memory_mb(e, t).unwrap());
                assert!(
                    mb <= production(t) * 11 / 10,
                    "{} at {t} GB: {mb} MiB",
                    e.id
                );
            }
            assert_eq!(geometry_at(e, 16), None);
        }
    }

    /// The GB10's router script and the catalog write gemma's preset twice,
    /// and #857 had to edit both by hand to move its window: every setting
    /// but the file paths must agree, or `model add` installs a different
    /// model than the script serves (the shape of
    /// `router_unit::tests::the_pinned_preset_is_the_script_s_production_section`).
    #[test]
    fn the_scripts_gemma_section_is_the_catalogs() {
        let script = include_str!("../../scripts/start-router.sh");
        let section = script
            .split_once("\n[gemma-4-26b-a4b]\n")
            .expect("the script's gemma section moved")
            .1;
        let section = &section[..section.find("\nEOF").unwrap()];
        let paths = ["model", "mmproj", "model-draft"];
        let settings = |text: &str| -> std::collections::BTreeMap<String, String> {
            text.lines()
                .filter_map(|l| l.split_once(" = "))
                .filter(|(k, _)| !paths.contains(&k.trim()))
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .collect()
        };
        let p = Paths {
            model: "/m/w.gguf".into(),
            mmproj: Some("/m/p.gguf".into()),
            draft: Some("/m/d.gguf".into()),
        };
        let ours = section_text(entry("gemma-4-26b-a4b").unwrap(), 128, &p).unwrap();
        assert_eq!(settings(section), settings(&ours));
        assert!(
            settings(section).contains_key("ctx-size"),
            "the comparison is not vacuous"
        );
    }

    /// The same agreement for Inkling, whose script section is a function
    /// both its presets share: every line but the file paths is the
    /// catalog's at 128 GB, the GB10's tier.
    #[test]
    fn the_scripts_inkling_section_is_the_catalogs() {
        let script = include_str!("../../scripts/start-router.sh");
        let body = script
            .split_once("\ninkling() {\n")
            .expect("the script's inkling function moved")
            .1;
        let body = &body[..body.find("\nEOF").unwrap()];
        let settings = |text: &str| -> std::collections::BTreeMap<String, String> {
            text.lines()
                .filter_map(|l| l.split_once(" = "))
                .filter(|(k, _)| !["model", "mmproj"].contains(&k.trim()))
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .collect()
        };
        let p = Paths {
            model: "/m/w.gguf".into(),
            mmproj: Some("/m/p.gguf".into()),
            draft: None,
        };
        let e = entry("inkling-2-mini").unwrap();
        let ours = section_text(e, 128, &p).unwrap();
        assert_eq!(settings(body), settings(&ours));
        assert!(
            settings(body).contains_key("ctx-size"),
            "the comparison is not vacuous"
        );
        // The paths themselves: the script looks up the pinned weights by
        // name, and its projector is `mmproj_in`'s first choice — which must
        // be the pinned one, since this revision ships two.
        let pinned = |r: Role| e.pins.iter().find(|p| p.role == r).unwrap().file.path;
        assert!(
            script.contains(&format!("hub_file \"$R\" {}", pinned(Role::Weights))),
            "the script serves other weights"
        );
        let mmproj = include_str!("../../scripts/mmproj.sh");
        let first = mmproj
            .split_once("for p in \"${1%/}/")
            .expect("mmproj_in's list moved")
            .1;
        let first = &first[..first.find('"').unwrap()];
        assert_eq!(
            first,
            pinned(Role::Projector),
            "the router would serve another projector"
        );
    }

    #[test]
    fn a_section_names_its_files_geometry_and_lines() {
        let e = entry("gemma-4-26b-a4b").unwrap();
        let p = Paths {
            model: "/m/w.gguf".into(),
            mmproj: Some("/m/p.gguf".into()),
            draft: Some("/m/d.gguf".into()),
        };
        let s = section_text(e, 128, &p).unwrap();
        assert!(s.starts_with("[gemma-4-26b-a4b]\nmodel = /m/w.gguf\nmmproj = /m/p.gguf\n"));
        assert!(s.contains("ctx-size = 262144\nparallel = 1\n"));
        assert!(s.contains("model-draft = /m/d.gguf\n"));
        assert!(!s.contains("cache-ram"));
        assert!(!s.contains("temp"));

        let u = entry("qwen3.6-35b-a3b-uncensored").unwrap();
        let s = section_text(u, 128, &p).unwrap();
        let g = crate::recommend::chat_geometry(128).unwrap();
        assert!(s.contains(&format!(
            "ctx-size = {}\nparallel = {}\ncache-ram = {}\n",
            g.ctx, g.slots, g.cache_ram_mb
        )));
        assert!(s.contains("spec-draft-n-max = 2\ntemp = 0.6\ntop-p = 0.95\n"));
        assert!(section_text(u, 16, &p).is_err());
    }

    #[test]
    fn the_record_keeps_order_and_ignores_comments() {
        let dir = scratch("record");
        let home = dir.as_path();
        crate::sidecar::Manifest::begin(home, crate::router_unit::ID).unwrap();
        assert!(installed(home).unwrap().is_empty());
        record_add(home, "gemma-4-26b-a4b").unwrap();
        record_add(home, "qwen3.8-27b").unwrap();
        record_add(home, "gemma-4-26b-a4b").unwrap();
        assert_eq!(installed(home).unwrap(), ["gemma-4-26b-a4b", "qwen3.8-27b"]);
        assert!(record_remove(home, "gemma-4-26b-a4b").unwrap());
        assert!(!record_remove(home, "gemma-4-26b-a4b").unwrap());
        assert_eq!(installed(home).unwrap(), ["qwen3.8-27b"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A recorded id whose files are gone, or that this build does not know,
    /// is left out of the presets and said — never a section naming a file
    /// that is not there.
    #[test]
    fn a_section_is_rendered_only_for_files_in_place() {
        let dir = scratch("sections");
        let home = dir.join("mecha");
        let hub = dir.join("hub");
        crate::sidecar::Manifest::begin(&home, crate::router_unit::ID).unwrap();
        record_add(&home, "gemma-4-26b-a4b").unwrap();
        record_add(&home, "a-model-from-a-newer-build").unwrap();
        let (text, skipped) = sections(&home, &hub, Some(128), "qwen3.6-35b-a3b").unwrap();
        assert!(text.is_empty());
        assert_eq!(skipped.len(), 2, "{skipped:?}");
        assert!(skipped[0].contains("mecha model add gemma-4-26b-a4b"));
        assert!(skipped[1].contains("not in this build's catalog"));

        // Files of the pinned size put in the hub by hand — which
        // `fetch_hub_file` hash-checks and leaves where they are — are in
        // place: the section is rendered (review of #727). Sparse, so the
        // test writes no 17 GB.
        let e = entry("gemma-4-26b-a4b").unwrap();
        for p in e.pins {
            let snap = crate::fetch::snapshot_path(&hub, p.repo, p.revision, &p.file);
            std::fs::create_dir_all(snap.parent().unwrap()).unwrap();
            std::fs::File::create(&snap)
                .unwrap()
                .set_len(p.file.bytes)
                .unwrap();
        }
        assert!(present(e, &hub).unwrap());
        let (text, skipped) = sections(&home, &hub, Some(128), "qwen3.6-35b-a3b").unwrap();
        assert!(text.contains("[gemma-4-26b-a4b]"), "{text}");
        assert_eq!(skipped.len(), 1, "{skipped:?}");
        // A base model of the same name keeps it; the catalog's is left out.
        let (text, skipped) = sections(&home, &hub, Some(128), "gemma-4-26b-a4b").unwrap();
        assert!(!text.contains("[gemma-4-26b-a4b]"), "{text}");
        assert!(skipped.iter().any(|s| s.contains("already has that name")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A progress record whose writer is gone reads as no install.
    #[test]
    fn a_dead_installer_s_progress_is_not_reported() {
        let dir = scratch("progress");
        let home = dir.as_path();
        let mut p = Progress {
            id: "qwen3.8-27b".into(),
            phase: "fetching".into(),
            done_bytes: 1,
            total_bytes: 2,
            pid: std::process::id(),
            started_at: chrono::Utc::now(),
        };
        progress_write(home, &p).unwrap();
        assert_eq!(progress_read(home), Some(p.clone()));
        p.pid = u32::MAX - 1;
        progress_write(home, &p).unwrap();
        assert_eq!(progress_read(home), None);
        assert!(!progress_path(home).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
