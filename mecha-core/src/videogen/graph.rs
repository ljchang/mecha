//! The ComfyUI graph for one clip, built in code from typed values: the
//! model picks a prompt, a length and a shape, never a node. It follows
//! Comfy-Org's `video_minimax_h3_{t2v,i2v,r2v}` templates (workflow_templates
//! 0.11.78), with every file name from `[video]`.
//!
//! The diffusion model reaches the sampler through a chain of
//! `LoraLoaderModelOnly` nodes ([`lora_chain`]); turbo is the one link today,
//! so a LoRA the owner adds is one more entry in that chain and nothing else.

use super::VideoConfig;
use serde_json::{json, Map, Value};

/// H3's frame rate.
pub const FPS: u32 = 24;
pub const MIN_SECONDS: f64 = 4.0;
pub const MAX_SECONDS: f64 = 15.0;
/// H3's reference slots for pictures.
pub const MAX_REFERENCES: usize = 9;
/// H3's standalone reference audios (`ref_audios`).
pub const MAX_VOICES: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Words, optionally pinned to a first and a last frame (`fl2va`).
    Frames,
    /// Reference pictures named `<Picture i>` in the prompt, and voices
    /// named `<Audio j>` (`ref2va`); frames are pinned beside them.
    References,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Frames => "frames",
            Mode::References => "references",
        }
    }

    fn model(self, cfg: &VideoConfig) -> &str {
        match self {
            Mode::Frames => &cfg.frames_model,
            Mode::References => &cfg.references_model,
        }
    }

    /// The turbo LoRA and the steps it was distilled for.
    fn turbo(self, cfg: &VideoConfig) -> (&str, u32) {
        match self {
            Mode::Frames => (&cfg.frames_turbo_lora, cfg.frames_turbo_steps),
            Mode::References => (&cfg.references_turbo_lora, cfg.references_turbo_steps),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// ~0.4 MP, the templates' default (864x480 at 16:9).
    Draft,
    /// The model's native canvas: a 768 short edge, the area capped at
    /// 768x1344.
    Full,
}

impl Size {
    pub fn name(self) -> &'static str {
        match self {
            Size::Draft => "draft",
            Size::Full => "full",
        }
    }

    pub fn parse(s: &str) -> Option<Size> {
        match s {
            "draft" => Some(Size::Draft),
            "full" => Some(Size::Full),
            _ => None,
        }
    }
}

/// The shapes offered, landscape to portrait.
pub const ASPECTS: [&str; 5] = ["16:9", "4:3", "1:1", "3:4", "9:16"];

/// Width and height for an aspect and size, multiples of 32. Draft follows
/// ComfyUI's ResolutionSelector at 0.4 MP; full follows `adapt_canvas` in
/// ComfyUI's `nodes_minimax_h3.py`.
pub fn canvas(aspect: &str, size: Size) -> Option<(u32, u32)> {
    if !ASPECTS.contains(&aspect) {
        return None;
    }
    let (aw, ah) = aspect.split_once(':')?;
    let ratio = aw.parse::<f64>().ok()? / ah.parse::<f64>().ok()?;
    let (w, h) = match size {
        Size::Draft => {
            // ResolutionSelector's megapixel is 1024², which is what makes
            // 16:9 land on 864x480 rather than 832x480.
            let pixels = 0.4 * 1024.0 * 1024.0;
            let h = (pixels / ratio).sqrt();
            (h * ratio, h)
        }
        Size::Full => {
            let (mut w, mut h) = if ratio >= 1.0 {
                (768.0 * ratio, 768.0)
            } else {
                (768.0, 768.0 / ratio)
            };
            let cap = 768.0 * 1344.0;
            if w * h > cap {
                let s = (cap / (w * h)).sqrt();
                w *= s;
                h *= s;
            }
            (w, h)
        }
    };
    let round32 = |v: f64| ((v / 32.0).round() as u32).max(1) * 32;
    Some((round32(w), round32(h)))
}

