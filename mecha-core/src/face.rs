//! Where a face is in a picture, and the crop a persona's edit is anchored to.
//!
//! An edit of a picture keeps only part of the face it was given — measured
//! at about 0.77 ArcFace similarity per step on the owner's persona chats, so
//! a chain of edits ends as somebody else (2026-10-05). Sending a tight crop
//! of the character's own portrait beside the picture holds the face, and a
//! crop with no outfit or pose in it leaves the rest of the edit free
//! (`ARCHITECTURE.md` §Image generation, "anchored to its face").
//!
//! The detector is py-feat's RetinaFace with a ResNet34 backbone
//! (`py-feat/retinaface_r34`, MIT), run on the CPU by the small network below
//! and checked against py-feat itself to within a pixel. It runs once per
//! portrait: the crop is cached under the library, named by the portrait's
//! blob, so a new portrait is a new crop and nothing has to be invalidated.
//!
//! The network is this file's own rather than a framework's: it needs
//! convolution, pooling, upsampling and addition, and the one framework
//! tried (candle) brought 68 crates into mecha-core — two of them C and C++
//! libraries compiled from source — for that (the owner's ruling,
//! 2026-10-05). Convolutions are matrix products on `matrixmultiply`, batch
//! norm is folded into them when the weights load.

use crate::recommend::HubFile;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The weights, pinned: py-feat's own distribution of RetinaFace-R34.
pub const REPO: &str = "py-feat/retinaface_r34";
pub const REVISION: &str = "2e4495d934ed359ae8ddc0946d82f85e1e328e52";
pub const WEIGHTS: HubFile = HubFile {
    path: "model.safetensors",
    sha256: "f339890a869284bb39dc16ed6f52acc21d74a73d6e419088697e46ee0b54eaf2",
    bytes: 88_606_408,
};

/// The command that installs the weights, said wherever they are missing.
pub const INSTALL_HINT: &str = "mecha imagelib install-face-detector";

/// The longest side a picture is detected at. A portrait is read once, so
/// this bounds the time a large photo costs, not the accuracy a face the
/// size of a portrait's needs.
const DETECT_SIDE: u32 = 1024;

/// The crop's side over the face box's larger side. The experiments cropped
/// 1.05 times InsightFace's box; RetinaFace draws a tighter box, and 1.12
/// times it is the same crop — 258 px against 258 on the owner's portrait
/// (2026-10-05).
const CROP_SCALE: f32 = 1.12;

/// A face, in the picture's own pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Face {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub score: f32,
}

impl Face {
    fn area(&self) -> f32 {
        (self.x1 - self.x0).max(0.0) * (self.y1 - self.y0).max(0.0)
    }
}

/// The weights' path when the pinned file is in the model cache, `None` when
/// it is not. Never downloads: that is [`install`]'s, run by the owner.
pub fn weights() -> Result<Option<PathBuf>> {
    let hub = crate::fetch::hub_dir()?;
    Ok(
        match crate::fetch::cached(&hub, REPO, REVISION, &WEIGHTS, false)? {
            crate::fetch::Cached::Verified | crate::fetch::Cached::Unverified => {
                Some(crate::fetch::snapshot_path(&hub, REPO, REVISION, &WEIGHTS))
            }
            crate::fetch::Cached::Mismatch | crate::fetch::Cached::Absent => None,
        },
    )
}

/// Fetch the pinned weights into the model cache, checked against the pin.
pub async fn install(progress: &mut dyn FnMut(u64)) -> Result<PathBuf> {
    let hub = crate::fetch::hub_dir()?;
    crate::fetch::fetch_hub_file(&hub, REPO, REVISION, &WEIGHTS, progress).await
}

/// RetinaFace-R34, loaded.
pub struct Detector {
    net: Net,
}

impl Detector {
    pub fn load(weights: &Path) -> Result<Self> {
        let bytes = std::fs::read(weights).with_context(|| {
            format!("reading the face detector's weights {}", weights.display())
        })?;
        let mut tensors =
            read_safetensors(&bytes).context("reading the face detector's weights")?;
        Ok(Detector {
            net: Net::load(&mut tensors).context("loading the face detector's weights")?,
        })
    }

    /// Every face in `img` above py-feat's own thresholds (score 0.5 after
    /// NMS at IoU 0.4), in its pixels, best first.
    pub fn detect(&self, img: &image::RgbImage) -> Result<Vec<Face>> {
        let (w, h) = (img.width() as usize, img.height() as usize);
        if w < 32 || h < 32 {
            return Ok(Vec::new());
        }
        // py-feat's preprocessing: RGB in [0, 255] less the training mean,
        // no scaling.
        const MEAN: [f32; 3] = [123.0, 117.0, 104.0];
        let mut v = vec![0f32; 3 * h * w];
        for (x, y, p) in img.enumerate_pixels() {
            for c in 0..3 {
                v[c * h * w + y as usize * w + x as usize] = p[c] as f32 - MEAN[c];
            }
        }
        let (loc, conf) = self.net.forward(&Map { c: 3, h, w, v });
        Ok(decode(&loc, &conf, w, h))
    }
}

