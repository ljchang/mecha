//! `mecha replay --judge` — the judges over a persona replay's renders
//! (`persona::replay_judge`): each picture's faces, people and clauses,
//! read against its scene record, and a contact sheet with one row per arm.
//!
//! It reads what `mecha replay --persona` left: the samples file and each
//! sample's scratch folder (its pictures, manifests, prompt log and staged
//! scene store). Nothing is drawn again, so a judge that changes can be run
//! over old renders.
//!
//! The questions go to the persona's judge binding (seeded, as its other
//! one-shots are), one quarantined request per question with the picture
//! beside it, under a hold per picture and only when the owner is not
//! using the model. The verdicts hold clause words from the chat's scene,
//! so they go in the private research store beside the samples (0600), and
//! standard output carries counts only.

use crate::follow::Follower;
use crate::setup::PersonaUse;
use crate::GlobalOpts;
use anyhow::{bail, Context, Result};
use mecha_core::face::FaceAnchors;
use mecha_core::persona::replay_judge::{self as rj, Judgement, Verdict};
use mecha_core::quarantine::{QuarantinedPass, Shown};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

/// One samples file: its arm, its persona, and its samples.
struct Arm {
    file: PathBuf,
    label: String,
    persona: String,
    samples: Vec<Value>,
}

fn read_arm(file: &Path) -> Result<Arm> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Value = serde_json::from_str(lines.next().unwrap_or("{}"))
        .with_context(|| format!("{}: the first line is not JSON", file.display()))?;
    let Some(header) = header.get("header") else {
        bail!(
            "{} is not a persona replay's samples (no header line)",
            file.display()
        );
    };
    if header["render"] != Value::Bool(true) {
        bail!(
            "{}: nothing was drawn in this replay (it ran with --no-render, or the persona \
             cannot draw), so there is nothing to judge",
            file.display()
        );
    }
    let persona = header["persona"]
        .as_str()
        .context("the header names no persona")?
        .to_string();
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let label = match header["arm"].as_str() {
        Some(arm) => format!("{arm} ({name})"),
        None => name,
    };
    let samples = lines
        .map(serde_json::from_str)
        .collect::<std::result::Result<Vec<Value>, _>>()
        .with_context(|| format!("{}: a sample is not JSON", file.display()))?;
    Ok(Arm {
        file: file.to_path_buf(),
        label,
        persona,
        samples,
    })
}

/// Every arm under its own label, since the summary is keyed by it: two
/// files of one name in different folders (two runs of one overlay) are
/// told apart by their paths, and one file named twice is refused (review
/// of #644).
fn distinct_labels(arms: &mut [Arm]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for a in arms.iter() {
        // Canonical, so `a/../a/c.jsonl` is `a/c.jsonl` (read_arm has
        // already read it, so it exists).
        let path = std::fs::canonicalize(&a.file)
            .with_context(|| format!("resolving {}", a.file.display()))?;
        if !seen.insert(path.clone()) {
            bail!("{} is named twice", path.display());
        }
    }
    let labels: Vec<String> = arms.iter().map(|a| a.label.clone()).collect();
    for a in arms.iter_mut() {
        if labels.iter().filter(|l| **l == a.label).count() > 1 {
            a.label = format!("{} ({})", a.label, a.file.display());
        }
    }
    Ok(())
}

/// Every output refused before any question is asked if it is there
/// already: a verdict file is one judges' reading, never two mixed.
fn refuse_existing(outputs: &[PathBuf]) -> Result<()> {
    // Two outputs at one path would write over each other (a `--sheet`
    // named as an arm's verdicts), so they are told apart first.
    let mut seen = std::collections::HashSet::new();
    for out in outputs {
        let at = std::path::absolute(out)?;
        if !seen.insert(at.clone()) {
            bail!("{} would be written twice", at.display());
        }
    }
    for out in outputs {
        if out.exists() {
            bail!(
                "{} exists: these judges already wrote it, or a run of them stopped part way \
                 (remove it to judge again)",
                out.display()
            );
        }
    }
    Ok(())
}