/// Frames for a duration, snapped up to the model's 17k+5 grid at 24 fps —
/// the template's `max(5, round(s*24)) + (5 - that % 17) % 17`.
pub fn frames(seconds: f64) -> u32 {
    let n = ((seconds * FPS as f64).round() as u32).max(5);
    n + (17 + 5 - n % 17) % 17
}

/// What one clip is drawn from. Every picture and voice is a name the server
/// gave its upload into its temp directory (`comfy::Client::upload_temp`).
pub struct Spec<'a> {
    pub mode: Mode,
    pub prompt: &'a str,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub turbo: bool,
    pub seed: u64,
    pub first_frame: Option<&'a str>,
    pub last_frame: Option<&'a str>,
    /// The reference pictures, `<Picture 1>` first. References mode only.
    pub references: &'a [String],
    /// The voices the speech should carry, `<Audio 1>` first, at most
    /// [`MAX_VOICES`]. References mode only: the frames checkpoint takes
    /// none.
    pub reference_audios: &'a [String],
    /// Where `SaveVideo` files the clip, under the server's output folder.
    pub filename_prefix: &'a str,
    /// A VAE that decodes the clip in place of `[video] vae`, when the
    /// caller has seen the server list it.
    pub decoder: Option<&'a str>,
}

impl Spec<'_> {
    pub fn steps(&self, cfg: &VideoConfig) -> u32 {
        if self.turbo {
            self.mode.turbo(cfg).1
        } else {
            cfg.steps
        }
    }

    /// The LoRAs between the diffusion model and the sampler, in order.
    fn loras<'c>(&self, cfg: &'c VideoConfig) -> Vec<Lora<'c>> {
        let mut chain = Vec::new();
        if self.turbo {
            chain.push(Lora {
                file: self.mode.turbo(cfg).0,
                strength: 1.0,
            });
        }
        chain
    }
}

/// One LoRA applied to the diffusion model: a `LoraLoaderModelOnly` link.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lora<'a> {
    pub file: &'a str,
    pub strength: f64,
}

fn put(g: &mut Map<String, Value>, id: &str, class: &str, inputs: Value) {
    g.insert(id.into(), json!({"class_type": class, "inputs": inputs}));
}

/// A temp upload as a loader names it.
fn temp(name: &str) -> String {
    format!("{name} [temp]")
}

/// Chain `loras` onto the model output `from`, as `lora0`, `lora1`, …, and
/// return the last link's output: what the scheduler and the guider take.
fn lora_chain(g: &mut Map<String, Value>, from: Value, loras: &[Lora]) -> Value {
    loras.iter().enumerate().fold(from, |model, (i, lora)| {
        let id = format!("lora{i}");
        put(
            g,
            &id,
            "LoraLoaderModelOnly",
            json!({"model": model, "lora_name": lora.file, "strength_model": lora.strength}),
        );
        json!([id, 0])
    })
}

