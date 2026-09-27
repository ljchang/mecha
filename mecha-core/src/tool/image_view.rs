//! `image_view`: put a workspace image in front of the model, on request.
//!
//! **On request, never by default.** A picture costs context by its pixels —
//! on the Qwen-VL presets about one token per 32×32 px, ~1000 for a square
//! generation — and the transcript resends it every turn for the rest of the
//! conversation. So `image_generate` hands back a path and a seed, and a run
//! that has to check what it made, or was asked about an image it was not
//! shown, calls this. Before it existed the model had no way to look at all:
//! `fs_read` returns a PNG as bytes that are not text.
//!
//! The pixels travel as [`ToolOutput::image`], which the loop folds into the
//! user turn beside the results (`ARCHITECTURE.md` §Images) — a
//! `role: "tool"` message cannot carry one — and only for a model that can
//! see. The file is read through the path jail, so this is `fs_read` for
//! pictures and declares what `fs_read` declares.

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

/// A file larger than this is refused rather than read: the look is bounded
/// by `image::rendered_block` either way, and decoding a hundred-megabyte
/// file to throw most of it away is a cost with nothing to show for it.
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

pub struct ImageView;

#[async_trait]
impl Tool for ImageView {
    fn name(&self) -> &str {
        "image_view"
    }

