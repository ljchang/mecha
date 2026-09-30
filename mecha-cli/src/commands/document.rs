//! `mecha document` — the owner's door onto [`mecha_core::document`]: extract
//! a PDF from the terminal, and keep the extraction cache in check.
//!
//! Same extractor, same confinement, same cache as the `document_read` tool,
//! so a page read here is not OCR'd again when a run asks for it. The path is
//! the owner's own argument, so there is no path jail — this is `cat`, not a
//! tool call.

use anyhow::{bail, Context, Result};
use mecha_core::config::Config;
use mecha_core::document::{Cache, Extractor, Mode};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Extract pages of a PDF: its text layer, and an OCR transcript where a
    /// page has none (or with --mode ocr|both). A picture of text (PNG, JPEG,
    /// WebP, GIF) is one page, read by OCR only.
    Extract {
        /// The PDF or image.
        file: std::path::PathBuf,
        /// Which pages: all, 3, 2-5, 1,4,7-9.
        #[arg(long, default_value = "all")]
        pages: String,
        /// auto | text | ocr | both.
        #[arg(long, default_value = "auto")]
        mode: String,
        /// The whole extraction as JSON, including each page's text-layer
        /// regions and, for a transcript read through the layout stage, its
        /// layout regions (boxes in PDF points; in pixels for an image).
        #[arg(long)]
        json: bool,
        /// Neither read nor write the cache.
        #[arg(long)]
        no_cache: bool,
    },
    /// Remove cached extractions not read for N days (default: [documents]
    /// cache_days).
    Prune {
        #[arg(long)]
        days: Option<u32>,
    },
    /// Remove one file's cached extraction, by the file or by its sha256.
    Forget { target: String },
}

pub async fn execute(args: Args) -> Result<()> {
    // Extraction is the feature, and its switch answers first — before the
    // settings check, so an install that is both unconfigured and off is
    // told the switch, not `mecha config init` (review of #451). `prune`
    // and `forget` only delete what the cache already holds, and deleting
    // data you have is never refused.
    if matches!(args.cmd, Cmd::Extract { .. }) {
        super::features::require(mecha_core::feature::Feature::Documents)?;
    }
    let cwd = std::env::current_dir()?;
    let cfg = Config::load(&cwd)?;
    let Some(docs) = cfg.documents.clone() else {
        bail!(
            "no [documents] table in ~/.mecha/config.toml — document extraction is off \
             (see `mecha config init` for the commented block)"
        );
    };
    let cache_dir = Cache::default_dir()?;
    match args.cmd {
        Cmd::Extract {
            file,
            pages,
            mode,
            json,
            no_cache,
        } => {
            let Some(mode) = Mode::parse(&mode) else {
                bail!("--mode must be auto, text, ocr or both");
            };
            let meta = std::fs::metadata(&file).with_context(|| format!("{}", file.display()))?;
            if !meta.is_file() {
                bail!("{} is not a regular file", file.display());
            }
            if meta.len() > docs.max_file_bytes() {
                bail!(
                    "{} is {} MB; [documents] max_file_mb is {}",
                    file.display(),
                    meta.len() / (1024 * 1024),
                    docs.max_file_mb
                );
            }
            let bytes = std::fs::read(&file).with_context(|| format!("{}", file.display()))?;
            let cache = (docs.cache && !no_cache).then(|| Cache::new(cache_dir));
            let confine = docs.confine;
            let ex = Extractor::new(docs, cache)?;
            if confine == mecha_core::sandbox::Backend::None {
                eprintln!(
                    "mecha: the PDF renderer is NOT confined ([documents] confine = \"none\")"
                );
            }
            let out = ex.extract(&bytes, &pages, mode, json, None).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                print!("{}", out.render(&file.display().to_string()));
            }
            if let Some(why) = &out.layout_unavailable {
                eprintln!(
                    "mecha: the layout stage is unavailable, OCR pages were read whole — {why}"
                );
            }
            // A page that failed, or a region of one, is printed in place; the
            // exit status says so too, so a script cannot mistake a partial
            // extraction for a whole.
            let failed = out.incomplete_pages();
            if failed > 0 {
                bail!("{failed} page(s) could not be transcribed whole — see above");
            }
            Ok(())
        }
        Cmd::Prune { days } => {
            let days = days.unwrap_or(docs.cache_days);
            let removed = Cache::new(cache_dir.clone())
                .prune(std::time::Duration::from_secs(u64::from(days) * 86_400))?;
            println!(
                "removed {} cached extraction(s) older than {days} day(s) from {}",
                removed.len(),
                cache_dir.display()
            );
            Ok(())
        }
        Cmd::Forget { target } => {
            let sha = if std::path::Path::new(&target).is_file() {
                mecha_core::document::sha256_hex(&std::fs::read(&target)?)
            } else {
                target
            };
            let gone = Cache::new(cache_dir).forget(&sha)?;
            println!(
                "{} {sha}",
                if gone {
                    "removed"
                } else {
                    "nothing cached for"
                }
            );
            Ok(())
        }
    }
}
