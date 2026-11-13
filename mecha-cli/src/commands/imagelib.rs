//! `mecha imagelib` — the owner's door into the image library
//! (`docs/IMAGE-COMPILER-DESIGN.md` §6).
//!
//! Everything made here is the owner's and approved on creation; a model can
//! only stage a candidate (`image_library_propose`), and this is where a
//! candidate becomes usable. An **untrusted** candidate — proposed in a
//! conversation holding third-party content — is approved only after its text
//! has been shown and the question answered: `--yes` does not skip it, because
//! approved text rides into every future prompt that names the character.
//!
//! It builds no provider and connects to nothing, so the library can be
//! inspected and edited with the model and the image server both down. The
//! exceptions fetch pinned weights once: `install-face-detector`, and
//! `face compare`'s first run, which fetches ArcFace as a judge does. And
//! `upscale` asks the image server for its redraw.

use anyhow::{bail, Context, Result};
use mecha_core::imagelib::{self, Entry, Kind, Library, NewEntry, Origin, RefKind, Status};
use std::path::PathBuf;

use crate::GlobalOpts;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum KindArg {
    Character,
    Style,
    Location,
    Outfit,
}

/// A reference's kind, as `--as` spells it (`--kind` is the entry's kind).
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum RefArg {
    Face,
    #[value(name = "full_body")]
    FullBody,
    Detail,
    /// A location's or an outfit's picture.
    View,
}

impl From<RefArg> for RefKind {
    fn from(r: RefArg) -> RefKind {
        match r {
            RefArg::Face => RefKind::Face,
            RefArg::FullBody => RefKind::FullBody,
            RefArg::Detail => RefKind::Detail,
            RefArg::View => RefKind::View,
        }
    }
}

impl RefArg {
    /// The entry kind this reference goes on: the rest on a character, a
    /// view on a location or an outfit — `--kind` when given, else whichever
    /// holds the name (`resolve` asks for `--kind` when both do).
    fn entry_kind(self, kind: Option<KindArg>) -> Option<KindArg> {
        match self {
            RefArg::View => kind,
            RefArg::Face | RefArg::FullBody | RefArg::Detail => Some(KindArg::Character),
        }
    }
}

impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Kind {
        match k {
            KindArg::Character => Kind::Character,
            KindArg::Style => Kind::Style,
            KindArg::Location => Kind::Location,
            KindArg::Outfit => Kind::Outfit,
        }
    }
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Approved characters, styles and locations; `--all` adds candidates.
    List {
        #[arg(long)]
        all: bool,
        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// One entry: its text, where it came from, and its portrait's path.
    Show {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// Add a character — yours, so approved at once.
    AddCharacter {
        name: String,
        /// A front-facing portrait (PNG, JPEG or WebP).
        #[arg(long)]
        portrait: PathBuf,
        /// A short description, including build and height.
        #[arg(long)]
        description: String,
        /// The seed that drew the portrait, if it was generated.
        #[arg(long)]
        seed: Option<u64>,
        /// Hide it while browsing unless locked content is shown.
        #[arg(long)]
        locked: bool,
    },
    /// Add a style — yours, so approved at once.
    AddStyle {
        name: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        locked: bool,
    },
    /// Add an outfit — yours, so approved at once. Its picture, the garment
    /// alone with no one wearing it, goes on with `add-reference <name> --as
    /// view`; video sends it as what its wearer has on.
    AddOutfit {
        name: String,
        /// What it is: garment, colour, cut, neckline.
        #[arg(long)]
        text: String,
        #[arg(long)]
        locked: bool,
    },
    /// Add a location — yours, so approved at once. Its pictures go on
    /// with `add-reference <name> --as view`.
    AddLocation {
        name: String,
        /// What the place looks like: its fixtures, layout and light.
        #[arg(long)]
        text: String,
        #[arg(long)]
        locked: bool,
    },
    /// Make a candidate usable, after reading what it says.
    Approve {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        /// Skip the question — refused for a candidate proposed in a
        /// conversation that held third-party content.
        #[arg(long)]
        yes: bool,
    },
    /// Set the password that shows locked entries while browsing the web
    /// library. Read from the terminal without echo, or from stdin.
    SetLockPassword,
    /// Lock the web library and personas again after this many minutes
    /// with no one using them (1 to 240; 15 when never set). Takes effect
    /// at the next unlock.
    SetAutolock { minutes: u32 },
    /// Say which character is you: whom "me", "my" and "we" name when you
    /// ask for a picture. With none set you are the viewer, never drawn.
    SetOwner {
        /// An approved character.
        #[arg(required_unless_present = "clear")]
        name: Option<String>,
        /// Clear the setting: you are the viewer again.
        #[arg(long, conflicts_with = "name")]
        clear: bool,
    },
    /// Remove a candidate.
    Reject {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// Remove any entry (moved aside under `removed/`, not deleted).
    Remove {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        #[arg(long)]
        yes: bool,
    },
    /// Hide an entry while browsing. Generation is unaffected.
    Lock {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// Show an entry while browsing.
    Unlock {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// A new version of an entry; the old one is kept in its history.
    Update {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        /// A character's new description, or a style's new text.
        #[arg(long, alias = "description")]
        text: Option<String>,
        /// A character's new portrait.
        #[arg(long)]
        portrait: Option<PathBuf>,
        #[arg(long)]
        seed: Option<u64>,
    },
    /// Redraw a character's small portrait at about a megapixel with the
    /// image model, as a new version; the old one is kept in its history. A
    /// small portrait reaches a video clip as a small face. When ArcFace is
    /// installed the redraw is compared with the portrait and kept only if
    /// it is the same face.
    Upscale {
        name: String,
        /// The image model's seed; a fresh one when not given.
        #[arg(long)]
        seed: Option<u64>,
        /// Write the redraw here instead of saving it, to look first.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Fetch the face detector a persona's edits are anchored with
    /// (py-feat's RetinaFace-R34, 89 MB, checked against its pin). Without
    /// it those edits draw as before and their manifests say why.
    InstallFaceDetector,
    /// Faces in pictures, read with mecha's own face detector and ArcFace:
    /// find them, cut the crop an edit is anchored to, or compare who they
    /// are.
    Face {
        #[command(subcommand)]
        cmd: FaceCmd,
    },
    /// Attach a typed picture to an approved character: a `face`, a
    /// `full_body` in a neutral, square-on stance (its pose carries into
    /// pictures), or a labelled `detail` such as a tattoo; or a `view` to an
    /// approved location. A new version.
    AddReference {
        name: String,
        /// What the picture is of.
        #[arg(long = "as", value_enum)]
        as_kind: RefArg,
        /// The picture.
        file: PathBuf,
        /// What it shows; required for a detail ("left-forearm tattoo").
        #[arg(long)]
        label: Option<String>,
        /// For a view: the location or outfit it goes on, when both have
        /// the name.
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// Take a typed picture off a character; `--label` says which detail.
    /// A new version; the picture stays in the store for the history.
    RemoveReference {
        name: String,
        #[arg(long = "as", value_enum)]
        as_kind: RefArg,
        #[arg(long)]
        label: Option<String>,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// Reference changes a model staged for you: from a conversation that
    /// had read outside content, or an incognito chat.
    Pending {
        #[arg(long)]
        json: bool,
    },
    /// Apply a staged reference change, by the id `pending` shows.
    ApproveReference { id: String },
    /// Drop a staged reference change; its picture goes once nothing names it.
    RejectReference { id: String },
}

#[derive(clap::Subcommand, Debug)]
pub enum FaceCmd {
    /// Every face in each picture: its box, in the picture's own pixels,
    /// and the detector's score, best first.
    Detect {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// The tight crop of a picture's largest face that an edit is anchored
    /// to, as a PNG. Refuses to write over a file.
    Crop {
        file: PathBuf,
        /// Where the PNG goes.
        #[arg(long, short)]
        out: PathBuf,
    },
    /// How alike the faces in each picture are to one reference face:
    /// ArcFace's cosine similarity, from -1 to 1, largest face first. The
    /// reference is a picture's largest face, or a character's face
    /// reference (its portrait when it has none). ArcFace's weights are
    /// fetched the first time, and their licence said.
    Compare {
        /// The picture whose largest face is the reference.
        #[arg(
            long,
            conflicts_with = "character",
            required_unless_present = "character"
        )]
        reference: Option<PathBuf>,
        /// A library character whose face is the reference.
        #[arg(long)]
        character: Option<String>,
        #[arg(required = true)]
        files: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

/// One y/N question, where EOF is "no" — the outbox's rule: a review surface
/// reached by a script with no stdin must not release anything.
pub(crate) fn confirm(question: &str) -> Result<bool> {
    use std::io::Write;
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
        println!();
        return Ok(false);
    }
    Ok(line.trim().eq_ignore_ascii_case("y"))
}

fn atty_stdin() -> bool {
    // SAFETY: isatty reads no memory of ours.
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

/// One line from stdin, without echo when stdin is a terminal. The password
/// is never an argument: arguments land in shell history and `ps`.
fn read_secret(prompt: &str) -> Result<String> {
    use std::io::Write;
    let tty = atty_stdin();
    let mut saved: Option<libc::termios> = None;
    if tty {
        eprint!("{prompt}");
        std::io::stderr().flush()?;
        // SAFETY: tcgetattr fills a zeroed termios for a descriptor we hold.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut t) == 0 {
                let mut off = t;
                off.c_lflag &= !libc::ECHO;
                // Gated on the call that turns echo off, not the one that
                // reads it (review of #385).
                if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &off) == 0 {
                    saved = Some(t);
                }
            }
        }
        // A terminal whose echo cannot be turned off would show the password
        // as it is typed; refuse rather than do that silently.
        if saved.is_none() {
            eprintln!();
            bail!("cannot turn off echo on this terminal; pipe the password on stdin instead");
        }
    }
    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);
    if let Some(t) = saved {
        // SAFETY: restoring the settings read above.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
        }
        eprintln!();
    }
    read?;
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

/// The one entry a typed name means. Names are unique per kind, so a name
/// held by a character and a style needs `--kind`.
fn resolve<'a>(lib: &'a Library, name: &str, kind: Option<KindArg>) -> Result<&'a Entry> {
    let name = name.trim().to_lowercase();
    if let Some(kind) = kind {
        return lib
            .get(kind.into(), &name)
            .with_context(|| format!("no {} named `{name}`", Kind::from(kind).label()));
    }
    match lib.find(&name).as_slice() {
        [] => bail!("nothing in the image library is named `{name}`"),
        [one] => Ok(one),
        several => {
            let kinds: Vec<String> = several
                .iter()
                .map(|e| e.kind.with_article().to_string())
                .collect();
            bail!(
                "`{name}` names {}; pass --kind",
                mecha_core::imagelib::join_and(&kinds)
            )
        }
    }
}

fn origin_label(o: Origin) -> &'static str {
    match o {
        Origin::Owner => "yours",
        Origin::ModelClean => "proposed by the model (clean conversation)",
        Origin::ModelUntrusted => {
            "proposed by the model in a conversation holding third-party content"
        }
    }
}

fn describe(lib: &Library, e: &Entry) {
    println!("{} {} (v{})", e.kind.label(), e.name, e.version);
    println!(
        "  status:  {}{}",
        match e.status {
            Status::Approved => "approved",
            Status::Candidate => "candidate",
        },
        if e.locked { ", locked" } else { "" }
    );
    println!("  origin:  {}", origin_label(e.origin));
    println!("  text:    {}", e.text);
    if let Some(blob) = &e.portrait {
        println!("  portrait: {}", lib.blob_path(blob).display());
    }
    if let Some(seed) = e.source_seed {
        println!("  seed:    {seed}");
    }
    let sendable = e.sendable_references();
    for r in &e.references {
        // A kind this build does not know, or one this entry kind does not
        // take, is shown as such: kept, never sent (§9.3). By identity, as
        // `read_reference` asks, so a hand-edited copy past the limit reads
        // as not sent (review of #685).
        let note = if sendable.iter().any(|s| std::ptr::eq(*s, r)) {
            ""
        } else {
            " (not sent)"
        };
        let label = r
            .label
            .as_deref()
            .map(|l| format!(" — {l}"))
            .unwrap_or_default();
        // Attached by a model from a clean conversation (2026-10-14): said,
        // so the owner can tell a chat's write from their own.
        let by = if r.origin.is_some() {
            " (from a chat)"
        } else {
            ""
        };
        println!(
            "  reference: {}{label}: {}{note}{by}",
            r.kind.label(),
            lib.blob_path(&r.blob).display()
        );
    }
    let (pending, unreadable) = imagelib::pending_references(lib.dir());
    if e.kind == Kind::Character && !unreadable.is_empty() {
        println!(
            "  waiting: {} staged change{} could not be read — `mecha imagelib pending`",
            unreadable.len(),
            if unreadable.len() == 1 { "" } else { "s" }
        );
    }
    // Only a character's: a style sharing the name has no references.
    for p in pending
        .iter()
        .filter(|p| e.kind == Kind::Character && p.name == e.name)
    {
        println!(
            "  waiting: {} (`mecha imagelib approve-reference {}`)",
            pending_line(p),
            p.id
        );
    }
}

/// One staged change, as `pending` and `show` say it.
fn pending_line(p: &imagelib::PendingReference) -> String {
    let what = match p.action {
        imagelib::PendingAction::Add => "add",
        imagelib::PendingAction::Remove => "remove",
        imagelib::PendingAction::Unknown => "an unknown change to",
    };
    let label = p
        .label
        .as_deref()
        .map(|l| format!(" — {l}"))
        .unwrap_or_default();
    format!(
        "{what} {}'s {}{label}, {}{}",
        p.name,
        p.kind.label(),
        origin_label(p.origin),
        if p.locked { ", locked" } else { "" }
    )
}

fn read_picture(path: &PathBuf) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if !meta.is_file() {
        bail!("{} is not a file", path.display());
    }
    if meta.len() > imagelib::MAX_PORTRAIT_BYTES {
        bail!(
            "{} is over the {} MB portrait cap",
            path.display(),
            imagelib::MAX_PORTRAIT_BYTES / (1024 * 1024)
        );
    }
    Ok(std::fs::read(path)?)
}

