//! Turning a file on disk into an image block a provider will accept.
//!
//! One function does the whole job, and it is here rather than at each entry
//! point because there are three of those already — the Slack connector, the
//! TUI, and whatever comes next — and the caps below are the kind of number
//! that gets copied once and then diverges.
//!
//! Two caps, and they are enforced for different reasons:
//!
//! - **[`MAX_BYTES`] is a provider limit.** Anthropic rejects any single
//!   image over 5 MB outright. llama-server does not care — measured here, a
//!   5.6 MB PNG went through and cost ~256 prompt tokens, because the server
//!   tiles it before the model ever sees it. So the cap is not about context
//!   at all: it is the smaller of what the two backends accept, applied to
//!   both, because a conversation is one object and a `/model` switch must
//!   not turn a working transcript into a rejected request.
//! - **[`MAX_EDGE`] is about what is worth carrying.** Above roughly this,
//!   both families downsample server-side anyway, so the extra pixels buy
//!   nothing and are paid for twice — once on the wire, and once *for the
//!   life of the session*, because the transcript is append-only and every
//!   turn resends the whole history.
//!
//! The second cost is the one that decides the shape here. A resized image
//! is what gets recorded, never the original, so the bill is paid once at
//! the door rather than on every turn afterwards.

use crate::message::{image_media_type, Block};
use anyhow::{bail, Context, Result};
use std::path::Path;

/// The largest encoded image any provider here will be handed.
///
/// Anthropic's documented hard limit. Deliberately applied to local servers
/// too — see the module docs.
pub const MAX_BYTES: usize = 5 * 1024 * 1024;

/// Longest edge kept. Both provider families downsample above about this, so
/// pixels beyond it are re-sent every turn and never looked at.
pub const MAX_EDGE: u32 = 1568;

/// At or under this, a picture a tool shows the model is passed through as
/// it is ([`rendered_block`]). 1 MiB: above a typical screenshot of text, and
/// below every generated picture measured here.
pub const PASS_THROUGH_BYTES: usize = 1024 * 1024;

/// What a re-encode costs in fidelity. 85 is the usual "cannot tell without
/// looking for it" point, and the thing being carried is almost always a
/// screenshot of text, where the artefacts that matter are the ones that
/// close up a glyph.
const JPEG_QUALITY: u8 = 85;

/// Read `path` and produce an image block bounded by the caps above.
///
/// **Untouched when it already fits.** A small PNG is passed through byte for
/// byte rather than round-tripped through a decoder — re-encoding a crisp
/// screenshot of text as JPEG to no purpose is a real loss, and it is the
/// exact case this is most often used for.
///
/// Returns `Ok(None)` when the extension is not one both backends read, so a
/// caller can say "here is a path" for a PDF instead of failing.
pub fn block_from_path(path: &Path) -> Result<Option<Block>> {
    let Some(media_type) = image_media_type(path) else {
        return Ok(None);
    };
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    block_from_bytes(media_type, bytes, name, &path.display().to_string()).map(Some)
}

/// [`block_from_path`] for bytes a caller already read — through
/// `WorkspaceFiles::read`, say, whose descriptor-held walk a second open by
/// path would undo. `media_type` is the caller's (from the name, as
/// [`image_media_type`] decides it); `what` names the file in errors.
pub fn block_from_bytes(
    media_type: &'static str,
    bytes: Vec<u8>,
    name: Option<String>,
    what: &str,
) -> Result<Block> {
    // Dimensions are read from the header alone, so the common case — an
    // image that is already small — never pays to decode the pixels.
    let dims = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok());
    let oversized = dims.is_some_and(|(w, h)| w.max(h) > MAX_EDGE);
    if !oversized && bytes.len() <= MAX_BYTES {
        return Ok(Block::image(media_type, &bytes, name));
    }

    let img = image::load_from_memory(&bytes)
        .with_context(|| format!("{what} is named as an image but did not decode"))?;
    // `thumbnail` preserves the aspect ratio and takes the *bound* rather
    // than a target, so an image that is oversized in only one dimension is
    // not stretched to fill the other.
    let img = img.thumbnail(MAX_EDGE, MAX_EDGE);

    let mut out = Vec::new();
    // JPEG regardless of what came in. The alternative — keeping PNG — makes
    // the size of the result depend on the *content*: a photograph of a
    // screen, which is the case that motivated all of this, is several
    // megabytes as a PNG at any resolution worth sending, so the resize
    // would leave it still over the cap and the failure would look like the
    // resize not working.
    img.to_rgb8()
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut out,
            JPEG_QUALITY,
        ))
        .context("re-encoding a resized image")?;

    if out.len() > MAX_BYTES {
        bail!(
            "{what} is {} after resizing to {MAX_EDGE}px and stays above the {} limit",
            human(out.len()),
            human(MAX_BYTES),
        );
    }
    Ok(Block::image("image/jpeg", &out, name))
}

