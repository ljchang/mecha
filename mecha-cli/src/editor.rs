//! The one `$EDITOR` shell-out, shared by `outbox edit` and the TUI's ^G.
//!
//! One implementation on purpose: the scratch-file dance (write, spawn,
//! check the exit, read back, clean up) has enough edges — a failing editor
//! must leave the original untouched, the scratch must not linger — that two
//! copies would drift on exactly the edge that mattered.

use anyhow::{bail, Context, Result};

/// Open `initial` in `$VISUAL`/`$EDITOR`/`vi` and return what the user saved.
///
/// Blocks until the editor exits — the caller owns the terminal and must have
/// handed it over first. A non-zero exit is an error and the original text is
/// the caller's to keep: an editor that was quit in anger must not "save".
pub fn edit_text(initial: &str, scratch_name: &str) -> Result<String> {
    let scratch = std::env::temp_dir().join(scratch_name);
    std::fs::write(&scratch, initial).with_context(|| format!("writing {}", scratch.display()))?;

    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "{editor} {}",
            shell_quote(&scratch.to_string_lossy())
        ))
        .status()
        .with_context(|| format!("launching {editor}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&scratch);
        bail!("{editor} exited with {status}");
    }

    let text = std::fs::read_to_string(&scratch)?;
    let _ = std::fs::remove_file(&scratch);
    Ok(text)
}

/// Open `path` itself in `$VISUAL`/`$EDITOR`/`vi` — no scratch file, because
/// the file *is* the store. For the charter, whose write path is deliberately
/// "the owner with a text editor" and nothing else: mecha hands the terminal
/// over and reads the result back through the ordinary load, so a failed
/// save, a quit-in-anger, or a syntax error all land exactly where a hand-run
/// `vi ~/.mecha/charter.toml` would have put them.
///
/// Blocks until the editor exits — the caller owns the terminal and must have
/// handed it over first.
pub fn edit_file(path: &std::path::Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} {}", shell_quote(&path.to_string_lossy())))
        .status()
        .with_context(|| format!("launching {editor}"))?;
    if !status.success() {
        bail!("{editor} exited with {status}");
    }
    Ok(())
}

/// What editing the charter actually did, established by looking at the file.
///
/// Shared by `mecha charter edit` and the TUI's `/charter` `e` because the
/// subtle part is not the editing — it is deciding what happened afterwards,
/// and there are two traps in it that only bite sometimes. Two copies would
/// drift on exactly the arm that mattered, which is the argument this
/// module's own doc opens with.
#[derive(Debug, PartialEq, Eq)]
pub enum CharterEdit {
    /// The template was written and the editor closed without adding a line.
    TemplateCreated,
    /// The file came back byte-identical.
    Unchanged,
    /// It changed, and a run would load it.
    Saved,
    /// It changed and will **not** load — every run starts un-chartered
    /// until it does, which is worth saying louder than "saved".
    SavedButInvalid(String),
    /// The editor exited non-zero and the file changed anyway (`:cq` after a
    /// write, a wrapper script). Reporting "unchanged" here would be wrong
    /// about the one file that rides in every prompt.
    EditorFailedButChanged {
        error: String,
        loads: Option<String>,
    },
    /// The editor exited non-zero and nothing changed.
    EditorFailed(String),
}