pub async fn execute(_global: &GlobalOpts, args: Args) -> Result<()> {
    // The library follows `image` (ruling L1). Its reads stay open — a store
    // on disk is the owner's to look at — and so does the lock password,
    // which Personas' locks share (the web's unlock routes are core for the
    // same reason). What adds to, changes or removes an entry is refused.
    if !matches!(
        args.cmd,
        Cmd::List { .. }
            | Cmd::Show { .. }
            | Cmd::Pending { .. }
            | Cmd::SetLockPassword
            | Cmd::SetAutolock { .. }
            | Cmd::Face { .. }
    ) {
        super::features::require(mecha_core::feature::Feature::Library)?;
    }
    if let Cmd::InstallFaceDetector = args.cmd {
        super::features::require(mecha_core::feature::Feature::Image)?;
        return install_face_detector().await;
    }
    if let Cmd::Upscale { name, seed, out } = args.cmd {
        super::features::require(mecha_core::feature::Feature::Image)?;
        return upscale(&Library::default_dir()?, &name, seed, out).await;
    }
    // Reads only: pictures on disk, and a character's face for `compare`.
    // The detector is the image feature's, as its install is.
    if let Cmd::Face { cmd } = args.cmd {
        super::features::require(mecha_core::feature::Feature::Image)?;
        return face(cmd).await;
    }
    let dir = Library::default_dir()?;
    run(&dir, args.cmd)
}