/// A picture a tool put in front of the model, as an image block.
///
/// **Passed through byte for byte when it is small**, as [`block_from_path`]
/// does, because a small file is most often a screenshot of text, where JPEG's
/// artefacts close up glyphs and reading them is what the look is for (found
/// on review of #365). **Re-encoded otherwise**, including a picture that fits
/// both caps: a generated picture is a photograph's kind of content, where
/// JPEG costs nothing anyone checking it would see, and its PNG is not small —
/// the first twenty Qwen-Image results here ran 1.05–2.35 MB (median 1.8 MB),
/// every one above [`PASS_THROUGH_BYTES`], resent every turn for the rest of
/// the conversation. The file on disk stays the original.
pub fn rendered_block(bytes: &[u8], name: Option<String>) -> Result<Block> {
    let img = image::load_from_memory(bytes).context("the image did not decode")?;
    if bytes.len() <= PASS_THROUGH_BYTES && img.width().max(img.height()) <= MAX_EDGE {
        let media_type = match image::guess_format(bytes) {
            Ok(image::ImageFormat::Png) => "image/png",
            Ok(image::ImageFormat::Jpeg) => "image/jpeg",
            Ok(image::ImageFormat::Gif) => "image/gif",
            Ok(image::ImageFormat::WebP) => "image/webp",
            // Decoded, but not a type both backends read: re-encode below.
            _ => "",
        };
        if !media_type.is_empty() {
            return Ok(Block::image(media_type, bytes, name));
        }
    }
    let img = if img.width().max(img.height()) > MAX_EDGE {
        img.thumbnail(MAX_EDGE, MAX_EDGE)
    } else {
        img
    };
    let mut out = Vec::new();
    img.to_rgb8()
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut out,
            JPEG_QUALITY,
        ))
        .context("re-encoding a rendered image")?;
    if out.len() > MAX_BYTES {
        bail!(
            "the rendered image is {} as JPEG and stays above the {} limit",
            human(out.len()),
            human(MAX_BYTES),
        );
    }
    Ok(Block::image("image/jpeg", &out, name))
}

