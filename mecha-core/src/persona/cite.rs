//! Checked citations (PERSONA-DESIGN §10.4): a persona cites its files as
//! `[file, p. N: "an exact quote"]`, and the harness looks each quote up in
//! what the chat actually received — the files block and `file_read`'s
//! results, page by page — and says whether it is there.
//!
//! The lookup is [`crate::grounding::admit`]'s: a literal span of a named
//! referent, with its refusals. What this module adds is the walk that
//! builds the referents and a normalisation both sides go through, because a
//! PDF's text breaks lines, hyphenates across them and spells quotes and
//! dashes typographically, and a model copying a sentence does none of that.
//!
//! **A citation is not entailment.** A quote that is there can still be
//! made to support the wrong claim; the page says "quoted, not checked for
//! support", which is all this establishes. And the check is the harness's,
//! never a tool: a check the model may decline to call is not a check.

use crate::grounding::{self, Claim, Evidence, Refusal};
use crate::message::{Block, Message, Role};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The tools whose results are a file's rendered text, as `files::read` and
/// `document_read` render it: a `document: <name> · …` header, then pages.
const READERS: [&str; 2] = ["file_read", "document_read"];

/// Below this a quote matches by accident ("the", "in 2025").
const MIN_QUOTE_CHARS: usize = 12;

/// The longest citation looked at, bracket to bracket. Past it a `[` is
/// prose, not a citation.
const MAX_CITATION_CHARS: usize = 800;

/// One citation as the persona wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    /// The bracketed text exactly as it appears in the reply, so the page
    /// can find it again.
    pub raw: String,
    pub file: String,
    /// The pages cited, first and last; `None` when none was given.
    pub pages: Option<(u32, u32)>,
    pub quote: String,
}

/// What the check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Verdict {
    /// The quote is on the page cited — or in the file, where no page was
    /// given or the file has none. `found` is the page it is on.
    Quoted { found: Option<u32> },
    /// The quote is in the file, on another page than the one cited.
    OtherPage { found: u32 },
    /// Not one of the files this chat read.
    NoSuchFile,
    /// Too short to tell a quote from a coincidence.
    TooShort,
    /// The file was read and the quote is not in it.
    NotFound,
}

/// A citation with its verdict, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    pub raw: String,
    /// The file as the chat names it (`@kelp/survey.pdf`), resolved from
    /// what the persona wrote where that was unambiguous — so the page can
    /// open it.
    pub file: String,
    /// The first page the persona cited, if it cited one.
    pub cited: Option<u32>,
    /// The quote as the persona wrote it, for the page to mark.
    pub quote: String,
    #[serde(flatten)]
    pub verdict: Verdict,
}

/// Every citation in `text`, in order. Accepts what models write for the
/// asked-for form: `p.` or `pp.` with a range, `page`, no page at all, a
/// comma or none before it, straight or curly quotes.
pub fn parse(text: &str) -> Vec<Citation> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = closing(after) else {
            rest = after;
            continue;
        };
        let inner = &after[..close];
        if let Some(c) = parse_inner(inner) {
            out.push(Citation {
                raw: format!("[{inner}]"),
                ..c
            });
            rest = &after[close + 1..];
        } else {
            rest = after;
        }
    }
    out
}

/// Where the citation opened just before `s` closes: the first `]` that
/// follows a closing quote, within reach.
fn closing(s: &str) -> Option<usize> {
    let mut chars = 0usize;
    let mut last = None;
    for (i, c) in s.char_indices() {
        chars += 1;
        if chars > MAX_CITATION_CHARS || c == '[' || c == '\n' {
            return None;
        }
        if c == ']' && matches!(last, Some('"' | '\u{201d}')) {
            return Some(i);
        }
        if !c.is_whitespace() {
            last = Some(c);
        }
    }
    None
}