/// Where an arm's verdicts go: beside its samples, named by the judges, so
/// a later version never writes over an earlier one's.
fn judged_path(samples: &Path) -> PathBuf {
    samples.with_extension(format!("{}.jsonl", rj::JUDGES))
}

pub async fn execute(
    global: &GlobalOpts,
    files: &[PathBuf],
    sheet: Option<&Path>,
    json: bool,
) -> Result<()> {
    let mut arms = files
        .iter()
        .map(|f| read_arm(f))
        .collect::<Result<Vec<_>>>()?;
    distinct_labels(&mut arms)?;
    let Some(first) = arms.first() else {
        bail!("name the samples to judge: --judge <FILE>...");
    };
    let sheet = sheet.map(Path::to_path_buf).unwrap_or_else(|| {
        first
            .file
            .with_extension(format!("{}.sheet.html", rj::JUDGES))
    });
    refuse_existing(
        &arms
            .iter()
            .map(|a| judged_path(&a.file))
            .chain([sheet.clone()])
            .collect::<Vec<_>>(),
    )?;
    let follower = Follower::start(global.clone(), Box::new(|_| {})).await?;
    let router = follower.router_base();
    let model = follower.current().model.clone();
    super::replay_persona::guard_load(router.as_deref(), &model, false).await?;
    if !crate::setup::persona_provider(&follower.current(), PersonaUse::Judge)?.vision() {
        bail!("{model} cannot see pictures, so it cannot judge them; name one that can (-m)");
    }
    // Made once every refusal that needs no question has passed (the load
    // guard, the vision check), and before the first question: a folder it
    // cannot be written in stops the run then, not after every question
    // (review of #644), and a refusal leaves no empty sheet behind for the
    // next run to be refused on (review of #652).
    let mut sheet_file = super::replay_persona::private_file(&sheet)?;
    // The local allowance, not an answer's length: `no_thinking` is only a
    // request a template may ignore, and a reasoning model below
    // `--reasoning-budget` answers HTTP 200 with nothing in it (CLAUDE.md;
    // `voice_direction::pass` is the same one-shot). An empty answer is
    // then said by name (`ask`), never counted as no verdict.
    let pass = QuarantinedPass::new(model.clone(), mecha_core::provider::LOCAL_MAX_TOKENS)
        .system(rj::SYSTEM)
        .no_thinking();

    let mut rows = Vec::new();
    let mut summaries = serde_json::Map::new();
    for arm in &arms {
        let out = judged_path(&arm.file);
        let mut file = super::replay_persona::private_file(&out)?;
        writeln!(
            file,
            "{}",
            serde_json::json!({"header": {
                "judges": rj::JUDGES,
                "samples": arm.file.display().to_string(),
                "model": model,
            }})
        )?;
        let mut judged = Vec::new();
        let mut tiles = Vec::new();
        for sample in &arm.samples {
            let Some(render) = sample.get("render").filter(|r| !r.is_null()) else {
                continue;
            };
            let index = sample["sample"].as_u64().unwrap_or_default() as usize;
            // The pictures the tool delivered: each has a manifest. A layered
            // build also saves its plate and each cutout into the workspace,
            // and judging those would make a layers arm's count of pictures
            // unlike a plain arm's (review of #644, pass 2).
            let pictures: Vec<&str> = render["pictures"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|p| manifest_of(render, Path::new(p)).is_some())
                .collect();
            for picture in pictures {
                let j = judge_one(
                    &follower,
                    router.as_deref(),
                    &pass,
                    &arm.persona,
                    index,
                    render,
                    Path::new(picture),
                )
                .await?;
                writeln!(file, "{}", serde_json::to_string(&j)?)?;
                tiles.push(rj::Tile {
                    src: file_url(picture),
                    caption: rj::caption(&j),
                });
                eprintln!(
                    "{}: sample {index}, {} clause(s)",
                    arm.label,
                    j.clauses.len()
                );
                judged.push(j);
            }
        }
        let mut summary = serde_json::to_value(rj::summarise(&judged))?;
        summary["verdicts"] = Value::String(out.display().to_string());
        summaries.insert(arm.label.clone(), summary);
        rows.push((arm.label.clone(), tiles));
    }
    let title = format!("{} — {}", first.persona, rj::JUDGES);
    sheet_file.write_all(rj::sheet_html(&title, &rows).as_bytes())?;
    if json {
        println!(
            "{}",
            serde_json::json!({"sheet": sheet.display().to_string(), "arms": summaries})
        );
    } else {
        println!("contact sheet: {}", sheet.display());
        println!("{}", serde_json::to_string_pretty(&summaries)?);
    }
    Ok(())
}