/// The anchor crop of a picture's largest face: square, [`CROP_SCALE`] times
/// the face's larger side, centred a little below its box's centre, kept
/// inside the picture, as a PNG. `None` when no face is found.
///
/// Tight on purpose (measured 2026-10-05): a crop at 1.6 times the face
/// carried the hair and the shoulders' pose into edits and softened the
/// expressions they asked for, and the whole portrait carried its outfit and
/// its selfie pose into four of six scenes. The tight crop's edits kept their
/// asked-for frowns, lowered eyes, hair changes and camera.
pub fn anchor_crop(detector: &Detector, bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let picture = crate::image::decode_upright(bytes, "the portrait")?.to_rgb8();
    let (pw, ph) = picture.dimensions();
    let scale = (DETECT_SIDE as f32 / pw.max(ph) as f32).min(1.0);
    let seen = if scale < 1.0 {
        image::imageops::resize(
            &picture,
            ((pw as f32 * scale).round() as u32).max(1),
            ((ph as f32 * scale).round() as u32).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        picture.clone()
    };
    let faces = detector.detect(&seen)?;
    let Some(face) = faces
        .iter()
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .copied()
    else {
        return Ok(None);
    };
    let (x, y, side) = crop_square(face, scale, pw, ph);
    let crop = image::imageops::crop_imm(&picture, x, y, side, side).to_image();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(crop)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| anyhow!("encoding the face crop: {e}"))?;
    Ok(Some(png.into_inner()))
}

/// The crop's top-left corner and side, in the picture's pixels, for a face
/// found at `scale`: shifted to stay inside the picture, and shrunk only when
/// the picture itself is smaller.
fn crop_square(face: Face, scale: f32, pw: u32, ph: u32) -> (u32, u32, u32) {
    let (x0, y0, x1, y1) = (
        face.x0 / scale,
        face.y0 / scale,
        face.x1 / scale,
        face.y1 / scale,
    );
    let (fw, fh) = (x1 - x0, y1 - y0);
    let side = (fw.max(fh) * CROP_SCALE)
        .round()
        .max(1.0)
        .min(pw.min(ph) as f32) as u32;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0 + 0.05 * fh);
    let place = |c: f32, limit: u32| -> u32 {
        let start = (c - side as f32 / 2.0).round();
        start.clamp(0.0, (limit - side) as f32) as u32
    };
    (place(cx, pw), place(cy, ph), side)
}

/// What an anchor lookup came to, for the manifest when it is not a crop.
#[derive(Debug, Clone, PartialEq)]
pub enum Anchor {
    Crop(Vec<u8>),
    /// The detector ran and found no face in the portrait.
    NoFace,
    /// It could not run, and why — said, never silently drawn without.
    Unavailable(String),
}

/// Where an anchor crop comes from. A trait so tests can stand in for the
/// detector, which needs 89 MB of weights.
pub trait FaceAnchors: Send + Sync {
    fn anchor(&self, library: &crate::imagelib::Library, entry: &crate::imagelib::Entry) -> Anchor;
}

/// The real source: the cached crop of a portrait, else RetinaFace on it.
pub struct CachedRetinaFace;

/// Where a portrait's crop, or the record that it has no face, is cached:
/// `faces/` beside `blobs/`, named by the portrait's blob. Derived and
/// regenerable — blob collection never walks it, and a crop of a portrait
/// that was removed is simply never asked for again.
pub fn cache_paths(library_dir: &Path, blob: &str) -> (PathBuf, PathBuf) {
    let stem = blob.split('.').next().unwrap_or(blob);
    let dir = library_dir.join("faces");
    (
        dir.join(format!("{stem}.png")),
        dir.join(format!("{stem}.none")),
    )
}

impl FaceAnchors for CachedRetinaFace {
    fn anchor(&self, library: &crate::imagelib::Library, entry: &crate::imagelib::Entry) -> Anchor {
        let Some(blob) = entry.portrait.as_deref() else {
            return Anchor::Unavailable("the character has no portrait".into());
        };
        let (png, none) = cache_paths(library.dir(), blob);
        if let Ok(bytes) = std::fs::read(&png) {
            return Anchor::Crop(bytes);
        }
        if none.exists() {
            return Anchor::NoFace;
        }
        let made = (|| -> Result<Anchor> {
            let Some(weights) = weights()? else {
                return Ok(Anchor::Unavailable(format!(
                    "the face detector is not installed ({INSTALL_HINT})"
                )));
            };
            // Re-hashed on read: a portrait whose bytes no longer match its
            // blob name is refused, as it is for a cast.
            let (bytes, _) = library.read_portrait(entry)?;
            let detector = Detector::load(&weights)?;
            let anchor = match anchor_crop(&detector, &bytes)? {
                Some(crop) => {
                    crate::imagelib::write_atomic_mode(&png, &crop, Some(0o600))?;
                    Anchor::Crop(crop)
                }
                None => {
                    crate::imagelib::write_atomic_mode(&none, b"", Some(0o600))?;
                    Anchor::NoFace
                }
            };
            Ok(anchor)
        })();
        made.unwrap_or_else(|e| Anchor::Unavailable(format!("{e:#}")))
    }
}