    fn description(&self) -> &str {
        "Look at an image in the workspace — a picture image_generate saved (images/...), one \
         the user attached (inbox/...), or any PNG, JPEG, GIF or WebP. Use it only when the \
         task needs you to see the image — the user asked you to check, compare or describe \
         it, or an edit depends on what is where — not to confirm that a picture was made. \
         Each look costs about a thousand tokens of context for the rest of the conversation."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path relative to the workspace root, or absolute inside it."}
            },
            "required": ["path"]
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    /// The owner's files, as `fs_read` declares them. The loop arms
    /// `private_data` again when it folds the pixels in; this is the
    /// declaration the interlock and the approval gate read beforehand.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default().private()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(raw) = input.get("path").and_then(Value::as_str) else {
            return Ok(ToolOutput::err("missing required string argument `path`"));
        };
        let path = ctx.resolve(raw)?;
        // Named by where it is, not by how the model spelled it: the result's
        // first line is what the web chat reads to draw the picture for the
        // owner, and `./images/a.png` or an absolute path would not match.
        let shown = ctx
            .workspace
            .canonicalize()
            .ok()
            .and_then(|root| path.strip_prefix(root).ok().map(|p| p.to_path_buf()))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| raw.to_string());
        if crate::message::image_media_type(&path).is_none() {
            return Ok(ToolOutput::err(format!(
                "{raw} is not an image this can show — PNG, JPEG, GIF and WebP only."
            )));
        }
        // A workspace can contain a FIFO, and its name is only a claim: open
        // without waiting for a writer and refuse anything that is not a
        // regular file before reading it — `fs_read`'s guard, for the same
        // reason (an `open` on a writer-less FIFO never returns, and a tool
        // is never interrupted mid-call).
        let mut options = tokio::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NONBLOCK);
        let file = match options.open(&path).await {
            Ok(f) => f,
            Err(e) => return Ok(ToolOutput::err(format!("cannot read {raw}: {e}"))),
        };
        let meta = match file.metadata().await {
            Ok(m) => m,
            Err(e) => return Ok(ToolOutput::err(format!("cannot read {raw}: {e}"))),
        };
        if !meta.is_file() {
            return Ok(ToolOutput::err(format!(
                "{raw} is not a regular file, so there is no picture here to show."
            )));
        }
        if meta.len() > MAX_FILE_BYTES {
            return Ok(ToolOutput::err(format!(
                "{raw} is {} MB; images are capped at {} MB.",
                meta.len() / (1024 * 1024),
                MAX_FILE_BYTES / (1024 * 1024)
            )));
        }
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        {
            use tokio::io::AsyncReadExt;
            // Bounded even if the file grows between the check and the read.
            if let Err(e) = file.take(MAX_FILE_BYTES).read_to_end(&mut bytes).await {
                return Ok(ToolOutput::err(format!("cannot read {raw}: {e}")));
            }
        }
        let name = shown.clone();
        let block =
            tokio::task::spawn_blocking(move || crate::image::rendered_block(&bytes, Some(name)))
                .await?;
        match block {
            Ok(block) => Ok(ToolOutput::ok(format!("image: {shown}")).with_image(block)),
            Err(e) => Ok(ToolOutput::err(format!("{raw} could not be shown: {e:#}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Block;

    fn ctx(dir: &std::path::Path) -> ToolCtx {
        ToolCtx {
            workspace: dir.to_path_buf(),
            ..Default::default()
        }
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mecha-view-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("images")).unwrap();
        dir
    }

    #[tokio::test]
    async fn a_workspace_picture_comes_back_as_pixels_named_by_its_path() {
        let dir = scratch("ok");
        let mut png = Vec::new();
        image::RgbImage::from_pixel(64, 32, image::Rgb([200, 30, 30]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(dir.join("images/a.png"), &png).unwrap();

        let out = ImageView
            .call(json!({"path": "./images/../images/a.png"}), &ctx(&dir))
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            out.content, "image: images/a.png",
            "named by where it is, the line the web chat reads"
        );
        let Some(Block::Image { source, .. }) = &out.image else {
            panic!("expected pixels, got {:?}", out.image)
        };
        assert_eq!(source.as_deref(), Some("images/a.png"));
        std::fs::remove_dir_all(dir).ok();
    }

    /// Recoverable failures say what is wrong and carry no pixels.
    #[tokio::test]
    async fn a_non_image_or_a_broken_one_is_an_error_without_pixels() {
        let dir = scratch("bad");
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();
        std::fs::write(dir.join("images/lie.png"), "not a png").unwrap();
        for path in ["notes.txt", "images/lie.png", "images/missing.png"] {
            let out = ImageView
                .call(json!({"path": path}), &ctx(&dir))
                .await
                .unwrap();
            assert!(
                out.is_error && out.image.is_none(),
                "{path}: {}",
                out.content
            );
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// The path jail holds: a look is a read, and a read outside the
    /// workspace is refused. The target is a real image one level up, so the
    /// only thing that can refuse it is `ToolCtx::resolve` — a nonexistent
    /// path would pass on ENOENT with the jail deleted (found on review).
    #[tokio::test]
    async fn a_path_outside_the_workspace_is_refused() {
        let dir = scratch("jail");
        let mut png = Vec::new();
        image::RgbImage::from_pixel(8, 8, image::Rgb([0, 0, 0]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let outside = dir
            .parent()
            .unwrap()
            .join(format!("mecha-view-outside-{}.png", std::process::id()));
        std::fs::write(&outside, &png).unwrap();
        let name = outside.file_name().unwrap().to_string_lossy().into_owned();
        for path in [format!("../{name}"), outside.display().to_string()] {
            let out = ImageView.call(json!({"path": path}), &ctx(&dir)).await;
            assert!(
                out.is_err() || out.as_ref().is_ok_and(|o| o.is_error && o.image.is_none()),
                "{path}: {out:?}"
            );
        }
        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(dir).ok();
    }

    /// A FIFO named like a picture is refused, not waited on: without the
    /// non-blocking open this test never finishes.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_fifo_named_like_a_picture_is_refused_rather_than_waited_on() {
        let dir = scratch("fifo");
        let fifo = dir.join("images/p.png");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("skipping: mkfifo unavailable");
            return;
        }
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            ImageView.call(json!({"path": "images/p.png"}), &ctx(&dir)),
        )
        .await
        .expect("image_view waited on a FIFO")
        .unwrap();
        assert!(out.is_error && out.image.is_none(), "{}", out.content);
        std::fs::remove_dir_all(dir).ok();
    }
}