/// The API-format prompt for one clip.
pub fn build(cfg: &VideoConfig, spec: &Spec) -> Value {
    let mut g = Map::new();
    put(
        &mut g,
        "unet",
        "UNETLoader",
        json!({"unet_name": spec.mode.model(cfg), "weight_dtype": "default"}),
    );
    let model = lora_chain(&mut g, json!(["unet", 0]), &spec.loras(cfg));
    put(
        &mut g,
        "clip",
        "CLIPLoader",
        json!({"clip_name": cfg.text_encoder, "type": "minimax", "device": "default"}),
    );
    put(&mut g, "vae", "VAELoader", json!({"vae_name": cfg.vae}));
    put(
        &mut g,
        "audio_vae",
        "VAELoader",
        json!({"vae_name": cfg.audio_vae}),
    );

    // The conditioning the guider samples from: the mode node's, or the
    // last pinned frame's.
    let mut positive = json!(["cond", 0]);
    match spec.mode {
        Mode::Frames => {
            let mut inputs = json!({
                "clip": ["clip", 0], "vae": ["vae", 0], "prompt": spec.prompt,
                "width": spec.width, "height": spec.height, "length": spec.frames,
            });
            for (key, image) in [
                ("first_frame", spec.first_frame),
                ("last_frame", spec.last_frame),
            ] {
                if let Some(image) = image {
                    put(&mut g, key, "LoadImage", json!({"image": temp(image)}));
                    inputs[key] = json!([key, 0]);
                }
            }
            put(&mut g, "cond", "MiniMaxH3ImageToVideo", inputs);
        }
        Mode::References => {
            let mut inputs = json!({
                "clip": ["clip", 0], "vae": ["vae", 0], "audio_vae": ["audio_vae", 0],
                "prompt": spec.prompt, "width": spec.width, "height": spec.height,
                "length": spec.frames, "ref_image_size": "match",
            });
            for (i, image) in spec.references.iter().enumerate() {
                let id = format!("ref{i}");
                put(&mut g, &id, "LoadImage", json!({"image": temp(image)}));
                inputs[format!("ref_images.ref_image_{i}")] = json!([id, 0]);
            }
            for (j, audio) in spec.reference_audios.iter().enumerate() {
                let id = format!("ref_audio{j}");
                put(&mut g, &id, "LoadAudio", json!({"audio": temp(audio)}));
                inputs[format!("ref_audios.ref_audio_{j}")] = json!([id, 0]);
            }
            put(&mut g, "cond", "MiniMaxH3ReferenceToVideo", inputs);
            // A frame pinned beside the references (`MiniMaxH3AddGuide`): the
            // portraits hold the faces, the frame holds the scene. Measured
            // 2026-10-13: the clip opens exactly on a frame pinned this way,
            // where the same frame sent as a reference was reinterpreted.
            for (key, image, at) in [
                ("first_frame", spec.first_frame, 0),
                ("last_frame", spec.last_frame, -1),
            ] {
                if let Some(image) = image {
                    put(&mut g, key, "LoadImage", json!({"image": temp(image)}));
                    let id = format!("guide_{key}");
                    put(
                        &mut g,
                        &id,
                        "MiniMaxH3AddGuide",
                        json!({
                            "positive": positive.clone(), "vae": ["vae", 0], "latent": ["cond", 1],
                            "image": [key, 0], "frame_idx": at,
                        }),
                    );
                    positive = json!([id, 0]);
                }
            }
        }
    }

    put(
        &mut g,
        "noise",
        "RandomNoise",
        json!({"noise_seed": spec.seed}),
    );
    put(
        &mut g,
        "sampler",
        "KSamplerSelect",
        json!({"sampler_name": "res_multistep"}),
    );
    put(
        &mut g,
        "sigmas",
        "BasicScheduler",
        json!({"model": model, "scheduler": "simple", "steps": spec.steps(cfg), "denoise": 1.0}),
    );
    put(
        &mut g,
        "guider",
        "BasicGuider",
        json!({"model": model, "conditioning": positive}),
    );
    put(
        &mut g,
        "sample",
        "SamplerCustomAdvanced",
        json!({"noise": ["noise", 0], "guider": ["guider", 0], "sampler": ["sampler", 0],
               "sigmas": ["sigmas", 0], "latent_image": ["cond", 1]}),
    );
    let decode_vae = match spec.decoder {
        Some(name) => {
            put(&mut g, "vae_decode", "VAELoader", json!({"vae_name": name}));
            json!(["vae_decode", 0])
        }
        None => json!(["vae", 0]),
    };
    put(
        &mut g,
        "decode",
        "VAEDecode",
        json!({"samples": ["sample", 0], "vae": decode_vae}),
    );
    put(
        &mut g,
        "decode_audio",
        "VAEDecodeAudio",
        json!({"samples": ["sample", 0], "vae": ["audio_vae", 0]}),
    );
    put(
        &mut g,
        "video",
        "CreateVideo",
        json!({"images": ["decode", 0], "audio": ["decode_audio", 0], "fps": FPS}),
    );
    put(
        &mut g,
        "save",
        "SaveVideo",
        json!({"video": ["video", 0], "filename_prefix": spec.filename_prefix,
               "format": "mp4", "format.codec": "h264"}),
    );
    Value::Object(g)
}

