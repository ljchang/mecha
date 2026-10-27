//! How a picture is sampled: the owner's choice for chat and per persona,
//! and what each choice left unset resolves to.
//!
//! The graph stays fixed in code (`imagegen::comfy_graph`); what the owner
//! may tune are the sampler's numbers and names, each from a closed range or
//! list, and whether the fixed negative prompt is sent. No model authors any
//! of it: the tool's schema has no such field, and the host stamps the
//! persona's settings onto the turn as it stamps the layers switch.
//!
//! Two layers (the owner, 2026-10-11): the chat's settings, in
//! `picture-settings.toml` under the mecha home, and a persona's, in its
//! `state.toml`. A field the persona leaves unset is the chat's
//! ([`Tuning::over`]), and one the chat leaves unset is the reference
//! setting every picture used before this existed (cfg 1, euler, simple,
//! the configured steps, no negative) — so with neither tuned, a picture
//! draws byte-for-byte as before.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Which diffusion model draws: the configured one, or its turbo
/// distillation. A closed enum written to a store is a wire format, and a
/// stored name this build does not know reads as **unset** — the layer
/// below, the chat's — never as a standard override: the stores are read
/// through [`Model::named`] (`Tuning::lenient`), and the settings routes
/// refuse an unknown name. `#[serde(other)]` serves only the derived
/// `Deserialize`, which no store reads (mecha-7e, 2026-10-11).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Model {
    Turbo,
    #[default]
    #[serde(other)]
    Standard,
}

impl Model {
    /// The closed set by name, and nothing else: a stored line this build
    /// does not know is unset — inherited — not a standard override.
    fn named(s: &str) -> Option<Model> {
        match s {
            "standard" => Some(Model::Standard),
            "turbo" => Some(Model::Turbo),
            _ => None,
        }
    }
}

/// The turbo model's file, as Comfy-Org ships it beside the standard one.
pub const TURBO_MODEL: &str = "qwen_image_2.1_turbo_int8_convrot.safetensors";
/// The standard model's file: int8 ConvRot, on native int8 kernels.
pub const STANDARD_MODEL: &str = "qwen_image_2.1_int8_convrot.safetensors";
/// The two files on a Mac: bf16, since MPS has no int8 kernels and emulating
/// them costs ~1.6x per step (the owner, 2026-10-13; FEATURES-DESIGN §12.3).
pub const TURBO_MODEL_MAC: &str = "qwen_image_2.1_turbo_bf16.safetensors";
pub const STANDARD_MODEL_MAC: &str = "qwen_image_2.1_bf16.safetensors";

/// The turbo model's file on this system.
pub fn turbo_model() -> &'static str {
    turbo_model_on(std::env::consts::OS)
}

/// The standard model's file on this system.
pub fn standard_model() -> &'static str {
    standard_model_on(std::env::consts::OS)
}

/// [`turbo_model`] on `os`.
pub fn turbo_model_on(os: &str) -> &'static str {
    if os == "macos" {
        TURBO_MODEL_MAC
    } else {
        TURBO_MODEL
    }
}

/// [`standard_model`] on `os`.
pub fn standard_model_on(os: &str) -> &'static str {
    if os == "macos" {
        STANDARD_MODEL_MAC
    } else {
        STANDARD_MODEL
    }
}

/// The turbo model's own setting, from its model card: few steps, no
/// guidance. 10-15 s a picture against ~35 s (mecha-a3, 2026-10-11).
pub const TURBO_STEPS: u32 = 8;

/// The guidance scale's range on the settings page. ComfyUI takes 0-100;
/// every measured or published value for Qwen-Image falls in 1-5, and below
/// half a point the difference is under seed noise (mecha-a3, 2026-10-11).
pub const CFG_MIN: f64 = 1.0;
pub const CFG_MAX: f64 = 5.0;
pub const CFG_STEP: f64 = 0.5;

/// The steps a persona may choose.
pub const STEPS_MIN: u32 = 4;
pub const STEPS_MAX: u32 = 60;