fn parse_inner(inner: &str) -> Option<Citation> {
    // The quote opens at the first `:` followed by a quote mark — a file
    // name may hold a colon (`@group:all/…`), a quote mark never does.
    let (head, quote) = inner.char_indices().find_map(|(i, c)| {
        (c == ':').then(|| {
            let tail = inner[i + 1..].trim_start();
            let q = tail
                .strip_prefix('"')
                .or_else(|| tail.strip_prefix('\u{201c}'))?;
            let q = q.trim_end();
            let q = q.strip_suffix('"').or_else(|| q.strip_suffix('\u{201d}'))?;
            Some((&inner[..i], q))
        })?
    })?;
    let head = head.trim();
    let (file, pages) = split_pages(head);
    let file = file.trim().trim_end_matches(',').trim();
    if file.is_empty() || quote.trim().is_empty() {
        return None;
    }
    Some(Citation {
        raw: String::new(),
        file: file.to_string(),
        pages,
        quote: quote.to_string(),
    })
}

/// `paper.pdf, p. 3` → (`paper.pdf`, 3–3); `x.pdf pp. 3-4` → 3–4;
/// `x.pdf, page 2`; `x.pdf` → no pages.
fn split_pages(head: &str) -> (&str, Option<(u32, u32)>) {
    let lower = head.to_ascii_lowercase();
    for marker in [" pp.", ",pp.", " p.", ",p.", " page ", ",page ", " pages "] {
        if let Some(at) = lower.rfind(marker) {
            let spec = head[at + marker.len()..].trim();
            if let Some(range) = page_range(spec) {
                return (&head[..at], Some(range));
            }
        }
    }
    (head, None)
}

fn page_range(spec: &str) -> Option<(u32, u32)> {
    let mut parts = spec.split(['-', '\u{2013}', '\u{2014}']);
    let first: u32 = parts.next()?.trim().parse().ok()?;
    let last = match parts.next() {
        Some(p) => p.trim().parse().ok()?,
        None => first,
    };
    parts.next().is_none().then_some((first, last.max(first)))
}

/// One page of one file, as the chat received it.
#[derive(Debug, Clone)]
struct Page {
    file: String,
    /// `None` for a text file, which has no pages.
    page: Option<u32>,
    text: String,
}

/// What the chat received of its files, page by page: the files block in
/// its first turn, and every reader result the model read
/// ([`grounding::calls`]: first seen wins, a stale or failed result is
/// never evidence).
fn received(messages: &[Message]) -> Vec<Page> {
    let read: HashMap<&str, &str> = grounding::calls(messages)
        .into_iter()
        .filter(|c| READERS.contains(&c.name))
        .filter_map(|c| Some((c.id, c.result?)))
        .collect();
    let mut pages = Vec::new();
    for m in messages {
        if m.role != Role::User {
            continue;
        }
        for block in &m.content {
            match block {
                // The same predicate as `files::carries` and
                // `Taint::arm_for_content` (review of #465).
                Block::Text { text } if text.trim_start().starts_with(super::files::FILES_STEM) => {
                    pages.extend(documents(text));
                }
                Block::ToolResult { tool_use_id, .. } => {
                    if let Some(text) = read.get(tool_use_id.as_str()) {
                        pages.extend(documents(text));
                    }
                }
                _ => {}
            }
        }
    }
    // A page received twice — the files block and a later `file_read` of
    // it, or text layer and OCR — is one page, whatever came between, so a
    // file is named once and a citation without its `@group/` resolves
    // (review of #465: a consecutive-only dedup listed it twice, and the
    // tail match then refused as ambiguous).
    let mut merged: Vec<Page> = Vec::new();
    for p in pages {
        match merged
            .iter_mut()
            .find(|m| m.file == p.file && m.page == p.page)
        {
            Some(m) => {
                m.text.push('\n');
                m.text.push_str(&p.text);
            }
            None => merged.push(p),
        }
    }
    merged
}