/// Anchors, decoded boxes, NMS: py-feat's `generate_priors`, `decode_boxes`
/// and post-processing, on one image. `loc` is four regressions per anchor
/// and `conf` the face probability per anchor, in the order the heads emit
/// them.
fn decode(loc: &[[f32; 4]], conf: &[f32], w: usize, h: usize) -> Vec<Face> {
    let pri = priors(h, w);
    let mut found: Vec<Face> = Vec::new();
    for (i, p) in pri.iter().enumerate().take(loc.len().min(conf.len())) {
        let score = conf[i];
        if score <= 0.02 {
            continue;
        }
        let l = &loc[i];
        let (cx, cy) = (p[0] + l[0] * 0.1 * p[2], p[1] + l[1] * 0.1 * p[3]);
        let (bw, bh) = (p[2] * (l[2] * 0.2).exp(), p[3] * (l[3] * 0.2).exp());
        found.push(Face {
            x0: (cx - bw / 2.0) * w as f32,
            y0: (cy - bh / 2.0) * h as f32,
            x1: (cx + bw / 2.0) * w as f32,
            y1: (cy + bh / 2.0) * h as f32,
            score,
        });
    }
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Face> = Vec::new();
    for f in found {
        if kept.iter().all(|k| iou(k, &f) <= 0.4) {
            kept.push(f);
        }
    }
    kept.retain(|f| f.score >= 0.5);
    kept
}

/// The anchor boxes, (cx, cy, w, h) in normalised coordinates, in the order
/// the heads emit them: level, row, column, size.
fn priors(h: usize, w: usize) -> Vec<[f32; 4]> {
    const MIN_SIZES: [[f32; 2]; 3] = [[16.0, 32.0], [64.0, 128.0], [256.0, 512.0]];
    const STEPS: [f32; 3] = [8.0, 16.0, 32.0];
    let mut out = Vec::new();
    for k in 0..3 {
        let fh = (h as f32 / STEPS[k]).ceil() as usize;
        let fw = (w as f32 / STEPS[k]).ceil() as usize;
        for i in 0..fh {
            for j in 0..fw {
                for m in MIN_SIZES[k] {
                    out.push([
                        (j as f32 + 0.5) * STEPS[k] / w as f32,
                        (i as f32 + 0.5) * STEPS[k] / h as f32,
                        m / w as f32,
                        m / h as f32,
                    ]);
                }
            }
        }
    }
    out
}

fn iou(a: &Face, b: &Face) -> f32 {
    let inter = Face {
        x0: a.x0.max(b.x0),
        y0: a.y0.max(b.y0),
        x1: a.x1.min(b.x1),
        y1: a.y1.min(b.y1),
        score: 0.0,
    }
    .area();
    inter / (a.area() + b.area() - inter)
}

// The network: py-feat's `Retinaface_model.RetinaFace`, layer for layer, its
// safetensors read by name. Every activation is a ReLU — the leaky slope is
// 0 at 128 channels, as there.

/// The f32 tensors of a safetensors file, by name: an eight-byte header
/// length, a JSON header of `{name: {dtype, shape, data_offsets}}`, then the
/// little-endian data. Other dtypes (BatchNorm's `num_batches_tracked`) are
/// skipped.
fn read_safetensors(bytes: &[u8]) -> Result<Tensors> {
    let len = u64::from_le_bytes(
        bytes
            .get(..8)
            .ok_or_else(|| anyhow!("too short"))?
            .try_into()?,
    ) as usize;
    let header_end = 8usize
        .checked_add(len)
        .filter(|&e| e <= bytes.len())
        .ok_or_else(|| anyhow!("a header longer than the file"))?;
    let header: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&bytes[8..header_end])?;
    let data = &bytes[header_end..];
    let mut out = HashMap::new();
    for (name, t) in &header {
        if name == "__metadata__" || t["dtype"] != "F32" {
            continue;
        }
        let shape: Vec<usize> = serde_json::from_value(t["shape"].clone())?;
        let [start, end]: [usize; 2] = serde_json::from_value(t["data_offsets"].clone())?;
        let raw = data
            .get(start..end)
            .filter(|r| r.len() == 4 * shape.iter().product::<usize>())
            .ok_or_else(|| anyhow!("`{name}`'s data is not where its header says"))?;
        let v = raw
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        out.insert(name.clone(), (shape, v));
    }
    Ok(out)
}

/// One image's feature map: channels × height × width, row-major.
#[derive(Debug, Clone, PartialEq)]
struct Map {
    c: usize,
    h: usize,
    w: usize,
    v: Vec<f32>,
}