/// KSampler's samplers, read from `/object_info/KSampler` at ComfyUI
/// 88ab4a06 (2026-10-11). A name outside the list never reaches the graph,
/// and one the running server lacks is refused by `imagegen`'s preflight
/// naming the setting.
pub const SAMPLERS: &[&str] = &[
    "euler",
    "euler_cfg_pp",
    "euler_ancestral",
    "euler_ancestral_cfg_pp",
    "heun",
    "heunpp2",
    "exp_heun_2_x0",
    "exp_heun_2_x0_sde",
    "dpm_2",
    "dpm_2_ancestral",
    "lms",
    "dpm_fast",
    "dpm_adaptive",
    "dpmpp_2s_ancestral",
    "dpmpp_2s_ancestral_cfg_pp",
    "dpmpp_sde",
    "dpmpp_sde_gpu",
    "dpmpp_2m",
    "dpmpp_2m_cfg_pp",
    "dpmpp_2m_sde",
    "dpmpp_2m_sde_gpu",
    "dpmpp_2m_sde_heun",
    "dpmpp_2m_sde_heun_gpu",
    "dpmpp_3m_sde",
    "dpmpp_3m_sde_gpu",
    "ddpm",
    "lcm",
    "ipndm",
    "ipndm_v",
    "deis",
    "cfgpp_ud10_ab",
    "res_multistep",
    "res_multistep_cfg_pp",
    "res_multistep_ancestral",
    "res_multistep_ancestral_cfg_pp",
    "gradient_estimation",
    "gradient_estimation_cfg_pp",
    "er_sde",
    "seeds_2",
    "seeds_3",
    "sa_solver",
    "sa_solver_pece",
    "ddim",
    "uni_pc",
    "uni_pc_bh2",
];

/// KSampler's schedulers, read and checked the same way.
pub const SCHEDULERS: &[&str] = &[
    "simple",
    "sgm_uniform",
    "karras",
    "exponential",
    "ddim_uniform",
    "beta",
    "normal",
    "linear_quadratic",
    "kl_optimal",
];

/// The negative prompt, fixed in code: the owner turns it on or off, never
/// writes it, and no model does. Qwen-Image's published negative with the
/// duplicates mecha-a3 measured against added (2026-10-11). At cfg 1 the
/// sampler skips the unconditioned pass, so it has no effect there.
pub const NEGATIVE: &str = "Low resolution, poor image quality, distorted limbs, deformed \
    fingers, oversaturated colors, wax-figure appearance, lack of detail in faces, overly \
    smooth skin, AI-generated look, chaotic composition, blurry text, duplicate person, \
    extra people, extra limbs";

/// What the owner chose for the chat's pictures or a persona's. Every field
/// is optional, and only an unset one is left out of the file: a persona's
/// explicit `standard` or `negative = false` overrides a chat that chose
/// otherwise, so it must be written (mecha-7e, 2026-10-11).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tuning {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Model>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampler: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative: Option<bool>,
}

/// How one picture is sampled, every field decided: what the graph is
/// built from.
#[derive(Debug, Clone, PartialEq)]
pub struct Sampling {
    pub model: Model,
    pub steps: u32,
    pub cfg: f64,
    pub sampler: String,
    pub scheduler: String,
    /// The negative prompt sent, empty when off.
    pub negative: String,
}

impl Sampling {
    /// The reference setting at `steps`: every picture before tuning.
    pub fn reference(steps: u32) -> Sampling {
        Sampling {
            model: Model::Standard,
            steps,
            cfg: 1.0,
            sampler: "euler".into(),
            scheduler: "simple".into(),
            negative: String::new(),
        }
    }
}

impl Tuning {
    /// Each field in its range or list, or the whole is refused, naming the
    /// field: the settings page's door.
    pub fn checked(self) -> Result<Tuning> {
        if let Some(c) = self.cfg {
            if !c.is_finite() || !(CFG_MIN..=CFG_MAX).contains(&c) {
                bail!("cfg is {CFG_MIN} to {CFG_MAX}");
            }
            if ((c - CFG_MIN) / CFG_STEP).fract().abs() > 1e-9 {
                bail!("cfg moves in steps of {CFG_STEP}");
            }
        }
        if let Some(s) = self.steps {
            if !(STEPS_MIN..=STEPS_MAX).contains(&s) {
                bail!("steps are {STEPS_MIN} to {STEPS_MAX}");
            }
        }
        if let Some(s) = &self.sampler {
            if !SAMPLERS.contains(&s.as_str()) {
                bail!("no sampler named {s:?}");
            }
        }
        if let Some(s) = &self.scheduler {
            if !SCHEDULERS.contains(&s.as_str()) {
                bail!("no scheduler named {s:?}");
            }
        }
        Ok(self)
    }

