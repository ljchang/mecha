//! Who a face is: an embedding to compare against a library portrait's.
//!
//! The network is ArcFace's IResNet-50, InsightFace's `buffalo_l` recognition
//! model (`w600k_r50`, trained on WebFace600K), as py-feat repackages it with
//! each block's trailing batch norms folded into its convolutions
//! (`py-feat/arcface_r50`). Its weights are **InsightFace's non-commercial
//! research licence**: the owner chose it for the replay judges (2026-10-11)
//! over FaceNet, which tells two different people apart far less well, on
//! the terms that it is fetched only when a judge first needs it, says its
//! licence when it is, and is never part of a feature's install.
//!
//! A face is aligned before it is embedded, as ArcFace was trained: its five
//! landmarks (`face::Face::points`) carried onto InsightFace's 112×112
//! template by a similarity transform. The network runs on `face`'s own
//! convolution, for the reason `face` gives.

use crate::face::{all_claimed, read_safetensors, take, Conv, Face, Map, Tensors};
use crate::recommend::HubFile;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// The weights, pinned: py-feat's distribution of InsightFace's
/// `w600k_r50`.
pub const REPO: &str = "py-feat/arcface_r50";
pub const REVISION: &str = "1e2637bc33aef12c2eeec84f76080509f9d5ab0d";
pub const WEIGHTS: HubFile = HubFile {
    path: "arcface_r50.safetensors",
    sha256: "911ba0172feb170e53f2eb94fb2f29099bc722cb82a2c14147bbedd253056313",
    bytes: 174_387_800,
};

/// Said whenever the weights are fetched.
pub const LICENCE: &str = "ArcFace R50 (InsightFace buffalo_l, py-feat/arcface_r50): \
InsightFace's non-commercial research licence, \
https://github.com/deepinsight/insightface/blob/master/LICENSE";

/// The side of the aligned face the network reads.
pub const SIDE: u32 = 112;

/// InsightFace's template: where the five landmarks sit in an aligned face
/// (`arcface_dst` in `insightface.utils.face_align`).
const TEMPLATE: [[f32; 2]; 5] = [
    [38.2946, 51.6963],
    [73.5318, 51.5014],
    [56.0252, 71.7366],
    [41.5493, 92.3655],
    [70.7299, 92.2041],
];

/// The weights' path when the pinned file is in the model cache, `None` when
/// it is not. Never downloads: that is [`install`]'s.
pub fn weights() -> Result<Option<PathBuf>> {
    let hub = crate::fetch::hub_dir()?;
    Ok(
        match crate::fetch::cached(&hub, REPO, REVISION, &WEIGHTS, false)? {
            crate::fetch::Cached::Verified => {
                Some(crate::fetch::snapshot_path(&hub, REPO, REVISION, &WEIGHTS))
            }
            crate::fetch::Cached::Unverified
            | crate::fetch::Cached::Mismatch
            | crate::fetch::Cached::Absent => None,
        },
    )
}

/// Fetch the pinned weights into the model cache, checked against the pin.
pub async fn install(progress: &mut (dyn FnMut(u64) + Send)) -> Result<PathBuf> {
    let hub = crate::fetch::hub_dir()?;
    crate::fetch::fetch_hub_file(&hub, REPO, REVISION, &WEIGHTS, progress).await
}

/// The similarity transform carrying `points` onto [`TEMPLATE`], as
/// `[[a, -b, tx], [b, a, ty]]`: the least-squares rotation, uniform scale
/// and shift, which is what scikit-image's Umeyama estimate gives in two
/// dimensions when no reflection is wanted.
fn to_template(points: &[[f32; 2]; 5]) -> [[f32; 3]; 2] {
    let n = points.len() as f32;
    let mean = |p: &[[f32; 2]; 5]| {
        let (x, y) = p.iter().fold((0.0, 0.0), |(x, y), q| (x + q[0], y + q[1]));
        (x / n, y / n)
    };
    let (sx, sy) = mean(points);
    let (dx, dy) = mean(&TEMPLATE);
    // a = Σ conj(z)·w / Σ |z|², over the centred points as complex numbers.
    let (mut re, mut im, mut norm) = (0f32, 0f32, 0f32);
    for (p, q) in points.iter().zip(&TEMPLATE) {
        let (zx, zy) = (p[0] - sx, p[1] - sy);
        let (wx, wy) = (q[0] - dx, q[1] - dy);
        re += zx * wx + zy * wy;
        im += zx * wy - zy * wx;
        norm += zx * zx + zy * zy;
    }
    let (a, b) = if norm > 0.0 {
        (re / norm, im / norm)
    } else {
        (1.0, 0.0)
    };
    [
        [a, -b, dx - (a * sx - b * sy)],
        [b, a, dy - (b * sx + a * sy)],
    ]
}

