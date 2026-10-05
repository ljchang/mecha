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
    if image_media_type(path).is_none() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    block_from_bytes(bytes, name, &path.display().to_string()).map(Some)
}

/// [`block_from_path`] for bytes a caller already read — through
/// `WorkspaceFiles::read`, say, whose descriptor-held walk a second open by
/// path would undo. `what` names the file in errors.
///
/// **The bytes decide what it is, never the name.** A caller gates on the
/// extension ([`image_media_type`]) to decide whether to try at all; the
/// media type sent is the one the header says. Both halves are found on
/// review of #368, and both fail the same way: a block the provider rejects
/// rides into append-only history and fails every later request of that
/// conversation — reachable from Slack and the web chat.
pub fn block_from_bytes(bytes: Vec<u8>, name: Option<String>, what: &str) -> Result<Block> {
    // Format and dimensions from the header alone, so the common case — an
    // image that is already small — never pays to decode the pixels.
    let header = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| {
            let format = r.format()?;
            r.into_dimensions().ok().map(|dims| (format, dims))
        });
    // A header that cannot be read is not a picture, whatever its name says.
    let Some((format, (w, h))) = header else {
        bail!("{what} is named as an image but did not decode (its header could not be read)");
    };
    let oversized = w.max(h) > MAX_EDGE;
    // A real JPEG named `.png` is sent as a JPEG: the type the provider
    // checks is the type the bytes are. A format neither backend reads
    // (a TIFF named `.png`) is re-encoded below rather than passed through.
    if let Some(media_type) = media_type_of(format) {
        if !oversized && bytes.len() <= MAX_BYTES {
            return Ok(Block::image(media_type, &bytes, name));
        }
    }

    // Upright: the JPEG below carries no orientation tag, and a phone photo
    // is stored sideways with one (review of #560).
    let img = decode_upright(&bytes, what)?;
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
    // Decoded even when it will pass through untouched: the decode is the
    // proof it is a picture. A good header on a broken body would otherwise
    // ride into the transcript, and a picture the provider rejects fails
    // every later request of that conversation, not just this one.
    // Upright, for the same reason as `block_from_bytes`: a re-encode drops
    // the tag that said which way up the photo was.
    let img = decode_upright(bytes, "the picture")?;
    if bytes.len() <= PASS_THROUGH_BYTES && img.width().max(img.height()) <= MAX_EDGE {
        // Decoded, but not a type both backends read: re-encode below.
        if let Some(media_type) = image::guess_format(bytes).ok().and_then(media_type_of) {
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

/// The most pixels decoded, read from the header, and the allocation bound
/// a decode runs under. The header is the file's claim about itself, and a
/// few-hundred-kilobyte PNG can claim 40000×40000.
///
/// **What this adds, stated exactly** (corrected on review of #368): the
/// allocation bound is `image`'s own default (`Limits::default()` carries
/// `max_alloc: Some(512 MiB)`, and `load_from_memory` used it), so such a
/// file was already refused — as "did not decode", which is the wrong
/// diagnosis and the wrong fix. The pixel check says "too large to show"
/// instead, from the header, before any decoder runs; `max_alloc` is set
/// explicitly so the bound does not rest on a default nobody reads.
///
/// **By area, never by side** (found on review of #368): a per-side bound
/// refused a 1440×20000 full-page screenshot — ~100 MB decoded — which the
/// caps above exist to shrink and show. 128 megapixels is 512 MiB as 8-bit
/// RGBA; a 16-bit decode of the same size is twice that and is stopped by
/// `max_alloc`, not by this check.
const MAX_DECODE_PIXELS: u64 = 128 * 1024 * 1024;
const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;

/// Decode `bytes`, refusing at the header a picture past
/// [`MAX_DECODE_PIXELS`] — said as too large, not as a failed decode, since
/// the two lead to different fixes. `what` names it in either error.
pub(crate) fn decode(bytes: &[u8], what: &str) -> Result<image::DynamicImage> {
    decode_with(bytes, what, false)
}

/// [`decode`], turned the way the file's EXIF orientation says — the way a
/// browser shows it. A phone photo is stored sideways with a tag saying so,
/// and re-encoding it drops the tag, so anything that re-encodes a photo
/// must decode it this way or hand on a sideways picture.
pub(crate) fn decode_upright(bytes: &[u8], what: &str) -> Result<image::DynamicImage> {
    decode_with(bytes, what, true)
}

fn decode_with(bytes: &[u8], what: &str, upright: bool) -> Result<image::DynamicImage> {
    use image::ImageDecoder;
    let reader = || image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format();
    if let Ok((w, h)) = reader()
        .map_err(anyhow::Error::from)
        .and_then(|r| Ok(r.into_dimensions()?))
    {
        if u64::from(w) * u64::from(h) > MAX_DECODE_PIXELS {
            bail!(
                "{what} is {w}×{h} — too large to show (over {} megapixels)",
                MAX_DECODE_PIXELS / (1024 * 1024)
            );
        }
    }
    let mut reader = reader().with_context(|| format!("{what} could not be read"))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let failed = || format!("{what} is named as an image but did not decode");
    if !upright {
        return reader.decode().with_context(failed);
    }
    let mut decoder = reader.into_decoder().with_context(failed)?;
    // An unreadable tag is no tag: the picture as stored, as before.
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).with_context(failed)?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// The media type both backends read for a sniffed format, or `None` for
/// one they do not.
fn media_type_of(format: image::ImageFormat) -> Option<&'static str> {
    match format {
        image::ImageFormat::Png => Some("image/png"),
        image::ImageFormat::Jpeg => Some("image/jpeg"),
        image::ImageFormat::Gif => Some("image/gif"),
        image::ImageFormat::WebP => Some("image/webp"),
        _ => None,
    }
}

fn human(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

// ── Attached pictures ───────────────────────────────────────────────────

/// At most this many pictures ride on one turn; the rest are named by path
/// only. Each costs context for the rest of the conversation (~1000–1500
/// tokens on the Qwen-VL presets), and a burst of phone photos should not
/// spend a window by accident.
pub const MAX_ATTACHED_IMAGES: usize = 8;

/// The pictures among `paths`, read out of the session jail and capped at
/// the door (`image::block_from_bytes`) — the Slack door's rule: the path is
/// named in the text *and* the pixels ride on the turn, so the model has
/// both something to look at and something to pass to a tool. Read through
/// `WorkspaceFiles::read`, the download route's containment walk, because
/// the paths come from the caller (a page, a case file). What cannot be read or decoded is left to
/// its path and logged, never a failed turn.
pub fn attached_images(workspace: &std::path::Path, paths: &[String]) -> Vec<Block> {
    let files = match crate::workspace_files::WorkspaceFiles::open(workspace) {
        Ok(files) => files,
        Err(e) => {
            tracing::warn!("attachments not read: {e}");
            return Vec::new();
        }
    };
    let mut blocks = Vec::new();
    for path in paths {
        // The name decides whether to try; the bytes decide what it is
        // (`image::block_from_bytes`).
        if image_media_type(std::path::Path::new(path)).is_none() {
            continue;
        }
        if blocks.len() == MAX_ATTACHED_IMAGES {
            tracing::info!(
                "more than {MAX_ATTACHED_IMAGES} pictures on one turn; the rest by path"
            );
            break;
        }
        let read = files.read(path).and_then(|(file, _)| {
            use std::io::Read;
            let mut bytes = Vec::new();
            // One byte past the cap tells "exactly at it" from "cut short": a
            // truncated JPEG can still decode, as half a picture.
            file.take(MAX_ATTACHMENT_BYTES + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
                return Err(std::io::Error::other(format!(
                    "larger than {} MB",
                    MAX_ATTACHMENT_BYTES / (1024 * 1024)
                )));
            }
            Ok(bytes)
        });
        let bytes = match read {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!("attachment {path} not read: {e}");
                continue;
            }
        };
        match block_from_bytes(bytes, Some(path.clone()), path) {
            Ok(block) => blocks.push(block),
            Err(e) => tracing::warn!("attachment {path} not attached: {e:#}"),
        }
    }
    blocks
}

/// A file larger than this is named by path only: the door caps what rides
/// on the turn either way, and a phone photo is a few megabytes.
pub const MAX_ATTACHMENT_BYTES: u64 = 50 * 1024 * 1024;

/// A JPEG of `w`×`h`, red on its left half and blue on its right, with an
/// EXIF orientation tag spliced in ahead of the encoder's own segments — the
/// way a phone stores a photo it took sideways.
#[cfg(test)]
pub(crate) fn jpeg_with_orientation(w: u32, h: u32, orientation: u16) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, _| {
        image::Rgb(if x < w / 2 { [255, 0, 0] } else { [0, 0, 255] })
    });
    let mut jpg = Vec::new();
    img.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
        &mut jpg, 90,
    ))
    .unwrap();
    let [lo, hi] = orientation.to_le_bytes();
    // TIFF, little-endian: IFD0 at 8 holding one entry, 0x0112
    // (Orientation), SHORT, count 1, the value; no next IFD.
    let tiff = [
        b'I', b'I', 0x2A, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, lo, hi, 0, 0, 0, 0, 0,
        0,
    ];
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(&tiff);
    let len = u16::try_from(app1.len() + 2).unwrap().to_be_bytes();
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1, len[0], len[1]];
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpg[2..]);
    out
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

    /// A PNG that is only a header — signature, an IHDR claiming `w`×`h`, an
    /// empty IDAT and IEND — so a test can ask about enormous dimensions without allocating
    /// them.
    fn png_claiming(w: u32, h: u32) -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut c = !0u32;
            for &b in bytes {
                c ^= u32::from(b);
                for _ in 0..8 {
                    c = if c & 1 != 0 {
                        (c >> 1) ^ 0xEDB8_8320
                    } else {
                        c >> 1
                    };
                }
            }
            !c
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let start = out.len();
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            let crc = crc32(&out[start..]);
            out.extend_from_slice(&crc.to_be_bytes());
        }
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        // The decoder reports dimensions only once it reaches image data; an
        // empty stream is enough, since nothing here is ever decoded.
        chunk(&mut out, b"IDAT", &[]);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// A picture whose header claims more than the decode bound is refused
    /// at the header, and said to be too large rather than broken. Fails with
    /// `decode` back on a bare `load_from_memory`, which reports a failed
    /// decode (or, on a real file of that size, allocates it).
    #[test]
    fn a_picture_claiming_enormous_dimensions_is_refused_as_too_large() {
        let huge = png_claiming(40_000, 40_000);
        let err = format!("{:#}", rendered_block(&huge, None).unwrap_err());
        assert!(err.contains("too large"), "{err}");
        let err = format!(
            "{:#}",
            block_from_bytes(huge, None, "huge.png").unwrap_err()
        );
        assert!(
            err.contains("huge.png") && err.contains("too large"),
            "{err}"
        );
    }

    /// A tall screenshot — past any per-side bound, far inside the area one —
    /// is shrunk and shown, as it was before the decode bound existed. Fails
    /// on a per-side limit (found on review of #368).
    #[test]
    fn a_tall_screenshot_is_shrunk_rather_than_refused() {
        let tall = png(200, 17_000);
        let Block::Image { data, .. } = rendered_block(&tall, None).unwrap() else {
            panic!("expected an image block")
        };
        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&data)
            .unwrap();
        let i = image::load_from_memory(&decoded).unwrap();
        assert!(image::GenericImageView::height(&i) <= MAX_EDGE);
        assert!(block_from_bytes(tall, None, "tall.png").is_ok());
    }

    /// A real JPEG named `.png` goes out as `image/jpeg`: the type the
    /// provider checks is the type the bytes are. Fails on the extension's
    /// answer, which Anthropic rejects as a mismatch (found on review of
    /// #368).
    #[test]
    fn a_picture_is_typed_by_its_bytes_not_its_name() {
        let mut jpeg = Vec::new();
        image::RgbImage::from_pixel(40, 20, image::Rgb([200, 30, 30]))
            .write_with_encoder(image::codecs::jpeg::JpegEncoder::new(&mut jpeg))
            .unwrap();
        let dir = std::env::temp_dir().join(format!("mecha-img-type-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = write(&dir, "shot.png", &jpeg);
        let Block::Image { media_type, .. } = block_from_path(&p).unwrap().unwrap() else {
            panic!("expected an image block")
        };
        assert_eq!(media_type, "image/jpeg");
        std::fs::remove_dir_all(&dir).ok();
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
        // Both sides of `MAX_BYTES`: under it is the pass-through path, which
        // never decodes, so only the header check stands between a lie and
        // the transcript (found on review of #368; the 6 MB case alone
        // passed through the decode path and hid it).
        for (name, size) in [("lie.png", 6 * 1024 * 1024), ("small-lie.png", 1024 * 1024)] {
            let p = write(&dir, name, &vec![7u8; size]);
            let err = block_from_path(&p).unwrap_err().to_string();
            assert!(err.contains("did not decode"), "{name}: {err}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A photo shown to the model is shown upright: one over the edge cap is
    /// re-encoded, the tag that said which way up it was is lost, so the
    /// pixels are turned first — as the edit path turns them (review of #560).
    #[test]
    fn an_oversized_sideways_photo_is_shown_upright() {
        let photo = super::jpeg_with_orientation(MAX_EDGE + 32, 600, 6);
        for block in [
            block_from_bytes(photo.clone(), None, "the photo").unwrap(),
            rendered_block(&photo, None).unwrap(),
        ] {
            let Block::Image { data, .. } = block else {
                panic!("expected an image block")
            };
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&data)
                .unwrap();
            let img = image::load_from_memory(&bytes).unwrap();
            assert!(
                img.height() > img.width(),
                "{}×{}",
                img.width(),
                img.height()
            );
        }
    }
}