    /// This persona's choices over `base`, the chat's: each field this one
    /// leaves unset is the chat's. A persona that picks a model other than
    /// the chat's takes none of the chat's numbers, which were chosen for
    /// the other model — turbo inheriting 30 guided steps would undo it.
    pub fn over(&self, base: &Tuning) -> Tuning {
        if self
            .model
            .is_some_and(|m| m != base.model.unwrap_or_default())
        {
            return self.clone();
        }
        Tuning {
            model: self.model.or(base.model),
            cfg: self.cfg.or(base.cfg),
            steps: self.steps.or(base.steps),
            sampler: self.sampler.clone().or_else(|| base.sampler.clone()),
            scheduler: self.scheduler.clone().or_else(|| base.scheduler.clone()),
            negative: self.negative.or(base.negative),
        }
    }

    /// Every field decided: the turbo model's own steps where the owner set
    /// none, the configured steps for the standard one, and the reference
    /// setting for the rest.
    pub fn resolve(&self, configured_steps: u32) -> Sampling {
        let model = self.model.unwrap_or_default();
        let base = Sampling::reference(match model {
            Model::Standard => configured_steps,
            Model::Turbo => TURBO_STEPS,
        });
        Sampling {
            model,
            steps: self.steps.unwrap_or(base.steps),
            cfg: self.cfg.unwrap_or(base.cfg),
            sampler: self.sampler.clone().unwrap_or(base.sampler),
            scheduler: self.scheduler.clone().unwrap_or(base.scheduler),
            negative: if self.negative == Some(true) {
                NEGATIVE.to_string()
            } else {
                String::new()
            },
        }
    }

    /// The fields of a stored table one at a time, each that does not read
    /// left unset: a bad line costs that line, never the persona's other
    /// settings — and an unset line inherits, so a damaged `model` is the
    /// chat's model, never a standard override (mecha-7e, 2026-10-11).
    pub fn lenient(table: &toml::Table) -> Tuning {
        let one = Tuning::default();
        let pick = |t: Tuning| t.checked().ok();
        Tuning {
            model: table
                .get("model")
                .and_then(toml::Value::as_str)
                .and_then(Model::named),
            cfg: table
                .get("cfg")
                .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
                .and_then(|c| {
                    pick(Tuning {
                        cfg: Some(c),
                        ..one.clone()
                    })
                })
                .and_then(|t| t.cfg),
            steps: table
                .get("steps")
                .and_then(toml::Value::as_integer)
                .and_then(|s| u32::try_from(s).ok())
                .and_then(|s| {
                    pick(Tuning {
                        steps: Some(s),
                        ..one.clone()
                    })
                })
                .and_then(|t| t.steps),
            sampler: table
                .get("sampler")
                .and_then(toml::Value::as_str)
                .and_then(|s| {
                    pick(Tuning {
                        sampler: Some(s.into()),
                        ..one.clone()
                    })
                })
                .and_then(|t| t.sampler),
            scheduler: table
                .get("scheduler")
                .and_then(toml::Value::as_str)
                .and_then(|s| {
                    pick(Tuning {
                        scheduler: Some(s.into()),
                        ..one.clone()
                    })
                })
                .and_then(|t| t.scheduler),
            negative: table.get("negative").and_then(toml::Value::as_bool),
        }
    }
}

/// The chat's picture settings: one file under the mecha home, written by
/// the owner's settings page and nothing else — no tool, no model.
pub fn chat_path(mecha_home: &std::path::Path) -> std::path::PathBuf {
    mecha_home.join("picture-settings.toml")
}