/// A redraw closer to its portrait than this is the same face. Two redraws
/// of one 401 px portrait scored 0.95 and 0.947 against it, and ArcFace puts
/// two different people well under 0.5.
const SAME_FACE: f32 = 0.8;

/// `mecha imagelib upscale`: the portrait redrawn larger by the image
/// server, compared with the portrait when ArcFace is here, then saved as a
/// new version or written to `out`. ArcFace is never fetched for this: it is
/// a judge's download, said and agreed to at `face compare`.
async fn upscale(
    dir: &std::path::Path,
    name: &str,
    seed: Option<u64>,
    out: Option<PathBuf>,
) -> Result<()> {
    use mecha_core::{face, face_id};
    let (lib, _) = Library::load(dir);
    let key = name.trim().to_lowercase();
    let entry = lib
        .get(Kind::Character, &key)
        .filter(|e| e.status == Status::Approved)
        .with_context(|| format!("no approved character `{key}`"))?;
    let (portrait, _) = lib.read_portrait(entry)?;
    let cfg = mecha_core::config::Config::load_global()?;
    let image = cfg
        .image
        .clone()
        .context("no [image] server is configured: `mecha features enable image`")?;
    let mut tool = mecha_core::imagegen::ImageGenerate::new(image)?.with_floor(cfg.system.clone());
    if let Ok(home) = mecha_core::work::mecha_home() {
        // Which model the image server holds, shared with every process
        // that draws, as `setup` records it.
        tool = tool.with_model_record(home.join("image").join("held-model.json"));
    }
    let seed = seed.unwrap_or_else(|| uuid::Uuid::new_v4().as_u64_pair().0);
    eprintln!(
        "Redrawing {key}'s portrait at seed {seed}; about a minute once the model is loaded…"
    );
    let redraw = tool
        .upscale(&portrait, seed)
        .await
        .map_err(|why| anyhow::anyhow!("{key}'s portrait was not redrawn: {why}"))?;
    // ArcFace against the old portrait: `None` when it is not installed.
    let judge = || -> Result<Option<f32>> {
        let (Some(detector), Some(arcface)) = (face::weights()?, face_id::weights()?) else {
            return Ok(None);
        };
        let reader = face_id::Reader::load(&detector, &arcface)?;
        let first = |bytes: &[u8], what: &str| -> Result<Vec<f32>> {
            reader
                .read_bytes(bytes, what)?
                .into_iter()
                .next()
                .map(|(_, e)| e)
                .with_context(|| format!("no face was found in {what}"))
        };
        Ok(Some(face_id::similarity(
            &first(&portrait, "the portrait")?,
            &first(&redraw, "the redraw")?,
        )))
    };
    let said = |judged: Option<f32>| match judged {
        Some(s) => format!("ArcFace {s:.3} against the portrait"),
        None => "not compared: ArcFace is not installed (`mecha imagelib face compare` fetches it)"
            .to_string(),
    };
    if let Some(out) = out {
        // Written before the judge, whose score is only advice here: a
        // judge that cannot answer (no face found, say) costs the score,
        // never the minute of drawing (review of #735).
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&out)
            .and_then(|mut f| std::io::Write::write_all(&mut f, &redraw))
            .with_context(|| format!("writing {}", out.display()))?;
        let said = judge()
            .map(said)
            .unwrap_or_else(|e| format!("not compared: {e:#}"));
        println!(
            "{} ({said}). Nothing was saved to the library.",
            out.display()
        );
        return Ok(());
    }
    // Saving, the judge is the guard: one that cannot run stops the save.
    let judged = judge().context(
        "the redraw could not be compared with the portrait, so it was not saved; \
         --out writes it to a file to look at instead",
    )?;
    if let Some(s) = judged.filter(|s| *s < SAME_FACE) {
        bail!(
            "the redraw is not the same face (ArcFace {s:.3}, under {SAME_FACE}); nothing was \
             saved. Try another --seed, or --out to look at it."
        );
    }
    let said = said(judged);
    // A scene naming the character is never sampled at its portrait's
    // drawing seed, so one is kept (review of #735): the seed that drew the
    // original, whose face and pose the redraw keeps, else the redraw's.
    let drawn_at = entry.source_seed.unwrap_or(seed);
    let updated = imagelib::update(
        dir,
        Kind::Character,
        &key,
        None,
        Some(redraw),
        Some(drawn_at),
    )?;
    println!(
        "`{key}` is now v{}, redrawn larger ({said}).",
        updated.version
    );
    unframe(&updated.name);
    // The face crop an edit and a clip are anchored to, cut now rather than
    // at the next edit, so a clip made before then is not given the whole
    // portrait and its outfit.
    if face::weights()?.is_some() {
        use mecha_core::face::FaceAnchors;
        let (lib, _) = Library::load(dir);
        if let Some(e) = lib.get(Kind::Character, &key) {
            if let face::Anchor::Unavailable(why) = face::CachedRetinaFace.anchor(&lib, e) {
                eprintln!("mecha: the new portrait's face crop was not cut: {why}");
            }
        }
    }
    Ok(())
}