impl Map {
    fn relu(mut self) -> Map {
        self.v.iter_mut().for_each(|x| *x = x.max(0.0));
        self
    }

    fn add(mut self, other: &Map) -> Map {
        debug_assert_eq!((self.c, self.h, self.w), (other.c, other.h, other.w));
        self.v.iter_mut().zip(&other.v).for_each(|(a, b)| *a += b);
        self
    }

    /// Channels of each, in order (`torch.cat(dim=1)`).
    fn cat(parts: &[Map]) -> Map {
        let (h, w) = (parts[0].h, parts[0].w);
        Map {
            c: parts.iter().map(|p| p.c).sum(),
            h,
            w,
            v: parts.iter().flat_map(|p| p.v.iter().copied()).collect(),
        }
    }

    /// Max-pool 3×3, stride 2, padding 1, padding never chosen.
    fn max_pool(&self) -> Map {
        let (oh, ow) = ((self.h - 1) / 2 + 1, (self.w - 1) / 2 + 1);
        let mut v = vec![f32::NEG_INFINITY; self.c * oh * ow];
        for c in 0..self.c {
            for oy in 0..oh {
                for ox in 0..ow {
                    let o = &mut v[(c * oh + oy) * ow + ox];
                    for ky in 0..3 {
                        let Some(iy) = (2 * oy + ky).checked_sub(1).filter(|&y| y < self.h) else {
                            continue;
                        };
                        for kx in 0..3 {
                            let Some(ix) = (2 * ox + kx).checked_sub(1).filter(|&x| x < self.w)
                            else {
                                continue;
                            };
                            *o = o.max(self.v[(c * self.h + iy) * self.w + ix]);
                        }
                    }
                }
            }
        }
        Map {
            c: self.c,
            h: oh,
            w: ow,
            v,
        }
    }

    /// Nearest-neighbour resize to `h`×`w`, as PyTorch's `interpolate(mode =
    /// "nearest")` picks: source index `floor(dst * in / out)`.
    fn upsample(&self, h: usize, w: usize) -> Map {
        let mut v = Vec::with_capacity(self.c * h * w);
        for c in 0..self.c {
            for y in 0..h {
                let sy = ((y as f32 * self.h as f32 / h as f32) as usize).min(self.h - 1);
                for x in 0..w {
                    let sx = ((x as f32 * self.w as f32 / w as f32) as usize).min(self.w - 1);
                    v.push(self.v[(c * self.h + sy) * self.w + sx]);
                }
            }
        }
        Map { c: self.c, h, w, v }
    }
}

/// The weights `take` has not yet claimed, by name.
type Tensors = HashMap<String, (Vec<usize>, Vec<f32>)>;

fn take(t: &mut Tensors, name: &str) -> Result<(Vec<usize>, Vec<f32>)> {
    t.remove(name)
        .ok_or_else(|| anyhow!("the weights have no `{name}`"))
}

/// A convolution with its batch norm folded in, and an optional ReLU.
struct Conv {
    cout: usize,
    cin: usize,
    k: usize,
    stride: usize,
    pad: usize,
    /// `cout` rows of `cin·k·k`.
    w: Vec<f32>,
    b: Vec<f32>,
    relu: bool,
}

/// How many floats one im2col slab may hold before a convolution is done in
/// bands of output rows: bounds the memory a large picture costs (layer1 of
/// a 1024² picture is 37 million otherwise).
const COLS_BUDGET: usize = 1 << 22;

impl Conv {
    /// `conv` (weights, and `bias` if it has one), followed by the batch
    /// norm `bn` if there is one, folded: `w·γ/σ` and `β − μ·γ/σ`.
    fn load(
        t: &mut Tensors,
        conv: &str,
        bn: Option<&str>,
        stride: usize,
        relu: bool,
    ) -> Result<Conv> {
        let (shape, mut w) = take(t, &format!("{conv}.weight"))?;
        let [cout, cin, k, k2] = shape[..] else {
            bail!("`{conv}.weight` is not four-dimensional");
        };
        if k != k2 {
            bail!("`{conv}.weight` is not square");
        }
        let mut b = match t.remove(&format!("{conv}.bias")) {
            Some((_, b)) => b,
            None => vec![0.0; cout],
        };
        if let Some(bn) = bn {
            let (_, gamma) = take(t, &format!("{bn}.weight"))?;
            let (_, beta) = take(t, &format!("{bn}.bias"))?;
            let (_, mean) = take(t, &format!("{bn}.running_mean"))?;
            let (_, var) = take(t, &format!("{bn}.running_var"))?;
            let per = cin * k * k;
            for o in 0..cout {
                let s = gamma[o] / (var[o] + 1e-5).sqrt();
                w[o * per..(o + 1) * per].iter_mut().for_each(|x| *x *= s);
                b[o] = beta[o] + (b[o] - mean[o]) * s;
            }
        }
        Ok(Conv {
            cout,
            cin,
            k,
            stride,
            pad: (k - 1) / 2,
            w,
            b,
            relu,
        })
    }