/// A local picture's address for the sheet: a `#`, `?`, `%` or space in a
/// folder name would otherwise end or bend the URL (review of #644).
fn file_url(path: &str) -> String {
    let mut url = String::from("file://");
    for c in path.chars() {
        match c {
            '%' => url.push_str("%25"),
            '#' => url.push_str("%23"),
            '?' => url.push_str("%3F"),
            ' ' => url.push_str("%20"),
            c => url.push(c),
        }
    }
    url
}

/// The sample's own folder, absolute, or none: nothing is read on the
/// manifest's word from a path that would resolve against the current
/// directory (review of #644).
fn scratch_of(render: &Value) -> Option<PathBuf> {
    render["scratch"]
        .as_str()
        .filter(|s| Path::new(s).is_absolute())
        .map(PathBuf::from)
}

/// What a sample's manifests say of `picture`: the one whose `image` names
/// it.
fn manifest_of<'a>(render: &'a Value, picture: &Path) -> Option<&'a Value> {
    render["manifests"].as_array()?.iter().find(|m| {
        m["image"]
            .as_str()
            .is_some_and(|i| Path::new(i).file_name() == picture.file_name())
    })
}

/// The compiled prompt the prompt log holds for `picture`. A layered
/// picture logs one line per pass and no compiled prompt of its own.
fn prompt_of(render: &Value, picture: &Path) -> Option<String> {
    render["prompt_log"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|l| {
            l["image"]
                .as_str()
                .is_some_and(|i| Path::new(i).file_name() == picture.file_name())
        })
        .and_then(|l| l["prompt"].as_str().map(str::to_string))
}

/// One question, answered or not: the answer's text, or why there is none.
async fn ask(
    provider: &dyn mecha_core::provider::Provider,
    request: mecha_core::message::CompletionRequest,
) -> std::result::Result<String, String> {
    let response = provider
        .complete(&request, None)
        .await
        .map_err(|e| format!("the judge could not be reached ({e:#})"))?;
    if response.stop_reason == mecha_core::message::StopReason::Refusal {
        return Err("the judge refused".into());
    }
    let text = response.message.text();
    if text.trim().is_empty() {
        return Err(
            "the judge's answer was empty (its thinking may have taken the allowance)".into(),
        );
    }
    Ok(text)
}

/// One question about one picture, answered or not.
async fn ask_of(
    provider: &dyn mecha_core::provider::Provider,
    pass: &QuarantinedPass,
    picture: &Shown,
    question: &str,
) -> std::result::Result<String, String> {
    ask(provider, pass.ask_about(picture, question)).await
}

/// An answer read by `read`, or why there is no verdict: an answer it
/// could not read is said too, so "nothing answered" never reads as
/// "nothing to count".
fn verdict<T>(
    answer: std::result::Result<String, String>,
    read: fn(&str) -> Option<T>,
    j: &mut Judgement,
) -> Option<T> {
    let v = match answer {
        Ok(a) => {
            let v = read(&a);
            if v.is_none() {
                j.errors.push("an answer could not be read".into());
            }
            v
        }
        Err(e) => {
            j.errors.push(e);
            None
        }
    };
    j.unanswered += usize::from(v.is_none());
    v
}

/// The picture an edit started from, from its manifest: only a plain path
/// inside the sample's workspace, and one the workspace holds. Either
/// refusal is said, so a lost base-picture control never reads as a new
/// picture (review of #644).
fn base_of(manifest: Option<&Value>, work: &Path, errors: &mut Vec<String>) -> Option<PathBuf> {
    let named = manifest.and_then(|m| m["picture"].as_str())?;
    if !Path::new(named)
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        errors.push("the picture it started from is not a plain path in the workspace".into());
        return None;
    }
    let at = work.join(named);
    if !at.is_file() {
        errors.push("the picture it started from is not in the sample's workspace".into());
        return None;
    }
    Some(at)
}