/// The documents a rendered text holds, split at each `document: <name> ·`
/// header and each document at its `=== page N of M ·` markers.
fn documents(text: &str) -> Vec<Page> {
    let mut out = Vec::new();
    let mut current: Option<(String, Option<u32>, String)> = None;
    let flush = |cur: Option<(String, Option<u32>, String)>, out: &mut Vec<Page>| {
        if let Some((file, page, text)) = cur {
            out.push(Page { file, page, text });
        }
    };
    for line in text.lines() {
        if let Some(name) = header(line) {
            flush(current.take(), &mut out);
            current = Some((name.to_string(), None, String::new()));
            continue;
        }
        if let (Some(page), Some((file, _, _))) = (page_marker(line), &current) {
            let file = file.clone();
            flush(current.take(), &mut out);
            current = Some((file, Some(page), String::new()));
            continue;
        }
        if let Some((_, _, body)) = &mut current {
            body.push_str(line);
            body.push('\n');
        }
    }
    flush(current, &mut out);
    out
}

/// `document: paper.pdf · pdf · 12 page(s) · …` → `paper.pdf`.
fn header(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("document: ")?;
    let (name, _) = rest.split_once(" \u{b7} ")?;
    (!name.is_empty()).then_some(name)
}

/// `=== page 3 of 12 · text layer … ===` → 3.
fn page_marker(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("=== page ")?;
    let (n, rest) = rest.split_once(' ')?;
    rest.starts_with("of ").then_some(())?;
    n.parse().ok()
}

/// Lowercase, with typographic quotes and dashes made plain, ligatures
/// spelled out and soft hyphens dropped — character by character, so a
/// word keeps its place in the text.
fn plain(s: &str) -> String {
    let mut plain = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\u{2018}' | '\u{2019}' | '\u{201b}' | '\u{2032}' => plain.push('\''),
            '\u{201c}' | '\u{201d}' | '\u{201f}' | '\u{2033}' => plain.push('"'),
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2212}' => {
                plain.push('-')
            }
            '\u{fb00}' => plain.push_str("ff"),
            '\u{fb01}' => plain.push_str("fi"),
            '\u{fb02}' => plain.push_str("fl"),
            '\u{fb03}' => plain.push_str("ffi"),
            '\u{fb04}' => plain.push_str("ffl"),
            '\u{ad}' => {}
            c => plain.extend(c.to_lowercase()),
        }
    }
    plain
}

/// A word as [`normal`] leaves it: [`plain`], punctuation around it gone.
fn word(raw: &str) -> String {
    plain(raw)
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string()
}

/// What both sides go through before containment: lowercase, typographic
/// quotes and dashes made plain, ligatures spelled out, a word hyphenated
/// across a line break joined, the punctuation around each word dropped,
/// and the words joined by single spaces.
fn normal(s: &str) -> String {
    let plain = plain(s);
    // "kelp hold-\nfasts" is "kelp holdfasts": a hyphen that ends a line
    // before a lowercase letter is the line's, not the word's.
    let mut joined = String::with_capacity(plain.len());
    let mut chars = plain.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '-' && chars.peek() == Some(&'\n') {
            let mut ahead = chars.clone();
            ahead.next();
            while ahead.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
                ahead.next();
            }
            if ahead.peek().is_some_and(|c| c.is_lowercase()) {
                chars = ahead;
                continue;
            }
        }
        joined.push(c);
    }
    // Word by word, as `grounding::holds` compares: punctuation around a
    // word dropped, so a copy that loses a comma or the quote marks around
    // a term is still the same words.
    joined
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Which received file a citation names: exactly, or — a model dropping
/// the `@group/` prefix — the one file whose last part it is.
fn resolve<'a>(cited: &str, files: &[&'a str]) -> Option<&'a str> {
    if let Some(f) = files.iter().find(|f| **f == cited) {
        return Some(f);
    }
    let mut tail = files.iter().filter(|f| f.rsplit('/').next() == Some(cited));
    match (tail.next(), tail.next()) {
        (Some(f), None) => Some(f),
        _ => None,
    }
}