/// Create the comments-only template when the file is absent, hand the file
/// to `run_editor`, and report what actually happened.
///
/// **The one write mecha ever makes here is [`mecha_core::charter::TEMPLATE`]**,
/// which is comments only and has a test that fails on any uncommented
/// `[[line]]`. Every priority in the file is the owner's own typing — see
/// `charter.rs`'s module doc for why that, rather than the absence of a verb,
/// is the invariant.
///
/// `run_editor` is a parameter because the two callers hand over the terminal
/// differently: the TUI has to suspend its alternate screen first, and the
/// CLI already owns the terminal. Everything after it is identical, which is
/// the half worth sharing.
///
/// Returns the created-flag separately from the outcome so a caller can
/// distinguish "I made you a template" from "you had one".
pub fn edit_charter_with(
    path: &std::path::Path,
    run_editor: impl FnOnce(&std::path::Path) -> Result<()>,
) -> Result<CharterEdit> {
    let mut created = false;
    if !path.is_file() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(path, mecha_core::charter::TEMPLATE)
            .with_context(|| format!("writing the charter template to {}", path.display()))?;
        created = true;
    }

    // The file *is* the store, so what the editor did is established by
    // looking rather than by its exit code: a clean exit may have saved
    // nothing, and `:cq` may exit non-zero after a save landed.
    let before = std::fs::read(path).ok();
    let result = run_editor(path);
    let changed = std::fs::read(path).ok() != before;

    // Read back through the ordinary loader, not a parse written here: the
    // question is whether a *run* would get anything, and there must be one
    // answer to that.
    let load_error = |p: &std::path::Path| {
        mecha_core::charter::Charter::load(p)
            .err()
            .map(|e| format!("{e:#}"))
    };

    Ok(match (result, changed) {
        (Ok(()), false) if created => CharterEdit::TemplateCreated,
        (Ok(()), false) => CharterEdit::Unchanged,
        (Ok(()), true) => match load_error(path) {
            None => CharterEdit::Saved,
            Some(e) => CharterEdit::SavedButInvalid(e),
        },
        (Err(e), true) => CharterEdit::EditorFailedButChanged {
            error: e.to_string(),
            loads: load_error(path),
        },
        (Err(e), false) => CharterEdit::EditorFailed(e.to_string()),
    })
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quoted_path_survives_single_quotes() {
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    /// A scratch home that cleans up after itself.
    fn scratch(tag: u32) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mecha-charter-edit-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The only bytes mecha ever writes to a charter are the template's, and
    /// the template holds no priorities. This is the invariant the whole
    /// surface exists to keep, asserted where the write actually happens.
    #[test]
    fn a_first_edit_creates_a_template_that_authors_no_priority() {
        let dir = scratch(line!());
        let path = dir.join("charter.toml");

        // An editor that changes nothing, so what lands is exactly what
        // mecha wrote.
        let out = edit_charter_with(&path, |_| Ok(())).unwrap();
        assert_eq!(out, CharterEdit::TemplateCreated);

        let body = std::fs::read_to_string(&path).unwrap();
        for l in body.lines() {
            assert!(
                l.trim().is_empty() || l.trim_start().starts_with('#'),
                "mecha must never write an uncommented line into a charter: {l:?}"
            );
        }
        // And a run would get nothing from it, which is the point of a
        // template rather than a starter charter.
        assert!(mecha_core::charter::Charter::load(&path)
            .unwrap()
            .is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An existing charter is never overwritten by the template.
    #[test]
    fn an_existing_charter_is_left_alone() {
        let dir = scratch(line!());
        let path = dir.join("charter.toml");
        let mine = "[[line]]\nid = \"a\"\ntext = \"mine\"\n";
        std::fs::write(&path, mine).unwrap();

        assert_eq!(
            edit_charter_with(&path, |_| Ok(())).unwrap(),
            CharterEdit::Unchanged
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), mine);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The first trap.** A clean editor exit does not mean anything was
    /// saved, and a save that does not load must be louder than "saved" —
    /// the file rides in every future prompt, so the cost of getting this
    /// arm wrong is paid by every run rather than by this command.
    #[test]
    fn a_saved_charter_that_will_not_load_says_so_rather_than_saying_saved() {
        let dir = scratch(line!());
        let path = dir.join("charter.toml");
        std::fs::write(&path, "[[line]]\nid = \"a\"\ntext = \"ok\"\n").unwrap();

        // `[[lines]]` — the typo `deny_unknown_fields` turns into a load
        // error rather than a silently empty charter.
        let out = edit_charter_with(&path, |p| {
            std::fs::write(p, "[[lines]]\nid = \"a\"\n").unwrap();
            Ok(())
        })
        .unwrap();
        assert!(
            matches!(out, CharterEdit::SavedButInvalid(_)),
            "got {out:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The second trap.** `:cq` exits non-zero *after* a save has landed.
    /// Reporting "unchanged" there would be a false statement about the one
    /// file that rides in every prompt, so the file is believed over the
    /// exit code.
    #[test]
    fn an_editor_that_exits_nonzero_after_saving_is_not_reported_as_unchanged() {
        let dir = scratch(line!());
        let path = dir.join("charter.toml");
        std::fs::write(&path, "[[line]]\nid = \"a\"\ntext = \"before\"\n").unwrap();

        let out = edit_charter_with(&path, |p| {
            std::fs::write(p, "[[line]]\nid = \"a\"\ntext = \"after\"\n").unwrap();
            bail!("quit with :cq")
        })
        .unwrap();
        match out {
            CharterEdit::EditorFailedButChanged { loads, .. } => {
                assert!(loads.is_none(), "what landed does load: {loads:?}")
            }
            other => panic!("a save behind a non-zero exit must not read as unchanged: {other:?}"),
        }
        assert!(std::fs::read_to_string(&path).unwrap().contains("after"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// And a genuine failure with nothing written stays a failure.
    #[test]
    fn an_editor_that_fails_without_saving_leaves_the_charter_alone() {
        let dir = scratch(line!());
        let path = dir.join("charter.toml");
        let mine = "[[line]]\nid = \"a\"\ntext = \"mine\"\n";
        std::fs::write(&path, mine).unwrap();

        let out = edit_charter_with(&path, |_| bail!("no editor here")).unwrap();
        assert!(matches!(out, CharterEdit::EditorFailed(_)), "got {out:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), mine);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_scripted_editor_round_trips_the_text() {
        // `true` as the editor: exits 0 without touching the file, so what
        // comes back is what went in.
        std::env::set_var("VISUAL", "true");
        let text = edit_text("hello\nworld", "mecha-editor-test.txt").unwrap();
        assert_eq!(text, "hello\nworld");

        // `false` exits 1: the caller keeps its original.
        std::env::set_var("VISUAL", "false");
        assert!(edit_text("x", "mecha-editor-test-2.txt").is_err());
    }
}