/// The face at `points`, aligned to the template at [`SIDE`]², sampled
/// bilinearly with pixel centres at whole coordinates and black beyond the
/// picture, as `cv2.warpAffine` does in InsightFace's `norm_crop`.
pub fn align(img: &image::RgbImage, points: &[[f32; 2]; 5]) -> image::RgbImage {
    let m = to_template(points);
    let (a, b) = (m[0][0], m[1][0]);
    let det = a * a + b * b;
    let (w, h) = (img.width() as i64, img.height() as i64);
    let pixel = |x: i64, y: i64, c: usize| -> f32 {
        if x < 0 || y < 0 || x >= w || y >= h {
            0.0
        } else {
            img.get_pixel(x as u32, y as u32)[c] as f32
        }
    };
    image::RgbImage::from_fn(SIDE, SIDE, |u, v| {
        // The template point back into the picture: M⁻¹ of (u, v).
        let (px, py) = (u as f32 - m[0][2], v as f32 - m[1][2]);
        let (x, y) = if det > 0.0 {
            ((a * px + b * py) / det, (-b * px + a * py) / det)
        } else {
            (px, py)
        };
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        image::Rgb(std::array::from_fn(|c| {
            let top = pixel(x0, y0, c) * (1.0 - fx) + pixel(x0 + 1, y0, c) * fx;
            let bottom = pixel(x0, y0 + 1, c) * (1.0 - fx) + pixel(x0 + 1, y0 + 1, c) * fx;
            (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
        }))
    })
}

/// How alike two embeddings are: their cosine, in [-1, 1]. Each is unit
/// length (`Embedder::embed`), so it is their dot product.
pub fn similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// A batch norm, as the per-channel scale and shift it is at inference.
struct Affine {
    scale: Vec<f32>,
    shift: Vec<f32>,
}

impl Affine {
    fn load(t: &mut Tensors, at: &str) -> Result<Affine> {
        let mut get = |f: &str| take(t, &format!("{at}.{f}")).map(|(_, v)| v);
        let (gamma, beta, mean, var) = (
            get("weight")?,
            get("bias")?,
            get("running_mean")?,
            get("running_var")?,
        );
        let n = gamma.len();
        if [beta.len(), mean.len(), var.len()] != [n, n, n] {
            bail!("`{at}`'s batch norm does not hold one value per channel");
        }
        let scale: Vec<f32> = (0..n).map(|i| gamma[i] / (var[i] + 1e-5).sqrt()).collect();
        let shift = (0..n).map(|i| beta[i] - mean[i] * scale[i]).collect();
        Ok(Affine { scale, shift })
    }

    /// Over a map's channels.
    fn map(&self, mut x: Map) -> Map {
        assert_eq!(x.c, self.scale.len(), "a batch norm of another width");
        let plane = x.h * x.w;
        for (c, p) in x.v.chunks_exact_mut(plane).enumerate() {
            let (s, b) = (self.scale[c], self.shift[c]);
            p.iter_mut().for_each(|v| *v = *v * s + b);
        }
        x
    }

    /// Over a vector's entries.
    fn vector(&self, x: &mut [f32]) {
        assert_eq!(x.len(), self.scale.len(), "a batch norm of another width");
        for (i, v) in x.iter_mut().enumerate() {
            *v = *v * self.scale[i] + self.shift[i];
        }
    }
}

/// A PReLU: the slope each channel's negative values keep.
struct Prelu(Vec<f32>);

impl Prelu {
    fn load(t: &mut Tensors, at: &str) -> Result<Prelu> {
        Ok(Prelu(take(t, &format!("{at}.weight"))?.1))
    }

    fn run(&self, mut x: Map) -> Map {
        assert_eq!(x.c, self.0.len(), "a PReLU of another width");
        let plane = x.h * x.w;
        for (c, p) in x.v.chunks_exact_mut(plane).enumerate() {
            let a = self.0[c];
            p.iter_mut().for_each(|v| {
                if *v < 0.0 {
                    *v *= a
                }
            });
        }
        x
    }
}

/// InsightFace's `IBasicBlock`: a batch norm before the first convolution
/// (kept, since zero padding follows it), the block's other two batch norms
/// folded into its convolutions' biases.
struct Block {
    bn1: Affine,
    conv1: Conv,
    prelu: Prelu,
    conv2: Conv,
    down: Option<Conv>,
}

impl Block {
    fn load(t: &mut Tensors, at: &str, stride: usize) -> Result<Block> {
        let down = t
            .contains_key(&format!("{at}.downsample.weight"))
            .then(|| Conv::load(t, &format!("{at}.downsample"), None, stride, false))
            .transpose()?;
        Ok(Block {
            bn1: Affine::load(t, &format!("{at}.bn1"))?,
            conv1: Conv::load(t, &format!("{at}.conv1"), None, 1, false)?,
            prelu: Prelu::load(t, &format!("{at}.prelu"))?,
            conv2: Conv::load(t, &format!("{at}.conv2"), None, stride, false)?,
            down,
        })
    }

    fn run(&self, x: &Map) -> Map {
        let y = self
            .conv2
            .run(&self.prelu.run(self.conv1.run(&self.bn1.map(x.clone()))));
        match &self.down {
            Some(d) => y.add(&d.run(x)),
            None => y.add(x),
        }
    }
}

/// IResNet-50.
struct Net {
    conv1: Conv,
    prelu: Prelu,
    layers: Vec<Vec<Block>>,
    bn2: Affine,
    fc_w: Vec<f32>,
    fc_b: Vec<f32>,
    features: Affine,
}

/// The length of an embedding.
pub const DIM: usize = 512;

impl Net {
    fn load(t: &mut Tensors) -> Result<Net> {
        let mut layers = Vec::new();
        for (i, blocks) in [3, 4, 14, 3].into_iter().enumerate() {
            layers.push(
                (0..blocks)
                    .map(|b| {
                        Block::load(
                            t,
                            &format!("layer{}.{b}", i + 1),
                            if b == 0 { 2 } else { 1 },
                        )
                    })
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        let (fc_shape, fc_w) = take(t, "fc.weight")?;
        let fc_b = take(t, "fc.bias")?.1;
        let flat = 512 * 7 * 7;
        if fc_shape != [DIM, flat] || fc_b.len() != DIM {
            bail!("`fc` is not {DIM}×{flat}");
        }
        Ok(Net {
            conv1: Conv::load(t, "conv1", None, 1, false)?,
            prelu: Prelu::load(t, "prelu")?,
            layers,
            bn2: Affine::load(t, "bn2")?,
            fc_w,
            fc_b,
            features: Affine::load(t, "features")?,
        })
    }

    /// The embedding of an aligned face, before it is normalised.
    fn forward(&self, x: &Map) -> Vec<f32> {
        let mut y = self.prelu.run(self.conv1.run(x));
        for layer in &self.layers {
            for block in layer {
                y = block.run(&y);
            }
        }
        let y = self.bn2.map(y);
        let mut out = self.fc_b.clone();
        for (o, row) in out.iter_mut().zip(self.fc_w.chunks_exact(y.v.len())) {
            *o += row.iter().zip(&y.v).map(|(w, v)| w * v).sum::<f32>();
        }
        self.features.vector(&mut out);
        out
    }
}

/// ArcFace, loaded.
pub struct Embedder {
    net: Net,
}

impl Embedder {
    pub fn load(weights: &Path) -> Result<Self> {
        let bytes = std::fs::read(weights)
            .with_context(|| format!("reading ArcFace's weights {}", weights.display()))?;
        let mut tensors = read_safetensors(&bytes).context("reading ArcFace's weights")?;
        let net = Net::load(&mut tensors).context("loading ArcFace's weights")?;
        all_claimed(&tensors, "ArcFace")?;
        Ok(Embedder { net })
    }

    /// An aligned face's embedding, unit length. The face is RGB in
    /// [-1, 1], as InsightFace feeds it (`(x - 127.5) / 127.5`).
    pub fn embed_aligned(&self, face: &image::RgbImage) -> Result<Vec<f32>> {
        if face.dimensions() != (SIDE, SIDE) {
            bail!("an aligned face is {SIDE}×{SIDE}");
        }
        let n = (SIDE * SIDE) as usize;
        let mut v = vec![0f32; 3 * n];
        for (x, y, p) in face.enumerate_pixels() {
            for c in 0..3 {
                v[c * n + (y * SIDE + x) as usize] = (p[c] as f32 - 127.5) / 127.5;
            }
        }
        let mut e = self.net.forward(&Map {
            c: 3,
            h: SIDE as usize,
            w: SIDE as usize,
            v,
        });
        let norm = e.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            e.iter_mut().for_each(|x| *x /= norm);
        }
        Ok(e)
    }

    /// `face`'s embedding in `img`: aligned by its landmarks, then embedded.
    pub fn embed(&self, img: &image::RgbImage, face: &Face) -> Result<Vec<f32>> {
        self.embed_aligned(&align(img, &face.points))
    }
}

/// The face detector and ArcFace together: every face in a picture with its
/// embedding. The replay judges read pictures through it, and so does
/// `mecha imagelib face`, so a number the owner checks by hand is the number
/// a judge would give.
pub struct Reader {
    detector: crate::face::Detector,
    embedder: Embedder,
}

impl Reader {
    /// Both models from their weights.
    pub fn load(detector: &Path, arcface: &Path) -> Result<Self> {
        Ok(Reader {
            detector: crate::face::Detector::load(detector)?,
            embedder: Embedder::load(arcface)?,
        })
    }

    /// Every face in `img` with its embedding, found on a copy no longer
    /// than the detector's bound on its long side (`face::DETECT_SIDE`), as
    /// a portrait's crop is, so a large photo costs no more than a small
    /// one. Each face is embedded in the copy it was found in, then its box
    /// and landmarks are given in `img`'s own pixels. CPU work: run it off
    /// an async runtime.
    pub fn read(&self, img: &image::RgbImage) -> Result<Vec<(Face, Vec<f32>)>> {
        let (looked, scale) = crate::face::bounded(img);
        self.detector
            .detect(&looked)?
            .into_iter()
            .map(|f| Ok((f.unscaled(scale), self.embedder.embed(&looked, &f)?)))
            .collect()
    }

    /// [`Reader::read`] over a picture file's `bytes`, upright as a browser
    /// shows it, largest face first. `what` names the picture in an error.
    pub fn read_bytes(&self, bytes: &[u8], what: &str) -> Result<Vec<(Face, Vec<f32>)>> {
        let img = crate::image::decode_upright(bytes, what)?.to_rgb8();
        let mut faces = self.read(&img)?;
        faces.sort_by(|a, b| b.0.area().total_cmp(&a.0.area()));
        Ok(faces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Points already on the template align to themselves: the identity.
    #[test]
    fn the_template_aligns_to_itself() {
        let m = to_template(&TEMPLATE);
        for (got, want) in m.iter().flatten().zip([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]) {
            assert!((got - want).abs() < 1e-4, "{m:?}");
        }
    }

    /// A face twice the template's size, turned a quarter and moved, is
    /// carried back exactly: the transform recovers scale, rotation and
    /// shift.
    #[test]
    fn a_turned_scaled_moved_face_is_carried_onto_the_template() {
        let moved: [[f32; 2]; 5] = TEMPLATE.map(|[x, y]| [300.0 - 2.0 * y, 100.0 + 2.0 * x]);
        let m = to_template(&moved);
        for (p, q) in moved.iter().zip(&TEMPLATE) {
            let x = m[0][0] * p[0] + m[0][1] * p[1] + m[0][2];
            let y = m[1][0] * p[0] + m[1][1] * p[1] + m[1][2];
            assert!((x - q[0]).abs() < 1e-3 && (y - q[1]).abs() < 1e-3, "{m:?}");
        }
    }

    /// Aligning a picture already on the template returns it, pixel for
    /// pixel, and what lies beyond the picture is black.
    #[test]
    fn alignment_samples_the_picture_and_is_black_beyond_it() {
        let img = image::RgbImage::from_fn(SIDE, SIDE, |x, y| {
            image::Rgb([(x * 2) as u8, (y * 2) as u8, 77])
        });
        assert_eq!(align(&img, &TEMPLATE), img);
        let small = image::RgbImage::from_pixel(20, 20, image::Rgb([200, 200, 200]));
        let out = align(&small, &TEMPLATE);
        assert_eq!(out.get_pixel(100, 100), &image::Rgb([0, 0, 0]));
    }

    #[test]
    fn similarity_is_the_dot_of_unit_vectors() {
        let a = [0.6, 0.8];
        assert!((similarity(&a, &a) - 1.0).abs() < 1e-6);
        assert!((similarity(&a, &[0.8, -0.6])).abs() < 1e-6);
    }

    /// Both pinned weights load with every tensor claimed: a layer map that
    /// disagrees with its file is refused at load (`face::all_claimed`).
    /// Needs both fetched, so it is not part of the default run.
    #[test]
    #[ignore = "model: needs the installed face detector and the fetched ArcFace weights"]
    fn the_pinned_weights_are_all_claimed() {
        crate::face::Detector::load(&crate::face::weights().unwrap().expect("detector")).unwrap();
        Embedder::load(&weights().unwrap().expect("arcface")).unwrap();
    }

    /// The alignment against InsightFace's own: `MECHA_FACE_ALIGN_PARITY`
    /// names a JSON map of `{name: {"path": picture, "points": [[x, y]; 5],
    /// "aligned": png}}`, the png written by Umeyama's similarity and
    /// `cv2.warpAffine` onto the same template. Each pixel must agree to
    /// within a step or two of rounding, and on average to well under one.
    #[test]
    #[ignore = "reference: needs MECHA_FACE_ALIGN_PARITY"]
    fn the_alignment_agrees_with_insightface() {
        let reference = std::env::var("MECHA_FACE_ALIGN_PARITY").expect("MECHA_FACE_ALIGN_PARITY");
        let cases: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(reference).unwrap()).unwrap();
        for (name, case) in cases.as_object().unwrap() {
            let img = image::open(case["path"].as_str().unwrap())
                .unwrap()
                .to_rgb8();
            let points: [[f32; 2]; 5] = serde_json::from_value(case["points"].clone()).unwrap();
            let want = image::open(case["aligned"].as_str().unwrap())
                .unwrap()
                .to_rgb8();
            let got = align(&img, &points);
            let diffs: Vec<i32> = got
                .as_raw()
                .iter()
                .zip(want.as_raw())
                .map(|(a, b)| (*a as i32 - *b as i32).abs())
                .collect();
            let mean = diffs.iter().sum::<i32>() as f64 / diffs.len() as f64;
            let max = diffs.iter().copied().max().unwrap_or(0);
            eprintln!("{name}: mean {mean:.3}, max {max}");
            assert!(mean < 0.5 && max <= 3, "{name}: mean {mean}, max {max}");
        }
    }

    /// The network against InsightFace's own ONNX model: `MECHA_FACE_ID_PARITY`
    /// names a JSON map of `{name: {"aligned": png, "embedding": [512]}}`
    /// written by onnxruntime from `w600k_r50.onnx` on the same aligned
    /// crops, normalised. Each must agree at a cosine of 0.999 or more.
    /// Needs the fetched weights, so it is not part of the default run.
    #[test]
    #[ignore = "model: needs the fetched ArcFace weights and MECHA_FACE_ID_PARITY"]
    fn the_embedding_agrees_with_insightface() {
        let reference = std::env::var("MECHA_FACE_ID_PARITY").expect("MECHA_FACE_ID_PARITY");
        let weights = weights().unwrap().expect("ArcFace's weights are fetched");
        let embedder = Embedder::load(&weights).unwrap();
        let cases: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(reference).unwrap()).unwrap();
        for (name, case) in cases.as_object().unwrap() {
            let face = image::open(case["aligned"].as_str().unwrap())
                .unwrap()
                .to_rgb8();
            let started = std::time::Instant::now();
            let got = embedder.embed_aligned(&face).unwrap();
            let want: Vec<f32> = serde_json::from_value(case["embedding"].clone()).unwrap();
            let cos = similarity(&got, &want);
            eprintln!(
                "{name}: cosine {cos:.5}, {} ms",
                started.elapsed().as_millis()
            );
            assert!(cos >= 0.999, "{name}: cosine {cos}");
        }
    }
}