/// Every citation in the persona's replies, each checked against what the
/// chat had received of its files by the time it was written — a quote
/// from a page read only later does not count for an earlier answer.
pub fn check_conversation(messages: &[Message]) -> Vec<Checked> {
    let mut out = Vec::new();
    for (i, m) in messages.iter().enumerate() {
        if m.role != Role::Assistant {
            continue;
        }
        let text: String = m
            .content
            .iter()
            .filter_map(|b| match b {
                Block::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let cites = parse(&text);
        if cites.is_empty() {
            continue;
        }
        let pages = received(&messages[..i]);
        out.extend(cites.iter().map(|c| check(c, &pages)));
    }
    out
}

fn claim<'a>(statement: &'a str, id: &'a str, quote: &'a str) -> Claim<'a> {
    Claim {
        statement,
        id,
        quote,
    }
}

fn check(c: &Citation, pages: &[Page]) -> Checked {
    let mut files: Vec<&str> = pages.iter().map(|p| p.file.as_str()).collect();
    files.sort_unstable();
    files.dedup();
    let Some(file) = resolve(&c.file, &files) else {
        return Checked {
            raw: c.raw.clone(),
            file: c.file.clone(),
            cited: c.pages.map(|p| p.0),
            quote: c.quote.clone(),
            verdict: Verdict::NoSuchFile,
        };
    };
    let cited = c.pages.map(|p| p.0);
    let done = |verdict| Checked {
        raw: c.raw.clone(),
        file: file.to_string(),
        cited,
        quote: c.quote.clone(),
        verdict,
    };
    let quote = normal(&c.quote);
    let packet: Vec<Evidence> = pages
        .iter()
        .filter(|p| p.file == file)
        .map(|p| Evidence {
            id: p.page.map(|n| n.to_string()).unwrap_or_default(),
            source: p.file.clone(),
            text: normal(&p.text),
        })
        .collect();
    // The page cited first, then the rest of the file: admit is the lookup
    // either way, so its refusals are the verdicts.
    let in_range = |e: &Evidence| match (c.pages, e.id.parse::<u32>().ok()) {
        (Some((a, b)), Some(n)) => (a..=b).contains(&n),
        // No page cited, or a text file with none: the file is the page.
        _ => true,
    };

    let mut short = false;
    for e in packet.iter().filter(|e| in_range(e)) {
        match grounding::admit(
            &claim(&c.raw, &e.id, &quote),
            std::slice::from_ref(e),
            MIN_QUOTE_CHARS,
        ) {
            Ok(e) => {
                return done(Verdict::Quoted {
                    found: e.id.parse().ok(),
                })
            }
            Err(Refusal::QuoteTooShort) => short = true,
            Err(_) => {}
        }
    }
    if short {
        return done(Verdict::TooShort);
    }
    for e in packet.iter().filter(|e| !in_range(e)) {
        if grounding::admit(
            &claim(&c.raw, &e.id, &quote),
            std::slice::from_ref(e),
            MIN_QUOTE_CHARS,
        )
        .is_ok()
        {
            if let Ok(found) = e.id.parse() {
                return done(Verdict::OtherPage { found });
            }
        }
    }
    done(Verdict::NotFound)
}

/// One page of `file` as the chat received it — the files block and every
/// reader result, a page read twice joined — for the page to show beside a
/// citation. `None` for a page it never received. A text file's one body
/// answers any page.
pub fn page_text(messages: &[Message], file: &str, page: Option<u32>) -> Option<String> {
    let pages = received(messages);
    let mut files: Vec<&str> = pages.iter().map(|p| p.file.as_str()).collect();
    files.sort_unstable();
    files.dedup();
    let file = resolve(file, &files)?;
    let mut of_file = pages.iter().filter(|p| p.file == file);
    let unpaged = of_file.clone().all(|p| p.page.is_none());
    of_file
        .find(|p| unpaged || p.page == page)
        .map(|p| p.text.trim().to_string())
}

