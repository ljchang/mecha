//! How a picture is sampled: the owner's choice per persona, and what each
//! choice left unset resolves to.
//!
//! The graph stays fixed in code (`imagegen::comfy_graph`); what the owner
//! may tune are the sampler's numbers and names, each from a closed range or
//! list, and whether the fixed negative prompt is sent. No model authors any
//! of it: the tool's schema has no such field, and the host stamps the
//! persona's settings onto the turn as it stamps the layers switch.
//!
//! The defaults are the reference setting every picture used before this
//! existed (cfg 1, euler, simple, the configured steps, no negative), so a
//! persona nobody tuned draws byte-for-byte as before.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Which diffusion model draws: the configured one, or its turbo
/// distillation. A value this build does not know reads as standard: a
/// closed enum written to a store is a wire format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Model {
    Turbo,
    #[default]
    #[serde(other)]
    Standard,
}

impl Model {
    fn is_standard(&self) -> bool {
        *self == Model::Standard
    }
}

/// The turbo model's file, as Comfy-Org ships it beside the standard one.
pub const TURBO_MODEL: &str = "qwen_image_2.1_turbo_int8_convrot.safetensors";

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

/// What the owner chose for a persona's pictures. Every unset field is the
/// model's own default ([`Tuning::resolve`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tuning {
    #[serde(default, skip_serializing_if = "Model::is_standard")]
    pub model: Model,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampler: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub negative: bool,
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

    /// Every field decided: the turbo model's own steps where the owner set
    /// none, the configured steps for the standard one, and the reference
    /// setting for the rest.
    pub fn resolve(&self, configured_steps: u32) -> Sampling {
        let base = Sampling::reference(match self.model {
            Model::Standard => configured_steps,
            Model::Turbo => TURBO_STEPS,
        });
        Sampling {
            model: self.model,
            steps: self.steps.unwrap_or(base.steps),
            cfg: self.cfg.unwrap_or(base.cfg),
            sampler: self.sampler.clone().unwrap_or(base.sampler),
            scheduler: self.scheduler.clone().unwrap_or(base.scheduler),
            negative: if self.negative {
                NEGATIVE.to_string()
            } else {
                String::new()
            },
        }
    }

    /// The fields of a stored table one at a time, each that does not read
    /// left unset: a bad line costs that line, never the persona's other
    /// settings.
    pub fn lenient(table: &toml::Table) -> Tuning {
        let one = Tuning::default();
        let pick = |t: Tuning| t.checked().ok();
        Tuning {
            model: table
                .get("model")
                .and_then(|v| v.clone().try_into::<Model>().ok())
                .unwrap_or_default(),
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
            negative: table
                .get("negative")
                .and_then(toml::Value::as_bool)
                .unwrap_or(false),
        }
    }
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
            model: Model::Turbo,
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
            negative: true,
            ..Tuning::default()
        };
        assert_eq!(on.resolve(40).negative, NEGATIVE);
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
        assert_eq!(t.model, Model::Turbo);
        assert_eq!(t.cfg, None, "out of range");
        assert_eq!(t.steps, Some(30));
        assert_eq!(t.sampler, None, "not in the list");
        assert_eq!(t.scheduler.as_deref(), Some("beta"));
        assert!(t.negative);
        // An integer cfg reads as its float, and a model this build does not
        // know reads as standard.
        let table: toml::Table = toml::from_str("model = \"hyper\"\ncfg = 3").unwrap();
        let t = Tuning::lenient(&table);
        assert_eq!((t.model, t.cfg), (Model::Standard, Some(3.0)));
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
            negative: true,
            ..Tuning::default()
        };
        assert_eq!(at(g3), 53);
        assert_eq!(
            at(Tuning {
                model: Model::Turbo,
                ..Tuning::default()
            }),
            11
        );
    }
}