    fn run(&self, x: &Map) -> Map {
        self.run_in(x, COLS_BUDGET)
    }

    /// Im2col and one matrix product per band of output rows.
    fn run_in(&self, x: &Map, budget: usize) -> Map {
        debug_assert_eq!(x.c, self.cin);
        let (k, s, p) = (self.k, self.stride, self.pad);
        let oh = (x.h + 2 * p - k) / s + 1;
        let ow = (x.w + 2 * p - k) / s + 1;
        let kk = self.cin * k * k;
        let mut out = vec![0f32; self.cout * oh * ow];
        let rows = (budget / (kk * ow).max(1)).clamp(1, oh);
        let mut cols = Vec::new();
        for r0 in (0..oh).step_by(rows) {
            let r1 = (r0 + rows).min(oh);
            let n = (r1 - r0) * ow;
            let direct = k == 1 && s == 1 && p == 0 && rows == oh;
            let b: &[f32] = if direct {
                &x.v
            } else {
                cols.clear();
                cols.resize(kk * n, 0.0);
                for ci in 0..self.cin {
                    for ky in 0..k {
                        for kx in 0..k {
                            let row = &mut cols[((ci * k + ky) * k + kx) * n..][..n];
                            for oy in r0..r1 {
                                let Some(iy) = (oy * s + ky).checked_sub(p).filter(|&y| y < x.h)
                                else {
                                    continue;
                                };
                                let src = &x.v[(ci * x.h + iy) * x.w..][..x.w];
                                let dst = &mut row[(oy - r0) * ow..][..ow];
                                for (ox, d) in dst.iter_mut().enumerate() {
                                    if let Some(ix) =
                                        (ox * s + kx).checked_sub(p).filter(|&ix| ix < x.w)
                                    {
                                        *d = src[ix];
                                    }
                                }
                            }
                        }
                    }
                }
                &cols
            };
            // SAFETY: `w` is cout×kk, `b` kk×n and the output block cout×n at
            // row stride oh·ow inside `out`, all in bounds by construction.
            unsafe {
                matrixmultiply::sgemm(
                    self.cout,
                    kk,
                    n,
                    1.0,
                    self.w.as_ptr(),
                    kk as isize,
                    1,
                    b.as_ptr(),
                    n as isize,
                    1,
                    0.0,
                    out.as_mut_ptr().add(r0 * ow),
                    (oh * ow) as isize,
                    1,
                );
            }
        }
        for (o, plane) in out.chunks_exact_mut(oh * ow).enumerate() {
            let bias = self.b[o];
            if self.relu {
                plane.iter_mut().for_each(|v| *v = (*v + bias).max(0.0));
            } else {
                plane.iter_mut().for_each(|v| *v += bias);
            }
        }
        Map {
            c: self.cout,
            h: oh,
            w: ow,
            v: out,
        }
    }
}

/// torchvision's `BasicBlock`.
struct Basic {
    conv1: Conv,
    conv2: Conv,
    down: Option<Conv>,
}

impl Basic {
    fn load(t: &mut Tensors, at: &str, stride: usize, down: bool) -> Result<Basic> {
        Ok(Basic {
            conv1: Conv::load(
                t,
                &format!("{at}.conv1"),
                Some(&format!("{at}.bn1")),
                stride,
                true,
            )?,
            conv2: Conv::load(
                t,
                &format!("{at}.conv2"),
                Some(&format!("{at}.bn2")),
                1,
                false,
            )?,
            down: down
                .then(|| {
                    Conv::load(
                        t,
                        &format!("{at}.downsample.0"),
                        Some(&format!("{at}.downsample.1")),
                        stride,
                        false,
                    )
                })
                .transpose()?,
        })
    }

    fn run(&self, x: &Map) -> Map {
        let y = self.conv2.run(&self.conv1.run(x));
        match &self.down {
            Some(d) => y.add(&d.run(x)),
            None => y.add(x),
        }
        .relu()
    }
}

/// Single Stage Headless context module.
struct Ssh {
    c3: Conv,
    c5_1: Conv,
    c5_2: Conv,
    c7_2: Conv,
    c7_3: Conv,
}

impl Ssh {
    fn load(t: &mut Tensors, at: &str) -> Result<Ssh> {
        let mut cb = |name: &str, relu: bool| {
            Conv::load(
                t,
                &format!("{at}.{name}.0"),
                Some(&format!("{at}.{name}.1")),
                1,
                relu,
            )
        };
        Ok(Ssh {
            c3: cb("conv3X3", false)?,
            c5_1: cb("conv5X5_1", true)?,
            c5_2: cb("conv5X5_2", false)?,
            c7_2: cb("conv7X7_2", true)?,
            c7_3: cb("conv7x7_3", false)?,
        })
    }

    fn run(&self, x: &Map) -> Map {
        // py-feat feeds `conv5X5_1(x)` to both branches (upstream's quirk,
        // kept there for the trained weights); computed once here.
        let t = self.c5_1.run(x);
        Map::cat(&[
            self.c3.run(x),
            self.c5_2.run(&t),
            self.c7_3.run(&self.c7_2.run(&t)),
        ])
        .relu()
    }
}