/// Where `quote` sits in `text`, as byte offsets of its first and last
/// word — the words compared as the check compares them, so the passage it
/// admitted is the passage marked. `None` when it is not there.
pub fn mark(text: &str, quote: &str) -> Option<(usize, usize)> {
    // Every word with its span in `text`; a word hyphenated across a line
    // break is one word, as `normal` joins it.
    let mut words: Vec<(usize, usize, String)> = Vec::new();
    let mut spans = text
        .split_whitespace()
        .map(|w| {
            let start = w.as_ptr() as usize - text.as_ptr() as usize;
            (start, start + w.len())
        })
        .peekable();
    while let Some((start, mut end)) = spans.next() {
        let mut joined = word(&text[start..end]);
        while plain(&text[start..end]).ends_with('-') {
            let Some(&(next, next_end)) = spans.peek() else {
                break;
            };
            let gap = &text[end..next];
            let broken = gap.starts_with('\n') && gap[1..].chars().all(|c| c == ' ' || c == '\t');
            if !(broken && plain(&text[next..next_end]).starts_with(char::is_lowercase)) {
                break;
            }
            spans.next();
            joined.push_str(&word(&text[next..next_end]));
            end = next_end;
        }
        if !joined.is_empty() {
            words.push((start, end, joined));
        }
    }
    let quote = normal(quote);
    let wanted: Vec<&str> = quote.split(' ').filter(|w| !w.is_empty()).collect();
    if wanted.is_empty() {
        return None;
    }
    words
        .windows(wanted.len())
        .find(|run| run.iter().map(|w| w.2.as_str()).eq(wanted.iter().copied()))
        .map(|run| (run[0].0, run[run.len() - 1].1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;
    use serde_json::json;

    const PAPER: &str =
        "document: kelp.pdf \u{b7} pdf \u{b7} 2 page(s) \u{b7} sha256 0123456789ab\n\
        \n=== page 1 of 2 \u{b7} text layer (the file's own words) ===\n\
        Sea urchins graze kelp hold-\nfasts at night.\n\
        \n=== page 2 of 2 \u{b7} text layer (the file's own words) ===\n\
        Sea otters keep urchin \u{201c}barrens\u{201d} in check \u{2014} mostly.\n";

    fn said(text: &str) -> Message {
        Message::assistant(vec![Block::Text { text: text.into() }])
    }

    fn calls(id: &str, input: serde_json::Value) -> Message {
        Message::assistant(vec![Block::ToolUse {
            id: id.into(),
            name: "file_read".into(),
            input,
        }])
    }

    fn result(id: &str, is_error: bool, content: &str) -> Message {
        Message {
            role: Role::User,
            content: vec![Block::ToolResult {
                tool_use_id: id.into(),
                content: content.into(),
                is_error,
            }],
            ..Message::user("")
        }
    }

    fn files_block(body: &str) -> String {
        format!(
            "{} \u{2014} material.)\n\n{body}",
            super::super::files::FILES_STEM
        )
    }

    fn chat(first: &str, reply: &str) -> Vec<Message> {
        let mut user = Message::user("What do urchins do?");
        user.content.push(Block::Text {
            text: files_block(first),
        });
        vec![user, said(reply)]
    }

    #[test]
    fn citations_parse_in_the_forms_models_write() {
        let text = "Urchins graze [kelp.pdf, p. 1: \"graze kelp holdfasts\"] and otters \
            check them [kelp.pdf pp. 1-2: \u{201c}keep urchin barrens\u{201d}]. See \
            [@group:all/guide.md: \"a field guide\"] and [not a citation] and \
            [notes.md, page 3: \"x\"].";
        let got = parse(text);
        type Row<'a> = (&'a str, Option<(u32, u32)>, &'a str);
        let summary: Vec<Row> = got
            .iter()
            .map(|c| (c.file.as_str(), c.pages, c.quote.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                ("kelp.pdf", Some((1, 1)), "graze kelp holdfasts"),
                ("kelp.pdf", Some((1, 2)), "keep urchin barrens"),
                ("@group:all/guide.md", None, "a field guide"),
                ("notes.md", Some((3, 3)), "x"),
            ]
        );
        assert!(text.contains(&got[1].raw), "{}", got[1].raw);
    }

    /// The text breaks the line mid-word and spells its quotes and dashes
    /// typographically; the model's copy does neither. Both are the same
    /// words, so the quote is there.
    #[test]
    fn a_quote_across_a_hyphenated_line_break_is_found_on_its_page() {
        let msgs = chat(
            PAPER,
            "They graze [kelp.pdf, p. 1: \"Sea urchins graze kelp holdfasts at night\"]; \
             otters hold them [kelp.pdf, p. 2: \"keep urchin \"barrens\" in check - mostly\"].",
        );
        let got = check_conversation(&msgs);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0].verdict, Verdict::Quoted { found: Some(1) });
        assert_eq!(got[1].verdict, Verdict::Quoted { found: Some(2) });
    }

    #[test]
    fn a_wrong_page_a_made_up_quote_an_unread_file_and_a_short_quote_are_each_said() {
        let msgs = chat(
            PAPER,
            "[kelp.pdf, p. 1: \"Sea otters keep urchin barrens in check\"] \
             [kelp.pdf, p. 2: \"otters eat forty urchins a day\"] \
             [other.pdf, p. 1: \"anything at all here\"] \
             [kelp.pdf, p. 1: \"at night\"]",
        );
        let got: Vec<Verdict> = check_conversation(&msgs)
            .into_iter()
            .map(|c| c.verdict)
            .collect();
        assert_eq!(
            got,
            [
                Verdict::OtherPage { found: 2 },
                Verdict::NotFound,
                Verdict::NoSuchFile,
                Verdict::TooShort,
            ]
        );
    }

    /// A page the chat read with `file_read` is evidence; one it never
    /// read is not, even when the file holds it — the check is over what
    /// the run received. And only from the moment it was read.
    #[test]
    fn a_file_read_result_counts_from_when_it_was_read() {
        let mut user = Message::user("Read the survey.");
        user.content.push(Block::Text {
            text: files_block("Your files are too long.\n\n- `@kelp/survey.pdf`"),
        });
        let early = said("Before reading: [survey.pdf, p. 4: \"transects were run monthly\"]");
        let ask = calls("t1", json!({"file": "@kelp/survey.pdf", "pages": "4"}));
        let answer = result(
            "t1",
            false,
            "document: @kelp/survey.pdf \u{b7} pdf \u{b7} 9 page(s) \u{b7} sha256 x\n\
             \n=== page 4 of 9 \u{b7} text layer (the file's own words) ===\n\
             The transects were run monthly from May.\n",
        );
        let late = said(
            "[survey.pdf, p. 4: \"transects were run monthly\"] but not \
             [@kelp/survey.pdf, p. 5: \"transects were run monthly\"]",
        );
        let got = check_conversation(&[user, early, ask, answer, late]);
        let v: Vec<(&str, &Verdict)> = got.iter().map(|c| (c.file.as_str(), &c.verdict)).collect();
        assert_eq!(
            v,
            [
                ("survey.pdf", &Verdict::NoSuchFile),
                ("@kelp/survey.pdf", &Verdict::Quoted { found: Some(4) }),
                ("@kelp/survey.pdf", &Verdict::OtherPage { found: 4 }),
            ]
        );
    }

    /// A stale result — evicted by compaction — never grounds a quote,
    /// and an errored read is no evidence (grounding's walk).
    #[test]
    fn a_failed_read_is_no_evidence() {
        let ask = calls("t1", json!({"file": "kelp.pdf"}));
        let failed = result("t1", true, PAPER);
        let reply = said("[kelp.pdf, p. 1: \"Sea urchins graze kelp\"]");
        let got = check_conversation(&[Message::user("hi"), ask, failed, reply]);
        assert_eq!(got[0].verdict, Verdict::NoSuchFile);
    }

    #[test]
    fn a_text_file_has_no_pages_so_any_page_cited_is_the_file() {
        let msgs = chat(
            "document: notes.md \u{b7} text\n\nUrchins graze kelp holdfasts.",
            "[notes.md, p. 3: \"Urchins graze kelp holdfasts\"]",
        );
        assert_eq!(
            check_conversation(&msgs)[0].verdict,
            Verdict::Quoted { found: None }
        );
    }

    #[test]
    fn a_verdict_serialises_flat_for_the_page() {
        let c = Checked {
            raw: "[kelp.pdf, p. 1: \"x\"]".into(),
            file: "kelp.pdf".into(),
            cited: Some(1),
            quote: "x".into(),
            verdict: Verdict::OtherPage { found: 2 },
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["status"], "other_page");
        assert_eq!(
            (v["cited"].as_u64(), v["found"].as_u64()),
            (Some(1), Some(2)),
            "{v}"
        );
    }

    /// The page shows the page the chat read, with the passage the check
    /// admitted marked — across the line break and the curly quotes the
    /// model's copy did not have.
    #[test]
    fn the_cited_page_comes_back_with_its_passage_marked() {
        let msgs = chat(PAPER, "ok");
        let page = page_text(&msgs, "kelp.pdf", Some(1)).unwrap();
        assert!(page.starts_with("Sea urchins graze kelp hold-"), "{page}");
        let (a, b) = mark(&page, "urchins graze kelp holdfasts").unwrap();
        assert_eq!(&page[a..b], "urchins graze kelp hold-\nfasts");
        let two = page_text(&msgs, "kelp.pdf", Some(2)).unwrap();
        let (a, b) = mark(&two, "keep urchin barrens in check").unwrap();
        assert_eq!(&two[a..b], "keep urchin \u{201c}barrens\u{201d} in check");
        assert_eq!(mark(&two, "otters eat urchins"), None);
        assert_eq!(
            page_text(&msgs, "kelp.pdf", Some(3)),
            None,
            "never received"
        );
        assert_eq!(page_text(&msgs, "other.pdf", Some(1)), None);
    }

    /// Review of #465: a file received twice, with another between, is one
    /// file — so `survey.pdf` still names `@kelp/survey.pdf` and its quote
    /// is quoted, not "no such file".
    #[test]
    fn a_file_received_twice_with_another_between_still_resolves() {
        let survey = "document: @kelp/survey.pdf \u{b7} pdf \u{b7} 9 page(s) \u{b7} sha256 x\n\
                      \n=== page 4 of 9 \u{b7} text layer (the file's own words) ===\n\
                      The transects were run monthly from May.\n";
        let mut user = Message::user("Read them.");
        user.content.push(Block::Text {
            text: files_block(&format!("{survey}\n{PAPER}")),
        });
        let ask = calls("t1", json!({"file": "@kelp/survey.pdf", "pages": "4"}));
        let answer = result("t1", false, survey);
        let reply = said("[survey.pdf, p. 4: \"transects were run monthly\"]");
        let msgs = [user, ask, answer, reply];
        let got = check_conversation(&msgs);
        assert_eq!(
            got[0].verdict,
            Verdict::Quoted { found: Some(4) },
            "{got:?}"
        );
        assert_eq!(got[0].file, "@kelp/survey.pdf");
        assert!(page_text(&msgs, "survey.pdf", Some(4)).is_some());
    }
}