#[allow(clippy::too_many_arguments)]
async fn judge_one(
    follower: &Follower,
    router: Option<&str>,
    pass: &QuarantinedPass,
    persona: &str,
    sample: usize,
    render: &Value,
    picture: &Path,
) -> Result<Judgement> {
    let mut j = Judgement {
        judges: rj::JUDGES.into(),
        sample,
        picture: picture.display().to_string(),
        ..Judgement::default()
    };
    let bytes = match std::fs::read(picture) {
        Ok(b) => b,
        Err(e) => {
            j.errors.push(format!("the picture did not read: {e}"));
            return Ok(j);
        }
    };
    let Some(scratch) = scratch_of(render) else {
        j.errors
            .push("the sample names no scratch folder, so nothing was judged".into());
        return Ok(j);
    };
    let manifest = manifest_of(render, picture);
    j.route = manifest.and_then(|m| m["route"].as_str().map(str::to_string));
    j.roles = manifest.and_then(|m| m.get("roles").cloned());
    j.reader = manifest.and_then(|m| m.get("reader").cloned());
    j.prompt = prompt_of(render, picture);
    // The picture an edit started from, as the sample's workspace holds it.
    // The manifest is the tool's own, but nothing is read from beyond the
    // sample's folder on its word.
    let base = base_of(manifest, &scratch.join("work"), &mut j.errors);
    let base_bytes = match base.as_ref().map(std::fs::read) {
        Some(Ok(b)) => Some(b),
        Some(Err(e)) => {
            j.errors.push(format!(
                "the picture it started from did not read, so nothing was asked of it: {e}"
            ));
            None
        }
        None => None,
    };
    j.base = base.map(|p| p.display().to_string());
    // The picture's own record, from the sample's staged store, through
    // the slot that owns its layout; a record that does not read is said
    // as that, not as none (review of #644).
    let slot = mecha_core::scene::persona_slot(&scratch.join("personas"), persona, "");
    let clauses = match slot.lookup_said(&bytes) {
        Ok(Some(s)) => {
            j.people_recorded = s.people_known.then_some(s.people.len());
            rj::clauses(&s)
        }
        Ok(None) => {
            j.errors.push("the picture has no scene record".into());
            Vec::new()
        }
        Err(e) => {
            j.errors.push(e);
            Vec::new()
        }
    };
    // The detector and the picture's rendering for the model are CPU work:
    // off the runtime, as `imagegen` keeps the detector, and each picture
    // rendered once for all its questions.
    let (faces, shown, shown_base, layout) = {
        let bytes = bytes.clone();
        let base_bytes = base_bytes.clone();
        tokio::task::spawn_blocking(move || {
            (
                mecha_core::face::CachedRetinaFace.faces_in(&bytes),
                Shown::of(&bytes),
                base_bytes.as_deref().map(Shown::of),
                base_bytes
                    .as_deref()
                    .and_then(|b| mecha_core::imagegen::layout_similarity(b, &bytes)),
            )
        })
        .await?
    };
    j.layout_similarity = layout;
    match faces {
        Ok(n) => j.faces = Some(n),
        Err(e) => j.errors.push(e),
    }
    let shown = match shown {
        Ok(s) => s,
        Err(e) => {
            j.errors
                .push(format!("the picture could not be shown: {e:#}"));
            return Ok(j);
        }
    };
    let shown_base = match shown_base {
        Some(Ok(s)) => Some(s),
        Some(Err(e)) => {
            j.errors.push(format!(
                "the picture it started from could not be shown: {e:#}"
            ));
            None
        }
        None => None,
    };

    super::replay_persona::wait_for_owner(router).await?;
    super::replay_persona::guard_load(router, pass.model(), false).await?;
    let (held, bound) = follower
        .enter("replay judges", |s| {
            eprintln!("waiting: the model is switching to {}", s.to)
        })
        .await?;
    let provider = crate::setup::persona_provider(&bound, PersonaUse::Judge)?;
    j.people_asked = verdict(
        ask_of(provider.as_ref(), pass, &shown, rj::PEOPLE_QUESTION).await,
        rj::count,
        &mut j,
    );
    for clause in clauses {
        let question = clause.question();
        let picture_says = verdict(
            ask_of(provider.as_ref(), pass, &shown, &question).await,
            rj::yes_no,
            &mut j,
        );
        let base_says = match &shown_base {
            Some(b) => verdict(
                ask_of(provider.as_ref(), pass, b, &question).await,
                rj::yes_no,
                &mut j,
            ),
            None => None,
        };
        j.clauses.push(Verdict {
            clause,
            picture: picture_says,
            base: base_says,
        });
    }
    drop(held);
    Ok(j)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture's manifest and prompt are found by its file name, which is
    /// how the tool names it relative to the workspace.
    #[test]
    fn a_pictures_manifest_and_prompt_are_found_by_its_name() {
        let render = serde_json::json!({
            "manifests": [
                {"image": "images/a.png", "roles": "applied"},
                {"image": "images/b.png", "roles": "fell back: one person", "picture": "images/a.png"},
            ],
            "prompt_log": [
                serde_json::json!({"image": "images/a.png", "prompt": "first"}).to_string(),
                serde_json::json!({"image": "images/b.png", "prompt": "second"}).to_string(),
            ],
        });
        let b = Path::new("/scratch/sample-000/work/images/b.png");
        assert_eq!(
            manifest_of(&render, b).unwrap()["roles"],
            "fell back: one person"
        );
        assert_eq!(prompt_of(&render, b).as_deref(), Some("second"));
        assert!(manifest_of(&render, Path::new("/x/c.png")).is_none());
    }

    /// A replay that drew nothing is refused by name, before any model is
    /// asked anything.
    #[test]
    fn a_replay_that_drew_nothing_is_refused() {
        let dir = std::env::temp_dir().join(format!("mecha-rj-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s.jsonl");
        std::fs::write(
            &f,
            "{\"header\":{\"persona\":\"p\",\"render\":false}}\n{\"sample\":0}\n",
        )
        .unwrap();
        let err = read_arm(&f).err().map(|e| format!("{e:#}")).unwrap();
        assert!(err.contains("nothing was drawn"), "{err}");
        std::fs::write(&f, "{\"sample\":0}\n").unwrap();
        assert!(read_arm(&f).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Outputs already there refuse the run before any question, and two
    /// arms of one name are told apart rather than merged (review of #644).
    #[test]
    fn outputs_and_labels_are_settled_before_any_question() {
        let dir = std::env::temp_dir().join(format!("mecha-rj-{}", uuid::Uuid::new_v4()));
        for d in ["a", "b"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
            std::fs::write(dir.join(d).join("c.jsonl"), "").unwrap();
        }
        let there = dir.join("a").join("v.jsonl");
        std::fs::write(&there, "").unwrap();
        let err = refuse_existing(&[dir.join("none"), there.clone()]).unwrap_err();
        assert!(format!("{err:#}").contains("stopped part way"), "{err:#}");
        assert!(refuse_existing(&[dir.join("none")]).is_ok());
        let arm = |f: PathBuf| Arm {
            file: f,
            label: "x (c.jsonl)".into(),
            persona: "p".into(),
            samples: Vec::new(),
        };
        let mut arms = vec![arm(dir.join("a/c.jsonl")), arm(dir.join("b/c.jsonl"))];
        distinct_labels(&mut arms).unwrap();
        assert_ne!(arms[0].label, arms[1].label);
        // One file by two spellings is one file.
        let mut twice = vec![arm(dir.join("a/c.jsonl")), arm(dir.join("a/../a/c.jsonl"))];
        assert!(distinct_labels(&mut twice).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Only a picture the tool delivered has a manifest: a layered build's
    /// plate and cutouts are not judged (review of #644, pass 2).
    #[test]
    fn only_a_delivered_picture_has_a_manifest() {
        let render = serde_json::json!({"manifests": [{"image": "images/1-2.png"}]});
        assert!(manifest_of(&render, Path::new("/s/work/images/1-2.png")).is_some());
        assert!(manifest_of(&render, Path::new("/s/work/images/1-2-plate.png")).is_none());
        assert!(manifest_of(&render, Path::new("/s/work/images/1-2-cutout-mara.png")).is_none());
    }

    /// A sheet named as an arm's verdicts is refused, and a tile's address
    /// survives the characters a folder may hold.
    #[test]
    fn outputs_never_collide_and_addresses_survive_their_folders() {
        let v = PathBuf::from("/r/c.replay-judges-1.jsonl");
        assert!(refuse_existing(&[v.clone(), v]).is_err());
        assert_eq!(
            file_url("/r/run #2/a?.png"),
            "file:///r/run%20%232/a%3F.png"
        );
    }

    /// An empty or unreadable answer is said, never only counted as none.
    #[test]
    fn an_answer_with_no_verdict_is_said() {
        let mut j = Judgement::default();
        assert_eq!(verdict(Ok("Yes".into()), rj::yes_no, &mut j), Some(true));
        assert!(j.errors.is_empty());
        assert_eq!(verdict(Ok("maybe".into()), rj::yes_no, &mut j), None);
        assert_eq!(verdict(Err("empty".into()), rj::count, &mut j), None);
        assert_eq!(j.errors.len(), 2);
        assert_eq!(j.unanswered, 2, "counted for the summary as well as said");
    }

    /// The base is read only from a plain path the workspace holds, and a
    /// refusal is said (review of #644).
    #[test]
    fn a_base_picture_is_read_only_from_inside_the_workspace() {
        let work = std::env::temp_dir().join(format!("mecha-rjb-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(work.join("images")).unwrap();
        std::fs::write(work.join("images/a.png"), b"x").unwrap();
        let m = |p: &str| serde_json::json!({"picture": p});
        let mut errors = Vec::new();
        assert_eq!(
            base_of(Some(&m("images/a.png")), &work, &mut errors),
            Some(work.join("images/a.png"))
        );
        assert!(errors.is_empty());
        for bad in [
            "../a.png",
            "/etc/passwd",
            "images/../../a.png",
            "images/gone.png",
        ] {
            assert_eq!(base_of(Some(&m(bad)), &work, &mut errors), None, "{bad}");
        }
        assert_eq!(errors.len(), 4, "each refusal is said");
        assert_eq!(
            base_of(Some(&serde_json::json!({})), &work, &mut errors),
            None
        );
        assert_eq!(
            errors.len(),
            4,
            "a new picture has no base and nothing to say"
        );
        let _ = std::fs::remove_dir_all(&work);
    }

    /// A sample that names no folder, or a relative one, is judged on
    /// nothing rather than on the current directory.
    #[test]
    fn a_sample_is_read_only_from_its_own_folder() {
        let at = |v: Value| scratch_of(&serde_json::json!({ "scratch": v }));
        assert_eq!(
            at("/r/c.work/sample-000".into()),
            Some(PathBuf::from("/r/c.work/sample-000"))
        );
        assert_eq!(at("".into()), None);
        assert_eq!(at("c.work/sample-000".into()), None);
        assert_eq!(scratch_of(&serde_json::json!({})), None);
    }

    /// The scene reader and role splitter are sent the sample's image seed,
    /// so two arms at one base read as well as draw at the same seeds
    /// (review of #615).
    #[test]
    fn a_replays_readers_are_seeded_by_the_sample() {
        let src = include_str!("replay_persona.rs");
        assert!(src.contains("persona_provider_seeded(&bound, PersonaUse::Judge, image_seed)"));
        assert!(!src.contains("persona_provider(&bound, PersonaUse::Judge)"));
    }

    #[test]
    fn verdicts_are_named_by_the_judges() {
        assert_eq!(
            judged_path(Path::new("/r/c-L5-x.jsonl")),
            PathBuf::from(format!("/r/c-L5-x.{}.jsonl", rj::JUDGES))
        );
    }
}
