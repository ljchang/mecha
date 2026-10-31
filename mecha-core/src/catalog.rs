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

/// The same window at each of `tiers`, one slot.
const fn one_slot(ctx: u32) -> [TierGeometry; 3] {
    [
        TierGeometry {
            tier_gb: 32,
            ctx,
            slots: 1,
        },
        TierGeometry {
            tier_gb: 64,
            ctx,
            slots: 1,
        },
        TierGeometry {
            tier_gb: 128,
            ctx,
            slots: 1,
        },
    ]
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
const GEMMA_GEOMETRY: [TierGeometry; 3] = one_slot(32_768);

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

/// The registry, in the order the chip lists it. Every entry was loaded,
/// answered and drafted on the owner's Mac on 2026-10-19 from these pins
/// (design §4), from the GB10 router's presets of the same day.
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
    },
    Entry {
        id: "gemma-4-26b-a4b",
        provider: "gemma26",
        label: "Gemma 4 26B-A4B UD-Q4_K_M, with its vision projector and MTP draft file",
        note: "a 32k context — an eighth of the Qwen models' — and llama-server's own sampling",
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
        geometry: Geometry::PerTier(&GEMMA_GEOMETRY),
        // No sampling lines and no image floor, deliberately: the GB10 serves
        // it on llama-server's defaults, so the provider has no temperature.
        lines: &["spec-type = draft-mtp", "n-gpu-layers-draft = 999"],
        temperature: None,
        // An estimate, not measured: the GB10's note says 19–21 GiB in all.
        kv_kib_per_token: 64,
        state_mb_per_slot: 0,
    },
];

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
        assert!(s.contains("ctx-size = 32768\nparallel = 1\n"));
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
