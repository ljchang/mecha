//! A form over a Markdown file the owner writes: a title, a note, text, and
//! `## ` sections, each with its own note and text.
//!
//! **Canonical on the way out, lossless in meaning.** [`split`] reads any
//! file into a [`Doc`]; [`join`] writes a `Doc` back in one layout — a blank
//! line between parts, each note a comment right under its heading — so
//! every file saved from the form looks the same. What it may not change is
//! what the file *means*: the text a prompt receives (comments stripped) has
//! the same sections with the same words, and every comment's text is still
//! in the file. The tests hold both.
//!
//! **A note is a comment, and stays one.** A note holding `-->` would close
//! its comment early and send the rest to every chat, so [`join`] refuses it
//! rather than escaping it — the owner sees why, and nothing is guessed.
//!
//! Headings are found the way `persona::sections` finds them: a `## ` line
//! outside a comment and outside fenced code.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// The longest heading or title the form writes.
pub const MAX_HEADING: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Doc {
    /// The `# ` line, without its marker.
    #[serde(default)]
    pub title: String,
    /// The comment under the title, without its markers.
    #[serde(default)]
    pub note: String,
    /// Text before the first section.
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub sections: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub heading: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub body: String,
}

/// Where each line starts a heading: a `## ` line outside a comment and
/// outside fenced code. Returns the lines with that mark.
fn lines(text: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let mut comment = false;
    let mut fenced = false;
    for line in text.lines() {
        // Plain: a line that starts outside a comment and outside a fence —
        // the only place a heading or the title can be.
        let plain = !comment && !fenced;
        if !comment && line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        // Comment state at the end of this line: the last opener or closer
        // on it wins.
        let mut rest = line;
        loop {
            let found = if comment {
                rest.find("-->")
            } else {
                rest.find("<!--")
            };
            match found {
                Some(at) if !fenced => {
                    rest = &rest[at + if comment { 3 } else { 4 }..];
                    comment = !comment;
                }
                _ => break,
            }
        }
        out.push((line, plain));
    }
    out
}

/// A leading comment taken off `text`: its inner text, and what follows. Only
/// a comment that opens the text and whose close ends its line.
fn leading_note(text: &str) -> (String, String) {
    let t = text.trim_start();
    if let Some(inner) = t.strip_prefix("<!--") {
        if let Some(end) = inner.find("-->") {
            let after = &inner[end + 3..];
            let line_rest = after.split('\n').next().unwrap_or("");
            if line_rest.trim().is_empty() {
                let note = dedent(&inner[..end]);
                return (note, after.trim().to_string());
            }
        }
    }
    (String::new(), text.trim().to_string())
}