/// A new picture under the same name: a persona framed against the old one
/// starts unframed (review of #491). After the update has landed, so a
/// failure costs only the framing, and is said.
fn unframe(name: &str) {
    let cleared = mecha_core::persona::Store::default_dir()
        .and_then(|personas| mecha_core::persona::clear_frames_for(&personas, name));
    match cleared {
        Ok(names) if !names.is_empty() => {
            println!("Its new picture is unframed in {}.", names.join(", "))
        }
        Ok(_) => {}
        Err(e) => eprintln!("mecha: a persona's old framing was not cleared: {e:#}"),
    }
}

async fn install_face_detector() -> Result<()> {
    use mecha_core::face;
    if let Some(path) = face::weights()? {
        println!("The face detector is already installed: {}", path.display());
        return Ok(());
    }
    let total = face::WEIGHTS.bytes;
    let mut shown = 0u64;
    let path = face::install(&mut |got| {
        // A line per tenth, not per chunk: this is a terminal, not a bar.
        if got >= shown + total / 10 || got == total {
            shown = got;
            eprintln!("  {} of {} MB", got / 1_000_000, total / 1_000_000);
        }
    })
    .await?;
    println!(
        "Installed the face detector ({} {}): {}",
        face::REPO,
        &face::REVISION[..8],
        path.display()
    );
    Ok(())
}

/// `mecha imagelib face`. Nothing here judges: a similarity is printed as
/// the number it is, and a picture with no face says so rather than a zero.
/// A picture that cannot be read is said in its row, the rest are still
/// read, and the command then fails, so a script sees it.
async fn face(cmd: FaceCmd) -> Result<()> {
    use mecha_core::{face, face_id};
    let detector = face::weights()?.with_context(|| {
        format!(
            "the face detector is not installed: `{}`",
            face::INSTALL_HINT
        )
    })?;
    let read = |p: &PathBuf| std::fs::read(p).with_context(|| format!("reading {}", p.display()));
    let rounded = |f: &face::Face| [f.x0, f.y0, f.x1, f.y1].map(|v| v.round() as i64);
    let mut unread = 0usize;
    match cmd {
        FaceCmd::Detect { files, json } => {
            let det = face::Detector::load(&detector)?;
            let mut all = Vec::new();
            for file in &files {
                let faces = match read(file).and_then(|b| face::detect_bytes(&det, &b)) {
                    Ok(faces) => faces,
                    Err(e) => {
                        unread += 1;
                        if json {
                            all.push(serde_json::json!({"file": file, "error": format!("{e:#}")}));
                        } else {
                            println!("{}: not read: {e:#}", file.display());
                        }
                        continue;
                    }
                };
                if json {
                    all.push(serde_json::json!({
                        "file": file,
                        "faces": faces.iter().map(|f| serde_json::json!({
                            "box": rounded(f), "score": f.score,
                        })).collect::<Vec<_>>(),
                    }));
                } else if faces.is_empty() {
                    println!("{}: no face", file.display());
                } else {
                    for f in &faces {
                        let [x0, y0, x1, y1] = rounded(f);
                        println!(
                            "{}: face at {x0},{y0} to {x1},{y1}, score {:.2}",
                            file.display(),
                            f.score
                        );
                    }
                }
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&all)?);
            }
        }
        FaceCmd::Crop { file, out } => {
            let det = face::Detector::load(&detector)?;
            let Some(png) = face::anchor_crop(&det, &read(&file)?)? else {
                bail!("no face was found in {}", file.display());
            };
            // Created, never replaced: the check and the write are one call.
            let mut created = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&out)
                .with_context(|| format!("writing {}", out.display()))?;
            if let Err(e) = std::io::Write::write_all(&mut created, &png) {
                // A half-written crop would block the retry it asks for.
                drop(created);
                std::fs::remove_file(&out).ok();
                return Err(e).with_context(|| format!("writing {}", out.display()));
            }
            println!("{}", out.display());
        }
        FaceCmd::Compare {
            reference,
            character,
            files,
            json,
        } => {
            // The reference first: a mistyped path or name should not cost
            // ArcFace's download before it is said.
            let (label, bytes) = match (reference, character) {
                (Some(p), _) => (p.display().to_string(), read(&p)?),
                (None, Some(name)) => {
                    let (lib, errors) = Library::load(&Library::default_dir()?);
                    let key = name.trim().to_lowercase();
                    // An entry that did not load is said, never read as a
                    // character that does not exist.
                    for e in &errors {
                        eprintln!("mecha: {} did not load — {}", e.path.display(), e.why);
                    }
                    let entry = lib
                        .get(Kind::Character, &key)
                        .filter(|e| e.status == Status::Approved)
                        .with_context(|| format!("no approved character `{key}`"))?;
                    let face = lib.face_picture(entry)?;
                    (format!("{key}'s {}", face.which()), face.bytes)
                }
                (None, None) => bail!("give --reference or --character"),
            };
            let arcface = super::replay_judge::arcface_weights().await?;
            let reader = face_id::Reader::load(&detector, &arcface)?;
            let Some((_, want)) = reader.read_bytes(&bytes, &label)?.into_iter().next() else {
                bail!("no face was found in {label}");
            };
            let mut all = Vec::new();
            if !json {
                println!("against {label}:");
            }
            for file in &files {
                let faces = match read(file)
                    .and_then(|b| reader.read_bytes(&b, &file.display().to_string()))
                {
                    Ok(faces) => faces,
                    Err(e) => {
                        unread += 1;
                        if json {
                            all.push(serde_json::json!({"file": file, "error": format!("{e:#}")}));
                        } else {
                            println!("  {:>8}  {}: {e:#}", "not read", file.display());
                        }
                        continue;
                    }
                };
                let scored: Vec<(face::Face, f32)> = faces
                    .iter()
                    .map(|(f, e)| (*f, face_id::similarity(&want, e)))
                    .collect();
                if json {
                    all.push(serde_json::json!({
                        "file": file,
                        "faces": scored.iter().map(|(f, s)| serde_json::json!({
                            "box": rounded(f), "score": f.score, "similarity": s,
                        })).collect::<Vec<_>>(),
                    }));
                    continue;
                }
                match scored.split_first() {
                    None => println!("  {:>8}  {}", "no face", file.display()),
                    Some(((_, s), rest)) => {
                        let more = if rest.is_empty() {
                            String::new()
                        } else {
                            let others: Vec<String> =
                                rest.iter().map(|(_, s)| format!("{s:.3}")).collect();
                            format!("  (smaller faces: {})", others.join(", "))
                        };
                        println!("  {s:>8.3}  {}{more}", file.display());
                    }
                }
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "reference": label, "files": all,
                    }))?
                );
            }
        }
    }
    if unread > 0 {
        bail!(
            "{unread} picture{} could not be read",
            if unread == 1 { "" } else { "s" }
        );
    }
    Ok(())
}