/// The chat's settings, read the way a persona's are: a missing file is
/// nothing chosen, a bad line costs that line. A file that cannot be read
/// at all is said, never taken for nothing chosen in silence.
pub fn chat(path: &std::path::Path) -> Tuning {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Tuning::default(),
        Err(e) => {
            tracing::warn!(
                "{} could not be read ({e}); pictures use the reference settings",
                path.display()
            );
            return Tuning::default();
        }
    };
    match toml::from_str::<toml::Table>(&text) {
        Ok(table) => Tuning::lenient(&table),
        Err(e) => {
            tracing::warn!(
                "{} is not TOML ({e}); pictures use the reference settings",
                path.display()
            );
            Tuning::default()
        }
    }
}

/// Write the chat's settings, held to their ranges and lists first. The
/// file's LoRA list is not a setting of this route and is written back as it
/// was (`docs/LORA-DESIGN.md` §6): a save from a locked page, which cannot
/// hold the list, must not clear it.
pub fn set_chat(path: &std::path::Path, tuning: Tuning) -> Result<Tuning> {
    let tuning = tuning.checked()?;
    let _one = CHAT_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut table = match toml::Value::try_from(&tuning)? {
        toml::Value::Table(t) => t,
        _ => toml::Table::new(),
    };
    // A file that is not TOML is replaced, as before the list shared it:
    // the chat's damaged list and an absent one both apply nothing. A file
    // that cannot be read is refused, so a transient error never writes the
    // owner's list away (review of #751, pass 4).
    let old = match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str::<toml::Table>(&text).ok(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => bail!("{} cannot be read ({e})", path.display()),
    };
    if let Some(v) = old.and_then(|t| t.get("loras").cloned()) {
        table.insert("loras".into(), v);
    }
    write_chat(path, &table)?;
    Ok(tuning)
}

/// The chat's picture LoRA list (`loras::Layer`), read like its settings:
/// a missing file has none. A file that does not read is a damaged list,
/// never an empty one, so the page and the result can say so.
pub fn chat_loras(path: &std::path::Path) -> crate::loras::Layer {
    match chat_table(path) {
        Ok(t) => crate::loras::Layer::read(t.as_ref().and_then(|t| t.get("loras"))),
        Err(_) => crate::loras::Layer::Damaged(toml::Value::Boolean(false)),
    }
}

/// Write the chat's picture LoRA list and nothing else. A settings file that
/// is not TOML is refused rather than replaced, since replacing it would
/// lose the owner's other settings in it.
pub fn set_chat_loras(path: &std::path::Path, layer: &crate::loras::Layer) -> Result<()> {
    let _one = CHAT_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut table = chat_table(path)?.unwrap_or_default();
    match layer.value() {
        Some(v) => table.insert("loras".into(), v),
        None => table.remove("loras"),
    };
    write_chat(path, &table)
}

/// The file's two writers, the settings route and the LoRA route, each
/// read it to keep the other's half: one at a time, or a save from each
/// pane at once could lose one half.
static CHAT_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The settings file as a table, `None` when there is none.
fn chat_table(path: &std::path::Path) -> Result<Option<toml::Table>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(toml::from_str(&text).map_err(|e| {
            anyhow::anyhow!(
                "{} is not TOML ({e}); fix or remove it first",
                path.display()
            )
        })?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => bail!("{} cannot be read ({e})", path.display()),
    }
}

