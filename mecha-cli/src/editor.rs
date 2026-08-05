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