fn human(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Block;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn write(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    /// The case this whole path exists for: a small screenshot must reach the
    /// model exactly as it was taken. A re-encode here would blur the text
    /// that is the entire reason somebody sent a screenshot.
    #[test]
    fn an_image_that_already_fits_is_passed_through_byte_for_byte() {
        let dir = std::env::temp_dir().join(format!("mecha-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = png(64, 48);
        let p = write(&dir, "small.png", &bytes);

        let block = block_from_path(&p).unwrap().unwrap();
        let Block::Image {
            media_type,
            data,
            source,
        } = block
        else {
            panic!("expected an image block")
        };
        assert_eq!(media_type, "image/png", "the source format is kept");
        assert_eq!(source.as_deref(), Some("small.png"));

        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&data)
            .unwrap();
        assert_eq!(decoded, bytes, "the original bytes, not a re-encode");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Verified to fail on the old behaviour by construction: without the
    /// resize this block would carry a 4000px image, and the assertion is on
    /// the dimensions of what came back rather than merely on its size.
    #[test]
    fn an_oversized_image_is_resized_and_re_encoded() {
        let dir = std::env::temp_dir().join(format!("mecha-img-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = write(&dir, "huge.png", &png(4000, 2000));

        let block = block_from_path(&p).unwrap().unwrap();
        let Block::Image {
            media_type, data, ..
        } = block
        else {
            panic!("expected an image block")
        };
        assert_eq!(media_type, "image/jpeg", "a resize re-encodes");

        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&data)
            .unwrap();
        assert!(decoded.len() <= MAX_BYTES, "under the provider cap");
        let (w, h) = image::load_from_memory(&decoded)
            .map(|i| {
                (
                    image::GenericImageView::width(&i),
                    image::GenericImageView::height(&i),
                )
            })
            .unwrap();
        assert!(
            w.max(h) <= MAX_EDGE,
            "long edge {w}x{h} bounded by {MAX_EDGE}"
        );
        assert_eq!(w * 2000, h * 4000, "aspect ratio preserved, not stretched");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Photo-like noise: incompressible, so a 1024² PNG of it is several
    /// megabytes — the shape of a generated picture, which fits both caps.
    fn noisy_png(w: u32, h: u32) -> Vec<u8> {
        let mut x: u32 = 0x9E37_79B9;
        let img = image::RgbImage::from_fn(w, h, |_, _| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            image::Rgb([x as u8, (x >> 8) as u8, (x >> 16) as u8])
        });
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// A small picture — a screenshot of text, most often — is passed through
    /// byte for byte; a large one that fits both caps, as a generated picture
    /// does, is re-encoded; one past the edge is bounded. Fails on either
    /// flat rule: "always JPEG" blurs the screenshot, and `block_from_path`'s
    /// caps alone would carry a generation's multi-megabyte PNG every turn.
    #[test]
    fn a_small_picture_passes_through_and_a_large_one_is_re_encoded() {
        use base64::Engine as _;
        let small = png(400, 200);
        assert!(small.len() <= PASS_THROUGH_BYTES);
        let Block::Image {
            media_type,
            data,
            source,
        } = rendered_block(&small, Some("inbox/shot.png".into())).unwrap()
        else {
            panic!("expected an image block")
        };
        assert_eq!(media_type, "image/png");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(&data)
                .unwrap(),
            small,
            "byte for byte"
        );
        assert_eq!(source.as_deref(), Some("inbox/shot.png"));

        let generated = noisy_png(1024, 1024);
        assert!(generated.len() > PASS_THROUGH_BYTES, "{}", generated.len());
        let Block::Image { media_type, .. } = rendered_block(&generated, None).unwrap() else {
            panic!("expected an image block")
        };
        assert_eq!(media_type, "image/jpeg");

        let Block::Image { data, .. } = rendered_block(&png(3000, 1500), None).unwrap() else {
            panic!("expected an image block")
        };
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&data)
            .unwrap();
        let i = image::load_from_memory(&decoded).unwrap();
        let (w, h) = (
            image::GenericImageView::width(&i),
            image::GenericImageView::height(&i),
        );
        assert!(w.max(h) <= MAX_EDGE, "long edge {w}x{h} bounded");
        assert!(rendered_block(b"not a png", None).is_err());
    }

    /// A caller must be able to tell "not an image" from "an image that
    /// failed", because the first is a normal thing to attach and the answer
    /// to it is to name the path.
    #[test]
    fn a_file_that_is_not_an_image_is_none_rather_than_an_error() {
        let dir = std::env::temp_dir().join(format!("mecha-img-pdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = write(&dir, "report.pdf", b"%PDF-1.4");
        assert!(block_from_path(&p).unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The extension is a claim, not a fact — the file arrived from Slack.
    #[test]
    fn a_file_named_png_that_is_not_one_fails_loudly() {
        let dir = std::env::temp_dir().join(format!("mecha-img-lie-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = write(&dir, "lie.png", &vec![7u8; 6 * 1024 * 1024]);
        let err = block_from_path(&p).unwrap_err().to_string();
        assert!(err.contains("did not decode"), "got: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