/// One file a graph names, as the server's `/object_info/<node>` lists the
/// choices for `input`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Need {
    pub node: &'static str,
    pub input: &'static str,
    pub file: String,
}

/// Every model file [`build`] names for `spec`, to ask the server for before
/// anything is uploaded: a missing file otherwise fails deep in a render, as
/// a node validation error the model cannot act on. Uploads are not here:
/// they are on the server by the time the graph is built.
pub fn needs(cfg: &VideoConfig, spec: &Spec) -> Vec<Need> {
    let need = |node, input, file: &str| Need {
        node,
        input,
        file: file.to_string(),
    };
    let mut out = vec![
        need("UNETLoader", "unet_name", spec.mode.model(cfg)),
        need("CLIPLoader", "clip_name", &cfg.text_encoder),
        need("VAELoader", "vae_name", &cfg.vae),
        need("VAELoader", "vae_name", &cfg.audio_vae),
    ];
    out.extend(
        spec.loras(cfg)
            .iter()
            .map(|l| need("LoraLoaderModelOnly", "lora_name", l.file)),
    );
    if let Some(decoder) = spec.decoder {
        out.push(need("VAELoader", "vae_name", decoder));
    }
    out
}

/// The H3 nodes a clip in `mode` uses, which only a ComfyUI with H3 support
/// has: asked for so an older server is named as one.
pub fn h3_nodes(mode: Mode, pinned: bool) -> &'static [&'static str] {
    match (mode, pinned) {
        (Mode::Frames, _) => &["MiniMaxH3ImageToVideo"],
        (Mode::References, false) => &["MiniMaxH3ReferenceToVideo"],
        (Mode::References, true) => &["MiniMaxH3ReferenceToVideo", "MiniMaxH3AddGuide"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_snap_to_the_17k_plus_5_grid() {
        // The node's own tooltip: 124 is ~5 s; the trained range is 124-362.
        assert_eq!(frames(5.0), 124);
        assert_eq!(frames(10.0), 243);
        assert_eq!(frames(15.0), 362);
        assert_eq!(frames(4.0), 107);
        for s in [4.0, 6.5, 9.0, 12.25] {
            assert_eq!(frames(s) % 17, 5, "{s}");
        }
    }

    #[test]
    fn canvases_match_the_template_table_and_the_node() {
        // ResolutionSelector at 0.4 MP, 16:9 → 864x480 (template note).
        assert_eq!(canvas("16:9", Size::Draft), Some((864, 480)));
        assert_eq!(canvas("9:16", Size::Draft), Some((480, 864)));
        // adapt_canvas: 768 short edge, 768*1344 area cap.
        assert_eq!(canvas("16:9", Size::Full), Some((1344, 768)));
        assert_eq!(canvas("9:16", Size::Full), Some((768, 1344)));
        assert_eq!(canvas("1:1", Size::Full), Some((768, 768)));
        assert_eq!(canvas("2:1", Size::Full), None);
        for aspect in ASPECTS {
            for size in [Size::Draft, Size::Full] {
                let (w, h) = canvas(aspect, size).unwrap();
                assert_eq!((w % 32, h % 32), (0, 0), "{aspect} {size:?}");
            }
        }
        assert_eq!(Size::parse("full"), Some(Size::Full));
        assert_eq!(Size::parse(Size::Draft.name()), Some(Size::Draft));
        assert_eq!(Size::parse("huge"), None);
    }

    fn spec<'a>(refs: &'a [String], audios: &'a [String]) -> Spec<'a> {
        Spec {
            mode: Mode::References,
            prompt: "p",
            width: 480,
            height: 864,
            frames: 124,
            turbo: true,
            seed: 1,
            first_frame: None,
            last_frame: None,
            references: refs,
            reference_audios: audios,
            filename_prefix: "mecha/x",
            decoder: None,
        }
    }

    #[test]
    fn references_wire_as_autogrow_slots_and_turbo_uses_the_references_lora() {
        let cfg = VideoConfig::default();
        let refs = vec!["a.png".to_string(), "b.png".to_string()];
        let audios = vec!["v.wav".to_string(), "w.wav".to_string()];
        let s = spec(&refs, &audios);
        let g = build(&cfg, &s);
        let cond = &g["cond"]["inputs"];
        assert_eq!(g["unet"]["inputs"]["unet_name"], cfg.references_model);
        assert_eq!(cond["ref_images.ref_image_0"], json!(["ref0", 0]));
        assert_eq!(cond["ref_images.ref_image_1"], json!(["ref1", 0]));
        // Read from the temp directory, where the upload put them.
        assert_eq!(g["ref1"]["inputs"]["image"], "b.png [temp]");
        assert_eq!(g["sigmas"]["inputs"]["steps"], cfg.references_turbo_steps);
        assert_eq!(
            g["lora0"]["inputs"]["lora_name"],
            cfg.references_turbo_lora.as_str()
        );
        assert_eq!(g["lora0"]["inputs"]["model"], json!(["unet", 0]));
        assert_eq!(g["guider"]["inputs"]["model"], json!(["lora0", 0]));
        assert_eq!(g["sigmas"]["inputs"]["model"], json!(["lora0", 0]));
        assert_eq!(cond["ref_audios.ref_audio_0"], json!(["ref_audio0", 0]));
        assert_eq!(g["ref_audio0"]["inputs"]["audio"], "v.wav [temp]");
        assert_eq!(cond["ref_audios.ref_audio_1"], json!(["ref_audio1", 0]));
        // Without a pinned frame the guider samples the references alone.
        assert_eq!(g["guider"]["inputs"]["conditioning"], json!(["cond", 0]));

        let pinned = build(
            &cfg,
            &Spec {
                first_frame: Some("f.png"),
                last_frame: Some("l.png"),
                ..spec(&refs, &audios)
            },
        );
        assert_eq!(pinned["guide_first_frame"]["inputs"]["frame_idx"], 0);
        assert_eq!(
            pinned["guide_first_frame"]["inputs"]["positive"],
            json!(["cond", 0])
        );
        assert_eq!(pinned["guide_last_frame"]["inputs"]["frame_idx"], -1);
        assert_eq!(
            pinned["guide_last_frame"]["inputs"]["positive"],
            json!(["guide_first_frame", 0])
        );
        assert_eq!(
            pinned["guider"]["inputs"]["conditioning"],
            json!(["guide_last_frame", 0])
        );
        assert_eq!(pinned["first_frame"]["inputs"]["image"], "f.png [temp]");

        // The full VAE decodes unless a decoder is named.
        assert_eq!(g["decode"]["inputs"]["vae"], json!(["vae", 0]));
        assert!(g.get("vae_decode").is_none());
        // A decoder replaces the decode only: the references are still
        // encoded by the full VAE, which the conditioning node needs.
        let light = build(
            &cfg,
            &Spec {
                decoder: Some("small-decoder.safetensors"),
                ..spec(&refs, &audios)
            },
        );
        assert_eq!(light["decode"]["inputs"]["vae"], json!(["vae_decode", 0]));
        assert_eq!(
            light["vae_decode"]["inputs"]["vae_name"],
            "small-decoder.safetensors"
        );
        assert_eq!(light["cond"]["inputs"]["vae"], json!(["vae", 0]));
        assert_eq!(light["vae"]["inputs"]["vae_name"], cfg.vae.as_str());
    }

    #[test]
    fn frames_mode_without_turbo_has_no_lora_and_the_full_steps() {
        let cfg = VideoConfig::default();
        let g = build(
            &cfg,
            &Spec {
                mode: Mode::Frames,
                turbo: false,
                first_frame: Some("f.png"),
                ..spec(&[], &[])
            },
        );
        assert!(!g.as_object().unwrap().keys().any(|k| k.starts_with("lora")));
        assert_eq!(g["unet"]["inputs"]["unet_name"], cfg.frames_model.as_str());
        assert_eq!(g["sigmas"]["inputs"]["steps"], cfg.steps);
        assert_eq!(
            g["cond"]["inputs"]["first_frame"],
            json!(["first_frame", 0])
        );
        assert_eq!(g["cond"]["class_type"], "MiniMaxH3ImageToVideo");
        assert!(g["cond"]["inputs"].get("last_frame").is_none());
        assert_eq!(g["guider"]["inputs"]["model"], json!(["unet", 0]));
        // The frames turbo is its own LoRA and step count.
        let turbo = build(
            &cfg,
            &Spec {
                mode: Mode::Frames,
                ..spec(&[], &[])
            },
        );
        assert_eq!(
            turbo["lora0"]["inputs"]["lora_name"],
            cfg.frames_turbo_lora.as_str()
        );
        assert_eq!(turbo["sigmas"]["inputs"]["steps"], cfg.frames_turbo_steps);
    }

    /// The chain is open-ended: each link takes the one before it, and the
    /// sampler and the guider take the last.
    #[test]
    fn loras_chain_from_the_loader_to_the_sampler() {
        let mut g = Map::new();
        let out = lora_chain(
            &mut g,
            json!(["unet", 0]),
            &[
                Lora {
                    file: "a.safetensors",
                    strength: 1.0,
                },
                Lora {
                    file: "b.safetensors",
                    strength: 0.6,
                },
            ],
        );
        assert_eq!(out, json!(["lora1", 0]));
        assert_eq!(g["lora0"]["inputs"]["model"], json!(["unet", 0]));
        assert_eq!(g["lora1"]["inputs"]["model"], json!(["lora0", 0]));
        assert_eq!(g["lora1"]["inputs"]["strength_model"], 0.6);
        let mut none = Map::new();
        assert_eq!(
            lora_chain(&mut none, json!(["unet", 0]), &[]),
            json!(["unet", 0])
        );
        assert!(none.is_empty());
    }

    /// Every model file a built graph names is one [`needs`] asks for, and
    /// nothing else: a file added to the graph and not to the preflight
    /// would fail mid-render, which is what the preflight is for.
    #[test]
    fn the_preflight_asks_for_every_file_the_graph_names() {
        let cfg = VideoConfig::default();
        let refs = vec!["a.png".to_string()];
        let audios = vec!["v.wav".to_string()];
        for s in [
            spec(&refs, &audios),
            Spec {
                turbo: false,
                decoder: Some("d.safetensors"),
                ..spec(&refs, &audios)
            },
            Spec {
                mode: Mode::Frames,
                first_frame: Some("f.png"),
                ..spec(&[], &[])
            },
        ] {
            let g = build(&cfg, &s);
            let mut named: Vec<Need> = g
                .as_object()
                .unwrap()
                .values()
                .filter_map(|n| {
                    let (node, input) = match n["class_type"].as_str()? {
                        "UNETLoader" => ("UNETLoader", "unet_name"),
                        "CLIPLoader" => ("CLIPLoader", "clip_name"),
                        "VAELoader" => ("VAELoader", "vae_name"),
                        "LoraLoaderModelOnly" => ("LoraLoaderModelOnly", "lora_name"),
                        _ => return None,
                    };
                    let file = n["inputs"][input].as_str()?.to_string();
                    Some(Need { node, input, file })
                })
                .collect();
            let mut asked = needs(&cfg, &s);
            named.sort();
            asked.sort();
            assert_eq!(named, asked);
        }
    }

    #[test]
    fn the_h3_nodes_follow_the_mode() {
        let cfg = VideoConfig::default();
        let refs = vec!["a.png".to_string()];
        let pinned = build(
            &cfg,
            &Spec {
                first_frame: Some("f.png"),
                ..spec(&refs, &[])
            },
        );
        let classes: Vec<&str> = pinned
            .as_object()
            .unwrap()
            .values()
            .filter_map(|n| n["class_type"].as_str())
            .filter(|c| c.starts_with("MiniMaxH3"))
            .collect();
        for node in h3_nodes(Mode::References, true) {
            assert!(classes.contains(node), "{node}");
        }
        assert_eq!(h3_nodes(Mode::Frames, true), ["MiniMaxH3ImageToVideo"]);
    }
}