fn run(dir: &std::path::Path, cmd: Cmd) -> Result<()> {
    let (lib, errors) = Library::load(dir);
    for e in &errors {
        eprintln!("mecha: {} did not load — {}", e.path.display(), e.why);
    }
    match cmd {
        // Async, so `execute` runs it before reaching here.
        Cmd::InstallFaceDetector => anyhow::bail!("install-face-detector runs from `execute`"),
        Cmd::Face { .. } => anyhow::bail!("face runs from `execute`"),
        Cmd::Upscale { .. } => anyhow::bail!("upscale runs from `execute`"),
        Cmd::List { all, json } => {
            let entries: Vec<&Entry> = lib
                .all()
                .iter()
                .filter(|e| all || e.status == Status::Approved)
                .collect();
            if json {
                let out: Vec<_> = entries
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "kind": e.kind, "name": e.name, "version": e.version,
                            "status": e.status, "origin": e.origin, "locked": e.locked,
                            "text": e.text,
                            "portrait": e.portrait.as_deref().map(|b| lib.blob_path(b)),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            if entries.is_empty() {
                println!("The image library at {} is empty.", dir.display());
                return Ok(());
            }
            for e in entries {
                let flags = [
                    (e.status == Status::Candidate).then_some("candidate"),
                    e.locked.then_some("locked"),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ");
                let flags = if flags.is_empty() {
                    String::new()
                } else {
                    format!(" [{flags}]")
                };
                println!(
                    "{:9} {} (v{}){flags}: {}",
                    e.kind.label(),
                    e.name,
                    e.version,
                    e.text
                );
            }
            let pending = lib.candidates().count();
            if !all && pending > 0 {
                println!("\n{pending} candidate(s) waiting — `mecha imagelib list --all`.");
            }
        }
        Cmd::Show { name, kind } => describe(&lib, resolve(&lib, &name, kind)?),
        Cmd::Pending { json } => {
            let (pending, errors) = imagelib::pending_references(dir);
            if json {
                println!("{}", serde_json::to_string_pretty(&pending)?);
            } else if pending.is_empty() {
                println!("No reference changes are waiting.");
            } else {
                for p in &pending {
                    println!("{}  {}", p.id, pending_line(p));
                    if let Some(blob) = &p.blob {
                        println!("    picture: {}", lib.blob_path(blob).display());
                    }
                }
            }
            for e in &errors {
                eprintln!("mecha: {} would not load: {}", e.path.display(), e.why);
            }
        }
        Cmd::ApproveReference { id } => {
            let e = imagelib::approve_reference(dir, &id)?;
            println!(
                "`{}` is now v{} with the change applied.",
                e.name, e.version
            );
        }
        Cmd::RejectReference { id } => {
            imagelib::reject_reference(dir, &id)?;
            println!("Dropped the staged change.");
        }
        Cmd::AddCharacter {
            name,
            portrait,
            description,
            seed,
            locked,
        } => {
            let e = imagelib::create(
                dir,
                NewEntry {
                    kind: Kind::Character,
                    name,
                    text: description,
                    portrait: Some(read_picture(&portrait)?),
                    source_seed: seed,
                    origin: Origin::Owner,
                    locked,
                },
            )?;
            println!("Added character `{}`.", e.name);
        }
        Cmd::AddStyle { name, text, locked } => {
            let e = imagelib::create(
                dir,
                NewEntry {
                    kind: Kind::Style,
                    name,
                    text,
                    portrait: None,
                    source_seed: None,
                    origin: Origin::Owner,
                    locked,
                },
            )?;
            println!("Added style `{}`.", e.name);
        }
        Cmd::AddOutfit { name, text, locked } => {
            let e = imagelib::create(
                dir,
                NewEntry {
                    kind: Kind::Outfit,
                    name,
                    text,
                    portrait: None,
                    source_seed: None,
                    origin: Origin::Owner,
                    locked,
                },
            )?;
            println!(
                "Added outfit `{}`. Add a picture of the garment alone with `mecha imagelib add-reference {} --as view <file>`.",
                e.name, e.name
            );
        }
        Cmd::AddLocation { name, text, locked } => {
            let e = imagelib::create(
                dir,
                NewEntry {
                    kind: Kind::Location,
                    name,
                    text,
                    portrait: None,
                    source_seed: None,
                    origin: Origin::Owner,
                    locked,
                },
            )?;
            println!(
                "Added location `{}`. Add its pictures with `mecha imagelib add-reference {} --as view <file>`.",
                e.name, e.name
            );
        }
        Cmd::Approve { name, kind, yes } => {
            let e = resolve(&lib, &name, kind)?;
            if e.status == Status::Approved {
                bail!("`{}` is already approved", e.name);
            }
            describe(&lib, e);
            if e.origin == Origin::ModelUntrusted && yes {
                bail!(
                    "`{}` was proposed in a conversation holding third-party content, and once \
                     approved its text rides into every prompt that names it. Read it above and \
                     answer the question — run again without --yes.",
                    e.name
                );
            }
            if !yes && !confirm(&format!("Approve {} `{}`?", e.kind.label(), e.name))? {
                println!("Not approved.");
                return Ok(());
            }
            imagelib::approve(dir, e.kind, &e.name)?;
            println!("Approved `{}`.", e.name);
        }
        Cmd::SetLockPassword => {
            let first = read_secret("New lock password: ")?;
            if atty_stdin() {
                let again = read_secret("Again: ")?;
                if again != first {
                    bail!("the two entries differ; nothing was changed");
                }
            }
            imagelib::set_lock_password(dir, &first)?;
            println!("Lock password set. Locked entries show in the web library once unlocked.");
        }
        Cmd::SetOwner { name, clear: _ } => {
            imagelib::set_owner(dir, name.as_deref())?;
            match name {
                Some(n) => println!(
                    "{} is you now: \"me\", \"my\" and \"we\" in what you ask of a picture name them.",
                    mecha_core::picture::shown(&mecha_core::scene::Who::Library(
                        n.trim().to_lowercase()
                    ))
                ),
                None => println!("Cleared: in what you ask of a picture, you are the viewer."),
            }
        }
        Cmd::SetAutolock { minutes } => {
            imagelib::set_autolock_minutes(dir, minutes)?;
            println!(
                "Unlocks now lapse after {minutes} minute{} with no one using them, from the next unlock.",
                if minutes == 1 { "" } else { "s" }
            );
        }
        Cmd::Reject { name, kind } => {
            let e = resolve(&lib, &name, kind)?;
            if e.status != Status::Candidate {
                bail!(
                    "`{}` is approved; `mecha imagelib remove` removes it",
                    e.name
                );
            }
            imagelib::reject(dir, e.kind, &e.name)?;
            println!("Rejected `{}`.", e.name);
        }
        Cmd::Remove { name, kind, yes } => {
            let e = resolve(&lib, &name, kind)?;
            describe(&lib, e);
            if !yes && !confirm(&format!("Remove {} `{}`?", e.kind.label(), e.name))? {
                println!("Kept.");
                return Ok(());
            }
            imagelib::remove(dir, e.kind, &e.name)?;
            println!("Removed `{}` (kept under removed/).", e.name);
        }
        Cmd::Lock { name, kind } => {
            let e = resolve(&lib, &name, kind)?;
            imagelib::set_locked(dir, e.kind, &e.name, true)?;
            println!("Locked `{}`: hidden while browsing, still usable.", e.name);
        }
        Cmd::Unlock { name, kind } => {
            let e = resolve(&lib, &name, kind)?;
            imagelib::set_locked(dir, e.kind, &e.name, false)?;
            println!("Unlocked `{}`.", e.name);
        }
        Cmd::AddReference {
            name,
            as_kind,
            file,
            label,
            kind,
        } => {
            let e = resolve(&lib, &name, as_kind.entry_kind(kind))?;
            let bytes = read_picture(&file)?;
            let kind: RefKind = as_kind.into();
            let updated =
                imagelib::add_reference_to(dir, e.kind, &e.name, kind, &bytes, label, None)?;
            println!(
                "`{}` is now v{} with a {} reference.",
                updated.name,
                updated.version,
                kind.label()
            );
            if kind == RefKind::FullBody {
                println!(
                    "A full-body picture's stance carries into pictures: neutral and square-on \
                     works best. It goes with a new picture of that person alone; pictures with \
                     others, edits and layered pictures use portraits."
                );
            }
        }
        Cmd::RemoveReference {
            name,
            as_kind,
            label,
            kind,
        } => {
            let e = resolve(&lib, &name, as_kind.entry_kind(kind))?;
            let kind: RefKind = as_kind.into();
            let updated =
                imagelib::remove_reference_from(dir, e.kind, &e.name, kind, label.as_deref())?;
            println!(
                "`{}` is now v{}, without that {} reference.",
                updated.name,
                updated.version,
                kind.label()
            );
        }
        Cmd::Update {
            name,
            kind,
            text,
            portrait,
            seed,
        } => {
            let e = resolve(&lib, &name, kind)?;
            let portrait = portrait.as_ref().map(read_picture).transpose()?;
            let updated = imagelib::update(dir, e.kind, &e.name, text, portrait, seed)?;
            println!("`{}` is now v{}.", updated.name, updated.version);
            // Compared, not "a --portrait was passed": portraits are
            // content-addressed, so the same file passed again changes
            // nothing to reframe.
            if updated.portrait != e.portrait {
                unframe(&updated.name);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mecha-imagelib-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `KindArg` converts into `Kind`, so no match can fail when `Kind`
    /// grows; this walks `Kind::ALL` instead (§9.5: a kind missing here is
    /// one the CLI cannot name).
    #[test]
    fn every_entry_kind_has_a_cli_spelling() {
        use clap::ValueEnum;
        for kind in Kind::ALL {
            let arg = KindArg::from_str(kind.label(), false)
                .unwrap_or_else(|_| panic!("no --kind {}", kind.label()));
            assert_eq!(Kind::from(arg), kind);
        }
        assert_eq!(KindArg::value_variants().len(), Kind::ALL.len());
    }

    fn propose(dir: &std::path::Path, name: &str, origin: Origin) {
        imagelib::create(
            dir,
            NewEntry {
                kind: Kind::Style,
                name: name.into(),
                text: "watercolour".into(),
                portrait: None,
                source_seed: None,
                origin,
                locked: false,
            },
        )
        .unwrap();
    }

    fn approve(dir: &std::path::Path, name: &str) -> Result<()> {
        run(
            dir,
            Cmd::Approve {
                name: name.into(),
                kind: None,
                yes: true,
            },
        )
    }

    /// `face compare` takes exactly one reference: a picture or a
    /// character, never both and never neither.
    #[test]
    fn face_compare_takes_one_reference() {
        use clap::Parser;
        let parse = |args: &[&str]| {
            crate::Cli::try_parse_from(["mecha", "imagelib", "face", "compare"].iter().chain(args))
        };
        assert!(parse(&["--character", "wren", "a.png"]).is_ok());
        assert!(parse(&["--reference", "r.png", "a.png", "b.png"]).is_ok());
        assert!(parse(&["a.png"]).is_err());
        assert!(parse(&["--reference", "r.png", "--character", "wren", "a.png"]).is_err());
        assert!(parse(&["--character", "wren"]).is_err());
    }

    #[test]
    fn yes_never_skips_reading_an_untrusted_candidate() {
        let dir = scratch();
        propose(&dir, "dirty", Origin::ModelUntrusted);
        propose(&dir, "clean", Origin::ModelClean);
        let refused = approve(&dir, "dirty").unwrap_err();
        assert!(format!("{refused}").contains("without --yes"), "{refused}");
        approve(&dir, "clean").unwrap();
        let (lib, _) = Library::load(&dir);
        assert_eq!(
            lib.get(Kind::Style, "dirty").unwrap().status,
            Status::Candidate
        );
        assert_eq!(
            lib.get(Kind::Style, "clean").unwrap().status,
            Status::Approved
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn locking_is_per_item_and_changes_nothing_else() {
        let dir = scratch();
        propose(&dir, "noir", Origin::Owner);
        run(
            &dir,
            Cmd::Lock {
                name: "noir".into(),
                kind: None,
            },
        )
        .unwrap();
        let (lib, _) = Library::load(&dir);
        let e = lib.get(Kind::Style, "noir").unwrap();
        assert!(e.locked);
        assert_eq!((e.status, e.version), (Status::Approved, 1));
        run(
            &dir,
            Cmd::Unlock {
                name: "noir".into(),
                kind: None,
            },
        )
        .unwrap();
        assert!(
            !Library::load(&dir)
                .0
                .get(Kind::Style, "noir")
                .unwrap()
                .locked
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reject_is_for_candidates_only() {
        let dir = scratch();
        propose(&dir, "mine", Origin::Owner);
        let refused = run(
            &dir,
            Cmd::Reject {
                name: "mine".into(),
                kind: None,
            },
        )
        .unwrap_err();
        assert!(format!("{refused}").contains("remove"), "{refused}");
        std::fs::remove_dir_all(dir).ok();
    }
}