/// A note's lines with the continuation indent a comment is written with
/// taken off, and blank edges trimmed.
fn dedent(s: &str) -> String {
    s.trim()
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Read a Markdown file into a [`Doc`].
pub fn split(text: &str) -> Doc {
    let mut pre = String::new();
    let mut sections: Vec<(String, String)> = Vec::new();
    for (line, plain) in lines(text) {
        if plain && line.starts_with("## ") {
            sections.push((line[3..].trim().to_string(), String::new()));
            continue;
        }
        let into = match sections.last_mut() {
            Some((_, body)) => body,
            None => &mut pre,
        };
        into.push_str(line);
        into.push('\n');
    }
    // The title: a `# ` line outside a comment and outside fenced code, with
    // nothing but comments and blank lines before it. Taken out and written
    // first; a `# ` line after other text stays where it is, or the join
    // would move prompt text ahead of what preceded it.
    let mut title = String::new();
    let mut rest = String::new();
    for (line, plain) in lines(&pre) {
        let first = crate::persona::strip_comments(&rest).0.trim().is_empty();
        if title.is_empty() && first && plain && line.starts_with("# ") {
            title = line[2..].trim().to_string();
            continue;
        }
        rest.push_str(line);
        rest.push('\n');
    }
    let (note, body) = leading_note(&rest);
    Doc {
        title,
        note,
        body,
        sections: sections
            .into_iter()
            .map(|(heading, body)| {
                let (note, body) = leading_note(&body);
                Part {
                    heading,
                    note,
                    body,
                }
            })
            .collect(),
    }
}

fn one_line(what: &str, s: &str) -> Result<()> {
    if s.contains(['\n', '\r']) {
        bail!("{what} is one line");
    }
    // A heading is prompt text: `<!--` in one would open a comment that
    // swallows the rest of the file — `## Core` included — and `-->` would
    // close one it never opened (review of #430).
    if s.contains("<!--") || s.contains("-->") {
        bail!("{what} cannot hold `<!--` or `-->`");
    }
    if s.chars().count() > MAX_HEADING {
        bail!("{what} is at most {MAX_HEADING} characters");
    }
    Ok(())
}

fn note_ok(where_: &str, note: &str) -> Result<()> {
    if note.contains("-->") || note.contains("<!--") {
        bail!(
            "the note on {where_} holds `-->` or `<!--`, which would end or nest its comment \
             — reword it, or edit the file as text"
        );
    }
    Ok(())
}

/// Write a [`Doc`] in the canonical layout. `fixed` names sections that must
/// be present (`Core`): a doc without one is refused, never saved.
pub fn join(doc: &Doc, fixed: &[&str]) -> Result<String> {
    let title = doc.title.trim();
    one_line("the title", title)?;
    note_ok("the title", &doc.note)?;
    for want in fixed {
        if !doc.sections.iter().any(|s| s.heading.trim() == *want) {
            bail!("`## {want}` cannot be removed or renamed");
        }
    }
    let mut parts: Vec<String> = Vec::new();
    if !title.is_empty() {
        parts.push(format!("# {title}"));
    }
    if let Some(n) = comment(&doc.note) {
        parts.push(n);
    }
    if !doc.body.trim().is_empty() {
        parts.push(doc.body.trim().to_string());
    }
    let mut seen = std::collections::BTreeSet::new();
    for s in &doc.sections {
        let heading = s.heading.trim();
        if heading.is_empty() {
            bail!("a section needs a heading");
        }
        // The page says so first; the server is the fence for any client.
        if !seen.insert(heading) {
            bail!("two sections are called `{heading}`");
        }
        one_line(&format!("the heading `{heading}`"), heading)?;
        note_ok(&format!("`## {heading}`"), &s.note)?;
        parts.push(format!("## {heading}"));
        if let Some(n) = comment(&s.note) {
            parts.push(n);
        }
        if !s.body.trim().is_empty() {
            parts.push(s.body.trim().to_string());
        }
    }
    Ok(parts.join("\n\n") + "\n")
}

/// A note as a comment: one line, or continuation lines indented under the
/// opener, as the templates write them.
fn comment(note: &str) -> Option<String> {
    let note = dedent(note);
    if note.is_empty() {
        return None;
    }
    Some(format!("<!-- {} -->", note.replace('\n', "\n     ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::{sections, strip_comments};

    const IDENTITY: &str = "# Mara\n\
        \n\
        <!-- Who Mara is, in your words. Comments\n     \
        never reach a chat. -->\n\
        \n\
        ## Core\n\
        Dry and precise.\n\
        <!-- was: warm -->\n\
        Distrusts easy answers.\n\
        ## How they talk\n\
        <!-- short sentences -->\n\
        Short.\n\
        \n\
        ```\n\
        ## not a heading\n\
        ```\n\
        <!-- a comment\n\
        ## not a heading either\n\
        -->\n\
        ## Background\n";

    /// The words a prompt receives, whitespace aside.
    fn meaning(text: &str) -> Vec<(String, String)> {
        let squash = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let (stripped, _) = strip_comments(text);
        let pre = stripped.split("\n## ").next().unwrap_or("").to_string();
        let mut out = vec![(String::new(), squash(&pre))];
        out.extend(
            sections(text)
                .into_iter()
                .map(|s| (s.heading, squash(&s.body))),
        );
        out
    }

    fn comments(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(s) = rest.find("<!--") {
            let e = rest[s..].find("-->").unwrap() + s;
            out.push(
                rest[s + 4..e]
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            rest = &rest[e + 3..];
        }
        out
    }

    #[test]
    fn a_file_splits_into_title_note_and_sections() {
        let d = split(IDENTITY);
        assert_eq!(d.title, "Mara");
        assert_eq!(
            d.note,
            "Who Mara is, in your words. Comments\nnever reach a chat."
        );
        let heads: Vec<&str> = d.sections.iter().map(|s| s.heading.as_str()).collect();
        assert_eq!(heads, ["Core", "How they talk", "Background"]);
        assert_eq!(d.sections[1].note, "short sentences");
        // A comment inside the text stays in the text, where it was.
        assert!(d.sections[0].body.contains("<!-- was: warm -->"));
        assert!(d.sections[1].body.contains("## not a heading"));
    }

    /// The property the module promises: same meaning, same comments, and a
    /// second pass changes nothing.
    #[test]
    fn a_join_keeps_the_meaning_and_every_comment_and_is_stable() {
        for text in [
            IDENTITY,
            "# What Mara wants\n\n<!-- Their wants. -->\n",
            "Just text, no title.\n",
            "<!-- a note first -->\n# Title\nIntro.\n## A\nx\n",
            // A `# ` line in a fence before the title stays in its fence.
            "```\n# not the title\n```\n# Title\n## A\nx\n",
            // Prose before a `# ` line: it is not hoisted over the prose.
            "Intro first.\n# Later heading\n## A\nx\n",
            "",
        ] {
            let once = join(&split(text), &[]).unwrap();
            assert_eq!(meaning(&once), meaning(text), "{text:?} → {once:?}");
            let mut before = comments(text);
            let mut after = comments(&once);
            before.sort();
            after.sort();
            assert_eq!(after, before, "{once}");
            assert_eq!(
                join(&split(&once), &[]).unwrap(),
                once,
                "not stable: {once}"
            );
        }
        let canonical = join(&split(IDENTITY), &["Core"]).unwrap();
        assert!(
            canonical.starts_with("# Mara\n\n<!-- Who Mara is, in your words. Comments\n     never reach a chat. -->\n\n## Core\n\nDry and precise."),
            "{canonical}"
        );
    }

    #[test]
    fn a_note_cannot_close_its_comment_and_core_cannot_go() {
        let mut d = split(IDENTITY);
        d.sections[1].note = "end --> and now this reaches the chat".into();
        assert!(join(&d, &[]).is_err());
        let mut d = split(IDENTITY);
        d.sections[0].heading = "Centre".into();
        let e = join(&d, &["Core"]).unwrap_err().to_string();
        assert!(e.contains("## Core"), "{e}");
        let mut d = split(IDENTITY);
        d.sections[2].heading = "two\nlines".into();
        assert!(join(&d, &[]).is_err());
        d.sections[2].heading = "  ".into();
        assert!(join(&d, &[]).is_err());
        // A heading or title that would open or close a comment.
        let mut d = split(IDENTITY);
        d.sections[1].heading = "Voice <!--".into();
        assert!(join(&d, &["Core"]).is_err());
        let mut d = split(IDENTITY);
        d.title = "Mara -->".into();
        assert!(join(&d, &[]).is_err());
        // Two sections with one heading: a second `## Core` included.
        let mut d = split(IDENTITY);
        d.sections[2].heading = "Core".into();
        assert!(join(&d, &["Core"]).is_err());
    }
}