struct Net {
    conv1: Conv,
    layers: Vec<Vec<Basic>>,
    outputs: [Conv; 3],
    merge1: Conv,
    merge2: Conv,
    ssh: [Ssh; 3],
    bbox: Vec<Conv>,
    class: Vec<Conv>,
}

impl Net {
    fn load(t: &mut Tensors) -> Result<Net> {
        let mut layers = Vec::new();
        for (i, (blocks, stride)) in [(3, 1), (4, 2), (6, 2), (3, 2)].into_iter().enumerate() {
            let mut layer = Vec::new();
            for b in 0..blocks {
                let first = b == 0;
                layer.push(Basic::load(
                    t,
                    &format!("body.layer{}.{b}", i + 1),
                    if first { stride } else { 1 },
                    first && i > 0,
                )?);
            }
            layers.push(layer);
        }
        let mut fpn = |name: &str| {
            Conv::load(
                t,
                &format!("fpn.{name}.0"),
                Some(&format!("fpn.{name}.1")),
                1,
                true,
            )
        };
        let outputs = [fpn("output1")?, fpn("output2")?, fpn("output3")?];
        let (merge1, merge2) = (fpn("merge1")?, fpn("merge2")?);
        let mut head = |name: &str| -> Result<Vec<Conv>> {
            (0..3)
                .map(|i| Conv::load(t, &format!("{name}._convs.{i}"), None, 1, false))
                .collect()
        };
        let (bbox, class) = (head("BboxHead")?, head("ClassHead")?);
        Ok(Net {
            conv1: Conv::load(t, "body.conv1", Some("body.bn1"), 2, true)?,
            layers,
            outputs,
            merge1,
            merge2,
            ssh: [
                Ssh::load(t, "ssh1")?,
                Ssh::load(t, "ssh2")?,
                Ssh::load(t, "ssh3")?,
            ],
            bbox,
            class,
        })
    }

