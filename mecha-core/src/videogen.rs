//! Local video generation: short clips with sound from MiniMax H3, rendered
//! by a ComfyUI on this machine.
//!
//! The rules are `imagegen`'s, for the same reasons:
//!
//! - **The model never authors the workflow.** The graph is fixed in code
//!   ([`graph::build`]); the model supplies typed values — a typed scene
//!   ([`scene::compile`]) or a prompt, a length, a shape — and they reach the
//!   graph as JSON string and number values, so nothing in them can become a
//!   node.
//! - **No egress, earned rather than asserted.** `[video] url` must be
//!   loopback ([`crate::comfy::loopback_url`], checked at load by
//!   `Config::validate`), and `[video]` loads from the global config only
//!   (`merge_file` strips it from a project layer): a cloned repository must
//!   not be able to choose where model-written prompts and the owner's
//!   pictures go.
//! - **The prompt is compiled, in two halves** ([`compose`]): the model
//!   writes what varies from clip to clip (the shot timeline, the speakers,
//!   the sound), and this code writes what persists — who each reference
//!   picture is, what of them is kept, whose voice each audio clip is — in
//!   the sectioned shape H3 was trained on.
//!
//! **Nothing here ships a model.** The file names below are defaults, the
//! names MiniMax's ComfyUI release gives them, and the operator brings the
//! files and their licence; a server without one is refused by name before
//! anything is uploaded (`graph::needs`).

use serde::{Deserialize, Serialize};

pub mod args;
pub mod compose;
pub mod graph;
pub mod loras;
pub mod scene;
pub mod tool;

pub use tool::{VideoGenerate, VideoPromptGuide};

/// `[video]`: where the local video server is and what it should load.
///
/// Absent means no video tool. Global-file only (see the module doc).
/// A clip's turbo pass is one LoRA on the diffusion model, paired with the
/// step count it was distilled for; full quality is [`VideoConfig::steps`]
/// without it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VideoConfig {
    /// The server's base URL. Must be loopback. Its own server, not
    /// `[image]`'s: a clip holds ~30 GB while it renders, and a picture
    /// should not queue behind one.
    pub url: String,
    /// The diffusion model for a clip from words, optionally pinned to a
    /// first or last frame (H3's `fl2va` checkpoint).
    pub frames_model: String,
    /// The diffusion model for a clip from reference pictures and voices
    /// (H3's `ref2va` checkpoint).
    pub references_model: String,
    /// The turbo LoRA for `frames_model`, and its steps.
    pub frames_turbo_lora: String,
    pub frames_turbo_steps: u32,
    /// The turbo LoRA for `references_model`, and its steps.
    pub references_turbo_lora: String,
    pub references_turbo_steps: u32,
    pub text_encoder: String,
    /// The video VAE: encodes the references and pinned frames, and decodes
    /// the clip unless `decoder` names another.
    pub vae: String,
    pub audio_vae: String,
    /// A VAE that decodes the clip in place of `vae`, used only when the
    /// server lists it. Unset by default: the one tried (a pruned H3
    /// decoder) cut a 5 s clip's decode from ~48 s to ~16 s, and the owner
    /// found its clips noticeably worse (2026-10-17).
    pub decoder: Option<String>,
    /// Sampling steps without turbo.
    pub steps: u32,
    /// The owner's own lessons for writing clips, a Markdown file appended to
    /// `video_prompt_guide`'s answer: what this machine's renders taught, kept
    /// out of the shipped guide. Read on each call, so an edit applies to the
    /// next clip.
    pub guide_addendum: Option<std::path::PathBuf>,
    /// The owner's LoRA registry ([`loras`]): entries the model may pick per
    /// clip, and ones on every clip. Read when the tool is built, since the
    /// names sit in the tool's schema; a restart picks up an edit.
    pub loras: Option<std::path::PathBuf>,
    /// The folders the video server loads LoRAs from (its `models/loras`,
    /// and any `extra_model_paths.yaml` adds), where each clip entry's files
    /// are looked for and checked (`loras`). Nothing installs the video
    /// server, so there is no default: unset, every clip LoRA is left out.
    pub loras_dir: Option<Vec<std::path::PathBuf>>,
}

impl Default for VideoConfig {
    fn default() -> Self {
        VideoConfig {
            url: "http://127.0.0.1:8189".into(),
            frames_model: "minimax_h3_fl2va_pruned_int8_convrot.safetensors".into(),
            references_model: "minimax_h3_ref2va_pruned_int8_convrot.safetensors".into(),
            // The turbo distillations' own pairing (Comfy-Org's H3
            // templates): the 8-step frames LoRA sampled at 6, the 4-step
            // references LoRA at 4.
            frames_turbo_lora: "minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors".into(),
            frames_turbo_steps: 6,
            references_turbo_lora: "minimax_h3_ref2v_turbo_4step_v0.1_comfyui_bf16.safetensors"
                .into(),
            references_turbo_steps: 4,
            text_encoder: "qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors".into(),
            vae: "minimax_h3_video_vae_fp16.safetensors".into(),
            audio_vae: "minimax_h3_audio_vae_fp32.safetensors".into(),
            decoder: None,
            steps: 20,
            guide_addendum: None,
            loras: None,
            loras_dir: None,
        }
    }
}

impl VideoConfig {
    /// The server's URL, refused off this machine.
    pub fn loopback_url(&self) -> anyhow::Result<reqwest::Url> {
        crate::comfy::loopback_url("[video] url", "the video tool", &self.url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults read back from a file as written, and a file that names
    /// only what it changes keeps the rest: the table is wholesale per layer
    /// (`ConfigLayer::apply`), so every default must survive a partial one.
    #[test]
    fn a_partial_table_keeps_the_other_defaults() {
        let cfg: VideoConfig =
            toml::from_str("url = \"http://localhost:9000\"\nsteps = 30\n").unwrap();
        assert_eq!(cfg.url, "http://localhost:9000");
        assert_eq!(cfg.steps, 30);
        assert_eq!(
            VideoConfig {
                url: VideoConfig::default().url,
                steps: VideoConfig::default().steps,
                ..cfg
            },
            VideoConfig::default()
        );
        let round: VideoConfig =
            toml::from_str(&toml::to_string(&VideoConfig::default()).unwrap()).unwrap();
        assert_eq!(round, VideoConfig::default());
        assert!(toml::from_str::<VideoConfig>("lora = \"x\"\n").is_err());
    }

    #[test]
    fn only_a_server_on_this_machine_is_accepted() {
        VideoConfig::default().loopback_url().unwrap();
        let away = VideoConfig {
            url: "http://192.168.1.20:8189".into(),
            ..VideoConfig::default()
        };
        let err = format!("{:#}", away.loopback_url().unwrap_err());
        assert!(err.starts_with("[video] url"), "{err}");
    }
}