fn write_chat(path: &std::path::Path, table: &toml::Table) -> Result<()> {
    let tmp = path.with_extension("toml.part");
    std::fs::write(&tmp, toml::to_string(table)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Roughly how long a picture takes on this machine, in seconds: a fixed
/// cost plus each step, and a step costs about twice as much once guidance
/// runs the unconditioned pass too. Fitted to mecha-a3's medians of
/// 2026-10-11 (standard 40 steps ~35 s, G3 30 steps at cfg 3 ~50 s, turbo
/// 8 steps 10-15 s); the settings page says "about".
pub fn estimate_secs(s: &Sampling) -> u32 {
    let per_step = if s.cfg > 1.0 { 1.6 } else { 0.8 };
    (5.0 + s.steps as f64 * per_step).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_untuned_persona_samples_as_every_picture_did_before() {
        assert_eq!(Tuning::default().resolve(40), Sampling::reference(40));
        let s = Tuning::default().resolve(40);
        assert_eq!(
            (s.cfg, s.sampler.as_str(), s.scheduler.as_str()),
            (1.0, "euler", "simple")
        );
        assert!(s.negative.is_empty());
    }

    #[test]
    fn turbo_takes_its_own_steps_unless_the_owner_set_some() {
        let turbo = Tuning {
            model: Some(Model::Turbo),
            ..Tuning::default()
        };
        assert_eq!(turbo.resolve(40).steps, TURBO_STEPS);
        let set = Tuning {
            steps: Some(12),
            ..turbo
        };
        assert_eq!(set.resolve(40).steps, 12);
    }

    #[test]
    fn the_negative_is_the_fixed_text_or_nothing() {
        let on = Tuning {
            negative: Some(true),
            ..Tuning::default()
        };
        assert_eq!(on.resolve(40).negative, NEGATIVE);
    }

    /// A persona's unset field is the chat's, its own wins, an explicit
    /// standard or no-negative overrides a chat that chose otherwise, and a
    /// persona on another model takes none of the chat's numbers (the
    /// owner, 2026-10-11).
    #[test]
    fn a_persona_inherits_the_chat_and_overrides_it() {
        let chat = Tuning {
            cfg: Some(3.0),
            steps: Some(30),
            sampler: Some("res_multistep".into()),
            negative: Some(true),
            ..Tuning::default()
        };
        assert_eq!(
            Tuning::default().over(&chat),
            chat,
            "nothing set: the chat's"
        );
        let own = Tuning {
            cfg: Some(2.0),
            negative: Some(false),
            ..Tuning::default()
        };
        let both = own.over(&chat);
        assert_eq!(
            (both.cfg, both.steps, both.negative),
            (Some(2.0), Some(30), Some(false))
        );
        assert!(both.resolve(40).negative.is_empty(), "an explicit off wins");
        let turbo = Tuning {
            model: Some(Model::Turbo),
            ..Tuning::default()
        };
        let t = turbo.over(&chat).resolve(40);
        assert_eq!((t.steps, t.cfg), (TURBO_STEPS, 1.0), "turbo's own numbers");
        // An explicit standard under a turbo chat is standard.
        let turbo_chat = Tuning {
            model: Some(Model::Turbo),
            ..Tuning::default()
        };
        let standard = Tuning {
            model: Some(Model::Standard),
            ..Tuning::default()
        };
        assert_eq!(
            standard.over(&turbo_chat).resolve(40).model,
            Model::Standard
        );
        assert_eq!(
            Tuning::default().over(&turbo_chat).resolve(40).model,
            Model::Turbo
        );
    }

    /// An explicit default is written, so it survives the round trip as an
    /// override rather than reading back as inherit (mecha-7e, 2026-10-11).
    #[test]
    fn an_explicit_default_is_written() {
        let t = Tuning {
            model: Some(Model::Standard),
            negative: Some(false),
            ..Tuning::default()
        };
        let text = toml::to_string(&t).unwrap();
        let back = Tuning::lenient(&toml::from_str(&text).unwrap());
        assert_eq!(back, t, "{text}");
        assert_eq!(toml::to_string(&Tuning::default()).unwrap().trim(), "");
    }

    /// The chat's file: missing is nothing chosen, a write is read back, and
    /// the door holds there too.
    #[test]
    fn the_chat_settings_round_trip() {
        let dir = std::env::temp_dir().join(format!("mecha-chatpic-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = chat_path(&dir);
        let missing = chat(&path);
        let g = Tuning {
            cfg: Some(3.0),
            negative: Some(true),
            ..Tuning::default()
        };
        let wrote = set_chat(&path, g.clone());
        let read = chat(&path);
        let refused = set_chat(
            &path,
            Tuning {
                cfg: Some(9.0),
                ..Tuning::default()
            },
        );
        let after = chat(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(missing, Tuning::default());
        assert_eq!(wrote.unwrap(), g);
        assert_eq!(read, g);
        assert!(refused.is_err());
        assert_eq!(after, g, "a refused write leaves the file");
    }

    #[test]
    fn the_door_refuses_what_the_graph_could_not_take() {
        let t = |f: fn(&mut Tuning)| {
            let mut t = Tuning::default();
            f(&mut t);
            t.checked()
        };
        assert!(t(|t| t.cfg = Some(3.0)).is_ok());
        assert!(t(|t| t.cfg = Some(0.5)).is_err());
        assert!(t(|t| t.cfg = Some(5.5)).is_err());
        assert!(t(|t| t.cfg = Some(2.2)).is_err(), "between steps");
        assert!(t(|t| t.cfg = Some(f64::NAN)).is_err());
        assert!(t(|t| t.steps = Some(3)).is_err());
        assert!(t(|t| t.steps = Some(61)).is_err());
        assert!(t(|t| t.sampler = Some("res_multistep".into())).is_ok());
        assert!(t(|t| t.sampler = Some("euler; rm".into())).is_err());
        assert!(t(|t| t.scheduler = Some("beta".into())).is_ok());
        assert!(t(|t| t.scheduler = Some("nope".into())).is_err());
    }

    #[test]
    fn a_bad_stored_line_costs_that_line_only() {
        let table: toml::Table = toml::from_str(
            "model = \"turbo\"\ncfg = 9\nsteps = 30\nsampler = \"made_up\"\nscheduler = \"beta\"\nnegative = true",
        )
        .unwrap();
        let t = Tuning::lenient(&table);
        assert_eq!(t.model, Some(Model::Turbo));
        assert_eq!(t.cfg, None, "out of range");
        assert_eq!(t.steps, Some(30));
        assert_eq!(t.sampler, None, "not in the list");
        assert_eq!(t.scheduler.as_deref(), Some("beta"));
        assert_eq!(t.negative, Some(true));
        // An integer cfg reads as its float, and a model this build does not
        // know is unset — inherited, never a standard override.
        let table: toml::Table = toml::from_str("model = \"hyper\"\ncfg = 3").unwrap();
        let t = Tuning::lenient(&table);
        assert_eq!((t.model, t.cfg), (None, Some(3.0)));
    }

    #[test]
    fn the_estimate_matches_the_measured_settings() {
        let at = |t: Tuning| estimate_secs(&t.resolve(40));
        assert_eq!(at(Tuning::default()), 37);
        let g3 = Tuning {
            cfg: Some(3.0),
            steps: Some(30),
            sampler: Some("res_multistep".into()),
            scheduler: Some("beta".into()),
            negative: Some(true),
            ..Tuning::default()
        };
        assert_eq!(at(g3), 53);
        assert_eq!(
            at(Tuning {
                model: Some(Model::Turbo),
                ..Tuning::default()
            }),
            11
        );
    }

    /// The chat's settings file holds its LoRA list too, and each route
    /// writes only its own half: a settings save keeps the list, and a list
    /// save keeps the settings. A file that is not TOML is refused by the
    /// list's route rather than replaced.
    #[test]
    fn the_chat_file_keeps_each_routes_half() {
        use crate::loras::Layer;
        let dir = std::env::temp_dir().join(format!("mecha-chat-loras-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = chat_path(&dir);
        assert_eq!(chat_loras(&path), Layer::Inherit);
        set_chat_loras(&path, &Layer::Set(vec!["paint".into()])).unwrap();
        let guided = Tuning {
            cfg: Some(3.0),
            ..Tuning::default()
        };
        set_chat(&path, guided.clone()).unwrap();
        assert_eq!(chat_loras(&path), Layer::Set(vec!["paint".into()]));
        set_chat_loras(&path, &Layer::Set(vec![])).unwrap();
        assert_eq!(chat(&path), guided);
        assert_eq!(chat_loras(&path), Layer::Set(vec![]));
        std::fs::write(&path, "cfg = [").unwrap();
        assert!(matches!(chat_loras(&path), Layer::Damaged(_)));
        assert!(set_chat_loras(&path, &Layer::Set(vec![])).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "cfg = [");
        std::fs::remove_dir_all(dir).ok();
    }
}
