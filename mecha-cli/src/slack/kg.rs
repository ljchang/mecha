//! The `note` and `queues` command words: capture into the knowledge graph
//! and read the review backlog, from the screen in hand.
//!
//! Both ride the `doctor` pattern, and its two boundaries hold here too:
//!
//! - **Owner-tier only.** The connector's gate runs before either word is
//!   looked at, so the matcher never sees a stranger's message. For `note`
//!   the order is what makes the capture *trustable*: the body lands in the
//!   graph as the owner's own words, because the gate proved the speaker.
//! - **Never on the ack path.** A capture starts an MCP server and the
//!   queues report reads five stores through child processes; both run in
//!   spawned work, because the three-second ack budget is Slack's.
//!
//! **A capture is deterministic, not a prompt.** `note buy milk` is matched
//! before the text can reach the model — the same precedence `doctor` and
//! `review` take, and for the same reason inverted: a note *asked* of the
//! model ("please remember this") may or may not become a `kg_upsert`, and a
//! capture that depends on a model's mood is not a capture. The cost is that
//! a message starting with the word `note` cannot be a prompt in an owner
//! channel; the word was chosen because that sentence is almost always a
//! capture anyway, and the bare word alone still falls through.

/// `note <text>` (or `note: <text>`, `notes …`), first word exactly: the
/// remainder is the note. The BARE word falls through to the model — it
/// carries nothing to capture, and "notes?" is a question, not a note.
pub fn note_command(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let (first, rest) = trimmed.split_once(char::is_whitespace)?;
    let word = first.trim_end_matches(':');
    if !(word.eq_ignore_ascii_case("note") || word.eq_ignore_ascii_case("notes")) {
        return None;
    }
    let body = rest.trim();
    (!body.is_empty()).then(|| body.to_string())
}

/// A message that is the word `queues` and nothing else, however cased or
/// padded — `is_doctor_command`'s rule: anything more is a prompt.
pub fn is_queues_command(text: &str) -> bool {
    text.trim().eq_ignore_ascii_case("queues")
}

/// Capture one note through `mecha kg note` and hand back the child's own
/// first line ("noted (episode N, M entities linked)") — the same landing
/// as the TUI's /note, one argv element, never a shell.
pub async fn capture(body: &str) -> String {
    let exe = crate::exe::self_exe();
    let out = tokio::process::Command::new(exe)
        .args(["kg", "note", body])
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    match out {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("noted")
            .to_string(),
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            format!(
                "the note did not land: {}",
                err.trim()
                    .lines()
                    .next_back()
                    .unwrap_or("mecha kg note failed")
            )
        }
        Err(e) => format!("the note could not run: {e}"),
    }
}

/// The review backlog, as `mecha review queues` prints it — the same rollup
/// `/queues` opens on, in a code block so the columns survive Slack. Read
/// only, deliberately: verdict buttons on a phone are a design pass of their
/// own (a group cascade is one tap with a two-hundred-item blast radius),
/// and a rollup that names the terminal verbs is honest about where the
/// deciding still happens.
pub async fn queues_report() -> String {
    let exe = crate::exe::self_exe();
    let out = tokio::process::Command::new(exe)
        .args(["review", "queues"])
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    match out {
        Ok(out) if out.status.success() => {
            format!("```{}```", String::from_utf8_lossy(&out.stdout).trim_end())
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            format!(
                "the queues could not be read: {}",
                err.trim()
                    .lines()
                    .next_back()
                    .unwrap_or("mecha review failed")
            )
        }
        Err(e) => format!("the queues could not be read: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The capture matcher takes exactly the sentences that are captures,
    /// and lets everything else become a prompt — a front-end that swallows
    /// prompts is worse than one with no command word at all.
    #[test]
    fn the_note_word_captures_and_everything_else_falls_through() {
        assert_eq!(
            note_command("note buy milk").as_deref(),
            Some("buy milk"),
            "the plain capture"
        );
        assert_eq!(
            note_command("  Note: met Sarah about the fMRI slot  ").as_deref(),
            Some("met Sarah about the fMRI slot"),
            "cased, colon, padded"
        );
        assert_eq!(
            note_command("notes from the lab meeting were great").as_deref(),
            Some("from the lab meeting were great"),
            "the notes spelling captures too"
        );
        assert_eq!(note_command("note"), None, "the bare word is a prompt");
        assert_eq!(note_command("notes?"), None, "a question is a prompt");
        assert_eq!(
            note_command("noted everything down"),
            None,
            "a word that merely starts with it is a prompt"
        );
        assert_eq!(
            note_command("can you note this down"),
            None,
            "the word mid-sentence is a prompt"
        );
    }

    #[test]
    fn the_queues_word_is_exact() {
        assert!(is_queues_command("queues"));
        assert!(is_queues_command("  Queues  "));
        assert!(!is_queues_command("queues?"));
        assert!(!is_queues_command("show me the queues"));
    }
}