    /// Box regressions and face probabilities, one per anchor in the order
    /// the heads emit them: level, row, column, anchor.
    fn forward(&self, x: &Map) -> (Vec<[f32; 4]>, Vec<f32>) {
        let mut y = self.conv1.run(x).max_pool();
        let mut taps = Vec::new();
        for (i, layer) in self.layers.iter().enumerate() {
            for block in layer {
                y = block.run(&y);
            }
            if i >= 1 {
                taps.push(y.clone());
            }
        }
        let o1 = self.outputs[0].run(&taps[0]);
        let o2 = self.outputs[1].run(&taps[1]);
        let o3 = self.outputs[2].run(&taps[2]);
        let o2 = self.merge2.run(&o2.clone().add(&o3.upsample(o2.h, o2.w)));
        let o1 = self.merge1.run(&o1.clone().add(&o2.upsample(o1.h, o1.w)));
        let features = [
            self.ssh[0].run(&o1),
            self.ssh[1].run(&o2),
            self.ssh[2].run(&o3),
        ];
        let (mut loc, mut conf) = (Vec::new(), Vec::new());
        for (f, (b, c)) in features.iter().zip(self.bbox.iter().zip(&self.class)) {
            let (bm, cm) = (b.run(f), c.run(f));
            let plane = f.h * f.w;
            for pos in 0..plane {
                for a in 0..2 {
                    let at = |m: &Map, per: usize, j: usize| m.v[(a * per + j) * plane + pos];
                    loc.push([at(&bm, 4, 0), at(&bm, 4, 1), at(&bm, 4, 2), at(&bm, 4, 3)]);
                    // Softmax over (not face, face), as its second half.
                    conf.push(1.0 / (1.0 + (at(&cm, 2, 0) - at(&cm, 2, 1)).exp()));
                }
            }
        }
        (loc, conf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic spread of values, so the tests need no RNG crate.
    fn values(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s % 2001) as f32 / 1000.0 - 1.0
            })
            .collect()
    }

    /// The reference: every output element summed directly from the
    /// definition, padding read as zero.
    fn naive(conv: &Conv, x: &Map) -> Map {
        let (k, s, p) = (conv.k, conv.stride, conv.pad);
        let oh = (x.h + 2 * p - k) / s + 1;
        let ow = (x.w + 2 * p - k) / s + 1;
        let mut v = Vec::new();
        for o in 0..conv.cout {
            for oy in 0..oh {
                for ox in 0..ow {
                    let mut acc = conv.b[o];
                    for ci in 0..conv.cin {
                        for ky in 0..k {
                            for kx in 0..k {
                                let (iy, ix) = (oy * s + ky, ox * s + kx);
                                if iy < p || ix < p || iy - p >= x.h || ix - p >= x.w {
                                    continue;
                                }
                                acc += conv.w[((o * conv.cin + ci) * k + ky) * k + kx]
                                    * x.v[(ci * x.h + iy - p) * x.w + ix - p];
                            }
                        }
                    }
                    v.push(if conv.relu { acc.max(0.0) } else { acc });
                }
            }
        }
        Map {
            c: conv.cout,
            h: oh,
            w: ow,
            v,
        }
    }

    /// Every convolution shape the network uses — 7×7 stride 2, 3×3 stride
    /// 1 and 2, 1×1 stride 1 and 2, with and without ReLU — against the
    /// definition, on uneven sizes, whole and in bands of one and three
    /// output rows: the band seams are where an im2col goes wrong. (candle
    /// 0.9.2's convolution was silently wrong at one shape, which is why
    /// the shapes are checked rather than trusted, 2026-10-05.)
    #[test]
    fn the_convolution_matches_its_definition_at_every_shape_used() {
        for (i, (cin, cout, k, stride, relu)) in [
            (3, 4, 7, 2, true),
            (5, 6, 3, 1, false),
            (5, 6, 3, 2, true),
            (4, 3, 1, 1, false),
            (4, 3, 1, 2, false),
        ]
        .into_iter()
        .enumerate()
        {
            let conv = Conv {
                cout,
                cin,
                k,
                stride,
                pad: (k - 1) / 2,
                w: values(cout * cin * k * k, i as u32),
                b: values(cout, 100 + i as u32),
                relu,
            };
            let x = Map {
                c: cin,
                h: 13,
                w: 11,
                v: values(cin * 13 * 11, 200 + i as u32),
            };
            let want = naive(&conv, &x);
            for budget in [COLS_BUDGET, cin * k * k * 11, 3 * cin * k * k * 11] {
                let got = conv.run_in(&x, budget);
                assert_eq!((got.c, got.h, got.w), (want.c, want.h, want.w));
                let worst = got
                    .v
                    .iter()
                    .zip(&want.v)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0, f32::max);
                assert!(worst < 1e-4, "shape {i}, budget {budget}: off by {worst}");
            }
        }
    }

    #[test]
    fn batch_norm_folds_into_the_convolution() {
        let mut t = Tensors::new();
        t.insert("c.weight".into(), (vec![2, 1, 1, 1], vec![1.0, 2.0]));
        t.insert("n.weight".into(), (vec![2], vec![2.0, 1.0]));
        t.insert("n.bias".into(), (vec![2], vec![0.5, -1.0]));
        t.insert("n.running_mean".into(), (vec![2], vec![1.0, 0.0]));
        t.insert(
            "n.running_var".into(),
            (vec![2], vec![4.0 - 1e-5, 1.0 - 1e-5]),
        );
        let conv = Conv::load(&mut t, "c", Some("n"), 1, false).unwrap();
        let x = Map {
            c: 1,
            h: 1,
            w: 1,
            v: vec![3.0],
        };
        // Channel 0: ((3·1 − 1) / 2)·2 + 0.5; channel 1: (3·2 − 0)·1 − 1.
        let y = conv.run(&x).v;
        assert!(
            (y[0] - 2.5).abs() < 1e-4 && (y[1] - 5.0).abs() < 1e-4,
            "{y:?}"
        );
        assert!(t.is_empty(), "every tensor the layer names was used");
    }

    #[test]
    fn pooling_and_upsampling_follow_pytorch() {
        let x = Map {
            c: 1,
            h: 4,
            w: 5,
            v: (0..20).map(|i| i as f32).collect(),
        };
        // 3×3, stride 2, padding 1 on 4×5: 2×3 windows.
        assert_eq!(x.max_pool().v, vec![6.0, 8.0, 9.0, 16.0, 18.0, 19.0]);
        let small = Map {
            c: 1,
            h: 2,
            w: 2,
            v: vec![1.0, 2.0, 3.0, 4.0],
        };
        assert_eq!(
            small.upsample(4, 4).v,
            vec![1., 1., 2., 2., 1., 1., 2., 2., 3., 3., 4., 4., 3., 3., 4., 4.]
        );
        // An uneven ratio picks floor(dst·in/out), as `interpolate` does.
        let row = Map {
            c: 1,
            h: 1,
            w: 3,
            v: vec![1.0, 2.0, 3.0],
        };
        assert_eq!(row.upsample(1, 5).v, vec![1., 1., 2., 2., 3.]);
    }

    #[test]
    fn a_safetensors_file_is_read_and_a_short_one_refused() {
        let header = br#"{"a":{"dtype":"F32","shape":[2],"data_offsets":[0,8]},"n":{"dtype":"I64","shape":[],"data_offsets":[8,16]}}"#;
        let mut file = (header.len() as u64).to_le_bytes().to_vec();
        file.extend_from_slice(header);
        file.extend_from_slice(&1.5f32.to_le_bytes());
        file.extend_from_slice(&(-2.0f32).to_le_bytes());
        file.extend_from_slice(&7i64.to_le_bytes());
        let t = read_safetensors(&file).unwrap();
        assert_eq!(t["a"], (vec![2], vec![1.5, -2.0]));
        assert!(!t.contains_key("n"), "a counter is not a weight");
        assert!(read_safetensors(&file[..file.len() - 12]).is_err());
        assert!(read_safetensors(b"\xff\xff\xff\xff\xff\xff\xff\x7f").is_err());
    }

    #[test]
    fn priors_follow_the_three_levels_two_sizes_each() {
        // 1024²: 128², 64² and 32² cells, two anchors each.
        assert_eq!(
            priors(1024, 1024).len(),
            2 * (128 * 128 + 64 * 64 + 32 * 32)
        );
        // Uneven sides round up, as py-feat's `ceil` does.
        assert_eq!(priors(100, 60).len(), 2 * (13 * 8 + 7 * 4 + 4 * 2));
        let first = priors(1024, 1024)[0];
        assert_eq!(
            first,
            [4.0 / 1024.0, 4.0 / 1024.0, 16.0 / 1024.0, 16.0 / 1024.0]
        );
    }

    #[test]
    fn overlapping_boxes_keep_the_best_and_weak_ones_are_dropped() {
        let (w, h) = (64, 64);
        let n = priors(h, w).len();
        let mut loc = vec![[0.0; 4]; n];
        let mut conf = vec![0.0; n];
        conf[0] = 0.9;
        // The same cell's larger anchor, shrunk by its regression to the
        // smaller one's box: a duplicate of it.
        conf[1] = 0.8;
        let half = (0.5f32).ln() / 0.2;
        loc[1] = [0.0, 0.0, half, half];
        // Alone, but under 0.5 after NMS.
        conf[n - 1] = 0.4;
        let faces = decode(&loc, &conf, w, h);
        assert_eq!(faces.len(), 1, "{faces:?}");
        assert!((faces[0].score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn the_crop_is_square_tight_and_inside_the_picture() {
        let face = Face {
            x0: 100.0,
            y0: 50.0,
            x1: 200.0,
            y1: 180.0,
            score: 1.0,
        };
        let (x, y, side) = crop_square(face, 1.0, 1024, 1024);
        // CROP_SCALE times the larger side (130), centred 5% of the height
        // lower than the box's centre.
        assert_eq!(side, 146);
        assert_eq!((x, y), (77, 49));
        // At the picture's edge it shifts in rather than shrinking.
        let edge = Face {
            x0: 0.0,
            y0: 0.0,
            x1: 100.0,
            y1: 100.0,
            score: 1.0,
        };
        assert_eq!(crop_square(edge, 1.0, 400, 300), (0, 0, 112));
        // Found on a half-size copy: mapped back to the picture's pixels.
        let half = Face {
            x0: 50.0,
            y0: 25.0,
            x1: 100.0,
            y1: 90.0,
            score: 1.0,
        };
        assert_eq!(
            crop_square(half, 0.5, 1024, 1024),
            crop_square(face, 1.0, 1024, 1024)
        );
        // A picture smaller than the crop gives a crop of the picture's size.
        assert_eq!(crop_square(face, 1.0, 120, 90).2, 90);
    }

    /// The detector against py-feat itself: `MECHA_FACE_PARITY` names a JSON
    /// map of `{name: {"path": picture, "boxes": [[x0, y0, x1, y1, score]]}}`
    /// written by py-feat's own `Retinaface` on the same pictures, and every
    /// box must agree within a pixel — none where it found none. Needs the
    /// installed weights, so it is not part of the default run.
    #[test]
    #[ignore = "model: needs the installed face detector and MECHA_FACE_PARITY"]
    fn the_detector_agrees_with_py_feat() {
        let reference = std::env::var("MECHA_FACE_PARITY").expect("MECHA_FACE_PARITY");
        let weights = weights().unwrap().expect("the face detector is installed");
        let detector = Detector::load(&weights).unwrap();
        let cases: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(reference).unwrap()).unwrap();
        for (name, case) in cases.as_object().unwrap() {
            let img = image::open(case["path"].as_str().unwrap())
                .unwrap()
                .to_rgb8();
            let started = std::time::Instant::now();
            let got = detector.detect(&img).unwrap();
            eprintln!("{name}: {} ms", started.elapsed().as_millis());
            let want = case["boxes"].as_array().unwrap();
            assert_eq!(got.len(), want.len(), "{name}: {got:?} against {want:?}");
            for (g, w) in got.iter().zip(want) {
                for (i, v) in [g.x0, g.y0, g.x1, g.y1].iter().enumerate() {
                    let d = (*v as f64 - w[i].as_f64().unwrap()).abs();
                    assert!(d <= 1.0, "{name}: {got:?} against {want:?}");
                }
            }
        }
    }

    #[test]
    fn the_cache_is_named_by_the_portrait_blob() {
        let (png, none) = cache_paths(Path::new("/lib"), "sha256-abc.jpg");
        assert_eq!(png, Path::new("/lib/faces/sha256-abc.png"));
        assert_eq!(none, Path::new("/lib/faces/sha256-abc.none"));
    }
}
